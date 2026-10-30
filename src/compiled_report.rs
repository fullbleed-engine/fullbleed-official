//! Coverage of the final text selected by a compiled program, without another layout pass.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::{Command, Document, FontRegistry, GlyphCoverageReport, pdf, text_shape};

/// Diagnostics for one row of a compiled binding batch.
#[derive(Debug, Clone)]
pub struct CompiledRecordReport {
    /// Zero-based position in the input columns.
    pub record_index: usize,
    /// Zero-based position of this record's first page in the combined PDF.
    pub page_offset: usize,
    pub page_count: usize,
    /// Characters absent from the registered fonts selected for the final text.
    ///
    /// Fixed bindings keep their compiled fonts; another registered font does
    /// not cover a character unless the output actually selects that font.
    /// Base-14 fonts and text already converted to images are not validated.
    /// This is character coverage, not a shaping, layout, or conformance check.
    pub glyph_report: GlyphCoverageReport,
}

/// Ordered record diagnostics returned alongside a compiled batch's PDF bytes.
#[derive(Debug, Clone, Default)]
pub struct CompiledBatchReport {
    pub page_count: usize,
    pub records: Vec<CompiledRecordReport>,
}

impl CompiledBatchReport {
    pub(crate) fn push(&mut self, page_count: usize, glyph_report: GlyphCoverageReport) {
        self.records.push(CompiledRecordReport {
            record_index: self.records.len(),
            page_offset: self.page_count,
            page_count,
            glyph_report,
        });
        self.page_count = self.page_count.saturating_add(page_count);
    }
}

struct TextRun {
    font: Arc<str>,
    text: Arc<str>,
    // Only page-local text can receive a compiled flow override.
    location: Option<(usize, usize)>,
}

#[derive(Default)]
pub(crate) struct CoveragePlan {
    runs: Vec<TextRun>,
}

impl CoveragePlan {
    pub(crate) fn new(document: &Document) -> Self {
        let mut plan = Self::default();
        let mut forms = HashMap::new();
        let mut active_forms = HashSet::new();
        for (page, source) in document.pages.iter().enumerate() {
            plan.commands(&source.commands, Some(page), &mut forms, &mut active_forms);
        }
        plan
    }

    fn commands<'a>(
        &mut self,
        commands: &'a [Command],
        page: Option<usize>,
        forms: &mut HashMap<&'a str, &'a [Command]>,
        active_forms: &mut HashSet<&'a str>,
    ) {
        // PDF page and Form content streams start with independent text state.
        let mut font = Arc::<str>::from("Helvetica");
        let mut stack = Vec::new();
        for (index, command) in commands.iter().enumerate() {
            match command {
                Command::SaveState => stack.push(font.clone()),
                Command::RestoreState => {
                    if let Some(saved) = stack.pop() {
                        font = saved;
                    }
                }
                Command::SetFontName(name) => font = Arc::from(name.as_str()),
                Command::DrawString { text, .. } | Command::DrawStringTransformed { text, .. } => {
                    self.runs.push(TextRun {
                        font: font.clone(),
                        text: Arc::from(text.as_str()),
                        location: page.map(|page| (page, index)),
                    });
                }
                Command::DefineForm {
                    resource_id,
                    commands,
                    ..
                }
                | Command::DefineIsolatedForm {
                    resource_id,
                    commands,
                    ..
                } => {
                    forms.insert(resource_id, commands);
                }
                Command::DrawForm { resource_id, .. }
                | Command::DrawFilteredForm { resource_id, .. } => {
                    self.form(resource_id, forms, active_forms);
                }
                Command::DrawMaskedForm {
                    resource_id,
                    layers,
                    ..
                } => {
                    self.form(resource_id, forms, active_forms);
                    for layer in layers {
                        self.form(&layer.resource_id, forms, active_forms);
                    }
                }
                _ => {}
            }
        }
    }

    fn form<'a>(
        &mut self,
        name: &'a str,
        forms: &mut HashMap<&'a str, &'a [Command]>,
        active_forms: &mut HashSet<&'a str>,
    ) {
        if let Some(commands) = forms.get(name).copied() {
            // A recursive Form does not introduce another encodable text run.
            if active_forms.insert(name) {
                self.commands(commands, None, forms, active_forms);
                active_forms.remove(name);
            }
        }
    }

    pub(crate) fn fixed(
        &self,
        coverage: &mut FontCoverage<'_>,
        slots: &HashMap<String, usize>,
        columns: &[&[String]],
        row: usize,
    ) -> GlyphCoverageReport {
        let mut report = GlyphCoverageReport::default();
        for run in &self.runs {
            let text = pdf::instantiate_binding_text(&run.text, slots, columns, row);
            coverage.record(&run.font, &text, &mut report);
        }
        report
    }

    pub(crate) fn reflow(
        &self,
        coverage: &mut FontCoverage<'_>,
        overrides: &[Vec<pdf::CompiledFlowTextOverride>],
    ) -> GlyphCoverageReport {
        let mut report = GlyphCoverageReport::default();
        for run in &self.runs {
            let text = run
                .location
                .and_then(|(page, command)| {
                    let page_overrides = overrides.get(page)?;
                    let index = page_overrides
                        .binary_search_by_key(&command, |item| item.command)
                        .ok()?;
                    Some(page_overrides[index].text.as_ref())
                })
                .unwrap_or(&run.text);
            coverage.record(&run.font, text, &mut report);
        }
        report
    }
}

pub(crate) struct FontCoverage<'a> {
    registry: &'a FontRegistry,
    supported: HashMap<(Arc<str>, char), bool>,
    registered: HashMap<Arc<str>, bool>,
}

impl<'a> FontCoverage<'a> {
    pub(crate) fn new(registry: &'a FontRegistry) -> Self {
        Self {
            registry,
            supported: HashMap::new(),
            registered: HashMap::new(),
        }
    }

    fn record(&mut self, font: &Arc<str>, text: &str, report: &mut GlyphCoverageReport) {
        if !*self
            .registered
            .entry(font.clone())
            .or_insert_with(|| self.registry.resolve(font).is_some())
        {
            return;
        }
        let (_, text) = text_shape::decode_shape_options(text);
        for ch in text.chars().filter(|ch| !ch.is_ascii_control()) {
            let supported = self
                .supported
                .entry((font.clone(), ch))
                .or_insert_with(|| self.registry.map_glyph_id_for_char(font, ch) != 0);
            if !*supported {
                report.record_missing(ch, vec![font.to_string()]);
            }
        }
    }
}
