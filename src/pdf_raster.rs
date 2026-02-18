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
use std::sync::Arc;
use ttf_parser::Face;

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
struct PdfFontResource {
    font_name: String,
    to_unicode: HashMap<u16, String>,
    embedded_font: Option<Arc<Vec<u8>>>,
    metrics: PdfFontMetrics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PdfCharCodeWidthEncoding {
    SingleByte,
    TwoByteBigEndian,
}

impl Default for PdfCharCodeWidthEncoding {
    fn default() -> Self {
        Self::SingleByte
    }
}

#[derive(Clone, Default)]
struct PdfFontMetrics {
    default_width: f32,
    widths: HashMap<u16, f32>,
    code_encoding: PdfCharCodeWidthEncoding,
}

#[derive(Clone, Default)]
struct PdfResources {
    fonts: HashMap<String, PdfFontResource>,
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
    font_resource: Option<String>,
    font_name: String,
    font_size: Pt,
    text_matrix: Matrix,
    text_line_matrix: Matrix,
    text_leading: f32,
    char_spacing: f32,
    word_spacing: f32,
    text_h_scale: f32,
    text_rise: f32,
    text_render_mode: i64,
}

impl Default for ParseState {
    fn default() -> Self {
        Self {
            ctm: Matrix::identity(),
            font_resource: None,
            font_name: "Helvetica".to_string(),
            font_size: Pt::from_f32(12.0),
            text_matrix: Matrix::identity(),
            text_line_matrix: Matrix::identity(),
            text_leading: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            text_h_scale: 1.0,
            text_rise: 0.0,
            text_render_mode: 0,
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
    let (pages, embedded_fonts) = parse_pdf_pages(&doc)?;
    if pages.is_empty() {
        return Err(FullBleedError::InvalidConfiguration(
            "pdf raster error: no pages".to_string(),
        ));
    }

    let mut embedded_registry = FontRegistry::new();
    for (font_name, font_bytes) in &embedded_fonts {
        let _ = embedded_registry.register_bytes((**font_bytes).clone(), Some(font_name));
    }
    let effective_registry = if embedded_fonts.is_empty() {
        registry
    } else {
        Some(&embedded_registry)
    };

    let mut out = Vec::with_capacity(pages.len());
    for parsed in pages {
        let document = Document {
            page_size: parsed.size,
            pages: vec![Page {
                commands: parsed.commands,
            }],
        };
        let mut pngs =
            raster::document_to_png_pages(&document, dpi, effective_registry, shape_text)?;
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

fn parse_pdf_pages(
    doc: &LoDocument,
) -> Result<(Vec<ParsedPage>, HashMap<String, Arc<Vec<u8>>>), FullBleedError> {
    let page_map = doc.get_pages();
    let mut out = Vec::with_capacity(page_map.len());
    let mut cache = PdfRasterCache::default();
    let mut embedded_fonts: HashMap<String, Arc<Vec<u8>>> = HashMap::new();
    for (_page_no, page_id) in page_map {
        out.push(parse_page(doc, page_id, &mut cache, &mut embedded_fonts)?);
    }
    Ok((out, embedded_fonts))
}

fn parse_page(
    doc: &LoDocument,
    page_id: ObjectId,
    cache: &mut PdfRasterCache,
    embedded_fonts: &mut HashMap<String, Arc<Vec<u8>>>,
) -> Result<ParsedPage, FullBleedError> {
    let size = page_size_for_id(doc, page_id)?;
    let page_dict = doc
        .get_object(page_id)
        .map_err(lopdf_err)?
        .as_dict()
        .map_err(lopdf_err)?;
    let resources = resources_from_page(doc, page_dict, embedded_fonts)?;
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
        embedded_fonts,
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
    embedded_fonts: &mut HashMap<String, Arc<Vec<u8>>>,
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
            "TL" => {
                if let Some(leading) = op_f32(op, 0) {
                    state.text_leading = leading;
                }
            }
            "Tc" => {
                if let Some(spacing) = op_f32(op, 0) {
                    state.char_spacing = spacing;
                }
            }
            "Tw" => {
                if let Some(spacing) = op_f32(op, 0) {
                    state.word_spacing = spacing;
                }
            }
            "Tz" => {
                if let Some(scale_percent) = op_f32(op, 0) {
                    state.text_h_scale = (scale_percent / 100.0).max(0.0);
                }
            }
            "Ts" => {
                if let Some(rise) = op_f32(op, 0) {
                    state.text_rise = rise;
                }
            }
            "Tr" => {
                if let Some(mode) = op_i64(op, 0) {
                    state.text_render_mode = mode.clamp(0, 7);
                }
            }
            "Tf" => {
                if let Some(font_res_name) = op_name(op, 0) {
                    let font_res =
                        resources
                            .fonts
                            .get(&font_res_name)
                            .cloned()
                            .unwrap_or_else(|| PdfFontResource {
                                font_name: font_res_name.clone(),
                                to_unicode: HashMap::new(),
                                embedded_font: None,
                                metrics: PdfFontMetrics::default(),
                            });
                    let size = op_f32(op, 1).unwrap_or(12.0).abs();
                    state.font_resource = Some(font_res_name);
                    state.font_name = font_res.font_name.clone();
                    state.font_size = Pt::from_f32(size.max(0.0));
                    commands.push(Command::SetFontName(state.font_name.clone()));
                    commands.push(Command::SetFontSize(state.font_size));
                }
            }
            "Td" | "TD" => {
                if let Some([tx, ty]) = op_f32_2(op) {
                    if op.operator == "TD" {
                        state.text_leading = -ty;
                    }
                    let (ux, uy) = text_space_delta_to_user(state.text_line_matrix, tx, ty);
                    let t = Matrix::translation(ux, uy);
                    state.text_line_matrix = state.text_line_matrix.concat(t);
                    state.text_matrix = state.text_line_matrix;
                }
            }
            "T*" => {
                let (ux, uy) =
                    text_space_delta_to_user(state.text_line_matrix, 0.0, -state.text_leading);
                let t = Matrix::translation(ux, uy);
                state.text_line_matrix = state.text_line_matrix.concat(t);
                state.text_matrix = state.text_line_matrix;
            }
            "Tm" => {
                if let Some([a, b, c, d, e, f]) = op_f32_6(op) {
                    let tm = Matrix::from_operands(a, b, c, d, e, f);
                    state.text_matrix = tm;
                    state.text_line_matrix = tm;
                }
            }
            "Tj" => {
                let current_font = state
                    .font_resource
                    .as_ref()
                    .and_then(|res| resources.fonts.get(res));
                let text_obj = op.operands.get(0);
                if let Some(text) = decode_text_operand(text_obj, current_font) {
                    emit_text(commands, state, page_height, &text);
                    let advance = text_obj
                        .and_then(|obj| {
                            estimate_text_advance_from_operand(obj, state, current_font, &text)
                        })
                        .unwrap_or_else(|| {
                            estimate_text_advance_fallback(&text, state, current_font)
                        });
                    advance_text_matrix(state, advance);
                }
            }
            "TJ" => {
                let current_font = state
                    .font_resource
                    .as_ref()
                    .and_then(|res| resources.fonts.get(res));
                if let Some(arr) = op.operands.get(0).and_then(|o| o.as_array().ok()) {
                    for item in arr {
                        if let Some(text) = decode_text_operand(Some(item), current_font) {
                            emit_text(commands, state, page_height, &text);
                            let advance = estimate_text_advance_from_operand(
                                item,
                                state,
                                current_font,
                                &text,
                            )
                            .unwrap_or_else(|| {
                                estimate_text_advance_fallback(&text, state, current_font)
                            });
                            advance_text_matrix(state, advance);
                        } else if let Some(adj) = obj_to_f32(item) {
                            // TJ adjustment is thousandths of text-space units.
                            let tx = -(adj / 1000.0)
                                * state.font_size.to_f32()
                                * state.text_h_scale.max(0.0);
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
                            embedded_fonts,
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
    embedded_fonts: &mut HashMap<String, Arc<Vec<u8>>>,
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
            Ok(obj) => resources_from_object(doc, obj, embedded_fonts)?,
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
            embedded_fonts,
        )?;
        visited_forms.remove(&obj_id);
        return Ok(());
    }

    if subtype == "Image" {
        let data_uri = if let Some(cached) = cache.image_data_uri_by_object.get(&obj_id) {
            cached.clone()
        } else {
            let built = image_stream_to_data_uri(doc, stream).ok_or_else(|| {
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
    if state.text_render_mode == 3 || state.text_render_mode == 7 {
        return;
    }
    let (tx, ty) = state.text_matrix.transform_point(0.0, state.text_rise);
    let (x_pdf, y_pdf) = state.ctm.transform_point(tx, ty);
    let effective_size = effective_font_size(state);
    let y_top = page_height - y_pdf - effective_size;
    commands.push(Command::SetFontSize(Pt::from_f32(effective_size)));
    commands.push(Command::DrawString {
        x: Pt::from_f32(x_pdf),
        y: Pt::from_f32(y_top),
        text: text.to_string(),
    });
}

fn advance_text_matrix(state: &mut ParseState, tx: f32) {
    let (ux, uy) = text_space_delta_to_user(state.text_matrix, tx, 0.0);
    state.text_matrix = state.text_matrix.concat(Matrix::translation(ux, uy));
}

fn estimate_text_advance_from_operand(
    obj: &LoObject,
    state: &ParseState,
    font: Option<&PdfFontResource>,
    text: &str,
) -> Option<f32> {
    let bytes = obj.as_str().ok()?;
    let font = font?;
    advance_from_pdf_codes(bytes, state, font)
        .or_else(|| Some(estimate_text_advance_fallback(text, state, Some(font))))
}

fn advance_from_pdf_codes(bytes: &[u8], state: &ParseState, font: &PdfFontResource) -> Option<f32> {
    let codes = pdf_string_codes(bytes, font.metrics.code_encoding)?;
    if codes.is_empty() {
        return Some(0.0);
    }

    let mut sum = 0.0f32;
    let font_size = state.font_size.to_f32();
    for code in codes {
        let width = font
            .metrics
            .widths
            .get(&code)
            .copied()
            .unwrap_or(font.metrics.default_width)
            .max(0.0);
        sum += (width / 1000.0) * font_size + state.char_spacing;
        if code_is_space(font, code) {
            sum += state.word_spacing;
        }
    }

    Some(sum * state.text_h_scale.max(0.0))
}

fn pdf_string_codes(bytes: &[u8], encoding: PdfCharCodeWidthEncoding) -> Option<Vec<u16>> {
    match encoding {
        PdfCharCodeWidthEncoding::SingleByte => {
            let mut out = Vec::with_capacity(bytes.len());
            for b in bytes {
                out.push(*b as u16);
            }
            Some(out)
        }
        PdfCharCodeWidthEncoding::TwoByteBigEndian => {
            if bytes.len() < 2 {
                return None;
            }
            let mut out = Vec::with_capacity(bytes.len() / 2);
            for chunk in bytes.chunks_exact(2) {
                out.push(u16::from_be_bytes([chunk[0], chunk[1]]));
            }
            Some(out)
        }
    }
}

fn code_is_space(font: &PdfFontResource, code: u16) -> bool {
    if code == 0x0020 {
        return true;
    }
    font.to_unicode
        .get(&code)
        .map(|mapped| mapped.as_str() == " ")
        .unwrap_or(false)
}

fn estimate_text_advance_fallback(
    text: &str,
    state: &ParseState,
    font: Option<&PdfFontResource>,
) -> f32 {
    let glyph_advance = estimate_glyph_advance_fallback(text, state, font);
    let fallback = state.font_size.to_f32().max(0.01) * 0.5;
    let mut sum = 0.0f32;
    for (idx, ch) in text.chars().enumerate() {
        sum += glyph_advance.get(idx).copied().unwrap_or(fallback) + state.char_spacing;
        if ch == ' ' {
            sum += state.word_spacing;
        }
    }
    sum * state.text_h_scale.max(0.0)
}

fn estimate_glyph_advance_fallback(
    text: &str,
    state: &ParseState,
    font: Option<&PdfFontResource>,
) -> Vec<f32> {
    let fallback = state.font_size.to_f32().max(0.01) * 0.5;
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let Some(font_bytes) = font
        .and_then(|f| f.embedded_font.as_ref())
        .map(|arc| arc.as_slice())
    else {
        return vec![fallback; chars.len()];
    };
    let Ok(face) = Face::parse(font_bytes, 0) else {
        return vec![fallback; chars.len()];
    };
    let upem = face.units_per_em().max(1) as f32;
    let scale = state.font_size.to_f32() / upem;
    let mut out = Vec::with_capacity(chars.len());
    for ch in chars {
        let adv = face
            .glyph_index(ch)
            .and_then(|gid| face.glyph_hor_advance(gid))
            .map(|w| (w as f32) * scale)
            .unwrap_or(fallback);
        out.push(adv);
    }
    out
}

fn text_matrix_scale_x(m: Matrix) -> f32 {
    (m.a * m.a + m.b * m.b).sqrt()
}

fn text_matrix_scale_y(m: Matrix) -> f32 {
    (m.c * m.c + m.d * m.d).sqrt()
}

fn effective_font_size(state: &ParseState) -> f32 {
    let sx = text_matrix_scale_x(state.text_matrix);
    let sy = text_matrix_scale_y(state.text_matrix);
    let matrix_scale = if sy > 0.0001 {
        sy
    } else if sx > 0.0001 {
        sx
    } else {
        1.0
    };
    (state.font_size.to_f32() * matrix_scale).max(0.01)
}

fn text_space_delta_to_user(m: Matrix, tx: f32, ty: f32) -> (f32, f32) {
    let ux = m.a * tx + m.c * ty;
    let uy = m.b * tx + m.d * ty;
    (ux, uy)
}

fn decode_text_operand(obj: Option<&LoObject>, font: Option<&PdfFontResource>) -> Option<String> {
    let obj = obj?;
    if let Some(bytes) = obj.as_str().ok() {
        if let Some(font_resource) = font {
            if !font_resource.to_unicode.is_empty() {
                if let Some(decoded) = decode_with_to_unicode(bytes, &font_resource.to_unicode) {
                    return Some(decoded);
                }
            }
        }
    }
    if let Ok(decoded) = lopdf::decode_text_string(obj) {
        return Some(decoded);
    }
    if let Ok(bytes) = obj.as_str() {
        return Some(String::from_utf8_lossy(bytes).to_string());
    }
    None
}

fn decode_with_to_unicode(bytes: &[u8], cmap: &HashMap<u16, String>) -> Option<String> {
    if bytes.is_empty() {
        return Some(String::new());
    }
    if bytes.len() % 2 == 0 {
        let mut out = String::new();
        let mut mapped_any = false;
        for chunk in bytes.chunks_exact(2) {
            let code = u16::from_be_bytes([chunk[0], chunk[1]]);
            if let Some(mapped) = cmap.get(&code) {
                out.push_str(mapped);
                mapped_any = true;
            } else if let Some(ch) = char::from_u32(code as u32) {
                out.push(ch);
            } else {
                out.push('?');
            }
        }
        if mapped_any {
            return Some(out);
        }
    }

    let mut out = String::new();
    let mut mapped_any = false;
    for b in bytes {
        let code = *b as u16;
        if let Some(mapped) = cmap.get(&code) {
            out.push_str(mapped);
            mapped_any = true;
        } else if let Some(ch) = char::from_u32(code as u32) {
            out.push(ch);
        } else {
            out.push('?');
        }
    }
    if mapped_any {
        return Some(out);
    }
    None
}
fn resources_from_page(
    doc: &LoDocument,
    page_dict: &LoDictionary,
    embedded_fonts: &mut HashMap<String, Arc<Vec<u8>>>,
) -> Result<PdfResources, FullBleedError> {
    match page_dict.get(b"Resources") {
        Ok(obj) => resources_from_object(doc, obj, embedded_fonts),
        Err(_) => Ok(PdfResources::default()),
    }
}

fn resources_from_object(
    doc: &LoDocument,
    obj: &LoObject,
    embedded_fonts: &mut HashMap<String, Arc<Vec<u8>>>,
) -> Result<PdfResources, FullBleedError> {
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
            let font = resolve_font_resource(doc, font_ref_obj)?;
            if let Some(data) = font.embedded_font.as_ref() {
                embedded_fonts
                    .entry(font.font_name.clone())
                    .or_insert_with(|| data.clone());
            }
            out.fonts.insert(resource_name, font);
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

fn resolve_font_resource(
    doc: &LoDocument,
    obj: &LoObject,
) -> Result<PdfFontResource, FullBleedError> {
    let resolved = resolve_object(doc, obj)?;
    let dict = match resolved {
        LoObject::Dictionary(d) => d,
        _ => {
            return Ok(PdfFontResource {
                font_name: "Helvetica".to_string(),
                to_unicode: HashMap::new(),
                embedded_font: None,
                metrics: PdfFontMetrics::default(),
            });
        }
    };
    let font_name = dict
        .get(b"BaseFont")
        .ok()
        .and_then(|obj| obj.as_name().ok())
        .map(name_bytes_to_string)
        .map(|name| normalize_pdf_font_name(&name))
        .unwrap_or_else(|| "Helvetica".to_string());
    let to_unicode = parse_to_unicode_cmap(doc, dict);
    let embedded_font = resolve_embedded_font_bytes(doc, dict).map(Arc::new);
    let metrics = parse_font_metrics(doc, dict, &to_unicode);
    Ok(PdfFontResource {
        font_name,
        to_unicode,
        embedded_font,
        metrics,
    })
}

fn parse_font_metrics(
    doc: &LoDocument,
    font_dict: &LoDictionary,
    to_unicode: &HashMap<u16, String>,
) -> PdfFontMetrics {
    let subtype = font_dict
        .get(b"Subtype")
        .ok()
        .and_then(|o| o.as_name().ok())
        .map(name_bytes_to_string)
        .unwrap_or_default();

    if subtype == "Type0" {
        return parse_type0_font_metrics(doc, font_dict, to_unicode);
    }
    parse_simple_font_metrics(doc, font_dict)
}

fn parse_type0_font_metrics(
    doc: &LoDocument,
    font_dict: &LoDictionary,
    to_unicode: &HashMap<u16, String>,
) -> PdfFontMetrics {
    let encoding_name = font_dict
        .get(b"Encoding")
        .ok()
        .and_then(|o| resolve_object(doc, o).ok())
        .and_then(|o| o.as_name().ok())
        .map(name_bytes_to_string)
        .unwrap_or_default();
    let code_encoding = if encoding_name == "Identity-H" || encoding_name == "Identity-V" {
        PdfCharCodeWidthEncoding::TwoByteBigEndian
    } else if to_unicode.keys().any(|code| *code > 0x00FF) {
        PdfCharCodeWidthEncoding::TwoByteBigEndian
    } else {
        PdfCharCodeWidthEncoding::SingleByte
    };

    let mut default_width = 1000.0f32;
    let mut widths = HashMap::new();

    if let Some(descendant_dict) = font_dict
        .get(b"DescendantFonts")
        .ok()
        .and_then(|o| resolve_object(doc, o).ok())
        .and_then(|o| o.as_array().ok())
        .and_then(|arr| arr.first())
        .and_then(|obj| resolve_object(doc, obj).ok())
        .and_then(|obj| obj.as_dict().ok())
    {
        if let Ok(dw_obj) = descendant_dict.get(b"DW") {
            if let Some(dw) = resolved_obj_to_f32(doc, dw_obj) {
                default_width = dw.max(0.0);
            }
        }
        if let Ok(w_obj) = descendant_dict.get(b"W") {
            widths = parse_cid_font_widths(doc, w_obj);
        }
    }

    PdfFontMetrics {
        default_width,
        widths,
        code_encoding,
    }
}

fn parse_simple_font_metrics(doc: &LoDocument, font_dict: &LoDictionary) -> PdfFontMetrics {
    let mut default_width = 500.0f32;
    if let Ok(descriptor_obj) = font_dict.get(b"FontDescriptor") {
        if let Some(descriptor_dict) = resolve_object(doc, descriptor_obj)
            .ok()
            .and_then(|obj| obj.as_dict().ok())
        {
            if let Ok(missing_obj) = descriptor_dict.get(b"MissingWidth") {
                if let Some(missing) = resolved_obj_to_f32(doc, missing_obj) {
                    default_width = missing.max(0.0);
                }
            }
        }
    }

    let first_char = font_dict
        .get(b"FirstChar")
        .ok()
        .and_then(|obj| resolved_obj_to_u16(doc, obj))
        .unwrap_or(0u16);
    let mut widths = HashMap::new();
    if let Ok(widths_obj) = font_dict.get(b"Widths") {
        if let Some(width_arr) = resolve_object(doc, widths_obj)
            .ok()
            .and_then(|obj| obj.as_array().ok())
        {
            for (idx, width_obj) in width_arr.iter().enumerate() {
                let Some(width) = resolved_obj_to_f32(doc, width_obj) else {
                    continue;
                };
                let Ok(offset) = u16::try_from(idx) else {
                    break;
                };
                let Some(code) = first_char.checked_add(offset) else {
                    break;
                };
                widths.insert(code, width.max(0.0));
            }
        }
    }

    PdfFontMetrics {
        default_width,
        widths,
        code_encoding: PdfCharCodeWidthEncoding::SingleByte,
    }
}

fn parse_cid_font_widths(doc: &LoDocument, obj: &LoObject) -> HashMap<u16, f32> {
    let mut out = HashMap::new();
    let Some(width_items) = resolve_object(doc, obj)
        .ok()
        .and_then(|resolved| resolved.as_array().ok())
    else {
        return out;
    };

    let mut idx = 0usize;
    while idx < width_items.len() {
        let Some(start_cid) = resolved_obj_to_u16(doc, &width_items[idx]) else {
            idx += 1;
            continue;
        };
        if idx + 1 >= width_items.len() {
            break;
        }

        let next_obj = match resolve_object(doc, &width_items[idx + 1]) {
            Ok(obj) => obj,
            Err(_) => {
                idx += 1;
                continue;
            }
        };

        if let Ok(width_list) = next_obj.as_array() {
            for (offset, width_obj) in width_list.iter().enumerate() {
                let Some(width) = resolved_obj_to_f32(doc, width_obj) else {
                    continue;
                };
                let Ok(step) = u16::try_from(offset) else {
                    break;
                };
                let Some(code) = start_cid.checked_add(step) else {
                    break;
                };
                out.insert(code, width.max(0.0));
            }
            idx += 2;
            continue;
        }

        let Some(end_cid) = resolved_obj_to_u16(doc, &width_items[idx + 1]) else {
            idx += 1;
            continue;
        };
        let Some(width_obj) = width_items.get(idx + 2) else {
            break;
        };
        let Some(width) = resolved_obj_to_f32(doc, width_obj) else {
            idx += 3;
            continue;
        };

        for code in start_cid..=end_cid {
            out.insert(code, width.max(0.0));
            if code == u16::MAX {
                break;
            }
        }
        idx += 3;
    }

    out
}

fn resolved_obj_to_f32(doc: &LoDocument, obj: &LoObject) -> Option<f32> {
    let resolved = resolve_object(doc, obj).ok()?;
    obj_to_f32(resolved)
}

fn resolved_obj_to_u16(doc: &LoDocument, obj: &LoObject) -> Option<u16> {
    let resolved = resolve_object(doc, obj).ok()?;
    if let Ok(v) = resolved.as_i64() {
        return u16::try_from(v).ok();
    }
    let v = obj_to_f32(resolved)?;
    if !(0.0..=(u16::MAX as f32)).contains(&v) {
        return None;
    }
    Some(v.round() as u16)
}

fn resolve_embedded_font_bytes(doc: &LoDocument, font_dict: &LoDictionary) -> Option<Vec<u8>> {
    let subtype = font_dict
        .get(b"Subtype")
        .ok()
        .and_then(|o| o.as_name().ok())
        .map(name_bytes_to_string)
        .unwrap_or_default();

    if subtype == "Type0" {
        let descendants = font_dict.get(b"DescendantFonts").ok()?.as_array().ok()?;
        let descendant = descendants.first()?;
        let descendant_dict = resolve_object(doc, descendant).ok()?.as_dict().ok()?;
        let descriptor_obj = descendant_dict.get(b"FontDescriptor").ok()?;
        return font_descriptor_file_bytes(doc, descriptor_obj);
    }

    let descriptor_obj = font_dict.get(b"FontDescriptor").ok()?;
    font_descriptor_file_bytes(doc, descriptor_obj)
}

fn font_descriptor_file_bytes(doc: &LoDocument, descriptor_obj: &LoObject) -> Option<Vec<u8>> {
    let descriptor = resolve_object(doc, descriptor_obj).ok()?.as_dict().ok()?;
    for key in [
        b"FontFile2".as_slice(),
        b"FontFile3".as_slice(),
        b"FontFile".as_slice(),
    ] {
        if let Ok(obj) = descriptor.get(key) {
            if let Some(data) = resolve_object(doc, obj)
                .ok()
                .and_then(|o| o.as_stream().ok())
                .and_then(|s| s.get_plain_content().ok())
            {
                if !data.is_empty() {
                    return Some(data);
                }
            }
        }
    }
    None
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

#[derive(Clone, Copy)]
enum RasterDirectColor {
    Gray,
    Rgb,
    Cmyk,
}

impl RasterDirectColor {
    fn channels(self) -> usize {
        match self {
            Self::Gray => 1,
            Self::Rgb => 3,
            Self::Cmyk => 4,
        }
    }

    fn rgb_from_bytes(self, bytes: &[u8]) -> Option<(u8, u8, u8)> {
        match self {
            Self::Gray => {
                let v = *bytes.first()?;
                Some((v, v, v))
            }
            Self::Rgb => Some((*bytes.first()?, *bytes.get(1)?, *bytes.get(2)?)),
            Self::Cmyk => {
                let c = (*bytes.first()? as f32) / 255.0;
                let m = (*bytes.get(1)? as f32) / 255.0;
                let y = (*bytes.get(2)? as f32) / 255.0;
                let k = (*bytes.get(3)? as f32) / 255.0;
                let (rf, gf, bf) = cmyk_to_rgb(c, m, y, k);
                Some((
                    (rf.clamp(0.0, 1.0) * 255.0) as u8,
                    (gf.clamp(0.0, 1.0) * 255.0) as u8,
                    (bf.clamp(0.0, 1.0) * 255.0) as u8,
                ))
            }
        }
    }
}

enum RasterColorSpace {
    Direct(RasterDirectColor),
    Indexed {
        base: RasterDirectColor,
        lookup: Vec<u8>,
    },
}

fn direct_color_from_name(name: &[u8]) -> Option<RasterDirectColor> {
    match name {
        b"DeviceGray" => Some(RasterDirectColor::Gray),
        b"DeviceRGB" => Some(RasterDirectColor::Rgb),
        b"DeviceCMYK" => Some(RasterDirectColor::Cmyk),
        _ => None,
    }
}

fn parse_raster_color_space(doc: &LoDocument, obj: &LoObject) -> Option<RasterColorSpace> {
    let resolved = resolve_object(doc, obj).ok()?;
    match resolved {
        LoObject::Name(name) => {
            let direct = direct_color_from_name(name.as_slice())?;
            Some(RasterColorSpace::Direct(direct))
        }
        LoObject::Array(arr) => parse_raster_color_space_array(doc, arr),
        _ => None,
    }
}

fn parse_raster_color_space_array(doc: &LoDocument, arr: &[LoObject]) -> Option<RasterColorSpace> {
    let head = arr.first()?;
    let head_name = resolve_object(doc, head).ok()?.as_name().ok()?;

    if let Some(direct) = direct_color_from_name(head_name) {
        return Some(RasterColorSpace::Direct(direct));
    }
    if head_name != b"Indexed" {
        return None;
    }
    if arr.len() < 4 {
        return None;
    }

    let base = match parse_raster_color_space(doc, arr.get(1)?)? {
        RasterColorSpace::Direct(mode) => mode,
        RasterColorSpace::Indexed { .. } => return None,
    };
    let lookup = lookup_table_bytes(doc, arr.get(3)?)?;
    Some(RasterColorSpace::Indexed { base, lookup })
}

fn lookup_table_bytes(doc: &LoDocument, obj: &LoObject) -> Option<Vec<u8>> {
    let resolved = resolve_object(doc, obj).ok()?;
    match resolved {
        LoObject::String(bytes, _) => Some(bytes.clone()),
        LoObject::Stream(stream) => stream.get_plain_content().ok(),
        _ => None,
    }
}

fn image_stream_to_data_uri(doc: &LoDocument, stream: &lopdf::Stream) -> Option<String> {
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

    if let Some(uri) = raw_image_data_to_png_uri(doc, stream, &plain) {
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

fn raw_image_data_to_png_uri(
    doc: &LoDocument,
    stream: &lopdf::Stream,
    plain: &[u8],
) -> Option<String> {
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

    let color_space = match stream.dict.get(b"ColorSpace") {
        Ok(obj) => parse_raster_color_space(doc, obj)?,
        Err(_) => RasterColorSpace::Direct(RasterDirectColor::Gray),
    };
    let pixels = (width as usize).saturating_mul(height as usize);
    let expected = match &color_space {
        RasterColorSpace::Direct(mode) => pixels.saturating_mul(mode.channels()),
        RasterColorSpace::Indexed { .. } => pixels,
    };
    if plain.len() < expected {
        return None;
    }

    let mut rgba = vec![0u8; (width as usize) * (height as usize) * 4];
    let mut src = 0usize;
    let mut dst = 0usize;
    while dst + 4 <= rgba.len() {
        let (r, g, b) = match &color_space {
            RasterColorSpace::Direct(mode) => {
                let channels = mode.channels();
                if src + channels > plain.len() {
                    return None;
                }
                let rgb = mode.rgb_from_bytes(&plain[src..(src + channels)])?;
                src += channels;
                rgb
            }
            RasterColorSpace::Indexed { base, lookup } => {
                let idx = *plain.get(src)? as usize;
                src += 1;
                let channels = base.channels();
                let offset = idx.saturating_mul(channels);
                if offset + channels > lookup.len() {
                    return None;
                }
                base.rgb_from_bytes(&lookup[offset..(offset + channels)])?
            }
        };
        rgba[dst] = r;
        rgba[dst + 1] = g;
        rgba[dst + 2] = b;
        rgba[dst + 3] = 255;
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
    if let Ok(v) = obj.as_float() {
        return Some(v);
    }
    obj.as_i64().ok().map(|v| v as f32)
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

fn parse_to_unicode_cmap(doc: &LoDocument, font_dict: &LoDictionary) -> HashMap<u16, String> {
    let mut map = HashMap::new();
    let to_unicode_obj = match font_dict.get(b"ToUnicode") {
        Ok(obj) => obj,
        Err(_) => return map,
    };
    let stream = match resolve_object(doc, to_unicode_obj)
        .ok()
        .and_then(|obj| obj.as_stream().ok())
    {
        Some(s) => s,
        None => return map,
    };
    let bytes = match stream.get_plain_content() {
        Ok(data) => data,
        Err(_) => return map,
    };
    let text = String::from_utf8_lossy(&bytes);

    let mut in_bfchar = false;
    let mut in_bfrange = false;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if line.ends_with("beginbfchar") {
            in_bfchar = true;
            in_bfrange = false;
            continue;
        }
        if line.ends_with("endbfchar") {
            in_bfchar = false;
            continue;
        }
        if line.ends_with("beginbfrange") {
            in_bfrange = true;
            in_bfchar = false;
            continue;
        }
        if line.ends_with("endbfrange") {
            in_bfrange = false;
            continue;
        }
        if in_bfchar {
            let tokens = extract_hex_tokens(line);
            if tokens.len() >= 2 {
                if let Some(src) = hex_bytes_to_u16(&tokens[0]) {
                    let dst = hex_bytes_to_unicode(&tokens[1]);
                    map.insert(src, dst);
                }
            }
            continue;
        }
        if in_bfrange {
            let tokens = extract_hex_tokens(line);
            if tokens.len() < 3 {
                continue;
            }
            let start = match hex_bytes_to_u16(&tokens[0]) {
                Some(v) => v,
                None => continue,
            };
            let end = match hex_bytes_to_u16(&tokens[1]) {
                Some(v) => v,
                None => continue,
            };
            if start > end {
                continue;
            }
            if line.contains('[') {
                for (idx, token) in tokens.iter().skip(2).enumerate() {
                    let code = start.saturating_add(idx as u16);
                    if code > end {
                        break;
                    }
                    map.insert(code, hex_bytes_to_unicode(token));
                }
            } else if let Some(base) = hex_bytes_to_u16(&tokens[2]) {
                for code in start..=end {
                    let dst = base.saturating_add(code.saturating_sub(start));
                    if let Some(ch) = char::from_u32(dst as u32) {
                        map.insert(code, ch.to_string());
                    }
                }
            }
        }
    }
    map
}

fn extract_hex_tokens(line: &str) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != b'>' {
                i += 1;
            }
            if i <= bytes.len() {
                let token = &line[start..i];
                if let Some(decoded) = parse_hex(token) {
                    out.push(decoded);
                }
            }
        }
        i += 1;
    }
    out
}

fn parse_hex(token: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut nibbles = Vec::new();
    for ch in token.chars() {
        if ch.is_whitespace() {
            continue;
        }
        let val = ch.to_digit(16)? as u8;
        nibbles.push(val);
    }
    if nibbles.is_empty() {
        return Some(bytes);
    }
    if nibbles.len() % 2 != 0 {
        nibbles.push(0);
    }
    for pair in nibbles.chunks_exact(2) {
        bytes.push((pair[0] << 4) | pair[1]);
    }
    Some(bytes)
}

fn hex_bytes_to_u16(bytes: &[u8]) -> Option<u16> {
    if bytes.len() == 2 {
        Some(u16::from_be_bytes([bytes[0], bytes[1]]))
    } else {
        None
    }
}

fn hex_bytes_to_unicode(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    if bytes.len() % 2 == 0 {
        let mut units = Vec::with_capacity(bytes.len() / 2);
        for chunk in bytes.chunks_exact(2) {
            units.push(u16::from_be_bytes([chunk[0], chunk[1]]));
        }
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes).to_string()
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
        compose_overlay_with_template_catalog, ComposePagePlan, TemplateAsset, TemplateCatalog,
    };
    use lopdf::{dictionary, Dictionary as LoDictionary, Stream as LoStream};
    use std::collections::HashMap;

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

    fn non_white_bounds(img: &image::RgbaImage) -> Option<(u32, u32, u32, u32)> {
        let mut min_x = u32::MAX;
        let mut min_y = u32::MAX;
        let mut max_x = 0u32;
        let mut max_y = 0u32;
        let mut found = false;
        for (x, y, px) in img.enumerate_pixels() {
            let [r, g, b, _a] = px.0;
            if r == 255 && g == 255 && b == 255 {
                continue;
            }
            found = true;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        if found {
            Some((min_x, min_y, max_x, max_y))
        } else {
            None
        }
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
    fn pdf_raster_handles_t_star_line_advance() {
        let temp_dir = std::env::temp_dir().join(format!(
            "fullbleed_pdf_raster_tstar_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_dir).expect("mkdir");
        let pdf_path = temp_dir.join("tstar.pdf");

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
        let content =
            b"0 0 0 rg\nBT\n/F1 20 Tf\n18 TL\n36 720 Td\n(LINE1) Tj\nT*\n(LINE2) Tj\nET\n".to_vec();
        let content_id = doc.add_object(LoStream::new(LoDictionary::new(), content));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
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
        doc.save(&pdf_path).expect("save");

        let pages = pdf_path_to_png_pages(&pdf_path, 144, None, true).expect("raster");
        assert_eq!(pages.len(), 1);
        let img = image::load_from_memory(&pages[0]).expect("png").to_rgba8();
        let (_min_x, min_y, _max_x, max_y) = non_white_bounds(&img).expect("ink bounds");
        let span = max_y.saturating_sub(min_y);
        assert!(
            span >= 30,
            "expected multiline vertical span from T* advance, got {span}"
        );
    }

    #[test]
    fn image_stream_to_data_uri_supports_indexed_cmyk_lookup_stream() {
        let mut doc = LoDocument::with_version("1.7");
        let lookup_id = doc.add_object(LoStream::new(
            LoDictionary::new(),
            vec![
                0, 0, 0, 0, // index 0 -> white (CMYK)
                0, 255, 255, 0, // index 1 -> red (CMYK)
            ],
        ));
        let image_stream = LoStream::new(
            dictionary! {
                "Subtype" => "Image",
                "Width" => 2,
                "Height" => 1,
                "BitsPerComponent" => 8,
                "ColorSpace" => vec![
                    LoObject::Name(b"Indexed".to_vec()),
                    LoObject::Name(b"DeviceCMYK".to_vec()),
                    1.into(),
                    lookup_id.into(),
                ],
            },
            vec![0u8, 1u8],
        );

        let uri = image_stream_to_data_uri(&doc, &image_stream).expect("indexed image to data uri");
        let b64 = uri.split_once(',').expect("data uri").1;
        let png = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("base64 decode");
        let img = image::load_from_memory(&png)
            .expect("decode png")
            .to_rgba8();

        assert_eq!(img.width(), 2);
        assert_eq!(img.height(), 1);
        let left = img.get_pixel(0, 0).0;
        let right = img.get_pixel(1, 0).0;
        assert!(
            left[0] > 240 && left[1] > 240 && left[2] > 240,
            "expected white from indexed palette, got {:?}",
            left
        );
        assert!(
            right[0] > 200 && right[1] < 80 && right[2] < 80,
            "expected red from indexed palette, got {:?}",
            right
        );
    }

    #[test]
    fn decode_with_to_unicode_handles_utf16be_codes() {
        let mut cmap = HashMap::new();
        cmap.insert(0x0026u16, "C".to_string());
        cmap.insert(0x004Bu16, "h".to_string());
        cmap.insert(0x0048u16, "e".to_string());
        cmap.insert(0x0046u16, "c".to_string());
        cmap.insert(0x004Eu16, "k".to_string());
        cmap.insert(0x0003u16, " ".to_string());
        let bytes = vec![
            0x00, 0x26, 0x00, 0x4B, 0x00, 0x48, 0x00, 0x46, 0x00, 0x4E, 0x00, 0x03,
        ];
        let decoded = decode_with_to_unicode(&bytes, &cmap).expect("decode");
        assert_eq!(decoded, "Check ");
    }

    #[test]
    fn decode_with_to_unicode_handles_single_byte_codes() {
        let mut cmap = HashMap::new();
        cmap.insert(0x0048u16, "H".to_string());
        cmap.insert(0x0069u16, "i".to_string());
        let decoded = decode_with_to_unicode(b"Hi", &cmap).expect("decode");
        assert_eq!(decoded, "Hi");
    }

    #[test]
    fn advance_from_pdf_codes_uses_type0_widths_without_double_scaling() {
        let mut state = ParseState::default();
        state.font_size = Pt::from_f32(1.0);
        state.text_matrix = Matrix::from_operands(7.0, 0.0, 0.0, 7.0, 0.0, 0.0);
        state.text_h_scale = 1.0;

        let mut font = PdfFontResource::default();
        font.metrics.default_width = 1000.0;
        font.metrics.code_encoding = PdfCharCodeWidthEncoding::TwoByteBigEndian;
        font.metrics.widths.insert(0x0041, 600.0); // 'A'
        font.metrics.widths.insert(0x0003, 250.0); // mapped space
        font.to_unicode.insert(0x0003, " ".to_string());

        let bytes = [0x00, 0x41, 0x00, 0x03, 0x00, 0x41];
        let tx = advance_from_pdf_codes(&bytes, &state, &font).expect("advance");
        assert!(
            (tx - 1.45).abs() < 0.001,
            "expected text-space advance 1.45, got {tx}"
        );

        let mut moved = state.clone();
        advance_text_matrix(&mut moved, tx);
        assert!(
            (moved.text_matrix.e - 10.15).abs() < 0.02,
            "expected user-space x move 10.15 from matrix scale, got {}",
            moved.text_matrix.e
        );
    }

    #[test]
    fn pdf_raster_skips_invisible_text_render_mode() {
        let temp_dir = std::env::temp_dir().join(format!(
            "fullbleed_pdf_raster_tr3_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_dir).expect("mkdir");
        let pdf_path = temp_dir.join("tr3.pdf");

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
        let content = b"BT\n/F1 36 Tf\n3 Tr\n72 720 Td\n(HIDDEN TEXT) Tj\nET\n".to_vec();
        let content_id = doc.add_object(LoStream::new(LoDictionary::new(), content));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
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
        doc.save(&pdf_path).expect("save");

        let pages = pdf_path_to_png_pages(&pdf_path, 120, None, true).expect("raster");
        assert_eq!(pages.len(), 1);
        let img = image::load_from_memory(&pages[0]).expect("png").to_rgba8();
        assert!(
            !has_non_white_pixel(&img),
            "expected no visible text for Tr=3 mode"
        );
    }

    #[test]
    fn normalize_pdf_font_name_strips_subset_prefix() {
        assert_eq!(
            normalize_pdf_font_name("ABCDEF+Helvetica-Bold"),
            "Helvetica-Bold"
        );
        assert_eq!(
            normalize_pdf_font_name("/XYZQWE+Inter-Italic"),
            "Inter-Italic"
        );
        assert_eq!(normalize_pdf_font_name("Helvetica"), "Helvetica");
    }
}
