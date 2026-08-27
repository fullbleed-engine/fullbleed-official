mod assets;
mod authoring;
mod base64;
mod canvas;
mod chart;
mod css_native;
mod css_queries;
mod debug;
mod doc_context;
mod doc_template;
mod error;
mod finalize;
mod flate_native;
mod flowable;
mod font;
mod font_subset;
mod frame;
mod glyph_report;
mod html;
mod html_dom;
mod html_entities;
mod image_native;
mod jit;
mod jpeg_native;
mod math;
mod metrics;
mod native_shape;
mod page_data;
mod page_template;
mod parallel;
mod pdf;
mod pdf_encodings;
mod pdf_native;
mod pdf_raster;
mod pdfinspect;
mod perf;
mod plan;
#[cfg(feature = "python")]
mod python;
#[cfg(feature = "python")]
mod python_abi;
mod raster;
mod raster_native;
mod sfnt;
mod sfnt_cff;
mod sfnt_outline;
mod spill;
mod style;
mod svg;
mod text_shape;
mod types;
mod unicode_data;
mod xml;

pub use assets::{Asset, AssetBundle, AssetKind};
pub use authoring::{
    AUTHORING_LANGUAGE_REPORT_SCHEMA, AUTHORING_READING_PREVIEW_SCHEMA, AuthoringCancellationToken,
    AuthoringDiagnostic, AuthoringDiagnosticSeverity, AuthoringLanguageDiagnostic,
    AuthoringLanguageFacts, AuthoringLanguageFeature, AuthoringLanguageFeatureContext,
    AuthoringLanguageReportV1, AuthoringLanguageRequest, AuthoringLayoutFragment,
    AuthoringLayoutNode, AuthoringLayoutPage, AuthoringLayoutSnapshotV1,
    AuthoringPreviewArtifactV1, AuthoringPreviewPhase, AuthoringPreviewProgress,
    AuthoringPreviewRequest, AuthoringReadingNode, AuthoringReadingPage, AuthoringReadingPreviewV1,
    AuthoringSourceLanguage, authoring_language_features, inspect_authoring_source,
};
pub use canvas::{Canvas, Command, Document, Page};
pub use chart::{
    CHART_COMPILER_SCHEMA, ChartArtifact, ChartDiagnostic, ChartError, ChartKind, ChartSeries,
    ChartSpec, ChartTable, ChartTrace, compile_chart,
};
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
#[cfg(feature = "python")]
use font::RegisteredFontTrace;
pub use font::extract_font_face_bytes;
pub use frame::{AddResult, Frame};
use fullbleed_audit_contract as audit_contract;
pub use glyph_report::{GlyphCoverageReport, MissingGlyph};
use html_dom::{NodeData, NodeRef};
pub use jit::JitMode;
pub use metrics::{DocumentMetrics, PageMetrics};
pub use page_data::{PageDataContext, PageDataOp, PageDataValue, PaginatedContextSpec};
use page_template::PageSelector;
pub use page_template::{FrameSpec, PageTemplate};
use pdf::PdfOptions;
pub use pdf::{CompiledFlowCompression, OutputIntent, PdfProfile, PdfVersion};
pub use pdfinspect::{
    PdfInspectError, PdfInspectErrorCode, PdfInspectReport, PdfInspectWarning,
    composition_compatibility_issues, inspect_pdf_bytes, inspect_pdf_path,
    require_pdf_composition_compatibility,
};
use perf::PerfLogger;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::f32::consts::PI;
use std::sync::{Arc, Condvar, Mutex};
pub use types::{Color, ColorSpace, I32F32, Margins, Pt, Rect, Size};

const FILE_OUTPUT_BUFFER_BYTES: usize = 1024 * 1024;

/// Per-job execution options for compiled content-reflow bindings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompiledReflowOptions {
    pub compression: CompiledFlowCompression,
}

fn render_to_buffered_file<T>(
    path: impl AsRef<std::path::Path>,
    render: impl FnOnce(&mut std::io::BufWriter<std::fs::File>) -> Result<T, FullBleedError>,
) -> Result<T, FullBleedError> {
    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::with_capacity(FILE_OUTPUT_BUFFER_BYTES, file);
    let result = render(&mut writer)?;
    std::io::Write::flush(&mut writer)?;
    Ok(result)
}

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
    asset_bundle: Arc<AssetBundle>,
    render_context_cache: Mutex<RenderContextCache>,
}

struct CompiledReflowPlan {
    template: html_dom::CompiledHtmlBindingTemplate,
    context: RenderContext,
    runtime: Arc<FullBleed>,
    flow_programs: Mutex<HashMap<Arc<str>, Arc<CompiledFlowProgramEntry>>>,
    html_input_cache: html_dom::CompiledFlowHtmlInputCache,
    shape_cache: pdf::CompiledFlowShapeCache,
    pdf_program_cache: Mutex<Option<(usize, Arc<pdf::CompiledFlowPdfProgramCache>)>>,
}

#[derive(Default)]
struct CompiledFlowProgramState {
    programs: Vec<Arc<CompiledFlowRecordProgram>>,
    compiling: bool,
}

#[derive(Default)]
struct CompiledFlowProgramEntry {
    state: Mutex<CompiledFlowProgramState>,
    ready: Condvar,
}

#[derive(Clone, Copy)]
struct CompiledTextConstraint {
    origin_x: Pt,
    position_max_width: Pt,
    old_position_width: Pt,
    guard_max_width: Pt,
    old_width: Pt,
    letter_spacing: Pt,
    word_spacing: Pt,
    css_pixel_snap_metrics: bool,
    align: u8,
}

#[derive(Clone)]
struct CompiledFlowTextEdit {
    start: usize,
    end: usize,
    value_slots: Arc<[usize]>,
}

#[derive(Clone)]
struct CompiledFlowTextPatch {
    page: usize,
    command: usize,
    transformed: bool,
    prototype_text: Arc<str>,
    edits: Arc<[CompiledFlowTextEdit]>,
    font_name: Arc<str>,
    font_index: Option<usize>,
    font_size: Pt,
    prototype_x_pdf: Arc<str>,
    old_measured: Pt,
    prototype_shaped: Option<pdf::ShapedText>,
    numeric_shader: Option<Arc<pdf::CompiledNumericFontShader>>,
    constraint: Option<CompiledTextConstraint>,
}

struct CompiledFlowRecordProgram {
    document: Arc<Document>,
    prototype_values: Arc<[Arc<str>]>,
    patches: Arc<[CompiledFlowTextPatch]>,
    page_patch_counts: Arc<[usize]>,
    patched_values: Arc<[bool]>,
}

struct CompiledFlowBoundRecord {
    program: Arc<CompiledFlowRecordProgram>,
    page_overrides: Vec<Vec<pdf::CompiledFlowTextOverride>>,
    font_glyphs: Vec<(Arc<str>, BTreeMap<u16, String>)>,
    encoded_pages: Option<Vec<pdf::EncodedCompiledFlowPage>>,
}

type CompiledFlowWorkerGlyphs = HashMap<Arc<str>, Box<[u64; 1024]>>;

impl CompiledFlowBoundRecord {
    fn page_count(&self) -> usize {
        self.program.document.pages.len()
    }
}

impl CompiledTextConstraint {
    fn parse(value: &str) -> Option<Self> {
        let mut fields = value.split(':');
        let origin_x = Pt::from_milli_i64(fields.next()?.parse().ok()?);
        let position_max_width = Pt::from_milli_i64(fields.next()?.parse().ok()?);
        let old_position_width = Pt::from_milli_i64(fields.next()?.parse().ok()?);
        let guard_max_width = Pt::from_milli_i64(fields.next()?.parse().ok()?);
        let old_width = Pt::from_milli_i64(fields.next()?.parse().ok()?);
        let letter_spacing = Pt::from_milli_i64(fields.next()?.parse().ok()?);
        let word_spacing = Pt::from_milli_i64(fields.next()?.parse().ok()?);
        let css_pixel_snap_metrics = match fields.next()? {
            "0" => false,
            "1" => true,
            _ => return None,
        };
        let align = match fields.next()? {
            "l" => b'l',
            "c" => b'c',
            "r" => b'r',
            "j" => b'j',
            _ => return None,
        };
        fields.next().is_none().then_some(Self {
            origin_x,
            position_max_width,
            old_position_width,
            guard_max_width,
            old_width,
            letter_spacing,
            word_spacing,
            css_pixel_snap_metrics,
            align,
        })
    }

    fn measured_width(self, base: Pt, text: &str) -> Pt {
        let character_count = text.chars().count();
        let letter_extra = if character_count > 1 {
            self.letter_spacing * ((character_count - 1) as i32)
        } else {
            Pt::ZERO
        };
        let word_extra = self.word_spacing
            * (text.as_bytes().iter().filter(|byte| **byte == b' ').count() as i32);
        (base + letter_extra + word_extra).max(Pt::ZERO)
    }

    fn aligned_x(self, position_width: Pt) -> Pt {
        let remaining = (self.position_max_width - position_width).max(Pt::ZERO);
        let x = match self.align {
            b'r' => self.origin_x + remaining,
            b'c' => self.origin_x + remaining.mul_ratio(1, 2),
            _ => self.origin_x,
        };
        if self.css_pixel_snap_metrics {
            flowable::browser_registered_text_paint_x(x)
        } else {
            x
        }
    }
}

impl CompiledFlowRecordProgram {
    fn text_edits(text: &str, lookup: &HashMap<&str, Vec<usize>>) -> Vec<CompiledFlowTextEdit> {
        if let Some(slots) = lookup.get(text) {
            return vec![CompiledFlowTextEdit {
                start: 0,
                end: text.len(),
                value_slots: slots.clone().into(),
            }];
        }

        // Inline formatting can fuse adjacent DOM text nodes into one painted
        // glyph run. Compile only unique, non-overlapping source fragments;
        // ambiguous or very short fragments stay guarded by the normal-layout
        // fallback instead of risking a substitution into static prose.
        let mut candidates = Vec::<(usize, usize, &[usize])>::new();
        for (value, slots) in lookup {
            if value.len() < 4 || value.len() > text.len() {
                continue;
            }
            let mut occurrences = text.match_indices(*value);
            let Some((start, _)) = occurrences.next() else {
                continue;
            };
            if occurrences.next().is_some() {
                continue;
            }
            candidates.push((start, start + value.len(), slots.as_slice()));
        }
        candidates.sort_by(|left, right| {
            (right.1 - right.0)
                .cmp(&(left.1 - left.0))
                .then_with(|| left.0.cmp(&right.0))
        });

        let mut selected = Vec::<(usize, usize, &[usize])>::new();
        for candidate in candidates {
            if selected
                .iter()
                .any(|current| candidate.0 < current.1 && current.0 < candidate.1)
            {
                continue;
            }
            selected.push(candidate);
        }
        selected.sort_by_key(|candidate| candidate.0);
        selected
            .into_iter()
            .map(|(start, end, slots)| CompiledFlowTextEdit {
                start,
                end,
                value_slots: slots.to_vec().into(),
            })
            .collect()
    }

    fn compile(
        document: Document,
        values: &[Arc<str>],
        registry: &FontRegistry,
        shape_cache: &pdf::CompiledFlowShapeCache,
    ) -> Self {
        let mut lookup = HashMap::<&str, Vec<usize>>::new();
        for (index, value) in values.iter().enumerate() {
            if !value.is_empty() {
                lookup.entry(value.as_ref()).or_default().push(index);
            }
        }

        let mut patches = Vec::new();
        let mut patched_values = vec![false; values.len()];
        for (page_index, page) in document.pages.iter().enumerate() {
            let mut font_name = Arc::<str>::from("Helvetica");
            let mut font_size = Pt::from_f32(12.0);
            let mut state_stack = Vec::<(Arc<str>, Pt)>::new();
            let mut pending_constraint = None;
            for (command_index, command) in page.commands.iter().enumerate() {
                match command {
                    Command::SaveState => state_stack.push((font_name.clone(), font_size)),
                    Command::RestoreState => {
                        if let Some((saved_name, saved_size)) = state_stack.pop() {
                            font_name = saved_name;
                            font_size = saved_size;
                        }
                    }
                    Command::SetFontName(value) => font_name = Arc::from(value.as_str()),
                    Command::SetFontSize(value) => font_size = *value,
                    Command::Meta { key, value }
                        if key == flowable::META_COMPILED_TEXT_CONSTRAINT_KEY =>
                    {
                        pending_constraint = CompiledTextConstraint::parse(value);
                    }
                    Command::DrawString { x, text, .. } => {
                        let constraint = pending_constraint.take();
                        let edits = Self::text_edits(text, &lookup);
                        if edits.is_empty() {
                            continue;
                        }
                        for edit in &edits {
                            for slot in edit.value_slots.iter() {
                                patched_values[*slot] = true;
                            }
                        }
                        let font_index = registry.compiled_font_index(font_name.as_ref());
                        patches.push(CompiledFlowTextPatch {
                            page: page_index,
                            command: command_index,
                            transformed: false,
                            prototype_text: Arc::from(text.as_str()),
                            edits: edits.into(),
                            font_name: font_name.clone(),
                            font_index,
                            font_size,
                            prototype_x_pdf: Arc::from(pdf::format_compiled_flow_pt(*x)),
                            old_measured: font_index
                                .and_then(|index| {
                                    registry.measure_compiled_basic_latin_width_at(
                                        index, font_size, text,
                                    )
                                })
                                .unwrap_or_else(|| {
                                    registry.measure_compiled_basic_latin_width(
                                        font_name.as_ref(),
                                        font_size,
                                        text,
                                    )
                                }),
                            prototype_shaped: pdf::shape_compiled_flow_prototype(
                                shape_cache,
                                registry,
                                font_name.as_ref(),
                                text,
                            ),
                            numeric_shader: shape_cache.font_shader(registry, font_name.as_ref()),
                            constraint,
                        });
                    }
                    Command::DrawStringTransformed { x, text, .. } => {
                        let constraint = pending_constraint.take();
                        let edits = Self::text_edits(text, &lookup);
                        if edits.is_empty() {
                            continue;
                        }
                        for edit in &edits {
                            for slot in edit.value_slots.iter() {
                                patched_values[*slot] = true;
                            }
                        }
                        let font_index = registry.compiled_font_index(font_name.as_ref());
                        patches.push(CompiledFlowTextPatch {
                            page: page_index,
                            command: command_index,
                            transformed: true,
                            prototype_text: Arc::from(text.as_str()),
                            edits: edits.into(),
                            font_name: font_name.clone(),
                            font_index,
                            font_size,
                            prototype_x_pdf: Arc::from(pdf::format_compiled_flow_pt(*x)),
                            old_measured: font_index
                                .and_then(|index| {
                                    registry.measure_compiled_basic_latin_width_at(
                                        index, font_size, text,
                                    )
                                })
                                .unwrap_or_else(|| {
                                    registry.measure_compiled_basic_latin_width(
                                        font_name.as_ref(),
                                        font_size,
                                        text,
                                    )
                                }),
                            prototype_shaped: pdf::shape_compiled_flow_prototype(
                                shape_cache,
                                registry,
                                font_name.as_ref(),
                                text,
                            ),
                            numeric_shader: shape_cache.font_shader(registry, font_name.as_ref()),
                            constraint,
                        });
                    }
                    _ => {}
                }
            }
        }

        let mut page_patch_counts = vec![0usize; document.pages.len()];
        for patch in &patches {
            page_patch_counts[patch.page] = page_patch_counts[patch.page].saturating_add(1);
        }
        Self {
            document: Arc::new(document),
            prototype_values: values.to_vec().into(),
            patches: patches.into(),
            page_patch_counts: page_patch_counts.into(),
            patched_values: patched_values.into(),
        }
    }

    fn instantiate(
        self: &Arc<Self>,
        values: &[Arc<str>],
        registry: &FontRegistry,
        shape_cache: &pdf::CompiledFlowShapeCache,
        worker_glyphs: &mut CompiledFlowWorkerGlyphs,
    ) -> Option<CompiledFlowBoundRecord> {
        if values.len() != self.prototype_values.len() {
            return None;
        }
        for (index, patched) in self.patched_values.iter().enumerate() {
            if !patched && values[index] != self.prototype_values[index] {
                return None;
            }
        }

        let mut updates = Vec::with_capacity(self.patches.len());
        for patch in self.patches.iter() {
            let direct = patch.edits.len() == 1
                && patch.edits[0].start == 0
                && patch.edits[0].end == patch.prototype_text.len();
            let mut built_text =
                (!direct).then(|| String::with_capacity(patch.prototype_text.len()));
            let mut direct_text = None;
            let mut cursor = 0usize;
            for edit in patch.edits.iter() {
                if edit.start < cursor
                    || edit.end > patch.prototype_text.len()
                    || !patch.prototype_text.is_char_boundary(edit.start)
                    || !patch.prototype_text.is_char_boundary(edit.end)
                {
                    return None;
                }
                let first_slot = *edit.value_slots.first()?;
                let new_value = &values[first_slot];
                if edit
                    .value_slots
                    .iter()
                    .any(|slot| values[*slot] != *new_value)
                {
                    return None;
                }
                let old_value = &self.prototype_values[first_slot];
                if edit
                    .value_slots
                    .iter()
                    .any(|slot| self.prototype_values[*slot] != *old_value)
                    || &patch.prototype_text[edit.start..edit.end] != old_value.as_ref()
                {
                    return None;
                }
                if let Some(text) = built_text.as_mut() {
                    text.push_str(&patch.prototype_text[cursor..edit.start]);
                    text.push_str(new_value.as_ref());
                } else {
                    direct_text = Some(new_value.clone());
                }
                cursor = edit.end;
            }
            let new_text = if let Some(mut text) = built_text {
                text.push_str(&patch.prototype_text[cursor..]);
                Arc::from(text)
            } else {
                direct_text?
            };

            let old_measured = patch.old_measured;
            let changed = new_text.as_ref() != patch.prototype_text.as_ref();
            let new_measured = if changed {
                patch
                    .font_index
                    .and_then(|index| {
                        registry.measure_compiled_basic_latin_width_at(
                            index,
                            patch.font_size,
                            new_text.as_ref(),
                        )
                    })
                    .unwrap_or_else(|| {
                        registry.measure_compiled_basic_latin_width(
                            patch.font_name.as_ref(),
                            patch.font_size,
                            new_text.as_ref(),
                        )
                    })
            } else {
                old_measured
            };
            let constrained_width = patch.constraint.map(|constraint| {
                if changed {
                    // Calibrate the fast registry measurement against the exact width
                    // captured by the layout target. This retains any font/style-specific
                    // fixed-point bias (for example synthetic face metrics) while the
                    // explicit spacing terms account for target-length changes.
                    let prototype_measured =
                        constraint.measured_width(old_measured, patch.prototype_text.as_ref());
                    let measurement_bias = constraint.old_width - prototype_measured;
                    (constraint.measured_width(new_measured, new_text.as_ref()) + measurement_bias)
                        .max(Pt::ZERO)
                } else {
                    constraint.old_width
                }
            });
            if changed {
                match patch.constraint {
                    Some(constraint) => {
                        if new_text.contains(['\n', '\r', '\t'])
                            || constrained_width? > constraint.guard_max_width + Pt::from_f32(0.01)
                        {
                            return None;
                        }
                        let slack =
                            (constraint.guard_max_width - constraint.old_width).max(Pt::ZERO);
                        if constrained_width? != constraint.old_width && slack < Pt::from_f32(0.5) {
                            return None;
                        }
                    }
                    None if new_measured != old_measured => return None,
                    None => {}
                }
            }
            updates.push((new_text, constrained_width));
        }

        let mut page_overrides = self
            .page_patch_counts
            .iter()
            .map(|count| Vec::with_capacity(*count))
            .collect::<Vec<_>>();
        let mut font_glyphs = BTreeMap::<Arc<str>, BTreeMap<u16, String>>::new();
        for (patch, (new_text, constrained_width)) in self.patches.iter().zip(updates) {
            let command = self
                .document
                .pages
                .get(patch.page)?
                .commands
                .get(patch.command)?;
            let (x, prototype_text) = match command {
                Command::DrawString { x, text, .. } if !patch.transformed => (*x, text),
                Command::DrawStringTransformed { x, text, .. } if patch.transformed => (*x, text),
                _ => return None,
            };
            let _ = prototype_text;
            let bound_x = match (patch.constraint, constrained_width) {
                (Some(constraint), Some(width)) => {
                    let position_extra = constraint.old_position_width - constraint.old_width;
                    let target_position_width = (width + position_extra).max(Pt::ZERO);
                    constraint.aligned_x(target_position_width)
                }
                _ => x,
            };
            let shaped = pdf::shape_compiled_flow_text(
                shape_cache,
                registry,
                patch.font_name.as_ref(),
                patch.prototype_text.as_ref(),
                patch.prototype_shaped.as_ref(),
                patch.numeric_shader.as_ref(),
                new_text.as_ref(),
            )
            .or_else(|| {
                Some(pdf::shape_compiled_flow_cid_fallback(
                    registry,
                    patch.font_name.as_ref(),
                    new_text.as_ref(),
                ))
            });
            if let Some(shaped) = shaped.as_ref() {
                let seen = worker_glyphs
                    .entry(patch.font_name.clone())
                    .or_insert_with(|| Box::new([0; 1024]));
                shaped.for_each_glyph_source(new_text.as_ref(), |gid, value| {
                    let word = usize::from(gid) >> 6;
                    let mask = 1_u64 << (u32::from(gid) & 63);
                    if seen[word] & mask == 0 {
                        font_glyphs
                            .entry(patch.font_name.clone())
                            .or_default()
                            .entry(gid)
                            .or_insert_with(|| value.to_string());
                        seen[word] |= mask;
                    }
                });
            }
            let x_pdf = if bound_x == x {
                patch.prototype_x_pdf.clone()
            } else {
                Arc::from(pdf::format_compiled_flow_pt(bound_x))
            };
            page_overrides[patch.page].push(pdf::CompiledFlowTextOverride {
                command: patch.command,
                transformed: patch.transformed,
                x: bound_x,
                x_pdf,
                text: new_text,
                shaped,
            });
        }
        Some(CompiledFlowBoundRecord {
            program: self.clone(),
            page_overrides,
            font_glyphs: font_glyphs.into_iter().collect(),
            encoded_pages: None,
        })
    }
}

impl CompiledReflowPlan {
    fn flow_program_entry(&self, key: &str) -> Arc<CompiledFlowProgramEntry> {
        let mut programs = self
            .flow_programs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = programs.get(key) {
            return entry.clone();
        }
        programs
            .entry(Arc::from(key))
            .or_insert_with(|| Arc::new(CompiledFlowProgramEntry::default()))
            .clone()
    }

    fn render_compiled_flow_record(
        &self,
        row: usize,
        document: &mut html_dom::BoundHtmlDocument,
        worker_glyphs: &mut CompiledFlowWorkerGlyphs,
        columns: &[&[String]],
    ) -> Result<CompiledFlowBoundRecord, FullBleedError> {
        document.prepare_flow_program_input(columns, row, &self.html_input_cache);
        let entry = self.flow_program_entry(document.flow_program_key());
        let mut values = document.flow_dynamic_text().to_vec();
        loop {
            let programs = entry
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .programs
                .clone();
            for program in &programs {
                if let Some(document) = program.instantiate(
                    &values,
                    self.runtime.font_registry.as_ref(),
                    &self.shape_cache,
                    worker_glyphs,
                ) {
                    if let Some(perf) = self.runtime.perf.as_deref() {
                        perf.log_counts("compile.flow", Some(row), &[("cache_hit", 1)]);
                    }
                    return Ok(document);
                }
            }
            let mut state = entry
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.programs.len() != programs.len() {
                drop(state);
                continue;
            }
            if !state.compiling {
                state.compiling = true;
                drop(state);
                break;
            }
            let state = entry
                .ready
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            drop(state);
        }

        let started = std::time::Instant::now();
        let rendered = match document.materialize_flow_program_dom(columns, row) {
            Ok(()) => {
                values = document.flow_dynamic_text().to_vec();
                let _capture = flowable::set_compiled_flow_capture(true);
                self.runtime
                    .render_to_document_and_page_data_with_parsed_resolver_and_report_at(
                        row,
                        document.root(),
                        &self.context.page_templates,
                        &self.context.resolver,
                        None,
                    )
                    .map(|(document, _page_data)| document)
            }
            Err(error) => Err(FullBleedError::InvalidConfiguration(error)),
        };

        let mut state = entry
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.compiling = false;
        match rendered {
            Ok(document) => {
                let program = Arc::new(CompiledFlowRecordProgram::compile(
                    document,
                    &values,
                    self.runtime.font_registry.as_ref(),
                    &self.shape_cache,
                ));
                let output = program
                    .instantiate(
                        &values,
                        self.runtime.font_registry.as_ref(),
                        &self.shape_cache,
                        worker_glyphs,
                    )
                    .expect("a freshly compiled flow program must instantiate its prototype");
                let patch_count = program.patches.len();
                let patched_value_count = program
                    .patched_values
                    .iter()
                    .filter(|patched| **patched)
                    .count();
                state.programs.push(program);
                let variant_count = state.programs.len();
                entry.ready.notify_all();
                if let Some(perf) = self.runtime.perf.as_deref() {
                    perf.log_span_ms(
                        "compile.flow.program",
                        Some(row),
                        started.elapsed().as_secs_f64() * 1000.0,
                    );
                    perf.log_counts(
                        "compile.flow",
                        Some(row),
                        &[
                            ("cache_miss", 1),
                            ("values", values.len() as u64),
                            ("patched_values", patched_value_count as u64),
                            ("patches", patch_count as u64),
                            ("variants", variant_count as u64),
                        ],
                    );
                }
                Ok(output)
            }
            Err(error) => {
                entry.ready.notify_all();
                Err(error)
            }
        }
    }
}

/// Immutable output of the HTML/CSS frontend, binding compiler, and layout pipeline.
///
/// A compiled document owns the fixed-point display commands and every linker resource required
/// to render them. It can therefore be linked repeatedly, or from multiple threads, without
/// reparsing HTML/CSS or rebuilding layout. Text markers written as ``{{slot_name}}`` can also
/// be lowered either to fixed-geometry paint bindings or to a parsed-DOM reflow program. The
/// reflow program reuses one worker-local DOM plus the compiled CSS context while rerunning the
/// shaping, layout, and pagination stages required by size-changing values.
pub struct CompiledDocument {
    document: Arc<Document>,
    font_registry: Arc<FontRegistry>,
    pdf_options: PdfOptions,
    debug: Option<Arc<DebugLogger>>,
    perf: Option<Arc<PerfLogger>>,
    compile_nanos: u64,
    command_count: usize,
    binding_slots: Vec<String>,
    binding_plan: Option<Result<Arc<pdf::CompiledBindingPlan>, Arc<str>>>,
    reflow_plan: Option<Result<Arc<CompiledReflowPlan>, Arc<str>>>,
}

pub(crate) fn valid_binding_slot_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

pub(crate) fn binding_slot_spans(text: &str) -> Vec<(usize, usize, &str)> {
    let mut spans = Vec::new();
    let mut search = 0usize;
    while let Some(relative_start) = text[search..].find("{{") {
        let marker_start = search + relative_start;
        let name_start = marker_start + 2;
        let Some(relative_end) = text[name_start..].find("}}") else {
            break;
        };
        let name_end = name_start + relative_end;
        let marker_end = name_end + 2;
        let name = text[name_start..name_end].trim();
        if valid_binding_slot_name(name) {
            spans.push((marker_start, marker_end, name));
        }
        search = marker_end;
    }
    spans
}

pub(crate) fn binding_slot_names(text: &str) -> Vec<&str> {
    binding_slot_spans(text)
        .into_iter()
        .map(|(_, _, name)| name)
        .collect()
}

fn collect_binding_slots(commands: &[Command], slots: &mut BTreeSet<String>) {
    for command in commands {
        match command {
            Command::DrawString { text, .. } | Command::DrawStringTransformed { text, .. } => {
                for name in binding_slot_names(text) {
                    slots.insert(name.to_string());
                }
            }
            // Form-local slots require form-coordinate/state capture. Keep this first binding
            // contract page-local so its fixed-geometry guarantee remains explicit.
            Command::DefineForm { .. } | Command::DefineIsolatedForm { .. } => {}
            _ => {}
        }
    }
}

impl CompiledDocument {
    pub fn page_count(&self) -> usize {
        self.document.pages.len()
    }

    pub fn command_count(&self) -> usize {
        self.command_count
    }

    pub fn compile_time_ms(&self) -> f64 {
        self.compile_nanos as f64 / 1_000_000.0
    }

    pub fn binding_slots(&self) -> &[String] {
        &self.binding_slots
    }

    pub fn binding_program_page_count(&self) -> usize {
        self.binding_plan
            .as_ref()
            .and_then(|plan| plan.as_ref().ok())
            .map_or(0, |plan| plan.page_count())
    }

    pub fn binding_program_command_count(&self) -> usize {
        self.binding_plan
            .as_ref()
            .and_then(|plan| plan.as_ref().ok())
            .map_or(0, |plan| plan.command_count())
    }

    pub fn reflow_binding_slots(&self) -> &[String] {
        self.reflow_plan
            .as_ref()
            .and_then(|plan| plan.as_ref().ok())
            .map_or(&[], |plan| plan.template.slot_names())
    }

    pub fn reflow_program_node_count(&self) -> usize {
        self.reflow_plan
            .as_ref()
            .and_then(|plan| plan.as_ref().ok())
            .map_or(0, |plan| plan.template.node_count())
    }

    pub fn reflow_program_binding_text_node_count(&self) -> usize {
        self.reflow_plan
            .as_ref()
            .and_then(|plan| plan.as_ref().ok())
            .map_or(0, |plan| plan.template.binding_text_node_count())
    }

    pub fn reflow_program_html_binding_node_count(&self) -> usize {
        self.reflow_plan
            .as_ref()
            .and_then(|plan| plan.as_ref().ok())
            .map_or(0, |plan| plan.template.html_binding_node_count())
    }

    pub fn reflow_program_ready(&self) -> bool {
        self.reflow_plan.as_ref().is_some_and(|plan| plan.is_ok())
    }

    pub fn render_to_buffer(&self) -> Result<Vec<u8>, FullBleedError> {
        Ok(pdf::document_to_pdf_with_metrics_and_registry_with_logs(
            &self.document,
            None,
            Some(self.font_registry.as_ref()),
            &self.pdf_options,
            self.debug.clone(),
            self.perf.clone(),
        )?)
    }

    pub fn render_to_writer<W: std::io::Write>(
        &self,
        writer: &mut W,
    ) -> Result<usize, FullBleedError> {
        Ok(
            pdf::document_to_pdf_with_metrics_and_registry_to_writer_with_logs(
                &self.document,
                None,
                Some(self.font_registry.as_ref()),
                &self.pdf_options,
                writer,
                self.debug.clone(),
                self.perf.clone(),
            )?,
        )
    }

    pub fn render_to_file(
        &self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<usize, FullBleedError> {
        render_to_buffered_file(path, |writer| self.render_to_writer(writer))
    }

    /// Link `copies` of the compiled document into one ordered PDF without cloning the display
    /// command tree. For a one-page document this is the compiled pages-per-second lane.
    pub fn render_many_to_buffer(&self, copies: usize) -> Result<Vec<u8>, FullBleedError> {
        let mut out = Vec::new();
        self.render_many_to_writer(copies, &mut out)?;
        Ok(out)
    }

    pub fn render_many_to_writer<W: std::io::Write>(
        &self,
        copies: usize,
        writer: &mut W,
    ) -> Result<usize, FullBleedError> {
        if copies == 0 {
            return Err(FullBleedError::EmptyDocumentSet);
        }
        let mut pdf_stream = pdf::PdfStreamWriter::new(
            writer,
            self.document.page_size,
            Some(self.font_registry.as_ref()),
            self.pdf_options.clone(),
            self.debug.clone(),
            self.perf.clone(),
        )?;
        pdf_stream.add_compiled_document_copies(0, &self.document, copies)?;
        Ok(pdf_stream.finish()?)
    }

    fn ordered_binding_columns<'a>(
        &self,
        bindings: &'a HashMap<String, Vec<String>>,
    ) -> Result<(usize, Vec<&'a [String]>), FullBleedError> {
        Self::ordered_columns_for_slots(
            &self.binding_slots,
            bindings,
            "compiled document has no page-local {{slot}} text bindings",
        )
    }

    fn ordered_columns_for_slots<'a>(
        slots: &[String],
        bindings: &'a HashMap<String, Vec<String>>,
        empty_message: &str,
    ) -> Result<(usize, Vec<&'a [String]>), FullBleedError> {
        if slots.is_empty() {
            return Err(FullBleedError::InvalidConfiguration(
                empty_message.to_string(),
            ));
        }
        if bindings.len() != slots.len() {
            let unknown = bindings
                .keys()
                .filter(|name| !slots.contains(name))
                .cloned()
                .collect::<Vec<_>>();
            let missing = slots
                .iter()
                .filter(|name| !bindings.contains_key(*name))
                .cloned()
                .collect::<Vec<_>>();
            return Err(FullBleedError::InvalidConfiguration(format!(
                "binding columns do not match compiled slots (missing={missing:?}, unknown={unknown:?})"
            )));
        }

        let mut count = None;
        let mut ordered = Vec::with_capacity(slots.len());
        for slot in slots {
            let column = bindings.get(slot).ok_or_else(|| {
                FullBleedError::InvalidConfiguration(format!(
                    "missing binding column for slot {slot:?}"
                ))
            })?;
            match count {
                Some(expected) if expected != column.len() => {
                    return Err(FullBleedError::InvalidConfiguration(format!(
                        "binding column {slot:?} has {} rows; expected {expected}",
                        column.len()
                    )));
                }
                None => count = Some(column.len()),
                _ => {}
            }
            ordered.push(column.as_slice());
        }
        let count = count.unwrap_or(0);
        if count == 0 {
            return Err(FullBleedError::EmptyDocumentSet);
        }
        Ok((count, ordered))
    }

    /// Execute page-local, fixed-geometry text slots over a columnar record batch.
    ///
    /// Static page paint is linked once. Each record receives a distinct, uncompressed text
    /// overlay stream, so values vary without reparsing HTML or rerunning layout.
    pub fn render_bindings_to_buffer(
        &self,
        bindings: &HashMap<String, Vec<String>>,
    ) -> Result<Vec<u8>, FullBleedError> {
        let (record_count, columns) = self.ordered_binding_columns(bindings)?;
        let estimated = record_count.saturating_mul(512).saturating_add(64 * 1024);
        let mut out = Vec::with_capacity(estimated);
        self.render_bindings_to_writer_ordered(record_count, &columns, &mut out, None)?;
        Ok(out)
    }

    /// Execute fixed-geometry bindings with cooperative row-boundary cancellation.
    pub fn render_bindings_to_buffer_cancellable(
        &self,
        bindings: &HashMap<String, Vec<String>>,
        cancellation: &AuthoringCancellationToken,
    ) -> Result<Vec<u8>, FullBleedError> {
        let (record_count, columns) = self.ordered_binding_columns(bindings)?;
        let estimated = record_count.saturating_mul(512).saturating_add(64 * 1024);
        let mut out = Vec::with_capacity(estimated);
        self.render_bindings_to_writer_ordered(
            record_count,
            &columns,
            &mut out,
            Some(cancellation),
        )?;
        Ok(out)
    }

    fn render_bindings_to_writer_ordered<W: std::io::Write>(
        &self,
        record_count: usize,
        columns: &[&[String]],
        writer: &mut W,
        cancellation: Option<&AuthoringCancellationToken>,
    ) -> Result<usize, FullBleedError> {
        let binding_plan = self
            .binding_plan
            .as_ref()
            .ok_or_else(|| {
                FullBleedError::InvalidConfiguration(
                    "compiled document has no binding command plan".to_string(),
                )
            })?
            .as_ref()
            .map_err(|error| FullBleedError::InvalidConfiguration(error.to_string()))?;
        let mut pdf_stream = pdf::PdfStreamWriter::new(
            writer,
            self.document.page_size,
            Some(self.font_registry.as_ref()),
            self.pdf_options.clone(),
            self.debug.clone(),
            self.perf.clone(),
        )?;
        let result = pdf_stream.add_compiled_document_bindings(
            0,
            &self.document,
            binding_plan,
            columns,
            record_count,
            || cancellation.is_some_and(AuthoringCancellationToken::is_cancelled),
        );
        if result
            .as_ref()
            .is_err_and(|error| error.kind() == std::io::ErrorKind::Interrupted)
        {
            return Err(FullBleedError::Cancelled);
        }
        result?;
        Ok(pdf_stream.finish()?)
    }

    pub fn render_bindings_to_writer<W: std::io::Write>(
        &self,
        bindings: &HashMap<String, Vec<String>>,
        writer: &mut W,
    ) -> Result<usize, FullBleedError> {
        let (record_count, columns) = self.ordered_binding_columns(bindings)?;
        self.render_bindings_to_writer_ordered(record_count, &columns, writer, None)
    }

    pub fn render_bindings_to_file(
        &self,
        bindings: &HashMap<String, Vec<String>>,
        path: impl AsRef<std::path::Path>,
    ) -> Result<usize, FullBleedError> {
        render_to_buffered_file(path, |writer| {
            self.render_bindings_to_writer(bindings, writer)
        })
    }

    pub fn reflow_program_error(&self) -> Option<&str> {
        self.reflow_plan
            .as_ref()
            .and_then(|plan| plan.as_ref().err())
            .map(AsRef::as_ref)
    }

    fn compiled_reflow_plan(&self) -> Result<&CompiledReflowPlan, FullBleedError> {
        self.reflow_plan
            .as_ref()
            .ok_or_else(|| {
                FullBleedError::InvalidConfiguration(
                    "compiled document has no reflow-capable {{slot}} text bindings".to_string(),
                )
            })?
            .as_ref()
            .map(AsRef::as_ref)
            .map_err(|error| FullBleedError::InvalidConfiguration(error.to_string()))
    }

    /// Execute size-changing text slots through shaping, layout, and pagination.
    ///
    /// HTML tokenization, tree recovery, binding discovery, CSS parsing, and selector compilation
    /// are compile-time work. Each worker materializes one private DOM, mutates its bound text
    /// nodes per record, and emits completed documents to the ordered streaming PDF linker.
    pub fn render_reflow_bindings_to_buffer(
        &self,
        bindings: &HashMap<String, Vec<String>>,
    ) -> Result<Vec<u8>, FullBleedError> {
        self.render_reflow_bindings_to_buffer_with_options(
            bindings,
            CompiledReflowOptions::default(),
        )
    }

    /// Execute compiled content reflow with explicit per-job options.
    pub fn render_reflow_bindings_to_buffer_with_options(
        &self,
        bindings: &HashMap<String, Vec<String>>,
        options: CompiledReflowOptions,
    ) -> Result<Vec<u8>, FullBleedError> {
        let plan = self.compiled_reflow_plan()?;
        let (record_count, columns) = Self::ordered_columns_for_slots(
            plan.template.slot_names(),
            bindings,
            "compiled document has no reflow-capable {{slot}} text bindings",
        )?;
        let estimated = record_count.saturating_mul(4096).saturating_add(64 * 1024);
        let mut out = Vec::with_capacity(estimated);
        self.render_reflow_bindings_to_writer_ordered(
            plan,
            record_count,
            &columns,
            &mut out,
            options,
            None,
        )?;
        Ok(out)
    }

    /// Execute compiled reflow with explicit options and cooperative cancellation.
    pub fn render_reflow_bindings_to_buffer_with_options_cancellable(
        &self,
        bindings: &HashMap<String, Vec<String>>,
        options: CompiledReflowOptions,
        cancellation: &AuthoringCancellationToken,
    ) -> Result<Vec<u8>, FullBleedError> {
        let plan = self.compiled_reflow_plan()?;
        let (record_count, columns) = Self::ordered_columns_for_slots(
            plan.template.slot_names(),
            bindings,
            "compiled document has no reflow-capable {{slot}} text bindings",
        )?;
        let estimated = record_count.saturating_mul(4096).saturating_add(64 * 1024);
        let mut out = Vec::with_capacity(estimated);
        self.render_reflow_bindings_to_writer_ordered(
            plan,
            record_count,
            &columns,
            &mut out,
            options,
            Some(cancellation),
        )?;
        Ok(out)
    }

    fn render_reflow_bindings_to_writer_ordered<W: std::io::Write>(
        &self,
        plan: &CompiledReflowPlan,
        record_count: usize,
        columns: &[&[String]],
        writer: &mut W,
        options: CompiledReflowOptions,
        cancellation: Option<&AuthoringCancellationToken>,
    ) -> Result<usize, FullBleedError> {
        use std::collections::BTreeMap;
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc::{self, TrySendError};

        if cancellation.is_some_and(AuthoringCancellationToken::is_cancelled) {
            return Err(FullBleedError::Cancelled);
        }
        let started = std::time::Instant::now();
        let page_size = plan
            .context
            .page_templates
            .first()
            .ok_or(FullBleedError::MissingPageTemplate)?
            .page_size;
        let mut pdf_options = self.pdf_options.clone();
        pdf_options.compiled_flow_compression = options.compression;
        let mut pdf_stream = pdf::PdfStreamWriter::new(
            writer,
            page_size,
            Some(self.font_registry.as_ref()),
            pdf_options,
            self.debug.clone(),
            self.perf.clone(),
        )?;
        if let Some(cache) = plan
            .pdf_program_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .filter(|(cached_records, _)| *cached_records == record_count)
            .map(|(_, cache)| cache.clone())
        {
            pdf_stream.use_compiled_flow_program_cache(cache);
        }
        let pdf_program_cache = pdf_stream.compiled_flow_program_cache();
        let compress_content_streams = self.pdf_options.compress_content_streams;
        let compress_content_stream_min_bytes = self.pdf_options.compress_content_stream_min_bytes;

        let worker_count = crate::parallel::current_num_threads()
            .max(1)
            .min(record_count);
        let buffer_cap = worker_count.saturating_mul(4).clamp(1, 256);
        let (sender, receiver) = mpsc::sync_channel::<(
            usize,
            Result<CompiledFlowBoundRecord, FullBleedError>,
        )>(buffer_cap);
        let cancelled = AtomicBool::new(false);
        // Seed only a bounded window of row indexes. The linker releases exactly one new job for
        // every row it commits, so rendering, the result channel, and ordered reassembly together
        // can never advance more than `buffer_cap` records beyond linked output. A shared job
        // receiver retains dynamic load balancing without a scheduler lock on the linker path.
        let (job_sender, job_receiver) = mpsc::channel::<usize>();
        let initial_jobs = buffer_cap.min(record_count);
        for row in 0..initial_jobs {
            job_sender
                .send(row)
                .expect("compiled reflow job receiver exists before workers start");
        }
        let job_receiver = Mutex::new(job_receiver);
        let mut next_row_to_schedule = initial_jobs;
        let mut render_error = None;
        let mut total_pages = 0usize;

        std::thread::scope(|scope| {
            for _worker in 0..worker_count {
                let sender = sender.clone();
                let cancelled = &cancelled;
                let external_cancellation = cancellation;
                let job_receiver = &job_receiver;
                let pdf_program_cache = pdf_program_cache.clone();
                scope.spawn(move || {
                    let mut document = plan.template.instantiate();
                    let mut worker_glyphs = CompiledFlowWorkerGlyphs::new();
                    loop {
                        if external_cancellation
                            .is_some_and(AuthoringCancellationToken::is_cancelled)
                        {
                            cancelled.store(true, Ordering::Release);
                            return;
                        }
                        let row = match job_receiver
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .recv()
                        {
                            Ok(row) => row,
                            Err(_) => return,
                        };
                        if cancelled.load(Ordering::Acquire) {
                            return;
                        }
                        let rendered = plan
                            .render_compiled_flow_record(
                                row,
                                &mut document,
                                &mut worker_glyphs,
                                columns,
                            )
                            .and_then(|mut document| {
                                if let Some(encoded) = pdf::encode_compiled_flow_document(
                                    pdf_program_cache.as_ref(),
                                    document.program.document.as_ref(),
                                    &document.page_overrides,
                                    compress_content_streams,
                                    compress_content_stream_min_bytes,
                                    options.compression,
                                ) {
                                    document.encoded_pages =
                                        Some(encoded.map_err(FullBleedError::Io)?);
                                }
                                Ok(document)
                            });
                        if external_cancellation
                            .is_some_and(AuthoringCancellationToken::is_cancelled)
                        {
                            cancelled.store(true, Ordering::Release);
                            return;
                        }
                        let mut message = (row, rendered);
                        loop {
                            match sender.try_send(message) {
                                Ok(()) => break,
                                Err(TrySendError::Full(returned)) => {
                                    if cancelled.load(Ordering::Acquire)
                                        || external_cancellation
                                            .is_some_and(AuthoringCancellationToken::is_cancelled)
                                    {
                                        cancelled.store(true, Ordering::Release);
                                        return;
                                    }
                                    message = returned;
                                    std::thread::yield_now();
                                }
                                Err(TrySendError::Disconnected(_)) => return,
                            }
                        }
                    }
                });
            }
            drop(sender);

            let mut pending = BTreeMap::new();
            let mut next_row = 0usize;
            while next_row < record_count {
                if cancellation.is_some_and(AuthoringCancellationToken::is_cancelled) {
                    render_error = Some(FullBleedError::Cancelled);
                    break;
                }
                let message = match receiver.recv_timeout(std::time::Duration::from_millis(25)) {
                    Ok(message) => message,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        if cancellation.is_some_and(AuthoringCancellationToken::is_cancelled) {
                            render_error = Some(FullBleedError::Cancelled);
                            break;
                        }
                        render_error = Some(FullBleedError::Io(std::io::Error::new(
                            std::io::ErrorKind::BrokenPipe,
                            "compiled reflow worker channel closed early",
                        )));
                        break;
                    }
                };
                if cancellation.is_some_and(AuthoringCancellationToken::is_cancelled) {
                    render_error = Some(FullBleedError::Cancelled);
                    break;
                }
                let (row, result) = message;
                match result {
                    Ok(document) => {
                        pending.insert(row, document);
                    }
                    Err(error) => {
                        render_error = Some(error);
                        break;
                    }
                }
                while let Some(document) = pending.remove(&next_row) {
                    if cancellation.is_some_and(AuthoringCancellationToken::is_cancelled) {
                        render_error = Some(FullBleedError::Cancelled);
                        break;
                    }
                    total_pages = total_pages.saturating_add(document.page_count());
                    if let Err(error) = pdf_stream.add_compiled_flow_document(
                        next_row,
                        document.program.document.as_ref(),
                        document.page_overrides,
                        document.font_glyphs,
                        document.encoded_pages,
                    ) {
                        render_error = Some(FullBleedError::Io(error));
                        break;
                    }
                    next_row = next_row.saturating_add(1);
                    if next_row_to_schedule < record_count {
                        if job_sender.send(next_row_to_schedule).is_err() {
                            render_error = Some(FullBleedError::Io(std::io::Error::new(
                                std::io::ErrorKind::BrokenPipe,
                                "compiled reflow workers stopped accepting scheduled records",
                            )));
                            break;
                        }
                        next_row_to_schedule = next_row_to_schedule.saturating_add(1);
                    }
                }
                if render_error.is_some() {
                    break;
                }
            }
            cancelled.store(true, Ordering::Release);
            drop(job_sender);
        });

        if cancellation.is_some_and(AuthoringCancellationToken::is_cancelled) {
            return Err(FullBleedError::Cancelled);
        }
        if let Some(error) = render_error {
            return Err(error);
        }
        let replay_cache = pdf_stream.compiled_flow_replay_cache();
        let bytes_written = pdf_stream.finish()?;
        *plan
            .pdf_program_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((record_count, replay_cache));
        if let Some(perf) = self.perf.as_deref() {
            perf.log_span_ms(
                "compile.reflow.batch",
                None,
                started.elapsed().as_secs_f64() * 1000.0,
            );
            perf.log_counts(
                "compile.reflow.batch",
                None,
                &[
                    ("records", record_count as u64),
                    ("pages", total_pages as u64),
                    ("workers", worker_count as u64),
                    ("buffer_cap", buffer_cap as u64),
                    ("bytes", bytes_written as u64),
                    (
                        "compression_compact",
                        u64::from(options.compression == CompiledFlowCompression::Compact),
                    ),
                ],
            );
        }
        Ok(bytes_written)
    }

    pub fn render_reflow_bindings_to_writer<W: std::io::Write>(
        &self,
        bindings: &HashMap<String, Vec<String>>,
        writer: &mut W,
    ) -> Result<usize, FullBleedError> {
        self.render_reflow_bindings_to_writer_with_options(
            bindings,
            writer,
            CompiledReflowOptions::default(),
        )
    }

    pub fn render_reflow_bindings_to_writer_with_options<W: std::io::Write>(
        &self,
        bindings: &HashMap<String, Vec<String>>,
        writer: &mut W,
        options: CompiledReflowOptions,
    ) -> Result<usize, FullBleedError> {
        let plan = self.compiled_reflow_plan()?;
        let (record_count, columns) = Self::ordered_columns_for_slots(
            plan.template.slot_names(),
            bindings,
            "compiled document has no reflow-capable {{slot}} text bindings",
        )?;
        self.render_reflow_bindings_to_writer_ordered(
            plan,
            record_count,
            &columns,
            writer,
            options,
            None,
        )
    }

    pub fn render_reflow_bindings_to_file(
        &self,
        bindings: &HashMap<String, Vec<String>>,
        path: impl AsRef<std::path::Path>,
    ) -> Result<usize, FullBleedError> {
        self.render_reflow_bindings_to_file_with_options(
            bindings,
            path,
            CompiledReflowOptions::default(),
        )
    }

    pub fn render_reflow_bindings_to_file_with_options(
        &self,
        bindings: &HashMap<String, Vec<String>>,
        path: impl AsRef<std::path::Path>,
        options: CompiledReflowOptions,
    ) -> Result<usize, FullBleedError> {
        render_to_buffered_file(path, |writer| {
            self.render_reflow_bindings_to_writer_with_options(bindings, writer, options)
        })
    }
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

#[derive(Clone)]
struct RenderContext {
    resolver: Arc<style::StyleResolver>,
    page_templates: Arc<[PageTemplate]>,
}

struct ResolvedCssPageContext {
    page_size: Size,
    base_margins: Margins,
    page_margins: std::collections::BTreeMap<usize, Margins>,
    page_styles: style::CssPageStyles,
}

#[derive(Clone)]
struct PageRootTextContext {
    style: TextStyle,
    line_height: Option<style::CssPageLineHeight>,
}

const RENDER_CONTEXT_CACHE_MAX_ENTRIES: usize = 32;

struct RenderContextCache {
    map: HashMap<String, RenderContext>,
    order: VecDeque<String>,
}

impl RenderContextCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&self, css: &str) -> Option<RenderContext> {
        self.map.get(css).cloned()
    }

    fn insert(&mut self, css: String, context: RenderContext) {
        if self.map.contains_key(&css) {
            return;
        }
        self.order.push_back(css.clone());
        self.map.insert(css, context);
        while self.map.len() > RENDER_CONTEXT_CACHE_MAX_ENTRIES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.map.remove(&oldest);
        }
    }
}

struct LayoutBuildResult {
    document: Document,
    story_ms: f64,
    layout_ms: f64,
}

#[derive(Clone, Copy)]
enum HtmlLayoutInput<'a> {
    Source(&'a str),
    Parsed(&'a NodeRef),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutStrategy {
    Eager,
    Lazy,
}

#[derive(Debug, Clone)]
pub struct A11yVerifierEvidence {
    pub selector: Option<String>,
    pub values: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct HtmlTableFacts {
    pub has_caption: bool,
    pub th_count: usize,
    pub th_scope_count: usize,
}

#[derive(Debug, Clone)]
pub struct A11yVerifierFinding {
    pub rule_id: String,
    pub applicability: String,
    pub verification_mode: String,
    pub verdict: String,
    pub severity: String,
    pub confidence: String,
    pub stage: String,
    pub source: String,
    pub message: String,
    pub evidence: Vec<A11yVerifierEvidence>,
}

#[derive(Debug, Clone)]
pub struct A11yVerifierFacts {
    pub html_lang: Option<String>,
    pub title: String,
    pub part_lang_attr_count: usize,
    pub invalid_part_lang_attr_count: usize,
    pub main_count: usize,
    pub duplicate_ids: Vec<String>,
    pub missing_idrefs: Vec<(String, String)>,
    pub has_html_wrapper: bool,
    pub has_css_link: bool,
    pub css_link_hrefs: Vec<String>,
    pub signature_semantic_count: usize,
    pub empty_heading_count: usize,
    pub empty_label_count: usize,
    pub empty_aria_label_count: usize,
    pub unlabeled_region_count: usize,
    pub image_count: usize,
    pub image_missing_alt_count: usize,
    pub image_title_only_count: usize,
    pub image_semantic_conflict_count: usize,
    pub figure_informative_count: usize,
    pub figure_alt_length_budget: usize,
    pub figure_alt_over_budget_count: usize,
    pub figure_max_alt_len: usize,
    pub figure_caption_redundancy_threshold: f64,
    pub figure_caption_redundancy_count: usize,
    pub figure_max_caption_similarity: f64,
    pub figure_missing_effective_text_count: usize,
    pub dl_block_count: usize,
    pub dl_fragmentation_count: usize,
    pub dl_group_consistency_count: usize,
    pub redundant_role_native_count: usize,
    pub redundant_state_native_count: usize,
    pub form_control_count: usize,
    pub unlabeled_form_control_count: usize,
    pub invalid_form_control_count: usize,
    pub unidentified_error_form_control_count: usize,
    pub tabindex_attr_count: usize,
    pub positive_tabindex_count: usize,
    pub invalid_tabindex_count: usize,
    pub link_count: usize,
    pub unnamed_link_count: usize,
    pub generic_link_text_count: usize,
    pub custom_click_handler_count: usize,
    pub pointer_only_click_handler_count: usize,
    pub script_element_count: usize,
    pub embedded_active_content_count: usize,
    pub autoplay_media_count: usize,
    pub blink_marquee_count: usize,
    pub inline_event_handler_attr_count: usize,
    pub meta_refresh_count: usize,
    pub tables: Vec<HtmlTableFacts>,
    pub body_text: String,
}

#[derive(Debug, Clone)]
pub struct A11yVerifierCoreReport {
    pub profile: String,
    pub findings: Vec<A11yVerifierFinding>,
    pub facts: A11yVerifierFacts,
}

#[derive(Debug, Clone, Default)]
pub struct PaginationTraceSummary {
    pub page_count: Option<i64>,
    pub event_count: Option<i64>,
    pub transition_count: Option<i64>,
    pub page_transition_count: Option<i64>,
    pub frame_transition_count: Option<i64>,
    pub placement_count: Option<i64>,
    pub split_count: Option<i64>,
    pub overflow_event_count: Option<i64>,
    pub recoverable_overflow_count: Option<i64>,
    pub fatal_overflow_count: Option<i64>,
    pub low_coverage_page_count: Option<i64>,
    pub flowable_overlap_count: Option<i64>,
    pub text_overlap_count: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct PmrCoreEvidence {
    pub selector: Option<String>,
    pub diagnostic_ref: Option<String>,
    pub values: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct PmrCoreAudit {
    pub audit_id: String,
    pub category: String,
    pub weight: f64,
    pub class_name: String,
    pub verification_mode: String,
    pub severity: String,
    pub stage: String,
    pub source: String,
    pub verdict: String,
    pub scored: bool,
    pub score: Option<f64>,
    pub message: String,
    pub fix_hint: Option<String>,
    pub evidence: Vec<PmrCoreEvidence>,
}

#[derive(Debug, Clone)]
pub struct PmrCoreCategory {
    pub id: String,
    pub name: String,
    pub weight: f64,
    pub score: f64,
    pub confidence: f64,
    pub audit_count: usize,
    pub fail_count: usize,
    pub warn_count: usize,
}

#[derive(Debug, Clone)]
pub struct PmrCoreManualDebtItem {
    pub id: String,
    pub reason: String,
    pub severity: String,
    pub category: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PmrCoreGate {
    pub ok: bool,
    pub mode: String,
    pub error_count: usize,
    pub warn_count: usize,
    pub failed_audit_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PmrCoreRank {
    pub score: f64,
    pub confidence: f64,
    pub band: String,
    pub raw_score: f64,
}

#[derive(Debug, Clone)]
pub struct PmrCoreCoverage {
    pub evaluated_audit_count: usize,
    pub applicable_audit_count: usize,
    pub scored_audit_count: usize,
    pub manual_needed_count: usize,
    pub not_evaluated_audit_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct PmrCoreContext {
    pub overflow_count: Option<i64>,
    pub known_loss_count: Option<i64>,
    pub pagination_summary: Option<PaginationTraceSummary>,
    pub source_page_count: Option<i64>,
    pub render_page_count: Option<i64>,
    pub review_queue_items: Option<i64>,
    pub html_artifact_bytes: Option<u64>,
    pub css_artifact_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct PmrCoreReport {
    pub profile: String,
    pub mode: String,
    pub audits: Vec<PmrCoreAudit>,
    pub categories: Vec<PmrCoreCategory>,
    pub manual_debt_item_count: usize,
    pub manual_debt_high_risk_count: usize,
    pub manual_debt_items: Vec<PmrCoreManualDebtItem>,
    pub coverage: PmrCoreCoverage,
    pub rank: PmrCoreRank,
    pub gate: PmrCoreGate,
    pub facts: A11yVerifierFacts,
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

fn document_target_pages(doc: &Document) -> HashMap<String, usize> {
    fn collect_target_ids(
        commands: &[Command],
        page_number: usize,
        targets: &mut HashMap<String, usize>,
    ) {
        for command in commands {
            match command {
                Command::Meta { key, value } if key == "fb.owner.id" => {
                    if !value.is_empty() {
                        targets.entry(value.clone()).or_insert(page_number);
                    }
                }
                Command::DefineForm { commands, .. }
                | Command::DefineIsolatedForm { commands, .. } => {
                    collect_target_ids(commands, page_number, targets);
                }
                _ => {}
            }
        }
    }

    let mut targets = HashMap::new();
    for (page_index, page) in doc.pages.iter().enumerate() {
        collect_target_ids(&page.commands, page_index + 1, &mut targets);
    }
    targets
}

fn apply_html_page_shrink_to_fit(doc: &mut Document) {
    fn html_page_area(commands: &[Command]) -> Option<Rect> {
        commands.iter().find_map(|command| {
            let Command::Meta { key, value } = command else {
                return None;
            };
            if key != canvas::META_HTML_PAGE_AREA_KEY {
                return None;
            }
            let mut values = value.split(',').filter_map(|part| part.parse::<i64>().ok());
            let rect = Rect {
                x: Pt::from_milli_i64(values.next()?),
                y: Pt::from_milli_i64(values.next()?),
                width: Pt::from_milli_i64(values.next()?),
                height: Pt::from_milli_i64(values.next()?),
            };
            (values.next().is_none() && rect.width > Pt::ZERO && rect.height > Pt::ZERO)
                .then_some(rect)
        })
    }

    fn max_scrollable_right_milli(commands: &[Command], max_right: &mut i64) {
        for command in commands {
            match command {
                Command::Meta { key, value } if key == canvas::META_HTML_SCROLLABLE_RIGHT_KEY => {
                    if let Ok(right) = value.parse::<i64>() {
                        *max_right = (*max_right).max(right);
                    }
                }
                Command::DefineForm { commands, .. }
                | Command::DefineIsolatedForm { commands, .. } => {
                    max_scrollable_right_milli(commands, max_right);
                }
                _ => {}
            }
        }
    }

    fn min_scrollable_top_milli(commands: &[Command], min_top: &mut Option<i64>) {
        for command in commands {
            match command {
                Command::Meta { key, value } if key == canvas::META_HTML_SCROLLABLE_TOP_KEY => {
                    if let Ok(top) = value.parse::<i64>() {
                        *min_top = Some(min_top.map_or(top, |current| current.min(top)));
                    }
                }
                Command::DefineForm { commands, .. }
                | Command::DefineIsolatedForm { commands, .. } => {
                    min_scrollable_top_milli(commands, min_top);
                }
                _ => {}
            }
        }
    }

    fn max_scrollable_bottom_milli(commands: &[Command], max_bottom: &mut Option<i64>) {
        for command in commands {
            match command {
                Command::Meta { key, value } if key == canvas::META_HTML_SCROLLABLE_BOTTOM_KEY => {
                    if let Ok(bottom) = value.parse::<i64>() {
                        *max_bottom =
                            Some(max_bottom.map_or(bottom, |current| current.max(bottom)));
                    }
                }
                Command::DefineForm { commands, .. }
                | Command::DefineIsolatedForm { commands, .. } => {
                    max_scrollable_bottom_milli(commands, max_bottom);
                }
                _ => {}
            }
        }
    }

    #[derive(Clone)]
    struct HtmlBlockEndOverflowScope {
        owner: String,
        start: usize,
        end: usize,
    }

    struct OpenDiagnosticScope {
        start: usize,
        owner: Option<String>,
        max_bottom_milli: Option<i64>,
    }

    fn html_block_end_overflow_scopes(
        commands: &[Command],
        allowed_bottom_milli: i64,
    ) -> Vec<HtmlBlockEndOverflowScope> {
        let mut stack: Vec<OpenDiagnosticScope> = Vec::new();
        let mut scopes = Vec::new();
        for (index, command) in commands.iter().enumerate() {
            match command {
                Command::Meta { key, .. } if key == canvas::META_DIAGNOSTIC_SCOPE_BEGIN_KEY => {
                    stack.push(OpenDiagnosticScope {
                        start: index,
                        owner: None,
                        max_bottom_milli: None,
                    });
                }
                Command::Meta { key, value } if key == "fb.owner.dom_path" => {
                    if let Some(scope) = stack.last_mut() {
                        scope.owner = Some(value.clone());
                    }
                }
                Command::Meta { key, value } if key == canvas::META_HTML_SCROLLABLE_BOTTOM_KEY => {
                    if let (Some(scope), Ok(bottom)) = (stack.last_mut(), value.parse::<i64>()) {
                        scope.max_bottom_milli = Some(
                            scope
                                .max_bottom_milli
                                .map_or(bottom, |current| current.max(bottom)),
                        );
                    }
                }
                Command::Meta { key, .. } if key == canvas::META_DIAGNOSTIC_SCOPE_END_KEY => {
                    let Some(scope) = stack.pop() else {
                        continue;
                    };
                    if scope
                        .max_bottom_milli
                        .is_some_and(|bottom| bottom > allowed_bottom_milli)
                    {
                        if let Some(owner) = scope.owner {
                            scopes.push(HtmlBlockEndOverflowScope {
                                owner,
                                start: scope.start,
                                end: index,
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        // If both a transformed ancestor and descendant cross the fragment
        // edge, replaying the ancestor already includes the descendant's paint.
        // Keep only the outermost overflowing visual scopes.
        scopes.sort_by(|left, right| {
            left.start
                .cmp(&right.start)
                .then_with(|| right.end.cmp(&left.end))
        });
        let mut outermost: Vec<HtmlBlockEndOverflowScope> = Vec::new();
        for scope in scopes {
            if outermost
                .iter()
                .any(|parent| parent.start <= scope.start && parent.end >= scope.end)
            {
                continue;
            }
            outermost.push(scope);
        }
        outermost
    }

    fn diagnostic_owner_scope_start(commands: &[Command], owner: &str) -> Option<usize> {
        let mut scope_starts = Vec::new();
        for (index, command) in commands.iter().enumerate() {
            match command {
                Command::Meta { key, .. } if key == canvas::META_DIAGNOSTIC_SCOPE_BEGIN_KEY => {
                    scope_starts.push(index);
                }
                Command::Meta { key, value } if key == "fb.owner.dom_path" && value == owner => {
                    if let Some(start) = scope_starts.last() {
                        return Some(*start);
                    }
                }
                Command::Meta { key, .. } if key == canvas::META_DIAGNOSTIC_SCOPE_END_KEY => {
                    scope_starts.pop();
                }
                _ => {}
            }
        }
        None
    }

    fn visual_replay_commands(commands: &[Command]) -> Vec<Command> {
        commands
            .iter()
            .filter_map(|command| match command {
                // A cross-fragment paint replay is an artifact of the owning
                // element, not a second semantic occurrence.
                Command::Meta { .. }
                | Command::BeginTag { .. }
                | Command::BeginTagActualText { .. }
                | Command::EndTag => None,
                Command::DefineForm {
                    resource_id,
                    width,
                    height,
                    commands,
                } => Some(Command::DefineForm {
                    resource_id: resource_id.clone(),
                    width: *width,
                    height: *height,
                    commands: visual_replay_commands(commands),
                }),
                Command::DefineIsolatedForm {
                    resource_id,
                    width,
                    height,
                    commands,
                } => Some(Command::DefineIsolatedForm {
                    resource_id: resource_id.clone(),
                    width: *width,
                    height: *height,
                    commands: visual_replay_commands(commands),
                }),
                _ => Some(command.clone()),
            })
            .collect()
    }

    fn extend_html_canvas_background_block_end(commands: &mut [Command], guard: Pt) {
        let mut scope_depth = 0usize;
        for command in commands {
            match command {
                Command::Meta { key, value }
                    if key == canvas::META_HTML_CANVAS_BACKGROUND_KEY && value == "begin" =>
                {
                    scope_depth = scope_depth.saturating_add(1);
                }
                Command::Meta { key, value }
                    if key == canvas::META_HTML_CANVAS_BACKGROUND_KEY && value == "end" =>
                {
                    scope_depth = scope_depth.saturating_sub(1);
                }
                Command::DrawRect { height, .. } if scope_depth > 0 => {
                    *height += guard;
                }
                _ => {}
            }
        }
    }

    #[derive(Clone, Copy)]
    struct HtmlPageFit {
        page_width_milli: i64,
        page_size: Size,
        page_area: Rect,
        max_right_milli: i64,
        min_top_milli: Option<i64>,
        max_bottom_milli: Option<i64>,
        local_scale: f32,
        content_start: usize,
    }

    let default_page_size = doc.page_size;
    let page_fits: Vec<_> = doc
        .pages
        .iter()
        .map(|page| {
            let named_page_size = page
                .commands
                .iter()
                .rev()
                .find_map(|command| match command {
                    Command::Meta { key, value } if key == canvas::META_PAGE_SIZE_KEY => {
                        value.split_once(',').and_then(|(width, height)| {
                            Some((width.parse::<i64>().ok()?, height.parse::<i64>().ok()?))
                        })
                    }
                    _ => None,
                })
                .unwrap_or((
                    default_page_size.width.to_milli_i64(),
                    default_page_size.height.to_milli_i64(),
                ));
            let page_width_milli = named_page_size.0;
            let page_size = Size {
                width: Pt::from_milli_i64(named_page_size.0),
                height: Pt::from_milli_i64(named_page_size.1),
            };
            let page_area = html_page_area(&page.commands).unwrap_or(Rect {
                x: Pt::ZERO,
                y: Pt::ZERO,
                width: Pt::from_milli_i64(page_width_milli),
                height: Pt::from_milli_i64(named_page_size.1),
            });
            let allowed_right_milli = (page_area.x + page_area.width).to_milli_i64();
            let mut max_right_milli = allowed_right_milli;
            max_scrollable_right_milli(&page.commands, &mut max_right_milli);
            let mut min_top_milli = None;
            min_scrollable_top_milli(&page.commands, &mut min_top_milli);
            let mut max_bottom_milli = None;
            max_scrollable_bottom_milli(&page.commands, &mut max_bottom_milli);
            let overflow_width_milli = max_right_milli - page_area.x.to_milli_i64();
            let local_scale = if page_width_milli > 0
                && max_right_milli > allowed_right_milli
                && overflow_width_milli > 0
            {
                (page_area.width.to_milli_i64() as f64 / overflow_width_milli as f64) as f32
            } else {
                1.0
            };
            let content_start = page
                .commands
                .iter()
                .position(|command| {
                    matches!(command, Command::Meta { key, .. } if key == META_PAGE_TEMPLATE_KEY)
                })
                .map_or(0, |index| index + 1);
            HtmlPageFit {
                page_width_milli,
                page_size,
                page_area,
                max_right_milli,
                min_top_milli,
                max_bottom_milli,
                local_scale,
                content_start,
            }
        })
        .collect();
    let scale = page_fits
        .iter()
        .map(|fit| fit.local_scale)
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .fold(1.0_f32, f32::min);
    let should_scale = (0.0..1.0).contains(&scale);
    let has_previous_fragment_overflow = page_fits.iter().skip(1).any(|fit| {
        fit.min_top_milli
            .is_some_and(|top| top < fit.page_area.y.to_milli_i64())
    });
    let has_next_fragment_overflow = page_fits
        .iter()
        .take(page_fits.len().saturating_sub(1))
        .any(|fit| {
            fit.max_bottom_milli.is_some_and(|bottom| {
                bottom > (fit.page_area.y + fit.page_area.height).to_milli_i64()
            })
        });
    if !should_scale && !has_previous_fragment_overflow && !has_next_fragment_overflow {
        return;
    }

    if should_scale {
        // Blink paints the propagated root canvas through its fitted surface.
        // Retain the resulting quarter-point block-end coverage phase while
        // leaving @page backgrounds and margin boxes outside the fit program.
        let block_end_guard = Pt::from_f32(0.25);
        for page in &mut doc.pages {
            extend_html_canvas_background_block_end(&mut page.commands, block_end_guard);
        }
    }

    let pristine_contents: Vec<_> = doc
        .pages
        .iter()
        .zip(&page_fits)
        .map(|(page, fit)| page.commands[fit.content_start..].to_vec())
        .collect();
    let mut original_contents = pristine_contents.clone();

    // A transformed fragment's block-end visual overflow paints on the next
    // fragmentainer before that element's continuation. Key the carry layer by
    // the stable DOM owner scope retained across split clones, so unrelated
    // transformed elements cannot reorder one another. The replay stays as
    // display-list bytecode and inherits the destination page's single global
    // fit transform; no raster surface or second layout pass is required.
    for source_index in 0..page_fits.len().saturating_sub(1) {
        let source_fit = page_fits[source_index];
        let source_bottom = source_fit.page_area.y + source_fit.page_area.height;
        if !source_fit
            .max_bottom_milli
            .is_some_and(|bottom| bottom > source_bottom.to_milli_i64())
        {
            continue;
        }
        let destination_index = source_index + 1;
        let destination_fit = page_fits[destination_index];
        let scopes = html_block_end_overflow_scopes(
            &pristine_contents[source_index],
            source_bottom.to_milli_i64(),
        );
        for scope in scopes {
            let Some(insert_at) =
                diagnostic_owner_scope_start(&original_contents[destination_index], &scope.owner)
            else {
                continue;
            };
            let replay =
                visual_replay_commands(&pristine_contents[source_index][scope.start..=scope.end]);
            if replay.is_empty() {
                continue;
            }
            let destination_area = destination_fit.page_area;
            let source_area = source_fit.page_area;
            let mut carry = Vec::with_capacity(replay.len() + 6);
            carry.push(Command::SaveState);
            carry.push(Command::ClipRect {
                x: destination_area.x,
                y: destination_area.y,
                width: destination_area.width,
                height: destination_area.height,
            });
            carry.push(Command::BeginArtifact { subtype: None });
            carry.push(Command::ConcatMatrix {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e: destination_area.x - source_area.x,
                f: destination_area.y - source_area.y - source_area.height,
            });
            carry.extend(replay);
            carry.push(Command::EndMarkedContent);
            carry.push(Command::RestoreState);
            original_contents[destination_index].splice(insert_at..insert_at, carry);
        }
    }

    // Chromium chooses one document-wide print fit, then anchors it to each
    // page's own asymmetric page area. Overflow discovered on a later page
    // therefore scales earlier pages by the same amount.
    if !should_scale {
        for (page_index, (page, fit)) in doc.pages.iter_mut().zip(&page_fits).enumerate() {
            page.commands.truncate(fit.content_start);
            page.commands
                .extend(original_contents[page_index].iter().cloned());
        }
    }
    if should_scale {
        for (page_index, (page, fit)) in doc.pages.iter_mut().zip(&page_fits).enumerate() {
            let page_area = fit.page_area;
            if std::env::var_os("FULLBLEED_PAGE_SHRINK_DEBUG").is_some() {
                eprintln!(
                    "fullbleed page shrink: page_width_milli={} page_area={page_area:?} max_right_milli={} local_scale={} document_scale={scale}",
                    fit.page_width_milli, fit.max_right_milli, fit.local_scale,
                );
            }

            // Page backgrounds, print marks, and margin boxes are installed by
            // the selected template before its metadata marker. Browser page
            // fitting scales only the document content after that marker.
            page.commands.truncate(fit.content_start);
            let commands = original_contents[page_index].clone();
            let mut scaled = Vec::with_capacity(commands.len() + 6);
            // Keep the fitted document surface just inside a nonzero
            // block-start page-area edge. PDF fill paths own a coincident
            // raster row differently from Chromium, and the propagated root
            // canvas would otherwise cover one row of the @page background.
            // One serialized millipoint changes only that boundary decision;
            // it is far below a device pixel at normal print resolutions.
            let block_start_guard = Pt::from_milli_i64(1);
            let clip_guard = if page_area.y > Pt::ZERO && page_area.height > block_start_guard {
                block_start_guard
            } else {
                Pt::ZERO
            };
            scaled.push(Command::SaveState);
            scaled.push(Command::ClipRect {
                x: page_area.x,
                y: page_area.y + clip_guard,
                width: page_area.width,
                height: page_area.height - clip_guard,
            });
            scaled.push(Command::CssTransformOrigin {
                x: page_area.x,
                y: page_area.y,
                inverse: false,
            });
            scaled.push(Command::Scale(scale, scale));
            scaled.push(Command::CssTransformOrigin {
                x: page_area.x,
                y: page_area.y,
                inverse: true,
            });
            scaled.extend(commands);
            scaled.push(Command::RestoreState);
            page.commands.extend(scaled);
        }
    }

    // A transformed box that crosses the block-start edge belongs visually to
    // the preceding fragmentainer. Replay only its clipped visual surface on
    // that page; tags and metadata remain owned by the source page.
    for source_index in 1..page_fits.len() {
        let source_fit = page_fits[source_index];
        if !source_fit
            .min_top_milli
            .is_some_and(|top| top < source_fit.page_area.y.to_milli_i64())
        {
            continue;
        }
        let destination_index = source_index - 1;
        let destination_fit = page_fits[destination_index];
        let destination_area = destination_fit.page_area;
        let source_area = source_fit.page_area;
        let replay = visual_replay_commands(&pristine_contents[source_index]);
        if replay.is_empty() {
            continue;
        }
        let page = &mut doc.pages[destination_index];
        page.commands.push(Command::SaveState);
        page.commands.push(Command::ClipRect {
            x: destination_area.x,
            y: destination_area.y,
            width: destination_area.width,
            height: destination_area.height,
        });
        let replay_x = destination_area.x - source_area.x * scale;
        let replay_y = destination_area.y + destination_area.height - source_area.y * scale
            + Pt::from_f32(0.25);
        page.commands.push(Command::BeginArtifact { subtype: None });
        if source_fit.page_size == destination_fit.page_size {
            // Replay the original vector program directly. Besides avoiding a
            // surface allocation, this keeps edge antialiasing on the same
            // device phase as Chromium's continuous fragment paint stream.
            page.commands.push(Command::ConcatMatrix {
                a: scale,
                b: 0.0,
                c: 0.0,
                d: scale,
                e: replay_x,
                f: replay_y + source_fit.page_size.height * scale
                    - destination_fit.page_size.height,
            });
            page.commands.extend(replay);
        } else {
            // Mixed physical page sizes need an explicit source coordinate
            // space. Keep that uncommon path vector-backed through a form.
            let resource_id = format!("html-fragment-overflow:{}", source_index + 1);
            page.commands.push(Command::DefineIsolatedForm {
                resource_id: resource_id.clone(),
                width: source_fit.page_size.width,
                height: source_fit.page_size.height,
                commands: replay,
            });
            page.commands.push(Command::DrawForm {
                x: replay_x,
                y: replay_y,
                width: source_fit.page_size.width * scale,
                height: source_fit.page_size.height * scale,
                resource_id,
            });
        }
        page.commands.push(Command::EndMarkedContent);
        page.commands.push(Command::RestoreState);
    }
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
    profile.as_str()
}

fn validate_pdf_options(options: &PdfOptions) -> Result<(), FullBleedError> {
    if !options.pdf_profile.requires_output_intent() {
        return Ok(());
    }

    let Some(intent) = options.output_intent.as_ref() else {
        return Err(FullBleedError::InvalidConfiguration(format!(
            "pdf_profile={} requires output_intent",
            options.pdf_profile.as_str()
        )));
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
                Command::DefineForm { resource_id, .. }
                | Command::DefineIsolatedForm { resource_id, .. } => {
                    defs += 1;
                    if resource_id.starts_with("svg:") {
                        svg_defs += 1;
                    }
                }
                Command::DrawForm { resource_id, .. }
                | Command::DrawFilteredForm { resource_id, .. } => {
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
    asset_bundle: Option<Arc<AssetBundle>>,
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
        asset_bundle,
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
            AddResult::Placed(_) => {}
            AddResult::Split(_remaining, _) => break,
            AddResult::Overflow(_remaining, _) => break,
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
            Command::DefineIsolatedForm {
                resource_id,
                width,
                height,
                commands,
            } => Command::DefineIsolatedForm {
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
    asset_bundle: Option<Arc<AssetBundle>>,
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
                    asset_bundle.clone(),
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
                        asset_bundle.clone(),
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
                asset_bundle.clone(),
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

fn watermark_image_bytes(bundle: Option<&AssetBundle>, source: &str) -> Option<Vec<u8>> {
    let resolved = assets::resolve_image_asset(bundle, source);
    resolved.trace.success.then_some(resolved.bytes)
}

fn watermark_image_size(
    bundle: Option<&AssetBundle>,
    source: &str,
    page_size: Size,
) -> Option<Size> {
    let bytes = watermark_image_bytes(bundle, source)?;
    let (w, h) = image_native::dimensions(&bytes).ok()?;
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
    asset_bundle: Option<Arc<AssetBundle>>,
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
                asset_bundle.clone(),
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
            let resolved_path = assets::renderable_image_source(asset_bundle.as_deref(), path)
                .unwrap_or_else(|| path.clone());
            let size =
                watermark_image_size(asset_bundle.as_deref(), path, page_size).unwrap_or(Size {
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
                resource_id: resolved_path,
                interpolate: true,
                source_clip: None,
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
    asset_bundle: Option<Arc<AssetBundle>>,
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
            asset_bundle.clone(),
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

fn escape_html_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

impl FullBleed {
    pub fn builder() -> FullBleedBuilder {
        FullBleedBuilder::new()
    }

    fn clone_for_compiled_reflow(&self) -> Self {
        Self {
            default_page_size: self.default_page_size,
            default_margins: self.default_margins,
            page_margins: self.page_margins.clone(),
            page_size_explicit: self.page_size_explicit,
            margins_explicit: self.margins_explicit,
            font_registry: self.font_registry.clone(),
            pdf_options: self.pdf_options.clone(),
            svg_form_xobjects: self.svg_form_xobjects,
            svg_raster_fallback: self.svg_raster_fallback,
            debug: self.debug.clone(),
            perf: self.perf.clone(),
            jit_mode: self.jit_mode,
            layout_strategy: self.layout_strategy,
            lazy_max_passes: self.lazy_max_passes,
            lazy_budget_ms: self.lazy_budget_ms,
            page_header: self.page_header.clone(),
            page_header_html: self.page_header_html.clone(),
            page_footer: self.page_footer.clone(),
            paginated_context: self.paginated_context.clone(),
            template_binding_spec: self.template_binding_spec.clone(),
            watermark: self.watermark.clone(),
            asset_css: self.asset_css.clone(),
            asset_bundle: self.asset_bundle.clone(),
            render_context_cache: Mutex::new(RenderContextCache::new()),
        }
    }

    #[cfg(feature = "python")]
    pub(crate) fn measure_text_width_for_trace(
        &self,
        font_name: &str,
        font_size: Pt,
        text: &str,
    ) -> Pt {
        self.font_registry
            .measure_text_width(font_name, font_size, text)
    }

    #[cfg(feature = "python")]
    pub(crate) fn resolve_registered_font_trace(
        &self,
        font_name: &str,
    ) -> Option<RegisteredFontTrace> {
        self.font_registry.resolve_trace(font_name)
    }

    #[cfg(feature = "python")]
    pub(crate) fn asset_bundle_ref(&self) -> &AssetBundle {
        self.asset_bundle.as_ref()
    }

    #[cfg(feature = "python")]
    pub(crate) fn svg_raster_fallback_enabled(&self) -> bool {
        self.svg_raster_fallback
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

    pub fn document_lang(&self) -> Option<&str> {
        self.pdf_options.document_lang.as_deref()
    }

    pub fn document_title(&self) -> Option<&str> {
        self.pdf_options.document_title.as_deref()
    }

    pub fn compose_document_html(&self, body_html: &str) -> String {
        let lang = self.document_lang().unwrap_or("en");
        let title = self.document_title().unwrap_or("fullbleed document");
        let mut out = String::with_capacity(body_html.len() + title.len() + lang.len() + 160);
        out.push_str("<!doctype html><html lang=\"");
        out.push_str(&escape_html_text(lang));
        out.push_str("\"><head><meta charset=\"utf-8\" /><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" /><title>");
        out.push_str(&escape_html_text(title));
        out.push_str("</title></head><body>");
        out.push_str(body_html);
        out.push_str("</body></html>");
        out
    }

    pub fn compose_artifact_css(&self, css: &str) -> String {
        self.merge_css(css)
    }

    pub fn emit_html_artifact(
        &self,
        html: &str,
        path: impl AsRef<std::path::Path>,
        wrap_document: bool,
    ) -> Result<String, FullBleedError> {
        let text = if wrap_document {
            self.compose_document_html(html)
        } else {
            html.to_string()
        };
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(path, &text)?;
        Ok(text)
    }

    pub fn emit_css_artifact(
        &self,
        css: &str,
        path: impl AsRef<std::path::Path>,
    ) -> Result<String, FullBleedError> {
        let text = self.compose_artifact_css(css);
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(path, &text)?;
        Ok(text)
    }

    pub fn emit_html_css_artifacts(
        &self,
        html: &str,
        css: &str,
        html_path: impl AsRef<std::path::Path>,
        css_path: impl AsRef<std::path::Path>,
        wrap_document: bool,
    ) -> Result<(String, String), FullBleedError> {
        let html_text = self.emit_html_artifact(html, html_path, wrap_document)?;
        let css_text = self.emit_css_artifact(css, css_path)?;
        Ok((html_text, css_text))
    }

    fn verify_accessibility_html_facts(&self, html: &str) -> A11yVerifierFacts {
        let document = html_dom::parse_html(html);

        let mut html_lang: Option<String> = None;
        if let Ok(mut html_nodes) = document.select("html") {
            if let Some(node) = html_nodes.next() {
                let attrs = node.attributes.borrow();
                html_lang = attrs.get("lang").map(|v| v.trim().to_string());
                if matches!(html_lang.as_deref(), Some("")) {
                    html_lang = None;
                }
            }
        }

        let mut title = String::new();
        if let Ok(mut titles) = document.select("head title, title") {
            if let Some(node) = titles.next() {
                title = node.text_contents().trim().to_string();
            }
        }

        let mut main_count = 0usize;
        if let Ok(nodes) = document.select("main") {
            main_count = nodes.count();
        }

        let mut ids_seen: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        let mut duplicate_ids: Vec<String> = Vec::new();
        let mut idrefs: Vec<(String, String)> = Vec::new();
        let mut css_link_hrefs: Vec<String> = Vec::new();
        let mut signature_semantic_count = 0usize;
        let mut empty_heading_count = 0usize;
        let mut empty_label_count = 0usize;
        let mut empty_aria_label_count = 0usize;
        let mut unlabeled_region_count = 0usize;
        let mut image_count = 0usize;
        let mut image_missing_alt_count = 0usize;
        let mut image_title_only_count = 0usize;
        let mut image_semantic_conflict_count = 0usize;
        let figure_alt_length_budget = 150usize;
        let figure_caption_redundancy_threshold = 0.8f64;
        let mut figure_informative_count = 0usize;
        let mut figure_alt_over_budget_count = 0usize;
        let mut figure_max_alt_len = 0usize;
        let mut figure_caption_redundancy_count = 0usize;
        let mut figure_max_caption_similarity = 0.0f64;
        let mut figure_missing_effective_text_count = 0usize;
        let mut dl_block_count = 0usize;
        let mut dl_fragmentation_count = 0usize;
        let mut dl_group_consistency_count = 0usize;
        let mut redundant_role_native_count = 0usize;
        let mut redundant_state_native_count = 0usize;
        let mut label_for_targets: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        let mut form_controls: Vec<(String, bool, bool, bool, String, String, bool)> = Vec::new(); // (id, in_label, aria_label_nonempty, aria_labelledby_nonempty, aria_describedby, aria_errormessage, invalid_state)
        let mut tabindex_attr_count = 0usize;
        let mut positive_tabindex_count = 0usize;
        let mut invalid_tabindex_count = 0usize;
        let mut link_count = 0usize;
        let mut unnamed_link_count = 0usize;
        let mut generic_link_text_count = 0usize;
        let mut custom_click_handler_count = 0usize;
        let mut pointer_only_click_handler_count = 0usize;
        let mut script_element_count = 0usize;
        let mut embedded_active_content_count = 0usize;
        let mut autoplay_media_count = 0usize;
        let mut blink_marquee_count = 0usize;
        let mut inline_event_handler_attr_count = 0usize;
        let mut meta_refresh_count = 0usize;
        let mut has_html = false;
        let mut has_head = false;
        let mut has_body = false;
        let mut part_lang_attr_count = 0usize;
        let mut invalid_part_lang_attr_count = 0usize;

        if let Ok(nodes) = document.select("*") {
            for node in nodes {
                let mut tag_name: Option<String> = None;
                if let NodeData::Element(el) = node.as_node().data() {
                    let tag = el.name.local.as_ref().to_ascii_lowercase();
                    tag_name = Some(tag.clone());
                    match tag.as_str() {
                        "html" => has_html = true,
                        "head" => has_head = true,
                        "body" => has_body = true,
                        "script" => script_element_count += 1,
                        "iframe" | "embed" | "object" | "frame" => {
                            embedded_active_content_count += 1
                        }
                        "audio" | "video" => {}
                        "blink" | "marquee" => blink_marquee_count += 1,
                        _ => {}
                    }
                }
                let attrs = node.attributes.borrow();
                inline_event_handler_attr_count += attrs
                    .map
                    .iter()
                    .filter(|(name, _)| {
                        let attr_name = name.local.as_ref().to_ascii_lowercase();
                        attr_name.len() > 2 && attr_name.starts_with("on")
                    })
                    .count();
                if let Some(id) = attrs.get("id") {
                    let id = id.trim();
                    if !id.is_empty() {
                        let count = ids_seen.entry(id.to_string()).or_insert(0);
                        *count += 1;
                        if *count == 2 {
                            duplicate_ids.push(id.to_string());
                        }
                    }
                }
                for attr_name in ["aria-labelledby", "aria-describedby"] {
                    if let Some(val) = attrs.get(attr_name) {
                        for tok in val.split_whitespace() {
                            let tok = tok.trim();
                            if !tok.is_empty() {
                                idrefs.push((attr_name.to_string(), tok.to_string()));
                            }
                        }
                    }
                }
                if attrs.get("data-fb-a11y-signature-status").is_some() {
                    signature_semantic_count += 1;
                }
                if let Some(aria_label) = attrs.get("aria-label") {
                    if aria_label.trim().is_empty() {
                        empty_aria_label_count += 1;
                    }
                }
                if let Some(tabindex) = attrs.get("tabindex") {
                    let tabindex = tabindex.trim();
                    if !tabindex.is_empty() {
                        tabindex_attr_count += 1;
                        match tabindex.parse::<i32>() {
                            Ok(v) if v > 0 => positive_tabindex_count += 1,
                            Ok(_) => {}
                            Err(_) => invalid_tabindex_count += 1,
                        }
                    } else {
                        tabindex_attr_count += 1;
                        invalid_tabindex_count += 1;
                    }
                }
                if !matches!(tag_name.as_deref(), Some("html")) {
                    if let Some(lang_attr) = attrs.get("lang") {
                        let lang_attr = lang_attr.trim();
                        if !lang_attr.is_empty() {
                            part_lang_attr_count += 1;
                            if !Self::a11y_lang_value_is_valid(lang_attr) {
                                invalid_part_lang_attr_count += 1;
                            }
                        } else {
                            part_lang_attr_count += 1;
                            invalid_part_lang_attr_count += 1;
                        }
                    }
                }
                if matches!(tag_name.as_deref(), Some("audio" | "video"))
                    && attrs.get("autoplay").is_some()
                {
                    autoplay_media_count += 1;
                }
                if tag_name.as_deref() == Some("meta") {
                    let http_equiv_refresh = attrs
                        .get("http-equiv")
                        .map(|v| v.trim().eq_ignore_ascii_case("refresh"))
                        .unwrap_or(false);
                    if http_equiv_refresh {
                        meta_refresh_count += 1;
                    }
                }
                if tag_name.as_deref() == Some("label") {
                    // no-op: text check occurs after releasing attrs borrow
                }
                let role = attrs
                    .get("role")
                    .map(|v| v.trim().to_ascii_lowercase())
                    .unwrap_or_default();
                let role_tokens: std::collections::BTreeSet<&str> =
                    role.split_whitespace().collect();
                let expected_native_role = match tag_name.as_deref() {
                    Some("nav") => Some("navigation"),
                    Some("main") => Some("main"),
                    Some("aside") => Some("complementary"),
                    Some("form") => Some("form"),
                    Some("table") => Some("table"),
                    Some("ul" | "ol") => Some("list"),
                    Some("li") => Some("listitem"),
                    Some("button") => Some("button"),
                    Some("img") => Some("img"),
                    Some("a") => {
                        if attrs
                            .get("href")
                            .map(|v| !v.trim().is_empty())
                            .unwrap_or(false)
                        {
                            Some("link")
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(native_role) = expected_native_role {
                    if role_tokens.contains(native_role) {
                        redundant_role_native_count += 1;
                    }
                }
                if attrs.get("disabled").is_some()
                    && attrs
                        .get("aria-disabled")
                        .map(|v| {
                            matches!(
                                v.trim().to_ascii_lowercase().as_str(),
                                "1" | "true" | "yes" | "on"
                            )
                        })
                        .unwrap_or(false)
                {
                    redundant_state_native_count += 1;
                }
                if attrs.get("required").is_some()
                    && attrs
                        .get("aria-required")
                        .map(|v| {
                            matches!(
                                v.trim().to_ascii_lowercase().as_str(),
                                "1" | "true" | "yes" | "on"
                            )
                        })
                        .unwrap_or(false)
                {
                    redundant_state_native_count += 1;
                }
                if attrs.get("readonly").is_some()
                    && attrs
                        .get("aria-readonly")
                        .map(|v| {
                            matches!(
                                v.trim().to_ascii_lowercase().as_str(),
                                "1" | "true" | "yes" | "on"
                            )
                        })
                        .unwrap_or(false)
                {
                    redundant_state_native_count += 1;
                }
                if tag_name.as_deref() == Some("input") {
                    let is_checkbox_or_radio = attrs
                        .get("type")
                        .map(|v| {
                            matches!(v.trim().to_ascii_lowercase().as_str(), "checkbox" | "radio")
                        })
                        .unwrap_or(false);
                    if is_checkbox_or_radio
                        && attrs.get("checked").is_some()
                        && attrs
                            .get("aria-checked")
                            .map(|v| {
                                matches!(
                                    v.trim().to_ascii_lowercase().as_str(),
                                    "1" | "true" | "yes" | "on"
                                )
                            })
                            .unwrap_or(false)
                    {
                        redundant_state_native_count += 1;
                    }
                }
                if tag_name.as_deref() == Some("option")
                    && attrs.get("selected").is_some()
                    && attrs
                        .get("aria-selected")
                        .map(|v| {
                            matches!(
                                v.trim().to_ascii_lowercase().as_str(),
                                "1" | "true" | "yes" | "on"
                            )
                        })
                        .unwrap_or(false)
                {
                    redundant_state_native_count += 1;
                }
                let has_onclick = attrs.get("onclick").is_some();
                let has_keyboard_handler = attrs.get("onkeydown").is_some()
                    || attrs.get("onkeyup").is_some()
                    || attrs.get("onkeypress").is_some();
                let has_tabindex_attr = attrs.get("tabindex").is_some();
                let is_native_keyboard_interactive = match tag_name.as_deref() {
                    Some("a") => attrs
                        .get("href")
                        .map(|v| !v.trim().is_empty())
                        .unwrap_or(false),
                    Some("button" | "select" | "textarea" | "summary") => true,
                    Some("input") => !attrs
                        .get("type")
                        .map(|v| v.trim().eq_ignore_ascii_case("hidden"))
                        .unwrap_or(false),
                    _ => false,
                };
                if has_onclick && !is_native_keyboard_interactive {
                    custom_click_handler_count += 1;
                    if !has_keyboard_handler && !has_tabindex_attr {
                        pointer_only_click_handler_count += 1;
                    }
                }
                if role == "region" {
                    let aria_label_nonempty = attrs
                        .get("aria-label")
                        .map(|v| !v.trim().is_empty())
                        .unwrap_or(false);
                    let aria_labelledby_nonempty = attrs
                        .get("aria-labelledby")
                        .map(|v| v.split_whitespace().any(|tok| !tok.trim().is_empty()))
                        .unwrap_or(false);
                    if !aria_label_nonempty && !aria_labelledby_nonempty {
                        unlabeled_region_count += 1;
                    }
                }
                if matches!(tag_name.as_deref(), Some("img" | "svg")) {
                    image_count += 1;
                    let role_decorative = attrs
                        .get("role")
                        .map(|v| {
                            let v = v.trim().to_ascii_lowercase();
                            v == "presentation" || v == "none"
                        })
                        .unwrap_or(false);
                    let aria_hidden = attrs
                        .get("aria-hidden")
                        .map(|v| {
                            matches!(
                                v.trim().to_ascii_lowercase().as_str(),
                                "1" | "true" | "yes" | "on"
                            )
                        })
                        .unwrap_or(false);
                    let explicit_decorative = attrs
                        .get("data-fb-a11y-decorative")
                        .map(|v| {
                            matches!(
                                v.trim().to_ascii_lowercase().as_str(),
                                "1" | "true" | "yes" | "on"
                            )
                        })
                        .unwrap_or(false);
                    let aria_label = attrs.get("aria-label");
                    let aria_labelledby = attrs.get("aria-labelledby");
                    let alt_value = attrs.get("alt");
                    let title_value = attrs.get("title");
                    let has_informative_name =
                        aria_label.map(|v| !v.trim().is_empty()).unwrap_or(false)
                            || aria_labelledby
                                .map(|v| v.split_whitespace().any(|tok| !tok.trim().is_empty()))
                                .unwrap_or(false)
                            || alt_value.map(|v| !v.trim().is_empty()).unwrap_or(false);
                    let alt_empty = alt_value.map(|v| v.is_empty()).unwrap_or(false);
                    let decorative =
                        explicit_decorative || aria_hidden || role_decorative || alt_empty;
                    if decorative && has_informative_name {
                        image_semantic_conflict_count += 1;
                    } else if !decorative && !has_informative_name {
                        if title_value.map(|v| !v.trim().is_empty()).unwrap_or(false) {
                            image_title_only_count += 1;
                        } else {
                            image_missing_alt_count += 1;
                        }
                    }
                }
                if tag_name.as_deref() == Some("label") {
                    if let Some(for_id) = attrs.get("for") {
                        let for_id = for_id.trim();
                        if !for_id.is_empty() {
                            label_for_targets.insert(for_id.to_string());
                        }
                    }
                }
                if matches!(tag_name.as_deref(), Some("input" | "select" | "textarea")) {
                    let is_hidden_input = tag_name.as_deref() == Some("input")
                        && attrs
                            .get("type")
                            .map(|v| v.trim().eq_ignore_ascii_case("hidden"))
                            .unwrap_or(false);
                    if !is_hidden_input {
                        let ctl_id = attrs.get("id").map(|v| v.trim()).unwrap_or("").to_string();
                        let aria_label_nonempty = attrs
                            .get("aria-label")
                            .map(|v| !v.trim().is_empty())
                            .unwrap_or(false);
                        let aria_labelledby_nonempty = attrs
                            .get("aria-labelledby")
                            .map(|v| v.split_whitespace().any(|tok| !tok.trim().is_empty()))
                            .unwrap_or(false);
                        let aria_describedby = attrs
                            .get("aria-describedby")
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let aria_errormessage = attrs
                            .get("aria-errormessage")
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let invalid_state = attrs
                            .get("aria-invalid")
                            .map(|v| {
                                let t = v.trim().to_ascii_lowercase();
                                !t.is_empty() && !matches!(t.as_str(), "0" | "false" | "no" | "off")
                            })
                            .unwrap_or(false);
                        let in_label = node.as_node().ancestors().any(|anc| {
                            if let NodeData::Element(el) = anc.data() {
                                el.name.local.as_ref().eq_ignore_ascii_case("label")
                            } else {
                                false
                            }
                        });
                        form_controls.push((
                            ctl_id,
                            in_label,
                            aria_label_nonempty,
                            aria_labelledby_nonempty,
                            aria_describedby,
                            aria_errormessage,
                            invalid_state,
                        ));
                    }
                }
                if tag_name.as_deref() == Some("a") {
                    let has_href = attrs
                        .get("href")
                        .map(|v| !v.trim().is_empty())
                        .unwrap_or(false);
                    if has_href {
                        link_count += 1;
                        let aria_label_nonempty = attrs
                            .get("aria-label")
                            .map(|v| !v.trim().is_empty())
                            .unwrap_or(false);
                        let aria_labelledby_nonempty = attrs
                            .get("aria-labelledby")
                            .map(|v| v.split_whitespace().any(|tok| !tok.trim().is_empty()))
                            .unwrap_or(false);
                        let link_text = node.as_node().text_contents();
                        let link_text_norm = link_text
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .to_ascii_lowercase();
                        let text_nonempty = !link_text_norm.trim().is_empty();
                        if !(aria_label_nonempty || aria_labelledby_nonempty || text_nonempty) {
                            unnamed_link_count += 1;
                        } else if text_nonempty
                            && matches!(
                                link_text_norm.as_str(),
                                "click here"
                                    | "here"
                                    | "read more"
                                    | "learn more"
                                    | "more"
                                    | "more..."
                            )
                        {
                            generic_link_text_count += 1;
                        }
                    }
                }
                if let Some(rel) = attrs.get("rel") {
                    let rel_l = rel.to_ascii_lowercase();
                    if rel_l.split_whitespace().any(|tok| tok == "stylesheet") {
                        if let Some(href) = attrs.get("href") {
                            let href = href.trim();
                            if !href.is_empty() {
                                css_link_hrefs.push(href.to_string());
                            }
                        }
                    }
                }
                drop(attrs);
                if let Some(tag) = tag_name.as_deref() {
                    let is_empty_text = node.as_node().text_contents().trim().is_empty();
                    if matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") && is_empty_text {
                        empty_heading_count += 1;
                    }
                    if tag == "label" && is_empty_text {
                        empty_label_count += 1;
                    }
                }
            }
        }

        let id_set: std::collections::BTreeSet<String> = ids_seen.keys().cloned().collect();
        let mut missing_idrefs: Vec<(String, String)> = Vec::new();
        for (attr_name, tok) in idrefs {
            if !id_set.contains(&tok) {
                missing_idrefs.push((attr_name, tok));
            }
        }
        let mut unlabeled_form_control_count = 0usize;
        for (ctl_id, in_label, aria_label_nonempty, aria_labelledby_nonempty, _, _, _) in
            &form_controls
        {
            if *aria_label_nonempty || *aria_labelledby_nonempty || *in_label {
                continue;
            }
            if !ctl_id.is_empty() && label_for_targets.contains(ctl_id) {
                continue;
            }
            unlabeled_form_control_count += 1;
        }
        let mut invalid_form_control_count = 0usize;
        let mut unidentified_error_form_control_count = 0usize;
        for (_, _, _, _, aria_describedby, aria_errormessage, invalid_state) in &form_controls {
            if !*invalid_state {
                continue;
            }
            invalid_form_control_count += 1;
            let describedby_ok = aria_describedby
                .split_whitespace()
                .any(|tok| !tok.trim().is_empty() && id_set.contains(tok));
            let errormessage_ok = aria_errormessage
                .split_whitespace()
                .any(|tok| !tok.trim().is_empty() && id_set.contains(tok));
            if !(describedby_ok || errormessage_ok) {
                unidentified_error_form_control_count += 1;
            }
        }

        let body_text = if let Ok(mut bodies) = document.select("body") {
            if let Some(body) = bodies.next() {
                body.as_node()
                    .text_contents()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let mut tables: Vec<HtmlTableFacts> = Vec::new();
        if let Ok(table_nodes) = document.select("table") {
            for table in table_nodes {
                let node = table.as_node();
                let has_caption = node
                    .select("caption")
                    .ok()
                    .map(|mut it| it.next().is_some())
                    .unwrap_or(false);
                let th_count = node.select("th").ok().map(|it| it.count()).unwrap_or(0);
                let th_scope_count = node
                    .select("th[scope]")
                    .ok()
                    .map(|it| it.count())
                    .unwrap_or(0);
                tables.push(HtmlTableFacts {
                    has_caption,
                    th_count,
                    th_scope_count,
                });
            }
        }

        if let Ok(figure_nodes) = document.select("figure") {
            for figure in figure_nodes {
                let figure_node = figure.as_node();
                let caption_text = if let Ok(captions) = figure_node.select("figcaption") {
                    let caption_joined = captions
                        .map(|cap| cap.as_node().text_contents())
                        .collect::<Vec<_>>()
                        .join(" ");
                    Self::a11y_normalize_text(&caption_joined)
                } else {
                    String::new()
                };

                let mut informative_image_count = 0usize;
                let mut effective_name_present = false;
                let mut alt_texts: Vec<String> = Vec::new();
                let mut local_max_alt_len = 0usize;

                if let Ok(images) = figure_node.select("img, svg") {
                    for image in images {
                        let attrs = image.attributes.borrow();
                        let role_decorative = attrs
                            .get("role")
                            .map(|v| {
                                let v = v.trim().to_ascii_lowercase();
                                v == "presentation" || v == "none"
                            })
                            .unwrap_or(false);
                        let aria_hidden = attrs
                            .get("aria-hidden")
                            .map(|v| {
                                matches!(
                                    v.trim().to_ascii_lowercase().as_str(),
                                    "1" | "true" | "yes" | "on"
                                )
                            })
                            .unwrap_or(false);
                        let explicit_decorative = attrs
                            .get("data-fb-a11y-decorative")
                            .map(|v| {
                                matches!(
                                    v.trim().to_ascii_lowercase().as_str(),
                                    "1" | "true" | "yes" | "on"
                                )
                            })
                            .unwrap_or(false);
                        let aria_label = attrs.get("aria-label");
                        let aria_labelledby = attrs.get("aria-labelledby");
                        let alt_value = attrs.get("alt");
                        let has_informative_name =
                            aria_label.map(|v| !v.trim().is_empty()).unwrap_or(false)
                                || aria_labelledby
                                    .map(|v| v.split_whitespace().any(|tok| !tok.trim().is_empty()))
                                    .unwrap_or(false)
                                || alt_value.map(|v| !v.trim().is_empty()).unwrap_or(false);
                        let alt_empty = alt_value.map(|v| v.is_empty()).unwrap_or(false);
                        let decorative =
                            explicit_decorative || aria_hidden || role_decorative || alt_empty;
                        if decorative {
                            continue;
                        }
                        informative_image_count += 1;
                        if has_informative_name {
                            effective_name_present = true;
                        }
                        if let Some(alt) = alt_value {
                            let alt_norm = Self::a11y_normalize_text(alt);
                            if !alt_norm.is_empty() {
                                local_max_alt_len = local_max_alt_len.max(alt_norm.chars().count());
                                alt_texts.push(alt_norm);
                            }
                        }
                    }
                }

                if informative_image_count == 0 {
                    continue;
                }

                figure_informative_count += 1;
                figure_max_alt_len = figure_max_alt_len.max(local_max_alt_len);
                if local_max_alt_len > figure_alt_length_budget {
                    figure_alt_over_budget_count += 1;
                }
                if !effective_name_present && caption_text.is_empty() {
                    figure_missing_effective_text_count += 1;
                }
                if !alt_texts.is_empty() && !caption_text.is_empty() {
                    let longest_alt = alt_texts
                        .iter()
                        .max_by_key(|txt| txt.chars().count())
                        .cloned()
                        .unwrap_or_default();
                    let similarity = Self::a11y_text_similarity(&longest_alt, &caption_text);
                    figure_max_caption_similarity = figure_max_caption_similarity.max(similarity);
                    if similarity >= figure_caption_redundancy_threshold {
                        figure_caption_redundancy_count += 1;
                    }
                }
            }
        }

        if let Ok(parent_nodes) = document.select("*") {
            for parent in parent_nodes {
                let mut run_total = 0usize;
                let mut run_tiny = 0usize;
                for child in parent.as_node().children() {
                    let is_dl = if let NodeData::Element(el) = child.data() {
                        el.name.local.as_ref().eq_ignore_ascii_case("dl")
                    } else {
                        false
                    };
                    if is_dl {
                        dl_block_count += 1;
                        run_total += 1;
                        let dt_count = child.select("dt").ok().map(|it| it.count()).unwrap_or(0);
                        let dd_count = child.select("dd").ok().map(|it| it.count()).unwrap_or(0);
                        if dt_count.min(dd_count) <= 2 {
                            run_tiny += 1;
                        }
                    } else if run_total >= 2 {
                        dl_fragmentation_count += run_total - 1;
                        if run_tiny == run_total {
                            dl_group_consistency_count += run_total - 1;
                        }
                        run_total = 0;
                        run_tiny = 0;
                    } else {
                        run_total = 0;
                        run_tiny = 0;
                    }
                }
                if run_total >= 2 {
                    dl_fragmentation_count += run_total - 1;
                    if run_tiny == run_total {
                        dl_group_consistency_count += run_total - 1;
                    }
                }
            }
        }

        A11yVerifierFacts {
            html_lang,
            title,
            part_lang_attr_count,
            invalid_part_lang_attr_count,
            main_count,
            duplicate_ids,
            missing_idrefs,
            has_html_wrapper: has_html && has_head && has_body,
            has_css_link: !css_link_hrefs.is_empty(),
            css_link_hrefs,
            signature_semantic_count,
            empty_heading_count,
            empty_label_count,
            empty_aria_label_count,
            unlabeled_region_count,
            image_count,
            image_missing_alt_count,
            image_title_only_count,
            image_semantic_conflict_count,
            figure_informative_count,
            figure_alt_length_budget,
            figure_alt_over_budget_count,
            figure_max_alt_len,
            figure_caption_redundancy_threshold,
            figure_caption_redundancy_count,
            figure_max_caption_similarity,
            figure_missing_effective_text_count,
            dl_block_count,
            dl_fragmentation_count,
            dl_group_consistency_count,
            redundant_role_native_count,
            redundant_state_native_count,
            form_control_count: form_controls.len(),
            unlabeled_form_control_count,
            invalid_form_control_count,
            unidentified_error_form_control_count,
            tabindex_attr_count,
            positive_tabindex_count,
            invalid_tabindex_count,
            link_count,
            unnamed_link_count,
            generic_link_text_count,
            custom_click_handler_count,
            pointer_only_click_handler_count,
            script_element_count,
            embedded_active_content_count,
            autoplay_media_count,
            blink_marquee_count,
            inline_event_handler_attr_count,
            meta_refresh_count,
            tables,
            body_text,
        }
    }

    fn push_a11y_finding(
        findings: &mut Vec<A11yVerifierFinding>,
        rule_id: &str,
        verdict: &str,
        severity: &str,
        stage: &str,
        source: &str,
        message: String,
        evidence: Vec<A11yVerifierEvidence>,
    ) {
        findings.push(A11yVerifierFinding {
            rule_id: rule_id.to_string(),
            applicability: "applicable".to_string(),
            verification_mode: "machine".to_string(),
            verdict: verdict.to_string(),
            severity: severity.to_string(),
            confidence: "certain".to_string(),
            stage: stage.to_string(),
            source: source.to_string(),
            message,
            evidence,
        });
    }

    pub fn verify_accessibility_html_core(
        &self,
        html: &str,
        profile: &str,
    ) -> A11yVerifierCoreReport {
        let facts = self.verify_accessibility_html_facts(html);
        let mut findings: Vec<A11yVerifierFinding> = Vec::new();

        let observed_lang = facts.html_lang.clone().unwrap_or_default();
        let expected_lang = self.document_lang().map(|value| value.to_string());
        let lang_value_valid = facts
            .html_lang
            .as_deref()
            .map(Self::a11y_lang_value_is_valid)
            .unwrap_or(false);
        let lang_ok = lang_value_valid
            && expected_lang
                .as_deref()
                .map(|expected| facts.html_lang.as_deref() == Some(expected))
                .unwrap_or(true);
        let lang_failure_kind = if facts.html_lang.is_none() {
            "missing"
        } else if !lang_value_valid {
            "invalid"
        } else if expected_lang
            .as_deref()
            .map(|expected| facts.html_lang.as_deref() != Some(expected))
            .unwrap_or(false)
        {
            "metadata_mismatch"
        } else {
            "none"
        };
        Self::push_a11y_finding(
            &mut findings,
            "fb.a11y.html.lang_present_valid",
            if lang_ok { "pass" } else { "fail" },
            "high",
            "post-emit",
            "fullbleed",
            if lang_ok {
                "HTML lang attribute is present and valid.".to_string()
            } else if lang_failure_kind == "metadata_mismatch" {
                format!(
                    "HTML lang attribute is present and valid in the emitted DOM, but engine metadata persistence mismatched (observed DOM={}, expected metadata={}).",
                    observed_lang,
                    expected_lang.clone().unwrap_or_default()
                )
            } else if lang_failure_kind == "invalid" {
                "HTML lang attribute is present but invalid.".to_string()
            } else {
                "HTML lang attribute is missing.".to_string()
            },
            vec![A11yVerifierEvidence {
                selector: Some("html".to_string()),
                values: vec![
                    ("lang".to_string(), observed_lang),
                    (
                        "observed_lang".to_string(),
                        facts.html_lang.clone().unwrap_or_default(),
                    ),
                    (
                        "expected_document_lang".to_string(),
                        expected_lang.unwrap_or_default(),
                    ),
                    ("failure_kind".to_string(), lang_failure_kind.to_string()),
                ],
            }],
        );

        if facts.part_lang_attr_count == 0 {
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.language.parts_declared_valid_seed".to_string(),
                applicability: "not_applicable".to_string(),
                verification_mode: "hybrid".to_string(),
                verdict: "not_applicable".to_string(),
                severity: "low".to_string(),
                confidence: "high".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message: "No inline lang declarations on descendant elements detected; language-of-parts rule not applicable for this document."
                    .to_string(),
                evidence: vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![
                        (
                            "part_lang_attr_count".to_string(),
                            facts.part_lang_attr_count.to_string(),
                        ),
                        (
                            "invalid_part_lang_attr_count".to_string(),
                            facts.invalid_part_lang_attr_count.to_string(),
                        ),
                    ],
                }],
            });
        } else {
            let part_lang_fail = facts.invalid_part_lang_attr_count > 0;
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.language.parts_declared_valid_seed".to_string(),
                applicability: "applicable".to_string(),
                verification_mode: "hybrid".to_string(),
                verdict: if part_lang_fail { "fail" } else { "pass" }.to_string(),
                severity: if part_lang_fail { "medium" } else { "low" }.to_string(),
                confidence: "high".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message: if part_lang_fail {
                    "Invalid or empty inline language-of-parts declarations detected.".to_string()
                } else {
                    "Inline language-of-parts declarations are syntactically valid.".to_string()
                },
                evidence: vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![
                        (
                            "part_lang_attr_count".to_string(),
                            facts.part_lang_attr_count.to_string(),
                        ),
                        (
                            "invalid_part_lang_attr_count".to_string(),
                            facts.invalid_part_lang_attr_count.to_string(),
                        ),
                    ],
                }],
            });
        }

        let expected_title = self.document_title().map(|value| value.to_string());
        let title_present = !facts.title.trim().is_empty();
        let title_ok = title_present
            && expected_title
                .as_deref()
                .map(|expected| facts.title == expected)
                .unwrap_or(true);
        let title_failure_kind = if !title_present {
            "missing"
        } else if expected_title
            .as_deref()
            .map(|expected| facts.title != expected)
            .unwrap_or(false)
        {
            "metadata_mismatch"
        } else {
            "none"
        };
        Self::push_a11y_finding(
            &mut findings,
            "fb.a11y.html.title_present_nonempty",
            if title_ok { "pass" } else { "fail" },
            "high",
            "post-emit",
            "fullbleed",
            if title_ok {
                "Document title is present and non-empty.".to_string()
            } else if title_failure_kind == "metadata_mismatch" {
                format!(
                    "Document title is present in the emitted DOM, but engine metadata persistence mismatched (observed DOM={}, expected metadata={}).",
                    facts.title,
                    expected_title.clone().unwrap_or_default()
                )
            } else {
                "Document title is missing or empty.".to_string()
            },
            vec![A11yVerifierEvidence {
                selector: Some("head > title".to_string()),
                values: vec![
                    ("title".to_string(), facts.title.clone()),
                    ("observed_title".to_string(), facts.title.clone()),
                    (
                        "expected_document_title".to_string(),
                        expected_title.unwrap_or_default(),
                    ),
                    ("failure_kind".to_string(), title_failure_kind.to_string()),
                ],
            }],
        );

        let main_ok = facts.main_count == 1;
        Self::push_a11y_finding(
            &mut findings,
            "fb.a11y.structure.single_main",
            if main_ok { "pass" } else { "fail" },
            "medium",
            "post-emit",
            "fullbleed",
            if main_ok {
                "Single primary content root detected.".to_string()
            } else {
                format!("Expected exactly one <main>; found {}.", facts.main_count)
            },
            vec![A11yVerifierEvidence {
                selector: Some("main".to_string()),
                values: vec![("count".to_string(), facts.main_count.to_string())],
            }],
        );

        let hl_fail =
            (facts.empty_heading_count + facts.empty_label_count + facts.empty_aria_label_count)
                > 0;
        let hl_warn = facts.unlabeled_region_count > 0;
        findings.push(A11yVerifierFinding {
            rule_id: "fb.a11y.headings_labels.present_nonempty".to_string(),
            applicability: "applicable".to_string(),
            verification_mode: "hybrid".to_string(),
            verdict: if hl_fail {
                "fail"
            } else if hl_warn {
                "warn"
            } else {
                "pass"
            }
            .to_string(),
            severity: if hl_fail { "high" } else { "medium" }.to_string(),
            confidence: if hl_fail || hl_warn { "high" } else { "medium" }.to_string(),
            stage: "post-emit".to_string(),
            source: "fullbleed".to_string(),
            message: if hl_fail {
                "Empty heading/label naming signals detected.".to_string()
            } else if hl_warn {
                "Headings/labels are non-empty, but some region landmarks are unlabeled."
                    .to_string()
            } else {
                "No empty headings/labels or unlabeled regions detected.".to_string()
            },
            evidence: vec![A11yVerifierEvidence {
                selector: None,
                values: vec![
                    (
                        "empty_heading_count".to_string(),
                        facts.empty_heading_count.to_string(),
                    ),
                    (
                        "empty_label_count".to_string(),
                        facts.empty_label_count.to_string(),
                    ),
                    (
                        "empty_aria_label_count".to_string(),
                        facts.empty_aria_label_count.to_string(),
                    ),
                    (
                        "unlabeled_region_count".to_string(),
                        facts.unlabeled_region_count.to_string(),
                    ),
                ],
            }],
        });

        if facts.image_count == 0 {
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.images.alt_or_decorative".to_string(),
                applicability: "not_applicable".to_string(),
                verification_mode: "machine".to_string(),
                verdict: "not_applicable".to_string(),
                severity: "low".to_string(),
                confidence: "high".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message:
                    "No img/svg elements detected; non-text-content image rule not applicable."
                        .to_string(),
                evidence: Vec::new(),
            });
        } else {
            let img_fail =
                facts.image_missing_alt_count > 0 || facts.image_semantic_conflict_count > 0;
            let img_warn = facts.image_title_only_count > 0;
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.images.alt_or_decorative".to_string(),
                applicability: "applicable".to_string(),
                verification_mode: "machine".to_string(),
                verdict: if img_fail {
                    "fail"
                } else if img_warn {
                    "warn"
                } else {
                    "pass"
                }
                .to_string(),
                severity: if img_fail { "high" } else { "medium" }.to_string(),
                confidence: "high".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message: if img_fail {
                    "Image text alternative errors detected.".to_string()
                } else if img_warn {
                    "Some images rely on title without alt/ARIA text alternatives.".to_string()
                } else {
                    "Image text alternatives/decorative semantics look consistent.".to_string()
                },
                evidence: vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![
                        ("image_count".to_string(), facts.image_count.to_string()),
                        (
                            "image_missing_alt_count".to_string(),
                            facts.image_missing_alt_count.to_string(),
                        ),
                        (
                            "image_title_only_count".to_string(),
                            facts.image_title_only_count.to_string(),
                        ),
                        (
                            "image_semantic_conflict_count".to_string(),
                            facts.image_semantic_conflict_count.to_string(),
                        ),
                    ],
                }],
            });
        }

        if facts.figure_informative_count == 0 {
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.figure.alt_length_budget_seed".to_string(),
                applicability: "not_applicable".to_string(),
                verification_mode: "machine".to_string(),
                verdict: "not_applicable".to_string(),
                severity: "low".to_string(),
                confidence: "high".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message:
                    "No informative figures detected; figure alt-length budget rule not applicable."
                        .to_string(),
                evidence: vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![
                        (
                            "figure_informative_count".to_string(),
                            facts.figure_informative_count.to_string(),
                        ),
                        (
                            "figure_alt_length_budget".to_string(),
                            facts.figure_alt_length_budget.to_string(),
                        ),
                        (
                            "figure_alt_over_budget_count".to_string(),
                            facts.figure_alt_over_budget_count.to_string(),
                        ),
                        (
                            "figure_max_alt_len".to_string(),
                            facts.figure_max_alt_len.to_string(),
                        ),
                    ],
                }],
            });
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.figure.caption_redundancy_seed".to_string(),
                applicability: "not_applicable".to_string(),
                verification_mode: "machine".to_string(),
                verdict: "not_applicable".to_string(),
                severity: "low".to_string(),
                confidence: "high".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message: "No informative figures detected; figure caption-redundancy rule not applicable."
                    .to_string(),
                evidence: vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![
                        (
                            "figure_informative_count".to_string(),
                            facts.figure_informative_count.to_string(),
                        ),
                        (
                            "figure_caption_redundancy_threshold".to_string(),
                            facts.figure_caption_redundancy_threshold.to_string(),
                        ),
                        (
                            "figure_caption_redundancy_count".to_string(),
                            facts.figure_caption_redundancy_count.to_string(),
                        ),
                        (
                            "figure_max_caption_similarity".to_string(),
                            format!("{:.3}", facts.figure_max_caption_similarity),
                        ),
                    ],
                }],
            });
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.figure.missing_effective_text_seed".to_string(),
                applicability: "not_applicable".to_string(),
                verification_mode: "machine".to_string(),
                verdict: "not_applicable".to_string(),
                severity: "low".to_string(),
                confidence: "high".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message: "No informative figures detected; missing-effective-figure-text rule not applicable."
                    .to_string(),
                evidence: vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![
                        (
                            "figure_informative_count".to_string(),
                            facts.figure_informative_count.to_string(),
                        ),
                        (
                            "figure_missing_effective_text_count".to_string(),
                            facts.figure_missing_effective_text_count.to_string(),
                        ),
                    ],
                }],
            });
        } else {
            let over_budget = facts.figure_alt_over_budget_count > 0;
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.figure.alt_length_budget_seed".to_string(),
                applicability: "applicable".to_string(),
                verification_mode: "machine".to_string(),
                verdict: if over_budget { "warn" } else { "pass" }.to_string(),
                severity: "medium".to_string(),
                confidence: "high".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message: if over_budget {
                    "Figure alternative text exceeds recommended length budget.".to_string()
                } else {
                    "Informative figure alternative text lengths are within the recommended budget."
                        .to_string()
                },
                evidence: vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![
                        (
                            "figure_informative_count".to_string(),
                            facts.figure_informative_count.to_string(),
                        ),
                        (
                            "figure_alt_length_budget".to_string(),
                            facts.figure_alt_length_budget.to_string(),
                        ),
                        (
                            "figure_alt_over_budget_count".to_string(),
                            facts.figure_alt_over_budget_count.to_string(),
                        ),
                        (
                            "figure_max_alt_len".to_string(),
                            facts.figure_max_alt_len.to_string(),
                        ),
                    ],
                }],
            });
            let caption_redundant = facts.figure_caption_redundancy_count > 0;
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.figure.caption_redundancy_seed".to_string(),
                applicability: "applicable".to_string(),
                verification_mode: "machine".to_string(),
                verdict: if caption_redundant { "warn" } else { "pass" }.to_string(),
                severity: "medium".to_string(),
                confidence: "high".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message: if caption_redundant {
                    "Figure alt and figcaption content appear near-duplicate; announce-once optimization recommended."
                        .to_string()
                } else {
                    "Figure alt and figcaption content are sufficiently distinct.".to_string()
                },
                evidence: vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![
                        (
                            "figure_informative_count".to_string(),
                            facts.figure_informative_count.to_string(),
                        ),
                        (
                            "figure_caption_redundancy_threshold".to_string(),
                            facts.figure_caption_redundancy_threshold.to_string(),
                        ),
                        (
                            "figure_caption_redundancy_count".to_string(),
                            facts.figure_caption_redundancy_count.to_string(),
                        ),
                        (
                            "figure_max_caption_similarity".to_string(),
                            format!("{:.3}", facts.figure_max_caption_similarity),
                        ),
                    ],
                }],
            });
            let missing_effective = facts.figure_missing_effective_text_count > 0;
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.figure.missing_effective_text_seed".to_string(),
                applicability: "applicable".to_string(),
                verification_mode: "machine".to_string(),
                verdict: if missing_effective { "fail" } else { "pass" }.to_string(),
                severity: "high".to_string(),
                confidence: "high".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message: if missing_effective {
                    "Informative figure(s) missing effective text alternatives (alt/ARIA/caption)."
                        .to_string()
                } else {
                    "Informative figures expose effective text alternatives (alt/ARIA/caption)."
                        .to_string()
                },
                evidence: vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![
                        (
                            "figure_informative_count".to_string(),
                            facts.figure_informative_count.to_string(),
                        ),
                        (
                            "figure_missing_effective_text_count".to_string(),
                            facts.figure_missing_effective_text_count.to_string(),
                        ),
                    ],
                }],
            });
        }

        let dl_fragmented = facts.dl_fragmentation_count > 0;
        findings.push(A11yVerifierFinding {
            rule_id: "fb.a11y.dl.fragmentation_seed".to_string(),
            applicability: "applicable".to_string(),
            verification_mode: "machine".to_string(),
            verdict: if dl_fragmented { "warn" } else { "pass" }.to_string(),
            severity: "medium".to_string(),
            confidence: "high".to_string(),
            stage: "post-emit".to_string(),
            source: "fullbleed".to_string(),
            message: if dl_fragmented {
                "Adjacent description-list siblings detected; consolidate into a single logical list where possible."
                    .to_string()
            } else {
                "No adjacent description-list fragmentation detected.".to_string()
            },
            evidence: vec![A11yVerifierEvidence {
                selector: None,
                values: vec![
                    ("dl_block_count".to_string(), facts.dl_block_count.to_string()),
                    (
                        "dl_fragmentation_count".to_string(),
                        facts.dl_fragmentation_count.to_string(),
                    ),
                ],
            }],
        });
        let dl_inconsistent = facts.dl_group_consistency_count > 0;
        findings.push(A11yVerifierFinding {
            rule_id: "fb.a11y.dl.group_consistency_seed".to_string(),
            applicability: "applicable".to_string(),
            verification_mode: "machine".to_string(),
            verdict: if dl_inconsistent { "warn" } else { "pass" }.to_string(),
            severity: "medium".to_string(),
            confidence: "high".to_string(),
            stage: "post-emit".to_string(),
            source: "fullbleed".to_string(),
            message: if dl_inconsistent {
                "Repeated tiny description-list groups detected with similar structure; unify group semantics."
                    .to_string()
            } else {
                "Description-list grouping consistency looks stable.".to_string()
            },
            evidence: vec![A11yVerifierEvidence {
                selector: None,
                values: vec![
                    ("dl_block_count".to_string(), facts.dl_block_count.to_string()),
                    (
                        "dl_group_consistency_count".to_string(),
                        facts.dl_group_consistency_count.to_string(),
                    ),
                ],
            }],
        });

        if facts.form_control_count == 0 {
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.forms.labels_or_instructions_present".to_string(),
                applicability: "not_applicable".to_string(),
                verification_mode: "hybrid".to_string(),
                verdict: "not_applicable".to_string(),
                severity: "low".to_string(),
                confidence: "high".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message: "No form controls detected; labels/instructions rule not applicable."
                    .to_string(),
                evidence: Vec::new(),
            });
        } else {
            let ctrl_fail = facts.unlabeled_form_control_count > 0;
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.forms.labels_or_instructions_present".to_string(),
                applicability: "applicable".to_string(),
                verification_mode: "hybrid".to_string(),
                verdict: if ctrl_fail { "fail" } else { "pass" }.to_string(),
                severity: if ctrl_fail { "high" } else { "medium" }.to_string(),
                confidence: "medium".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message: if ctrl_fail {
                    "Unlabeled form controls detected.".to_string()
                } else {
                    "Detected form controls have label/ARIA naming signals.".to_string()
                },
                evidence: vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![
                        (
                            "form_control_count".to_string(),
                            facts.form_control_count.to_string(),
                        ),
                        (
                            "unlabeled_form_control_count".to_string(),
                            facts.unlabeled_form_control_count.to_string(),
                        ),
                    ],
                }],
            });
        }

        if facts.invalid_form_control_count == 0 {
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.forms.error_identification_present".to_string(),
                applicability: "not_applicable".to_string(),
                verification_mode: "hybrid".to_string(),
                verdict: "not_applicable".to_string(),
                severity: "low".to_string(),
                confidence: "high".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message:
                    "No invalid form controls detected; error-identification rule not applicable."
                        .to_string(),
                evidence: Vec::new(),
            });
        } else {
            let err_fail = facts.unidentified_error_form_control_count > 0;
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.forms.error_identification_present".to_string(),
                applicability: "applicable".to_string(),
                verification_mode: "hybrid".to_string(),
                verdict: if err_fail { "fail" } else { "pass" }.to_string(),
                severity: if err_fail { "high" } else { "medium" }.to_string(),
                confidence: "medium".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message: if err_fail {
                    "Invalid form controls without associated error-identification text detected."
                        .to_string()
                } else {
                    "Invalid form controls expose associated error-identification text signals."
                        .to_string()
                },
                evidence: vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![
                        (
                            "invalid_form_control_count".to_string(),
                            facts.invalid_form_control_count.to_string(),
                        ),
                        (
                            "unidentified_error_form_control_count".to_string(),
                            facts.unidentified_error_form_control_count.to_string(),
                        ),
                    ],
                }],
            });
        }

        let focus_order_target_count = facts.link_count + facts.form_control_count;
        if focus_order_target_count == 0 {
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.focus.order_seed".to_string(),
                applicability: "not_applicable".to_string(),
                verification_mode: "hybrid".to_string(),
                verdict: "not_applicable".to_string(),
                severity: "medium".to_string(),
                confidence: "high".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message:
                    "No interactive links or form controls detected; focus-order seed not applicable."
                        .to_string(),
                evidence: vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![
                        (
                            "interactive_focus_target_count".to_string(),
                            focus_order_target_count.to_string(),
                        ),
                        ("link_count".to_string(), facts.link_count.to_string()),
                        (
                            "form_control_count".to_string(),
                            facts.form_control_count.to_string(),
                        ),
                        (
                            "tabindex_attr_count".to_string(),
                            facts.tabindex_attr_count.to_string(),
                        ),
                        (
                            "positive_tabindex_count".to_string(),
                            facts.positive_tabindex_count.to_string(),
                        ),
                        (
                            "invalid_tabindex_count".to_string(),
                            facts.invalid_tabindex_count.to_string(),
                        ),
                    ],
                }],
            });
        } else {
            let focus_order_warn =
                facts.positive_tabindex_count > 0 || facts.invalid_tabindex_count > 0;
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.focus.order_seed".to_string(),
                applicability: "applicable".to_string(),
                verification_mode: "hybrid".to_string(),
                verdict: if focus_order_warn { "warn" } else { "pass" }.to_string(),
                severity: "medium".to_string(),
                confidence: if focus_order_warn { "medium" } else { "medium" }.to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message: if facts.positive_tabindex_count > 0 {
                    "Positive tabindex values detected; focus order may diverge from DOM order and requires manual review."
                        .to_string()
                } else if facts.invalid_tabindex_count > 0 {
                    "Invalid tabindex values detected; focus order behavior may be inconsistent and requires manual review."
                        .to_string()
                } else {
                    "No positive/invalid tabindex focus-order override signals detected for interactive content."
                        .to_string()
                },
                evidence: vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![
                        (
                            "interactive_focus_target_count".to_string(),
                            focus_order_target_count.to_string(),
                        ),
                        ("link_count".to_string(), facts.link_count.to_string()),
                        (
                            "form_control_count".to_string(),
                            facts.form_control_count.to_string(),
                        ),
                        (
                            "tabindex_attr_count".to_string(),
                            facts.tabindex_attr_count.to_string(),
                        ),
                        (
                            "positive_tabindex_count".to_string(),
                            facts.positive_tabindex_count.to_string(),
                        ),
                        (
                            "invalid_tabindex_count".to_string(),
                            facts.invalid_tabindex_count.to_string(),
                        ),
                    ],
                }],
            });
        }

        if facts.link_count == 0 {
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.links.purpose_in_context".to_string(),
                applicability: "not_applicable".to_string(),
                verification_mode: "hybrid".to_string(),
                verdict: "not_applicable".to_string(),
                severity: "low".to_string(),
                confidence: "high".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message: "No links detected; link-purpose rule not applicable.".to_string(),
                evidence: Vec::new(),
            });
        } else {
            let link_fail = facts.unnamed_link_count > 0;
            let link_warn = facts.generic_link_text_count > 0;
            findings.push(A11yVerifierFinding {
                rule_id: "fb.a11y.links.purpose_in_context".to_string(),
                applicability: "applicable".to_string(),
                verification_mode: "hybrid".to_string(),
                verdict: if link_fail {
                    "fail"
                } else if link_warn {
                    "warn"
                } else {
                    "pass"
                }
                .to_string(),
                severity: if link_fail { "high" } else { "medium" }.to_string(),
                confidence: "medium".to_string(),
                stage: "post-emit".to_string(),
                source: "fullbleed".to_string(),
                message: if link_fail {
                    "Links without discernible text purpose signals detected.".to_string()
                } else if link_warn {
                    "Generic link text detected; contextual purpose may require manual review."
                        .to_string()
                } else {
                    "Detected links have discernible text purpose signals.".to_string()
                },
                evidence: vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![
                        ("link_count".to_string(), facts.link_count.to_string()),
                        (
                            "unnamed_link_count".to_string(),
                            facts.unnamed_link_count.to_string(),
                        ),
                        (
                            "generic_link_text_count".to_string(),
                            facts.generic_link_text_count.to_string(),
                        ),
                    ],
                }],
            });
        }

        let sensory_hits = Self::a11y_sensory_instruction_hits(&facts.body_text);
        findings.push(A11yVerifierFinding {
            rule_id: "fb.a11y.instructions.sensory_characteristics_seed".to_string(),
            applicability: "applicable".to_string(),
            verification_mode: "hybrid".to_string(),
            verdict: if sensory_hits.is_empty() { "pass" } else { "warn" }.to_string(),
            severity: "medium".to_string(),
            confidence: if sensory_hits.is_empty() { "high" } else { "medium" }.to_string(),
            stage: "post-emit".to_string(),
            source: "fullbleed".to_string(),
            message: if sensory_hits.is_empty() {
                "No obvious sensory-characteristics instruction phrases detected."
                    .to_string()
            } else {
                "Potential sensory-characteristics instruction phrases detected; manual review required."
                    .to_string()
            },
            evidence: vec![A11yVerifierEvidence {
                selector: None,
                values: vec![
                    (
                        "sensory_phrase_hit_count".to_string(),
                        sensory_hits.len().to_string(),
                    ),
                    ("sensory_phrase_hits".to_string(), sensory_hits.join("|")),
                ],
            }],
        });

        if facts.duplicate_ids.is_empty() {
            Self::push_a11y_finding(
                &mut findings,
                "fb.a11y.ids.duplicate_id",
                "pass",
                "critical",
                "post-emit",
                "fullbleed",
                "No duplicate IDs detected.".to_string(),
                Vec::new(),
            );
        } else {
            for dup in &facts.duplicate_ids {
                Self::push_a11y_finding(
                    &mut findings,
                    "fb.a11y.ids.duplicate_id",
                    "fail",
                    "critical",
                    "post-emit",
                    "fullbleed",
                    format!("Duplicate id {dup:?} detected."),
                    vec![A11yVerifierEvidence {
                        selector: None,
                        values: vec![("id".to_string(), dup.clone())],
                    }],
                );
            }
        }

        if facts.missing_idrefs.is_empty() {
            Self::push_a11y_finding(
                &mut findings,
                "fb.a11y.aria.reference_target_exists",
                "pass",
                "critical",
                "post-emit",
                "fullbleed",
                "No broken ARIA ID references detected.".to_string(),
                Vec::new(),
            );
        } else {
            for (attr, target) in &facts.missing_idrefs {
                Self::push_a11y_finding(
                    &mut findings,
                    "fb.a11y.aria.reference_target_exists",
                    "fail",
                    "critical",
                    "post-emit",
                    "fullbleed",
                    format!("{attr} references missing id {target:?}."),
                    vec![A11yVerifierEvidence {
                        selector: None,
                        values: vec![
                            ("attr".to_string(), attr.clone()),
                            ("target_id".to_string(), target.clone()),
                        ],
                    }],
                );
            }
        }

        Self::push_a11y_finding(
            &mut findings,
            "fb.a11y.aria.redundant_role_native_seed",
            if facts.redundant_role_native_count > 0 {
                "warn"
            } else {
                "pass"
            },
            "medium",
            "post-emit",
            "fullbleed",
            if facts.redundant_role_native_count > 0 {
                "Explicit roles duplicate native semantics on one or more elements.".to_string()
            } else {
                "No obvious redundant explicit-role/native-semantic duplication detected."
                    .to_string()
            },
            vec![A11yVerifierEvidence {
                selector: None,
                values: vec![(
                    "redundant_role_native_count".to_string(),
                    facts.redundant_role_native_count.to_string(),
                )],
            }],
        );
        Self::push_a11y_finding(
            &mut findings,
            "fb.a11y.aria.redundant_state_native_seed",
            if facts.redundant_state_native_count > 0 {
                "warn"
            } else {
                "pass"
            },
            "medium",
            "post-emit",
            "fullbleed",
            if facts.redundant_state_native_count > 0 {
                "ARIA state/property duplicates equivalent native HTML state on one or more elements."
                    .to_string()
            } else {
                "No obvious redundant ARIA state/native-state duplication detected.".to_string()
            },
            vec![A11yVerifierEvidence {
                selector: None,
                values: vec![(
                    "redundant_state_native_count".to_string(),
                    facts.redundant_state_native_count.to_string(),
                )],
            }],
        );

        if profile.eq_ignore_ascii_case("cav") || profile.eq_ignore_ascii_case("transactional") {
            let body_text_l = facts.body_text.to_ascii_lowercase();
            let sig_cue_present =
                body_text_l.contains("signature") || body_text_l.contains("signed");
            let sig_ok = facts.signature_semantic_count > 0;
            let sig_na = !sig_ok && !sig_cue_present;
            Self::push_a11y_finding(
                &mut findings,
                "fb.a11y.signatures.text_semantics_present",
                if sig_ok {
                    "pass"
                } else if sig_na {
                    "not_applicable"
                } else {
                    "fail"
                },
                "medium",
                "post-emit",
                "fullbleed",
                if sig_ok {
                    "Signature fields include text-first semantics.".to_string()
                } else if sig_na {
                    "No signature-bearing content cues detected; signature semantics check not applicable."
                        .to_string()
                } else {
                    "No text-first signature semantics detected.".to_string()
                },
                vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![
                        (
                            "signature_semantic_count".to_string(),
                            facts.signature_semantic_count.to_string(),
                        ),
                        (
                            "signature_cue_text_present".to_string(),
                            sig_cue_present.to_string(),
                        ),
                    ],
                }],
            );
        }

        let non_interference_signal_count = facts.script_element_count
            + facts.embedded_active_content_count
            + facts.autoplay_media_count
            + facts.blink_marquee_count
            + facts.inline_event_handler_attr_count
            + facts.meta_refresh_count;
        findings.push(A11yVerifierFinding {
            rule_id: "fb.a11y.claim.non_interference_seed".to_string(),
            applicability: "applicable".to_string(),
            verification_mode: "hybrid".to_string(),
            verdict: if non_interference_signal_count == 0 {
                "pass"
            } else {
                "warn"
            }
            .to_string(),
            severity: "medium".to_string(),
            confidence: if non_interference_signal_count == 0 {
                "high"
            } else {
                "medium"
            }
            .to_string(),
            stage: "adapter".to_string(),
            source: "adapter".to_string(),
            message: if non_interference_signal_count == 0 {
                "No obvious active-content non-interference risk signals detected in emitted HTML."
                    .to_string()
            } else {
                "Potential non-interference risk signals detected; manual review required."
                    .to_string()
            },
            evidence: vec![A11yVerifierEvidence {
                selector: None,
                values: vec![
                    (
                        "script_element_count".to_string(),
                        facts.script_element_count.to_string(),
                    ),
                    (
                        "embedded_active_content_count".to_string(),
                        facts.embedded_active_content_count.to_string(),
                    ),
                    (
                        "autoplay_media_count".to_string(),
                        facts.autoplay_media_count.to_string(),
                    ),
                    (
                        "blink_marquee_count".to_string(),
                        facts.blink_marquee_count.to_string(),
                    ),
                    (
                        "inline_event_handler_attr_count".to_string(),
                        facts.inline_event_handler_attr_count.to_string(),
                    ),
                    (
                        "meta_refresh_count".to_string(),
                        facts.meta_refresh_count.to_string(),
                    ),
                ],
            }],
        });

        A11yVerifierCoreReport {
            profile: profile.to_string(),
            findings,
            facts,
        }
    }

    fn pmr_push_audit(
        audits: &mut Vec<PmrCoreAudit>,
        audit_id: &str,
        category: &str,
        weight: f64,
        class_name: &str,
        verification_mode: &str,
        severity: &str,
        stage: &str,
        source: &str,
        verdict: &str,
        scored: bool,
        message: String,
        evidence: Vec<PmrCoreEvidence>,
        fix_hint: Option<String>,
    ) {
        let score = if scored {
            match verdict {
                "pass" => Some(1.0),
                "warn" => Some(0.5),
                "fail" => Some(0.0),
                _ => None,
            }
        } else {
            None
        };
        audits.push(PmrCoreAudit {
            audit_id: audit_id.to_string(),
            category: category.to_string(),
            weight,
            class_name: class_name.to_string(),
            verification_mode: verification_mode.to_string(),
            severity: severity.to_string(),
            stage: stage.to_string(),
            source: source.to_string(),
            verdict: verdict.to_string(),
            scored,
            score,
            message,
            fix_hint,
            evidence,
        });
    }

    fn pmr_push_contract_audit(
        audits: &mut Vec<PmrCoreAudit>,
        audit_id: &str,
        verification_mode: &str,
        verdict: &str,
        scored: bool,
        message: String,
        evidence: Vec<PmrCoreEvidence>,
        fix_hint: Option<String>,
    ) {
        let spec = audit_contract::pmr_audit_def(audit_id)
            .unwrap_or_else(|| panic!("missing PMR audit contract definition for {audit_id}"));
        Self::pmr_push_audit(
            audits,
            spec.id,
            spec.category,
            spec.weight,
            spec.class_name,
            verification_mode,
            spec.severity,
            spec.stage,
            "fullbleed",
            verdict,
            scored,
            message,
            evidence,
            fix_hint,
        );
    }

    fn pmr_clamp(v: f64, lo: f64, hi: f64) -> f64 {
        if v < lo {
            lo
        } else if v > hi {
            hi
        } else {
            v
        }
    }

    fn pmr_band(score: f64) -> &'static str {
        if score >= 95.0 {
            "excellent"
        } else if score >= 85.0 {
            "good"
        } else if score >= 70.0 {
            "watch"
        } else {
            "poor"
        }
    }

    fn pmr_note_hits(body_text: &str) -> Vec<String> {
        let lowered = body_text.to_ascii_lowercase();
        [
            "review queue",
            "parity report",
            "source analysis",
            "component validation",
            "a11y validation",
            "transcription sidecar",
            "debug log",
            "remediation note",
        ]
        .iter()
        .filter_map(|needle| {
            if lowered.contains(needle) {
                Some((*needle).to_string())
            } else {
                None
            }
        })
        .collect()
    }

    fn a11y_sensory_instruction_hits(body_text: &str) -> Vec<String> {
        let lowered = body_text.to_ascii_lowercase();
        [
            "see above",
            "see below",
            "shown above",
            "shown below",
            "on the left",
            "on the right",
            "left side",
            "right side",
            "top of the page",
            "bottom of the page",
            "red button",
            "green button",
            "blue button",
        ]
        .iter()
        .filter_map(|needle| {
            if lowered.contains(needle) {
                Some((*needle).to_string())
            } else {
                None
            }
        })
        .collect()
    }

    fn a11y_normalize_text(value: &str) -> String {
        value.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn a11y_text_tokens(value: &str) -> std::collections::BTreeSet<String> {
        let mut tokens: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut current = String::new();
        for ch in value.chars().flat_map(|c| c.to_lowercase()) {
            if ch.is_ascii_alphanumeric() {
                current.push(ch);
            } else if !current.is_empty() {
                tokens.insert(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            tokens.insert(current);
        }
        tokens
    }

    fn a11y_text_similarity(a: &str, b: &str) -> f64 {
        let ta = Self::a11y_text_tokens(a);
        let tb = Self::a11y_text_tokens(b);
        if ta.is_empty() || tb.is_empty() {
            return 0.0;
        }
        let intersection = ta.intersection(&tb).count() as f64;
        let union = ta.union(&tb).count() as f64;
        if union <= 0.0 {
            0.0
        } else {
            intersection / union
        }
    }

    fn a11y_lang_value_is_valid(lang: &str) -> bool {
        let lang = lang.trim();
        !lang.is_empty()
            && !lang.starts_with('-')
            && !lang.ends_with('-')
            && lang
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    }

    fn pmr_gate(audits: &[PmrCoreAudit], profile: &str, mode: &str) -> PmrCoreGate {
        let mode_norm = {
            let m = mode.trim().to_ascii_lowercase();
            if m.is_empty() { "error".to_string() } else { m }
        };
        let mut ec = 0usize;
        let mut wc = 0usize;
        let mut failed: Vec<String> = Vec::new();
        for audit in audits {
            let verdict = audit.verdict.as_str();
            if verdict != "fail" && verdict != "warn" {
                continue;
            }
            let level = audit_contract::pmr_effective_gate_level(profile, &audit.audit_id);
            if mode_norm == "off" || level == "off" {
                continue;
            }
            if mode_norm == "warn" {
                wc += 1;
                continue;
            }
            if verdict == "warn" {
                wc += 1;
            } else if level == "error" {
                ec += 1;
                failed.push(audit.audit_id.clone());
            } else {
                wc += 1;
            }
        }
        PmrCoreGate {
            ok: ec == 0,
            mode: mode_norm,
            error_count: ec,
            warn_count: wc,
            failed_audit_ids: failed,
        }
    }

    fn lang_is_valid(lang: &str) -> bool {
        !lang.trim().is_empty()
            && lang
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
            && !lang.starts_with('-')
            && !lang.ends_with('-')
    }

    pub fn verify_paged_media_rank_html_core(
        &self,
        html: &str,
        profile: &str,
        mode: &str,
        ctx: &PmrCoreContext,
    ) -> PmrCoreReport {
        let facts = self.verify_accessibility_html_facts(html);
        let mut audits: Vec<PmrCoreAudit> = Vec::new();

        let observed_lang = facts.html_lang.clone().unwrap_or_default();
        let expected_lang = self.document_lang().map(|value| value.to_string());
        let lang_value_valid = facts
            .html_lang
            .as_deref()
            .map(Self::lang_is_valid)
            .unwrap_or(false);
        let lang_pass = lang_value_valid
            && expected_lang
                .as_deref()
                .map(|expected| facts.html_lang.as_deref() == Some(expected))
                .unwrap_or(true);
        let lang_failure_kind = if facts.html_lang.is_none() {
            "missing"
        } else if !lang_value_valid {
            "invalid"
        } else if expected_lang
            .as_deref()
            .map(|expected| facts.html_lang.as_deref() != Some(expected))
            .unwrap_or(false)
        {
            "metadata_mismatch"
        } else {
            "none"
        };
        Self::pmr_push_contract_audit(
            &mut audits,
            "pmr.doc.lang_present_valid",
            "machine",
            if lang_pass { "pass" } else { "fail" },
            true,
            if lang_pass {
                "HTML lang is present and valid.".to_string()
            } else if lang_failure_kind == "metadata_mismatch" {
                format!(
                    "HTML lang is present and valid in the emitted DOM, but engine metadata persistence mismatched (observed DOM={}, expected metadata={}).",
                    observed_lang,
                    expected_lang.clone().unwrap_or_default()
                )
            } else if lang_failure_kind == "invalid" {
                "HTML lang attribute is present but invalid.".to_string()
            } else {
                "HTML lang attribute is missing.".to_string()
            },
            vec![PmrCoreEvidence {
                selector: Some("html".to_string()),
                diagnostic_ref: None,
                values: vec![
                    ("lang".to_string(), observed_lang),
                    (
                        "observed_lang".to_string(),
                        facts.html_lang.clone().unwrap_or_default(),
                    ),
                    (
                        "expected_document_lang".to_string(),
                        expected_lang.unwrap_or_default(),
                    ),
                    ("failure_kind".to_string(), lang_failure_kind.to_string()),
                ],
            }],
            None,
        );

        let expected_title = self.document_title().map(|value| value.to_string());
        let title_present = !facts.title.trim().is_empty();
        let title_pass = title_present
            && expected_title
                .as_deref()
                .map(|expected| facts.title == expected)
                .unwrap_or(true);
        let title_failure_kind = if !title_present {
            "missing"
        } else if expected_title
            .as_deref()
            .map(|expected| facts.title != expected)
            .unwrap_or(false)
        {
            "metadata_mismatch"
        } else {
            "none"
        };
        Self::pmr_push_contract_audit(
            &mut audits,
            "pmr.doc.title_present_nonempty",
            "machine",
            if title_pass { "pass" } else { "fail" },
            true,
            if title_pass {
                "Document title is present and non-empty.".to_string()
            } else if title_failure_kind == "metadata_mismatch" {
                format!(
                    "Document title is present in the emitted DOM, but engine metadata persistence mismatched (observed DOM={}, expected metadata={}).",
                    facts.title,
                    expected_title.clone().unwrap_or_default()
                )
            } else {
                "Document title is missing or empty.".to_string()
            },
            vec![PmrCoreEvidence {
                selector: Some("head > title".to_string()),
                diagnostic_ref: None,
                values: vec![
                    ("title".to_string(), facts.title.clone()),
                    ("observed_title".to_string(), facts.title.clone()),
                    (
                        "expected_document_title".to_string(),
                        expected_title.unwrap_or_default(),
                    ),
                    ("failure_kind".to_string(), title_failure_kind.to_string()),
                ],
            }],
            None,
        );

        if self.document_lang().is_none() && self.document_title().is_none() {
            Self::pmr_push_contract_audit(
                &mut audits,
                "pmr.doc.metadata_engine_persistence",
                "manual",
                "manual_needed",
                false,
                "Expected metadata not supplied; cannot verify engine persistence.".to_string(),
                Vec::new(),
                None,
            );
        } else {
            let ok = lang_pass && title_pass;
            Self::pmr_push_contract_audit(
                &mut audits,
                "pmr.doc.metadata_engine_persistence",
                "machine",
                if ok { "pass" } else { "fail" },
                true,
                if ok {
                    "Engine metadata persisted into emitted HTML.".to_string()
                } else {
                    "Engine metadata persistence check failed.".to_string()
                },
                Vec::new(),
                None,
            );
        }

        let pagination_summary = ctx.pagination_summary.as_ref();
        let pagination_overflow =
            pagination_summary.and_then(|summary| summary.overflow_event_count);
        let overflow = pagination_overflow.or(ctx.overflow_count).unwrap_or(0);
        Self::pmr_push_contract_audit(
            &mut audits,
            "pmr.layout.overflow_none",
            "machine",
            if overflow == 0 { "pass" } else { "fail" },
            true,
            if overflow == 0 {
                "No overflow placements detected.".to_string()
            } else {
                format!("Overflow placements detected ({overflow}).")
            },
            vec![PmrCoreEvidence {
                selector: None,
                diagnostic_ref: Some(
                    if pagination_overflow.is_some() {
                        "pagination_trace_summary.overflow_event_count"
                    } else {
                        "component_validation.overflow_count"
                    }
                    .to_string(),
                ),
                values: {
                    let mut values = vec![("overflow_count".to_string(), overflow.to_string())];
                    if let Some(count) = pagination_overflow {
                        values.push((
                            "pagination_overflow_event_count".to_string(),
                            count.to_string(),
                        ));
                    }
                    if let Some(count) = ctx.overflow_count {
                        values.push((
                            "component_validation_overflow_count".to_string(),
                            count.to_string(),
                        ));
                    }
                    values
                },
            }],
            None,
        );

        let known_loss = ctx.known_loss_count.unwrap_or(0);
        Self::pmr_push_contract_audit(
            &mut audits,
            "pmr.layout.known_loss_none_critical",
            "machine",
            if known_loss == 0 { "pass" } else { "fail" },
            true,
            if known_loss == 0 {
                "No critical known-loss events detected.".to_string()
            } else {
                format!("Known-loss events detected ({known_loss}).")
            },
            vec![PmrCoreEvidence {
                selector: None,
                diagnostic_ref: Some("component_validation.known_loss_count".to_string()),
                values: vec![("known_loss_count".to_string(), known_loss.to_string())],
            }],
            None,
        );

        let render_page_count = pagination_summary
            .and_then(|summary| summary.page_count)
            .or(ctx.render_page_count);
        match (ctx.source_page_count, render_page_count) {
            (Some(src), Some(rnd)) => {
                let ok = src == rnd;
                Self::pmr_push_contract_audit(
                    &mut audits,
                    "pmr.layout.page_count_target",
                    "machine",
                    if ok { "pass" } else { "fail" },
                    true,
                    if ok {
                        "Page-count target satisfied.".to_string()
                    } else {
                        format!("Page-count parity mismatch (source={src}, render={rnd}).")
                    },
                    vec![PmrCoreEvidence {
                        selector: None,
                        diagnostic_ref: pagination_summary
                            .and_then(|summary| summary.page_count)
                            .map(|_| "pagination_trace_summary.page_count".to_string()),
                        values: {
                            let mut values = vec![
                                ("source_page_count".to_string(), src.to_string()),
                                ("render_page_count".to_string(), rnd.to_string()),
                            ];
                            if let Some(count) =
                                pagination_summary.and_then(|summary| summary.page_count)
                            {
                                values.push((
                                    "pagination_trace_page_count".to_string(),
                                    count.to_string(),
                                ));
                            }
                            if let Some(count) = ctx.render_page_count {
                                values.push((
                                    "runtime_render_page_count".to_string(),
                                    count.to_string(),
                                ));
                            }
                            values
                        },
                    }],
                    None,
                );
            }
            _ => {
                Self::pmr_push_contract_audit(
                    &mut audits,
                    "pmr.layout.page_count_target",
                    "manual",
                    "manual_needed",
                    false,
                    "Page-count target could not be evaluated.".to_string(),
                    Vec::new(),
                    None,
                );
            }
        }

        let ids_ok = facts.duplicate_ids.is_empty() && facts.missing_idrefs.is_empty();
        Self::pmr_push_contract_audit(
            &mut audits,
            "pmr.forms.id_ref_integrity",
            "machine",
            if ids_ok { "pass" } else { "fail" },
            true,
            if ids_ok {
                "ID and IDREF integrity checks passed.".to_string()
            } else {
                "Duplicate IDs or missing IDREF targets detected.".to_string()
            },
            vec![PmrCoreEvidence {
                selector: None,
                diagnostic_ref: None,
                values: vec![
                    (
                        "duplicate_id_count".to_string(),
                        facts.duplicate_ids.len().to_string(),
                    ),
                    (
                        "missing_idref_count".to_string(),
                        facts.missing_idrefs.len().to_string(),
                    ),
                ],
            }],
            None,
        );

        if facts.tables.is_empty() {
            Self::pmr_push_contract_audit(
                &mut audits,
                "pmr.tables.semantic_table_headers",
                "machine",
                "not_applicable",
                false,
                "No table elements detected.".to_string(),
                Vec::new(),
                None,
            );
        } else {
            let mut ok = true;
            let mut evidence = Vec::new();
            for (idx, tbl) in facts.tables.iter().enumerate() {
                if tbl.th_count > 0 {
                    let this_ok = tbl.has_caption || tbl.th_scope_count > 0;
                    ok = ok && this_ok;
                    evidence.push(PmrCoreEvidence {
                        selector: None,
                        diagnostic_ref: None,
                        values: vec![
                            ("table_index".to_string(), idx.to_string()),
                            ("has_caption".to_string(), tbl.has_caption.to_string()),
                            ("th_count".to_string(), tbl.th_count.to_string()),
                            ("th_scope_count".to_string(), tbl.th_scope_count.to_string()),
                        ],
                    });
                }
            }
            if evidence.is_empty() {
                evidence.push(PmrCoreEvidence {
                    selector: None,
                    diagnostic_ref: None,
                    values: vec![("table_count".to_string(), facts.tables.len().to_string())],
                });
            }
            Self::pmr_push_contract_audit(
                &mut audits,
                "pmr.tables.semantic_table_headers",
                "machine",
                if ok { "pass" } else { "fail" },
                true,
                if ok {
                    "Semantic table header checks passed.".to_string()
                } else {
                    "Semantic table header checks failed.".to_string()
                },
                evidence,
                None,
            );
        }

        let profile_l = profile.to_ascii_lowercase();
        if profile_l == "cav" || profile_l == "transactional" {
            let body_text_l = facts.body_text.to_ascii_lowercase();
            let sig_cue_present =
                body_text_l.contains("signature") || body_text_l.contains("signed");
            let sig_ok = facts.signature_semantic_count > 0;
            let sig_na = !sig_ok && !sig_cue_present;
            Self::pmr_push_contract_audit(
                &mut audits,
                "pmr.signatures.text_semantics_present",
                "machine",
                if sig_ok {
                    "pass"
                } else if sig_na {
                    "not_applicable"
                } else {
                    "fail"
                },
                !sig_na,
                if sig_ok {
                    "Text signature semantics detected.".to_string()
                } else if sig_na {
                    "No signature-bearing content cues detected; signature semantics check not applicable."
                        .to_string()
                } else {
                    "No text signature semantics detected.".to_string()
                },
                vec![PmrCoreEvidence {
                    selector: None,
                    diagnostic_ref: None,
                    values: vec![
                        (
                            "signature_semantic_count".to_string(),
                            facts.signature_semantic_count.to_string(),
                        ),
                        (
                            "signature_cue_text_present".to_string(),
                            sig_cue_present.to_string(),
                        ),
                    ],
                }],
                None,
            );
        } else {
            Self::pmr_push_contract_audit(
                &mut audits,
                "pmr.signatures.text_semantics_present",
                "machine",
                "not_applicable",
                false,
                "Not applicable for this profile.".to_string(),
                Vec::new(),
                None,
            );
        }

        if profile_l == "cav" {
            let hits = Self::pmr_note_hits(&facts.body_text);
            let mut ev = Vec::new();
            if !hits.is_empty() {
                ev.push(PmrCoreEvidence {
                    selector: None,
                    diagnostic_ref: None,
                    values: vec![("hits".to_string(), hits.join(", "))],
                });
            }
            Self::pmr_push_contract_audit(
                &mut audits,
                "pmr.cav.document_only_content",
                "machine",
                if hits.is_empty() { "pass" } else { "fail" },
                true,
                if hits.is_empty() {
                    "CAV deliverable body contains document-only content.".to_string()
                } else {
                    "Potential remediation/provenance note leakage detected in CAV deliverable body."
                        .to_string()
                },
                ev,
                None,
            );
        } else {
            Self::pmr_push_contract_audit(
                &mut audits,
                "pmr.cav.document_only_content",
                "machine",
                "not_applicable",
                false,
                "Not a CAV profile.".to_string(),
                Vec::new(),
                None,
            );
        }

        let html_ok = ctx
            .html_artifact_bytes
            .map(|n| n > 0)
            .unwrap_or(!html.is_empty());
        Self::pmr_push_contract_audit(
            &mut audits,
            "pmr.artifacts.html_emitted",
            "machine",
            if html_ok { "pass" } else { "fail" },
            true,
            if html_ok {
                "HTML artifact emitted.".to_string()
            } else {
                "HTML artifact missing or empty.".to_string()
            },
            Vec::new(),
            None,
        );

        let css_ok = ctx.css_artifact_bytes.map(|n| n > 0).unwrap_or(true);
        Self::pmr_push_contract_audit(
            &mut audits,
            "pmr.artifacts.css_emitted",
            "machine",
            if css_ok { "pass" } else { "fail" },
            true,
            if css_ok {
                "CSS artifact emitted.".to_string()
            } else {
                "CSS artifact missing or empty.".to_string()
            },
            Vec::new(),
            None,
        );

        Self::pmr_push_contract_audit(
            &mut audits,
            "pmr.artifacts.linked_css_reference",
            "machine",
            if facts.has_css_link { "pass" } else { "warn" },
            false,
            if facts.has_css_link {
                "HTML artifact includes linked CSS reference.".to_string()
            } else {
                "HTML artifact does not include linked CSS reference (separate artifact mode)."
                    .to_string()
            },
            vec![PmrCoreEvidence {
                selector: Some("link[rel~=stylesheet]".to_string()),
                diagnostic_ref: None,
                values: vec![("hrefs".to_string(), facts.css_link_hrefs.join(", "))],
            }],
            if facts.has_css_link {
                None
            } else {
                Some(
                    "Enable CSS link injection packaging mode for standalone HTML artifacts."
                        .to_string(),
                )
            },
        );

        let review_queue_items = ctx.review_queue_items.unwrap_or(0).max(0) as usize;
        let mut manual_debt_items = Vec::new();
        if review_queue_items > 0 {
            manual_debt_items.push(PmrCoreManualDebtItem {
                id: "manual.transcription_quality.review_queue".to_string(),
                reason: format!(
                    "{review_queue_items} review-queue item(s) require human verification."
                ),
                severity: "medium".to_string(),
                category: None,
            });
        }

        let mut categories = Vec::new();
        for cat_def in audit_contract::pmr_category_defs_v1() {
            let cid = cat_def.id;
            let name = cat_def.name;
            let weight = cat_def.weight;
            let subset: Vec<&PmrCoreAudit> = audits.iter().filter(|a| a.category == cid).collect();
            let scored: Vec<(f64, f64)> = subset
                .iter()
                .filter_map(|a| {
                    if a.scored {
                        a.score.map(|s| (s, a.weight))
                    } else {
                        None
                    }
                })
                .collect();
            let denom = scored.iter().map(|(_, w)| *w).sum::<f64>();
            let cat_score = if scored.is_empty() {
                100.0
            } else {
                let d = if denom == 0.0 { 1.0 } else { denom };
                100.0 * (scored.iter().map(|(s, w)| s * w).sum::<f64>() / d)
            };
            let warn_n = subset.iter().filter(|a| a.verdict == "warn").count();
            let fail_n = subset.iter().filter(|a| a.verdict == "fail").count();
            let manual_n = subset
                .iter()
                .filter(|a| a.verdict == "manual_needed")
                .count();
            let conf = Self::pmr_clamp(
                100.0 - (10.0 * manual_n as f64) - (3.0 * warn_n as f64) - (5.0 * fail_n as f64),
                0.0,
                100.0,
            );
            categories.push(PmrCoreCategory {
                id: cid.to_string(),
                name: name.to_string(),
                weight,
                score: ((cat_score * 100.0).round()) / 100.0,
                confidence: ((conf * 100.0).round()) / 100.0,
                audit_count: subset.len(),
                fail_count: fail_n,
                warn_count: warn_n,
            });
        }

        let cat_weight_sum = categories.iter().map(|c| c.weight).sum::<f64>();
        let cat_weight_denom = if cat_weight_sum == 0.0 {
            1.0
        } else {
            cat_weight_sum
        };
        let score = categories.iter().map(|c| c.score * c.weight).sum::<f64>() / cat_weight_denom;
        let mut confidence = categories
            .iter()
            .map(|c| c.confidence * c.weight)
            .sum::<f64>()
            / cat_weight_denom;
        if review_queue_items > 0 {
            confidence = Self::pmr_clamp(
                confidence - (3.0 * review_queue_items as f64).min(25.0),
                0.0,
                100.0,
            );
        }

        let gate = Self::pmr_gate(&audits, profile, mode);
        let coverage = PmrCoreCoverage {
            evaluated_audit_count: audits.len(),
            applicable_audit_count: audits
                .iter()
                .filter(|a| a.verdict != "not_applicable")
                .count(),
            scored_audit_count: audits.iter().filter(|a| a.scored).count(),
            manual_needed_count: audits
                .iter()
                .filter(|a| a.verdict == "manual_needed")
                .count(),
            not_evaluated_audit_count: 0,
        };
        PmrCoreReport {
            profile: profile.to_string(),
            mode: gate.mode.clone(),
            audits,
            categories,
            manual_debt_item_count: review_queue_items,
            manual_debt_high_risk_count: 0,
            manual_debt_items,
            coverage,
            rank: PmrCoreRank {
                score: ((score * 100.0).round()) / 100.0,
                confidence: ((confidence * 100.0).round()) / 100.0,
                band: Self::pmr_band(score).to_string(),
                raw_score: ((score * 100.0).round()) / 100.0,
            },
            gate,
            facts,
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
        self.build_document_with_layout_strategy_input(
            doc_id,
            HtmlLayoutInput::Source(html),
            page_templates,
            resolver,
            report,
        )
    }

    fn build_document_with_layout_strategy_from_document(
        &self,
        doc_id: usize,
        document: &NodeRef,
        page_templates: &[PageTemplate],
        resolver: &style::StyleResolver,
        report: Option<&mut GlyphCoverageReport>,
    ) -> Result<LayoutBuildResult, FullBleedError> {
        self.build_document_with_layout_strategy_input(
            doc_id,
            HtmlLayoutInput::Parsed(document),
            page_templates,
            resolver,
            report,
        )
    }

    fn build_document_with_layout_strategy_input(
        &self,
        doc_id: usize,
        input: HtmlLayoutInput<'_>,
        page_templates: &[PageTemplate],
        resolver: &style::StyleResolver,
        report: Option<&mut GlyphCoverageReport>,
    ) -> Result<LayoutBuildResult, FullBleedError> {
        let lazy = self.layout_strategy == LayoutStrategy::Lazy;
        let uses_target_counters = resolver.has_target_counter_content();
        let pass_limit = self
            .layout_pass_limit()
            .max(if uses_target_counters { 4 } else { 1 });
        let started = std::time::Instant::now();
        let mut story_ms = 0.0;
        let mut layout_ms = 0.0;
        let mut passes = 0usize;
        let mut converged = false;
        let mut budget_hit = false;
        let mut previous_signature: Option<u64> = None;
        let mut target_pages = Arc::new(HashMap::new());
        let mut built: Option<Document> = None;
        let mut report = report;
        let collect_report = report.is_some();
        let mut final_report: Option<GlyphCoverageReport> = None;
        let canvas_background = match input {
            HtmlLayoutInput::Source(html) => html::document_canvas_background(html, resolver),
            HtmlLayoutInput::Parsed(document) => {
                html::document_canvas_background_for_document(document, resolver)
            }
        };
        let page_templates = if let Some((color, alpha)) = canvas_background {
            page_templates
                .iter()
                .cloned()
                .map(|template| {
                    let Some(frame) = template.primary_frame_rect() else {
                        return template;
                    };
                    template.append_on_page(move |canvas, _| {
                        // A PDF rectangle grows from its block-end edge
                        // toward block-start.  Poppler owns the exact far
                        // edge for the fill, while Chromium leaves that
                        // coincident row to the page background.  Retire
                        // one serialized millipoint at a nonzero page-area
                        // start without moving the block-end edge.
                        let far_edge_guard = Pt::from_milli_i64(1);
                        let block_start_guard =
                            if frame.y > Pt::ZERO && frame.height > far_edge_guard {
                                far_edge_guard
                            } else {
                                Pt::ZERO
                            };
                        canvas.meta(canvas::META_HTML_CANVAS_BACKGROUND_KEY, "begin");
                        canvas.save_state();
                        canvas.set_fill_color(color);
                        if alpha < 1.0 {
                            canvas.set_opacity(alpha, alpha);
                        }
                        canvas.draw_rect(
                            frame.x,
                            frame.y + block_start_guard,
                            frame.width,
                            frame.height - block_start_guard,
                        );
                        canvas.restore_state();
                        canvas.meta(canvas::META_HTML_CANVAS_BACKGROUND_KEY, "end");
                    })
                })
                .collect::<Vec<_>>()
        } else {
            page_templates.to_vec()
        };

        for pass in 0..pass_limit {
            if lazy
                && !uses_target_counters
                && pass > 0
                && started.elapsed().as_secs_f64() * 1000.0 >= self.lazy_budget_ms
            {
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
            let story = match input {
                HtmlLayoutInput::Source(html) => {
                    html::html_to_story_with_resolver_and_fonts_and_report_and_target_pages(
                        html,
                        resolver,
                        Some(self.font_registry.clone()),
                        Some(self.asset_bundle.clone()),
                        pass_report_ref.as_deref_mut(),
                        self.svg_form_xobjects,
                        self.svg_raster_fallback,
                        self.perf.as_deref(),
                        Some(doc_id),
                        uses_target_counters.then(|| target_pages.clone()),
                    )
                }
                HtmlLayoutInput::Parsed(document) => {
                    html::html_document_to_story_with_resolver_and_fonts_and_report_and_target_pages(
                        document,
                        resolver,
                        Some(self.font_registry.clone()),
                        Some(self.asset_bundle.clone()),
                        pass_report_ref.as_deref_mut(),
                        self.svg_form_xobjects,
                        self.svg_raster_fallback,
                        self.perf.as_deref(),
                        Some(doc_id),
                        uses_target_counters.then(|| target_pages.clone()),
                    )
                }
            };
            story_ms += t_story.elapsed().as_secs_f64() * 1000.0;

            let mut doc = DocTemplate::new(page_templates.clone());
            if let Some(logger) = self.debug.clone() {
                doc = doc.with_debug(logger, Some(doc_id));
            }
            for flowable in story {
                doc.add_flowable(flowable);
            }

            let t_layout = std::time::Instant::now();
            let _perf_guard = flowable::set_perf_context(self.perf.clone(), Some(doc_id));
            let mut next_built = doc.build()?;
            apply_html_page_shrink_to_fit(&mut next_built);
            layout_ms += t_layout.elapsed().as_secs_f64() * 1000.0;

            let signature = document_layout_signature(&next_built);
            let next_target_pages = document_target_pages(&next_built);
            let target_pages_converged =
                !uses_target_counters || (pass > 0 && target_pages.as_ref() == &next_target_pages);
            let layout_converged =
                !lazy || previous_signature.is_some_and(|last| last == signature);
            converged = layout_converged && target_pages_converged;
            previous_signature = Some(signature);
            target_pages = Arc::new(next_target_pages);
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

    fn resolve_css_page_context(
        &self,
        merged_css: &str,
        doc_id: Option<usize>,
    ) -> ResolvedCssPageContext {
        let mut page_size = self.default_page_size;
        let mut base_margins = self.default_margins;
        let mut page_margins = self.page_margins.clone();
        let mut page_styles = style::extract_css_page_styles(
            merged_css,
            self.debug.as_deref(),
            Some(self.default_page_size),
        );
        let page_setup = page_styles.base.clone();

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

        if let Some(css_margins) = page_setup.resolve_margins(base_margins, page_size) {
            if self.margins_explicit {
                if let Some(logger) = self.debug.as_deref() {
                    logger.increment("jit.page_margin.css_overridden", 1);
                }
            } else {
                base_margins = css_margins;
                page_margins.clear();
            }
        }

        if self.margins_explicit {
            for setup in [
                &mut page_styles.base,
                &mut page_styles.first,
                &mut page_styles.left,
                &mut page_styles.right,
                &mut page_styles.blank,
            ] {
                setup.margin_top = None;
                setup.margin_right = None;
                setup.margin_bottom = None;
                setup.margin_left = None;
                setup.margin_top_percent = None;
                setup.margin_right_percent = None;
                setup.margin_bottom_percent = None;
                setup.margin_left_percent = None;
            }
            for named in &mut page_styles.named {
                for setup in [
                    &mut named.base,
                    &mut named.first,
                    &mut named.left,
                    &mut named.right,
                    &mut named.blank,
                ] {
                    setup.margin_top = None;
                    setup.margin_right = None;
                    setup.margin_bottom = None;
                    setup.margin_left = None;
                    setup.margin_top_percent = None;
                    setup.margin_right_percent = None;
                    setup.margin_bottom_percent = None;
                    setup.margin_left_percent = None;
                }
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

        ResolvedCssPageContext {
            page_size,
            base_margins,
            page_margins,
            page_styles,
        }
    }

    fn compile_css_render_parts(
        &self,
        merged_css: &str,
        doc_id: Option<usize>,
    ) -> (Vec<PageTemplate>, Arc<style::StyleResolver>) {
        let resolved = self.resolve_css_page_context(merged_css, doc_id);
        let resolver_viewport =
            if resolved.page_margins.is_empty() && resolved.page_styles.has_pseudo_rules() {
                resolved
                    .page_styles
                    .base
                    .cascaded_with(&resolved.page_styles.right)
                    .cascaded_with(&resolved.page_styles.first)
                    .size
                    .unwrap_or(resolved.page_size)
            } else {
                resolved.page_size
            };
        let resolver = Arc::new(style::StyleResolver::new_with_debug_viewport_and_fonts(
            merged_css,
            self.debug.clone(),
            Some(resolver_viewport),
            Some(self.font_registry.clone()),
        ));
        let root_style = resolver.computed_root_element_style();
        let root_text = PageRootTextContext {
            style: root_style.to_text_style(),
            line_height: root_style.page_context_line_height(),
        };
        let page_templates = build_page_templates(
            resolved.page_size,
            resolved.base_margins,
            &resolved.page_margins,
            resolved.page_styles,
            self.font_registry.clone(),
            root_text,
        );
        (page_templates, resolver)
    }

    #[cfg(test)]
    fn resolve_page_templates_for_css(
        &self,
        merged_css: &str,
        doc_id: Option<usize>,
    ) -> Vec<PageTemplate> {
        self.compile_css_render_parts(merged_css, doc_id).0
    }

    fn build_render_context(&self, css: &str, doc_id: Option<usize>) -> RenderContext {
        let t_css = std::time::Instant::now();
        let cached_context = self
            .render_context_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(css));
        if let Some(context) = cached_context {
            let lookup_ms = t_css.elapsed().as_secs_f64() * 1000.0;
            if let Some(logger) = self.debug.as_deref() {
                let doc_id = doc_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "null".to_string());
                logger.log_json(&format!(
                    "{{\"type\":\"jit.css\",\"doc_id\":{},\"css_ms\":{:.3},\"bytes\":{},\"cache_hit\":true}}",
                    doc_id,
                    lookup_ms,
                    css.len()
                ));
            }
            if let Some(logger) = self.perf.as_deref() {
                logger.log_span_ms("css.cache_lookup", doc_id, lookup_ms);
                logger.log_counts("css.cache", doc_id, &[("hits", 1), ("misses", 0)]);
            }
            return context;
        }
        let merged_css = self.merge_css(css);
        let (page_templates, resolver) = self.compile_css_render_parts(&merged_css, doc_id);
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
            logger.log_counts("css.cache", doc_id, &[("hits", 0), ("misses", 1)]);
        }
        let context = RenderContext {
            resolver,
            page_templates: page_templates.into(),
        };
        if let Ok(mut cache) = self.render_context_cache.lock() {
            cache.insert(css.to_string(), context.clone());
        }
        context
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
                    Some(self.asset_bundle.clone()),
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
                Some(self.asset_bundle.clone()),
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
                    Some(self.asset_bundle.clone()),
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
        self.render_to_document_and_page_data_with_resolver_and_report_input_at(
            doc_id,
            HtmlLayoutInput::Source(html),
            page_templates,
            resolver,
            report,
        )
    }

    fn render_to_document_and_page_data_with_parsed_resolver_and_report_at(
        &self,
        doc_id: usize,
        document: &NodeRef,
        page_templates: &[PageTemplate],
        resolver: &style::StyleResolver,
        report: Option<&mut GlyphCoverageReport>,
    ) -> Result<(Document, Option<PageDataContext>), FullBleedError> {
        self.render_to_document_and_page_data_with_resolver_and_report_input_at(
            doc_id,
            HtmlLayoutInput::Parsed(document),
            page_templates,
            resolver,
            report,
        )
    }

    fn render_to_document_and_page_data_with_resolver_and_report_input_at(
        &self,
        doc_id: usize,
        input: HtmlLayoutInput<'_>,
        page_templates: &[PageTemplate],
        resolver: &style::StyleResolver,
        report: Option<&mut GlyphCoverageReport>,
    ) -> Result<(Document, Option<PageDataContext>), FullBleedError> {
        let mut report = report;
        let perf = self.perf.as_deref();
        if let HtmlLayoutInput::Source(html) = input {
            self.emit_html_asset_warnings(doc_id, html);
        }
        let layout = match input {
            HtmlLayoutInput::Source(html) => self.build_document_with_layout_strategy(
                doc_id,
                html,
                page_templates,
                resolver,
                report.as_deref_mut(),
            )?,
            HtmlLayoutInput::Parsed(document) => self
                .build_document_with_layout_strategy_from_document(
                    doc_id,
                    document,
                    page_templates,
                    resolver,
                    report.as_deref_mut(),
                )?,
        };
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

    /// Compile HTML/CSS into both the immutable fixed-point display document and, when text slots
    /// are present, a parsed-DOM reflow program with a reusable CSS context.
    pub fn compile_document(
        &self,
        html: &str,
        css: &str,
    ) -> Result<CompiledDocument, FullBleedError> {
        let started = std::time::Instant::now();
        let context = self.build_render_context(css, Some(0));
        let parsed_template = html_dom::parse_html(html);
        self.emit_html_asset_warnings(0, html);
        let document = Arc::new(
            self.render_to_document_and_page_data_with_parsed_resolver_and_report_at(
                0,
                &parsed_template,
                &context.page_templates,
                &context.resolver,
                None,
            )?
            .0,
        );
        let command_count = document.pages.iter().map(|page| page.commands.len()).sum();
        let mut binding_slots = BTreeSet::new();
        for page in &document.pages {
            collect_binding_slots(&page.commands, &mut binding_slots);
        }
        let binding_slots = binding_slots.into_iter().collect::<Vec<_>>();
        let binding_plan = (!binding_slots.is_empty()).then(|| {
            pdf::compile_binding_plan(&document, &binding_slots)
                .map(Arc::new)
                .map_err(|error| Arc::<str>::from(error.to_string()))
        });
        let reflow_plan =
            match html_dom::CompiledHtmlBindingTemplate::compile_document(&parsed_template) {
                Ok(template) if template.slot_names().is_empty() => None,
                Ok(template) => Some(Ok(Arc::new(CompiledReflowPlan {
                    template,
                    context,
                    runtime: Arc::new(self.clone_for_compiled_reflow()),
                    flow_programs: Mutex::new(HashMap::new()),
                    html_input_cache: html_dom::CompiledFlowHtmlInputCache::default(),
                    shape_cache: pdf::CompiledFlowShapeCache::default(),
                    pdf_program_cache: Mutex::new(None),
                }))),
                Err(error) => Some(Err(Arc::<str>::from(error))),
            };
        let reflow_slot_count = reflow_plan
            .as_ref()
            .and_then(|plan| plan.as_ref().ok())
            .map_or(0, |plan| plan.template.slot_names().len());
        let elapsed = started.elapsed();
        let compile_nanos = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX);
        if let Some(logger) = self.perf.as_deref() {
            logger.log_span_ms("compile.document", Some(0), elapsed.as_secs_f64() * 1000.0);
            logger.log_counts(
                "compile.document",
                Some(0),
                &[
                    ("pages", document.pages.len() as u64),
                    ("commands", command_count as u64),
                    ("reflow_slots", reflow_slot_count as u64),
                ],
            );
        }
        Ok(CompiledDocument {
            document,
            font_registry: self.font_registry.clone(),
            pdf_options: self.pdf_options.clone(),
            debug: self.debug.clone(),
            perf: self.perf.clone(),
            compile_nanos,
            command_count,
            binding_slots,
            binding_plan,
            reflow_plan,
        })
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

    pub fn render_to_document_with_glyph_report(
        &self,
        html: &str,
        css: &str,
    ) -> Result<(Document, GlyphCoverageReport), FullBleedError> {
        let context = self.build_render_context(css, Some(0));
        let mut report = GlyphCoverageReport::default();
        let (document, _page_data) = self
            .render_to_document_and_page_data_with_resolver_and_report(
                html,
                &context.page_templates,
                &context.resolver,
                Some(&mut report),
            )?;
        self.emit_debug_summary("render_to_document_with_glyph_report");
        Ok((document, report))
    }

    pub fn render_with_glyph_report_and_document(
        &self,
        html: &str,
        css: &str,
    ) -> Result<(Vec<u8>, GlyphCoverageReport, Document), FullBleedError> {
        let (document, report) = self.render_to_document_with_glyph_report(html, css)?;
        let bytes = pdf::document_to_pdf_with_metrics_and_registry_with_logs(
            &document,
            None,
            Some(self.font_registry.as_ref()),
            &self.pdf_options,
            self.debug.clone(),
            self.perf.clone(),
        )?;
        self.emit_debug_summary("render_with_glyph_report_and_document");
        Ok((bytes, report, document))
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
        render_to_buffered_file(path, |writer| self.render_to_writer(html, css, writer))
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
        render_to_buffered_file(path, |writer| {
            self.render_many_to_writer(html_list, css, writer)
        })
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
        render_to_buffered_file(path, |writer| {
            self.render_many_to_writer_with_css(jobs, writer)
        })
    }

    // Parallel batch rendering: build documents in parallel, then merge in input order.
    pub fn render_many_to_buffer_parallel(
        &self,
        html_list: &[String],
        css: &str,
    ) -> Result<Vec<u8>, FullBleedError> {
        self.render_many_to_buffer_parallel_ordered(html_list, css, None)
    }

    /// Render independent source documents in input order with cooperative cancellation.
    ///
    /// Cancellation is observed before and after each record layout and before the final PDF link.
    /// A record already inside layout is allowed to reach its next safe boundary.
    pub fn render_many_to_buffer_parallel_cancellable(
        &self,
        html_list: &[String],
        css: &str,
        cancellation: &AuthoringCancellationToken,
    ) -> Result<Vec<u8>, FullBleedError> {
        self.render_many_to_buffer_parallel_ordered(html_list, css, Some(cancellation))
    }

    fn render_many_to_buffer_parallel_ordered(
        &self,
        html_list: &[String],
        css: &str,
        cancellation: Option<&AuthoringCancellationToken>,
    ) -> Result<Vec<u8>, FullBleedError> {
        if cancellation.is_some_and(AuthoringCancellationToken::is_cancelled) {
            return Err(FullBleedError::Cancelled);
        }
        let context = self.build_render_context(css, None);
        let mut results: Vec<(usize, Result<Document, FullBleedError>)> =
            crate::parallel::map_indexed_ordered(html_list, |idx, html| {
                if cancellation.is_some_and(AuthoringCancellationToken::is_cancelled) {
                    return (idx, Err(FullBleedError::Cancelled));
                }
                let res = self
                    .render_to_document_and_page_data_with_resolver_and_report_at(
                        idx,
                        html,
                        &context.page_templates,
                        &context.resolver,
                        None,
                    )
                    .map(|(doc, _page_data)| doc)
                    .and_then(|document| {
                        if cancellation.is_some_and(AuthoringCancellationToken::is_cancelled) {
                            Err(FullBleedError::Cancelled)
                        } else {
                            Ok(document)
                        }
                    });
                (idx, res)
            });
        results.sort_by_key(|(idx, _)| *idx);

        let mut documents = Vec::with_capacity(results.len());
        for (_, res) in results {
            documents.push(res?);
        }

        if cancellation.is_some_and(AuthoringCancellationToken::is_cancelled) {
            return Err(FullBleedError::Cancelled);
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
        if cancellation.is_some_and(AuthoringCancellationToken::is_cancelled) {
            return Err(FullBleedError::Cancelled);
        }
        Ok(bytes)
    }

    pub fn render_many_to_buffer_parallel_with_page_data(
        &self,
        html_list: &[String],
        css: &str,
    ) -> Result<(Vec<u8>, Vec<Option<PageDataContext>>), FullBleedError> {
        let context = self.build_render_context(css, None);
        let mut results: Vec<(
            usize,
            Result<(Document, Option<PageDataContext>), FullBleedError>,
        )> = crate::parallel::map_indexed_ordered(html_list, |idx, html| {
            let res = self.render_to_document_and_page_data_with_resolver_and_report_at(
                idx,
                html,
                &context.page_templates,
                &context.resolver,
                None,
            );
            (idx, res)
        });
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
            let parallelism = crate::parallel::current_num_threads();
            let buffer_cap = (parallelism * 4).min(256);
            let (tx, rx) =
                mpsc::sync_channel::<(usize, Result<Document, FullBleedError>)>(buffer_cap);
            let mut render_error: Option<FullBleedError> = None;

            thread::scope(|scope| {
                let rx = rx;
                let spill_store = spill_store.as_ref();

                // Producer: plan + paint in parallel.
                scope.spawn(|| {
                    crate::parallel::with_thread_count(parallelism, || {
                        crate::parallel::for_each_indexed(html_list, |idx, html| {
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

        // Pipeline: render HTML->Document on scoped worker threads while a single writer thread
        // serializes to PDF in input order. This keeps memory bounded and keeps CPU busy.
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
        let parallelism = crate::parallel::current_num_threads();
        let buffer_cap = (parallelism * 4).min(256);
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
                crate::parallel::with_thread_count(parallelism, || {
                    crate::parallel::for_each_indexed(html_list, |idx, html| {
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
        let context = self.build_render_context(css, None);
        let mut results: Vec<(
            usize,
            Result<(Document, Option<PageDataContext>), FullBleedError>,
        )> = crate::parallel::map_indexed_ordered(html_list, |idx, html| {
            let res = self.render_to_document_and_page_data_with_resolver_and_report_at(
                idx,
                html,
                &context.page_templates,
                &context.resolver,
                None,
            );
            (idx, res)
        });
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
        render_to_buffered_file(path, |writer| {
            self.render_many_to_writer_parallel(html_list, css, writer)
        })
    }

    pub fn render_many_to_file_parallel_with_page_data(
        &self,
        html_list: &[String],
        css: &str,
        path: impl AsRef<std::path::Path>,
    ) -> Result<(usize, Vec<Option<PageDataContext>>), FullBleedError> {
        render_to_buffered_file(path, |writer| {
            self.render_many_to_writer_parallel_with_page_data(html_list, css, writer)
        })
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

    // Toggle shaping for complex scripts. Disabling skips native OpenType shaping and
    // uses direct codepoint->gid mapping for Identity-H fonts.
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

    pub fn clear_document_lang(mut self) -> Self {
        self.pdf_options.document_lang = None;
        self
    }

    pub fn document_lang_value(&self) -> Option<&str> {
        self.pdf_options.document_lang.as_deref()
    }

    // Document title for metadata (Info + XMP).
    pub fn document_title(mut self, title: impl Into<String>) -> Self {
        self.pdf_options.document_title = Some(title.into());
        self
    }

    pub fn clear_document_title(mut self) -> Self {
        self.pdf_options.document_title = None;
        self
    }

    pub fn document_title_value(&self) -> Option<&str> {
        self.pdf_options.document_title.as_deref()
    }

    // Toggle Unicode-aware layout measurements (native OpenType shaping).
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
            registry.register_bundle_font_bytes(asset.data.clone(), Some(&asset.name))?;
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
            asset_bundle: Arc::new(self.asset_bundle),
            render_context_cache: Mutex::new(RenderContextCache::new()),
        })
    }
}

fn build_page_templates(
    page_size: Size,
    base_margins: Margins,
    page_margins: &std::collections::BTreeMap<usize, Margins>,
    page_styles: style::CssPageStyles,
    font_registry: Arc<FontRegistry>,
    root_text: PageRootTextContext,
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
        if page_styles.has_pseudo_rules() {
            let first = page_styles
                .base
                .cascaded_with(&page_styles.right)
                .cascaded_with(&page_styles.first);
            templates.push(build_css_page_template(
                "First",
                page_size,
                base_margins,
                first,
                PageSelector::First,
                font_registry.clone(),
                root_text.clone(),
            ));
            templates.push(build_css_page_template(
                "Left",
                page_size,
                base_margins,
                page_styles.base.cascaded_with(&page_styles.left),
                PageSelector::Left,
                font_registry.clone(),
                root_text.clone(),
            ));
            templates.push(build_css_page_template(
                "Right",
                page_size,
                base_margins,
                page_styles.base.cascaded_with(&page_styles.right),
                PageSelector::Right,
                font_registry.clone(),
                root_text.clone(),
            ));
            templates.push(build_css_page_template(
                "BlankLeft",
                page_size,
                base_margins,
                page_styles
                    .base
                    .cascaded_with(&page_styles.left)
                    .cascaded_with(&page_styles.blank),
                PageSelector::BlankLeft,
                font_registry.clone(),
                root_text.clone(),
            ));
            templates.push(build_css_page_template(
                "BlankRight",
                page_size,
                base_margins,
                page_styles
                    .base
                    .cascaded_with(&page_styles.right)
                    .cascaded_with(&page_styles.blank),
                PageSelector::BlankRight,
                font_registry.clone(),
                root_text.clone(),
            ));
            for named in &page_styles.named {
                let base = page_styles.base.cascaded_with(&named.base);
                templates.push(build_css_page_template(
                    format!("Named:{}:First", named.name),
                    page_size,
                    base_margins,
                    page_styles
                        .base
                        .cascaded_with(&page_styles.right)
                        .cascaded_with(&named.base)
                        .cascaded_with(&named.right)
                        .cascaded_with(&named.first),
                    PageSelector::NamedFirst(named.id),
                    font_registry.clone(),
                    root_text.clone(),
                ));
                templates.push(build_css_page_template(
                    format!("Named:{}:Left", named.name),
                    page_size,
                    base_margins,
                    page_styles
                        .base
                        .cascaded_with(&page_styles.left)
                        .cascaded_with(&named.base)
                        .cascaded_with(&named.left),
                    PageSelector::NamedLeft(named.id),
                    font_registry.clone(),
                    root_text.clone(),
                ));
                templates.push(build_css_page_template(
                    format!("Named:{}:Right", named.name),
                    page_size,
                    base_margins,
                    page_styles
                        .base
                        .cascaded_with(&page_styles.right)
                        .cascaded_with(&named.base)
                        .cascaded_with(&named.right),
                    PageSelector::NamedRight(named.id),
                    font_registry.clone(),
                    root_text.clone(),
                ));
                templates.push(build_css_page_template(
                    format!("Named:{}:BlankLeft", named.name),
                    page_size,
                    base_margins,
                    page_styles
                        .base
                        .cascaded_with(&page_styles.left)
                        .cascaded_with(&page_styles.blank)
                        .cascaded_with(&named.base)
                        .cascaded_with(&named.left)
                        .cascaded_with(&named.blank),
                    PageSelector::NamedBlankLeft(named.id),
                    font_registry.clone(),
                    root_text.clone(),
                ));
                templates.push(build_css_page_template(
                    format!("Named:{}:BlankRight", named.name),
                    page_size,
                    base_margins,
                    page_styles
                        .base
                        .cascaded_with(&page_styles.right)
                        .cascaded_with(&page_styles.blank)
                        .cascaded_with(&named.base)
                        .cascaded_with(&named.right)
                        .cascaded_with(&named.blank),
                    PageSelector::NamedBlankRight(named.id),
                    font_registry.clone(),
                    root_text.clone(),
                ));
                templates.push(build_css_page_template(
                    format!("Named:{}", named.name),
                    page_size,
                    base_margins,
                    base,
                    PageSelector::NamedAny(named.id),
                    font_registry.clone(),
                    root_text.clone(),
                ));
            }
            templates.push(build_css_page_template(
                "Page",
                page_size,
                base_margins,
                page_styles.base.clone(),
                PageSelector::Any,
                font_registry.clone(),
                root_text.clone(),
            ));
        } else {
            let mut template = PageTemplate::new("Page1", page_size).with_frame(frame_rect);
            if let Some(background) = page_styles.base.background {
                template = with_page_background(template, page_size, Pt::ZERO, background);
            }
            templates.push(template);
        }
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
        let mut template = PageTemplate::new(format!("Page{page_number}"), page_size)
            .with_frame(rect)
            .with_page_presentation(page_styles.base.page_presentation());
        let presentation = page_styles.base.page_presentation();
        if let Some(background) = page_styles.base.background {
            template =
                with_page_background(template, page_size, presentation.media_extent(), background);
        }
        template = with_page_print_marks(template, page_size, presentation);
        templates.push(template);
    }
    templates
}

fn build_css_page_template(
    name: impl Into<String>,
    page_size: Size,
    base_margins: Margins,
    setup: style::CssPageSetup,
    selector: PageSelector,
    font_registry: Arc<FontRegistry>,
    root_text: PageRootTextContext,
) -> PageTemplate {
    let page_size = setup.size.unwrap_or(page_size).quantized();
    let margins = setup
        .resolve_margins(base_margins, page_size)
        .unwrap_or(base_margins)
        .quantized();
    let rect = Rect {
        x: margins.left,
        y: margins.top,
        width: (page_size.width - margins.left - margins.right).max(Pt::ZERO),
        height: (page_size.height - margins.top - margins.bottom).max(Pt::ZERO),
    }
    .quantized();
    let mut template = PageTemplate::new(name, page_size)
        .with_frame(rect)
        .with_page_selector(selector)
        .with_page_counter(setup.page_counter_reset, setup.page_counter_increment)
        .with_page_presentation(setup.page_presentation());
    let presentation = setup.page_presentation();
    if let Some(background) = setup.background {
        template =
            with_page_background(template, page_size, presentation.media_extent(), background);
    }
    template = with_page_print_marks(template, page_size, presentation);
    template = with_page_margin_boxes(
        template,
        page_size,
        margins,
        setup,
        font_registry,
        root_text,
    );
    template
}

fn page_margin_box_rect(
    page_size: Size,
    margins: Margins,
    kind: style::CssPageMarginBoxKind,
    width_override: Option<Pt>,
) -> (Rect, flowable::TextAlign) {
    use flowable::TextAlign;
    use style::CssPageMarginBoxKind::*;

    let content_width = (page_size.width - margins.left - margins.right).max(Pt::ZERO);
    let content_height = (page_size.height - margins.top - margins.bottom).max(Pt::ZERO);
    let half_width = content_width / 2;
    let half_height = content_height / 2;
    let (mut rect, align) = match kind {
        TopLeftCorner => (
            Rect {
                x: Pt::ZERO,
                y: Pt::ZERO,
                width: margins.left,
                height: margins.top,
            },
            TextAlign::Right,
        ),
        TopLeft => (
            Rect {
                x: margins.left,
                y: Pt::ZERO,
                width: half_width,
                height: margins.top,
            },
            TextAlign::Left,
        ),
        TopCenter => (
            Rect {
                x: margins.left,
                y: Pt::ZERO,
                width: content_width,
                height: margins.top,
            },
            TextAlign::Center,
        ),
        TopRight => (
            Rect {
                x: margins.left + half_width,
                y: Pt::ZERO,
                width: half_width,
                height: margins.top,
            },
            TextAlign::Right,
        ),
        TopRightCorner => (
            Rect {
                x: page_size.width - margins.right,
                y: Pt::ZERO,
                width: margins.right,
                height: margins.top,
            },
            TextAlign::Left,
        ),
        BottomLeftCorner => (
            Rect {
                x: Pt::ZERO,
                y: page_size.height - margins.bottom,
                width: margins.left,
                height: margins.bottom,
            },
            TextAlign::Right,
        ),
        BottomLeft => (
            Rect {
                x: margins.left,
                y: page_size.height - margins.bottom,
                width: half_width,
                height: margins.bottom,
            },
            TextAlign::Left,
        ),
        BottomCenter => (
            Rect {
                x: margins.left,
                y: page_size.height - margins.bottom,
                width: content_width,
                height: margins.bottom,
            },
            TextAlign::Center,
        ),
        BottomRight => (
            Rect {
                x: margins.left + half_width,
                y: page_size.height - margins.bottom,
                width: half_width,
                height: margins.bottom,
            },
            TextAlign::Right,
        ),
        BottomRightCorner => (
            Rect {
                x: page_size.width - margins.right,
                y: page_size.height - margins.bottom,
                width: margins.right,
                height: margins.bottom,
            },
            TextAlign::Left,
        ),
        LeftTop => (
            Rect {
                x: Pt::ZERO,
                y: margins.top,
                width: margins.left,
                height: half_height,
            },
            TextAlign::Center,
        ),
        LeftMiddle => (
            Rect {
                x: Pt::ZERO,
                y: margins.top,
                width: margins.left,
                height: content_height,
            },
            TextAlign::Center,
        ),
        LeftBottom => (
            Rect {
                x: Pt::ZERO,
                y: margins.top + half_height,
                width: margins.left,
                height: half_height,
            },
            TextAlign::Center,
        ),
        RightTop => (
            Rect {
                x: page_size.width - margins.right,
                y: margins.top,
                width: margins.right,
                height: half_height,
            },
            TextAlign::Center,
        ),
        RightMiddle => (
            Rect {
                x: page_size.width - margins.right,
                y: margins.top,
                width: margins.right,
                height: content_height,
            },
            TextAlign::Center,
        ),
        RightBottom => (
            Rect {
                x: page_size.width - margins.right,
                y: margins.top + half_height,
                width: margins.right,
                height: half_height,
            },
            TextAlign::Center,
        ),
    };
    if let Some(width) = width_override {
        let width = width.max(Pt::ZERO).min(page_size.width);
        match kind {
            TopLeft | BottomLeft => rect.width = width,
            TopCenter | BottomCenter => {
                rect.x = (page_size.width - width) / 2;
                rect.width = width;
            }
            TopRight | BottomRight => {
                rect.x = page_size.width - width;
                rect.width = width;
            }
            _ => {}
        }
    }
    (rect.quantized(), align)
}

fn with_page_margin_boxes(
    mut template: PageTemplate,
    page_size: Size,
    margins: Margins,
    setup: style::CssPageSetup,
    font_registry: Arc<FontRegistry>,
    root_text: PageRootTextContext,
) -> PageTemplate {
    for margin_box in &setup.margin_boxes {
        let Some(kind) = margin_box.kind else {
            continue;
        };
        let Some(parts) = margin_box.content.clone() else {
            continue;
        };
        if parts.is_empty() {
            continue;
        }
        let (rect, align) = page_margin_box_rect(page_size, margins, kind, margin_box.width);
        if rect.width <= Pt::ZERO || rect.height <= Pt::ZERO {
            continue;
        }
        let font_size = margin_box
            .font_size
            .or(setup.font_size)
            .unwrap_or(root_text.style.font_size);
        let line_height_spec = margin_box
            .line_height
            .or(setup.line_height)
            .or(root_text.line_height);
        let (line_height, line_height_is_auto) = match line_height_spec {
            Some(style::CssPageLineHeight::Absolute(value)) => (value, false),
            Some(style::CssPageLineHeight::Number(value)) => (font_size * value, false),
            None => (font_size.mul_ratio(6, 5), true),
        };
        let mut text_style = root_text.style.clone();
        text_style.font_size = font_size;
        text_style.line_height = line_height.max(Pt::ZERO);
        text_style.line_height_is_auto = line_height_is_auto;
        text_style.color = margin_box
            .color
            .or(setup.color)
            .unwrap_or(root_text.style.color);
        if let Some(font_name) = margin_box
            .font_name
            .as_deref()
            .or(setup.font_name.as_deref())
        {
            text_style.font_name = Arc::<str>::from(font_name);
            text_style.font_fallbacks.clear();
            text_style.font_unicode_ranges.clear();
            text_style.font_unicode_ranges.push(None);
            text_style.font_face_satisfies_weight = false;
            text_style.font_face_satisfies_style = false;
        }
        if let Some(font_weight) = margin_box.font_weight.or(setup.font_weight) {
            text_style.font_weight = font_weight;
            text_style.font_face_satisfies_weight = false;
        }
        text_style.css_pixel_snap_metrics = true;

        let background = margin_box.background;
        let auto_width = margin_box.width.is_none();
        let registry = font_registry.clone();
        template = template.append_on_page_finalize(move |canvas, context| {
            let mut content_rect = rect;
            if auto_width {
                for part in &parts {
                    let style::CssPageMarginContentPart::RunningElement { name, position } = part
                    else {
                        continue;
                    };
                    let Some(element) = context.running_element(name, position) else {
                        continue;
                    };
                    if element.width > content_rect.width {
                        content_rect =
                            page_margin_box_rect(page_size, margins, kind, Some(element.width)).0;
                    }
                }
            }
            canvas.save_state();
            if let Some((color, alpha)) = background {
                if alpha > 0.0 {
                    canvas.save_state();
                    canvas.set_fill_color(color);
                    if alpha < 1.0 {
                        canvas.set_opacity(alpha, alpha);
                    }
                    canvas.draw_rect(
                        content_rect.x,
                        content_rect.y,
                        content_rect.width,
                        content_rect.height,
                    );
                    canvas.restore_state();
                }
            }
            let mut text = String::new();
            for part in &parts {
                match part {
                    style::CssPageMarginContentPart::Text(value) => text.push_str(value),
                    style::CssPageMarginContentPart::PageCounter => {
                        text.push_str(&context.page_counter.to_string());
                    }
                    style::CssPageMarginContentPart::PagesCounter => {
                        text.push_str(&context.total_pages.to_string());
                    }
                    style::CssPageMarginContentPart::NamedString { name, position } => {
                        if let Some(value) = context.named_string(name, position) {
                            text.push_str(value);
                        }
                    }
                    style::CssPageMarginContentPart::RunningElement { name, position } => {
                        if let Some(element) = context.running_element(name, position) {
                            let x = match align {
                                flowable::TextAlign::Left | flowable::TextAlign::Justify => {
                                    content_rect.x
                                }
                                flowable::TextAlign::Center => {
                                    content_rect.x + (content_rect.width - element.width) / 2
                                }
                                flowable::TextAlign::Right => {
                                    content_rect.x + content_rect.width - element.width
                                }
                            };
                            let y = content_rect.y + (content_rect.height - element.height) / 2;
                            canvas.draw_form(
                                x,
                                y,
                                element.width,
                                element.height,
                                element.resource_id.clone(),
                            );
                        }
                    }
                }
            }
            if !text.is_empty() {
                let paragraph = Paragraph::new(text)
                    .with_style(text_style.clone())
                    .with_align(align)
                    .with_font_registry(Some(registry.clone()));
                let text_size = paragraph.wrap(content_rect.width, content_rect.height);
                // CSS page-margin boxes use `vertical-align: middle` by
                // default. Oversized line boxes therefore overflow equally
                // above and below the margin area instead of being pinned to
                // its top edge.
                let y = content_rect.y + (content_rect.height - text_size.height) / 2;
                paragraph.draw(
                    canvas,
                    content_rect.x,
                    y,
                    content_rect.width,
                    content_rect.height,
                );
            }
            canvas.restore_state();
        });
    }
    template
}

fn with_page_background(
    template: PageTemplate,
    page_size: Size,
    media_extent: Pt,
    (color, alpha): (Color, f32),
) -> PageTemplate {
    template.set_on_page(move |canvas, _| {
        if alpha <= 0.0 {
            return;
        }
        canvas.save_state();
        canvas.set_fill_color(color);
        if alpha < 1.0 {
            canvas.set_opacity(alpha, alpha);
        }
        canvas.draw_rect(
            -media_extent,
            -media_extent,
            page_size.width + media_extent + media_extent,
            page_size.height + media_extent + media_extent,
        );
        canvas.restore_state();
    })
}

fn with_page_print_marks(
    template: PageTemplate,
    page_size: Size,
    presentation: types::PagePresentation,
) -> PageTemplate {
    if !presentation.marks.crop && !presentation.marks.cross {
        return template;
    }
    let extent = presentation.media_extent();
    if extent <= Pt::ZERO {
        return template;
    }
    template.append_on_page(move |canvas, _| {
        canvas.save_state();
        canvas.begin_artifact(Some("Pagination".to_string()));
        canvas.set_stroke_color(Color::BLACK);
        canvas.set_line_cap(0);
        canvas.set_line_join(0);

        if presentation.marks.crop {
            let half = extent / 2;
            canvas.set_line_width(Pt::from_f32(0.75));

            // Horizontal crop marks at the top and bottom trim edges.
            for y in [Pt::ZERO, page_size.height] {
                canvas.move_to(-extent, y);
                canvas.line_to(-half, y);
                canvas.move_to(page_size.width + half, y);
                canvas.line_to(page_size.width + extent, y);
            }
            // Vertical crop marks at the left and right trim edges.
            for x in [Pt::ZERO, page_size.width] {
                canvas.move_to(x, -extent);
                canvas.line_to(x, -half);
                canvas.move_to(x, page_size.height + half);
                canvas.line_to(x, page_size.height + extent);
            }
            canvas.stroke();
        }

        if presentation.marks.cross {
            let center_offset = extent.mul_ratio(3, 4);
            let half_span = extent / 4;
            let radius = extent / 8;
            let circle_width = extent / 32;
            let cross_width = extent / 16;
            let kappa = Pt::from_f32(radius.to_f32() * 0.552_284_8);
            let centers = [
                (page_size.width / 2, -center_offset, true),
                (page_size.width / 2, page_size.height + center_offset, true),
                (-center_offset, page_size.height / 2, false),
                (page_size.width + center_offset, page_size.height / 2, false),
            ];
            for (x, y, vertical_lane) in centers {
                canvas.set_line_width(circle_width);
                canvas.move_to(x + radius, y);
                canvas.curve_to(x + radius, y + kappa, x + kappa, y + radius, x, y + radius);
                canvas.curve_to(x - kappa, y + radius, x - radius, y + kappa, x - radius, y);
                canvas.curve_to(x - radius, y - kappa, x - kappa, y - radius, x, y - radius);
                canvas.curve_to(x + kappa, y - radius, x + radius, y - kappa, x + radius, y);
                canvas.close_path();
                canvas.stroke();

                canvas.set_line_width(cross_width);
                if vertical_lane {
                    canvas.move_to(x - half_span, y);
                    canvas.line_to(x + half_span, y);
                    if y < Pt::ZERO {
                        canvas.move_to(x, -extent);
                        canvas.line_to(x, -extent / 2);
                    } else {
                        canvas.move_to(x, page_size.height + extent / 2);
                        canvas.line_to(x, page_size.height + extent);
                    }
                } else {
                    canvas.move_to(x, y - half_span);
                    canvas.line_to(x, y + half_span);
                    if x < Pt::ZERO {
                        canvas.move_to(-extent, y);
                        canvas.line_to(-extent / 2, y);
                    } else {
                        canvas.move_to(page_size.width + extent / 2, y);
                        canvas.line_to(page_size.width + extent, y);
                    }
                }
                canvas.stroke();
            }
        }
        canvas.end_marked_content();
        canvas.restore_state();
    })
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
    use crate::flowable::{
        BorderCollapseMode, BorderSpec, TableCell, TableLayoutMode, TextAlign, VerticalAlign,
    };
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

    fn page_contains_fill_color(page: &Page, color: Color) -> bool {
        page.commands
            .iter()
            .any(|command| matches!(command, Command::SetFillColor(value) if *value == color))
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

    #[test]
    fn html_page_shrink_uses_scrollable_right_edge() {
        let page_width = Pt::from_f32(252.0);
        let overflow_right = Pt::from_f32(255.0);
        let mut doc = Document {
            page_size: Size {
                width: page_width,
                height: Pt::from_f32(72.0),
            },
            pages: vec![Page {
                commands: vec![Command::Meta {
                    key: canvas::META_HTML_SCROLLABLE_RIGHT_KEY.to_string(),
                    value: overflow_right.to_milli_i64().to_string(),
                }],
            }],
        };

        apply_html_page_shrink_to_fit(&mut doc);

        let (scale_x, scale_y) = doc.pages[0]
            .commands
            .iter()
            .find_map(|command| match command {
                Command::Scale(scale_x, scale_y) => Some((*scale_x, *scale_y)),
                _ => None,
            })
            .expect("expected a page shrink transform");
        let expected = page_width.to_f32() / overflow_right.to_f32();
        assert!((scale_x - expected).abs() < 1.0e-6);
        assert!((scale_y - expected).abs() < 1.0e-6);
    }

    #[test]
    fn html_page_shrink_anchors_to_page_area_and_preserves_template_paint() {
        let page_size = Size {
            width: Pt::from_f32(144.0),
            height: Pt::from_f32(150.0),
        };
        let page_area = Rect {
            x: Pt::from_f32(24.0),
            y: Pt::from_f32(12.0),
            width: Pt::from_f32(114.0),
            height: Pt::from_f32(132.0),
        };
        let overflow_right = Pt::from_f32(143.0);
        let template_color = Color::rgb(1.0, 0.0, 0.0);
        let mut doc = Document {
            page_size,
            pages: vec![Page {
                commands: vec![
                    Command::SetFillColor(template_color),
                    Command::DrawRect {
                        x: Pt::ZERO,
                        y: Pt::ZERO,
                        width: page_size.width,
                        height: page_size.height,
                    },
                    Command::Meta {
                        key: META_PAGE_TEMPLATE_KEY.to_string(),
                        value: "Right".to_string(),
                    },
                    Command::Meta {
                        key: canvas::META_HTML_PAGE_AREA_KEY.to_string(),
                        value: format!(
                            "{},{},{},{}",
                            page_area.x.to_milli_i64(),
                            page_area.y.to_milli_i64(),
                            page_area.width.to_milli_i64(),
                            page_area.height.to_milli_i64(),
                        ),
                    },
                    Command::Meta {
                        key: canvas::META_HTML_SCROLLABLE_RIGHT_KEY.to_string(),
                        value: overflow_right.to_milli_i64().to_string(),
                    },
                ],
            }],
        };

        apply_html_page_shrink_to_fit(&mut doc);

        assert!(matches!(
            doc.pages[0].commands.first(),
            Some(Command::SetFillColor(color)) if *color == template_color
        ));
        assert!(matches!(
            doc.pages[0].commands.get(1),
            Some(Command::DrawRect { x, y, width, height })
                if *x == Pt::ZERO
                    && *y == Pt::ZERO
                    && *width == page_size.width
                    && *height == page_size.height
        ));
        let block_start_guard = Pt::from_milli_i64(1);
        assert!(doc.pages[0].commands.iter().any(|command| matches!(
            command,
            Command::ClipRect { x, y, width, height }
                if *x == page_area.x
                    && *y == page_area.y + block_start_guard
                    && *width == page_area.width
                    && *height == page_area.height - block_start_guard
        )));
        assert!(doc.pages[0].commands.iter().any(|command| matches!(
            command,
            Command::CssTransformOrigin { x, y, inverse: false }
                if *x == page_area.x && *y == page_area.y
        )));
        let scale = doc.pages[0]
            .commands
            .iter()
            .find_map(|command| match command {
                Command::Scale(scale_x, scale_y) => Some((*scale_x, *scale_y)),
                _ => None,
            })
            .expect("expected page-area shrink transform");
        let expected = page_area.width.to_f32() / (overflow_right - page_area.x).to_f32();
        assert!((scale.0 - expected).abs() < 1.0e-6);
        assert!((scale.1 - expected).abs() < 1.0e-6);
    }

    #[test]
    fn html_page_shrink_extends_only_propagated_canvas_background_block_end() {
        let page_size = Size {
            width: Pt::from_f32(144.0),
            height: Pt::from_f32(150.0),
        };
        let page_area = Rect {
            x: Pt::from_f32(24.0),
            y: Pt::from_f32(12.0),
            width: Pt::from_f32(114.0),
            height: Pt::from_f32(132.0),
        };
        let canvas_height = Pt::from_f32(132.0);
        let content_height = Pt::from_f32(9.0);
        let mut doc = Document {
            page_size,
            pages: vec![Page {
                commands: vec![
                    Command::Meta {
                        key: META_PAGE_TEMPLATE_KEY.to_string(),
                        value: "Right".to_string(),
                    },
                    Command::Meta {
                        key: canvas::META_HTML_PAGE_AREA_KEY.to_string(),
                        value: format!(
                            "{},{},{},{}",
                            page_area.x.to_milli_i64(),
                            page_area.y.to_milli_i64(),
                            page_area.width.to_milli_i64(),
                            page_area.height.to_milli_i64(),
                        ),
                    },
                    Command::Meta {
                        key: canvas::META_HTML_SCROLLABLE_RIGHT_KEY.to_string(),
                        value: Pt::from_f32(143.0).to_milli_i64().to_string(),
                    },
                    Command::Meta {
                        key: canvas::META_HTML_CANVAS_BACKGROUND_KEY.to_string(),
                        value: "begin".to_string(),
                    },
                    Command::DrawRect {
                        x: Pt::ZERO,
                        y: Pt::ZERO,
                        width: page_area.width,
                        height: canvas_height,
                    },
                    Command::Meta {
                        key: canvas::META_HTML_CANVAS_BACKGROUND_KEY.to_string(),
                        value: "end".to_string(),
                    },
                    Command::DrawRect {
                        x: Pt::ZERO,
                        y: Pt::ZERO,
                        width: Pt::from_f32(7.0),
                        height: content_height,
                    },
                ],
            }],
        };

        apply_html_page_shrink_to_fit(&mut doc);

        assert!(doc.pages[0].commands.iter().any(|command| matches!(
            command,
            Command::DrawRect { width, height, .. }
                if *width == page_area.width
                    && *height == canvas_height + Pt::from_f32(0.25)
        )));
        assert!(doc.pages[0].commands.iter().any(|command| matches!(
            command,
            Command::DrawRect { width, height, .. }
                if *width == Pt::from_f32(7.0) && *height == content_height
        )));
    }

    #[test]
    fn html_page_shrink_uses_one_document_scale_across_asymmetric_page_areas() {
        let first_area = Rect {
            x: Pt::from_f32(6.0),
            y: Pt::from_f32(6.0),
            width: Pt::from_f32(114.0),
            height: Pt::from_f32(126.0),
        };
        let second_area = Rect {
            x: Pt::from_f32(24.0),
            y: Pt::from_f32(12.0),
            width: Pt::from_f32(114.0),
            height: Pt::from_f32(132.0),
        };
        let page = |area: Rect, scrollable_right: Pt| Page {
            commands: vec![
                Command::Meta {
                    key: META_PAGE_TEMPLATE_KEY.to_string(),
                    value: "Page".to_string(),
                },
                Command::Meta {
                    key: canvas::META_HTML_PAGE_AREA_KEY.to_string(),
                    value: format!(
                        "{},{},{},{}",
                        area.x.to_milli_i64(),
                        area.y.to_milli_i64(),
                        area.width.to_milli_i64(),
                        area.height.to_milli_i64(),
                    ),
                },
                Command::Meta {
                    key: canvas::META_HTML_SCROLLABLE_RIGHT_KEY.to_string(),
                    value: scrollable_right.to_milli_i64().to_string(),
                },
            ],
        };
        let overflow_right = Pt::from_f32(143.0);
        let mut doc = Document {
            page_size: Size {
                width: Pt::from_f32(144.0),
                height: Pt::from_f32(150.0),
            },
            pages: vec![
                page(first_area, first_area.x + first_area.width),
                page(second_area, overflow_right),
            ],
        };

        apply_html_page_shrink_to_fit(&mut doc);

        let expected = second_area.width.to_f32() / (overflow_right - second_area.x).to_f32();
        for page in &doc.pages {
            let scale = page.commands.iter().find_map(|command| match command {
                Command::Scale(scale_x, scale_y) => Some((*scale_x, *scale_y)),
                _ => None,
            });
            assert_eq!(scale, Some((expected, expected)));
        }
        assert!(doc.pages[0].commands.iter().any(|command| matches!(
            command,
            Command::CssTransformOrigin { x, y, inverse: false }
                if *x == first_area.x && *y == first_area.y
        )));
        assert!(doc.pages[1].commands.iter().any(|command| matches!(
            command,
            Command::CssTransformOrigin { x, y, inverse: false }
                if *x == second_area.x && *y == second_area.y
        )));
    }

    #[test]
    fn html_transformed_block_start_overflow_replays_on_previous_fragmentainer() {
        let first_area = Rect {
            x: Pt::from_f32(6.0),
            y: Pt::from_f32(6.0),
            width: Pt::from_f32(114.0),
            height: Pt::from_f32(126.0),
        };
        let second_area = Rect {
            x: Pt::from_f32(24.0),
            y: Pt::from_f32(12.0),
            width: Pt::from_f32(114.0),
            height: Pt::from_f32(132.0),
        };
        let area_meta = |area: Rect| Command::Meta {
            key: canvas::META_HTML_PAGE_AREA_KEY.to_string(),
            value: format!(
                "{},{},{},{}",
                area.x.to_milli_i64(),
                area.y.to_milli_i64(),
                area.width.to_milli_i64(),
                area.height.to_milli_i64(),
            ),
        };
        let mut doc = Document {
            page_size: Size {
                width: Pt::from_f32(144.0),
                height: Pt::from_f32(150.0),
            },
            pages: vec![
                Page {
                    commands: vec![
                        Command::Meta {
                            key: META_PAGE_TEMPLATE_KEY.to_string(),
                            value: "Left".to_string(),
                        },
                        area_meta(first_area),
                    ],
                },
                Page {
                    commands: vec![
                        Command::Meta {
                            key: META_PAGE_TEMPLATE_KEY.to_string(),
                            value: "Right".to_string(),
                        },
                        area_meta(second_area),
                        Command::Meta {
                            key: canvas::META_HTML_SCROLLABLE_TOP_KEY.to_string(),
                            value: (second_area.y - Pt::from_f32(3.0))
                                .to_milli_i64()
                                .to_string(),
                        },
                        Command::BeginTag {
                            role: "P".to_string(),
                            mcid: Some(0),
                            alt: None,
                            scope: None,
                            table_id: None,
                            col_index: None,
                            group_only: false,
                        },
                        Command::DrawRect {
                            x: second_area.x,
                            y: second_area.y - Pt::from_f32(3.0),
                            width: Pt::from_f32(12.0),
                            height: Pt::from_f32(6.0),
                        },
                        Command::EndTag,
                    ],
                },
            ],
        };

        apply_html_page_shrink_to_fit(&mut doc);

        assert!(
            !doc.pages
                .iter()
                .flat_map(|page| &page.commands)
                .any(|command| matches!(command, Command::Scale(..)))
        );
        let artifact_start = doc.pages[0]
            .commands
            .iter()
            .position(|command| matches!(command, Command::BeginArtifact { subtype: None }))
            .expect("expected a visual overflow artifact");
        let artifact_end = doc.pages[0].commands[artifact_start + 1..]
            .iter()
            .position(|command| matches!(command, Command::EndMarkedContent))
            .map(|index| artifact_start + 1 + index)
            .expect("expected the visual overflow artifact terminator");
        let replay = &doc.pages[0].commands[artifact_start + 1..artifact_end];
        assert!(
            replay
                .iter()
                .any(|command| matches!(command, Command::DrawRect { .. }))
        );
        assert!(!replay.iter().any(|command| matches!(
            command,
            Command::Meta { .. }
                | Command::BeginTag { .. }
                | Command::BeginTagActualText { .. }
                | Command::EndTag
        )));
        let expected_x = first_area.x - second_area.x;
        let expected_y = first_area.y + first_area.height - second_area.y + Pt::from_f32(0.25);
        assert!(doc.pages[0].commands.iter().any(|command| matches!(
            command,
            Command::ConcatMatrix { a, b, c, d, e, f }
                if (*a - 1.0).abs() < f32::EPSILON
                    && b.abs() < f32::EPSILON
                    && c.abs() < f32::EPSILON
                    && (*d - 1.0).abs() < f32::EPSILON
                    && *e == expected_x
                    && *f == expected_y
        )));
        assert!(
            doc.pages[0]
                .commands
                .iter()
                .any(|command| matches!(command, Command::BeginArtifact { subtype: None }))
        );
    }

    #[test]
    fn html_transformed_block_end_overflow_replays_before_matching_continuation() {
        let page_size = Size {
            width: Pt::from_f32(144.0),
            height: Pt::from_f32(150.0),
        };
        let page_area = Rect {
            x: Pt::ZERO,
            y: Pt::ZERO,
            width: page_size.width,
            height: page_size.height,
        };
        let owner = "html:nth-of-type(1) > body:nth-of-type(1) > div:nth-of-type(1)";
        let area_meta = || Command::Meta {
            key: canvas::META_HTML_PAGE_AREA_KEY.to_string(),
            value: format!(
                "{},{},{},{}",
                page_area.x.to_milli_i64(),
                page_area.y.to_milli_i64(),
                page_area.width.to_milli_i64(),
                page_area.height.to_milli_i64(),
            ),
        };
        let scope_begin = || Command::Meta {
            key: canvas::META_DIAGNOSTIC_SCOPE_BEGIN_KEY.to_string(),
            value: "flowable".to_string(),
        };
        let scope_end = || Command::Meta {
            key: canvas::META_DIAGNOSTIC_SCOPE_END_KEY.to_string(),
            value: "flowable".to_string(),
        };
        let owner_meta = || Command::Meta {
            key: "fb.owner.dom_path".to_string(),
            value: owner.to_string(),
        };
        let source_rect = Rect {
            x: Pt::from_f32(11.0),
            y: Pt::from_f32(120.0),
            width: Pt::from_f32(80.0),
            height: Pt::from_f32(36.0),
        };
        let continuation_color = Color::rgb(0.2, 0.4, 0.6);
        let mut doc = Document {
            page_size,
            pages: vec![
                Page {
                    commands: vec![
                        Command::Meta {
                            key: META_PAGE_TEMPLATE_KEY.to_string(),
                            value: "Source".to_string(),
                        },
                        area_meta(),
                        scope_begin(),
                        owner_meta(),
                        Command::Meta {
                            key: canvas::META_HTML_SCROLLABLE_BOTTOM_KEY.to_string(),
                            value: (page_area.y + page_area.height + Pt::from_f32(6.0))
                                .to_milli_i64()
                                .to_string(),
                        },
                        Command::BeginTag {
                            role: "Div".to_string(),
                            mcid: Some(0),
                            alt: None,
                            scope: None,
                            table_id: None,
                            col_index: None,
                            group_only: false,
                        },
                        Command::DrawRect {
                            x: source_rect.x,
                            y: source_rect.y,
                            width: source_rect.width,
                            height: source_rect.height,
                        },
                        Command::EndTag,
                        scope_end(),
                    ],
                },
                Page {
                    commands: vec![
                        Command::Meta {
                            key: META_PAGE_TEMPLATE_KEY.to_string(),
                            value: "Destination".to_string(),
                        },
                        area_meta(),
                        Command::SetFillColor(continuation_color),
                        scope_begin(),
                        owner_meta(),
                        Command::DrawRect {
                            x: Pt::from_f32(11.0),
                            y: Pt::ZERO,
                            width: Pt::from_f32(80.0),
                            height: Pt::from_f32(24.0),
                        },
                        scope_end(),
                    ],
                },
            ],
        };

        apply_html_page_shrink_to_fit(&mut doc);

        let commands = &doc.pages[1].commands;
        let ancestor_paint = commands
            .iter()
            .position(|command| {
                matches!(command, Command::SetFillColor(color) if *color == continuation_color)
            })
            .expect("expected continuation ancestor paint");
        let artifact_start = commands
            .iter()
            .position(|command| matches!(command, Command::BeginArtifact { subtype: None }))
            .expect("expected a carried block-end artifact");
        let artifact_end = commands[artifact_start + 1..]
            .iter()
            .position(|command| matches!(command, Command::EndMarkedContent))
            .map(|index| artifact_start + 1 + index)
            .expect("expected the carried block-end artifact terminator");
        let continuation_scope = commands[artifact_end + 1..]
            .iter()
            .position(|command| {
                matches!(
                    command,
                    Command::Meta { key, .. } if key == canvas::META_DIAGNOSTIC_SCOPE_BEGIN_KEY
                )
            })
            .map(|index| artifact_end + 1 + index)
            .expect("expected the matching continuation scope");
        assert!(ancestor_paint < artifact_start);
        assert!(artifact_end < continuation_scope);
        let replay = &commands[artifact_start + 1..artifact_end];
        assert!(replay.iter().any(|command| matches!(
            command,
            Command::ConcatMatrix { a, b, c, d, e, f }
                if (*a - 1.0).abs() < f32::EPSILON
                    && b.abs() < f32::EPSILON
                    && c.abs() < f32::EPSILON
                    && (*d - 1.0).abs() < f32::EPSILON
                    && *e == Pt::ZERO
                    && *f == -page_area.height
        )));
        assert!(replay.iter().any(|command| matches!(
            command,
            Command::DrawRect { x, y, width, height }
                if *x == source_rect.x
                    && *y == source_rect.y
                    && *width == source_rect.width
                    && *height == source_rect.height
        )));
        assert!(!replay.iter().any(|command| matches!(
            command,
            Command::Meta { .. }
                | Command::BeginTag { .. }
                | Command::BeginTagActualText { .. }
                | Command::EndTag
        )));
    }

    #[test]
    fn html_page_shrink_uses_each_pages_named_physical_width() {
        let default_width = Pt::from_f32(252.0);
        let named_width = Pt::from_f32(841.89);
        let mut doc = Document {
            page_size: Size {
                width: default_width,
                height: Pt::from_f32(228.0),
            },
            pages: vec![Page {
                commands: vec![
                    Command::Meta {
                        key: canvas::META_PAGE_SIZE_KEY.to_string(),
                        value: format!("{},1190551", named_width.to_milli_i64()),
                    },
                    Command::Meta {
                        key: canvas::META_HTML_SCROLLABLE_RIGHT_KEY.to_string(),
                        value: Pt::from_f32(817.89).to_milli_i64().to_string(),
                    },
                ],
            }],
        };

        apply_html_page_shrink_to_fit(&mut doc);

        assert!(
            !doc.pages[0]
                .commands
                .iter()
                .any(|command| matches!(command, Command::Scale(..))),
            "content inside the named page width must not be scaled to the document default"
        );
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
    fn tagged_pdf_preserves_html_th_scope_row() {
        let engine = FullBleed::builder()
            .pdf_profile(PdfProfile::Tagged)
            .build()
            .expect("tagged engine");
        let html = r#"
        <!doctype html>
        <html>
          <body>
            <table>
              <tbody>
                <tr>
                  <th scope="row">Name</th>
                  <td>Jane</td>
                </tr>
              </tbody>
            </table>
          </body>
        </html>
        "#;
        let css = "table, th, td { border: 1px solid #000; }";
        let pdf = engine
            .render_to_buffer(html, css)
            .expect("render tagged pdf");
        assert!(
            count_token(&pdf, b"/Scope /Row") >= 1,
            "expected tagged PDF to contain TH scope derived from HTML scope=row"
        );
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
    fn repeated_css_reuses_compiled_render_context() {
        let engine = FullBleed::builder().build().expect("engine");
        let css = "@page { size: letter; margin: 0.5in } body { color: #123456 }";
        let first = engine.build_render_context(css, Some(0));
        let second = engine.build_render_context(css, Some(1));

        assert!(Arc::ptr_eq(&first.resolver, &second.resolver));
        assert!(Arc::ptr_eq(&first.page_templates, &second.page_templates));
        assert_eq!(
            engine
                .render_context_cache
                .lock()
                .expect("render context cache")
                .map
                .len(),
            1
        );
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
        let wm = build_watermark_document(
            &base, &spec, &resolver, None, None, None, None, false, false,
        );

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
        let wm = build_watermark_document(
            &base, &spec, &resolver, None, None, None, None, false, false,
        );

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
            &spec, page_size, 1, 1, None, &resolver, None, None, None, false, false,
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
            &spec, page_size, 1, 1, None, &resolver, None, None, None, false, false,
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
    fn paged_footnotes_compile_calls_counters_and_bottom_area() {
        let html = r#"<!doctype html><html><body>
            <p>One<span class="fn">first note</span> two<span class="fn">second note</span>.</p>
        </body></html>"#;
        let css = r#"
            @page { size: 200px 152px; margin: 10px; }
            html { font-family: Helvetica; line-height: 1.5; font-size: 12px; }
            * { margin: 0; box-sizing: border-box; }
            body { counter-reset: footnote 4; }
            p { background: #e0f2fe; }
            .fn { float: footnote; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine.render_to_document(html, css).expect("render");
        assert_eq!(document.pages.len(), 1);
        let page = &document.pages[0];
        assert!(page_contains_text(page, "One"));
        assert!(page_contains_text(page, "5"));
        assert!(page_contains_text(page, "6"));
        assert!(page_contains_text(page, "first note"));
        assert!(page_contains_text(page, "second note"));

        let body_y = page
            .commands
            .iter()
            .find_map(|command| match command {
                Command::DrawString { text, y, .. } if text.contains("One") => Some(*y),
                _ => None,
            })
            .expect("body text");
        let note_y = page
            .commands
            .iter()
            .find_map(|command| match command {
                Command::DrawString { text, y, .. } if text.contains("first note") => Some(*y),
                _ => None,
            })
            .expect("footnote text");
        assert!(note_y > body_y + Pt::from_f32(40.0));

        let control = engine
            .render_to_document(
                r#"<!doctype html><html><body><p>One two.</p></body></html>"#,
                css,
            )
            .expect("control render");
        let control_body_y = control.pages[0]
            .commands
            .iter()
            .find_map(|command| match command {
                Command::DrawString { text, y, .. } if text.contains("One") => Some(*y),
                _ => None,
            })
            .expect("control body text");
        assert_eq!(
            body_y, control_body_y,
            "a synthesized footnote call must not enlarge the owning line box"
        );
    }

    #[test]
    fn footnote_policy_block_moves_the_owning_paragraph() {
        let html = r#"<!doctype html><html><body>
            <div class="lead"></div><p>Body<span class="fn">note body</span></p>
        </body></html>"#;
        let css = r#"
            @page { size: 200px 144px; margin: 16px; }
            * { margin: 0; box-sizing: border-box; }
            .lead { height: 64px; background: #1d4ed8; }
            p { height: 40px; font-size: 0; line-height: 20px; background: #facc15; }
            .fn { float: footnote; footnote-policy: block; font-size: 12px; color: #fff; }
            .fn::footnote-call { color: #fde68a; }
            .fn::footnote-marker { content: ""; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine.render_to_document(html, css).expect("render");
        assert_eq!(document.pages.len(), 2);
        assert!(!page_contains_text(&document.pages[0], "Body"));
        assert!(page_contains_text(&document.pages[1], "Body"));
    }

    #[test]
    fn footnote_policy_line_keeps_the_preceding_compiled_line() {
        let html = r#"<!doctype html><html><body>
            <div class="lead"></div>
            <p>First line stays here<br>Second line has notes<span class="fn">first note</span><span class="fn">second note</span></p>
        </body></html>"#;
        let css = r#"
            @page {
                size: 192px 136px;
                margin: 8px;
                @footnote { border-top: 8px solid #111; padding-top: 8px; }
            }
            html { font-family: Helvetica; font-size: 12px; line-height: 20px; }
            * { margin: 0; box-sizing: border-box; }
            .lead { width: 176px; height: 72px; background: #1d4ed8; }
            p { width: 176px; line-height: 20px; background: #fde68a; }
            .fn { float: footnote; footnote-display: inline; footnote-policy: line; color: #d00000; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine.render_to_document(html, css).expect("render");
        assert_eq!(document.pages.len(), 2);
        assert!(page_contains_text(
            &document.pages[0],
            "First line stays here"
        ));
        assert!(!page_contains_text(
            &document.pages[0],
            "Second line has notes"
        ));
        assert!(page_contains_text(
            &document.pages[1],
            "Second line has notes"
        ));
        assert!(page_contains_text(&document.pages[1], "first note"));
        assert!(page_contains_text(&document.pages[1], "second note"));
        assert!(document.pages[1].commands.iter().any(|command| {
            matches!(
                command,
                Command::DrawRect { width, height, .. }
                    if *width == Pt::from_f32(132.0) && *height == Pt::from_f32(6.0)
            )
        }));
    }

    #[test]
    fn footnote_max_height_uses_a_compiled_footnote_only_continuation_page() {
        let html = r#"<!doctype html><html><body>
            <div class="lead"></div><p>Line with note<span class="fn">long note one long note two long note three long note four</span></p>
        </body></html>"#;
        let css = r#"
            @page {
                size: 200px 150px;
                margin: 10px;
                @footnote { max-height: 30px; border-top: 2px solid #111; }
            }
            html { font-family: Helvetica; line-height: 1.5; font-size: 12px; }
            * { margin: 0; box-sizing: border-box; }
            .lead { height: 80px; background: #bfdbfe; }
            .fn { float: footnote; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine.render_to_document(html, css).expect("render");
        assert_eq!(document.pages.len(), 2);
        assert!(page_contains_text(&document.pages[0], "Line with note"));
        assert!(!page_contains_text(&document.pages[0], "long note one"));
        assert!(page_contains_text(&document.pages[1], "long note one"));
        assert!(page_contains_text(&document.pages[1], "four"));
        assert!(document.pages[1].commands.iter().any(|command| {
            matches!(
                command,
                Command::DrawString { text, .. } if text.starts_with("1. long note one")
            )
        }));
        assert!(document.pages[1].commands.iter().any(|command| {
            matches!(
                command,
                Command::DrawRect { width, height, .. }
                    if *width == Pt::from_f32(135.0) && *height == Pt::from_f32(1.5)
            )
        }));
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
    fn grid_display_contents_promotes_children_into_parent_tracks() {
        let html = r#"
            <!doctype html>
            <html><body>
              <div class="grid">
                <div class="contents"><div>A</div><div>B</div></div>
                <div>C</div>
              </div>
            </body></html>
        "#;
        let css = r#"
            @page { size: 4in 2in; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            .grid {
              display: grid;
              grid-template-columns: 70px 70px 70px;
              grid-template-rows: 60px;
              width: 210px;
            }
            .contents { display: contents; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");
        let mut positions = std::collections::HashMap::new();
        for command in &page.commands {
            if let Command::DrawString { text, x, y } = command {
                let text = text.trim();
                if matches!(text, "A" | "B" | "C") {
                    positions.insert(text.to_string(), (*x, *y));
                }
            }
        }
        let (a_x, a_y) = positions["A"];
        let (b_x, b_y) = positions["B"];
        let (c_x, c_y) = positions["C"];
        assert!(
            a_x < b_x && b_x < c_x,
            "grid items must occupy three columns"
        );
        assert_eq!(a_y, b_y);
        assert_eq!(b_y, c_y);
    }

    #[test]
    fn vertical_rl_multicol_renders_compiled_x_axis_plan_and_blank_tail_page() {
        let html = r#"
            <!doctype html>
            <html><body><div class="cols">
              <div class="item a">A</div><div class="item b">B</div><div class="item c">C</div>
            </div></body></html>
        "#;
        let css = r#"
            @page { size: 224px 160px; margin: 0; }
            html { line-height: 1.5; }
            * { margin: 0; box-sizing: border-box; }
            .cols { writing-mode: vertical-rl; width: 200px; height: 140px;
                    column-count: 2; column-gap: 20px; column-fill: auto;
                    background: #f8fafc; }
            .item { width: 40px; height: 80px; font-size: 12px; }
            .a { background: #ef476f; } .b { background: #ffd166; }
            .c { background: #06d6a0; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine.render_to_document(html, css).expect("render");

        assert_eq!(document.pages.len(), 2);
        let rects: Vec<(Pt, Pt)> = document.pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::DrawRect {
                    x,
                    y,
                    width,
                    height,
                } if *width == Pt::from_f32(30.0) && *height == Pt::from_f32(60.0) => {
                    Some((*x, *y))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            rects,
            vec![
                (Pt::from_f32(120.0), Pt::ZERO),
                (Pt::from_f32(90.0), Pt::ZERO),
                (Pt::from_f32(60.0), Pt::ZERO),
            ]
        );
        assert_eq!(
            document.pages[0]
                .commands
                .iter()
                .filter(|command| matches!(command, Command::Rotate(_)))
                .count(),
            3
        );
        assert!(
            document.pages[1]
                .commands
                .iter()
                .all(|command| matches!(command, Command::Meta { .. })),
            "the Chromium-compatible tail page must remain paint-empty"
        );
    }

    #[test]
    fn auto_height_grid_rows_split_at_fragmentainer_boundaries() {
        let html = r#"<!doctype html><html><body><div class="grid"><div></div><div></div><div></div></div></body></html>"#;
        let css = r#"
            * { margin: 0; box-sizing: border-box; }
            .grid { display: grid; grid-template-columns: 180px; grid-auto-rows: 90px; width: 180px; border: 2px solid #000; }
            .grid > div { background: #d7263d; }
        "#;
        let resolver = style::StyleResolver::new(css);
        let story = html::html_to_story_with_resolver_and_fonts_and_report(
            html, &resolver, None, None, None, false, false, None, None,
        );
        assert_eq!(story.len(), 1);
        let page_width = Pt::from_f32(168.0);
        let page_height = Pt::from_f32(102.0);
        let size = story[0].wrap(page_width, page_height);
        assert!(
            size.height > page_height,
            "grid must expose its full row height, got {size:?}"
        );
        let (first, remaining) = story[0]
            .split(page_width, page_height)
            .expect("grid body should split after its first fixed row");
        assert!(first.wrap(page_width, page_height).height <= page_height);
        assert!(remaining.wrap(page_width, page_height).height > Pt::ZERO);
    }

    #[test]
    fn paged_grid_slices_fixed_height_items_into_remaining_page_space() {
        let html = r#"<!doctype html><html><body><div class="grid"><div class="a">A</div><div class="b">B</div><div class="c">C</div><div class="d">D</div><div class="e">E</div><div class="f">F</div></div></body></html>"#;
        let css = r#"
            @page { size: 160px 144px; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            .grid { display: grid; grid-template-columns: 160px; width: 160px; }
            .grid > div { height: 50px; border-bottom: 2px solid #fff; }
            .a { background: #ef476f; } .b { background: #ffd166; }
            .c { background: #06d6a0; } .d { background: #118ab2; }
            .e { background: #a78bfa; } .f { background: #f97316; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        let green = Color::rgb(6.0 / 255.0, 214.0 / 255.0, 160.0 / 255.0);

        assert_eq!(doc.pages.len(), 3, "grid continuations must survive");

        let mut fill = Color::BLACK;
        let first_page_has_green_fragment =
            doc.pages[0].commands.iter().any(|command| match command {
                Command::SetFillColor(color) => {
                    fill = *color;
                    false
                }
                Command::DrawRect { height, .. } => fill == green && *height > Pt::ZERO,
                _ => false,
            });
        assert!(
            first_page_has_green_fragment,
            "row C should fill page 1's remainder"
        );
    }

    #[test]
    fn paged_column_flex_slices_fixed_height_items_into_remaining_page_space() {
        let html = r#"<!doctype html><html><body><div class="flex"><div class="a">A</div><div class="b">B</div><div class="c">C</div><div class="d">D</div><div class="e">E</div><div class="f">F</div></div></body></html>"#;
        let css = r#"
            @page { size: 160px 144px; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            .flex { display: flex; flex-direction: column; width: 160px; }
            .flex > div { height: 50px; border-bottom: 2px solid #fff; }
            .a { background: #ef476f; } .b { background: #ffd166; }
            .c { background: #06d6a0; } .d { background: #118ab2; }
            .e { background: #a78bfa; } .f { background: #f97316; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        let green = Color::rgb(6.0 / 255.0, 214.0 / 255.0, 160.0 / 255.0);

        assert_eq!(doc.pages.len(), 3, "flex continuations must survive");

        let mut fill = Color::BLACK;
        let first_page_has_green_fragment =
            doc.pages[0].commands.iter().any(|command| match command {
                Command::SetFillColor(color) => {
                    fill = *color;
                    false
                }
                Command::DrawRect { height, .. } => fill == green && *height > Pt::ZERO,
                _ => false,
            });
        assert!(
            first_page_has_green_fragment,
            "item C should fill page 1's remainder"
        );
    }

    #[test]
    fn tall_svg_backed_images_reuse_one_vector_surface_across_page_fragments() {
        let image = "data:image/svg+xml,%3Csvg%20xmlns%3D%27http%3A//www.w3.org/2000/svg%27%20width%3D%2780%27%20height%3D%27260%27%3E%3Crect%20width%3D%2780%27%20height%3D%27260%27%20fill%3D%27%23118ab2%27/%3E%3Crect%20width%3D%2780%27%20height%3D%2765%27%20fill%3D%27%23ef476f%27/%3E%3C/svg%3E";
        let engine = FullBleed::builder().build().expect("engine");

        let plain = engine
            .render_to_document(
                &format!("<!doctype html><html><body><img src='{image}'></body></html>"),
                "@page { size: 144px 120px; margin: 0; } \
                 * { margin: 0; box-sizing: border-box; } \
                 img { display: block; width: 80px; height: 260px; object-fit: fill; }",
            )
            .expect("render plain tall SVG image");
        assert_eq!(plain.pages.len(), 3);

        let bordered = engine
            .render_to_document(
                &format!("<!doctype html><html><body><img src='{image}'></body></html>"),
                "@page { size: 152px 120px; margin: 0; } \
                 * { margin: 0; box-sizing: border-box; } \
                 img { display: block; width: 90px; height: 260px; object-fit: fill; border: 8px solid #111; }",
            )
            .expect("render bordered tall SVG image");
        let border = Color::rgb(17.0 / 255.0, 17.0 / 255.0, 17.0 / 255.0);
        assert_eq!(
            bordered.pages.len(),
            3,
            "command counts: {:?}",
            bordered
                .pages
                .iter()
                .map(|page| page.commands.len())
                .collect::<Vec<_>>()
        );
        assert!(
            bordered
                .pages
                .iter()
                .all(|page| page.commands.iter().any(|command| {
                    matches!(
                        command,
                        Command::SetFillColor(color) | Command::SetStrokeColor(color)
                            if *color == border
                    )
                })),
            "sliced border sides must remain painted on every fragment"
        );
    }

    #[test]
    fn tall_raster_images_reuse_one_full_source_lattice_across_page_fragments() {
        let rgba = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];
        let png = crate::image_native::encode_png_rgba8(&rgba, 2, 2).expect("encode raster");
        let image = format!(
            "data:image/png;base64,{}",
            crate::base64::encode_standard(png)
        );
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine
            .render_to_document(
                &format!("<!doctype html><html><body><img src='{image}'></body></html>"),
                "@page { size: 144px 120px; margin: 0; } \
                 * { margin: 0; box-sizing: border-box; } \
                 img { display: block; width: 80px; height: 260px; object-fit: fill; }",
            )
            .expect("render tall raster image");

        assert_eq!(document.pages.len(), 3);
        let expected_y = [Pt::ZERO, Pt::from_f32(-90.0), Pt::from_f32(-180.0)];
        for (page, expected_y) in document.pages.iter().zip(expected_y) {
            let draw = page
                .commands
                .iter()
                .find_map(|command| match command {
                    Command::DrawImage {
                        y,
                        width,
                        height,
                        resource_id,
                        source_clip,
                        ..
                    } => Some((*y, *width, *height, resource_id, source_clip)),
                    _ => None,
                })
                .expect("each fragment should draw the same raster source");
            assert_eq!(draw.0, expected_y);
            assert_eq!(draw.1, Pt::from_f32(60.0));
            assert_eq!(draw.2, Pt::from_f32(195.0));
            assert_eq!(draw.3, &image);
            assert!(draw.4.is_none());
        }
    }

    #[test]
    fn oversized_single_table_row_reuses_one_compiled_surface_across_pages() {
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine
            .render_to_document(
                "<!doctype html><html><body><table><tr><td></td></tr></table></body></html>",
                "@page { size: 320px 200px; margin: 32px; } \
                 * { margin: 0; box-sizing: border-box; } \
                 table { border-collapse: collapse; width: 100%; } \
                 td { background: #cfe3ff; height: 360px; vertical-align: top; }",
            )
            .expect("render oversized table row");

        assert_eq!(document.pages.len(), 3);
        let draws: Vec<_> = document
            .pages
            .iter()
            .map(|page| {
                page.commands
                    .iter()
                    .find_map(|command| match command {
                        Command::DrawForm {
                            y,
                            width,
                            height,
                            resource_id,
                            ..
                        } if resource_id.starts_with("table-row-fragment:") => {
                            Some((*y, *width, *height, resource_id.clone()))
                        }
                        _ => None,
                    })
                    .expect("each page should draw the compiled row surface")
            })
            .collect();
        assert_eq!(draws[0].0, Pt::from_f32(24.0));
        assert_eq!(draws[1].0, Pt::from_f32(-78.0));
        assert_eq!(draws[2].0, Pt::from_f32(-180.0));
        assert!(draws.iter().all(|draw| {
            draw.1 == Pt::from_f32(192.0) && draw.2 == Pt::from_f32(270.0) && draw.3 == draws[0].3
        }));
    }

    #[test]
    fn named_page_transition_reflows_continuations_in_named_geometry() {
        let engine = FullBleed::builder().build().expect("engine");
        let html = "<!doctype html><html><body><div class='cover'></div><div class='chapter'></div></body></html>";
        let css = "@page { size: 160px 120px; margin: 0; } \
                   @page chapter { size: 160px 104px; margin: 20px; } \
                   * { margin: 0; box-sizing: border-box; } \
                   .cover { height: 120px; background: #1d4ed8; } \
                   .chapter { page: chapter; height: 220px; background: #16a34a; }";
        let document = engine
            .render_to_document(html, css)
            .expect("render named page continuation");

        assert_eq!(document.pages.len(), 5);
        assert_eq!(
            document.page_size,
            Size {
                width: Pt::from_f32(120.0),
                height: Pt::from_f32(90.0),
            }
        );
        for page in &document.pages[1..] {
            assert!(page.commands.iter().any(|command| {
                matches!(command, Command::Meta { key, value }
                    if key == canvas::META_PAGE_SIZE_KEY && value == "120000,78000")
            }));
        }
        let pdf = engine
            .render_to_buffer(html, css)
            .expect("emit mixed-size named-page PDF");
        let pdf = String::from_utf8_lossy(&pdf);
        assert!(pdf.contains("/MediaBox [0 0 120 90]"));
        assert_eq!(pdf.matches("/MediaBox [0 0 120 78]").count(), 4);
    }

    #[test]
    fn explicit_builder_margins_override_named_page_margins() {
        let engine = FullBleed::builder()
            .margin_all(9.0)
            .build()
            .expect("engine");
        let document = engine
            .render_to_document(
                "<!doctype html><html><body><div class='chapter'></div></body></html>",
                "@page chapter { size: 160px 120px; margin: 40px; } \
                 * { margin: 0; box-sizing: border-box; } \
                 .chapter { page: chapter; height: 40px; background: #16a34a; }",
            )
            .expect("render named page with explicit runtime margins");

        assert_eq!(document.pages.len(), 1);
        assert!(document.pages[0].commands.iter().any(|command| {
            matches!(command, Command::DrawRect { x, y, .. }
                if *x == Pt::from_f32(9.0) && *y == Pt::from_f32(9.0))
        }));
    }

    #[test]
    fn compiled_named_page_copies_preserve_page_sizes() {
        let engine = FullBleed::builder().build().expect("engine");
        let compiled = engine
            .compile_document(
                "<!doctype html><html><body><div class='cover'></div><div class='chapter'></div></body></html>",
                "@page { size: 160px 120px; margin: 0; } \
                 @page chapter { size: 160px 104px; margin: 0; } \
                 * { margin: 0; box-sizing: border-box; } \
                 .cover { height: 120px; background: #1d4ed8; } \
                 .chapter { page: chapter; height: 104px; background: #16a34a; }",
            )
            .expect("compile mixed-size named pages");
        assert_eq!(compiled.page_count(), 2);

        let batch = compiled
            .render_many_to_buffer(3)
            .expect("link mixed-size compiled copies");
        let pdf = String::from_utf8_lossy(&batch);
        assert_eq!(pdf.matches("/MediaBox [0 0 120 90]").count(), 3);
        assert_eq!(pdf.matches("/MediaBox [0 0 120 78]").count(), 3);
    }

    #[test]
    fn compiled_named_page_bindings_preserve_page_sizes() {
        let engine = FullBleed::builder().build().expect("engine");
        let compiled = engine
            .compile_document(
                "<!doctype html><html><body><div class='cover'>{{record}}</div><div class='chapter'>{{record}}</div></body></html>",
                "@page { size: 160px 120px; margin: 0; } \
                 @page chapter { size: 160px 104px; margin: 0; } \
                 * { margin: 0; box-sizing: border-box; } \
                 .cover { height: 120px; background: #1d4ed8; } \
                 .chapter { page: chapter; height: 104px; background: #16a34a; }",
            )
            .expect("compile mixed-size named-page bindings");
        assert_eq!(compiled.page_count(), 2);

        let bindings = std::collections::HashMap::from([(
            "record".to_string(),
            vec!["A-001".to_string(), "A-002".to_string()],
        )]);
        let batch = compiled
            .render_bindings_to_buffer(&bindings)
            .expect("link mixed-size binding records");
        let pdf = String::from_utf8_lossy(&batch);
        assert_eq!(pdf.matches("/MediaBox [0 0 120 90]").count(), 2);
        assert_eq!(pdf.matches("/MediaBox [0 0 120 78]").count(), 2);
    }

    #[test]
    fn named_page_selector_lists_and_physical_sides_select_compiled_templates() {
        let engine = FullBleed::builder().build().expect("engine");
        let listed = engine
            .render_to_document(
                "<!doctype html><html><body><div class='title'></div><div class='chapter'></div></body></html>",
                "@page { size: 184px 120px; margin: 0; } \
                 @page title, chapter { size: 184px 120px; margin-left: 40px; } \
                 * { margin: 0; box-sizing: border-box; } \
                 .title { page: title; height: 120px; background: #ffd166; } \
                 .chapter { page: chapter; height: 120px; background: #118ab2; }",
            )
            .expect("render named selector list");
        assert_eq!(listed.pages.len(), 2);
        for page in &listed.pages {
            assert!(page.commands.iter().any(|command| {
                matches!(command, Command::DrawRect { x, .. } if *x == Pt::from_f32(30.0))
            }));
        }

        let sided = engine
            .render_to_document(
                "<!doctype html><html><body><div class='cover'></div><div class='chapter'></div></body></html>",
                "@page { size: 184px 120px; margin: 0; } \
                 @page chapter:left { size: 184px 120px; margin-left: 40px; } \
                 @page chapter:right { size: 184px 120px; margin-left: 0; } \
                 * { margin: 0; box-sizing: border-box; } \
                 .cover { height: 120px; background: #1d4ed8; } \
                 .chapter { page: chapter; height: 240px; background: #16a34a; }",
            )
            .expect("render named page sides");
        assert_eq!(sided.pages.len(), 3);
        assert!(sided.pages[1].commands.iter().any(|command| {
            matches!(command, Command::DrawRect { x, .. } if *x == Pt::from_f32(30.0))
        }));
        assert!(
            sided.pages[2].commands.iter().any(|command| {
                matches!(command, Command::DrawRect { x, .. } if *x == Pt::ZERO)
            })
        );
    }

    #[test]
    fn named_page_cascades_its_margin_box_paint() {
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine
            .render_to_document(
                "<!doctype html><html><body><div class='page'></div><div class='page chapter'></div></body></html>",
                "@page { size: 160px 120px; margin: 24px 0 0; \
                         @top-center { content: ' '; background: #1d4ed8; width: 160px; } } \
                 @page chapter { size: 160px 120px; margin: 24px 0 0; \
                                 @top-center { content: ' '; background: #16a34a; width: 160px; } } \
                 * { margin: 0; box-sizing: border-box; } \
                 .page { height: 96px; } .chapter { page: chapter; }",
            )
            .expect("render named page margin box");
        let blue = Color::rgb(29.0 / 255.0, 78.0 / 255.0, 216.0 / 255.0);
        let green = Color::rgb(22.0 / 255.0, 163.0 / 255.0, 74.0 / 255.0);

        assert_eq!(document.pages.len(), 2);
        assert!(page_contains_fill_color(&document.pages[0], blue));
        assert!(!page_contains_fill_color(&document.pages[0], green));
        assert!(
            page_contains_fill_color(&document.pages[1], green),
            "named-page commands: {:#?}",
            document.pages[1].commands
        );
    }

    #[test]
    fn page_margin_boxes_compile_content_regions_and_page_context() {
        let engine = FullBleed::builder().build().expect("engine");
        let regions = engine
            .render_to_document(
                "<!doctype html><html><body><div class='box'></div></body></html>",
                "@page { size: 200px 144px; margin: 24px 0; \
                   @top-left { content: 'TL'; } @top-center { content: 'TC'; } \
                   @top-right { content: 'TR'; } @bottom-center { content: 'BC'; } } \
                 * { margin: 0; box-sizing: border-box; } \
                 .box { height: 60px; background: #06d6a0; }",
            )
            .expect("render page margin regions");
        assert_eq!(regions.pages.len(), 1);
        for text in ["TL", "TC", "TR", "BC"] {
            assert!(
                page_contains_text(&regions.pages[0], text),
                "missing {text}"
            );
        }

        let cascaded = engine
            .render_to_document(
                "<!doctype html><html><body><div class='p'></div><div class='q'></div></body></html>",
                "@page { size: 184px 120px; margin: 20px 0 0; font-size: 16px; \
                         @top-center { content: 'BASE'; } } \
                 @page :first { font-size: 20px; @top-center { content: 'FIRST'; } } \
                 * { margin: 0; box-sizing: border-box; } \
                 .p, .q { height: 100px; }",
            )
            .expect("render cascaded page margin text");
        assert_eq!(cascaded.pages.len(), 2);
        assert!(page_contains_text(&cascaded.pages[0], "FIRST"));
        assert!(!page_contains_text(&cascaded.pages[0], "BASE"));
        assert!(page_contains_text(&cascaded.pages[1], "BASE"));
        assert!(cascaded.pages[0].commands.iter().any(
            |command| matches!(command, Command::SetFontSize(size) if *size == Pt::from_f32(15.0))
        ));
        assert!(!cascaded.pages[1].commands.iter().any(
            |command| matches!(command, Command::SetFontSize(size) if *size == Pt::from_f32(15.0))
        ));
    }

    #[test]
    fn page_margin_box_counter_is_bound_from_the_compiled_page_context() {
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine
            .render_to_document(
                "<!doctype html><html><body><div></div><div></div></body></html>",
                "@page { size: 160px 120px; margin: 24px 0 0; \
                         @top-center { content: 'P' counter(page); } } \
                 * { margin: 0; box-sizing: border-box; } div { height: 96px; }",
            )
            .expect("render margin-box page counters");
        assert_eq!(document.pages.len(), 2);
        assert!(page_contains_text(&document.pages[0], "P1"));
        assert!(page_contains_text(&document.pages[1], "P2"));
    }

    #[test]
    fn page_counter_programs_finalize_after_pagination() {
        let engine = FullBleed::builder().build().expect("engine");
        let incremented = engine
            .render_to_document(
                "<!doctype html><html><body><div></div><div></div><div></div></body></html>",
                "@page { size: 160px 104px; margin: 20px 0 0; \
                         counter-increment: page 2; \
                         @top-center { content: 'P' counter(page); } } \
                 * { margin: 0; box-sizing: border-box; } \
                 div { height: 40px; } div + div { break-before: page; }",
            )
            .expect("render incremented page counter");
        assert_eq!(incremented.pages.len(), 3);
        for (page, expected) in incremented.pages.iter().zip(["P2", "P4", "P6"]) {
            assert!(page_contains_text(page, expected), "missing {expected}");
        }

        let reset = engine
            .render_to_document(
                "<!doctype html><html><body><div></div><div></div></body></html>",
                "@page { size: 160px 104px; margin: 20px 0 0; \
                         counter-reset: page 7; counter-increment: page 2; \
                         @top-center { content: 'R' counter(page) ' of ' counter(pages); } } \
                 * { margin: 0; box-sizing: border-box; } \
                 div { height: 40px; } div + div { break-before: page; }",
            )
            .expect("render reset and total page counters");
        assert_eq!(reset.pages.len(), 2);
        for page in &reset.pages {
            assert!(page_contains_text(page, "R9 of 2"));
            assert!(matches!(page.commands.first(), Some(Command::SaveState)));
            let overlay_text = page
                .commands
                .iter()
                .position(|command| {
                    matches!(command, Command::DrawString { text, .. } if text == "R9 of 2")
                })
                .expect("finalized page-counter text");
            assert!(
                page.commands[..overlay_text]
                    .windows(2)
                    .any(|commands| matches!(
                        commands,
                        [Command::RestoreState, Command::SaveState]
                    ))
            );
        }
    }

    #[test]
    fn running_element_first_except_reuses_a_compiled_vector_surface() {
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine
            .render_to_document(
                "<!doctype html><html><body><h2>RUN HEAD</h2><div class='p'></div><div class='q'></div></body></html>",
                "@page { size: 180px 120px; margin: 22px 0 0; \
                         @top-center { content: element(head, first-except); } } \
                 html { font-family: Helvetica; line-height: 1.5; } \
                 * { margin: 0; box-sizing: border-box; } \
                 h2 { position: running(head); height: 18px; background: #ffd166; font-size: 12px; } \
                 .p { height: 98px; background: #dbeafe; } \
                 .q { height: 98px; background: #bbf7d0; }",
            )
            .expect("render running header");
        assert_eq!(document.pages.len(), 2);
        assert!(!document.pages[0].commands.iter().any(|command| {
            matches!(command, Command::DrawForm { resource_id, .. } if resource_id.starts_with("css-running-"))
        }));
        let selected = document.pages[1]
            .commands
            .iter()
            .find_map(|command| match command {
                Command::DrawForm { resource_id, .. }
                    if resource_id.starts_with("css-running-") =>
                {
                    Some(resource_id.clone())
                }
                _ => None,
            })
            .expect("running element form on page two");
        let surface = document
            .pages
            .iter()
            .flat_map(|page| page.commands.iter())
            .find_map(|command| match command {
                Command::DefineForm {
                    resource_id,
                    commands,
                    ..
                } if resource_id == &selected => Some(commands),
                _ => None,
            })
            .expect("compiled running element surface");
        assert!(surface.iter().any(
            |command| matches!(command, Command::DrawString { text, .. } if text == "RUN HEAD")
        ));
    }

    #[test]
    fn running_elements_nested_in_main_repeat_on_overflow_pages() {
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine
            .render_to_document(
                "<!doctype html><html><body><main><header>RUN HEAD</header><footer>RUN FOOT</footer><div class='p'></div><div class='q'></div></main></body></html>",
                "@page { size: 180px 120px; margin: 22px 0; \
                         @top-center { content: element(head); } \
                         @bottom-center { content: element(foot); } } \
                 * { margin: 0; box-sizing: border-box; } \
                 header { position: running(head); height: 18px; font-size: 12px; } \
                 footer { position: running(foot); height: 18px; font-size: 12px; } \
                 .p, .q { height: 50px; } \
                 .p { background: #dbeafe; } \
                 .q { background: #bbf7d0; }",
            )
            .expect("render nested running elements");
        assert_eq!(document.pages.len(), 2);
        let running_form_counts = document
            .pages
            .iter()
            .map(|page| {
                page.commands
                    .iter()
                    .filter(|command| {
                        matches!(command, Command::DrawForm { resource_id, .. } if resource_id.starts_with("css-running-"))
                    })
                    .count()
            })
            .collect::<Vec<_>>();
        assert_eq!(running_form_counts, vec![2; document.pages.len()]);
    }

    #[test]
    fn running_element_marker_does_not_force_an_empty_fixed_box_to_fragment() {
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine
            .render_to_document(
                "<!doctype html><html><body><div class='head'></div><div class='first'></div><div class='spacer'></div><div class='next'></div></body></html>",
                "@page { size: 250px 170px; margin: 40px 0 0; \
                         @top-left { content: element(head); } } \
                 * { margin: 0; box-sizing: border-box; } \
                 body { padding: 0 12px 12px; } \
                 .head { position: running(head); width: 96px; height: 24px; background: #d7263d; } \
                 .first, .next { height: 28px; background: #dbeafe; } \
                 .next { background: #bbf7d0; } \
                 .spacer { height: 76px; }",
            )
            .expect("render running-element fragmentation fixture");
        assert_eq!(document.pages.len(), 2);

        let green = Color::rgb(187.0 / 255.0, 247.0 / 255.0, 208.0 / 255.0);
        let green_rect_heights = |page: &Page| {
            let mut fill = Color::BLACK;
            let mut fill_stack = Vec::new();
            page.commands
                .iter()
                .filter_map(|command| match command {
                    Command::SaveState => {
                        fill_stack.push(fill);
                        None
                    }
                    Command::RestoreState => {
                        if let Some(saved) = fill_stack.pop() {
                            fill = saved;
                        }
                        None
                    }
                    Command::SetFillColor(color) => {
                        fill = *color;
                        None
                    }
                    Command::DrawRect { height, .. } if fill == green => Some(*height),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert!(
            green_rect_heights(&document.pages[0]).is_empty(),
            "the final empty block must not leave a painted fragment on page one"
        );
        assert_eq!(
            green_rect_heights(&document.pages[1]),
            [Pt::from_f32(21.0)],
            "the complete 28px block must paint on page two"
        );
    }

    #[test]
    fn running_element_last_and_named_string_share_finalized_page_state() {
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine
            .render_to_document(
                "<!doctype html><html><body><h2>ONE</h2><div class='spacer'></div><h2>TWO</h2><p>body</p></body></html>",
                "@page { size: 200px 150px; margin: 24px 0 0; \
                         @top-left { content: string(section, last); background: #1d4ed8; } \
                         @top-right { content: element(head, last); background: #16a34a; } } \
                 html { font-family: Helvetica; line-height: 1.5; } \
                 * { margin: 0; box-sizing: border-box; } \
                 h2 { position: running(head); string-set: section content(); \
                      width: 200px; height: 18px; font-size: 12px; line-height: 18px; } \
                 .spacer { width: 200px; height: 42px; } \
                 p { width: 200px; height: 60px; font-size: 12px; }",
            )
            .expect("render last running content");
        assert_eq!(document.pages.len(), 1);
        assert!(page_contains_text(&document.pages[0], "TWO"));
        let selected = document.pages[0]
            .commands
            .iter()
            .find_map(|command| match command {
                Command::DrawForm { resource_id, .. }
                    if resource_id.starts_with("css-running-") =>
                {
                    Some(resource_id.clone())
                }
                _ => None,
            })
            .expect("selected last running element");
        let selected_surface = document.pages[0]
            .commands
            .iter()
            .find_map(|command| match command {
                Command::DefineForm {
                    resource_id,
                    commands,
                    ..
                } if resource_id == &selected => Some(commands),
                _ => None,
            })
            .expect("selected surface definition");
        assert!(
            selected_surface.iter().any(
                |command| matches!(command, Command::DrawString { text, .. } if text == "TWO")
            )
        );

        let green = Color::rgb(22.0 / 255.0, 163.0 / 255.0, 74.0 / 255.0);
        let mut fill = Color::BLACK;
        let mut fill_stack = Vec::new();
        let expanded_green_rect = document.pages[0]
            .commands
            .iter()
            .find_map(|command| match command {
                Command::SaveState => {
                    fill_stack.push(fill);
                    None
                }
                Command::RestoreState => {
                    if let Some(saved) = fill_stack.pop() {
                        fill = saved;
                    }
                    None
                }
                Command::SetFillColor(color) => {
                    fill = *color;
                    None
                }
                Command::DrawRect { x, width, .. } if fill == green => Some((*x, *width)),
                _ => None,
            })
            .expect("expanded running-element margin background");
        assert_eq!(expanded_green_rect, (Pt::ZERO, Pt::from_f32(150.0)));
    }

    #[test]
    fn named_string_start_carries_attribute_value_to_following_pages() {
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine
            .render_to_document(
                "<!doctype html><html><body><h2 data-title='ALPHA'>ignored text</h2><div></div></body></html>",
                "@page { size: 192px 136px; margin: 24px 0 0; \
                         @top-center { content: string(section, start); } } \
                 * { margin: 0; box-sizing: border-box; } \
                 h2 { string-set: section attr(data-title); height: 20px; font-size: 12px; } \
                 div { height: 100px; break-before: page; }",
            )
            .expect("render carried named string");
        assert_eq!(document.pages.len(), 2);
        for page in &document.pages {
            assert!(page_contains_text(page, "ALPHA"));
        }
    }

    #[test]
    fn named_string_content_text_populates_continuation_margin_boxes() {
        let engine = FullBleed::builder().build().expect("engine");
        let body = (0..48)
            .map(|index| format!("<p>RUNNING-LINE-{index:03} words words words words</p>"))
            .collect::<String>();
        let document = engine
            .render_to_document(
                &format!("<h1>RUNNING-DOC-741</h1>{body}"),
                "@page { size: letter; margin: 36pt; \
                         @top-right { content: string(document-title); } } \
                 * { box-sizing: border-box; } \
                 body { margin: 0; font: 9pt/11pt Helvetica, sans-serif; } \
                 h1 { margin: 0 0 6pt; font-size: 12pt; line-height: 14pt; \
                      string-set: document-title content(text); } \
                 p { margin: 0 0 5pt; }",
            )
            .expect("render content(text) named string");
        assert!(document.pages.len() > 1);
        for (index, page) in document.pages.iter().enumerate() {
            assert!(
                page_contains_text(page, "RUNNING-DOC-741"),
                "the named string must be carried into every continuation header; missing on page {} of {}",
                index + 1,
                document.pages.len(),
            );
        }
    }

    #[test]
    fn oversized_avoid_and_long_table_fragment_in_the_current_page_remainder() {
        let engine = FullBleed::builder().build().expect("engine");
        let oversized_lines = (0..80)
            .map(|index| format!("<p>OVERSIZE-LINE-{index:03} detail detail detail</p>"))
            .collect::<String>();
        let oversized = engine
            .render_to_document(
                &format!(
                    "<h1>Oversized keep</h1><section><strong>OVERSIZE-START</strong>{oversized_lines}<strong>OVERSIZE-END</strong></section>"
                ),
                "@page { size: 240pt 180pt; margin: 18pt; } \
                 body { margin: 0; font: 9pt/11pt Helvetica, sans-serif; } \
                 h1 { margin: 0 0 6pt; } section { break-inside: avoid; } \
                 p { margin: 0 0 3pt; }",
            )
            .expect("render oversized avoid block");
        assert!(oversized.pages.len() > 1);
        assert!(
            page_contains_text(&oversized.pages[0], "OVERSIZE-START"),
            "an avoid box taller than a fresh frame must relax avoidance immediately"
        );

        let rows = (0..80)
            .map(|index| format!("<tr><td>ROW-{index:03}</td><td>variable table content</td></tr>"))
            .collect::<String>();
        let table = engine
            .render_to_document(
                &format!(
                    "<h1>Long table</h1><table><thead><tr><th>ROW-ID</th><th>DESCRIPTION-HEADER</th></tr></thead><tbody>{rows}</tbody></table>"
                ),
                "@page { size: 240pt 180pt; margin: 18pt; } \
                 body { margin: 0; font: 8pt/10pt Helvetica, sans-serif; } \
                 h1 { margin: 0 0 6pt; font-size: 12pt; line-height: 14pt; } \
                 table { width: 100%; border-collapse: collapse; } \
                 th, td { border: 0.5pt solid #999; padding: 2pt; }",
            )
            .expect("render long table");
        assert!(table.pages.len() > 1);
        assert!(page_contains_text(&table.pages[0], "Long table"));
        assert!(
            page_contains_text(&table.pages[0], "DESCRIPTION-HEADER"),
            "a splittable table must use the remainder after its heading"
        );
        assert!(page_contains_text(&table.pages[0], "ROW-000"));
    }

    #[test]
    fn page_margin_boxes_inherit_root_typography_and_corner_alignment() {
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine
            .render_to_document(
                "<!doctype html><html><body><div></div></body></html>",
                "@page { size: 200px 144px; margin: 24px; \
                         @top-left-corner { content: 'C'; } \
                         @top-left { content: 'L'; } } \
                 html { font-family: Courier; font-size: 20px; line-height: 1.5; } \
                 * { margin: 0; box-sizing: border-box; } div { height: 70px; }",
            )
            .expect("render inherited page typography");

        assert!(
            document.pages[0]
                .commands
                .iter()
                .any(|command| matches!(command, Command::SetFontName(name) if name == "Courier"))
        );
        assert!(document.pages[0].commands.iter().any(
            |command| matches!(command, Command::SetFontSize(size) if *size == Pt::from_f32(15.0))
        ));
        let page_size = Size {
            width: Pt::from_f32(150.0),
            height: Pt::from_f32(108.0),
        };
        let margins = Margins {
            top: Pt::from_f32(18.0),
            right: Pt::from_f32(18.0),
            bottom: Pt::from_f32(18.0),
            left: Pt::from_f32(18.0),
        };
        let (_, left_corner_align) = page_margin_box_rect(
            page_size,
            margins,
            style::CssPageMarginBoxKind::TopLeftCorner,
            None,
        );
        let (_, right_corner_align) = page_margin_box_rect(
            page_size,
            margins,
            style::CssPageMarginBoxKind::TopRightCorner,
            None,
        );
        assert_eq!(left_corner_align, TextAlign::Right);
        assert_eq!(right_corner_align, TextAlign::Left);
    }

    #[test]
    fn paragraph_reflows_intact_when_page_remainder_cannot_satisfy_orphans() {
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine
            .render_to_document(
                "<!doctype html><html><body><div class='spacer'></div>\
                 <p class='para'>Orphans line one<br>Orphans line two<br>\
                 Orphans line three<br>Orphans line four<br>Orphans line five</p>\
                 </body></html>",
                "@page { size: 600px 192px; margin: 0; } \
                 * { margin: 0; box-sizing: border-box; } \
                 .spacer { height: 100px; } \
                 .para { font-size: 20px; line-height: 1.5; orphans: 4; widows: 2; }",
            )
            .expect("render orphan-constrained paragraph");

        assert_eq!(document.pages.len(), 2);
        assert!(!page_contains_text(&document.pages[0], "Orphans"));
        for suffix in ["one", "two", "three", "four", "five"] {
            assert!(
                page_contains_text(&document.pages[1], suffix),
                "page two is missing line {suffix}"
            );
        }
    }

    #[test]
    fn named_page_custom_idents_remain_case_sensitive() {
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine
            .render_to_document(
                "<!doctype html><html><body>\
                 <div class='sheet chapter'></div><div class='sheet chapter'></div>\
                 <div class='sheet appendix'></div><div class='sheet appendix'></div>\
                 <div class='sheet lowercase'></div></body></html>",
                "@page { size: 160px 120px; margin: 0; background: #ef476f; } \
                 @page Chapter { background: #ffd166; } \
                 @page Chapter:left, Appendix:right { background: #3a86ff; } \
                 @page Chapter:first { background: #06d6a0; } \
                 * { margin: 0; box-sizing: border-box; } \
                 .sheet { width: 48px; height: 40px; background: #111827; } \
                 .sheet + .sheet { break-before: page; } \
                 .chapter { page: Chapter; } .appendix { page: Appendix; } \
                 .lowercase { page: chapter; }",
            )
            .expect("render case-sensitive named pages");
        let green = Color::rgb(6.0 / 255.0, 214.0 / 255.0, 160.0 / 255.0);
        let blue = Color::rgb(58.0 / 255.0, 134.0 / 255.0, 1.0);
        let red = Color::rgb(239.0 / 255.0, 71.0 / 255.0, 111.0 / 255.0);

        assert_eq!(document.pages.len(), 5);
        assert!(page_contains_fill_color(&document.pages[0], green));
        assert!(page_contains_fill_color(&document.pages[1], blue));
        assert!(page_contains_fill_color(&document.pages[2], blue));
        assert!(page_contains_fill_color(&document.pages[3], red));
        assert!(page_contains_fill_color(&document.pages[4], red));
    }

    #[test]
    fn forced_page_sides_compile_required_blank_pages_and_boundary_precedence() {
        let cases = [
            (
                "<div class='block first'></div><div class='block second'></div>",
                ".first { break-after: right; }",
                3,
            ),
            (
                "<div class='block a'></div><div class='block b'></div><div class='block c'></div>",
                ".b { break-after: left; }",
                4,
            ),
            (
                "<div class='block a'></div><div class='block b'></div><div class='block c'></div>",
                ".c { break-before: verso; }",
                4,
            ),
            (
                "<div class='block first'></div><div class='block second'></div>",
                ".first { break-after: left; } .second { break-before: right; }",
                3,
            ),
        ];
        let engine = FullBleed::builder().build().expect("engine");
        for (body, rule, expected_pages) in cases {
            let html = format!("<!doctype html><html><body>{body}</body></html>");
            let css = format!(
                "@page {{ size: 160px 120px; margin: 0; }} \
                 * {{ margin: 0; box-sizing: border-box; }} \
                 .block {{ width: 160px; height: 120px; }} {rule}"
            );
            let document = engine
                .render_to_document(&html, &css)
                .expect("render forced page side");
            assert_eq!(
                document.pages.len(),
                expected_pages,
                "unexpected page count for {rule}",
            );
        }
    }

    #[test]
    fn terminal_break_after_does_not_create_a_trailing_blank_page() {
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine
            .render_to_document(
                "<!doctype html><html><body><div class='only'></div></body></html>",
                "@page { size: 160px 120px; margin: 0; } \
                 * { margin: 0; box-sizing: border-box; } \
                 .only { width: 160px; height: 120px; break-after: always; }",
            )
            .expect("render terminal break-after");

        assert_eq!(document.pages.len(), 1);
    }

    #[test]
    fn generated_blank_page_uses_the_blank_page_pseudo_style() {
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine
            .render_to_document(
                "<!doctype html><html><body><div class='block first'></div><div class='block second'></div><div class='block third'></div></body></html>",
                "@page { size: 160px 120px; margin: 0; background: #ffffff; } \
                 @page :left { background: #ef476f; } \
                 @page :blank { background: #ffd166; } \
                 * { margin: 0; box-sizing: border-box; } \
                 .block { width: 64px; height: 48px; } \
                 .first { break-after: right; } \
                 .third { break-before: page; }",
            )
            .expect("render blank page style");
        let blank_yellow = Color::rgb(1.0, 209.0 / 255.0, 102.0 / 255.0);
        let left_red = Color::rgb(239.0 / 255.0, 71.0 / 255.0, 111.0 / 255.0);

        assert_eq!(document.pages.len(), 4);
        assert!(page_contains_fill_color(&document.pages[1], blank_yellow));
        assert!(!page_contains_fill_color(&document.pages[1], left_red));
    }

    #[test]
    fn generated_blank_page_cascades_top_center_margin_box_paint() {
        let engine = FullBleed::builder().build().expect("engine");
        let document = engine
            .render_to_document(
                "<!doctype html><html><body><div class='block'></div><div class='full'></div><div class='tail'></div></body></html>",
                "@page { size: 160px 120px; margin: 24px 0 0 0; } \
                 @page :right { @top-center { content: ' '; background: #ef476f; width: 160px; } } \
                 @page :blank { @top-center { content: ' '; background: #118ab2; width: 160px; } } \
                 * { margin: 0; box-sizing: border-box; } \
                 .block { height: 96px; } \
                 .full { height: 96px; break-after: left; } \
                 .tail { height: 96px; }",
            )
            .expect("render blank margin box");
        let blank_blue = Color::rgb(17.0 / 255.0, 138.0 / 255.0, 178.0 / 255.0);
        let right_red = Color::rgb(239.0 / 255.0, 71.0 / 255.0, 111.0 / 255.0);
        let blank_page = &document.pages[2];
        let mut fill = Color::BLACK;
        let has_blank_bar = blank_page.commands.iter().any(|command| match command {
            Command::SetFillColor(color) => {
                fill = *color;
                false
            }
            Command::DrawRect {
                x,
                y,
                width,
                height,
            } => {
                fill == blank_blue
                    && *x == Pt::ZERO
                    && *y == Pt::ZERO
                    && *width == Pt::from_f32(120.0)
                    && *height == Pt::from_f32(18.0)
            }
            _ => false,
        });

        assert_eq!(document.pages.len(), 4);
        assert!(has_blank_bar);
        assert!(!page_contains_fill_color(blank_page, right_red));
    }

    #[test]
    fn descendant_forced_breaks_propagate_to_their_parent_box() {
        let engine = FullBleed::builder().build().expect("engine");
        let html = "<!doctype html><html><body><div class='top'></div><div class='wrap'><div class='child'></div></div></body></html>";
        let css = "@page { size: 184px 120px; margin: 0; } \
                   * { margin: 0; box-sizing: border-box; } \
                   .top { height: 80px; background: #1d4ed8; } \
                   .wrap { padding: 10px; background: #fde68a; } \
                   .child { height: 60px; background: #16a34a; break-before: page; }";
        let document = engine
            .render_to_document(html, css)
            .expect("render propagated child break");
        let yellow = Color::rgb(253.0 / 255.0, 230.0 / 255.0, 138.0 / 255.0);
        let green = Color::rgb(22.0 / 255.0, 163.0 / 255.0, 74.0 / 255.0);

        assert_eq!(document.pages.len(), 2);
        assert!(!page_contains_fill_color(&document.pages[0], yellow));
        assert!(!page_contains_fill_color(&document.pages[0], green));
        assert!(page_contains_fill_color(&document.pages[1], yellow));
        assert!(page_contains_fill_color(&document.pages[1], green));
    }

    #[test]
    fn avoid_page_boundary_moves_both_boxes_to_the_next_page() {
        let engine = FullBleed::builder().build().expect("engine");
        let html = "<!doctype html><html><body><div class='spacer'></div><div class='head'></div><div class='next'></div></body></html>";
        let css = "@page { size: 160px 120px; margin: 0; } \
                   * { margin: 0; box-sizing: border-box; } \
                   .spacer { height: 72px; background: #bfdbfe; } \
                   .head { height: 24px; background: #f59e0b; break-after: avoid; } \
                   .next { height: 32px; background: #22c55e; }";
        let document = engine
            .render_to_document(html, css)
            .expect("render avoided page boundary");
        let orange = Color::rgb(245.0 / 255.0, 158.0 / 255.0, 11.0 / 255.0);
        let green = Color::rgb(34.0 / 255.0, 197.0 / 255.0, 94.0 / 255.0);

        assert_eq!(document.pages.len(), 2);
        assert!(!page_contains_fill_color(&document.pages[0], orange));
        assert!(!page_contains_fill_color(&document.pages[0], green));
        assert!(page_contains_fill_color(&document.pages[1], orange));
        assert!(page_contains_fill_color(&document.pages[1], green));
    }

    #[test]
    fn fragmented_min_height_box_does_not_repaint_a_full_minimum_on_continuation() {
        let html = r#"<!doctype html><html><body><div class="outer"><div class="a"></div><div class="b"></div></div></body></html>"#;
        let css = r#"
            @page { size: 160px 100px; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            .outer { min-height: 80px; background: #ef476f; }
            .a { height: 30px; background: #1d4ed8; }
            .b { height: 20px; background: #16a34a; break-before: page; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        let outer = Color::rgb(239.0 / 255.0, 71.0 / 255.0, 111.0 / 255.0);

        assert_eq!(
            doc.pages.len(),
            2,
            "the forced child break should make two pages"
        );
        let mut fill = Color::BLACK;
        let (background_index, virtual_background_height) = doc.pages[1]
            .commands
            .iter()
            .enumerate()
            .find_map(|(index, command)| match command {
                Command::SetFillColor(color) => {
                    fill = *color;
                    None
                }
                Command::DrawRect { height, .. } if fill == outer => Some((index, *height)),
                _ => None,
            })
            .expect("continuation should paint the outer background");
        let scope_start = doc.pages[1].commands[..background_index]
            .iter()
            .rposition(|command| matches!(command, Command::SaveState))
            .expect("virtual background should have an isolated clip scope");
        let continuation_height = doc.pages[1].commands[scope_start..background_index]
            .iter()
            .find_map(|command| match command {
                Command::ClipRect { height, .. } => Some(*height),
                _ => None,
            })
            .expect("virtual background should be clipped to the continuation");
        assert_eq!(
            continuation_height,
            Pt::from_f32(15.0),
            "the second fragment should follow its 20 CSS-pixel child, not reapply 80px"
        );
        assert!(
            virtual_background_height >= continuation_height,
            "the shared decoration surface must cover its visible continuation slice"
        );
    }

    #[test]
    fn bottom_anchored_absolute_paints_in_final_containing_block_fragment() {
        let html = r#"<!doctype html><html><body><div class="outer"><div class="own"><span>Ag</span><span class="token">Bb</span></div><div class="inner">AB</div></div></body></html>"#;
        let css = r#"
            @page { size: 192px 200px; margin: 0; }
            * { box-sizing: border-box; margin: 0; }
            html { font-size: 16px; line-height: 1.2; }
            body { font-size: 16px; }
            .outer {
                position: relative;
                width: 126px;
                min-height: 96px;
                padding: 7px;
                border: 2px solid #577590;
            }
            .own { height: 22px; white-space: nowrap; }
            .token { position: absolute; right: 4px; bottom: 4px; }
            .inner {
                width: 58px;
                height: 48px;
                padding: 5px;
                break-before: page;
                break-inside: avoid;
            }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");

        assert_eq!(
            doc.pages.len(),
            2,
            "the forced descendant break should fragment the containing block"
        );
        assert!(page_contains_text(&doc.pages[0], "Ag"));
        assert!(
            !page_contains_text(&doc.pages[0], "Bb"),
            "a bottom-anchored absolute must not paint in the first fragment"
        );
        assert!(page_contains_text(&doc.pages[1], "AB"));
        assert!(
            page_contains_text(&doc.pages[1], "Bb"),
            "the final containing-block fragment owns the bottom-anchored absolute"
        );
    }

    #[test]
    fn avoided_figure_keeps_its_empty_fixed_height_caption() {
        let html = r#"<!doctype html><html><body><div class="spacer"></div><figure><img alt="box" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='360' height='120'%3E%3Crect width='360' height='120' fill='%23f4a259'/%3E%3C/svg%3E"><figcaption></figcaption></figure></body></html>"#;
        let css = r#"
            @page { size: 360px 304px; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            .spacer { height: 200px; background: #eef1f5; border-bottom: 3px solid #6b7785; }
            figure { break-inside: avoid; }
            figure img { display: block; width: 360px; height: 120px; }
            figcaption { height: 56px; background: #34506b; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        let caption = Color::rgb(52.0 / 255.0, 80.0 / 255.0, 107.0 / 255.0);

        assert_eq!(
            doc.pages.len(),
            2,
            "the avoided figure should move to page 2"
        );
        let mut fill = Color::BLACK;
        let second_page_has_caption = doc.pages[1].commands.iter().any(|command| match command {
            Command::SetFillColor(color) => {
                fill = *color;
                false
            }
            Command::DrawRect { height, .. } => fill == caption && *height > Pt::ZERO,
            _ => false,
        });
        assert!(
            second_page_has_caption,
            "empty figcaption box must be painted"
        );
    }

    #[test]
    fn page_height_fixed_container_with_positioned_descendant_stays_atomic() {
        let html = r#"<!doctype html><html><body><div class="cb"><div class="mid"><div class="abs"></div></div></div></body></html>"#;
        let css = r#"
            @page { size: 4in 2in; margin: 0; }
            body { margin: 0; background: #fff; }
            .cb { position: relative; width: 2in; height: 2in; margin-left: 1in; background: #f00; }
            .mid { width: 1in; height: 1in; margin-left: 1in; margin-top: 0.5in; background: #0f0; }
            .abs { position: absolute; left: 0.25in; top: 0.25in; width: 0.5in; height: 0.5in; background: #00f; z-index: 1; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");

        assert_eq!(
            doc.pages.len(),
            1,
            "a page-height containing block must not create a tail fragment"
        );
        let red = Color::rgb(1.0, 0.0, 0.0);
        let blue = Color::rgb(0.0, 0.0, 1.0);
        let mut fill = Color::BLACK;
        let mut painted_red = false;
        let mut painted_blue = false;
        for command in &doc.pages[0].commands {
            match command {
                Command::SetFillColor(color) => fill = *color,
                Command::DrawRect { width, height, .. }
                    if *width > Pt::ZERO && *height > Pt::ZERO =>
                {
                    painted_red |= fill == red;
                    painted_blue |= fill == blue;
                }
                _ => {}
            }
        }
        assert!(painted_red, "the positioned containing block must paint");
        assert!(painted_blue, "the absolute descendant must paint");
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
    fn html_table_layout_fixed_ignores_later_row_intrinsic_pressure() {
        let html = r#"
            <!doctype html>
            <html>
              <body>
                <table class="t">
                  <tr><td>ROW1_LEFT</td><td>ROW1_RIGHT</td></tr>
                  <tr><td>SUPERCALIFRAGILISTICEXTRAORDINARILYLONGTOKEN</td><td>R2</td></tr>
                </table>
              </body>
            </html>
        "#;
        let css_fixed = r#"
            @page { size: 4in 4in; margin: 0.25in; }
            body { margin: 0; font-size: 12px; line-height: 1.2; }
            table.t { border-collapse: collapse; width: 300px; table-layout: fixed; }
            table.t td { border: 0; padding: 0; }
        "#;
        let css_auto = r#"
            @page { size: 4in 4in; margin: 0.25in; }
            body { margin: 0; font-size: 12px; line-height: 1.2; }
            table.t { border-collapse: collapse; width: 300px; table-layout: auto; }
            table.t td { border: 0; padding: 0; }
        "#;

        let engine = FullBleed::builder().build().expect("engine");

        let doc_fixed = engine
            .render_to_document(html, css_fixed)
            .expect("render fixed document");
        let page_fixed = doc_fixed.pages.first().expect("fixed page");
        let mut right_fixed_x: Option<Pt> = None;
        for cmd in &page_fixed.commands {
            if let Command::DrawString { text, x, .. } = cmd {
                if text == "R2" {
                    right_fixed_x = Some(*x);
                }
            }
        }
        let right_fixed_x = right_fixed_x.expect("expected R2 draw command in fixed layout");

        let doc_auto = engine
            .render_to_document(html, css_auto)
            .expect("render auto document");
        let page_auto = doc_auto.pages.first().expect("auto page");
        let mut right_auto_x: Option<Pt> = None;
        for cmd in &page_auto.commands {
            if let Command::DrawString { text, x, .. } = cmd {
                if text == "R2" {
                    right_auto_x = Some(*x);
                }
            }
        }
        let right_auto_x = right_auto_x.expect("expected R2 draw command in auto layout");

        assert!(
            right_auto_x > right_fixed_x + Pt::from_f32(15.0),
            "expected fixed layout to keep the right column further left than auto layout, fixed={} auto={}",
            right_fixed_x.to_f32(),
            right_auto_x.to_f32()
        );
    }

    #[test]
    fn html_table_cell_height_sets_the_row_minimum() {
        let html = r#"
            <!doctype html>
            <html>
              <body>
                <table><tr><td></td></tr></table>
              </body>
            </html>
        "#;
        let css = r#"
            @page { size: 4in 4in; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            body { margin: 0; }
            table { border-collapse: collapse; width: 120px; }
            td { height: 60px; background: #a8dadc; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");

        let mut fill = Color::BLACK;
        let mut cell_height = None;
        for command in &page.commands {
            match command {
                Command::SetFillColor(color) => fill = *color,
                Command::DrawRect { height, .. }
                    if fill == Color::rgb(168.0 / 255.0, 218.0 / 255.0, 220.0 / 255.0) =>
                {
                    cell_height = Some(*height);
                    break;
                }
                _ => {}
            }
        }

        assert_eq!(cell_height, Some(Pt::from_f32(45.0)));
    }

    #[test]
    fn html_table_column_and_row_group_backgrounds_propagate_into_transparent_cells() {
        let html = r#"
            <!doctype html><html><body><table>
              <colgroup><col class="first"><col></colgroup>
              <tbody><tr><td></td><td></td></tr></tbody>
            </table></body></html>
        "#;
        let css = r#"
            @page { size: 4in 4in; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            table { border-collapse: separate; border-spacing: 0; }
            col.first { background: #e63946; }
            tbody { background: #2a9d8f; }
            td { width: 60px; height: 40px; background: transparent; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");
        let row_group = Color::rgb(42.0 / 255.0, 157.0 / 255.0, 143.0 / 255.0);
        let mut fill = Color::BLACK;
        let mut row_group_rects = 0usize;
        for command in &page.commands {
            match command {
                Command::SetFillColor(color) => fill = *color,
                Command::DrawRect { .. } if fill == row_group => row_group_rects += 1,
                _ => {}
            }
        }
        assert_eq!(row_group_rects, 2);
    }

    #[test]
    fn collapsed_table_appends_the_trailing_grid_borders() {
        let html = r#"
            <!doctype html>
            <html><body><table>
              <tr><td></td><td></td><td></td></tr>
              <tr><td></td><td></td><td></td></tr>
            </table></body></html>
        "#;
        let css = r#"
            @page { size: 5in 5in; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            table { border-collapse: collapse; table-layout: fixed; width: 360px; }
            td { width: 120px; height: 60px; border: 3px solid #1d3557; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");

        let border_color = Color::rgb(29.0 / 255.0, 53.0 / 255.0, 87.0 / 255.0);
        let mut fill = Color::BLACK;
        let mut right = Pt::ZERO;
        let mut bottom = Pt::ZERO;
        for command in &page.commands {
            match command {
                Command::SetFillColor(color) => fill = *color,
                Command::DrawRect {
                    x,
                    y,
                    width,
                    height,
                } if fill == border_color => {
                    right = right.max(*x + *width);
                    bottom = bottom.max(*y + *height);
                }
                _ => {}
            }
        }

        assert_eq!(right, Pt::from_f32(272.25));
        assert_eq!(bottom, Pt::from_f32(92.25));
    }

    #[test]
    fn html_table_rowspan_reserves_columns_and_spans_row_heights() {
        let html = r#"
            <!doctype html>
            <html><body><table>
              <tr><td class="span" rowspan="2"></td><td></td><td></td></tr>
              <tr><td></td><td></td></tr>
            </table></body></html>
        "#;
        let css = r#"
            @page { size: 5in 5in; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            table { border-collapse: collapse; table-layout: fixed; width: 300px; }
            td { width: 100px; height: 55px; border: 3px solid #003049; background: #d62828; }
            td.span { background: #fcbf49; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");

        let span_color = Color::rgb(252.0 / 255.0, 191.0 / 255.0, 73.0 / 255.0);
        let cell_color = Color::rgb(214.0 / 255.0, 40.0 / 255.0, 40.0 / 255.0);
        let mut fill = Color::BLACK;
        let mut span_rect = None;
        let mut second_row_cell = false;
        for command in &page.commands {
            match command {
                Command::SetFillColor(color) => fill = *color,
                Command::DrawRect {
                    x,
                    y,
                    width,
                    height,
                } if fill == span_color => span_rect = Some((*x, *y, *width, *height)),
                Command::DrawRect { x, y, .. }
                    if fill == cell_color
                        && *x == Pt::from_f32(77.25)
                        && *y == Pt::from_f32(43.5) =>
                {
                    second_row_cell = true;
                }
                _ => {}
            }
        }

        assert_eq!(
            span_rect,
            Some((Pt::ZERO, Pt::ZERO, Pt::from_f32(75.0), Pt::from_f32(82.5),))
        );
        assert!(
            second_row_cell,
            "the second row must start after the occupied rowspan column"
        );
    }

    #[test]
    fn separate_table_cell_transparent_border_keeps_geometry_without_paint() {
        let html = r#"<!doctype html><html><body><table><tr><td></td></tr></table></body></html>"#;
        let css = r#"
            @page { size: 120px 80px; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            html, body { background: #fff; }
            table { border-collapse: separate; border-spacing: 0; }
            td { width: 40px; height: 24px; border: 2px solid transparent; background: #eaf2f8; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");
        let background = Color::rgb(234.0 / 255.0, 242.0 / 255.0, 248.0 / 255.0);
        let mut fill = Color::BLACK;
        let mut black_rectangles = 0usize;
        let mut background_rectangles = 0usize;
        for command in &page.commands {
            match command {
                Command::SetFillColor(color) => fill = *color,
                Command::DrawRect { .. } if fill == Color::BLACK => black_rectangles += 1,
                Command::DrawRect { .. } if fill == background => background_rectangles += 1,
                _ => {}
            }
        }

        assert_eq!(black_rectangles, 0, "transparent borders must not paint");
        assert_eq!(background_rectangles, 1, "the cell box still participates");
    }

    #[test]
    fn table_cell_vertical_align_does_not_leak_into_anonymous_text() {
        let html = r#"
            <!doctype html><html><body>
              <table>
                <tr>
                  <td>alpha beta gamma<span class="suffix">!</span></td>
                </tr>
              </table>
            </body></html>
        "#;
        let css = r#"
            @page { size: 280px 136px; margin: 0; }
            * { box-sizing: border-box; margin: 0; }
            body { padding: 14px; color: #17202a; font: 16px/20px Inter; }
            table { width: auto; table-layout: auto; border-collapse: separate; border-spacing: 0; }
            td { padding: 7px 9px; border: 2px solid transparent; background: #eaf2f8; white-space: normal; vertical-align: middle; }
            .suffix { color: #b03a2e; }
        "#;
        let engine = FullBleed::builder()
            .register_font_file(repo_font_path("Inter-Variable.ttf"))
            .build()
            .expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        assert_eq!(doc.pages.len(), 1);
        let text_y = doc.pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::DrawString { text, y, .. }
                    if text == "alpha beta gamma" || text == "!" =>
                {
                    Some((text.as_str(), *y))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(text_y.len(), 2);
        assert_eq!(
            text_y[0].1, text_y[1].1,
            "table-cell vertical-align positions the cell contents, not descendant text runs"
        );
    }

    #[test]
    fn html_table_rowspan_minimum_is_distributed_across_rows() {
        let html = r#"
            <!doctype html>
            <html><body><table>
              <tr><td class="span" rowspan="2"></td><td class="peer"></td></tr>
              <tr><td class="peer"></td></tr>
            </table></body></html>
        "#;
        let css = r#"
            @page { size: 4in 4in; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            table { border-collapse: separate; border-spacing: 0; }
            td { width: 64px; height: 28px; border: 2px solid #111; }
            td.span { height: 100px; background: #e63946; }
            td.peer { background: #2a9d8f; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");

        let span_color = Color::rgb(230.0 / 255.0, 57.0 / 255.0, 70.0 / 255.0);
        let peer_color = Color::rgb(42.0 / 255.0, 157.0 / 255.0, 143.0 / 255.0);
        let mut fill = Color::BLACK;
        let mut span_height = None;
        let mut peer_rows = Vec::new();
        for command in &page.commands {
            match command {
                Command::SetFillColor(color) => fill = *color,
                Command::DrawRect { height, .. } if fill == span_color => {
                    span_height = Some(*height)
                }
                Command::DrawRect { y, .. } if fill == peer_color => peer_rows.push(*y),
                _ => {}
            }
        }

        assert_eq!(span_height, Some(Pt::from_f32(75.0)));
        assert!(peer_rows.contains(&Pt::ZERO));
        assert!(peer_rows.contains(&Pt::from_f32(37.5)));
    }

    #[test]
    fn fixed_table_slack_is_shared_by_first_row_cells_across_colspans() {
        let html = r#"
            <!doctype html>
            <html><body><table>
              <tr><td class="big" colspan="2" rowspan="2"></td><td></td></tr>
              <tr><td></td></tr>
              <tr><td></td><td></td><td></td></tr>
            </table></body></html>
        "#;
        let css = r#"
            @page { size: 5in 5in; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            table { border-collapse: collapse; table-layout: fixed; width: 360px; }
            td { width: 120px; height: 50px; border: 3px solid #1d3557; background: #a8dadc; }
            td.big { background: #e63946; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");

        let big_color = Color::rgb(230.0 / 255.0, 57.0 / 255.0, 70.0 / 255.0);
        let mut fill = Color::BLACK;
        let mut big_width = None;
        for command in &page.commands {
            match command {
                Command::SetFillColor(color) => fill = *color,
                Command::DrawRect { width, .. } if fill == big_color => {
                    big_width = Some(*width);
                    break;
                }
                _ => {}
            }
        }

        assert_eq!(big_width, Some(Pt::from_f32(133.875)));
    }

    #[test]
    fn html_auto_table_shrink_wraps_cells_and_distributes_table_height() {
        let html = r#"
            <!doctype html>
            <html><body><table><tr><td></td><td></td></tr></table></body></html>
        "#;
        let css = r#"
            @page { size: 4in 4in; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            table { height: 120px; border-collapse: separate; border-spacing: 0; }
            td { width: 70px; height: 30px; border: 2px solid #111; background: #2a9d8f; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");

        let cell_color = Color::rgb(42.0 / 255.0, 157.0 / 255.0, 143.0 / 255.0);
        let mut fill = Color::BLACK;
        let mut cells = Vec::new();
        for command in &page.commands {
            match command {
                Command::SetFillColor(color) => fill = *color,
                Command::DrawRect {
                    x,
                    y,
                    width,
                    height,
                } if fill == cell_color => cells.push((*x, *y, *width, *height)),
                _ => {}
            }
        }

        assert_eq!(
            cells,
            vec![
                (Pt::ZERO, Pt::ZERO, Pt::from_f32(52.5), Pt::from_f32(90.0),),
                (
                    Pt::from_f32(52.5),
                    Pt::ZERO,
                    Pt::from_f32(52.5),
                    Pt::from_f32(90.0),
                ),
            ]
        );
    }

    #[test]
    fn html_legacy_cellspacing_and_cellpadding_attributes_affect_the_grid() {
        let engine = FullBleed::builder().build().expect("engine");
        let spacing_doc = engine
            .render_to_document(
                r#"<!doctype html><html><body><table cellspacing="14"><tr><td class="a"></td><td class="b"></td></tr></table></body></html>"#,
                r#"
                    @page { size: 4in 4in; margin: 0; }
                    * { margin: 0; box-sizing: border-box; }
                    table { background: #111; }
                    td { width: 50px; height: 42px; padding: 0; border: 0; }
                    .a { background: #e63946; }
                    .b { background: #2a9d8f; }
                "#,
            )
            .expect("render cellspacing document");
        let page = spacing_doc.pages.first().expect("spacing page");
        let first_color = Color::rgb(230.0 / 255.0, 57.0 / 255.0, 70.0 / 255.0);
        let second_color = Color::rgb(42.0 / 255.0, 157.0 / 255.0, 143.0 / 255.0);
        let mut fill = Color::BLACK;
        let mut first_x = None;
        let mut second_x = None;
        for command in &page.commands {
            match command {
                Command::SetFillColor(color) => fill = *color,
                Command::DrawRect { x, .. } if fill == first_color => first_x = Some(*x),
                Command::DrawRect { x, .. } if fill == second_color => second_x = Some(*x),
                _ => {}
            }
        }
        assert_eq!(first_x, Some(Pt::from_f32(10.5)));
        assert_eq!(second_x, Some(Pt::from_f32(58.5)));

        let padding_doc = engine
            .render_to_document(
                r#"<!doctype html><html><body><table cellpadding="16"><tr><td><div class="mark"></div></td></tr></table></body></html>"#,
                r#"
                    @page { size: 4in 4in; margin: 0; }
                    * { margin: 0; box-sizing: border-box; }
                    table { border-collapse: separate; border-spacing: 0; }
                    td { width: 80px; height: 60px; border: 2px solid #111; }
                    .mark { width: 28px; height: 20px; background: #e63946; }
                "#,
            )
            .expect("render cellpadding document");
        let page = padding_doc.pages.first().expect("padding page");
        let mut fill = Color::BLACK;
        let mut marker_x = None;
        for command in &page.commands {
            match command {
                Command::SetFillColor(color) => fill = *color,
                Command::DrawRect { x, .. } if fill == first_color => marker_x = Some(*x),
                _ => {}
            }
        }
        assert_eq!(marker_x, Some(Pt::from_f32(13.5)));
    }

    #[test]
    fn html_legacy_border_attribute_builds_beveled_table_and_cell_edges() {
        let html = r#"<!doctype html><html><body><table border="4"><tr><td class="a"></td><td class="b"></td></tr></table></body></html>"#;
        let css = r#"
            @page { size: 4in 4in; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            td { width: 64px; height: 44px; }
            .a { background: #e63946; }
            .b { background: #2a9d8f; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");

        let light = Color::rgb(238.0 / 255.0, 238.0 / 255.0, 238.0 / 255.0);
        let dark = Color::rgb(154.0 / 255.0, 154.0 / 255.0, 154.0 / 255.0);
        let mut fill = Color::BLACK;
        let mut light_seen = false;
        let mut dark_seen = false;
        let mut right = Pt::ZERO;
        let mut bottom = Pt::ZERO;
        for command in &page.commands {
            match command {
                Command::SetFillColor(color) => fill = *color,
                Command::DrawRect {
                    x,
                    y,
                    width,
                    height,
                } => {
                    if fill == light {
                        light_seen = true;
                    }
                    if fill == dark {
                        dark_seen = true;
                    }
                    right = right.max(*x + *width);
                    bottom = bottom.max(*y + *height);
                }
                _ => {}
            }
        }
        assert!(light_seen && dark_seen);
        assert_eq!(right, Pt::from_f32(106.5));
        assert_eq!(bottom, Pt::from_f32(42.0));
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
    fn generated_counter_prefix_does_not_narrow_following_inline_text() {
        let html = r#"
            <!doctype html>
            <html><body>
              <div class="row">minus two</div>
              <div class="row">minus one</div>
              <div class="row">zero</div>
            </body></html>
        "#;
        let css = r#"
            @page { size: 336px 160px; margin: 0; }
            html { font-family: Inter; line-height: 1.5; }
            * { margin: 0; box-sizing: border-box; }
            @counter-style negwrap {
              system: extends decimal;
              negative: '(' ')';
              suffix: ' ';
            }
            body { padding: 12px; counter-reset: n -3; }
            .row {
              counter-increment: n;
              font-size: 22px;
              line-height: 32px;
              margin-bottom: 4px;
            }
            .row::before { content: counter(n, negwrap); font-weight: bold; }
        "#;
        let engine = FullBleed::builder()
            .register_font_file(repo_font_path("Inter-Variable.ttf"))
            .build()
            .expect("engine");
        let doc = engine
            .render_to_document(html, css)
            .expect("render document");
        let page = doc.pages.first().expect("page");
        let draws: Vec<_> = page
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::DrawString { text, x, y } => Some((text.as_str(), *x, *y)),
                _ => None,
            })
            .collect();
        assert!(
            draws.iter().any(|(text, _, _)| text.contains("minus two")),
            "the first row must remain a single inline run: {draws:?}"
        );
        assert!(
            draws.iter().any(|(text, _, _)| text.contains("minus one")),
            "the second row must remain a single inline run: {draws:?}"
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
    fn css_page_pseudo_classes_select_physical_margins_and_background() {
        let css = r#"
            @page { size: 192px 200px; margin: 16px; background: #ffd6d6; }
            @page :left { margin: 8px 32px 24px 8px; }
            @page :right { margin: 16px 8px 8px 32px; }
            @page :first { margin: 32px 8px 16px 24px; }
            html, body { background: #ffffff; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let templates = engine.resolve_page_templates_for_css(css, None);
        let frame_for = |selector| {
            templates
                .iter()
                .find(|template| template.page_selector() == selector)
                .expect("page selector template")
                .instantiate_frames()[0]
                .rect()
        };

        let first = frame_for(PageSelector::First);
        assert_eq!(first.x, Pt::from_f32(18.0));
        assert_eq!(first.y, Pt::from_f32(24.0));
        assert_eq!(first.width, Pt::from_f32(120.0));
        assert_eq!(first.height, Pt::from_f32(114.0));

        let left = frame_for(PageSelector::Left);
        assert_eq!(left.x, Pt::from_f32(6.0));
        assert_eq!(left.y, Pt::from_f32(6.0));
        assert_eq!(left.width, Pt::from_f32(114.0));
        assert_eq!(left.height, Pt::from_f32(126.0));

        let right = frame_for(PageSelector::Right);
        assert_eq!(right.x, Pt::from_f32(24.0));
        assert_eq!(right.y, Pt::from_f32(12.0));
        assert_eq!(right.width, Pt::from_f32(114.0));
        assert_eq!(right.height, Pt::from_f32(132.0));

        let html = r#"
            <div style="break-after: page">one</div>
            <div style="break-after: page">two</div>
            <div>three</div>
        "#;
        let doc = engine.render_to_document(html, css).expect("render");
        assert_eq!(doc.pages.len(), 3);
        let names: Vec<_> = doc
            .pages
            .iter()
            .map(|page| {
                page.commands
                    .iter()
                    .find_map(|command| match command {
                        Command::Meta { key, value } if key == META_PAGE_TEMPLATE_KEY => {
                            Some(value.as_str())
                        }
                        _ => None,
                    })
                    .expect("page template metadata")
            })
            .collect();
        assert_eq!(names, ["First", "Left", "Right"]);
        assert!(doc.pages.iter().all(|page| {
            page.commands.iter().any(|command| {
                matches!(
                    command,
                    Command::DrawRect { x, y, width, height }
                        if *x == Pt::ZERO
                            && *y == Pt::ZERO
                            && *width == Pt::from_f32(144.0)
                            && *height == Pt::from_f32(150.0)
                )
            })
        }));
        assert!(doc.pages[2].commands.iter().any(|command| {
            matches!(
                command,
                Command::DrawRect { x, y, width, height }
                    if *x == Pt::from_f32(24.0)
                        && *y == Pt::from_f32(12.0) + Pt::from_milli_i64(1)
                        && *width == Pt::from_f32(114.0)
                        && *height == Pt::from_f32(132.0) - Pt::from_milli_i64(1)
            )
        }));
    }

    #[test]
    fn generated_before_and_after_are_flex_items_in_source_order() {
        let html = "<html><body><div class='row'><span>M</span></div></body></html>";
        let css = r#"
            @page { size: 200px 100px; margin: 0; }
            body { margin: 0; }
            .row { display: flex; width: 120px; gap: 8px; }
            .row::before { content: 'L'; }
            .row::after { content: 'R'; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        let draws: Vec<_> = doc.pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::DrawString { text, x, .. } if matches!(text.as_str(), "L" | "M" | "R") => {
                    Some((text.as_str(), *x))
                }
                _ => None,
            })
            .collect();
        assert_eq!(draws.len(), 3, "expected both pseudo flex items: {draws:?}");
        assert_eq!(
            draws.iter().map(|(text, _)| *text).collect::<Vec<_>>(),
            ["L", "M", "R"]
        );
        assert!(draws[0].1 < draws[1].1 && draws[1].1 < draws[2].1);
    }

    #[test]
    fn generated_block_before_precedes_the_anonymous_body_lines() {
        let html = "<html><body><div class='card'>Body text follows the block header pseudo-element.</div></body></html>";
        let css = r#"
            @page { size: 432px 208px; margin: 0; }
            * { box-sizing: border-box; margin: 0; }
            .card {
                width: 300px; margin: 24px; padding: 12px;
                border: 2px solid #0b3954; background: #bfd7ea;
                font-size: 22px; line-height: 1.5; color: #0b3954;
            }
            .card::before {
                content: 'HEADER'; display: block; padding: 4px 8px;
                background: #ff6b6b; color: #ffffff;
            }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        let draws: Vec<_> = doc.pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::DrawString { text, x, y }
                    if text == "HEADER" || text.starts_with("Body text") =>
                {
                    Some((text.as_str(), *x, *y))
                }
                _ => None,
            })
            .collect();
        assert_eq!(draws.len(), 2, "expected pseudo and body text: {draws:?}");
        assert!(
            draws[0].0 == "HEADER" && draws[0].2 < draws[1].2,
            "block ::before must occupy flow before body text: {draws:?}"
        );
        assert_eq!(
            (draws[0].1, draws[0].2),
            (Pt::from_f32(34.5), Pt::from_f32(32.25)),
            "block pseudo text must use its unshifted padding-box content origin"
        );
    }

    #[test]
    fn generated_target_text_resolves_document_fragment_content() {
        let html = "<html><body><a href='#target'>Jump</a><div id='target'>Deep <span>Section</span></div></body></html>";
        let css = r#"
            @page { size: 344px 152px; margin: 0; }
            * { box-sizing: border-box; margin: 0; }
            body { padding: 12px; font-size: 20px; line-height: 30px; }
            a::after { content: 'REF ' target-text(attr(href)); }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        assert!(doc.pages[0].commands.iter().any(
            |command| matches!(command, Command::DrawString { text, .. } if text == "REF Deep Section")
        ));
    }

    #[test]
    fn generated_target_page_counter_recompiles_after_pagination() {
        let html = "<html><body><a href='#target'>See target</a><div id='target'>Target</div></body></html>";
        let css = r#"
            @page { size: 240px 120px; margin: 0; }
            * { box-sizing: border-box; margin: 0; }
            body { padding: 12px; font-size: 20px; line-height: 30px; }
            a::after { content: ' p.' target-counter(attr(href), page); }
            #target { break-before: page; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        assert_eq!(document_target_pages(&doc).get("target"), Some(&2));
        assert!(doc.pages[0].commands.iter().any(
            |command| matches!(command, Command::DrawString { text, .. } if text.contains("p.2"))
        ));
    }

    #[test]
    fn generated_content_url_emits_an_intrinsically_sized_inline_image() {
        let html = "<html><body><span class='icon'> label</span></body></html>";
        let css = r#"
            @page { size: 240px 100px; margin: 0; }
            * { box-sizing: border-box; margin: 0; }
            body { padding: 12px; font-size: 20px; line-height: 30px; }
            .icon::before {
                content: url("data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABgAAAAYCAIAAABvFaqvAAAAIklEQVQ4y2N8ZunGQA3ARBVTRg0aNWjUoFGDRg0aNYgiAAA0vAGVKP7aoAAAAABJRU5ErkJggg==");
                width: 40px;
                height: 40px;
            }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        let image = doc.pages[0]
            .commands
            .iter()
            .find_map(|command| match command {
                Command::DrawImage {
                    x, width, height, ..
                } if *width == Pt::from_f32(18.0) && *height == Pt::from_f32(18.0) => {
                    Some((*x, *width))
                }
                _ => None,
            })
            .expect("intrinsically sized generated image");
        let label_x = doc.pages[0]
            .commands
            .iter()
            .find_map(|command| match command {
                Command::DrawString { text, x, .. } if text == "label" => Some(*x),
                _ => None,
            })
            .expect("label following generated image");
        assert!(
            label_x > image.0 + image.1,
            "the collapsible DOM space after generated content must advance the label"
        );
    }

    #[test]
    fn generated_content_leader_expands_before_its_suffix() {
        let html = "<html><body><div class='toc'><a href='#s1'>Chapter title</a></div><span id='s1'></span></body></html>";
        let css = r#"
            @page { size: 330px 130px; margin: 0; }
            * { box-sizing: border-box; margin: 0; }
            body { padding: 12px; font-size: 20px; line-height: 30px; }
            .toc { width: 280px; }
            .toc a { display: block; }
            .toc a::after { content: leader('.') ' 7'; color: #d7263d; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        let draws = doc.pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::DrawString { text, x, .. } => Some((text.as_str(), *x)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let leader = draws
            .iter()
            .find(|(text, _)| !text.is_empty() && text.chars().all(|ch| ch == '.'))
            .unwrap_or_else(|| panic!("expanded leader run missing from {draws:?}"));
        let suffix = draws
            .iter()
            .find(|(text, _)| *text == "7")
            .expect("leader suffix");
        assert!(leader.1 < suffix.1);
        assert!(
            suffix.1 > Pt::from_f32(180.0),
            "suffix must reach the line end: {draws:?}"
        );
    }

    #[test]
    fn balanced_multicol_fragments_a_generated_fixed_height_child() {
        let html = "<html><body><div class='node outer multicol'><div class='own'>AgBb</div><div class='node inner generated'><div class='own'>AB</div></div></div></body></html>";
        let css = r#"
            @page { size: 180px 120px; margin: 0; }
            html { font-size: 16px; line-height: 1.2; }
            * { box-sizing: border-box; margin: 0; }
            body { margin: 0; }
            .node { padding: 7px; border: 2px solid #577590; }
            .outer { width: 126px; height: 96px; }
            .inner { width: 58px; height: 48px; padding: 5px; }
            .own { height: 22px; white-space: nowrap; }
            .generated::before { content: '‹'; }
            .generated::after { content: '›'; }
            .multicol { column-count: 2; column-gap: 7px; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        let draws = doc.pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::DrawString { text, x, y, .. }
                    if matches!(text.as_str(), "AgBb" | "AB" | "‹" | "›") =>
                {
                    Some((text.as_str(), *x, *y))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let position = |text: &str| {
            draws
                .iter()
                .find(|(candidate, _, _)| *candidate == text)
                .map(|(_, x, y)| (*x, *y))
                .unwrap_or_else(|| panic!("missing {text:?} in {draws:?}"))
        };
        let before = position("‹");
        let own = position("AgBb");
        let inner = position("AB");
        let after = position("›");
        assert!(before.0 > own.0 && before.0 < inner.0, "{draws:?}");
        assert_eq!(after.0, inner.0, "{draws:?}");
    }

    #[test]
    fn definite_flex_item_width_caps_its_automatic_minimum() {
        let html =
            "<html><body><div class='row'><div class='item'>123456789</div></div></body></html>";
        let css = r#"
            @page { size: 200px 100px; margin: 0; }
            * { box-sizing: border-box; }
            body { margin: 0; }
            .row { display: flex; width: 108px; }
            .item { width: 58px; padding: 5px; border: 2px solid #000; background: #e7f5ff; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        assert!(doc.pages[0].commands.iter().any(|command| {
            matches!(
                command,
                Command::DrawRect { width, .. } if *width == Pt::from_f32(43.5)
            )
        }));
    }

    #[test]
    fn margin_only_empty_block_survives_html_construction_and_collapses() {
        let html = "<html><body><div class='a'></div><div class='gap'></div><div class='b'></div></body></html>";
        let css = r#"
            @page { size: 320px 184px; margin: 0; }
            html, body { margin: 0; }
            .a, .b { width: 240px; height: 70px; }
            .a { background: #2d6cdf; }
            .gap { margin-top: 40px; margin-bottom: 40px; }
            .b { background: #d94f4f; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        let blue = Color::rgb(45.0 / 255.0, 108.0 / 255.0, 223.0 / 255.0);
        let red = Color::rgb(217.0 / 255.0, 79.0 / 255.0, 79.0 / 255.0);
        let mut fill = Color::BLACK;
        let mut blue_y = None;
        let mut red_y = None;
        for command in &doc.pages[0].commands {
            match command {
                Command::SetFillColor(color) => fill = *color,
                Command::DrawRect { y, .. } if fill == blue => blue_y = Some(*y),
                Command::DrawRect { y, .. } if fill == red => red_y = Some(*y),
                _ => {}
            }
        }
        assert_eq!(blue_y, Some(Pt::ZERO));
        assert_eq!(red_y, Some(Pt::from_f32(82.5)));
    }

    #[test]
    fn html_column_flex_basis_preserves_item_margins() {
        let html = "<html><body><div class='flex'><div class='a'></div><div class='b'></div></div></body></html>";
        let css = r#"
            @page { size: 320px 216px; margin: 0; }
            html, body { margin: 0; }
            .flex { display: flex; flex-direction: column; width: 240px; background: #cfd8e3; }
            .a { height: 70px; margin-bottom: 40px; background: #2d6cdf; }
            .b { height: 70px; margin-top: 30px; background: #d94f4f; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        let flex_fill = Color::rgb(207.0 / 255.0, 216.0 / 255.0, 227.0 / 255.0);
        let blue = Color::rgb(45.0 / 255.0, 108.0 / 255.0, 223.0 / 255.0);
        let red = Color::rgb(217.0 / 255.0, 79.0 / 255.0, 79.0 / 255.0);
        let mut fill = Color::BLACK;
        let mut rectangles = Vec::new();
        for command in &doc.pages[0].commands {
            match command {
                Command::SetFillColor(color) => fill = *color,
                Command::DrawRect { y, height, .. }
                    if fill == flex_fill || fill == blue || fill == red =>
                {
                    rectangles.push((fill, *y, *height));
                }
                _ => {}
            }
        }
        assert!(rectangles.contains(&(flex_fill, Pt::ZERO, Pt::from_f32(157.5))));
        assert!(rectangles.contains(&(blue, Pt::ZERO, Pt::from_f32(52.5))));
        assert!(rectangles.contains(&(red, Pt::from_f32(105.0), Pt::from_f32(52.5))));
    }

    #[test]
    fn display_contents_discards_its_box_paint_and_promotes_children() {
        let html = "<html><body><div class='wrapper'><div class='child'></div><div class='child'></div></div></body></html>";
        let css = r#"
            @page { size: 184px 120px; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            body { padding: 20px; }
            .wrapper { display: contents; background: #c62828; padding: 20px; }
            .child { width: 70px; height: 28px; background: #0a7f2e; margin-bottom: 8px; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        let discarded_red = Color::rgb(198.0 / 255.0, 40.0 / 255.0, 40.0 / 255.0);
        let green = Color::rgb(10.0 / 255.0, 127.0 / 255.0, 46.0 / 255.0);
        let mut fill = Color::BLACK;
        let mut green_rectangles = Vec::new();
        let mut painted_discarded_box = false;
        for command in &doc.pages[0].commands {
            match command {
                Command::SetFillColor(color) => fill = *color,
                Command::DrawRect {
                    x,
                    y,
                    width,
                    height,
                } if fill == green => {
                    green_rectangles.push((*x, *y, *width, *height));
                }
                Command::DrawRect { .. } if fill == discarded_red => {
                    painted_discarded_box = true;
                }
                _ => {}
            }
        }
        assert!(!painted_discarded_box);
        assert_eq!(
            green_rectangles,
            vec![
                (
                    Pt::from_f32(15.0),
                    Pt::from_f32(15.0),
                    Pt::from_f32(52.5),
                    Pt::from_f32(21.0),
                ),
                (
                    Pt::from_f32(15.0),
                    Pt::from_f32(42.0),
                    Pt::from_f32(52.5),
                    Pt::from_f32(21.0),
                ),
            ]
        );
    }

    #[test]
    fn intrinsic_keywords_constrain_authored_block_widths() {
        let engine = FullBleed::builder().build().expect("engine");
        let html = "<html><body><div class='wrap'><div class='box'>alpha betabetabeta gamma</div></div></body></html>";
        let css = r#"
            @page { size: 264px 112px; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            .wrap { width: 220px; }
            .box { width: 200px; max-width: min-content; font-family: ParitySans; font-size: 16px; line-height: 24px; background: #0a7f2e; }
        "#;
        let min_content = engine
            .render_to_document(html, css)
            .expect("render min-content");
        let green = Color::rgb(10.0 / 255.0, 127.0 / 255.0, 46.0 / 255.0);
        let green_width = |document: &Document| {
            let mut fill = Color::BLACK;
            document.pages[0]
                .commands
                .iter()
                .find_map(|command| match command {
                    Command::SetFillColor(color) => {
                        fill = *color;
                        None
                    }
                    Command::DrawRect { width, .. } if fill == green => Some(*width),
                    _ => None,
                })
        };
        let min_content_width = green_width(&min_content).expect("min-content background");
        assert!(min_content_width > Pt::ZERO);
        assert!(min_content_width < Pt::from_f32(150.0));

        let html = "<html><body><div class='wrap'><div class='box'>unbreakablewideword</div></div></body></html>";
        let css = r#"
            @page { size: 264px 80px; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            .wrap { width: 220px; }
            .box { width: 70px; min-width: max-content; white-space: nowrap; font-family: ParitySans; font-size: 16px; line-height: 30px; background: #0a7f2e; }
        "#;
        let max_content = engine
            .render_to_document(html, css)
            .expect("render max-content");
        assert!(green_width(&max_content).expect("max-content background") > Pt::from_f32(52.5));
    }

    #[test]
    fn zero_font_size_inline_blocks_wrap_without_a_parent_strut_gap() {
        let html = "<html><body><div class='container'><span class='chip'></span><span class='chip'></span><span class='chip'></span><span class='chip'></span><span class='chip'></span></div></body></html>";
        let css = r#"
            @page { size: 400px 184px; margin: 0; }
            html, body { margin: 0; }
            .container { width: 320px; font-size: 0; background: #e8eef6; }
            .chip { display: inline-block; width: 120px; height: 60px; background: #2d6cdf; }
            .chip:nth-child(even) { background: #d94f4f; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        let blue = Color::rgb(45.0 / 255.0, 108.0 / 255.0, 223.0 / 255.0);
        let red = Color::rgb(217.0 / 255.0, 79.0 / 255.0, 79.0 / 255.0);
        let mut fill = Color::BLACK;
        let mut blue_y = Vec::new();
        let mut red_y = Vec::new();
        for command in &doc.pages[0].commands {
            match command {
                Command::SetFillColor(color) => fill = *color,
                Command::DrawRect { y, .. } if fill == blue => blue_y.push(*y),
                Command::DrawRect { y, .. } if fill == red => red_y.push(*y),
                _ => {}
            }
        }
        assert_eq!(
            blue_y,
            vec![Pt::ZERO, Pt::from_f32(45.0), Pt::from_f32(90.0)]
        );
        assert_eq!(red_y, vec![Pt::ZERO, Pt::from_f32(45.0)]);
    }

    #[test]
    fn inside_disc_marker_uses_native_shape_and_css_marker_advance() {
        let html = "<html><body><div class='item'>mark</div></body></html>";
        let css = r#"
            @page { size: 200px 96px; margin: 0; }
            * { margin: 0; box-sizing: border-box; }
            .item { display: list-item; list-style: inside disc; width: 150px; height: 48px; font-size: 30px; line-height: 48px; color: #0a7f2e; }
        "#;
        let engine = FullBleed::builder().build().expect("engine");
        let doc = engine.render_to_document(html, css).expect("render");
        let text_positions: Vec<Pt> = doc.pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::DrawString { text, x, .. } if text == "mark" => Some(*x),
                _ => None,
            })
            .collect();
        assert_eq!(text_positions, vec![Pt::from_f32(30.0)]);
        assert!(
            !doc.pages[0].commands.iter().any(|command| matches!(
                command,
                Command::DrawString { text, .. } if text.contains('\u{2022}')
            )),
            "native bullets must not be serialized as font glyphs"
        );
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
            assert!(matches!(result, AddResult::Placed(_)));
        }

        let result = frame.add(Box::new(Spacer::new_pt(Pt::from_f32(5.0))), &mut canvas);
        assert!(matches!(result, AddResult::Overflow(_, _)));
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
            AddResult::Split(rest, _) => rest,
            other => panic!(
                "expected split, got variant {:?}",
                std::mem::discriminant(&other)
            ),
        };

        canvas.show_page();
        let mut frame2 = Frame::new(frame_rect);
        let result2 = frame2.add(second, &mut canvas);
        assert!(matches!(
            result2,
            AddResult::Placed(_) | AddResult::Split(_, _)
        ));

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
            AddResult::Split(rest, _) => rest,
            other => panic!(
                "expected split, got variant {:?}",
                std::mem::discriminant(&other)
            ),
        };

        canvas.show_page();
        let mut frame2 = Frame::new(frame_rect);
        let result2 = frame2.add(second, &mut canvas);
        assert!(matches!(
            result2,
            AddResult::Placed(_) | AddResult::Split(_, _)
        ));

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
        assert!(matches!(result, AddResult::Placed(_)));

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
    fn collapsed_vertical_winner_paints_after_the_adjacent_cell_junctions() {
        let teal = Color::rgb(42.0 / 255.0, 157.0 / 255.0, 143.0 / 255.0);
        let blue = Color::rgb(69.0 / 255.0, 123.0 / 255.0, 157.0 / 255.0);
        let widths = EdgeSizes {
            top: abs(10.0),
            right: abs(10.0),
            bottom: abs(10.0),
            left: abs(10.0),
        };
        let left = table_cell_with_border("", widths, teal);
        let right = table_cell_with_border("", widths, blue);
        let table = TableFlowable::new(vec![vec![left, right]])
            .with_border_collapse(BorderCollapseMode::Collapse)
            .with_table_layout(TableLayoutMode::Fixed);
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

        assert!(matches!(
            frame.add(Box::new(table), &mut canvas),
            AddResult::Placed(_)
        ));
        let document = canvas.finish();
        let border_inks = document.pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::SetFillColor(color) if *color == teal || *color == blue => Some(*color),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            border_inks.last(),
            Some(&teal),
            "the LTR shared vertical winner must cover the later blue cell's two junctions"
        );
        assert!(
            document.pages[0]
                .commands
                .iter()
                .all(|command| !matches!(command, Command::ClipPath { .. })),
            "solid collapsed-border winners must meet as square segments without miter clips"
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
    fn template_binding_honors_page_break_inside_transparent_div_wrapper() {
        let html = r#"
<!doctype html>
<html>
  <body>
    <div class="ui-stack">
      <section>
        <div data-fb="fb.feature.red=1"></div>
        <p>Page one marker</p>
      </section>
      <section style="page-break-before: always;">
        <div data-fb="fb.feature.green=1"></div>
        <p>Page two marker</p>
      </section>
    </div>
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
    fn compiled_document_skips_frontend_and_links_deterministically() {
        let engine = FullBleed::builder().build().expect("engine");
        let html = "<!doctype html><html><body><h1>Compiled</h1><p>fixed point</p></body></html>";
        let css = "@page { size: letter; margin: 0.5in; } body { margin: 0; font-size: 12pt; }";
        let expected = engine.render_to_buffer(html, css).expect("ordinary render");
        let compiled = engine
            .compile_document(html, css)
            .expect("compile document");

        assert_eq!(compiled.page_count(), 1);
        assert!(compiled.command_count() > 0);
        let first = compiled.render_to_buffer().expect("compiled render");
        let second = compiled.render_to_buffer().expect("compiled rerender");
        assert_eq!(first, expected);
        assert_eq!(second, expected);

        let batch = compiled.render_many_to_buffer(3).expect("compiled batch");
        let inspection = inspect_pdf_bytes(&batch).expect("inspect compiled batch");
        assert_eq!(inspection.page_count, 3);
        let parsed = crate::pdf_native::Document::load_mem(&batch).expect("parse compiled batch");
        let content_ids: std::collections::BTreeSet<_> = parsed
            .get_pages()
            .values()
            .map(|page_id| {
                parsed
                    .get_object(*page_id)
                    .and_then(crate::pdf_native::Object::as_dict)
                    .and_then(|page| page.get(b"Contents"))
                    .and_then(crate::pdf_native::Object::as_reference)
                    .expect("page content reference")
            })
            .collect();
        assert_eq!(content_ids.len(), 1, "compiled copies share one stream");
        assert!(matches!(
            compiled.render_many_to_buffer(0),
            Err(FullBleedError::EmptyDocumentSet)
        ));
    }

    #[test]
    fn compiled_document_binds_distinct_columnar_records_without_relayout() {
        let engine = FullBleed::builder().build().expect("engine");
        let html = r#"<!doctype html>
<html><body>
  <main class="invoice">
    <h1>Invoice</h1>
    <p class="static-copy">Compiled fixed geometry</p>
    <p><span>Invoice: </span><span>{{invoice_id}}</span></p>
    <p><span>Customer: </span><span>{{customer}}</span></p>
    <p><span>Amount: </span><span>{{amount}}</span></p>
  </main>
</body></html>"#;
        let css = r#"
@page { size: letter; margin: 0.5in; }
body { margin: 0; font-family: Helvetica, sans-serif; font-size: 12pt; }
h1 { font-size: 20pt; }
"#;
        let compiled = engine
            .compile_document(html, css)
            .expect("compile binding template");
        assert_eq!(
            compiled.binding_slots(),
            &[
                "amount".to_string(),
                "customer".to_string(),
                "invoice_id".to_string()
            ]
        );
        assert_eq!(compiled.binding_program_page_count(), 1);
        assert!(compiled.binding_program_command_count() > 0);

        let bindings = std::collections::HashMap::from([
            (
                "invoice_id".to_string(),
                vec![
                    "INV-0001".to_string(),
                    "INV-0002".to_string(),
                    "INV(0003)".to_string(),
                ],
            ),
            (
                "customer".to_string(),
                vec![
                    "Ada Lovelace".to_string(),
                    "Grace Hopper".to_string(),
                    "Katherine Johnson".to_string(),
                ],
            ),
            (
                "amount".to_string(),
                vec![
                    "$101.25".to_string(),
                    "$202.50".to_string(),
                    "$303.75".to_string(),
                ],
            ),
        ]);
        let batch = compiled
            .render_bindings_to_buffer(&bindings)
            .expect("render distinct bindings");
        let inspection = inspect_pdf_bytes(&batch).expect("inspect binding batch");
        assert_eq!(inspection.page_count, 3);

        let parsed = crate::pdf_native::Document::load_mem(&batch).expect("parse binding batch");
        let expected = [
            (
                b"INV-0001".as_slice(),
                b"Ada Lovelace".as_slice(),
                b"$101.25".as_slice(),
            ),
            (
                b"INV-0002".as_slice(),
                b"Grace Hopper".as_slice(),
                b"$202.50".as_slice(),
            ),
            (
                b"INV\\(0003\\)".as_slice(),
                b"Katherine Johnson".as_slice(),
                b"$303.75".as_slice(),
            ),
        ];
        for ((page_number, page_id), (invoice, customer, amount)) in
            parsed.get_pages().into_iter().zip(expected)
        {
            let page = parsed
                .get_object(page_id)
                .and_then(crate::pdf_native::Object::as_dict)
                .expect("page dictionary");
            assert_eq!(
                page.get(b"Contents")
                    .and_then(crate::pdf_native::Object::as_array)
                    .expect("static plus dynamic content array")
                    .len(),
                2
            );
            let content = parsed
                .get_page_content(page_id)
                .expect("combined page content");
            assert!(
                content
                    .windows(invoice.len())
                    .any(|window| window == invoice),
                "page {page_number} is missing its distinct invoice id"
            );
            assert!(
                content
                    .windows(customer.len())
                    .any(|window| window == customer)
            );
            assert!(content.windows(amount.len()).any(|window| window == amount));
            assert!(
                content
                    .windows(23)
                    .any(|window| window == b"Compiled fixed geometry")
            );
            assert!(!content.windows(2).any(|window| window == b"{{"));
        }

        let missing = std::collections::HashMap::from([(
            "invoice_id".to_string(),
            vec!["INV-ONLY".to_string()],
        )]);
        assert!(matches!(
            compiled.render_bindings_to_buffer(&missing),
            Err(FullBleedError::InvalidConfiguration(_))
        ));

        let uneven = std::collections::HashMap::from([
            (
                "invoice_id".to_string(),
                vec!["A".to_string(), "B".to_string()],
            ),
            ("customer".to_string(), vec!["C".to_string()]),
            ("amount".to_string(), vec!["1".to_string(), "2".to_string()]),
        ]);
        assert!(matches!(
            compiled.render_bindings_to_buffer(&uneven),
            Err(FullBleedError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn batch_rendering_apis_honor_pre_cancelled_tokens() {
        let engine = FullBleed::builder().build().expect("engine");
        let compiled = engine
            .compile_document(
                "<main><h1>{{name}}</h1><p>Cancellation contract</p></main>",
                "@page { size: 180pt 120pt; margin: 12pt; }",
            )
            .expect("compile cancellable template");
        let bindings = std::collections::HashMap::from([(
            "name".to_string(),
            vec!["Ada".to_string(), "Grace".to_string()],
        )]);
        let cancellation = AuthoringCancellationToken::new();
        cancellation.cancel();

        assert!(matches!(
            compiled.render_bindings_to_buffer_cancellable(&bindings, &cancellation),
            Err(FullBleedError::Cancelled)
        ));
        assert!(matches!(
            compiled.render_reflow_bindings_to_buffer_with_options_cancellable(
                &bindings,
                CompiledReflowOptions::default(),
                &cancellation,
            ),
            Err(FullBleedError::Cancelled)
        ));
        assert!(matches!(
            engine.render_many_to_buffer_parallel_cancellable(
                &["<main>One</main>".to_string()],
                "",
                &cancellation,
            ),
            Err(FullBleedError::Cancelled)
        ));
    }

    #[test]
    fn compiled_document_binds_registered_identity_h_fonts_without_literal_slots() {
        let mut bundle = AssetBundle::default();
        bundle.add(Asset::new(
            "VdpCustom".to_string(),
            AssetKind::Font,
            std::fs::read(repo_font_path("NotoSans-Regular.ttf")).expect("read custom font"),
            None,
            true,
        ));
        let engine = FullBleed::builder()
            .register_bundle(bundle)
            .build()
            .expect("engine");
        let compiled = engine
            .compile_document(
                "<main><h1>{{attendee}}</h1><p>{{role}}</p><strong>{{seat}}</strong></main>",
                "@page { size: 260pt 140pt; margin: 12pt; } body { margin: 0; font-family: VdpCustom; font-weight: 400; } h1 { font-size: 20pt; font-weight: 700; }",
            )
            .expect("compile registered-font template");
        let bindings = std::collections::HashMap::from([
            (
                "attendee".to_string(),
                vec!["ADA RIVERA".to_string(), "GRACE HOPPER".to_string()],
            ),
            (
                "role".to_string(),
                vec!["RESEARCHER".to_string(), "ENGINEER".to_string()],
            ),
            (
                "seat".to_string(),
                vec!["A12".to_string(), "B07".to_string()],
            ),
        ]);
        let pdf = compiled
            .render_bindings_to_buffer(&bindings)
            .expect("registered-font fixed bindings");
        let inspection = inspect_pdf_bytes(&pdf).expect("inspect binding output");
        assert_eq!(inspection.page_count, 2);

        let parsed = crate::pdf_native::Document::load_mem(&pdf).expect("parse binding PDF");
        let page_numbers = parsed.get_pages().keys().copied().collect::<Vec<_>>();
        let text = parsed
            .extract_text_chunks(&page_numbers)
            .into_iter()
            .collect::<crate::pdf_native::Result<Vec<_>>>()
            .expect("extract registered-font bindings")
            .join("\n");
        assert!(text.contains("ADA RIVERA"));
        assert!(text.contains("GRACE HOPPER"));
        assert!(text.contains("RESEARCHER"));
        assert!(text.contains("ENGINEER"));
        assert!(text.contains("A12"));
        assert!(text.contains("B07"));
        assert!(!text.contains("{{"));
    }

    #[test]
    fn compiled_reflow_bindings_reuse_dom_and_generate_variable_page_counts() {
        let engine = FullBleed::builder().build().expect("engine");
        let template = r#"<!doctype html><html><body>
<main>
  <h1>{{record_id}}</h1>
  <div class="content">{{content}}</div>
  <p class="tail">END {{record_id}}</p>
</main>
</body></html>"#;
        let css = r#"
@page { size: 240pt 180pt; margin: 18pt; }
body { margin: 0; font-family: Helvetica, sans-serif; font-size: 10pt; line-height: 12pt; }
h1 { margin: 0 0 6pt; font-size: 14pt; line-height: 16pt; }
.content { white-space: pre-wrap; }
.tail { margin: 6pt 0 0; }
"#;
        let compiled = engine
            .compile_document(template, css)
            .expect("compile reflow template");
        assert!(compiled.reflow_program_ready());
        assert_eq!(
            compiled.reflow_binding_slots(),
            &["content".to_string(), "record_id".to_string()]
        );
        assert!(compiled.reflow_program_node_count() > 5);
        assert_eq!(compiled.reflow_program_binding_text_node_count(), 3);
        assert_eq!(compiled.reflow_program_error(), None);

        let lines = |prefix: &str, count: usize| {
            (0..count)
                .map(|index| format!("{prefix} marker {index:03}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let bindings = std::collections::HashMap::from([
            (
                "record_id".to_string(),
                vec![
                    "REC-A".to_string(),
                    "REC-B".to_string(),
                    "REC-C".to_string(),
                ],
            ),
            (
                "content".to_string(),
                vec![lines("alpha", 3), lines("bravo", 18), lines("charlie", 35)],
            ),
        ]);

        let first = compiled
            .render_reflow_bindings_to_buffer(&bindings)
            .expect("compiled reflow batch");
        let second = compiled
            .render_reflow_bindings_to_buffer(&bindings)
            .expect("deterministic compiled reflow batch");
        assert_eq!(first, second, "compiled reflow must be byte deterministic");
        let explicit_throughput = compiled
            .render_reflow_bindings_to_buffer_with_options(
                &bindings,
                CompiledReflowOptions {
                    compression: CompiledFlowCompression::Throughput,
                },
            )
            .expect("explicit throughput reflow batch");
        assert_eq!(first, explicit_throughput);
        let compact = compiled
            .render_reflow_bindings_to_buffer_with_options(
                &bindings,
                CompiledReflowOptions {
                    compression: CompiledFlowCompression::Compact,
                },
            )
            .expect("compact reflow batch");
        assert_eq!(
            inspect_pdf_bytes(&compact)
                .expect("inspect compact batch")
                .page_count,
            inspect_pdf_bytes(&first)
                .expect("inspect throughput batch")
                .page_count,
        );
        assert!(compact.len() <= first.len());
        assert_eq!(
            compiled
                .render_reflow_bindings_to_buffer_with_options(
                    &bindings,
                    CompiledReflowOptions {
                        compression: CompiledFlowCompression::Throughput,
                    },
                )
                .expect("throughput batch after compact batch"),
            first,
            "compression policy must be isolated to one render call",
        );
        let inspection = inspect_pdf_bytes(&first).expect("inspect compiled reflow batch");
        assert!(
            inspection.page_count >= 6,
            "three records should naturally expand beyond one page each: {:?}",
            inspection.page_count
        );

        let parsed = crate::pdf_native::Document::load_mem(&first).expect("parse reflow PDF");
        let page_numbers = parsed.get_pages().keys().copied().collect::<Vec<_>>();
        let text = parsed
            .extract_text_chunks(&page_numbers)
            .into_iter()
            .collect::<crate::pdf_native::Result<Vec<_>>>()
            .expect("extract compiled reflow text")
            .join("\n");
        let a = text.find("REC-A").expect("record A marker");
        let b = text.find("REC-B").expect("record B marker");
        let c = text.find("REC-C").expect("record C marker");
        assert!(a < b && b < c, "record order must survive parallel reflow");
        assert!(text.contains("alpha marker 002"));
        assert!(text.contains("bravo marker 017"));
        assert!(text.contains("charlie marker 034"));

        let one_record = std::collections::HashMap::from([
            ("record_id".to_string(), vec!["REC <1> & final".to_string()]),
            ("content".to_string(), vec![lines("single", 14)]),
        ]);
        let compiled_one = compiled
            .render_reflow_bindings_to_buffer(&one_record)
            .expect("single compiled reflow record");
        let ordinary_html = template
            .replace("{{record_id}}", &escape_html_text("REC <1> & final"))
            .replace("{{content}}", &escape_html_text(&lines("single", 14)));
        let ordinary = engine
            .render_to_buffer(&ordinary_html, css)
            .expect("ordinary equivalent reflow");
        assert_eq!(
            compiled_one, ordinary,
            "compiled reflow must be byte-identical to the ordinary rendering path"
        );
    }

    #[test]
    fn compiled_reflow_constraint_reexecutes_the_browser_text_paint_phase() {
        let constraint = CompiledTextConstraint::parse("563:100000:30000:100000:30000:0:0:1:r")
            .expect("compiled text constraint");

        assert_eq!(
            constraint.aligned_x(Pt::from_milli_i64(100_000)),
            Pt::from_milli_i64(538),
            "the prototype phase must receive the browser's 0.025pt hint correction",
        );
        assert_eq!(
            constraint.aligned_x(Pt::from_milli_i64(99_500)),
            Pt::from_milli_i64(1_063),
            "a width change must execute the shader at its new phase, not move the snapped prototype",
        );

        let unsnapped = CompiledTextConstraint::parse("563:100000:30000:100000:30000:0:0:0:r")
            .expect("unsnapped compiled text constraint");
        assert_eq!(
            unsnapped.aligned_x(Pt::from_milli_i64(100_000)),
            Pt::from_milli_i64(563),
        );
    }

    #[test]
    fn compiled_reflow_materializes_trusted_html_fragments_in_container_context() {
        let engine = FullBleed::builder().build().expect("engine");
        let template = r#"<!doctype html><html><body>
<main>
  <h1>{{record_id}}</h1>
  <section class="narrative" data-fb-bind-html="sections"></section>
  <table><thead><tr><th>Item</th><th>Value</th></tr></thead>
    <tbody data-fb-bind-html="rows"></tbody>
  </table>
  <p>END {{record_id}}</p>
</main>
</body></html>"#;
        let css = r#"
@page { size: 260pt 190pt; margin: 16pt; }
body { margin: 0; font-family: Helvetica, sans-serif; font-size: 9pt; line-height: 11pt; }
h1, p { margin: 0 0 5pt; }
table { width: 100%; border-collapse: collapse; }
th, td { border: 1pt solid #999; padding: 3pt; }
thead { display: table-header-group; }
tr { break-inside: avoid; }
"#;
        let compiled = engine
            .compile_document(template, css)
            .expect("compile structural reflow template");
        assert_eq!(compiled.reflow_program_html_binding_node_count(), 2);
        assert_eq!(
            compiled.reflow_binding_slots(),
            &[
                "record_id".to_string(),
                "rows".to_string(),
                "sections".to_string()
            ]
        );

        let sections = |record: &str, count: usize| {
            (0..count)
                .map(|index| format!("<p>{record}-P-{index:03} structural narrative</p>"))
                .collect::<String>()
        };
        let rows = |record: &str, count: usize| {
            (0..count)
                .map(|index| {
                    format!(
                        "<tr><td>{record}-R-{index:03}</td><td>{}</td></tr>",
                        index * 17
                    )
                })
                .collect::<String>()
        };
        let bindings = std::collections::HashMap::from([
            (
                "record_id".to_string(),
                vec!["STRUCT-A".to_string(), "STRUCT-B".to_string()],
            ),
            (
                "sections".to_string(),
                vec![sections("STRUCT-A", 2), sections("STRUCT-B", 14)],
            ),
            (
                "rows".to_string(),
                vec![rows("STRUCT-A", 3), rows("STRUCT-B", 28)],
            ),
        ]);
        let batch = compiled
            .render_reflow_bindings_to_buffer(&bindings)
            .expect("render structural reflow batch");
        let inspection = inspect_pdf_bytes(&batch).expect("inspect structural reflow batch");
        assert!(inspection.page_count >= 4);
        let parsed = crate::pdf_native::Document::load_mem(&batch).expect("parse structural PDF");
        let page_numbers = parsed.get_pages().keys().copied().collect::<Vec<_>>();
        let text = parsed
            .extract_text_chunks(&page_numbers)
            .into_iter()
            .collect::<crate::pdf_native::Result<Vec<_>>>()
            .expect("extract structural reflow text")
            .join("\n");
        assert!(text.find("STRUCT-A").unwrap() < text.find("STRUCT-B").unwrap());
        assert!(text.contains("STRUCT-A-R-002"));
        assert!(text.contains("STRUCT-B-P-013"));
        assert!(text.contains("STRUCT-B-R-027"));

        let record_id = "STRUCT-ONE";
        let one_sections = sections(record_id, 7);
        let one_rows = rows(record_id, 12);
        let one = std::collections::HashMap::from([
            ("record_id".to_string(), vec![record_id.to_string()]),
            ("sections".to_string(), vec![one_sections.clone()]),
            ("rows".to_string(), vec![one_rows.clone()]),
        ]);
        let compiled_one = compiled
            .render_reflow_bindings_to_buffer(&one)
            .expect("single structural reflow record");
        let ordinary_html = template
            .replace("{{record_id}}", record_id)
            .replace(
                "<section class=\"narrative\" data-fb-bind-html=\"sections\"></section>",
                &format!(
                    "<section class=\"narrative\" data-fb-bind-html=\"sections\">{one_sections}</section>"
                ),
            )
            .replace(
                "<tbody data-fb-bind-html=\"rows\"></tbody>",
                &format!("<tbody data-fb-bind-html=\"rows\">{one_rows}</tbody>"),
            );
        let ordinary = engine
            .render_to_buffer(&ordinary_html, css)
            .expect("ordinary structural equivalent");
        assert_eq!(compiled_one, ordinary);
    }

    #[test]
    fn compiled_reflow_rejects_non_text_binding_contexts_without_breaking_fixed_compile() {
        let engine = FullBleed::builder().build().expect("engine");
        let compiled = engine
            .compile_document(
                r#"<main data-account="{{account}}"><p>Static body</p></main>"#,
                "",
            )
            .expect("fixed document still compiles");
        assert!(!compiled.reflow_program_ready());
        assert!(
            compiled
                .reflow_program_error()
                .is_some_and(|error| error.contains("text nodes only"))
        );
        let bindings =
            std::collections::HashMap::from([("account".to_string(), vec!["ACCT-1".to_string()])]);
        assert!(matches!(
            compiled.render_reflow_bindings_to_buffer(&bindings),
            Err(FullBleedError::InvalidConfiguration(message))
                if message.contains("text nodes only")
        ));

        let outside_body = engine
            .compile_document(
                r#"<html><head><title data-fb-bind-html="title"></title></head><body><p>Static</p></body></html>"#,
                "",
            )
            .expect("fixed document with a head-only structural marker still compiles");
        assert!(
            outside_body
                .reflow_program_error()
                .is_some_and(|error| error.contains("inside the document body"))
        );
    }

    #[test]
    fn compiled_reflow_cancels_a_windowed_batch_without_worker_deadlock() {
        let engine = FullBleed::builder().build().expect("engine");
        let compiled = engine
            .compile_document(
                r#"<main><h1>{{record_id}}</h1><section data-fb-bind-html="body"></section></main>"#,
                "body { font: 9pt Helvetica, sans-serif; }",
            )
            .expect("compile windowed reflow template");
        let record_count = 300usize;
        let record_ids = (0..record_count)
            .map(|index| format!("WINDOW-{index:03}"))
            .collect::<Vec<_>>();
        let mut bodies = record_ids
            .iter()
            .map(|record| format!("<p>{record} body</p>"))
            .collect::<Vec<_>>();
        bodies[200] =
            r#"<p data-fullbleed-compiler-binding-root="999">reserved attribute</p>"#.to_string();
        let bindings = std::collections::HashMap::from([
            ("record_id".to_string(), record_ids),
            ("body".to_string(), bodies),
        ]);

        assert!(matches!(
            compiled.render_reflow_bindings_to_buffer(&bindings),
            Err(FullBleedError::InvalidConfiguration(message))
                if message.contains("reserved data-fullbleed-compiler-binding-root attribute")
        ));
    }

    #[test]
    fn compiled_document_bindings_virtualize_transform_and_clip_state() {
        let engine = FullBleed::builder().build().expect("engine");
        let template = r#"<!doctype html><html><body>
<main>
  <div class="card"><span>{{account}}</span></div>
  <p class="plain"><span>{{region}}</span></p>
</main>
</body></html>"#;
        let css = r#"
@page { size: 300px 180px; margin: 10px; }
body { margin: 0; font-family: Helvetica, sans-serif; font-size: 16px; }
.card {
  width: 160px;
  height: 36px;
  overflow: hidden;
  transform: translate(18px, 9px) rotate(3deg);
  background: #dcecff;
}
.plain { margin-top: 24px; }
"#;
        let compiled = engine
            .compile_document(template, css)
            .expect("compile transformed binding template");
        assert_eq!(
            compiled.binding_slots(),
            &["account".to_string(), "region".to_string()]
        );
        assert_eq!(compiled.binding_program_page_count(), 1);

        let bindings = std::collections::HashMap::from([
            ("account".to_string(), vec!["ACCT-001".to_string()]),
            ("region".to_string(), vec!["North".to_string()]),
        ]);
        let bound = compiled
            .render_bindings_to_buffer(&bindings)
            .expect("render transformed binding overlay");
        let fixed = engine
            .render_to_buffer(
                &template
                    .replace("{{account}}", "ACCT-001")
                    .replace("{{region}}", "North"),
                css,
            )
            .expect("render equivalent fixed document");

        let parsed = crate::pdf_native::Document::load_mem(&bound).expect("parse binding PDF");
        let page_id = *parsed.get_pages().values().next().expect("bound page");
        let content = parsed
            .get_page_content(page_id)
            .expect("combined bound page content");
        let account_offset = content
            .windows(b"ACCT-001".len())
            .position(|window| window == b"ACCT-001")
            .expect("bound account text");
        let account_program = &content[..account_offset];
        assert!(
            account_program.windows(4).any(|window| window == b" cm\n"),
            "the dynamic account program should replay its compiled transform"
        );
        assert!(
            account_program.windows(4).any(|window| window == b"W\nn\n"),
            "the dynamic account program should replay its compiled clip"
        );

        let bound_pages = crate::pdf_raster::pdf_bytes_to_png_pages(&bound, 144, None, false)
            .expect("raster bound PDF");
        let fixed_pages = crate::pdf_raster::pdf_bytes_to_png_pages(&fixed, 144, None, false)
            .expect("raster fixed PDF");
        assert_eq!(
            bound_pages, fixed_pages,
            "compiled transform/clip replay must be pixel-identical to ordinary fixed paint"
        );
    }

    #[test]
    fn compiled_tagged_copies_keep_page_specific_content_streams() {
        let engine = FullBleed::builder()
            .pdf_profile(PdfProfile::Tagged)
            .build()
            .expect("tagged engine");
        let compiled = engine
            .compile_document("<main><p>Tagged copy</p></main>", "")
            .expect("compile tagged document");
        let batch = compiled.render_many_to_buffer(2).expect("tagged batch");
        let parsed = crate::pdf_native::Document::load_mem(&batch).expect("parse tagged batch");
        let content_ids: std::collections::BTreeSet<_> = parsed
            .get_pages()
            .values()
            .map(|page_id| {
                parsed
                    .get_object(*page_id)
                    .and_then(crate::pdf_native::Object::as_dict)
                    .and_then(|page| page.get(b"Contents"))
                    .and_then(crate::pdf_native::Object::as_reference)
                    .expect("tagged page content reference")
            })
            .collect();
        assert_eq!(content_ids.len(), 2);
        assert_eq!(count_token(&batch, b"/StructParents "), 2);
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
            crate::parallel::with_thread_count(threads, || {
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

    #[test]
    fn css_mask_program_affects_raster_and_pdf_output() {
        let engine = FullBleed::builder().build().expect("engine");
        let html = "<!doctype html><html><body><div class='masked'></div></body></html>";
        let css = "
            @page { size: 100px 100px; margin: 0; }
            html, body { margin: 0; padding: 0; }
            .masked {
                width: 100px;
                height: 100px;
                background: red;
                mask-image: linear-gradient(90deg, black 0 50%, transparent 50% 100%);
                mask-repeat: no-repeat;
            }
        ";

        let pages = engine
            .render_image_pages(html, css, 96)
            .expect("masked raster render");
        let image = crate::image_native::load_from_memory(&pages[0])
            .expect("decode masked preview")
            .to_rgba8();
        let left = image.get_pixel(20, 50).0;
        let right = image.get_pixel(80, 50).0;
        assert!(
            left[0] > 220 && left[1] < 40 && left[2] < 40,
            "left={left:?}"
        );
        assert!(
            right[0] > 220 && right[1] > 220 && right[2] > 220,
            "right={right:?}"
        );

        let pdf = engine
            .render_to_buffer(html, css)
            .expect("masked pdf render");
        let pdf_text = String::from_utf8_lossy(&pdf);
        assert_eq!(
            pdf_text.matches("/Subtype /Image").count(),
            0,
            "the PDF should compile the linear mask shader without rasterizing either surface"
        );
        assert!(pdf_text.contains("/CS /DeviceRGB"));
        assert!(pdf_text.contains("/ShadingType 2"));
        assert!(pdf_text.contains("/Subtype /Form"));
        assert!(pdf_text.contains("/SMask"));
    }

    #[test]
    fn css_mask_program_handles_svg_alpha_inline_refs_and_border_slices() {
        let engine = FullBleed::builder().build().expect("engine");
        let page_css =
            "@page { size: 200px 140px; margin: 0; } html, body { margin: 0; padding: 0; }";
        let render = |html: &str, css: &str| {
            let pages = engine
                .render_image_pages(html, css, 96)
                .expect("mask feature raster");
            crate::image_native::load_from_memory(&pages[0])
                .expect("decode mask feature preview")
                .to_rgba8()
        };
        let is_green = |pixel: [u8; 4]| pixel[1] > 90 && pixel[0] < 100 && pixel[2] < 100;
        let is_white = |pixel: [u8; 4]| pixel[0] > 220 && pixel[1] > 220 && pixel[2] > 220;

        let svg_source = "data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22%20width=%22180%22%20height=%22100%22%3E%3Crect%20width=%2260%22%20height=%22100%22%20fill=%22green%22/%3E%3Crect%20x=%2260%22%20width=%2260%22%20height=%22100%22%20fill=%22transparent%22/%3E%3Crect%20x=%22120%22%20width=%2260%22%20height=%22100%22%20fill=%22white%22/%3E%3C/svg%3E";
        let data_css = format!(
            "{page_css} .box {{ width: 180px; height: 100px; background: #2e7d32; mask-image: url(\"{svg_source}\"); mask-mode: alpha; mask-size: 180px 100px; mask-repeat: no-repeat; }}"
        );
        let data_image = render(
            "<html><body><div class='box'></div></body></html>",
            &data_css,
        );
        assert!(is_green(data_image.get_pixel(30, 50).0));
        assert!(is_white(data_image.get_pixel(90, 50).0));
        assert!(is_green(data_image.get_pixel(150, 50).0));

        let inline_html = "<html><body><svg style='position:absolute;width:0;height:0'><defs><mask id='alpha-mask' maskUnits='userSpaceOnUse' x='0' y='0' width='180' height='100' mask-type='alpha'><rect width='60' height='100' fill='black'/><rect x='60' width='60' height='100' fill='transparent'/><rect x='120' width='60' height='100' fill='black'/></mask></defs></svg><div class='inline'></div></body></html>";
        let inline_css = format!(
            "{page_css} .inline {{ width: 180px; height: 100px; background: #2e7d32; mask-image: url(#alpha-mask); }}"
        );
        let inline_image = render(inline_html, &inline_css);
        assert!(is_green(inline_image.get_pixel(30, 50).0));
        assert!(is_white(inline_image.get_pixel(90, 50).0));
        assert!(is_green(inline_image.get_pixel(150, 50).0));

        let conic_css = "@page { size: 200px 200px; margin: 0; } html, body { margin: 0; } .fan { width: 200px; height: 200px; background: #ef6c00; mask-image: conic-gradient(from 0deg, black 0 50%, transparent 50% 100%); }";
        let conic_image = render(
            "<html><body><div class='fan'></div></body></html>",
            conic_css,
        );
        let orange_pixels = conic_image
            .pixels()
            .filter(|pixel| pixel.0[0] > 180 && pixel.0[1] < 160 && pixel.0[2] < 60)
            .count();
        assert!(
            (15_000..=25_000).contains(&orange_pixels),
            "orange_pixels={orange_pixels}"
        );
        let conic_program = engine
            .render_to_document(
                "<html><body><div class='fan'></div></body></html>",
                conic_css,
            )
            .expect("compile conic mask program");
        fn count_conic_commands(commands: &[Command]) -> usize {
            commands
                .iter()
                .map(|command| match command {
                    Command::ShadingFill(crate::types::Shading::Conic { .. }) => 1,
                    Command::DefineForm { commands, .. }
                    | Command::DefineIsolatedForm { commands, .. } => {
                        count_conic_commands(commands)
                    }
                    _ => 0,
                })
                .sum()
        }
        let conic_commands = conic_program
            .pages
            .iter()
            .map(|page| count_conic_commands(&page.commands))
            .sum::<usize>();
        assert_eq!(
            conic_commands, 1,
            "a conic mask should remain one compiled shader command"
        );

        let radial_css = "@page { size: 224px 144px; margin: 0; } html, body { margin: 0; } .disc { width: 220px; height: 140px; background: #1565c0; mask-image: radial-gradient(circle 30px at 40px 40px, #000 0 30px, transparent 31px); mask-size: 80px 80px; mask-repeat: no-repeat; mask-position: 120px 54px; }";
        let radial_image = render(
            "<html><body><div class='disc'></div></body></html>",
            radial_css,
        );
        let disc_center = radial_image.get_pixel(160, 94).0;
        let tile_corner = radial_image.get_pixel(125, 59).0;
        assert!(
            disc_center[2] > 140 && disc_center[0] < 80,
            "disc_center={disc_center:?}"
        );
        assert!(is_white(tile_corner), "tile_corner={tile_corner:?}");

        let border_css = format!(
            "{page_css} .ring {{ width: 200px; height: 140px; background: #d32f2f; mask-border-source: linear-gradient(black, black); mask-border-slice: 28; mask-border-width: 28px; mask-border-repeat: stretch; }}"
        );
        let border_image = render(
            "<html><body><div class='ring'></div></body></html>",
            &border_css,
        );
        let edge = border_image.get_pixel(10, 70).0;
        let center = border_image.get_pixel(100, 70).0;
        assert!(edge[0] > 180 && edge[1] < 100, "edge={edge:?}");
        assert!(is_white(center), "center={center:?}");
    }
}
