use crate::assets::{
    AssetBundle, load_svg_xml_from_image_source, raster_image_intrinsic_dimensions,
    renderable_image_source,
};
use crate::flowable::{
    AbsolutePositionedFlowable, AlignContent, AlignItems, BackgroundPaintFlowable, BorderRadiiSpec,
    BorderSpec, CalcLength, ContainerFlowable, EdgeSizes, FlexDirection, FlexFlowable,
    ImageFlowable, InlineBlockLayoutFlowable, JustifyContent, LengthSpec, ListItemFlowable,
    MetaFlowable, Paragraph, RelativePositionedFlowable, Spacer, SvgFlowable, TableCell,
    TableColumnBorder, TableColumnGroupBorder, TableColumnWidthHint, TableFlowable,
    TableLayoutMode, TextAlign, TextStyle, VerticalAlign,
};
use crate::font::FontRegistry;
use crate::glyph_report::GlyphCoverageReport;
use crate::html_dom::{NodeData, NodeRef, parse_html};
use crate::style::{
    AlignContentMode, AlignItemsMode, AlignSelfMode, ComputedStyle, DirectionMode, DisplayMode,
    ElementInfo, FlexDirectionMode, FlexWrapMode, GeneratedContentPart, GeneratedCounterContent,
    GeneratedCounterStyle, GeneratedCountersContent, JustifyContentMode, ListStylePositionMode,
    ListStyleTypeMode, OverflowMode, PositionMode, StyleResolver, TextAlignLastMode, TextAlignMode,
    TextWrapMode, VerticalAlignMode, VisibilityMode, WhiteSpaceMode,
};
use crate::types::Pt;
use crate::{BreakAfter, BreakBefore, BreakInside, Flowable};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct HtmlAssetWarning {
    pub kind: String,
    pub message: String,
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct CounterState {
    values: HashMap<String, Vec<i32>>,
}

impl CounterState {
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
    match style.vertical_align {
        VerticalAlignMode::Middle => VerticalAlign::Middle,
        VerticalAlignMode::Bottom | VerticalAlignMode::TextBottom | VerticalAlignMode::Sub => {
            VerticalAlign::Bottom
        }
        _ => VerticalAlign::Top,
    }
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
    let base_style = resolver.default_style();
    let mut ancestors: Vec<ElementInfo> = Vec::new();
    let mut report = report;
    let mut counters = CounterState::default();

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
        root_style =
            resolver.compute_style(&html_info, &base_style, inline_style.as_deref(), &ancestors);
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
        let body_node = body.as_node();
        let body_element = body_node.as_element().expect("body element");
        let mut body_info = element_info(body_node, resolver.has_sibling_selectors());
        let inline_style = body_element
            .attributes
            .borrow()
            .get("style")
            .map(|s| s.to_string());
        let t_body = std::time::Instant::now();
        let body_style =
            resolver.compute_style(&body_info, &root_style, inline_style.as_deref(), &ancestors);
        body_info.apply_computed_container_style(&body_style);
        apply_style_counters_for_node(
            body_node,
            resolver,
            &body_style,
            &body_info,
            &ancestors,
            &mut counters,
        );
        ancestors.push(body_info);
        if let Some(perf_logger) = perf {
            let ms = t_body.elapsed().as_secs_f64() * 1000.0;
            perf_logger.log_span_ms("story.style.body", doc_id, ms);
        }
        let t_collect = std::time::Instant::now();
        let items = collect_children(
            body_node,
            resolver,
            &body_style,
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
            &document,
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
        || info.attrs.contains_key("data-fb-role")
        || info.attrs.contains_key("data-fb-component")
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
    for child in node.children() {
        out.extend(node_to_flowables(
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
        ));
    }
    out
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
        NodeData::Text(text) => {
            if let Some(perf_logger) = perf {
                perf_logger.log_counts("story.text_nodes", doc_id, &[("count", 1)]);
            }
            let text = text.borrow();
            let t_norm = std::time::Instant::now();
            let cleaned = normalize_text(&text, parent_style.white_space, true);
            if let Some(perf_logger) = perf {
                let ms = t_norm.elapsed().as_secs_f64() * 1000.0;
                perf_logger.log_span_ms("story.text.normalize", doc_id, ms);
            }
            if cleaned.is_empty() {
                Vec::new()
            } else {
                let t_transform = std::time::Instant::now();
                let cleaned = apply_text_transform(&cleaned, parent_style.text_transform);
                if let Some(perf_logger) = perf {
                    let ms = t_transform.elapsed().as_secs_f64() * 1000.0;
                    perf_logger.log_span_ms("story.text.transform", doc_id, ms);
                }
                let text_style = text_style_for_flow_text(parent_style);
                let t_glyph = std::time::Instant::now();
                report_missing_glyphs(
                    report.as_deref_mut(),
                    font_registry.as_deref(),
                    &text_style,
                    &cleaned,
                );
                if let Some(perf_logger) = perf {
                    let ms = t_glyph.elapsed().as_secs_f64() * 1000.0;
                    perf_logger.log_span_ms("story.glyph.report", doc_id, ms);
                }
                let paragraph = Paragraph::new(cleaned)
                    .with_style(text_style)
                    .with_align(text_align_from_style(parent_style))
                    .with_last_align(text_align_last_from_style(parent_style))
                    .with_whitespace(
                        preserve_whitespace(parent_style.white_space),
                        no_wrap(parent_style),
                    )
                    .with_pagination(parent_style.pagination)
                    .with_font_registry(font_registry.clone())
                    .with_tag_role("P");
                vec![LayoutItem::Block {
                    flowable: Box::new(paragraph) as Box<dyn Flowable>,
                    flex_grow: 0.0,
                    flex_shrink: 1.0,
                    width_spec: None,
                    order: 0,
                }]
            }
        }
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
            let mut style =
                resolver.compute_style(&info, parent_style, inline_style.as_deref(), ancestors);
            info.apply_computed_container_style(&style);
            if let Some(perf_logger) = perf {
                let ms = t_style.elapsed().as_secs_f64() * 1000.0;
                perf_logger.log_span_ms("story.style.compute", doc_id, ms);
            }
            let parent_is_flex = matches!(
                parent_style.display,
                DisplayMode::Flex
                    | DisplayMode::InlineFlex
                    | DisplayMode::Grid
                    | DisplayMode::InlineGrid
            );
            let has_renderable_content = node_has_renderable_content(node);
            let mut flex_item_width_spec = None;
            let mut flex_item_width_from_width = false;
            if parent_is_flex {
                if !matches!(
                    style.flex_basis,
                    LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
                ) {
                    flex_item_width_spec = Some(style.flex_basis);
                } else if !matches!(
                    style.width,
                    LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
                ) {
                    flex_item_width_spec = Some(style.width);
                    flex_item_width_from_width = true;
                }
                if flex_item_width_from_width && has_renderable_content {
                    style.width = LengthSpec::Auto;
                }
            }
            if info.classes.iter().any(|c| c == "keep-together") {
                style.pagination.break_inside = BreakInside::Avoid;
            }
            let node_meta = authored_owner_metadata(&info, ancestors, &explicit_node_meta, &style);

            if matches!(style.display, DisplayMode::None) {
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

            let mut before_counter_probe = counters.clone();
            let before_items = pseudo_items_for(
                resolver,
                &info,
                &style,
                ancestors,
                &mut before_counter_probe,
                font_registry.clone(),
                report.as_deref_mut(),
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
                report.as_deref_mut(),
                crate::style::PseudoTarget::After,
            );

            // Maintain an ancestor stack instead of cloning it for every element.
            let list_item_marker_ancestors = if style_is_css_list_item(&style) {
                Some(ancestors.clone())
            } else {
                None
            };
            ancestors.push(info.clone());

            // Contents/inline are usually transparent containers in our layout model, except
            // replaced/special inline elements that render atomic content.
            let transparent_inline =
                matches!(style.display, DisplayMode::Contents | DisplayMode::Inline)
                    && !matches!(info.tag.as_str(), "img" | "svg" | "br");
            if transparent_inline {
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

            let mut flowables = if let Some(marker_ancestors) = list_item_marker_ancestors.as_ref()
            {
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
                    "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        let role = match info.tag.as_str() {
                            "h1" => "H1",
                            "h2" => "H2",
                            "h3" => "H3",
                            "h4" => "H4",
                            "h5" => "H5",
                            "h6" => "H6",
                            _ => "P",
                        };

                        if inline_children_only(node, resolver, &style, ancestors) {
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
                                    .with_whitespace(
                                        preserve_whitespace(style.white_space),
                                        no_wrap(&style),
                                    )
                                    .with_pagination(style.pagination)
                                    .with_font_registry(font_registry.clone())
                                    .with_tag_role(role);
                                let items = vec![LayoutItem::Block {
                                    flowable: Box::new(paragraph) as Box<dyn Flowable>,
                                    flex_grow: 0.0,
                                    flex_shrink: 1.0,
                                    width_spec: None,
                                    order: 0,
                                }];
                                container_flowables(items, &style)
                            }
                        } else {
                            let coerce_mixed_inline =
                                inline_or_replaced_children_only(node, resolver, &style, ancestors);
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
                                )
                            } else {
                                children
                            };
                            container_flowables_with_role(children, &style, Some(role))
                        }
                    }
                    "pre" => {
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
                    }
                    "br" => {
                        let height = style.to_text_style().line_height.max(style.font_size);
                        let spacer = Spacer::new_pt(height);
                        vec![LayoutItem::Block {
                            flowable: Box::new(spacer) as Box<dyn Flowable>,
                            flex_grow: 0.0,
                            flex_shrink: 1.0,
                            width_spec: flex_item_basis(&style),
                            order: 0,
                        }]
                    }
                    "img" => {
                        let attrs = element.attributes.borrow();
                        let src = attrs.get("src").unwrap_or("image");
                        let inline_width_height = inline_dimensions(inline_style.as_deref());
                        let css_width = if matches!(
                            style.width,
                            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
                        ) {
                            None
                        } else {
                            let resolved = style.width.resolve_width(
                                Pt::from_f32(300.0),
                                style.font_size,
                                style.root_font_size,
                            );
                            (resolved > Pt::ZERO).then_some(resolved)
                        };
                        let css_height = if matches!(
                            style.height,
                            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
                        ) {
                            None
                        } else {
                            let resolved = style.height.resolve_height(
                                Pt::from_f32(150.0),
                                style.font_size,
                                style.root_font_size,
                            );
                            (resolved > Pt::ZERO).then_some(resolved)
                        };
                        let mut width = inline_width_height
                            .0
                            .or_else(|| parse_dimension(attrs.get("width")))
                            .or(css_width);
                        let mut height = inline_width_height
                            .1
                            .or_else(|| parse_dimension(attrs.get("height")))
                            .or(css_height);
                        let intrinsic_size =
                            raster_image_intrinsic_dimensions(asset_bundle.as_deref(), src).map(
                                |(w, h)| {
                                    (Pt::from_f32(w as f32 * 0.75), Pt::from_f32(h as f32 * 0.75))
                                },
                            );
                        let intrinsic_ratio = intrinsic_size.and_then(|(w, h)| {
                            let h = h.to_f32();
                            if h <= 0.0 || !h.is_finite() {
                                None
                            } else {
                                Some(w.to_f32() / h)
                            }
                        });
                        if width.is_none() {
                            if let (Some(h), Some(ratio)) = (height, intrinsic_ratio) {
                                if ratio.is_finite() && ratio > 0.0 {
                                    width = Some(Pt::from_f32(h.to_f32() * ratio));
                                }
                            }
                        }
                        if height.is_none() {
                            if let (Some(w), Some(ratio)) = (width, intrinsic_ratio) {
                                if ratio.is_finite() && ratio > 0.0 {
                                    height = Some(Pt::from_f32(w.to_f32() / ratio));
                                }
                            }
                        }
                        if width.is_none() && height.is_none() {
                            if let Some((intrinsic_width, intrinsic_height)) = intrinsic_size {
                                width = Some(intrinsic_width);
                                height = Some(intrinsic_height);
                            }
                        }
                        let width = width
                            .unwrap_or_else(|| style.font_size * 4.0)
                            .max(Pt::from_f32(1.0));
                        let height = height
                            .unwrap_or_else(|| style.font_size * 3.0)
                            .max(Pt::from_f32(1.0));
                        let alt = attrs
                            .get("alt")
                            .or_else(|| attrs.get("aria-label"))
                            .or_else(|| attrs.get("title"))
                            .map(|s| s.to_string());
                        let width_spec = flex_item_basis(&style);
                        if let Some(xml) =
                            load_svg_xml_from_image_source(asset_bundle.as_deref(), src)
                        {
                            if svg_raster_fallback && crate::svg::svg_needs_raster_fallback(&xml) {
                                if let Some(data_uri) =
                                    crate::svg::rasterize_svg_to_data_uri(&xml, width, height)
                                {
                                    let image = ImageFlowable::new_pt(width, height, data_uri)
                                        .with_object_fit(style.object_fit)
                                        .with_object_position(style.object_position)
                                        .with_intrinsic_size(intrinsic_size)
                                        .with_font_metrics(style.font_size, style.root_font_size)
                                        .with_pagination(style.pagination)
                                        .with_visible(style.visibility.paints())
                                        .with_tag_role("Figure")
                                        .with_alt(alt);
                                    vec![LayoutItem::Block {
                                        flowable: Box::new(image) as Box<dyn Flowable>,
                                        flex_grow: 0.0,
                                        flex_shrink: 1.0,
                                        width_spec,
                                        order: 0,
                                    }]
                                } else {
                                    let xml_len = xml.len() as u64;
                                    let t_svg = std::time::Instant::now();
                                    let svg = SvgFlowable::new_pt(width, height, xml)
                                        .with_pagination(style.pagination)
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
                                    vec![LayoutItem::Block {
                                        flowable: Box::new(svg) as Box<dyn Flowable>,
                                        flex_grow: 0.0,
                                        flex_shrink: 1.0,
                                        width_spec,
                                        order: 0,
                                    }]
                                }
                            } else {
                                let xml_len = xml.len() as u64;
                                let t_svg = std::time::Instant::now();
                                let svg = SvgFlowable::new_pt(width, height, xml)
                                    .with_pagination(style.pagination)
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
                                vec![LayoutItem::Block {
                                    flowable: Box::new(svg) as Box<dyn Flowable>,
                                    flex_grow: 0.0,
                                    flex_shrink: 1.0,
                                    width_spec,
                                    order: 0,
                                }]
                            }
                        } else {
                            let image_source =
                                renderable_image_source(asset_bundle.as_deref(), src)
                                    .unwrap_or_else(|| src.to_string());
                            let image = ImageFlowable::new_pt(width, height, image_source)
                                .with_object_fit(style.object_fit)
                                .with_object_position(style.object_position)
                                .with_intrinsic_size(intrinsic_size)
                                .with_font_metrics(style.font_size, style.root_font_size)
                                .with_pagination(style.pagination)
                                .with_visible(style.visibility.paints())
                                .with_tag_role("Figure")
                                .with_alt(alt);
                            vec![LayoutItem::Block {
                                flowable: Box::new(image) as Box<dyn Flowable>,
                                flex_grow: 0.0,
                                flex_shrink: 1.0,
                                width_spec,
                                order: 0,
                            }]
                        }
                    }
                    "svg" => {
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
                                    .with_font_metrics(style.font_size, style.root_font_size)
                                    .with_pagination(style.pagination)
                                    .with_visible(style.visibility.paints())
                                    .with_tag_role("Figure")
                                    .with_alt(alt);
                                vec![LayoutItem::Block {
                                    flowable: Box::new(image) as Box<dyn Flowable>,
                                    flex_grow: 0.0,
                                    flex_shrink: 1.0,
                                    width_spec: flex_item_basis(&style),
                                    order: 0,
                                }]
                            } else {
                                let xml_len = xml.len() as u64;
                                let t_svg = std::time::Instant::now();
                                let svg = SvgFlowable::new_pt(width, height, xml)
                                    .with_pagination(style.pagination)
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
                                vec![LayoutItem::Block {
                                    flowable: Box::new(svg) as Box<dyn Flowable>,
                                    flex_grow: 0.0,
                                    flex_shrink: 1.0,
                                    width_spec: flex_item_basis(&style),
                                    order: 0,
                                }]
                            }
                        } else {
                            let xml_len = xml.len() as u64;
                            let t_svg = std::time::Instant::now();
                            let svg = SvgFlowable::new_pt(width, height, xml)
                                .with_pagination(style.pagination)
                                .with_form_enabled(svg_form)
                                .with_visible(style.visibility.paints())
                                .with_tag_role("Figure")
                                .with_alt(alt);
                            if let Some(perf_logger) = perf {
                                let ms = t_svg.elapsed().as_secs_f64() * 1000.0;
                                perf_logger.log_span_ms("svg.compile", None, ms);
                                perf_logger.log_counts("svg.compile", None, &[("bytes", xml_len)]);
                            }
                            vec![LayoutItem::Block {
                                flowable: Box::new(svg) as Box<dyn Flowable>,
                                flex_grow: 0.0,
                                flex_shrink: 1.0,
                                width_spec: flex_item_basis(&style),
                                order: 0,
                            }]
                        }
                    }
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
                    "table" => {
                        let include_prev_siblings = resolver.has_sibling_selectors();
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
                        let table_border_colors = style.resolved_border_colors(style.color);
                        let table_border_styles = style.resolved_border_styles();
                        let table_hidden_borders = style.border_hidden_sides();
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
                        .with_font_metrics(style.font_size, style.root_font_size);

                        let mut table_children: Vec<Box<dyn Flowable>> = Vec::new();
                        table_children.extend(top_caption_flowables);
                        table_children.push(Box::new(table) as Box<dyn Flowable>);
                        table_children.extend(bottom_caption_flowables);

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
                        .with_border_styles(
                            table_border_styles.top,
                            table_border_styles.right,
                            table_border_styles.bottom,
                            table_border_styles.left,
                        )
                        .with_border_radius(style.border_radius)
                        .with_outline(
                            style.outline_width,
                            style.outline_offset,
                            style.outline_style,
                            style.resolved_outline_color(),
                            style.outline_visible,
                        )
                        .with_padding(style.padding)
                        .with_box_sizing(style.box_sizing)
                        .with_width(style.width)
                        .with_max_width(style.max_width)
                        .with_min_width(style.min_width)
                        .with_height(style.height)
                        .with_min_height(style.min_height)
                        .with_max_height(style.max_height)
                        .with_background(style.background_color)
                        .with_background_paint(style.background_paint.clone())
                        .with_background_layers(
                            style.background_paints.clone(),
                            style.background_sizes.clone(),
                            style.background_positions.clone(),
                            style.background_repeats.clone(),
                            style.background_origins.clone(),
                            style.background_clips.clone(),
                        )
                        .with_background_blend_modes(style.background_blend_modes.clone())
                        .with_clip_path(style.clip_path.clone())
                        .with_clip_path_reference_box(style.clip_path_reference_box)
                        .with_box_shadows(style.box_shadows.clone())
                        .with_paint_filter(style.paint_filter.clone())
                        .with_backdrop_filter(style.backdrop_filter.clone())
                        .with_will_change_backdrop_root(style.will_change_backdrop_root)
                        .with_mask_backdrop_root(style.mask_backdrop_root)
                        .with_mix_blend_mode(style.mix_blend_mode)
                        .with_isolation(style.isolation)
                        .with_opacity(style.opacity)
                        .with_transforms(style.transform.clone())
                        .with_transform_origin(style.transform_origin)
                        .with_overflow_hidden(matches!(style.overflow, OverflowMode::Hidden))
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
                    "ul" | "ol" => {
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
                        container_flowables_with_role(items, &style, Some("L"))
                    }
                    "li" => {
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
                    }
                    "div" | "span" | "section" | "article" | "header" | "footer" | "aside"
                    | "nav" | "main" | "blockquote" | "dl" | "dt" | "dd" => {
                        let dl_container_role = definition_list_container_role(info.tag.as_str());
                        let dl_inline_text_role =
                            definition_list_inline_text_role(info.tag.as_str());
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
                            )
                        } else if matches!(
                            style.display,
                            DisplayMode::Block | DisplayMode::InlineBlock
                        ) && inline_children_only(node, resolver, &style, ancestors)
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
                            if text.is_empty() {
                                if matches!(info.tag.as_str(), "dl") {
                                    container_flowables_with_role(
                                        Vec::new(),
                                        &style,
                                        dl_container_role,
                                    )
                                } else {
                                    container_flowables(Vec::new(), &style)
                                }
                            } else {
                                let text = apply_text_transform(&text, style.text_transform);
                                let text_style = text_style_for_flow_text(&style);
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
                                    .with_pagination(style.pagination)
                                    .with_font_registry(font_registry.clone());
                                let paragraph = if let Some(role) = dl_inline_text_role {
                                    paragraph.with_tag_role(role)
                                } else {
                                    paragraph
                                };
                                let items = vec![LayoutItem::Block {
                                    flowable: Box::new(paragraph) as Box<dyn Flowable>,
                                    flex_grow: 0.0,
                                    flex_shrink: 1.0,
                                    width_spec: None,
                                    order: 0,
                                }];
                                if matches!(info.tag.as_str(), "dl") {
                                    container_flowables_with_role(items, &style, dl_container_role)
                                } else {
                                    container_flowables(items, &style)
                                }
                            }
                        } else {
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
                            if dl_container_role.is_some() {
                                container_flowables_with_role(children, &style, dl_container_role)
                            } else {
                                container_flowables(children, &style)
                            }
                        }
                    }
                    _ => {
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
                    }
                }
            };

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
            }

            let width_spec_override = if parent_is_flex {
                flex_item_width_spec
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
            if matches!(
                style.display,
                DisplayMode::InlineBlock
                    | DisplayMode::InlineTable
                    | DisplayMode::InlineFlex
                    | DisplayMode::InlineGrid
            ) {
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
        TextAlignMode::Center => TextAlign::Center,
        TextAlignMode::Right => TextAlign::Right,
        TextAlignMode::Left => TextAlign::Left,
        TextAlignMode::Justify | TextAlignMode::JustifyAll => TextAlign::Justify,
    }
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

fn generated_content_text(style: &ComputedStyle, counters: &CounterState) -> Option<String> {
    let Some(parts) = &style.generated_content else {
        return style.content.clone();
    };
    let mut out = String::new();
    for part in parts {
        match part {
            GeneratedContentPart::Text(text) => out.push_str(text),
            GeneratedContentPart::Counter(counter) => {
                out.push_str(&generated_counter_text(
                    counter,
                    counters.get(&counter.name),
                ));
            }
            GeneratedContentPart::Counters(counter) => {
                out.push_str(&generated_counters_text(
                    counter,
                    &counters.get_all(&counter.name),
                ));
            }
        }
    }
    Some(out)
}

fn pseudo_content_items(
    style: &ComputedStyle,
    counters: &mut CounterState,
    font_registry: Option<Arc<FontRegistry>>,
    report: Option<&mut GlyphCoverageReport>,
) -> Vec<LayoutItem> {
    if !style_can_mutate_counters(style) {
        return Vec::new();
    }
    apply_style_counters(style, counters);
    let Some(content) = generated_content_text(style, counters) else {
        return Vec::new();
    };
    if content.is_empty() {
        return Vec::new();
    }
    let text = apply_text_transform(&content, style.text_transform);
    let text_style = text_style_for_flow_text(style);
    report_missing_glyphs(report, font_registry.as_deref(), &text_style, &text);
    let paragraph = Paragraph::new(text)
        .with_style(text_style)
        .with_align(text_align_from_style(style))
        .with_last_align(text_align_last_from_style(style))
        .with_whitespace(preserve_whitespace(style.white_space), no_wrap(style))
        .with_pagination(style.pagination)
        .with_font_registry(font_registry);
    let is_inline = matches!(
        style.display,
        DisplayMode::Inline
            | DisplayMode::InlineBlock
            | DisplayMode::InlineTable
            | DisplayMode::InlineFlex
            | DisplayMode::InlineGrid
    );
    if is_inline {
        let valign = vertical_align_from_style(style);
        vec![LayoutItem::Inline {
            flowable: Box::new(paragraph) as Box<dyn Flowable>,
            valign,
            flex_grow: style.flex_grow,
            flex_shrink: style.flex_shrink,
            width_spec: flex_item_basis(style),
            order: 0,
        }]
    } else {
        vec![LayoutItem::Block {
            flowable: Box::new(paragraph) as Box<dyn Flowable>,
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
    report: Option<&mut GlyphCoverageReport>,
    pseudo: crate::style::PseudoTarget,
) -> Vec<LayoutItem> {
    let Some(pseudo_style) = resolver.compute_pseudo_style(info, style, ancestors, pseudo) else {
        return Vec::new();
    };
    pseudo_content_items(&pseudo_style, counters, font_registry, report)
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

fn marker_prefix_from_pseudo(
    resolver: &StyleResolver,
    info: &ElementInfo,
    style: &ComputedStyle,
    ancestors: &[ElementInfo],
    counters: &CounterState,
) -> Option<Option<String>> {
    let marker_style = resolver.compute_pseudo_style(
        info,
        style,
        ancestors,
        crate::style::PseudoTarget::Marker,
    )?;
    Some(
        generated_content_text(&marker_style, counters)
            .map(|content| apply_text_transform(&content, marker_style.text_transform)),
    )
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
    let size = style.font_size.max(Pt::from_f32(1.0));
    if let Some(paint) = style.list_style_image_paint.as_ref() {
        let marker = BackgroundPaintFlowable::new_pt(size, size, paint.clone())
            .with_pagination(style.pagination)
            .with_visible(style.visibility.paints())
            .with_tag_role("Lbl");
        return Some(Box::new(marker) as Box<dyn Flowable>);
    }
    let intrinsic_size = raster_image_intrinsic_dimensions(asset_bundle, source)
        .map(|(w, h)| (Pt::from_f32(w as f32 * 0.75), Pt::from_f32(h as f32 * 0.75)));
    if let Some(xml) = load_svg_xml_from_image_source(asset_bundle, source) {
        if svg_raster_fallback && crate::svg::svg_needs_raster_fallback(&xml) {
            if let Some(data_uri) = crate::svg::rasterize_svg_to_data_uri(&xml, size, size) {
                let image = ImageFlowable::new_pt(size, size, data_uri)
                    .with_intrinsic_size(intrinsic_size)
                    .with_font_metrics(style.font_size, style.root_font_size)
                    .with_pagination(style.pagination)
                    .with_visible(style.visibility.paints())
                    .with_tag_role("Lbl");
                return Some(Box::new(image) as Box<dyn Flowable>);
            }
        }
        let svg = SvgFlowable::new_pt(size, size, xml)
            .with_pagination(style.pagination)
            .with_form_enabled(svg_form)
            .with_visible(style.visibility.paints())
            .with_tag_role("Lbl");
        return Some(Box::new(svg) as Box<dyn Flowable>);
    }

    let image_source =
        renderable_image_source(asset_bundle, source).unwrap_or_else(|| source.to_string());
    let image = ImageFlowable::new_pt(size, size, image_source)
        .with_intrinsic_size(intrinsic_size)
        .with_font_metrics(style.font_size, style.root_font_size)
        .with_pagination(style.pagination)
        .with_visible(style.visibility.paints())
        .with_tag_role("Lbl");
    Some(Box::new(image) as Box<dyn Flowable>)
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
        // This flattening fast-path is text-centric. Replaced/media elements must keep
        // structural flowables or content can disappear (for example img/svg-only wrappers).
        let tag = element.name.local.as_ref().to_ascii_lowercase();
        if matches!(
            tag.as_str(),
            "img"
                | "svg"
                | "br"
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
        let info = element_info(&child, resolver.has_sibling_selectors());
        let inline_style = element
            .attributes
            .borrow()
            .get("style")
            .map(|s| s.to_string());
        let child_style =
            resolver.compute_style(&info, parent_style, inline_style.as_deref(), ancestors);
        match child_style.display {
            DisplayMode::Inline
            | DisplayMode::InlineBlock
            | DisplayMode::InlineFlex
            | DisplayMode::InlineGrid
            | DisplayMode::Contents => {}
            _ => return false,
        }
    }
    true
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
                if matches!(
                    tag.as_str(),
                    "br" | "hr" | "canvas" | "video" | "audio" | "iframe" | "object" | "embed"
                ) {
                    return false;
                }
                let info = element_info(&child, resolver.has_sibling_selectors());
                let inline_style = element
                    .attributes
                    .borrow()
                    .get("style")
                    .map(|s| s.to_string());
                let child_style =
                    resolver.compute_style(&info, parent_style, inline_style.as_deref(), ancestors);
                match child_style.display {
                    DisplayMode::Inline
                    | DisplayMode::InlineBlock
                    | DisplayMode::InlineFlex
                    | DisplayMode::InlineGrid
                    | DisplayMode::InlineTable
                    | DisplayMode::Contents => {
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

fn coerce_items_to_inline_run(
    items: Vec<LayoutItem>,
    default_valign: VerticalAlign,
) -> Vec<LayoutItem> {
    items
        .into_iter()
        .map(|item| match item {
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
        })
        .collect()
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
    let marker_override =
        marker_prefix_from_pseudo(resolver, info, style, marker_ancestors, counters);
    let marker_image = if marker_override.is_none() {
        list_marker_image_flowable(
            style,
            asset_bundle.as_deref(),
            svg_form,
            svg_raster_fallback,
        )
    } else {
        None
    };
    let marker_prefix = match marker_override {
        Some(prefix) => prefix,
        None if marker_image.is_some() => None,
        None => list_marker_prefix(style, false, current_list_item_counter_index(counters)),
    };
    let marker_is_inside = matches!(style.list_style_position, ListStylePositionMode::Inside);
    let mut consumed_inside_marker = false;
    let body: Box<dyn Flowable> = if marker_is_inside
        && marker_prefix.is_some()
        && inline_children_only(node, resolver, style, child_ancestors)
    {
        let text = extract_text(node, style.white_space);
        let text = format!("{}{}", marker_prefix.as_deref().unwrap_or(""), text);
        let text = apply_text_transform(&text, style.text_transform);
        let text_style = text_style_for_flow_text(style);
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
                .with_align(text_align_from_style(style))
                .with_last_align(text_align_last_from_style(style))
                .with_whitespace(preserve_whitespace(style.white_space), no_wrap(style))
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
            ListItemFlowable::new_with_label(label, body, Pt::from_f32(4.0))
                .with_pagination(style.pagination),
        ) as Box<dyn Flowable>
    } else if marker_prefix.is_none() {
        body
    } else {
        let prefix = marker_prefix.unwrap_or_default();
        let text_style = text_style_for_flow_text(style);
        report_missing_glyphs(
            report.as_deref_mut(),
            font_registry.as_deref(),
            &text_style,
            &prefix,
        );
        let label_para = Paragraph::new(prefix)
            .with_style(text_style)
            .with_align(text_align_from_style(style))
            .with_last_align(text_align_last_from_style(style))
            .with_whitespace(preserve_whitespace(style.white_space), no_wrap(style))
            .with_pagination(style.pagination)
            .with_font_registry(font_registry.clone())
            .with_tag_role("Lbl");
        Box::new(
            ListItemFlowable::new(label_para, body, Pt::from_f32(4.0))
                .with_pagination(style.pagination),
        ) as Box<dyn Flowable>
    };

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

fn node_has_renderable_content(node: &NodeRef) -> bool {
    if node.text_contents().trim().is_empty() {
        node.children()
            .any(|child| matches!(child.data(), NodeData::Element(_)))
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq_pt(value: Pt, expected: f32) -> bool {
        (value.to_f32() - expected).abs() <= 0.01
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
                suffix: " ".to_string(),
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
    let mut index = list_start_index(node, ordered);

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
            apply_style_counters(&style, counters);
            apply_implicit_list_item_counter(&style, counters);
            let is_inline = matches!(
                style.display,
                DisplayMode::Inline
                    | DisplayMode::InlineBlock
                    | DisplayMode::InlineTable
                    | DisplayMode::InlineFlex
                    | DisplayMode::InlineGrid
            );
            let marker_override = if is_inline {
                None
            } else {
                marker_prefix_from_pseudo(resolver, &info, &style, ancestors, counters)
            };
            let marker_image = if !is_inline && marker_override.is_none() {
                list_marker_image_flowable(
                    &style,
                    asset_bundle.as_deref(),
                    svg_form,
                    svg_raster_fallback,
                )
            } else {
                None
            };
            let marker_prefix = if is_inline {
                None
            } else {
                match marker_override {
                    Some(prefix) => prefix,
                    None if marker_image.is_some() => None,
                    None => list_marker_prefix(&style, ordered, index),
                }
            };
            let marker_is_inside =
                matches!(style.list_style_position, ListStylePositionMode::Inside);
            index = index.saturating_add(1);

            let mut li_ancestors = ancestors.to_vec();
            li_ancestors.push(info.clone());
            let mut consumed_inside_marker = false;
            let li_body: Box<dyn Flowable> = if marker_is_inside
                && marker_prefix.is_some()
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
                        .with_pagination(style.pagination)
                        .with_font_registry(font_registry.clone())
                        .with_tag_role("LBody"),
                ) as Box<dyn Flowable>
            } else {
                let mut li_body_items: Vec<LayoutItem> = Vec::new();
                for li_child in child.children() {
                    li_body_items.extend(node_to_flowables(
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
                    ListItemFlowable::new_with_label(label, li_body, Pt::from_f32(4.0))
                        .with_pagination(style.pagination),
                ) as Box<dyn Flowable>
            } else if let Some(prefix) = marker_prefix {
                let text_style = text_style_for_flow_text(&style);
                report_missing_glyphs(
                    report.as_deref_mut(),
                    font_registry.as_deref(),
                    &text_style,
                    &prefix,
                );
                let label_para = Paragraph::new(prefix)
                    .with_style(text_style)
                    .with_align(text_align_from_style(&style))
                    .with_last_align(text_align_last_from_style(&style))
                    .with_whitespace(preserve_whitespace(style.white_space), no_wrap(&style))
                    .with_pagination(style.pagination)
                    .with_font_registry(font_registry.clone())
                    .with_tag_role("Lbl");
                Box::new(
                    ListItemFlowable::new(label_para, li_body, Pt::from_f32(4.0))
                        .with_pagination(style.pagination),
                ) as Box<dyn Flowable>
            } else {
                li_body
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

fn list_start_index(node: &NodeRef, ordered: bool) -> usize {
    if !ordered {
        return 1;
    }
    node.as_element()
        .and_then(|el| {
            el.attributes
                .borrow()
                .get("start")
                .and_then(|value| value.trim().parse::<usize>().ok())
        })
        .filter(|value| *value > 0)
        .unwrap_or(1)
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
            .and_then(|symbols| positive.map(|index| anonymous_symbols_list_marker(index, symbols)))
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
                Some(format!(
                    "{}{}",
                    anonymous_symbols_list_marker(index, symbols),
                    symbols.suffix
                ))
            } else {
                Some(format!("{}. ", index))
            }
        }
        crate::style::ListStyleTypeMode::AnonymousSymbols => {
            style.list_style_symbols.as_ref().map(|symbols| {
                format!(
                    "{}{}",
                    anonymous_symbols_list_marker(index, symbols),
                    symbols.suffix
                )
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
    if spec.symbols.is_empty() {
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

fn establishes_abs_containing_block(style: &ComputedStyle) -> bool {
    !matches!(style.position, PositionMode::Static) || !style.transform.is_empty()
}

fn container_flowable_with_role(
    children: Vec<LayoutItem>,
    style: &ComputedStyle,
    role: Option<&str>,
) -> Option<Box<dyn Flowable>> {
    let has_box = !matches!(style.width, LengthSpec::Auto)
        || !matches!(style.height, LengthSpec::Auto)
        || style.background_color.is_some()
        || style.background_paint.is_some()
        || style.clip_path.is_some()
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
        || style.border_width != EdgeSizes::zero();

    if children.is_empty() && !has_box {
        // Preserve page-break semantics even for empty elements.
        if style.pagination.break_before != BreakBefore::Auto
            || style.pagination.break_after != BreakAfter::Auto
        {
            let mut container =
                ContainerFlowable::new_pt(Vec::new(), style.font_size, style.root_font_size)
                    .with_establishes_abs_containing_block(establishes_abs_containing_block(style))
                    .with_transforms(style.transform.clone())
                    .with_transform_origin(style.transform_origin)
                    .with_self_visible(style.visibility.paints())
                    .with_pagination(style.pagination);
            if let Some(role) = role {
                container = container.with_tag_role(role);
            }
            return Some(Box::new(container) as Box<dyn Flowable>);
        }
        return None;
    }

    let forced_line_height = match style.height {
        LengthSpec::Absolute(value) if value > Pt::ZERO => Some(value),
        _ => None,
    };

    let flowables = layout_children_to_flowables(children, forced_line_height);
    let mut container = ContainerFlowable::new_pt(flowables, style.font_size, style.root_font_size)
        .with_establishes_abs_containing_block(establishes_abs_containing_block(style))
        .with_margin(style.margin)
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
        .with_border_styles(
            style.resolved_border_styles().top,
            style.resolved_border_styles().right,
            style.resolved_border_styles().bottom,
            style.resolved_border_styles().left,
        )
        .with_border_radius(style.border_radius)
        .with_outline(
            style.outline_width,
            style.outline_offset,
            style.outline_style,
            style.resolved_outline_color(),
            style.outline_visible,
        )
        .with_padding(style.padding)
        .with_box_sizing(style.box_sizing)
        .with_width(style.width)
        .with_max_width(style.max_width)
        .with_min_width(style.min_width)
        .with_height(style.height)
        .with_min_height(style.min_height)
        .with_max_height(style.max_height)
        .with_background(style.background_color)
        .with_background_paint(style.background_paint.clone())
        .with_background_layers(
            style.background_paints.clone(),
            style.background_sizes.clone(),
            style.background_positions.clone(),
            style.background_repeats.clone(),
            style.background_origins.clone(),
            style.background_clips.clone(),
        )
        .with_background_blend_modes(style.background_blend_modes.clone())
        .with_clip_path(style.clip_path.clone())
        .with_clip_path_reference_box(style.clip_path_reference_box)
        .with_box_shadows(style.box_shadows.clone())
        .with_paint_filter(style.paint_filter.clone())
        .with_backdrop_filter(style.backdrop_filter.clone())
        .with_will_change_backdrop_root(style.will_change_backdrop_root)
        .with_mask_backdrop_root(style.mask_backdrop_root)
        .with_mix_blend_mode(style.mix_blend_mode)
        .with_isolation(style.isolation)
        .with_opacity(style.opacity)
        .with_transforms(style.transform.clone())
        .with_transform_origin(style.transform_origin)
        .with_overflow_hidden(matches!(style.overflow, OverflowMode::Hidden))
        .with_self_visible(style.visibility.paints())
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
    let Some(container) = container_flowable_with_role(children, style, role) else {
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

fn definition_list_inline_text_role(tag: &str) -> Option<&'static str> {
    match tag {
        "dt" => Some("Lbl"),
        "dd" => Some("LBody"),
        _ => None,
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
    let mut out: Vec<Box<dyn Flowable>> = Vec::new();
    let mut inline_group: Vec<(Box<dyn Flowable>, VerticalAlign)> = Vec::new();

    for item in items {
        match item {
            LayoutItem::Inline {
                flowable, valign, ..
            } => inline_group.push((flowable, valign)),
            LayoutItem::Block { flowable, .. } => {
                if !inline_group.is_empty() {
                    out.push(Box::new(InlineBlockLayoutFlowable::new_pt(
                        inline_group,
                        Pt::ZERO,
                        forced_line_height,
                    )));
                    inline_group = Vec::new();
                }
                out.push(flowable);
            }
        }
    }

    if !inline_group.is_empty() {
        out.push(Box::new(InlineBlockLayoutFlowable::new_pt(
            inline_group,
            Pt::ZERO,
            forced_line_height,
        )));
    }

    out
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
    .with_pagination(style.pagination);
    vec![LayoutItem::Block {
        flowable: Box::new(rel) as Box<dyn Flowable>,
        flex_grow: style.flex_grow,
        flex_shrink: style.flex_shrink,
        width_spec: flex_item_basis(style),
        order: 0,
    }]
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
) -> Vec<LayoutItem> {
    fn align_self_override(mode: AlignSelfMode) -> Option<AlignItems> {
        match mode {
            AlignSelfMode::Auto => None,
            AlignSelfMode::FlexEnd => Some(AlignItems::FlexEnd),
            AlignSelfMode::Center => Some(AlignItems::Center),
            AlignSelfMode::Stretch => Some(AlignItems::Stretch),
            AlignSelfMode::FlexStart => Some(AlignItems::FlexStart),
        }
    }

    let is_grid_like = matches!(style.display, DisplayMode::Grid | DisplayMode::InlineGrid);
    let grid_child_hint = if is_grid_like {
        node.children()
            .filter(|child| child.as_element().is_some())
            .count()
            .max(1)
    } else {
        0
    };
    let grid_track_count = if is_grid_like {
        resolve_grid_track_count(style, grid_child_hint)
    } else {
        0
    };
    let grid_basis = if is_grid_like && grid_track_count > 0 {
        Some(grid_track_basis(grid_track_count, style.gap))
    } else {
        None
    };

    let mut items_with_order: Vec<(
        i32,
        usize,
        Box<dyn Flowable>,
        f32,
        f32,
        Option<LengthSpec>,
        Option<AlignItems>,
    )> = Vec::new();
    let mut grid_auto_slot = 0usize;
    let mut grid_occupied_slots: std::collections::HashSet<usize> =
        std::collections::HashSet::new();
    let mut report = report;

    for (child_idx, child) in node.children().enumerate() {
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
        let grow = child_items
            .iter()
            .map(|it| it.flex_grow())
            .fold(0.0, f32::max);
        let shrink = child_items
            .iter()
            .map(|it| it.flex_shrink())
            .fold(1.0, f32::max);
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
        let boxed: Box<dyn Flowable> = if flowables.len() == 1 {
            flowables.into_iter().next().unwrap()
        } else {
            Box::new(
                ContainerFlowable::new_pt(flowables, style.font_size, style.root_font_size)
                    .with_self_visible(style.visibility.paints()),
            )
        };
        let effective_width_spec = width_spec.or(grid_basis);
        let effective_grow = if is_grid_like { 0.0 } else { grow };
        let effective_shrink = if is_grid_like { 1.0 } else { shrink };
        let child_style = child.as_element().map(|el| {
            let child_info = element_info(&child, resolver.has_sibling_selectors());
            let inline_style = el.attributes.borrow().get("style").map(|s| s.to_string());
            resolver.compute_style(&child_info, style, inline_style.as_deref(), ancestors)
        });
        let align_self = child_style
            .as_ref()
            .and_then(|child_style| align_self_override(child_style.align_self));
        let effective_order = if is_grid_like && grid_track_count > 0 {
            grid_item_order_slot(
                grid_track_count,
                child_style.as_ref(),
                &mut grid_auto_slot,
                &mut grid_occupied_slots,
            )
        } else {
            order
        };

        items_with_order.push((
            effective_order,
            child_idx,
            boxed,
            effective_grow,
            effective_shrink,
            effective_width_spec,
            align_self,
        ));
    }

    items_with_order.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let items: Vec<(
        Box<dyn Flowable>,
        f32,
        f32,
        Option<LengthSpec>,
        Option<AlignItems>,
    )> = if is_grid_like && grid_track_count > 0 {
        let mut padded_items: Vec<(
            Box<dyn Flowable>,
            f32,
            f32,
            Option<LengthSpec>,
            Option<AlignItems>,
        )> = Vec::new();
        let max_slot = items_with_order
            .iter()
            .map(|(slot, _, _, _, _, _, _)| *slot)
            .max()
            .unwrap_or(-1)
            .max(0);
        let mut iter = items_with_order.into_iter().peekable();
        for slot in 0..=max_slot {
            let mut placed = false;
            loop {
                let should_take = iter
                    .peek()
                    .map(|(item_slot, _, _, _, _, _, _)| *item_slot == slot)
                    .unwrap_or(false);
                if !should_take {
                    break;
                }
                if let Some((_, _, boxed, grow, shrink, width_spec, align_self)) = iter.next() {
                    padded_items.push((boxed, grow, shrink, width_spec, align_self));
                    placed = true;
                }
            }
            if !placed {
                padded_items.push((
                    Box::new(Spacer::new_pt(Pt::ZERO)) as Box<dyn Flowable>,
                    0.0,
                    1.0,
                    grid_basis,
                    None,
                ));
            }
        }
        while let Some((_, _, boxed, grow, shrink, width_spec, align_self)) = iter.next() {
            padded_items.push((boxed, grow, shrink, width_spec, align_self));
        }
        padded_items
    } else {
        items_with_order
            .into_iter()
            .map(|(_, _, boxed, grow, shrink, width_spec, align_self)| {
                (boxed, grow, shrink, width_spec, align_self)
            })
            .collect()
    };
    let grid_wrap = is_grid_like && grid_track_count > 0 && items.len() > grid_track_count;

    let dir = if is_grid_like {
        FlexDirection::Row
    } else {
        match style.flex_direction {
            FlexDirectionMode::Column => FlexDirection::Column,
            _ => FlexDirection::Row,
        }
    };
    let justify = match style.justify_content {
        JustifyContentMode::FlexEnd => JustifyContent::FlexEnd,
        JustifyContentMode::Center => JustifyContent::Center,
        JustifyContentMode::SpaceBetween => JustifyContent::SpaceBetween,
        JustifyContentMode::SpaceAround => JustifyContent::SpaceAround,
        JustifyContentMode::SpaceEvenly => JustifyContent::SpaceEvenly,
        _ => JustifyContent::FlexStart,
    };
    let align = match style.align_items {
        AlignItemsMode::FlexEnd => AlignItems::FlexEnd,
        AlignItemsMode::Center => AlignItems::Center,
        AlignItemsMode::Stretch => AlignItems::Stretch,
        _ => AlignItems::FlexStart,
    };
    let align_content = match style.align_content {
        AlignContentMode::FlexEnd => AlignContent::FlexEnd,
        AlignContentMode::Center => AlignContent::Center,
        AlignContentMode::SpaceBetween => AlignContent::SpaceBetween,
        AlignContentMode::SpaceAround => AlignContent::SpaceAround,
        AlignContentMode::SpaceEvenly => AlignContent::SpaceEvenly,
        _ => AlignContent::FlexStart,
    };

    let flex = FlexFlowable::new_pt(
        items,
        dir,
        justify,
        align,
        align_content,
        style.gap,
        if is_grid_like {
            grid_wrap
        } else {
            matches!(style.flex_wrap, FlexWrapMode::Wrap)
        },
        style.font_size,
        style.root_font_size,
    );

    let container =
        ContainerFlowable::new_pt(vec![Box::new(flex)], style.font_size, style.root_font_size)
            .with_establishes_abs_containing_block(establishes_abs_containing_block(&style))
            .with_margin(style.margin)
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
            .with_border_styles(
                style.resolved_border_styles().top,
                style.resolved_border_styles().right,
                style.resolved_border_styles().bottom,
                style.resolved_border_styles().left,
            )
            .with_border_radius(style.border_radius)
            .with_outline(
                style.outline_width,
                style.outline_offset,
                style.outline_style,
                style.resolved_outline_color(),
                style.outline_visible,
            )
            .with_padding(style.padding)
            .with_box_sizing(style.box_sizing)
            .with_width(style.width)
            .with_max_width(style.max_width)
            .with_min_width(style.min_width)
            .with_height(style.height)
            .with_min_height(style.min_height)
            .with_max_height(style.max_height)
            .with_background(style.background_color)
            .with_background_paint(style.background_paint.clone())
            .with_background_layers(
                style.background_paints.clone(),
                style.background_sizes.clone(),
                style.background_positions.clone(),
                style.background_repeats.clone(),
                style.background_origins.clone(),
                style.background_clips.clone(),
            )
            .with_background_blend_modes(style.background_blend_modes.clone())
            .with_clip_path(style.clip_path.clone())
            .with_clip_path_reference_box(style.clip_path_reference_box)
            .with_box_shadows(style.box_shadows.clone())
            .with_paint_filter(style.paint_filter.clone())
            .with_backdrop_filter(style.backdrop_filter.clone())
            .with_will_change_backdrop_root(style.will_change_backdrop_root)
            .with_mask_backdrop_root(style.mask_backdrop_root)
            .with_mix_blend_mode(style.mix_blend_mode)
            .with_isolation(style.isolation)
            .with_opacity(style.opacity)
            .with_transforms(style.transform.clone())
            .with_transform_origin(style.transform_origin)
            .with_overflow_hidden(matches!(style.overflow, OverflowMode::Hidden))
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
        return None;
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
) -> Vec<LayoutItem> {
    let mut report = report;
    let include_prev_siblings = resolver.has_sibling_selectors();
    let mut table_children: Vec<Box<dyn Flowable>> = Vec::new();
    let mut header_group_flowables: Vec<Box<dyn Flowable>> = Vec::new();
    let mut footer_group_flowables: Vec<Box<dyn Flowable>> = Vec::new();
    let mut top_caption_flowables: Vec<Box<dyn Flowable>> = Vec::new();
    let mut bottom_caption_flowables: Vec<Box<dyn Flowable>> = Vec::new();
    let mut anon_cells: Vec<(NodeRef, ComputedStyle)> = Vec::new();
    let collapsed_columns = collect_css_table_collapsed_columns(
        node,
        resolver,
        style,
        ancestors,
        include_prev_siblings,
    );

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
            continue;
        }

        if matches!(child_style.display, DisplayMode::TableCaption) {
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

    let Some(table_flowable) = container_flowable_with_role(table_items, style, Some("Table"))
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
    if let Some(columns) = style.grid_columns {
        return columns.max(1);
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
        LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial => {}
    }

    LengthSpec::Calc(calc)
}

fn grid_explicit_slot(
    track_count: usize,
    row_start: Option<usize>,
    column_start: Option<usize>,
) -> Option<usize> {
    if row_start.is_none() && column_start.is_none() {
        return None;
    }
    let columns = track_count.max(1);
    let row = row_start.unwrap_or(1).saturating_sub(1);
    let column = column_start.unwrap_or(1).saturating_sub(1);
    Some(row.saturating_mul(columns).saturating_add(column))
}

fn grid_item_order_slot(
    track_count: usize,
    child_style: Option<&ComputedStyle>,
    auto_slot: &mut usize,
    occupied_slots: &mut std::collections::HashSet<usize>,
) -> i32 {
    while occupied_slots.contains(auto_slot) {
        *auto_slot = auto_slot.saturating_add(1);
    }

    let auto_row = auto_slot
        .saturating_div(track_count.max(1))
        .saturating_add(1);
    let auto_column = auto_slot
        .checked_rem(track_count.max(1))
        .unwrap_or(0)
        .saturating_add(1);

    let mut assigned_slot = child_style
        .and_then(|style| {
            if style.grid_row_start.is_some() || style.grid_column_start.is_some() {
                let row_start = style.grid_row_start.or(Some(auto_row));
                let column_start = style.grid_column_start.or(Some(auto_column));
                grid_explicit_slot(track_count, row_start, column_start)
            } else {
                None
            }
        })
        .unwrap_or(*auto_slot);

    while occupied_slots.contains(&assigned_slot) {
        assigned_slot = assigned_slot.saturating_add(1);
    }
    occupied_slots.insert(assigned_slot);

    *auto_slot = assigned_slot.saturating_add(1);
    while occupied_slots.contains(auto_slot) {
        *auto_slot = auto_slot.saturating_add(1);
    }

    assigned_slot.min(i32::MAX as usize) as i32
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
    let mut header_rows: Vec<Vec<TableCell>> = Vec::new();
    let mut body_rows: Vec<Vec<TableCell>> = Vec::new();
    let mut body_row_meta: Vec<Vec<(String, String)>> = Vec::new();
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
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial => true,
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
        in_thead: bool,
        in_explicit_row_group: bool,
        group_row_index: usize,
        group_row_count: usize,
    }

    fn collect_rows_from_group(group: &NodeRef, in_thead: bool, out: &mut Vec<TableRowInput>) {
        let row_nodes: Vec<NodeRef> = group
            .children()
            .filter(|child| {
                child
                    .as_element()
                    .map(|el| el.name.local.as_ref() == "tr")
                    .unwrap_or(false)
            })
            .collect();
        let group_row_count = row_nodes.len().max(1);
        for (idx, child) in row_nodes.into_iter().enumerate() {
            out.push(TableRowInput {
                node: child,
                in_thead,
                in_explicit_row_group: true,
                group_row_index: idx + 1,
                group_row_count,
            });
        }
    }

    fn collect_rows(table: &NodeRef, out: &mut Vec<TableRowInput>) {
        let direct_row_count = table
            .children()
            .filter(|child| {
                child
                    .as_element()
                    .map(|el| el.name.local.as_ref() == "tr")
                    .unwrap_or(false)
            })
            .count()
            .max(1);
        let mut direct_row_index = 0usize;
        for child in table.children() {
            let Some(el) = child.as_element() else {
                continue;
            };
            match el.name.local.as_ref() {
                "thead" => collect_rows_from_group(&child, true, out),
                "tbody" | "tfoot" => collect_rows_from_group(&child, false, out),
                "tr" => {
                    direct_row_index += 1;
                    out.push(TableRowInput {
                        node: child,
                        in_thead: false,
                        in_explicit_row_group: false,
                        group_row_index: direct_row_index,
                        group_row_count: direct_row_count,
                    });
                }
                _ => {}
            }
        }
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
        Vec<bool>,
    ) {
        let column_nodes: Vec<NodeRef> = table
            .children()
            .filter(|child| {
                child
                    .as_element()
                    .map(|el| matches!(el.name.local.as_ref(), "col" | "colgroup"))
                    .unwrap_or(false)
            })
            .collect();
        let child_count = column_nodes.len().max(1);
        let mut hints: Vec<Option<TableColumnWidthHint>> = Vec::new();
        let mut borders: Vec<Option<TableColumnBorder>> = Vec::new();
        let mut group_borders: Vec<Option<TableColumnGroupBorder>> = Vec::new();
        let mut collapsed_columns: Vec<bool> = Vec::new();
        let mut prev_infos: Vec<ElementInfo> = Vec::new();

        for (child_index, child) in column_nodes.iter().enumerate() {
            let Some(el) = child.as_element() else {
                continue;
            };
            let tag = el.name.local.as_ref();
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

            match tag {
                "col" => {
                    let span = span_attr(child);
                    append_column_hint(&mut hints, column_width_hint_from_style(&computed), span);
                    append_column_border(&mut borders, column_border_from_style(&computed), span);
                    append_column_group_border(&mut group_borders, None, span, 0, span);
                    append_column_collapsed(
                        &mut collapsed_columns,
                        matches!(computed.visibility, VisibilityMode::Collapse),
                        span,
                    );
                }
                "colgroup" => {
                    let group_collapsed = matches!(computed.visibility, VisibilityMode::Collapse);
                    ancestors.push(info);
                    let cols: Vec<NodeRef> = child
                        .children()
                        .filter(|col| {
                            col.as_element()
                                .map(|col_el| col_el.name.local.as_ref() == "col")
                                .unwrap_or(false)
                        })
                        .collect();
                    if cols.is_empty() {
                        let span = span_attr(child);
                        append_column_hint(
                            &mut hints,
                            column_width_hint_from_style(&computed),
                            span,
                        );
                        append_column_border(&mut borders, None, span);
                        append_column_group_border(
                            &mut group_borders,
                            column_border_from_style(&computed),
                            span,
                            0,
                            span,
                        );
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
                            let base_col_info = element_info_basic(
                                col,
                                col_index + 1,
                                col_count,
                                false,
                                Vec::new(),
                            );
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
                _ => {}
            }
        }

        (hints, borders, group_borders, collapsed_columns)
    }

    let mut rows: Vec<TableRowInput> = Vec::new();
    collect_rows(node, &mut rows);
    if let Some(logger) = resolver.debug_logger() {
        let header_count = rows.iter().filter(|row| row.in_thead).count();
        let body_count = rows.len().saturating_sub(header_count);
        let json = format!(
            "{{\"type\":\"table.rows\",\"total\":{},\"header\":{},\"body\":{}}}",
            rows.len(),
            header_count,
            body_count
        );
        logger.log_json(&json);
    }
    let include_prev_siblings = resolver.has_sibling_selectors();
    let (column_width_hints, column_borders, column_group_borders, collapsed_columns) =
        collect_column_metadata(node, style, resolver, ancestors, include_prev_siblings);
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

    let header_count = rows.iter().filter(|row| row.in_thead).count();
    let body_count = rows.len().saturating_sub(header_count);
    let mut prev_row_infos: Vec<ElementInfo> = Vec::new();
    let mut header_index = 0usize;
    let mut body_index = 0usize;

    for row_input in rows {
        let row = row_input.node;
        let is_header = row_input.in_thead;
        let in_explicit_row_group = row_input.in_explicit_row_group;
        let row_group_starts = row_input.group_row_index == 1;
        let row_group_ends = row_input.group_row_index == row_input.group_row_count;
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
            let tag = parent_el.name.local.as_ref().to_ascii_lowercase();
            match tag.as_str() {
                "thead" | "tbody" | "tfoot" => {
                    let info = element_info(&parent, include_prev_siblings);
                    let inline_style = parent_el
                        .attributes
                        .borrow()
                        .get("style")
                        .map(|s| s.to_string());
                    Some((info, inline_style))
                }
                "table" => Some((
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
                        child_index: 1,
                        child_count: 1,
                        prev_siblings: Vec::new(),
                    },
                    None,
                )),
                _ => None,
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
        let mut row_info = row
            .as_element()
            .map(|_| {
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
                child_index: row_child_index,
                child_count: row_child_count,
                prev_siblings: Vec::new(),
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
        let row_inline_style = row
            .as_element()
            .and_then(|el| el.attributes.borrow().get("style").map(|s| s.to_string()));
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
        let mut cell_nodes: Vec<NodeRef> = Vec::new();
        for cell_child in row.children() {
            let Some(cell_el) = cell_child.as_element() else {
                continue;
            };
            let tag = cell_el.name.local.as_ref();
            if tag != "th" && tag != "td" {
                continue;
            }
            cell_nodes.push(cell_child);
        }
        let cell_total = cell_nodes.len().max(1);
        let mut prev_cell_infos: Vec<ElementInfo> = Vec::new();
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
            cell_info.apply_computed_container_style(cell_style);

            let has_element_children = cell_child
                .children()
                .any(|child| child.as_element().is_some());
            let mut cell_content: Option<Box<dyn Flowable>> = None;
            let mut cell_text = String::new();
            if has_element_children {
                let before_items = pseudo_items_for(
                    resolver,
                    &cell_info,
                    cell_style,
                    ancestors,
                    counters,
                    font_registry.clone(),
                    report.as_deref_mut(),
                    crate::style::PseudoTarget::Before,
                );
                let after_items = pseudo_items_for(
                    resolver,
                    &cell_info,
                    cell_style,
                    ancestors,
                    counters,
                    font_registry.clone(),
                    report.as_deref_mut(),
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
                        .with_establishes_abs_containing_block(establishes_abs_containing_block(
                            &cell_style,
                        ))
                        .with_self_visible(cell_style.visibility.paints()),
                    ) as Box<dyn Flowable>)
                };
            }

            if cell_content.is_none() {
                let t_cell_text = std::time::Instant::now();
                let text = cell_child.text_contents();
                cell_text_ms += t_cell_text.elapsed().as_secs_f64() * 1000.0;
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    let transformed = apply_text_transform(trimmed, cell_style.text_transform);
                    text_chars = text_chars.saturating_add(transformed.chars().count() as u64);
                    cell_text = transformed;
                }
            }

            let align = text_align_from_style(&cell_style);
            let valign = vertical_align_from_style(&cell_style);

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
            let border = BorderSpec {
                widths: border_widths,
                color: if collapsed_table {
                    cell_style.border_color.unwrap_or(cell_style.color)
                } else {
                    cell_style
                        .border_color
                        .or(row_style.border_color)
                        .unwrap_or(cell_style.color)
                },
            };
            let border_styles = cell_style.resolved_border_styles();
            let hidden_borders = cell_style.border_hidden_sides();

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
                cell_style.padding,
                cell_style.background_color,
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
                .with_row_min_height(row_min_height)
                .with_hide_empty_cells(cell_style.empty_cells_hide);
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
            } else {
                cell
            };
            cells.push(cell);
        }

        ancestors.pop();

        if cells.is_empty() {
            continue;
        }
        if is_header {
            header_rows.push(cells);
        } else {
            body_rows.push(cells);
            body_row_meta.push(row_meta);
        }

        if pushed_section {
            ancestors.pop();
        }
    }

    if body_rows.is_empty() && !header_rows.is_empty() {
        body_rows = header_rows.clone();
        header_rows.clear();
        body_row_meta = vec![Vec::new(); body_rows.len()];
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
        .with_row_backgrounds(false)
        .with_body_row_meta(body_row_meta)
        .with_column_width_hints(column_width_hints)
        .with_column_borders(column_borders)
        .with_column_group_borders(column_group_borders)
        .with_collapsed_columns(collapsed_columns)
        .with_pagination(style.pagination)
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
        child_index,
        child_count,
        prev_siblings,
    }
}

fn element_info_with_context(
    node: &NodeRef,
    child_index: usize,
    child_count: usize,
    is_root: bool,
    include_prev_siblings: bool,
) -> ElementInfo {
    let mut prev_siblings: Vec<ElementInfo> = Vec::new();
    if include_prev_siblings {
        if let Some(parent) = node.parent() {
            let mut siblings: Vec<NodeRef> = Vec::new();
            let mut seen = 0usize;
            for sibling in parent.children() {
                if sibling.as_element().is_none() {
                    continue;
                }
                seen += 1;
                if seen >= child_index {
                    break;
                }
                siblings.push(sibling);
            }
            if !siblings.is_empty() {
                prev_siblings = siblings
                    .iter()
                    .enumerate()
                    .map(|(idx, sibling)| {
                        element_info_basic(sibling, idx + 1, child_count, false, Vec::new())
                    })
                    .collect();
            }
        }
    }

    element_info_basic(node, child_index, child_count, is_root, prev_siblings)
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
                out.push('\n');
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
            return text.replace("\r\n", "\n").replace('\r', "\n");
        }
        WhiteSpaceMode::PreserveSpaces => {
            return text
                .replace("\r\n", "\n")
                .replace('\r', "\n")
                .chars()
                .map(|ch| if ch == '\n' || ch == '\t' { ' ' } else { ch })
                .collect();
        }
        _ => {}
    }

    let mut out = String::new();
    let mut in_space = false;
    for ch in text.chars() {
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
    if !matches!(style.overflow, OverflowMode::Hidden) {
        text_style.text_overflow = crate::style::TextOverflowMode::Clip;
    }
    text_style
}

fn no_wrap(style: &ComputedStyle) -> bool {
    matches!(style.text_wrap_mode, TextWrapMode::NoWrap)
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
