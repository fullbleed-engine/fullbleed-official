//! Conservative PDF-space paint bounds for explicitly managed page furniture.
//!
//! This is not layout. It consumes the final display list, including authored
//! clips, and deliberately refuses effects whose extent it cannot establish.

use crate::canvas::Command;
use crate::font::{FontRegistry, GlyphOutlineCommand};
use crate::pdf::PdfOptions;
use crate::types::{Pt, Size};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bounds {
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub top: f64,
}

impl Bounds {
    fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            bottom: self.bottom.min(other.bottom),
            right: self.right.max(other.right),
            top: self.top.max(other.top),
        }
    }

    fn intersect(self, other: Self) -> Option<Self> {
        let result = Self {
            left: self.left.max(other.left),
            bottom: self.bottom.max(other.bottom),
            right: self.right.min(other.right),
            top: self.top.min(other.top),
        };
        (result.left <= result.right && result.bottom <= result.top).then_some(result)
    }

    fn finite(self) -> bool {
        [self.left, self.bottom, self.right, self.top]
            .iter()
            .all(|n| n.is_finite())
    }

    pub(crate) fn fits(self, width: Pt, height: Pt) -> bool {
        // Match layout's one-millipoint tolerance. Bounds themselves are rounded
        // outwards, so this is a declared tolerance rather than an inward snap.
        self.finite()
            && self.left >= -0.001
            && self.bottom >= -0.001
            && self.right <= number(width).hi + 0.001
            && self.top <= number(height).hi + 0.001
    }

    pub(crate) fn page_rect(self, height: Pt) -> [i64; 4] {
        let h = number(height);
        [
            self.left.floor_milli(),
            (h.lo - self.top).floor_milli(),
            self.right.ceil_milli(),
            (h.hi - self.bottom).ceil_milli(),
        ]
    }
}

trait Milli {
    fn floor_milli(self) -> i64;
    fn ceil_milli(self) -> i64;
}
impl Milli for f64 {
    fn floor_milli(self) -> i64 {
        (self * 1000.0).floor() as i64
    }
    fn ceil_milli(self) -> i64 {
        (self * 1000.0).ceil() as i64
    }
}

// Directed rounding encloses arithmetic on serialized PDF numbers. Do not use
// a raster sampling window to decide that a transformed object fits.
fn next_up(value: f64) -> f64 {
    if value.is_nan() || value == f64::INFINITY {
        return value;
    }
    if value == 0.0 {
        return f64::from_bits(1);
    }
    f64::from_bits(if value > 0.0 {
        value.to_bits() + 1
    } else {
        value.to_bits() - 1
    })
}
fn next_down(value: f64) -> f64 {
    -next_up(-value)
}

#[derive(Clone, Copy, Debug)]
struct Span {
    lo: f64,
    hi: f64,
}
impl Span {
    const fn exact(value: f64) -> Self {
        Self {
            lo: value,
            hi: value,
        }
    }
    fn enclosing(value: f64) -> Self {
        Self {
            lo: next_down(value),
            hi: next_up(value),
        }
    }
    fn add(self, rhs: Self) -> Self {
        let (lo, hi) = (self.lo + rhs.lo, self.hi + rhs.hi);
        if lo.is_nan() || hi.is_nan() {
            return Self {
                lo: f64::NEG_INFINITY,
                hi: f64::INFINITY,
            };
        }
        Self {
            lo: next_down(lo),
            hi: next_up(hi),
        }
    }
    fn neg(self) -> Self {
        Self {
            lo: -self.hi,
            hi: -self.lo,
        }
    }
    fn mul(self, rhs: Self) -> Self {
        let terms = [
            self.lo * rhs.lo,
            self.lo * rhs.hi,
            self.hi * rhs.lo,
            self.hi * rhs.hi,
        ];
        if terms.iter().any(|value| value.is_nan()) {
            return Self {
                lo: f64::NEG_INFINITY,
                hi: f64::INFINITY,
            };
        }
        Self {
            lo: next_down(terms.iter().copied().fold(f64::INFINITY, f64::min)),
            hi: next_up(terms.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
        }
    }
    fn magnitude(self) -> f64 {
        self.lo.abs().max(self.hi.abs())
    }
}
fn number(value: Pt) -> Span {
    Span::enclosing(value.to_milli_i64() as f64).mul(Span::enclosing(0.001))
}
fn real(value: f32) -> Span {
    Span::enclosing(crate::pdf::serialized_real(value))
}

#[derive(Clone, Copy, Debug)]
struct Matrix([Span; 6]);
impl Matrix {
    const IDENTITY: Self = Self([
        Span::exact(1.0),
        Span::exact(0.0),
        Span::exact(0.0),
        Span::exact(1.0),
        Span::exact(0.0),
        Span::exact(0.0),
    ]);
    fn then(self, rhs: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [g, h, i, j, k, l] = rhs.0;
        Self([
            a.mul(g).add(c.mul(h)),
            b.mul(g).add(d.mul(h)),
            a.mul(i).add(c.mul(j)),
            b.mul(i).add(d.mul(j)),
            a.mul(k).add(c.mul(l)).add(e),
            b.mul(k).add(d.mul(l)).add(f),
        ])
    }
    fn point(self, x: Span, y: Span) -> Bounds {
        let [a, b, c, d, e, f] = self.0;
        let x1 = a.mul(x).add(c.mul(y)).add(e);
        let y1 = b.mul(x).add(d.mul(y)).add(f);
        Bounds {
            left: x1.lo,
            bottom: y1.lo,
            right: x1.hi,
            top: y1.hi,
        }
    }
    fn bounds(self, bounds: Bounds) -> Bounds {
        let mut result = self.point(Span::enclosing(bounds.left), Span::enclosing(bounds.bottom));
        for (x, y) in [
            (bounds.left, bounds.top),
            (bounds.right, bounds.bottom),
            (bounds.right, bounds.top),
        ] {
            result = result.union(self.point(Span::enclosing(x), Span::enclosing(y)));
        }
        result
    }
    fn translate(x: Span, y: Span) -> Self {
        let mut matrix = Self::IDENTITY;
        matrix.0[4] = x;
        matrix.0[5] = y;
        matrix
    }
}

#[derive(Clone, Copy)]
enum Clip {
    Infinite,
    Empty,
    Finite(Bounds),
}
impl Clip {
    fn intersect(self, bounds: Option<Bounds>) -> Self {
        match (self, bounds) {
            (Self::Empty, _) | (_, None) => Self::Empty,
            (Self::Infinite, Some(b)) => Self::Finite(b),
            (Self::Finite(a), Some(b)) => a.intersect(b).map_or(Self::Empty, Self::Finite),
        }
    }
    fn apply(self, bounds: Bounds) -> Option<Bounds> {
        match self {
            Self::Infinite => Some(bounds),
            Self::Empty => None,
            Self::Finite(b) => b.intersect(bounds),
        }
    }
}

#[derive(Clone)]
struct State {
    matrix: Matrix,
    clip: Clip,
    width: Pt,
    cap: u8,
    join: u8,
    miter: Pt,
    font: String,
    font_size: Pt,
    text_mode: u8,
    fill: bool,
    stroke: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            matrix: Matrix::IDENTITY,
            clip: Clip::Infinite,
            width: Pt::from_i32(1),
            cap: 0,
            join: 0,
            miter: Pt::from_i32(10),
            font: "Helvetica".into(),
            font_size: Pt::from_i32(12),
            text_mode: 0,
            fill: true,
            stroke: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unknown {
    pub reason: &'static str,
    pub command: usize,
    pub forms: Vec<String>,
}

#[derive(Clone)]
struct Form {
    size: Size,
    commands: Vec<Command>,
    identity: String,
    isolated: bool,
}

struct Evaluator<'a> {
    registry: &'a FontRegistry,
    options: &'a PdfOptions,
    forms: HashMap<String, Form>,
    active: Vec<String>,
    remaining: usize,
}

pub(crate) fn measure(
    commands: &[Command],
    size: Size,
    registry: &FontRegistry,
    options: &PdfOptions,
) -> Result<Option<Bounds>, Unknown> {
    Evaluator {
        registry,
        options,
        forms: HashMap::new(),
        active: Vec::new(),
        remaining: 100_000,
    }
    .program(commands, size, State::default())
}

impl Evaluator<'_> {
    fn error(&self, reason: &'static str, command: usize) -> Unknown {
        Unknown {
            reason,
            command,
            forms: self.active.clone(),
        }
    }

    fn program(
        &mut self,
        commands: &[Command],
        size: Size,
        mut state: State,
    ) -> Result<Option<Bounds>, Unknown> {
        let mut stack = Vec::new();
        let mut path: Option<Bounds> = None;
        let mut painted: Option<Bounds> = None;
        for (index, command) in commands.iter().enumerate() {
            if self.remaining == 0 {
                return Err(self.error("command_budget", index));
            }
            self.remaining -= 1;
            let mut ink = None;
            let mut paint_path = None;
            match command {
                Command::SaveState => stack.push(state.clone()),
                Command::RestoreState => {
                    state = stack
                        .pop()
                        .ok_or_else(|| self.error("graphics_state_underflow", index))?
                }
                Command::Translate(x, y) => {
                    state.matrix = state
                        .matrix
                        .then(Matrix::translate(number(*x), number(-*y)))
                }
                Command::CssTransformOrigin { x, y, inverse } => {
                    let (x, y) = if *inverse {
                        (-*x, -(size.height - *y))
                    } else {
                        (*x, size.height - *y)
                    };
                    state.matrix = state.matrix.then(Matrix::translate(number(x), number(y)));
                }
                Command::Scale(x, y) => {
                    if !x.is_finite() || !y.is_finite() {
                        return Err(self.error("nonfinite_transform", index));
                    }
                    state.matrix = state.matrix.then(Matrix([
                        real(*x),
                        Span::exact(0.0),
                        Span::exact(0.0),
                        real(*y),
                        Span::exact(0.0),
                        Span::exact(0.0),
                    ]));
                }
                Command::Rotate(angle) => {
                    if !angle.is_finite() {
                        return Err(self.error("nonfinite_transform", index));
                    }
                    let (sin, cos) = crate::math::sin_cos(-*angle);
                    state.matrix = state.matrix.then(Matrix([
                        real(cos),
                        real(sin),
                        real(-sin),
                        real(cos),
                        Span::exact(0.0),
                        Span::exact(0.0),
                    ]));
                }
                Command::ConcatMatrix { a, b, c, d, e, f } => {
                    if ![a, b, c, d].iter().all(|n| n.is_finite()) {
                        return Err(self.error("nonfinite_transform", index));
                    }
                    state.matrix = state.matrix.then(Matrix([
                        real(*a),
                        real(-*b),
                        real(-*c),
                        real(*d),
                        number(*e),
                        number(-*f),
                    ]));
                }
                Command::SetLineWidth(width) => state.width = *width,
                Command::SetLineCap(cap) => state.cap = *cap,
                Command::SetLineJoin(join) => state.join = *join,
                Command::SetMiterLimit(limit) => state.miter = *limit,
                Command::SetFontName(name) => state.font = name.clone(),
                Command::SetFontSize(font_size) => state.font_size = *font_size,
                Command::SetTextRenderingMode(mode) => state.text_mode = (*mode).min(7),
                Command::SetOpacity { fill, stroke } => {
                    if !fill.is_finite() || !stroke.is_finite() {
                        return Err(self.error("nonfinite_opacity", index));
                    }
                    state.fill = ((*fill * 1000.0).round() as i32).clamp(0, 1000) != 0;
                    state.stroke = ((*stroke * 1000.0).round() as i32).clamp(0, 1000) != 0;
                }
                Command::MoveTo { x, y } | Command::LineTo { x, y } => {
                    include(
                        &mut path,
                        state.matrix.point(number(*x), number(size.height - *y)),
                    );
                }
                Command::CurveTo {
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                } => {
                    for (x, y) in [(*x1, *y1), (*x2, *y2), (*x, *y)] {
                        include(
                            &mut path,
                            state.matrix.point(number(x), number(size.height - y)),
                        );
                    }
                }
                Command::ClosePath => {}
                Command::ClipRect {
                    x,
                    y,
                    width,
                    height,
                } => {
                    include(
                        &mut path,
                        rectangle(
                            state.matrix,
                            *x,
                            size.height - *y - *height,
                            *width,
                            *height,
                        ),
                    );
                    state.clip = state.clip.intersect(path.take());
                }
                Command::ClipPath { .. } => state.clip = state.clip.intersect(path.take()),
                Command::Fill | Command::FillEvenOdd => paint_path = Some((true, false)),
                Command::Stroke => paint_path = Some((false, true)),
                Command::FillStroke | Command::FillStrokeEvenOdd => paint_path = Some((true, true)),
                Command::DrawRect {
                    x,
                    y,
                    width,
                    height,
                } => {
                    include(
                        &mut path,
                        rectangle(
                            state.matrix,
                            *x,
                            size.height - *y - *height,
                            *width,
                            *height,
                        ),
                    );
                    paint_path = Some((true, false));
                }
                Command::ShadingFill(_) => {
                    if state.fill {
                        match state.clip {
                            Clip::Finite(b) => ink = Some(b),
                            Clip::Empty => {}
                            Clip::Infinite => return Err(self.error("unbounded_shading", index)),
                        }
                    }
                }
                Command::DrawImage {
                    x,
                    y,
                    width,
                    height,
                    source_clip,
                    ..
                } => {
                    if source_clip.is_some() {
                        return Err(self.error("cropped_image_extent", index));
                    }
                    if state.fill {
                        ink = Some(rectangle(
                            state.matrix,
                            *x,
                            size.height - *y - *height,
                            *width,
                            *height,
                        ));
                    }
                }
                Command::DefineForm {
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
                } => {
                    let isolated = matches!(command, Command::DefineIsolatedForm { .. });
                    let identity = format!("{isolated}:{width:?}:{height:?}:{commands:?}");
                    if self
                        .forms
                        .get(resource_id)
                        .is_some_and(|old| old.identity != identity)
                    {
                        return Err(self.error("resource_redefinition", index));
                    }
                    self.forms.insert(
                        resource_id.clone(),
                        Form {
                            size: Size {
                                width: *width,
                                height: *height,
                            },
                            commands: commands.clone(),
                            identity,
                            isolated,
                        },
                    );
                }
                Command::DrawForm {
                    x,
                    y,
                    width,
                    height,
                    resource_id,
                } => {
                    if self.active.len() >= 64 || self.active.contains(resource_id) {
                        return Err(self.error("recursive_form", index));
                    }
                    let form = self
                        .forms
                        .get(resource_id)
                        .cloned()
                        .ok_or_else(|| self.error("missing_form", index))?;
                    if form.size.width <= Pt::ZERO || form.size.height <= Pt::ZERO {
                        return Err(self.error("degenerate_form", index));
                    }
                    self.active.push(resource_id.clone());
                    let initial = State {
                        matrix: Matrix::IDENTITY,
                        clip: Clip::Infinite,
                        font: "Helvetica".into(),
                        font_size: Pt::from_i32(12),
                        // Transparency groups reset alpha before painting their
                        // contents. Inherited zero stroke alpha must not make a
                        // potentially visible group look empty to the checker.
                        fill: form.isolated || state.fill,
                        stroke: form.isolated || state.stroke,
                        ..state.clone()
                    };
                    let child = self.program(&form.commands, form.size, initial)?;
                    if child.is_some_and(|bounds| !bounds.fits(form.size.width, form.size.height)) {
                        return Err(self.error("nested_form_clip", index));
                    }
                    self.active.pop();
                    if let Some(child) = child {
                        let sx = width.to_f32() / form.size.width.to_f32();
                        let sy = height.to_f32() / form.size.height.to_f32();
                        let mapping = Matrix([
                            real(sx),
                            Span::exact(0.0),
                            Span::exact(0.0),
                            real(sy),
                            number(*x),
                            number(size.height - *y - *height),
                        ]);
                        ink = Some(state.matrix.then(mapping).bounds(child));
                    }
                }
                Command::DrawString { x, y, text } => {
                    let transform = Matrix([
                        number(state.font_size),
                        Span::exact(0.0),
                        Span::exact(0.0),
                        number(state.font_size),
                        number(*x),
                        number(size.height - *y - state.font_size),
                    ]);
                    ink = self.text(&state, text, transform, index)?;
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
                    let scale = number(state.font_size);
                    let transform = Matrix([
                        real(*m00).mul(scale),
                        real(*m01).mul(scale),
                        real(*m10).mul(scale),
                        real(*m11).mul(scale),
                        number(*x),
                        number(*y),
                    ]);
                    ink = self.text(&state, text, transform, index)?;
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
                } => {
                    if state.text_mode == 3 {
                        continue;
                    }
                    if state.text_mode >= 4 {
                        return Err(self.error("text_clip", index));
                    }
                    let mut pen_x = *x;
                    let mut pen_y = size.height - *y;
                    let scale = number(state.font_size);
                    for (i, gid) in glyph_ids.iter().enumerate() {
                        let transform = Matrix([
                            real(*m00).mul(scale),
                            real(*m01).mul(scale),
                            real(*m10).mul(scale),
                            real(*m11).mul(scale),
                            number(pen_x),
                            number(pen_y),
                        ]);
                        if let Some(bbox) =
                            crate::pdf::type3_glyph_paint_bounds(self.registry, &state.font, *gid)
                        {
                            let scale = Span::enclosing(0.001);
                            let font_matrix = Matrix([
                                scale,
                                Span::exact(0.0),
                                Span::exact(0.0),
                                scale.neg(),
                                Span::exact(0.0),
                                Span::exact(0.0),
                            ]);
                            include(
                                &mut ink,
                                state
                                    .matrix
                                    .then(transform)
                                    .then(font_matrix)
                                    .bounds(Bounds {
                                        left: bbox[0],
                                        bottom: bbox[1],
                                        right: bbox[2],
                                        top: bbox[3],
                                    }),
                            );
                        }
                        if let Some((dx, dy)) = advances.get(i) {
                            pen_x = pen_x + *dx;
                            pen_y = pen_y + *dy;
                        }
                    }
                }
                Command::DrawSyntheticBoldGlyphRun {
                    x,
                    y,
                    glyph_ids,
                    advances,
                    offsets,
                    stroke_width,
                } => {
                    if state.text_mode == 3 {
                        continue;
                    }
                    if state.text_mode >= 4 {
                        return Err(self.error("text_clip", index));
                    }
                    let mut pen_x = *x;
                    let mut pen_y = *y;
                    for (i, gid) in glyph_ids.iter().enumerate() {
                        let offset = offsets.get(i).copied().unwrap_or((Pt::ZERO, Pt::ZERO));
                        if let Some(bbox) = crate::pdf::synthetic_glyph_paint_bounds(
                            self.registry,
                            &state.font,
                            *gid,
                            *stroke_width,
                            state.font_size,
                        ) {
                            let scale = number(state.font_size).mul(Span::enclosing(0.001));
                            let transform = Matrix([
                                scale,
                                Span::exact(0.0),
                                Span::exact(0.0),
                                scale.neg(),
                                number(pen_x + offset.0),
                                number(size.height - pen_y - offset.1),
                            ]);
                            include(
                                &mut ink,
                                state.matrix.then(transform).bounds(Bounds {
                                    left: bbox[0],
                                    bottom: bbox[1],
                                    right: bbox[2],
                                    top: bbox[3],
                                }),
                            );
                        }
                        if let Some((dx, dy)) = advances.get(i) {
                            pen_x = pen_x + *dx;
                            pen_y = pen_y + *dy;
                        }
                    }
                }
                Command::DrawFilteredForm { .. } => {
                    return Err(self.error("filtered_form_extent", index));
                }
                Command::DrawMaskedForm { .. } => {
                    return Err(self.error("masked_form_extent", index));
                }
                Command::ApplyBackdropFilter { .. } => {
                    return Err(self.error("backdrop_filter_extent", index));
                }
                Command::Meta { .. }
                | Command::SetFillColor(_)
                | Command::SetStrokeColor(_)
                | Command::SetDash { .. }
                | Command::SetBlendMode { .. }
                | Command::BeginTag { .. }
                | Command::BeginTagActualText { .. }
                | Command::EndTag
                | Command::BeginArtifact { .. }
                | Command::BeginOptionalContent { .. }
                | Command::EndMarkedContent => {}
            }
            if let Some((fill, stroke)) = paint_path {
                if let Some(mut b) = path.take() {
                    if stroke && state.stroke {
                        if state.width <= Pt::ZERO {
                            return Err(self.error("hairline_stroke", index));
                        }
                        if state.cap > 2 || state.join > 2 || state.miter < Pt::from_i32(1) {
                            return Err(self.error("invalid_stroke_state", index));
                        }
                        let join = if state.join == 0 {
                            number(state.miter).hi.max(2.0)
                        } else {
                            2.0
                        };
                        let radius = number(state.width).mul(Span::enclosing(join / 2.0));
                        let [a, bm, c, d, _, _] = state.matrix.0;
                        let dx = radius
                            .mul(Span::enclosing(a.magnitude() + c.magnitude()))
                            .hi;
                        let dy = radius
                            .mul(Span::enclosing(bm.magnitude() + d.magnitude()))
                            .hi;
                        b = Bounds {
                            left: next_down(b.left - dx),
                            bottom: next_down(b.bottom - dy),
                            right: next_up(b.right + dx),
                            top: next_up(b.top + dy),
                        };
                        ink = Some(b);
                    } else if fill && state.fill {
                        ink = Some(b);
                    }
                }
            }
            if let Some(bounds) = ink {
                if !bounds.finite() {
                    return Err(self.error("nonfinite_geometry", index));
                }
                if let Some(bounds) = state.clip.apply(bounds) {
                    include(&mut painted, bounds);
                }
            }
        }
        if !stack.is_empty() {
            return Err(self.error("unbalanced_graphics_state", commands.len()));
        }
        Ok(painted)
    }

    fn glyph(&self, font: &str, gid: u16, matrix: Matrix) -> Result<Option<Bounds>, ()> {
        let Some(outline) = self.registry.glyph_outline_for_id(font, gid) else {
            return if self.registry.glyph_program_is_empty(font, gid) {
                Ok(None)
            } else {
                Err(())
            };
        };
        let scale = Span::enclosing(1.0 / f64::from(outline.units_per_em.max(1)));
        let mut bounds = None;
        let mut point = |x: f32, y: f32| {
            include(
                &mut bounds,
                matrix.point(
                    Span::enclosing(f64::from(x)).mul(scale),
                    Span::enclosing(f64::from(y)).mul(scale),
                ),
            )
        };
        for command in &outline.commands {
            match *command {
                GlyphOutlineCommand::MoveTo(x, y) | GlyphOutlineCommand::LineTo(x, y) => {
                    point(x, y)
                }
                GlyphOutlineCommand::QuadTo(cx, cy, x, y) => {
                    point(cx, cy);
                    point(x, y);
                }
                GlyphOutlineCommand::CurveTo(a, b, c, d, x, y) => {
                    point(a, b);
                    point(c, d);
                    point(x, y);
                }
                GlyphOutlineCommand::Close => {}
            }
        }
        Ok(bounds)
    }

    fn text(
        &self,
        state: &State,
        text: &str,
        transform: Matrix,
        index: usize,
    ) -> Result<Option<Bounds>, Unknown> {
        if state.text_mode == 3 || text.is_empty() {
            return Ok(None);
        }
        if state.text_mode >= 4 {
            return Err(self.error("text_clip", index));
        }
        if state.text_mode != 0 {
            return Err(self.error("stroked_text", index));
        }
        if !state.fill {
            return Ok(None);
        }
        let plan = crate::pdf::text_paint_plan(self.registry, self.options, &state.font, text)
            .map_err(|reason| self.error(reason, index))?;
        let mut bounds = None;
        for glyph in plan.glyphs {
            let offset = Matrix::translate(
                Span::enclosing(glyph.x as f64).mul(Span::enclosing(0.00000001)),
                Span::exact(0.0),
            );
            if let Some(b) = self
                .glyph(
                    &plan.font,
                    glyph.gid,
                    state.matrix.then(transform).then(offset),
                )
                .map_err(|()| self.error("unresolved_glyph_outline", index))?
            {
                include(&mut bounds, b);
            }
        }
        Ok(bounds)
    }
}

fn include(target: &mut Option<Bounds>, value: Bounds) {
    *target = Some(target.map_or(value, |old| old.union(value)));
}

fn rectangle(matrix: Matrix, x: Pt, y: Pt, width: Pt, height: Pt) -> Bounds {
    let (x, y, w, h) = (number(x), number(y), number(width), number(height));
    matrix
        .point(x, y)
        .union(matrix.point(x.add(w), y))
        .union(matrix.point(x, y.add(h)))
        .union(matrix.point(x.add(w), y.add(h)))
}

#[cfg(test)]
mod tests;
