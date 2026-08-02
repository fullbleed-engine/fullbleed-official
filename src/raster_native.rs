//! Dependency-free deterministic vector raster primitives.
//!
//! The public surface in this module is intentionally narrow: it is the subset used by
//! `raster.rs` for page previews and SVG fallback. Paths are flattened in device space, fills use
//! subpixel scan conversion, strokes are rasterized as the union of their geometric primitives,
//! and all compositing operates on premultiplied RGBA8 pixels.

use std::marker::PhantomData;

const SUBPIXEL_SCALE: i32 = 4;
const SUBPIXEL_SAMPLES: u32 = (SUBPIXEL_SCALE * SUBPIXEL_SCALE) as u32;
const CURVE_TOLERANCE: f32 = 0.18;
const MAX_CURVE_DEPTH: u8 = 12;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Color {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl Color {
    pub(crate) fn from_rgba(r: f32, g: f32, b: f32, a: f32) -> Option<Self> {
        if [r, g, b, a]
            .into_iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
        {
            Some(Self { r, g, b, a })
        } else {
            None
        }
    }

    pub(crate) fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        const SCALE: f32 = 1.0 / 255.0;
        Self {
            r: r as f32 * SCALE,
            g: g as f32 * SCALE,
            b: b as f32 * SCALE,
            a: a as f32 * SCALE,
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::from_rgba8(0, 0, 0, 255)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BlendMode {
    SourceOver,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    Plus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FillRule {
    Winding,
    EvenOdd,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FilterQuality {
    Nearest,
    Bilinear,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LineCap {
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LineJoin {
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SpreadMode {
    Pad,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Point {
    x: f32,
    y: f32,
}

impl Point {
    pub(crate) fn from_xy(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl Rect {
    pub(crate) fn from_xywh(x: f32, y: f32, width: f32, height: f32) -> Option<Self> {
        if [x, y, width, height].into_iter().all(f32::is_finite) && width > 0.0 && height > 0.0 {
            Some(Self {
                x,
                y,
                width,
                height,
            })
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Transform {
    sx: f32,
    kx: f32,
    ky: f32,
    sy: f32,
    tx: f32,
    ty: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

impl Transform {
    pub(crate) const fn identity() -> Self {
        Self {
            sx: 1.0,
            kx: 0.0,
            ky: 0.0,
            sy: 1.0,
            tx: 0.0,
            ty: 0.0,
        }
    }

    pub(crate) const fn from_row(sx: f32, ky: f32, kx: f32, sy: f32, tx: f32, ty: f32) -> Self {
        Self {
            sx,
            kx,
            ky,
            sy,
            tx,
            ty,
        }
    }

    pub(crate) const fn from_translate(tx: f32, ty: f32) -> Self {
        Self::from_row(1.0, 0.0, 0.0, 1.0, tx, ty)
    }

    pub(crate) const fn from_scale(sx: f32, sy: f32) -> Self {
        Self::from_row(sx, 0.0, 0.0, sy, 0.0, 0.0)
    }

    pub(crate) fn from_rotate(degrees: f32) -> Self {
        let radians = degrees.to_radians();
        let sin = radians.sin();
        let cos = radians.cos();
        Self::from_row(cos, sin, -sin, cos, 0.0, 0.0)
    }

    pub(crate) fn pre_concat(self, other: Self) -> Self {
        concat(self, other)
    }

    pub(crate) fn post_concat(self, other: Self) -> Self {
        concat(other, self)
    }

    pub(crate) fn get_scale(self) -> (f32, f32) {
        (
            (self.sx * self.sx + self.kx * self.kx).sqrt(),
            (self.ky * self.ky + self.sy * self.sy).sqrt(),
        )
    }

    fn map(self, point: Point) -> Point {
        Point {
            x: point.x * self.sx + point.y * self.kx + self.tx,
            y: point.x * self.ky + point.y * self.sy + self.ty,
        }
    }

    fn inverse(self) -> Option<Self> {
        let determinant = self.sx as f64 * self.sy as f64 - self.kx as f64 * self.ky as f64;
        if !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
            return None;
        }
        let inverse = 1.0 / determinant;
        let sx = (self.sy as f64 * inverse) as f32;
        let ky = (-self.ky as f64 * inverse) as f32;
        let kx = (-self.kx as f64 * inverse) as f32;
        let sy = (self.sx as f64 * inverse) as f32;
        let tx = -sx * self.tx - kx * self.ty;
        let ty = -ky * self.tx - sy * self.ty;
        Some(Self::from_row(sx, ky, kx, sy, tx, ty))
    }
}

fn concat(a: Transform, b: Transform) -> Transform {
    Transform::from_row(
        (a.sx as f64 * b.sx as f64 + a.kx as f64 * b.ky as f64) as f32,
        (a.ky as f64 * b.sx as f64 + a.sy as f64 * b.ky as f64) as f32,
        (a.sx as f64 * b.kx as f64 + a.kx as f64 * b.sy as f64) as f32,
        (a.ky as f64 * b.kx as f64 + a.sy as f64 * b.sy as f64) as f32,
        (a.sx as f64 * b.tx as f64 + a.kx as f64 * b.ty as f64) as f32 + a.tx,
        (a.ky as f64 * b.tx as f64 + a.sy as f64 * b.ty as f64) as f32 + a.ty,
    )
}

#[derive(Clone, Copy, Debug)]
enum PathVerb {
    Move(Point),
    Line(Point),
    Quad(Point, Point),
    Cubic(Point, Point, Point),
    Close,
}

#[derive(Clone, Debug)]
pub(crate) struct Path {
    verbs: Vec<PathVerb>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PathBuilder {
    verbs: Vec<PathVerb>,
    current: Option<Point>,
    contour_start: Option<Point>,
}

impl PathBuilder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn from_rect(rect: Rect) -> Path {
        let mut builder = Self::new();
        builder.move_to(rect.x, rect.y);
        builder.line_to(rect.x + rect.width, rect.y);
        builder.line_to(rect.x + rect.width, rect.y + rect.height);
        builder.line_to(rect.x, rect.y + rect.height);
        builder.close();
        builder.finish().expect("rectangle path")
    }

    pub(crate) fn move_to(&mut self, x: f32, y: f32) {
        let point = Point { x, y };
        if finite_point(point) {
            self.verbs.push(PathVerb::Move(point));
            self.current = Some(point);
            self.contour_start = Some(point);
        }
    }

    pub(crate) fn line_to(&mut self, x: f32, y: f32) {
        let point = Point { x, y };
        if finite_point(point) && self.current.is_some() {
            self.verbs.push(PathVerb::Line(point));
            self.current = Some(point);
        }
    }

    pub(crate) fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let control = Point { x: x1, y: y1 };
        let point = Point { x, y };
        if finite_point(control) && finite_point(point) && self.current.is_some() {
            self.verbs.push(PathVerb::Quad(control, point));
            self.current = Some(point);
        }
    }

    pub(crate) fn cubic_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let first = Point { x: x1, y: y1 };
        let second = Point { x: x2, y: y2 };
        let point = Point { x, y };
        if finite_point(first)
            && finite_point(second)
            && finite_point(point)
            && self.current.is_some()
        {
            self.verbs.push(PathVerb::Cubic(first, second, point));
            self.current = Some(point);
        }
    }

    pub(crate) fn close(&mut self) {
        if self.current.is_some() && self.contour_start.is_some() {
            self.verbs.push(PathVerb::Close);
            self.current = self.contour_start;
        }
    }

    pub(crate) fn finish(self) -> Option<Path> {
        (!self.verbs.is_empty()).then_some(Path { verbs: self.verbs })
    }
}

fn finite_point(point: Point) -> bool {
    point.x.is_finite() && point.y.is_finite()
}

#[derive(Clone, Debug)]
struct Contour {
    points: Vec<Point>,
    closed: bool,
}

fn flatten_path(path: &Path, transform: Transform) -> Vec<Contour> {
    let mut contours = Vec::new();
    let mut points = Vec::new();
    let mut current = None;
    let mut start = None;
    let mut closed = false;

    let finish_contour = |contours: &mut Vec<Contour>, points: &mut Vec<Point>, closed: bool| {
        if points.len() >= 2 {
            contours.push(Contour {
                points: std::mem::take(points),
                closed,
            });
        } else {
            points.clear();
        }
    };

    for verb in &path.verbs {
        match *verb {
            PathVerb::Move(point) => {
                finish_contour(&mut contours, &mut points, closed);
                let mapped = transform.map(point);
                points.push(mapped);
                current = Some(mapped);
                start = Some(mapped);
                closed = false;
            }
            PathVerb::Line(point) => {
                let mapped = transform.map(point);
                if current.is_some() && finite_point(mapped) {
                    push_distinct(&mut points, mapped);
                    current = Some(mapped);
                }
            }
            PathVerb::Quad(control, point) => {
                let Some(from) = current else { continue };
                let control = transform.map(control);
                let point = transform.map(point);
                flatten_quad(from, control, point, 0, &mut points);
                current = Some(point);
            }
            PathVerb::Cubic(first, second, point) => {
                let Some(from) = current else { continue };
                let first = transform.map(first);
                let second = transform.map(second);
                let point = transform.map(point);
                flatten_cubic(from, first, second, point, 0, &mut points);
                current = Some(point);
            }
            PathVerb::Close => {
                if let Some(first) = start {
                    push_distinct(&mut points, first);
                    current = Some(first);
                    closed = true;
                }
            }
        }
    }
    finish_contour(&mut contours, &mut points, closed);
    contours
}

fn push_distinct(points: &mut Vec<Point>, point: Point) {
    if points.last().is_none_or(|previous| {
        (previous.x - point.x).abs() > 1.0e-6 || (previous.y - point.y).abs() > 1.0e-6
    }) {
        points.push(point);
    }
}

fn flatten_quad(from: Point, control: Point, to: Point, depth: u8, output: &mut Vec<Point>) {
    if depth >= MAX_CURVE_DEPTH || point_line_distance(control, from, to) <= CURVE_TOLERANCE {
        push_distinct(output, to);
        return;
    }
    let first = midpoint(from, control);
    let second = midpoint(control, to);
    let middle = midpoint(first, second);
    flatten_quad(from, first, middle, depth + 1, output);
    flatten_quad(middle, second, to, depth + 1, output);
}

fn flatten_cubic(
    from: Point,
    first: Point,
    second: Point,
    to: Point,
    depth: u8,
    output: &mut Vec<Point>,
) {
    let flatness = point_line_distance(first, from, to).max(point_line_distance(second, from, to));
    if depth >= MAX_CURVE_DEPTH || flatness <= CURVE_TOLERANCE {
        push_distinct(output, to);
        return;
    }
    let p01 = midpoint(from, first);
    let p12 = midpoint(first, second);
    let p23 = midpoint(second, to);
    let p012 = midpoint(p01, p12);
    let p123 = midpoint(p12, p23);
    let middle = midpoint(p012, p123);
    flatten_cubic(from, p01, p012, middle, depth + 1, output);
    flatten_cubic(middle, p123, p23, to, depth + 1, output);
}

fn midpoint(first: Point, second: Point) -> Point {
    Point {
        x: (first.x + second.x) * 0.5,
        y: (first.y + second.y) * 0.5,
    }
}

fn point_line_distance(point: Point, from: Point, to: Point) -> f32 {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let denominator = (dx * dx + dy * dy).sqrt();
    if denominator <= 1.0e-6 {
        ((point.x - from.x).powi(2) + (point.y - from.y).powi(2)).sqrt()
    } else {
        ((dy * point.x - dx * point.y + to.x * from.y - to.y * from.x).abs()) / denominator
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct IntSize {
    width: u32,
    height: u32,
}

impl IntSize {
    pub(crate) fn from_wh(width: u32, height: u32) -> Option<Self> {
        (width > 0 && height > 0).then_some(Self { width, height })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct GradientStop {
    offset: f32,
    color: Color,
}

impl GradientStop {
    pub(crate) fn new(offset: f32, color: Color) -> Self {
        Self {
            offset: offset.clamp(0.0, 1.0),
            color,
        }
    }
}

#[derive(Clone, Debug)]
enum ShaderKind {
    Solid(Color),
    Linear {
        start: Point,
        end: Point,
        stops: Vec<GradientStop>,
        transform: Transform,
    },
    Radial {
        start: Point,
        end: Point,
        radius: f32,
        stops: Vec<GradientStop>,
        transform: Transform,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct Shader<'a> {
    kind: ShaderKind,
    marker: PhantomData<&'a ()>,
}

impl<'a> Shader<'a> {
    fn solid(color: Color) -> Self {
        Self {
            kind: ShaderKind::Solid(color),
            marker: PhantomData,
        }
    }

    fn sample(&self, point: Point) -> Color {
        match &self.kind {
            ShaderKind::Solid(color) => *color,
            ShaderKind::Linear {
                start,
                end,
                stops,
                transform,
            } => {
                let point = transform
                    .inverse()
                    .map_or(point, |inverse| inverse.map(point));
                let dx = end.x - start.x;
                let dy = end.y - start.y;
                let length_squared = dx * dx + dy * dy;
                let t = if length_squared <= f32::EPSILON {
                    1.0
                } else {
                    ((point.x - start.x) * dx + (point.y - start.y) * dy) / length_squared
                };
                sample_stops(stops, t.clamp(0.0, 1.0))
            }
            ShaderKind::Radial {
                start,
                end,
                radius,
                stops,
                transform,
            } => {
                let point = transform
                    .inverse()
                    .map_or(point, |inverse| inverse.map(point));
                let dc = Point {
                    x: end.x - start.x,
                    y: end.y - start.y,
                };
                let q = Point {
                    x: point.x - start.x,
                    y: point.y - start.y,
                };
                let a = dc.x * dc.x + dc.y * dc.y - radius * radius;
                let b = -2.0 * (q.x * dc.x + q.y * dc.y);
                let c = q.x * q.x + q.y * q.y;
                let t = solve_gradient_parameter(a, b, c).unwrap_or(0.0);
                sample_stops(stops, t.clamp(0.0, 1.0))
            }
        }
    }
}

fn solve_gradient_parameter(a: f32, b: f32, c: f32) -> Option<f32> {
    if a.abs() <= 1.0e-8 {
        return (b.abs() > 1.0e-8).then_some(-c / b);
    }
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return None;
    }
    let root = discriminant.sqrt();
    let first = (-b - root) / (2.0 * a);
    let second = (-b + root) / (2.0 * a);
    [first, second]
        .into_iter()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .min_by(|left, right| left.total_cmp(right))
}

fn sample_stops(stops: &[GradientStop], t: f32) -> Color {
    let Some(first) = stops.first() else {
        return Color::default();
    };
    if t <= first.offset {
        return first.color;
    }
    for window in stops.windows(2) {
        let left = window[0];
        let right = window[1];
        if t <= right.offset {
            let span = right.offset - left.offset;
            let local = if span <= f32::EPSILON {
                1.0
            } else {
                ((t - left.offset) / span).clamp(0.0, 1.0)
            };
            return interpolate_color(left.color, right.color, local);
        }
    }
    stops.last().map(|stop| stop.color).unwrap_or_default()
}

fn interpolate_color(first: Color, second: Color, t: f32) -> Color {
    let inverse = 1.0 - t;
    let alpha = first.a * inverse + second.a * t;
    if alpha <= f32::EPSILON {
        return Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        };
    }
    Color {
        r: (first.r * first.a * inverse + second.r * second.a * t) / alpha,
        g: (first.g * first.a * inverse + second.g * second.a * t) / alpha,
        b: (first.b * first.a * inverse + second.b * second.a * t) / alpha,
        a: alpha,
    }
}

pub(crate) struct LinearGradient;

impl LinearGradient {
    pub(crate) fn new(
        start: Point,
        end: Point,
        mut stops: Vec<GradientStop>,
        _spread: SpreadMode,
        transform: Transform,
    ) -> Option<Shader<'static>> {
        if !finite_point(start) || !finite_point(end) || stops.is_empty() {
            return None;
        }
        stops.sort_by(|left, right| left.offset.total_cmp(&right.offset));
        Some(Shader {
            kind: ShaderKind::Linear {
                start,
                end,
                stops,
                transform,
            },
            marker: PhantomData,
        })
    }
}

pub(crate) struct RadialGradient;

impl RadialGradient {
    pub(crate) fn new(
        start: Point,
        end: Point,
        radius: f32,
        mut stops: Vec<GradientStop>,
        _spread: SpreadMode,
        transform: Transform,
    ) -> Option<Shader<'static>> {
        if !finite_point(start)
            || !finite_point(end)
            || !radius.is_finite()
            || radius <= 0.0
            || stops.is_empty()
        {
            return None;
        }
        stops.sort_by(|left, right| left.offset.total_cmp(&right.offset));
        Some(Shader {
            kind: ShaderKind::Radial {
                start,
                end,
                radius,
                stops,
                transform,
            },
            marker: PhantomData,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Paint<'a> {
    pub(crate) shader: Shader<'a>,
    pub(crate) anti_alias: bool,
    pub(crate) blend_mode: BlendMode,
}

impl Default for Paint<'static> {
    fn default() -> Self {
        Self {
            shader: Shader::solid(Color::default()),
            anti_alias: true,
            blend_mode: BlendMode::SourceOver,
        }
    }
}

impl<'a> Paint<'a> {
    pub(crate) fn set_color(&mut self, color: Color) {
        self.shader = Shader::solid(color);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StrokeDash {
    pattern: Vec<f32>,
    offset: f32,
}

impl StrokeDash {
    pub(crate) fn new(pattern: Vec<f32>, offset: f32) -> Option<Self> {
        if pattern.len() < 2
            || pattern.len() % 2 != 0
            || !offset.is_finite()
            || pattern
                .iter()
                .any(|value| !value.is_finite() || *value < 0.0)
            || pattern.iter().copied().sum::<f32>() <= f32::EPSILON
        {
            return None;
        }
        Some(Self { pattern, offset })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Stroke {
    pub(crate) width: f32,
    pub(crate) miter_limit: f32,
    pub(crate) line_cap: LineCap,
    pub(crate) line_join: LineJoin,
    pub(crate) dash: Option<StrokeDash>,
}

impl Default for Stroke {
    fn default() -> Self {
        Self {
            width: 1.0,
            miter_limit: 4.0,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter,
            dash: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PixmapPaint {
    pub(crate) opacity: f32,
    pub(crate) blend_mode: BlendMode,
    pub(crate) quality: FilterQuality,
}

impl Default for PixmapPaint {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            blend_mode: BlendMode::SourceOver,
            quality: FilterQuality::Nearest,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Pixmap {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PixmapRef<'a> {
    width: u32,
    height: u32,
    data: &'a [u8],
}

impl Pixmap {
    pub(crate) fn new(width: u32, height: u32) -> Option<Self> {
        let length = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        (width > 0 && height > 0).then(|| Self {
            width,
            height,
            data: vec![0; length],
        })
    }

    pub(crate) fn from_vec(data: Vec<u8>, size: IntSize) -> Option<Self> {
        let expected = (size.width as usize)
            .checked_mul(size.height as usize)?
            .checked_mul(4)?;
        (data.len() == expected).then_some(Self {
            width: size.width,
            height: size.height,
            data,
        })
    }

    pub(crate) const fn width(&self) -> u32 {
        self.width
    }

    pub(crate) const fn height(&self) -> u32 {
        self.height
    }

    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }

    pub(crate) fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    pub(crate) fn as_ref(&self) -> PixmapRef<'_> {
        PixmapRef {
            width: self.width,
            height: self.height,
            data: &self.data,
        }
    }

    pub(crate) fn fill(&mut self, color: Color) {
        let alpha = unit_to_u8(color.a);
        let pixel = [
            unit_to_u8(color.r * color.a),
            unit_to_u8(color.g * color.a),
            unit_to_u8(color.b * color.a),
            alpha,
        ];
        for destination in self.data.chunks_exact_mut(4) {
            destination.copy_from_slice(&pixel);
        }
    }

    pub(crate) fn fill_path(
        &mut self,
        path: &Path,
        paint: &Paint<'_>,
        fill_rule: FillRule,
        transform: Transform,
        clip_mask: Option<&Mask>,
    ) {
        let contours = flatten_path(path, transform);
        let Some(coverage) = rasterize_contours(
            &contours,
            fill_rule,
            self.width,
            self.height,
            paint.anti_alias,
        ) else {
            return;
        };
        let inverse = transform.inverse();
        composite_coverage(
            self,
            &coverage,
            clip_mask,
            |x, y| {
                let device = Point {
                    x: x as f32 + 0.5,
                    y: y as f32 + 0.5,
                };
                let local = inverse.map_or(device, |value| value.map(device));
                paint.shader.sample(local)
            },
            paint.blend_mode,
        );
    }

    pub(crate) fn stroke_path(
        &mut self,
        path: &Path,
        paint: &Paint<'_>,
        stroke: &Stroke,
        transform: Transform,
        clip_mask: Option<&Mask>,
    ) {
        let Some(coverage) = rasterize_stroke(
            path,
            stroke,
            transform,
            self.width,
            self.height,
            paint.anti_alias,
        ) else {
            return;
        };
        let inverse = transform.inverse();
        composite_coverage(
            self,
            &coverage,
            clip_mask,
            |x, y| {
                let device = Point {
                    x: x as f32 + 0.5,
                    y: y as f32 + 0.5,
                };
                let local = inverse.map_or(device, |value| value.map(device));
                paint.shader.sample(local)
            },
            paint.blend_mode,
        );
    }

    pub(crate) fn draw_pixmap(
        &mut self,
        x: i32,
        y: i32,
        source: PixmapRef<'_>,
        paint: &PixmapPaint,
        transform: Transform,
        clip_mask: Option<&Mask>,
    ) {
        let placement = transform.pre_concat(Transform::from_translate(x as f32, y as f32));
        let Some(inverse) = placement.inverse() else {
            return;
        };
        let corners = [
            placement.map(Point::from_xy(0.0, 0.0)),
            placement.map(Point::from_xy(source.width as f32, 0.0)),
            placement.map(Point::from_xy(source.width as f32, source.height as f32)),
            placement.map(Point::from_xy(0.0, source.height as f32)),
        ];
        let Some((min_x, min_y, max_x, max_y)) = pixel_bounds(&corners, self.width, self.height)
        else {
            return;
        };
        let opacity = paint.opacity.clamp(0.0, 1.0);
        for destination_y in min_y..max_y {
            for destination_x in min_x..max_x {
                let source_point = inverse.map(Point {
                    x: destination_x as f32 + 0.5,
                    y: destination_y as f32 + 0.5,
                });
                if source_point.x < 0.0
                    || source_point.y < 0.0
                    || source_point.x >= source.width as f32
                    || source_point.y >= source.height as f32
                {
                    continue;
                }
                let sample = sample_pixmap(source, source_point, paint.quality);
                let clip = clip_mask
                    .map(|mask| mask.alpha_at(destination_x, destination_y) as f32 / 255.0)
                    .unwrap_or(1.0);
                if clip <= 0.0 || sample[3] <= 0.0 {
                    continue;
                }
                let index =
                    ((destination_y as usize * self.width as usize) + destination_x as usize) * 4;
                blend_premultiplied_pixel(
                    &mut self.data[index..index + 4],
                    sample,
                    opacity * clip,
                    paint.blend_mode,
                );
            }
        }
    }
}

fn sample_pixmap(source: PixmapRef<'_>, point: Point, quality: FilterQuality) -> [f32; 4] {
    match quality {
        FilterQuality::Nearest => {
            let x = (point.x.floor() as i32).clamp(0, source.width as i32 - 1) as u32;
            let y = (point.y.floor() as i32).clamp(0, source.height as i32 - 1) as u32;
            source_pixel(source, x, y)
        }
        FilterQuality::Bilinear => {
            let fx = point.x - 0.5;
            let fy = point.y - 0.5;
            let x0 = fx.floor() as i32;
            let y0 = fy.floor() as i32;
            let tx = fx - x0 as f32;
            let ty = fy - y0 as f32;
            let mut output = [0.0; 4];
            for (sample_y, weight_y) in [(y0, 1.0 - ty), (y0 + 1, ty)] {
                for (sample_x, weight_x) in [(x0, 1.0 - tx), (x0 + 1, tx)] {
                    let clamped_x = sample_x.clamp(0, source.width as i32 - 1) as u32;
                    let clamped_y = sample_y.clamp(0, source.height as i32 - 1) as u32;
                    let pixel = source_pixel(source, clamped_x, clamped_y);
                    let weight = weight_x * weight_y;
                    for channel in 0..4 {
                        output[channel] += pixel[channel] * weight;
                    }
                }
            }
            output
        }
    }
}

fn source_pixel(source: PixmapRef<'_>, x: u32, y: u32) -> [f32; 4] {
    let index = ((y as usize * source.width as usize) + x as usize) * 4;
    const SCALE: f32 = 1.0 / 255.0;
    [
        source.data[index] as f32 * SCALE,
        source.data[index + 1] as f32 * SCALE,
        source.data[index + 2] as f32 * SCALE,
        source.data[index + 3] as f32 * SCALE,
    ]
}

#[derive(Clone, Debug)]
pub(crate) struct Mask {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Mask {
    pub(crate) fn new(width: u32, height: u32) -> Option<Self> {
        let length = (width as usize).checked_mul(height as usize)?;
        (width > 0 && height > 0).then(|| Self {
            width,
            height,
            data: vec![0; length],
        })
    }

    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }

    pub(crate) fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    pub(crate) fn fill_path(
        &mut self,
        path: &Path,
        fill_rule: FillRule,
        anti_alias: bool,
        transform: Transform,
    ) {
        self.data.fill(0);
        let contours = flatten_path(path, transform);
        if let Some(coverage) =
            rasterize_contours(&contours, fill_rule, self.width, self.height, anti_alias)
        {
            coverage.copy_into(&mut self.data, self.width, false);
        }
    }

    pub(crate) fn intersect_path(
        &mut self,
        path: &Path,
        fill_rule: FillRule,
        anti_alias: bool,
        transform: Transform,
    ) {
        let contours = flatten_path(path, transform);
        let mut next = vec![0; self.data.len()];
        if let Some(coverage) =
            rasterize_contours(&contours, fill_rule, self.width, self.height, anti_alias)
        {
            coverage.copy_into(&mut next, self.width, false);
        }
        for (current, incoming) in self.data.iter_mut().zip(next) {
            *current = multiply_u8(*current, incoming);
        }
    }

    fn alpha_at(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            0
        } else {
            self.data[y as usize * self.width as usize + x as usize]
        }
    }
}

#[derive(Clone, Debug)]
struct Coverage {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    alpha: Vec<u8>,
}

impl Coverage {
    fn copy_into(&self, destination: &mut [u8], destination_width: u32, multiply: bool) {
        for row in 0..self.height as usize {
            let source_offset = row * self.width as usize;
            let destination_offset =
                (self.y as usize + row) * destination_width as usize + self.x as usize;
            for column in 0..self.width as usize {
                let source = self.alpha[source_offset + column];
                let destination = &mut destination[destination_offset + column];
                *destination = if multiply {
                    multiply_u8(*destination, source)
                } else {
                    source
                };
            }
        }
    }
}

fn rasterize_contours(
    contours: &[Contour],
    fill_rule: FillRule,
    canvas_width: u32,
    canvas_height: u32,
    anti_alias: bool,
) -> Option<Coverage> {
    let mut edges = Vec::new();
    let mut all_points = Vec::new();
    for contour in contours {
        if contour.points.len() < 2 {
            continue;
        }
        all_points.extend_from_slice(&contour.points);
        for pair in contour.points.windows(2) {
            if pair[0].y != pair[1].y {
                edges.push((pair[0], pair[1]));
            }
        }
        let first = contour.points[0];
        let last = *contour.points.last().unwrap_or(&first);
        if (last.x != first.x || last.y != first.y) && last.y != first.y {
            edges.push((last, first));
        }
    }
    if edges.is_empty() {
        return None;
    }
    let (min_x, min_y, max_x, max_y) = pixel_bounds(&all_points, canvas_width, canvas_height)?;
    let width = max_x - min_x;
    let height = max_y - min_y;
    let scale = if anti_alias { SUBPIXEL_SCALE } else { 1 };
    let sample_count = (scale * scale) as u32;
    let mut samples = vec![0u16; width as usize * height as usize];
    let start_sample_y = min_y as i32 * scale;
    let end_sample_y = max_y as i32 * scale;
    let mut intersections = Vec::with_capacity(edges.len());
    for sample_y in start_sample_y..end_sample_y {
        let y = (sample_y as f32 + 0.5) / scale as f32;
        intersections.clear();
        for &(first, second) in &edges {
            let low = first.y.min(second.y);
            let high = first.y.max(second.y);
            if y < low || y >= high {
                continue;
            }
            let fraction = (y - first.y) / (second.y - first.y);
            let x = first.x + (second.x - first.x) * fraction;
            let winding = if second.y > first.y { 1i16 } else { -1i16 };
            intersections.push((x, winding));
        }
        intersections.sort_by(|left, right| left.0.total_cmp(&right.0));
        match fill_rule {
            FillRule::EvenOdd => {
                let mut index = 0;
                while index + 1 < intersections.len() {
                    mark_sample_span(
                        &mut samples,
                        min_x,
                        min_y,
                        width,
                        height,
                        scale,
                        sample_y,
                        intersections[index].0,
                        intersections[index + 1].0,
                    );
                    index += 2;
                }
            }
            FillRule::Winding => {
                let mut winding = 0i32;
                let mut span_start = None;
                let mut index = 0;
                while index < intersections.len() {
                    let x = intersections[index].0;
                    let was_inside = winding != 0;
                    while index < intersections.len()
                        && (intersections[index].0 - x).abs() <= 1.0e-6
                    {
                        winding += i32::from(intersections[index].1);
                        index += 1;
                    }
                    let is_inside = winding != 0;
                    if !was_inside && is_inside {
                        span_start = Some(x);
                    } else if was_inside && !is_inside {
                        if let Some(start) = span_start.take() {
                            mark_sample_span(
                                &mut samples,
                                min_x,
                                min_y,
                                width,
                                height,
                                scale,
                                sample_y,
                                start,
                                x,
                            );
                        }
                    }
                }
            }
        }
    }
    let alpha = samples
        .into_iter()
        .map(|bits| {
            let covered = bits.count_ones().min(sample_count);
            ((covered * 255 + sample_count / 2) / sample_count) as u8
        })
        .collect();
    Some(Coverage {
        x: min_x,
        y: min_y,
        width,
        height,
        alpha,
    })
}

#[allow(clippy::too_many_arguments)]
fn mark_sample_span(
    samples: &mut [u16],
    min_x: u32,
    min_y: u32,
    width: u32,
    height: u32,
    scale: i32,
    sample_y: i32,
    first_x: f32,
    second_x: f32,
) {
    let left = first_x.min(second_x);
    let right = first_x.max(second_x);
    if right <= left {
        return;
    }
    let start = (left * scale as f32 - 0.5).ceil() as i32;
    let end = (right * scale as f32 - 0.5).ceil() as i32;
    let min_sample_x = min_x as i32 * scale;
    let max_sample_x = (min_x + width) as i32 * scale;
    let sample_y_min = min_y as i32 * scale;
    let sample_y_max = (min_y + height) as i32 * scale;
    if sample_y < sample_y_min || sample_y >= sample_y_max {
        return;
    }
    for sample_x in start.max(min_sample_x)..end.min(max_sample_x) {
        let pixel_x = sample_x.div_euclid(scale) - min_x as i32;
        let pixel_y = sample_y.div_euclid(scale) - min_y as i32;
        if pixel_x < 0 || pixel_y < 0 || pixel_x >= width as i32 || pixel_y >= height as i32 {
            continue;
        }
        let sub_x = sample_x.rem_euclid(scale) as u16;
        let sub_y = sample_y.rem_euclid(scale) as u16;
        let bit = sub_y * scale as u16 + sub_x;
        let index = pixel_y as usize * width as usize + pixel_x as usize;
        samples[index] |= 1u16 << bit;
    }
}

fn pixel_bounds(points: &[Point], width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    if points.is_empty() || width == 0 || height == 0 {
        return None;
    }
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for point in points {
        if !finite_point(*point) {
            continue;
        }
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    if !min_x.is_finite() || !min_y.is_finite() || max_x <= min_x || max_y <= min_y {
        return None;
    }
    let min_x = (min_x.floor() as i64 - 1).clamp(0, width as i64) as u32;
    let min_y = (min_y.floor() as i64 - 1).clamp(0, height as i64) as u32;
    let max_x = (max_x.ceil() as i64 + 1).clamp(0, width as i64) as u32;
    let max_y = (max_y.ceil() as i64 + 1).clamp(0, height as i64) as u32;
    (max_x > min_x && max_y > min_y).then_some((min_x, min_y, max_x, max_y))
}

fn composite_coverage(
    pixmap: &mut Pixmap,
    coverage: &Coverage,
    clip_mask: Option<&Mask>,
    mut color_at: impl FnMut(u32, u32) -> Color,
    blend_mode: BlendMode,
) {
    for local_y in 0..coverage.height {
        let y = coverage.y + local_y;
        for local_x in 0..coverage.width {
            let x = coverage.x + local_x;
            let coverage_alpha =
                coverage.alpha[local_y as usize * coverage.width as usize + local_x as usize];
            if coverage_alpha == 0 {
                continue;
            }
            let clip_alpha = clip_mask.map(|mask| mask.alpha_at(x, y)).unwrap_or(255);
            let alpha = multiply_u8(coverage_alpha, clip_alpha);
            if alpha == 0 {
                continue;
            }
            let index = (y as usize * pixmap.width as usize + x as usize) * 4;
            blend_color_pixel(
                &mut pixmap.data[index..index + 4],
                color_at(x, y),
                alpha as f32 / 255.0,
                blend_mode,
            );
        }
    }
}

fn multiply_u8(first: u8, second: u8) -> u8 {
    let product = u16::from(first) * u16::from(second) + 127;
    ((product + (product >> 8)) >> 8) as u8
}

fn unit_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn rasterize_stroke(
    path: &Path,
    stroke: &Stroke,
    transform: Transform,
    canvas_width: u32,
    canvas_height: u32,
    anti_alias: bool,
) -> Option<Coverage> {
    if !stroke.width.is_finite() || stroke.width <= 0.0 {
        return None;
    }
    let contours = flatten_path(path, Transform::identity());
    let mut polygons = Vec::new();
    for contour in contours {
        let pieces = if let Some(dash) = stroke.dash.as_ref() {
            dash_contour(&contour, dash)
        } else {
            vec![normalize_contour(contour)]
        };
        for piece in pieces {
            append_stroke_polygons(&piece, stroke, transform, &mut polygons);
        }
    }
    rasterize_polygon_union(&polygons, canvas_width, canvas_height, anti_alias)
}

fn normalize_contour(mut contour: Contour) -> Contour {
    if contour.points.len() >= 2
        && points_nearly_equal(contour.points[0], *contour.points.last().unwrap())
    {
        contour.points.pop();
        contour.closed = true;
    }
    contour
}

fn dash_contour(contour: &Contour, dash: &StrokeDash) -> Vec<Contour> {
    let contour = normalize_contour(contour.clone());
    if contour.points.len() < 2 {
        return Vec::new();
    }
    let mut source = contour.points.clone();
    if contour.closed {
        source.push(source[0]);
    }
    let total: f32 = dash.pattern.iter().sum();
    if total <= f32::EPSILON {
        return Vec::new();
    }
    let mut phase = dash.offset.rem_euclid(total);
    let mut pattern_index = 0usize;
    while phase >= dash.pattern[pattern_index] && dash.pattern[pattern_index] > 0.0 {
        phase -= dash.pattern[pattern_index];
        pattern_index = (pattern_index + 1) % dash.pattern.len();
    }
    let mut remaining = (dash.pattern[pattern_index] - phase).max(0.0);
    let mut active = pattern_index % 2 == 0;
    let starts_active = active;
    let mut pieces = Vec::new();
    let mut current = Vec::new();

    for pair in source.windows(2) {
        let from = pair[0];
        let to = pair[1];
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        let length = (dx * dx + dy * dy).sqrt();
        if length <= 1.0e-7 {
            continue;
        }
        let mut consumed = 0.0;
        while consumed < length - 1.0e-7 {
            while remaining <= 1.0e-7 {
                if active && current.len() >= 2 {
                    pieces.push(Contour {
                        points: std::mem::take(&mut current),
                        closed: false,
                    });
                }
                pattern_index = (pattern_index + 1) % dash.pattern.len();
                active = pattern_index % 2 == 0;
                remaining = dash.pattern[pattern_index];
            }
            let step = remaining.min(length - consumed);
            let start_fraction = consumed / length;
            let end_fraction = (consumed + step) / length;
            let start = lerp_point(from, to, start_fraction);
            let end = lerp_point(from, to, end_fraction);
            if active {
                if current.is_empty() {
                    current.push(start);
                }
                push_distinct(&mut current, end);
            }
            consumed += step;
            remaining -= step;
        }
    }
    if active && current.len() >= 2 {
        pieces.push(Contour {
            points: current,
            closed: false,
        });
    }

    if contour.closed
        && starts_active
        && active
        && pieces.len() >= 2
        && pieces
            .first()
            .and_then(|piece| piece.points.first())
            .is_some_and(|point| points_nearly_equal(*point, contour.points[0]))
        && pieces
            .last()
            .and_then(|piece| piece.points.last())
            .is_some_and(|point| points_nearly_equal(*point, contour.points[0]))
    {
        let mut last = pieces.pop().unwrap().points;
        let first = pieces.remove(0).points;
        if last
            .last()
            .is_some_and(|point| points_nearly_equal(*point, first[0]))
        {
            last.pop();
        }
        last.extend(first);
        pieces.insert(
            0,
            Contour {
                points: last,
                closed: false,
            },
        );
    }
    pieces
}

fn lerp_point(from: Point, to: Point, fraction: f32) -> Point {
    Point {
        x: from.x + (to.x - from.x) * fraction,
        y: from.y + (to.y - from.y) * fraction,
    }
}

fn points_nearly_equal(first: Point, second: Point) -> bool {
    (first.x - second.x).abs() <= 1.0e-5 && (first.y - second.y).abs() <= 1.0e-5
}

fn append_stroke_polygons(
    contour: &Contour,
    stroke: &Stroke,
    transform: Transform,
    polygons: &mut Vec<Vec<Point>>,
) {
    if contour.points.len() < 2 {
        return;
    }
    let half = stroke.width * 0.5;
    let segment_count = if contour.closed {
        contour.points.len()
    } else {
        contour.points.len() - 1
    };
    for index in 0..segment_count {
        let from = contour.points[index];
        let to = contour.points[(index + 1) % contour.points.len()];
        let Some((direction, normal)) = segment_vectors(from, to) else {
            continue;
        };
        let extend_start = !contour.closed && index == 0 && stroke.line_cap == LineCap::Square;
        let extend_end =
            !contour.closed && index + 1 == segment_count && stroke.line_cap == LineCap::Square;
        let from = if extend_start {
            Point {
                x: from.x - direction.x * half,
                y: from.y - direction.y * half,
            }
        } else {
            from
        };
        let to = if extend_end {
            Point {
                x: to.x + direction.x * half,
                y: to.y + direction.y * half,
            }
        } else {
            to
        };
        polygons.push(map_polygon(
            &[
                Point {
                    x: from.x + normal.x * half,
                    y: from.y + normal.y * half,
                },
                Point {
                    x: to.x + normal.x * half,
                    y: to.y + normal.y * half,
                },
                Point {
                    x: to.x - normal.x * half,
                    y: to.y - normal.y * half,
                },
                Point {
                    x: from.x - normal.x * half,
                    y: from.y - normal.y * half,
                },
            ],
            transform,
        ));
    }

    if contour.closed {
        for index in 0..contour.points.len() {
            let previous =
                contour.points[(index + contour.points.len() - 1) % contour.points.len()];
            let current = contour.points[index];
            let next = contour.points[(index + 1) % contour.points.len()];
            append_join_polygon(previous, current, next, half, stroke, transform, polygons);
        }
    } else {
        for window in contour.points.windows(3) {
            append_join_polygon(
                window[0], window[1], window[2], half, stroke, transform, polygons,
            );
        }
        if stroke.line_cap == LineCap::Round {
            polygons.push(circle_polygon(contour.points[0], half, transform));
            polygons.push(circle_polygon(
                *contour.points.last().unwrap(),
                half,
                transform,
            ));
        }
    }
}

fn segment_vectors(from: Point, to: Point) -> Option<(Point, Point)> {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let length = (dx * dx + dy * dy).sqrt();
    if length <= 1.0e-7 {
        None
    } else {
        let direction = Point {
            x: dx / length,
            y: dy / length,
        };
        Some((
            direction,
            Point {
                x: -direction.y,
                y: direction.x,
            },
        ))
    }
}

fn append_join_polygon(
    previous: Point,
    current: Point,
    next: Point,
    half: f32,
    stroke: &Stroke,
    transform: Transform,
    polygons: &mut Vec<Vec<Point>>,
) {
    let Some((previous_direction, previous_normal)) = segment_vectors(previous, current) else {
        return;
    };
    let Some((next_direction, next_normal)) = segment_vectors(current, next) else {
        return;
    };
    let cross = previous_direction.x * next_direction.y - previous_direction.y * next_direction.x;
    if cross.abs() <= 1.0e-6 {
        return;
    }
    if stroke.line_join == LineJoin::Round {
        polygons.push(circle_polygon(current, half, transform));
        return;
    }
    let side = if cross > 0.0 { -1.0 } else { 1.0 };
    let first_outer = Point {
        x: current.x + previous_normal.x * half * side,
        y: current.y + previous_normal.y * half * side,
    };
    let second_outer = Point {
        x: current.x + next_normal.x * half * side,
        y: current.y + next_normal.y * half * side,
    };
    let mut polygon = vec![first_outer];
    if stroke.line_join == LineJoin::Miter {
        if let Some(miter) = line_intersection(
            first_outer,
            previous_direction,
            second_outer,
            next_direction,
        ) {
            let distance = ((miter.x - current.x).powi(2) + (miter.y - current.y).powi(2)).sqrt();
            if distance <= half * stroke.miter_limit.max(1.0) {
                polygon.push(miter);
            }
        }
    }
    polygon.push(second_outer);
    polygon.push(current);
    polygons.push(map_polygon(&polygon, transform));
}

fn line_intersection(
    origin_a: Point,
    direction_a: Point,
    origin_b: Point,
    direction_b: Point,
) -> Option<Point> {
    let denominator = direction_a.x * direction_b.y - direction_a.y * direction_b.x;
    if denominator.abs() <= 1.0e-7 {
        return None;
    }
    let delta = Point {
        x: origin_b.x - origin_a.x,
        y: origin_b.y - origin_a.y,
    };
    let parameter = (delta.x * direction_b.y - delta.y * direction_b.x) / denominator;
    Some(Point {
        x: origin_a.x + direction_a.x * parameter,
        y: origin_a.y + direction_a.y * parameter,
    })
}

fn circle_polygon(center: Point, radius: f32, transform: Transform) -> Vec<Point> {
    let (scale_x, scale_y) = transform.get_scale();
    let circumference = core::f32::consts::TAU * radius * scale_x.max(scale_y).max(1.0);
    let segments = ((circumference / 1.5).ceil() as usize).clamp(16, 96);
    (0..segments)
        .map(|index| {
            let angle = core::f32::consts::TAU * index as f32 / segments as f32;
            transform.map(Point {
                x: center.x + angle.cos() * radius,
                y: center.y + angle.sin() * radius,
            })
        })
        .collect()
}

fn map_polygon(points: &[Point], transform: Transform) -> Vec<Point> {
    points.iter().map(|point| transform.map(*point)).collect()
}

fn rasterize_polygon_union(
    polygons: &[Vec<Point>],
    canvas_width: u32,
    canvas_height: u32,
    anti_alias: bool,
) -> Option<Coverage> {
    let all_points: Vec<Point> = polygons.iter().flatten().copied().collect();
    let (min_x, min_y, max_x, max_y) = pixel_bounds(&all_points, canvas_width, canvas_height)?;
    let width = max_x - min_x;
    let height = max_y - min_y;
    let scale = if anti_alias { SUBPIXEL_SCALE } else { 1 };
    let sample_count = if anti_alias { SUBPIXEL_SAMPLES } else { 1 };
    let mut samples = vec![0u16; width as usize * height as usize];
    for polygon in polygons {
        if polygon.len() < 3 {
            continue;
        }
        let start_sample_y = min_y as i32 * scale;
        let end_sample_y = max_y as i32 * scale;
        let mut intersections = Vec::with_capacity(polygon.len());
        for sample_y in start_sample_y..end_sample_y {
            let y = (sample_y as f32 + 0.5) / scale as f32;
            intersections.clear();
            for index in 0..polygon.len() {
                let first = polygon[index];
                let second = polygon[(index + 1) % polygon.len()];
                let low = first.y.min(second.y);
                let high = first.y.max(second.y);
                if first.y == second.y || y < low || y >= high {
                    continue;
                }
                let fraction = (y - first.y) / (second.y - first.y);
                intersections.push(first.x + (second.x - first.x) * fraction);
            }
            intersections.sort_by(f32::total_cmp);
            for pair in intersections.chunks_exact(2) {
                mark_sample_span(
                    &mut samples,
                    min_x,
                    min_y,
                    width,
                    height,
                    scale,
                    sample_y,
                    pair[0],
                    pair[1],
                );
            }
        }
    }
    Some(Coverage {
        x: min_x,
        y: min_y,
        width,
        height,
        alpha: samples
            .into_iter()
            .map(|bits| {
                let covered = bits.count_ones().min(sample_count);
                ((covered * 255 + sample_count / 2) / sample_count) as u8
            })
            .collect(),
    })
}

fn blend_color_pixel(destination: &mut [u8], color: Color, coverage: f32, mode: BlendMode) {
    let alpha = (color.a * coverage).clamp(0.0, 1.0);
    blend_premultiplied_pixel(
        destination,
        [color.r * alpha, color.g * alpha, color.b * alpha, alpha],
        1.0,
        mode,
    );
}

fn blend_premultiplied_pixel(
    destination: &mut [u8],
    source: [f32; 4],
    opacity: f32,
    mode: BlendMode,
) {
    const SCALE: f32 = 1.0 / 255.0;
    let source_alpha = (source[3] * opacity).clamp(0.0, 1.0);
    if source_alpha <= 0.0 {
        return;
    }
    let source_straight = if source[3] > 1.0e-8 {
        [
            (source[0] / source[3]).clamp(0.0, 1.0),
            (source[1] / source[3]).clamp(0.0, 1.0),
            (source[2] / source[3]).clamp(0.0, 1.0),
        ]
    } else {
        [0.0; 3]
    };
    let destination_alpha = destination[3] as f32 * SCALE;
    let destination_premultiplied = [
        destination[0] as f32 * SCALE,
        destination[1] as f32 * SCALE,
        destination[2] as f32 * SCALE,
    ];
    let destination_straight = if destination_alpha > 1.0e-8 {
        [
            (destination_premultiplied[0] / destination_alpha).clamp(0.0, 1.0),
            (destination_premultiplied[1] / destination_alpha).clamp(0.0, 1.0),
            (destination_premultiplied[2] / destination_alpha).clamp(0.0, 1.0),
        ]
    } else {
        [0.0; 3]
    };

    if mode == BlendMode::Plus {
        destination[0] =
            unit_to_u8((destination_premultiplied[0] + source_straight[0] * source_alpha).min(1.0));
        destination[1] =
            unit_to_u8((destination_premultiplied[1] + source_straight[1] * source_alpha).min(1.0));
        destination[2] =
            unit_to_u8((destination_premultiplied[2] + source_straight[2] * source_alpha).min(1.0));
        destination[3] = unit_to_u8((destination_alpha + source_alpha).min(1.0));
        return;
    }

    let blended = blend_rgb(mode, destination_straight, source_straight);
    let output_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    for channel in 0..3 {
        let output = (1.0 - source_alpha) * destination_premultiplied[channel]
            + (1.0 - destination_alpha) * source_straight[channel] * source_alpha
            + source_alpha * destination_alpha * blended[channel];
        destination[channel] = unit_to_u8(output);
    }
    destination[3] = unit_to_u8(output_alpha);
}

fn blend_rgb(mode: BlendMode, backdrop: [f32; 3], source: [f32; 3]) -> [f32; 3] {
    match mode {
        BlendMode::SourceOver => source,
        BlendMode::Multiply => component_map(backdrop, source, |b, s| b * s),
        BlendMode::Screen => component_map(backdrop, source, |b, s| b + s - b * s),
        BlendMode::Overlay => component_map(backdrop, source, overlay_component),
        BlendMode::Darken => component_map(backdrop, source, f32::min),
        BlendMode::Lighten => component_map(backdrop, source, f32::max),
        BlendMode::ColorDodge => component_map(backdrop, source, |backdrop, source| {
            if source >= 1.0 {
                1.0
            } else {
                (backdrop / (1.0 - source)).min(1.0)
            }
        }),
        BlendMode::ColorBurn => component_map(backdrop, source, |backdrop, source| {
            if source <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - backdrop) / source).min(1.0)
            }
        }),
        BlendMode::HardLight => component_map(backdrop, source, |backdrop, source| {
            overlay_component(source, backdrop)
        }),
        BlendMode::SoftLight => component_map(backdrop, source, |backdrop, source| {
            if source <= 0.5 {
                backdrop - (1.0 - 2.0 * source) * backdrop * (1.0 - backdrop)
            } else {
                let d = if backdrop <= 0.25 {
                    ((16.0 * backdrop - 12.0) * backdrop + 4.0) * backdrop
                } else {
                    backdrop.sqrt()
                };
                backdrop + (2.0 * source - 1.0) * (d - backdrop)
            }
        }),
        BlendMode::Difference => component_map(backdrop, source, |b, s| (b - s).abs()),
        BlendMode::Exclusion => component_map(backdrop, source, |b, s| b + s - 2.0 * b * s),
        BlendMode::Hue => set_lum(set_sat(source, saturation(backdrop)), luminosity(backdrop)),
        BlendMode::Saturation => {
            set_lum(set_sat(backdrop, saturation(source)), luminosity(backdrop))
        }
        BlendMode::Color => set_lum(source, luminosity(backdrop)),
        BlendMode::Luminosity => set_lum(backdrop, luminosity(source)),
        BlendMode::Plus => source,
    }
}

fn component_map(
    backdrop: [f32; 3],
    source: [f32; 3],
    operation: impl Fn(f32, f32) -> f32,
) -> [f32; 3] {
    [
        operation(backdrop[0], source[0]).clamp(0.0, 1.0),
        operation(backdrop[1], source[1]).clamp(0.0, 1.0),
        operation(backdrop[2], source[2]).clamp(0.0, 1.0),
    ]
}

fn overlay_component(backdrop: f32, source: f32) -> f32 {
    if backdrop <= 0.5 {
        2.0 * backdrop * source
    } else {
        1.0 - 2.0 * (1.0 - backdrop) * (1.0 - source)
    }
}

fn luminosity(color: [f32; 3]) -> f32 {
    0.3 * color[0] + 0.59 * color[1] + 0.11 * color[2]
}

fn saturation(color: [f32; 3]) -> f32 {
    color.into_iter().fold(f32::NEG_INFINITY, f32::max)
        - color.into_iter().fold(f32::INFINITY, f32::min)
}

fn set_lum(mut color: [f32; 3], target: f32) -> [f32; 3] {
    let difference = target - luminosity(color);
    for component in &mut color {
        *component += difference;
    }
    clip_color(color)
}

fn clip_color(mut color: [f32; 3]) -> [f32; 3] {
    let lum = luminosity(color);
    let min = color.into_iter().fold(f32::INFINITY, f32::min);
    let max = color.into_iter().fold(f32::NEG_INFINITY, f32::max);
    if min < 0.0 && (lum - min).abs() > f32::EPSILON {
        for component in &mut color {
            *component = lum + ((*component - lum) * lum) / (lum - min);
        }
    }
    if max > 1.0 && (max - lum).abs() > f32::EPSILON {
        for component in &mut color {
            *component = lum + ((*component - lum) * (1.0 - lum)) / (max - lum);
        }
    }
    color.map(|component| component.clamp(0.0, 1.0))
}

fn set_sat(mut color: [f32; 3], target: f32) -> [f32; 3] {
    let mut indices = [0usize, 1, 2];
    indices.sort_by(|left, right| color[*left].total_cmp(&color[*right]));
    let min = indices[0];
    let middle = indices[1];
    let max = indices[2];
    if color[max] > color[min] {
        color[middle] = ((color[middle] - color[min]) * target) / (color[max] - color[min]);
        color[max] = target;
    } else {
        color[middle] = 0.0;
        color[max] = 0.0;
    }
    color[min] = 0.0;
    color
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opaque(r: u8, g: u8, b: u8) -> Color {
        Color::from_rgba8(r, g, b, 255)
    }

    fn native_geometry_scene() -> Pixmap {
        let mut pixmap = Pixmap::new(64, 64).unwrap();
        pixmap.fill(opaque(248, 247, 244));

        let mut shape = PathBuilder::new();
        shape.move_to(4.0, 50.0);
        shape.cubic_to(8.0, 2.0, 43.0, 4.0, 60.0, 43.0);
        shape.line_to(48.0, 58.0);
        shape.quad_to(28.0, 44.0, 4.0, 50.0);
        shape.close();
        let shape = shape.finish().unwrap();
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba(0.86, 0.12, 0.18, 0.82).unwrap());
        pixmap.fill_path(
            &shape,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );

        let mut clip_builder = PathBuilder::new();
        clip_builder.move_to(50.0, 32.0);
        clip_builder.cubic_to(50.0, 42.0, 42.0, 50.0, 32.0, 50.0);
        clip_builder.cubic_to(22.0, 50.0, 14.0, 42.0, 14.0, 32.0);
        clip_builder.cubic_to(14.0, 22.0, 22.0, 14.0, 32.0, 14.0);
        clip_builder.cubic_to(42.0, 14.0, 50.0, 22.0, 50.0, 32.0);
        clip_builder.close();
        let clip_path = clip_builder.finish().unwrap();
        let mut clip = Mask::new(64, 64).unwrap();
        clip.fill_path(&clip_path, FillRule::Winding, true, Transform::identity());
        let rectangle = PathBuilder::from_rect(Rect::from_xywh(8.0, 18.0, 50.0, 28.0).unwrap());
        let mut clipped = Paint::default();
        clipped.set_color(Color::from_rgba(0.08, 0.62, 0.35, 0.7).unwrap());
        clipped.blend_mode = BlendMode::Multiply;
        pixmap.fill_path(
            &rectangle,
            &clipped,
            FillRule::Winding,
            Transform::identity(),
            Some(&clip),
        );

        let mut line = PathBuilder::new();
        line.move_to(5.0, 10.0);
        line.cubic_to(20.0, 27.0, 40.0, -2.0, 59.0, 15.0);
        let line = line.finish().unwrap();
        let mut stroke_paint = Paint::default();
        stroke_paint.set_color(Color::from_rgba(0.08, 0.22, 0.88, 0.9).unwrap());
        let stroke = Stroke {
            width: 3.5,
            miter_limit: 5.0,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            dash: StrokeDash::new(vec![7.0, 3.0, 2.0, 3.0], 1.5),
        };
        pixmap.stroke_path(
            &line,
            &stroke_paint,
            &stroke,
            Transform::from_row(1.0, 0.06, -0.08, 1.0, 1.5, -0.5),
            None,
        );
        pixmap
    }

    fn native_paint_scene() -> Pixmap {
        let mut pixmap = Pixmap::new(64, 64).unwrap();
        let page = PathBuilder::from_rect(Rect::from_xywh(0.0, 0.0, 64.0, 64.0).unwrap());
        let mut gradient = Paint::default();
        gradient.shader = LinearGradient::new(
            Point::from_xy(3.0, 4.0),
            Point::from_xy(61.0, 57.0),
            vec![
                GradientStop::new(0.0, opaque(16, 43, 120)),
                GradientStop::new(0.42, opaque(36, 190, 154)),
                GradientStop::new(1.0, opaque(252, 200, 52)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap();
        pixmap.fill_path(
            &page,
            &gradient,
            FillRule::Winding,
            Transform::identity(),
            None,
        );

        let mut circle = PathBuilder::new();
        circle.move_to(51.0, 29.0);
        circle.cubic_to(51.0, 41.2, 41.2, 51.0, 29.0, 51.0);
        circle.cubic_to(16.8, 51.0, 7.0, 41.2, 7.0, 29.0);
        circle.cubic_to(7.0, 16.8, 16.8, 7.0, 29.0, 7.0);
        circle.cubic_to(41.2, 7.0, 51.0, 16.8, 51.0, 29.0);
        circle.close();
        let circle = circle.finish().unwrap();
        let mut radial = Paint::default();
        radial.shader = RadialGradient::new(
            Point::from_xy(24.0, 24.0),
            Point::from_xy(29.0, 29.0),
            22.0,
            vec![
                GradientStop::new(0.0, Color::from_rgba(1.0, 0.15, 0.38, 0.9).unwrap()),
                GradientStop::new(0.58, Color::from_rgba(0.35, 0.12, 0.8, 0.72).unwrap()),
                GradientStop::new(1.0, Color::from_rgba(0.02, 0.02, 0.12, 0.2).unwrap()),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap();
        radial.blend_mode = BlendMode::Screen;
        pixmap.fill_path(
            &circle,
            &radial,
            FillRule::Winding,
            Transform::identity(),
            None,
        );

        let overlay = PathBuilder::from_rect(Rect::from_xywh(34.0, 9.0, 24.0, 28.0).unwrap());
        let mut overlay_paint = Paint::default();
        overlay_paint.set_color(Color::from_rgba(0.92, 0.18, 0.65, 0.62).unwrap());
        overlay_paint.blend_mode = BlendMode::Overlay;
        pixmap.fill_path(
            &overlay,
            &overlay_paint,
            FillRule::Winding,
            Transform::from_rotate(7.0),
            None,
        );

        let mut source = Pixmap::new(2, 2).unwrap();
        source.data_mut().copy_from_slice(&[
            255, 40, 20, 255, 20, 220, 80, 255, 20, 80, 255, 255, 240, 240, 240, 255,
        ]);
        let image_paint = PixmapPaint {
            opacity: 0.82,
            blend_mode: BlendMode::Luminosity,
            quality: FilterQuality::Bilinear,
        };
        pixmap.draw_pixmap(
            0,
            0,
            source.as_ref(),
            &image_paint,
            Transform::from_row(9.0, 1.5, -1.2, 9.0, 39.0, 40.0),
            None,
        );
        pixmap
    }

    fn block_contract(data: &[u8], width: usize, height: usize) -> Vec<u8> {
        let mut output = Vec::with_capacity(8 * 8 * 4);
        for block_y in 0..8 {
            for block_x in 0..8 {
                let start_x = block_x * width / 8;
                let end_x = (block_x + 1) * width / 8;
                let start_y = block_y * height / 8;
                let end_y = (block_y + 1) * height / 8;
                let count = ((end_x - start_x) * (end_y - start_y)) as u32;
                let mut sums = [0u32; 4];
                for y in start_y..end_y {
                    for x in start_x..end_x {
                        let index = (y * width + x) * 4;
                        for channel in 0..4 {
                            sums[channel] += u32::from(data[index + channel]);
                        }
                    }
                }
                output.extend(sums.map(|sum| ((sum + count / 2) / count) as u8));
            }
        }
        output
    }

    fn decode_hex_contract(value: &str) -> Vec<u8> {
        assert_eq!(value.len() % 2, 0);
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let digit = |byte: u8| match byte {
                    b'0'..=b'9' => byte - b'0',
                    b'a'..=b'f' => byte - b'a' + 10,
                    _ => panic!("invalid frozen hex contract"),
                };
                (digit(pair[0]) << 4) | digit(pair[1])
            })
            .collect()
    }

    #[test]
    fn geometry_scene_stays_within_frozen_reference_contract() {
        let native = block_contract(native_geometry_scene().data(), 64, 64);
        let reference = decode_hex_contract(concat!(
            "f8f7f4fff8f7f4fff8f7f4fff8f7f4fff8f7f4fff8f7f4fff8f7f4fff8f7f4ff",
            "c2caefffadb8edffc9ceefffaeb2e6ff9ca1e2ffa7b3edff9eabecffe6e8f2ff",
            "f8f7f4ffe3c5d2ff6a3c5aff6c383cff753e3bff9a998efff4f4f4ffe4e6f2ff",
            "f8f6f4ffcc6268ff50342dff50342dff50342dff50342dffd6adadfff8f7f4ff",
            "f3d4d4ffc8434cff50342dff50342dff50342dff50342dffc94b53fff6e4e3ff",
            "efafb2ffe14652ff8f3c3dff743936ff743936ff8f3c3dffe14652fff1c0c2ff",
            "f6e6e4fff6e8e7fff7eeecfff4d9d9ffeda5a9ffe35863ffea8d93fff8f7f4ff",
            "f8f7f4fff8f7f4fff8f7f4fff8f7f4fff8f7f4fff7eeecfff7f3f0fff8f7f4ff",
        ));
        assert_eq!(native.len(), reference.len());
        let total_error: u32 = native
            .iter()
            .zip(&reference)
            .map(|(left, right)| u32::from(left.abs_diff(*right)))
            .sum();
        let max_error = native
            .iter()
            .zip(&reference)
            .map(|(left, right)| left.abs_diff(*right))
            .max()
            .unwrap_or(0);
        let mean_error = total_error as f32 / native.len() as f32;
        assert!(
            mean_error <= 1.0 && max_error <= 4,
            "geometry perceptual contract: mean={mean_error:.3}, max={max_error}"
        );
    }

    #[test]
    fn paint_scene_stays_within_frozen_reference_contract() {
        let native = block_contract(native_paint_scene().data(), 64, 64);
        let reference = decode_hex_contract(concat!(
            "113179ff14497fff186385ff1b7d8bff1f9891ff23b197ff33be93ff4fc086ff",
            "14467eff196186ff2e7fa2ff409ab5ff3eaaafff3ab99aff50be88ff69c17aff",
            "175e84ff2e7da1ff809ec8ff9cb5c8ffa4abd5ff86a9b1ff9aa992ff83c26eff",
            "1a768aff3f96b4ff9ab4c8ffbac1c6ffc7afd2ffb4acb2ffb7ad84ff9cc461ff",
            "1e8e8fff33abaaff6bc2ccff9bbfccffc5afc9ffc5aca0ffc2b173ffb5c555ff",
            "21a695ff2cbc99ff52c1a2ff75c2a3ff88b484ff7b8b41ffa9a644ffcdc447ff",
            "28ba97ff41bf8dff5dc180ff79c275ff7dab4fff7f9326ffcfc94fffe8c73dff",
            "3ebf8eff5ac180ff76c273ff92c366ffa7bd52ffbab53cfff1d764fffac835ff",
        ));
        assert_eq!(native.len(), reference.len());
        let total_error: u32 = native
            .iter()
            .zip(&reference)
            .map(|(left, right)| u32::from(left.abs_diff(*right)))
            .sum();
        let max_error = native
            .iter()
            .zip(&reference)
            .map(|(left, right)| left.abs_diff(*right))
            .max()
            .unwrap_or(0);
        let mean_error = total_error as f32 / native.len() as f32;
        assert!(
            mean_error <= 2.0 && max_error <= 16,
            "paint perceptual contract: mean={mean_error:.3}, max={max_error}"
        );
    }

    #[test]
    fn transform_concatenation_and_inverse_round_trip() {
        let transform = Transform::from_scale(2.0, 3.0)
            .pre_concat(Transform::from_translate(4.0, -2.0))
            .pre_concat(Transform::from_rotate(17.0));
        let point = Point::from_xy(8.0, 5.0);
        let mapped = transform.map(point);
        let restored = transform.inverse().expect("invertible").map(mapped);
        assert!((restored.x - point.x).abs() < 1.0e-4);
        assert!((restored.y - point.y).abs() < 1.0e-4);
    }

    #[test]
    fn winding_and_evenodd_fills_preserve_holes() {
        let mut builder = PathBuilder::new();
        for (x, y, width, height) in [(1.0, 1.0, 10.0, 10.0), (3.0, 3.0, 6.0, 6.0)] {
            builder.move_to(x, y);
            builder.line_to(x + width, y);
            builder.line_to(x + width, y + height);
            builder.line_to(x, y + height);
            builder.close();
        }
        let path = builder.finish().unwrap();
        let mut evenodd = Pixmap::new(12, 12).unwrap();
        let mut paint = Paint::default();
        paint.set_color(opaque(255, 0, 0));
        evenodd.fill_path(
            &path,
            &paint,
            FillRule::EvenOdd,
            Transform::identity(),
            None,
        );
        assert_eq!(evenodd.data[(6 * 12 + 6) * 4 + 3], 0);
        assert_eq!(evenodd.data[(2 * 12 + 2) * 4 + 3], 255);

        let mut winding = Pixmap::new(12, 12).unwrap();
        winding.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
        assert_eq!(winding.data[(6 * 12 + 6) * 4 + 3], 255);
    }

    #[test]
    fn clipping_and_source_over_use_premultiplied_pixels() {
        let mut pixmap = Pixmap::new(8, 4).unwrap();
        pixmap.fill(opaque(255, 255, 255));
        let clip_path = PathBuilder::from_rect(Rect::from_xywh(0.0, 0.0, 4.0, 4.0).unwrap());
        let mut mask = Mask::new(8, 4).unwrap();
        mask.fill_path(&clip_path, FillRule::Winding, false, Transform::identity());
        let path = PathBuilder::from_rect(Rect::from_xywh(0.0, 0.0, 8.0, 4.0).unwrap());
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba(1.0, 0.0, 0.0, 0.5).unwrap());
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            Some(&mask),
        );
        assert!(pixmap.data[0] > pixmap.data[1]);
        assert_eq!(&pixmap.data[(6 * 4)..(6 * 4 + 4)], &[255, 255, 255, 255]);
    }

    #[test]
    fn dashed_round_stroke_produces_separated_coverage() {
        let mut builder = PathBuilder::new();
        builder.move_to(2.0, 8.0);
        builder.line_to(30.0, 8.0);
        let path = builder.finish().unwrap();
        let mut paint = Paint::default();
        paint.set_color(opaque(0, 0, 0));
        let stroke = Stroke {
            width: 4.0,
            line_cap: LineCap::Round,
            dash: StrokeDash::new(vec![6.0, 8.0], 0.0),
            ..Stroke::default()
        };
        let mut pixmap = Pixmap::new(32, 16).unwrap();
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        assert!(pixmap.data[(8 * 32 + 3) * 4 + 3] > 0);
        assert_eq!(pixmap.data[(8 * 32 + 11) * 4 + 3], 0);
        assert!(pixmap.data[(8 * 32 + 17) * 4 + 3] > 0);
    }

    #[test]
    fn bilinear_pixmap_draw_respects_affine_placement() {
        let mut source = Pixmap::new(2, 1).unwrap();
        source.data_mut()[0..4].copy_from_slice(&[255, 0, 0, 255]);
        source.data_mut()[4..8].copy_from_slice(&[0, 0, 255, 255]);
        let mut destination = Pixmap::new(8, 4).unwrap();
        let paint = PixmapPaint {
            quality: FilterQuality::Bilinear,
            ..PixmapPaint::default()
        };
        destination.draw_pixmap(
            0,
            0,
            source.as_ref(),
            &paint,
            Transform::from_row(3.0, 0.0, 0.0, 2.0, 1.0, 1.0),
            None,
        );
        let left = (1 * 8 + 1) * 4;
        let right = (1 * 8 + 6) * 4;
        assert!(destination.data[left] > destination.data[left + 2]);
        assert!(destination.data[right + 2] > destination.data[right]);
    }

    #[test]
    fn blend_modes_match_reference_endpoints() {
        assert_eq!(blend_rgb(BlendMode::Multiply, [0.4; 3], [0.5; 3]), [0.2; 3]);
        assert_eq!(blend_rgb(BlendMode::Screen, [0.0; 3], [0.7; 3]), [0.7; 3]);
        assert_eq!(
            blend_rgb(BlendMode::Difference, [0.2; 3], [0.8; 3]),
            [0.6; 3]
        );
        for mode in [
            BlendMode::Hue,
            BlendMode::Saturation,
            BlendMode::Color,
            BlendMode::Luminosity,
        ] {
            assert!(
                blend_rgb(mode, [0.2, 0.6, 0.9], [0.8, 0.3, 0.1])
                    .into_iter()
                    .all(|value| (0.0..=1.0).contains(&value))
            );
        }
    }
}
