use crate::canvas::{
    Command, Document, ImageSourceClip, META_PAGE_PRESENTATION_KEY, META_PAGE_SIZE_KEY, Page,
    ResolvedImageSourceCrop,
};
use crate::debug::json_escape;
use crate::font::{
    FontProgramKind, FontRegistry, GlyphOutlineCommand, RegisteredFont, RegisteredGlyphOutline,
};
use crate::metrics::{DocumentMetrics, PageMetrics};
use crate::perf::PerfLogger;
use crate::sfnt::{Face as SfntFace, GlyphId};
use crate::types::{
    Color, ColorSpace, MixBlendMode, PageOrientation, PagePresentation, Pt, Shading, ShadingStop,
    Size,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{self, Write};
use std::path::Path;

fn effective_page_size(page: &Page, fallback: Size) -> Size {
    page.commands
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Meta { key, value } if key == META_PAGE_SIZE_KEY => {
                let (width, height) = value.split_once(',')?;
                Some(Size {
                    width: Pt::from_milli_i64(width.parse().ok()?),
                    height: Pt::from_milli_i64(height.parse().ok()?),
                })
            }
            _ => None,
        })
        .unwrap_or(fallback)
        .quantized()
}

fn effective_page_presentation(page: &Page) -> PagePresentation {
    page.commands
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Meta { key, value } if key == META_PAGE_PRESENTATION_KEY => {
                PagePresentation::decode(value)
            }
            _ => None,
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PageGeometry {
    logical_size: Size,
    media_size: Size,
    presentation: PagePresentation,
}

impl PageGeometry {
    fn for_page(page: &Page, fallback: Size) -> Self {
        let logical_size = effective_page_size(page, fallback);
        let presentation = effective_page_presentation(page);
        let extent = presentation.media_extent();
        let unrotated = Size {
            width: logical_size.width + extent + extent,
            height: logical_size.height + extent + extent,
        }
        .quantized();
        let media_size = match presentation.orientation {
            PageOrientation::Upright => unrotated,
            PageOrientation::RotateLeft | PageOrientation::RotateRight => Size {
                width: unrotated.height,
                height: unrotated.width,
            },
        }
        .quantized();
        Self {
            logical_size,
            media_size,
            presentation,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PdfOptions {
    // When true, identical image bytes (even if referenced via different paths/data URIs)
    // are embedded once and reused via a single XObject resource.
    pub reuse_xobjects: bool,
    // When false, force WinAnsi fonts (no CID/Unicode) for maximum speed.
    pub unicode_support: bool,
    // When false, skip shaping; use direct codepoint->gid mapping for Identity-H fonts.
    pub shape_text: bool,
    pub pdf_version: PdfVersion,
    pub pdf_profile: PdfProfile,
    pub output_intent: Option<OutputIntent>,
    pub document_lang: Option<String>,
    pub document_title: Option<String>,
    pub color_space: ColorSpace,
    // When true, page/form command streams are Flate-compressed.
    pub compress_content_streams: bool,
    // Keep tiny streams uncompressed to avoid compression overhead.
    pub compress_content_stream_min_bytes: usize,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            reuse_xobjects: true,
            unicode_support: true,
            shape_text: true,
            pdf_version: PdfVersion::Pdf17,
            pdf_profile: PdfProfile::None,
            output_intent: None,
            document_lang: None,
            document_title: None,
            color_space: ColorSpace::Rgb,
            compress_content_streams: true,
            compress_content_stream_min_bytes: 128,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfVersion {
    Pdf17,
    Pdf20,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfProfile {
    None,
    PdfA1a,
    PdfA1b,
    PdfA2a,
    PdfA2b,
    PdfA2u,
    PdfA3a,
    PdfA3b,
    PdfA3u,
    PdfA4,
    PdfA4e,
    PdfA4f,
    PdfX4,
    PdfUa1,
    PdfUa2,
    PdfVt1,
    Wtpdf1r,
    Wtpdf1a,
    Tagged,
}

impl PdfProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            PdfProfile::None => "none",
            PdfProfile::PdfA1a => "pdfa1a",
            PdfProfile::PdfA1b => "pdfa1b",
            PdfProfile::PdfA2a => "pdfa2a",
            PdfProfile::PdfA2b => "pdfa2b",
            PdfProfile::PdfA2u => "pdfa2u",
            PdfProfile::PdfA3a => "pdfa3a",
            PdfProfile::PdfA3b => "pdfa3b",
            PdfProfile::PdfA3u => "pdfa3u",
            PdfProfile::PdfA4 => "pdfa4",
            PdfProfile::PdfA4e => "pdfa4e",
            PdfProfile::PdfA4f => "pdfa4f",
            PdfProfile::PdfX4 => "pdfx4",
            PdfProfile::PdfUa1 => "pdfua1",
            PdfProfile::PdfUa2 => "pdfua2",
            PdfProfile::PdfVt1 => "pdfvt1",
            PdfProfile::Wtpdf1r => "wtpdf1r",
            PdfProfile::Wtpdf1a => "wtpdf1a",
            PdfProfile::Tagged => "tagged",
        }
    }

    pub(crate) fn emits_tagged_structure(self) -> bool {
        matches!(
            self,
            PdfProfile::Tagged
                | PdfProfile::PdfUa1
                | PdfProfile::PdfA1a
                | PdfProfile::PdfA2a
                | PdfProfile::PdfA3a
                | PdfProfile::PdfUa2
                | PdfProfile::Wtpdf1r
                | PdfProfile::Wtpdf1a
        )
    }

    pub(crate) fn requires_output_intent(self) -> bool {
        matches!(
            self,
            PdfProfile::PdfA1a
                | PdfProfile::PdfA1b
                | PdfProfile::PdfA2a
                | PdfProfile::PdfA2b
                | PdfProfile::PdfA2u
                | PdfProfile::PdfA3a
                | PdfProfile::PdfA3b
                | PdfProfile::PdfA3u
                | PdfProfile::PdfA4
                | PdfProfile::PdfA4e
                | PdfProfile::PdfA4f
                | PdfProfile::PdfX4
                | PdfProfile::PdfVt1
        )
    }

    pub(crate) fn requires_embedded_fonts(self) -> bool {
        matches!(
            self,
            PdfProfile::PdfA1a
                | PdfProfile::PdfA1b
                | PdfProfile::PdfA2a
                | PdfProfile::PdfA2b
                | PdfProfile::PdfA2u
                | PdfProfile::PdfA3a
                | PdfProfile::PdfA3b
                | PdfProfile::PdfA3u
                | PdfProfile::PdfA4
                | PdfProfile::PdfA4e
                | PdfProfile::PdfA4f
                | PdfProfile::PdfX4
                | PdfProfile::PdfUa1
                | PdfProfile::PdfUa2
                | PdfProfile::PdfVt1
                | PdfProfile::Wtpdf1r
                | PdfProfile::Wtpdf1a
        )
    }

    pub(crate) fn uses_pdfx_page_boxes(self) -> bool {
        matches!(self, PdfProfile::PdfX4 | PdfProfile::PdfVt1)
    }

    pub(crate) fn output_intent_subtype(self) -> &'static str {
        match self {
            PdfProfile::PdfX4 | PdfProfile::PdfVt1 => "GTS_PDFX",
            _ => "GTS_PDFA1",
        }
    }

    pub(crate) fn effective_pdf_version(self, requested: PdfVersion) -> PdfVersion {
        match self {
            PdfProfile::PdfA4 | PdfProfile::PdfA4e | PdfProfile::PdfA4f | PdfProfile::PdfUa2 => {
                PdfVersion::Pdf20
            }
            PdfProfile::Wtpdf1r | PdfProfile::Wtpdf1a => PdfVersion::Pdf20,
            _ => requested,
        }
    }

    pub(crate) fn uses_pdf20_structure_namespace(self) -> bool {
        matches!(
            self,
            PdfProfile::PdfUa2 | PdfProfile::Wtpdf1r | PdfProfile::Wtpdf1a
        )
    }

    pub(crate) fn is_pdfa4_family(self) -> bool {
        matches!(
            self,
            PdfProfile::PdfA4 | PdfProfile::PdfA4e | PdfProfile::PdfA4f
        )
    }
}

fn pdf_header_bytes(version: PdfVersion) -> &'static [u8] {
    match version {
        PdfVersion::Pdf17 => b"%PDF-1.7\n",
        PdfVersion::Pdf20 => b"%PDF-2.0\n",
    }
}

#[derive(Debug, Clone)]
struct TagRecord {
    page_index: usize,
    mcid: Option<u32>,
    role: String,
    alt: Option<String>,
    scope: Option<String>,
    parent: Option<usize>,
    table_id: Option<u32>,
    col_index: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct OutputIntent {
    pub icc_profile: Vec<u8>,
    pub n_components: u8,
    pub identifier: String,
    pub info: Option<String>,
}

impl OutputIntent {
    pub fn new(
        icc_profile: Vec<u8>,
        n_components: u8,
        identifier: impl Into<String>,
        info: Option<String>,
    ) -> Self {
        Self {
            icc_profile,
            n_components,
            identifier: identifier.into(),
            info,
        }
    }
}

const PDF_CATALOG_ID: usize = 1;
const PDF_PAGES_ID: usize = 2;
const PDF_RESOURCES_ID: usize = 3;
const PDF_FILTER_RASTER_DPI: u32 = 300;

// Keep the page tree shallow but avoid huge /Kids arrays for large outputs.
const PDF_PAGE_NODE_MAX_KIDS: usize = 256;

#[derive(Clone)]
struct ShapedText {
    tj: String,
    glyph_map: BTreeMap<u16, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamFontKind {
    Type1,
    TrueTypeWinAnsi,
    TrueTypeIdentityH,
}

struct StreamFont<'a> {
    logical_name: String,
    resource: String,
    encoding: FontEncoding,
    start_id: usize,
    kind: StreamFontKind,
    glyph_map: BTreeMap<u16, String>,
    font_data: Option<&'a [u8]>,
}

struct Type3StreamFont {
    logical_name: String,
    resource: String,
    font_id: usize,
    glyph_ids: BTreeSet<u16>,
    synthetic_bold_millionths: u32,
}

impl StreamFont<'_> {
    fn font_object_id(&self) -> usize {
        match self.kind {
            StreamFontKind::Type1 => self.start_id,
            StreamFontKind::TrueTypeWinAnsi => self.start_id + 2,
            StreamFontKind::TrueTypeIdentityH => self.start_id + 4,
        }
    }
}

struct PdfPageNode {
    id: usize,
    kids: Vec<usize>,
}

enum BindingContentSegment {
    Static(Box<[u8]>),
    Slot(usize),
}

struct BindingPageProgram {
    static_content_id: usize,
    static_content_len: usize,
    geometry: PageGeometry,
    dynamic_segments: Vec<BindingContentSegment>,
    dynamic_capacity: usize,
    dynamic_slot_occurrences: usize,
}

struct CompiledBindingPage {
    static_page: Page,
    overlay_page: Page,
}

/// Immutable command partition produced once for a compiled template.
///
/// PDF resource names and object identifiers remain linker-owned, but the expensive semantic
/// walk that separates static paint from patchable slot paint does not need to run for every
/// binding batch.
pub(crate) struct CompiledBindingPlan {
    pages: Vec<CompiledBindingPage>,
    slot_lookup: HashMap<String, usize>,
    static_command_count: usize,
    overlay_command_count: usize,
}

impl CompiledBindingPlan {
    pub(crate) fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub(crate) fn command_count(&self) -> usize {
        self.static_command_count
            .saturating_add(self.overlay_command_count)
    }
}

fn compile_binding_content_segments(
    content: &str,
    slot_lookup: &HashMap<String, usize>,
) -> (Vec<BindingContentSegment>, usize, usize) {
    let bytes = content.as_bytes();
    let mut segments = Vec::new();
    let mut cursor = 0usize;
    let mut search = 0usize;
    let mut occurrences = 0usize;
    let mut static_bytes = 0usize;

    while search + 3 < bytes.len() {
        let Some(relative_start) = bytes[search..].windows(2).position(|pair| pair == b"{{") else {
            break;
        };
        let start = search + relative_start;
        let Some(relative_end) = bytes[start + 2..].windows(2).position(|pair| pair == b"}}")
        else {
            break;
        };
        let end = start + 2 + relative_end;
        let name = std::str::from_utf8(&bytes[start + 2..end])
            .ok()
            .map(str::trim);
        let Some(slot_index) = name.and_then(|name| slot_lookup.get(name)).copied() else {
            search = start + 2;
            continue;
        };
        if start > cursor {
            let value = bytes[cursor..start].to_vec().into_boxed_slice();
            static_bytes = static_bytes.saturating_add(value.len());
            segments.push(BindingContentSegment::Static(value));
        }
        segments.push(BindingContentSegment::Slot(slot_index));
        occurrences = occurrences.saturating_add(1);
        cursor = end + 2;
        search = cursor;
    }
    if cursor < bytes.len() {
        let value = bytes[cursor..].to_vec().into_boxed_slice();
        static_bytes = static_bytes.saturating_add(value.len());
        segments.push(BindingContentSegment::Static(value));
    }
    (segments, occurrences, static_bytes)
}

fn append_binding_value(out: &mut Vec<u8>, value: &str) {
    if value.is_ascii() {
        for byte in value.bytes() {
            match byte {
                b'\\' => out.extend_from_slice(b"\\\\"),
                b'(' => out.extend_from_slice(b"\\("),
                b')' => out.extend_from_slice(b"\\)"),
                b'\n' => out.extend_from_slice(b"\\n"),
                b'\r' => out.extend_from_slice(b"\\r"),
                0x00..=0x1f | 0x7f => {
                    out.push(b'\\');
                    out.push(b'0' + ((byte >> 6) & 0x07));
                    out.push(b'0' + ((byte >> 3) & 0x07));
                    out.push(b'0' + (byte & 0x07));
                }
                _ => out.push(byte),
            }
        }
    } else {
        let encoded = encode_winansi_pdf_string(value);
        out.extend_from_slice(encoded.text.as_bytes());
    }
}

fn instantiate_binding_content(
    segments: &[BindingContentSegment],
    columns: &[&[String]],
    row: usize,
    out: &mut Vec<u8>,
) {
    out.clear();
    for segment in segments {
        match segment {
            BindingContentSegment::Static(value) => out.extend_from_slice(value),
            BindingContentSegment::Slot(slot_index) => {
                append_binding_value(out, &columns[*slot_index][row]);
            }
        }
    }
}

#[derive(Clone)]
struct BindingPaintState {
    font_name: String,
    font_size: Pt,
    fill: Color,
    stroke: Color,
    opacity_fill: f32,
    opacity_stroke: f32,
    blend_mode: MixBlendMode,
    text_rendering_mode: u8,
}

impl Default for BindingPaintState {
    fn default() -> Self {
        Self {
            font_name: "Helvetica".to_string(),
            font_size: Pt::from_f32(12.0),
            fill: Color::BLACK,
            stroke: Color::BLACK,
            opacity_fill: 1.0,
            opacity_stroke: 1.0,
            blend_mode: MixBlendMode::Normal,
            text_rendering_mode: 0,
        }
    }
}

fn commands_contain_binding_marker(commands: &[Command]) -> bool {
    commands.iter().any(|command| match command {
        Command::DrawString { text, .. } | Command::DrawStringTransformed { text, .. } => {
            !crate::binding_slot_names(text).is_empty()
        }
        Command::DefineForm { commands, .. } | Command::DefineIsolatedForm { commands, .. } => {
            commands_contain_binding_marker(commands)
        }
        _ => false,
    })
}

fn commands_contain_filtered_form(commands: &[Command]) -> bool {
    commands.iter().any(|command| match command {
        Command::DrawFilteredForm { .. } => true,
        Command::DefineForm { commands, .. } | Command::DefineIsolatedForm { commands, .. } => {
            commands_contain_filtered_form(commands)
        }
        _ => false,
    })
}

fn binding_static_commands(
    commands: &[Command],
    slot_lookup: &HashMap<String, usize>,
) -> Vec<Command> {
    commands
        .iter()
        .filter(|command| match command {
            Command::DrawString { text, .. } | Command::DrawStringTransformed { text, .. } => {
                !crate::binding_slot_names(text)
                    .iter()
                    .any(|name| slot_lookup.contains_key(*name))
            }
            _ => true,
        })
        .cloned()
        .collect()
}

fn binding_overlay_commands(
    commands: &[Command],
    slot_lookup: &HashMap<String, usize>,
) -> io::Result<(Vec<Command>, Vec<usize>)> {
    let mut state = BindingPaintState::default();
    let mut stack = Vec::new();
    // Dynamic text is painted in a compact page overlay after immutable page paint. Preserve the
    // active page-space transform and clip program so fixed-geometry slots can execute there
    // without replaying layout or retaining the complete display list. Each SaveState records a
    // bytecode checkpoint; RestoreState discards only the state compiled inside that scope.
    let mut coordinate_program = Vec::<Command>::new();
    let mut coordinate_stack = Vec::<usize>::new();
    let mut current_path = Vec::<Command>::new();
    let mut overlay = Vec::new();
    let mut counts = vec![0usize; slot_lookup.len()];

    for command in commands {
        match command {
            Command::SaveState => {
                stack.push(state.clone());
                coordinate_stack.push(coordinate_program.len());
            }
            Command::RestoreState => {
                if let Some(saved) = stack.pop() {
                    state = saved;
                }
                if let Some(checkpoint) = coordinate_stack.pop() {
                    coordinate_program.truncate(checkpoint);
                }
            }
            command @ (Command::Translate(..)
            | Command::CssTransformOrigin { .. }
            | Command::Scale(..)
            | Command::Rotate(..)
            | Command::ConcatMatrix { .. }
            | Command::ClipRect { .. }) => coordinate_program.push((*command).clone()),
            command @ (Command::MoveTo { .. }
            | Command::LineTo { .. }
            | Command::CurveTo { .. }
            | Command::ClosePath) => current_path.push((*command).clone()),
            command @ Command::ClipPath { .. } => {
                coordinate_program.append(&mut current_path);
                coordinate_program.push((*command).clone());
            }
            Command::Fill
            | Command::FillEvenOdd
            | Command::Stroke
            | Command::FillStroke
            | Command::FillStrokeEvenOdd => current_path.clear(),
            Command::SetFillColor(value) => state.fill = *value,
            Command::SetStrokeColor(value) => state.stroke = *value,
            Command::SetOpacity { fill, stroke } => {
                state.opacity_fill = *fill;
                state.opacity_stroke = *stroke;
            }
            Command::SetBlendMode { mode } => state.blend_mode = *mode,
            Command::SetFontName(value) => state.font_name.clone_from(value),
            Command::SetFontSize(value) => state.font_size = *value,
            Command::SetTextRenderingMode(value) => state.text_rendering_mode = *value,
            Command::DefineForm { commands, .. } | Command::DefineIsolatedForm { commands, .. }
                if commands_contain_binding_marker(commands) =>
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "compiled binding slots inside form XObjects are not supported",
                ));
            }
            Command::DrawString { x, y, text } => {
                let names = crate::binding_slot_names(text);
                if names.is_empty() {
                    continue;
                }
                let slot_indices = names
                    .iter()
                    .filter_map(|name| slot_lookup.get(*name).copied())
                    .collect::<Vec<_>>();
                if slot_indices.is_empty() {
                    continue;
                }
                for slot_index in slot_indices {
                    counts[slot_index] = counts[slot_index].saturating_add(1);
                }
                overlay.push(Command::SaveState);
                overlay.extend(coordinate_program.iter().cloned());
                overlay.push(Command::SetFillColor(state.fill));
                overlay.push(Command::SetStrokeColor(state.stroke));
                if state.opacity_fill < 1.0 || state.opacity_stroke < 1.0 {
                    overlay.push(Command::SetOpacity {
                        fill: state.opacity_fill,
                        stroke: state.opacity_stroke,
                    });
                }
                if state.blend_mode != MixBlendMode::Normal {
                    overlay.push(Command::SetBlendMode {
                        mode: state.blend_mode,
                    });
                }
                overlay.push(Command::SetFontName(state.font_name.clone()));
                overlay.push(Command::SetFontSize(state.font_size));
                if state.text_rendering_mode != 0 {
                    overlay.push(Command::SetTextRenderingMode(state.text_rendering_mode));
                }
                overlay.push(Command::DrawString {
                    x: *x,
                    y: *y,
                    text: text.clone(),
                });
                overlay.push(Command::RestoreState);
            }
            Command::DrawStringTransformed {
                x,
                y,
                text,
                m00,
                m01,
                m10,
                m11,
            } => {
                let names = crate::binding_slot_names(text);
                if names.is_empty() {
                    continue;
                }
                let slot_indices = names
                    .iter()
                    .filter_map(|name| slot_lookup.get(*name).copied())
                    .collect::<Vec<_>>();
                if slot_indices.is_empty() {
                    continue;
                }
                for slot_index in slot_indices {
                    counts[slot_index] = counts[slot_index].saturating_add(1);
                }
                overlay.push(Command::SaveState);
                overlay.extend(coordinate_program.iter().cloned());
                overlay.push(Command::SetFillColor(state.fill));
                overlay.push(Command::SetStrokeColor(state.stroke));
                if state.opacity_fill < 1.0 || state.opacity_stroke < 1.0 {
                    overlay.push(Command::SetOpacity {
                        fill: state.opacity_fill,
                        stroke: state.opacity_stroke,
                    });
                }
                if state.blend_mode != MixBlendMode::Normal {
                    overlay.push(Command::SetBlendMode {
                        mode: state.blend_mode,
                    });
                }
                overlay.push(Command::SetFontName(state.font_name.clone()));
                overlay.push(Command::SetFontSize(state.font_size));
                if state.text_rendering_mode != 0 {
                    overlay.push(Command::SetTextRenderingMode(state.text_rendering_mode));
                }
                overlay.push(Command::DrawStringTransformed {
                    x: *x,
                    y: *y,
                    text: text.clone(),
                    m00: *m00,
                    m01: *m01,
                    m10: *m10,
                    m11: *m11,
                });
                overlay.push(Command::RestoreState);
            }
            _ => {}
        }
    }
    Ok((overlay, counts))
}

pub(crate) fn compile_binding_plan(
    document: &Document,
    binding_slots: &[String],
) -> io::Result<CompiledBindingPlan> {
    let slot_lookup = binding_slots
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut total_slot_counts = vec![0usize; binding_slots.len()];
    let mut pages = Vec::with_capacity(document.pages.len());
    let mut static_command_count = 0usize;
    let mut overlay_command_count = 0usize;

    for page in &document.pages {
        let (overlay_commands, page_slot_counts) =
            binding_overlay_commands(&page.commands, &slot_lookup)?;
        let static_commands = binding_static_commands(&page.commands, &slot_lookup);
        for (total, count) in total_slot_counts.iter_mut().zip(page_slot_counts) {
            *total = total.saturating_add(count);
        }
        static_command_count = static_command_count.saturating_add(static_commands.len());
        overlay_command_count = overlay_command_count.saturating_add(overlay_commands.len());
        pages.push(CompiledBindingPage {
            static_page: Page {
                commands: static_commands,
            },
            overlay_page: Page {
                commands: overlay_commands,
            },
        });
    }

    for (index, count) in total_slot_counts.into_iter().enumerate() {
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "compiled binding slot {:?} is not a page-local text command",
                    binding_slots[index]
                ),
            ));
        }
    }

    Ok(CompiledBindingPlan {
        pages,
        slot_lookup,
        static_command_count,
        overlay_command_count,
    })
}

pub(crate) struct PdfStreamWriter<'a, W: Write> {
    writer: &'a mut W,
    offset: usize,
    offsets: Vec<usize>, // index by object id; 0 is the free object.
    next_id: usize,
    page_size: Size,
    options: PdfOptions,
    registry: Option<&'a FontRegistry>,
    debug: Option<std::sync::Arc<crate::debug::DebugLogger>>,
    perf: Option<std::sync::Arc<PerfLogger>>,

    // Resources
    fonts: BTreeMap<String, StreamFont<'a>>,
    next_font_resource: usize,
    type3_fonts: BTreeMap<(String, u8, u32), Type3StreamFont>,
    next_type3_resource: usize,
    current_doc_id: usize,
    doc_font_usage: BTreeMap<usize, BTreeSet<String>>,

    image_resources: Vec<(String, usize)>,
    image_name_map: HashMap<String, String>,
    image_crop_map: HashMap<String, Option<ResolvedImageSourceCrop>>,
    image_content_map: HashMap<u64, (String, usize)>,
    next_image_index: usize,
    image_bytes_total: usize,

    form_resources: Vec<(String, usize)>,
    form_name_map: HashMap<String, String>,
    form_content_map: HashMap<u64, (String, usize)>,
    form_size_map: HashMap<String, Size>,
    form_definition_map: HashMap<String, (Pt, Pt, Vec<Command>)>,
    form_isolated_map: HashMap<String, bool>,
    masked_form_raster_cache: HashMap<String, Option<crate::raster::FilteredFormRaster>>,
    mask_coverage_raster_cache: HashMap<String, Option<crate::raster::MaskCoverageRaster>>,
    mask_coverage_gs_map: HashMap<String, String>,
    next_form_index: usize,

    gs_resources: Vec<(String, usize)>,
    gs_name_map: HashMap<(u16, u16), String>,
    gs_blend_name_map: HashMap<MixBlendMode, String>,
    next_gs_index: usize,

    shading_resources: Vec<(String, usize)>,
    shading_name_map: HashMap<u64, String>,
    shading_alpha_gs_map: HashMap<u64, String>,
    next_shading_index: usize,

    optional_content_names: BTreeSet<String>,

    // Page tree
    page_nodes: Vec<PdfPageNode>,
    current_node: Option<PdfPageNode>,

    // Text shaping cache (per document)
    shaped_cache: HashMap<String, ShapedText>,

    // Tagged PDF state
    tag_records: Vec<TagRecord>,
    page_ids: Vec<usize>,
    page_content_bytes: Vec<usize>,
    page_content_stream_count: usize,
    page_content_reused_references: usize,
    content_stream_raw_bytes: usize,
    content_stream_encoded_bytes: usize,
    content_stream_compressed_count: usize,
    font_program_source_bytes: usize,
    font_program_subset_bytes: usize,
    font_program_encoded_bytes: usize,
    font_program_subset_count: usize,
    font_program_compressed_count: usize,
    font_program_subset_glyphs: usize,
    pdfvt_dpart_root_id: Option<usize>,
    pdfvt_dpart_node_id: Option<usize>,
}

impl<'a, W: Write> PdfStreamWriter<'a, W> {
    pub(crate) fn new(
        writer: &'a mut W,
        page_size: Size,
        registry: Option<&'a FontRegistry>,
        options: PdfOptions,
        debug: Option<std::sync::Arc<crate::debug::DebugLogger>>,
        perf: Option<std::sync::Arc<PerfLogger>>,
    ) -> io::Result<Self> {
        let mut options = options;
        options.pdf_version = options
            .pdf_profile
            .effective_pdf_version(options.pdf_version);
        validate_profile_output_intent(&options)?;
        let mut offset: usize = 0;
        write_bytes(writer, pdf_header_bytes(options.pdf_version), &mut offset)?;
        write_bytes(writer, b"%\xE2\xE3\xCF\xD3\n", &mut offset)?;
        let mut next_id = PDF_RESOURCES_ID + 1;
        let (pdfvt_dpart_root_id, pdfvt_dpart_node_id) =
            if options.pdf_profile == PdfProfile::PdfVt1 {
                let root_id = next_id;
                let node_id = next_id + 1;
                next_id += 2;
                (Some(root_id), Some(node_id))
            } else {
                (None, None)
            };

        let s = Self {
            writer,
            offset,
            offsets: vec![0; next_id],
            next_id,
            page_size,
            options,
            registry,
            debug,
            perf,
            fonts: BTreeMap::new(),
            next_font_resource: 1,
            type3_fonts: BTreeMap::new(),
            next_type3_resource: 1,
            current_doc_id: 0,
            doc_font_usage: BTreeMap::new(),
            image_resources: Vec::new(),
            image_name_map: HashMap::new(),
            image_crop_map: HashMap::new(),
            image_content_map: HashMap::new(),
            next_image_index: 1,
            image_bytes_total: 0,
            form_resources: Vec::new(),
            form_name_map: HashMap::new(),
            form_content_map: HashMap::new(),
            form_size_map: HashMap::new(),
            form_definition_map: HashMap::new(),
            form_isolated_map: HashMap::new(),
            masked_form_raster_cache: HashMap::new(),
            mask_coverage_raster_cache: HashMap::new(),
            mask_coverage_gs_map: HashMap::new(),
            next_form_index: 1,
            gs_resources: Vec::new(),
            gs_name_map: HashMap::new(),
            gs_blend_name_map: HashMap::new(),
            next_gs_index: 1,
            shading_resources: Vec::new(),
            shading_name_map: HashMap::new(),
            shading_alpha_gs_map: HashMap::new(),
            next_shading_index: 1,
            optional_content_names: BTreeSet::new(),
            page_nodes: Vec::new(),
            current_node: None,
            shaped_cache: HashMap::new(),
            tag_records: Vec::new(),
            page_ids: Vec::new(),
            page_content_bytes: Vec::new(),
            page_content_stream_count: 0,
            page_content_reused_references: 0,
            content_stream_raw_bytes: 0,
            content_stream_encoded_bytes: 0,
            content_stream_compressed_count: 0,
            font_program_source_bytes: 0,
            font_program_subset_bytes: 0,
            font_program_encoded_bytes: 0,
            font_program_subset_count: 0,
            font_program_compressed_count: 0,
            font_program_subset_glyphs: 0,
            pdfvt_dpart_root_id,
            pdfvt_dpart_node_id,
        };

        Ok(s)
    }

    pub(crate) fn add_document(&mut self, doc_id: usize, document: &Document) -> io::Result<()> {
        // Guardrail: multi-doc streaming assumes a single page size.
        if (document.page_size.width - self.page_size.width).abs() > Pt::from_f32(0.01)
            || (document.page_size.height - self.page_size.height).abs() > Pt::from_f32(0.01)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mixed page sizes are not supported in a single PDF stream",
            ));
        }
        validate_profile_font_embedding(document, self.registry, &self.options)?;
        self.current_doc_id = doc_id;
        self.shaped_cache.clear();
        for page in &document.pages {
            self.add_page(page)?;
        }
        Ok(())
    }

    /// Add identical copies of one already-compiled document while writing each unique page
    /// content stream only once. Page dictionaries remain distinct and ordered, but point at the
    /// shared immutable stream and global resource dictionary.
    pub(crate) fn add_compiled_document_copies(
        &mut self,
        doc_id: usize,
        document: &Document,
        copies: usize,
    ) -> io::Result<()> {
        if copies == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "compiled document copy count must be greater than zero",
            ));
        }
        if copies == 1 {
            return self.add_document(doc_id, document);
        }
        // Tagged content uses page-specific structure records. Keep the fully general path until
        // structure-record virtualization is implemented.
        if self.options.pdf_profile.emits_tagged_structure() {
            for copy in 0..copies {
                self.add_document(doc_id.saturating_add(copy), document)?;
            }
            return Ok(());
        }
        if (document.page_size.width - self.page_size.width).abs() > Pt::from_f32(0.01)
            || (document.page_size.height - self.page_size.height).abs() > Pt::from_f32(0.01)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mixed page sizes are not supported in a single PDF stream",
            ));
        }
        validate_profile_font_embedding(document, self.registry, &self.options)?;
        self.current_doc_id = doc_id;
        self.shaped_cache.clear();
        self.ensure_page_node();

        let first_page_index = self.page_ids.len();
        let mut shared_contents = Vec::with_capacity(document.pages.len());
        for (source_index, page) in document.pages.iter().enumerate() {
            let content_id = self.alloc_ids(1);
            let content_stream = self.render_page(page, first_page_index + source_index)?;
            let content_len = content_stream.len();
            let geometry = PageGeometry::for_page(page, self.page_size);
            self.write_content_stream_object(content_id, "", content_stream.as_bytes())?;
            self.page_content_stream_count = self.page_content_stream_count.saturating_add(1);
            shared_contents.push((content_id, content_len, geometry));
        }

        self.page_content_reused_references = self.page_content_reused_references.saturating_add(
            document
                .pages
                .len()
                .saturating_mul(copies.saturating_sub(1)),
        );

        if let Some(usage) = self.doc_font_usage.get(&doc_id).cloned() {
            for copy in 1..copies {
                self.doc_font_usage
                    .insert(doc_id.saturating_add(copy), usage.clone());
            }
        }
        for _copy in 0..copies {
            for (content_id, content_len, geometry) in &shared_contents {
                self.add_page_reference_sized(*content_id, *content_len, *geometry)?;
            }
        }
        Ok(())
    }

    /// Add a columnar fixed-geometry binding batch. Static page paint is written once per source
    /// page; every output page receives only a compact, record-specific text overlay stream.
    pub(crate) fn add_compiled_document_bindings(
        &mut self,
        doc_id: usize,
        document: &Document,
        binding_plan: &CompiledBindingPlan,
        columns: &[&[String]],
        record_count: usize,
    ) -> io::Result<()> {
        if record_count == 0
            || binding_plan.slot_lookup.is_empty()
            || columns.len() != binding_plan.slot_lookup.len()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "compiled binding batch requires non-empty, aligned slot columns",
            ));
        }
        if columns.iter().any(|column| column.len() != record_count) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "compiled binding columns have inconsistent row counts",
            ));
        }
        if self.options.pdf_profile.emits_tagged_structure() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "compiled fixed-geometry bindings do not yet support tagged page structure",
            ));
        }
        if (document.page_size.width - self.page_size.width).abs() > Pt::from_f32(0.01)
            || (document.page_size.height - self.page_size.height).abs() > Pt::from_f32(0.01)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mixed page sizes are not supported in a compiled binding batch",
            ));
        }
        validate_profile_font_embedding(document, self.registry, &self.options)?;
        self.current_doc_id = doc_id;
        self.shaped_cache.clear();
        self.ensure_page_node();

        if binding_plan.pages.len() != document.pages.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "compiled binding plan page count does not match its display document",
            ));
        }
        let mut programs = Vec::with_capacity(binding_plan.pages.len());

        for page in &binding_plan.pages {
            let page_index = self.page_ids.len();
            let geometry = PageGeometry::for_page(&page.static_page, self.page_size);
            let static_content = self.render_page(&page.static_page, page_index)?;
            let static_content_id = self.alloc_ids(1);
            self.write_content_stream_object(static_content_id, "", static_content.as_bytes())?;
            self.page_content_stream_count = self.page_content_stream_count.saturating_add(1);

            let overlay_template =
                self.render_page_sized(&page.overlay_page, page_index, geometry)?;
            let (dynamic_segments, dynamic_slot_occurrences, dynamic_static_bytes) =
                compile_binding_content_segments(&overlay_template, &binding_plan.slot_lookup);
            if dynamic_slot_occurrences == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "compiled slot overlay could not be lowered to patchable WinAnsi text",
                ));
            }
            programs.push(BindingPageProgram {
                static_content_id,
                static_content_len: static_content.len(),
                geometry,
                dynamic_segments,
                dynamic_capacity: dynamic_static_bytes.saturating_add(128),
                dynamic_slot_occurrences,
            });
        }

        let max_dynamic_capacity = programs
            .iter()
            .map(|program| program.dynamic_capacity)
            .max()
            .unwrap_or(0);
        let mut dynamic_content = Vec::with_capacity(max_dynamic_capacity);
        for row in 0..record_count {
            for program in &programs {
                instantiate_binding_content(
                    &program.dynamic_segments,
                    columns,
                    row,
                    &mut dynamic_content,
                );
                self.add_bound_page(
                    program.static_content_id,
                    program.static_content_len,
                    &dynamic_content,
                    program.geometry,
                )?;
                debug_assert!(program.dynamic_slot_occurrences > 0);
            }
        }
        Ok(())
    }

    fn add_page(&mut self, page: &Page) -> io::Result<()> {
        let page_index = self.page_ids.len();
        let geometry = PageGeometry::for_page(page, self.page_size);
        let parent_id = self.ensure_page_node();
        let start = self.alloc_ids(2);
        let content_id = start;
        let page_id = start + 1;

        if let Some(node) = self.current_node.as_mut() {
            node.kids.push(page_id);
        }

        let content_stream = self.render_page(page, page_index)?;
        self.page_content_bytes.push(content_stream.len());
        self.write_content_stream_object(content_id, "", content_stream.as_bytes())?;
        self.page_content_stream_count = self.page_content_stream_count.saturating_add(1);
        self.write_page_reference_sized(parent_id, content_id, page_id, page_index, geometry)
    }

    fn add_page_reference_sized(
        &mut self,
        content_id: usize,
        content_len: usize,
        geometry: PageGeometry,
    ) -> io::Result<()> {
        let page_index = self.page_ids.len();
        let parent_id = self.ensure_page_node();
        let page_id = self.alloc_ids(1);
        if let Some(node) = self.current_node.as_mut() {
            node.kids.push(page_id);
        }
        self.page_content_bytes.push(content_len);
        self.write_page_reference_sized(parent_id, content_id, page_id, page_index, geometry)
    }

    fn add_bound_page(
        &mut self,
        static_content_id: usize,
        static_content_len: usize,
        dynamic_content: &[u8],
        geometry: PageGeometry,
    ) -> io::Result<()> {
        let page_index = self.page_ids.len();
        let parent_id = self.ensure_page_node();
        let start = self.alloc_ids(2);
        let dynamic_content_id = start;
        let page_id = start + 1;
        if let Some(node) = self.current_node.as_mut() {
            node.kids.push(page_id);
        }
        self.page_content_bytes
            .push(static_content_len.saturating_add(dynamic_content.len()));
        self.write_uncompressed_content_stream_object(dynamic_content_id, "", dynamic_content)?;
        self.page_content_stream_count = self.page_content_stream_count.saturating_add(1);
        let contents = format!("[{} 0 R {} 0 R]", static_content_id, dynamic_content_id);
        self.write_page_reference_contents_sized(
            parent_id, &contents, page_id, page_index, geometry,
        )
    }

    fn write_page_reference_sized(
        &mut self,
        parent_id: usize,
        content_id: usize,
        page_id: usize,
        page_index: usize,
        geometry: PageGeometry,
    ) -> io::Result<()> {
        self.write_page_reference_contents_sized(
            parent_id,
            &format!("{} 0 R", content_id),
            page_id,
            page_index,
            geometry,
        )
    }

    fn write_page_reference_contents_sized(
        &mut self,
        parent_id: usize,
        contents: &str,
        page_id: usize,
        page_index: usize,
        geometry: PageGeometry,
    ) -> io::Result<()> {
        self.page_ids.push(page_id);

        let (struct_parents, tabs) = if self.options.pdf_profile.emits_tagged_structure() {
            (format!(" /StructParents {}", page_index), " /Tabs /S")
        } else {
            (String::new(), "")
        };
        let page_boxes = page_box_entries(self.options.pdf_profile, geometry);
        let dpart = self
            .pdfvt_dpart_node_id
            .map(|id| format!(" /DPart {} 0 R", id))
            .unwrap_or_default();
        let page_obj = format!(
            "<< /Type /Page /Parent {} 0 R /MediaBox [0 0 {} {}]{} /Resources {} 0 R /Contents {}{}{}{} >>",
            parent_id,
            fmt_pt(geometry.media_size.width),
            fmt_pt(geometry.media_size.height),
            page_boxes,
            PDF_RESOURCES_ID,
            contents,
            dpart,
            struct_parents,
            tabs
        );
        self.write_object(page_id, &page_obj)
    }

    pub(crate) fn finish(&mut self) -> io::Result<usize> {
        let t_finish = std::time::Instant::now();
        if let Some(node) = self.current_node.take() {
            self.page_nodes.push(node);
        }

        // 1) Fonts (some objects were allocated early but not written yet).
        let fonts = std::mem::take(&mut self.fonts);
        let type3_fonts = std::mem::take(&mut self.type3_fonts);
        let doc_font_usage = std::mem::take(&mut self.doc_font_usage);

        if let Some(logger) = self.debug.as_deref() {
            let mut glyph_counts: BTreeMap<String, usize> = BTreeMap::new();
            for (key, font_state) in &fonts {
                glyph_counts.insert(key.clone(), font_state.glyph_map.len());
            }

            let mut doc_map: BTreeMap<usize, Vec<(String, usize)>> = BTreeMap::new();
            for (doc_id, names) in doc_font_usage {
                let mut rows: Vec<(String, usize)> = Vec::new();
                for name in names {
                    let key = normalize_font_key(&name);
                    let glyphs = glyph_counts.get(&key).copied().unwrap_or(0);
                    rows.push((name, glyphs));
                }
                rows.sort_by(|a, b| a.0.cmp(&b.0));
                doc_map.insert(doc_id, rows);
            }

            let mut out = String::from("{\"type\":\"jit.fonts\",\"docs\":[");
            let mut first_doc = true;
            for (doc_id, fonts) in doc_map {
                if !first_doc {
                    out.push(',');
                }
                first_doc = false;
                out.push_str(&format!("{{\"doc_id\":{},\"fonts\":[", doc_id));
                let mut first_font = true;
                for (name, glyphs) in fonts {
                    if !first_font {
                        out.push(',');
                    }
                    first_font = false;
                    out.push_str(&format!(
                        "{{\"name\":\"{}\",\"glyphs\":{}}}",
                        json_escape(&name),
                        glyphs
                    ));
                }
                out.push_str("]}");
            }
            out.push_str("]}");
            logger.log_json(&out);
        }

        if let Some(registry) = self.registry {
            for (_name, font_state) in &fonts {
                match font_state.kind {
                    StreamFontKind::Type1 => {
                        self.write_object(
                            font_state.start_id,
                            &font_object(&font_state.logical_name),
                        )?;
                    }
                    StreamFontKind::TrueTypeWinAnsi => {
                        let Some(font) = registry.resolve(&font_state.logical_name) else {
                            return Err(io::Error::new(
                                io::ErrorKind::NotFound,
                                format!("font not found in registry: {}", font_state.logical_name),
                            ));
                        };
                        let font_file_id = font_state.start_id;
                        let descriptor_id = font_state.start_id + 1;
                        let font_id = font_state.start_id + 2;
                        let used_gids: BTreeSet<u16> = font
                            .metrics
                            .glyph_ids
                            .iter()
                            .copied()
                            .filter(|glyph| *glyph != 0)
                            .collect();
                        let subset = registry.cached_truetype_subset(&font.name, &used_gids);
                        let base_name =
                            subset_font_name(font, subset.as_ref().map(|value| &value.tag));
                        let program = subset
                            .as_ref()
                            .map(|value| value.data.as_slice())
                            .unwrap_or(font.data.as_slice());
                        self.write_font_file_stream_object(
                            font_file_id,
                            program,
                            subset
                                .as_ref()
                                .and_then(|value| value.compressed.as_deref()),
                            font.program_kind,
                            font.data.len(),
                            subset.as_ref().map(|value| value.glyph_count),
                        )?;
                        self.write_object(
                            descriptor_id,
                            &font_descriptor_object(font, font_file_id, &base_name),
                        )?;
                        self.write_object(
                            font_id,
                            &truetype_font_object(font, descriptor_id, &base_name),
                        )?;
                    }
                    StreamFontKind::TrueTypeIdentityH => {
                        let Some(font) = registry.resolve(&font_state.logical_name) else {
                            return Err(io::Error::new(
                                io::ErrorKind::NotFound,
                                format!("font not found in registry: {}", font_state.logical_name),
                            ));
                        };
                        let font_file_id = font_state.start_id;
                        let descriptor_id = font_state.start_id + 1;
                        let cid_font_id = font_state.start_id + 2;
                        let to_unicode_id = font_state.start_id + 3;
                        let type0_font_id = font_state.start_id + 4;

                        let mut glyph_map = font_state.glyph_map.clone();
                        if glyph_map.is_empty() {
                            let gid = registry.map_glyph_id_for_char(&font.name, ' ');
                            if gid != 0 {
                                glyph_map.insert(gid, " ".to_string());
                            }
                        }
                        let used_gids: BTreeSet<u16> = glyph_map.keys().copied().collect();
                        let truetype_subset =
                            registry.cached_truetype_subset(&font.name, &used_gids);
                        let cff_subset = registry.cached_cff_subset(&font.name, &used_gids);
                        let subset_tag = truetype_subset
                            .as_ref()
                            .map(|value| &value.tag)
                            .or_else(|| cff_subset.as_ref().map(|value| &value.tag));
                        let base_name = subset_font_name(font, subset_tag);
                        let program = truetype_subset
                            .as_ref()
                            .map(|value| value.data.as_slice())
                            .or_else(|| cff_subset.as_ref().map(|value| value.data.as_slice()))
                            .unwrap_or(font.data.as_slice());
                        let compressed = truetype_subset
                            .as_ref()
                            .and_then(|value| value.compressed.as_deref())
                            .or_else(|| {
                                cff_subset
                                    .as_ref()
                                    .and_then(|value| value.compressed.as_deref())
                            });
                        let subset_glyph_count = truetype_subset
                            .as_ref()
                            .map(|value| value.glyph_count)
                            .or_else(|| cff_subset.as_ref().map(|value| value.glyph_count));
                        self.write_font_file_stream_object(
                            font_file_id,
                            program,
                            compressed,
                            font.program_kind,
                            font.data.len(),
                            subset_glyph_count,
                        )?;
                        self.write_object(
                            descriptor_id,
                            &font_descriptor_object(font, font_file_id, &base_name),
                        )?;

                        let mut w_entries: Vec<String> = Vec::new();
                        for gid in &used_gids {
                            let width = registry
                                .glyph_advance_units(&font.name, *gid)
                                .filter(|(advance, _)| *advance > 0)
                                .map(|(advance, units_per_em)| {
                                    format_font_units(i64::from(advance), units_per_em)
                                })
                                .unwrap_or_else(|| font.metrics.missing_width.to_string());
                            let cid = cff_subset
                                .as_ref()
                                .and_then(|subset| subset.old_to_new.get(gid))
                                .copied()
                                .unwrap_or(*gid);
                            w_entries.push(format!("{} [{}]", cid, width));
                        }
                        let w_array = if w_entries.is_empty() {
                            String::new()
                        } else {
                            format!("/W [{}]", w_entries.join(" "))
                        };
                        let (cid_subtype, cid_to_gid_map, encoding) = match font.program_kind {
                            FontProgramKind::OpenTypeCff => {
                                let encoding = if let Some(subset) = cff_subset.as_ref() {
                                    let encoding_id = self.alloc_ids(1);
                                    let cmap = cid_encoding_cmap(&subset.old_to_new);
                                    self.write_stream_object_bytes(
                                        encoding_id,
                                        "",
                                        cmap.as_bytes(),
                                    )?;
                                    format!("{} 0 R", encoding_id)
                                } else {
                                    "/Identity-H".to_string()
                                };
                                ("CIDFontType0", "", encoding)
                            }
                            FontProgramKind::TrueType => (
                                "CIDFontType2",
                                "/CIDToGIDMap /Identity",
                                "/Identity-H".to_string(),
                            ),
                        };
                        self.write_object(
                            cid_font_id,
                            &format!(
                                "<< /Type /Font /Subtype /{} /BaseFont /{} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {} 0 R {} {} >>",
                                cid_subtype,
                                base_name,
                                descriptor_id,
                                w_array,
                                cid_to_gid_map,
                            ),
                        )?;

                        let to_unicode = to_unicode_cmap(&glyph_map);
                        self.write_stream_object_bytes(to_unicode_id, "", to_unicode.as_bytes())?;
                        self.write_object(
                            type0_font_id,
                            &format!(
                                "<< /Type /Font /Subtype /Type0 /BaseFont /{} /Encoding {} /DescendantFonts [{} 0 R] /ToUnicode {} 0 R >>",
                                base_name,
                                encoding,
                                cid_font_id,
                                to_unicode_id
                            ),
                        )?;
                    }
                }
            }
        } else {
            // No registry: still emit basic Type1 font objects.
            for (_name, font_state) in &fonts {
                if font_state.kind == StreamFontKind::Type1 {
                    self.write_object(font_state.start_id, &font_object(&font_state.logical_name))?;
                }
            }
        }

        if let Some(registry) = self.registry {
            for type3_font in type3_fonts.values() {
                self.write_type3_font(type3_font, registry)?;
            }
        }

        let optional_content_names = std::mem::take(&mut self.optional_content_names);
        let mut optional_content_entries: Vec<(String, usize)> = Vec::new();
        for name in optional_content_names {
            let obj_id = self.alloc_ids(1);
            self.write_object(obj_id, &optional_content_group_object(&name))?;
            optional_content_entries.push((name, obj_id));
        }

        // 2) Resources dictionary (referenced by every page).
        let mut font_entries: Vec<(String, usize)> = Vec::new();
        for (_name, font_state) in &fonts {
            font_entries.push((font_state.resource.clone(), font_state.font_object_id()));
        }
        for font_state in type3_fonts.values() {
            font_entries.push((font_state.resource.clone(), font_state.font_id));
        }
        let mut resources = vec![format!("/Font {}", font_resources(&font_entries))];
        let mut xobjects: Vec<(String, usize)> = Vec::new();
        xobjects.extend(self.image_resources.iter().cloned());
        xobjects.extend(self.form_resources.iter().cloned());
        if !xobjects.is_empty() {
            resources.push(format!("/XObject {}", xobject_resources(&xobjects)));
        }
        if !self.gs_resources.is_empty() {
            resources.push(format!(
                "/ExtGState {}",
                extgstate_resources(&self.gs_resources)
            ));
        }
        if !self.shading_resources.is_empty() {
            resources.push(format!(
                "/Shading {}",
                shading_resources(&self.shading_resources)
            ));
        }
        if !optional_content_entries.is_empty() {
            resources.push(format!(
                "/Properties {}",
                optional_content_resources(&optional_content_entries)
            ));
        }
        self.write_object(PDF_RESOURCES_ID, &format!("<< {} >>", resources.join(" ")))?;

        // 3) Page tree nodes + root.
        let page_nodes = std::mem::take(&mut self.page_nodes);
        for node in &page_nodes {
            self.write_object(
                node.id,
                &format!(
                    "<< /Type /Pages /Parent {} 0 R /Count {} /Kids [{}] >>",
                    PDF_PAGES_ID,
                    node.kids.len(),
                    node.kids
                        .iter()
                        .map(|id| format!("{} 0 R", id))
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
            )?;
        }

        let total_pages: usize = page_nodes.iter().map(|n| n.kids.len()).sum();
        let kids = page_nodes
            .iter()
            .map(|n| format!("{} 0 R", n.id))
            .collect::<Vec<_>>()
            .join(" ");
        self.write_object(
            PDF_PAGES_ID,
            &format!("<< /Type /Pages /Count {} /Kids [{}] >>", total_pages, kids),
        )?;

        // 4) Tagged PDF structure (optional).
        let mut struct_tree_root_id: Option<usize> = None;
        if self.options.pdf_profile.emits_tagged_structure() {
            let uses_pdf20_structure_namespace =
                self.options.pdf_profile.uses_pdf20_structure_namespace();
            let tag_records = std::mem::take(&mut self.tag_records);
            let tag_count = tag_records.len();
            let extra_pdf20_structure_objects = if uses_pdf20_structure_namespace { 2 } else { 0 };
            let start_id = self.alloc_ids(tag_count + 2 + extra_pdf20_structure_objects);
            let parent_tree_id = start_id + tag_count;
            let root_id = start_id + tag_count + 1;
            let document_node_id =
                uses_pdf20_structure_namespace.then_some(start_id + tag_count + 2);
            let namespace_id = uses_pdf20_structure_namespace.then_some(start_id + tag_count + 3);

            let mut children: Vec<Vec<usize>> = vec![Vec::new(); tag_count];
            for (idx, tag) in tag_records.iter().enumerate() {
                if let Some(parent) = tag.parent {
                    if let Some(list) = children.get_mut(parent) {
                        list.push(idx);
                    }
                }
            }
            let mut page_parent_tree: Vec<Vec<Option<usize>>> =
                vec![Vec::new(); self.page_ids.len()];
            let mut header_map: HashMap<(u32, u16), usize> = HashMap::new();
            for (idx, tag) in tag_records.iter().enumerate() {
                if tag.role == "TH" {
                    if let (Some(table_id), Some(col)) = (tag.table_id, tag.col_index) {
                        header_map.entry((table_id, col)).or_insert(start_id + idx);
                    }
                }
            }
            let mut root_kids: Vec<usize> = Vec::new();
            for (i, tag) in tag_records.iter().enumerate() {
                if let Some(page_id) = self.page_ids.get(tag.page_index).copied() {
                    let id = start_id + i;
                    let role = escape_pdf_name(&tag.role);
                    let parent_id = tag
                        .parent
                        .map(|p| start_id + p)
                        .or(document_node_id)
                        .unwrap_or(root_id);
                    let mut k_parts: Vec<String> = Vec::new();
                    if let Some(mcid) = tag.mcid {
                        k_parts.push(format!("{}", mcid));
                    }
                    if let Some(kids) = children.get(i) {
                        for child in kids {
                            k_parts.push(format!("{} 0 R", start_id + *child));
                        }
                    }
                    let k_entry = if k_parts.is_empty() {
                        "[]".to_string()
                    } else if k_parts.len() == 1 {
                        k_parts[0].clone()
                    } else {
                        format!("[{}]", k_parts.join(" "))
                    };
                    let mut obj = format!(
                        "<< /Type /StructElem /S /{} /P {} 0 R /Pg {} 0 R /K {}",
                        role, parent_id, page_id, k_entry
                    );
                    if let Some(alt) = tag.alt.as_deref() {
                        obj.push_str(&format!(" /Alt ({})", escape_pdf_string(alt)));
                    }
                    if let Some(scope) = tag.scope.as_deref() {
                        obj.push_str(&format!(" /Scope /{}", escape_pdf_name(scope)));
                    }
                    if tag.role == "TD" {
                        if let (Some(table_id), Some(col)) = (tag.table_id, tag.col_index) {
                            if let Some(th_id) = header_map.get(&(table_id, col)) {
                                obj.push_str(&format!(" /Headers [{} 0 R]", th_id));
                            }
                        }
                    }
                    obj.push_str(" >>");
                    self.write_object(id, &obj)?;
                    if tag.parent.is_none() {
                        root_kids.push(id);
                    }
                    if let (Some(list), Some(mcid)) =
                        (page_parent_tree.get_mut(tag.page_index), tag.mcid)
                    {
                        let mcid = mcid as usize;
                        if list.len() <= mcid {
                            list.resize(mcid + 1, None);
                        }
                        list[mcid] = Some(id);
                    }
                }
            }

            let mut nums_entries: Vec<String> = Vec::new();
            for (idx, elems) in page_parent_tree.iter().enumerate() {
                let refs = elems
                    .iter()
                    .map(|id| {
                        id.map(|v| format!("{} 0 R", v))
                            .unwrap_or_else(|| "null".to_string())
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                nums_entries.push(format!("{} [{}]", idx, refs));
            }
            let parent_tree_obj = format!("<< /Nums [{}] >>", nums_entries.join(" "));
            self.write_object(parent_tree_id, &parent_tree_obj)?;

            let kids = root_kids
                .iter()
                .map(|id| format!("{} 0 R", id))
                .collect::<Vec<_>>()
                .join(" ");
            if let Some(namespace_id) = namespace_id {
                self.write_object(
                    namespace_id,
                    "<< /Type /Namespace /NS (http://iso.org/pdf2/ssn) >>",
                )?;
            }
            if let (Some(document_node_id), Some(namespace_id)) = (document_node_id, namespace_id) {
                let document_obj = format!(
                    "<< /Type /StructElem /S /Document /P {} 0 R /NS {} 0 R /K [{}] >>",
                    root_id, namespace_id, kids
                );
                self.write_object(document_node_id, &document_obj)?;
            }
            let root_k_entry = document_node_id
                .map(|id| format!("[{} 0 R]", id))
                .unwrap_or_else(|| format!("[{}]", kids));
            let root_obj = format!(
                "<< /Type /StructTreeRoot /K {} /ParentTree {} 0 R >>",
                root_k_entry, parent_tree_id
            );
            self.write_object(root_id, &root_obj)?;
            struct_tree_root_id = Some(root_id);
        }

        let mut pdfvt_dpart_root_id: Option<usize> = None;
        if let (Some(root_id), Some(node_id), Some(first_page_id)) = (
            self.pdfvt_dpart_root_id,
            self.pdfvt_dpart_node_id,
            self.page_ids.first().copied(),
        ) {
            let mut dpart_node = format!(
                "<< /Type /DPart /Parent {} 0 R /Start {} 0 R",
                root_id, first_page_id
            );
            if let Some(last_page_id) = self.page_ids.last().copied() {
                if last_page_id != first_page_id {
                    dpart_node.push_str(&format!(" /End {} 0 R", last_page_id));
                }
            }
            dpart_node.push_str(" >>");
            self.write_object(node_id, &dpart_node)?;
            let dpart_root = format!(
                "<< /Type /DPartRoot /DPartRootNode {} 0 R /NodeNameList [/Document] >>",
                node_id
            );
            self.write_object(root_id, &dpart_root)?;
            pdfvt_dpart_root_id = Some(root_id);
        }

        // 5) Compliance objects + Catalog.
        let mut metadata_id: Option<usize> = None;
        let mut output_intent_id: Option<usize> = None;
        let mut info_id: Option<usize> = None;
        let mut embedded_files_names_id: Option<usize> = None;
        let mut embedded_file_spec_id: Option<usize> = None;
        let pdf_profile = self.options.pdf_profile;
        let doc_lang = self.options.document_lang.clone();
        let doc_title = self.options.document_title.clone();
        let output_intent = self.options.output_intent.clone();
        if pdf_profile != PdfProfile::None {
            if let Some(xmp) = build_xmp_metadata(
                pdf_profile,
                self.options.pdf_version,
                doc_lang.as_deref(),
                doc_title.as_deref(),
            ) {
                let id = self.alloc_ids(1);
                self.write_object(id, &metadata_stream_object(&xmp))?;
                metadata_id = Some(id);
            }
            if let Some(oi) = output_intent.as_ref() {
                let icc_id = self.alloc_ids(1);
                self.write_icc_profile_stream_object(icc_id, &oi.icc_profile, oi.n_components)?;
                let oi_id = self.alloc_ids(1);
                self.write_object(oi_id, &output_intent_object(oi, icc_id, pdf_profile))?;
                output_intent_id = Some(oi_id);
            }
            if pdf_profile == PdfProfile::PdfA4f {
                let embedded_file_id = self.alloc_ids(1);
                self.write_object(embedded_file_id, &pdfa4f_seed_embedded_file_stream_object())?;
                let file_spec_id = self.alloc_ids(1);
                self.write_object(
                    file_spec_id,
                    &pdfa4f_seed_file_spec_object(embedded_file_id),
                )?;
                let names_id = self.alloc_ids(1);
                self.write_object(names_id, &pdfa4f_seed_names_object(file_spec_id))?;
                embedded_files_names_id = Some(names_id);
                embedded_file_spec_id = Some(file_spec_id);
            }
        }

        let mut catalog = format!("<< /Type /Catalog /Pages {} 0 R", PDF_PAGES_ID);
        if let Some(lang) = doc_lang.as_deref() {
            catalog.push_str(&format!(" /Lang ({})", escape_pdf_string(lang)));
        }
        if doc_title.is_some() {
            catalog.push_str(" /ViewerPreferences << /DisplayDocTitle true >>");
        }
        if (doc_title.is_some() && !pdf_profile.is_pdfa4_family())
            || matches!(pdf_profile, PdfProfile::PdfX4 | PdfProfile::PdfVt1)
        {
            let id = self.alloc_ids(1);
            self.write_object(id, &info_object(doc_title.as_deref(), pdf_profile))?;
            info_id = Some(id);
        }
        if let Some(id) = metadata_id {
            catalog.push_str(&format!(" /Metadata {} 0 R", id));
        }
        if let Some(id) = output_intent_id {
            catalog.push_str(&format!(" /OutputIntents [{} 0 R]", id));
        }
        if let Some(id) = pdfvt_dpart_root_id {
            catalog.push_str(&format!(" /DPartRoot {} 0 R", id));
        }
        if let Some(id) = embedded_files_names_id {
            catalog.push_str(&format!(" /Names << /EmbeddedFiles {} 0 R >>", id));
        }
        if let Some(id) = embedded_file_spec_id {
            catalog.push_str(&format!(" /AF [{} 0 R]", id));
        }
        if !optional_content_entries.is_empty() {
            let ocg_ids = optional_content_entries
                .iter()
                .map(|(_, id)| *id)
                .collect::<Vec<_>>();
            catalog.push_str(&format!(" /OCProperties {}", ocproperties_dict(&ocg_ids)));
        }
        if let Some(id) = struct_tree_root_id {
            catalog.push_str(&format!(
                " /StructTreeRoot {} 0 R /MarkInfo << /Marked true >>",
                id
            ));
        }
        catalog.push_str(" >>");
        self.write_object(PDF_CATALOG_ID, &catalog)?;

        // 6) XRef + trailer.
        let total_objects = self.next_id.saturating_sub(1);
        let xref_start = self.offset;
        write_str(
            self.writer,
            &format!("xref\n0 {}\n", total_objects + 1),
            &mut self.offset,
        )?;
        write_bytes(self.writer, b"0000000000 65535 f \n", &mut self.offset)?;
        for id in 1..=total_objects {
            let obj_offset = self.offsets.get(id).copied().unwrap_or(0);
            write_str(
                self.writer,
                &format!("{:010} 00000 n \n", obj_offset),
                &mut self.offset,
            )?;
        }
        let mut trailer = format!(
            "trailer\n<< /Size {} /Root {} 0 R",
            total_objects + 1,
            PDF_CATALOG_ID
        );
        if let Some(id) = info_id {
            trailer.push_str(&format!(" /Info {} 0 R", id));
        }
        if pdf_profile != PdfProfile::None {
            let file_id = deterministic_file_id(
                pdf_profile,
                self.options.pdf_version,
                doc_lang.as_deref(),
                doc_title.as_deref(),
                self.page_ids.len(),
                total_objects,
                xref_start,
            );
            trailer.push_str(&format!(" /ID [<{}> <{}>]", file_id, file_id));
        }
        trailer.push_str(&format!(" >>\nstartxref\n{}\n%%EOF", xref_start));
        write_str(self.writer, &trailer, &mut self.offset)?;

        let bytes_written = self.offset;
        let content_ratio_ppm = if self.content_stream_raw_bytes == 0 {
            1_000_000u64
        } else {
            ((self.content_stream_encoded_bytes as u128)
                .saturating_mul(1_000_000)
                .saturating_div(self.content_stream_raw_bytes as u128)) as u64
        };
        let finish_ms = t_finish.elapsed().as_secs_f64() * 1000.0;
        if let Some(logger) = self.debug.as_deref() {
            let json = format!(
                "{{\"type\":\"jit.link\",\"ms\":{:.3},\"bytes\":{},\"pages\":{},\"fonts\":{},\"images\":{},\"forms\":{},\"shadings\":{},\"extgstates\":{},\"image_bytes\":{},\"content_stream_raw_bytes\":{},\"content_stream_encoded_bytes\":{},\"content_stream_compressed\":{},\"content_stream_ratio_ppm\":{},\"page_content_streams\":{},\"page_content_reused_references\":{},\"font_program_source_bytes\":{},\"font_program_subset_bytes\":{},\"font_program_encoded_bytes\":{},\"font_program_subsets\":{},\"font_program_compressed\":{},\"font_program_subset_glyphs\":{}}}",
                finish_ms,
                bytes_written,
                self.page_ids.len(),
                fonts.len(),
                self.image_resources.len(),
                self.form_resources.len(),
                self.shading_resources.len(),
                self.gs_resources.len(),
                self.image_bytes_total,
                self.content_stream_raw_bytes,
                self.content_stream_encoded_bytes,
                self.content_stream_compressed_count,
                content_ratio_ppm,
                self.page_content_stream_count,
                self.page_content_reused_references,
                self.font_program_source_bytes,
                self.font_program_subset_bytes,
                self.font_program_encoded_bytes,
                self.font_program_subset_count,
                self.font_program_compressed_count,
                self.font_program_subset_glyphs,
            );
            logger.log_json(&json);
            let profile_json = format!(
                "{{\"type\":\"jit.pdf_profile\",\"pdf_version\":\"{}\",\"pdf_profile\":\"{}\",\"metadata\":{},\"output_intent\":{},\"tagged_structure\":{},\"struct_tree_root\":{},\"page_boxes\":{},\"pdfvt_dpart_root\":{},\"embedded_files\":{},\"pdf_declaration\":{},\"requires_output_intent\":{},\"requires_embedded_fonts\":{}}}",
                match self.options.pdf_version {
                    PdfVersion::Pdf17 => "1.7",
                    PdfVersion::Pdf20 => "2.0",
                },
                self.options.pdf_profile.as_str(),
                metadata_id.is_some(),
                output_intent_id.is_some(),
                self.options.pdf_profile.emits_tagged_structure(),
                struct_tree_root_id.is_some(),
                self.options.pdf_profile.uses_pdfx_page_boxes(),
                pdfvt_dpart_root_id.is_some(),
                embedded_files_names_id.is_some(),
                matches!(
                    self.options.pdf_profile,
                    PdfProfile::Wtpdf1r | PdfProfile::Wtpdf1a
                ),
                self.options.pdf_profile.requires_output_intent(),
                self.options.pdf_profile.requires_embedded_fonts(),
            );
            logger.log_json(&profile_json);
        }
        if let Some(perf_logger) = self.perf.as_deref() {
            perf_logger.log_span_ms("pdf.link", None, finish_ms);
            perf_logger.log_counts(
                "pdf.link",
                None,
                &[
                    ("bytes", bytes_written as u64),
                    ("pages", self.page_ids.len() as u64),
                    ("fonts", fonts.len() as u64),
                    ("images", self.image_resources.len() as u64),
                    ("forms", self.form_resources.len() as u64),
                    ("shadings", self.shading_resources.len() as u64),
                    ("extgstates", self.gs_resources.len() as u64),
                    ("image_bytes", self.image_bytes_total as u64),
                    (
                        "content_stream_raw_bytes",
                        self.content_stream_raw_bytes as u64,
                    ),
                    (
                        "content_stream_encoded_bytes",
                        self.content_stream_encoded_bytes as u64,
                    ),
                    (
                        "content_stream_compressed",
                        self.content_stream_compressed_count as u64,
                    ),
                    ("content_stream_ratio_ppm", content_ratio_ppm),
                    (
                        "page_content_streams",
                        self.page_content_stream_count as u64,
                    ),
                    (
                        "page_content_reused_references",
                        self.page_content_reused_references as u64,
                    ),
                    (
                        "font_program_source_bytes",
                        self.font_program_source_bytes as u64,
                    ),
                    (
                        "font_program_subset_bytes",
                        self.font_program_subset_bytes as u64,
                    ),
                    (
                        "font_program_encoded_bytes",
                        self.font_program_encoded_bytes as u64,
                    ),
                    (
                        "font_program_subsets",
                        self.font_program_subset_count as u64,
                    ),
                    (
                        "font_program_compressed",
                        self.font_program_compressed_count as u64,
                    ),
                    (
                        "font_program_subset_glyphs",
                        self.font_program_subset_glyphs as u64,
                    ),
                    (
                        "profile_metadata",
                        if metadata_id.is_some() { 1 } else { 0 },
                    ),
                    (
                        "profile_output_intent",
                        if output_intent_id.is_some() { 1 } else { 0 },
                    ),
                    (
                        "profile_tagged_structure",
                        if struct_tree_root_id.is_some() { 1 } else { 0 },
                    ),
                    (
                        "profile_pdfvt_dpart_root",
                        if pdfvt_dpart_root_id.is_some() { 1 } else { 0 },
                    ),
                    (
                        "profile_embedded_files",
                        if embedded_files_names_id.is_some() {
                            1
                        } else {
                            0
                        },
                    ),
                ],
            );
        }
        Ok(bytes_written)
    }

    fn render_page(&mut self, page: &Page, page_index: usize) -> io::Result<String> {
        let geometry = PageGeometry::for_page(page, self.page_size);
        self.render_page_sized(page, page_index, geometry)
    }

    fn render_page_sized(
        &mut self,
        page: &Page,
        page_index: usize,
        geometry: PageGeometry,
    ) -> io::Result<String> {
        let content = self.render_commands(
            &page.commands,
            geometry.logical_size.height,
            Some(page_index),
        )?;
        let content = wrap_page_content_for_presentation(content, geometry);
        Ok(wrap_page_content_for_print_device_phase(
            content,
            geometry.media_size.height,
        ))
    }

    fn render_commands(
        &mut self,
        commands: &[Command],
        page_height: Pt,
        page_index: Option<usize>,
    ) -> io::Result<String> {
        self.render_commands_with_filter_offset(commands, page_height, page_index, None)
    }

    fn render_commands_with_filter_offset(
        &mut self,
        commands: &[Command],
        page_height: Pt,
        page_index: Option<usize>,
        filter_raster_offset: Option<(Pt, Pt)>,
    ) -> io::Result<String> {
        let mut out = String::new();
        let mut current_font_size = Pt::from_f32(12.0);
        let mut current_font_name = "Helvetica".to_string();
        let mut current_fill = Color::BLACK;
        let mut graphics_state_stack: Vec<(Pt, String, Color)> = Vec::new();
        let mut tag_stack: Vec<usize> = Vec::new();
        let tag_enabled = self.options.pdf_profile.emits_tagged_structure() && page_index.is_some();

        for cmd in commands {
            match cmd {
                Command::SaveState => {
                    graphics_state_stack.push((
                        current_font_size,
                        current_font_name.clone(),
                        current_fill,
                    ));
                    out.push_str("q\n");
                }
                Command::RestoreState => {
                    if let Some((font_size, font_name, fill)) = graphics_state_stack.pop() {
                        current_font_size = font_size;
                        current_font_name = font_name;
                        current_fill = fill;
                    }
                    out.push_str("Q\n");
                }
                Command::Translate(x, y) => {
                    // Canvas transform translations use CSS's top-down axis;
                    // PDF's user space is bottom-up.
                    out.push_str(&format!("1 0 0 1 {} {} cm\n", fmt_pt(*x), fmt_pt(-*y)));
                }
                Command::CssTransformOrigin { x, y, inverse } => {
                    let pdf_y = page_height - *y;
                    let (tx, ty) = if *inverse { (-*x, -pdf_y) } else { (*x, pdf_y) };
                    out.push_str(&format!("1 0 0 1 {} {} cm\n", fmt_pt(tx), fmt_pt(ty)));
                }
                Command::Scale(x, y) => {
                    out.push_str(&format!("{} 0 0 {} 0 0 cm\n", fmt(*x), fmt(*y)));
                }
                Command::Rotate(angle) => {
                    // Canvas uses CSS/SVG's top-down axis. Conjugate the
                    // rotation into PDF's bottom-up user space.
                    let (sin, cos) = crate::math::sin_cos(-*angle);
                    out.push_str(&format!(
                        "{} {} {} {} 0 0 cm\n",
                        fmt(cos),
                        fmt(sin),
                        fmt(-sin),
                        fmt(cos)
                    ));
                }
                Command::ConcatMatrix { a, b, c, d, e, f } => {
                    // F * M * F converts a top-down affine matrix into PDF's
                    // bottom-up user space (F flips the y axis).
                    out.push_str(&format!(
                        "{} {} {} {} {} {} cm\n",
                        fmt(*a),
                        fmt(-*b),
                        fmt(-*c),
                        fmt(*d),
                        fmt_pt(*e),
                        fmt_pt(-*f)
                    ));
                }
                Command::Meta { .. } => {}
                Command::BeginTag {
                    role,
                    mcid,
                    alt,
                    scope,
                    table_id,
                    col_index,
                    group_only,
                } => {
                    if tag_enabled {
                        let role_raw = role.clone();
                        let role = escape_pdf_name(role);
                        if *group_only {
                            out.push_str(&format!("/{role} BMC\n"));
                        } else if let Some(mcid) = mcid {
                            out.push_str(&format!("/{role} <</MCID {}>> BDC\n", mcid));
                        }
                        let parent = tag_stack.last().copied();
                        let idx = self.tag_records.len();
                        self.tag_records.push(TagRecord {
                            page_index: page_index.unwrap_or(0),
                            mcid: *mcid,
                            role: role_raw,
                            alt: alt.clone(),
                            scope: scope.clone(),
                            parent,
                            table_id: *table_id,
                            col_index: *col_index,
                        });
                        tag_stack.push(idx);
                    }
                }
                Command::EndTag => {
                    if tag_enabled {
                        out.push_str("EMC\n");
                        let _ = tag_stack.pop();
                    }
                }
                Command::BeginArtifact { subtype } => {
                    if let Some(subtype) = subtype.as_deref() {
                        out.push_str(&format!(
                            "/Artifact <</Subtype /{}>> BDC\n",
                            escape_pdf_name(subtype)
                        ));
                    } else {
                        out.push_str("/Artifact BMC\n");
                    }
                }
                Command::BeginOptionalContent { name } => {
                    self.optional_content_names.insert(name.clone());
                    out.push_str(&format!("/OC /{} BDC\n", escape_pdf_name(name)));
                }
                Command::EndMarkedContent => {
                    out.push_str("EMC\n");
                }
                Command::SetFillColor(color) => {
                    current_fill = *color;
                    out.push_str(&color_to_pdf_fill(*color, self.options.color_space));
                }
                Command::SetStrokeColor(color) => {
                    out.push_str(&color_to_pdf_stroke(*color, self.options.color_space));
                }
                Command::SetLineWidth(width) => {
                    out.push_str(&format!("{} w\n", fmt_pt(*width)));
                }
                Command::SetLineCap(cap) => {
                    out.push_str(&format!("{} J\n", cap));
                }
                Command::SetLineJoin(join) => {
                    out.push_str(&format!("{} j\n", join));
                }
                Command::SetMiterLimit(limit) => {
                    out.push_str(&format!("{} M\n", fmt_pt(*limit)));
                }
                Command::SetDash { pattern, phase } => {
                    let pat = if pattern.is_empty() {
                        "[]".to_string()
                    } else {
                        let items = pattern
                            .iter()
                            .map(|v| fmt_pt(*v))
                            .collect::<Vec<_>>()
                            .join(" ");
                        format!("[{}]", items)
                    };
                    out.push_str(&format!("{} {} d\n", pat, fmt_pt(*phase)));
                }
                Command::SetOpacity { fill, stroke } => {
                    // Map opacity to an ExtGState resource. We quantize to 0..1000.
                    let k = ((*fill * 1000.0).round() as i32).clamp(0, 1000) as u16;
                    let ks = ((*stroke * 1000.0).round() as i32).clamp(0, 1000) as u16;
                    if let Some(name) = self.ensure_extgstate((k, ks))? {
                        out.push_str(&format!("/{} gs\n", name));
                    }
                }
                Command::SetBlendMode { mode } => {
                    if let Some(name) = self.ensure_blend_extgstate(*mode)? {
                        out.push_str(&format!("/{} gs\n", name));
                    }
                }
                Command::ApplyBackdropFilter { .. } => {}
                Command::SetFontName(name) => {
                    current_font_name = name.clone();
                    self.ensure_font(&current_font_name)?;
                }
                Command::SetFontSize(size) => {
                    current_font_size = *size;
                }
                Command::SetTextRenderingMode(mode) => {
                    out.push_str(&format!("{} Tr\n", (*mode).min(7)));
                }
                Command::ClipRect {
                    x,
                    y,
                    width,
                    height,
                } => {
                    let draw_y = page_height - *y - *height;
                    out.push_str(&format!(
                        "{} {} {} {} re\nW\nn\n",
                        fmt_pt(*x),
                        fmt_pt(draw_y),
                        fmt_pt(*width),
                        fmt_pt(*height)
                    ));
                }
                Command::ClipPath { evenodd } => {
                    if *evenodd {
                        out.push_str("W*\n");
                    } else {
                        out.push_str("W\n");
                    }
                    out.push_str("n\n");
                }
                Command::ShadingFill(shading) => {
                    if matches!(shading, Shading::Conic { .. }) {
                        self.append_conic_shading(&mut out, shading, page_height)?;
                    } else {
                        let key = hash_shading_at_height(shading, page_height);
                        if let Some((name, alpha_gs)) =
                            self.ensure_shading(key, shading, page_height)?
                        {
                            if let Some(alpha_gs) = alpha_gs {
                                out.push_str("q\n");
                                out.push_str(&format!("/{} gs\n", alpha_gs));
                                out.push_str(&format!("/{} sh\n", name));
                                out.push_str("Q\n");
                            } else {
                                out.push_str(&format!("/{} sh\n", name));
                            }
                        }
                    }
                }
                Command::MoveTo { x, y } => {
                    out.push_str(&format!("{} {} m\n", fmt_pt(*x), fmt_pt(page_height - *y)));
                }
                Command::LineTo { x, y } => {
                    out.push_str(&format!("{} {} l\n", fmt_pt(*x), fmt_pt(page_height - *y)));
                }
                Command::CurveTo {
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                } => {
                    out.push_str(&format!(
                        "{} {} {} {} {} {} c\n",
                        fmt_pt(*x1),
                        fmt_pt(page_height - *y1),
                        fmt_pt(*x2),
                        fmt_pt(page_height - *y2),
                        fmt_pt(*x),
                        fmt_pt(page_height - *y),
                    ));
                }
                Command::ClosePath => out.push_str("h\n"),
                Command::Fill => out.push_str("f\n"),
                Command::FillEvenOdd => out.push_str("f*\n"),
                Command::Stroke => out.push_str("S\n"),
                Command::FillStroke => out.push_str("B\n"),
                Command::FillStrokeEvenOdd => out.push_str("B*\n"),
                Command::DrawString { x, y, text } => {
                    let font_key = self.font_key(&current_font_name);
                    if !self.fonts.contains_key(&font_key) {
                        self.ensure_font(&current_font_name)?;
                    }
                    let Some((resource, encoding)) = self
                        .fonts
                        .get(&font_key)
                        .map(|f| (f.resource.clone(), f.encoding))
                    else {
                        continue;
                    };
                    out.push_str("BT\n");
                    out.push_str(&format!("/{} {} Tf\n", resource, fmt_pt(current_font_size)));
                    out.push_str(&format!(
                        "{} {} Td\n",
                        fmt_pt(*x),
                        fmt_pt(page_height - *y - current_font_size)
                    ));

                    match encoding {
                        FontEncoding::WinAnsi => {
                            let encoded = encode_winansi_pdf_string(text);
                            if encoded.replaced > 0 {
                                if let Some(logger) = self.debug.as_deref() {
                                    let json = format!(
                                        "{{\"type\":\"pdf.winansi.lossy\",\"font\":{},\"replaced\":{},\"sample\":{}}}",
                                        json_escape(&current_font_name),
                                        encoded.replaced,
                                        json_escape(&truncate_preview(text, 80))
                                    );
                                    logger.log_json(&json);
                                    logger.increment("pdf.winansi.lossy", encoded.replaced as u64);
                                }
                            }
                            if encoded.fallbacks > 0 {
                                if let Some(logger) = self.debug.as_deref() {
                                    let json = format!(
                                        "{{\"type\":\"pdf.winansi.fallback\",\"font\":{},\"fallbacks\":{},\"sample\":{}}}",
                                        json_escape(&current_font_name),
                                        encoded.fallbacks,
                                        json_escape(&truncate_preview(text, 80))
                                    );
                                    logger.log_json(&json);
                                    logger.increment(
                                        "pdf.winansi.fallback",
                                        encoded.fallbacks as u64,
                                    );
                                    let known_loss = format!(
                                        "{{\"type\":\"jit.known_loss\",\"code\":\"FONT_FALLBACK_USED\",\"font\":{},\"fallbacks\":{},\"sample\":{}}}",
                                        json_escape(&current_font_name),
                                        encoded.fallbacks,
                                        json_escape(&truncate_preview(text, 80))
                                    );
                                    logger.log_json(&known_loss);
                                    logger.increment(
                                        "jit.known_loss.font_fallback_used",
                                        encoded.fallbacks as u64,
                                    );
                                }
                            }
                            out.push_str(&format!("({}) Tj\n", encoded.text));
                        }
                        FontEncoding::IdentityH => {
                            if let Some(tj) = self.shape_text_to_tj(
                                &font_key,
                                &current_font_name,
                                current_font_size,
                                text,
                            ) {
                                out.push_str(tj);
                            } else {
                                let hex = self.encode_cid_hex_fallback(
                                    &font_key,
                                    &current_font_name,
                                    text,
                                );
                                out.push_str(&format!("{} Tj\n", hex));
                            }
                        }
                    }
                    out.push_str("ET\n");
                }
                Command::DrawStringTransformed {
                    x,
                    y,
                    text,
                    m00,
                    m01,
                    m10,
                    m11,
                } => {
                    let font_key = self.font_key(&current_font_name);
                    if !self.fonts.contains_key(&font_key) {
                        self.ensure_font(&current_font_name)?;
                    }
                    let Some((resource, encoding)) = self
                        .fonts
                        .get(&font_key)
                        .map(|font| (font.resource.clone(), font.encoding))
                    else {
                        continue;
                    };
                    out.push_str("BT\n");
                    out.push_str(&format!("/{} {} Tf\n", resource, fmt_pt(current_font_size)));
                    out.push_str(&format!(
                        "{} {} {} {} {} {} Tm\n",
                        fmt(*m00),
                        fmt(*m01),
                        fmt(*m10),
                        fmt(*m11),
                        fmt_pt(*x),
                        fmt_pt(*y)
                    ));
                    match encoding {
                        FontEncoding::WinAnsi => {
                            let encoded = encode_winansi_pdf_string(text);
                            if encoded.replaced > 0 {
                                if let Some(logger) = self.debug.as_deref() {
                                    let json = format!(
                                        "{{\"type\":\"pdf.winansi.lossy\",\"font\":{},\"replaced\":{},\"sample\":{}}}",
                                        json_escape(&current_font_name),
                                        encoded.replaced,
                                        json_escape(&truncate_preview(text, 80))
                                    );
                                    logger.log_json(&json);
                                    logger.increment("pdf.winansi.lossy", encoded.replaced as u64);
                                }
                            }
                            if encoded.fallbacks > 0 {
                                if let Some(logger) = self.debug.as_deref() {
                                    let json = format!(
                                        "{{\"type\":\"pdf.winansi.fallback\",\"font\":{},\"fallbacks\":{},\"sample\":{}}}",
                                        json_escape(&current_font_name),
                                        encoded.fallbacks,
                                        json_escape(&truncate_preview(text, 80))
                                    );
                                    logger.log_json(&json);
                                    logger.increment(
                                        "pdf.winansi.fallback",
                                        encoded.fallbacks as u64,
                                    );
                                }
                            }
                            out.push_str(&format!("({}) Tj\n", encoded.text));
                        }
                        FontEncoding::IdentityH => {
                            if let Some(tj) = self.shape_text_to_tj(
                                &font_key,
                                &current_font_name,
                                current_font_size,
                                text,
                            ) {
                                out.push_str(tj);
                            } else {
                                let hex = self.encode_cid_hex_fallback(
                                    &font_key,
                                    &current_font_name,
                                    text,
                                );
                                out.push_str(&format!("{} Tj\n", hex));
                            }
                        }
                    }
                    out.push_str("ET\n");
                }
                Command::DrawGlyphRun {
                    x,
                    y,
                    glyph_ids,
                    advances,
                    m00,
                    m01,
                    m10,
                    m11,
                } => {
                    let mut pen_x = *x;
                    let mut pen_y = page_height - *y;
                    for (index, glyph_id) in glyph_ids.iter().copied().enumerate() {
                        if let Some(resource) =
                            self.ensure_type3_glyph(&current_font_name, glyph_id, 0)?
                        {
                            out.push_str("BT\n");
                            out.push_str(&format!(
                                "/{} {} Tf\n",
                                resource,
                                fmt_pt(current_font_size)
                            ));
                            out.push_str(&format!(
                                "{} {} {} {} {} {} Tm\n",
                                fmt(*m00),
                                fmt(*m01),
                                fmt(*m10),
                                fmt(*m11),
                                fmt_pt(pen_x),
                                fmt_pt(pen_y),
                            ));
                            out.push_str(&format!("<{:02X}> Tj\nET\n", glyph_id & 0x00ff));
                        }
                        if let Some((advance_x, advance_y)) = advances.get(index) {
                            pen_x = pen_x + *advance_x;
                            pen_y = pen_y + *advance_y;
                        }
                    }
                }
                Command::DrawSyntheticBoldGlyphRun {
                    x,
                    y,
                    glyph_ids,
                    advances,
                    offsets,
                    stroke_width,
                } => {
                    let synthetic_bold_millionths =
                        synthetic_bold_ratio_millionths(*stroke_width, current_font_size);
                    let mut pen_x = *x;
                    let mut pen_y = *y;
                    for (index, glyph_id) in glyph_ids.iter().copied().enumerate() {
                        let offset = offsets.get(index).copied().unwrap_or((Pt::ZERO, Pt::ZERO));
                        if let Some(resource) = self.ensure_type3_glyph(
                            &current_font_name,
                            glyph_id,
                            synthetic_bold_millionths,
                        )? {
                            out.push_str("BT\n");
                            out.push_str(&format!(
                                "/{} {} Tf\n",
                                resource,
                                fmt_pt(current_font_size)
                            ));
                            out.push_str(&format!(
                                "1 0 0 1 {} {} Tm\n",
                                fmt_pt(pen_x + offset.0),
                                fmt_pt(page_height - pen_y - offset.1),
                            ));
                            out.push_str(&format!("<{:02X}> Tj\nET\n", glyph_id & 0x00ff));
                        }
                        if let Some((advance_x, advance_y)) = advances.get(index) {
                            pen_x = pen_x + *advance_x;
                            pen_y = pen_y + *advance_y;
                        }
                    }
                }
                Command::DrawRect {
                    x,
                    y,
                    width,
                    height,
                } => {
                    let draw_y = page_height - *y - *height;
                    out.push_str(&format!(
                        "{} {} {} {} re\nf\n",
                        fmt_pt(*x),
                        fmt_pt(draw_y),
                        fmt_pt(*width),
                        fmt_pt(*height)
                    ));
                }
                Command::DrawImage {
                    x,
                    y,
                    width,
                    height,
                    resource_id,
                    source_clip,
                    ..
                } => {
                    let image = if let Some(source_clip) = source_clip {
                        self.ensure_image_variant(resource_id, *source_clip, *width, *height)?
                    } else {
                        self.ensure_image(resource_id)?.map(|name| (name, None))
                    };
                    if let Some((name, source_crop)) = image {
                        let (draw_x, draw_top, draw_width, draw_height) = source_crop
                            .map(|crop| {
                                source_clip
                                    .as_ref()
                                    .copied()
                                    .expect("resolved crop has a source clip")
                                    .snap_target_rect(crop.target_rect(*x, *y, *width, *height))
                            })
                            .unwrap_or((*x, *y, *width, *height));
                        let draw_y = page_height - draw_top - draw_height;
                        out.push_str("q\n");
                        out.push_str(&format!(
                            "{} 0 0 {} {} {} cm\n",
                            fmt_pt(draw_width),
                            fmt_pt(draw_height),
                            fmt_pt(draw_x),
                            fmt_pt(draw_y)
                        ));
                        out.push_str(&format!("/{} Do\n", name));
                        out.push_str("Q\n");
                    } else {
                        // Image missing: draw a solid block to avoid silent layout shifts.
                        out.push_str(&color_to_pdf_fill(current_fill, self.options.color_space));
                    }
                }
                Command::DefineForm {
                    resource_id,
                    width,
                    height,
                    commands,
                } => {
                    self.register_form_definition(resource_id, *width, *height, commands, false);
                }
                Command::DefineIsolatedForm {
                    resource_id,
                    width,
                    height,
                    commands,
                } => {
                    self.register_form_definition(resource_id, *width, *height, commands, true);
                }
                Command::DrawForm {
                    x,
                    y,
                    width,
                    height,
                    resource_id,
                } => {
                    if let Some(name) = self.ensure_registered_form(resource_id)? {
                        let draw_y = page_height - *y - *height;
                        let (sx, sy) = self
                            .form_size_map
                            .get(resource_id)
                            .map(|size| {
                                let sx = if size.width.to_f32() > 0.0 {
                                    width.to_f32() / size.width.to_f32()
                                } else {
                                    1.0
                                };
                                let sy = if size.height.to_f32() > 0.0 {
                                    height.to_f32() / size.height.to_f32()
                                } else {
                                    1.0
                                };
                                (sx, sy)
                            })
                            .unwrap_or((1.0, 1.0));

                        out.push_str("q\n");
                        out.push_str(&format!(
                            "{} 0 0 {} {} {} cm\n",
                            fmt(sx),
                            fmt(sy),
                            fmt_pt(*x),
                            fmt_pt(draw_y)
                        ));
                        out.push_str(&format!("/{} Do\n", name));
                        out.push_str("Q\n");
                    }
                }
                Command::DrawFilteredForm {
                    x,
                    y,
                    width,
                    height,
                    resource_id,
                    filter,
                    css_shadow,
                } => {
                    let Some((form_width, form_height, form_commands)) =
                        self.form_definition_map.get(resource_id).cloned()
                    else {
                        continue;
                    };
                    let (raster_x, raster_y) = filter_raster_offset
                        .map(|(offset_x, offset_y)| (*x + offset_x, *y + offset_y))
                        .unwrap_or((*x, *y));
                    let raster = crate::raster::rasterize_filtered_form(
                        self.page_size.width,
                        self.page_size.height,
                        form_width,
                        form_height,
                        &form_commands,
                        raster_x,
                        raster_y,
                        *width,
                        *height,
                        filter,
                        *css_shadow,
                        PDF_FILTER_RASTER_DPI,
                        self.registry,
                        self.options.shape_text,
                    )
                    .map_err(|error| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("filtered form rasterization failed: {error}"),
                        )
                    })?;
                    let Some(raster) = raster else {
                        continue;
                    };
                    let Some(image) = image_data_from_premultiplied_rgba(
                        raster.pixel_width,
                        raster.pixel_height,
                        &raster.premultiplied_rgba,
                    ) else {
                        continue;
                    };
                    let image_key = format!(
                        "__fullbleed_filter_{:016x}_{}x{}",
                        hash_bytes(&raster.premultiplied_rgba),
                        raster.pixel_width,
                        raster.pixel_height,
                    );
                    if let Some(name) = self.ensure_image_data(&image_key, image)? {
                        let points_per_pixel = 72.0 / PDF_FILTER_RASTER_DPI as f32;
                        let (emit_x, emit_y) = filter_raster_offset
                            .map(|(offset_x, offset_y)| (raster.x - offset_x, raster.y - offset_y))
                            .unwrap_or((raster.x, raster.y));
                        let device_x = filter_device_coordinate(emit_x);
                        let device_top = filter_device_coordinate(emit_y);
                        let device_bottom = device_top + f64::from(raster.pixel_height);
                        out.push_str("q\n");
                        if filter_raster_is_point_grid_aligned(
                            device_x,
                            device_top,
                            raster.pixel_width,
                            raster.pixel_height,
                        ) {
                            // The print-device tile maps to exact six-point
                            // increments, so collapse the two image matrices.
                            // This avoids a redundant CTM concatenation under
                            // the page's print-phase wrapper without rounding
                            // any device coordinate or resampling the image.
                            let point_scale = 72.0 / f64::from(PDF_FILTER_RASTER_DPI);
                            let image_width = f64::from(raster.pixel_width) * point_scale;
                            let image_height = f64::from(raster.pixel_height) * point_scale;
                            let image_x = device_x * point_scale;
                            let image_bottom =
                                f64::from(page_height.to_f32()) - device_bottom * point_scale;
                            out.push_str(&format!(
                                "{} 0 0 {} {} {} cm\n",
                                fmt_pdf_f64(image_width, 9),
                                fmt_pdf_f64(image_height, 9),
                                fmt_pdf_f64(image_x, 9),
                                fmt_pdf_f64(image_bottom, 9),
                            ));
                        } else {
                            out.push_str(&format!(
                                "{} 0 0 -{} 0 {} cm\n",
                                fmt(points_per_pixel),
                                fmt(points_per_pixel),
                                fmt_pt(page_height),
                            ));
                            out.push_str(&format!(
                                "{} 0 0 -{} {} {} cm\n",
                                raster.pixel_width,
                                raster.pixel_height,
                                fmt_pdf_f64(device_x, 9),
                                fmt_pdf_f64(device_bottom, 9),
                            ));
                        }
                        out.push_str(&format!("/{} Do\n", name));
                        out.push_str("Q\n");
                    }
                }
                Command::DrawMaskedForm {
                    x,
                    y,
                    width,
                    height,
                    resource_id,
                    layers,
                } => {
                    let Some((form_width, form_height, form_commands)) =
                        self.form_definition_map.get(resource_id).cloned()
                    else {
                        continue;
                    };
                    let mut raster_layers = Vec::with_capacity(layers.len());
                    let mut complete = true;
                    for layer in layers {
                        let Some((layer_width, layer_height, layer_commands)) =
                            self.form_definition_map.get(&layer.resource_id).cloned()
                        else {
                            complete = false;
                            break;
                        };
                        raster_layers.push(crate::raster::MaskedFormRasterLayer {
                            program: layer.clone(),
                            width: layer_width,
                            height: layer_height,
                            commands: layer_commands,
                        });
                    }
                    if !complete {
                        continue;
                    }

                    // A filtered form already carries an intrinsic image soft
                    // mask for its blurred alpha. Applying a second PDF soft
                    // mask around that image is not portable across consumers
                    // (Poppler, in particular, drops the outer mask). Execute
                    // the immutable filter and compiled mask together once at
                    // the authenticated print lattice, then cache the combined
                    // tile. Variable content that does not participate in the
                    // filter remains vector and this specialization is reused
                    // across repeated page programs.
                    if commands_contain_filtered_form(&form_commands) {
                        let mut flattened_key = format!(
                            "filtered-mask:{}:{}:{}:{}:{}:{}:{}:{}:{}",
                            resource_id,
                            self.page_size.width.to_milli_i64(),
                            self.page_size.height.to_milli_i64(),
                            x.to_milli_i64(),
                            y.to_milli_i64(),
                            width.to_milli_i64(),
                            height.to_milli_i64(),
                            PDF_FILTER_RASTER_DPI,
                            layers.len(),
                        );
                        for layer in &raster_layers {
                            flattened_key.push_str(&format!(
                                ":{}:{:?}:{:?}",
                                layer.program.resource_id,
                                layer.program.mode,
                                layer.program.composite,
                            ));
                        }
                        let raster = if let Some(cached) =
                            self.masked_form_raster_cache.get(&flattened_key).cloned()
                        {
                            cached
                        } else {
                            let rendered = crate::raster::rasterize_masked_form(
                                self.page_size.width,
                                self.page_size.height,
                                form_width,
                                form_height,
                                &form_commands,
                                &raster_layers,
                                &self.form_definition_map,
                                &self.form_isolated_map,
                                *x,
                                *y,
                                *width,
                                *height,
                                PDF_FILTER_RASTER_DPI,
                                self.registry,
                                self.options.shape_text,
                            )
                            .map_err(|error| {
                                io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    format!("filtered masked form rasterization failed: {error}"),
                                )
                            })?;
                            self.masked_form_raster_cache
                                .insert(flattened_key, rendered.clone());
                            rendered
                        };
                        let Some(raster) = raster else {
                            continue;
                        };
                        let Some(image) = image_data_from_premultiplied_rgba(
                            raster.pixel_width,
                            raster.pixel_height,
                            &raster.premultiplied_rgba,
                        ) else {
                            continue;
                        };
                        let image_key = format!(
                            "__fullbleed_filtered_mask_{:016x}_{}x{}",
                            hash_bytes(&raster.premultiplied_rgba),
                            raster.pixel_width,
                            raster.pixel_height,
                        );
                        if let Some(name) = self.ensure_image_data(&image_key, image)? {
                            let points_per_pixel = 72.0 / PDF_FILTER_RASTER_DPI as f32;
                            let device_x = filter_device_coordinate(raster.x);
                            let device_top = filter_device_coordinate(raster.y);
                            let device_bottom = device_top + f64::from(raster.pixel_height);
                            out.push_str("q\n");
                            if filter_raster_is_point_grid_aligned(
                                device_x,
                                device_top,
                                raster.pixel_width,
                                raster.pixel_height,
                            ) {
                                let point_scale = 72.0 / f64::from(PDF_FILTER_RASTER_DPI);
                                let image_width = f64::from(raster.pixel_width) * point_scale;
                                let image_height = f64::from(raster.pixel_height) * point_scale;
                                let image_x = device_x * point_scale;
                                let image_bottom =
                                    f64::from(page_height.to_f32()) - device_bottom * point_scale;
                                out.push_str(&format!(
                                    "{} 0 0 {} {} {} cm\n",
                                    fmt_pdf_f64(image_width, 9),
                                    fmt_pdf_f64(image_height, 9),
                                    fmt_pdf_f64(image_x, 9),
                                    fmt_pdf_f64(image_bottom, 9),
                                ));
                            } else {
                                out.push_str(&format!(
                                    "{} 0 0 -{} 0 {} cm\n",
                                    fmt(points_per_pixel),
                                    fmt(points_per_pixel),
                                    fmt_pt(page_height),
                                ));
                                out.push_str(&format!(
                                    "{} 0 0 -{} {} {} cm\n",
                                    raster.pixel_width,
                                    raster.pixel_height,
                                    fmt_pdf_f64(device_x, 9),
                                    fmt_pdf_f64(device_bottom, 9),
                                ));
                            }
                            out.push_str(&format!("/{} Do\n", name));
                            out.push_str("Q\n");
                        }
                        continue;
                    }

                    // The coverage program is independent of the source form.
                    // Cache it by compiled mask IR and target geometry so VDP
                    // bindings can change text/content without rerasterizing an
                    // immutable mask for every record.
                    let mut cache_key = format!(
                        "{}:{}:{}:{}:{}:{}",
                        form_width.to_milli_i64(),
                        form_height.to_milli_i64(),
                        width.to_milli_i64(),
                        height.to_milli_i64(),
                        PDF_FILTER_RASTER_DPI,
                        layers.len(),
                    );
                    for layer in &raster_layers {
                        cache_key.push_str(&format!(
                            ":{:?}:{:?}:{}:{}:{:016x}",
                            layer.program.mode,
                            layer.program.composite,
                            layer.width.to_milli_i64(),
                            layer.height.to_milli_i64(),
                            hash_bytes(format!("{:?}", layer.commands).as_bytes()),
                        ));
                    }
                    let hard_sample_rows = raster_layers.len() == 1
                        && commands_have_alpha_discontinuity(&raster_layers[0].commands);
                    cache_key.push_str(if hard_sample_rows {
                        ":hard-rows"
                    } else {
                        ":smooth-rows"
                    });
                    let vector_mask = (raster_layers.len() == 1
                        && matches!(
                            raster_layers[0].program.mode,
                            crate::flowable::MaskMode::MatchSource
                                | crate::flowable::MaskMode::Alpha
                        ))
                    .then(|| vector_alpha_mask_commands(&raster_layers[0].commands))
                    .flatten();
                    let mask_gs = if let Some(vector_commands) = vector_mask {
                        let vector_key = format!("vector:{cache_key}");
                        self.ensure_vector_mask_extgstate(
                            &vector_key,
                            &vector_commands,
                            raster_layers[0].width,
                            raster_layers[0].height,
                            form_width,
                            form_height,
                        )?
                    } else {
                        let raster = if let Some(cached) =
                            self.mask_coverage_raster_cache.get(&cache_key).cloned()
                        {
                            cached
                        } else {
                            let rendered = crate::raster::rasterize_mask_coverage(
                                &raster_layers,
                                *width,
                                *height,
                                PDF_FILTER_RASTER_DPI,
                                self.registry,
                                self.options.shape_text,
                            )
                            .map_err(|error| {
                                io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    format!("masked form rasterization failed: {error}"),
                                )
                            })?;
                            self.mask_coverage_raster_cache
                                .insert(cache_key.clone(), rendered.clone());
                            rendered
                        };
                        if let Some(raster) = raster {
                            self.ensure_mask_coverage_extgstate(
                                &cache_key,
                                &raster,
                                form_width,
                                form_height,
                                hard_sample_rows,
                            )?
                        } else {
                            None
                        }
                    };
                    let Some(mask_gs) = mask_gs else {
                        continue;
                    };
                    let sx = if form_width > Pt::ZERO {
                        width.to_f32() / form_width.to_f32()
                    } else {
                        1.0
                    };
                    let sy = if form_height > Pt::ZERO {
                        height.to_f32() / form_height.to_f32()
                    } else {
                        1.0
                    };
                    let phase_specialized = commands_contain_filtered_form(&form_commands)
                        && (sx - 1.0).abs() <= 1.0e-6
                        && (sy - 1.0).abs() <= 1.0e-6;
                    let form_name = if phase_specialized {
                        self.ensure_registered_form_with_filter_offset(resource_id, (*x, *y))?
                    } else {
                        self.ensure_registered_form(resource_id)?
                    };
                    let Some(form_name) = form_name else {
                        continue;
                    };
                    let draw_y = page_height - *y - *height;
                    out.push_str("q\n");
                    out.push_str(&format!(
                        "{} 0 0 {} {} {} cm\n",
                        fmt(sx),
                        fmt(sy),
                        fmt_pt(*x),
                        fmt_pt(draw_y),
                    ));
                    out.push_str(&format!("/{} gs\n/{} Do\n", mask_gs, form_name));
                    out.push_str("Q\n");
                }
            }
        }
        Ok(out)
    }

    fn ensure_offsets_len(&mut self, required_len: usize) {
        if self.offsets.len() < required_len {
            self.offsets.resize(required_len, 0);
        }
    }

    fn alloc_ids(&mut self, count: usize) -> usize {
        let start = self.next_id;
        self.next_id = self.next_id.saturating_add(count);
        self.ensure_offsets_len(self.next_id);
        start
    }

    fn write_object(&mut self, obj_id: usize, body: &str) -> io::Result<()> {
        write_pdf_object(
            self.writer,
            &mut self.offset,
            &mut self.offsets,
            obj_id,
            body,
        )
    }

    fn write_stream_object_bytes(
        &mut self,
        obj_id: usize,
        dict_entries: &str,
        data: &[u8],
    ) -> io::Result<()> {
        write_pdf_stream_object(
            self.writer,
            &mut self.offset,
            &mut self.offsets,
            obj_id,
            dict_entries,
            data,
        )
    }

    fn write_content_stream_object(
        &mut self,
        obj_id: usize,
        dict_entries: &str,
        content: &[u8],
    ) -> io::Result<()> {
        self.content_stream_raw_bytes = self.content_stream_raw_bytes.saturating_add(content.len());
        let should_compress = self.options.compress_content_streams
            && content.len() >= self.options.compress_content_stream_min_bytes;
        if should_compress {
            let compressed = flate_compress(content);
            self.content_stream_encoded_bytes = self
                .content_stream_encoded_bytes
                .saturating_add(compressed.len());
            self.content_stream_compressed_count =
                self.content_stream_compressed_count.saturating_add(1);
            let mut dict = String::new();
            if !dict_entries.trim().is_empty() {
                dict.push_str(dict_entries.trim());
                dict.push(' ');
            }
            dict.push_str("/Filter /FlateDecode");
            self.write_stream_object_bytes(obj_id, &dict, &compressed)
        } else {
            self.content_stream_encoded_bytes = self
                .content_stream_encoded_bytes
                .saturating_add(content.len());
            self.write_stream_object_bytes(obj_id, dict_entries, content)
        }
    }

    fn write_uncompressed_content_stream_object(
        &mut self,
        obj_id: usize,
        dict_entries: &str,
        content: &[u8],
    ) -> io::Result<()> {
        self.content_stream_raw_bytes = self.content_stream_raw_bytes.saturating_add(content.len());
        self.content_stream_encoded_bytes = self
            .content_stream_encoded_bytes
            .saturating_add(content.len());
        self.write_stream_object_bytes(obj_id, dict_entries, content)
    }

    fn write_image_smask_stream_object(
        &mut self,
        obj_id: usize,
        alpha: &AlphaData,
    ) -> io::Result<()> {
        let dict = format!(
            "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent {} /Interpolate false /Filter {}",
            alpha.width, alpha.height, alpha.bits_per_component, alpha.filter
        );
        self.write_stream_object_bytes(obj_id, &dict, &alpha.data)
    }

    fn write_image_stream_object(
        &mut self,
        obj_id: usize,
        image: &ImageData,
        smask_id: Option<usize>,
    ) -> io::Result<()> {
        let smask = smask_id
            .map(|id| format!(" /SMask {} 0 R", id))
            .unwrap_or_default();
        let decode = image
            .decode
            .map(|value| format!(" /Decode {value}"))
            .unwrap_or_default();
        let dict = format!(
            "/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {} /BitsPerComponent {} /Interpolate false /Filter {}{}{}",
            image.width,
            image.height,
            image.color_space,
            image.bits_per_component,
            image.filter,
            decode,
            smask
        );
        self.write_stream_object_bytes(obj_id, &dict, &image.data)
    }

    fn write_font_file_stream_object(
        &mut self,
        obj_id: usize,
        data: &[u8],
        compressed: Option<&[u8]>,
        kind: FontProgramKind,
        source_len: usize,
        subset_glyphs: Option<usize>,
    ) -> io::Result<()> {
        let mut dict = format!("/Length1 {}", data.len());
        if matches!(kind, FontProgramKind::OpenTypeCff) {
            dict.push_str(" /Subtype /OpenType");
        }
        self.font_program_source_bytes = self.font_program_source_bytes.saturating_add(source_len);
        self.font_program_subset_bytes = self.font_program_subset_bytes.saturating_add(data.len());
        if let Some(glyphs) = subset_glyphs {
            self.font_program_subset_count = self.font_program_subset_count.saturating_add(1);
            self.font_program_subset_glyphs =
                self.font_program_subset_glyphs.saturating_add(glyphs);
            if let Some(compressed) = compressed.filter(|encoded| encoded.len() < data.len()) {
                dict.push_str(" /Filter /FlateDecode");
                self.font_program_encoded_bytes = self
                    .font_program_encoded_bytes
                    .saturating_add(compressed.len());
                self.font_program_compressed_count =
                    self.font_program_compressed_count.saturating_add(1);
                return self.write_stream_object_bytes(obj_id, &dict, compressed);
            }
        }
        self.font_program_encoded_bytes =
            self.font_program_encoded_bytes.saturating_add(data.len());
        self.write_stream_object_bytes(obj_id, &dict, data)
    }

    fn write_icc_profile_stream_object(
        &mut self,
        obj_id: usize,
        data: &[u8],
        n_components: u8,
    ) -> io::Result<()> {
        self.write_stream_object_bytes(obj_id, &format!("/N {}", n_components), data)
    }

    fn ensure_page_node(&mut self) -> usize {
        let needs_new = self
            .current_node
            .as_ref()
            .map(|n| n.kids.len() >= PDF_PAGE_NODE_MAX_KIDS)
            .unwrap_or(true);
        if needs_new {
            if let Some(node) = self.current_node.take() {
                self.page_nodes.push(node);
            }
            let id = self.alloc_ids(1);
            self.current_node = Some(PdfPageNode {
                id,
                kids: Vec::with_capacity(PDF_PAGE_NODE_MAX_KIDS),
            });
        }
        self.current_node
            .as_ref()
            .map(|n| n.id)
            .unwrap_or(PDF_PAGES_ID)
    }

    fn canonical_font_name(&self, name: &str) -> String {
        let trimmed = name.trim().trim_matches('"').trim_matches('\'');
        if let Some(registry) = self.registry {
            if let Some(font) = registry.resolve(trimmed) {
                return font.name.clone();
            }
        }
        trimmed.to_string()
    }

    fn record_doc_font_usage(&mut self, logical_name: &str) {
        self.doc_font_usage
            .entry(self.current_doc_id)
            .or_default()
            .insert(logical_name.to_string());
    }

    fn font_key(&self, name: &str) -> String {
        normalize_font_key(&self.canonical_font_name(name))
    }

    fn ensure_font(&mut self, name: &str) -> io::Result<()> {
        let logical_name = self.canonical_font_name(name);
        let key = self.font_key(&logical_name);
        if self.fonts.contains_key(&key) {
            self.record_doc_font_usage(&logical_name);
            return Ok(());
        }

        let resource = format!("F{}", self.next_font_resource);
        self.next_font_resource += 1;

        let mut kind = StreamFontKind::Type1;
        let mut encoding = FontEncoding::WinAnsi;
        let mut font_data = None;

        if self.options.pdf_profile.requires_embedded_fonts() {
            let Some(registry) = self.registry else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "{} requires a font registry for embedded font resolution",
                        self.options.pdf_profile.as_str()
                    ),
                ));
            };
            let Some(font) = registry.resolve(&logical_name) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "{} requires embedded fonts; unresolved font '{}'. register an embeddable font asset.",
                        self.options.pdf_profile.as_str(),
                        logical_name,
                    ),
                ));
            };
            if self.options.unicode_support {
                kind = StreamFontKind::TrueTypeIdentityH;
                encoding = FontEncoding::IdentityH;
                font_data = Some(font.data.as_slice());
            } else {
                kind = StreamFontKind::TrueTypeWinAnsi;
                encoding = FontEncoding::WinAnsi;
            }
        } else {
            // Default to base14 fonts when possible for speed and portability.
            let base14 = is_base14_font(&logical_name);
            if !base14 && self.options.unicode_support {
                if let Some(registry) = self.registry {
                    if let Some(font) = registry.resolve(&logical_name) {
                        kind = StreamFontKind::TrueTypeIdentityH;
                        encoding = FontEncoding::IdentityH;
                        font_data = Some(font.data.as_slice());
                    }
                }
            }
        }

        let start_id = self.alloc_ids(match kind {
            StreamFontKind::Type1 => 1,
            StreamFontKind::TrueTypeWinAnsi => 3,
            StreamFontKind::TrueTypeIdentityH => 5,
        });

        self.fonts.insert(
            key,
            StreamFont {
                logical_name: logical_name.clone(),
                resource,
                encoding,
                start_id,
                kind,
                glyph_map: BTreeMap::new(),
                font_data,
            },
        );
        self.record_doc_font_usage(&logical_name);
        Ok(())
    }

    fn ensure_type3_glyph(
        &mut self,
        name: &str,
        glyph_id: u16,
        synthetic_bold_millionths: u32,
    ) -> io::Result<Option<String>> {
        if glyph_id == 0 {
            return Ok(None);
        }
        let logical_name = self.canonical_font_name(name);
        let Some(registry) = self.registry else {
            return Ok(None);
        };
        if registry
            .glyph_outline_for_id(&logical_name, glyph_id)
            .is_none()
        {
            return Ok(None);
        }

        let key = (
            normalize_font_key(&logical_name),
            (glyph_id >> 8) as u8,
            synthetic_bold_millionths,
        );
        if !self.type3_fonts.contains_key(&key) {
            let resource = format!("T3F{}", self.next_type3_resource);
            self.next_type3_resource += 1;
            let font_id = self.alloc_ids(1);
            self.type3_fonts.insert(
                key.clone(),
                Type3StreamFont {
                    logical_name: logical_name.clone(),
                    resource,
                    font_id,
                    glyph_ids: BTreeSet::new(),
                    synthetic_bold_millionths,
                },
            );
        }
        let Some(font) = self.type3_fonts.get_mut(&key) else {
            return Ok(None);
        };
        font.glyph_ids.insert(glyph_id);
        Ok(Some(font.resource.clone()))
    }

    fn write_type3_font(
        &mut self,
        font_state: &Type3StreamFont,
        registry: &FontRegistry,
    ) -> io::Result<()> {
        let mut glyphs = Vec::with_capacity(font_state.glyph_ids.len());
        for glyph_id in &font_state.glyph_ids {
            let Some(outline) = registry.glyph_outline_for_id(&font_state.logical_name, *glyph_id)
            else {
                continue;
            };
            let (program, bbox, width) =
                type3_glyph_program(&outline, font_state.synthetic_bold_millionths);
            glyphs.push((*glyph_id, program, bbox, width));
        }
        if glyphs.is_empty() {
            return self.write_object(
                font_state.font_id,
                "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 0 0] /FontMatrix [0.001 0 0 -0.001 0 0] /CharProcs << >> /Encoding << /Type /Encoding /Differences [] >> /FirstChar 0 /LastChar 0 /Widths [0] /Resources << >> >>",
            );
        }

        let stream_start = self.alloc_ids(glyphs.len() + 1);
        let notdef_id = stream_start;
        self.write_stream_object_bytes(notdef_id, "", b"0 0 d0\n")?;

        let mut char_procs = vec![format!("/.notdef {} 0 R", notdef_id)];
        let mut differences = Vec::with_capacity(glyphs.len() * 2);
        let mut widths_by_code = BTreeMap::new();
        let mut font_bbox = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for (index, (glyph_id, program, bbox, width)) in glyphs.iter().enumerate() {
            let object_id = stream_start + index + 1;
            self.write_stream_object_bytes(object_id, "", program.as_bytes())?;
            let glyph_name = format!("g{:04X}", glyph_id);
            char_procs.push(format!("/{} {} 0 R", glyph_name, object_id));
            let code = glyph_id & 0x00ff;
            differences.push(code.to_string());
            differences.push(format!("/{}", glyph_name));
            widths_by_code.insert(code, *width);
            font_bbox[0] = font_bbox[0].min(bbox[0]);
            font_bbox[1] = font_bbox[1].min(bbox[1]);
            font_bbox[2] = font_bbox[2].max(bbox[2]);
            font_bbox[3] = font_bbox[3].max(bbox[3]);
        }

        let first_char = *widths_by_code.keys().next().unwrap_or(&0);
        let last_char = *widths_by_code.keys().next_back().unwrap_or(&first_char);
        let widths = (first_char..=last_char)
            .map(|code| fmt(*widths_by_code.get(&code).unwrap_or(&0.0)))
            .collect::<Vec<_>>()
            .join(" ");
        let base = format!(
            "{}-T3-{:02X}-B{}",
            sanitize_font_name(&font_state.logical_name),
            font_state.glyph_ids.iter().next().copied().unwrap_or(0) >> 8,
            font_state.synthetic_bold_millionths,
        );
        let object = format!(
            "<< /Type /Font /Subtype /Type3 /BaseFont /{} /FontBBox [{} {} {} {}] /FontMatrix [0.001 0 0 -0.001 0 0] /CharProcs << {} >> /Encoding << /Type /Encoding /Differences [{}] >> /FirstChar {} /LastChar {} /Widths [{}] /Resources << >> >>",
            base,
            fmt(font_bbox[0]),
            fmt(font_bbox[1]),
            fmt(font_bbox[2]),
            fmt(font_bbox[3]),
            char_procs.join(" "),
            differences.join(" "),
            first_char,
            last_char,
            widths,
        );
        self.write_object(font_state.font_id, &object)
    }

    fn ensure_image(&mut self, source: &str) -> io::Result<Option<String>> {
        if let Some(name) = self.image_name_map.get(source) {
            return Ok(Some(name.clone()));
        }
        let t_decode = std::time::Instant::now();
        let image = load_image(source);
        if let Some(perf) = self.perf.as_deref() {
            let ms = t_decode.elapsed().as_secs_f64() * 1000.0;
            perf.log_span_ms("image.decode", None, ms);
            if let Some(img) = &image {
                let mut bytes = img.data.len() as u64;
                if let Some(alpha) = &img.alpha {
                    bytes += alpha.data.len() as u64;
                }
                perf.log_counts("image.decode", None, &[("bytes", bytes)]);
            } else {
                perf.log_counts("image.decode", None, &[("missing", 1)]);
            }
        }

        let Some(image) = image else {
            if let Some(logger) = self.debug.as_deref() {
                let cwd = std::env::current_dir()
                    .ok()
                    .and_then(|p| p.to_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "<unknown>".to_string());
                let json = format!(
                    "{{\"type\":\"pdf.image.missing\",\"source\":{},\"cwd\":{}}}",
                    json_escape(source),
                    json_escape(&cwd)
                );
                logger.log_json(&json);
            }
            return Ok(None);
        };

        self.ensure_image_data(source, image)
    }

    fn ensure_image_variant(
        &mut self,
        source: &str,
        source_clip: ImageSourceClip,
        target_width: Pt,
        target_height: Pt,
    ) -> io::Result<Option<(String, Option<ResolvedImageSourceCrop>)>> {
        let image_key = format!(
            "{source}\0fullbleed-source-clip:{}:{}:{}:{}:{}:{}",
            source_clip.left.to_milli_i64(),
            source_clip.top.to_milli_i64(),
            source_clip.right.to_milli_i64(),
            source_clip.bottom.to_milli_i64(),
            target_width.to_milli_i64(),
            target_height.to_milli_i64(),
        );
        if let Some(name) = self.image_name_map.get(&image_key) {
            let crop = self.image_crop_map.get(&image_key).copied().flatten();
            return Ok(Some((name.clone(), crop)));
        }

        let t_decode = std::time::Instant::now();
        let loaded = load_image_variant(source, source_clip, target_width, target_height);
        if let Some(perf) = self.perf.as_deref() {
            perf.log_span_ms(
                "image.decode.crop",
                None,
                t_decode.elapsed().as_secs_f64() * 1000.0,
            );
            if let Some(loaded) = &loaded {
                let source_pixels = loaded
                    .crop
                    .map(|crop| {
                        u64::from(crop.source_width).saturating_mul(u64::from(crop.source_height))
                    })
                    .unwrap_or_else(|| {
                        u64::from(loaded.image.width).saturating_mul(u64::from(loaded.image.height))
                    });
                let embedded_pixels =
                    u64::from(loaded.image.width).saturating_mul(u64::from(loaded.image.height));
                perf.log_counts(
                    "image.decode.crop",
                    None,
                    &[
                        ("source_pixels", source_pixels),
                        ("embedded_pixels", embedded_pixels),
                    ],
                );
            } else {
                perf.log_counts("image.decode.crop", None, &[("missing", 1)]);
            }
        }

        let Some(loaded) = loaded else {
            if let Some(logger) = self.debug.as_deref() {
                let json = format!(
                    "{{\"type\":\"pdf.image.missing\",\"source\":{}}}",
                    json_escape(source),
                );
                logger.log_json(&json);
            }
            return Ok(None);
        };
        let crop = loaded.crop;
        let Some(name) = self.ensure_image_data(&image_key, loaded.image)? else {
            return Ok(None);
        };
        self.image_crop_map.insert(image_key, crop);
        Ok(Some((name, crop)))
    }

    fn ensure_image_data(&mut self, source: &str, image: ImageData) -> io::Result<Option<String>> {
        if let Some(name) = self.image_name_map.get(source) {
            return Ok(Some(name.clone()));
        }

        let hash = hash_image(&image);
        if self.options.reuse_xobjects {
            if let Some((name, _obj_id)) = self.image_content_map.get(&hash) {
                self.image_name_map.insert(source.to_string(), name.clone());
                return Ok(Some(name.clone()));
            }
        }

        let smask_id = image.alpha.as_ref().map(|_| self.alloc_ids(1));
        let obj_id = self.alloc_ids(1);
        let name = format!("Im{}", self.next_image_index);
        self.next_image_index += 1;

        self.image_bytes_total += image.data.len();
        if let Some(alpha) = &image.alpha {
            self.image_bytes_total += alpha.data.len();
        }
        if let (Some(alpha), Some(mask_id)) = (image.alpha.as_ref(), smask_id) {
            self.write_image_smask_stream_object(mask_id, alpha)?;
        }
        self.write_image_stream_object(obj_id, &image, smask_id)?;
        self.image_resources.push((name.clone(), obj_id));
        self.image_name_map.insert(source.to_string(), name.clone());
        if self.options.reuse_xobjects {
            self.image_content_map.insert(hash, (name.clone(), obj_id));
        }
        Ok(Some(name))
    }

    fn register_form_definition(
        &mut self,
        resource_id: &str,
        width: Pt,
        height: Pt,
        commands: &[Command],
        isolated: bool,
    ) {
        self.form_definition_map
            .entry(resource_id.to_string())
            .or_insert_with(|| (width, height, commands.to_vec()));
        self.form_isolated_map
            .entry(resource_id.to_string())
            .or_insert(isolated);
    }

    fn ensure_registered_form(&mut self, resource_id: &str) -> io::Result<Option<String>> {
        let Some((width, height, commands)) = self.form_definition_map.get(resource_id).cloned()
        else {
            return Ok(None);
        };
        let isolated = self
            .form_isolated_map
            .get(resource_id)
            .copied()
            .unwrap_or(false);
        self.ensure_form(resource_id, width, height, &commands, isolated)
    }

    fn ensure_registered_form_with_filter_offset(
        &mut self,
        resource_id: &str,
        filter_raster_offset: (Pt, Pt),
    ) -> io::Result<Option<String>> {
        let Some((width, height, commands)) = self.form_definition_map.get(resource_id).cloned()
        else {
            return Ok(None);
        };
        let isolated = self
            .form_isolated_map
            .get(resource_id)
            .copied()
            .unwrap_or(false);
        let specialized_id = format!(
            "{resource_id}#filter-offset:{}:{}",
            filter_raster_offset.0.to_milli_i64(),
            filter_raster_offset.1.to_milli_i64(),
        );
        self.ensure_form_with_filter_offset(
            &specialized_id,
            width,
            height,
            &commands,
            isolated,
            Some(filter_raster_offset),
        )
    }

    fn ensure_form(
        &mut self,
        resource_id: &str,
        width: Pt,
        height: Pt,
        commands: &[Command],
        isolated: bool,
    ) -> io::Result<Option<String>> {
        self.ensure_form_with_filter_offset(resource_id, width, height, commands, isolated, None)
    }

    fn ensure_form_with_filter_offset(
        &mut self,
        resource_id: &str,
        width: Pt,
        height: Pt,
        commands: &[Command],
        isolated: bool,
        filter_raster_offset: Option<(Pt, Pt)>,
    ) -> io::Result<Option<String>> {
        self.register_form_definition(resource_id, width, height, commands, isolated);
        if let Some(name) = self.form_name_map.get(resource_id) {
            return Ok(Some(name.clone()));
        }

        let content =
            self.render_commands_with_filter_offset(commands, height, None, filter_raster_offset)?;
        let mut hash_input = Vec::with_capacity(content.len() + 1);
        hash_input.push(u8::from(isolated));
        hash_input.extend_from_slice(content.as_bytes());
        let hash = hash_bytes(&hash_input);
        if self.options.reuse_xobjects {
            if let Some((name, _obj_id)) = self.form_content_map.get(&hash) {
                self.form_name_map
                    .insert(resource_id.to_string(), name.clone());
                self.form_size_map
                    .insert(resource_id.to_string(), Size { width, height });
                return Ok(Some(name.clone()));
            }
        }

        let obj_id = self.alloc_ids(1);
        let name = format!("Fm{}", self.next_form_index);
        self.next_form_index += 1;

        let group = if isolated {
            " /Group << /S /Transparency /I true /K false >>"
        } else {
            ""
        };
        let dict = format!(
            "/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 {} {}] /Resources {} 0 R{}",
            fmt_pt(width),
            fmt_pt(height),
            PDF_RESOURCES_ID,
            group,
        );

        self.write_content_stream_object(obj_id, &dict, content.as_bytes())?;
        self.form_resources.push((name.clone(), obj_id));
        self.form_name_map
            .insert(resource_id.to_string(), name.clone());
        self.form_size_map
            .insert(resource_id.to_string(), Size { width, height });
        if self.options.reuse_xobjects {
            self.form_content_map.insert(hash, (name.clone(), obj_id));
        }

        Ok(Some(name))
    }

    fn ensure_extgstate(&mut self, key: (u16, u16)) -> io::Result<Option<String>> {
        if let Some(name) = self.gs_name_map.get(&key) {
            return Ok(Some(name.clone()));
        }

        let (f, s) = key;
        let obj_id = self.alloc_ids(1);
        let name = format!("GS{}", self.next_gs_index);
        self.next_gs_index += 1;

        let ca = (f as f32) / 1000.0;
        let ca_stroke = (s as f32) / 1000.0;
        let obj = format!(
            "<< /Type /ExtGState /ca {} /CA {} >>",
            fmt(ca),
            fmt(ca_stroke)
        );
        self.write_object(obj_id, &obj)?;
        self.gs_resources.push((name.clone(), obj_id));
        self.gs_name_map.insert(key, name.clone());
        Ok(Some(name))
    }

    fn ensure_blend_extgstate(&mut self, mode: MixBlendMode) -> io::Result<Option<String>> {
        if matches!(mode, MixBlendMode::Normal) {
            return Ok(None);
        }
        if let Some(name) = self.gs_blend_name_map.get(&mode) {
            return Ok(Some(name.clone()));
        }

        let obj_id = self.alloc_ids(1);
        let name = format!("GS{}", self.next_gs_index);
        self.next_gs_index += 1;
        let blend = match mode {
            MixBlendMode::Normal => "Normal",
            MixBlendMode::Multiply => "Multiply",
            MixBlendMode::Screen => "Screen",
            MixBlendMode::Overlay => "Overlay",
            MixBlendMode::Darken => "Darken",
            MixBlendMode::Lighten => "Lighten",
            MixBlendMode::ColorDodge => "ColorDodge",
            MixBlendMode::ColorBurn => "ColorBurn",
            MixBlendMode::HardLight => "HardLight",
            MixBlendMode::SoftLight => "SoftLight",
            MixBlendMode::Difference => "Difference",
            MixBlendMode::Exclusion => "Exclusion",
            MixBlendMode::Hue => "Hue",
            MixBlendMode::Saturation => "Saturation",
            MixBlendMode::Color => "Color",
            MixBlendMode::Luminosity => "Luminosity",
            // PDF 1.7 has no standard additive plus-lighter blend mode.
            // Keep generated PDFs valid while the raster path provides the
            // deterministic plus-lighter oracle used by fixture validation.
            MixBlendMode::PlusLighter => "Lighten",
            // PDF 1.7 has no standard plus-darker/linear-burn blend mode.
            // Keep generated PDFs valid while raster output remains canonical.
            MixBlendMode::PlusDarker => "Darken",
        };
        let obj = format!("<< /Type /ExtGState /BM /{} >>", blend);
        self.write_object(obj_id, &obj)?;
        self.gs_resources.push((name.clone(), obj_id));
        self.gs_blend_name_map.insert(mode, name.clone());
        Ok(Some(name))
    }

    fn ensure_mask_coverage_extgstate(
        &mut self,
        cache_key: &str,
        raster: &crate::raster::MaskCoverageRaster,
        form_width: Pt,
        form_height: Pt,
        hard_sample_rows: bool,
    ) -> io::Result<Option<String>> {
        if let Some(name) = self.mask_coverage_gs_map.get(cache_key) {
            return Ok(Some(name.clone()));
        }

        let image_key = format!(
            "__fullbleed_mask_coverage_{:016x}_{}x{}",
            hash_bytes(&raster.coverage),
            raster.pixel_width,
            raster.pixel_height,
        );
        let image = ImageData {
            width: raster.pixel_width,
            height: raster.pixel_height,
            color_space: "/DeviceGray",
            bits_per_component: 8,
            filter: "/FlateDecode",
            decode: None,
            data: flate_compress(&raster.coverage),
            alpha: None,
        };
        let Some(image_name) = self.ensure_image_data(&image_key, image)? else {
            return Ok(None);
        };
        let Some(image_id) = self
            .image_resources
            .iter()
            .find_map(|(name, id)| (name == &image_name).then_some(*id))
        else {
            return Ok(None);
        };

        let mask_form_id = self.alloc_ids(1);
        let mask_dict = format!(
            "/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 {} {}] /Resources << /XObject << /{} {} 0 R >> >> /Group << /S /Transparency /CS /DeviceGray /I true /K false >>",
            fmt_pt(form_width),
            fmt_pt(form_height),
            image_name,
            image_id,
        );
        // Preserve the coverage kernel's native 300-DPI pixel grid. Stretching
        // a rounded raster dimension back to an exact fractional-point form
        // size introduces a second resampling phase (notably at 130/140 CSS px
        // heights). Align the top-left origins and let the form BBox clip the
        // sub-pixel excess or supply the sub-pixel transparent remainder.
        let points_per_pixel = 72.0 / PDF_FILTER_RASTER_DPI as f32;
        let raster_width = Pt::from_f32(raster.pixel_width as f32 * points_per_pixel);
        let raster_height = Pt::from_f32(
            (raster.pixel_height as f32 - if hard_sample_rows { 0.5 } else { 0.0 })
                * points_per_pixel,
        );
        let raster_y = if hard_sample_rows {
            // A half-row contraction makes PDF hard-stop masks sample one
            // cached shader row per output row instead of averaging adjacent
            // binary rows. Keep the contraction bottom-aligned; the analytic
            // half-pixel shader phase then matches the browser lattice.
            Pt::ZERO
        } else {
            form_height - raster_height
        };
        let mask_content = format!(
            "q\n{} 0 0 {} 0 {} cm\n/{} Do\nQ\n",
            fmt_pt(raster_width),
            fmt_pt(raster_height),
            fmt_pt(raster_y),
            image_name,
        );
        self.write_content_stream_object(mask_form_id, &mask_dict, mask_content.as_bytes())?;

        let gs_id = self.alloc_ids(1);
        let gs_name = format!("GS{}", self.next_gs_index);
        self.next_gs_index += 1;
        self.write_object(
            gs_id,
            &format!(
                "<< /Type /ExtGState /SMask << /S /Luminosity /G {} 0 R /BC [0] >> /AIS false >>",
                mask_form_id
            ),
        )?;
        self.gs_resources.push((gs_name.clone(), gs_id));
        self.mask_coverage_gs_map
            .insert(cache_key.to_string(), gs_name.clone());
        Ok(Some(gs_name))
    }

    fn ensure_vector_mask_extgstate(
        &mut self,
        cache_key: &str,
        commands: &[Command],
        command_width: Pt,
        command_height: Pt,
        form_width: Pt,
        form_height: Pt,
    ) -> io::Result<Option<String>> {
        if let Some(name) = self.mask_coverage_gs_map.get(cache_key) {
            return Ok(Some(name.clone()));
        }
        if command_width <= Pt::ZERO
            || command_height <= Pt::ZERO
            || form_width <= Pt::ZERO
            || form_height <= Pt::ZERO
        {
            return Ok(None);
        }

        // Compile the immutable alpha shader directly into a grayscale vector
        // transparency group. The source form remains a reusable vector XObject,
        // while only unsupported/composited mask programs fall back to a cached
        // coverage bitmap.
        let content = self.render_commands(commands, command_height, None)?;
        let sx = form_width.to_f32() / command_width.to_f32();
        let sy = form_height.to_f32() / command_height.to_f32();
        let content = if (sx - 1.0).abs() <= 1.0e-6 && (sy - 1.0).abs() <= 1.0e-6 {
            content
        } else {
            format!("q\n{} 0 0 {} 0 0 cm\n{}Q\n", fmt(sx), fmt(sy), content)
        };

        let mask_form_id = self.alloc_ids(1);
        let mask_dict = format!(
            "/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 {} {}] /Resources {} 0 R /Group << /S /Transparency /CS /DeviceRGB /I true /K false >>",
            fmt_pt(form_width),
            fmt_pt(form_height),
            PDF_RESOURCES_ID,
        );
        self.write_content_stream_object(mask_form_id, &mask_dict, content.as_bytes())?;

        let gs_id = self.alloc_ids(1);
        let gs_name = format!("GS{}", self.next_gs_index);
        self.next_gs_index += 1;
        self.write_object(
            gs_id,
            &format!(
                "<< /Type /ExtGState /SMask << /S /Luminosity /G {} 0 R /BC [0 0 0] >> /AIS false >>",
                mask_form_id
            ),
        )?;
        self.gs_resources.push((gs_name.clone(), gs_id));
        self.mask_coverage_gs_map
            .insert(cache_key.to_string(), gs_name.clone());
        Ok(Some(gs_name))
    }

    fn append_conic_shading(
        &mut self,
        out: &mut String,
        shading: &Shading,
        page_height: Pt,
    ) -> io::Result<()> {
        let Shading::Conic {
            center_x,
            center_y,
            radius,
            start_angle_deg,
            stops,
            hard_stops,
        } = shading
        else {
            return Ok(());
        };
        if stops.len() < 2 || *radius <= 0.0 {
            return Ok(());
        }

        let center_y = page_height.to_f32() - *center_y;
        // The IR radius reaches the farthest point in the CSS paint box. PDF
        // lowers a conic shader to triangles, whose outer edge is a chord: a
        // wide wedge at that radius can therefore cut back through the box.
        // Extend the rays well past the clip so every tessellated chord stays
        // outside the authored paint area. The surrounding background clip
        // still bounds all emitted pixels.
        let coverage_radius = *radius * 4.0;
        let append_wedge = |this: &mut Self,
                            out: &mut String,
                            start: f32,
                            end: f32,
                            stop: ShadingStop|
         -> io::Result<()> {
            if stop.alpha <= 1.0e-6 || end - start <= 1.0e-7 {
                return Ok(());
            }
            let opacity =
                ((stop.alpha.clamp(0.0, 1.0) * 1000.0).round() as i32).clamp(0, 1000) as u16;
            let gs = this.ensure_extgstate((opacity, opacity))?;
            let angle0 = (*start_angle_deg + start * 360.0).to_radians();
            let angle1 = (*start_angle_deg + end * 360.0).to_radians();
            let p0x = *center_x + angle0.sin() * coverage_radius;
            let p0y = center_y + angle0.cos() * coverage_radius;
            let p1x = *center_x + angle1.sin() * coverage_radius;
            let p1y = center_y + angle1.cos() * coverage_radius;
            out.push_str(&color_to_pdf_fill(stop.color, this.options.color_space));
            if let Some(gs) = gs {
                out.push_str(&format!("/{} gs\n", gs));
            }
            out.push_str(&format!(
                "{} {} m\n{} {} l\n{} {} l\nh\nf\n",
                fmt(*center_x),
                fmt(center_y),
                fmt(p0x),
                fmt(p0y),
                fmt(p1x),
                fmt(p1y),
            ));
            Ok(())
        };
        let append_sector = |this: &mut Self,
                             out: &mut String,
                             start: f32,
                             end: f32,
                             stop: ShadingStop,
                             pieces: usize|
         -> io::Result<()> {
            if stop.alpha <= 1.0e-6 || end - start <= 1.0e-7 {
                return Ok(());
            }
            let opacity =
                ((stop.alpha.clamp(0.0, 1.0) * 1000.0).round() as i32).clamp(0, 1000) as u16;
            let gs = this.ensure_extgstate((opacity, opacity))?;
            out.push_str(&color_to_pdf_fill(stop.color, this.options.color_space));
            if let Some(gs) = gs {
                out.push_str(&format!("/{} gs\n", gs));
            }
            out.push_str(&format!("{} {} m\n", fmt(*center_x), fmt(center_y)));
            for edge in 0..=pieces {
                let position = start + (end - start) * edge as f32 / pieces as f32;
                let angle = (*start_angle_deg + position * 360.0).to_radians();
                let px = *center_x + angle.sin() * coverage_radius;
                let py = center_y + angle.cos() * coverage_radius;
                out.push_str(&format!("{} {} l\n", fmt(px), fmt(py)));
            }
            out.push_str("h\nf\n");
            Ok(())
        };

        out.push_str("q\n");
        if *hard_stops {
            const BOUNDARY_PHASE: f32 = 1.0e-5;
            const MAX_HARD_WEDGE_TURN: f32 = 0.125;
            for pair in stops.windows(2) {
                if pair[1].offset - pair[0].offset <= 1.0e-6 {
                    continue;
                }
                let start = if pair[0].offset <= 0.0 {
                    0.0
                } else {
                    pair[0].offset + BOUNDARY_PHASE
                };
                let end = if pair[1].offset >= 1.0 {
                    1.0
                } else {
                    pair[1].offset + BOUNDARY_PHASE
                };
                // Preserve one compact conic command in the compiled IR, but
                // tessellate each broad constant-colour sector into one path
                // at PDF emission. One triangle cannot represent a sector
                // wider than 180deg, and even a 90deg chord can expose clipped
                // box corners. A single multi-vertex path also avoids internal
                // antialias seams between same-colour triangles.
                let band_span = pair[1].offset - pair[0].offset;
                let pieces = (band_span / MAX_HARD_WEDGE_TURN).ceil().max(1.0) as usize;
                append_sector(self, out, start, end, pair[0], pieces)?;
            }
        } else {
            let steps = ((*radius * std::f32::consts::TAU) / 2.0)
                .round()
                .clamp(128.0, 720.0) as usize;
            let overlap = 0.4 / steps as f32;
            for index in 0..steps {
                let start = index as f32 / steps as f32;
                let end = (index + 1) as f32 / steps as f32;
                let sample = sample_shading_stop(stops, (start + end) * 0.5);
                append_wedge(self, out, start - overlap, end + overlap, sample)?;
            }
        }
        out.push_str("Q\n");
        Ok(())
    }

    fn ensure_shading(
        &mut self,
        key: u64,
        shading: &Shading,
        page_height: Pt,
    ) -> io::Result<Option<(String, Option<String>)>> {
        if let Some(name) = self.shading_name_map.get(&key) {
            return Ok(Some((
                name.clone(),
                self.shading_alpha_gs_map.get(&key).cloned(),
            )));
        }

        let name = format!("Sh{}", self.next_shading_index);
        self.next_shading_index += 1;

        let has_alpha = shading_stops(shading)
            .iter()
            .any(|stop| stop.alpha < 1.0 - f32::EPSILON);
        let color_shading = if has_alpha {
            premultiplied_color_shading(shading)
        } else {
            shading.clone()
        };

        let start_id = self.next_id;
        let (objs, sh_obj_id, new_next) = shading_to_objects(
            &color_shading,
            start_id,
            page_height,
            self.options.color_space,
        );
        self.next_id = new_next;
        self.ensure_offsets_len(self.next_id);

        for (i, obj) in objs.iter().enumerate() {
            self.write_object(start_id + i, obj)?;
        }

        self.shading_resources.push((name.clone(), sh_obj_id));
        self.shading_name_map.insert(key, name.clone());

        let alpha_gs = if has_alpha {
            let alpha_shading = alpha_only_shading(shading);
            let alpha_start_id = self.next_id;
            let (alpha_objs, alpha_shading_id, alpha_next_id) =
                shading_to_objects(&alpha_shading, alpha_start_id, page_height, ColorSpace::Rgb);
            self.next_id = alpha_next_id;
            self.ensure_offsets_len(self.next_id);
            for (i, obj) in alpha_objs.iter().enumerate() {
                self.write_object(alpha_start_id + i, obj)?;
            }

            let mask_form_id = self.alloc_ids(1);
            let mask_dict = format!(
                "/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 {} {}] /Resources << /Shading << /MaskSh {} 0 R >> >> /Group << /S /Transparency /CS /DeviceRGB /I true /K false >>",
                fmt_pt(self.page_size.width),
                fmt_pt(page_height),
                alpha_shading_id,
            );
            self.write_stream_object_bytes(mask_form_id, &mask_dict, b"/MaskSh sh\n")?;

            let gs_id = self.alloc_ids(1);
            let gs_name = format!("GS{}", self.next_gs_index);
            self.next_gs_index += 1;
            self.write_object(
                gs_id,
                &format!(
                    "<< /Type /ExtGState /SMask << /S /Luminosity /G {} 0 R /BC [0 0 0] >> /AIS false >>",
                    mask_form_id
                ),
            )?;
            self.gs_resources.push((gs_name.clone(), gs_id));
            self.shading_alpha_gs_map.insert(key, gs_name.clone());
            Some(gs_name)
        } else {
            None
        };

        Ok(Some((name, alpha_gs)))
    }

    fn shape_text_to_tj(
        &mut self,
        font_key: &str,
        _font_name: &str,
        font_size: Pt,
        text: &str,
    ) -> Option<&str> {
        if !self.options.shape_text {
            return None;
        }
        let key = tj_cache_key(font_key, font_size, text);
        if !self.shaped_cache.contains_key(&key) {
            let shaped = {
                let font_state = self.fonts.get_mut(font_key)?;
                let font_data = font_state.font_data?;
                let shaped = shape_text_native(font_data, text)?;
                for (gid, s) in &shaped.glyph_map {
                    font_state
                        .glyph_map
                        .entry(*gid)
                        .or_insert_with(|| s.clone());
                }
                shaped
            };
            self.shaped_cache.insert(key.clone(), shaped);
        }
        self.shaped_cache.get(&key).map(|s| s.tj.as_str())
    }

    fn encode_cid_hex_fallback(&mut self, font_key: &str, font_name: &str, text: &str) -> String {
        let (_, text) = crate::text_shape::decode_shape_options(text);
        let mut out = String::new();
        out.push('<');
        if let Some(registry) = self.registry {
            for ch in text.chars() {
                let gid = registry.map_glyph_id_for_char(font_name, ch);
                if gid != 0 {
                    if let Some(font_state) = self.fonts.get_mut(font_key) {
                        font_state
                            .glyph_map
                            .entry(gid)
                            .or_insert_with(|| ch.to_string());
                    }
                }
                out.push_str(&format!("{:04X}", gid));
            }
        } else {
            for _ in text.chars() {
                out.push_str("0000");
            }
        }
        out.push('>');
        out
    }
}

fn normalize_font_key(name: &str) -> String {
    name.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_lowercase()
}

fn is_base14_font(name: &str) -> bool {
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

fn shape_text_native(font_data: &[u8], text: &str) -> Option<ShapedText> {
    let (_, clean_text) = crate::text_shape::decode_shape_options(text);
    let face = SfntFace::parse(font_data, 0).ok()?;
    let shaped = crate::text_shape::shape(font_data, text)?;
    let units_per_em = shaped.units_per_em.max(1);
    if shaped.glyphs.is_empty() {
        return None;
    }

    // Build a map from glyph id -> source unicode string (cluster range).
    let mut boundaries: Vec<usize> = shaped
        .glyphs
        .iter()
        .map(|glyph| glyph.cluster as usize)
        .collect();
    boundaries.sort_unstable();
    boundaries.dedup();
    if boundaries.last().copied() != Some(clean_text.len()) {
        boundaries.push(clean_text.len());
    }

    let mut glyph_map: BTreeMap<u16, String> = BTreeMap::new();
    for glyph in &shaped.glyphs {
        let start = (glyph.cluster as usize).min(clean_text.len());
        let idx = match boundaries.binary_search(&start) {
            Ok(i) => i,
            Err(i) => i,
        };
        let end = boundaries
            .get(idx + 1)
            .copied()
            .unwrap_or(clean_text.len())
            .min(clean_text.len());
        if start < end {
            glyph_map
                .entry(glyph.glyph_id)
                .or_insert_with(|| clean_text[start..end].to_string());
        }
    }

    // Build a TJ array.
    let mut parts: Vec<String> = Vec::new();
    for glyph in &shaped.glyphs {
        let gid = glyph.glyph_id;
        if gid == 0 {
            continue;
        }

        if glyph.x_offset != 0 {
            parts.push(format_font_units(-i64::from(glyph.x_offset), units_per_em));
        }
        parts.push(format!("<{:04X}>", gid));

        let adv_default = i64::from(face.glyph_hor_advance(GlyphId(gid)).unwrap_or(0));
        let adjust = adv_default - i64::from(glyph.x_advance);
        if adjust != 0 {
            parts.push(format_font_units(adjust, units_per_em));
        }
    }

    if parts.is_empty() {
        return None;
    }

    Some(ShapedText {
        tj: format!("[{}] TJ\n", parts.join(" ")),
        glyph_map,
    })
}

#[allow(dead_code)]
pub fn document_to_pdf(document: &Document) -> io::Result<Vec<u8>> {
    document_to_pdf_with_registry(document, None)
}

#[allow(dead_code)]
pub fn document_to_pdf_with_metrics(
    document: &Document,
    mut metrics: Option<&mut DocumentMetrics>,
) -> io::Result<Vec<u8>> {
    let options = PdfOptions::default();
    document_to_pdf_with_metrics_and_registry(document, metrics.as_deref_mut(), None, &options)
}

pub(crate) fn document_to_pdf_with_registry(
    document: &Document,
    registry: Option<&FontRegistry>,
) -> io::Result<Vec<u8>> {
    let options = PdfOptions::default();
    document_to_pdf_with_metrics_and_registry(document, None, registry, &options)
}

pub(crate) fn document_to_pdf_with_metrics_and_registry(
    document: &Document,
    mut metrics: Option<&mut DocumentMetrics>,
    registry: Option<&FontRegistry>,
    options: &PdfOptions,
) -> io::Result<Vec<u8>> {
    document_to_pdf_with_metrics_and_registry_with_logs(
        document,
        metrics.as_deref_mut(),
        registry,
        options,
        None,
        None,
    )
}

pub(crate) fn document_to_pdf_with_metrics_and_registry_with_logs(
    document: &Document,
    mut metrics: Option<&mut DocumentMetrics>,
    registry: Option<&FontRegistry>,
    options: &PdfOptions,
    debug: Option<std::sync::Arc<crate::debug::DebugLogger>>,
    perf: Option<std::sync::Arc<PerfLogger>>,
) -> io::Result<Vec<u8>> {
    let mut bytes: Vec<u8> = Vec::new();
    let _ = document_to_pdf_with_metrics_and_registry_to_writer_with_logs(
        document,
        metrics.as_deref_mut(),
        registry,
        options,
        &mut bytes,
        debug,
        perf,
    )?;
    Ok(bytes)
}

#[allow(dead_code)]
pub(crate) fn document_to_pdf_with_metrics_and_registry_to_writer<W: Write>(
    document: &Document,
    metrics: Option<&mut DocumentMetrics>,
    registry: Option<&FontRegistry>,
    options: &PdfOptions,
    writer: &mut W,
) -> io::Result<usize> {
    document_to_pdf_with_metrics_and_registry_to_writer_with_logs(
        document, metrics, registry, options, writer, None, None,
    )
}

pub(crate) fn document_to_pdf_with_metrics_and_registry_to_writer_with_logs<W: Write>(
    document: &Document,
    mut metrics: Option<&mut DocumentMetrics>,
    registry: Option<&FontRegistry>,
    options: &PdfOptions,
    writer: &mut W,
    debug: Option<std::sync::Arc<crate::debug::DebugLogger>>,
    perf: Option<std::sync::Arc<PerfLogger>>,
) -> io::Result<usize> {
    let mut pdf_stream = PdfStreamWriter::new(
        writer,
        document.page_size,
        registry,
        options.clone(),
        debug,
        perf,
    )?;
    pdf_stream.add_document(0, document)?;
    let total_bytes = pdf_stream.finish()?;

    if let Some(metrics) = metrics.as_deref_mut() {
        metrics.total_bytes = total_bytes;
        for (page_index, content_bytes) in pdf_stream.page_content_bytes.iter().enumerate() {
            if metrics.pages.len() <= page_index {
                metrics
                    .pages
                    .resize_with(page_index + 1, PageMetrics::default);
            }
            let entry = &mut metrics.pages[page_index];
            if entry.page_number == 0 {
                entry.page_number = page_index + 1;
            }
            entry.content_bytes = *content_bytes;
        }
    }

    Ok(total_bytes)
}

fn collect_used_font_names_in_commands(commands: &[Command], names: &mut BTreeSet<String>) {
    let mut current_font = "Helvetica".to_string();
    for cmd in commands {
        match cmd {
            Command::SetFontName(name) => current_font = name.clone(),
            Command::DrawString { .. } | Command::DrawStringTransformed { .. } => {
                names.insert(current_font.clone());
            }
            Command::DefineForm {
                commands: form_commands,
                ..
            }
            | Command::DefineIsolatedForm {
                commands: form_commands,
                ..
            } => collect_used_font_names_in_commands(form_commands, names),
            _ => {}
        }
    }
}

fn collect_used_font_names(document: &Document) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for page in &document.pages {
        collect_used_font_names_in_commands(&page.commands, &mut names);
    }
    names
}

#[allow(dead_code)]
fn collect_font_names(document: &Document) -> Vec<String> {
    collect_used_font_names(document).into_iter().collect()
}

fn validate_profile_font_embedding(
    document: &Document,
    registry: Option<&FontRegistry>,
    options: &PdfOptions,
) -> io::Result<()> {
    if !options.pdf_profile.requires_embedded_fonts() {
        return Ok(());
    }
    let used_fonts = collect_used_font_names(document);
    if used_fonts.is_empty() {
        return Ok(());
    }
    let Some(registry) = registry else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} requires a font registry for embedded font resolution",
                options.pdf_profile.as_str()
            ),
        ));
    };
    for name in used_fonts {
        if registry.resolve(&name).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{} requires embedded fonts; unresolved font '{}'. register an embeddable font asset.",
                    options.pdf_profile.as_str(),
                    name
                ),
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_profile_output_intent(options: &PdfOptions) -> io::Result<()> {
    if !options.pdf_profile.requires_output_intent() {
        return Ok(());
    }
    let Some(intent) = options.output_intent.as_ref() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} requires an output intent", options.pdf_profile.as_str()),
        ));
    };
    if intent.icc_profile.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} output intent ICC profile cannot be empty",
                options.pdf_profile.as_str()
            ),
        ));
    }
    if !matches!(intent.n_components, 1 | 3 | 4) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} output intent n_components must be one of 1, 3, or 4 (got {})",
                options.pdf_profile.as_str(),
                intent.n_components,
            ),
        ));
    }
    if intent.identifier.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} output intent identifier cannot be empty",
                options.pdf_profile.as_str()
            ),
        ));
    }
    Ok(())
}

#[allow(dead_code)]
fn collect_optional_content_names_in_commands(commands: &[Command], names: &mut BTreeSet<String>) {
    for cmd in commands {
        match cmd {
            Command::BeginOptionalContent { name } => {
                names.insert(name.clone());
            }
            Command::DefineForm {
                commands: form_commands,
                ..
            }
            | Command::DefineIsolatedForm {
                commands: form_commands,
                ..
            } => collect_optional_content_names_in_commands(form_commands, names),
            _ => {}
        }
    }
}

#[allow(dead_code)]
fn collect_optional_content_names(document: &Document) -> Vec<String> {
    let mut names = BTreeSet::new();
    for page in &document.pages {
        collect_optional_content_names_in_commands(&page.commands, &mut names);
    }
    names.into_iter().collect()
}

#[allow(dead_code)]
fn collect_tag_records(document: &Document) -> Vec<TagRecord> {
    let mut records = Vec::new();
    for (page_index, page) in document.pages.iter().enumerate() {
        let mut stack: Vec<usize> = Vec::new();
        for cmd in &page.commands {
            match cmd {
                Command::BeginTag {
                    role,
                    mcid,
                    alt,
                    scope,
                    table_id,
                    col_index,
                    group_only: _,
                } => {
                    let parent = stack.last().copied();
                    let idx = records.len();
                    records.push(TagRecord {
                        page_index,
                        mcid: *mcid,
                        role: role.clone(),
                        alt: alt.clone(),
                        scope: scope.clone(),
                        parent,
                        table_id: *table_id,
                        col_index: *col_index,
                    });
                    stack.push(idx);
                }
                Command::EndTag => {
                    let _ = stack.pop();
                }
                _ => {}
            }
        }
    }
    records
}

#[allow(dead_code)]
fn collect_font_usage(
    document: &Document,
    registry: Option<&FontRegistry>,
    glyph_cache: &mut HashMap<String, BTreeMap<u16, String>>,
    options: &PdfOptions,
) -> HashMap<String, FontUsage> {
    let mut map: HashMap<String, FontUsage> = HashMap::new();
    let mut current_font = "Helvetica".to_string();

    for page in &document.pages {
        for cmd in &page.commands {
            match cmd {
                Command::SetFontName(name) => current_font = name.clone(),
                Command::DrawString { text, .. } | Command::DrawStringTransformed { text, .. } => {
                    let Some(registry) = registry else {
                        continue;
                    };
                    let Some(font) = registry.resolve(&current_font) else {
                        continue;
                    };
                    let usage = map.entry(current_font.clone()).or_default();
                    let cache_key = glyph_cache_key(&current_font, text);
                    let glyph_map = if let Some(cached) = glyph_cache.get(&cache_key) {
                        cached.clone()
                    } else {
                        let local_map = if options.shape_text && options.unicode_support {
                            if let Some(glyph_map) = shape_text_to_glyph_map(&font.data, text) {
                                glyph_map
                            } else {
                                let mut fallback = BTreeMap::new();
                                for ch in text.chars() {
                                    let gid = registry.map_glyph_id_for_char(&current_font, ch);
                                    if gid != 0 {
                                        fallback.entry(gid).or_insert(ch.to_string());
                                    }
                                }
                                fallback
                            }
                        } else {
                            let mut fallback = BTreeMap::new();
                            for ch in text.chars() {
                                let gid = registry.map_glyph_id_for_char(&current_font, ch);
                                if gid != 0 {
                                    fallback.entry(gid).or_insert(ch.to_string());
                                }
                            }
                            fallback
                        };
                        glyph_cache.insert(cache_key, local_map.clone());
                        local_map
                    };
                    for (gid, s) in glyph_map {
                        usage.glyph_map.entry(gid).or_insert(s);
                    }
                }
                _ => {}
            }
        }
    }
    map
}

#[allow(dead_code)]
fn collect_image_sources(document: &Document) -> Vec<String> {
    let mut sources = BTreeSet::new();
    for page in &document.pages {
        for cmd in &page.commands {
            if let Command::DrawImage { resource_id, .. } = cmd {
                sources.insert(resource_id.clone());
            }
        }
    }
    sources.into_iter().collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FontEncoding {
    WinAnsi,
    IdentityH,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct FontResource {
    resource: String,
    encoding: FontEncoding,
}

#[derive(Default)]
struct FontUsage {
    glyph_map: BTreeMap<u16, String>,
}

#[allow(dead_code)]
fn build_font_map(fonts: &[String]) -> BTreeMap<String, FontResource> {
    let mut map = BTreeMap::new();
    for (index, name) in fonts.iter().enumerate() {
        map.insert(
            name.clone(),
            FontResource {
                resource: format!("F{}", index + 1),
                encoding: FontEncoding::WinAnsi,
            },
        );
    }
    map
}

#[allow(dead_code)]
fn glyph_cache_key(font_name: &str, text: &str) -> String {
    let mut key = String::with_capacity(font_name.len() + 1 + text.len());
    key.push_str(font_name);
    key.push('\0');
    key.push_str(text);
    key
}

fn tj_cache_key(font_name: &str, font_size: Pt, text: &str) -> String {
    let mut key = String::with_capacity(font_name.len() + 1 + text.len() + 16);
    key.push_str(font_name);
    key.push('\0');
    key.push_str(&font_size.to_milli_i64().to_string());
    key.push('\0');
    key.push_str(text);
    key
}

#[allow(dead_code)]
fn cached_shape_text_to_tj(
    registry: &FontRegistry,
    font_name: &str,
    font_size: Pt,
    text: &str,
    tj_cache: &mut HashMap<String, String>,
    options: &PdfOptions,
) -> Option<String> {
    if !options.shape_text {
        return None;
    }
    let key = tj_cache_key(font_name, font_size, text);
    if let Some(cached) = tj_cache.get(&key) {
        return Some(cached.clone());
    }
    let tj = shape_text_to_tj(registry, font_name, font_size, text)?;
    tj_cache.insert(key, tj.clone());
    Some(tj)
}

#[allow(dead_code)]
fn build_extgstate_objects(
    document: &Document,
    start_id: usize,
) -> (
    Vec<String>,
    Vec<(String, usize)>,
    HashMap<(u16, u16), String>,
    usize,
) {
    // Map (fill_alpha, stroke_alpha) -> /GSn resource.
    let mut pairs: BTreeSet<(u16, u16)> = BTreeSet::new();
    for page in &document.pages {
        for cmd in &page.commands {
            if let Command::SetOpacity { fill, stroke } = cmd {
                let f = ((*fill * 1000.0).round() as i32).clamp(0, 1000) as u16;
                let s = ((*stroke * 1000.0).round() as i32).clamp(0, 1000) as u16;
                pairs.insert((f, s));
            }
        }
    }

    let mut objects = Vec::new();
    let mut resources = Vec::new();
    let mut name_map: HashMap<(u16, u16), String> = HashMap::new();
    let mut next_id = start_id;
    let mut index = 1usize;

    for (f, s) in pairs {
        let obj_id = next_id;
        next_id += 1;
        let name = format!("GS{}", index);
        index += 1;

        let ca = (f as f32) / 1000.0;
        let ca_stroke = (s as f32) / 1000.0;
        objects.push(format!(
            "<< /Type /ExtGState /ca {} /CA {} >>",
            fmt(ca),
            fmt(ca_stroke)
        ));
        resources.push((name.clone(), obj_id));
        name_map.insert((f, s), name);
    }

    (objects, resources, name_map, next_id)
}

#[allow(dead_code)]
fn build_shading_objects(
    document: &Document,
    start_id: usize,
    page_height: Pt,
    color_space: ColorSpace,
) -> (
    Vec<String>,
    Vec<(String, usize)>,
    HashMap<u64, String>,
    usize,
) {
    // Map shading hash -> /ShN resource.
    let mut unique: BTreeMap<u64, Shading> = BTreeMap::new();
    for page in &document.pages {
        for cmd in &page.commands {
            if let Command::ShadingFill(sh) = cmd {
                unique.entry(hash_shading(sh)).or_insert_with(|| sh.clone());
            }
        }
    }

    let mut objects = Vec::new();
    let mut resources = Vec::new();
    let mut name_map: HashMap<u64, String> = HashMap::new();
    let mut next_id = start_id;
    let mut index = 1usize;

    for (key, shading) in unique {
        let name = format!("Sh{}", index);
        index += 1;

        let (mut sh_objs, sh_obj_id, new_next) =
            shading_to_objects(&shading, next_id, page_height, color_space);
        next_id = new_next;

        objects.append(&mut sh_objs);
        resources.push((name.clone(), sh_obj_id));
        name_map.insert(key, name);
    }

    (objects, resources, name_map, next_id)
}

struct ImageData {
    width: u32,
    height: u32,
    color_space: &'static str,
    bits_per_component: u8,
    filter: &'static str,
    decode: Option<&'static str>,
    data: Vec<u8>,
    alpha: Option<AlphaData>,
}

struct AlphaData {
    width: u32,
    height: u32,
    bits_per_component: u8,
    filter: &'static str,
    data: Vec<u8>,
}

struct ImageVariantData {
    image: ImageData,
    crop: Option<ResolvedImageSourceCrop>,
}

fn load_image(source: &str) -> Option<ImageData> {
    if let Some((mime, data)) = parse_data_uri(source) {
        return decode_image_bytes(&data, Some(&mime));
    }

    let path = Path::new(source);
    let bytes = std::fs::read(path).ok()?;
    decode_image_bytes(&bytes, None)
}

fn load_image_variant(
    source: &str,
    source_clip: ImageSourceClip,
    target_width: Pt,
    target_height: Pt,
) -> Option<ImageVariantData> {
    if let Some((mime, data)) = parse_data_uri(source) {
        return decode_image_bytes_variant(
            &data,
            Some(&mime),
            source_clip,
            target_width,
            target_height,
        );
    }

    let bytes = std::fs::read(Path::new(source)).ok()?;
    decode_image_bytes_variant(&bytes, None, source_clip, target_width, target_height)
}

fn image_data_from_premultiplied_rgba(
    width: u32,
    height: u32,
    premultiplied: &[u8],
) -> Option<ImageData> {
    let pixel_count = width.checked_mul(height)? as usize;
    if premultiplied.len() != pixel_count.checked_mul(4)? {
        return None;
    }

    let mut rgb = Vec::with_capacity(pixel_count * 3);
    let mut alpha = Vec::with_capacity(pixel_count);
    let mut has_alpha = false;
    for pixel in premultiplied.chunks_exact(4) {
        let a = pixel[3];
        has_alpha |= a != 255;
        let unpremultiply = |channel: u8| -> u8 {
            if a == 0 {
                0
            } else {
                ((channel as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8
            }
        };
        rgb.extend_from_slice(&[
            unpremultiply(pixel[0]),
            unpremultiply(pixel[1]),
            unpremultiply(pixel[2]),
        ]);
        alpha.push(a);
    }

    Some(ImageData {
        width,
        height,
        color_space: "/DeviceRGB",
        bits_per_component: 8,
        filter: "/FlateDecode",
        decode: None,
        data: flate_compress(&rgb),
        alpha: has_alpha.then(|| AlphaData {
            width,
            height,
            bits_per_component: 8,
            filter: "/FlateDecode",
            data: flate_compress(&alpha),
        }),
    })
}

fn decode_image_bytes(data: &[u8], mime: Option<&str>) -> Option<ImageData> {
    let format = if let Some(mime) = mime {
        if mime.contains("png") {
            Some(crate::image_native::ImageFormat::Png)
        } else if mime.contains("jpeg") || mime.contains("jpg") {
            Some(crate::image_native::ImageFormat::Jpeg)
        } else {
            None
        }
    } else {
        crate::image_native::guess_format(data).ok()
    };

    let decoded = crate::image_native::load_from_memory(data).ok()?;
    let (width, height) = decoded.dimensions();

    if matches!(format, Some(crate::image_native::ImageFormat::Jpeg)) {
        let color_space = match decoded.color() {
            crate::image_native::ImageColor::Gray | crate::image_native::ImageColor::GrayAlpha => {
                "/DeviceGray"
            }
            crate::image_native::ImageColor::Cmyk => "/DeviceCMYK",
            _ => "/DeviceRGB",
        };
        return Some(ImageData {
            width,
            height,
            color_space,
            bits_per_component: 8,
            filter: "/DCTDecode",
            decode: (matches!(decoded.color(), crate::image_native::ImageColor::Cmyk)
                && decoded.jpeg_adobe_transform().is_some())
            .then_some("[1 0 1 0 1 0 1 0]"),
            data: data.to_vec(),
            alpha: None,
        });
    }

    let rgba = decoded.to_rgba8();
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    let mut alpha = Vec::with_capacity((width * height) as usize);
    let mut has_alpha = false;
    for pixel in rgba.pixels() {
        let [r, g, b, a] = pixel.0;
        if a != 255 {
            has_alpha = true;
        }
        rgb.extend_from_slice(&[r, g, b]);
        alpha.push(a);
    }

    let compressed = flate_compress(&rgb);
    let alpha = if has_alpha {
        Some(AlphaData {
            width,
            height,
            bits_per_component: 8,
            filter: "/FlateDecode",
            data: flate_compress(&alpha),
        })
    } else {
        None
    };
    Some(ImageData {
        width,
        height,
        color_space: "/DeviceRGB",
        bits_per_component: 8,
        filter: "/FlateDecode",
        decode: None,
        data: compressed,
        alpha,
    })
}

fn decode_image_bytes_variant(
    data: &[u8],
    mime: Option<&str>,
    source_clip: ImageSourceClip,
    target_width: Pt,
    target_height: Pt,
) -> Option<ImageVariantData> {
    let format = if let Some(mime) = mime {
        if mime.contains("png") {
            Some(crate::image_native::ImageFormat::Png)
        } else if mime.contains("jpeg") || mime.contains("jpg") {
            Some(crate::image_native::ImageFormat::Jpeg)
        } else {
            None
        }
    } else {
        crate::image_native::guess_format(data).ok()
    };
    let decoded = if let Some(format) = format {
        crate::image_native::load_from_memory_with_format(data, format).ok()?
    } else {
        crate::image_native::load_from_memory(data).ok()?
    };
    let (source_width, source_height) = decoded.dimensions();
    let crop = source_clip.resolve(target_width, target_height, source_width, source_height);
    let Some(crop) = crop else {
        return decode_image_bytes(data, mime).map(|image| ImageVariantData { image, crop: None });
    };

    let rgba = decoded.to_rgba8();
    let source = rgba.as_raw();
    let pixel_count = crop.width.checked_mul(crop.height)? as usize;
    let mut rgb = Vec::with_capacity(pixel_count.checked_mul(3)?);
    let mut alpha = Vec::with_capacity(pixel_count);
    let mut has_alpha = false;
    for source_y in crop.y..crop.y + crop.height {
        let row_start = (u64::from(source_y)
            .checked_mul(u64::from(source_width))?
            .checked_add(u64::from(crop.x))?
            .checked_mul(4)?) as usize;
        let row_bytes = (crop.width as usize).checked_mul(4)?;
        let row = source.get(row_start..row_start.checked_add(row_bytes)?)?;
        for pixel in row.chunks_exact(4) {
            let [r, g, b, a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
            has_alpha |= a != 255;
            rgb.extend_from_slice(&[r, g, b]);
            alpha.push(a);
        }
    }
    let alpha = has_alpha.then(|| AlphaData {
        width: crop.width,
        height: crop.height,
        bits_per_component: 8,
        filter: "/FlateDecode",
        data: flate_compress(&alpha),
    });
    Some(ImageVariantData {
        image: ImageData {
            width: crop.width,
            height: crop.height,
            color_space: "/DeviceRGB",
            bits_per_component: 8,
            filter: "/FlateDecode",
            decode: None,
            data: flate_compress(&rgb),
            alpha,
        },
        crop: Some(crop),
    })
}

fn parse_data_uri(uri: &str) -> Option<(String, Vec<u8>)> {
    if !uri.starts_with("data:") {
        return None;
    }
    let parts: Vec<&str> = uri.splitn(2, ',').collect();
    if parts.len() != 2 {
        return None;
    }
    let header = parts[0];
    let data_part = parts[1];
    let mime = header
        .trim_start_matches("data:")
        .split(';')
        .next()
        .unwrap_or("application/octet-stream")
        .to_string();
    let data = if header.contains("base64") {
        crate::base64::decode_standard(data_part).ok()?
    } else {
        data_part.as_bytes().to_vec()
    };
    Some((mime, data))
}

fn flate_compress(data: &[u8]) -> Vec<u8> {
    crate::flate_native::zlib_deflate_parallel(data)
}

fn hash_bytes(data: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}

fn hash_image(image: &ImageData) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    image.width.hash(&mut hasher);
    image.height.hash(&mut hasher);
    image.color_space.hash(&mut hasher);
    image.bits_per_component.hash(&mut hasher);
    image.filter.hash(&mut hasher);
    image.decode.hash(&mut hasher);
    image.data.hash(&mut hasher);
    if let Some(alpha) = &image.alpha {
        alpha.data.hash(&mut hasher);
    }
    hasher.finish()
}

fn subset_font_name(font: &RegisteredFont, tag: Option<&[u8; 6]>) -> String {
    let base = sanitize_font_name(&font.name);
    let Some(tag) = tag else {
        return base;
    };
    let tag = std::str::from_utf8(tag).unwrap_or("AAAAAA");
    format!("{}+{}", tag, base)
}

fn truetype_font_object(font: &RegisteredFont, descriptor_id: usize, base_name: &str) -> String {
    let metrics = &font.metrics;
    let subtype = match font.program_kind {
        FontProgramKind::OpenTypeCff => "Type1",
        FontProgramKind::TrueType => "TrueType",
    };
    let widths = metrics
        .widths
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let encoding = if metrics.is_symbolic() {
        String::new()
    } else {
        " /Encoding /WinAnsiEncoding".to_string()
    };
    format!(
        "<< /Type /Font /Subtype /{} /BaseFont /{} /FirstChar {} /LastChar {} /Widths [{}] /FontDescriptor {} 0 R{} >>",
        subtype, base_name, metrics.first_char, metrics.last_char, widths, descriptor_id, encoding
    )
}

fn font_descriptor_object(font: &RegisteredFont, font_file_id: usize, base_name: &str) -> String {
    let metrics = &font.metrics;
    let mut flags = if metrics.is_symbolic() { 4 } else { 32 };
    if metrics.is_fixed_pitch {
        flags |= 1;
    }
    let font_file_entry = match font.program_kind {
        FontProgramKind::OpenTypeCff => "FontFile3",
        FontProgramKind::TrueType => "FontFile2",
    };
    format!(
        "<< /Type /FontDescriptor /FontName /{} /Flags {} /FontBBox [{} {} {} {}] /ItalicAngle {} /Ascent {} /Descent {} /CapHeight {} /StemV {} /MissingWidth {} /{} {} 0 R >>",
        base_name,
        flags,
        metrics.bbox.0,
        metrics.bbox.1,
        metrics.bbox.2,
        metrics.bbox.3,
        metrics.italic_angle,
        metrics.ascent,
        metrics.descent,
        metrics.cap_height,
        metrics.stem_v,
        metrics.missing_width,
        font_file_entry,
        font_file_id
    )
}

fn output_intent_object(oi: &OutputIntent, icc_id: usize, profile: PdfProfile) -> String {
    let subtype = profile.output_intent_subtype();
    let mut dict = format!(
        "<< /Type /OutputIntent /S /{} /DestOutputProfile {} 0 R /OutputConditionIdentifier ({}) /OutputCondition ({})",
        subtype,
        icc_id,
        escape_pdf_string(&oi.identifier),
        escape_pdf_string(&oi.identifier),
    );
    dict.push_str(" /RegistryName (http://www.color.org)");
    let info = oi.info.as_deref().unwrap_or(&oi.identifier);
    dict.push_str(&format!(" /Info ({})", escape_pdf_string(info)));
    dict.push_str(" >>");
    dict
}

fn font_object(name: &str) -> String {
    let base = sanitize_font_name(name);
    format!(
        "<< /Type /Font /Subtype /Type1 /BaseFont /{} /Encoding /WinAnsiEncoding >>",
        base
    )
}

fn font_resources(fonts: &[(String, usize)]) -> String {
    let mut entries = Vec::new();
    for (resource, font_id) in fonts {
        entries.push(format!("/{} {} 0 R", resource, font_id));
    }
    format!("<< {} >>", entries.join(" "))
}

fn xobject_resources(images: &[(String, usize)]) -> String {
    let mut entries = Vec::new();
    for (resource, image_id) in images {
        entries.push(format!("/{} {} 0 R", resource, image_id));
    }
    format!("<< {} >>", entries.join(" "))
}

fn extgstate_resources(states: &[(String, usize)]) -> String {
    let mut entries = Vec::new();
    for (resource, obj_id) in states {
        entries.push(format!("/{} {} 0 R", resource, obj_id));
    }
    format!("<< {} >>", entries.join(" "))
}

fn shading_resources(shadings: &[(String, usize)]) -> String {
    let mut entries = Vec::new();
    for (resource, obj_id) in shadings {
        entries.push(format!("/{} {} 0 R", resource, obj_id));
    }
    format!("<< {} >>", entries.join(" "))
}

fn optional_content_resources(entries: &[(String, usize)]) -> String {
    let mut out = Vec::new();
    for (resource, obj_id) in entries {
        out.push(format!("/{} {} 0 R", escape_pdf_name(resource), obj_id));
    }
    format!("<< {} >>", out.join(" "))
}

fn optional_content_group_object(name: &str) -> String {
    format!(
        "<< /Type /OCG /Name ({}) /Intent [/View /Design] /Usage << /View << /ViewState /ON >> /Print << /PrintState /ON >> >> >>",
        escape_pdf_string(name)
    )
}

fn ocproperties_dict(ocg_ids: &[usize]) -> String {
    if ocg_ids.is_empty() {
        return String::new();
    }
    let refs = ocg_ids
        .iter()
        .map(|id| format!("{} 0 R", id))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "<< /OCGs [{}] /D << /Order [{}] /ON [{}] /AS [<< /Event /View /Category [/View] /OCGs [{}] >> << /Event /Print /Category [/Print] /OCGs [{}] >>] >> >>",
        refs, refs, refs, refs, refs
    )
}

fn sanitize_font_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            out.push(ch);
        } else if ch == ' ' {
            out.push('-');
        }
    }
    if out.is_empty() {
        "Helvetica".to_string()
    } else {
        out
    }
}

#[allow(dead_code)]
fn render_page(
    page: &Page,
    page_height: Pt,
    font_map: &BTreeMap<String, FontResource>,
    font_glyph_maps: &HashMap<String, BTreeMap<u16, String>>,
    image_map: &HashMap<String, String>,
    gs_map: &HashMap<(u16, u16), String>,
    gs_blend_map: &HashMap<MixBlendMode, String>,
    shading_map: &HashMap<u64, String>,
    registry: Option<&FontRegistry>,
    tj_cache: &mut HashMap<String, String>,
    options: &PdfOptions,
    page_index: usize,
    mut tag_records: Option<&mut Vec<TagRecord>>,
) -> String {
    let mut out = String::new();
    let mut current_font_size = Pt::from_f32(12.0);
    let mut current_font_name = "Helvetica".to_string();
    let mut current_fill = Color::BLACK;
    let mut graphics_state_stack: Vec<(Pt, String, Color)> = Vec::new();
    let mut tag_stack: Vec<usize> = Vec::new();

    for cmd in &page.commands {
        match cmd {
            Command::SaveState => {
                graphics_state_stack.push((
                    current_font_size,
                    current_font_name.clone(),
                    current_fill,
                ));
                out.push_str("q\n");
            }
            Command::RestoreState => {
                if let Some((font_size, font_name, fill)) = graphics_state_stack.pop() {
                    current_font_size = font_size;
                    current_font_name = font_name;
                    current_fill = fill;
                }
                out.push_str("Q\n");
            }
            Command::Translate(x, y) => {
                out.push_str(&format!("1 0 0 1 {} {} cm\n", fmt_pt(*x), fmt_pt(-*y)));
            }
            Command::CssTransformOrigin { x, y, inverse } => {
                let pdf_y = page_height - *y;
                let (tx, ty) = if *inverse { (-*x, -pdf_y) } else { (*x, pdf_y) };
                out.push_str(&format!("1 0 0 1 {} {} cm\n", fmt_pt(tx), fmt_pt(ty)));
            }
            Command::Scale(x, y) => {
                out.push_str(&format!("{} 0 0 {} 0 0 cm\n", fmt(*x), fmt(*y)));
            }
            Command::Rotate(angle) => {
                let (sin, cos) = crate::math::sin_cos(-*angle);
                out.push_str(&format!(
                    "{} {} {} {} 0 0 cm\n",
                    fmt(cos),
                    fmt(sin),
                    fmt(-sin),
                    fmt(cos)
                ));
            }
            Command::ConcatMatrix { a, b, c, d, e, f } => {
                out.push_str(&format!(
                    "{} {} {} {} {} {} cm\n",
                    fmt(*a),
                    fmt(-*b),
                    fmt(-*c),
                    fmt(*d),
                    fmt_pt(*e),
                    fmt_pt(-*f)
                ));
            }
            Command::Meta { .. } => {}
            Command::BeginTag {
                role,
                mcid,
                alt,
                scope,
                table_id,
                col_index,
                group_only,
            } => {
                if options.pdf_profile.emits_tagged_structure() {
                    let role_raw = role.clone();
                    let role = escape_pdf_name(role);
                    if *group_only {
                        out.push_str(&format!("/{role} BMC\n"));
                    } else if let Some(mcid) = mcid {
                        out.push_str(&format!("/{role} <</MCID {}>> BDC\n", mcid));
                    }
                    if let Some(records) = tag_records.as_deref_mut() {
                        let parent = tag_stack.last().copied();
                        let idx = records.len();
                        records.push(TagRecord {
                            page_index,
                            mcid: *mcid,
                            role: role_raw,
                            alt: alt.clone(),
                            scope: scope.clone(),
                            parent,
                            table_id: *table_id,
                            col_index: *col_index,
                        });
                        tag_stack.push(idx);
                    }
                }
            }
            Command::EndTag => {
                if options.pdf_profile.emits_tagged_structure() {
                    out.push_str("EMC\n");
                    let _ = tag_stack.pop();
                }
            }
            Command::BeginArtifact { subtype } => {
                if let Some(subtype) = subtype.as_deref() {
                    out.push_str(&format!(
                        "/Artifact <</Subtype /{}>> BDC\n",
                        escape_pdf_name(subtype)
                    ));
                } else {
                    out.push_str("/Artifact BMC\n");
                }
            }
            Command::BeginOptionalContent { name } => {
                out.push_str(&format!("/OC /{} BDC\n", escape_pdf_name(name)));
            }
            Command::EndMarkedContent => {
                out.push_str("EMC\n");
            }
            Command::SetFillColor(color) => {
                current_fill = *color;
                out.push_str(&color_to_pdf_fill(*color, options.color_space));
            }
            Command::SetStrokeColor(color) => {
                out.push_str(&color_to_pdf_stroke(*color, options.color_space));
            }
            Command::SetLineWidth(width) => {
                out.push_str(&format!("{} w\n", fmt_pt(*width)));
            }
            Command::SetLineCap(cap) => {
                out.push_str(&format!("{} J\n", cap));
            }
            Command::SetLineJoin(join) => {
                out.push_str(&format!("{} j\n", join));
            }
            Command::SetMiterLimit(limit) => {
                out.push_str(&format!("{} M\n", fmt_pt(*limit)));
            }
            Command::SetDash { pattern, phase } => {
                let pat = if pattern.is_empty() {
                    "[]".to_string()
                } else {
                    let items = pattern
                        .iter()
                        .map(|v| fmt_pt(*v))
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!("[{}]", items)
                };
                out.push_str(&format!("{} {} d\n", pat, fmt_pt(*phase)));
            }
            Command::SetOpacity { fill, stroke } => {
                // Map opacity to an ExtGState resource. We quantize to 0..1000 in build_extgstate_objects.
                let k = ((*fill * 1000.0).round() as i32).clamp(0, 1000) as u16;
                let ks = ((*stroke * 1000.0).round() as i32).clamp(0, 1000) as u16;
                if let Some(name) = gs_map.get(&(k, ks)) {
                    out.push_str(&format!("/{} gs\n", name));
                }
            }
            Command::SetBlendMode { mode } => {
                if let Some(name) = gs_blend_map.get(mode) {
                    out.push_str(&format!("/{} gs\n", name));
                }
            }
            Command::ApplyBackdropFilter { .. } => {}
            Command::SetFontName(name) => {
                current_font_name = name.clone();
            }
            Command::SetFontSize(size) => {
                current_font_size = *size;
            }
            Command::SetTextRenderingMode(mode) => {
                out.push_str(&format!("{} Tr\n", (*mode).min(7)));
            }
            Command::ClipRect {
                x,
                y,
                width,
                height,
            } => {
                // Define a rectangular clipping path and apply it.
                // Coordinates are in our top-left-origin space; PDF uses bottom-left-origin.
                out.push_str(&format!(
                    "{} {} {} {} re\nW\nn\n",
                    fmt_pt(*x),
                    fmt_pt(page_height - *y - *height),
                    fmt_pt(*width),
                    fmt_pt(*height)
                ));
            }
            Command::ClipPath { evenodd } => {
                if *evenodd {
                    out.push_str("W*\n");
                } else {
                    out.push_str("W\n");
                }
                out.push_str("n\n");
            }
            Command::ShadingFill(shading) => {
                let key = hash_shading(shading);
                if let Some(name) = shading_map.get(&key) {
                    out.push_str(&format!("/{} sh\n", name));
                }
            }
            Command::MoveTo { x, y } => {
                out.push_str(&format!("{} {} m\n", fmt_pt(*x), fmt_pt(page_height - *y)));
            }
            Command::LineTo { x, y } => {
                out.push_str(&format!("{} {} l\n", fmt_pt(*x), fmt_pt(page_height - *y)));
            }
            Command::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                out.push_str(&format!(
                    "{} {} {} {} {} {} c\n",
                    fmt_pt(*x1),
                    fmt_pt(page_height - *y1),
                    fmt_pt(*x2),
                    fmt_pt(page_height - *y2),
                    fmt_pt(*x),
                    fmt_pt(page_height - *y),
                ));
            }
            Command::ClosePath => out.push_str("h\n"),
            Command::Fill => out.push_str("f\n"),
            Command::FillEvenOdd => out.push_str("f*\n"),
            Command::Stroke => out.push_str("S\n"),
            Command::FillStroke => out.push_str("B\n"),
            Command::FillStrokeEvenOdd => out.push_str("B*\n"),
            Command::DrawString { x, y, text } => {
                out.push_str("BT\n");
                let font_res = font_map.get(&current_font_name);
                let resource = font_res.map(|v| v.resource.as_str()).unwrap_or("F1");
                out.push_str(&format!("/{} {} Tf\n", resource, fmt_pt(current_font_size)));
                out.push_str(&format!(
                    "{} {} Td\n",
                    fmt_pt(*x),
                    fmt_pt(page_height - *y - current_font_size)
                ));
                match font_res
                    .map(|v| v.encoding)
                    .unwrap_or(FontEncoding::WinAnsi)
                {
                    FontEncoding::WinAnsi => {
                        let encoded = encode_winansi_pdf_string(text);
                        out.push_str(&format!("({}) Tj\n", encoded.text));
                    }
                    FontEncoding::IdentityH => {
                        if let Some(registry) = registry {
                            if let Some(tj) = cached_shape_text_to_tj(
                                registry,
                                &current_font_name,
                                current_font_size,
                                text,
                                tj_cache,
                                options,
                            ) {
                                out.push_str(&tj);
                                out.push_str("ET\n");
                                continue;
                            }
                        }
                        let cmap = font_glyph_maps.get(&current_font_name);
                        let hex = encode_cid_hex(text, cmap);
                        out.push_str(&format!("{} Tj\n", hex));
                    }
                }
                out.push_str("ET\n");
            }
            Command::DrawStringTransformed {
                x,
                y,
                text,
                m00,
                m01,
                m10,
                m11,
            } => {
                out.push_str("BT\n");
                let font_res = font_map.get(&current_font_name);
                let resource = font_res
                    .map(|value| value.resource.as_str())
                    .unwrap_or("F1");
                out.push_str(&format!("/{} {} Tf\n", resource, fmt_pt(current_font_size)));
                out.push_str(&format!(
                    "{} {} {} {} {} {} Tm\n",
                    fmt(*m00),
                    fmt(*m01),
                    fmt(*m10),
                    fmt(*m11),
                    fmt_pt(*x),
                    fmt_pt(*y)
                ));
                match font_res
                    .map(|value| value.encoding)
                    .unwrap_or(FontEncoding::WinAnsi)
                {
                    FontEncoding::WinAnsi => {
                        let encoded = encode_winansi_pdf_string(text);
                        out.push_str(&format!("({}) Tj\n", encoded.text));
                    }
                    FontEncoding::IdentityH => {
                        if let Some(registry) = registry {
                            if let Some(tj) = cached_shape_text_to_tj(
                                registry,
                                &current_font_name,
                                current_font_size,
                                text,
                                tj_cache,
                                options,
                            ) {
                                out.push_str(&tj);
                                out.push_str("ET\n");
                                continue;
                            }
                        }
                        let cmap = font_glyph_maps.get(&current_font_name);
                        let hex = encode_cid_hex(text, cmap);
                        out.push_str(&format!("{} Tj\n", hex));
                    }
                }
                out.push_str("ET\n");
            }
            Command::DrawGlyphRun { .. } | Command::DrawSyntheticBoldGlyphRun { .. } => {
                // Raster-only command; PDF writer does not emit glyph runs directly.
            }
            Command::DrawRect {
                x,
                y,
                width,
                height,
            } => {
                out.push_str(&format!(
                    "{} {} {} {} re\nf\n",
                    fmt_pt(*x),
                    fmt_pt(page_height - *y - *height),
                    fmt_pt(*width),
                    fmt_pt(*height)
                ));
            }
            Command::DrawImage {
                x,
                y,
                width,
                height,
                resource_id,
                ..
            } => {
                if let Some(name) = image_map.get(resource_id) {
                    let draw_y = page_height - *y - *height;
                    out.push_str("q\n");
                    out.push_str(&format!(
                        "{} 0 0 {} {} {} cm\n",
                        fmt_pt(*width),
                        fmt_pt(*height),
                        fmt_pt(*x),
                        fmt_pt(draw_y)
                    ));
                    out.push_str(&format!("/{} Do\n", name));
                    out.push_str("Q\n");
                } else {
                    out.push_str(&color_to_pdf_fill(current_fill, options.color_space));
                }
            }
            Command::DefineForm { .. } => {}
            Command::DefineIsolatedForm { .. } => {}
            Command::DrawForm { .. } => {}
            Command::DrawFilteredForm { .. } => {}
            Command::DrawMaskedForm { .. } => {}
        }
    }

    out
}

fn hash_shading(shading: &Shading) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();

    fn hash_f32(hasher: &mut std::collections::hash_map::DefaultHasher, v: f32) {
        v.to_bits().hash(hasher);
    }
    fn hash_color(hasher: &mut std::collections::hash_map::DefaultHasher, c: Color) {
        hash_f32(hasher, c.r);
        hash_f32(hasher, c.g);
        hash_f32(hasher, c.b);
    }
    fn hash_stops(hasher: &mut std::collections::hash_map::DefaultHasher, stops: &[ShadingStop]) {
        stops.len().hash(hasher);
        for s in stops {
            hash_f32(hasher, s.offset);
            hash_color(hasher, s.color);
            hash_f32(hasher, s.alpha);
        }
    }

    match shading {
        Shading::Axial {
            x0,
            y0,
            x1,
            y1,
            stops,
        } => {
            1u8.hash(&mut hasher);
            hash_f32(&mut hasher, *x0);
            hash_f32(&mut hasher, *y0);
            hash_f32(&mut hasher, *x1);
            hash_f32(&mut hasher, *y1);
            hash_stops(&mut hasher, stops);
        }
        Shading::Radial {
            x0,
            y0,
            r0,
            x1,
            y1,
            r1,
            stops,
            hard_stops,
        } => {
            2u8.hash(&mut hasher);
            hard_stops.hash(&mut hasher);
            hash_f32(&mut hasher, *x0);
            hash_f32(&mut hasher, *y0);
            hash_f32(&mut hasher, *r0);
            hash_f32(&mut hasher, *x1);
            hash_f32(&mut hasher, *y1);
            hash_f32(&mut hasher, *r1);
            hash_stops(&mut hasher, stops);
        }
        Shading::Conic {
            center_x,
            center_y,
            radius,
            start_angle_deg,
            stops,
            hard_stops,
        } => {
            3u8.hash(&mut hasher);
            hard_stops.hash(&mut hasher);
            hash_f32(&mut hasher, *center_x);
            hash_f32(&mut hasher, *center_y);
            hash_f32(&mut hasher, *radius);
            hash_f32(&mut hasher, *start_angle_deg);
            hash_stops(&mut hasher, stops);
        }
    }
    hasher.finish()
}

fn hash_shading_at_height(shading: &Shading, page_height: Pt) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hash_shading(shading).hash(&mut hasher);
    page_height.to_milli_i64().hash(&mut hasher);
    hasher.finish()
}

fn vector_alpha_mask_commands(commands: &[Command]) -> Option<Vec<Command>> {
    let mut output = Vec::with_capacity(commands.len());
    let mut saw_vector_shader = false;
    let hard_axial_shader_count = commands
        .iter()
        .filter(|command| {
            matches!(
                command,
                Command::ShadingFill(shading @ Shading::Axial { .. })
                    if shading_has_alpha_discontinuity(shading)
            )
        })
        .count();
    for command in commands {
        match command {
            Command::ShadingFill(
                shading @ (Shading::Axial { .. } | Shading::Radial { .. } | Shading::Conic { .. }),
            ) => {
                // PDF axial stitching functions preserve coincident stops as
                // an exact vector half-plane. Radial and conic discontinuities
                // still use the cached raster path because their native PDF
                // approximations cannot represent every CSS hard-stop shape.
                if !matches!(shading, Shading::Axial { .. })
                    && shading_has_alpha_discontinuity(shading)
                {
                    return None;
                }
                if matches!(shading, Shading::Axial { .. })
                    && shading_has_alpha_discontinuity(shading)
                    && (hard_axial_shader_count != 1
                        || shading_alpha_discontinuity_count(shading) != 1)
                {
                    return None;
                }
                let stops = shading_stops(shading)
                    .iter()
                    .map(|stop| {
                        let coverage = stop.alpha.clamp(0.0, 1.0);
                        ShadingStop {
                            offset: stop.offset,
                            color: Color::rgb(coverage, coverage, coverage),
                            alpha: 1.0,
                        }
                    })
                    .collect();
                let mut compiled = with_shading_stops(shading, stops);
                if matches!(shading, Shading::Axial { .. })
                    && shading_has_alpha_discontinuity(shading)
                {
                    compiled = phase_hard_axial_mask_shading(compiled);
                }
                output.push(Command::ShadingFill(compiled));
                saw_vector_shader = true;
            }
            Command::SaveState
            | Command::RestoreState
            | Command::Translate(_, _)
            | Command::CssTransformOrigin { .. }
            | Command::Scale(_, _)
            | Command::Rotate(_)
            | Command::ConcatMatrix { .. }
            | Command::Meta { .. }
            | Command::SetFillColor(_)
            | Command::SetStrokeColor(_)
            | Command::SetLineWidth(_)
            | Command::SetLineCap(_)
            | Command::SetLineJoin(_)
            | Command::SetMiterLimit(_)
            | Command::SetDash { .. }
            | Command::ClipRect { .. }
            | Command::ClipPath { .. }
            | Command::MoveTo { .. }
            | Command::LineTo { .. }
            | Command::CurveTo { .. }
            | Command::ClosePath => output.push(command.clone()),
            Command::SetOpacity { fill, stroke }
                if *fill >= 1.0 - f32::EPSILON && *stroke >= 1.0 - f32::EPSILON => {}
            Command::SetBlendMode {
                mode: MixBlendMode::Normal,
            } => {}
            _ => return None,
        }
    }
    saw_vector_shader.then_some(output)
}

fn phase_hard_axial_mask_shading(shading: Shading) -> Shading {
    let Shading::Axial {
        x0,
        y0,
        x1,
        y1,
        stops,
    } = shading
    else {
        return shading;
    };
    let dx = x1 - x0;
    let dy = y1 - y0;
    let length = dx.hypot(dy);
    if length <= 1.0e-6 {
        return Shading::Axial {
            x0,
            y0,
            x1,
            y1,
            stops,
        };
    }

    // Chrome's device-space Type 1 mask shader retains the hard-stop pixel
    // on the opaque side. A PDF Type 2 stitching function samples the same
    // coincident stop one print-device pixel earlier, so advance the compiled
    // axis by one virtual print pixel while keeping the shader fully vector.
    let phase = 72.0 / PDF_FILTER_RASTER_DPI as f32;
    let phase_x = dx / length * phase;
    let phase_y = dy / length * phase;
    Shading::Axial {
        x0: x0 + phase_x,
        y0: y0 + phase_y,
        x1: x1 + phase_x,
        y1: y1 + phase_y,
        stops,
    }
}

fn shading_has_alpha_discontinuity(shading: &Shading) -> bool {
    shading_alpha_discontinuity_count(shading) != 0
}

fn shading_alpha_discontinuity_count(shading: &Shading) -> usize {
    shading_stops(shading)
        .windows(2)
        .filter(|pair| {
            (pair[1].offset - pair[0].offset).abs() <= 1.0e-6
                && (pair[1].alpha - pair[0].alpha).abs() > 1.0e-6
        })
        .count()
}

fn commands_have_alpha_discontinuity(commands: &[Command]) -> bool {
    commands.iter().any(|command| {
        matches!(command, Command::ShadingFill(shading) if shading_has_alpha_discontinuity(shading))
    })
}

fn shading_stops(shading: &Shading) -> &[ShadingStop] {
    match shading {
        Shading::Axial { stops, .. }
        | Shading::Radial { stops, .. }
        | Shading::Conic { stops, .. } => stops,
    }
}

fn with_shading_stops(shading: &Shading, stops: Vec<ShadingStop>) -> Shading {
    match shading {
        Shading::Axial { x0, y0, x1, y1, .. } => Shading::Axial {
            x0: *x0,
            y0: *y0,
            x1: *x1,
            y1: *y1,
            stops,
        },
        Shading::Radial {
            x0,
            y0,
            r0,
            x1,
            y1,
            r1,
            hard_stops,
            ..
        } => Shading::Radial {
            x0: *x0,
            y0: *y0,
            r0: *r0,
            x1: *x1,
            y1: *y1,
            r1: *r1,
            stops,
            hard_stops: *hard_stops,
        },
        Shading::Conic {
            center_x,
            center_y,
            radius,
            start_angle_deg,
            hard_stops,
            ..
        } => Shading::Conic {
            center_x: *center_x,
            center_y: *center_y,
            radius: *radius,
            start_angle_deg: *start_angle_deg,
            stops,
            hard_stops: *hard_stops,
        },
    }
}

fn sample_shading_stop(stops: &[ShadingStop], position: f32) -> ShadingStop {
    let position = position.clamp(0.0, 1.0);
    let Some(first) = stops.first().copied() else {
        return ShadingStop {
            offset: position,
            color: Color::BLACK,
            alpha: 0.0,
        };
    };
    if position <= first.offset {
        return ShadingStop {
            offset: position,
            ..first
        };
    }
    for pair in stops.windows(2) {
        if position > pair[1].offset {
            continue;
        }
        let span = pair[1].offset - pair[0].offset;
        let amount = if span <= 1.0e-6 {
            1.0
        } else {
            ((position - pair[0].offset) / span).clamp(0.0, 1.0)
        };
        return ShadingStop {
            offset: position,
            color: Color::rgb(
                pair[0].color.r + (pair[1].color.r - pair[0].color.r) * amount,
                pair[0].color.g + (pair[1].color.g - pair[0].color.g) * amount,
                pair[0].color.b + (pair[1].color.b - pair[0].color.b) * amount,
            ),
            alpha: pair[0].alpha + (pair[1].alpha - pair[0].alpha) * amount,
        };
    }
    ShadingStop {
        offset: position,
        ..stops[stops.len() - 1]
    }
}

fn alpha_only_shading(shading: &Shading) -> Shading {
    let stops = shading_stops(shading)
        .iter()
        .map(|stop| {
            let alpha = stop.alpha.clamp(0.0, 1.0);
            ShadingStop {
                offset: stop.offset,
                color: Color::rgb(alpha, alpha, alpha),
                alpha: 1.0,
            }
        })
        .collect();
    with_shading_stops(shading, stops)
}

fn premultiplied_color_shading(shading: &Shading) -> Shading {
    // CSS interpolates translucent stops in premultiplied colour space. A PDF
    // shading interpolates colour and the soft-mask alpha independently, so
    // sample the unpremultiplied colour curve finely enough to keep 8-bit
    // raster output within one channel value of the authored gradient.
    const SAMPLES_PER_SEGMENT: usize = 128;

    fn sample(a: ShadingStop, b: ShadingStop, t: f32) -> ShadingStop {
        let alpha_a = a.alpha.clamp(0.0, 1.0);
        let alpha_b = b.alpha.clamp(0.0, 1.0);
        let alpha = alpha_a + (alpha_b - alpha_a) * t;
        let channel = |ca: f32, cb: f32| {
            if alpha <= f32::EPSILON {
                0.0
            } else {
                (ca * alpha_a + (cb * alpha_b - ca * alpha_a) * t) / alpha
            }
        };
        ShadingStop {
            offset: a.offset + (b.offset - a.offset) * t,
            color: Color::rgb(
                channel(a.color.r, b.color.r),
                channel(a.color.g, b.color.g),
                channel(a.color.b, b.color.b),
            ),
            alpha: 1.0,
        }
    }

    let source = shading_stops(shading);
    if source.len() < 2 {
        return with_shading_stops(
            shading,
            source
                .iter()
                .map(|stop| ShadingStop {
                    alpha: 1.0,
                    ..*stop
                })
                .collect(),
        );
    }

    let mut stops = Vec::with_capacity((source.len() - 1) * SAMPLES_PER_SEGMENT + 1);
    stops.push(sample(source[0], source[1], 0.0));
    for pair in source.windows(2) {
        let a = pair[0];
        let b = pair[1];
        if (b.offset - a.offset).abs() <= f32::EPSILON {
            stops.push(sample(a, b, 1.0));
            continue;
        }
        for step in 1..=SAMPLES_PER_SEGMENT {
            stops.push(sample(a, b, step as f32 / SAMPLES_PER_SEGMENT as f32));
        }
    }
    with_shading_stops(shading, stops)
}

fn shading_to_objects(
    shading: &Shading,
    start_id: usize,
    page_height: Pt,
    color_space: ColorSpace,
) -> (Vec<String>, usize, usize) {
    // Returns (objects, shading_obj_id, next_id).
    // We emit the /Function objects first, then the shading dict.
    let mut objects: Vec<String> = Vec::new();
    let mut next_id = start_id;

    let stops = match shading {
        Shading::Axial { stops, .. } => stops.clone(),
        Shading::Radial { stops, .. } => stops.clone(),
        Shading::Conic { stops, .. } => stops.clone(),
    };

    let hard_stops = matches!(
        shading,
        Shading::Radial {
            hard_stops: true,
            ..
        }
    );
    let (fun_objects, fun_id, new_next) = if hard_stops {
        build_hard_gradient_function_object(&stops, next_id, color_space)
    } else {
        build_gradient_function_objects(&stops, next_id, color_space)
    };
    objects.extend(fun_objects);
    next_id = new_next;

    let sh_obj_id = next_id;
    next_id += 1;

    let space = match color_space {
        ColorSpace::Rgb => "/DeviceRGB",
        ColorSpace::Cmyk => "/DeviceCMYK",
    };
    let sh_dict = match shading {
        Shading::Axial { x0, y0, x1, y1, .. } => format!(
            "<< /ShadingType 2 /ColorSpace {} /Coords [{} {} {} {}] /Function {} 0 R /Extend [true true] >>",
            space,
            fmt(*x0),
            fmt(page_height.to_f32() - *y0),
            fmt(*x1),
            fmt(page_height.to_f32() - *y1),
            fun_id,
        ),
        Shading::Radial {
            x0,
            y0,
            r1,
            hard_stops: true,
            ..
        } => format!(
            "<< /ShadingType 1 /ColorSpace {} /Domain [-1000 1000 -1000 1000] /Matrix [{} 0 0 -{} {} {}] /Function {} 0 R >>",
            space,
            fmt(*r1),
            fmt(*r1),
            fmt(*x0),
            fmt(page_height.to_f32() - *y0),
            fun_id,
        ),
        Shading::Radial {
            x0,
            y0,
            r0,
            x1,
            y1,
            r1,
            ..
        } => format!(
            "<< /ShadingType 3 /ColorSpace {} /Coords [{} {} {} {} {} {}] /Function {} 0 R /Extend [true true] >>",
            space,
            fmt(*x0),
            fmt(page_height.to_f32() - *y0),
            fmt(*r0),
            fmt(*x1),
            fmt(page_height.to_f32() - *y1),
            fmt(*r1),
            fun_id,
        ),
        // Conic shaders are lowered directly into the content stream by the
        // streaming writer. This fallback keeps the legacy object collector
        // total without making it part of the active conic path.
        Shading::Conic {
            center_x,
            center_y,
            radius,
            ..
        } => format!(
            "<< /ShadingType 2 /ColorSpace {} /Coords [{} {} {} {}] /Function {} 0 R /Extend [true true] >>",
            space,
            fmt(*center_x),
            fmt(page_height.to_f32() - *center_y),
            fmt(*center_x),
            fmt(page_height.to_f32() - *center_y + *radius),
            fun_id,
        ),
    };
    objects.push(sh_dict);

    (objects, sh_obj_id, next_id)
}

fn build_hard_gradient_function_object(
    stops: &[ShadingStop],
    start_id: usize,
    color_space: ColorSpace,
) -> (Vec<String>, usize, usize) {
    let mut stops = stops.to_vec();
    stops.sort_by(|a, b| {
        a.offset
            .partial_cmp(&b.offset)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut bands = Vec::new();
    for pair in stops.windows(2) {
        if pair[1].offset - pair[0].offset <= 1.0e-6 {
            continue;
        }
        bands.push((pair[1].offset.clamp(0.0, 1.0), pair[0].color));
    }
    if bands.is_empty() {
        return build_gradient_function_objects(stops.as_slice(), start_id, color_space);
    }

    let components = |color: Color| {
        color_components(color, color_space)
            .iter()
            .map(|value| fmt(*value))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut body = format!("pop {}", components(bands[bands.len() - 1].1));
    for (end, color) in bands.iter().copied().take(bands.len() - 1).rev() {
        body = format!(
            "dup {} le {{ pop {} }} {{ {} }} ifelse",
            fmt(end),
            components(color),
            body
        );
    }
    let program = format!("{{ dup mul exch dup mul add sqrt {} }}", body);
    let component_count = color_components(Color::BLACK, color_space).len();
    let range = (0..component_count)
        .map(|_| "0 1")
        .collect::<Vec<_>>()
        .join(" ");
    let object = format!(
        "<< /FunctionType 4 /Domain [-1000 1000 -1000 1000] /Range [{}] /Length {} >>\nstream\n{}\nendstream",
        range,
        program.as_bytes().len(),
        program
    );
    (vec![object], start_id, start_id + 1)
}

fn build_gradient_function_objects(
    stops: &[ShadingStop],
    start_id: usize,
    color_space: ColorSpace,
) -> (Vec<String>, usize, usize) {
    // Build a single function object id that maps t in [0,1] to RGB.
    // For 0/1 stops: emit a constant-ish Type 2 function.
    // For N stops: emit N-1 Type 2 functions stitched with a Type 3 function.
    let mut stops = stops.to_vec();
    if stops.is_empty() {
        stops.push(ShadingStop {
            offset: 0.0,
            color: Color::BLACK,
            alpha: 1.0,
        });
        stops.push(ShadingStop {
            offset: 1.0,
            color: Color::BLACK,
            alpha: 1.0,
        });
    } else if stops.len() == 1 {
        stops.push(ShadingStop {
            offset: 1.0,
            color: stops[0].color,
            alpha: stops[0].alpha,
        });
    }

    // Normalize + sort.
    for s in &mut stops {
        s.offset = s.offset.clamp(0.0, 1.0);
    }
    stops.sort_by(|a, b| {
        a.offset
            .partial_cmp(&b.offset)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Ensure first/last are 0/1.
    if stops[0].offset > 0.0 {
        stops.insert(
            0,
            ShadingStop {
                offset: 0.0,
                color: stops[0].color,
                alpha: stops[0].alpha,
            },
        );
    }
    if stops[stops.len() - 1].offset < 1.0 {
        let last = stops[stops.len() - 1];
        stops.push(ShadingStop {
            offset: 1.0,
            color: last.color,
            alpha: last.alpha,
        });
    }

    let mut objects: Vec<String> = Vec::new();
    let mut next_id = start_id;

    let mut seg_fun_ids: Vec<usize> = Vec::new();
    for i in 0..(stops.len() - 1) {
        let f_id = next_id;
        next_id += 1;
        seg_fun_ids.push(f_id);
        let c0 = stops[i].color;
        let c1 = stops[i + 1].color;
        let c0_vals = color_components(c0, color_space);
        let c1_vals = color_components(c1, color_space);
        let c0_str = c0_vals
            .iter()
            .map(|v| fmt(*v))
            .collect::<Vec<_>>()
            .join(" ");
        let c1_str = c1_vals
            .iter()
            .map(|v| fmt(*v))
            .collect::<Vec<_>>()
            .join(" ");
        objects.push(format!(
            "<< /FunctionType 2 /Domain [0 1] /C0 [{}] /C1 [{}] /N 1 >>",
            c0_str, c1_str,
        ));
    }

    if seg_fun_ids.len() == 1 {
        return (objects, seg_fun_ids[0], next_id);
    }

    let stitch_id = next_id;
    next_id += 1;
    let mut bounds: Vec<String> = Vec::new();
    for s in stops.iter().skip(1).take(stops.len() - 2) {
        bounds.push(fmt(s.offset));
    }
    let mut encode: Vec<String> = Vec::new();
    for _ in 0..seg_fun_ids.len() {
        encode.push("0".to_string());
        encode.push("1".to_string());
    }
    let fun_refs = seg_fun_ids
        .iter()
        .map(|id| format!("{} 0 R", id))
        .collect::<Vec<_>>()
        .join(" ");

    objects.push(format!(
        "<< /FunctionType 3 /Domain [0 1] /Functions [{}] /Bounds [{}] /Encode [{}] >>",
        fun_refs,
        bounds.join(" "),
        encode.join(" "),
    ));

    (objects, stitch_id, next_id)
}

fn metadata_stream_object(content: &str) -> String {
    let length = content.as_bytes().len();
    format!(
        "<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n{}\nendstream",
        length, content
    )
}

fn pdfa4f_seed_embedded_file_stream_object() -> String {
    const CONTENT: &str = "FullBleed deterministic PDF/A-4f associated file seed.";
    format!(
        "<< /Type /EmbeddedFile /Subtype /text#2Fplain /Params << /Size {} /CreationDate (D:19700101000000Z) /ModDate (D:19700101000000Z) >> /Length {} >>\nstream\n{}\nendstream",
        CONTENT.as_bytes().len(),
        CONTENT.as_bytes().len(),
        CONTENT
    )
}

fn pdfa4f_seed_file_spec_object(embedded_file_id: usize) -> String {
    "<< /Type /Filespec /F (fullbleed-pdfa4f-seed.txt) /UF (fullbleed-pdfa4f-seed.txt) /Desc (FullBleed PDF/A-4f associated file seed) /AFRelationship /Data /EF << /F {id} 0 R /UF {id} 0 R >> >>"
        .replace("{id}", &embedded_file_id.to_string())
}

fn pdfa4f_seed_names_object(file_spec_id: usize) -> String {
    format!(
        "<< /Names [(fullbleed-pdfa4f-seed.txt) {} 0 R] >>",
        file_spec_id
    )
}

fn deterministic_file_id(
    profile: PdfProfile,
    version: PdfVersion,
    lang: Option<&str>,
    title: Option<&str>,
    page_count: usize,
    object_count: usize,
    xref_start: usize,
) -> String {
    let version = match version {
        PdfVersion::Pdf17 => "1.7",
        PdfVersion::Pdf20 => "2.0",
    };
    let mut state = 0xcbf29ce484222325u64;
    fn mix(state: &mut u64, bytes: &[u8]) {
        for byte in bytes {
            *state ^= u64::from(*byte);
            *state = state.wrapping_mul(0x100000001b3);
        }
        *state ^= 0xff;
        *state = state.wrapping_mul(0x100000001b3);
    }
    mix(&mut state, b"fullbleed-pdf-id-v1");
    mix(&mut state, profile.as_str().as_bytes());
    mix(&mut state, version.as_bytes());
    if let Some(lang) = lang {
        mix(&mut state, lang.as_bytes());
    }
    if let Some(title) = title {
        mix(&mut state, title.as_bytes());
    }
    mix(&mut state, page_count.to_string().as_bytes());
    mix(&mut state, object_count.to_string().as_bytes());
    mix(&mut state, xref_start.to_string().as_bytes());
    let first = state;
    mix(&mut state, b"secondary");
    let second = state;
    format!("{:016X}{:016X}", first, second)
}

fn page_box_entries(profile: PdfProfile, geometry: PageGeometry) -> String {
    let extent = geometry.presentation.media_extent();
    let has_sheet_area = extent > Pt::ZERO;
    if !profile.uses_pdfx_page_boxes() && !has_sheet_area {
        return String::new();
    }
    let trim_left = if has_sheet_area { extent } else { Pt::ZERO };
    let trim_bottom = trim_left;
    let trim_right = geometry.media_size.width - trim_left;
    let trim_top = geometry.media_size.height - trim_bottom;
    format!(
        " /TrimBox [{} {} {} {}] /BleedBox [0 0 {} {}] /CropBox [0 0 {} {}]",
        fmt_pt(trim_left),
        fmt_pt(trim_bottom),
        fmt_pt(trim_right),
        fmt_pt(trim_top),
        fmt_pt(geometry.media_size.width),
        fmt_pt(geometry.media_size.height),
        fmt_pt(geometry.media_size.width),
        fmt_pt(geometry.media_size.height),
    )
}

fn info_object(title: Option<&str>, profile: PdfProfile) -> String {
    let mut entries: Vec<String> = Vec::new();
    if let Some(title) = title {
        entries.push(format!("/Title ({})", escape_pdf_string(title)));
    }
    if matches!(profile, PdfProfile::PdfX4 | PdfProfile::PdfVt1) {
        entries.push("/GTS_PDFXVersion (PDF/X-4)".to_string());
        entries.push("/Trapped /False".to_string());
    }
    if profile == PdfProfile::PdfVt1 {
        entries.push("/GTS_PDFVTVersion (PDF/VT-1)".to_string());
    }
    if entries.is_empty() {
        entries.push("/Producer (FullBleed)".to_string());
    }
    format!("<< {} >>", entries.join(" "))
}

#[allow(dead_code)]
fn build_pdf(
    objects: Vec<String>,
    catalog_id: usize,
    info_id: Option<usize>,
    version: PdfVersion,
) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(pdf_header_bytes(version));
    out.extend_from_slice(b"%\xE2\xE3\xCF\xD3\n");

    let mut offsets = Vec::new();
    for (index, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        let obj_id = index + 1;
        out.extend_from_slice(format!("{} 0 obj\n", obj_id).as_bytes());
        out.extend_from_slice(obj.as_bytes());
        out.extend_from_slice(b"\nendobj\n");
    }

    let xref_start = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        out.extend_from_slice(format!("{:010} 00000 n \n", offset).as_bytes());
    }

    let mut trailer = format!(
        "trailer\n<< /Size {} /Root {} 0 R",
        objects.len() + 1,
        catalog_id
    );
    if let Some(info_id) = info_id {
        trailer.push_str(&format!(" /Info {} 0 R", info_id));
    }
    trailer.push_str(&format!(" >>\nstartxref\n{}\n%%EOF", xref_start));
    out.extend_from_slice(trailer.as_bytes());

    out
}

fn write_pdf_object<W: Write>(
    writer: &mut W,
    offset: &mut usize,
    offsets: &mut [usize],
    obj_id: usize,
    body: &str,
) -> io::Result<()> {
    if let Some(slot) = offsets.get_mut(obj_id) {
        *slot = *offset;
    }
    write_str(writer, &format!("{} 0 obj\n", obj_id), offset)?;
    write_bytes(writer, body.as_bytes(), offset)?;
    write_bytes(writer, b"\nendobj\n", offset)?;
    Ok(())
}

fn write_pdf_stream_object<W: Write>(
    writer: &mut W,
    offset: &mut usize,
    offsets: &mut [usize],
    obj_id: usize,
    dict_entries: &str,
    stream_data: &[u8],
) -> io::Result<()> {
    if let Some(slot) = offsets.get_mut(obj_id) {
        *slot = *offset;
    }
    write_str(writer, &format!("{} 0 obj\n", obj_id), offset)?;
    if dict_entries.trim().is_empty() {
        write_str(
            writer,
            &format!("<< /Length {} >>\nstream\n", stream_data.len()),
            offset,
        )?;
    } else {
        write_str(
            writer,
            &format!(
                "<< {} /Length {} >>\nstream\n",
                dict_entries.trim(),
                stream_data.len()
            ),
            offset,
        )?;
    }
    write_bytes(writer, stream_data, offset)?;
    write_bytes(writer, b"\nendstream\nendobj\n", offset)?;
    Ok(())
}

fn write_bytes<W: Write>(writer: &mut W, data: &[u8], offset: &mut usize) -> io::Result<()> {
    writer.write_all(data)?;
    *offset += data.len();
    Ok(())
}

fn write_str<W: Write>(writer: &mut W, data: &str, offset: &mut usize) -> io::Result<()> {
    write_bytes(writer, data.as_bytes(), offset)
}

fn escape_pdf_string(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out
}

struct WinAnsiEncoded {
    text: String,
    replaced: usize,
    fallbacks: usize,
}

fn encode_winansi_pdf_string(input: &str) -> WinAnsiEncoded {
    let mut out = String::new();
    let mut replaced = 0usize;
    let mut fallbacks = 0usize;
    for ch in input.chars() {
        // Common ASCII fallbacks for symbols that are not WinAnsi.
        match ch {
            '\u{2265}' => {
                out.push('>');
                out.push('=');
                fallbacks += 1;
                continue;
            }
            '\u{2264}' => {
                out.push('<');
                out.push('=');
                fallbacks += 1;
                continue;
            }
            _ => {}
        }

        let byte = match ch {
            // ASCII
            '\u{0000}'..='\u{007F}' => ch as u8,
            // Latin-1
            '\u{00A0}'..='\u{00FF}' => ch as u8,
            // WinAnsi extensions (cp1252)
            '\u{20AC}' => 0x80,
            '\u{201A}' => 0x82,
            '\u{0192}' => 0x83,
            '\u{201E}' => 0x84,
            '\u{2026}' => 0x85,
            '\u{2020}' => 0x86,
            '\u{2021}' => 0x87,
            '\u{02C6}' => 0x88,
            '\u{2030}' => 0x89,
            '\u{0160}' => 0x8A,
            '\u{2039}' => 0x8B,
            '\u{0152}' => 0x8C,
            '\u{017D}' => 0x8E,
            '\u{2018}' => 0x91,
            '\u{2019}' => 0x92,
            '\u{201C}' => 0x93,
            '\u{201D}' => 0x94,
            '\u{2022}' => 0x95,
            '\u{2013}' => 0x96,
            '\u{2014}' => 0x97,
            '\u{02DC}' => 0x98,
            '\u{2122}' => 0x99,
            '\u{0161}' => 0x9A,
            '\u{203A}' => 0x9B,
            '\u{0153}' => 0x9C,
            '\u{017E}' => 0x9E,
            '\u{0178}' => 0x9F,
            _ => {
                replaced += 1;
                b'?'
            }
        };

        match byte {
            b'\\' => out.push_str("\\\\"),
            b'(' => out.push_str("\\("),
            b')' => out.push_str("\\)"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b if b < 0x20 || b >= 0x7f => out.push_str(&format!("\\{:03o}", b)),
            b => out.push(b as char),
        }
    }

    WinAnsiEncoded {
        text: out,
        replaced,
        fallbacks,
    }
}

fn truncate_preview(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut out = String::new();
    for (idx, ch) in input.chars().enumerate() {
        if idx >= max_chars {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn escape_pdf_name(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            out.push(ch);
        } else {
            // PDF name escaping: # followed by two hex digits.
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).as_bytes() {
                out.push('#');
                out.push_str(&format!("{:02X}", b));
            }
        }
    }
    if out.is_empty() {
        "Span".to_string()
    } else {
        out
    }
}

fn escape_xml_text(input: &str) -> String {
    let mut out = String::new();
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
    out
}

fn build_xmp_metadata(
    profile: PdfProfile,
    version: PdfVersion,
    lang: Option<&str>,
    title: Option<&str>,
) -> Option<String> {
    if matches!(profile, PdfProfile::None) {
        return None;
    }

    let mut out = String::new();
    out.push_str(r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>"#);
    out.push_str("\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n");
    out.push_str("<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n");
    out.push_str("<rdf:Description xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" ");
    out.push_str("xmp:CreateDate=\"1970-01-01T00:00:00Z\" ");
    out.push_str("xmp:ModifyDate=\"1970-01-01T00:00:00Z\" ");
    out.push_str("xmp:MetadataDate=\"1970-01-01T00:00:00Z\"/>\n");
    out.push_str("<rdf:Description xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\" ");
    out.push_str("pdf:Producer=\"FullBleed\" ");
    out.push_str(&format!(
        "pdf:PDFVersion=\"{}\"/>\n",
        match version {
            PdfVersion::Pdf17 => "1.7",
            PdfVersion::Pdf20 => "2.0",
        }
    ));

    match profile {
        PdfProfile::PdfA1a => {
            out.push_str("<rdf:Description xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" ");
            out.push_str("pdfaid:part=\"1\" pdfaid:conformance=\"A\"/>\n");
        }
        PdfProfile::PdfA1b => {
            out.push_str("<rdf:Description xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" ");
            out.push_str("pdfaid:part=\"1\" pdfaid:conformance=\"B\"/>\n");
        }
        PdfProfile::PdfA2a => {
            out.push_str("<rdf:Description xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" ");
            out.push_str("pdfaid:part=\"2\" pdfaid:conformance=\"A\"/>\n");
        }
        PdfProfile::PdfA2b => {
            out.push_str("<rdf:Description xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" ");
            out.push_str("pdfaid:part=\"2\" pdfaid:conformance=\"B\"/>\n");
        }
        PdfProfile::PdfA2u => {
            out.push_str("<rdf:Description xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" ");
            out.push_str("pdfaid:part=\"2\" pdfaid:conformance=\"U\"/>\n");
        }
        PdfProfile::PdfA3a => {
            out.push_str("<rdf:Description xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" ");
            out.push_str("pdfaid:part=\"3\" pdfaid:conformance=\"A\"/>\n");
        }
        PdfProfile::PdfA3b => {
            out.push_str("<rdf:Description xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" ");
            out.push_str("pdfaid:part=\"3\" pdfaid:conformance=\"B\"/>\n");
        }
        PdfProfile::PdfA3u => {
            out.push_str("<rdf:Description xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" ");
            out.push_str("pdfaid:part=\"3\" pdfaid:conformance=\"U\"/>\n");
        }
        PdfProfile::PdfA4 => {
            out.push_str("<rdf:Description xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" ");
            out.push_str("pdfaid:part=\"4\" pdfaid:rev=\"2020\"/>\n");
        }
        PdfProfile::PdfA4e => {
            out.push_str("<rdf:Description xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" ");
            out.push_str("pdfaid:part=\"4\" pdfaid:conformance=\"E\" pdfaid:rev=\"2020\"/>\n");
        }
        PdfProfile::PdfA4f => {
            out.push_str("<rdf:Description xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" ");
            out.push_str("pdfaid:part=\"4\" pdfaid:conformance=\"F\" pdfaid:rev=\"2020\"/>\n");
        }
        PdfProfile::PdfX4 => {
            out.push_str("<rdf:Description xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\">");
            out.push_str("<pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion>");
            out.push_str("</rdf:Description>\n");
        }
        PdfProfile::PdfUa1 => {
            out.push_str("<rdf:Description xmlns:pdfuaid=\"http://www.aiim.org/pdfua/ns/id/\" ");
            out.push_str("pdfuaid:part=\"1\"/>\n");
        }
        PdfProfile::PdfUa2 => {
            out.push_str("<rdf:Description xmlns:pdfuaid=\"http://www.aiim.org/pdfua/ns/id/\" ");
            out.push_str("pdfuaid:part=\"2\" pdfuaid:rev=\"2024\"/>\n");
        }
        PdfProfile::PdfVt1 => {
            out.push_str("<rdf:Description xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\">");
            out.push_str("<pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion>");
            out.push_str("</rdf:Description>\n");
            out.push_str("<rdf:Description xmlns:pdfvtid=\"http://www.npes.org/pdfvt/ns/id/\" ");
            out.push_str("pdfvtid:GTS_PDFVTVersion=\"PDF/VT-1\" ");
            out.push_str("pdfvtid:GTS_PDFVTModDate=\"1970-01-01T00:00:00Z\"/>\n");
        }
        PdfProfile::Wtpdf1r => {
            push_pdf_declaration(&mut out, "http://pdfa.org/declarations/wtpdf#reuse1.0");
        }
        PdfProfile::Wtpdf1a => {
            push_pdf_declaration(
                &mut out,
                "http://pdfa.org/declarations/wtpdf#accessibility1.0",
            );
        }
        _ => {}
    }

    if let Some(lang) = lang {
        out.push_str("<rdf:Description xmlns:dc=\"http://purl.org/dc/elements/1.1/\">");
        out.push_str("<dc:language><rdf:Bag><rdf:li>");
        out.push_str(&escape_xml_text(lang));
        out.push_str("</rdf:li></rdf:Bag></dc:language></rdf:Description>\n");
    }

    if let Some(title) = title {
        out.push_str("<rdf:Description xmlns:dc=\"http://purl.org/dc/elements/1.1/\">");
        out.push_str("<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">");
        out.push_str(&escape_xml_text(title));
        out.push_str("</rdf:li></rdf:Alt></dc:title></rdf:Description>\n");
    }

    out.push_str("</rdf:RDF>\n</x:xmpmeta>\n");
    out.push_str("<?xpacket end=\"w\"?>");
    Some(out)
}

fn push_pdf_declaration(out: &mut String, conforms_to: &str) {
    out.push_str("<rdf:Description rdf:about=\"\" xmlns:pdfd=\"http://pdfa.org/declarations/\">");
    out.push_str("<pdfd:declarations><rdf:Bag><rdf:li rdf:parseType=\"Resource\">");
    out.push_str("<pdfd:conformsTo>");
    out.push_str(&escape_xml_text(conforms_to));
    out.push_str("</pdfd:conformsTo>");
    out.push_str("</rdf:li></rdf:Bag></pdfd:declarations>");
    out.push_str("</rdf:Description>\n");
}

fn cid_encoding_cmap(old_to_new: &BTreeMap<u16, u16>) -> String {
    let entries: Vec<(u16, u16)> = old_to_new
        .iter()
        .map(|(old_gid, new_cid)| (*old_gid, *new_cid))
        .collect();

    let mut out = String::new();
    out.push_str("/CIDInit /ProcSet findresource begin\n");
    out.push_str("12 dict begin\n");
    out.push_str("begincmap\n");
    out.push_str("/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def\n");
    out.push_str("/CMapName /FullBleed-CFFSubset def\n");
    out.push_str("/CMapType 1 def\n");
    out.push_str("/WMode 0 def\n");
    out.push_str("1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");

    let mut index = 0usize;
    while index < entries.len() {
        let end = (index + 100).min(entries.len());
        out.push_str(&format!("{} begincidchar\n", end - index));
        for (old_gid, new_cid) in &entries[index..end] {
            out.push_str(&format!("<{:04X}> {}\n", old_gid, new_cid));
        }
        out.push_str("endcidchar\n");
        index = end;
    }

    out.push_str("endcmap\n");
    out.push_str("CMapName currentdict /CMap defineresource pop\n");
    out.push_str("end\nend\n");
    out
}

fn to_unicode_cmap(glyph_map: &BTreeMap<u16, String>) -> String {
    let entries: Vec<(u16, String)> = glyph_map.iter().map(|(g, s)| (*g, s.clone())).collect();

    let mut out = String::new();
    out.push_str("/CIDInit /ProcSet findresource begin\n");
    out.push_str("12 dict begin\n");
    out.push_str("begincmap\n");
    out.push_str("/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def\n");
    out.push_str("/CMapName /Adobe-Identity-UCS def\n");
    out.push_str("/CMapType 2 def\n");
    out.push_str("1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");

    let mut idx = 0usize;
    while idx < entries.len() {
        let end = (idx + 100).min(entries.len());
        out.push_str(&format!("{} beginbfchar\n", end - idx));
        for (gid, s) in &entries[idx..end] {
            let mut uni = String::new();
            for ch in s.chars() {
                let code = ch as u32;
                if code <= 0xFFFF {
                    uni.push_str(&format!("{:04X}", code));
                } else {
                    let code = code - 0x1_0000;
                    let high = 0xD800 | ((code >> 10) as u32);
                    let low = 0xDC00 | (code & 0x3FF);
                    uni.push_str(&format!("{:04X}{:04X}", high, low));
                }
            }
            out.push_str(&format!("<{:04X}> <{}>\n", gid, uni));
        }
        out.push_str("endbfchar\n");
        idx = end;
    }

    out.push_str("endcmap\n");
    out.push_str("CMapName currentdict /CMap defineresource pop\n");
    out.push_str("end\nend\n");
    out
}

#[allow(dead_code)]
fn encode_cid_hex(text: &str, glyph_map: Option<&BTreeMap<u16, String>>) -> String {
    let mut out = String::new();
    out.push('<');
    for ch in text.chars() {
        let mut gid = 0;
        if let Some(map) = glyph_map {
            // Fallback: find first glyph that maps to this char.
            for (g, s) in map {
                if s.chars().next() == Some(ch) {
                    gid = *g;
                    break;
                }
            }
        }
        out.push_str(&format!("{:04X}", gid));
    }
    out.push('>');
    out
}

#[allow(dead_code)]
fn shape_text_to_glyph_map(font_data: &[u8], text: &str) -> Option<BTreeMap<u16, String>> {
    let (_, clean_text) = crate::text_shape::decode_shape_options(text);
    let shaped = crate::text_shape::shape(font_data, text)?;
    if shaped.glyphs.is_empty() {
        return None;
    }
    let mut map: BTreeMap<u16, String> = BTreeMap::new();
    let mut clusters: Vec<usize> = shaped
        .glyphs
        .iter()
        .map(|glyph| glyph.cluster as usize)
        .collect();
    clusters.sort_unstable();
    clusters.dedup();
    if clusters.last().copied() != Some(clean_text.len()) {
        clusters.push(clean_text.len());
    }
    for glyph in &shaped.glyphs {
        let start = (glyph.cluster as usize).min(clean_text.len());
        let boundary = clusters.binary_search(&start).unwrap_or_else(|index| index);
        let end = clusters
            .get(boundary + 1)
            .copied()
            .unwrap_or(clean_text.len())
            .min(clean_text.len());
        if start >= end {
            continue;
        }
        let s = clean_text[start..end].to_string();
        let gid = glyph.glyph_id;
        if gid != 0 {
            map.entry(gid).or_insert(s);
        }
    }
    Some(map)
}

#[allow(dead_code)]
fn shape_text_to_tj(
    registry: &FontRegistry,
    font_name: &str,
    _font_size: Pt,
    text: &str,
) -> Option<String> {
    let font = registry.resolve(font_name)?;
    let shaped = crate::text_shape::shape(&font.data, text)?;
    let units_per_em = shaped.units_per_em.max(1);
    if shaped.glyphs.is_empty() {
        return None;
    }

    let mut parts: Vec<String> = Vec::new();
    for glyph in &shaped.glyphs {
        let gid = glyph.glyph_id;
        if gid == 0 {
            continue;
        }
        if glyph.x_offset != 0 {
            parts.push(format_font_units(-i64::from(glyph.x_offset), units_per_em));
        }
        parts.push(format!("<{:04X}>", gid));

        let adv_default = registry
            .glyph_advance_units(font_name, gid)
            .map(|(advance, _)| i64::from(advance))
            .unwrap_or(0);
        let adjust = adv_default - i64::from(glyph.x_advance);
        if adjust != 0 {
            parts.push(format_font_units(adjust, units_per_em));
        }
    }

    if parts.is_empty() {
        return None;
    }
    Some(format!("[{}] TJ\n", parts.join(" ")))
}

fn synthetic_bold_ratio_millionths(stroke_width: Pt, font_size: Pt) -> u32 {
    let stroke = i128::from(stroke_width.max(Pt::ZERO).to_milli_i64());
    let size = i128::from(font_size.max(Pt::ZERO).to_milli_i64());
    if stroke == 0 || size == 0 {
        return 0;
    }
    ((stroke.saturating_mul(1_000_000) + size / 2) / size).clamp(0, i128::from(u32::MAX)) as u32
}

fn type3_glyph_program(
    outline: &RegisteredGlyphOutline,
    synthetic_bold_millionths: u32,
) -> (String, [f32; 4], f32) {
    let scale = 1000.0 / f32::from(outline.units_per_em.max(1));
    let transform = |x: f32, y: f32| (x * scale, -y * scale);
    let mut path = String::new();
    let mut current = (0.0_f32, 0.0_f32);
    let mut contour_start = current;
    let mut bbox = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    let include = |point: (f32, f32), bbox: &mut [f32; 4]| {
        bbox[0] = bbox[0].min(point.0);
        bbox[1] = bbox[1].min(point.1);
        bbox[2] = bbox[2].max(point.0);
        bbox[3] = bbox[3].max(point.1);
    };

    for command in &outline.commands {
        match *command {
            GlyphOutlineCommand::MoveTo(x, y) => {
                current = transform(x, y);
                contour_start = current;
                include(current, &mut bbox);
                path.push_str(&format!("{} {} m\n", fmt(current.0), fmt(current.1)));
            }
            GlyphOutlineCommand::LineTo(x, y) => {
                current = transform(x, y);
                include(current, &mut bbox);
                path.push_str(&format!("{} {} l\n", fmt(current.0), fmt(current.1)));
            }
            GlyphOutlineCommand::QuadTo(cx, cy, x, y) => {
                let control = transform(cx, cy);
                let end = transform(x, y);
                let c1 = (
                    current.0 + (control.0 - current.0) * (2.0 / 3.0),
                    current.1 + (control.1 - current.1) * (2.0 / 3.0),
                );
                let c2 = (
                    end.0 + (control.0 - end.0) * (2.0 / 3.0),
                    end.1 + (control.1 - end.1) * (2.0 / 3.0),
                );
                include(control, &mut bbox);
                include(end, &mut bbox);
                path.push_str(&format!(
                    "{} {} {} {} {} {} c\n",
                    fmt(c1.0),
                    fmt(c1.1),
                    fmt(c2.0),
                    fmt(c2.1),
                    fmt(end.0),
                    fmt(end.1),
                ));
                current = end;
            }
            GlyphOutlineCommand::CurveTo(c1x, c1y, c2x, c2y, x, y) => {
                let c1 = transform(c1x, c1y);
                let c2 = transform(c2x, c2y);
                let end = transform(x, y);
                include(c1, &mut bbox);
                include(c2, &mut bbox);
                include(end, &mut bbox);
                path.push_str(&format!(
                    "{} {} {} {} {} {} c\n",
                    fmt(c1.0),
                    fmt(c1.1),
                    fmt(c2.0),
                    fmt(c2.1),
                    fmt(end.0),
                    fmt(end.1),
                ));
                current = end;
            }
            GlyphOutlineCommand::Close => {
                path.push_str("h\n");
                current = contour_start;
            }
        }
    }

    if !bbox[0].is_finite() {
        bbox = [0.0; 4];
    }
    let stroke_width = synthetic_bold_millionths as f32 / 1000.0;
    if stroke_width > 0.0 {
        let expansion = stroke_width * 0.5;
        bbox[0] -= expansion;
        bbox[1] -= expansion;
        bbox[2] += expansion;
        bbox[3] += expansion;
    }
    let width = f32::from(outline.advance) * scale;
    let mut program = format!(
        "{} 0 {} {} {} {} d1\n",
        fmt(width),
        fmt(bbox[0]),
        fmt(bbox[1]),
        fmt(bbox[2]),
        fmt(bbox[3]),
    );
    if stroke_width > 0.0 {
        // Skia's synthetic font weight is a fill plus centered outline with a
        // four-unit miter limit. Keeping it inside the Type 3 glyph program
        // makes the expansion reusable and gives every occurrence the same
        // unhinted vector-mask raster phase.
        program.push_str(&format!(
            "0 J\n0 j\n4 M\n{} w\n",
            format_fixed(i64::from(synthetic_bold_millionths), 3)
        ));
    }
    program.push_str(&path);
    program.push_str(if stroke_width > 0.0 { "B\n" } else { "f\n" });
    (program, bbox, width)
}

fn fmt(value: f32) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    // `value` has already crossed the PDF serialization boundary. Multiplying
    // it by one million in Q32.32 used to saturate at 2_147_483_647, turning a
    // valid coordinate such as 2830.5 into 2147.483647. Large gradient axes are
    // common when a one-pixel border-image corner is stretched; use the exact
    // f32 value in f64 solely for bounded decimal formatting.
    let scaled = f64::from(value) * 1_000_000.0;
    let millionths = if scaled >= i64::MAX as f64 {
        i64::MAX
    } else if scaled <= i64::MIN as f64 {
        i64::MIN
    } else {
        scaled.round() as i64
    };
    format_fixed(millionths, 6)
}

fn format_font_units(units: i64, units_per_em: u16) -> String {
    let denominator = i128::from(units_per_em.max(1));
    let numerator = i128::from(units).saturating_mul(100_000_000);
    let rounded = if numerator >= 0 {
        (numerator + denominator / 2) / denominator
    } else {
        (numerator - denominator / 2) / denominator
    };
    format_fixed(
        rounded.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64,
        5,
    )
}

fn format_milli(milli: i64) -> String {
    format_fixed(milli, 3)
}

fn format_fixed(value: i64, decimal_places: usize) -> String {
    if value == 0 {
        return "0".to_string();
    }
    let sign = if value < 0 { "-" } else { "" };
    let abs = value.abs();
    let denominator = 10_i64.pow(decimal_places as u32);
    let int_part = abs / denominator;
    let frac_part = abs % denominator;
    if frac_part == 0 {
        format!("{}{}", sign, int_part)
    } else {
        let mut s = format!(
            "{}{}.{:0width$}",
            sign,
            int_part,
            frac_part,
            width = decimal_places
        );
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
        s
    }
}

fn fmt_pt(value: Pt) -> String {
    format_milli(value.to_milli_i64())
}

fn fmt_pdf_f64(value: f64, decimal_places: usize) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let scale = 10_i64.pow(decimal_places as u32) as f64;
    let scaled = value * scale;
    let fixed = if scaled >= i64::MAX as f64 {
        i64::MAX
    } else if scaled <= i64::MIN as f64 {
        i64::MIN
    } else {
        scaled.round() as i64
    };
    format_fixed(fixed, decimal_places)
}

fn filter_device_coordinate(value: Pt) -> f64 {
    f64::from(value.to_f32()) * f64::from(PDF_FILTER_RASTER_DPI) / 72.0
}

fn filter_raster_is_point_grid_aligned(
    device_x: f64,
    device_top: f64,
    pixel_width: u32,
    pixel_height: u32,
) -> bool {
    // At 300 DPI one device pixel is 6/25pt.  A tile maps to an exact
    // point-space CTM only when both dimensions and its device origin are
    // multiples of 25 pixels.  Fractional tiles retain the two-stage device
    // matrix so their image-sampling lattice remains browser-identical.
    fn aligned_coordinate(value: f64) -> bool {
        if !value.is_finite() {
            return false;
        }
        let rounded = value.round();
        (value - rounded).abs() < 1.0e-5
            && rounded >= i64::MIN as f64
            && rounded <= i64::MAX as f64
            && (rounded as i64).rem_euclid(25) == 0
    }

    pixel_width % 25 == 0
        && pixel_height % 25 == 0
        && aligned_coordinate(device_x)
        && aligned_coordinate(device_top)
}

/// Apply the physical sheet transform around an already compiled display list.
/// The display list stays in top-down trim coordinates, so bleed/marks and
/// rotation do not invalidate layout or variable-data binding plans.
fn wrap_page_content_for_presentation(content: String, geometry: PageGeometry) -> String {
    let extent = geometry.presentation.media_extent();
    let matrix = match geometry.presentation.orientation {
        PageOrientation::Upright => (1, 0, 0, 1, extent, extent),
        // PDF-space transform for CSS `page-orientation: rotate-left`:
        // x' = logical-height + extent - y, y' = x + extent.
        PageOrientation::RotateLeft => (0, 1, -1, 0, geometry.logical_size.height + extent, extent),
        // Mirrored counterpart: x' = y + extent,
        // y' = logical-width + extent - x.
        PageOrientation::RotateRight => (0, -1, 1, 0, extent, geometry.logical_size.width + extent),
    };
    if matrix == (1, 0, 0, 1, Pt::ZERO, Pt::ZERO) {
        return content;
    }
    let mut wrapped = String::with_capacity(content.len() + 80);
    wrapped.push_str("q\n");
    wrapped.push_str(&format!(
        "{} {} {} {} {} {} cm\n",
        matrix.0,
        matrix.1,
        matrix.2,
        matrix.3,
        fmt_pt(matrix.4),
        fmt_pt(matrix.5)
    ));
    wrapped.push_str(&content);
    if !content.ends_with('\n') {
        wrapped.push('\n');
    }
    wrapped.push_str("Q\n");
    wrapped
}

/// Preserve the device-pixel phase used by browser print pipelines.
///
/// Chromium's PDF path converts its 300-DPI print-device coordinates back to
/// points with a serialized `0.23999999` factor.  The mathematically equivalent
/// point-space scale is therefore just below one.  Keeping that minute scale,
/// anchored at the page top, prevents half-device-pixel CSS edges from landing
/// on the opposite raster row or column when both PDFs are rendered by the same
/// PDF rasterizer.  The physical-size delta is less than one part in 25 million.
fn wrap_page_content_for_print_device_phase(content: String, page_height: Pt) -> String {
    const PRINT_DEVICE_PHASE_SCALE: f64 = 0.999_999_96;

    let top_anchor = f64::from(page_height.to_f32()) * (1.0 - PRINT_DEVICE_PHASE_SCALE);
    let mut wrapped = String::with_capacity(content.len() + 80);
    wrapped.push_str("q\n0.99999996 0 0 0.99999996 0 ");
    wrapped.push_str(&format!("{top_anchor:.9}"));
    wrapped.push_str(" cm\n");
    wrapped.push_str(&content);
    if !content.ends_with('\n') {
        wrapped.push('\n');
    }
    wrapped.push_str("Q\n");
    wrapped
}

fn clamp_unit(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else if value < 0.0 {
        0.0
    } else if value > 1.0 {
        1.0
    } else {
        value
    }
}

fn rgb_to_cmyk(color: Color) -> (f32, f32, f32, f32) {
    let r = clamp_unit(color.r);
    let g = clamp_unit(color.g);
    let b = clamp_unit(color.b);
    let k = 1.0 - r.max(g).max(b);
    if k >= 1.0 - 1e-6 {
        return (0.0, 0.0, 0.0, 1.0);
    }
    let c = (1.0 - r - k) / (1.0 - k);
    let m = (1.0 - g - k) / (1.0 - k);
    let y = (1.0 - b - k) / (1.0 - k);
    (clamp_unit(c), clamp_unit(m), clamp_unit(y), clamp_unit(k))
}

fn color_components(color: Color, space: ColorSpace) -> Vec<f32> {
    match space {
        ColorSpace::Rgb => vec![color.r, color.g, color.b],
        ColorSpace::Cmyk => {
            let (c, m, y, k) = rgb_to_cmyk(color);
            vec![c, m, y, k]
        }
    }
}

fn color_to_pdf_fill(color: Color, space: ColorSpace) -> String {
    match space {
        ColorSpace::Rgb => format!("{} {} {} rg\n", fmt(color.r), fmt(color.g), fmt(color.b)),
        ColorSpace::Cmyk => {
            let (c, m, y, k) = rgb_to_cmyk(color);
            format!("{} {} {} {} k\n", fmt(c), fmt(m), fmt(y), fmt(k))
        }
    }
}

fn color_to_pdf_stroke(color: Color, space: ColorSpace) -> String {
    match space {
        ColorSpace::Rgb => format!("{} {} {} RG\n", fmt(color.r), fmt(color.g), fmt(color.b)),
        ColorSpace::Cmyk => {
            let (c, m, y, k) = rgb_to_cmyk(color);
            format!("{} {} {} {} K\n", fmt(c), fmt(m), fmt(y), fmt(k))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Page;
    use crate::pdf_native::{Document as LoDocument, Object as LoObject};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn binding_plan_compiles_coordinate_state_and_restores_its_checkpoint() {
        let document = Document {
            page_size: Size {
                width: Pt::from_f32(200.0),
                height: Pt::from_f32(120.0),
            },
            pages: vec![Page {
                commands: vec![
                    Command::SaveState,
                    Command::Translate(Pt::from_f32(12.0), Pt::from_f32(8.0)),
                    Command::MoveTo {
                        x: Pt::ZERO,
                        y: Pt::ZERO,
                    },
                    Command::LineTo {
                        x: Pt::from_f32(80.0),
                        y: Pt::ZERO,
                    },
                    Command::LineTo {
                        x: Pt::from_f32(80.0),
                        y: Pt::from_f32(24.0),
                    },
                    Command::ClosePath,
                    Command::ClipPath { evenodd: false },
                    Command::DrawString {
                        x: Pt::from_f32(4.0),
                        y: Pt::from_f32(16.0),
                        text: "{{inside}}".to_string(),
                    },
                    Command::RestoreState,
                    Command::DrawString {
                        x: Pt::from_f32(4.0),
                        y: Pt::from_f32(48.0),
                        text: "{{outside}}".to_string(),
                    },
                ],
            }],
        };
        let plan = compile_binding_plan(&document, &["inside".to_string(), "outside".to_string()])
            .expect("compile coordinate-state binding plan");
        let overlay = &plan.pages[0].overlay_page.commands;

        let inside = overlay
            .iter()
            .position(|command| {
                matches!(command, Command::DrawString { text, .. } if text == "{{inside}}")
            })
            .expect("inside binding command");
        let inside_start = overlay[..inside]
            .iter()
            .rposition(|command| matches!(command, Command::SaveState))
            .expect("inside binding state");
        let inside_program = &overlay[inside_start..inside];
        assert!(
            inside_program
                .iter()
                .any(|command| matches!(command, Command::Translate(..)))
        );
        assert!(
            inside_program
                .iter()
                .any(|command| matches!(command, Command::MoveTo { .. }))
        );
        assert!(
            inside_program
                .iter()
                .any(|command| matches!(command, Command::ClipPath { evenodd: false }))
        );

        let outside = overlay
            .iter()
            .position(|command| {
                matches!(command, Command::DrawString { text, .. } if text == "{{outside}}")
            })
            .expect("outside binding command");
        let outside_start = overlay[..outside]
            .iter()
            .rposition(|command| matches!(command, Command::SaveState))
            .expect("outside binding state");
        let outside_program = &overlay[outside_start..outside];
        assert!(
            !outside_program.iter().any(|command| matches!(
                command,
                Command::Translate(..)
                    | Command::MoveTo { .. }
                    | Command::LineTo { .. }
                    | Command::ClipPath { .. }
            )),
            "restoring the source graphics state must discard its virtual coordinate program"
        );
    }

    #[test]
    fn to_unicode_cmap_handles_surrogates() {
        let mut map = BTreeMap::new();
        map.insert(3u16, "A".to_string());
        map.insert(4u16, "\u{1F600}".to_string());
        let cmap = to_unicode_cmap(&map);
        assert!(cmap.contains("<0003> <0041>"));
        assert!(cmap.contains("<0004> <D83DDE00>"));
    }

    #[test]
    fn cff_subset_encoding_cmap_maps_original_codes_to_dense_cids() {
        let map = BTreeMap::from([(0u16, 0u16), (42u16, 1u16), (8192u16, 2u16)]);
        let cmap = cid_encoding_cmap(&map);
        assert!(cmap.contains("/CMapType 1 def"));
        assert!(cmap.contains("<002A> 1"));
        assert!(cmap.contains("<2000> 2"));
    }

    #[test]
    fn floating_pdf_values_keep_sub_milli_precision() {
        assert_eq!(fmt(252.0 / 255.0), "0.988235");
        assert_eq!(fmt(183.0 / 255.0), "0.717647");
        assert_eq!(fmt(1.0), "1");
        assert_eq!(fmt(2830.5), "2830.5");
        assert_eq!(fmt(-2830.5), "-2830.5");
    }

    #[test]
    fn hard_conic_pdf_lowering_tessellates_wide_bands() {
        let red = Color::rgb(1.0, 0.0, 0.0);
        let green = Color::rgb(0.0, 1.0, 0.0);
        let blue = Color::rgb(0.0, 0.0, 1.0);
        let white = Color::rgb(1.0, 1.0, 1.0);
        let mut stops = Vec::new();
        for (start, end, color) in [
            (0.0, 0.25, red),
            (0.25, 0.5, green),
            (0.5, 0.75, blue),
            (0.75, 1.0, white),
        ] {
            stops.push(ShadingStop {
                offset: start,
                color,
                alpha: 1.0,
            });
            stops.push(ShadingStop {
                offset: end,
                color,
                alpha: 1.0,
            });
        }
        let doc = one_page_document(vec![Command::ShadingFill(Shading::Conic {
            center_x: 100.0,
            center_y: 100.0,
            radius: 142.0,
            start_angle_deg: 0.0,
            stops,
            hard_stops: true,
        })]);

        let bytes = document_to_pdf(&doc).expect("render hard conic pdf");
        let content = page_content_bytes(&bytes);
        assert_eq!(count_token(&content, b"h\nf\n"), 4);
        assert_eq!(count_token(&content, b" l\n"), 12);
    }

    #[test]
    fn vector_mask_compiler_keeps_axial_hard_stops_and_smooth_other_shaders() {
        let axial_hard_stop = vec![Command::ShadingFill(Shading::Axial {
            x0: 0.0,
            y0: 10.0,
            x1: 20.0,
            y1: 10.0,
            stops: vec![
                ShadingStop {
                    offset: 0.5,
                    color: Color::BLACK,
                    alpha: 1.0,
                },
                ShadingStop {
                    offset: 0.5,
                    color: Color::BLACK,
                    alpha: 0.0,
                },
            ],
        })];
        let axial_repeating = vec![Command::ShadingFill(Shading::Axial {
            x0: 0.0,
            y0: 10.0,
            x1: 20.0,
            y1: 10.0,
            stops: vec![
                ShadingStop {
                    offset: 0.25,
                    color: Color::BLACK,
                    alpha: 1.0,
                },
                ShadingStop {
                    offset: 0.25,
                    color: Color::BLACK,
                    alpha: 0.0,
                },
                ShadingStop {
                    offset: 0.75,
                    color: Color::BLACK,
                    alpha: 0.0,
                },
                ShadingStop {
                    offset: 0.75,
                    color: Color::BLACK,
                    alpha: 1.0,
                },
            ],
        })];
        let radial = |stops| {
            Command::ShadingFill(Shading::Radial {
                x0: 10.0,
                y0: 10.0,
                r0: 0.0,
                x1: 10.0,
                y1: 10.0,
                r1: 20.0,
                stops,
                hard_stops: false,
            })
        };
        let smooth = vec![radial(vec![
            ShadingStop {
                offset: 0.0,
                color: Color::BLACK,
                alpha: 1.0,
            },
            ShadingStop {
                offset: 1.0,
                color: Color::BLACK,
                alpha: 0.0,
            },
        ])];
        let discontinuous = vec![radial(vec![
            ShadingStop {
                offset: 0.5,
                color: Color::BLACK,
                alpha: 1.0,
            },
            ShadingStop {
                offset: 0.5,
                color: Color::BLACK,
                alpha: 0.0,
            },
        ])];

        let axial = vector_alpha_mask_commands(&axial_hard_stop).expect("compiled axial mask");
        let Command::ShadingFill(Shading::Axial { x0, x1, .. }) = &axial[0] else {
            panic!("expected axial mask shader");
        };
        assert!((*x0 - 0.24).abs() <= 1.0e-6);
        assert!((*x1 - 20.24).abs() <= 1.0e-6);
        assert!(vector_alpha_mask_commands(&axial_repeating).is_none());
        assert!(
            vector_alpha_mask_commands(&[axial_hard_stop[0].clone(), axial_hard_stop[0].clone(),])
                .is_none()
        );
        assert!(vector_alpha_mask_commands(&smooth).is_some());
        assert!(vector_alpha_mask_commands(&discontinuous).is_none());
    }

    #[test]
    fn masked_filter_form_specializes_raster_to_outer_device_phase() {
        let width = Pt::from_f32(112.5);
        let height = Pt::from_f32(60.0);
        let filtered_content = "phase-filter-content".to_string();
        let masked_source = "phase-masked-source".to_string();
        let mask = "phase-mask".to_string();
        let mut filter = crate::flowable::PaintFilterSpec::identity();
        filter
            .operations
            .push(crate::flowable::PaintFilterOperation::Blur(Pt::from_f32(
                6.75,
            )));
        let doc = Document {
            page_size: Size {
                width: Pt::from_f32(150.0),
                height: Pt::from_f32(102.0),
            },
            pages: vec![Page {
                commands: vec![
                    Command::DefineForm {
                        resource_id: filtered_content.clone(),
                        width,
                        height,
                        commands: vec![
                            Command::SetFillColor(Color::rgb(0.83, 0.18, 0.18)),
                            Command::DrawRect {
                                x: Pt::ZERO,
                                y: Pt::ZERO,
                                width,
                                height,
                            },
                        ],
                    },
                    Command::DefineIsolatedForm {
                        resource_id: masked_source.clone(),
                        width,
                        height,
                        commands: vec![Command::DrawFilteredForm {
                            x: Pt::ZERO,
                            y: Pt::ZERO,
                            width,
                            height,
                            resource_id: filtered_content,
                            filter,
                            css_shadow: false,
                        }],
                    },
                    Command::DefineForm {
                        resource_id: mask.clone(),
                        width,
                        height,
                        commands: vec![Command::DrawRect {
                            x: Pt::ZERO,
                            y: Pt::ZERO,
                            width,
                            height,
                        }],
                    },
                    Command::DrawMaskedForm {
                        x: Pt::from_f32(18.75),
                        y: Pt::from_f32(23.25),
                        width,
                        height,
                        resource_id: masked_source,
                        layers: vec![crate::canvas::CompiledMaskLayer {
                            resource_id: mask,
                            mode: crate::flowable::MaskMode::Alpha,
                            composite: crate::flowable::MaskComposite::Add,
                        }],
                    },
                ],
            }],
        };

        let bytes = document_to_pdf(&doc).expect("render phase-specialized masked filter");
        assert!(
            bytes
                .windows(b"/Width 469 /Height 251".len())
                .any(|window| window == b"/Width 469 /Height 251")
        );
    }

    #[test]
    fn font_units_serialize_without_thousand_em_rounding() {
        assert_eq!(format_font_units(1401, 2048), "684.08203");
        assert_eq!(format_font_units(36, 2048), "17.57813");
        assert_eq!(format_font_units(-36, 2048), "-17.57813");
    }

    fn one_page_document(commands: Vec<Command>) -> Document {
        Document {
            page_size: Size::a4(),
            pages: vec![Page { commands }],
        }
    }

    #[test]
    fn page_content_keeps_browser_print_device_phase_top_anchored() {
        let doc = Document {
            page_size: Size {
                width: Pt::from_f32(390.0),
                height: Pt::from_f32(150.0),
            },
            pages: vec![Page {
                commands: vec![Command::DrawRect {
                    x: Pt::from_f32(12.0),
                    y: Pt::from_f32(12.0),
                    width: Pt::from_f32(117.0),
                    height: Pt::from_f32(123.0),
                }],
            }],
        };

        let bytes = document_to_pdf(&doc).expect("render phase-preserving pdf");
        let content = page_content_bytes(&bytes);
        assert!(content.starts_with(b"q\n0.99999996 0 0 0.99999996 0 0.000006000 cm\n"));
        assert!(content.ends_with(b"Q\n\n"));
    }

    #[test]
    fn filtered_raster_lowers_points_back_to_print_device_coordinates() {
        assert!((filter_device_coordinate(Pt::from_f32(132.0)) - 550.0).abs() < 1.0e-4);
        assert!((filter_device_coordinate(Pt::from_f32(7.2)) - 30.0).abs() < 1.0e-4);
        assert!(filter_raster_is_point_grid_aligned(0.0, 200.0, 750, 300));
        assert!(!filter_raster_is_point_grid_aligned(0.0, 0.0, 757, 476));
        assert!(!filter_raster_is_point_grid_aligned(30.0, 63.0, 475, 182));
    }

    #[test]
    fn page_presentation_geometry_is_virtualized_after_layout() {
        let logical = Size {
            width: Pt::from_f32(78.0),
            height: Pt::from_f32(120.0),
        };
        let presentation = PagePresentation {
            bleed: Pt::from_f32(9.0),
            marks: crate::types::PageMarks {
                crop: true,
                cross: false,
            },
            orientation: PageOrientation::RotateLeft,
        };
        let page = Page {
            commands: vec![Command::Meta {
                key: META_PAGE_PRESENTATION_KEY.to_string(),
                value: presentation.encode(),
            }],
        };
        let geometry = PageGeometry::for_page(&page, logical);
        assert_eq!(geometry.logical_size, logical);
        assert_eq!(geometry.media_size.width, Pt::from_f32(138.0));
        assert_eq!(geometry.media_size.height, Pt::from_f32(96.0));
        let wrapped = wrap_page_content_for_presentation("0 0 m\n".to_string(), geometry);
        assert!(wrapped.contains("0 1 -1 0 129 9 cm"));
        assert_eq!(
            page_box_entries(PdfProfile::None, geometry),
            " /TrimBox [9 9 129 87] /BleedBox [0 0 138 96] /CropBox [0 0 138 96]"
        );
    }

    #[test]
    fn shading_coordinates_are_serialized_in_pdf_bottom_up_space() {
        let shading = Shading::Axial {
            x0: 10.0,
            y0: 20.0,
            x1: 30.0,
            y1: 40.0,
            stops: vec![
                ShadingStop {
                    offset: 0.0,
                    color: Color::BLACK,
                    alpha: 1.0,
                },
                ShadingStop {
                    offset: 1.0,
                    color: Color::rgb(1.0, 1.0, 1.0),
                    alpha: 1.0,
                },
            ],
        };

        let (objects, _, _) =
            shading_to_objects(&shading, 10, Pt::from_f32(100.0), ColorSpace::Rgb);
        let dictionary = objects.last().expect("shading dictionary");
        assert!(dictionary.contains("/Coords [10 80 30 60]"));
        assert!(!dictionary.contains("/Matrix"));
    }

    #[test]
    fn translucent_shading_uses_a_luminosity_soft_mask() {
        let shading = Shading::Axial {
            x0: 0.0,
            y0: 0.0,
            x1: 100.0,
            y1: 0.0,
            stops: vec![
                ShadingStop {
                    offset: 0.0,
                    color: Color::rgb(1.0, 0.0, 0.0),
                    alpha: 0.0,
                },
                ShadingStop {
                    offset: 1.0,
                    color: Color::rgb(1.0, 0.0, 0.0),
                    alpha: 1.0,
                },
            ],
        };
        let doc = one_page_document(vec![Command::ShadingFill(shading)]);

        let bytes = document_to_pdf(&doc).expect("render translucent shading");
        let pdf = String::from_utf8_lossy(&bytes);
        assert!(pdf.contains("/SMask << /S /Luminosity"));
        assert!(pdf.contains("/Group << /S /Transparency /CS /DeviceRGB"));
        assert!(pdf.contains("/MaskSh sh"));
        assert!(pdf.contains(" gs\n/Sh"));
    }

    #[test]
    fn filtered_form_is_rasterized_into_an_alpha_image() {
        let size = Size {
            width: Pt::from_f32(40.0),
            height: Pt::from_f32(40.0),
        };
        let mut filter = crate::flowable::PaintFilterSpec::identity();
        filter.brightness = 0.5;
        let doc = Document {
            page_size: size,
            pages: vec![Page {
                commands: vec![
                    Command::DefineForm {
                        resource_id: "filtered".to_string(),
                        width: size.width,
                        height: size.height,
                        commands: vec![
                            Command::SetFillColor(Color::rgb(1.0, 0.0, 0.0)),
                            Command::DrawRect {
                                x: Pt::from_f32(10.0),
                                y: Pt::from_f32(10.0),
                                width: Pt::from_f32(20.0),
                                height: Pt::from_f32(20.0),
                            },
                        ],
                    },
                    Command::DrawFilteredForm {
                        x: Pt::ZERO,
                        y: Pt::ZERO,
                        width: size.width,
                        height: size.height,
                        resource_id: "filtered".to_string(),
                        filter,
                        css_shadow: false,
                    },
                ],
            }],
        };

        let bytes = document_to_pdf(&doc).expect("render filtered form");
        let pdf = String::from_utf8_lossy(&bytes);
        let content_bytes = page_content_bytes(&bytes);
        let content = String::from_utf8_lossy(&content_bytes);
        assert!(pdf.contains("/Subtype /Image"));
        assert!(pdf.contains("/SMask"));
        assert!(content.contains("/Im"));
        assert!(content.contains("0.24 0 0 -0.24 0 40 cm"));
    }

    #[test]
    fn synthetic_bold_emits_fill_stroke_for_one_text_run() {
        let doc = one_page_document(vec![
            Command::SetFontName("Helvetica".to_string()),
            Command::SetFontSize(Pt::from_f32(12.0)),
            Command::SaveState,
            Command::SetLineWidth(Pt::from_f32(0.5)),
            Command::SetTextRenderingMode(2),
            Command::DrawString {
                x: Pt::from_f32(10.0),
                y: Pt::from_f32(20.0),
                text: "Bold".to_string(),
            },
            Command::RestoreState,
        ]);
        let bytes = document_to_pdf(&doc).expect("render synthetic bold pdf");
        let content = page_content_bytes(&bytes);
        assert_eq!(count_token(&content, b"2 Tr"), 1);
        assert_eq!(count_token(&content, b"(Bold) Tj"), 1);
    }

    #[test]
    fn pdf_emitter_restores_tracked_font_size_after_graphics_scope() {
        let doc = one_page_document(vec![
            Command::SaveState,
            Command::SetFontSize(Pt::from_f32(9.0)),
            Command::DrawString {
                x: Pt::from_f32(10.0),
                y: Pt::from_f32(20.0),
                text: "Scoped".to_string(),
            },
            Command::RestoreState,
            Command::DrawString {
                x: Pt::from_f32(10.0),
                y: Pt::from_f32(40.0),
                text: "Default".to_string(),
            },
        ]);

        let bytes = document_to_pdf(&doc).expect("render state-restoring pdf");
        let content = page_content_bytes(&bytes);
        assert_eq!(count_token(&content, b"/F1 9 Tf"), 1);
        assert_eq!(count_token(&content, b"/F1 12 Tf"), 1);
    }

    #[test]
    fn transformed_text_emits_an_extractable_text_matrix() {
        let doc = one_page_document(vec![
            Command::SetFontName("Helvetica".to_string()),
            Command::SetFontSize(Pt::from_f32(12.0)),
            Command::DrawStringTransformed {
                x: Pt::from_f32(10.0),
                y: Pt::from_f32(60.0),
                text: "Italic".to_string(),
                m00: 1.0,
                m01: 0.0,
                m10: 0.25,
                m11: 1.0,
            },
        ]);
        let bytes = document_to_pdf(&doc).expect("render transformed text pdf");
        let content = page_content_bytes(&bytes);

        assert_eq!(count_token(&content, b"1 0 0.25 1 10 60 Tm"), 1);
        assert_eq!(count_token(&content, b"(Italic) Tj"), 1);
    }

    #[test]
    fn css_translation_converts_the_top_down_y_axis_to_pdf_user_space() {
        let doc = one_page_document(vec![Command::Translate(
            Pt::from_f32(10.0),
            Pt::from_f32(20.0),
        )]);
        let bytes = document_to_pdf(&doc).expect("render translated pdf");
        let content = page_content_bytes(&bytes);
        assert_eq!(count_token(&content, b"1 0 0 1 10 -20 cm"), 1);
    }

    #[test]
    fn affine_matrix_converts_the_top_down_y_axis_to_pdf_user_space() {
        let doc = one_page_document(vec![Command::ConcatMatrix {
            a: 1.0,
            b: 2.0,
            c: 3.0,
            d: 4.0,
            e: Pt::from_f32(10.0),
            f: Pt::from_f32(20.0),
        }]);
        let bytes = document_to_pdf(&doc).expect("render affine pdf");
        let content = page_content_bytes(&bytes);
        assert_eq!(count_token(&content, b"1 -2 -3 4 10 -20 cm"), 1);
    }

    fn count_token(bytes: &[u8], token: &[u8]) -> usize {
        if token.is_empty() || bytes.len() < token.len() {
            return 0;
        }
        bytes.windows(token.len()).filter(|w| *w == token).count()
    }

    fn page_content_bytes(bytes: &[u8]) -> Vec<u8> {
        let doc = LoDocument::load_mem(bytes).expect("load pdf");
        let mut out = Vec::new();
        for (_, page_id) in doc.get_pages() {
            let content = doc.get_page_content(page_id).expect("page content");
            out.extend_from_slice(&content);
            out.push(b'\n');
        }
        out
    }

    fn count_page_content_token(bytes: &[u8], token: &[u8]) -> usize {
        let content = page_content_bytes(bytes);
        count_token(&content, token)
    }

    fn first_page_content_filter(bytes: &[u8]) -> Option<Vec<u8>> {
        let doc = LoDocument::load_mem(bytes).expect("load pdf");
        let page_id = *doc
            .get_pages()
            .values()
            .next()
            .expect("at least one page expected");
        let page = doc
            .get_object(page_id)
            .and_then(LoObject::as_dict)
            .expect("page dict");
        let contents = page.get(b"Contents").expect("page contents");
        let content_id = match contents {
            LoObject::Reference(id) => *id,
            LoObject::Array(arr) => arr
                .first()
                .and_then(|o| o.as_reference().ok())
                .expect("content array has reference"),
            _ => panic!("unsupported page contents object"),
        };
        let stream = doc
            .get_object(content_id)
            .and_then(LoObject::as_stream)
            .expect("content stream");
        match stream.dict.get(b"Filter") {
            Ok(LoObject::Name(name)) => Some(name.clone()),
            Ok(LoObject::Array(arr)) => arr
                .first()
                .and_then(|o| o.as_name().ok())
                .map(|n| n.to_vec()),
            _ => None,
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

    fn text_page(font_name: &str, text: &str) -> Document {
        one_page_document(vec![
            Command::SetFontName(font_name.to_string()),
            Command::SetFontSize(Pt::from_f32(11.0)),
            Command::DrawString {
                x: Pt::from_f32(72.0),
                y: Pt::from_f32(88.0),
                text: text.to_string(),
            },
        ])
    }

    #[test]
    fn pdfx4_requires_output_intent() {
        let doc = one_page_document(vec![]);
        let mut options = PdfOptions::default();
        options.pdf_profile = PdfProfile::PdfX4;

        let err = document_to_pdf_with_metrics_and_registry(&doc, None, None, &options)
            .expect_err("pdfx4 should fail without output intent");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("output intent"));
    }

    #[test]
    fn pdfx4_emits_required_tokens() {
        let doc = one_page_document(vec![]);
        let mut options = PdfOptions::default();
        options.pdf_profile = PdfProfile::PdfX4;
        options.output_intent = Some(OutputIntent::new(
            vec![0x00, 0x01, 0x02],
            3,
            "sRGB IEC61966-2.1",
            Some("sRGB".to_string()),
        ));

        let bytes = document_to_pdf_with_metrics_and_registry(&doc, None, None, &options).unwrap();
        let pdf = String::from_utf8_lossy(&bytes);
        assert!(pdf.contains("/OutputIntents"));
        assert!(pdf.contains("/S /GTS_PDFX"));
        assert!(pdf.contains("/TrimBox"));
        assert!(pdf.contains("/BleedBox"));
        assert!(pdf.contains("/CropBox"));
        assert!(pdf.contains("/Trapped /False"));
        assert!(pdf.contains("/GTS_PDFXVersion (PDF/X-4)"));
    }

    #[test]
    fn pdfa_variants_require_output_intent_and_emit_identification_xmp() {
        let doc = one_page_document(vec![]);
        for (profile, part_token, conformance_token, header_token) in [
            (
                PdfProfile::PdfA1a,
                "pdfaid:part=\"1\"",
                Some("pdfaid:conformance=\"A\""),
                "%PDF-1.7",
            ),
            (
                PdfProfile::PdfA1b,
                "pdfaid:part=\"1\"",
                Some("pdfaid:conformance=\"B\""),
                "%PDF-1.7",
            ),
            (
                PdfProfile::PdfA2a,
                "pdfaid:part=\"2\"",
                Some("pdfaid:conformance=\"A\""),
                "%PDF-1.7",
            ),
            (
                PdfProfile::PdfA2b,
                "pdfaid:part=\"2\"",
                Some("pdfaid:conformance=\"B\""),
                "%PDF-1.7",
            ),
            (
                PdfProfile::PdfA2u,
                "pdfaid:part=\"2\"",
                Some("pdfaid:conformance=\"U\""),
                "%PDF-1.7",
            ),
            (
                PdfProfile::PdfA3a,
                "pdfaid:part=\"3\"",
                Some("pdfaid:conformance=\"A\""),
                "%PDF-1.7",
            ),
            (
                PdfProfile::PdfA3b,
                "pdfaid:part=\"3\"",
                Some("pdfaid:conformance=\"B\""),
                "%PDF-1.7",
            ),
            (
                PdfProfile::PdfA3u,
                "pdfaid:part=\"3\"",
                Some("pdfaid:conformance=\"U\""),
                "%PDF-1.7",
            ),
            (
                PdfProfile::PdfA4,
                "pdfaid:part=\"4\"",
                Some("pdfaid:rev=\"2020\""),
                "%PDF-2.0",
            ),
            (
                PdfProfile::PdfA4e,
                "pdfaid:part=\"4\"",
                Some("pdfaid:conformance=\"E\""),
                "%PDF-2.0",
            ),
            (
                PdfProfile::PdfA4f,
                "pdfaid:part=\"4\"",
                Some("pdfaid:conformance=\"F\""),
                "%PDF-2.0",
            ),
        ] {
            let mut missing = PdfOptions::default();
            missing.pdf_profile = profile;
            let err = document_to_pdf_with_metrics_and_registry(&doc, None, None, &missing)
                .expect_err("pdf/a profile should require output intent");
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
            assert!(err.to_string().contains(profile.as_str()));

            let mut options = PdfOptions::default();
            options.pdf_profile = profile;
            options.document_title = Some("PDF/A seed".to_string());
            options.output_intent = Some(OutputIntent::new(
                vec![0x00, 0x01, 0x02],
                3,
                "sRGB IEC61966-2.1",
                Some("sRGB".to_string()),
            ));

            let bytes =
                document_to_pdf_with_metrics_and_registry(&doc, None, None, &options).unwrap();
            let pdf = String::from_utf8_lossy(&bytes);
            assert!(pdf.starts_with(header_token));
            assert!(pdf.contains(part_token));
            if let Some(token) = conformance_token {
                assert!(pdf.contains(token));
            }
            if profile.is_pdfa4_family() {
                let trailer = pdf
                    .rsplit("trailer\n")
                    .next()
                    .expect("pdf should contain a trailer");
                assert!(!trailer.contains(" /Info "));
                if profile == PdfProfile::PdfA4 {
                    assert!(!pdf.contains("pdfaid:conformance"));
                }
            }
            if profile == PdfProfile::PdfA4f {
                assert!(pdf.contains("/EmbeddedFiles"));
                assert!(pdf.contains("/Type /Filespec"));
                assert!(pdf.contains("/Type /EmbeddedFile"));
                assert!(pdf.contains("/AFRelationship /Data"));
                assert!(pdf.contains("/AF ["));
            }
            assert!(pdf.contains("/Type /Metadata /Subtype /XML"));
            assert!(pdf.contains("/S /GTS_PDFA1"));
            assert!(pdf.contains("/ID [<"));
        }
    }

    #[test]
    fn explicit_glyph_run_emits_embedded_type3_outline_font() {
        let inter_path = repo_font_path("Inter-Variable.ttf");
        let inter_bytes = std::fs::read(&inter_path).expect("read inter");
        let mut registry = FontRegistry::new();
        let inter_name = registry
            .register_bytes(inter_bytes, Some(inter_path.to_string_lossy().as_ref()))
            .expect("register inter");
        let glyph_id = registry.map_glyph_id_for_char(&inter_name, 'A');
        assert_ne!(glyph_id, 0);
        let advance = Pt::from_f32(21.0).mul_ratio(
            i32::from(registry.glyph_advance(&inter_name, glyph_id)),
            1000,
        );
        let doc = one_page_document(vec![
            Command::SetFontName(inter_name),
            Command::SetFontSize(Pt::from_f32(21.0)),
            Command::DrawGlyphRun {
                x: Pt::from_f32(13.5),
                y: Pt::from_f32(45.0),
                glyph_ids: vec![glyph_id],
                advances: vec![(advance, Pt::ZERO)],
                m00: 1.0,
                m01: 0.0,
                m10: 0.0,
                m11: 1.0,
            },
        ]);

        let bytes = document_to_pdf_with_metrics_and_registry(
            &doc,
            None,
            Some(&registry),
            &PdfOptions::default(),
        )
        .expect("render type3 glyph run");
        let pdf = String::from_utf8_lossy(&bytes);
        assert!(pdf.contains("/Subtype /Type3"));
        assert!(pdf.contains("/FontMatrix [0.001 0 0 -0.001 0 0]"));
        assert!(pdf.contains("/CharProcs << /.notdef"));
        assert!(pdf.contains(&format!("/g{:04X}", glyph_id)));
        assert!(pdf.contains("/T3F1 "));
    }

    #[test]
    fn synthetic_bold_glyph_run_reuses_one_stroked_type3_program() {
        let inter_path = repo_font_path("Inter-Variable.ttf");
        let inter_bytes = std::fs::read(&inter_path).expect("read inter");
        let mut registry = FontRegistry::new();
        let inter_name = registry
            .register_bytes(inter_bytes, Some(inter_path.to_string_lossy().as_ref()))
            .expect("register inter");
        let glyph_id = registry.map_glyph_id_for_char(&inter_name, 'A');
        assert_ne!(glyph_id, 0);
        let advance = Pt::from_f32(21.0).mul_ratio(
            i32::from(registry.glyph_advance(&inter_name, glyph_id)),
            1000,
        );
        let doc = one_page_document(vec![
            Command::SetFontName(inter_name),
            Command::SetFontSize(Pt::from_f32(21.0)),
            Command::DrawSyntheticBoldGlyphRun {
                x: Pt::from_f32(13.5),
                y: Pt::from_f32(45.0),
                glyph_ids: vec![glyph_id, glyph_id, glyph_id],
                advances: vec![(advance, Pt::ZERO); 3],
                offsets: vec![(Pt::ZERO, Pt::ZERO); 3],
                stroke_width: Pt::from_milli_i64(656),
            },
        ]);

        let bytes = document_to_pdf_with_metrics_and_registry(
            &doc,
            None,
            Some(&registry),
            &PdfOptions::default(),
        )
        .expect("render reusable synthetic-bold glyph run");
        let pdf = String::from_utf8_lossy(&bytes);
        let content = page_content_bytes(&bytes);
        assert!(pdf.contains("/Subtype /Type3"));
        assert!(pdf.contains("31.238 w"));
        assert!(pdf.contains("B\n"));
        assert_eq!(count_token(&content, b" Tj"), 3);
        // CharProc and Encoding each name the glyph once; repeated page draws
        // reference that shared program instead of expanding its outline.
        assert_eq!(
            count_token(&bytes, format!("/g{:04X}", glyph_id).as_bytes()),
            2
        );
    }

    #[test]
    fn pdfua1_emits_pdfua_xmp_and_tagged_structure() {
        let inter_path = repo_font_path("Inter-Variable.ttf");
        let inter_bytes = std::fs::read(&inter_path).expect("read inter");
        let mut registry = FontRegistry::new();
        let inter_name = registry
            .register_bytes(inter_bytes, Some(inter_path.to_string_lossy().as_ref()))
            .expect("register inter");
        let doc = one_page_document(vec![
            Command::BeginTag {
                role: "P".to_string(),
                mcid: Some(0),
                alt: None,
                scope: None,
                table_id: None,
                col_index: None,
                group_only: false,
            },
            Command::SetFontName(inter_name),
            Command::SetFontSize(Pt::from_f32(12.0)),
            Command::DrawString {
                x: Pt::from_f32(72.0),
                y: Pt::from_f32(88.0),
                text: "Tagged PDF/UA seed".to_string(),
            },
            Command::EndTag,
        ]);
        let mut options = PdfOptions::default();
        options.pdf_profile = PdfProfile::PdfUa1;
        options.document_lang = Some("en-US".to_string());
        options.document_title = Some("PDF/UA seed".to_string());

        let bytes =
            document_to_pdf_with_metrics_and_registry(&doc, None, Some(&registry), &options)
                .expect("pdf/ua seed bytes");
        let pdf = String::from_utf8_lossy(&bytes);
        assert!(pdf.contains("pdfuaid:part=\"1\""));
        assert!(pdf.contains("/Type /Metadata /Subtype /XML"));
        assert!(
            pdf.contains("<dc:language><rdf:Bag><rdf:li>en-US</rdf:li></rdf:Bag></dc:language>")
        );
        assert!(pdf.contains("/StructTreeRoot"));
        assert!(pdf.contains("/MarkInfo << /Marked true >>"));
        assert!(pdf.contains("/Lang (en-US)"));
        assert!(pdf.contains("/FontFile2"));
    }

    #[test]
    fn pdfua1_requires_embedded_fonts_for_text() {
        let doc = text_page("Helvetica", "PDF/UA-1 requires embedded text fonts");
        let mut options = PdfOptions::default();
        options.pdf_profile = PdfProfile::PdfUa1;
        options.document_lang = Some("en-US".to_string());
        options.document_title = Some("PDF/UA seed".to_string());

        let err = document_to_pdf_with_metrics_and_registry(&doc, None, None, &options)
            .expect_err("pdf/ua-1 text should fail without an embedded font registry");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("pdfua1 requires a font registry"));
    }

    #[test]
    fn wtpdf_profiles_emit_pdf_declarations_and_pdf20_structure() {
        let doc = one_page_document(vec![]);
        for (profile, declaration) in [
            (
                PdfProfile::Wtpdf1r,
                "http://pdfa.org/declarations/wtpdf#reuse1.0",
            ),
            (
                PdfProfile::Wtpdf1a,
                "http://pdfa.org/declarations/wtpdf#accessibility1.0",
            ),
        ] {
            let mut options = PdfOptions::default();
            options.pdf_profile = profile;
            options.document_lang = Some("en-US".to_string());
            options.document_title = Some("WTPDF seed".to_string());

            let bytes = document_to_pdf_with_metrics_and_registry(&doc, None, None, &options)
                .expect("wtpdf seed bytes");
            let pdf = String::from_utf8_lossy(&bytes);
            assert!(pdf.starts_with("%PDF-2.0"));
            assert!(pdf.contains("<pdfd:declarations>"));
            assert!(pdf.contains(declaration));
            assert!(pdf.contains("/StructTreeRoot"));
            assert!(pdf.contains("/MarkInfo << /Marked true >>"));
            assert!(pdf.contains("/S /Document"));
            assert!(pdf.contains("/NS (http://iso.org/pdf2/ssn)"));
            assert!(pdf.contains("/Lang (en-US)"));
        }
    }

    #[test]
    fn pdfvt1_emits_pdfx_and_pdfvt_metadata_deterministically() {
        let doc = one_page_document(vec![]);
        let mut options = PdfOptions::default();
        options.pdf_profile = PdfProfile::PdfVt1;
        options.output_intent = Some(OutputIntent::new(
            vec![0x00, 0x01, 0x02],
            3,
            "sRGB IEC61966-2.1",
            Some("sRGB".to_string()),
        ));

        let first = document_to_pdf_with_metrics_and_registry(&doc, None, None, &options).unwrap();
        let second = document_to_pdf_with_metrics_and_registry(&doc, None, None, &options).unwrap();
        assert_eq!(first, second);

        let pdf = String::from_utf8_lossy(&first);
        assert!(pdf.contains("<pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion>"));
        assert!(pdf.contains("pdfvtid:GTS_PDFVTVersion=\"PDF/VT-1\""));
        assert!(pdf.contains("pdfvtid:GTS_PDFVTModDate=\"1970-01-01T00:00:00Z\""));
        assert!(pdf.contains("xmp:ModifyDate=\"1970-01-01T00:00:00Z\""));
        assert!(pdf.contains("/Type /Metadata /Subtype /XML"));
        assert!(pdf.contains("/S /GTS_PDFX"));
        assert!(pdf.contains("/GTS_PDFVTVersion (PDF/VT-1)"));
        assert!(pdf.contains("/DPartRoot"));
        assert!(pdf.contains("/DPartRootNode"));
        assert!(pdf.contains("/NodeNameList [/Document]"));
        assert!(pdf.contains("/Type /DPart"));
        assert!(pdf.contains("/DPart "));
        assert!(pdf.contains("/ID [<"));
    }

    #[test]
    fn profile_debug_log_reports_deterministic_profile_state() {
        let doc = one_page_document(vec![]);
        let mut options = PdfOptions::default();
        options.pdf_profile = PdfProfile::PdfVt1;
        options.output_intent = Some(OutputIntent::new(
            vec![0x00, 0x01, 0x02],
            3,
            "sRGB IEC61966-2.1",
            Some("sRGB".to_string()),
        ));
        let path = temp_log_path("pdf_profile_debug");
        let debug = Arc::new(crate::debug::DebugLogger::new(&path).expect("debug logger"));

        let _ = document_to_pdf_with_metrics_and_registry_with_logs(
            &doc,
            None,
            None,
            &options,
            Some(debug.clone()),
            None,
        )
        .expect("pdf bytes");
        debug.flush();
        drop(debug);

        let log = std::fs::read_to_string(&path).expect("read debug log");
        assert!(log.contains("\"type\":\"jit.pdf_profile\""));
        assert!(log.contains("\"pdf_profile\":\"pdfvt1\""));
        assert!(log.contains("\"metadata\":true"));
        assert!(log.contains("\"output_intent\":true"));
        assert!(log.contains("\"pdfvt_dpart_root\":true"));
        assert!(log.contains("\"requires_embedded_fonts\":true"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn ocg_and_artifact_marked_content_emit_tokens() {
        let doc = one_page_document(vec![
            Command::BeginOptionalContent {
                name: "WM".to_string(),
            },
            Command::BeginArtifact {
                subtype: Some("Watermark".to_string()),
            },
            Command::DrawRect {
                x: Pt::from_f32(12.0),
                y: Pt::from_f32(16.0),
                width: Pt::from_f32(40.0),
                height: Pt::from_f32(20.0),
            },
            Command::EndMarkedContent,
            Command::EndMarkedContent,
        ]);

        let bytes =
            document_to_pdf_with_metrics_and_registry(&doc, None, None, &PdfOptions::default())
                .unwrap();
        let pdf = String::from_utf8_lossy(&bytes);
        assert!(pdf.contains("/OCProperties"));
        assert!(pdf.contains("/Type /OCG"));
        assert!(pdf.contains("/Properties << /WM"));
        assert!(count_page_content_token(&bytes, b"/Artifact <</Subtype /Watermark>> BDC") > 0);
        assert!(count_page_content_token(&bytes, b"/OC /WM BDC") > 0);
    }

    #[test]
    fn image_xobject_reused_across_pages_for_same_source() {
        let image_source = "examples/img/full_bleed-logo_small.png".to_string();
        let image_cmd = |resource_id: String| Command::DrawImage {
            x: Pt::from_f32(12.0),
            y: Pt::from_f32(16.0),
            width: Pt::from_f32(60.0),
            height: Pt::from_f32(30.0),
            resource_id,
            interpolate: true,
            source_clip: None,
        };

        let doc_one = Document {
            page_size: Size::a4(),
            pages: vec![Page {
                commands: vec![image_cmd(image_source.clone())],
            }],
        };
        let doc_multi = Document {
            page_size: Size::a4(),
            pages: vec![
                Page {
                    commands: vec![image_cmd(image_source.clone())],
                },
                Page {
                    commands: vec![image_cmd(image_source)],
                },
            ],
        };

        let pdf_one =
            document_to_pdf_with_metrics_and_registry(&doc_one, None, None, &PdfOptions::default())
                .unwrap();
        let pdf_multi = document_to_pdf_with_metrics_and_registry(
            &doc_multi,
            None,
            None,
            &PdfOptions::default(),
        )
        .unwrap();

        // The same image source should embed once and be drawn on every page.
        let image_objs_one = count_token(&pdf_one, b"/Subtype /Image");
        let image_objs_multi = count_token(&pdf_multi, b"/Subtype /Image");
        let draws_one = count_page_content_token(&pdf_one, b"/Im1 Do");
        let draws_multi = count_page_content_token(&pdf_multi, b"/Im1 Do");

        assert!(image_objs_one > 0);
        assert_eq!(image_objs_one, image_objs_multi);
        assert_eq!(draws_one, 1);
        assert_eq!(draws_multi, 2);
    }

    #[test]
    fn form_xobject_emitted_in_writer_path() {
        let form_id = "wm-form".to_string();
        let doc = one_page_document(vec![
            Command::DefineForm {
                resource_id: form_id.clone(),
                width: Pt::from_f32(64.0),
                height: Pt::from_f32(24.0),
                commands: vec![
                    Command::SetFillColor(Color {
                        r: 1.0,
                        g: 0.0,
                        b: 0.0,
                    }),
                    Command::DrawRect {
                        x: Pt::from_f32(0.0),
                        y: Pt::from_f32(0.0),
                        width: Pt::from_f32(64.0),
                        height: Pt::from_f32(24.0),
                    },
                ],
            },
            Command::DrawForm {
                x: Pt::from_f32(72.0),
                y: Pt::from_f32(88.0),
                width: Pt::from_f32(64.0),
                height: Pt::from_f32(24.0),
                resource_id: form_id,
            },
        ]);

        let mut bytes = Vec::new();
        let written = document_to_pdf_with_metrics_and_registry_to_writer(
            &doc,
            None,
            None,
            &PdfOptions::default(),
            &mut bytes,
        )
        .unwrap();
        assert_eq!(written, bytes.len());

        let pdf = String::from_utf8_lossy(&bytes);
        assert!(pdf.contains("/Subtype /Form"));
        assert!(pdf.contains("/XObject"));
        assert!(count_page_content_token(&bytes, b"/Fm1 Do") > 0);
    }

    #[test]
    fn form_xobject_emitted_in_buffer_path() {
        let form_id = "wm-form".to_string();
        let doc = one_page_document(vec![
            Command::DefineForm {
                resource_id: form_id.clone(),
                width: Pt::from_f32(50.0),
                height: Pt::from_f32(20.0),
                commands: vec![Command::DrawRect {
                    x: Pt::from_f32(0.0),
                    y: Pt::from_f32(0.0),
                    width: Pt::from_f32(50.0),
                    height: Pt::from_f32(20.0),
                }],
            },
            Command::DrawForm {
                x: Pt::from_f32(40.0),
                y: Pt::from_f32(40.0),
                width: Pt::from_f32(50.0),
                height: Pt::from_f32(20.0),
                resource_id: form_id,
            },
        ]);

        let bytes =
            document_to_pdf_with_metrics_and_registry(&doc, None, None, &PdfOptions::default())
                .unwrap();
        let pdf = String::from_utf8_lossy(&bytes);
        assert!(pdf.contains("/Subtype /Form"));
        assert!(count_page_content_token(&bytes, b"/Fm1 Do") > 0);
    }

    #[test]
    fn isolated_form_xobject_emits_transparency_group() {
        let form_id = "isolated-form".to_string();
        let doc = one_page_document(vec![
            Command::DefineIsolatedForm {
                resource_id: form_id.clone(),
                width: Pt::from_f32(50.0),
                height: Pt::from_f32(20.0),
                commands: vec![Command::DrawRect {
                    x: Pt::from_f32(0.0),
                    y: Pt::from_f32(0.0),
                    width: Pt::from_f32(50.0),
                    height: Pt::from_f32(20.0),
                }],
            },
            Command::DrawForm {
                x: Pt::from_f32(40.0),
                y: Pt::from_f32(40.0),
                width: Pt::from_f32(50.0),
                height: Pt::from_f32(20.0),
                resource_id: form_id,
            },
        ]);

        let bytes =
            document_to_pdf_with_metrics_and_registry(&doc, None, None, &PdfOptions::default())
                .unwrap();
        let pdf = String::from_utf8_lossy(&bytes);
        assert!(pdf.contains("/Subtype /Form"));
        assert!(pdf.contains("/Group << /S /Transparency /I true /K false >>"));
    }

    #[test]
    fn large_page_content_stream_is_flate_compressed_by_default() {
        let mut commands = Vec::new();
        commands.push(Command::SetFontName("Helvetica".to_string()));
        commands.push(Command::SetFontSize(Pt::from_f32(10.0)));
        for i in 0..240 {
            commands.push(Command::DrawString {
                x: Pt::from_f32(36.0),
                y: Pt::from_f32(36.0 + (i as f32)),
                text: format!("compression_probe_line_{}", i),
            });
        }
        let doc = one_page_document(commands);

        let bytes =
            document_to_pdf_with_metrics_and_registry(&doc, None, None, &PdfOptions::default())
                .expect("pdf bytes");
        let filter = first_page_content_filter(&bytes).expect("filter present");
        assert_eq!(filter.as_slice(), b"FlateDecode");
    }

    #[test]
    fn content_stream_compression_can_be_threshold_disabled() {
        let doc = one_page_document(vec![Command::DrawRect {
            x: Pt::from_f32(1.0),
            y: Pt::from_f32(2.0),
            width: Pt::from_f32(3.0),
            height: Pt::from_f32(4.0),
        }]);
        let mut options = PdfOptions::default();
        options.compress_content_stream_min_bytes = usize::MAX;

        let bytes = document_to_pdf_with_metrics_and_registry(&doc, None, None, &options)
            .expect("pdf bytes");
        assert!(first_page_content_filter(&bytes).is_none());
    }

    #[test]
    fn source_clipped_image_embeds_only_enclosing_source_pixels() {
        let mut source = crate::image_native::RgbaImage::new(8, 4);
        for y in 0..4 {
            for x in 0..8 {
                source.put_pixel(
                    x,
                    y,
                    crate::image_native::Rgba(if x < 4 {
                        [255, 0, 0, 255]
                    } else {
                        [0, 255, 0, 255]
                    }),
                );
            }
        }
        let png = crate::image_native::encode_png_rgba8(source.as_bytes(), 8, 4)
            .expect("encode source image");
        let data_uri = format!(
            "data:image/png;base64,{}",
            crate::base64::encode_standard(png)
        );
        let doc = one_page_document(vec![Command::DrawImage {
            x: Pt::from_f32(400.2),
            y: Pt::from_f32(20.0),
            width: Pt::from_f32(48.0),
            height: Pt::from_f32(24.0),
            resource_id: data_uri,
            interpolate: true,
            source_clip: Some(ImageSourceClip {
                left: Pt::from_f32(9.8),
                top: Pt::ZERO,
                right: Pt::from_f32(43.8),
                bottom: Pt::from_f32(24.0),
                snap_target_origin_to_css_pixel: false,
            }),
        }]);

        let bytes =
            document_to_pdf_with_metrics_and_registry(&doc, None, None, &PdfOptions::default())
                .expect("pdf bytes");
        assert!(
            bytes
                .windows(b"/Width 7 /Height 4".len())
                .any(|window| window == b"/Width 7 /Height 4"),
            "expected the hidden first source column to be omitted"
        );
        assert!(
            !bytes
                .windows(b"/Width 8 /Height 4".len())
                .any(|window| window == b"/Width 8 /Height 4")
        );
    }

    #[test]
    fn image_streams_emit_binary_filters_without_asciihex() {
        let image_source = "examples/img/full_bleed-logo_small.png".to_string();
        let doc = one_page_document(vec![Command::DrawImage {
            x: Pt::from_f32(12.0),
            y: Pt::from_f32(16.0),
            width: Pt::from_f32(60.0),
            height: Pt::from_f32(30.0),
            resource_id: image_source,
            interpolate: true,
            source_clip: None,
        }]);

        let bytes =
            document_to_pdf_with_metrics_and_registry(&doc, None, None, &PdfOptions::default())
                .expect("pdf bytes");
        assert!(count_token(&bytes, b"/Subtype /Image") > 0);
        assert!(count_token(&bytes, b"/Filter /FlateDecode") > 0);
        assert_eq!(count_token(&bytes, b"/ASCIIHexDecode"), 0);
    }

    #[test]
    fn embedded_font_streams_emit_without_asciihex() {
        let inter_path = repo_font_path("Inter-Variable.ttf");
        let inter_bytes = std::fs::read(&inter_path).expect("read inter");

        let mut registry = FontRegistry::new();
        let inter_name = registry
            .register_bytes(inter_bytes, Some(inter_path.to_string_lossy().as_ref()))
            .expect("register inter");
        let doc = text_page(&inter_name, "Font binary stream check");

        let bytes = document_to_pdf_with_metrics_and_registry(
            &doc,
            None,
            Some(&registry),
            &PdfOptions::default(),
        )
        .expect("pdf bytes");
        assert_eq!(count_token(&bytes, b"/FontFile2"), 1);
        assert_eq!(count_token(&bytes, b"/ASCIIHexDecode"), 0);
    }

    #[test]
    fn static_truetype_fonts_are_subset_named_compressed_and_parseable() {
        let source =
            include_bytes!("../python/fullbleed_assets/fonts/NotoSansMath-Regular.ttf").to_vec();
        let mut registry = FontRegistry::new();
        let name = registry
            .register_bytes(source.clone(), Some("NotoSansMath-Regular.ttf"))
            .expect("register static TrueType");
        let doc = text_page(&name, "Subset ABC xyz 0123");

        let bytes = document_to_pdf_with_metrics_and_registry(
            &doc,
            None,
            Some(&registry),
            &PdfOptions::default(),
        )
        .expect("subset PDF");
        let pdf = String::from_utf8_lossy(&bytes);
        let marker = "+NotoSansMath-Regular";
        let marker_offset = pdf.find(marker).expect("subset base font name");
        let tag = pdf
            .get(marker_offset.saturating_sub(6)..marker_offset)
            .expect("six-character subset tag");
        assert_eq!(tag.len(), 6);
        assert!(tag.bytes().all(|byte| byte.is_ascii_uppercase()));

        let parsed_pdf = LoDocument::load_mem(&bytes).expect("parse PDF");
        let font_stream = parsed_pdf
            .objects
            .values()
            .filter_map(|object| match object {
                LoObject::Stream(stream) if stream.dict.get(b"Length1").is_ok() => Some(stream),
                _ => None,
            })
            .find(|stream| {
                stream
                    .get_plain_content()
                    .is_ok_and(|data| data.starts_with(b"\0\x01\0\0"))
            })
            .expect("embedded TrueType stream");
        assert_eq!(
            font_stream.filters().expect("font filters"),
            vec![b"FlateDecode".as_slice()]
        );
        let program = font_stream
            .get_plain_content()
            .expect("inflate font program");
        assert!(program.len() < source.len() / 2);
        assert!(SfntFace::parse(&program, 0).is_ok());
        assert!(bytes.len() < source.len() / 2);
    }

    #[test]
    fn icc_stream_emits_without_asciihex() {
        let doc = one_page_document(vec![]);
        let mut options = PdfOptions::default();
        options.pdf_profile = PdfProfile::PdfX4;
        options.output_intent = Some(OutputIntent::new(
            vec![0x00, 0x01, 0x02, 0x03],
            3,
            "sRGB IEC61966-2.1",
            Some("sRGB".to_string()),
        ));

        let bytes = document_to_pdf_with_metrics_and_registry(&doc, None, None, &options)
            .expect("pdf bytes");
        assert!(count_token(&bytes, b"/OutputIntents") > 0);
        assert!(count_token(&bytes, b"/DestOutputProfile") > 0);
        assert_eq!(count_token(&bytes, b"/ASCIIHexDecode"), 0);
    }

    #[test]
    fn pdf_link_perf_reports_content_stream_compression_counters() {
        let mut commands = Vec::new();
        commands.push(Command::SetFontName("Helvetica".to_string()));
        commands.push(Command::SetFontSize(Pt::from_f32(10.0)));
        for i in 0..220 {
            commands.push(Command::DrawString {
                x: Pt::from_f32(40.0),
                y: Pt::from_f32(40.0 + i as f32),
                text: format!("perf_counter_probe_line_{}", i),
            });
        }
        let doc = one_page_document(commands);
        let path = temp_log_path("pdf_link_content_stream_perf");
        let perf = Arc::new(crate::perf::PerfLogger::new(&path).expect("perf logger"));

        let mut writer = Vec::new();
        let _ = document_to_pdf_with_metrics_and_registry_to_writer_with_logs(
            &doc,
            None,
            None,
            &PdfOptions::default(),
            &mut writer,
            None,
            Some(perf.clone()),
        )
        .expect("pdf write");
        perf.flush();
        drop(perf);

        let log = std::fs::read_to_string(&path).expect("read perf log");
        assert!(log.contains("\"name\":\"pdf.link\""));
        assert!(log.contains("\"content_stream_raw_bytes\""));
        assert!(log.contains("\"content_stream_encoded_bytes\""));
        assert!(log.contains("\"content_stream_compressed\""));
        assert!(log.contains("\"content_stream_ratio_ppm\""));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn streaming_writer_dedupes_same_embedded_font_across_documents() {
        let inter_path = repo_font_path("Inter-Variable.ttf");
        let inter_bytes = std::fs::read(&inter_path).expect("read inter");

        let mut registry = FontRegistry::new();
        let inter_name = registry
            .register_bytes(inter_bytes, Some(inter_path.to_string_lossy().as_ref()))
            .expect("register inter");

        let doc_a = text_page(&inter_name, "Record A");
        let doc_b = text_page(&inter_name, "Record B");

        let mut out = Vec::new();
        let mut stream = PdfStreamWriter::new(
            &mut out,
            Size::a4(),
            Some(&registry),
            PdfOptions::default(),
            None,
            None,
        )
        .expect("stream writer");
        stream.add_document(0, &doc_a).expect("add doc a");
        stream.add_document(1, &doc_b).expect("add doc b");
        let written = stream.finish().expect("finish stream");
        assert_eq!(written, out.len());

        // One embedded TrueType program and one Type0 wrapper for the shared font.
        assert_eq!(count_token(&out, b"/FontFile2"), 1);
        assert_eq!(count_token(&out, b"/Subtype /Type0"), 1);
    }

    #[test]
    fn streaming_writer_keeps_distinct_embedded_fonts_distinct() {
        let inter_path = repo_font_path("Inter-Variable.ttf");
        let noto_path = repo_font_path("NotoSans-Regular.ttf");
        let inter_bytes = std::fs::read(&inter_path).expect("read inter");
        let noto_bytes = std::fs::read(&noto_path).expect("read noto");

        let mut registry = FontRegistry::new();
        let inter_name = registry
            .register_bytes(inter_bytes, Some(inter_path.to_string_lossy().as_ref()))
            .expect("register inter");
        let noto_name = registry
            .register_bytes(noto_bytes, Some(noto_path.to_string_lossy().as_ref()))
            .expect("register noto");

        let doc_a = text_page(&inter_name, "Inter sample");
        let doc_b = text_page(&noto_name, "Noto sample");

        let mut out = Vec::new();
        let mut stream = PdfStreamWriter::new(
            &mut out,
            Size::a4(),
            Some(&registry),
            PdfOptions::default(),
            None,
            None,
        )
        .expect("stream writer");
        stream.add_document(0, &doc_a).expect("add doc a");
        stream.add_document(1, &doc_b).expect("add doc b");
        let written = stream.finish().expect("finish stream");
        assert_eq!(written, out.len());

        // Two distinct embedded font programs for two distinct logical fonts.
        assert_eq!(count_token(&out, b"/FontFile2"), 2);
    }

    #[test]
    fn winansi_fallback_emits_font_fallback_known_loss() {
        let doc = one_page_document(vec![
            Command::SetFontName("Helvetica".to_string()),
            Command::SetFontSize(Pt::from_f32(12.0)),
            Command::DrawString {
                x: Pt::from_f32(72.0),
                y: Pt::from_f32(72.0),
                text: "A \u{2265} B and C \u{2264} D".to_string(),
            },
        ]);
        let mut options = PdfOptions::default();
        options.unicode_support = false;

        let path = temp_log_path("winansi_fallback");
        let logger = Arc::new(crate::debug::DebugLogger::new(&path).expect("debug logger"));
        let _ = document_to_pdf_with_metrics_and_registry_with_logs(
            &doc,
            None,
            None,
            &options,
            Some(logger.clone()),
            None,
        )
        .expect("pdf bytes");
        logger.flush();
        drop(logger);

        let log = std::fs::read_to_string(&path).expect("read debug log");
        assert!(log.contains("\"pdf.winansi.fallback\""));
        assert!(log.contains("\"FONT_FALLBACK_USED\""));
        let _ = std::fs::remove_file(path);
    }
}
