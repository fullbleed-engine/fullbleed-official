use crate::canvas::{Canvas, META_DIAGNOSTIC_SCOPE_BEGIN_KEY, META_DIAGNOSTIC_SCOPE_END_KEY};
use crate::font::FontRegistry;
use crate::perf::PerfLogger;
use crate::style::{DirectionMode, ObjectFitMode};
use crate::svg;
use crate::types::{BoxSizingMode, Color, MixBlendMode, Pt, Rect, Shading, ShadingStop, Size};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path as FsPath;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const SOFT_HYPHEN: char = '\u{00AD}';

fn huge_pt() -> Pt {
    // Large but safe sentinel for "unbounded" layout measurements.
    Pt::from_f32(1.0e9)
}

fn table_debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("FULLBLEED_TABLE_DEBUG")
            .ok()
            .map(|v| {
                let v = v.trim();
                v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
            })
            .unwrap_or(false)
    })
}

fn table_debug_verbose_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("FULLBLEED_TABLE_DEBUG_VERBOSE")
            .ok()
            .map(|v| {
                let v = v.trim();
                v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
            })
            .unwrap_or(false)
    })
}

fn image_intrinsic_size_pt(source: &str) -> Option<(Pt, Pt)> {
    let bytes = if let Some((_, data)) = parse_image_data_uri(source) {
        data
    } else {
        std::fs::read(FsPath::new(source)).ok()?
    };
    let (width, height) = crate::image_native::dimensions(&bytes).ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    Some((css_px_to_pt(width as f32), css_px_to_pt(height as f32)))
}

fn parse_image_data_uri(uri: &str) -> Option<(String, Vec<u8>)> {
    if !uri.starts_with("data:") {
        return None;
    }
    let (header, payload) = uri.split_once(',')?;
    let mime = header
        .trim_start_matches("data:")
        .split(';')
        .next()
        .filter(|v| !v.is_empty())
        .unwrap_or("application/octet-stream")
        .to_string();
    let data = if header.contains(";base64") {
        crate::base64::decode_standard(payload).ok()?
    } else {
        payload.as_bytes().to_vec()
    };
    Some((mime, data))
}

fn css_px_to_pt(px: f32) -> Pt {
    Pt::from_f32(px * 0.75)
}

struct PerfContext {
    logger: Arc<PerfLogger>,
    doc_id: Option<usize>,
}

pub(crate) struct PerfGuard {
    prev: Option<PerfContext>,
}

thread_local! {
    static PERF_CTX: RefCell<Option<PerfContext>> = RefCell::new(None);
}

pub(crate) fn set_perf_context(perf: Option<Arc<PerfLogger>>, doc_id: Option<usize>) -> PerfGuard {
    let next = perf.map(|logger| PerfContext { logger, doc_id });
    PERF_CTX.with(|ctx| {
        let mut slot = ctx.borrow_mut();
        let prev = slot.take();
        *slot = next;
        PerfGuard { prev }
    })
}

impl Drop for PerfGuard {
    fn drop(&mut self) {
        let prev = self.prev.take();
        PERF_CTX.with(|ctx| {
            *ctx.borrow_mut() = prev;
        });
    }
}

fn perf_enabled() -> bool {
    PERF_CTX.with(|ctx| ctx.borrow().is_some())
}

fn perf_start() -> Option<Instant> {
    if perf_enabled() {
        Some(Instant::now())
    } else {
        None
    }
}

fn log_perf_span(name: &str, start: Instant) {
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    PERF_CTX.with(|ctx| {
        if let Some(ctx) = ctx.borrow().as_ref() {
            ctx.logger.log_span_ms(name, ctx.doc_id, ms);
        }
    });
}

fn log_perf_counts(name: &str, counts: &[(&str, u64)]) {
    PERF_CTX.with(|ctx| {
        if let Some(ctx) = ctx.borrow().as_ref() {
            ctx.logger.log_counts(name, ctx.doc_id, counts);
        }
    });
}

fn perf_end(name: &str, start: Option<Instant>) {
    if let Some(start) = start {
        log_perf_span(name, start);
    }
}

#[derive(Debug, Clone)]
struct LineLayout {
    text: String,
    width: Pt,
    text_width: Pt,
    indent: Pt,
    forced_start: bool,
}

#[derive(Debug, Clone)]
struct PendingLineLayout {
    text: String,
    forced_start: bool,
}

#[derive(Debug, Default)]
struct TextLayoutCache {
    entries: Vec<(i64, Arc<Vec<LineLayout>>)>,
}

#[derive(Debug, Default)]
struct TextWidthCache {
    entries: Vec<(Arc<str>, Pt)>,
}

impl TextWidthCache {
    fn get(&self, key: &str) -> Option<Pt> {
        self.entries
            .iter()
            .find_map(|(k, v)| if k.as_ref() == key { Some(*v) } else { None })
    }

    fn insert(&mut self, key: &str, value: Pt) {
        if let Some(pos) = self.entries.iter().position(|(k, _)| k.as_ref() == key) {
            self.entries.remove(pos);
        }
        self.entries.push((Arc::<str>::from(key), value));
        const MAX_ENTRIES: usize = 64;
        if self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
        }
    }
}

impl TextLayoutCache {
    fn get(&self, key: i64) -> Option<Arc<Vec<LineLayout>>> {
        self.entries
            .iter()
            .find_map(|(k, v)| if *k == key { Some(v.clone()) } else { None })
    }

    fn insert(&mut self, key: i64, value: Arc<Vec<LineLayout>>) {
        if let Some(pos) = self.entries.iter().position(|(k, _)| *k == key) {
            self.entries.remove(pos);
        }
        self.entries.push((key, value));
        const MAX_ENTRIES: usize = 4;
        if self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakBefore {
    Auto,
    Avoid,
    Always,
    All,
    AvoidPage,
    Page,
    Left,
    Right,
    Recto,
    Verso,
    AvoidColumn,
    Column,
    AvoidRegion,
    Region,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakAfter {
    Auto,
    Avoid,
    Always,
    All,
    AvoidPage,
    Page,
    Left,
    Right,
    Recto,
    Verso,
    AvoidColumn,
    Column,
    AvoidRegion,
    Region,
}

impl BreakBefore {
    pub fn forces_page(self) -> bool {
        matches!(
            self,
            BreakBefore::Always
                | BreakBefore::All
                | BreakBefore::Page
                | BreakBefore::Left
                | BreakBefore::Right
                | BreakBefore::Recto
                | BreakBefore::Verso
        )
    }
}

impl BreakAfter {
    pub fn forces_page(self) -> bool {
        matches!(
            self,
            BreakAfter::Always
                | BreakAfter::All
                | BreakAfter::Page
                | BreakAfter::Left
                | BreakAfter::Right
                | BreakAfter::Recto
                | BreakAfter::Verso
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakInside {
    Auto,
    Avoid,
    AvoidPage,
    AvoidColumn,
    AvoidRegion,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pagination {
    pub break_before: BreakBefore,
    pub break_after: BreakAfter,
    pub break_inside: BreakInside,
    pub orphans: usize,
    pub widows: usize,
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            break_before: BreakBefore::Auto,
            break_after: BreakAfter::Auto,
            break_inside: BreakInside::Auto,
            orphans: 2,
            widows: 2,
        }
    }
}

impl Pagination {
    fn resolved_orphans(self) -> usize {
        self.orphans.max(1)
    }

    fn resolved_widows(self) -> usize {
        self.widows.max(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthSpec {
    Auto,
    Absolute(Pt),
    Percent(f32),
    Em(f32),
    Rem(f32),
    Calc(CalcLength),
    Inherit,
    Initial,
}

/// The sizing component of an explicit CSS grid track.
///
/// Keeping this representation compact lets the style layer retain the
/// authored track contract without invoking grid layout for non-grid nodes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridTrackBreadth {
    Auto,
    Length(LengthSpec),
    Fraction(f32),
    MinContent,
    MaxContent,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridTrackSize {
    pub min: GridTrackBreadth,
    pub max: GridTrackBreadth,
}

impl GridTrackSize {
    pub const fn auto() -> Self {
        Self {
            min: GridTrackBreadth::Auto,
            max: GridTrackBreadth::Auto,
        }
    }

    pub const fn fixed(length: LengthSpec) -> Self {
        Self {
            min: GridTrackBreadth::Length(length),
            max: GridTrackBreadth::Length(length),
        }
    }

    pub const fn fraction(factor: f32) -> Self {
        Self {
            min: GridTrackBreadth::Auto,
            max: GridTrackBreadth::Fraction(factor),
        }
    }

    pub(crate) fn fixed_breadth(self) -> Option<LengthSpec> {
        match (self.min, self.max) {
            (GridTrackBreadth::Length(min), GridTrackBreadth::Length(max)) if min == max => {
                Some(max)
            }
            (_, GridTrackBreadth::Length(max)) => Some(max),
            _ => None,
        }
    }

    pub(crate) fn fraction_factor(self) -> Option<f32> {
        match self.max {
            GridTrackBreadth::Fraction(factor) if factor.is_finite() && factor > 0.0 => {
                Some(factor)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TabSizeSpec {
    Spaces(f32),
    Length(LengthSpec),
    Inherit,
    Initial,
}

impl TabSizeSpec {
    pub fn initial() -> Self {
        Self::Spaces(8.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalcLength {
    pub abs: Pt,
    pub percent: f32,
    pub em: f32,
    pub rem: f32,
}

impl CalcLength {
    pub fn zero() -> Self {
        Self {
            abs: Pt::ZERO,
            percent: 0.0,
            em: 0.0,
            rem: 0.0,
        }
    }

    pub fn resolve(self, avail: Pt, font_size: Pt, root_font_size: Pt) -> Pt {
        self.abs + (avail * self.percent) + (font_size * self.em) + (root_font_size * self.rem)
    }
}

impl LengthSpec {
    pub(crate) fn resolve_width(self, avail_width: Pt, font_size: Pt, root_font_size: Pt) -> Pt {
        let value = match self {
            LengthSpec::Auto => Pt::ZERO,
            LengthSpec::Absolute(value) => value,
            LengthSpec::Percent(value) => avail_width * value,
            LengthSpec::Em(value) => font_size * value,
            LengthSpec::Rem(value) => root_font_size * value,
            LengthSpec::Calc(calc) => calc.resolve(avail_width, font_size, root_font_size),
            LengthSpec::Inherit | LengthSpec::Initial => Pt::ZERO,
        };
        value
    }

    pub(crate) fn resolve_height(self, avail_height: Pt, font_size: Pt, root_font_size: Pt) -> Pt {
        let value = match self {
            LengthSpec::Auto => Pt::ZERO,
            LengthSpec::Absolute(value) => value,
            LengthSpec::Percent(value) => avail_height * value,
            LengthSpec::Em(value) => font_size * value,
            LengthSpec::Rem(value) => root_font_size * value,
            LengthSpec::Calc(calc) => calc.resolve(avail_height, font_size, root_font_size),
            LengthSpec::Inherit | LengthSpec::Initial => Pt::ZERO,
        };
        value
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CssTransformOp {
    Translate {
        x: LengthSpec,
        y: LengthSpec,
    },
    Scale {
        x: f32,
        y: f32,
    },
    Rotate {
        radians: f32,
    },
    Skew {
        x_radians: f32,
        y_radians: f32,
    },
    Matrix {
        a: f32,
        b: f32,
        c: f32,
        d: f32,
        e: Pt,
        f: Pt,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CssTransformOrigin {
    pub x: LengthSpec,
    pub y: LengthSpec,
}

impl CssTransformOrigin {
    pub fn center() -> Self {
        Self {
            x: LengthSpec::Percent(0.5),
            y: LengthSpec::Percent(0.5),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeSizes {
    pub top: LengthSpec,
    pub right: LengthSpec,
    pub bottom: LengthSpec,
    pub left: LengthSpec,
}

impl EdgeSizes {
    pub fn zero() -> Self {
        Self {
            top: LengthSpec::Absolute(Pt::ZERO),
            right: LengthSpec::Absolute(Pt::ZERO),
            bottom: LengthSpec::Absolute(Pt::ZERO),
            left: LengthSpec::Absolute(Pt::ZERO),
        }
    }

    fn resolve(self, avail_width: Pt, font_size: Pt, root_font_size: Pt) -> ResolvedEdges {
        ResolvedEdges {
            top: self
                .top
                .resolve_width(avail_width, font_size, root_font_size),
            right: self
                .right
                .resolve_width(avail_width, font_size, root_font_size),
            bottom: self
                .bottom
                .resolve_width(avail_width, font_size, root_font_size),
            left: self
                .left
                .resolve_width(avail_width, font_size, root_font_size),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderCollapseMode {
    Collapse,
    Separate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableLayoutMode {
    Auto,
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderSpacingSpec {
    pub horizontal: LengthSpec,
    pub vertical: LengthSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineLineStyle {
    Solid,
    Dotted,
    Dashed,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
}

impl BorderSpacingSpec {
    pub fn zero() -> Self {
        Self {
            horizontal: LengthSpec::Absolute(Pt::ZERO),
            vertical: LengthSpec::Absolute(Pt::ZERO),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderRadiusSpec {
    pub top_left: LengthSpec,
    pub top_right: LengthSpec,
    pub bottom_right: LengthSpec,
    pub bottom_left: LengthSpec,
}

impl BorderRadiusSpec {
    pub fn zero() -> Self {
        Self {
            top_left: LengthSpec::Absolute(Pt::ZERO),
            top_right: LengthSpec::Absolute(Pt::ZERO),
            bottom_right: LengthSpec::Absolute(Pt::ZERO),
            bottom_left: LengthSpec::Absolute(Pt::ZERO),
        }
    }

    pub fn resolve(
        &self,
        avail_width: Pt,
        font_size: Pt,
        root_font_size: Pt,
    ) -> ResolvedBorderRadius {
        ResolvedBorderRadius {
            top_left: self
                .top_left
                .resolve_width(avail_width, font_size, root_font_size),
            top_right: self
                .top_right
                .resolve_width(avail_width, font_size, root_font_size),
            bottom_right: self
                .bottom_right
                .resolve_width(avail_width, font_size, root_font_size),
            bottom_left: self
                .bottom_left
                .resolve_width(avail_width, font_size, root_font_size),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderRadiiSpec {
    pub horizontal: BorderRadiusSpec,
    pub vertical: BorderRadiusSpec,
}

impl BorderRadiiSpec {
    pub fn zero() -> Self {
        Self {
            horizontal: BorderRadiusSpec::zero(),
            vertical: BorderRadiusSpec::zero(),
        }
    }

    pub fn circular(radius: BorderRadiusSpec) -> Self {
        Self {
            horizontal: radius,
            vertical: radius,
        }
    }

    fn resolve(
        &self,
        width: Pt,
        height: Pt,
        font_size: Pt,
        root_font_size: Pt,
    ) -> ResolvedClipPathRadii {
        let horizontal = self.horizontal.resolve(width, font_size, root_font_size);
        let vertical = self.vertical.resolve(height, font_size, root_font_size);
        ResolvedClipPathRadii {
            top_left_x: horizontal.top_left,
            top_left_y: vertical.top_left,
            top_right_x: horizontal.top_right,
            top_right_y: vertical.top_right,
            bottom_right_x: horizontal.bottom_right,
            bottom_right_y: vertical.bottom_right,
            bottom_left_x: horizontal.bottom_left,
            bottom_left_y: vertical.bottom_left,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedBorderRadius {
    pub top_left: Pt,
    pub top_right: Pt,
    pub bottom_right: Pt,
    pub bottom_left: Pt,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoxShadowSpec {
    pub offset_x: LengthSpec,
    pub offset_y: LengthSpec,
    pub blur: LengthSpec,
    pub spread: LengthSpec,
    pub color: Color,
    pub opacity: f32,
    pub inset: bool,
    pub color_var: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilterDropShadowSpec {
    pub offset_x: Pt,
    pub offset_y: Pt,
    pub blur_radius: Pt,
    pub color: Color,
    pub opacity: f32,
    pub color_is_current_color: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaintFilterSpec {
    pub saturate: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub invert: f32,
    pub sepia: f32,
    pub hue_rotate: f32,
    pub opacity: f32,
    pub blur_radius: Pt,
    pub drop_shadows: Vec<FilterDropShadowSpec>,
}

impl PaintFilterSpec {
    pub fn identity() -> Self {
        Self {
            saturate: 1.0,
            brightness: 1.0,
            contrast: 1.0,
            invert: 0.0,
            sepia: 0.0,
            hue_rotate: 0.0,
            opacity: 1.0,
            blur_radius: Pt::ZERO,
            drop_shadows: Vec::new(),
        }
    }

    pub fn is_identity(&self) -> bool {
        (self.saturate - 1.0).abs() <= 1.0e-6
            && (self.brightness - 1.0).abs() <= 1.0e-6
            && (self.contrast - 1.0).abs() <= 1.0e-6
            && self.invert.abs() <= 1.0e-6
            && self.sepia.abs() <= 1.0e-6
            && self.hue_rotate.abs() <= 1.0e-6
            && (self.opacity - 1.0).abs() <= 1.0e-6
            && self.blur_radius <= Pt::ZERO
            && self.drop_shadows.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundPaint {
    Image {
        source: String,
    },
    LinearGradient {
        angle_deg: f32,
        stops: Vec<ShadingStop>,
    },
    RadialGradient {
        center_x_pct: f32,
        center_y_pct: f32,
        stops: Vec<ShadingStop>,
    },
    ConicGradient {
        start_angle_deg: f32,
        center_x_pct: f32,
        center_y_pct: f32,
        stops: Vec<ShadingStop>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackgroundPositionComponent {
    Start(LengthSpec),
    Center,
    End(LengthSpec),
}

impl Default for BackgroundPositionComponent {
    fn default() -> Self {
        Self::Start(LengthSpec::Absolute(Pt::ZERO))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackgroundPositionSpec {
    pub x: BackgroundPositionComponent,
    pub y: BackgroundPositionComponent,
}

impl Default for BackgroundPositionSpec {
    fn default() -> Self {
        Self {
            x: BackgroundPositionComponent::Start(LengthSpec::Absolute(Pt::ZERO)),
            y: BackgroundPositionComponent::Start(LengthSpec::Absolute(Pt::ZERO)),
        }
    }
}

impl BackgroundPositionSpec {
    pub fn center() -> Self {
        Self {
            x: BackgroundPositionComponent::Center,
            y: BackgroundPositionComponent::Center,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackgroundRepeatSpec {
    pub x: BackgroundRepeatMode,
    pub y: BackgroundRepeatMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundBox {
    Border,
    Padding,
    Content,
}

impl Default for BackgroundBox {
    fn default() -> Self {
        Self::Padding
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundClipBox {
    Border,
    Padding,
    Content,
}

impl Default for BackgroundClipBox {
    fn default() -> Self {
        Self::Border
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundRepeatMode {
    Repeat,
    NoRepeat,
    Space,
    Round,
}

impl Default for BackgroundRepeatMode {
    fn default() -> Self {
        Self::Repeat
    }
}

impl Default for BackgroundRepeatSpec {
    fn default() -> Self {
        Self {
            x: BackgroundRepeatMode::Repeat,
            y: BackgroundRepeatMode::Repeat,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackgroundSizeSpec {
    pub mode: BackgroundSizeMode,
    pub width: LengthSpec,
    pub height: LengthSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundSizeMode {
    Explicit,
    Cover,
    Contain,
}

impl Default for BackgroundSizeMode {
    fn default() -> Self {
        Self::Explicit
    }
}

impl Default for BackgroundSizeSpec {
    fn default() -> Self {
        Self {
            mode: BackgroundSizeMode::Explicit,
            width: LengthSpec::Auto,
            height: LengthSpec::Auto,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipPathInsetSpec {
    pub top: LengthSpec,
    pub right: LengthSpec,
    pub bottom: LengthSpec,
    pub left: LengthSpec,
    pub radius: Option<ClipPathRadiusSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipPathShapeRadius {
    Length(LengthSpec),
    ClosestSide,
    FarthestSide,
    ClosestCorner,
    FarthestCorner,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipPathCircleSpec {
    pub radius: ClipPathShapeRadius,
    pub center_x: LengthSpec,
    pub center_y: LengthSpec,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClipPathEllipseSpec {
    pub radius_x: ClipPathShapeRadius,
    pub radius_y: ClipPathShapeRadius,
    pub center_x: LengthSpec,
    pub center_y: LengthSpec,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipPathXywhSpec {
    pub x: LengthSpec,
    pub y: LengthSpec,
    pub width: LengthSpec,
    pub height: LengthSpec,
    pub radius: Option<ClipPathRadiusSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipPathRectSpec {
    pub top: LengthSpec,
    pub right: LengthSpec,
    pub bottom: LengthSpec,
    pub left: LengthSpec,
    pub radius: Option<ClipPathRadiusSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipPathRadiusSpec {
    pub horizontal: BorderRadiusSpec,
    pub vertical: BorderRadiusSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipPathReferenceBox {
    Margin,
    Border,
    HalfBorder,
    Padding,
    Content,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClipPathPolygonSpec {
    pub evenodd: bool,
    pub fill_rule_explicit: bool,
    pub points: Vec<(LengthSpec, LengthSpec)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipPathPathCommand {
    MoveTo {
        x: Pt,
        y: Pt,
    },
    LineTo {
        x: Pt,
        y: Pt,
    },
    CurveTo {
        x1: Pt,
        y1: Pt,
        x2: Pt,
        y2: Pt,
        x: Pt,
        y: Pt,
    },
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClipPathPathSpec {
    pub evenodd: bool,
    pub fill_rule_explicit: bool,
    pub data: String,
    pub commands: Vec<ClipPathPathCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipPathShapeControlAnchor {
    Start,
    End,
    Origin,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipPathShapeFunctionCommand {
    MoveTo {
        x: LengthSpec,
        y: LengthSpec,
        relative: bool,
    },
    LineTo {
        x: LengthSpec,
        y: LengthSpec,
        relative: bool,
    },
    HLine {
        x: LengthSpec,
        relative: bool,
    },
    VLine {
        y: LengthSpec,
        relative: bool,
    },
    CurveTo {
        x: LengthSpec,
        y: LengthSpec,
        relative: bool,
        control1_x: LengthSpec,
        control1_y: LengthSpec,
        control1_anchor: ClipPathShapeControlAnchor,
        control2_x: Option<LengthSpec>,
        control2_y: Option<LengthSpec>,
        control2_anchor: Option<ClipPathShapeControlAnchor>,
    },
    SmoothTo {
        x: LengthSpec,
        y: LengthSpec,
        relative: bool,
        control_x: Option<LengthSpec>,
        control_y: Option<LengthSpec>,
        control_anchor: Option<ClipPathShapeControlAnchor>,
    },
    ArcTo {
        x: LengthSpec,
        y: LengthSpec,
        relative: bool,
        radius_x: LengthSpec,
        radius_y: LengthSpec,
        large_arc: bool,
        sweep: bool,
        rotation_deg: f32,
    },
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClipPathShapeFunctionSpec {
    pub evenodd: bool,
    pub fill_rule_explicit: bool,
    pub start_x: LengthSpec,
    pub start_y: LengthSpec,
    pub commands: Vec<ClipPathShapeFunctionCommand>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClipPathShapeSpec {
    Inset(ClipPathInsetSpec),
    Circle(ClipPathCircleSpec),
    Ellipse(ClipPathEllipseSpec),
    Xywh(ClipPathXywhSpec),
    Rect(ClipPathRectSpec),
    Polygon(ClipPathPolygonSpec),
    Path(ClipPathPathSpec),
    ShapeFunction(ClipPathShapeFunctionSpec),
    ReferenceBox,
}

#[derive(Debug, Clone, Copy)]
struct ResolvedEdges {
    top: Pt,
    right: Pt,
    bottom: Pt,
    left: Pt,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ResolvedEdgeColors {
    pub(crate) top: Color,
    pub(crate) right: Color,
    pub(crate) bottom: Color,
    pub(crate) left: Color,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ResolvedEdgeStyles {
    pub(crate) top: OutlineLineStyle,
    pub(crate) right: OutlineLineStyle,
    pub(crate) bottom: OutlineLineStyle,
    pub(crate) left: OutlineLineStyle,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ResolvedEdgeHidden {
    pub(crate) top: bool,
    pub(crate) right: bool,
    pub(crate) bottom: bool,
    pub(crate) left: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BorderSide {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Debug, Clone, Copy)]
struct ResolvedClipPathRadii {
    top_left_x: Pt,
    top_left_y: Pt,
    top_right_x: Pt,
    top_right_y: Pt,
    bottom_right_x: Pt,
    bottom_right_y: Pt,
    bottom_left_x: Pt,
    bottom_left_y: Pt,
}

impl ResolvedEdgeColors {
    fn uniform(color: Color) -> Self {
        Self {
            top: color,
            right: color,
            bottom: color,
            left: color,
        }
    }
}

impl ResolvedEdgeStyles {
    fn uniform(style: OutlineLineStyle) -> Self {
        Self {
            top: style,
            right: style,
            bottom: style,
            left: style,
        }
    }

    fn is_uniform(self) -> bool {
        self.top == self.right && self.top == self.bottom && self.top == self.left
    }

    fn collapsed_table(self) -> Self {
        Self {
            top: collapsed_table_border_style(self.top),
            right: collapsed_table_border_style(self.right),
            bottom: collapsed_table_border_style(self.bottom),
            left: collapsed_table_border_style(self.left),
        }
    }
}

fn collapsed_table_border_style(style: OutlineLineStyle) -> OutlineLineStyle {
    match style {
        OutlineLineStyle::Inset => OutlineLineStyle::Ridge,
        OutlineLineStyle::Outset => OutlineLineStyle::Groove,
        _ => style,
    }
}

fn collapsed_table_style_priority(style: OutlineLineStyle) -> u8 {
    match style {
        OutlineLineStyle::Double => 8,
        OutlineLineStyle::Solid => 7,
        OutlineLineStyle::Dashed => 6,
        OutlineLineStyle::Dotted => 5,
        OutlineLineStyle::Ridge => 4,
        OutlineLineStyle::Outset => 3,
        OutlineLineStyle::Groove => 2,
        OutlineLineStyle::Inset => 1,
    }
}

impl ResolvedEdgeHidden {
    fn none() -> Self {
        Self {
            top: false,
            right: false,
            bottom: false,
            left: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ResolvedBorder {
    widths: ResolvedEdges,
    colors: ResolvedEdgeColors,
    styles: ResolvedEdgeStyles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BorderConflictSource {
    Table,
    ColumnGroup,
    Column,
    RowGroup,
    Row,
    Cell,
}

impl BorderConflictSource {
    fn priority(self) -> u8 {
        match self {
            BorderConflictSource::Table => 0,
            BorderConflictSource::ColumnGroup => 1,
            BorderConflictSource::Column => 2,
            BorderConflictSource::RowGroup => 3,
            BorderConflictSource::Row => 4,
            BorderConflictSource::Cell => 5,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CollapsedBorderEdge {
    width: Pt,
    color: Color,
    style: OutlineLineStyle,
    hidden: bool,
    source: BorderConflictSource,
}

impl CollapsedBorderEdge {
    fn new(
        width: Pt,
        color: Color,
        style: OutlineLineStyle,
        hidden: bool,
        source: BorderConflictSource,
    ) -> Self {
        Self {
            width,
            color,
            style,
            hidden,
            source,
        }
    }
}

pub trait Flowable: FlowableClone + Send + Sync {
    fn wrap(&self, avail_width: Pt, avail_height: Pt) -> Size;
    fn split(
        &self,
        avail_width: Pt,
        avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)>;
    fn draw(&self, canvas: &mut Canvas, x: Pt, y: Pt, avail_width: Pt, avail_height: Pt);

    /// Paint an auto-sized box into a definite cross-axis slot. Most flowables
    /// have no stretchable box of their own, so their normal draw behavior is
    /// already correct. CSS box flowables override this hook.
    fn draw_stretched(&self, canvas: &mut Canvas, x: Pt, y: Pt, avail_width: Pt, avail_height: Pt) {
        self.draw(canvas, x, y, avail_width, avail_height);
    }

    fn intrinsic_width(&self) -> Option<Pt> {
        None
    }

    /// Distance from the flowable's top margin edge to its first inline
    /// baseline. Inline formatting contexts use this to align text and
    /// replaced/inline-block boxes without inspecting concrete flowable types.
    fn first_baseline(&self, _avail_width: Pt) -> Option<Pt> {
        None
    }

    // Out-of-flow items (e.g. position:absolute) should not affect normal flow placement.
    fn out_of_flow(&self) -> bool {
        false
    }

    // Z-index used for out-of-flow stacking order. Higher is drawn later.
    fn z_index(&self) -> i32 {
        0
    }
    fn pagination(&self) -> Pagination {
        Pagination::default()
    }

    // Some flowables (for example relative-position wrappers) need containing-block
    // draw-space dimensions rather than the child's own wrapped height.
    fn prefers_containing_block_draw_space(&self) -> bool {
        false
    }

    fn is_fixed_positioned(&self) -> bool {
        false
    }

    fn debug_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn diagnostic_metadata(&self) -> Vec<(String, String)> {
        Vec::new()
    }
}

pub trait FlowableClone {
    fn clone_box(&self) -> Box<dyn Flowable>;
}

impl<T> FlowableClone for T
where
    T: 'static + Flowable + Clone,
{
    fn clone_box(&self) -> Box<dyn Flowable> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn Flowable> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub font_size: Pt,
    pub line_height: Pt,
    pub line_height_is_auto: bool,
    pub color: Color,
    pub font_name: Arc<str>,
    pub font_fallbacks: Vec<Arc<str>>,
    pub font_weight: u16,
    pub font_style: crate::style::FontStyleMode,
    pub text_decoration: crate::style::TextDecorationMode,
    pub text_decoration_color: Color,
    pub text_decoration_thickness: crate::style::TextDecorationThicknessMode,
    pub text_decoration_style: crate::style::TextDecorationStyleMode,
    pub text_underline_offset: crate::style::TextUnderlineOffsetMode,
    pub text_underline_position: crate::style::TextUnderlinePositionMode,
    pub text_shadows: Vec<BoxShadowSpec>,
    pub text_overflow: crate::style::TextOverflowMode,
    pub text_indent: LengthSpec,
    pub text_indent_hanging: bool,
    pub text_indent_each_line: bool,
    pub word_break: crate::style::WordBreakMode,
    pub line_break: crate::style::LineBreakMode,
    pub text_justify: crate::style::TextJustifyMode,
    pub hyphens: crate::style::HyphensMode,
    pub hyphenate_character: Option<String>,
    pub writing_mode: crate::style::WritingModeMode,
    pub letter_spacing: Pt,
    pub word_spacing: Pt,
    pub tab_size: TabSizeSpec,
    pub root_font_size: Pt,
    pub visible: bool,
    /// CSS layout obtains integer-CSS-pixel font extents from the browser font
    /// substrate before distributing line-height leading. Point-native users
    /// keep exact font metrics by leaving this disabled.
    pub css_pixel_snap_metrics: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        let font_size = Pt::from_f32(12.0);
        Self {
            font_size,
            line_height: font_size.mul_ratio(6, 5),
            line_height_is_auto: true,
            color: Color::BLACK,
            font_name: Arc::<str>::from("Helvetica"),
            font_fallbacks: Vec::new(),
            font_weight: 400,
            font_style: crate::style::FontStyleMode::Normal,
            text_decoration: crate::style::TextDecorationMode::default(),
            text_decoration_color: Color::BLACK,
            text_decoration_thickness: crate::style::TextDecorationThicknessMode::Auto,
            text_decoration_style: crate::style::TextDecorationStyleMode::Solid,
            text_underline_offset: crate::style::TextUnderlineOffsetMode::Auto,
            text_underline_position: crate::style::TextUnderlinePositionMode::auto(),
            text_shadows: Vec::new(),
            text_overflow: crate::style::TextOverflowMode::Clip,
            text_indent: LengthSpec::Absolute(Pt::ZERO),
            text_indent_hanging: false,
            text_indent_each_line: false,
            word_break: crate::style::WordBreakMode::Normal,
            line_break: crate::style::LineBreakMode::Auto,
            text_justify: crate::style::TextJustifyMode::Auto,
            hyphens: crate::style::HyphensMode::Manual,
            hyphenate_character: None,
            writing_mode: crate::style::WritingModeMode::HorizontalTb,
            letter_spacing: Pt::ZERO,
            word_spacing: Pt::ZERO,
            tab_size: TabSizeSpec::initial(),
            root_font_size: font_size,
            visible: true,
            css_pixel_snap_metrics: false,
        }
    }
}

fn text_style_has_spacing(style: &TextStyle) -> bool {
    style.letter_spacing != Pt::ZERO || style.word_spacing != Pt::ZERO
}

fn strip_soft_hyphens(text: &str) -> Option<String> {
    text.contains(SOFT_HYPHEN)
        .then(|| text.chars().filter(|ch| *ch != SOFT_HYPHEN).collect())
}

fn hyphenate_character(style: &TextStyle) -> &str {
    style.hyphenate_character.as_deref().unwrap_or("-")
}

fn text_spacing_extra(style: &TextStyle, text: &str) -> Pt {
    let mut extra = Pt::ZERO;
    if style.letter_spacing != Pt::ZERO {
        let count = text.chars().count();
        if count > 1 {
            extra = extra + style.letter_spacing * ((count - 1) as i32);
        }
    }
    if style.word_spacing != Pt::ZERO {
        let count = text.chars().filter(|ch| *ch == ' ').count();
        if count > 0 {
            extra = extra + style.word_spacing * (count as i32);
        }
    }
    extra
}

fn text_width_with_spacing(base: Pt, style: &TextStyle, text: &str) -> Pt {
    (base + text_spacing_extra(style, text)).max(Pt::ZERO)
}

fn text_spacing_after_char(style: &TextStyle, ch: char, remaining_after_char: usize) -> Pt {
    if remaining_after_char == 0 {
        return Pt::ZERO;
    }
    let mut extra = Pt::ZERO;
    if style.letter_spacing != Pt::ZERO {
        extra = extra + style.letter_spacing;
    }
    if ch == ' ' && style.word_spacing != Pt::ZERO {
        extra = extra + style.word_spacing;
    }
    extra
}

fn resolve_tab_advance(style: &TextStyle, space_width: Pt) -> Pt {
    let value = match style.tab_size {
        TabSizeSpec::Spaces(spaces) if spaces.is_finite() => {
            Pt::from_f32(space_width.to_f32() * spaces.max(0.0))
        }
        TabSizeSpec::Length(spec) => {
            spec.resolve_width(Pt::ZERO, style.font_size, style.root_font_size)
        }
        TabSizeSpec::Spaces(_) | TabSizeSpec::Inherit | TabSizeSpec::Initial => {
            Pt::from_f32(space_width.to_f32() * 8.0)
        }
    };
    value.max(Pt::ZERO)
}

fn ceil_to_css_pixel(value: Pt) -> Pt {
    let milli = value.to_milli_i64();
    if milli <= 0 {
        return value;
    }
    // One CSS px is 72/96 pt.
    Pt::from_milli_i64(((milli + 749) / 750) * 750)
}

fn text_baseline_for_line(
    style: &TextStyle,
    font_registry: Option<&FontRegistry>,
    line_height: Pt,
) -> Pt {
    let metrics = font_registry.and_then(|registry| {
        let (primary, _) = resolve_font_stack(Some(registry), style);
        registry.vertical_metrics(&primary, style.font_size)
    });
    let (mut ascent, mut descent) = metrics.unwrap_or_else(|| {
        // Stable Base-14 fallback. Registered fonts use their exact hhea metrics.
        (style.font_size * 0.8, style.font_size * 0.2)
    });
    if style.css_pixel_snap_metrics {
        ascent = ceil_to_css_pixel(ascent);
        descent = ceil_to_css_pixel(descent);
    }
    let font_box = ascent + descent;
    let half_leading = (line_height - font_box).mul_ratio(1, 2);
    half_leading + ascent
}

fn text_draw_y_for_line(
    style: &TextStyle,
    font_registry: Option<&FontRegistry>,
    line_top: Pt,
    line_height: Pt,
) -> Pt {
    let baseline_from_top = text_baseline_for_line(style, font_registry, line_height);

    // Canvas::draw_string takes the top of a one-em text box and converts it to
    // a PDF baseline by adding font_size. Shift that top so the baseline follows
    // CSS inline formatting metrics instead of sitting one full em below the line.
    line_top + baseline_from_top - style.font_size
}

#[cfg(test)]
mod text_baseline_tests {
    use super::{Pt, TextStyle, text_baseline_for_line, text_draw_y_for_line};

    #[test]
    fn base14_text_uses_css_baseline_instead_of_full_em_offset() {
        let mut style = TextStyle::default();
        style.font_size = Pt::from_f32(10.0);
        style.line_height = Pt::from_f32(10.0);
        style.line_height_is_auto = false;

        let draw_y = text_draw_y_for_line(&style, None, Pt::from_f32(20.0), Pt::from_f32(10.0));
        assert_eq!(draw_y, Pt::from_f32(18.0));
    }

    #[test]
    fn css_font_extents_snap_outward_before_leading_is_distributed() {
        let mut style = TextStyle::default();
        style.font_size = Pt::from_f32(11.0);
        style.line_height = Pt::from_f32(11.0);
        style.line_height_is_auto = false;
        style.css_pixel_snap_metrics = true;

        // Raw Base-14 extents are 8.8pt/2.2pt. CSS snaps those outward to
        // 9pt/2.25pt before distributing the resulting negative half-leading.
        assert_eq!(
            text_baseline_for_line(&style, None, style.line_height),
            Pt::from_f32(8.875)
        );
    }
}

fn expanded_integer_tabs(style: &TextStyle, text: &str) -> Option<String> {
    if !text.contains('\t') {
        return None;
    }
    let TabSizeSpec::Spaces(spaces) = style.tab_size else {
        return None;
    };
    if !spaces.is_finite() {
        return None;
    }
    let rounded = spaces.round();
    if (spaces - rounded).abs() > 1.0e-4 || !(0.0..=64.0).contains(&rounded) {
        return None;
    }
    let spaces = " ".repeat(rounded as usize);
    Some(text.replace('\t', &spaces))
}

fn tabbed_text_width<F>(style: &TextStyle, text: &str, mut measure_plain: F) -> Option<Pt>
where
    F: FnMut(&str) -> Pt,
{
    if !text.contains('\t') {
        return None;
    }
    let space_width = measure_plain(" ");
    let tab_advance = resolve_tab_advance(style, space_width);
    let mut width = Pt::ZERO;
    for (idx, part) in text.split('\t').enumerate() {
        if idx > 0 {
            width = width + tab_advance;
        }
        if !part.is_empty() {
            width = width + measure_plain(part);
        }
    }
    Some(width.max(Pt::ZERO))
}

fn line_width_with_indent(text_width: Pt, indent: Pt) -> Pt {
    if indent >= Pt::ZERO {
        (text_width + indent).max(Pt::ZERO)
    } else {
        text_width.max(Pt::ZERO)
    }
}

fn resolve_font_variant_name(
    registry: Option<&FontRegistry>,
    base: &Arc<str>,
    weight: u16,
    style: crate::style::FontStyleMode,
) -> Arc<str> {
    let italic = matches!(
        style,
        crate::style::FontStyleMode::Italic | crate::style::FontStyleMode::Oblique(_)
    );
    let bold = weight >= 600;
    if !italic && !bold {
        return base.clone();
    }

    let base_str = base.as_ref();
    if let Some(base14) = base14_variant_name(base_str, bold, italic) {
        return Arc::<str>::from(base14);
    }

    let Some(registry) = registry else {
        return base.clone();
    };

    let mut candidates: Vec<String> = Vec::new();
    if bold && italic {
        candidates.push(format!("{base_str} Bold Italic"));
        candidates.push(format!("{base_str} BoldItalic"));
        candidates.push(format!("{base_str}-BoldItalic"));
        candidates.push(format!("{base_str}-BoldItalic"));
        candidates.push(format!("{base_str} Italic Bold"));
    }
    if bold {
        candidates.push(format!("{base_str} Bold"));
        candidates.push(format!("{base_str} SemiBold"));
        candidates.push(format!("{base_str} Semibold"));
        candidates.push(format!("{base_str}-Bold"));
    }
    if italic {
        candidates.push(format!("{base_str} Italic"));
        candidates.push(format!("{base_str} Oblique"));
        candidates.push(format!("{base_str}-Italic"));
    }

    for candidate in candidates {
        if registry.resolve(&candidate).is_some() {
            return Arc::<str>::from(candidate);
        }
    }

    base.clone()
}

fn base14_variant_name(base: &str, bold: bool, italic: bool) -> Option<&'static str> {
    let norm = base
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_lowercase();
    match norm.as_str() {
        "helvetica" => Some(match (bold, italic) {
            (true, true) => "Helvetica-BoldOblique",
            (true, false) => "Helvetica-Bold",
            (false, true) => "Helvetica-Oblique",
            (false, false) => "Helvetica",
        }),
        "times-roman" => Some(match (bold, italic) {
            (true, true) => "Times-BoldItalic",
            (true, false) => "Times-Bold",
            (false, true) => "Times-Italic",
            (false, false) => "Times-Roman",
        }),
        "courier" => Some(match (bold, italic) {
            (true, true) => "Courier-BoldOblique",
            (true, false) => "Courier-Bold",
            (false, true) => "Courier-Oblique",
            (false, false) => "Courier",
        }),
        _ => None,
    }
}

fn draw_text_decorations(
    canvas: &mut Canvas,
    style: &TextStyle,
    font_registry: Option<&FontRegistry>,
    x: Pt,
    y: Pt,
    width: Pt,
) {
    if style.text_decoration.is_none() || width <= Pt::ZERO {
        return;
    }
    let baseline = y + style.font_size;
    let default_thickness = (style.font_size * 0.05).max(Pt::from_f32(0.5));
    let mut overline_y = baseline - style.font_size.mul_ratio(9, 10);
    let mut line_through_y = baseline - style.font_size.mul_ratio(3, 10);
    let mut underline_y = baseline + style.font_size.mul_ratio(1, 10);
    let mut overline_thickness = default_thickness;
    let mut line_through_thickness = default_thickness;
    let mut underline_thickness = default_thickness;

    if let Some(registry) = font_registry {
        let (primary, _) = resolve_font_stack(Some(registry), style);
        if let Some(font) = registry.resolve(&primary) {
            if let Some(metrics) = font.metrics.underline_metrics {
                let pos = metrics.position as i32;
                let thickness = metrics.thickness as i32;
                if thickness > 0 {
                    underline_thickness = style.font_size.mul_ratio(thickness, 1000);
                }
                underline_y = baseline + style.font_size.mul_ratio(-pos, 1000);
            }
            if let Some(metrics) = font.metrics.strikeout_metrics {
                let pos = metrics.position as i32;
                let thickness = metrics.thickness as i32;
                if thickness > 0 {
                    line_through_thickness = style.font_size.mul_ratio(thickness, 1000);
                }
                line_through_y = baseline + style.font_size.mul_ratio(-pos, 1000);
            }
            let mut overline_units = font.metrics.cap_height as i32;
            if overline_units <= 0 {
                overline_units = font.metrics.ascent as i32;
            }
            if overline_units > 0 {
                overline_y = baseline - style.font_size.mul_ratio(overline_units, 1000);
            }
        }
    }
    if style.text_underline_position.is_under() {
        underline_y = baseline + style.font_size.mul_ratio(7, 25);
    }
    if let Some(thickness) = explicit_text_decoration_thickness(style) {
        overline_thickness = thickness;
        line_through_thickness = thickness;
        underline_thickness = thickness;
    }
    if let Some(offset) = explicit_text_underline_offset(style) {
        underline_y = underline_y + offset;
    }

    canvas.save_state();
    canvas.set_stroke_color(style.text_decoration_color);
    if style.text_decoration.overline {
        draw_styled_text_decoration_line(canvas, style, x, overline_y, width, overline_thickness);
    }
    if style.text_decoration.line_through {
        draw_styled_text_decoration_line(
            canvas,
            style,
            x,
            line_through_y,
            width,
            line_through_thickness,
        );
    }
    if style.text_decoration.underline {
        draw_styled_text_decoration_line(canvas, style, x, underline_y, width, underline_thickness);
    }
    canvas.restore_state();
}

fn stroke_text_decoration_segment(canvas: &mut Canvas, x: Pt, y: Pt, width: Pt) {
    canvas.move_to(x, y);
    canvas.line_to(x + width, y);
    canvas.stroke();
}

fn draw_solid_text_decoration_line(canvas: &mut Canvas, x: Pt, y: Pt, width: Pt, thickness: Pt) {
    canvas.set_line_cap(0);
    canvas.set_dash(Vec::new(), Pt::ZERO);
    canvas.set_line_width(thickness);
    stroke_text_decoration_segment(canvas, x, y, width);
}

fn draw_wavy_text_decoration_line(canvas: &mut Canvas, x: Pt, y: Pt, width: Pt, thickness: Pt) {
    if width <= Pt::ZERO {
        return;
    }
    let amplitude = (thickness * 1.25).max(Pt::from_f32(1.0));
    let wave = (thickness * 6.0).max(Pt::from_f32(6.0));
    let half_wave = wave / 2.0;
    let control_delta = half_wave / 2.0;
    let mut cursor = Pt::ZERO;
    let mut up = true;
    canvas.set_line_cap(0);
    canvas.set_dash(Vec::new(), Pt::ZERO);
    canvas.set_line_width(thickness);
    canvas.move_to(x, y);
    while cursor < width {
        let segment = half_wave.min(width - cursor);
        let end_x = x + cursor + segment;
        let end_y = y;
        let control_y = if up { y - amplitude } else { y + amplitude };
        let c1x = x + cursor + control_delta.min(segment);
        let c2x = end_x - control_delta.min(segment);
        canvas.curve_to(c1x, control_y, c2x, control_y, end_x, end_y);
        cursor = cursor + segment;
        up = !up;
    }
    canvas.stroke();
}

fn draw_styled_text_decoration_line(
    canvas: &mut Canvas,
    style: &TextStyle,
    x: Pt,
    y: Pt,
    width: Pt,
    thickness: Pt,
) {
    match style.text_decoration_style {
        crate::style::TextDecorationStyleMode::Solid => {
            draw_solid_text_decoration_line(canvas, x, y, width, thickness);
        }
        crate::style::TextDecorationStyleMode::Double => {
            let gap = (thickness * 2.0).max(Pt::from_f32(1.0));
            draw_solid_text_decoration_line(canvas, x, y - gap, width, thickness);
            draw_solid_text_decoration_line(canvas, x, y + gap, width, thickness);
        }
        crate::style::TextDecorationStyleMode::Dotted => {
            canvas.set_line_cap(1);
            canvas.set_dash(
                vec![(thickness * 0.01).max(Pt::from_f32(0.01)), thickness * 2.0],
                Pt::ZERO,
            );
            canvas.set_line_width(thickness);
            stroke_text_decoration_segment(canvas, x, y, width);
            canvas.set_dash(Vec::new(), Pt::ZERO);
            canvas.set_line_cap(0);
        }
        crate::style::TextDecorationStyleMode::Dashed => {
            canvas.set_line_cap(0);
            canvas.set_dash(vec![thickness * 3.0, thickness * 2.0], Pt::ZERO);
            canvas.set_line_width(thickness);
            stroke_text_decoration_segment(canvas, x, y, width);
            canvas.set_dash(Vec::new(), Pt::ZERO);
        }
        crate::style::TextDecorationStyleMode::Wavy => {
            draw_wavy_text_decoration_line(canvas, x, y, width, thickness);
        }
    }
}

fn explicit_text_decoration_thickness(style: &TextStyle) -> Option<Pt> {
    match style.text_decoration_thickness {
        crate::style::TextDecorationThicknessMode::Auto
        | crate::style::TextDecorationThicknessMode::FromFont => None,
        crate::style::TextDecorationThicknessMode::Length(spec) => {
            let value = match spec {
                LengthSpec::Absolute(value) => value.max(Pt::ZERO),
                LengthSpec::Percent(pct) => (style.font_size * pct).max(Pt::from_f32(0.5)),
                LengthSpec::Em(scale) => (style.font_size * scale).max(Pt::ZERO),
                LengthSpec::Rem(scale) => (style.root_font_size * scale).max(Pt::ZERO),
                LengthSpec::Calc(calc) => calc
                    .resolve(style.font_size, style.font_size, style.root_font_size)
                    .max(Pt::ZERO),
                LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial => return None,
            };
            Some(value)
        }
    }
}

fn explicit_text_underline_offset(style: &TextStyle) -> Option<Pt> {
    match style.text_underline_offset {
        crate::style::TextUnderlineOffsetMode::Auto => None,
        crate::style::TextUnderlineOffsetMode::Length(spec) => {
            let value = match spec {
                LengthSpec::Absolute(value) => value,
                LengthSpec::Percent(pct) => style.font_size * pct,
                LengthSpec::Em(scale) => style.font_size * scale,
                LengthSpec::Rem(scale) => style.root_font_size * scale,
                LengthSpec::Calc(calc) => {
                    calc.resolve(style.font_size, style.font_size, style.root_font_size)
                }
                LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial => return None,
            };
            Some(value)
        }
    }
}

fn resolve_font_stack(
    registry: Option<&FontRegistry>,
    style: &TextStyle,
) -> (Arc<str>, Vec<Arc<str>>) {
    let primary = resolve_font_variant_name(
        registry,
        &style.font_name,
        style.font_weight,
        style.font_style,
    );
    let fallbacks: Vec<Arc<str>> = style
        .font_fallbacks
        .iter()
        .map(|name| resolve_font_variant_name(registry, name, style.font_weight, style.font_style))
        .collect();
    if let Some(registry) = registry {
        let mut resolved: Vec<Arc<str>> = Vec::new();
        for name in std::iter::once(primary.clone()).chain(fallbacks.iter().cloned()) {
            if registry.resolve(&name).is_some() || is_base14_name(&name) {
                resolved.push(name);
            }
        }
        if let Some(first) = resolved.first() {
            return (first.clone(), resolved.into_iter().skip(1).collect());
        }
        return (Arc::<str>::from("Helvetica"), Vec::new());
    }
    (primary, fallbacks)
}

fn draw_registered_text_run(
    canvas: &mut Canvas,
    registry: &FontRegistry,
    style: &TextStyle,
    font_name: &str,
    x: Pt,
    y: Pt,
    text: String,
) {
    if registry.requires_synthetic_bold(font_name, style.font_weight) {
        // A compact outline stroke approximates synthetic emboldening. PDF
        // fill+stroke preserves a single extractable text run and its authored
        // advances while expanding the glyph outline in both axes.
        let strength = (style.font_size * (1.0 / 32.0)).max(Pt::from_f32(0.25));
        canvas.draw_string_synthetic_bold(x, y, text, strength);
    } else {
        canvas.draw_string(x, y, text);
    }
}

fn is_base14_name(name: &str) -> bool {
    let n = name
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_lowercase();
    matches!(
        n.as_str(),
        "courier"
            | "courier-bold"
            | "courier-oblique"
            | "courier-boldoblique"
            | "helvetica"
            | "helvetica-bold"
            | "helvetica-oblique"
            | "helvetica-boldoblique"
            | "times-roman"
            | "times-bold"
            | "times-italic"
            | "times-bolditalic"
            | "symbol"
            | "zapfdingbats"
    )
}

fn emit_font_resolution_meta(
    canvas: &mut Canvas,
    registry: &FontRegistry,
    style: &TextStyle,
    resolved_name: &str,
    primary_resolved: &str,
) {
    let requested_name = style.font_name.as_ref();
    if requested_name == resolved_name && resolved_name == primary_resolved {
        return;
    }
    canvas.meta("font.requested_name", requested_name.to_string());
    let reason = if requested_name != primary_resolved
        && registry.resolve(requested_name).is_none()
        && !is_base14_name(requested_name)
    {
        if resolved_name != primary_resolved {
            "unregistered_primary_glyph_fallback"
        } else {
            "unregistered_primary_fallback"
        }
    } else if resolved_name != primary_resolved {
        "glyph_fallback"
    } else {
        "variant_resolution"
    };
    canvas.meta("font.fallback_reason", reason);
}

#[derive(Debug, Clone)]
pub struct Paragraph {
    text: String,
    style: TextStyle,
    align: TextAlign,
    align_last: Option<TextAlign>,
    pagination: Pagination,
    preserve_whitespace: bool,
    no_wrap: bool,
    suppress_first_line_indent: bool,
    tag_role: Option<Arc<str>>,
    font_registry: Option<Arc<FontRegistry>>,
    layout_cache: Arc<Mutex<TextLayoutCache>>,
    width_cache: Arc<Mutex<TextWidthCache>>,
}

impl Paragraph {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: TextStyle::default(),
            align: TextAlign::Left,
            align_last: None,
            pagination: Pagination::default(),
            preserve_whitespace: false,
            no_wrap: false,
            suppress_first_line_indent: false,
            tag_role: None,
            font_registry: None,
            layout_cache: Arc::new(Mutex::new(TextLayoutCache::default())),
            width_cache: Arc::new(Mutex::new(TextWidthCache::default())),
        }
    }

    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    pub fn with_align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    pub fn with_last_align(mut self, align: Option<TextAlign>) -> Self {
        self.align_last = align;
        self
    }

    pub fn with_tag_role(mut self, role: impl Into<Arc<str>>) -> Self {
        self.tag_role = Some(role.into());
        self
    }

    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = pagination;
        self
    }

    pub fn with_whitespace(mut self, preserve_whitespace: bool, no_wrap: bool) -> Self {
        self.preserve_whitespace = preserve_whitespace;
        self.no_wrap = no_wrap;
        self
    }

    pub(crate) fn with_font_registry(mut self, registry: Option<Arc<FontRegistry>>) -> Self {
        self.font_registry = registry;
        self
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn style(&self) -> &TextStyle {
        &self.style
    }

    fn measure_text_width(&self, text: &str) -> Pt {
        if let Some(stripped) = strip_soft_hyphens(text) {
            return self.measure_text_width(&stripped);
        }
        if let Ok(cache) = self.width_cache.lock() {
            if let Some(value) = cache.get(text) {
                if perf_enabled() {
                    log_perf_counts("layout.text.width", &[("cache_hit", 1)]);
                }
                return value;
            }
        }
        if let Some(value) =
            tabbed_text_width(&self.style, text, |part| self.measure_text_width(part))
        {
            if let Ok(mut cache) = self.width_cache.lock() {
                cache.insert(text, value);
            }
            if perf_enabled() {
                log_perf_counts("layout.text.width", &[("cache_miss", 1)]);
            }
            return value;
        }
        if let Some(registry) = &self.font_registry {
            let (primary, fallbacks) = resolve_font_stack(Some(registry), &self.style);
            let base = if fallbacks.is_empty() {
                registry.measure_text_width(&primary, self.style.font_size, text)
            } else {
                registry.measure_text_width_with_fallbacks(
                    &primary,
                    &fallbacks,
                    self.style.font_size,
                    text,
                )
            };
            let value = text_width_with_spacing(base, &self.style, text);
            if let Ok(mut cache) = self.width_cache.lock() {
                cache.insert(text, value);
            }
            if perf_enabled() {
                log_perf_counts("layout.text.width", &[("cache_miss", 1)]);
            }
            value
        } else {
            let char_width = (self.style.font_size * 0.6).max(Pt::from_f32(1.0));
            let count = text.chars().count();
            let base = char_width * (count as i32);
            let value = text_width_with_spacing(base, &self.style, text);
            if let Ok(mut cache) = self.width_cache.lock() {
                cache.insert(text, value);
            }
            if perf_enabled() {
                log_perf_counts("layout.text.width", &[("cache_miss", 1)]);
            }
            value
        }
    }

    fn effective_line_height(&self) -> Pt {
        if self.style.line_height_is_auto {
            if let Some(registry) = &self.font_registry {
                return registry.line_height(
                    &self.style.font_name,
                    self.style.font_size,
                    self.style.line_height,
                );
            }
            return self.style.font_size.mul_ratio(6, 5);
        }
        self.style.line_height
    }

    fn is_vertical_text(&self) -> bool {
        !matches!(
            self.style.writing_mode,
            crate::style::WritingModeMode::HorizontalTb
        )
    }

    fn resolved_text_indent(&self, avail_width: Pt) -> Pt {
        self.style.text_indent.resolve_width(
            avail_width,
            self.style.font_size,
            self.style.root_font_size,
        )
    }

    fn line_receives_text_indent(&self, line_idx: usize, forced_start: bool) -> bool {
        let first_line = line_idx == 0 && !self.suppress_first_line_indent;
        let affected = first_line || (self.style.text_indent_each_line && forced_start);
        if self.style.text_indent_hanging {
            !affected
        } else {
            affected
        }
    }

    fn line_text_indent(&self, line_idx: usize, forced_start: bool, indent: Pt) -> Pt {
        if self.line_receives_text_indent(line_idx, forced_start) {
            indent
        } else {
            Pt::ZERO
        }
    }

    fn line_limit(&self, max_width: Pt, indent: Pt) -> Pt {
        (max_width - indent).max(Pt::from_f32(1.0))
    }

    fn vertical_columns(&self, avail_height: Pt) -> Vec<String> {
        let glyph_advance = self.effective_line_height().max(Pt::from_f32(1.0));
        let max_units = if avail_height >= huge_pt() || avail_height <= Pt::ZERO {
            usize::MAX
        } else {
            (avail_height.to_f32() / glyph_advance.to_f32())
                .floor()
                .max(1.0) as usize
        };

        let mut columns: Vec<String> = Vec::new();
        let mut current = String::new();
        let mut current_count = 0usize;
        for segment in self.text.split('\n') {
            for ch in segment.chars() {
                if current_count >= max_units {
                    columns.push(current);
                    current = String::new();
                    current_count = 0;
                }
                current.push(ch);
                current_count += 1;
            }
            columns.push(std::mem::take(&mut current));
            current_count = 0;
        }
        if !current.is_empty() {
            columns.push(current);
        }
        if columns.is_empty() {
            columns.push(String::new());
        }
        columns
    }

    fn vertical_size(&self, avail_height: Pt) -> Size {
        let columns = self.vertical_columns(avail_height);
        let advance = self.effective_line_height().max(Pt::from_f32(1.0));
        let max_count = columns
            .iter()
            .map(|column| column.chars().count())
            .max()
            .unwrap_or(0);
        Size {
            width: advance * (columns.len() as i32),
            height: advance * (max_count as i32),
        }
    }

    fn draw_text_with_fallbacks(&self, canvas: &mut Canvas, x: Pt, y: Pt, text: &str) {
        if let Some(stripped) = strip_soft_hyphens(text) {
            self.draw_text_with_fallbacks(canvas, x, y, &stripped);
            return;
        }
        if let Some(expanded) = expanded_integer_tabs(&self.style, text) {
            self.draw_text_with_fallbacks(canvas, x, y, &expanded);
            return;
        }
        if text.contains('\t') {
            let space_width = self.measure_text_width(" ");
            let tab_advance = resolve_tab_advance(&self.style, space_width);
            let mut cursor_x = x;
            for (idx, part) in text.split('\t').enumerate() {
                if idx > 0 {
                    cursor_x = cursor_x + tab_advance;
                }
                if !part.is_empty() {
                    self.draw_text_with_fallbacks(canvas, cursor_x, y, part);
                    cursor_x = cursor_x + self.measure_text_width(part);
                }
            }
            return;
        }
        if let Some(registry) = &self.font_registry {
            let (primary, fallbacks) = resolve_font_stack(Some(registry), &self.style);
            let runs = registry.split_text_by_fallbacks(&primary, &fallbacks, text);
            let mut cursor_x = x;
            let mut remaining = text.chars().count();
            for run in runs {
                emit_font_resolution_meta(
                    canvas,
                    registry,
                    &self.style,
                    &run.font_name,
                    primary.as_ref(),
                );
                canvas.set_font_name(&run.font_name);
                if !text_style_has_spacing(&self.style) {
                    let run_text = run.text;
                    let run_len = run_text.chars().count();
                    let w = registry.measure_text_width(
                        &run.font_name,
                        self.style.font_size,
                        &run_text,
                    );
                    draw_registered_text_run(
                        canvas,
                        registry,
                        &self.style,
                        &run.font_name,
                        cursor_x,
                        y,
                        run_text,
                    );
                    cursor_x = cursor_x + w;
                    remaining = remaining.saturating_sub(run_len);
                } else {
                    if registry.resolve(&run.font_name).is_none() {
                        // Without real glyph metrics, per-character placement can visibly over-space
                        // narrow glyphs (for example, "I") when letter-spacing is enabled.
                        // Fall back to whole-run draw to avoid pathological spacing artifacts.
                        let run_text = run.text;
                        let run_len = run_text.chars().count();
                        let w = registry.measure_text_width(
                            &run.font_name,
                            self.style.font_size,
                            &run_text,
                        );
                        draw_registered_text_run(
                            canvas,
                            registry,
                            &self.style,
                            &run.font_name,
                            cursor_x,
                            y,
                            run_text,
                        );
                        cursor_x = cursor_x + w;
                        remaining = remaining.saturating_sub(run_len);
                        continue;
                    }
                    for ch in run.text.chars() {
                        let ch_str = ch.to_string();
                        draw_registered_text_run(
                            canvas,
                            registry,
                            &self.style,
                            &run.font_name,
                            cursor_x,
                            y,
                            ch_str.clone(),
                        );
                        let w = registry.measure_text_width(
                            &run.font_name,
                            self.style.font_size,
                            &ch_str,
                        );
                        remaining = remaining.saturating_sub(1);
                        cursor_x =
                            cursor_x + w + text_spacing_after_char(&self.style, ch, remaining);
                    }
                }
            }
            return;
        }

        let font_name = resolve_font_variant_name(
            None,
            &self.style.font_name,
            self.style.font_weight,
            self.style.font_style,
        );
        canvas.set_font_name(font_name.as_ref());
        if !text_style_has_spacing(&self.style) {
            canvas.draw_string(x, y, text);
        } else {
            // No registry means no reliable glyph advances. Prefer stable whole-run rendering over
            // synthetic per-character placement that can produce severe spacing artifacts.
            canvas.draw_string(x, y, text);
        }
    }

    fn text_shadow_length_to_pt(&self, spec: LengthSpec) -> Pt {
        match spec {
            LengthSpec::Absolute(value) => value,
            LengthSpec::Em(scale) => self.style.font_size * scale,
            LengthSpec::Rem(scale) => self.style.root_font_size * scale,
            LengthSpec::Calc(calc) => calc.resolve(
                self.style.font_size,
                self.style.font_size,
                self.style.root_font_size,
            ),
            LengthSpec::Auto
            | LengthSpec::Percent(_)
            | LengthSpec::Inherit
            | LengthSpec::Initial => Pt::ZERO,
        }
    }

    fn draw_text_shadow_sample(
        &self,
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        text: &str,
        width: Pt,
        color: Color,
        opacity: f32,
    ) {
        if opacity <= 0.001 {
            return;
        }
        canvas.save_state();
        canvas.set_fill_color(color);
        canvas.set_opacity(opacity, opacity);
        self.draw_text_with_fallbacks(canvas, x, y, text);
        if !self.style.text_decoration.is_none() {
            let mut shadow_style = self.style.clone();
            shadow_style.text_decoration_color = color;
            draw_text_decorations(
                canvas,
                &shadow_style,
                self.font_registry.as_deref(),
                x,
                y,
                width,
            );
        }
        canvas.restore_state();
    }

    fn draw_text_shadow(
        &self,
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        text: &str,
        width: Pt,
        shadow: &BoxShadowSpec,
    ) {
        if shadow.opacity <= 0.001 {
            return;
        }
        let base_x = x + self.text_shadow_length_to_pt(shadow.offset_x);
        let base_y = y + self.text_shadow_length_to_pt(shadow.offset_y);
        let blur = self.text_shadow_length_to_pt(shadow.blur).max(Pt::ZERO);
        let color = shadow.color;
        let opacity = shadow.opacity.clamp(0.0, 1.0);
        if blur <= Pt::from_f32(0.01) {
            self.draw_text_shadow_sample(canvas, base_x, base_y, text, width, color, opacity);
            return;
        }

        let spread = (blur * 0.45).max(Pt::from_f32(0.5));
        let samples = [
            (Pt::ZERO, Pt::ZERO, 0.36_f32),
            (spread, Pt::ZERO, 0.10_f32),
            (-spread, Pt::ZERO, 0.10_f32),
            (Pt::ZERO, spread, 0.10_f32),
            (Pt::ZERO, -spread, 0.10_f32),
            (spread, spread, 0.06_f32),
            (-spread, spread, 0.06_f32),
            (spread, -spread, 0.06_f32),
            (-spread, -spread, 0.06_f32),
        ];
        for (dx, dy, weight) in samples {
            self.draw_text_shadow_sample(
                canvas,
                base_x + dx,
                base_y + dy,
                text,
                width,
                color,
                opacity * weight,
            );
        }
    }

    fn draw_text_shadows_for_line(&self, canvas: &mut Canvas, x: Pt, y: Pt, text: &str, width: Pt) {
        if self.style.text_shadows.is_empty() || text.is_empty() {
            return;
        }
        for shadow in self.style.text_shadows.iter().rev() {
            self.draw_text_shadow(canvas, x, y, text, width, shadow);
        }
    }

    fn draw_vertical_text(
        &self,
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        avail_width: Pt,
        avail_height: Pt,
    ) {
        let columns = self.vertical_columns(avail_height);
        let advance = self.effective_line_height().max(Pt::from_f32(1.0));
        let rtl_block = matches!(
            self.style.writing_mode,
            crate::style::WritingModeMode::VerticalRl | crate::style::WritingModeMode::SidewaysRl
        );
        for (column_idx, column) in columns.iter().enumerate() {
            let column_offset = advance * (column_idx as i32);
            let column_x = if rtl_block {
                x + (avail_width - advance - column_offset).max(Pt::ZERO)
            } else {
                x + column_offset
            };
            let mut cursor_y = y;
            for ch in column.chars() {
                let glyph = ch.to_string();
                let glyph_width = self.measure_text_width(&glyph);
                self.draw_text_shadows_for_line(canvas, column_x, cursor_y, &glyph, glyph_width);
                self.draw_text_with_fallbacks(canvas, column_x, cursor_y, &glyph);
                cursor_y = cursor_y + advance;
            }
        }
    }

    fn layout_lines(&self, avail_width: Pt) -> Arc<Vec<LineLayout>> {
        let perf = perf_start();
        let max_width = avail_width.max(Pt::from_f32(1.0));
        let indent_value = self.resolved_text_indent(max_width);
        let key = max_width.to_milli_i64();
        if let Ok(cache) = self.layout_cache.lock() {
            if let Some(lines) = cache.get(key) {
                if perf_enabled() {
                    log_perf_counts(
                        "layout.text.counts",
                        &[
                            ("bytes", self.text.len() as u64),
                            ("lines", lines.len() as u64),
                            ("cache_hit", 1),
                        ],
                    );
                }
                perf_end("layout.text.lines", perf);
                return lines;
            }
        }
        if self.no_wrap {
            let mut line_layouts = Vec::new();
            for (idx, line) in self.text.split('\n').enumerate() {
                let forced_start = idx > 0;
                let line_indent = self.line_text_indent(idx, forced_start, indent_value);
                let line_limit = self.line_limit(max_width, line_indent);
                let resolved = if line.is_empty() {
                    String::new()
                } else if matches!(
                    self.style.text_overflow,
                    crate::style::TextOverflowMode::Ellipsis
                ) {
                    truncate_text_with_ellipsis(self, line, line_limit)
                } else {
                    line.to_string()
                };
                let text_width = if resolved.is_empty() {
                    Pt::ZERO
                } else {
                    self.measure_text_width(&resolved)
                };
                let width = line_width_with_indent(text_width, line_indent);
                line_layouts.push(LineLayout {
                    text: resolved,
                    width,
                    text_width,
                    indent: line_indent,
                    forced_start,
                });
            }
            let lines = Arc::new(line_layouts);
            if let Ok(mut cache) = self.layout_cache.lock() {
                cache.insert(key, lines.clone());
            }
            if perf_enabled() {
                log_perf_counts(
                    "layout.text.counts",
                    &[
                        ("bytes", self.text.len() as u64),
                        ("lines", lines.len() as u64),
                        ("cache_miss", 1),
                    ],
                );
            }
            perf_end("layout.text.lines", perf);
            return lines;
        }

        let allow_break_long =
            matches!(
                self.style.word_break,
                crate::style::WordBreakMode::BreakWord
                    | crate::style::WordBreakMode::BreakAll
                    | crate::style::WordBreakMode::Anywhere
            ) || matches!(self.style.line_break, crate::style::LineBreakMode::Anywhere);

        let mut lines: Vec<PendingLineLayout> = Vec::new();
        let mut word_widths: HashMap<&str, Pt> = HashMap::new();
        if self.preserve_whitespace {
            for (segment_idx, segment) in self.text.split('\n').enumerate() {
                push_preserved_wrapped_segment(
                    self,
                    segment,
                    segment_idx > 0,
                    max_width,
                    indent_value,
                    allow_break_long,
                    &mut lines,
                );
            }
        } else {
            let space_width = self.measure_text_width(" ");
            for (segment_idx, segment) in self.text.split('\n').enumerate() {
                let mut current_forced_start = segment_idx > 0;
                if segment.is_empty() {
                    lines.push(PendingLineLayout {
                        text: String::new(),
                        forced_start: current_forced_start,
                    });
                    continue;
                }
                let mut current = String::new();
                let mut current_width = Pt::ZERO;
                let words: Vec<(&str, Pt)> = segment
                    .split_whitespace()
                    .map(|word| {
                        let width = if let Some(value) = word_widths.get(word) {
                            *value
                        } else {
                            let value = self.measure_text_width(word);
                            word_widths.insert(word, value);
                            value
                        };
                        (word, width)
                    })
                    .collect();
                for (word, word_width) in words {
                    let current_indent =
                        self.line_text_indent(lines.len(), current_forced_start, indent_value);
                    let current_limit = self.line_limit(max_width, current_indent);
                    if current.is_empty() {
                        if word_width > current_limit {
                            if let Some(parts) =
                                split_word_by_soft_hyphen(self, word, current_limit)
                            {
                                for part in parts {
                                    lines.push(PendingLineLayout {
                                        text: part,
                                        forced_start: current_forced_start,
                                    });
                                    current_forced_start = false;
                                }
                                current.clear();
                            } else if allow_break_long {
                                for part in split_long_word_by_width(self, word, current_limit) {
                                    lines.push(PendingLineLayout {
                                        text: part,
                                        forced_start: current_forced_start,
                                    });
                                    current_forced_start = false;
                                }
                                current.clear();
                            } else {
                                lines.push(PendingLineLayout {
                                    text: word.to_string(),
                                    forced_start: current_forced_start,
                                });
                                current_forced_start = false;
                                current.clear();
                            }
                        } else {
                            current.push_str(word);
                            current_width = word_width;
                        }
                    } else {
                        let next_width = current_width + space_width + word_width;
                        if next_width <= current_limit {
                            current.push(' ');
                            current.push_str(word);
                            current_width = next_width;
                        } else {
                            lines.push(PendingLineLayout {
                                text: current,
                                forced_start: current_forced_start,
                            });
                            current = String::new();
                            current_forced_start = false;
                            let follow_indent =
                                self.line_text_indent(lines.len(), false, indent_value);
                            let follow_limit = self.line_limit(max_width, follow_indent);
                            if word_width > follow_limit {
                                if let Some(parts) =
                                    split_word_by_soft_hyphen(self, word, follow_limit)
                                {
                                    for part in parts {
                                        lines.push(PendingLineLayout {
                                            text: part,
                                            forced_start: false,
                                        });
                                    }
                                } else if allow_break_long {
                                    for part in split_long_word_by_width(self, word, follow_limit) {
                                        lines.push(PendingLineLayout {
                                            text: part,
                                            forced_start: false,
                                        });
                                    }
                                } else {
                                    lines.push(PendingLineLayout {
                                        text: word.to_string(),
                                        forced_start: false,
                                    });
                                }
                            } else {
                                current.push_str(word);
                                current_width = word_width;
                            }
                        }
                    }
                }
                if !current.is_empty() {
                    lines.push(PendingLineLayout {
                        text: current,
                        forced_start: current_forced_start,
                    });
                }
            }
        }

        if lines.is_empty() {
            lines.push(PendingLineLayout {
                text: String::new(),
                forced_start: false,
            });
        }

        let mut line_layouts = Vec::with_capacity(lines.len());
        for (idx, line) in lines.into_iter().enumerate() {
            let text_width = if line.text.is_empty() {
                Pt::ZERO
            } else {
                self.measure_text_width(&line.text)
            };
            let indent = self.line_text_indent(idx, line.forced_start, indent_value);
            let width = line_width_with_indent(text_width, indent);
            line_layouts.push(LineLayout {
                text: line.text,
                width,
                text_width,
                indent,
                forced_start: line.forced_start,
            });
        }
        let lines = Arc::new(line_layouts);
        if let Ok(mut cache) = self.layout_cache.lock() {
            cache.insert(key, lines.clone());
        }
        if perf_enabled() {
            log_perf_counts(
                "layout.text.counts",
                &[
                    ("bytes", self.text.len() as u64),
                    ("lines", lines.len() as u64),
                    ("cache_miss", 1),
                ],
            );
        }
        perf_end("layout.text.lines", perf);
        lines
    }
}

fn truncate_text_with_ellipsis(paragraph: &Paragraph, text: &str, max_width: Pt) -> String {
    if text.is_empty() {
        return String::new();
    }
    if paragraph.measure_text_width(text) <= max_width {
        return text.to_string();
    }

    let ellipsis = "\u{2026}";
    if max_width <= Pt::ZERO {
        return String::new();
    }
    let ellipsis_width = paragraph.measure_text_width(ellipsis);
    if ellipsis_width >= max_width {
        return ellipsis.to_string();
    }

    let mut boundaries: Vec<usize> = text.char_indices().map(|(idx, _)| idx).collect();
    boundaries.push(text.len());
    if boundaries.len() <= 1 {
        return ellipsis.to_string();
    }

    let mut lo = 0usize;
    let mut hi = boundaries.len() - 1;
    let mut best = 0usize;
    while lo <= hi {
        let mid = (lo + hi) / 2;
        let end = boundaries[mid];
        let candidate = &text[..end];
        let mut candidate_text = String::with_capacity(end + ellipsis.len());
        candidate_text.push_str(candidate);
        candidate_text.push_str(ellipsis);
        let width = paragraph.measure_text_width(&candidate_text);
        if width <= max_width {
            best = mid;
            lo = mid + 1;
        } else {
            if mid == 0 {
                break;
            }
            hi = mid - 1;
        }
    }

    let end = boundaries[best];
    let mut out = String::new();
    out.push_str(&text[..end]);
    out.push_str(ellipsis);
    out
}

fn split_long_word_by_width(paragraph: &Paragraph, word: &str, max_width: Pt) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut current_width = Pt::ZERO;
    let mut ascii_widths: [Option<Pt>; 128] = std::array::from_fn(|_| None);
    let mut non_ascii_widths: HashMap<char, Pt> = HashMap::new();
    for ch in word.chars() {
        let w = if (ch as u32) < 128 {
            let idx = ch as usize;
            if let Some(value) = ascii_widths[idx] {
                value
            } else {
                let value = paragraph.measure_text_width(&ch.to_string());
                ascii_widths[idx] = Some(value);
                value
            }
        } else if let Some(value) = non_ascii_widths.get(&ch) {
            *value
        } else {
            let value = paragraph.measure_text_width(&ch.to_string());
            non_ascii_widths.insert(ch, value);
            value
        };
        let mut next_width = current_width + w;
        if !current.is_empty() && next_width > max_width {
            parts.push(current);
            current = String::new();
            next_width = w;
        }
        current.push(ch);
        current_width = next_width;
    }
    if !current.is_empty() {
        parts.push(current);
    }
    if parts.is_empty() {
        parts.push(String::new());
    }
    parts
}

fn last_preserved_whitespace_break(paragraph: &Paragraph, text: &str) -> Option<(usize, Pt)> {
    let mut width = Pt::ZERO;
    let mut last = None;
    for (idx, ch) in text.char_indices() {
        width = width + paragraph.measure_text_width(&ch.to_string());
        if ch.is_whitespace() {
            last = Some((idx + ch.len_utf8(), width));
        }
    }
    last
}

fn push_preserved_wrapped_segment(
    paragraph: &Paragraph,
    segment: &str,
    forced_start: bool,
    max_width: Pt,
    indent_value: Pt,
    allow_break_long: bool,
    lines: &mut Vec<PendingLineLayout>,
) {
    if segment.is_empty() {
        lines.push(PendingLineLayout {
            text: String::new(),
            forced_start,
        });
        return;
    }

    let mut current = String::new();
    let mut current_width = Pt::ZERO;
    let mut current_forced_start = forced_start;
    let mut last_break: Option<(usize, Pt)> = None;
    for ch in segment.chars() {
        let ch_width = paragraph.measure_text_width(&ch.to_string());
        let indent = paragraph.line_text_indent(lines.len(), current_forced_start, indent_value);
        let limit = paragraph.line_limit(max_width, indent);
        if !current.is_empty() && current_width + ch_width > limit {
            if let Some((break_idx, break_width)) = last_break {
                if break_idx == current.len() {
                    lines.push(PendingLineLayout {
                        text: current,
                        forced_start: current_forced_start,
                    });
                    current = String::new();
                    current_width = Pt::ZERO;
                    last_break = None;
                } else {
                    let line = current[..break_idx].to_string();
                    let remainder = current[break_idx..].to_string();
                    let remainder_width = (current_width - break_width).max(Pt::ZERO);
                    lines.push(PendingLineLayout {
                        text: line,
                        forced_start: current_forced_start,
                    });
                    current = remainder;
                    current_width = remainder_width;
                    last_break = last_preserved_whitespace_break(paragraph, &current);
                }
                current_forced_start = false;
            } else if allow_break_long {
                lines.push(PendingLineLayout {
                    text: current,
                    forced_start: current_forced_start,
                });
                current = String::new();
                current_width = Pt::ZERO;
                current_forced_start = false;
                last_break = None;
            }
        }
        current.push(ch);
        current_width = current_width + ch_width;
        if ch.is_whitespace() {
            last_break = Some((current.len(), current_width));
        }
    }
    if !current.is_empty() {
        lines.push(PendingLineLayout {
            text: current,
            forced_start: current_forced_start,
        });
    }
}

fn split_word_by_soft_hyphen(
    paragraph: &Paragraph,
    word: &str,
    max_width: Pt,
) -> Option<Vec<String>> {
    if matches!(paragraph.style.hyphens, crate::style::HyphensMode::None)
        || !word.contains(SOFT_HYPHEN)
    {
        return None;
    }
    let segments: Vec<&str> = word.split(SOFT_HYPHEN).collect();
    if segments.len() < 2 {
        return None;
    }

    let mut parts = Vec::new();
    let mut index = 0usize;
    let marker = hyphenate_character(&paragraph.style);
    while index < segments.len() {
        let mut candidate = String::new();
        let mut best: Option<(usize, String)> = None;
        for next in index..segments.len() {
            candidate.push_str(segments[next]);
            let is_final = next + 1 == segments.len();
            let rendered = if is_final {
                candidate.clone()
            } else {
                format!("{candidate}{marker}")
            };
            if paragraph.measure_text_width(&rendered) <= max_width {
                best = Some((next + 1, rendered));
            } else if next == index {
                if is_final && !parts.is_empty() {
                    best = Some((next + 1, rendered));
                } else {
                    return None;
                }
            } else {
                break;
            }
        }
        let (next_index, rendered) = best?;
        parts.push(rendered);
        index = next_index;
    }

    (!parts.is_empty()).then_some(parts)
}

impl Flowable for Paragraph {
    fn wrap(&self, avail_width: Pt, avail_height: Pt) -> Size {
        let perf = perf_start();
        if self.is_vertical_text() {
            let size = self.vertical_size(avail_height);
            perf_end("layout.text.wrap", perf);
            return Size {
                width: size.width.min(avail_width),
                height: size.height,
            };
        }
        let lines = self.layout_lines(avail_width);
        let line_height = self.effective_line_height();
        let height = line_height * (lines.len() as i32);
        let width = lines
            .iter()
            .fold(Pt::ZERO, |acc, line| acc.max(line.width))
            .min(avail_width);
        perf_end("layout.text.wrap", perf);
        Size { width, height }
    }

    fn intrinsic_width(&self) -> Option<Pt> {
        if self.is_vertical_text() {
            return Some(self.effective_line_height().max(Pt::from_f32(1.0)));
        }
        let mut max_w = Pt::ZERO;
        for line in self.text.split('\n') {
            max_w = max_w.max(self.measure_text_width(line));
        }
        Some(max_w)
    }

    fn first_baseline(&self, _avail_width: Pt) -> Option<Pt> {
        if self.is_vertical_text() {
            return None;
        }
        let line_height = self.effective_line_height();
        Some(text_baseline_for_line(
            &self.style,
            self.font_registry.as_deref(),
            line_height,
        ))
    }

    fn split(
        &self,
        avail_width: Pt,
        avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        if self.is_vertical_text() {
            return None;
        }
        let lines = self.layout_lines(avail_width);
        let line_height = self.effective_line_height();
        let lh = line_height.to_milli_i64();
        let ah = avail_height.to_milli_i64();
        if lh <= 0 || ah <= 0 {
            return None;
        }
        let max_lines = (ah / lh) as usize;
        if max_lines == 0 || max_lines >= lines.len() {
            return None;
        }

        let mut split_at = max_lines;
        let total_lines = lines.len();
        let orphans = self.pagination.resolved_orphans();
        let widows = self.pagination.resolved_widows();

        if split_at < orphans {
            split_at = 0;
        }

        if total_lines - split_at < widows {
            let adjusted = total_lines.saturating_sub(widows);
            if adjusted >= orphans {
                split_at = adjusted;
            } else if max_lines >= orphans {
                split_at = max_lines.min(adjusted.max(orphans));
            } else {
                split_at = 0;
            }
        }

        if split_at == 0 || split_at >= total_lines {
            if max_lines >= 1 {
                split_at = max_lines.min(total_lines - 1);
            } else {
                return None;
            }
        }

        if total_lines - split_at < widows && split_at > 1 {
            split_at = (total_lines - widows).max(1);
        }

        if split_at == 0 || split_at >= total_lines {
            return None;
        }

        let first_text = lines[..split_at]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let second_text = lines[split_at..]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let first = Paragraph {
            text: first_text,
            style: self.style.clone(),
            align: self.align,
            align_last: self.align_last,
            pagination: Pagination {
                break_before: BreakBefore::Auto,
                break_after: BreakAfter::Auto,
                ..self.pagination
            },
            preserve_whitespace: self.preserve_whitespace,
            no_wrap: self.no_wrap,
            suppress_first_line_indent: self.suppress_first_line_indent,
            tag_role: self.tag_role.clone(),
            font_registry: self.font_registry.clone(),
            layout_cache: Arc::new(Mutex::new(TextLayoutCache::default())),
            width_cache: Arc::new(Mutex::new(TextWidthCache::default())),
        };
        let second = Paragraph {
            text: second_text,
            style: self.style.clone(),
            align: self.align,
            align_last: self.align_last,
            pagination: Pagination {
                break_before: BreakBefore::Auto,
                ..self.pagination
            },
            preserve_whitespace: self.preserve_whitespace,
            no_wrap: self.no_wrap,
            suppress_first_line_indent: true,
            tag_role: self.tag_role.clone(),
            font_registry: self.font_registry.clone(),
            layout_cache: Arc::new(Mutex::new(TextLayoutCache::default())),
            width_cache: Arc::new(Mutex::new(TextWidthCache::default())),
        };
        Some((Box::new(first), Box::new(second)))
    }

    fn draw(&self, canvas: &mut Canvas, x: Pt, y: Pt, avail_width: Pt, avail_height: Pt) {
        let perf = perf_start();
        if !self.style.visible {
            perf_end("layout.text.draw", perf);
            return;
        }
        let tagged = self.tag_role.as_ref().map(|role| {
            canvas.begin_tag(role.as_ref(), None, None, None, None, false);
        });
        canvas.set_fill_color(self.style.color);
        canvas.set_font_size(self.style.font_size);
        if self.is_vertical_text() {
            self.draw_vertical_text(canvas, x, y, avail_width, avail_height);
            if tagged.is_some() {
                canvas.end_tag();
            }
            perf_end("layout.text.draw", perf);
            return;
        }

        let lines = self.layout_lines(avail_width);
        let mut cursor_y = y;
        let line_height = self.effective_line_height();
        for (idx, line) in lines.iter().enumerate() {
            let line_width = line.width;
            let align = self.effective_text_align_for_line(idx, &lines);
            let draw_y = text_draw_y_for_line(
                &self.style,
                self.font_registry.as_deref(),
                cursor_y,
                line_height,
            );
            if matches!(align, TextAlign::Justify)
                && self.draw_justified_line(canvas, x, draw_y, avail_width, line)
            {
                cursor_y = cursor_y + line_height;
                continue;
            }
            let offset = text_align_offset(align, avail_width, line_width);
            let text_x = x + offset + line.indent;
            self.draw_text_shadows_for_line(canvas, text_x, draw_y, &line.text, line.text_width);
            self.draw_text_with_fallbacks(canvas, text_x, draw_y, &line.text);
            draw_text_decorations(
                canvas,
                &self.style,
                self.font_registry.as_deref(),
                text_x,
                draw_y,
                line.text_width,
            );
            cursor_y = cursor_y + line_height;
        }
        if tagged.is_some() {
            canvas.end_tag();
        }
        perf_end("layout.text.draw", perf);
    }

    fn pagination(&self) -> Pagination {
        self.pagination
    }
}

impl Paragraph {
    fn line_is_final_or_forced_break(&self, line_idx: usize, lines: &[LineLayout]) -> bool {
        line_idx + 1 == lines.len()
            || lines
                .get(line_idx + 1)
                .map(|line| line.forced_start)
                .unwrap_or(false)
    }

    fn line_receives_text_align_last(&self, line_idx: usize, lines: &[LineLayout]) -> bool {
        if self.align_last.is_none() {
            return false;
        }
        self.line_is_final_or_forced_break(line_idx, lines)
    }

    fn effective_text_align_for_line(&self, line_idx: usize, lines: &[LineLayout]) -> TextAlign {
        if self.line_receives_text_align_last(line_idx, lines) {
            return self.align_last.unwrap_or(self.align);
        }
        if matches!(self.align, TextAlign::Justify)
            && self.line_is_final_or_forced_break(line_idx, lines)
        {
            return TextAlign::Left;
        }
        self.align
    }

    fn draw_justified_line(
        &self,
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        avail_width: Pt,
        line: &LineLayout,
    ) -> bool {
        if matches!(self.style.text_justify, crate::style::TextJustifyMode::None) {
            return false;
        }
        if matches!(
            self.style.text_justify,
            crate::style::TextJustifyMode::InterCharacter
        ) {
            return self.draw_inter_character_justified_line(canvas, x, y, avail_width, line);
        }
        self.draw_inter_word_justified_line(canvas, x, y, avail_width, line)
    }

    fn draw_inter_word_justified_line(
        &self,
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        avail_width: Pt,
        line: &LineLayout,
    ) -> bool {
        if self.preserve_whitespace || line.text.is_empty() || line.text.contains('\t') {
            return false;
        }

        let words: Vec<&str> = line.text.split_whitespace().collect();
        let space_count = line.text.chars().filter(|ch| *ch == ' ').count();
        if words.len() < 2 || space_count != words.len() - 1 {
            return false;
        }

        let target_width = (avail_width - line.indent).max(Pt::ZERO);
        let extra = (target_width - line.text_width).max(Pt::ZERO);
        if extra <= Pt::from_f32(0.01) {
            return false;
        }

        let word_widths: Vec<Pt> = words
            .iter()
            .map(|word| self.measure_text_width(word))
            .collect();
        let word_width_total = word_widths
            .iter()
            .copied()
            .fold(Pt::ZERO, |acc, width| acc + width);
        let gap_advance =
            ((line.text_width - word_width_total).max(Pt::ZERO) + extra) / (space_count as i32);

        let mut cursor_x = x + line.indent;
        for (idx, word) in words.iter().enumerate() {
            let word_width = word_widths[idx];
            self.draw_text_shadows_for_line(canvas, cursor_x, y, word, word_width);
            self.draw_text_with_fallbacks(canvas, cursor_x, y, word);
            cursor_x = cursor_x + word_width;
            if idx + 1 < words.len() {
                cursor_x = cursor_x + gap_advance;
            }
        }
        draw_text_decorations(
            canvas,
            &self.style,
            self.font_registry.as_deref(),
            x + line.indent,
            y,
            target_width,
        );
        true
    }

    fn draw_inter_character_justified_line(
        &self,
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        avail_width: Pt,
        line: &LineLayout,
    ) -> bool {
        if self.preserve_whitespace || line.text.is_empty() || line.text.contains('\t') {
            return false;
        }
        let chars: Vec<char> = line.text.chars().collect();
        if chars.len() < 2 {
            return false;
        }

        let target_width = (avail_width - line.indent).max(Pt::ZERO);
        let extra = (target_width - line.text_width).max(Pt::ZERO);
        if extra <= Pt::from_f32(0.01) {
            return false;
        }
        let extra_advance = extra / ((chars.len() - 1) as i32);

        let mut cursor_x = x + line.indent;
        for (idx, ch) in chars.iter().enumerate() {
            let text = ch.to_string();
            let width = self.measure_text_width(&text);
            self.draw_text_shadows_for_line(canvas, cursor_x, y, &text, width);
            self.draw_text_with_fallbacks(canvas, cursor_x, y, &text);
            let remaining = chars.len().saturating_sub(idx + 1);
            cursor_x = cursor_x + width + text_spacing_after_char(&self.style, *ch, remaining);
            if remaining > 0 {
                cursor_x = cursor_x + extra_advance;
            }
        }
        draw_text_decorations(
            canvas,
            &self.style,
            self.font_registry.as_deref(),
            x + line.indent,
            y,
            target_width,
        );
        true
    }
}

#[derive(Clone)]
pub struct ListItemFlowable {
    label: Box<dyn Flowable>,
    body: Box<dyn Flowable>,
    gap: Pt,
    pagination: Pagination,
}

impl ListItemFlowable {
    pub fn new(label: Paragraph, body: Box<dyn Flowable>, gap: Pt) -> Self {
        Self::new_with_label(Box::new(label), body, gap)
    }

    pub fn new_with_label(label: Box<dyn Flowable>, body: Box<dyn Flowable>, gap: Pt) -> Self {
        Self {
            label,
            body,
            gap,
            pagination: Pagination::default(),
        }
    }

    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = pagination;
        self
    }
}

impl Flowable for ListItemFlowable {
    fn wrap(&self, avail_width: Pt, avail_height: Pt) -> Size {
        let label_size = self.label.wrap(avail_width, huge_pt());
        let body_width = (avail_width - label_size.width - self.gap).max(Pt::from_f32(1.0));
        let body_size = self.body.wrap(body_width, avail_height);
        Size {
            width: avail_width,
            height: label_size.height.max(body_size.height),
        }
    }

    fn split(
        &self,
        _avail_width: Pt,
        _avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        None
    }

    fn draw(&self, canvas: &mut Canvas, x: Pt, y: Pt, avail_width: Pt, avail_height: Pt) {
        let label_size = self.label.wrap(avail_width, huge_pt());
        let body_width = (avail_width - label_size.width - self.gap).max(Pt::from_f32(1.0));
        self.label
            .draw(canvas, x, y, label_size.width, avail_height);
        self.body.draw(
            canvas,
            x + label_size.width + self.gap,
            y,
            body_width,
            avail_height,
        );
    }

    fn pagination(&self) -> Pagination {
        self.pagination
    }
}

#[derive(Debug, Clone)]
pub struct Spacer {
    height: Pt,
    pagination: Pagination,
}

impl Spacer {
    pub fn new(height: f32) -> Self {
        Self::new_pt(Pt::from_f32(height))
    }

    pub fn new_pt(height: Pt) -> Self {
        Self {
            height,
            pagination: Pagination::default(),
        }
    }

    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = pagination;
        self
    }
}

impl Flowable for Spacer {
    fn wrap(&self, avail_width: Pt, _avail_height: Pt) -> Size {
        Size {
            width: avail_width,
            height: self.height.max(Pt::ZERO),
        }
    }

    fn intrinsic_width(&self) -> Option<Pt> {
        // A line break / spacer contributes vertical rhythm, not horizontal demand.
        Some(Pt::ZERO)
    }

    fn split(
        &self,
        _avail_width: Pt,
        _avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        None
    }

    fn draw(&self, _canvas: &mut Canvas, _x: Pt, _y: Pt, _avail_width: Pt, _avail_height: Pt) {}

    fn pagination(&self) -> Pagination {
        self.pagination
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CollapsibleSpaceFlowable {
    width: Pt,
    height: Pt,
    baseline: Pt,
}

impl CollapsibleSpaceFlowable {
    pub(crate) fn new(style: TextStyle, font_registry: Option<Arc<FontRegistry>>) -> Self {
        let probe = Paragraph::new(" ")
            .with_style(style)
            .with_font_registry(font_registry);
        let width = probe.measure_text_width(" ").max(Pt::ZERO);
        let height = probe.effective_line_height().max(Pt::ZERO);
        let baseline = probe.first_baseline(width).unwrap_or(height);
        Self {
            width,
            height,
            baseline,
        }
    }
}

impl Flowable for CollapsibleSpaceFlowable {
    fn wrap(&self, _avail_width: Pt, _avail_height: Pt) -> Size {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn intrinsic_width(&self) -> Option<Pt> {
        Some(self.width)
    }

    fn first_baseline(&self, _avail_width: Pt) -> Option<Pt> {
        Some(self.baseline)
    }

    fn split(
        &self,
        _avail_width: Pt,
        _avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        None
    }

    fn draw(&self, _canvas: &mut Canvas, _x: Pt, _y: Pt, _avail_width: Pt, _avail_height: Pt) {}
}

#[derive(Debug, Clone)]
pub struct BackgroundPaintFlowable {
    width: Pt,
    height: Pt,
    paint: BackgroundPaint,
    tag_role: Option<Arc<str>>,
    pagination: Pagination,
    visible: bool,
}

impl BackgroundPaintFlowable {
    pub fn new_pt(width: Pt, height: Pt, paint: BackgroundPaint) -> Self {
        Self {
            width,
            height,
            paint,
            tag_role: None,
            pagination: Pagination::default(),
            visible: true,
        }
    }

    pub fn with_tag_role(mut self, role: impl Into<Arc<str>>) -> Self {
        self.tag_role = Some(role.into());
        self
    }

    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = pagination;
        self
    }

    pub fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }
}

impl Flowable for BackgroundPaintFlowable {
    fn wrap(&self, _avail_width: Pt, _avail_height: Pt) -> Size {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn intrinsic_width(&self) -> Option<Pt> {
        Some(self.width)
    }

    fn split(
        &self,
        _avail_width: Pt,
        _avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        None
    }

    fn draw(&self, canvas: &mut Canvas, x: Pt, y: Pt, _avail_width: Pt, _avail_height: Pt) {
        if !self.visible {
            return;
        }
        let tagged = self.tag_role.as_ref().map(|role| {
            canvas.begin_tag(role.as_ref(), None, None, None, None, false);
        });
        ContainerFlowable::draw_background_paint(
            canvas,
            x,
            y,
            self.width,
            self.height,
            Pt::ZERO,
            &self.paint,
        );
        if tagged.is_some() {
            canvas.end_tag();
        }
    }

    fn pagination(&self) -> Pagination {
        self.pagination
    }
}

#[derive(Debug, Clone)]
pub struct ImageFlowable {
    pub width: Pt,
    pub height: Pt,
    pub resource_id: String,
    object_fit: ObjectFitMode,
    object_position: BackgroundPositionSpec,
    intrinsic_size: Option<Size>,
    font_size: Pt,
    root_font_size: Pt,
    tag_role: Option<Arc<str>>,
    alt: Option<String>,
    pagination: Pagination,
    visible: bool,
}

impl ImageFlowable {
    pub fn new(width: f32, height: f32, resource_id: impl Into<String>) -> Self {
        Self::new_pt(Pt::from_f32(width), Pt::from_f32(height), resource_id)
    }

    pub fn new_pt(width: Pt, height: Pt, resource_id: impl Into<String>) -> Self {
        Self {
            width,
            height,
            resource_id: resource_id.into(),
            object_fit: ObjectFitMode::Fill,
            object_position: BackgroundPositionSpec::center(),
            intrinsic_size: None,
            font_size: Pt::from_f32(12.0),
            root_font_size: Pt::from_f32(12.0),
            tag_role: None,
            alt: None,
            pagination: Pagination::default(),
            visible: true,
        }
    }

    pub fn with_tag_role(mut self, role: impl Into<Arc<str>>) -> Self {
        self.tag_role = Some(role.into());
        self
    }

    pub fn with_alt(mut self, alt: Option<String>) -> Self {
        self.alt = alt.filter(|v| !v.trim().is_empty());
        self
    }

    pub fn with_object_fit(mut self, object_fit: ObjectFitMode) -> Self {
        self.object_fit = object_fit;
        self
    }

    pub fn with_object_position(mut self, object_position: BackgroundPositionSpec) -> Self {
        self.object_position = object_position;
        self
    }

    pub fn with_intrinsic_size(mut self, intrinsic_size: Option<(Pt, Pt)>) -> Self {
        self.intrinsic_size = intrinsic_size.and_then(|(width, height)| {
            if width > Pt::ZERO && height > Pt::ZERO {
                Some(Size { width, height })
            } else {
                None
            }
        });
        self
    }

    pub fn with_font_metrics(mut self, font_size: Pt, root_font_size: Pt) -> Self {
        self.font_size = font_size;
        self.root_font_size = root_font_size;
        self
    }

    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = pagination;
        self
    }

    pub fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    fn resolve_object_position_component(
        &self,
        component: BackgroundPositionComponent,
        area: Pt,
        object: Pt,
        horizontal: bool,
    ) -> Pt {
        match component {
            BackgroundPositionComponent::Start(offset) => {
                self.resolve_object_position_offset(offset, area - object, horizontal)
            }
            BackgroundPositionComponent::Center => (area - object) / 2.0,
            BackgroundPositionComponent::End(offset) => {
                area - object
                    - self.resolve_object_position_offset(offset, area - object, horizontal)
            }
        }
    }

    fn resolve_object_position_offset(
        &self,
        offset: LengthSpec,
        percent_basis: Pt,
        horizontal: bool,
    ) -> Pt {
        if horizontal {
            offset.resolve_width(percent_basis, self.font_size, self.root_font_size)
        } else {
            offset.resolve_height(percent_basis, self.font_size, self.root_font_size)
        }
    }

    fn positioned_object_rect(&self, width: Pt, height: Pt, clip: bool) -> (Pt, Pt, Pt, Pt, bool) {
        let offset_x =
            self.resolve_object_position_component(self.object_position.x, self.width, width, true);
        let offset_y = self.resolve_object_position_component(
            self.object_position.y,
            self.height,
            height,
            false,
        );
        (offset_x, offset_y, width, height, clip)
    }

    fn object_fit_rect(&self) -> (Pt, Pt, Pt, Pt, bool) {
        let fill = (Pt::ZERO, Pt::ZERO, self.width, self.height, false);
        let Some(intrinsic_size) = self.intrinsic_size else {
            return fill;
        };
        if intrinsic_size.width <= Pt::ZERO
            || intrinsic_size.height <= Pt::ZERO
            || self.width <= Pt::ZERO
            || self.height <= Pt::ZERO
        {
            return fill;
        }

        let width_scale = self.width.to_f32() / intrinsic_size.width.to_f32();
        let height_scale = self.height.to_f32() / intrinsic_size.height.to_f32();
        if !width_scale.is_finite()
            || !height_scale.is_finite()
            || width_scale <= 0.0
            || height_scale <= 0.0
        {
            return fill;
        }

        let contain = |scale: f32| {
            let draw_width = Pt::from_f32(intrinsic_size.width.to_f32() * scale);
            let draw_height = Pt::from_f32(intrinsic_size.height.to_f32() * scale);
            (draw_width, draw_height)
        };
        let none = || (intrinsic_size.width, intrinsic_size.height);

        match self.object_fit {
            ObjectFitMode::Fill => fill,
            ObjectFitMode::Contain => {
                let (width, height) = contain(width_scale.min(height_scale));
                self.positioned_object_rect(width, height, false)
            }
            ObjectFitMode::Cover => {
                let (width, height) = contain(width_scale.max(height_scale));
                self.positioned_object_rect(width, height, true)
            }
            ObjectFitMode::None => {
                let (width, height) = none();
                self.positioned_object_rect(width, height, true)
            }
            ObjectFitMode::ScaleDown => {
                if intrinsic_size.width <= self.width && intrinsic_size.height <= self.height {
                    let (width, height) = none();
                    self.positioned_object_rect(width, height, true)
                } else {
                    let (width, height) = contain(width_scale.min(height_scale));
                    self.positioned_object_rect(width, height, false)
                }
            }
        }
    }
}

impl Flowable for ImageFlowable {
    fn wrap(&self, _avail_width: Pt, _avail_height: Pt) -> Size {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn intrinsic_width(&self) -> Option<Pt> {
        Some(self.width)
    }

    fn split(
        &self,
        _avail_width: Pt,
        _avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        None
    }

    fn draw(&self, canvas: &mut Canvas, x: Pt, y: Pt, _avail_width: Pt, _avail_height: Pt) {
        if !self.visible {
            return;
        }
        let tagged = self.tag_role.as_ref().map(|role| {
            canvas.begin_tag(role.as_ref(), self.alt.clone(), None, None, None, false);
        });
        let (offset_x, offset_y, width, height, clip) = self.object_fit_rect();
        if clip {
            canvas.save_state();
            canvas.clip_rect(x, y, self.width, self.height);
        }
        canvas.draw_image(
            x + offset_x,
            y + offset_y,
            width,
            height,
            self.resource_id.clone(),
        );
        if clip {
            canvas.restore_state();
        }
        if tagged.is_some() {
            canvas.end_tag();
        }
    }

    fn pagination(&self) -> Pagination {
        self.pagination
    }
}

#[derive(Debug, Clone)]
pub struct SvgFlowable {
    width: Pt,
    height: Pt,
    svg_xml: String,
    compiled: std::sync::Arc<Vec<svg::CompiledItem>>,
    use_form: bool,
    tag_role: Option<Arc<str>>,
    alt: Option<String>,
    pagination: Pagination,
    visible: bool,
}

impl SvgFlowable {
    pub fn new(width: f32, height: f32, svg_xml: impl Into<String>) -> Self {
        Self::new_pt(
            Pt::from_f32(width.max(0.0)),
            Pt::from_f32(height.max(0.0)),
            svg_xml,
        )
    }

    pub fn new_pt(width: Pt, height: Pt, svg_xml: impl Into<String>) -> Self {
        let width = width.max(Pt::ZERO);
        let height = height.max(Pt::ZERO);
        let svg_xml = svg_xml.into();
        let compiled = std::sync::Arc::new(svg::compile_svg(&svg_xml, width, height));
        Self {
            width,
            height,
            svg_xml,
            compiled,
            use_form: false,
            tag_role: None,
            alt: None,
            pagination: Pagination::default(),
            visible: true,
        }
    }

    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = pagination;
        self
    }

    pub fn with_tag_role(mut self, role: impl Into<Arc<str>>) -> Self {
        self.tag_role = Some(role.into());
        self
    }

    pub fn with_form_enabled(mut self, enabled: bool) -> Self {
        self.use_form = enabled;
        self
    }

    pub fn with_alt(mut self, alt: Option<String>) -> Self {
        self.alt = alt.filter(|v| !v.trim().is_empty());
        self
    }

    pub fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }
}

impl Flowable for SvgFlowable {
    fn wrap(&self, _avail_width: Pt, _avail_height: Pt) -> Size {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn intrinsic_width(&self) -> Option<Pt> {
        Some(self.width)
    }

    fn split(
        &self,
        _avail_width: Pt,
        _avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        None
    }

    fn draw(&self, canvas: &mut Canvas, x: Pt, y: Pt, _avail_width: Pt, _avail_height: Pt) {
        if !self.visible {
            return;
        }
        let tagged = self.tag_role.as_ref().map(|role| {
            canvas.begin_tag(role.as_ref(), self.alt.clone(), None, None, None, false);
        });
        if self.use_form {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            self.svg_xml.hash(&mut hasher);
            self.width.to_milli_i64().hash(&mut hasher);
            self.height.to_milli_i64().hash(&mut hasher);
            let form_id = format!("svg:{:x}", hasher.finish());

            let mut temp = Canvas::new(Size {
                width: self.width,
                height: self.height,
            });
            temp.save_state();
            temp.clip_rect(Pt::ZERO, Pt::ZERO, self.width, self.height);
            svg::render_compiled_items(&self.compiled, &mut temp, Pt::ZERO, Pt::ZERO);
            temp.restore_state();
            let doc = temp.finish();
            let commands = doc
                .pages
                .first()
                .map(|p| p.commands.clone())
                .unwrap_or_default();

            canvas.define_form(form_id.clone(), self.width, self.height, commands);
            canvas.draw_form(x, y, self.width, self.height, form_id);
        } else {
            // SVG should never spill outside its viewport in print contexts.
            canvas.save_state();
            canvas.clip_rect(x, y, self.width, self.height);

            // Render a precompiled, opinionated SVG 1.1-ish subset.
            // We still keep the original XML around for debugging, but avoid parsing on every draw.
            svg::render_compiled_items(&self.compiled, canvas, x, y);

            canvas.restore_state();
        }
        if tagged.is_some() {
            canvas.end_tag();
        }
    }

    fn pagination(&self) -> Pagination {
        self.pagination
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
    Justify,
}

fn text_align_offset(align: TextAlign, avail_width: Pt, line_width: Pt) -> Pt {
    match align {
        TextAlign::Left | TextAlign::Justify => Pt::ZERO,
        TextAlign::Center => ((avail_width - line_width).max(Pt::ZERO)).mul_ratio(1, 2),
        TextAlign::Right => (avail_width - line_width).max(Pt::ZERO),
    }
}

#[derive(Debug, Clone, Copy)]
pub enum VerticalAlign {
    Baseline,
    Top,
    Middle,
    Bottom,
}

#[derive(Debug, Clone, Copy)]
pub struct TableColumnWidthHint {
    width: LengthSpec,
    font_size: Pt,
    root_font_size: Pt,
}

impl TableColumnWidthHint {
    pub fn new(width: LengthSpec, font_size: Pt, root_font_size: Pt) -> Self {
        Self {
            width,
            font_size,
            root_font_size,
        }
    }

    fn resolve_width(self, avail_width: Pt) -> Pt {
        self.width
            .resolve_width(avail_width, self.font_size, self.root_font_size)
            .max(Pt::ZERO)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TableColumnBorder {
    widths: EdgeSizes,
    colors: ResolvedEdgeColors,
    styles: ResolvedEdgeStyles,
    hidden: ResolvedEdgeHidden,
    font_size: Pt,
    root_font_size: Pt,
}

impl TableColumnBorder {
    pub(crate) fn new(
        widths: EdgeSizes,
        colors: ResolvedEdgeColors,
        styles: ResolvedEdgeStyles,
        hidden: ResolvedEdgeHidden,
        font_size: Pt,
        root_font_size: Pt,
    ) -> Self {
        Self {
            widths,
            colors,
            styles,
            hidden,
            font_size,
            root_font_size,
        }
    }

    fn resolved_widths(self, avail_width: Pt) -> ResolvedEdges {
        self.widths
            .resolve(avail_width, self.font_size, self.root_font_size)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TableColumnGroupBorder {
    border: TableColumnBorder,
    starts_group: bool,
    ends_group: bool,
}

impl TableColumnGroupBorder {
    pub(crate) fn new(border: TableColumnBorder, starts_group: bool, ends_group: bool) -> Self {
        Self {
            border,
            starts_group,
            ends_group,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BorderSpec {
    pub widths: EdgeSizes,
    pub color: Color,
}

#[derive(Clone)]
pub struct TableCell {
    pub text: String,
    pub style: TextStyle,
    pub align: TextAlign,
    pub valign: VerticalAlign,
    pub padding: EdgeSizes,
    pub background: Option<Color>,
    pub border: BorderSpec,
    border_styles: ResolvedEdgeStyles,
    border_hidden: ResolvedEdgeHidden,
    self_visible: bool,
    row_collapsed: bool,
    row_border_widths: EdgeSizes,
    row_border_colors: ResolvedEdgeColors,
    row_border_styles: ResolvedEdgeStyles,
    row_border_hidden: ResolvedEdgeHidden,
    row_group_border_widths: EdgeSizes,
    row_group_border_colors: ResolvedEdgeColors,
    row_group_border_styles: ResolvedEdgeStyles,
    row_group_border_hidden: ResolvedEdgeHidden,
    row_group_starts: bool,
    row_group_ends: bool,
    pub box_shadow: Option<BoxShadowSpec>,
    pub tag_role: Option<Arc<str>>,
    pub scope: Option<String>,
    col_span: usize,
    pub root_font_size: Pt,
    row_min_height: Pt,
    preferred_width: Option<LengthSpec>,
    preferred_width_font_size: Pt,
    preferred_width_root_font_size: Pt,
    hide_empty_cells: bool,
    content: Option<Box<dyn Flowable>>,
    font_registry: Option<Arc<FontRegistry>>,
    cached_line_height: Pt,
    preserve_whitespace: bool,
    no_wrap: bool,
    layout_cache: Arc<Mutex<TextLayoutCache>>,
    width_cache: Arc<Mutex<TextWidthCache>>,
}

impl TableCell {
    pub(crate) fn new(
        text: String,
        style: TextStyle,
        align: TextAlign,
        valign: VerticalAlign,
        padding: EdgeSizes,
        background: Option<Color>,
        border: BorderSpec,
        box_shadow: Option<BoxShadowSpec>,
        tag_role: Option<Arc<str>>,
        scope: Option<String>,
        col_span: usize,
        root_font_size: Pt,
        font_registry: Option<Arc<FontRegistry>>,
        preserve_whitespace: bool,
        no_wrap: bool,
    ) -> Self {
        let style_font_size = style.font_size;
        let cached_line_height = if style.line_height_is_auto {
            if let Some(registry) = font_registry.as_deref() {
                registry.line_height(&style.font_name, style.font_size, style.line_height)
            } else {
                style.line_height
            }
        } else {
            style.line_height
        };

        Self {
            text,
            style,
            align,
            valign,
            padding,
            background,
            border,
            border_styles: ResolvedEdgeStyles::uniform(OutlineLineStyle::Solid),
            border_hidden: ResolvedEdgeHidden::none(),
            self_visible: true,
            row_collapsed: false,
            row_border_widths: EdgeSizes::zero(),
            row_border_colors: ResolvedEdgeColors::uniform(Color::BLACK),
            row_border_styles: ResolvedEdgeStyles::uniform(OutlineLineStyle::Solid),
            row_border_hidden: ResolvedEdgeHidden::none(),
            row_group_border_widths: EdgeSizes::zero(),
            row_group_border_colors: ResolvedEdgeColors::uniform(Color::BLACK),
            row_group_border_styles: ResolvedEdgeStyles::uniform(OutlineLineStyle::Solid),
            row_group_border_hidden: ResolvedEdgeHidden::none(),
            row_group_starts: false,
            row_group_ends: false,
            box_shadow,
            tag_role,
            scope,
            col_span: col_span.max(1),
            root_font_size,
            row_min_height: Pt::ZERO,
            preferred_width: None,
            preferred_width_font_size: style_font_size,
            preferred_width_root_font_size: root_font_size,
            hide_empty_cells: false,
            content: None,
            font_registry,
            cached_line_height,
            preserve_whitespace,
            no_wrap,
            layout_cache: Arc::new(Mutex::new(TextLayoutCache::default())),
            width_cache: Arc::new(Mutex::new(TextWidthCache::default())),
        }
    }

    pub(crate) fn with_content(mut self, content: Box<dyn Flowable>) -> Self {
        self.content = Some(content);
        self
    }

    pub(crate) fn col_span(&self) -> usize {
        self.col_span.max(1)
    }

    pub(crate) fn with_row_min_height(mut self, min_height: Pt) -> Self {
        self.row_min_height = min_height.max(Pt::ZERO);
        self
    }

    pub(crate) fn with_preferred_width(
        mut self,
        width: LengthSpec,
        font_size: Pt,
        root_font_size: Pt,
    ) -> Self {
        self.preferred_width = Some(width);
        self.preferred_width_font_size = font_size;
        self.preferred_width_root_font_size = root_font_size;
        self
    }

    pub(crate) fn with_hide_empty_cells(mut self, hide: bool) -> Self {
        self.hide_empty_cells = hide;
        self
    }

    pub(crate) fn with_border_styles(
        mut self,
        top: OutlineLineStyle,
        right: OutlineLineStyle,
        bottom: OutlineLineStyle,
        left: OutlineLineStyle,
    ) -> Self {
        self.border_styles = ResolvedEdgeStyles {
            top,
            right,
            bottom,
            left,
        };
        self
    }

    pub(crate) fn with_hidden_borders(
        mut self,
        top: bool,
        right: bool,
        bottom: bool,
        left: bool,
    ) -> Self {
        self.border_hidden = ResolvedEdgeHidden {
            top,
            right,
            bottom,
            left,
        };
        self
    }

    pub(crate) fn with_self_visible(mut self, visible: bool) -> Self {
        self.self_visible = visible;
        self
    }

    pub(crate) fn with_row_collapsed(mut self, collapsed: bool) -> Self {
        self.row_collapsed = collapsed;
        self
    }

    pub(crate) fn with_row_border(
        mut self,
        border_widths: EdgeSizes,
        border_colors: ResolvedEdgeColors,
        border_styles: ResolvedEdgeStyles,
        border_hidden: ResolvedEdgeHidden,
    ) -> Self {
        self.row_border_widths = border_widths;
        self.row_border_colors = border_colors;
        self.row_border_styles = border_styles;
        self.row_border_hidden = border_hidden;
        self
    }

    pub(crate) fn with_row_group_border(
        mut self,
        border_widths: EdgeSizes,
        border_colors: ResolvedEdgeColors,
        border_styles: ResolvedEdgeStyles,
        border_hidden: ResolvedEdgeHidden,
        starts_group: bool,
        ends_group: bool,
    ) -> Self {
        self.row_group_border_widths = border_widths;
        self.row_group_border_colors = border_colors;
        self.row_group_border_styles = border_styles;
        self.row_group_border_hidden = border_hidden;
        self.row_group_starts = starts_group;
        self.row_group_ends = ends_group;
        self
    }

    fn should_hide_empty_paint(&self) -> bool {
        self.hide_empty_cells && self.content.is_none() && self.text.trim().is_empty()
    }

    fn measure_text_width(&self, text: &str) -> Pt {
        if let Ok(cache) = self.width_cache.lock() {
            if let Some(value) = cache.get(text) {
                if perf_enabled() {
                    log_perf_counts("layout.tablecell.width", &[("cache_hit", 1)]);
                }
                return value;
            }
        }
        if let Some(value) =
            tabbed_text_width(&self.style, text, |part| self.measure_text_width(part))
        {
            if let Ok(mut cache) = self.width_cache.lock() {
                cache.insert(text, value);
            }
            if perf_enabled() {
                log_perf_counts("layout.tablecell.width", &[("cache_miss", 1)]);
            }
            return value;
        }
        if let Some(registry) = self.font_registry.as_deref() {
            let (primary, fallbacks) = resolve_font_stack(Some(registry), &self.style);
            let base = if fallbacks.is_empty() {
                registry.measure_text_width(&primary, self.style.font_size, text)
            } else {
                registry.measure_text_width_with_fallbacks(
                    &primary,
                    &fallbacks,
                    self.style.font_size,
                    text,
                )
            };
            let value = text_width_with_spacing(base, &self.style, text);
            if let Ok(mut cache) = self.width_cache.lock() {
                cache.insert(text, value);
            }
            if perf_enabled() {
                log_perf_counts("layout.tablecell.width", &[("cache_miss", 1)]);
            }
            value
        } else {
            let char_width = (self.style.font_size * 0.6).max(Pt::from_f32(1.0));
            let count = text.chars().count();
            let base = char_width * (count as i32);
            let value = text_width_with_spacing(base, &self.style, text);
            if let Ok(mut cache) = self.width_cache.lock() {
                cache.insert(text, value);
            }
            if perf_enabled() {
                log_perf_counts("layout.tablecell.width", &[("cache_miss", 1)]);
            }
            value
        }
    }

    fn effective_line_height(&self) -> Pt {
        self.cached_line_height
    }

    fn max_line_width(&self) -> Pt {
        if let Some(content) = self.content.as_ref() {
            return content.intrinsic_width().unwrap_or(Pt::ZERO);
        }
        let mut max = Pt::ZERO;
        for line in self.text.split('\n') {
            max = max.max(self.measure_text_width(line));
        }
        max
    }

    fn min_word_width(&self) -> Pt {
        if let Some(content) = self.content.as_ref() {
            return content.intrinsic_width().unwrap_or(Pt::ZERO);
        }
        if self.no_wrap {
            return self.max_line_width();
        }

        let mut max = Pt::ZERO;
        if self.preserve_whitespace {
            for ch in self.text.chars() {
                let w = self.measure_text_width(&ch.to_string());
                max = max.max(w);
            }
            return max;
        }

        for word in self.text.split_whitespace() {
            let w = self.measure_text_width(word);
            max = max.max(w);
        }
        if max == Pt::ZERO {
            max = self.measure_text_width(&self.text);
        }
        max
    }

    fn layout_lines(&self, avail_width: Pt) -> Arc<Vec<LineLayout>> {
        let perf = perf_start();
        let max_width = avail_width.max(Pt::from_f32(1.0));
        let key = max_width.to_milli_i64();
        if let Ok(cache) = self.layout_cache.lock() {
            if let Some(lines) = cache.get(key) {
                if perf_enabled() {
                    log_perf_counts(
                        "layout.tablecell.counts",
                        &[
                            ("bytes", self.text.len() as u64),
                            ("lines", lines.len() as u64),
                            ("cache_hit", 1),
                        ],
                    );
                }
                perf_end("layout.tablecell.lines", perf);
                return lines;
            }
        }
        if self.no_wrap {
            let mut line_layouts = Vec::new();
            for line in self.text.split('\n') {
                let width = if line.is_empty() {
                    Pt::ZERO
                } else {
                    self.measure_text_width(line)
                };
                line_layouts.push(LineLayout {
                    text: line.to_string(),
                    width,
                    text_width: width,
                    indent: Pt::ZERO,
                    forced_start: false,
                });
            }
            let lines = Arc::new(line_layouts);
            if let Ok(mut cache) = self.layout_cache.lock() {
                cache.insert(key, lines.clone());
            }
            if perf_enabled() {
                log_perf_counts(
                    "layout.tablecell.counts",
                    &[
                        ("bytes", self.text.len() as u64),
                        ("lines", lines.len() as u64),
                        ("cache_miss", 1),
                    ],
                );
            }
            perf_end("layout.tablecell.lines", perf);
            return lines;
        }

        let mut lines = Vec::new();
        let mut word_widths: HashMap<&str, Pt> = HashMap::new();
        if self.preserve_whitespace {
            let mut ascii_widths: [Option<Pt>; 128] = std::array::from_fn(|_| None);
            let mut non_ascii_widths: HashMap<char, Pt> = HashMap::new();
            for segment in self.text.split('\n') {
                if segment.is_empty() {
                    lines.push(String::new());
                    continue;
                }
                let mut current = String::new();
                let mut current_width = Pt::ZERO;
                for ch in segment.chars() {
                    let w = if (ch as u32) < 128 {
                        let idx = ch as usize;
                        if let Some(value) = ascii_widths[idx] {
                            value
                        } else {
                            let value = self.measure_text_width(&ch.to_string());
                            ascii_widths[idx] = Some(value);
                            value
                        }
                    } else if let Some(value) = non_ascii_widths.get(&ch) {
                        *value
                    } else {
                        let value = self.measure_text_width(&ch.to_string());
                        non_ascii_widths.insert(ch, value);
                        value
                    };
                    let mut next_width = current_width + w;
                    if !current.is_empty() && next_width > max_width {
                        lines.push(current);
                        current = String::new();
                        next_width = w;
                    }
                    current.push(ch);
                    current_width = next_width;
                }
                if !current.is_empty() {
                    lines.push(current);
                }
            }
        } else {
            let space_width = self.measure_text_width(" ");
            for segment in self.text.split('\n') {
                if segment.is_empty() {
                    lines.push(String::new());
                    continue;
                }
                let mut current = String::new();
                let mut current_width = Pt::ZERO;
                let words: Vec<(&str, Pt)> = segment
                    .split_whitespace()
                    .map(|word| {
                        let width = if let Some(value) = word_widths.get(word) {
                            *value
                        } else {
                            let value = self.measure_text_width(word);
                            word_widths.insert(word, value);
                            value
                        };
                        (word, width)
                    })
                    .collect();
                for (word, word_width) in words {
                    if current.is_empty() {
                        if word_width > max_width {
                            lines.extend(split_long_word_by_width_paragraph(self, word, max_width));
                            current.clear();
                        } else {
                            current.push_str(word);
                            current_width = word_width;
                        }
                    } else {
                        let next_width = current_width + space_width + word_width;
                        if next_width <= max_width {
                            current.push(' ');
                            current.push_str(word);
                            current_width = next_width;
                        } else {
                            lines.push(current);
                            current = String::new();
                            if word_width > max_width {
                                lines.extend(split_long_word_by_width_paragraph(
                                    self, word, max_width,
                                ));
                            } else {
                                current.push_str(word);
                                current_width = word_width;
                            }
                        }
                    }
                }
                if !current.is_empty() {
                    lines.push(current);
                }
            }
        }

        if lines.is_empty() {
            lines.push(String::new());
        }

        let mut line_layouts = Vec::with_capacity(lines.len());
        for line in lines {
            let width = if line.is_empty() {
                Pt::ZERO
            } else {
                self.measure_text_width(&line)
            };
            line_layouts.push(LineLayout {
                text: line,
                width,
                text_width: width,
                indent: Pt::ZERO,
                forced_start: false,
            });
        }
        let lines = Arc::new(line_layouts);
        if let Ok(mut cache) = self.layout_cache.lock() {
            cache.insert(key, lines.clone());
        }
        if perf_enabled() {
            log_perf_counts(
                "layout.tablecell.counts",
                &[
                    ("bytes", self.text.len() as u64),
                    ("lines", lines.len() as u64),
                    ("cache_miss", 1),
                ],
            );
        }
        perf_end("layout.tablecell.lines", perf);
        lines
    }

    fn resolved_padding(&self, avail_width: Pt) -> ResolvedEdges {
        self.padding
            .resolve(avail_width, self.style.font_size, self.root_font_size)
    }

    fn resolved_border(&self, avail_width: Pt) -> ResolvedEdges {
        self.border
            .widths
            .resolve(avail_width, self.style.font_size, self.root_font_size)
    }

    fn resolved_row_border(&self, avail_width: Pt) -> ResolvedEdges {
        self.row_border_widths
            .resolve(avail_width, self.style.font_size, self.root_font_size)
    }

    fn resolved_row_group_border(&self, avail_width: Pt) -> ResolvedEdges {
        self.row_group_border_widths
            .resolve(avail_width, self.style.font_size, self.root_font_size)
    }

    fn draw_inset_box_shadow(&self, canvas: &mut Canvas, x: Pt, y: Pt, width: Pt, height: Pt) {
        let Some(shadow) = self.box_shadow.as_ref() else {
            return;
        };
        if !shadow.inset || shadow.opacity <= 0.0 {
            return;
        }

        let blur = shadow
            .blur
            .resolve_width(width, self.style.font_size, self.root_font_size)
            .max(Pt::ZERO);
        let offset_x =
            shadow
                .offset_x
                .resolve_width(width, self.style.font_size, self.root_font_size);
        let offset_y =
            shadow
                .offset_y
                .resolve_height(height, self.style.font_size, self.root_font_size);
        let spread = shadow
            .spread
            .resolve_width(width, self.style.font_size, self.root_font_size)
            .max(Pt::ZERO);

        canvas.save_state();
        canvas.clip_rect(x, y, width, height);
        ContainerFlowable::draw_inset_shadow_layers(
            canvas,
            x,
            y,
            width,
            height,
            offset_x,
            offset_y,
            spread,
            blur,
            shadow.opacity.clamp(0.0, 1.0),
            shadow.color,
        );
        canvas.restore_state();
    }

    fn draw_text_line(&self, canvas: &mut Canvas, x: Pt, y: Pt, text: &str) {
        if let Some(expanded) = expanded_integer_tabs(&self.style, text) {
            self.draw_text_line(canvas, x, y, &expanded);
            return;
        }
        if text.contains('\t') {
            let space_width = self.measure_text_width(" ");
            let tab_advance = resolve_tab_advance(&self.style, space_width);
            let mut cursor_x = x;
            for (idx, part) in text.split('\t').enumerate() {
                if idx > 0 {
                    cursor_x = cursor_x + tab_advance;
                }
                if !part.is_empty() {
                    self.draw_text_line(canvas, cursor_x, y, part);
                    cursor_x = cursor_x + self.measure_text_width(part);
                }
            }
            return;
        }
        if let Some(registry) = self.font_registry.as_deref() {
            let (primary, fallbacks) = resolve_font_stack(Some(registry), &self.style);
            let runs = registry.split_text_by_fallbacks(&primary, &fallbacks, text);
            let mut cursor_x = x;
            let mut remaining = text.chars().count();
            for run in runs {
                emit_font_resolution_meta(
                    canvas,
                    registry,
                    &self.style,
                    &run.font_name,
                    primary.as_ref(),
                );
                canvas.set_font_name(&run.font_name);
                if !text_style_has_spacing(&self.style) {
                    let run_text = run.text;
                    let run_len = run_text.chars().count();
                    let w = registry.measure_text_width(
                        &run.font_name,
                        self.style.font_size,
                        &run_text,
                    );
                    draw_registered_text_run(
                        canvas,
                        registry,
                        &self.style,
                        &run.font_name,
                        cursor_x,
                        y,
                        run_text,
                    );
                    cursor_x = cursor_x + w;
                    remaining = remaining.saturating_sub(run_len);
                } else {
                    if registry.resolve(&run.font_name).is_none() {
                        let run_text = run.text;
                        let run_len = run_text.chars().count();
                        let w = registry.measure_text_width(
                            &run.font_name,
                            self.style.font_size,
                            &run_text,
                        );
                        draw_registered_text_run(
                            canvas,
                            registry,
                            &self.style,
                            &run.font_name,
                            cursor_x,
                            y,
                            run_text,
                        );
                        cursor_x = cursor_x + w;
                        remaining = remaining.saturating_sub(run_len);
                        continue;
                    }
                    for ch in run.text.chars() {
                        let ch_str = ch.to_string();
                        draw_registered_text_run(
                            canvas,
                            registry,
                            &self.style,
                            &run.font_name,
                            cursor_x,
                            y,
                            ch_str.clone(),
                        );
                        let w = registry.measure_text_width(
                            &run.font_name,
                            self.style.font_size,
                            &ch_str,
                        );
                        remaining = remaining.saturating_sub(1);
                        cursor_x =
                            cursor_x + w + text_spacing_after_char(&self.style, ch, remaining);
                    }
                }
            }
            return;
        }

        let font_name = resolve_font_variant_name(
            None,
            &self.style.font_name,
            self.style.font_weight,
            self.style.font_style,
        );
        canvas.set_font_name(font_name.as_ref());
        if !text_style_has_spacing(&self.style) {
            canvas.draw_string(x, y, text);
        } else {
            canvas.draw_string(x, y, text);
        }
    }
}

impl std::fmt::Debug for TableCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableCell")
            .field("text", &self.text)
            .field("style", &self.style)
            .field("align", &self.align)
            .field("valign", &self.valign)
            .field("padding", &self.padding)
            .field("background", &self.background)
            .field("border", &self.border)
            .field("border_styles", &self.border_styles)
            .field("border_hidden", &self.border_hidden)
            .field("row_border_widths", &self.row_border_widths)
            .field("row_border_colors", &self.row_border_colors)
            .field("row_border_styles", &self.row_border_styles)
            .field("row_border_hidden", &self.row_border_hidden)
            .field("row_group_border_widths", &self.row_group_border_widths)
            .field("row_group_border_colors", &self.row_group_border_colors)
            .field("row_group_border_styles", &self.row_group_border_styles)
            .field("row_group_border_hidden", &self.row_group_border_hidden)
            .field("row_group_starts", &self.row_group_starts)
            .field("row_group_ends", &self.row_group_ends)
            .field("box_shadow", &self.box_shadow)
            .field("tag_role", &self.tag_role)
            .field("scope", &self.scope)
            .field("col_span", &self.col_span)
            .field("root_font_size", &self.root_font_size)
            .field("row_min_height", &self.row_min_height)
            .field("preferred_width", &self.preferred_width)
            .field("has_content_flowable", &self.content.is_some())
            .finish()
    }
}

fn split_long_word_by_width_paragraph(cell: &TableCell, word: &str, max_width: Pt) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut current_width = Pt::ZERO;
    let mut ascii_widths: [Option<Pt>; 128] = std::array::from_fn(|_| None);
    let mut non_ascii_widths: HashMap<char, Pt> = HashMap::new();
    for ch in word.chars() {
        let w = if (ch as u32) < 128 {
            let idx = ch as usize;
            if let Some(value) = ascii_widths[idx] {
                value
            } else {
                let value = cell.measure_text_width(&ch.to_string());
                ascii_widths[idx] = Some(value);
                value
            }
        } else if let Some(value) = non_ascii_widths.get(&ch) {
            *value
        } else {
            let value = cell.measure_text_width(&ch.to_string());
            non_ascii_widths.insert(ch, value);
            value
        };
        let mut next_width = current_width + w;
        if !current.is_empty() && next_width > max_width {
            parts.push(current);
            current = String::new();
            next_width = w;
        }
        current.push(ch);
        current_width = next_width;
    }
    if !current.is_empty() {
        parts.push(current);
    }
    if parts.is_empty() {
        parts.push(String::new());
    }
    parts
}

#[derive(Debug, Clone)]
pub struct TableFlowable {
    data: Arc<TableFlowableData>,
    body_range: std::ops::Range<usize>,
    include_header: bool,
    repeat_header: bool,
    draw_background: bool,
    tag_role: Option<Arc<str>>,
    table_id: u32,
    border_collapse: BorderCollapseMode,
    border_spacing: BorderSpacingSpec,
    table_layout: TableLayoutMode,
    direction: DirectionMode,
    table_border_width: EdgeSizes,
    table_border_colors: ResolvedEdgeColors,
    table_border_styles: ResolvedEdgeStyles,
    table_border_hidden: ResolvedEdgeHidden,
    font_size: Pt,
    root_font_size: Pt,
    pagination: Pagination,
}

impl TableFlowable {
    pub fn new(rows: Vec<Vec<TableCell>>) -> Self {
        static TABLE_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
        let table_id = TABLE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let len = rows.len();
        Self {
            data: Arc::new(TableFlowableData {
                header_rows: Vec::new(),
                body_rows: rows,
                body_row_meta: vec![Vec::new(); len],
                column_width_hints: Vec::new(),
                column_borders: Vec::new(),
                column_group_borders: Vec::new(),
                collapsed_columns: Vec::new(),
                layout_cache: std::sync::OnceLock::new(),
            }),
            body_range: 0..len,
            include_header: true,
            repeat_header: false,
            draw_background: false,
            tag_role: None,
            table_id,
            border_collapse: BorderCollapseMode::Separate,
            border_spacing: BorderSpacingSpec::zero(),
            table_layout: TableLayoutMode::Auto,
            direction: DirectionMode::Ltr,
            table_border_width: EdgeSizes::zero(),
            table_border_colors: ResolvedEdgeColors::uniform(Color::BLACK),
            table_border_styles: ResolvedEdgeStyles::uniform(OutlineLineStyle::Solid),
            table_border_hidden: ResolvedEdgeHidden::none(),
            font_size: Pt::from_f32(12.0),
            root_font_size: Pt::from_f32(12.0),
            pagination: Pagination::default(),
        }
    }

    pub fn with_header(mut self, header_rows: Vec<Vec<TableCell>>) -> Self {
        if let Some(data) = Arc::get_mut(&mut self.data) {
            data.header_rows = header_rows;
            data.layout_cache = std::sync::OnceLock::new();
        } else {
            let mut owned = (*self.data).clone();
            owned.header_rows = header_rows;
            self.data = Arc::new(owned);
        }
        self
    }

    pub fn repeat_header(mut self, repeat: bool) -> Self {
        self.repeat_header = repeat;
        self
    }

    pub fn with_row_backgrounds(mut self, enabled: bool) -> Self {
        self.draw_background = enabled;
        self
    }

    pub fn with_tag_role(mut self, role: impl Into<Arc<str>>) -> Self {
        self.tag_role = Some(role.into());
        self
    }

    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = pagination;
        self
    }

    pub fn with_border_collapse(mut self, mode: BorderCollapseMode) -> Self {
        self.border_collapse = mode;
        self
    }

    pub fn with_border_spacing(mut self, spacing: BorderSpacingSpec) -> Self {
        self.border_spacing = spacing;
        self
    }

    pub fn with_table_layout(mut self, mode: TableLayoutMode) -> Self {
        self.table_layout = mode;
        self
    }

    pub fn with_direction(mut self, direction: DirectionMode) -> Self {
        self.direction = direction;
        self
    }

    pub fn with_table_border(mut self, border_width: EdgeSizes, border_color: Color) -> Self {
        self.table_border_width = border_width;
        self.table_border_colors = ResolvedEdgeColors::uniform(border_color);
        self
    }

    pub fn with_table_border_colors(
        mut self,
        top: Color,
        right: Color,
        bottom: Color,
        left: Color,
    ) -> Self {
        self.table_border_colors = ResolvedEdgeColors {
            top,
            right,
            bottom,
            left,
        };
        self
    }

    pub fn with_table_border_styles(
        mut self,
        top: OutlineLineStyle,
        right: OutlineLineStyle,
        bottom: OutlineLineStyle,
        left: OutlineLineStyle,
    ) -> Self {
        self.table_border_styles = ResolvedEdgeStyles {
            top,
            right,
            bottom,
            left,
        };
        self
    }

    pub fn with_table_hidden_borders(
        mut self,
        top: bool,
        right: bool,
        bottom: bool,
        left: bool,
    ) -> Self {
        self.table_border_hidden = ResolvedEdgeHidden {
            top,
            right,
            bottom,
            left,
        };
        self
    }

    pub fn with_column_width_hints(mut self, hints: Vec<Option<TableColumnWidthHint>>) -> Self {
        if let Some(data) = Arc::get_mut(&mut self.data) {
            data.column_width_hints = hints;
            data.layout_cache = std::sync::OnceLock::new();
        } else {
            let mut owned = (*self.data).clone();
            owned.column_width_hints = hints;
            owned.layout_cache = std::sync::OnceLock::new();
            self.data = Arc::new(owned);
        }
        self
    }

    pub fn with_column_borders(mut self, borders: Vec<Option<TableColumnBorder>>) -> Self {
        if let Some(data) = Arc::get_mut(&mut self.data) {
            data.column_borders = borders;
            data.layout_cache = std::sync::OnceLock::new();
        } else {
            let mut owned = (*self.data).clone();
            owned.column_borders = borders;
            owned.layout_cache = std::sync::OnceLock::new();
            self.data = Arc::new(owned);
        }
        self
    }

    pub fn with_column_group_borders(
        mut self,
        borders: Vec<Option<TableColumnGroupBorder>>,
    ) -> Self {
        if let Some(data) = Arc::get_mut(&mut self.data) {
            data.column_group_borders = borders;
            data.layout_cache = std::sync::OnceLock::new();
        } else {
            let mut owned = (*self.data).clone();
            owned.column_group_borders = borders;
            owned.layout_cache = std::sync::OnceLock::new();
            self.data = Arc::new(owned);
        }
        self
    }

    pub fn with_collapsed_columns(mut self, columns: Vec<bool>) -> Self {
        if let Some(data) = Arc::get_mut(&mut self.data) {
            data.collapsed_columns = columns;
            data.layout_cache = std::sync::OnceLock::new();
        } else {
            let mut owned = (*self.data).clone();
            owned.collapsed_columns = columns;
            owned.layout_cache = std::sync::OnceLock::new();
            self.data = Arc::new(owned);
        }
        self
    }

    pub fn with_font_metrics(mut self, font_size: Pt, root_font_size: Pt) -> Self {
        self.font_size = font_size;
        self.root_font_size = root_font_size;
        self
    }

    fn resolve_spacing(&self, avail_width: Pt) -> (Pt, Pt) {
        if matches!(self.border_collapse, BorderCollapseMode::Collapse) {
            return (Pt::ZERO, Pt::ZERO);
        }
        let horizontal = self.border_spacing.horizontal.resolve_width(
            avail_width,
            self.font_size,
            self.root_font_size,
        );
        let vertical = self.border_spacing.vertical.resolve_width(
            avail_width,
            self.font_size,
            self.root_font_size,
        );
        (horizontal.max(Pt::ZERO), vertical.max(Pt::ZERO))
    }

    fn column_spacing_total(visible_columns: usize, col_gap: Pt) -> Pt {
        if visible_columns == 0 {
            Pt::ZERO
        } else {
            col_gap * ((visible_columns + 1) as i32)
        }
    }

    fn has_draw_rows(&self) -> bool {
        self.visible_header_row_count() > 0
            || self.visible_body_row_count(self.body_range.start, self.body_range.end) > 0
    }

    fn outer_row_spacing(&self, row_gap: Pt) -> Pt {
        if self.has_draw_rows() {
            row_gap * 2
        } else {
            Pt::ZERO
        }
    }

    pub fn with_body_row_meta(mut self, meta: Vec<Vec<(String, String)>>) -> Self {
        let body_len = self.data.body_rows.len();
        let mut meta = meta;
        // Keep lengths aligned to avoid panics during draw/split.
        if meta.len() < body_len {
            meta.resize_with(body_len, Vec::new);
        } else if meta.len() > body_len {
            meta.truncate(body_len);
        }

        if let Some(data) = Arc::get_mut(&mut self.data) {
            data.body_row_meta = meta;
            data.layout_cache = std::sync::OnceLock::new();
        } else {
            let mut owned = (*self.data).clone();
            owned.body_row_meta = meta;
            self.data = Arc::new(owned);
        }
        self
    }

    fn max_columns(&self) -> usize {
        let mut max_cols = 0usize;
        for row in &self.data.header_rows {
            max_cols = max_cols.max(Self::row_total_columns(row));
        }
        for row in &self.data.body_rows {
            max_cols = max_cols.max(Self::row_total_columns(row));
        }
        max_cols = max_cols.max(self.data.column_width_hints.len());
        max_cols = max_cols.max(self.data.column_borders.len());
        max_cols = max_cols.max(self.data.column_group_borders.len());
        max_cols = max_cols.max(self.data.collapsed_columns.len());
        max_cols.max(1)
    }

    fn column_is_collapsed(&self, index: usize) -> bool {
        self.data.column_is_collapsed(index)
    }

    fn visible_column_count(&self, columns: usize) -> usize {
        (0..columns)
            .filter(|index| !self.column_is_collapsed(*index))
            .count()
    }

    fn has_visible_column_after(&self, next_index: usize, total_columns: usize) -> bool {
        (next_index..total_columns).any(|index| !self.column_is_collapsed(index))
    }

    fn visible_columns_in_span(
        &self,
        col_start: usize,
        col_span: usize,
        total_columns: usize,
    ) -> usize {
        let end = col_start.saturating_add(col_span).min(total_columns);
        (col_start..end)
            .filter(|index| !self.column_is_collapsed(*index))
            .count()
    }

    fn row_total_columns(row: &[TableCell]) -> usize {
        row.iter().map(TableCell::col_span).sum::<usize>().max(1)
    }

    fn row_is_collapsed(row: &[TableCell]) -> bool {
        row.first().map(|cell| cell.row_collapsed).unwrap_or(false)
    }

    fn visible_row_count(rows: &[Vec<TableCell>]) -> usize {
        rows.iter()
            .filter(|row| !Self::row_is_collapsed(row))
            .count()
    }

    fn visible_body_row_count(&self, start: usize, end: usize) -> usize {
        let start = start.min(self.data.body_rows.len());
        let end = end.min(self.data.body_rows.len()).max(start);
        Self::visible_row_count(&self.data.body_rows[start..end])
    }

    fn visible_header_row_count(&self) -> usize {
        if self.include_header {
            Self::visible_row_count(&self.data.header_rows)
        } else {
            0
        }
    }

    fn has_visible_header_row_after(&self, next_index: usize) -> bool {
        if !self.include_header {
            return false;
        }
        self.data
            .header_rows
            .iter()
            .skip(next_index)
            .any(|row| !Self::row_is_collapsed(row))
    }

    fn has_visible_body_row_after(&self, next_index: usize) -> bool {
        let end = self.body_range.end.min(self.data.body_rows.len());
        self.data
            .body_rows
            .iter()
            .enumerate()
            .skip(next_index.max(self.body_range.start))
            .take(end.saturating_sub(next_index.max(self.body_range.start)))
            .any(|(_, row)| !Self::row_is_collapsed(row))
    }

    fn cell_span_for_start(cell: &TableCell, col_start: usize, total_columns: usize) -> usize {
        let remaining = total_columns.saturating_sub(col_start).max(1);
        cell.col_span().min(remaining).max(1)
    }

    fn span_width(col_widths: &[Pt], col_start: usize, col_span: usize) -> Pt {
        let mut width = Pt::ZERO;
        for col in col_start..col_start.saturating_add(col_span) {
            width = width + col_widths.get(col).copied().unwrap_or(Pt::ZERO);
        }
        width
    }

    fn resolved_table_border(&self, table_width: Pt) -> ResolvedEdges {
        self.table_border_width
            .resolve(table_width, self.font_size, self.root_font_size)
    }

    fn column_border(&self, index: usize) -> Option<TableColumnBorder> {
        self.data
            .column_borders
            .get(index)
            .and_then(|border| *border)
    }

    fn column_group_border(&self, index: usize) -> Option<TableColumnGroupBorder> {
        self.data
            .column_group_borders
            .get(index)
            .and_then(|border| *border)
    }

    fn row_height(row: &[TableCell], col_widths: &[Pt]) -> Pt {
        if Self::row_is_collapsed(row) {
            return Pt::ZERO;
        }
        let mut max_height = Pt::ZERO;
        let mut cursor_col = 0usize;
        let total_columns = col_widths.len().max(1);
        for cell in row.iter() {
            let col_span = Self::cell_span_for_start(cell, cursor_col, total_columns);
            let col_width = Self::span_width(col_widths, cursor_col, col_span);
            if col_width <= Pt::ZERO {
                cursor_col = cursor_col.saturating_add(col_span);
                continue;
            }
            let padding = cell.resolved_padding(col_width);
            let border = cell.resolved_border(col_width);
            let pad_left = padding.left + border.left;
            let pad_right = padding.right + border.right;
            let pad_top = padding.top + border.top;
            let pad_bottom = padding.bottom + border.bottom;
            let content_width = (col_width - pad_left - pad_right).max(Pt::ZERO);
            let content_height = if let Some(content) = cell.content.as_ref() {
                content.wrap(content_width, huge_pt()).height
            } else {
                let lines = cell.layout_lines(content_width);
                cell.effective_line_height() * (lines.len() as i32)
            };
            let height = (content_height + pad_top + pad_bottom).max(cell.row_min_height);
            max_height = max_height.max(height);
            cursor_col = cursor_col.saturating_add(col_span);
        }
        max_height.max(Pt::ZERO)
    }

    fn row_height_for_draw_index(
        &self,
        draw_row_index: usize,
        row: &[TableCell],
        col_widths: &[Pt],
    ) -> Pt {
        if Self::row_is_collapsed(row) {
            return Pt::ZERO;
        }
        if matches!(self.border_collapse, BorderCollapseMode::Separate) {
            return Self::row_height(row, col_widths);
        }
        let mut max_height = Pt::ZERO;
        let mut cursor_col = 0usize;
        let total_columns = col_widths.len().max(1);
        for cell in row.iter() {
            let col_span = Self::cell_span_for_start(cell, cursor_col, total_columns);
            let col_width = Self::span_width(col_widths, cursor_col, col_span);
            if col_width <= Pt::ZERO {
                cursor_col = cursor_col.saturating_add(col_span);
                continue;
            }
            let padding = cell.resolved_padding(col_width);
            let border = self
                .collapsed_border_for_cell(draw_row_index, cursor_col, col_span, col_widths, cell)
                .widths;
            let pad_left = padding.left + border.left;
            let pad_right = padding.right + border.right;
            let pad_top = padding.top + border.top;
            let pad_bottom = padding.bottom + border.bottom;
            let content_width = (col_width - pad_left - pad_right).max(Pt::ZERO);
            let content_height = if let Some(content) = cell.content.as_ref() {
                content.wrap(content_width, huge_pt()).height
            } else {
                let lines = cell.layout_lines(content_width);
                cell.effective_line_height() * (lines.len() as i32)
            };
            let height = (content_height + pad_top + pad_bottom).max(cell.row_min_height);
            max_height = max_height.max(height);
            cursor_col = cursor_col.saturating_add(col_span);
        }
        max_height.max(Pt::ZERO)
    }

    fn row_height_and_lines_for_draw_index(
        &self,
        draw_row_index: usize,
        row: &[TableCell],
        col_widths: &[Pt],
    ) -> (Pt, Vec<Arc<Vec<LineLayout>>>) {
        if Self::row_is_collapsed(row) {
            return (Pt::ZERO, Vec::new());
        }
        if matches!(self.border_collapse, BorderCollapseMode::Separate) {
            return TableLayoutCache::row_height_and_lines(row, col_widths);
        }
        let mut max_height = Pt::ZERO;
        let mut lines_out: Vec<Arc<Vec<LineLayout>>> = Vec::with_capacity(row.len());
        let mut cursor_col = 0usize;
        let total_columns = col_widths.len().max(1);
        for cell in row.iter() {
            let col_span = Self::cell_span_for_start(cell, cursor_col, total_columns);
            let col_width = Self::span_width(col_widths, cursor_col, col_span);
            if col_width <= Pt::ZERO {
                lines_out.push(Arc::new(Vec::<LineLayout>::new()));
                cursor_col = cursor_col.saturating_add(col_span);
                continue;
            }
            let padding = cell.resolved_padding(col_width);
            let border = self
                .collapsed_border_for_cell(draw_row_index, cursor_col, col_span, col_widths, cell)
                .widths;
            let pad_left = padding.left + border.left;
            let pad_right = padding.right + border.right;
            let pad_top = padding.top + border.top;
            let pad_bottom = padding.bottom + border.bottom;
            let content_width = (col_width - pad_left - pad_right).max(Pt::ZERO);
            let (height, lines) = if let Some(content) = cell.content.as_ref() {
                let content_height = content.wrap(content_width, huge_pt()).height;
                (
                    (content_height + pad_top + pad_bottom).max(cell.row_min_height),
                    Arc::new(Vec::<LineLayout>::new()),
                )
            } else {
                let lines = cell.layout_lines(content_width);
                (
                    (cell.effective_line_height() * (lines.len() as i32) + pad_top + pad_bottom)
                        .max(cell.row_min_height),
                    lines,
                )
            };
            max_height = max_height.max(height);
            lines_out.push(lines);
            cursor_col = cursor_col.saturating_add(col_span);
        }
        (max_height.max(Pt::ZERO), lines_out)
    }

    fn row_by_draw_index(&self, draw_row_index: usize) -> Option<&[TableCell]> {
        let header_len = if self.include_header {
            self.data.header_rows.len()
        } else {
            0
        };
        if draw_row_index < header_len {
            return self
                .data
                .header_rows
                .get(draw_row_index)
                .map(|row| row.as_slice());
        }
        let body_local = draw_row_index.saturating_sub(header_len);
        let body_index = self.body_range.start + body_local;
        if body_index >= self.body_range.end {
            return None;
        }
        self.data
            .body_rows
            .get(body_index)
            .map(|row| row.as_slice())
    }

    fn cell_layout_by_draw_index(
        &self,
        draw_row_index: usize,
        col_index: usize,
        total_columns: usize,
    ) -> Option<(&TableCell, usize, usize)> {
        let row = self.row_by_draw_index(draw_row_index)?;
        let mut cursor_col = 0usize;
        for cell in row.iter() {
            let col_span = Self::cell_span_for_start(cell, cursor_col, total_columns);
            let end_col = cursor_col.saturating_add(col_span);
            if col_index >= cursor_col && col_index < end_col {
                return Some((cell, cursor_col, col_span));
            }
            cursor_col = end_col;
        }
        None
    }

    fn stronger_edge(
        current: CollapsedBorderEdge,
        candidate: CollapsedBorderEdge,
        candidate_wins_equal_source_order: bool,
    ) -> CollapsedBorderEdge {
        if current.hidden || candidate.hidden {
            return CollapsedBorderEdge {
                width: Pt::ZERO,
                color: current.color,
                style: current.style,
                hidden: true,
                source: current.source,
            };
        }
        let current_style_priority = collapsed_table_style_priority(current.style);
        let candidate_style_priority = collapsed_table_style_priority(candidate.style);
        let current_source_priority = current.source.priority();
        let candidate_source_priority = candidate.source.priority();
        let candidate_wins = candidate.width > current.width
            || (candidate.width == current.width
                && (candidate_style_priority > current_style_priority
                    || (candidate_style_priority == current_style_priority
                        && (candidate_source_priority > current_source_priority
                            || (candidate_source_priority == current_source_priority
                                && candidate_wins_equal_source_order)))));
        if candidate_wins { candidate } else { current }
    }

    fn collapsed_border_for_cell(
        &self,
        row_index: usize,
        col_start: usize,
        col_span: usize,
        col_widths: &[Pt],
        cell: &TableCell,
    ) -> ResolvedBorder {
        let total_columns = col_widths.len().max(1);
        let col_span = col_span
            .max(1)
            .min(total_columns.saturating_sub(col_start).max(1));
        let col_end = col_start.saturating_add(col_span);
        let col_width = Self::span_width(col_widths, col_start, col_span);
        let table_width = Self::span_width(col_widths, 0, total_columns);
        let table_border = self.resolved_table_border(table_width);
        let cell_border = cell.resolved_border(col_width);
        let row_border = cell.resolved_row_border(col_width);
        let row_group_border = cell.resolved_row_group_border(col_width);
        let mut top = CollapsedBorderEdge::new(
            cell_border.top,
            cell.border.color,
            cell.border_styles.top,
            cell.border_hidden.top,
            BorderConflictSource::Cell,
        );
        let mut right = CollapsedBorderEdge::new(
            cell_border.right,
            cell.border.color,
            cell.border_styles.right,
            cell.border_hidden.right,
            BorderConflictSource::Cell,
        );
        let mut bottom = CollapsedBorderEdge::new(
            cell_border.bottom,
            cell.border.color,
            cell.border_styles.bottom,
            cell.border_hidden.bottom,
            BorderConflictSource::Cell,
        );
        let mut left = CollapsedBorderEdge::new(
            cell_border.left,
            cell.border.color,
            cell.border_styles.left,
            cell.border_hidden.left,
            BorderConflictSource::Cell,
        );

        top = Self::stronger_edge(
            top,
            CollapsedBorderEdge::new(
                row_border.top,
                cell.row_border_colors.top,
                cell.row_border_styles.top,
                cell.row_border_hidden.top,
                BorderConflictSource::Row,
            ),
            false,
        );
        bottom = Self::stronger_edge(
            bottom,
            CollapsedBorderEdge::new(
                row_border.bottom,
                cell.row_border_colors.bottom,
                cell.row_border_styles.bottom,
                cell.row_border_hidden.bottom,
                BorderConflictSource::Row,
            ),
            false,
        );

        if cell.row_group_starts {
            top = Self::stronger_edge(
                top,
                CollapsedBorderEdge::new(
                    row_group_border.top,
                    cell.row_group_border_colors.top,
                    cell.row_group_border_styles.top,
                    cell.row_group_border_hidden.top,
                    BorderConflictSource::RowGroup,
                ),
                false,
            );
        }
        if cell.row_group_ends {
            bottom = Self::stronger_edge(
                bottom,
                CollapsedBorderEdge::new(
                    row_group_border.bottom,
                    cell.row_group_border_colors.bottom,
                    cell.row_group_border_styles.bottom,
                    cell.row_group_border_hidden.bottom,
                    BorderConflictSource::RowGroup,
                ),
                false,
            );
        }

        if row_index == 0 {
            top = Self::stronger_edge(
                top,
                CollapsedBorderEdge::new(
                    table_border.top,
                    self.table_border_colors.top,
                    self.table_border_styles.top,
                    self.table_border_hidden.top,
                    BorderConflictSource::Table,
                ),
                false,
            );
        }

        if col_start == 0 {
            left = Self::stronger_edge(
                left,
                CollapsedBorderEdge::new(
                    row_group_border.left,
                    cell.row_group_border_colors.left,
                    cell.row_group_border_styles.left,
                    cell.row_group_border_hidden.left,
                    BorderConflictSource::RowGroup,
                ),
                false,
            );
            if let Some(group_border) = self.column_group_border(0) {
                if group_border.starts_group {
                    let column_width = col_widths.first().copied().unwrap_or(Pt::ZERO);
                    let group_widths = group_border.border.resolved_widths(column_width);
                    left = Self::stronger_edge(
                        left,
                        CollapsedBorderEdge::new(
                            group_widths.left,
                            group_border.border.colors.left,
                            group_border.border.styles.left,
                            group_border.border.hidden.left,
                            BorderConflictSource::ColumnGroup,
                        ),
                        false,
                    );
                }
            }
            if let Some(column_border) = self.column_border(0) {
                let column_width = col_widths.first().copied().unwrap_or(Pt::ZERO);
                let column_widths = column_border.resolved_widths(column_width);
                left = Self::stronger_edge(
                    left,
                    CollapsedBorderEdge::new(
                        column_widths.left,
                        column_border.colors.left,
                        column_border.styles.left,
                        column_border.hidden.left,
                        BorderConflictSource::Column,
                    ),
                    false,
                );
            }
            left = Self::stronger_edge(
                left,
                CollapsedBorderEdge::new(
                    table_border.left,
                    self.table_border_colors.left,
                    self.table_border_styles.left,
                    self.table_border_hidden.left,
                    BorderConflictSource::Table,
                ),
                false,
            );
        }

        if col_end >= total_columns {
            right = Self::stronger_edge(
                right,
                CollapsedBorderEdge::new(
                    row_group_border.right,
                    cell.row_group_border_colors.right,
                    cell.row_group_border_styles.right,
                    cell.row_group_border_hidden.right,
                    BorderConflictSource::RowGroup,
                ),
                false,
            );
        }

        if let Some(group_border) = self.column_group_border(col_end.saturating_sub(1)) {
            if group_border.ends_group {
                let column_width = col_widths
                    .get(col_end.saturating_sub(1))
                    .copied()
                    .unwrap_or(Pt::ZERO);
                let group_widths = group_border.border.resolved_widths(column_width);
                right = Self::stronger_edge(
                    right,
                    CollapsedBorderEdge::new(
                        group_widths.right,
                        group_border.border.colors.right,
                        group_border.border.styles.right,
                        group_border.border.hidden.right,
                        BorderConflictSource::ColumnGroup,
                    ),
                    false,
                );
            }
        }

        if let Some(column_border) = self.column_border(col_end.saturating_sub(1)) {
            let column_width = col_widths
                .get(col_end.saturating_sub(1))
                .copied()
                .unwrap_or(Pt::ZERO);
            let column_widths = column_border.resolved_widths(column_width);
            right = Self::stronger_edge(
                right,
                CollapsedBorderEdge::new(
                    column_widths.right,
                    column_border.colors.right,
                    column_border.styles.right,
                    column_border.hidden.right,
                    BorderConflictSource::Column,
                ),
                false,
            );
        }

        if col_end < total_columns {
            if let Some((right_cell, right_start, right_span)) =
                self.cell_layout_by_draw_index(row_index, col_end, total_columns)
            {
                let right_col_width = Self::span_width(col_widths, right_start, right_span);
                let right_border = right_cell.resolved_border(right_col_width);
                right = Self::stronger_edge(
                    right,
                    CollapsedBorderEdge::new(
                        right_border.left,
                        right_cell.border.color,
                        right_cell.border_styles.left,
                        right_cell.border_hidden.left,
                        BorderConflictSource::Cell,
                    ),
                    matches!(self.direction, DirectionMode::Rtl),
                );
            }
            if let Some(next_column_border) = self.column_border(col_end) {
                let next_column_width = col_widths.get(col_end).copied().unwrap_or(Pt::ZERO);
                let next_column_widths = next_column_border.resolved_widths(next_column_width);
                right = Self::stronger_edge(
                    right,
                    CollapsedBorderEdge::new(
                        next_column_widths.left,
                        next_column_border.colors.left,
                        next_column_border.styles.left,
                        next_column_border.hidden.left,
                        BorderConflictSource::Column,
                    ),
                    matches!(self.direction, DirectionMode::Rtl),
                );
            }
            if let Some(next_group_border) = self.column_group_border(col_end) {
                if next_group_border.starts_group {
                    let next_column_width = col_widths.get(col_end).copied().unwrap_or(Pt::ZERO);
                    let next_group_widths =
                        next_group_border.border.resolved_widths(next_column_width);
                    right = Self::stronger_edge(
                        right,
                        CollapsedBorderEdge::new(
                            next_group_widths.left,
                            next_group_border.border.colors.left,
                            next_group_border.border.styles.left,
                            next_group_border.border.hidden.left,
                            BorderConflictSource::ColumnGroup,
                        ),
                        matches!(self.direction, DirectionMode::Rtl),
                    );
                }
            }
        } else {
            right = Self::stronger_edge(
                right,
                CollapsedBorderEdge::new(
                    table_border.right,
                    self.table_border_colors.right,
                    self.table_border_styles.right,
                    self.table_border_hidden.right,
                    BorderConflictSource::Table,
                ),
                false,
            );
        }

        for below_col in col_start..col_end {
            if let Some((below_cell, below_start, below_span)) =
                self.cell_layout_by_draw_index(row_index + 1, below_col, total_columns)
            {
                let below_col_width = Self::span_width(col_widths, below_start, below_span);
                let below_border = below_cell.resolved_border(below_col_width);
                bottom = Self::stronger_edge(
                    bottom,
                    CollapsedBorderEdge::new(
                        below_border.top,
                        below_cell.border.color,
                        below_cell.border_styles.top,
                        below_cell.border_hidden.top,
                        BorderConflictSource::Cell,
                    ),
                    false,
                );
                let below_row_border = below_cell.resolved_row_border(below_col_width);
                bottom = Self::stronger_edge(
                    bottom,
                    CollapsedBorderEdge::new(
                        below_row_border.top,
                        below_cell.row_border_colors.top,
                        below_cell.row_border_styles.top,
                        below_cell.row_border_hidden.top,
                        BorderConflictSource::Row,
                    ),
                    false,
                );
                if below_cell.row_group_starts {
                    let below_row_group_border =
                        below_cell.resolved_row_group_border(below_col_width);
                    bottom = Self::stronger_edge(
                        bottom,
                        CollapsedBorderEdge::new(
                            below_row_group_border.top,
                            below_cell.row_group_border_colors.top,
                            below_cell.row_group_border_styles.top,
                            below_cell.row_group_border_hidden.top,
                            BorderConflictSource::RowGroup,
                        ),
                        false,
                    );
                }
            }
        }
        if self.row_by_draw_index(row_index + 1).is_none() {
            bottom = Self::stronger_edge(
                bottom,
                CollapsedBorderEdge::new(
                    table_border.bottom,
                    self.table_border_colors.bottom,
                    self.table_border_styles.bottom,
                    self.table_border_hidden.bottom,
                    BorderConflictSource::Table,
                ),
                false,
            );
        }

        if row_index > 0 {
            top.width = Pt::ZERO;
        }
        if col_start > 0 {
            left.width = Pt::ZERO;
        }
        let styles = ResolvedEdgeStyles {
            top: top.style,
            right: right.style,
            bottom: bottom.style,
            left: left.style,
        }
        .collapsed_table();

        ResolvedBorder {
            widths: ResolvedEdges {
                top: top.width,
                right: right.width,
                bottom: bottom.width,
                left: left.width,
            },
            colors: ResolvedEdgeColors {
                top: top.color,
                right: right.color,
                bottom: bottom.color,
                left: left.color,
            },
            styles,
        }
    }

    fn draw_row_at(
        &self,
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        col_widths: &[Pt],
        col_gap: Pt,
        row: &[TableCell],
        row_height: Pt,
        row_index: usize,
        row_lines: Option<&[Arc<Vec<LineLayout>>]>,
    ) -> Pt {
        let row_tagged = self.tag_role.as_ref().map(|_| {
            canvas.begin_tag("TR", None, None, Some(self.table_id), None, true);
        });
        let total_columns = col_widths.len().max(1);
        let visible_columns = self.visible_column_count(total_columns);
        let mut cursor_x = if visible_columns > 0 { x + col_gap } else { x };
        let mut cursor_col = 0usize;
        for (cell_index, cell) in row.iter().enumerate() {
            let col_span = Self::cell_span_for_start(cell, cursor_col, total_columns);
            let visible_span_columns =
                self.visible_columns_in_span(cursor_col, col_span, total_columns);
            let internal_gaps = if visible_span_columns > 1 {
                col_gap * ((visible_span_columns - 1) as i32)
            } else {
                Pt::ZERO
            };
            let col_width = Self::span_width(col_widths, cursor_col, col_span) + internal_gaps;
            if col_width <= Pt::ZERO || visible_span_columns == 0 {
                cursor_col = cursor_col.saturating_add(col_span);
                continue;
            }
            let cell_x = cursor_x;
            let cell_y = y;
            let padding = cell.resolved_padding(col_width);
            let (border, border_colors, border_styles) =
                if matches!(self.border_collapse, BorderCollapseMode::Collapse) {
                    let resolved = self.collapsed_border_for_cell(
                        row_index, cursor_col, col_span, col_widths, cell,
                    );
                    (resolved.widths, resolved.colors, resolved.styles)
                } else {
                    (
                        cell.resolved_border(col_width),
                        ResolvedEdgeColors::uniform(cell.border.color),
                        cell.border_styles,
                    )
                };
            let pad_left = padding.left + border.left;
            let pad_right = padding.right + border.right;
            let pad_top = padding.top + border.top;
            let pad_bottom = padding.bottom + border.bottom;

            let tagged = cell.tag_role.as_ref().map(|role| {
                let col = u16::try_from(cursor_col).ok();
                canvas.begin_tag(
                    role.as_ref(),
                    None,
                    cell.scope.clone(),
                    Some(self.table_id),
                    col,
                    false,
                );
            });

            let hide_empty_paint = matches!(self.border_collapse, BorderCollapseMode::Separate)
                && cell.should_hide_empty_paint();

            if hide_empty_paint {
                if tagged.is_some() {
                    canvas.end_tag();
                }
                cursor_x = cursor_x + col_width;
                cursor_col = cursor_col.saturating_add(col_span);
                if self.has_visible_column_after(cursor_col, total_columns) {
                    cursor_x = cursor_x + col_gap;
                }
                continue;
            }

            let paint_self = cell.self_visible;
            if paint_self {
                if let Some(bg) = cell.background {
                    canvas.set_fill_color(bg);
                    canvas.draw_rect(cell_x, cell_y, col_width, row_height);
                }
                cell.draw_inset_box_shadow(canvas, cell_x, cell_y, col_width, row_height);

                if border.top > Pt::ZERO
                    || border.right > Pt::ZERO
                    || border.bottom > Pt::ZERO
                    || border.left > Pt::ZERO
                {
                    Self::draw_cell_border(
                        canvas,
                        cell_x,
                        cell_y,
                        col_width,
                        row_height,
                        border,
                        border_colors,
                        border_styles,
                    );
                }
            }

            let content_width = (col_width - pad_left - pad_right).max(Pt::ZERO);
            let content_height = (row_height - pad_top - pad_bottom).max(Pt::ZERO);
            if let Some(content) = cell.content.as_ref() {
                let wrapped = content.wrap(content_width, content_height);
                let draw_h = wrapped.height.min(content_height).max(Pt::ZERO);
                let draw_y = match cell.valign {
                    VerticalAlign::Baseline | VerticalAlign::Top => cell_y + pad_top,
                    VerticalAlign::Middle => {
                        cell_y + pad_top + (content_height - draw_h).mul_ratio(1, 2)
                    }
                    VerticalAlign::Bottom => cell_y + row_height - pad_bottom - draw_h,
                };
                content.draw(canvas, cell_x + pad_left, draw_y, content_width, draw_h);
            } else {
                let lines = if let Some(lines_for_row) = row_lines {
                    lines_for_row
                        .get(cell_index)
                        .cloned()
                        .unwrap_or_else(|| cell.layout_lines(content_width))
                } else {
                    cell.layout_lines(content_width)
                };
                let line_height = cell.effective_line_height();
                let text_block_height = line_height * (lines.len() as i32);
                let text_y = match cell.valign {
                    VerticalAlign::Baseline | VerticalAlign::Top => cell_y + pad_top,
                    VerticalAlign::Middle => {
                        cell_y + pad_top + (content_height - text_block_height).mul_ratio(1, 2)
                    }
                    VerticalAlign::Bottom => cell_y + row_height - pad_bottom - text_block_height,
                };

                if paint_self {
                    canvas.set_fill_color(cell.style.color);
                    canvas.set_font_size(cell.style.font_size);
                    let mut cursor_y = text_y.max(cell_y + pad_top);
                    for line in lines.iter() {
                        let line_width = line.width.min(content_width);
                        let text_x = cell_x
                            + pad_left
                            + text_align_offset(cell.align, content_width, line_width);
                        let draw_y = text_draw_y_for_line(
                            &cell.style,
                            cell.font_registry.as_deref(),
                            cursor_y,
                            line_height,
                        );
                        cell.draw_text_line(canvas, text_x, draw_y, &line.text);
                        draw_text_decorations(
                            canvas,
                            &cell.style,
                            cell.font_registry.as_deref(),
                            text_x,
                            draw_y,
                            line_width,
                        );
                        cursor_y = cursor_y + line_height;
                    }
                }
            }

            if tagged.is_some() {
                canvas.end_tag();
            }
            cursor_x = cursor_x + col_width;
            cursor_col = cursor_col.saturating_add(col_span);
            if self.has_visible_column_after(cursor_col, total_columns) {
                cursor_x = cursor_x + col_gap;
            }
        }
        if row_tagged.is_some() {
            canvas.end_tag();
        }
        row_height
    }

    fn draw_cell_border(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        border: ResolvedEdges,
        colors: ResolvedEdgeColors,
        styles: ResolvedEdgeStyles,
    ) {
        ContainerFlowable::draw_border(canvas, x, y, width, height, border, colors, styles);
    }
}

impl Flowable for TableFlowable {
    fn wrap(&self, avail_width: Pt, _avail_height: Pt) -> Size {
        let perf = perf_start();
        let columns = self.max_columns();
        let (col_gap, row_gap) = self.resolve_spacing(avail_width);
        let visible_columns = self.visible_column_count(columns);
        let gap_total = Self::column_spacing_total(visible_columns, col_gap);
        let avail_cols_width = (avail_width - gap_total).max(Pt::ZERO);
        let mut height = self.outer_row_spacing(row_gap);
        let cache = if matches!(self.border_collapse, BorderCollapseMode::Collapse) {
            None
        } else {
            self.data
                .cache_for_width(avail_cols_width, columns, self.table_layout)
        };
        let header_visible_count = self.visible_header_row_count();
        let body_visible_count =
            self.visible_body_row_count(self.body_range.start, self.body_range.end);
        if let Some(cache) = cache {
            if self.include_header {
                height += cache.header_total;
                if header_visible_count > 0 && body_visible_count > 0 {
                    height += row_gap;
                }
                if header_visible_count > 1 {
                    height += row_gap * ((header_visible_count - 1) as i32);
                }
            }
            let body_count = self.body_range.end.saturating_sub(self.body_range.start);
            if body_count > 0 {
                height += cache.body_prefix[self.body_range.end]
                    - cache.body_prefix[self.body_range.start];
                if body_visible_count > 1 {
                    height += row_gap * ((body_visible_count - 1) as i32);
                }
            }
        } else {
            let col_widths =
                self.data
                    .compute_column_widths(avail_cols_width, columns, self.table_layout);
            let mut row_index = 0usize;
            if self.include_header {
                for row in &self.data.header_rows {
                    height += self.row_height_for_draw_index(row_index, row, &col_widths);
                    row_index += 1;
                }
                if header_visible_count > 0 && body_visible_count > 0 {
                    height += row_gap;
                }
                if header_visible_count > 1 {
                    height += row_gap * ((header_visible_count - 1) as i32);
                }
            }
            for row in &self.data.body_rows[self.body_range.clone()] {
                height += self.row_height_for_draw_index(row_index, row, &col_widths);
                row_index += 1;
            }
            if body_visible_count > 1 {
                height += row_gap * ((body_visible_count - 1) as i32);
            }
        }
        if perf_enabled() {
            let header_rows = self.data.header_rows.len() as u64;
            let body_rows = self.body_range.end.saturating_sub(self.body_range.start) as u64;
            log_perf_counts(
                "layout.table.counts",
                &[
                    ("cols", columns as u64),
                    ("header_rows", header_rows),
                    ("body_rows", body_rows),
                ],
            );
        }
        if table_debug_enabled() {
            eprintln!(
                "[table.debug.wrap] id={} data_ptr={:p} cols={} avail_width_pt={:.3} body_rows={} include_header={} height_pt={:.3}",
                self.table_id,
                Arc::as_ptr(&self.data),
                columns,
                avail_width.to_f32(),
                self.body_range.end.saturating_sub(self.body_range.start),
                self.include_header,
                height.to_f32()
            );
        }
        perf_end("layout.table.wrap", perf);
        Size {
            width: avail_width,
            height,
        }
    }

    fn split(
        &self,
        avail_width: Pt,
        avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        let columns = self.max_columns();
        let (col_gap, row_gap) = self.resolve_spacing(avail_width);
        let visible_columns = self.visible_column_count(columns);
        let gap_total = Self::column_spacing_total(visible_columns, col_gap);
        let avail_cols_width = (avail_width - gap_total).max(Pt::ZERO);
        let cache = if matches!(self.border_collapse, BorderCollapseMode::Collapse) {
            None
        } else {
            self.data
                .cache_for_width(avail_cols_width, columns, self.table_layout)
        };
        let header_visible_count = self.visible_header_row_count();
        let body_visible_count =
            self.visible_body_row_count(self.body_range.start, self.body_range.end);
        let header_height = if self.include_header {
            if let Some(cache) = cache {
                let mut height = cache.header_total;
                if header_visible_count > 1 {
                    height += row_gap * ((header_visible_count - 1) as i32);
                }
                if header_visible_count > 0 && body_visible_count > 0 {
                    height += row_gap;
                }
                height
            } else {
                let col_widths =
                    self.data
                        .compute_column_widths(avail_cols_width, columns, self.table_layout);
                let mut height = Pt::ZERO;
                for (row_index, row) in self.data.header_rows.iter().enumerate() {
                    height += self.row_height_for_draw_index(row_index, row, &col_widths);
                }
                if header_visible_count > 1 {
                    height += row_gap * ((header_visible_count - 1) as i32);
                }
                if header_visible_count > 0 && body_visible_count > 0 {
                    height += row_gap;
                }
                height
            }
        } else {
            Pt::ZERO
        };
        let available = avail_height - header_height - self.outer_row_spacing(row_gap);
        if available <= Pt::ZERO {
            return None;
        }

        let start = self.body_range.start;
        let end = self.body_range.end;
        let body_len = end.saturating_sub(start);

        let split_at = if let Some(cache) = cache {
            // Binary search for the largest end index where sum(row_heights[start..end]) <= available.
            let prefix = &cache.body_prefix;
            let mut lo = start;
            let mut hi = end;
            while lo < hi {
                let mid = lo + (hi - lo + 1) / 2;
                let mut used = prefix[mid] - prefix[start];
                let visible_count = self.visible_body_row_count(start, mid);
                if visible_count > 1 {
                    used = used + row_gap * ((visible_count - 1) as i32);
                }
                if used <= available {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            lo
        } else {
            // Fallback: scan rows (should be rare; widths should be stable).
            let col_widths =
                self.data
                    .compute_column_widths(avail_cols_width, columns, self.table_layout);
            let mut used = Pt::ZERO;
            let mut idx = start;
            let mut visible_count = 0usize;
            let mut draw_row_index = if self.include_header {
                self.data.header_rows.len()
            } else {
                0
            };
            for row in &self.data.body_rows[self.body_range.clone()] {
                let row_height = self.row_height_for_draw_index(draw_row_index, row, &col_widths);
                let row_visible = !Self::row_is_collapsed(row);
                let gap = if row_visible && visible_count > 0 {
                    row_gap
                } else {
                    Pt::ZERO
                };
                if used + gap + row_height > available {
                    break;
                }
                used = used + gap + row_height;
                if row_visible {
                    visible_count += 1;
                }
                idx += 1;
                draw_row_index += 1;
            }
            idx
        };

        let max_rows = split_at.saturating_sub(start);
        if max_rows == 0 || max_rows >= body_len {
            return None;
        }

        let first = TableFlowable {
            data: self.data.clone(),
            body_range: start..split_at,
            include_header: self.include_header,
            repeat_header: self.repeat_header,
            draw_background: self.draw_background,
            tag_role: self.tag_role.clone(),
            table_id: self.table_id,
            border_collapse: self.border_collapse,
            border_spacing: self.border_spacing,
            table_layout: self.table_layout,
            direction: self.direction,
            table_border_width: self.table_border_width,
            table_border_colors: self.table_border_colors,
            table_border_styles: self.table_border_styles,
            table_border_hidden: self.table_border_hidden,
            font_size: self.font_size,
            root_font_size: self.root_font_size,
            pagination: Pagination {
                break_before: BreakBefore::Auto,
                break_after: BreakAfter::Auto,
                ..self.pagination
            },
        };
        let second = TableFlowable {
            data: self.data.clone(),
            body_range: split_at..end,
            include_header: self.repeat_header,
            repeat_header: self.repeat_header,
            draw_background: self.draw_background,
            tag_role: self.tag_role.clone(),
            table_id: self.table_id,
            border_collapse: self.border_collapse,
            border_spacing: self.border_spacing,
            table_layout: self.table_layout,
            direction: self.direction,
            table_border_width: self.table_border_width,
            table_border_colors: self.table_border_colors,
            table_border_styles: self.table_border_styles,
            table_border_hidden: self.table_border_hidden,
            font_size: self.font_size,
            root_font_size: self.root_font_size,
            pagination: Pagination {
                break_before: BreakBefore::Auto,
                ..self.pagination
            },
        };
        Some((Box::new(first), Box::new(second)))
    }

    fn draw(&self, canvas: &mut Canvas, x: Pt, y: Pt, avail_width: Pt, _avail_height: Pt) {
        let perf = perf_start();
        let tagged = self.tag_role.as_ref().map(|role| {
            canvas.begin_tag(role.as_ref(), None, None, None, None, true);
        });
        let columns = self.max_columns();
        let (col_gap, row_gap) = self.resolve_spacing(avail_width);
        let visible_columns = self.visible_column_count(columns);
        let gap_total = Self::column_spacing_total(visible_columns, col_gap);
        let avail_cols_width = (avail_width - gap_total).max(Pt::ZERO);
        let cache = if matches!(self.border_collapse, BorderCollapseMode::Collapse) {
            None
        } else {
            self.data
                .cache_for_width(avail_cols_width, columns, self.table_layout)
        };
        let col_widths = if let Some(cache) = cache {
            std::borrow::Cow::Borrowed(cache.col_widths.as_slice())
        } else {
            std::borrow::Cow::Owned(self.data.compute_column_widths(
                avail_cols_width,
                columns,
                self.table_layout,
            ))
        };
        let body_visible_count =
            self.visible_body_row_count(self.body_range.start, self.body_range.end);
        if table_debug_enabled() {
            let widths: Vec<String> = col_widths
                .iter()
                .map(|w| format!("{:.3}", w.to_f32()))
                .collect();
            eprintln!(
                "[table.debug.draw] id={} data_ptr={:p} cols={} avail_width_pt={:.3} col_widths_pt=[{}] body_rows={} include_header={} x_pt={:.3} y_pt={:.3}",
                self.table_id,
                Arc::as_ptr(&self.data),
                columns,
                avail_width.to_f32(),
                widths.join(","),
                self.body_range.end.saturating_sub(self.body_range.start),
                self.include_header,
                x.to_f32(),
                y.to_f32()
            );
        }
        let mut cursor_y = if self.has_draw_rows() { y + row_gap } else { y };
        let mut row_index = 0usize;
        if self.include_header && !self.data.header_rows.is_empty() {
            let head_tagged = self.tag_role.as_ref().map(|_| {
                canvas.begin_tag("THead", None, None, Some(self.table_id), None, true);
            });
            for (idx, row) in self.data.header_rows.iter().enumerate() {
                let cached_row_lines = cache.and_then(|c| c.header_row_lines.get(idx));
                let mut owned_row_lines: Option<Vec<Arc<Vec<LineLayout>>>> = None;
                let row_height = if let Some(value) =
                    cache.and_then(|c| c.header_row_heights.get(idx).copied())
                {
                    value
                } else {
                    let (height, lines) = self.row_height_and_lines_for_draw_index(
                        row_index,
                        row,
                        col_widths.as_ref(),
                    );
                    owned_row_lines = Some(lines);
                    height
                };
                if row_height <= Pt::ZERO {
                    row_index += 1;
                    continue;
                }
                let row_lines = if let Some(lines) = cached_row_lines {
                    Some(lines.as_slice())
                } else {
                    owned_row_lines.as_ref().map(|lines| lines.as_slice())
                };
                if self.draw_background {
                    canvas.set_fill_color(Color::rgb(0.9, 0.9, 0.9));
                    canvas.draw_rect(x, cursor_y, avail_width, row_height);
                }
                let row_height = self.draw_row_at(
                    canvas,
                    x,
                    cursor_y,
                    col_widths.as_ref(),
                    col_gap,
                    row,
                    row_height,
                    row_index,
                    row_lines,
                );
                cursor_y = cursor_y + row_height;
                row_index += 1;
                if self.has_visible_header_row_after(idx + 1) || body_visible_count > 0 {
                    cursor_y = cursor_y + row_gap;
                }
            }
            if head_tagged.is_some() {
                canvas.end_tag();
            }
        }

        let body_tagged = self.tag_role.as_ref().map(|_| {
            canvas.begin_tag("TBody", None, None, Some(self.table_id), None, true);
        });
        for (i, row) in self.data.body_rows[self.body_range.clone()]
            .iter()
            .enumerate()
        {
            let meta_index = self.body_range.start + i;
            let cached_row_lines = cache.and_then(|c| c.body_row_lines.get(meta_index));
            let mut owned_row_lines: Option<Vec<Arc<Vec<LineLayout>>>> = None;
            let row_height = if let Some(value) =
                cache.and_then(|c| c.body_row_heights.get(meta_index).copied())
            {
                value
            } else {
                let (height, lines) =
                    self.row_height_and_lines_for_draw_index(row_index, row, col_widths.as_ref());
                owned_row_lines = Some(lines);
                height
            };
            if row_height <= Pt::ZERO {
                row_index += 1;
                continue;
            }
            let row_lines = if let Some(lines) = cached_row_lines {
                Some(lines.as_slice())
            } else {
                owned_row_lines.as_ref().map(|lines| lines.as_slice())
            };
            if let Some(meta) = self.data.body_row_meta.get(meta_index) {
                for (k, v) in meta {
                    canvas.meta(k.clone(), v.clone());
                }
            }
            if self.draw_background {
                if row_index % 2 == 0 {
                    canvas.set_fill_color(Color::rgb(0.95, 0.95, 0.95));
                    canvas.draw_rect(x, cursor_y, avail_width, row_height);
                }
            }
            let row_height = self.draw_row_at(
                canvas,
                x,
                cursor_y,
                col_widths.as_ref(),
                col_gap,
                row,
                row_height,
                row_index,
                row_lines,
            );
            cursor_y = cursor_y + row_height;
            row_index += 1;
            if self.has_visible_body_row_after(meta_index + 1) {
                cursor_y = cursor_y + row_gap;
            }
        }
        if body_tagged.is_some() {
            canvas.end_tag();
        }
        if tagged.is_some() {
            canvas.end_tag();
        }
        perf_end("layout.table.draw", perf);
    }

    fn pagination(&self) -> Pagination {
        self.pagination
    }
}

#[derive(Debug)]
struct TableFlowableData {
    header_rows: Vec<Vec<TableCell>>,
    body_rows: Vec<Vec<TableCell>>,
    body_row_meta: Vec<Vec<(String, String)>>,
    column_width_hints: Vec<Option<TableColumnWidthHint>>,
    column_borders: Vec<Option<TableColumnBorder>>,
    column_group_borders: Vec<Option<TableColumnGroupBorder>>,
    collapsed_columns: Vec<bool>,
    layout_cache: std::sync::OnceLock<TableLayoutCache>,
}

impl TableFlowableData {
    fn column_is_collapsed(&self, index: usize) -> bool {
        self.collapsed_columns.get(index).copied().unwrap_or(false)
    }

    fn apply_collapsed_columns(&self, widths: &mut [i64]) {
        for (idx, width) in widths.iter_mut().enumerate() {
            if self.column_is_collapsed(idx) {
                *width = 0;
            }
        }
    }

    fn cache_for_width(
        &self,
        avail_width: Pt,
        columns: usize,
        table_layout: TableLayoutMode,
    ) -> Option<&TableLayoutCache> {
        let key = avail_width.to_milli_i64();
        if let Some(existing) = self.layout_cache.get() {
            if existing.avail_width_milli == key
                && existing.col_widths.len() == columns
                && existing.table_layout == table_layout
            {
                return Some(existing);
            }
            // Unexpected: width changed. Prefer correctness over caching.
            return None;
        }
        Some(
            self.layout_cache
                .get_or_init(|| TableLayoutCache::new(self, avail_width, columns, table_layout)),
        )
    }

    fn compute_column_widths(
        &self,
        avail_width: Pt,
        columns: usize,
        table_layout: TableLayoutMode,
    ) -> Vec<Pt> {
        let columns = columns.max(1);
        let debug_verbose = table_debug_enabled() && table_debug_verbose_enabled();
        let data_ptr = self as *const TableFlowableData as usize;

        if matches!(table_layout, TableLayoutMode::Fixed) {
            return self.compute_fixed_column_widths(avail_width, columns, debug_verbose, data_ptr);
        }

        let approx_col = avail_width / (columns as i32);
        if debug_verbose {
            eprintln!(
                "[table.debug.widths.begin] data_ptr=0x{:x} mode={:?} columns={} avail_width_pt={:.3} approx_col_pt={:.3} header_rows={} body_rows={}",
                data_ptr,
                table_layout,
                columns,
                avail_width.to_f32(),
                approx_col.to_f32(),
                self.header_rows.len(),
                self.body_rows.len()
            );
        }

        let row_count = self.header_rows.len() + self.body_rows.len();
        let mut min_widths = vec![0i64; columns];
        let mut max_widths = vec![0i64; columns];
        let mut preferred_widths = vec![0i64; columns];

        let ensure_span_requirement =
            |out: &mut [i64], start: usize, span: usize, required: i64| {
                if required <= 0 || start >= out.len() {
                    return;
                }
                let end = start.saturating_add(span).min(out.len());
                if start >= end {
                    return;
                }
                if end - start == 1 {
                    if required > out[start] {
                        out[start] = required;
                    }
                    return;
                }

                let current: i64 = out[start..end].iter().sum();
                if current >= required {
                    return;
                }
                let mut deficit = required - current;
                let slots = (end - start) as i64;
                let base = deficit / slots;
                if base > 0 {
                    for value in out[start..end].iter_mut() {
                        *value += base;
                    }
                    deficit -= base * slots;
                }
                let mut idx = start;
                while deficit > 0 {
                    out[idx] += 1;
                    deficit -= 1;
                    idx += 1;
                    if idx >= end {
                        idx = start;
                    }
                }
            };

        let update_row = |row_kind: &str,
                          row_index: usize,
                          row: &Vec<TableCell>,
                          min_out: &mut [i64],
                          max_out: &mut [i64],
                          pref_out: &mut [i64]| {
            let mut cursor_col = 0usize;
            for (cell_index, cell) in row.iter().enumerate() {
                if cursor_col >= columns {
                    break;
                }
                let col_span = cell
                    .col_span()
                    .min(columns.saturating_sub(cursor_col))
                    .max(1);
                let span_width = approx_col * (col_span as i32);
                let resolved_preferred = cell.preferred_width.map(|width_spec| {
                    width_spec
                        .resolve_width(
                            avail_width,
                            cell.preferred_width_font_size,
                            cell.preferred_width_root_font_size,
                        )
                        .max(Pt::ZERO)
                        .to_milli_i64()
                });
                if let Some(resolved) = resolved_preferred {
                    ensure_span_requirement(pref_out, cursor_col, col_span, resolved);
                }
                let padding = cell.resolved_padding(span_width);
                let border = cell.resolved_border(span_width);
                let extra = padding.left + padding.right + border.left + border.right;
                let (min_text, max_text) = if let Some(content) = cell.content.as_ref() {
                    let intrinsic = content.intrinsic_width().unwrap_or(Pt::ZERO);
                    let wrapped = content
                        .wrap(span_width.max(Pt::from_f32(1.0)), huge_pt())
                        .width;
                    (intrinsic, wrapped.max(intrinsic))
                } else {
                    (cell.min_word_width(), cell.max_line_width())
                };
                let min_w = (min_text + extra).to_milli_i64();
                let max_w = (max_text + extra).to_milli_i64();
                ensure_span_requirement(min_out, cursor_col, col_span, min_w);
                ensure_span_requirement(max_out, cursor_col, col_span, max_w);
                if debug_verbose {
                    let text_preview = cell
                        .text
                        .chars()
                        .take(24)
                        .collect::<String>()
                        .replace('\n', "\\n");
                    eprintln!(
                        "[table.debug.widths.cell] data_ptr=0x{:x} row={}#{} cell={} col_start={} span={} pref_milli={} has_content={} text_len={} text_preview=\"{}\" min_milli={} max_milli={} span_width_pt={:.3}",
                        data_ptr,
                        row_kind,
                        row_index,
                        cell_index,
                        cursor_col,
                        col_span,
                        resolved_preferred.unwrap_or(0),
                        cell.content.is_some(),
                        cell.text.chars().count(),
                        text_preview,
                        min_w,
                        max_w,
                        span_width.to_f32()
                    );
                }
                cursor_col = cursor_col.saturating_add(col_span);
            }
            if debug_verbose {
                eprintln!(
                    "[table.debug.widths.row] data_ptr=0x{:x} row={}#{} min={:?} max={:?} pref={:?}",
                    data_ptr, row_kind, row_index, min_out, max_out, pref_out
                );
            }
        };

        if row_count >= 64 && !debug_verbose {
            let merge = |mut a: (Vec<i64>, Vec<i64>, Vec<i64>),
                         b: (Vec<i64>, Vec<i64>, Vec<i64>)| {
                for i in 0..columns {
                    if b.0[i] > a.0[i] {
                        a.0[i] = b.0[i];
                    }
                    if b.1[i] > a.1[i] {
                        a.1[i] = b.1[i];
                    }
                    if b.2[i] > a.2[i] {
                        a.2[i] = b.2[i];
                    }
                }
                a
            };
            let (min_h, max_h, pref_h) = crate::parallel::fold_reduce(
                &self.header_rows,
                || {
                    (
                        vec![0i64; columns],
                        vec![0i64; columns],
                        vec![0i64; columns],
                    )
                },
                |mut acc, row| {
                    update_row("header", 0, row, &mut acc.0, &mut acc.1, &mut acc.2);
                    acc
                },
                &merge,
            );
            let (min_b, max_b, pref_b) = crate::parallel::fold_reduce(
                &self.body_rows,
                || {
                    (
                        vec![0i64; columns],
                        vec![0i64; columns],
                        vec![0i64; columns],
                    )
                },
                |mut acc, row| {
                    update_row("body", 0, row, &mut acc.0, &mut acc.1, &mut acc.2);
                    acc
                },
                &merge,
            );
            for i in 0..columns {
                min_widths[i] = min_h[i].max(min_b[i]);
                max_widths[i] = max_h[i].max(max_b[i]);
                preferred_widths[i] = pref_h[i].max(pref_b[i]);
            }
        } else {
            for (row_index, row) in self.header_rows.iter().enumerate() {
                update_row(
                    "header",
                    row_index,
                    row,
                    &mut min_widths,
                    &mut max_widths,
                    &mut preferred_widths,
                );
            }
            for (row_index, row) in self.body_rows.iter().enumerate() {
                update_row(
                    "body",
                    row_index,
                    row,
                    &mut min_widths,
                    &mut max_widths,
                    &mut preferred_widths,
                );
            }
        }

        for (column_index, hint) in self.column_width_hints.iter().enumerate().take(columns) {
            if let Some(hint) = hint {
                let resolved = hint.resolve_width(avail_width).to_milli_i64();
                if resolved > 0 {
                    min_widths[column_index] = min_widths[column_index].max(resolved);
                    max_widths[column_index] = max_widths[column_index].max(resolved);
                    preferred_widths[column_index] = preferred_widths[column_index].max(resolved);
                }
            }
        }

        for i in 0..columns {
            if preferred_widths[i] > 0 {
                min_widths[i] = min_widths[i].max(preferred_widths[i]);
                max_widths[i] = max_widths[i].max(preferred_widths[i]);
            }
        }

        let avail = avail_width.to_milli_i64().max(1);
        let total_min: i64 = min_widths.iter().sum();
        let total_max: i64 = max_widths.iter().sum();

        let mut widths = vec![0i64; columns];

        if total_max <= avail {
            let extra = avail - total_max;
            if total_max > 0 {
                let mut used = 0i64;
                for i in 0..columns {
                    let add = (extra as i128 * max_widths[i] as i128 / total_max as i128) as i64;
                    widths[i] = max_widths[i] + add;
                    used += add;
                }
                let mut rem = extra - used;
                let mut i = 0usize;
                while rem > 0 {
                    widths[i % columns] += 1;
                    rem -= 1;
                    i += 1;
                }
            } else {
                let base = avail / (columns as i64);
                let mut rem = avail - base * (columns as i64);
                for i in 0..columns {
                    widths[i] = base;
                    if rem > 0 {
                        widths[i] += 1;
                        rem -= 1;
                    }
                }
            }
        } else if total_min >= avail {
            if total_min > 0 {
                for i in 0..columns {
                    widths[i] = min_widths[i];
                }
            } else {
                let base = avail / (columns as i64);
                let mut rem = avail - base * (columns as i64);
                for i in 0..columns {
                    widths[i] = base;
                    if rem > 0 {
                        widths[i] += 1;
                        rem -= 1;
                    }
                }
            }
        } else {
            let extra = avail - total_min;
            let flex = total_max - total_min;
            let mut used = 0i64;
            for i in 0..columns {
                let span = max_widths[i] - min_widths[i];
                let add = if flex > 0 {
                    (extra as i128 * span as i128 / flex as i128) as i64
                } else {
                    0
                };
                widths[i] = min_widths[i] + add;
                used += add;
            }
            let mut rem = extra - used;
            let mut i = 0usize;
            while rem > 0 {
                widths[i % columns] += 1;
                rem -= 1;
                i += 1;
            }
        }

        if debug_verbose {
            eprintln!(
                "[table.debug.widths.end] data_ptr=0x{:x} mode={:?} avail_milli={} total_min={} total_max={} min={:?} max={:?} pref={:?} out={:?}",
                data_ptr,
                table_layout,
                avail,
                total_min,
                total_max,
                min_widths,
                max_widths,
                preferred_widths,
                widths
            );
        }

        self.apply_collapsed_columns(&mut widths);
        widths.into_iter().map(Pt::from_milli_i64).collect()
    }

    fn compute_fixed_column_widths(
        &self,
        avail_width: Pt,
        columns: usize,
        debug_verbose: bool,
        data_ptr: usize,
    ) -> Vec<Pt> {
        let columns = columns.max(1);
        let avail = avail_width.to_milli_i64().max(1);
        let approx_col = avail_width / (columns as i32);

        if debug_verbose {
            eprintln!(
                "[table.debug.widths.fixed.begin] data_ptr=0x{:x} columns={} avail_width_pt={:.3} approx_col_pt={:.3} header_rows={} body_rows={}",
                data_ptr,
                columns,
                avail_width.to_f32(),
                approx_col.to_f32(),
                self.header_rows.len(),
                self.body_rows.len()
            );
        }

        let mut hinted_widths = vec![0i64; columns];
        let mut hinted_columns = vec![false; columns];

        fn apply_hint_indices(
            hinted_widths: &mut [i64],
            hinted_columns: &mut [bool],
            indices: &[usize],
            required: i64,
            explicit_hint: bool,
        ) {
            if indices.is_empty() {
                return;
            }
            let required = required.max(0);
            let slots = indices.len() as i64;
            let base = required / slots;
            let mut rem = required - base * slots;
            for idx in indices {
                if *idx >= hinted_widths.len() {
                    continue;
                }
                if explicit_hint {
                    hinted_columns[*idx] = true;
                }
                let mut candidate = base;
                if rem > 0 {
                    candidate += 1;
                    rem -= 1;
                }
                if candidate > hinted_widths[*idx] {
                    hinted_widths[*idx] = candidate;
                }
            }
        }

        fn apply_hint_span(
            hinted_widths: &mut [i64],
            hinted_columns: &mut [bool],
            start: usize,
            span: usize,
            required: i64,
            explicit_hint: bool,
        ) {
            if start >= hinted_widths.len() {
                return;
            }
            let end = start.saturating_add(span).min(hinted_widths.len());
            if start >= end {
                return;
            }
            let indices: Vec<usize> = (start..end).collect();
            apply_hint_indices(
                hinted_widths,
                hinted_columns,
                &indices,
                required,
                explicit_hint,
            );
        }

        for (column_index, hint) in self.column_width_hints.iter().enumerate().take(columns) {
            if let Some(hint) = hint {
                let resolved = hint.resolve_width(avail_width).to_milli_i64();
                apply_hint_span(
                    &mut hinted_widths,
                    &mut hinted_columns,
                    column_index,
                    1,
                    resolved,
                    true,
                );
            }
        }

        let seed_row = self.header_rows.first().or_else(|| self.body_rows.first());
        if let Some(row) = seed_row {
            let mut cursor_col = 0usize;
            for cell in row {
                if cursor_col >= columns {
                    break;
                }
                let col_span = cell
                    .col_span()
                    .min(columns.saturating_sub(cursor_col))
                    .max(1);
                if let Some(width_spec) = cell.preferred_width {
                    let resolved = width_spec
                        .resolve_width(
                            avail_width,
                            cell.preferred_width_font_size,
                            cell.preferred_width_root_font_size,
                        )
                        .max(Pt::ZERO)
                        .to_milli_i64();
                    let span_end = cursor_col.saturating_add(col_span).min(columns);
                    let occupied: i64 = (cursor_col..span_end)
                        .filter(|idx| hinted_columns[*idx])
                        .map(|idx| hinted_widths[idx])
                        .sum();
                    let unhinted_indices: Vec<usize> = (cursor_col..span_end)
                        .filter(|idx| !hinted_columns[*idx])
                        .collect();
                    apply_hint_indices(
                        &mut hinted_widths,
                        &mut hinted_columns,
                        &unhinted_indices,
                        resolved.saturating_sub(occupied),
                        true,
                    );
                }
                cursor_col = cursor_col.saturating_add(col_span);
            }
        }

        let mut widths = vec![0i64; columns];
        let hinted_indices: Vec<usize> = (0..columns).filter(|&idx| hinted_columns[idx]).collect();
        let unhinted_indices: Vec<usize> =
            (0..columns).filter(|&idx| !hinted_columns[idx]).collect();
        let hinted_total: i64 = hinted_indices.iter().map(|idx| hinted_widths[*idx]).sum();

        let distribute_even = |out: &mut [i64], indices: &[usize], total: i64| {
            if indices.is_empty() || total <= 0 {
                return;
            }
            let slots = indices.len() as i64;
            let base = total / slots;
            let mut rem = total - base * slots;
            for idx in indices {
                out[*idx] += base;
                if rem > 0 {
                    out[*idx] += 1;
                    rem -= 1;
                }
            }
        };

        if hinted_indices.is_empty() {
            let all_indices: Vec<usize> = (0..columns).collect();
            distribute_even(&mut widths, &all_indices, avail);
        } else if hinted_total >= avail {
            for idx in &hinted_indices {
                widths[*idx] = hinted_widths[*idx];
            }
        } else {
            for idx in &hinted_indices {
                widths[*idx] = hinted_widths[*idx];
            }
            let rem = avail - hinted_total;
            if !unhinted_indices.is_empty() {
                distribute_even(&mut widths, &unhinted_indices, rem);
            } else {
                distribute_even(&mut widths, &hinted_indices, rem);
            }
        }

        let mut total: i64 = widths.iter().sum();
        if total < avail {
            let deficit = avail - total;
            let all_indices: Vec<usize> = (0..columns).collect();
            distribute_even(&mut widths, &all_indices, deficit);
            total = widths.iter().sum();
        }

        if debug_verbose {
            eprintln!(
                "[table.debug.widths.fixed.end] data_ptr=0x{:x} avail_milli={} hinted={:?} hinted_cols={:?} total_milli={} out={:?}",
                data_ptr, avail, hinted_widths, hinted_columns, total, widths
            );
        }

        self.apply_collapsed_columns(&mut widths);
        widths.into_iter().map(Pt::from_milli_i64).collect()
    }
}

impl Clone for TableFlowableData {
    fn clone(&self) -> Self {
        Self {
            header_rows: self.header_rows.clone(),
            body_rows: self.body_rows.clone(),
            body_row_meta: self.body_row_meta.clone(),
            column_width_hints: self.column_width_hints.clone(),
            column_borders: self.column_borders.clone(),
            column_group_borders: self.column_group_borders.clone(),
            collapsed_columns: self.collapsed_columns.clone(),
            layout_cache: std::sync::OnceLock::new(),
        }
    }
}

#[derive(Debug)]
struct TableLayoutCache {
    avail_width_milli: i64,
    table_layout: TableLayoutMode,
    col_widths: Vec<Pt>,
    header_row_heights: Vec<Pt>,
    body_row_heights: Vec<Pt>,
    header_row_lines: Vec<Vec<Arc<Vec<LineLayout>>>>,
    body_row_lines: Vec<Vec<Arc<Vec<LineLayout>>>>,
    header_total: Pt,
    body_prefix: Vec<Pt>,
}

impl TableLayoutCache {
    fn new(
        data: &TableFlowableData,
        avail_width: Pt,
        columns: usize,
        table_layout: TableLayoutMode,
    ) -> Self {
        let col_widths = data.compute_column_widths(avail_width, columns, table_layout);
        let mut header_row_heights = Vec::with_capacity(data.header_rows.len());
        let mut header_row_lines = Vec::with_capacity(data.header_rows.len());
        let mut header_total = Pt::ZERO;
        let header_results: Vec<(Pt, Vec<Arc<Vec<LineLayout>>>)> = if data.header_rows.len() >= 32 {
            crate::parallel::map_ordered(&data.header_rows, |row| {
                TableLayoutCache::row_height_and_lines(row, &col_widths)
            })
        } else {
            data.header_rows
                .iter()
                .map(|row| TableLayoutCache::row_height_and_lines(row, &col_widths))
                .collect()
        };
        for (h, lines) in header_results {
            header_row_heights.push(h);
            header_row_lines.push(lines);
            header_total = header_total + h;
        }

        let body_results: Vec<(Pt, Vec<Arc<Vec<LineLayout>>>)> = if data.body_rows.len() >= 32 {
            crate::parallel::map_ordered(&data.body_rows, |row| {
                TableLayoutCache::row_height_and_lines(row, &col_widths)
            })
        } else {
            data.body_rows
                .iter()
                .map(|row| TableLayoutCache::row_height_and_lines(row, &col_widths))
                .collect()
        };

        let mut body_row_heights = Vec::with_capacity(body_results.len());
        let mut body_row_lines = Vec::with_capacity(body_results.len());
        let mut body_prefix = Vec::with_capacity(body_results.len() + 1);
        body_prefix.push(Pt::ZERO);
        let mut acc = Pt::ZERO;
        for (h, lines) in body_results {
            body_row_heights.push(h);
            body_row_lines.push(lines);
            acc = acc + h;
            body_prefix.push(acc);
        }

        Self {
            avail_width_milli: avail_width.to_milli_i64(),
            table_layout,
            col_widths,
            header_row_heights,
            body_row_heights,
            header_row_lines,
            body_row_lines,
            header_total,
            body_prefix,
        }
    }

    fn row_height_and_lines(
        row: &[TableCell],
        col_widths: &[Pt],
    ) -> (Pt, Vec<Arc<Vec<LineLayout>>>) {
        if TableFlowable::row_is_collapsed(row) {
            return (Pt::ZERO, Vec::new());
        }
        let mut max_height = Pt::ZERO;
        let mut lines_out: Vec<Arc<Vec<LineLayout>>> = Vec::with_capacity(row.len());
        let mut cursor_col = 0usize;
        let total_columns = col_widths.len().max(1);
        for cell in row.iter() {
            let col_span = TableFlowable::cell_span_for_start(cell, cursor_col, total_columns);
            let col_width = TableFlowable::span_width(col_widths, cursor_col, col_span);
            if col_width <= Pt::ZERO {
                lines_out.push(Arc::new(Vec::<LineLayout>::new()));
                cursor_col = cursor_col.saturating_add(col_span);
                continue;
            }
            let padding = cell.resolved_padding(col_width);
            let border = cell.resolved_border(col_width);
            let pad_left = padding.left + border.left;
            let pad_right = padding.right + border.right;
            let pad_top = padding.top + border.top;
            let pad_bottom = padding.bottom + border.bottom;
            let content_width = (col_width - pad_left - pad_right).max(Pt::ZERO);
            let (height, lines) = if let Some(content) = cell.content.as_ref() {
                let content_height = content.wrap(content_width, huge_pt()).height;
                (
                    (content_height + pad_top + pad_bottom).max(cell.row_min_height),
                    Arc::new(Vec::<LineLayout>::new()),
                )
            } else {
                let lines = cell.layout_lines(content_width);
                (
                    (cell.effective_line_height() * (lines.len() as i32) + pad_top + pad_bottom)
                        .max(cell.row_min_height),
                    lines,
                )
            };
            max_height = max_height.max(height);
            lines_out.push(lines);
            cursor_col = cursor_col.saturating_add(col_span);
        }
        (max_height.max(Pt::ZERO), lines_out)
    }
}

#[derive(Clone)]
struct InlineItemLayout {
    idx: usize,
    x_off: Pt,
    size: Size,
    valign: VerticalAlign,
    baseline: Option<Pt>,
}

#[derive(Clone)]
struct InlineLineLayout {
    line_height: Pt,
    baseline: Option<Pt>,
    items: Vec<InlineItemLayout>,
}

#[derive(Clone)]
struct InlineLayoutCache {
    avail_width_milli: i64,
    max_width: Pt,
    total_height: Pt,
    lines: Vec<InlineLineLayout>,
}

#[derive(Clone)]
pub struct InlineBlockLayoutFlowable {
    children: Vec<(Box<dyn Flowable>, VerticalAlign)>,
    gap: Pt,
    forced_line_height: Option<Pt>,
    css_pixel_snap: bool,
    pagination: Pagination,
    layout_cache: Arc<Mutex<Option<InlineLayoutCache>>>,
}

impl InlineBlockLayoutFlowable {
    pub fn new_pt(
        children: Vec<(Box<dyn Flowable>, VerticalAlign)>,
        gap: Pt,
        forced_line_height: Option<Pt>,
    ) -> Self {
        Self {
            children,
            gap,
            forced_line_height,
            css_pixel_snap: false,
            pagination: Pagination::default(),
            layout_cache: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_css_pixel_line_snap(mut self, enabled: bool) -> Self {
        self.css_pixel_snap = enabled;
        self
    }

    fn compute_layout(&self, avail_width: Pt) -> InlineLayoutCache {
        let forced = self.forced_line_height.unwrap_or(Pt::ZERO);
        let mut max_width = Pt::ZERO;
        let mut total_height = Pt::ZERO;
        let mut lines: Vec<InlineLineLayout> = Vec::new();

        let mut line_items: Vec<InlineItemLayout> = Vec::new();
        let mut line_width = Pt::ZERO;
        let mut line_height = forced;
        let css_pixel_snap = self.css_pixel_snap;

        let flush_line = |lines: &mut Vec<InlineLineLayout>,
                          line_items: &mut Vec<InlineItemLayout>,
                          line_width: Pt,
                          mut line_height: Pt,
                          max_width: &mut Pt,
                          total_height: &mut Pt| {
            if line_items.is_empty() {
                return;
            }
            let mut line_baseline: Option<Pt> = None;
            let mut max_descent = Pt::ZERO;
            for item in line_items.iter() {
                if !matches!(item.valign, VerticalAlign::Baseline) {
                    continue;
                }
                let item_baseline = item.baseline.unwrap_or(item.size.height);
                line_baseline = Some(
                    line_baseline
                        .map(|current| current.max(item_baseline))
                        .unwrap_or(item_baseline),
                );
                max_descent = max_descent.max((item.size.height - item_baseline).max(Pt::ZERO));
            }
            if let Some(baseline) = line_baseline {
                line_height = line_height.max(baseline + max_descent);
            }
            if css_pixel_snap {
                line_height = ceil_to_css_pixel(line_height);
            }
            *total_height = *total_height + line_height;
            *max_width = (*max_width).max(line_width);
            let items = std::mem::take(line_items);
            lines.push(InlineLineLayout {
                line_height,
                baseline: line_baseline,
                items,
            });
        };

        for (idx, (child, valign)) in self.children.iter().enumerate() {
            let size = child.wrap(avail_width, huge_pt());
            let next_width = if line_items.is_empty() {
                size.width
            } else {
                line_width + self.gap + size.width
            };
            if next_width > avail_width && !line_items.is_empty() {
                flush_line(
                    &mut lines,
                    &mut line_items,
                    line_width,
                    line_height,
                    &mut max_width,
                    &mut total_height,
                );
                line_width = Pt::ZERO;
                line_height = forced;
            }

            let x_off = if line_items.is_empty() {
                Pt::ZERO
            } else {
                line_width + self.gap
            };
            line_items.push(InlineItemLayout {
                idx,
                x_off,
                size,
                valign: *valign,
                baseline: if matches!(valign, VerticalAlign::Baseline) {
                    child.first_baseline(avail_width).or(Some(size.height))
                } else {
                    None
                },
            });
            line_width = x_off + size.width;
            line_height = line_height.max(size.height);
        }

        if !line_items.is_empty() {
            flush_line(
                &mut lines,
                &mut line_items,
                line_width,
                line_height,
                &mut max_width,
                &mut total_height,
            );
        }

        InlineLayoutCache {
            avail_width_milli: avail_width.to_milli_i64(),
            max_width,
            total_height,
            lines,
        }
    }

    fn cached_layout(&self, avail_width: Pt) -> InlineLayoutCache {
        let key = avail_width.to_milli_i64();
        if let Some(cache) = self.layout_cache.lock().unwrap().as_ref() {
            if cache.avail_width_milli == key {
                return cache.clone();
            }
        }
        let cache = self.compute_layout(avail_width);
        *self.layout_cache.lock().unwrap() = Some(cache.clone());
        cache
    }
}

impl Flowable for InlineBlockLayoutFlowable {
    fn wrap(&self, avail_width: Pt, _avail_height: Pt) -> Size {
        let perf = perf_start();
        let layout = self.cached_layout(avail_width);

        if perf_enabled() {
            log_perf_counts(
                "layout.inline.counts",
                &[("items", self.children.len() as u64)],
            );
        }
        perf_end("layout.inline.wrap", perf);
        Size {
            width: layout.max_width.min(avail_width),
            height: layout.total_height,
        }
    }

    fn intrinsic_width(&self) -> Option<Pt> {
        let mut total = Pt::ZERO;
        let mut seen = false;
        for (child, _) in &self.children {
            if child.out_of_flow() {
                continue;
            }
            let child_width = child.intrinsic_width()?;
            if seen {
                total = total + self.gap.max(Pt::ZERO);
            }
            total = total + child_width.max(Pt::ZERO);
            seen = true;
        }
        Some(total.max(Pt::ZERO))
    }

    fn first_baseline(&self, avail_width: Pt) -> Option<Pt> {
        self.cached_layout(avail_width)
            .lines
            .first()
            .and_then(|line| line.baseline)
    }

    fn split(
        &self,
        _avail_width: Pt,
        _avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        None
    }

    fn draw(&self, canvas: &mut Canvas, x: Pt, y: Pt, avail_width: Pt, avail_height: Pt) {
        let perf = perf_start();
        let layout = self.cached_layout(avail_width);
        let mut cursor_y = y;
        for line in &layout.lines {
            for item in &line.items {
                let y_off = match item.valign {
                    VerticalAlign::Baseline => line
                        .baseline
                        .zip(item.baseline)
                        .map(|(line_baseline, item_baseline)| line_baseline - item_baseline)
                        .unwrap_or(Pt::ZERO),
                    VerticalAlign::Top => Pt::ZERO,
                    VerticalAlign::Middle => (line.line_height - item.size.height).mul_ratio(1, 2),
                    VerticalAlign::Bottom => line.line_height - item.size.height,
                };
                let (child, _) = &self.children[item.idx];
                child.draw(
                    canvas,
                    x + item.x_off,
                    cursor_y + y_off,
                    item.size.width.min(avail_width),
                    item.size.height.min(avail_height),
                );
            }
            cursor_y = cursor_y + line.line_height;
        }
        perf_end("layout.inline.draw", perf);
    }

    fn pagination(&self) -> Pagination {
        self.pagination
    }
}

#[cfg(test)]
mod inline_baseline_tests {
    use super::{Canvas, Flowable, InlineBlockLayoutFlowable, Pagination, Pt, Size, VerticalAlign};

    #[derive(Clone)]
    struct BaselineProbe {
        size: Size,
        baseline: Pt,
    }

    impl Flowable for BaselineProbe {
        fn wrap(&self, _avail_width: Pt, _avail_height: Pt) -> Size {
            self.size
        }

        fn split(
            &self,
            _avail_width: Pt,
            _avail_height: Pt,
        ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
            None
        }

        fn draw(&self, _canvas: &mut Canvas, _x: Pt, _y: Pt, _avail_width: Pt, _avail_height: Pt) {}

        fn first_baseline(&self, _avail_width: Pt) -> Option<Pt> {
            Some(self.baseline)
        }

        fn pagination(&self) -> Pagination {
            Pagination::default()
        }
    }

    #[test]
    fn baseline_union_includes_parent_strut_descent() {
        let strut = BaselineProbe {
            size: Size {
                width: Pt::ZERO,
                height: Pt::from_f32(18.0),
            },
            baseline: Pt::from_f32(12.0),
        };
        let large_run = BaselineProbe {
            size: Size {
                width: Pt::from_f32(90.0),
                height: Pt::from_f32(30.0),
            },
            baseline: Pt::from_f32(25.0),
        };
        let line = InlineBlockLayoutFlowable::new_pt(
            vec![
                (Box::new(strut), VerticalAlign::Baseline),
                (Box::new(large_run), VerticalAlign::Baseline),
            ],
            Pt::ZERO,
            None,
        )
        .with_css_pixel_line_snap(true);

        assert_eq!(
            line.wrap(Pt::from_f32(200.0), Pt::from_f32(200.0)).height,
            Pt::from_f32(31.5)
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    Column,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JustifyContent {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignItems {
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignContent {
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Clone)]
pub struct FlexItem {
    child: Box<dyn Flowable>,
    grow: f32,
    #[allow(dead_code)]
    shrink: f32,
    basis: Option<LengthSpec>,
    align_self: Option<AlignItems>,
}

#[derive(Clone)]
struct FlexLineLayout {
    indices: Vec<usize>,
    widths: Vec<Pt>,
    child_avails: Vec<Pt>,
    sizes: Vec<Size>,
    line_h: Pt,
}

#[derive(Clone)]
enum FlexLayout {
    RowNoWrap {
        widths: Vec<Pt>,
        child_avails: Vec<Pt>,
        sizes: Vec<Size>,
        container_h: Pt,
    },
    RowWrap {
        lines: Vec<FlexLineLayout>,
        container_h: Pt,
    },
    Column {
        sizes: Vec<Size>,
        container_h: Pt,
    },
}

#[derive(Clone)]
struct FlexLayoutCache {
    avail_width_milli: i64,
    avail_height_milli: i64,
    lines_count: Option<usize>,
    layout: FlexLayout,
}

#[derive(Clone)]
pub struct FlexFlowable {
    items: Vec<FlexItem>,
    direction: FlexDirection,
    justify: JustifyContent,
    align: AlignItems,
    align_content: AlignContent,
    gap: LengthSpec,
    wrap: bool,
    line_item_limit: Option<usize>,
    row_tracks: Vec<GridTrackSize>,
    row_track_offset: usize,
    font_size: Pt,
    root_font_size: Pt,
    pagination: Pagination,
    layout_cache: Arc<Mutex<Option<FlexLayoutCache>>>,
}

impl FlexFlowable {
    pub fn new_pt(
        items: Vec<(
            Box<dyn Flowable>,
            f32,
            f32,
            Option<LengthSpec>,
            Option<AlignItems>,
        )>,
        direction: FlexDirection,
        justify: JustifyContent,
        align: AlignItems,
        align_content: AlignContent,
        gap: LengthSpec,
        wrap: bool,
        font_size: Pt,
        root_font_size: Pt,
    ) -> Self {
        Self {
            items: items
                .into_iter()
                .map(|(child, grow, shrink, basis, align_self)| FlexItem {
                    child,
                    grow: grow.max(0.0),
                    shrink: shrink.max(0.0),
                    basis,
                    align_self,
                })
                .collect(),
            direction,
            justify,
            align,
            align_content,
            gap,
            wrap,
            line_item_limit: None,
            row_tracks: Vec::new(),
            row_track_offset: 0,
            font_size,
            root_font_size,
            pagination: Pagination::default(),
            layout_cache: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_grid_tracks(mut self, column_count: usize, row_tracks: Vec<GridTrackSize>) -> Self {
        self.line_item_limit = Some(column_count.max(1));
        self.row_tracks = row_tracks;
        self.layout_cache = Arc::new(Mutex::new(None));
        self
    }

    fn bounded_height(avail_height: Pt) -> Option<Pt> {
        if avail_height > Pt::ZERO && avail_height < huge_pt() {
            Some(avail_height)
        } else {
            None
        }
    }

    fn resolved_gap(&self, avail_width: Pt) -> Pt {
        self.gap
            .resolve_width(avail_width, self.font_size, self.root_font_size)
            .max(Pt::ZERO)
    }

    fn row_track(&self, line_index: usize) -> Option<GridTrackSize> {
        self.row_tracks
            .get(self.row_track_offset.saturating_add(line_index))
            .copied()
    }

    fn fixed_row_track_height(&self, line_index: usize, avail_height: Pt) -> Option<Pt> {
        self.row_track(line_index)
            .and_then(GridTrackSize::fixed_breadth)
            .map(|spec| {
                spec.resolve_height(avail_height, self.font_size, self.root_font_size)
                    .max(Pt::ZERO)
            })
    }

    fn with_items(&self, items: Vec<FlexItem>, first: bool) -> FlexFlowable {
        let pagination = if first {
            Pagination {
                break_before: BreakBefore::Auto,
                break_after: BreakAfter::Auto,
                ..self.pagination
            }
        } else {
            Pagination {
                break_before: BreakBefore::Auto,
                ..self.pagination
            }
        };

        FlexFlowable {
            items,
            direction: self.direction,
            justify: self.justify,
            align: self.align,
            align_content: self.align_content,
            gap: self.gap,
            wrap: self.wrap,
            line_item_limit: self.line_item_limit,
            row_tracks: self.row_tracks.clone(),
            row_track_offset: self.row_track_offset,
            font_size: self.font_size,
            root_font_size: self.root_font_size,
            pagination,
            layout_cache: Arc::new(Mutex::new(None)),
        }
    }

    fn compute_layout(&self, avail_width: Pt, avail_height: Pt) -> FlexLayoutCache {
        let n = self.items.len();
        let gap = self.resolved_gap(avail_width);
        let (layout, lines_count) = match self.direction {
            FlexDirection::Row => {
                if !self.wrap {
                    let gap_total = gap * (n.saturating_sub(1) as i32);
                    let available = (avail_width - gap_total).max(Pt::ZERO);
                    let mut widths = vec![Pt::ZERO; n];
                    let mut child_avails = vec![Pt::ZERO; n];
                    let mut sizes: Vec<Option<Size>> = vec![None; n];
                    let mut fixed_total = Pt::ZERO;
                    let mut flex_indices: Vec<usize> = Vec::new();
                    let mut total_grow: f32 = 0.0;

                    for (idx, item) in self.items.iter().enumerate() {
                        let basis = item.basis.and_then(|spec| match spec {
                            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial => None,
                            _ => Some(
                                spec.resolve_width(
                                    avail_width,
                                    self.font_size,
                                    self.root_font_size,
                                )
                                .max(Pt::ZERO),
                            ),
                        });
                        if item.grow <= 0.0 {
                            if let Some(basis) = basis {
                                let child_avail = basis;
                                let size = item.child.wrap(child_avail, avail_height);
                                widths[idx] = basis;
                                child_avails[idx] = child_avail;
                                sizes[idx] = Some(size);
                                fixed_total = fixed_total + basis;
                                continue;
                            } else {
                                let intrinsic = item.child.intrinsic_width().unwrap_or_else(|| {
                                    item.child.wrap(avail_width, avail_height).width
                                });
                                let child_avail = intrinsic.min(avail_width).max(Pt::ZERO);
                                let size = item.child.wrap(child_avail, avail_height);
                                widths[idx] = child_avail;
                                child_avails[idx] = child_avail;
                                sizes[idx] = Some(size);
                                fixed_total = fixed_total + child_avail;
                                continue;
                            }
                        }
                        flex_indices.push(idx);
                        total_grow += item.grow;
                    }

                    let remaining = (available - fixed_total).max(Pt::ZERO);
                    for idx in &flex_indices {
                        let item = &self.items[*idx];
                        let w = if total_grow > 0.0 {
                            remaining * (item.grow / total_grow)
                        } else if !flex_indices.is_empty() {
                            remaining / (flex_indices.len() as i32)
                        } else {
                            Pt::ZERO
                        };
                        let w = w.max(Pt::ZERO);
                        widths[*idx] = w;
                        child_avails[*idx] = w;
                    }

                    let mut max_h = Pt::ZERO;
                    let mut final_sizes: Vec<Size> = Vec::with_capacity(n);
                    for (idx, item) in self.items.iter().enumerate() {
                        let size = if let Some(size) = sizes[idx] {
                            size
                        } else {
                            let size = item.child.wrap(child_avails[idx], avail_height);
                            sizes[idx] = Some(size);
                            size
                        };
                        max_h = max_h.max(size.height);
                        final_sizes.push(size);
                    }

                    let container_h = Self::bounded_height(avail_height).unwrap_or(max_h);
                    (
                        FlexLayout::RowNoWrap {
                            widths,
                            child_avails,
                            sizes: final_sizes,
                            container_h,
                        },
                        Some(1),
                    )
                } else {
                    let lines = self.row_lines(avail_width, avail_height);
                    let mut line_layouts: Vec<FlexLineLayout> = Vec::new();
                    for (line_index, line) in lines.iter().enumerate() {
                        let (widths, child_avails, sizes, intrinsic_line_h) =
                            self.row_line_layout(line, avail_width, avail_height);
                        let line_h = self
                            .fixed_row_track_height(line_index, avail_height)
                            .unwrap_or(intrinsic_line_h);
                        line_layouts.push(FlexLineLayout {
                            indices: line.clone(),
                            widths,
                            child_avails,
                            sizes,
                            line_h,
                        });
                    }
                    let mut total_h = line_layouts
                        .iter()
                        .fold(Pt::ZERO, |acc, line| acc + line.line_h);
                    let bounded_height = Self::bounded_height(avail_height);
                    if let Some(target_height) = bounded_height
                        .filter(|height| !line_layouts.is_empty() && *height > total_h)
                    {
                        let fraction_total: f32 = line_layouts
                            .iter()
                            .enumerate()
                            .filter_map(|(index, _)| {
                                self.row_track(index)
                                    .and_then(GridTrackSize::fraction_factor)
                            })
                            .sum();
                        if fraction_total > 0.0 {
                            let distributable = target_height - total_h;
                            let mut assigned = Pt::ZERO;
                            let fraction_indices: Vec<(usize, f32)> = line_layouts
                                .iter()
                                .enumerate()
                                .filter_map(|(index, _)| {
                                    self.row_track(index)
                                        .and_then(GridTrackSize::fraction_factor)
                                        .map(|factor| (index, factor))
                                })
                                .collect();
                            for (position, (index, factor)) in
                                fraction_indices.iter().copied().enumerate()
                            {
                                let extra = if position + 1 == fraction_indices.len() {
                                    distributable - assigned
                                } else {
                                    distributable * (factor / fraction_total)
                                };
                                line_layouts[index].line_h = line_layouts[index].line_h + extra;
                                assigned = assigned + extra;
                            }
                            total_h = target_height;
                        } else if matches!(self.align_content, AlignContent::Stretch) {
                            let stretch_indices: Vec<usize> = if self.row_tracks.is_empty() {
                                (0..line_layouts.len()).collect()
                            } else {
                                line_layouts
                                    .iter()
                                    .enumerate()
                                    .filter_map(|(index, _)| {
                                        matches!(
                                            self.row_track(index).map(|track| track.max),
                                            Some(GridTrackBreadth::Auto) | None
                                        )
                                        .then_some(index)
                                    })
                                    .collect()
                            };
                            if !stretch_indices.is_empty() {
                                let distributable = target_height - total_h;
                                let share = distributable / (stretch_indices.len() as i32);
                                let mut assigned = Pt::ZERO;
                                for (position, index) in stretch_indices.iter().copied().enumerate()
                                {
                                    let extra = if position + 1 == stretch_indices.len() {
                                        distributable - assigned
                                    } else {
                                        share
                                    };
                                    line_layouts[index].line_h = line_layouts[index].line_h + extra;
                                    assigned = assigned + extra;
                                }
                                total_h = target_height;
                            }
                        }
                    }
                    let container_h = bounded_height.unwrap_or(total_h);
                    (
                        FlexLayout::RowWrap {
                            lines: line_layouts,
                            container_h,
                        },
                        Some(lines.len()),
                    )
                }
            }
            FlexDirection::Column => {
                let mut total_h = Pt::ZERO;
                let mut sizes: Vec<Size> = Vec::with_capacity(n);
                for item in &self.items {
                    let size = item.child.wrap(avail_width, avail_height);
                    total_h = total_h + size.height;
                    sizes.push(size);
                }
                total_h = total_h + gap * (n.saturating_sub(1) as i32);
                let container_h = Self::bounded_height(avail_height).unwrap_or(total_h);
                (FlexLayout::Column { sizes, container_h }, None)
            }
        };

        FlexLayoutCache {
            avail_width_milli: avail_width.to_milli_i64(),
            avail_height_milli: avail_height.to_milli_i64(),
            lines_count,
            layout,
        }
    }

    fn cached_layout(&self, avail_width: Pt, avail_height: Pt) -> FlexLayoutCache {
        let key_w = avail_width.to_milli_i64();
        let key_h = avail_height.to_milli_i64();
        if let Some(cache) = self.layout_cache.lock().unwrap().as_ref() {
            if cache.avail_width_milli == key_w && cache.avail_height_milli == key_h {
                return cache.clone();
            }
        }
        let cache = self.compute_layout(avail_width, avail_height);
        *self.layout_cache.lock().unwrap() = Some(cache.clone());
        cache
    }

    fn split_column(
        &self,
        avail_width: Pt,
        avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        let n = self.items.len();
        if n == 0 {
            return None;
        }
        let gap = self.resolved_gap(avail_width);
        let mut remaining_height = avail_height;
        let mut placed: Vec<FlexItem> = Vec::new();
        let mut remaining: Vec<FlexItem> = Vec::new();

        for (idx, item) in self.items.iter().cloned().enumerate() {
            let size = item.child.wrap(avail_width, remaining_height);
            if size.height <= remaining_height {
                placed.push(item);
                remaining_height = (remaining_height - size.height).max(Pt::ZERO);
                if idx + 1 < n {
                    remaining_height = (remaining_height - gap).max(Pt::ZERO);
                }
                continue;
            }

            if let Some((first, second)) = item.child.split(avail_width, remaining_height) {
                placed.push(FlexItem {
                    child: first,
                    ..item
                });
                remaining.push(FlexItem {
                    child: second,
                    ..item
                });
                for rest in self.items[idx + 1..].iter().cloned() {
                    remaining.push(rest);
                }
                break;
            } else {
                remaining.push(item);
                for rest in self.items[idx + 1..].iter().cloned() {
                    remaining.push(rest);
                }
                break;
            }
        }

        if placed.is_empty() || remaining.is_empty() {
            return None;
        }

        let first = self.with_items(placed, true);
        let second = self.with_items(remaining, false);
        Some((Box::new(first), Box::new(second)))
    }

    fn split_single_row_item(
        &self,
        avail_width: Pt,
        avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        if self.items.len() != 1 {
            return None;
        }
        let item = self.items[0].clone();
        if let Some((first, second)) = item.child.split(avail_width, avail_height) {
            let first_item = FlexItem {
                child: first,
                ..item.clone()
            };
            let second_item = FlexItem {
                child: second,
                ..item
            };
            let first = self.with_items(vec![first_item], true);
            let second = self.with_items(vec![second_item], false);
            return Some((Box::new(first), Box::new(second)));
        }
        None
    }

    fn split_row_wrapped(
        &self,
        avail_width: Pt,
        avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        let n = self.items.len();
        if n == 0 {
            return None;
        }

        let lines = self.row_lines(avail_width, huge_pt());
        if lines.is_empty() {
            return None;
        }

        let gap = self.resolved_gap(avail_width);
        let mut remaining_height = avail_height;
        let mut split_at: Option<usize> = None;
        let mut any_line = false;

        for (line_idx, line) in lines.iter().enumerate() {
            let (_, _, _, line_h) = self.row_line_layout(line, avail_width, huge_pt());
            if line_h <= remaining_height {
                any_line = true;
                remaining_height = (remaining_height - line_h).max(Pt::ZERO);
                if line_idx + 1 < lines.len() {
                    remaining_height = (remaining_height - gap).max(Pt::ZERO);
                }
                if let Some(last) = line.last() {
                    split_at = Some(last + 1);
                }
                continue;
            }

            if !any_line && line.len() == 1 {
                let item_idx = line[0];
                let item = self.items[item_idx].clone();
                if let Some((first, second)) = item.child.split(avail_width, remaining_height) {
                    let mut placed: Vec<FlexItem> = Vec::new();
                    let mut remaining: Vec<FlexItem> = Vec::new();
                    placed.push(FlexItem {
                        child: first,
                        ..item.clone()
                    });
                    remaining.push(FlexItem {
                        child: second,
                        ..item
                    });
                    for rest in self.items[item_idx + 1..].iter().cloned() {
                        remaining.push(rest);
                    }
                    let first = self.with_items(placed, true);
                    let second = self.with_items(remaining, false);
                    return Some((Box::new(first), Box::new(second)));
                }
            }
            break;
        }

        let split_at = split_at?;
        if split_at >= self.items.len() {
            return None;
        }

        let placed = self.items[..split_at].to_vec();
        let remaining = self.items[split_at..].to_vec();
        let first = self.with_items(placed, true);
        let second = self.with_items(remaining, false);
        Some((Box::new(first), Box::new(second)))
    }

    fn row_lines(&self, avail_width: Pt, avail_height: Pt) -> Vec<Vec<usize>> {
        let n = self.items.len();
        if n == 0 {
            return Vec::new();
        }
        if !self.wrap {
            return vec![(0..n).collect()];
        }
        if let Some(limit) = self.line_item_limit {
            let limit = limit.max(1);
            return (0..n)
                .collect::<Vec<_>>()
                .chunks(limit)
                .map(|chunk| chunk.to_vec())
                .collect();
        }
        let gap = self.resolved_gap(avail_width);
        let mut lines: Vec<Vec<usize>> = Vec::new();
        let mut current: Vec<usize> = Vec::new();
        let mut used = Pt::ZERO;

        for idx in 0..n {
            let item = &self.items[idx];
            let basis = item.basis.and_then(|spec| match spec {
                LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial => None,
                _ => Some(
                    spec.resolve_width(avail_width, self.font_size, self.root_font_size)
                        .max(Pt::ZERO),
                ),
            });
            let min_w = if let Some(basis) = basis {
                basis
            } else if item.grow <= 0.0 {
                let intrinsic = item
                    .child
                    .intrinsic_width()
                    .unwrap_or_else(|| item.child.wrap(avail_width, avail_height).width);
                intrinsic.min(avail_width)
            } else {
                Pt::ZERO
            };

            let extra_gap = if current.is_empty() { Pt::ZERO } else { gap };
            if !current.is_empty() && used + extra_gap + min_w > avail_width {
                lines.push(current);
                current = Vec::new();
                used = Pt::ZERO;
            }
            if !current.is_empty() {
                used = used + gap;
            }
            current.push(idx);
            used = used + min_w;
        }
        if !current.is_empty() {
            lines.push(current);
        }
        lines
    }

    fn row_line_layout(
        &self,
        indices: &[usize],
        avail_width: Pt,
        avail_height: Pt,
    ) -> (Vec<Pt>, Vec<Pt>, Vec<Size>, Pt) {
        let n = indices.len();
        let mut widths = vec![Pt::ZERO; n];
        let mut child_avails = vec![Pt::ZERO; n];
        let mut sizes: Vec<Option<Size>> = vec![None; n];
        let mut flex_basis = vec![Pt::ZERO; n];
        let mut fixed_total = Pt::ZERO;
        let mut flex_indices: Vec<usize> = Vec::new();
        let mut total_grow: f32 = 0.0;

        for (pos, idx) in indices.iter().enumerate() {
            let item = &self.items[*idx];
            let basis = item.basis.and_then(|spec| match spec {
                LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial => None,
                _ => Some(
                    spec.resolve_width(avail_width, self.font_size, self.root_font_size)
                        .max(Pt::ZERO),
                ),
            });
            if item.grow <= 0.0 {
                if let Some(basis) = basis {
                    let child_avail = basis;
                    let size = item.child.wrap(child_avail, avail_height);
                    widths[pos] = basis;
                    child_avails[pos] = child_avail;
                    sizes[pos] = Some(size);
                    fixed_total = fixed_total + basis;
                } else {
                    let intrinsic = item
                        .child
                        .intrinsic_width()
                        .unwrap_or_else(|| item.child.wrap(avail_width, avail_height).width);
                    let child_avail = intrinsic.min(avail_width).max(Pt::ZERO);
                    let size = item.child.wrap(child_avail, avail_height);
                    widths[pos] = child_avail;
                    child_avails[pos] = child_avail;
                    sizes[pos] = Some(size);
                    fixed_total = fixed_total + child_avail;
                }
                continue;
            }
            if let Some(basis) = basis {
                fixed_total = fixed_total + basis;
                flex_basis[pos] = basis;
            }
            flex_indices.push(pos);
            total_grow += item.grow;
        }

        let gap = self.resolved_gap(avail_width);
        let gap_total = gap * (n.saturating_sub(1) as i32);
        let available = (avail_width - gap_total).max(Pt::ZERO);
        let remaining = (available - fixed_total).max(Pt::ZERO);

        for pos in &flex_indices {
            let item = &self.items[indices[*pos]];
            let w = if total_grow > 0.0 {
                remaining * (item.grow / total_grow)
            } else if !flex_indices.is_empty() {
                remaining / (flex_indices.len() as i32)
            } else {
                Pt::ZERO
            };
            let w = w.max(Pt::ZERO);
            let total_w = w + flex_basis[*pos];
            widths[*pos] = total_w;
            child_avails[*pos] = total_w;
        }

        let mut max_h = Pt::ZERO;
        let mut final_sizes: Vec<Size> = Vec::with_capacity(n);
        for (pos, idx) in indices.iter().enumerate() {
            let size = if let Some(size) = sizes[pos] {
                size
            } else {
                let size = self.items[*idx].child.wrap(child_avails[pos], avail_height);
                size
            };
            max_h = max_h.max(size.height);
            final_sizes.push(size);
        }

        (widths, child_avails, final_sizes, max_h)
    }
}

impl Flowable for FlexFlowable {
    fn wrap(&self, avail_width: Pt, avail_height: Pt) -> Size {
        let perf = perf_start();
        let n = self.items.len();
        if n == 0 {
            perf_end("layout.flex.wrap", perf);
            return Size {
                width: Pt::ZERO,
                height: Pt::ZERO,
            };
        }
        let layout = self.cached_layout(avail_width, avail_height);
        let size = match &layout.layout {
            FlexLayout::RowNoWrap { container_h, .. } => Size {
                width: avail_width,
                height: *container_h,
            },
            FlexLayout::RowWrap { container_h, .. } => Size {
                width: avail_width,
                height: *container_h,
            },
            FlexLayout::Column { container_h, .. } => Size {
                width: avail_width,
                height: *container_h,
            },
        };

        if perf_enabled() {
            let mut counts: Vec<(&str, u64)> = Vec::new();
            counts.push(("items", n as u64));
            if let Some(lines) = layout.lines_count {
                counts.push(("lines", lines as u64));
            }
            log_perf_counts("layout.flex.counts", &counts);
        }
        perf_end("layout.flex.wrap", perf);
        size
    }

    fn split(
        &self,
        _avail_width: Pt,
        _avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        let avail_width = _avail_width;
        let avail_height = _avail_height;
        if avail_height <= Pt::ZERO {
            return None;
        }

        match self.direction {
            FlexDirection::Column => self.split_column(avail_width, avail_height),
            FlexDirection::Row => {
                if self.wrap {
                    self.split_row_wrapped(avail_width, avail_height)
                } else {
                    self.split_single_row_item(avail_width, avail_height)
                }
            }
        }
    }

    fn draw(&self, canvas: &mut Canvas, x: Pt, y: Pt, avail_width: Pt, avail_height: Pt) {
        let perf = perf_start();
        let n = self.items.len();
        if n == 0 {
            perf_end("layout.flex.draw", perf);
            return;
        }

        let layout = self.cached_layout(avail_width, avail_height);
        let gap_base = self.resolved_gap(avail_width);

        match &layout.layout {
            FlexLayout::RowNoWrap {
                widths,
                child_avails,
                sizes,
                container_h,
            } => {
                let used_w: Pt = widths.iter().fold(Pt::ZERO, |acc, w| acc + *w)
                    + gap_base * (n.saturating_sub(1) as i32);
                let extra = (avail_width - used_w).max(Pt::ZERO);
                let mut gap = gap_base;
                let mut start_x = Pt::ZERO;
                let total_grow: f32 = self.items.iter().map(|item| item.grow).sum();
                match self.justify {
                    JustifyContent::Center => start_x = extra.mul_ratio(1, 2),
                    JustifyContent::FlexEnd => start_x = extra,
                    JustifyContent::SpaceBetween if n > 1 && total_grow == 0.0 => {
                        gap = gap_base + (extra / ((n as i32) - 1));
                    }
                    JustifyContent::SpaceAround if n > 0 && total_grow == 0.0 => {
                        let share = extra / (n as i32);
                        start_x = share.mul_ratio(1, 2);
                        gap = gap_base + share;
                    }
                    JustifyContent::SpaceEvenly if n > 0 && total_grow == 0.0 => {
                        let share = extra / ((n as i32) + 1);
                        start_x = share;
                        gap = gap_base + share;
                    }
                    _ => {}
                }

                let mut cursor_x = x + start_x;
                for (idx, item) in self.items.iter().enumerate() {
                    let size = sizes[idx];
                    let item_align = item.align_self.unwrap_or(self.align);
                    let y_off = match item_align {
                        AlignItems::Center => (*container_h - size.height).mul_ratio(1, 2),
                        AlignItems::FlexEnd => *container_h - size.height,
                        _ => Pt::ZERO,
                    }
                    .max(Pt::ZERO);

                    if matches!(item_align, AlignItems::Stretch) {
                        item.child.draw_stretched(
                            canvas,
                            cursor_x,
                            y + y_off,
                            child_avails[idx],
                            *container_h,
                        );
                    } else {
                        item.child.draw(
                            canvas,
                            cursor_x,
                            y + y_off,
                            child_avails[idx],
                            *container_h,
                        );
                    }
                    cursor_x = cursor_x + widths[idx];
                    if idx + 1 < n {
                        cursor_x = cursor_x + gap;
                    }
                }
            }
            FlexLayout::RowWrap { lines, container_h } => {
                let total_lines_h: Pt = lines
                    .iter()
                    .fold(Pt::ZERO, |acc, line| acc + line.line_h.max(Pt::ZERO));
                let extra_cross = (*container_h - total_lines_h).max(Pt::ZERO);
                let line_count = lines.len();
                let mut start_y = Pt::ZERO;
                let mut line_gap = Pt::ZERO;
                match self.align_content {
                    AlignContent::Center => {
                        start_y = extra_cross.mul_ratio(1, 2);
                    }
                    AlignContent::FlexEnd => {
                        start_y = extra_cross;
                    }
                    AlignContent::SpaceBetween if line_count > 1 => {
                        line_gap = extra_cross / ((line_count as i32) - 1);
                    }
                    AlignContent::SpaceAround if line_count > 0 => {
                        line_gap = extra_cross / (line_count as i32);
                        start_y = line_gap.mul_ratio(1, 2);
                    }
                    AlignContent::SpaceEvenly if line_count > 0 => {
                        line_gap = extra_cross / ((line_count as i32) + 1);
                        start_y = line_gap;
                    }
                    _ => {}
                }

                let mut cursor_y = y + start_y;
                for (line_idx, line) in lines.iter().enumerate() {
                    let used_w: Pt = line.widths.iter().fold(Pt::ZERO, |acc, w| acc + *w)
                        + gap_base * (line.indices.len().saturating_sub(1) as i32);
                    let extra = (avail_width - used_w).max(Pt::ZERO);
                    let mut gap = gap_base;
                    let mut start_x = Pt::ZERO;
                    match self.justify {
                        JustifyContent::Center => start_x = extra.mul_ratio(1, 2),
                        JustifyContent::FlexEnd => start_x = extra,
                        JustifyContent::SpaceBetween if line.indices.len() > 1 => {
                            gap = gap_base + (extra / ((line.indices.len() as i32) - 1));
                        }
                        JustifyContent::SpaceAround if !line.indices.is_empty() => {
                            let share = extra / (line.indices.len() as i32);
                            start_x = share.mul_ratio(1, 2);
                            gap = gap_base + share;
                        }
                        JustifyContent::SpaceEvenly if !line.indices.is_empty() => {
                            let share = extra / ((line.indices.len() as i32) + 1);
                            start_x = share;
                            gap = gap_base + share;
                        }
                        _ => {}
                    }

                    let mut cursor_x = x + start_x;
                    for (pos, idx) in line.indices.iter().enumerate() {
                        let size = line.sizes[pos];
                        let item_align = self.items[*idx].align_self.unwrap_or(self.align);
                        let y_off = match item_align {
                            AlignItems::Center => (line.line_h - size.height).mul_ratio(1, 2),
                            AlignItems::FlexEnd => line.line_h - size.height,
                            _ => Pt::ZERO,
                        }
                        .max(Pt::ZERO);

                        if matches!(item_align, AlignItems::Stretch) {
                            self.items[*idx].child.draw_stretched(
                                canvas,
                                cursor_x,
                                cursor_y + y_off,
                                line.child_avails[pos],
                                line.line_h,
                            );
                        } else {
                            self.items[*idx].child.draw(
                                canvas,
                                cursor_x,
                                cursor_y + y_off,
                                line.child_avails[pos],
                                line.line_h,
                            );
                        }
                        cursor_x = cursor_x + line.widths[pos];
                        if pos + 1 < line.indices.len() {
                            cursor_x = cursor_x + gap;
                        }
                    }
                    cursor_y = cursor_y + line.line_h;
                    if line_idx + 1 < line_count {
                        cursor_y = cursor_y + line_gap;
                    }
                }
            }
            FlexLayout::Column { sizes, container_h } => {
                let used_h = sizes.iter().fold(Pt::ZERO, |acc, size| acc + size.height)
                    + gap_base * (n.saturating_sub(1) as i32);
                let extra = (*container_h - used_h).max(Pt::ZERO);

                let mut gap = gap_base;
                let mut start_y = Pt::ZERO;
                match self.justify {
                    JustifyContent::Center => start_y = extra.mul_ratio(1, 2),
                    JustifyContent::FlexEnd => start_y = extra,
                    JustifyContent::SpaceBetween if n > 1 => {
                        gap = gap_base + (extra / ((n as i32) - 1));
                    }
                    JustifyContent::SpaceAround if n > 0 => {
                        let share = extra / (n as i32);
                        start_y = share.mul_ratio(1, 2);
                        gap = gap_base + share;
                    }
                    JustifyContent::SpaceEvenly if n > 0 => {
                        let share = extra / ((n as i32) + 1);
                        start_y = share;
                        gap = gap_base + share;
                    }
                    _ => {}
                }

                let mut cursor_y = y + start_y;
                for (idx, item) in self.items.iter().enumerate() {
                    let size = sizes[idx];
                    let item_align = item.align_self.unwrap_or(self.align);
                    let x_off = match item_align {
                        AlignItems::Center => (avail_width - size.width).mul_ratio(1, 2),
                        AlignItems::FlexEnd => avail_width - size.width,
                        _ => Pt::ZERO,
                    }
                    .max(Pt::ZERO);

                    item.child
                        .draw(canvas, x + x_off, cursor_y, avail_width, size.height);
                    cursor_y = cursor_y + size.height;
                    if idx + 1 < n {
                        cursor_y = cursor_y + gap;
                    }
                }
            }
        }
        perf_end("layout.flex.draw", perf);
    }

    fn pagination(&self) -> Pagination {
        self.pagination
    }
}

#[derive(Clone)]
struct ContainerLayoutCache {
    avail_width_milli: i64,
    avail_height_milli: i64,
    margin: ResolvedEdges,
    border: ResolvedEdges,
    padding: ResolvedEdges,
    content_width: Pt,
    border_box_width: Pt,
    #[allow(dead_code)]
    content_height: Pt,
    border_box_height: Pt,
    total_width: Pt,
    total_height: Pt,
    child_avail_height: Pt,
    child_sizes: Vec<Option<Size>>,
}

#[derive(Clone)]
pub struct ContainerFlowable {
    children: Vec<Box<dyn Flowable>>,
    margin: EdgeSizes,
    border_width: EdgeSizes,
    border_colors: ResolvedEdgeColors,
    border_styles: ResolvedEdgeStyles,
    border_radius: BorderRadiiSpec,
    outline_width: LengthSpec,
    outline_offset: LengthSpec,
    outline_style: OutlineLineStyle,
    outline_color: Color,
    outline_visible: bool,
    padding: EdgeSizes,
    width: LengthSpec,
    max_width: LengthSpec,
    min_width: LengthSpec,
    height: LengthSpec,
    min_height: LengthSpec,
    max_height: LengthSpec,
    box_sizing: BoxSizingMode,
    background: Option<Color>,
    background_paint: Option<BackgroundPaint>,
    background_paints: Vec<BackgroundPaint>,
    background_sizes: Vec<BackgroundSizeSpec>,
    background_positions: Vec<BackgroundPositionSpec>,
    background_repeats: Vec<BackgroundRepeatSpec>,
    background_blend_modes: Vec<MixBlendMode>,
    background_origins: Vec<BackgroundBox>,
    background_clips: Vec<BackgroundClipBox>,
    clip_path: Option<ClipPathShapeSpec>,
    clip_path_reference_box: ClipPathReferenceBox,
    clip_path_backdrop_root_group_suppressed: bool,
    will_change_backdrop_root: bool,
    will_change_backdrop_root_group_suppressed: bool,
    mask_backdrop_root: bool,
    mask_backdrop_root_group_suppressed: bool,
    box_shadow: Option<BoxShadowSpec>,
    box_shadows: Vec<BoxShadowSpec>,
    paint_filter: Option<PaintFilterSpec>,
    backdrop_filter: Option<PaintFilterSpec>,
    mix_blend_mode: MixBlendMode,
    isolation: bool,
    opacity: f32,
    transforms: Vec<CssTransformOp>,
    transform_origin: CssTransformOrigin,
    overflow_hidden: bool,
    self_visible: bool,
    tag_role: Option<Arc<str>>,
    establishes_abs_containing_block: bool,
    font_size: Pt,
    root_font_size: Pt,
    pagination: Pagination,
    layout_cache: Arc<Mutex<Option<ContainerLayoutCache>>>,
}

impl ContainerFlowable {
    pub fn new(children: Vec<Box<dyn Flowable>>, font_size: f32, root_font_size: f32) -> Self {
        Self::new_pt(
            children,
            Pt::from_f32(font_size),
            Pt::from_f32(root_font_size),
        )
    }

    pub fn new_pt(children: Vec<Box<dyn Flowable>>, font_size: Pt, root_font_size: Pt) -> Self {
        Self {
            children,
            margin: EdgeSizes::zero(),
            border_width: EdgeSizes::zero(),
            border_colors: ResolvedEdgeColors::uniform(Color::BLACK),
            border_styles: ResolvedEdgeStyles::uniform(OutlineLineStyle::Solid),
            border_radius: BorderRadiiSpec::zero(),
            outline_width: LengthSpec::Absolute(Pt::ZERO),
            outline_offset: LengthSpec::Absolute(Pt::ZERO),
            outline_style: OutlineLineStyle::Solid,
            outline_color: Color::BLACK,
            outline_visible: false,
            padding: EdgeSizes::zero(),
            width: LengthSpec::Auto,
            max_width: LengthSpec::Auto,
            min_width: LengthSpec::Auto,
            height: LengthSpec::Auto,
            min_height: LengthSpec::Auto,
            max_height: LengthSpec::Auto,
            box_sizing: BoxSizingMode::ContentBox,
            background: None,
            background_paint: None,
            background_paints: Vec::new(),
            background_sizes: Vec::new(),
            background_positions: Vec::new(),
            background_repeats: Vec::new(),
            background_blend_modes: Vec::new(),
            background_origins: Vec::new(),
            background_clips: Vec::new(),
            clip_path: None,
            clip_path_reference_box: ClipPathReferenceBox::Border,
            clip_path_backdrop_root_group_suppressed: false,
            will_change_backdrop_root: false,
            will_change_backdrop_root_group_suppressed: false,
            mask_backdrop_root: false,
            mask_backdrop_root_group_suppressed: false,
            box_shadow: None,
            box_shadows: Vec::new(),
            paint_filter: None,
            backdrop_filter: None,
            mix_blend_mode: MixBlendMode::Normal,
            isolation: false,
            opacity: 1.0,
            transforms: Vec::new(),
            transform_origin: CssTransformOrigin::center(),
            overflow_hidden: false,
            self_visible: true,
            tag_role: None,
            establishes_abs_containing_block: false,
            font_size,
            root_font_size,
            pagination: Pagination::default(),
            layout_cache: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_margin(mut self, margin: EdgeSizes) -> Self {
        self.margin = margin;
        self
    }

    pub fn with_border(mut self, border_width: EdgeSizes, border_color: Color) -> Self {
        self.border_width = border_width;
        self.border_colors = ResolvedEdgeColors::uniform(border_color);
        self
    }

    pub fn with_border_colors(
        mut self,
        top: Color,
        right: Color,
        bottom: Color,
        left: Color,
    ) -> Self {
        self.border_colors = ResolvedEdgeColors {
            top,
            right,
            bottom,
            left,
        };
        self
    }

    pub fn with_border_styles(
        mut self,
        top: OutlineLineStyle,
        right: OutlineLineStyle,
        bottom: OutlineLineStyle,
        left: OutlineLineStyle,
    ) -> Self {
        self.border_styles = ResolvedEdgeStyles {
            top,
            right,
            bottom,
            left,
        };
        self
    }

    pub fn with_border_radius(mut self, radius: BorderRadiiSpec) -> Self {
        self.border_radius = radius;
        self
    }

    pub fn with_outline(
        mut self,
        width: LengthSpec,
        offset: LengthSpec,
        style: OutlineLineStyle,
        color: Color,
        visible: bool,
    ) -> Self {
        self.outline_width = width;
        self.outline_offset = offset;
        self.outline_style = style;
        self.outline_color = color;
        self.outline_visible = visible;
        self
    }

    pub fn with_padding(mut self, padding: EdgeSizes) -> Self {
        self.padding = padding;
        self
    }

    pub fn with_width(mut self, width: LengthSpec) -> Self {
        self.width = width;
        self
    }

    pub fn with_max_width(mut self, max_width: LengthSpec) -> Self {
        self.max_width = max_width;
        self
    }

    pub fn with_min_width(mut self, min_width: LengthSpec) -> Self {
        self.min_width = min_width;
        self
    }

    pub fn with_height(mut self, height: LengthSpec) -> Self {
        self.height = height;
        self
    }

    pub fn with_min_height(mut self, min_height: LengthSpec) -> Self {
        self.min_height = min_height;
        self
    }

    pub fn with_max_height(mut self, max_height: LengthSpec) -> Self {
        self.max_height = max_height;
        self
    }

    pub fn with_box_sizing(mut self, box_sizing: BoxSizingMode) -> Self {
        self.box_sizing = box_sizing;
        self
    }

    pub fn with_background(mut self, color: Option<Color>) -> Self {
        self.background = color;
        self
    }

    pub fn with_background_paint(mut self, paint: Option<BackgroundPaint>) -> Self {
        self.background_paints = paint.iter().cloned().collect();
        self.background_paint = paint;
        self
    }

    pub fn with_background_layers(
        mut self,
        paints: Vec<BackgroundPaint>,
        sizes: Vec<BackgroundSizeSpec>,
        positions: Vec<BackgroundPositionSpec>,
        repeats: Vec<BackgroundRepeatSpec>,
        origins: Vec<BackgroundBox>,
        clips: Vec<BackgroundClipBox>,
    ) -> Self {
        self.background_paint = paints.first().cloned();
        self.background_paints = paints;
        self.background_sizes = sizes;
        self.background_positions = positions;
        self.background_repeats = repeats;
        self.background_origins = origins;
        self.background_clips = clips;
        self
    }

    pub fn with_background_blend_modes(mut self, modes: Vec<MixBlendMode>) -> Self {
        self.background_blend_modes = modes;
        self
    }

    pub fn with_clip_path(mut self, clip_path: Option<ClipPathShapeSpec>) -> Self {
        self.clip_path = clip_path;
        self
    }

    pub fn with_clip_path_reference_box(mut self, reference_box: ClipPathReferenceBox) -> Self {
        self.clip_path_reference_box = reference_box;
        self
    }

    pub fn with_will_change_backdrop_root(mut self, root: bool) -> Self {
        self.will_change_backdrop_root = root;
        self
    }

    pub fn with_mask_backdrop_root(mut self, root: bool) -> Self {
        self.mask_backdrop_root = root;
        self
    }

    pub fn with_box_shadow(mut self, shadow: Option<BoxShadowSpec>) -> Self {
        self.box_shadow = shadow.clone();
        self.box_shadows = shadow.into_iter().collect();
        self
    }

    pub fn with_box_shadows(mut self, shadows: Vec<BoxShadowSpec>) -> Self {
        self.box_shadow = shadows.first().cloned();
        self.box_shadows = shadows;
        self
    }

    pub fn with_paint_filter(mut self, filter: Option<PaintFilterSpec>) -> Self {
        self.paint_filter = filter.and_then(|value| {
            if value.is_identity() {
                None
            } else {
                Some(value)
            }
        });
        self
    }

    pub fn with_backdrop_filter(mut self, filter: Option<PaintFilterSpec>) -> Self {
        self.backdrop_filter = filter.and_then(|value| {
            if value.is_identity() {
                None
            } else {
                Some(value)
            }
        });
        self
    }

    pub fn with_mix_blend_mode(mut self, mode: MixBlendMode) -> Self {
        self.mix_blend_mode = mode;
        self
    }

    pub fn with_isolation(mut self, isolation: bool) -> Self {
        self.isolation = isolation;
        self
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    pub fn with_transforms(mut self, transforms: Vec<CssTransformOp>) -> Self {
        self.transforms = transforms;
        self
    }

    pub fn with_transform_origin(mut self, transform_origin: CssTransformOrigin) -> Self {
        self.transform_origin = transform_origin;
        self
    }

    pub fn with_overflow_hidden(mut self, overflow_hidden: bool) -> Self {
        self.overflow_hidden = overflow_hidden;
        self
    }

    pub fn with_self_visible(mut self, visible: bool) -> Self {
        self.self_visible = visible;
        self
    }

    pub fn with_tag_role(mut self, role: impl Into<Arc<str>>) -> Self {
        self.tag_role = Some(role.into());
        self
    }

    pub fn with_establishes_abs_containing_block(mut self, enabled: bool) -> Self {
        self.establishes_abs_containing_block = enabled;
        self
    }

    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = pagination;
        self
    }

    fn resolve_fixed_height(&self, avail_height: Pt) -> Option<Pt> {
        match self.height {
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial => None,
            LengthSpec::Percent(_) if avail_height >= huge_pt() => None,
            LengthSpec::Calc(calc) if calc.percent != 0.0 && avail_height >= huge_pt() => None,
            _ => Some(
                self.height
                    .resolve_height(avail_height, self.font_size, self.root_font_size)
                    .max(Pt::ZERO),
            ),
        }
    }

    fn resolve_width_constraint(&self, spec: LengthSpec, avail_width: Pt) -> Option<Pt> {
        match spec {
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial => None,
            _ => Some(
                spec.resolve_width(avail_width, self.font_size, self.root_font_size)
                    .max(Pt::ZERO),
            ),
        }
    }

    fn resolve_height_constraint(&self, spec: LengthSpec, avail_height: Pt) -> Option<Pt> {
        match spec {
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial => None,
            LengthSpec::Percent(_) if avail_height >= huge_pt() => None,
            LengthSpec::Calc(calc) if calc.percent != 0.0 && avail_height >= huge_pt() => None,
            _ => Some(
                spec.resolve_height(avail_height, self.font_size, self.root_font_size)
                    .max(Pt::ZERO),
            ),
        }
    }

    fn resolve_box(
        &self,
        avail_width: Pt,
    ) -> (ResolvedEdges, ResolvedEdges, ResolvedEdges, Pt, Pt) {
        let margin_spec = self.margin;
        let mut margin = margin_spec.resolve(avail_width, self.font_size, self.root_font_size);
        let auto_left = matches!(margin_spec.left, LengthSpec::Auto);
        let auto_right = matches!(margin_spec.right, LengthSpec::Auto);
        let border = self
            .border_width
            .resolve(avail_width, self.font_size, self.root_font_size);
        let padding = self
            .padding
            .resolve(avail_width, self.font_size, self.root_font_size);

        let available_content_width = (avail_width
            - margin.left
            - margin.right
            - border.left
            - border.right
            - padding.left
            - padding.right)
            .max(Pt::ZERO);
        let mut content_width = match self.width {
            LengthSpec::Auto => available_content_width,
            _ => self
                .width
                .resolve_width(avail_width, self.font_size, self.root_font_size),
        };
        let mut border_box_width = if matches!(self.width, LengthSpec::Auto) {
            border.left + padding.left + content_width + padding.right + border.right
        } else if matches!(self.box_sizing, BoxSizingMode::BorderBox) {
            let resolved = self
                .width
                .resolve_width(avail_width, self.font_size, self.root_font_size)
                .max(Pt::ZERO);
            content_width = (resolved - border.left - border.right - padding.left - padding.right)
                .max(Pt::ZERO);
            resolved
        } else {
            border.left + padding.left + content_width + padding.right + border.right
        };
        if !matches!(
            self.max_width,
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
        ) {
            let max_width =
                self.max_width
                    .resolve_width(avail_width, self.font_size, self.root_font_size);
            if matches!(self.box_sizing, BoxSizingMode::BorderBox) {
                if border_box_width > max_width {
                    border_box_width = max_width;
                    content_width = (border_box_width
                        - border.left
                        - border.right
                        - padding.left
                        - padding.right)
                        .max(Pt::ZERO);
                }
            } else if content_width > max_width {
                content_width = max_width;
                border_box_width =
                    border.left + padding.left + content_width + padding.right + border.right;
            }
        }
        if let Some(min_width) = self.resolve_width_constraint(self.min_width, avail_width) {
            if matches!(self.box_sizing, BoxSizingMode::BorderBox) {
                if border_box_width < min_width {
                    border_box_width = min_width;
                    content_width = (border_box_width
                        - border.left
                        - border.right
                        - padding.left
                        - padding.right)
                        .max(Pt::ZERO);
                }
            } else if content_width < min_width {
                content_width = min_width;
                border_box_width =
                    border.left + padding.left + content_width + padding.right + border.right;
            }
        }
        let content_width = content_width.max(Pt::ZERO);
        if matches!(self.box_sizing, BoxSizingMode::ContentBox)
            || matches!(self.width, LengthSpec::Auto)
        {
            border_box_width =
                border.left + padding.left + content_width + padding.right + border.right;
        }
        let extra = (avail_width - (border_box_width + margin.left + margin.right)).max(Pt::ZERO);
        if auto_left && auto_right {
            let half = extra.mul_ratio(1, 2);
            margin.left = half;
            margin.right = extra - half;
        } else if auto_left {
            margin.left = extra;
        } else if auto_right {
            margin.right = extra;
        }
        (margin, border, padding, content_width, border_box_width)
    }

    fn has_transforms(&self) -> bool {
        !self.transforms.is_empty()
    }

    fn apply_transforms(&self, canvas: &mut Canvas, ref_width: Pt, ref_height: Pt) {
        for op in &self.transforms {
            match op {
                CssTransformOp::Translate { x, y } => {
                    let tx = x.resolve_width(ref_width, self.font_size, self.root_font_size);
                    let ty = y.resolve_height(ref_height, self.font_size, self.root_font_size);
                    canvas.translate(tx, ty);
                }
                CssTransformOp::Scale { x, y } => {
                    canvas.scale(*x, *y);
                }
                CssTransformOp::Rotate { radians } => {
                    // CSS uses a top-down coordinate system, so positive angles
                    // are clockwise. PDF's current transformation matrix is
                    // bottom-up, which requires the opposite mathematical sign.
                    canvas.rotate(-*radians);
                }
                CssTransformOp::Skew {
                    x_radians,
                    y_radians,
                } => {
                    let c = x_radians.tan();
                    let b = y_radians.tan();
                    if c.is_finite() && b.is_finite() {
                        canvas.concat_matrix(1.0, b, c, 1.0, Pt::ZERO, Pt::ZERO);
                    }
                }
                CssTransformOp::Matrix { a, b, c, d, e, f } => {
                    if a.is_finite() && b.is_finite() && c.is_finite() && d.is_finite() {
                        canvas.concat_matrix(*a, *b, *c, *d, *e, *f);
                    }
                }
            }
        }
    }

    fn compute_layout(&self, avail_width: Pt, avail_height: Pt) -> ContainerLayoutCache {
        let (margin, border, padding, content_width, border_box_width) =
            self.resolve_box(avail_width);

        let fixed_height = self.resolve_fixed_height(avail_height);
        let (fixed_content_height, fixed_border_box_height) = if let Some(resolved) = fixed_height {
            if matches!(self.box_sizing, BoxSizingMode::BorderBox) {
                let border_box_height = resolved.max(Pt::ZERO);
                let content_height =
                    (border_box_height - border.top - border.bottom - padding.top - padding.bottom)
                        .max(Pt::ZERO);
                (Some(content_height), Some(border_box_height))
            } else {
                let content_height = resolved.max(Pt::ZERO);
                let border_box_height =
                    border.top + padding.top + content_height + padding.bottom + border.bottom;
                (Some(content_height), Some(border_box_height))
            }
        } else {
            (None, None)
        };

        // Only provide a bounded height to children when we have an explicit height. Otherwise,
        // children should measure naturally (important for flex rows not ballooning to page height).
        let child_avail_height = fixed_content_height.unwrap_or(huge_pt());
        let mut content_height: Pt = Pt::ZERO;
        let mut child_sizes: Vec<Option<Size>> = Vec::with_capacity(self.children.len());
        let mut in_flow_pagination: Vec<Pagination> = Vec::new();
        for child in &self.children {
            if child.out_of_flow() {
                child_sizes.push(None);
                continue;
            }
            in_flow_pagination.push(child.pagination());
            let size = child.wrap(content_width, child_avail_height);
            content_height = content_height + size.height;
            child_sizes.push(Some(size));
        }

        if fixed_content_height.is_none() && !in_flow_pagination.is_empty() {
            let mut forced_breaks = 0usize;
            for (idx, pagination) in in_flow_pagination.iter().enumerate() {
                if idx > 0 && pagination.break_before.forces_page() {
                    forced_breaks += 1;
                }
                if idx + 1 < in_flow_pagination.len() && pagination.break_after.forces_page() {
                    forced_breaks += 1;
                }
            }
            if forced_breaks > 0 {
                let break_unit = if avail_height >= huge_pt() {
                    Pt::from_f32(792.0)
                } else {
                    avail_height.max(Pt::from_f32(1.0))
                };
                for _ in 0..forced_breaks {
                    content_height = content_height + break_unit;
                }
            }
        }

        let mut content_height = fixed_content_height.unwrap_or(content_height);
        let mut border_box_height = fixed_border_box_height.unwrap_or_else(|| {
            border.top + padding.top + content_height + padding.bottom + border.bottom
        });
        if let Some(max_height) = self.resolve_height_constraint(self.max_height, avail_height) {
            if matches!(self.box_sizing, BoxSizingMode::BorderBox) {
                if border_box_height > max_height {
                    border_box_height = max_height;
                    content_height = (border_box_height
                        - border.top
                        - border.bottom
                        - padding.top
                        - padding.bottom)
                        .max(Pt::ZERO);
                }
            } else if content_height > max_height {
                content_height = max_height;
                border_box_height =
                    border.top + padding.top + content_height + padding.bottom + border.bottom;
            }
        }
        if let Some(min_height) = self.resolve_height_constraint(self.min_height, avail_height) {
            if matches!(self.box_sizing, BoxSizingMode::BorderBox) {
                if border_box_height < min_height {
                    border_box_height = min_height;
                    content_height = (border_box_height
                        - border.top
                        - border.bottom
                        - padding.top
                        - padding.bottom)
                        .max(Pt::ZERO);
                }
            } else if content_height < min_height {
                content_height = min_height;
                border_box_height =
                    border.top + padding.top + content_height + padding.bottom + border.bottom;
            }
        }
        let total_height = margin.top + border_box_height + margin.bottom;
        let total_width = margin.left + border_box_width + margin.right;

        ContainerLayoutCache {
            avail_width_milli: avail_width.to_milli_i64(),
            avail_height_milli: avail_height.to_milli_i64(),
            margin,
            border,
            padding,
            content_width,
            border_box_width,
            content_height,
            border_box_height,
            total_width,
            total_height,
            child_avail_height,
            child_sizes,
        }
    }

    fn cached_layout(&self, avail_width: Pt, avail_height: Pt) -> ContainerLayoutCache {
        let key_w = avail_width.to_milli_i64();
        let key_h = avail_height.to_milli_i64();
        if let Some(cache) = self.layout_cache.lock().unwrap().as_ref() {
            if cache.avail_width_milli == key_w && cache.avail_height_milli == key_h {
                return cache.clone();
            }
        }
        let cache = self.compute_layout(avail_width, avail_height);
        *self.layout_cache.lock().unwrap() = Some(cache.clone());
        cache
    }

    fn zero_top(mut edges: EdgeSizes) -> EdgeSizes {
        edges.top = LengthSpec::Absolute(Pt::ZERO);
        edges
    }

    fn zero_bottom(mut edges: EdgeSizes) -> EdgeSizes {
        edges.bottom = LengthSpec::Absolute(Pt::ZERO);
        edges
    }

    fn has_border(edges: ResolvedEdges) -> bool {
        edges.top > Pt::ZERO
            || edges.right > Pt::ZERO
            || edges.bottom > Pt::ZERO
            || edges.left > Pt::ZERO
    }

    fn draw_border(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        border: ResolvedEdges,
        colors: ResolvedEdgeColors,
        styles: ResolvedEdgeStyles,
    ) {
        Self::draw_border_side(
            canvas,
            BorderSide::Top,
            x,
            y,
            width,
            height,
            border,
            colors.top,
            styles.top,
        );
        Self::draw_border_side(
            canvas,
            BorderSide::Bottom,
            x,
            y,
            width,
            height,
            border,
            colors.bottom,
            styles.bottom,
        );
        Self::draw_border_side(
            canvas,
            BorderSide::Left,
            x,
            y,
            width,
            height,
            border,
            colors.left,
            styles.left,
        );
        Self::draw_border_side(
            canvas,
            BorderSide::Right,
            x,
            y,
            width,
            height,
            border,
            colors.right,
            styles.right,
        );
    }

    fn draw_border_side(
        canvas: &mut Canvas,
        side: BorderSide,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        border: ResolvedEdges,
        color: Color,
        style: OutlineLineStyle,
    ) {
        let side_width = match side {
            BorderSide::Top => border.top,
            BorderSide::Right => border.right,
            BorderSide::Bottom => border.bottom,
            BorderSide::Left => border.left,
        };
        if side_width <= Pt::ZERO {
            return;
        }
        match style {
            OutlineLineStyle::Solid => {
                Self::draw_solid_border_side(canvas, side, x, y, width, height, side_width, color);
            }
            OutlineLineStyle::Dotted | OutlineLineStyle::Dashed => {
                Self::draw_stroked_border_side(
                    canvas, side, x, y, width, height, side_width, color, style,
                );
            }
            OutlineLineStyle::Double => {
                Self::draw_double_border_side(canvas, side, x, y, width, height, side_width, color);
            }
            OutlineLineStyle::Groove
            | OutlineLineStyle::Ridge
            | OutlineLineStyle::Inset
            | OutlineLineStyle::Outset => {
                Self::draw_3d_border_side(
                    canvas, side, x, y, width, height, side_width, color, style,
                );
            }
        }
    }

    fn draw_solid_border_side(
        canvas: &mut Canvas,
        side: BorderSide,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        side_width: Pt,
        color: Color,
    ) {
        canvas.set_fill_color(color);
        match side {
            BorderSide::Top => canvas.draw_rect(x, y, width, side_width),
            BorderSide::Bottom => canvas.draw_rect(x, y + height - side_width, width, side_width),
            BorderSide::Left => canvas.draw_rect(x, y, side_width, height),
            BorderSide::Right => canvas.draw_rect(x + width - side_width, y, side_width, height),
        }
    }

    fn draw_stroked_border_side(
        canvas: &mut Canvas,
        side: BorderSide,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        side_width: Pt,
        color: Color,
        style: OutlineLineStyle,
    ) {
        canvas.save_state();
        canvas.set_stroke_color(color);
        canvas.set_line_width(side_width);
        Self::apply_outline_stroke_style(canvas, style, side_width);
        match side {
            BorderSide::Top => {
                let cy = y + side_width / 2.0;
                canvas.move_to(x, cy);
                canvas.line_to(x + width, cy);
            }
            BorderSide::Bottom => {
                let cy = y + height - side_width / 2.0;
                canvas.move_to(x, cy);
                canvas.line_to(x + width, cy);
            }
            BorderSide::Left => {
                let cx = x + side_width / 2.0;
                canvas.move_to(cx, y);
                canvas.line_to(cx, y + height);
            }
            BorderSide::Right => {
                let cx = x + width - side_width / 2.0;
                canvas.move_to(cx, y);
                canvas.line_to(cx, y + height);
            }
        }
        canvas.stroke();
        canvas.restore_state();
    }

    fn draw_double_border_side(
        canvas: &mut Canvas,
        side: BorderSide,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        side_width: Pt,
        color: Color,
    ) {
        let band = side_width / 3.0;
        if band <= Pt::ZERO {
            return;
        }
        let inner_offset = (side_width - band).max(Pt::ZERO);
        canvas.set_fill_color(color);
        match side {
            BorderSide::Top => {
                canvas.draw_rect(x, y, width, band);
                canvas.draw_rect(x, y + inner_offset, width, band);
            }
            BorderSide::Bottom => {
                canvas.draw_rect(x, y + height - side_width, width, band);
                canvas.draw_rect(x, y + height - band, width, band);
            }
            BorderSide::Left => {
                canvas.draw_rect(x, y, band, height);
                canvas.draw_rect(x + inner_offset, y, band, height);
            }
            BorderSide::Right => {
                canvas.draw_rect(x + width - side_width, y, band, height);
                canvas.draw_rect(x + width - band, y, band, height);
            }
        }
    }

    fn draw_3d_border_side(
        canvas: &mut Canvas,
        side: BorderSide,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        side_width: Pt,
        color: Color,
        style: OutlineLineStyle,
    ) {
        match style {
            OutlineLineStyle::Inset | OutlineLineStyle::Outset => {
                let shaded = Self::border_3d_side_color(side, style, color);
                Self::draw_solid_border_side(canvas, side, x, y, width, height, side_width, shaded);
            }
            OutlineLineStyle::Groove | OutlineLineStyle::Ridge => {
                let outer = side_width / 2.0;
                let inner = (side_width - outer).max(Pt::ZERO);
                if outer <= Pt::ZERO || inner <= Pt::ZERO {
                    return;
                }
                let outer_color = Self::border_3d_side_color(side, style, color);
                let inner_style = match style {
                    OutlineLineStyle::Groove => OutlineLineStyle::Ridge,
                    OutlineLineStyle::Ridge => OutlineLineStyle::Groove,
                    _ => style,
                };
                let inner_color = Self::border_3d_side_color(side, inner_style, color);
                Self::draw_split_border_side(
                    canvas,
                    side,
                    x,
                    y,
                    width,
                    height,
                    outer,
                    inner,
                    outer_color,
                    inner_color,
                );
            }
            _ => {}
        }
    }

    fn draw_split_border_side(
        canvas: &mut Canvas,
        side: BorderSide,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        outer: Pt,
        inner: Pt,
        outer_color: Color,
        inner_color: Color,
    ) {
        canvas.set_fill_color(outer_color);
        match side {
            BorderSide::Top => canvas.draw_rect(x, y, width, outer),
            BorderSide::Bottom => canvas.draw_rect(x, y + height - outer, width, outer),
            BorderSide::Left => canvas.draw_rect(x, y, outer, height),
            BorderSide::Right => canvas.draw_rect(x + width - outer, y, outer, height),
        }
        canvas.set_fill_color(inner_color);
        match side {
            BorderSide::Top => canvas.draw_rect(x, y + outer, width, inner),
            BorderSide::Bottom => canvas.draw_rect(x, y + height - outer - inner, width, inner),
            BorderSide::Left => canvas.draw_rect(x + outer, y, inner, height),
            BorderSide::Right => canvas.draw_rect(x + width - outer - inner, y, inner, height),
        }
    }

    fn border_3d_side_color(side: BorderSide, style: OutlineLineStyle, color: Color) -> Color {
        let (top_left, bottom_right) = Self::outline_3d_edge_colors(style, color);
        match side {
            BorderSide::Top | BorderSide::Left => top_left,
            BorderSide::Right | BorderSide::Bottom => bottom_right,
        }
    }

    fn draw_outline(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        outline_width: Pt,
        outline_offset: Pt,
        style: OutlineLineStyle,
        color: Color,
        radius: ResolvedClipPathRadii,
    ) {
        if outline_width <= Pt::ZERO {
            return;
        }
        if matches!(
            style,
            OutlineLineStyle::Groove
                | OutlineLineStyle::Ridge
                | OutlineLineStyle::Inset
                | OutlineLineStyle::Outset
        ) {
            Self::draw_3d_outline(
                canvas,
                x,
                y,
                width,
                height,
                outline_width,
                outline_offset,
                style,
                color,
                radius,
            );
            return;
        }
        if matches!(style, OutlineLineStyle::Double) {
            Self::draw_double_outline(
                canvas,
                x,
                y,
                width,
                height,
                outline_width,
                outline_offset,
                color,
                radius,
            );
            return;
        }
        if !matches!(style, OutlineLineStyle::Solid) {
            Self::draw_stroked_outline(
                canvas,
                x,
                y,
                width,
                height,
                outline_width,
                outline_offset,
                style,
                color,
                radius,
            );
            return;
        }
        if Self::clip_radii_have_rounding(radius) {
            let offset = outline_offset + (outline_width / 2.0);
            let outline_box_width = (width + offset * 2.0).max(Pt::ZERO);
            let outline_box_height = (height + offset * 2.0).max(Pt::ZERO);
            if outline_box_width <= Pt::ZERO || outline_box_height <= Pt::ZERO {
                return;
            }
            canvas.save_state();
            canvas.set_dash(Vec::new(), Pt::ZERO);
            canvas.set_stroke_color(color);
            canvas.set_line_width(outline_width);
            Self::draw_rounded_rect_corners_stroke(
                canvas,
                x - offset,
                y - offset,
                outline_box_width,
                outline_box_height,
                Self::offset_clip_radii(radius, offset),
            );
            canvas.restore_state();
            return;
        }

        let outer_x = x - outline_offset - outline_width;
        let outer_y = y - outline_offset - outline_width;
        let horizontal_width = (width + (outline_offset + outline_width) * 2.0).max(Pt::ZERO);
        let vertical_height = (height + outline_offset * 2.0).max(Pt::ZERO);
        canvas.set_fill_color(color);
        if horizontal_width > Pt::ZERO {
            canvas.draw_rect(outer_x, outer_y, horizontal_width, outline_width);
            canvas.draw_rect(
                outer_x,
                y + height + outline_offset,
                horizontal_width,
                outline_width,
            );
        }
        if vertical_height > Pt::ZERO {
            canvas.draw_rect(outer_x, y - outline_offset, outline_width, vertical_height);
            canvas.draw_rect(
                x + width + outline_offset,
                y - outline_offset,
                outline_width,
                vertical_height,
            );
        }
    }

    fn apply_outline_stroke_style(canvas: &mut Canvas, style: OutlineLineStyle, outline_width: Pt) {
        match style {
            OutlineLineStyle::Dotted => {
                canvas.set_line_cap(1);
                canvas.set_dash(
                    vec![
                        (outline_width * 0.01).max(Pt::from_f32(0.01)),
                        outline_width * 2.0,
                    ],
                    Pt::ZERO,
                );
            }
            OutlineLineStyle::Dashed => {
                canvas.set_line_cap(2);
                canvas.set_dash(vec![outline_width * 3.0, outline_width * 2.0], Pt::ZERO);
            }
            OutlineLineStyle::Solid
            | OutlineLineStyle::Double
            | OutlineLineStyle::Groove
            | OutlineLineStyle::Ridge
            | OutlineLineStyle::Inset
            | OutlineLineStyle::Outset => {
                canvas.set_line_cap(0);
                canvas.set_dash(Vec::new(), Pt::ZERO);
            }
        }
    }

    fn stroke_outline_rect(canvas: &mut Canvas, x: Pt, y: Pt, width: Pt, height: Pt, radius: Pt) {
        if radius > Pt::ZERO {
            Self::draw_rounded_rect_stroke(canvas, x, y, width, height, radius);
        } else {
            Self::rounded_rect_path(canvas, x, y, width, height, Pt::ZERO);
            canvas.stroke();
        }
    }

    fn draw_stroked_outline(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        outline_width: Pt,
        outline_offset: Pt,
        style: OutlineLineStyle,
        color: Color,
        radius: ResolvedClipPathRadii,
    ) {
        let offset = outline_offset + (outline_width / 2.0);
        let outline_box_width = (width + offset * 2.0).max(Pt::ZERO);
        let outline_box_height = (height + offset * 2.0).max(Pt::ZERO);
        if outline_box_width <= Pt::ZERO || outline_box_height <= Pt::ZERO {
            return;
        }
        canvas.save_state();
        canvas.set_stroke_color(color);
        canvas.set_line_width(outline_width);
        Self::apply_outline_stroke_style(canvas, style, outline_width);
        if Self::clip_radii_have_rounding(radius) {
            Self::draw_rounded_rect_corners_stroke(
                canvas,
                x - offset,
                y - offset,
                outline_box_width,
                outline_box_height,
                Self::offset_clip_radii(radius, offset),
            );
        } else {
            Self::stroke_outline_rect(
                canvas,
                x - offset,
                y - offset,
                outline_box_width,
                outline_box_height,
                offset.max(Pt::ZERO),
            );
        }
        canvas.restore_state();
    }

    fn draw_double_outline(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        outline_width: Pt,
        outline_offset: Pt,
        color: Color,
        radius: ResolvedClipPathRadii,
    ) {
        let line_width = outline_width / 3.0;
        if line_width <= Pt::ZERO {
            return;
        }
        canvas.save_state();
        canvas.set_stroke_color(color);
        canvas.set_line_width(line_width);
        Self::apply_outline_stroke_style(canvas, OutlineLineStyle::Solid, line_width);

        for offset in [
            outline_offset + outline_width - line_width / 2.0,
            outline_offset + line_width / 2.0,
        ] {
            let outline_box_width = (width + offset * 2.0).max(Pt::ZERO);
            let outline_box_height = (height + offset * 2.0).max(Pt::ZERO);
            if outline_box_width <= Pt::ZERO || outline_box_height <= Pt::ZERO {
                continue;
            }
            if Self::clip_radii_have_rounding(radius) {
                Self::draw_rounded_rect_corners_stroke(
                    canvas,
                    x - offset,
                    y - offset,
                    outline_box_width,
                    outline_box_height,
                    Self::offset_clip_radii(radius, offset),
                );
            } else {
                Self::stroke_outline_rect(
                    canvas,
                    x - offset,
                    y - offset,
                    outline_box_width,
                    outline_box_height,
                    offset.max(Pt::ZERO),
                );
            }
        }

        canvas.restore_state();
    }

    fn draw_3d_outline(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        outline_width: Pt,
        outline_offset: Pt,
        style: OutlineLineStyle,
        color: Color,
        radius: ResolvedClipPathRadii,
    ) {
        if outline_width <= Pt::ZERO {
            return;
        }
        let outer_x = x - outline_offset - outline_width;
        let outer_y = y - outline_offset - outline_width;
        let outer_width = (width + (outline_offset + outline_width) * 2.0).max(Pt::ZERO);
        let outer_height = (height + (outline_offset + outline_width) * 2.0).max(Pt::ZERO);
        if outer_width <= Pt::ZERO || outer_height <= Pt::ZERO {
            return;
        }

        if Self::clip_radii_have_rounding(radius) {
            let rounded_radius = Self::offset_clip_radii(radius, outline_offset + outline_width);
            Self::draw_rounded_uniform_3d_border(
                canvas,
                outer_x,
                outer_y,
                outer_width,
                outer_height,
                outline_width,
                color,
                style,
                rounded_radius,
            );
            return;
        }

        canvas.save_state();
        match style {
            OutlineLineStyle::Inset | OutlineLineStyle::Outset => {
                let (top_left, bottom_right) = Self::outline_3d_edge_colors(style, color);
                Self::draw_outline_band_box(
                    canvas,
                    outer_x,
                    outer_y,
                    outer_width,
                    outer_height,
                    outline_width,
                    top_left,
                    bottom_right,
                    top_left,
                    bottom_right,
                );
            }
            OutlineLineStyle::Groove | OutlineLineStyle::Ridge => {
                let outer_band = outline_width / 2.0;
                let inner_band = (outline_width - outer_band).max(Pt::ZERO);
                let (outer_top_left, outer_bottom_right) =
                    Self::outline_3d_edge_colors(style, color);
                let (inner_top_left, inner_bottom_right) = match style {
                    OutlineLineStyle::Groove => {
                        Self::outline_3d_edge_colors(OutlineLineStyle::Ridge, color)
                    }
                    OutlineLineStyle::Ridge => {
                        Self::outline_3d_edge_colors(OutlineLineStyle::Groove, color)
                    }
                    _ => unreachable!(),
                };
                Self::draw_outline_band_box(
                    canvas,
                    outer_x,
                    outer_y,
                    outer_width,
                    outer_height,
                    outer_band,
                    outer_top_left,
                    outer_bottom_right,
                    outer_top_left,
                    outer_bottom_right,
                );
                let inner_x = outer_x + outer_band;
                let inner_y = outer_y + outer_band;
                let inner_width = (outer_width - outer_band * 2.0).max(Pt::ZERO);
                let inner_height = (outer_height - outer_band * 2.0).max(Pt::ZERO);
                Self::draw_outline_band_box(
                    canvas,
                    inner_x,
                    inner_y,
                    inner_width,
                    inner_height,
                    inner_band,
                    inner_top_left,
                    inner_bottom_right,
                    inner_top_left,
                    inner_bottom_right,
                );
            }
            _ => {}
        }
        canvas.restore_state();
    }

    fn outline_3d_edge_colors(style: OutlineLineStyle, color: Color) -> (Color, Color) {
        let light = Self::lerp_color(color, Color::rgb(1.0, 1.0, 1.0), 0.45);
        let dark = Self::lerp_color(color, Color::BLACK, 0.45);
        match style {
            OutlineLineStyle::Groove | OutlineLineStyle::Inset => (dark, light),
            OutlineLineStyle::Ridge | OutlineLineStyle::Outset => (light, dark),
            _ => (color, color),
        }
    }

    fn draw_outline_band_box(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        band: Pt,
        top: Color,
        right: Color,
        left: Color,
        bottom: Color,
    ) {
        if band <= Pt::ZERO || width <= Pt::ZERO || height <= Pt::ZERO {
            return;
        }
        let side_height = (height - band * 2.0).max(Pt::ZERO);
        canvas.set_fill_color(top);
        canvas.draw_rect(x, y, width, band);
        canvas.set_fill_color(bottom);
        canvas.draw_rect(x, y + height - band, width, band);
        if side_height > Pt::ZERO {
            canvas.set_fill_color(left);
            canvas.draw_rect(x, y + band, band, side_height);
            canvas.set_fill_color(right);
            canvas.draw_rect(x + width - band, y + band, band, side_height);
        }
    }

    fn offset_clip_radii(radius: ResolvedClipPathRadii, offset: Pt) -> ResolvedClipPathRadii {
        ResolvedClipPathRadii {
            top_left_x: (radius.top_left_x + offset).max(Pt::ZERO),
            top_left_y: (radius.top_left_y + offset).max(Pt::ZERO),
            top_right_x: (radius.top_right_x + offset).max(Pt::ZERO),
            top_right_y: (radius.top_right_y + offset).max(Pt::ZERO),
            bottom_right_x: (radius.bottom_right_x + offset).max(Pt::ZERO),
            bottom_right_y: (radius.bottom_right_y + offset).max(Pt::ZERO),
            bottom_left_x: (radius.bottom_left_x + offset).max(Pt::ZERO),
            bottom_left_y: (radius.bottom_left_y + offset).max(Pt::ZERO),
        }
    }

    fn clip_radii_have_rounding(radius: ResolvedClipPathRadii) -> bool {
        radius.top_left_x > Pt::ZERO
            || radius.top_left_y > Pt::ZERO
            || radius.top_right_x > Pt::ZERO
            || radius.top_right_y > Pt::ZERO
            || radius.bottom_right_x > Pt::ZERO
            || radius.bottom_right_y > Pt::ZERO
            || radius.bottom_left_x > Pt::ZERO
            || radius.bottom_left_y > Pt::ZERO
    }

    fn inset_clip_radii(radius: ResolvedClipPathRadii, inset: Pt) -> ResolvedClipPathRadii {
        ResolvedClipPathRadii {
            top_left_x: (radius.top_left_x - inset).max(Pt::ZERO),
            top_left_y: (radius.top_left_y - inset).max(Pt::ZERO),
            top_right_x: (radius.top_right_x - inset).max(Pt::ZERO),
            top_right_y: (radius.top_right_y - inset).max(Pt::ZERO),
            bottom_right_x: (radius.bottom_right_x - inset).max(Pt::ZERO),
            bottom_right_y: (radius.bottom_right_y - inset).max(Pt::ZERO),
            bottom_left_x: (radius.bottom_left_x - inset).max(Pt::ZERO),
            bottom_left_y: (radius.bottom_left_y - inset).max(Pt::ZERO),
        }
    }

    fn inset_clip_radii_edges(
        radius: ResolvedClipPathRadii,
        edges: ResolvedEdges,
    ) -> ResolvedClipPathRadii {
        ResolvedClipPathRadii {
            top_left_x: (radius.top_left_x - edges.left).max(Pt::ZERO),
            top_left_y: (radius.top_left_y - edges.top).max(Pt::ZERO),
            top_right_x: (radius.top_right_x - edges.right).max(Pt::ZERO),
            top_right_y: (radius.top_right_y - edges.top).max(Pt::ZERO),
            bottom_right_x: (radius.bottom_right_x - edges.right).max(Pt::ZERO),
            bottom_right_y: (radius.bottom_right_y - edges.bottom).max(Pt::ZERO),
            bottom_left_x: (radius.bottom_left_x - edges.left).max(Pt::ZERO),
            bottom_left_y: (radius.bottom_left_y - edges.bottom).max(Pt::ZERO),
        }
    }

    fn outset_clip_radii_edges(
        radius: ResolvedClipPathRadii,
        edges: ResolvedEdges,
    ) -> ResolvedClipPathRadii {
        ResolvedClipPathRadii {
            top_left_x: (radius.top_left_x + edges.left).max(Pt::ZERO),
            top_left_y: (radius.top_left_y + edges.top).max(Pt::ZERO),
            top_right_x: (radius.top_right_x + edges.right).max(Pt::ZERO),
            top_right_y: (radius.top_right_y + edges.top).max(Pt::ZERO),
            bottom_right_x: (radius.bottom_right_x + edges.right).max(Pt::ZERO),
            bottom_right_y: (radius.bottom_right_y + edges.bottom).max(Pt::ZERO),
            bottom_left_x: (radius.bottom_left_x + edges.left).max(Pt::ZERO),
            bottom_left_y: (radius.bottom_left_y + edges.bottom).max(Pt::ZERO),
        }
    }

    fn uniform_radius_from_clip_radii(radius: ResolvedClipPathRadii) -> Pt {
        let mut r = radius.top_left_x.min(radius.top_left_y);
        r = r.min(radius.top_right_x).min(radius.top_right_y);
        r = r.min(radius.bottom_right_x).min(radius.bottom_right_y);
        r = r.min(radius.bottom_left_x).min(radius.bottom_left_y);
        r
    }

    fn rounded_rect_path(canvas: &mut Canvas, x: Pt, y: Pt, width: Pt, height: Pt, radius: Pt) {
        let mut r = radius;
        if r <= Pt::ZERO {
            canvas.draw_rect(x, y, width, height);
            return;
        }
        let max_r = (width / 2.0).min(height / 2.0);
        if r > max_r {
            r = max_r;
        }
        let k = 0.55228475;
        let c = r * k;
        let right = x + width;
        let bottom = y + height;

        canvas.move_to(x + r, y);
        canvas.line_to(right - r, y);
        canvas.curve_to(right - r + c, y, right, y + r - c, right, y + r);
        canvas.line_to(right, bottom - r);
        canvas.curve_to(
            right,
            bottom - r + c,
            right - r + c,
            bottom,
            right - r,
            bottom,
        );
        canvas.line_to(x + r, bottom);
        canvas.curve_to(x + r - c, bottom, x, bottom - r + c, x, bottom - r);
        canvas.line_to(x, y + r);
        canvas.curve_to(x, y + r - c, x + r - c, y, x + r, y);
        canvas.close_path();
    }

    fn draw_rounded_rect_fill(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: Pt,
    ) {
        Self::rounded_rect_path(canvas, x, y, width, height, radius);
        canvas.fill();
    }

    fn draw_rounded_rect_stroke(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: Pt,
    ) {
        Self::rounded_rect_path(canvas, x, y, width, height, radius);
        canvas.stroke();
    }

    fn draw_rounded_rect_corners_fill(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: ResolvedClipPathRadii,
    ) {
        Self::rounded_rect_corners_path(canvas, x, y, width, height, radius);
        canvas.fill();
    }

    fn draw_rounded_rect_corners_stroke(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: ResolvedClipPathRadii,
    ) {
        Self::rounded_rect_corners_path(canvas, x, y, width, height, radius);
        canvas.stroke();
    }

    fn draw_rounded_uniform_border_stroke(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        border_width: Pt,
        color: Color,
        style: OutlineLineStyle,
        radius: ResolvedClipPathRadii,
    ) {
        if border_width <= Pt::ZERO {
            return;
        }
        let inset = border_width / 2.0;
        let stroke_width = (width - border_width).max(Pt::ZERO);
        let stroke_height = (height - border_width).max(Pt::ZERO);
        if stroke_width <= Pt::ZERO || stroke_height <= Pt::ZERO {
            return;
        }
        let stroke_radius = Self::inset_clip_radii(radius, inset);
        canvas.save_state();
        canvas.set_stroke_color(color);
        canvas.set_line_width(border_width);
        Self::apply_outline_stroke_style(canvas, style, border_width);
        Self::draw_rounded_rect_corners_stroke(
            canvas,
            x + inset,
            y + inset,
            stroke_width,
            stroke_height,
            stroke_radius,
        );
        canvas.restore_state();
    }

    fn draw_rounded_uniform_double_border(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        border_width: Pt,
        color: Color,
        radius: ResolvedClipPathRadii,
    ) {
        if border_width <= Pt::ZERO {
            return;
        }
        let line_width = border_width / 3.0;
        if line_width <= Pt::ZERO {
            return;
        }
        canvas.save_state();
        canvas.set_stroke_color(color);
        canvas.set_line_width(line_width);
        Self::apply_outline_stroke_style(canvas, OutlineLineStyle::Solid, line_width);

        for inset in [line_width / 2.0, border_width - (line_width / 2.0)] {
            let stroke_width = (width - inset * 2.0).max(Pt::ZERO);
            let stroke_height = (height - inset * 2.0).max(Pt::ZERO);
            if stroke_width <= Pt::ZERO || stroke_height <= Pt::ZERO {
                continue;
            }
            let stroke_radius = Self::inset_clip_radii(radius, inset);
            Self::draw_rounded_rect_corners_stroke(
                canvas,
                x + inset,
                y + inset,
                stroke_width,
                stroke_height,
                stroke_radius,
            );
        }

        canvas.restore_state();
    }

    fn draw_rounded_border_stroke_region(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        line_width: Pt,
        color: Color,
        radius: ResolvedClipPathRadii,
        clip_x: Pt,
        clip_y: Pt,
        clip_width: Pt,
        clip_height: Pt,
    ) {
        if line_width <= Pt::ZERO || width <= Pt::ZERO || height <= Pt::ZERO {
            return;
        }
        canvas.save_state();
        canvas.clip_rect(clip_x, clip_y, clip_width, clip_height);
        canvas.set_stroke_color(color);
        canvas.set_line_width(line_width);
        Self::apply_outline_stroke_style(canvas, OutlineLineStyle::Solid, line_width);
        Self::draw_rounded_rect_corners_stroke(canvas, x, y, width, height, radius);
        canvas.restore_state();
    }

    fn draw_rounded_shaded_border_stroke(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        inset: Pt,
        line_width: Pt,
        top_left_color: Color,
        bottom_right_color: Color,
        radius: ResolvedClipPathRadii,
    ) {
        if line_width <= Pt::ZERO {
            return;
        }
        let stroke_width = (width - inset * 2.0).max(Pt::ZERO);
        let stroke_height = (height - inset * 2.0).max(Pt::ZERO);
        if stroke_width <= Pt::ZERO || stroke_height <= Pt::ZERO {
            return;
        }
        let stroke_x = x + inset;
        let stroke_y = y + inset;
        let stroke_radius = Self::inset_clip_radii(radius, inset);
        let bleed = line_width * 2.0;
        let split_x = x + width / 2.0;
        let split_y = y + height / 2.0;

        Self::draw_rounded_border_stroke_region(
            canvas,
            stroke_x,
            stroke_y,
            stroke_width,
            stroke_height,
            line_width,
            top_left_color,
            stroke_radius,
            x - bleed,
            y - bleed,
            width + bleed * 2.0,
            (height / 2.0) + bleed,
        );
        Self::draw_rounded_border_stroke_region(
            canvas,
            stroke_x,
            stroke_y,
            stroke_width,
            stroke_height,
            line_width,
            bottom_right_color,
            stroke_radius,
            x - bleed,
            split_y,
            width + bleed * 2.0,
            (height / 2.0) + bleed,
        );
        Self::draw_rounded_border_stroke_region(
            canvas,
            stroke_x,
            stroke_y,
            stroke_width,
            stroke_height,
            line_width,
            top_left_color,
            stroke_radius,
            x - bleed,
            y - bleed,
            (width / 2.0) + bleed,
            height + bleed * 2.0,
        );
        Self::draw_rounded_border_stroke_region(
            canvas,
            stroke_x,
            stroke_y,
            stroke_width,
            stroke_height,
            line_width,
            bottom_right_color,
            stroke_radius,
            split_x,
            y - bleed,
            (width / 2.0) + bleed,
            height + bleed * 2.0,
        );
    }

    fn draw_rounded_uniform_3d_border(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        border_width: Pt,
        color: Color,
        style: OutlineLineStyle,
        radius: ResolvedClipPathRadii,
    ) {
        if border_width <= Pt::ZERO {
            return;
        }
        match style {
            OutlineLineStyle::Inset | OutlineLineStyle::Outset => {
                let (top_left, bottom_right) = Self::outline_3d_edge_colors(style, color);
                Self::draw_rounded_shaded_border_stroke(
                    canvas,
                    x,
                    y,
                    width,
                    height,
                    border_width / 2.0,
                    border_width,
                    top_left,
                    bottom_right,
                    radius,
                );
            }
            OutlineLineStyle::Groove | OutlineLineStyle::Ridge => {
                let outer = border_width / 2.0;
                let inner = (border_width - outer).max(Pt::ZERO);
                if outer <= Pt::ZERO || inner <= Pt::ZERO {
                    return;
                }
                let (outer_top_left, outer_bottom_right) =
                    Self::outline_3d_edge_colors(style, color);
                let inner_style = match style {
                    OutlineLineStyle::Groove => OutlineLineStyle::Ridge,
                    OutlineLineStyle::Ridge => OutlineLineStyle::Groove,
                    _ => style,
                };
                let (inner_top_left, inner_bottom_right) =
                    Self::outline_3d_edge_colors(inner_style, color);
                Self::draw_rounded_shaded_border_stroke(
                    canvas,
                    x,
                    y,
                    width,
                    height,
                    outer / 2.0,
                    outer,
                    outer_top_left,
                    outer_bottom_right,
                    radius,
                );
                Self::draw_rounded_shaded_border_stroke(
                    canvas,
                    x,
                    y,
                    width,
                    height,
                    outer + inner / 2.0,
                    inner,
                    inner_top_left,
                    inner_bottom_right,
                    radius,
                );
            }
            _ => {}
        }
    }

    fn resolve_clip_path_inset_rect(
        &self,
        spec: ClipPathInsetSpec,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
    ) -> Option<(Pt, Pt, Pt, Pt)> {
        let top = spec
            .top
            .resolve_height(height, self.font_size, self.root_font_size)
            .max(Pt::ZERO)
            .min(height);
        let right = spec
            .right
            .resolve_width(width, self.font_size, self.root_font_size)
            .max(Pt::ZERO)
            .min(width);
        let bottom = spec
            .bottom
            .resolve_height(height, self.font_size, self.root_font_size)
            .max(Pt::ZERO)
            .min(height);
        let left = spec
            .left
            .resolve_width(width, self.font_size, self.root_font_size)
            .max(Pt::ZERO)
            .min(width);

        let clip_w = (width - left - right).max(Pt::ZERO);
        let clip_h = (height - top - bottom).max(Pt::ZERO);
        Some((x + left, y + top, clip_w, clip_h))
    }

    fn resolve_clip_path_circle(
        &self,
        spec: ClipPathCircleSpec,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
    ) -> (Pt, Pt, Pt) {
        let center_x = x + spec
            .center_x
            .resolve_width(width, self.font_size, self.root_font_size);
        let center_y =
            y + spec
                .center_y
                .resolve_height(height, self.font_size, self.root_font_size);
        let radius = match spec.radius {
            ClipPathShapeRadius::Length(LengthSpec::Percent(value)) => {
                let w = width.to_f32();
                let h = height.to_f32();
                Pt::from_f32(((w * w + h * h).sqrt() / std::f32::consts::SQRT_2) * value)
            }
            ClipPathShapeRadius::Length(other) => {
                other.resolve_width(width, self.font_size, self.root_font_size)
            }
            ClipPathShapeRadius::ClosestSide => {
                let left = (center_x - x).abs();
                let right = (x + width - center_x).abs();
                let top = (center_y - y).abs();
                let bottom = (y + height - center_y).abs();
                left.min(right).min(top).min(bottom)
            }
            ClipPathShapeRadius::FarthestSide => {
                let left = (center_x - x).abs();
                let right = (x + width - center_x).abs();
                let top = (center_y - y).abs();
                let bottom = (y + height - center_y).abs();
                left.max(right).max(top).max(bottom)
            }
            ClipPathShapeRadius::ClosestCorner => {
                Self::clip_path_corner_distance(center_x, center_y, x, y, width, height, false)
            }
            ClipPathShapeRadius::FarthestCorner => {
                Self::clip_path_corner_distance(center_x, center_y, x, y, width, height, true)
            }
        }
        .max(Pt::ZERO);
        (center_x, center_y, radius)
    }

    fn clip_path_corner_distance(
        center_x: Pt,
        center_y: Pt,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        farthest: bool,
    ) -> Pt {
        let corners = [
            (x, y),
            (x + width, y),
            (x + width, y + height),
            (x, y + height),
        ];
        let mut selected: Option<Pt> = None;
        for (corner_x, corner_y) in corners {
            let dx = (corner_x - center_x).to_f32();
            let dy = (corner_y - center_y).to_f32();
            let distance = Pt::from_f32((dx * dx + dy * dy).sqrt());
            selected = Some(match selected {
                Some(current) if farthest => current.max(distance),
                Some(current) => current.min(distance),
                None => distance,
            });
        }
        selected.unwrap_or(Pt::ZERO)
    }

    fn resolve_clip_path_ellipse(
        &self,
        spec: ClipPathEllipseSpec,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
    ) -> (Pt, Pt, Pt, Pt) {
        let center_x = x + spec
            .center_x
            .resolve_width(width, self.font_size, self.root_font_size);
        let center_y =
            y + spec
                .center_y
                .resolve_height(height, self.font_size, self.root_font_size);
        let radius_x = self
            .resolve_clip_path_ellipse_radius(spec.radius_x, center_x, x, width, true)
            .max(Pt::ZERO);
        let radius_y = self
            .resolve_clip_path_ellipse_radius(spec.radius_y, center_y, y, height, false)
            .max(Pt::ZERO);
        (center_x, center_y, radius_x, radius_y)
    }

    fn resolve_clip_path_ellipse_radius(
        &self,
        radius: ClipPathShapeRadius,
        center: Pt,
        origin: Pt,
        size: Pt,
        horizontal: bool,
    ) -> Pt {
        match radius {
            ClipPathShapeRadius::Length(length) => {
                if horizontal {
                    length.resolve_width(size, self.font_size, self.root_font_size)
                } else {
                    length.resolve_height(size, self.font_size, self.root_font_size)
                }
            }
            ClipPathShapeRadius::ClosestSide | ClipPathShapeRadius::ClosestCorner => {
                let start = (center - origin).abs();
                let end = (origin + size - center).abs();
                start.min(end)
            }
            ClipPathShapeRadius::FarthestSide | ClipPathShapeRadius::FarthestCorner => {
                let start = (center - origin).abs();
                let end = (origin + size - center).abs();
                start.max(end)
            }
        }
    }

    fn resolve_clip_path_xywh_rect(
        &self,
        spec: ClipPathXywhSpec,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
    ) -> (Pt, Pt, Pt, Pt) {
        let clip_x = x + spec
            .x
            .resolve_width(width, self.font_size, self.root_font_size);
        let clip_y = y + spec
            .y
            .resolve_height(height, self.font_size, self.root_font_size);
        let clip_w = spec
            .width
            .resolve_width(width, self.font_size, self.root_font_size)
            .max(Pt::ZERO);
        let clip_h = spec
            .height
            .resolve_height(height, self.font_size, self.root_font_size)
            .max(Pt::ZERO);
        (clip_x, clip_y, clip_w, clip_h)
    }

    fn resolve_clip_path_rect(
        &self,
        spec: ClipPathRectSpec,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
    ) -> (Pt, Pt, Pt, Pt) {
        let resolve_x = |value: LengthSpec, auto_value: Pt| -> Pt {
            match value {
                LengthSpec::Auto => auto_value,
                other => other.resolve_width(width, self.font_size, self.root_font_size),
            }
        };
        let resolve_y = |value: LengthSpec, auto_value: Pt| -> Pt {
            match value {
                LengthSpec::Auto => auto_value,
                other => other.resolve_height(height, self.font_size, self.root_font_size),
            }
        };
        let top = resolve_y(spec.top, Pt::ZERO);
        let left = resolve_x(spec.left, Pt::ZERO);
        let right = resolve_x(spec.right, width).max(left);
        let bottom = resolve_y(spec.bottom, height).max(top);
        (x + left, y + top, right - left, bottom - top)
    }

    fn resolve_clip_path_reference_rect(
        reference_box: ClipPathReferenceBox,
        margin_box_x: Pt,
        margin_box_y: Pt,
        margin: ResolvedEdges,
        border: ResolvedEdges,
        padding: ResolvedEdges,
        content_width: Pt,
        content_height: Pt,
        border_box_x: Pt,
        border_box_y: Pt,
        border_box_width: Pt,
        border_box_height: Pt,
    ) -> (Pt, Pt, Pt, Pt) {
        match reference_box {
            ClipPathReferenceBox::Margin => (
                margin_box_x,
                margin_box_y,
                (border_box_width + margin.left + margin.right).max(Pt::ZERO),
                (border_box_height + margin.top + margin.bottom).max(Pt::ZERO),
            ),
            ClipPathReferenceBox::Border => (
                border_box_x,
                border_box_y,
                border_box_width.max(Pt::ZERO),
                border_box_height.max(Pt::ZERO),
            ),
            ClipPathReferenceBox::HalfBorder => (
                border_box_x + border.left / 2.0,
                border_box_y + border.top / 2.0,
                (border_box_width - (border.left + border.right) / 2.0).max(Pt::ZERO),
                (border_box_height - (border.top + border.bottom) / 2.0).max(Pt::ZERO),
            ),
            ClipPathReferenceBox::Padding => (
                border_box_x + border.left,
                border_box_y + border.top,
                (border_box_width - border.left - border.right).max(Pt::ZERO),
                (border_box_height - border.top - border.bottom).max(Pt::ZERO),
            ),
            ClipPathReferenceBox::Content => (
                border_box_x + border.left + padding.left,
                border_box_y + border.top + padding.top,
                content_width.max(Pt::ZERO),
                content_height.max(Pt::ZERO),
            ),
        }
    }

    fn resolve_clip_path_reference_radii(
        reference_box: ClipPathReferenceBox,
        border_radius: ResolvedClipPathRadii,
        margin: ResolvedEdges,
        border: ResolvedEdges,
        padding: ResolvedEdges,
    ) -> ResolvedClipPathRadii {
        match reference_box {
            ClipPathReferenceBox::Margin => Self::outset_clip_radii_edges(border_radius, margin),
            ClipPathReferenceBox::Border => border_radius,
            ClipPathReferenceBox::HalfBorder => Self::inset_clip_radii_edges(
                border_radius,
                ResolvedEdges {
                    top: border.top / 2.0,
                    right: border.right / 2.0,
                    bottom: border.bottom / 2.0,
                    left: border.left / 2.0,
                },
            ),
            ClipPathReferenceBox::Padding => Self::inset_clip_radii_edges(border_radius, border),
            ClipPathReferenceBox::Content => Self::inset_clip_radii_edges(
                border_radius,
                ResolvedEdges {
                    top: border.top + padding.top,
                    right: border.right + padding.right,
                    bottom: border.bottom + padding.bottom,
                    left: border.left + padding.left,
                },
            ),
        }
    }

    fn apply_clip_path_reference_box(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: ResolvedClipPathRadii,
    ) {
        if Self::clip_radii_have_rounding(radius) {
            Self::rounded_rect_corners_path(canvas, x, y, width, height, radius);
            canvas.clip_path(false);
        } else {
            canvas.clip_rect(x, y, width, height);
        }
    }

    fn rounded_rect_corners_path(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: ResolvedClipPathRadii,
    ) {
        let mut top_left_x = radius.top_left_x.max(Pt::ZERO);
        let mut top_left_y = radius.top_left_y.max(Pt::ZERO);
        let mut top_right_x = radius.top_right_x.max(Pt::ZERO);
        let mut top_right_y = radius.top_right_y.max(Pt::ZERO);
        let mut bottom_right_x = radius.bottom_right_x.max(Pt::ZERO);
        let mut bottom_right_y = radius.bottom_right_y.max(Pt::ZERO);
        let mut bottom_left_x = radius.bottom_left_x.max(Pt::ZERO);
        let mut bottom_left_y = radius.bottom_left_y.max(Pt::ZERO);
        let mut scale = 1.0_f32;
        for (sum, side) in [
            (top_left_x + top_right_x, width),
            (bottom_left_x + bottom_right_x, width),
            (top_left_y + bottom_left_y, height),
            (top_right_y + bottom_right_y, height),
        ] {
            if sum > side && sum > Pt::ZERO {
                scale = scale.min(side.to_f32() / sum.to_f32());
            }
        }
        top_left_x = top_left_x * scale;
        top_left_y = top_left_y * scale;
        top_right_x = top_right_x * scale;
        top_right_y = top_right_y * scale;
        bottom_right_x = bottom_right_x * scale;
        bottom_right_y = bottom_right_y * scale;
        bottom_left_x = bottom_left_x * scale;
        bottom_left_y = bottom_left_y * scale;

        if top_left_x <= Pt::ZERO
            && top_left_y <= Pt::ZERO
            && top_right_x <= Pt::ZERO
            && top_right_y <= Pt::ZERO
            && bottom_right_x <= Pt::ZERO
            && bottom_right_y <= Pt::ZERO
            && bottom_left_x <= Pt::ZERO
            && bottom_left_y <= Pt::ZERO
        {
            canvas.draw_rect(x, y, width, height);
            return;
        }

        let k = 0.552_284_75;
        let right = x + width;
        let bottom = y + height;

        canvas.move_to(x + top_left_x, y);
        canvas.line_to(right - top_right_x, y);
        if top_right_x > Pt::ZERO && top_right_y > Pt::ZERO {
            let cx = top_right_x * k;
            let cy = top_right_y * k;
            canvas.curve_to(
                right - top_right_x + cx,
                y,
                right,
                y + top_right_y - cy,
                right,
                y + top_right_y,
            );
        } else {
            canvas.line_to(right, y);
        }

        canvas.line_to(right, bottom - bottom_right_y);
        if bottom_right_x > Pt::ZERO && bottom_right_y > Pt::ZERO {
            let cx = bottom_right_x * k;
            let cy = bottom_right_y * k;
            canvas.curve_to(
                right,
                bottom - bottom_right_y + cy,
                right - bottom_right_x + cx,
                bottom,
                right - bottom_right_x,
                bottom,
            );
        } else {
            canvas.line_to(right, bottom);
        }

        canvas.line_to(x + bottom_left_x, bottom);
        if bottom_left_x > Pt::ZERO && bottom_left_y > Pt::ZERO {
            let cx = bottom_left_x * k;
            let cy = bottom_left_y * k;
            canvas.curve_to(
                x + bottom_left_x - cx,
                bottom,
                x,
                bottom - bottom_left_y + cy,
                x,
                bottom - bottom_left_y,
            );
        } else {
            canvas.line_to(x, bottom);
        }

        canvas.line_to(x, y + top_left_y);
        if top_left_x > Pt::ZERO && top_left_y > Pt::ZERO {
            let cx = top_left_x * k;
            let cy = top_left_y * k;
            canvas.curve_to(
                x,
                y + top_left_y - cy,
                x + top_left_x - cx,
                y,
                x + top_left_x,
                y,
            );
        } else {
            canvas.line_to(x, y);
        }
        canvas.close_path();
    }

    fn resolve_clip_path_radius(
        &self,
        radius: ClipPathRadiusSpec,
        width: Pt,
        height: Pt,
    ) -> ResolvedClipPathRadii {
        let horizontal = radius
            .horizontal
            .resolve(width, self.font_size, self.root_font_size);
        let vertical = radius
            .vertical
            .resolve(height, self.font_size, self.root_font_size);
        ResolvedClipPathRadii {
            top_left_x: horizontal.top_left,
            top_left_y: vertical.top_left,
            top_right_x: horizontal.top_right,
            top_right_y: vertical.top_right,
            bottom_right_x: horizontal.bottom_right,
            bottom_right_y: vertical.bottom_right,
            bottom_left_x: horizontal.bottom_left,
            bottom_left_y: vertical.bottom_left,
        }
    }

    fn apply_clip_rect_or_rounded(
        &self,
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: Option<ClipPathRadiusSpec>,
    ) {
        if let Some(radius) = radius {
            let radius = self.resolve_clip_path_radius(radius, width, height);
            Self::rounded_rect_corners_path(canvas, x, y, width, height, radius);
            canvas.clip_path(false);
        } else {
            canvas.clip_rect(x, y, width, height);
        }
    }

    fn ellipse_path(canvas: &mut Canvas, cx: Pt, cy: Pt, rx: Pt, ry: Pt) {
        let k = 0.552_284_8;
        let ox = rx * k;
        let oy = ry * k;
        canvas.move_to(cx + rx, cy);
        canvas.curve_to(cx + rx, cy + oy, cx + ox, cy + ry, cx, cy + ry);
        canvas.curve_to(cx - ox, cy + ry, cx - rx, cy + oy, cx - rx, cy);
        canvas.curve_to(cx - rx, cy - oy, cx - ox, cy - ry, cx, cy - ry);
        canvas.curve_to(cx + ox, cy - ry, cx + rx, cy - oy, cx + rx, cy);
        canvas.close_path();
    }

    fn polygon_path(
        &self,
        canvas: &mut Canvas,
        spec: &ClipPathPolygonSpec,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
    ) -> bool {
        let Some((first_x, first_y)) = spec.points.first().copied() else {
            return false;
        };
        let resolve = |point: (LengthSpec, LengthSpec)| -> (Pt, Pt) {
            (
                x + point
                    .0
                    .resolve_width(width, self.font_size, self.root_font_size),
                y + point
                    .1
                    .resolve_height(height, self.font_size, self.root_font_size),
            )
        };
        let (start_x, start_y) = resolve((first_x, first_y));
        canvas.move_to(start_x, start_y);
        for point in spec.points.iter().skip(1).copied() {
            let (px, py) = resolve(point);
            canvas.line_to(px, py);
        }
        canvas.close_path();
        true
    }

    fn css_path(&self, canvas: &mut Canvas, spec: &ClipPathPathSpec, x: Pt, y: Pt) -> bool {
        if spec.commands.is_empty() {
            return false;
        }
        for command in &spec.commands {
            match *command {
                ClipPathPathCommand::MoveTo { x: px, y: py } => canvas.move_to(x + px, y + py),
                ClipPathPathCommand::LineTo { x: px, y: py } => canvas.line_to(x + px, y + py),
                ClipPathPathCommand::CurveTo {
                    x1,
                    y1,
                    x2,
                    y2,
                    x: px,
                    y: py,
                } => canvas.curve_to(x + x1, y + y1, x + x2, y + y2, x + px, y + py),
                ClipPathPathCommand::Close => canvas.close_path(),
            }
        }
        true
    }

    fn css_shape_function_path(
        &self,
        canvas: &mut Canvas,
        spec: &ClipPathShapeFunctionSpec,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
    ) -> bool {
        if spec.commands.is_empty() {
            return false;
        }
        let resolve_x =
            |value: LengthSpec| value.resolve_width(width, self.font_size, self.root_font_size);
        let resolve_y =
            |value: LengthSpec| value.resolve_height(height, self.font_size, self.root_font_size);
        let resolve_control = |control_x: LengthSpec,
                               control_y: LengthSpec,
                               anchor: ClipPathShapeControlAnchor,
                               start_x: Pt,
                               start_y: Pt,
                               end_x: Pt,
                               end_y: Pt| {
            let offset_x = resolve_x(control_x);
            let offset_y = resolve_y(control_y);
            match anchor {
                ClipPathShapeControlAnchor::Start => (start_x + offset_x, start_y + offset_y),
                ClipPathShapeControlAnchor::End => (end_x + offset_x, end_y + offset_y),
                ClipPathShapeControlAnchor::Origin => (offset_x, offset_y),
            }
        };
        let mut current_x = resolve_x(spec.start_x);
        let mut current_y = resolve_y(spec.start_y);
        let mut subpath_x = current_x;
        let mut subpath_y = current_y;
        let mut last_control: Option<(Pt, Pt)> = None;
        canvas.move_to(x + current_x, y + current_y);
        for command in &spec.commands {
            match *command {
                ClipPathShapeFunctionCommand::MoveTo {
                    x: px,
                    y: py,
                    relative,
                } => {
                    if relative {
                        current_x += resolve_x(px);
                        current_y += resolve_y(py);
                    } else {
                        current_x = resolve_x(px);
                        current_y = resolve_y(py);
                    }
                    subpath_x = current_x;
                    subpath_y = current_y;
                    last_control = None;
                    canvas.move_to(x + current_x, y + current_y);
                }
                ClipPathShapeFunctionCommand::LineTo {
                    x: px,
                    y: py,
                    relative,
                } => {
                    if relative {
                        current_x += resolve_x(px);
                        current_y += resolve_y(py);
                    } else {
                        current_x = resolve_x(px);
                        current_y = resolve_y(py);
                    }
                    last_control = None;
                    canvas.line_to(x + current_x, y + current_y);
                }
                ClipPathShapeFunctionCommand::HLine { x: px, relative } => {
                    if relative {
                        current_x += resolve_x(px);
                    } else {
                        current_x = resolve_x(px);
                    }
                    last_control = None;
                    canvas.line_to(x + current_x, y + current_y);
                }
                ClipPathShapeFunctionCommand::VLine { y: py, relative } => {
                    if relative {
                        current_y += resolve_y(py);
                    } else {
                        current_y = resolve_y(py);
                    }
                    last_control = None;
                    canvas.line_to(x + current_x, y + current_y);
                }
                ClipPathShapeFunctionCommand::CurveTo {
                    x: px,
                    y: py,
                    relative,
                    control1_x,
                    control1_y,
                    control1_anchor,
                    control2_x,
                    control2_y,
                    control2_anchor,
                } => {
                    let start_x = current_x;
                    let start_y = current_y;
                    let end_x = if relative {
                        current_x + resolve_x(px)
                    } else {
                        resolve_x(px)
                    };
                    let end_y = if relative {
                        current_y + resolve_y(py)
                    } else {
                        resolve_y(py)
                    };
                    let (c1x, c1y) = resolve_control(
                        control1_x,
                        control1_y,
                        control1_anchor,
                        start_x,
                        start_y,
                        end_x,
                        end_y,
                    );
                    let (cubic_c1x, cubic_c1y, cubic_c2x, cubic_c2y) =
                        if let (Some(control2_x), Some(control2_y), Some(control2_anchor)) =
                            (control2_x, control2_y, control2_anchor)
                        {
                            let (c2x, c2y) = resolve_control(
                                control2_x,
                                control2_y,
                                control2_anchor,
                                start_x,
                                start_y,
                                end_x,
                                end_y,
                            );
                            (c1x, c1y, c2x, c2y)
                        } else {
                            (
                                start_x + (c1x - start_x) * (2.0 / 3.0),
                                start_y + (c1y - start_y) * (2.0 / 3.0),
                                end_x + (c1x - end_x) * (2.0 / 3.0),
                                end_y + (c1y - end_y) * (2.0 / 3.0),
                            )
                        };
                    canvas.curve_to(
                        x + cubic_c1x,
                        y + cubic_c1y,
                        x + cubic_c2x,
                        y + cubic_c2y,
                        x + end_x,
                        y + end_y,
                    );
                    last_control = if control2_x.is_some()
                        && control2_y.is_some()
                        && control2_anchor.is_some()
                    {
                        Some((cubic_c2x, cubic_c2y))
                    } else {
                        Some((c1x, c1y))
                    };
                    current_x = end_x;
                    current_y = end_y;
                }
                ClipPathShapeFunctionCommand::SmoothTo {
                    x: px,
                    y: py,
                    relative,
                    control_x,
                    control_y,
                    control_anchor,
                } => {
                    let start_x = current_x;
                    let start_y = current_y;
                    let end_x = if relative {
                        current_x + resolve_x(px)
                    } else {
                        resolve_x(px)
                    };
                    let end_y = if relative {
                        current_y + resolve_y(py)
                    } else {
                        resolve_y(py)
                    };
                    let reflected = last_control
                        .map(|(control_x, control_y)| {
                            (
                                start_x + (start_x - control_x),
                                start_y + (start_y - control_y),
                            )
                        })
                        .unwrap_or((start_x, start_y));
                    if let (Some(control_x), Some(control_y), Some(control_anchor)) =
                        (control_x, control_y, control_anchor)
                    {
                        let (c2x, c2y) = resolve_control(
                            control_x,
                            control_y,
                            control_anchor,
                            start_x,
                            start_y,
                            end_x,
                            end_y,
                        );
                        canvas.curve_to(
                            x + reflected.0,
                            y + reflected.1,
                            x + c2x,
                            y + c2y,
                            x + end_x,
                            y + end_y,
                        );
                        last_control = Some((c2x, c2y));
                    } else {
                        let c1x = start_x + (reflected.0 - start_x) * (2.0 / 3.0);
                        let c1y = start_y + (reflected.1 - start_y) * (2.0 / 3.0);
                        let c2x = end_x + (reflected.0 - end_x) * (2.0 / 3.0);
                        let c2y = end_y + (reflected.1 - end_y) * (2.0 / 3.0);
                        canvas.curve_to(x + c1x, y + c1y, x + c2x, y + c2y, x + end_x, y + end_y);
                        last_control = Some(reflected);
                    }
                    current_x = end_x;
                    current_y = end_y;
                }
                ClipPathShapeFunctionCommand::ArcTo {
                    x: px,
                    y: py,
                    relative,
                    radius_x,
                    radius_y,
                    large_arc,
                    sweep,
                    rotation_deg,
                } => {
                    let start_x = current_x;
                    let start_y = current_y;
                    let end_x = if relative {
                        current_x + resolve_x(px)
                    } else {
                        resolve_x(px)
                    };
                    let end_y = if relative {
                        current_y + resolve_y(py)
                    } else {
                        resolve_y(py)
                    };
                    let radius_x = resolve_x(radius_x).to_f32().abs();
                    let radius_y = resolve_y(radius_y).to_f32().abs();
                    for segment in svg::svg_arc_to_cubic_segments(
                        start_x.to_f32(),
                        start_y.to_f32(),
                        radius_x,
                        radius_y,
                        rotation_deg,
                        large_arc,
                        sweep,
                        end_x.to_f32(),
                        end_y.to_f32(),
                    ) {
                        match segment {
                            svg::SvgPathSegment::MoveTo(_, _) => {}
                            svg::SvgPathSegment::LineTo(px, py) => {
                                canvas.line_to(x + Pt::from_f32(px), y + Pt::from_f32(py));
                            }
                            svg::SvgPathSegment::CurveTo(x1, y1, x2, y2, px, py) => {
                                canvas.curve_to(
                                    x + Pt::from_f32(x1),
                                    y + Pt::from_f32(y1),
                                    x + Pt::from_f32(x2),
                                    y + Pt::from_f32(y2),
                                    x + Pt::from_f32(px),
                                    y + Pt::from_f32(py),
                                );
                            }
                            svg::SvgPathSegment::Close => canvas.close_path(),
                        }
                    }
                    last_control = None;
                    current_x = end_x;
                    current_y = end_y;
                }
                ClipPathShapeFunctionCommand::Close => {
                    canvas.close_path();
                    current_x = subpath_x;
                    current_y = subpath_y;
                    last_control = None;
                }
            }
        }
        true
    }

    fn draw_gradient_background(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: Pt,
        paint: &BackgroundPaint,
    ) {
        match paint {
            BackgroundPaint::Image { .. } => {}
            BackgroundPaint::LinearGradient { angle_deg, stops } => {
                Self::draw_linear_gradient_background(
                    canvas, x, y, width, height, radius, *angle_deg, stops,
                );
            }
            BackgroundPaint::RadialGradient {
                center_x_pct,
                center_y_pct,
                stops,
            } => {
                Self::draw_radial_gradient_background(
                    canvas,
                    x,
                    y,
                    width,
                    height,
                    radius,
                    *center_x_pct,
                    *center_y_pct,
                    stops,
                );
            }
            BackgroundPaint::ConicGradient {
                start_angle_deg,
                center_x_pct,
                center_y_pct,
                stops,
            } => {
                Self::draw_conic_gradient_background(
                    canvas,
                    x,
                    y,
                    width,
                    height,
                    radius,
                    *start_angle_deg,
                    *center_x_pct,
                    *center_y_pct,
                    stops,
                );
            }
        }
    }

    fn draw_background_paint(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: Pt,
        paint: &BackgroundPaint,
    ) {
        match paint {
            BackgroundPaint::Image { source } => {
                canvas.draw_image(x, y, width, height, source.clone());
            }
            _ => {
                Self::draw_gradient_background(canvas, x, y, width, height, radius, paint);
            }
        }
    }

    fn draw_linear_gradient_background(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: Pt,
        angle_deg: f32,
        stops: &[ShadingStop],
    ) {
        if stops.len() < 2 {
            return;
        }
        let rad = angle_deg.to_radians();
        let dx = rad.sin();
        let dy = -rad.cos();
        let w = width.to_f32();
        let h = height.to_f32();
        let proj = (w.abs() * dx.abs() + h.abs() * dy.abs()) * 0.5;
        let cx = x.to_f32() + w * 0.5;
        let cy = y.to_f32() + h * 0.5;
        let shading = Shading::Axial {
            x0: cx - dx * proj,
            y0: cy - dy * proj,
            x1: cx + dx * proj,
            y1: cy + dy * proj,
            stops: stops.to_vec(),
        };
        canvas.save_state();
        if radius > Pt::ZERO {
            Self::rounded_rect_path(canvas, x, y, width, height, radius);
            canvas.clip_path(false);
        } else {
            canvas.clip_rect(x, y, width, height);
        }
        canvas.shading_fill(shading);
        canvas.restore_state();
    }

    fn draw_radial_gradient_background(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: Pt,
        center_x_pct: f32,
        center_y_pct: f32,
        stops: &[ShadingStop],
    ) {
        if stops.len() < 2 {
            return;
        }
        let w = width.to_f32().max(0.0);
        let h = height.to_f32().max(0.0);
        if w <= 0.0 || h <= 0.0 {
            return;
        }

        let cx = x.to_f32() + w * center_x_pct.clamp(0.0, 1.0);
        let cy = y.to_f32() + h * center_y_pct.clamp(0.0, 1.0);
        let corners = [
            (x.to_f32(), y.to_f32()),
            (x.to_f32() + w, y.to_f32()),
            (x.to_f32(), y.to_f32() + h),
            (x.to_f32() + w, y.to_f32() + h),
        ];
        let mut max_dist2 = 0.0f32;
        for (px, py) in corners {
            let dx = px - cx;
            let dy = py - cy;
            let dist2 = dx * dx + dy * dy;
            if dist2 > max_dist2 {
                max_dist2 = dist2;
            }
        }
        let r1 = max_dist2.sqrt().max(1.0);
        let shading = Shading::Radial {
            x0: cx,
            y0: cy,
            r0: 0.0,
            x1: cx,
            y1: cy,
            r1,
            stops: stops.to_vec(),
        };

        canvas.save_state();
        if radius > Pt::ZERO {
            Self::rounded_rect_path(canvas, x, y, width, height, radius);
            canvas.clip_path(false);
        } else {
            canvas.clip_rect(x, y, width, height);
        }
        canvas.shading_fill(shading);
        canvas.restore_state();
    }

    fn draw_conic_gradient_background(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: Pt,
        start_angle_deg: f32,
        center_x_pct: f32,
        center_y_pct: f32,
        stops: &[ShadingStop],
    ) {
        if stops.len() < 2 {
            return;
        }
        let w = width.to_f32().max(0.0);
        let h = height.to_f32().max(0.0);
        if w <= 0.0 || h <= 0.0 {
            return;
        }

        let cx = x.to_f32() + w * center_x_pct.clamp(0.0, 1.0);
        let cy = y.to_f32() + h * center_y_pct.clamp(0.0, 1.0);

        let corners = [
            (x.to_f32(), y.to_f32()),
            (x.to_f32() + w, y.to_f32()),
            (x.to_f32(), y.to_f32() + h),
            (x.to_f32() + w, y.to_f32() + h),
        ];
        let mut max_dist2 = 0.0f32;
        for (px, py) in corners {
            let dx = px - cx;
            let dy = py - cy;
            let dist2 = dx * dx + dy * dy;
            if dist2 > max_dist2 {
                max_dist2 = dist2;
            }
        }
        let radius_px = max_dist2.sqrt().max(1.0) + 1.0;
        let steps = ((radius_px * std::f32::consts::TAU) / 2.0)
            .round()
            .clamp(128.0, 720.0) as usize;
        let step_deg = 360.0 / steps as f32;
        let overlap_deg = step_deg * 0.4;

        canvas.save_state();
        if radius > Pt::ZERO {
            Self::rounded_rect_path(canvas, x, y, width, height, radius);
            canvas.clip_path(false);
        } else {
            canvas.clip_rect(x, y, width, height);
        }

        for idx in 0..steps {
            let t0 = idx as f32 / steps as f32;
            let t1 = (idx + 1) as f32 / steps as f32;
            let tm = (t0 + t1) * 0.5;
            let color = Self::sample_gradient_color(stops, tm);
            canvas.set_fill_color(color);
            let a0 = (start_angle_deg + t0 * 360.0 - overlap_deg).to_radians();
            let a1 = (start_angle_deg + t1 * 360.0 + overlap_deg).to_radians();
            let p0x = cx + a0.sin() * radius_px;
            let p0y = cy - a0.cos() * radius_px;
            let p1x = cx + a1.sin() * radius_px;
            let p1y = cy - a1.cos() * radius_px;
            canvas.move_to(Pt::from_f32(cx), Pt::from_f32(cy));
            canvas.line_to(Pt::from_f32(p0x), Pt::from_f32(p0y));
            canvas.line_to(Pt::from_f32(p1x), Pt::from_f32(p1y));
            canvas.close_path();
            canvas.fill();
        }
        canvas.restore_state();
    }

    fn sample_gradient_color(stops: &[ShadingStop], t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        if t <= stops[0].offset {
            return stops[0].color;
        }
        for window in stops.windows(2) {
            let a = window[0];
            let b = window[1];
            if t > b.offset {
                continue;
            }
            let span = (b.offset - a.offset).max(1.0e-6);
            let alpha = ((t - a.offset) / span).clamp(0.0, 1.0);
            return Self::lerp_color(a.color, b.color, alpha);
        }
        stops[stops.len() - 1].color
    }

    fn lerp_color(a: Color, b: Color, alpha: f32) -> Color {
        let alpha = alpha.clamp(0.0, 1.0);
        Color::rgb(
            a.r + (b.r - a.r) * alpha,
            a.g + (b.g - a.g) * alpha,
            a.b + (b.b - a.b) * alpha,
        )
    }

    fn apply_paint_filter_color(&self, color: Color) -> Color {
        let Some(filter) = self.paint_filter.as_ref() else {
            return color;
        };
        let saturate = filter.saturate.max(0.0);
        let brightness = filter.brightness.max(0.0);
        let contrast = filter.contrast.max(0.0);
        let invert = filter.invert.clamp(0.0, 1.0);
        let sepia = filter.sepia.clamp(0.0, 1.0);
        let hue_rotate = filter.hue_rotate;
        if (saturate - 1.0).abs() <= 1.0e-6
            && (brightness - 1.0).abs() <= 1.0e-6
            && (contrast - 1.0).abs() <= 1.0e-6
            && invert <= 1.0e-6
            && sepia <= 1.0e-6
            && hue_rotate.abs() <= 1.0e-6
        {
            return color;
        }
        let luma = 0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b;
        let mut r = ((luma + (color.r - luma) * saturate - 0.5) * contrast) + 0.5;
        let mut g = ((luma + (color.g - luma) * saturate - 0.5) * contrast) + 0.5;
        let mut b = ((luma + (color.b - luma) * saturate - 0.5) * contrast) + 0.5;
        if hue_rotate.abs() > 1.0e-6 {
            let cos = hue_rotate.cos();
            let sin = hue_rotate.sin();
            let hue_r = r * (0.213 + cos * 0.787 - sin * 0.213)
                + g * (0.715 - cos * 0.715 - sin * 0.715)
                + b * (0.072 - cos * 0.072 + sin * 0.928);
            let hue_g = r * (0.213 - cos * 0.213 + sin * 0.143)
                + g * (0.715 + cos * 0.285 + sin * 0.140)
                + b * (0.072 - cos * 0.072 - sin * 0.283);
            let hue_b = r * (0.213 - cos * 0.213 - sin * 0.787)
                + g * (0.715 - cos * 0.715 + sin * 0.715)
                + b * (0.072 + cos * 0.928 + sin * 0.072);
            r = hue_r;
            g = hue_g;
            b = hue_b;
        }
        if invert > 1.0e-6 {
            r = r * (1.0 - invert) + (1.0 - r) * invert;
            g = g * (1.0 - invert) + (1.0 - g) * invert;
            b = b * (1.0 - invert) + (1.0 - b) * invert;
        }
        if sepia > 1.0e-6 {
            let sepia_r = r * 0.393 + g * 0.769 + b * 0.189;
            let sepia_g = r * 0.349 + g * 0.686 + b * 0.168;
            let sepia_b = r * 0.272 + g * 0.534 + b * 0.131;
            r = r * (1.0 - sepia) + sepia_r * sepia;
            g = g * (1.0 - sepia) + sepia_g * sepia;
            b = b * (1.0 - sepia) + sepia_b * sepia;
        }
        Color::rgb(
            (r * brightness).clamp(0.0, 1.0),
            (g * brightness).clamp(0.0, 1.0),
            (b * brightness).clamp(0.0, 1.0),
        )
    }

    fn apply_paint_filter_stops(&self, stops: &[ShadingStop]) -> Vec<ShadingStop> {
        stops
            .iter()
            .map(|stop| ShadingStop {
                offset: stop.offset,
                color: self.apply_paint_filter_color(stop.color),
            })
            .collect()
    }

    fn filtered_background_paint(&self, paint: &BackgroundPaint) -> BackgroundPaint {
        match paint {
            BackgroundPaint::Image { source } => BackgroundPaint::Image {
                source: source.clone(),
            },
            BackgroundPaint::LinearGradient { angle_deg, stops } => {
                BackgroundPaint::LinearGradient {
                    angle_deg: *angle_deg,
                    stops: self.apply_paint_filter_stops(stops),
                }
            }
            BackgroundPaint::RadialGradient {
                center_x_pct,
                center_y_pct,
                stops,
            } => BackgroundPaint::RadialGradient {
                center_x_pct: *center_x_pct,
                center_y_pct: *center_y_pct,
                stops: self.apply_paint_filter_stops(stops),
            },
            BackgroundPaint::ConicGradient {
                start_angle_deg,
                center_x_pct,
                center_y_pct,
                stops,
            } => BackgroundPaint::ConicGradient {
                start_angle_deg: *start_angle_deg,
                center_x_pct: *center_x_pct,
                center_y_pct: *center_y_pct,
                stops: self.apply_paint_filter_stops(stops),
            },
        }
    }

    fn background_layer_value<T: Copy + Default>(values: &[T], idx: usize) -> T {
        if values.is_empty() {
            T::default()
        } else {
            values[idx % values.len()]
        }
    }

    fn background_box_rect(
        box_kind: BackgroundBox,
        border_box_x: Pt,
        border_box_y: Pt,
        border_box_width: Pt,
        border_box_height: Pt,
        border: ResolvedEdges,
        padding: ResolvedEdges,
        content_width: Pt,
        content_height: Pt,
    ) -> (Pt, Pt, Pt, Pt) {
        match box_kind {
            BackgroundBox::Border => (
                border_box_x,
                border_box_y,
                border_box_width.max(Pt::ZERO),
                border_box_height.max(Pt::ZERO),
            ),
            BackgroundBox::Padding => (
                border_box_x + border.left,
                border_box_y + border.top,
                (border_box_width - border.left - border.right).max(Pt::ZERO),
                (border_box_height - border.top - border.bottom).max(Pt::ZERO),
            ),
            BackgroundBox::Content => (
                border_box_x + border.left + padding.left,
                border_box_y + border.top + padding.top,
                content_width.max(Pt::ZERO),
                content_height.max(Pt::ZERO),
            ),
        }
    }

    fn background_clip_rect(
        clip: BackgroundClipBox,
        border_box_x: Pt,
        border_box_y: Pt,
        border_box_width: Pt,
        border_box_height: Pt,
        border: ResolvedEdges,
        padding: ResolvedEdges,
        content_width: Pt,
        content_height: Pt,
    ) -> (Pt, Pt, Pt, Pt) {
        let box_kind = match clip {
            BackgroundClipBox::Border => BackgroundBox::Border,
            BackgroundClipBox::Padding => BackgroundBox::Padding,
            BackgroundClipBox::Content => BackgroundBox::Content,
        };
        Self::background_box_rect(
            box_kind,
            border_box_x,
            border_box_y,
            border_box_width,
            border_box_height,
            border,
            padding,
            content_width,
            content_height,
        )
    }

    fn resolve_background_layer_size(
        &self,
        paint: &BackgroundPaint,
        size: BackgroundSizeSpec,
        box_width: Pt,
        box_height: Pt,
    ) -> (Pt, Pt) {
        let intrinsic = Self::background_paint_intrinsic_size(paint);
        let intrinsic_ratio = intrinsic.and_then(|(width, height)| {
            let w = width.to_f32();
            let h = height.to_f32();
            (w > 0.0 && h > 0.0).then_some(w / h)
        });

        match size.mode {
            BackgroundSizeMode::Contain | BackgroundSizeMode::Cover => {
                if let Some((intrinsic_width, intrinsic_height)) = intrinsic {
                    let iw = intrinsic_width.to_f32();
                    let ih = intrinsic_height.to_f32();
                    let bw = box_width.to_f32();
                    let bh = box_height.to_f32();
                    if iw > 0.0 && ih > 0.0 && bw > 0.0 && bh > 0.0 {
                        let scale_x = bw / iw;
                        let scale_y = bh / ih;
                        let scale = if size.mode == BackgroundSizeMode::Contain {
                            scale_x.min(scale_y)
                        } else {
                            scale_x.max(scale_y)
                        };
                        return (
                            Pt::from_f32(iw * scale).max(Pt::ZERO),
                            Pt::from_f32(ih * scale).max(Pt::ZERO),
                        );
                    }
                }
                return (box_width.max(Pt::ZERO), box_height.max(Pt::ZERO));
            }
            BackgroundSizeMode::Explicit => {}
        }

        let width_auto = matches!(
            size.width,
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
        );
        let height_auto = matches!(
            size.height,
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
        );

        let explicit_width = (!width_auto).then(|| {
            size.width
                .resolve_width(box_width, self.font_size, self.root_font_size)
                .max(Pt::ZERO)
        });
        let explicit_height = (!height_auto).then(|| {
            size.height
                .resolve_height(box_height, self.font_size, self.root_font_size)
                .max(Pt::ZERO)
        });

        match (explicit_width, explicit_height) {
            (Some(width), Some(height)) => (width, height),
            (Some(width), None) => {
                let height = intrinsic_ratio
                    .filter(|ratio| *ratio > 0.0)
                    .map(|ratio| Pt::from_f32(width.to_f32() / ratio))
                    .or_else(|| intrinsic.map(|(_, height)| height))
                    .unwrap_or(box_height)
                    .max(Pt::ZERO);
                (width, height)
            }
            (None, Some(height)) => {
                let width = intrinsic_ratio
                    .filter(|ratio| *ratio > 0.0)
                    .map(|ratio| Pt::from_f32(height.to_f32() * ratio))
                    .or_else(|| intrinsic.map(|(width, _)| width))
                    .unwrap_or(box_width)
                    .max(Pt::ZERO);
                (width, height)
            }
            (None, None) => intrinsic.unwrap_or((box_width, box_height)),
        }
    }

    fn background_paint_intrinsic_size(paint: &BackgroundPaint) -> Option<(Pt, Pt)> {
        match paint {
            BackgroundPaint::Image { source } => image_intrinsic_size_pt(source),
            _ => None,
        }
    }

    fn resolve_background_position_component(
        &self,
        component: BackgroundPositionComponent,
        area: Pt,
        layer: Pt,
    ) -> Pt {
        match component {
            BackgroundPositionComponent::Start(offset) => {
                self.resolve_background_position_offset(offset, area, layer)
            }
            BackgroundPositionComponent::Center => (area - layer) / 2.0,
            BackgroundPositionComponent::End(offset) => {
                let resolved = self.resolve_background_position_offset(offset, area, layer);
                area - layer - resolved
            }
        }
    }

    fn resolve_background_position_offset(&self, offset: LengthSpec, area: Pt, layer: Pt) -> Pt {
        let percent_basis = area - layer;
        match offset {
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial => Pt::ZERO,
            LengthSpec::Absolute(value) => value,
            LengthSpec::Percent(value) => percent_basis * value,
            LengthSpec::Em(value) => self.font_size * value,
            LengthSpec::Rem(value) => self.root_font_size * value,
            LengthSpec::Calc(calc) => {
                calc.abs
                    + (percent_basis * calc.percent)
                    + (self.font_size * calc.em)
                    + (self.root_font_size * calc.rem)
            }
        }
    }

    fn background_axis_tiles(
        mode: BackgroundRepeatMode,
        offset: f32,
        area: f32,
        tile: f32,
    ) -> (Vec<f32>, f32) {
        if area <= 0.0 || tile <= 0.0 {
            return (Vec::new(), tile.max(0.01));
        }
        let tile = tile.max(0.01);
        match mode {
            BackgroundRepeatMode::NoRepeat => (vec![offset], tile),
            BackgroundRepeatMode::Repeat => {
                let mut start = offset;
                while start > 0.0 {
                    start -= tile;
                }
                while start + tile <= 0.0 {
                    start += tile;
                }
                let mut positions = Vec::new();
                let mut current = start;
                let mut guard = 0usize;
                while current < area && guard <= 10_000 {
                    positions.push(current);
                    current += tile;
                    guard += 1;
                }
                (positions, tile)
            }
            BackgroundRepeatMode::Space => {
                let count = (area / tile).floor() as usize;
                if count <= 1 {
                    return (vec![offset], tile);
                }
                let gap = (area - (count as f32 * tile)) / (count.saturating_sub(1) as f32);
                let mut positions = Vec::with_capacity(count);
                for idx in 0..count {
                    positions.push(idx as f32 * (tile + gap));
                }
                (positions, tile)
            }
            BackgroundRepeatMode::Round => {
                let count = (area / tile).round().max(1.0) as usize;
                let adjusted_tile = area / count as f32;
                let mut positions = Vec::with_capacity(count);
                for idx in 0..count {
                    positions.push(idx as f32 * adjusted_tile);
                }
                (positions, adjusted_tile)
            }
        }
    }

    fn draw_background_layer(
        &self,
        canvas: &mut Canvas,
        border_box_x: Pt,
        border_box_y: Pt,
        border_box_width: Pt,
        border_box_height: Pt,
        border: ResolvedEdges,
        padding: ResolvedEdges,
        content_width: Pt,
        content_height: Pt,
        radius: Pt,
        paint: &BackgroundPaint,
        size: BackgroundSizeSpec,
        position: BackgroundPositionSpec,
        repeat: BackgroundRepeatSpec,
        blend_mode: MixBlendMode,
        origin: BackgroundBox,
        clip: BackgroundClipBox,
    ) {
        let (origin_x, origin_y, origin_width, origin_height) = Self::background_box_rect(
            origin,
            border_box_x,
            border_box_y,
            border_box_width,
            border_box_height,
            border,
            padding,
            content_width,
            content_height,
        );
        let (clip_x, clip_y, clip_width, clip_height) = Self::background_clip_rect(
            clip,
            border_box_x,
            border_box_y,
            border_box_width,
            border_box_height,
            border,
            padding,
            content_width,
            content_height,
        );
        let (layer_width, layer_height) =
            self.resolve_background_layer_size(paint, size, origin_width, origin_height);
        let offset_x =
            self.resolve_background_position_component(position.x, origin_width, layer_width);
        let offset_y =
            self.resolve_background_position_component(position.y, origin_height, layer_height);
        if layer_width <= Pt::ZERO || layer_height <= Pt::ZERO {
            return;
        }

        let box_width = origin_width.to_f32().max(0.0);
        let box_height = origin_height.to_f32().max(0.0);
        let (tile_x_positions, tile_width) = Self::background_axis_tiles(
            repeat.x,
            offset_x.to_f32(),
            box_width,
            layer_width.to_f32(),
        );
        let (tile_y_positions, tile_height) = Self::background_axis_tiles(
            repeat.y,
            offset_y.to_f32(),
            box_height,
            layer_height.to_f32(),
        );
        if tile_x_positions.is_empty() || tile_y_positions.is_empty() {
            return;
        }
        let layer_width = Pt::from_f32(tile_width);
        let layer_height = Pt::from_f32(tile_height);

        canvas.save_state();
        if clip == BackgroundClipBox::Border && radius > Pt::ZERO {
            Self::rounded_rect_path(canvas, clip_x, clip_y, clip_width, clip_height, radius);
            canvas.clip_path(false);
        } else {
            canvas.clip_rect(clip_x, clip_y, clip_width, clip_height);
        }
        if blend_mode != MixBlendMode::Normal {
            canvas.set_blend_mode(blend_mode);
        }

        for tile_y in tile_y_positions {
            for tile_x in tile_x_positions.iter().copied() {
                Self::draw_background_paint(
                    canvas,
                    origin_x + Pt::from_f32(tile_x),
                    origin_y + Pt::from_f32(tile_y),
                    layer_width,
                    layer_height,
                    Pt::ZERO,
                    paint,
                );
            }
        }
        canvas.restore_state();
    }

    fn draw_box_shadow(
        &self,
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: Pt,
        shadow: &BoxShadowSpec,
    ) {
        if shadow.inset {
            return;
        }
        let offset_x = shadow
            .offset_x
            .resolve_width(width, self.font_size, self.root_font_size);
        let offset_y = shadow
            .offset_y
            .resolve_height(height, self.font_size, self.root_font_size);
        let blur = shadow
            .blur
            .resolve_width(width, self.font_size, self.root_font_size)
            .max(Pt::ZERO);
        let spread = shadow
            .spread
            .resolve_width(width, self.font_size, self.root_font_size);

        let base_x = x + offset_x - spread;
        let base_y = y + offset_y - spread;
        let base_w = (width + spread * 2).max(Pt::ZERO);
        let base_h = (height + spread * 2).max(Pt::ZERO);
        let base_r = (radius + spread).max(Pt::ZERO);
        if base_w <= Pt::ZERO || base_h <= Pt::ZERO {
            return;
        }

        let shadow_color = self.apply_paint_filter_color(shadow.color);
        if blur <= Pt::ZERO {
            let opacity = shadow.opacity.clamp(0.0, 1.0);
            canvas.set_opacity(opacity, opacity);
            canvas.set_fill_color(shadow_color);
            if base_r > Pt::ZERO {
                Self::draw_rounded_rect_fill(canvas, base_x, base_y, base_w, base_h, base_r);
            } else {
                canvas.draw_rect(base_x, base_y, base_w, base_h);
            }
            canvas.set_opacity(1.0, 1.0);
            return;
        }

        let blur_f = blur.to_f32().max(0.0);
        let steps = ((blur_f / 1.5).ceil() as usize).clamp(6, 24);
        let sigma = 0.38_f32;
        let mut weights = Vec::with_capacity(steps);
        let mut weight_sum = 0.0_f32;
        for i in 0..steps {
            let t = (i as f32 + 0.5) / (steps as f32);
            let z = t / sigma;
            let w = (-0.5 * z * z).exp();
            weights.push(w);
            weight_sum += w;
        }
        let norm = if weight_sum > 0.0 { weight_sum } else { 1.0 };
        for i in (0..steps).rev() {
            let frac = (i + 1) as f32 / (steps as f32);
            let extra = blur * frac;
            let opacity = (shadow.opacity * (weights[i] / norm)).clamp(0.0, 1.0);
            if opacity <= 0.0 {
                continue;
            }
            let x0 = base_x - extra;
            let y0 = base_y - extra;
            let w0 = (base_w + extra * 2).max(Pt::ZERO);
            let h0 = (base_h + extra * 2).max(Pt::ZERO);
            if w0 <= Pt::ZERO || h0 <= Pt::ZERO {
                continue;
            }
            let r0 = (base_r + extra).max(Pt::ZERO);
            canvas.set_opacity(opacity, opacity);
            canvas.set_fill_color(shadow_color);
            if r0 > Pt::ZERO {
                Self::draw_rounded_rect_fill(canvas, x0, y0, w0, h0, r0);
            } else {
                canvas.draw_rect(x0, y0, w0, h0);
            }
        }
        canvas.set_opacity(1.0, 1.0);
    }

    fn draw_inset_box_shadow(
        &self,
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: Pt,
        shadow: &BoxShadowSpec,
    ) {
        if !shadow.inset || shadow.opacity <= 0.0 || width <= Pt::ZERO || height <= Pt::ZERO {
            return;
        }
        let blur = shadow
            .blur
            .resolve_width(width, self.font_size, self.root_font_size)
            .max(Pt::ZERO);
        let offset_x = shadow
            .offset_x
            .resolve_width(width, self.font_size, self.root_font_size);
        let offset_y = shadow
            .offset_y
            .resolve_height(height, self.font_size, self.root_font_size);
        let spread = shadow
            .spread
            .resolve_width(width, self.font_size, self.root_font_size)
            .max(Pt::ZERO);

        canvas.save_state();
        if radius > Pt::ZERO {
            Self::rounded_rect_path(canvas, x, y, width, height, radius);
            canvas.clip_path(false);
        } else {
            canvas.clip_rect(x, y, width, height);
        }
        let opacity = shadow.opacity.clamp(0.0, 1.0);
        Self::draw_inset_shadow_layers(
            canvas,
            x,
            y,
            width,
            height,
            offset_x,
            offset_y,
            spread,
            blur,
            opacity,
            self.apply_paint_filter_color(shadow.color),
        );
        canvas.restore_state();
    }

    fn draw_inset_shadow_layers(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        offset_x: Pt,
        offset_y: Pt,
        spread: Pt,
        blur: Pt,
        opacity: f32,
        color: Color,
    ) {
        let max_thickness = (width / 2.0).min(height / 2.0);
        let spread = spread.min(max_thickness).max(Pt::ZERO);
        let blur = blur.max(Pt::ZERO);
        let offset_present = offset_x != Pt::ZERO || offset_y != Pt::ZERO;
        let core = ResolvedEdges {
            top: (spread
                + if offset_y > Pt::ZERO {
                    offset_y
                } else {
                    Pt::ZERO
                })
            .min(max_thickness),
            right: (spread
                + if offset_x < Pt::ZERO {
                    -offset_x
                } else {
                    Pt::ZERO
                })
            .min(max_thickness),
            bottom: (spread
                + if offset_y < Pt::ZERO {
                    -offset_y
                } else {
                    Pt::ZERO
                })
            .min(max_thickness),
            left: (spread
                + if offset_x > Pt::ZERO {
                    offset_x
                } else {
                    Pt::ZERO
                })
            .min(max_thickness),
        };
        let blur_growth = ResolvedEdges {
            top: if spread > Pt::ZERO || offset_y > Pt::ZERO || !offset_present {
                blur
            } else {
                Pt::ZERO
            },
            right: if spread > Pt::ZERO || offset_x < Pt::ZERO || !offset_present {
                blur
            } else {
                Pt::ZERO
            },
            bottom: if spread > Pt::ZERO || offset_y < Pt::ZERO || !offset_present {
                blur
            } else {
                Pt::ZERO
            },
            left: if spread > Pt::ZERO || offset_x > Pt::ZERO || !offset_present {
                blur
            } else {
                Pt::ZERO
            },
        };
        let has_core = Self::inset_shadow_sides_have_paint(core);
        let has_blur = Self::inset_shadow_sides_have_paint(blur_growth);
        if opacity <= 0.0 || (!has_core && !has_blur) {
            return;
        }

        canvas.set_fill_color(color);
        if blur <= Pt::ZERO {
            canvas.set_opacity(opacity, opacity);
            Self::draw_inset_shadow_sides(canvas, x, y, width, height, core);
            canvas.set_opacity(1.0, 1.0);
            return;
        }

        if has_core {
            canvas.set_opacity(opacity, opacity);
            Self::draw_inset_shadow_sides(canvas, x, y, width, height, core);
        }

        if !has_blur {
            canvas.set_opacity(1.0, 1.0);
            return;
        }

        let blur_f = blur.to_f32().max(0.0);
        let steps = ((blur_f / 1.5).ceil() as usize).clamp(6, 24);
        let sigma = 0.45_f32;
        for i in 0..steps {
            let t0 = i as f32 / steps as f32;
            let t1 = (i + 1) as f32 / steps as f32;
            let outer = Self::inset_shadow_sides_lerp(core, blur_growth, t0, max_thickness);
            let inner = Self::inset_shadow_sides_lerp(core, blur_growth, t1, max_thickness);
            let midpoint = (i as f32 + 0.5) / steps as f32;
            let z = midpoint / sigma;
            let band_opacity = (opacity * (-0.5 * z * z).exp()).clamp(0.0, 1.0);
            if band_opacity <= 0.0 {
                continue;
            }
            canvas.set_opacity(band_opacity, band_opacity);
            Self::draw_inset_shadow_sides_band(canvas, x, y, width, height, outer, inner);
        }
        canvas.set_opacity(1.0, 1.0);
    }

    fn inset_shadow_sides_have_paint(sides: ResolvedEdges) -> bool {
        sides.top > Pt::ZERO
            || sides.right > Pt::ZERO
            || sides.bottom > Pt::ZERO
            || sides.left > Pt::ZERO
    }

    fn inset_shadow_sides_lerp(
        core: ResolvedEdges,
        growth: ResolvedEdges,
        t: f32,
        max_thickness: Pt,
    ) -> ResolvedEdges {
        ResolvedEdges {
            top: (core.top + growth.top * t).min(max_thickness),
            right: (core.right + growth.right * t).min(max_thickness),
            bottom: (core.bottom + growth.bottom * t).min(max_thickness),
            left: (core.left + growth.left * t).min(max_thickness),
        }
    }

    fn draw_inset_shadow_sides(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        inner: ResolvedEdges,
    ) {
        Self::draw_inset_shadow_sides_band(
            canvas,
            x,
            y,
            width,
            height,
            ResolvedEdges {
                top: Pt::ZERO,
                right: Pt::ZERO,
                bottom: Pt::ZERO,
                left: Pt::ZERO,
            },
            inner,
        );
    }

    fn draw_inset_shadow_sides_band(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        outer: ResolvedEdges,
        inner: ResolvedEdges,
    ) {
        let max_thickness = (width / 2.0).min(height / 2.0);
        let outer = ResolvedEdges {
            top: outer.top.min(max_thickness).max(Pt::ZERO),
            right: outer.right.min(max_thickness).max(Pt::ZERO),
            bottom: outer.bottom.min(max_thickness).max(Pt::ZERO),
            left: outer.left.min(max_thickness).max(Pt::ZERO),
        };
        let inner = ResolvedEdges {
            top: inner.top.min(max_thickness).max(outer.top),
            right: inner.right.min(max_thickness).max(outer.right),
            bottom: inner.bottom.min(max_thickness).max(outer.bottom),
            left: inner.left.min(max_thickness).max(outer.left),
        };
        let top_band = inner.top - outer.top;
        let right_band = inner.right - outer.right;
        let bottom_band = inner.bottom - outer.bottom;
        let left_band = inner.left - outer.left;
        if top_band <= Pt::ZERO
            && right_band <= Pt::ZERO
            && bottom_band <= Pt::ZERO
            && left_band <= Pt::ZERO
        {
            return;
        }

        let horizontal_x = x + outer.left;
        let horizontal_w = (width - outer.left - outer.right).max(Pt::ZERO);
        if top_band > Pt::ZERO && horizontal_w > Pt::ZERO {
            canvas.draw_rect(horizontal_x, y + outer.top, horizontal_w, top_band);
        }
        if bottom_band > Pt::ZERO && horizontal_w > Pt::ZERO {
            canvas.draw_rect(
                horizontal_x,
                y + height - inner.bottom,
                horizontal_w,
                bottom_band,
            );
        }

        let side_y = y + inner.top;
        let side_h = (height - inner.top - inner.bottom).max(Pt::ZERO);
        if left_band > Pt::ZERO && side_h > Pt::ZERO {
            canvas.draw_rect(x + outer.left, side_y, left_band, side_h);
        }
        if right_band > Pt::ZERO && side_h > Pt::ZERO {
            canvas.draw_rect(x + width - inner.right, side_y, right_band, side_h);
        }
    }
}

impl Flowable for ContainerFlowable {
    fn intrinsic_width(&self) -> Option<Pt> {
        if !matches!(
            self.width,
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
        ) {
            return None;
        }

        let mut max_child = Pt::ZERO;
        for child in &self.children {
            if child.out_of_flow() {
                continue;
            }
            let child_width = child.intrinsic_width()?;
            max_child = max_child.max(child_width);
        }

        let margin = self
            .margin
            .resolve(max_child, self.font_size, self.root_font_size);
        let border = self
            .border_width
            .resolve(max_child, self.font_size, self.root_font_size);
        let padding = self
            .padding
            .resolve(max_child, self.font_size, self.root_font_size);

        let content_width = max_child.max(Pt::ZERO);
        let border_box_width =
            border.left + padding.left + content_width + padding.right + border.right;

        Some((border_box_width + margin.left + margin.right).max(Pt::ZERO))
    }

    fn first_baseline(&self, avail_width: Pt) -> Option<Pt> {
        let (margin, border, padding, content_width, _) = self.resolve_box(avail_width);
        let mut offset = margin.top + border.top + padding.top;
        for child in &self.children {
            if child.out_of_flow() {
                continue;
            }
            if let Some(baseline) = child.first_baseline(content_width) {
                return Some(offset + baseline);
            }
            offset = offset + child.wrap(content_width, huge_pt()).height;
        }
        None
    }

    fn wrap(&self, avail_width: Pt, avail_height: Pt) -> Size {
        let cache = self.cached_layout(avail_width, avail_height);
        Size {
            width: cache.total_width,
            height: cache.total_height,
        }
    }

    fn split(
        &self,
        avail_width: Pt,
        avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        let (margin, border, padding, content_width, _border_box_width) =
            self.resolve_box(avail_width);
        let available_content_height = avail_height
            - margin.top
            - margin.bottom
            - border.top
            - border.bottom
            - padding.top
            - padding.bottom;
        if available_content_height <= Pt::ZERO {
            return None;
        }

        let mut remaining_height = available_content_height;
        let mut placed: Vec<Box<dyn Flowable>> = Vec::new();
        let mut remaining: Vec<Box<dyn Flowable>> = Vec::new();
        let out_of_flow: Vec<Box<dyn Flowable>> = self
            .children
            .iter()
            .cloned()
            .filter(|child| child.out_of_flow())
            .collect();
        let flow_children: Vec<Box<dyn Flowable>> = self
            .children
            .iter()
            .cloned()
            .filter(|child| !child.out_of_flow())
            .collect();

        for (index, child) in flow_children.iter().cloned().enumerate() {
            let pagination = child.pagination();
            if pagination.break_before.forces_page() && !placed.is_empty() {
                remaining.push(child);
                for rest in flow_children[index + 1..].iter().cloned() {
                    remaining.push(rest);
                }
                break;
            }

            let size = child.wrap(content_width, remaining_height);
            if size.height <= remaining_height {
                placed.push(child);
                remaining_height -= size.height;
                if pagination.break_after.forces_page() {
                    for rest in flow_children[index + 1..].iter().cloned() {
                        remaining.push(rest);
                    }
                    break;
                }
                continue;
            }

            if let Some((first, second)) = child.split(content_width, remaining_height) {
                placed.push(first);
                remaining.push(second);
                for rest in flow_children[index + 1..].iter().cloned() {
                    remaining.push(rest);
                }
                break;
            } else {
                remaining.push(child);
                for rest in flow_children[index + 1..].iter().cloned() {
                    remaining.push(rest);
                }
                break;
            }
        }

        if placed.is_empty() || remaining.is_empty() {
            return None;
        }

        if !out_of_flow.is_empty() {
            for child in &out_of_flow {
                placed.push(child.clone());
            }
            for child in &out_of_flow {
                remaining.push(child.clone());
            }
        }

        let first = ContainerFlowable {
            children: placed,
            margin: Self::zero_bottom(self.margin),
            border_width: Self::zero_bottom(self.border_width),
            border_colors: self.border_colors,
            border_styles: self.border_styles,
            border_radius: self.border_radius,
            outline_width: self.outline_width,
            outline_offset: self.outline_offset,
            outline_style: self.outline_style,
            outline_color: self.outline_color,
            outline_visible: self.outline_visible,
            padding: Self::zero_bottom(self.padding),
            width: self.width,
            max_width: self.max_width,
            min_width: self.min_width,
            height: self.height,
            min_height: self.min_height,
            max_height: self.max_height,
            box_sizing: self.box_sizing,
            background: self.background,
            background_paint: self.background_paint.clone(),
            background_paints: self.background_paints.clone(),
            background_sizes: self.background_sizes.clone(),
            background_positions: self.background_positions.clone(),
            background_repeats: self.background_repeats.clone(),
            background_blend_modes: self.background_blend_modes.clone(),
            background_origins: self.background_origins.clone(),
            background_clips: self.background_clips.clone(),
            clip_path: self.clip_path.clone(),
            clip_path_reference_box: self.clip_path_reference_box,
            clip_path_backdrop_root_group_suppressed: self.clip_path_backdrop_root_group_suppressed,
            will_change_backdrop_root: self.will_change_backdrop_root,
            will_change_backdrop_root_group_suppressed: self
                .will_change_backdrop_root_group_suppressed,
            mask_backdrop_root: self.mask_backdrop_root,
            mask_backdrop_root_group_suppressed: self.mask_backdrop_root_group_suppressed,
            box_shadow: self.box_shadow.clone(),
            box_shadows: self.box_shadows.clone(),
            paint_filter: self.paint_filter.clone(),
            backdrop_filter: self.backdrop_filter.clone(),
            mix_blend_mode: self.mix_blend_mode,
            isolation: self.isolation,
            opacity: self.opacity,
            transforms: self.transforms.clone(),
            transform_origin: self.transform_origin,
            overflow_hidden: self.overflow_hidden,
            self_visible: self.self_visible,
            tag_role: self.tag_role.clone(),
            establishes_abs_containing_block: self.establishes_abs_containing_block,
            font_size: self.font_size,
            root_font_size: self.root_font_size,
            pagination: Pagination {
                break_before: BreakBefore::Auto,
                break_after: BreakAfter::Auto,
                ..self.pagination
            },
            layout_cache: Arc::new(Mutex::new(None)),
        };
        let second = ContainerFlowable {
            children: remaining,
            margin: Self::zero_top(self.margin),
            border_width: Self::zero_top(self.border_width),
            border_colors: self.border_colors,
            border_styles: self.border_styles,
            border_radius: self.border_radius,
            outline_width: self.outline_width,
            outline_offset: self.outline_offset,
            outline_style: self.outline_style,
            outline_color: self.outline_color,
            outline_visible: self.outline_visible,
            padding: Self::zero_top(self.padding),
            width: self.width,
            max_width: self.max_width,
            min_width: self.min_width,
            height: self.height,
            min_height: self.min_height,
            max_height: self.max_height,
            box_sizing: self.box_sizing,
            background: self.background,
            background_paint: self.background_paint.clone(),
            background_paints: self.background_paints.clone(),
            background_sizes: self.background_sizes.clone(),
            background_positions: self.background_positions.clone(),
            background_repeats: self.background_repeats.clone(),
            background_blend_modes: self.background_blend_modes.clone(),
            background_origins: self.background_origins.clone(),
            background_clips: self.background_clips.clone(),
            clip_path: self.clip_path.clone(),
            clip_path_reference_box: self.clip_path_reference_box,
            clip_path_backdrop_root_group_suppressed: self.clip_path_backdrop_root_group_suppressed,
            will_change_backdrop_root: self.will_change_backdrop_root,
            will_change_backdrop_root_group_suppressed: self
                .will_change_backdrop_root_group_suppressed,
            mask_backdrop_root: self.mask_backdrop_root,
            mask_backdrop_root_group_suppressed: self.mask_backdrop_root_group_suppressed,
            box_shadow: self.box_shadow.clone(),
            box_shadows: self.box_shadows.clone(),
            paint_filter: self.paint_filter.clone(),
            backdrop_filter: self.backdrop_filter.clone(),
            mix_blend_mode: self.mix_blend_mode,
            isolation: self.isolation,
            opacity: self.opacity,
            transforms: self.transforms.clone(),
            transform_origin: self.transform_origin,
            overflow_hidden: self.overflow_hidden,
            self_visible: self.self_visible,
            tag_role: self.tag_role.clone(),
            establishes_abs_containing_block: self.establishes_abs_containing_block,
            font_size: self.font_size,
            root_font_size: self.root_font_size,
            pagination: Pagination {
                break_before: BreakBefore::Auto,
                ..self.pagination
            },
            layout_cache: Arc::new(Mutex::new(None)),
        };

        Some((Box::new(first), Box::new(second)))
    }

    fn draw(&self, canvas: &mut Canvas, x: Pt, y: Pt, avail_width: Pt, avail_height: Pt) {
        let group_opacity = self.opacity.clamp(0.0, 1.0);
        let opacity_applied = group_opacity < 1.0 - 1.0e-6;
        let blend_applied = self.mix_blend_mode != MixBlendMode::Normal;
        if opacity_applied || blend_applied {
            let page_size = canvas.page_size();
            let form_id = format!(
                "composite:{}:{}:{}",
                canvas.current_command_count(),
                x.to_milli_i64(),
                y.to_milli_i64()
            );
            let mut grouped = self.clone();
            grouped.mix_blend_mode = MixBlendMode::Normal;
            grouped.isolation = false;
            grouped.opacity = 1.0;

            let mut temp = Canvas::new(page_size);
            grouped.draw(&mut temp, x, y, avail_width, avail_height);
            let doc = temp.finish();
            let commands = doc
                .pages
                .first()
                .map(|page| page.commands.clone())
                .unwrap_or_default();

            canvas.define_isolated_form(
                form_id.clone(),
                page_size.width,
                page_size.height,
                commands,
            );
            canvas.save_state();
            if opacity_applied {
                canvas.set_opacity(group_opacity, group_opacity);
            }
            if blend_applied {
                canvas.set_blend_mode(self.mix_blend_mode);
            }
            canvas.draw_form(
                Pt::ZERO,
                Pt::ZERO,
                page_size.width,
                page_size.height,
                form_id,
            );
            canvas.restore_state();
            return;
        }

        if self.isolation {
            let page_size = canvas.page_size();
            let form_id = format!(
                "isolation:{}:{}:{}",
                canvas.current_command_count(),
                x.to_milli_i64(),
                y.to_milli_i64()
            );
            let mut grouped = self.clone();
            grouped.isolation = false;

            let mut temp = Canvas::new(page_size);
            grouped.draw(&mut temp, x, y, avail_width, avail_height);
            let doc = temp.finish();
            let commands = doc
                .pages
                .first()
                .map(|page| page.commands.clone())
                .unwrap_or_default();

            canvas.define_isolated_form(
                form_id.clone(),
                page_size.width,
                page_size.height,
                commands,
            );
            canvas.draw_form(
                Pt::ZERO,
                Pt::ZERO,
                page_size.width,
                page_size.height,
                form_id,
            );
            return;
        }

        if let Some(filter) = self.paint_filter.as_ref() {
            let page_size = canvas.page_size();
            let form_id = format!(
                "filter:{}:{}:{}",
                canvas.current_command_count(),
                x.to_milli_i64(),
                y.to_milli_i64()
            );
            let mut grouped = self.clone();
            grouped.paint_filter = None;

            let mut temp = Canvas::new(page_size);
            grouped.draw(&mut temp, x, y, avail_width, avail_height);
            let doc = temp.finish();
            let commands = doc
                .pages
                .first()
                .map(|page| page.commands.clone())
                .unwrap_or_default();

            canvas.define_form(form_id.clone(), page_size.width, page_size.height, commands);
            canvas.draw_filtered_form(
                Pt::ZERO,
                Pt::ZERO,
                page_size.width,
                page_size.height,
                form_id,
                filter.clone(),
            );
            return;
        }

        if self.clip_path.is_some() && !self.clip_path_backdrop_root_group_suppressed {
            let page_size = canvas.page_size();
            let form_id = format!(
                "clip-root:{}:{}:{}",
                canvas.current_command_count(),
                x.to_milli_i64(),
                y.to_milli_i64()
            );
            let mut grouped = self.clone();
            grouped.clip_path_backdrop_root_group_suppressed = true;

            let mut temp = Canvas::new(page_size);
            grouped.draw(&mut temp, x, y, avail_width, avail_height);
            let doc = temp.finish();
            let commands = doc
                .pages
                .first()
                .map(|page| page.commands.clone())
                .unwrap_or_default();

            canvas.define_isolated_form(
                form_id.clone(),
                page_size.width,
                page_size.height,
                commands,
            );
            canvas.draw_form(
                Pt::ZERO,
                Pt::ZERO,
                page_size.width,
                page_size.height,
                form_id,
            );
            return;
        }

        if let Some(backdrop_filter) = self.backdrop_filter.as_ref() {
            let page_size = canvas.page_size();
            let cache = self.cached_layout(avail_width, avail_height);
            let margin = cache.margin;
            let border_box_width = cache.border_box_width;
            let border_box_height = cache.border_box_height;
            let border_box_x = x + margin.left;
            let border_box_y = y + margin.top;
            let border_clip_radii = self.border_radius.resolve(
                border_box_width,
                border_box_height,
                self.font_size,
                self.root_font_size,
            );
            let radius = Self::uniform_radius_from_clip_radii(border_clip_radii);

            if self.self_visible {
                let transformed = self.has_transforms();
                if transformed {
                    let origin_dx = self.transform_origin.x.resolve_width(
                        border_box_width,
                        self.font_size,
                        self.root_font_size,
                    );
                    let origin_dy = self.transform_origin.y.resolve_height(
                        border_box_height,
                        self.font_size,
                        self.root_font_size,
                    );
                    let origin_x = border_box_x + origin_dx;
                    let origin_y = border_box_y + origin_dy;
                    canvas.save_state();
                    canvas.translate_css_transform_origin(origin_x, origin_y, false);
                    self.apply_transforms(canvas, border_box_width, border_box_height);
                    canvas.translate_css_transform_origin(origin_x, origin_y, true);
                }
                canvas.apply_backdrop_filter(
                    border_box_x,
                    border_box_y,
                    border_box_width,
                    border_box_height,
                    radius,
                    backdrop_filter.clone(),
                );
                if transformed {
                    canvas.restore_state();
                }
            }

            let form_id = format!(
                "backdrop-root:{}:{}:{}",
                canvas.current_command_count(),
                x.to_milli_i64(),
                y.to_milli_i64()
            );
            let mut grouped = self.clone();
            grouped.backdrop_filter = None;

            let mut temp = Canvas::new(page_size);
            grouped.draw(&mut temp, x, y, avail_width, avail_height);
            let doc = temp.finish();
            let commands = doc
                .pages
                .first()
                .map(|page| page.commands.clone())
                .unwrap_or_default();

            canvas.define_isolated_form(
                form_id.clone(),
                page_size.width,
                page_size.height,
                commands,
            );
            canvas.draw_form(
                Pt::ZERO,
                Pt::ZERO,
                page_size.width,
                page_size.height,
                form_id,
            );
            return;
        }

        if self.will_change_backdrop_root && !self.will_change_backdrop_root_group_suppressed {
            let page_size = canvas.page_size();
            let form_id = format!(
                "will-change-root:{}:{}:{}",
                canvas.current_command_count(),
                x.to_milli_i64(),
                y.to_milli_i64()
            );
            let mut grouped = self.clone();
            grouped.will_change_backdrop_root_group_suppressed = true;

            let mut temp = Canvas::new(page_size);
            grouped.draw(&mut temp, x, y, avail_width, avail_height);
            let doc = temp.finish();
            let commands = doc
                .pages
                .first()
                .map(|page| page.commands.clone())
                .unwrap_or_default();

            canvas.define_isolated_form(
                form_id.clone(),
                page_size.width,
                page_size.height,
                commands,
            );
            canvas.draw_form(
                Pt::ZERO,
                Pt::ZERO,
                page_size.width,
                page_size.height,
                form_id,
            );
            return;
        }

        if self.mask_backdrop_root && !self.mask_backdrop_root_group_suppressed {
            let page_size = canvas.page_size();
            let form_id = format!(
                "mask-root:{}:{}:{}",
                canvas.current_command_count(),
                x.to_milli_i64(),
                y.to_milli_i64()
            );
            let mut grouped = self.clone();
            grouped.mask_backdrop_root_group_suppressed = true;

            let mut temp = Canvas::new(page_size);
            grouped.draw(&mut temp, x, y, avail_width, avail_height);
            let doc = temp.finish();
            let commands = doc
                .pages
                .first()
                .map(|page| page.commands.clone())
                .unwrap_or_default();

            canvas.define_isolated_form(
                form_id.clone(),
                page_size.width,
                page_size.height,
                commands,
            );
            canvas.draw_form(
                Pt::ZERO,
                Pt::ZERO,
                page_size.width,
                page_size.height,
                form_id,
            );
            return;
        }

        let tagged = self.tag_role.as_ref().map(|role| {
            canvas.begin_tag(role.as_ref(), None, None, None, None, true);
        });
        let cache = self.cached_layout(avail_width, avail_height);
        let margin = cache.margin;
        let border = cache.border;
        let padding = cache.padding;
        let content_width = cache.content_width;
        let content_height = cache.content_height;
        let border_box_width = cache.border_box_width;
        let border_box_height = cache.border_box_height;
        let child_avail_height = cache.child_avail_height;

        let border_box_x = x + margin.left;
        let border_box_y = y + margin.top;
        let transformed = self.has_transforms();
        if transformed {
            // CSS transforms apply around transform-origin (default: center center) and do
            // not participate in wrap/split geometry in this phase.
            let origin_dx = self.transform_origin.x.resolve_width(
                border_box_width,
                self.font_size,
                self.root_font_size,
            );
            let origin_dy = self.transform_origin.y.resolve_height(
                border_box_height,
                self.font_size,
                self.root_font_size,
            );
            let origin_x = border_box_x + origin_dx;
            let origin_y = border_box_y + origin_dy;
            canvas.save_state();
            canvas.translate_css_transform_origin(origin_x, origin_y, false);
            self.apply_transforms(canvas, border_box_width, border_box_height);
            canvas.translate_css_transform_origin(origin_x, origin_y, true);
        }
        let border_clip_radii = self.border_radius.resolve(
            border_box_width,
            border_box_height,
            self.font_size,
            self.root_font_size,
        );
        let radius = Self::uniform_radius_from_clip_radii(border_clip_radii);

        let mut clip_path_applied = false;
        if let Some(clip_path) = self.clip_path.as_ref() {
            canvas.save_state();
            let (clip_ref_x, clip_ref_y, clip_ref_w, clip_ref_h) =
                Self::resolve_clip_path_reference_rect(
                    self.clip_path_reference_box,
                    x,
                    y,
                    margin,
                    border,
                    padding,
                    content_width,
                    content_height,
                    border_box_x,
                    border_box_y,
                    border_box_width,
                    border_box_height,
                );
            let clip_ref_radius = Self::resolve_clip_path_reference_radii(
                self.clip_path_reference_box,
                border_clip_radii,
                margin,
                border,
                padding,
            );
            match clip_path {
                ClipPathShapeSpec::Inset(spec) => {
                    if let Some((clip_x, clip_y, clip_w, clip_h)) = self
                        .resolve_clip_path_inset_rect(
                            *spec, clip_ref_x, clip_ref_y, clip_ref_w, clip_ref_h,
                        )
                    {
                        self.apply_clip_rect_or_rounded(
                            canvas,
                            clip_x,
                            clip_y,
                            clip_w,
                            clip_h,
                            spec.radius,
                        );
                    }
                }
                ClipPathShapeSpec::Circle(spec) => {
                    let (cx, cy, radius) = self.resolve_clip_path_circle(
                        *spec, clip_ref_x, clip_ref_y, clip_ref_w, clip_ref_h,
                    );
                    Self::rounded_rect_path(
                        canvas,
                        cx - radius,
                        cy - radius,
                        radius * 2.0,
                        radius * 2.0,
                        radius,
                    );
                    canvas.clip_path(false);
                }
                ClipPathShapeSpec::Ellipse(spec) => {
                    let (cx, cy, radius_x, radius_y) = self.resolve_clip_path_ellipse(
                        spec.clone(),
                        clip_ref_x,
                        clip_ref_y,
                        clip_ref_w,
                        clip_ref_h,
                    );
                    Self::ellipse_path(canvas, cx, cy, radius_x, radius_y);
                    canvas.clip_path(false);
                }
                ClipPathShapeSpec::Xywh(spec) => {
                    let (clip_x, clip_y, clip_w, clip_h) = self.resolve_clip_path_xywh_rect(
                        *spec, clip_ref_x, clip_ref_y, clip_ref_w, clip_ref_h,
                    );
                    self.apply_clip_rect_or_rounded(
                        canvas,
                        clip_x,
                        clip_y,
                        clip_w,
                        clip_h,
                        spec.radius,
                    );
                }
                ClipPathShapeSpec::Rect(spec) => {
                    let (clip_x, clip_y, clip_w, clip_h) = self.resolve_clip_path_rect(
                        *spec, clip_ref_x, clip_ref_y, clip_ref_w, clip_ref_h,
                    );
                    self.apply_clip_rect_or_rounded(
                        canvas,
                        clip_x,
                        clip_y,
                        clip_w,
                        clip_h,
                        spec.radius,
                    );
                }
                ClipPathShapeSpec::Polygon(spec) => {
                    if self
                        .polygon_path(canvas, spec, clip_ref_x, clip_ref_y, clip_ref_w, clip_ref_h)
                    {
                        canvas.clip_path(spec.evenodd);
                    }
                }
                ClipPathShapeSpec::Path(spec) => {
                    if self.css_path(canvas, spec, clip_ref_x, clip_ref_y) {
                        canvas.clip_path(spec.evenodd);
                    }
                }
                ClipPathShapeSpec::ShapeFunction(spec) => {
                    if self.css_shape_function_path(
                        canvas, spec, clip_ref_x, clip_ref_y, clip_ref_w, clip_ref_h,
                    ) {
                        canvas.clip_path(spec.evenodd);
                    }
                }
                ClipPathShapeSpec::ReferenceBox => {
                    Self::apply_clip_path_reference_box(
                        canvas,
                        clip_ref_x,
                        clip_ref_y,
                        clip_ref_w,
                        clip_ref_h,
                        clip_ref_radius,
                    );
                }
            }
            clip_path_applied = true;
        }

        let paint_self = self.self_visible;

        if paint_self {
            if let Some(backdrop_filter) = self.backdrop_filter.as_ref() {
                canvas.apply_backdrop_filter(
                    border_box_x,
                    border_box_y,
                    border_box_width,
                    border_box_height,
                    radius,
                    backdrop_filter.clone(),
                );
            }
        }

        let paint_opacity = self
            .paint_filter
            .as_ref()
            .map(|filter| filter.opacity.clamp(0.0, 1.0))
            .unwrap_or(1.0);
        let paint_opacity_applied = paint_opacity < 1.0 - 1.0e-6;
        if paint_opacity_applied {
            canvas.save_state();
            canvas.set_opacity(paint_opacity, paint_opacity);
        }

        if paint_self && !self.box_shadows.is_empty() {
            for shadow in self.box_shadows.iter().rev() {
                self.draw_box_shadow(
                    canvas,
                    border_box_x,
                    border_box_y,
                    border_box_width,
                    border_box_height,
                    radius,
                    shadow,
                );
            }
        }

        if paint_self {
            if let Some(color) = self.background {
                canvas.set_fill_color(self.apply_paint_filter_color(color));
                let color_clip = Self::background_layer_value(&self.background_clips, 0);
                let (clip_x, clip_y, clip_width, clip_height) = Self::background_clip_rect(
                    color_clip,
                    border_box_x,
                    border_box_y,
                    border_box_width,
                    border_box_height,
                    border,
                    padding,
                    content_width,
                    content_height,
                );
                if color_clip == BackgroundClipBox::Border
                    && Self::clip_radii_have_rounding(border_clip_radii)
                {
                    Self::draw_rounded_rect_corners_fill(
                        canvas,
                        clip_x,
                        clip_y,
                        clip_width,
                        clip_height,
                        border_clip_radii,
                    );
                } else {
                    canvas.draw_rect(clip_x, clip_y, clip_width, clip_height);
                }
            }
        }

        let background_paints: Vec<BackgroundPaint> = if !paint_self {
            Vec::new()
        } else if !self.background_paints.is_empty() {
            self.background_paints.clone()
        } else {
            self.background_paint.iter().cloned().collect()
        };
        for (idx, paint) in background_paints.iter().enumerate().rev() {
            let paint_filtered = self.filtered_background_paint(paint);
            let size = Self::background_layer_value(&self.background_sizes, idx);
            let position = Self::background_layer_value(&self.background_positions, idx);
            let repeat = Self::background_layer_value(&self.background_repeats, idx);
            let blend_mode = Self::background_layer_value(&self.background_blend_modes, idx);
            let origin = Self::background_layer_value(&self.background_origins, idx);
            let clip = Self::background_layer_value(&self.background_clips, idx);
            self.draw_background_layer(
                canvas,
                border_box_x,
                border_box_y,
                border_box_width,
                border_box_height,
                border,
                padding,
                content_width,
                content_height,
                radius,
                &paint_filtered,
                size,
                position,
                repeat,
                blend_mode,
                origin,
                clip,
            );
        }

        if paint_self && !self.box_shadows.is_empty() {
            for shadow in self.box_shadows.iter().rev() {
                self.draw_inset_box_shadow(
                    canvas,
                    border_box_x,
                    border_box_y,
                    border_box_width,
                    border_box_height,
                    radius,
                    shadow,
                );
            }
        }

        if paint_self && Self::has_border(border) {
            let uniform_width = border.top == border.right
                && border.top == border.bottom
                && border.top == border.left;
            let border_colors = ResolvedEdgeColors {
                top: self.apply_paint_filter_color(self.border_colors.top),
                right: self.apply_paint_filter_color(self.border_colors.right),
                bottom: self.apply_paint_filter_color(self.border_colors.bottom),
                left: self.apply_paint_filter_color(self.border_colors.left),
            };
            let uniform_color = border_colors.top == border_colors.right
                && border_colors.top == border_colors.bottom
                && border_colors.top == border_colors.left;
            let rounded_uniform_style = if self.border_styles.is_uniform() {
                Some(self.border_styles.top)
            } else {
                None
            };
            if Self::clip_radii_have_rounding(border_clip_radii)
                && uniform_width
                && uniform_color
                && border.top > Pt::ZERO
                && rounded_uniform_style.is_some()
            {
                match rounded_uniform_style.unwrap() {
                    OutlineLineStyle::Double => Self::draw_rounded_uniform_double_border(
                        canvas,
                        border_box_x,
                        border_box_y,
                        border_box_width,
                        border_box_height,
                        border.top,
                        border_colors.top,
                        border_clip_radii,
                    ),
                    style @ (OutlineLineStyle::Groove
                    | OutlineLineStyle::Ridge
                    | OutlineLineStyle::Inset
                    | OutlineLineStyle::Outset) => Self::draw_rounded_uniform_3d_border(
                        canvas,
                        border_box_x,
                        border_box_y,
                        border_box_width,
                        border_box_height,
                        border.top,
                        border_colors.top,
                        style,
                        border_clip_radii,
                    ),
                    style => Self::draw_rounded_uniform_border_stroke(
                        canvas,
                        border_box_x,
                        border_box_y,
                        border_box_width,
                        border_box_height,
                        border.top,
                        border_colors.top,
                        style,
                        border_clip_radii,
                    ),
                }
            } else {
                Self::draw_border(
                    canvas,
                    border_box_x,
                    border_box_y,
                    border_box_width,
                    border_box_height,
                    border,
                    border_colors,
                    self.border_styles,
                );
            }
        }

        let outline_width = if paint_self && self.outline_visible {
            self.outline_width
                .resolve_width(border_box_width, self.font_size, self.root_font_size)
                .max(Pt::ZERO)
        } else {
            Pt::ZERO
        };
        if outline_width > Pt::ZERO {
            let outline_offset = self.outline_offset.resolve_width(
                border_box_width,
                self.font_size,
                self.root_font_size,
            );
            Self::draw_outline(
                canvas,
                border_box_x,
                border_box_y,
                border_box_width,
                border_box_height,
                outline_width,
                outline_offset,
                self.outline_style,
                self.apply_paint_filter_color(self.outline_color),
                border_clip_radii,
            );
        }

        if self.overflow_hidden {
            // Clip children to the padding box (CSS-ish overflow clipping).
            let padding_box_x = border_box_x + border.left;
            let padding_box_y = border_box_y + border.top;
            let padding_box_w = (border_box_width - border.left - border.right).max(Pt::ZERO);
            let padding_box_h = (border_box_height - border.top - border.bottom).max(Pt::ZERO);
            canvas.save_state();
            if radius > Pt::ZERO {
                Self::rounded_rect_path(
                    canvas,
                    padding_box_x,
                    padding_box_y,
                    padding_box_w,
                    padding_box_h,
                    radius,
                );
                canvas.clip_path(false);
            } else {
                canvas.clip_rect(padding_box_x, padding_box_y, padding_box_w, padding_box_h);
            }
        }

        let inner_y = border_box_y + border.top + padding.top;
        let mut cursor_y = inner_y;
        let inner_x = border_box_x + border.left + padding.left;

        let padding_box_x = border_box_x + border.left;
        let padding_box_y = border_box_y + border.top;
        let padding_box_w = (border_box_width - border.left - border.right).max(Pt::ZERO);
        let padding_box_h = (border_box_height - border.top - border.bottom).max(Pt::ZERO);

        let pushed_abs_cb = if self.establishes_abs_containing_block {
            canvas.push_abs_containing_block(Rect {
                x: padding_box_x,
                y: padding_box_y,
                width: padding_box_w,
                height: padding_box_h,
            });
            true
        } else {
            false
        };

        let mut out_of_flow_neg: Vec<(i32, usize, Pt, Pt, &Box<dyn Flowable>)> = Vec::new();
        let mut out_of_flow_zero: Vec<(usize, Pt, Pt, &Box<dyn Flowable>)> = Vec::new();
        let mut out_of_flow_pos: Vec<(i32, usize, Pt, Pt, &Box<dyn Flowable>)> = Vec::new();
        let mut static_cursor_y = inner_y;
        for (idx, child) in self.children.iter().enumerate() {
            if child.out_of_flow() {
                let static_x = inner_x;
                let static_y = static_cursor_y;
                let z = child.z_index();
                if z < 0 {
                    out_of_flow_neg.push((z, idx, static_x, static_y, child));
                } else if z > 0 {
                    out_of_flow_pos.push((z, idx, static_x, static_y, child));
                } else {
                    out_of_flow_zero.push((idx, static_x, static_y, child));
                }
                continue;
            }
            let size = cache
                .child_sizes
                .get(idx)
                .copied()
                .flatten()
                .unwrap_or_else(|| child.wrap(content_width, child_avail_height));
            static_cursor_y = static_cursor_y + size.height;
        }

        if !out_of_flow_neg.is_empty() {
            out_of_flow_neg.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            for (_, _, static_x, static_y, child) in out_of_flow_neg {
                child.draw(canvas, static_x, static_y, padding_box_w, padding_box_h);
            }
        }

        for (idx, child) in self.children.iter().enumerate() {
            if child.out_of_flow() {
                continue;
            }
            let size = cache
                .child_sizes
                .get(idx)
                .copied()
                .flatten()
                .unwrap_or_else(|| child.wrap(content_width, child_avail_height));
            let draw_height = if child.prefers_containing_block_draw_space()
                && child_avail_height < Pt::from_f32(1.0e8)
            {
                child_avail_height
            } else {
                size.height
            };
            child.draw(canvas, inner_x, cursor_y, content_width, draw_height);
            cursor_y = cursor_y + size.height;
        }

        if !out_of_flow_zero.is_empty() {
            out_of_flow_zero.sort_by(|a, b| a.0.cmp(&b.0));
            for (_, static_x, static_y, child) in out_of_flow_zero {
                child.draw(canvas, static_x, static_y, padding_box_w, padding_box_h);
            }
        }

        if !out_of_flow_pos.is_empty() {
            out_of_flow_pos.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            for (_, _, static_x, static_y, child) in out_of_flow_pos {
                child.draw(canvas, static_x, static_y, padding_box_w, padding_box_h);
            }
        }

        if pushed_abs_cb {
            canvas.pop_abs_containing_block();
        }

        if self.overflow_hidden {
            canvas.restore_state();
        }
        if paint_opacity_applied {
            canvas.restore_state();
        }
        if clip_path_applied {
            canvas.restore_state();
        }
        if transformed {
            canvas.restore_state();
        }
        if tagged.is_some() {
            canvas.end_tag();
        }
    }

    fn draw_stretched(&self, canvas: &mut Canvas, x: Pt, y: Pt, avail_width: Pt, avail_height: Pt) {
        if !matches!(
            self.height,
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
        ) {
            self.draw(canvas, x, y, avail_width, avail_height);
            return;
        }

        let (margin, border, padding, _, _) = self.resolve_box(avail_width);
        let border_box_height = (avail_height - margin.top - margin.bottom).max(Pt::ZERO);
        let forced_height = if matches!(self.box_sizing, BoxSizingMode::BorderBox) {
            border_box_height
        } else {
            (border_box_height - border.top - border.bottom - padding.top - padding.bottom)
                .max(Pt::ZERO)
        };
        let mut stretched = self.clone();
        stretched.height = LengthSpec::Absolute(forced_height);
        stretched.layout_cache = Arc::new(Mutex::new(None));
        stretched.draw(canvas, x, y, avail_width, avail_height);
    }

    fn pagination(&self) -> Pagination {
        self.pagination
    }
}

#[derive(Clone)]
pub struct AbsolutePositionedFlowable {
    child: Box<dyn Flowable>,
    left: LengthSpec,
    top: LengthSpec,
    right: LengthSpec,
    bottom: LengthSpec,
    width_spec: LengthSpec,
    height_spec: LengthSpec,
    z_index: i32,
    font_size: Pt,
    root_font_size: Pt,
    pagination: Pagination,
    fixed_positioned: bool,
}

#[derive(Clone)]
pub struct RelativePositionedFlowable {
    child: Box<dyn Flowable>,
    left: LengthSpec,
    top: LengthSpec,
    right: LengthSpec,
    bottom: LengthSpec,
    font_size: Pt,
    root_font_size: Pt,
    pagination: Pagination,
}

#[derive(Clone)]
pub struct MetaFlowable {
    child: Box<dyn Flowable>,
    metadata: Arc<Vec<(String, String)>>,
}

impl MetaFlowable {
    pub fn new(child: Box<dyn Flowable>, metadata: Vec<(String, String)>) -> Self {
        Self {
            child,
            metadata: Arc::new(metadata),
        }
    }
}

impl Flowable for MetaFlowable {
    fn wrap(&self, avail_width: Pt, avail_height: Pt) -> Size {
        let mut size = self.child.wrap(avail_width, avail_height);
        if !self.metadata.is_empty() && size.height <= Pt::ZERO {
            size.height = Pt::from_f32(0.01);
        }
        size
    }

    fn split(
        &self,
        avail_width: Pt,
        avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        let (first, second) = self.child.split(avail_width, avail_height)?;
        let meta = self.metadata.as_ref().clone();
        Some((
            Box::new(Self::new(first, meta.clone())) as Box<dyn Flowable>,
            Box::new(Self::new(second, meta)) as Box<dyn Flowable>,
        ))
    }

    fn draw(&self, canvas: &mut Canvas, x: Pt, y: Pt, avail_width: Pt, avail_height: Pt) {
        canvas.meta(META_DIAGNOSTIC_SCOPE_BEGIN_KEY, "flowable");
        for (k, v) in self.metadata.iter() {
            canvas.meta(k.clone(), v.clone());
        }
        self.child.draw(canvas, x, y, avail_width, avail_height);
        canvas.meta(META_DIAGNOSTIC_SCOPE_END_KEY, "flowable");
    }

    fn draw_stretched(&self, canvas: &mut Canvas, x: Pt, y: Pt, avail_width: Pt, avail_height: Pt) {
        canvas.meta(META_DIAGNOSTIC_SCOPE_BEGIN_KEY, "flowable");
        for (key, value) in self.metadata.iter() {
            canvas.meta(key.clone(), value.clone());
        }
        self.child
            .draw_stretched(canvas, x, y, avail_width, avail_height);
        canvas.meta(META_DIAGNOSTIC_SCOPE_END_KEY, "flowable");
    }

    fn intrinsic_width(&self) -> Option<Pt> {
        self.child.intrinsic_width()
    }

    fn first_baseline(&self, avail_width: Pt) -> Option<Pt> {
        self.child.first_baseline(avail_width)
    }

    fn out_of_flow(&self) -> bool {
        self.child.out_of_flow()
    }

    fn z_index(&self) -> i32 {
        self.child.z_index()
    }

    fn pagination(&self) -> Pagination {
        self.child.pagination()
    }

    fn is_fixed_positioned(&self) -> bool {
        self.child.is_fixed_positioned()
    }

    fn prefers_containing_block_draw_space(&self) -> bool {
        self.child.prefers_containing_block_draw_space()
    }

    fn diagnostic_metadata(&self) -> Vec<(String, String)> {
        let mut out = self.child.diagnostic_metadata();
        for (key, value) in self.metadata.iter() {
            if let Some(existing) = out.iter_mut().find(|(k, _)| k == key) {
                existing.1 = value.clone();
            } else {
                out.push((key.clone(), value.clone()));
            }
        }
        out
    }
}

impl AbsolutePositionedFlowable {
    pub fn new(
        child: Box<dyn Flowable>,
        left: LengthSpec,
        top: LengthSpec,
        right: LengthSpec,
        bottom: LengthSpec,
        width_spec: LengthSpec,
        height_spec: LengthSpec,
        z_index: i32,
        font_size: f32,
        root_font_size: f32,
    ) -> Self {
        Self::new_pt(
            child,
            left,
            top,
            right,
            bottom,
            width_spec,
            height_spec,
            z_index,
            Pt::from_f32(font_size),
            Pt::from_f32(root_font_size),
        )
    }

    pub fn new_pt(
        child: Box<dyn Flowable>,
        left: LengthSpec,
        top: LengthSpec,
        right: LengthSpec,
        bottom: LengthSpec,
        width_spec: LengthSpec,
        height_spec: LengthSpec,
        z_index: i32,
        font_size: Pt,
        root_font_size: Pt,
    ) -> Self {
        Self {
            child,
            left,
            top,
            right,
            bottom,
            width_spec,
            height_spec,
            z_index,
            font_size,
            root_font_size,
            pagination: Pagination::default(),
            fixed_positioned: false,
        }
    }

    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = pagination;
        self
    }

    pub fn with_fixed_positioned(mut self, fixed_positioned: bool) -> Self {
        self.fixed_positioned = fixed_positioned;
        self
    }
}

impl RelativePositionedFlowable {
    pub fn new_pt(
        child: Box<dyn Flowable>,
        left: LengthSpec,
        top: LengthSpec,
        right: LengthSpec,
        bottom: LengthSpec,
        font_size: Pt,
        root_font_size: Pt,
    ) -> Self {
        Self {
            child,
            left,
            top,
            right,
            bottom,
            font_size,
            root_font_size,
            pagination: Pagination::default(),
        }
    }

    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = pagination;
        self
    }
}

impl Flowable for AbsolutePositionedFlowable {
    fn wrap(&self, _avail_width: Pt, _avail_height: Pt) -> Size {
        Size {
            width: Pt::ZERO,
            height: Pt::ZERO,
        }
    }

    fn split(
        &self,
        _avail_width: Pt,
        _avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        None
    }

    fn draw(&self, canvas: &mut Canvas, x: Pt, y: Pt, _avail_width: Pt, _avail_height: Pt) {
        let (containing_x, containing_y, containing_width, containing_height) =
            if let Some(rect) = canvas.current_abs_containing_block() {
                (
                    rect.x,
                    rect.y,
                    rect.width.max(Pt::ZERO),
                    rect.height.max(Pt::ZERO),
                )
            } else {
                let page = canvas.page_size();
                (
                    Pt::ZERO,
                    Pt::ZERO,
                    page.width.max(Pt::ZERO),
                    page.height.max(Pt::ZERO),
                )
            };
        let has_left = !matches!(self.left, LengthSpec::Auto);
        let has_right = !matches!(self.right, LengthSpec::Auto);
        let has_top = !matches!(self.top, LengthSpec::Auto);
        let has_bottom = !matches!(self.bottom, LengthSpec::Auto);

        let left = if has_left {
            self.left
                .resolve_width(containing_width, self.font_size, self.root_font_size)
        } else {
            Pt::ZERO
        };
        let right = if has_right {
            self.right
                .resolve_width(containing_width, self.font_size, self.root_font_size)
        } else {
            Pt::ZERO
        };
        let top = if has_top {
            self.top
                .resolve_height(containing_height, self.font_size, self.root_font_size)
        } else {
            Pt::ZERO
        };
        let bottom = if has_bottom {
            self.bottom
                .resolve_height(containing_height, self.font_size, self.root_font_size)
        } else {
            Pt::ZERO
        };
        let left = left;
        let right = right;
        let top = top;
        let bottom = bottom;

        // CSS-ish positioning behavior:
        // - If both sides are set and size is auto, stretch to fill.
        // - If both sides are set and size is explicit, keep explicit size
        //   and anchor from the start side (left/top in current LTR model).
        // - If only one side is set, anchor there and use the child's intrinsic size.
        // - If neither is set, default to 0.
        let width_auto = matches!(self.width_spec, LengthSpec::Auto);
        let height_auto = matches!(self.height_spec, LengthSpec::Auto);
        let stretch_w = has_left && has_right && width_auto;
        let stretch_h = has_top && has_bottom && height_auto;

        let explicit_w = if width_auto {
            None
        } else {
            Some(
                self.width_spec
                    .resolve_width(containing_width, self.font_size, self.root_font_size)
                    .max(Pt::ZERO),
            )
        };
        let explicit_h = if height_auto {
            None
        } else {
            Some(
                self.height_spec
                    .resolve_height(containing_height, self.font_size, self.root_font_size)
                    .max(Pt::ZERO),
            )
        };

        let avail_w_for_child = if let Some(explicit_w) = explicit_w {
            explicit_w.max(Pt::ZERO)
        } else if stretch_w {
            (containing_width - left - right).max(Pt::ZERO)
        } else if has_left {
            (containing_width - left).max(Pt::ZERO)
        } else if has_right {
            (containing_width - right).max(Pt::ZERO)
        } else {
            containing_width
        };
        let avail_h_for_child = if let Some(explicit_h) = explicit_h {
            explicit_h.max(Pt::ZERO)
        } else if stretch_h {
            (containing_height - top - bottom).max(Pt::ZERO)
        } else if has_top {
            (containing_height - top).max(Pt::ZERO)
        } else if has_bottom {
            (containing_height - bottom).max(Pt::ZERO)
        } else {
            containing_height
        };

        // Shrink-to-fit for absolutely positioned elements when width is auto.
        // Prefer intrinsic width when available, clamped to the available space.
        let target_w = if let Some(explicit_w) = explicit_w {
            explicit_w.max(Pt::ZERO)
        } else if stretch_w {
            avail_w_for_child
        } else if let Some(intrinsic) = self.child.intrinsic_width() {
            intrinsic.min(avail_w_for_child).max(Pt::ZERO)
        } else {
            avail_w_for_child
        };

        let size = self.child.wrap(target_w, avail_h_for_child);

        let child_w = if let Some(explicit_w) = explicit_w {
            explicit_w.max(Pt::ZERO)
        } else if stretch_w {
            avail_w_for_child
        } else {
            size.width.min(target_w).max(Pt::ZERO)
        };
        let child_h = if let Some(explicit_h) = explicit_h {
            explicit_h.max(Pt::ZERO)
        } else if stretch_h {
            avail_h_for_child
        } else {
            size.height.min(avail_h_for_child).max(Pt::ZERO)
        };

        let child_x = if has_left {
            containing_x + left
        } else if has_right {
            containing_x + (containing_width - right - child_w)
        } else {
            x
        };
        let child_y = if has_top {
            containing_y + top
        } else if has_bottom {
            containing_y + (containing_height - bottom - child_h)
        } else {
            y
        };

        self.child.draw(canvas, child_x, child_y, child_w, child_h);
    }

    fn out_of_flow(&self) -> bool {
        true
    }

    fn z_index(&self) -> i32 {
        self.z_index
    }

    fn pagination(&self) -> Pagination {
        self.pagination
    }

    fn is_fixed_positioned(&self) -> bool {
        self.fixed_positioned
    }
}

impl Flowable for RelativePositionedFlowable {
    fn wrap(&self, avail_width: Pt, avail_height: Pt) -> Size {
        self.child.wrap(avail_width, avail_height)
    }

    fn split(
        &self,
        avail_width: Pt,
        avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        let (first, second) = self.child.split(avail_width, avail_height)?;
        let first = Self {
            child: first,
            left: self.left,
            top: self.top,
            right: self.right,
            bottom: self.bottom,
            font_size: self.font_size,
            root_font_size: self.root_font_size,
            pagination: self.pagination,
        };
        let second = Self {
            child: second,
            left: self.left,
            top: self.top,
            right: self.right,
            bottom: self.bottom,
            font_size: self.font_size,
            root_font_size: self.root_font_size,
            pagination: self.pagination,
        };
        Some((
            Box::new(first) as Box<dyn Flowable>,
            Box::new(second) as Box<dyn Flowable>,
        ))
    }

    fn draw(&self, canvas: &mut Canvas, x: Pt, y: Pt, avail_width: Pt, avail_height: Pt) {
        let _size = self.child.wrap(avail_width, avail_height);

        let has_left = !matches!(self.left, LengthSpec::Auto);
        let has_right = !matches!(self.right, LengthSpec::Auto);
        let has_top = !matches!(self.top, LengthSpec::Auto);
        let has_bottom = !matches!(self.bottom, LengthSpec::Auto);

        // Relative inset percentages resolve against the containing block dimensions.
        // For horizontal offsets use provided available width; for vertical use available
        // height (which is currently the container-provided draw height in this pipeline).
        let offset_width_basis = avail_width.max(Pt::ZERO);
        let offset_height_basis = avail_height.max(Pt::ZERO);

        let left = if has_left {
            self.left
                .resolve_width(offset_width_basis, self.font_size, self.root_font_size)
        } else {
            Pt::ZERO
        };
        let right = if has_right {
            self.right
                .resolve_width(offset_width_basis, self.font_size, self.root_font_size)
        } else {
            Pt::ZERO
        };
        let top = if has_top {
            self.top
                .resolve_height(offset_height_basis, self.font_size, self.root_font_size)
        } else {
            Pt::ZERO
        };
        let bottom = if has_bottom {
            self.bottom
                .resolve_height(offset_height_basis, self.font_size, self.root_font_size)
        } else {
            Pt::ZERO
        };

        // Relative positioning shifts paint only; the element keeps its original flow slot.
        // For phase-1 parity we use left/top when provided, otherwise right/bottom as inverse.
        let dx = if has_left {
            left
        } else if has_right {
            -right
        } else {
            Pt::ZERO
        };
        let dy = if has_top {
            top
        } else if has_bottom {
            -bottom
        } else {
            Pt::ZERO
        };

        self.child
            .draw(canvas, x + dx, y + dy, avail_width, avail_height);
    }

    fn intrinsic_width(&self) -> Option<Pt> {
        self.child.intrinsic_width()
    }

    fn first_baseline(&self, avail_width: Pt) -> Option<Pt> {
        self.child.first_baseline(avail_width)
    }

    fn out_of_flow(&self) -> bool {
        self.child.out_of_flow()
    }

    fn z_index(&self) -> i32 {
        self.child.z_index()
    }

    fn pagination(&self) -> Pagination {
        self.pagination
    }

    fn prefers_containing_block_draw_space(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod grid_and_transform_regression_tests {
    use super::*;
    use crate::canvas::Command;

    #[test]
    fn explicit_grid_tracks_keep_fixed_columns_and_rows_when_they_overflow() {
        let fixed = LengthSpec::Absolute(Pt::from_f32(75.0));
        let items = (0..4)
            .map(|_| {
                (
                    Box::new(Spacer::new_pt(Pt::ZERO)) as Box<dyn Flowable>,
                    0.0,
                    0.0,
                    Some(fixed),
                    None,
                )
            })
            .collect();
        let flex = FlexFlowable::new_pt(
            items,
            FlexDirection::Row,
            JustifyContent::FlexStart,
            AlignItems::Stretch,
            AlignContent::Stretch,
            LengthSpec::Absolute(Pt::ZERO),
            true,
            Pt::from_f32(12.0),
            Pt::from_f32(12.0),
        )
        .with_grid_tracks(
            2,
            vec![GridTrackSize::fixed(fixed), GridTrackSize::fixed(fixed)],
        );

        let layout = flex.compute_layout(Pt::from_f32(147.0), Pt::from_f32(147.0));
        let FlexLayout::RowWrap { lines, container_h } = layout.layout else {
            panic!("expected wrapped grid rows");
        };
        assert_eq!(container_h, Pt::from_f32(147.0));
        assert_eq!(lines.len(), 2);
        for line in lines {
            assert_eq!(line.widths, vec![Pt::from_f32(75.0); 2]);
            assert_eq!(line.line_h, Pt::from_f32(75.0));
        }
    }

    #[test]
    fn css_rotation_uses_clockwise_sign_around_the_default_center() {
        let box_width = Pt::from_f32(75.0);
        let box_height = Pt::from_f32(45.0);
        let flowable =
            ContainerFlowable::new_pt(Vec::new(), Pt::from_f32(12.0), Pt::from_f32(12.0))
                .with_width(LengthSpec::Absolute(box_width))
                .with_height(LengthSpec::Absolute(box_height))
                .with_transforms(vec![CssTransformOp::Rotate { radians: 0.5 }]);
        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(200.0),
            height: Pt::from_f32(200.0),
        });
        flowable.draw(
            &mut canvas,
            Pt::ZERO,
            Pt::ZERO,
            Pt::from_f32(200.0),
            Pt::from_f32(200.0),
        );
        let document = canvas.finish();
        let commands = &document.pages[0].commands;
        let transform = commands.windows(3).find(|window| {
            matches!(
                window,
                [
                    Command::CssTransformOrigin { inverse: false, .. },
                    Command::Rotate(_),
                    Command::CssTransformOrigin { inverse: true, .. }
                ]
            )
        });
        let Some(
            [
                Command::CssTransformOrigin {
                    x: origin_x,
                    y: origin_y,
                    inverse: false,
                },
                Command::Rotate(angle),
                Command::CssTransformOrigin {
                    x: return_x,
                    y: return_y,
                    inverse: true,
                },
            ],
        ) = transform
        else {
            panic!("missing transform-origin command sequence: {commands:?}");
        };
        assert_eq!(*origin_x, box_width.mul_ratio(1, 2));
        assert_eq!(*origin_y, box_height.mul_ratio(1, 2));
        assert!((*angle + 0.5).abs() < f32::EPSILON);
        assert_eq!(*return_x, *origin_x);
        assert_eq!(*return_y, *origin_y);
    }
}
