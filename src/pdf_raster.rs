use crate::canvas::{Command, Document, Page};
use crate::error::FullBleedError;
use crate::font::FontRegistry;
use crate::raster;
use crate::types::{Color, Pt, Size};
use base64::Engine;
use image::codecs::png::PngEncoder;
use image::{ColorType, ImageEncoder};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary as LoDictionary, Document as LoDocument, Object as LoObject, ObjectId};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Clone, Copy, Debug)]
struct Matrix {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Matrix {
    fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    fn from_operands(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Self {
        Self { a, b, c, d, e, f }
    }

    fn translation(tx: f32, ty: f32) -> Self {
        Self::from_operands(1.0, 0.0, 0.0, 1.0, tx, ty)
    }

    fn concat(self, rhs: Self) -> Self {
        Self {
            a: self.a * rhs.a + self.b * rhs.c,
            b: self.a * rhs.b + self.b * rhs.d,
            c: self.c * rhs.a + self.d * rhs.c,
            d: self.c * rhs.b + self.d * rhs.d,
            e: self.e * rhs.a + self.f * rhs.c + rhs.e,
            f: self.e * rhs.b + self.f * rhs.d + rhs.f,
        }
    }

    fn transform_point(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    fn axis_aligned_unit_rect(self) -> Option<(f32, f32, f32, f32)> {
        if self.b.abs() > 0.0001 || self.c.abs() > 0.0001 {
            return None;
        }
        let x0 = self.e;
        let x1 = self.e + self.a;
        let y0 = self.f;
        let y1 = self.f + self.d;
        let left = x0.min(x1);
        let right = x0.max(x1);
        let bottom = y0.min(y1);
        let top = y0.max(y1);
        Some((left, bottom, right, top))
    }
}

#[derive(Clone, Default)]
struct PdfResources {
    fonts: HashMap<String, String>,
    xobjects: HashMap<String, ObjectId>,
    extgstates: HashMap<String, (f32, f32)>,
}

impl PdfResources {
    fn merged(&self, child: &PdfResources) -> PdfResources {
        let mut out = self.clone();
        for (k, v) in &child.fonts {
            out.fonts.insert(k.clone(), v.clone());
        }
        for (k, v) in &child.xobjects {
            out.xobjects.insert(k.clone(), *v);
        }
        for (k, v) in &child.extgstates {
            out.extgstates.insert(k.clone(), *v);
        }
        out
    }
}

#[derive(Clone)]
struct ParseState {
    ctm: Matrix,
    font_name: String,
    font_size: Pt,
    text_matrix: Matrix,
    text_line_matrix: Matrix,
}

impl Default for ParseState {
    fn default() -> Self {
        Self {
            ctm: Matrix::identity(),
            font_name: "Helvetica".to_string(),
            font_size: Pt::from_f32(12.0),
            text_matrix: Matrix::identity(),
            text_line_matrix: Matrix::identity(),
        }
    }
}

#[derive(Clone)]
struct ParsedPage {
    size: Size,
    commands: Vec<Command>,
}

#[derive(Default)]
struct PdfRasterCache {
    image_data_uri_by_object: HashMap<ObjectId, String>,
}

pub(crate) fn pdf_path_to_png_pages(
    path: &Path,
    dpi: u32,
    registry: Option<&FontRegistry>,
    shape_text: bool,
) -> Result<Vec<Vec<u8>>, FullBleedError> {
    let bytes = std::fs::read(path)?;
    pdf_bytes_to_png_pages(&bytes, dpi, registry, shape_text)
}

pub(crate) fn pdf_bytes_to_png_pages(
    bytes: &[u8],
    dpi: u32,
    registry: Option<&FontRegistry>,
    shape_text: bool,
) -> Result<Vec<Vec<u8>>, FullBleedError> {
    let doc = LoDocument::load_mem(bytes).map_err(lopdf_err)?;
    let pages = parse_pdf_pages(&doc)?;
    if pages.is_empty() {
        return Err(FullBleedError::InvalidConfiguration(
            "pdf raster error: no pages".to_string(),
        ));
    }

    let mut out = Vec::with_capacity(pages.len());
    for parsed in pages {
        let document = Document {
            page_size: parsed.size,
            pages: vec![Page {
                commands: parsed.commands,
            }],
        };
        let mut pngs = raster::document_to_png_pages(&document, dpi, registry, shape_text)?;
        if let Some(page_png) = pngs.pop() {
            out.push(page_png);
        } else {
            return Err(FullBleedError::InvalidConfiguration(
                "pdf raster error: no rendered page output".to_string(),
            ));
        }
    }
    Ok(out)
}

fn parse_pdf_pages(doc: &LoDocument) -> Result<Vec<ParsedPage>, FullBleedError> {
    let page_map = doc.get_pages();
    let mut out = Vec::with_capacity(page_map.len());
    let mut cache = PdfRasterCache::default();
    for (_page_no, page_id) in page_map {
        out.push(parse_page(doc, page_id, &mut cache)?);
    }
    Ok(out)
}

fn parse_page(
    doc: &LoDocument,
    page_id: ObjectId,
    cache: &mut PdfRasterCache,
) -> Result<ParsedPage, FullBleedError> {
    let size = page_size_for_id(doc, page_id)?;
    let page_dict = doc
        .get_object(page_id)
        .map_err(lopdf_err)?
        .as_dict()
        .map_err(lopdf_err)?;
    let resources = resources_from_page(doc, page_dict)?;
    let content_bytes = doc.get_page_content(page_id).map_err(lopdf_err)?;
    let content = Content::decode(&content_bytes).map_err(lopdf_err)?;

    let mut state = ParseState::default();
    let mut stack: Vec<ParseState> = Vec::new();
    let mut commands: Vec<Command> = Vec::new();
    let mut visited_forms: HashSet<ObjectId> = HashSet::new();

    parse_operations(
        doc,
        &content.operations,
        &resources,
        size.height.to_f32(),
        &mut state,
        &mut stack,
        &mut commands,
        &mut visited_forms,
        cache,
    )?;

    Ok(ParsedPage { size, commands })
}

#[allow(clippy::too_many_arguments)]
fn parse_operations(
    doc: &LoDocument,
    operations: &[Operation],
    resources: &PdfResources,
    page_height: f32,
    state: &mut ParseState,
    stack: &mut Vec<ParseState>,
    commands: &mut Vec<Command>,
    visited_forms: &mut HashSet<ObjectId>,
    cache: &mut PdfRasterCache,
) -> Result<(), FullBleedError> {
    for op in operations {
        match op.operator.as_str() {
            "q" => {
                stack.push(state.clone());
                commands.push(Command::SaveState);
            }
            "Q" => {
                if let Some(prev) = stack.pop() {
                    *state = prev;
                }
                commands.push(Command::RestoreState);
            }
            "cm" => {
                if let Some([a, b, c, d, e, f]) = op_f32_6(op) {
                    state.ctm = state.ctm.concat(Matrix::from_operands(a, b, c, d, e, f));
                }
            }
            "w" => {
                if let Some(width) = op_f32(op, 0) {
                    commands.push(Command::SetLineWidth(Pt::from_f32(width.max(0.0))));
                }
            }
            "J" => {
                if let Some(cap) = op_i64(op, 0) {
                    commands.push(Command::SetLineCap(cap.clamp(0, 2) as u8));
                }
            }
            "j" => {
                if let Some(join) = op_i64(op, 0) {
                    commands.push(Command::SetLineJoin(join.clamp(0, 2) as u8));
                }
            }
            "M" => {
                if let Some(limit) = op_f32(op, 0) {
                    commands.push(Command::SetMiterLimit(Pt::from_f32(limit.max(0.0))));
                }
            }
            "d" => {
                if op.operands.len() >= 2 {
                    let pattern = op
                        .operands
                        .get(0)
                        .and_then(|o| o.as_array().ok())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(obj_to_f32)
                                .map(|v| Pt::from_f32(v.abs()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let phase = op.operands.get(1).and_then(obj_to_f32).unwrap_or(0.0);
                    commands.push(Command::SetDash {
                        pattern,
                        phase: Pt::from_f32(phase),
                    });
                }
            }
            "gs" => {
                if let Some(name) = op_name(op, 0) {
                    if let Some((fill, stroke)) = resources.extgstates.get(&name).copied() {
                        commands.push(Command::SetOpacity { fill, stroke });
                    }
                }
            }
            "rg" => {
                if let Some([r, g, b]) = op_f32_3(op) {
                    commands.push(Command::SetFillColor(Color::rgb(r, g, b)));
                }
            }
            "RG" => {
                if let Some([r, g, b]) = op_f32_3(op) {
                    commands.push(Command::SetStrokeColor(Color::rgb(r, g, b)));
                }
            }
            "g" => {
                if let Some(gray) = op_f32(op, 0) {
                    commands.push(Command::SetFillColor(Color::rgb(gray, gray, gray)));
                }
            }
            "G" => {
                if let Some(gray) = op_f32(op, 0) {
                    commands.push(Command::SetStrokeColor(Color::rgb(gray, gray, gray)));
                }
            }
            "k" => {
                if let Some([c, m, y, k]) = op_f32_4(op) {
                    let (r, g, b) = cmyk_to_rgb(c, m, y, k);
                    commands.push(Command::SetFillColor(Color::rgb(r, g, b)));
                }
            }
            "K" => {
                if let Some([c, m, y, k]) = op_f32_4(op) {
                    let (r, g, b) = cmyk_to_rgb(c, m, y, k);
                    commands.push(Command::SetStrokeColor(Color::rgb(r, g, b)));
                }
            }
            "m" => {
                if let Some([x, y]) = op_f32_2(op) {
                    let (x_pdf, y_pdf) = state.ctm.transform_point(x, y);
                    let (x_top, y_top) = to_top_left(x_pdf, y_pdf, page_height);
                    commands.push(Command::MoveTo {
                        x: Pt::from_f32(x_top),
                        y: Pt::from_f32(y_top),
                    });
                }
            }
            "l" => {
                if let Some([x, y]) = op_f32_2(op) {
                    let (x_pdf, y_pdf) = state.ctm.transform_point(x, y);
                    let (x_top, y_top) = to_top_left(x_pdf, y_pdf, page_height);
                    commands.push(Command::LineTo {
                        x: Pt::from_f32(x_top),
                        y: Pt::from_f32(y_top),
                    });
                }
            }
            "c" => {
                if let Some([x1, y1, x2, y2, x, y]) = op_f32_6(op) {
                    let (x1_pdf, y1_pdf) = state.ctm.transform_point(x1, y1);
                    let (x2_pdf, y2_pdf) = state.ctm.transform_point(x2, y2);
                    let (x_pdf, y_pdf) = state.ctm.transform_point(x, y);
                    let (x1_top, y1_top) = to_top_left(x1_pdf, y1_pdf, page_height);
                    let (x2_top, y2_top) = to_top_left(x2_pdf, y2_pdf, page_height);
                    let (x_top, y_top) = to_top_left(x_pdf, y_pdf, page_height);
                    commands.push(Command::CurveTo {
                        x1: Pt::from_f32(x1_top),
                        y1: Pt::from_f32(y1_top),
                        x2: Pt::from_f32(x2_top),
                        y2: Pt::from_f32(y2_top),
                        x: Pt::from_f32(x_top),
                        y: Pt::from_f32(y_top),
                    });
                }
            }
            "re" => {
                if let Some([x, y, w, h]) = op_f32_4(op) {
                    let p0 = state.ctm.transform_point(x, y);
                    let p1 = state.ctm.transform_point(x + w, y);
                    let p2 = state.ctm.transform_point(x + w, y + h);
                    let p3 = state.ctm.transform_point(x, y + h);
                    let (x0, y0) = to_top_left(p0.0, p0.1, page_height);
                    let (x1, y1) = to_top_left(p1.0, p1.1, page_height);
                    let (x2, y2) = to_top_left(p2.0, p2.1, page_height);
                    let (x3, y3) = to_top_left(p3.0, p3.1, page_height);
                    commands.push(Command::MoveTo {
                        x: Pt::from_f32(x0),
                        y: Pt::from_f32(y0),
                    });
                    commands.push(Command::LineTo {
                        x: Pt::from_f32(x1),
                        y: Pt::from_f32(y1),
                    });
                    commands.push(Command::LineTo {
                        x: Pt::from_f32(x2),
                        y: Pt::from_f32(y2),
                    });
                    commands.push(Command::LineTo {
                        x: Pt::from_f32(x3),
                        y: Pt::from_f32(y3),
                    });
                    commands.push(Command::ClosePath);
                }
            }
            "h" => commands.push(Command::ClosePath),
            "W" => commands.push(Command::ClipPath { evenodd: false }),
            "W*" => commands.push(Command::ClipPath { evenodd: true }),
            "f" | "F" => commands.push(Command::Fill),
            "f*" => commands.push(Command::FillEvenOdd),
            "S" => commands.push(Command::Stroke),
            "B" => commands.push(Command::FillStroke),
            "B*" => commands.push(Command::FillStrokeEvenOdd),
            "s" => {
                commands.push(Command::ClosePath);
                commands.push(Command::Stroke);
            }
            "b" => {
                commands.push(Command::ClosePath);
                commands.push(Command::FillStroke);
            }
            "b*" => {
                commands.push(Command::ClosePath);
                commands.push(Command::FillStrokeEvenOdd);
            }
            "n" => {
                // Path end without painting. Current raster command set has no explicit path reset.
            }
            "BT" => {
                state.text_matrix = Matrix::identity();
                state.text_line_matrix = Matrix::identity();
            }
            "ET" => {}
            "Tf" => {
                if let Some(font_res_name) = op_name(op, 0) {
                    let font_name = resources
                        .fonts
                        .get(&font_res_name)
                        .cloned()
                        .unwrap_or(font_res_name);
                    let size = op_f32(op, 1).unwrap_or(12.0).abs();
                    state.font_name = font_name.clone();
                    state.font_size = Pt::from_f32(size.max(0.0));
                    commands.push(Command::SetFontName(font_name));
                    commands.push(Command::SetFontSize(state.font_size));
                }
            }
            "Td" | "TD" => {
                if let Some([tx, ty]) = op_f32_2(op) {
                    let t = Matrix::translation(tx, ty);
                    state.text_line_matrix = state.text_line_matrix.concat(t);
                    state.text_matrix = state.text_line_matrix;
                }
            }
            "Tm" => {
                if let Some([a, b, c, d, e, f]) = op_f32_6(op) {
                    let tm = Matrix::from_operands(a, b, c, d, e, f);
                    state.text_matrix = tm;
                    state.text_line_matrix = tm;
                }
            }
            "Tj" => {
                if let Some(text) = decode_text_operand(op.operands.get(0)) {
                    emit_text(commands, state, page_height, &text);
                    advance_text_matrix(state, estimate_text_advance(&text, state.font_size));
                }
            }
            "TJ" => {
                if let Some(arr) = op.operands.get(0).and_then(|o| o.as_array().ok()) {
                    for item in arr {
                        if let Some(text) = decode_text_operand(Some(item)) {
                            emit_text(commands, state, page_height, &text);
                            advance_text_matrix(
                                state,
                                estimate_text_advance(&text, state.font_size),
                            );
                        } else if let Some(adj) = obj_to_f32(item) {
                            // TJ adjustment is thousandths of text-space units.
                            let tx = -(adj / 1000.0) * state.font_size.to_f32();
                            advance_text_matrix(state, tx);
                        }
                    }
                }
            }
            "Do" => {
                if let Some(name) = op_name(op, 0) {
                    if let Some(obj_id) = resources.xobjects.get(&name).copied() {
                        parse_xobject(
                            doc,
                            obj_id,
                            resources,
                            page_height,
                            state,
                            commands,
                            visited_forms,
                            cache,
                        )?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn parse_xobject(
    doc: &LoDocument,
    obj_id: ObjectId,
    parent_resources: &PdfResources,
    page_height: f32,
    state: &ParseState,
    commands: &mut Vec<Command>,
    visited_forms: &mut HashSet<ObjectId>,
    cache: &mut PdfRasterCache,
) -> Result<(), FullBleedError> {
    let stream = doc
        .get_object(obj_id)
        .map_err(lopdf_err)?
        .as_stream()
        .map_err(lopdf_err)?;
    let subtype = stream
        .dict
        .get(b"Subtype")
        .ok()
        .and_then(|o| o.as_name().ok())
        .map(name_bytes_to_string)
        .unwrap_or_default();

    if subtype == "Form" {
        if !visited_forms.insert(obj_id) {
            return Ok(());
        }
        let form_bytes = stream
            .get_plain_content()
            .map_err(|e| FullBleedError::InvalidConfiguration(format!("pdf raster error: {e}")))?;
        let form_content = Content::decode(&form_bytes).map_err(lopdf_err)?;
        let form_resources = match stream.dict.get(b"Resources") {
            Ok(obj) => resources_from_object(doc, obj)?,
            Err(_) => PdfResources::default(),
        };
        let merged_resources = parent_resources.merged(&form_resources);
        let form_matrix = stream
            .dict
            .get(b"Matrix")
            .ok()
            .and_then(parse_matrix_object)
            .unwrap_or_else(Matrix::identity);

        let mut nested_state = state.clone();
        nested_state.ctm = nested_state.ctm.concat(form_matrix);
        let mut nested_stack = Vec::new();
        parse_operations(
            doc,
            &form_content.operations,
            &merged_resources,
            page_height,
            &mut nested_state,
            &mut nested_stack,
            commands,
            visited_forms,
            cache,
        )?;
        visited_forms.remove(&obj_id);
        return Ok(());
    }

    if subtype == "Image" {
        let data_uri = if let Some(cached) = cache.image_data_uri_by_object.get(&obj_id) {
            cached.clone()
        } else {
            let built = image_stream_to_data_uri(stream).ok_or_else(|| {
                FullBleedError::InvalidConfiguration(
                    "pdf raster error: unsupported image xobject encoding".to_string(),
                )
            })?;
            cache.image_data_uri_by_object.insert(obj_id, built.clone());
            built
        };
        if let Some((left, bottom, right, top)) = state.ctm.axis_aligned_unit_rect() {
            let width = right - left;
            let height = top - bottom;
            if width > 0.0 && height > 0.0 {
                let y_top = page_height - top;
                commands.push(Command::DrawImage {
                    x: Pt::from_f32(left),
                    y: Pt::from_f32(y_top),
                    width: Pt::from_f32(width),
                    height: Pt::from_f32(height),
                    resource_id: data_uri,
                });
            }
        }
    }

    Ok(())
}

fn emit_text(commands: &mut Vec<Command>, state: &ParseState, page_height: f32, text: &str) {
    if text.is_empty() {
        return;
    }
    let (tx, ty) = state.text_matrix.transform_point(0.0, 0.0);
    let (x_pdf, y_pdf) = state.ctm.transform_point(tx, ty);
    let y_top = page_height - y_pdf - state.font_size.to_f32();
    commands.push(Command::DrawString {
        x: Pt::from_f32(x_pdf),
        y: Pt::from_f32(y_top),
        text: text.to_string(),
    });
}

fn advance_text_matrix(state: &mut ParseState, tx: f32) {
    state.text_matrix = state.text_matrix.concat(Matrix::translation(tx, 0.0));
}

fn estimate_text_advance(text: &str, size: Pt) -> f32 {
    (text.chars().count() as f32) * size.to_f32() * 0.5
}

fn decode_text_operand(obj: Option<&LoObject>) -> Option<String> {
    let obj = obj?;
    if let Ok(decoded) = lopdf::decode_text_string(obj) {
        return Some(decoded);
    }
    if let Ok(bytes) = obj.as_str() {
        return Some(String::from_utf8_lossy(bytes).to_string());
    }
    None
}
fn resources_from_page(
    doc: &LoDocument,
    page_dict: &LoDictionary,
) -> Result<PdfResources, FullBleedError> {
    match page_dict.get(b"Resources") {
        Ok(obj) => resources_from_object(doc, obj),
        Err(_) => Ok(PdfResources::default()),
    }
}

fn resources_from_object(doc: &LoDocument, obj: &LoObject) -> Result<PdfResources, FullBleedError> {
    let resolved = resolve_object(doc, obj)?;
    let dict = match resolved {
        LoObject::Dictionary(d) => d,
        _ => return Ok(PdfResources::default()),
    };

    let mut out = PdfResources::default();

    if let Ok(font_obj) = dict.get(b"Font") {
        let font_dict = resolve_dict(doc, font_obj)?;
        for (name, font_ref_obj) in font_dict.iter() {
            let resource_name = name_bytes_to_string(name);
            let font_name = resolve_font_name(doc, font_ref_obj)?;
            out.fonts.insert(resource_name, font_name);
        }
    }

    if let Ok(xobj_obj) = dict.get(b"XObject") {
        let xobj_dict = resolve_dict(doc, xobj_obj)?;
        for (name, ref_obj) in xobj_dict.iter() {
            if let Ok(id) = ref_obj.as_reference() {
                out.xobjects.insert(name_bytes_to_string(name), id);
            }
        }
    }

    if let Ok(gs_obj) = dict.get(b"ExtGState") {
        let gs_dict = resolve_dict(doc, gs_obj)?;
        for (name, gs_ref_obj) in gs_dict.iter() {
            let gs_name = name_bytes_to_string(name);
            let resolved_gs = resolve_object(doc, gs_ref_obj)?;
            let gs_dict = match resolved_gs {
                LoObject::Dictionary(d) => d,
                _ => continue,
            };
            let fill = gs_dict
                .get(b"ca")
                .ok()
                .and_then(obj_to_f32)
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            let stroke = gs_dict
                .get(b"CA")
                .ok()
                .and_then(obj_to_f32)
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            out.extgstates.insert(gs_name, (fill, stroke));
        }
    }

    Ok(out)
}

fn resolve_font_name(doc: &LoDocument, obj: &LoObject) -> Result<String, FullBleedError> {
    let resolved = resolve_object(doc, obj)?;
    let dict = match resolved {
        LoObject::Dictionary(d) => d,
        _ => return Ok("Helvetica".to_string()),
    };
    if let Ok(base) = dict.get(b"BaseFont").and_then(LoObject::as_name) {
        return Ok(normalize_pdf_font_name(&name_bytes_to_string(base)));
    }
    Ok("Helvetica".to_string())
}

fn page_size_for_id(doc: &LoDocument, mut id: ObjectId) -> Result<Size, FullBleedError> {
    loop {
        let dict = doc
            .get_object(id)
            .map_err(lopdf_err)?
            .as_dict()
            .map_err(lopdf_err)?;
        if let Ok(arr) = dict.get(b"MediaBox").and_then(LoObject::as_array) {
            if let Some(size) = parse_media_box_array(arr) {
                return Ok(size);
            }
        }
        id = match dict.get(b"Parent").and_then(LoObject::as_reference) {
            Ok(parent_id) => parent_id,
            Err(_) => break,
        };
    }
    Ok(Size::letter())
}

fn parse_media_box_array(arr: &[LoObject]) -> Option<Size> {
    if arr.len() < 4 {
        return None;
    }
    let x0 = obj_to_f32(&arr[0])?;
    let y0 = obj_to_f32(&arr[1])?;
    let x1 = obj_to_f32(&arr[2])?;
    let y1 = obj_to_f32(&arr[3])?;
    let width = (x1 - x0).abs().max(1.0);
    let height = (y1 - y0).abs().max(1.0);
    Some(Size {
        width: Pt::from_f32(width),
        height: Pt::from_f32(height),
    })
}

fn image_stream_to_data_uri(stream: &lopdf::Stream) -> Option<String> {
    let filters = stream.filters().unwrap_or_default();
    let has_dct = filters.iter().any(|f| *f == b"DCTDecode");
    if has_dct {
        return Some(data_uri("image/jpeg", stream.content.as_slice()));
    }

    if filters.is_empty() {
        if let Ok(fmt) = image::guess_format(&stream.content) {
            let mime = match fmt {
                image::ImageFormat::Png => Some("image/png"),
                image::ImageFormat::Jpeg => Some("image/jpeg"),
                _ => None,
            }?;
            return Some(data_uri(mime, stream.content.as_slice()));
        }
    }

    let plain = stream.get_plain_content().ok()?;

    if let Some(uri) = raw_image_data_to_png_uri(stream, &plain) {
        return Some(uri);
    }

    if let Ok(fmt) = image::guess_format(&plain) {
        let mime = match fmt {
            image::ImageFormat::Png => Some("image/png"),
            image::ImageFormat::Jpeg => Some("image/jpeg"),
            _ => None,
        }?;
        return Some(data_uri(mime, plain.as_slice()));
    }

    None
}

fn raw_image_data_to_png_uri(stream: &lopdf::Stream, plain: &[u8]) -> Option<String> {
    let width = stream
        .dict
        .get(b"Width")
        .ok()
        .and_then(|o| o.as_i64().ok())
        .and_then(|v| u32::try_from(v).ok())?;
    let height = stream
        .dict
        .get(b"Height")
        .ok()
        .and_then(|o| o.as_i64().ok())
        .and_then(|v| u32::try_from(v).ok())?;
    let bpc = stream
        .dict
        .get(b"BitsPerComponent")
        .ok()
        .and_then(obj_to_f32)
        .unwrap_or(8.0);
    if (bpc - 8.0).abs() > 0.01 {
        return None;
    }

    let channels = match stream.dict.get(b"ColorSpace") {
        Ok(obj) => color_space_channels(obj)?,
        Err(_) => 1,
    };
    let expected = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(channels);
    if plain.len() < expected {
        return None;
    }

    let mut rgba = vec![0u8; (width as usize) * (height as usize) * 4];
    let mut src = 0usize;
    let mut dst = 0usize;
    while src + channels <= plain.len() && dst + 4 <= rgba.len() {
        let (r, g, b) = match channels {
            1 => {
                let v = plain[src];
                (v, v, v)
            }
            3 => (plain[src], plain[src + 1], plain[src + 2]),
            4 => {
                let c = plain[src] as f32 / 255.0;
                let m = plain[src + 1] as f32 / 255.0;
                let y = plain[src + 2] as f32 / 255.0;
                let k = plain[src + 3] as f32 / 255.0;
                let (rf, gf, bf) = cmyk_to_rgb(c, m, y, k);
                (
                    (rf.clamp(0.0, 1.0) * 255.0) as u8,
                    (gf.clamp(0.0, 1.0) * 255.0) as u8,
                    (bf.clamp(0.0, 1.0) * 255.0) as u8,
                )
            }
            _ => return None,
        };
        rgba[dst] = r;
        rgba[dst + 1] = g;
        rgba[dst + 2] = b;
        rgba[dst + 3] = 255;
        src += channels;
        dst += 4;
    }

    let mut png = Vec::new();
    let encoder = PngEncoder::new(&mut png);
    if encoder
        .write_image(&rgba, width, height, ColorType::Rgba8.into())
        .is_err()
    {
        return None;
    }
    Some(data_uri("image/png", &png))
}

fn color_space_channels(obj: &LoObject) -> Option<usize> {
    match obj {
        LoObject::Name(name) => match name.as_slice() {
            b"DeviceGray" => Some(1),
            b"DeviceRGB" => Some(3),
            b"DeviceCMYK" => Some(4),
            _ => None,
        },
        LoObject::Array(arr) => {
            let head = arr.first()?;
            let head_name = head.as_name().ok()?;
            match head_name {
                b"DeviceGray" => Some(1),
                b"DeviceRGB" => Some(3),
                b"DeviceCMYK" => Some(4),
                _ => None,
            }
        }
        _ => None,
    }
}

fn parse_matrix_object(obj: &LoObject) -> Option<Matrix> {
    let arr = obj.as_array().ok()?;
    if arr.len() < 6 {
        return None;
    }
    Some(Matrix::from_operands(
        obj_to_f32(&arr[0])?,
        obj_to_f32(&arr[1])?,
        obj_to_f32(&arr[2])?,
        obj_to_f32(&arr[3])?,
        obj_to_f32(&arr[4])?,
        obj_to_f32(&arr[5])?,
    ))
}

fn resolve_object<'a>(
    doc: &'a LoDocument,
    mut obj: &'a LoObject,
) -> Result<&'a LoObject, FullBleedError> {
    loop {
        match obj {
            LoObject::Reference(id) => {
                obj = doc.get_object(*id).map_err(lopdf_err)?;
            }
            _ => return Ok(obj),
        }
    }
}

fn resolve_dict(doc: &LoDocument, obj: &LoObject) -> Result<LoDictionary, FullBleedError> {
    let resolved = resolve_object(doc, obj)?;
    match resolved {
        LoObject::Dictionary(d) => Ok(d.clone()),
        _ => Ok(LoDictionary::new()),
    }
}

fn op_name(op: &Operation, idx: usize) -> Option<String> {
    let obj = op.operands.get(idx)?;
    let name = obj.as_name().ok()?;
    Some(name_bytes_to_string(name))
}

fn op_f32(op: &Operation, idx: usize) -> Option<f32> {
    obj_to_f32(op.operands.get(idx)?)
}

fn op_i64(op: &Operation, idx: usize) -> Option<i64> {
    op.operands.get(idx)?.as_i64().ok()
}

fn op_f32_2(op: &Operation) -> Option<[f32; 2]> {
    Some([op_f32(op, 0)?, op_f32(op, 1)?])
}

fn op_f32_3(op: &Operation) -> Option<[f32; 3]> {
    Some([op_f32(op, 0)?, op_f32(op, 1)?, op_f32(op, 2)?])
}

fn op_f32_4(op: &Operation) -> Option<[f32; 4]> {
    Some([
        op_f32(op, 0)?,
        op_f32(op, 1)?,
        op_f32(op, 2)?,
        op_f32(op, 3)?,
    ])
}

fn op_f32_6(op: &Operation) -> Option<[f32; 6]> {
    Some([
        op_f32(op, 0)?,
        op_f32(op, 1)?,
        op_f32(op, 2)?,
        op_f32(op, 3)?,
        op_f32(op, 4)?,
        op_f32(op, 5)?,
    ])
}

fn obj_to_f32(obj: &LoObject) -> Option<f32> {
    obj.as_float().ok()
}

fn to_top_left(x_pdf: f32, y_pdf: f32, page_height: f32) -> (f32, f32) {
    (x_pdf, page_height - y_pdf)
}

fn name_bytes_to_string(name: &[u8]) -> String {
    String::from_utf8_lossy(name).to_string()
}

fn normalize_pdf_font_name(name: &str) -> String {
    let trimmed = name
        .trim()
        .trim_start_matches('/')
        .trim_matches('"')
        .trim_matches('\'');
    if let Some((prefix, rest)) = trimmed.split_once('+') {
        if prefix.len() == 6 && prefix.chars().all(|c| c.is_ascii_alphabetic()) {
            return rest.to_string();
        }
    }
    trimmed.to_string()
}

fn data_uri(mime: &str, data: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(data);
    format!("data:{mime};base64,{b64}")
}

fn cmyk_to_rgb(c: f32, m: f32, y: f32, k: f32) -> (f32, f32, f32) {
    let c = c.clamp(0.0, 1.0);
    let m = m.clamp(0.0, 1.0);
    let y = y.clamp(0.0, 1.0);
    let k = k.clamp(0.0, 1.0);
    let r = (1.0 - c) * (1.0 - k);
    let g = (1.0 - m) * (1.0 - k);
    let b = (1.0 - y) * (1.0 - k);
    (r, g, b)
}

fn lopdf_err(err: lopdf::Error) -> FullBleedError {
    FullBleedError::InvalidConfiguration(format!("pdf raster error: {err}"))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ComposePagePlan, TemplateAsset, TemplateCatalog, compose_overlay_with_template_catalog,
    };
    use lopdf::{Dictionary as LoDictionary, Stream as LoStream, dictionary};

    fn write_text_pdf(path: &Path, fill_rgb: (f32, f32, f32), text: &str, width: i64, height: i64) {
        let mut doc = LoDocument::with_version("1.7");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content = format!(
            "{} {} {} rg\n0 0 {} {} re\nf\n0 0 0 rg\nBT\n/F1 18 Tf\n36 {} Td\n({}) Tj\nET\n",
            fill_rgb.0,
            fill_rgb.1,
            fill_rgb.2,
            width,
            height,
            height - 40,
            text
        )
        .into_bytes();
        let content_id = doc.add_object(LoStream::new(LoDictionary::new(), content));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
        });
        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        };
        doc.objects.insert(pages_id, LoObject::Dictionary(pages));
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        doc.compress();
        doc.save(path).expect("save");
    }

    fn has_non_white_pixel(img: &image::RgbaImage) -> bool {
        img.pixels().any(|p| {
            let [r, g, b, _a] = p.0;
            !(r == 255 && g == 255 && b == 255)
        })
    }

    #[test]
    fn pdf_raster_smoke_text_and_fill() {
        let temp_dir = std::env::temp_dir().join(format!(
            "fullbleed_pdf_raster_smoke_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_dir).expect("mkdir");
        let pdf_path = temp_dir.join("page.pdf");
        write_text_pdf(&pdf_path, (0.8, 0.9, 1.0), "HELLO", 612, 792);

        let pages = pdf_path_to_png_pages(&pdf_path, 120, None, true).expect("raster");
        assert_eq!(pages.len(), 1);
        let img = image::load_from_memory(&pages[0]).expect("png").to_rgba8();
        assert!(has_non_white_pixel(&img));
    }

    #[test]
    fn pdf_raster_compose_includes_template_background() {
        let temp_dir = std::env::temp_dir().join(format!(
            "fullbleed_pdf_raster_compose_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_dir).expect("mkdir");
        let template = temp_dir.join("template.pdf");
        let overlay = temp_dir.join("overlay.pdf");
        let composed = temp_dir.join("composed.pdf");

        write_text_pdf(&template, (0.0, 0.0, 1.0), "TPL", 612, 792);
        write_text_pdf(&overlay, (1.0, 1.0, 1.0), "OVL", 612, 792);

        let mut catalog = TemplateCatalog::default();
        catalog
            .insert(TemplateAsset {
                template_id: "tpl-blue".to_string(),
                pdf_path: template.clone(),
                sha256: None,
                page_count: None,
            })
            .expect("insert tpl");

        let plan = vec![ComposePagePlan {
            template_id: "tpl-blue".to_string(),
            template_page_index: 0,
            overlay_page_index: 0,
            dx: 0.0,
            dy: 0.0,
        }];
        compose_overlay_with_template_catalog(&catalog, &overlay, &composed, &plan)
            .expect("compose");

        let pages = pdf_path_to_png_pages(&composed, 120, None, true).expect("raster composed");
        assert_eq!(pages.len(), 1);
        let img = image::load_from_memory(&pages[0]).expect("png").to_rgba8();
        let px = img.get_pixel(10, 10).0;
        assert!(
            px[2] > 180,
            "expected blue template background, got {:?}",
            px
        );
    }

    #[test]
    fn normalize_pdf_font_name_strips_subset_prefix() {
        assert_eq!(
            normalize_pdf_font_name("ABCDEF+Helvetica-Bold"),
            "Helvetica-Bold"
        );
        assert_eq!(normalize_pdf_font_name("/XYZQWE+Inter-Italic"), "Inter-Italic");
        assert_eq!(normalize_pdf_font_name("Helvetica"), "Helvetica");
    }
}
