use crate::canvas::{Command, Document, Page};
use crate::debug::json_escape;
use crate::font::{FontProgramKind, FontRegistry, RegisteredFont};
use crate::metrics::{DocumentMetrics, PageMetrics};
use crate::perf::PerfLogger;
use crate::types::{Color, ColorSpace, Pt, Shading, ShadingStop, Size};
use base64::Engine;
use fixed::types::I32F32;
use image::GenericImageView;
use rustybuzz::{
    Face as HbFace, Language as HbLanguage, Script as HbScript, ShapePlan, UnicodeBuffer,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{self, Write};
use std::path::Path;

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
    PdfA2b,
    PdfX4,
    Tagged,
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
    face: Option<HbFace<'a>>,
    plans: HashMap<(rustybuzz::Direction, HbScript, Option<HbLanguage>), ShapePlan>,
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
    current_doc_id: usize,

    image_resources: Vec<(String, usize)>,
    image_name_map: HashMap<String, String>,
    image_content_map: HashMap<u64, (String, usize)>,
    next_image_index: usize,
    image_bytes_total: usize,

    form_resources: Vec<(String, usize)>,
    form_name_map: HashMap<String, String>,
    form_content_map: HashMap<u64, (String, usize)>,
    form_size_map: HashMap<String, Size>,
    next_form_index: usize,

    gs_resources: Vec<(String, usize)>,
    gs_name_map: HashMap<(u16, u16), String>,
    next_gs_index: usize,

    shading_resources: Vec<(String, usize)>,
    shading_name_map: HashMap<u64, String>,
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
        validate_pdfx4_output_intent(&options)?;
        let mut offset: usize = 0;
        write_bytes(writer, pdf_header_bytes(options.pdf_version), &mut offset)?;
        write_bytes(writer, b"%\xE2\xE3\xCF\xD3\n", &mut offset)?;

        let s = Self {
            writer,
            offset,
            offsets: vec![0; PDF_RESOURCES_ID + 1],
            next_id: PDF_RESOURCES_ID + 1,
            page_size,
            options,
            registry,
            debug,
            perf,
            fonts: BTreeMap::new(),
            next_font_resource: 1,
            current_doc_id: 0,
            image_resources: Vec::new(),
            image_name_map: HashMap::new(),
            image_content_map: HashMap::new(),
            next_image_index: 1,
            image_bytes_total: 0,
            form_resources: Vec::new(),
            form_name_map: HashMap::new(),
            form_content_map: HashMap::new(),
            form_size_map: HashMap::new(),
            next_form_index: 1,
            gs_resources: Vec::new(),
            gs_name_map: HashMap::new(),
            next_gs_index: 1,
            shading_resources: Vec::new(),
            shading_name_map: HashMap::new(),
            next_shading_index: 1,
            optional_content_names: BTreeSet::new(),
            page_nodes: Vec::new(),
            current_node: None,
            shaped_cache: HashMap::new(),
            tag_records: Vec::new(),
            page_ids: Vec::new(),
            page_content_bytes: Vec::new(),
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
        validate_pdfx4_font_embedding(document, self.registry, &self.options)?;
        self.current_doc_id = doc_id;
        self.shaped_cache.clear();
        for page in &document.pages {
            self.add_page(page)?;
        }
        Ok(())
    }

    fn add_page(&mut self, page: &Page) -> io::Result<()> {
        let page_index = self.page_ids.len();
        let parent_id = self.ensure_page_node();
        let start = self.alloc_ids(2);
        let content_id = start;
        let page_id = start + 1;

        if let Some(node) = self.current_node.as_mut() {
            node.kids.push(page_id);
        }

        let content_stream = self.render_page(page, page_index)?;
        self.page_content_bytes
            .push(content_stream.as_bytes().len());
        self.write_object(content_id, &stream_object(&content_stream))?;
        self.page_ids.push(page_id);

        let (struct_parents, tabs) = if self.options.pdf_profile == PdfProfile::Tagged {
            (format!(" /StructParents {}", page_index), " /Tabs /S")
        } else {
            (String::new(), "")
        };
        let page_boxes = page_box_entries(self.options.pdf_profile, self.page_size);
        let page_obj = format!(
            "<< /Type /Page /Parent {} 0 R /MediaBox [0 0 {} {}]{} /Resources {} 0 R /Contents {} 0 R{}{} >>",
            parent_id,
            fmt_pt(self.page_size.width),
            fmt_pt(self.page_size.height),
            page_boxes,
            PDF_RESOURCES_ID,
            content_id,
            struct_parents,
            tabs
        );
        self.write_object(page_id, &page_obj)?;
        Ok(())
    }

    pub(crate) fn finish(&mut self) -> io::Result<usize> {
        let t_finish = std::time::Instant::now();
        if let Some(node) = self.current_node.take() {
            self.page_nodes.push(node);
        }

        // 1) Fonts (some objects were allocated early but not written yet).
        let fonts = std::mem::take(&mut self.fonts);

        if let Some(logger) = self.debug.as_deref() {
            let mut doc_map: BTreeMap<usize, Vec<(String, usize)>> = BTreeMap::new();
            for (key, font_state) in &fonts {
                let doc_id = key
                    .splitn(2, "::")
                    .next()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                doc_map
                    .entry(doc_id)
                    .or_default()
                    .push((font_state.logical_name.clone(), font_state.glyph_map.len()));
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
                        let (objs, _font_id, _next) =
                            build_truetype_font_objects(font, font_state.start_id);
                        for (i, obj) in objs.iter().enumerate() {
                            self.write_object(font_state.start_id + i, obj)?;
                        }
                    }
                    StreamFontKind::TrueTypeIdentityH => {
                        let Some(font) = registry.resolve(&font_state.logical_name) else {
                            return Err(io::Error::new(
                                io::ErrorKind::NotFound,
                                format!("font not found in registry: {}", font_state.logical_name),
                            ));
                        };
                        let usage = FontUsage {
                            glyph_map: font_state.glyph_map.clone(),
                        };
                        let (objs, _type0_id, _glyph_map, _next) = build_cidfont_objects(
                            font,
                            registry,
                            Some(&usage),
                            font_state.start_id,
                        );
                        for (i, obj) in objs.iter().enumerate() {
                            self.write_object(font_state.start_id + i, obj)?;
                        }
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
        if self.options.pdf_profile == PdfProfile::Tagged {
            let tag_records = std::mem::take(&mut self.tag_records);
            let tag_count = tag_records.len();
            let start_id = self.alloc_ids(tag_count + 2);
            let parent_tree_id = start_id + tag_count;
            let root_id = start_id + tag_count + 1;

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
                    let parent_id = tag.parent.map(|p| start_id + p).unwrap_or(root_id);
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
            let root_obj = format!(
                "<< /Type /StructTreeRoot /K [{}] /ParentTree {} 0 R >>",
                kids, parent_tree_id
            );
            self.write_object(root_id, &root_obj)?;
            struct_tree_root_id = Some(root_id);
        }

        // 5) Compliance objects + Catalog.
        let mut metadata_id: Option<usize> = None;
        let mut output_intent_id: Option<usize> = None;
        let mut info_id: Option<usize> = None;
        let pdf_profile = self.options.pdf_profile;
        let doc_lang = self.options.document_lang.clone();
        let doc_title = self.options.document_title.clone();
        let output_intent = self.options.output_intent.clone();
        if pdf_profile != PdfProfile::None {
            if let Some(xmp) =
                build_xmp_metadata(pdf_profile, doc_lang.as_deref(), doc_title.as_deref())
            {
                let id = self.alloc_ids(1);
                self.write_object(id, &stream_object(&xmp))?;
                metadata_id = Some(id);
            }
            if let Some(oi) = output_intent.as_ref() {
                let icc_id = self.alloc_ids(1);
                self.write_object(
                    icc_id,
                    &icc_profile_object(&oi.icc_profile, oi.n_components),
                )?;
                let oi_id = self.alloc_ids(1);
                self.write_object(oi_id, &output_intent_object(oi, icc_id, pdf_profile))?;
                output_intent_id = Some(oi_id);
            }
        }

        let mut catalog = format!("<< /Type /Catalog /Pages {} 0 R", PDF_PAGES_ID);
        if let Some(lang) = doc_lang.as_deref() {
            catalog.push_str(&format!(" /Lang ({})", escape_pdf_string(lang)));
        }
        if doc_title.is_some() {
            catalog.push_str(" /ViewerPreferences << /DisplayDocTitle true >>");
        }
        if doc_title.is_some() || pdf_profile == PdfProfile::PdfX4 {
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
        trailer.push_str(&format!(" >>\nstartxref\n{}\n%%EOF", xref_start));
        write_str(self.writer, &trailer, &mut self.offset)?;

        let bytes_written = self.offset;
        let finish_ms = t_finish.elapsed().as_secs_f64() * 1000.0;
        if let Some(logger) = self.debug.as_deref() {
            let json = format!(
                "{{\"type\":\"jit.link\",\"ms\":{:.3},\"bytes\":{},\"pages\":{},\"fonts\":{},\"images\":{},\"forms\":{},\"shadings\":{},\"extgstates\":{},\"image_bytes\":{}}}",
                finish_ms,
                bytes_written,
                self.page_ids.len(),
                fonts.len(),
                self.image_resources.len(),
                self.form_resources.len(),
                self.shading_resources.len(),
                self.gs_resources.len(),
                self.image_bytes_total
            );
            logger.log_json(&json);
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
                ],
            );
        }
        Ok(bytes_written)
    }

    fn render_page(&mut self, page: &Page, page_index: usize) -> io::Result<String> {
        self.render_commands(&page.commands, self.page_size.height, Some(page_index))
    }

    fn render_commands(
        &mut self,
        commands: &[Command],
        page_height: Pt,
        page_index: Option<usize>,
    ) -> io::Result<String> {
        let mut out = String::new();
        let mut current_font_size = Pt::from_f32(12.0);
        let mut current_font_name = "Helvetica".to_string();
        let mut current_fill = Color::BLACK;
        let mut tag_stack: Vec<usize> = Vec::new();
        let tag_enabled = self.options.pdf_profile == PdfProfile::Tagged && page_index.is_some();

        for cmd in commands {
            match cmd {
                Command::SaveState => out.push_str("q\n"),
                Command::RestoreState => out.push_str("Q\n"),
                Command::Translate(x, y) => {
                    out.push_str(&format!("1 0 0 1 {} {} cm\n", fmt_pt(*x), fmt_pt(*y)));
                }
                Command::Scale(x, y) => {
                    out.push_str(&format!("{} 0 0 {} 0 0 cm\n", fmt(*x), fmt(*y)));
                }
                Command::Rotate(angle) => {
                    let sin = libm::sinf(*angle);
                    let cos = libm::cosf(*angle);
                    out.push_str(&format!(
                        "{} {} {} {} 0 0 cm\n",
                        fmt(cos),
                        fmt(sin),
                        fmt(-sin),
                        fmt(cos)
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
                Command::SetFontName(name) => {
                    current_font_name = name.clone();
                    self.ensure_font(&current_font_name)?;
                }
                Command::SetFontSize(size) => {
                    current_font_size = *size;
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
                    let key = hash_shading(shading);
                    if let Some(name) = self.ensure_shading(key, shading)? {
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
                } => {
                    if let Some(name) = self.ensure_image(resource_id)? {
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
                    let _ = self.ensure_form(resource_id, *width, *height, commands);
                }
                Command::DrawForm {
                    x,
                    y,
                    width,
                    height,
                    resource_id,
                } => {
                    if let Some(name) = self.form_name_map.get(resource_id) {
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

    fn font_key(&self, name: &str) -> String {
        format!("{}::{}", self.current_doc_id, name)
    }

    fn ensure_font(&mut self, name: &str) -> io::Result<()> {
        let key = self.font_key(name);
        if self.fonts.contains_key(&key) {
            return Ok(());
        }

        let resource = format!("F{}", self.next_font_resource);
        self.next_font_resource += 1;

        let mut kind = StreamFontKind::Type1;
        let mut encoding = FontEncoding::WinAnsi;
        let mut face = None;

        if self.options.pdf_profile == PdfProfile::PdfX4 {
            let Some(registry) = self.registry else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "pdfx4 requires a font registry for embedded font resolution",
                ));
            };
            let Some(font) = registry.resolve(name) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "pdfx4 requires embedded fonts; unresolved font '{}'. register an embeddable font asset.",
                        name
                    ),
                ));
            };
            if self.options.unicode_support
                && matches!(font.program_kind, FontProgramKind::TrueType)
            {
                kind = StreamFontKind::TrueTypeIdentityH;
                encoding = FontEncoding::IdentityH;
                face = HbFace::from_slice(&font.data, 0);
            } else {
                kind = StreamFontKind::TrueTypeWinAnsi;
                encoding = FontEncoding::WinAnsi;
            }
        } else {
            // Default to base14 fonts when possible for speed and portability.
            let base14 = is_base14_font(name);
            if !base14 && self.options.unicode_support {
                if let Some(registry) = self.registry {
                    if let Some(font) = registry.resolve(name) {
                        if matches!(font.program_kind, FontProgramKind::TrueType) {
                            kind = StreamFontKind::TrueTypeIdentityH;
                            encoding = FontEncoding::IdentityH;
                            face = HbFace::from_slice(&font.data, 0);
                        } else {
                            // OpenType CFF: keep WinAnsi for now (no full Unicode CFF path yet).
                            kind = StreamFontKind::TrueTypeWinAnsi;
                            encoding = FontEncoding::WinAnsi;
                        }
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
                logical_name: name.to_string(),
                resource,
                encoding,
                start_id,
                kind,
                glyph_map: BTreeMap::new(),
                face,
                plans: HashMap::new(),
            },
        );
        Ok(())
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
            self.write_object(mask_id, &image_smask_object(alpha))?;
        }
        self.write_object(obj_id, &image_object(&image, smask_id))?;
        self.image_resources.push((name.clone(), obj_id));
        self.image_name_map.insert(source.to_string(), name.clone());
        if self.options.reuse_xobjects {
            self.image_content_map.insert(hash, (name.clone(), obj_id));
        }
        Ok(Some(name))
    }

    fn ensure_form(
        &mut self,
        resource_id: &str,
        width: Pt,
        height: Pt,
        commands: &[Command],
    ) -> io::Result<Option<String>> {
        if let Some(name) = self.form_name_map.get(resource_id) {
            return Ok(Some(name.clone()));
        }

        let content = self.render_commands(commands, height, None)?;
        let hash = hash_bytes(content.as_bytes());
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

        let obj = format!(
            "<< /Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 {} {}] /Resources {} 0 R /Length {} >>\nstream\n{}\nendstream",
            fmt_pt(width),
            fmt_pt(height),
            PDF_RESOURCES_ID,
            content.len(),
            content
        );

        self.write_object(obj_id, &obj)?;
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

    fn ensure_shading(&mut self, key: u64, shading: &Shading) -> io::Result<Option<String>> {
        if let Some(name) = self.shading_name_map.get(&key) {
            return Ok(Some(name.clone()));
        }

        let name = format!("Sh{}", self.next_shading_index);
        self.next_shading_index += 1;

        let start_id = self.next_id;
        let (objs, sh_obj_id, new_next) = shading_to_objects(
            shading,
            start_id,
            self.page_size.height,
            self.options.color_space,
        );
        self.next_id = new_next;
        self.ensure_offsets_len(self.next_id);

        for (i, obj) in objs.iter().enumerate() {
            self.write_object(start_id + i, obj)?;
        }

        self.shading_resources.push((name.clone(), sh_obj_id));
        self.shading_name_map.insert(key, name.clone());
        Ok(Some(name))
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
                let face = font_state.face.as_ref()?;
                let shaped = shape_text_with_plans(face, &mut font_state.plans, text)?;
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

fn shape_text_with_plans(
    face: &HbFace<'_>,
    plans: &mut HashMap<(rustybuzz::Direction, HbScript, Option<HbLanguage>), ShapePlan>,
    text: &str,
) -> Option<ShapedText> {
    use rustybuzz::ttf_parser::GlyphId;

    let units_per_em = face.units_per_em().max(1);
    let scale = 1000.0 / units_per_em as f32;

    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();

    let dir = buffer.direction();
    let script = buffer.script();
    let lang = buffer.language();

    let plan = plans
        .entry((dir, script, lang.clone()))
        .or_insert_with(|| ShapePlan::new(face, dir, Some(script), lang.as_ref(), &[]));

    let output = rustybuzz::shape_with_plan(face, plan, buffer);
    let infos = output.glyph_infos();
    let positions = output.glyph_positions();
    if infos.is_empty() || infos.len() != positions.len() {
        return None;
    }

    // Build a map from glyph id -> source unicode string (cluster range).
    let mut boundaries: Vec<usize> = infos.iter().map(|g| g.cluster as usize).collect();
    boundaries.sort_unstable();
    boundaries.dedup();
    if boundaries.last().copied() != Some(text.len()) {
        boundaries.push(text.len());
    }

    let mut glyph_map: BTreeMap<u16, String> = BTreeMap::new();
    for info in infos {
        let start = (info.cluster as usize).min(text.len());
        let idx = match boundaries.binary_search(&start) {
            Ok(i) => i,
            Err(i) => i,
        };
        let end = boundaries
            .get(idx + 1)
            .copied()
            .unwrap_or(text.len())
            .min(text.len());
        if start < end {
            glyph_map
                .entry(info.glyph_id as u16)
                .or_insert_with(|| text[start..end].to_string());
        }
    }

    // Build a TJ array.
    let mut parts: Vec<String> = Vec::new();
    for (info, pos) in output
        .glyph_infos()
        .iter()
        .zip(output.glyph_positions().iter())
    {
        let gid = info.glyph_id as u16;
        if gid == 0 {
            continue;
        }

        let x_offset = (pos.x_offset as f32 * scale).round() as i32;
        if x_offset != 0 {
            parts.push(format!("{}", -x_offset));
        }
        parts.push(format!("<{:04X}>", gid));

        let adv_default = face.glyph_hor_advance(GlyphId(gid)).unwrap_or(0) as f32;
        let adv_default = (adv_default * scale).round() as i32;
        let adv_shaped = (pos.x_advance as f32 * scale).round() as i32;
        let adjust = adv_default - adv_shaped;
        if adjust != 0 {
            parts.push(format!("{}", adjust));
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
            Command::DrawString { .. } => {
                names.insert(current_font.clone());
            }
            Command::DefineForm {
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

fn validate_pdfx4_font_embedding(
    document: &Document,
    registry: Option<&FontRegistry>,
    options: &PdfOptions,
) -> io::Result<()> {
    if options.pdf_profile != PdfProfile::PdfX4 {
        return Ok(());
    }
    let used_fonts = collect_used_font_names(document);
    if used_fonts.is_empty() {
        return Ok(());
    }
    let Some(registry) = registry else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pdfx4 requires a font registry for embedded font resolution",
        ));
    };
    for name in used_fonts {
        if registry.resolve(&name).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "pdfx4 requires embedded fonts; unresolved font '{}'. register an embeddable font asset.",
                    name
                ),
            ));
        }
    }
    Ok(())
}

fn validate_pdfx4_output_intent(options: &PdfOptions) -> io::Result<()> {
    if options.pdf_profile != PdfProfile::PdfX4 {
        return Ok(());
    }
    let Some(intent) = options.output_intent.as_ref() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pdfx4 requires an output intent",
        ));
    };
    if intent.icc_profile.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pdfx4 output intent ICC profile cannot be empty",
        ));
    }
    if !matches!(intent.n_components, 1 | 3 | 4) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "pdfx4 output intent n_components must be one of 1, 3, or 4 (got {})",
                intent.n_components
            ),
        ));
    }
    if intent.identifier.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pdfx4 output intent identifier cannot be empty",
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
                Command::DrawString { text, .. } => {
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
fn build_font_objects(
    font_names: &[String],
    font_map: &mut BTreeMap<String, FontResource>,
    registry: Option<&FontRegistry>,
    start_id: usize,
    font_usage: &HashMap<String, FontUsage>,
    options: &PdfOptions,
) -> (
    Vec<String>,
    Vec<(String, usize)>,
    HashMap<String, BTreeMap<u16, String>>,
    usize,
) {
    let mut objects = Vec::new();
    let mut resources = Vec::new();
    let mut next_id = start_id;
    let mut glyph_maps: HashMap<String, BTreeMap<u16, String>> = HashMap::new();

    for name in font_names {
        let resource = font_map
            .get(name)
            .map(|v| v.resource.clone())
            .unwrap_or_else(|| "F1".to_string());
        if let Some(font) = registry.and_then(|registry| registry.resolve(name)) {
            if options.unicode_support && matches!(font.program_kind, FontProgramKind::TrueType) {
                let usage = font_usage.get(name);
                let reg = registry.expect("registry is Some when font is resolved");
                let (font_objects, font_id, glyph_map, new_next) =
                    build_cidfont_objects(font, reg, usage, next_id);
                objects.extend(font_objects);
                resources.push((resource.clone(), font_id));
                if let Some(entry) = font_map.get_mut(name) {
                    entry.encoding = FontEncoding::IdentityH;
                }
                glyph_maps.insert(name.clone(), glyph_map);
                next_id = new_next;
            } else {
                // Fallback to WinAnsi Type1/TrueType path for OpenType CFF for now.
                let (font_objects, font_id, new_next) = build_truetype_font_objects(font, next_id);
                objects.extend(font_objects);
                resources.push((resource.clone(), font_id));
                next_id = new_next;
            }
        } else {
            let font_id = next_id;
            objects.push(font_object(name));
            resources.push((resource.clone(), font_id));
            next_id += 1;
        }
    }

    (objects, resources, glyph_maps, next_id)
}

fn build_truetype_font_objects(
    font: &RegisteredFont,
    start_id: usize,
) -> (Vec<String>, usize, usize) {
    let font_file_id = start_id;
    let descriptor_id = start_id + 1;
    let font_id = start_id + 2;
    let font_file = font_file_object(&font.data, font.program_kind);
    let descriptor = font_descriptor_object(font, font_file_id);
    let font_object = truetype_font_object(font, descriptor_id);
    (
        vec![font_file, descriptor, font_object],
        font_id,
        start_id + 3,
    )
}

fn build_cidfont_objects(
    font: &RegisteredFont,
    registry: &FontRegistry,
    usage: Option<&FontUsage>,
    start_id: usize,
) -> (Vec<String>, usize, BTreeMap<u16, String>, usize) {
    let font_file_id = start_id;
    let descriptor_id = start_id + 1;
    let cid_font_id = start_id + 2;
    let to_unicode_id = start_id + 3;
    let type0_font_id = start_id + 4;

    let mut objects = Vec::new();
    objects.push(font_file_object(&font.data, font.program_kind));
    objects.push(font_descriptor_object(font, font_file_id));

    let mut glyph_map: BTreeMap<u16, String> =
        usage.map(|u| u.glyph_map.clone()).unwrap_or_default();
    if glyph_map.is_empty() {
        // Fallback: at least include space.
        let gid = registry.map_glyph_id_for_char(&font.name, ' ');
        if gid != 0 {
            glyph_map.insert(gid, " ".to_string());
        }
    }
    let used_gids: BTreeSet<u16> = glyph_map.keys().copied().collect();

    let mut w_entries: Vec<String> = Vec::new();
    for gid in &used_gids {
        let adv = registry.glyph_advance(&font.name, *gid);
        let width = if adv > 0 {
            adv
        } else {
            font.metrics.missing_width
        };
        w_entries.push(format!("{} [{}]", gid, width));
    }
    let w_array = if w_entries.is_empty() {
        String::new()
    } else {
        format!("/W [{}]", w_entries.join(" "))
    };

    let cid_font = format!(
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {} 0 R {} /CIDToGIDMap /Identity >>",
        sanitize_font_name(&font.name),
        descriptor_id,
        w_array
    );
    objects.push(cid_font);

    let to_unicode = to_unicode_cmap(&glyph_map);
    objects.push(stream_object(&to_unicode));

    let type0 = format!(
        "<< /Type /Font /Subtype /Type0 /BaseFont /{} /Encoding /Identity-H /DescendantFonts [{} 0 R] /ToUnicode {} 0 R >>",
        sanitize_font_name(&font.name),
        cid_font_id,
        to_unicode_id
    );
    objects.push(type0);

    (objects, type0_font_id, glyph_map, start_id + 5)
}

#[allow(dead_code)]
fn build_image_objects(
    sources: &[String],
    start_id: usize,
    reuse_xobjects: bool,
) -> (
    Vec<String>,
    Vec<(String, usize)>,
    HashMap<String, String>,
    usize,
) {
    let mut objects = Vec::new();
    let mut resources = Vec::new();
    let mut name_map = HashMap::new();
    let mut content_map: HashMap<u64, (String, usize)> = HashMap::new();
    let mut next_id = start_id;
    let mut image_index = 1usize;

    for source in sources {
        if let Some(image) = load_image(source) {
            let hash = hash_image(&image);
            if reuse_xobjects {
                if let Some((name, _obj_id)) = content_map.get(&hash) {
                    name_map.insert(source.clone(), name.clone());
                    continue;
                }
            }

            let smask_id = image.alpha.as_ref().map(|_| {
                let id = next_id;
                next_id += 1;
                id
            });
            let obj_id = next_id;
            next_id += 1;
            let name = format!("Im{}", image_index);
            image_index += 1;

            if let (Some(alpha), Some(mask_id)) = (image.alpha.as_ref(), smask_id) {
                objects.push(image_smask_object(alpha));
                objects.push(image_object(&image, Some(mask_id)));
            } else {
                objects.push(image_object(&image, None));
            }
            resources.push((name.clone(), obj_id));
            name_map.insert(source.clone(), name.clone());
            if reuse_xobjects {
                content_map.insert(hash, (name, obj_id));
            }
        }
    }

    (objects, resources, name_map, next_id)
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

fn load_image(source: &str) -> Option<ImageData> {
    if let Some((mime, data)) = parse_data_uri(source) {
        return decode_image_bytes(&data, Some(&mime));
    }

    let path = Path::new(source);
    let bytes = std::fs::read(path).ok()?;
    decode_image_bytes(&bytes, None)
}

fn decode_image_bytes(data: &[u8], mime: Option<&str>) -> Option<ImageData> {
    let format = if let Some(mime) = mime {
        if mime.contains("png") {
            Some(image::ImageFormat::Png)
        } else if mime.contains("jpeg") || mime.contains("jpg") {
            Some(image::ImageFormat::Jpeg)
        } else {
            None
        }
    } else {
        image::guess_format(data).ok()
    };

    let decoded = image::load_from_memory(data).ok()?;
    let (width, height) = decoded.dimensions();

    if matches!(format, Some(image::ImageFormat::Jpeg)) {
        let color_space = match decoded.color() {
            image::ColorType::L8 | image::ColorType::La8 => "/DeviceGray",
            _ => "/DeviceRGB",
        };
        return Some(ImageData {
            width,
            height,
            color_space,
            bits_per_component: 8,
            filter: "/DCTDecode",
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
        data: compressed,
        alpha,
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
        base64::engine::general_purpose::STANDARD
            .decode(data_part)
            .ok()?
    } else {
        data_part.as_bytes().to_vec()
    };
    Some((mime, data))
}

fn flate_compress(data: &[u8]) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::ZlibEncoder;

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    let _ = encoder.write_all(data);
    encoder.finish().unwrap_or_default()
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
    image.data.hash(&mut hasher);
    if let Some(alpha) = &image.alpha {
        alpha.data.hash(&mut hasher);
    }
    hasher.finish()
}

fn image_object(image: &ImageData, smask_id: Option<usize>) -> String {
    let stream_data = encode_stream_data(&image.data);
    let filters = match image.filter {
        "/DCTDecode" => "[/ASCIIHexDecode /DCTDecode]",
        _ => "[/ASCIIHexDecode /FlateDecode]",
    };
    let smask = smask_id
        .map(|id| format!(" /SMask {} 0 R", id))
        .unwrap_or_default();
    format!(
        "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {} /BitsPerComponent {} /Length {} /Filter {}{} >>
stream
{}
endstream",
        image.width,
        image.height,
        image.color_space,
        image.bits_per_component,
        stream_data.as_bytes().len(),
        filters,
        smask,
        stream_data
    )
}

fn image_smask_object(alpha: &AlphaData) -> String {
    let stream_data = encode_stream_data(&alpha.data);
    let filters = match alpha.filter {
        "/DCTDecode" => "[/ASCIIHexDecode /DCTDecode]",
        _ => "[/ASCIIHexDecode /FlateDecode]",
    };
    format!(
        "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent {} /Length {} /Filter {} >>
stream
{}
endstream",
        alpha.width,
        alpha.height,
        alpha.bits_per_component,
        stream_data.as_bytes().len(),
        filters,
        stream_data
    )
}

fn encode_stream_data(data: &[u8]) -> String {
    let mut hex = ascii_hex_encode(data);
    hex.push('>');
    hex
}

fn truetype_font_object(font: &RegisteredFont, descriptor_id: usize) -> String {
    let base = sanitize_font_name(&font.name);
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
        subtype, base, metrics.first_char, metrics.last_char, widths, descriptor_id, encoding
    )
}

fn font_descriptor_object(font: &RegisteredFont, font_file_id: usize) -> String {
    let base = sanitize_font_name(&font.name);
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
        base,
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

fn font_file_object(data: &[u8], kind: FontProgramKind) -> String {
    let hex = ascii_hex_encode(data);
    let mut stream_data = String::new();
    stream_data.push_str(&hex);
    stream_data.push('>');
    stream_data.push('\n');
    let length = stream_data.as_bytes().len();
    let mut dict = format!(
        "<< /Length {} /Length1 {} /Filter /ASCIIHexDecode",
        length,
        data.len()
    );
    if matches!(kind, FontProgramKind::OpenTypeCff) {
        dict.push_str(" /Subtype /OpenType");
    }
    dict.push_str(" >>\nstream\n");
    format!("{}{}endstream", dict, stream_data)
}

fn icc_profile_object(data: &[u8], n_components: u8) -> String {
    let hex = ascii_hex_encode(data);
    let mut stream_data = String::new();
    stream_data.push_str(&hex);
    stream_data.push('>');
    stream_data.push('\n');
    let length = stream_data.as_bytes().len();
    format!(
        "<< /N {} /Length {} /Filter /ASCIIHexDecode >>\nstream\n{}endstream",
        n_components, length, stream_data
    )
}

fn output_intent_object(oi: &OutputIntent, icc_id: usize, profile: PdfProfile) -> String {
    let subtype = match profile {
        PdfProfile::PdfX4 => "GTS_PDFX",
        _ => "GTS_PDFA1",
    };
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

fn ascii_hex_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 2);
    for (index, byte) in data.iter().enumerate() {
        use std::fmt::Write;
        let _ = write!(&mut out, "{:02X}", byte);
        if index % 32 == 31 {
            out.push('\n');
        }
    }
    out
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
    let mut tag_stack: Vec<usize> = Vec::new();

    for cmd in &page.commands {
        match cmd {
            Command::SaveState => out.push_str("q\n"),
            Command::RestoreState => out.push_str("Q\n"),
            Command::Translate(x, y) => {
                out.push_str(&format!("1 0 0 1 {} {} cm\n", fmt_pt(*x), fmt_pt(*y)));
            }
            Command::Scale(x, y) => {
                out.push_str(&format!("{} 0 0 {} 0 0 cm\n", fmt(*x), fmt(*y)));
            }
            Command::Rotate(angle) => {
                let sin = libm::sinf(*angle);
                let cos = libm::cosf(*angle);
                out.push_str(&format!(
                    "{} {} {} {} 0 0 cm\n",
                    fmt(cos),
                    fmt(sin),
                    fmt(-sin),
                    fmt(cos)
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
                if options.pdf_profile == PdfProfile::Tagged {
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
                if options.pdf_profile == PdfProfile::Tagged {
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
            Command::SetFontName(name) => {
                current_font_name = name.clone();
            }
            Command::SetFontSize(size) => {
                current_font_size = *size;
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
            Command::DrawForm { .. } => {}
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
        } => {
            2u8.hash(&mut hasher);
            hash_f32(&mut hasher, *x0);
            hash_f32(&mut hasher, *y0);
            hash_f32(&mut hasher, *r0);
            hash_f32(&mut hasher, *x1);
            hash_f32(&mut hasher, *y1);
            hash_f32(&mut hasher, *r1);
            hash_stops(&mut hasher, stops);
        }
    }
    hasher.finish()
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
    };

    let (fun_objects, fun_id, new_next) =
        build_gradient_function_objects(&stops, next_id, color_space);
    objects.extend(fun_objects);
    next_id = new_next;

    let sh_obj_id = next_id;
    next_id += 1;

    // Flip from our top-left coordinates into PDF user space.
    let matrix = format!("[1 0 0 -1 0 {}]", fmt_pt(page_height));

    let space = match color_space {
        ColorSpace::Rgb => "/DeviceRGB",
        ColorSpace::Cmyk => "/DeviceCMYK",
    };
    let sh_dict = match shading {
        Shading::Axial { x0, y0, x1, y1, .. } => format!(
            "<< /ShadingType 2 /ColorSpace {} /Coords [{} {} {} {}] /Function {} 0 R /Extend [true true] /Matrix {} >>",
            space,
            fmt(*x0),
            fmt(*y0),
            fmt(*x1),
            fmt(*y1),
            fun_id,
            matrix
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
            "<< /ShadingType 3 /ColorSpace {} /Coords [{} {} {} {} {} {}] /Function {} 0 R /Extend [true true] /Matrix {} >>",
            space,
            fmt(*x0),
            fmt(*y0),
            fmt(*r0),
            fmt(*x1),
            fmt(*y1),
            fmt(*r1),
            fun_id,
            matrix
        ),
    };
    objects.push(sh_dict);

    (objects, sh_obj_id, next_id)
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
        });
        stops.push(ShadingStop {
            offset: 1.0,
            color: Color::BLACK,
        });
    } else if stops.len() == 1 {
        stops.push(ShadingStop {
            offset: 1.0,
            color: stops[0].color,
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
            },
        );
    }
    if stops[stops.len() - 1].offset < 1.0 {
        let last = stops[stops.len() - 1].color;
        stops.push(ShadingStop {
            offset: 1.0,
            color: last,
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

fn stream_object(content: &str) -> String {
    let length = content.as_bytes().len();
    format!("<< /Length {} >>\nstream\n{}\nendstream", length, content)
}

fn page_box_entries(profile: PdfProfile, page_size: Size) -> String {
    if profile != PdfProfile::PdfX4 {
        return String::new();
    }
    format!(
        " /TrimBox [0 0 {} {}] /BleedBox [0 0 {} {}] /CropBox [0 0 {} {}]",
        fmt_pt(page_size.width),
        fmt_pt(page_size.height),
        fmt_pt(page_size.width),
        fmt_pt(page_size.height),
        fmt_pt(page_size.width),
        fmt_pt(page_size.height),
    )
}

fn info_object(title: Option<&str>, profile: PdfProfile) -> String {
    let mut entries: Vec<String> = Vec::new();
    if let Some(title) = title {
        entries.push(format!("/Title ({})", escape_pdf_string(title)));
    }
    if profile == PdfProfile::PdfX4 {
        entries.push("/GTS_PDFXVersion (PDF/X-4)".to_string());
        entries.push("/Trapped /False".to_string());
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

    match profile {
        PdfProfile::PdfA2b => {
            out.push_str("<rdf:Description xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" ");
            out.push_str("pdfaid:part=\"2\" pdfaid:conformance=\"B\"/>\n");
        }
        PdfProfile::PdfX4 => {
            out.push_str("<rdf:Description xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\" ");
            out.push_str("pdfxid:part=\"4\" pdfxid:GTS_PDFXVersion=\"PDF/X-4\"/>\n");
        }
        _ => {}
    }

    if let Some(lang) = lang {
        out.push_str("<rdf:Description xmlns:dc=\"http://purl.org/dc/elements/1.1/\">");
        out.push_str("<dc:language><rdf:Seq><rdf:li>");
        out.push_str(&escape_xml_text(lang));
        out.push_str("</rdf:li></rdf:Seq></dc:language></rdf:Description>\n");
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
    let face = HbFace::from_slice(font_data, 0)?;
    let mut buffer = UnicodeBuffer::new();
    buffer.set_direction(detect_direction(text));
    buffer.push_str(text);
    let output = rustybuzz::shape(&face, &[], buffer);
    let infos = output.glyph_infos();
    if infos.is_empty() {
        return None;
    }
    let mut map: BTreeMap<u16, String> = BTreeMap::new();
    let mut clusters: Vec<usize> = infos.iter().map(|g| g.cluster as usize).collect();
    clusters.push(text.len());
    for i in 0..infos.len() {
        let start = clusters[i].min(text.len());
        let end = clusters[i + 1].min(text.len());
        if start >= end {
            continue;
        }
        let s = text[start..end].to_string();
        let gid = infos[i].glyph_id as u16;
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
    let face = HbFace::from_slice(&font.data, 0)?;
    let units_per_em = face.units_per_em().max(1);
    let scale = 1000.0 / units_per_em as f32;

    let mut buffer = UnicodeBuffer::new();
    buffer.set_direction(detect_direction(text));
    buffer.push_str(text);
    let output = rustybuzz::shape(&face, &[], buffer);
    let infos = output.glyph_infos();
    let positions = output.glyph_positions();
    if infos.is_empty() || infos.len() != positions.len() {
        return None;
    }

    let mut parts: Vec<String> = Vec::new();
    for (info, pos) in infos.iter().zip(positions.iter()) {
        let gid = info.glyph_id as u16;
        if gid == 0 {
            continue;
        }
        let x_offset = (pos.x_offset as f32 * scale).round() as i32;
        if x_offset != 0 {
            parts.push(format!("{}", -x_offset));
        }
        parts.push(format!("<{:04X}>", gid));

        let adv_default = registry.glyph_advance(font_name, gid) as i32;
        let adv_shaped = (pos.x_advance as f32 * scale).round() as i32;
        let adjust = adv_default - adv_shaped;
        if adjust != 0 {
            parts.push(format!("{}", adjust));
        }
    }

    if parts.is_empty() {
        return None;
    }
    Some(format!("[{}] TJ\n", parts.join(" ")))
}

#[allow(dead_code)]
fn detect_direction(text: &str) -> rustybuzz::Direction {
    for ch in text.chars() {
        let code = ch as u32;
        let rtl = matches!(
            code,
            0x0590..=0x08FF
                | 0xFB1D..=0xFDFF
                | 0xFE70..=0xFEFF
                | 0x1EE00..=0x1EEFF
        );
        if rtl {
            return rustybuzz::Direction::RightToLeft;
        }
    }
    rustybuzz::Direction::LeftToRight
}

fn fmt(value: f32) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let fixed = I32F32::from_num(value);
    let scaled = (fixed * I32F32::from_num(1000)).round();
    let milli: i64 = scaled.to_num();
    format_milli(milli)
}

fn format_milli(milli: i64) -> String {
    if milli == 0 {
        return "0".to_string();
    }
    let sign = if milli < 0 { "-" } else { "" };
    let abs = milli.abs();
    let int_part = abs / 1000;
    let frac_part = abs % 1000;
    if frac_part == 0 {
        format!("{}{}", sign, int_part)
    } else {
        let mut s = format!("{}{}.{:03}", sign, int_part, frac_part);
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
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn to_unicode_cmap_handles_surrogates() {
        let mut map = BTreeMap::new();
        map.insert(3u16, "A".to_string());
        map.insert(4u16, "\u{1F600}".to_string());
        let cmap = to_unicode_cmap(&map);
        assert!(cmap.contains("<0003> <0041>"));
        assert!(cmap.contains("<0004> <D83DDE00>"));
    }

    fn one_page_document(commands: Vec<Command>) -> Document {
        Document {
            page_size: Size::a4(),
            pages: vec![Page { commands }],
        }
    }

    fn count_token(bytes: &[u8], token: &[u8]) -> usize {
        if token.is_empty() || bytes.len() < token.len() {
            return 0;
        }
        bytes.windows(token.len()).filter(|w| *w == token).count()
    }

    fn temp_log_path(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("fullbleed_{tag}_{}_{}.jsonl", std::process::id(), nanos))
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
        assert!(pdf.contains("/Artifact <</Subtype /Watermark>> BDC"));
        assert!(pdf.contains("/OC /WM BDC"));
        assert!(pdf.contains("/OCProperties"));
        assert!(pdf.contains("/Type /OCG"));
        assert!(pdf.contains("/Properties << /WM"));
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
        let draws_one = count_token(&pdf_one, b"/Im1 Do");
        let draws_multi = count_token(&pdf_multi, b"/Im1 Do");

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
        assert!(pdf.contains("/Fm1 Do"));
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
        assert!(pdf.contains("/Fm1 Do"));
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
