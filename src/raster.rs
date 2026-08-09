use crate::canvas::{Command, CompiledMaskLayer, Document};
use crate::error::FullBleedError;
use crate::flowable::{
    FilterDropShadowSpec, MaskComposite, MaskMode, PaintFilterOperation, PaintFilterSpec,
    SvgComponentTransferFunction, SvgFilterInput, SvgFilterPrimitive, SvgFilterProgram,
    SvgMorphologyOperator,
};
use crate::font::FontRegistry;
use crate::raster_native::{
    BlendMode as SkBlendMode, Color as RasterColor, ConicGradient, FillRule, FilterQuality,
    GradientStop, IntSize, LineCap, LineJoin, LinearGradient, Mask, Paint, Path, PathBuilder,
    Pixmap, PixmapPaint, PixmapRef, Point, RadialGradient, Rect, Shader, SpreadMode, Stroke,
    StrokeDash, Transform,
};
use crate::sfnt::{Face as SfntFace, GlyphId as SfntGlyphId};
use crate::sfnt_cff::{Cff2Outlines, CffOutlines};
use crate::sfnt_outline::OutlineBuilder as NativeOutlineBuilder;
use crate::text_shape;
use crate::types::{Color, MixBlendMode, Pt, Shading, ShadingStop};
use std::collections::{HashMap, VecDeque};
use std::path::Path as FsPath;
use std::sync::{Arc, Mutex, OnceLock};

type PixelBounds = (u32, u32, u32, u32);

// The authenticated Chrome PDF oracle samples CSS mask shaders on a half
// 300-DPI device-pixel phase relative to our analytic pixel-center convention.
const MASK_SHADER_PHASE_PT: f32 = 0.5 * 72.0 / 300.0;

fn filter_retains_css_surface_guard(filter: &PaintFilterSpec) -> bool {
    if !filter.operations.is_empty() {
        let mut has_adjustment = false;
        for operation in &filter.operations {
            match operation {
                PaintFilterOperation::Saturate(_)
                | PaintFilterOperation::Brightness(_)
                | PaintFilterOperation::Contrast(_)
                | PaintFilterOperation::Invert(_)
                | PaintFilterOperation::Sepia(_)
                | PaintFilterOperation::HueRotate(_)
                | PaintFilterOperation::Opacity(_) => has_adjustment = true,
                PaintFilterOperation::DropShadow(shadow) if shadow.blur_radius <= Pt::ZERO => {}
                _ => return false,
            }
        }
        return has_adjustment;
    }

    filter.blur_radius <= Pt::ZERO
        && filter
            .drop_shadows
            .iter()
            .all(|shadow| shadow.blur_radius <= Pt::ZERO)
        && ((filter.saturate - 1.0).abs() > 1.0e-6
            || (filter.brightness - 1.0).abs() > 1.0e-6
            || (filter.contrast - 1.0).abs() > 1.0e-6
            || filter.invert.abs() > 1.0e-6
            || filter.sepia.abs() > 1.0e-6
            || filter.hue_rotate.abs() > 1.0e-6
            || (filter.opacity - 1.0).abs() > 1.0e-6)
}

fn filter_surface_guard_uses_effect_bounds(filter: &PaintFilterSpec) -> bool {
    if !filter.operations.is_empty() {
        filter
            .operations
            .iter()
            .any(|operation| matches!(operation, PaintFilterOperation::DropShadow(_)))
    } else {
        !filter.drop_shadows.is_empty()
    }
}

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
    blend_mode: MixBlendMode,
    font_name: String,
    font_size: Pt,
    text_rendering_mode: u8,
    clip_mask: Option<Mask>,
    mask_shader_phase: (f32, f32),
    filtered_output_bounds: Option<PixelBounds>,
    discrete_image_sampling: bool,
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
            blend_mode: MixBlendMode::Normal,
            font_name: "Helvetica".to_string(),
            font_size: Pt::from_f32(12.0),
            text_rendering_mode: 0,
            clip_mask: None,
            mask_shader_phase: (0.0, 0.0),
            filtered_output_bounds: None,
            discrete_image_sampling: false,
        }
    }
}

#[derive(Clone)]
struct FormDefinition {
    width: Pt,
    height: Pt,
    isolated: bool,
    commands: Vec<Command>,
}

#[derive(Clone)]
pub(crate) struct FilteredFormRaster {
    pub x: Pt,
    pub y: Pt,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub premultiplied_rgba: Vec<u8>,
}

#[derive(Clone)]
pub(crate) struct MaskedFormRasterLayer {
    pub program: CompiledMaskLayer,
    pub width: Pt,
    pub height: Pt,
    pub commands: Vec<Command>,
}

#[derive(Clone)]
pub(crate) struct MaskCoverageRaster {
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub coverage: Vec<u8>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn rasterize_filtered_form(
    page_width: Pt,
    page_height: Pt,
    form_width: Pt,
    form_height: Pt,
    commands: &[Command],
    x: Pt,
    y: Pt,
    width: Pt,
    height: Pt,
    filter: &PaintFilterSpec,
    css_shadow: bool,
    dpi: u32,
    registry: Option<&FontRegistry>,
    shape_text: bool,
) -> Result<Option<FilteredFormRaster>, FullBleedError> {
    let dpi = dpi.max(72);
    let full_width_px = pt_milli_to_px_u32(page_width.to_milli_i64(), dpi)? as i64;
    let full_height_px = pt_milli_to_px_u32(page_height.to_milli_i64(), dpi)? as i64;
    // Blink's adjustment filters, including chains followed by a zero-blur
    // shadow, allocate a one-device-pixel transparent guard around the source
    // surface. Retaining that compiled metadata keeps downstream PDF image
    // interpolation on the same lattice without adding work to the kernels.
    let surface_guard_px = i64::from(filter_retains_css_surface_guard(filter));
    let surface_guard_uses_effect_bounds =
        surface_guard_px != 0 && filter_surface_guard_uses_effect_bounds(filter);
    let coordinate_to_floor_pixel = |coordinate: Pt| -> i64 {
        let numerator = i128::from(coordinate.to_milli_i64()) * i128::from(dpi);
        numerator.div_euclid(72_000) as i64
    };
    let coordinate_to_ceil_pixel = |coordinate: Pt| -> i64 {
        let numerator = i128::from(coordinate.to_milli_i64()) * i128::from(dpi);
        (-(-numerator).div_euclid(72_000)) as i64
    };
    let minimum_surface_pixel = -surface_guard_px;
    let maximum_surface_x = full_width_px + surface_guard_px;
    let maximum_surface_y = full_height_px + surface_guard_px;
    let left_px = (coordinate_to_floor_pixel(x) - surface_guard_px)
        .clamp(minimum_surface_pixel, maximum_surface_x);
    let top_px = (coordinate_to_floor_pixel(y) - surface_guard_px)
        .clamp(minimum_surface_pixel, maximum_surface_y);
    let right_px = (coordinate_to_ceil_pixel(x + width) + surface_guard_px)
        .clamp(minimum_surface_pixel, maximum_surface_x);
    let bottom_px = (coordinate_to_ceil_pixel(y + height) + surface_guard_px)
        .clamp(minimum_surface_pixel, maximum_surface_y);
    if left_px >= right_px || top_px >= bottom_px {
        return Ok(None);
    }
    let width_px = (right_px - left_px) as u32;
    let height_px = (bottom_px - top_px) as u32;
    let pixel_to_pt = |pixel: i64| -> Pt {
        let numerator = i128::from(pixel) * 72_000;
        let adjustment = if numerator >= 0 {
            i128::from(dpi / 2)
        } else {
            -i128::from(dpi / 2)
        };
        let milli = (numerator + adjustment) / i128::from(dpi);
        Pt::from_milli_i64(milli as i64)
    };
    let origin_x = pixel_to_pt(left_px);
    let origin_y = pixel_to_pt(top_px);
    let raster_page_width = pixel_to_pt(i64::from(width_px));
    let raster_page_height = pixel_to_pt(i64::from(height_px));
    let mut pixmap = Pixmap::new(width_px, height_px).ok_or_else(|| {
        FullBleedError::InvalidConfiguration(format!(
            "invalid filtered-form raster size {}x{} at {} DPI",
            width_px, height_px, dpi
        ))
    })?;

    let page_height_pt = raster_page_height.to_f32();
    let page_width_pt = raster_page_width.to_f32();
    let scale = dpi as f32 / 72.0;
    let base_transform = Transform::from_row(scale, 0.0, 0.0, -scale, 0.0, page_height_pt * scale);
    let resource_id = "__fullbleed_filtered_form".to_string();
    let mut forms = HashMap::new();
    forms.insert(
        resource_id.clone(),
        FormDefinition {
            width: form_width,
            height: form_height,
            isolated: false,
            commands: commands.to_vec(),
        },
    );
    let draw = [Command::DrawFilteredForm {
        x: x - origin_x,
        y: y - origin_y,
        width,
        height,
        resource_id,
        filter: filter.clone(),
        css_shadow,
    }];
    let mut image_cache = HashMap::new();
    let mut state = RasterState::default();
    state.discrete_image_sampling = true;
    let mut stack = Vec::new();
    let mut path_builder = PathBuilder::new();
    let mut has_path = false;
    crate::raster_native::with_precise_antialias(|| {
        render_commands(
            &mut pixmap,
            page_height_pt,
            page_width_pt,
            &draw,
            base_transform,
            &mut state,
            &mut stack,
            &mut path_builder,
            &mut has_path,
            &mut forms,
            &mut image_cache,
            registry,
            shape_text,
        )
    })?;

    let mut min_x = width_px;
    let mut min_y = height_px;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    for (index, pixel) in pixmap.data().chunks_exact(4).enumerate() {
        if pixel[3] == 0 {
            continue;
        }
        let px = index as u32 % width_px;
        let py = index as u32 / width_px;
        min_x = min_x.min(px);
        min_y = min_y.min(py);
        max_x = max_x.max(px + 1);
        max_y = max_y.max(py + 1);
    }
    if min_x >= max_x || min_y >= max_y {
        return Ok(None);
    }
    if let Some((left, top, right, bottom)) = state.filtered_output_bounds {
        let left = left.min(width_px);
        let top = top.min(height_px);
        let right = right.min(width_px);
        let bottom = bottom.min(height_px);
        if left < right && top < bottom {
            min_x = left;
            min_y = top;
            max_x = right;
            max_y = bottom;
        }
    }
    if surface_guard_px != 0 {
        if surface_guard_uses_effect_bounds {
            min_x = min_x.saturating_sub(1);
            min_y = min_y.saturating_sub(1);
            max_x = max_x.saturating_add(1).min(width_px);
            max_y = max_y.saturating_add(1).min(height_px);
        } else {
            min_x = 0;
            min_y = 0;
            max_x = width_px;
            max_y = height_px;
        }
    }

    let crop_width = max_x - min_x;
    let crop_height = max_y - min_y;
    let source_stride = width_px as usize * 4;
    let crop_stride = crop_width as usize * 4;
    let mut cropped = vec![0u8; crop_stride * crop_height as usize];
    for row in 0..crop_height as usize {
        let source_start = (min_y as usize + row) * source_stride + min_x as usize * 4;
        let target_start = row * crop_stride;
        cropped[target_start..target_start + crop_stride]
            .copy_from_slice(&pixmap.data()[source_start..source_start + crop_stride]);
    }

    let points_per_pixel = 72.0 / dpi as f32;
    Ok(Some(FilteredFormRaster {
        x: origin_x + Pt::from_f32(min_x as f32 * points_per_pixel),
        y: origin_y + Pt::from_f32(min_y as f32 * points_per_pixel),
        pixel_width: crop_width,
        pixel_height: crop_height,
        premultiplied_rgba: cropped,
    }))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn rasterize_masked_form(
    page_width: Pt,
    page_height: Pt,
    form_width: Pt,
    form_height: Pt,
    commands: &[Command],
    layers: &[MaskedFormRasterLayer],
    form_definitions: &HashMap<String, (Pt, Pt, Vec<Command>)>,
    form_isolated: &HashMap<String, bool>,
    x: Pt,
    y: Pt,
    width: Pt,
    height: Pt,
    dpi: u32,
    registry: Option<&FontRegistry>,
    shape_text: bool,
) -> Result<Option<FilteredFormRaster>, FullBleedError> {
    let dpi = dpi.max(72);
    let full_width_px = pt_milli_to_px_u32(page_width.to_milli_i64(), dpi)? as i64;
    let full_height_px = pt_milli_to_px_u32(page_height.to_milli_i64(), dpi)? as i64;
    let coordinate_to_floor_pixel = |coordinate: Pt| -> i64 {
        let numerator = i128::from(coordinate.to_milli_i64()) * i128::from(dpi);
        numerator.div_euclid(72_000) as i64
    };
    let coordinate_to_ceil_pixel = |coordinate: Pt| -> i64 {
        let numerator = i128::from(coordinate.to_milli_i64()) * i128::from(dpi);
        (-(-numerator).div_euclid(72_000)) as i64
    };
    let left_px = coordinate_to_floor_pixel(x).clamp(0, full_width_px);
    let top_px = coordinate_to_floor_pixel(y).clamp(0, full_height_px);
    let right_px = coordinate_to_ceil_pixel(x + width).clamp(0, full_width_px);
    let bottom_px = coordinate_to_ceil_pixel(y + height).clamp(0, full_height_px);
    if left_px >= right_px || top_px >= bottom_px {
        return Ok(None);
    }
    let width_px = (right_px - left_px) as u32;
    let height_px = (bottom_px - top_px) as u32;
    let pixel_to_pt = |pixel: i64| -> Pt {
        let numerator = i128::from(pixel) * 72_000;
        let milli = (numerator + i128::from(dpi / 2)) / i128::from(dpi);
        Pt::from_milli_i64(milli as i64)
    };
    let origin_x = pixel_to_pt(left_px);
    let origin_y = pixel_to_pt(top_px);
    let raster_page_width = pixel_to_pt(i64::from(width_px));
    let raster_page_height = pixel_to_pt(i64::from(height_px));
    let mut pixmap = Pixmap::new(width_px, height_px).ok_or_else(|| {
        FullBleedError::InvalidConfiguration(format!(
            "invalid masked-form raster size {}x{} at {} DPI",
            width_px, height_px, dpi
        ))
    })?;

    let page_height_pt = raster_page_height.to_f32();
    let page_width_pt = raster_page_width.to_f32();
    let scale = dpi as f32 / 72.0;
    let base_transform = Transform::from_row(scale, 0.0, 0.0, -scale, 0.0, page_height_pt * scale);
    let source_id = "__fullbleed_masked_form_source".to_string();
    let mut forms = HashMap::new();
    for (resource_id, (width, height, commands)) in form_definitions {
        forms.insert(
            resource_id.clone(),
            FormDefinition {
                width: *width,
                height: *height,
                isolated: form_isolated.get(resource_id).copied().unwrap_or(false),
                commands: commands.clone(),
            },
        );
    }
    forms.insert(
        source_id.clone(),
        FormDefinition {
            width: form_width,
            height: form_height,
            isolated: true,
            commands: commands.to_vec(),
        },
    );
    for layer in layers {
        forms.insert(
            layer.program.resource_id.clone(),
            FormDefinition {
                width: layer.width,
                height: layer.height,
                isolated: false,
                commands: layer.commands.clone(),
            },
        );
    }
    let draw = [Command::DrawMaskedForm {
        x: x - origin_x,
        y: y - origin_y,
        width,
        height,
        resource_id: source_id,
        layers: layers.iter().map(|layer| layer.program.clone()).collect(),
    }];
    let mut image_cache = HashMap::new();
    let mut state = RasterState::default();
    let mut stack = Vec::new();
    let mut path_builder = PathBuilder::new();
    let mut has_path = false;
    crate::raster_native::with_precise_antialias(|| {
        render_commands(
            &mut pixmap,
            page_height_pt,
            page_width_pt,
            &draw,
            base_transform,
            &mut state,
            &mut stack,
            &mut path_builder,
            &mut has_path,
            &mut forms,
            &mut image_cache,
            registry,
            shape_text,
        )
    })?;

    let mut min_x = width_px;
    let mut min_y = height_px;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    for (index, pixel) in pixmap.data().chunks_exact(4).enumerate() {
        if pixel[3] == 0 {
            continue;
        }
        let px = index as u32 % width_px;
        let py = index as u32 / width_px;
        min_x = min_x.min(px);
        min_y = min_y.min(py);
        max_x = max_x.max(px + 1);
        max_y = max_y.max(py + 1);
    }
    if min_x >= max_x || min_y >= max_y {
        return Ok(None);
    }

    let crop_width = max_x - min_x;
    let crop_height = max_y - min_y;
    let source_stride = width_px as usize * 4;
    let crop_stride = crop_width as usize * 4;
    let mut cropped = vec![0u8; crop_stride * crop_height as usize];
    for row in 0..crop_height as usize {
        let source_start = (min_y as usize + row) * source_stride + min_x as usize * 4;
        let target_start = row * crop_stride;
        cropped[target_start..target_start + crop_stride]
            .copy_from_slice(&pixmap.data()[source_start..source_start + crop_stride]);
    }

    let points_per_pixel = 72.0 / dpi as f32;
    Ok(Some(FilteredFormRaster {
        x: origin_x + Pt::from_f32(min_x as f32 * points_per_pixel),
        y: origin_y + Pt::from_f32(min_y as f32 * points_per_pixel),
        pixel_width: crop_width,
        pixel_height: crop_height,
        premultiplied_rgba: cropped,
    }))
}

/// Execute only the immutable mask program into a local grayscale coverage
/// surface. PDF emission can apply this cached surface to a vector source form,
/// so variable-data changes do not invalidate or rerun mask compilation.
#[allow(clippy::too_many_arguments)]
pub(crate) fn rasterize_mask_coverage(
    layers: &[MaskedFormRasterLayer],
    width: Pt,
    height: Pt,
    dpi: u32,
    registry: Option<&FontRegistry>,
    shape_text: bool,
) -> Result<Option<MaskCoverageRaster>, FullBleedError> {
    let dpi = dpi.max(72);
    let pixel_width = pt_milli_to_px_u32(width.to_milli_i64(), dpi)?;
    let pixel_height = pt_milli_to_px_u32(height.to_milli_i64(), dpi)?;
    let page_width_pt = width.to_f32();
    let page_height_pt = height.to_f32();
    let scale = dpi as f32 / 72.0;
    let base_transform = Transform::from_row(scale, 0.0, 0.0, -scale, 0.0, page_height_pt * scale);

    let mut forms = HashMap::new();
    for layer in layers {
        forms.insert(
            layer.program.resource_id.clone(),
            FormDefinition {
                width: layer.width,
                height: layer.height,
                isolated: false,
                commands: layer.commands.clone(),
            },
        );
    }
    let programs = layers
        .iter()
        .map(|layer| layer.program.clone())
        .collect::<Vec<_>>();
    let mut image_cache = HashMap::new();
    let state = RasterState::default();
    let pdf_mask_shader_phase = (MASK_SHADER_PHASE_PT, MASK_SHADER_PHASE_PT);
    let coverage = crate::raster_native::with_precise_antialias(|| {
        render_mask_coverage(
            pixel_width,
            pixel_height,
            page_height_pt,
            page_width_pt,
            &programs,
            Pt::ZERO,
            Pt::ZERO,
            width,
            height,
            base_transform,
            &state,
            pdf_mask_shader_phase,
            &mut forms,
            &mut image_cache,
            registry,
            shape_text,
        )
    })?;
    let Some(coverage) = coverage else {
        return Ok(None);
    };
    let coverage = coverage
        .into_iter()
        .map(|alpha| (alpha * 255.0).round().clamp(0.0, 255.0) as u8)
        .collect::<Vec<_>>();
    if !coverage.iter().any(|alpha| *alpha != 0) {
        return Ok(None);
    }
    Ok(Some(MaskCoverageRaster {
        pixel_width,
        pixel_height,
        coverage,
    }))
}

pub(crate) fn document_to_png_pages(
    document: &Document,
    dpi: u32,
    registry: Option<&FontRegistry>,
    shape_text: bool,
) -> Result<Vec<Vec<u8>>, FullBleedError> {
    document_to_png_pages_with_background(document, dpi, registry, shape_text, false)
}

pub(crate) fn document_to_transparent_png_pages(
    document: &Document,
    dpi: u32,
    registry: Option<&FontRegistry>,
    shape_text: bool,
) -> Result<Vec<Vec<u8>>, FullBleedError> {
    document_to_png_pages_with_background(document, dpi, registry, shape_text, true)
}

fn document_to_png_pages_with_background(
    document: &Document,
    dpi: u32,
    registry: Option<&FontRegistry>,
    shape_text: bool,
    transparent: bool,
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
        if !transparent {
            pixmap.fill(RasterColor::from_rgba8(255, 255, 255, 255));
        }

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

        let png = crate::image_native::encode_png_premultiplied_rgba8(
            pixmap.data(),
            pixmap.width(),
            pixmap.height(),
        )
        .map_err(|error| FullBleedError::Asset(format!("png encode failed: {error}")))?;
        png_pages.push(png);
    }

    Ok(png_pages)
}

#[allow(clippy::too_many_arguments)]
fn render_form_to_surface(
    pixel_width: u32,
    pixel_height: u32,
    page_height_pt: f32,
    page_width_pt: f32,
    form: &FormDefinition,
    x: Pt,
    y: Pt,
    width: Pt,
    height: Pt,
    base_transform: Transform,
    parent_state: &RasterState,
    forms: &mut HashMap<String, FormDefinition>,
    image_cache: &mut HashMap<String, Option<Pixmap>>,
    registry: Option<&FontRegistry>,
    shape_text: bool,
) -> Result<Option<Pixmap>, FullBleedError> {
    let Some(mut surface) = Pixmap::new(pixel_width, pixel_height) else {
        return Ok(None);
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
    let form_transform = Transform::from_row(sx, 0.0, 0.0, sy, x.to_f32(), draw_y);
    let mut form_state = parent_state.clone();
    form_state.blend_mode = MixBlendMode::Normal;
    form_state.fill_opacity = 1.0;
    form_state.stroke_opacity = 1.0;
    form_state.transform = form_state.transform.post_concat(form_transform);
    let mut form_stack = Vec::new();
    let mut form_path = PathBuilder::new();
    let mut form_has_path = false;
    render_commands(
        &mut surface,
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
    let _ = page_width_pt;
    Ok(Some(surface))
}

#[allow(clippy::too_many_arguments)]
fn render_mask_coverage(
    pixel_width: u32,
    pixel_height: u32,
    page_height_pt: f32,
    page_width_pt: f32,
    layers: &[CompiledMaskLayer],
    x: Pt,
    y: Pt,
    width: Pt,
    height: Pt,
    base_transform: Transform,
    parent_state: &RasterState,
    mask_shader_phase: (f32, f32),
    forms: &mut HashMap<String, FormDefinition>,
    image_cache: &mut HashMap<String, Option<Pixmap>>,
    registry: Option<&FontRegistry>,
    shape_text: bool,
) -> Result<Option<Vec<f32>>, FullBleedError> {
    let mut coverage: Option<Vec<f32>> = None;
    let mut mask_state = parent_state.clone();
    mask_state.mask_shader_phase = mask_shader_phase;
    for index in (0..layers.len()).rev() {
        let layer = &layers[index];
        let Some(mask_form) = forms.get(&layer.resource_id).cloned() else {
            continue;
        };
        let Some(mask_surface) = render_form_to_surface(
            pixel_width,
            pixel_height,
            page_height_pt,
            page_width_pt,
            &mask_form,
            x,
            y,
            width,
            height,
            base_transform,
            &mask_state,
            forms,
            image_cache,
            registry,
            shape_text,
        )?
        else {
            continue;
        };
        let layer_coverage = mask_surface
            .data()
            .chunks_exact(4)
            .map(|pixel| match layer.mode {
                MaskMode::MatchSource | MaskMode::Alpha => pixel[3] as f32 / 255.0,
                // The pixmap is premultiplied, so luminance of its RGB
                // channels already includes source alpha.
                MaskMode::Luminance => {
                    (0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32)
                        / 255.0
                }
            })
            .collect::<Vec<_>>();
        if let Some(destination) = coverage.as_mut() {
            for (dst, source_alpha) in destination.iter_mut().zip(layer_coverage.into_iter()) {
                let destination_alpha = *dst;
                *dst = match layer.composite {
                    MaskComposite::Add => source_alpha + destination_alpha * (1.0 - source_alpha),
                    MaskComposite::Subtract => source_alpha * (1.0 - destination_alpha),
                    MaskComposite::DestinationOut => destination_alpha * (1.0 - source_alpha),
                    MaskComposite::Intersect => source_alpha * destination_alpha,
                    MaskComposite::Exclude => {
                        source_alpha * (1.0 - destination_alpha)
                            + destination_alpha * (1.0 - source_alpha)
                    }
                }
                .clamp(0.0, 1.0);
            }
        } else {
            coverage = Some(layer_coverage);
        }
    }
    Ok(coverage)
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
                    .pre_concat(Transform::from_translate(x.to_f32(), -y.to_f32()));
            }
            Command::CssTransformOrigin { x, y, inverse } => {
                let sign = if *inverse { -1.0 } else { 1.0 };
                let pdf_y = page_height_pt - y.to_f32();
                state.transform = state
                    .transform
                    .pre_concat(Transform::from_translate(x.to_f32() * sign, pdf_y * sign));
            }
            Command::Scale(x, y) => {
                state.transform = state.transform.pre_concat(Transform::from_scale(*x, *y));
            }
            Command::Rotate(angle) => {
                let deg = -*angle * 180.0 / core::f32::consts::PI;
                state.transform = state.transform.pre_concat(Transform::from_rotate(deg));
            }
            Command::ConcatMatrix { a, b, c, d, e, f } => {
                state.transform = state.transform.pre_concat(Transform::from_row(
                    *a,
                    -*b,
                    -*c,
                    *d,
                    e.to_f32(),
                    -f.to_f32(),
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
            Command::SetBlendMode { mode } => {
                state.blend_mode = *mode;
            }
            Command::ApplyBackdropFilter {
                x,
                y,
                width,
                height,
                radius,
                filter,
            } => {
                apply_backdrop_filter(
                    pixmap,
                    state,
                    page_height_pt,
                    base_transform,
                    *x,
                    *y,
                    *width,
                    *height,
                    *radius,
                    filter,
                );
            }
            Command::SetFontName(name) => state.font_name = name.clone(),
            Command::SetFontSize(size) => state.font_size = *size,
            Command::SetTextRenderingMode(mode) => state.text_rendering_mode = (*mode).min(7),
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
            Command::DrawSyntheticBoldGlyphRun {
                x,
                y,
                glyph_ids,
                advances,
                offsets,
                stroke_width,
            } => {
                draw_synthetic_bold_glyph_run(
                    pixmap,
                    state,
                    x.to_f32(),
                    y.to_f32(),
                    glyph_ids,
                    advances,
                    offsets,
                    *stroke_width,
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
                    let paint = fill_paint(state.fill_color, state.fill_opacity, state.blend_mode);
                    fill_path_blended(
                        pixmap,
                        &path,
                        &paint,
                        FillRule::Winding,
                        base_transform.pre_concat(state.transform),
                        state.clip_mask.as_ref(),
                        state.blend_mode,
                    );
                }
            }
            Command::DrawImage {
                x,
                y,
                width,
                height,
                resource_id,
                interpolate,
                source_clip,
            } => {
                let (source_width, source_height) = {
                    let source = image_cache
                        .entry(resource_id.clone())
                        .or_insert_with(|| load_image_pixmap(resource_id));
                    source
                        .as_ref()
                        .map(|image| (image.width(), image.height()))
                        .unwrap_or((0, 0))
                };
                let source_crop = source_clip
                    .and_then(|clip| clip.resolve(*width, *height, source_width, source_height));
                let (image_key, draw_x, draw_y, draw_width, draw_height) =
                    if let Some(crop) = source_crop {
                        let image_key = format!(
                            "{resource_id}\0fullbleed-source-crop:{}:{}:{}:{}",
                            crop.x, crop.y, crop.width, crop.height,
                        );
                        if !image_cache.contains_key(&image_key) {
                            let cropped = image_cache
                                .get(resource_id)
                                .and_then(|source| source.as_ref())
                                .and_then(|source| {
                                    source.crop(crop.x, crop.y, crop.width, crop.height)
                                });
                            image_cache.insert(image_key.clone(), cropped);
                        }
                        let (draw_x, draw_y, draw_width, draw_height) = source_clip
                            .expect("resolved crop has a source clip")
                            .snap_target_rect(crop.target_rect(*x, *y, *width, *height));
                        (image_key, draw_x, draw_y, draw_width, draw_height)
                    } else {
                        (resource_id.clone(), *x, *y, *width, *height)
                    };
                let source = image_cache
                    .get(&image_key)
                    .and_then(|source| source.as_ref());
                if let Some(image) = source {
                    let src_w = image.width() as f32;
                    let src_h = image.height() as f32;
                    if src_w > 0.0 && src_h > 0.0 {
                        let sx = draw_width.to_f32() / src_w;
                        let sy = draw_height.to_f32() / src_h;
                        // DrawImage coordinates are top-left based. Convert to user-space with a
                        // local y-flip so source row 0 lands at the visual top, matching PDF /Im Do.
                        let image_ts = Transform::from_row(
                            sx,
                            0.0,
                            0.0,
                            -sy,
                            draw_x.to_f32(),
                            page_height_pt - draw_y.to_f32(),
                        );
                        // Image placement is in local object space; then apply current state CTM.
                        let ctm = state.transform.pre_concat(image_ts);
                        let device_ts = base_transform.pre_concat(ctm);
                        let mut paint = PixmapPaint::default();
                        paint.quality = if *interpolate && !state.discrete_image_sampling {
                            FilterQuality::Bilinear
                        } else {
                            FilterQuality::Nearest
                        };
                        paint.opacity = state.fill_opacity.clamp(0.0, 1.0);
                        paint.blend_mode = sk_blend_mode(state.blend_mode);
                        draw_pixmap_blended(
                            pixmap,
                            0,
                            0,
                            image.as_ref(),
                            &paint,
                            device_ts,
                            state.clip_mask.as_ref(),
                            state.blend_mode,
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
                        isolated: false,
                        commands: commands.clone(),
                    },
                );
            }
            Command::DefineIsolatedForm {
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
                        isolated: true,
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
                if form.isolated {
                    let Some(mut offscreen) = Pixmap::new(pixmap.width(), pixmap.height()) else {
                        continue;
                    };
                    form_state.blend_mode = MixBlendMode::Normal;
                    form_state.fill_opacity = 1.0;
                    form_state.stroke_opacity = 1.0;
                    form_state.transform = form_state.transform.post_concat(form_ts);
                    let mut form_stack: Vec<RasterState> = Vec::new();
                    let mut form_path = PathBuilder::new();
                    let mut form_has_path = false;
                    render_commands(
                        &mut offscreen,
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

                    let mut paint = PixmapPaint::default();
                    paint.quality = FilterQuality::Bilinear;
                    paint.opacity = state.fill_opacity.clamp(0.0, 1.0);
                    paint.blend_mode = sk_blend_mode(state.blend_mode);
                    draw_pixmap_blended(
                        pixmap,
                        0,
                        0,
                        offscreen.as_ref(),
                        &paint,
                        Transform::identity(),
                        state.clip_mask.as_ref(),
                        state.blend_mode,
                    );
                    continue;
                }
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
            Command::DrawFilteredForm {
                x,
                y,
                width,
                height,
                resource_id,
                filter,
                css_shadow,
            } => {
                let Some(form) = forms.get(resource_id).cloned() else {
                    continue;
                };
                let Some(mut offscreen) = Pixmap::new(pixmap.width(), pixmap.height()) else {
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
                form_state.blend_mode = MixBlendMode::Normal;
                form_state.transform = form_state.transform.post_concat(form_ts);
                let filter_transform = base_transform.pre_concat(form_state.transform);
                let mut form_stack: Vec<RasterState> = Vec::new();
                let mut form_path = PathBuilder::new();
                let mut form_has_path = false;
                render_commands(
                    &mut offscreen,
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
                let output_bounds = apply_foreground_filter_group(
                    pixmap,
                    &offscreen,
                    state,
                    filter,
                    filter_transform,
                    *css_shadow,
                );
                if output_bounds.is_some() {
                    state.filtered_output_bounds = output_bounds;
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
                let Some(source_form) = forms.get(resource_id).cloned() else {
                    continue;
                };
                let Some(mut source) = render_form_to_surface(
                    pixmap.width(),
                    pixmap.height(),
                    page_height_pt,
                    page_width_pt,
                    &source_form,
                    *x,
                    *y,
                    *width,
                    *height,
                    base_transform,
                    state,
                    forms,
                    image_cache,
                    registry,
                    shape_text,
                )?
                else {
                    continue;
                };

                let Some(coverage) = render_mask_coverage(
                    pixmap.width(),
                    pixmap.height(),
                    page_height_pt,
                    page_width_pt,
                    layers,
                    *x,
                    *y,
                    *width,
                    *height,
                    base_transform,
                    state,
                    (MASK_SHADER_PHASE_PT, MASK_SHADER_PHASE_PT),
                    forms,
                    image_cache,
                    registry,
                    shape_text,
                )?
                else {
                    continue;
                };
                for (pixel, alpha) in source
                    .data_mut()
                    .chunks_exact_mut(4)
                    .zip(coverage.into_iter())
                {
                    for channel in pixel {
                        *channel = ((*channel as f32 * alpha).round().clamp(0.0, 255.0)) as u8;
                    }
                }
                let mut paint = PixmapPaint::default();
                paint.quality = FilterQuality::Bilinear;
                paint.opacity = state.fill_opacity.clamp(0.0, 1.0);
                paint.blend_mode = sk_blend_mode(state.blend_mode);
                draw_pixmap_blended(
                    pixmap,
                    0,
                    0,
                    source.as_ref(),
                    &paint,
                    Transform::identity(),
                    state.clip_mask.as_ref(),
                    state.blend_mode,
                );
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
    let paint = fill_paint(state.fill_color, state.fill_opacity, state.blend_mode);
    fill_path_blended(
        pixmap,
        &path,
        &paint,
        fill_rule,
        base_transform.pre_concat(state.transform),
        state.clip_mask.as_ref(),
        state.blend_mode,
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
    let paint = fill_paint(state.stroke_color, state.stroke_opacity, state.blend_mode);
    let stroke = build_stroke(state);
    stroke_path_blended(
        pixmap,
        &path,
        &paint,
        &stroke,
        base_transform.pre_concat(state.transform),
        state.clip_mask.as_ref(),
        state.blend_mode,
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
    let fill = fill_paint(state.fill_color, state.fill_opacity, state.blend_mode);
    fill_path_blended(
        pixmap,
        &path,
        &fill,
        fill_rule,
        base_transform.pre_concat(state.transform),
        state.clip_mask.as_ref(),
        state.blend_mode,
    );
    let stroke_paint = fill_paint(state.stroke_color, state.stroke_opacity, state.blend_mode);
    let stroke = build_stroke(state);
    stroke_path_blended(
        pixmap,
        &path,
        &stroke_paint,
        &stroke,
        base_transform.pre_concat(state.transform),
        state.clip_mask.as_ref(),
        state.blend_mode,
    );
}

fn fill_path_blended(
    pixmap: &mut Pixmap,
    path: &Path,
    paint: &Paint<'static>,
    fill_rule: FillRule,
    transform: Transform,
    clip_mask: Option<&Mask>,
    blend_mode: MixBlendMode,
) {
    if blend_mode != MixBlendMode::PlusDarker {
        pixmap.fill_path(path, paint, fill_rule, transform, clip_mask);
        return;
    }
    let Some(mut source) = Pixmap::new(pixmap.width(), pixmap.height()) else {
        return;
    };
    let mut source_paint = paint.clone();
    source_paint.blend_mode = SkBlendMode::SourceOver;
    source.fill_path(path, &source_paint, fill_rule, transform, clip_mask);
    composite_plus_darker(pixmap, &source);
}

fn stroke_path_blended(
    pixmap: &mut Pixmap,
    path: &Path,
    paint: &Paint<'static>,
    stroke: &Stroke,
    transform: Transform,
    clip_mask: Option<&Mask>,
    blend_mode: MixBlendMode,
) {
    if blend_mode != MixBlendMode::PlusDarker {
        pixmap.stroke_path(path, paint, stroke, transform, clip_mask);
        return;
    }
    let Some(mut source) = Pixmap::new(pixmap.width(), pixmap.height()) else {
        return;
    };
    let mut source_paint = paint.clone();
    source_paint.blend_mode = SkBlendMode::SourceOver;
    source.stroke_path(path, &source_paint, stroke, transform, clip_mask);
    composite_plus_darker(pixmap, &source);
}

fn draw_pixmap_blended(
    pixmap: &mut Pixmap,
    x: i32,
    y: i32,
    source: PixmapRef<'_>,
    paint: &PixmapPaint,
    transform: Transform,
    clip_mask: Option<&Mask>,
    blend_mode: MixBlendMode,
) {
    if blend_mode != MixBlendMode::PlusDarker {
        pixmap.draw_pixmap(x, y, source, paint, transform, clip_mask);
        return;
    }
    let Some(mut layer) = Pixmap::new(pixmap.width(), pixmap.height()) else {
        return;
    };
    let mut source_paint = *paint;
    source_paint.blend_mode = SkBlendMode::SourceOver;
    layer.draw_pixmap(x, y, source, &source_paint, transform, clip_mask);
    composite_plus_darker(pixmap, &layer);
}

fn composite_plus_darker(dst: &mut Pixmap, src: &Pixmap) {
    if dst.width() != src.width() || dst.height() != src.height() {
        return;
    }
    let src_data = src.data();
    let dst_data = dst.data_mut();
    for (src_px, dst_px) in src_data.chunks_exact(4).zip(dst_data.chunks_exact_mut(4)) {
        let sa = (src_px[3] as f32) / 255.0;
        if sa <= 0.0 {
            continue;
        }
        let da = (dst_px[3] as f32) / 255.0;
        let out_a = (sa + da * (1.0 - sa)).clamp(0.0, 1.0);
        for channel in 0..3 {
            let sc = (src_px[channel] as f32) / 255.0;
            let dc = (dst_px[channel] as f32) / 255.0;
            let out = (out_a - (da - dc) - (sa - sc)).clamp(0.0, out_a);
            dst_px[channel] = unit_to_u8(out);
        }
        dst_px[3] = unit_to_u8(out_a);
    }
}

fn unit_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
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

#[allow(clippy::too_many_arguments)]
fn apply_backdrop_filter(
    pixmap: &mut Pixmap,
    state: &RasterState,
    page_height_pt: f32,
    base_transform: Transform,
    x: Pt,
    y: Pt,
    width: Pt,
    height: Pt,
    radius: Pt,
    filter: &PaintFilterSpec,
) {
    if filter.is_identity() {
        return;
    }
    let width_f = width.to_f32().max(0.0);
    let height_f = height.to_f32().max(0.0);
    if width_f <= 0.0 || height_f <= 0.0 {
        return;
    }

    let draw_y = page_height_pt - y.to_f32() - height_f;
    let Some(path) = rounded_rect_path(x.to_f32(), draw_y, width_f, height_f, radius.to_f32())
    else {
        return;
    };

    let device_transform = base_transform.pre_concat(state.transform);
    let Some(mut mask) = Mask::new(pixmap.width(), pixmap.height()) else {
        return;
    };
    mask.fill_path(&path, FillRule::Winding, true, device_transform);

    if let Some(clip) = state.clip_mask.as_ref() {
        let mask_data = mask.data_mut();
        let clip_data = clip.data();
        for (dst, src) in mask_data.iter_mut().zip(clip_data.iter()) {
            *dst = mul_alpha_u8(*dst, *src);
        }
    }

    let (min_x, min_y, max_x, max_y) = mask_bounds(mask.data(), pixmap.width(), pixmap.height());
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let roi_w = max_x - min_x;
    let roi_h = max_y - min_y;
    if roi_w == 0 || roi_h == 0 {
        return;
    }

    let row_stride = (pixmap.width() as usize) * 4;
    let mut src_rgba = vec![0u8; (roi_w as usize) * (roi_h as usize) * 4];
    {
        let data = pixmap.data();
        for row in 0..roi_h as usize {
            let src_off = ((min_y as usize + row) * row_stride) + (min_x as usize * 4);
            let dst_off = row * (roi_w as usize) * 4;
            for col in 0..roi_w as usize {
                let s = src_off + col * 4;
                let d = dst_off + col * 4;
                let (r, g, b, a) = unpremul_rgba(data[s], data[s + 1], data[s + 2], data[s + 3]);
                src_rgba[d] = r;
                src_rgba[d + 1] = g;
                src_rgba[d + 2] = b;
                src_rgba[d + 3] = a;
            }
        }
    }

    let base_img = match crate::image_native::RgbaImage::from_raw(roi_w, roi_h, src_rgba.clone()) {
        Some(img) => img,
        None => return,
    };

    let blur_px = backdrop_blur_sigma_px(filter.blur_radius, device_transform);
    let filtered_img = if blur_px > 0.05 {
        crate::image_native::blur_rgba(&base_img, blur_px)
    } else {
        base_img.clone()
    };
    let filtered = filtered_img.as_raw();
    let saturate = filter.saturate.max(0.0);
    let brightness = filter.brightness.max(0.0);
    let contrast = filter.contrast.max(0.0);
    let invert = filter.invert.clamp(0.0, 1.0);
    let sepia = filter.sepia.clamp(0.0, 1.0);
    let hue_rotate = filter.hue_rotate;
    let filter_opacity = filter.opacity.clamp(0.0, 1.0);

    for drop_shadow in &filter.drop_shadows {
        draw_backdrop_filter_drop_shadow(
            pixmap,
            &mask,
            state,
            *drop_shadow,
            filter_opacity,
            device_transform,
        );
    }

    let mask_data = mask.data();
    let pixmap_width = pixmap.width() as usize;
    let dst = pixmap.data_mut();
    for row in 0..roi_h as usize {
        let global_y = min_y as usize + row;
        for col in 0..roi_w as usize {
            let global_x = min_x as usize + col;
            let mask_idx = global_y * pixmap_width + global_x;
            let mask_alpha = mask_data[mask_idx];
            if mask_alpha == 0 {
                continue;
            }
            let mix = ((mask_alpha as f32) / 255.0) * filter_opacity;
            let src_idx = (row * roi_w as usize + col) * 4;
            let dst_idx = global_y * row_stride + global_x * 4;

            let orig_r = src_rgba[src_idx] as f32;
            let orig_g = src_rgba[src_idx + 1] as f32;
            let orig_b = src_rgba[src_idx + 2] as f32;
            let orig_a = src_rgba[src_idx + 3];

            let mut filt_r = filtered[src_idx] as f32;
            let mut filt_g = filtered[src_idx + 1] as f32;
            let mut filt_b = filtered[src_idx + 2] as f32;
            apply_saturate_rgb(&mut filt_r, &mut filt_g, &mut filt_b, saturate);
            apply_contrast_rgb(&mut filt_r, &mut filt_g, &mut filt_b, contrast);
            apply_hue_rotate_rgb(&mut filt_r, &mut filt_g, &mut filt_b, hue_rotate);
            apply_invert_rgb(&mut filt_r, &mut filt_g, &mut filt_b, invert);
            apply_sepia_rgb(&mut filt_r, &mut filt_g, &mut filt_b, sepia);
            apply_brightness_rgb(&mut filt_r, &mut filt_g, &mut filt_b, brightness);

            let out_r = (orig_r * (1.0 - mix) + filt_r * mix).clamp(0.0, 255.0);
            let out_g = (orig_g * (1.0 - mix) + filt_g * mix).clamp(0.0, 255.0);
            let out_b = (orig_b * (1.0 - mix) + filt_b * mix).clamp(0.0, 255.0);

            dst[dst_idx + 3] = orig_a;
            dst[dst_idx] = premul_u8(out_r.round() as u8, orig_a);
            dst[dst_idx + 1] = premul_u8(out_g.round() as u8, orig_a);
            dst[dst_idx + 2] = premul_u8(out_b.round() as u8, orig_a);
        }
    }
}

fn draw_backdrop_filter_drop_shadow(
    pixmap: &mut Pixmap,
    source_mask: &Mask,
    state: &RasterState,
    shadow: FilterDropShadowSpec,
    filter_opacity: f32,
    device_transform: Transform,
) {
    let width = pixmap.width();
    let height = pixmap.height();
    if width == 0 || height == 0 || shadow.opacity <= 0.0 {
        return;
    }

    let (sx, sy) = device_transform.get_scale();
    let dx = (shadow.offset_x.to_f32() * sx.abs()).round() as i32;
    let dy = (shadow.offset_y.to_f32() * sy.abs()).round() as i32;
    let alpha_scale = (shadow.opacity * filter_opacity.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    if alpha_scale <= 0.0 {
        return;
    }

    let mut shadow_data = vec![0_u8; (width as usize) * (height as usize) * 4];
    {
        let src_mask = source_mask.data();
        let src = pixmap.data();
        let width_i = width as i32;
        let height_i = height as i32;
        let color_r = (shadow.color.r.clamp(0.0, 1.0) * 255.0).round() as u8;
        let color_g = (shadow.color.g.clamp(0.0, 1.0) * 255.0).round() as u8;
        let color_b = (shadow.color.b.clamp(0.0, 1.0) * 255.0).round() as u8;

        for y in 0..height_i {
            let target_y = y + dy;
            if target_y < 0 || target_y >= height_i {
                continue;
            }
            for x in 0..width_i {
                let target_x = x + dx;
                if target_x < 0 || target_x >= width_i {
                    continue;
                }
                let src_pixel_idx = (y as usize) * (width as usize) + x as usize;
                let mask_alpha = src_mask[src_pixel_idx];
                if mask_alpha == 0 {
                    continue;
                }
                let src_idx = src_pixel_idx * 4;
                let src_alpha = src[src_idx + 3];
                if src_alpha == 0 {
                    continue;
                }
                let dst_idx = ((target_y as usize) * (width as usize) + target_x as usize) * 4;
                let masked_alpha = ((mask_alpha as u16) * (src_alpha as u16) / 255) as f32;
                let alpha = (masked_alpha * alpha_scale).round().clamp(0.0, 255.0) as u8;
                if alpha <= shadow_data[dst_idx + 3] {
                    continue;
                }
                shadow_data[dst_idx] = color_r;
                shadow_data[dst_idx + 1] = color_g;
                shadow_data[dst_idx + 2] = color_b;
                shadow_data[dst_idx + 3] = alpha;
            }
        }
    }

    let blur_px = backdrop_blur_sigma_px(shadow.blur_radius, device_transform);
    if blur_px > 0.05 {
        let Some(base_img) = crate::image_native::RgbaImage::from_raw(width, height, shadow_data)
        else {
            return;
        };
        shadow_data = crate::image_native::blur_rgba(&base_img, blur_px).into_raw();
    }
    premultiply_rgba(&mut shadow_data);

    let Some(size) = IntSize::from_wh(width, height) else {
        return;
    };
    let Some(shadow_pixmap) = Pixmap::from_vec(shadow_data, size) else {
        return;
    };

    let mut paint = PixmapPaint::default();
    paint.quality = FilterQuality::Bilinear;
    paint.opacity = 1.0;
    paint.blend_mode = sk_blend_mode(state.blend_mode);
    draw_pixmap_blended(
        pixmap,
        0,
        0,
        shadow_pixmap.as_ref(),
        &paint,
        Transform::identity(),
        state.clip_mask.as_ref(),
        state.blend_mode,
    );
}

fn apply_foreground_filter_group(
    pixmap: &mut Pixmap,
    offscreen: &Pixmap,
    state: &RasterState,
    filter: &PaintFilterSpec,
    device_transform: Transform,
    css_shadow: bool,
) -> Option<PixelBounds> {
    let width = offscreen.width();
    let height = offscreen.height();
    if width == 0 || height == 0 {
        return None;
    }

    let (filtered_data, output_bounds) = if filter.operations.is_empty() {
        for drop_shadow in &filter.drop_shadows {
            draw_filter_drop_shadow(
                pixmap,
                offscreen,
                state,
                *drop_shadow,
                filter.opacity,
                device_transform,
            );
        }

        let mut filtered_data = offscreen.data().to_vec();
        let blur_px = backdrop_blur_sigma_px(filter.blur_radius, device_transform);
        if blur_px > 0.05 {
            let Some(base_img) =
                crate::image_native::RgbaImage::from_raw(width, height, filtered_data)
            else {
                return None;
            };
            filtered_data = if css_shadow {
                crate::image_native::blur_rgba_css_shadow(&base_img, blur_px).into_raw()
            } else {
                // Blink routes CSS filter blur through Skia's PlanGauss path,
                // the same linear-time three-box kernel used by SVG filters.
                // Keeping foreground filters on that compiled kernel both
                // matches the browser alpha profile and removes radius-scaled
                // convolution work from variable-data rendering.
                crate::image_native::blur_rgba_svg_filter(&base_img, blur_px).into_raw()
            };
        }
        apply_filter_to_premul_rgba(&mut filtered_data, filter);
        (filtered_data, None)
    } else {
        let Some(filtered) = apply_ordered_filter_operations(
            offscreen.data(),
            width,
            height,
            &filter.operations,
            device_transform,
            css_shadow,
        ) else {
            return None;
        };
        filtered
    };

    let Some(size) = IntSize::from_wh(width, height) else {
        return None;
    };
    let Some(filtered_pixmap) = Pixmap::from_vec(filtered_data, size) else {
        return None;
    };

    let mut paint = PixmapPaint::default();
    paint.quality = FilterQuality::Bilinear;
    paint.opacity = 1.0;
    paint.blend_mode = sk_blend_mode(state.blend_mode);
    draw_pixmap_blended(
        pixmap,
        0,
        0,
        filtered_pixmap.as_ref(),
        &paint,
        Transform::identity(),
        state.clip_mask.as_ref(),
        state.blend_mode,
    );
    output_bounds
}

fn apply_ordered_filter_operations(
    source: &[u8],
    width: u32,
    height: u32,
    operations: &[PaintFilterOperation],
    device_transform: Transform,
    css_shadow: bool,
) -> Option<(Vec<u8>, Option<PixelBounds>)> {
    let mut data = source.to_vec();
    let mut output_bounds = None;
    let mut effect_bounds = alpha_pixel_bounds(source, width, height);
    let mut operation_index = 0usize;
    while operation_index < operations.len() {
        if is_color_filter_operation(&operations[operation_index]) {
            let run_start = operation_index;
            while operation_index < operations.len()
                && is_color_filter_operation(&operations[operation_index])
            {
                operation_index += 1;
            }
            // Skia executes adjacent colour filters in one floating-point
            // raster pipeline. Preserve authored order but quantize only once
            // at the spatial-operation boundary.
            apply_color_filter_operation_chain(&mut data, &operations[run_start..operation_index]);
            continue;
        }

        let operation = &operations[operation_index];
        match operation {
            PaintFilterOperation::Saturate(_)
            | PaintFilterOperation::Brightness(_)
            | PaintFilterOperation::Contrast(_)
            | PaintFilterOperation::Invert(_)
            | PaintFilterOperation::Sepia(_)
            | PaintFilterOperation::HueRotate(_)
            | PaintFilterOperation::Opacity(_) => unreachable!("colour run handled above"),
            PaintFilterOperation::Blur(radius) => {
                let blur_px = backdrop_blur_sigma_px(*radius, device_transform);
                if blur_px > 0.05 {
                    let image = crate::image_native::RgbaImage::from_raw(width, height, data)?;
                    data = if css_shadow {
                        crate::image_native::blur_rgba_css_shadow(&image, blur_px).into_raw()
                    } else {
                        crate::image_native::blur_rgba_svg_filter(&image, blur_px).into_raw()
                    };
                    if let Some((left, top, right, bottom)) = effect_bounds {
                        // CSS filter regions retain three standard deviations of
                        // transparent visual overflow. Preserve that virtual
                        // surface instead of alpha-cropping it away so PDF image
                        // placement and interpolation stay phase-identical to the
                        // browser while the retained pixel payload remains bounded.
                        let outset = (blur_px * 3.0).ceil().max(0.0) as i64;
                        let expanded = (
                            (i64::from(left) - outset).clamp(0, i64::from(width)) as u32,
                            (i64::from(top) - outset).clamp(0, i64::from(height)) as u32,
                            (i64::from(right) + outset).clamp(0, i64::from(width)) as u32,
                            (i64::from(bottom) + outset).clamp(0, i64::from(height)) as u32,
                        );
                        effect_bounds = Some(expanded);
                        output_bounds = Some(expanded);
                    }
                }
            }
            PaintFilterOperation::DropShadow(shadow) => {
                data = filter_drop_shadow_buffer(
                    &data,
                    width,
                    height,
                    *shadow,
                    device_transform,
                    false,
                    false,
                )?;
                effect_bounds = alpha_pixel_bounds(&data, width, height);
            }
            PaintFilterOperation::Svg(program) => {
                let filtered =
                    apply_svg_filter_program(&data, width, height, program, device_transform)?;
                data = filtered.0;
                effect_bounds = Some(filtered.1);
                if svg_filter_retains_transparent_region(program) {
                    output_bounds = Some(filtered.1);
                }
            }
            // HTML compilation resolves local URLs into Svg bytecode. Keep an
            // unresolved operation transparent for non-HTML callers.
            PaintFilterOperation::Url(_) => {}
        }
        operation_index += 1;
    }
    Some((data, output_bounds))
}

fn is_color_filter_operation(operation: &PaintFilterOperation) -> bool {
    matches!(
        operation,
        PaintFilterOperation::Saturate(_)
            | PaintFilterOperation::Brightness(_)
            | PaintFilterOperation::Contrast(_)
            | PaintFilterOperation::Invert(_)
            | PaintFilterOperation::Sepia(_)
            | PaintFilterOperation::HueRotate(_)
            | PaintFilterOperation::Opacity(_)
    )
}

fn svg_filter_retains_transparent_region(program: &SvgFilterProgram) -> bool {
    program.nodes.iter().any(|node| match &node.primitive {
        SvgFilterPrimitive::GaussianBlur {
            std_deviation_x,
            std_deviation_y,
            ..
        } => *std_deviation_x > Pt::ZERO || *std_deviation_y > Pt::ZERO,
        SvgFilterPrimitive::DropShadow { shadow, .. } => shadow.blur_radius > Pt::ZERO,
        _ => false,
    })
}

fn apply_color_filter_operation_chain(data: &mut [u8], operations: &[PaintFilterOperation]) {
    for px in data.chunks_exact_mut(4) {
        let alpha = px[3];
        if alpha == 0 {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
            continue;
        }
        let (r, g, b, _) = unpremul_rgba(px[0], px[1], px[2], alpha);
        let mut r = r as f32;
        let mut g = g as f32;
        let mut b = b as f32;
        let mut out_alpha = alpha as f32;
        for operation in operations {
            match operation {
                PaintFilterOperation::Saturate(value) => {
                    apply_saturate_rgb(&mut r, &mut g, &mut b, value.max(0.0));
                }
                PaintFilterOperation::Brightness(value) => {
                    apply_brightness_rgb(&mut r, &mut g, &mut b, value.max(0.0));
                }
                PaintFilterOperation::Contrast(value) => {
                    apply_contrast_rgb(&mut r, &mut g, &mut b, value.max(0.0));
                }
                PaintFilterOperation::Invert(value) => {
                    apply_invert_rgb(&mut r, &mut g, &mut b, value.clamp(0.0, 1.0));
                }
                PaintFilterOperation::Sepia(value) => {
                    apply_sepia_rgb(&mut r, &mut g, &mut b, value.clamp(0.0, 1.0));
                }
                PaintFilterOperation::HueRotate(value) => {
                    apply_hue_rotate_rgb(&mut r, &mut g, &mut b, *value);
                }
                PaintFilterOperation::Opacity(value) => {
                    out_alpha *= value.clamp(0.0, 1.0);
                }
                _ => unreachable!("spatial operation in colour filter run"),
            }
        }
        let out_alpha = out_alpha.round().clamp(0.0, 255.0) as u8;
        px[0] = premul_u8(r.round().clamp(0.0, 255.0) as u8, out_alpha);
        px[1] = premul_u8(g.round().clamp(0.0, 255.0) as u8, out_alpha);
        px[2] = premul_u8(b.round().clamp(0.0, 255.0) as u8, out_alpha);
        px[3] = out_alpha;
    }
}

fn filter_drop_shadow_buffer(
    source: &[u8],
    width: u32,
    height: u32,
    shadow: FilterDropShadowSpec,
    device_transform: Transform,
    linear_rgb: bool,
    svg_filter: bool,
) -> Option<Vec<u8>> {
    let (sx, sy) = device_transform.get_scale();
    let dx = shadow.offset_x.to_f32() * sx.abs();
    let dy = shadow.offset_y.to_f32() * sy.abs();
    let opacity = shadow.opacity.clamp(0.0, 1.0);
    let mut shadow_data = vec![0u8; source.len()];
    let color = filter_working_color(shadow.color, linear_rgb);
    let sample_alpha = |x: i32, y: i32| -> f32 {
        if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
            return 0.0;
        }
        source[((y as usize * width as usize + x as usize) * 4) + 3] as f32
    };
    for target_y in 0..height {
        let source_y = target_y as f32 - dy;
        let source_y0 = source_y.floor() as i32;
        let fy = source_y - source_y0 as f32;
        for target_x in 0..width {
            let source_x = target_x as f32 - dx;
            let source_x0 = source_x.floor() as i32;
            let fx = source_x - source_x0 as f32;
            let top = sample_alpha(source_x0, source_y0) * (1.0 - fx)
                + sample_alpha(source_x0 + 1, source_y0) * fx;
            let bottom = sample_alpha(source_x0, source_y0 + 1) * (1.0 - fx)
                + sample_alpha(source_x0 + 1, source_y0 + 1) * fx;
            let alpha = ((top * (1.0 - fy) + bottom * fy) * opacity)
                .round()
                .clamp(0.0, 255.0) as u8;
            if alpha == 0 {
                continue;
            }
            let index = (target_y as usize * width as usize + target_x as usize) * 4;
            shadow_data[index] = color[0];
            shadow_data[index + 1] = color[1];
            shadow_data[index + 2] = color[2];
            shadow_data[index + 3] = alpha;
        }
    }
    let blur_px = backdrop_blur_sigma_px(shadow.blur_radius, device_transform);
    if blur_px > 0.05 {
        let image = crate::image_native::RgbaImage::from_raw(width, height, shadow_data)?;
        shadow_data = if svg_filter {
            crate::image_native::blur_rgba_svg_filter(&image, blur_px).into_raw()
        } else {
            crate::image_native::blur_rgba(&image, blur_px).into_raw()
        };
    }
    premultiply_rgba(&mut shadow_data);
    composite_source_over(&mut shadow_data, source);
    Some(shadow_data)
}

fn composite_source_over(destination: &mut [u8], source: &[u8]) {
    for (dst, src) in destination.chunks_exact_mut(4).zip(source.chunks_exact(4)) {
        let inverse_alpha = 255u16.saturating_sub(src[3] as u16);
        for channel in 0..3 {
            dst[channel] = (src[channel] as u16
                + ((dst[channel] as u16 * inverse_alpha + 127) / 255))
                .min(255) as u8;
        }
        dst[3] = (src[3] as u16 + ((dst[3] as u16 * inverse_alpha + 127) / 255)).min(255) as u8;
    }
}

fn apply_svg_filter_program(
    source: &[u8],
    width: u32,
    height: u32,
    program: &SvgFilterProgram,
    device_transform: Transform,
) -> Option<(Vec<u8>, PixelBounds)> {
    let mut source_graphic = source.to_vec();
    if program.linear_rgb {
        convert_filter_buffer_to_linear(&mut source_graphic);
    }
    let mut source_alpha = vec![0u8; source.len()];
    for (dst, src) in source_alpha.chunks_exact_mut(4).zip(source.chunks_exact(4)) {
        dst[3] = src[3];
    }
    let region = svg_filter_region_bounds(source, width, height, program.region);
    let mut previous = source_graphic.clone();
    let mut named: HashMap<String, Vec<u8>> = HashMap::new();

    let resolve =
        |input: &SvgFilterInput, previous: &[u8], named: &HashMap<String, Vec<u8>>| -> Vec<u8> {
            match input {
                SvgFilterInput::SourceGraphic => source_graphic.clone(),
                SvgFilterInput::SourceAlpha => source_alpha.clone(),
                SvgFilterInput::Previous => previous.to_vec(),
                SvgFilterInput::Named(name) => named
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| vec![0u8; source.len()]),
            }
        };

    for node in &program.nodes {
        let output = match &node.primitive {
            SvgFilterPrimitive::GaussianBlur {
                input,
                std_deviation_x,
                std_deviation_y,
            } => {
                let input = resolve(input, &previous, &named);
                let sigma = (backdrop_blur_sigma_px(*std_deviation_x, device_transform)
                    + backdrop_blur_sigma_px(*std_deviation_y, device_transform))
                    * 0.5;
                if sigma <= 0.05 {
                    input
                } else {
                    let image = crate::image_native::RgbaImage::from_raw(width, height, input)?;
                    crate::image_native::blur_rgba_svg_filter(&image, sigma).into_raw()
                }
            }
            SvgFilterPrimitive::Offset { input, dx, dy } => {
                let input = resolve(input, &previous, &named);
                offset_filter_buffer(&input, width, height, *dx, *dy, device_transform)
            }
            SvgFilterPrimitive::ColorMatrix { input, matrix } => {
                let mut input = resolve(input, &previous, &named);
                apply_svg_color_matrix(&mut input, matrix);
                input
            }
            SvgFilterPrimitive::ComponentTransfer { input, functions } => {
                let mut input = resolve(input, &previous, &named);
                apply_svg_component_transfer(&mut input, functions);
                input
            }
            SvgFilterPrimitive::Flood { color, opacity } => flood_filter_buffer(
                source.len(),
                width,
                *color,
                *opacity,
                region,
                program.linear_rgb,
            ),
            SvgFilterPrimitive::CompositeIn { input, input2 } => {
                let mut input = resolve(input, &previous, &named);
                let input2 = resolve(input2, &previous, &named);
                composite_in(&mut input, &input2);
                input
            }
            SvgFilterPrimitive::Morphology {
                input,
                operator,
                radius_x,
                radius_y,
            } => {
                let input = resolve(input, &previous, &named);
                let (sx, sy) = device_transform.get_scale();
                let radius_x = (radius_x.to_f32() * sx.abs()).round().max(0.0) as usize;
                let radius_y = (radius_y.to_f32() * sy.abs()).round().max(0.0) as usize;
                morphology_filter_buffer(
                    &input,
                    width,
                    height,
                    radius_x,
                    radius_y,
                    matches!(operator, SvgMorphologyOperator::Dilate),
                )
            }
            SvgFilterPrimitive::DropShadow { input, shadow } => {
                let input = resolve(input, &previous, &named);
                filter_drop_shadow_buffer(
                    &input,
                    width,
                    height,
                    *shadow,
                    device_transform,
                    program.linear_rgb,
                    true,
                )?
            }
            SvgFilterPrimitive::Merge { inputs } => {
                let mut merged = vec![0u8; source.len()];
                for input in inputs {
                    let input = resolve(input, &previous, &named);
                    composite_source_over(&mut merged, &input);
                }
                merged
            }
            SvgFilterPrimitive::Blend {
                input,
                input2,
                mode,
            } => {
                let input = resolve(input, &previous, &named);
                let input2 = resolve(input2, &previous, &named);
                blend_filter_buffers(&input, &input2, *mode)
            }
        };
        if let Some(name) = node.result.as_ref() {
            named.insert(name.clone(), output.clone());
        }
        previous = output;
    }

    clear_outside_filter_region(&mut previous, width, height, region);
    if program.linear_rgb {
        convert_filter_buffer_to_srgb(&mut previous);
    }
    Some((previous, region))
}

fn svg_filter_region_bounds(
    source: &[u8],
    width: u32,
    height: u32,
    region: crate::flowable::SvgFilterRegion,
) -> (u32, u32, u32, u32) {
    let Some((min_x, min_y, max_x, max_y)) = alpha_pixel_bounds(source, width, height) else {
        return (0, 0, 0, 0);
    };
    let source_width = (max_x - min_x) as f32;
    let source_height = (max_y - min_y) as f32;
    let left = (min_x as f32 + region.x * source_width).floor() as i64;
    let top = (min_y as f32 + region.y * source_height).floor() as i64;
    let right = (min_x as f32 + (region.x + region.width) * source_width).ceil() as i64;
    let bottom = (min_y as f32 + (region.y + region.height) * source_height).ceil() as i64;
    (
        left.clamp(0, width as i64) as u32,
        top.clamp(0, height as i64) as u32,
        right.clamp(0, width as i64) as u32,
        bottom.clamp(0, height as i64) as u32,
    )
}

fn alpha_pixel_bounds(source: &[u8], width: u32, height: u32) -> Option<PixelBounds> {
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0;
    let mut max_y = 0;
    for (index, pixel) in source.chunks_exact(4).enumerate() {
        if pixel[3] == 0 {
            continue;
        }
        let x = index as u32 % width;
        let y = index as u32 / width;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + 1);
        max_y = max_y.max(y + 1);
    }
    (min_x < max_x && min_y < max_y).then_some((min_x, min_y, max_x, max_y))
}

fn clear_outside_filter_region(
    data: &mut [u8],
    width: u32,
    height: u32,
    region: (u32, u32, u32, u32),
) {
    let (left, top, right, bottom) = region;
    for y in 0..height {
        for x in 0..width {
            if x >= left && x < right && y >= top && y < bottom {
                continue;
            }
            let index = (y as usize * width as usize + x as usize) * 4;
            data[index..index + 4].fill(0);
        }
    }
}

fn flood_filter_buffer(
    length: usize,
    width: u32,
    color: Color,
    opacity: f32,
    region: (u32, u32, u32, u32),
    linear_rgb: bool,
) -> Vec<u8> {
    let mut data = vec![0u8; length];
    let alpha = unit_to_u8(opacity);
    let working = filter_working_color(color, linear_rgb);
    let color = [
        premul_u8(working[0], alpha),
        premul_u8(working[1], alpha),
        premul_u8(working[2], alpha),
        alpha,
    ];
    for y in region.1..region.3 {
        for x in region.0..region.2 {
            let index = (y as usize * width as usize + x as usize) * 4;
            data[index..index + 4].copy_from_slice(&color);
        }
    }
    data
}

fn offset_filter_buffer(
    source: &[u8],
    width: u32,
    height: u32,
    dx: Pt,
    dy: Pt,
    device_transform: Transform,
) -> Vec<u8> {
    let (sx, sy) = device_transform.get_scale();
    let dx = dx.to_f32() * sx.abs();
    let dy = dy.to_f32() * sy.abs();
    let mut output = vec![0u8; source.len()];
    let sample = |x: i32, y: i32, channel: usize| -> f32 {
        if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
            0.0
        } else {
            source[((y as usize * width as usize + x as usize) * 4) + channel] as f32
        }
    };
    for y in 0..height {
        let source_y = y as f32 - dy;
        let y0 = source_y.floor() as i32;
        let fy = source_y - y0 as f32;
        for x in 0..width {
            let source_x = x as f32 - dx;
            let x0 = source_x.floor() as i32;
            let fx = source_x - x0 as f32;
            let index = (y as usize * width as usize + x as usize) * 4;
            for channel in 0..4 {
                let top = sample(x0, y0, channel) * (1.0 - fx) + sample(x0 + 1, y0, channel) * fx;
                let bottom =
                    sample(x0, y0 + 1, channel) * (1.0 - fx) + sample(x0 + 1, y0 + 1, channel) * fx;
                output[index + channel] =
                    (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    output
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

fn filter_working_color(color: Color, linear_rgb: bool) -> [u8; 3] {
    let convert = |value: f32| {
        let value = value.clamp(0.0, 1.0);
        unit_to_u8(if linear_rgb {
            srgb_to_linear(value)
        } else {
            value
        })
    };
    [convert(color.r), convert(color.g), convert(color.b)]
}

fn convert_filter_buffer_to_linear(data: &mut [u8]) {
    convert_filter_buffer_color_space(data, srgb_to_linear);
}

fn convert_filter_buffer_to_srgb(data: &mut [u8]) {
    convert_filter_buffer_color_space(data, linear_to_srgb);
}

fn convert_filter_buffer_color_space(data: &mut [u8], convert: fn(f32) -> f32) {
    for pixel in data.chunks_exact_mut(4) {
        let alpha = pixel[3];
        if alpha == 0 {
            pixel.fill(0);
            continue;
        }
        let (r, g, b, _) = unpremul_rgba(pixel[0], pixel[1], pixel[2], alpha);
        pixel[0] = premul_u8(unit_to_u8(convert(r as f32 / 255.0)), alpha);
        pixel[1] = premul_u8(unit_to_u8(convert(g as f32 / 255.0)), alpha);
        pixel[2] = premul_u8(unit_to_u8(convert(b as f32 / 255.0)), alpha);
    }
}

fn apply_svg_color_matrix(data: &mut [u8], matrix: &[f32; 20]) {
    for pixel in data.chunks_exact_mut(4) {
        let alpha = pixel[3];
        if alpha == 0 {
            pixel.fill(0);
            continue;
        }
        let (r, g, b, _) = unpremul_rgba(pixel[0], pixel[1], pixel[2], alpha);
        let channels = [
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            alpha as f32 / 255.0,
        ];
        let mut output = [0.0; 4];
        for row in 0..4 {
            output[row] = (matrix[row * 5] * channels[0]
                + matrix[row * 5 + 1] * channels[1]
                + matrix[row * 5 + 2] * channels[2]
                + matrix[row * 5 + 3] * channels[3]
                + matrix[row * 5 + 4])
                .clamp(0.0, 1.0);
        }
        let output_alpha = unit_to_u8(output[3]);
        pixel[0] = premul_u8(unit_to_u8(output[0]), output_alpha);
        pixel[1] = premul_u8(unit_to_u8(output[1]), output_alpha);
        pixel[2] = premul_u8(unit_to_u8(output[2]), output_alpha);
        pixel[3] = output_alpha;
    }
}

fn component_transfer_value(function: &SvgComponentTransferFunction, value: f32) -> f32 {
    match function {
        SvgComponentTransferFunction::Identity => value,
        SvgComponentTransferFunction::Table(values) => {
            if values.is_empty() {
                return value;
            }
            if values.len() == 1 {
                return values[0];
            }
            let position = value.clamp(0.0, 1.0) * (values.len() - 1) as f32;
            let index = position.floor() as usize;
            let next = (index + 1).min(values.len() - 1);
            values[index] + (values[next] - values[index]) * (position - index as f32)
        }
        SvgComponentTransferFunction::Discrete(values) => {
            if values.is_empty() {
                value
            } else {
                values[((value.clamp(0.0, 1.0) * values.len() as f32).floor() as usize)
                    .min(values.len() - 1)]
            }
        }
        SvgComponentTransferFunction::Linear { slope, intercept } => slope * value + intercept,
        SvgComponentTransferFunction::Gamma {
            amplitude,
            exponent,
            offset,
        } => amplitude * value.max(0.0).powf(*exponent) + offset,
    }
    .clamp(0.0, 1.0)
}

fn apply_svg_component_transfer(data: &mut [u8], functions: &[SvgComponentTransferFunction; 4]) {
    for pixel in data.chunks_exact_mut(4) {
        let alpha = pixel[3];
        if alpha == 0 {
            pixel.fill(0);
            continue;
        }
        let (r, g, b, _) = unpremul_rgba(pixel[0], pixel[1], pixel[2], alpha);
        let mut channels = [
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            alpha as f32 / 255.0,
        ];
        for index in 0..4 {
            channels[index] = component_transfer_value(&functions[index], channels[index]);
        }
        let output_alpha = unit_to_u8(channels[3]);
        pixel[0] = premul_u8(unit_to_u8(channels[0]), output_alpha);
        pixel[1] = premul_u8(unit_to_u8(channels[1]), output_alpha);
        pixel[2] = premul_u8(unit_to_u8(channels[2]), output_alpha);
        pixel[3] = output_alpha;
    }
}

fn composite_in(input: &mut [u8], input2: &[u8]) {
    for (first, second) in input.chunks_exact_mut(4).zip(input2.chunks_exact(4)) {
        let alpha = second[3] as u16;
        for channel in first.iter_mut() {
            *channel = ((*channel as u16 * alpha + 127) / 255) as u8;
        }
    }
}

fn blend_filter_buffers(foreground: &[u8], background: &[u8], mode: MixBlendMode) -> Vec<u8> {
    let mut output = vec![0u8; foreground.len()];
    for ((out, source), backdrop) in output
        .chunks_exact_mut(4)
        .zip(foreground.chunks_exact(4))
        .zip(background.chunks_exact(4))
    {
        let source_alpha = source[3] as f32 / 255.0;
        let backdrop_alpha = backdrop[3] as f32 / 255.0;
        let source_rgb = if source[3] == 0 {
            [0.0; 3]
        } else {
            let (r, g, b, _) = unpremul_rgba(source[0], source[1], source[2], source[3]);
            [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
        };
        let backdrop_rgb = if backdrop[3] == 0 {
            [0.0; 3]
        } else {
            let (r, g, b, _) = unpremul_rgba(backdrop[0], backdrop[1], backdrop[2], backdrop[3]);
            [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
        };
        let source_space = source_rgb;
        let backdrop_space = backdrop_rgb;
        let output_alpha = source_alpha + backdrop_alpha - source_alpha * backdrop_alpha;
        for channel in 0..3 {
            let blended = match mode {
                MixBlendMode::Multiply => source_space[channel] * backdrop_space[channel],
                MixBlendMode::Screen => {
                    source_space[channel] + backdrop_space[channel]
                        - source_space[channel] * backdrop_space[channel]
                }
                MixBlendMode::Darken => source_space[channel].min(backdrop_space[channel]),
                MixBlendMode::Lighten => source_space[channel].max(backdrop_space[channel]),
                _ => source_space[channel],
            };
            let premultiplied = (1.0 - backdrop_alpha) * source_space[channel] * source_alpha
                + (1.0 - source_alpha) * backdrop_space[channel] * backdrop_alpha
                + source_alpha * backdrop_alpha * blended;
            let unpremultiplied = if output_alpha <= f32::EPSILON {
                0.0
            } else {
                premultiplied / output_alpha
            };
            out[channel] = premul_u8(unit_to_u8(unpremultiplied), unit_to_u8(output_alpha));
        }
        out[3] = unit_to_u8(output_alpha);
    }
    output
}

fn morphology_filter_buffer(
    source: &[u8],
    width: u32,
    height: u32,
    radius_x: usize,
    radius_y: usize,
    dilate: bool,
) -> Vec<u8> {
    if radius_x == 0 && radius_y == 0 {
        return source.to_vec();
    }
    let width = width as usize;
    let height = height as usize;
    let mut horizontal = vec![0u8; source.len()];
    let mut output = vec![0u8; source.len()];
    for y in 0..height {
        for channel in 0..4 {
            let line = (0..width)
                .map(|x| source[(y * width + x) * 4 + channel])
                .collect::<Vec<_>>();
            let filtered = sliding_extreme(&line, radius_x, dilate);
            for (x, value) in filtered.into_iter().enumerate() {
                horizontal[(y * width + x) * 4 + channel] = value;
            }
        }
    }
    for x in 0..width {
        for channel in 0..4 {
            let line = (0..height)
                .map(|y| horizontal[(y * width + x) * 4 + channel])
                .collect::<Vec<_>>();
            let filtered = sliding_extreme(&line, radius_y, dilate);
            for (y, value) in filtered.into_iter().enumerate() {
                output[(y * width + x) * 4 + channel] = value;
            }
        }
    }
    output
}

fn sliding_extreme(values: &[u8], radius: usize, maximum: bool) -> Vec<u8> {
    if radius == 0 || values.is_empty() {
        return values.to_vec();
    }
    let mut output = vec![0u8; values.len()];
    let mut deque: VecDeque<(isize, u8)> = VecDeque::new();
    let radius = radius as isize;
    for index in -radius..values.len() as isize + radius {
        let value = if index < 0 || index >= values.len() as isize {
            0
        } else {
            values[index as usize]
        };
        while deque.back().is_some_and(|(_, candidate)| {
            if maximum {
                *candidate <= value
            } else {
                *candidate >= value
            }
        }) {
            deque.pop_back();
        }
        deque.push_back((index, value));
        let minimum_index = index - radius * 2;
        while deque
            .front()
            .is_some_and(|(candidate, _)| *candidate < minimum_index)
        {
            deque.pop_front();
        }
        if index >= radius {
            let output_index = (index - radius) as usize;
            if output_index < output.len() {
                output[output_index] = deque.front().map(|(_, value)| *value).unwrap_or(0);
            }
        }
    }
    output
}

fn draw_filter_drop_shadow(
    pixmap: &mut Pixmap,
    offscreen: &Pixmap,
    state: &RasterState,
    shadow: FilterDropShadowSpec,
    filter_opacity: f32,
    device_transform: Transform,
) {
    let width = offscreen.width();
    let height = offscreen.height();
    if width == 0 || height == 0 || shadow.opacity <= 0.0 {
        return;
    }

    let (sx, sy) = device_transform.get_scale();
    let dx = shadow.offset_x.to_f32() * sx.abs();
    let dy = shadow.offset_y.to_f32() * sy.abs();
    let alpha_scale = (shadow.opacity * filter_opacity.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    if alpha_scale <= 0.0 {
        return;
    }

    let mut shadow_data = vec![0_u8; (width as usize) * (height as usize) * 4];
    let src = offscreen.data();
    let color_r = (shadow.color.r.clamp(0.0, 1.0) * 255.0).round() as u8;
    let color_g = (shadow.color.g.clamp(0.0, 1.0) * 255.0).round() as u8;
    let color_b = (shadow.color.b.clamp(0.0, 1.0) * 255.0).round() as u8;

    let sample_alpha = |x: i32, y: i32| -> f32 {
        if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
            return 0.0;
        }
        let index = ((y as usize) * (width as usize) + x as usize) * 4 + 3;
        src[index] as f32
    };
    for target_y in 0..height {
        let source_y = target_y as f32 - dy;
        let source_y0 = source_y.floor() as i32;
        let fy = source_y - source_y0 as f32;
        if source_y0 < -1 || source_y0 >= height as i32 {
            continue;
        }
        for target_x in 0..width {
            let source_x = target_x as f32 - dx;
            let source_x0 = source_x.floor() as i32;
            let fx = source_x - source_x0 as f32;
            if source_x0 < -1 || source_x0 >= width as i32 {
                continue;
            }
            let top = sample_alpha(source_x0, source_y0) * (1.0 - fx)
                + sample_alpha(source_x0 + 1, source_y0) * fx;
            let bottom = sample_alpha(source_x0, source_y0 + 1) * (1.0 - fx)
                + sample_alpha(source_x0 + 1, source_y0 + 1) * fx;
            let source_alpha = top * (1.0 - fy) + bottom * fy;
            if source_alpha <= 0.0 {
                continue;
            }
            let dst_idx = ((target_y as usize) * (width as usize) + target_x as usize) * 4;
            let alpha = (source_alpha * alpha_scale).round().clamp(0.0, 255.0) as u8;
            shadow_data[dst_idx] = color_r;
            shadow_data[dst_idx + 1] = color_g;
            shadow_data[dst_idx + 2] = color_b;
            shadow_data[dst_idx + 3] = alpha;
        }
    }

    let blur_px = backdrop_blur_sigma_px(shadow.blur_radius, device_transform);
    if blur_px > 0.05 {
        let Some(base_img) = crate::image_native::RgbaImage::from_raw(width, height, shadow_data)
        else {
            return;
        };
        shadow_data = crate::image_native::blur_rgba(&base_img, blur_px).into_raw();
    }
    premultiply_rgba(&mut shadow_data);

    let Some(size) = IntSize::from_wh(width, height) else {
        return;
    };
    let Some(shadow_pixmap) = Pixmap::from_vec(shadow_data, size) else {
        return;
    };

    let mut paint = PixmapPaint::default();
    paint.quality = FilterQuality::Bilinear;
    paint.opacity = 1.0;
    paint.blend_mode = sk_blend_mode(state.blend_mode);
    draw_pixmap_blended(
        pixmap,
        0,
        0,
        shadow_pixmap.as_ref(),
        &paint,
        Transform::identity(),
        state.clip_mask.as_ref(),
        state.blend_mode,
    );
}

fn premultiply_rgba(data: &mut [u8]) {
    for px in data.chunks_exact_mut(4) {
        let alpha = px[3];
        px[0] = premul_u8(px[0], alpha);
        px[1] = premul_u8(px[1], alpha);
        px[2] = premul_u8(px[2], alpha);
    }
}

fn apply_filter_to_premul_rgba(data: &mut [u8], filter: &PaintFilterSpec) {
    let saturate = filter.saturate.max(0.0);
    let brightness = filter.brightness.max(0.0);
    let contrast = filter.contrast.max(0.0);
    let invert = filter.invert.clamp(0.0, 1.0);
    let sepia = filter.sepia.clamp(0.0, 1.0);
    let hue_rotate = filter.hue_rotate;
    let filter_opacity = filter.opacity.clamp(0.0, 1.0);

    for px in data.chunks_exact_mut(4) {
        let alpha = px[3];
        if alpha == 0 {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
            continue;
        }

        let (r, g, b, _) = unpremul_rgba(px[0], px[1], px[2], alpha);
        let mut filt_r = r as f32;
        let mut filt_g = g as f32;
        let mut filt_b = b as f32;
        apply_saturate_rgb(&mut filt_r, &mut filt_g, &mut filt_b, saturate);
        apply_contrast_rgb(&mut filt_r, &mut filt_g, &mut filt_b, contrast);
        apply_hue_rotate_rgb(&mut filt_r, &mut filt_g, &mut filt_b, hue_rotate);
        apply_invert_rgb(&mut filt_r, &mut filt_g, &mut filt_b, invert);
        apply_sepia_rgb(&mut filt_r, &mut filt_g, &mut filt_b, sepia);
        apply_brightness_rgb(&mut filt_r, &mut filt_g, &mut filt_b, brightness);

        let out_alpha = ((alpha as f32) * filter_opacity).round().clamp(0.0, 255.0) as u8;
        px[0] = premul_u8(filt_r.round().clamp(0.0, 255.0) as u8, out_alpha);
        px[1] = premul_u8(filt_g.round().clamp(0.0, 255.0) as u8, out_alpha);
        px[2] = premul_u8(filt_b.round().clamp(0.0, 255.0) as u8, out_alpha);
        px[3] = out_alpha;
    }
}

fn apply_hue_rotate_rgb(r: &mut f32, g: &mut f32, b: &mut f32, radians: f32) {
    if radians.abs() <= 1.0e-6 {
        return;
    }
    let cos = radians.cos();
    let sin = radians.sin();
    let out_r = *r * (0.213 + cos * 0.787 - sin * 0.213)
        + *g * (0.715 - cos * 0.715 - sin * 0.715)
        + *b * (0.072 - cos * 0.072 + sin * 0.928);
    let out_g = *r * (0.213 - cos * 0.213 + sin * 0.143)
        + *g * (0.715 + cos * 0.285 + sin * 0.140)
        + *b * (0.072 - cos * 0.072 - sin * 0.283);
    let out_b = *r * (0.213 - cos * 0.213 - sin * 0.787)
        + *g * (0.715 - cos * 0.715 + sin * 0.715)
        + *b * (0.072 + cos * 0.928 + sin * 0.072);
    *r = out_r.clamp(0.0, 255.0);
    *g = out_g.clamp(0.0, 255.0);
    *b = out_b.clamp(0.0, 255.0);
}

fn apply_sepia_rgb(r: &mut f32, g: &mut f32, b: &mut f32, sepia: f32) {
    if sepia <= 1.0e-6 {
        return;
    }
    let amount = sepia.clamp(0.0, 1.0);
    let sepia_r = *r * 0.393 + *g * 0.769 + *b * 0.189;
    let sepia_g = *r * 0.349 + *g * 0.686 + *b * 0.168;
    let sepia_b = *r * 0.272 + *g * 0.534 + *b * 0.131;
    *r = (*r * (1.0 - amount) + sepia_r * amount).clamp(0.0, 255.0);
    *g = (*g * (1.0 - amount) + sepia_g * amount).clamp(0.0, 255.0);
    *b = (*b * (1.0 - amount) + sepia_b * amount).clamp(0.0, 255.0);
}

fn apply_invert_rgb(r: &mut f32, g: &mut f32, b: &mut f32, invert: f32) {
    if invert <= 1.0e-6 {
        return;
    }
    let amount = invert.clamp(0.0, 1.0);
    *r = (*r * (1.0 - amount) + (255.0 - *r) * amount).clamp(0.0, 255.0);
    *g = (*g * (1.0 - amount) + (255.0 - *g) * amount).clamp(0.0, 255.0);
    *b = (*b * (1.0 - amount) + (255.0 - *b) * amount).clamp(0.0, 255.0);
}

fn apply_contrast_rgb(r: &mut f32, g: &mut f32, b: &mut f32, contrast: f32) {
    if (contrast - 1.0).abs() <= 1.0e-6 {
        return;
    }
    let factor = contrast.max(0.0);
    let intercept = 127.5 * (1.0 - factor);
    *r = (*r * factor + intercept).clamp(0.0, 255.0);
    *g = (*g * factor + intercept).clamp(0.0, 255.0);
    *b = (*b * factor + intercept).clamp(0.0, 255.0);
}

fn apply_brightness_rgb(r: &mut f32, g: &mut f32, b: &mut f32, brightness: f32) {
    if (brightness - 1.0).abs() <= 1.0e-6 {
        return;
    }
    let factor = brightness.max(0.0);
    *r = (*r * factor).clamp(0.0, 255.0);
    *g = (*g * factor).clamp(0.0, 255.0);
    *b = (*b * factor).clamp(0.0, 255.0);
}

fn backdrop_blur_sigma_px(blur_radius: Pt, transform: Transform) -> f32 {
    if blur_radius <= Pt::ZERO {
        return 0.0;
    }
    let (sx, sy) = transform.get_scale();
    let scale = ((sx.abs() + sy.abs()) * 0.5).max(0.0);
    blur_radius.to_f32().max(0.0) * scale
}

fn rounded_rect_path(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Option<Path> {
    let rect = Rect::from_xywh(x, y, width, height)?;
    if radius <= 0.0 {
        return Some(PathBuilder::from_rect(rect));
    }
    let mut r = radius.max(0.0);
    let max_r = (width * 0.5).min(height * 0.5);
    if r > max_r {
        r = max_r;
    }
    if r <= 0.0 {
        return Some(PathBuilder::from_rect(rect));
    }

    let mut builder = PathBuilder::new();
    let k = 0.55228475_f32;
    let c = r * k;
    let right = x + width;
    let bottom = y + height;
    builder.move_to(x + r, y);
    builder.line_to(right - r, y);
    builder.cubic_to(right - r + c, y, right, y + r - c, right, y + r);
    builder.line_to(right, bottom - r);
    builder.cubic_to(
        right,
        bottom - r + c,
        right - r + c,
        bottom,
        right - r,
        bottom,
    );
    builder.line_to(x + r, bottom);
    builder.cubic_to(x + r - c, bottom, x, bottom - r + c, x, bottom - r);
    builder.line_to(x, y + r);
    builder.cubic_to(x, y + r - c, x + r - c, y, x + r, y);
    builder.close();
    builder.finish()
}

fn mask_bounds(mask: &[u8], width: u32, height: u32) -> (u32, u32, u32, u32) {
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0_u32;
    let mut max_y = 0_u32;
    for y in 0..height {
        let row_off = (y as usize) * (width as usize);
        for x in 0..width {
            if mask[row_off + x as usize] == 0 {
                continue;
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + 1);
            max_y = max_y.max(y + 1);
        }
    }
    (min_x, min_y, max_x, max_y)
}

fn mul_alpha_u8(a: u8, b: u8) -> u8 {
    let prod = (a as u16) * (b as u16) + 127;
    ((prod + (prod >> 8)) >> 8) as u8
}

fn unpremul_rgba(r: u8, g: u8, b: u8, a: u8) -> (u8, u8, u8, u8) {
    if a == 0 {
        return (0, 0, 0, 0);
    }
    if a == 255 {
        return (r, g, b, a);
    }
    let inv = 255.0 / (a as f32);
    (
        ((r as f32) * inv).round().clamp(0.0, 255.0) as u8,
        ((g as f32) * inv).round().clamp(0.0, 255.0) as u8,
        ((b as f32) * inv).round().clamp(0.0, 255.0) as u8,
        a,
    )
}

fn apply_saturate_rgb(r: &mut f32, g: &mut f32, b: &mut f32, saturate: f32) {
    if (saturate - 1.0).abs() <= 1.0e-6 {
        return;
    }
    let rr = *r;
    let gg = *g;
    let bb = *b;
    let s = saturate.max(0.0);
    *r = ((0.213 + 0.787 * s) * rr + (0.715 - 0.715 * s) * gg + (0.072 - 0.072 * s) * bb)
        .clamp(0.0, 255.0);
    *g = ((0.213 - 0.213 * s) * rr + (0.715 + 0.285 * s) * gg + (0.072 - 0.072 * s) * bb)
        .clamp(0.0, 255.0);
    *b = ((0.213 - 0.213 * s) * rr + (0.715 - 0.715 * s) * gg + (0.072 + 0.928 * s) * bb)
        .clamp(0.0, 255.0);
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
    let Some(shader) = build_shading_shader(
        shading,
        page_height_pt,
        state.fill_opacity,
        state.mask_shader_phase,
    ) else {
        return;
    };
    let mut paint = Paint::default();
    paint.shader = shader;
    paint.anti_alias = true;
    paint.blend_mode = sk_blend_mode(state.blend_mode);
    fill_path_blended(
        pixmap,
        &page_path,
        &paint,
        FillRule::Winding,
        base_transform.pre_concat(state.transform),
        state.clip_mask.as_ref(),
        state.blend_mode,
    );
}

fn build_shading_shader(
    shading: &Shading,
    page_height_pt: f32,
    opacity: f32,
    mask_phase: (f32, f32),
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
            ..
        } => {
            let start = Point::from_xy(*x0 + mask_phase.0, page_height_pt - *y0 - mask_phase.1);
            let end = Point::from_xy(*x1 + mask_phase.0, page_height_pt - *y1 - mask_phase.1);
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
        Shading::Conic {
            center_x,
            center_y,
            start_angle_deg,
            stops,
            ..
        } => {
            let center = Point::from_xy(
                *center_x + mask_phase.0,
                page_height_pt - *center_y - mask_phase.1,
            );
            let stops = shading_stops(stops, opacity);
            ConicGradient::new(center, *start_angle_deg, stops, Transform::identity())
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
            to_sk_color(stop.color, opacity * stop.alpha.clamp(0.0, 1.0)),
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
    let paint = fill_paint(state.fill_color, state.fill_opacity, state.blend_mode);
    let device_transform = base_transform.pre_concat(state.transform);
    let mut try_draw = |font_data: &[u8], used_system_fallback: bool| -> Result<(), &'static str> {
        let Ok(face) = SfntFace::parse(font_data, 0) else {
            return Err("parse_failed");
        };
        let outlines = raster_outline_source(&face)?;

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
            if !outline_raster_glyph(&face, &outlines, placement.glyph_id, &mut builder) {
                continue;
            }
            let Some(path) = builder.finish() else {
                continue;
            };
            if matches!(state.text_rendering_mode, 0 | 2) {
                fill_path_blended(
                    pixmap,
                    &path,
                    &paint,
                    FillRule::Winding,
                    device_transform,
                    state.clip_mask.as_ref(),
                    state.blend_mode,
                );
            }
            if matches!(state.text_rendering_mode, 1 | 2) {
                let stroke_paint =
                    fill_paint(state.stroke_color, state.stroke_opacity, state.blend_mode);
                let stroke = build_stroke(state);
                stroke_path_blended(
                    pixmap,
                    &path,
                    &stroke_paint,
                    &stroke,
                    device_transform,
                    state.clip_mask.as_ref(),
                    state.blend_mode,
                );
            }
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

    let paint = fill_paint(state.fill_color, state.fill_opacity, state.blend_mode);
    let device_transform = base_transform.pre_concat(state.transform);
    let run_transform = Transform::from_row(m00, m01, m10, m11, x, y);

    let mut try_draw = |font_data: &[u8]| -> Result<(), &'static str> {
        let Ok(face) = SfntFace::parse(font_data, 0) else {
            return Err("parse_failed");
        };
        let outlines = raster_outline_source(&face)?;
        let placements = layout_text_glyphs(font_data, text, font_size, 0.0, 0.0, shape_text);
        if placements.is_empty() {
            return Err("no_placements");
        }
        let mut drawn = 0usize;
        for placement in placements {
            let mut builder =
                GlyphPathBuilder::new(placement.origin_x, placement.origin_y, placement.scale);
            if !outline_raster_glyph(&face, &outlines, placement.glyph_id, &mut builder) {
                continue;
            }
            let Some(path) = builder.finish() else {
                continue;
            };
            fill_path_blended(
                pixmap,
                &path,
                &paint,
                FillRule::Winding,
                device_transform.pre_concat(run_transform),
                state.clip_mask.as_ref(),
                state.blend_mode,
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
    draw_glyph_run_impl(
        pixmap,
        state,
        x,
        y,
        glyph_ids,
        advances,
        &[],
        None,
        m00,
        m01,
        m10,
        m11,
        page_height_pt,
        base_transform,
        registry,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_synthetic_bold_glyph_run(
    pixmap: &mut Pixmap,
    state: &RasterState,
    x: f32,
    y: f32,
    glyph_ids: &[u16],
    advances: &[(Pt, Pt)],
    offsets: &[(Pt, Pt)],
    stroke_width: Pt,
    page_height_pt: f32,
    base_transform: Transform,
    registry: Option<&FontRegistry>,
) {
    let stroke_width = browser_effect_synthetic_bold_stroke_width(state.font_size, stroke_width);
    draw_glyph_run_impl(
        pixmap,
        state,
        x,
        y,
        glyph_ids,
        advances,
        offsets,
        Some(stroke_width.max(Pt::ZERO)),
        1.0,
        0.0,
        0.0,
        1.0,
        page_height_pt,
        base_transform,
        registry,
    );
}

fn browser_effect_synthetic_bold_stroke_width(font_size: Pt, stroke_width: Pt) -> Pt {
    if stroke_width <= Pt::ZERO || font_size <= Pt::ZERO {
        return Pt::ZERO;
    }

    // Skia's fake-bold raster paint interpolates its added stroke from 1/24em
    // at 9 CSS px to 1/32em at 36 CSS px.  Fullbleed's reusable vector glyph
    // program stores the stable 1/32em width used by the PDF path; apply only
    // the missing raster-paint multiplier while compiling an effect surface.
    // This keeps layout, advances, extraction text, and direct Type 3 output
    // immutable while matching the browser shader at its actual CSS size.
    let css_px = font_size.to_f32() * (4.0 / 3.0);
    let multiplier = if css_px <= 9.0 {
        4.0 / 3.0
    } else if css_px >= 36.0 {
        1.0
    } else {
        let progress = (css_px - 9.0) / 27.0;
        (4.0 / 3.0) + progress * (1.0 - 4.0 / 3.0)
    };
    Pt::from_f32(stroke_width.to_f32() * multiplier)
}

#[allow(clippy::too_many_arguments)]
fn draw_glyph_run_impl(
    pixmap: &mut Pixmap,
    state: &RasterState,
    x: f32,
    y: f32,
    glyph_ids: &[u16],
    advances: &[(Pt, Pt)],
    offsets: &[(Pt, Pt)],
    synthetic_stroke_width: Option<Pt>,
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
    let paint = fill_paint(state.fill_color, state.fill_opacity, state.blend_mode);
    let stroke_paint = fill_paint(state.stroke_color, state.stroke_opacity, state.blend_mode);
    let synthetic_stroke = synthetic_stroke_width.and_then(|width| {
        if width <= Pt::ZERO {
            return None;
        }
        let mut stroke = Stroke::default();
        stroke.width = width.to_f32();
        stroke.miter_limit = 4.0;
        stroke.line_cap = LineCap::Butt;
        stroke.line_join = LineJoin::Miter;
        Some(stroke)
    });
    let device_transform = base_transform.pre_concat(state.transform);

    let mut try_draw = |font_data: &[u8], used_system_fallback: bool| -> Result<(), &'static str> {
        let Ok(face) = SfntFace::parse(font_data, 0) else {
            return Err("parse_failed");
        };
        let outlines = raster_outline_source(&face)?;
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
                if outline_raster_glyph(&face, &outlines, *gid, &mut builder) {
                    if let Some(path) = builder.finish() {
                        let (offset_x, offset_y) = offsets
                            .get(idx)
                            .map(|(ox, oy)| (ox.to_f32(), oy.to_f32()))
                            .unwrap_or((0.0, 0.0));
                        let local = Transform::from_row(
                            m00,
                            m01,
                            m10,
                            m11,
                            pen_x + offset_x,
                            pen_y - offset_y,
                        );
                        fill_path_blended(
                            pixmap,
                            &path,
                            &paint,
                            FillRule::Winding,
                            device_transform.pre_concat(local),
                            state.clip_mask.as_ref(),
                            state.blend_mode,
                        );
                        if let Some(stroke) = synthetic_stroke.as_ref() {
                            stroke_path_blended(
                                pixmap,
                                &path,
                                &stroke_paint,
                                stroke,
                                device_transform.pre_concat(local),
                                state.clip_mask.as_ref(),
                                state.blend_mode,
                            );
                        }
                        drawn += 1;
                    }
                } else if face.glyph_hor_advance(SfntGlyphId(*gid)).is_some() {
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
                    face.glyph_hor_advance(SfntGlyphId(*gid)).map(|w| {
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

    let Some(shaped) = text_shape::shape(font_data, text) else {
        return layout_text_glyphs_unshaped(font_data, text, font_size, baseline_x, baseline_y);
    };
    let hb_units = shaped.units_per_em as f32;
    let scale = font_size / hb_units;
    if shaped.glyphs.is_empty() {
        return layout_text_glyphs_unshaped(font_data, text, font_size, baseline_x, baseline_y);
    }

    let mut out = Vec::with_capacity(shaped.glyphs.len());
    let mut pen_x = 0.0f32;
    let mut pen_y = 0.0f32;
    for glyph in shaped.glyphs {
        let gid = glyph.glyph_id;
        if gid == 0 {
            pen_x += (glyph.x_advance as f32 / hb_units) * font_size;
            pen_y += (glyph.y_advance as f32 / hb_units) * font_size;
            continue;
        }
        let x_off = (glyph.x_offset as f32 / hb_units) * font_size;
        let y_off = (glyph.y_offset as f32 / hb_units) * font_size;
        out.push(GlyphPlacement {
            glyph_id: gid,
            origin_x: baseline_x + pen_x + x_off,
            origin_y: baseline_y + pen_y + y_off,
            scale,
        });
        pen_x += (glyph.x_advance as f32 / hb_units) * font_size;
        pen_y += (glyph.y_advance as f32 / hb_units) * font_size;
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
    let Ok(face) = SfntFace::parse(font_data, 0) else {
        return Vec::new();
    };
    let units_per_em = face.units_per_em().max(1) as f32;
    let scale = font_size / units_per_em;

    let mut out = Vec::new();
    let mut pen_x = 0.0f32;
    for ch in text.chars() {
        let gid = face.glyph_index(ch as u32).map(|id| id.0).unwrap_or(0);
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
        let advance_units = face.glyph_hor_advance(SfntGlyphId(gid)).unwrap_or(0) as f32;
        let mut adv = (advance_units / units_per_em) * font_size;
        if adv <= 0.0 {
            adv = font_size * 0.5;
        }
        pen_x += adv;
    }
    out
}

#[derive(Clone, Debug)]
struct SystemFontCacheEntry {
    bytes: Arc<Vec<u8>>,
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    path: std::path::PathBuf,
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    candidate_file_names: Vec<String>,
}

#[cfg(feature = "python")]
#[derive(Clone, Debug)]
pub(crate) struct SystemFontResolutionTrace {
    pub(crate) matched_family: String,
    pub(crate) resolved_path: std::path::PathBuf,
    pub(crate) resolved_file_name: String,
    pub(crate) candidate_file_names: Vec<String>,
}

static SYSTEM_FONT_CACHE: OnceLock<Mutex<HashMap<String, Option<SystemFontCacheEntry>>>> =
    OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FontStyleVariant {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

fn resolve_system_font_bytes(font_name: &str) -> Option<Arc<Vec<u8>>> {
    resolve_system_font_match(font_name).map(|entry| entry.bytes)
}

#[cfg(feature = "python")]
pub(crate) fn inspect_system_font_resolution(font_name: &str) -> Option<SystemFontResolutionTrace> {
    let families = font_family_candidates(font_name);
    for family in families {
        let key = normalize_font_family(&family);
        if key.is_empty() {
            continue;
        }
        let cache = SYSTEM_FONT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Ok(cache_guard) = cache.lock() {
            if let Some(Some(entry)) = cache_guard.get(&key) {
                return Some(build_system_font_resolution_trace(&family, entry));
            }
        }
        if let Some(entry) = load_system_font_from_candidates(&family) {
            if let Ok(mut cache_guard) = cache.lock() {
                cache_guard.insert(key, Some(entry.clone()));
            }
            return Some(build_system_font_resolution_trace(&family, &entry));
        }
    }
    None
}

#[cfg(feature = "python")]
fn build_system_font_resolution_trace(
    matched_family: &str,
    entry: &SystemFontCacheEntry,
) -> SystemFontResolutionTrace {
    let resolved_file_name = entry
        .path
        .file_name()
        .and_then(|v| v.to_str())
        .map(|v| v.to_string())
        .unwrap_or_else(|| entry.path.to_string_lossy().to_string());
    SystemFontResolutionTrace {
        matched_family: matched_family.to_string(),
        resolved_path: entry.path.clone(),
        resolved_file_name,
        candidate_file_names: entry.candidate_file_names.clone(),
    }
}

fn resolve_system_font_match(font_name: &str) -> Option<SystemFontCacheEntry> {
    let families = font_family_candidates(font_name);
    let cache = SYSTEM_FONT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    for family in families {
        let key = normalize_font_family(&family);
        if key.is_empty() {
            continue;
        }

        if let Ok(cache_guard) = cache.lock() {
            if let Some(entry) = cache_guard.get(&key) {
                if let Some(entry) = entry {
                    return Some(entry.clone());
                }
                continue;
            }
        }

        let loaded = load_system_font_from_candidates(&family);
        if let Ok(mut cache_guard) = cache.lock() {
            cache_guard.insert(key, loaded.clone());
        }
        if let Some(entry) = loaded {
            return Some(entry);
        }
    }

    None
}

fn load_system_font_from_candidates(font_name: &str) -> Option<SystemFontCacheEntry> {
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
            if SfntFace::parse(&bytes, 0).is_ok() {
                return Some(SystemFontCacheEntry {
                    bytes: Arc::new(bytes),
                    path,
                    candidate_file_names: candidates.clone(),
                });
            }
        }
    }
    None
}

fn system_font_dirs() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();

    #[cfg(target_os = "windows")]
    {
        roots.push(std::path::PathBuf::from(r"C:\Windows\Fonts"));
        if let Ok(windir) = std::env::var("WINDIR") {
            roots.push(std::path::PathBuf::from(windir).join("Fonts"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        roots.push(std::path::PathBuf::from("/usr/share/fonts"));
        roots.push(std::path::PathBuf::from("/usr/local/share/fonts"));
        if let Ok(home) = std::env::var("HOME") {
            roots.push(std::path::PathBuf::from(home).join(".fonts"));
        }
    }

    #[cfg(target_os = "macos")]
    {
        roots.push(std::path::PathBuf::from("/System/Library/Fonts"));
        roots.push(std::path::PathBuf::from("/Library/Fonts"));
        if let Ok(home) = std::env::var("HOME") {
            roots.push(std::path::PathBuf::from(home).join("Library/Fonts"));
        }
    }

    if let Ok(extra) = std::env::var("FULLBLEED_FONT_DIR") {
        for path in std::env::split_paths(&extra) {
            if !path.as_os_str().is_empty() {
                roots.push(path);
            }
        }
    }

    let mut dirs = Vec::new();
    for root in roots {
        collect_system_font_dirs(&root, 0, &mut dirs);
    }
    dirs
}

fn collect_system_font_dirs(
    directory: &std::path::Path,
    depth: usize,
    out: &mut Vec<std::path::PathBuf>,
) {
    if !directory.is_dir() {
        return;
    }
    out.push(directory.to_path_buf());
    if depth >= 4 {
        return;
    }

    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut children = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|file_type| file_type.is_dir())
                .map(|_| entry.path())
        })
        .collect::<Vec<_>>();
    children.sort();
    for child in children {
        collect_system_font_dirs(&child, depth + 1, out);
    }
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
                &[
                    "consolaz.ttf",
                    "courbi.ttf",
                    "LiberationMono-BoldItalic.ttf",
                ],
            );
        }
        "serif" => {
            extend_style_candidates(
                &mut out,
                style,
                &[
                    "times.ttf",
                    "timesnewroman.ttf",
                    "LiberationSerif-Regular.ttf",
                ],
                &[
                    "timesbd.ttf",
                    "timesnewromanbold.ttf",
                    "LiberationSerif-Bold.ttf",
                ],
                &[
                    "timesi.ttf",
                    "timesnewromanitalic.ttf",
                    "LiberationSerif-Italic.ttf",
                ],
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
                &[
                    "times.ttf",
                    "timesnewroman.ttf",
                    "LiberationSerif-Regular.ttf",
                ],
                &[
                    "timesbd.ttf",
                    "timesnewromanbold.ttf",
                    "LiberationSerif-Bold.ttf",
                ],
                &[
                    "timesi.ttf",
                    "timesnewromanitalic.ttf",
                    "LiberationSerif-Italic.ttf",
                ],
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
                &[
                    "SCHLBKBI.TTF",
                    "timesbi.ttf",
                    "LiberationSerif-BoldItalic.ttf",
                ],
            );
        }
        "courier" | "courier new" => {
            extend_style_candidates(
                &mut out,
                style,
                &["cour.ttf", "consola.ttf", "LiberationMono-Regular.ttf"],
                &["courbd.ttf", "consolab.ttf", "LiberationMono-Bold.ttf"],
                &["couri.ttf", "consolai.ttf", "LiberationMono-Italic.ttf"],
                &[
                    "courbi.ttf",
                    "consolaz.ttf",
                    "LiberationMono-BoldItalic.ttf",
                ],
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
                &[
                    "LiberationMono-BoldItalic.ttf",
                    "consolaz.ttf",
                    "consola.ttf",
                ],
            );
        }
        _ => {}
    }
    if !out.is_empty() {
        if matches!(
            family.as_str(),
            "ui-monospace" | "monospace" | "courier" | "courier new" | "liberation mono"
        ) {
            extend_style_candidates(
                &mut out,
                style,
                &["DejaVuSansMono.ttf"],
                &["DejaVuSansMono-Bold.ttf"],
                &["DejaVuSansMono-Oblique.ttf"],
                &["DejaVuSansMono-BoldOblique.ttf"],
            );
        } else if matches!(
            family.as_str(),
            "serif"
                | "times"
                | "times roman"
                | "times new roman"
                | "century schoolbook"
                | "new century schoolbook"
                | "liberation serif"
        ) {
            extend_style_candidates(
                &mut out,
                style,
                &["DejaVuSerif.ttf"],
                &["DejaVuSerif-Bold.ttf"],
                &["DejaVuSerif-Italic.ttf"],
                &["DejaVuSerif-BoldItalic.ttf"],
            );
        } else {
            extend_style_candidates(
                &mut out,
                style,
                &["DejaVuSans.ttf"],
                &["DejaVuSans-Bold.ttf"],
                &["DejaVuSans-Oblique.ttf"],
                &["DejaVuSans-BoldOblique.ttf"],
            );
        }
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
        if matches!(
            token,
            "bold" | "semibold" | "demibold" | "black" | "blk" | "bd"
        ) || token.contains("blk")
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
            if !out
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(candidate))
            {
                out.push((*candidate).to_string());
            }
        }
    }
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

enum RasterOutlineSource<'a> {
    TrueType,
    Cff1(CffOutlines<'a>),
    Cff2(Cff2Outlines<'a>),
}

fn raster_outline_source<'a>(face: &SfntFace<'a>) -> Result<RasterOutlineSource<'a>, &'static str> {
    if face.has_true_type_outlines() {
        Ok(RasterOutlineSource::TrueType)
    } else if face.has_cff1_outlines() {
        CffOutlines::parse(face)
            .map(RasterOutlineSource::Cff1)
            .ok_or("parse_failed")
    } else if face.has_cff2_outlines() {
        Cff2Outlines::parse(face)
            .map(RasterOutlineSource::Cff2)
            .ok_or("parse_failed")
    } else {
        Err("missing_outlines")
    }
}

fn outline_raster_glyph(
    face: &SfntFace<'_>,
    outlines: &RasterOutlineSource<'_>,
    glyph_id: u16,
    builder: &mut GlyphPathBuilder,
) -> bool {
    match outlines {
        RasterOutlineSource::TrueType => {
            face.outline_glyph(SfntGlyphId(glyph_id), builder).is_some()
        }
        RasterOutlineSource::Cff1(cff) => cff.outline(SfntGlyphId(glyph_id), builder).is_some(),
        RasterOutlineSource::Cff2(cff) => cff.outline(SfntGlyphId(glyph_id), builder).is_some(),
    }
}

impl NativeOutlineBuilder for GlyphPathBuilder {
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

fn fill_paint(color: Color, opacity: f32, blend_mode: MixBlendMode) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(to_sk_color(color, opacity));
    paint.anti_alias = true;
    paint.blend_mode = sk_blend_mode(blend_mode);
    paint
}

fn sk_blend_mode(mode: MixBlendMode) -> SkBlendMode {
    match mode {
        MixBlendMode::Normal => SkBlendMode::SourceOver,
        MixBlendMode::Multiply => SkBlendMode::Multiply,
        MixBlendMode::Screen => SkBlendMode::Screen,
        MixBlendMode::Overlay => SkBlendMode::Overlay,
        MixBlendMode::Darken => SkBlendMode::Darken,
        MixBlendMode::Lighten => SkBlendMode::Lighten,
        MixBlendMode::ColorDodge => SkBlendMode::ColorDodge,
        MixBlendMode::ColorBurn => SkBlendMode::ColorBurn,
        MixBlendMode::HardLight => SkBlendMode::HardLight,
        MixBlendMode::SoftLight => SkBlendMode::SoftLight,
        MixBlendMode::Difference => SkBlendMode::Difference,
        MixBlendMode::Exclusion => SkBlendMode::Exclusion,
        MixBlendMode::Hue => SkBlendMode::Hue,
        MixBlendMode::Saturation => SkBlendMode::Saturation,
        MixBlendMode::Color => SkBlendMode::Color,
        MixBlendMode::Luminosity => SkBlendMode::Luminosity,
        MixBlendMode::PlusLighter => SkBlendMode::Plus,
        MixBlendMode::PlusDarker => SkBlendMode::SourceOver,
    }
}

fn to_sk_color(color: Color, opacity: f32) -> RasterColor {
    let r = color.r.clamp(0.0, 1.0);
    let g = color.g.clamp(0.0, 1.0);
    let b = color.b.clamp(0.0, 1.0);
    let a = opacity.clamp(0.0, 1.0);
    RasterColor::from_rgba(r, g, b, a).unwrap_or_else(|| RasterColor::from_rgba8(0, 0, 0, 255))
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
            Some(crate::image_native::ImageFormat::Png)
        } else if mime.contains("jpeg") || mime.contains("jpg") {
            Some(crate::image_native::ImageFormat::Jpeg)
        } else {
            None
        }
    } else {
        crate::image_native::guess_format(data).ok()
    };

    let decoded = if let Some(fmt) = guessed_format {
        crate::image_native::load_from_memory_with_format(data, fmt).ok()?
    } else {
        crate::image_native::load_from_memory(data).ok()?
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
        crate::base64::decode_standard(payload).ok()?
    } else {
        payload.as_bytes().to_vec()
    };
    Some((mime, data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flowable::{SvgFilterNode, SvgFilterRegion};
    use crate::image_native::{Rgba, RgbaImage};

    fn has_non_white_pixel(img: &RgbaImage) -> bool {
        img.pixels().any(|p| {
            let [r, g, b, _a] = p.0;
            !(r == 255 && g == 255 && b == 255)
        })
    }

    #[test]
    fn svg_filter_graph_quantizes_once_in_linear_rgb_working_space() {
        let program = SvgFilterProgram {
            nodes: vec![
                SvgFilterNode {
                    primitive: SvgFilterPrimitive::Flood {
                        color: Color::rgb(21.0 / 255.0, 101.0 / 255.0, 192.0 / 255.0),
                        opacity: 1.0,
                    },
                    result: Some("blue".to_string()),
                },
                SvgFilterNode {
                    primitive: SvgFilterPrimitive::Blend {
                        input: SvgFilterInput::SourceGraphic,
                        input2: SvgFilterInput::Named("blue".to_string()),
                        mode: MixBlendMode::Multiply,
                    },
                    result: None,
                },
            ],
            region: SvgFilterRegion::default(),
            linear_rgb: true,
        };

        let output =
            apply_svg_filter_program(&[213, 0, 0, 255], 1, 1, &program, Transform::identity())
                .expect("linear RGB filter output");
        assert_eq!(output.0, [13, 0, 0, 255]);
        assert_eq!(output.1, (0, 0, 1, 1));
    }

    #[test]
    fn svg_filter_graph_uses_srgb_math_when_requested() {
        let program = SvgFilterProgram {
            nodes: vec![
                SvgFilterNode {
                    primitive: SvgFilterPrimitive::Flood {
                        color: Color::rgb(21.0 / 255.0, 101.0 / 255.0, 192.0 / 255.0),
                        opacity: 1.0,
                    },
                    result: Some("blue".to_string()),
                },
                SvgFilterNode {
                    primitive: SvgFilterPrimitive::Blend {
                        input: SvgFilterInput::SourceGraphic,
                        input2: SvgFilterInput::Named("blue".to_string()),
                        mode: MixBlendMode::Multiply,
                    },
                    result: None,
                },
            ],
            region: SvgFilterRegion::default(),
            linear_rgb: false,
        };

        let output =
            apply_svg_filter_program(&[213, 0, 0, 255], 1, 1, &program, Transform::identity())
                .expect("sRGB filter output");
        assert_eq!(output.0, [18, 0, 0, 255]);
        assert_eq!(output.1, (0, 0, 1, 1));
    }

    #[test]
    fn filtered_form_retains_transparent_svg_filter_region() {
        let mut filter = PaintFilterSpec::identity();
        filter
            .operations
            .push(PaintFilterOperation::Svg(SvgFilterProgram {
                nodes: vec![SvgFilterNode {
                    primitive: SvgFilterPrimitive::GaussianBlur {
                        input: SvgFilterInput::SourceGraphic,
                        std_deviation_x: Pt::from_f32(1.0),
                        std_deviation_y: Pt::from_f32(1.0),
                    },
                    result: None,
                }],
                region: SvgFilterRegion::default(),
                linear_rgb: false,
            }));
        let page = Pt::from_f32(72.0);
        let raster = rasterize_filtered_form(
            page,
            page,
            page,
            page,
            &[Command::DrawRect {
                x: Pt::from_f32(24.0),
                y: Pt::from_f32(24.0),
                width: Pt::from_f32(24.0),
                height: Pt::from_f32(24.0),
            }],
            Pt::ZERO,
            Pt::ZERO,
            page,
            page,
            &filter,
            false,
            300,
            None,
            false,
        )
        .expect("filter raster")
        .expect("non-empty filter raster");

        assert_eq!(raster.pixel_width, 120);
        assert_eq!(raster.pixel_height, 120);
        assert_eq!(raster.x, Pt::from_f32(90.0 * 72.0 / 300.0));
        assert_eq!(raster.y, Pt::from_f32(90.0 * 72.0 / 300.0));
    }

    #[test]
    fn color_filter_surface_retains_one_device_pixel_guard() {
        let mut filter = PaintFilterSpec::identity();
        filter
            .operations
            .push(PaintFilterOperation::Brightness(0.75));
        let page = Pt::from_f32(72.0);
        let raster = rasterize_filtered_form(
            page,
            page,
            page,
            page,
            &[Command::DrawRect {
                x: Pt::ZERO,
                y: Pt::ZERO,
                width: page,
                height: page,
            }],
            Pt::ZERO,
            Pt::ZERO,
            page,
            page,
            &filter,
            false,
            300,
            None,
            false,
        )
        .expect("filter raster")
        .expect("non-empty filter raster");

        assert_eq!(raster.pixel_width, 302);
        assert_eq!(raster.pixel_height, 302);
        assert_eq!(raster.x, Pt::from_milli_i64(-240));
        assert_eq!(raster.y, Pt::from_milli_i64(-240));
        let first_alpha = raster.premultiplied_rgba[3];
        let last_alpha = raster.premultiplied_rgba[raster.premultiplied_rgba.len() - 1];
        assert_eq!((first_alpha, last_alpha), (0, 0));
    }

    #[test]
    fn mixed_adjustment_and_zero_blur_shadow_retains_surface_guard() {
        let shadow = FilterDropShadowSpec {
            offset_x: Pt::from_f32(1.5),
            offset_y: Pt::from_f32(0.75),
            blur_radius: Pt::ZERO,
            color: Color::BLACK,
            opacity: 1.0,
            color_is_current_color: false,
        };
        let mut filter = PaintFilterSpec::identity();
        filter.operations = vec![
            PaintFilterOperation::Saturate(0.82),
            PaintFilterOperation::Contrast(1.08),
            PaintFilterOperation::DropShadow(shadow),
        ];
        assert!(filter_retains_css_surface_guard(&filter));
        assert!(filter_surface_guard_uses_effect_bounds(&filter));

        filter.operations = vec![PaintFilterOperation::DropShadow(shadow)];
        assert!(!filter_retains_css_surface_guard(&filter));
        filter.operations = vec![
            PaintFilterOperation::Contrast(1.08),
            PaintFilterOperation::DropShadow(FilterDropShadowSpec {
                blur_radius: Pt::from_f32(0.75),
                ..shadow
            }),
        ];
        assert!(!filter_retains_css_surface_guard(&filter));

        filter.operations = vec![
            PaintFilterOperation::Saturate(0.82),
            PaintFilterOperation::DropShadow(shadow),
        ];
        let page = Pt::from_f32(72.0);
        let raster = rasterize_filtered_form(
            page,
            page,
            page,
            page,
            &[Command::DrawRect {
                x: Pt::from_f32(24.0),
                y: Pt::from_f32(24.0),
                width: Pt::from_f32(24.0),
                height: Pt::from_f32(24.0),
            }],
            Pt::ZERO,
            Pt::ZERO,
            page,
            page,
            &filter,
            false,
            300,
            None,
            false,
        )
        .expect("mixed filter raster")
        .expect("non-empty mixed filter raster");
        assert!((100..150).contains(&raster.pixel_width));
        assert!((100..150).contains(&raster.pixel_height));
    }

    #[test]
    fn adjacent_color_filters_quantize_once_at_pipeline_boundary() {
        let mut pixel = [220, 238, 255, 255];
        apply_color_filter_operation_chain(
            &mut pixel,
            &[
                PaintFilterOperation::Saturate(0.82),
                PaintFilterOperation::Contrast(1.08),
            ],
        );
        assert_eq!(pixel, [230, 246, 255, 255]);
    }

    #[test]
    fn css_transform_origin_scales_around_center() {
        let half = Pt::from_f32(36.0);
        let edge = Pt::from_f32(72.0);
        let doc = Document {
            page_size: crate::types::Size {
                width: edge,
                height: edge,
            },
            pages: vec![crate::canvas::Page {
                commands: vec![
                    Command::SaveState,
                    Command::CssTransformOrigin {
                        x: half,
                        y: half,
                        inverse: false,
                    },
                    Command::Scale(0.5, 1.0),
                    Command::CssTransformOrigin {
                        x: half,
                        y: half,
                        inverse: true,
                    },
                    Command::SetFillColor(Color::rgb(0.0, 0.0, 1.0)),
                    Command::MoveTo {
                        x: Pt::ZERO,
                        y: Pt::ZERO,
                    },
                    Command::LineTo {
                        x: edge,
                        y: Pt::ZERO,
                    },
                    Command::LineTo { x: edge, y: edge },
                    Command::LineTo {
                        x: Pt::ZERO,
                        y: edge,
                    },
                    Command::ClosePath,
                    Command::Fill,
                    Command::RestoreState,
                ],
            }],
        };

        let pngs = document_to_png_pages(&doc, 72, None, true).unwrap();
        let img = crate::image_native::load_from_memory(&pngs[0])
            .unwrap()
            .into_rgba8();
        assert_eq!(img.get_pixel(17, 36).0, [255, 255, 255, 255]);
        assert_eq!(img.get_pixel(18, 36).0, [0, 0, 255, 255]);
        assert_eq!(img.get_pixel(53, 36).0, [0, 0, 255, 255]);
        assert_eq!(img.get_pixel(54, 36).0, [255, 255, 255, 255]);
    }

    #[test]
    fn css_transform_origin_rotates_clockwise_in_top_down_space() {
        let page_width = Pt::from_f32(288.0);
        let page_height = Pt::from_f32(144.0);
        let doc = Document {
            page_size: crate::types::Size {
                width: page_width,
                height: page_height,
            },
            pages: vec![crate::canvas::Page {
                commands: vec![
                    Command::SaveState,
                    Command::CssTransformOrigin {
                        x: Pt::from_f32(108.0),
                        y: Pt::from_f32(54.0),
                        inverse: false,
                    },
                    Command::Rotate(core::f32::consts::FRAC_PI_2),
                    Command::CssTransformOrigin {
                        x: Pt::from_f32(108.0),
                        y: Pt::from_f32(54.0),
                        inverse: true,
                    },
                    Command::SetFillColor(Color::rgb(1.0, 0.0, 0.0)),
                    Command::MoveTo {
                        x: Pt::from_f32(72.0),
                        y: Pt::from_f32(36.0),
                    },
                    Command::LineTo {
                        x: Pt::from_f32(144.0),
                        y: Pt::from_f32(36.0),
                    },
                    Command::LineTo {
                        x: Pt::from_f32(144.0),
                        y: Pt::from_f32(72.0),
                    },
                    Command::LineTo {
                        x: Pt::from_f32(72.0),
                        y: Pt::from_f32(72.0),
                    },
                    Command::ClosePath,
                    Command::Fill,
                    Command::RestoreState,
                ],
            }],
        };

        let pngs = document_to_png_pages(&doc, 72, None, true).unwrap();
        let img = crate::image_native::load_from_memory(&pngs[0])
            .unwrap()
            .into_rgba8();
        assert_eq!(img.get_pixel(100, 30).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(120, 80).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(80, 54).0, [255, 255, 255, 255]);
        assert_eq!(img.get_pixel(130, 54).0, [255, 255, 255, 255]);
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
    fn filter_drop_shadow_keeps_fractional_device_coverage() {
        let mut source = Pixmap::new(6, 4).expect("source pixmap");
        let source_index = (6 + 1) * 4;
        source.data_mut()[source_index..source_index + 4].copy_from_slice(&[255; 4]);
        let mut target = Pixmap::new(6, 4).expect("target pixmap");
        let shadow = FilterDropShadowSpec {
            offset_x: Pt::from_f32(1.0),
            offset_y: Pt::ZERO,
            blur_radius: Pt::ZERO,
            color: Color::BLACK,
            opacity: 1.0,
            color_is_current_color: false,
        };

        draw_filter_drop_shadow(
            &mut target,
            &source,
            &RasterState::default(),
            shadow,
            1.0,
            Transform::from_scale(1.25, 1.0),
        );

        let leading_alpha = target.data()[(6 + 2) * 4 + 3];
        let trailing_alpha = target.data()[(6 + 3) * 4 + 3];
        assert_eq!(leading_alpha, 191);
        assert_eq!(trailing_alpha, 64);
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
        src.put_pixel(0, 0, Rgba([255, 0, 0, 128]));
        let bytes = crate::image_native::encode_png_rgba8(src.as_bytes(), 1, 1).unwrap();
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
        let img = crate::image_native::load_from_memory(&pngs[0])
            .unwrap()
            .into_rgba8();
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
        assert!(
            candidates
                .iter()
                .any(|v| v.eq_ignore_ascii_case("arial.ttf"))
        );
        assert!(
            candidates
                .iter()
                .any(|v| v.eq_ignore_ascii_case("DejaVuSans-Bold.ttf"))
        );
    }

    #[test]
    fn system_font_candidates_normalize_subset_prefix_and_style() {
        let candidates = system_font_file_candidates("ABCDEF+Helvetica-BoldOblique");
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0], "arialbi.ttf");
    }

    #[test]
    fn effect_synthetic_bold_interpolates_browser_raster_strength_by_css_size() {
        let small = Pt::from_f32(6.75); // 9 CSS px.
        let medium = Pt::from_f32(22.5); // 30 CSS px.
        let large = Pt::from_f32(39.0); // 52 CSS px.

        let small_width = browser_effect_synthetic_bold_stroke_width(small, small.mul_ratio(1, 32));
        let medium_width =
            browser_effect_synthetic_bold_stroke_width(medium, medium.mul_ratio(1, 32));
        let large_width = browser_effect_synthetic_bold_stroke_width(large, large.mul_ratio(1, 32));

        // Pt intentionally stores millipoint-rounded author coordinates.
        assert!((small_width.to_f32() - small.to_f32() / 24.0).abs() < 6.0e-4);
        assert!((medium_width.to_f32() - 0.755_208_3).abs() < 6.0e-4);
        assert!((large_width.to_f32() - large.to_f32() / 32.0).abs() < 6.0e-4);
        assert_eq!(
            browser_effect_synthetic_bold_stroke_width(medium, Pt::ZERO),
            Pt::ZERO
        );
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
                        interpolate: true,
                        source_clip: None,
                    },
                ],
            }],
        };

        let pngs = document_to_png_pages(&doc, 72, None, true).unwrap();
        assert_eq!(pngs.len(), 1);
        let img = crate::image_native::load_from_memory(&pngs[0])
            .unwrap()
            .into_rgba8();
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
        let img = crate::image_native::load_from_memory(&pngs[0])
            .unwrap()
            .into_rgba8();
        assert!(
            has_non_white_pixel(&img),
            "expected DrawForm containing DrawImage to render non-white pixels"
        );
    }

    #[test]
    fn draw_image_preserves_top_to_bottom_source_orientation() {
        let mut src = RgbaImage::new(1, 2);
        src.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        src.put_pixel(0, 1, Rgba([0, 0, 255, 255]));
        let bytes = crate::image_native::encode_png_rgba8(src.as_bytes(), 1, 2).unwrap();
        let data_uri = format!(
            "data:image/png;base64,{}",
            crate::base64::encode_standard(bytes)
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
                    interpolate: true,
                    source_clip: None,
                }],
            }],
        };

        let pngs = document_to_png_pages(&doc, 72, None, true).unwrap();
        assert_eq!(pngs.len(), 1);
        let img = crate::image_native::load_from_memory(&pngs[0])
            .unwrap()
            .into_rgba8();
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
