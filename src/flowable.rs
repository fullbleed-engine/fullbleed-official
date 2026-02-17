use crate::canvas::Canvas;
use crate::font::FontRegistry;
use crate::perf::PerfLogger;
use crate::svg;
use crate::types::{BoxSizingMode, Color, Pt, Shading, ShadingStop, Size};
use rayon::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

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
    Page,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakAfter {
    Auto,
    Page,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakInside {
    Auto,
    Avoid,
    AvoidPage,
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderSpacingSpec {
    pub horizontal: LengthSpec,
    pub vertical: LengthSpec,
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

#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundPaint {
    LinearGradient {
        angle_deg: f32,
        stops: Vec<ShadingStop>,
    },
}

#[derive(Debug, Clone, Copy)]
struct ResolvedEdges {
    top: Pt,
    right: Pt,
    bottom: Pt,
    left: Pt,
}

#[derive(Debug, Clone, Copy)]
struct ResolvedEdgeColors {
    top: Color,
    right: Color,
    bottom: Color,
    left: Color,
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

#[derive(Debug, Clone, Copy)]
struct ResolvedBorder {
    widths: ResolvedEdges,
    colors: ResolvedEdgeColors,
}

pub trait Flowable: FlowableClone + Send + Sync {
    fn wrap(&self, avail_width: Pt, avail_height: Pt) -> Size;
    fn split(
        &self,
        avail_width: Pt,
        avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)>;
    fn draw(&self, canvas: &mut Canvas, x: Pt, y: Pt, avail_width: Pt, avail_height: Pt);

    fn intrinsic_width(&self) -> Option<Pt> {
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

    fn debug_name(&self) -> &'static str {
        std::any::type_name::<Self>()
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

#[derive(Debug, Clone)]
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
    pub text_overflow: crate::style::TextOverflowMode,
    pub word_break: crate::style::WordBreakMode,
    pub letter_spacing: Pt,
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
            text_overflow: crate::style::TextOverflowMode::Clip,
            word_break: crate::style::WordBreakMode::Normal,
            letter_spacing: Pt::ZERO,
        }
    }
}

fn resolve_font_variant_name(
    registry: Option<&FontRegistry>,
    base: &Arc<str>,
    weight: u16,
    style: crate::style::FontStyleMode,
) -> Arc<str> {
    let italic = matches!(style, crate::style::FontStyleMode::Italic);
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
    let overline_thickness = default_thickness;
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

    canvas.save_state();
    canvas.set_stroke_color(style.color);
    if style.text_decoration.overline {
        canvas.set_line_width(overline_thickness);
        canvas.move_to(x, overline_y);
        canvas.line_to(x + width, overline_y);
        canvas.stroke();
    }
    if style.text_decoration.line_through {
        canvas.set_line_width(line_through_thickness);
        canvas.move_to(x, line_through_y);
        canvas.line_to(x + width, line_through_y);
        canvas.stroke();
    }
    if style.text_decoration.underline {
        canvas.set_line_width(underline_thickness);
        canvas.move_to(x, underline_y);
        canvas.line_to(x + width, underline_y);
        canvas.stroke();
    }
    canvas.restore_state();
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

#[derive(Debug, Clone)]
pub struct Paragraph {
    text: String,
    style: TextStyle,
    align: TextAlign,
    pagination: Pagination,
    preserve_whitespace: bool,
    no_wrap: bool,
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
            pagination: Pagination::default(),
            preserve_whitespace: false,
            no_wrap: false,
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
        if let Ok(cache) = self.width_cache.lock() {
            if let Some(value) = cache.get(text) {
                if perf_enabled() {
                    log_perf_counts("layout.text.width", &[("cache_hit", 1)]);
                }
                return value;
            }
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
            let count = text.chars().count();
            if count > 1 && self.style.letter_spacing != Pt::ZERO {
                let value = base + self.style.letter_spacing * ((count - 1) as i32);
                if let Ok(mut cache) = self.width_cache.lock() {
                    cache.insert(text, value);
                }
                if perf_enabled() {
                    log_perf_counts("layout.text.width", &[("cache_miss", 1)]);
                }
                value
            } else {
                if let Ok(mut cache) = self.width_cache.lock() {
                    cache.insert(text, base);
                }
                if perf_enabled() {
                    log_perf_counts("layout.text.width", &[("cache_miss", 1)]);
                }
                base
            }
        } else {
            let char_width = (self.style.font_size * 0.6).max(Pt::from_f32(1.0));
            let count = text.chars().count();
            let base = char_width * (count as i32);
            if count > 1 && self.style.letter_spacing != Pt::ZERO {
                let value = base + self.style.letter_spacing * ((count - 1) as i32);
                if let Ok(mut cache) = self.width_cache.lock() {
                    cache.insert(text, value);
                }
                if perf_enabled() {
                    log_perf_counts("layout.text.width", &[("cache_miss", 1)]);
                }
                value
            } else {
                if let Ok(mut cache) = self.width_cache.lock() {
                    cache.insert(text, base);
                }
                if perf_enabled() {
                    log_perf_counts("layout.text.width", &[("cache_miss", 1)]);
                }
                base
            }
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

    fn draw_text_with_fallbacks(&self, canvas: &mut Canvas, x: Pt, y: Pt, text: &str) {
        if let Some(registry) = &self.font_registry {
            let (primary, fallbacks) = resolve_font_stack(Some(registry), &self.style);
            let runs = registry.split_text_by_fallbacks(&primary, &fallbacks, text);
            let mut cursor_x = x;
            let mut remaining = text.chars().count();
            for run in runs {
                canvas.set_font_name(&run.font_name);
                if self.style.letter_spacing == Pt::ZERO {
                    let run_text = run.text;
                    let run_len = run_text.chars().count();
                    let w = registry.measure_text_width(
                        &run.font_name,
                        self.style.font_size,
                        &run_text,
                    );
                    canvas.draw_string(cursor_x, y, run_text);
                    cursor_x = cursor_x + w;
                    remaining = remaining.saturating_sub(run_len);
                } else {
                    for ch in run.text.chars() {
                        let ch_str = ch.to_string();
                        canvas.draw_string(cursor_x, y, ch_str.clone());
                        let w = registry.measure_text_width(
                            &run.font_name,
                            self.style.font_size,
                            &ch_str,
                        );
                        remaining = remaining.saturating_sub(1);
                        if remaining > 0 {
                            cursor_x = cursor_x + w + self.style.letter_spacing;
                        } else {
                            cursor_x = cursor_x + w;
                        }
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
        if self.style.letter_spacing == Pt::ZERO {
            canvas.draw_string(x, y, text);
        } else {
            let mut cursor_x = x;
            let mut remaining = text.chars().count();
            let char_width = (self.style.font_size * 0.6).max(Pt::from_f32(1.0));
            for ch in text.chars() {
                let ch_str = ch.to_string();
                canvas.draw_string(cursor_x, y, ch_str);
                remaining = remaining.saturating_sub(1);
                if remaining > 0 {
                    cursor_x = cursor_x + char_width + self.style.letter_spacing;
                } else {
                    cursor_x = cursor_x + char_width;
                }
            }
        }
    }

    fn layout_lines(&self, avail_width: Pt) -> Arc<Vec<LineLayout>> {
        let perf = perf_start();
        let max_width = avail_width.max(Pt::from_f32(1.0));
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
            for line in self.text.split('\n') {
                let resolved = if line.is_empty() {
                    String::new()
                } else if matches!(
                    self.style.text_overflow,
                    crate::style::TextOverflowMode::Ellipsis
                ) {
                    truncate_text_with_ellipsis(self, line, max_width)
                } else {
                    line.to_string()
                };
                let width = if resolved.is_empty() {
                    Pt::ZERO
                } else {
                    self.measure_text_width(&resolved)
                };
                line_layouts.push(LineLayout {
                    text: resolved,
                    width,
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

        let allow_break_long = matches!(
            self.style.word_break,
            crate::style::WordBreakMode::BreakWord
                | crate::style::WordBreakMode::BreakAll
                | crate::style::WordBreakMode::Anywhere
        );

        let mut lines = Vec::new();
        let mut word_widths: HashMap<&str, Pt> = HashMap::new();
        if self.preserve_whitespace {
            if !allow_break_long {
                for segment in self.text.split('\n') {
                    lines.push(segment.to_string());
                }
            } else {
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
                            if allow_break_long {
                                lines.extend(split_long_word_by_width(self, word, max_width));
                                current.clear();
                            } else {
                                lines.push(word.to_string());
                                current.clear();
                            }
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
                                if allow_break_long {
                                    lines.extend(split_long_word_by_width(self, word, max_width));
                                } else {
                                    lines.push(word.to_string());
                                }
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
            line_layouts.push(LineLayout { text: line, width });
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

impl Flowable for Paragraph {
    fn wrap(&self, avail_width: Pt, _avail_height: Pt) -> Size {
        let perf = perf_start();
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
        let mut max_w = Pt::ZERO;
        for line in self.text.split('\n') {
            max_w = max_w.max(self.measure_text_width(line));
        }
        Some(max_w)
    }

    fn split(
        &self,
        avail_width: Pt,
        avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
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
            pagination: Pagination {
                break_before: BreakBefore::Auto,
                break_after: BreakAfter::Auto,
                ..self.pagination
            },
            preserve_whitespace: self.preserve_whitespace,
            no_wrap: self.no_wrap,
            tag_role: self.tag_role.clone(),
            font_registry: self.font_registry.clone(),
            layout_cache: Arc::new(Mutex::new(TextLayoutCache::default())),
            width_cache: Arc::new(Mutex::new(TextWidthCache::default())),
        };
        let second = Paragraph {
            text: second_text,
            style: self.style.clone(),
            align: self.align,
            pagination: Pagination {
                break_before: BreakBefore::Auto,
                ..self.pagination
            },
            preserve_whitespace: self.preserve_whitespace,
            no_wrap: self.no_wrap,
            tag_role: self.tag_role.clone(),
            font_registry: self.font_registry.clone(),
            layout_cache: Arc::new(Mutex::new(TextLayoutCache::default())),
            width_cache: Arc::new(Mutex::new(TextWidthCache::default())),
        };
        Some((Box::new(first), Box::new(second)))
    }

    fn draw(&self, canvas: &mut Canvas, x: Pt, y: Pt, avail_width: Pt, _avail_height: Pt) {
        let perf = perf_start();
        let lines = self.layout_lines(avail_width);
        let tagged = self.tag_role.as_ref().map(|role| {
            canvas.begin_tag(role.as_ref(), None, None, None, None, false);
        });
        canvas.set_fill_color(self.style.color);
        canvas.set_font_size(self.style.font_size);

        let mut cursor_y = y;
        let line_height = self.effective_line_height();
        for line in lines.iter() {
            let line_width = line.width;
            let offset = match self.align {
                TextAlign::Left => Pt::ZERO,
                TextAlign::Center => ((avail_width - line_width).max(Pt::ZERO)).mul_ratio(1, 2),
                TextAlign::Right => (avail_width - line_width).max(Pt::ZERO),
            };
            self.draw_text_with_fallbacks(canvas, x + offset, cursor_y, &line.text);
            draw_text_decorations(
                canvas,
                &self.style,
                self.font_registry.as_deref(),
                x + offset,
                cursor_y,
                line_width,
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

#[derive(Clone)]
pub struct ListItemFlowable {
    label: Paragraph,
    body: Box<dyn Flowable>,
    gap: Pt,
    pagination: Pagination,
}

impl ListItemFlowable {
    pub fn new(label: Paragraph, body: Box<dyn Flowable>, gap: Pt) -> Self {
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
pub struct ImageFlowable {
    pub width: Pt,
    pub height: Pt,
    pub resource_id: String,
    tag_role: Option<Arc<str>>,
    alt: Option<String>,
    pagination: Pagination,
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
            tag_role: None,
            alt: None,
            pagination: Pagination::default(),
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

    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = pagination;
        self
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
        let tagged = self.tag_role.as_ref().map(|role| {
            canvas.begin_tag(role.as_ref(), self.alt.clone(), None, None, None, false);
        });
        canvas.draw_image(x, y, self.width, self.height, self.resource_id.clone());
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

#[derive(Debug, Clone, Copy)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy)]
pub enum VerticalAlign {
    Top,
    Middle,
    Bottom,
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
    pub box_shadow: Option<BoxShadowSpec>,
    pub tag_role: Option<Arc<str>>,
    pub scope: Option<String>,
    col_span: usize,
    pub root_font_size: Pt,
    row_min_height: Pt,
    preferred_width: Option<LengthSpec>,
    preferred_width_font_size: Pt,
    preferred_width_root_font_size: Pt,
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
            box_shadow,
            tag_role,
            scope,
            col_span: col_span.max(1),
            root_font_size,
            row_min_height: Pt::ZERO,
            preferred_width: None,
            preferred_width_font_size: style_font_size,
            preferred_width_root_font_size: root_font_size,
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

    fn measure_text_width(&self, text: &str) -> Pt {
        if let Ok(cache) = self.width_cache.lock() {
            if let Some(value) = cache.get(text) {
                if perf_enabled() {
                    log_perf_counts("layout.tablecell.width", &[("cache_hit", 1)]);
                }
                return value;
            }
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
            let count = text.chars().count();
            if count > 1 && self.style.letter_spacing != Pt::ZERO {
                let value = base + self.style.letter_spacing * ((count - 1) as i32);
                if let Ok(mut cache) = self.width_cache.lock() {
                    cache.insert(text, value);
                }
                if perf_enabled() {
                    log_perf_counts("layout.tablecell.width", &[("cache_miss", 1)]);
                }
                value
            } else {
                if let Ok(mut cache) = self.width_cache.lock() {
                    cache.insert(text, base);
                }
                if perf_enabled() {
                    log_perf_counts("layout.tablecell.width", &[("cache_miss", 1)]);
                }
                base
            }
        } else {
            let char_width = (self.style.font_size * 0.6).max(Pt::from_f32(1.0));
            let count = text.chars().count();
            let base = char_width * (count as i32);
            if count > 1 && self.style.letter_spacing != Pt::ZERO {
                let value = base + self.style.letter_spacing * ((count - 1) as i32);
                if let Ok(mut cache) = self.width_cache.lock() {
                    cache.insert(text, value);
                }
                if perf_enabled() {
                    log_perf_counts("layout.tablecell.width", &[("cache_miss", 1)]);
                }
                value
            } else {
                if let Ok(mut cache) = self.width_cache.lock() {
                    cache.insert(text, base);
                }
                if perf_enabled() {
                    log_perf_counts("layout.tablecell.width", &[("cache_miss", 1)]);
                }
                base
            }
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
            line_layouts.push(LineLayout { text: line, width });
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

    fn draw_inset_box_shadow(&self, canvas: &mut Canvas, x: Pt, y: Pt, width: Pt, height: Pt) {
        let Some(shadow) = self.box_shadow.as_ref() else {
            return;
        };
        if !shadow.inset || shadow.opacity <= 0.0 {
            return;
        }

        let offset_x =
            shadow
                .offset_x
                .resolve_width(width, self.style.font_size, self.root_font_size);
        let offset_y =
            shadow
                .offset_y
                .resolve_height(height, self.style.font_size, self.root_font_size);
        let blur = shadow
            .blur
            .resolve_width(width, self.style.font_size, self.root_font_size)
            .max(Pt::ZERO);
        let spread = shadow
            .spread
            .resolve_width(width, self.style.font_size, self.root_font_size)
            .max(Pt::ZERO);

        let extra = spread + blur;
        let shadow_x = x + offset_x - extra;
        let shadow_y = y + offset_y - extra;
        let shadow_w = (width + extra * 2).max(Pt::ZERO);
        let shadow_h = (height + extra * 2).max(Pt::ZERO);

        canvas.save_state();
        canvas.clip_rect(x, y, width, height);
        canvas.set_opacity(
            shadow.opacity.clamp(0.0, 1.0),
            shadow.opacity.clamp(0.0, 1.0),
        );
        canvas.set_fill_color(shadow.color);
        canvas.draw_rect(shadow_x, shadow_y, shadow_w, shadow_h);
        canvas.set_opacity(1.0, 1.0);
        canvas.restore_state();
    }

    fn draw_text_line(&self, canvas: &mut Canvas, x: Pt, y: Pt, text: &str) {
        if let Some(registry) = self.font_registry.as_deref() {
            let (primary, fallbacks) = resolve_font_stack(Some(registry), &self.style);
            let runs = registry.split_text_by_fallbacks(&primary, &fallbacks, text);
            let mut cursor_x = x;
            let mut remaining = text.chars().count();
            for run in runs {
                canvas.set_font_name(&run.font_name);
                if self.style.letter_spacing == Pt::ZERO {
                    let run_text = run.text;
                    let run_len = run_text.chars().count();
                    let w = registry.measure_text_width(
                        &run.font_name,
                        self.style.font_size,
                        &run_text,
                    );
                    canvas.draw_string(cursor_x, y, run_text);
                    cursor_x = cursor_x + w;
                    remaining = remaining.saturating_sub(run_len);
                } else {
                    for ch in run.text.chars() {
                        let ch_str = ch.to_string();
                        canvas.draw_string(cursor_x, y, ch_str.clone());
                        let w = registry.measure_text_width(
                            &run.font_name,
                            self.style.font_size,
                            &ch_str,
                        );
                        remaining = remaining.saturating_sub(1);
                        if remaining > 0 {
                            cursor_x = cursor_x + w + self.style.letter_spacing;
                        } else {
                            cursor_x = cursor_x + w;
                        }
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
        if self.style.letter_spacing == Pt::ZERO {
            canvas.draw_string(x, y, text);
        } else {
            let mut cursor_x = x;
            let mut remaining = text.chars().count();
            let char_width = (self.style.font_size * 0.6).max(Pt::from_f32(1.0));
            for ch in text.chars() {
                let ch_str = ch.to_string();
                canvas.draw_string(cursor_x, y, ch_str);
                remaining = remaining.saturating_sub(1);
                if remaining > 0 {
                    cursor_x = cursor_x + char_width + self.style.letter_spacing;
                } else {
                    cursor_x = cursor_x + char_width;
                }
            }
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
        max_cols.max(1)
    }

    fn row_total_columns(row: &[TableCell]) -> usize {
        row.iter().map(TableCell::col_span).sum::<usize>().max(1)
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

    fn row_height(row: &[TableCell], col_widths: &[Pt]) -> Pt {
        let mut max_height = Pt::ZERO;
        let mut cursor_col = 0usize;
        let total_columns = col_widths.len().max(1);
        for cell in row.iter() {
            let col_span = Self::cell_span_for_start(cell, cursor_col, total_columns);
            let col_width = Self::span_width(col_widths, cursor_col, col_span);
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
        if matches!(self.border_collapse, BorderCollapseMode::Separate) {
            return Self::row_height(row, col_widths);
        }
        let mut max_height = Pt::ZERO;
        let mut cursor_col = 0usize;
        let total_columns = col_widths.len().max(1);
        for cell in row.iter() {
            let col_span = Self::cell_span_for_start(cell, cursor_col, total_columns);
            let col_width = Self::span_width(col_widths, cursor_col, col_span);
            let padding = cell.resolved_padding(col_width);
            let border = self
                .collapsed_border_for_cell(
                    draw_row_index,
                    cursor_col,
                    col_span,
                    col_widths,
                    cell,
                )
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
            let padding = cell.resolved_padding(col_width);
            let border = self
                .collapsed_border_for_cell(
                    draw_row_index,
                    cursor_col,
                    col_span,
                    col_widths,
                    cell,
                )
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
        current_width: Pt,
        current_color: Color,
        candidate_width: Pt,
        candidate_color: Color,
    ) -> (Pt, Color) {
        if candidate_width > current_width {
            (candidate_width, candidate_color)
        } else {
            (current_width, current_color)
        }
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
        let col_span = col_span.max(1).min(total_columns.saturating_sub(col_start).max(1));
        let col_end = col_start.saturating_add(col_span);
        let col_width = Self::span_width(col_widths, col_start, col_span);
        let mut widths = cell.resolved_border(col_width);
        let mut colors = ResolvedEdgeColors::uniform(cell.border.color);

        if col_end < total_columns {
            if let Some((right_cell, right_start, right_span)) =
                self.cell_layout_by_draw_index(row_index, col_end, total_columns)
            {
                let right_col_width = Self::span_width(col_widths, right_start, right_span);
                let right_border = right_cell.resolved_border(right_col_width);
                let (width, color) = Self::stronger_edge(
                    widths.right,
                    colors.right,
                    right_border.left,
                    right_cell.border.color,
                );
                widths.right = width;
                colors.right = color;
            }
        }

        for below_col in col_start..col_end {
            if let Some((below_cell, below_start, below_span)) =
                self.cell_layout_by_draw_index(row_index + 1, below_col, total_columns)
            {
                let below_col_width = Self::span_width(col_widths, below_start, below_span);
                let below_border = below_cell.resolved_border(below_col_width);
                let (width, color) = Self::stronger_edge(
                    widths.bottom,
                    colors.bottom,
                    below_border.top,
                    below_cell.border.color,
                );
                widths.bottom = width;
                colors.bottom = color;
            }
        }

        if row_index > 0 {
            widths.top = Pt::ZERO;
        }
        if col_start > 0 {
            widths.left = Pt::ZERO;
        }

        ResolvedBorder { widths, colors }
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
        let mut cursor_x = x;
        let total_columns = col_widths.len().max(1);
        let mut cursor_col = 0usize;
        for (cell_index, cell) in row.iter().enumerate() {
            let col_span = Self::cell_span_for_start(cell, cursor_col, total_columns);
            let internal_gaps = if col_span > 1 {
                col_gap * ((col_span - 1) as i32)
            } else {
                Pt::ZERO
            };
            let col_width = Self::span_width(col_widths, cursor_col, col_span) + internal_gaps;
            let cell_x = cursor_x;
            let cell_y = y;
            let padding = cell.resolved_padding(col_width);
            let (border, border_colors) =
                if matches!(self.border_collapse, BorderCollapseMode::Collapse) {
                    let resolved = self.collapsed_border_for_cell(
                        row_index,
                        cursor_col,
                        col_span,
                        col_widths,
                        cell,
                    );
                    (resolved.widths, resolved.colors)
                } else {
                    (
                        cell.resolved_border(col_width),
                        ResolvedEdgeColors::uniform(cell.border.color),
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
                );
            }

            let content_width = (col_width - pad_left - pad_right).max(Pt::ZERO);
            let content_height = (row_height - pad_top - pad_bottom).max(Pt::ZERO);
            if let Some(content) = cell.content.as_ref() {
                let wrapped = content.wrap(content_width, content_height);
                let draw_h = wrapped.height.min(content_height).max(Pt::ZERO);
                let draw_y = match cell.valign {
                    VerticalAlign::Top => cell_y + pad_top,
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
                    VerticalAlign::Top => cell_y + pad_top,
                    VerticalAlign::Middle => {
                        cell_y + pad_top + (content_height - text_block_height).mul_ratio(1, 2)
                    }
                    VerticalAlign::Bottom => cell_y + row_height - pad_bottom - text_block_height,
                };

                canvas.set_fill_color(cell.style.color);
                canvas.set_font_size(cell.style.font_size);
                let mut cursor_y = text_y.max(cell_y + pad_top);
                for line in lines.iter() {
                    let line_width = line.width.min(content_width);
                    let text_x = match cell.align {
                        TextAlign::Left => cell_x + pad_left,
                        TextAlign::Center => {
                            cell_x + pad_left + (content_width - line_width).mul_ratio(1, 2)
                        }
                        TextAlign::Right => cell_x + col_width - pad_right - line_width,
                    };
                    cell.draw_text_line(canvas, text_x, cursor_y, &line.text);
                    draw_text_decorations(
                        canvas,
                        &cell.style,
                        cell.font_registry.as_deref(),
                        text_x,
                        cursor_y,
                        line_width,
                    );
                    cursor_y = cursor_y + line_height;
                }
            }

            if tagged.is_some() {
                canvas.end_tag();
            }
            cursor_x = cursor_x + col_width;
            cursor_col = cursor_col.saturating_add(col_span);
            if cursor_col < total_columns {
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
    ) {
        if border.top > Pt::ZERO {
            canvas.set_fill_color(colors.top);
            canvas.draw_rect(x, y, width, border.top);
        }
        if border.bottom > Pt::ZERO {
            canvas.set_fill_color(colors.bottom);
            canvas.draw_rect(x, y + height - border.bottom, width, border.bottom);
        }
        if border.left > Pt::ZERO {
            canvas.set_fill_color(colors.left);
            canvas.draw_rect(x, y, border.left, height);
        }
        if border.right > Pt::ZERO {
            canvas.set_fill_color(colors.right);
            canvas.draw_rect(x + width - border.right, y, border.right, height);
        }
    }
}

impl Flowable for TableFlowable {
    fn wrap(&self, avail_width: Pt, _avail_height: Pt) -> Size {
        let perf = perf_start();
        let columns = self.max_columns();
        let (col_gap, row_gap) = self.resolve_spacing(avail_width);
        let gap_total = if columns > 1 {
            col_gap * ((columns - 1) as i32)
        } else {
            Pt::ZERO
        };
        let avail_cols_width = (avail_width - gap_total).max(Pt::ZERO);
        let mut height = Pt::ZERO;
        let cache = if matches!(self.border_collapse, BorderCollapseMode::Collapse) {
            None
        } else {
            self.data.cache_for_width(avail_cols_width, columns)
        };
        if let Some(cache) = cache {
            if self.include_header {
                height += cache.header_total;
                if !self.data.header_rows.is_empty()
                    && !self.data.body_rows[self.body_range.clone()].is_empty()
                {
                    height += row_gap;
                }
                if self.data.header_rows.len() > 1 {
                    height += row_gap * ((self.data.header_rows.len() - 1) as i32);
                }
            }
            let body_count = self.body_range.end.saturating_sub(self.body_range.start);
            if body_count > 0 {
                height += cache.body_prefix[self.body_range.end]
                    - cache.body_prefix[self.body_range.start];
                if body_count > 1 {
                    height += row_gap * ((body_count - 1) as i32);
                }
            }
        } else {
            let col_widths = self.data.compute_column_widths(avail_cols_width, columns);
            let mut row_index = 0usize;
            if self.include_header {
                for row in &self.data.header_rows {
                    height += self.row_height_for_draw_index(row_index, row, &col_widths);
                    row_index += 1;
                }
                if !self.data.header_rows.is_empty()
                    && !self.data.body_rows[self.body_range.clone()].is_empty()
                {
                    height += row_gap;
                }
                if self.data.header_rows.len() > 1 {
                    height += row_gap * ((self.data.header_rows.len() - 1) as i32);
                }
            }
            for row in &self.data.body_rows[self.body_range.clone()] {
                height += self.row_height_for_draw_index(row_index, row, &col_widths);
                row_index += 1;
            }
            let body_count = self.body_range.end.saturating_sub(self.body_range.start);
            if body_count > 1 {
                height += row_gap * ((body_count - 1) as i32);
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
        let gap_total = if columns > 1 {
            col_gap * ((columns - 1) as i32)
        } else {
            Pt::ZERO
        };
        let avail_cols_width = (avail_width - gap_total).max(Pt::ZERO);
        let cache = if matches!(self.border_collapse, BorderCollapseMode::Collapse) {
            None
        } else {
            self.data.cache_for_width(avail_cols_width, columns)
        };
        let header_height = if self.include_header {
            if let Some(cache) = cache {
                let mut height = cache.header_total;
                if self.data.header_rows.len() > 1 {
                    height += row_gap * ((self.data.header_rows.len() - 1) as i32);
                }
                if !self.data.header_rows.is_empty()
                    && !self.data.body_rows[self.body_range.clone()].is_empty()
                {
                    height += row_gap;
                }
                height
            } else {
                let col_widths = self.data.compute_column_widths(avail_cols_width, columns);
                let mut height = Pt::ZERO;
                for (row_index, row) in self.data.header_rows.iter().enumerate() {
                    height += self.row_height_for_draw_index(row_index, row, &col_widths);
                }
                if self.data.header_rows.len() > 1 {
                    height += row_gap * ((self.data.header_rows.len() - 1) as i32);
                }
                if !self.data.header_rows.is_empty()
                    && !self.data.body_rows[self.body_range.clone()].is_empty()
                {
                    height += row_gap;
                }
                height
            }
        } else {
            Pt::ZERO
        };
        let available = avail_height - header_height;
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
                let count = mid.saturating_sub(start);
                if count > 1 {
                    used = used + row_gap * ((count - 1) as i32);
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
            let col_widths = self.data.compute_column_widths(avail_cols_width, columns);
            let mut used = Pt::ZERO;
            let mut idx = start;
            let mut draw_row_index = if self.include_header {
                self.data.header_rows.len()
            } else {
                0
            };
            for row in &self.data.body_rows[self.body_range.clone()] {
                let row_height = self.row_height_for_draw_index(draw_row_index, row, &col_widths);
                let gap = if idx > start { row_gap } else { Pt::ZERO };
                if used + gap + row_height > available {
                    break;
                }
                used = used + gap + row_height;
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
        let gap_total = if columns > 1 {
            col_gap * ((columns - 1) as i32)
        } else {
            Pt::ZERO
        };
        let avail_cols_width = (avail_width - gap_total).max(Pt::ZERO);
        let cache = if matches!(self.border_collapse, BorderCollapseMode::Collapse) {
            None
        } else {
            self.data.cache_for_width(avail_cols_width, columns)
        };
        let col_widths = if let Some(cache) = cache {
            std::borrow::Cow::Borrowed(cache.col_widths.as_slice())
        } else {
            std::borrow::Cow::Owned(self.data.compute_column_widths(avail_cols_width, columns))
        };
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
        let mut cursor_y = y;
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
                if idx + 1 < self.data.header_rows.len()
                    || (!self.data.header_rows.is_empty()
                        && !self.data.body_rows[self.body_range.clone()].is_empty())
                {
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
                if i + 1 < self.data.body_rows[self.body_range.clone()].len() {
                    cursor_y = cursor_y + row_gap;
                }
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
            if i + 1 < self.data.body_rows[self.body_range.clone()].len() {
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
    layout_cache: std::sync::OnceLock<TableLayoutCache>,
}

impl TableFlowableData {
    fn cache_for_width(&self, avail_width: Pt, columns: usize) -> Option<&TableLayoutCache> {
        let key = avail_width.to_milli_i64();
        if let Some(existing) = self.layout_cache.get() {
            if existing.avail_width_milli == key && existing.col_widths.len() == columns {
                return Some(existing);
            }
            // Unexpected: width changed. Prefer correctness over caching.
            return None;
        }
        Some(
            self.layout_cache
                .get_or_init(|| TableLayoutCache::new(self, avail_width, columns)),
        )
    }

    fn compute_column_widths(&self, avail_width: Pt, columns: usize) -> Vec<Pt> {
        let columns = columns.max(1);
        let approx_col = avail_width / (columns as i32);
        let debug_verbose = table_debug_enabled() && table_debug_verbose_enabled();
        let data_ptr = self as *const TableFlowableData as usize;
        if debug_verbose {
            eprintln!(
                "[table.debug.widths.begin] data_ptr=0x{:x} columns={} avail_width_pt={:.3} approx_col_pt={:.3} header_rows={} body_rows={}",
                data_ptr,
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
                let col_span = cell.col_span().min(columns.saturating_sub(cursor_col)).max(1);
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
                    let wrapped =
                        content.wrap(span_width.max(Pt::from_f32(1.0)), huge_pt()).width;
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
            let merge =
                |mut a: (Vec<i64>, Vec<i64>, Vec<i64>), b: (Vec<i64>, Vec<i64>, Vec<i64>)| {
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
            let (min_h, max_h, pref_h) = self
                .header_rows
                .par_iter()
                .fold(
                    || (vec![0i64; columns], vec![0i64; columns], vec![0i64; columns]),
                    |mut acc, row| {
                        update_row("header", 0, row, &mut acc.0, &mut acc.1, &mut acc.2);
                        acc
                    },
                )
                .reduce(
                    || (vec![0i64; columns], vec![0i64; columns], vec![0i64; columns]),
                    merge,
                );
            let (min_b, max_b, pref_b) = self
                .body_rows
                .par_iter()
                .fold(
                    || (vec![0i64; columns], vec![0i64; columns], vec![0i64; columns]),
                    |mut acc, row| {
                        update_row("body", 0, row, &mut acc.0, &mut acc.1, &mut acc.2);
                        acc
                    },
                )
                .reduce(
                    || (vec![0i64; columns], vec![0i64; columns], vec![0i64; columns]),
                    merge,
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
                let mut used = 0i64;
                for i in 0..columns {
                    let w = (min_widths[i] as i128 * avail as i128 / total_min as i128) as i64;
                    widths[i] = w;
                    used += w;
                }
                let mut rem = avail - used;
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
                "[table.debug.widths.end] data_ptr=0x{:x} avail_milli={} total_min={} total_max={} min={:?} max={:?} pref={:?} out={:?}",
                data_ptr,
                avail,
                total_min,
                total_max,
                min_widths,
                max_widths,
                preferred_widths,
                widths
            );
        }

        widths.into_iter().map(Pt::from_milli_i64).collect()
    }
}

impl Clone for TableFlowableData {
    fn clone(&self) -> Self {
        Self {
            header_rows: self.header_rows.clone(),
            body_rows: self.body_rows.clone(),
            body_row_meta: self.body_row_meta.clone(),
            layout_cache: std::sync::OnceLock::new(),
        }
    }
}

#[derive(Debug)]
struct TableLayoutCache {
    avail_width_milli: i64,
    col_widths: Vec<Pt>,
    header_row_heights: Vec<Pt>,
    body_row_heights: Vec<Pt>,
    header_row_lines: Vec<Vec<Arc<Vec<LineLayout>>>>,
    body_row_lines: Vec<Vec<Arc<Vec<LineLayout>>>>,
    header_total: Pt,
    body_prefix: Vec<Pt>,
}

impl TableLayoutCache {
    fn new(data: &TableFlowableData, avail_width: Pt, columns: usize) -> Self {
        let col_widths = data.compute_column_widths(avail_width, columns);
        let mut header_row_heights = Vec::with_capacity(data.header_rows.len());
        let mut header_row_lines = Vec::with_capacity(data.header_rows.len());
        let mut header_total = Pt::ZERO;
        let header_results: Vec<(Pt, Vec<Arc<Vec<LineLayout>>>)> = if data.header_rows.len() >= 32 {
            data.header_rows
                .par_iter()
                .map(|row| TableLayoutCache::row_height_and_lines(row, &col_widths))
                .collect()
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
            data.body_rows
                .par_iter()
                .map(|row| TableLayoutCache::row_height_and_lines(row, &col_widths))
                .collect()
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
        let mut max_height = Pt::ZERO;
        let mut lines_out: Vec<Arc<Vec<LineLayout>>> = Vec::with_capacity(row.len());
        let mut cursor_col = 0usize;
        let total_columns = col_widths.len().max(1);
        for cell in row.iter() {
            let col_span = TableFlowable::cell_span_for_start(cell, cursor_col, total_columns);
            let col_width = TableFlowable::span_width(col_widths, cursor_col, col_span);
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
}

#[derive(Clone)]
struct InlineLineLayout {
    line_height: Pt,
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
            pagination: Pagination::default(),
            layout_cache: Arc::new(Mutex::new(None)),
        }
    }

    fn compute_layout(&self, avail_width: Pt) -> InlineLayoutCache {
        let forced = self.forced_line_height.unwrap_or(Pt::ZERO);
        let mut max_width = Pt::ZERO;
        let mut total_height = Pt::ZERO;
        let mut lines: Vec<InlineLineLayout> = Vec::new();

        let mut line_items: Vec<InlineItemLayout> = Vec::new();
        let mut line_width = Pt::ZERO;
        let mut line_height = forced;

        let flush_line = |lines: &mut Vec<InlineLineLayout>,
                          line_items: &mut Vec<InlineItemLayout>,
                          line_width: Pt,
                          line_height: Pt,
                          max_width: &mut Pt,
                          total_height: &mut Pt| {
            if line_items.is_empty() {
                return;
            }
            *total_height = *total_height + line_height;
            *max_width = (*max_width).max(line_width);
            let items = std::mem::take(line_items);
            lines.push(InlineLineLayout { line_height, items });
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignItems {
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
}

#[derive(Clone)]
pub struct FlexItem {
    child: Box<dyn Flowable>,
    grow: f32,
    #[allow(dead_code)]
    shrink: f32,
    basis: Option<LengthSpec>,
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
    gap: LengthSpec,
    wrap: bool,
    font_size: Pt,
    root_font_size: Pt,
    pagination: Pagination,
    layout_cache: Arc<Mutex<Option<FlexLayoutCache>>>,
}

impl FlexFlowable {
    pub fn new_pt(
        items: Vec<(Box<dyn Flowable>, f32, f32, Option<LengthSpec>)>,
        direction: FlexDirection,
        justify: JustifyContent,
        align: AlignItems,
        gap: LengthSpec,
        wrap: bool,
        font_size: Pt,
        root_font_size: Pt,
    ) -> Self {
        Self {
            items: items
                .into_iter()
                .map(|(child, grow, shrink, basis)| FlexItem {
                    child,
                    grow: grow.max(0.0),
                    shrink: shrink.max(0.0),
                    basis,
                })
                .collect(),
            direction,
            justify,
            align,
            gap,
            wrap,
            font_size,
            root_font_size,
            pagination: Pagination::default(),
            layout_cache: Arc::new(Mutex::new(None)),
        }
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
            gap: self.gap,
            wrap: self.wrap,
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
                    let mut total_h = Pt::ZERO;
                    let mut line_layouts: Vec<FlexLineLayout> = Vec::new();
                    for line in &lines {
                        let (widths, child_avails, sizes, line_h) =
                            self.row_line_layout(line, avail_width, avail_height);
                        total_h = total_h + line_h;
                        line_layouts.push(FlexLineLayout {
                            indices: line.clone(),
                            widths,
                            child_avails,
                            sizes,
                            line_h,
                        });
                    }
                    let container_h = Self::bounded_height(avail_height).unwrap_or(total_h);
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
                    _ => {}
                }

                let mut cursor_x = x + start_x;
                for (idx, item) in self.items.iter().enumerate() {
                    let size = sizes[idx];
                    let y_off = match self.align {
                        AlignItems::Center => (*container_h - size.height).mul_ratio(1, 2),
                        AlignItems::FlexEnd => *container_h - size.height,
                        _ => Pt::ZERO,
                    }
                    .max(Pt::ZERO);

                    item.child
                        .draw(canvas, cursor_x, y + y_off, child_avails[idx], *container_h);
                    cursor_x = cursor_x + widths[idx];
                    if idx + 1 < n {
                        cursor_x = cursor_x + gap;
                    }
                }
            }
            FlexLayout::RowWrap { lines, .. } => {
                let mut cursor_y = y;
                for line in lines {
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
                        _ => {}
                    }

                    let mut cursor_x = x + start_x;
                    for (pos, idx) in line.indices.iter().enumerate() {
                        let size = line.sizes[pos];
                        let y_off = match self.align {
                            AlignItems::Center => (line.line_h - size.height).mul_ratio(1, 2),
                            AlignItems::FlexEnd => line.line_h - size.height,
                            _ => Pt::ZERO,
                        }
                        .max(Pt::ZERO);

                        self.items[*idx].child.draw(
                            canvas,
                            cursor_x,
                            cursor_y + y_off,
                            line.child_avails[pos],
                            line.line_h,
                        );
                        cursor_x = cursor_x + line.widths[pos];
                        if pos + 1 < line.indices.len() {
                            cursor_x = cursor_x + gap;
                        }
                    }
                    cursor_y = cursor_y + line.line_h;
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
                    _ => {}
                }

                let mut cursor_y = y + start_y;
                for (idx, item) in self.items.iter().enumerate() {
                    let size = sizes[idx];
                    let x_off = match self.align {
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
    border_color: Color,
    border_radius: BorderRadiusSpec,
    padding: EdgeSizes,
    width: LengthSpec,
    max_width: LengthSpec,
    height: LengthSpec,
    box_sizing: BoxSizingMode,
    background: Option<Color>,
    background_paint: Option<BackgroundPaint>,
    box_shadow: Option<BoxShadowSpec>,
    overflow_hidden: bool,
    tag_role: Option<Arc<str>>,
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
            border_color: Color::BLACK,
            border_radius: BorderRadiusSpec::zero(),
            padding: EdgeSizes::zero(),
            width: LengthSpec::Auto,
            max_width: LengthSpec::Auto,
            height: LengthSpec::Auto,
            box_sizing: BoxSizingMode::ContentBox,
            background: None,
            background_paint: None,
            box_shadow: None,
            overflow_hidden: false,
            tag_role: None,
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
        self.border_color = border_color;
        self
    }

    pub fn with_border_radius(mut self, radius: BorderRadiusSpec) -> Self {
        self.border_radius = radius;
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

    pub fn with_height(mut self, height: LengthSpec) -> Self {
        self.height = height;
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
        self.background_paint = paint;
        self
    }

    pub fn with_box_shadow(mut self, shadow: Option<BoxShadowSpec>) -> Self {
        self.box_shadow = shadow;
        self
    }

    pub fn with_overflow_hidden(mut self, overflow_hidden: bool) -> Self {
        self.overflow_hidden = overflow_hidden;
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
        for child in &self.children {
            if child.out_of_flow() {
                child_sizes.push(None);
                continue;
            }
            let size = child.wrap(content_width, child_avail_height);
            content_height = content_height + size.height;
            child_sizes.push(Some(size));
        }

        let content_height = fixed_content_height.unwrap_or(content_height);
        let border_box_height = fixed_border_box_height.unwrap_or_else(|| {
            border.top + padding.top + content_height + padding.bottom + border.bottom
        });
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
        color: Color,
    ) {
        if border.top > Pt::ZERO {
            canvas.set_fill_color(color);
            canvas.draw_rect(x, y, width, border.top);
        }
        if border.bottom > Pt::ZERO {
            canvas.set_fill_color(color);
            canvas.draw_rect(x, y + height - border.bottom, width, border.bottom);
        }
        if border.left > Pt::ZERO {
            canvas.set_fill_color(color);
            canvas.draw_rect(x, y, border.left, height);
        }
        if border.right > Pt::ZERO {
            canvas.set_fill_color(color);
            canvas.draw_rect(x + width - border.right, y, border.right, height);
        }
    }

    fn uniform_radius(radius: ResolvedBorderRadius) -> Pt {
        let mut r = radius.top_left;
        r = r.min(radius.top_right);
        r = r.min(radius.bottom_right);
        r = r.min(radius.bottom_left);
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

    fn draw_gradient_background(
        canvas: &mut Canvas,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: Pt,
        paint: &BackgroundPaint,
    ) {
        let BackgroundPaint::LinearGradient { angle_deg, stops } = paint;
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
            stops: stops.clone(),
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
            let opacity = shadow.opacity.clamp(0.0, 1.0);
            canvas.set_opacity(opacity, opacity);
            canvas.set_fill_color(shadow.color);
            if radius > Pt::ZERO {
                Self::draw_rounded_rect_fill(canvas, x, y, width, height, radius);
            } else {
                canvas.draw_rect(x, y, width, height);
            }
            canvas.set_opacity(1.0, 1.0);
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
            .resolve_width(width, self.font_size, self.root_font_size)
            .max(Pt::ZERO);

        let base_x = x + offset_x - spread;
        let base_y = y + offset_y - spread;
        let base_w = width + spread * 2;
        let base_h = height + spread * 2;
        let base_r = (radius + spread).max(Pt::ZERO);

        let steps = if blur > Pt::ZERO { 3 } else { 1 };
        for i in 0..steps {
            let frac = (i + 1) as f32 / (steps as f32);
            let extra = blur * frac;
            let opacity = (shadow.opacity * (1.0 - frac * 0.6)).clamp(0.0, 1.0);
            canvas.set_opacity(opacity, opacity);
            canvas.set_fill_color(shadow.color);
            let x0 = base_x - extra;
            let y0 = base_y - extra;
            let w0 = base_w + extra * 2;
            let h0 = base_h + extra * 2;
            let r0 = (base_r + extra).max(Pt::ZERO);
            if r0 > Pt::ZERO {
                Self::draw_rounded_rect_fill(canvas, x0, y0, w0, h0, r0);
            } else {
                canvas.draw_rect(x0, y0, w0, h0);
            }
        }
        canvas.set_opacity(1.0, 1.0);
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
            if matches!(pagination.break_before, BreakBefore::Page) && !placed.is_empty() {
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
                if matches!(pagination.break_after, BreakAfter::Page) {
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
            border_color: self.border_color,
            border_radius: self.border_radius,
            padding: Self::zero_bottom(self.padding),
            width: self.width,
            max_width: self.max_width,
            height: self.height,
            box_sizing: self.box_sizing,
            background: self.background,
            background_paint: self.background_paint.clone(),
            box_shadow: self.box_shadow.clone(),
            overflow_hidden: self.overflow_hidden,
            tag_role: self.tag_role.clone(),
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
            border_color: self.border_color,
            border_radius: self.border_radius,
            padding: Self::zero_top(self.padding),
            width: self.width,
            max_width: self.max_width,
            height: self.height,
            box_sizing: self.box_sizing,
            background: self.background,
            background_paint: self.background_paint.clone(),
            box_shadow: self.box_shadow.clone(),
            overflow_hidden: self.overflow_hidden,
            tag_role: self.tag_role.clone(),
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
        let tagged = self.tag_role.as_ref().map(|role| {
            canvas.begin_tag(role.as_ref(), None, None, None, None, true);
        });
        let cache = self.cached_layout(avail_width, avail_height);
        let margin = cache.margin;
        let border = cache.border;
        let padding = cache.padding;
        let content_width = cache.content_width;
        let border_box_width = cache.border_box_width;
        let border_box_height = cache.border_box_height;
        let child_avail_height = cache.child_avail_height;

        let border_box_x = x + margin.left;
        let border_box_y = y + margin.top;
        let radius = Self::uniform_radius(self.border_radius.resolve(
            border_box_width,
            self.font_size,
            self.root_font_size,
        ));

        if let Some(shadow) = &self.box_shadow {
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

        if let Some(paint) = &self.background_paint {
            Self::draw_gradient_background(
                canvas,
                border_box_x,
                border_box_y,
                border_box_width,
                border_box_height,
                radius,
                paint,
            );
        } else if let Some(color) = self.background {
            canvas.set_fill_color(color);
            if radius > Pt::ZERO {
                Self::draw_rounded_rect_fill(
                    canvas,
                    border_box_x,
                    border_box_y,
                    border_box_width,
                    border_box_height,
                    radius,
                );
            } else {
                canvas.draw_rect(
                    border_box_x,
                    border_box_y,
                    border_box_width,
                    border_box_height,
                );
            }
        }

        if Self::has_border(border) {
            let uniform = border.top == border.right
                && border.top == border.bottom
                && border.top == border.left;
            if radius > Pt::ZERO && uniform && border.top > Pt::ZERO {
                let inset = border.top / 2.0;
                let stroke_radius = (radius - inset).max(Pt::ZERO);
                canvas.set_stroke_color(self.border_color);
                canvas.set_line_width(border.top);
                Self::draw_rounded_rect_stroke(
                    canvas,
                    border_box_x + inset,
                    border_box_y + inset,
                    (border_box_width - border.top).max(Pt::ZERO),
                    (border_box_height - border.top).max(Pt::ZERO),
                    stroke_radius,
                );
            } else {
                Self::draw_border(
                    canvas,
                    border_box_x,
                    border_box_y,
                    border_box_width,
                    border_box_height,
                    border,
                    self.border_color,
                );
            }
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

        let mut out_of_flow_neg: Vec<(i32, usize, &Box<dyn Flowable>)> = Vec::new();
        let mut out_of_flow_zero: Vec<(usize, &Box<dyn Flowable>)> = Vec::new();
        let mut out_of_flow_pos: Vec<(i32, usize, &Box<dyn Flowable>)> = Vec::new();
        for (idx, child) in self.children.iter().enumerate() {
            if !child.out_of_flow() {
                continue;
            }
            let z = child.z_index();
            if z < 0 {
                out_of_flow_neg.push((z, idx, child));
            } else if z > 0 {
                out_of_flow_pos.push((z, idx, child));
            } else {
                out_of_flow_zero.push((idx, child));
            }
        }

        if !out_of_flow_neg.is_empty() {
            out_of_flow_neg.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            for (_, _, child) in out_of_flow_neg {
                child.draw(
                    canvas,
                    padding_box_x,
                    padding_box_y,
                    padding_box_w,
                    padding_box_h,
                );
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
            child.draw(canvas, inner_x, cursor_y, content_width, size.height);
            cursor_y = cursor_y + size.height;
        }

        if !out_of_flow_zero.is_empty() {
            out_of_flow_zero.sort_by(|a, b| a.0.cmp(&b.0));
            for (_, child) in out_of_flow_zero {
                child.draw(
                    canvas,
                    padding_box_x,
                    padding_box_y,
                    padding_box_w,
                    padding_box_h,
                );
            }
        }

        if !out_of_flow_pos.is_empty() {
            out_of_flow_pos.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            for (_, _, child) in out_of_flow_pos {
                child.draw(
                    canvas,
                    padding_box_x,
                    padding_box_y,
                    padding_box_w,
                    padding_box_h,
                );
            }
        }

        if self.overflow_hidden {
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

#[derive(Clone)]
pub struct AbsolutePositionedFlowable {
    child: Box<dyn Flowable>,
    left: LengthSpec,
    top: LengthSpec,
    right: LengthSpec,
    bottom: LengthSpec,
    z_index: i32,
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
        for (k, v) in self.metadata.iter() {
            canvas.meta(k.clone(), v.clone());
        }
        self.child.draw(canvas, x, y, avail_width, avail_height);
    }

    fn intrinsic_width(&self) -> Option<Pt> {
        self.child.intrinsic_width()
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
}

impl AbsolutePositionedFlowable {
    pub fn new(
        child: Box<dyn Flowable>,
        left: LengthSpec,
        top: LengthSpec,
        right: LengthSpec,
        bottom: LengthSpec,
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
            z_index,
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

    fn draw(&self, canvas: &mut Canvas, x: Pt, y: Pt, avail_width: Pt, avail_height: Pt) {
        let has_left = !matches!(self.left, LengthSpec::Auto);
        let has_right = !matches!(self.right, LengthSpec::Auto);
        let has_top = !matches!(self.top, LengthSpec::Auto);
        let has_bottom = !matches!(self.bottom, LengthSpec::Auto);

        let left = if has_left {
            self.left
                .resolve_width(avail_width, self.font_size, self.root_font_size)
        } else {
            Pt::ZERO
        };
        let right = if has_right {
            self.right
                .resolve_width(avail_width, self.font_size, self.root_font_size)
        } else {
            Pt::ZERO
        };
        let top = if has_top {
            self.top
                .resolve_height(avail_height, self.font_size, self.root_font_size)
        } else {
            Pt::ZERO
        };
        let bottom = if has_bottom {
            self.bottom
                .resolve_height(avail_height, self.font_size, self.root_font_size)
        } else {
            Pt::ZERO
        };
        let left = left;
        let right = right;
        let top = top;
        let bottom = bottom;

        // CSS-ish positioning behavior:
        // - If both sides are set, stretch to fill.
        // - If only one side is set, anchor there and use the child's intrinsic size.
        // - If neither is set, default to 0.
        let stretch_w = has_left && has_right;
        let stretch_h = has_top && has_bottom;

        let avail_w_for_child = if stretch_w {
            (avail_width - left - right).max(Pt::ZERO)
        } else if has_left {
            (avail_width - left).max(Pt::ZERO)
        } else if has_right {
            (avail_width - right).max(Pt::ZERO)
        } else {
            avail_width
        };
        let avail_h_for_child = if stretch_h {
            (avail_height - top - bottom).max(Pt::ZERO)
        } else if has_top {
            (avail_height - top).max(Pt::ZERO)
        } else if has_bottom {
            (avail_height - bottom).max(Pt::ZERO)
        } else {
            avail_height
        };

        // Shrink-to-fit for absolutely positioned elements when width is auto.
        // Prefer intrinsic width when available, clamped to the available space.
        let target_w = if stretch_w {
            avail_w_for_child
        } else if let Some(intrinsic) = self.child.intrinsic_width() {
            intrinsic.min(avail_w_for_child).max(Pt::ZERO)
        } else {
            avail_w_for_child
        };

        let size = self.child.wrap(target_w, avail_h_for_child);

        let child_w = if stretch_w {
            avail_w_for_child
        } else {
            size.width.min(target_w).max(Pt::ZERO)
        };
        let child_h = if stretch_h {
            avail_h_for_child
        } else {
            size.height.min(avail_h_for_child).max(Pt::ZERO)
        };

        let child_x = if has_left {
            x + left
        } else if has_right {
            x + (avail_width - right - child_w)
        } else {
            x
        };
        let child_y = if has_top {
            y + top
        } else if has_bottom {
            y + (avail_height - bottom - child_h)
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
}
