use crate::assets::{
    AssetBundle, load_svg_xml_from_image_source, raster_image_intrinsic_dimensions,
    renderable_image_source,
};
use crate::flowable::{
    AbsolutePositionedFlowable, AlignContent, AlignItems, BackgroundPaint, BackgroundPaintFlowable,
    BorderRadiiSpec, BorderSpacingSpec, BorderSpec, CalcLength, CjkDecimalMarkerFlowable,
    ClearFlowable, ClipPathCircleSpec, ClipPathEllipseSpec, ClipPathPathCommand, ClipPathPathSpec,
    ClipPathRectSpec, ClipPathShapeRadius, ClipPathShapeSpec, ClipPathXywhSpec,
    CollapsibleSpaceFlowable, ContainerFlowable, CssLineBoxFlowable, CssPixelHeightFlowable,
    EdgeSizes, ExpandedWidthFlowable, FilterDropShadowSpec, FlexDirection, FlexFlowable,
    FloatClear, FloatFlowable, FloatSide, FootnoteCallFlowable, GridTrackBreadth, GridTrackSize,
    ImageFlowable, InlineBackgroundFlowable, InlineBlockLayoutFlowable, JustifyContent,
    LeaderFlowable, LengthSpec, ListBulletFlowable, ListBulletKind, ListItemFlowable, MaskMode,
    MetaFlowable, MultiColumnFlowable, OutlineLineStyle, OverlayFlowable, PageFootnoteAreaStyle,
    PageFootnoteEntry, PaintFilterOperation, PaintFilterSpec, Paragraph,
    RelativePositionedFlowable, RunningElementFlowable, ScreenReaderTextFlowable, Spacer,
    SvgComponentTransferFunction, SvgFilterInput, SvgFilterNode, SvgFilterPrimitive,
    SvgFilterProgram, SvgFilterRegion, SvgFlowable, SvgMorphologyOperator, TableCell,
    TableColumnBorder, TableColumnGroupBorder, TableColumnWidthHint, TableFlowable,
    TableLayoutMode, TextAlign, TextStyle, VerticalAlign,
    css_direct_text_prefers_nearest_baseline_snap, css_print_line_prefers_nearest_baseline_snap,
};
use crate::font::FontRegistry;
use crate::glyph_report::GlyphCoverageReport;
use crate::html_dom::{NodeData, NodeRef, parse_html};
use crate::style::{
    AlignContentMode, AlignItemsMode, AlignSelfMode, ClearMode, ColumnSpanMode, ComputedStyle,
    DirectionMode, DisplayMode, ElementInfo, FlexDirectionMode, FlexWrapMode, FloatMode,
    GeneratedContentPart, GeneratedCounterContent, GeneratedCounterStyle, GeneratedCountersContent,
    GridAutoFlowMode, GridAutoRepeatMode, GridLineSpec, JustifyContentMode, ListStylePositionMode,
    ListStyleTypeMode, NamedStringSource, OverflowMode, PositionMode, StyleResolver,
    TextAlignLastMode, TextAlignMode, TextWrapMode, VerticalAlignMode, VisibilityMode,
    WhiteSpaceMode, WritingModeMode,
};
use crate::types::{Pt, Size};
use crate::{BreakAfter, BreakBefore, BreakInside, Color, Flowable};
use std::collections::HashMap;
use std::sync::Arc;

const FORCED_LINE_BREAK: char = '\u{2028}';

pub(crate) fn document_canvas_background(
    html: &str,
    resolver: &StyleResolver,
) -> Option<(Color, f32)> {
    let document = parse_html(html);
    document_canvas_background_for_document(&document, resolver)
}

pub(crate) fn document_canvas_background_for_document(
    document: &NodeRef,
    resolver: &StyleResolver,
) -> Option<(Color, f32)> {
    let base_style = resolver.default_style();
    let html_el = document.select_first("html").ok()?;
    let html_node = html_el.as_node();
    let html_element = html_node.as_element()?;
    let mut html_info = element_info(html_node, resolver.has_sibling_selectors());
    let html_inline_style = html_element
        .attributes
        .borrow()
        .get("style")
        .map(|value| value.to_string());
    let root_style =
        resolver.compute_style(&html_info, &base_style, html_inline_style.as_deref(), &[]);
    if let Some(color) = root_style
        .background_source_color
        .or(root_style.background_color)
    {
        if root_style.background_alpha > 0.0 {
            return Some((color, root_style.background_alpha));
        }
    }

    html_info.apply_computed_container_style(&root_style);
    let body_el = document.select_first("body").ok()?;
    let body_node = body_el.as_node();
    let body_element = body_node.as_element()?;
    let body_info = element_info(body_node, resolver.has_sibling_selectors());
    let body_inline_style = body_element
        .attributes
        .borrow()
        .get("style")
        .map(|value| value.to_string());
    let body_style = resolver.compute_style(
        &body_info,
        &root_style,
        body_inline_style.as_deref(),
        &[html_info],
    );
    body_style
        .background_source_color
        .or(body_style.background_color)
        .filter(|_| body_style.background_alpha > 0.0)
        .map(|color| (color, body_style.background_alpha))
}

#[derive(Debug, Clone)]
pub struct HtmlAssetWarning {
    pub kind: String,
    pub message: String,
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct CounterState {
    values: HashMap<String, Vec<i32>>,
    quote_depth: usize,
    target_texts: Arc<HashMap<String, String>>,
    target_pages: Arc<HashMap<String, usize>>,
}

impl CounterState {
    fn with_target_context(
        target_texts: HashMap<String, String>,
        target_pages: Arc<HashMap<String, usize>>,
    ) -> Self {
        Self {
            target_texts: Arc::new(target_texts),
            target_pages,
            ..Self::default()
        }
    }

    fn target_text(&self, reference: &str) -> Option<&str> {
        let (_, fragment) = reference.rsplit_once('#')?;
        if fragment.is_empty() {
            return None;
        }
        self.target_texts.get(fragment).map(String::as_str)
    }

    fn target_page(&self, reference: &str) -> Option<usize> {
        let (_, fragment) = reference.rsplit_once('#')?;
        if fragment.is_empty() {
            return None;
        }
        self.target_pages.get(fragment).copied()
    }

    fn reset(&mut self, name: &str, value: i32) {
        self.values.entry(name.to_string()).or_default().push(value);
    }

    fn set(&mut self, name: &str, value: i32) {
        let values = self.values.entry(name.to_string()).or_default();
        if let Some(current) = values.last_mut() {
            *current = value;
        } else {
            values.push(value);
        }
    }

    fn increment(&mut self, name: &str, value: i32) {
        let values = self.values.entry(name.to_string()).or_default();
        if values.is_empty() {
            values.push(0);
        }
        let entry = values.last_mut().expect("counter stack has a value");
        *entry += value;
    }

    fn get(&self, name: &str) -> i32 {
        self.values
            .get(name)
            .and_then(|values| values.last().copied())
            .unwrap_or(0)
    }

    fn get_all(&self, name: &str) -> Vec<i32> {
        self.values
            .get(name)
            .filter(|values| !values.is_empty())
            .cloned()
            .unwrap_or_else(|| vec![0])
    }

    fn pop_reset_scope(&mut self, name: &str) {
        if let Some(values) = self.values.get_mut(name) {
            values.pop();
            if values.is_empty() {
                self.values.remove(name);
            }
        }
    }

    fn pop_reset_scopes(&mut self, names: &[String]) {
        for name in names.iter().rev() {
            self.pop_reset_scope(name);
        }
    }

    fn open_quote(&mut self, quotes: &[(String, String)], paint: bool) -> Option<String> {
        let text = paint
            .then(|| quote_pair_at_depth(quotes, self.quote_depth).map(|pair| pair.0.clone()))
            .flatten();
        self.quote_depth = self.quote_depth.saturating_add(1);
        text
    }

    fn close_quote(&mut self, quotes: &[(String, String)], paint: bool) -> Option<String> {
        if self.quote_depth == 0 {
            return None;
        }
        self.quote_depth -= 1;
        paint
            .then(|| quote_pair_at_depth(quotes, self.quote_depth).map(|pair| pair.1.clone()))
            .flatten()
    }
}

fn document_target_texts(document: &NodeRef) -> HashMap<String, String> {
    let mut targets = HashMap::new();
    for node in document.descendants() {
        let Some(element) = node.as_element() else {
            continue;
        };
        let id = element
            .attributes
            .borrow()
            .get("id")
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string);
        let Some(id) = id else {
            continue;
        };
        let text = node
            .text_contents()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        targets.entry(id).or_insert(text);
    }
    targets
}

fn quote_pair_at_depth(quotes: &[(String, String)], depth: usize) -> Option<&(String, String)> {
    quotes.get(depth).or_else(|| quotes.last())
}

fn apply_counter_mutations(
    counter_reset: &[crate::style::CounterMutation],
    style: &ComputedStyle,
    counters: &mut CounterState,
) -> Vec<String> {
    let mut reset_scopes = Vec::new();
    for mutation in counter_reset {
        counters.reset(&mutation.name, mutation.value);
        reset_scopes.push(mutation.name.clone());
    }
    for mutation in &style.counter_increment {
        counters.increment(&mutation.name, mutation.value);
    }
    for mutation in &style.counter_set {
        counters.set(&mutation.name, mutation.value);
    }
    reset_scopes
}

fn apply_style_counters(style: &ComputedStyle, counters: &mut CounterState) -> Vec<String> {
    apply_counter_mutations(&style.counter_reset, style, counters)
}

fn apply_style_counters_for_node(
    node: &NodeRef,
    resolver: &StyleResolver,
    style: &ComputedStyle,
    info: &ElementInfo,
    ancestors: &[ElementInfo],
    counters: &mut CounterState,
) -> Vec<String> {
    if style
        .counter_reset
        .iter()
        .all(|mutation| !mutation.auto_reversed_initial)
    {
        return apply_style_counters(style, counters);
    }
    let mut resolved_reset = style.counter_reset.clone();
    for mutation in &mut resolved_reset {
        if mutation.auto_reversed_initial {
            mutation.value = auto_reversed_counter_initial(
                node,
                resolver,
                style,
                info,
                ancestors,
                &mutation.name,
            );
        }
    }
    apply_counter_mutations(&resolved_reset, style, counters)
}

fn style_can_mutate_counters(style: &ComputedStyle) -> bool {
    !matches!(style.display, DisplayMode::None | DisplayMode::Contents)
}

fn style_is_css_list_item(style: &ComputedStyle) -> bool {
    matches!(
        style.display,
        DisplayMode::ListItem | DisplayMode::InlineListItem
    )
}

fn style_is_inline_list_item(style: &ComputedStyle) -> bool {
    matches!(style.display, DisplayMode::InlineListItem)
}

fn apply_implicit_list_item_counter(style: &ComputedStyle, counters: &mut CounterState) {
    if !style
        .counter_increment
        .iter()
        .any(|mutation| mutation.name == "list-item")
    {
        counters.increment("list-item", 1);
    }
}

fn current_list_item_counter_index(counters: &CounterState) -> usize {
    usize::try_from(counters.get("list-item"))
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or(1)
}

fn auto_reversed_counter_initial(
    node: &NodeRef,
    resolver: &StyleResolver,
    style: &ComputedStyle,
    info: &ElementInfo,
    ancestors: &[ElementInfo],
    name: &str,
) -> i32 {
    let scan =
        scan_auto_reversed_counter_events(node, resolver, style, info, ancestors, name, true);
    scan.total
        .saturating_add(scan.last_nonzero_increment_negated)
}

#[derive(Debug, Clone, Copy, Default)]
struct AutoReversedCounterScan {
    total: i32,
    last_nonzero_increment_negated: i32,
    stopped: bool,
}

impl AutoReversedCounterScan {
    fn absorb(&mut self, other: AutoReversedCounterScan) {
        self.total = self.total.saturating_add(other.total);
        if other.last_nonzero_increment_negated != 0 {
            self.last_nonzero_increment_negated = other.last_nonzero_increment_negated;
        }
        self.stopped |= other.stopped;
    }
}

fn scan_auto_reversed_counter_events(
    node: &NodeRef,
    resolver: &StyleResolver,
    style: &ComputedStyle,
    info: &ElementInfo,
    ancestors: &[ElementInfo],
    name: &str,
    is_root: bool,
) -> AutoReversedCounterScan {
    let can_mutate_counters = style_can_mutate_counters(style);
    if can_mutate_counters
        && !is_root
        && style
            .counter_reset
            .iter()
            .any(|mutation| mutation.name == name)
    {
        return AutoReversedCounterScan::default();
    }
    let mut scan = AutoReversedCounterScan::default();
    if can_mutate_counters {
        scan_auto_reversed_counter_event(&mut scan, style, name);
        if scan.stopped {
            return scan;
        }
    }

    if let Some(pseudo_style) =
        resolver.compute_pseudo_style(info, style, ancestors, crate::style::PseudoTarget::Before)
    {
        if style_can_mutate_counters(&pseudo_style) {
            scan_auto_reversed_counter_event(&mut scan, &pseudo_style, name);
            if scan.stopped {
                return scan;
            }
        }
    }

    let mut child_ancestors = ancestors.to_vec();
    child_ancestors.push(info.clone());
    for child in node.children() {
        let Some(element) = child.as_element() else {
            continue;
        };
        let mut child_info = element_info(&child, resolver.has_sibling_selectors());
        let inline_style = element
            .attributes
            .borrow()
            .get("style")
            .map(|s| s.to_string());
        let child_style = resolver.compute_style(
            &child_info,
            style,
            inline_style.as_deref(),
            &child_ancestors,
        );
        if matches!(child_style.display, DisplayMode::None) {
            continue;
        }
        child_info.apply_computed_container_style(&child_style);
        let child_scan = scan_auto_reversed_counter_events(
            &child,
            resolver,
            &child_style,
            &child_info,
            &child_ancestors,
            name,
            false,
        );
        scan.absorb(child_scan);
        if scan.stopped {
            return scan;
        }
    }

    if let Some(pseudo_style) =
        resolver.compute_pseudo_style(info, style, ancestors, crate::style::PseudoTarget::After)
    {
        if style_can_mutate_counters(&pseudo_style) {
            scan_auto_reversed_counter_event(&mut scan, &pseudo_style, name);
        }
    }
    scan
}

fn scan_auto_reversed_counter_event(
    scan: &mut AutoReversedCounterScan,
    style: &ComputedStyle,
    name: &str,
) {
    if scan.stopped {
        return;
    }
    let increment_negated = counter_increment_negated(style, name);
    if increment_negated != 0 {
        scan.last_nonzero_increment_negated = increment_negated;
    }
    if let Some(value) = counter_set_value(style, name) {
        scan.total = scan.total.saturating_add(value);
        scan.stopped = true;
    } else {
        scan.total = scan.total.saturating_add(increment_negated);
    }
}

fn counter_increment_negated(style: &ComputedStyle, name: &str) -> i32 {
    style
        .counter_increment
        .iter()
        .filter(|mutation| mutation.name == name)
        .map(|mutation| mutation.value.saturating_neg())
        .fold(0, i32::saturating_add)
}

fn counter_set_value(style: &ComputedStyle, name: &str) -> Option<i32> {
    style
        .counter_set
        .iter()
        .rev()
        .find(|mutation| mutation.name == name)
        .map(|mutation| mutation.value)
}

fn vertical_align_from_style(style: &ComputedStyle) -> VerticalAlign {
    vertical_align_from_style_with_font_size(style, style.font_size)
}

fn anonymous_text_vertical_align(style: &ComputedStyle) -> VerticalAlign {
    // `vertical-align` positions an inline-level principal box in its parent
    // line, but has a separate meaning on table cells. Anonymous text inside
    // a block/table-cell formatting context therefore starts on the baseline;
    // only a transparent inline element transfers its own alignment to the
    // flattened text run that represents that principal box.
    if matches!(style.display, DisplayMode::Inline) {
        vertical_align_from_style(style)
    } else {
        VerticalAlign::Baseline
    }
}

fn vertical_align_from_style_with_font_size(
    style: &ComputedStyle,
    containing_font_size: Pt,
) -> VerticalAlign {
    match style.vertical_align {
        VerticalAlignMode::Baseline => VerticalAlign::Baseline,
        VerticalAlignMode::Middle => VerticalAlign::Middle,
        VerticalAlignMode::Bottom => VerticalAlign::Bottom,
        VerticalAlignMode::TextBottom => VerticalAlign::TextBottom,
        VerticalAlignMode::Top => VerticalAlign::Top,
        VerticalAlignMode::TextTop => VerticalAlign::TextTop,
        VerticalAlignMode::Sub => {
            VerticalAlign::BaselineShift(css_script_baseline_shift(containing_font_size, 5))
        }
        VerticalAlignMode::Super => {
            VerticalAlign::BaselineShift(-css_script_baseline_shift(containing_font_size, 3))
        }
        VerticalAlignMode::Length(length) => VerticalAlign::BaselineShift(-length.resolve_height(
            style.to_text_style().line_height,
            style.font_size,
            style.root_font_size,
        )),
    }
}

fn css_script_baseline_shift(font_size: Pt, divisor: i32) -> Pt {
    // Blink's legacy inline layout (used by the pinned oracle) shifts `sub`
    // and `super` by parent-font-size/divisor + 1 CSS px, quantized down to a
    // 1/64 CSS-pixel LayoutUnit. Keep that quantization in fixed-point layout.
    let raw = font_size.mul_ratio(1, divisor) + Pt::from_milli_i64(750);
    let units = (raw.to_milli_i64() as i128 * 256).div_euclid(3_000);
    let milli = (units * 3_000 + 128).div_euclid(256);
    Pt::from_milli_i64(milli.clamp(i64::MIN as i128, i64::MAX as i128) as i64)
}

fn override_inline_vertical_align(
    items: Vec<LayoutItem>,
    vertical_align: VerticalAlign,
) -> Vec<LayoutItem> {
    items
        .into_iter()
        .map(|item| match item {
            LayoutItem::Inline {
                flowable,
                flex_grow,
                flex_shrink,
                width_spec,
                order,
                ..
            } => LayoutItem::Inline {
                flowable,
                valign: vertical_align,
                flex_grow,
                flex_shrink,
                width_spec,
                order,
            },
            LayoutItem::Block {
                flowable,
                flex_grow,
                flex_shrink,
                width_spec,
                order,
            } => LayoutItem::Inline {
                flowable,
                valign: vertical_align,
                flex_grow,
                flex_shrink,
                width_spec,
                order,
            },
        })
        .collect()
}

pub fn scan_html_asset_warnings(html: &str) -> Vec<HtmlAssetWarning> {
    let document = parse_html(html);
    let mut warnings: Vec<HtmlAssetWarning> = Vec::new();

    let mut stylesheet_links: Vec<String> = Vec::new();
    let mut font_links: Vec<String> = Vec::new();
    if let Ok(links) = document.select("link[rel][href]") {
        for link in links {
            let attrs = link.attributes.borrow();
            let rel = attrs.get("rel").unwrap_or("").to_ascii_lowercase();
            let href = attrs.get("href").unwrap_or("").to_string();
            if rel.contains("stylesheet") {
                stylesheet_links.push(href);
                continue;
            }
            if rel.contains("preload") || rel.contains("prefetch") {
                let as_attr = attrs.get("as").unwrap_or("").to_ascii_lowercase();
                let ty_attr = attrs.get("type").unwrap_or("").to_ascii_lowercase();
                if as_attr == "font" || ty_attr.starts_with("font/") {
                    font_links.push(href);
                }
            }
        }
    }

    if !stylesheet_links.is_empty() {
        warnings.push(HtmlAssetWarning {
            kind: "stylesheet".to_string(),
            message: "HTML <link rel=\"stylesheet\"> detected. FullBleed ignores external CSS in HTML; use AssetBundle and engine.register_bundle(bundle).".to_string(),
            details: stylesheet_links,
        });
    }

    if !font_links.is_empty() {
        warnings.push(HtmlAssetWarning {
            kind: "font-preload".to_string(),
            message: "HTML font preload detected. FullBleed does not resolve font preloads in HTML; register fonts via AssetBundle.".to_string(),
            details: font_links,
        });
    }

    if let Ok(styles) = document.select("style") {
        let mut count = 0usize;
        let mut has_import = false;
        for style in styles {
            let node = style.as_node();
            let nested_in_svg = node.ancestors().any(|ancestor| {
                if let NodeData::Element(el) = ancestor.data() {
                    el.name.local.as_ref().eq_ignore_ascii_case("svg")
                } else {
                    false
                }
            });
            if nested_in_svg {
                continue;
            }
            count += 1;
            let text = node.text_contents();
            if text.contains("@import") {
                has_import = true;
            }
        }
        if count > 0 {
            let mut message = format!(
                "HTML contains {count} <style> block(s). FullBleed ignores embedded CSS in HTML; use AssetBundle instead."
            );
            if has_import {
                message.push_str(" Detected @import which will be ignored.");
            }
            warnings.push(HtmlAssetWarning {
                kind: "style-tag".to_string(),
                message,
                details: Vec::new(),
            });
        }
    }

    if let Ok(scripts) = document.select("script[src]") {
        let mut script_srcs = Vec::new();
        for script in scripts {
            let attrs = script.attributes.borrow();
            if let Some(src) = attrs.get("src") {
                script_srcs.push(src.to_string());
            }
        }
        if !script_srcs.is_empty() {
            warnings.push(HtmlAssetWarning {
                kind: "script".to_string(),
                message: "HTML <script src=...> detected. FullBleed does not execute JS; remove scripts or precompute markup.".to_string(),
                details: script_srcs,
            });
        }
    }

    warnings
}

#[derive(Clone)]
enum LayoutItem {
    Block {
        flowable: Box<dyn Flowable>,
        flex_grow: f32,
        flex_shrink: f32,
        width_spec: Option<LengthSpec>,
        order: i32,
    },
    Inline {
        flowable: Box<dyn Flowable>,
        valign: VerticalAlign,
        flex_grow: f32,
        flex_shrink: f32,
        width_spec: Option<LengthSpec>,
        order: i32,
    },
}

#[derive(Clone)]
struct ForcedLineBreakFlowable {
    line_height: Pt,
}

impl ForcedLineBreakFlowable {
    fn new(line_height: Pt) -> Self {
        Self { line_height }
    }
}

impl Flowable for ForcedLineBreakFlowable {
    fn wrap(&self, _avail_width: Pt, _avail_height: Pt) -> Size {
        Size {
            width: Pt::ZERO,
            height: self.line_height,
        }
    }

    fn split(
        &self,
        _avail_width: Pt,
        _avail_height: Pt,
    ) -> Option<(Box<dyn Flowable>, Box<dyn Flowable>)> {
        None
    }

    fn draw(
        &self,
        _canvas: &mut crate::Canvas,
        _x: Pt,
        _y: Pt,
        _avail_width: Pt,
        _avail_height: Pt,
    ) {
    }

    fn forced_line_break_height(&self) -> Option<Pt> {
        Some(self.line_height)
    }
}

impl LayoutItem {
    fn flex_grow(&self) -> f32 {
        match self {
            LayoutItem::Block { flex_grow, .. } => *flex_grow,
            LayoutItem::Inline { flex_grow, .. } => *flex_grow,
        }
    }

    fn flex_shrink(&self) -> f32 {
        match self {
            LayoutItem::Block { flex_shrink, .. } => *flex_shrink,
            LayoutItem::Inline { flex_shrink, .. } => *flex_shrink,
        }
    }

    fn width_spec(&self) -> Option<LengthSpec> {
        match self {
            LayoutItem::Block { width_spec, .. } => *width_spec,
            LayoutItem::Inline { width_spec, .. } => *width_spec,
        }
    }

    fn order(&self) -> i32 {
        match self {
            LayoutItem::Block { order, .. } => *order,
            LayoutItem::Inline { order, .. } => *order,
        }
    }

    fn with_flex_grow(self, grow: f32) -> Self {
        match self {
            LayoutItem::Block {
                flowable,
                flex_shrink,
                width_spec,
                order,
                ..
            } => LayoutItem::Block {
                flowable,
                flex_grow: grow.max(0.0),
                flex_shrink,
                width_spec,
                order,
            },
            LayoutItem::Inline {
                flowable,
                valign,
                flex_shrink,
                width_spec,
                order,
                ..
            } => LayoutItem::Inline {
                flowable,
                valign,
                flex_grow: grow.max(0.0),
                flex_shrink,
                width_spec,
                order,
            },
        }
    }

    fn with_flex_shrink(self, shrink: f32) -> Self {
        let shrink = shrink.max(0.0);
        match self {
            LayoutItem::Block {
                flowable,
                flex_grow,
                width_spec,
                order,
                ..
            } => LayoutItem::Block {
                flowable,
                flex_grow,
                flex_shrink: shrink,
                width_spec,
                order,
            },
            LayoutItem::Inline {
                flowable,
                valign,
                flex_grow,
                width_spec,
                order,
                ..
            } => LayoutItem::Inline {
                flowable,
                valign,
                flex_grow,
                flex_shrink: shrink,
                width_spec,
                order,
            },
        }
    }

    fn with_order(self, order: i32) -> Self {
        match self {
            LayoutItem::Block {
                flowable,
                flex_grow,
                flex_shrink,
                width_spec,
                ..
            } => LayoutItem::Block {
                flowable,
                flex_grow,
                flex_shrink,
                width_spec,
                order,
            },
            LayoutItem::Inline {
                flowable,
                valign,
                flex_grow,
                flex_shrink,
                width_spec,
                ..
            } => LayoutItem::Inline {
                flowable,
                valign,
                flex_grow,
                flex_shrink,
                width_spec,
                order,
            },
        }
    }
}

fn flex_item_basis(style: &ComputedStyle) -> Option<LengthSpec> {
    if !matches!(
        style.flex_basis,
        LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
    ) {
        Some(style.flex_basis)
    } else {
        None
    }
}

#[inline(never)]
fn compute_boxed_style(
    resolver: &StyleResolver,
    info: &ElementInfo,
    parent_style: &ComputedStyle,
    inline_style: Option<&str>,
    ancestors: &[ElementInfo],
) -> Box<ComputedStyle> {
    Box::new(resolver.compute_style(info, parent_style, inline_style, ancestors))
}

pub fn html_to_story_with_resolver_and_fonts_and_report(
    html: &str,
    resolver: &StyleResolver,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<Arc<AssetBundle>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
) -> Vec<Box<dyn Flowable>> {
    html_to_story_with_resolver_and_fonts_and_report_and_target_pages(
        html,
        resolver,
        font_registry,
        asset_bundle,
        report,
        svg_form,
        svg_raster_fallback,
        perf,
        doc_id,
        None,
    )
}

pub(crate) fn html_to_story_with_resolver_and_fonts_and_report_and_target_pages(
    html: &str,
    resolver: &StyleResolver,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<Arc<AssetBundle>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
    target_pages: Option<Arc<HashMap<String, usize>>>,
) -> Vec<Box<dyn Flowable>> {
    let t_parse = std::time::Instant::now();
    let document = parse_html(html);
    if let Some(perf_logger) = perf {
        let ms = t_parse.elapsed().as_secs_f64() * 1000.0;
        perf_logger.log_span_ms("story.parse_html", doc_id, ms);
        let mut nodes: u64 = 0;
        let mut elements: u64 = 0;
        for node in document.descendants() {
            nodes += 1;
            if node.as_element().is_some() {
                elements += 1;
            }
        }
        perf_logger.log_counts(
            "story.nodes",
            doc_id,
            &[("nodes", nodes), ("elements", elements)],
        );
    }
    html_document_to_story_with_resolver_and_fonts_and_report_and_target_pages(
        &document,
        resolver,
        font_registry,
        asset_bundle,
        report,
        svg_form,
        svg_raster_fallback,
        perf,
        doc_id,
        target_pages,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn html_document_to_story_with_resolver_and_fonts_and_report_and_target_pages(
    document: &NodeRef,
    resolver: &StyleResolver,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<Arc<AssetBundle>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
    target_pages: Option<Arc<HashMap<String, usize>>>,
) -> Vec<Box<dyn Flowable>> {
    // `ComputedStyle` intentionally carries the complete typed CSS state and is
    // consequently a large value. Keep both document-level styles off the
    // comparatively small Windows test/worker stacks before descending into
    // the recursive DOM compiler.
    let base_style = Box::new(resolver.default_style());
    let mut ancestors: Vec<ElementInfo> = Vec::new();
    let mut report = report;
    let mut counters = CounterState::with_target_context(
        document_target_texts(document),
        target_pages.unwrap_or_else(|| Arc::new(HashMap::new())),
    );

    let mut root_style = base_style.clone();
    if let Ok(html_el) = document.select_first("html") {
        let t_root = std::time::Instant::now();
        let html_node = html_el.as_node();
        let html_element = html_node.as_element().expect("html element");
        let mut html_info = element_info(html_node, resolver.has_sibling_selectors());
        let inline_style = html_element
            .attributes
            .borrow()
            .get("style")
            .map(|s| s.to_string());
        root_style = compute_boxed_style(
            resolver,
            &html_info,
            &base_style,
            inline_style.as_deref(),
            &ancestors,
        );
        html_info.apply_computed_container_style(&root_style);
        apply_style_counters_for_node(
            html_node,
            resolver,
            &root_style,
            &html_info,
            &ancestors,
            &mut counters,
        );
        ancestors.push(html_info);
        if let Some(perf_logger) = perf {
            let ms = t_root.elapsed().as_secs_f64() * 1000.0;
            perf_logger.log_span_ms("story.style.root", doc_id, ms);
        }
    }

    let items = if let Ok(body) = document.select_first("body") {
        let t_collect = std::time::Instant::now();
        // The body establishes a real CSS box. Routing it through the normal
        // element path preserves its margin, border, padding, backgrounds,
        // formatting context, pseudo-elements, and pagination semantics.
        // Flattening directly to its children discarded all of those and also
        // prevented inline flex/grid children from sharing one line box.
        let items = node_to_flowables(
            body.as_node(),
            resolver,
            &root_style,
            &mut ancestors,
            &mut counters,
            font_registry.clone(),
            asset_bundle.clone(),
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
            perf,
            doc_id,
        );
        if let Some(perf_logger) = perf {
            let ms = t_collect.elapsed().as_secs_f64() * 1000.0;
            perf_logger.log_span_ms("story.collect", doc_id, ms);
            perf_logger.log_counts(
                "story.items",
                doc_id,
                &[("layout_items", items.len() as u64)],
            );
        }
        items
    } else {
        let t_collect = std::time::Instant::now();
        let items = collect_children(
            document,
            resolver,
            &root_style,
            &mut ancestors,
            &mut counters,
            font_registry.clone(),
            asset_bundle.clone(),
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
            perf,
            doc_id,
        );
        if let Some(perf_logger) = perf {
            let ms = t_collect.elapsed().as_secs_f64() * 1000.0;
            perf_logger.log_span_ms("story.collect", doc_id, ms);
            perf_logger.log_counts(
                "story.items",
                doc_id,
                &[("layout_items", items.len() as u64)],
            );
        }
        items
    };
    let t_flowables = std::time::Instant::now();
    let flowables = layout_children_to_flowables(items, None);
    if let Some(perf_logger) = perf {
        let ms = t_flowables.elapsed().as_secs_f64() * 1000.0;
        perf_logger.log_span_ms("story.flowables", doc_id, ms);
        perf_logger.log_counts(
            "story.flowables",
            doc_id,
            &[("flowables", flowables.len() as u64)],
        );
    }
    flowables
}

pub fn template_uses_attribute_placeholders(html: &str) -> bool {
    let document = parse_html(html);
    for node in document.descendants() {
        let Some(element) = node.as_element() else {
            continue;
        };
        let attrs = element.attributes.borrow();
        for (_k, v) in attrs.map.iter() {
            if contains_placeholder(&v.value) {
                return true;
            }
        }
    }
    false
}

fn contains_placeholder(value: &str) -> bool {
    value.contains("{page}")
        || value.contains("{pages}")
        || value.contains("{sum:")
        || value.contains("{total:")
}

fn parse_data_fb(raw: &str) -> Vec<(String, String)> {
    // data-fb="key=value; other.key=other_value"
    raw.split(';')
        .filter_map(|pair| {
            let pair = pair.trim();
            if pair.is_empty() {
                return None;
            }
            let (k, v) = pair.split_once('=')?;
            let k = k.trim();
            let v = v.trim();
            if k.is_empty() {
                return None;
            }
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

fn selector_fragment(info: &ElementInfo) -> String {
    let mut out = info.tag.clone();
    if let Some(id) = &info.id {
        out.push('#');
        out.push_str(id);
    }
    for class_name in info.classes.iter().take(3) {
        out.push('.');
        out.push_str(class_name);
    }
    out.push_str(&format!(":nth-of-type({})", info.child_index));
    out
}

fn dom_path_for_node(ancestors: &[ElementInfo], info: &ElementInfo) -> String {
    let mut parts: Vec<String> = ancestors
        .iter()
        .filter(|ancestor| !ancestor.tag.is_empty())
        .map(selector_fragment)
        .collect();
    parts.push(selector_fragment(info));
    parts.join(" > ")
}

fn authored_owner_metadata(
    info: &ElementInfo,
    ancestors: &[ElementInfo],
    explicit_meta: &[(String, String)],
    style: &ComputedStyle,
) -> Vec<(String, String)> {
    let should_attach = !explicit_meta.is_empty()
        || info.id.is_some()
        || !info.classes.is_empty()
        || info.attrs.contains_key("data-fb-id")
        || info.attrs.contains_key("data-fb-role")
        || info.attrs.contains_key("data-fb-component")
        || !style.transform.is_empty()
        || matches!(
            info.tag.as_str(),
            "main"
                | "section"
                | "article"
                | "nav"
                | "aside"
                | "header"
                | "footer"
                | "form"
                | "table"
                | "thead"
                | "tbody"
                | "tfoot"
                | "tr"
                | "td"
                | "th"
                | "dl"
                | "figure"
        )
        || matches!(style.position, PositionMode::Absolute | PositionMode::Fixed);
    if !should_attach {
        return explicit_meta.to_vec();
    }

    let mut out = explicit_meta.to_vec();
    let upsert = |rows: &mut Vec<(String, String)>, key: &str, value: Option<String>| {
        let Some(value) = value.map(|value| value.trim().to_string()) else {
            return;
        };
        if value.is_empty() {
            return;
        }
        if let Some(existing) = rows.iter_mut().find(|(k, _)| k == key) {
            existing.1 = value;
        } else {
            rows.push((key.to_string(), value));
        }
    };

    upsert(&mut out, "fb.owner.tag", Some(info.tag.clone()));
    upsert(&mut out, "fb.owner.selector", Some(selector_fragment(info)));
    upsert(
        &mut out,
        "fb.owner.dom_path",
        Some(dom_path_for_node(ancestors, info)),
    );
    upsert(&mut out, "fb.owner.id", info.id.clone());
    upsert(
        &mut out,
        "fb.owner.source_id",
        info.attrs.get("data-fb-id").cloned(),
    );
    if !info.classes.is_empty() {
        upsert(&mut out, "fb.owner.classes", Some(info.classes.join(" ")));
    }
    upsert(
        &mut out,
        "fb.owner.role",
        info.attrs.get("data-fb-role").cloned(),
    );
    upsert(
        &mut out,
        "fb.owner.page",
        info.attrs.get("data-fb-page").cloned(),
    );

    let component = out
        .iter()
        .find_map(|(key, value)| match key.as_str() {
            "component" | "fb.component" | "fb.owner.component" => Some(value.clone()),
            _ => None,
        })
        .or_else(|| info.attrs.get("data-fb-component").cloned());
    upsert(&mut out, "fb.owner.component", component);

    out
}

#[allow(clippy::too_many_arguments)]
fn anonymous_table_cell_run_flowables(
    node: &NodeRef,
    resolver: &StyleResolver,
    parent_style: &ComputedStyle,
    ancestors: &[ElementInfo],
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<Arc<AssetBundle>>,
    mut report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
) -> Option<Vec<LayoutItem>> {
    if node.children().any(
        |child| matches!(child.data(), NodeData::Text(text) if !text.borrow().trim().is_empty()),
    ) {
        return None;
    }
    let children: Vec<NodeRef> = node
        .children()
        .filter(|child| child.as_element().is_some())
        .collect();
    if children.is_empty() {
        return None;
    }

    let include_prev_siblings = resolver.has_sibling_selectors();
    let mut styled_cells = Vec::with_capacity(children.len());
    let mut trailing_replaced = Vec::new();
    for child in children {
        let child_element = child.as_element().expect("filtered element child");
        let info = element_info(&child, include_prev_siblings);
        let inline_style = node_inline_style_attr(&child);
        let style = resolver.compute_style(&info, parent_style, inline_style.as_deref(), ancestors);
        if matches!(style.display, DisplayMode::None) {
            continue;
        }
        let is_table_cell = matches!(style.display, DisplayMode::TableCell);
        let is_inline_replaced_sibling = child_element.name.local.as_ref() == "img"
            && matches!(
                style.display,
                DisplayMode::Inline | DisplayMode::InlineBlock
            );
        if is_inline_replaced_sibling {
            trailing_replaced.push(child);
            continue;
        }
        if !is_table_cell || !trailing_replaced.is_empty() {
            return None;
        }
        // Rich descendants use the general CSS table path; the anonymous
        // sibling fixup here covers the common atomic/text cell run.
        if child
            .children()
            .any(|descendant| descendant.as_element().is_some())
        {
            return None;
        }
        styled_cells.push((child, info, style));
    }
    if styled_cells.is_empty() {
        return None;
    }

    let mut cells = Vec::with_capacity(styled_cells.len());
    for (child, cell_info, cell_style) in styled_cells {
        if style_can_mutate_counters(&cell_style) {
            apply_style_counters(&cell_style, counters);
        }
        let mut text = extract_text(&child, cell_style.white_space);
        if !preserve_whitespace(cell_style.white_space) {
            text = text.trim().to_string();
        }
        text = apply_text_transform(&text, cell_style.text_transform);
        let text_style = text_style_for_flow_text(&cell_style);
        let mut before_counter_probe = counters.clone();
        let before_items = pseudo_items_for(
            resolver,
            &cell_info,
            &cell_style,
            ancestors,
            &mut before_counter_probe,
            font_registry.clone(),
            asset_bundle.as_deref(),
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
            crate::style::PseudoTarget::Before,
        );
        let mut after_counter_probe = counters.clone();
        let after_items = pseudo_items_for(
            resolver,
            &cell_info,
            &cell_style,
            ancestors,
            &mut after_counter_probe,
            font_registry.clone(),
            asset_bundle.as_deref(),
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
            crate::style::PseudoTarget::After,
        );
        let has_generated_content = !before_items.is_empty() || !after_items.is_empty();
        if !has_generated_content && !text.is_empty() {
            report_missing_glyphs(
                report.as_deref_mut(),
                font_registry.as_deref(),
                &text_style,
                &text,
            );
        }
        let cell_content = if has_generated_content {
            let mut items = before_items;
            items.extend(text_node_to_flowables(
                &text,
                &cell_style,
                true,
                true,
                font_registry.clone(),
                report.as_deref_mut(),
                perf,
                doc_id,
                true,
            ));
            items.extend(after_items);
            let items = coerce_items_to_inline_run(
                items,
                VerticalAlign::Baseline,
                &cell_style,
                font_registry.clone(),
                false,
            );
            let mut flowables = layout_children_to_flowables(items, None);
            if flowables.is_empty() {
                None
            } else if flowables.len() == 1 {
                Some(flowables.remove(0))
            } else {
                Some(Box::new(
                    ContainerFlowable::new_pt(
                        flowables,
                        cell_style.font_size,
                        cell_style.root_font_size,
                    )
                    .with_self_visible(cell_style.visibility.paints()),
                ) as Box<dyn Flowable>)
            }
        } else {
            None
        };
        let border_colors = cell_style.resolved_border_colors(cell_style.color);
        let border_opacities = cell_style.resolved_border_opacities();
        let border_styles = cell_style.resolved_border_styles();
        let hidden_borders = cell_style.border_hidden_sides();
        let min_height = match cell_style.height {
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial => Pt::ZERO,
            value => value
                .resolve_height(Pt::ZERO, cell_style.font_size, cell_style.root_font_size)
                .max(Pt::ZERO),
        };
        let mut cell = TableCell::new(
            if cell_content.is_some() {
                String::new()
            } else {
                text
            },
            text_style,
            text_align_from_style(&cell_style),
            vertical_align_from_style(&cell_style),
            cell_style.padding,
            cell_style.background_color,
            BorderSpec {
                widths: cell_style.border_width,
                color: border_colors.top,
            },
            cell_style.box_shadow.clone(),
            Some(Arc::<str>::from("TD")),
            None,
            1,
            cell_style.root_font_size,
            font_registry.clone(),
            preserve_whitespace(cell_style.white_space),
            no_wrap(&cell_style),
        )
        .with_border_styles(
            border_styles.top,
            border_styles.right,
            border_styles.bottom,
            border_styles.left,
        )
        .with_border_colors(
            border_colors.top,
            border_colors.right,
            border_colors.bottom,
            border_colors.left,
        )
        .with_border_opacities(
            border_opacities.top,
            border_opacities.right,
            border_opacities.bottom,
            border_opacities.left,
        )
        .with_hidden_borders(
            hidden_borders.top,
            hidden_borders.right,
            hidden_borders.bottom,
            hidden_borders.left,
        )
        .with_row_min_height(min_height)
        .with_hide_empty_cells(cell_style.empty_cells_hide)
        .with_self_visible(cell_style.visibility.paints())
        .with_overflow_hidden(matches!(
            cell_style.overflow,
            OverflowMode::Hidden | OverflowMode::Clip
        ));
        if let Some(content) = cell_content {
            cell = cell.with_content(content).with_inline_content_phase(true);
        }
        if !matches!(
            cell_style.width,
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
        ) {
            cell = cell.with_preferred_width(
                cell_style.width,
                cell_style.font_size,
                cell_style.root_font_size,
            );
        }
        cells.push(cell);
    }

    let table = TableFlowable::new(vec![cells])
        .with_row_backgrounds(false)
        // Anonymous table boxes inherit the surrounding table model's spacing.
        // This matters for runs of `display: table-cell` descendants generated
        // beneath a non-table wrapper (CSS 2.1 anonymous table fixup).
        .with_border_spacing(parent_style.border_spacing)
        .with_direction(parent_style.direction)
        .with_font_metrics(parent_style.font_size, parent_style.root_font_size)
        .with_tag_role("Table");
    let used_width = table.intrinsic_width().unwrap_or(Pt::ZERO);
    let container = ContainerFlowable::new_pt(
        vec![Box::new(table) as Box<dyn Flowable>],
        parent_style.font_size,
        parent_style.root_font_size,
    )
    .with_width(LengthSpec::Absolute(used_width));
    let mut items = vec![LayoutItem::Block {
        flowable: Box::new(container),
        flex_grow: 0.0,
        flex_shrink: 1.0,
        width_spec: Some(LengthSpec::Absolute(used_width)),
        order: 0,
    }];
    for child in trailing_replaced {
        let mut child_ancestors = ancestors.to_vec();
        items.extend(node_to_flowables(
            &child,
            resolver,
            parent_style,
            &mut child_ancestors,
            counters,
            font_registry.clone(),
            asset_bundle.clone(),
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
            perf,
            doc_id,
        ));
    }
    Some(items)
}

fn collect_children(
    node: &NodeRef,
    resolver: &StyleResolver,
    parent_style: &ComputedStyle,
    ancestors: &mut Vec<ElementInfo>,
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<Arc<AssetBundle>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
) -> Vec<LayoutItem> {
    let mut out = Vec::new();
    let mut report = report;
    if let Some(anonymous_table) = anonymous_table_cell_run_flowables(
        node,
        resolver,
        parent_style,
        ancestors,
        counters,
        font_registry.clone(),
        asset_bundle.clone(),
        report.as_deref_mut(),
        svg_form,
        svg_raster_fallback,
        perf,
        doc_id,
    ) {
        return anonymous_table;
    }
    let children: Vec<NodeRef> = node.children().collect();
    let has_boundary_space_candidate = children.iter().any(|child| match child.data() {
        // Whitespace-only DOM nodes between atomic inline boxes generate one
        // collapsible CSS space. Ignoring indentation-only nodes here removes
        // the browser's inter-box advance and changes both wrapping and box
        // placement.
        NodeData::Text(text) => text.borrow().chars().any(char::is_whitespace),
        _ => false,
    });
    let inline_context = (matches!(parent_style.display, DisplayMode::Inline)
        || has_boundary_space_candidate)
        && !matches!(
            parent_style.display,
            DisplayMode::Flex
                | DisplayMode::InlineFlex
                | DisplayMode::Grid
                | DisplayMode::InlineGrid
        )
        && inline_or_replaced_children_only(node, resolver, parent_style, ancestors);
    for (index, child) in children.iter().enumerate() {
        if inline_context {
            if let NodeData::Text(text) = child.data() {
                let has_before = children[..index].iter().any(dom_child_has_inline_content);
                let has_after = children[index + 1..]
                    .iter()
                    .any(dom_child_has_inline_content);
                out.extend(text_node_to_flowables(
                    &text.borrow(),
                    parent_style,
                    !has_before,
                    !has_after,
                    font_registry.clone(),
                    report.as_deref_mut(),
                    perf,
                    doc_id,
                    true,
                ));
                continue;
            }
        }
        out.extend(node_to_flowables(
            child,
            resolver,
            parent_style,
            ancestors,
            counters,
            font_registry.clone(),
            asset_bundle.clone(),
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
            perf,
            doc_id,
        ));
    }
    out
}

fn dom_child_has_inline_content(node: &NodeRef) -> bool {
    match node.data() {
        NodeData::Text(text) => !text.borrow().trim().is_empty(),
        NodeData::Element(element) => !matches!(element.name.local.as_ref(), "script" | "style"),
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn text_node_to_flowables(
    text: &str,
    parent_style: &ComputedStyle,
    trim_start: bool,
    trim_end: bool,
    font_registry: Option<Arc<FontRegistry>>,
    mut report: Option<&mut GlyphCoverageReport>,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
    inline_context: bool,
) -> Vec<LayoutItem> {
    if let Some(perf_logger) = perf {
        perf_logger.log_counts("story.text_nodes", doc_id, &[("count", 1)]);
    }
    let t_norm = std::time::Instant::now();
    let cleaned =
        normalize_text_node_boundaries(text, parent_style.white_space, trim_start, trim_end);
    if let Some(perf_logger) = perf {
        let ms = t_norm.elapsed().as_secs_f64() * 1000.0;
        perf_logger.log_span_ms("story.text.normalize", doc_id, ms);
    }
    let split_boundary_spaces = !preserve_whitespace(parent_style.white_space);
    let leading_space = split_boundary_spaces && cleaned.starts_with(' ');
    let trailing_space = split_boundary_spaces && cleaned.ends_with(' ');
    let cleaned = if split_boundary_spaces {
        cleaned.trim_matches(' ').to_string()
    } else {
        cleaned
    };
    if cleaned.is_empty() && !leading_space && !trailing_space {
        return Vec::new();
    }

    let t_transform = std::time::Instant::now();
    let cleaned = apply_text_transform(&cleaned, parent_style.text_transform);
    if let Some(perf_logger) = perf {
        let ms = t_transform.elapsed().as_secs_f64() * 1000.0;
        perf_logger.log_span_ms("story.text.transform", doc_id, ms);
    }
    let text_style = text_style_for_flow_text(parent_style);
    let t_glyph = std::time::Instant::now();
    if !cleaned.is_empty() {
        report_missing_glyphs(
            report.as_deref_mut(),
            font_registry.as_deref(),
            &text_style,
            &cleaned,
        );
    }
    if let Some(perf_logger) = perf {
        let ms = t_glyph.elapsed().as_secs_f64() * 1000.0;
        perf_logger.log_span_ms("story.glyph.report", doc_id, ms);
    }
    let inline_item = |flowable: Box<dyn Flowable>| {
        if inline_context {
            LayoutItem::Inline {
                flowable,
                valign: anonymous_text_vertical_align(parent_style),
                flex_grow: 0.0,
                flex_shrink: 1.0,
                width_spec: None,
                order: 0,
            }
        } else {
            LayoutItem::Block {
                flowable,
                flex_grow: 0.0,
                flex_shrink: 1.0,
                width_spec: None,
                order: 0,
            }
        }
    };
    let mut items = Vec::new();
    let has_text = !cleaned.is_empty();
    if leading_space || (!has_text && trailing_space) {
        items.push(inline_item(Box::new(CollapsibleSpaceFlowable::new(
            text_style.clone(),
            font_registry.clone(),
        ))));
    }
    if has_text {
        let paragraph = Paragraph::new(cleaned)
            .with_style(text_style.clone())
            .with_align(text_align_from_style(parent_style))
            .with_last_align(text_align_last_from_style(parent_style))
            .with_whitespace(
                preserve_whitespace(parent_style.white_space),
                no_wrap(parent_style),
            )
            .with_break_spaces(matches!(
                parent_style.white_space,
                WhiteSpaceMode::BreakSpaces
            ))
            .with_pagination(parent_style.pagination)
            .with_font_registry(font_registry.clone())
            .with_tag_role("P");
        items.push(inline_item(Box::new(paragraph)));
    }
    if trailing_space && has_text {
        items.push(inline_item(Box::new(CollapsibleSpaceFlowable::new(
            text_style,
            font_registry,
        ))));
    }
    items
}

fn inherited_subgrid_line_names(
    parent: &[Vec<String>],
    authored: &[Vec<String>],
    start: usize,
    span: usize,
) -> Vec<Vec<String>> {
    let mut lines = vec![Vec::new(); span.saturating_add(1)];
    for (local, names) in lines.iter_mut().enumerate() {
        if let Some(parent_names) = parent.get(start.saturating_add(local)) {
            names.extend(parent_names.iter().cloned());
        }
    }
    for (local, names) in authored.iter().enumerate() {
        if let Some(target) = lines.get_mut(local.min(span)) {
            for name in names {
                if !target.contains(name) {
                    target.push(name.clone());
                }
            }
        }
    }
    lines
}

fn adopt_parent_subgrid_tracks(style: &mut ComputedStyle, parent: &ComputedStyle) {
    if !matches!(parent.display, DisplayMode::Grid | DisplayMode::InlineGrid) {
        return;
    }
    if style.grid_subgrid_columns {
        let names = grid_line_name_map(
            &parent.grid_column_line_names,
            &parent.grid_template_areas,
            true,
        );
        let (start, span) = resolve_grid_axis(
            &style.grid_column_line_start,
            &style.grid_column_line_end,
            parent.grid_column_tracks.len().max(1),
            &names,
        )
        .unwrap_or((0, parent.grid_column_tracks.len().max(1)));
        let authored = style.grid_column_line_names.clone();
        style.grid_column_tracks = (0..span)
            .map(|offset| {
                parent
                    .grid_column_tracks
                    .get(start.saturating_add(offset))
                    .copied()
                    .unwrap_or_else(GridTrackSize::auto)
            })
            .collect();
        style.grid_columns = Some(span.max(1));
        style.grid_column_line_names =
            inherited_subgrid_line_names(&parent.grid_column_line_names, &authored, start, span);
        style.gap = parent.gap;
    }
    if style.grid_subgrid_rows {
        let names = grid_line_name_map(
            &parent.grid_row_line_names,
            &parent.grid_template_areas,
            false,
        );
        let (start, span) = resolve_grid_axis(
            &style.grid_row_line_start,
            &style.grid_row_line_end,
            parent.grid_row_tracks.len().max(1),
            &names,
        )
        .unwrap_or((0, parent.grid_row_tracks.len().max(1)));
        let authored = style.grid_row_line_names.clone();
        style.grid_row_tracks = (0..span)
            .map(|offset| {
                parent
                    .grid_row_tracks
                    .get(start.saturating_add(offset))
                    .copied()
                    .unwrap_or_else(GridTrackSize::auto)
            })
            .collect();
        style.grid_rows = Some(span.max(1));
        style.grid_row_line_names =
            inherited_subgrid_line_names(&parent.grid_row_line_names, &authored, start, span);
        style.row_gap = parent.row_gap;
    }
}

fn compiled_page_footnote_area(resolver: &StyleResolver) -> PageFootnoteAreaStyle {
    let area = resolver.page_footnote_area();
    PageFootnoteAreaStyle {
        border_top_width: area.border_top_width.unwrap_or(Pt::ZERO).max(Pt::ZERO),
        border_top_color: area.border_top_color.unwrap_or(Color::BLACK),
        border_top_visible: area
            .border_top_visible
            .unwrap_or_else(|| area.border_top_width.is_some_and(|width| width > Pt::ZERO)),
        padding_top: area.padding_top.unwrap_or(Pt::ZERO).max(Pt::ZERO),
        max_height: area.max_height.map(|height| height.max(Pt::ZERO)),
    }
}

fn footnote_pseudo_text_item(
    text: String,
    style: &ComputedStyle,
    containing_font_size: Pt,
    default_superscript: bool,
    font_registry: Option<Arc<FontRegistry>>,
    report: Option<&mut GlyphCoverageReport>,
) -> LayoutItem {
    let text = apply_text_transform(&text, style.text_transform);
    let mut text_style = text_style_for_flow_text(style);
    if default_superscript {
        // CSS Fonts synthesizes a missing superscript form at 80% of the
        // originating em. Its paint is shifted separately so the generated
        // call does not participate in the owning line's baseline union.
        text_style.font_size = containing_font_size.mul_ratio(4, 5);
        text_style.line_height = text_style.font_size;
        text_style.line_height_is_auto = true;
    }
    report_missing_glyphs(report, font_registry.as_deref(), &text_style, &text);
    let paragraph = Paragraph::new(text)
        .with_style(text_style)
        .with_align(text_align_from_style(style))
        .with_last_align(text_align_last_from_style(style))
        .with_whitespace(false, true)
        .with_font_registry(font_registry);
    let valign = if default_superscript {
        VerticalAlign::Baseline
    } else {
        vertical_align_from_style_with_font_size(style, containing_font_size)
    };
    LayoutItem::Inline {
        flowable: Box::new(paragraph),
        valign,
        flex_grow: 0.0,
        flex_shrink: 1.0,
        width_spec: None,
        order: 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn footnote_node_to_layout_item(
    node: &NodeRef,
    resolver: &StyleResolver,
    info: &ElementInfo,
    style: &ComputedStyle,
    ancestors: &mut Vec<ElementInfo>,
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<Arc<AssetBundle>>,
    mut report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
) -> LayoutItem {
    if !style
        .counter_increment
        .iter()
        .any(|mutation| mutation.name == "footnote")
    {
        counters.increment("footnote", 1);
    }
    let number = counters.get("footnote");

    let call_pseudo = resolver.compute_pseudo_style(
        info,
        style,
        ancestors,
        crate::style::PseudoTarget::FootnoteCall,
    );
    let call_style = call_pseudo.as_ref().unwrap_or(style);
    let call_text = call_pseudo
        .as_ref()
        .and_then(|pseudo| generated_content_text(pseudo, counters))
        .unwrap_or_else(|| number.to_string());
    let call_item = footnote_pseudo_text_item(
        call_text,
        call_style,
        style.font_size,
        call_pseudo.is_none(),
        font_registry.clone(),
        report.as_deref_mut(),
    );
    let (call_flowable, call_valign) = match call_item {
        LayoutItem::Inline {
            flowable, valign, ..
        } => (flowable, valign),
        LayoutItem::Block { .. } => unreachable!("footnote calls compile as inline content"),
    };

    let marker_pseudo = resolver.compute_pseudo_style(
        info,
        style,
        ancestors,
        crate::style::PseudoTarget::FootnoteMarker,
    );
    let marker_style = marker_pseudo.as_ref().unwrap_or(style);
    let marker_text = marker_pseudo
        .as_ref()
        .and_then(|pseudo| generated_content_text(pseudo, counters))
        .unwrap_or_else(|| format!("{number}. "));
    let direct_text_body = marker_pseudo.is_none()
        && node
            .children()
            .all(|child| matches!(child.data(), NodeData::Text(_)));
    let note_body = if direct_text_body {
        // Compile a default marker and its plain body into one paragraph. This
        // lets the marker take part in the first line's shaping and wrapping;
        // treating a multi-line body as an atomic inline would align the
        // marker to its last baseline and create clipped visual overflow.
        let body_text = extract_text(node, style.white_space);
        let body_text = apply_text_transform(&body_text, style.text_transform);
        let text = format!("{marker_text}{body_text}");
        let text_style = text_style_for_flow_text(style);
        report_missing_glyphs(
            report.as_deref_mut(),
            font_registry.as_deref(),
            &text_style,
            &text,
        );
        Box::new(
            Paragraph::new(text)
                .with_style(text_style)
                .with_align(text_align_from_style(style))
                .with_last_align(text_align_last_from_style(style))
                .with_whitespace(preserve_whitespace(style.white_space), no_wrap(style))
                .with_break_spaces(matches!(style.white_space, WhiteSpaceMode::BreakSpaces))
                .with_pagination(style.pagination)
                .with_font_registry(font_registry.clone())
                .with_tag_role("P"),
        ) as Box<dyn Flowable>
    } else {
        let marker_item = footnote_pseudo_text_item(
            marker_text,
            marker_style,
            style.font_size,
            false,
            font_registry.clone(),
            report.as_deref_mut(),
        );

        ancestors.push(info.clone());
        let body_items = collect_children(
            node,
            resolver,
            style,
            ancestors,
            counters,
            font_registry.clone(),
            asset_bundle,
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
            perf,
            doc_id,
        );
        ancestors.pop();
        let mut note_items = vec![marker_item];
        note_items.extend(coerce_items_to_inline_run(
            body_items,
            VerticalAlign::Baseline,
            style,
            font_registry,
            false,
        ));
        let mut note_flowables = layout_children_to_flowables(note_items, None);
        if note_flowables.len() == 1 {
            note_flowables.pop().expect("one compiled footnote body")
        } else {
            Box::new(
                ContainerFlowable::new_pt(note_flowables, style.font_size, style.root_font_size)
                    .with_self_visible(style.visibility.paints()),
            ) as Box<dyn Flowable>
        }
    };

    let entry = PageFootnoteEntry {
        body: note_body,
        display: style.footnote_display,
        policy: style.footnote_policy,
        area: compiled_page_footnote_area(resolver),
    };
    let call = FootnoteCallFlowable::new(call_flowable, entry);
    let call = if call_pseudo.is_none() {
        // GCPM's default `font-variant-position: super` keeps the call's
        // baseline-aligned inline metrics and translates only the synthesized
        // glyph paint by 0.38em.
        call.with_paint_shift_y(-style.font_size.mul_ratio(38, 100))
    } else {
        call
    };
    LayoutItem::Inline {
        flowable: Box::new(call),
        valign: call_valign,
        flex_grow: 0.0,
        flex_shrink: 1.0,
        width_spec: None,
        order: style.order,
    }
}

fn node_to_flowables(
    node: &NodeRef,
    resolver: &StyleResolver,
    parent_style: &ComputedStyle,
    ancestors: &mut Vec<ElementInfo>,
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<Arc<AssetBundle>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
) -> Vec<LayoutItem> {
    let mut report = report;
    match node.data() {
        NodeData::Text(text) => text_node_to_flowables(
            &text.borrow(),
            parent_style,
            true,
            true,
            font_registry,
            report.as_deref_mut(),
            perf,
            doc_id,
            false,
        ),
        NodeData::Element(element) => {
            if let Some(perf_logger) = perf {
                perf_logger.log_counts("story.elements", doc_id, &[("count", 1)]);
            }
            let t_info = std::time::Instant::now();
            let mut info = element_info(node, resolver.has_sibling_selectors());
            if let Some(perf_logger) = perf {
                let ms = t_info.elapsed().as_secs_f64() * 1000.0;
                perf_logger.log_span_ms("story.element_info", doc_id, ms);
                perf_logger.log_counts(
                    "story.classes",
                    doc_id,
                    &[("count", info.classes.len() as u64)],
                );
                perf_logger.log_counts(
                    "story.attrs",
                    doc_id,
                    &[("count", info.attrs.len() as u64)],
                );
            }
            let inline_style = element
                .attributes
                .borrow()
                .get("style")
                .map(|s| s.to_string());
            let explicit_node_meta = element
                .attributes
                .borrow()
                .get("data-fb")
                .map(parse_data_fb)
                .unwrap_or_default();
            if inline_style.is_some() {
                if let Some(perf_logger) = perf {
                    perf_logger.log_counts("story.inline_style", doc_id, &[("count", 1)]);
                }
            }
            let t_style = std::time::Instant::now();
            // DOM compilation is recursive and `ComputedStyle` is deliberately
            // wide. Heap-own the per-element style so nesting depth does not
            // multiply that value across the native thread stack.
            let mut style = compute_boxed_style(
                resolver,
                &info,
                parent_style,
                inline_style.as_deref(),
                ancestors,
            );
            resolve_html_auto_direction(node, &mut style);
            resolve_inline_svg_mask_sources(node, &mut style);
            resolve_inline_svg_clip_source(node, &mut style);
            resolve_inline_svg_filter_sources(node, &mut style);
            adopt_parent_subgrid_tracks(&mut style, parent_style);
            info.apply_computed_container_style(&style);
            if let Some(perf_logger) = perf {
                let ms = t_style.elapsed().as_secs_f64() * 1000.0;
                perf_logger.log_span_ms("story.style.compute", doc_id, ms);
            }
            let parent_is_grid = matches!(
                parent_style.display,
                DisplayMode::Grid | DisplayMode::InlineGrid
            );
            let suppress_unused_multicol_rule =
                parent_is_grid || style.word_spacing != parent_style.word_spacing;
            let parent_is_flex = matches!(
                parent_style.display,
                DisplayMode::Flex
                    | DisplayMode::InlineFlex
                    | DisplayMode::Grid
                    | DisplayMode::InlineGrid
            );
            let mut flex_item_basis_override = None;
            if parent_is_flex {
                let parent_main_axis_is_vertical = if matches!(
                    parent_style.display,
                    DisplayMode::Flex | DisplayMode::InlineFlex
                ) {
                    let logical_column = matches!(
                        parent_style.flex_direction,
                        FlexDirectionMode::Column | FlexDirectionMode::ColumnReverse
                    );
                    let vertical_writing =
                        !matches!(parent_style.writing_mode, WritingModeMode::HorizontalTb);
                    logical_column != vertical_writing
                } else {
                    false
                };
                if !matches!(
                    style.flex_basis,
                    LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
                ) {
                    flex_item_basis_override = Some(style.flex_basis);
                } else if parent_main_axis_is_vertical {
                    if !matches!(
                        style.height,
                        LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
                    ) {
                        flex_item_basis_override = Some(style.height);
                    }
                } else if !matches!(
                    style.width,
                    LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
                ) {
                    flex_item_basis_override = Some(style.width);
                }
            }
            if info.classes.iter().any(|c| c == "keep-together") {
                style.pagination.break_inside = BreakInside::Avoid;
            }
            let node_meta = authored_owner_metadata(&info, ancestors, &explicit_node_meta, &style);

            // `data-fb-a11y-only` is an explicit document-compiler primitive,
            // not a browser clipping trick. Preserve resolved source text in
            // tagged reading order without creating visual flow geometry. The
            // authored inline hiding style remains useful when the
            // same source is opened or exported as ordinary HTML. Compile the
            // zero-size carrier in flow so its tag remains at the exact source
            // reading-order position; the semantic primitive ignores visual
            // positioning/float declarations rather than entering a deferred
            // paint phase.
            let screen_reader_text = info
                .attrs
                .get("data-fb-a11y-only")
                .filter(|value| data_attribute_value_is_truthy(value))
                .map(|_| extract_text(node, style.white_space))
                .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|text| !text.is_empty());
            if screen_reader_text.is_some() {
                style.position = PositionMode::Static;
                style.float_mode = FloatMode::None;
                style.clear_mode = ClearMode::None;
            }

            if matches!(style.display, DisplayMode::None) && screen_reader_text.is_none() {
                return Vec::new();
            }
            let counter_reset_scopes = if style_can_mutate_counters(&style) {
                apply_style_counters_for_node(node, resolver, &style, &info, ancestors, counters)
            } else {
                Vec::new()
            };
            if style_is_css_list_item(&style) {
                apply_implicit_list_item_counter(&style, counters);
            }

            if matches!(style.float_mode, FloatMode::Footnote) {
                let item = footnote_node_to_layout_item(
                    node,
                    resolver,
                    &info,
                    &style,
                    ancestors,
                    counters,
                    font_registry,
                    asset_bundle,
                    report.as_deref_mut(),
                    svg_form,
                    svg_raster_fallback,
                    perf,
                    doc_id,
                );
                counters.pop_reset_scopes(&counter_reset_scopes);
                return vec![item];
            }

            let mut before_counter_probe = counters.clone();
            let before_items = pseudo_items_for(
                resolver,
                &info,
                &style,
                ancestors,
                &mut before_counter_probe,
                font_registry.clone(),
                asset_bundle.as_deref(),
                report.as_deref_mut(),
                svg_form,
                svg_raster_fallback,
                crate::style::PseudoTarget::Before,
            );
            let mut after_counter_probe = counters.clone();
            let after_items = pseudo_items_for(
                resolver,
                &info,
                &style,
                ancestors,
                &mut after_counter_probe,
                font_registry.clone(),
                asset_bundle.as_deref(),
                report.as_deref_mut(),
                svg_form,
                svg_raster_fallback,
                crate::style::PseudoTarget::After,
            );
            let first_letter_style = resolver.compute_pseudo_style(
                &info,
                &style,
                ancestors,
                crate::style::PseudoTarget::FirstLetter,
            );
            let pseudo_quote_depth_changes = before_counter_probe.quote_depth
                != counters.quote_depth
                || after_counter_probe.quote_depth != counters.quote_depth;
            let has_structured_pseudo = !before_items.is_empty() || !after_items.is_empty();

            // Maintain an ancestor stack instead of cloning it for every element.
            let list_item_marker_ancestors = if style_is_css_list_item(&style) {
                Some(ancestors.clone())
            } else {
                None
            };
            ancestors.push(info.clone());

            // `display: contents` generates no principal box. Its children and
            // generated content participate directly in the parent formatting
            // context, while paint and box-model properties on the discarded
            // box have no effect.
            if matches!(style.display, DisplayMode::Contents) {
                let out = collect_children(
                    node,
                    resolver,
                    &style,
                    ancestors,
                    counters,
                    font_registry.clone(),
                    asset_bundle.clone(),
                    report.as_deref_mut(),
                    svg_form,
                    svg_raster_fallback,
                    perf,
                    doc_id,
                );
                let out = inject_pseudo_items(out, &before_items, &after_items);
                ancestors.pop();
                counters.pop_reset_scopes(&counter_reset_scopes);
                return out;
            }

            // Inline elements are transparent containers in our layout model,
            // except replaced/special inline elements that render atomically.
            let transparent_inline = screen_reader_text.is_none()
                    && matches!(style.display, DisplayMode::Inline)
                    && !matches!(info.tag.as_str(), "img" | "svg" | "br")
                    // Grid items are blockified even when their specified
                    // outer display type is inline.
                    && !matches!(parent_style.display, DisplayMode::Grid | DisplayMode::InlineGrid)
                    // Positioned inline boxes need the normal wrapper path so the
                    // absolute/relative flowable survives into the parent's layout.
                    // Returning the children directly here silently discarded
                    // `position` and left absolute text in the inline run.
                    && matches!(style.position, PositionMode::Static)
                    && style.running_name.is_none()
                    && style.string_set.is_empty();
            if transparent_inline {
                let out = if pseudo_quote_depth_changes {
                    // Quote nesting is a document-order state machine. Resolve
                    // an opening pseudo against the live state before walking
                    // descendants, then resolve the closing pseudo afterward.
                    // Ordinary pseudo probes remain isolated so counter sizing
                    // and fast-path selection stay side-effect free.
                    let pseudo_ancestors = ancestors[..ancestors.len().saturating_sub(1)].to_vec();
                    let ordered_before = pseudo_items_for(
                        resolver,
                        &info,
                        &style,
                        &pseudo_ancestors,
                        counters,
                        font_registry.clone(),
                        asset_bundle.as_deref(),
                        report.as_deref_mut(),
                        svg_form,
                        svg_raster_fallback,
                        crate::style::PseudoTarget::Before,
                    );
                    let children = collect_children(
                        node,
                        resolver,
                        &style,
                        ancestors,
                        counters,
                        font_registry.clone(),
                        asset_bundle.clone(),
                        report.as_deref_mut(),
                        svg_form,
                        svg_raster_fallback,
                        perf,
                        doc_id,
                    );
                    let ordered_after = pseudo_items_for(
                        resolver,
                        &info,
                        &style,
                        &pseudo_ancestors,
                        counters,
                        font_registry.clone(),
                        asset_bundle.as_deref(),
                        report.as_deref_mut(),
                        svg_form,
                        svg_raster_fallback,
                        crate::style::PseudoTarget::After,
                    );
                    inject_transparent_inline_pseudo_items(
                        children,
                        &ordered_before,
                        &ordered_after,
                        node,
                        &style,
                        font_registry.clone(),
                    )
                } else {
                    let children = collect_children(
                        node,
                        resolver,
                        &style,
                        ancestors,
                        counters,
                        font_registry.clone(),
                        asset_bundle.clone(),
                        report.as_deref_mut(),
                        svg_form,
                        svg_raster_fallback,
                        perf,
                        doc_id,
                    );
                    inject_transparent_inline_pseudo_items(
                        children,
                        &before_items,
                        &after_items,
                        node,
                        &style,
                        font_registry.clone(),
                    )
                };
                let out = if matches!(
                    style.vertical_align,
                    VerticalAlignMode::Sub | VerticalAlignMode::Super
                ) {
                    override_inline_vertical_align(
                        out,
                        vertical_align_from_style_with_font_size(&style, parent_style.font_size),
                    )
                } else {
                    out
                };
                let decorated_inline = style.background_color.is_some()
                    || style.background_paint.is_some()
                    || style.border_width != EdgeSizes::zero()
                    || style.padding != EdgeSizes::zero();
                let simple_background = style.background_color.filter(|_| {
                    style.background_paint.is_none()
                        && style.border_width == EdgeSizes::zero()
                        && style.padding == EdgeSizes::zero()
                        && !out.is_empty()
                        && out
                            .iter()
                            .all(|item| matches!(item, LayoutItem::Inline { .. }))
                });
                if let Some(background) = simple_background {
                    let fragmented_inline = out.len() > 1;
                    let font_box_height = font_registry
                        .as_deref()
                        .and_then(|registry| {
                            registry.vertical_metrics(style.font_name.as_ref(), style.font_size)
                        })
                        .map(|(ascent, descent)| ascent + descent)
                        .unwrap_or(style.font_size * 1.2);
                    let paint_offset_y = if fragmented_inline
                        || matches!(style.white_space, WhiteSpaceMode::BreakSpaces)
                    {
                        // LayoutNG places the inherited break-spaces inline
                        // paint box one CSS pixel below the legacy collapsed-
                        // whitespace phase. Multi-fragment inline boxes use
                        // the same line-fragment phase instead of the atomic
                        // single-child correction.
                        Pt::ZERO
                    } else {
                        -Pt::from_f32(0.75)
                    };
                    let css_pixel_snap = text_style_for_flow_text(&style).css_pixel_snap_metrics;
                    let out = out
                        .into_iter()
                        .map(|item| match item {
                            LayoutItem::Inline {
                                flowable,
                                valign,
                                flex_grow,
                                flex_shrink,
                                width_spec,
                                order,
                            } => LayoutItem::Inline {
                                flowable: Box::new(
                                    InlineBackgroundFlowable::new_pt(
                                        flowable,
                                        background,
                                        font_box_height,
                                        paint_offset_y,
                                    )
                                    .with_css_pixel_snap(css_pixel_snap)
                                    .with_pagination(style.pagination),
                                ),
                                valign,
                                flex_grow,
                                flex_shrink,
                                width_spec,
                                order,
                            },
                            LayoutItem::Block { .. } => unreachable!(
                                "simple inline backgrounds require inline layout fragments"
                            ),
                        })
                        .collect();
                    ancestors.pop();
                    counters.pop_reset_scopes(&counter_reset_scopes);
                    return out;
                }
                if decorated_inline {
                    let intrinsic_width = out
                        .iter()
                        .filter_map(|item| match item {
                            LayoutItem::Block { flowable, .. }
                            | LayoutItem::Inline { flowable, .. } => flowable.intrinsic_width(),
                        })
                        .fold(Pt::ZERO, |sum, width| sum + width);
                    let mut inline_box_style = style.clone();
                    if intrinsic_width > Pt::ZERO
                        && matches!(
                            inline_box_style.width,
                            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
                        )
                    {
                        inline_box_style.width = LengthSpec::Absolute(intrinsic_width);
                    }
                    let Some(flowable) = container_flowable_with_role(out, &inline_box_style, None)
                    else {
                        ancestors.pop();
                        counters.pop_reset_scopes(&counter_reset_scopes);
                        return Vec::new();
                    };
                    ancestors.pop();
                    counters.pop_reset_scopes(&counter_reset_scopes);
                    return vec![LayoutItem::Inline {
                        flowable,
                        valign: vertical_align_from_style_with_font_size(
                            &inline_box_style,
                            parent_style.font_size,
                        ),
                        flex_grow: inline_box_style.flex_grow,
                        flex_shrink: inline_box_style.flex_shrink,
                        width_spec: flex_item_basis(&inline_box_style),
                        order: 0,
                    }];
                }
                ancestors.pop();
                counters.pop_reset_scopes(&counter_reset_scopes);
                return out;
            }

            let mut flowables = if let Some(text) = screen_reader_text.as_ref() {
                vec![LayoutItem::Block {
                    flowable: Box::new(
                        ScreenReaderTextFlowable::new(text.clone())
                            .with_pagination(style.pagination),
                    ) as Box<dyn Flowable>,
                    flex_grow: 0.0,
                    flex_shrink: 0.0,
                    width_spec: None,
                    order: 0,
                }]
            } else if let Some(marker_ancestors) = list_item_marker_ancestors.as_ref() {
                css_display_list_item_flowables(
                    node,
                    resolver,
                    &info,
                    &style,
                    marker_ancestors,
                    ancestors,
                    counters,
                    font_registry.clone(),
                    asset_bundle.clone(),
                    report.as_deref_mut(),
                    svg_form,
                    svg_raster_fallback,
                    perf,
                    doc_id,
                )
            } else {
                match info.tag.as_str() {
                    "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => (|| {
                        let role = match info.tag.as_str() {
                            "h1" => "H1",
                            "h2" => "H2",
                            "h3" => "H3",
                            "h4" => "H4",
                            "h5" => "H5",
                            "h6" => "H6",
                            _ => "P",
                        };

                        if inline_children_only(node, resolver, &style, ancestors)
                            && !has_structured_pseudo
                        {
                            // Resolve the large computed pseudo style only once
                            // the single-paragraph compiler path is selected.
                            // Retaining it on every recursive element frame can
                            // exhaust the smaller Windows test-thread stack on
                            // deeply nested display:contents/grid documents.
                            let pseudo_ancestors = &ancestors[..ancestors.len().saturating_sub(1)];
                            let first_line_style = resolver.compute_pseudo_style(
                                &info,
                                &style,
                                pseudo_ancestors,
                                crate::style::PseudoTarget::FirstLine,
                            );
                            let t_extract = std::time::Instant::now();
                            let mut text = extract_text(node, style.white_space);
                            if let Some(perf_logger) = perf {
                                let ms = t_extract.elapsed().as_secs_f64() * 1000.0;
                                perf_logger.log_span_ms("story.text.extract", doc_id, ms);
                            }
                            let before = pseudo_text_for(
                                resolver,
                                &info,
                                &style,
                                ancestors,
                                counters,
                                font_registry.clone(),
                                report.as_deref_mut(),
                                crate::style::PseudoTarget::Before,
                            );
                            let after = pseudo_text_for(
                                resolver,
                                &info,
                                &style,
                                ancestors,
                                counters,
                                font_registry.clone(),
                                report.as_deref_mut(),
                                crate::style::PseudoTarget::After,
                            );
                            if !before.is_empty() || !after.is_empty() {
                                text = format!("{before}{text}{after}");
                            }
                            if text.is_empty() {
                                container_flowables_with_role(Vec::new(), &style, Some(role))
                            } else {
                                let t_transform = std::time::Instant::now();
                                let text = apply_text_transform(&text, style.text_transform);
                                let text_style = text_style_for_flow_text(&style);
                                let initial_letter =
                                    first_letter_style.as_ref().and_then(|pseudo_style| {
                                        let first_style = text_style_for_flow_text(pseudo_style);
                                        let material = pseudo_style.initial_letter.is_some()
                                            || first_style != text_style
                                            || pseudo_style.background_color.is_some()
                                            || pseudo_style.text_transform != style.text_transform;
                                        if !material {
                                            return None;
                                        }
                                        let end = css_first_letter_prefix_end(&text)?;
                                        let first = apply_text_transform(
                                            &text[..end],
                                            pseudo_style.text_transform,
                                        );
                                        Some((
                                            first,
                                            text[end..].to_string(),
                                            first_style,
                                            pseudo_style.initial_letter,
                                            pseudo_style.background_color,
                                            pseudo_style.background_alpha,
                                        ))
                                    });
                                if let Some(perf_logger) = perf {
                                    let ms = t_transform.elapsed().as_secs_f64() * 1000.0;
                                    perf_logger.log_span_ms("story.text.transform", doc_id, ms);
                                }
                                let first_line =
                                    first_line_style.as_ref().and_then(|pseudo_style| {
                                        let pseudo_text_style =
                                            text_style_for_flow_text(pseudo_style);
                                        (pseudo_text_style != text_style
                                            || pseudo_style.background_color.is_some()
                                            || pseudo_style.text_transform != style.text_transform)
                                            .then_some((
                                                pseudo_text_style,
                                                pseudo_style.background_color,
                                                pseudo_style.background_alpha,
                                                pseudo_style.text_transform,
                                            ))
                                    });
                                let t_glyph = std::time::Instant::now();
                                report_missing_glyphs(
                                    report.as_deref_mut(),
                                    font_registry.as_deref(),
                                    &text_style,
                                    &text,
                                );
                                if let Some((first, _, first_style, _, _, _)) = &initial_letter {
                                    report_missing_glyphs(
                                        report.as_deref_mut(),
                                        font_registry.as_deref(),
                                        first_style,
                                        first,
                                    );
                                }
                                if let Some((first_style, _, _, transform)) = &first_line {
                                    let transformed = apply_text_transform(&text, *transform);
                                    report_missing_glyphs(
                                        report.as_deref_mut(),
                                        font_registry.as_deref(),
                                        first_style,
                                        &transformed,
                                    );
                                }
                                if let Some(perf_logger) = perf {
                                    let ms = t_glyph.elapsed().as_secs_f64() * 1000.0;
                                    perf_logger.log_span_ms("story.glyph.report", doc_id, ms);
                                }
                                let paragraph_text = initial_letter
                                    .as_ref()
                                    .map(|(_, remainder, _, _, _, _)| remainder.clone())
                                    .unwrap_or_else(|| text.clone());
                                let round_baseline = css_direct_text_prefers_nearest_baseline_snap(
                                    &text_style,
                                    font_registry.as_deref(),
                                );
                                let block_owns_top_overflow = text_style.text_shadows.is_empty();
                                let mut paragraph = Paragraph::new(paragraph_text)
                                    .with_style(text_style)
                                    .with_align(text_align_from_style(&style))
                                    .with_last_align(text_align_last_from_style(&style))
                                    .with_whitespace(
                                        preserve_whitespace(style.white_space),
                                        no_wrap(&style),
                                    )
                                    .with_break_spaces(matches!(
                                        style.white_space,
                                        WhiteSpaceMode::BreakSpaces
                                    ))
                                    .with_pagination(style.pagination)
                                    .with_font_registry(font_registry.clone())
                                    .with_tag_role(role);
                                if let Some((
                                    first_style,
                                    background,
                                    background_opacity,
                                    transform,
                                )) = first_line
                                {
                                    paragraph = paragraph.with_first_line_style(
                                        first_style,
                                        background,
                                        background_opacity,
                                        transform,
                                    );
                                }
                                if let Some((
                                    first,
                                    _,
                                    first_style,
                                    value,
                                    background,
                                    background_opacity,
                                )) = initial_letter
                                {
                                    paragraph = paragraph.with_first_letter(
                                        first,
                                        first_style,
                                        value,
                                        background,
                                        background_opacity,
                                    );
                                }
                                let items = vec![LayoutItem::Block {
                                    flowable: Box::new(
                                        CssLineBoxFlowable::new(Box::new(paragraph))
                                            .with_round_baseline(round_baseline)
                                            .with_parent_positioned_top_overflow(
                                                block_owns_top_overflow,
                                            ),
                                    )
                                        as Box<dyn Flowable>,
                                    flex_grow: 0.0,
                                    flex_shrink: 1.0,
                                    width_spec: None,
                                    order: 0,
                                }];
                                container_flowables(items, &style)
                            }
                        } else {
                            let coerce_mixed_inline =
                                pseudo_items_are_inline(&before_items, &after_items)
                                    && inline_or_replaced_children_only(
                                        node, resolver, &style, ancestors,
                                    );
                            let children = collect_children(
                                node,
                                resolver,
                                &style,
                                ancestors,
                                counters,
                                font_registry.clone(),
                                asset_bundle.clone(),
                                report.as_deref_mut(),
                                svg_form,
                                svg_raster_fallback,
                                perf,
                                doc_id,
                            );
                            let children =
                                inject_pseudo_items(children, &before_items, &after_items);
                            let children = if coerce_mixed_inline {
                                coerce_items_to_inline_run(
                                    children,
                                    vertical_align_from_style(&style),
                                    &style,
                                    font_registry.clone(),
                                    true,
                                )
                            } else {
                                children
                            };
                            container_flowables_with_role(children, &style, Some(role))
                        }
                    })(),
                    "pre" => (|| {
                        let t_extract = std::time::Instant::now();
                        let mut text = extract_text(node, WhiteSpaceMode::Pre);
                        if let Some(perf_logger) = perf {
                            let ms = t_extract.elapsed().as_secs_f64() * 1000.0;
                            perf_logger.log_span_ms("story.text.extract", doc_id, ms);
                        }
                        let before = pseudo_text_for(
                            resolver,
                            &info,
                            &style,
                            ancestors,
                            counters,
                            font_registry.clone(),
                            report.as_deref_mut(),
                            crate::style::PseudoTarget::Before,
                        );
                        let after = pseudo_text_for(
                            resolver,
                            &info,
                            &style,
                            ancestors,
                            counters,
                            font_registry.clone(),
                            report.as_deref_mut(),
                            crate::style::PseudoTarget::After,
                        );
                        if !before.is_empty() || !after.is_empty() {
                            text = format!("{before}{text}{after}");
                        }
                        if text.is_empty() {
                            container_flowables(Vec::new(), &style)
                        } else {
                            let t_transform = std::time::Instant::now();
                            let text = apply_text_transform(&text, style.text_transform);
                            if let Some(perf_logger) = perf {
                                let ms = t_transform.elapsed().as_secs_f64() * 1000.0;
                                perf_logger.log_span_ms("story.text.transform", doc_id, ms);
                            }
                            let text_style = text_style_for_flow_text(&style);
                            let t_glyph = std::time::Instant::now();
                            report_missing_glyphs(
                                report.as_deref_mut(),
                                font_registry.as_deref(),
                                &text_style,
                                &text,
                            );
                            if let Some(perf_logger) = perf {
                                let ms = t_glyph.elapsed().as_secs_f64() * 1000.0;
                                perf_logger.log_span_ms("story.glyph.report", doc_id, ms);
                            }
                            let paragraph = Paragraph::new(text)
                                .with_style(text_style)
                                .with_align(text_align_from_style(&style))
                                .with_last_align(text_align_last_from_style(&style))
                                .with_whitespace(true, true)
                                .with_pagination(style.pagination)
                                .with_font_registry(font_registry.clone())
                                .with_tag_role("Code");
                            let items = vec![LayoutItem::Block {
                                flowable: Box::new(paragraph) as Box<dyn Flowable>,
                                flex_grow: 0.0,
                                flex_shrink: 1.0,
                                width_spec: None,
                                order: 0,
                            }];
                            container_flowables(items, &style)
                        }
                    })(),
                    "br" => {
                        let height = style.to_text_style().line_height.max(style.font_size);
                        vec![LayoutItem::Block {
                            flowable: Box::new(ForcedLineBreakFlowable::new(height))
                                as Box<dyn Flowable>,
                            flex_grow: 0.0,
                            flex_shrink: 1.0,
                            width_spec: flex_item_basis(&style),
                            order: 0,
                        }]
                    }
                    "img" => (|| {
                        let attrs = element.attributes.borrow();
                        let src = attrs.get("src").unwrap_or("image");
                        let svg_xml = load_svg_xml_from_image_source(asset_bundle.as_deref(), src);
                        let intrinsic_size =
                            raster_image_intrinsic_dimensions(asset_bundle.as_deref(), src)
                                .map(|(w, h)| {
                                    (Pt::from_f32(w as f32 * 0.75), Pt::from_f32(h as f32 * 0.75))
                                })
                                .or_else(|| {
                                    svg_xml.as_deref().and_then(svg_image_intrinsic_dimensions)
                                });
                        let replaced_sizing = resolve_replaced_image_sizing(
                            &style,
                            parse_dimension(attrs.get("width")),
                            parse_dimension(attrs.get("height")),
                            intrinsic_size,
                        );
                        let width = replaced_sizing.nominal_width;
                        let height = replaced_sizing.nominal_height;
                        let alt = attrs
                            .get("alt")
                            .or_else(|| attrs.get("aria-label"))
                            .or_else(|| attrs.get("title"))
                            .map(|s| s.to_string());
                        if let Some(xml) = svg_xml {
                            if svg_raster_fallback && crate::svg::svg_needs_raster_fallback(&xml) {
                                if let Some(data_uri) =
                                    crate::svg::rasterize_svg_to_data_uri(&xml, width, height)
                                {
                                    let image = ImageFlowable::new_pt(width, height, data_uri)
                                        .with_available_size(true)
                                        .with_object_fit(style.object_fit)
                                        .with_object_position(style.object_position)
                                        .with_image_rendering(style.image_rendering)
                                        .with_intrinsic_size(intrinsic_size)
                                        .with_font_metrics(style.font_size, style.root_font_size)
                                        .with_visible(style.visibility.paints())
                                        .with_tag_role("Figure")
                                        .with_alt(alt);
                                    replaced_image_flowables(image, &style, replaced_sizing)
                                } else {
                                    let xml_len = xml.len() as u64;
                                    let t_svg = std::time::Instant::now();
                                    let svg = SvgFlowable::new_pt(width, height, xml)
                                        .with_form_enabled(svg_form)
                                        .with_object_fit(style.object_fit)
                                        .with_object_position(style.object_position)
                                        .with_intrinsic_size(intrinsic_size)
                                        .with_font_metrics(style.font_size, style.root_font_size)
                                        .with_visible(style.visibility.paints())
                                        .with_tag_role("Figure")
                                        .with_alt(alt);
                                    if let Some(perf_logger) = perf {
                                        let ms = t_svg.elapsed().as_secs_f64() * 1000.0;
                                        perf_logger.log_span_ms("svg.compile", None, ms);
                                        perf_logger.log_counts(
                                            "svg.compile",
                                            None,
                                            &[("bytes", xml_len)],
                                        );
                                    }
                                    replaced_svg_image_flowables(svg, &style, replaced_sizing)
                                }
                            } else {
                                let xml_len = xml.len() as u64;
                                let t_svg = std::time::Instant::now();
                                let svg = SvgFlowable::new_pt(width, height, xml)
                                    .with_form_enabled(svg_form)
                                    .with_object_fit(style.object_fit)
                                    .with_object_position(style.object_position)
                                    .with_intrinsic_size(intrinsic_size)
                                    .with_font_metrics(style.font_size, style.root_font_size)
                                    .with_visible(style.visibility.paints())
                                    .with_tag_role("Figure")
                                    .with_alt(alt);
                                if let Some(perf_logger) = perf {
                                    let ms = t_svg.elapsed().as_secs_f64() * 1000.0;
                                    perf_logger.log_span_ms("svg.compile", None, ms);
                                    perf_logger.log_counts(
                                        "svg.compile",
                                        None,
                                        &[("bytes", xml_len)],
                                    );
                                }
                                replaced_svg_image_flowables(svg, &style, replaced_sizing)
                            }
                        } else {
                            let image_source =
                                renderable_image_source(asset_bundle.as_deref(), src)
                                    .unwrap_or_else(|| src.to_string());
                            let image = ImageFlowable::new_pt(width, height, image_source)
                                .with_available_size(true)
                                .with_object_fit(style.object_fit)
                                .with_object_position(style.object_position)
                                .with_image_rendering(style.image_rendering)
                                .with_intrinsic_size(intrinsic_size)
                                .with_font_metrics(style.font_size, style.root_font_size)
                                .with_visible(style.visibility.paints())
                                .with_tag_role("Figure")
                                .with_alt(alt);
                            replaced_image_flowables(image, &style, replaced_sizing)
                        }
                    })(),
                    "svg" => (|| {
                        // Inline SVG. We intentionally treat this as a leaf node and render it with a
                        // dedicated subset parser, rather than trying to interpret SVG children as HTML.
                        let xml = serialize_svg_node(node);
                        let attrs = element.attributes.borrow();
                        let (inline_w, inline_h) = inline_dimensions(inline_style.as_deref());
                        let (width, height) = resolve_svg_dimensions(
                            inline_w,
                            inline_h,
                            attrs.get("width"),
                            attrs.get("height"),
                            attrs.get("viewBox").or_else(|| attrs.get("viewbox")),
                            &style,
                        );
                        let alt = attrs
                            .get("aria-label")
                            .or_else(|| attrs.get("title"))
                            .map(|s| s.to_string());
                        if svg_raster_fallback && crate::svg::svg_needs_raster_fallback(&xml) {
                            if let Some(data_uri) =
                                crate::svg::rasterize_svg_to_data_uri(&xml, width, height)
                            {
                                let image = ImageFlowable::new_pt(width, height, data_uri)
                                    .with_object_fit(style.object_fit)
                                    .with_object_position(style.object_position)
                                    .with_image_rendering(style.image_rendering)
                                    .with_font_metrics(style.font_size, style.root_font_size)
                                    .with_visible(style.visibility.paints())
                                    .with_tag_role("Figure")
                                    .with_alt(alt);
                                replaced_svg_flowables(
                                    Box::new(image) as Box<dyn Flowable>,
                                    &style,
                                    width,
                                    height,
                                )
                            } else {
                                let xml_len = xml.len() as u64;
                                let t_svg = std::time::Instant::now();
                                let svg = SvgFlowable::new_pt(width, height, xml)
                                    .with_form_enabled(svg_form)
                                    .with_visible(style.visibility.paints())
                                    .with_tag_role("Figure")
                                    .with_alt(alt);
                                if let Some(perf_logger) = perf {
                                    let ms = t_svg.elapsed().as_secs_f64() * 1000.0;
                                    perf_logger.log_span_ms("svg.compile", None, ms);
                                    perf_logger.log_counts(
                                        "svg.compile",
                                        None,
                                        &[("bytes", xml_len)],
                                    );
                                }
                                replaced_svg_flowables(
                                    Box::new(svg) as Box<dyn Flowable>,
                                    &style,
                                    width,
                                    height,
                                )
                            }
                        } else {
                            let xml_len = xml.len() as u64;
                            let t_svg = std::time::Instant::now();
                            let svg = SvgFlowable::new_pt(width, height, xml)
                                .with_form_enabled(svg_form)
                                .with_visible(style.visibility.paints())
                                .with_tag_role("Figure")
                                .with_alt(alt);
                            if let Some(perf_logger) = perf {
                                let ms = t_svg.elapsed().as_secs_f64() * 1000.0;
                                perf_logger.log_span_ms("svg.compile", None, ms);
                                perf_logger.log_counts("svg.compile", None, &[("bytes", xml_len)]);
                            }
                            replaced_svg_flowables(
                                Box::new(svg) as Box<dyn Flowable>,
                                &style,
                                width,
                                height,
                            )
                        }
                    })(),
                    "hr" => {
                        let spacer = Spacer::new_pt(style.to_text_style().line_height * 0.5);
                        vec![LayoutItem::Block {
                            flowable: Box::new(spacer) as Box<dyn Flowable>,
                            flex_grow: 0.0,
                            flex_shrink: 1.0,
                            width_spec: flex_item_basis(&style),
                            order: 0,
                        }]
                    }
                    "table" => (|| {
                        let include_prev_siblings = resolver.has_sibling_selectors();
                        let legacy_border_width = legacy_table_length_attribute(node, "border")
                            .filter(|width| *width > Pt::ZERO);
                        let legacy_cell_spacing =
                            legacy_table_length_attribute(node, "cellspacing");
                        let mut top_caption_flowables: Vec<Box<dyn Flowable>> = Vec::new();
                        let mut bottom_caption_flowables: Vec<Box<dyn Flowable>> = Vec::new();
                        for child in node.children() {
                            let Some(el) = child.as_element() else {
                                continue;
                            };
                            if el.name.local.as_ref() != "caption" {
                                continue;
                            }
                            let caption_info = element_info(&child, include_prev_siblings);
                            let caption_inline_style =
                                el.attributes.borrow().get("style").map(|s| s.to_string());
                            let caption_style = resolver.compute_style(
                                &caption_info,
                                &style,
                                caption_inline_style.as_deref(),
                                ancestors,
                            );
                            let mut caption_text = extract_text(&child, caption_style.white_space);
                            if caption_text.trim().is_empty() {
                                if let Some(flowable) = container_flowable_with_role(
                                    Vec::new(),
                                    &caption_style,
                                    Some("Caption"),
                                ) {
                                    if matches!(
                                        caption_style.caption_side,
                                        crate::style::CaptionSideMode::Bottom
                                    ) {
                                        bottom_caption_flowables.push(flowable);
                                    } else {
                                        top_caption_flowables.push(flowable);
                                    }
                                }
                                continue;
                            }
                            if !preserve_whitespace(caption_style.white_space) {
                                caption_text = caption_text.trim().to_string();
                            }
                            let caption_text =
                                apply_text_transform(&caption_text, caption_style.text_transform);
                            let caption_text_style = text_style_for_flow_text(&caption_style);
                            report_missing_glyphs(
                                report.as_deref_mut(),
                                font_registry.as_deref(),
                                &caption_text_style,
                                &caption_text,
                            );
                            let paragraph = Paragraph::new(caption_text)
                                .with_style(caption_text_style)
                                .with_align(text_align_from_style(&caption_style))
                                .with_last_align(text_align_last_from_style(&caption_style))
                                .with_whitespace(
                                    preserve_whitespace(caption_style.white_space),
                                    no_wrap(&caption_style),
                                )
                                .with_break_spaces(matches!(
                                    caption_style.white_space,
                                    WhiteSpaceMode::BreakSpaces
                                ))
                                .with_pagination(caption_style.pagination)
                                .with_font_registry(font_registry.clone())
                                .with_tag_role("Caption");
                            let caption_side = caption_style.caption_side;
                            if let Some(flowable) = container_flowable_with_role(
                                vec![LayoutItem::Block {
                                    flowable: Box::new(paragraph) as Box<dyn Flowable>,
                                    flex_grow: 0.0,
                                    flex_shrink: 1.0,
                                    width_spec: None,
                                    order: 0,
                                }],
                                &caption_style,
                                Some("Caption"),
                            ) {
                                if matches!(caption_side, crate::style::CaptionSideMode::Bottom) {
                                    bottom_caption_flowables.push(flowable);
                                } else {
                                    top_caption_flowables.push(flowable);
                                }
                            }
                        }

                        let effective_table_layout =
                            if matches!(style.table_layout, TableLayoutMode::Fixed)
                                && matches!(
                                    style.width,
                                    LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
                                )
                            {
                                TableLayoutMode::Auto
                            } else {
                                style.table_layout
                            };
                        let minimum_table_height = match style.height {
                            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial => {
                                Pt::ZERO
                            }
                            height => height
                                .resolve_height(Pt::ZERO, style.font_size, style.root_font_size)
                                .max(Pt::ZERO),
                        };
                        let mut table_border_width = style.border_width;
                        let mut table_border_colors = style.resolved_border_colors(style.color);
                        let mut table_border_styles = style.resolved_border_styles();
                        let mut table_hidden_borders = style.border_hidden_sides();
                        if let Some(width) = legacy_border_width {
                            let borders_are_zero = |edges: EdgeSizes| {
                                let is_zero = |value: LengthSpec| match value {
                                    LengthSpec::Absolute(value) => value <= Pt::ZERO,
                                    LengthSpec::Percent(value)
                                    | LengthSpec::Em(value)
                                    | LengthSpec::Rem(value) => value <= 0.0,
                                    LengthSpec::Calc(value) => {
                                        value.abs <= Pt::ZERO
                                            && value.percent <= 0.0
                                            && value.em <= 0.0
                                            && value.rem <= 0.0
                                    }
                                    LengthSpec::Clamped(_) => false,
                                    LengthSpec::FontRelative(_) => false,
                                    LengthSpec::Auto
                                    | LengthSpec::Content
                                    | LengthSpec::MinContent
                                    | LengthSpec::MaxContent
                                    | LengthSpec::FitContent
                                    | LengthSpec::Inherit
                                    | LengthSpec::Initial => true,
                                };
                                is_zero(edges.top)
                                    && is_zero(edges.right)
                                    && is_zero(edges.bottom)
                                    && is_zero(edges.left)
                            };
                            if borders_are_zero(table_border_width) {
                                let width = LengthSpec::Absolute(width);
                                table_border_width = EdgeSizes {
                                    top: width,
                                    right: width,
                                    bottom: width,
                                    left: width,
                                };
                                let light = Color::rgb(238.0 / 255.0, 238.0 / 255.0, 238.0 / 255.0);
                                let dark = Color::rgb(154.0 / 255.0, 154.0 / 255.0, 154.0 / 255.0);
                                table_border_colors.top = light;
                                table_border_colors.left = light;
                                table_border_colors.right = dark;
                                table_border_colors.bottom = dark;
                                table_border_styles.top = OutlineLineStyle::Solid;
                                table_border_styles.right = OutlineLineStyle::Solid;
                                table_border_styles.bottom = OutlineLineStyle::Solid;
                                table_border_styles.left = OutlineLineStyle::Solid;
                                table_hidden_borders.top = false;
                                table_hidden_borders.right = false;
                                table_hidden_borders.bottom = false;
                                table_hidden_borders.left = false;
                            }
                        }
                        let effective_border_spacing = if let Some(spacing) = legacy_cell_spacing {
                            let spacing = LengthSpec::Absolute(spacing);
                            BorderSpacingSpec {
                                horizontal: spacing,
                                vertical: spacing,
                            }
                        } else if legacy_border_width.is_some()
                            && style.border_spacing == BorderSpacingSpec::zero()
                        {
                            let spacing = LengthSpec::Absolute(Pt::from_f32(1.5));
                            BorderSpacingSpec {
                                horizontal: spacing,
                                vertical: spacing,
                            }
                        } else {
                            style.border_spacing
                        };
                        let collapsed_table = matches!(
                            style.border_collapse,
                            crate::flowable::BorderCollapseMode::Collapse
                        );

                        let table = table_flowable(
                            node,
                            &style,
                            resolver,
                            ancestors,
                            counters,
                            font_registry.clone(),
                            asset_bundle.clone(),
                            report.as_deref_mut(),
                            svg_form,
                            svg_raster_fallback,
                            perf,
                            doc_id,
                        )
                        .with_tag_role("Table")
                        .with_border_collapse(style.border_collapse)
                        .with_border_spacing(effective_border_spacing)
                        .with_table_layout(effective_table_layout)
                        .with_direction(style.direction)
                        .with_table_border(
                            table_border_width,
                            style.border_color.unwrap_or(style.color),
                        )
                        .with_table_border_colors(
                            table_border_colors.top,
                            table_border_colors.right,
                            table_border_colors.bottom,
                            table_border_colors.left,
                        )
                        .with_table_border_styles(
                            table_border_styles.top,
                            table_border_styles.right,
                            table_border_styles.bottom,
                            table_border_styles.left,
                        )
                        .with_table_hidden_borders(
                            table_hidden_borders.top,
                            table_hidden_borders.right,
                            table_hidden_borders.bottom,
                            table_hidden_borders.left,
                        )
                        .with_font_metrics(style.font_size, style.root_font_size);
                        let table = table.with_minimum_height(minimum_table_height);

                        let table_intrinsic_width = table.intrinsic_width();
                        let has_caption = !top_caption_flowables.is_empty()
                            || !bottom_caption_flowables.is_empty();
                        let caption_width_overflow = match style.width {
                            LengthSpec::Absolute(width) if has_caption => {
                                table.collapsed_caption_width_overflow(width)
                            }
                            _ => Pt::ZERO,
                        };
                        if caption_width_overflow > Pt::ZERO {
                            top_caption_flowables = top_caption_flowables
                                .into_iter()
                                .map(|caption| {
                                    Box::new(ExpandedWidthFlowable::new(
                                        caption,
                                        caption_width_overflow,
                                    )) as Box<dyn Flowable>
                                })
                                .collect();
                            bottom_caption_flowables = bottom_caption_flowables
                                .into_iter()
                                .map(|caption| {
                                    Box::new(ExpandedWidthFlowable::new(
                                        caption,
                                        caption_width_overflow,
                                    )) as Box<dyn Flowable>
                                })
                                .collect();
                        }
                        let mut table_children: Vec<Box<dyn Flowable>> = Vec::new();
                        table_children.extend(top_caption_flowables);
                        table_children.push(Box::new(table) as Box<dyn Flowable>);
                        table_children.extend(bottom_caption_flowables);
                        let used_table_width = if matches!(
                            style.width,
                            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
                        ) {
                            table_children
                                .iter()
                                .filter_map(|child| child.intrinsic_width())
                                .reduce(Pt::max)
                                .map(|content_width| {
                                    let container_border = if collapsed_table {
                                        EdgeSizes::zero()
                                    } else {
                                        table_border_width
                                    };
                                    let resolve = |value: LengthSpec| {
                                        value.resolve_width(
                                            content_width,
                                            style.font_size,
                                            style.root_font_size,
                                        )
                                    };
                                    let horizontal_edges = resolve(container_border.left)
                                        + resolve(container_border.right)
                                        + resolve(style.padding.left)
                                        + resolve(style.padding.right);
                                    let used = if matches!(
                                        style.box_sizing,
                                        crate::types::BoxSizingMode::BorderBox
                                    ) {
                                        content_width + horizontal_edges
                                    } else {
                                        content_width
                                    };
                                    LengthSpec::Absolute(used)
                                })
                                .unwrap_or(style.width)
                        } else if has_caption && collapsed_table {
                            match (style.width, table_intrinsic_width) {
                                (LengthSpec::Absolute(width), Some(intrinsic)) => {
                                    LengthSpec::Absolute(width.max(intrinsic))
                                }
                                _ => style.width,
                            }
                        } else {
                            style.width
                        };

                        let container = ContainerFlowable::new_pt(
                            table_children,
                            style.font_size,
                            style.root_font_size,
                        )
                        .with_establishes_abs_containing_block(establishes_abs_containing_block(
                            &style,
                        ))
                        .with_margin(style.margin)
                        .with_border(
                            if collapsed_table {
                                EdgeSizes::zero()
                            } else {
                                table_border_width
                            },
                            style.border_color.unwrap_or(style.color),
                        )
                        .with_border_colors(
                            table_border_colors.top,
                            table_border_colors.right,
                            table_border_colors.bottom,
                            table_border_colors.left,
                        )
                        .with_border_opacities(
                            style.resolved_border_opacities().top,
                            style.resolved_border_opacities().right,
                            style.resolved_border_opacities().bottom,
                            style.resolved_border_opacities().left,
                        )
                        .with_border_styles(
                            table_border_styles.top,
                            table_border_styles.right,
                            table_border_styles.bottom,
                            table_border_styles.left,
                        )
                        .with_border_radius(style.border_radius)
                        .with_box_decoration_break(style.box_decoration_break)
                        .with_border_image(style.border_image.clone())
                        .with_outline(
                            style.outline_width,
                            style.outline_offset,
                            style.outline_style,
                            style.resolved_outline_color(),
                            style.outline_visible,
                        )
                        .with_padding(style.padding)
                        .with_box_sizing(style.box_sizing)
                        .with_width(used_table_width)
                        .with_max_width(style.max_width)
                        .with_min_width(style.min_width)
                        .with_height(style.height)
                        .with_aspect_ratio(style.aspect_ratio)
                        .with_min_height(style.min_height)
                        .with_max_height(style.max_height)
                        .with_background(style.background_source_color.or(style.background_color))
                        .with_background_opacity(style.background_alpha)
                        .with_background_paint(style.background_paint.clone())
                        .with_background_layers(
                            style.background_paints.clone(),
                            style.background_sizes.clone(),
                            style.background_positions.clone(),
                            style.background_repeats.clone(),
                            style.background_attachments.clone(),
                            style.background_origins.clone(),
                            style.background_clips.clone(),
                        )
                        .with_background_blend_modes(style.background_blend_modes.clone())
                        .with_clip_path(style.clip_path.clone())
                        .with_clip_path_reference_box(style.clip_path_reference_box)
                        .with_legacy_clip(effective_legacy_clip(&style))
                        .with_box_shadows(style.box_shadows.clone())
                        .with_paint_filter(style.paint_filter.clone())
                        .with_backdrop_filter(style.backdrop_filter.clone())
                        .with_will_change_backdrop_root(style.will_change_backdrop_root)
                        .with_mask(style.mask.clone())
                        .with_mask_backdrop_root(style.mask_backdrop_root)
                        .with_mix_blend_mode(style.mix_blend_mode)
                        .with_isolation(style.isolation)
                        .with_opacity(style.opacity)
                        .with_transforms(style.transform.clone())
                        .with_transform_origin(style.transform_origin)
                        .with_transform_box(style.transform_box)
                        .with_perspective(style.perspective, style.perspective_origin)
                        .with_transform_style(style.transform_style)
                        .with_overflow_modes(style.overflow_x, style.overflow_y)
                        .with_overflow_clip_margin(style.overflow_clip_margin)
                        .with_scrollbar_gutter(
                            style.scrollbar_gutter,
                            style.direction,
                            style.writing_mode,
                        )
                        .with_line_clamp(
                            style.line_clamp,
                            text_style_for_flow_text(&style).line_height,
                        )
                        .with_self_visible(style.visibility.paints())
                        .with_pagination(style.pagination);

                        vec![LayoutItem::Block {
                            flowable: Box::new(container) as Box<dyn Flowable>,
                            flex_grow: style.flex_grow,
                            flex_shrink: style.flex_shrink,
                            width_spec: flex_item_basis(&style),
                            order: 0,
                        }]
                    })(),
                    "ul" | "ol" => (|| {
                        let items = list_flowables(
                            node,
                            resolver,
                            &style,
                            ancestors,
                            counters,
                            font_registry.clone(),
                            asset_bundle.clone(),
                            report.as_deref_mut(),
                            svg_form,
                            svg_raster_fallback,
                            perf,
                            doc_id,
                        );
                        let items = inject_pseudo_items(items, &before_items, &after_items);
                        let containers = container_flowables_with_role(items, &style, Some("L"));
                        if ancestors.iter().any(|ancestor| ancestor.tag == "li") {
                            containers
                        } else {
                            containers
                                .into_iter()
                                .map(|item| match item {
                                    LayoutItem::Block {
                                        flowable,
                                        flex_grow,
                                        flex_shrink,
                                        width_spec,
                                        order,
                                    } => LayoutItem::Block {
                                        flowable: Box::new(CssPixelHeightFlowable::new(flowable))
                                            as Box<dyn Flowable>,
                                        flex_grow,
                                        flex_shrink,
                                        width_spec,
                                        order,
                                    },
                                    LayoutItem::Inline {
                                        flowable,
                                        valign,
                                        flex_grow,
                                        flex_shrink,
                                        width_spec,
                                        order,
                                    } => LayoutItem::Inline {
                                        flowable: Box::new(CssPixelHeightFlowable::new(flowable))
                                            as Box<dyn Flowable>,
                                        valign,
                                        flex_grow,
                                        flex_shrink,
                                        width_spec,
                                        order,
                                    },
                                })
                                .collect()
                        }
                    })(),
                    "li" => (|| {
                        let text = extract_text(node, style.white_space);
                        if text.is_empty() {
                            container_flowables_with_role(Vec::new(), &style, Some("LI"))
                        } else {
                            let text = apply_text_transform(&text, style.text_transform);
                            let label = format!("- {}", text);
                            let text_style = text_style_for_flow_text(&style);
                            report_missing_glyphs(
                                report.as_deref_mut(),
                                font_registry.as_deref(),
                                &text_style,
                                &label,
                            );
                            let paragraph = Paragraph::new(label)
                                .with_style(text_style)
                                .with_align(text_align_from_style(&style))
                                .with_last_align(text_align_last_from_style(&style))
                                .with_whitespace(
                                    preserve_whitespace(style.white_space),
                                    no_wrap(&style),
                                )
                                .with_break_spaces(matches!(
                                    style.white_space,
                                    WhiteSpaceMode::BreakSpaces
                                ))
                                .with_pagination(style.pagination)
                                .with_font_registry(font_registry.clone())
                                .with_tag_role("LI");
                            vec![LayoutItem::Block {
                                flowable: Box::new(paragraph) as Box<dyn Flowable>,
                                flex_grow: 0.0,
                                flex_shrink: 1.0,
                                width_spec: flex_item_basis(&style),
                                order: 0,
                            }]
                        }
                    })(),
                    "body" | "div" | "span" | "i" | "section" | "article" | "header" | "footer"
                    | "aside" | "nav" | "main" | "blockquote" | "figure" | "figcaption" | "dl"
                    | "dt" | "dd" => (|| {
                        let dl_container_role = definition_list_container_role(info.tag.as_str());
                        let direct_text_role = direct_text_structure_role(info.tag.as_str());
                        if is_table_container_display(style.display) {
                            table_container_flowables(
                                node,
                                resolver,
                                &style,
                                ancestors,
                                counters,
                                font_registry.clone(),
                                asset_bundle.clone(),
                                report.as_deref_mut(),
                                svg_form,
                                svg_raster_fallback,
                                perf,
                                doc_id,
                                &before_items,
                                &after_items,
                            )
                        } else if matches!(
                            style.display,
                            DisplayMode::Flex
                                | DisplayMode::InlineFlex
                                | DisplayMode::Grid
                                | DisplayMode::InlineGrid
                        ) {
                            flex_container_flowables(
                                node,
                                resolver,
                                &style,
                                ancestors,
                                counters,
                                font_registry.clone(),
                                asset_bundle.clone(),
                                report.as_deref_mut(),
                                svg_form,
                                svg_raster_fallback,
                                perf,
                                doc_id,
                                &before_items,
                                &after_items,
                            )
                        } else if matches!(
                            style.display,
                            DisplayMode::Block | DisplayMode::FlowRoot | DisplayMode::InlineBlock
                        ) && inline_children_only(node, resolver, &style, ancestors)
                            && !has_structured_pseudo
                        {
                            let ancestors_no_self = &ancestors[..ancestors.len().saturating_sub(1)];
                            let mut text = extract_text(node, style.white_space);
                            let before = pseudo_text_for(
                                resolver,
                                &info,
                                &style,
                                ancestors_no_self,
                                counters,
                                font_registry.clone(),
                                report.as_deref_mut(),
                                crate::style::PseudoTarget::Before,
                            );
                            let after = pseudo_text_for(
                                resolver,
                                &info,
                                &style,
                                ancestors_no_self,
                                counters,
                                font_registry.clone(),
                                report.as_deref_mut(),
                                crate::style::PseudoTarget::After,
                            );
                            if !before.is_empty() || !after.is_empty() {
                                text = format!("{before}{text}{after}");
                            }
                            let container_options = ContainerCompilationOptions {
                                suppress_single_used_column_rule: suppress_unused_multicol_rule,
                            };
                            if text.is_empty() {
                                if matches!(info.tag.as_str(), "dl") {
                                    container_flowables_with_role_options(
                                        Vec::new(),
                                        &style,
                                        dl_container_role,
                                        container_options,
                                    )
                                } else {
                                    container_flowables_with_options(
                                        Vec::new(),
                                        &style,
                                        container_options,
                                    )
                                }
                            } else {
                                let text = apply_text_transform(&text, style.text_transform);
                                let text_style = text_style_for_flow_text(&style);
                                let round_baseline = css_direct_text_prefers_nearest_baseline_snap(
                                    &text_style,
                                    font_registry.as_deref(),
                                );
                                let block_owns_top_overflow = text_style.text_shadows.is_empty();
                                report_missing_glyphs(
                                    report.as_deref_mut(),
                                    font_registry.as_deref(),
                                    &text_style,
                                    &text,
                                );
                                let paragraph = Paragraph::new(text)
                                    .with_style(text_style)
                                    .with_align(text_align_from_style(&style))
                                    .with_last_align(text_align_last_from_style(&style))
                                    .with_whitespace(
                                        preserve_whitespace(style.white_space),
                                        no_wrap(&style),
                                    )
                                    .with_break_spaces(matches!(
                                        style.white_space,
                                        WhiteSpaceMode::BreakSpaces
                                    ))
                                    .with_pagination(style.pagination)
                                    .with_font_registry(font_registry.clone())
                                    .with_tag_role(direct_text_role);
                                let items = vec![LayoutItem::Block {
                                    flowable: Box::new(
                                        CssLineBoxFlowable::new(Box::new(paragraph))
                                            .with_round_baseline(round_baseline)
                                            .with_parent_positioned_top_overflow(
                                                block_owns_top_overflow,
                                            ),
                                    )
                                        as Box<dyn Flowable>,
                                    flex_grow: 0.0,
                                    flex_shrink: 1.0,
                                    width_spec: None,
                                    order: 0,
                                }];
                                if matches!(info.tag.as_str(), "dl") {
                                    container_flowables_with_role_options(
                                        items,
                                        &style,
                                        dl_container_role,
                                        container_options,
                                    )
                                } else {
                                    container_flowables_with_options(
                                        items,
                                        &style,
                                        container_options,
                                    )
                                }
                            }
                        } else {
                            let coerce_mixed_inline = info.tag.as_str() != "dl"
                                && pseudo_items_are_inline(&before_items, &after_items)
                                && inline_or_replaced_children_only(
                                    node, resolver, &style, ancestors,
                                );
                            let children = if info.tag.as_str() == "dl" {
                                definition_list_children_flowables(
                                    node,
                                    resolver,
                                    &style,
                                    ancestors,
                                    counters,
                                    font_registry.clone(),
                                    asset_bundle.clone(),
                                    report.as_deref_mut(),
                                    svg_form,
                                    svg_raster_fallback,
                                    perf,
                                    doc_id,
                                )
                            } else {
                                collect_children(
                                    node,
                                    resolver,
                                    &style,
                                    ancestors,
                                    counters,
                                    font_registry.clone(),
                                    asset_bundle.clone(),
                                    report.as_deref_mut(),
                                    svg_form,
                                    svg_raster_fallback,
                                    perf,
                                    doc_id,
                                )
                            };
                            let children =
                                inject_pseudo_items(children, &before_items, &after_items);
                            let children = if coerce_mixed_inline {
                                coerce_items_to_inline_run(
                                    children,
                                    vertical_align_from_style(&style),
                                    &style,
                                    font_registry.clone(),
                                    true,
                                )
                            } else {
                                children
                            };
                            if dl_container_role.is_some() {
                                container_flowables_with_role_options(
                                    children,
                                    &style,
                                    dl_container_role,
                                    ContainerCompilationOptions {
                                        suppress_single_used_column_rule:
                                            suppress_unused_multicol_rule,
                                    },
                                )
                            } else {
                                container_flowables_with_options(
                                    children,
                                    &style,
                                    ContainerCompilationOptions {
                                        suppress_single_used_column_rule:
                                            suppress_unused_multicol_rule,
                                    },
                                )
                            }
                        }
                    })(),
                    _ => (|| {
                        if is_table_container_display(style.display) {
                            table_container_flowables(
                                node,
                                resolver,
                                &style,
                                ancestors,
                                counters,
                                font_registry.clone(),
                                asset_bundle.clone(),
                                report.as_deref_mut(),
                                svg_form,
                                svg_raster_fallback,
                                perf,
                                doc_id,
                                &before_items,
                                &after_items,
                            )
                        } else {
                            let children = collect_children(
                                node,
                                resolver,
                                &style,
                                ancestors,
                                counters,
                                font_registry.clone(),
                                asset_bundle.clone(),
                                report.as_deref_mut(),
                                svg_form,
                                svg_raster_fallback,
                                perf,
                                doc_id,
                            );
                            inject_pseudo_items(children, &before_items, &after_items)
                        }
                    })(),
                }
            };

            // A visible replaced inline can paint outside a fixed-height block. CSS paints that
            // inline content after later in-flow block backgrounds and borders in the same
            // stacking context. Our block layout normally draws each sibling atomically, so
            // defer this otherwise-static wrapper into the zero z-index paint phase. It remains
            // in flow and therefore keeps exactly the same Q32.32 layout geometry.
            let defer_visible_replaced_overflow = !parent_is_flex
                && static_block_with_visible_replaced_overflow(node, resolver, &style, ancestors);

            // Preserve metadata-only nodes (for example <div data-fb="..."></div>) so
            // feature flags can be emitted even when the element has no visual content.
            // Use a tiny spacer to ensure the flowable is drawable; zero-height carriers can
            // be skipped in layout paths and lose metadata emission.
            if flowables.is_empty() && !node_meta.is_empty() {
                let carrier = Spacer::new_pt(Pt::from_f32(0.01)).with_pagination(style.pagination);
                flowables.push(LayoutItem::Block {
                    flowable: Box::new(carrier) as Box<dyn Flowable>,
                    flex_grow: style.flex_grow,
                    flex_shrink: style.flex_shrink,
                    width_spec: flex_item_basis(&style),
                    order: style.order,
                });
            }

            if matches!(style.position, PositionMode::Absolute | PositionMode::Fixed) {
                flowables = wrap_absolute(flowables, &style);
            } else if matches!(style.position, PositionMode::Relative) {
                flowables = wrap_relative(flowables, &style);
            } else if defer_visible_replaced_overflow {
                flowables = wrap_relative(flowables, &style);
            } else if !parent_is_flex && !matches!(style.float_mode, FloatMode::None) {
                flowables = wrap_float(flowables, &style);
            }
            if !parent_is_flex && !matches!(style.clear_mode, ClearMode::None) {
                flowables = wrap_clear(flowables, &style);
            }
            let named_strings = named_string_values_for_node(node, &style);
            if let Some(running_name) = style.running_name.as_ref() {
                flowables =
                    wrap_running_element(flowables, &style, running_name.clone(), named_strings);
            } else if !named_strings.is_empty() {
                flowables = wrap_named_string_occurrence(flowables, named_strings);
            }

            let width_spec_override = if parent_is_flex {
                flex_item_basis_override
            } else {
                None
            };
            let mut items: Vec<LayoutItem> = flowables
                .into_iter()
                .map(|it| it.with_flex_grow(style.flex_grow))
                .map(|it| it.with_flex_shrink(style.flex_shrink))
                .map(|it| it.with_order(style.order))
                .map(|item| {
                    if let Some(spec) = width_spec_override {
                        match item {
                            LayoutItem::Block {
                                flowable,
                                flex_grow,
                                flex_shrink,
                                order,
                                ..
                            } => LayoutItem::Block {
                                flowable,
                                flex_grow,
                                flex_shrink,
                                width_spec: Some(spec),
                                order,
                            },
                            LayoutItem::Inline {
                                flowable,
                                valign,
                                flex_grow,
                                flex_shrink,
                                order,
                                ..
                            } => LayoutItem::Inline {
                                flowable,
                                valign,
                                flex_grow,
                                flex_shrink,
                                width_spec: Some(spec),
                                order,
                            },
                        }
                    } else {
                        item
                    }
                })
                .collect();
            if !node_meta.is_empty() {
                items = items
                    .into_iter()
                    .map(|item| match item {
                        LayoutItem::Block {
                            flowable,
                            flex_grow,
                            flex_shrink,
                            width_spec,
                            order,
                        } => LayoutItem::Block {
                            flowable: Box::new(MetaFlowable::new(flowable, node_meta.clone()))
                                as Box<dyn Flowable>,
                            flex_grow,
                            flex_shrink,
                            width_spec,
                            order,
                        },
                        LayoutItem::Inline {
                            flowable,
                            valign,
                            flex_grow,
                            flex_shrink,
                            width_spec,
                            order,
                        } => LayoutItem::Inline {
                            flowable: Box::new(MetaFlowable::new(flowable, node_meta.clone()))
                                as Box<dyn Flowable>,
                            valign,
                            flex_grow,
                            flex_shrink,
                            width_spec,
                            order,
                        },
                    })
                    .collect();
            }
            // Absolute/fixed boxes, floats, flex items, and grid items have a
            // blockified outer display. Their inner formatting context remains
            // inline-block/table/flex/grid, but converting the completed outer
            // wrapper back into a LayoutItem::Inline would hide out-of-flow and
            // z-index traits inside an anonymous line box.
            let blockified_outer_display =
                matches!(style.position, PositionMode::Absolute | PositionMode::Fixed)
                    || !matches!(style.float_mode, FloatMode::None)
                    || parent_is_flex;
            if !blockified_outer_display
                && matches!(
                    style.display,
                    DisplayMode::InlineBlock
                        | DisplayMode::InlineTable
                        | DisplayMode::InlineFlex
                        | DisplayMode::InlineGrid
                )
            {
                let valign = vertical_align_from_style(&style);
                items = items
                    .into_iter()
                    .map(|item| match item {
                        LayoutItem::Block {
                            flowable,
                            flex_grow,
                            flex_shrink,
                            width_spec,
                            order,
                        }
                        | LayoutItem::Inline {
                            flowable,
                            flex_grow,
                            flex_shrink,
                            width_spec,
                            order,
                            ..
                        } => LayoutItem::Inline {
                            flowable,
                            valign,
                            flex_grow,
                            flex_shrink,
                            width_spec,
                            order,
                        },
                    })
                    .collect();
            }

            ancestors.pop();
            counters.pop_reset_scopes(&counter_reset_scopes);
            items
        }
        _ => Vec::new(),
    }
}

fn serialize_svg_node(node: &NodeRef) -> String {
    // The HTML parser applies HTML recovery semantics, while the SVG compiler expects well-formed
    // XML. Emit a minimal XML serialization for the foreign-content subtree.
    let mut out = String::new();
    write_svg_xml(node, &mut out);
    out
}

fn resolve_inline_svg_mask_sources(node: &NodeRef, style: &mut ComputedStyle) {
    if !style.mask.paints.iter().any(|paint| {
        matches!(paint, BackgroundPaint::Image { source } if source.trim().starts_with('#'))
    }) {
        return;
    }
    let Some(root) = node.ancestors().last() else {
        return;
    };
    if style.mask.modes.len() < style.mask.paints.len() {
        style
            .mask
            .modes
            .resize(style.mask.paints.len(), MaskMode::MatchSource);
    }

    for (index, paint) in style.mask.paints.iter_mut().enumerate() {
        let BackgroundPaint::Image { source } = paint else {
            continue;
        };
        let Some(id) = source.trim().strip_prefix('#') else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let Ok(selected) = root.select_first(&format!("#{id}")) else {
            continue;
        };
        let mask_node = selected.as_node();
        let Some(mask_element) = mask_node.as_element() else {
            continue;
        };
        if !mask_element
            .name
            .local
            .as_ref()
            .eq_ignore_ascii_case("mask")
        {
            continue;
        }
        let attrs = mask_element.attributes.borrow();
        let x = attrs.get("x").unwrap_or("0");
        let y = attrs.get("y").unwrap_or("0");
        let width = attrs.get("width").unwrap_or("100");
        let height = attrs.get("height").unwrap_or("100");
        let mask_type = attrs
            .get("mask-type")
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| "luminance".to_string());
        let mut xml = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"{x} {y} {width} {height}\">"
        );
        drop(attrs);
        for child in mask_node.children() {
            write_svg_xml(&child, &mut xml);
        }
        xml.push_str("</svg>");
        *source = format!(
            "data:image/svg+xml;base64,{}",
            crate::base64::encode_standard(xml.as_bytes())
        );
        if style.mask.modes[index] == MaskMode::MatchSource {
            style.mask.modes[index] = if mask_type == "alpha" {
                MaskMode::Alpha
            } else {
                MaskMode::Luminance
            };
        }
    }
}

fn resolve_inline_svg_clip_source(node: &NodeRef, style: &mut ComputedStyle) {
    let Some(ClipPathShapeSpec::Url(source)) = style.clip_path.as_ref() else {
        return;
    };
    let Some(id) = source.trim().strip_prefix('#').map(str::to_string) else {
        return;
    };
    if id.is_empty() {
        return;
    }
    let Some(root) = node.ancestors().last() else {
        return;
    };
    let Ok(selected) = root.select_first(&format!("#{id}")) else {
        return;
    };
    let clip_node = selected.as_node();
    let Some(element) = clip_node.as_element() else {
        return;
    };
    if !element.name.local.as_ref().eq_ignore_ascii_case("clipPath") {
        return;
    }

    let object_bounding_box = node_attribute(clip_node, "clipPathUnits")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("objectBoundingBox"));
    let evenodd = clip_path_rule_is_evenodd(clip_node);
    let children = clip_node
        .children()
        .filter(|child| child.as_element().is_some())
        .collect::<Vec<_>>();
    let Some(resolved) = resolve_single_svg_clip_shape(&children, object_bounding_box, evenodd)
        .or_else(|| resolve_svg_clip_paths(&children, object_bounding_box, evenodd))
    else {
        return;
    };

    style.clip_path = Some(resolved);
    style.clip_path_inset = None;
}

fn node_attribute(node: &NodeRef, name: &str) -> Option<String> {
    let element = node.as_element()?;
    element
        .attributes
        .borrow()
        .map
        .iter()
        .find(|(key, _)| key.local.as_ref().eq_ignore_ascii_case(name))
        .map(|(_, value)| value.value.clone())
}

fn clip_path_rule_is_evenodd(clip_node: &NodeRef) -> bool {
    std::iter::once(clip_node.clone())
        .chain(clip_node.children())
        .any(|node| {
            node_attribute(&node, "clip-rule")
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("evenodd"))
                || node_attribute(&node, "style").is_some_and(|value| {
                    value.to_ascii_lowercase().split(';').any(|declaration| {
                        declaration.split_once(':').is_some_and(|(name, value)| {
                            name.trim() == "clip-rule" && value.trim() == "evenodd"
                        })
                    })
                })
        })
}

fn svg_bbox_fraction(raw: Option<String>, default: f32) -> Option<f32> {
    let Some(raw) = raw else {
        return Some(default);
    };
    let raw = raw.trim();
    let value = if let Some(percent) = raw.strip_suffix('%') {
        percent.trim().parse::<f32>().ok()? / 100.0
    } else {
        raw.parse::<f32>().ok()?
    };
    value.is_finite().then_some(value)
}

fn svg_user_pt(raw: Option<String>, default: f32) -> Option<Pt> {
    let Some(raw) = raw else {
        return Some(Pt::from_f32(default * 0.75));
    };
    let raw = raw.trim();
    let number = raw.strip_suffix("px").unwrap_or(raw).trim();
    let value = number.parse::<f32>().ok()?;
    value
        .is_finite()
        .then_some(Pt::from_f32(value * (72.0 / 96.0)))
}

fn resolve_single_svg_clip_shape(
    children: &[NodeRef],
    object_bounding_box: bool,
    _evenodd: bool,
) -> Option<ClipPathShapeSpec> {
    let [child] = children else {
        return None;
    };
    let tag = child.as_element()?.name.local.to_string();
    if object_bounding_box {
        match tag.to_ascii_lowercase().as_str() {
            "circle" => {
                let cx = svg_bbox_fraction(node_attribute(child, "cx"), 0.0)?;
                let cy = svg_bbox_fraction(node_attribute(child, "cy"), 0.0)?;
                let radius = svg_bbox_fraction(node_attribute(child, "r"), 0.0)?.max(0.0);
                Some(ClipPathShapeSpec::Ellipse(ClipPathEllipseSpec {
                    radius_x: ClipPathShapeRadius::Length(LengthSpec::Percent(radius)),
                    radius_y: ClipPathShapeRadius::Length(LengthSpec::Percent(radius)),
                    center_x: LengthSpec::Percent(cx),
                    center_y: LengthSpec::Percent(cy),
                }))
            }
            "ellipse" => Some(ClipPathShapeSpec::Ellipse(ClipPathEllipseSpec {
                radius_x: ClipPathShapeRadius::Length(LengthSpec::Percent(
                    svg_bbox_fraction(node_attribute(child, "rx"), 0.0)?.max(0.0),
                )),
                radius_y: ClipPathShapeRadius::Length(LengthSpec::Percent(
                    svg_bbox_fraction(node_attribute(child, "ry"), 0.0)?.max(0.0),
                )),
                center_x: LengthSpec::Percent(svg_bbox_fraction(node_attribute(child, "cx"), 0.0)?),
                center_y: LengthSpec::Percent(svg_bbox_fraction(node_attribute(child, "cy"), 0.0)?),
            })),
            "rect" => Some(ClipPathShapeSpec::Xywh(ClipPathXywhSpec {
                x: LengthSpec::Percent(svg_bbox_fraction(node_attribute(child, "x"), 0.0)?),
                y: LengthSpec::Percent(svg_bbox_fraction(node_attribute(child, "y"), 0.0)?),
                width: LengthSpec::Percent(
                    svg_bbox_fraction(node_attribute(child, "width"), 0.0)?.max(0.0),
                ),
                height: LengthSpec::Percent(
                    svg_bbox_fraction(node_attribute(child, "height"), 0.0)?.max(0.0),
                ),
                radius: None,
            })),
            _ => None,
        }
    } else {
        match tag.to_ascii_lowercase().as_str() {
            "circle" => Some(ClipPathShapeSpec::Circle(ClipPathCircleSpec {
                radius: ClipPathShapeRadius::Length(LengthSpec::Absolute(
                    svg_user_pt(node_attribute(child, "r"), 0.0)?.max(Pt::ZERO),
                )),
                center_x: LengthSpec::Absolute(svg_user_pt(node_attribute(child, "cx"), 0.0)?),
                center_y: LengthSpec::Absolute(svg_user_pt(node_attribute(child, "cy"), 0.0)?),
            })),
            "ellipse" => Some(ClipPathShapeSpec::Ellipse(ClipPathEllipseSpec {
                radius_x: ClipPathShapeRadius::Length(LengthSpec::Absolute(
                    svg_user_pt(node_attribute(child, "rx"), 0.0)?.max(Pt::ZERO),
                )),
                radius_y: ClipPathShapeRadius::Length(LengthSpec::Absolute(
                    svg_user_pt(node_attribute(child, "ry"), 0.0)?.max(Pt::ZERO),
                )),
                center_x: LengthSpec::Absolute(svg_user_pt(node_attribute(child, "cx"), 0.0)?),
                center_y: LengthSpec::Absolute(svg_user_pt(node_attribute(child, "cy"), 0.0)?),
            })),
            "rect" => Some(ClipPathShapeSpec::Xywh(ClipPathXywhSpec {
                x: LengthSpec::Absolute(svg_user_pt(node_attribute(child, "x"), 0.0)?),
                y: LengthSpec::Absolute(svg_user_pt(node_attribute(child, "y"), 0.0)?),
                width: LengthSpec::Absolute(
                    svg_user_pt(node_attribute(child, "width"), 0.0)?.max(Pt::ZERO),
                ),
                height: LengthSpec::Absolute(
                    svg_user_pt(node_attribute(child, "height"), 0.0)?.max(Pt::ZERO),
                ),
                radius: None,
            })),
            _ => None,
        }
    }
}

fn resolve_svg_clip_paths(
    children: &[NodeRef],
    object_bounding_box: bool,
    evenodd: bool,
) -> Option<ClipPathShapeSpec> {
    if object_bounding_box {
        return None;
    }
    let mut data = Vec::new();
    let mut commands = Vec::new();
    for child in children {
        let Some(element) = child.as_element() else {
            continue;
        };
        if !element.name.local.as_ref().eq_ignore_ascii_case("path") {
            continue;
        }
        let path_data = node_attribute(child, "d")?;
        for command in crate::svg::parse_svg_path_data(&path_data) {
            let to_pt = |value: f32| Pt::from_f32(value * (72.0 / 96.0));
            commands.push(match command {
                crate::svg::SvgPathSegment::MoveTo(x, y) => ClipPathPathCommand::MoveTo {
                    x: to_pt(x),
                    y: to_pt(y),
                },
                crate::svg::SvgPathSegment::LineTo(x, y) => ClipPathPathCommand::LineTo {
                    x: to_pt(x),
                    y: to_pt(y),
                },
                crate::svg::SvgPathSegment::CurveTo(x1, y1, x2, y2, x, y) => {
                    ClipPathPathCommand::CurveTo {
                        x1: to_pt(x1),
                        y1: to_pt(y1),
                        x2: to_pt(x2),
                        y2: to_pt(y2),
                        x: to_pt(x),
                        y: to_pt(y),
                    }
                }
                crate::svg::SvgPathSegment::Close => ClipPathPathCommand::Close,
            });
        }
        data.push(path_data);
    }
    if commands.is_empty() {
        return None;
    }
    Some(ClipPathShapeSpec::Path(ClipPathPathSpec {
        evenodd,
        fill_rule_explicit: true,
        data: data.join(" "),
        commands,
    }))
}

fn resolve_inline_svg_filter_sources(node: &NodeRef, style: &mut ComputedStyle) {
    let Some(root) = node.ancestors().last() else {
        return;
    };
    style.paint_filter = style
        .paint_filter
        .take()
        .and_then(|filter| resolve_inline_svg_filter(&root, filter));
    style.backdrop_filter = style
        .backdrop_filter
        .take()
        .and_then(|filter| resolve_inline_svg_filter(&root, filter));
}

fn resolve_inline_svg_filter(
    root: &NodeRef,
    mut filter: PaintFilterSpec,
) -> Option<PaintFilterSpec> {
    for operation in &mut filter.operations {
        let PaintFilterOperation::Url(source) = operation else {
            continue;
        };
        let program = compile_inline_svg_filter(root, source)?;
        *operation = PaintFilterOperation::Svg(program);
    }
    Some(filter)
}

fn compile_inline_svg_filter(root: &NodeRef, source: &str) -> Option<SvgFilterProgram> {
    let id = source.trim().strip_prefix('#')?;
    if id.is_empty() {
        return None;
    }
    let selected = root.select_first(&format!("#{id}")).ok()?;
    let filter_node = selected.as_node();
    let element = filter_node.as_element()?;
    if !element.name.local.as_ref().eq_ignore_ascii_case("filter") {
        return None;
    }

    let region = SvgFilterRegion {
        x: svg_bbox_fraction(node_attribute(filter_node, "x"), -0.1)?,
        y: svg_bbox_fraction(node_attribute(filter_node, "y"), -0.1)?,
        width: svg_bbox_fraction(node_attribute(filter_node, "width"), 1.2)?.max(0.0),
        height: svg_bbox_fraction(node_attribute(filter_node, "height"), 1.2)?.max(0.0),
    };
    let linear_rgb = !node_attribute(filter_node, "color-interpolation-filters")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("srgb"));
    let mut nodes = Vec::new();
    for child in filter_node
        .children()
        .filter(|node| node.as_element().is_some())
    {
        let primitive = compile_svg_filter_primitive(&child)?;
        nodes.push(SvgFilterNode {
            primitive,
            result: node_attribute(&child, "result"),
        });
    }
    (!nodes.is_empty()).then_some(SvgFilterProgram {
        nodes,
        region,
        linear_rgb,
    })
}

fn svg_filter_input(node: &NodeRef, attribute: &str) -> SvgFilterInput {
    match node_attribute(node, attribute).as_deref().map(str::trim) {
        Some(value) if value.eq_ignore_ascii_case("SourceGraphic") => SvgFilterInput::SourceGraphic,
        Some(value) if value.eq_ignore_ascii_case("SourceAlpha") => SvgFilterInput::SourceAlpha,
        Some(value) if !value.is_empty() => SvgFilterInput::Named(value.to_string()),
        _ => SvgFilterInput::Previous,
    }
}

fn svg_number_list(raw: Option<String>) -> Option<Vec<f32>> {
    raw?.split(|ch: char| ch.is_ascii_whitespace() || ch == ',')
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<f32>().ok().filter(|value| value.is_finite()))
        .collect()
}

fn svg_number(raw: Option<String>, default: f32) -> Option<f32> {
    let Some(raw) = raw else {
        return Some(default);
    };
    raw.trim()
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
}

fn svg_filter_pt(raw: Option<String>, default: f32) -> Option<Pt> {
    svg_number(raw, default).map(|value| Pt::from_f32(value * (72.0 / 96.0)))
}

fn svg_filter_color(node: &NodeRef, name: &str, default: Color) -> (Color, f32) {
    node_attribute(node, name)
        .and_then(|value| crate::style::parse_color_string(&value))
        .unwrap_or((default, 1.0))
}

fn svg_saturate_matrix(amount: f32) -> [f32; 20] {
    let amount = amount.max(0.0);
    [
        0.213 + 0.787 * amount,
        0.715 - 0.715 * amount,
        0.072 - 0.072 * amount,
        0.0,
        0.0,
        0.213 - 0.213 * amount,
        0.715 + 0.285 * amount,
        0.072 - 0.072 * amount,
        0.0,
        0.0,
        0.213 - 0.213 * amount,
        0.715 - 0.715 * amount,
        0.072 + 0.928 * amount,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    ]
}

fn svg_luminance_to_alpha_matrix() -> [f32; 20] {
    [
        0.0, 0.0, 0.0, 0.0, 0.0, // R
        0.0, 0.0, 0.0, 0.0, 0.0, // G
        0.0, 0.0, 0.0, 0.0, 0.0, // B
        0.2125, 0.7154, 0.0721, 0.0, 0.0, // A
    ]
}

fn compile_svg_component_transfer_function(node: &NodeRef) -> SvgComponentTransferFunction {
    match node_attribute(node, "type")
        .unwrap_or_else(|| "identity".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "table" => SvgComponentTransferFunction::Table(
            svg_number_list(node_attribute(node, "tableValues")).unwrap_or_default(),
        ),
        "discrete" => SvgComponentTransferFunction::Discrete(
            svg_number_list(node_attribute(node, "tableValues")).unwrap_or_default(),
        ),
        "linear" => SvgComponentTransferFunction::Linear {
            slope: svg_number(node_attribute(node, "slope"), 1.0).unwrap_or(1.0),
            intercept: svg_number(node_attribute(node, "intercept"), 0.0).unwrap_or(0.0),
        },
        "gamma" => SvgComponentTransferFunction::Gamma {
            amplitude: svg_number(node_attribute(node, "amplitude"), 1.0).unwrap_or(1.0),
            exponent: svg_number(node_attribute(node, "exponent"), 1.0).unwrap_or(1.0),
            offset: svg_number(node_attribute(node, "offset"), 0.0).unwrap_or(0.0),
        },
        _ => SvgComponentTransferFunction::Identity,
    }
}

fn compile_svg_filter_primitive(node: &NodeRef) -> Option<SvgFilterPrimitive> {
    let tag = node
        .as_element()?
        .name
        .local
        .to_string()
        .to_ascii_lowercase();
    let input = || svg_filter_input(node, "in");
    match tag.as_str() {
        "fegaussianblur" => {
            let values = svg_number_list(node_attribute(node, "stdDeviation"))?;
            let x = *values.first().unwrap_or(&0.0);
            let y = *values.get(1).unwrap_or(&x);
            Some(SvgFilterPrimitive::GaussianBlur {
                input: input(),
                std_deviation_x: Pt::from_f32(x.max(0.0) * 0.75),
                std_deviation_y: Pt::from_f32(y.max(0.0) * 0.75),
            })
        }
        "feoffset" => Some(SvgFilterPrimitive::Offset {
            input: input(),
            dx: svg_filter_pt(node_attribute(node, "dx"), 0.0)?,
            dy: svg_filter_pt(node_attribute(node, "dy"), 0.0)?,
        }),
        "fecolormatrix" => {
            let matrix = match node_attribute(node, "type")
                .unwrap_or_else(|| "matrix".to_string())
                .to_ascii_lowercase()
                .as_str()
            {
                "saturate" => {
                    svg_saturate_matrix(svg_number(node_attribute(node, "values"), 1.0)?.max(0.0))
                }
                "luminancetoalpha" => svg_luminance_to_alpha_matrix(),
                "matrix" => svg_number_list(node_attribute(node, "values"))?
                    .try_into()
                    .ok()?,
                _ => return None,
            };
            Some(SvgFilterPrimitive::ColorMatrix {
                input: input(),
                matrix,
            })
        }
        "fecomponenttransfer" => {
            let mut functions = [
                SvgComponentTransferFunction::Identity,
                SvgComponentTransferFunction::Identity,
                SvgComponentTransferFunction::Identity,
                SvgComponentTransferFunction::Identity,
            ];
            for child in node.children().filter(|node| node.as_element().is_some()) {
                let channel = child
                    .as_element()?
                    .name
                    .local
                    .to_string()
                    .to_ascii_lowercase();
                let index = match channel.as_str() {
                    "fefuncr" => 0,
                    "fefuncg" => 1,
                    "fefuncb" => 2,
                    "fefunca" => 3,
                    _ => continue,
                };
                functions[index] = compile_svg_component_transfer_function(&child);
            }
            Some(SvgFilterPrimitive::ComponentTransfer {
                input: input(),
                functions,
            })
        }
        "feflood" => {
            let (color, color_opacity) = svg_filter_color(node, "flood-color", Color::BLACK);
            let opacity = svg_number(node_attribute(node, "flood-opacity"), 1.0)?.clamp(0.0, 1.0)
                * color_opacity;
            Some(SvgFilterPrimitive::Flood { color, opacity })
        }
        "fecomposite"
            if node_attribute(node, "operator")
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("in")) =>
        {
            Some(SvgFilterPrimitive::CompositeIn {
                input: input(),
                input2: svg_filter_input(node, "in2"),
            })
        }
        "femorphology" => {
            let values = svg_number_list(node_attribute(node, "radius"))?;
            let x = *values.first().unwrap_or(&0.0);
            let y = *values.get(1).unwrap_or(&x);
            Some(SvgFilterPrimitive::Morphology {
                input: input(),
                operator: if node_attribute(node, "operator")
                    .is_some_and(|value| value.trim().eq_ignore_ascii_case("dilate"))
                {
                    SvgMorphologyOperator::Dilate
                } else {
                    SvgMorphologyOperator::Erode
                },
                radius_x: Pt::from_f32(x.max(0.0) * 0.75),
                radius_y: Pt::from_f32(y.max(0.0) * 0.75),
            })
        }
        "fedropshadow" => {
            let (color, color_opacity) = svg_filter_color(node, "flood-color", Color::BLACK);
            let opacity = svg_number(node_attribute(node, "flood-opacity"), 1.0)?.clamp(0.0, 1.0)
                * color_opacity;
            let deviations =
                svg_number_list(node_attribute(node, "stdDeviation")).unwrap_or_else(|| vec![0.0]);
            Some(SvgFilterPrimitive::DropShadow {
                input: input(),
                shadow: FilterDropShadowSpec {
                    offset_x: svg_filter_pt(node_attribute(node, "dx"), 2.0)?,
                    offset_y: svg_filter_pt(node_attribute(node, "dy"), 2.0)?,
                    blur_radius: Pt::from_f32(
                        deviations.iter().copied().sum::<f32>() / deviations.len().max(1) as f32
                            * 0.75,
                    ),
                    color,
                    opacity,
                    color_is_current_color: false,
                },
            })
        }
        "femerge" => Some(SvgFilterPrimitive::Merge {
            inputs: node
                .children()
                .filter(|child| {
                    child.as_element().is_some_and(|element| {
                        element
                            .name
                            .local
                            .as_ref()
                            .eq_ignore_ascii_case("feMergeNode")
                    })
                })
                .map(|child| svg_filter_input(&child, "in"))
                .collect(),
        }),
        "feblend" => Some(SvgFilterPrimitive::Blend {
            input: input(),
            input2: svg_filter_input(node, "in2"),
            mode: match node_attribute(node, "mode")
                .unwrap_or_else(|| "normal".to_string())
                .to_ascii_lowercase()
                .as_str()
            {
                "multiply" => crate::types::MixBlendMode::Multiply,
                "screen" => crate::types::MixBlendMode::Screen,
                "darken" => crate::types::MixBlendMode::Darken,
                "lighten" => crate::types::MixBlendMode::Lighten,
                _ => crate::types::MixBlendMode::Normal,
            },
        }),
        _ => None,
    }
}

fn write_svg_xml(node: &NodeRef, out: &mut String) {
    match node.data() {
        NodeData::Element(el) => {
            let tag = el.name.local.as_ref();
            out.push('<');
            out.push_str(tag);

            let attrs = el.attributes.borrow();
            let mut has_xmlns = false;
            for (k, v) in attrs.map.iter() {
                let key = k.local.as_ref();
                if key.eq_ignore_ascii_case("xmlns") {
                    has_xmlns = true;
                }
                out.push(' ');
                out.push_str(key);
                out.push_str("=\"");
                escape_xml_attr(&v.value, out);
                out.push('"');
            }
            if tag.eq_ignore_ascii_case("svg") && !has_xmlns {
                out.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
            }

            out.push('>');

            for child in node.children() {
                write_svg_xml(&child, out);
            }

            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        }
        NodeData::Text(t) => {
            escape_xml_text(&t.borrow(), out);
        }
        _ => {}
    }
}

fn escape_xml_attr(input: &str, out: &mut String) {
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
}

fn escape_xml_text(input: &str, out: &mut String) {
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

fn text_align_mode_to_flow(align: TextAlignMode, direction: DirectionMode) -> TextAlign {
    match align {
        TextAlignMode::Start => match direction {
            DirectionMode::Rtl => TextAlign::Right,
            DirectionMode::Ltr => TextAlign::Left,
        },
        TextAlignMode::End => match direction {
            DirectionMode::Rtl => TextAlign::Left,
            DirectionMode::Ltr => TextAlign::Right,
        },
        TextAlignMode::MatchParent => match direction {
            DirectionMode::Rtl => TextAlign::Right,
            DirectionMode::Ltr => TextAlign::Left,
        },
        TextAlignMode::Center => TextAlign::Center,
        TextAlignMode::Right => TextAlign::Right,
        TextAlignMode::Left => TextAlign::Left,
        TextAlignMode::Justify | TextAlignMode::JustifyAll => TextAlign::Justify,
    }
}

fn resolve_html_auto_direction(node: &NodeRef, style: &mut ComputedStyle) {
    let is_auto = node
        .as_element()
        .and_then(|element| element.attributes.borrow().get("dir").map(str::to_string))
        .is_some_and(|value| value.eq_ignore_ascii_case("auto"));
    if !is_auto {
        return;
    }

    style.direction = match unicode_bidi::get_base_direction(node.text_contents().as_str()) {
        unicode_bidi::Direction::Rtl => DirectionMode::Rtl,
        unicode_bidi::Direction::Ltr => DirectionMode::Ltr,
        unicode_bidi::Direction::Mixed => style.direction,
    };
}

fn text_align_from_style(style: &ComputedStyle) -> TextAlign {
    text_align_mode_to_flow(style.text_align, style.direction)
}

fn text_align_last_from_style(style: &ComputedStyle) -> Option<TextAlign> {
    let align = match style.text_align_last {
        TextAlignLastMode::Auto => {
            return if matches!(style.text_align, TextAlignMode::Justify) {
                Some(text_align_mode_to_flow(
                    TextAlignMode::Start,
                    style.direction,
                ))
            } else if matches!(style.text_align, TextAlignMode::JustifyAll) {
                Some(TextAlign::Justify)
            } else {
                None
            };
        }
        TextAlignLastMode::Start => TextAlignMode::Start,
        TextAlignLastMode::End => TextAlignMode::End,
        TextAlignLastMode::Left => TextAlignMode::Left,
        TextAlignLastMode::Center => TextAlignMode::Center,
        TextAlignLastMode::Right => TextAlignMode::Right,
        TextAlignLastMode::Justify => TextAlignMode::Justify,
    };
    Some(text_align_mode_to_flow(align, style.direction))
}

fn inject_pseudo_items(
    mut children: Vec<LayoutItem>,
    before_items: &[LayoutItem],
    after_items: &[LayoutItem],
) -> Vec<LayoutItem> {
    if !before_items.is_empty() {
        let mut merged =
            Vec::with_capacity(before_items.len() + children.len() + after_items.len());
        merged.extend(before_items.iter().cloned());
        merged.append(&mut children);
        if !after_items.is_empty() {
            merged.extend(after_items.iter().cloned());
        }
        merged
    } else if !after_items.is_empty() {
        children.extend(after_items.iter().cloned());
        children
    } else {
        children
    }
}

fn starts_with_collapsible_dom_space(node: &NodeRef) -> bool {
    for child in node.children() {
        match child.data() {
            NodeData::Text(text) => {
                let text = text.borrow();
                if text.is_empty() {
                    continue;
                }
                return text
                    .chars()
                    .next()
                    .is_some_and(|ch| matches!(ch, ' ' | '\n' | '\r' | '\t' | '\u{000c}'));
            }
            NodeData::Element(element)
                if matches!(element.name.local.as_ref(), "script" | "style") =>
            {
                continue;
            }
            NodeData::Comment(_) => continue,
            _ => return false,
        }
    }
    false
}

fn ends_with_collapsible_dom_space(node: &NodeRef) -> bool {
    let children = node.children().collect::<Vec<_>>();
    for child in children.iter().rev() {
        match child.data() {
            NodeData::Text(text) => {
                let text = text.borrow();
                if text.is_empty() {
                    continue;
                }
                return text
                    .chars()
                    .next_back()
                    .is_some_and(|ch| matches!(ch, ' ' | '\n' | '\r' | '\t' | '\u{000c}'));
            }
            NodeData::Element(element)
                if matches!(element.name.local.as_ref(), "script" | "style") =>
            {
                continue;
            }
            NodeData::Comment(_) => continue,
            _ => return false,
        }
    }
    false
}

fn collapsible_boundary_space_item(
    style: &ComputedStyle,
    font_registry: Option<Arc<FontRegistry>>,
) -> LayoutItem {
    LayoutItem::Inline {
        flowable: Box::new(CollapsibleSpaceFlowable::new(
            text_style_for_flow_text(style),
            font_registry,
        )),
        valign: anonymous_text_vertical_align(style),
        flex_grow: 0.0,
        flex_shrink: 1.0,
        width_spec: None,
        order: 0,
    }
}

fn inject_transparent_inline_pseudo_items(
    mut children: Vec<LayoutItem>,
    before_items: &[LayoutItem],
    after_items: &[LayoutItem],
    node: &NodeRef,
    style: &ComputedStyle,
    font_registry: Option<Arc<FontRegistry>>,
) -> Vec<LayoutItem> {
    let preserve_before_space = !preserve_whitespace(style.white_space)
        && !before_items.is_empty()
        && starts_with_collapsible_dom_space(node)
        && before_items.last().is_some_and(|item| match item {
            LayoutItem::Inline { flowable, .. } => !flowable.is_collapsible_inline_space(),
            LayoutItem::Block { .. } => false,
        });
    let preserve_after_space = !preserve_whitespace(style.white_space)
        && !after_items.is_empty()
        && ends_with_collapsible_dom_space(node)
        && after_items.first().is_some_and(|item| match item {
            LayoutItem::Inline { flowable, .. } => !flowable.is_collapsible_inline_space(),
            LayoutItem::Block { .. } => false,
        });
    let mut merged = Vec::with_capacity(
        before_items.len()
            + children.len()
            + after_items.len()
            + usize::from(preserve_before_space)
            + usize::from(preserve_after_space),
    );
    merged.extend(before_items.iter().cloned());
    if preserve_before_space {
        merged.push(collapsible_boundary_space_item(
            style,
            font_registry.clone(),
        ));
    }
    merged.append(&mut children);
    if preserve_after_space {
        merged.push(collapsible_boundary_space_item(style, font_registry));
    }
    merged.extend(after_items.iter().cloned());
    merged
}

fn pseudo_items_are_inline(before_items: &[LayoutItem], after_items: &[LayoutItem]) -> bool {
    before_items
        .iter()
        .chain(after_items)
        .all(|item| matches!(item, LayoutItem::Inline { .. }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedGeneratedContentPart {
    Text(String),
    Image(String),
    Leader(String),
}

fn push_resolved_generated_text(parts: &mut Vec<ResolvedGeneratedContentPart>, text: &str) {
    if text.is_empty() {
        return;
    }
    match parts.last_mut() {
        Some(ResolvedGeneratedContentPart::Text(existing)) => existing.push_str(text),
        _ => parts.push(ResolvedGeneratedContentPart::Text(text.to_string())),
    }
}

fn generated_content_parts(
    style: &ComputedStyle,
    counters: &mut CounterState,
) -> Option<Vec<ResolvedGeneratedContentPart>> {
    let Some(parts) = &style.generated_content else {
        return style
            .content
            .as_ref()
            .map(|text| vec![ResolvedGeneratedContentPart::Text(text.clone())]);
    };
    let mut out = Vec::new();
    for part in parts {
        match part {
            GeneratedContentPart::Text(text) => push_resolved_generated_text(&mut out, text),
            GeneratedContentPart::Image(source) => {
                out.push(ResolvedGeneratedContentPart::Image(source.clone()));
            }
            GeneratedContentPart::Leader(pattern) => {
                out.push(ResolvedGeneratedContentPart::Leader(pattern.clone()));
            }
            GeneratedContentPart::Counter(counter) => {
                push_resolved_generated_text(
                    &mut out,
                    &generated_counter_text(counter, counters.get(&counter.name)),
                );
            }
            GeneratedContentPart::Counters(counter) => {
                push_resolved_generated_text(
                    &mut out,
                    &generated_counters_text(counter, &counters.get_all(&counter.name)),
                );
            }
            GeneratedContentPart::TargetText(target) => {
                if let Some(text) = counters.target_text(target) {
                    push_resolved_generated_text(&mut out, text);
                }
            }
            GeneratedContentPart::TargetCounter(counter) => {
                let value = if counter.name.eq_ignore_ascii_case("page") {
                    counters
                        .target_page(&counter.target)
                        .and_then(|page| i32::try_from(page).ok())
                        .unwrap_or(0)
                } else {
                    0
                };
                push_resolved_generated_text(
                    &mut out,
                    &generated_counter_text(
                        &GeneratedCounterContent {
                            name: counter.name.clone(),
                            style: counter.style.clone(),
                        },
                        value,
                    ),
                );
            }
            GeneratedContentPart::OpenQuote => {
                if let Some(quote) = counters.open_quote(&style.quotes, true) {
                    push_resolved_generated_text(&mut out, &quote);
                }
            }
            GeneratedContentPart::CloseQuote => {
                if let Some(quote) = counters.close_quote(&style.quotes, true) {
                    push_resolved_generated_text(&mut out, &quote);
                }
            }
            GeneratedContentPart::NoOpenQuote => {
                counters.open_quote(&style.quotes, false);
            }
            GeneratedContentPart::NoCloseQuote => {
                counters.close_quote(&style.quotes, false);
            }
        }
    }
    Some(out)
}

fn generated_content_text(style: &ComputedStyle, counters: &mut CounterState) -> Option<String> {
    generated_content_parts(style, counters).map(|parts| {
        let mut text = String::new();
        for part in parts {
            if let ResolvedGeneratedContentPart::Text(value) = part {
                text.push_str(&value);
            }
        }
        text
    })
}

fn pseudo_inline_text_paint_phase_y(pseudo: crate::style::PseudoTarget, style: &TextStyle) -> Pt {
    // Chromium's generated synthetic-bold glyph programs land slightly ahead
    // of ordinary DOM text on the CSS raster grid. Regular registered-font
    // counter text does not use that Type 3 phase. The closing pseudo has the
    // later inline cursor phase, so it needs the larger correction. Keep these
    // in fixed point; they are paint offsets and never alter line measurement.
    let synthetic_bold = style.font_synthesis_weight
        && !style.font_face_satisfies_weight
        && style.font_weight >= 600;
    if !synthetic_bold {
        return Pt::ZERO;
    }
    match pseudo {
        crate::style::PseudoTarget::Before => -Pt::from_milli_i64(75),
        crate::style::PseudoTarget::After => -Pt::from_milli_i64(225),
        _ => Pt::ZERO,
    }
}

fn phase_inline_pseudo_text(
    paragraph: Paragraph,
    pseudo: crate::style::PseudoTarget,
    style: &TextStyle,
) -> Paragraph {
    let phase_y = pseudo_inline_text_paint_phase_y(pseudo, style);
    paragraph.with_paint_offset_y(phase_y)
}

fn generated_content_image_flowable(
    source: &str,
    style: &ComputedStyle,
    asset_bundle: Option<&AssetBundle>,
    svg_form: bool,
    svg_raster_fallback: bool,
) -> Option<Box<dyn Flowable>> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }
    let svg_xml = load_svg_xml_from_image_source(asset_bundle, source);
    let intrinsic_size = raster_image_intrinsic_dimensions(asset_bundle, source)
        .map(|(width, height)| {
            (
                Pt::from_f32(width as f32 * 0.75),
                Pt::from_f32(height as f32 * 0.75),
            )
        })
        .or_else(|| svg_xml.as_deref().and_then(svg_image_intrinsic_dimensions));
    let fallback = style.font_size.max(Pt::from_f32(1.0));
    let (width, height) = intrinsic_size.unwrap_or((fallback, fallback));

    if let Some(xml) = svg_xml {
        if svg_raster_fallback && crate::svg::svg_needs_raster_fallback(&xml) {
            if let Some(data_uri) = crate::svg::rasterize_svg_to_data_uri(&xml, width, height) {
                let image = ImageFlowable::new_pt(width, height, data_uri)
                    .with_intrinsic_size(intrinsic_size)
                    .with_object_fit(style.object_fit)
                    .with_object_position(style.object_position)
                    .with_image_rendering(style.image_rendering)
                    .with_font_metrics(style.font_size, style.root_font_size)
                    .with_pagination(style.pagination)
                    .with_visible(style.visibility.paints())
                    .with_mix_blend_mode(style.mix_blend_mode)
                    .with_paint_filter(style.paint_filter.clone())
                    .with_css_pixel_paint_origin_snap(true)
                    .with_tag_role("Artifact");
                return Some(Box::new(image));
            }
        }
        let svg = SvgFlowable::new_pt(width, height, xml)
            .with_intrinsic_size(intrinsic_size)
            .with_font_metrics(style.font_size, style.root_font_size)
            .with_pagination(style.pagination)
            .with_form_enabled(svg_form)
            .with_visible(style.visibility.paints())
            .with_mix_blend_mode(style.mix_blend_mode)
            .with_tag_role("Artifact");
        return Some(Box::new(svg));
    }

    let image_source =
        renderable_image_source(asset_bundle, source).unwrap_or_else(|| source.to_string());
    let image = ImageFlowable::new_pt(width, height, image_source)
        .with_intrinsic_size(intrinsic_size)
        .with_object_fit(style.object_fit)
        .with_object_position(style.object_position)
        .with_image_rendering(style.image_rendering)
        .with_font_metrics(style.font_size, style.root_font_size)
        .with_pagination(style.pagination)
        .with_visible(style.visibility.paints())
        .with_mix_blend_mode(style.mix_blend_mode)
        .with_paint_filter(style.paint_filter.clone())
        .with_css_pixel_paint_origin_snap(true)
        .with_tag_role("Artifact");
    Some(Box::new(image))
}

fn generated_content_part_text_items(
    text: &str,
    style: &ComputedStyle,
    font_registry: Option<Arc<FontRegistry>>,
    report: Option<&mut GlyphCoverageReport>,
    pseudo: crate::style::PseudoTarget,
) -> Vec<LayoutItem> {
    let text = apply_text_transform(text, style.text_transform);
    let text = normalize_text(&text, style.white_space, false);
    if text.is_empty() {
        return Vec::new();
    }
    let text_style = text_style_for_flow_text(style);
    report_missing_glyphs(report, font_registry.as_deref(), &text_style, &text);
    let valign = vertical_align_from_style(style);
    let make_item = |flowable: Box<dyn Flowable>| LayoutItem::Inline {
        flowable,
        valign,
        flex_grow: style.flex_grow,
        flex_shrink: style.flex_shrink,
        width_spec: flex_item_basis(style),
        order: 0,
    };

    if preserve_whitespace(style.white_space) {
        let paragraph = Paragraph::new(text)
            .with_style(text_style.clone())
            .with_align(text_align_from_style(style))
            .with_last_align(text_align_last_from_style(style))
            .with_whitespace(true, no_wrap(style))
            .with_break_spaces(matches!(style.white_space, WhiteSpaceMode::BreakSpaces))
            .with_pagination(style.pagination)
            .with_font_registry(font_registry);
        return vec![make_item(Box::new(phase_inline_pseudo_text(
            paragraph,
            pseudo,
            &text_style,
        )))];
    }

    let leading_space = text.starts_with(' ');
    let trailing_space = text.ends_with(' ');
    let core = text.trim_matches(' ');
    let mut items = Vec::with_capacity(3);
    if leading_space || (core.is_empty() && trailing_space) {
        items.push(make_item(Box::new(CollapsibleSpaceFlowable::new(
            text_style.clone(),
            font_registry.clone(),
        ))));
    }
    if !core.is_empty() {
        let paragraph = Paragraph::new(core)
            .with_style(text_style.clone())
            .with_align(text_align_from_style(style))
            .with_last_align(text_align_last_from_style(style))
            .with_whitespace(false, no_wrap(style))
            .with_pagination(style.pagination)
            .with_font_registry(font_registry.clone());
        items.push(make_item(Box::new(phase_inline_pseudo_text(
            paragraph,
            pseudo,
            &text_style,
        ))));
    }
    if trailing_space && !core.is_empty() {
        items.push(make_item(Box::new(CollapsibleSpaceFlowable::new(
            text_style,
            font_registry,
        ))));
    }
    items
}

fn pseudo_image_content_items(
    parts: &[ResolvedGeneratedContentPart],
    style: &ComputedStyle,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<&AssetBundle>,
    mut report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    pseudo: crate::style::PseudoTarget,
) -> Vec<LayoutItem> {
    let is_inline = matches!(
        style.display,
        DisplayMode::Inline
            | DisplayMode::InlineBlock
            | DisplayMode::InlineTable
            | DisplayMode::InlineFlex
            | DisplayMode::InlineGrid
    );
    let valign = vertical_align_from_style(style);
    let mut children = Vec::new();
    for part in parts {
        match part {
            ResolvedGeneratedContentPart::Text(text) => {
                children.extend(generated_content_part_text_items(
                    text,
                    style,
                    font_registry.clone(),
                    report.as_deref_mut(),
                    pseudo,
                ));
            }
            ResolvedGeneratedContentPart::Image(source) => {
                if let Some(flowable) = generated_content_image_flowable(
                    source,
                    style,
                    asset_bundle,
                    svg_form,
                    svg_raster_fallback,
                ) {
                    children.push(LayoutItem::Inline {
                        flowable,
                        valign,
                        flex_grow: style.flex_grow,
                        flex_shrink: style.flex_shrink,
                        width_spec: None,
                        order: 0,
                    });
                }
            }
            ResolvedGeneratedContentPart::Leader(pattern) => {
                let text_style = text_style_for_flow_text(style);
                report_missing_glyphs(
                    report.as_deref_mut(),
                    font_registry.as_deref(),
                    &text_style,
                    pattern,
                );
                let leader =
                    LeaderFlowable::new(pattern, text_style.clone(), font_registry.clone())
                        .with_pagination(style.pagination)
                        .with_paint_offset_y(pseudo_inline_text_paint_phase_y(pseudo, &text_style));
                children.push(LayoutItem::Inline {
                    flowable: Box::new(leader),
                    valign,
                    flex_grow: style.flex_grow,
                    flex_shrink: style.flex_shrink,
                    width_spec: None,
                    order: 0,
                });
            }
        }
    }
    if children.is_empty() {
        return Vec::new();
    }

    let has_inline_box_paint = style.margin != EdgeSizes::zero()
        || style.padding != EdgeSizes::zero()
        || style.border_width != EdgeSizes::zero()
        || style.border_image.source.is_some()
        || style.background_color.is_some()
        || style.background_paint.is_some()
        || !style.background_paints.is_empty()
        || style.border_radius != BorderRadiiSpec::zero()
        || style.outline_visible
        || style.box_shadow.is_some()
        || style.paint_filter.is_some()
        || style.opacity < 1.0 - 1.0e-6
        || !style.transform.is_empty();
    if matches!(style.display, DisplayMode::Inline) && !has_inline_box_paint {
        // Width and height do not apply to a non-atomic inline pseudo box. The
        // generated replaced child therefore keeps its intrinsic dimensions.
        return children;
    }

    let mut box_style = style.clone();
    if matches!(style.display, DisplayMode::Inline) {
        box_style.width = LengthSpec::Auto;
        box_style.height = LengthSpec::Auto;
        box_style.min_width = LengthSpec::Auto;
        box_style.max_width = LengthSpec::Auto;
        box_style.min_height = LengthSpec::Auto;
        box_style.max_height = LengthSpec::Auto;
    }
    let Some(flowable) = container_flowable_with_role(children, &box_style, None) else {
        return Vec::new();
    };
    if is_inline {
        vec![LayoutItem::Inline {
            flowable,
            valign,
            flex_grow: style.flex_grow,
            flex_shrink: style.flex_shrink,
            width_spec: flex_item_basis(style),
            order: 0,
        }]
    } else {
        vec![LayoutItem::Block {
            flowable,
            flex_grow: style.flex_grow,
            flex_shrink: style.flex_shrink,
            width_spec: flex_item_basis(style),
            order: 0,
        }]
    }
}

fn pseudo_content_items(
    style: &ComputedStyle,
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<&AssetBundle>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    pseudo: crate::style::PseudoTarget,
) -> Vec<LayoutItem> {
    if !style_can_mutate_counters(style) {
        return Vec::new();
    }
    apply_style_counters(style, counters);
    let Some(parts) = generated_content_parts(style, counters) else {
        return Vec::new();
    };
    if parts.iter().any(|part| {
        matches!(
            part,
            ResolvedGeneratedContentPart::Image(_) | ResolvedGeneratedContentPart::Leader(_)
        )
    }) {
        return pseudo_image_content_items(
            &parts,
            style,
            font_registry,
            asset_bundle,
            report,
            svg_form,
            svg_raster_fallback,
            pseudo,
        );
    }
    let content = parts
        .into_iter()
        .filter_map(|part| match part {
            ResolvedGeneratedContentPart::Text(text) => Some(text),
            ResolvedGeneratedContentPart::Image(_) | ResolvedGeneratedContentPart::Leader(_) => {
                None
            }
        })
        .collect::<String>();
    let text = apply_text_transform(&content, style.text_transform);
    let text = normalize_text(&text, style.white_space, false);
    let content_is_empty = text.is_empty();
    let text_style = text_style_for_flow_text(style);
    let has_styled_box = !matches!(style.width, LengthSpec::Auto)
        || !matches!(style.height, LengthSpec::Auto)
        || !matches!(style.min_width, LengthSpec::Auto)
        || !matches!(style.max_width, LengthSpec::Auto)
        || !matches!(style.min_height, LengthSpec::Auto)
        || !matches!(style.max_height, LengthSpec::Auto)
        || style.margin != EdgeSizes::zero()
        || style.padding != EdgeSizes::zero()
        || style.border_width != EdgeSizes::zero()
        || style.border_image.source.is_some()
        || style.background_color.is_some()
        || style.background_paint.is_some()
        || !style.background_paints.is_empty()
        || style.border_radius != BorderRadiiSpec::zero()
        || style.outline_visible
        || style.box_shadow.is_some()
        || style.paint_filter.is_some()
        || style.opacity < 1.0 - 1.0e-6
        || !style.transform.is_empty();
    if content.is_empty() && !has_styled_box {
        return Vec::new();
    }
    let is_inline = matches!(
        style.display,
        DisplayMode::Inline
            | DisplayMode::InlineBlock
            | DisplayMode::InlineTable
            | DisplayMode::InlineFlex
            | DisplayMode::InlineGrid
    );
    if is_inline && !has_styled_box && !preserve_whitespace(style.white_space) {
        let leading_space = text.starts_with(' ');
        let trailing_space = text.ends_with(' ');
        let core = text.trim_matches(' ');
        report_missing_glyphs(report, font_registry.as_deref(), &text_style, core);
        let valign = vertical_align_from_style(style);
        let make_item = |flowable: Box<dyn Flowable>| LayoutItem::Inline {
            flowable,
            valign,
            flex_grow: style.flex_grow,
            flex_shrink: style.flex_shrink,
            width_spec: flex_item_basis(style),
            order: 0,
        };
        let mut items = Vec::with_capacity(3);
        if leading_space || (core.is_empty() && trailing_space) {
            items.push(make_item(Box::new(CollapsibleSpaceFlowable::new(
                text_style.clone(),
                font_registry.clone(),
            ))));
        }
        if !core.is_empty() {
            let paragraph = Paragraph::new(core)
                .with_style(text_style.clone())
                .with_align(text_align_from_style(style))
                .with_last_align(text_align_last_from_style(style))
                .with_whitespace(false, no_wrap(style))
                .with_pagination(style.pagination)
                .with_font_registry(font_registry.clone());
            items.push(make_item(Box::new(phase_inline_pseudo_text(
                paragraph,
                pseudo,
                &text_style,
            ))));
        }
        if trailing_space && !core.is_empty() {
            items.push(make_item(Box::new(CollapsibleSpaceFlowable::new(
                text_style,
                font_registry,
            ))));
        }
        return items;
    }

    report_missing_glyphs(report, font_registry.as_deref(), &text_style, &text);
    let inline_paint_phase_y = pseudo_inline_text_paint_phase_y(pseudo, &text_style);
    let paragraph = Paragraph::new(text)
        .with_style(text_style)
        .with_align(text_align_from_style(style))
        .with_last_align(text_align_last_from_style(style))
        .with_whitespace(preserve_whitespace(style.white_space), no_wrap(style))
        .with_break_spaces(matches!(style.white_space, WhiteSpaceMode::BreakSpaces))
        .with_pagination(style.pagination)
        .with_font_registry(font_registry);
    let mut flowable = if has_styled_box {
        // Generated text inside an inline CSS box follows browser line-box
        // phase: its centered glyph origin sits slightly before the raw PDF
        // text origin. A block pseudo starts an ordinary block line box and
        // must keep the unshifted content origin.
        let children = if content_is_empty {
            // An empty generated inline-block has no line box. Its synthesized
            // inline baseline therefore comes from the bottom margin edge;
            // retaining an empty Paragraph invents a font baseline and pulls
            // the decorative box above adjacent text.
            Vec::new()
        } else {
            let paragraph: Box<dyn Flowable> = if is_inline {
                Box::new(
                    RelativePositionedFlowable::new_pt(
                        Box::new(paragraph) as Box<dyn Flowable>,
                        LengthSpec::Absolute(-Pt::from_f32(0.25)),
                        LengthSpec::Absolute(-Pt::from_f32(0.5)),
                        LengthSpec::Auto,
                        LengthSpec::Auto,
                        style.font_size,
                        style.root_font_size,
                    )
                    .with_pagination(style.pagination),
                )
            } else {
                Box::new(paragraph)
            };
            vec![LayoutItem::Block {
                flowable: paragraph,
                flex_grow: 0.0,
                flex_shrink: 1.0,
                width_spec: None,
                order: 0,
            }]
        };
        container_flowable_with_role(children, style, None)
            .expect("generated content with a styled box must produce a flowable")
    } else {
        if is_inline {
            Box::new(paragraph.with_paint_offset_y(inline_paint_phase_y)) as Box<dyn Flowable>
        } else {
            Box::new(paragraph) as Box<dyn Flowable>
        }
    };
    if style_is_css_list_item(style) {
        if let Some((label, outside_gap)) = native_list_bullet_flowable(style, false) {
            let marker_inside = matches!(style.list_style_position, ListStylePositionMode::Inside);
            let gap = if marker_inside {
                let label_width = label.intrinsic_width().unwrap_or(Pt::ZERO);
                (style.font_size.mul_ratio(4, 3) - label_width).max(Pt::ZERO)
            } else {
                outside_gap
            };
            let list_item = ListItemFlowable::new_with_label(label, flowable, gap)
                .with_marker_inside(marker_inside)
                .with_pagination(style.pagination);
            flowable = Box::new(CssLineBoxFlowable::new(Box::new(list_item))) as Box<dyn Flowable>;
        }
    }
    if is_inline {
        let valign = vertical_align_from_style(style);
        vec![LayoutItem::Inline {
            flowable,
            valign,
            flex_grow: style.flex_grow,
            flex_shrink: style.flex_shrink,
            width_spec: flex_item_basis(style),
            order: 0,
        }]
    } else {
        vec![LayoutItem::Block {
            flowable,
            flex_grow: style.flex_grow,
            flex_shrink: style.flex_shrink,
            width_spec: flex_item_basis(style),
            order: 0,
        }]
    }
}

fn pseudo_items_for(
    resolver: &StyleResolver,
    info: &ElementInfo,
    style: &ComputedStyle,
    ancestors: &[ElementInfo],
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<&AssetBundle>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    pseudo: crate::style::PseudoTarget,
) -> Vec<LayoutItem> {
    let Some(pseudo_style) = resolver.compute_pseudo_style(info, style, ancestors, pseudo) else {
        return Vec::new();
    };
    let items = pseudo_content_items(
        &pseudo_style,
        counters,
        font_registry,
        asset_bundle,
        report,
        svg_form,
        svg_raster_fallback,
        pseudo,
    );
    // Pseudo-elements generate real CSS boxes and participate in positioned
    // layout exactly like element-backed boxes. Their content used to be
    // injected before the ordinary element wrapper stage, which discarded
    // both the inset geometry and stacking level (`::before` therefore painted
    // at its parent's content origin and at z-index zero). Blockify and wrap
    // positioned pseudo boxes before they enter the parent's child stream.
    if matches!(
        pseudo_style.position,
        PositionMode::Absolute | PositionMode::Fixed
    ) {
        wrap_absolute(items, &pseudo_style)
    } else if matches!(pseudo_style.position, PositionMode::Relative) {
        wrap_relative(items, &pseudo_style)
    } else {
        items
    }
}

fn pseudo_text_for(
    resolver: &StyleResolver,
    info: &ElementInfo,
    style: &ComputedStyle,
    ancestors: &[ElementInfo],
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    report: Option<&mut GlyphCoverageReport>,
    pseudo: crate::style::PseudoTarget,
) -> String {
    let Some(pseudo_style) = resolver.compute_pseudo_style(info, style, ancestors, pseudo) else {
        return String::new();
    };
    if !style_can_mutate_counters(&pseudo_style) {
        return String::new();
    }
    apply_style_counters(&pseudo_style, counters);
    let Some(content) = generated_content_text(&pseudo_style, counters) else {
        return String::new();
    };
    let text = apply_text_transform(&content, pseudo_style.text_transform);
    let text_style = text_style_for_flow_text(&pseudo_style);
    report_missing_glyphs(report, font_registry.as_deref(), &text_style, &text);
    text
}

fn marker_presentation(
    resolver: &StyleResolver,
    info: &ElementInfo,
    style: &ComputedStyle,
    ancestors: &[ElementInfo],
    counters: &mut CounterState,
) -> (Option<Option<String>>, Option<ComputedStyle>) {
    let Some(marker_style) = resolver.compute_marker_style(info, style, ancestors) else {
        return (None, None);
    };
    let content_override = marker_style.marker_content_overridden.then(|| {
        generated_content_text(&marker_style, counters)
            .map(|content| apply_text_transform(&content, marker_style.text_transform))
    });
    (content_override, Some(marker_style))
}

fn list_marker_image_flowable(
    style: &ComputedStyle,
    asset_bundle: Option<&AssetBundle>,
    svg_form: bool,
    svg_raster_fallback: bool,
) -> Option<Box<dyn Flowable>> {
    let source = style.list_style_image.as_deref()?.trim();
    if source.is_empty() {
        return None;
    }
    let fallback_size = style.font_size.max(Pt::from_f32(1.0));
    let intrinsic_size = raster_image_intrinsic_dimensions(asset_bundle, source)
        .map(|(w, h)| (Pt::from_f32(w as f32 * 0.75), Pt::from_f32(h as f32 * 0.75)));
    let (marker_width, marker_height) = intrinsic_size.unwrap_or((fallback_size, fallback_size));
    if let Some(paint) = style.list_style_image_paint.as_ref() {
        let marker = BackgroundPaintFlowable::new_pt(marker_width, marker_height, paint.clone())
            .with_pagination(style.pagination)
            .with_visible(style.visibility.paints())
            .with_mix_blend_mode(style.mix_blend_mode)
            .with_tag_role("Lbl");
        return Some(Box::new(marker) as Box<dyn Flowable>);
    }
    if let Some(xml) = load_svg_xml_from_image_source(asset_bundle, source) {
        if svg_raster_fallback && crate::svg::svg_needs_raster_fallback(&xml) {
            if let Some(data_uri) =
                crate::svg::rasterize_svg_to_data_uri(&xml, marker_width, marker_height)
            {
                let image = ImageFlowable::new_pt(marker_width, marker_height, data_uri)
                    .with_intrinsic_size(intrinsic_size)
                    .with_image_rendering(style.image_rendering)
                    .with_font_metrics(style.font_size, style.root_font_size)
                    .with_pagination(style.pagination)
                    .with_visible(style.visibility.paints())
                    .with_mix_blend_mode(style.mix_blend_mode)
                    .with_paint_filter(style.paint_filter.clone())
                    .with_tag_role("Lbl");
                return Some(Box::new(image) as Box<dyn Flowable>);
            }
        }
        let svg = SvgFlowable::new_pt(marker_width, marker_height, xml)
            .with_pagination(style.pagination)
            .with_form_enabled(svg_form)
            .with_visible(style.visibility.paints())
            .with_mix_blend_mode(style.mix_blend_mode)
            .with_tag_role("Lbl");
        return Some(Box::new(svg) as Box<dyn Flowable>);
    }

    let image_source =
        renderable_image_source(asset_bundle, source).unwrap_or_else(|| source.to_string());
    let image = ImageFlowable::new_pt(marker_width, marker_height, image_source)
        .with_intrinsic_size(intrinsic_size)
        .with_image_rendering(style.image_rendering)
        .with_font_metrics(style.font_size, style.root_font_size)
        .with_pagination(style.pagination)
        .with_visible(style.visibility.paints())
        .with_mix_blend_mode(style.mix_blend_mode)
        .with_paint_filter(style.paint_filter.clone())
        .with_tag_role("Lbl");
    Some(Box::new(image) as Box<dyn Flowable>)
}

fn native_list_bullet_flowable(
    style: &ComputedStyle,
    ordered: bool,
) -> Option<(Box<dyn Flowable>, Pt)> {
    let kind = match style.list_style_type {
        ListStyleTypeMode::Disc => ListBulletKind::Disc,
        ListStyleTypeMode::Auto if !ordered => ListBulletKind::Disc,
        ListStyleTypeMode::Circle => ListBulletKind::Circle,
        ListStyleTypeMode::Square => ListBulletKind::Square,
        _ => return None,
    };
    let marker_size = match kind {
        ListBulletKind::Circle => style.font_size.mul_ratio(4, 11),
        ListBulletKind::Disc | ListBulletKind::Square => style.font_size.mul_ratio(7, 22),
    };
    // CSS native bullets are centered three quarters of an em before the
    // principal list-item content edge.
    let outside_gap = (style.font_size.mul_ratio(3, 4) - marker_size.mul_ratio(1, 2)).max(Pt::ZERO);
    let marker = ListBulletFlowable::new_pt(
        kind,
        style.font_size,
        text_style_for_flow_text(style).line_height,
        style.color,
    )
    .with_visible(style.visibility.paints())
    .with_pagination(style.pagination)
    .with_tag_role("Lbl");
    Some((Box::new(marker) as Box<dyn Flowable>, outside_gap))
}

fn cjk_decimal_marker_flowable(style: &ComputedStyle, index: usize) -> Option<Box<dyn Flowable>> {
    if !matches!(style.list_style_type, ListStyleTypeMode::CjkDecimal) || !(1..=3).contains(&index)
    {
        return None;
    }
    Some(Box::new(
        CjkDecimalMarkerFlowable::new_pt(
            index,
            style.font_size,
            text_style_for_flow_text(style).line_height,
            style.color,
        )
        .with_visible(style.visibility.paints())
        .with_pagination(style.pagination)
        .with_tag_role("Lbl"),
    ) as Box<dyn Flowable>)
}

fn marker_text_style(marker_style: &ComputedStyle, item_style: &ComputedStyle) -> TextStyle {
    let mut text_style = text_style_for_flow_text(marker_style);
    if marker_style.font_size > item_style.font_size {
        // An enlarged marker contributes its actual font extents to the first
        // line box even when it inherits a smaller absolute line-height.
        text_style.line_height_is_auto = true;
    }
    text_style
}

fn inline_children_only(
    node: &NodeRef,
    resolver: &StyleResolver,
    parent_style: &ComputedStyle,
    ancestors: &[ElementInfo],
) -> bool {
    for child in node.children() {
        let Some(element) = child.as_element() else {
            continue;
        };
        let info = element_info(&child, resolver.has_sibling_selectors());
        let inline_style = element
            .attributes
            .borrow()
            .get("style")
            .map(|s| s.to_string());
        let child_style =
            resolver.compute_style(&info, parent_style, inline_style.as_deref(), ancestors);
        // This semantic child must be compiled as its own zero-size tag at the
        // exact DOM position. Flattening the parent into one text run would
        // paint the alternative text and discard its `/ActualText` carrier.
        if info
            .attrs
            .get("data-fb-a11y-only")
            .is_some_and(|value| data_attribute_value_is_truthy(value))
        {
            return false;
        }
        // display:none generates no box and contributes no text, so it cannot
        // invalidate an otherwise flattenable inline formatting context.
        if matches!(child_style.display, DisplayMode::None) {
            continue;
        }
        // This flattening fast-path is text-centric. Rendered replaced/media
        // elements must keep structural flowables or their content can vanish.
        let tag = element.name.local.as_ref().to_ascii_lowercase();
        if tag == "br" {
            continue;
        }
        if matches!(
            tag.as_str(),
            "img"
                | "svg"
                | "hr"
                | "canvas"
                | "video"
                | "audio"
                | "iframe"
                | "object"
                | "embed"
                | "input"
        ) {
            return false;
        }
        if !matches!(
            child_style.display,
            DisplayMode::Inline | DisplayMode::Contents
        ) || !inline_style_can_flatten_into(&child_style, parent_style)
            || resolver
                .compute_pseudo_style(
                    &info,
                    &child_style,
                    ancestors,
                    crate::style::PseudoTarget::Before,
                )
                .is_some()
            || resolver
                .compute_pseudo_style(
                    &info,
                    &child_style,
                    ancestors,
                    crate::style::PseudoTarget::After,
                )
                .is_some()
        {
            return false;
        }

        let mut child_ancestors = ancestors.to_vec();
        child_ancestors.push(info);
        if !inline_children_only(&child, resolver, &child_style, &child_ancestors) {
            return false;
        }
    }
    true
}

fn data_attribute_value_is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "1" | "true" | "yes" | "on"
    )
}

/// Return whether an inline wrapper is semantically transparent to the single-style
/// paragraph fast path.  Display alone is not enough: flattening a styled descendant
/// would otherwise discard its typography or inline box paint.
fn inline_style_can_flatten_into(style: &ComputedStyle, parent: &ComputedStyle) -> bool {
    text_style_for_flow_text(style) == text_style_for_flow_text(parent)
        && style.white_space == parent.white_space
        && style.text_transform == parent.text_transform
        && style.text_wrap_mode == parent.text_wrap_mode
        && style.direction == parent.direction
        && style.vertical_align == parent.vertical_align
        && style.margin == EdgeSizes::zero()
        && style.padding == EdgeSizes::zero()
        && style.border_width == EdgeSizes::zero()
        && style.border_image.source.is_none()
        && style.background_color.is_none()
        && style.background_paint.is_none()
        && style.background_paints.is_empty()
        && style.box_shadow.is_none()
        && style.box_shadows.is_empty()
        && style.paint_filter.is_none()
        && style.backdrop_filter.is_none()
        && style.clip_path.is_none()
        && style.legacy_clip.is_none()
        && !style.outline_visible
        && style.opacity >= 1.0 - 1.0e-6
        && style.transform.is_empty()
        && matches!(style.position, PositionMode::Static)
        && matches!(style.float_mode, FloatMode::None)
        && matches!(style.width, LengthSpec::Auto)
        && matches!(style.height, LengthSpec::Auto)
        && style.content.is_none()
        && style.generated_content.is_none()
}

fn inline_or_replaced_children_only(
    node: &NodeRef,
    resolver: &StyleResolver,
    parent_style: &ComputedStyle,
    ancestors: &[ElementInfo],
) -> bool {
    let mut saw_content = false;
    for child in node.children() {
        match child.data() {
            NodeData::Text(text) => {
                let cleaned = normalize_text(&text.borrow(), parent_style.white_space, true);
                if !cleaned.is_empty() {
                    saw_content = true;
                }
            }
            NodeData::Element(element) => {
                let tag = element.name.local.as_ref().to_ascii_lowercase();
                let info = element_info(&child, resolver.has_sibling_selectors());
                let inline_style = element
                    .attributes
                    .borrow()
                    .get("style")
                    .map(|s| s.to_string());
                let child_style =
                    resolver.compute_style(&info, parent_style, inline_style.as_deref(), ancestors);
                if matches!(child_style.display, DisplayMode::None) {
                    continue;
                }
                if tag == "br" {
                    saw_content = true;
                    continue;
                }
                if matches!(child_style.display, DisplayMode::Contents) {
                    let mut child_ancestors = ancestors.to_vec();
                    child_ancestors.push(info);
                    if !inline_or_replaced_children_only(
                        &child,
                        resolver,
                        &child_style,
                        &child_ancestors,
                    ) {
                        return false;
                    }
                    saw_content = true;
                    continue;
                }
                if matches!(
                    tag.as_str(),
                    "hr" | "canvas" | "video" | "audio" | "iframe" | "object" | "embed"
                ) {
                    return false;
                }
                match child_style.display {
                    DisplayMode::Inline
                    | DisplayMode::InlineBlock
                    | DisplayMode::InlineFlex
                    | DisplayMode::InlineGrid
                    | DisplayMode::InlineTable => {
                        saw_content = true;
                    }
                    _ => return false,
                }
            }
            _ => {}
        }
    }
    saw_content
}

fn static_block_with_visible_replaced_overflow(
    node: &NodeRef,
    resolver: &StyleResolver,
    style: &ComputedStyle,
    ancestors: &[ElementInfo],
) -> bool {
    if !matches!(style.position, PositionMode::Static)
        || !matches!(style.overflow, OverflowMode::Visible)
    {
        return false;
    }
    let Some(block_height) = resolve_non_auto_css_dimension(
        style.height,
        Pt::from_f32(150.0),
        style.font_size,
        style.root_font_size,
        true,
    ) else {
        return false;
    };

    node.children().any(|child| {
        let NodeData::Element(element) = child.data() else {
            return false;
        };
        if !element.name.local.as_ref().eq_ignore_ascii_case("img") {
            return false;
        }
        let info = element_info(&child, resolver.has_sibling_selectors());
        let inline_style = element
            .attributes
            .borrow()
            .get("style")
            .map(|value| value.to_string());
        let child_style = resolver.compute_style(&info, style, inline_style.as_deref(), ancestors);
        if matches!(child_style.display, DisplayMode::None) {
            return false;
        }
        resolve_non_auto_css_dimension(
            child_style.height,
            block_height,
            child_style.font_size,
            child_style.root_font_size,
            true,
        )
        .is_some_and(|replaced_height| replaced_height > block_height)
    })
}

fn coerce_items_to_inline_run(
    items: Vec<LayoutItem>,
    default_valign: VerticalAlign,
    parent_style: &ComputedStyle,
    font_registry: Option<Arc<FontRegistry>>,
    snap_baseline_phase: bool,
) -> Vec<LayoutItem> {
    let preserve_nested_floor = snap_baseline_phase
        && items.iter().any(|item| {
            matches!(
                item,
                LayoutItem::Inline {
                    valign: VerticalAlign::BaselineShift(shift),
                    ..
                } if *shift < Pt::ZERO && shift.to_milli_i64().rem_euclid(750) != 0
            )
        });
    let parent_text_style = parent_style.to_text_style();
    let round_nested_baseline = snap_baseline_phase
        && !preserve_nested_floor
        && (css_print_line_prefers_nearest_baseline_snap(
            &parent_text_style,
            font_registry.as_deref(),
        ) || (parent_text_style.line_height.to_milli_i64().rem_euclid(750) != 0
            && items.iter().any(|item| {
                matches!(
                    item,
                    LayoutItem::Inline {
                        valign: VerticalAlign::Bottom,
                        ..
                    }
                )
            })));
    let wrap_line_box = |flowable: Box<dyn Flowable>| -> Box<dyn Flowable> {
        let line = CssLineBoxFlowable::new(flowable)
            .with_round_baseline(round_nested_baseline)
            .with_nested_floor_compensation(preserve_nested_floor)
            .with_parent_positioned_top_overflow(true);
        Box::new(line) as Box<dyn Flowable>
    };
    let make_strut = || {
        let strut = Paragraph::new("")
            .with_style(text_style_for_flow_text(parent_style))
            .with_whitespace(preserve_whitespace(parent_style.white_space), true)
            .with_break_spaces(matches!(
                parent_style.white_space,
                WhiteSpaceMode::BreakSpaces
            ))
            .with_font_registry(font_registry.clone());
        LayoutItem::Inline {
            flowable: if snap_baseline_phase {
                wrap_line_box(Box::new(strut))
            } else {
                Box::new(strut) as Box<dyn Flowable>
            },
            valign: VerticalAlign::Baseline,
            flex_grow: 0.0,
            flex_shrink: 0.0,
            width_spec: None,
            order: 0,
        }
    };
    let item_count = items.len();
    let mut inline = Vec::with_capacity(item_count.saturating_mul(2) + 1);
    inline.push(make_strut());
    for (index, item) in items.into_iter().enumerate() {
        let is_forced_break = match &item {
            LayoutItem::Block { flowable, .. } | LayoutItem::Inline { flowable, .. } => {
                flowable.forced_line_break_height().is_some()
            }
        };
        let item = match item {
            LayoutItem::Inline { .. } => item,
            LayoutItem::Block {
                flowable,
                flex_grow,
                flex_shrink,
                width_spec,
                order,
            } => LayoutItem::Inline {
                flowable,
                valign: default_valign,
                flex_grow,
                flex_shrink,
                width_spec,
                order,
            },
        };
        let item = if !snap_baseline_phase || is_forced_break {
            item
        } else {
            match item {
                LayoutItem::Inline {
                    flowable,
                    valign,
                    flex_grow,
                    flex_shrink,
                    width_spec,
                    order,
                } => LayoutItem::Inline {
                    flowable: wrap_line_box(flowable),
                    valign,
                    flex_grow,
                    flex_shrink,
                    width_spec,
                    order,
                },
                LayoutItem::Block { .. } => unreachable!("coerced inline item"),
            }
        };
        inline.push(item);
        if is_forced_break && index + 1 < item_count {
            inline.push(make_strut());
        }
    }
    inline
}

fn css_display_list_item_flowables(
    node: &NodeRef,
    resolver: &StyleResolver,
    info: &ElementInfo,
    style: &ComputedStyle,
    marker_ancestors: &[ElementInfo],
    child_ancestors: &mut Vec<ElementInfo>,
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<Arc<AssetBundle>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
) -> Vec<LayoutItem> {
    let mut report = report;
    let (marker_override, marker_pseudo_style) =
        marker_presentation(resolver, info, style, marker_ancestors, counters);
    let marker_style = marker_pseudo_style.as_ref().unwrap_or(style);
    let round_marker_baseline = marker_style.font_size > style.font_size;
    let has_marker_content_override = marker_override.is_some();
    let marker_image = if !has_marker_content_override {
        list_marker_image_flowable(
            marker_style,
            asset_bundle.as_deref(),
            svg_form,
            svg_raster_fallback,
        )
    } else {
        None
    };
    let marker_index = current_list_item_counter_index(counters);
    let marker_prefix = match marker_override {
        Some(prefix) => prefix,
        None if marker_image.is_some() => None,
        None => list_marker_prefix(style, false, marker_index),
    };
    let marker_is_inside = matches!(style.list_style_position, ListStylePositionMode::Inside);
    let marker_bullet = if !has_marker_content_override && marker_image.is_none() {
        native_list_bullet_flowable(marker_style, false)
    } else {
        None
    };
    let marker_cjk = if !marker_is_inside && !has_marker_content_override && marker_image.is_none()
    {
        cjk_decimal_marker_flowable(marker_style, marker_index)
    } else {
        None
    };
    let mut consumed_inside_marker = false;
    let body: Box<dyn Flowable> = if marker_is_inside
        && (marker_prefix.is_some() || marker_bullet.is_some())
        && inline_children_only(node, resolver, style, child_ancestors)
    {
        let text = extract_text(node, style.white_space);
        let text = if marker_bullet.is_some() {
            text
        } else {
            format!("{}{}", marker_prefix.as_deref().unwrap_or(""), text)
        };
        let text = apply_text_transform(&text, style.text_transform);
        let text_style = text_style_for_flow_text(style);
        report_missing_glyphs(
            report.as_deref_mut(),
            font_registry.as_deref(),
            &text_style,
            &text,
        );
        consumed_inside_marker = marker_bullet.is_none();
        Box::new(
            Paragraph::new(text)
                .with_style(text_style)
                .with_align(text_align_from_style(style))
                .with_last_align(text_align_last_from_style(style))
                .with_whitespace(preserve_whitespace(style.white_space), no_wrap(style))
                .with_break_spaces(matches!(style.white_space, WhiteSpaceMode::BreakSpaces))
                .with_pagination(style.pagination)
                .with_font_registry(font_registry.clone())
                .with_tag_role("LBody"),
        ) as Box<dyn Flowable>
    } else {
        let children = collect_children(
            node,
            resolver,
            style,
            child_ancestors,
            counters,
            font_registry.clone(),
            asset_bundle,
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
            perf,
            doc_id,
        );
        container_flowable_with_role(children, style, Some("LBody"))
            .unwrap_or_else(|| Box::new(Spacer::new_pt(Pt::ZERO)) as Box<dyn Flowable>)
    };

    let flowable = if consumed_inside_marker {
        body
    } else if let Some(label) = marker_image {
        Box::new(
            ListItemFlowable::new_with_label(label, body, Pt::from_f32(6.0))
                .with_marker_inside(marker_is_inside)
                .with_marker_line_height(text_style_for_flow_text(marker_style).line_height)
                .with_pagination(style.pagination),
        ) as Box<dyn Flowable>
    } else if let Some((label, outside_gap)) = marker_bullet {
        let gap = if marker_is_inside {
            let label_width = label.intrinsic_width().unwrap_or(Pt::ZERO);
            (marker_style.font_size.mul_ratio(4, 3) - label_width).max(Pt::ZERO)
        } else {
            outside_gap
        };
        Box::new(
            ListItemFlowable::new_with_label(label, body, gap)
                .with_marker_inside(marker_is_inside)
                .with_pagination(style.pagination),
        ) as Box<dyn Flowable>
    } else if let Some(label) = marker_cjk {
        Box::new(
            ListItemFlowable::new_with_label(label, body, Pt::ZERO)
                .with_marker_inside(false)
                .with_pagination(style.pagination),
        ) as Box<dyn Flowable>
    } else if marker_prefix.is_none() {
        body
    } else {
        let prefix = marker_prefix.unwrap_or_default();
        let text_style = marker_text_style(marker_style, style);
        report_missing_glyphs(
            report.as_deref_mut(),
            font_registry.as_deref(),
            &text_style,
            &prefix,
        );
        let label_para = Paragraph::new(prefix)
            .with_style(text_style)
            .with_align(text_align_from_style(marker_style))
            .with_last_align(text_align_last_from_style(marker_style))
            .with_whitespace(
                preserve_whitespace(marker_style.white_space),
                no_wrap(marker_style),
            )
            .with_break_spaces(matches!(
                marker_style.white_space,
                WhiteSpaceMode::BreakSpaces
            ))
            .with_pagination(style.pagination)
            .with_font_registry(font_registry.clone())
            .with_tag_role("Lbl");
        Box::new(
            ListItemFlowable::new(label_para, body, Pt::ZERO)
                .with_marker_inside(marker_is_inside)
                .with_pagination(style.pagination),
        ) as Box<dyn Flowable>
    };
    let flowable =
        Box::new(CssLineBoxFlowable::new(flowable).with_round_baseline(round_marker_baseline))
            as Box<dyn Flowable>;

    let items = vec![LayoutItem::Block {
        flowable,
        flex_grow: 0.0,
        flex_shrink: 1.0,
        width_spec: flex_item_basis(style),
        order: 0,
    }];
    if style_is_inline_list_item(style) {
        if let Some(container) = container_flowable_with_role(items, style, Some("LI")) {
            let valign = vertical_align_from_style(style);
            vec![LayoutItem::Inline {
                flowable: container,
                valign,
                flex_grow: style.flex_grow,
                flex_shrink: style.flex_shrink,
                width_spec: flex_item_basis(style),
                order: 0,
            }]
        } else {
            Vec::new()
        }
    } else {
        container_flowables_with_role(items, style, Some("LI"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{Canvas, Command};

    fn approx_eq_pt(value: Pt, expected: f32) -> bool {
        (value.to_f32() - expected).abs() <= 0.01
    }

    #[test]
    fn percentage_length_components_request_child_paint_snapping() {
        assert!(length_has_percentage_component(LengthSpec::Percent(0.05)));
        assert!(length_has_percentage_component(LengthSpec::Calc(
            CalcLength {
                abs: Pt::from_f32(2.0),
                percent: 0.05,
                em: 0.0,
                rem: 0.0,
            }
        )));
        assert!(!length_has_percentage_component(LengthSpec::Absolute(
            Pt::from_f32(3.5)
        )));
    }

    fn resolved_clip_path_from_html(html: &str, css: &str) -> ClipPathShapeSpec {
        let document = parse_html(html);
        let box_node = document
            .select_first("#box")
            .expect("select clipped box")
            .as_node()
            .clone();
        let resolver = StyleResolver::new(css);
        let parent = resolver.default_style();
        let info = element_info(&box_node, resolver.has_sibling_selectors());
        let mut style = resolver.compute_style(&info, &parent, None, &[]);
        resolve_inline_svg_clip_source(&box_node, &mut style);
        style.clip_path.expect("resolved clip path")
    }

    fn resolved_filter_from_html(html: &str, css: &str) -> PaintFilterSpec {
        let document = parse_html(html);
        let box_node = document
            .select_first("#box")
            .expect("select filtered box")
            .as_node()
            .clone();
        let resolver = StyleResolver::new(css);
        let parent = resolver.default_style();
        let info = element_info(&box_node, resolver.has_sibling_selectors());
        let mut style = resolver.compute_style(&info, &parent, None, &[]);
        resolve_inline_svg_filter_sources(&box_node, &mut style);
        style.paint_filter.expect("resolved filter program")
    }

    #[test]
    fn inline_svg_object_bbox_circle_compiles_to_scalable_vector_clip() {
        let clip = resolved_clip_path_from_html(
            r#"<html><body><svg><defs><clipPath id="clip" clipPathUnits="objectBoundingBox"><circle cx="0.5" cy="0.5" r="0.42"/></clipPath></defs></svg><div id="box"></div></body></html>"#,
            "#box { clip-path: url(#clip); }",
        );
        assert_eq!(
            clip,
            ClipPathShapeSpec::Ellipse(ClipPathEllipseSpec {
                radius_x: ClipPathShapeRadius::Length(LengthSpec::Percent(0.42)),
                radius_y: ClipPathShapeRadius::Length(LengthSpec::Percent(0.42)),
                center_x: LengthSpec::Percent(0.5),
                center_y: LengthSpec::Percent(0.5),
            })
        );
    }

    #[test]
    fn inline_svg_user_space_path_preserves_evenodd_holes() {
        let clip = resolved_clip_path_from_html(
            r#"<html><body><svg><defs><clipPath id="ring" clipPathUnits="userSpaceOnUse"><path clip-rule="evenodd" d="M0 0H200V140H0Z M60 40H140V100H60Z"/></clipPath></defs></svg><div id="box"></div></body></html>"#,
            "#box { clip-path: url(#ring); }",
        );
        let ClipPathShapeSpec::Path(path) = clip else {
            panic!("expected vector path clip");
        };
        assert!(path.evenodd);
        assert_eq!(path.commands.len(), 10);
    }

    #[test]
    fn inline_svg_filter_compiles_to_ordered_vector_program() {
        let filter = resolved_filter_from_html(
            r##"<html><body><svg><defs><filter id="fx" x="-20%" width="140%"><feFlood flood-color="#1565c0" result="blue"/><feBlend in="SourceGraphic" in2="blue" mode="multiply"/></filter></defs></svg><div id="box"></div></body></html>"##,
            "#box { filter: url(#fx); }",
        );
        let [PaintFilterOperation::Svg(program)] = filter.operations.as_slice() else {
            panic!("expected compiled SVG filter operation");
        };
        assert!(program.linear_rgb);
        assert_eq!(program.region.x, -0.2);
        assert_eq!(program.region.width, 1.4);
        assert_eq!(program.nodes.len(), 2);
        assert_eq!(program.nodes[0].result.as_deref(), Some("blue"));
        assert!(matches!(
            program.nodes[1].primitive,
            SvgFilterPrimitive::Blend {
                input: SvgFilterInput::SourceGraphic,
                input2: SvgFilterInput::Named(ref name),
                mode: crate::types::MixBlendMode::Multiply,
            } if name == "blue"
        ));
    }

    #[test]
    fn inline_svg_filter_retains_srgb_color_interpolation_mode() {
        let filter = resolved_filter_from_html(
            r#"<html><body><svg><defs><filter id="fx" color-interpolation-filters="sRGB"><feColorMatrix type="saturate" values="0"/></filter></defs></svg><div id="box"></div></body></html>"#,
            "#box { filter: url(#fx); }",
        );
        let [PaintFilterOperation::Svg(program)] = filter.operations.as_slice() else {
            panic!("expected compiled SVG filter operation");
        };
        assert!(!program.linear_rgb);
        assert!(matches!(
            program.nodes[0].primitive,
            SvgFilterPrimitive::ColorMatrix { .. }
        ));
    }

    #[test]
    fn styled_empty_i_compiles_a_principal_box() {
        let document = parse_html("<html><body><i id='swatch'></i></body></html>");
        let resolver = StyleResolver::new(
            "#swatch { display: block; position: absolute; width: 96px; height: 24px; background: red; }",
        );
        let root = resolver.default_style();
        let swatch = document.select_first("#swatch").expect("styled i element");
        let items = node_to_flowables(
            swatch.as_node(),
            &resolver,
            &root,
            &mut Vec::new(),
            &mut CounterState::default(),
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );

        assert_eq!(items.len(), 1, "a styled empty phrasing element owns a box");
        let LayoutItem::Block { flowable, .. } = &items[0] else {
            panic!("display:block must compile a block principal box");
        };
        assert!(flowable.out_of_flow());
    }

    #[test]
    fn css_filter_is_compiled_before_vector_clip_path() {
        let document = parse_html("<html><body><div id='box'></div></body></html>");
        let resolver = StyleResolver::new(
            "#box { width: 120px; height: 120px; background: red; filter: blur(10px); clip-path: circle(46px at 60px 60px); }",
        );
        let root = resolver.default_style();
        let node = document.select_first("#box").expect("filtered box");
        let items = node_to_flowables(
            node.as_node(),
            &resolver,
            &root,
            &mut Vec::new(),
            &mut CounterState::default(),
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );
        let [LayoutItem::Block { flowable, .. }] = items.as_slice() else {
            panic!("expected one filtered block");
        };
        let page = Size {
            width: Pt::from_f32(180.0),
            height: Pt::from_f32(180.0),
        };
        let mut canvas = Canvas::new(page);
        flowable.draw(&mut canvas, Pt::ZERO, Pt::ZERO, page.width, page.height);
        let commands = &canvas.finish().pages[0].commands;
        let clip = commands
            .iter()
            .position(|command| matches!(command, Command::ClipPath { .. }))
            .expect("post-filter vector clip");
        let filter = commands
            .iter()
            .position(|command| matches!(command, Command::DrawFilteredForm { .. }))
            .expect("compiled filter surface");
        assert!(clip < filter, "clip must consume the filtered surface");
    }

    #[test]
    fn zero_sized_absolute_svg_definitions_do_not_consume_flow_height() {
        let document = parse_html(
            "<html><body><svg class='defs' width='0' height='0' aria-hidden='true'><defs><filter id='fx'><feColorMatrix type='saturate' values='0'/></filter></defs></svg></body></html>",
        );
        let resolver = StyleResolver::new(
            "svg.defs { position: absolute; width: 0; height: 0; overflow: hidden; }",
        );
        let root = resolver.default_style();
        let node = document.select_first("svg.defs").expect("definition svg");
        let items = node_to_flowables(
            node.as_node(),
            &resolver,
            &root,
            &mut Vec::new(),
            &mut CounterState::default(),
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );
        assert_eq!(items.len(), 1, "expected one positioned SVG carrier");
        let flowable = match &items[0] {
            LayoutItem::Block { flowable, .. } | LayoutItem::Inline { flowable, .. } => flowable,
        };
        assert!(flowable.out_of_flow());
        assert_eq!(
            flowable
                .wrap(Pt::from_f32(180.0), Pt::from_f32(90.0))
                .height,
            Pt::ZERO
        );
    }

    #[test]
    fn synthetic_bold_inline_pseudo_text_uses_fixed_browser_paint_phases() {
        let regular = TextStyle::default();
        assert_eq!(
            pseudo_inline_text_paint_phase_y(crate::style::PseudoTarget::Before, &regular),
            Pt::ZERO
        );

        let mut synthetic_bold = regular.clone();
        synthetic_bold.font_weight = 700;
        assert_eq!(
            pseudo_inline_text_paint_phase_y(crate::style::PseudoTarget::Before, &synthetic_bold,),
            -Pt::from_milli_i64(75)
        );
        assert_eq!(
            pseudo_inline_text_paint_phase_y(crate::style::PseudoTarget::After, &synthetic_bold,),
            -Pt::from_milli_i64(225)
        );

        synthetic_bold.font_face_satisfies_weight = true;
        assert_eq!(
            pseudo_inline_text_paint_phase_y(crate::style::PseudoTarget::Before, &synthetic_bold,),
            Pt::ZERO
        );
    }

    #[test]
    fn generated_quote_tokens_share_document_order_depth() {
        let resolver = StyleResolver::new("");
        let mut style = resolver.default_style();
        style.quotes = vec![
            ("<".to_string(), ">".to_string()),
            ("[".to_string(), "]".to_string()),
        ];
        style.generated_content = Some(vec![
            GeneratedContentPart::OpenQuote,
            GeneratedContentPart::Text("outer ".to_string()),
            GeneratedContentPart::OpenQuote,
            GeneratedContentPart::Text("inner".to_string()),
            GeneratedContentPart::CloseQuote,
            GeneratedContentPart::CloseQuote,
        ]);
        let mut state = CounterState::default();
        assert_eq!(
            generated_content_text(&style, &mut state).as_deref(),
            Some("<outer [inner]>")
        );
        assert_eq!(state.quote_depth, 0);

        style.generated_content = Some(vec![GeneratedContentPart::NoOpenQuote]);
        assert_eq!(
            generated_content_text(&style, &mut state).as_deref(),
            Some("")
        );
        assert_eq!(state.quote_depth, 1);
        style.generated_content = Some(vec![
            GeneratedContentPart::OpenQuote,
            GeneratedContentPart::Text("nested".to_string()),
            GeneratedContentPart::CloseQuote,
        ]);
        assert_eq!(
            generated_content_text(&style, &mut state).as_deref(),
            Some("[nested]")
        );
        assert_eq!(state.quote_depth, 1);
        style.generated_content = Some(vec![GeneratedContentPart::NoCloseQuote]);
        generated_content_text(&style, &mut state);
        assert_eq!(state.quote_depth, 0);
    }

    #[test]
    fn first_letter_prefix_includes_leading_and_trailing_punctuation() {
        let text = "\u{201c}Q\u{201d}uoted";
        let end = css_first_letter_prefix_end(text).expect("first-letter prefix");
        assert_eq!(&text[..end], "\u{201c}Q\u{201d}");
        assert_eq!(&text[end..], "uoted");
        assert_eq!(
            css_first_letter_prefix_end("Hello").map(|end| &"Hello"[..end]),
            Some("H")
        );
    }

    #[test]
    fn multicol_span_boundaries_compile_independent_column_runs() {
        let resolver = StyleResolver::new("");
        let mut style = resolver.default_style();
        style.column_count = 2;
        style.column_count_auto = false;
        style.gap = LengthSpec::Absolute(Pt::ZERO);
        style.column_gap_normal = false;

        let block = || {
            Box::new(
                ContainerFlowable::new_pt(Vec::new(), style.font_size, style.root_font_size)
                    .with_height(LengthSpec::Absolute(Pt::from_f32(40.0))),
            ) as Box<dyn Flowable>
        };
        let span = Box::new(
            ContainerFlowable::new_pt(Vec::new(), style.font_size, style.root_font_size)
                .with_height(LengthSpec::Absolute(Pt::from_f32(32.0)))
                .with_column_span_all(true),
        ) as Box<dyn Flowable>;

        let compiled = compile_multicol_child_stream(
            vec![block(), block(), span, block(), block()],
            &style,
            ContainerCompilationOptions::default(),
        );

        assert_eq!(compiled.len(), 3);
        assert!(compiled[0].debug_name().contains("MultiColumnFlowable"));
        assert!(compiled[1].spans_all_columns());
        assert!(compiled[2].debug_name().contains("MultiColumnFlowable"));
        assert_eq!(
            compiled[0]
                .wrap(Pt::from_f32(200.0), Pt::from_f32(500.0))
                .height,
            Pt::from_f32(40.0)
        );
        assert_eq!(
            compiled[1]
                .wrap(Pt::from_f32(200.0), Pt::from_f32(500.0))
                .height,
            Pt::from_f32(32.0)
        );
    }

    #[test]
    fn empty_block_column_span_survives_dom_lowering() {
        let document = parse_html(
            "<html><body><div class='cols'><div class='banner'></div></div></body></html>",
        );
        let resolver = StyleResolver::new(
            ".cols { column-count: 3; } .banner { column-span: all; height: 40px; background: red; }",
        );
        let root = resolver.default_style();
        let cols = document.select_first(".cols").expect("column container");
        let mut cols_info = element_info(cols.as_node(), resolver.has_sibling_selectors());
        let cols_style = resolver.compute_style(&cols_info, &root, None, &[]);
        cols_info.apply_computed_container_style(&cols_style);
        let banner = document.select_first(".banner").expect("spanning block");
        let mut ancestors = vec![cols_info];
        let mut counters = CounterState::default();
        let items = node_to_flowables(
            banner.as_node(),
            &resolver,
            &cols_style,
            &mut ancestors,
            &mut counters,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );

        assert_eq!(items.len(), 1);
        let flowable = match &items[0] {
            LayoutItem::Block { flowable, .. } | LayoutItem::Inline { flowable, .. } => flowable,
        };
        assert!(
            flowable.spans_all_columns(),
            "column-span marker lost on {}",
            flowable.debug_name()
        );
    }

    #[test]
    fn replaced_image_percentage_width_is_deferred_to_its_containing_block() {
        let resolver = StyleResolver::new("");
        let mut style = resolver.default_style();
        style.width = LengthSpec::Percent(0.5);
        style.height = LengthSpec::Auto;
        let sizing = resolve_replaced_image_sizing(
            &style,
            None,
            None,
            Some((Pt::from_f32(6.0), Pt::from_f32(3.0))),
        );

        assert_eq!(sizing.width, LengthSpec::Percent(0.5));
        assert_eq!(sizing.height, LengthSpec::Auto);
        assert_eq!(sizing.aspect_ratio, Some(2.0));
    }

    #[test]
    fn css_replaced_size_overrides_html_presentational_hint() {
        let resolver = StyleResolver::new("");
        let mut style = resolver.default_style();
        style.width = LengthSpec::Absolute(Pt::from_f32(75.0));
        let sizing = resolve_replaced_image_sizing(
            &style,
            Some(Pt::from_f32(150.0)),
            None,
            Some((Pt::from_f32(6.0), Pt::from_f32(3.0))),
        );

        assert_eq!(sizing.width, style.width);
        assert_eq!(sizing.nominal_width, Pt::from_f32(75.0));
        assert_eq!(sizing.nominal_height, Pt::from_f32(37.5));
    }

    #[test]
    fn replaced_max_height_compiles_a_ratio_preserving_used_size() {
        let resolver = StyleResolver::new("");
        let mut style = resolver.default_style();
        // 22px wide with an 18:100 intrinsic ratio tentatively produces a
        // 122.222px height. A 118px cap transfers back to a 21.24px width.
        style.width = LengthSpec::Absolute(Pt::from_f32(16.5));
        style.height = LengthSpec::Auto;
        style.max_height = LengthSpec::Absolute(Pt::from_f32(88.5));
        let sizing = resolve_replaced_image_sizing(
            &style,
            None,
            None,
            Some((Pt::from_f32(13.5), Pt::from_f32(75.0))),
        );

        assert_eq!(sizing.width, LengthSpec::Absolute(Pt::from_f32(15.93)));
        assert_eq!(sizing.height, LengthSpec::Absolute(Pt::from_f32(88.5)));
        assert_eq!(sizing.nominal_width, Pt::from_f32(15.93));
        assert_eq!(sizing.nominal_height, Pt::from_f32(88.5));
    }

    #[test]
    fn replaced_constraint_table_preserves_ratio_until_constraints_conflict() {
        let w = Pt::from_f32(100.0);
        let h = Pt::from_f32(50.0);
        assert_eq!(
            constrain_replaced_size(w, h, None, None, None, Some(Pt::from_f32(40.0)),),
            (Pt::from_f32(80.0), Pt::from_f32(40.0))
        );
        assert_eq!(
            constrain_replaced_size(
                w,
                h,
                None,
                Some(Pt::from_f32(90.0)),
                None,
                Some(Pt::from_f32(40.0)),
            ),
            (Pt::from_f32(80.0), Pt::from_f32(40.0))
        );
        assert_eq!(
            constrain_replaced_size(
                w,
                h,
                Some(Pt::from_f32(120.0)),
                None,
                Some(Pt::from_f32(80.0)),
                None,
            ),
            (Pt::from_f32(160.0), Pt::from_f32(80.0))
        );
        assert_eq!(
            constrain_replaced_size(
                w,
                h,
                Some(Pt::from_f32(120.0)),
                None,
                None,
                Some(Pt::from_f32(40.0)),
            ),
            (Pt::from_f32(120.0), Pt::from_f32(40.0))
        );
    }

    #[test]
    fn text_style_for_flow_text_gates_ellipsis_on_overflow_clipping() {
        let resolver = StyleResolver::new("");
        let mut style = resolver.default_style();
        style.text_overflow = crate::style::TextOverflowMode::Ellipsis;

        style.overflow = OverflowMode::Visible;
        assert_eq!(
            text_style_for_flow_text(&style).text_overflow,
            crate::style::TextOverflowMode::Clip
        );

        style.overflow = OverflowMode::Hidden;
        assert_eq!(
            text_style_for_flow_text(&style).text_overflow,
            crate::style::TextOverflowMode::Ellipsis
        );

        style.overflow = OverflowMode::Clip;
        assert_eq!(
            text_style_for_flow_text(&style).text_overflow,
            crate::style::TextOverflowMode::Ellipsis
        );
    }

    #[test]
    fn element_info_tracks_type_position_across_mixed_siblings() {
        let document = parse_html(
            "<html><body><div><em></em><span id='first'></span><strong></strong><span id='second'></span></div></body></html>",
        );
        let first = document.select_first("#first").expect("first span");
        let first = element_info(first.as_node(), false);
        assert_eq!((first.child_index, first.child_count), (2, 4));
        assert_eq!((first.type_index, first.type_count), (1, 2));

        let second = document.select_first("#second").expect("second span");
        let second = element_info(second.as_node(), false);
        assert_eq!((second.child_index, second.child_count), (4, 4));
        assert_eq!((second.type_index, second.type_count), (2, 2));
    }

    #[test]
    fn inline_text_boundaries_keep_collapsible_spaces_between_items() {
        assert_eq!(
            normalize_text_node_boundaries("Text \n", WhiteSpaceMode::Normal, true, false),
            "Text "
        );
        assert_eq!(
            normalize_text_node_boundaries("\n Text", WhiteSpaceMode::Normal, false, true),
            " Text"
        );
        assert_eq!(
            normalize_text_node_boundaries(" \n Text \n ", WhiteSpaceMode::Normal, true, true),
            "Text"
        );
    }

    #[test]
    fn whitespace_only_nodes_between_inline_blocks_create_inline_space() {
        let document = parse_html(
            "<html><body><div class='row'><span class='item'></span> \n \
             <span class='item'></span></div></body></html>",
        );
        let resolver = StyleResolver::new(
            ".row { display: block; } \
             .item { display: inline-block; width: 70px; height: 28px; }",
        );
        let root = resolver.default_style();
        let row = document.select_first(".row").expect("row");
        let row_info = element_info(row.as_node(), resolver.has_sibling_selectors());
        let row_style = resolver.compute_style(&row_info, &root, None, &[]);
        let mut ancestors = vec![row_info];
        let items = collect_children(
            row.as_node(),
            &resolver,
            &row_style,
            &mut ancestors,
            &mut CounterState::default(),
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );

        assert_eq!(items.len(), 3, "two boxes plus one collapsed space");
        assert!(
            items
                .iter()
                .all(|item| matches!(item, LayoutItem::Inline { .. }))
        );
        let LayoutItem::Inline { flowable, .. } = &items[1] else {
            unreachable!("space is asserted inline")
        };
        assert!(
            flowable
                .intrinsic_width()
                .is_some_and(|width| width > Pt::ZERO),
            "the collapsed space must retain the active font's advance"
        );
    }

    #[test]
    fn block_pseudo_content_is_not_coerced_into_the_inline_run() {
        let document = parse_html(
            "<html><body><div class='card'>Body text follows the header.</div></body></html>",
        );
        let resolver = StyleResolver::new(
            ".card { display: block; font-size: 22px; } \
             .card::before { content: 'HEADER'; display: block; padding: 4px 8px; }",
        );
        let root = resolver.default_style();
        let card = document.select_first(".card").expect("card");
        let info = element_info(card.as_node(), resolver.has_sibling_selectors());
        let style = resolver.compute_style(&info, &root, None, &[]);
        let before = pseudo_items_for(
            &resolver,
            &info,
            &style,
            &[],
            &mut CounterState::default(),
            None,
            None,
            None,
            false,
            false,
            crate::style::PseudoTarget::Before,
        );

        assert_eq!(before.len(), 1);
        assert!(matches!(before[0], LayoutItem::Block { .. }));
        assert!(!pseudo_items_are_inline(&before, &[]));
    }

    #[test]
    fn empty_styled_inline_pseudo_synthesizes_its_bottom_edge_baseline() {
        let document = parse_html("<html><body><span class='item'>marker</span></body></html>");
        let resolver = StyleResolver::new(
            ".item::before { content: ''; display: inline-block; width: 18px; height: 18px; border: 2px solid black; }",
        );
        let root = resolver.default_style();
        let item = document.select_first(".item").expect("item");
        let info = element_info(item.as_node(), resolver.has_sibling_selectors());
        let style = resolver.compute_style(&info, &root, None, &[]);
        let before = pseudo_items_for(
            &resolver,
            &info,
            &style,
            &[],
            &mut CounterState::default(),
            None,
            None,
            None,
            false,
            false,
            crate::style::PseudoTarget::Before,
        );

        let LayoutItem::Inline { flowable, .. } = &before[0] else {
            panic!("inline-block pseudo must remain inline")
        };
        let available = Pt::from_f32(200.0);
        assert!(flowable.wrap(available, available).height > Pt::ZERO);
        assert_eq!(
            flowable.inline_baseline(available),
            None,
            "an empty inline-block has no internal line baseline"
        );
    }

    #[test]
    fn generated_list_item_compiles_a_native_square_marker() {
        let document = parse_html("<html><body><div class='box'>Body</div></body></html>");
        let resolver = StyleResolver::new(
            ".box::before { content: 'Generated item'; display: list-item; list-style-type: square; color: red; }",
        );
        let root = resolver.default_style();
        let item = document.select_first(".box").expect("box");
        let info = element_info(item.as_node(), resolver.has_sibling_selectors());
        let style = resolver.compute_style(&info, &root, None, &[]);
        let before = pseudo_items_for(
            &resolver,
            &info,
            &style,
            &[],
            &mut CounterState::default(),
            None,
            None,
            None,
            false,
            false,
            crate::style::PseudoTarget::Before,
        );

        let LayoutItem::Block { flowable, .. } = &before[0] else {
            panic!("generated list-item must be a block")
        };
        let available = Pt::from_f32(200.0);
        let mut canvas = Canvas::new(Size {
            width: available,
            height: available,
        });
        flowable.draw(
            &mut canvas,
            Pt::from_f32(40.0),
            Pt::ZERO,
            available,
            available,
        );
        assert!(canvas.finish().pages[0].commands.iter().any(|command| {
            matches!(
                command,
                Command::DrawRect { width, height, .. }
                    if *width > Pt::ZERO && *width == *height
            )
        }));
    }

    #[test]
    fn text_align_start_and_end_resolve_against_direction() {
        let resolver = StyleResolver::new("");
        let mut style = resolver.default_style();

        style.text_align = TextAlignMode::Start;
        style.direction = DirectionMode::Ltr;
        assert!(matches!(text_align_from_style(&style), TextAlign::Left));
        style.direction = DirectionMode::Rtl;
        assert!(matches!(text_align_from_style(&style), TextAlign::Right));

        style.text_align = TextAlignMode::End;
        assert!(matches!(text_align_from_style(&style), TextAlign::Left));
        style.direction = DirectionMode::Ltr;
        assert!(matches!(text_align_from_style(&style), TextAlign::Right));
    }

    #[test]
    fn text_align_justify_and_last_align_map_to_flow() {
        let resolver = StyleResolver::new("");
        let mut style = resolver.default_style();

        style.text_align = TextAlignMode::Justify;
        style.direction = DirectionMode::Ltr;
        assert!(matches!(text_align_from_style(&style), TextAlign::Justify));
        assert!(matches!(
            text_align_last_from_style(&style),
            Some(TextAlign::Left)
        ));

        style.direction = DirectionMode::Rtl;
        assert!(matches!(
            text_align_last_from_style(&style),
            Some(TextAlign::Right)
        ));

        style.text_align = TextAlignMode::JustifyAll;
        style.direction = DirectionMode::Ltr;
        assert!(matches!(
            text_align_last_from_style(&style),
            Some(TextAlign::Justify)
        ));

        style.text_align = TextAlignMode::Left;
        style.text_align_last = TextAlignLastMode::Justify;
        assert!(matches!(
            text_align_last_from_style(&style),
            Some(TextAlign::Justify)
        ));
    }

    #[test]
    fn element_info_marks_only_html_element_as_root() {
        let document = parse_html("<!doctype html><html><body><p>root</p></body></html>");
        let html = document.select_first("html").expect("html element");
        let body = document.select_first("body").expect("body element");
        let paragraph = document.select_first("p").expect("paragraph element");

        assert!(element_info(html.as_node(), false).is_root);
        assert!(!element_info(body.as_node(), false).is_root);
        assert!(!element_info(paragraph.as_node(), false).is_root);
    }

    #[test]
    fn html_th_scope_maps_known_values_to_pdf_scope_names() {
        assert_eq!(
            html_th_scope_to_pdf_scope(Some("col")).as_deref(),
            Some("Column")
        );
        assert_eq!(
            html_th_scope_to_pdf_scope(Some("row")).as_deref(),
            Some("Row")
        );
        assert_eq!(
            html_th_scope_to_pdf_scope(Some("colgroup")).as_deref(),
            Some("ColGroup")
        );
        assert_eq!(
            html_th_scope_to_pdf_scope(Some("rowgroup")).as_deref(),
            Some("RowGroup")
        );
        assert_eq!(html_th_scope_to_pdf_scope(Some(" ")), None);
        assert_eq!(
            html_th_scope_to_pdf_scope(Some("custom-scope")).as_deref(),
            Some("custom-scope")
        );
    }

    #[test]
    fn svg_serialization_roundtrip() {
        let html = r##"
        <html>
          <body>
            <svg width="24" height="24" viewBox="0 0 24 24">
              <path d="M12 1 C9 6 6 9 6 13 C6 17 9 20 12 20 C15 20 18 17 18 13 C18 9 15 6 12 1 Z"
                fill="#2e86d6" />
            </svg>
          </body>
        </html>
        "##;
        let doc = parse_html(html);
        let svg = doc.select_first("svg").expect("svg");
        let xml = serialize_svg_node(svg.as_node());
        let compiled = crate::svg::compile_svg(&xml, Pt::from_f32(24.0), Pt::from_f32(24.0));
        assert!(
            !compiled.is_empty(),
            "expected compiled SVG paths, got none. xml={}",
            xml
        );
    }

    #[test]
    fn svg_serialization_preserves_text_content_for_native_compilation() {
        let html = r##"
        <html><body>
          <svg width="160" height="64" viewBox="0 0 160 64">
            <text x="8" y="46" font-family="Arial, sans-serif" font-size="44" fill="#cc0000">SVG</text>
          </svg>
        </body></html>
        "##;
        let document = parse_html(html);
        let svg = document.select_first("svg").expect("svg");
        let xml = serialize_svg_node(svg.as_node());
        let compiled = crate::svg::compile_svg(&xml, Pt::from_f32(160.0), Pt::from_f32(64.0));
        assert!(
            compiled
                .iter()
                .any(|item| matches!(item, crate::svg::CompiledItem::Text(_))),
            "serialized inline SVG text should compile: {xml}"
        );
    }

    #[test]
    fn svg_serialization_preserves_native_filter_attributes() {
        let html = r##"
        <html><body>
          <svg width="80" height="40" viewBox="0 0 80 40">
            <defs><filter id="blur"><feGaussianBlur stdDeviation="2" /></filter></defs>
            <rect x="10" y="8" width="40" height="20" fill="red" filter="url(#blur)" />
          </svg>
        </body></html>
        "##;
        let document = parse_html(html);
        let svg = document.select_first("svg").expect("svg");
        let xml = serialize_svg_node(svg.as_node());
        let compiled = crate::svg::compile_svg(&xml, Pt::from_f32(80.0), Pt::from_f32(40.0));
        assert!(
            compiled
                .iter()
                .any(|item| matches!(item, crate::svg::CompiledItem::Group(_))),
            "serialized inline SVG filter should compile: {xml}"
        );
    }

    #[test]
    fn inline_children_only_excludes_img_and_svg_replaced_content() {
        let doc = parse_html(
            r##"
            <html>
              <body>
                <div id="imgbox"><img src="examples/img/full_bleed-logo_small.png" /></div>
                <div id="svbbox"><svg width="24" height="24"><rect width="24" height="24" fill="#000"/></svg></div>
              </body>
            </html>
            "##,
        );
        let resolver = StyleResolver::new("");
        let parent = resolver.default_style();
        let ancestors: Vec<ElementInfo> = Vec::new();

        let img_div = doc.select_first("#imgbox").expect("imgbox");
        let svg_div = doc.select_first("#svbbox").expect("svbbox");

        assert!(
            !inline_children_only(img_div.as_node(), &resolver, &parent, &ancestors),
            "img-only wrappers must not be flattened to text"
        );
        assert!(
            !inline_children_only(svg_div.as_node(), &resolver, &parent, &ancestors),
            "svg-only wrappers must not be flattened to text"
        );
    }

    #[test]
    fn fixed_height_block_defers_visible_taller_image_paint() {
        let document = parse_html(
            "<html><body><div class='own'><span>Ag</span><img class='asset'></div></body></html>",
        );
        let resolver = StyleResolver::new(
            ".own { height: 22px; overflow: visible; } .asset { display: inline-block; height: 24px; }",
        );
        let own = document.select_first(".own").expect("own");
        let parent = resolver.default_style();
        let info = element_info(own.as_node(), resolver.has_sibling_selectors());
        let style = resolver.compute_style(&info, &parent, None, &[]);
        let ancestors = vec![info];

        assert!(static_block_with_visible_replaced_overflow(
            own.as_node(),
            &resolver,
            &style,
            &ancestors,
        ));
    }

    #[test]
    fn inline_children_only_ignores_display_none_replaced_content() {
        let document = parse_html(
            "<html><body><div class='line'><span>Ag</span><span>Bb</span><img class='asset'></div></body></html>",
        );
        let resolver = StyleResolver::new(".asset { display: none; }");
        let parent = resolver.default_style();
        let line = document.select_first(".line").expect("line");

        assert!(inline_children_only(
            line.as_node(),
            &resolver,
            &parent,
            &[]
        ));
        assert!(inline_or_replaced_children_only(
            line.as_node(),
            &resolver,
            &parent,
            &[]
        ));
    }

    #[test]
    fn hidden_sibling_does_not_break_anonymous_table_cell_fixup() {
        let document = parse_html(
            "<html><body><div class='own'><span class='token'>A</span><span class='token'>B</span><img class='asset'></div></body></html>",
        );
        let resolver =
            StyleResolver::new(".token { display: table-cell; } .asset { display: none; }");
        let parent_style = resolver.default_style();
        let own = document.select_first(".own").expect("own");
        let mut counters = CounterState::default();

        let items = anonymous_table_cell_run_flowables(
            own.as_node(),
            &resolver,
            &parent_style,
            &[],
            &mut counters,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        )
        .expect("visible table-cell siblings should form one anonymous table");

        assert_eq!(items.len(), 1);
    }

    #[test]
    fn replaced_sibling_follows_the_anonymous_authored_cell_row() {
        let document = parse_html(
            "<html><body><div class='own'><span class='token'>A</span><span class='token'>B</span><img class='asset' alt='' src='data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAECAIAAAA8r+mnAAAAIElEQVR42mM4IScHR3I9NnDEgFNCzu0EHN1ZJQJHOCUAni4lgeO2HLIAAAAASUVORK5CYII='></div></body></html>",
        );
        let resolver = StyleResolver::new(
            ".own { height: 22px; white-space: nowrap; border-spacing: 3px; } \
             .token { display: table-cell; vertical-align: middle; } \
             .asset { display: inline-block; width: 34px; height: 24px; }",
        );
        let root = resolver.default_style();
        let own = document.select_first(".own").expect("own");
        let own_info = element_info(own.as_node(), false);
        let own_style = resolver.compute_style(&own_info, &root, None, &[]);
        let items = anonymous_table_cell_run_flowables(
            own.as_node(),
            &resolver,
            &own_style,
            std::slice::from_ref(&own_info),
            &mut CounterState::default(),
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        )
        .expect("the table-cell run should retain the trailing replaced sibling");
        assert_eq!(items.len(), 2);
        let flowable = ContainerFlowable::new_pt(
            layout_children_to_flowables(items, None),
            own_style.font_size,
            own_style.root_font_size,
        );
        let page = Size {
            width: Pt::from_f32(200.0),
            height: Pt::from_f32(80.0),
        };
        let size = flowable.wrap(page.width, page.height);
        let mut canvas = Canvas::new(page);
        flowable.draw(&mut canvas, Pt::ZERO, Pt::ZERO, size.width, size.height);
        let page = &canvas.finish().pages[0];
        let text_position = |needle: &str| {
            page.commands.iter().find_map(|command| match command {
                Command::DrawString { text, x, y, .. } if text == needle => Some((*x, *y)),
                _ => None,
            })
        };
        let image_position = page.commands.iter().find_map(|command| match command {
            Command::DrawImage { x, y, .. } => Some((*x, *y)),
            _ => None,
        });
        let first = text_position("A").expect("first cell text");
        let second = text_position("B").expect("second cell text");
        let image = image_position.expect("trailing image");

        assert!(first.0 < second.0, "authored cells must share one row");
        assert!(
            image.1 > first.1.max(second.1),
            "the improper sibling follows the synthesized table row"
        );
    }

    #[test]
    fn anonymous_table_cells_compile_generated_counter_content() {
        let document = parse_html(
            "<html><body><div class='own'><span class='token'>Ag</span><span class='token'>Bb</span></div></body></html>",
        );
        let resolver = StyleResolver::new(
            ".own { counter-reset: pair-item; } \
             .token { display: table-cell; counter-increment: pair-item; } \
             .token::before { content: counter(pair-item) '.'; color: #d62828; }",
        );
        let root = resolver.default_style();
        let own = document.select_first(".own").expect("own");
        let own_info = element_info(own.as_node(), false);
        let own_style = resolver.compute_style(&own_info, &root, None, &[]);
        let mut counters = CounterState::default();
        apply_style_counters(&own_style, &mut counters);
        let mut items = anonymous_table_cell_run_flowables(
            own.as_node(),
            &resolver,
            &own_style,
            std::slice::from_ref(&own_info),
            &mut counters,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        )
        .expect("generated table-cell siblings should retain anonymous table fixup");
        let flowable = match items.remove(0) {
            LayoutItem::Block { flowable, .. } | LayoutItem::Inline { flowable, .. } => flowable,
        };
        let page = Size {
            width: Pt::from_f32(200.0),
            height: Pt::from_f32(80.0),
        };
        let mut canvas = Canvas::new(page);
        flowable.draw(&mut canvas, Pt::ZERO, Pt::ZERO, page.width, page.height);
        let red = Color::rgb(214.0 / 255.0, 40.0 / 255.0, 40.0 / 255.0);

        assert!(
            canvas.finish().pages[0]
                .commands
                .iter()
                .any(|command| matches!(command, Command::SetFillColor(color) if *color == red))
        );
    }

    #[test]
    fn anonymous_table_cell_fixup_keeps_inherited_border_spacing() {
        let document = parse_html(
            "<html><body><div class='table'><div class='own'><span class='token'>A</span><span class='token'>B</span></div></div></body></html>",
        );
        let resolver =
            StyleResolver::new(".table { border-spacing: 3px; } .token { display: table-cell; }");
        let root = resolver.default_style();
        let table = document.select_first(".table").expect("table");
        let table_info = element_info(table.as_node(), false);
        let table_style = resolver.compute_style(&table_info, &root, None, &[]);
        let own = document.select_first(".own").expect("own");
        let own_info = element_info(own.as_node(), false);
        let own_style = resolver.compute_style(
            &own_info,
            &table_style,
            None,
            std::slice::from_ref(&table_info),
        );
        assert_eq!(own_style.border_spacing, table_style.border_spacing);
        let ancestors = vec![table_info, own_info];

        let mut counters = CounterState::default();
        let items = anonymous_table_cell_run_flowables(
            own.as_node(),
            &resolver,
            &own_style,
            &ancestors,
            &mut counters,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        )
        .expect("table-cell siblings should form one anonymous table");
        let width = match &items[0] {
            LayoutItem::Block { flowable, .. } | LayoutItem::Inline { flowable, .. } => {
                flowable.intrinsic_width().expect("intrinsic table width")
            }
        };

        // Two cells generate three 3px border-spacing intervals, so the
        // anonymous table must be 9 CSS px wider than its zero-spacing peer.
        let mut zero_style = own_style.clone();
        zero_style.border_spacing = BorderSpacingSpec::zero();
        let zero_items = anonymous_table_cell_run_flowables(
            own.as_node(),
            &resolver,
            &zero_style,
            &ancestors,
            &mut CounterState::default(),
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        )
        .expect("zero-spacing comparison table");
        let zero_width = match &zero_items[0] {
            LayoutItem::Block { flowable, .. } | LayoutItem::Inline { flowable, .. } => {
                flowable.intrinsic_width().expect("intrinsic table width")
            }
        };
        assert_eq!(width - zero_width, Pt::from_f32(6.75));
    }

    #[test]
    fn improper_table_child_sequence_gets_outer_border_spacing() {
        let resolver = StyleResolver::new(".table { border-spacing: 3px; }");
        let root = resolver.default_style();
        let style = resolver.compute_style(
            &element_info(
                parse_html("<div class='table'></div>")
                    .select_first(".table")
                    .expect("table")
                    .as_node(),
                false,
            ),
            &root,
            None,
            &[],
        );
        let child = ContainerFlowable::new_pt(Vec::new(), style.font_size, style.root_font_size)
            .with_width(LengthSpec::Absolute(Pt::from_f32(10.0)))
            .with_height(LengthSpec::Absolute(Pt::from_f32(5.0)));

        let wrapped = anonymous_table_sequence_with_spacing(
            vec![Box::new(child) as Box<dyn Flowable>],
            &style,
        );

        assert_eq!(wrapped.len(), 1);
        assert_eq!(wrapped[0].intrinsic_width(), Some(Pt::from_f32(14.5)));
        assert_eq!(
            wrapped[0]
                .wrap(Pt::from_f32(100.0), Pt::from_f32(100.0))
                .height,
            Pt::from_f32(9.5)
        );
    }

    #[test]
    fn generated_table_pseudos_participate_in_anonymous_table_fixup() {
        let document = parse_html(
            "<html><body><div class='table'><div class='own'>AB</div></div></body></html>",
        );
        let resolver = StyleResolver::new(
            ".table { display: table; height: 20px; border-spacing: 3px; box-sizing: border-box; } \
             .table::before { content: '['; } \
             .table::after { content: ']'; }",
        );
        let root = resolver.default_style();
        let table = document.select_first(".table").expect("table");
        let mut counters = CounterState::default();
        let mut ancestors = Vec::new();
        let items = node_to_flowables(
            table.as_node(),
            &resolver,
            &root,
            &mut ancestors,
            &mut counters,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );
        let flowable = match items.as_slice() {
            [LayoutItem::Block { flowable, .. }] => flowable,
            _ => panic!("expected one generated table flowable"),
        };
        let page = Size {
            width: Pt::from_f32(200.0),
            height: Pt::from_f32(200.0),
        };
        let mut canvas = Canvas::new(page);
        let size = flowable.wrap(page.width, page.height);
        assert!(
            size.height > Pt::from_f32(15.0),
            "generated anonymous rows must expand a table beyond its authored minimum height: {size:?}"
        );
        flowable.draw(&mut canvas, Pt::ZERO, Pt::ZERO, size.width, size.height);
        let rendered = canvas.finish();
        let texts = rendered.pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::DrawString { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(texts.contains(&"["), "missing table ::before in {texts:?}");
        assert!(texts.contains(&"]"), "missing table ::after in {texts:?}");
    }

    #[test]
    fn anonymous_table_height_minimum_is_bounded_by_max_height() {
        let document = parse_html(
            "<html><body><div class='table'><div class='own'></div></div></body></html>",
        );
        let resolver = StyleResolver::new(
            ".table { display: table; height: 68px; max-height: 58px; \
             border-spacing: 0; box-sizing: border-box; } \
             .own { height: 10px; }",
        );
        let root = resolver.default_style();
        let table = document.select_first(".table").expect("table");
        let mut counters = CounterState::default();
        let mut ancestors = Vec::new();
        let items = node_to_flowables(
            table.as_node(),
            &resolver,
            &root,
            &mut ancestors,
            &mut counters,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );
        let flowable = match items.as_slice() {
            [LayoutItem::Block { flowable, .. }] => flowable,
            _ => panic!("expected one anonymous table flowable"),
        };

        assert_eq!(
            flowable
                .wrap(Pt::from_f32(200.0), Pt::from_f32(200.0))
                .height,
            Pt::from_f32(43.5),
            "58 CSS px should cap the table's 68 CSS px minimum height"
        );
    }

    #[test]
    fn simple_br_content_uses_one_paragraph_with_forced_line_breaks() {
        let document = parse_html("<html><body><p>one<br>two<br>three</p></body></html>");
        let resolver = StyleResolver::new("");
        let parent = resolver.default_style();
        let paragraph = document.select_first("p").expect("paragraph");

        assert!(inline_children_only(
            paragraph.as_node(),
            &resolver,
            &parent,
            &[]
        ));
        assert!(inline_or_replaced_children_only(
            paragraph.as_node(),
            &resolver,
            &parent,
            &[]
        ));
        assert_eq!(
            extract_text(paragraph.as_node(), WhiteSpaceMode::Normal),
            "one\ntwo\nthree"
        );
    }

    #[test]
    fn body_box_model_contributes_padding_to_story_height() {
        let resolver = StyleResolver::new(
            "* { box-sizing: border-box; margin: 0; } body { padding: 16px; } .box { height: 10px; }",
        );
        let story = html_to_story_with_resolver_and_fonts_and_report(
            "<!doctype html><html><body><div class='box'></div></body></html>",
            &resolver,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );

        assert_eq!(story.len(), 1);
        assert_eq!(
            story[0]
                .wrap(Pt::from_f32(200.0), Pt::from_f32(1_000.0))
                .height,
            Pt::from_f32(31.5),
            "10px content plus 16px body padding on both block-axis edges"
        );
    }

    #[test]
    fn html_absolute_siblings_paint_in_positive_z_index_order() {
        let resolver = StyleResolver::new(
            "* { box-sizing: border-box; margin: 0; padding: 0; } \
             html, body { width: 120px; height: 120px; } \
             .panel { position: absolute; z-index: 0; width: 100px; height: 100px; overflow: hidden; border: 4px solid #111827; background: #e5e7eb; } \
             .panel::before { content: ''; display: block; position: absolute; z-index: 3; width: 80px; height: 80px; background: #ef233c; } \
             .low { position: absolute; z-index: 2; width: 80px; height: 80px; background: #2563eb; }",
        );
        let story = html_to_story_with_resolver_and_fonts_and_report(
            "<!doctype html><html><body><section class='panel'><div class='low'></div></section></body></html>",
            &resolver,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );
        let page = Size {
            width: Pt::from_f32(90.0),
            height: Pt::from_f32(90.0),
        };
        let mut canvas = Canvas::new(page);
        for flowable in story {
            flowable.draw(&mut canvas, Pt::ZERO, Pt::ZERO, page.width, page.height);
        }
        let low = Color::rgb(37.0 / 255.0, 99.0 / 255.0, 235.0 / 255.0);
        let high = Color::rgb(239.0 / 255.0, 35.0 / 255.0, 60.0 / 255.0);
        let paint_order = canvas.finish().pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::SetFillColor(color) if *color == low => Some("low"),
                Command::SetFillColor(color) if *color == high => Some("high"),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(paint_order, vec!["low", "high"]);
    }

    #[test]
    fn absolute_replaced_sibling_retains_positive_z_index() {
        let resolver = StyleResolver::new(
            "* { box-sizing: border-box; margin: 0; padding: 0; } \
             html, body { width: 90px; height: 90px; } \
             .panel { position: absolute; z-index: 0; width: 80px; height: 80px; overflow: hidden; } \
             .high { position: absolute; z-index: 3; width: 64px; height: 64px; } \
             .low { position: absolute; z-index: 2; width: 64px; height: 64px; background: #2563eb; }",
        );
        let story = html_to_story_with_resolver_and_fonts_and_report(
            "<!doctype html><html><body><section class='panel'><img class='high' alt='' src='data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAECAIAAAA8r+mnAAAAIElEQVR42mM4IScHR3I9NnDEgFNCzu0EHN1ZJQJHOCUAni4lgeO2HLIAAAAASUVORK5CYII='><div class='low'></div></section></body></html>",
            &resolver,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );
        let page = Size {
            width: Pt::from_f32(90.0),
            height: Pt::from_f32(90.0),
        };
        let mut canvas = Canvas::new(page);
        for flowable in story {
            flowable.draw(&mut canvas, Pt::ZERO, Pt::ZERO, page.width, page.height);
        }
        let low = Color::rgb(37.0 / 255.0, 99.0 / 255.0, 235.0 / 255.0);
        let rendered = canvas.finish();
        let paint_order = rendered.pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::SetFillColor(color) if *color == low => Some("low"),
                Command::DrawImage { .. } => Some("high"),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(paint_order, vec!["low", "high"]);
    }

    #[test]
    fn overlapping_grid_items_use_order_modified_paint_order() {
        std::thread::Builder::new()
            .name("grid-order-paint-regression".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                let resolver = StyleResolver::new(
            "* { box-sizing: border-box; margin: 0; padding: 0; } \
             html, body { width: 90px; height: 90px; } \
             .grid { position: absolute; display: grid; grid-template: 80px / 80px; width: 80px; height: 80px; } \
             .item { grid-area: 1 / 1; width: 80px; height: 80px; } \
             .high { order: 2; background: #ef233c; } \
             .low { order: 1; background: #2563eb; }",
        );
        let story = html_to_story_with_resolver_and_fonts_and_report(
            "<!doctype html><html><body><section class='grid'><div class='item high'></div><div class='item low'></div></section></body></html>",
            &resolver,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );
        let page = Size {
            width: Pt::from_f32(90.0),
            height: Pt::from_f32(90.0),
        };
        let mut canvas = Canvas::new(page);
        for flowable in story {
            flowable.draw(&mut canvas, Pt::ZERO, Pt::ZERO, page.width, page.height);
        }
        let low = Color::rgb(37.0 / 255.0, 99.0 / 255.0, 235.0 / 255.0);
        let high = Color::rgb(239.0 / 255.0, 35.0 / 255.0, 60.0 / 255.0);
        let paint_order = canvas.finish().pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::SetFillColor(color) if *color == low => Some("low"),
                Command::SetFillColor(color) if *color == high => Some("high"),
                _ => None,
            })
            .collect::<Vec<_>>();

                assert_eq!(paint_order, vec!["low", "high"]);
            })
            .expect("spawn grid paint-order regression")
            .join()
            .expect("grid paint-order regression");
    }

    #[test]
    fn inline_children_only_respects_display_block_on_span_children() {
        let doc = parse_html(
            r##"
            <html>
              <body>
                <h1 class="title"><span class="line">A</span><span class="line">B</span></h1>
              </body>
            </html>
            "##,
        );
        let resolver = StyleResolver::new(".title > .line { display: block; }");
        let root = resolver.default_style();

        let h1 = doc.select_first("h1.title").expect("title");
        let h1_info = element_info(h1.as_node(), resolver.has_sibling_selectors());
        let h1_style = resolver.compute_style(&h1_info, &root, None, &[]);

        let span = h1
            .as_node()
            .children()
            .find(|child| child.as_element().is_some())
            .expect("span child");
        let span_info = element_info(&span, resolver.has_sibling_selectors());
        let span_style = resolver.compute_style(&span_info, &h1_style, None, &[h1_info.clone()]);
        assert_eq!(
            span_style.display,
            DisplayMode::Block,
            "expected .title > .line selector to force span display:block"
        );

        let ancestors = vec![h1_info];
        assert!(
            !inline_children_only(h1.as_node(), &resolver, &h1_style, &ancestors),
            "h1 with span display:block children must not take inline flatten path"
        );
    }

    #[test]
    fn inline_children_only_rejects_styled_inline_descendants() {
        let document = parse_html(
            r##"
            <html><body>
              <div class="line"><span class="styled">Baseline Hxy</span></div>
              <div class="plain"><span>Plain text</span></div>
              <div class="paint"><span>Painted text</span></div>
            </body></html>
            "##,
        );
        let resolver = StyleResolver::new(
            ".styled { font-size: 40px; color: #102a43; } .paint > span { background: red; }",
        );
        let root = resolver.default_style();

        let check = |selector: &str| {
            let element = document.select_first(selector).expect("container");
            let info = element_info(element.as_node(), resolver.has_sibling_selectors());
            let style = resolver.compute_style(&info, &root, None, &[]);
            let ancestors = vec![info];
            inline_children_only(element.as_node(), &resolver, &style, &ancestors)
        };

        assert!(
            !check(".line"),
            "font and color changes must preserve a styled run"
        );
        assert!(
            check(".plain"),
            "a semantically transparent span may use the fast path"
        );
        assert!(
            !check(".paint"),
            "inline box paint must not be flattened away"
        );
    }

    #[test]
    fn absolutely_positioned_inline_keeps_its_out_of_flow_wrapper() {
        let document =
            parse_html("<html><body><div><span class='token'>Bb</span></div></body></html>");
        let resolver =
            StyleResolver::new(".token { position: absolute; right: 4px; bottom: 4px; }");
        let parent_style = resolver.default_style();
        let token = document.select_first(".token").expect("token");
        let mut ancestors = Vec::new();
        let mut counters = CounterState::default();
        let items = node_to_flowables(
            token.as_node(),
            &resolver,
            &parent_style,
            &mut ancestors,
            &mut counters,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );

        assert_eq!(items.len(), 1);
        let flowable = match &items[0] {
            LayoutItem::Block { flowable, .. } | LayoutItem::Inline { flowable, .. } => flowable,
        };
        assert!(
            flowable.out_of_flow(),
            "position:absolute on an inline box must survive transparent-inline handling"
        );
    }

    #[test]
    fn absolute_terminal_baseline_rounding_uses_resolved_line_height_phase() {
        let document = parse_html(
            "<html><body><span class='normal'>A</span><span class='early'>B</span><span class='late'>C</span></body></html>",
        );
        let resolver = StyleResolver::new(
            ".normal, .early, .late { position: absolute; right: 4px; bottom: 4px; } \
             .early { line-height: 1.2; } \
             .late { line-height: 1.35; }",
        );
        let root = resolver.default_style();

        let computed = |selector: &str| {
            let node = document.select_first(selector).expect("absolute token");
            let info = element_info(node.as_node(), resolver.has_sibling_selectors());
            resolver.compute_style(&info, &root, None, &[])
        };

        assert!(absolute_needs_terminal_baseline_rounding(&computed(
            ".normal"
        )));
        assert!(absolute_needs_terminal_baseline_rounding(&computed(
            ".early"
        )));
        assert!(
            !absolute_needs_terminal_baseline_rounding(&computed(".late")),
            "a late fractional line-height phase has already rounded upward"
        );
    }

    #[test]
    fn absolute_inline_does_not_destroy_its_inline_containers_intrinsic_width() {
        let document = parse_html(
            "<html><body><div class='own'><span>Ag</span><span class='token'>Bb</span></div></body></html>",
        );
        let resolver = StyleResolver::new(
            ".own { display: block; height: 22px; } \
             .token { position: absolute; right: 4px; bottom: 4px; }",
        );
        let parent_style = resolver.default_style();
        let own = document.select_first(".own").expect("own");
        let mut ancestors = Vec::new();
        let mut counters = CounterState::default();
        let items = node_to_flowables(
            own.as_node(),
            &resolver,
            &parent_style,
            &mut ancestors,
            &mut counters,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );

        assert_eq!(items.len(), 1);
        let flowable = match &items[0] {
            LayoutItem::Block { flowable, .. } | LayoutItem::Inline { flowable, .. } => flowable,
        };
        assert!(
            flowable.intrinsic_width().is_some(),
            "an absolute inline child must be excluded instead of making intrinsic sizing unknown"
        );
    }

    #[test]
    fn fixed_height_does_not_expand_anonymous_lines_around_block_children() {
        let resolver = StyleResolver::new("");
        let mut style = resolver.default_style();
        style.height = LengthSpec::Absolute(Pt::from_f32(72.0));
        let inline = || LayoutItem::Inline {
            flowable: Box::new(Spacer::new_pt(Pt::from_f32(12.0))) as Box<dyn Flowable>,
            valign: VerticalAlign::Baseline,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            width_spec: None,
            order: 0,
        };
        let block = LayoutItem::Block {
            flowable: Box::new(Spacer::new_pt(Pt::from_f32(12.0))) as Box<dyn Flowable>,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            width_spec: None,
            order: 0,
        };

        assert_eq!(
            forced_inline_line_height(&[inline()], &style),
            Some(Pt::from_f32(72.0))
        );
        assert_eq!(
            forced_inline_line_height(&[inline(), block, inline()], &style),
            None,
            "anonymous inline boxes around a block child use their line-height, not the outer height"
        );
        assert!(mixes_inline_and_block_children(&[
            inline(),
            LayoutItem::Block {
                flowable: Box::new(Spacer::new_pt(Pt::from_f32(12.0))) as Box<dyn Flowable>,
                flex_grow: 0.0,
                flex_shrink: 1.0,
                width_spec: None,
                order: 0,
            },
            inline(),
        ]));
    }

    #[test]
    fn styled_inline_descendant_contributes_its_font_size_to_the_line_box() {
        let resolver = StyleResolver::new(
            "html { font-family: ParitySans; line-height: 1.5; } \
             .line { width: 300px; border-bottom: 2px solid black; } \
             .t { font-family: ParitySans; font-size: 40px; line-height: 1; color: #102a43; }",
        );
        let inheritance_document = parse_html(
            "<html><body><div class='line'><span class='t'>Baseline Hxy</span></div></body></html>",
        );
        let html_element = inheritance_document.select_first("html").unwrap();
        let body_element = inheritance_document.select_first("body").unwrap();
        let line_element = inheritance_document.select_first(".line").unwrap();
        let html_info = element_info(html_element.as_node(), false);
        let body_info = element_info(body_element.as_node(), false);
        let line_info = element_info(line_element.as_node(), false);
        let root_style = resolver.compute_style(&html_info, &resolver.default_style(), None, &[]);
        let body_style = resolver.compute_style(
            &body_info,
            &root_style,
            None,
            std::slice::from_ref(&html_info),
        );
        let line_style =
            resolver.compute_style(&line_info, &body_style, None, &[html_info, body_info]);
        assert_eq!(line_style.font_name.as_ref(), "ParitySans");
        assert_eq!(line_style.to_text_style().line_height, Pt::from_f32(18.0));
        let story = html_to_story_with_resolver_and_fonts_and_report(
            "<html><body><div class='line'><span class='t'>Baseline Hxy</span></div></body></html>",
            &resolver,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );

        assert_eq!(story.len(), 1);
        let size = story[0].wrap(Pt::from_f32(288.0), Pt::from_f32(1_000.0));
        assert_eq!(
            size.height,
            Pt::from_f32(31.5),
            "40px text plus a 2px border should form a 42px box with Base-14 metrics"
        );
    }

    #[test]
    fn direct_text_block_does_not_reapply_inline_top_overflow() {
        let resolver = StyleResolver::new(
            "* { margin: 0; padding: 0; } \
             html, body, div { display: block; } \
             .t { font-family: Helvetica; font-size: 42px; line-height: 1; }",
        );
        let story = html_to_story_with_resolver_and_fonts_and_report(
            "<html><body><div class='t'>Direct</div></body></html>",
            &resolver,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );
        assert_eq!(story.len(), 1);

        let page = Size {
            width: Pt::from_f32(200.0),
            height: Pt::from_f32(80.0),
        };
        let size = story[0].wrap(page.width, page.height);
        let mut canvas = Canvas::new(page);
        story[0].draw(&mut canvas, Pt::ZERO, Pt::ZERO, size.width, size.height);
        let draw_y = canvas.finish().pages[0]
            .commands
            .iter()
            .find_map(|command| match command {
                Command::DrawString { text, y, .. } if text == "Direct" => Some(*y),
                _ => None,
            })
            .expect("direct text draw command");

        assert_eq!(
            draw_y,
            Pt::from_f32(-6.75),
            "the standalone block owns its line origin; inline-union overflow must not shift it down"
        );
    }

    #[test]
    fn direct_text_shadow_retains_its_shadow_form_line_phase() {
        let resolver = StyleResolver::new(
            "* { margin: 0; padding: 0; } \
             html, body, div { display: block; } \
             .t { font-family: Helvetica; font-size: 42px; line-height: 1; \
                  text-shadow: 1px 1px 0 #000; }",
        );
        let story = html_to_story_with_resolver_and_fonts_and_report(
            "<html><body><div class='t'>Shadow</div></body></html>",
            &resolver,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );
        assert_eq!(story.len(), 1);

        let page = Size {
            width: Pt::from_f32(200.0),
            height: Pt::from_f32(80.0),
        };
        let size = story[0].wrap(page.width, page.height);
        let mut canvas = Canvas::new(page);
        story[0].draw(&mut canvas, Pt::ZERO, Pt::ZERO, size.width, size.height);
        let draw_y = canvas.finish().pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::DrawString { text, y, .. } if text == "Shadow" => Some(*y),
                _ => None,
            })
            .last()
            .expect("shadowed direct text draw command");

        assert_eq!(
            draw_y,
            Pt::from_f32(-6.0),
            "the shadow form keeps the inline-union phase shared by its source glyph run"
        );
    }

    #[test]
    fn semantic_container_direct_text_is_emitted_as_real_paragraph_content() {
        let resolver = StyleResolver::new(
            "* { margin: 0; padding: 0; } html, body, footer { display: block; }",
        );
        let story = html_to_story_with_resolver_and_fonts_and_report(
            "<html><body><footer>Questions? Call support.</footer></body></html>",
            &resolver,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );
        let page = Size {
            width: Pt::from_f32(300.0),
            height: Pt::from_f32(120.0),
        };
        let mut canvas = Canvas::new(page);
        let mut y = Pt::ZERO;
        for flowable in story {
            let size = flowable.wrap(page.width, page.height);
            flowable.draw(&mut canvas, Pt::ZERO, y, page.width, size.height);
            y += size.height;
        }
        let commands = &canvas.finish().pages[0].commands;
        let text_index = commands
            .iter()
            .position(|command| {
                matches!(command, Command::DrawString { text, .. } if text == "Questions? Call support.")
            })
            .expect("footer text command");
        assert!(
            commands[..text_index].iter().any(|command| {
                matches!(command, Command::BeginTag { role, .. } if role == "P")
            })
        );
        assert!(
            commands[text_index + 1..]
                .iter()
                .any(|command| matches!(command, Command::EndTag))
        );
    }

    #[test]
    fn screen_reader_only_text_emits_actual_text_without_paint_or_flow_height() {
        let resolver =
            StyleResolver::new("* { margin: 0; padding: 0; } html, body, p { display: block; }");
        let compile = |html: &str| {
            let story = html_to_story_with_resolver_and_fonts_and_report(
                html, &resolver, None, None, None, false, false, None, None,
            );
            let page = Size {
                width: Pt::from_f32(300.0),
                height: Pt::from_f32(120.0),
            };
            let mut canvas = Canvas::new(page);
            let mut y = Pt::ZERO;
            for flowable in &story {
                let size = flowable.wrap(page.width, page.height);
                flowable.draw(&mut canvas, Pt::ZERO, y, page.width, size.height);
                y += size.height;
            }
            (y, canvas.finish().pages[0].commands.clone())
        };

        let (baseline_height, _) =
            compile("<html><body><p>Visible before</p><p>Visible after</p></body></html>");
        let (authored_height, commands) = compile(
            "<html><body><p>Visible before</p><span data-fb-a11y-only='true' style='display:none'>Account values are shown in the following table.</span><p>Visible after</p></body></html>",
        );

        assert_eq!(authored_height, baseline_height);
        assert!(commands.iter().any(|command| matches!(
            command,
            Command::BeginTagActualText { role, actual_text, .. }
                if role == "Span" && actual_text == "Account values are shown in the following table."
        )));
        assert!(!commands.iter().any(|command| matches!(
            command,
            Command::DrawString { text, .. }
                if text.contains("Account values are shown")
        )));
        let before = commands
            .iter()
            .position(|command| matches!(command, Command::DrawString { text, .. } if text.contains("Visible before")))
            .expect("visible predecessor paint");
        let semantic = commands
            .iter()
            .position(|command| matches!(command, Command::BeginTagActualText { .. }))
            .expect("screen-reader-only semantic carrier");
        let after = commands
            .iter()
            .position(|command| matches!(command, Command::DrawString { text, .. } if text.contains("Visible after")))
            .expect("visible successor paint");
        assert!(before < semantic && semantic < after);

        let (inline_baseline_height, _) =
            compile("<html><body><p>Visible before visible after</p></body></html>");
        let (inline_height, inline_commands) = compile(
            "<html><body><p>Visible before <span data-fb-a11y-only='true'>Inline semantic context</span> visible after</p></body></html>",
        );
        assert_eq!(inline_height, inline_baseline_height);
        assert!(
            inline_commands.iter().any(|command| matches!(
                command,
                Command::BeginTagActualText { actual_text, .. }
                    if actual_text == "Inline semantic context"
            )),
            "inline commands: {inline_commands:#?}"
        );
        assert!(!inline_commands.iter().any(|command| matches!(
            command,
            Command::DrawString { text, .. } if text.contains("Inline semantic context")
        )));
        let inline_before = inline_commands
            .iter()
            .position(|command| matches!(command, Command::DrawString { text, .. } if text.contains("Visible before")))
            .expect("inline visible predecessor paint");
        let inline_semantic = inline_commands
            .iter()
            .position(|command| matches!(command, Command::BeginTagActualText { .. }))
            .expect("inline semantic carrier");
        let inline_after = inline_commands
            .iter()
            .position(|command| matches!(command, Command::DrawString { text, .. } if text.contains("visible after")))
            .expect("inline visible successor paint");
        assert!(inline_before < inline_semantic && inline_semantic < inline_after);
    }

    #[test]
    fn auto_width_inline_blocks_shrink_to_fit_on_the_same_line() {
        let resolver = StyleResolver::new(
            "* { margin: 0; padding: 0; } \
             html, body, p { display: block; } \
             .row { width: 300px; font-size: 12px; line-height: 1; } \
             .label { display: inline-block; width: 100px; } \
             .value { display: inline-block; }",
        );
        let story = html_to_story_with_resolver_and_fonts_and_report(
            "<html><body><p class='row'><span class='label'>Account:</span><span class='value'>7392</span></p></body></html>",
            &resolver,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
        );

        let height = story
            .iter()
            .map(|flowable| {
                flowable
                    .wrap(Pt::from_f32(300.0), Pt::from_f32(1_000.0))
                    .height
            })
            .fold(Pt::ZERO, |sum, value| sum + value);

        assert_eq!(
            height,
            Pt::from_f32(9.75),
            "the auto-width value should shrink to its text instead of occupying a second line"
        );
    }

    #[test]
    fn subscript_baseline_shift_expands_the_inline_line_box() {
        let base_css = "* { margin: 0; } html, body, div { display: block; } \
                        sub { display: inline; font-size: 83%; } \
                        .t { font-size: 28px; line-height: 1.4; }";
        let height = |vertical_align: &str| {
            let resolver = StyleResolver::new(&format!(
                "{base_css} sub {{ vertical-align: {vertical_align}; }}"
            ));
            let story = html_to_story_with_resolver_and_fonts_and_report(
                "<html><body><div class='t'>H<sub>2</sub>O</div></body></html>",
                &resolver,
                None,
                None,
                None,
                false,
                false,
                None,
                None,
            );
            story
                .iter()
                .map(|flowable| {
                    flowable
                        .wrap(Pt::from_f32(288.0), Pt::from_f32(1_000.0))
                        .height
                })
                .fold(Pt::ZERO, |sum, value| sum + value)
        };

        assert!(height("sub") > height("baseline"));
    }

    #[test]
    fn svg_auto_dimensions_use_viewbox_when_present() {
        let resolver = StyleResolver::new("");
        let style = resolver.default_style();
        let (w, h) = resolve_svg_dimensions(None, None, None, None, Some("0 0 220 120"), &style);
        assert!(
            approx_eq_pt(w, 165.0) && approx_eq_pt(h, 90.0),
            "expected viewBox fallback to 220x120px -> 165x90pt, got {}x{}",
            w.to_f32(),
            h.to_f32()
        );
    }

    #[test]
    fn svg_auto_dimensions_do_not_collapse_to_single_point() {
        let resolver = StyleResolver::new("");
        let style = resolver.default_style();
        let (w, h) = resolve_svg_dimensions(None, None, None, None, None, &style);
        assert!(
            w > Pt::from_f32(1.0) && h > Pt::from_f32(1.0),
            "expected non-trivial default SVG size, got {}x{}",
            w.to_f32(),
            h.to_f32()
        );
    }

    #[test]
    fn svg_single_dimension_uses_viewbox_aspect_ratio() {
        let resolver = StyleResolver::new("");
        let style = resolver.default_style();
        let (w, h) = resolve_svg_dimensions(
            Some(Pt::from_f32(165.0)),
            None,
            None,
            None,
            Some("0 0 220 120"),
            &style,
        );
        assert!(
            approx_eq_pt(w, 165.0) && approx_eq_pt(h, 90.0),
            "expected inferred height from viewBox ratio, got {}x{}",
            w.to_f32(),
            h.to_f32()
        );
    }

    #[test]
    fn load_svg_from_data_uri_image_source() {
        let xml = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 4 4'><rect width='4' height='4' fill='#000'/></svg>";
        let payload = crate::base64::encode_standard(xml.as_bytes());
        let uri = format!("data:image/svg+xml;base64,{payload}");
        let decoded = load_svg_xml_from_image_source(None, &uri).expect("svg xml from data uri");
        assert!(
            decoded.contains("<svg") && decoded.contains("<rect"),
            "expected decoded inline SVG xml, got: {decoded}"
        );
    }

    #[test]
    fn svg_image_intrinsic_dimensions_use_root_size() {
        let xml = "<svg xmlns='http://www.w3.org/2000/svg' width='80' height='40' viewBox='0 0 80 40'></svg>";
        let (width, height) = svg_image_intrinsic_dimensions(xml).expect("intrinsic size");
        assert!(approx_eq_pt(width, 60.0));
        assert!(approx_eq_pt(height, 30.0));
    }

    #[test]
    fn non_svg_data_uri_image_source_is_not_treated_as_svg() {
        let payload = crate::base64::encode_standard([0u8, 1u8, 2u8]);
        let uri = format!("data:image/png;base64,{payload}");
        assert!(
            load_svg_xml_from_image_source(None, &uri).is_none(),
            "png data uri must stay on raster image path"
        );
    }

    #[test]
    fn japanese_longhand_markers_follow_additive_tables() {
        assert_eq!(
            additive_or_cjk_decimal_list_marker(11, JAPANESE_INFORMAL_ADDITIVE, 9999),
            "\u{5341}\u{4e00}"
        );
        assert_eq!(
            additive_or_cjk_decimal_list_marker(101, JAPANESE_INFORMAL_ADDITIVE, 9999),
            "\u{767e}\u{4e00}"
        );
        assert_eq!(
            additive_or_cjk_decimal_list_marker(6001, JAPANESE_INFORMAL_ADDITIVE, 9999),
            "\u{516d}\u{5343}\u{4e00}"
        );
        assert_eq!(
            additive_or_cjk_decimal_list_marker(11, JAPANESE_FORMAL_ADDITIVE, 9999),
            "\u{58f1}\u{62fe}\u{58f1}"
        );
        assert_eq!(
            additive_or_cjk_decimal_list_marker(101, JAPANESE_FORMAL_ADDITIVE, 9999),
            "\u{58f1}\u{767e}\u{58f1}"
        );
        assert_eq!(
            additive_or_cjk_decimal_list_marker(6001, JAPANESE_FORMAL_ADDITIVE, 9999),
            "\u{516d}\u{9621}\u{58f1}"
        );
        assert_eq!(
            additive_or_cjk_decimal_list_marker(10000, JAPANESE_INFORMAL_ADDITIVE, 9999),
            "\u{4e00}\u{3007}\u{3007}\u{3007}\u{3007}"
        );
    }

    #[test]
    fn korean_longhand_markers_follow_additive_tables() {
        assert_eq!(
            additive_or_cjk_decimal_list_marker(11, KOREAN_HANGUL_FORMAL_ADDITIVE, 9999),
            "\u{c77c}\u{c2ed}\u{c77c}"
        );
        assert_eq!(
            additive_or_cjk_decimal_list_marker(101, KOREAN_HANGUL_FORMAL_ADDITIVE, 9999),
            "\u{c77c}\u{bc31}\u{c77c}"
        );
        assert_eq!(
            additive_or_cjk_decimal_list_marker(6001, KOREAN_HANGUL_FORMAL_ADDITIVE, 9999),
            "\u{c721}\u{cc9c}\u{c77c}"
        );
        assert_eq!(
            additive_or_cjk_decimal_list_marker(11, KOREAN_HANJA_INFORMAL_ADDITIVE, 9999),
            "\u{5341}\u{4e00}"
        );
        assert_eq!(
            additive_or_cjk_decimal_list_marker(6001, KOREAN_HANJA_INFORMAL_ADDITIVE, 9999),
            "\u{516d}\u{5343}\u{4e00}"
        );
        assert_eq!(
            additive_or_cjk_decimal_list_marker(11, KOREAN_HANJA_FORMAL_ADDITIVE, 9999),
            "\u{58f9}\u{62fe}\u{58f9}"
        );
        assert_eq!(
            additive_or_cjk_decimal_list_marker(6001, KOREAN_HANJA_FORMAL_ADDITIVE, 9999),
            "\u{516d}\u{4edf}\u{58f9}"
        );
    }

    #[test]
    fn chinese_longhand_markers_follow_zero_collapse_algorithm() {
        assert_eq!(
            chinese_longhand_list_marker(11, CHINESE_INFORMAL_DIGITS, CHINESE_INFORMAL_UNITS, true),
            "\u{5341}\u{4e00}"
        );
        assert_eq!(
            chinese_longhand_list_marker(
                101,
                CHINESE_INFORMAL_DIGITS,
                CHINESE_INFORMAL_UNITS,
                true
            ),
            "\u{4e00}\u{767e}\u{96f6}\u{4e00}"
        );
        assert_eq!(
            chinese_longhand_list_marker(
                110,
                CHINESE_INFORMAL_DIGITS,
                CHINESE_INFORMAL_UNITS,
                true
            ),
            "\u{4e00}\u{767e}\u{4e00}\u{5341}"
        );
        assert_eq!(
            chinese_longhand_list_marker(
                6001,
                CHINESE_INFORMAL_DIGITS,
                CHINESE_INFORMAL_UNITS,
                true
            ),
            "\u{516d}\u{5343}\u{96f6}\u{4e00}"
        );
        assert_eq!(
            chinese_longhand_list_marker(
                11,
                SIMP_CHINESE_FORMAL_DIGITS,
                CHINESE_FORMAL_UNITS,
                false,
            ),
            "\u{58f9}\u{62fe}\u{58f9}"
        );
        assert_eq!(
            chinese_longhand_list_marker(
                101,
                SIMP_CHINESE_FORMAL_DIGITS,
                CHINESE_FORMAL_UNITS,
                false,
            ),
            "\u{58f9}\u{4f70}\u{96f6}\u{58f9}"
        );
        assert_eq!(
            chinese_longhand_list_marker(
                6001,
                SIMP_CHINESE_FORMAL_DIGITS,
                CHINESE_FORMAL_UNITS,
                false,
            ),
            "\u{9646}\u{4edf}\u{96f6}\u{58f9}"
        );
        assert_eq!(
            chinese_longhand_list_marker(
                6001,
                TRAD_CHINESE_FORMAL_DIGITS,
                CHINESE_FORMAL_UNITS,
                false,
            ),
            "\u{9678}\u{4edf}\u{96f6}\u{58f9}"
        );
    }

    #[test]
    fn ordered_list_start_attribute_sets_initial_marker_index() {
        let doc = parse_html(
            "<!doctype html><html><body><ol start='101'><li>one</li></ol><ul start='9'><li>bullet</li></ul></body></html>",
        );
        let ordered = doc.select_first("ol").expect("ordered list");
        let unordered = doc.select_first("ul").expect("unordered list");

        assert_eq!(list_start_index(ordered.as_node(), true), 101);
        assert_eq!(list_start_index(unordered.as_node(), false), 1);

        let reversed_doc = parse_html(
            "<!doctype html><html><body><ol reversed><li>three</li><li>two</li><li>one</li></ol></body></html>",
        );
        let reversed = reversed_doc.select_first("ol").expect("reversed list");
        assert_eq!(list_start_index(reversed.as_node(), true), 3);
    }

    #[test]
    fn nested_ordered_list_keeps_nested_and_trailing_items_in_their_own_scopes() {
        let doc = parse_html(
            "<!doctype html><html><body><ol><li>one</li><li>two<ol><li>two-one</li><li>two-two</li></ol></li><li>three</li></ol></body></html>",
        );
        let outer = doc.select_first("ol").expect("outer list");
        let direct_items = outer
            .as_node()
            .children()
            .filter(|child| {
                child
                    .as_element()
                    .is_some_and(|element| element.name.local.as_ref() == "li")
            })
            .collect::<Vec<_>>();
        assert_eq!(direct_items.len(), 3);
        assert!(direct_items[1].select_first("ol").is_ok());
        assert_eq!(direct_items[2].text_contents(), "three");
    }

    #[test]
    fn ethiopic_numeric_marker_follows_group_algorithm() {
        assert_eq!(ethiopic_numeric_list_marker(1), "\u{1369}");
        assert_eq!(ethiopic_numeric_list_marker(10), "\u{1372}");
        assert_eq!(ethiopic_numeric_list_marker(100), "\u{137b}");
        assert_eq!(ethiopic_numeric_list_marker(101), "\u{137b}\u{1369}");
        assert_eq!(
            ethiopic_numeric_list_marker(78010092),
            "\u{1378}\u{1370}\u{137b}\u{1369}\u{137c}\u{137a}\u{136a}"
        );
        assert_eq!(
            ethiopic_numeric_list_marker(780100000092),
            "\u{1378}\u{1370}\u{137b}\u{1369}\u{137c}\u{137c}\u{137a}\u{136a}"
        );
    }

    #[test]
    fn anonymous_symbols_markers_follow_counter_systems() {
        fn symbols(
            system: crate::style::AnonymousListStyleSymbolsSystem,
            values: &[&str],
        ) -> crate::style::AnonymousListStyleSymbols {
            crate::style::AnonymousListStyleSymbols {
                system,
                symbols: values.iter().map(|value| value.to_string()).collect(),
                prefix: String::new(),
                suffix: " ".to_string(),
                negative_prefix: "-".to_string(),
                negative_suffix: String::new(),
                pad_width: 0,
                pad_symbol: "0".to_string(),
                fixed_start: 1,
            }
        }

        let symbolic = symbols(
            crate::style::AnonymousListStyleSymbolsSystem::Symbolic,
            &["*", "+"],
        );
        assert_eq!(anonymous_symbols_list_marker(1, &symbolic), "*");
        assert_eq!(anonymous_symbols_list_marker(2, &symbolic), "+");
        assert_eq!(anonymous_symbols_list_marker(3, &symbolic), "**");
        assert_eq!(anonymous_symbols_list_marker(4, &symbolic), "++");

        let cyclic = symbols(
            crate::style::AnonymousListStyleSymbolsSystem::Cyclic,
            &["*", "+"],
        );
        assert_eq!(anonymous_symbols_list_marker(3, &cyclic), "*");

        let fixed = symbols(
            crate::style::AnonymousListStyleSymbolsSystem::Fixed,
            &["A", "B"],
        );
        assert_eq!(anonymous_symbols_list_marker(1, &fixed), "A");
        assert_eq!(anonymous_symbols_list_marker(3, &fixed), "3");

        let alphabetic = symbols(
            crate::style::AnonymousListStyleSymbolsSystem::Alphabetic,
            &["A", "B"],
        );
        assert_eq!(anonymous_symbols_list_marker(3, &alphabetic), "AA");
        assert_eq!(anonymous_symbols_list_marker(4, &alphabetic), "AB");

        let numeric = symbols(
            crate::style::AnonymousListStyleSymbolsSystem::Numeric,
            &["0", "1"],
        );
        assert_eq!(anonymous_symbols_list_marker(1, &numeric), "1");
        assert_eq!(anonymous_symbols_list_marker(2, &numeric), "10");
        assert_eq!(anonymous_symbols_list_marker(3, &numeric), "11");

        let mut decimal = symbols(
            crate::style::AnonymousListStyleSymbolsSystem::ExtendsDecimal,
            &[],
        );
        decimal.prefix = "[".to_string();
        decimal.suffix = "] ".to_string();
        decimal.negative_prefix = "(".to_string();
        decimal.negative_suffix = ")".to_string();
        decimal.pad_width = 2;
        decimal.pad_symbol = "0".to_string();
        assert_eq!(counter_style_value(8, &decimal, true), "[08] ");
        assert_eq!(counter_style_value(-2, &decimal, false), "(02)");
    }

    #[test]
    fn explicitly_placed_grid_items_can_share_the_same_slot() {
        let resolver = StyleResolver::new("");
        let container = resolver.default_style();
        let mut style = resolver.default_style();
        style.grid_row_start = Some(1);
        style.grid_column_start = Some(1);
        style.grid_row_line_start = GridLineSpec::Line(1);
        style.grid_column_line_start = GridLineSpec::Line(1);
        let mut auto_slot = 0;
        let mut occupied = std::collections::HashSet::new();

        let first = grid_item_order_slot(
            1,
            1,
            GridAutoFlowMode::Row,
            &container,
            Some(&style),
            &mut auto_slot,
            &mut occupied,
        );
        let second = grid_item_order_slot(
            1,
            1,
            GridAutoFlowMode::Row,
            &container,
            Some(&style),
            &mut auto_slot,
            &mut occupied,
        );

        assert_eq!(first.slot, 0);
        assert_eq!(second.slot, 0);
        assert_eq!(auto_slot, 0);
    }

    #[test]
    fn fully_definite_grid_items_do_not_advance_the_auto_placement_cursor() {
        let resolver = StyleResolver::new("");
        let container = resolver.default_style();
        let mut explicit = resolver.default_style();
        explicit.grid_row_start = Some(1);
        explicit.grid_column_start = Some(2);
        explicit.grid_row_line_start = GridLineSpec::Line(1);
        explicit.grid_column_line_start = GridLineSpec::Line(2);
        let mut auto_slot = 0;
        let mut occupied = std::collections::HashSet::new();

        let placed = grid_item_order_slot(
            3,
            1,
            GridAutoFlowMode::Row,
            &container,
            Some(&explicit),
            &mut auto_slot,
            &mut occupied,
        );
        let automatic = grid_item_order_slot(
            3,
            1,
            GridAutoFlowMode::Row,
            &container,
            None,
            &mut auto_slot,
            &mut occupied,
        );

        assert_eq!(placed.slot, 1);
        assert_eq!(automatic.slot, 0);
        assert_eq!(auto_slot, 2);
    }

    #[test]
    fn spanning_grid_items_reserve_every_covered_cell() {
        let resolver = StyleResolver::new("");
        let container = resolver.default_style();
        let mut spanning = resolver.default_style();
        spanning.grid_column_line_start = GridLineSpec::Line(1);
        spanning.grid_column_line_end = GridLineSpec::Span(2);
        let mut auto_slot = 0;
        let mut occupied = std::collections::HashSet::new();

        let first = grid_item_order_slot(
            3,
            2,
            GridAutoFlowMode::Row,
            &container,
            Some(&spanning),
            &mut auto_slot,
            &mut occupied,
        );
        let second = grid_item_order_slot(
            3,
            2,
            GridAutoFlowMode::Row,
            &container,
            None,
            &mut auto_slot,
            &mut occupied,
        );

        assert_eq!(first.slot, 0);
        assert_eq!(first.column_span, 2);
        assert!(occupied.contains(&0));
        assert!(occupied.contains(&1));
        assert_eq!(second.slot, 2);
    }

    #[test]
    fn named_grid_areas_resolve_to_their_rectangular_bounds() {
        let resolver = StyleResolver::new("");
        let mut container = resolver.default_style();
        container.grid_template_areas = vec![
            vec![Some("header".to_string()), Some("header".to_string())],
            vec![Some("main".to_string()), Some("side".to_string())],
        ];
        let mut item = resolver.default_style();
        item.grid_area_name = Some("header".to_string());
        let mut auto_slot = 0;
        let mut occupied = std::collections::HashSet::new();

        let placement = grid_item_order_slot(
            2,
            2,
            GridAutoFlowMode::Row,
            &container,
            Some(&item),
            &mut auto_slot,
            &mut occupied,
        );

        assert_eq!(placement.slot, 0);
        assert_eq!(placement.column_span, 2);
        assert_eq!(placement.row_span, 1);
    }

    #[test]
    fn negative_grid_lines_count_back_from_the_explicit_grid() {
        let resolver = StyleResolver::new("");
        let container = resolver.default_style();
        let mut item = resolver.default_style();
        item.grid_column_line_start = GridLineSpec::Line(-3);
        item.grid_column_line_end = GridLineSpec::Line(-1);
        let mut auto_slot = 0;
        let mut occupied = std::collections::HashSet::new();

        let placement = grid_item_order_slot(
            3,
            1,
            GridAutoFlowMode::Row,
            &container,
            Some(&item),
            &mut auto_slot,
            &mut occupied,
        );

        assert_eq!(placement.slot, 1);
        assert_eq!(placement.column_span, 2);
    }

    #[test]
    fn matching_start_and_end_lines_create_an_implicit_named_area() {
        let resolver = StyleResolver::new("");
        let mut container = resolver.default_style();
        container.grid_column_tracks = vec![GridTrackSize::auto(), GridTrackSize::auto()];
        container.grid_row_tracks = vec![GridTrackSize::auto(), GridTrackSize::auto()];
        container.grid_column_line_names = vec![
            vec!["panel-start".to_string()],
            Vec::new(),
            vec!["panel-end".to_string()],
        ];
        container.grid_row_line_names = container.grid_column_line_names.clone();
        let mut item = resolver.default_style();
        item.grid_area_name = Some("panel".to_string());
        let mut auto_slot = 0;
        let mut occupied = std::collections::HashSet::new();

        let placement = grid_item_order_slot(
            2,
            2,
            GridAutoFlowMode::Row,
            &container,
            Some(&item),
            &mut auto_slot,
            &mut occupied,
        );

        assert_eq!(placement.slot, 0);
        assert_eq!(placement.column_span, 2);
        assert_eq!(placement.row_span, 2);
    }

    #[test]
    fn grid_column_auto_flow_fills_rows_before_advancing_columns() {
        let container = StyleResolver::new("").default_style();
        let mut auto_slot = 0;
        let mut occupied = std::collections::HashSet::new();
        let slots: Vec<i32> = (0..4)
            .map(|_| {
                grid_item_order_slot(
                    3,
                    2,
                    GridAutoFlowMode::Column,
                    &container,
                    None,
                    &mut auto_slot,
                    &mut occupied,
                )
            })
            .map(|placement| placement.slot)
            .collect();
        assert_eq!(slots, vec![0, 3, 1, 4]);
    }

    #[test]
    fn grid_auto_repeat_uses_available_width_and_collapses_auto_fit_tracks() {
        let resolver = StyleResolver::new("");
        let mut style = resolver.default_style();
        style.width = LengthSpec::Absolute(Pt::from_f32(375.0));
        style.gap = LengthSpec::Absolute(Pt::from_f32(7.5));
        let fixed = GridTrackSize::fixed(LengthSpec::Absolute(Pt::from_f32(60.0)));
        style.grid_column_auto_repeat = Some(crate::style::GridAutoRepeatSpec {
            mode: GridAutoRepeatMode::Fill,
            tracks: vec![fixed],
        });

        assert_eq!(
            resolve_grid_auto_repeat_columns(&style, 5).unwrap().len(),
            5
        );

        style.grid_column_auto_repeat = Some(crate::style::GridAutoRepeatSpec {
            mode: GridAutoRepeatMode::Fit,
            tracks: vec![GridTrackSize {
                min: GridTrackBreadth::Length(LengthSpec::Absolute(Pt::from_f32(60.0))),
                max: GridTrackBreadth::Fraction(1.0),
            }],
        });
        assert_eq!(
            resolve_grid_auto_repeat_columns(&style, 4).unwrap().len(),
            4
        );
    }

    #[test]
    fn column_dense_prepass_discovers_implicit_column_extent_without_aliasing_rows() {
        let resolver = StyleResolver::new("");
        let container = resolver.default_style();
        let mut spanning = resolver.default_style();
        spanning.grid_row_line_start = GridLineSpec::Span(2);
        let mut cursor = 0usize;
        let mut occupied = std::collections::HashSet::new();
        let placements = [Some(&spanning), Some(&spanning), None]
            .into_iter()
            .map(|style| {
                grid_item_order_slot(
                    3,
                    3,
                    GridAutoFlowMode::ColumnDense,
                    &container,
                    style,
                    &mut cursor,
                    &mut occupied,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            placements.iter().map(|item| item.slot).collect::<Vec<_>>(),
            vec![0, 1, 6]
        );
        let needed_columns = placements
            .iter()
            .map(|item| (item.slot as usize % 3).saturating_add(item.column_span))
            .max()
            .unwrap();
        assert_eq!(needed_columns, 2);
    }
}

fn list_flowables(
    node: &NodeRef,
    resolver: &StyleResolver,
    parent_style: &ComputedStyle,
    ancestors: &[ElementInfo],
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<Arc<AssetBundle>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
) -> Vec<LayoutItem> {
    let mut out = Vec::new();
    let mut report = report;
    let ordered = node
        .as_element()
        .map(|el| el.name.local.as_ref() == "ol")
        .unwrap_or(false);
    let reversed = ordered
        && node
            .as_element()
            .is_some_and(|el| el.attributes.borrow().get("reversed").is_some());
    let index_step = if reversed { -1 } else { 1 };
    let mut index = list_start_index(node, ordered);
    let nested_list_context = ancestors.iter().any(|ancestor| ancestor.tag == "li");
    let mut rendered_item_count = 0usize;

    for child in node.children() {
        if let Some(element) = child.as_element() {
            let tag_is_li = element.name.local.as_ref() == "li";
            let mut info = element_info(&child, resolver.has_sibling_selectors());
            let inline_style = element
                .attributes
                .borrow()
                .get("style")
                .map(|s| s.to_string());
            let style =
                resolver.compute_style(&info, parent_style, inline_style.as_deref(), ancestors);
            info.apply_computed_container_style(&style);
            if !tag_is_li && !style_is_css_list_item(&style) {
                continue;
            }
            if matches!(style.display, DisplayMode::None) {
                continue;
            }
            if matches!(style.display, DisplayMode::Contents) {
                let mut li_ancestors = ancestors.to_vec();
                li_ancestors.push(info);
                for li_child in child.children() {
                    out.extend(node_to_flowables(
                        &li_child,
                        resolver,
                        &style,
                        &mut li_ancestors,
                        counters,
                        font_registry.clone(),
                        asset_bundle.clone(),
                        report.as_deref_mut(),
                        svg_form,
                        svg_raster_fallback,
                        perf,
                        doc_id,
                    ));
                }
                continue;
            }
            let is_first_rendered_item = rendered_item_count == 0;
            rendered_item_count = rendered_item_count.saturating_add(1);
            if ordered {
                if let Some(value) = element
                    .attributes
                    .borrow()
                    .get("value")
                    .and_then(|value| value.trim().parse::<i32>().ok())
                {
                    index = value;
                }
            }
            let explicitly_increments_list_item = style
                .counter_increment
                .iter()
                .any(|mutation| mutation.name == "list-item");
            apply_style_counters(&style, counters);
            apply_implicit_list_item_counter(&style, counters);
            if ordered && !explicitly_increments_list_item {
                counters.set("list-item", index);
            }
            let effective_index = if explicitly_increments_list_item {
                counters.get("list-item")
            } else {
                index
            };
            let mut before_counter_probe = counters.clone();
            let before_items = pseudo_items_for(
                resolver,
                &info,
                &style,
                ancestors,
                &mut before_counter_probe,
                font_registry.clone(),
                asset_bundle.as_deref(),
                report.as_deref_mut(),
                svg_form,
                svg_raster_fallback,
                crate::style::PseudoTarget::Before,
            );
            let mut after_counter_probe = counters.clone();
            let after_items = pseudo_items_for(
                resolver,
                &info,
                &style,
                ancestors,
                &mut after_counter_probe,
                font_registry.clone(),
                asset_bundle.as_deref(),
                report.as_deref_mut(),
                svg_form,
                svg_raster_fallback,
                crate::style::PseudoTarget::After,
            );
            let has_structured_pseudo = !before_items.is_empty() || !after_items.is_empty();
            let is_inline = matches!(
                style.display,
                DisplayMode::Inline
                    | DisplayMode::InlineBlock
                    | DisplayMode::InlineTable
                    | DisplayMode::InlineFlex
                    | DisplayMode::InlineGrid
            );
            let (marker_override, marker_pseudo_style) = if is_inline {
                (None, None)
            } else {
                marker_presentation(resolver, &info, &style, ancestors, counters)
            };
            let marker_style = marker_pseudo_style.as_ref().unwrap_or(&style);
            let round_marker_baseline = marker_style.font_size > style.font_size
                || (nested_list_context && is_first_rendered_item);
            let has_marker_content_override = marker_override.is_some();
            let marker_image = if !is_inline && !has_marker_content_override {
                list_marker_image_flowable(
                    marker_style,
                    asset_bundle.as_deref(),
                    svg_form,
                    svg_raster_fallback,
                )
            } else {
                None
            };
            let marker_index = usize::try_from(effective_index)
                .ok()
                .filter(|value| *value > 0)
                .unwrap_or(1);
            let marker_prefix = if is_inline {
                None
            } else {
                match marker_override {
                    Some(prefix) => prefix,
                    None if marker_image.is_some() => None,
                    None => list_marker_prefix(&style, ordered, marker_index),
                }
            };
            let marker_is_inside =
                matches!(style.list_style_position, ListStylePositionMode::Inside);
            let marker_bullet = if !is_inline
                && !marker_is_inside
                && !has_marker_content_override
                && marker_image.is_none()
            {
                native_list_bullet_flowable(marker_style, ordered)
            } else {
                None
            };
            let marker_cjk = if !is_inline
                && !marker_is_inside
                && !has_marker_content_override
                && marker_image.is_none()
            {
                cjk_decimal_marker_flowable(marker_style, marker_index)
            } else {
                None
            };
            index = index.saturating_add(index_step);

            let mut li_ancestors = ancestors.to_vec();
            li_ancestors.push(info.clone());
            let mut consumed_inside_marker = false;
            let li_body: Box<dyn Flowable> = if marker_is_inside
                && marker_prefix.is_some()
                && !has_structured_pseudo
                && inline_children_only(&child, resolver, &style, &li_ancestors)
            {
                let text = extract_text(&child, style.white_space);
                let text = format!("{}{}", marker_prefix.as_deref().unwrap_or(""), text);
                let text = apply_text_transform(&text, style.text_transform);
                let text_style = text_style_for_flow_text(&style);
                report_missing_glyphs(
                    report.as_deref_mut(),
                    font_registry.as_deref(),
                    &text_style,
                    &text,
                );
                consumed_inside_marker = true;
                Box::new(
                    Paragraph::new(text)
                        .with_style(text_style)
                        .with_align(text_align_from_style(&style))
                        .with_last_align(text_align_last_from_style(&style))
                        .with_whitespace(preserve_whitespace(style.white_space), no_wrap(&style))
                        .with_break_spaces(matches!(style.white_space, WhiteSpaceMode::BreakSpaces))
                        .with_pagination(style.pagination)
                        .with_font_registry(font_registry.clone())
                        .with_tag_role("LBody"),
                ) as Box<dyn Flowable>
            } else {
                let coerce_all_inline = has_structured_pseudo
                    && inline_or_replaced_children_only(&child, resolver, &style, &li_ancestors);
                let mut li_body_items = before_items;
                for li_child in child.children() {
                    let direct_text = matches!(li_child.data(), NodeData::Text(_));
                    let child_items = node_to_flowables(
                        &li_child,
                        resolver,
                        &style,
                        &mut li_ancestors,
                        counters,
                        font_registry.clone(),
                        asset_bundle.clone(),
                        report.as_deref_mut(),
                        svg_form,
                        svg_raster_fallback,
                        perf,
                        doc_id,
                    );
                    if has_structured_pseudo && direct_text && !coerce_all_inline {
                        let valign = vertical_align_from_style(&style);
                        li_body_items.extend(child_items.into_iter().map(|item| match item {
                            LayoutItem::Inline { .. } => item,
                            LayoutItem::Block {
                                flowable,
                                flex_grow,
                                flex_shrink,
                                width_spec,
                                order,
                            } => LayoutItem::Inline {
                                flowable,
                                valign,
                                flex_grow,
                                flex_shrink,
                                width_spec,
                                order,
                            },
                        }));
                    } else {
                        li_body_items.extend(child_items);
                    }
                }
                li_body_items.extend(after_items);
                let li_body_items = if coerce_all_inline {
                    coerce_items_to_inline_run(
                        li_body_items,
                        vertical_align_from_style(&style),
                        &style,
                        font_registry.clone(),
                        false,
                    )
                } else {
                    li_body_items
                };
                if li_body_items.is_empty() {
                    continue;
                }

                let li_body_flowables = layout_children_to_flowables(li_body_items, None);
                if li_body_flowables.is_empty() {
                    continue;
                }
                Box::new(
                    ContainerFlowable::new_pt(
                        li_body_flowables,
                        style.font_size,
                        style.root_font_size,
                    )
                    .with_establishes_abs_containing_block(establishes_abs_containing_block(&style))
                    .with_self_visible(style.visibility.paints())
                    .with_pagination(style.pagination)
                    .with_tag_role("LBody"),
                ) as Box<dyn Flowable>
            };

            let li_flowable: Box<dyn Flowable> = if consumed_inside_marker {
                li_body
            } else if let Some(label) = marker_image {
                Box::new(
                    ListItemFlowable::new_with_label(label, li_body, Pt::from_f32(6.0))
                        .with_marker_inside(marker_is_inside)
                        .with_marker_line_height(text_style_for_flow_text(marker_style).line_height)
                        .with_pagination(style.pagination),
                ) as Box<dyn Flowable>
            } else if let Some((label, gap)) = marker_bullet {
                Box::new(
                    ListItemFlowable::new_with_label(label, li_body, gap)
                        .with_marker_inside(false)
                        .with_pagination(style.pagination),
                ) as Box<dyn Flowable>
            } else if let Some(label) = marker_cjk {
                Box::new(
                    ListItemFlowable::new_with_label(label, li_body, Pt::ZERO)
                        .with_marker_inside(false)
                        .with_pagination(style.pagination),
                ) as Box<dyn Flowable>
            } else if let Some(prefix) = marker_prefix {
                let text_style = marker_text_style(marker_style, &style);
                report_missing_glyphs(
                    report.as_deref_mut(),
                    font_registry.as_deref(),
                    &text_style,
                    &prefix,
                );
                let label_para = Paragraph::new(prefix)
                    .with_style(text_style)
                    .with_align(text_align_from_style(marker_style))
                    .with_last_align(text_align_last_from_style(marker_style))
                    .with_whitespace(
                        preserve_whitespace(marker_style.white_space),
                        no_wrap(marker_style),
                    )
                    .with_break_spaces(matches!(
                        marker_style.white_space,
                        WhiteSpaceMode::BreakSpaces
                    ))
                    .with_pagination(style.pagination)
                    .with_font_registry(font_registry.clone())
                    .with_tag_role("Lbl");
                Box::new(
                    ListItemFlowable::new(label_para, li_body, Pt::ZERO)
                        .with_marker_inside(marker_is_inside)
                        .with_pagination(style.pagination),
                ) as Box<dyn Flowable>
            } else {
                li_body
            };
            let contains_nested_list = child.children().any(|li_child| {
                li_child
                    .as_element()
                    .is_some_and(|element| matches!(element.name.local.as_ref(), "ol" | "ul"))
            });
            let li_flowable = Box::new(
                CssLineBoxFlowable::new(li_flowable).with_round_baseline(round_marker_baseline),
            ) as Box<dyn Flowable>;
            let li_flowable = if contains_nested_list {
                Box::new(CssPixelHeightFlowable::new(li_flowable)) as Box<dyn Flowable>
            } else {
                li_flowable
            };

            let items = vec![LayoutItem::Block {
                flowable: li_flowable,
                flex_grow: 0.0,
                flex_shrink: 1.0,
                width_spec: flex_item_basis(&style),
                order: 0,
            }];
            if is_inline {
                if let Some(container) = container_flowable_with_role(items, &style, Some("LI")) {
                    let valign = vertical_align_from_style(&style);
                    out.push(LayoutItem::Inline {
                        flowable: container,
                        valign,
                        flex_grow: style.flex_grow,
                        flex_shrink: style.flex_shrink,
                        width_spec: flex_item_basis(&style),
                        order: 0,
                    });
                }
            } else {
                out.extend(container_flowables_with_role(items, &style, Some("LI")));
            }
        }
    }
    out
}

fn list_start_index(node: &NodeRef, ordered: bool) -> i32 {
    if !ordered {
        return 1;
    }
    if let Some(start) = node.as_element().and_then(|el| {
        el.attributes
            .borrow()
            .get("start")
            .and_then(|value| value.trim().parse::<i32>().ok())
    }) {
        return start;
    }
    let reversed = node
        .as_element()
        .is_some_and(|el| el.attributes.borrow().get("reversed").is_some());
    if reversed {
        i32::try_from(
            node.children()
                .filter(|child| {
                    child
                        .as_element()
                        .is_some_and(|element| element.name.local.as_ref() == "li")
                })
                .count(),
        )
        .unwrap_or(i32::MAX)
        .max(1)
    } else {
        1
    }
}

fn generated_counter_text(counter: &GeneratedCounterContent, value: i32) -> String {
    generated_counter_text_with_style(&counter.style, value)
}

fn generated_counters_text(counter: &GeneratedCountersContent, values: &[i32]) -> String {
    values
        .iter()
        .map(|value| generated_counter_text_with_style(&counter.style, *value))
        .collect::<Vec<_>>()
        .join(&counter.separator)
}

fn generated_counter_text_with_style(style: &GeneratedCounterStyle, value: i32) -> String {
    let positive = (value > 0).then_some(value as usize);
    match style.list_style_type {
        ListStyleTypeMode::None => String::new(),
        ListStyleTypeMode::Decimal | ListStyleTypeMode::Auto => value.to_string(),
        ListStyleTypeMode::DecimalLeadingZero => {
            if value < 0 {
                format!("-{:02}", value.abs())
            } else {
                format!("{value:02}")
            }
        }
        ListStyleTypeMode::ArabicIndic => positive
            .map(|index| numeric_symbol_list_marker(index, ARABIC_INDIC_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Armenian => positive
            .map(|index| additive_symbol_list_marker(index, UPPER_ARMENIAN_ADDITIVE, 9999))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Bengali => positive
            .map(|index| numeric_symbol_list_marker(index, BENGALI_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Cambodian => positive
            .map(|index| numeric_symbol_list_marker(index, CAMBODIAN_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::CjkDecimal => positive
            .map(|index| numeric_symbol_list_marker(index, CJK_DECIMAL_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::CjkEarthlyBranch => positive
            .map(|index| fixed_symbol_list_marker(index, &CJK_EARTHLY_BRANCH_SYMBOLS, ""))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::CjkHeavenlyStem => positive
            .map(|index| fixed_symbol_list_marker(index, &CJK_HEAVENLY_STEM_SYMBOLS, ""))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Devanagari => positive
            .map(|index| numeric_symbol_list_marker(index, DEVANAGARI_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::EthiopicNumeric => positive
            .map(ethiopic_numeric_list_marker)
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Georgian => positive
            .map(|index| additive_symbol_list_marker(index, GEORGIAN_ADDITIVE, 19999))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Gujarati => positive
            .map(|index| numeric_symbol_list_marker(index, GUJARATI_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Gurmukhi => positive
            .map(|index| numeric_symbol_list_marker(index, GURMUKHI_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Hebrew => positive
            .map(|index| additive_symbol_list_marker(index, HEBREW_ADDITIVE, 10999))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::JapaneseInformal => positive
            .map(|index| {
                additive_or_cjk_decimal_list_marker(index, JAPANESE_INFORMAL_ADDITIVE, 9999)
            })
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::JapaneseFormal => positive
            .map(|index| additive_or_cjk_decimal_list_marker(index, JAPANESE_FORMAL_ADDITIVE, 9999))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Kannada => positive
            .map(|index| numeric_symbol_list_marker(index, KANNADA_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::KoreanHangulFormal => positive
            .map(|index| {
                additive_or_cjk_decimal_list_marker(index, KOREAN_HANGUL_FORMAL_ADDITIVE, 9999)
            })
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::KoreanHanjaInformal => positive
            .map(|index| {
                additive_or_cjk_decimal_list_marker(index, KOREAN_HANJA_INFORMAL_ADDITIVE, 9999)
            })
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::KoreanHanjaFormal => positive
            .map(|index| {
                additive_or_cjk_decimal_list_marker(index, KOREAN_HANJA_FORMAL_ADDITIVE, 9999)
            })
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Lao => positive
            .map(|index| numeric_symbol_list_marker(index, LAO_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::LowerArmenian => positive
            .map(|index| additive_symbol_list_marker(index, LOWER_ARMENIAN_ADDITIVE, 9999))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Malayalam => positive
            .map(|index| numeric_symbol_list_marker(index, MALAYALAM_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Mongolian => positive
            .map(|index| numeric_symbol_list_marker(index, MONGOLIAN_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Myanmar => positive
            .map(|index| numeric_symbol_list_marker(index, MYANMAR_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Oriya => positive
            .map(|index| numeric_symbol_list_marker(index, ORIYA_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Persian => positive
            .map(|index| numeric_symbol_list_marker(index, PERSIAN_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::SimpChineseInformal => positive
            .map(|index| {
                chinese_longhand_list_marker(
                    index,
                    CHINESE_INFORMAL_DIGITS,
                    CHINESE_INFORMAL_UNITS,
                    true,
                )
            })
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::SimpChineseFormal => positive
            .map(|index| {
                chinese_longhand_list_marker(
                    index,
                    SIMP_CHINESE_FORMAL_DIGITS,
                    CHINESE_FORMAL_UNITS,
                    false,
                )
            })
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Tamil => positive
            .map(|index| numeric_symbol_list_marker(index, TAMIL_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Telugu => positive
            .map(|index| numeric_symbol_list_marker(index, TELUGU_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Thai => positive
            .map(|index| numeric_symbol_list_marker(index, THAI_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Tibetan => positive
            .map(|index| numeric_symbol_list_marker(index, TIBETAN_DIGITS))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::TradChineseInformal => positive
            .map(|index| {
                chinese_longhand_list_marker(
                    index,
                    CHINESE_INFORMAL_DIGITS,
                    CHINESE_INFORMAL_UNITS,
                    true,
                )
            })
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::TradChineseFormal => positive
            .map(|index| {
                chinese_longhand_list_marker(
                    index,
                    TRAD_CHINESE_FORMAL_DIGITS,
                    CHINESE_FORMAL_UNITS,
                    false,
                )
            })
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::LowerRoman => positive
            .map(|index| roman_list_marker(index, false))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::UpperRoman => positive
            .map(|index| roman_list_marker(index, true))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::LowerAlpha => positive
            .map(|index| alphabetic_list_marker(index, false))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::UpperAlpha => positive
            .map(|index| alphabetic_list_marker(index, true))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::LowerGreek => positive
            .map(lower_greek_list_marker)
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Hiragana => positive
            .map(hiragana_list_marker)
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::HiraganaIroha => positive
            .map(hiragana_iroha_list_marker)
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Katakana => positive
            .map(katakana_list_marker)
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::KatakanaIroha => positive
            .map(katakana_iroha_list_marker)
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::DisclosureOpen => "\u{25be}".to_string(),
        ListStyleTypeMode::DisclosureClosed => "\u{25b8}".to_string(),
        ListStyleTypeMode::CustomString => {
            style.marker.clone().unwrap_or_else(|| value.to_string())
        }
        ListStyleTypeMode::CustomCounterStyleName | ListStyleTypeMode::AnonymousSymbols => style
            .symbols
            .as_ref()
            .map(|symbols| counter_style_value(value, symbols, false))
            .unwrap_or_else(|| value.to_string()),
        ListStyleTypeMode::Disc => "\u{2022}".to_string(),
        ListStyleTypeMode::Circle => "\u{25e6}".to_string(),
        ListStyleTypeMode::Square => "\u{25a0}".to_string(),
    }
}

fn list_marker_prefix(style: &ComputedStyle, ordered: bool, index: usize) -> Option<String> {
    match style.list_style_type {
        crate::style::ListStyleTypeMode::None => None,
        crate::style::ListStyleTypeMode::Disc => Some("\u{2022} ".to_string()),
        crate::style::ListStyleTypeMode::Circle => Some("\u{25e6} ".to_string()),
        crate::style::ListStyleTypeMode::Square => Some("\u{25a0} ".to_string()),
        crate::style::ListStyleTypeMode::Decimal => Some(format!("{}. ", index)),
        crate::style::ListStyleTypeMode::DecimalLeadingZero => Some(format!("{:02}. ", index)),
        crate::style::ListStyleTypeMode::ArabicIndic => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, ARABIC_INDIC_DIGITS)
        )),
        crate::style::ListStyleTypeMode::Armenian => Some(format!(
            "{}. ",
            additive_symbol_list_marker(index, UPPER_ARMENIAN_ADDITIVE, 9999)
        )),
        crate::style::ListStyleTypeMode::Bengali => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, BENGALI_DIGITS)
        )),
        crate::style::ListStyleTypeMode::Cambodian => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, CAMBODIAN_DIGITS)
        )),
        crate::style::ListStyleTypeMode::CjkDecimal => Some(format!(
            "{}\u{3001}",
            numeric_symbol_list_marker(index, CJK_DECIMAL_DIGITS)
        )),
        crate::style::ListStyleTypeMode::CjkEarthlyBranch => Some(fixed_symbol_list_marker(
            index,
            &CJK_EARTHLY_BRANCH_SYMBOLS,
            "\u{3001}",
        )),
        crate::style::ListStyleTypeMode::CjkHeavenlyStem => Some(fixed_symbol_list_marker(
            index,
            &CJK_HEAVENLY_STEM_SYMBOLS,
            "\u{3001}",
        )),
        crate::style::ListStyleTypeMode::Devanagari => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, DEVANAGARI_DIGITS)
        )),
        crate::style::ListStyleTypeMode::EthiopicNumeric => {
            Some(format!("{}/ ", ethiopic_numeric_list_marker(index)))
        }
        crate::style::ListStyleTypeMode::Georgian => Some(format!(
            "{}. ",
            additive_symbol_list_marker(index, GEORGIAN_ADDITIVE, 19999)
        )),
        crate::style::ListStyleTypeMode::Gujarati => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, GUJARATI_DIGITS)
        )),
        crate::style::ListStyleTypeMode::Gurmukhi => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, GURMUKHI_DIGITS)
        )),
        crate::style::ListStyleTypeMode::Hebrew => Some(format!(
            "{}. ",
            additive_symbol_list_marker(index, HEBREW_ADDITIVE, 10999)
        )),
        crate::style::ListStyleTypeMode::JapaneseInformal => Some(format!(
            "{}",
            additive_or_cjk_decimal_marker_with_suffix(
                index,
                JAPANESE_INFORMAL_ADDITIVE,
                9999,
                "\u{3001}",
            )
        )),
        crate::style::ListStyleTypeMode::JapaneseFormal => Some(format!(
            "{}",
            additive_or_cjk_decimal_marker_with_suffix(
                index,
                JAPANESE_FORMAL_ADDITIVE,
                9999,
                "\u{3001}",
            )
        )),
        crate::style::ListStyleTypeMode::Kannada => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, KANNADA_DIGITS)
        )),
        crate::style::ListStyleTypeMode::KoreanHangulFormal => Some(format!(
            "{}",
            additive_or_cjk_decimal_marker_with_suffix(
                index,
                KOREAN_HANGUL_FORMAL_ADDITIVE,
                9999,
                ", ",
            )
        )),
        crate::style::ListStyleTypeMode::KoreanHanjaInformal => Some(format!(
            "{}",
            additive_or_cjk_decimal_marker_with_suffix(
                index,
                KOREAN_HANJA_INFORMAL_ADDITIVE,
                9999,
                ", ",
            )
        )),
        crate::style::ListStyleTypeMode::KoreanHanjaFormal => Some(format!(
            "{}",
            additive_or_cjk_decimal_marker_with_suffix(
                index,
                KOREAN_HANJA_FORMAL_ADDITIVE,
                9999,
                ", ",
            )
        )),
        crate::style::ListStyleTypeMode::Lao => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, LAO_DIGITS)
        )),
        crate::style::ListStyleTypeMode::LowerArmenian => Some(format!(
            "{}. ",
            additive_symbol_list_marker(index, LOWER_ARMENIAN_ADDITIVE, 9999)
        )),
        crate::style::ListStyleTypeMode::Malayalam => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, MALAYALAM_DIGITS)
        )),
        crate::style::ListStyleTypeMode::Mongolian => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, MONGOLIAN_DIGITS)
        )),
        crate::style::ListStyleTypeMode::Myanmar => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, MYANMAR_DIGITS)
        )),
        crate::style::ListStyleTypeMode::Oriya => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, ORIYA_DIGITS)
        )),
        crate::style::ListStyleTypeMode::Persian => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, PERSIAN_DIGITS)
        )),
        crate::style::ListStyleTypeMode::SimpChineseInformal => {
            Some(chinese_longhand_marker_with_suffix(
                index,
                CHINESE_INFORMAL_DIGITS,
                CHINESE_INFORMAL_UNITS,
                true,
            ))
        }
        crate::style::ListStyleTypeMode::SimpChineseFormal => {
            Some(chinese_longhand_marker_with_suffix(
                index,
                SIMP_CHINESE_FORMAL_DIGITS,
                CHINESE_FORMAL_UNITS,
                false,
            ))
        }
        crate::style::ListStyleTypeMode::Tamil => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, TAMIL_DIGITS)
        )),
        crate::style::ListStyleTypeMode::Telugu => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, TELUGU_DIGITS)
        )),
        crate::style::ListStyleTypeMode::Thai => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, THAI_DIGITS)
        )),
        crate::style::ListStyleTypeMode::Tibetan => Some(format!(
            "{}. ",
            numeric_symbol_list_marker(index, TIBETAN_DIGITS)
        )),
        crate::style::ListStyleTypeMode::TradChineseInformal => {
            Some(chinese_longhand_marker_with_suffix(
                index,
                CHINESE_INFORMAL_DIGITS,
                CHINESE_INFORMAL_UNITS,
                true,
            ))
        }
        crate::style::ListStyleTypeMode::TradChineseFormal => {
            Some(chinese_longhand_marker_with_suffix(
                index,
                TRAD_CHINESE_FORMAL_DIGITS,
                CHINESE_FORMAL_UNITS,
                false,
            ))
        }
        crate::style::ListStyleTypeMode::LowerRoman => {
            Some(format!("{}. ", roman_list_marker(index, false)))
        }
        crate::style::ListStyleTypeMode::UpperRoman => {
            Some(format!("{}. ", roman_list_marker(index, true)))
        }
        crate::style::ListStyleTypeMode::LowerAlpha => {
            Some(format!("{}. ", alphabetic_list_marker(index, false)))
        }
        crate::style::ListStyleTypeMode::UpperAlpha => {
            Some(format!("{}. ", alphabetic_list_marker(index, true)))
        }
        crate::style::ListStyleTypeMode::LowerGreek => {
            Some(format!("{}. ", lower_greek_list_marker(index)))
        }
        crate::style::ListStyleTypeMode::Hiragana => {
            Some(format!("{}\u{3001}", hiragana_list_marker(index)))
        }
        crate::style::ListStyleTypeMode::HiraganaIroha => {
            Some(format!("{}\u{3001}", hiragana_iroha_list_marker(index)))
        }
        crate::style::ListStyleTypeMode::Katakana => {
            Some(format!("{}\u{3001}", katakana_list_marker(index)))
        }
        crate::style::ListStyleTypeMode::KatakanaIroha => {
            Some(format!("{}\u{3001}", katakana_iroha_list_marker(index)))
        }
        crate::style::ListStyleTypeMode::DisclosureOpen => Some("\u{25be} ".to_string()),
        crate::style::ListStyleTypeMode::DisclosureClosed => Some("\u{25b8} ".to_string()),
        crate::style::ListStyleTypeMode::CustomString => style.list_style_marker.clone(),
        crate::style::ListStyleTypeMode::CustomCounterStyleName => {
            if let Some(symbols) = &style.list_style_symbols {
                Some(counter_style_value(
                    i32::try_from(index).unwrap_or(i32::MAX),
                    symbols,
                    true,
                ))
            } else {
                Some(format!("{}. ", index))
            }
        }
        crate::style::ListStyleTypeMode::AnonymousSymbols => {
            style.list_style_symbols.as_ref().map(|symbols| {
                counter_style_value(i32::try_from(index).unwrap_or(i32::MAX), symbols, true)
            })
        }
        crate::style::ListStyleTypeMode::Auto => {
            if ordered {
                Some(format!("{}. ", index))
            } else {
                Some("\u{2022} ".to_string())
            }
        }
    }
}

const ARABIC_INDIC_DIGITS: [&str; 10] = [
    "\u{0660}", "\u{0661}", "\u{0662}", "\u{0663}", "\u{0664}", "\u{0665}", "\u{0666}", "\u{0667}",
    "\u{0668}", "\u{0669}",
];

const UPPER_ARMENIAN_ADDITIVE: &[(usize, &str)] = &[
    (9000, "\u{0554}"),
    (8000, "\u{0553}"),
    (7000, "\u{0552}"),
    (6000, "\u{0551}"),
    (5000, "\u{0550}"),
    (4000, "\u{054f}"),
    (3000, "\u{054e}"),
    (2000, "\u{054d}"),
    (1000, "\u{054c}"),
    (900, "\u{054b}"),
    (800, "\u{054a}"),
    (700, "\u{0549}"),
    (600, "\u{0548}"),
    (500, "\u{0547}"),
    (400, "\u{0546}"),
    (300, "\u{0545}"),
    (200, "\u{0544}"),
    (100, "\u{0543}"),
    (90, "\u{0542}"),
    (80, "\u{0541}"),
    (70, "\u{0540}"),
    (60, "\u{053f}"),
    (50, "\u{053e}"),
    (40, "\u{053d}"),
    (30, "\u{053c}"),
    (20, "\u{053b}"),
    (10, "\u{053a}"),
    (9, "\u{0539}"),
    (8, "\u{0538}"),
    (7, "\u{0537}"),
    (6, "\u{0536}"),
    (5, "\u{0535}"),
    (4, "\u{0534}"),
    (3, "\u{0533}"),
    (2, "\u{0532}"),
    (1, "\u{0531}"),
];

const BENGALI_DIGITS: [&str; 10] = [
    "\u{09e6}", "\u{09e7}", "\u{09e8}", "\u{09e9}", "\u{09ea}", "\u{09eb}", "\u{09ec}", "\u{09ed}",
    "\u{09ee}", "\u{09ef}",
];

const CAMBODIAN_DIGITS: [&str; 10] = [
    "\u{17e0}", "\u{17e1}", "\u{17e2}", "\u{17e3}", "\u{17e4}", "\u{17e5}", "\u{17e6}", "\u{17e7}",
    "\u{17e8}", "\u{17e9}",
];

const CJK_DECIMAL_DIGITS: [&str; 10] = [
    "\u{3007}", "\u{4e00}", "\u{4e8c}", "\u{4e09}", "\u{56db}", "\u{4e94}", "\u{516d}", "\u{4e03}",
    "\u{516b}", "\u{4e5d}",
];

const CJK_EARTHLY_BRANCH_SYMBOLS: [&str; 12] = [
    "\u{5b50}", "\u{4e11}", "\u{5bc5}", "\u{536f}", "\u{8fb0}", "\u{5df3}", "\u{5348}", "\u{672a}",
    "\u{7533}", "\u{9149}", "\u{620c}", "\u{4ea5}",
];

const CJK_HEAVENLY_STEM_SYMBOLS: [&str; 10] = [
    "\u{7532}", "\u{4e59}", "\u{4e19}", "\u{4e01}", "\u{620a}", "\u{5df1}", "\u{5e9a}", "\u{8f9b}",
    "\u{58ec}", "\u{7678}",
];

const DEVANAGARI_DIGITS: [&str; 10] = [
    "\u{0966}", "\u{0967}", "\u{0968}", "\u{0969}", "\u{096a}", "\u{096b}", "\u{096c}", "\u{096d}",
    "\u{096e}", "\u{096f}",
];

const ETHIOPIC_TENS: [&str; 10] = [
    "", "\u{1372}", "\u{1373}", "\u{1374}", "\u{1375}", "\u{1376}", "\u{1377}", "\u{1378}",
    "\u{1379}", "\u{137a}",
];

const ETHIOPIC_UNITS: [&str; 10] = [
    "", "\u{1369}", "\u{136a}", "\u{136b}", "\u{136c}", "\u{136d}", "\u{136e}", "\u{136f}",
    "\u{1370}", "\u{1371}",
];

const GEORGIAN_ADDITIVE: &[(usize, &str)] = &[
    (10000, "\u{10f5}"),
    (9000, "\u{10f0}"),
    (8000, "\u{10ef}"),
    (7000, "\u{10f4}"),
    (6000, "\u{10ee}"),
    (5000, "\u{10ed}"),
    (4000, "\u{10ec}"),
    (3000, "\u{10eb}"),
    (2000, "\u{10ea}"),
    (1000, "\u{10e9}"),
    (900, "\u{10e8}"),
    (800, "\u{10e7}"),
    (700, "\u{10e6}"),
    (600, "\u{10e5}"),
    (500, "\u{10e4}"),
    (400, "\u{10f3}"),
    (300, "\u{10e2}"),
    (200, "\u{10e1}"),
    (100, "\u{10e0}"),
    (90, "\u{10df}"),
    (80, "\u{10de}"),
    (70, "\u{10dd}"),
    (60, "\u{10f2}"),
    (50, "\u{10dc}"),
    (40, "\u{10db}"),
    (30, "\u{10da}"),
    (20, "\u{10d9}"),
    (10, "\u{10d8}"),
    (9, "\u{10d7}"),
    (8, "\u{10f1}"),
    (7, "\u{10d6}"),
    (6, "\u{10d5}"),
    (5, "\u{10d4}"),
    (4, "\u{10d3}"),
    (3, "\u{10d2}"),
    (2, "\u{10d1}"),
    (1, "\u{10d0}"),
];

const GUJARATI_DIGITS: [&str; 10] = [
    "\u{0ae6}", "\u{0ae7}", "\u{0ae8}", "\u{0ae9}", "\u{0aea}", "\u{0aeb}", "\u{0aec}", "\u{0aed}",
    "\u{0aee}", "\u{0aef}",
];

const GURMUKHI_DIGITS: [&str; 10] = [
    "\u{0a66}", "\u{0a67}", "\u{0a68}", "\u{0a69}", "\u{0a6a}", "\u{0a6b}", "\u{0a6c}", "\u{0a6d}",
    "\u{0a6e}", "\u{0a6f}",
];

const HEBREW_ADDITIVE: &[(usize, &str)] = &[
    (10000, "\u{05d9}\u{05f3}"),
    (9000, "\u{05d8}\u{05f3}"),
    (8000, "\u{05d7}\u{05f3}"),
    (7000, "\u{05d6}\u{05f3}"),
    (6000, "\u{05d5}\u{05f3}"),
    (5000, "\u{05d4}\u{05f3}"),
    (4000, "\u{05d3}\u{05f3}"),
    (3000, "\u{05d2}\u{05f3}"),
    (2000, "\u{05d1}\u{05f3}"),
    (1000, "\u{05d0}\u{05f3}"),
    (400, "\u{05ea}"),
    (300, "\u{05e9}"),
    (200, "\u{05e8}"),
    (100, "\u{05e7}"),
    (90, "\u{05e6}"),
    (80, "\u{05e4}"),
    (70, "\u{05e2}"),
    (60, "\u{05e1}"),
    (50, "\u{05e0}"),
    (40, "\u{05de}"),
    (30, "\u{05dc}"),
    (20, "\u{05db}"),
    (19, "\u{05d9}\u{05d8}"),
    (18, "\u{05d9}\u{05d7}"),
    (17, "\u{05d9}\u{05d6}"),
    (16, "\u{05d8}\u{05d6}"),
    (15, "\u{05d8}\u{05d5}"),
    (10, "\u{05d9}"),
    (9, "\u{05d8}"),
    (8, "\u{05d7}"),
    (7, "\u{05d6}"),
    (6, "\u{05d5}"),
    (5, "\u{05d4}"),
    (4, "\u{05d3}"),
    (3, "\u{05d2}"),
    (2, "\u{05d1}"),
    (1, "\u{05d0}"),
];

const JAPANESE_INFORMAL_ADDITIVE: &[(usize, &str)] = &[
    (9000, "\u{4e5d}\u{5343}"),
    (8000, "\u{516b}\u{5343}"),
    (7000, "\u{4e03}\u{5343}"),
    (6000, "\u{516d}\u{5343}"),
    (5000, "\u{4e94}\u{5343}"),
    (4000, "\u{56db}\u{5343}"),
    (3000, "\u{4e09}\u{5343}"),
    (2000, "\u{4e8c}\u{5343}"),
    (1000, "\u{5343}"),
    (900, "\u{4e5d}\u{767e}"),
    (800, "\u{516b}\u{767e}"),
    (700, "\u{4e03}\u{767e}"),
    (600, "\u{516d}\u{767e}"),
    (500, "\u{4e94}\u{767e}"),
    (400, "\u{56db}\u{767e}"),
    (300, "\u{4e09}\u{767e}"),
    (200, "\u{4e8c}\u{767e}"),
    (100, "\u{767e}"),
    (90, "\u{4e5d}\u{5341}"),
    (80, "\u{516b}\u{5341}"),
    (70, "\u{4e03}\u{5341}"),
    (60, "\u{516d}\u{5341}"),
    (50, "\u{4e94}\u{5341}"),
    (40, "\u{56db}\u{5341}"),
    (30, "\u{4e09}\u{5341}"),
    (20, "\u{4e8c}\u{5341}"),
    (10, "\u{5341}"),
    (9, "\u{4e5d}"),
    (8, "\u{516b}"),
    (7, "\u{4e03}"),
    (6, "\u{516d}"),
    (5, "\u{4e94}"),
    (4, "\u{56db}"),
    (3, "\u{4e09}"),
    (2, "\u{4e8c}"),
    (1, "\u{4e00}"),
];

const JAPANESE_FORMAL_ADDITIVE: &[(usize, &str)] = &[
    (9000, "\u{4e5d}\u{9621}"),
    (8000, "\u{516b}\u{9621}"),
    (7000, "\u{4e03}\u{9621}"),
    (6000, "\u{516d}\u{9621}"),
    (5000, "\u{4f0d}\u{9621}"),
    (4000, "\u{56db}\u{9621}"),
    (3000, "\u{53c2}\u{9621}"),
    (2000, "\u{5f10}\u{9621}"),
    (1000, "\u{58f1}\u{9621}"),
    (900, "\u{4e5d}\u{767e}"),
    (800, "\u{516b}\u{767e}"),
    (700, "\u{4e03}\u{767e}"),
    (600, "\u{516d}\u{767e}"),
    (500, "\u{4f0d}\u{767e}"),
    (400, "\u{56db}\u{767e}"),
    (300, "\u{53c2}\u{767e}"),
    (200, "\u{5f10}\u{767e}"),
    (100, "\u{58f1}\u{767e}"),
    (90, "\u{4e5d}\u{62fe}"),
    (80, "\u{516b}\u{62fe}"),
    (70, "\u{4e03}\u{62fe}"),
    (60, "\u{516d}\u{62fe}"),
    (50, "\u{4f0d}\u{62fe}"),
    (40, "\u{56db}\u{62fe}"),
    (30, "\u{53c2}\u{62fe}"),
    (20, "\u{5f10}\u{62fe}"),
    (10, "\u{58f1}\u{62fe}"),
    (9, "\u{4e5d}"),
    (8, "\u{516b}"),
    (7, "\u{4e03}"),
    (6, "\u{516d}"),
    (5, "\u{4f0d}"),
    (4, "\u{56db}"),
    (3, "\u{53c2}"),
    (2, "\u{5f10}"),
    (1, "\u{58f1}"),
];

const KANNADA_DIGITS: [&str; 10] = [
    "\u{0ce6}", "\u{0ce7}", "\u{0ce8}", "\u{0ce9}", "\u{0cea}", "\u{0ceb}", "\u{0cec}", "\u{0ced}",
    "\u{0cee}", "\u{0cef}",
];

const KOREAN_HANGUL_FORMAL_ADDITIVE: &[(usize, &str)] = &[
    (9000, "\u{ad6c}\u{cc9c}"),
    (8000, "\u{d314}\u{cc9c}"),
    (7000, "\u{ce60}\u{cc9c}"),
    (6000, "\u{c721}\u{cc9c}"),
    (5000, "\u{c624}\u{cc9c}"),
    (4000, "\u{c0ac}\u{cc9c}"),
    (3000, "\u{c0bc}\u{cc9c}"),
    (2000, "\u{c774}\u{cc9c}"),
    (1000, "\u{c77c}\u{cc9c}"),
    (900, "\u{ad6c}\u{bc31}"),
    (800, "\u{d314}\u{bc31}"),
    (700, "\u{ce60}\u{bc31}"),
    (600, "\u{c721}\u{bc31}"),
    (500, "\u{c624}\u{bc31}"),
    (400, "\u{c0ac}\u{bc31}"),
    (300, "\u{c0bc}\u{bc31}"),
    (200, "\u{c774}\u{bc31}"),
    (100, "\u{c77c}\u{bc31}"),
    (90, "\u{ad6c}\u{c2ed}"),
    (80, "\u{d314}\u{c2ed}"),
    (70, "\u{ce60}\u{c2ed}"),
    (60, "\u{c721}\u{c2ed}"),
    (50, "\u{c624}\u{c2ed}"),
    (40, "\u{c0ac}\u{c2ed}"),
    (30, "\u{c0bc}\u{c2ed}"),
    (20, "\u{c774}\u{c2ed}"),
    (10, "\u{c77c}\u{c2ed}"),
    (9, "\u{ad6c}"),
    (8, "\u{d314}"),
    (7, "\u{ce60}"),
    (6, "\u{c721}"),
    (5, "\u{c624}"),
    (4, "\u{c0ac}"),
    (3, "\u{c0bc}"),
    (2, "\u{c774}"),
    (1, "\u{c77c}"),
];

const KOREAN_HANJA_INFORMAL_ADDITIVE: &[(usize, &str)] = &[
    (9000, "\u{4e5d}\u{5343}"),
    (8000, "\u{516b}\u{5343}"),
    (7000, "\u{4e03}\u{5343}"),
    (6000, "\u{516d}\u{5343}"),
    (5000, "\u{4e94}\u{5343}"),
    (4000, "\u{56db}\u{5343}"),
    (3000, "\u{4e09}\u{5343}"),
    (2000, "\u{4e8c}\u{5343}"),
    (1000, "\u{5343}"),
    (900, "\u{4e5d}\u{767e}"),
    (800, "\u{516b}\u{767e}"),
    (700, "\u{4e03}\u{767e}"),
    (600, "\u{516d}\u{767e}"),
    (500, "\u{4e94}\u{767e}"),
    (400, "\u{56db}\u{767e}"),
    (300, "\u{4e09}\u{767e}"),
    (200, "\u{4e8c}\u{767e}"),
    (100, "\u{767e}"),
    (90, "\u{4e5d}\u{5341}"),
    (80, "\u{516b}\u{5341}"),
    (70, "\u{4e03}\u{5341}"),
    (60, "\u{516d}\u{5341}"),
    (50, "\u{4e94}\u{5341}"),
    (40, "\u{56db}\u{5341}"),
    (30, "\u{4e09}\u{5341}"),
    (20, "\u{4e8c}\u{5341}"),
    (10, "\u{5341}"),
    (9, "\u{4e5d}"),
    (8, "\u{516b}"),
    (7, "\u{4e03}"),
    (6, "\u{516d}"),
    (5, "\u{4e94}"),
    (4, "\u{56db}"),
    (3, "\u{4e09}"),
    (2, "\u{4e8c}"),
    (1, "\u{4e00}"),
];

const KOREAN_HANJA_FORMAL_ADDITIVE: &[(usize, &str)] = &[
    (9000, "\u{4e5d}\u{4edf}"),
    (8000, "\u{516b}\u{4edf}"),
    (7000, "\u{4e03}\u{4edf}"),
    (6000, "\u{516d}\u{4edf}"),
    (5000, "\u{4e94}\u{4edf}"),
    (4000, "\u{56db}\u{4edf}"),
    (3000, "\u{53c3}\u{4edf}"),
    (2000, "\u{8cb3}\u{4edf}"),
    (1000, "\u{58f9}\u{4edf}"),
    (900, "\u{4e5d}\u{767e}"),
    (800, "\u{516b}\u{767e}"),
    (700, "\u{4e03}\u{767e}"),
    (600, "\u{516d}\u{767e}"),
    (500, "\u{4e94}\u{767e}"),
    (400, "\u{56db}\u{767e}"),
    (300, "\u{53c3}\u{767e}"),
    (200, "\u{8cb3}\u{767e}"),
    (100, "\u{58f9}\u{767e}"),
    (90, "\u{4e5d}\u{62fe}"),
    (80, "\u{516b}\u{62fe}"),
    (70, "\u{4e03}\u{62fe}"),
    (60, "\u{516d}\u{62fe}"),
    (50, "\u{4e94}\u{62fe}"),
    (40, "\u{56db}\u{62fe}"),
    (30, "\u{53c3}\u{62fe}"),
    (20, "\u{8cb3}\u{62fe}"),
    (10, "\u{58f9}\u{62fe}"),
    (9, "\u{4e5d}"),
    (8, "\u{516b}"),
    (7, "\u{4e03}"),
    (6, "\u{516d}"),
    (5, "\u{4e94}"),
    (4, "\u{56db}"),
    (3, "\u{53c3}"),
    (2, "\u{8cb3}"),
    (1, "\u{58f9}"),
];

const LAO_DIGITS: [&str; 10] = [
    "\u{0ed0}", "\u{0ed1}", "\u{0ed2}", "\u{0ed3}", "\u{0ed4}", "\u{0ed5}", "\u{0ed6}", "\u{0ed7}",
    "\u{0ed8}", "\u{0ed9}",
];

const LOWER_ARMENIAN_ADDITIVE: &[(usize, &str)] = &[
    (9000, "\u{0584}"),
    (8000, "\u{0583}"),
    (7000, "\u{0582}"),
    (6000, "\u{0581}"),
    (5000, "\u{0580}"),
    (4000, "\u{057f}"),
    (3000, "\u{057e}"),
    (2000, "\u{057d}"),
    (1000, "\u{057c}"),
    (900, "\u{057b}"),
    (800, "\u{057a}"),
    (700, "\u{0579}"),
    (600, "\u{0578}"),
    (500, "\u{0577}"),
    (400, "\u{0576}"),
    (300, "\u{0575}"),
    (200, "\u{0574}"),
    (100, "\u{0573}"),
    (90, "\u{0572}"),
    (80, "\u{0571}"),
    (70, "\u{0570}"),
    (60, "\u{056f}"),
    (50, "\u{056e}"),
    (40, "\u{056d}"),
    (30, "\u{056c}"),
    (20, "\u{056b}"),
    (10, "\u{056a}"),
    (9, "\u{0569}"),
    (8, "\u{0568}"),
    (7, "\u{0567}"),
    (6, "\u{0566}"),
    (5, "\u{0565}"),
    (4, "\u{0564}"),
    (3, "\u{0563}"),
    (2, "\u{0562}"),
    (1, "\u{0561}"),
];

const MALAYALAM_DIGITS: [&str; 10] = [
    "\u{0d66}", "\u{0d67}", "\u{0d68}", "\u{0d69}", "\u{0d6a}", "\u{0d6b}", "\u{0d6c}", "\u{0d6d}",
    "\u{0d6e}", "\u{0d6f}",
];

const MONGOLIAN_DIGITS: [&str; 10] = [
    "\u{1810}", "\u{1811}", "\u{1812}", "\u{1813}", "\u{1814}", "\u{1815}", "\u{1816}", "\u{1817}",
    "\u{1818}", "\u{1819}",
];

const MYANMAR_DIGITS: [&str; 10] = [
    "\u{1040}", "\u{1041}", "\u{1042}", "\u{1043}", "\u{1044}", "\u{1045}", "\u{1046}", "\u{1047}",
    "\u{1048}", "\u{1049}",
];

const ORIYA_DIGITS: [&str; 10] = [
    "\u{0b66}", "\u{0b67}", "\u{0b68}", "\u{0b69}", "\u{0b6a}", "\u{0b6b}", "\u{0b6c}", "\u{0b6d}",
    "\u{0b6e}", "\u{0b6f}",
];

const PERSIAN_DIGITS: [&str; 10] = [
    "\u{06f0}", "\u{06f1}", "\u{06f2}", "\u{06f3}", "\u{06f4}", "\u{06f5}", "\u{06f6}", "\u{06f7}",
    "\u{06f8}", "\u{06f9}",
];

const CHINESE_INFORMAL_DIGITS: [&str; 10] = [
    "\u{96f6}", "\u{4e00}", "\u{4e8c}", "\u{4e09}", "\u{56db}", "\u{4e94}", "\u{516d}", "\u{4e03}",
    "\u{516b}", "\u{4e5d}",
];

const SIMP_CHINESE_FORMAL_DIGITS: [&str; 10] = [
    "\u{96f6}", "\u{58f9}", "\u{8d30}", "\u{53c1}", "\u{8086}", "\u{4f0d}", "\u{9646}", "\u{67d2}",
    "\u{634c}", "\u{7396}",
];

const TRAD_CHINESE_FORMAL_DIGITS: [&str; 10] = [
    "\u{96f6}", "\u{58f9}", "\u{8cb3}", "\u{53c3}", "\u{8086}", "\u{4f0d}", "\u{9678}", "\u{67d2}",
    "\u{634c}", "\u{7396}",
];

const CHINESE_INFORMAL_UNITS: [&str; 4] = ["", "\u{5341}", "\u{767e}", "\u{5343}"];

const CHINESE_FORMAL_UNITS: [&str; 4] = ["", "\u{62fe}", "\u{4f70}", "\u{4edf}"];

const TAMIL_DIGITS: [&str; 10] = [
    "\u{0be6}", "\u{0be7}", "\u{0be8}", "\u{0be9}", "\u{0bea}", "\u{0beb}", "\u{0bec}", "\u{0bed}",
    "\u{0bee}", "\u{0bef}",
];

const TELUGU_DIGITS: [&str; 10] = [
    "\u{0c66}", "\u{0c67}", "\u{0c68}", "\u{0c69}", "\u{0c6a}", "\u{0c6b}", "\u{0c6c}", "\u{0c6d}",
    "\u{0c6e}", "\u{0c6f}",
];

const THAI_DIGITS: [&str; 10] = [
    "\u{0e50}", "\u{0e51}", "\u{0e52}", "\u{0e53}", "\u{0e54}", "\u{0e55}", "\u{0e56}", "\u{0e57}",
    "\u{0e58}", "\u{0e59}",
];

const TIBETAN_DIGITS: [&str; 10] = [
    "\u{0f20}", "\u{0f21}", "\u{0f22}", "\u{0f23}", "\u{0f24}", "\u{0f25}", "\u{0f26}", "\u{0f27}",
    "\u{0f28}", "\u{0f29}",
];

fn alphabetic_list_marker(mut index: usize, uppercase: bool) -> String {
    if index == 0 {
        index = 1;
    }
    let base = if uppercase { b'A' } else { b'a' };
    let mut chars = Vec::new();
    while index > 0 {
        index -= 1;
        chars.push((base + (index % 26) as u8) as char);
        index /= 26;
    }
    chars.iter().rev().collect()
}

fn numeric_symbol_list_marker(mut value: usize, digits: [&str; 10]) -> String {
    if value == 0 {
        return digits[0].to_string();
    }
    let mut parts = Vec::new();
    while value > 0 {
        parts.push(digits[value % 10]);
        value /= 10;
    }
    parts.iter().rev().copied().collect::<Vec<_>>().join("")
}

fn fixed_symbol_list_marker(index: usize, symbols: &[&str], suffix: &str) -> String {
    if let Some(symbol) = index.checked_sub(1).and_then(|idx| symbols.get(idx)) {
        return format!("{}{}", symbol, suffix);
    }
    format!("{}. ", index.max(1))
}

fn anonymous_symbols_list_marker(
    index: usize,
    spec: &crate::style::AnonymousListStyleSymbols,
) -> String {
    if spec.symbols.is_empty()
        && !matches!(
            spec.system,
            crate::style::AnonymousListStyleSymbolsSystem::ExtendsDecimal
        )
    {
        return index.to_string();
    }
    match spec.system {
        crate::style::AnonymousListStyleSymbolsSystem::Cyclic => {
            spec.symbols[index.saturating_sub(1) % spec.symbols.len()].clone()
        }
        crate::style::AnonymousListStyleSymbolsSystem::Fixed => spec
            .symbols
            .get(
                (index as i64 - spec.fixed_start as i64)
                    .try_into()
                    .ok()
                    .unwrap_or(usize::MAX),
            )
            .cloned()
            .unwrap_or_else(|| index.to_string()),
        crate::style::AnonymousListStyleSymbolsSystem::Symbolic => {
            let symbol = &spec.symbols[index.saturating_sub(1) % spec.symbols.len()];
            let repeat_count = index.saturating_sub(1) / spec.symbols.len() + 1;
            symbol.repeat(repeat_count)
        }
        crate::style::AnonymousListStyleSymbolsSystem::Alphabetic => {
            alphabetic_symbols_marker(index, &spec.symbols)
        }
        crate::style::AnonymousListStyleSymbolsSystem::Numeric => {
            numeric_symbols_marker(index, &spec.symbols)
        }
        crate::style::AnonymousListStyleSymbolsSystem::ExtendsDecimal => index.to_string(),
    }
}

fn counter_style_value(
    value: i32,
    spec: &crate::style::AnonymousListStyleSymbols,
    include_affixes: bool,
) -> String {
    let negative = value < 0;
    let magnitude = value.unsigned_abs() as usize;
    let mut representation = anonymous_symbols_list_marker(magnitude, spec);
    if spec.pad_width > 0 && !spec.pad_symbol.is_empty() {
        let symbol_len = spec.pad_symbol.chars().count().max(1);
        let current_len = representation.chars().count();
        if current_len < spec.pad_width {
            let missing = spec.pad_width - current_len;
            let repeats = missing.div_ceil(symbol_len);
            representation = format!("{}{}", spec.pad_symbol.repeat(repeats), representation);
        }
    }
    if negative {
        representation = format!(
            "{}{}{}",
            spec.negative_prefix, representation, spec.negative_suffix
        );
    }
    if include_affixes {
        format!("{}{}{}", spec.prefix, representation, spec.suffix)
    } else {
        representation
    }
}

fn alphabetic_symbols_marker(mut index: usize, symbols: &[String]) -> String {
    if symbols.len() < 2 || index == 0 {
        return index.to_string();
    }
    let base = symbols.len();
    let mut out = Vec::new();
    while index > 0 {
        index -= 1;
        out.push(symbols[index % base].as_str());
        index /= base;
    }
    out.reverse();
    out.join("")
}

fn numeric_symbols_marker(mut index: usize, symbols: &[String]) -> String {
    if symbols.len() < 2 {
        return index.to_string();
    }
    if index == 0 {
        return symbols[0].clone();
    }
    let base = symbols.len();
    let mut out = Vec::new();
    while index > 0 {
        out.push(symbols[index % base].as_str());
        index /= base;
    }
    out.reverse();
    out.join("")
}

fn additive_symbol_list_marker(
    index: usize,
    symbols: &[(usize, &str)],
    max_value: usize,
) -> String {
    if index == 0 || index > max_value {
        return index.max(1).to_string();
    }
    let mut value = index;
    let mut out = String::new();
    for (amount, symbol) in symbols {
        if value >= *amount {
            out.push_str(symbol);
            value -= *amount;
        }
    }
    if out.is_empty() {
        index.to_string()
    } else {
        out
    }
}

fn additive_or_cjk_decimal_list_marker(
    index: usize,
    symbols: &[(usize, &str)],
    max_value: usize,
) -> String {
    if index == 0 || index > max_value {
        return numeric_symbol_list_marker(index.max(1), CJK_DECIMAL_DIGITS);
    }
    additive_symbol_list_marker(index, symbols, max_value)
}

fn additive_or_cjk_decimal_marker_with_suffix(
    index: usize,
    symbols: &[(usize, &str)],
    max_value: usize,
    suffix: &str,
) -> String {
    let marker = additive_or_cjk_decimal_list_marker(index, symbols, max_value);
    if index == 0 || index > max_value {
        return format!("{}\u{3001}", marker);
    }
    format!("{}{}", marker, suffix)
}

fn chinese_longhand_marker_with_suffix(
    index: usize,
    digits: [&str; 10],
    units: [&str; 4],
    informal_tens: bool,
) -> String {
    if index == 0 || index > 9999 {
        return format!(
            "{}\u{3001}",
            numeric_symbol_list_marker(index.max(1), CJK_DECIMAL_DIGITS)
        );
    }
    format!(
        "{}\u{3001}",
        chinese_longhand_list_marker(index, digits, units, informal_tens)
    )
}

fn chinese_longhand_list_marker(
    index: usize,
    digits: [&str; 10],
    units: [&str; 4],
    informal_tens: bool,
) -> String {
    if index == 0 {
        return digits[0].to_string();
    }

    let places = [
        ((index / 1000) % 10, 3usize),
        ((index / 100) % 10, 2usize),
        ((index / 10) % 10, 1usize),
        (index % 10, 0usize),
    ];
    let mut parts: Vec<Option<String>> = Vec::new();
    for (digit, unit_idx) in places {
        if digit == 0 {
            if !parts.is_empty() {
                parts.push(None);
            }
            continue;
        }
        let text = if informal_tens && unit_idx == 1 && digit == 1 && index < 20 {
            units[unit_idx].to_string()
        } else {
            format!("{}{}", digits[digit], units[unit_idx])
        };
        parts.push(Some(text));
    }

    while matches!(parts.last(), Some(None)) {
        parts.pop();
    }

    let mut out = String::new();
    let mut pending_zero = false;
    for part in parts {
        match part {
            Some(text) => {
                if pending_zero && !out.is_empty() {
                    out.push_str(digits[0]);
                }
                out.push_str(&text);
                pending_zero = false;
            }
            None => pending_zero = true,
        }
    }
    if out.is_empty() {
        digits[0].to_string()
    } else {
        out
    }
}

fn ethiopic_numeric_list_marker(index: usize) -> String {
    let mut value = index.max(1);
    if value == 1 {
        return ETHIOPIC_UNITS[1].to_string();
    }

    let mut groups = Vec::new();
    while value > 0 {
        groups.push(value % 100);
        value /= 100;
    }

    let most_significant = groups.len().saturating_sub(1);
    let mut out = String::new();
    for (group_index, group_value) in groups.iter().enumerate().rev() {
        let group_value = *group_value;
        let remove_digits = group_value == 0
            || (group_index == most_significant && group_value == 1)
            || (group_index % 2 == 1 && group_value == 1);

        if !remove_digits {
            let tens = group_value / 10;
            let units = group_value % 10;
            if tens > 0 {
                out.push_str(ETHIOPIC_TENS[tens]);
            }
            if units > 0 {
                out.push_str(ETHIOPIC_UNITS[units]);
            }
        }

        if group_index % 2 == 1 && group_value != 0 {
            out.push('\u{137b}');
        }
        if group_index % 2 == 0 && group_index != 0 {
            out.push('\u{137c}');
        }
    }
    out
}

fn lower_greek_list_marker(index: usize) -> String {
    const GREEK: [&str; 24] = [
        "\u{03b1}", "\u{03b2}", "\u{03b3}", "\u{03b4}", "\u{03b5}", "\u{03b6}", "\u{03b7}",
        "\u{03b8}", "\u{03b9}", "\u{03ba}", "\u{03bb}", "\u{03bc}", "\u{03bd}", "\u{03be}",
        "\u{03bf}", "\u{03c0}", "\u{03c1}", "\u{03c3}", "\u{03c4}", "\u{03c5}", "\u{03c6}",
        "\u{03c7}", "\u{03c8}", "\u{03c9}",
    ];
    symbolic_alphabetic_list_marker(index, &GREEK)
}

fn hiragana_list_marker(index: usize) -> String {
    const HIRAGANA: [&str; 48] = [
        "\u{3042}", "\u{3044}", "\u{3046}", "\u{3048}", "\u{304a}", "\u{304b}", "\u{304d}",
        "\u{304f}", "\u{3051}", "\u{3053}", "\u{3055}", "\u{3057}", "\u{3059}", "\u{305b}",
        "\u{305d}", "\u{305f}", "\u{3061}", "\u{3064}", "\u{3066}", "\u{3068}", "\u{306a}",
        "\u{306b}", "\u{306c}", "\u{306d}", "\u{306e}", "\u{306f}", "\u{3072}", "\u{3075}",
        "\u{3078}", "\u{307b}", "\u{307e}", "\u{307f}", "\u{3080}", "\u{3081}", "\u{3082}",
        "\u{3084}", "\u{3086}", "\u{3088}", "\u{3089}", "\u{308a}", "\u{308b}", "\u{308c}",
        "\u{308d}", "\u{308f}", "\u{3090}", "\u{3091}", "\u{3092}", "\u{3093}",
    ];
    symbolic_alphabetic_list_marker(index, &HIRAGANA)
}

fn hiragana_iroha_list_marker(index: usize) -> String {
    const HIRAGANA_IROHA: [&str; 47] = [
        "\u{3044}", "\u{308d}", "\u{306f}", "\u{306b}", "\u{307b}", "\u{3078}", "\u{3068}",
        "\u{3061}", "\u{308a}", "\u{306c}", "\u{308b}", "\u{3092}", "\u{308f}", "\u{304b}",
        "\u{3088}", "\u{305f}", "\u{308c}", "\u{305d}", "\u{3064}", "\u{306d}", "\u{306a}",
        "\u{3089}", "\u{3080}", "\u{3046}", "\u{3090}", "\u{306e}", "\u{304a}", "\u{304f}",
        "\u{3084}", "\u{307e}", "\u{3051}", "\u{3075}", "\u{3053}", "\u{3048}", "\u{3066}",
        "\u{3042}", "\u{3055}", "\u{304d}", "\u{3086}", "\u{3081}", "\u{307f}", "\u{3057}",
        "\u{3091}", "\u{3072}", "\u{3082}", "\u{305b}", "\u{3059}",
    ];
    symbolic_alphabetic_list_marker(index, &HIRAGANA_IROHA)
}

fn katakana_list_marker(index: usize) -> String {
    const KATAKANA: [&str; 48] = [
        "\u{30a2}", "\u{30a4}", "\u{30a6}", "\u{30a8}", "\u{30aa}", "\u{30ab}", "\u{30ad}",
        "\u{30af}", "\u{30b1}", "\u{30b3}", "\u{30b5}", "\u{30b7}", "\u{30b9}", "\u{30bb}",
        "\u{30bd}", "\u{30bf}", "\u{30c1}", "\u{30c4}", "\u{30c6}", "\u{30c8}", "\u{30ca}",
        "\u{30cb}", "\u{30cc}", "\u{30cd}", "\u{30ce}", "\u{30cf}", "\u{30d2}", "\u{30d5}",
        "\u{30d8}", "\u{30db}", "\u{30de}", "\u{30df}", "\u{30e0}", "\u{30e1}", "\u{30e2}",
        "\u{30e4}", "\u{30e6}", "\u{30e8}", "\u{30e9}", "\u{30ea}", "\u{30eb}", "\u{30ec}",
        "\u{30ed}", "\u{30ef}", "\u{30f0}", "\u{30f1}", "\u{30f2}", "\u{30f3}",
    ];
    symbolic_alphabetic_list_marker(index, &KATAKANA)
}

fn katakana_iroha_list_marker(index: usize) -> String {
    const KATAKANA_IROHA: [&str; 47] = [
        "\u{30a4}", "\u{30ed}", "\u{30cf}", "\u{30cb}", "\u{30db}", "\u{30d8}", "\u{30c8}",
        "\u{30c1}", "\u{30ea}", "\u{30cc}", "\u{30eb}", "\u{30f2}", "\u{30ef}", "\u{30ab}",
        "\u{30e8}", "\u{30bf}", "\u{30ec}", "\u{30bd}", "\u{30c4}", "\u{30cd}", "\u{30ca}",
        "\u{30e9}", "\u{30e0}", "\u{30a6}", "\u{30f0}", "\u{30ce}", "\u{30aa}", "\u{30af}",
        "\u{30e4}", "\u{30de}", "\u{30b1}", "\u{30d5}", "\u{30b3}", "\u{30a8}", "\u{30c6}",
        "\u{30a2}", "\u{30b5}", "\u{30ad}", "\u{30e6}", "\u{30e1}", "\u{30df}", "\u{30b7}",
        "\u{30f1}", "\u{30d2}", "\u{30e2}", "\u{30bb}", "\u{30b9}",
    ];
    symbolic_alphabetic_list_marker(index, &KATAKANA_IROHA)
}

fn symbolic_alphabetic_list_marker(mut index: usize, symbols: &[&str]) -> String {
    if index == 0 {
        index = 1;
    }
    let base = symbols.len();
    if base == 0 {
        return String::new();
    }
    let mut parts = Vec::new();
    while index > 0 {
        index -= 1;
        parts.push(symbols[index % base]);
        index /= base;
    }
    parts.iter().rev().copied().collect::<Vec<_>>().join("")
}

fn roman_list_marker(index: usize, uppercase: bool) -> String {
    let mut value = index.min(3999);
    if value == 0 {
        value = 1;
    }
    let table = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    for (amount, marker) in table {
        while value >= amount {
            out.push_str(marker);
            value -= amount;
        }
    }
    if uppercase {
        out.to_ascii_uppercase()
    } else {
        out
    }
}

fn container_flowables(children: Vec<LayoutItem>, style: &ComputedStyle) -> Vec<LayoutItem> {
    container_flowables_with_role(children, style, None)
}

#[derive(Clone, Copy)]
struct ContainerCompilationOptions {
    suppress_single_used_column_rule: bool,
}

impl Default for ContainerCompilationOptions {
    fn default() -> Self {
        Self {
            suppress_single_used_column_rule: false,
        }
    }
}

fn container_flowables_with_options(
    children: Vec<LayoutItem>,
    style: &ComputedStyle,
    options: ContainerCompilationOptions,
) -> Vec<LayoutItem> {
    container_flowables_with_role_options(children, style, None, options)
}

fn establishes_abs_containing_block(style: &ComputedStyle) -> bool {
    !matches!(style.position, PositionMode::Static) || !style.transform.is_empty()
}

fn establishes_stacking_context(style: &ComputedStyle) -> bool {
    matches!(style.position, PositionMode::Fixed | PositionMode::Sticky)
        || (!style.z_index_auto && !matches!(style.position, PositionMode::Static))
        || style.opacity < 1.0 - 1.0e-6
        || style.paint_filter_stacking_context
        || style.backdrop_filter.is_some()
        || !style.transform.is_empty()
        || style.perspective.is_some()
        || style.isolation
        || !matches!(style.mix_blend_mode, crate::types::MixBlendMode::Normal)
        || style.mask_backdrop_root
        || style.will_change_backdrop_root
        || style.clip_path.is_some()
}

fn mixes_inline_and_block_children(children: &[LayoutItem]) -> bool {
    let has_inline = children
        .iter()
        .any(|child| matches!(child, LayoutItem::Inline { .. }));
    let has_block = children
        .iter()
        .any(|child| matches!(child, LayoutItem::Block { .. }));
    has_inline && has_block
}

fn forced_inline_line_height(children: &[LayoutItem], style: &ComputedStyle) -> Option<Pt> {
    let has_inline = children
        .iter()
        .any(|child| matches!(child, LayoutItem::Inline { .. }));

    // The fixed-height compatibility line box applies only to a single inline
    // formatting context. Mixed inline/block children create anonymous block
    // boxes; forcing every anonymous inline line to the container's height
    // makes generated ::before/::after content advance by that height each time.
    if has_inline && !mixes_inline_and_block_children(children) {
        match style.height {
            LengthSpec::Absolute(value) if value > Pt::ZERO => Some(value),
            _ => None,
        }
    } else {
        None
    }
}

fn normal_flow_container_width(style: &ComputedStyle) -> LengthSpec {
    if matches!(style.display, DisplayMode::InlineBlock)
        && matches!(
            style.width,
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
        )
    {
        // An auto-width inline-block is atomic inline content and therefore
        // shrink-to-fit. Treating it like an ordinary auto-width block makes
        // each sibling consume the full line and forces otherwise-fitting
        // inline-blocks onto separate lines.
        LengthSpec::FitContent
    } else {
        style.width
    }
}

fn length_has_percentage_component(length: LengthSpec) -> bool {
    match length {
        LengthSpec::Percent(value) => value.abs() > f32::EPSILON,
        LengthSpec::Calc(value) => value.percent.abs() > f32::EPSILON,
        LengthSpec::Clamped(value) => value.value.percent.abs() > f32::EPSILON,
        LengthSpec::FontRelative(value) => value.base.percent.abs() > f32::EPSILON,
        _ => false,
    }
}

fn compiled_multicol_run(
    children: Vec<Box<dyn Flowable>>,
    style: &ComputedStyle,
    options: ContainerCompilationOptions,
) -> Box<dyn Flowable> {
    let suppress_single_used_column_rule = options.suppress_single_used_column_rule
        || children.iter().any(|child| child.has_replaced_descendant());
    Box::new(
        MultiColumnFlowable::new_pt(
            children,
            style.column_count,
            style.column_count_auto,
            style.column_width,
            style.column_fill,
            style.direction,
            style.gap,
            style.column_gap_normal,
            style.column_rule_width,
            style.column_rule_style,
            style.resolved_column_rule_color(),
            style.column_rule_visible,
            style.font_size,
            style.root_font_size,
        )
        .with_writing_mode(style.writing_mode)
        .with_column_item_inline_paint_snapping(matches!(
            style.writing_mode,
            WritingModeMode::HorizontalTb
        ))
        .with_single_used_column_rule_suppression(
            suppress_single_used_column_rule && matches!(style.position, PositionMode::Static),
        ),
    )
}

fn compile_multicol_child_stream(
    flowables: Vec<Box<dyn Flowable>>,
    style: &ComputedStyle,
    options: ContainerCompilationOptions,
) -> Vec<Box<dyn Flowable>> {
    if !flowables.iter().any(|child| child.spans_all_columns()) {
        return vec![compiled_multicol_run(flowables, style, options)];
    }

    // A spanning child terminates the current column row, participates in the
    // parent's ordinary block flow at full width, and starts a fresh balanced
    // row afterward. Lower that structure once here so wrap/draw can reuse the
    // same compiled runs instead of rediscovering span boundaries per page.
    let mut compiled = Vec::new();
    let mut run = Vec::new();
    for child in flowables {
        if child.spans_all_columns() {
            if !run.is_empty() {
                compiled.push(compiled_multicol_run(
                    std::mem::take(&mut run),
                    style,
                    options,
                ));
            }
            compiled.push(child);
        } else {
            run.push(child);
        }
    }
    if !run.is_empty() {
        compiled.push(compiled_multicol_run(run, style, options));
    }
    compiled
}

fn container_flowable_with_role(
    children: Vec<LayoutItem>,
    style: &ComputedStyle,
    role: Option<&str>,
) -> Option<Box<dyn Flowable>> {
    container_flowable_with_role_options(
        children,
        style,
        role,
        ContainerCompilationOptions::default(),
    )
}

fn effective_legacy_clip(style: &ComputedStyle) -> Option<ClipPathRectSpec> {
    matches!(style.position, PositionMode::Absolute | PositionMode::Fixed)
        .then_some(style.legacy_clip)
        .flatten()
}

fn container_flowable_with_role_options(
    children: Vec<LayoutItem>,
    style: &ComputedStyle,
    role: Option<&str>,
    options: ContainerCompilationOptions,
) -> Option<Box<dyn Flowable>> {
    let has_box = !matches!(style.width, LengthSpec::Auto)
        || !matches!(style.height, LengthSpec::Auto)
        || !matches!(style.min_width, LengthSpec::Auto)
        || !matches!(style.max_width, LengthSpec::Auto)
        || !matches!(style.min_height, LengthSpec::Auto)
        || !matches!(style.max_height, LengthSpec::Auto)
        || style.margin != EdgeSizes::zero()
        || style.padding != EdgeSizes::zero()
        || style.background_color.is_some()
        || style.background_paint.is_some()
        || style.clip_path.is_some()
        || style.legacy_clip.is_some()
        || style.box_shadow.is_some()
        || style.paint_filter.is_some()
        || style.backdrop_filter.is_some()
        || style.will_change_backdrop_root
        || style.mask_backdrop_root
        || !matches!(style.mix_blend_mode, crate::types::MixBlendMode::Normal)
        || style.isolation
        || style.opacity < 1.0 - 1.0e-6
        || style.border_radius != BorderRadiiSpec::zero()
        || style.outline_visible
        || style.border_width != EdgeSizes::zero()
        || style.border_image.source.is_some();

    if children.is_empty() && !has_box {
        // Preserve page-break semantics even for empty elements.
        if style.pagination.break_before != BreakBefore::Auto
            || style.pagination.break_after != BreakAfter::Auto
        {
            let mut container =
                ContainerFlowable::new_pt(Vec::new(), style.font_size, style.root_font_size)
                    .with_establishes_abs_containing_block(establishes_abs_containing_block(style))
                    .with_establishes_stacking_context(establishes_stacking_context(style))
                    .with_transforms(style.transform.clone())
                    .with_transform_origin(style.transform_origin)
                    .with_transform_box(style.transform_box)
                    .with_perspective(style.perspective, style.perspective_origin)
                    .with_transform_style(style.transform_style)
                    .with_self_visible(style.visibility.paints())
                    .with_column_span_all(matches!(style.column_span, ColumnSpanMode::All))
                    .with_pagination(style.pagination);
            if let Some(role) = role {
                container = container.with_tag_role(role);
            }
            return Some(Box::new(container) as Box<dyn Flowable>);
        }
        return None;
    }

    let forced_line_height = forced_inline_line_height(&children, style);
    let anonymous_block_lines = mixes_inline_and_block_children(&children);

    let mut flowables = layout_children_to_flowables_with_options(
        children,
        forced_line_height,
        no_wrap(style),
        !anonymous_block_lines,
        anonymous_block_lines,
    );
    let is_multicol_container = (style.column_count > 1
        || !matches!(style.column_width, LengthSpec::Auto))
        && matches!(
            style.display,
            DisplayMode::Block | DisplayMode::FlowRoot | DisplayMode::InlineBlock
        );
    if is_multicol_container {
        flowables = compile_multicol_child_stream(flowables, style, options);
    }
    if matches!(style.writing_mode, WritingModeMode::HorizontalTb)
        && length_has_percentage_component(style.padding.left)
    {
        // LayoutNG retains the fractional content origin created by percentage
        // padding, but snaps each in-flow child's painted border box to CSS
        // pixels. Reuse the paint-only transform already used by grid items so
        // text/replaced content keeps its fixed-point layout phase.
        flowables = flowables
            .into_iter()
            .map(|child| child.with_grid_item_inline_paint_snap())
            .collect();
    }
    let mut container = ContainerFlowable::new_pt(flowables, style.font_size, style.root_font_size)
        .with_establishes_abs_containing_block(establishes_abs_containing_block(style))
        .with_establishes_stacking_context(establishes_stacking_context(style))
        .with_float_containment(
            matches!(style.display, DisplayMode::FlowRoot)
                || matches!(style.overflow, OverflowMode::Hidden),
        )
        .with_margin(style.margin)
        .with_inline_paint_snapping(
            matches!(style.margin.left, LengthSpec::Auto)
                && matches!(style.margin.right, LengthSpec::Auto),
        )
        .with_border(
            style.border_width,
            style.border_color.unwrap_or(style.color),
        )
        .with_border_colors(
            style.resolved_border_colors(style.color).top,
            style.resolved_border_colors(style.color).right,
            style.resolved_border_colors(style.color).bottom,
            style.resolved_border_colors(style.color).left,
        )
        .with_border_opacities(
            style.resolved_border_opacities().top,
            style.resolved_border_opacities().right,
            style.resolved_border_opacities().bottom,
            style.resolved_border_opacities().left,
        )
        .with_border_styles(
            style.resolved_border_styles().top,
            style.resolved_border_styles().right,
            style.resolved_border_styles().bottom,
            style.resolved_border_styles().left,
        )
        .with_border_radius(style.border_radius)
        .with_box_decoration_break(style.box_decoration_break)
        .with_border_image(style.border_image.clone())
        .with_outline(
            style.outline_width,
            style.outline_offset,
            style.outline_style,
            style.resolved_outline_color(),
            style.outline_visible,
        )
        .with_padding(style.padding)
        .with_box_sizing(style.box_sizing)
        .with_width(normal_flow_container_width(style))
        .with_max_width(style.max_width)
        .with_min_width(style.min_width)
        .with_height(style.height)
        .with_aspect_ratio(style.aspect_ratio)
        .with_min_height(style.min_height)
        .with_max_height(style.max_height)
        .with_background(style.background_source_color.or(style.background_color))
        .with_background_opacity(style.background_alpha)
        .with_background_paint(style.background_paint.clone())
        .with_background_layers(
            style.background_paints.clone(),
            style.background_sizes.clone(),
            style.background_positions.clone(),
            style.background_repeats.clone(),
            style.background_attachments.clone(),
            style.background_origins.clone(),
            style.background_clips.clone(),
        )
        .with_background_blend_modes(style.background_blend_modes.clone())
        .with_clip_path(style.clip_path.clone())
        .with_clip_path_reference_box(style.clip_path_reference_box)
        .with_legacy_clip(effective_legacy_clip(style))
        .with_box_shadows(style.box_shadows.clone())
        .with_paint_filter(style.paint_filter.clone())
        .with_backdrop_filter(style.backdrop_filter.clone())
        .with_will_change_backdrop_root(style.will_change_backdrop_root)
        .with_mask(style.mask.clone())
        .with_mask_backdrop_root(style.mask_backdrop_root)
        .with_mix_blend_mode(style.mix_blend_mode)
        .with_isolation(style.isolation)
        .with_opacity(style.opacity)
        .with_transforms(style.transform.clone())
        .with_transform_origin(style.transform_origin)
        .with_transform_box(style.transform_box)
        .with_perspective(style.perspective, style.perspective_origin)
        .with_transform_style(style.transform_style)
        .with_overflow_modes(style.overflow_x, style.overflow_y)
        .with_overflow_clip_margin(style.overflow_clip_margin)
        .with_scrollbar_gutter(style.scrollbar_gutter, style.direction, style.writing_mode)
        .with_line_clamp(
            style.line_clamp,
            text_style_for_flow_text(style).line_height,
        )
        .with_self_visible(style.visibility.paints())
        .with_column_span_all(matches!(style.column_span, ColumnSpanMode::All))
        .with_pagination(style.pagination);
    if let Some(role) = role {
        container = container.with_tag_role(role);
    }
    Some(Box::new(container) as Box<dyn Flowable>)
}

fn container_flowables_with_role(
    children: Vec<LayoutItem>,
    style: &ComputedStyle,
    role: Option<&str>,
) -> Vec<LayoutItem> {
    container_flowables_with_role_options(
        children,
        style,
        role,
        ContainerCompilationOptions::default(),
    )
}

fn container_flowables_with_role_options(
    children: Vec<LayoutItem>,
    style: &ComputedStyle,
    role: Option<&str>,
    options: ContainerCompilationOptions,
) -> Vec<LayoutItem> {
    let Some(container) = container_flowable_with_role_options(children, style, role, options)
    else {
        return Vec::new();
    };
    vec![LayoutItem::Block {
        flowable: container,
        flex_grow: style.flex_grow,
        flex_shrink: style.flex_shrink,
        width_spec: flex_item_basis(&style),
        order: 0,
    }]
}

fn definition_list_container_role(tag: &str) -> Option<&'static str> {
    match tag {
        "dl" => Some("L"),
        "dt" => Some("Lbl"),
        "dd" => Some("LBody"),
        _ => None,
    }
}

fn direct_text_structure_role(tag: &str) -> &'static str {
    match tag {
        "dl" | "dd" => "LBody",
        "dt" => "Lbl",
        "blockquote" => "BlockQuote",
        "figcaption" => "Caption",
        "span" | "i" => "Span",
        _ => "P",
    }
}

fn wrap_definition_list_item(children: Vec<LayoutItem>, style: &ComputedStyle) -> Vec<LayoutItem> {
    if children.is_empty() {
        return Vec::new();
    }
    let flowables = layout_children_to_flowables(children, None);
    let container = ContainerFlowable::new_pt(flowables, style.font_size, style.root_font_size)
        .with_self_visible(style.visibility.paints())
        .with_tag_role("LI");
    vec![LayoutItem::Block {
        flowable: Box::new(container) as Box<dyn Flowable>,
        flex_grow: 0.0,
        flex_shrink: 1.0,
        width_spec: None,
        order: 0,
    }]
}

fn definition_list_children_flowables(
    node: &NodeRef,
    resolver: &StyleResolver,
    parent_style: &ComputedStyle,
    ancestors: &mut Vec<ElementInfo>,
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<Arc<AssetBundle>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
) -> Vec<LayoutItem> {
    let mut out: Vec<LayoutItem> = Vec::new();
    let mut current_li: Vec<LayoutItem> = Vec::new();
    let mut current_has_dd = false;
    let mut report = report;

    for child in node.children() {
        let child_tag = child
            .as_element()
            .map(|el| el.name.local.as_ref().to_ascii_lowercase());

        let is_dt = matches!(child_tag.as_deref(), Some("dt"));
        let is_dd = matches!(child_tag.as_deref(), Some("dd"));

        if is_dt && !current_li.is_empty() && current_has_dd {
            out.extend(wrap_definition_list_item(current_li, parent_style));
            current_li = Vec::new();
            current_has_dd = false;
        } else if !is_dt && !is_dd && !current_li.is_empty() {
            out.extend(wrap_definition_list_item(current_li, parent_style));
            current_li = Vec::new();
            current_has_dd = false;
        }

        let child_items = node_to_flowables(
            &child,
            resolver,
            parent_style,
            ancestors,
            counters,
            font_registry.clone(),
            asset_bundle.clone(),
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
            perf,
            doc_id,
        );

        if is_dt || is_dd {
            if is_dd {
                current_has_dd = true;
            }
            current_li.extend(child_items);
        } else {
            out.extend(child_items);
        }
    }

    if !current_li.is_empty() {
        out.extend(wrap_definition_list_item(current_li, parent_style));
    }

    out
}

fn layout_children_to_flowables(
    items: Vec<LayoutItem>,
    forced_line_height: Option<Pt>,
) -> Vec<Box<dyn Flowable>> {
    layout_children_to_flowables_with_options(items, forced_line_height, false, true, false)
}

fn layout_children_to_flowables_with_options(
    items: Vec<LayoutItem>,
    forced_line_height: Option<Pt>,
    prevent_soft_wrap: bool,
    snap_line_height_to_css_pixel: bool,
    anonymous_block_context: bool,
) -> Vec<Box<dyn Flowable>> {
    let mut out: Vec<Box<dyn Flowable>> = Vec::new();
    let mut inline_group: Vec<(Box<dyn Flowable>, VerticalAlign)> = Vec::new();

    for item in items {
        match item {
            LayoutItem::Inline {
                flowable, valign, ..
            } => inline_group.push((flowable, valign)),
            LayoutItem::Block { flowable, .. } => {
                if !inline_group.is_empty() {
                    out.push(Box::new(
                        InlineBlockLayoutFlowable::new_pt(
                            inline_group,
                            Pt::ZERO,
                            forced_line_height,
                        )
                        .with_css_pixel_line_snap(snap_line_height_to_css_pixel)
                        .with_anonymous_block_context(anonymous_block_context)
                        .with_no_wrap(prevent_soft_wrap),
                    ));
                    inline_group = Vec::new();
                }
                out.push(flowable);
            }
        }
    }

    if !inline_group.is_empty() {
        out.push(Box::new(
            InlineBlockLayoutFlowable::new_pt(inline_group, Pt::ZERO, forced_line_height)
                .with_css_pixel_line_snap(snap_line_height_to_css_pixel)
                .with_anonymous_block_context(anonymous_block_context)
                .with_no_wrap(prevent_soft_wrap),
        ));
    }

    out
}

fn absolute_needs_terminal_baseline_rounding(style: &ComputedStyle) -> bool {
    let line_height = style.to_text_style().line_height.to_milli_i64();
    let css_pixel = 750_i64;
    let line_height_phase = line_height.rem_euclid(css_pixel);
    !matches!(style.inset_bottom, LengthSpec::Auto)
        && matches!(style.inset_top, LengthSpec::Auto)
        && matches!(style.height, LengthSpec::Auto)
        && line_height_phase > 0
        && line_height_phase < css_pixel / 2
}

fn collapse_named_string_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn named_string_values_for_node(node: &NodeRef, style: &ComputedStyle) -> Vec<(String, String)> {
    if style.string_set.is_empty() {
        return Vec::new();
    }
    let attributes = node.as_element().map(|element| element.attributes.borrow());
    style
        .string_set
        .iter()
        .map(|assignment| {
            let value = match &assignment.source {
                NamedStringSource::Content => node.text_contents(),
                NamedStringSource::Attribute(name) => attributes
                    .as_ref()
                    .and_then(|attributes| attributes.get(name))
                    .unwrap_or("")
                    .to_string(),
                NamedStringSource::Text(value) => value.clone(),
            };
            (
                assignment.name.clone(),
                collapse_named_string_whitespace(&value),
            )
        })
        .collect()
}

fn wrap_named_string_occurrence(
    flowables: Vec<LayoutItem>,
    named_strings: Vec<(String, String)>,
) -> Vec<LayoutItem> {
    let metadata = named_strings
        .into_iter()
        .map(|(name, value)| {
            (
                format!("{}{name}", crate::canvas::META_NAMED_STRING_PREFIX),
                value,
            )
        })
        .collect::<Vec<_>>();
    flowables
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            if index != 0 {
                return item;
            }
            match item {
                LayoutItem::Block {
                    flowable,
                    flex_grow,
                    flex_shrink,
                    width_spec,
                    order,
                } => LayoutItem::Block {
                    flowable: Box::new(MetaFlowable::new(flowable, metadata.clone())),
                    flex_grow,
                    flex_shrink,
                    width_spec,
                    order,
                },
                LayoutItem::Inline {
                    flowable,
                    valign,
                    flex_grow,
                    flex_shrink,
                    width_spec,
                    order,
                } => LayoutItem::Inline {
                    flowable: Box::new(MetaFlowable::new(flowable, metadata.clone())),
                    valign,
                    flex_grow,
                    flex_shrink,
                    width_spec,
                    order,
                },
            }
        })
        .collect()
}

fn wrap_running_element(
    flowables: Vec<LayoutItem>,
    style: &ComputedStyle,
    name: Arc<str>,
    named_strings: Vec<(String, String)>,
) -> Vec<LayoutItem> {
    if flowables.is_empty() {
        return Vec::new();
    }
    let child: Box<dyn Flowable> = if flowables.len() == 1 {
        match flowables.into_iter().next().expect("one running element") {
            LayoutItem::Block { flowable, .. } | LayoutItem::Inline { flowable, .. } => flowable,
        }
    } else {
        Box::new(
            ContainerFlowable::new_pt(
                layout_children_to_flowables(flowables, None),
                style.font_size,
                style.root_font_size,
            )
            .with_self_visible(style.visibility.paints()),
        )
    };
    let marker = RunningElementFlowable::new(child, name)
        .with_named_strings(named_strings)
        .with_pagination(style.pagination);
    vec![LayoutItem::Block {
        flowable: Box::new(marker),
        flex_grow: 0.0,
        flex_shrink: 0.0,
        width_spec: None,
        order: style.order,
    }]
}

fn wrap_absolute(flowables: Vec<LayoutItem>, style: &ComputedStyle) -> Vec<LayoutItem> {
    if flowables.is_empty() {
        return Vec::new();
    }
    let boxed: Box<dyn Flowable> = if flowables.len() == 1 {
        match flowables.into_iter().next().unwrap() {
            LayoutItem::Block { flowable, .. } => flowable,
            LayoutItem::Inline { flowable, .. } => flowable,
        }
    } else {
        let flowables = layout_children_to_flowables(flowables, None);
        Box::new(
            ContainerFlowable::new_pt(flowables, style.font_size, style.root_font_size)
                .with_establishes_abs_containing_block(true)
                .with_self_visible(style.visibility.paints()),
        )
    };
    let round_terminal_baseline = absolute_needs_terminal_baseline_rounding(style);
    let explicit_grid_area = !matches!(&style.grid_column_line_start, GridLineSpec::Auto)
        || !matches!(&style.grid_column_line_end, GridLineSpec::Auto)
        || !matches!(&style.grid_row_line_start, GridLineSpec::Auto)
        || !matches!(&style.grid_row_line_end, GridLineSpec::Auto)
        || style.grid_column_start.is_some()
        || style.grid_row_start.is_some()
        || style.grid_area_name.is_some();
    let abs = AbsolutePositionedFlowable::new_pt(
        boxed,
        style.inset_left,
        style.inset_top,
        style.inset_right,
        style.inset_bottom,
        style.width,
        style.height,
        style.z_index,
        style.font_size,
        style.root_font_size,
    )
    .with_css_terminal_baseline_rounding(round_terminal_baseline)
    .with_explicit_grid_area_containing_block(explicit_grid_area)
    .with_pagination(style.pagination)
    .with_fixed_positioned(matches!(style.position, PositionMode::Fixed));
    vec![LayoutItem::Block {
        flowable: Box::new(abs) as Box<dyn Flowable>,
        flex_grow: style.flex_grow,
        flex_shrink: style.flex_shrink,
        width_spec: flex_item_basis(&style),
        order: 0,
    }]
}

fn wrap_relative(flowables: Vec<LayoutItem>, style: &ComputedStyle) -> Vec<LayoutItem> {
    if flowables.is_empty() {
        return Vec::new();
    }
    let boxed: Box<dyn Flowable> = if flowables.len() == 1 {
        match flowables.into_iter().next().unwrap() {
            LayoutItem::Block { flowable, .. } => flowable,
            LayoutItem::Inline { flowable, .. } => flowable,
        }
    } else {
        let flowables = layout_children_to_flowables(flowables, None);
        Box::new(
            ContainerFlowable::new_pt(flowables, style.font_size, style.root_font_size)
                .with_establishes_abs_containing_block(true)
                .with_self_visible(style.visibility.paints()),
        )
    };
    let rel = RelativePositionedFlowable::new_pt(
        boxed,
        style.inset_left,
        style.inset_top,
        style.inset_right,
        style.inset_bottom,
        style.font_size,
        style.root_font_size,
    )
    .with_pagination(style.pagination)
    .with_z_index(style.z_index);
    vec![LayoutItem::Block {
        flowable: Box::new(rel) as Box<dyn Flowable>,
        flex_grow: style.flex_grow,
        flex_shrink: style.flex_shrink,
        width_spec: flex_item_basis(style),
        order: 0,
    }]
}

fn wrap_float(flowables: Vec<LayoutItem>, style: &ComputedStyle) -> Vec<LayoutItem> {
    if flowables.is_empty() {
        return Vec::new();
    }
    let side = match style.float_mode {
        FloatMode::Left => FloatSide::Left,
        FloatMode::Right => FloatSide::Right,
        FloatMode::None | FloatMode::Footnote => return flowables,
    };
    let boxed: Box<dyn Flowable> = if flowables.len() == 1 {
        match flowables.into_iter().next().unwrap() {
            LayoutItem::Block { flowable, .. } | LayoutItem::Inline { flowable, .. } => flowable,
        }
    } else {
        let children = layout_children_to_flowables(flowables, None);
        Box::new(
            ContainerFlowable::new_pt(children, style.font_size, style.root_font_size)
                .with_establishes_abs_containing_block(true)
                .with_self_visible(style.visibility.paints()),
        )
    };
    vec![LayoutItem::Block {
        flowable: Box::new(FloatFlowable::new(boxed, side)) as Box<dyn Flowable>,
        flex_grow: style.flex_grow,
        flex_shrink: style.flex_shrink,
        width_spec: flex_item_basis(style),
        order: style.order,
    }]
}

fn wrap_clear(flowables: Vec<LayoutItem>, style: &ComputedStyle) -> Vec<LayoutItem> {
    if flowables.is_empty() {
        return Vec::new();
    }
    let clear = match style.clear_mode {
        ClearMode::Left => FloatClear::Left,
        ClearMode::Right => FloatClear::Right,
        ClearMode::Both => FloatClear::Both,
        ClearMode::None => return flowables,
    };
    let boxed: Box<dyn Flowable> = if flowables.len() == 1 {
        match flowables.into_iter().next().unwrap() {
            LayoutItem::Block { flowable, .. } | LayoutItem::Inline { flowable, .. } => flowable,
        }
    } else {
        let children = layout_children_to_flowables(flowables, None);
        Box::new(
            ContainerFlowable::new_pt(children, style.font_size, style.root_font_size)
                .with_establishes_abs_containing_block(true)
                .with_self_visible(style.visibility.paints()),
        )
    };
    vec![LayoutItem::Block {
        flowable: Box::new(ClearFlowable::new(boxed, clear)) as Box<dyn Flowable>,
        flex_grow: style.flex_grow,
        flex_shrink: style.flex_shrink,
        width_spec: flex_item_basis(style),
        order: style.order,
    }]
}

type StagedFlexItem = (
    i32,
    usize,
    Box<dyn Flowable>,
    f32,
    f32,
    Option<LengthSpec>,
    Option<AlignItems>,
    Option<AlignItems>,
    i32,
    i32,
);

fn push_generated_flex_item(
    staged: &mut Vec<StagedFlexItem>,
    pseudo_items: &[LayoutItem],
    source_index: usize,
    style: &ComputedStyle,
    is_grid_like: bool,
    grid_track_count: usize,
    grid_row_count: usize,
    grid_auto_flow: GridAutoFlowMode,
    grid_basis: Option<LengthSpec>,
    grid_auto_slot: &mut usize,
    grid_occupied_slots: &mut std::collections::HashSet<usize>,
) {
    if pseudo_items.is_empty() {
        return;
    }
    let grow = pseudo_items
        .iter()
        .map(LayoutItem::flex_grow)
        .fold(0.0, f32::max);
    let shrink = pseudo_items
        .iter()
        .map(LayoutItem::flex_shrink)
        .reduce(f32::max)
        .unwrap_or(1.0);
    let order = pseudo_items
        .iter()
        .map(LayoutItem::order)
        .min()
        .unwrap_or(0);
    let width_spec = pseudo_items
        .iter()
        .filter_map(LayoutItem::width_spec)
        .find(|spec| {
            !matches!(
                spec,
                LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
            )
        });
    let boxed: Box<dyn Flowable> = if pseudo_items.len() == 1 {
        match pseudo_items[0].clone() {
            LayoutItem::Block { flowable, .. } | LayoutItem::Inline { flowable, .. } => flowable,
        }
    } else {
        let flowables = layout_children_to_flowables(pseudo_items.to_vec(), None);
        Box::new(
            ContainerFlowable::new_pt(flowables, style.font_size, style.root_font_size)
                .with_self_visible(style.visibility.paints()),
        )
    };
    let z_index = boxed.z_index();
    let effective_order = if is_grid_like && grid_track_count > 0 {
        grid_item_order_slot(
            grid_track_count,
            grid_row_count,
            grid_auto_flow,
            style,
            None,
            grid_auto_slot,
            grid_occupied_slots,
        )
        .slot
    } else {
        order
    };
    let (effective_grow, effective_width_spec) = if is_grid_like && grid_track_count > 0 {
        grid_column_item_sizing(
            &style.grid_column_tracks,
            (effective_order.max(0) as usize) % grid_track_count,
            width_spec,
            grid_basis,
        )
    } else {
        (grow, width_spec)
    };
    staged.push((
        effective_order,
        source_index,
        boxed,
        effective_grow,
        if is_grid_like { 0.0 } else { shrink },
        effective_width_spec,
        None,
        None,
        order,
        z_index,
    ));
}

fn flex_container_flowables(
    node: &NodeRef,
    resolver: &StyleResolver,
    style: &ComputedStyle,
    ancestors: &mut Vec<ElementInfo>,
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<Arc<AssetBundle>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
    before_items: &[LayoutItem],
    after_items: &[LayoutItem],
) -> Vec<LayoutItem> {
    fn align_self_override(mode: AlignSelfMode) -> Option<AlignItems> {
        match mode {
            AlignSelfMode::Auto => None,
            AlignSelfMode::FlexEnd => Some(AlignItems::FlexEnd),
            AlignSelfMode::Center => Some(AlignItems::Center),
            AlignSelfMode::Stretch => Some(AlignItems::Stretch),
            AlignSelfMode::FlexStart => Some(AlignItems::FlexStart),
            AlignSelfMode::FirstBaseline => Some(AlignItems::FirstBaseline),
            AlignSelfMode::LastBaseline => Some(AlignItems::LastBaseline),
        }
    }

    fn align_items_value(mode: AlignItemsMode) -> AlignItems {
        match mode {
            AlignItemsMode::FlexEnd => AlignItems::FlexEnd,
            AlignItemsMode::Center => AlignItems::Center,
            AlignItemsMode::Stretch => AlignItems::Stretch,
            AlignItemsMode::FirstBaseline => AlignItems::FirstBaseline,
            AlignItemsMode::LastBaseline => AlignItems::LastBaseline,
            AlignItemsMode::FlexStart => AlignItems::FlexStart,
        }
    }

    let is_grid_like = matches!(style.display, DisplayMode::Grid | DisplayMode::InlineGrid);
    let vertical_grid =
        is_grid_like && !matches!(style.writing_mode, WritingModeMode::HorizontalTb);
    let grid_child_hint = if is_grid_like {
        node.children()
            .filter(|child| child.as_element().is_some())
            .count()
            .max(1)
    } else {
        0
    };
    let mut effective_grid_column_tracks = if vertical_grid {
        style.grid_row_tracks.clone()
    } else if is_grid_like {
        resolve_grid_auto_repeat_columns(style, grid_child_hint)
            .unwrap_or_else(|| style.grid_column_tracks.clone())
    } else {
        Vec::new()
    };
    let mut grid_track_count = if is_grid_like {
        if effective_grid_column_tracks.is_empty() {
            resolve_grid_track_count(style, grid_child_hint)
        } else {
            effective_grid_column_tracks.len()
        }
    } else {
        0
    };
    if is_grid_like {
        grid_track_count = grid_track_count.max(
            style
                .grid_template_areas
                .iter()
                .map(Vec::len)
                .max()
                .unwrap_or(0),
        );
        let column_names = grid_line_name_map(
            &style.grid_column_line_names,
            &style.grid_template_areas,
            true,
        );
        for child in node.children() {
            let Some(element) = child.as_element() else {
                continue;
            };
            let child_info = element_info(&child, resolver.has_sibling_selectors());
            let inline_style = element
                .attributes
                .borrow()
                .get("style")
                .map(|value| value.to_string());
            let child_style =
                resolver.compute_style(&child_info, style, inline_style.as_deref(), ancestors);
            let resolved = child_style
                .grid_area_name
                .as_deref()
                .and_then(|name| grid_area_bounds(&style.grid_template_areas, name))
                .map(|(_, _, start, end)| (start, end.saturating_sub(start).saturating_add(1)))
                .or_else(|| {
                    resolve_grid_axis(
                        &child_style.grid_column_line_start,
                        &child_style.grid_column_line_end,
                        style.grid_column_tracks.len().max(grid_track_count),
                        &column_names,
                    )
                })
                .or_else(|| {
                    child_style
                        .grid_area_name
                        .as_ref()
                        .map(|_| (grid_track_count, 1))
                });
            if let Some((start, span)) = resolved {
                grid_track_count = grid_track_count.max(start.saturating_add(span));
            }
        }
        if effective_grid_column_tracks.len() < grid_track_count {
            let explicit_count = effective_grid_column_tracks.len();
            for column in explicit_count..grid_track_count {
                let track = if style.grid_auto_column_tracks.is_empty() {
                    GridTrackSize::auto()
                } else {
                    style.grid_auto_column_tracks
                        [(column - explicit_count) % style.grid_auto_column_tracks.len()]
                };
                effective_grid_column_tracks.push(track);
            }
        }
    }
    let grid_row_count = if is_grid_like {
        style
            .grid_rows
            .or_else(|| (!style.grid_row_tracks.is_empty()).then_some(style.grid_row_tracks.len()))
            .unwrap_or_else(|| {
                grid_child_hint
                    .saturating_add(grid_track_count.saturating_sub(1))
                    .saturating_div(grid_track_count.max(1))
            })
            .max(style.grid_template_areas.len())
            .max(1)
    } else {
        0
    };
    if is_grid_like
        && matches!(
            style.grid_auto_flow,
            GridAutoFlowMode::Column | GridAutoFlowMode::ColumnDense
        )
    {
        // Column auto-flow may create implicit columns. Placement slots use a
        // row-major stride, so determine that extent with a generous bounded
        // stride before assigning the final slots; otherwise a one-column
        // explicit grid aliases (row 1, column 0) with (row 0, column 1).
        let sizing_stride = grid_track_count.max(grid_child_hint).max(1);
        let mut sizing_cursor = 0usize;
        let mut sizing_occupied = std::collections::HashSet::new();
        let mut sizing_children = Vec::new();
        for (child_index, child) in node.children().enumerate() {
            let Some(element) = child.as_element() else {
                continue;
            };
            let child_info = element_info(&child, resolver.has_sibling_selectors());
            let inline_style = element
                .attributes
                .borrow()
                .get("style")
                .map(|value| value.to_string());
            let child_style =
                resolver.compute_style(&child_info, style, inline_style.as_deref(), ancestors);
            if matches!(
                child_style.display,
                DisplayMode::None | DisplayMode::Contents
            ) || matches!(
                child_style.position,
                PositionMode::Absolute | PositionMode::Fixed
            ) {
                continue;
            }
            sizing_children.push((child_index, child_style));
        }
        sizing_children.sort_by_key(|(child_index, child)| (child.order, *child_index));
        let mut needed_columns = grid_track_count.max(1);
        for (_, child_style) in sizing_children {
            let placement = grid_item_order_slot(
                sizing_stride,
                grid_row_count,
                style.grid_auto_flow,
                style,
                Some(&child_style),
                &mut sizing_cursor,
                &mut sizing_occupied,
            );
            let slot = placement.slot.max(0) as usize;
            needed_columns =
                needed_columns.max((slot % sizing_stride).saturating_add(placement.column_span));
        }
        grid_track_count = needed_columns;
        if effective_grid_column_tracks.len() < grid_track_count {
            let explicit_count = effective_grid_column_tracks.len();
            for column in explicit_count..grid_track_count {
                let track = if style.grid_auto_column_tracks.is_empty() {
                    GridTrackSize::auto()
                } else {
                    style.grid_auto_column_tracks
                        [(column - explicit_count) % style.grid_auto_column_tracks.len()]
                };
                effective_grid_column_tracks.push(track);
            }
        }
    }
    let grid_basis = if is_grid_like && grid_track_count > 0 {
        Some(grid_track_basis(grid_track_count, style.gap))
    } else {
        None
    };

    let mut items_with_order: Vec<StagedFlexItem> = Vec::new();
    let mut grid_auto_slot = 0usize;
    let mut grid_occupied_slots: std::collections::HashSet<usize> =
        std::collections::HashSet::new();
    let mut report = report;

    push_generated_flex_item(
        &mut items_with_order,
        before_items,
        0,
        style,
        is_grid_like,
        grid_track_count,
        grid_row_count,
        style.grid_auto_flow,
        grid_basis,
        &mut grid_auto_slot,
        &mut grid_occupied_slots,
    );

    // Grid auto-placement runs in order-modified document order. Keep DOM
    // construction/counter evaluation in source order below, but precompute
    // slots when `order` actually changes that order.
    let mut grid_preplaced: HashMap<usize, GridPlacementSlot> = HashMap::new();
    if is_grid_like {
        let mut ordered_children: Vec<(usize, ComputedStyle)> = node
            .children()
            .enumerate()
            .filter_map(|(child_index, child)| {
                let element = child.as_element()?;
                let child_info = element_info(&child, resolver.has_sibling_selectors());
                let inline_style = element
                    .attributes
                    .borrow()
                    .get("style")
                    .map(|value| value.to_string());
                let child_style =
                    resolver.compute_style(&child_info, style, inline_style.as_deref(), ancestors);
                (!matches!(
                    child_style.display,
                    DisplayMode::None | DisplayMode::Contents
                ) && !matches!(
                    child_style.position,
                    PositionMode::Absolute | PositionMode::Fixed
                ))
                .then_some((child_index, child_style))
            })
            .collect();
        if ordered_children.iter().any(|(_, child)| child.order != 0) {
            ordered_children.sort_by_key(|(child_index, child)| (child.order, *child_index));
            for (child_index, child_style) in ordered_children {
                let placement = grid_item_order_slot(
                    grid_track_count,
                    grid_row_count,
                    style.grid_auto_flow,
                    style,
                    Some(&child_style),
                    &mut grid_auto_slot,
                    &mut grid_occupied_slots,
                );
                grid_preplaced.insert(child_index, placement);
            }
        }
    }

    for (child_idx, child) in node.children().enumerate() {
        let child_style = child.as_element().map(|el| {
            let child_info = element_info(&child, resolver.has_sibling_selectors());
            let inline_style = el.attributes.borrow().get("style").map(|s| s.to_string());
            resolver.compute_style(&child_info, style, inline_style.as_deref(), ancestors)
        });
        let child_items = node_to_flowables(
            &child,
            resolver,
            style,
            ancestors,
            counters,
            font_registry.clone(),
            asset_bundle.clone(),
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
            perf,
            doc_id,
        );
        if child_items.is_empty() {
            continue;
        }
        if child_style
            .as_ref()
            .is_some_and(|child_style| matches!(child_style.display, DisplayMode::Contents))
        {
            // A `display: contents` wrapper generates no flex/grid item of its
            // own. Its generated children participate directly in the parent
            // formatting context instead of becoming one boxed vertical group.
            for (contents_idx, item) in child_items.into_iter().enumerate() {
                let grow = item.flex_grow();
                let shrink = item.flex_shrink();
                let order = item.order();
                let width_spec = item.width_spec().filter(|spec| {
                    !matches!(
                        spec,
                        LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
                    )
                });
                let mut flowables = layout_children_to_flowables(vec![item], None);
                let Some(mut boxed) = flowables.pop() else {
                    continue;
                };
                if is_grid_like {
                    boxed = boxed.establish_independent_formatting_context();
                }
                let z_index = boxed.z_index();
                let effective_order = if is_grid_like {
                    grid_item_order_slot(
                        grid_track_count,
                        grid_row_count,
                        style.grid_auto_flow,
                        style,
                        None,
                        &mut grid_auto_slot,
                        &mut grid_occupied_slots,
                    )
                    .slot
                } else {
                    order
                };
                let (effective_grow, effective_width_spec) = if is_grid_like {
                    grid_column_item_sizing(
                        &effective_grid_column_tracks,
                        (effective_order.max(0) as usize) % grid_track_count.max(1),
                        width_spec,
                        grid_basis,
                    )
                } else {
                    (grow, width_spec)
                };
                items_with_order.push((
                    effective_order,
                    child_idx
                        .saturating_add(1)
                        .saturating_mul(1024)
                        .saturating_add(contents_idx),
                    boxed,
                    effective_grow,
                    if is_grid_like { 0.0 } else { shrink },
                    effective_width_spec,
                    None,
                    None,
                    order,
                    z_index,
                ));
            }
            continue;
        }
        let grow = child_items
            .iter()
            .map(|it| it.flex_grow())
            .fold(0.0, f32::max);
        let shrink = child_items
            .iter()
            .map(|it| it.flex_shrink())
            .reduce(f32::max)
            .unwrap_or(1.0);
        let order = child_items.iter().map(|it| it.order()).min().unwrap_or(0);
        let width_spec = child_items
            .iter()
            .filter_map(|it| it.width_spec())
            .find(|spec| {
                !matches!(
                    spec,
                    LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
                )
            });

        let flowables = layout_children_to_flowables(child_items, None);
        let mut boxed: Box<dyn Flowable> = if flowables.len() == 1 {
            flowables.into_iter().next().unwrap()
        } else {
            Box::new(
                ContainerFlowable::new_pt(flowables, style.font_size, style.root_font_size)
                    .with_self_visible(style.visibility.paints()),
            )
        };
        if is_grid_like {
            boxed = boxed.establish_independent_formatting_context();
        }
        let align_self = child_style
            .as_ref()
            .and_then(|child_style| align_self_override(child_style.align_self));
        let justify_self = child_style
            .as_ref()
            .and_then(|child_style| align_self_override(child_style.justify_self));
        let grid_placement = if is_grid_like && grid_track_count > 0 {
            if boxed.out_of_flow() {
                // Absolute grid children use their specified grid area as the
                // containing block but never reserve cells or move the shared
                // auto-placement cursor.
                let mut local_cursor = 0usize;
                let mut local_occupied = std::collections::HashSet::new();
                Some(grid_item_order_slot(
                    grid_track_count,
                    grid_row_count,
                    style.grid_auto_flow,
                    style,
                    child_style.as_ref(),
                    &mut local_cursor,
                    &mut local_occupied,
                ))
            } else {
                grid_preplaced.get(&child_idx).copied().or_else(|| {
                    Some(grid_item_order_slot(
                        grid_track_count,
                        grid_row_count,
                        style.grid_auto_flow,
                        style,
                        child_style.as_ref(),
                        &mut grid_auto_slot,
                        &mut grid_occupied_slots,
                    ))
                })
            }
        } else {
            None
        };
        let effective_order = grid_placement.map_or(order, |placement| placement.slot);
        if let Some(placement) = grid_placement {
            let start = placement.slot.max(0) as usize;
            let column = start % grid_track_count.max(1);
            let row = start / grid_track_count.max(1);
            let extra_width = fixed_grid_span_extra(style, column, placement.column_span, true)
                .unwrap_or(Pt::ZERO);
            let extra_height =
                fixed_grid_span_extra(style, row, placement.row_span, false).unwrap_or(Pt::ZERO);
            let equal_auto_width_span = (placement.column_span > 1
                && effective_grid_column_tracks
                    .get(column..column.saturating_add(placement.column_span))
                    .is_some_and(|tracks| {
                        tracks.iter().all(|track| {
                            matches!(
                                (track.min, track.max),
                                (GridTrackBreadth::Auto, GridTrackBreadth::Auto)
                            )
                        })
                    }))
            .then_some(placement.column_span);
            if let Some(width_span) = equal_auto_width_span {
                boxed = Box::new(ExpandedWidthFlowable::new_equal_grid_span(
                    boxed,
                    width_span,
                    extra_height,
                    placement.row_span,
                ));
            } else if extra_width > Pt::ZERO || extra_height > Pt::ZERO {
                boxed = Box::new(ExpandedWidthFlowable::new_grid_area(
                    boxed,
                    extra_width,
                    extra_height,
                    placement.column_span,
                    placement.row_span,
                ));
            }
        }
        let (effective_grow, effective_width_spec) = if is_grid_like && grid_track_count > 0 {
            grid_column_item_sizing(
                &effective_grid_column_tracks,
                (effective_order.max(0) as usize) % grid_track_count,
                width_spec,
                grid_basis,
            )
        } else {
            (grow, width_spec)
        };
        let effective_shrink = if is_grid_like { 0.0 } else { shrink };

        items_with_order.push((
            effective_order,
            child_idx.saturating_add(1).saturating_mul(1024),
            boxed,
            effective_grow,
            effective_shrink,
            effective_width_spec,
            align_self,
            justify_self,
            order,
            child_style.as_ref().map_or(0, |style| style.z_index),
        ));
    }

    push_generated_flex_item(
        &mut items_with_order,
        after_items,
        usize::MAX,
        style,
        is_grid_like,
        grid_track_count,
        grid_row_count,
        style.grid_auto_flow,
        grid_basis,
        &mut grid_auto_slot,
        &mut grid_occupied_slots,
    );

    items_with_order.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let items_with_z: Vec<(
        Box<dyn Flowable>,
        f32,
        f32,
        Option<LengthSpec>,
        Option<AlignItems>,
        Option<AlignItems>,
        i32,
    )> = if is_grid_like && grid_track_count > 0 {
        let mut padded_items: Vec<(
            Box<dyn Flowable>,
            f32,
            f32,
            Option<LengthSpec>,
            Option<AlignItems>,
            Option<AlignItems>,
            i32,
        )> = Vec::new();
        let max_slot = items_with_order
            .iter()
            .map(|(slot, _, _, _, _, _, _, _, _, _)| *slot)
            .max()
            .unwrap_or(-1)
            .max(
                grid_occupied_slots
                    .iter()
                    .copied()
                    .max()
                    .unwrap_or(0)
                    .min(i32::MAX as usize) as i32,
            )
            .max(0);
        // Materialize the trailing cells of the final grid row as synthetic
        // placeholders. Without them, flex-style row sizing gives a lone item
        // in column one the entire row width instead of only its grid track.
        let max_slot = {
            let columns = grid_track_count.max(1);
            let last = max_slot.max(0) as usize;
            last.saturating_div(columns)
                .saturating_add(1)
                .saturating_mul(columns)
                .saturating_sub(1)
                .min(i32::MAX as usize) as i32
        };
        let mut iter = items_with_order.into_iter().peekable();
        for slot in 0..=max_slot {
            let mut slot_items = Vec::new();
            loop {
                let should_take = iter
                    .peek()
                    .map(|(item_slot, _, _, _, _, _, _, _, _, _)| *item_slot == slot)
                    .unwrap_or(false);
                if !should_take {
                    break;
                }
                if let Some((
                    _,
                    _,
                    boxed,
                    grow,
                    shrink,
                    width_spec,
                    align_self,
                    justify_self,
                    paint_order,
                    z_index,
                )) = iter.next()
                {
                    slot_items.push((
                        boxed,
                        grow,
                        shrink,
                        width_spec,
                        align_self,
                        justify_self,
                        paint_order,
                        z_index,
                    ));
                }
            }
            if slot_items.is_empty() {
                let (grow, basis) = grid_column_item_sizing(
                    &effective_grid_column_tracks,
                    (slot as usize) % grid_track_count,
                    None,
                    grid_basis,
                );
                padded_items.push((
                    Box::new(Spacer::new_pt(Pt::ZERO)) as Box<dyn Flowable>,
                    grow,
                    0.0,
                    basis,
                    None,
                    None,
                    0,
                ));
            } else if slot_items.len() == 1 {
                let (boxed, grow, shrink, width_spec, align_self, justify_self, _, z_index) =
                    slot_items.pop().expect("one grid slot item");
                padded_items.push((
                    boxed,
                    grow,
                    shrink,
                    width_spec,
                    align_self,
                    justify_self,
                    z_index,
                ));
            } else {
                // Grid item painting uses order-modified document order before
                // the z-index stacking buckets are applied. Layout placement
                // and paint order are independent: explicitly overlapping
                // items share one slot but still honor `order`.
                slot_items.sort_by_key(|(_, _, _, _, _, _, paint_order, _)| *paint_order);
                let mut layers = Vec::with_capacity(slot_items.len());
                let mut slot_items = slot_items.into_iter();
                let (boxed, grow, shrink, width_spec, align_self, justify_self, _, z_index) =
                    slot_items.next().expect("overlapping grid slot items");
                layers.push((boxed, z_index));
                layers
                    .extend(slot_items.map(|(boxed, _, _, _, _, _, _, z_index)| (boxed, z_index)));
                padded_items.push((
                    Box::new(OverlayFlowable::new(layers)) as Box<dyn Flowable>,
                    grow,
                    shrink,
                    width_spec,
                    align_self,
                    justify_self,
                    0,
                ));
            }
        }
        while let Some((
            _,
            _,
            boxed,
            grow,
            shrink,
            width_spec,
            align_self,
            justify_self,
            _,
            z_index,
        )) = iter.next()
        {
            padded_items.push((
                boxed,
                grow,
                shrink,
                width_spec,
                align_self,
                justify_self,
                z_index,
            ));
        }
        padded_items
    } else {
        items_with_order
            .into_iter()
            .map(
                |(_, _, boxed, grow, shrink, width_spec, align_self, justify_self, _, z_index)| {
                    (
                        boxed,
                        grow,
                        shrink,
                        width_spec,
                        align_self,
                        justify_self,
                        z_index,
                    )
                },
            )
            .collect()
    };
    let mut items = Vec::with_capacity(items_with_z.len());
    let mut z_indices = Vec::with_capacity(items_with_z.len());
    let mut grid_justify_self = Vec::with_capacity(items_with_z.len());
    for (boxed, grow, shrink, width_spec, align_self, justify_self, z_index) in items_with_z {
        items.push((boxed, grow, shrink, width_spec, align_self));
        grid_justify_self.push(justify_self);
        z_indices.push(z_index);
    }
    let grid_wrap = is_grid_like && grid_track_count > 0;

    let vertical_flex =
        !is_grid_like && !matches!(style.writing_mode, WritingModeMode::HorizontalTb);
    let dir = if is_grid_like {
        FlexDirection::Row
    } else if vertical_flex {
        match style.flex_direction {
            FlexDirectionMode::Column | FlexDirectionMode::ColumnReverse => FlexDirection::Row,
            FlexDirectionMode::Row | FlexDirectionMode::RowReverse => FlexDirection::Column,
        }
    } else {
        match style.flex_direction {
            FlexDirectionMode::Column | FlexDirectionMode::ColumnReverse => FlexDirection::Column,
            FlexDirectionMode::Row | FlexDirectionMode::RowReverse => FlexDirection::Row,
        }
    };
    let reverse_main = if vertical_grid {
        matches!(
            style.writing_mode,
            WritingModeMode::VerticalRl | WritingModeMode::SidewaysRl
        )
    } else if is_grid_like {
        matches!(style.direction, DirectionMode::Rtl)
    } else if vertical_flex {
        match style.flex_direction {
            FlexDirectionMode::Row => matches!(style.direction, DirectionMode::Rtl),
            FlexDirectionMode::RowReverse => !matches!(style.direction, DirectionMode::Rtl),
            FlexDirectionMode::Column => matches!(
                style.writing_mode,
                WritingModeMode::VerticalRl | WritingModeMode::SidewaysRl
            ),
            FlexDirectionMode::ColumnReverse => !matches!(
                style.writing_mode,
                WritingModeMode::VerticalRl | WritingModeMode::SidewaysRl
            ),
        }
    } else {
        match style.flex_direction {
            FlexDirectionMode::Row => matches!(style.direction, DirectionMode::Rtl),
            FlexDirectionMode::RowReverse => !matches!(style.direction, DirectionMode::Rtl),
            FlexDirectionMode::Column => false,
            FlexDirectionMode::ColumnReverse => true,
        }
    };
    let reverse_cross = if vertical_grid {
        matches!(style.direction, DirectionMode::Rtl)
    } else if is_grid_like {
        false
    } else {
        match style.flex_direction {
            FlexDirectionMode::Row | FlexDirectionMode::RowReverse => matches!(
                style.writing_mode,
                WritingModeMode::VerticalRl | WritingModeMode::SidewaysRl
            ),
            FlexDirectionMode::Column | FlexDirectionMode::ColumnReverse => {
                matches!(style.direction, DirectionMode::Rtl)
            }
        }
    };
    let (physical_row_gap, physical_column_gap) = if vertical_flex || vertical_grid {
        (style.gap, style.row_gap)
    } else {
        (style.row_gap, style.gap)
    };
    let wrap_reverse = !is_grid_like && matches!(style.flex_wrap, FlexWrapMode::WrapReverse);
    let justify = match style.justify_content {
        JustifyContentMode::FlexEnd => JustifyContent::FlexEnd,
        JustifyContentMode::Center => JustifyContent::Center,
        JustifyContentMode::SafeCenter => JustifyContent::SafeCenter,
        JustifyContentMode::SpaceBetween => JustifyContent::SpaceBetween,
        JustifyContentMode::SpaceAround => JustifyContent::SpaceAround,
        JustifyContentMode::SpaceEvenly => JustifyContent::SpaceEvenly,
        _ => JustifyContent::FlexStart,
    };
    let align = align_items_value(style.align_items);
    let grid_justify_items = align_items_value(style.justify_items);
    let align_content = match style.align_content {
        AlignContentMode::FlexEnd => AlignContent::FlexEnd,
        AlignContentMode::Center => AlignContent::Center,
        AlignContentMode::Stretch => AlignContent::Stretch,
        AlignContentMode::SpaceBetween => AlignContent::SpaceBetween,
        AlignContentMode::SpaceAround => AlignContent::SpaceAround,
        AlignContentMode::SpaceEvenly => AlignContent::SpaceEvenly,
        _ => AlignContent::FlexStart,
    };
    let mut effective_grid_row_tracks = if vertical_grid {
        style.grid_column_tracks.clone()
    } else {
        style.grid_row_tracks.clone()
    };
    if is_grid_like && grid_track_count > 0 && !style.grid_auto_row_tracks.is_empty() {
        let needed_rows = items
            .len()
            .saturating_add(grid_track_count.saturating_sub(1))
            .saturating_div(grid_track_count)
            .max(grid_row_count);
        let explicit_rows = effective_grid_row_tracks.len();
        for row in explicit_rows..needed_rows {
            effective_grid_row_tracks.push(
                style.grid_auto_row_tracks
                    [(row - explicit_rows) % style.grid_auto_row_tracks.len()],
            );
        }
    }

    let flex = FlexFlowable::new_pt(
        items,
        dir,
        justify,
        align,
        align_content,
        physical_column_gap,
        if is_grid_like {
            grid_wrap
        } else {
            matches!(
                style.flex_wrap,
                FlexWrapMode::Wrap | FlexWrapMode::WrapReverse
            )
        },
        style.font_size,
        style.root_font_size,
    )
    .with_item_z_indices(z_indices)
    .with_reversals(reverse_main, wrap_reverse)
    .with_cross_reversal(reverse_cross)
    .with_css_pixel_main_axis_alignment_snap(matches!(style.display, DisplayMode::InlineFlex))
    .with_axis_gaps(physical_row_gap, physical_column_gap);
    let flex = if is_grid_like && grid_track_count > 0 {
        flex.with_grid_tracks(grid_track_count, effective_grid_row_tracks)
            .with_grid_column_tracks(effective_grid_column_tracks.clone())
            .with_grid_definite_height(!matches!(
                style.height,
                LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
            ))
            .with_grid_resolved_parent_height(
                !matches!(
                    style.min_height,
                    LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
                ) || !matches!(
                    style.max_height,
                    LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
                ),
            )
            .with_grid_item_justification(grid_justify_items, grid_justify_self)
    } else {
        flex
    };

    let inline_grid_width = if matches!(style.display, DisplayMode::InlineGrid)
        && matches!(
            style.width,
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
        )
        && !effective_grid_column_tracks.is_empty()
    {
        let fixed_tracks = effective_grid_column_tracks
            .iter()
            .copied()
            .map(|track| fixed_grid_track_length(track, true, style))
            .collect::<Option<Vec<_>>>();
        fixed_tracks.map(|tracks| {
            let gap = style
                .gap
                .resolve_width(Pt::ZERO, style.font_size, style.root_font_size)
                .max(Pt::ZERO);
            LengthSpec::Absolute(
                tracks.into_iter().fold(Pt::ZERO, |sum, track| sum + track)
                    + gap * (grid_track_count.saturating_sub(1) as i32),
            )
        })
    } else {
        None
    };
    let container_width = inline_grid_width.unwrap_or(style.width);
    let container =
        ContainerFlowable::new_pt(vec![Box::new(flex)], style.font_size, style.root_font_size)
            .with_establishes_abs_containing_block(establishes_abs_containing_block(&style))
            .with_margin(style.margin)
            .with_inline_paint_snapping(
                matches!(style.margin.left, LengthSpec::Auto)
                    && matches!(style.margin.right, LengthSpec::Auto),
            )
            .with_border(
                style.border_width,
                style.border_color.unwrap_or(style.color),
            )
            .with_border_colors(
                style.resolved_border_colors(style.color).top,
                style.resolved_border_colors(style.color).right,
                style.resolved_border_colors(style.color).bottom,
                style.resolved_border_colors(style.color).left,
            )
            .with_border_opacities(
                style.resolved_border_opacities().top,
                style.resolved_border_opacities().right,
                style.resolved_border_opacities().bottom,
                style.resolved_border_opacities().left,
            )
            .with_border_styles(
                style.resolved_border_styles().top,
                style.resolved_border_styles().right,
                style.resolved_border_styles().bottom,
                style.resolved_border_styles().left,
            )
            .with_border_radius(style.border_radius)
            .with_box_decoration_break(style.box_decoration_break)
            .with_border_image(style.border_image.clone())
            .with_outline(
                style.outline_width,
                style.outline_offset,
                style.outline_style,
                style.resolved_outline_color(),
                style.outline_visible,
            )
            .with_padding(style.padding)
            .with_box_sizing(style.box_sizing)
            .with_width(container_width)
            .with_max_width(style.max_width)
            .with_min_width(style.min_width)
            .with_height(style.height)
            .with_aspect_ratio(style.aspect_ratio)
            .with_min_height(style.min_height)
            .with_max_height(style.max_height)
            .with_background(style.background_source_color.or(style.background_color))
            .with_background_opacity(style.background_alpha)
            .with_background_paint(style.background_paint.clone())
            .with_background_layers(
                style.background_paints.clone(),
                style.background_sizes.clone(),
                style.background_positions.clone(),
                style.background_repeats.clone(),
                style.background_attachments.clone(),
                style.background_origins.clone(),
                style.background_clips.clone(),
            )
            .with_background_blend_modes(style.background_blend_modes.clone())
            .with_clip_path(style.clip_path.clone())
            .with_clip_path_reference_box(style.clip_path_reference_box)
            .with_legacy_clip(effective_legacy_clip(style))
            .with_box_shadows(style.box_shadows.clone())
            .with_paint_filter(style.paint_filter.clone())
            .with_backdrop_filter(style.backdrop_filter.clone())
            .with_will_change_backdrop_root(style.will_change_backdrop_root)
            .with_mask(style.mask.clone())
            .with_mask_backdrop_root(style.mask_backdrop_root)
            .with_mix_blend_mode(style.mix_blend_mode)
            .with_isolation(style.isolation)
            .with_opacity(style.opacity)
            .with_transforms(style.transform.clone())
            .with_transform_origin(style.transform_origin)
            .with_transform_box(style.transform_box)
            .with_perspective(style.perspective, style.perspective_origin)
            .with_transform_style(style.transform_style)
            .with_overflow_modes(style.overflow_x, style.overflow_y)
            .with_overflow_clip_margin(style.overflow_clip_margin)
            .with_scrollbar_gutter(style.scrollbar_gutter, style.direction, style.writing_mode)
            .with_line_clamp(
                style.line_clamp,
                text_style_for_flow_text(&style).line_height,
            )
            .with_self_visible(style.visibility.paints())
            .with_pagination(style.pagination);

    vec![LayoutItem::Block {
        flowable: Box::new(container) as Box<dyn Flowable>,
        flex_grow: style.flex_grow,
        flex_shrink: style.flex_shrink,
        width_spec: flex_item_basis(&style),
        order: 0,
    }]
}

fn is_table_container_display(display: DisplayMode) -> bool {
    matches!(display, DisplayMode::Table | DisplayMode::InlineTable)
}

fn is_table_row_group_display(display: DisplayMode) -> bool {
    matches!(
        display,
        DisplayMode::TableRowGroup | DisplayMode::TableHeaderGroup | DisplayMode::TableFooterGroup
    )
}

fn is_table_column_display(display: DisplayMode) -> bool {
    matches!(
        display,
        DisplayMode::TableColumnGroup | DisplayMode::TableColumn
    )
}

fn table_group_role(display: DisplayMode) -> &'static str {
    match display {
        DisplayMode::TableHeaderGroup => "THead",
        DisplayMode::TableFooterGroup => "TFoot",
        _ => "TBody",
    }
}

fn node_inline_style_attr(node: &NodeRef) -> Option<String> {
    node.as_element().and_then(|element| {
        element
            .attributes
            .borrow()
            .get("style")
            .map(|s| s.to_string())
    })
}

fn table_caption_flowable_from_node(
    node: &NodeRef,
    caption_style: &ComputedStyle,
    mut report: Option<&mut GlyphCoverageReport>,
    font_registry: Option<Arc<FontRegistry>>,
) -> Option<Box<dyn Flowable>> {
    let mut caption_text = extract_text(node, caption_style.white_space);
    if caption_text.trim().is_empty() {
        return container_flowable_with_role(Vec::new(), caption_style, Some("Caption"));
    }
    if !preserve_whitespace(caption_style.white_space) {
        caption_text = caption_text.trim().to_string();
    }
    let caption_text = apply_text_transform(&caption_text, caption_style.text_transform);
    let caption_text_style = text_style_for_flow_text(caption_style);
    report_missing_glyphs(
        report.as_deref_mut(),
        font_registry.as_deref(),
        &caption_text_style,
        &caption_text,
    );
    let paragraph = Paragraph::new(caption_text)
        .with_style(caption_text_style)
        .with_align(text_align_from_style(caption_style))
        .with_last_align(text_align_last_from_style(caption_style))
        .with_whitespace(
            preserve_whitespace(caption_style.white_space),
            no_wrap(caption_style),
        )
        .with_break_spaces(matches!(
            caption_style.white_space,
            WhiteSpaceMode::BreakSpaces
        ))
        .with_pagination(caption_style.pagination)
        .with_font_registry(font_registry)
        .with_tag_role("Caption");

    container_flowable_with_role(
        vec![LayoutItem::Block {
            flowable: Box::new(paragraph) as Box<dyn Flowable>,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            width_spec: None,
            order: 0,
        }],
        caption_style,
        Some("Caption"),
    )
}

fn collect_css_table_collapsed_columns(
    node: &NodeRef,
    resolver: &StyleResolver,
    table_style: &ComputedStyle,
    ancestors: &[ElementInfo],
    include_prev_siblings: bool,
) -> Vec<bool> {
    let mut collapsed_columns = Vec::new();

    for child in node.children() {
        if child.as_element().is_none() {
            continue;
        }
        let mut child_info = element_info(&child, include_prev_siblings);
        let child_inline_style = node_inline_style_attr(&child);
        let child_style = resolver.compute_style(
            &child_info,
            table_style,
            child_inline_style.as_deref(),
            ancestors,
        );

        match child_style.display {
            DisplayMode::TableColumn => {
                collapsed_columns.push(matches!(child_style.visibility, VisibilityMode::Collapse));
            }
            DisplayMode::TableColumnGroup => {
                let group_collapsed = matches!(child_style.visibility, VisibilityMode::Collapse);
                child_info.apply_computed_container_style(&child_style);
                let mut group_ancestors = ancestors.to_vec();
                group_ancestors.push(child_info);
                let before_group = collapsed_columns.len();

                for col_node in child.children() {
                    if col_node.as_element().is_none() {
                        continue;
                    }
                    let col_info = element_info(&col_node, include_prev_siblings);
                    let col_inline_style = node_inline_style_attr(&col_node);
                    let col_style = resolver.compute_style(
                        &col_info,
                        &child_style,
                        col_inline_style.as_deref(),
                        &group_ancestors,
                    );
                    if matches!(col_style.display, DisplayMode::TableColumn) {
                        collapsed_columns.push(
                            group_collapsed
                                || matches!(col_style.visibility, VisibilityMode::Collapse),
                        );
                    }
                }

                if collapsed_columns.len() == before_group {
                    collapsed_columns.push(group_collapsed);
                }
            }
            _ => {}
        }
    }

    collapsed_columns
}

#[allow(clippy::too_many_arguments)]
fn native_css_table_container_flowables(
    node: &NodeRef,
    resolver: &StyleResolver,
    style: &ComputedStyle,
    ancestors: &[ElementInfo],
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<Arc<AssetBundle>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
) -> Option<Vec<LayoutItem>> {
    let mut report = report;
    let include_prev_siblings = resolver.has_sibling_selectors();
    let mut has_table_structure = false;
    let mut top_caption_flowables: Vec<Box<dyn Flowable>> = Vec::new();
    let mut bottom_caption_flowables: Vec<Box<dyn Flowable>> = Vec::new();

    for child in node.children() {
        let Some(_) = child.as_element() else {
            continue;
        };
        let child_info = element_info(&child, include_prev_siblings);
        let inline_style = node_inline_style_attr(&child);
        let child_style =
            resolver.compute_style(&child_info, style, inline_style.as_deref(), ancestors);
        if matches!(child_style.display, DisplayMode::None) {
            continue;
        }
        if matches!(
            child_style.display,
            DisplayMode::TableRow
                | DisplayMode::TableCell
                | DisplayMode::TableRowGroup
                | DisplayMode::TableHeaderGroup
                | DisplayMode::TableFooterGroup
                | DisplayMode::TableColumn
                | DisplayMode::TableColumnGroup
        ) {
            has_table_structure = true;
        }
        if matches!(child_style.display, DisplayMode::TableCaption) {
            has_table_structure = true;
            if let Some(caption) = table_caption_flowable_from_node(
                &child,
                &child_style,
                report.as_deref_mut(),
                font_registry.clone(),
            ) {
                if matches!(
                    child_style.caption_side,
                    crate::style::CaptionSideMode::Bottom
                ) {
                    bottom_caption_flowables.push(caption);
                } else {
                    top_caption_flowables.push(caption);
                }
            }
        }
    }
    if !has_table_structure {
        return None;
    }

    let effective_table_layout = if matches!(style.table_layout, TableLayoutMode::Fixed)
        && matches!(
            style.width,
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
        ) {
        TableLayoutMode::Auto
    } else {
        style.table_layout
    };
    let minimum_table_height = match style.height {
        LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial => Pt::ZERO,
        height => height
            .resolve_height(Pt::ZERO, style.font_size, style.root_font_size)
            .max(Pt::ZERO),
    };
    let table_border_colors = style.resolved_border_colors(style.color);
    let table_border_styles = style.resolved_border_styles();
    let table_hidden_borders = style.border_hidden_sides();
    let collapsed_table = matches!(
        style.border_collapse,
        crate::flowable::BorderCollapseMode::Collapse
    );
    let mut table_ancestors = ancestors.to_vec();
    let table = table_flowable(
        node,
        style,
        resolver,
        &mut table_ancestors,
        counters,
        font_registry,
        asset_bundle,
        report.as_deref_mut(),
        svg_form,
        svg_raster_fallback,
        perf,
        doc_id,
    )
    .with_tag_role("Table")
    .with_border_collapse(style.border_collapse)
    .with_border_spacing(style.border_spacing)
    .with_table_layout(effective_table_layout)
    .with_direction(style.direction)
    .with_table_border(
        style.border_width,
        style.border_color.unwrap_or(style.color),
    )
    .with_table_border_colors(
        table_border_colors.top,
        table_border_colors.right,
        table_border_colors.bottom,
        table_border_colors.left,
    )
    .with_table_border_styles(
        table_border_styles.top,
        table_border_styles.right,
        table_border_styles.bottom,
        table_border_styles.left,
    )
    .with_table_hidden_borders(
        table_hidden_borders.top,
        table_hidden_borders.right,
        table_hidden_borders.bottom,
        table_hidden_borders.left,
    )
    .with_font_metrics(style.font_size, style.root_font_size)
    .with_minimum_height(minimum_table_height);

    let has_caption = !top_caption_flowables.is_empty() || !bottom_caption_flowables.is_empty();
    let caption_width_overflow = match style.width {
        LengthSpec::Absolute(width) if has_caption => table.collapsed_caption_width_overflow(width),
        _ => Pt::ZERO,
    };
    if caption_width_overflow > Pt::ZERO {
        top_caption_flowables = top_caption_flowables
            .into_iter()
            .map(|caption| {
                Box::new(ExpandedWidthFlowable::new(caption, caption_width_overflow))
                    as Box<dyn Flowable>
            })
            .collect();
        bottom_caption_flowables = bottom_caption_flowables
            .into_iter()
            .map(|caption| {
                Box::new(ExpandedWidthFlowable::new(caption, caption_width_overflow))
                    as Box<dyn Flowable>
            })
            .collect();
    }
    let mut table_children: Vec<Box<dyn Flowable>> = Vec::new();
    table_children.extend(top_caption_flowables);
    table_children.push(Box::new(table));
    table_children.extend(bottom_caption_flowables);
    let used_table_width = if matches!(
        style.width,
        LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
    ) {
        table_children
            .iter()
            .filter_map(|child| child.intrinsic_width())
            .reduce(Pt::max)
            .map(LengthSpec::Absolute)
            .unwrap_or(style.width)
    } else {
        style.width
    };

    let container =
        ContainerFlowable::new_pt(table_children, style.font_size, style.root_font_size)
            .with_establishes_abs_containing_block(establishes_abs_containing_block(style))
            .with_margin(style.margin)
            .with_border(
                if collapsed_table {
                    EdgeSizes::zero()
                } else {
                    style.border_width
                },
                style.border_color.unwrap_or(style.color),
            )
            .with_border_colors(
                table_border_colors.top,
                table_border_colors.right,
                table_border_colors.bottom,
                table_border_colors.left,
            )
            .with_border_opacities(
                style.resolved_border_opacities().top,
                style.resolved_border_opacities().right,
                style.resolved_border_opacities().bottom,
                style.resolved_border_opacities().left,
            )
            .with_border_styles(
                table_border_styles.top,
                table_border_styles.right,
                table_border_styles.bottom,
                table_border_styles.left,
            )
            .with_border_radius(style.border_radius)
            .with_box_decoration_break(style.box_decoration_break)
            .with_border_image(style.border_image.clone())
            .with_outline(
                style.outline_width,
                style.outline_offset,
                style.outline_style,
                style.resolved_outline_color(),
                style.outline_visible,
            )
            .with_padding(style.padding)
            .with_box_sizing(style.box_sizing)
            .with_width(used_table_width)
            .with_max_width(style.max_width)
            .with_min_width(style.min_width)
            .with_height(style.height)
            .with_aspect_ratio(style.aspect_ratio)
            .with_min_height(style.min_height)
            .with_max_height(style.max_height)
            .with_background(style.background_source_color.or(style.background_color))
            .with_background_opacity(style.background_alpha)
            .with_background_paint(style.background_paint.clone())
            .with_background_layers(
                style.background_paints.clone(),
                style.background_sizes.clone(),
                style.background_positions.clone(),
                style.background_repeats.clone(),
                style.background_attachments.clone(),
                style.background_origins.clone(),
                style.background_clips.clone(),
            )
            .with_background_blend_modes(style.background_blend_modes.clone())
            .with_clip_path(style.clip_path.clone())
            .with_clip_path_reference_box(style.clip_path_reference_box)
            .with_legacy_clip(effective_legacy_clip(style))
            .with_box_shadows(style.box_shadows.clone())
            .with_paint_filter(style.paint_filter.clone())
            .with_backdrop_filter(style.backdrop_filter.clone())
            .with_will_change_backdrop_root(style.will_change_backdrop_root)
            .with_mask(style.mask.clone())
            .with_mask_backdrop_root(style.mask_backdrop_root)
            .with_mix_blend_mode(style.mix_blend_mode)
            .with_isolation(style.isolation)
            .with_opacity(style.opacity)
            .with_transforms(style.transform.clone())
            .with_transform_origin(style.transform_origin)
            .with_transform_box(style.transform_box)
            .with_perspective(style.perspective, style.perspective_origin)
            .with_transform_style(style.transform_style)
            .with_overflow_modes(style.overflow_x, style.overflow_y)
            .with_overflow_clip_margin(style.overflow_clip_margin)
            .with_scrollbar_gutter(style.scrollbar_gutter, style.direction, style.writing_mode)
            .with_line_clamp(
                style.line_clamp,
                text_style_for_flow_text(&style).line_height,
            )
            .with_self_visible(style.visibility.paints())
            .with_pagination(style.pagination);

    let flowable = Box::new(container) as Box<dyn Flowable>;
    Some(if matches!(style.display, DisplayMode::InlineTable) {
        vec![LayoutItem::Inline {
            flowable,
            valign: vertical_align_from_style(style),
            flex_grow: style.flex_grow,
            flex_shrink: style.flex_shrink,
            width_spec: flex_item_basis(style),
            order: 0,
        }]
    } else {
        vec![LayoutItem::Block {
            flowable,
            flex_grow: style.flex_grow,
            flex_shrink: style.flex_shrink,
            width_spec: flex_item_basis(style),
            order: 0,
        }]
    })
}

fn table_container_flowables(
    node: &NodeRef,
    resolver: &StyleResolver,
    style: &ComputedStyle,
    ancestors: &[ElementInfo],
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<Arc<AssetBundle>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
    before_items: &[LayoutItem],
    after_items: &[LayoutItem],
) -> Vec<LayoutItem> {
    let mut report = report;
    if before_items.is_empty() && after_items.is_empty() {
        if let Some(native) = native_css_table_container_flowables(
            node,
            resolver,
            style,
            ancestors,
            counters,
            font_registry.clone(),
            asset_bundle.clone(),
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
            perf,
            doc_id,
        ) {
            return native;
        }
    }
    let include_prev_siblings = resolver.has_sibling_selectors();
    let mut table_children: Vec<Box<dyn Flowable>> = Vec::new();
    let mut header_group_flowables: Vec<Box<dyn Flowable>> = Vec::new();
    let mut footer_group_flowables: Vec<Box<dyn Flowable>> = Vec::new();
    let mut top_caption_flowables: Vec<Box<dyn Flowable>> = Vec::new();
    let mut bottom_caption_flowables: Vec<Box<dyn Flowable>> = Vec::new();
    let mut anon_cells: Vec<(NodeRef, ComputedStyle)> = Vec::new();
    let mut has_proper_table_child = false;
    let mut has_improper_table_child = false;
    let collapsed_columns = collect_css_table_collapsed_columns(
        node,
        resolver,
        style,
        ancestors,
        include_prev_siblings,
    );

    if !before_items.is_empty() {
        has_improper_table_child = true;
        table_children.extend(layout_children_to_flowables(before_items.to_vec(), None));
    }

    for child in node.children() {
        let Some(child_element) = child.as_element() else {
            continue;
        };
        let child_info = element_info(&child, include_prev_siblings);
        let child_inline_style = child_element
            .attributes
            .borrow()
            .get("style")
            .map(|s| s.to_string());
        let child_style =
            resolver.compute_style(&child_info, style, child_inline_style.as_deref(), ancestors);
        if matches!(child_style.display, DisplayMode::None) {
            continue;
        }
        if style_can_mutate_counters(&child_style) {
            apply_style_counters(&child_style, counters);
        }

        if matches!(child_style.display, DisplayMode::TableCell) {
            has_proper_table_child = true;
            anon_cells.push((child.clone(), child_style));
            continue;
        }

        if let Some(row_flowable) = table_row_flowable_from_cells(
            std::mem::take(&mut anon_cells),
            style,
            resolver,
            ancestors,
            counters,
            font_registry.clone(),
            asset_bundle.clone(),
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
            perf,
            doc_id,
            &collapsed_columns,
        ) {
            table_children.push(row_flowable);
        }

        if is_table_column_display(child_style.display) {
            has_proper_table_child = true;
            continue;
        }

        if matches!(child_style.display, DisplayMode::TableCaption) {
            has_proper_table_child = true;
            if let Some(caption_flowable) = table_caption_flowable_from_node(
                &child,
                &child_style,
                report.as_deref_mut(),
                font_registry.clone(),
            ) {
                if matches!(
                    child_style.caption_side,
                    crate::style::CaptionSideMode::Bottom
                ) {
                    bottom_caption_flowables.push(caption_flowable);
                } else {
                    top_caption_flowables.push(caption_flowable);
                }
            }
            continue;
        }

        if matches!(child_style.display, DisplayMode::TableRow) {
            has_proper_table_child = true;
            if matches!(child_style.visibility, VisibilityMode::Collapse) {
                continue;
            }
            if let Some(row_flowable) = table_row_flowable_from_node(
                &child,
                &child_style,
                resolver,
                ancestors,
                counters,
                font_registry.clone(),
                asset_bundle.clone(),
                report.as_deref_mut(),
                svg_form,
                svg_raster_fallback,
                perf,
                doc_id,
                &collapsed_columns,
            ) {
                table_children.push(row_flowable);
            }
            continue;
        }

        if is_table_row_group_display(child_style.display) {
            has_proper_table_child = true;
            let group_collapsed = matches!(child_style.visibility, VisibilityMode::Collapse);
            let mut group_rows: Vec<Box<dyn Flowable>> = Vec::new();
            for row_node in child.children() {
                let Some(row_element) = row_node.as_element() else {
                    continue;
                };
                let row_info = element_info(&row_node, include_prev_siblings);
                let row_inline_style = row_element
                    .attributes
                    .borrow()
                    .get("style")
                    .map(|s| s.to_string());
                let row_style = resolver.compute_style(
                    &row_info,
                    &child_style,
                    row_inline_style.as_deref(),
                    ancestors,
                );
                if !matches!(row_style.display, DisplayMode::TableRow) {
                    continue;
                }
                apply_style_counters(&row_style, counters);
                if group_collapsed || matches!(row_style.visibility, VisibilityMode::Collapse) {
                    continue;
                }
                if let Some(row_flowable) = table_row_flowable_from_node(
                    &row_node,
                    &row_style,
                    resolver,
                    ancestors,
                    counters,
                    font_registry.clone(),
                    asset_bundle.clone(),
                    report.as_deref_mut(),
                    svg_form,
                    svg_raster_fallback,
                    perf,
                    doc_id,
                    &collapsed_columns,
                ) {
                    group_rows.push(row_flowable);
                }
            }
            if !group_rows.is_empty() {
                let group_items: Vec<LayoutItem> = group_rows
                    .into_iter()
                    .map(|flowable| LayoutItem::Block {
                        flowable,
                        flex_grow: 0.0,
                        flex_shrink: 1.0,
                        width_spec: None,
                        order: 0,
                    })
                    .collect();
                if let Some(group_flowable) = container_flowable_with_role(
                    group_items,
                    &child_style,
                    Some(table_group_role(child_style.display)),
                ) {
                    match child_style.display {
                        DisplayMode::TableHeaderGroup => {
                            header_group_flowables.push(group_flowable);
                        }
                        DisplayMode::TableFooterGroup => {
                            footer_group_flowables.push(group_flowable);
                        }
                        _ => {
                            table_children.push(group_flowable);
                        }
                    }
                }
            }
            continue;
        }

        has_improper_table_child = true;
        let mut child_ancestors = ancestors.to_vec();
        let child_items = node_to_flowables(
            &child,
            resolver,
            style,
            &mut child_ancestors,
            counters,
            font_registry.clone(),
            asset_bundle.clone(),
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
            perf,
            doc_id,
        );
        table_children.extend(layout_children_to_flowables(child_items, None));
    }

    if let Some(row_flowable) = table_row_flowable_from_cells(
        std::mem::take(&mut anon_cells),
        style,
        resolver,
        ancestors,
        counters,
        font_registry.clone(),
        asset_bundle.clone(),
        report.as_deref_mut(),
        svg_form,
        svg_raster_fallback,
        perf,
        doc_id,
        &collapsed_columns,
    ) {
        table_children.push(row_flowable);
    }
    if !after_items.is_empty() {
        has_improper_table_child = true;
        table_children.extend(layout_children_to_flowables(after_items.to_vec(), None));
    }

    let mut ordered_table_children = Vec::with_capacity(
        top_caption_flowables.len()
            + header_group_flowables.len()
            + table_children.len()
            + footer_group_flowables.len()
            + bottom_caption_flowables.len(),
    );
    ordered_table_children.extend(top_caption_flowables);
    ordered_table_children.extend(header_group_flowables);
    ordered_table_children.extend(table_children);
    ordered_table_children.extend(footer_group_flowables);
    ordered_table_children.extend(bottom_caption_flowables);
    if has_improper_table_child && !has_proper_table_child {
        ordered_table_children =
            anonymous_table_sequence_with_spacing(ordered_table_children, style);
    }

    let table_items: Vec<LayoutItem> = ordered_table_children
        .into_iter()
        .map(|flowable| LayoutItem::Block {
            flowable,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            width_spec: None,
            order: 0,
        })
        .collect();

    // CSS 2.1 defines a table's computed `height` as a minimum height for the
    // table box. The generic path is used for anonymous-table fixup (including
    // generated content), so leaving the authored height as a fixed container
    // height clips overflowing anonymous rows instead of growing the table.
    let mut table_box_style = style.clone();
    if !matches!(
        style.height,
        LengthSpec::Auto
            | LengthSpec::Inherit
            | LengthSpec::Initial
            | LengthSpec::Content
            | LengthSpec::MinContent
            | LengthSpec::MaxContent
            | LengthSpec::FitContent
    ) {
        table_box_style.height = LengthSpec::Auto;
        let table_height_minimum = match (style.height, style.max_height) {
            // A table's `height` is a content-driven minimum, but it is still
            // bounded by an applicable max-height. Converting it directly into
            // an ordinary min-height would invoke the generic min-wins conflict
            // rule and incorrectly keep an overlarge anonymous table box.
            (LengthSpec::Absolute(height), LengthSpec::Absolute(maximum)) => {
                LengthSpec::Absolute(height.min(maximum))
            }
            _ => style.height,
        };
        table_box_style.min_height = match (table_height_minimum, style.min_height) {
            (LengthSpec::Absolute(height), LengthSpec::Absolute(minimum)) => {
                LengthSpec::Absolute(height.max(minimum))
            }
            (_, LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial) => {
                table_height_minimum
            }
            // Retain an explicit min-height when the two authored constraints
            // cannot be combined without the containing block available.
            (_, minimum) => minimum,
        };
    }

    let Some(table_flowable) =
        container_flowable_with_role(table_items, &table_box_style, Some("Table"))
    else {
        return Vec::new();
    };

    if matches!(style.display, DisplayMode::InlineTable) {
        let valign = vertical_align_from_style(style);
        vec![LayoutItem::Inline {
            flowable: table_flowable,
            valign,
            flex_grow: style.flex_grow,
            flex_shrink: style.flex_shrink,
            width_spec: flex_item_basis(style),
            order: 0,
        }]
    } else {
        vec![LayoutItem::Block {
            flowable: table_flowable,
            flex_grow: style.flex_grow,
            flex_shrink: style.flex_shrink,
            width_spec: flex_item_basis(style),
            order: 0,
        }]
    }
}

fn anonymous_table_sequence_with_spacing(
    children: Vec<Box<dyn Flowable>>,
    style: &ComputedStyle,
) -> Vec<Box<dyn Flowable>> {
    if children.is_empty()
        || matches!(
            style.border_collapse,
            crate::flowable::BorderCollapseMode::Collapse
        )
        || style.border_spacing == BorderSpacingSpec::zero()
    {
        return children;
    }

    // Consecutive improper children of a table generate one anonymous row and
    // one anonymous cell around the sequence. In the separated-border model,
    // that cell is inset by one border-spacing interval on every table edge.
    let padding = EdgeSizes {
        top: style.border_spacing.vertical,
        right: style.border_spacing.horizontal,
        bottom: style.border_spacing.vertical,
        left: style.border_spacing.horizontal,
    };
    vec![Box::new(
        ContainerFlowable::new_pt(children, style.font_size, style.root_font_size)
            .with_padding(padding)
            .with_self_visible(style.visibility.paints()),
    ) as Box<dyn Flowable>]
}

fn table_row_flowable_from_node(
    row_node: &NodeRef,
    row_style: &ComputedStyle,
    resolver: &StyleResolver,
    ancestors: &[ElementInfo],
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<Arc<AssetBundle>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
    collapsed_columns: &[bool],
) -> Option<Box<dyn Flowable>> {
    let include_prev_siblings = resolver.has_sibling_selectors();
    let mut cells: Vec<(NodeRef, ComputedStyle)> = Vec::new();
    for cell_node in row_node.children() {
        let Some(cell_element) = cell_node.as_element() else {
            continue;
        };
        let cell_info = element_info(&cell_node, include_prev_siblings);
        let cell_inline_style = cell_element
            .attributes
            .borrow()
            .get("style")
            .map(|s| s.to_string());
        let cell_style = resolver.compute_style(
            &cell_info,
            row_style,
            cell_inline_style.as_deref(),
            ancestors,
        );
        if matches!(cell_style.display, DisplayMode::None) {
            continue;
        }
        if style_can_mutate_counters(&cell_style) {
            apply_style_counters(&cell_style, counters);
        }
        cells.push((cell_node.clone(), cell_style));
    }
    table_row_flowable_from_cells(
        cells,
        row_style,
        resolver,
        ancestors,
        counters,
        font_registry,
        asset_bundle,
        report,
        svg_form,
        svg_raster_fallback,
        perf,
        doc_id,
        collapsed_columns,
    )
}

fn table_row_flowable_from_cells(
    cells: Vec<(NodeRef, ComputedStyle)>,
    row_style: &ComputedStyle,
    resolver: &StyleResolver,
    ancestors: &[ElementInfo],
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<Arc<AssetBundle>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
    collapsed_columns: &[bool],
) -> Option<Box<dyn Flowable>> {
    if cells.is_empty() {
        return None;
    }
    let mut report = report;
    let original_cell_count = cells.len().max(1) as f32;
    let has_collapsed_columns = cells
        .iter()
        .enumerate()
        .any(|(index, _)| collapsed_columns.get(index).copied().unwrap_or(false));
    let mut row_items: Vec<(
        Box<dyn Flowable>,
        f32,
        f32,
        Option<LengthSpec>,
        Option<AlignItems>,
    )> = Vec::new();

    for (cell_index, (cell_node, cell_style)) in cells.into_iter().enumerate() {
        if collapsed_columns.get(cell_index).copied().unwrap_or(false) {
            continue;
        }
        let mut cell_ancestors = ancestors.to_vec();
        let cell_items = node_to_flowables(
            &cell_node,
            resolver,
            row_style,
            &mut cell_ancestors,
            counters,
            font_registry.clone(),
            asset_bundle.clone(),
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
            perf,
            doc_id,
        );
        let mut cell_flowables = layout_children_to_flowables(cell_items, None);
        let cell_flowable: Box<dyn Flowable> = if cell_flowables.is_empty() {
            Box::new(
                ContainerFlowable::new_pt(
                    Vec::new(),
                    cell_style.font_size,
                    cell_style.root_font_size,
                )
                .with_establishes_abs_containing_block(establishes_abs_containing_block(
                    &cell_style,
                ))
                .with_self_visible(cell_style.visibility.paints()),
            )
        } else if cell_flowables.len() == 1 {
            cell_flowables.remove(0)
        } else {
            Box::new(
                ContainerFlowable::new_pt(
                    cell_flowables,
                    cell_style.font_size,
                    cell_style.root_font_size,
                )
                .with_establishes_abs_containing_block(establishes_abs_containing_block(
                    &cell_style,
                ))
                .with_self_visible(cell_style.visibility.paints()),
            )
        };
        let explicit_width = !matches!(cell_style.width, LengthSpec::Auto);
        let width_spec = if explicit_width {
            Some(cell_style.width)
        } else {
            Some(LengthSpec::Percent(1.0 / original_cell_count))
        };
        row_items.push((
            cell_flowable,
            if explicit_width || has_collapsed_columns {
                0.0
            } else {
                1.0
            },
            1.0,
            width_spec,
            None,
        ));
    }
    if row_items.is_empty() {
        return None;
    }

    let row_core: Box<dyn Flowable> = Box::new(FlexFlowable::new_pt(
        row_items,
        FlexDirection::Row,
        JustifyContent::FlexStart,
        AlignItems::Stretch,
        AlignContent::FlexStart,
        row_style.gap,
        false,
        row_style.font_size,
        row_style.root_font_size,
    ));
    let row_wrapped = container_flowable_with_role(
        vec![LayoutItem::Block {
            flowable: row_core.clone(),
            flex_grow: 0.0,
            flex_shrink: 1.0,
            width_spec: None,
            order: 0,
        }],
        row_style,
        Some("TR"),
    )
    .unwrap_or(row_core);
    Some(row_wrapped)
}

fn resolve_grid_track_count(style: &ComputedStyle, child_hint: usize) -> usize {
    if !style.grid_column_tracks.is_empty() {
        return style.grid_column_tracks.len();
    }
    if let Some(columns) = style.grid_columns {
        return columns.max(1);
    }
    if !style.grid_row_tracks.is_empty() {
        let rows = style.grid_row_tracks.len().max(1);
        return child_hint
            .saturating_add(rows.saturating_sub(1))
            .saturating_div(rows)
            .max(1);
    }
    if let Some(rows) = style.grid_rows {
        let rows = rows.max(1);
        return child_hint
            .saturating_add(rows.saturating_sub(1))
            .saturating_div(rows)
            .max(1);
    }
    1
}

fn resolve_grid_auto_repeat_columns(
    style: &ComputedStyle,
    child_hint: usize,
) -> Option<Vec<GridTrackSize>> {
    let repeat = style.grid_column_auto_repeat.as_ref()?;
    let available = match style.width {
        LengthSpec::Absolute(value) => value,
        LengthSpec::Em(value) => style.font_size * value,
        LengthSpec::Rem(value) => style.root_font_size * value,
        LengthSpec::Calc(calc) if calc.percent.abs() <= f32::EPSILON => {
            calc.resolve(Pt::ZERO, style.font_size, style.root_font_size)
        }
        _ => return None,
    }
    .max(Pt::ZERO);
    let gap = match style.gap {
        LengthSpec::Percent(value) => available * value,
        value => value.resolve_width(available, style.font_size, style.root_font_size),
    }
    .max(Pt::ZERO);
    let mut pattern_width = Pt::ZERO;
    for track in &repeat.tracks {
        let minimum = match track.min {
            GridTrackBreadth::Length(value) => {
                value.resolve_width(available, style.font_size, style.root_font_size)
            }
            _ => track.fixed_breadth()?.resolve_width(
                available,
                style.font_size,
                style.root_font_size,
            ),
        };
        pattern_width += minimum.max(Pt::ZERO);
    }
    pattern_width += gap * (repeat.tracks.len().saturating_sub(1) as i32);
    if pattern_width <= Pt::ZERO {
        return None;
    }
    let repetition_stride = pattern_width + gap;
    let mut repetitions =
        ((available + gap).to_f32() / repetition_stride.to_f32()).floor() as usize;
    repetitions = repetitions.max(1).min(4096);
    if matches!(repeat.mode, GridAutoRepeatMode::Fit) {
        let occupied_repetitions = child_hint
            .saturating_add(repeat.tracks.len().saturating_sub(1))
            .saturating_div(repeat.tracks.len().max(1))
            .max(1);
        repetitions = repetitions.min(occupied_repetitions);
    }
    let mut tracks = Vec::with_capacity(repeat.tracks.len().saturating_mul(repetitions));
    for _ in 0..repetitions {
        tracks.extend(repeat.tracks.iter().copied());
    }
    Some(tracks)
}

fn grid_column_item_sizing(
    tracks: &[GridTrackSize],
    column: usize,
    item_width: Option<LengthSpec>,
    fallback_basis: Option<LengthSpec>,
) -> (f32, Option<LengthSpec>) {
    let Some(track) = tracks.get(column).copied() else {
        return (0.0, fallback_basis.or(item_width));
    };

    if let Some(factor) = track.fraction_factor() {
        let track_minimum = match track.min {
            GridTrackBreadth::Length(length) => Some(length),
            _ => None,
        };
        // A definite grid-item width contributes to an auto minimum track.
        // Preserve it as the lower bound for the fractional-track allocator;
        // otherwise two `1fr` tracks incorrectly split evenly even when one
        // item has a larger specified size.
        let item_minimum = item_width.filter(|width| {
            !matches!(
                width,
                LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
            )
        });
        let basis = item_minimum.or(track_minimum);
        return (factor, basis);
    }
    if let Some(length) = track.fixed_breadth() {
        return (0.0, Some(length));
    }
    match (track.min, track.max) {
        (GridTrackBreadth::Auto, GridTrackBreadth::Auto) => (1.0, item_width),
        (GridTrackBreadth::MinContent, GridTrackBreadth::MinContent) => {
            (0.0, Some(LengthSpec::MinContent))
        }
        (GridTrackBreadth::MaxContent, GridTrackBreadth::MaxContent) => {
            (0.0, Some(LengthSpec::MaxContent))
        }
        _ => (0.0, item_width),
    }
}

fn grid_track_basis(track_count: usize, gap: LengthSpec) -> LengthSpec {
    let columns = track_count.max(1) as f32;
    let base_percent = 1.0 / columns;
    if track_count <= 1 {
        return LengthSpec::Percent(base_percent);
    }

    let gap_share = (track_count.saturating_sub(1) as f32) / columns;
    let mut calc = CalcLength::zero();
    calc.percent = base_percent;

    match gap {
        LengthSpec::Absolute(value) => {
            calc.abs = -(value * gap_share);
        }
        LengthSpec::Percent(value) => {
            calc.percent -= value * gap_share;
        }
        LengthSpec::Em(value) => {
            calc.em = -(value * gap_share);
        }
        LengthSpec::Rem(value) => {
            calc.rem = -(value * gap_share);
        }
        LengthSpec::Calc(value) => {
            calc.abs = -(value.abs * gap_share);
            calc.percent -= value.percent * gap_share;
            calc.em = -(value.em * gap_share);
            calc.rem = -(value.rem * gap_share);
        }
        LengthSpec::Clamped(_) => {}
        LengthSpec::FontRelative(_) => {}
        LengthSpec::Auto
        | LengthSpec::Content
        | LengthSpec::MinContent
        | LengthSpec::MaxContent
        | LengthSpec::FitContent
        | LengthSpec::Inherit
        | LengthSpec::Initial => {}
    }

    LengthSpec::Calc(calc)
}

fn fixed_grid_track_length(
    track: GridTrackSize,
    horizontal: bool,
    style: &ComputedStyle,
) -> Option<Pt> {
    let length = track.fixed_breadth()?;
    match length {
        LengthSpec::Percent(_) => None,
        LengthSpec::Calc(CalcLength { percent, .. }) if percent.abs() > f32::EPSILON => None,
        _ => Some(
            if horizontal {
                length.resolve_width(Pt::ZERO, style.font_size, style.root_font_size)
            } else {
                length.resolve_height(Pt::ZERO, style.font_size, style.root_font_size)
            }
            .max(Pt::ZERO),
        ),
    }
}

fn fixed_grid_span_extra(
    style: &ComputedStyle,
    start: usize,
    span: usize,
    horizontal: bool,
) -> Option<Pt> {
    if span <= 1 {
        return Some(Pt::ZERO);
    }
    let (explicit, automatic, gap) = if horizontal {
        (
            &style.grid_column_tracks,
            &style.grid_auto_column_tracks,
            style.gap,
        )
    } else {
        (
            &style.grid_row_tracks,
            &style.grid_auto_row_tracks,
            style.row_gap,
        )
    };
    let gap = match gap {
        LengthSpec::Percent(_) => return None,
        LengthSpec::Calc(CalcLength { percent, .. }) if percent.abs() > f32::EPSILON => {
            return None;
        }
        _ => if horizontal {
            gap.resolve_width(Pt::ZERO, style.font_size, style.root_font_size)
        } else {
            gap.resolve_height(Pt::ZERO, style.font_size, style.root_font_size)
        }
        .max(Pt::ZERO),
    };
    let mut extra = gap * (span.saturating_sub(1) as i32);
    for track_index in start.saturating_add(1)..start.saturating_add(span) {
        let track = explicit.get(track_index).copied().or_else(|| {
            (!automatic.is_empty())
                .then(|| automatic[(track_index.saturating_sub(explicit.len())) % automatic.len()])
        })?;
        extra += fixed_grid_track_length(track, horizontal, style)?;
    }
    Some(extra)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GridPlacementSlot {
    slot: i32,
    column_span: usize,
    row_span: usize,
}

fn grid_area_bounds(
    areas: &[Vec<Option<String>>],
    name: &str,
) -> Option<(usize, usize, usize, usize)> {
    let mut bounds = None;
    for (row, cells) in areas.iter().enumerate() {
        for (column, cell) in cells.iter().enumerate() {
            if cell.as_deref() != Some(name) {
                continue;
            }
            bounds = Some(bounds.map_or(
                (row, row, column, column),
                |(r0, r1, c0, c1): (usize, usize, usize, usize)| {
                    (r0.min(row), r1.max(row), c0.min(column), c1.max(column))
                },
            ));
        }
    }
    bounds
}

fn grid_line_name_map(
    names: &[Vec<String>],
    areas: &[Vec<Option<String>>],
    columns: bool,
) -> HashMap<String, Vec<usize>> {
    let mut map: HashMap<String, Vec<usize>> = HashMap::new();
    for (line, line_names) in names.iter().enumerate() {
        for name in line_names {
            let entries = map.entry(name.clone()).or_default();
            if entries.last().copied() != Some(line) {
                entries.push(line);
            }
        }
    }
    let mut area_names = std::collections::HashSet::new();
    for name in areas.iter().flatten().flatten() {
        area_names.insert(name.as_str());
    }
    for name in area_names {
        if let Some((r0, r1, c0, c1)) = grid_area_bounds(areas, name) {
            let (start, end) = if columns {
                (c0, c1.saturating_add(1))
            } else {
                (r0, r1.saturating_add(1))
            };
            map.entry(format!("{name}-start")).or_default().push(start);
            map.entry(format!("{name}-end")).or_default().push(end);
        }
    }
    map
}

fn resolve_grid_line(
    line: &GridLineSpec,
    explicit_tracks: usize,
    names: &HashMap<String, Vec<usize>>,
) -> Option<usize> {
    match line {
        GridLineSpec::Line(line) if *line > 0 => Some((*line - 1) as usize),
        GridLineSpec::Line(line) if *line < 0 => Some(
            explicit_tracks
                .saturating_add(1)
                .saturating_sub((-*line) as usize),
        ),
        GridLineSpec::Named(name) => names.get(name).and_then(|lines| lines.first().copied()),
        GridLineSpec::NamedOccurrence { occurrence, name } if *occurrence > 0 => names
            .get(name)
            .and_then(|lines| lines.get((*occurrence as usize).saturating_sub(1)).copied()),
        GridLineSpec::NamedOccurrence { occurrence, name } if *occurrence < 0 => names
            .get(name)
            .and_then(|lines| {
                lines
                    .iter()
                    .rev()
                    .nth(((-*occurrence) as usize).saturating_sub(1))
            })
            .copied(),
        _ => None,
    }
}

fn resolve_grid_axis(
    start: &GridLineSpec,
    end: &GridLineSpec,
    explicit_tracks: usize,
    names: &HashMap<String, Vec<usize>>,
) -> Option<(usize, usize)> {
    let span = |line: &GridLineSpec| match line {
        GridLineSpec::Span(count) | GridLineSpec::SpanNamed { count, .. } => Some((*count).max(1)),
        _ => None,
    };
    let start_line = resolve_grid_line(start, explicit_tracks, names);
    let end_line = resolve_grid_line(end, explicit_tracks, names);
    match (start_line, end_line) {
        (Some(start), Some(end)) => {
            let (lo, hi) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            Some((lo, hi.saturating_sub(lo).max(1)))
        }
        (Some(start), None) => {
            let width = match end {
                GridLineSpec::SpanNamed { count, name } => {
                    let lines = names.get(name).map(Vec::as_slice).unwrap_or(&[]);
                    lines
                        .iter()
                        .copied()
                        .filter(|line| *line > start)
                        .nth(count.saturating_sub(1))
                        .map(|line| line.saturating_sub(start))
                        .unwrap_or((*count).max(1))
                }
                _ => span(end).unwrap_or(1),
            };
            Some((start, width.max(1)))
        }
        (None, Some(end)) => {
            let width = match start {
                GridLineSpec::SpanNamed { count, name } => names
                    .get(name)
                    .and_then(|lines| {
                        lines
                            .iter()
                            .rev()
                            .copied()
                            .filter(|line| *line < end)
                            .nth(count.saturating_sub(1))
                    })
                    .map(|line| end.saturating_sub(line))
                    .unwrap_or((*count).max(1)),
                _ => span(start).unwrap_or(1),
            };
            Some((end.saturating_sub(width), width.max(1)))
        }
        (None, None) => None,
    }
}

fn grid_item_order_slot(
    track_count: usize,
    row_count: usize,
    auto_flow: GridAutoFlowMode,
    container_style: &ComputedStyle,
    child_style: Option<&ComputedStyle>,
    auto_cursor: &mut usize,
    occupied_slots: &mut std::collections::HashSet<usize>,
) -> GridPlacementSlot {
    let columns = track_count.max(1);
    let rows = row_count.max(1);
    let column_flow = matches!(
        auto_flow,
        GridAutoFlowMode::Column | GridAutoFlowMode::ColumnDense
    );
    let dense = matches!(
        auto_flow,
        GridAutoFlowMode::RowDense | GridAutoFlowMode::ColumnDense
    );
    let sequence_slot = |sequence: usize| {
        if column_flow {
            let row = sequence % rows;
            let column = sequence / rows;
            row.saturating_mul(columns).saturating_add(column)
        } else {
            sequence
        }
    };
    let column_names = grid_line_name_map(
        &container_style.grid_column_line_names,
        &container_style.grid_template_areas,
        true,
    );
    let row_names = grid_line_name_map(
        &container_style.grid_row_line_names,
        &container_style.grid_template_areas,
        false,
    );
    let mut resolved_column = child_style.and_then(|style| {
        resolve_grid_axis(
            &style.grid_column_line_start,
            &style.grid_column_line_end,
            container_style.grid_column_tracks.len().max(columns),
            &column_names,
        )
    });
    let mut resolved_row = child_style.and_then(|style| {
        resolve_grid_axis(
            &style.grid_row_line_start,
            &style.grid_row_line_end,
            container_style.grid_row_tracks.len().max(rows),
            &row_names,
        )
    });
    if let Some(area_name) = child_style.and_then(|style| style.grid_area_name.as_deref()) {
        if let Some((r0, r1, c0, c1)) =
            grid_area_bounds(&container_style.grid_template_areas, area_name)
        {
            resolved_column = Some((c0, c1.saturating_sub(c0).saturating_add(1)));
            resolved_row = Some((r0, r1.saturating_sub(r0).saturating_add(1)));
        } else {
            let start = GridLineSpec::Named(format!("{area_name}-start"));
            let end = GridLineSpec::Named(format!("{area_name}-end"));
            resolved_column = resolve_grid_axis(
                &start,
                &end,
                container_style.grid_column_tracks.len().max(columns),
                &column_names,
            );
            resolved_row = resolve_grid_axis(
                &start,
                &end,
                container_style.grid_row_tracks.len().max(rows),
                &row_names,
            );
            // Unknown named areas generate implicit lines at the far edge of
            // the explicit grid. Their default auto tracks remain zero-sized
            // for empty items instead of falling back to auto-placement in an
            // explicit cell.
            if resolved_column.is_none() {
                resolved_column = Some((container_style.grid_column_tracks.len(), 1));
            }
            if resolved_row.is_none() {
                resolved_row = Some((container_style.grid_row_tracks.len(), 1));
            }
        }
    }
    let column_span = resolved_column.map_or_else(
        || {
            child_style.map_or(1, |style| {
                match (&style.grid_column_line_start, &style.grid_column_line_end) {
                    (GridLineSpec::Span(span), _) | (_, GridLineSpec::Span(span)) => (*span).max(1),
                    (GridLineSpec::SpanNamed { count, .. }, _)
                    | (_, GridLineSpec::SpanNamed { count, .. }) => (*count).max(1),
                    _ => 1,
                }
            })
        },
        |(_, span)| span,
    );
    let row_span = resolved_row.map_or_else(
        || {
            child_style.map_or(1, |style| {
                match (&style.grid_row_line_start, &style.grid_row_line_end) {
                    (GridLineSpec::Span(span), _) | (_, GridLineSpec::Span(span)) => (*span).max(1),
                    (GridLineSpec::SpanNamed { count, .. }, _)
                    | (_, GridLineSpec::SpanNamed { count, .. }) => (*count).max(1),
                    _ => 1,
                }
            })
        },
        |(_, span)| span,
    );
    let style_row = resolved_row.map(|(start, _)| start);
    let style_column = resolved_column.map(|(start, _)| start);
    let fully_definite = style_row.is_some() && style_column.is_some();
    let mut sequence = if dense && !fully_definite {
        0
    } else if style_row.is_some() && style_column.is_none() && !column_flow {
        style_row.unwrap_or(0).saturating_mul(columns)
    } else {
        *auto_cursor
    };
    let assigned_slot = loop {
        let candidate = sequence_slot(sequence);
        let auto_row = candidate.saturating_div(columns);
        let auto_column = candidate.checked_rem(columns).unwrap_or(0);
        let row = style_row.unwrap_or(auto_row);
        let column = style_column.unwrap_or(auto_column);
        if !column_flow && style_column.is_none() && column.saturating_add(column_span) > columns {
            sequence = sequence.saturating_add(1);
            continue;
        }
        if column_flow && style_row.is_none() && row.saturating_add(row_span) > rows {
            sequence = sequence.saturating_add(1);
            continue;
        }
        let slot = row.saturating_mul(columns).saturating_add(column);
        let fits = (row..row.saturating_add(row_span)).all(|occupied_row| {
            (column..column.saturating_add(column_span)).all(|occupied_column| {
                !occupied_slots.contains(
                    &occupied_row
                        .saturating_mul(columns)
                        .saturating_add(occupied_column),
                )
            })
        });
        if fully_definite || fits {
            break slot;
        }
        sequence = if style_column.is_some() && !column_flow {
            sequence.saturating_add(columns)
        } else if style_row.is_some() && column_flow {
            sequence.saturating_add(rows)
        } else {
            sequence.saturating_add(1)
        };
    };
    let assigned_row = assigned_slot / columns;
    let assigned_column = assigned_slot % columns;
    for row in assigned_row..assigned_row.saturating_add(row_span) {
        for column in assigned_column..assigned_column.saturating_add(column_span) {
            occupied_slots.insert(row.saturating_mul(columns).saturating_add(column));
        }
    }
    // The auto-placement cursor advances past earlier definite placements in
    // document order, while later definite items may still intentionally use
    // the same slot (grid overlap).
    if !fully_definite {
        let assigned_sequence = if column_flow {
            assigned_column
                .saturating_mul(rows)
                .saturating_add(assigned_row)
        } else {
            assigned_slot
        };
        if !(style_row.is_some() && style_column.is_none() && !column_flow) {
            *auto_cursor = (*auto_cursor).max(assigned_sequence.saturating_add(1));
        }
        while occupied_slots.contains(&sequence_slot(*auto_cursor)) {
            *auto_cursor = auto_cursor.saturating_add(1);
        }
    }

    let placement = GridPlacementSlot {
        slot: assigned_slot.min(i32::MAX as usize) as i32,
        column_span,
        row_span,
    };
    if !matches!(container_style.writing_mode, WritingModeMode::HorizontalTb) {
        let physical_columns = rows.max(1);
        let block = assigned_row;
        let inline = assigned_column;
        // Transpose logical axes here. Flex geometry performs the block-axis
        // reversal for vertical-rl so overflowing tracks anchor to the right
        // edge and extend toward the physical left.
        let physical_column = block;
        let physical_slot = inline
            .saturating_mul(physical_columns)
            .saturating_add(physical_column);
        return GridPlacementSlot {
            slot: physical_slot.min(i32::MAX as usize) as i32,
            column_span: row_span,
            row_span: column_span,
        };
    }
    placement
}

fn table_flowable(
    node: &NodeRef,
    style: &ComputedStyle,
    resolver: &StyleResolver,
    ancestors: &mut Vec<ElementInfo>,
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    asset_bundle: Option<Arc<AssetBundle>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
) -> TableFlowable {
    let mut report = report;
    let legacy_cell_padding = legacy_table_length_attribute(node, "cellpadding");
    let legacy_border_width =
        legacy_table_length_attribute(node, "border").filter(|width| *width > Pt::ZERO);
    let mut header_rows: Vec<Vec<TableCell>> = Vec::new();
    let mut body_rows: Vec<Vec<TableCell>> = Vec::new();
    let mut body_row_meta: Vec<Vec<(String, String)>> = Vec::new();
    let mut body_row_pagination = Vec::new();
    let mut body_row_keep_ranges = Vec::new();
    let mut footer_row_count = 0usize;
    let mut row_style_ms = 0.0;
    let mut cell_style_ms = 0.0;
    let mut cell_text_ms = 0.0;
    let mut cell_report_ms = 0.0;
    let mut row_info_ms = 0.0;
    let mut cell_info_ms = 0.0;
    let mut row_style_cache_hit = 0u64;
    let mut row_style_cache_miss = 0u64;
    let mut cell_style_cache_hit = 0u64;
    let mut cell_style_cache_miss = 0u64;
    let mut cell_report_calls = 0u64;
    let mut row_info_calls = 0u64;
    let mut cell_info_calls = 0u64;
    let mut row_count = 0u64;
    let mut cell_count = 0u64;
    let mut text_chars = 0u64;
    let t_table = std::time::Instant::now();

    fn length_spec_is_zero(spec: LengthSpec) -> bool {
        match spec {
            LengthSpec::Absolute(v) => v <= Pt::ZERO,
            LengthSpec::Percent(v) => v <= 0.0,
            LengthSpec::Em(v) => v <= 0.0,
            LengthSpec::Rem(v) => v <= 0.0,
            LengthSpec::Calc(calc) => {
                calc.abs <= Pt::ZERO && calc.percent <= 0.0 && calc.em <= 0.0 && calc.rem <= 0.0
            }
            LengthSpec::Clamped(_) => false,
            LengthSpec::FontRelative(_) => false,
            LengthSpec::Auto
            | LengthSpec::Content
            | LengthSpec::MinContent
            | LengthSpec::MaxContent
            | LengthSpec::FitContent
            | LengthSpec::Inherit
            | LengthSpec::Initial => true,
        }
    }

    fn resolve_non_auto_height(spec: LengthSpec, font_size: Pt, root_font_size: Pt) -> Pt {
        match spec {
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial => Pt::ZERO,
            _ => spec
                .resolve_height(Pt::ZERO, font_size, root_font_size)
                .max(Pt::ZERO),
        }
    }

    #[derive(Clone)]
    struct TableRowInput {
        node: NodeRef,
        anonymous_cells: Option<Vec<NodeRef>>,
        section: TableRowSection,
        in_explicit_row_group: bool,
        group_row_index: usize,
        group_row_count: usize,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum TableRowSection {
        Header,
        Body,
        Footer,
    }

    fn collect_rows_from_group(
        group: &NodeRef,
        group_style: &ComputedStyle,
        section: TableRowSection,
        resolver: &StyleResolver,
        ancestors: &mut Vec<ElementInfo>,
        include_prev_siblings: bool,
    ) -> Vec<TableRowInput> {
        let mut raw_rows: Vec<(NodeRef, Option<Vec<NodeRef>>)> = Vec::new();
        let mut anonymous_cells: Vec<NodeRef> = Vec::new();
        let flush_anonymous = |rows: &mut Vec<(NodeRef, Option<Vec<NodeRef>>)>,
                               cells: &mut Vec<NodeRef>| {
            if let Some(first) = cells.first().cloned() {
                rows.push((first, Some(std::mem::take(cells))));
            }
        };

        for child in group.children() {
            let Some(element) = child.as_element() else {
                continue;
            };
            let tag = element.name.local.as_ref();
            let child_info = element_info(&child, include_prev_siblings);
            let inline_style = node_inline_style_attr(&child);
            let child_style = resolver.compute_style(
                &child_info,
                group_style,
                inline_style.as_deref(),
                ancestors,
            );
            if matches!(child_style.display, DisplayMode::None) {
                continue;
            }
            if tag == "tr" || matches!(child_style.display, DisplayMode::TableRow) {
                flush_anonymous(&mut raw_rows, &mut anonymous_cells);
                raw_rows.push((child, None));
            } else if !is_table_column_display(child_style.display)
                && !matches!(child_style.display, DisplayMode::TableCaption)
            {
                // CSS table fixup wraps improper row-group children in an
                // anonymous row, then in anonymous cells.  Keeping the
                // authored node as the cell preserves its sizing and paint.
                anonymous_cells.push(child);
            }
        }
        flush_anonymous(&mut raw_rows, &mut anonymous_cells);

        let group_row_count = raw_rows.len().max(1);
        raw_rows
            .into_iter()
            .enumerate()
            .map(|(index, (node, anonymous_cells))| TableRowInput {
                node,
                anonymous_cells,
                section,
                in_explicit_row_group: true,
                group_row_index: index + 1,
                group_row_count,
            })
            .collect()
    }

    fn collect_rows(
        table: &NodeRef,
        table_style: &ComputedStyle,
        resolver: &StyleResolver,
        ancestors: &mut Vec<ElementInfo>,
        include_prev_siblings: bool,
        out: &mut Vec<TableRowInput>,
    ) {
        let mut header_rows = Vec::new();
        let mut body_rows = Vec::new();
        let mut footer_rows = Vec::new();
        let mut direct_rows: Vec<(NodeRef, Option<Vec<NodeRef>>)> = Vec::new();
        let mut anonymous_cells: Vec<NodeRef> = Vec::new();
        let mut saw_header_group = false;
        let flush_direct_anonymous =
            |rows: &mut Vec<(NodeRef, Option<Vec<NodeRef>>)>, cells: &mut Vec<NodeRef>| {
                if let Some(first) = cells.first().cloned() {
                    rows.push((first, Some(std::mem::take(cells))));
                }
            };

        for child in table.children() {
            let Some(element) = child.as_element() else {
                continue;
            };
            let tag = element.name.local.as_ref();
            let mut child_info = element_info(&child, include_prev_siblings);
            let inline_style = node_inline_style_attr(&child);
            let child_style = resolver.compute_style(
                &child_info,
                table_style,
                inline_style.as_deref(),
                ancestors,
            );
            if matches!(child_style.display, DisplayMode::None) {
                continue;
            }

            if matches!(tag, "thead" | "tbody" | "tfoot")
                || is_table_row_group_display(child_style.display)
            {
                flush_direct_anonymous(&mut direct_rows, &mut anonymous_cells);
                child_info.apply_computed_container_style(&child_style);
                ancestors.push(child_info);
                let mut section = match (tag, child_style.display) {
                    ("thead", _) | (_, DisplayMode::TableHeaderGroup) => TableRowSection::Header,
                    ("tfoot", _) | (_, DisplayMode::TableFooterGroup) => TableRowSection::Footer,
                    _ => TableRowSection::Body,
                };
                if section == TableRowSection::Header {
                    if saw_header_group {
                        // CSS only repeats the first header group. Later
                        // table-header-group boxes participate as ordinary
                        // rows in visual order.
                        section = TableRowSection::Body;
                    } else {
                        saw_header_group = true;
                    }
                }
                let rows = collect_rows_from_group(
                    &child,
                    &child_style,
                    section,
                    resolver,
                    ancestors,
                    include_prev_siblings,
                );
                ancestors.pop();
                match section {
                    TableRowSection::Header => header_rows.extend(rows),
                    TableRowSection::Body => body_rows.extend(rows),
                    TableRowSection::Footer => footer_rows.extend(rows),
                }
            } else if tag == "tr" || matches!(child_style.display, DisplayMode::TableRow) {
                flush_direct_anonymous(&mut direct_rows, &mut anonymous_cells);
                direct_rows.push((child, None));
            } else if matches!(tag, "td" | "th")
                || matches!(child_style.display, DisplayMode::TableCell)
            {
                anonymous_cells.push(child);
            }
        }
        flush_direct_anonymous(&mut direct_rows, &mut anonymous_cells);

        let direct_row_count = direct_rows.len().max(1);
        body_rows.extend(direct_rows.into_iter().enumerate().map(
            |(index, (node, anonymous_cells))| TableRowInput {
                node,
                anonymous_cells,
                section: TableRowSection::Body,
                in_explicit_row_group: false,
                group_row_index: index + 1,
                group_row_count: direct_row_count,
            },
        ));

        // Header and footer groups have a visual order independent of source
        // order. The table fragmenter receives the section identity below so
        // it can repeat footers without treating them as ordinary body rows.
        out.extend(header_rows);
        out.extend(body_rows);
        out.extend(footer_rows);
    }

    fn span_attr(node: &NodeRef) -> usize {
        node.as_element()
            .and_then(|el| {
                el.attributes
                    .borrow()
                    .get("span")
                    .and_then(|raw| raw.trim().parse::<usize>().ok())
            })
            .filter(|value| *value > 0)
            .unwrap_or(1)
    }

    fn inline_style_attr(node: &NodeRef) -> Option<String> {
        node.as_element()
            .and_then(|el| el.attributes.borrow().get("style").map(|s| s.to_string()))
    }

    fn column_width_hint_from_style(style: &ComputedStyle) -> Option<TableColumnWidthHint> {
        if matches!(
            style.width,
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
        ) {
            None
        } else {
            Some(TableColumnWidthHint::new(
                style.width,
                style.font_size,
                style.root_font_size,
            ))
        }
    }

    fn column_border_from_style(style: &ComputedStyle) -> Option<TableColumnBorder> {
        let hidden = style.border_hidden_sides();
        let has_visible_width = !length_spec_is_zero(style.border_width.top)
            || !length_spec_is_zero(style.border_width.right)
            || !length_spec_is_zero(style.border_width.bottom)
            || !length_spec_is_zero(style.border_width.left);
        let has_hidden_border = hidden.top || hidden.right || hidden.bottom || hidden.left;
        if !has_visible_width && !has_hidden_border {
            return None;
        }
        let colors = style.resolved_border_colors(style.color);
        let styles = style.resolved_border_styles();
        Some(TableColumnBorder::new(
            style.border_width,
            crate::flowable::ResolvedEdgeColors {
                top: colors.top,
                right: colors.right,
                bottom: colors.bottom,
                left: colors.left,
            },
            crate::flowable::ResolvedEdgeStyles {
                top: styles.top,
                right: styles.right,
                bottom: styles.bottom,
                left: styles.left,
            },
            crate::flowable::ResolvedEdgeHidden {
                top: hidden.top,
                right: hidden.right,
                bottom: hidden.bottom,
                left: hidden.left,
            },
            style.font_size,
            style.root_font_size,
        ))
    }

    fn append_column_hint(
        out: &mut Vec<Option<TableColumnWidthHint>>,
        hint: Option<TableColumnWidthHint>,
        span: usize,
    ) {
        for _ in 0..span.max(1) {
            out.push(hint);
        }
    }

    fn append_column_border(
        out: &mut Vec<Option<TableColumnBorder>>,
        border: Option<TableColumnBorder>,
        span: usize,
    ) {
        for _ in 0..span.max(1) {
            out.push(border);
        }
    }

    fn append_column_group_border(
        out: &mut Vec<Option<TableColumnGroupBorder>>,
        border: Option<TableColumnBorder>,
        span: usize,
        group_offset: usize,
        group_span: usize,
    ) {
        let span = span.max(1);
        let group_span = group_span.max(1);
        for i in 0..span {
            let col_offset = group_offset.saturating_add(i);
            out.push(border.map(|border| {
                TableColumnGroupBorder::new(
                    border,
                    col_offset == 0,
                    col_offset.saturating_add(1) == group_span,
                )
            }));
        }
    }

    fn append_column_collapsed(out: &mut Vec<bool>, collapsed: bool, span: usize) {
        for _ in 0..span.max(1) {
            out.push(collapsed);
        }
    }

    fn append_column_background(out: &mut Vec<Option<Color>>, color: Option<Color>, span: usize) {
        for _ in 0..span.max(1) {
            out.push(color);
        }
    }

    fn collect_column_metadata(
        table: &NodeRef,
        table_style: &ComputedStyle,
        resolver: &StyleResolver,
        ancestors: &mut Vec<ElementInfo>,
        include_prev_siblings: bool,
    ) -> (
        Vec<Option<TableColumnWidthHint>>,
        Vec<Option<TableColumnBorder>>,
        Vec<Option<TableColumnGroupBorder>>,
        Vec<Option<Color>>,
        Vec<bool>,
    ) {
        let column_nodes: Vec<NodeRef> = table
            .children()
            .filter(|child| child.as_element().is_some())
            .collect();
        let child_count = column_nodes.len().max(1);
        let mut hints: Vec<Option<TableColumnWidthHint>> = Vec::new();
        let mut borders: Vec<Option<TableColumnBorder>> = Vec::new();
        let mut group_borders: Vec<Option<TableColumnGroupBorder>> = Vec::new();
        let mut backgrounds: Vec<Option<Color>> = Vec::new();
        let mut collapsed_columns: Vec<bool> = Vec::new();
        let mut prev_infos: Vec<ElementInfo> = Vec::new();

        for (child_index, child) in column_nodes.iter().enumerate() {
            let Some(element) = child.as_element() else {
                continue;
            };
            let tag = element.name.local.as_ref();
            let base_info =
                element_info_basic(child, child_index + 1, child_count, false, Vec::new());
            let mut info = if include_prev_siblings {
                let mut with_prev = base_info.clone();
                with_prev.prev_siblings = prev_infos.clone();
                prev_infos.push(base_info);
                with_prev
            } else {
                base_info
            };
            let inline_style = inline_style_attr(child);
            let computed =
                resolver.compute_style(&info, table_style, inline_style.as_deref(), ancestors);
            info.apply_computed_container_style(&computed);

            if tag == "col" || matches!(computed.display, DisplayMode::TableColumn) {
                let span = span_attr(child);
                append_column_hint(&mut hints, column_width_hint_from_style(&computed), span);
                append_column_border(&mut borders, column_border_from_style(&computed), span);
                append_column_group_border(&mut group_borders, None, span, 0, span);
                append_column_background(&mut backgrounds, computed.background_color, span);
                append_column_collapsed(
                    &mut collapsed_columns,
                    matches!(computed.visibility, VisibilityMode::Collapse),
                    span,
                );
            } else if tag == "colgroup" || matches!(computed.display, DisplayMode::TableColumnGroup)
            {
                let group_collapsed = matches!(computed.visibility, VisibilityMode::Collapse);
                ancestors.push(info);
                let cols: Vec<NodeRef> = child
                    .children()
                    .filter(|col| col.as_element().is_some())
                    .collect();
                if cols.is_empty() {
                    let span = span_attr(child);
                    append_column_hint(&mut hints, column_width_hint_from_style(&computed), span);
                    append_column_border(&mut borders, None, span);
                    append_column_group_border(
                        &mut group_borders,
                        column_border_from_style(&computed),
                        span,
                        0,
                        span,
                    );
                    append_column_background(&mut backgrounds, computed.background_color, span);
                    append_column_collapsed(
                        &mut collapsed_columns,
                        matches!(computed.visibility, VisibilityMode::Collapse),
                        span,
                    );
                } else {
                    let col_count = cols.len().max(1);
                    let group_span = cols.iter().map(span_attr).sum::<usize>().max(1);
                    let group_border = column_border_from_style(&computed);
                    let mut group_offset = 0usize;
                    let mut prev_col_infos: Vec<ElementInfo> = Vec::new();
                    for (col_index, col) in cols.iter().enumerate() {
                        let base_col_info =
                            element_info_basic(col, col_index + 1, col_count, false, Vec::new());
                        let col_info = if include_prev_siblings {
                            let mut with_prev = base_col_info.clone();
                            with_prev.prev_siblings = prev_col_infos.clone();
                            prev_col_infos.push(base_col_info);
                            with_prev
                        } else {
                            base_col_info
                        };
                        let col_inline_style = inline_style_attr(col);
                        let col_style = resolver.compute_style(
                            &col_info,
                            &computed,
                            col_inline_style.as_deref(),
                            ancestors,
                        );
                        let col_tag = col
                            .as_element()
                            .map(|element| element.name.local.as_ref() == "col")
                            .unwrap_or(false);
                        if !col_tag && !matches!(col_style.display, DisplayMode::TableColumn) {
                            continue;
                        }
                        append_column_hint(
                            &mut hints,
                            column_width_hint_from_style(&col_style),
                            span_attr(col),
                        );
                        let span = span_attr(col);
                        append_column_border(
                            &mut borders,
                            column_border_from_style(&col_style),
                            span,
                        );
                        append_column_group_border(
                            &mut group_borders,
                            group_border,
                            span,
                            group_offset,
                            group_span,
                        );
                        append_column_background(
                            &mut backgrounds,
                            col_style.background_color.or(computed.background_color),
                            span,
                        );
                        append_column_collapsed(
                            &mut collapsed_columns,
                            group_collapsed
                                || matches!(col_style.visibility, VisibilityMode::Collapse),
                            span,
                        );
                        group_offset = group_offset.saturating_add(span);
                    }
                }
                ancestors.pop();
            }
        }

        (
            hints,
            borders,
            group_borders,
            backgrounds,
            collapsed_columns,
        )
    }

    let include_prev_siblings = resolver.has_sibling_selectors();
    let mut rows: Vec<TableRowInput> = Vec::new();
    collect_rows(
        node,
        style,
        resolver,
        ancestors,
        include_prev_siblings,
        &mut rows,
    );
    if let Some(logger) = resolver.debug_logger() {
        let header_count = rows
            .iter()
            .filter(|row| row.section == TableRowSection::Header)
            .count();
        let body_count = rows.len().saturating_sub(header_count);
        let json = format!(
            "{{\"type\":\"table.rows\",\"total\":{},\"header\":{},\"body\":{}}}",
            rows.len(),
            header_count,
            body_count
        );
        logger.log_json(&json);
    }
    let (
        column_width_hints,
        column_borders,
        column_group_borders,
        column_backgrounds,
        collapsed_columns,
    ) = collect_column_metadata(node, style, resolver, ancestors, include_prev_siblings);
    let collapsed_table = matches!(
        style.border_collapse,
        crate::flowable::BorderCollapseMode::Collapse
    );

    // Table-local style caches. Tables dominate typical VDP docs, so reducing selector
    // evaluation here is a big win.
    let mut cached_header_tr_style: Option<ComputedStyle> = None;
    let mut cached_body_tr_style: Option<ComputedStyle> = None;
    let mut cached_header_th_empty: Option<ComputedStyle> = None;
    let mut cached_header_th_num: Option<ComputedStyle> = None;
    let mut cached_body_td_empty: Option<ComputedStyle> = None;
    let mut cached_body_td_num: Option<ComputedStyle> = None;

    enum StyleRef<'a> {
        Borrowed(&'a ComputedStyle),
        Owned(ComputedStyle),
    }
    impl<'a> StyleRef<'a> {
        fn as_ref(&self) -> &ComputedStyle {
            match self {
                StyleRef::Borrowed(s) => s,
                StyleRef::Owned(s) => s,
            }
        }
    }

    let header_count = rows
        .iter()
        .filter(|row| row.section == TableRowSection::Header)
        .count();
    let body_count = rows.len().saturating_sub(header_count);
    let mut prev_row_infos: Vec<ElementInfo> = Vec::new();
    let mut header_index = 0usize;
    let mut body_index = 0usize;
    let mut active_rowspans: Vec<Option<(TableCell, usize)>> = Vec::new();
    let table_node = node.clone();

    for row_input in rows {
        let row = row_input.node;
        let anonymous_cells = row_input.anonymous_cells;
        let anonymous_row = anonymous_cells.is_some();
        let is_header = row_input.section == TableRowSection::Header;
        let is_footer = row_input.section == TableRowSection::Footer;
        let in_explicit_row_group = row_input.in_explicit_row_group;
        let group_row_index = row_input.group_row_index;
        let group_row_count = row_input.group_row_count;
        let row_group_starts = group_row_index == 1;
        let row_group_ends = group_row_index == group_row_count;
        if row_group_starts {
            active_rowspans.clear();
            // Cached row/cell styles inherit from their section.  Reusing a
            // tbody-derived entry for a later tfoot (or a second tbody) leaks
            // section backgrounds, visibility and selectors across groups.
            cached_header_tr_style = None;
            cached_body_tr_style = None;
            cached_header_th_empty = None;
            cached_header_th_num = None;
            cached_body_td_empty = None;
            cached_body_td_num = None;
        }
        let mut next_rowspans: Vec<Option<(TableCell, usize)>> = active_rowspans
            .iter()
            .map(|entry| {
                entry.as_ref().and_then(|(cell, remaining)| {
                    remaining
                        .checked_sub(1)
                        .filter(|remaining| *remaining > 0)
                        .map(|remaining| (cell.clone(), remaining))
                })
            })
            .collect();
        row_count = row_count.saturating_add(1);
        let row_meta = row
            .as_element()
            .and_then(|el| {
                el.attributes
                    .borrow()
                    .get("data-fb")
                    .map(|s| parse_data_fb(s))
            })
            .unwrap_or_default();
        let section_context = row.parent().and_then(|parent| {
            let parent_el = parent.as_element()?;
            if parent == table_node {
                Some((
                    ElementInfo {
                        tag: "tbody".to_string(),
                        id: None,
                        classes: Vec::new(),
                        attrs: std::collections::HashMap::new(),
                        container_names: Vec::new(),
                        container_type: crate::style::ContainerQueryType::Normal,
                        container_width: None,
                        container_height: None,
                        container_inline_size: None,
                        container_block_size: None,
                        is_root: false,
                        is_empty: true,
                        is_defined: true,
                        language: None,
                        direction: None,
                        child_index: 1,
                        child_count: 1,
                        type_index: 1,
                        type_count: 1,
                        prev_siblings: Vec::new(),
                        next_siblings: Vec::new(),
                        children: Vec::new(),
                    },
                    None,
                ))
            } else {
                let info = element_info(&parent, include_prev_siblings);
                let inline_style = parent_el
                    .attributes
                    .borrow()
                    .get("style")
                    .map(|s| s.to_string());
                Some((info, inline_style))
            }
        });
        let mut pushed_section = false;
        let mut can_cache_section = true;
        let section_style_owned: Option<ComputedStyle> =
            if let Some((mut section, inline_style)) = section_context {
                can_cache_section =
                    section.id.is_none() && section.classes.is_empty() && inline_style.is_none();
                let t_section_style = std::time::Instant::now();
                let computed =
                    resolver.compute_style(&section, style, inline_style.as_deref(), ancestors);
                section.apply_computed_container_style(&computed);
                row_style_ms += t_section_style.elapsed().as_secs_f64() * 1000.0;
                ancestors.push(section);
                pushed_section = true;
                Some(computed)
            } else {
                None
            };
        let row_parent_style: &ComputedStyle = section_style_owned.as_ref().unwrap_or(style);
        if is_header {
            header_index += 1;
        } else {
            body_index += 1;
        }
        let row_child_index = if is_header { header_index } else { body_index };
        let row_child_count = if is_header {
            header_count.max(1)
        } else {
            body_count.max(1)
        };
        let mut row_info = if anonymous_row {
            None
        } else {
            row.as_element().map(|_| {
                let t_info = std::time::Instant::now();
                let base_info =
                    element_info_basic(&row, row_child_index, row_child_count, false, Vec::new());
                let info = if include_prev_siblings {
                    let mut with_prev = base_info.clone();
                    with_prev.prev_siblings = prev_row_infos.clone();
                    prev_row_infos.push(base_info);
                    with_prev
                } else {
                    base_info
                };
                row_info_ms += t_info.elapsed().as_secs_f64() * 1000.0;
                row_info_calls = row_info_calls.saturating_add(1);
                info
            })
        }
        .unwrap_or(ElementInfo {
            tag: "tr".to_string(),
            id: None,
            classes: Vec::new(),
            attrs: std::collections::HashMap::new(),
            container_names: Vec::new(),
            container_type: crate::style::ContainerQueryType::Normal,
            container_width: None,
            container_height: None,
            container_inline_size: None,
            container_block_size: None,
            is_root: false,
            is_empty: true,
            is_defined: true,
            language: None,
            direction: None,
            child_index: row_child_index,
            child_count: row_child_count,
            type_index: row_child_index,
            type_count: row_child_count,
            prev_siblings: Vec::new(),
            next_siblings: Vec::new(),
            children: Vec::new(),
        });
        if let Some(logger) = resolver.debug_logger() {
            let kind = if is_header { "header" } else { "body" };
            let json = format!(
                "{{\"type\":\"table.row\",\"kind\":\"{}\",\"row_index\":{},\"child_index\":{},\"child_count\":{}}}",
                kind,
                if is_header { header_index } else { body_index },
                row_info.child_index,
                row_info.child_count
            );
            logger.log_json(&json);
        }

        // Compute row style once so `td` can inherit from `tr` (more correct than inheriting from `table`).
        let row_inline_style = if anonymous_row {
            None
        } else {
            row.as_element()
                .and_then(|el| el.attributes.borrow().get("style").map(|s| s.to_string()))
        };
        let can_cache_row = !resolver.has_positional_selectors()
            && row_info.id.is_none()
            && row_info.classes.is_empty()
            && row_inline_style.is_none()
            && can_cache_section;
        let row_style_tmp = if can_cache_row {
            None
        } else {
            let t_row_style = std::time::Instant::now();
            let computed = resolver.compute_style(
                &row_info,
                row_parent_style,
                row_inline_style.as_deref(),
                ancestors,
            );
            row_style_ms += t_row_style.elapsed().as_secs_f64() * 1000.0;
            row_style_cache_miss = row_style_cache_miss.saturating_add(1);
            Some(computed)
        };
        let row_style: &ComputedStyle = if can_cache_row {
            let slot = if is_header {
                &mut cached_header_tr_style
            } else {
                &mut cached_body_tr_style
            };
            if slot.is_some() {
                row_style_cache_hit = row_style_cache_hit.saturating_add(1);
            } else {
                row_style_cache_miss = row_style_cache_miss.saturating_add(1);
            }
            let t_row_style = std::time::Instant::now();
            let computed = slot.get_or_insert_with(|| {
                resolver.compute_style(&row_info, row_parent_style, None, ancestors)
            });
            row_style_ms += t_row_style.elapsed().as_secs_f64() * 1000.0;
            computed
        } else {
            row_style_tmp.as_ref().unwrap()
        };
        let row_min_height = resolve_non_auto_height(
            row_style.height,
            row_style.font_size,
            row_style.root_font_size,
        );
        let row_collapsed = (in_explicit_row_group
            && matches!(row_parent_style.visibility, VisibilityMode::Collapse))
            || matches!(row_style.visibility, VisibilityMode::Collapse);
        let row_border_colors = row_style.resolved_border_colors(row_style.color);
        let row_border_styles = row_style.resolved_border_styles();
        let row_hidden_borders = row_style.border_hidden_sides();
        let row_group_border_colors =
            row_parent_style.resolved_border_colors(row_parent_style.color);
        let row_group_border_styles = row_parent_style.resolved_border_styles();
        let row_group_hidden_borders = row_parent_style.border_hidden_sides();

        row_info.apply_computed_container_style(row_style);
        ancestors.push(row_info);

        let mut cells: Vec<TableCell> = Vec::new();
        let cell_nodes: Vec<NodeRef> = if let Some(cells) = anonymous_cells {
            cells
        } else {
            row.children()
                .filter(|child| child.as_element().is_some())
                .collect()
        };
        let cell_total = cell_nodes.len().max(1);
        let mut prev_cell_infos: Vec<ElementInfo> = Vec::new();
        let mut logical_col = 0usize;
        for (cell_idx, cell_child) in cell_nodes.iter().enumerate() {
            let cell_el = cell_child.as_element().expect("cell element");
            let tag = cell_el.name.local.as_ref();
            cell_count = cell_count.saturating_add(1);
            let col_span = cell_el
                .attributes
                .borrow()
                .get("colspan")
                .and_then(|raw| raw.trim().parse::<usize>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(1);
            while let Some(Some((spanning_cell, _))) = active_rowspans.get(logical_col) {
                cells.push(
                    spanning_cell
                        .as_rowspan_placeholder()
                        .with_row_collapsed(row_collapsed)
                        .with_row_border(
                            row_style.border_width,
                            crate::flowable::ResolvedEdgeColors {
                                top: row_border_colors.top,
                                right: row_border_colors.right,
                                bottom: row_border_colors.bottom,
                                left: row_border_colors.left,
                            },
                            crate::flowable::ResolvedEdgeStyles {
                                top: row_border_styles.top,
                                right: row_border_styles.right,
                                bottom: row_border_styles.bottom,
                                left: row_border_styles.left,
                            },
                            crate::flowable::ResolvedEdgeHidden {
                                top: row_hidden_borders.top,
                                right: row_hidden_borders.right,
                                bottom: row_hidden_borders.bottom,
                                left: row_hidden_borders.left,
                            },
                        )
                        .with_row_group_border(
                            row_parent_style.border_width,
                            crate::flowable::ResolvedEdgeColors {
                                top: row_group_border_colors.top,
                                right: row_group_border_colors.right,
                                bottom: row_group_border_colors.bottom,
                                left: row_group_border_colors.left,
                            },
                            crate::flowable::ResolvedEdgeStyles {
                                top: row_group_border_styles.top,
                                right: row_group_border_styles.right,
                                bottom: row_group_border_styles.bottom,
                                left: row_group_border_styles.left,
                            },
                            crate::flowable::ResolvedEdgeHidden {
                                top: row_group_hidden_borders.top,
                                right: row_group_hidden_borders.right,
                                bottom: row_group_hidden_borders.bottom,
                                left: row_group_hidden_borders.left,
                            },
                            row_group_starts,
                            row_group_ends,
                        ),
                );
                logical_col = logical_col.saturating_add(1);
            }
            let remaining_group_rows = row_input
                .group_row_count
                .saturating_sub(row_input.group_row_index)
                .saturating_add(1)
                .max(1);
            let row_span = match cell_el
                .attributes
                .borrow()
                .get("rowspan")
                .and_then(|raw| raw.trim().parse::<usize>().ok())
            {
                Some(0) => remaining_group_rows,
                Some(value) => value.max(1).min(remaining_group_rows),
                None => 1,
            };

            let mut cell_info = {
                let t_info = std::time::Instant::now();
                let base_info =
                    element_info_basic(cell_child, cell_idx + 1, cell_total, false, Vec::new());
                let info = if include_prev_siblings {
                    let mut with_prev = base_info.clone();
                    with_prev.prev_siblings = prev_cell_infos.clone();
                    prev_cell_infos.push(base_info);
                    with_prev
                } else {
                    base_info
                };
                cell_info_ms += t_info.elapsed().as_secs_f64() * 1000.0;
                cell_info_calls = cell_info_calls.saturating_add(1);
                info
            };
            let inline_style = cell_el
                .attributes
                .borrow()
                .get("style")
                .map(|s| s.to_string());
            let can_cache_cell = can_cache_row
                && cell_info.id.is_none()
                && inline_style.is_none()
                && (cell_info.classes.is_empty()
                    || (cell_info.classes.len() == 1 && cell_info.classes[0] == "num"));

            let cell_style_ref = if can_cache_cell {
                let is_num = cell_info.classes.len() == 1;
                let slot: Option<&mut Option<ComputedStyle>> = match (is_header, tag, is_num) {
                    (true, "th", false) => Some(&mut cached_header_th_empty),
                    (true, "th", true) => Some(&mut cached_header_th_num),
                    (false, "td", false) => Some(&mut cached_body_td_empty),
                    (false, "td", true) => Some(&mut cached_body_td_num),
                    // Fallback for uncommon mixes (e.g. td in thead): don't cache.
                    _ => None,
                };

                if let Some(slot) = slot {
                    if slot.is_some() {
                        cell_style_cache_hit = cell_style_cache_hit.saturating_add(1);
                    } else {
                        cell_style_cache_miss = cell_style_cache_miss.saturating_add(1);
                    }
                    let t_cell_style = std::time::Instant::now();
                    let st = slot.get_or_insert_with(|| {
                        resolver.compute_style(&cell_info, row_style, None, ancestors)
                    });
                    cell_style_ms += t_cell_style.elapsed().as_secs_f64() * 1000.0;
                    StyleRef::Borrowed(st)
                } else {
                    let t_cell_style = std::time::Instant::now();
                    let computed = resolver.compute_style(&cell_info, row_style, None, ancestors);
                    cell_style_ms += t_cell_style.elapsed().as_secs_f64() * 1000.0;
                    cell_style_cache_miss = cell_style_cache_miss.saturating_add(1);
                    StyleRef::Owned(computed)
                }
            } else {
                let t_cell_style = std::time::Instant::now();
                let computed = resolver.compute_style(
                    &cell_info,
                    row_style,
                    inline_style.as_deref(),
                    ancestors,
                );
                cell_style_ms += t_cell_style.elapsed().as_secs_f64() * 1000.0;
                cell_style_cache_miss = cell_style_cache_miss.saturating_add(1);
                StyleRef::Owned(computed)
            };
            let cell_style = cell_style_ref.as_ref();
            if matches!(cell_style.display, DisplayMode::None) {
                continue;
            }
            cell_info.apply_computed_container_style(cell_style);

            let has_element_children = cell_child
                .children()
                .any(|child| child.as_element().is_some());
            let mut cell_content: Option<Box<dyn Flowable>> = None;
            let mut inline_content_phase = false;
            let mut cell_text = String::new();
            if has_element_children {
                let coerce_mixed_inline =
                    inline_or_replaced_children_only(cell_child, resolver, cell_style, ancestors);
                inline_content_phase = coerce_mixed_inline;
                let before_items = pseudo_items_for(
                    resolver,
                    &cell_info,
                    cell_style,
                    ancestors,
                    counters,
                    font_registry.clone(),
                    asset_bundle.as_deref(),
                    report.as_deref_mut(),
                    svg_form,
                    svg_raster_fallback,
                    crate::style::PseudoTarget::Before,
                );
                let after_items = pseudo_items_for(
                    resolver,
                    &cell_info,
                    cell_style,
                    ancestors,
                    counters,
                    font_registry.clone(),
                    asset_bundle.as_deref(),
                    report.as_deref_mut(),
                    svg_form,
                    svg_raster_fallback,
                    crate::style::PseudoTarget::After,
                );

                ancestors.push(cell_info.clone());
                let mut cell_items = before_items;
                cell_items.extend(collect_children(
                    cell_child,
                    resolver,
                    cell_style,
                    ancestors,
                    counters,
                    font_registry.clone(),
                    asset_bundle.clone(),
                    report.as_deref_mut(),
                    svg_form,
                    svg_raster_fallback,
                    perf,
                    doc_id,
                ));
                ancestors.pop();
                cell_items.extend(after_items);

                let cell_items = if coerce_mixed_inline {
                    // `vertical-align` on a table cell controls the cell's
                    // contents as a group; it is not inherited by anonymous
                    // inline children. Those runs participate in their own
                    // line box on the baseline, while TableFlowable applies
                    // the cell-level alignment to the completed content box.
                    coerce_items_to_inline_run(
                        cell_items,
                        VerticalAlign::Baseline,
                        cell_style,
                        font_registry.clone(),
                        false,
                    )
                } else {
                    cell_items
                };

                let mut cell_flowables = layout_children_to_flowables(cell_items, None);
                cell_content = if cell_flowables.is_empty() {
                    None
                } else if cell_flowables.len() == 1 {
                    Some(cell_flowables.remove(0))
                } else {
                    Some(Box::new(
                        ContainerFlowable::new_pt(
                            cell_flowables,
                            cell_style.font_size,
                            cell_style.root_font_size,
                        )
                        .with_self_visible(cell_style.visibility.paints()),
                    ) as Box<dyn Flowable>)
                };
            }

            if cell_content.is_none() {
                let t_cell_text = std::time::Instant::now();
                let text = cell_child.text_contents();
                cell_text_ms += t_cell_text.elapsed().as_secs_f64() * 1000.0;
                // HTML collapsible whitespace does not include NBSP.  Keeping
                // it here is also what makes `empty-cells: hide` treat a cell
                // containing `&nbsp;` as non-empty.
                let trimmed =
                    text.trim_matches(|ch| matches!(ch, ' ' | '\n' | '\r' | '\t' | '\u{000c}'));
                if !trimmed.is_empty() {
                    let transformed = apply_text_transform(trimmed, cell_style.text_transform);
                    text_chars = text_chars.saturating_add(transformed.chars().count() as u64);
                    cell_text = transformed;
                }
            }

            let align = text_align_from_style(&cell_style);
            let valign = match cell_style.vertical_align {
                VerticalAlignMode::TextTop
                | VerticalAlignMode::TextBottom
                | VerticalAlignMode::Sub
                | VerticalAlignMode::Super => VerticalAlign::Baseline,
                _ => vertical_align_from_style(&cell_style),
            };
            let cell_min_height = resolve_non_auto_height(
                cell_style.height,
                cell_style.font_size,
                cell_style.root_font_size,
            );

            let mut cell_padding = cell_style.padding;
            if let Some(padding) = legacy_cell_padding {
                let ua_padding = LengthSpec::Absolute(Pt::from_f32(0.75));
                let is_zero = length_spec_is_zero(cell_padding.top)
                    && length_spec_is_zero(cell_padding.right)
                    && length_spec_is_zero(cell_padding.bottom)
                    && length_spec_is_zero(cell_padding.left);
                let is_ua_default = cell_padding
                    == EdgeSizes {
                        top: ua_padding,
                        right: ua_padding,
                        bottom: ua_padding,
                        left: ua_padding,
                    };
                if is_zero || is_ua_default {
                    let padding = LengthSpec::Absolute(padding);
                    cell_padding = EdgeSizes {
                        top: padding,
                        right: padding,
                        bottom: padding,
                        left: padding,
                    };
                }
            }

            let mut border_widths = cell_style.border_width;
            if !collapsed_table {
                if length_spec_is_zero(border_widths.top)
                    && !length_spec_is_zero(row_style.border_width.top)
                {
                    border_widths.top = row_style.border_width.top;
                }
                if length_spec_is_zero(border_widths.bottom)
                    && !length_spec_is_zero(row_style.border_width.bottom)
                {
                    border_widths.bottom = row_style.border_width.bottom;
                }
            }
            let mut border_styles = cell_style.resolved_border_styles();
            let mut hidden_borders = cell_style.border_hidden_sides();
            let base_border_color = if collapsed_table {
                cell_style.border_color.unwrap_or(cell_style.color)
            } else {
                cell_style
                    .border_color
                    .or(row_style.border_color)
                    .unwrap_or(cell_style.color)
            };
            let resolved_cell_border_colors = cell_style.resolved_border_colors(base_border_color);
            let resolved_cell_border_opacities = cell_style.resolved_border_opacities();
            let mut border_color_top = resolved_cell_border_colors.top;
            let mut border_color_right = resolved_cell_border_colors.right;
            let mut border_color_bottom = resolved_cell_border_colors.bottom;
            let mut border_color_left = resolved_cell_border_colors.left;
            let mut border_opacity_top = resolved_cell_border_opacities.top;
            let mut border_opacity_right = resolved_cell_border_opacities.right;
            let mut border_opacity_bottom = resolved_cell_border_opacities.bottom;
            let mut border_opacity_left = resolved_cell_border_opacities.left;
            if legacy_border_width.is_some()
                && length_spec_is_zero(border_widths.top)
                && length_spec_is_zero(border_widths.right)
                && length_spec_is_zero(border_widths.bottom)
                && length_spec_is_zero(border_widths.left)
            {
                let one_css_px = LengthSpec::Absolute(Pt::from_f32(0.75));
                border_widths = EdgeSizes {
                    top: one_css_px,
                    right: one_css_px,
                    bottom: one_css_px,
                    left: one_css_px,
                };
                border_styles.top = OutlineLineStyle::Solid;
                border_styles.right = OutlineLineStyle::Solid;
                border_styles.bottom = OutlineLineStyle::Solid;
                border_styles.left = OutlineLineStyle::Solid;
                hidden_borders.top = false;
                hidden_borders.right = false;
                hidden_borders.bottom = false;
                hidden_borders.left = false;
                let light = Color::rgb(238.0 / 255.0, 238.0 / 255.0, 238.0 / 255.0);
                let dark = Color::rgb(154.0 / 255.0, 154.0 / 255.0, 154.0 / 255.0);
                border_color_top = dark;
                border_color_left = dark;
                border_color_right = light;
                border_color_bottom = light;
                border_opacity_top = 1.0;
                border_opacity_right = 1.0;
                border_opacity_bottom = 1.0;
                border_opacity_left = 1.0;
            }
            let border = BorderSpec {
                widths: border_widths,
                color: border_color_top,
            };
            let effective_cell_background = cell_style
                .background_color
                .or(row_style.background_color)
                .or(row_parent_style.background_color)
                .or_else(|| column_backgrounds.get(logical_col).copied().flatten());

            let text_style = text_style_for_flow_text(&cell_style);
            if !cell_text.is_empty() {
                let t_report = std::time::Instant::now();
                report_missing_glyphs(
                    report.as_deref_mut(),
                    font_registry.as_deref(),
                    &text_style,
                    &cell_text,
                );
                cell_report_ms += t_report.elapsed().as_secs_f64() * 1000.0;
                cell_report_calls = cell_report_calls.saturating_add(1);
            }
            let cell = TableCell::new(
                cell_text,
                text_style,
                align,
                valign,
                cell_padding,
                effective_cell_background,
                border,
                cell_style.box_shadow.clone(),
                Some(Arc::<str>::from(if tag == "th" { "TH" } else { "TD" })),
                if tag == "th" {
                    html_th_scope_to_pdf_scope(cell_el.attributes.borrow().get("scope"))
                        .or_else(|| Some("Column".to_string()))
                } else {
                    None
                },
                col_span,
                cell_style.root_font_size,
                font_registry.clone(),
                preserve_whitespace(cell_style.white_space),
                no_wrap(&cell_style),
            );
            let mut cell = cell
                .with_border_styles(
                    border_styles.top,
                    border_styles.right,
                    border_styles.bottom,
                    border_styles.left,
                )
                .with_border_colors(
                    border_color_top,
                    border_color_right,
                    border_color_bottom,
                    border_color_left,
                )
                .with_border_opacities(
                    border_opacity_top,
                    border_opacity_right,
                    border_opacity_bottom,
                    border_opacity_left,
                )
                .with_hidden_borders(
                    hidden_borders.top,
                    hidden_borders.right,
                    hidden_borders.bottom,
                    hidden_borders.left,
                )
                .with_self_visible(cell_style.visibility.paints())
                .with_row_collapsed(row_collapsed)
                .with_row_border(
                    row_style.border_width,
                    crate::flowable::ResolvedEdgeColors {
                        top: row_border_colors.top,
                        right: row_border_colors.right,
                        bottom: row_border_colors.bottom,
                        left: row_border_colors.left,
                    },
                    crate::flowable::ResolvedEdgeStyles {
                        top: row_border_styles.top,
                        right: row_border_styles.right,
                        bottom: row_border_styles.bottom,
                        left: row_border_styles.left,
                    },
                    crate::flowable::ResolvedEdgeHidden {
                        top: row_hidden_borders.top,
                        right: row_hidden_borders.right,
                        bottom: row_hidden_borders.bottom,
                        left: row_hidden_borders.left,
                    },
                )
                .with_row_group_border(
                    row_parent_style.border_width,
                    crate::flowable::ResolvedEdgeColors {
                        top: row_group_border_colors.top,
                        right: row_group_border_colors.right,
                        bottom: row_group_border_colors.bottom,
                        left: row_group_border_colors.left,
                    },
                    crate::flowable::ResolvedEdgeStyles {
                        top: row_group_border_styles.top,
                        right: row_group_border_styles.right,
                        bottom: row_group_border_styles.bottom,
                        left: row_group_border_styles.left,
                    },
                    crate::flowable::ResolvedEdgeHidden {
                        top: row_group_hidden_borders.top,
                        right: row_group_hidden_borders.right,
                        bottom: row_group_hidden_borders.bottom,
                        left: row_group_hidden_borders.left,
                    },
                    row_group_starts,
                    row_group_ends,
                )
                .with_row_min_height(row_min_height.max(cell_min_height))
                .with_row_span(row_span)
                .with_hide_empty_cells(cell_style.empty_cells_hide)
                .with_establishes_abs_containing_block(establishes_abs_containing_block(cell_style))
                .with_overflow_hidden(matches!(
                    cell_style.overflow,
                    OverflowMode::Hidden | OverflowMode::Clip
                ));
            if !matches!(
                cell_style.width,
                LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
            ) {
                cell = cell.with_preferred_width(
                    cell_style.width,
                    cell_style.font_size,
                    cell_style.root_font_size,
                );
            }
            let cell = if let Some(content) = cell_content {
                cell.with_content(content)
                    .with_inline_content_phase(inline_content_phase)
            } else {
                cell
            };
            if row_span > 1 {
                let required_columns = logical_col.saturating_add(col_span);
                if next_rowspans.len() < required_columns {
                    next_rowspans.resize(required_columns, None);
                }
                for slot in &mut next_rowspans[logical_col..required_columns] {
                    *slot = Some((cell.clone(), row_span - 1));
                }
            }
            cells.push(cell);
            logical_col = logical_col.saturating_add(col_span);
        }

        while logical_col < active_rowspans.len() {
            if let Some((spanning_cell, _)) = active_rowspans[logical_col].as_ref() {
                cells.push(
                    spanning_cell
                        .as_rowspan_placeholder()
                        .with_row_collapsed(row_collapsed)
                        .with_row_border(
                            row_style.border_width,
                            crate::flowable::ResolvedEdgeColors {
                                top: row_border_colors.top,
                                right: row_border_colors.right,
                                bottom: row_border_colors.bottom,
                                left: row_border_colors.left,
                            },
                            crate::flowable::ResolvedEdgeStyles {
                                top: row_border_styles.top,
                                right: row_border_styles.right,
                                bottom: row_border_styles.bottom,
                                left: row_border_styles.left,
                            },
                            crate::flowable::ResolvedEdgeHidden {
                                top: row_hidden_borders.top,
                                right: row_hidden_borders.right,
                                bottom: row_hidden_borders.bottom,
                                left: row_hidden_borders.left,
                            },
                        )
                        .with_row_group_border(
                            row_parent_style.border_width,
                            crate::flowable::ResolvedEdgeColors {
                                top: row_group_border_colors.top,
                                right: row_group_border_colors.right,
                                bottom: row_group_border_colors.bottom,
                                left: row_group_border_colors.left,
                            },
                            crate::flowable::ResolvedEdgeStyles {
                                top: row_group_border_styles.top,
                                right: row_group_border_styles.right,
                                bottom: row_group_border_styles.bottom,
                                left: row_group_border_styles.left,
                            },
                            crate::flowable::ResolvedEdgeHidden {
                                top: row_group_hidden_borders.top,
                                right: row_group_hidden_borders.right,
                                bottom: row_group_hidden_borders.bottom,
                                left: row_group_hidden_borders.left,
                            },
                            row_group_starts,
                            row_group_ends,
                        ),
                );
            }
            logical_col = logical_col.saturating_add(1);
        }
        active_rowspans = next_rowspans;

        ancestors.pop();

        if cells.is_empty() {
            if pushed_section {
                ancestors.pop();
            }
            continue;
        }
        if is_header {
            header_rows.push(cells);
        } else {
            let body_index = body_rows.len();
            let keep_range = if !is_footer
                && in_explicit_row_group
                && matches!(
                    row_parent_style.pagination.break_inside,
                    BreakInside::Avoid | BreakInside::AvoidPage
                ) {
                let start = body_index.saturating_sub(group_row_index.saturating_sub(1));
                Some((start, start.saturating_add(group_row_count)))
            } else {
                None
            };
            body_rows.push(cells);
            body_row_meta.push(row_meta);
            body_row_pagination.push(row_style.pagination);
            body_row_keep_ranges.push(keep_range);
            if is_footer {
                footer_row_count += 1;
            }
        }

        if pushed_section {
            ancestors.pop();
        }
    }

    if body_rows.is_empty() && !header_rows.is_empty() {
        body_rows = header_rows.clone();
        header_rows.clear();
        body_row_meta = vec![Vec::new(); body_rows.len()];
        body_row_pagination = vec![crate::flowable::Pagination::default(); body_rows.len()];
        body_row_keep_ranges = vec![None; body_rows.len()];
        footer_row_count = 0;
    }

    if let Some(perf_logger) = perf {
        let ms = t_table.elapsed().as_secs_f64() * 1000.0;
        perf_logger.log_span_ms("story.table", doc_id, ms);
        perf_logger.log_span_ms("story.table.row_style", doc_id, row_style_ms);
        perf_logger.log_span_ms("story.table.cell_style", doc_id, cell_style_ms);
        perf_logger.log_span_ms("story.table.cell_text", doc_id, cell_text_ms);
        perf_logger.log_span_ms("story.table.glyph_report", doc_id, cell_report_ms);
        perf_logger.log_span_ms("story.table.row_info", doc_id, row_info_ms);
        perf_logger.log_span_ms("story.table.cell_info", doc_id, cell_info_ms);
        perf_logger.log_counts(
            "story.table",
            doc_id,
            &[
                ("rows", row_count),
                ("cells", cell_count),
                ("text_chars", text_chars),
                ("row_style_cache_hit", row_style_cache_hit),
                ("row_style_cache_miss", row_style_cache_miss),
                ("cell_style_cache_hit", cell_style_cache_hit),
                ("cell_style_cache_miss", cell_style_cache_miss),
                ("glyph_report_calls", cell_report_calls),
                ("row_info_calls", row_info_calls),
                ("cell_info_calls", cell_info_calls),
            ],
        );
    }

    TableFlowable::new(body_rows)
        .with_header(header_rows)
        .repeat_header(true)
        .with_footer_row_count(footer_row_count)
        .repeat_footer(footer_row_count > 0)
        .with_row_backgrounds(false)
        .with_body_row_meta(body_row_meta)
        .with_body_row_pagination(body_row_pagination)
        .with_body_row_keep_ranges(body_row_keep_ranges)
        .with_column_width_hints(column_width_hints)
        .with_column_borders(column_borders)
        .with_column_group_borders(column_group_borders)
        .with_collapsed_columns(collapsed_columns)
        .with_pagination(style.pagination)
}

fn legacy_table_length_attribute(node: &NodeRef, name: &str) -> Option<Pt> {
    let raw = node
        .as_element()?
        .attributes
        .borrow()
        .get(name)?
        .trim()
        .parse::<f32>()
        .ok()?;
    if !raw.is_finite() || raw < 0.0 {
        return None;
    }
    Some(Pt::from_f32(raw * 0.75))
}

fn report_missing_glyphs(
    report: Option<&mut GlyphCoverageReport>,
    registry: Option<&FontRegistry>,
    text_style: &TextStyle,
    text: &str,
) {
    if let (Some(report), Some(registry)) = (report, registry) {
        registry.report_missing_glyphs(
            &text_style.font_name,
            &text_style.font_fallbacks,
            text,
            report,
        );
        if matches!(
            text_style.text_overflow,
            crate::style::TextOverflowMode::Ellipsis
        ) {
            registry.report_missing_glyphs(
                &text_style.font_name,
                &text_style.font_fallbacks,
                "\u{2026}",
                report,
            );
        }
    }
}

fn element_info_basic(
    node: &NodeRef,
    child_index: usize,
    child_count: usize,
    is_root: bool,
    prev_siblings: Vec<ElementInfo>,
) -> ElementInfo {
    let element = node.as_element().expect("element node");
    let tag = element.name.local.as_ref().to_ascii_lowercase();
    let (type_index, type_count) = node
        .parent()
        .map(|parent| {
            let mut count = 0usize;
            let mut index = 0usize;
            for sibling in parent.children() {
                let Some(sibling_element) = sibling.as_element() else {
                    continue;
                };
                if sibling_element
                    .name
                    .local
                    .as_ref()
                    .eq_ignore_ascii_case(&tag)
                {
                    count += 1;
                    if sibling == *node {
                        index = count;
                    }
                }
            }
            (index.max(1), count.max(1))
        })
        .unwrap_or((1, 1));
    let attrs = element.attributes.borrow();
    let id = attrs.get("id").map(|s| s.to_ascii_lowercase());
    let classes = attrs
        .get("class")
        .map(|class| {
            class
                .split_whitespace()
                .map(|c| c.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut attr_map = std::collections::HashMap::new();
    for (name, attr) in attrs.map.iter() {
        if name.ns.is_empty() {
            let key = name.local.to_string().to_ascii_lowercase();
            attr_map.insert(key, attr.value.clone());
        }
    }
    let inherited_attribute = |name: &str| {
        node.ancestors().find_map(|ancestor| {
            let element = ancestor.as_element()?;
            element.attributes.borrow().get(name).map(str::to_string)
        })
    };
    let is_empty = !node.children().any(|child| match child.data() {
        NodeData::Element(_) => true,
        NodeData::Text(text) => !text.borrow().is_empty(),
        _ => false,
    });

    ElementInfo {
        tag,
        id,
        classes,
        attrs: attr_map,
        container_names: Vec::new(),
        container_type: crate::style::ContainerQueryType::Normal,
        container_width: None,
        container_height: None,
        container_inline_size: None,
        container_block_size: None,
        is_root,
        is_empty,
        is_defined: !element.name.local.as_ref().contains('-'),
        language: inherited_attribute("lang"),
        direction: inherited_attribute("dir").map(|value| value.to_ascii_lowercase()),
        child_index,
        child_count,
        type_index,
        type_count,
        prev_siblings,
        next_siblings: Vec::new(),
        children: Vec::new(),
    }
}

fn element_info_selector_tree(node: &NodeRef) -> ElementInfo {
    let mut info = element_info(node, false);
    info.children = node
        .children()
        .filter(|child| child.as_element().is_some())
        .map(|child| element_info_selector_tree(&child))
        .collect();
    info
}

fn element_info_with_context(
    node: &NodeRef,
    child_index: usize,
    child_count: usize,
    is_root: bool,
    include_prev_siblings: bool,
) -> ElementInfo {
    let mut prev_siblings: Vec<ElementInfo> = Vec::new();
    let mut next_siblings: Vec<ElementInfo> = Vec::new();
    if include_prev_siblings {
        if let Some(parent) = node.parent() {
            let mut prior: Vec<NodeRef> = Vec::new();
            let mut following: Vec<NodeRef> = Vec::new();
            let mut seen = 0usize;
            for sibling in parent.children() {
                if sibling.as_element().is_none() {
                    continue;
                }
                seen += 1;
                if seen < child_index {
                    prior.push(sibling);
                } else if seen > child_index {
                    following.push(sibling);
                }
            }
            if !prior.is_empty() {
                prev_siblings = prior
                    .iter()
                    .enumerate()
                    .map(|(idx, sibling)| {
                        element_info_basic(sibling, idx + 1, child_count, false, Vec::new())
                    })
                    .collect();
            }
            if !following.is_empty() {
                next_siblings = following.iter().map(element_info_selector_tree).collect();
            }
        }
    }

    let mut info = element_info_basic(node, child_index, child_count, is_root, prev_siblings);
    if include_prev_siblings {
        info.next_siblings = next_siblings;
        info.children = node
            .children()
            .filter(|child| child.as_element().is_some())
            .map(|child| element_info_selector_tree(&child))
            .collect();
    }
    info
}

fn element_info(node: &NodeRef, include_prev_siblings: bool) -> ElementInfo {
    let mut child_index = 1usize;
    let mut child_count = 1usize;

    if let Some(parent) = node.parent() {
        let mut count = 0usize;
        let mut seen = 0usize;
        for sibling in parent.children() {
            if sibling.as_element().is_none() {
                continue;
            }
            count += 1;
            if sibling == *node {
                seen = count;
            }
        }
        if count > 0 {
            child_count = count;
        }
        if seen > 0 {
            child_index = seen;
        }
    }

    let is_html = node
        .as_element()
        .map(|element| element.name.local.as_ref().eq_ignore_ascii_case("html"))
        .unwrap_or(false);
    let is_root = is_html
        && node
            .ancestors()
            .skip(1)
            .all(|ancestor| ancestor.as_element().is_none());

    element_info_with_context(
        node,
        child_index,
        child_count,
        is_root,
        include_prev_siblings,
    )
}

#[derive(Debug, Clone, Copy)]
struct ReplacedImageSizing {
    width: LengthSpec,
    height: LengthSpec,
    nominal_width: Pt,
    nominal_height: Pt,
    aspect_ratio: Option<f32>,
}

fn css_size_is_auto(spec: LengthSpec) -> bool {
    matches!(
        spec,
        LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
    )
}

/// Resolve a length that does not depend on the eventual containing block.
///
/// Replaced-element sizing runs while the HTML tree is compiled. Resolving
/// absolute and font-relative constraints here turns the final used image
/// size into immutable layout input that compiled documents can reuse. Any
/// percentage-bearing value stays deferred to the normal container layout.
fn resolve_compile_time_css_dimension(
    spec: LengthSpec,
    font_size: Pt,
    root_font_size: Pt,
    is_height: bool,
) -> Option<Pt> {
    let definite = match spec {
        LengthSpec::Absolute(_) | LengthSpec::Em(_) | LengthSpec::Rem(_) => true,
        LengthSpec::Calc(calc) => calc.percent == 0.0,
        LengthSpec::Clamped(clamped) => clamped.value.percent == 0.0,
        LengthSpec::FontRelative(relative) => relative.base.percent == 0.0,
        _ => false,
    };
    if !definite {
        return None;
    }
    let resolved = if is_height {
        spec.resolve_height(Pt::ZERO, font_size, root_font_size)
    } else {
        spec.resolve_width(Pt::ZERO, font_size, root_font_size)
    };
    Some(resolved.max(Pt::ZERO))
}

/// `Some(None)` is an unbounded automatic constraint, while `None` means the
/// constraint depends on layout and therefore cannot be compiled yet.
fn resolve_compile_time_replaced_constraint(
    spec: LengthSpec,
    font_size: Pt,
    root_font_size: Pt,
    is_height: bool,
) -> Option<Option<Pt>> {
    if css_size_is_auto(spec) {
        return Some(None);
    }
    resolve_compile_time_css_dimension(spec, font_size, root_font_size, is_height).map(Some)
}

fn scale_replaced_dimension(value: Pt, target: Pt, original: Pt) -> Pt {
    if value <= Pt::ZERO || target <= Pt::ZERO || original <= Pt::ZERO {
        return Pt::ZERO;
    }
    Pt::from_f32(value.to_f32() * target.to_f32() / original.to_f32())
}

/// Apply the CSS replaced-element min/max constraint table while retaining
/// the tentative aspect ratio whenever the constraints are compatible.
fn constrain_replaced_size(
    width: Pt,
    height: Pt,
    min_width: Option<Pt>,
    max_width: Option<Pt>,
    min_height: Option<Pt>,
    max_height: Option<Pt>,
) -> (Pt, Pt) {
    if width <= Pt::ZERO || height <= Pt::ZERO {
        return (width.max(Pt::ZERO), height.max(Pt::ZERO));
    }

    let min_width = min_width.unwrap_or(Pt::ZERO).max(Pt::ZERO);
    let min_height = min_height.unwrap_or(Pt::ZERO).max(Pt::ZERO);
    // CSS requires max(min, max) for this table so contradictory authored
    // constraints still have a deterministic result.
    let max_width = max_width.map(|value| value.max(min_width));
    let max_height = max_height.map(|value| value.max(min_height));
    let width_high = max_width.is_some_and(|value| width > value);
    let width_low = width < min_width;
    let height_high = max_height.is_some_and(|value| height > value);
    let height_low = height < min_height;

    if width_high && height_high {
        let max_width = max_width.expect("high width has a maximum");
        let max_height = max_height.expect("high height has a maximum");
        let width_scale = max_width.to_f32() / width.to_f32();
        let height_scale = max_height.to_f32() / height.to_f32();
        if width_scale <= height_scale {
            return (
                max_width,
                scale_replaced_dimension(height, max_width, width).max(min_height),
            );
        }
        return (
            scale_replaced_dimension(width, max_height, height).max(min_width),
            max_height,
        );
    }
    if width_low && height_low {
        let width_scale = min_width.to_f32() / width.to_f32();
        let height_scale = min_height.to_f32() / height.to_f32();
        if width_scale <= height_scale {
            let resolved_width = scale_replaced_dimension(width, min_height, height);
            return (
                max_width
                    .map(|maximum| resolved_width.min(maximum))
                    .unwrap_or(resolved_width),
                min_height,
            );
        }
        let resolved_height = scale_replaced_dimension(height, min_width, width);
        return (
            min_width,
            max_height
                .map(|maximum| resolved_height.min(maximum))
                .unwrap_or(resolved_height),
        );
    }
    if width_low && height_high {
        return (min_width, max_height.expect("high height has a maximum"));
    }
    if width_high && height_low {
        return (max_width.expect("high width has a maximum"), min_height);
    }
    if width_high {
        let max_width = max_width.expect("high width has a maximum");
        return (
            max_width,
            scale_replaced_dimension(height, max_width, width).max(min_height),
        );
    }
    if width_low {
        let resolved_height = scale_replaced_dimension(height, min_width, width);
        return (
            min_width,
            max_height
                .map(|maximum| resolved_height.min(maximum))
                .unwrap_or(resolved_height),
        );
    }
    if height_high {
        let max_height = max_height.expect("high height has a maximum");
        return (
            scale_replaced_dimension(width, max_height, height).max(min_width),
            max_height,
        );
    }
    if height_low {
        let resolved_width = scale_replaced_dimension(width, min_height, height);
        return (
            max_width
                .map(|maximum| resolved_width.min(maximum))
                .unwrap_or(resolved_width),
            min_height,
        );
    }
    (width, height)
}

fn resolve_replaced_image_sizing(
    style: &ComputedStyle,
    attr_width: Option<Pt>,
    attr_height: Option<Pt>,
    intrinsic_size: Option<(Pt, Pt)>,
) -> ReplacedImageSizing {
    // Width/height attributes are presentational hints. Any definite CSS size
    // wins; otherwise retain the hint as a definite size until layout.
    let mut width = if css_size_is_auto(style.width) {
        attr_width
            .map(LengthSpec::Absolute)
            .unwrap_or(LengthSpec::Auto)
    } else {
        style.width
    };
    let mut height = if css_size_is_auto(style.height) {
        attr_height
            .map(LengthSpec::Absolute)
            .unwrap_or(LengthSpec::Auto)
    } else {
        style.height
    };
    let intrinsic_ratio = intrinsic_size.and_then(|(width, height)| {
        let height = height.to_f32();
        (height.is_finite() && height > 0.0)
            .then_some(width.to_f32() / height)
            .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
    });
    let aspect_ratio = style.aspect_ratio.or(intrinsic_ratio).or(Some(4.0 / 3.0));

    let width_was_auto = css_size_is_auto(width);
    let height_was_auto = css_size_is_auto(height);

    // A replaced element with both axes auto uses its natural width rather
    // than the fill-available width of an ordinary block box. Keeping height
    // auto lets min/max-width recompute it through the intrinsic ratio.
    if css_size_is_auto(width) && css_size_is_auto(height) {
        width = LengthSpec::Absolute(
            intrinsic_size
                .map(|size| size.0)
                .unwrap_or(style.font_size * 4.0),
        );
    }

    let mut nominal_width = resolve_non_auto_css_dimension(
        width,
        Pt::from_f32(225.0),
        style.font_size,
        style.root_font_size,
        false,
    );
    let mut nominal_height = resolve_non_auto_css_dimension(
        height,
        Pt::from_f32(112.5),
        style.font_size,
        style.root_font_size,
        true,
    );
    if nominal_width.is_none() {
        if let (Some(height), Some(ratio)) = (nominal_height, aspect_ratio) {
            nominal_width = Some(Pt::from_f32(height.to_f32() * ratio));
        }
    }
    if nominal_height.is_none() {
        if let (Some(width), Some(ratio)) = (nominal_width, aspect_ratio) {
            nominal_height = Some(Pt::from_f32(width.to_f32() / ratio));
        }
    }
    if width_was_auto || height_was_auto {
        let compile_time_width =
            resolve_compile_time_css_dimension(width, style.font_size, style.root_font_size, false);
        let compile_time_height =
            resolve_compile_time_css_dimension(height, style.font_size, style.root_font_size, true);
        let tentative = match (compile_time_width, compile_time_height, aspect_ratio) {
            (Some(width), Some(height), _) => Some((width, height)),
            (Some(width), None, Some(ratio)) if height_was_auto => {
                Some((width, Pt::from_f32(width.to_f32() / ratio)))
            }
            (None, Some(height), Some(ratio)) if width_was_auto => {
                Some((Pt::from_f32(height.to_f32() * ratio), height))
            }
            _ => None,
        };
        let constraints = (
            resolve_compile_time_replaced_constraint(
                style.min_width,
                style.font_size,
                style.root_font_size,
                false,
            ),
            resolve_compile_time_replaced_constraint(
                style.max_width,
                style.font_size,
                style.root_font_size,
                false,
            ),
            resolve_compile_time_replaced_constraint(
                style.min_height,
                style.font_size,
                style.root_font_size,
                true,
            ),
            resolve_compile_time_replaced_constraint(
                style.max_height,
                style.font_size,
                style.root_font_size,
                true,
            ),
        );
        if let (
            Some((tentative_width, tentative_height)),
            (Some(min_width), Some(max_width), Some(min_height), Some(max_height)),
        ) = (tentative, constraints)
        {
            let (used_width, used_height) = constrain_replaced_size(
                tentative_width,
                tentative_height,
                min_width,
                max_width,
                min_height,
                max_height,
            );
            if used_width != tentative_width || used_height != tentative_height {
                width = LengthSpec::Absolute(used_width);
                height = LengthSpec::Absolute(used_height);
                nominal_width = Some(used_width);
                nominal_height = Some(used_height);
            }
        }
    }
    let nominal_width = nominal_width
        .or_else(|| intrinsic_size.map(|size| size.0))
        .unwrap_or(style.font_size * 4.0)
        .max(Pt::from_f32(1.0));
    let nominal_height = nominal_height
        .or_else(|| intrinsic_size.map(|size| size.1))
        .unwrap_or(style.font_size * 3.0)
        .max(Pt::from_f32(1.0));

    ReplacedImageSizing {
        width,
        height,
        nominal_width,
        nominal_height,
        aspect_ratio,
    }
}

fn is_direct_fixed_replaced_box(style: &ComputedStyle, sizing: ReplacedImageSizing) -> bool {
    matches!(sizing.width, LengthSpec::Absolute(_))
        && matches!(sizing.height, LengthSpec::Absolute(_))
        && matches!(style.min_width, LengthSpec::Auto)
        && matches!(style.max_width, LengthSpec::Auto)
        && matches!(style.min_height, LengthSpec::Auto)
        && matches!(style.max_height, LengthSpec::Auto)
        && style.margin == EdgeSizes::zero()
        && style.padding == EdgeSizes::zero()
        && style.border_width == EdgeSizes::zero()
        && style.border_image.source.is_none()
        && style.border_radius == BorderRadiiSpec::zero()
        && style.background_color.is_none()
        && style.background_source_color.is_none()
        && style.background_paint.is_none()
        && style.background_paints.is_empty()
        && style.clip_path.is_none()
        && style.legacy_clip.is_none()
        && style.box_shadow.is_none()
        && style.box_shadows.is_empty()
        && !style.outline_visible
        && style.transform.is_empty()
        && !style.isolation
        && (style.opacity - 1.0).abs() <= 1.0e-6
}

fn replaced_image_flowables(
    mut image: ImageFlowable,
    style: &ComputedStyle,
    sizing: ReplacedImageSizing,
) -> Vec<LayoutItem> {
    // Blink raster-paints replaced-element origins on CSS-pixel boundaries
    // while their inline advance remains in fixed-point layout space.
    image = image.with_css_pixel_paint_origin_snap(true);
    if is_direct_fixed_replaced_box(style, sizing) {
        image = image
            .with_available_size(false)
            .with_pagination(style.pagination)
            .with_mix_blend_mode(style.mix_blend_mode)
            .with_paint_filter(style.paint_filter.clone());
        return vec![LayoutItem::Block {
            flowable: Box::new(image) as Box<dyn Flowable>,
            flex_grow: style.flex_grow,
            flex_shrink: style.flex_shrink,
            width_spec: flex_item_basis(style),
            order: 0,
        }];
    }

    let mut replaced_style = style.clone();
    replaced_style.width = sizing.width;
    replaced_style.height = sizing.height;
    replaced_style.aspect_ratio = sizing.aspect_ratio;
    let image = LayoutItem::Block {
        flowable: Box::new(image) as Box<dyn Flowable>,
        flex_grow: 0.0,
        flex_shrink: 1.0,
        width_spec: None,
        order: 0,
    };
    container_flowables(vec![image], &replaced_style)
}

fn replaced_svg_image_flowables(
    mut svg: SvgFlowable,
    style: &ComputedStyle,
    sizing: ReplacedImageSizing,
) -> Vec<LayoutItem> {
    if is_direct_fixed_replaced_box(style, sizing) {
        svg = svg
            .with_available_size(false)
            .with_pagination(style.pagination)
            .with_mix_blend_mode(style.mix_blend_mode);
        return vec![LayoutItem::Block {
            flowable: Box::new(svg) as Box<dyn Flowable>,
            flex_grow: style.flex_grow,
            flex_shrink: style.flex_shrink,
            width_spec: flex_item_basis(style),
            order: 0,
        }];
    }

    let mut replaced_style = style.clone();
    replaced_style.width = sizing.width;
    replaced_style.height = sizing.height;
    replaced_style.aspect_ratio = sizing.aspect_ratio;
    let child = LayoutItem::Block {
        flowable: Box::new(svg.with_available_size(true)) as Box<dyn Flowable>,
        flex_grow: 0.0,
        flex_shrink: 1.0,
        width_spec: None,
        order: 0,
    };
    container_flowables(vec![child], &replaced_style)
}

fn replaced_svg_flowables(
    flowable: Box<dyn Flowable>,
    style: &ComputedStyle,
    width: Pt,
    height: Pt,
) -> Vec<LayoutItem> {
    let mut replaced_style = style.clone();
    if css_size_is_auto(replaced_style.width) {
        replaced_style.width = LengthSpec::Absolute(width);
    }
    if css_size_is_auto(replaced_style.height) {
        replaced_style.height = LengthSpec::Absolute(height);
    }
    let child = LayoutItem::Block {
        flowable,
        flex_grow: 0.0,
        flex_shrink: 1.0,
        width_spec: None,
        order: 0,
    };
    container_flowables(vec![child], &replaced_style)
}

fn parse_dimension(value: Option<&str>) -> Option<Pt> {
    let value = value?;
    let trimmed = value.trim_end_matches("px").trim();
    trimmed
        .parse::<f32>()
        .ok()
        .map(|px| Pt::from_f32(px * 0.75))
}

fn html_th_scope_to_pdf_scope(value: Option<&str>) -> Option<String> {
    let raw = value?.trim();
    if raw.is_empty() {
        return None;
    }
    let lower = raw.to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "col" => "Column".to_string(),
        "row" => "Row".to_string(),
        "colgroup" => "ColGroup".to_string(),
        "rowgroup" => "RowGroup".to_string(),
        "both" => "Both".to_string(),
        _ => raw.to_string(),
    };
    Some(mapped)
}

fn resolve_non_auto_css_dimension(
    spec: LengthSpec,
    basis: Pt,
    font_size: Pt,
    root_font_size: Pt,
    is_height: bool,
) -> Option<Pt> {
    if matches!(
        spec,
        LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
    ) {
        return None;
    }
    let resolved = if is_height {
        spec.resolve_height(basis, font_size, root_font_size)
    } else {
        spec.resolve_width(basis, font_size, root_font_size)
    };
    (resolved > Pt::ZERO).then_some(resolved)
}

fn parse_svg_viewbox_dimensions(value: Option<&str>) -> Option<(Pt, Pt)> {
    let raw = value?.trim();
    if raw.is_empty() {
        return None;
    }
    let mut nums = raw
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<f32>().ok());
    let _min_x = nums.next()?;
    let _min_y = nums.next()?;
    let width = nums.next()?;
    let height = nums.next()?;
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some((Pt::from_f32(width * 0.75), Pt::from_f32(height * 0.75)))
}

fn svg_image_intrinsic_dimensions(xml: &str) -> Option<(Pt, Pt)> {
    let document = crate::xml::Document::parse(xml).ok()?;
    let root = document
        .descendants()
        .find(|node| node.tag_name().name().eq_ignore_ascii_case("svg"))?;
    let width = parse_dimension(root.attribute("width"));
    let height = parse_dimension(root.attribute("height"));
    let viewbox = parse_svg_viewbox_dimensions(
        root.attribute("viewBox")
            .or_else(|| root.attribute("viewbox")),
    );
    let ratio = viewbox.and_then(|(width, height)| {
        let height = height.to_f32();
        (height.is_finite() && height > 0.0).then_some(width.to_f32() / height)
    });

    match (width, height, ratio, viewbox) {
        (Some(width), Some(height), _, _) => Some((width, height)),
        (Some(width), None, Some(ratio), _) if ratio.is_finite() && ratio > 0.0 => {
            Some((width, Pt::from_f32(width.to_f32() / ratio)))
        }
        (None, Some(height), Some(ratio), _) if ratio.is_finite() && ratio > 0.0 => {
            Some((Pt::from_f32(height.to_f32() * ratio), height))
        }
        (None, None, _, Some(viewbox)) => Some(viewbox),
        _ => None,
    }
}

fn resolve_svg_dimensions(
    inline_width: Option<Pt>,
    inline_height: Option<Pt>,
    attr_width: Option<&str>,
    attr_height: Option<&str>,
    view_box: Option<&str>,
    style: &ComputedStyle,
) -> (Pt, Pt) {
    let default_width = Pt::from_f32(300.0 * 0.75);
    let default_height = Pt::from_f32(150.0 * 0.75);
    let css_width = resolve_non_auto_css_dimension(
        style.width,
        default_width,
        style.font_size,
        style.root_font_size,
        false,
    );
    let css_height = resolve_non_auto_css_dimension(
        style.height,
        default_height,
        style.font_size,
        style.root_font_size,
        true,
    );
    let viewbox_size = parse_svg_viewbox_dimensions(view_box);
    let viewbox_ratio = viewbox_size.and_then(|(w, h)| {
        let h = h.to_f32();
        if h <= 0.0 || !h.is_finite() {
            None
        } else {
            Some(w.to_f32() / h)
        }
    });

    let mut width = inline_width
        .or_else(|| parse_dimension(attr_width))
        .or(css_width);
    let mut height = inline_height
        .or_else(|| parse_dimension(attr_height))
        .or(css_height);

    if width.is_none() {
        if let (Some(h), Some(ratio)) = (height, viewbox_ratio) {
            if ratio.is_finite() && ratio > 0.0 {
                width = Some(Pt::from_f32(h.to_f32() * ratio));
            }
        }
    }
    if height.is_none() {
        if let (Some(w), Some(ratio)) = (width, viewbox_ratio) {
            if ratio.is_finite() && ratio > 0.0 {
                height = Some(Pt::from_f32(w.to_f32() / ratio));
            }
        }
    }

    if width.is_none() && height.is_none() {
        if let Some((vbw, vbh)) = viewbox_size {
            width = Some(vbw);
            height = Some(vbh);
        } else {
            width = Some(default_width);
            height = Some(default_height);
        }
    } else {
        if width.is_none() {
            width = Some(default_width);
        }
        if height.is_none() {
            height = Some(default_height);
        }
    }

    (
        width.unwrap_or(default_width).max(Pt::from_f32(1.0)),
        height.unwrap_or(default_height).max(Pt::from_f32(1.0)),
    )
}

fn extract_text(node: &NodeRef, mode: WhiteSpaceMode) -> String {
    let mut out = String::new();
    collect_text(node, &mut out);
    normalize_text(&out, mode, true)
}

fn collect_text(node: &NodeRef, out: &mut String) {
    match node.data() {
        NodeData::Text(text) => {
            out.push_str(&text.borrow());
        }
        NodeData::Element(element) => {
            let tag = element.name.local.as_ref();
            if tag.eq_ignore_ascii_case("br") {
                out.push(FORCED_LINE_BREAK);
                return;
            }
            if tag.eq_ignore_ascii_case("script") || tag.eq_ignore_ascii_case("style") {
                return;
            }
            for child in node.children() {
                collect_text(&child, out);
            }
        }
        _ => {}
    }
}

fn normalize_text(text: &str, mode: WhiteSpaceMode, trim: bool) -> String {
    match mode {
        WhiteSpaceMode::Pre | WhiteSpaceMode::PreWrap | WhiteSpaceMode::BreakSpaces => {
            return text
                .replace("\r\n", "\n")
                .replace('\r', "\n")
                .replace(FORCED_LINE_BREAK, "\n");
        }
        WhiteSpaceMode::PreserveSpaces => {
            return text
                .replace("\r\n", "\n")
                .replace('\r', "\n")
                .chars()
                .map(|ch| {
                    if ch == FORCED_LINE_BREAK {
                        '\n'
                    } else if ch == '\n' || ch == '\t' {
                        ' '
                    } else {
                        ch
                    }
                })
                .collect();
        }
        _ => {}
    }

    let mut out = String::new();
    let mut in_space = false;
    for ch in text.chars() {
        if ch == FORCED_LINE_BREAK {
            while out.ends_with(' ') {
                out.pop();
            }
            out.push('\n');
            in_space = false;
            continue;
        }
        let ch = if ch == '\u{00A0}' { ' ' } else { ch };
        if ch == '\n' {
            match mode {
                WhiteSpaceMode::PreLine => {
                    if out.ends_with(' ') {
                        out.pop();
                    }
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    in_space = false;
                }
                _ => {
                    if !in_space {
                        out.push(' ');
                        in_space = true;
                    }
                }
            }
        } else if ch.is_whitespace() {
            if !in_space {
                out.push(' ');
                in_space = true;
            }
        } else {
            out.push(ch);
            in_space = false;
        }
    }

    if trim {
        out = out
            .trim_matches(|c| c == ' ' || c == '\n' || c == '\t')
            .to_string();
    }
    out
}

fn normalize_text_node_boundaries(
    text: &str,
    mode: WhiteSpaceMode,
    trim_start: bool,
    trim_end: bool,
) -> String {
    let mut cleaned = normalize_text(text, mode, false);
    if preserve_whitespace(mode) {
        return cleaned;
    }
    if trim_start {
        cleaned = cleaned.trim_start_matches([' ', '\n', '\t']).to_string();
    }
    if trim_end {
        cleaned = cleaned.trim_end_matches([' ', '\n', '\t']).to_string();
    }
    cleaned
}

fn apply_text_transform(text: &str, mode: crate::style::TextTransformMode) -> String {
    match mode {
        crate::style::TextTransformMode::None => text.to_string(),
        crate::style::TextTransformMode::Uppercase => text.to_uppercase(),
        crate::style::TextTransformMode::Lowercase => text.to_lowercase(),
        crate::style::TextTransformMode::Capitalize => {
            let mut out = String::with_capacity(text.len());
            let mut new_word = true;
            for ch in text.chars() {
                if ch.is_whitespace() {
                    new_word = true;
                    out.push(ch);
                    continue;
                }
                if new_word {
                    for up in ch.to_uppercase() {
                        out.push(up);
                    }
                    new_word = false;
                } else {
                    out.push(ch);
                }
            }
            out
        }
    }
}

fn css_first_letter_prefix_end(text: &str) -> Option<usize> {
    let mut found_base = false;
    let mut end = 0;
    for (index, ch) in text.char_indices() {
        let next = index + ch.len_utf8();
        if ch.is_whitespace() && !found_base {
            continue;
        }
        let punctuation = ch.is_ascii_punctuation()
            || matches!(ch as u32, 0x2000..=0x206f | 0x2e00..=0x2e7f | 0x3001..=0x303f | 0xff01..=0xff65);
        if !found_base {
            end = next;
            if !punctuation {
                found_base = true;
            }
            continue;
        }
        if punctuation {
            end = next;
        } else {
            break;
        }
    }
    found_base.then_some(end)
}

fn preserve_whitespace(mode: WhiteSpaceMode) -> bool {
    matches!(
        mode,
        WhiteSpaceMode::Pre
            | WhiteSpaceMode::PreWrap
            | WhiteSpaceMode::BreakSpaces
            | WhiteSpaceMode::PreserveSpaces
    )
}

fn text_style_for_flow_text(style: &ComputedStyle) -> TextStyle {
    let mut text_style = style.to_text_style();
    if matches!(style.overflow, OverflowMode::Visible) {
        text_style.text_overflow = crate::style::TextOverflowMode::Clip;
    }
    text_style
}

fn no_wrap(style: &ComputedStyle) -> bool {
    matches!(style.text_wrap_mode, TextWrapMode::NoWrap)
        || matches!(
            style.white_space,
            WhiteSpaceMode::NoWrap | WhiteSpaceMode::Pre
        )
}

fn inline_dimensions(style: Option<&str>) -> (Option<Pt>, Option<Pt>) {
    let style = match style {
        Some(value) => value,
        None => return (None, None),
    };
    let declarations = match crate::css_native::parse_declaration_block(style) {
        Ok(value) => value,
        Err(_) => return (None, None),
    };
    let mut width = None;
    let mut height = None;
    for declaration in declarations.normal().chain(declarations.important()) {
        let slot = if declaration.name_eq("width") {
            &mut width
        } else if declaration.name_eq("height") {
            &mut height
        } else {
            continue;
        };
        if let Some(value) = inline_dimension_to_points(&declaration.value) {
            *slot = value;
        }
    }
    (width, height)
}

fn inline_dimension_to_points(value: &str) -> Option<Option<Pt>> {
    let value = value.trim();
    if let Some(px) = crate::css_native::parse_absolute_length_px(value) {
        if px < 0.0 {
            return None;
        }
        return Some(Some(Pt::from_f32(px * 0.75)));
    }
    let lower = value.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "auto" | "min-content" | "max-content" | "stretch" | "contain"
    ) || lower.ends_with('%')
        || [
            "em", "rem", "ex", "rex", "cap", "rcap", "ch", "rch", "ic", "ric", "lh", "rlh", "vw",
            "vh", "vi", "vb", "vmin", "vmax", "svw", "svh", "lvw", "lvh", "dvw", "dvh", "cqw",
            "cqh", "cqi", "cqb", "cqmin", "cqmax",
        ]
        .iter()
        .any(|unit| lower.ends_with(unit))
        || ["calc(", "min(", "max(", "clamp(", "fit-content("]
            .iter()
            .any(|prefix| lower.starts_with(prefix) && lower.ends_with(')'))
    {
        // These are valid CSS sizes but cannot be resolved without layout context. The previous
        // typed path likewise returned no intrinsic point override for them.
        return Some(None);
    }
    None
}
