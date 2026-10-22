//! Collect authored navigation from the final display list. Coordinates follow
//! the same PDF transforms as paint, including page presentation and clipping.

use crate::canvas::{
    Command, Document, META_DIAGNOSTIC_SCOPE_BEGIN_KEY, META_DIAGNOSTIC_SCOPE_END_KEY,
    META_READING_TEXT_KEY, PageGeometry,
};
use crate::font::FontRegistry;
use crate::link_semantics::{
    META_DESTINATION, META_LINK_ID, META_LINK_LABEL, META_LINK_TARGET, META_NAVIGATION_SUPPRESSED,
    local_fragment,
};
use crate::pdf::CompiledFlowTextOverride;
use crate::pdf_link_geometry::{Matrix, Point, Quad, Region, flatten_cubic};
use crate::types::{PageOrientation, Pt, Size};
use std::collections::{BTreeMap, HashMap};
use std::io;

/// Coordinates are milli-points in the current top-down drawing space. These
/// markers describe the authoring box, including padding, before CSS transforms.
pub(crate) const META_LINK_RECT: &str = "__fb_link_rect";
pub(crate) const META_LINK_QUAD: &str = "__fb_link_quad";
pub(crate) const META_DESTINATION_POINT: &str = "__fb_link_destination_point";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Uri(String),
    Local(String),
}

impl Target {
    fn from_href(href: &str) -> Self {
        let href = href.trim_matches(|c: char| c <= '\u{20}');
        if href.is_empty() {
            return Self::Local(String::new());
        }
        if let Some(name) = local_fragment(href) {
            return Self::Local(name);
        }
        // PDF URI actions carry ASCII URI bytes. Preserve URI delimiters and
        // existing escapes, and encode UTF-8 bytes without losing non-ASCII text.
        let mut uri = String::new();
        for byte in href.bytes() {
            if (0x21..=0x7e).contains(&byte) {
                uri.push(char::from(byte));
            } else {
                use std::fmt::Write;
                write!(&mut uri, "%{byte:02X}").unwrap();
            }
        }
        Self::Uri(uri)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Link {
    pub source_id: String,
    pub target: Target,
    pub label: String,
    pub region: Region,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PageLinks {
    pub links: Vec<Link>,
    pub destinations: Vec<(String, Point)>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocumentLinks {
    pub pages: Vec<PageLinks>,
    pub destinations: BTreeMap<String, (usize, Point)>,
}

impl DocumentLinks {
    pub(crate) fn collect(
        document: &Document,
        page_size: Size,
        registry: Option<&FontRegistry>,
        overrides: &[Vec<CompiledFlowTextOverride>],
    ) -> io::Result<Self> {
        if !document
            .pages
            .iter()
            .any(|page| has_navigation(&page.commands))
        {
            return Ok(Self::default());
        }
        let mut forms = BTreeMap::new();
        for page in &document.pages {
            collect_forms(&page.commands, &mut forms);
        }
        let mut result = Self::default();
        for (page_index, page) in document.pages.iter().enumerate() {
            let geometry = PageGeometry::for_page(page, page_size);
            let mut collector = Collector {
                registry,
                forms: &forms,
                output: PageLinks::default(),
                link_indices: HashMap::new(),
                labels: HashMap::new(),
                form_stack: Vec::new(),
            };
            let state = State {
                matrix: presentation_matrix(geometry),
                clip: Quad::rectangle(
                    Matrix::IDENTITY,
                    0.0,
                    0.0,
                    pt(geometry.media_size.width),
                    pt(geometry.media_size.height),
                )
                .map(Region::quad)
                .unwrap_or_default(),
                ..State::default()
            };
            collector.walk(
                &page.commands,
                pt(geometry.logical_size.height),
                state,
                Scope::default(),
                overrides.get(page_index).map(Vec::as_slice).unwrap_or(&[]),
            )?;
            for link in &mut collector.output.links {
                if link.label.is_empty() {
                    link.label = collector.labels.remove(&link.source_id).unwrap_or_default();
                }
                link.label = link.label.split_whitespace().collect::<Vec<_>>().join(" ");
            }
            collector
                .output
                .links
                .retain(|link| !link.region.quads.is_empty());
            for (name, point) in &collector.output.destinations {
                result
                    .destinations
                    .entry(name.clone())
                    .or_insert((page_index, *point));
            }
            if page_index == 0 {
                result.destinations.insert(
                    String::new(),
                    (
                        0,
                        presentation_matrix(geometry)
                            .point(Point::new(0.0, pt(geometry.logical_size.height))),
                    ),
                );
            }
            result.pages.push(collector.output);
        }
        // Missing local destinations must not produce dangling PDF references.
        for page in &mut result.pages {
            page.links.retain(|link| match &link.target {
                Target::Local(name) => result.destinations.contains_key(name),
                Target::Uri(_) => true,
            });
        }
        Ok(result)
    }
}

fn pt(value: Pt) -> f64 {
    value.to_milli_i64() as f64 / 1000.0
}

pub(crate) fn presentation_matrix(geometry: PageGeometry) -> Matrix {
    let extent = pt(geometry.presentation.media_extent());
    let presentation = match geometry.presentation.orientation {
        PageOrientation::Upright => Matrix([1.0, 0.0, 0.0, 1.0, extent, extent]),
        PageOrientation::RotateLeft => Matrix([
            0.0,
            1.0,
            -1.0,
            0.0,
            pt(geometry.logical_size.height) + extent,
            extent,
        ]),
        PageOrientation::RotateRight => Matrix([
            0.0,
            -1.0,
            1.0,
            0.0,
            extent,
            pt(geometry.logical_size.width) + extent,
        ]),
    };
    const SCALE: f64 = 0.999_999_96;
    // Match the nine-decimal translation emitted by the PDF content writer.
    let anchor =
        (f64::from(geometry.media_size.height.to_f32()) * (1.0 - SCALE) * 1e9).round() / 1e9;
    Matrix([SCALE, 0.0, 0.0, SCALE, 0.0, anchor]).then(presentation)
}

pub(crate) fn has_navigation(commands: &[Command]) -> bool {
    commands.iter().any(|command| match command {
        Command::Meta { key, .. } => {
            key == META_LINK_ID
                || key == META_DESTINATION
                || key == META_LINK_RECT
                || key == META_LINK_QUAD
        }
        Command::DefineForm { commands, .. } | Command::DefineIsolatedForm { commands, .. } => {
            has_navigation(commands)
        }
        _ => false,
    })
}

type Forms = BTreeMap<String, (Size, Vec<Command>)>;

fn collect_forms(commands: &[Command], forms: &mut Forms) {
    for command in commands {
        if let Command::DefineForm {
            resource_id,
            width,
            height,
            commands,
        }
        | Command::DefineIsolatedForm {
            resource_id,
            width,
            height,
            commands,
        } = command
        {
            forms.insert(
                resource_id.clone(),
                (
                    Size {
                        width: *width,
                        height: *height,
                    },
                    commands.clone(),
                ),
            );
            collect_forms(commands, forms);
        }
    }
}

#[derive(Clone)]
struct State {
    matrix: Matrix,
    clip: Region,
    font: String,
    size: Pt,
    text_mode: u8,
    fill: f32,
    stroke: f32,
    transparent_placement: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            matrix: Matrix::IDENTITY,
            clip: Region::default(),
            font: "Helvetica".into(),
            size: Pt::from_f32(12.0),
            text_mode: 0,
            fill: 1.0,
            stroke: 1.0,
            transparent_placement: false,
        }
    }
}

impl State {
    fn text_visible(&self) -> bool {
        if self.transparent_placement {
            return false;
        }
        match self.text_mode {
            0 | 4 => self.fill > 0.0,
            1 | 5 => self.stroke > 0.0,
            2 | 6 => self.fill > 0.0 || self.stroke > 0.0,
            _ => false,
        }
    }
}

#[derive(Clone, Default)]
struct Scope {
    source_id: Option<String>,
    target: Option<Target>,
    label: Option<String>,
    destinations: Vec<String>,
    navigation_suppressed: bool,
}

#[derive(Default)]
struct Path {
    contours: Vec<Vec<Point>>,
}

impl Path {
    fn move_to(&mut self, point: Point) {
        self.contours.push(vec![point]);
    }

    fn line_to(&mut self, point: Point) {
        if let Some(contour) = self.contours.last_mut() {
            contour.push(point);
        } else {
            self.move_to(point);
        }
    }

    fn curve_to(&mut self, a: Point, b: Point, end: Point) {
        if let Some(contour) = self.contours.last_mut() {
            if let Some(start) = contour.last().copied() {
                flatten_cubic(start, a, b, end, 0.01, contour);
            }
        }
    }

    fn close(&mut self) {
        if let Some(contour) = self.contours.last_mut() {
            if let Some(first) = contour.first().copied() {
                contour.push(first);
            }
        }
    }

    fn consume(&mut self, evenodd: bool) -> Region {
        let region = Region::from_contours(&self.contours, evenodd);
        self.contours.clear();
        region
    }
}

struct Collector<'a> {
    registry: Option<&'a FontRegistry>,
    forms: &'a Forms,
    output: PageLinks,
    link_indices: HashMap<String, usize>,
    labels: HashMap<String, String>,
    form_stack: Vec<String>,
}

impl Collector<'_> {
    fn text(&mut self, scope: &Scope, text: &str) {
        if scope.navigation_suppressed {
            return;
        }
        if let Some(id) = &scope.source_id {
            self.labels.entry(id.clone()).or_default().push_str(text);
        }
    }

    fn destination(&mut self, scope: &Scope, point: Point) {
        if scope.navigation_suppressed {
            return;
        }
        for name in &scope.destinations {
            if !self
                .output
                .destinations
                .iter()
                .any(|(existing, _)| existing == name)
            {
                self.output.destinations.push((name.clone(), point));
            }
        }
    }

    fn region(&mut self, scope: &Scope, state: &State, region: Region) {
        if scope.navigation_suppressed || state.transparent_placement {
            return;
        }
        let region = region.intersect(&state.clip);
        let Some(bounds) = region.bounds() else {
            return;
        };
        self.destination(scope, Point::new(bounds[0], bounds[3]));
        let (Some(id), Some(target)) = (&scope.source_id, &scope.target) else {
            return;
        };
        let index = if let Some(index) = self.link_indices.get(id) {
            *index
        } else {
            let index = self.output.links.len();
            self.output.links.push(Link {
                source_id: id.clone(),
                target: target.clone(),
                label: scope.label.clone().unwrap_or_default(),
                region: Region::default(),
            });
            self.link_indices.insert(id.clone(), index);
            index
        };
        self.output.links[index].region.quads.extend(region.quads);
    }

    fn rectangle(&mut self, scope: &Scope, state: &State, x: f64, y: f64, width: f64, height: f64) {
        if let Some(quad) = Quad::rectangle(state.matrix, x, y, width, height) {
            self.region(scope, state, Region::quad(quad));
        }
    }

    fn text_metrics(&self, state: &State, text: &str) -> (f64, f64, f64) {
        if let Some(registry) = self
            .registry
            .filter(|registry| registry.resolve(&state.font).is_some())
        {
            let width = pt(registry.measure_text_width(&state.font, state.size, text));
            let (ascent, descent) = registry
                .vertical_metrics(&state.font, state.size)
                .unwrap_or((state.size * 0.8, state.size * 0.2));
            return (width, pt(ascent), pt(descent));
        }
        let (_, clean) = crate::text_shape::decode_shape_options(text);
        let width = crate::base14_metrics::font(&state.font)
            .map(|font| {
                clean
                    .chars()
                    .map(|ch| {
                        font.glyph_by_unicode(ch)
                            .or_else(|| font.glyph_by_unicode('?'))
                    })
                    .flatten()
                    .map(|glyph| f64::from(glyph.width))
                    .sum::<f64>()
                    * pt(state.size)
                    / 1000.0
            })
            .unwrap_or(0.0);
        // CSS's standard-font line box metrics; embedded fonts use their own
        // hhea metrics above. These bounds describe the text box, not glyph ink.
        (width, pt(state.size) * 0.8, pt(state.size) * 0.2)
    }

    fn glyph_width(&self, state: &State, glyph: u16) -> f64 {
        self.registry
            .and_then(|registry| registry.glyph_advance_units(&state.font, glyph))
            .map(|(advance, units)| pt(state.size) * f64::from(advance) / f64::from(units))
            .unwrap_or(0.0)
    }

    fn walk(
        &mut self,
        commands: &[Command],
        height: f64,
        mut state: State,
        mut scope: Scope,
        overrides: &[CompiledFlowTextOverride],
    ) -> io::Result<()> {
        let mut states = Vec::new();
        let mut scopes = Vec::new();
        let mut path = Path::default();
        let mut next_override = 0;
        for (index, command) in commands.iter().enumerate() {
            let replacement = overrides
                .get(next_override)
                .filter(|value| value.command == index);
            if replacement.is_some() {
                next_override += 1;
            }
            match command {
                Command::SaveState => states.push(state.clone()),
                Command::RestoreState => {
                    if let Some(saved) = states.pop() {
                        state = saved;
                    }
                }
                Command::Translate(x, y) => {
                    state.matrix = state
                        .matrix
                        .then(Matrix([1.0, 0.0, 0.0, 1.0, pt(*x), -pt(*y)]))
                }
                Command::CssTransformOrigin { x, y, inverse } => {
                    let sign = if *inverse { -1.0 } else { 1.0 };
                    state.matrix = state.matrix.then(Matrix([
                        1.0,
                        0.0,
                        0.0,
                        1.0,
                        sign * pt(*x),
                        sign * (height - pt(*y)),
                    ]));
                }
                Command::Scale(x, y) => {
                    state.matrix = state.matrix.then(Matrix([
                        crate::pdf::serialized_real(*x),
                        0.0,
                        0.0,
                        crate::pdf::serialized_real(*y),
                        0.0,
                        0.0,
                    ]))
                }
                Command::Rotate(angle) => {
                    let (sin, cos) = crate::math::sin_cos(-*angle);
                    state.matrix = state.matrix.then(Matrix([
                        crate::pdf::serialized_real(cos),
                        crate::pdf::serialized_real(sin),
                        crate::pdf::serialized_real(-sin),
                        crate::pdf::serialized_real(cos),
                        0.0,
                        0.0,
                    ]));
                }
                Command::ConcatMatrix { a, b, c, d, e, f } => {
                    state.matrix = state.matrix.then(Matrix([
                        crate::pdf::serialized_real(*a),
                        -crate::pdf::serialized_real(*b),
                        -crate::pdf::serialized_real(*c),
                        crate::pdf::serialized_real(*d),
                        pt(*e),
                        -pt(*f),
                    ]))
                }
                Command::SetFontName(name) => state.font = name.clone(),
                Command::SetFontSize(size) => state.size = *size,
                Command::SetTextRenderingMode(mode) => state.text_mode = (*mode).min(7),
                Command::SetOpacity { fill, stroke } => {
                    state.fill = ((*fill * 1000.0).round() as i32).clamp(0, 1000) as f32 / 1000.0;
                    state.stroke =
                        ((*stroke * 1000.0).round() as i32).clamp(0, 1000) as f32 / 1000.0;
                }
                Command::Meta { key, .. } if key == META_DIAGNOSTIC_SCOPE_BEGIN_KEY => {
                    scopes.push(scope.clone())
                }
                Command::Meta { key, .. } if key == META_DIAGNOSTIC_SCOPE_END_KEY => {
                    scope = scopes.pop().ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "unbalanced link metadata scope")
                    })?;
                }
                Command::Meta { key, value } if key == META_LINK_ID => {
                    scope.source_id = Some(value.clone());
                    scope.target = None;
                    scope.label = None;
                }
                Command::Meta { key, value } if key == META_LINK_TARGET => {
                    scope.target = Some(Target::from_href(value))
                }
                Command::Meta { key, value } if key == META_LINK_LABEL => {
                    scope.label = Some(value.clone())
                }
                Command::Meta { key, value } if key == META_DESTINATION => {
                    scope.destinations.push(value.clone())
                }
                Command::Meta { key, value } if key == META_NAVIGATION_SUPPRESSED => {
                    scope.navigation_suppressed = value == "true";
                }
                Command::Meta { key, value } if key == META_READING_TEXT_KEY => {
                    self.text(&scope, value)
                }
                Command::Meta { key, value } if key == META_LINK_RECT && state.fill > 0.0 => {
                    if let Some([x, y, width, box_height]) = coordinates::<4>(value) {
                        self.rectangle(
                            &scope,
                            &state,
                            x,
                            height - y - box_height,
                            width,
                            box_height,
                        );
                    }
                }
                Command::Meta { key, value } if key == META_DESTINATION_POINT => {
                    if let Some([x, y]) = coordinates::<2>(value) {
                        // A target identifies a layout position. Clipping and
                        // opacity affect painted link regions, not positions.
                        self.destination(&scope, state.matrix.point(Point::new(x, height - y)));
                    }
                }
                Command::Meta { key, value } if key == META_LINK_QUAD && state.fill > 0.0 => {
                    if let Some(values) = coordinates::<8>(value) {
                        let points = std::array::from_fn(|index| {
                            state.matrix.point(Point::new(
                                values[index * 2],
                                height - values[index * 2 + 1],
                            ))
                        });
                        if let Some(quad) = Quad::new(points) {
                            self.region(&scope, &state, Region::quad(quad));
                        }
                    }
                }
                Command::MoveTo { x, y } => {
                    path.move_to(state.matrix.point(Point::new(pt(*x), height - pt(*y))))
                }
                Command::LineTo { x, y } => {
                    path.line_to(state.matrix.point(Point::new(pt(*x), height - pt(*y))))
                }
                Command::CurveTo {
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                } => path.curve_to(
                    state.matrix.point(Point::new(pt(*x1), height - pt(*y1))),
                    state.matrix.point(Point::new(pt(*x2), height - pt(*y2))),
                    state.matrix.point(Point::new(pt(*x), height - pt(*y))),
                ),
                Command::ClosePath => path.close(),
                Command::ClipRect {
                    x,
                    y,
                    width,
                    height: box_height,
                } => {
                    // `re W n` includes any existing path and then consumes it.
                    if let Some(quad) = Quad::rectangle(
                        state.matrix,
                        pt(*x),
                        height - pt(*y) - pt(*box_height),
                        pt(*width),
                        pt(*box_height),
                    ) {
                        path.contours.push(quad.0.to_vec());
                    }
                    state.clip = state.clip.intersect(&path.consume(false));
                }
                Command::ClipPath { evenodd } => {
                    state.clip = state.clip.intersect(&path.consume(*evenodd))
                }
                Command::Fill
                | Command::FillEvenOdd
                | Command::FillStroke
                | Command::FillStrokeEvenOdd
                | Command::Stroke => {
                    path.contours.clear();
                }
                Command::DrawString { x, y, text } => {
                    let (x, text) = replacement
                        .map(|value| (value.x, value.text.as_ref()))
                        .unwrap_or((*x, text.as_str()));
                    let (_, clean) = crate::text_shape::decode_shape_options(text);
                    self.text(&scope, clean);
                    if state.text_visible() {
                        let (width, ascent, descent) = self.text_metrics(&state, text);
                        self.rectangle(
                            &scope,
                            &state,
                            pt(x),
                            height - pt(*y) - pt(state.size) - descent,
                            width,
                            ascent + descent,
                        );
                    }
                }
                Command::DrawStringTransformed {
                    x,
                    y,
                    text,
                    m00,
                    m01,
                    m10,
                    m11,
                } => {
                    let (x, text) = replacement
                        .map(|value| (value.x, value.text.as_ref()))
                        .unwrap_or((*x, text.as_str()));
                    let (_, clean) = crate::text_shape::decode_shape_options(text);
                    self.text(&scope, clean);
                    if state.text_visible() {
                        let (width, ascent, descent) = self.text_metrics(&state, text);
                        let mut transformed = state.clone();
                        transformed.matrix = state.matrix.then(Matrix([
                            crate::pdf::serialized_real(*m00),
                            crate::pdf::serialized_real(*m01),
                            crate::pdf::serialized_real(*m10),
                            crate::pdf::serialized_real(*m11),
                            pt(x),
                            pt(*y),
                        ]));
                        self.rectangle(
                            &scope,
                            &transformed,
                            0.0,
                            -descent,
                            width,
                            ascent + descent,
                        );
                    }
                }
                Command::DrawGlyphRun {
                    x,
                    y,
                    glyph_ids,
                    advances,
                    m00,
                    m01,
                    m10,
                    m11,
                } if state.text_visible() => {
                    let (_, ascent, descent) = self.text_metrics(&state, "");
                    let mut pen = Point::new(pt(*x), height - pt(*y));
                    for (index, glyph) in glyph_ids.iter().enumerate() {
                        let mut transformed = state.clone();
                        transformed.matrix = state.matrix.then(Matrix([
                            crate::pdf::serialized_real(*m00),
                            crate::pdf::serialized_real(*m01),
                            crate::pdf::serialized_real(*m10),
                            crate::pdf::serialized_real(*m11),
                            pen.x,
                            pen.y,
                        ]));
                        self.rectangle(
                            &scope,
                            &transformed,
                            0.0,
                            -descent,
                            self.glyph_width(&state, *glyph),
                            ascent + descent,
                        );
                        if let Some((dx, dy)) = advances.get(index) {
                            pen.x += pt(*dx);
                            pen.y += pt(*dy);
                        }
                    }
                }
                Command::DrawSyntheticBoldGlyphRun {
                    x,
                    y,
                    glyph_ids,
                    advances,
                    offsets,
                    ..
                } if state.text_mode != 3 && state.fill > 0.0 => {
                    let (_, ascent, descent) = self.text_metrics(&state, "");
                    let mut pen = Point::new(pt(*x), height - pt(*y));
                    for (index, glyph) in glyph_ids.iter().enumerate() {
                        let offset = offsets.get(index).copied().unwrap_or((Pt::ZERO, Pt::ZERO));
                        self.rectangle(
                            &scope,
                            &state,
                            pen.x + pt(offset.0),
                            pen.y - pt(offset.1) - descent,
                            self.glyph_width(&state, *glyph),
                            ascent + descent,
                        );
                        if let Some((dx, dy)) = advances.get(index) {
                            pen.x += pt(*dx);
                            pen.y -= pt(*dy);
                        }
                    }
                }
                Command::DrawImage {
                    x,
                    y,
                    width,
                    height: box_height,
                    ..
                } if state.fill > 0.0 => {
                    self.rectangle(
                        &scope,
                        &state,
                        pt(*x),
                        height - pt(*y) - pt(*box_height),
                        pt(*width),
                        pt(*box_height),
                    );
                }
                Command::BeginTag { alt: Some(alt), .. } => self.text(&scope, alt),
                Command::DrawForm {
                    x,
                    y,
                    width,
                    height: box_height,
                    resource_id,
                }
                | Command::DrawFilteredForm {
                    x,
                    y,
                    width,
                    height: box_height,
                    resource_id,
                    ..
                }
                | Command::DrawMaskedForm {
                    x,
                    y,
                    width,
                    height: box_height,
                    resource_id,
                    ..
                } => {
                    let Some((size, form_commands)) = self.forms.get(resource_id) else {
                        continue;
                    };
                    if self.form_stack.len() >= 64 || self.form_stack.contains(resource_id) {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "recursive link form placement",
                        ));
                    }
                    if state.fill > 0.0
                        && !has_navigation(form_commands)
                        && scope.source_id.is_some()
                    {
                        self.rectangle(
                            &scope,
                            &state,
                            pt(*x),
                            height - pt(*y) - pt(*box_height),
                            pt(*width),
                            pt(*box_height),
                        );
                    }
                    let sx = if size.width > Pt::ZERO {
                        width.to_f32() / size.width.to_f32()
                    } else {
                        1.0
                    };
                    let sy = if size.height > Pt::ZERO {
                        box_height.to_f32() / size.height.to_f32()
                    } else {
                        1.0
                    };
                    let mut placed = state.clone();
                    // A form can contain target positions even when its
                    // placement is transparent. Its own graphics state cannot
                    // make link regions visible through that outer opacity.
                    placed.transparent_placement |= state.fill <= 0.0;
                    placed.matrix = state.matrix.then(Matrix([
                        crate::pdf::serialized_real(sx),
                        0.0,
                        0.0,
                        crate::pdf::serialized_real(sy),
                        pt(*x),
                        height - pt(*y) - pt(*box_height),
                    ]));
                    let bbox =
                        Quad::rectangle(placed.matrix, 0.0, 0.0, pt(size.width), pt(size.height))
                            .map(Region::quad)
                            .unwrap_or_default();
                    placed.clip = placed.clip.intersect(&bbox);
                    self.form_stack.push(resource_id.clone());
                    self.walk(form_commands, pt(size.height), placed, scope.clone(), &[])?;
                    self.form_stack.pop();
                }
                _ => {}
            }
        }
        if !scopes.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unclosed link metadata scope",
            ));
        }
        Ok(())
    }
}

fn coordinates<const N: usize>(value: &str) -> Option<[f64; N]> {
    let mut values = value.split(',');
    let mut result = [0.0; N];
    for coordinate in &mut result {
        *coordinate = values.next()?.parse::<i64>().ok()? as f64 / 1000.0;
    }
    values.next().is_none().then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Page;
    use std::sync::Arc;

    fn p(value: f32) -> Pt {
        Pt::from_f32(value)
    }
    fn meta(key: &str, value: &str) -> Command {
        Command::Meta {
            key: key.into(),
            value: value.into(),
        }
    }
    fn begin(id: &str, target: &str) -> Vec<Command> {
        vec![
            meta(META_DIAGNOSTIC_SCOPE_BEGIN_KEY, "test"),
            meta(META_LINK_ID, id),
            meta(META_LINK_TARGET, target),
        ]
    }
    fn end() -> Command {
        meta(META_DIAGNOSTIC_SCOPE_END_KEY, "test")
    }
    fn text(x: f32, y: f32, value: &str) -> Command {
        Command::DrawString {
            x: p(x),
            y: p(y),
            text: value.into(),
        }
    }
    fn image(x: f32, y: f32, width: f32, height: f32) -> Command {
        Command::DrawImage {
            x: p(x),
            y: p(y),
            width: p(width),
            height: p(height),
            resource_id: "test-image".into(),
            interpolate: false,
            source_clip: None,
        }
    }
    fn document(pages: Vec<Vec<Command>>) -> Document {
        Document {
            page_size: Size {
                width: p(200.0),
                height: p(200.0),
            },
            pages: pages
                .into_iter()
                .map(|commands| Page { commands })
                .collect(),
        }
    }
    fn collect(document: &Document) -> DocumentLinks {
        DocumentLinks::collect(document, document.page_size, None, &[]).unwrap()
    }
    fn mapped_point(document: &Document, x: f64, y: f64) -> Point {
        presentation_matrix(PageGeometry::for_page(
            &document.pages[0],
            document.page_size,
        ))
        .point(Point::new(x, y))
    }
    fn contains(document: &Document, link: &Link, x: f64, y: f64) -> bool {
        let point = mapped_point(document, x, y);
        link.region.quads.iter().any(|quad| quad.contains(point))
    }

    #[test]
    fn pdf_links_keep_adjacent_sources_separate_and_preserve_their_labels() {
        let mut commands = begin("1", "https://example.invalid/a");
        commands.extend([text(10.0, 10.0, "One"), end()]);
        commands.push(text(40.0, 10.0, "plain"));
        commands.extend(begin("2", "https://example.invalid/a"));
        commands.extend([
            text(80.0, 10.0, "Two"),
            meta(META_READING_TEXT_KEY, " "),
            text(105.0, 10.0, "words"),
            end(),
        ]);
        let doc = document(vec![commands]);
        let result = collect(&doc);
        assert_eq!(result.pages[0].links.len(), 2);
        assert_eq!(result.pages[0].links[0].source_id, "1");
        assert_eq!(result.pages[0].links[1].label, "Two words");
        assert!(contains(&doc, &result.pages[0].links[0], 12.0, 180.0));
        assert!(!contains(&doc, &result.pages[0].links[0], 42.0, 180.0));
        assert!(!contains(&doc, &result.pages[0].links[1], 42.0, 180.0));
    }

    #[test]
    fn pdf_links_resolve_forward_unicode_destinations_and_remove_missing_targets() {
        let mut first = begin("1", "#caf%C3%A9");
        first.extend([text(10.0, 10.0, "Forward"), end()]);
        first.extend(begin("2", "#absent"));
        first.extend([text(10.0, 40.0, "Missing"), end()]);
        first.extend(begin("3", "#"));
        first.extend([text(10.0, 70.0, "Top"), end()]);
        let second = vec![
            meta(META_DIAGNOSTIC_SCOPE_BEGIN_KEY, "target"),
            meta(META_DESTINATION, "café"),
            meta(META_DESTINATION_POINT, "20000,30000"),
            text(20.0, 30.0, "Here"),
            end(),
        ];
        let doc = document(vec![first, second]);
        let result = collect(&doc);
        assert_eq!(result.pages[0].links.len(), 2);
        assert_eq!(result.destinations["café"].0, 1);
        assert_eq!(
            result.destinations["café"].1,
            mapped_point(&doc, 20.0, 170.0)
        );
        assert_eq!(result.destinations[""].0, 0);
        assert_eq!(
            Target::from_href(" https://example.invalid/café?q=✓ "),
            Target::Uri("https://example.invalid/caf%C3%A9?q=%E2%9C%93".into())
        );
        assert_eq!(
            Target::from_href("mailto:hello@example.invalid"),
            Target::Uri("mailto:hello@example.invalid".into())
        );
    }

    #[test]
    fn pdf_links_clipping_retains_holes_and_graphics_state_restores() {
        let mut commands = begin("1", "https://example.invalid/clip");
        commands.push(Command::SaveState);
        for (x, y, w, h) in [(0.0, 0.0, 100.0, 100.0), (30.0, 30.0, 40.0, 40.0)] {
            commands.extend([
                Command::MoveTo { x: p(x), y: p(y) },
                Command::LineTo {
                    x: p(x + w),
                    y: p(y),
                },
                Command::LineTo {
                    x: p(x + w),
                    y: p(y + h),
                },
                Command::LineTo {
                    x: p(x),
                    y: p(y + h),
                },
                Command::ClosePath,
            ]);
        }
        commands.extend([
            Command::ClipPath { evenodd: true },
            image(0.0, 0.0, 100.0, 100.0),
            Command::RestoreState,
            image(120.0, 30.0, 20.0, 20.0),
            end(),
        ]);
        let doc = document(vec![commands]);
        let result = collect(&doc);
        let link = &result.pages[0].links[0];
        assert!(contains(&doc, link, 15.0, 185.0));
        assert!(!contains(&doc, link, 50.0, 150.0));
        assert!(!contains(&doc, link, 110.0, 150.0));
        assert!(contains(&doc, link, 130.0, 160.0));
    }

    #[test]
    fn pdf_links_destination_points_are_independent_of_clip_and_zero_opacity() {
        let mut commands = Vec::new();
        for (index, name) in ["inside", "outside", "transparent"].iter().enumerate() {
            commands.extend(begin(&index.to_string(), &format!("#{name}")));
            commands.extend([text(10.0, 10.0 + index as f32 * 12.0, name), end()]);
        }
        commands.extend([
            Command::SaveState,
            Command::ClipRect {
                x: p(20.0),
                y: p(50.0),
                width: p(50.0),
                height: p(50.0),
            },
            meta(META_DIAGNOSTIC_SCOPE_BEGIN_KEY, "target"),
            meta(META_DESTINATION, "inside"),
            meta(META_DESTINATION_POINT, "40000,60000"),
            end(),
            meta(META_DIAGNOSTIC_SCOPE_BEGIN_KEY, "target"),
            meta(META_DESTINATION, "outside"),
            meta(META_DESTINATION_POINT, "100000,60000"),
            end(),
            Command::RestoreState,
            Command::SetOpacity {
                fill: 0.0,
                stroke: 0.0,
            },
            meta(META_DIAGNOSTIC_SCOPE_BEGIN_KEY, "target"),
            meta(META_DESTINATION, "transparent"),
            meta(META_DESTINATION_POINT, "40000,60000"),
            end(),
        ]);
        let doc = document(vec![commands]);
        let result = collect(&doc);
        assert_eq!(result.pages[0].links.len(), 3);
        assert_eq!(result.pages[0].links[0].label, "inside");
        assert_eq!(
            result.destinations["inside"].1,
            mapped_point(&doc, 40.0, 140.0)
        );
        assert_eq!(
            result.destinations["outside"].1,
            mapped_point(&doc, 100.0, 140.0)
        );
        assert_eq!(
            result.destinations["transparent"].1,
            mapped_point(&doc, 40.0, 140.0)
        );
    }

    #[test]
    fn pdf_links_transforms_follow_top_down_canvas_coordinates() {
        let mut commands = begin("1", "https://example.invalid/transformed");
        commands.extend([
            Command::Translate(p(30.0), p(20.0)),
            Command::CssTransformOrigin {
                x: p(10.0),
                y: p(10.0),
                inverse: false,
            },
            Command::Scale(2.0, 2.0),
            Command::CssTransformOrigin {
                x: p(10.0),
                y: p(10.0),
                inverse: true,
            },
            image(10.0, 10.0, 10.0, 10.0),
            end(),
        ]);
        let doc = document(vec![commands]);
        let result = collect(&doc);
        let bounds = result.pages[0].links[0].region.bounds().unwrap();
        let low = mapped_point(&doc, 40.0, 150.0);
        let high = mapped_point(&doc, 60.0, 170.0);
        for (actual, expected) in bounds.into_iter().zip([low.x, low.y, high.x, high.y]) {
            assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
        }
    }

    #[test]
    fn pdf_links_ignore_invisible_text_zero_alpha_and_empty_regions() {
        let mut commands = begin("1", "https://example.invalid/hidden");
        commands.extend([
            Command::SetOpacity {
                fill: 0.0,
                stroke: 0.0,
            },
            image(0.0, 0.0, 10.0, 10.0),
            Command::SetOpacity {
                fill: 1.0,
                stroke: 1.0,
            },
            Command::SetTextRenderingMode(3),
            text(10.0, 10.0, "hidden"),
            image(30.0, 10.0, 0.0, 10.0),
            end(),
        ]);
        let doc = document(vec![commands]);
        assert!(collect(&doc).pages[0].links.is_empty());
    }

    #[test]
    fn pdf_links_form_placements_keep_their_own_transforms_and_scope() {
        let mut form = begin("1", "https://example.invalid/reused");
        form.extend([image(0.0, 0.0, 10.0, 10.0), end()]);
        let commands = vec![
            Command::DefineForm {
                resource_id: "reusable".into(),
                width: p(10.0),
                height: p(10.0),
                commands: form,
            },
            Command::DrawForm {
                resource_id: "reusable".into(),
                x: p(10.0),
                y: p(10.0),
                width: p(20.0),
                height: p(20.0),
            },
            Command::DrawForm {
                resource_id: "reusable".into(),
                x: p(100.0),
                y: p(100.0),
                width: p(10.0),
                height: p(10.0),
            },
            text(40.0, 10.0, "outside"),
        ];
        let doc = document(vec![commands]);
        let result = collect(&doc);
        assert_eq!(result.pages[0].links.len(), 1);
        let link = &result.pages[0].links[0];
        assert!(contains(&doc, link, 15.0, 185.0));
        assert!(contains(&doc, link, 105.0, 95.0));
        assert!(!contains(&doc, link, 50.0, 180.0));
    }

    #[test]
    fn pdf_links_transparent_forms_keep_destinations_but_not_click_regions() {
        let mut form = vec![
            Command::SetOpacity {
                fill: 1.0,
                stroke: 1.0,
            },
            meta(META_DIAGNOSTIC_SCOPE_BEGIN_KEY, "target"),
            meta(META_DESTINATION, "inside"),
            meta(META_DESTINATION_POINT, "2000,3000"),
        ];
        form.extend(begin("2", "https://example.invalid/hidden"));
        form.extend([image(0.0, 0.0, 10.0, 10.0), end(), end()]);
        let mut commands = begin("1", "#inside");
        commands.extend([
            text(50.0, 10.0, "Go"),
            end(),
            Command::DefineForm {
                resource_id: "transparent".into(),
                width: p(10.0),
                height: p(10.0),
                commands: form,
            },
            Command::SetOpacity {
                fill: 0.0,
                stroke: 0.0,
            },
            Command::DrawForm {
                resource_id: "transparent".into(),
                x: p(10.0),
                y: p(10.0),
                width: p(20.0),
                height: p(20.0),
            },
        ]);
        let doc = document(vec![commands]);
        let result = collect(&doc);
        assert_eq!(result.pages[0].links.len(), 1);
        assert_eq!(result.pages[0].links[0].label, "Go");
        assert_eq!(
            result.destinations["inside"].1,
            mapped_point(&doc, 14.0, 184.0)
        );
    }

    #[test]
    fn pdf_links_compiled_overrides_use_the_final_recipient_text_and_position() {
        let mut commands = begin("1", "https://example.invalid/recipient");
        commands.extend([text(10.0, 10.0, "X"), end()]);
        let doc = document(vec![commands]);
        let overrides = vec![vec![CompiledFlowTextOverride {
            command: 3,
            transformed: false,
            x: p(40.0),
            x_pdf: Arc::from("40"),
            text: Arc::from("MMMM"),
            shaped: None,
        }]];
        let result = DocumentLinks::collect(&doc, doc.page_size, None, &overrides).unwrap();
        let link = &result.pages[0].links[0];
        assert_eq!(link.label, "MMMM");
        assert!(contains(&doc, link, 70.0, 180.0));
        assert!(!contains(&doc, link, 12.0, 180.0));
    }

    #[test]
    fn pdf_links_authored_box_includes_padding_and_explicit_accessible_name() {
        let mut commands = begin("1", "https://example.invalid/padded");
        commands.extend([
            meta(META_LINK_LABEL, "Read the full report"),
            meta(META_LINK_RECT, "10000,10000,100000,40000"),
            text(30.0, 20.0, "Read"),
            end(),
        ]);
        let doc = document(vec![commands]);
        let result = collect(&doc);
        let link = &result.pages[0].links[0];
        assert_eq!(link.label, "Read the full report");
        assert!(contains(&doc, link, 12.0, 188.0));
        assert!(!contains(&doc, link, 9.0, 188.0));
    }

    #[test]
    fn pdf_links_wrapped_html_keeps_one_source_across_page_fragments() {
        let html = format!(
            "<p><a href=\"https://example.invalid/report\">{}</a></p>",
            "read this report ".repeat(90)
        );
        let engine = crate::FullBleed::builder().build().unwrap();
        let compiled = engine.compile_document(&html, "@page { size: 180pt 180pt; margin: 12pt; } p { margin: 0; font-family: Helvetica; font-size: 12pt; line-height: 16pt; }").unwrap();
        let doc = &compiled.document;
        let result = collect(doc);
        assert!(result.pages.len() > 2);
        let id = &result.pages[0].links[0].source_id;
        for page in &result.pages {
            assert_eq!(page.links.len(), 1);
            assert_eq!(&page.links[0].source_id, id);
            assert!(page.links[0].region.quads.len() > 1);
        }
    }

    fn retain_pdf(name: &str, bytes: &[u8]) {
        if let Some(directory) = std::env::var_os("FULLBLEED_LINK_TEST_OUTPUT") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(format!("{name}.pdf")), bytes).unwrap();
        }
    }

    #[test]
    fn pdf_links_block_destinations_do_not_depend_on_painted_content() {
        for (suffix, profile) in [
            ("plain", crate::PdfProfile::None),
            ("ua2", crate::PdfProfile::PdfUa2),
        ] {
            let engine = crate::FullBleed::builder()
                .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
                .pdf_profile(profile)
                .document_lang("en-US")
                .document_title("Unpainted destination")
                .build()
                .unwrap();
            for (name, style) in [
                ("visible", ""),
                ("transparent", "opacity:0"),
                ("hidden", "visibility:hidden"),
                ("clipped", "clip-path:inset(100%)"),
                ("filtered", "filter:opacity(0)"),
                (
                    "masked",
                    "mask-image:linear-gradient(transparent,transparent)",
                ),
                ("removed", "display:none"),
            ] {
                let html = format!(
                    r##"<p><a href="#target">Go to target</a></p><p id="target" style="{style}">Target</p><p>After target</p>"##
                );
                let css = "@page{size:240pt 240pt;margin:20pt}body{font-family:'Noto Sans';font-size:12pt;line-height:18pt}p{margin:0}";
                let compiled = engine.compile_document(&html, css).unwrap();
                let links = collect(&compiled.document);
                assert_eq!(
                    links.destinations.contains_key("target"),
                    name != "removed",
                    "{name}-{suffix}"
                );
                if name != "removed" {
                    let (page, point) = links.destinations["target"];
                    assert_eq!(page, 0);
                    assert!((point.x - 20.0).abs() < 0.001, "{name}-{suffix}: {point:?}");
                    assert!(
                        (point.y - 202.0).abs() < 0.001,
                        "{name}-{suffix}: {point:?}"
                    );
                }
                let bytes = compiled.render_to_buffer().unwrap();
                let pdf = String::from_utf8_lossy(&bytes);
                assert_eq!(links.pages[0].links.len(), usize::from(name != "removed"));
                assert_eq!(pdf.contains("/Subtype /Link "), name != "removed");
                retain_pdf(&format!("block-destination-{name}-{suffix}"), &bytes);
            }
        }
    }

    #[test]
    fn pdf_links_empty_anchors_keep_positions_without_adding_content() {
        let css = "@page{size:240pt 240pt;margin:20pt}body{font-family:'Noto Sans';font-size:12pt;line-height:18pt}p,h2{margin:0;font-size:12pt;font-weight:normal}";
        for (suffix, profile) in [
            ("plain", crate::PdfProfile::None),
            ("ua2", crate::PdfProfile::PdfUa2),
        ] {
            let engine = crate::FullBleed::builder()
                .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
                .pdf_profile(profile)
                .document_lang("en-US")
                .document_title("Empty destinations")
                .build()
                .unwrap();
            for (name, body) in [
                (
                    "between-blocks",
                    "<p>Before</p><a id='target'></a><h2>After</h2>",
                ),
                (
                    "margins",
                    "<p style='margin-bottom:24pt'>Before</p><span id='target'></span><p style='margin-top:30pt'>After</p>",
                ),
                (
                    "leading-margin",
                    "<div><span id='target'></span><p style='margin-top:24pt'>After</p></div>",
                ),
                (
                    "trailing-margin",
                    "<div><p style='margin-bottom:24pt'>Before</p><span id='target'></span></div><p>After</p>",
                ),
                ("inline", "<p>Before<span id='target'></span> after</p>"),
                ("inline-space", "<p>Before <a name='target'></a> after</p>"),
                (
                    "whitespace",
                    "<p>Before <span id='target'> </span> after</p>",
                ),
                (
                    "empty-paragraph",
                    "<p>Before</p><p><span id='target'></span></p><p>After</p>",
                ),
                (
                    "nested",
                    "<p>Before<span><span id='target'></span></span> after</p>",
                ),
                (
                    "empty-link",
                    "<p>Before<a id='target' href='https://example.invalid/empty'></a> after</p>",
                ),
                (
                    "hard-break",
                    "<p>Before<br><span id='target'></span>After</p>",
                ),
                ("trailing", "<p>Before</p><span id='target'></span>"),
                (
                    "empty-block",
                    "<p>Before</p><div id='target'></div><p>After</p>",
                ),
                (
                    "positioned",
                    "<p>Before</p><span id='target' style='position:absolute;left:40pt;top:80pt'></span><p>After</p>",
                ),
                (
                    "page-boundary",
                    "<div style='height:182pt'>Before</div><a id='target'></a><p>After</p>",
                ),
                (
                    "hidden",
                    "<p>Before</p><a id='target' style='visibility:hidden'></a><p>After</p>",
                ),
                (
                    "display-none",
                    "<p>Before</p><a id='target' style='display:none'></a><p>After</p>",
                ),
            ] {
                let html = format!("<p><a href='#target'>Go</a></p>{body}");
                let compiled = engine.compile_document(&html, css).unwrap();
                let control = engine
                    .compile_document(
                        &html
                            .replace(" id='target'", "")
                            .replace(" name='target'", ""),
                        css,
                    )
                    .unwrap();
                assert_eq!(
                    compiled.page_count(),
                    control.page_count(),
                    "{name}-{suffix}"
                );
                if matches!(
                    name,
                    "empty-link" | "hard-break" | "margins" | "leading-margin" | "trailing-margin"
                ) {
                    let text_commands = |document: &Document| {
                        document
                            .pages
                            .iter()
                            .flat_map(|page| &page.commands)
                            .filter_map(|command| match command {
                                Command::DrawString { x, y, text } => Some((*x, *y, text.clone())),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(
                        text_commands(&compiled.document),
                        text_commands(&control.document)
                    );
                }
                let links = collect(&compiled.document);
                assert_eq!(
                    links.destinations.contains_key("target"),
                    name != "display-none",
                    "{name}-{suffix}"
                );
                let bytes = compiled.render_to_buffer().unwrap();
                retain_pdf(&format!("empty-destination-{name}-{suffix}"), &bytes);
                retain_pdf(
                    &format!("empty-destination-{name}-{suffix}-control"),
                    &control.render_to_buffer().unwrap(),
                );
                assert_eq!(
                    String::from_utf8_lossy(&bytes).contains("/Subtype /Link "),
                    name != "display-none",
                    "{name}-{suffix}"
                );
            }
        }
    }

    #[test]
    fn pdf_links_writer_emits_actions_and_tagged_annotation_ownership() {
        for (name, profile) in [
            ("plain", crate::PdfProfile::None),
            ("tagged", crate::PdfProfile::Tagged),
            ("ua1", crate::PdfProfile::PdfUa1),
            ("ua2", crate::PdfProfile::PdfUa2),
        ] {
            let engine = crate::FullBleed::builder()
                .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
                .pdf_profile(profile)
                .document_lang("en-US")
                .document_title("Authored links")
                .build()
                .unwrap();
            let html = format!(
                r##"<p><a href="https://example.invalid/café?q=✓" aria-label="Read café report">{}</a></p><p><a href="#details">Details</a></p><h1 id="details">Details</h1><p>Ordinary text</p>"##,
                "Read the full report. ".repeat(36)
            );
            let css = "@page{size:200pt 200pt;margin:16pt}body{font-family:'Noto Sans';font-size:12pt;line-height:16pt}p{margin:0 0 12pt}";
            let compiled = engine.compile_document(&html, css).unwrap();
            let bytes = compiled.render_to_buffer().unwrap();
            let pdf = String::from_utf8_lossy(&bytes);
            assert!(pdf.contains("/Subtype /Link"), "{name}");
            assert!(pdf.contains("/URI (https://example.invalid/caf%C3%A9?q=%E2%9C%93)"));
            if profile == crate::PdfProfile::PdfUa2 {
                assert!(pdf.contains(" /S /GoTo /D ["));
                assert!(pdf.contains(" /SD ["));
                assert!(!pdf.contains(" /Dest ["));
            } else {
                assert!(pdf.contains(" /Dest ["));
            }
            if profile != crate::PdfProfile::None {
                assert_eq!(
                    pdf.matches("/S /Link ").count(),
                    2,
                    "one logical owner per authored link: {name}"
                );
                assert_eq!(
                    pdf.matches("/Type /OBJR ").count(),
                    pdf.matches("/Subtype /Link ").count()
                );
                assert_eq!(
                    pdf.matches(" /StructParent ").count(),
                    pdf.matches("/Subtype /Link ").count()
                );
            }
            retain_pdf(name, &bytes);
            let copies = compiled.render_many_to_buffer(2).unwrap();
            assert_eq!(
                String::from_utf8_lossy(&copies)
                    .matches("/Subtype /Link ")
                    .count(),
                2 * pdf.matches("/Subtype /Link ").count()
            );
            retain_pdf(&format!("{name}-copies"), &copies);
            assert_eq!(
                bytes,
                compiled.render_to_buffer().unwrap(),
                "deterministic {name}"
            );
        }
    }

    #[test]
    fn pdf_links_structure_destinations_keep_source_and_copy_identity() {
        let engine = crate::FullBleed::builder()
            .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
            .pdf_profile(crate::PdfProfile::PdfUa2)
            .document_lang("en-US")
            .document_title("Structure destinations")
            .build()
            .unwrap();
        let compiled = engine.compile_document(
            r##"<h1>Opening</h1><p><a href="#details">Details</a></p><p><a href="#">Top</a></p><h2 id="details">Target heading</h2><p id="details">Duplicate target must not win</p>"##,
            "@page{size:300pt 300pt;margin:16pt}body{font-family:'Noto Sans';font-size:12pt;line-height:16pt}p{margin:0 0 12pt}",
        ).unwrap();
        let bytes = compiled.render_many_to_buffer(2).unwrap();
        assert_eq!(String::from_utf8_lossy(&bytes).matches(" /SD [").count(), 4);
        retain_pdf("ua2-destinations-copies", &bytes);
    }

    #[test]
    fn pdf_links_table_destinations_follow_rows_cells_groups_and_page_fragments() {
        let names = [
            "head",
            "head-row",
            "head-cell",
            "body",
            "body-row",
            "body-cell",
            "foot",
            "foot-row",
            "foot-cell",
        ];
        let navigation = names
            .iter()
            .chain(["hidden-row", "hidden-cell"].iter())
            .map(|name| format!(r##"<a href="#{name}">{name}</a> "##))
            .collect::<String>();
        let html = format!(
            r##"<h1>Table navigation</h1><p>{navigation}</p>
            <table><thead id="head"><tr id="head-row"><th id="head-cell">Item</th><th>Value</th></tr></thead>
            <tbody id="body"><tr><td rowspan="2">Span</td><td>First</td></tr>
            <tr id="body-row"><td id="body-cell">Second</td></tr>
            <tr id="hidden-row" style="visibility:collapse"><td>Collapsed</td><td>Row</td></tr>
            <tr><td id="hidden-cell" style="display:none">Absent</td><td colspan="2">Visible</td></tr>{}</tbody>
            <tfoot id="foot"><tr id="foot-row"><td id="foot-cell">Total</td><td>42</td></tr></tfoot></table>"##,
            "<tr><td>Additional</td><td>Line</td></tr>".repeat(28)
        );
        let css = "@page{size:300pt 240pt;margin:16pt}body{font-family:'Noto Sans';font-size:10pt;line-height:14pt}h1{font-size:14pt;line-height:18pt;font-weight:normal;margin:0}p{margin:0 0 8pt}table{width:230pt;border-collapse:collapse}th{font-weight:normal}td,th{padding:3pt;border:1pt solid #555}";
        for (name, profile) in [
            ("plain", crate::PdfProfile::None),
            ("ua1", crate::PdfProfile::PdfUa1),
            ("ua2", crate::PdfProfile::PdfUa2),
        ] {
            let engine = crate::FullBleed::builder()
                .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
                .pdf_profile(profile)
                .document_lang("en-US")
                .document_title("Table navigation")
                .build()
                .unwrap();
            let compiled = engine.compile_document(&html, css).unwrap();
            let links = DocumentLinks::collect(
                &compiled.document,
                compiled.document.page_size,
                Some(&compiled.font_registry),
                &[],
            )
            .unwrap();
            assert!(links.pages.len() >= 3);
            assert_eq!(
                links
                    .pages
                    .iter()
                    .map(|page| page.links.len())
                    .sum::<usize>(),
                names.len(),
                "{name}"
            );
            assert!(!links.destinations.contains_key("hidden-row"));
            assert!(!links.destinations.contains_key("hidden-cell"));
            for target in names {
                assert!(links.destinations.contains_key(target), "{name}: {target}");
            }
            for target in ["head", "head-row", "head-cell"] {
                assert_eq!(links.destinations[target].0, 0, "{target}");
            }
            for target in ["foot", "foot-row", "foot-cell"] {
                assert_eq!(
                    links.destinations[target].0,
                    links.pages.len() - 1,
                    "{target}"
                );
            }
            let row = links.destinations["body-row"];
            let cell = links.destinations["body-cell"];
            assert_eq!(row.0, cell.0);
            assert_eq!(row.1.y, cell.1.y);
            assert!(
                cell.1.x > row.1.x + 50.0,
                "rowspan placeholder must retain the new row, not the spanning cell's identity"
            );
            let untargeted = names.iter().fold(html.clone(), |source, target| {
                source.replace(&format!(" id=\"{target}\""), "")
            });
            let baseline = engine.compile_document(&untargeted, css).unwrap();
            let paint = |document: &Document| {
                document
                    .pages
                    .iter()
                    .map(|page| {
                        page.commands
                            .iter()
                            .filter(|command| {
                                !matches!(
                                    command,
                                    Command::Meta { .. }
                                        | Command::BeginTag { .. }
                                        | Command::BeginTagActualText { .. }
                                        | Command::EndTag
                                )
                            })
                            .map(|command| format!("{command:?}"))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                paint(&compiled.document),
                paint(&baseline.document),
                "destinations must not change layout or paint"
            );
            let bytes = compiled.render_to_buffer().unwrap();
            assert_eq!(bytes, compiled.render_to_buffer().unwrap());
            if profile == crate::PdfProfile::PdfUa2 {
                assert_eq!(
                    String::from_utf8_lossy(&bytes).matches(" /SD [").count(),
                    names.len()
                );
            }
            retain_pdf(&format!("table-destinations-{name}"), &bytes);
            retain_pdf(
                &format!("table-destinations-{name}-copies"),
                &compiled.render_many_to_buffer(2).unwrap(),
            );
        }
    }

    #[test]
    fn pdf_links_table_destination_points_follow_transforms_and_opacity() {
        let engine = crate::FullBleed::builder().build().unwrap();
        let html = r##"<p><a href="#row">Row</a> <a href="#cell">Cell</a></p><div><table><tr id="row"><td id="cell">Target</td></tr></table></div>"##;
        let css = "@page{size:300pt 300pt;margin:16pt}body{font-family:Helvetica;font-size:12pt;line-height:16pt}p{margin:0}div{width:120pt}table{width:100pt}td{padding:4pt}";
        let plain = collect(&engine.compile_document(html, css).unwrap().document);
        assert_eq!(plain.pages[0].links.len(), 2);
        let transformed_document = engine
            .compile_document(
                html,
                &format!("{css}div{{transform:translate(13pt,9pt);opacity:0.6}}"),
            )
            .unwrap();
        retain_pdf(
            "table-destination-transform",
            &transformed_document.render_to_buffer().unwrap(),
        );
        let transformed = collect(&transformed_document.document);
        assert_eq!(transformed.pages[0].links.len(), 2);
        for name in ["row", "cell"] {
            assert!(
                (transformed.destinations[name].1.x - plain.destinations[name].1.x - 13.0).abs()
                    < 0.01,
                "{name}: plain {:?}, transformed {:?}",
                plain.destinations[name],
                transformed.destinations[name]
            );
            assert!(
                (transformed.destinations[name].1.y - plain.destinations[name].1.y + 9.0).abs()
                    < 0.01,
                "{name}: plain {:?}, transformed {:?}",
                plain.destinations[name],
                transformed.destinations[name]
            );
        }
        for effect in [
            "opacity:0",
            "visibility:hidden",
            "height:0;overflow:hidden",
            "clip-path:inset(100%)",
        ] {
            let hidden = collect(
                &engine
                    .compile_document(html, &format!("{css}div{{{effect}}}"))
                    .unwrap()
                    .document,
            );
            assert_eq!(hidden.pages[0].links.len(), 2, "{effect}");
            for name in ["row", "cell"] {
                assert_eq!(
                    hidden.destinations[name], plain.destinations[name],
                    "{effect}: {name}"
                );
            }
        }
    }

    #[test]
    fn pdf_links_writer_retains_actions_in_fixed_and_reflow_binding_batches() {
        let engine = crate::FullBleed::builder().build().unwrap();
        let template = r##"<p><a href="https://example.invalid/recipient">{{name}}</a></p><p><a href="#end">Jump</a></p><p id="end">End</p>"##;
        let css = "@page{size:220pt 220pt;margin:16pt}body{font-family:Helvetica;font-size:12pt;line-height:16pt}p{margin:0 0 12pt}";
        let compiled = engine.compile_document(template, css).unwrap();
        let bindings = HashMap::from([(
            "name".to_string(),
            vec!["Ada".to_string(), "Grace".to_string()],
        )]);
        for (name, bytes) in [
            (
                "fixed-bindings",
                compiled.render_bindings_to_buffer(&bindings).unwrap(),
            ),
            (
                "reflow-bindings",
                compiled
                    .render_reflow_bindings_to_buffer(&bindings)
                    .unwrap(),
            ),
        ] {
            let pdf = String::from_utf8_lossy(&bytes);
            assert_eq!(pdf.matches("/Subtype /Link ").count(), 4, "{name}");
            retain_pdf(name, &bytes);
        }
    }

    #[test]
    fn pdf_links_svg_images_keep_their_viewport_in_vector_and_form_output() {
        for (name, form) in [("svg-link", false), ("svg-form-link", true)] {
            for (suffix, profile) in [
                ("plain", crate::PdfProfile::None),
                ("ua2", crate::PdfProfile::PdfUa2),
            ] {
                let engine = crate::FullBleed::builder()
                    .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
                    .svg_form_xobjects(form)
                    .pdf_profile(profile)
                    .document_lang("en-US")
                    .document_title("Linked SVG image")
                    .build()
                    .unwrap();
                // Most of the SVG viewport is empty. The HTML image box, not
                // only its painted green rectangle, belongs to the link.
                let html = r#"<h1>Documents</h1><a href="https://example.invalid/documents"><img alt="Open documents" width="40" height="20" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='40' height='20'%3E%3Crect width='10' height='10' fill='green'/%3E%3C/svg%3E"></a>"#;
                let css = "@page{size:200pt 200pt;margin:16pt}body{font-family:'Noto Sans';font-size:12pt;line-height:16pt}h1{font-size:16pt;font-weight:normal;margin:0 0 12pt}";
                let compiled = engine.compile_document(html, css).unwrap();
                let result = collect(&compiled.document);
                assert_eq!(result.pages[0].links.len(), 1, "{name}-{suffix}");
                let link = &result.pages[0].links[0];
                assert_eq!(link.label, "Open documents");
                let [left, bottom, right, top] = link.region.bounds().unwrap();
                assert!((right - left - 30.0).abs() < 0.001);
                assert!((top - bottom - 15.0).abs() < 0.001);
                assert!(
                    link.region
                        .quads
                        .iter()
                        .any(|q| q.contains(Point::new(right - 1.0, bottom + 1.0)))
                );
                let bytes = compiled.render_to_buffer().unwrap();
                assert!(String::from_utf8_lossy(&bytes).contains("/Subtype /Link"));
                retain_pdf(&format!("{name}-{suffix}"), &bytes);
            }
        }
    }

    #[test]
    fn pdf_links_svg_fragments_keep_only_the_visible_replaced_viewport() {
        use crate::flowable::{Flowable, SvgFlowable};
        let svg = SvgFlowable::new(
            100.0,
            300.0,
            "<svg width='100' height='300'><rect width='100' height='300' fill='green'/></svg>",
        );
        let (first, second) = svg.split(p(100.0), p(120.0)).unwrap();
        let mut canvas = crate::Canvas::new(Size {
            width: p(200.0),
            height: p(200.0),
        });
        for (flowable, height) in [(first, 120.0), (second, 180.0)] {
            canvas.meta(META_DIAGNOSTIC_SCOPE_BEGIN_KEY, "image-link");
            canvas.meta(META_LINK_ID, "1");
            canvas.meta(META_LINK_TARGET, "https://example.invalid/image");
            flowable.draw(&mut canvas, p(10.0), p(10.0), p(100.0), p(height));
            canvas.meta(META_DIAGNOSTIC_SCOPE_END_KEY, "image-link");
            canvas.show_page();
        }
        let document = canvas.finish_without_show();
        let links = collect(&document);
        assert_eq!(links.pages.len(), 2);
        for (page, expected_height) in links.pages.iter().zip([120.0, 180.0]) {
            assert_eq!(page.links.len(), 1);
            let [left, bottom, right, top] = page.links[0].region.bounds().unwrap();
            assert!((left - 10.0).abs() < 0.001);
            assert!((right - 110.0).abs() < 0.001);
            assert!((top - 190.0).abs() < 0.001);
            assert!((top - bottom - expected_height).abs() < 0.001);
        }
    }

    #[test]
    fn pdf_links_text_shadows_do_not_repeat_labels_or_expand_activation() {
        let engine = crate::FullBleed::builder()
            .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
            .pdf_profile(crate::PdfProfile::PdfUa2)
            .document_lang("en-US")
            .document_title("Linked text shadow")
            .build()
            .unwrap();
        let html = r#"<p><a href="https://example.invalid/report">Read</a></p>"#;
        let base = "@page{size:200pt 200pt;margin:16pt}body{font-family:'Noto Sans';font-size:12pt;line-height:16pt}p{margin:0}";
        // Use the same embedded-font metrics as the PDF writer. The base-14
        // helper cannot measure a Noto Sans text run.
        let navigation = |document: &Document| {
            DocumentLinks::collect(
                document,
                document.page_size,
                Some(engine.font_registry.as_ref()),
                &[],
            )
            .unwrap()
        };
        let normal = engine.compile_document(html, base).unwrap();
        let normal_links = navigation(&normal.document);
        assert_eq!(normal_links.pages[0].links.len(), 1);
        let expected = normal_links.pages[0].links[0].region.clone();
        for (name, shadow) in [
            ("sharp", "40pt 35pt #aaa"),
            ("blurred", "40pt 35pt 4pt #aaa"),
            ("multiple", "40pt 35pt #aaa,-10pt -5pt 3pt #777"),
        ] {
            let compiled = engine
                .compile_document(html, &format!("{base}a{{text-shadow:{shadow}}}"))
                .unwrap();
            let links = navigation(&compiled.document);
            assert_eq!(links.pages[0].links.len(), 1, "{name}");
            assert_eq!(links.pages[0].links[0].label, "Read", "{name}");
            assert_eq!(
                links.pages[0].links[0].region.quads, expected.quads,
                "{name}"
            );
            retain_pdf(
                &format!("shadow-link-{name}-ua2"),
                &compiled.render_to_buffer().unwrap(),
            );
        }
    }

    #[test]
    fn pdf_links_html_padding_and_effect_forms_preserve_the_authored_click_region() {
        let engine = crate::FullBleed::builder().build().unwrap();
        let base = "@page{size:200pt 200pt;margin:0}body{margin:0;font-family:Helvetica;font-size:12pt}a{display:block;width:100pt;height:100pt;box-sizing:border-box;padding:10pt;}";
        let html = r#"<a href="https://example.invalid/geometry">Read</a>"#;
        for (name, extra) in [
            ("geometry-padded", ""),
            ("geometry-opacity", "a{opacity:0.6}"),
            (
                "geometry-clipped",
                "a{clip-path:polygon(0 0,100% 0,100% 40%,40% 40%,40% 100%,0 100%)}",
            ),
        ] {
            let compiled = engine
                .compile_document(html, &format!("{base}{extra}"))
                .unwrap();
            let doc = &compiled.document;
            let result = collect(doc);
            assert_eq!(result.pages[0].links.len(), 1, "{name}");
            let link = &result.pages[0].links[0];
            assert!(contains(doc, link, 5.0, 195.0), "padding: {name}");
            assert!(!contains(doc, link, 110.0, 190.0), "outside: {name}");
            assert_eq!(
                contains(doc, link, 80.0, 120.0),
                name != "geometry-clipped",
                "clipping: {name}"
            );
            retain_pdf(name, &compiled.render_to_buffer().unwrap());
        }
    }
}
