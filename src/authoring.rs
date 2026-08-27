//! In-memory authoring preview contract used by visual editors and agents.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

use fullbleed_audit_contract::sha256::Sha256;

use crate::canvas::{
    Command, META_DIAGNOSTIC_SCOPE_BEGIN_KEY, META_DIAGNOSTIC_SCOPE_END_KEY,
    META_FLOWABLE_BBOX_KEY, PageGeometry,
};
use crate::css_native::{AtRuleBlock, Rule};
use crate::html_dom::{NodeData, parse_html};
use crate::types::PageOrientation;
use crate::{Document, DocumentMetrics, FullBleed, FullBleedError, PageMetrics, pdf, raster};

const AUTHORING_LAYOUT_STACK_BYTES: usize = 16 * 1024 * 1024;

pub const AUTHORING_LANGUAGE_REPORT_SCHEMA: &str = "fullbleed.authoring_language_report.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoringSourceLanguage {
    Html,
    Css,
}

#[derive(Debug, Clone, Copy)]
pub struct AuthoringLanguageRequest<'a> {
    pub language: AuthoringSourceLanguage,
    pub source: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringLanguageDiagnostic {
    pub code: String,
    pub severity: AuthoringDiagnosticSeverity,
    pub message: String,
    pub byte_offset: usize,
    pub byte_length: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthoringLanguageFacts {
    pub element_count: usize,
    pub attribute_count: usize,
    pub rule_count: usize,
    pub declaration_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoringLanguageFeatureContext {
    HtmlAttribute,
    CssAtRule,
    CssProperty,
    CssValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringLanguageFeature {
    pub label: String,
    pub insert_text: String,
    pub context: AuthoringLanguageFeatureContext,
    pub detail: String,
    pub documentation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringLanguageReportV1 {
    pub schema: String,
    pub language: AuthoringSourceLanguage,
    pub parser: String,
    pub source_sha256: String,
    pub source_bytes: usize,
    pub valid: bool,
    pub recovery_enabled: bool,
    pub parse_micros: u64,
    pub facts: AuthoringLanguageFacts,
    pub diagnostics: Vec<AuthoringLanguageDiagnostic>,
    pub features: Vec<AuthoringLanguageFeature>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoringPreviewPhase {
    Layout,
    Pdf,
    Raster,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AuthoringPreviewProgress {
    pub phase: AuthoringPreviewPhase,
    pub completed: usize,
    pub total: usize,
    pub elapsed_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoringDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringDiagnostic {
    pub code: String,
    pub severity: AuthoringDiagnosticSeverity,
    pub message: String,
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringLayoutFragment {
    pub page_number: usize,
    pub x_milli_pt: i64,
    pub y_milli_pt: i64,
    pub width_milli_pt: i64,
    pub height_milli_pt: i64,
    pub paint_order: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringLayoutNode {
    pub source_id: String,
    pub geometry_available: bool,
    pub fragments: Vec<AuthoringLayoutFragment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringLayoutPage {
    pub page_number: usize,
    /// Physical raster/media width. This equals the trim width when the page
    /// has no bleed, marks, or orientation transform.
    pub width_milli_pt: i64,
    /// Physical raster/media height.
    pub height_milli_pt: i64,
    pub trim_x_milli_pt: i64,
    pub trim_y_milli_pt: i64,
    pub trim_width_milli_pt: i64,
    pub trim_height_milli_pt: i64,
    pub authored_bleed_milli_pt: i64,
    /// Sheet area reserved around trim. Crop/cross marks reserve at least 6pt
    /// even when the authored bleed is smaller.
    pub media_extent_milli_pt: i64,
    pub crop_marks: bool,
    pub cross_marks: bool,
    pub orientation: String,
    pub command_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringLayoutSnapshotV1 {
    pub schema: String,
    pub geometry_coverage: String,
    pub pages: Vec<AuthoringLayoutPage>,
    pub nodes: Vec<AuthoringLayoutNode>,
}

#[derive(Debug, Clone, Copy)]
pub struct AuthoringPreviewRequest<'a> {
    pub html: &'a str,
    pub css: &'a str,
    pub dpi: u32,
}

#[derive(Debug, Clone)]
pub struct AuthoringPreviewArtifactV1 {
    pub schema: String,
    pub pdf: Vec<u8>,
    pub png_pages: Vec<Vec<u8>>,
    pub pdf_sha256: String,
    pub metrics: DocumentMetrics,
    pub layout: AuthoringLayoutSnapshotV1,
    pub reading: AuthoringReadingPreviewV1,
    pub diagnostics: Vec<AuthoringDiagnostic>,
}

pub const AUTHORING_READING_PREVIEW_SCHEMA: &str = "fullbleed.authoring_reading_preview.v1";

/// A source-linked projection of the exact tag commands emitted by the laid-out document.
///
/// This is intentionally not a browser accessibility tree. It exposes the semantic commands
/// that feed Fullbleed's PDF structure-tree writer while keeping PDF-reader interoperability as
/// a separate, artifact-bound verification concern.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthoringReadingPreviewV1 {
    pub schema: String,
    pub coverage: String,
    pub node_count: usize,
    pub text_character_count: usize,
    pub actual_text_count: usize,
    pub alternate_text_count: usize,
    pub untagged_text_run_count: usize,
    pub artifact_marked_content_count: usize,
    pub pages: Vec<AuthoringReadingPage>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthoringReadingPage {
    pub page_number: usize,
    pub nodes: Vec<AuthoringReadingNode>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthoringReadingNode {
    pub role: String,
    pub source_id: Option<String>,
    pub text: String,
    pub alternate_text: Option<String>,
    pub actual_text: Option<String>,
    pub scope: Option<String>,
    pub table_id: Option<u32>,
    pub column_index: Option<u16>,
    pub group_only: bool,
    pub children: Vec<AuthoringReadingNode>,
}

#[derive(Debug, Clone, Default)]
pub struct AuthoringCancellationToken(Arc<AtomicBool>);

impl AuthoringCancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

fn check_cancelled(token: &AuthoringCancellationToken) -> Result<(), FullBleedError> {
    if token.is_cancelled() {
        Err(FullBleedError::Cancelled)
    } else {
        Ok(())
    }
}

fn source_ids(html: &str) -> Vec<String> {
    const NAME: &str = "data-fb-id";
    let mut ids = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut cursor = 0;
    while let Some(relative) = html[cursor..].find(NAME) {
        let start = cursor + relative + NAME.len();
        let bytes = html.as_bytes();
        let mut index = start;
        while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
            index += 1;
        }
        if bytes.get(index) != Some(&b'=') {
            cursor = start;
            continue;
        }
        index += 1;
        while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
            index += 1;
        }
        let Some(quote @ (b'\'' | b'"')) = bytes.get(index).copied() else {
            cursor = index;
            continue;
        };
        index += 1;
        let value_start = index;
        while bytes.get(index).is_some_and(|byte| *byte != quote) {
            index += 1;
        }
        if index > value_start {
            let value = html[value_start..index].to_owned();
            if seen.insert(value.clone()) {
                ids.push(value);
            }
        }
        cursor = index.saturating_add(1);
    }
    ids
}

fn parse_bbox(value: &str) -> Option<(i64, i64, i64, i64)> {
    let mut values = value.split(',').map(str::parse::<i64>);
    let result = (
        values.next()?.ok()?,
        values.next()?.ok()?,
        values.next()?.ok()?,
        values.next()?.ok()?,
    );
    values.next().is_none().then_some(result)
}

fn authored_fragments(
    document: &Document,
) -> std::collections::BTreeMap<String, Vec<AuthoringLayoutFragment>> {
    let mut by_source = std::collections::BTreeMap::<String, Vec<AuthoringLayoutFragment>>::new();
    for (page_index, page) in document.pages.iter().enumerate() {
        let mut current_source = None::<String>;
        let mut source_stack = Vec::<Option<String>>::new();
        for (command_index, command) in page.commands.iter().enumerate() {
            match command {
                Command::Meta { key, .. } if key == META_DIAGNOSTIC_SCOPE_BEGIN_KEY => {
                    source_stack.push(current_source.clone());
                }
                Command::Meta { key, .. } if key == META_DIAGNOSTIC_SCOPE_END_KEY => {
                    current_source = source_stack.pop().unwrap_or_default();
                }
                Command::Meta { key, value } if key == "fb.owner.source_id" => {
                    current_source = Some(value.clone());
                }
                Command::Meta { key, value } if key == META_FLOWABLE_BBOX_KEY => {
                    let (Some(source_id), Some((x, y, width, height))) =
                        (current_source.as_ref(), parse_bbox(value))
                    else {
                        continue;
                    };
                    let fragment = AuthoringLayoutFragment {
                        page_number: page_index + 1,
                        x_milli_pt: x,
                        y_milli_pt: y,
                        width_milli_pt: width,
                        height_milli_pt: height,
                        paint_order: command_index,
                    };
                    let fragments = by_source.entry(source_id.clone()).or_default();
                    if fragments.last() != Some(&fragment) {
                        fragments.push(fragment);
                    }
                }
                _ => {}
            }
        }
    }
    by_source
}

fn layout_snapshot(html: &str, document: &Document) -> AuthoringLayoutSnapshotV1 {
    let mut by_source = authored_fragments(document);
    let nodes = source_ids(html)
        .into_iter()
        .map(|source_id| {
            let fragments = by_source.remove(&source_id).unwrap_or_default();
            AuthoringLayoutNode {
                source_id,
                geometry_available: !fragments.is_empty(),
                fragments,
            }
        })
        .collect::<Vec<_>>();
    let available = nodes.iter().filter(|node| node.geometry_available).count();
    let geometry_coverage = if available == 0 {
        "structural_only"
    } else if available == nodes.len() {
        "authored_fragments"
    } else {
        "partial_authored_fragments"
    };
    AuthoringLayoutSnapshotV1 {
        schema: "fullbleed.layout_snapshot.v1".into(),
        geometry_coverage: geometry_coverage.into(),
        pages: document
            .pages
            .iter()
            .enumerate()
            .map(|(index, page)| {
                let geometry = PageGeometry::for_page(page, document.page_size);
                let extent = geometry.presentation.media_extent();
                AuthoringLayoutPage {
                    page_number: index + 1,
                    width_milli_pt: geometry.media_size.width.to_milli_i64(),
                    height_milli_pt: geometry.media_size.height.to_milli_i64(),
                    trim_x_milli_pt: extent.to_milli_i64(),
                    trim_y_milli_pt: extent.to_milli_i64(),
                    trim_width_milli_pt: (geometry.media_size.width - extent - extent)
                        .to_milli_i64(),
                    trim_height_milli_pt: (geometry.media_size.height - extent - extent)
                        .to_milli_i64(),
                    authored_bleed_milli_pt: geometry.presentation.bleed.to_milli_i64(),
                    media_extent_milli_pt: extent.to_milli_i64(),
                    crop_marks: geometry.presentation.marks.crop,
                    cross_marks: geometry.presentation.marks.cross,
                    orientation: match geometry.presentation.orientation {
                        PageOrientation::Upright => "upright",
                        PageOrientation::RotateLeft => "rotate_left",
                        PageOrientation::RotateRight => "rotate_right",
                    }
                    .into(),
                    command_count: page.commands.len(),
                }
            })
            .collect(),
        nodes,
    }
}

enum ReadingStackEntry {
    Node(AuthoringReadingNode),
    Suppressed,
}

fn normalize_reading_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn append_reading_text(target: &mut String, value: &str) {
    let value = normalize_reading_text(value);
    if value.is_empty() {
        return;
    }
    if !target.is_empty() {
        target.push(' ');
    }
    target.push_str(&value);
}

fn attach_reading_node(
    node: AuthoringReadingNode,
    stack: &mut [ReadingStackEntry],
    roots: &mut Vec<AuthoringReadingNode>,
) {
    match stack.last_mut() {
        Some(ReadingStackEntry::Node(parent)) => parent.children.push(node),
        Some(ReadingStackEntry::Suppressed) => {}
        None => roots.push(node),
    }
}

fn finish_reading_node(mut node: AuthoringReadingNode) -> AuthoringReadingNode {
    node.text = normalize_reading_text(&node.text);
    node.alternate_text = node
        .alternate_text
        .take()
        .map(|value| normalize_reading_text(&value))
        .filter(|value| !value.is_empty());
    node.actual_text = node
        .actual_text
        .take()
        .map(|value| normalize_reading_text(&value))
        .filter(|value| !value.is_empty());
    node
}

fn reading_node_facts(
    node: &AuthoringReadingNode,
    node_count: &mut usize,
    text_character_count: &mut usize,
    actual_text_count: &mut usize,
    alternate_text_count: &mut usize,
) {
    *node_count = node_count.saturating_add(1);
    *text_character_count = text_character_count.saturating_add(node.text.chars().count());
    if let Some(actual_text) = node.actual_text.as_deref() {
        *actual_text_count = actual_text_count.saturating_add(1);
        *text_character_count = text_character_count.saturating_add(actual_text.chars().count());
    }
    if let Some(alternate_text) = node.alternate_text.as_deref() {
        *alternate_text_count = alternate_text_count.saturating_add(1);
        *text_character_count = text_character_count.saturating_add(alternate_text.chars().count());
    }
    for child in &node.children {
        reading_node_facts(
            child,
            node_count,
            text_character_count,
            actual_text_count,
            alternate_text_count,
        );
    }
}

fn reading_preview(document: &Document) -> AuthoringReadingPreviewV1 {
    let mut pages = Vec::with_capacity(document.pages.len());
    let mut untagged_text_run_count = 0usize;
    let mut artifact_marked_content_count = 0usize;

    for (page_index, page) in document.pages.iter().enumerate() {
        let mut roots = Vec::new();
        let mut stack = Vec::<ReadingStackEntry>::new();
        let mut current_source = None::<String>;
        let mut source_stack = Vec::<Option<String>>::new();
        let mut marked_content_stack = Vec::<bool>::new();
        let mut artifact_depth = 0usize;

        for command in &page.commands {
            match command {
                Command::Meta { key, .. } if key == META_DIAGNOSTIC_SCOPE_BEGIN_KEY => {
                    source_stack.push(current_source.clone());
                }
                Command::Meta { key, .. } if key == META_DIAGNOSTIC_SCOPE_END_KEY => {
                    current_source = source_stack.pop().unwrap_or_default();
                }
                Command::Meta { key, value } if key == "fb.owner.source_id" => {
                    current_source = Some(value.clone());
                }
                Command::BeginArtifact { .. } => {
                    marked_content_stack.push(true);
                    artifact_depth = artifact_depth.saturating_add(1);
                    artifact_marked_content_count = artifact_marked_content_count.saturating_add(1);
                }
                Command::BeginOptionalContent { .. } => marked_content_stack.push(false),
                Command::EndMarkedContent => {
                    if marked_content_stack.pop().unwrap_or(false) {
                        artifact_depth = artifact_depth.saturating_sub(1);
                    }
                }
                Command::BeginTag {
                    role,
                    alt,
                    scope,
                    table_id,
                    col_index,
                    group_only,
                    ..
                } => {
                    if artifact_depth > 0 || role.eq_ignore_ascii_case("artifact") {
                        stack.push(ReadingStackEntry::Suppressed);
                    } else {
                        stack.push(ReadingStackEntry::Node(AuthoringReadingNode {
                            role: role.clone(),
                            source_id: current_source.clone(),
                            text: String::new(),
                            alternate_text: alt.clone(),
                            actual_text: None,
                            scope: scope.clone(),
                            table_id: *table_id,
                            column_index: *col_index,
                            group_only: *group_only,
                            children: Vec::new(),
                        }));
                    }
                }
                Command::BeginTagActualText {
                    role, actual_text, ..
                } => {
                    if artifact_depth > 0 || role.eq_ignore_ascii_case("artifact") {
                        stack.push(ReadingStackEntry::Suppressed);
                    } else {
                        stack.push(ReadingStackEntry::Node(AuthoringReadingNode {
                            role: role.clone(),
                            source_id: current_source.clone(),
                            text: String::new(),
                            alternate_text: None,
                            actual_text: Some(actual_text.clone()),
                            scope: None,
                            table_id: None,
                            column_index: None,
                            group_only: false,
                            children: Vec::new(),
                        }));
                    }
                }
                Command::EndTag => {
                    if let Some(ReadingStackEntry::Node(node)) = stack.pop() {
                        attach_reading_node(finish_reading_node(node), &mut stack, &mut roots);
                    }
                }
                Command::DrawString { text, .. } | Command::DrawStringTransformed { text, .. } => {
                    if artifact_depth > 0 {
                        continue;
                    }
                    if let Some(ReadingStackEntry::Node(node)) = stack.last_mut() {
                        append_reading_text(&mut node.text, text);
                    } else if !text.trim().is_empty() {
                        untagged_text_run_count = untagged_text_run_count.saturating_add(1);
                    }
                }
                _ => {}
            }
        }

        while let Some(entry) = stack.pop() {
            if let ReadingStackEntry::Node(node) = entry {
                attach_reading_node(finish_reading_node(node), &mut stack, &mut roots);
            }
        }
        pages.push(AuthoringReadingPage {
            page_number: page_index + 1,
            nodes: roots,
        });
    }

    let mut node_count = 0usize;
    let mut text_character_count = 0usize;
    let mut actual_text_count = 0usize;
    let mut alternate_text_count = 0usize;
    for page in &pages {
        for node in &page.nodes {
            reading_node_facts(
                node,
                &mut node_count,
                &mut text_character_count,
                &mut actual_text_count,
                &mut alternate_text_count,
            );
        }
    }
    let coverage = if node_count == 0 && untagged_text_run_count == 0 {
        "empty"
    } else if node_count == 0 {
        "unavailable"
    } else if untagged_text_run_count > 0 {
        "partial"
    } else {
        "compiled_tag_tree"
    };

    AuthoringReadingPreviewV1 {
        schema: AUTHORING_READING_PREVIEW_SCHEMA.into(),
        coverage: coverage.into(),
        node_count,
        text_character_count,
        actual_text_count,
        alternate_text_count,
        untagged_text_run_count,
        artifact_marked_content_count,
        pages,
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn language_feature(
    label: &str,
    insert_text: &str,
    context: AuthoringLanguageFeatureContext,
    detail: &str,
    documentation: &str,
) -> AuthoringLanguageFeature {
    AuthoringLanguageFeature {
        label: label.into(),
        insert_text: insert_text.into(),
        context,
        detail: detail.into(),
        documentation: documentation.into(),
    }
}

/// Returns the Fullbleed-native authoring vocabulary used by editor completion surfaces.
///
/// This catalog intentionally contains only engine-owned extensions and print primitives.
/// Hosts may add their own source dialect without presenting browser-only features as engine
/// support.
pub fn authoring_language_features(
    language: AuthoringSourceLanguage,
) -> Vec<AuthoringLanguageFeature> {
    use AuthoringLanguageFeatureContext::{CssAtRule, CssProperty, CssValue, HtmlAttribute};

    match language {
        AuthoringSourceLanguage::Html => vec![
            language_feature(
                "data-fb-id",
                "data-fb-id=\"${id}\"",
                HtmlAttribute,
                "Fullbleed authored-node identity",
                "Stable source identity retained in authoring layout snapshots and diagnostics.",
            ),
            language_feature(
                "data-fb-bind-html",
                "data-fb-bind-html=\"${slot}\"",
                HtmlAttribute,
                "Compiled reflow structural slot",
                "Trusted compiled-reflow slot whose escaped structural fragment replaces the element children.",
            ),
            language_feature(
                "data-fb-role",
                "data-fb-role=\"${role}\"",
                HtmlAttribute,
                "Fullbleed semantic role",
                "Carries a document-semantic role into Fullbleed ownership and accessibility metadata.",
            ),
            language_feature(
                "data-fb-component",
                "data-fb-component=\"${component}\"",
                HtmlAttribute,
                "Fullbleed component identity",
                "Names the authored component retained in Fullbleed document metadata.",
            ),
            language_feature(
                "data-fb-a11y-only",
                "data-fb-a11y-only=\"true\"",
                HtmlAttribute,
                "Screen-reader-only semantic text",
                "Compiles resolved element text into tagged PDF /ActualText without glyph paint or normal-flow geometry.",
            ),
            language_feature(
                "data-fb-page",
                "data-fb-page=\"${page}\"",
                HtmlAttribute,
                "Named page metadata",
                "Associates authored content with Fullbleed named-page metadata.",
            ),
        ],
        AuthoringSourceLanguage::Css => vec![
            language_feature(
                "@page",
                "@page {\n  size: letter;\n  margin: 0.5in;\n}",
                CssAtRule,
                "Paged-media page definition",
                "Defines physical page size, margins, bleed, marks, orientation, and margin boxes.",
            ),
            language_feature(
                "@font-face",
                "@font-face {\n  font-family: ${family};\n  src: url(\"${path}\");\n}",
                CssAtRule,
                "Vendored font face",
                "Declares a font face whose bytes must be registered or vendored by the host project.",
            ),
            language_feature(
                "@top-center",
                "@top-center {\n  content: ${content};\n}",
                CssAtRule,
                "Page margin box",
                "Creates a top-center margin box inside an @page rule.",
            ),
            language_feature(
                "@bottom-center",
                "@bottom-center {\n  content: ${content};\n}",
                CssAtRule,
                "Page margin box",
                "Creates a bottom-center margin box inside an @page rule.",
            ),
            language_feature(
                "@footnote",
                "@footnote {\n  border-top: 0.5pt solid #777;\n  padding-top: 6pt;\n}",
                CssAtRule,
                "Footnote area",
                "Styles the page footnote area inside an @page rule.",
            ),
            language_feature(
                "size",
                "size: ${letter};",
                CssProperty,
                "Physical page size",
                "Sets a named or explicit physical size inside @page.",
            ),
            language_feature(
                "bleed",
                "bleed: ${9pt};",
                CssProperty,
                "Page bleed",
                "Sets the authored page bleed extent used by print output.",
            ),
            language_feature(
                "marks",
                "marks: ${crop};",
                CssProperty,
                "Printer marks",
                "Enables crop and/or cross marks around the physical sheet.",
            ),
            language_feature(
                "page-orientation",
                "page-orientation: ${rotate-left};",
                CssProperty,
                "Physical page orientation",
                "Rotates the page presentation while retaining deterministic sheet geometry.",
            ),
            language_feature(
                "page",
                "page: ${chapter};",
                CssProperty,
                "Named page selection",
                "Assigns an element to a named @page definition.",
            ),
            language_feature(
                "string-set",
                "string-set: ${section} content(text);",
                CssProperty,
                "Named running string",
                "Captures content or an attribute for later use in page margin boxes.",
            ),
            language_feature(
                "break-before",
                "break-before: ${page};",
                CssProperty,
                "Pagination control",
                "Requests a deterministic fragmentation break before the box.",
            ),
            language_feature(
                "break-after",
                "break-after: ${page};",
                CssProperty,
                "Pagination control",
                "Requests a deterministic fragmentation break after the box.",
            ),
            language_feature(
                "break-inside",
                "break-inside: ${avoid};",
                CssProperty,
                "Pagination control",
                "Controls fragmentation inside a box.",
            ),
            language_feature(
                "position: running()",
                "position: running(${name});",
                CssValue,
                "Running element",
                "Captures an element for use by element() in a page margin box.",
            ),
            language_feature(
                "element()",
                "element(${name})",
                CssValue,
                "Running-element content",
                "Places a captured running element in generated page content.",
            ),
            language_feature(
                "string()",
                "string(${name})",
                CssValue,
                "Named-string content",
                "Reads a string-set value in generated page content.",
            ),
            language_feature(
                "target-counter()",
                "target-counter(attr(href), page)",
                CssValue,
                "Cross-reference page counter",
                "Resolves a target element's page counter for generated cross references.",
            ),
        ],
    }
}

fn count_css_rules(rules: &[Rule], rule_count: &mut usize, declaration_count: &mut usize) {
    for rule in rules {
        *rule_count = (*rule_count).saturating_add(1);
        match rule {
            Rule::Style(style) => {
                *declaration_count =
                    (*declaration_count).saturating_add(style.declarations.declarations.len());
            }
            Rule::At(at_rule) => match at_rule.block.as_ref() {
                Some(AtRuleBlock::Rules(nested)) => {
                    count_css_rules(nested, rule_count, declaration_count);
                }
                Some(AtRuleBlock::Declarations(block)) => {
                    *declaration_count =
                        (*declaration_count).saturating_add(block.declarations.len());
                }
                Some(AtRuleBlock::Raw(_)) | None => {}
            },
        }
    }
}

/// Parses one in-memory authoring source with the same deterministic parser used by Fullbleed.
///
/// HTML uses Fullbleed's recovery tree builder, so recoverable HTML remains valid. CSS reports
/// the engine parser's exact byte offset on a syntax failure. This operation performs no layout,
/// filesystem access, source mutation, or network access.
pub fn inspect_authoring_source(
    request: AuthoringLanguageRequest<'_>,
) -> AuthoringLanguageReportV1 {
    let started = Instant::now();
    let mut diagnostics = Vec::new();
    let mut facts = AuthoringLanguageFacts::default();
    let (parser, valid, recovery_enabled) = match request.language {
        AuthoringSourceLanguage::Html => {
            let document = parse_html(request.source);
            for node in document.descendants() {
                if let NodeData::Element(element) = node.data() {
                    facts.element_count = facts.element_count.saturating_add(1);
                    facts.attribute_count = facts
                        .attribute_count
                        .saturating_add(element.attributes.borrow().map.len());
                }
            }
            if let Some(byte_offset) = request.source.as_bytes().iter().position(|byte| *byte == 0)
            {
                diagnostics.push(AuthoringLanguageDiagnostic {
                    code: "HTML_NULL_RECOVERED".into(),
                    severity: AuthoringDiagnosticSeverity::Warning,
                    message:
                        "Fullbleed replaced a NULL character while building the HTML recovery tree."
                            .into(),
                    byte_offset,
                    byte_length: 1,
                });
            }
            ("fullbleed.html_dom.recovery.v1", true, true)
        }
        AuthoringSourceLanguage::Css => match crate::css_native::parse_stylesheet(request.source) {
            Ok(stylesheet) => {
                count_css_rules(
                    &stylesheet.rules,
                    &mut facts.rule_count,
                    &mut facts.declaration_count,
                );
                ("fullbleed.css_native.v1", true, false)
            }
            Err(error) => {
                diagnostics.push(AuthoringLanguageDiagnostic {
                    code: "CSS_PARSE_ERROR".into(),
                    severity: AuthoringDiagnosticSeverity::Error,
                    message: error.message().into(),
                    byte_offset: error.offset().min(request.source.len()),
                    byte_length: usize::from(error.offset() < request.source.len()),
                });
                ("fullbleed.css_native.v1", false, false)
            }
        },
    };
    let parse_micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    AuthoringLanguageReportV1 {
        schema: AUTHORING_LANGUAGE_REPORT_SCHEMA.into(),
        language: request.language,
        parser: parser.into(),
        source_sha256: hex_digest(request.source.as_bytes()),
        source_bytes: request.source.len(),
        valid,
        recovery_enabled,
        parse_micros,
        facts,
        diagnostics,
        features: authoring_language_features(request.language),
    }
}

impl FullBleed {
    /// Render an editor preview entirely from in-memory sources.
    ///
    /// PDF and PNG pages consume the same laid-out `Document`. Node IDs are retained in the
    /// structural snapshot immediately; `geometry_coverage` remains `structural_only` until the
    /// layout pipeline reports authored fragment boxes.
    pub fn render_authoring_preview(
        &self,
        request: AuthoringPreviewRequest<'_>,
        cancellation: &AuthoringCancellationToken,
        mut progress: impl FnMut(AuthoringPreviewProgress),
    ) -> Result<AuthoringPreviewArtifactV1, FullBleedError> {
        let started = Instant::now();
        check_cancelled(cancellation)?;
        progress(AuthoringPreviewProgress {
            phase: AuthoringPreviewPhase::Layout,
            completed: 0,
            total: 1,
            elapsed_ms: 0.0,
        });
        let layout_started = Instant::now();
        // Windows executables have a comparatively small default main-thread stack. Complex
        // layout paths (notably tables) legitimately need more space, so the authoring boundary
        // makes that resource explicit instead of inheriting an arbitrary host stack size.
        let document = std::thread::scope(|scope| -> Result<_, FullBleedError> {
            let worker = std::thread::Builder::new()
                .name("fullbleed-authoring-layout".into())
                .stack_size(AUTHORING_LAYOUT_STACK_BYTES)
                .spawn_scoped(scope, || self.render_to_document(request.html, request.css))
                .map_err(FullBleedError::Io)?;
            worker.join().map_err(|_| {
                FullBleedError::InvalidConfiguration(
                    "authoring layout worker terminated unexpectedly".into(),
                )
            })?
        })?;
        let layout_ms = layout_started.elapsed().as_secs_f64() * 1000.0;
        check_cancelled(cancellation)?;
        progress(AuthoringPreviewProgress {
            phase: AuthoringPreviewPhase::Layout,
            completed: 1,
            total: 1,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        });
        progress(AuthoringPreviewProgress {
            phase: AuthoringPreviewPhase::Pdf,
            completed: 0,
            total: 1,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        });
        let mut metrics = DocumentMetrics {
            pages: document
                .pages
                .iter()
                .enumerate()
                .map(|(index, page)| PageMetrics {
                    page_number: index + 1,
                    render_ms: 0.0,
                    command_count: page.commands.len(),
                    flowable_count: 0,
                    content_bytes: 0,
                })
                .collect(),
            total_render_ms: layout_ms,
            total_bytes: 0,
        };
        let pdf = pdf::document_to_pdf_with_metrics_and_registry_with_logs(
            &document,
            Some(&mut metrics),
            Some(self.font_registry.as_ref()),
            &self.pdf_options,
            self.debug.clone(),
            self.perf.clone(),
        )?;
        metrics.total_bytes = pdf.len();
        check_cancelled(cancellation)?;
        progress(AuthoringPreviewProgress {
            phase: AuthoringPreviewPhase::Pdf,
            completed: 1,
            total: 1,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        });
        progress(AuthoringPreviewProgress {
            phase: AuthoringPreviewPhase::Raster,
            completed: 0,
            total: document.pages.len(),
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        });
        let png_pages = raster::document_to_png_pages(
            &document,
            request.dpi.max(72),
            Some(self.font_registry.as_ref()),
            self.pdf_options.shape_text,
        )?;
        check_cancelled(cancellation)?;
        progress(AuthoringPreviewProgress {
            phase: AuthoringPreviewPhase::Raster,
            completed: png_pages.len(),
            total: png_pages.len(),
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        });
        let layout = layout_snapshot(request.html, &document);
        let reading = reading_preview(&document);
        let diagnostics = if layout.nodes.iter().all(|node| node.geometry_available) {
            Vec::new()
        } else {
            vec![AuthoringDiagnostic {
                code: "LAYOUT_GEOMETRY_PARTIAL".into(),
                severity: AuthoringDiagnosticSeverity::Info,
                message: "Some authored node IDs do not yet expose fragment geometry.".into(),
                source_id: None,
            }]
        };
        progress(AuthoringPreviewProgress {
            phase: AuthoringPreviewPhase::Complete,
            completed: 1,
            total: 1,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        });
        Ok(AuthoringPreviewArtifactV1 {
            schema: "fullbleed.authoring_preview.v1".into(),
            pdf_sha256: hex_digest(&pdf),
            pdf,
            png_pages,
            metrics,
            layout,
            reading,
            diagnostics,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authoring_language_report_uses_engine_html_and_css_parsers() {
        let html = inspect_authoring_source(AuthoringLanguageRequest {
            language: AuthoringSourceLanguage::Html,
            source: "<main data-fb-id='root'><p>Hello</p></main>",
        });
        assert!(html.valid);
        assert!(html.recovery_enabled);
        assert_eq!(html.parser, "fullbleed.html_dom.recovery.v1");
        assert!(html.facts.element_count >= 5);
        assert!(html.features.iter().any(|feature| {
            feature.label == "data-fb-bind-html"
                && feature.context == AuthoringLanguageFeatureContext::HtmlAttribute
        }));
        assert!(html.features.iter().any(|feature| {
            feature.label == "data-fb-a11y-only"
                && feature.context == AuthoringLanguageFeatureContext::HtmlAttribute
        }));

        let css = inspect_authoring_source(AuthoringLanguageRequest {
            language: AuthoringSourceLanguage::Css,
            source: "@page { size: letter; margin: 0.5in; } body { color: #123456; }",
        });
        assert!(css.valid);
        assert_eq!(css.parser, "fullbleed.css_native.v1");
        assert_eq!(css.facts.rule_count, 2);
        assert_eq!(css.facts.declaration_count, 3);
        assert!(css.features.iter().any(|feature| feature.label == "@page"));
    }

    #[test]
    fn authoring_language_report_retains_exact_css_error_offset() {
        let source = "body { color: red";
        let report = inspect_authoring_source(AuthoringLanguageRequest {
            language: AuthoringSourceLanguage::Css,
            source,
        });
        assert!(!report.valid);
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].code, "CSS_PARSE_ERROR");
        assert!(report.diagnostics[0].byte_offset <= source.len());
        assert_eq!(report.source_sha256, hex_digest(source.as_bytes()));
    }

    fn png_dimensions(png: &[u8]) -> (u32, u32) {
        assert_eq!(png.get(..8), Some(&b"\x89PNG\r\n\x1a\n"[..]));
        (
            u32::from_be_bytes(png[16..20].try_into().unwrap()),
            u32::from_be_bytes(png[20..24].try_into().unwrap()),
        )
    }

    #[test]
    fn preview_is_in_memory_and_retains_source_ids() {
        let engine = FullBleed::builder()
            .document_title("Preview")
            .document_lang("en-US")
            .build()
            .unwrap();
        let token = AuthoringCancellationToken::new();
        let mut phases = Vec::new();
        let artifact = engine
            .render_authoring_preview(
                AuthoringPreviewRequest {
                    html: "<main data-fb-id=\"node-1\"><p>Hello</p></main>",
                    css: "@page { size: 200pt 200pt; margin: 10pt; }",
                    dpi: 72,
                },
                &token,
                |event| phases.push(event.phase),
            )
            .unwrap();
        assert_eq!(artifact.layout.nodes[0].source_id, "node-1");
        assert!(artifact.layout.nodes[0].geometry_available);
        assert!(!artifact.layout.nodes[0].fragments.is_empty());
        assert_eq!(artifact.layout.geometry_coverage, "authored_fragments");
        assert_eq!(artifact.png_pages.len(), 1);
        assert!(artifact.pdf.starts_with(b"%PDF"));
        assert!(phases.contains(&AuthoringPreviewPhase::Complete));
    }

    fn collect_reading_nodes<'a>(
        nodes: &'a [AuthoringReadingNode],
        collected: &mut Vec<&'a AuthoringReadingNode>,
    ) {
        for node in nodes {
            collected.push(node);
            collect_reading_nodes(&node.children, collected);
        }
    }

    #[test]
    fn preview_exposes_the_compiled_tag_tree_for_nonvisual_authoring() {
        let engine = FullBleed::builder()
            .document_title("Accessible statement")
            .document_lang("en-US")
            .build()
            .unwrap();
        let artifact = engine
            .render_authoring_preview(
                AuthoringPreviewRequest {
                    html: r#"<main data-fb-id="root">
                        <h1 data-fb-id="title">Account statement</h1>
                        <p data-fb-id="summary">Balance due 42 dollars.</p>
                        <span data-fb-id="context" data-fb-a11y-only="true">Due on Friday.</span>
                        <p class="hidden">This must not be announced.</p>
                        <svg data-fb-id="chart" aria-label="Revenue increased" width="20" height="10"><rect width="20" height="10" /></svg>
                        <table data-fb-id="detail"><tr><th scope="col">Item</th><td>Service</td></tr></table>
                    </main>"#,
                    css: "@page { size: 300pt 300pt; margin: 12pt; } .hidden { display: none; }",
                    dpi: 72,
                },
                &AuthoringCancellationToken::new(),
                |_| {},
            )
            .unwrap();

        assert_eq!(artifact.reading.schema, AUTHORING_READING_PREVIEW_SCHEMA);
        assert_eq!(artifact.reading.coverage, "compiled_tag_tree");
        assert_eq!(artifact.reading.pages.len(), 1);
        assert_eq!(artifact.reading.untagged_text_run_count, 0);
        assert_eq!(artifact.reading.actual_text_count, 1);
        assert_eq!(artifact.reading.alternate_text_count, 1);

        let mut nodes = Vec::new();
        collect_reading_nodes(&artifact.reading.pages[0].nodes, &mut nodes);
        assert!(nodes.iter().any(|node| {
            node.role == "H1"
                && node.text == "Account statement"
                && node.source_id.as_deref() == Some("title")
        }));
        assert!(nodes.iter().any(|node| {
            node.actual_text.as_deref() == Some("Due on Friday.")
                && node.source_id.as_deref() == Some("context")
        }));
        assert!(nodes.iter().any(|node| {
            node.role == "Figure"
                && node.alternate_text.as_deref() == Some("Revenue increased")
                && node.source_id.as_deref() == Some("chart")
        }));
        assert!(nodes.iter().any(|node| node.role == "Table"));
        assert!(
            nodes
                .iter()
                .any(|node| node.role == "TH" && node.scope.as_deref() == Some("Column"))
        );
        assert!(
            !nodes
                .iter()
                .any(|node| node.text.contains("must not be announced"))
        );
    }

    #[test]
    fn preview_raster_and_layout_share_bleed_marks_and_orientation_geometry() {
        let engine = FullBleed::builder().build().unwrap();
        let token = AuthoringCancellationToken::new();
        let html = "<main data-fb-id=\"page\"><p>Physical sheet</p></main>";

        let upright = engine
            .render_authoring_preview(
                AuthoringPreviewRequest {
                    html,
                    css: "@page { size: 100pt 60pt; margin: 0; bleed: 10pt; marks: crop; } body { margin: 0; }",
                    dpi: 72,
                },
                &token,
                |_| {},
            )
            .unwrap();
        let page = &upright.layout.pages[0];
        assert_eq!(
            (page.width_milli_pt, page.height_milli_pt),
            (120_000, 80_000)
        );
        assert_eq!(
            (page.trim_x_milli_pt, page.trim_y_milli_pt),
            (10_000, 10_000)
        );
        assert_eq!(
            (page.trim_width_milli_pt, page.trim_height_milli_pt),
            (100_000, 60_000)
        );
        assert_eq!(page.authored_bleed_milli_pt, 10_000);
        assert_eq!(page.media_extent_milli_pt, 10_000);
        assert!(page.crop_marks);
        assert!(!page.cross_marks);
        assert_eq!(page.orientation, "upright");
        assert_eq!(png_dimensions(&upright.png_pages[0]), (120, 80));

        let rotated = engine
            .render_authoring_preview(
                AuthoringPreviewRequest {
                    html,
                    css: "@page { size: 100pt 60pt; margin: 0; bleed: 10pt; marks: crop; page-orientation: rotate-left; } body { margin: 0; }",
                    dpi: 72,
                },
                &token,
                |_| {},
            )
            .unwrap();
        let page = &rotated.layout.pages[0];
        assert_eq!(
            (page.width_milli_pt, page.height_milli_pt),
            (80_000, 120_000)
        );
        assert_eq!(
            (page.trim_width_milli_pt, page.trim_height_milli_pt),
            (60_000, 100_000)
        );
        assert_eq!(page.orientation, "rotate_left");
        assert_eq!(png_dimensions(&rotated.png_pages[0]), (80, 120));
    }

    #[test]
    fn source_ids_accept_authored_single_quotes_and_spacing() {
        assert_eq!(
            source_ids("<p data-fb-id = 'first'></p><p data-fb-id=\"second\"></p>"),
            ["first", "second"]
        );
    }

    #[test]
    fn transformed_inline_svg_descendants_have_engine_authored_geometry() {
        let engine = FullBleed::builder().build().unwrap();
        let artifact = engine
            .render_authoring_preview(
                AuthoringPreviewRequest {
                    html: r##"<main data-fb-id="page"><svg data-fb-id="art" viewBox="0 0 100 80" width="100" height="80" xmlns="http://www.w3.org/2000/svg"><g data-fb-id="group" transform="matrix(0 1 -1 0 60 10)"><rect data-fb-id="shape" x="10" y="20" width="30" height="10" fill="#123456"/></g></svg></main>"##,
                    css: "@page { size: 200pt 200pt; margin: 10pt; } body { margin: 0; }",
                    dpi: 72,
                },
                &AuthoringCancellationToken::new(),
                |_| {},
            )
            .unwrap();

        assert_eq!(artifact.layout.geometry_coverage, "authored_fragments");
        let node = |source_id: &str| {
            artifact
                .layout
                .nodes
                .iter()
                .find(|node| node.source_id == source_id)
                .expect("authored SVG source id")
        };
        for source_id in ["page", "art", "group", "shape"] {
            assert!(node(source_id).geometry_available, "missing {source_id}");
        }
        let group = &node("group").fragments[0];
        let shape = &node("shape").fragments[0];
        assert_eq!(group.x_milli_pt, shape.x_milli_pt);
        assert_eq!(group.y_milli_pt, shape.y_milli_pt);
        assert_eq!(group.width_milli_pt, shape.width_milli_pt);
        assert_eq!(group.height_milli_pt, shape.height_milli_pt);
        assert!(shape.height_milli_pt > shape.width_milli_pt);
    }

    #[test]
    fn preview_geometry_tracks_relative_positioned_paint() {
        let engine = FullBleed::builder().build().unwrap();
        let render = |positioning: &str| {
            engine
                .render_authoring_preview(
                    AuthoringPreviewRequest {
                        html: r#"<main data-fb-id="page"><h1 data-fb-id="target">Test</h1></main>"#,
                        css: &format!(
                            "@page {{ size: 200pt 200pt; margin: 0; }} body {{ margin: 0; }} h1 {{ margin: 0; font-size: 20pt; {positioning} }}"
                        ),
                        dpi: 72,
                    },
                    &AuthoringCancellationToken::new(),
                    |_| {},
                )
                .unwrap()
        };
        let unshifted = render("");
        let shifted = render("position: relative; left: 13pt; top: 47pt;");
        fn fragment(artifact: &AuthoringPreviewArtifactV1) -> &AuthoringLayoutFragment {
            artifact
                .layout
                .nodes
                .iter()
                .find(|node| node.source_id == "target")
                .and_then(|node| node.fragments.first())
                .expect("target authoring fragment")
        }
        let unshifted = fragment(&unshifted);
        let shifted = fragment(&shifted);

        assert_eq!(shifted.x_milli_pt - unshifted.x_milli_pt, 13_000);
        assert_eq!(shifted.y_milli_pt - unshifted.y_milli_pt, 47_000);
        assert_eq!(shifted.width_milli_pt, unshifted.width_milli_pt);
        assert_eq!(shifted.height_milli_pt, unshifted.height_milli_pt);
    }

    #[test]
    fn cancelled_preview_stops_before_layout() {
        let engine = FullBleed::builder().build().unwrap();
        let token = AuthoringCancellationToken::new();
        token.cancel();
        assert!(
            engine
                .render_authoring_preview(
                    AuthoringPreviewRequest {
                        html: "<p>x</p>",
                        css: "",
                        dpi: 72
                    },
                    &token,
                    |_| {}
                )
                .is_err()
        );
    }

    #[test]
    fn preview_table_layout_uses_an_explicit_worker_stack() {
        let engine = FullBleed::builder().build().unwrap();
        let artifact = engine
            .render_authoring_preview(
                AuthoringPreviewRequest {
                    html: "<table><tbody><tr><td>A</td></tr></tbody></table>",
                    css: "",
                    dpi: 72,
                },
                &AuthoringCancellationToken::new(),
                |_| {},
            )
            .unwrap();
        assert_eq!(artifact.layout.pages.len(), 1);
    }
}
