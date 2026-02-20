mod assets;
mod canvas;
mod debug;
mod doc_context;
mod doc_template;
mod error;
mod finalize;
mod flate_native;
mod flowable;
mod font;
mod frame;
mod glyph_report;
mod html;
mod jit;
mod metrics;
mod page_data;
mod page_template;
mod pdf;
mod pdf_raster;
mod pdfinspect;
mod perf;
mod plan;
#[cfg(feature = "python")]
mod python;
mod raster;
mod spill;
mod style;
mod svg;
mod types;

pub use assets::{Asset, AssetBundle, AssetKind};
use base64::Engine;
pub use canvas::{Canvas, Command, Document, Page};
use debug::DebugLogger;
pub use doc_context::DocContext;
pub use doc_template::DocTemplate;
pub use error::FullBleedError;
pub use finalize::{
    BindingSource, ComposeAnnotationMode, ComposePagePlan, FinalizeComposeSummary,
    FinalizeStampSummary, META_PAGE_TEMPLATE_KEY, PageBindingDecision, TemplateAsset,
    TemplateBindingSpec, TemplateCatalog, collect_page_feature_flags, collect_page_template_names,
    compose_overlay_with_template_catalog,
    compose_overlay_with_template_catalog_with_annotation_mode, default_page_map,
    resolve_template_bindings, resolve_template_bindings_for_document,
    stamp_overlay_on_template_pdf, validate_bindings_against_catalog, validate_page_map,
};
pub use flowable::{
    AbsolutePositionedFlowable, BreakAfter, BreakBefore, BreakInside, ContainerFlowable, EdgeSizes,
    Flowable, ImageFlowable, LengthSpec, Pagination, Paragraph, Spacer, SvgFlowable, TableFlowable,
    TextStyle,
};
use font::FontRegistry;
pub use frame::{AddResult, Frame};
pub use glyph_report::{GlyphCoverageReport, MissingGlyph};
use image::GenericImageView;
pub use jit::JitMode;
pub use metrics::{DocumentMetrics, PageMetrics};
pub use page_data::{PageDataContext, PageDataOp, PageDataValue, PaginatedContextSpec};
pub use page_template::{FrameSpec, PageTemplate};
use pdf::PdfOptions;
pub use pdf::{OutputIntent, PdfProfile, PdfVersion};
pub use pdfinspect::{
    PdfInspectError, PdfInspectErrorCode, PdfInspectReport, PdfInspectWarning,
    composition_compatibility_issues, inspect_pdf_bytes, inspect_pdf_path,
    require_pdf_composition_compatibility,
};
use perf::PerfLogger;
use std::f32::consts::PI;
use std::sync::Arc;
pub use types::{Color, ColorSpace, Margins, Pt, Rect, Size};

pub struct FullBleed {
    default_page_size: Size,
    default_margins: Margins,
    page_margins: std::collections::BTreeMap<usize, Margins>,
    page_size_explicit: bool,
    margins_explicit: bool,
    font_registry: Arc<FontRegistry>,
    pdf_options: PdfOptions,
    svg_form_xobjects: bool,
    svg_raster_fallback: bool,
    debug: Option<Arc<DebugLogger>>,
    perf: Option<Arc<PerfLogger>>,
    jit_mode: JitMode,
    layout_strategy: LayoutStrategy,
    lazy_max_passes: usize,
    lazy_budget_ms: f64,
    page_header: Option<PageHeaderSpec>,
    page_header_html: Option<PageHeaderHtmlSpec>,
    page_footer: Option<PageFooterSpec>,
    paginated_context: Option<PaginatedContextSpec>,
    template_binding_spec: Option<TemplateBindingSpec>,
    watermark: Option<WatermarkSpec>,
    asset_css: String,
}

#[derive(Clone)]
pub struct FullBleedBuilder {
    page_size: Size,
    margins: Margins,
    page_size_explicit: bool,
    margins_explicit: bool,
    font_dirs: Vec<std::path::PathBuf>,
    font_files: Vec<std::path::PathBuf>,
    pdf_options: PdfOptions,
    svg_form_xobjects: bool,
    svg_raster_fallback: bool,
    unicode_metrics: bool,
    debug_path: Option<std::path::PathBuf>,
    perf_enabled: bool,
    perf_path: Option<std::path::PathBuf>,
    jit_mode: JitMode,
    layout_strategy: LayoutStrategy,
    accept_lazy_layout_cost: bool,
    lazy_max_passes: usize,
    lazy_budget_ms: f64,
    page_header: Option<PageHeaderSpec>,
    page_header_html: Option<PageHeaderHtmlSpec>,
    page_footer: Option<PageFooterSpec>,
    paginated_context: Option<PaginatedContextSpec>,
    template_binding_spec: Option<TemplateBindingSpec>,
    page_margins: std::collections::BTreeMap<usize, Margins>,
    watermark: Option<WatermarkSpec>,
    asset_bundle: AssetBundle,
}

struct RenderContext {
    resolver: style::StyleResolver,
    page_templates: Vec<PageTemplate>,
}

struct LayoutBuildResult {
    document: Document,
    story_ms: f64,
    layout_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutStrategy {
    Eager,
    Lazy,
}

#[derive(Debug, Clone)]
pub struct PageHeaderSpec {
    pub first: Option<String>,
    pub each: Option<String>,
    pub last: Option<String>,
    pub font_name: String,
    pub font_size: Pt,
    pub color: Color,
    pub x: Pt,
    pub y_from_top: Pt,
}

#[derive(Debug, Clone)]
pub struct PageHeaderHtmlSpec {
    pub first: Option<String>,
    pub each: Option<String>,
    pub last: Option<String>,
    pub x: Pt,
    pub y_from_top: Pt,
    pub width: Pt,
    pub height: Pt,
}

#[derive(Debug, Clone)]
pub struct PageFooterSpec {
    pub first: Option<String>,
    pub each: Option<String>,
    pub last: Option<String>,
    pub font_name: String,
    pub font_size: Pt,
    pub color: Color,
    pub x: Pt,
    pub y_from_bottom: Pt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatermarkLayer {
    Background,
    Overlay,
}

#[derive(Debug, Clone)]
pub enum WatermarkKind {
    Text(String),
    Html(String),
    Image(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatermarkSemantics {
    Visual,
    Artifact,
    Ocg,
}

#[derive(Debug, Clone)]
pub struct WatermarkSpec {
    pub kind: WatermarkKind,
    pub layer: WatermarkLayer,
    pub semantics: WatermarkSemantics,
    pub opacity: f32,
    pub rotation_deg: f32,
    pub font_name: String,
    pub font_size: Pt,
    pub color: Color,
}

impl WatermarkSpec {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            kind: WatermarkKind::Text(text.into()),
            layer: WatermarkLayer::Overlay,
            semantics: WatermarkSemantics::Artifact,
            opacity: 0.15,
            rotation_deg: 0.0,
            font_name: "Helvetica".to_string(),
            font_size: Pt::from_f32(48.0),
            color: Color::rgb(0.6, 0.6, 0.6),
        }
    }

    pub fn html(html: impl Into<String>) -> Self {
        Self {
            kind: WatermarkKind::Html(html.into()),
            layer: WatermarkLayer::Overlay,
            semantics: WatermarkSemantics::Artifact,
            opacity: 0.15,
            rotation_deg: 0.0,
            font_name: "Helvetica".to_string(),
            font_size: Pt::from_f32(48.0),
            color: Color::rgb(0.6, 0.6, 0.6),
        }
    }

    pub fn image(path: impl Into<String>) -> Self {
        Self {
            kind: WatermarkKind::Image(path.into()),
            layer: WatermarkLayer::Overlay,
            semantics: WatermarkSemantics::Artifact,
            opacity: 0.15,
            rotation_deg: 0.0,
            font_name: "Helvetica".to_string(),
            font_size: Pt::from_f32(48.0),
            color: Color::rgb(0.6, 0.6, 0.6),
        }
    }
}

fn apply_page_header(
    doc: &mut Document,
    spec: &PageHeaderSpec,
    page_data: Option<&PageDataContext>,
    report: Option<&mut GlyphCoverageReport>,
    font_registry: Option<&FontRegistry>,
) {
    let mut report = report;
    let total_pages = doc.pages.len();
    if total_pages == 0 {
        return;
    }
    let font_name: Arc<str> = Arc::<str>::from(spec.font_name.as_str());

    for (idx0, page) in doc.pages.iter_mut().enumerate() {
        let page_number = idx0 + 1;
        // Header semantics: don't apply `each` to page 1 (use `first` if provided).
        let template = if total_pages == 1 {
            spec.first.as_deref().or(spec.last.as_deref())
        } else if page_number == 1 {
            spec.first.as_deref()
        } else if page_number == total_pages {
            spec.last.as_deref().or(spec.each.as_deref())
        } else {
            spec.each.as_deref()
        };
        let Some(tpl) = template else { continue };

        let text = page_data::substitute_placeholders(tpl, page_number, total_pages, page_data);

        if let (Some(report), Some(registry)) = (report.as_deref_mut(), font_registry) {
            registry.report_missing_glyphs(&font_name, &[], &text, report);
        }

        page.commands.push(Command::SetFillColor(spec.color));
        page.commands
            .push(Command::SetFontName(spec.font_name.clone()));
        page.commands.push(Command::SetFontSize(spec.font_size));
        page.commands.push(Command::DrawString {
            x: spec.x,
            y: spec.y_from_top,
            text,
        });
    }
}

fn hash_bytes_local(data: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}

fn document_layout_signature(doc: &Document) -> u64 {
    let debug_repr = format!("{:?}", doc);
    hash_bytes_local(debug_repr.as_bytes())
}

fn jit_mode_str(mode: JitMode) -> &'static str {
    match mode {
        JitMode::Off => "off",
        JitMode::PlanOnly => "plan",
        JitMode::PlanAndReplay => "replay",
    }
}

fn layout_strategy_str(strategy: LayoutStrategy) -> &'static str {
    match strategy {
        LayoutStrategy::Eager => "eager",
        LayoutStrategy::Lazy => "lazy",
    }
}

fn pdf_version_str(version: PdfVersion) -> &'static str {
    match version {
        PdfVersion::Pdf17 => "1.7",
        PdfVersion::Pdf20 => "2.0",
    }
}

fn pdf_profile_str(profile: PdfProfile) -> &'static str {
    match profile {
        PdfProfile::None => "none",
        PdfProfile::PdfA2b => "pdfa2b",
        PdfProfile::PdfX4 => "pdfx4",
        PdfProfile::Tagged => "tagged",
    }
}

fn validate_pdf_options(options: &PdfOptions) -> Result<(), FullBleedError> {
    if options.pdf_profile != PdfProfile::PdfX4 {
        return Ok(());
    }

    let Some(intent) = options.output_intent.as_ref() else {
        return Err(FullBleedError::InvalidConfiguration(
            "pdf_profile=pdfx4 requires output_intent".to_string(),
        ));
    };
    if intent.icc_profile.is_empty() {
        return Err(FullBleedError::InvalidConfiguration(
            "output_intent ICC profile cannot be empty".to_string(),
        ));
    }
    if !matches!(intent.n_components, 1 | 3 | 4) {
        return Err(FullBleedError::InvalidConfiguration(format!(
            "output_intent n_components must be one of 1, 3, or 4 (got {})",
            intent.n_components
        )));
    }
    if intent.identifier.trim().is_empty() {
        return Err(FullBleedError::InvalidConfiguration(
            "output_intent identifier cannot be empty".to_string(),
        ));
    }

    Ok(())
}

fn count_commands(doc: &Document) -> usize {
    doc.pages.iter().map(|p| p.commands.len()).sum()
}

fn count_form_commands(doc: &Document) -> (usize, usize, usize, usize) {
    let mut defs = 0usize;
    let mut draws = 0usize;
    let mut svg_defs = 0usize;
    let mut svg_draws = 0usize;
    for page in &doc.pages {
        for cmd in &page.commands {
            match cmd {
                Command::DefineForm { resource_id, .. } => {
                    defs += 1;
                    if resource_id.starts_with("svg:") {
                        svg_defs += 1;
                    }
                }
                Command::DrawForm { resource_id, .. } => {
                    draws += 1;
                    if resource_id.starts_with("svg:") {
                        svg_draws += 1;
                    }
                }
                _ => {}
            }
        }
    }
    (defs, draws, svg_defs, svg_draws)
}

fn count_page_data_entries(ctx: &PageDataContext) -> usize {
    let mut count = 0usize;
    for page in &ctx.pages {
        count += page.len();
    }
    count + ctx.totals.len()
}

fn log_jit_metrics(
    logger: &DebugLogger,
    doc_id: usize,
    mode: JitMode,
    options: &PdfOptions,
    story_ms: f64,
    layout_ms: f64,
    plan_ms: f64,
    finalize_ms: Option<f64>,
    doc: &Document,
    overlay: Option<&Document>,
    plan: Option<&jit::DocPlan>,
    page_data: Option<&PageDataContext>,
) {
    let pages = doc.pages.len();
    let commands = count_commands(doc);
    let overlay_commands = overlay.map(count_commands).unwrap_or(0);
    let (doc_form_defs, doc_form_draws, doc_svg_defs, doc_svg_draws) = count_form_commands(doc);
    let (ov_form_defs, ov_form_draws, ov_svg_defs, ov_svg_draws) =
        overlay.map(count_form_commands).unwrap_or((0, 0, 0, 0));
    let paintables = plan.map(|p| p.paintables.len()).unwrap_or(0);
    let placements = plan
        .map(|p| {
            p.pages
                .iter()
                .map(|page| page.placements.len())
                .sum::<usize>()
        })
        .unwrap_or(0);
    let page_data_entries = page_data.map(count_page_data_entries).unwrap_or(0);
    let finalize_json = finalize_ms
        .map(|v| format!("{v:.3}"))
        .unwrap_or_else(|| "null".to_string());

    let json = format!(
        "{{\"type\":\"jit.metrics\",\"doc_id\":{},\"mode\":\"{}\",\"pdf_version\":\"{}\",\"pdf_profile\":\"{}\",\"timing_ms\":{{\"story\":{:.3},\"layout\":{:.3},\"plan\":{:.3},\"finalize\":{}}},\"counts\":{{\"pages\":{},\"commands\":{},\"overlay_commands\":{},\"form_defs\":{},\"form_draws\":{},\"svg_form_defs\":{},\"svg_form_draws\":{},\"paintables\":{},\"placements\":{},\"page_data_entries\":{}}}}}",
        doc_id,
        jit_mode_str(mode),
        pdf_version_str(options.pdf_version),
        pdf_profile_str(options.pdf_profile),
        story_ms,
        layout_ms,
        plan_ms,
        finalize_json,
        pages,
        commands,
        overlay_commands,
        doc_form_defs + ov_form_defs,
        doc_form_draws + ov_form_draws,
        doc_svg_defs + ov_svg_defs,
        doc_svg_draws + ov_svg_draws,
        paintables,
        placements,
        page_data_entries
    );
    logger.log_json(&json);
}

fn render_html_snippet_to_commands(
    html_snippet: &str,
    resolver: &style::StyleResolver,
    page_size: Size,
    width: Pt,
    height: Pt,
    font_registry: Option<Arc<FontRegistry>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    transparent_body: bool,
    perf: Option<&PerfLogger>,
) -> Vec<Command> {
    // Wrap snippet in a minimal document so the HTML parser picks up body defaults.
    let body_style = if transparent_body {
        " style=\"background: transparent;\""
    } else {
        ""
    };
    let html = format!(
        "<!doctype html><html><body{body_style}>{}</body></html>",
        html_snippet
    );
    let story = html::html_to_story_with_resolver_and_fonts_and_report(
        &html,
        resolver,
        font_registry,
        report,
        svg_form,
        svg_raster_fallback,
        perf,
        None,
    );

    let mut canvas = Canvas::new(page_size);
    let mut frame = Frame::new(Rect {
        x: Pt::ZERO,
        y: Pt::ZERO,
        width,
        height,
    });

    for flowable in story {
        match frame.add(flowable, &mut canvas) {
            AddResult::Placed => {}
            AddResult::Split(_remaining) => break,
            AddResult::Overflow(_remaining) => break,
        }
    }

    let doc = canvas.finish();
    doc.pages
        .get(0)
        .map(|p| p.commands.clone())
        .unwrap_or_default()
}

fn substitute_placeholders_in_commands(
    commands: &[Command],
    page_number: usize,
    total_pages: usize,
    page_data: Option<&PageDataContext>,
) -> Vec<Command> {
    commands
        .iter()
        .map(|cmd| match cmd {
            Command::DrawString { x, y, text } => Command::DrawString {
                x: *x,
                y: *y,
                text: page_data::substitute_placeholders(text, page_number, total_pages, page_data),
            },
            Command::DefineForm {
                resource_id,
                width,
                height,
                commands,
            } => Command::DefineForm {
                resource_id: resource_id.clone(),
                width: *width,
                height: *height,
                commands: substitute_placeholders_in_commands(
                    commands,
                    page_number,
                    total_pages,
                    page_data,
                ),
            },
            _ => cmd.clone(),
        })
        .collect()
}

fn apply_page_header_html(
    doc: &mut Document,
    spec: &PageHeaderHtmlSpec,
    resolver: &style::StyleResolver,
    page_data: Option<&PageDataContext>,
    font_registry: Option<Arc<FontRegistry>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
    perf: Option<&PerfLogger>,
) {
    let total_pages = doc.pages.len();
    if total_pages == 0 {
        return;
    }

    struct HeaderTemplateEntry {
        base_commands: Vec<Command>,
        slot_eligible: bool,
        rendered_cache: std::collections::HashMap<u64, Vec<Command>>,
    }

    let mut template_cache: std::collections::HashMap<String, HeaderTemplateEntry> =
        std::collections::HashMap::new();
    let mut report = report;
    let use_cache = report.is_none();

    for (idx0, page) in doc.pages.iter_mut().enumerate() {
        let page_number = idx0 + 1;

        // Header semantics: don't apply `each` to page 1.
        let template = if total_pages == 1 {
            spec.first.as_deref().or(spec.last.as_deref())
        } else if page_number == 1 {
            spec.first.as_deref()
        } else if page_number == total_pages {
            spec.last.as_deref().or(spec.each.as_deref())
        } else {
            spec.each.as_deref()
        };
        let Some(tpl) = template else { continue };

        let rendered = page_data::substitute_placeholders(tpl, page_number, total_pages, page_data);

        let entry = template_cache.entry(tpl.to_string()).or_insert_with(|| {
            let slot_eligible = !html::template_uses_attribute_placeholders(tpl);
            let base_commands = if slot_eligible && use_cache {
                render_html_snippet_to_commands(
                    tpl,
                    resolver,
                    doc.page_size,
                    spec.width,
                    spec.height,
                    font_registry.clone(),
                    None,
                    svg_form,
                    svg_raster_fallback,
                    false,
                    perf,
                )
            } else {
                Vec::new()
            };
            HeaderTemplateEntry {
                base_commands,
                slot_eligible,
                rendered_cache: std::collections::HashMap::new(),
            }
        });

        let cmds = if use_cache && entry.slot_eligible {
            let key = hash_bytes_local(rendered.as_bytes());
            entry
                .rendered_cache
                .entry(key)
                .or_insert_with(|| {
                    substitute_placeholders_in_commands(
                        &entry.base_commands,
                        page_number,
                        total_pages,
                        page_data,
                    )
                })
                .clone()
        } else if use_cache {
            let key = hash_bytes_local(rendered.as_bytes());
            entry
                .rendered_cache
                .entry(key)
                .or_insert_with(|| {
                    render_html_snippet_to_commands(
                        &rendered,
                        resolver,
                        doc.page_size,
                        spec.width,
                        spec.height,
                        font_registry.clone(),
                        None,
                        svg_form,
                        svg_raster_fallback,
                        false,
                        perf,
                    )
                })
                .clone()
        } else {
            render_html_snippet_to_commands(
                &rendered,
                resolver,
                doc.page_size,
                spec.width,
                spec.height,
                font_registry.clone(),
                report.as_deref_mut(),
                svg_form,
                svg_raster_fallback,
                false,
                perf,
            )
        };

        let form_id = format!("hdr-{:016x}", hash_bytes_local(rendered.as_bytes()));
        page.commands.push(Command::DefineForm {
            resource_id: form_id.clone(),
            width: spec.width,
            height: spec.height,
            commands: cmds,
        });
        page.commands.push(Command::SaveState);
        page.commands
            .push(Command::Translate(spec.x, spec.y_from_top));
        page.commands.push(Command::ClipRect {
            x: Pt::ZERO,
            y: Pt::ZERO,
            width: spec.width,
            height: spec.height,
        });
        page.commands.push(Command::DrawForm {
            x: Pt::ZERO,
            y: Pt::ZERO,
            width: spec.width,
            height: spec.height,
            resource_id: form_id,
        });
        page.commands.push(Command::RestoreState);
    }
}

fn apply_page_footer(
    doc: &mut Document,
    spec: &PageFooterSpec,
    page_data: Option<&PageDataContext>,
    report: Option<&mut GlyphCoverageReport>,
    font_registry: Option<&FontRegistry>,
) {
    let mut report = report;
    let total_pages = doc.pages.len();
    if total_pages == 0 {
        return;
    }
    let font_name: Arc<str> = Arc::<str>::from(spec.font_name.as_str());

    for (idx0, page) in doc.pages.iter_mut().enumerate() {
        let page_number = idx0 + 1;
        let template = if total_pages == 1 {
            // A single-page document is both "first" and "last". Prefer `last` so a
            // "Grand Total" footer shows up even on 1-page records.
            spec.last
                .as_deref()
                .or(spec.first.as_deref())
                .or(spec.each.as_deref())
        } else if page_number == 1 {
            spec.first.as_deref().or(spec.each.as_deref())
        } else if page_number == total_pages {
            spec.last.as_deref().or(spec.each.as_deref())
        } else {
            spec.each.as_deref()
        };
        let Some(tpl) = template else { continue };

        let text = page_data::substitute_placeholders(tpl, page_number, total_pages, page_data);

        if let (Some(report), Some(registry)) = (report.as_deref_mut(), font_registry) {
            registry.report_missing_glyphs(&font_name, &[], &text, report);
        }

        // Our coordinate system is top-left origin; DrawString expects y = top of the text box.
        let y = (doc.page_size.height - spec.y_from_bottom - spec.font_size).max(Pt::ZERO);

        page.commands.push(Command::SetFillColor(spec.color));
        page.commands
            .push(Command::SetFontName(spec.font_name.clone()));
        page.commands.push(Command::SetFontSize(spec.font_size));
        page.commands
            .push(Command::DrawString { x: spec.x, y, text });
    }
}

fn watermark_image_bytes(source: &str) -> Option<Vec<u8>> {
    if source.starts_with("data:") {
        let parts: Vec<&str> = source.splitn(2, ',').collect();
        if parts.len() != 2 {
            return None;
        }
        let header = parts[0];
        let data_part = parts[1];
        if header.contains("base64") {
            return base64::engine::general_purpose::STANDARD
                .decode(data_part)
                .ok();
        }
        return Some(data_part.as_bytes().to_vec());
    }
    std::fs::read(source).ok()
}

fn watermark_image_size(source: &str, page_size: Size) -> Option<Size> {
    let bytes = watermark_image_bytes(source)?;
    let decoded = image::load_from_memory(&bytes).ok()?;
    let (w, h) = decoded.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let max_w = page_size.width.to_f32() * 0.35;
    let max_h = page_size.height.to_f32() * 0.35;
    let mut scale = max_w / (w as f32);
    let height = (h as f32) * scale;
    if height > max_h {
        scale = max_h / (h as f32);
    }
    let width = (w as f32) * scale;
    let height = (h as f32) * scale;
    Some(Size {
        width: Pt::from_f32(width),
        height: Pt::from_f32(height),
    })
}

const WATERMARK_OCG_RESOURCE_NAME: &str = "FBWM";

fn build_watermark_commands(
    spec: &WatermarkSpec,
    page_size: Size,
    page_number: usize,
    total_pages: usize,
    page_data: Option<&PageDataContext>,
    resolver: &style::StyleResolver,
    font_registry: Option<Arc<FontRegistry>>,
    report: Option<&mut GlyphCoverageReport>,
    svg_form: bool,
    svg_raster_fallback: bool,
) -> Vec<Command> {
    let mut commands = Vec::new();
    let angle = spec.rotation_deg * (PI / 180.0);
    let cx = page_size.width.mul_ratio(1, 2);
    let cy = page_size.height.mul_ratio(1, 2);

    let mut report = report;

    match spec.semantics {
        WatermarkSemantics::Visual => {}
        WatermarkSemantics::Artifact => {
            commands.push(Command::BeginArtifact {
                subtype: Some("Watermark".to_string()),
            });
        }
        WatermarkSemantics::Ocg => {
            commands.push(Command::BeginOptionalContent {
                name: WATERMARK_OCG_RESOURCE_NAME.to_string(),
            });
            commands.push(Command::BeginArtifact {
                subtype: Some("Watermark".to_string()),
            });
        }
    }

    match &spec.kind {
        WatermarkKind::Text(text) => {
            let rendered =
                page_data::substitute_placeholders(text, page_number, total_pages, page_data);
            let width = if let Some(registry) = font_registry.as_deref() {
                registry.measure_text_width(&spec.font_name, spec.font_size, &rendered)
            } else {
                let approx = spec.font_size.to_f32() * 0.6;
                Pt::from_f32(approx * rendered.chars().count() as f32)
            };
            // DrawString uses top-left page coordinates with a y-flip in PDF emission.
            // Compensate here so local transformed coordinates land where expected.
            let local_y = Pt::ZERO - spec.font_size.mul_ratio(1, 2);
            let compensated_y = page_size.height - local_y - spec.font_size;

            if let (Some(report), Some(registry)) =
                (report.as_deref_mut(), font_registry.as_deref())
            {
                let font_name: Arc<str> = Arc::<str>::from(spec.font_name.as_str());
                registry.report_missing_glyphs(&font_name, &[], &rendered, report);
            }

            commands.push(Command::SaveState);
            commands.push(Command::SetOpacity {
                fill: spec.opacity,
                stroke: spec.opacity,
            });
            commands.push(Command::SetFillColor(spec.color));
            commands.push(Command::Translate(cx, cy));
            if angle.abs() > f32::EPSILON {
                commands.push(Command::Rotate(angle));
            }
            commands.push(Command::SetFontName(spec.font_name.clone()));
            commands.push(Command::SetFontSize(spec.font_size));
            commands.push(Command::DrawString {
                x: Pt::ZERO - width.mul_ratio(1, 2),
                y: compensated_y,
                text: rendered,
            });
            commands.push(Command::RestoreState);
        }
        WatermarkKind::Html(html) => {
            let rendered =
                page_data::substitute_placeholders(html, page_number, total_pages, page_data);
            let width = page_size.width;
            let height = page_size.height;
            let cmds = render_html_snippet_to_commands(
                &rendered,
                resolver,
                page_size,
                width,
                height,
                font_registry,
                report.as_deref_mut(),
                svg_form,
                svg_raster_fallback,
                true,
                None,
            );
            let form_id = format!("wm-{:016x}", hash_bytes_local(rendered.as_bytes()));
            commands.push(Command::DefineForm {
                resource_id: form_id.clone(),
                width,
                height,
                commands: cmds,
            });
            commands.push(Command::SaveState);
            commands.push(Command::SetOpacity {
                fill: spec.opacity,
                stroke: spec.opacity,
            });
            commands.push(Command::Translate(cx, cy));
            if angle.abs() > f32::EPSILON {
                commands.push(Command::Rotate(angle));
            }
            let local_x = Pt::ZERO - width.mul_ratio(1, 2);
            let local_y = Pt::ZERO - height.mul_ratio(1, 2);
            let compensated_y = page_size.height - local_y - height;
            commands.push(Command::DrawForm {
                x: local_x,
                y: compensated_y,
                width,
                height,
                resource_id: form_id,
            });
            commands.push(Command::RestoreState);
        }
        WatermarkKind::Image(path) => {
            let size = watermark_image_size(path, page_size).unwrap_or(Size {
                width: page_size.width.mul_ratio(1, 3),
                height: page_size.height.mul_ratio(1, 3),
            });
            commands.push(Command::SaveState);
            commands.push(Command::SetOpacity {
                fill: spec.opacity,
                stroke: spec.opacity,
            });
            commands.push(Command::Translate(cx, cy));
            if angle.abs() > f32::EPSILON {
                commands.push(Command::Rotate(angle));
            }
            commands.push(Command::Translate(
                Pt::ZERO - size.width.mul_ratio(1, 2),
                Pt::ZERO - size.height.mul_ratio(1, 2),
            ));
            // DrawImage also uses top-left page coordinates with y-flip in PDF emission.
            // Use compensated y so local transformed origin maps to watermark center.
            let compensated_y = page_size.height - size.height;
            commands.push(Command::DrawImage {
                x: Pt::ZERO,
                y: compensated_y,
                width: size.width,
                height: size.height,
                resource_id: path.clone(),
            });
            commands.push(Command::RestoreState);
        }
    }

    match spec.semantics {
        WatermarkSemantics::Visual => {}
        WatermarkSemantics::Artifact => {
            commands.push(Command::EndMarkedContent);
        }
        WatermarkSemantics::Ocg => {
            commands.push(Command::EndMarkedContent);
            commands.push(Command::EndMarkedContent);
        }
    }

    commands
}

fn build_watermark_document(
    base: &Document,
    spec: &WatermarkSpec,
    resolver: &style::StyleResolver,
    page_data: Option<&PageDataContext>,
    mut report: Option<&mut GlyphCoverageReport>,
    font_registry: Option<Arc<FontRegistry>>,
    svg_form: bool,
    svg_raster_fallback: bool,
) -> Document {
    let total_pages = base.pages.len();
    let mut doc = Document {
        page_size: base.page_size,
        pages: base
            .pages
            .iter()
            .map(|_| Page {
                commands: Vec::new(),
            })
            .collect(),
    };

    for (idx, page) in doc.pages.iter_mut().enumerate() {
        let page_number = idx + 1;
        let cmds = build_watermark_commands(
            spec,
            base.page_size,
            page_number,
            total_pages,
            page_data,
            resolver,
            font_registry.clone(),
            report.as_deref_mut(),
            svg_form,
            svg_raster_fallback,
        );
        page.commands.extend(cmds);
    }

    doc
}

fn merge_background_commands(base: &mut Document, background: &Document) {
    if base.pages.len() != background.pages.len() {
        return;
    }
    for (base_page, bg_page) in base.pages.iter_mut().zip(background.pages.iter()) {
        if !bg_page.commands.is_empty() {
            base_page.commands.splice(0..0, bg_page.commands.clone());
        }
    }
}

impl FullBleed {
    pub fn builder() -> FullBleedBuilder {
        FullBleedBuilder::new()
    }

    fn emit_debug_summary(&self, context: &str) {
        if let Some(logger) = self.debug.as_deref() {
            logger.emit_summary(context);
            logger.flush();
        }
        if let Some(perf) = self.perf.as_deref() {
            perf.flush();
        }
    }

    fn emit_html_asset_warnings(&self, doc_id: usize, html: &str) {
        let warnings = html::scan_html_asset_warnings(html);
        if warnings.is_empty() {
            return;
        }
        for warning in warnings {
            let detail_preview = if warning.details.is_empty() {
                String::new()
            } else {
                let mut preview = warning.details.clone();
                if preview.len() > 3 {
                    preview.truncate(3);
                    preview.push("...".to_string());
                }
                format!(" ({})", preview.join(", "))
            };
            eprintln!(
                "[fullbleed][assets] doc {}: {}{}",
                doc_id, warning.message, detail_preview
            );
            if let Some(logger) = self.debug.as_deref() {
                let details = warning
                    .details
                    .iter()
                    .map(|d| format!("\"{}\"", d.replace('"', "\\\"")))
                    .collect::<Vec<_>>()
                    .join(",");
                let json = format!(
                    "{{\"type\":\"jit.html_asset_warning\",\"doc_id\":{},\"kind\":\"{}\",\"message\":\"{}\",\"details\":[{}]}}",
                    doc_id,
                    warning.kind.replace('"', "\\\""),
                    warning.message.replace('"', "\\\""),
                    details
                );
                logger.log_json(&json);
            }
        }
    }

    fn has_full_page_background(doc: &Document) -> bool {
        let page_w = doc.page_size.width.to_f32();
        let page_h = doc.page_size.height.to_f32();
        let min_w = page_w * 0.85;
        let min_h = page_h * 0.85;
        for page in &doc.pages {
            for cmd in &page.commands {
                if let Command::DrawRect {
                    x,
                    y,
                    width,
                    height,
                } = cmd
                {
                    let w = width.to_f32();
                    let h = height.to_f32();
                    let x = x.to_f32();
                    let y = y.to_f32();
                    if w >= min_w && h >= min_h && x <= 20.0 && y <= 20.0 {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn merge_css(&self, css: &str) -> String {
        if self.asset_css.is_empty() {
            css.to_string()
        } else if css.trim().is_empty() {
            self.asset_css.clone()
        } else {
            let mut merged = String::with_capacity(self.asset_css.len() + css.len() + 2);
            merged.push_str(&self.asset_css);
            merged.push('\n');
            merged.push('\n');
            merged.push_str(css);
            merged
        }
    }

    fn layout_pass_limit(&self) -> usize {
        match self.layout_strategy {
            LayoutStrategy::Eager => 1,
            LayoutStrategy::Lazy => self.lazy_max_passes.max(1),
        }
    }

    fn emit_layout_strategy_diagnostics(
        &self,
        doc_id: usize,
        pass_limit: usize,
        passes: usize,
        converged: bool,
        budget_hit: bool,
        elapsed_ms: f64,
    ) {
        if let Some(logger) = self.debug.as_deref() {
            let json = format!(
                "{{\"type\":\"jit.layout_strategy\",\"doc_id\":{},\"strategy\":\"{}\",\"pass_limit\":{},\"passes\":{},\"converged\":{},\"budget_ms\":{:.3},\"elapsed_ms\":{:.3},\"budget_hit\":{}}}",
                doc_id,
                layout_strategy_str(self.layout_strategy),
                pass_limit,
                passes,
                if converged { "true" } else { "false" },
                self.lazy_budget_ms,
                elapsed_ms,
                if budget_hit { "true" } else { "false" }
            );
            logger.log_json(&json);
            logger.increment("jit.layout.strategy", 1);
            if self.layout_strategy == LayoutStrategy::Lazy && !converged {
                logger.increment("jit.known_loss.lazy_layout_no_convergence", 1);
            }
        }
        if let Some(perf) = self.perf.as_deref() {
            perf.log_span_ms("layout.strategy", Some(doc_id), elapsed_ms);
            perf.log_counts(
                "layout.strategy",
                Some(doc_id),
                &[
                    ("passes", passes as u64),
                    ("pass_limit", pass_limit as u64),
                    ("converged", if converged { 1 } else { 0 }),
                    ("budget_hit", if budget_hit { 1 } else { 0 }),
                ],
            );
        }
    }

    fn build_document_with_layout_strategy(
        &self,
        doc_id: usize,
        html: &str,
        page_templates: &[PageTemplate],
        resolver: &style::StyleResolver,
        report: Option<&mut GlyphCoverageReport>,
    ) -> Result<LayoutBuildResult, FullBleedError> {
        let lazy = self.layout_strategy == LayoutStrategy::Lazy;
        let pass_limit = self.layout_pass_limit();
        let started = std::time::Instant::now();
        let mut story_ms = 0.0;
        let mut layout_ms = 0.0;
        let mut passes = 0usize;
        let mut converged = false;
        let mut budget_hit = false;
        let mut previous_signature: Option<u64> = None;
        let mut built: Option<Document> = None;
        let mut report = report;
        let collect_report = report.is_some();
        let mut final_report: Option<GlyphCoverageReport> = None;

        for pass in 0..pass_limit {
            if lazy && pass > 0 && started.elapsed().as_secs_f64() * 1000.0 >= self.lazy_budget_ms {
                budget_hit = true;
                break;
            }

            let mut pass_report = GlyphCoverageReport::default();
            let mut pass_report_ref = if collect_report {
                Some(&mut pass_report)
            } else {
                None
            };

            passes += 1;
            let t_story = std::time::Instant::now();
            let story = html::html_to_story_with_resolver_and_fonts_and_report(
                html,
                resolver,
                Some(self.font_registry.clone()),
                pass_report_ref.as_deref_mut(),
                self.svg_form_xobjects,
                self.svg_raster_fallback,
                self.perf.as_deref(),
                Some(doc_id),
            );
            story_ms += t_story.elapsed().as_secs_f64() * 1000.0;

            let mut doc = DocTemplate::new(page_templates.to_vec());
            if let Some(logger) = self.debug.clone() {
                doc = doc.with_debug(logger, Some(doc_id));
            }
            for flowable in story {
                doc.add_flowable(flowable);
            }

            let t_layout = std::time::Instant::now();
            let _perf_guard = flowable::set_perf_context(self.perf.clone(), Some(doc_id));
            let next_built = doc.build()?;
            layout_ms += t_layout.elapsed().as_secs_f64() * 1000.0;

            let signature = document_layout_signature(&next_built);
            converged = !lazy || previous_signature.is_some_and(|last| last == signature);
            previous_signature = Some(signature);
            built = Some(next_built);
            if collect_report {
                final_report = Some(pass_report);
            }
            if converged {
                break;
            }
        }

        self.emit_layout_strategy_diagnostics(
            doc_id,
            pass_limit,
            passes,
            converged,
            budget_hit,
            started.elapsed().as_secs_f64() * 1000.0,
        );

        if let Some(report) = report.as_deref_mut() {
            if let Some(pass_report) = final_report {
                *report = pass_report;
            }
        }

        let Some(document) = built else {
            return Err(FullBleedError::InvalidConfiguration(
                "layout pass budget prevented any layout pass".to_string(),
            ));
        };

        Ok(LayoutBuildResult {
            document,
            story_ms,
            layout_ms,
        })
    }

    fn resolve_page_templates_for_css(
        &self,
        merged_css: &str,
        doc_id: Option<usize>,
    ) -> Vec<PageTemplate> {
        let mut page_size = self.default_page_size;
        let mut base_margins = self.default_margins;
        let mut page_margins = self.page_margins.clone();
        let page_setup = style::extract_css_page_setup(
            merged_css,
            self.debug.as_deref(),
            Some(self.default_page_size),
        );

        if let Some(css_size) = page_setup.size {
            if self.page_size_explicit {
                if let Some(logger) = self.debug.as_deref() {
                    let doc_id = doc_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "null".to_string());
                    let json = format!(
                        "{{\"type\":\"jit.known_loss\",\"doc_id\":{},\"code\":\"PAGE_SIZE_OVERRIDDEN\",\"runtime\":{{\"w\":{:.3},\"h\":{:.3}}},\"css\":{{\"w\":{:.3},\"h\":{:.3}}}}}",
                        doc_id,
                        self.default_page_size.width.to_f32(),
                        self.default_page_size.height.to_f32(),
                        css_size.width.to_f32(),
                        css_size.height.to_f32()
                    );
                    logger.log_json(&json);
                    logger.increment("jit.known_loss.page_size_overridden", 1);
                }
            } else {
                page_size = css_size;
            }
        }

        if let Some(css_margins) = page_setup.resolve_margins(base_margins) {
            if self.margins_explicit {
                if let Some(logger) = self.debug.as_deref() {
                    logger.increment("jit.page_margin.css_overridden", 1);
                }
            } else {
                base_margins = css_margins;
                page_margins.clear();
            }
        }

        if let Some(logger) = self.debug.as_deref() {
            let doc_id = doc_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "null".to_string());
            let json = format!(
                "{{\"type\":\"jit.page_setup\",\"doc_id\":{},\"page_size\":{{\"w\":{:.3},\"h\":{:.3}}},\"margins\":{{\"top\":{:.3},\"right\":{:.3},\"bottom\":{:.3},\"left\":{:.3}}},\"css\":{{\"size_present\":{},\"margin_present\":{}}}}}",
                doc_id,
                page_size.width.to_f32(),
                page_size.height.to_f32(),
                base_margins.top.to_f32(),
                base_margins.right.to_f32(),
                base_margins.bottom.to_f32(),
                base_margins.left.to_f32(),
                if page_setup.size.is_some() {
                    "true"
                } else {
                    "false"
                },
                if page_setup.has_margin_override() {
                    "true"
                } else {
                    "false"
                }
            );
            logger.log_json(&json);
        }

        build_page_templates(page_size, base_margins, &page_margins)
    }

    fn build_render_context(&self, css: &str, doc_id: Option<usize>) -> RenderContext {
        let t_css = std::time::Instant::now();
        let merged_css = self.merge_css(css);
        let page_templates = self.resolve_page_templates_for_css(&merged_css, doc_id);
        let page_size = page_templates.get(0).map(|t| t.page_size).unwrap_or(Size {
            width: Pt::ZERO,
            height: Pt::ZERO,
        });
        let resolver = style::StyleResolver::new_with_debug_and_viewport(
            &merged_css,
            self.debug.clone(),
            Some(page_size),
        );
        if let Some(logger) = self.debug.as_deref() {
            let css_ms = t_css.elapsed().as_secs_f64() * 1000.0;
            let doc_id = doc_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "null".to_string());
            let json = format!(
                "{{\"type\":\"jit.css\",\"doc_id\":{},\"css_ms\":{:.3},\"bytes\":{}}}",
                doc_id,
                css_ms,
                merged_css.len()
            );
            logger.log_json(&json);
        }
        if let Some(logger) = self.perf.as_deref() {
            let css_ms = t_css.elapsed().as_secs_f64() * 1000.0;
            logger.log_span_ms("css.parse", doc_id, css_ms);
            logger.log_counts("css.parse", doc_id, &[("bytes", merged_css.len() as u64)]);
        }
        RenderContext {
            resolver,
            page_templates,
        }
    }

    fn merge_overlay_commands(base: &mut Document, overlay: &Document) {
        if base.pages.len() != overlay.pages.len() {
            return;
        }
        for (base_page, overlay_page) in base.pages.iter_mut().zip(overlay.pages.iter()) {
            if !overlay_page.commands.is_empty() {
                base_page.commands.extend(overlay_page.commands.clone());
            }
        }
    }

    fn build_overlay_documents(
        &self,
        base: &Document,
        resolver: &style::StyleResolver,
        page_data: Option<&PageDataContext>,
        report: Option<&mut GlyphCoverageReport>,
    ) -> (Option<Document>, Option<Document>) {
        let mut report = report;
        let mut overlay = Document {
            page_size: base.page_size,
            pages: base
                .pages
                .iter()
                .map(|_| Page {
                    commands: Vec::new(),
                })
                .collect(),
        };

        let mut has_overlay = false;

        if let Some(spec) = &self.watermark {
            let force_overlay =
                spec.layer == WatermarkLayer::Background && Self::has_full_page_background(base);
            let as_overlay = spec.layer == WatermarkLayer::Overlay || force_overlay;
            if as_overlay {
                let wm = build_watermark_document(
                    base,
                    spec,
                    resolver,
                    page_data,
                    report.as_deref_mut(),
                    Some(self.font_registry.clone()),
                    self.svg_form_xobjects,
                    self.svg_raster_fallback,
                );
                Self::merge_overlay_commands(&mut overlay, &wm);
                has_overlay = true;
                if force_overlay {
                    if let Some(logger) = self.debug.as_deref() {
                        let json = format!(
                            "{{\"type\":\"jit.watermark\",\"layer\":\"background\",\"fallback\":\"overlay\",\"reason\":\"body_background\"}}"
                        );
                        logger.log_json(&json);
                    }
                }
            }
        }

        if let Some(spec) = &self.page_header_html {
            apply_page_header_html(
                &mut overlay,
                spec,
                resolver,
                page_data,
                Some(self.font_registry.clone()),
                report.as_deref_mut(),
                self.svg_form_xobjects,
                self.svg_raster_fallback,
                self.perf.as_deref(),
            );
            has_overlay = true;
        } else if let Some(spec) = &self.page_header {
            apply_page_header(
                &mut overlay,
                spec,
                page_data,
                report.as_deref_mut(),
                Some(self.font_registry.as_ref()),
            );
            has_overlay = true;
        }

        if let Some(spec) = &self.page_footer {
            apply_page_footer(
                &mut overlay,
                spec,
                page_data,
                report.as_deref_mut(),
                Some(self.font_registry.as_ref()),
            );
            has_overlay = true;
        }

        let overlay = if has_overlay { Some(overlay) } else { None };

        let background = self.watermark.as_ref().and_then(|spec| {
            if spec.layer == WatermarkLayer::Background && !Self::has_full_page_background(base) {
                Some(build_watermark_document(
                    base,
                    spec,
                    resolver,
                    page_data,
                    report.as_deref_mut(),
                    Some(self.font_registry.clone()),
                    self.svg_form_xobjects,
                    self.svg_raster_fallback,
                ))
            } else {
                None
            }
        });

        (overlay, background)
    }

    fn finalize_with_jit(
        &self,
        doc_id: usize,
        mut base: Document,
        overlay: Option<Document>,
        background: Option<Document>,
        page_data: Option<PageDataContext>,
        plan: Option<jit::DocPlan>,
    ) -> Document {
        match self.jit_mode {
            JitMode::Off => {
                if let Some(ref bg_doc) = background {
                    merge_background_commands(&mut base, bg_doc);
                }
                if let Some(ref overlay_doc) = overlay {
                    Self::merge_overlay_commands(&mut base, overlay_doc);
                }
                base
            }
            JitMode::PlanOnly => {
                let _plan = plan.or_else(|| {
                    Some(jit::plan_document_with_overlay(
                        doc_id,
                        &base,
                        background.as_ref(),
                        overlay.as_ref(),
                        page_data,
                        self.debug.clone(),
                        Some(self.font_registry.as_ref()),
                    ))
                });
                if let Some(ref bg_doc) = background {
                    merge_background_commands(&mut base, bg_doc);
                }
                if let Some(ref overlay_doc) = overlay {
                    Self::merge_overlay_commands(&mut base, overlay_doc);
                }
                base
            }
            JitMode::PlanAndReplay => {
                let plan = plan.unwrap_or_else(|| {
                    jit::plan_document_with_overlay(
                        doc_id,
                        &base,
                        background.as_ref(),
                        overlay.as_ref(),
                        page_data,
                        self.debug.clone(),
                        Some(self.font_registry.as_ref()),
                    )
                });
                let ops = jit::paint_plan(&plan, self.debug.clone());
                jit::ops_to_document(plan.page_size, ops)
            }
        }
    }

    fn render_to_document_and_page_data_with_resolver_and_report_at(
        &self,
        doc_id: usize,
        html: &str,
        page_templates: &[PageTemplate],
        resolver: &style::StyleResolver,
        report: Option<&mut GlyphCoverageReport>,
    ) -> Result<(Document, Option<PageDataContext>), FullBleedError> {
        let mut report = report;
        let perf = self.perf.as_deref();
        self.emit_html_asset_warnings(doc_id, html);
        let layout = self.build_document_with_layout_strategy(
            doc_id,
            html,
            page_templates,
            resolver,
            report.as_deref_mut(),
        )?;
        let built = layout.document;
        let story_ms = layout.story_ms;
        let layout_ms = layout.layout_ms;

        let page_data_override = self
            .paginated_context
            .as_ref()
            .map(|spec| page_data::compute_page_data_context(&built, spec));

        let t_plan = std::time::Instant::now();
        let planned = plan::plan_document_with_overlay(
            doc_id,
            &built,
            self.paginated_context.as_ref(),
            self.template_binding_spec.as_ref(),
            page_data_override.clone(),
            self.debug.clone(),
            self.jit_mode,
            Some(self.font_registry.as_ref()),
            |page_data| {
                self.build_overlay_documents(&built, resolver, page_data, report.as_deref_mut())
            },
        )?;
        let plan_ms = t_plan.elapsed().as_secs_f64() * 1000.0;
        let template_binding_count = planned
            .template_bindings
            .as_ref()
            .map(|v| v.len())
            .unwrap_or(0);

        if let Some(logger) = self.debug.as_deref() {
            let t_finalize = std::time::Instant::now();
            let built = self.finalize_with_jit(
                doc_id,
                built,
                planned.overlay.clone(),
                planned.background.clone(),
                planned.page_data.clone(),
                planned.plan.clone(),
            );
            let finalize_ms = t_finalize.elapsed().as_secs_f64() * 1000.0;
            log_jit_metrics(
                logger,
                doc_id,
                self.jit_mode,
                &self.pdf_options,
                story_ms,
                layout_ms,
                plan_ms,
                Some(finalize_ms),
                &built,
                planned.overlay.as_ref(),
                planned.plan.as_ref(),
                planned.page_data.as_ref().or(page_data_override.as_ref()),
            );
            if let Some(perf_logger) = perf {
                perf_logger.log_span_ms("story", Some(doc_id), story_ms);
                perf_logger.log_span_ms("layout", Some(doc_id), layout_ms);
                perf_logger.log_span_ms("plan", Some(doc_id), plan_ms);
                perf_logger.log_span_ms("finalize", Some(doc_id), finalize_ms);
                let command_count: usize = built.pages.iter().map(|page| page.commands.len()).sum();
                perf_logger.log_counts(
                    "doc",
                    Some(doc_id),
                    &[
                        ("pages", built.pages.len() as u64),
                        ("commands", command_count as u64),
                        ("template_bindings", template_binding_count as u64),
                    ],
                );
            }
            return Ok((built, planned.page_data));
        }

        let built = self.finalize_with_jit(
            doc_id,
            built,
            planned.overlay,
            planned.background,
            planned.page_data.clone(),
            planned.plan,
        );
        if let Some(perf_logger) = perf {
            perf_logger.log_span_ms("story", Some(doc_id), story_ms);
            perf_logger.log_span_ms("layout", Some(doc_id), layout_ms);
            perf_logger.log_span_ms("plan", Some(doc_id), plan_ms);
            let command_count: usize = built.pages.iter().map(|page| page.commands.len()).sum();
            perf_logger.log_counts(
                "doc",
                Some(doc_id),
                &[
                    ("pages", built.pages.len() as u64),
                    ("commands", command_count as u64),
                    ("template_bindings", template_binding_count as u64),
                ],
            );
        }
        Ok((built, planned.page_data.or(page_data_override)))
    }

    fn render_to_planned_doc_with_resolver_and_report_at(
        &self,
        doc_id: usize,
        html: &str,
        page_templates: &[PageTemplate],
        resolver: &style::StyleResolver,
        report: Option<&mut GlyphCoverageReport>,
    ) -> Result<plan::PlannedDoc, FullBleedError> {
        let mut report = report;
        let perf = self.perf.as_deref();
        let layout = self.build_document_with_layout_strategy(
            doc_id,
            html,
            page_templates,
            resolver,
            report.as_deref_mut(),
        )?;
        let built = layout.document;
        let story_ms = layout.story_ms;
        let layout_ms = layout.layout_ms;

        let page_data_override = self
            .paginated_context
            .as_ref()
            .map(|spec| page_data::compute_page_data_context(&built, spec));

        let t_plan = std::time::Instant::now();
        let planned = plan::plan_document_with_overlay(
            doc_id,
            &built,
            self.paginated_context.as_ref(),
            self.template_binding_spec.as_ref(),
            page_data_override.clone(),
            self.debug.clone(),
            self.jit_mode,
            Some(self.font_registry.as_ref()),
            |page_data| {
                self.build_overlay_documents(&built, resolver, page_data, report.as_deref_mut())
            },
        )?;
        let plan_ms = t_plan.elapsed().as_secs_f64() * 1000.0;
        let template_binding_count = planned
            .template_bindings
            .as_ref()
            .map(|v| v.len())
            .unwrap_or(0);
        if let Some(logger) = self.debug.as_deref() {
            log_jit_metrics(
                logger,
                doc_id,
                self.jit_mode,
                &self.pdf_options,
                story_ms,
                layout_ms,
                plan_ms,
                None,
                &built,
                planned.overlay.as_ref(),
                planned.plan.as_ref(),
                planned.page_data.as_ref().or(page_data_override.as_ref()),
            );
        }
        if let Some(perf_logger) = perf {
            perf_logger.log_span_ms("story", Some(doc_id), story_ms);
            perf_logger.log_span_ms("layout", Some(doc_id), layout_ms);
            perf_logger.log_span_ms("plan", Some(doc_id), plan_ms);
            let command_count: usize = built.pages.iter().map(|page| page.commands.len()).sum();
            perf_logger.log_counts(
                "doc",
                Some(doc_id),
                &[
                    ("pages", built.pages.len() as u64),
                    ("commands", command_count as u64),
                    ("template_bindings", template_binding_count as u64),
                ],
            );
        }

        Ok(planned)
    }

    fn render_to_document_and_page_data_with_resolver_and_report(
        &self,
        html: &str,
        page_templates: &[PageTemplate],
        resolver: &style::StyleResolver,
        report: Option<&mut GlyphCoverageReport>,
    ) -> Result<(Document, Option<PageDataContext>), FullBleedError> {
        self.render_to_document_and_page_data_with_resolver_and_report_at(
            0,
            html,
            page_templates,
            resolver,
            report,
        )
    }

    fn render_to_document_and_page_data_with_resolver(
        &self,
        html: &str,
        page_templates: &[PageTemplate],
        resolver: &style::StyleResolver,
    ) -> Result<(Document, Option<PageDataContext>), FullBleedError> {
        self.render_to_document_and_page_data_with_resolver_and_report(
            html,
            page_templates,
            resolver,
            None,
        )
    }

    fn render_to_document_with_resolver(
        &self,
        html: &str,
        page_templates: &[PageTemplate],
        resolver: &style::StyleResolver,
    ) -> Result<Document, FullBleedError> {
        self.render_to_document_and_page_data_with_resolver(html, page_templates, resolver)
            .map(|(doc, _page_data)| doc)
    }

    pub fn render_to_document(&self, html: &str, css: &str) -> Result<Document, FullBleedError> {
        let context = self.build_render_context(css, Some(0));
        self.render_to_document_with_resolver(html, &context.page_templates, &context.resolver)
    }

    pub fn render_to_buffer(&self, html: &str, css: &str) -> Result<Vec<u8>, FullBleedError> {
        let document = self.render_to_document(html, css)?;
        let bytes = pdf::document_to_pdf_with_metrics_and_registry_with_logs(
            &document,
            None,
            Some(self.font_registry.as_ref()),
            &self.pdf_options,
            self.debug.clone(),
            self.perf.clone(),
        )?;
        self.emit_debug_summary("render_to_buffer");
        Ok(bytes)
    }

    pub fn render_with_metrics(
        &self,
        html: &str,
        css: &str,
    ) -> Result<(Vec<u8>, DocumentMetrics), FullBleedError> {
        let context = self.build_render_context(css, Some(0));
        let mut metrics = DocumentMetrics::default();
        let layout = self.build_document_with_layout_strategy(
            0,
            html,
            &context.page_templates,
            &context.resolver,
            None,
        )?;
        let document = layout.document;
        metrics.total_render_ms = layout.layout_ms;
        metrics.pages = document
            .pages
            .iter()
            .enumerate()
            .map(|(idx, page)| PageMetrics {
                page_number: idx + 1,
                render_ms: 0.0,
                command_count: page.commands.len(),
                flowable_count: 0,
                content_bytes: 0,
            })
            .collect();

        let page_data_override = self
            .paginated_context
            .as_ref()
            .map(|spec| page_data::compute_page_data_context(&document, spec));

        let planned = plan::plan_document_with_overlay(
            0,
            &document,
            self.paginated_context.as_ref(),
            self.template_binding_spec.as_ref(),
            page_data_override.clone(),
            self.debug.clone(),
            self.jit_mode,
            Some(self.font_registry.as_ref()),
            |page_data| self.build_overlay_documents(&document, &context.resolver, page_data, None),
        )?;
        let _template_binding_count = planned
            .template_bindings
            .as_ref()
            .map(|v| v.len())
            .unwrap_or(0);
        let document = self.finalize_with_jit(
            0,
            document,
            planned.overlay,
            planned.background,
            planned.page_data.or(page_data_override),
            planned.plan,
        );
        let bytes = pdf::document_to_pdf_with_metrics_and_registry_with_logs(
            &document,
            Some(&mut metrics),
            Some(self.font_registry.as_ref()),
            &self.pdf_options,
            self.debug.clone(),
            self.perf.clone(),
        )?;
        self.emit_debug_summary("render_with_metrics");
        Ok((bytes, metrics))
    }

    pub fn render_with_page_data(
        &self,
        html: &str,
        css: &str,
    ) -> Result<(Vec<u8>, Option<PageDataContext>), FullBleedError> {
        let context = self.build_render_context(css, Some(0));
        let (document, page_data) = self.render_to_document_and_page_data_with_resolver(
            html,
            &context.page_templates,
            &context.resolver,
        )?;
        let bytes = pdf::document_to_pdf_with_metrics_and_registry_with_logs(
            &document,
            None,
            Some(self.font_registry.as_ref()),
            &self.pdf_options,
            self.debug.clone(),
            self.perf.clone(),
        )?;
        self.emit_debug_summary("render_with_page_data");
        Ok((bytes, page_data))
    }

    pub fn render_with_page_data_and_glyph_report(
        &self,
        html: &str,
        css: &str,
    ) -> Result<(Vec<u8>, Option<PageDataContext>, GlyphCoverageReport), FullBleedError> {
        let context = self.build_render_context(css, Some(0));
        let mut report = GlyphCoverageReport::default();
        let (document, page_data) = self
            .render_to_document_and_page_data_with_resolver_and_report(
                html,
                &context.page_templates,
                &context.resolver,
                Some(&mut report),
            )?;
        let bytes = pdf::document_to_pdf_with_metrics_and_registry_with_logs(
            &document,
            None,
            Some(self.font_registry.as_ref()),
            &self.pdf_options,
            self.debug.clone(),
            self.perf.clone(),
        )?;
        self.emit_debug_summary("render_with_page_data_and_glyph_report");
        Ok((bytes, page_data, report))
    }

    pub fn render_with_page_data_and_template_bindings(
        &self,
        html: &str,
        css: &str,
    ) -> Result<
        (
            Vec<u8>,
            Option<PageDataContext>,
            Option<Vec<PageBindingDecision>>,
        ),
        FullBleedError,
    > {
        let context = self.build_render_context(css, Some(0));
        let (document, page_data) = self.render_to_document_and_page_data_with_resolver(
            html,
            &context.page_templates,
            &context.resolver,
        )?;
        let template_bindings = match self.template_binding_spec.as_ref() {
            Some(spec) => Some(resolve_template_bindings_for_document(&document, spec)?),
            None => None,
        };
        let bytes = pdf::document_to_pdf_with_metrics_and_registry_with_logs(
            &document,
            None,
            Some(self.font_registry.as_ref()),
            &self.pdf_options,
            self.debug.clone(),
            self.perf.clone(),
        )?;
        self.emit_debug_summary("render_with_page_data_and_template_bindings");
        Ok((bytes, page_data, template_bindings))
    }

    pub fn render_with_page_data_and_template_bindings_and_glyph_report(
        &self,
        html: &str,
        css: &str,
    ) -> Result<
        (
            Vec<u8>,
            Option<PageDataContext>,
            Option<Vec<PageBindingDecision>>,
            GlyphCoverageReport,
        ),
        FullBleedError,
    > {
        let context = self.build_render_context(css, Some(0));
        let mut report = GlyphCoverageReport::default();
        let (document, page_data) = self
            .render_to_document_and_page_data_with_resolver_and_report(
                html,
                &context.page_templates,
                &context.resolver,
                Some(&mut report),
            )?;
        let template_bindings = match self.template_binding_spec.as_ref() {
            Some(spec) => Some(resolve_template_bindings_for_document(&document, spec)?),
            None => None,
        };
        let bytes = pdf::document_to_pdf_with_metrics_and_registry_with_logs(
            &document,
            None,
            Some(self.font_registry.as_ref()),
            &self.pdf_options,
            self.debug.clone(),
            self.perf.clone(),
        )?;
        self.emit_debug_summary("render_with_page_data_and_template_bindings_and_glyph_report");
        Ok((bytes, page_data, template_bindings, report))
    }

    pub fn render_with_glyph_report(
        &self,
        html: &str,
        css: &str,
    ) -> Result<(Vec<u8>, GlyphCoverageReport), FullBleedError> {
        let (bytes, _page_data, report) = self.render_with_page_data_and_glyph_report(html, css)?;
        Ok((bytes, report))
    }

    pub fn render_to_writer<W: std::io::Write>(
        &self,
        html: &str,
        css: &str,
        writer: &mut W,
    ) -> Result<usize, FullBleedError> {
        let context = self.build_render_context(css, Some(0));
        let document = self.render_to_document_with_resolver(
            html,
            &context.page_templates,
            &context.resolver,
        )?;
        let bytes_written = pdf::document_to_pdf_with_metrics_and_registry_to_writer_with_logs(
            &document,
            None,
            Some(self.font_registry.as_ref()),
            &self.pdf_options,
            writer,
            self.debug.clone(),
            self.perf.clone(),
        )?;
        self.emit_debug_summary("render_to_writer");
        Ok(bytes_written)
    }

    pub fn render_to_file(
        &self,
        html: &str,
        css: &str,
        path: impl AsRef<std::path::Path>,
    ) -> Result<usize, FullBleedError> {
        let mut file = std::fs::File::create(path)?;
        self.render_to_writer(html, css, &mut file)
    }

    pub fn render_image_pages(
        &self,
        html: &str,
        css: &str,
        dpi: u32,
    ) -> Result<Vec<Vec<u8>>, FullBleedError> {
        let context = self.build_render_context(css, Some(0));
        let document = self.render_to_document_with_resolver(
            html,
            &context.page_templates,
            &context.resolver,
        )?;
        let start = std::time::Instant::now();
        let pages = raster::document_to_png_pages(
            &document,
            dpi,
            Some(self.font_registry.as_ref()),
            self.pdf_options.shape_text,
        )?;
        if let Some(perf_logger) = self.perf.as_deref() {
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            perf_logger.log_span_ms("raster", Some(0), elapsed_ms);
            perf_logger.log_counts("raster", Some(0), &[("pages", pages.len() as u64)]);
        }
        self.emit_debug_summary("render_image_pages");
        Ok(pages)
    }

    pub fn render_image_pages_to_dir(
        &self,
        html: &str,
        css: &str,
        out_dir: impl AsRef<std::path::Path>,
        stem: &str,
        dpi: u32,
    ) -> Result<Vec<std::path::PathBuf>, FullBleedError> {
        let pages = self.render_image_pages(html, css, dpi)?;
        let out_dir = out_dir.as_ref();
        std::fs::create_dir_all(out_dir)?;
        let stem = if stem.trim().is_empty() {
            "render"
        } else {
            stem
        };

        let mut paths = Vec::with_capacity(pages.len());
        for (idx0, page_bytes) in pages.into_iter().enumerate() {
            let path = out_dir.join(format!("{stem}_page{}.png", idx0 + 1));
            std::fs::write(&path, page_bytes)?;
            paths.push(path);
        }
        Ok(paths)
    }

    pub fn render_finalized_pdf_image_pages(
        &self,
        pdf_path: impl AsRef<std::path::Path>,
        dpi: u32,
    ) -> Result<Vec<Vec<u8>>, FullBleedError> {
        let start = std::time::Instant::now();
        let pages = pdf_raster::pdf_path_to_png_pages(
            pdf_path.as_ref(),
            dpi,
            Some(self.font_registry.as_ref()),
            self.pdf_options.shape_text,
        )?;
        if let Some(perf_logger) = self.perf.as_deref() {
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            perf_logger.log_span_ms("raster.finalized_pdf", Some(0), elapsed_ms);
            perf_logger.log_counts(
                "raster.finalized_pdf",
                Some(0),
                &[("pages", pages.len() as u64)],
            );
        }
        self.emit_debug_summary("render_finalized_pdf_image_pages");
        Ok(pages)
    }

    pub fn render_finalized_pdf_image_pages_to_dir(
        &self,
        pdf_path: impl AsRef<std::path::Path>,
        out_dir: impl AsRef<std::path::Path>,
        stem: &str,
        dpi: u32,
    ) -> Result<Vec<std::path::PathBuf>, FullBleedError> {
        let pages = self.render_finalized_pdf_image_pages(pdf_path, dpi)?;
        let out_dir = out_dir.as_ref();
        std::fs::create_dir_all(out_dir)?;
        let stem = if stem.trim().is_empty() {
            "render"
        } else {
            stem
        };

        let mut paths = Vec::with_capacity(pages.len());
        for (idx0, page_bytes) in pages.into_iter().enumerate() {
            let path = out_dir.join(format!("{stem}_page{}.png", idx0 + 1));
            std::fs::write(&path, page_bytes)?;
            paths.push(path);
        }
        Ok(paths)
    }

    pub fn render_many_to_buffer(
        &self,
        html_list: &[String],
        css: &str,
    ) -> Result<Vec<u8>, FullBleedError> {
        let context = self.build_render_context(css, None);
        let mut documents = Vec::with_capacity(html_list.len());
        for (idx, html) in html_list.iter().enumerate() {
            let (doc, _page_data) = self
                .render_to_document_and_page_data_with_resolver_and_report_at(
                    idx,
                    html,
                    &context.page_templates,
                    &context.resolver,
                    None,
                )?;
            documents.push(doc);
        }
        let merged = merge_documents(documents)?;
        let bytes = pdf::document_to_pdf_with_metrics_and_registry_with_logs(
            &merged,
            None,
            Some(self.font_registry.as_ref()),
            &self.pdf_options,
            self.debug.clone(),
            self.perf.clone(),
        )?;
        self.emit_debug_summary("render_many_to_buffer");
        Ok(bytes)
    }

    pub fn render_many_to_writer<W: std::io::Write>(
        &self,
        html_list: &[String],
        css: &str,
        writer: &mut W,
    ) -> Result<usize, FullBleedError> {
        let context = self.build_render_context(css, None);
        let page_size = context
            .page_templates
            .get(0)
            .ok_or(FullBleedError::MissingPageTemplate)?
            .page_size;

        let mut pdf_stream = pdf::PdfStreamWriter::new(
            writer,
            page_size,
            Some(self.font_registry.as_ref()),
            self.pdf_options.clone(),
            self.debug.clone(),
            self.perf.clone(),
        )?;

        for (idx, html) in html_list.iter().enumerate() {
            let (doc, _page_data) = self
                .render_to_document_and_page_data_with_resolver_and_report_at(
                    idx,
                    html,
                    &context.page_templates,
                    &context.resolver,
                    None,
                )?;
            pdf_stream.add_document(idx, &doc)?;
        }
        let bytes_written = pdf_stream.finish()?;
        self.emit_debug_summary("render_many_to_writer");
        Ok(bytes_written)
    }

    pub fn render_many_to_file(
        &self,
        html_list: &[String],
        css: &str,
        path: impl AsRef<std::path::Path>,
    ) -> Result<usize, FullBleedError> {
        let mut file = std::fs::File::create(path)?;
        self.render_many_to_writer(html_list, css, &mut file)
    }

    pub fn render_many_to_buffer_with_css(
        &self,
        jobs: &[(String, String)],
    ) -> Result<Vec<u8>, FullBleedError> {
        let mut documents = Vec::with_capacity(jobs.len());
        for (idx, (html, css)) in jobs.iter().enumerate() {
            let context = self.build_render_context(css, Some(idx));
            let (doc, _page_data) = self
                .render_to_document_and_page_data_with_resolver_and_report_at(
                    idx,
                    html,
                    &context.page_templates,
                    &context.resolver,
                    None,
                )?;
            documents.push(doc);
        }
        let merged = merge_documents(documents)?;
        Ok(pdf::document_to_pdf_with_metrics_and_registry_with_logs(
            &merged,
            None,
            Some(self.font_registry.as_ref()),
            &self.pdf_options,
            self.debug.clone(),
            self.perf.clone(),
        )?)
    }

    pub fn render_many_to_writer_with_css<W: std::io::Write>(
        &self,
        jobs: &[(String, String)],
        writer: &mut W,
    ) -> Result<usize, FullBleedError> {
        let page_size = jobs
            .get(0)
            .and_then(|(_, css)| {
                let context = self.build_render_context(css, Some(0));
                context.page_templates.get(0).map(|t| t.page_size)
            })
            .unwrap_or(self.default_page_size);

        let mut pdf_stream = pdf::PdfStreamWriter::new(
            writer,
            page_size,
            Some(self.font_registry.as_ref()),
            self.pdf_options.clone(),
            self.debug.clone(),
            self.perf.clone(),
        )?;

        for (idx, (html, css)) in jobs.iter().enumerate() {
            let context = self.build_render_context(css, Some(idx));
            let (doc, _page_data) = self
                .render_to_document_and_page_data_with_resolver_and_report_at(
                    idx,
                    html,
                    &context.page_templates,
                    &context.resolver,
                    None,
                )?;
            pdf_stream.add_document(idx, &doc)?;
        }

        Ok(pdf_stream.finish()?)
    }

    pub fn render_many_to_file_with_css(
        &self,
        jobs: &[(String, String)],
        path: impl AsRef<std::path::Path>,
    ) -> Result<usize, FullBleedError> {
        let mut file = std::fs::File::create(path)?;
        self.render_many_to_writer_with_css(jobs, &mut file)
    }

    // Parallel batch rendering: build documents in parallel, then merge in input order.
    pub fn render_many_to_buffer_parallel(
        &self,
        html_list: &[String],
        css: &str,
    ) -> Result<Vec<u8>, FullBleedError> {
        use rayon::prelude::*;

        let context = self.build_render_context(css, None);
        let mut results: Vec<(usize, Result<Document, FullBleedError>)> = html_list
            .par_iter()
            .enumerate()
            .map(|(idx, html)| {
                let res = self
                    .render_to_document_and_page_data_with_resolver_and_report_at(
                        idx,
                        html,
                        &context.page_templates,
                        &context.resolver,
                        None,
                    )
                    .map(|(doc, _page_data)| doc);
                (idx, res)
            })
            .collect();
        results.sort_by_key(|(idx, _)| *idx);

        let mut documents = Vec::with_capacity(results.len());
        for (_, res) in results {
            documents.push(res?);
        }

        let merged = merge_documents(documents)?;
        Ok(pdf::document_to_pdf_with_metrics_and_registry_with_logs(
            &merged,
            None,
            Some(self.font_registry.as_ref()),
            &self.pdf_options,
            self.debug.clone(),
            self.perf.clone(),
        )?)
    }

    pub fn render_many_to_buffer_parallel_with_page_data(
        &self,
        html_list: &[String],
        css: &str,
    ) -> Result<(Vec<u8>, Vec<Option<PageDataContext>>), FullBleedError> {
        use rayon::prelude::*;

        let context = self.build_render_context(css, None);
        let mut results: Vec<(
            usize,
            Result<(Document, Option<PageDataContext>), FullBleedError>,
        )> = html_list
            .par_iter()
            .enumerate()
            .map(|(idx, html)| {
                let res = self.render_to_document_and_page_data_with_resolver_and_report_at(
                    idx,
                    html,
                    &context.page_templates,
                    &context.resolver,
                    None,
                );
                (idx, res)
            })
            .collect();
        results.sort_by_key(|(idx, _)| *idx);

        let mut documents = Vec::with_capacity(results.len());
        let mut page_data_list = Vec::with_capacity(results.len());
        for (_, res) in results {
            let (doc, page_data) = res?;
            documents.push(doc);
            page_data_list.push(page_data);
        }

        let merged = merge_documents(documents)?;
        let bytes = pdf::document_to_pdf_with_metrics_and_registry_with_logs(
            &merged,
            None,
            Some(self.font_registry.as_ref()),
            &self.pdf_options,
            self.debug.clone(),
            self.perf.clone(),
        )?;
        Ok((bytes, page_data_list))
    }

    pub fn render_many_to_writer_parallel<W: std::io::Write>(
        &self,
        html_list: &[String],
        css: &str,
        writer: &mut W,
    ) -> Result<usize, FullBleedError> {
        let perf = self.perf.as_deref();
        let t_total = std::time::Instant::now();
        let context = self.build_render_context(css, None);
        let page_size = context
            .page_templates
            .get(0)
            .ok_or(FullBleedError::MissingPageTemplate)?
            .page_size;

        // Streaming writer: avoids holding a merged mega-Document or a Vec<String> of PDF objects.
        let mut pdf_stream = pdf::PdfStreamWriter::new(
            writer,
            page_size,
            Some(self.font_registry.as_ref()),
            self.pdf_options.clone(),
            self.debug.clone(),
            self.perf.clone(),
        )?;

        if matches!(self.jit_mode, JitMode::PlanAndReplay) {
            use rayon::prelude::*;
            use std::collections::BTreeMap;
            use std::path::PathBuf;
            use std::sync::mpsc;
            use std::thread;

            let n = html_list.len();
            if n == 0 {
                return Err(FullBleedError::EmptyDocumentSet);
            }

            let spill_enabled = std::env::var("FULLBLEED_JIT_SPILL")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            let spill_dir = std::env::var("FULLBLEED_JIT_SPILL_DIR")
                .ok()
                .map(PathBuf::from)
                .or_else(|| {
                    if spill_enabled {
                        Some(std::env::temp_dir().join("fullbleed_spill"))
                    } else {
                        None
                    }
                });
            let spill_store = match spill_dir {
                Some(dir) => Some(Arc::new(
                    spill::SpillStore::new(dir).map_err(FullBleedError::Io)?,
                )),
                None => None,
            };

            // Bound in-flight documents to keep memory stable.
            let buffer_cap = (rayon::current_num_threads().max(1) * 4).min(256);
            let (tx, rx) =
                mpsc::sync_channel::<(usize, Result<Document, FullBleedError>)>(buffer_cap);
            let mut render_error: Option<FullBleedError> = None;

            thread::scope(|scope| {
                let rx = rx;
                let spill_store = spill_store.as_ref();

                // Producer: plan + paint in parallel.
                scope.spawn(|| {
                    html_list
                        .par_iter()
                        .enumerate()
                        .for_each_with(tx, |tx, (idx, html)| {
                            let res = self
                                .render_to_planned_doc_with_resolver_and_report_at(
                                    idx,
                                    html,
                                    &context.page_templates,
                                    &context.resolver,
                                    None,
                                )
                                .and_then(|planned| {
                                    let plan = planned.plan.ok_or_else(|| {
                                        FullBleedError::Io(std::io::Error::new(
                                            std::io::ErrorKind::Other,
                                            "jit plan missing in PlanAndReplay mode",
                                        ))
                                    })?;
                                    let ops = jit::paint_plan_parallel(&plan, self.debug.clone());
                                    Ok(jit::ops_to_document(plan.page_size, ops))
                                });
                            let _ = tx.send((idx, res));
                        });
                });

                // Consumer: write in order with backpressure.
                enum PendingDoc {
                    InMemory(Document),
                    Spilled(PathBuf),
                }
                let mut pending: BTreeMap<usize, PendingDoc> = BTreeMap::new();
                let mut next_idx: usize = 0;
                let spill_threshold = buffer_cap;

                while next_idx < n {
                    match rx.recv() {
                        Ok((idx, res)) => match res {
                            Ok(doc) => {
                                let entry = if let Some(store) = spill_store {
                                    if pending.len() >= spill_threshold {
                                        match store.spill(&doc) {
                                            Ok(path) => PendingDoc::Spilled(path),
                                            Err(err) => {
                                                render_error = Some(FullBleedError::Io(err));
                                                break;
                                            }
                                        }
                                    } else {
                                        PendingDoc::InMemory(doc)
                                    }
                                } else {
                                    PendingDoc::InMemory(doc)
                                };
                                pending.insert(idx, entry);

                                while let Some(entry) = pending.remove(&next_idx) {
                                    let doc = match entry {
                                        PendingDoc::InMemory(doc) => doc,
                                        PendingDoc::Spilled(path) => {
                                            if let Some(store) = spill_store {
                                                match store.load(&path) {
                                                    Ok(doc) => doc,
                                                    Err(err) => {
                                                        render_error =
                                                            Some(FullBleedError::Io(err));
                                                        break;
                                                    }
                                                }
                                            } else {
                                                render_error =
                                                    Some(FullBleedError::Io(std::io::Error::new(
                                                        std::io::ErrorKind::Other,
                                                        "spill requested without spill store",
                                                    )));
                                                break;
                                            }
                                        }
                                    };
                                    if let Err(e) = pdf_stream.add_document(next_idx, &doc) {
                                        render_error = Some(FullBleedError::Io(e));
                                        break;
                                    }
                                    next_idx += 1;
                                }
                                if render_error.is_some() {
                                    break;
                                }
                            }
                            Err(e) => {
                                render_error = Some(e);
                                break;
                            }
                        },
                        Err(_) => {
                            render_error = Some(FullBleedError::Io(std::io::Error::new(
                                std::io::ErrorKind::Other,
                                "jit batch channel closed unexpectedly",
                            )));
                            break;
                        }
                    }
                }
            });

            if let Some(err) = render_error {
                return Err(err);
            }

            if let Some(store) = spill_store {
                let (files, bytes) = store.metrics();
                if let Some(logger) = self.debug.as_deref() {
                    let json = format!(
                        "{{\"type\":\"jit.spill\",\"files\":{},\"bytes\":{}}}",
                        files, bytes
                    );
                    logger.log_json(&json);
                }
            }

            return Ok(pdf_stream.finish()?);
        }

        // Pipeline: render HTML->Document on Rayon threads while a single writer thread
        // serializes to PDF in input order. This keeps memory bounded and keeps CPU busy.
        use rayon::prelude::*;
        use std::collections::BTreeMap;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::mpsc;
        use std::thread;
        use std::time::{Duration, Instant};

        let n = html_list.len();
        if n == 0 {
            return Err(FullBleedError::EmptyDocumentSet);
        }

        // Bound the number of in-flight Documents so we don’t blow up memory on huge batches.
        let buffer_cap = (rayon::current_num_threads().max(1) * 4).min(256);
        let (tx, rx) = mpsc::sync_channel::<(usize, Result<Document, FullBleedError>)>(buffer_cap);

        let mut render_error: Option<FullBleedError> = None;
        let timing_enabled = std::env::var("FULLBLEED_TIMING")
            .map(|v| v == "1")
            .unwrap_or(false);
        let mut recv_wait = Duration::ZERO;
        let mut write_time = Duration::ZERO;
        let send_wait = Arc::new(AtomicU64::new(0));
        let send_count = Arc::new(AtomicU64::new(0));
        let send_blocked = Arc::new(AtomicU64::new(0));
        let mut max_pending: usize = 0;

        thread::scope(|scope| {
            // Move the receiver into this scope so it gets dropped on early exit,
            // which unblocks producers waiting on a full sync_channel.
            let rx = rx;

            // Producer: render in parallel.
            scope.spawn(|| {
                let send_wait = send_wait.clone();
                let send_count = send_count.clone();
                let send_blocked = send_blocked.clone();
                html_list
                    .par_iter()
                    .enumerate()
                    .for_each_with(tx, |tx, (idx, html)| {
                        let res = self
                            .render_to_document_and_page_data_with_resolver_and_report_at(
                                idx,
                                html,
                                &context.page_templates,
                                &context.resolver,
                                None,
                            )
                            .map(|(doc, _page_data)| doc);
                        // If the receiver is gone (error), stop pushing.
                        let t_send = Instant::now();
                        let _ = tx.send((idx, res));
                        let waited = t_send.elapsed();
                        send_wait.fetch_add(waited.as_nanos() as u64, Ordering::Relaxed);
                        send_count.fetch_add(1, Ordering::Relaxed);
                        if waited > Duration::from_millis(1) {
                            send_blocked.fetch_add(1, Ordering::Relaxed);
                        }
                    });
            });

            // Consumer: write in order.
            let mut pending: BTreeMap<usize, Document> = BTreeMap::new();
            let mut next_idx: usize = 0;

            while next_idx < n {
                let t0 = Instant::now();
                let msg = rx.recv();
                recv_wait += t0.elapsed();
                match msg {
                    Ok((idx, res)) => match res {
                        Ok(doc) => {
                            pending.insert(idx, doc);
                            if pending.len() > max_pending {
                                max_pending = pending.len();
                            }
                            while let Some(doc) = pending.remove(&next_idx) {
                                let t1 = Instant::now();
                                if let Err(e) = pdf_stream.add_document(next_idx, &doc) {
                                    render_error = Some(FullBleedError::Io(e));
                                    break;
                                }
                                write_time += t1.elapsed();
                                next_idx += 1;
                            }
                        }
                        Err(e) => {
                            render_error = Some(e);
                            break;
                        }
                    },
                    Err(err) => {
                        render_error = Some(FullBleedError::Io(std::io::Error::new(
                            std::io::ErrorKind::BrokenPipe,
                            err.to_string(),
                        )));
                        break;
                    }
                }

                if render_error.is_some() {
                    break;
                }
            }
        });

        if let Some(e) = render_error {
            return Err(e);
        }

        if timing_enabled {
            eprintln!(
                "[fullbleed] parallel pipeline: recv_wait={:.2?} write_time={:.2?}",
                recv_wait, write_time
            );
        }

        let bytes_written = pdf_stream.finish()?;
        if let Some(perf_logger) = perf {
            perf_logger.log_span_ms("batch.recv_wait", None, recv_wait.as_secs_f64() * 1000.0);
            let send_wait_ms = send_wait.load(Ordering::Relaxed) as f64 / 1_000_000.0;
            perf_logger.log_span_ms("batch.send_wait", None, send_wait_ms);
            perf_logger.log_span_ms("batch.write_time", None, write_time.as_secs_f64() * 1000.0);
            perf_logger.log_span_ms(
                "batch.total",
                None,
                t_total.elapsed().as_secs_f64() * 1000.0,
            );
            perf_logger.log_counts(
                "batch",
                None,
                &[
                    ("bytes", bytes_written as u64),
                    ("docs", n as u64),
                    ("buffer_cap", buffer_cap as u64),
                    ("send_count", send_count.load(Ordering::Relaxed)),
                    ("send_blocked", send_blocked.load(Ordering::Relaxed)),
                    ("pending_max", max_pending as u64),
                ],
            );
        }

        Ok(bytes_written)
    }

    pub fn render_many_to_writer_parallel_with_page_data<W: std::io::Write>(
        &self,
        html_list: &[String],
        css: &str,
        writer: &mut W,
    ) -> Result<(usize, Vec<Option<PageDataContext>>), FullBleedError> {
        use rayon::prelude::*;

        let context = self.build_render_context(css, None);
        let mut results: Vec<(
            usize,
            Result<(Document, Option<PageDataContext>), FullBleedError>,
        )> = html_list
            .par_iter()
            .enumerate()
            .map(|(idx, html)| {
                let res = self.render_to_document_and_page_data_with_resolver_and_report_at(
                    idx,
                    html,
                    &context.page_templates,
                    &context.resolver,
                    None,
                );
                (idx, res)
            })
            .collect();
        results.sort_by_key(|(idx, _)| *idx);

        let mut documents = Vec::with_capacity(results.len());
        let mut page_data_list = Vec::with_capacity(results.len());
        for (_, res) in results {
            let (doc, page_data) = res?;
            documents.push(doc);
            page_data_list.push(page_data);
        }

        let merged = merge_documents(documents)?;
        let bytes_written = pdf::document_to_pdf_with_metrics_and_registry_to_writer_with_logs(
            &merged,
            None,
            Some(self.font_registry.as_ref()),
            &self.pdf_options,
            writer,
            self.debug.clone(),
            self.perf.clone(),
        )?;
        Ok((bytes_written, page_data_list))
    }

    pub fn render_many_to_file_parallel(
        &self,
        html_list: &[String],
        css: &str,
        path: impl AsRef<std::path::Path>,
    ) -> Result<usize, FullBleedError> {
        let mut file = std::fs::File::create(path)?;
        self.render_many_to_writer_parallel(html_list, css, &mut file)
    }

    pub fn render_many_to_file_parallel_with_page_data(
        &self,
        html_list: &[String],
        css: &str,
        path: impl AsRef<std::path::Path>,
    ) -> Result<(usize, Vec<Option<PageDataContext>>), FullBleedError> {
        let mut file = std::fs::File::create(path)?;
        self.render_many_to_writer_parallel_with_page_data(html_list, css, &mut file)
    }
}

impl FullBleedBuilder {
    pub fn new() -> Self {
        Self {
            page_size: Size::a4(),
            margins: Margins::all(36.0),
            page_size_explicit: false,
            margins_explicit: false,
            font_dirs: Vec::new(),
            font_files: Vec::new(),
            pdf_options: PdfOptions::default(),
            svg_form_xobjects: false,
            svg_raster_fallback: cfg!(feature = "svg_raster"),
            unicode_metrics: true,
            debug_path: None,
            perf_enabled: false,
            perf_path: None,
            jit_mode: JitMode::Off,
            layout_strategy: LayoutStrategy::Eager,
            accept_lazy_layout_cost: false,
            lazy_max_passes: 4,
            lazy_budget_ms: 50.0,
            page_header: None,
            page_header_html: None,
            page_footer: None,
            paginated_context: None,
            template_binding_spec: None,
            page_margins: std::collections::BTreeMap::new(),
            watermark: None,
            asset_bundle: AssetBundle::default(),
        }
    }

    pub fn page_size(mut self, size: Size) -> Self {
        self.page_size = size;
        self.page_size_explicit = true;
        self
    }

    pub fn margins(mut self, margins: Margins) -> Self {
        self.margins = margins;
        self.margins_explicit = true;
        self
    }

    pub fn margin_all(mut self, value: f32) -> Self {
        self.margins = Margins::all(value);
        self.margins_explicit = true;
        self
    }

    pub fn svg_form_xobjects(mut self, enabled: bool) -> Self {
        self.svg_form_xobjects = enabled;
        self
    }

    // Rasterize SVGs that use unsupported features (e.g. <text>, filters, masks).
    // Requires the optional "svg_raster" feature at build time.
    pub fn svg_raster_fallback(mut self, enabled: bool) -> Self {
        self.svg_raster_fallback = enabled;
        self
    }

    // Per-page margins for page template selection by page index.
    //
    // Selection rule:
    // - If you set page 1 and page 2 margins, those are used for pages 1 and 2.
    // - For page >= max specified page, the last specified margin repeats ("page_n").
    pub fn page_margin(mut self, page_number: usize, margins: Margins) -> Self {
        if page_number >= 1 {
            self.page_margins.insert(page_number, margins);
            self.margins_explicit = true;
        }
        self
    }

    pub fn register_font_dir(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.font_dirs.push(path.into());
        self
    }

    pub fn register_font_file(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.font_files.push(path.into());
        self
    }

    // When enabled (default), identical images are embedded once and reused via a single
    // PDF XObject resource. Turning this off can be useful for debugging or compatibility.
    pub fn reuse_xobjects(mut self, enabled: bool) -> Self {
        self.pdf_options.reuse_xobjects = enabled;
        self
    }

    // Toggle Unicode text support in PDF output (CID/Identity-H + ToUnicode).
    // When disabled, fonts are emitted as WinAnsi for maximum speed.
    pub fn unicode_support(mut self, enabled: bool) -> Self {
        self.pdf_options.unicode_support = enabled;
        self
    }

    // Toggle shaping for complex scripts. Disabling skips rustybuzz shaping and uses
    // direct codepoint->gid mapping for Identity-H fonts.
    pub fn shape_text(mut self, enabled: bool) -> Self {
        self.pdf_options.shape_text = enabled;
        self
    }

    // Batch JIT pipeline mode. Off by default.
    pub fn jit_mode(mut self, mode: JitMode) -> Self {
        self.jit_mode = mode;
        self
    }

    pub fn layout_strategy(mut self, strategy: LayoutStrategy) -> Self {
        self.layout_strategy = strategy;
        self
    }

    pub fn lazy_layout(mut self, enabled: bool) -> Self {
        self.layout_strategy = if enabled {
            LayoutStrategy::Lazy
        } else {
            LayoutStrategy::Eager
        };
        self
    }

    pub fn accept_lazy_layout_cost(mut self, accepted: bool) -> Self {
        self.accept_lazy_layout_cost = accepted;
        self
    }

    pub fn lazy_layout_limits(mut self, max_passes: usize, budget_ms: f64) -> Self {
        self.lazy_max_passes = max_passes;
        self.lazy_budget_ms = budget_ms;
        self
    }

    // PDF conformance/profile toggles (e.g. Tagged).
    pub fn pdf_profile(mut self, profile: PdfProfile) -> Self {
        self.pdf_options.pdf_profile = profile;
        self
    }

    // Output intent for conformance profiles that require device-independent output conditions.
    pub fn output_intent(mut self, intent: OutputIntent) -> Self {
        self.pdf_options.output_intent = Some(intent);
        self
    }

    pub fn clear_output_intent(mut self) -> Self {
        self.pdf_options.output_intent = None;
        self
    }

    // PDF version selector (default: PDF 1.7).
    pub fn pdf_version(mut self, version: PdfVersion) -> Self {
        self.pdf_options.pdf_version = version;
        self
    }

    // Output colorspace for vector paints (fills/strokes/shadings).
    pub fn color_space(mut self, space: ColorSpace) -> Self {
        self.pdf_options.color_space = space;
        self
    }

    // Document language (BCP-47, e.g. "en-US") for accessibility metadata.
    pub fn document_lang(mut self, lang: impl Into<String>) -> Self {
        self.pdf_options.document_lang = Some(lang.into());
        self
    }

    // Document title for metadata (Info + XMP).
    pub fn document_title(mut self, title: impl Into<String>) -> Self {
        self.pdf_options.document_title = Some(title.into());
        self
    }

    // Toggle Unicode-aware layout measurements (rustybuzz-based).
    // When disabled, layout uses basic metrics for speed.
    pub fn unicode_metrics(mut self, enabled: bool) -> Self {
        self.unicode_metrics = enabled;
        self
    }

    // Enable debug logging to a JSONL file for CSS/layout inspection.
    pub fn debug_log(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.debug_path = Some(path.into());
        self
    }

    // Enable performance logging to a JSONL file for timing/counter inspection.
    pub fn perf_log(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.perf_enabled = true;
        self.perf_path = Some(path.into());
        self
    }

    // Toggle performance logging (uses default file when enabled and no path is set).
    pub fn perf_enabled(mut self, enabled: bool) -> Self {
        self.perf_enabled = enabled;
        self
    }

    // Header text templates. Placeholders:
    // - {page}: 1-based page number within this record/document
    // - {pages}: total pages within this record/document
    // - {sum:key} / {total:key} when paginated_context is enabled
    //
    // Coordinates are in PDF points in our top-left-origin space.
    pub fn page_header(
        mut self,
        first: Option<String>,
        each: Option<String>,
        last: Option<String>,
        x: f32,
        y_from_top: f32,
        font_name: impl Into<String>,
        font_size: f32,
        color: Color,
    ) -> Self {
        self.page_header = Some(PageHeaderSpec {
            first,
            each,
            last,
            x: Pt::from_f32(x),
            y_from_top: Pt::from_f32(y_from_top),
            font_name: font_name.into(),
            font_size: Pt::from_f32(font_size),
            color,
        });
        self
    }

    pub fn page_header_html(
        mut self,
        first: Option<String>,
        each: Option<String>,
        last: Option<String>,
        x: f32,
        y_from_top: f32,
        width: f32,
        height: f32,
    ) -> Self {
        self.page_header_html = Some(PageHeaderHtmlSpec {
            first,
            each,
            last,
            x: Pt::from_f32(x),
            y_from_top: Pt::from_f32(y_from_top),
            width: Pt::from_f32(width),
            height: Pt::from_f32(height),
        });
        self
    }

    // Footer text templates. Placeholders:
    // - {page}: 1-based page number within this record/document
    // - {pages}: total pages within this record/document
    //
    // Coordinates are in PDF points in our top-left-origin space.
    pub fn page_footer(
        mut self,
        first: Option<String>,
        each: Option<String>,
        last: Option<String>,
        x: f32,
        y_from_bottom: f32,
        font_name: impl Into<String>,
        font_size: f32,
        color: Color,
    ) -> Self {
        self.page_footer = Some(PageFooterSpec {
            first,
            each,
            last,
            x: Pt::from_f32(x),
            y_from_bottom: Pt::from_f32(y_from_bottom),
            font_name: font_name.into(),
            font_size: Pt::from_f32(font_size),
            color,
        });
        self
    }

    pub fn watermark(mut self, spec: WatermarkSpec) -> Self {
        self.watermark = Some(spec);
        self
    }

    pub fn watermark_semantics(mut self, semantics: WatermarkSemantics) -> Self {
        if let Some(spec) = self.watermark.as_mut() {
            spec.semantics = semantics;
        }
        self
    }

    pub fn watermark_text(mut self, text: impl Into<String>) -> Self {
        self.watermark = Some(WatermarkSpec::text(text));
        self
    }

    pub fn watermark_html(mut self, html: impl Into<String>) -> Self {
        self.watermark = Some(WatermarkSpec::html(html));
        self
    }

    pub fn watermark_image(mut self, path: impl Into<String>) -> Self {
        self.watermark = Some(WatermarkSpec::image(path));
        self
    }

    pub fn paginated_context(mut self, spec: PaginatedContextSpec) -> Self {
        self.paginated_context = Some(spec);
        self
    }

    pub fn template_binding_spec(mut self, spec: TemplateBindingSpec) -> Self {
        self.template_binding_spec = Some(spec);
        self
    }

    pub fn register_bundle(mut self, bundle: AssetBundle) -> Self {
        self.asset_bundle = bundle;
        self
    }

    pub fn build(self) -> Result<FullBleed, FullBleedError> {
        if self.layout_strategy == LayoutStrategy::Lazy && !self.accept_lazy_layout_cost {
            return Err(FullBleedError::InvalidConfiguration(
                "layout_strategy=lazy requires accept_lazy_layout_cost(true)".to_string(),
            ));
        }
        if self.layout_strategy == LayoutStrategy::Lazy && self.lazy_max_passes < 2 {
            return Err(FullBleedError::InvalidConfiguration(
                "layout_strategy=lazy requires lazy_max_passes >= 2".to_string(),
            ));
        }
        if self.layout_strategy == LayoutStrategy::Lazy
            && (!self.lazy_budget_ms.is_finite() || self.lazy_budget_ms <= 0.0)
        {
            return Err(FullBleedError::InvalidConfiguration(
                "layout_strategy=lazy requires lazy_budget_ms > 0".to_string(),
            ));
        }
        validate_pdf_options(&self.pdf_options)?;
        let mut registry = FontRegistry::new();
        registry.set_use_full_unicode_metrics(self.unicode_metrics);
        for dir in &self.font_dirs {
            registry.register_dir(dir);
        }
        for file in &self.font_files {
            registry.register_file(file);
        }
        for asset in self.asset_bundle.font_assets() {
            registry.register_bytes(asset.data.clone(), Some(&asset.name))?;
        }
        let asset_css = self.asset_bundle.css_text();
        let debug = if let Some(path) = self.debug_path {
            Some(Arc::new(DebugLogger::new(path)?))
        } else {
            None
        };
        let perf = if self.perf_enabled || self.perf_path.is_some() {
            let path = self
                .perf_path
                .unwrap_or_else(|| std::path::PathBuf::from("fullbleed_perf.log"));
            Some(Arc::new(PerfLogger::new(path)?))
        } else {
            None
        };
        Ok(FullBleed {
            default_page_size: self.page_size,
            default_margins: self.margins,
            page_margins: self.page_margins,
            page_size_explicit: self.page_size_explicit,
            margins_explicit: self.margins_explicit,
            font_registry: Arc::new(registry),
            pdf_options: self.pdf_options,
            svg_form_xobjects: self.svg_form_xobjects,
            svg_raster_fallback: self.svg_raster_fallback,
            debug,
            perf,
            jit_mode: self.jit_mode,
            layout_strategy: self.layout_strategy,
            lazy_max_passes: self.lazy_max_passes,
            lazy_budget_ms: self.lazy_budget_ms,
            page_header: self.page_header,
            page_header_html: self.page_header_html,
            page_footer: self.page_footer,
            paginated_context: self.paginated_context,
            template_binding_spec: self.template_binding_spec,
            watermark: self.watermark,
            asset_css,
        })
    }
}

fn build_page_templates(
    page_size: Size,
    base_margins: Margins,
    page_margins: &std::collections::BTreeMap<usize, Margins>,
) -> Vec<PageTemplate> {
    let base_margins = base_margins.quantized();
    let content_width = (page_size.width - base_margins.left - base_margins.right).max(Pt::ZERO);
    let content_height = (page_size.height - base_margins.top - base_margins.bottom).max(Pt::ZERO);
    let frame_rect = Rect {
        x: base_margins.left,
        y: base_margins.top,
        width: content_width,
        height: content_height,
    }
    .quantized();

    let mut templates: Vec<PageTemplate> = Vec::new();
    if page_margins.is_empty() {
        templates.push(PageTemplate::new("Page1", page_size).with_frame(frame_rect));
        return templates;
    }

    let max_page = *page_margins.keys().max().unwrap_or(&1);
    for page_number in 1..=max_page {
        let margins = page_margins
            .get(&page_number)
            .copied()
            .unwrap_or(base_margins)
            .quantized();
        let content_width = (page_size.width - margins.left - margins.right).max(Pt::ZERO);
        let content_height = (page_size.height - margins.top - margins.bottom).max(Pt::ZERO);
        let rect = Rect {
            x: margins.left,
            y: margins.top,
            width: content_width,
            height: content_height,
        }
        .quantized();
        templates.push(PageTemplate::new(format!("Page{page_number}"), page_size).with_frame(rect));
    }
    templates
}

impl Default for FullBleedBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub fn merge_documents(documents: Vec<Document>) -> Result<Document, FullBleedError> {
    if documents.is_empty() {
        return Err(FullBleedError::EmptyDocumentSet);
    }
    let mut iter = documents.into_iter();
    let first = iter.next().expect("checked empty document set");
    let page_size = first.page_size;
    let mut pages = first.pages;

    for doc in iter {
        if doc.page_size != page_size {
            return Err(FullBleedError::InconsistentPageSize);
        }
        pages.extend(doc.pages);
    }

    Ok(Document { page_size, pages })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flowable::{BorderCollapseMode, BorderSpec, TableCell, TextAlign, VerticalAlign};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn table_cell(text: &str) -> TableCell {
        table_cell_with_border(text, EdgeSizes::zero(), Color::BLACK)
    }

    fn table_cell_with_border(text: &str, widths: EdgeSizes, color: Color) -> TableCell {
        TableCell::new(
            text.to_string(),
            TextStyle::default(),
            TextAlign::Left,
            VerticalAlign::Top,
            EdgeSizes::zero(),
            None,
            BorderSpec { widths, color },
            None,
            Some(Arc::<str>::from("TD")),
            None,
            1,
            Pt::from_f32(12.0),
            None,
            false,
            false,
        )
    }

    fn abs(v: f32) -> LengthSpec {
        LengthSpec::Absolute(Pt::from_f32(v))
    }

    fn page_contains_text(page: &Page, needle: &str) -> bool {
        page.commands.iter().any(|cmd| match cmd {
            Command::DrawString { text, .. } => text.contains(needle),
            _ => false,
        })
    }

    fn empty_document(page_count: usize) -> Document {
        Document {
            page_size: Size::a4(),
            pages: (0..page_count)
                .map(|_| Page {
                    commands: Vec::new(),
                })
                .collect(),
        }
    }

    fn temp_log_path(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "fullbleed_{tag}_{}_{}.jsonl",
            std::process::id(),
            nanos
        ))
    }

    fn repo_font_path(file_name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("python")
            .join("fullbleed_assets")
            .join("fonts")
            .join(file_name)
    }

    fn count_token(haystack: &[u8], token: &[u8]) -> usize {
        if token.is_empty() || haystack.len() < token.len() {
            return 0;
        }
        haystack
            .windows(token.len())
            .filter(|window| *window == token)
            .count()
    }

    #[test]
    fn pdfx4_builder_requires_output_intent() {
        let err = match FullBleed::builder().pdf_profile(PdfProfile::PdfX4).build() {
            Ok(_) => panic!("pdfx4 should fail without output intent"),
            Err(err) => err,
        };
        assert!(matches!(err, FullBleedError::InvalidConfiguration(_)));
        assert!(err.to_string().contains("output_intent"));
    }

    #[test]
    fn lazy_layout_requires_explicit_cost_acceptance() {
        let err = match FullBleed::builder()
            .layout_strategy(LayoutStrategy::Lazy)
            .build()
        {
            Ok(_) => panic!("lazy layout should require explicit opt-in"),
            Err(err) => err,
        };
        assert!(matches!(err, FullBleedError::InvalidConfiguration(_)));
        assert!(err.to_string().contains("accept_lazy_layout_cost"));
    }

    #[test]
    fn lazy_layout_limits_are_validated() {
        let err = match FullBleed::builder()
            .layout_strategy(LayoutStrategy::Lazy)
            .accept_lazy_layout_cost(true)
            .lazy_layout_limits(1, 50.0)
            .build()
        {
            Ok(_) => panic!("lazy max passes must be >= 2"),
            Err(err) => err,
        };
        assert!(matches!(err, FullBleedError::InvalidConfiguration(_)));
        assert!(err.to_string().contains("lazy_max_passes"));

        let err = match FullBleed::builder()
            .layout_strategy(LayoutStrategy::Lazy)
            .accept_lazy_layout_cost(true)
            .lazy_layout_limits(4, 0.0)
            .build()
        {
            Ok(_) => panic!("lazy budget must be positive"),
            Err(err) => err,
        };
        assert!(matches!(err, FullBleedError::InvalidConfiguration(_)));
        assert!(err.to_string().contains("lazy_budget_ms"));
    }

    #[test]
    fn lazy_layout_configuration_builds_with_opt_in() {
        FullBleed::builder()
            .layout_strategy(LayoutStrategy::Lazy)
            .accept_lazy_layout_cost(true)
            .lazy_layout_limits(4, 50.0)
            .build()
            .expect("valid lazy config should build");
    }

    #[test]
    fn batch_writer_file_and_parallel_paths_dedupe_embedded_fonts() {
        let inter_path = repo_font_path("Inter-Variable.ttf");
        let inter_bytes = std::fs::read(&inter_path).expect("read inter");

        let mut engine = FullBleed::builder().build().expect("engine");
        let font_name = {
            let registry = Arc::get_mut(&mut engine.font_registry).expect("unique registry");
            registry
                .register_bytes(inter_bytes, Some(inter_path.to_string_lossy().as_ref()))
                .expect("register inter")
        };

        let css = format!(
            "@page {{ size: 8.5in 11in; margin: 0.5in; }} body {{ margin: 0; font-family: '{}'; font-size: 12pt; }}",
            font_name
        );
        let html_list = vec![
            "<html><body><p>Record 1</p></body></html>".to_string(),
            "<html><body><p>Record 2</p></body></html>".to_string(),
            "<html><body><p>Record 3</p></body></html>".to_string(),
        ];
        let jobs: Vec<(String, String)> = html_list
            .iter()
            .map(|html| (html.clone(), css.clone()))
            .collect();

        let mut seq_writer = Vec::new();
        engine
            .render_many_to_writer(&html_list, &css, &mut seq_writer)
            .expect("render_many_to_writer");
        assert_eq!(count_token(&seq_writer, b"/FontFile2"), 1);

        let mut seq_css_writer = Vec::new();
        engine
            .render_many_to_writer_with_css(&jobs, &mut seq_css_writer)
            .expect("render_many_to_writer_with_css");
        assert_eq!(count_token(&seq_css_writer, b"/FontFile2"), 1);

        let mut parallel_writer = Vec::new();
        engine
            .render_many_to_writer_parallel(&html_list, &css, &mut parallel_writer)
            .expect("render_many_to_writer_parallel");
        assert_eq!(count_token(&parallel_writer, b"/FontFile2"), 1);

        let tmp_dir = std::env::temp_dir();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seq_path = tmp_dir.join(format!(
            "fullbleed_batch_font_dedup_seq_{}_{}.pdf",
            std::process::id(),
            stamp
        ));
        let parallel_path = tmp_dir.join(format!(
            "fullbleed_batch_font_dedup_parallel_{}_{}.pdf",
            std::process::id(),
            stamp
        ));

        engine
            .render_many_to_file(&html_list, &css, &seq_path)
            .expect("render_many_to_file");
        engine
            .render_many_to_file_parallel(&html_list, &css, &parallel_path)
            .expect("render_many_to_file_parallel");

        let seq_file_bytes = std::fs::read(&seq_path).expect("read seq file");
        let parallel_file_bytes = std::fs::read(&parallel_path).expect("read parallel file");
        assert_eq!(count_token(&seq_file_bytes, b"/FontFile2"), 1);
        assert_eq!(count_token(&parallel_file_bytes, b"/FontFile2"), 1);

        let _ = std::fs::remove_file(seq_path);
        let _ = std::fs::remove_file(parallel_path);
    }

    #[test]
    fn watermark_ocg_semantics_wrap_commands() {
        let mut spec = WatermarkSpec::text("CONFIDENTIAL");
        spec.semantics = WatermarkSemantics::Ocg;
        let resolver = style::StyleResolver::new("");
        let commands = build_watermark_commands(
            &spec,
            Size::a4(),
            1,
            1,
            None,
            &resolver,
            None,
            None,
            false,
            false,
        );
        assert!(matches!(
            commands.first(),
            Some(Command::BeginOptionalContent { name }) if name == WATERMARK_OCG_RESOURCE_NAME
        ));
        assert!(matches!(
            commands.get(1),
            Some(Command::BeginArtifact { subtype: Some(subtype) }) if subtype == "Watermark"
        ));
        assert!(matches!(
            commands.iter().rev().nth(1),
            Some(Command::EndMarkedContent)
        ));
        assert!(matches!(commands.last(), Some(Command::EndMarkedContent)));
    }

    #[test]
    fn watermark_text_applies_to_each_page() {
        let base = empty_document(3);
        let spec = WatermarkSpec::text("CONFIDENTIAL");
        let resolver = style::StyleResolver::new("");
        let wm = build_watermark_document(&base, &spec, &resolver, None, None, None, false, false);

        assert_eq!(wm.pages.len(), 3);
        for page in &wm.pages {
            assert!(page.commands.iter().any(
                |cmd| matches!(cmd, Command::DrawString { text, .. } if text == "CONFIDENTIAL")
            ));
        }
    }

    #[test]
    fn watermark_image_applies_to_each_page() {
        let base = empty_document(4);
        let image_source = "examples/img/full_bleed-logo_small.png".to_string();
        let spec = WatermarkSpec::image(image_source.clone());
        let resolver = style::StyleResolver::new("");
        let wm = build_watermark_document(&base, &spec, &resolver, None, None, None, false, false);

        assert_eq!(wm.pages.len(), 4);
        for page in &wm.pages {
            assert!(page.commands.iter().any(|cmd| {
                matches!(
                    cmd,
                    Command::DrawImage { resource_id, .. } if resource_id == &image_source
                )
            }));
        }
    }

    #[test]
    fn watermark_text_uses_transform_compatible_y_coordinate() {
        let spec = WatermarkSpec::text("WM");
        let resolver = style::StyleResolver::new("");
        let page_size = Size::a4();
        let commands = build_watermark_commands(
            &spec, page_size, 1, 1, None, &resolver, None, None, false, false,
        );

        let draw = commands.iter().find_map(|cmd| match cmd {
            Command::DrawString { x, y, text } => Some((*x, *y, text.as_str())),
            _ => None,
        });
        let (_x, y, text) = draw.expect("expected DrawString command");
        assert_eq!(text, "WM");
        assert_eq!(y, page_size.height - spec.font_size.mul_ratio(1, 2));
    }

    #[test]
    fn watermark_image_uses_transform_compatible_y_coordinate() {
        let spec = WatermarkSpec::image("missing-watermark-image.png");
        let resolver = style::StyleResolver::new("");
        let page_size = Size::a4();
        let commands = build_watermark_commands(
            &spec, page_size, 1, 1, None, &resolver, None, None, false, false,
        );

        let draw = commands.iter().find_map(|cmd| match cmd {
            Command::DrawImage { y, height, .. } => Some((*y, *height)),
            _ => None,
        });
        let (y, height) = draw.expect("expected DrawImage command");
        assert_eq!(y, page_size.height - height);
    }

    #[test]
    fn html_img_tag_emits_draw_image_command() {
        let html = r#"
            <!doctype html>
            <html>
              <body>
                <img class="logo" src="examples/img/full_bleed-logo_small.png" alt="logo" />
              </body>
            </html>
        "#;
        let css = r#"
            @page { size: 8.5in 11in; margin: 0.5in; }
            .logo { width: 210px; height: 86px; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let mut found = false;
        for page in &doc.pages {
            if page.commands.iter().any(|cmd| {
                matches!(cmd, Command::DrawImage { resource_id, .. } if resource_id == "examples/img/full_bleed-logo_small.png")
            }) {
                found = true;
                break;
            }
        }
        assert!(found, "expected <img> to emit DrawImage command");
    }

    #[test]
    fn display_table_cells_share_a_single_row() {
        let html = r#"
            <!doctype html>
            <html>
              <body>
                <div class="t"><span>AA</span><span>BB</span><span>CC</span></div>
              </body>
            </html>
        "#;
        let css = r#"
            @page { size: 4in 4in; margin: 0.25in; }
            body { margin: 0; font-size: 12px; line-height: 1.2; }
            .t { display: table; width: 240px; border: 1px solid #000; }
            .t > span { display: table-cell; padding: 2px 4px; border-right: 1px solid #000; }
            .t > span:last-child { border-right: 0; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");

        let mut aa: Option<(Pt, Pt)> = None;
        let mut bb: Option<(Pt, Pt)> = None;
        let mut cc: Option<(Pt, Pt)> = None;
        for cmd in &page.commands {
            if let Command::DrawString { text, x, y } = cmd {
                if text.contains("AA") {
                    aa = Some((*x, *y));
                } else if text.contains("BB") {
                    bb = Some((*x, *y));
                } else if text.contains("CC") {
                    cc = Some((*x, *y));
                }
            }
        }
        let (aa_x, aa_y) = aa.expect("missing AA draw");
        let (bb_x, bb_y) = bb.expect("missing BB draw");
        let (cc_x, cc_y) = cc.expect("missing CC draw");
        assert!((aa_y.to_f32() - bb_y.to_f32()).abs() < 1.0);
        assert!((bb_y.to_f32() - cc_y.to_f32()).abs() < 1.0);
        assert!(aa_x < bb_x && bb_x < cc_x);
    }

    #[test]
    fn heading_honors_display_block_on_inline_descendants() {
        let html = r#"
            <!doctype html>
            <html>
              <body>
                <h1 class="title">
                  <span class="line">TITLELINE_TOP</span>
                  <span class="line">TITLELINE_BOTTOM</span>
                </h1>
              </body>
            </html>
        "#;
        let css = r#"
            @page { size: 4in 4in; margin: 0.25in; }
            body { margin: 0; font-family: sans-serif; }
            .title { margin: 0; font-size: 28px; line-height: 1.1; }
            .title > .line { display: block; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");

        let mut top_y: Option<Pt> = None;
        let mut bottom_y: Option<Pt> = None;
        let mut merged_line = false;
        for cmd in &page.commands {
            if let Command::DrawString { text, y, .. } = cmd {
                if text.contains("TITLELINE_TOP TITLELINE_BOTTOM")
                    || text.contains("TITLELINE_BOTTOM TITLELINE_TOP")
                {
                    merged_line = true;
                }
                if text.contains("TITLELINE_TOP") {
                    top_y = Some(*y);
                }
                if text.contains("TITLELINE_BOTTOM") {
                    bottom_y = Some(*y);
                }
            }
        }

        assert!(
            !merged_line,
            "display:block spans inside heading must not collapse to one line"
        );
        let top_y = top_y.expect("expected top heading line draw command");
        let bottom_y = bottom_y.expect("expected bottom heading line draw command");
        assert!(
            (top_y.to_f32() - bottom_y.to_f32()).abs() > 1.0,
            "expected heading block lines at different y positions, got y={} and y={}",
            top_y.to_f32(),
            bottom_y.to_f32()
        );
        assert!(
            bottom_y > top_y,
            "expected second heading block line to render below first line"
        );
    }

    #[test]
    fn html_table_cell_block_children_preserve_vertical_flow() {
        let html = r#"
            <!doctype html>
            <html>
              <body>
                <table class="t">
                  <tr>
                    <td>
                      <div>TBLTOPMARK</div>
                      <div>TBLBOTTOMMARK</div>
                    </td>
                  </tr>
                </table>
              </body>
            </html>
        "#;
        let css = r#"
            @page { size: 4in 4in; margin: 0.25in; }
            body { margin: 0; font-size: 14px; line-height: 1.2; }
            table.t { border-collapse: collapse; width: 220px; }
            table.t td { border: 1px solid #000; padding: 2px; }
            table.t td > div { display: block; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");

        let mut top_y: Option<Pt> = None;
        let mut bottom_y: Option<Pt> = None;
        let mut merged_line = false;
        for cmd in &page.commands {
            if let Command::DrawString { text, y, .. } = cmd {
                if text.contains("TBLTOPMARK TBLBOTTOMMARK")
                    || text.contains("TBLBOTTOMMARK TBLTOPMARK")
                {
                    merged_line = true;
                }
                if text.contains("TBLTOPMARK") {
                    top_y = Some(*y);
                }
                if text.contains("TBLBOTTOMMARK") {
                    bottom_y = Some(*y);
                }
            }
        }

        assert!(
            !merged_line,
            "table cell block descendants should not collapse into one text line"
        );
        let top_y = top_y.expect("expected top marker draw command");
        let bottom_y = bottom_y.expect("expected bottom marker draw command");
        assert!(
            (top_y.to_f32() - bottom_y.to_f32()).abs() > 1.0,
            "expected block descendants at different y positions, got y={} and y={}",
            top_y.to_f32(),
            bottom_y.to_f32()
        );
        assert!(
            bottom_y > top_y,
            "expected second block to render below first block"
        );
    }

    #[test]
    fn html_table_second_row_starts_after_multiline_first_row() {
        let html = r#"
            <!doctype html>
            <html>
              <body>
                <table class="t">
                  <tr>
                    <td>
                      <div>ROW1A</div>
                      <div>ROW1B</div>
                      <div>ROW1C</div>
                    </td>
                  </tr>
                  <tr>
                    <td>ROW2ONLY</td>
                  </tr>
                </table>
              </body>
            </html>
        "#;
        let css = r#"
            @page { size: 4in 4in; margin: 0.25in; }
            body { margin: 0; font-size: 14px; line-height: 1.2; }
            table.t { border-collapse: collapse; width: 220px; }
            table.t td { border: 1px solid #000; padding: 2px; }
            table.t td > div { display: block; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");

        let mut row1_last_y: Option<Pt> = None;
        let mut row2_y: Option<Pt> = None;
        for cmd in &page.commands {
            if let Command::DrawString { text, y, .. } = cmd {
                if text.contains("ROW1C") {
                    row1_last_y = Some(*y);
                }
                if text.contains("ROW2ONLY") {
                    row2_y = Some(*y);
                }
            }
        }

        let row1_last_y = row1_last_y.expect("expected ROW1C draw command");
        let row2_y = row2_y.expect("expected ROW2ONLY draw command");
        assert!(
            row2_y > row1_last_y,
            "expected second row to render below first row content, got row1={} row2={}",
            row1_last_y.to_f32(),
            row2_y.to_f32()
        );
    }

    #[test]
    fn html_table_colspan_preserves_following_column_alignment() {
        let html = r#"
            <!doctype html>
            <html>
              <body>
                <table class="t">
                  <tr>
                    <td colspan="2">FULLSPAN HEADER</td>
                  </tr>
                  <tr>
                    <td>LEFT</td>
                    <td>RIGHT</td>
                  </tr>
                </table>
              </body>
            </html>
        "#;
        let css = r#"
            @page { size: 4in 4in; margin: 0.25in; }
            body { margin: 0; font-size: 14px; line-height: 1.2; }
            table.t { border-collapse: collapse; width: 300px; table-layout: fixed; }
            table.t td { border: 1px solid #000; padding: 2px; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");

        let mut left_x: Option<Pt> = None;
        let mut right_x: Option<Pt> = None;
        for cmd in &page.commands {
            if let Command::DrawString { text, x, .. } = cmd {
                if text == "LEFT" {
                    left_x = Some(*x);
                }
                if text == "RIGHT" {
                    right_x = Some(*x);
                }
            }
        }

        let left_x = left_x.expect("expected LEFT draw command");
        let right_x = right_x.expect("expected RIGHT draw command");
        let delta = right_x - left_x;
        assert!(
            delta > Pt::from_f32(50.0),
            "expected RIGHT to be in a separate column, got delta={}",
            delta.to_f32()
        );
        assert!(
            delta < Pt::from_f32(220.0),
            "colspan should not collapse following column to page edge, got delta={}",
            delta.to_f32()
        );
    }

    #[test]
    fn list_item_block_children_preserve_vertical_flow() {
        let html = r#"
            <!doctype html>
            <html>
              <body>
                <ul class="menu">
                  <li>
                    <div class="title">ITEMHEADONLY</div>
                    <div class="desc">ITEMDESCONLY</div>
                  </li>
                </ul>
              </body>
            </html>
        "#;
        let css = r#"
            @page { size: 4in 4in; margin: 0.25in; }
            body { margin: 0; font-size: 14px; line-height: 1.2; }
            ul, li { margin: 0; padding: 0; list-style: none; }
            .title, .desc { display: block; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");

        let mut title_y: Option<Pt> = None;
        let mut desc_y: Option<Pt> = None;
        let mut merged_line = false;
        for cmd in &page.commands {
            if let Command::DrawString { text, y, .. } = cmd {
                if text.contains("ITEMHEADONLY ITEMDESCONLY")
                    || text.contains("ITEMDESCONLY ITEMHEADONLY")
                {
                    merged_line = true;
                }
                if text.contains("ITEMHEADONLY") {
                    title_y = Some(*y);
                }
                if text.contains("ITEMDESCONLY") {
                    desc_y = Some(*y);
                }
            }
        }

        assert!(
            !merged_line,
            "list-item block children should not collapse into one line"
        );
        let title_y = title_y.expect("expected title draw command");
        let desc_y = desc_y.expect("expected description draw command");
        assert!(
            (title_y.to_f32() - desc_y.to_f32()).abs() > 1.0,
            "expected list-item block children on separate lines, got y={} and y={}",
            title_y.to_f32(),
            desc_y.to_f32()
        );
    }

    #[test]
    fn css_page_size_applies_when_builder_page_size_is_default() {
        let html = "<!doctype html><html><body><p>hello</p></body></html>";
        let css = "@page { size: letter; margin: 0.5in; }";
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        assert!((doc.page_size.width.to_f32() - 612.0).abs() < 0.01);
        assert!((doc.page_size.height.to_f32() - 792.0).abs() < 0.01);
    }

    #[test]
    fn explicit_builder_page_size_logs_page_size_overridden() {
        let log_path = temp_log_path("page_size_override");
        let html = "<!doctype html><html><body><p>hello</p></body></html>";
        let css = "@page { size: letter; }";
        let engine = FullBleed::builder()
            .page_size(Size::a4())
            .debug_log(&log_path)
            .build()
            .expect("engine");
        let _ = engine
            .render_to_document(html, css)
            .expect("render document");
        drop(engine);
        let log = std::fs::read_to_string(&log_path).expect("read debug log");
        assert!(log.contains("\"PAGE_SIZE_OVERRIDDEN\""));
        let _ = std::fs::remove_file(log_path);
    }

    #[test]
    fn pagination_emits_page_break_trigger_event() {
        let log_path = temp_log_path("page_break_trigger");
        let html = "<!doctype html><html><body><p>one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty.</p><p>one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty.</p><p>one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty.</p></body></html>";
        let css = "body { margin: 0; font-size: 14px; line-height: 1.2; }";
        let engine = FullBleed::builder()
            .page_size(Size::from_inches(3.0, 3.0))
            .margin_all(18.0)
            .debug_log(&log_path)
            .build()
            .expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        assert!(doc.pages.len() > 1);
        drop(engine);
        let log = std::fs::read_to_string(&log_path).expect("read debug log");
        assert!(log.contains("\"PAGE_BREAK_TRIGGER\""));
        let _ = std::fs::remove_file(log_path);
    }

    #[test]
    fn html_table_row_collection_excludes_nested_tables() {
        let log_path = temp_log_path("table_row_scope");
        let html = r#"
            <!doctype html>
            <html>
              <body>
                <table class="outer">
                  <tr>
                    <td>
                      <div>Outer left</div>
                      <table class="inner">
                        <tr>
                          <td>Inner A</td>
                          <td>Inner B</td>
                          <td>Inner C</td>
                        </tr>
                      </table>
                    </td>
                    <td>Outer right</td>
                  </tr>
                </table>
              </body>
            </html>
        "#;
        let css = r#"
            body { margin: 0; font-size: 10pt; }
            table { border-collapse: collapse; width: 100%; }
            td { border: 1px solid #333; padding: 2pt; }
        "#;
        let engine = FullBleed::builder()
            .page_size(Size::from_inches(4.0, 4.0))
            .margin_all(0.0)
            .debug_log(&log_path)
            .build()
            .expect("engine");
        let _doc = engine
            .render_to_document(html, css)
            .expect("render document");
        drop(engine);

        let log = std::fs::read_to_string(&log_path).expect("read debug log");
        let table_rows_events: Vec<&str> = log
            .lines()
            .filter(|line| line.contains("\"type\":\"table.rows\""))
            .collect();
        assert!(
            table_rows_events.len() >= 2,
            "expected outer and inner table row events, got {}",
            table_rows_events.len()
        );
        assert!(
            table_rows_events[0].contains("\"total\":1"),
            "outer table should only collect direct rows: {}",
            table_rows_events[0]
        );
        let _ = std::fs::remove_file(log_path);
    }

    #[test]
    fn paragraph_split_produces_two_parts() {
        let text = "one two three four five six seven eight nine ten eleven twelve";
        let mut style = TextStyle::default();
        style.font_size = Pt::from_f32(10.0);
        style.line_height = Pt::from_f32(12.0);
        let paragraph = Paragraph::new(text)
            .with_style(style)
            .with_pagination(Pagination {
                orphans: 1,
                widows: 1,
                ..Pagination::default()
            });

        let avail_width = Pt::from_f32(60.0);
        let avail_height = Pt::from_f32(24.0);
        let split = paragraph.split(avail_width, avail_height);
        assert!(split.is_some());

        let (first, second) = split.unwrap();
        let first_size = first.wrap(avail_width, avail_height);
        let second_size = second.wrap(avail_width, avail_height);

        assert!(first_size.height <= avail_height);
        assert!(second_size.height > Pt::ZERO);
    }

    #[test]
    fn frame_overflows_on_extra_spacer() {
        let mut frame = Frame::new(Rect {
            x: Pt::ZERO,
            y: Pt::ZERO,
            width: Pt::from_f32(200.0),
            height: Pt::from_f32(50.0),
        });
        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(200.0),
            height: Pt::from_f32(200.0),
        });

        for _ in 0..10 {
            let result = frame.add(Box::new(Spacer::new_pt(Pt::from_f32(5.0))), &mut canvas);
            assert!(matches!(result, AddResult::Placed));
        }

        let result = frame.add(Box::new(Spacer::new_pt(Pt::from_f32(5.0))), &mut canvas);
        assert!(matches!(result, AddResult::Overflow(_)));
    }

    #[test]
    fn table_split_repeats_header_when_enabled() {
        let header = vec![vec![table_cell("HDR_ID"), table_cell("HDR_NAME")]];
        let body: Vec<Vec<TableCell>> = (1..=20)
            .map(|i| vec![table_cell(&i.to_string()), table_cell(&format!("row-{i}"))])
            .collect();
        let table = TableFlowable::new(body)
            .with_header(header)
            .repeat_header(true);

        let frame_rect = Rect {
            x: Pt::ZERO,
            y: Pt::ZERO,
            width: Pt::from_f32(300.0),
            height: Pt::from_f32(72.0),
        };
        let mut frame1 = Frame::new(frame_rect);
        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(300.0),
            height: Pt::from_f32(200.0),
        });
        let second = match frame1.add(Box::new(table), &mut canvas) {
            AddResult::Split(rest) => rest,
            other => panic!(
                "expected split, got variant {:?}",
                std::mem::discriminant(&other)
            ),
        };

        canvas.show_page();
        let mut frame2 = Frame::new(frame_rect);
        let result2 = frame2.add(second, &mut canvas);
        assert!(matches!(result2, AddResult::Placed | AddResult::Split(_)));

        let doc = canvas.finish();
        assert!(doc.pages.len() >= 2);
        assert!(page_contains_text(&doc.pages[0], "HDR_ID"));
        assert!(page_contains_text(&doc.pages[1], "HDR_ID"));
    }

    #[test]
    fn table_split_does_not_repeat_header_when_disabled() {
        let header = vec![vec![table_cell("HDR_OFF"), table_cell("HDR_OFF_NAME")]];
        let body: Vec<Vec<TableCell>> = (1..=20)
            .map(|i| vec![table_cell(&i.to_string()), table_cell(&format!("row-{i}"))])
            .collect();
        let table = TableFlowable::new(body)
            .with_header(header)
            .repeat_header(false);

        let frame_rect = Rect {
            x: Pt::ZERO,
            y: Pt::ZERO,
            width: Pt::from_f32(300.0),
            height: Pt::from_f32(72.0),
        };
        let mut frame1 = Frame::new(frame_rect);
        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(300.0),
            height: Pt::from_f32(200.0),
        });
        let second = match frame1.add(Box::new(table), &mut canvas) {
            AddResult::Split(rest) => rest,
            other => panic!(
                "expected split, got variant {:?}",
                std::mem::discriminant(&other)
            ),
        };

        canvas.show_page();
        let mut frame2 = Frame::new(frame_rect);
        let result2 = frame2.add(second, &mut canvas);
        assert!(matches!(result2, AddResult::Placed | AddResult::Split(_)));

        let doc = canvas.finish();
        assert!(doc.pages.len() >= 2);
        assert!(page_contains_text(&doc.pages[0], "HDR_OFF"));
        assert!(!page_contains_text(&doc.pages[1], "HDR_OFF"));
    }

    #[test]
    fn collapsed_border_prefers_wider_adjacent_edge_color() {
        let left = table_cell_with_border(
            "A",
            EdgeSizes {
                top: abs(0.0),
                right: abs(1.0),
                bottom: abs(0.0),
                left: abs(0.0),
            },
            Color::rgb(1.0, 0.0, 0.0),
        );
        let right = table_cell_with_border(
            "B",
            EdgeSizes {
                top: abs(0.0),
                right: abs(0.0),
                bottom: abs(0.0),
                left: abs(4.0),
            },
            Color::rgb(0.0, 0.0, 1.0),
        );

        let table = TableFlowable::new(vec![vec![left, right]])
            .with_border_collapse(BorderCollapseMode::Collapse);

        let mut frame = Frame::new(Rect {
            x: Pt::ZERO,
            y: Pt::ZERO,
            width: Pt::from_f32(200.0),
            height: Pt::from_f32(60.0),
        });
        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(200.0),
            height: Pt::from_f32(60.0),
        });
        let result = frame.add(Box::new(table), &mut canvas);
        assert!(matches!(result, AddResult::Placed));

        let doc = canvas.finish();
        let page = &doc.pages[0];
        let mut current_fill = Color::BLACK;
        let mut found_winner_edge = false;
        for command in &page.commands {
            match command {
                Command::SetFillColor(color) => current_fill = *color,
                Command::DrawRect { width, .. } => {
                    if *width == Pt::from_f32(4.0) && current_fill == Color::rgb(0.0, 0.0, 1.0) {
                        found_winner_edge = true;
                        break;
                    }
                }
                _ => {}
            }
        }
        assert!(
            found_winner_edge,
            "expected 4pt shared border drawn in blue"
        );
    }

    #[test]
    fn rendered_pages_emit_page_template_meta_for_finalize_binding() {
        let html = "<!doctype html><html><body><p>hello</p></body></html>";
        let css = "@page { size: letter; margin: 0.5in; }";
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        assert!(!doc.pages.is_empty(), "expected at least one page");
        for page in &doc.pages {
            let has_template_meta = page.commands.iter().any(|cmd| {
                matches!(
                    cmd,
                    Command::Meta { key, value }
                    if key == META_PAGE_TEMPLATE_KEY && !value.trim().is_empty()
                )
            });
            assert!(
                has_template_meta,
                "expected page to include {} metadata",
                META_PAGE_TEMPLATE_KEY
            );
        }
    }

    #[test]
    fn template_binding_accepts_feature_meta_from_plain_div_data_fb() {
        let html = r#"
<!doctype html>
<html>
  <body>
    <section>
      <div data-fb="fb.feature.red=1"></div>
      <p>Page one marker</p>
    </section>
    <section style="page-break-before: always;">
      <div data-fb="fb.feature.green=1"></div>
      <p>Page two marker</p>
    </section>
  </body>
</html>
"#;
        let css = "@page { size: letter; margin: 0.5in; }";

        let mut spec = TemplateBindingSpec::default();
        spec.default_template_id = Some("tpl-default".to_string());
        spec.by_feature = std::collections::BTreeMap::from([
            ("red".to_string(), "tpl-red".to_string()),
            ("green".to_string(), "tpl-green".to_string()),
        ]);

        let engine = FullBleed::builder()
            .template_binding_spec(spec)
            .build()
            .expect("engine");

        let (_pdf, _page_data, bindings) = engine
            .render_with_page_data_and_template_bindings(html, css)
            .expect("render");
        let bindings = bindings.expect("expected bindings");
        assert_eq!(bindings.len(), 2, "expected two pages");
        assert_eq!(bindings[0].template_id, "tpl-red");
        assert_eq!(bindings[0].source, BindingSource::Feature);
        assert_eq!(bindings[1].template_id, "tpl-green");
        assert_eq!(bindings[1].source, BindingSource::Feature);
    }

    #[test]
    fn template_binding_accepts_feature_meta_from_metadata_only_div_pages() {
        let html = r#"
<!doctype html>
<html>
  <body>
    <section>
      <div data-fb="fb.feature.red=1"></div>
    </section>
    <section style="page-break-before: always;">
      <div data-fb="fb.feature.green=1"></div>
    </section>
  </body>
</html>
"#;
        let css = "@page { size: letter; margin: 0.5in; }";

        let mut spec = TemplateBindingSpec::default();
        spec.default_template_id = Some("tpl-default".to_string());
        spec.by_feature = std::collections::BTreeMap::from([
            ("red".to_string(), "tpl-red".to_string()),
            ("green".to_string(), "tpl-green".to_string()),
        ]);

        let engine = FullBleed::builder()
            .template_binding_spec(spec)
            .build()
            .expect("engine");

        let (_pdf, _page_data, bindings) = engine
            .render_with_page_data_and_template_bindings(html, css)
            .expect("render");
        let bindings = bindings.expect("expected bindings");
        assert_eq!(bindings.len(), 2, "expected two pages");
        assert_eq!(bindings[0].template_id, "tpl-red");
        assert_eq!(bindings[0].source, BindingSource::Feature);
        assert_eq!(bindings[1].template_id, "tpl-green");
        assert_eq!(bindings[1].source, BindingSource::Feature);
    }

    #[test]
    fn render_with_page_data_and_glyph_report_smoke() {
        let html = "<!doctype html><html><body><p>hello</p></body></html>";
        let css = "@page { size: letter; margin: 0.5in; }";
        let engine = FullBleed::builder().build().expect("engine");

        let (pdf, page_data, report) = engine
            .render_with_page_data_and_glyph_report(html, css)
            .expect("render");
        assert!(
            !pdf.is_empty(),
            "expected pdf bytes from combined page_data+glyph report render"
        );
        assert!(page_data.is_none(), "no page_data expected without context");
        assert!(
            report.is_empty(),
            "expected no missing glyphs for simple ascii sample"
        );
    }

    #[test]
    fn render_with_page_data_template_bindings_and_glyph_report_smoke() {
        let html = r#"
<!doctype html>
<html>
  <body>
    <section>
      <div data-fb="fb.feature.red=1"></div>
      <p>Page one marker</p>
    </section>
    <section style="page-break-before: always;">
      <div data-fb="fb.feature.green=1"></div>
      <p>Page two marker</p>
    </section>
  </body>
</html>
"#;
        let css = "@page { size: letter; margin: 0.5in; }";

        let mut spec = TemplateBindingSpec::default();
        spec.default_template_id = Some("tpl-default".to_string());
        spec.by_feature = std::collections::BTreeMap::from([
            ("red".to_string(), "tpl-red".to_string()),
            ("green".to_string(), "tpl-green".to_string()),
        ]);

        let engine = FullBleed::builder()
            .template_binding_spec(spec)
            .build()
            .expect("engine");

        let (pdf, _page_data, bindings, report) = engine
            .render_with_page_data_and_template_bindings_and_glyph_report(html, css)
            .expect("render");
        assert!(
            !pdf.is_empty(),
            "expected pdf bytes from combined bindings+glyph report render"
        );
        assert!(
            report.is_empty(),
            "expected no missing glyphs for simple ascii sample"
        );

        let bindings = bindings.expect("expected bindings");
        assert_eq!(bindings.len(), 2, "expected two pages");
        assert_eq!(bindings[0].template_id, "tpl-red");
        assert_eq!(bindings[1].template_id, "tpl-green");
    }

    #[test]
    fn render_to_buffer_pdf_bytes_are_deterministic() {
        let html =
            "<!doctype html><html><body><h1>Determinism</h1><p>alpha beta gamma</p></body></html>";
        let css = "@page { size: letter; margin: 0.5in; } body { margin: 0; font-size: 12pt; }";

        let bytes_a = FullBleed::builder()
            .build()
            .expect("engine a")
            .render_to_buffer(html, css)
            .expect("render a");
        let bytes_b = FullBleed::builder()
            .build()
            .expect("engine b")
            .render_to_buffer(html, css)
            .expect("render b");

        assert_eq!(
            bytes_a, bytes_b,
            "render_to_buffer should be byte deterministic for identical input"
        );
    }

    #[test]
    fn render_many_parallel_pdf_bytes_are_deterministic_across_thread_counts() {
        let engine = FullBleed::builder().build().expect("engine");
        let css = "@page { size: letter; margin: 0.5in; } body { margin: 0; font-size: 12pt; }";
        let html_list = vec![
            "<!doctype html><html><body><p>Row 1</p></body></html>".to_string(),
            "<!doctype html><html><body><p>Row 2</p></body></html>".to_string(),
            "<!doctype html><html><body><p>Row 3</p></body></html>".to_string(),
        ];

        let render_with_threads = |threads: usize| -> Vec<u8> {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .expect("thread pool");
            pool.install(|| {
                let mut out = Vec::new();
                engine
                    .render_many_to_writer_parallel(&html_list, css, &mut out)
                    .expect("parallel render");
                out
            })
        };

        let bytes_1 = render_with_threads(1);
        let bytes_4 = render_with_threads(4);
        assert_eq!(
            bytes_1, bytes_4,
            "parallel PDF output should be byte deterministic across thread counts"
        );
    }

    #[test]
    fn render_image_pages_png_bytes_are_deterministic() {
        let engine = FullBleed::builder().build().expect("engine");
        let html = "<!doctype html><html><body><h1>PNG Determinism</h1><p>same input same output</p></body></html>";
        let css = "@page { size: 6in 4in; margin: 0.25in; } body { margin: 0; font-size: 12pt; }";

        let pages_a = engine
            .render_image_pages(html, css, 120)
            .expect("image render a");
        let pages_b = engine
            .render_image_pages(html, css, 120)
            .expect("image render b");
        assert_eq!(
            pages_a, pages_b,
            "render_image_pages should be byte deterministic for identical input"
        );
    }

    #[test]
    fn render_finalized_pdf_image_pages_png_bytes_are_deterministic() {
        let engine = FullBleed::builder().build().expect("engine");
        let html = "<!doctype html><html><body><h1>Finalize PNG Determinism</h1><p>stable</p></body></html>";
        let css = "@page { size: 6in 4in; margin: 0.25in; } body { margin: 0; font-size: 12pt; }";
        let pdf = engine.render_to_buffer(html, css).expect("render pdf");

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pdf_path = std::env::temp_dir().join(format!(
            "fullbleed_finalize_png_determinism_{}_{}.pdf",
            std::process::id(),
            stamp
        ));
        std::fs::write(&pdf_path, &pdf).expect("write temp pdf");

        let pages_a = engine
            .render_finalized_pdf_image_pages(&pdf_path, 120)
            .expect("finalized raster a");
        let pages_b = engine
            .render_finalized_pdf_image_pages(&pdf_path, 120)
            .expect("finalized raster b");
        let _ = std::fs::remove_file(&pdf_path);

        assert_eq!(
            pages_a, pages_b,
            "render_finalized_pdf_image_pages should be byte deterministic for identical input"
        );
    }
}
