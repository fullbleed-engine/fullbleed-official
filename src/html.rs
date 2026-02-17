use crate::flowable::{
    AbsolutePositionedFlowable, AlignItems, BorderRadiusSpec, BorderSpec, CalcLength,
    ContainerFlowable,
    EdgeSizes, FlexDirection, FlexFlowable, ImageFlowable, InlineBlockLayoutFlowable,
    JustifyContent, LengthSpec, ListItemFlowable, MetaFlowable, Paragraph, Spacer, SvgFlowable,
    TableCell, TableFlowable, TextAlign, TextStyle, VerticalAlign,
};
use base64::Engine;
use crate::font::FontRegistry;
use crate::glyph_report::GlyphCoverageReport;
use crate::style::{
    AlignItemsMode, ComputedStyle, DisplayMode, ElementInfo, FlexDirectionMode, FlexWrapMode,
    JustifyContentMode, OverflowMode, PositionMode, StyleResolver, TextAlignMode, WhiteSpaceMode,
};
use crate::types::Pt;
use crate::{BreakAfter, BreakBefore, BreakInside, Flowable};
use kuchiki::traits::TendrilSink;
use kuchiki::{NodeData, NodeRef};
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct HtmlAssetWarning {
    pub kind: String,
    pub message: String,
    pub details: Vec<String>,
}

pub fn scan_html_asset_warnings(html: &str) -> Vec<HtmlAssetWarning> {
    let document = kuchiki::parse_html().one(html);
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
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
) -> Vec<Box<dyn Flowable>> {
    let t_parse = std::time::Instant::now();
    let document = kuchiki::parse_html().one(html);
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

    let mut root_style = base_style.clone();
    if let Ok(html_el) = document.select_first("html") {
        let t_root = std::time::Instant::now();
        let html_node = html_el.as_node();
        let html_element = html_node.as_element().expect("html element");
        let html_info = element_info(html_node, resolver.has_sibling_selectors());
        let inline_style = html_element
            .attributes
            .borrow()
            .get("style")
            .map(|s| s.to_string());
        root_style =
            resolver.compute_style(&html_info, &base_style, inline_style.as_deref(), &ancestors);
        ancestors.push(html_info);
        if let Some(perf_logger) = perf {
            let ms = t_root.elapsed().as_secs_f64() * 1000.0;
            perf_logger.log_span_ms("story.style.root", doc_id, ms);
        }
    }

    let items = if let Ok(body) = document.select_first("body") {
        let body_node = body.as_node();
        let body_element = body_node.as_element().expect("body element");
        let body_info = element_info(body_node, resolver.has_sibling_selectors());
        let inline_style = body_element
            .attributes
            .borrow()
            .get("style")
            .map(|s| s.to_string());
        let t_body = std::time::Instant::now();
        let body_style =
            resolver.compute_style(&body_info, &root_style, inline_style.as_deref(), &ancestors);
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
            font_registry.clone(),
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
            font_registry.clone(),
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
    let document = kuchiki::parse_html().one(html);
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

fn collect_children(
    node: &NodeRef,
    resolver: &StyleResolver,
    parent_style: &ComputedStyle,
    ancestors: &mut Vec<ElementInfo>,
    font_registry: Option<Arc<FontRegistry>>,
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
            font_registry.clone(),
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
    font_registry: Option<Arc<FontRegistry>>,
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
                let text_style = parent_style.to_text_style();
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
                    .with_whitespace(
                        preserve_whitespace(parent_style.white_space),
                        no_wrap(parent_style.white_space),
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
            let info = element_info(node, resolver.has_sibling_selectors());
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
            let node_meta = element
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

            if matches!(style.display, DisplayMode::None) {
                return Vec::new();
            }

            let before_items = pseudo_items_for(
                resolver,
                &info,
                &style,
                ancestors,
                font_registry.clone(),
                report.as_deref_mut(),
                crate::style::PseudoTarget::Before,
            );
            let after_items = pseudo_items_for(
                resolver,
                &info,
                &style,
                ancestors,
                font_registry.clone(),
                report.as_deref_mut(),
                crate::style::PseudoTarget::After,
            );

            // Maintain an ancestor stack instead of cloning it for every element.
            ancestors.push(info.clone());

            // Contents/inline are usually transparent containers in our layout model, except
            // replaced/special inline elements that render atomic content.
            let transparent_inline = matches!(style.display, DisplayMode::Contents | DisplayMode::Inline)
                && !matches!(info.tag.as_str(), "img" | "svg" | "br");
            if transparent_inline {
                let out = collect_children(
                    node,
                    resolver,
                    &style,
                    ancestors,
                    font_registry.clone(),
                    report.as_deref_mut(),
                    svg_form,
                    svg_raster_fallback,
                    perf,
                    doc_id,
                );
                let out = inject_pseudo_items(out, &before_items, &after_items);
                ancestors.pop();
                return out;
            }

            let mut flowables = match info.tag.as_str() {
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
                            font_registry.clone(),
                            report.as_deref_mut(),
                            crate::style::PseudoTarget::Before,
                        );
                        let after = pseudo_text_for(
                            resolver,
                            &info,
                            &style,
                            ancestors,
                            font_registry.clone(),
                            report.as_deref_mut(),
                            crate::style::PseudoTarget::After,
                        );
                        if !before.is_empty() || !after.is_empty() {
                            text = format!("{before}{text}{after}");
                        }
                        if text.is_empty() {
                            Vec::new()
                        } else {
                            let t_transform = std::time::Instant::now();
                            let text = apply_text_transform(&text, style.text_transform);
                            if let Some(perf_logger) = perf {
                                let ms = t_transform.elapsed().as_secs_f64() * 1000.0;
                                perf_logger.log_span_ms("story.text.transform", doc_id, ms);
                            }
                            let text_style = style.to_text_style();
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
                                .with_whitespace(
                                    preserve_whitespace(style.white_space),
                                    no_wrap(style.white_space),
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
                        let children = collect_children(
                            node,
                            resolver,
                            &style,
                            ancestors,
                            font_registry.clone(),
                            report.as_deref_mut(),
                            svg_form,
                            svg_raster_fallback,
                            perf,
                            doc_id,
                        );
                        let children = inject_pseudo_items(children, &before_items, &after_items);
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
                        font_registry.clone(),
                        report.as_deref_mut(),
                        crate::style::PseudoTarget::Before,
                    );
                    let after = pseudo_text_for(
                        resolver,
                        &info,
                        &style,
                        ancestors,
                        font_registry.clone(),
                        report.as_deref_mut(),
                        crate::style::PseudoTarget::After,
                    );
                    if !before.is_empty() || !after.is_empty() {
                        text = format!("{before}{text}{after}");
                    }
                    if text.is_empty() {
                        Vec::new()
                    } else {
                        let t_transform = std::time::Instant::now();
                        let text = apply_text_transform(&text, style.text_transform);
                        if let Some(perf_logger) = perf {
                            let ms = t_transform.elapsed().as_secs_f64() * 1000.0;
                            perf_logger.log_span_ms("story.text.transform", doc_id, ms);
                        }
                        let text_style = style.to_text_style();
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
                    let inline_width_height = inline_dimensions(inline_style.as_deref());
                    let css_width = if matches!(
                        style.width,
                        LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial
                    ) {
                        None
                    } else {
                        let resolved =
                            style
                                .width
                                .resolve_width(Pt::from_f32(300.0), style.font_size, style.root_font_size);
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
                    let width = inline_width_height
                        .0
                        .or_else(|| parse_dimension(attrs.get("width")))
                        .or(css_width)
                        .unwrap_or_else(|| style.font_size * 4.0);
                    let height = inline_width_height
                        .1
                        .or_else(|| parse_dimension(attrs.get("height")))
                        .or(css_height)
                        .unwrap_or_else(|| style.font_size * 3.0);
                    let src = attrs.get("src").unwrap_or("image");
                    let alt = attrs
                        .get("alt")
                        .or_else(|| attrs.get("aria-label"))
                        .or_else(|| attrs.get("title"))
                        .map(|s| s.to_string());
                    let width_spec = flex_item_basis(&style);
                    if let Some(xml) = load_svg_xml_from_image_source(src) {
                        if svg_raster_fallback && crate::svg::svg_needs_raster_fallback(&xml) {
                            if let Some(data_uri) =
                                crate::svg::rasterize_svg_to_data_uri(&xml, width, height)
                            {
                                let image = ImageFlowable::new_pt(width, height, data_uri)
                                    .with_pagination(style.pagination)
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
                                .with_tag_role("Figure")
                                .with_alt(alt);
                            if let Some(perf_logger) = perf {
                                let ms = t_svg.elapsed().as_secs_f64() * 1000.0;
                                perf_logger.log_span_ms("svg.compile", None, ms);
                                perf_logger
                                    .log_counts("svg.compile", None, &[("bytes", xml_len)]);
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
                        let image = ImageFlowable::new_pt(width, height, src)
                            .with_pagination(style.pagination)
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
                                .with_pagination(style.pagination)
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
                    } else {
                        let xml_len = xml.len() as u64;
                        let t_svg = std::time::Instant::now();
                        let svg = SvgFlowable::new_pt(width, height, xml)
                            .with_pagination(style.pagination)
                            .with_form_enabled(svg_form)
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
                    let mut caption_flowables: Vec<Box<dyn Flowable>> = Vec::new();
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
                        let caption_text_style = caption_style.to_text_style();
                        report_missing_glyphs(
                            report.as_deref_mut(),
                            font_registry.as_deref(),
                            &caption_text_style,
                            &caption_text,
                        );
                        let paragraph = Paragraph::new(caption_text)
                            .with_style(caption_text_style)
                            .with_align(text_align_from_style(&caption_style))
                            .with_whitespace(
                                preserve_whitespace(caption_style.white_space),
                                no_wrap(caption_style.white_space),
                            )
                            .with_pagination(caption_style.pagination)
                            .with_font_registry(font_registry.clone())
                            .with_tag_role("Caption");
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
                            caption_flowables.push(flowable);
                        }
                    }

                    let table = table_flowable(
                        node,
                        &style,
                        resolver,
                        ancestors,
                        font_registry.clone(),
                        report.as_deref_mut(),
                        svg_form,
                        svg_raster_fallback,
                        perf,
                        doc_id,
                    )
                    .with_tag_role("Table")
                    .with_border_collapse(style.border_collapse)
                    .with_border_spacing(style.border_spacing)
                    .with_font_metrics(style.font_size, style.root_font_size);

                    let mut table_children: Vec<Box<dyn Flowable>> = Vec::new();
                    if matches!(style.caption_side, crate::style::CaptionSideMode::Top) {
                        table_children.extend(caption_flowables);
                        table_children.push(Box::new(table) as Box<dyn Flowable>);
                    } else {
                        table_children.push(Box::new(table) as Box<dyn Flowable>);
                        table_children.extend(caption_flowables);
                    }

                    let container = ContainerFlowable::new_pt(
                        table_children,
                        style.font_size,
                        style.root_font_size,
                    )
                    .with_margin(style.margin)
                    .with_border(
                        style.border_width,
                        style.border_color.unwrap_or(style.color),
                    )
                    .with_border_radius(style.border_radius)
                    .with_padding(style.padding)
                    .with_box_sizing(style.box_sizing)
                    .with_width(style.width)
                    .with_max_width(style.max_width)
                    .with_height(style.height)
                    .with_background(style.background_color)
                    .with_background_paint(style.background_paint.clone())
                    .with_box_shadow(style.box_shadow.clone())
                    .with_overflow_hidden(matches!(style.overflow, OverflowMode::Hidden))
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
                        font_registry.clone(),
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
                        Vec::new()
                    } else {
                        let text = apply_text_transform(&text, style.text_transform);
                        let label = format!("- {}", text);
                        let text_style = style.to_text_style();
                        report_missing_glyphs(
                            report.as_deref_mut(),
                            font_registry.as_deref(),
                            &text_style,
                            &label,
                        );
                        let paragraph = Paragraph::new(label)
                            .with_style(text_style)
                            .with_align(text_align_from_style(&style))
                            .with_whitespace(
                                preserve_whitespace(style.white_space),
                                no_wrap(style.white_space),
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
                "div" | "span" | "section" | "article" | "header" | "footer" | "aside" | "nav"
                | "main" | "blockquote" | "dl" | "dt" | "dd" => {
                    if is_table_container_display(style.display) {
                        table_container_flowables(
                            node,
                            resolver,
                            &style,
                            ancestors,
                            font_registry.clone(),
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
                            font_registry.clone(),
                            report.as_deref_mut(),
                            svg_form,
                            svg_raster_fallback,
                            perf,
                            doc_id,
                        )
                    } else if matches!(style.display, DisplayMode::Block | DisplayMode::InlineBlock)
                        && inline_children_only(node, resolver, &style, ancestors)
                    {
                        let ancestors_no_self = &ancestors[..ancestors.len().saturating_sub(1)];
                        let mut text = extract_text(node, style.white_space);
                        let before = pseudo_text_for(
                            resolver,
                            &info,
                            &style,
                            ancestors_no_self,
                            font_registry.clone(),
                            report.as_deref_mut(),
                            crate::style::PseudoTarget::Before,
                        );
                        let after = pseudo_text_for(
                            resolver,
                            &info,
                            &style,
                            ancestors_no_self,
                            font_registry.clone(),
                            report.as_deref_mut(),
                            crate::style::PseudoTarget::After,
                        );
                        if !before.is_empty() || !after.is_empty() {
                            text = format!("{before}{text}{after}");
                        }
                        if text.is_empty() {
                            Vec::new()
                        } else {
                            let text = apply_text_transform(&text, style.text_transform);
                            let text_style = style.to_text_style();
                            report_missing_glyphs(
                                report.as_deref_mut(),
                                font_registry.as_deref(),
                                &text_style,
                                &text,
                            );
                            let paragraph = Paragraph::new(text)
                                .with_style(text_style)
                                .with_align(text_align_from_style(&style))
                                .with_whitespace(
                                    preserve_whitespace(style.white_space),
                                    no_wrap(style.white_space),
                                )
                                .with_pagination(style.pagination)
                                .with_font_registry(font_registry.clone());
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
                        let children = collect_children(
                            node,
                            resolver,
                            &style,
                            ancestors,
                            font_registry.clone(),
                            report.as_deref_mut(),
                            svg_form,
                            svg_raster_fallback,
                            perf,
                            doc_id,
                        );
                        let children = inject_pseudo_items(children, &before_items, &after_items);
                        container_flowables(children, &style)
                    }
                }
                _ => {
                    if is_table_container_display(style.display) {
                        table_container_flowables(
                            node,
                            resolver,
                            &style,
                            ancestors,
                            font_registry.clone(),
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
                            font_registry.clone(),
                            report.as_deref_mut(),
                            svg_form,
                            svg_raster_fallback,
                            perf,
                            doc_id,
                        );
                        inject_pseudo_items(children, &before_items, &after_items)
                    }
                }
            };

            // Preserve metadata-only nodes (for example <div data-fb="..."></div>) so
            // feature flags can be emitted even when the element has no visual content.
            // Use a tiny spacer to ensure the flowable is drawable; zero-height carriers can
            // be skipped in layout paths and lose metadata emission.
            if flowables.is_empty() && !node_meta.is_empty() {
                let carrier =
                    Spacer::new_pt(Pt::from_f32(0.01)).with_pagination(style.pagination);
                flowables.push(LayoutItem::Block {
                    flowable: Box::new(carrier) as Box<dyn Flowable>,
                    flex_grow: style.flex_grow,
                    flex_shrink: style.flex_shrink,
                    width_spec: flex_item_basis(&style),
                    order: style.order,
                });
            }

            if matches!(style.position, PositionMode::Absolute) {
                flowables = wrap_absolute(flowables, &style);
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
                let valign = match style.vertical_align {
                    crate::style::VerticalAlignMode::Middle => VerticalAlign::Middle,
                    crate::style::VerticalAlignMode::Bottom => VerticalAlign::Bottom,
                    _ => VerticalAlign::Top,
                };
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
            items
        }
        _ => Vec::new(),
    }
}

fn serialize_svg_node(node: &NodeRef) -> String {
    // kuchiki parses HTML, and `NodeRef::to_string()` produces HTML serialization which is not
    // necessarily well-formed XML (SVG void-ish children, attribute quoting, etc). roxmltree
    // expects well-formed XML, so we do a minimal XML serialization for the SVG subtree.
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

fn text_align_from_style(style: &ComputedStyle) -> TextAlign {
    match style.text_align {
        TextAlignMode::Center => TextAlign::Center,
        TextAlignMode::Right => TextAlign::Right,
        TextAlignMode::Left => TextAlign::Left,
    }
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

fn pseudo_content_items(
    style: &ComputedStyle,
    font_registry: Option<Arc<FontRegistry>>,
    report: Option<&mut GlyphCoverageReport>,
) -> Vec<LayoutItem> {
    let Some(content) = style.content.as_deref() else {
        return Vec::new();
    };
    if content.is_empty() {
        return Vec::new();
    }
    let text = apply_text_transform(content, style.text_transform);
    let text_style = style.to_text_style();
    report_missing_glyphs(report, font_registry.as_deref(), &text_style, &text);
    let paragraph = Paragraph::new(text)
        .with_style(text_style)
        .with_align(text_align_from_style(style))
        .with_whitespace(
            preserve_whitespace(style.white_space),
            no_wrap(style.white_space),
        )
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
        let valign = match style.vertical_align {
            crate::style::VerticalAlignMode::Middle => VerticalAlign::Middle,
            crate::style::VerticalAlignMode::Bottom => VerticalAlign::Bottom,
            _ => VerticalAlign::Top,
        };
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
    font_registry: Option<Arc<FontRegistry>>,
    report: Option<&mut GlyphCoverageReport>,
    pseudo: crate::style::PseudoTarget,
) -> Vec<LayoutItem> {
    let Some(pseudo_style) = resolver.compute_pseudo_style(info, style, ancestors, pseudo) else {
        return Vec::new();
    };
    pseudo_content_items(&pseudo_style, font_registry, report)
}

fn pseudo_text_for(
    resolver: &StyleResolver,
    info: &ElementInfo,
    style: &ComputedStyle,
    ancestors: &[ElementInfo],
    font_registry: Option<Arc<FontRegistry>>,
    report: Option<&mut GlyphCoverageReport>,
    pseudo: crate::style::PseudoTarget,
) -> String {
    let Some(pseudo_style) = resolver.compute_pseudo_style(info, style, ancestors, pseudo) else {
        return String::new();
    };
    let Some(content) = pseudo_style.content.clone() else {
        return String::new();
    };
    let text = apply_text_transform(&content, pseudo_style.text_transform);
    let text_style = pseudo_style.to_text_style();
    report_missing_glyphs(report, font_registry.as_deref(), &text_style, &text);
    text
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
        let doc = kuchiki::parse_html().one(html);
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
    fn inline_children_only_excludes_img_and_svg_replaced_content() {
        let doc = kuchiki::parse_html().one(
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
        let doc = kuchiki::parse_html().one(
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
        let (w, h) =
            resolve_svg_dimensions(None, None, None, None, Some("0 0 220 120"), &style);
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
        let payload = base64::engine::general_purpose::STANDARD.encode(xml.as_bytes());
        let uri = format!("data:image/svg+xml;base64,{payload}");
        let decoded = load_svg_xml_from_image_source(&uri).expect("svg xml from data uri");
        assert!(
            decoded.contains("<svg") && decoded.contains("<rect"),
            "expected decoded inline SVG xml, got: {decoded}"
        );
    }

    #[test]
    fn non_svg_data_uri_image_source_is_not_treated_as_svg() {
        let payload = base64::engine::general_purpose::STANDARD.encode([0u8, 1u8, 2u8]);
        let uri = format!("data:image/png;base64,{payload}");
        assert!(
            load_svg_xml_from_image_source(&uri).is_none(),
            "png data uri must stay on raster image path"
        );
    }
}

fn list_flowables(
    node: &NodeRef,
    resolver: &StyleResolver,
    parent_style: &ComputedStyle,
    ancestors: &[ElementInfo],
    font_registry: Option<Arc<FontRegistry>>,
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
    let mut index = 1usize;

    for child in node.children() {
        if let Some(element) = child.as_element() {
            if element.name.local.as_ref() != "li" {
                continue;
            }
            let info = element_info(&child, resolver.has_sibling_selectors());
            let inline_style = element
                .attributes
                .borrow()
                .get("style")
                .map(|s| s.to_string());
            let style =
                resolver.compute_style(&info, parent_style, inline_style.as_deref(), ancestors);
            let is_inline = matches!(
                style.display,
                DisplayMode::Inline
                    | DisplayMode::InlineBlock
                    | DisplayMode::InlineTable
                    | DisplayMode::InlineFlex
                    | DisplayMode::InlineGrid
            );
            let show_marker =
                matches!(style.list_style_type, crate::style::ListStyleTypeMode::Auto)
                    && !is_inline;

            let mut li_ancestors = ancestors.to_vec();
            li_ancestors.push(info.clone());
            let mut li_body_items: Vec<LayoutItem> = Vec::new();
            for li_child in child.children() {
                li_body_items.extend(node_to_flowables(
                    &li_child,
                    resolver,
                    &style,
                    &mut li_ancestors,
                    font_registry.clone(),
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
            let li_body: Box<dyn Flowable> = Box::new(
                ContainerFlowable::new_pt(li_body_flowables, style.font_size, style.root_font_size)
                    .with_pagination(style.pagination)
                    .with_tag_role("LBody"),
            );

            let li_flowable: Box<dyn Flowable> = if show_marker {
                let prefix = if ordered {
                    let label = format!("{}. ", index);
                    index += 1;
                    label
                } else {
                    "\u{2022} ".to_string()
                };
                let text_style = style.to_text_style();
                report_missing_glyphs(
                    report.as_deref_mut(),
                    font_registry.as_deref(),
                    &text_style,
                    &prefix,
                );
                let label_para = Paragraph::new(prefix)
                    .with_style(text_style)
                    .with_align(text_align_from_style(&style))
                    .with_whitespace(
                        preserve_whitespace(style.white_space),
                        no_wrap(style.white_space),
                    )
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
                    let valign = match style.vertical_align {
                        crate::style::VerticalAlignMode::Middle => VerticalAlign::Middle,
                        crate::style::VerticalAlignMode::Bottom => VerticalAlign::Bottom,
                        _ => VerticalAlign::Top,
                    };
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

fn container_flowables(children: Vec<LayoutItem>, style: &ComputedStyle) -> Vec<LayoutItem> {
    container_flowables_with_role(children, style, None)
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
        || style.box_shadow.is_some()
        || style.border_radius != BorderRadiusSpec::zero()
        || style.border_width != EdgeSizes::zero();

    if children.is_empty() && !has_box {
        // Preserve page-break semantics even for empty elements.
        if style.pagination.break_before != BreakBefore::Auto
            || style.pagination.break_after != BreakAfter::Auto
        {
            let mut container =
                ContainerFlowable::new_pt(Vec::new(), style.font_size, style.root_font_size)
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
        .with_margin(style.margin)
        .with_border(
            style.border_width,
            style.border_color.unwrap_or(style.color),
        )
        .with_border_radius(style.border_radius)
        .with_padding(style.padding)
        .with_box_sizing(style.box_sizing)
        .with_width(style.width)
        .with_max_width(style.max_width)
        .with_height(style.height)
        .with_background(style.background_color)
        .with_background_paint(style.background_paint.clone())
        .with_box_shadow(style.box_shadow.clone())
        .with_overflow_hidden(matches!(style.overflow, OverflowMode::Hidden))
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
        Box::new(ContainerFlowable::new_pt(
            flowables,
            style.font_size,
            style.root_font_size,
        ))
    };
    let abs = AbsolutePositionedFlowable::new_pt(
        boxed,
        style.inset_left,
        style.inset_top,
        style.inset_right,
        style.inset_bottom,
        style.z_index,
        style.font_size,
        style.root_font_size,
    )
    .with_pagination(style.pagination);
    vec![LayoutItem::Block {
        flowable: Box::new(abs) as Box<dyn Flowable>,
        flex_grow: style.flex_grow,
        flex_shrink: style.flex_shrink,
        width_spec: flex_item_basis(&style),
        order: 0,
    }]
}

fn flex_container_flowables(
    node: &NodeRef,
    resolver: &StyleResolver,
    style: &ComputedStyle,
    ancestors: &mut Vec<ElementInfo>,
    font_registry: Option<Arc<FontRegistry>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
) -> Vec<LayoutItem> {
    let is_grid_like = matches!(style.display, DisplayMode::Grid | DisplayMode::InlineGrid);
    let grid_track_count = style.grid_columns.unwrap_or(0);
    let grid_basis = if is_grid_like && grid_track_count > 0 {
        Some(grid_track_basis(grid_track_count, style.gap))
    } else {
        None
    };

    let mut items_with_order: Vec<(i32, usize, Box<dyn Flowable>, f32, f32, Option<LengthSpec>)> =
        Vec::new();
    let mut report = report;

    for (child_idx, child) in node.children().enumerate() {
        let child_items = node_to_flowables(
            &child,
            resolver,
            style,
            ancestors,
            font_registry.clone(),
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
            Box::new(ContainerFlowable::new_pt(
                flowables,
                style.font_size,
                style.root_font_size,
            ))
        };
        let effective_width_spec = width_spec.or(grid_basis);
        let effective_grow = if is_grid_like { 0.0 } else { grow };
        let effective_shrink = if is_grid_like { 1.0 } else { shrink };
        items_with_order.push((
            order,
            child_idx,
            boxed,
            effective_grow,
            effective_shrink,
            effective_width_spec,
        ));
    }

    items_with_order.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let items: Vec<(Box<dyn Flowable>, f32, f32, Option<LengthSpec>)> = items_with_order
        .into_iter()
        .map(|(_, _, boxed, grow, shrink, width_spec)| (boxed, grow, shrink, width_spec))
        .collect();
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
        _ => JustifyContent::FlexStart,
    };
    let align = match style.align_items {
        AlignItemsMode::FlexEnd => AlignItems::FlexEnd,
        AlignItemsMode::Center => AlignItems::Center,
        AlignItemsMode::Stretch => AlignItems::Stretch,
        _ => AlignItems::FlexStart,
    };

    let flex = FlexFlowable::new_pt(
        items,
        dir,
        justify,
        align,
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
            .with_margin(style.margin)
            .with_border(
                style.border_width,
                style.border_color.unwrap_or(style.color),
            )
            .with_border_radius(style.border_radius)
            .with_padding(style.padding)
            .with_box_sizing(style.box_sizing)
            .with_width(style.width)
            .with_max_width(style.max_width)
            .with_height(style.height)
            .with_background(style.background_color)
            .with_background_paint(style.background_paint.clone())
            .with_box_shadow(style.box_shadow.clone())
            .with_overflow_hidden(matches!(style.overflow, OverflowMode::Hidden))
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

fn table_group_role(display: DisplayMode) -> &'static str {
    match display {
        DisplayMode::TableHeaderGroup => "THead",
        DisplayMode::TableFooterGroup => "TFoot",
        _ => "TBody",
    }
}

fn table_container_flowables(
    node: &NodeRef,
    resolver: &StyleResolver,
    style: &ComputedStyle,
    ancestors: &[ElementInfo],
    font_registry: Option<Arc<FontRegistry>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
) -> Vec<LayoutItem> {
    let mut report = report;
    let include_prev_siblings = resolver.has_sibling_selectors();
    let mut table_children: Vec<Box<dyn Flowable>> = Vec::new();
    let mut anon_cells: Vec<(NodeRef, ComputedStyle)> = Vec::new();

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

        if matches!(child_style.display, DisplayMode::TableCell) {
            anon_cells.push((child.clone(), child_style));
            continue;
        }

        if let Some(row_flowable) = table_row_flowable_from_cells(
            std::mem::take(&mut anon_cells),
            style,
            resolver,
            ancestors,
            font_registry.clone(),
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
            perf,
            doc_id,
        ) {
            table_children.push(row_flowable);
        }

        if matches!(child_style.display, DisplayMode::TableRow) {
            if let Some(row_flowable) = table_row_flowable_from_node(
                &child,
                &child_style,
                resolver,
                ancestors,
                font_registry.clone(),
                report.as_deref_mut(),
                svg_form,
                svg_raster_fallback,
                perf,
                doc_id,
            ) {
                table_children.push(row_flowable);
            }
            continue;
        }

        if is_table_row_group_display(child_style.display) {
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
                if let Some(row_flowable) = table_row_flowable_from_node(
                    &row_node,
                    &row_style,
                    resolver,
                    ancestors,
                    font_registry.clone(),
                    report.as_deref_mut(),
                    svg_form,
                    svg_raster_fallback,
                    perf,
                    doc_id,
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
                    table_children.push(group_flowable);
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
            font_registry.clone(),
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
        font_registry.clone(),
        report.as_deref_mut(),
        svg_form,
        svg_raster_fallback,
        perf,
        doc_id,
    ) {
        table_children.push(row_flowable);
    }

    let table_items: Vec<LayoutItem> = table_children
        .into_iter()
        .map(|flowable| LayoutItem::Block {
            flowable,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            width_spec: None,
            order: 0,
        })
        .collect();

    let Some(table_flowable) = container_flowable_with_role(table_items, style, Some("Table")) else {
        return Vec::new();
    };

    if matches!(style.display, DisplayMode::InlineTable) {
        let valign = match style.vertical_align {
            crate::style::VerticalAlignMode::Middle => VerticalAlign::Middle,
            crate::style::VerticalAlignMode::Bottom => VerticalAlign::Bottom,
            _ => VerticalAlign::Top,
        };
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
    font_registry: Option<Arc<FontRegistry>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
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
        cells.push((cell_node.clone(), cell_style));
    }
    table_row_flowable_from_cells(
        cells,
        row_style,
        resolver,
        ancestors,
        font_registry,
        report,
        svg_form,
        svg_raster_fallback,
        perf,
        doc_id,
    )
}

fn table_row_flowable_from_cells(
    cells: Vec<(NodeRef, ComputedStyle)>,
    row_style: &ComputedStyle,
    resolver: &StyleResolver,
    ancestors: &[ElementInfo],
    font_registry: Option<Arc<FontRegistry>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&crate::perf::PerfLogger>,
    doc_id: Option<usize>,
) -> Option<Box<dyn Flowable>> {
    if cells.is_empty() {
        return None;
    }
    let mut report = report;
    let cell_count = cells.len().max(1) as f32;
    let mut row_items: Vec<(Box<dyn Flowable>, f32, f32, Option<LengthSpec>)> = Vec::new();

    for (cell_node, cell_style) in cells {
        let mut cell_ancestors = ancestors.to_vec();
        let cell_items = node_to_flowables(
            &cell_node,
            resolver,
            row_style,
            &mut cell_ancestors,
            font_registry.clone(),
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
            perf,
            doc_id,
        );
        let mut cell_flowables = layout_children_to_flowables(cell_items, None);
        let cell_flowable: Box<dyn Flowable> = if cell_flowables.is_empty() {
            Box::new(ContainerFlowable::new_pt(
                Vec::new(),
                cell_style.font_size,
                cell_style.root_font_size,
            ))
        } else if cell_flowables.len() == 1 {
            cell_flowables.remove(0)
        } else {
            Box::new(ContainerFlowable::new_pt(
                cell_flowables,
                cell_style.font_size,
                cell_style.root_font_size,
            ))
        };
        let explicit_width = !matches!(cell_style.width, LengthSpec::Auto);
        let width_spec = if explicit_width {
            Some(cell_style.width)
        } else {
            Some(LengthSpec::Percent(1.0 / cell_count))
        };
        row_items.push((
            cell_flowable,
            if explicit_width { 0.0 } else { 1.0 },
            1.0,
            width_spec,
        ));
    }

    let row_core: Box<dyn Flowable> = Box::new(
        FlexFlowable::new_pt(
            row_items,
            FlexDirection::Row,
            JustifyContent::FlexStart,
            AlignItems::Stretch,
            row_style.gap,
            false,
            row_style.font_size,
            row_style.root_font_size,
        ),
    );
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

fn table_flowable(
    node: &NodeRef,
    style: &ComputedStyle,
    resolver: &StyleResolver,
    ancestors: &mut Vec<ElementInfo>,
    font_registry: Option<Arc<FontRegistry>>,
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

    fn collect_rows_from_group(group: &NodeRef, in_thead: bool, out: &mut Vec<(NodeRef, bool)>) {
        for child in group.children() {
            let Some(el) = child.as_element() else {
                continue;
            };
            if el.name.local.as_ref() == "tr" {
                out.push((child, in_thead));
            }
        }
    }

    fn collect_rows(table: &NodeRef, out: &mut Vec<(NodeRef, bool)>) {
        for child in table.children() {
            let Some(el) = child.as_element() else {
                continue;
            };
            match el.name.local.as_ref() {
                "thead" => collect_rows_from_group(&child, true, out),
                "tbody" | "tfoot" => collect_rows_from_group(&child, false, out),
                "tr" => out.push((child, false)),
                _ => {}
            }
        }
    }

    let mut rows: Vec<(NodeRef, bool)> = Vec::new();
    collect_rows(node, &mut rows);
    if let Some(logger) = resolver.debug_logger() {
        let header_count = rows.iter().filter(|(_, is_header)| *is_header).count();
        let body_count = rows.len().saturating_sub(header_count);
        let json = format!(
            "{{\"type\":\"table.rows\",\"total\":{},\"header\":{},\"body\":{}}}",
            rows.len(),
            header_count,
            body_count
        );
        logger.log_json(&json);
    }

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

    let header_count = rows.iter().filter(|(_, is_header)| *is_header).count();
    let body_count = rows.len().saturating_sub(header_count);
    let include_prev_siblings = resolver.has_sibling_selectors();
    let mut prev_row_infos: Vec<ElementInfo> = Vec::new();
    let mut header_index = 0usize;
    let mut body_index = 0usize;

    for (row, is_header) in rows {
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
        let section_style_owned: Option<ComputedStyle> = if let Some((section, inline_style)) = section_context {
            can_cache_section =
                section.id.is_none() && section.classes.is_empty() && inline_style.is_none();
            let t_section_style = std::time::Instant::now();
            let computed =
                resolver.compute_style(&section, style, inline_style.as_deref(), ancestors);
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
        let row_info = row
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
        let row_min_height =
            resolve_non_auto_height(row_style.height, row_style.font_size, row_style.root_font_size);

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

            let cell_info = {
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

            let has_element_children = cell_child.children().any(|child| child.as_element().is_some());
            let mut cell_content: Option<Box<dyn Flowable>> = None;
            let mut cell_text = String::new();
            if has_element_children {
                let before_items = pseudo_items_for(
                    resolver,
                    &cell_info,
                    cell_style,
                    ancestors,
                    font_registry.clone(),
                    report.as_deref_mut(),
                    crate::style::PseudoTarget::Before,
                );
                let after_items = pseudo_items_for(
                    resolver,
                    &cell_info,
                    cell_style,
                    ancestors,
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
                    font_registry.clone(),
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
                    Some(Box::new(ContainerFlowable::new_pt(
                        cell_flowables,
                        cell_style.font_size,
                        cell_style.root_font_size,
                    )) as Box<dyn Flowable>)
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

            let align = match cell_style.text_align {
                crate::style::TextAlignMode::Center => TextAlign::Center,
                crate::style::TextAlignMode::Right => TextAlign::Right,
                _ => TextAlign::Left,
            };
            let valign = match cell_style.vertical_align {
                crate::style::VerticalAlignMode::Middle => VerticalAlign::Middle,
                crate::style::VerticalAlignMode::Bottom => VerticalAlign::Bottom,
                _ => VerticalAlign::Top,
            };

            let mut border_widths = cell_style.border_width;
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
            let border = BorderSpec {
                widths: border_widths,
                color: cell_style
                    .border_color
                    .or(row_style.border_color)
                    .unwrap_or(cell_style.color),
            };

            let text_style = cell_style.to_text_style();
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
                    Some("Column".to_string())
                } else {
                    None
                },
                col_span,
                cell_style.root_font_size,
                font_registry.clone(),
                preserve_whitespace(cell_style.white_space),
                no_wrap(cell_style.white_space),
            );
            let mut cell = cell.with_row_min_height(row_min_height);
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

    let is_root = node
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

fn parse_data_uri_bytes(uri: &str) -> Option<(String, Vec<u8>)> {
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
        .to_ascii_lowercase();
    let data = if header.contains(";base64") {
        base64::engine::general_purpose::STANDARD
            .decode(payload.as_bytes())
            .ok()?
    } else {
        decode_percent_encoded_bytes(payload)?
    };
    Some((mime, data))
}

fn decode_percent_encoded_bytes(input: &str) -> Option<Vec<u8>> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    return None;
                }
                let hi = hex_nibble(bytes[i + 1])?;
                let lo = hex_nibble(bytes[i + 2])?;
                out.push((hi << 4) | lo);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    Some(out)
}

fn hex_nibble(ch: u8) -> Option<u8> {
    match ch {
        b'0'..=b'9' => Some(ch - b'0'),
        b'a'..=b'f' => Some(ch - b'a' + 10),
        b'A'..=b'F' => Some(ch - b'A' + 10),
        _ => None,
    }
}

fn load_svg_xml_from_image_source(source: &str) -> Option<String> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }

    if let Some((mime, data)) = parse_data_uri_bytes(source) {
        if !mime.contains("svg") {
            return None;
        }
        let xml = String::from_utf8(data).ok()?;
        return Some(xml.trim_start_matches('\u{feff}').to_string());
    }

    let path_text = source.strip_prefix("file://").unwrap_or(source);
    let path_text = path_text.split('#').next().unwrap_or(path_text);
    let path_text = path_text.split('?').next().unwrap_or(path_text);
    if !path_text.to_ascii_lowercase().ends_with(".svg") {
        return None;
    }
    let path = Path::new(path_text);
    let xml = std::fs::read_to_string(path).ok()?;
    Some(xml.trim_start_matches('\u{feff}').to_string())
}

fn resolve_non_auto_css_dimension(
    spec: LengthSpec,
    basis: Pt,
    font_size: Pt,
    root_font_size: Pt,
    is_height: bool,
) -> Option<Pt> {
    if matches!(spec, LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial) {
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

    if width.is_none() && let (Some(h), Some(ratio)) = (height, viewbox_ratio) {
        if ratio.is_finite() && ratio > 0.0 {
            width = Some(Pt::from_f32(h.to_f32() * ratio));
        }
    }
    if height.is_none() && let (Some(w), Some(ratio)) = (width, viewbox_ratio) {
        if ratio.is_finite() && ratio > 0.0 {
            height = Some(Pt::from_f32(w.to_f32() / ratio));
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
        WhiteSpaceMode::Pre | WhiteSpaceMode::PreWrap | WhiteSpaceMode::BreakSpaces
    )
}

fn no_wrap(mode: WhiteSpaceMode) -> bool {
    matches!(mode, WhiteSpaceMode::NoWrap | WhiteSpaceMode::Pre)
}

fn inline_dimensions(style: Option<&str>) -> (Option<Pt>, Option<Pt>) {
    let style = match style {
        Some(value) => value,
        None => return (None, None),
    };
    let style_attr = match lightningcss::stylesheet::StyleAttribute::parse(
        style,
        lightningcss::stylesheet::ParserOptions::default(),
    ) {
        Ok(value) => value,
        Err(_) => return (None, None),
    };
    let mut width = None;
    let mut height = None;
    for prop in style_attr.declarations.declarations.iter() {
        match prop {
            lightningcss::properties::Property::Width(size) => {
                width = size_to_points(size);
            }
            lightningcss::properties::Property::Height(size) => {
                height = size_to_points(size);
            }
            _ => {}
        }
    }
    for prop in style_attr.declarations.important_declarations.iter() {
        match prop {
            lightningcss::properties::Property::Width(size) => {
                width = size_to_points(size);
            }
            lightningcss::properties::Property::Height(size) => {
                height = size_to_points(size);
            }
            _ => {}
        }
    }
    (width, height)
}

fn size_to_points(size: &lightningcss::properties::size::Size) -> Option<Pt> {
    match size {
        lightningcss::properties::size::Size::LengthPercentage(value) => match value {
            lightningcss::values::length::LengthPercentage::Dimension(length) => {
                length.to_px().map(|px| Pt::from_f32(px * 0.75))
            }
            _ => None,
        },
        _ => None,
    }
}
