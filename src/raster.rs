use crate::canvas::{Command, Document};
use crate::error::FullBleedError;
use crate::font::FontRegistry;
use crate::types::{Color, Pt, Shading, ShadingStop};
use base64::Engine;
use rustybuzz::{Direction as HbDirection, Face as HbFace, UnicodeBuffer};
use std::collections::HashMap;
use std::path::Path as FsPath;
use std::sync::{Arc, Mutex, OnceLock};
use tiny_skia::{
    FillRule, FilterQuality, GradientStop, LineCap, LineJoin, LinearGradient, Mask, Paint, Path,
    PathBuilder, Pixmap, PixmapPaint, Point, RadialGradient, Rect, Shader, SpreadMode, Stroke,
    StrokeDash, Transform,
};
use ttf_parser::{GlyphId, OutlineBuilder};

#[derive(Clone)]
struct RasterState {
    transform: Transform,
    fill_color: Color,
    stroke_color: Color,
    line_width: Pt,
    line_cap: u8,
    line_join: u8,
    miter_limit: Pt,
    dash_pattern: Vec<Pt>,
    dash_phase: Pt,
    fill_opacity: f32,
    stroke_opacity: f32,
    font_name: String,
    font_size: Pt,
    clip_mask: Option<Mask>,
}

impl Default for RasterState {
    fn default() -> Self {
        Self {
            transform: Transform::identity(),
            fill_color: Color::BLACK,
            stroke_color: Color::BLACK,
            line_width: Pt::from_f32(1.0),
            line_cap: 0,
            line_join: 0,
            miter_limit: Pt::from_f32(4.0),
            dash_pattern: Vec::new(),
            dash_phase: Pt::ZERO,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
            font_name: "Helvetica".to_string(),
            font_size: Pt::from_f32(12.0),
            clip_mask: None,
        }
    }
}

#[derive(Clone)]
struct FormDefinition {
    width: Pt,
    height: Pt,
    commands: Vec<Command>,
}

pub(crate) fn document_to_png_pages(
    document: &Document,
    dpi: u32,
    registry: Option<&FontRegistry>,
    shape_text: bool,
) -> Result<Vec<Vec<u8>>, FullBleedError> {
    let dpi = if dpi == 0 { 150 } else { dpi };
    let width_px = pt_milli_to_px_u32(document.page_size.width.to_milli_i64(), dpi)?;
    let height_px = pt_milli_to_px_u32(document.page_size.height.to_milli_i64(), dpi)?;
    let page_height_pt = document.page_size.height.to_f32();
    let page_width_pt = document.page_size.width.to_f32();
    let scale = dpi as f32 / 72.0;
    let base_transform = Transform::from_row(scale, 0.0, 0.0, -scale, 0.0, page_height_pt * scale);

    let mut png_pages = Vec::with_capacity(document.pages.len());
    let mut image_cache: HashMap<String, Option<Pixmap>> = HashMap::new();
    let mut forms: HashMap<String, FormDefinition> = HashMap::new();

    for page in &document.pages {
        let mut pixmap = Pixmap::new(width_px, height_px).ok_or_else(|| {
            FullBleedError::InvalidConfiguration(format!(
                "invalid raster size {}x{} at {} DPI",
                width_px, height_px, dpi
            ))
        })?;
        pixmap.fill(tiny_skia::Color::from_rgba8(255, 255, 255, 255));

        let mut state = RasterState::default();
        let mut stack: Vec<RasterState> = Vec::new();
        let mut path_builder = PathBuilder::new();
        let mut has_path = false;

        render_commands(
            &mut pixmap,
            page_height_pt,
            page_width_pt,
            &page.commands,
            base_transform,
            &mut state,
            &mut stack,
            &mut path_builder,
            &mut has_path,
            &mut forms,
            &mut image_cache,
            registry,
            shape_text,
        )?;

        let png = pixmap
            .encode_png()
            .map_err(|e| FullBleedError::Asset(format!("png encode failed: {e}")))?;
        png_pages.push(png);
    }

    Ok(png_pages)
}

#[allow(clippy::too_many_arguments)]
fn render_commands(
    pixmap: &mut Pixmap,
    page_height_pt: f32,
    page_width_pt: f32,
    commands: &[Command],
    base_transform: Transform,
    state: &mut RasterState,
    stack: &mut Vec<RasterState>,
    path_builder: &mut PathBuilder,
    has_path: &mut bool,
    forms: &mut HashMap<String, FormDefinition>,
    image_cache: &mut HashMap<String, Option<Pixmap>>,
    registry: Option<&FontRegistry>,
    shape_text: bool,
) -> Result<(), FullBleedError> {
    for cmd in commands {
        match cmd {
            Command::SaveState => stack.push(state.clone()),
            Command::RestoreState => {
                if let Some(restored) = stack.pop() {
                    *state = restored;
                }
            }
            Command::Translate(x, y) => {
                state.transform = state
                    .transform
                    .post_concat(Transform::from_translate(x.to_f32(), y.to_f32()));
            }
            Command::Scale(x, y) => {
                state.transform = state.transform.post_concat(Transform::from_scale(*x, *y));
            }
            Command::Rotate(angle) => {
                let deg = *angle * 180.0 / core::f32::consts::PI;
                state.transform = state.transform.post_concat(Transform::from_rotate(deg));
            }
            Command::ConcatMatrix { a, b, c, d, e, f } => {
                state.transform = state.transform.post_concat(Transform::from_row(
                    *a,
                    *b,
                    *c,
                    *d,
                    e.to_f32(),
                    f.to_f32(),
                ));
            }
            Command::Meta { .. } => {}
            Command::BeginTag { .. } => {}
            Command::EndTag => {}
            Command::BeginArtifact { .. } => {}
            Command::BeginOptionalContent { .. } => {}
            Command::EndMarkedContent => {}
            Command::SetFillColor(color) => state.fill_color = *color,
            Command::SetStrokeColor(color) => state.stroke_color = *color,
            Command::SetLineWidth(width) => {
                state.line_width = if *width < Pt::ZERO { Pt::ZERO } else { *width };
            }
            Command::SetLineCap(cap) => state.line_cap = *cap,
            Command::SetLineJoin(join) => state.line_join = *join,
            Command::SetMiterLimit(limit) => {
                state.miter_limit = if *limit < Pt::ZERO { Pt::ZERO } else { *limit };
            }
            Command::SetDash { pattern, phase } => {
                state.dash_pattern = pattern.clone();
                state.dash_phase = *phase;
            }
            Command::SetOpacity { fill, stroke } => {
                state.fill_opacity = fill.clamp(0.0, 1.0);
                state.stroke_opacity = stroke.clamp(0.0, 1.0);
            }
            Command::SetFontName(name) => state.font_name = name.clone(),
            Command::SetFontSize(size) => state.font_size = *size,
            Command::ClipRect {
                x,
                y,
                width,
                height,
            } => {
                let draw_y = page_height_pt - y.to_f32() - height.to_f32();
                if let Some(rect) =
                    Rect::from_xywh(x.to_f32(), draw_y, width.to_f32(), height.to_f32())
                {
                    let path = PathBuilder::from_rect(rect);
                    apply_clip_path(
                        state,
                        &path,
                        FillRule::Winding,
                        base_transform.pre_concat(state.transform),
                        pixmap.width(),
                        pixmap.height(),
                    );
                }
            }
            Command::ClipPath { evenodd } => {
                if let Some(path) = take_path(path_builder, has_path) {
                    let fill_rule = if *evenodd {
                        FillRule::EvenOdd
                    } else {
                        FillRule::Winding
                    };
                    apply_clip_path(
                        state,
                        &path,
                        fill_rule,
                        base_transform.pre_concat(state.transform),
                        pixmap.width(),
                        pixmap.height(),
                    );
                }
            }
            Command::ShadingFill(shading) => {
                draw_shading_fill(
                    pixmap,
                    shading,
                    state,
                    page_height_pt,
                    page_width_pt,
                    base_transform,
                );
            }
            Command::MoveTo { x, y } => {
                let y_pdf = page_height_pt - y.to_f32();
                path_builder.move_to(x.to_f32(), y_pdf);
                *has_path = true;
            }
            Command::LineTo { x, y } => {
                let y_pdf = page_height_pt - y.to_f32();
                path_builder.line_to(x.to_f32(), y_pdf);
                *has_path = true;
            }
            Command::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                path_builder.cubic_to(
                    x1.to_f32(),
                    page_height_pt - y1.to_f32(),
                    x2.to_f32(),
                    page_height_pt - y2.to_f32(),
                    x.to_f32(),
                    page_height_pt - y.to_f32(),
                );
                *has_path = true;
            }
            Command::ClosePath => {
                if *has_path {
                    path_builder.close();
                }
            }
            Command::Fill => {
                fill_current_path(
                    pixmap,
                    state,
                    path_builder,
                    has_path,
                    FillRule::Winding,
                    base_transform,
                );
            }
            Command::FillEvenOdd => {
                fill_current_path(
                    pixmap,
                    state,
                    path_builder,
                    has_path,
                    FillRule::EvenOdd,
                    base_transform,
                );
            }
            Command::Stroke => {
                stroke_current_path(pixmap, state, path_builder, has_path, base_transform);
            }
            Command::FillStroke => {
                fill_stroke_current_path(
                    pixmap,
                    state,
                    path_builder,
                    has_path,
                    FillRule::Winding,
                    base_transform,
                );
            }
            Command::FillStrokeEvenOdd => {
                fill_stroke_current_path(
                    pixmap,
                    state,
                    path_builder,
                    has_path,
                    FillRule::EvenOdd,
                    base_transform,
                );
            }
            Command::DrawString { x, y, text } => {
                draw_string(
                    pixmap,
                    state,
                    x.to_f32(),
                    y.to_f32(),
                    text,
                    page_height_pt,
                    base_transform,
                    registry,
                    shape_text,
                );
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
                draw_string_transformed(
                    pixmap,
                    state,
                    x.to_f32(),
                    y.to_f32(),
                    text,
                    *m00,
                    *m01,
                    *m10,
                    *m11,
                    base_transform,
                    registry,
                    shape_text,
                );
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
                draw_glyph_run(
                    pixmap,
                    state,
                    x.to_f32(),
                    y.to_f32(),
                    glyph_ids,
                    advances,
                    *m00,
                    *m01,
                    *m10,
                    *m11,
                    page_height_pt,
                    base_transform,
                    registry,
                );
            }
            Command::DrawRect {
                x,
                y,
                width,
                height,
            } => {
                let draw_y = page_height_pt - y.to_f32() - height.to_f32();
                if let Some(rect) =
                    Rect::from_xywh(x.to_f32(), draw_y, width.to_f32(), height.to_f32())
                {
                    let path = PathBuilder::from_rect(rect);
                    let paint = fill_paint(state.fill_color, state.fill_opacity);
                    pixmap.fill_path(
                        &path,
                        &paint,
                        FillRule::Winding,
                        base_transform.pre_concat(state.transform),
                        state.clip_mask.as_ref(),
                    );
                }
            }
            Command::DrawImage {
                x,
                y,
                width,
                height,
                resource_id,
            } => {
                let source = image_cache
                    .entry(resource_id.clone())
                    .or_insert_with(|| load_image_pixmap(resource_id));
                if let Some(image) = source.as_ref() {
                    let src_w = image.width() as f32;
                    let src_h = image.height() as f32;
                    if src_w > 0.0 && src_h > 0.0 {
                        let sx = width.to_f32() / src_w;
                        let sy = height.to_f32() / src_h;
                        // DrawImage coordinates are top-left based. Convert to user-space with a
                        // local y-flip so source row 0 lands at the visual top, matching PDF /Im Do.
                        let image_ts = Transform::from_row(
                            sx,
                            0.0,
                            0.0,
                            -sy,
                            x.to_f32(),
                            page_height_pt - y.to_f32(),
                        );
                        // Image placement is in local object space; then apply current state CTM.
                        let ctm = state.transform.pre_concat(image_ts);
                        let device_ts = base_transform.pre_concat(ctm);
                        let mut paint = PixmapPaint::default();
                        paint.quality = FilterQuality::Bilinear;
                        paint.opacity = state.fill_opacity.clamp(0.0, 1.0);
                        pixmap.draw_pixmap(
                            0,
                            0,
                            image.as_ref(),
                            &paint,
                            device_ts,
                            state.clip_mask.as_ref(),
                        );
                    }
                }
            }
            Command::DefineForm {
                resource_id,
                width,
                height,
                commands,
            } => {
                forms.insert(
                    resource_id.clone(),
                    FormDefinition {
                        width: *width,
                        height: *height,
                        commands: commands.clone(),
                    },
                );
            }
            Command::DrawForm {
                x,
                y,
                width,
                height,
                resource_id,
            } => {
                let Some(form) = forms.get(resource_id).cloned() else {
                    continue;
                };
                let draw_y = page_height_pt - y.to_f32() - height.to_f32();
                let sx = if form.width.to_f32() > 0.0 {
                    width.to_f32() / form.width.to_f32()
                } else {
                    1.0
                };
                let sy = if form.height.to_f32() > 0.0 {
                    height.to_f32() / form.height.to_f32()
                } else {
                    1.0
                };
                let form_ts = Transform::from_row(sx, 0.0, 0.0, sy, x.to_f32(), draw_y);
                let mut form_state = state.clone();
                // Form commands are emitted in local form space, then mapped by form placement CTM.
                form_state.transform = form_state.transform.post_concat(form_ts);
                let mut form_stack: Vec<RasterState> = Vec::new();
                let mut form_path = PathBuilder::new();
                let mut form_has_path = false;
                render_commands(
                    pixmap,
                    form.height.to_f32(),
                    form.width.to_f32(),
                    &form.commands,
                    base_transform,
                    &mut form_state,
                    &mut form_stack,
                    &mut form_path,
                    &mut form_has_path,
                    forms,
                    image_cache,
                    registry,
                    shape_text,
                )?;
            }
        }
    }
    Ok(())
}

fn fill_current_path(
    pixmap: &mut Pixmap,
    state: &RasterState,
    path_builder: &mut PathBuilder,
    has_path: &mut bool,
    fill_rule: FillRule,
    base_transform: Transform,
) {
    let Some(path) = take_path(path_builder, has_path) else {
        return;
    };
    let paint = fill_paint(state.fill_color, state.fill_opacity);
    pixmap.fill_path(
        &path,
        &paint,
        fill_rule,
        base_transform.pre_concat(state.transform),
        state.clip_mask.as_ref(),
    );
}

fn stroke_current_path(
    pixmap: &mut Pixmap,
    state: &RasterState,
    path_builder: &mut PathBuilder,
    has_path: &mut bool,
    base_transform: Transform,
) {
    let Some(path) = take_path(path_builder, has_path) else {
        return;
    };
    let paint = fill_paint(state.stroke_color, state.stroke_opacity);
    let stroke = build_stroke(state);
    pixmap.stroke_path(
        &path,
        &paint,
        &stroke,
        base_transform.pre_concat(state.transform),
        state.clip_mask.as_ref(),
    );
}

fn fill_stroke_current_path(
    pixmap: &mut Pixmap,
    state: &RasterState,
    path_builder: &mut PathBuilder,
    has_path: &mut bool,
    fill_rule: FillRule,
    base_transform: Transform,
) {
    let Some(path) = take_path(path_builder, has_path) else {
        return;
    };
    let fill = fill_paint(state.fill_color, state.fill_opacity);
    pixmap.fill_path(
        &path,
        &fill,
        fill_rule,
        base_transform.pre_concat(state.transform),
        state.clip_mask.as_ref(),
    );
    let stroke_paint = fill_paint(state.stroke_color, state.stroke_opacity);
    let stroke = build_stroke(state);
    pixmap.stroke_path(
        &path,
        &stroke_paint,
        &stroke,
        base_transform.pre_concat(state.transform),
        state.clip_mask.as_ref(),
    );
}

fn apply_clip_path(
    state: &mut RasterState,
    path: &Path,
    fill_rule: FillRule,
    transform: Transform,
    width: u32,
    height: u32,
) {
    if let Some(mask) = state.clip_mask.as_mut() {
        mask.intersect_path(path, fill_rule, true, transform);
        return;
    }
    let Some(mut mask) = Mask::new(width, height) else {
        return;
    };
    mask.fill_path(path, fill_rule, true, transform);
    state.clip_mask = Some(mask);
}

fn draw_shading_fill(
    pixmap: &mut Pixmap,
    shading: &Shading,
    state: &RasterState,
    page_height_pt: f32,
    page_width_pt: f32,
    base_transform: Transform,
) {
    let Some(page_rect) =
        Rect::from_xywh(0.0, 0.0, page_width_pt.max(0.0), page_height_pt.max(0.0))
    else {
        return;
    };
    let page_path = PathBuilder::from_rect(page_rect);
    let Some(shader) = build_shading_shader(shading, page_height_pt, state.fill_opacity) else {
        return;
    };
    let mut paint = Paint::default();
    paint.shader = shader;
    paint.anti_alias = true;
    pixmap.fill_path(
        &page_path,
        &paint,
        FillRule::Winding,
        base_transform.pre_concat(state.transform),
        state.clip_mask.as_ref(),
    );
}

fn build_shading_shader(
    shading: &Shading,
    page_height_pt: f32,
    opacity: f32,
) -> Option<Shader<'static>> {
    match shading {
        Shading::Axial {
            x0,
            y0,
            x1,
            y1,
            stops,
        } => {
            let start = Point::from_xy(*x0, page_height_pt - *y0);
            let end = Point::from_xy(*x1, page_height_pt - *y1);
            let stops = shading_stops(stops, opacity);
            LinearGradient::new(start, end, stops, SpreadMode::Pad, Transform::identity())
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
            let start = Point::from_xy(*x0, page_height_pt - *y0);
            let end = Point::from_xy(*x1, page_height_pt - *y1);
            let radius = (*r1 - *r0).abs().max(0.0001);
            let stops = shading_stops(stops, opacity);
            RadialGradient::new(
                start,
                end,
                radius,
                stops,
                SpreadMode::Pad,
                Transform::identity(),
            )
        }
    }
}

fn shading_stops(stops: &[ShadingStop], opacity: f32) -> Vec<GradientStop> {
    if stops.is_empty() {
        return vec![
            GradientStop::new(0.0, to_sk_color(Color::BLACK, opacity)),
            GradientStop::new(1.0, to_sk_color(Color::BLACK, opacity)),
        ];
    }
    let mut out = Vec::with_capacity(stops.len());
    for stop in stops {
        out.push(GradientStop::new(
            stop.offset.clamp(0.0, 1.0),
            to_sk_color(stop.color, opacity),
        ));
    }
    out
}

fn draw_string(
    pixmap: &mut Pixmap,
    state: &RasterState,
    x: f32,
    y: f32,
    text: &str,
    page_height_pt: f32,
    base_transform: Transform,
    registry: Option<&FontRegistry>,
    shape_text: bool,
) {
    let debug_text = std::env::var("FULLBLEED_RASTER_DEBUG_TEXT")
        .map(|v| !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"))
        .unwrap_or(false);

    let font_size = state.font_size.to_f32().max(0.0);
    if font_size <= 0.0 {
        return;
    }

    let baseline_x = x;
    let baseline_y = page_height_pt - y - font_size;
    let paint = fill_paint(state.fill_color, state.fill_opacity);
    let device_transform = base_transform.pre_concat(state.transform);
    let mut try_draw = |font_data: &[u8], used_system_fallback: bool| -> Result<(), &'static str> {
        let Ok(face) = ttf_parser::Face::parse(font_data, 0) else {
            return Err("parse_failed");
        };

        let placements = layout_text_glyphs(
            font_data, text, font_size, baseline_x, baseline_y, shape_text,
        );
        if placements.is_empty() {
            return Err("no_placements");
        }
        let first_origin = placements
            .first()
            .map(|p| (p.origin_x, p.origin_y))
            .unwrap_or((baseline_x, baseline_y));

        let mut drawn = 0usize;
        for placement in placements {
            let mut builder =
                GlyphPathBuilder::new(placement.origin_x, placement.origin_y, placement.scale);
            if face
                .outline_glyph(GlyphId(placement.glyph_id), &mut builder)
                .is_none()
            {
                continue;
            }
            let Some(path) = builder.finish() else {
                continue;
            };
            pixmap.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                device_transform,
                state.clip_mask.as_ref(),
            );
            drawn += 1;
        }

        if drawn == 0 {
            return Err("no_outlines");
        }

        if debug_text {
            eprintln!(
                "[raster-text] draw font='{}' fallback={} size={:.2} fill_opacity={:.2} clip={} glyphs={} at=({:.2},{:.2}) first=({:.2},{:.2}) text='{}'",
                state.font_name,
                used_system_fallback,
                font_size,
                state.fill_opacity,
                state.clip_mask.is_some(),
                drawn,
                baseline_x,
                baseline_y,
                first_origin.0,
                first_origin.1,
                truncate_debug_text(text)
            );
        }
        Ok(())
    };

    if let Some(registry) = registry {
        if let Some(font) = registry.resolve(&state.font_name) {
            match try_draw(font.data.as_slice(), false) {
                Ok(()) => return,
                Err(reason) => {
                    if let Some(system_bytes) = resolve_system_font_bytes(&state.font_name) {
                        if try_draw(system_bytes.as_slice(), true).is_ok() {
                            return;
                        }
                    }
                    if debug_text {
                        eprintln!(
                            "[raster-text] skip: {} font='{}' text='{}'",
                            reason,
                            state.font_name,
                            truncate_debug_text(text)
                        );
                    }
                    return;
                }
            }
        }
    }

    let Some(system_bytes) = resolve_system_font_bytes(&state.font_name) else {
        if debug_text {
            eprintln!(
                "[raster-text] skip: unresolved font='{}' text='{}'",
                state.font_name,
                truncate_debug_text(text)
            );
        }
        return;
    };

    if let Err(reason) = try_draw(system_bytes.as_slice(), true) {
        if debug_text {
            eprintln!(
                "[raster-text] skip: {} font='{}' text='{}'",
                reason,
                state.font_name,
                truncate_debug_text(text)
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_string_transformed(
    pixmap: &mut Pixmap,
    state: &RasterState,
    x: f32,
    y: f32,
    text: &str,
    m00: f32,
    m01: f32,
    m10: f32,
    m11: f32,
    base_transform: Transform,
    registry: Option<&FontRegistry>,
    shape_text: bool,
) {
    if text.is_empty() {
        return;
    }

    let font_size = state.font_size.to_f32().max(0.0);
    if font_size <= 0.0 {
        return;
    }

    let paint = fill_paint(state.fill_color, state.fill_opacity);
    let device_transform = base_transform.pre_concat(state.transform);
    let run_transform = Transform::from_row(m00, m01, m10, m11, x, y);

    let mut try_draw = |font_data: &[u8]| -> Result<(), &'static str> {
        let Ok(face) = ttf_parser::Face::parse(font_data, 0) else {
            return Err("parse_failed");
        };
        let placements = layout_text_glyphs(font_data, text, font_size, 0.0, 0.0, shape_text);
        if placements.is_empty() {
            return Err("no_placements");
        }
        let mut drawn = 0usize;
        for placement in placements {
            let mut builder =
                GlyphPathBuilder::new(placement.origin_x, placement.origin_y, placement.scale);
            if face
                .outline_glyph(GlyphId(placement.glyph_id), &mut builder)
                .is_none()
            {
                continue;
            }
            let Some(path) = builder.finish() else {
                continue;
            };
            pixmap.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                device_transform.pre_concat(run_transform),
                state.clip_mask.as_ref(),
            );
            drawn += 1;
        }
        if drawn == 0 {
            return Err("no_outlines");
        }
        Ok(())
    };

    if let Some(registry) = registry {
        if let Some(font) = registry.resolve(&state.font_name) {
            if try_draw(font.data.as_slice()).is_ok() {
                return;
            }
        }
    }
    if let Some(system_bytes) = resolve_system_font_bytes(&state.font_name) {
        let _ = try_draw(system_bytes.as_slice());
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_glyph_run(
    pixmap: &mut Pixmap,
    state: &RasterState,
    x: f32,
    y: f32,
    glyph_ids: &[u16],
    advances: &[(Pt, Pt)],
    m00: f32,
    m01: f32,
    m10: f32,
    m11: f32,
    page_height_pt: f32,
    base_transform: Transform,
    registry: Option<&FontRegistry>,
) {
    if glyph_ids.is_empty() {
        return;
    }

    let debug_text = std::env::var("FULLBLEED_RASTER_DEBUG_TEXT")
        .map(|v| !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"))
        .unwrap_or(false);

    let font_size = state.font_size.to_f32().max(0.0);
    if font_size <= 0.0 {
        return;
    }

    let baseline_x = x;
    let baseline_y = page_height_pt - y;
    let paint = fill_paint(state.fill_color, state.fill_opacity);
    let device_transform = base_transform.pre_concat(state.transform);

    let mut try_draw = |font_data: &[u8], used_system_fallback: bool| -> Result<(), &'static str> {
        let Ok(face) = ttf_parser::Face::parse(font_data, 0) else {
            return Err("parse_failed");
        };
        let upem = face.units_per_em().max(1) as f32;
        let scale = font_size / upem;

        let mut pen_x = baseline_x;
        let mut pen_y = baseline_y;
        let mut drawn = 0usize;
        let mut blank_glyphs = 0usize;
        let mut invalid_glyphs = 0usize;
        for (idx, gid) in glyph_ids.iter().enumerate() {
            if *gid != 0 {
                let mut builder = GlyphPathBuilder::new(0.0, 0.0, scale);
                if face.outline_glyph(GlyphId(*gid), &mut builder).is_some() {
                    if let Some(path) = builder.finish() {
                        let local = Transform::from_row(m00, m01, m10, m11, pen_x, pen_y);
                        pixmap.fill_path(
                            &path,
                            &paint,
                            FillRule::Winding,
                            device_transform.pre_concat(local),
                            state.clip_mask.as_ref(),
                        );
                        drawn += 1;
                    }
                } else if face.glyph_hor_advance(GlyphId(*gid)).is_some() {
                    // Some valid glyphs (e.g. spaces) intentionally have no outline.
                    blank_glyphs += 1;
                } else {
                    invalid_glyphs += 1;
                }
            }

            let (adv_x, adv_y) = advances
                .get(idx)
                .map(|(dx, dy)| (dx.to_f32(), dy.to_f32()))
                .or_else(|| {
                    face.glyph_hor_advance(GlyphId(*gid)).map(|w| {
                        let adv = (w as f32) * scale;
                        (m00 * adv, m01 * adv)
                    })
                })
                .unwrap_or((font_size * 0.5, 0.0));
            if adv_x.is_finite() {
                pen_x += adv_x;
            }
            if adv_y.is_finite() {
                pen_y += adv_y;
            }
        }

        if drawn == 0 {
            // Avoid incorrect system-font fallback for whitespace-only runs where glyph IDs
            // are valid but intentionally outline-less.
            if invalid_glyphs == 0 && blank_glyphs > 0 {
                return Ok(());
            }
            return Err("no_outlines");
        }

        if debug_text {
            eprintln!(
                "[raster-glyph] draw font='{}' fallback={} size={:.2} clip={} glyphs={} at=({:.2},{:.2})",
                state.font_name,
                used_system_fallback,
                font_size,
                state.clip_mask.is_some(),
                drawn,
                baseline_x,
                baseline_y
            );
        }
        Ok(())
    };

    if let Some(registry) = registry {
        if let Some(font) = registry.resolve(&state.font_name) {
            match try_draw(font.data.as_slice(), false) {
                Ok(()) => return,
                Err(_reason) => {
                    if let Some(system_bytes) = resolve_system_font_bytes(&state.font_name) {
                        if try_draw(system_bytes.as_slice(), true).is_ok() {
                            return;
                        }
                    }
                    return;
                }
            }
        }
    }

    let Some(system_bytes) = resolve_system_font_bytes(&state.font_name) else {
        return;
    };
    let _ = try_draw(system_bytes.as_slice(), true);
}

fn truncate_debug_text(text: &str) -> String {
    const MAX_CHARS: usize = 48;
    let mut out = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= MAX_CHARS {
            out.push_str("...");
            break;
        }
        if ch.is_control() {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

#[derive(Clone, Copy)]
struct GlyphPlacement {
    glyph_id: u16,
    origin_x: f32,
    origin_y: f32,
    scale: f32,
}

#[allow(clippy::too_many_arguments)]
fn layout_text_glyphs(
    font_data: &[u8],
    text: &str,
    font_size: f32,
    baseline_x: f32,
    baseline_y: f32,
    shape_text: bool,
) -> Vec<GlyphPlacement> {
    if !shape_text {
        return layout_text_glyphs_unshaped(font_data, text, font_size, baseline_x, baseline_y);
    }

    let Some(face) = HbFace::from_slice(font_data, 0) else {
        return layout_text_glyphs_unshaped(font_data, text, font_size, baseline_x, baseline_y);
    };
    let hb_units = face.units_per_em().max(1) as f32;
    let scale = font_size / hb_units;
    let mut buffer = UnicodeBuffer::new();
    buffer.set_direction(detect_direction(text));
    buffer.push_str(text);
    let output = rustybuzz::shape(&face, &[], buffer);
    let infos = output.glyph_infos();
    let positions = output.glyph_positions();
    if infos.is_empty() || infos.len() != positions.len() {
        return layout_text_glyphs_unshaped(font_data, text, font_size, baseline_x, baseline_y);
    }

    let mut out = Vec::with_capacity(infos.len());
    let mut pen_x = 0.0f32;
    let mut pen_y = 0.0f32;
    for (info, pos) in infos.iter().zip(positions.iter()) {
        let gid = info.glyph_id as u16;
        if gid == 0 {
            pen_x += (pos.x_advance as f32 / hb_units) * font_size;
            pen_y += (pos.y_advance as f32 / hb_units) * font_size;
            continue;
        }
        let x_off = (pos.x_offset as f32 / hb_units) * font_size;
        let y_off = (pos.y_offset as f32 / hb_units) * font_size;
        out.push(GlyphPlacement {
            glyph_id: gid,
            origin_x: baseline_x + pen_x + x_off,
            origin_y: baseline_y + pen_y + y_off,
            scale,
        });
        pen_x += (pos.x_advance as f32 / hb_units) * font_size;
        pen_y += (pos.y_advance as f32 / hb_units) * font_size;
    }
    out
}

fn layout_text_glyphs_unshaped(
    font_data: &[u8],
    text: &str,
    font_size: f32,
    baseline_x: f32,
    baseline_y: f32,
) -> Vec<GlyphPlacement> {
    let Ok(face) = ttf_parser::Face::parse(font_data, 0) else {
        return Vec::new();
    };
    let units_per_em = face.units_per_em().max(1) as f32;
    let scale = font_size / units_per_em;

    let mut out = Vec::new();
    let mut pen_x = 0.0f32;
    for ch in text.chars() {
        let gid = face.glyph_index(ch).map(|id| id.0).unwrap_or(0);
        if gid == 0 {
            pen_x += font_size * 0.5;
            continue;
        }
        out.push(GlyphPlacement {
            glyph_id: gid,
            origin_x: baseline_x + pen_x,
            origin_y: baseline_y,
            scale,
        });
        let advance_units = face.glyph_hor_advance(GlyphId(gid)).unwrap_or(0) as f32;
        let mut adv = (advance_units / units_per_em) * font_size;
        if adv <= 0.0 {
            adv = font_size * 0.5;
        }
        pen_x += adv;
    }
    out
}

static SYSTEM_FONT_CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<Vec<u8>>>>>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FontStyleVariant {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

fn resolve_system_font_bytes(font_name: &str) -> Option<Arc<Vec<u8>>> {
    let families = font_family_candidates(font_name);
    let cache = SYSTEM_FONT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    for family in families {
        let key = normalize_font_family(&family);
        if key.is_empty() {
            continue;
        }

        if let Ok(cache_guard) = cache.lock() {
            if let Some(entry) = cache_guard.get(&key) {
                if let Some(bytes) = entry {
                    return Some(bytes.clone());
                }
                continue;
            }
        }

        let loaded = load_system_font_from_candidates(&family);
        if let Ok(mut cache_guard) = cache.lock() {
            cache_guard.insert(key, loaded.clone());
        }
        if let Some(bytes) = loaded {
            return Some(bytes);
        }
    }

    None
}

fn load_system_font_from_candidates(font_name: &str) -> Option<Arc<Vec<u8>>> {
    let mut candidates = system_font_file_candidates(font_name);
    if candidates.is_empty() {
        // Heuristic fallback: synthesize likely file names from normalized family + style.
        let (family, style) = parse_system_font_request(font_name);
        let normalized = family.replace(' ', "");
        if !normalized.is_empty() {
            match style {
                FontStyleVariant::Regular => {
                    candidates.push(format!("{normalized}.ttf"));
                }
                FontStyleVariant::Bold => {
                    candidates.push(format!("{normalized}Bold.ttf"));
                    candidates.push(format!("{normalized}-Bold.ttf"));
                    candidates.push(format!("{normalized}.ttf"));
                }
                FontStyleVariant::Italic => {
                    candidates.push(format!("{normalized}Italic.ttf"));
                    candidates.push(format!("{normalized}-Italic.ttf"));
                    candidates.push(format!("{normalized}.ttf"));
                }
                FontStyleVariant::BoldItalic => {
                    candidates.push(format!("{normalized}BoldItalic.ttf"));
                    candidates.push(format!("{normalized}-BoldItalic.ttf"));
                    candidates.push(format!("{normalized}BoldOblique.ttf"));
                    candidates.push(format!("{normalized}-BoldOblique.ttf"));
                    candidates.push(format!("{normalized}.ttf"));
                }
            }
        }
    }

    let dirs = system_font_dirs();
    for dir in dirs {
        for file_name in &candidates {
            let path = dir.join(file_name);
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if ttf_parser::Face::parse(&bytes, 0).is_ok() {
                return Some(Arc::new(bytes));
            }
        }
    }
    None
}

fn system_font_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(target_os = "windows")]
    {
        dirs.push(std::path::PathBuf::from(r"C:\Windows\Fonts"));
        if let Ok(windir) = std::env::var("WINDIR") {
            dirs.push(std::path::PathBuf::from(windir).join("Fonts"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        dirs.push(std::path::PathBuf::from("/usr/share/fonts"));
        dirs.push(std::path::PathBuf::from("/usr/local/share/fonts"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(std::path::PathBuf::from(home).join(".fonts"));
        }
    }

    #[cfg(target_os = "macos")]
    {
        dirs.push(std::path::PathBuf::from("/System/Library/Fonts"));
        dirs.push(std::path::PathBuf::from("/Library/Fonts"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(std::path::PathBuf::from(home).join("Library/Fonts"));
        }
    }

    if let Ok(extra) = std::env::var("FULLBLEED_FONT_DIR") {
        for path in std::env::split_paths(&extra) {
            if !path.as_os_str().is_empty() {
                dirs.push(path);
            }
        }
    }

    dirs
}

fn system_font_file_candidates(font_name: &str) -> Vec<String> {
    let (family, style) = parse_system_font_request(font_name);
    let mut out = Vec::new();
    match family.as_str() {
        "system-ui" | "ui-sans-serif" | "sans-serif" => {
            extend_style_candidates(
                &mut out,
                style,
                &[
                    "segoeui.ttf",
                    "arial.ttf",
                    "NotoSans-Regular.ttf",
                    "LiberationSans-Regular.ttf",
                ],
                &[
                    "segoeuib.ttf",
                    "arialbd.ttf",
                    "NotoSans-Bold.ttf",
                    "LiberationSans-Bold.ttf",
                ],
                &[
                    "segoeuii.ttf",
                    "ariali.ttf",
                    "NotoSans-Italic.ttf",
                    "LiberationSans-Italic.ttf",
                ],
                &[
                    "segoeuiz.ttf",
                    "arialbi.ttf",
                    "NotoSans-BoldItalic.ttf",
                    "LiberationSans-BoldItalic.ttf",
                ],
            );
        }
        "ui-monospace" | "monospace" => {
            extend_style_candidates(
                &mut out,
                style,
                &["consola.ttf", "cour.ttf", "LiberationMono-Regular.ttf"],
                &["consolab.ttf", "courbd.ttf", "LiberationMono-Bold.ttf"],
                &["consolai.ttf", "couri.ttf", "LiberationMono-Italic.ttf"],
                &["consolaz.ttf", "courbi.ttf", "LiberationMono-BoldItalic.ttf"],
            );
        }
        "serif" => {
            extend_style_candidates(
                &mut out,
                style,
                &["times.ttf", "timesnewroman.ttf", "LiberationSerif-Regular.ttf"],
                &["timesbd.ttf", "timesnewromanbold.ttf", "LiberationSerif-Bold.ttf"],
                &["timesi.ttf", "timesnewromanitalic.ttf", "LiberationSerif-Italic.ttf"],
                &[
                    "timesbi.ttf",
                    "timesnewromanbolditalic.ttf",
                    "LiberationSerif-BoldItalic.ttf",
                ],
            );
        }
        "segoe ui" => {
            extend_style_candidates(
                &mut out,
                style,
                &["segoeui.ttf"],
                &["segoeuib.ttf"],
                &["segoeuii.ttf"],
                &["segoeuiz.ttf"],
            );
        }
        "roboto" => {
            extend_style_candidates(
                &mut out,
                style,
                &["Roboto-Regular.ttf", "arial.ttf"],
                &["Roboto-Bold.ttf", "arialbd.ttf", "arial.ttf"],
                &["Roboto-Italic.ttf", "ariali.ttf", "arial.ttf"],
                &["Roboto-BoldItalic.ttf", "arialbi.ttf", "arial.ttf"],
            );
        }
        "helvetica" | "helvetica neue" | "arial" => {
            extend_style_candidates(
                &mut out,
                style,
                &["arial.ttf", "LiberationSans-Regular.ttf"],
                &["arialbd.ttf", "LiberationSans-Bold.ttf", "arial.ttf"],
                &["ariali.ttf", "LiberationSans-Italic.ttf", "arial.ttf"],
                &["arialbi.ttf", "LiberationSans-BoldItalic.ttf", "arial.ttf"],
            );
        }
        "arial narrow" | "helvetica narrow" => {
            extend_style_candidates(
                &mut out,
                style,
                &["arialn.ttf", "arial.ttf", "LiberationSans-Regular.ttf"],
                &[
                    "arialnb.ttf",
                    "arialbd.ttf",
                    "arialn.ttf",
                    "LiberationSans-Bold.ttf",
                ],
                &[
                    "arialni.ttf",
                    "ariali.ttf",
                    "arialn.ttf",
                    "LiberationSans-Italic.ttf",
                ],
                &[
                    "arialnbi.ttf",
                    "arialbi.ttf",
                    "arialnb.ttf",
                    "LiberationSans-BoldItalic.ttf",
                ],
            );
        }
        "times" | "times roman" | "times new roman" => {
            extend_style_candidates(
                &mut out,
                style,
                &["times.ttf", "timesnewroman.ttf", "LiberationSerif-Regular.ttf"],
                &["timesbd.ttf", "timesnewromanbold.ttf", "LiberationSerif-Bold.ttf"],
                &["timesi.ttf", "timesnewromanitalic.ttf", "LiberationSerif-Italic.ttf"],
                &[
                    "timesbi.ttf",
                    "timesnewromanbolditalic.ttf",
                    "LiberationSerif-BoldItalic.ttf",
                ],
            );
        }
        "century schoolbook" | "new century schoolbook" => {
            extend_style_candidates(
                &mut out,
                style,
                &["SCHLBK.TTF", "times.ttf", "LiberationSerif-Regular.ttf"],
                &["SCHLBKB.TTF", "timesbd.ttf", "LiberationSerif-Bold.ttf"],
                &["SCHLBKI.TTF", "timesi.ttf", "LiberationSerif-Italic.ttf"],
                &["SCHLBKBI.TTF", "timesbi.ttf", "LiberationSerif-BoldItalic.ttf"],
            );
        }
        "courier" | "courier new" => {
            extend_style_candidates(
                &mut out,
                style,
                &["cour.ttf", "consola.ttf", "LiberationMono-Regular.ttf"],
                &["courbd.ttf", "consolab.ttf", "LiberationMono-Bold.ttf"],
                &["couri.ttf", "consolai.ttf", "LiberationMono-Italic.ttf"],
                &["courbi.ttf", "consolaz.ttf", "LiberationMono-BoldItalic.ttf"],
            );
        }
        "noto sans" => {
            extend_style_candidates(
                &mut out,
                style,
                &["NotoSans-Regular.ttf", "arial.ttf"],
                &["NotoSans-Bold.ttf", "arialbd.ttf", "arial.ttf"],
                &["NotoSans-Italic.ttf", "ariali.ttf", "arial.ttf"],
                &["NotoSans-BoldItalic.ttf", "arialbi.ttf", "arial.ttf"],
            );
        }
        "liberation sans" => {
            extend_style_candidates(
                &mut out,
                style,
                &["LiberationSans-Regular.ttf", "arial.ttf"],
                &["LiberationSans-Bold.ttf", "arialbd.ttf", "arial.ttf"],
                &["LiberationSans-Italic.ttf", "ariali.ttf", "arial.ttf"],
                &["LiberationSans-BoldItalic.ttf", "arialbi.ttf", "arial.ttf"],
            );
        }
        "liberation serif" => {
            extend_style_candidates(
                &mut out,
                style,
                &["LiberationSerif-Regular.ttf", "times.ttf"],
                &["LiberationSerif-Bold.ttf", "timesbd.ttf", "times.ttf"],
                &["LiberationSerif-Italic.ttf", "timesi.ttf", "times.ttf"],
                &["LiberationSerif-BoldItalic.ttf", "timesbi.ttf", "times.ttf"],
            );
        }
        "liberation mono" => {
            extend_style_candidates(
                &mut out,
                style,
                &["LiberationMono-Regular.ttf", "consola.ttf"],
                &["LiberationMono-Bold.ttf", "consolab.ttf", "consola.ttf"],
                &["LiberationMono-Italic.ttf", "consolai.ttf", "consola.ttf"],
                &["LiberationMono-BoldItalic.ttf", "consolaz.ttf", "consola.ttf"],
            );
        }
        _ => {}
    }
    out
}

fn font_family_candidates(font_name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in font_name.split(',') {
        let family = part.trim().trim_matches('"').trim_matches('\'').trim();
        if !family.is_empty() {
            out.push(family.to_string());
        }
    }
    if out.is_empty() {
        out.push(font_name.trim().to_string());
    }
    // Add generic fallback at the end.
    if !out.iter().any(|v| normalize_font_family(v) == "sans-serif") {
        out.push("sans-serif".to_string());
    }
    out
}

fn normalize_font_family(name: &str) -> String {
    name.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_lowercase()
}

fn parse_system_font_request(font_name: &str) -> (String, FontStyleVariant) {
    let normalized = normalize_font_family(font_name).replace('_', " ");
    let without_subset = strip_pdf_subset_prefix(&normalized);
    let style_probe = without_subset
        .replace("boldoblique", "bold oblique")
        .replace("bolditalic", "bold italic")
        .replace("semi-bold", "semibold")
        .replace("demi-bold", "demibold");

    let mut bold = false;
    let mut italic = false;
    let mut condensed = false;
    let mut kept: Vec<&str> = Vec::new();
    for token in style_probe.split(|c: char| c == '-' || c == '_' || c.is_whitespace()) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let mut consumed = false;
        if matches!(token, "bold" | "semibold" | "demibold" | "black" | "blk" | "bd")
            || token.contains("blk")
            || token.contains("black")
        {
            bold = true;
            consumed = true;
        }
        if matches!(token, "italic" | "oblique" | "it") {
            italic = true;
            consumed = true;
        }
        if token == "bi" {
            bold = true;
            italic = true;
            consumed = true;
        }
        if matches!(token, "cn" | "condensed" | "narrow")
            || token.ends_with("cn")
            || token.contains("condensed")
        {
            condensed = true;
            consumed = true;
        }
        if matches!(
            token,
            "regular" | "normal" | "book" | "medium" | "roman" | "mt" | "psmt"
        ) {
            consumed = true;
        }
        if consumed {
            continue;
        }
        kept.push(token);
    }

    let style = match (bold, italic) {
        (true, true) => FontStyleVariant::BoldItalic,
        (true, false) => FontStyleVariant::Bold,
        (false, true) => FontStyleVariant::Italic,
        (false, false) => FontStyleVariant::Regular,
    };

    let mut family = if kept.is_empty() {
        style_probe
    } else {
        kept.join(" ")
    };
    family = canonical_font_family_alias(&family);
    if condensed && matches!(family.as_str(), "helvetica" | "helvetica neue" | "arial") {
        family = "arial narrow".to_string();
    }
    (family, style)
}

fn strip_pdf_subset_prefix(name: &str) -> &str {
    if let Some((prefix, rest)) = name.split_once('+') {
        if prefix.len() == 6 && prefix.chars().all(|c| c.is_ascii_alphabetic()) {
            return rest;
        }
    }
    name
}

fn canonical_font_family_alias(name: &str) -> String {
    let normalized = normalize_font_family(name);
    let compact: String = normalized
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .collect();
    if compact.starts_with("helveticaworld")
        || compact.starts_with("helveticaltstd")
        || compact.starts_with("helveticaneueltstd")
    {
        return "helvetica".to_string();
    }
    if compact.starts_with("newcenturyschlbk") {
        return "century schoolbook".to_string();
    }
    if compact.starts_with("mercurytext") {
        return "times".to_string();
    }
    if compact.starts_with("decimal") {
        return "arial".to_string();
    }
    if compact.starts_with("notosanscjk") {
        return "noto sans".to_string();
    }
    match compact.as_str() {
        "helvetica" | "helveticaneue" => "helvetica".to_string(),
        "arial" | "arialmt" => "arial".to_string(),
        "times" | "timesroman" | "timesnewroman" | "timesnewromanpsmt" => "times".to_string(),
        "courier" | "couriernew" | "couriernewpsmt" => "courier".to_string(),
        "segoeui" => "segoe ui".to_string(),
        "notosans" => "noto sans".to_string(),
        "liberationsans" => "liberation sans".to_string(),
        "liberationserif" => "liberation serif".to_string(),
        "liberationmono" => "liberation mono".to_string(),
        "systemui" => "system-ui".to_string(),
        "uisansserif" => "ui-sans-serif".to_string(),
        "uimonospace" => "ui-monospace".to_string(),
        "sansserif" => "sans-serif".to_string(),
        _ => normalized,
    }
}

fn extend_style_candidates(
    out: &mut Vec<String>,
    style: FontStyleVariant,
    regular: &[&str],
    bold: &[&str],
    italic: &[&str],
    bold_italic: &[&str],
) {
    let groups: [&[&str]; 4] = match style {
        FontStyleVariant::Regular => [regular, bold, italic, bold_italic],
        FontStyleVariant::Bold => [bold, regular, bold_italic, italic],
        FontStyleVariant::Italic => [italic, regular, bold_italic, bold],
        FontStyleVariant::BoldItalic => [bold_italic, bold, italic, regular],
    };
    for group in groups {
        for candidate in group {
            if candidate.is_empty() {
                continue;
            }
            if !out.iter().any(|existing| existing.eq_ignore_ascii_case(candidate)) {
                out.push((*candidate).to_string());
            }
        }
    }
}

fn detect_direction(text: &str) -> HbDirection {
    for ch in text.chars() {
        let code = ch as u32;
        let rtl = matches!(
            code,
            0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x1EE00..=0x1EEFF
        );
        if rtl {
            return HbDirection::RightToLeft;
        }
    }
    HbDirection::LeftToRight
}

struct GlyphPathBuilder {
    builder: PathBuilder,
    origin_x: f32,
    origin_y: f32,
    scale: f32,
}

impl GlyphPathBuilder {
    fn new(origin_x: f32, origin_y: f32, scale: f32) -> Self {
        Self {
            builder: PathBuilder::new(),
            origin_x,
            origin_y,
            scale,
        }
    }

    fn finish(self) -> Option<Path> {
        self.builder.finish()
    }
}

impl OutlineBuilder for GlyphPathBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder.move_to(
            self.origin_x + x * self.scale,
            self.origin_y + y * self.scale,
        );
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.builder.line_to(
            self.origin_x + x * self.scale,
            self.origin_y + y * self.scale,
        );
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.builder.quad_to(
            self.origin_x + x1 * self.scale,
            self.origin_y + y1 * self.scale,
            self.origin_x + x * self.scale,
            self.origin_y + y * self.scale,
        );
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.builder.cubic_to(
            self.origin_x + x1 * self.scale,
            self.origin_y + y1 * self.scale,
            self.origin_x + x2 * self.scale,
            self.origin_y + y2 * self.scale,
            self.origin_x + x * self.scale,
            self.origin_y + y * self.scale,
        );
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

fn take_path(path_builder: &mut PathBuilder, has_path: &mut bool) -> Option<Path> {
    if !*has_path {
        return None;
    }
    *has_path = false;
    let builder = std::mem::replace(path_builder, PathBuilder::new());
    builder.finish()
}

fn build_stroke(state: &RasterState) -> Stroke {
    let mut stroke = Stroke::default();
    stroke.width = state.line_width.to_f32().max(0.0);
    stroke.miter_limit = state.miter_limit.to_f32().max(0.0);
    stroke.line_cap = match state.line_cap {
        1 => LineCap::Round,
        2 => LineCap::Square,
        _ => LineCap::Butt,
    };
    stroke.line_join = match state.line_join {
        1 => LineJoin::Round,
        2 => LineJoin::Bevel,
        _ => LineJoin::Miter,
    };

    if !state.dash_pattern.is_empty() {
        let mut pattern: Vec<f32> = state
            .dash_pattern
            .iter()
            .map(|p| p.abs().to_f32().max(0.0))
            .collect();
        if pattern.len() % 2 == 1 {
            let copy = pattern.clone();
            pattern.extend(copy);
        }
        if pattern.len() >= 2 {
            if let Some(dash) = StrokeDash::new(pattern, state.dash_phase.to_f32()) {
                stroke.dash = Some(dash);
            }
        }
    }

    stroke
}

fn fill_paint(color: Color, opacity: f32) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(to_sk_color(color, opacity));
    paint.anti_alias = true;
    paint
}

fn to_sk_color(color: Color, opacity: f32) -> tiny_skia::Color {
    let r = color.r.clamp(0.0, 1.0);
    let g = color.g.clamp(0.0, 1.0);
    let b = color.b.clamp(0.0, 1.0);
    let a = opacity.clamp(0.0, 1.0);
    tiny_skia::Color::from_rgba(r, g, b, a)
        .unwrap_or_else(|| tiny_skia::Color::from_rgba8(0, 0, 0, 255))
}

fn pt_milli_to_px_u32(pt_milli: i64, dpi: u32) -> Result<u32, FullBleedError> {
    let px = pt_milli_to_px_i64(pt_milli, dpi)?;
    if px <= 0 {
        return Err(FullBleedError::InvalidConfiguration(format!(
            "invalid non-positive pixel dimension {px} for pt_milli={pt_milli} dpi={dpi}"
        )));
    }
    u32::try_from(px).map_err(|_| {
        FullBleedError::InvalidConfiguration(format!(
            "pixel dimension out of range: {px} for pt_milli={pt_milli} dpi={dpi}"
        ))
    })
}

fn pt_milli_to_px_i64(pt_milli: i64, dpi: u32) -> Result<i64, FullBleedError> {
    if dpi == 0 {
        return Err(FullBleedError::InvalidConfiguration(
            "dpi must be > 0".to_string(),
        ));
    }

    let num = (pt_milli as i128).saturating_mul(dpi as i128);
    let den = 72_000_i128;
    let px = if num >= 0 {
        (num + (den / 2)) / den
    } else {
        -(((-num) + (den / 2)) / den)
    };
    i64::try_from(px).map_err(|_| {
        FullBleedError::InvalidConfiguration(format!(
            "pixel conversion overflow: pt_milli={pt_milli} dpi={dpi}"
        ))
    })
}

fn load_image_pixmap(source: &str) -> Option<Pixmap> {
    if let Some((mime, data)) = parse_data_uri(source) {
        return decode_image_to_pixmap(&data, Some(&mime));
    }

    let path = FsPath::new(source);
    let bytes = std::fs::read(path).ok()?;
    decode_image_to_pixmap(&bytes, None)
}

fn decode_image_to_pixmap(data: &[u8], mime: Option<&str>) -> Option<Pixmap> {
    let guessed_format = if let Some(mime) = mime {
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

    let decoded = if let Some(fmt) = guessed_format {
        image::load_from_memory_with_format(data, fmt).ok()?
    } else {
        image::load_from_memory(data).ok()?
    };
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut pixmap = Pixmap::new(width, height)?;
    let src = rgba.as_raw();
    let dst = pixmap.data_mut();
    for (src_px, dst_px) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        let r = src_px[0];
        let g = src_px[1];
        let b = src_px[2];
        let a = src_px[3];
        dst_px[0] = premul_u8(r, a);
        dst_px[1] = premul_u8(g, a);
        dst_px[2] = premul_u8(b, a);
        dst_px[3] = a;
    }
    Some(pixmap)
}

fn premul_u8(channel: u8, alpha: u8) -> u8 {
    let prod = (channel as u16) * (alpha as u16) + 127;
    ((prod + (prod >> 8)) >> 8) as u8
}

fn parse_data_uri(uri: &str) -> Option<(String, Vec<u8>)> {
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
        .to_string();
    let data = if header.contains(";base64") {
        base64::engine::general_purpose::STANDARD
            .decode(payload)
            .ok()?
    } else {
        payload.as_bytes().to_vec()
    };
    Some((mime, data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use image::RgbaImage;

    fn has_non_white_pixel(img: &image::RgbaImage) -> bool {
        img.pixels().any(|p| {
            let [r, g, b, _a] = p.0;
            !(r == 255 && g == 255 && b == 255)
        })
    }

    #[test]
    fn pt_milli_to_px_rounds_half_away_from_zero() {
        assert_eq!(pt_milli_to_px_i64(72_000, 150).unwrap(), 150);
        assert_eq!(pt_milli_to_px_i64(240, 150).unwrap(), 1);
        assert_eq!(pt_milli_to_px_i64(-240, 150).unwrap(), -1);
        assert_eq!(pt_milli_to_px_i64(239, 150).unwrap(), 0);
        assert_eq!(pt_milli_to_px_i64(-239, 150).unwrap(), 0);
    }

    #[test]
    fn parse_data_uri_base64_decodes_payload() {
        let uri = "data:text/plain;base64,SGVsbG8=";
        let (mime, data) = parse_data_uri(uri).unwrap();
        assert_eq!(mime, "text/plain");
        assert_eq!(data, b"Hello");
    }

    #[test]
    fn decode_image_to_pixmap_handles_png() {
        let mut src = RgbaImage::new(1, 1);
        src.put_pixel(0, 0, image::Rgba([255, 0, 0, 128]));
        let mut bytes = Vec::new();
        src.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        let pixmap = decode_image_to_pixmap(&bytes, Some("image/png")).unwrap();
        assert_eq!(pixmap.width(), 1);
        assert_eq!(pixmap.height(), 1);
    }

    #[test]
    fn text_raster_fallback_draws_non_white_pixels() {
        let doc = Document {
            page_size: crate::types::Size::from_inches(8.5, 11.0),
            pages: vec![crate::canvas::Page {
                commands: vec![
                    Command::SetFillColor(Color::BLACK),
                    Command::SetFontName("Helvetica-Bold".to_string()),
                    Command::SetFontSize(Pt::from_f32(24.0)),
                    Command::DrawString {
                        x: Pt::from_f32(72.0),
                        y: Pt::from_f32(72.0),
                        text: "Hello".to_string(),
                    },
                ],
            }],
        };
        let pngs = document_to_png_pages(&doc, 150, None, true).unwrap();
        assert_eq!(pngs.len(), 1);
        let img = image::load_from_memory(&pngs[0]).unwrap().to_rgba8();
        assert!(
            has_non_white_pixel(&img),
            "expected text to produce non-white pixels"
        );
    }

    #[test]
    fn system_font_candidates_prefer_bold_variant_for_helvetica() {
        let candidates = system_font_file_candidates("Helvetica-Bold");
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0], "arialbd.ttf");
        assert!(candidates.iter().any(|v| v.eq_ignore_ascii_case("arial.ttf")));
    }

    #[test]
    fn system_font_candidates_normalize_subset_prefix_and_style() {
        let candidates = system_font_file_candidates("ABCDEF+Helvetica-BoldOblique");
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0], "arialbi.ttf");
    }

    #[test]
    fn system_font_candidates_alias_helvetica_world_family() {
        let candidates = system_font_file_candidates("ABCDEF+HelveticaWorld-Bold");
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0], "arialbd.ttf");
    }

    #[test]
    fn system_font_candidates_treat_helvetica_lt_std_blk_as_bold() {
        let candidates = system_font_file_candidates("ABCDEF+HelveticaLTStd-Blk");
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0], "arialbd.ttf");
    }

    #[test]
    fn system_font_candidates_treat_bd_shorthand_as_bold() {
        let candidates = system_font_file_candidates("ABCDEF+HelveticaNeueLTStd-Bd");
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0], "arialbd.ttf");
    }

    #[test]
    fn system_font_candidates_treat_cn_shorthand_as_narrow() {
        let candidates = system_font_file_candidates("ABCDEF+HelveticaNeueLTStd-Cn");
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0], "arialn.ttf");
    }

    #[test]
    fn system_font_candidates_treat_blkcn_as_bold_narrow() {
        let candidates = system_font_file_candidates("ABCDEF+HelveticaNeueLTStd-BlkCn");
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0], "arialnb.ttf");
    }

    #[test]
    fn system_font_candidates_treat_it_shorthand_as_italic() {
        let candidates = system_font_file_candidates("ABCDEF+HelveticaNeueLTStd-It");
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0], "ariali.ttf");
    }

    #[test]
    fn system_font_candidates_treat_bi_shorthand_as_bold_italic() {
        let candidates = system_font_file_candidates("ABCDEF+HelveticaNeueLTStd-Bi");
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0], "arialbi.ttf");
    }

    #[test]
    fn missing_image_is_noop_and_does_not_paint_placeholder() {
        let doc = Document {
            page_size: crate::types::Size::from_inches(2.0, 2.0),
            pages: vec![crate::canvas::Page {
                commands: vec![
                    Command::SetFillColor(Color::rgb(1.0, 0.0, 0.0)),
                    Command::DrawImage {
                        x: Pt::from_f32(20.0),
                        y: Pt::from_f32(20.0),
                        width: Pt::from_f32(40.0),
                        height: Pt::from_f32(30.0),
                        resource_id: "missing-image-for-raster-parity-should-not-exist.png"
                            .to_string(),
                    },
                ],
            }],
        };

        let pngs = document_to_png_pages(&doc, 72, None, true).unwrap();
        assert_eq!(pngs.len(), 1);
        let img = image::load_from_memory(&pngs[0]).unwrap().to_rgba8();
        let px = img.get_pixel(40, 35).0;
        assert_eq!(px, [255, 255, 255, 255]);
    }

    #[test]
    fn draw_form_with_embedded_image_rasters_non_white_pixels() {
        let mut form_canvas = crate::canvas::Canvas::new(crate::types::Size::from_inches(2.0, 1.0));
        form_canvas.draw_image(
            Pt::ZERO,
            Pt::ZERO,
            Pt::from_f32(120.0),
            Pt::from_f32(48.0),
            "examples/img/full_bleed-logo_small.png",
        );
        let form_doc = form_canvas.finish();
        let form_commands = form_doc
            .pages
            .first()
            .map(|p| p.commands.clone())
            .unwrap_or_default();

        let doc = Document {
            page_size: crate::types::Size::from_inches(8.5, 11.0),
            pages: vec![crate::canvas::Page {
                commands: vec![
                    Command::DefineForm {
                        resource_id: "test-form-img".to_string(),
                        width: Pt::from_f32(120.0),
                        height: Pt::from_f32(48.0),
                        commands: form_commands,
                    },
                    Command::DrawForm {
                        x: Pt::from_f32(72.0),
                        y: Pt::from_f32(72.0),
                        width: Pt::from_f32(120.0),
                        height: Pt::from_f32(48.0),
                        resource_id: "test-form-img".to_string(),
                    },
                ],
            }],
        };

        let pngs = document_to_png_pages(&doc, 144, None, true).unwrap();
        assert_eq!(pngs.len(), 1);
        let img = image::load_from_memory(&pngs[0]).unwrap().to_rgba8();
        assert!(
            has_non_white_pixel(&img),
            "expected DrawForm containing DrawImage to render non-white pixels"
        );
    }

    #[test]
    fn draw_image_preserves_top_to_bottom_source_orientation() {
        let mut src = RgbaImage::new(1, 2);
        src.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        src.put_pixel(0, 1, image::Rgba([0, 0, 255, 255]));
        let mut bytes = Vec::new();
        src.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        let data_uri = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        );

        let doc = Document {
            page_size: crate::types::Size {
                width: Pt::from_f32(72.0),
                height: Pt::from_f32(72.0),
            },
            pages: vec![crate::canvas::Page {
                commands: vec![Command::DrawImage {
                    x: Pt::from_f32(10.0),
                    y: Pt::from_f32(10.0),
                    width: Pt::from_f32(20.0),
                    height: Pt::from_f32(20.0),
                    resource_id: data_uri,
                }],
            }],
        };

        let pngs = document_to_png_pages(&doc, 72, None, true).unwrap();
        assert_eq!(pngs.len(), 1);
        let img = image::load_from_memory(&pngs[0]).unwrap().to_rgba8();
        let top = img.get_pixel(20, 13).0;
        let bottom = img.get_pixel(20, 27).0;
        assert!(
            top[0] > top[2],
            "expected top sample to preserve red source row, got {:?}",
            top
        );
        assert!(
            bottom[2] > bottom[0],
            "expected bottom sample to preserve blue source row, got {:?}",
            bottom
        );
    }
}
