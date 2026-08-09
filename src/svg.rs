use crate::Canvas;
use crate::css_native::{self, AtRuleBlock, Declaration, DeclarationBlock, Rule as CssRule};
use crate::flowable::{FilterDropShadowSpec, PaintFilterSpec};
use crate::types::{Color, Pt};
use crate::xml::{ContentNode as XmlContentNode, Document as XmlDocument, Node as XmlNode};

// Lightweight detection for SVG features that our vector compiler does not support yet.
// When present, we can optionally fall back to rasterization.
pub(crate) fn svg_needs_raster_fallback(svg_xml: &str) -> bool {
    let Ok(doc) = XmlDocument::parse(svg_xml) else {
        return false;
    };

    for node in doc.descendants().filter(|n| n.is_element()) {
        let name = node.tag_name().name();
        match name {
            // Filter/mask pipelines are raster-only for us.
            "filter" | "mask" => return true,
            // Pattern and marker paint servers are not implemented in our subset.
            "pattern" | "marker" => return true,
            _ => {}
        }

        if node.attribute("mask").is_some() || node.attribute("filter").is_some() {
            return true;
        }
    }

    false
}

#[cfg(feature = "svg_raster")]
pub(crate) fn rasterize_svg_to_data_uri(svg_xml: &str, width: Pt, height: Pt) -> Option<String> {
    let width = width.max(Pt::from_f32(1.0e-3));
    let height = height.max(Pt::from_f32(1.0e-3));
    // Use one canvas point per CSS source pixel, then rasterize one point per
    // device pixel.  This keeps tiny viewBoxes (for example 2x1) fully covered
    // instead of encoding their rows with fractional alpha.
    let raster_width = width * (4.0 / 3.0);
    let raster_height = height * (4.0 / 3.0);
    let mut canvas = Canvas::new(crate::types::Size {
        width: raster_width,
        height: raster_height,
    });
    let compiled = compile_svg(svg_xml, raster_width, raster_height);
    if compiled.is_empty() {
        return None;
    }
    render_compiled_items(&compiled, &mut canvas, Pt::ZERO, Pt::ZERO);
    let document = canvas.finish();
    let png = crate::raster::document_to_transparent_png_pages(&document, 72, None, true)
        .ok()?
        .into_iter()
        .next()?;
    let b64 = crate::base64::encode_standard(&png);
    Some(format!("data:image/png;base64,{}", b64))
}

#[cfg(not(feature = "svg_raster"))]
pub(crate) fn rasterize_svg_to_data_uri(_svg_xml: &str, _width: Pt, _height: Pt) -> Option<String> {
    None
}

// Opinionated SVG 1.1-ish subset renderer.
//
// Goal: cover the common shapes exported by design tools and used for "web-like" charts,
// while mapping cleanly to PDF primitives (paths, fills, strokes).
//
// Supported (v1):
// - <svg> root with viewBox
// - <g> grouping
// - <path d="..."> with commands: M/m, L/l, H/h, V/v, C/c, Z/z
// - <rect>, <circle>, <ellipse>, <line>, <polyline>, <polygon> (converted to paths)
// - presentation attributes + style="" for: fill, stroke, stroke-width, stroke-linecap, stroke-linejoin
// - transform="" on elements: translate, scale, rotate, matrix
//
// Still handled outside the native vector path:
// - <mask>, <filter>, <foreignObject>

#[derive(Debug, Clone, Copy)]
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

    fn translate(tx: f32, ty: f32) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    fn scale(sx: f32, sy: f32) -> Self {
        Self {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            e: 0.0,
            f: 0.0,
        }
    }

    fn rotate(deg: f32) -> Self {
        let rad = deg.to_radians();
        let (s, c) = crate::math::sin_cos(rad);
        Self {
            a: c,
            b: s,
            c: -s,
            d: c,
            e: 0.0,
            f: 0.0,
        }
    }

    fn mul(self, other: Self) -> Self {
        // [self] * [other]
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }

    fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    fn scale_factor(self) -> f32 {
        // Approx: area scale -> sqrt(|det|). Good enough for scaling stroke widths in our subset.
        let det = self.a * self.d - self.b * self.c;
        crate::math::sqrt(det.abs()).max(0.0)
    }
}

fn q(value: f32) -> f32 {
    Pt::from_f32(value).to_f32()
}

#[derive(Debug, Clone)]
struct Paint {
    color: Option<Color>,        // None => "none" (unless gradient_id is set)
    gradient_id: Option<String>, // url(#id)
}

#[derive(Debug, Clone)]
struct SvgStyle {
    fill: Paint,
    stroke: Paint,
    stroke_width: f32,
    line_cap: u8,
    line_join: u8,
    miter_limit: f32,
    dash_pattern: Vec<f32>,
    dash_offset: f32,
    fill_rule_evenodd: bool,
    fill_opacity: f32,
    stroke_opacity: f32,
    fill_shading: Option<crate::types::Shading>,
    font_family: String,
    font_size: f32,
    font_weight: u16,
    font_italic: bool,
    text_anchor: TextAnchor,
    marker_start: Option<String>,
    marker_mid: Option<String>,
    marker_end: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextAnchor {
    Start,
    Middle,
    End,
}

impl SvgStyle {
    fn default() -> Self {
        // SVG defaults: black fill, no stroke.
        Self {
            fill: Paint {
                color: Some(Color::BLACK),
                gradient_id: None,
            },
            stroke: Paint {
                color: None,
                gradient_id: None,
            },
            stroke_width: 1.0,
            line_cap: 0,
            line_join: 0,
            miter_limit: 4.0,
            dash_pattern: Vec::new(),
            dash_offset: 0.0,
            fill_rule_evenodd: false,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
            fill_shading: None,
            font_family: "Helvetica".to_string(),
            font_size: 16.0,
            font_weight: 400,
            font_italic: false,
            text_anchor: TextAnchor::Start,
            marker_start: None,
            marker_mid: None,
            marker_end: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SvgSpecificity(u16, u16, u16);

#[derive(Debug, Clone)]
struct SvgSimpleSelector {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
}

#[derive(Debug, Clone)]
struct SvgSelector {
    parts: Vec<SvgSimpleSelector>,
    specificity: SvgSpecificity,
}

#[derive(Debug, Clone)]
struct SvgCssRule {
    selector: SvgSelector,
    declarations: DeclarationBlock,
    order: usize,
}

#[derive(Debug, Clone, Default)]
struct SvgStylesheet {
    rules: Vec<SvgCssRule>,
}

#[derive(Debug, Clone)]
pub(crate) enum SvgPathSegment {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    CurveTo(f32, f32, f32, f32, f32, f32),
    Close,
}

type PathSeg = SvgPathSegment;

#[derive(Debug, Clone)]
pub(crate) struct CompiledPath {
    segs: Vec<PathSeg>,
    style: SvgStyle,
    clip: Option<(Vec<PathSeg>, bool)>,
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledImage {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    source: String,
    opacity: f32,
    transform: Option<Matrix>,
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledText {
    x: f32,
    y: f32,
    text: String,
    font_name: String,
    font_size: f32,
    fill: Color,
    opacity: f32,
    anchor: TextAnchor,
    transform: Matrix,
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledMask {
    segs: Vec<PathSeg>,
    evenodd: bool,
    paints_anything: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledGroup {
    items: Vec<CompiledItem>,
    filter: Option<PaintFilterSpec>,
    mask: Option<CompiledMask>,
}

#[derive(Debug, Clone)]
pub(crate) enum CompiledItem {
    Path(CompiledPath),
    Image(CompiledImage),
    Text(CompiledText),
    Group(CompiledGroup),
}

pub(crate) fn compile_svg(svg_xml: &str, width: Pt, height: Pt) -> Vec<CompiledItem> {
    let Ok(doc) = XmlDocument::parse(svg_xml) else {
        return Vec::new();
    };
    let Some(root) = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case("svg"))
    else {
        return Vec::new();
    };

    let stylesheet = extract_svg_stylesheet(&doc);
    let gradients = extract_gradients(&doc, &stylesheet);
    let id_map = build_id_map(&doc);
    let view_box = parse_viewbox(
        root.attribute("viewBox")
            .or_else(|| root.attribute("viewbox")),
    );
    let viewport_width = width.to_f32();
    let viewport_height = height.to_f32();
    let viewport = if view_box.is_some() {
        viewbox_to_viewport_matrix_with_aspect(
            view_box,
            viewport_width,
            viewport_height,
            root.attribute("preserveAspectRatio")
                .or_else(|| root.attribute("preserveaspectratio")),
        )
    } else {
        let intrinsic_width = root.attribute("width").and_then(parse_number);
        let intrinsic_height = root.attribute("height").and_then(parse_number);
        match (intrinsic_width, intrinsic_height) {
            (Some(intrinsic_width), Some(intrinsic_height))
                if intrinsic_width > 0.0 && intrinsic_height > 0.0 =>
            {
                Matrix::scale(
                    viewport_width / intrinsic_width,
                    viewport_height / intrinsic_height,
                )
            }
            _ => Matrix::identity(),
        }
    };
    let base = viewport;

    let style = SvgStyle::default();
    let mut out = Vec::new();
    let mut resolving_ids = Vec::new();
    compile_element(
        &mut out,
        root,
        base,
        &style,
        &gradients,
        &id_map,
        &stylesheet,
        &mut resolving_ids,
    );
    out
}

pub(crate) fn render_compiled_items(items: &[CompiledItem], canvas: &mut Canvas, x: Pt, y: Pt) {
    for it in items {
        match it {
            CompiledItem::Path(path) => draw_compiled_path(canvas, path, x, y),
            CompiledItem::Image(img) => {
                canvas.save_state();
                canvas.set_opacity(img.opacity, img.opacity);
                if let Some(transform) = img.transform {
                    concat_top_left_matrix(canvas, transform, x, y);
                    canvas.draw_image(
                        Pt::from_f32(img.x),
                        Pt::from_f32(img.y),
                        Pt::from_f32(img.width),
                        Pt::from_f32(img.height),
                        img.source.clone(),
                    );
                } else {
                    canvas.draw_image(
                        x + Pt::from_f32(img.x),
                        y + Pt::from_f32(img.y),
                        Pt::from_f32(img.width),
                        Pt::from_f32(img.height),
                        img.source.clone(),
                    );
                }
                canvas.restore_state();
            }
            CompiledItem::Text(text) => draw_compiled_text(canvas, text, x, y),
            CompiledItem::Group(group) => draw_compiled_group(canvas, group, x, y),
        }
    }
}

fn emit_path(canvas: &mut Canvas, segs: &[PathSeg], x_off: Pt, y_off: Pt) {
    for seg in segs {
        match *seg {
            PathSeg::MoveTo(px, py) => {
                canvas.move_to(x_off + Pt::from_f32(px), y_off + Pt::from_f32(py))
            }
            PathSeg::LineTo(px, py) => {
                canvas.line_to(x_off + Pt::from_f32(px), y_off + Pt::from_f32(py))
            }
            PathSeg::CurveTo(x1, y1, x2, y2, x3, y3) => {
                canvas.curve_to(
                    x_off + Pt::from_f32(x1),
                    y_off + Pt::from_f32(y1),
                    x_off + Pt::from_f32(x2),
                    y_off + Pt::from_f32(y2),
                    x_off + Pt::from_f32(x3),
                    y_off + Pt::from_f32(y3),
                );
            }
            PathSeg::Close => canvas.close_path(),
        }
    }
}

fn draw_compiled_group(canvas: &mut Canvas, group: &CompiledGroup, x: Pt, y: Pt) {
    if group
        .mask
        .as_ref()
        .is_some_and(|mask| !mask.paints_anything)
    {
        return;
    }
    canvas.save_state();
    if let Some(mask) = &group.mask {
        emit_path(canvas, &mask.segs, x, y);
        canvas.clip_path(mask.evenodd);
    }

    if let Some(filter) = &group.filter {
        let size = canvas.page_size();
        let mut form_canvas = Canvas::new(size);
        render_compiled_items(&group.items, &mut form_canvas, x, y);
        let document = form_canvas.finish();
        let commands = document
            .pages
            .first()
            .map(|page| page.commands.clone())
            .unwrap_or_default();
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        format!("{group:?}").hash(&mut hasher);
        let resource_id = format!("svg-effect:{:016x}", hasher.finish());
        canvas.define_isolated_form(resource_id.clone(), size.width, size.height, commands);
        canvas.draw_filtered_form(
            Pt::ZERO,
            Pt::ZERO,
            size.width,
            size.height,
            resource_id,
            filter.clone(),
        );
    } else {
        render_compiled_items(&group.items, canvas, x, y);
    }
    canvas.restore_state();
}

fn concat_top_left_matrix(canvas: &mut Canvas, transform: Matrix, x: Pt, y: Pt) {
    // Canvas owns top-down transforms. PDF serializers perform the one required
    // axis conjugation at the backend boundary. This command is absolute in
    // page space, so compensate for the page-height origin term that a relative
    // canvas matrix does not otherwise carry.
    let absolute = Matrix::translate(x.to_f32(), y.to_f32()).mul(transform);
    let page_height = canvas.page_size().height.to_f32();
    canvas.concat_matrix(
        absolute.a,
        absolute.b,
        absolute.c,
        absolute.d,
        Pt::from_f32(absolute.e + absolute.c * page_height),
        Pt::from_f32(absolute.f - page_height * (1.0 - absolute.d)),
    );
}

fn draw_compiled_text(canvas: &mut Canvas, text: &CompiledText, x: Pt, y: Pt) {
    if text.text.is_empty() || text.font_size <= 0.0 || text.opacity <= 0.0 {
        return;
    }

    let approximate_width = text.font_size * 0.6 * text.text.chars().count() as f32;
    let anchor_offset = match text.anchor {
        TextAnchor::Start => 0.0,
        TextAnchor::Middle => approximate_width * 0.5,
        TextAnchor::End => approximate_width,
    };

    canvas.save_state();
    concat_top_left_matrix(canvas, text.transform, x, y);
    canvas.set_fill_color(text.fill);
    canvas.set_opacity(text.opacity, text.opacity);
    canvas.set_font_name(&text.font_name);
    canvas.set_font_size(Pt::from_f32(text.font_size));
    canvas.draw_string(
        Pt::from_f32(text.x - anchor_offset),
        Pt::from_f32(text.y - text.font_size),
        text.text.clone(),
    );
    canvas.restore_state();
}

#[cfg(test)]
pub(crate) fn render_svg_to_canvas(
    svg_xml: &str,
    canvas: &mut Canvas,
    x: Pt,
    y: Pt,
    width: Pt,
    height: Pt,
) {
    let compiled = compile_svg(svg_xml, width, height);
    render_compiled_items(&compiled, canvas, x, y);
}

fn compile_element(
    out: &mut Vec<CompiledItem>,
    node: XmlNode<'_>,
    ctm: Matrix,
    style: &SvgStyle,
    gradients: &std::collections::HashMap<String, GradientDef>,
    id_map: &std::collections::HashMap<String, XmlNode<'_>>,
    stylesheet: &SvgStylesheet,
    resolving_ids: &mut Vec<String>,
) {
    let mut effect_ctm = ctm;
    if let Some(transform) = node.attribute("transform") {
        effect_ctm = effect_ctm.mul(parse_transform(transform));
    }
    let filter = compile_filter_for_node(node, effect_ctm, id_map);
    let mask = compile_mask_for_node(node, effect_ctm, id_map, stylesheet);
    if filter.is_none() && mask.is_none() {
        compile_element_inner(
            out,
            node,
            ctm,
            style,
            gradients,
            id_map,
            stylesheet,
            resolving_ids,
        );
        return;
    }

    let mut items = Vec::new();
    compile_element_inner(
        &mut items,
        node,
        ctm,
        style,
        gradients,
        id_map,
        stylesheet,
        resolving_ids,
    );
    if !items.is_empty() {
        out.push(CompiledItem::Group(CompiledGroup {
            items,
            filter,
            mask,
        }));
    }
}

fn compile_element_inner(
    out: &mut Vec<CompiledItem>,
    node: XmlNode<'_>,
    ctm: Matrix,
    style: &SvgStyle,
    gradients: &std::collections::HashMap<String, GradientDef>,
    id_map: &std::collections::HashMap<String, XmlNode<'_>>,
    stylesheet: &SvgStylesheet,
    resolving_ids: &mut Vec<String>,
) {
    if !node.is_element() {
        return;
    }

    let mut local_style = style.clone();
    apply_presentation_and_style(node, stylesheet, &mut local_style);

    let mut local_ctm = ctm;
    if let Some(transform) = node.attribute("transform") {
        local_ctm = local_ctm.mul(parse_transform(transform));
    }

    let tag = node.tag_name().name();
    match tag {
        "defs" => {
            // Definitions should not render directly. <use> resolves these by id.
        }
        "g" | "svg" => {
            for child in node.children().filter(|n| n.is_element()) {
                compile_element(
                    out,
                    child,
                    local_ctm,
                    &local_style,
                    gradients,
                    id_map,
                    stylesheet,
                    resolving_ids,
                );
            }
        }
        "use" => {
            if let Some(id) = href_id(node) {
                if resolving_ids.iter().any(|active| active == &id) {
                    return;
                }
                if let Some(target) = id_map.get(&id).copied() {
                    let x = parse_number(node.attribute("x").unwrap_or("0")).unwrap_or(0.0);
                    let y = parse_number(node.attribute("y").unwrap_or("0")).unwrap_or(0.0);
                    let use_ctm = local_ctm.mul(Matrix::translate(x, y));
                    resolving_ids.push(id);
                    if target.tag_name().name().eq_ignore_ascii_case("symbol") {
                        compile_symbol_use(
                            out,
                            node,
                            target,
                            use_ctm,
                            &local_style,
                            gradients,
                            id_map,
                            stylesheet,
                            resolving_ids,
                        );
                    } else {
                        compile_element(
                            out,
                            target,
                            use_ctm,
                            &local_style,
                            gradients,
                            id_map,
                            stylesheet,
                            resolving_ids,
                        );
                    }
                    resolving_ids.pop();
                }
            }
        }
        "symbol" => {
            // Symbols establish a reusable viewport and only paint through <use>.
        }
        "text" => {
            compile_text_element(out, node, local_ctm, &local_style, stylesheet);
        }
        "path" => {
            if let Some(d) = node.attribute("d") {
                let segs = parse_path_data(d);
                compile_graphic_path(
                    out,
                    node,
                    &segs,
                    &local_style,
                    local_ctm,
                    gradients,
                    id_map,
                    stylesheet,
                    resolving_ids,
                );
            }
        }
        "rect" => {
            if let Some(segs) = rect_to_path(node) {
                compile_graphic_path(
                    out,
                    node,
                    &segs,
                    &local_style,
                    local_ctm,
                    gradients,
                    id_map,
                    stylesheet,
                    resolving_ids,
                );
            }
        }
        "circle" => {
            if let Some(segs) = circle_to_path(node) {
                compile_graphic_path(
                    out,
                    node,
                    &segs,
                    &local_style,
                    local_ctm,
                    gradients,
                    id_map,
                    stylesheet,
                    resolving_ids,
                );
            }
        }
        "ellipse" => {
            if let Some(segs) = ellipse_to_path(node) {
                compile_graphic_path(
                    out,
                    node,
                    &segs,
                    &local_style,
                    local_ctm,
                    gradients,
                    id_map,
                    stylesheet,
                    resolving_ids,
                );
            }
        }
        "line" => {
            if let Some(segs) = line_to_path(node) {
                compile_graphic_path(
                    out,
                    node,
                    &segs,
                    &local_style,
                    local_ctm,
                    gradients,
                    id_map,
                    stylesheet,
                    resolving_ids,
                );
            }
        }
        "polyline" => {
            if let Some(segs) = poly_points_to_path(node, false) {
                compile_graphic_path(
                    out,
                    node,
                    &segs,
                    &local_style,
                    local_ctm,
                    gradients,
                    id_map,
                    stylesheet,
                    resolving_ids,
                );
            }
        }
        "polygon" => {
            if let Some(segs) = poly_points_to_path(node, true) {
                compile_graphic_path(
                    out,
                    node,
                    &segs,
                    &local_style,
                    local_ctm,
                    gradients,
                    id_map,
                    stylesheet,
                    resolving_ids,
                );
            }
        }
        "image" => {
            // Raster image inside SVG (PNG/JPEG/data URI).
            let href = node
                .attribute("href")
                .or_else(|| node.attribute("xlink:href"))
                .unwrap_or("")
                .to_string();
            if href.is_empty() {
                return;
            }
            let x = parse_number(node.attribute("x").unwrap_or("0")).unwrap_or(0.0);
            let y = parse_number(node.attribute("y").unwrap_or("0")).unwrap_or(0.0);
            let w = parse_number(node.attribute("width").unwrap_or("0")).unwrap_or(0.0);
            let h = parse_number(node.attribute("height").unwrap_or("0")).unwrap_or(0.0);
            if w <= 0.0 || h <= 0.0 {
                return;
            }

            if local_ctm.b.abs() > 1e-4 || local_ctm.c.abs() > 1e-4 {
                out.push(CompiledItem::Image(CompiledImage {
                    x: q(x),
                    y: q(y),
                    width: q(w),
                    height: q(h),
                    source: href,
                    opacity: local_style.fill_opacity.clamp(0.0, 1.0),
                    transform: Some(local_ctm),
                }));
                return;
            }

            let (x0, y0) = local_ctm.apply(x, y);
            let (x1, y1) = local_ctm.apply(x + w, y + h);
            let mut ix = x0;
            let mut iy = y0;
            let mut iw = x1 - x0;
            let mut ih = y1 - y0;
            if iw < 0.0 {
                ix += iw;
                iw = -iw;
            }
            if ih < 0.0 {
                iy += ih;
                ih = -ih;
            }
            if iw <= 0.0 || ih <= 0.0 {
                return;
            }
            ix = q(ix);
            iy = q(iy);
            iw = q(iw);
            ih = q(ih);
            out.push(CompiledItem::Image(CompiledImage {
                x: ix,
                y: iy,
                width: iw,
                height: ih,
                source: href,
                opacity: local_style.fill_opacity.clamp(0.0, 1.0),
                transform: None,
            }));
        }
        _ => {
            // Ignore unknown tags in our subset.
        }
    }
}

fn compile_filter_for_node(
    node: XmlNode<'_>,
    ctm: Matrix,
    id_map: &std::collections::HashMap<String, XmlNode<'_>>,
) -> Option<PaintFilterSpec> {
    let id = node.attribute("filter").and_then(parse_url_ref)?;
    let filter_node = id_map.get(&id).copied()?;
    if !filter_node.tag_name().name().eq_ignore_ascii_case("filter") {
        return None;
    }
    let mut filter = PaintFilterSpec::identity();
    compile_filter_subtree(filter_node, ctm.scale_factor(), &mut filter);
    Some(filter)
}

fn compile_filter_subtree(node: XmlNode<'_>, scale: f32, filter: &mut PaintFilterSpec) {
    for child in node.children() {
        let tag = child.tag_name().name();
        if tag.eq_ignore_ascii_case("feGaussianBlur") {
            let deviations = parse_number_list(
                child
                    .attribute("stdDeviation")
                    .or_else(|| child.attribute("stddeviation"))
                    .unwrap_or("0"),
            );
            let sigma = deviations.iter().copied().fold(0.0_f32, f32::max).max(0.0) * scale;
            filter.blur_radius = filter.blur_radius.max(Pt::from_f32(sigma));
        } else if tag.eq_ignore_ascii_case("feDropShadow") {
            let sigma = parse_number_list(
                child
                    .attribute("stdDeviation")
                    .or_else(|| child.attribute("stddeviation"))
                    .unwrap_or("0"),
            )
            .into_iter()
            .fold(0.0_f32, f32::max)
            .max(0.0)
                * scale;
            let offset_x = child.attribute("dx").and_then(parse_number).unwrap_or(2.0) * scale;
            let offset_y = child.attribute("dy").and_then(parse_number).unwrap_or(2.0) * scale;
            let color = child
                .attribute("flood-color")
                .and_then(parse_color)
                .unwrap_or(Color::BLACK);
            let opacity = child
                .attribute("flood-opacity")
                .and_then(parse_number)
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            filter.drop_shadows.push(FilterDropShadowSpec {
                offset_x: Pt::from_f32(offset_x),
                offset_y: Pt::from_f32(offset_y),
                blur_radius: Pt::from_f32(sigma),
                color,
                opacity,
                color_is_current_color: false,
            });
        } else if tag.eq_ignore_ascii_case("feColorMatrix") {
            let values = parse_number_list(child.attribute("values").unwrap_or(""));
            match child.attribute("type").unwrap_or("matrix") {
                "saturate" => {
                    if let Some(value) = values.first() {
                        filter.saturate *= value.max(0.0);
                    }
                }
                value if value.eq_ignore_ascii_case("hueRotate") => {
                    if let Some(value) = values.first() {
                        filter.hue_rotate += value.to_radians();
                    }
                }
                _ => {}
            }
        } else if tag.eq_ignore_ascii_case("filter") || tag.eq_ignore_ascii_case("g") {
            compile_filter_subtree(child, scale, filter);
        }
    }
}

fn compile_mask_for_node(
    node: XmlNode<'_>,
    ctm: Matrix,
    id_map: &std::collections::HashMap<String, XmlNode<'_>>,
    stylesheet: &SvgStylesheet,
) -> Option<CompiledMask> {
    let id = node.attribute("mask").and_then(parse_url_ref)?;
    let mask_node = id_map.get(&id).copied()?;
    if !mask_node.tag_name().name().eq_ignore_ascii_case("mask") {
        return None;
    }
    let mut positive = Vec::new();
    let mut negative = Vec::new();
    let style = SvgStyle::default();
    compile_mask_subtree(
        mask_node,
        ctm,
        &style,
        id_map,
        stylesheet,
        &mut positive,
        &mut negative,
        0,
    );
    let paints_anything = !positive.is_empty();
    positive.extend(negative);
    Some(CompiledMask {
        segs: positive,
        evenodd: true,
        paints_anything,
    })
}

#[allow(clippy::too_many_arguments)]
fn compile_mask_subtree(
    node: XmlNode<'_>,
    ctm: Matrix,
    inherited_style: &SvgStyle,
    id_map: &std::collections::HashMap<String, XmlNode<'_>>,
    stylesheet: &SvgStylesheet,
    positive: &mut Vec<PathSeg>,
    negative: &mut Vec<PathSeg>,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    let mut style = inherited_style.clone();
    apply_presentation_and_style(node, stylesheet, &mut style);
    let mut local_ctm = ctm;
    if let Some(transform) = node.attribute("transform") {
        local_ctm = local_ctm.mul(parse_transform(transform));
    }
    let tag = node.tag_name().name();
    if matches!(tag, "mask" | "g" | "svg" | "defs") {
        for child in node.children() {
            compile_mask_subtree(
                child,
                local_ctm,
                &style,
                id_map,
                stylesheet,
                positive,
                negative,
                depth + 1,
            );
        }
        return;
    }
    if tag == "use" {
        if let Some(id) = href_id(node) {
            if let Some(target) = id_map.get(&id).copied() {
                let x = parse_number(node.attribute("x").unwrap_or("0")).unwrap_or(0.0);
                let y = parse_number(node.attribute("y").unwrap_or("0")).unwrap_or(0.0);
                compile_mask_subtree(
                    target,
                    local_ctm.mul(Matrix::translate(x, y)),
                    &style,
                    id_map,
                    stylesheet,
                    positive,
                    negative,
                    depth + 1,
                );
            }
        }
        return;
    }

    let raw = match tag {
        "path" => node.attribute("d").map(parse_path_data),
        "rect" => rect_to_path(node),
        "circle" => circle_to_path(node),
        "ellipse" => ellipse_to_path(node),
        "line" => line_to_path(node),
        "polyline" => poly_points_to_path(node, false),
        "polygon" => poly_points_to_path(node, true),
        _ => None,
    };
    let Some(raw) = raw else { return };
    let transformed = transform_path_segs(&raw, local_ctm);
    let luminance = style
        .fill
        .color
        .map(|color| 0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b)
        .unwrap_or_else(|| {
            if style.fill.gradient_id.is_some() {
                1.0
            } else {
                0.0
            }
        })
        * style.fill_opacity;
    if luminance >= 0.5 {
        positive.extend(transformed);
    } else {
        negative.extend(transformed);
    }
}

#[allow(clippy::too_many_arguments)]
fn compile_symbol_use(
    out: &mut Vec<CompiledItem>,
    use_node: XmlNode<'_>,
    symbol: XmlNode<'_>,
    use_ctm: Matrix,
    inherited_style: &SvgStyle,
    gradients: &std::collections::HashMap<String, GradientDef>,
    id_map: &std::collections::HashMap<String, XmlNode<'_>>,
    stylesheet: &SvgStylesheet,
    resolving_ids: &mut Vec<String>,
) {
    let view_box = parse_viewbox(
        symbol
            .attribute("viewBox")
            .or_else(|| symbol.attribute("viewbox")),
    );
    let view_box_width = view_box.map(|(_, _, width, _)| width);
    let view_box_height = view_box.map(|(_, _, _, height)| height);
    let width = use_node
        .attribute("width")
        .and_then(parse_number)
        .or_else(|| symbol.attribute("width").and_then(parse_number))
        .or(view_box_width)
        .unwrap_or(0.0);
    let height = use_node
        .attribute("height")
        .and_then(parse_number)
        .or_else(|| symbol.attribute("height").and_then(parse_number))
        .or(view_box_height)
        .unwrap_or(0.0);
    if width <= 0.0 || height <= 0.0 {
        return;
    }

    let mut symbol_style = inherited_style.clone();
    apply_presentation_and_style(symbol, stylesheet, &mut symbol_style);
    let mut symbol_ctm = use_ctm.mul(viewbox_to_viewport_matrix(view_box, width, height));
    if let Some(transform) = symbol.attribute("transform") {
        symbol_ctm = symbol_ctm.mul(parse_transform(transform));
    }

    for child in symbol.children() {
        compile_element(
            out,
            child,
            symbol_ctm,
            &symbol_style,
            gradients,
            id_map,
            stylesheet,
            resolving_ids,
        );
    }
}

fn compile_text_element(
    out: &mut Vec<CompiledItem>,
    node: XmlNode<'_>,
    ctm: Matrix,
    style: &SvgStyle,
    stylesheet: &SvgStylesheet,
) {
    let mut cursor = TextCursor {
        x: first_length(node.attribute("x")).unwrap_or(0.0),
        y: first_length(node.attribute("y")).unwrap_or(0.0),
    };
    cursor.x += first_length(node.attribute("dx")).unwrap_or(0.0);
    cursor.y += first_length(node.attribute("dy")).unwrap_or(0.0);
    apply_text_anchor_offset(node, style, stylesheet, &mut cursor, true);
    compile_text_content(out, node, ctm, style, stylesheet, &mut cursor);
}

#[derive(Debug, Clone, Copy)]
struct TextCursor {
    x: f32,
    y: f32,
}

fn compile_text_content(
    out: &mut Vec<CompiledItem>,
    node: XmlNode<'_>,
    ctm: Matrix,
    style: &SvgStyle,
    stylesheet: &SvgStylesheet,
    cursor: &mut TextCursor,
) {
    for content in node.content() {
        match content {
            XmlContentNode::Text(raw) => {
                let text = collapse_svg_text(raw);
                if text.trim().is_empty() {
                    continue;
                }
                let Some(fill) = style.fill.color else {
                    cursor.x += approximate_text_width(&text, style.font_size);
                    continue;
                };
                out.push(CompiledItem::Text(CompiledText {
                    x: q(cursor.x),
                    y: q(cursor.y),
                    text: text.clone(),
                    font_name: svg_font_name(style),
                    font_size: q(style.font_size.max(0.0)),
                    fill,
                    opacity: style.fill_opacity.clamp(0.0, 1.0),
                    anchor: TextAnchor::Start,
                    transform: ctm,
                }));
                cursor.x += approximate_text_width(&text, style.font_size);
            }
            XmlContentNode::Element(child) => {
                let tag = child.tag_name().name();
                if !tag.eq_ignore_ascii_case("tspan") && !tag.eq_ignore_ascii_case("a") {
                    continue;
                }
                let mut child_style = style.clone();
                apply_presentation_and_style(child, stylesheet, &mut child_style);
                let mut child_ctm = ctm;
                if let Some(transform) = child.attribute("transform") {
                    child_ctm = child_ctm.mul(parse_transform(transform));
                }
                let resets_chunk = child.attribute("x").is_some();
                if let Some(value) = first_length(child.attribute("x")) {
                    cursor.x = value;
                }
                if let Some(value) = first_length(child.attribute("y")) {
                    cursor.y = value;
                }
                cursor.x += first_length(child.attribute("dx")).unwrap_or(0.0);
                cursor.y += first_length(child.attribute("dy")).unwrap_or(0.0);
                apply_text_anchor_offset(child, &child_style, stylesheet, cursor, resets_chunk);
                compile_text_content(out, child, child_ctm, &child_style, stylesheet, cursor);
            }
        }
    }
}

fn apply_text_anchor_offset(
    node: XmlNode<'_>,
    style: &SvgStyle,
    stylesheet: &SvgStylesheet,
    cursor: &mut TextCursor,
    establishes_chunk: bool,
) {
    if !establishes_chunk || matches!(style.text_anchor, TextAnchor::Start) {
        return;
    }
    let width = estimate_text_content_width(node, style, stylesheet);
    cursor.x -= match style.text_anchor {
        TextAnchor::Start => 0.0,
        TextAnchor::Middle => width * 0.5,
        TextAnchor::End => width,
    };
}

fn estimate_text_content_width(
    node: XmlNode<'_>,
    style: &SvgStyle,
    stylesheet: &SvgStylesheet,
) -> f32 {
    let mut width = 0.0;
    for content in node.content() {
        match content {
            XmlContentNode::Text(raw) => {
                width += approximate_text_width(&collapse_svg_text(raw), style.font_size);
            }
            XmlContentNode::Element(child)
                if child.tag_name().name().eq_ignore_ascii_case("tspan")
                    || child.tag_name().name().eq_ignore_ascii_case("a") =>
            {
                let mut child_style = style.clone();
                apply_presentation_and_style(child, stylesheet, &mut child_style);
                width += estimate_text_content_width(child, &child_style, stylesheet);
            }
            XmlContentNode::Element(_) => {}
        }
    }
    width
}

fn collapse_svg_text(raw: &str) -> String {
    let mut out = String::new();
    let mut whitespace = false;
    for ch in raw.chars() {
        if ch.is_whitespace() {
            whitespace = true;
        } else {
            if whitespace && !out.is_empty() {
                out.push(' ');
            }
            whitespace = false;
            out.push(ch);
        }
    }
    if whitespace && !out.is_empty() {
        out.push(' ');
    }
    out
}

fn approximate_text_width(text: &str, font_size: f32) -> f32 {
    text.chars().count() as f32 * font_size.max(0.0) * 0.6
}

fn first_length(value: Option<&str>) -> Option<f32> {
    value?
        .split(|ch: char| ch.is_whitespace() || ch == ',')
        .find(|part| !part.is_empty())
        .and_then(parse_number)
}

fn svg_font_name(style: &SvgStyle) -> String {
    let family = style
        .font_family
        .split(',')
        .next()
        .unwrap_or("Helvetica")
        .trim()
        .trim_matches('"')
        .trim_matches('\'');
    let compact = family
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    let (base, base14) = match compact.as_str() {
        "arial" | "arialmt" | "helvetica" | "helveticaneue" | "sansserif" | "systemui" => {
            ("Helvetica".to_string(), true)
        }
        "times" | "timesroman" | "timesnewroman" | "serif" => ("Times".to_string(), true),
        "courier" | "couriernew" | "monospace" => ("Courier".to_string(), true),
        _ if family.is_empty() => ("Helvetica".to_string(), true),
        _ => (family.to_string(), false),
    };
    let bold = style.font_weight >= 600;
    match (base14, base.as_str(), bold, style.font_italic) {
        (true, "Times", false, false) => "Times-Roman".to_string(),
        (true, "Times", true, false) => "Times-Bold".to_string(),
        (true, "Times", false, true) => "Times-Italic".to_string(),
        (true, "Times", true, true) => "Times-BoldItalic".to_string(),
        (true, _, false, false) => base,
        (true, _, true, false) => format!("{base}-Bold"),
        (true, _, false, true) => format!("{base}-Oblique"),
        (true, _, true, true) => format!("{base}-BoldOblique"),
        (false, _, false, false) => base,
        (false, _, true, false) => format!("{base} Bold"),
        (false, _, false, true) => format!("{base} Italic"),
        (false, _, true, true) => format!("{base} Bold Italic"),
    }
}

fn build_id_map(doc: &XmlDocument) -> std::collections::HashMap<String, XmlNode<'_>> {
    let mut out = std::collections::HashMap::new();
    for node in doc.descendants().filter(|n| n.is_element()) {
        if let Some(id) = node.attribute("id") {
            // First wins (matches common SVG authoring expectations).
            out.entry(id.to_string()).or_insert(node);
        }
    }
    out
}

fn href_id(node: XmlNode<'_>) -> Option<String> {
    // Prefer plain href, then xlink:href.
    let raw = node
        .attribute("href")
        .or_else(|| node.attribute("xlink:href"))?;
    let raw = raw.trim().trim_matches('"').trim_matches('\'');
    let id = raw.strip_prefix('#')?;
    if id.is_empty() {
        return None;
    }
    Some(id.to_string())
}

#[allow(clippy::too_many_arguments)]
fn compile_graphic_path(
    out: &mut Vec<CompiledItem>,
    node: XmlNode<'_>,
    segs: &[PathSeg],
    style: &SvgStyle,
    ctm: Matrix,
    gradients: &std::collections::HashMap<String, GradientDef>,
    id_map: &std::collections::HashMap<String, XmlNode<'_>>,
    stylesheet: &SvgStylesheet,
    resolving_ids: &mut Vec<String>,
) {
    let clip = compile_clip_for_node(node, ctm, id_map);
    let pattern = style
        .fill
        .gradient_id
        .as_deref()
        .and_then(|id| id_map.get(id).copied().map(|pattern| (id, pattern)))
        .filter(|(_, pattern)| pattern.tag_name().name().eq_ignore_ascii_case("pattern"));
    if let Some((id, pattern)) = pattern {
        compile_pattern_fill(
            out,
            id,
            pattern,
            segs,
            style,
            ctm,
            gradients,
            id_map,
            stylesheet,
            resolving_ids,
        );
        let mut stroke_only = style.clone();
        stroke_only.fill.color = None;
        stroke_only.fill.gradient_id = None;
        push_compiled_path(out, segs, &stroke_only, ctm, gradients, clip);
    } else {
        push_compiled_path(out, segs, style, ctm, gradients, clip);
    }
    compile_path_markers(
        out,
        segs,
        style,
        ctm,
        gradients,
        id_map,
        stylesheet,
        resolving_ids,
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatternUnits {
    ObjectBoundingBox,
    UserSpaceOnUse,
}

fn resolve_pattern_coordinate(
    raw: Option<&str>,
    origin: f32,
    extent: f32,
    units: PatternUnits,
    is_extent: bool,
) -> f32 {
    let Some(raw) = raw else {
        return if is_extent { 0.0 } else { origin };
    };
    let raw = raw.trim();
    if let Some(percent) = raw.strip_suffix('%') {
        let fraction = percent.trim().parse::<f32>().unwrap_or(0.0) / 100.0;
        return if is_extent {
            extent * fraction
        } else {
            origin + extent * fraction
        };
    }
    let value = parse_number(raw).unwrap_or(0.0);
    match units {
        PatternUnits::ObjectBoundingBox => {
            if is_extent {
                extent * value
            } else {
                origin + extent * value
            }
        }
        PatternUnits::UserSpaceOnUse => value,
    }
}

#[allow(clippy::too_many_arguments)]
fn compile_pattern_fill(
    out: &mut Vec<CompiledItem>,
    id: &str,
    pattern: XmlNode<'_>,
    target_segs: &[PathSeg],
    target_style: &SvgStyle,
    ctm: Matrix,
    gradients: &std::collections::HashMap<String, GradientDef>,
    id_map: &std::collections::HashMap<String, XmlNode<'_>>,
    stylesheet: &SvgStylesheet,
    resolving_ids: &mut Vec<String>,
) {
    if resolving_ids.iter().any(|active| active == id) {
        return;
    }
    let Some((bbox_x, bbox_y, bbox_width, bbox_height)) = bbox_of_segs(target_segs) else {
        return;
    };
    if bbox_width <= 0.0 || bbox_height <= 0.0 {
        return;
    }
    let units = if pattern
        .attribute("patternUnits")
        .or_else(|| pattern.attribute("patternunits"))
        .is_some_and(|value| value.eq_ignore_ascii_case("userSpaceOnUse"))
    {
        PatternUnits::UserSpaceOnUse
    } else {
        PatternUnits::ObjectBoundingBox
    };
    let tile_origin_x =
        resolve_pattern_coordinate(pattern.attribute("x"), bbox_x, bbox_width, units, false);
    let tile_origin_y =
        resolve_pattern_coordinate(pattern.attribute("y"), bbox_y, bbox_height, units, false);
    let tile_width =
        resolve_pattern_coordinate(pattern.attribute("width"), bbox_x, bbox_width, units, true);
    let tile_height = resolve_pattern_coordinate(
        pattern.attribute("height"),
        bbox_y,
        bbox_height,
        units,
        true,
    );
    if tile_width <= 0.0
        || tile_height <= 0.0
        || !tile_width.is_finite()
        || !tile_height.is_finite()
    {
        return;
    }
    let first_x =
        tile_origin_x + crate::math::floor((bbox_x - tile_origin_x) / tile_width) * tile_width;
    let first_y =
        tile_origin_y + crate::math::floor((bbox_y - tile_origin_y) / tile_height) * tile_height;
    let columns =
        (crate::math::ceil((bbox_x + bbox_width - first_x) / tile_width) as usize + 1).min(256);
    let rows =
        (crate::math::ceil((bbox_y + bbox_height - first_y) / tile_height) as usize + 1).min(256);
    if columns.saturating_mul(rows) > 16_384 {
        return;
    }

    let view_box = parse_viewbox(
        pattern
            .attribute("viewBox")
            .or_else(|| pattern.attribute("viewbox")),
    );
    let pattern_transform = pattern
        .attribute("patternTransform")
        .or_else(|| pattern.attribute("patterntransform"))
        .map(parse_transform)
        .unwrap_or_else(Matrix::identity);
    let content_units_object_bbox = pattern
        .attribute("patternContentUnits")
        .or_else(|| pattern.attribute("patterncontentunits"))
        .is_some_and(|value| value.eq_ignore_ascii_case("objectBoundingBox"));
    let mut pattern_style = SvgStyle::default();
    apply_presentation_and_style(pattern, stylesheet, &mut pattern_style);
    pattern_style.marker_start = None;
    pattern_style.marker_mid = None;
    pattern_style.marker_end = None;

    let mut items = Vec::new();
    resolving_ids.push(id.to_string());
    for row in 0..rows {
        for column in 0..columns {
            let tile_x = first_x + column as f32 * tile_width;
            let tile_y = first_y + row as f32 * tile_height;
            let content_matrix = if let Some(view_box) = view_box {
                viewbox_to_viewport_matrix(Some(view_box), tile_width, tile_height)
            } else if content_units_object_bbox {
                Matrix::scale(bbox_width, bbox_height)
            } else {
                Matrix::identity()
            };
            let tile_ctm = ctm
                .mul(pattern_transform)
                .mul(Matrix::translate(tile_x, tile_y))
                .mul(content_matrix);
            for child in pattern.children() {
                compile_element(
                    &mut items,
                    child,
                    tile_ctm,
                    &pattern_style,
                    gradients,
                    id_map,
                    stylesheet,
                    resolving_ids,
                );
            }
        }
    }
    resolving_ids.pop();
    if items.is_empty() {
        return;
    }
    out.push(CompiledItem::Group(CompiledGroup {
        items,
        filter: None,
        mask: Some(CompiledMask {
            segs: transform_path_segs(target_segs, ctm),
            evenodd: target_style.fill_rule_evenodd,
            paints_anything: true,
        }),
    }));
}

#[derive(Debug, Clone, Copy)]
struct MarkerVertex {
    x: f32,
    y: f32,
    incoming: Option<f32>,
    outgoing: Option<f32>,
}

fn marker_vertices(segs: &[PathSeg]) -> Vec<MarkerVertex> {
    let mut vertices = Vec::new();
    let mut current = (0.0_f32, 0.0_f32);
    let mut subpath_start = None;
    for seg in segs {
        match *seg {
            PathSeg::MoveTo(x, y) => {
                current = (x, y);
                subpath_start = Some((x, y));
                vertices.push(MarkerVertex {
                    x,
                    y,
                    incoming: None,
                    outgoing: None,
                });
            }
            PathSeg::LineTo(x, y) => {
                let angle = crate::math::atan2(y - current.1, x - current.0);
                if let Some(vertex) = vertices.last_mut() {
                    vertex.outgoing = Some(angle);
                }
                vertices.push(MarkerVertex {
                    x,
                    y,
                    incoming: Some(angle),
                    outgoing: None,
                });
                current = (x, y);
            }
            PathSeg::CurveTo(x1, y1, x2, y2, x, y) => {
                let start_vector =
                    if (x1 - current.0).abs() > 1.0e-6 || (y1 - current.1).abs() > 1.0e-6 {
                        (x1 - current.0, y1 - current.1)
                    } else {
                        (x2 - current.0, y2 - current.1)
                    };
                let end_vector = if (x - x2).abs() > 1.0e-6 || (y - y2).abs() > 1.0e-6 {
                    (x - x2, y - y2)
                } else {
                    (x - x1, y - y1)
                };
                let start_angle = crate::math::atan2(start_vector.1, start_vector.0);
                let end_angle = crate::math::atan2(end_vector.1, end_vector.0);
                if let Some(vertex) = vertices.last_mut() {
                    vertex.outgoing = Some(start_angle);
                }
                vertices.push(MarkerVertex {
                    x,
                    y,
                    incoming: Some(end_angle),
                    outgoing: None,
                });
                current = (x, y);
            }
            PathSeg::Close => {
                if let Some((x, y)) = subpath_start {
                    let angle = crate::math::atan2(y - current.1, x - current.0);
                    if let Some(vertex) = vertices.last_mut() {
                        vertex.outgoing = Some(angle);
                    }
                    vertices.push(MarkerVertex {
                        x,
                        y,
                        incoming: Some(angle),
                        outgoing: None,
                    });
                    current = (x, y);
                }
            }
        }
    }
    vertices
}

fn marker_mid_angle(incoming: Option<f32>, outgoing: Option<f32>) -> f32 {
    match (incoming, outgoing) {
        (Some(incoming), Some(outgoing)) => {
            let (sin_in, cos_in) = crate::math::sin_cos(incoming);
            let (sin_out, cos_out) = crate::math::sin_cos(outgoing);
            crate::math::atan2(sin_in + sin_out, cos_in + cos_out)
        }
        (Some(angle), None) | (None, Some(angle)) => angle,
        (None, None) => 0.0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkerPosition {
    Start,
    Mid,
    End,
}

#[allow(clippy::too_many_arguments)]
fn compile_path_markers(
    out: &mut Vec<CompiledItem>,
    segs: &[PathSeg],
    style: &SvgStyle,
    ctm: Matrix,
    gradients: &std::collections::HashMap<String, GradientDef>,
    id_map: &std::collections::HashMap<String, XmlNode<'_>>,
    stylesheet: &SvgStylesheet,
    resolving_ids: &mut Vec<String>,
) {
    if style.marker_start.is_none() && style.marker_mid.is_none() && style.marker_end.is_none() {
        return;
    }
    let vertices = marker_vertices(segs);
    if vertices.len() < 2 {
        return;
    }
    if let Some(id) = style.marker_start.as_deref() {
        let vertex = vertices[0];
        compile_marker_instance(
            out,
            id,
            vertex.x,
            vertex.y,
            vertex.outgoing.unwrap_or(0.0),
            MarkerPosition::Start,
            style,
            ctm,
            gradients,
            id_map,
            stylesheet,
            resolving_ids,
        );
    }
    if let Some(id) = style.marker_mid.as_deref() {
        for vertex in &vertices[1..vertices.len() - 1] {
            compile_marker_instance(
                out,
                id,
                vertex.x,
                vertex.y,
                marker_mid_angle(vertex.incoming, vertex.outgoing),
                MarkerPosition::Mid,
                style,
                ctm,
                gradients,
                id_map,
                stylesheet,
                resolving_ids,
            );
        }
    }
    if let Some(id) = style.marker_end.as_deref() {
        let vertex = vertices[vertices.len() - 1];
        compile_marker_instance(
            out,
            id,
            vertex.x,
            vertex.y,
            vertex.incoming.unwrap_or(0.0),
            MarkerPosition::End,
            style,
            ctm,
            gradients,
            id_map,
            stylesheet,
            resolving_ids,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn compile_marker_instance(
    out: &mut Vec<CompiledItem>,
    id: &str,
    x: f32,
    y: f32,
    auto_angle: f32,
    position: MarkerPosition,
    path_style: &SvgStyle,
    ctm: Matrix,
    gradients: &std::collections::HashMap<String, GradientDef>,
    id_map: &std::collections::HashMap<String, XmlNode<'_>>,
    stylesheet: &SvgStylesheet,
    resolving_ids: &mut Vec<String>,
) {
    if resolving_ids.iter().any(|active| active == id) {
        return;
    }
    let Some(marker) = id_map.get(id).copied() else {
        return;
    };
    if !marker.tag_name().name().eq_ignore_ascii_case("marker") {
        return;
    }
    let marker_width = marker
        .attribute("markerWidth")
        .or_else(|| marker.attribute("markerwidth"))
        .and_then(parse_number)
        .unwrap_or(3.0)
        .max(0.0);
    let marker_height = marker
        .attribute("markerHeight")
        .or_else(|| marker.attribute("markerheight"))
        .and_then(parse_number)
        .unwrap_or(3.0)
        .max(0.0);
    if marker_width <= 0.0 || marker_height <= 0.0 {
        return;
    }
    let view_box = parse_viewbox(
        marker
            .attribute("viewBox")
            .or_else(|| marker.attribute("viewbox")),
    );
    let viewport = viewbox_to_viewport_matrix(view_box, marker_width, marker_height);
    let ref_x = marker
        .attribute("refX")
        .or_else(|| marker.attribute("refx"))
        .and_then(parse_number)
        .unwrap_or(0.0);
    let ref_y = marker
        .attribute("refY")
        .or_else(|| marker.attribute("refy"))
        .and_then(parse_number)
        .unwrap_or(0.0);
    let (reference_x, reference_y) = viewport.apply(ref_x, ref_y);
    let unit_scale = if marker
        .attribute("markerUnits")
        .or_else(|| marker.attribute("markerunits"))
        .is_some_and(|value| value.eq_ignore_ascii_case("userSpaceOnUse"))
    {
        1.0
    } else {
        path_style.stroke_width.max(0.0)
    };
    let orient = marker.attribute("orient").unwrap_or("0").trim();
    let angle_degrees = match orient {
        "auto" => auto_angle.to_degrees(),
        "auto-start-reverse" if matches!(position, MarkerPosition::Start) => {
            auto_angle.to_degrees() + 180.0
        }
        "auto-start-reverse" => auto_angle.to_degrees(),
        value => parse_number(value).unwrap_or(0.0),
    };
    let mut marker_ctm = ctm
        .mul(Matrix::translate(x, y))
        .mul(Matrix::rotate(angle_degrees))
        .mul(Matrix::scale(unit_scale, unit_scale))
        .mul(Matrix::translate(-reference_x, -reference_y))
        .mul(viewport);
    if let Some(transform) = marker.attribute("transform") {
        marker_ctm = marker_ctm.mul(parse_transform(transform));
    }

    let mut marker_style = SvgStyle::default();
    apply_presentation_and_style(marker, stylesheet, &mut marker_style);
    apply_marker_context_paint(marker, path_style, &mut marker_style);
    resolving_ids.push(id.to_string());
    for child in marker.children() {
        let mut child_style = marker_style.clone();
        apply_marker_context_paint(child, path_style, &mut child_style);
        compile_element(
            out,
            child,
            marker_ctm,
            &child_style,
            gradients,
            id_map,
            stylesheet,
            resolving_ids,
        );
    }
    resolving_ids.pop();
}

fn apply_marker_context_paint(node: XmlNode<'_>, path_style: &SvgStyle, style: &mut SvgStyle) {
    match node.attribute("fill").map(str::trim) {
        Some(value) if value.eq_ignore_ascii_case("context-stroke") => {
            style.fill = path_style.stroke.clone();
        }
        Some(value) if value.eq_ignore_ascii_case("context-fill") => {
            style.fill = path_style.fill.clone();
        }
        _ => {}
    }
    match node.attribute("stroke").map(str::trim) {
        Some(value) if value.eq_ignore_ascii_case("context-stroke") => {
            style.stroke = path_style.stroke.clone();
        }
        Some(value) if value.eq_ignore_ascii_case("context-fill") => {
            style.stroke = path_style.fill.clone();
        }
        _ => {}
    }
}

fn push_compiled_path(
    out: &mut Vec<CompiledItem>,
    segs: &[PathSeg],
    style: &SvgStyle,
    ctm: Matrix,
    gradients: &std::collections::HashMap<String, GradientDef>,
    clip: Option<(Vec<PathSeg>, bool)>,
) {
    let has_fill = style.fill.color.is_some() || style.fill.gradient_id.is_some();
    let has_stroke = style.stroke.color.is_some() && style.stroke_width > 0.0;
    if !has_fill && !has_stroke {
        return;
    }

    // Flatten CTM into points once (so render is cheap and thread-friendly).
    let mut out_segs: Vec<PathSeg> = Vec::with_capacity(segs.len());
    for seg in segs {
        match *seg {
            PathSeg::MoveTo(px, py) => {
                let (x, y) = ctm.apply(px, py);
                let x = q(x);
                let y = q(y);
                out_segs.push(PathSeg::MoveTo(x, y));
            }
            PathSeg::LineTo(px, py) => {
                let (x, y) = ctm.apply(px, py);
                let x = q(x);
                let y = q(y);
                out_segs.push(PathSeg::LineTo(x, y));
            }
            PathSeg::CurveTo(x1, y1, x2, y2, x3, y3) => {
                let (x1, y1) = ctm.apply(x1, y1);
                let (x2, y2) = ctm.apply(x2, y2);
                let (x3, y3) = ctm.apply(x3, y3);
                let x1 = q(x1);
                let y1 = q(y1);
                let x2 = q(x2);
                let y2 = q(y2);
                let x3 = q(x3);
                let y3 = q(y3);
                out_segs.push(PathSeg::CurveTo(x1, y1, x2, y2, x3, y3));
            }
            PathSeg::Close => out_segs.push(PathSeg::Close),
        }
    }

    let mut out_style = style.clone();
    out_style.fill_shading = None;
    if has_stroke {
        let sf = ctm.scale_factor();
        out_style.stroke_width = out_style.stroke_width * sf;
        if !out_style.dash_pattern.is_empty() {
            for v in &mut out_style.dash_pattern {
                *v *= sf;
            }
            out_style.dash_offset *= sf;
        }
    }

    // Resolve gradient fills into a concrete shading for this path instance.
    if out_style.fill.color.is_none() {
        if let Some(ref id) = out_style.fill.gradient_id {
            if let Some(b) = bbox_of_segs(&out_segs) {
                if let Some(sh) = resolve_gradient_fill(id, gradients, b) {
                    out_style.fill_shading = Some(sh);
                }
            }
        }
    }
    out.push(CompiledItem::Path(CompiledPath {
        segs: out_segs,
        style: out_style,
        clip,
    }));
}

fn bbox_of_segs(segs: &[PathSeg]) -> Option<(f32, f32, f32, f32)> {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    for seg in segs {
        match *seg {
            PathSeg::MoveTo(x, y) | PathSeg::LineTo(x, y) => {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
            PathSeg::CurveTo(x1, y1, x2, y2, x, y) => {
                for (px, py) in [(x1, y1), (x2, y2), (x, y)] {
                    min_x = min_x.min(px);
                    min_y = min_y.min(py);
                    max_x = max_x.max(px);
                    max_y = max_y.max(py);
                }
            }
            PathSeg::Close => {}
        }
    }

    if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
        return None;
    }
    let w = (max_x - min_x).max(0.0);
    let h = (max_y - min_y).max(0.0);
    Some((min_x, min_y, w, h))
}

fn resolve_gradient_fill(
    id: &str,
    gradients: &std::collections::HashMap<String, GradientDef>,
    bbox: (f32, f32, f32, f32),
) -> Option<crate::types::Shading> {
    use crate::types::Shading;

    let def = gradients.get(id)?;
    let (bx, by, bw, bh) = bbox;
    if bw <= 0.0 || bh <= 0.0 {
        return None;
    }

    fn frac(c: Coord) -> f32 {
        // For objectBoundingBox we treat both percent and unitless as fraction.
        c.v
    }
    fn coord_x(c: Coord, units: GradientUnits, bx: f32, bw: f32) -> f32 {
        match units {
            GradientUnits::ObjectBoundingBox => bx + bw * frac(c),
            GradientUnits::UserSpaceOnUse => {
                if c.is_percent {
                    bx + bw * c.v
                } else {
                    c.v
                }
            }
        }
    }
    fn coord_y(c: Coord, units: GradientUnits, by: f32, bh: f32) -> f32 {
        match units {
            GradientUnits::ObjectBoundingBox => by + bh * frac(c),
            GradientUnits::UserSpaceOnUse => {
                if c.is_percent {
                    by + bh * c.v
                } else {
                    c.v
                }
            }
        }
    }

    match def {
        GradientDef::Linear {
            x1,
            y1,
            x2,
            y2,
            units,
            transform,
            stops,
        } => {
            if stops.is_empty() {
                return None;
            }
            let mut x0 = coord_x(*x1, *units, bx, bw);
            let mut y0 = coord_y(*y1, *units, by, bh);
            let mut x1 = coord_x(*x2, *units, bx, bw);
            let mut y1 = coord_y(*y2, *units, by, bh);
            if let Some(m) = transform {
                (x0, y0) = m.apply(x0, y0);
                (x1, y1) = m.apply(x1, y1);
            }
            Some(Shading::Axial {
                x0: q(x0),
                y0: q(y0),
                x1: q(x1),
                y1: q(y1),
                stops: stops.clone(),
            })
        }
        GradientDef::Radial {
            cx,
            cy,
            r,
            units,
            transform,
            stops,
        } => {
            if stops.is_empty() {
                return None;
            }
            let mut cxv = coord_x(*cx, *units, bx, bw);
            let mut cyv = coord_y(*cy, *units, by, bh);
            let mut rv = match units {
                GradientUnits::ObjectBoundingBox => bw.min(bh) * frac(*r),
                GradientUnits::UserSpaceOnUse => {
                    if r.is_percent {
                        bw.min(bh) * r.v
                    } else {
                        r.v
                    }
                }
            };
            if let Some(m) = transform {
                (cxv, cyv) = m.apply(cxv, cyv);
                rv *= m.scale_factor();
            }
            Some(Shading::Radial {
                x0: q(cxv),
                y0: q(cyv),
                r0: 0.0,
                x1: q(cxv),
                y1: q(cyv),
                r1: q(rv.max(0.0)),
                stops: stops.clone(),
                hard_stops: false,
            })
        }
    }
}

fn translate_shading(sh: &crate::types::Shading, dx: f32, dy: f32) -> crate::types::Shading {
    use crate::types::Shading;
    match sh {
        Shading::Axial {
            x0,
            y0,
            x1,
            y1,
            stops,
        } => Shading::Axial {
            x0: x0 + dx,
            y0: y0 + dy,
            x1: x1 + dx,
            y1: y1 + dy,
            stops: stops.clone(),
        },
        Shading::Radial {
            x0,
            y0,
            r0,
            x1,
            y1,
            r1,
            stops,
            hard_stops,
        } => Shading::Radial {
            x0: x0 + dx,
            y0: y0 + dy,
            r0: *r0,
            x1: x1 + dx,
            y1: y1 + dy,
            r1: *r1,
            stops: stops.clone(),
            hard_stops: *hard_stops,
        },
        Shading::Conic {
            center_x,
            center_y,
            radius,
            start_angle_deg,
            stops,
            hard_stops,
        } => Shading::Conic {
            center_x: center_x + dx,
            center_y: center_y + dy,
            radius: *radius,
            start_angle_deg: *start_angle_deg,
            stops: stops.clone(),
            hard_stops: *hard_stops,
        },
    }
}

fn compile_clip_for_node(
    node: XmlNode<'_>,
    ctm: Matrix,
    id_map: &std::collections::HashMap<String, XmlNode<'_>>,
) -> Option<(Vec<PathSeg>, bool)> {
    let clip = node.attribute("clip-path")?;
    let id = parse_url_ref(clip)?;
    let clip_node = id_map.get(&id).copied()?;
    if !clip_node.is_element() || clip_node.tag_name().name() != "clipPath" {
        return None;
    }

    // clip-rule defaults to nonzero. (SVG also supports clip-rule on children; ignore for now.)
    let evenodd = clip_node
        .attribute("clip-rule")
        .map(|v| v.trim().eq_ignore_ascii_case("evenodd"))
        .unwrap_or(false);

    let mut out = Vec::new();
    compile_clip_subtree(&mut out, clip_node, ctm, id_map);
    if out.is_empty() {
        None
    } else {
        Some((out, evenodd))
    }
}

fn compile_clip_subtree(
    out: &mut Vec<PathSeg>,
    node: XmlNode<'_>,
    ctm: Matrix,
    id_map: &std::collections::HashMap<String, XmlNode<'_>>,
) {
    if !node.is_element() {
        return;
    }

    let mut local_ctm = ctm;
    if let Some(transform) = node.attribute("transform") {
        local_ctm = local_ctm.mul(parse_transform(transform));
    }

    match node.tag_name().name() {
        "clipPath" | "g" | "svg" | "defs" => {
            for child in node.children().filter(|n| n.is_element()) {
                compile_clip_subtree(out, child, local_ctm, id_map);
            }
        }
        "use" => {
            if let Some(id) = href_id(node) {
                if let Some(target) = id_map.get(&id).copied() {
                    let x = parse_number(node.attribute("x").unwrap_or("0")).unwrap_or(0.0);
                    let y = parse_number(node.attribute("y").unwrap_or("0")).unwrap_or(0.0);
                    let use_ctm = local_ctm.mul(Matrix::translate(x, y));
                    compile_clip_subtree(out, target, use_ctm, id_map);
                }
            }
        }
        "path" => {
            if let Some(d) = node.attribute("d") {
                let segs = parse_path_data(d);
                out.extend(transform_path_segs(&segs, local_ctm));
            }
        }
        "rect" => {
            if let Some(segs) = rect_to_path(node) {
                out.extend(transform_path_segs(&segs, local_ctm));
            }
        }
        "circle" => {
            if let Some(segs) = circle_to_path(node) {
                out.extend(transform_path_segs(&segs, local_ctm));
            }
        }
        "ellipse" => {
            if let Some(segs) = ellipse_to_path(node) {
                out.extend(transform_path_segs(&segs, local_ctm));
            }
        }
        "line" => {
            if let Some(segs) = line_to_path(node) {
                out.extend(transform_path_segs(&segs, local_ctm));
            }
        }
        "polyline" => {
            if let Some(segs) = poly_points_to_path(node, false) {
                out.extend(transform_path_segs(&segs, local_ctm));
            }
        }
        "polygon" => {
            if let Some(segs) = poly_points_to_path(node, true) {
                out.extend(transform_path_segs(&segs, local_ctm));
            }
        }
        _ => {}
    }
}

fn transform_path_segs(segs: &[PathSeg], ctm: Matrix) -> Vec<PathSeg> {
    let mut out = Vec::with_capacity(segs.len());
    for seg in segs {
        match *seg {
            PathSeg::MoveTo(x, y) => {
                let (x, y) = ctm.apply(x, y);
                out.push(PathSeg::MoveTo(x, y));
            }
            PathSeg::LineTo(x, y) => {
                let (x, y) = ctm.apply(x, y);
                out.push(PathSeg::LineTo(x, y));
            }
            PathSeg::CurveTo(x1, y1, x2, y2, x, y) => {
                let (x1, y1) = ctm.apply(x1, y1);
                let (x2, y2) = ctm.apply(x2, y2);
                let (x, y) = ctm.apply(x, y);
                out.push(PathSeg::CurveTo(x1, y1, x2, y2, x, y));
            }
            PathSeg::Close => out.push(PathSeg::Close),
        }
    }
    out
}

fn draw_compiled_path(canvas: &mut Canvas, path: &CompiledPath, x_off: Pt, y_off: Pt) {
    let has_fill = path.style.fill.color.is_some() || path.style.fill_shading.is_some();
    let has_stroke = path.style.stroke.color.is_some() && path.style.stroke_width > 0.0;
    if !has_fill && !has_stroke {
        return;
    }

    fn emit_path(canvas: &mut Canvas, segs: &[PathSeg], x_off: Pt, y_off: Pt) {
        for seg in segs {
            match *seg {
                PathSeg::MoveTo(px, py) => {
                    canvas.move_to(x_off + Pt::from_f32(px), y_off + Pt::from_f32(py))
                }
                PathSeg::LineTo(px, py) => {
                    canvas.line_to(x_off + Pt::from_f32(px), y_off + Pt::from_f32(py))
                }
                PathSeg::CurveTo(x1, y1, x2, y2, x3, y3) => {
                    canvas.curve_to(
                        x_off + Pt::from_f32(x1),
                        y_off + Pt::from_f32(y1),
                        x_off + Pt::from_f32(x2),
                        y_off + Pt::from_f32(y2),
                        x_off + Pt::from_f32(x3),
                        y_off + Pt::from_f32(y3),
                    );
                }
                PathSeg::Close => canvas.close_path(),
            }
        }
    }

    let mut clipped = false;
    if let Some((clip_segs, evenodd)) = &path.clip {
        canvas.save_state();
        emit_path(canvas, clip_segs, x_off, y_off);
        canvas.clip_path(*evenodd);
        clipped = true;
    }

    if has_stroke {
        canvas.set_miter_limit(Pt::from_f32(path.style.miter_limit));
        if !path.style.dash_pattern.is_empty() {
            let pattern = path
                .style
                .dash_pattern
                .iter()
                .map(|v| Pt::from_f32(*v))
                .collect::<Vec<_>>();
            canvas.set_dash(pattern, Pt::from_f32(path.style.dash_offset));
        } else {
            // Reset dash.
            canvas.set_dash(Vec::new(), Pt::ZERO);
        }
    }

    if path.style.fill_opacity < 1.0 || path.style.stroke_opacity < 1.0 {
        canvas.set_opacity(path.style.fill_opacity, path.style.stroke_opacity);
    } else {
        canvas.set_opacity(1.0, 1.0);
    }

    // Gradient fill path: clip then shade, then optionally stroke.
    if let Some(sh) = &path.style.fill_shading {
        let sh = translate_shading(sh, x_off.to_f32(), y_off.to_f32());
        canvas.save_state();
        emit_path(canvas, &path.segs, x_off, y_off);
        canvas.clip_path(path.style.fill_rule_evenodd);
        canvas.shading_fill(sh);
        canvas.restore_state();

        if has_stroke {
            if let Some(stroke) = path.style.stroke.color {
                canvas.set_stroke_color(stroke);
                canvas.set_line_width(Pt::from_f32(path.style.stroke_width));
                canvas.set_line_cap(path.style.line_cap);
                canvas.set_line_join(path.style.line_join);
            }
            emit_path(canvas, &path.segs, x_off, y_off);
            canvas.stroke();
        }
        if clipped {
            canvas.restore_state();
        }
        return;
    }

    if let Some(fill) = path.style.fill.color {
        canvas.set_fill_color(fill);
    }
    if let Some(stroke) = path.style.stroke.color {
        canvas.set_stroke_color(stroke);
        canvas.set_line_width(Pt::from_f32(path.style.stroke_width));
        canvas.set_line_cap(path.style.line_cap);
        canvas.set_line_join(path.style.line_join);
    }

    emit_path(canvas, &path.segs, x_off, y_off);

    match (has_fill, has_stroke) {
        (true, true) => {
            if path.style.fill_rule_evenodd {
                canvas.fill_stroke_evenodd()
            } else {
                canvas.fill_stroke()
            }
        }
        (true, false) => {
            if path.style.fill_rule_evenodd {
                canvas.fill_evenodd()
            } else {
                canvas.fill()
            }
        }
        (false, true) => canvas.stroke(),
        (false, false) => {}
    }

    if clipped {
        canvas.restore_state();
    }
}

fn parse_viewbox(view_box: Option<&str>) -> Option<(f32, f32, f32, f32)> {
    let vb = view_box?;
    let mut it = vb
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty());
    let min_x = it.next()?.parse::<f32>().ok()?;
    let min_y = it.next()?.parse::<f32>().ok()?;
    let w = it.next()?.parse::<f32>().ok()?;
    let h = it.next()?.parse::<f32>().ok()?;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    Some((min_x, min_y, w, h))
}

fn viewbox_to_viewport_matrix(view_box: Option<(f32, f32, f32, f32)>, w: f32, h: f32) -> Matrix {
    viewbox_to_viewport_matrix_with_aspect(view_box, w, h, None)
}

fn viewbox_to_viewport_matrix_with_aspect(
    view_box: Option<(f32, f32, f32, f32)>,
    w: f32,
    h: f32,
    preserve_aspect_ratio: Option<&str>,
) -> Matrix {
    let Some((min_x, min_y, vb_w, vb_h)) = view_box else {
        return Matrix::identity();
    };

    let sx = if vb_w > 0.0 { w / vb_w } else { 1.0 };
    let sy = if vb_h > 0.0 { h / vb_h } else { 1.0 };
    let raw = preserve_aspect_ratio.unwrap_or("xMidYMid meet").trim();
    let mut tokens = raw
        .split_ascii_whitespace()
        .filter(|token| *token != "defer");
    let align = tokens.next().unwrap_or("xMidYMid");
    if align.eq_ignore_ascii_case("none") {
        return Matrix::translate(-min_x * sx, -min_y * sy).mul(Matrix::scale(sx, sy));
    }
    let meet_or_slice = tokens.next().unwrap_or("meet");
    let s = if meet_or_slice.eq_ignore_ascii_case("slice") {
        sx.max(sy)
    } else {
        sx.min(sy)
    };
    let remaining_x = w - vb_w * s;
    let remaining_y = h - vb_h * s;
    let align_x = if align.starts_with("xMin") {
        0.0
    } else if align.starts_with("xMax") {
        1.0
    } else {
        0.5
    };
    let align_y = if align.ends_with("YMin") {
        0.0
    } else if align.ends_with("YMax") {
        1.0
    } else {
        0.5
    };
    let tx = remaining_x * align_x - min_x * s;
    let ty = remaining_y * align_y - min_y * s;
    Matrix::translate(tx, ty).mul(Matrix::scale(s, s))
}

fn extract_svg_stylesheet(doc: &XmlDocument) -> SvgStylesheet {
    let mut out = SvgStylesheet::default();
    let mut order = 0usize;

    for node in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case("style"))
    {
        let css = node.text().unwrap_or_default().trim();
        if css.is_empty() {
            continue;
        }
        let Ok(sheet) = css_native::parse_stylesheet(css) else {
            continue;
        };
        collect_svg_style_rules(&sheet.rules, &mut out.rules, &mut order);
    }

    out
}

fn collect_svg_style_rules(rules: &[CssRule], out: &mut Vec<SvgCssRule>, order: &mut usize) {
    for rule in rules {
        match rule {
            CssRule::Style(style_rule) => {
                if style_rule.declarations.is_empty() {
                    *order += 1;
                    continue;
                }
                let selectors = css_native::split_top_level(&style_rule.selectors, ',')
                    .unwrap_or_else(|_| vec![style_rule.selectors.clone()]);
                for selector_raw in selectors {
                    if let Some(selector) = parse_svg_selector(&selector_raw) {
                        out.push(SvgCssRule {
                            selector,
                            declarations: style_rule.declarations.clone(),
                            order: *order,
                        });
                    }
                }
                *order += 1;
            }
            CssRule::At(at_rule) if at_rule.name == "media" => {
                if let Some(AtRuleBlock::Rules(nested)) = &at_rule.block {
                    collect_svg_style_rules(nested, out, order);
                }
            }
            _ => {}
        }
    }
}

fn parse_svg_selector(raw: &str) -> Option<SvgSelector> {
    let selector = raw.trim();
    if selector.is_empty() {
        return None;
    }

    let mut parts = Vec::new();
    let mut id_count = 0u16;
    let mut class_count = 0u16;
    let mut tag_count = 0u16;

    for token in selector.split_whitespace() {
        let part = parse_svg_simple_selector(token)?;
        if part.id.is_some() {
            id_count += 1;
        }
        class_count += part.classes.len() as u16;
        if part.tag.is_some() {
            tag_count += 1;
        }
        parts.push(part);
    }

    if parts.is_empty() {
        return None;
    }

    Some(SvgSelector {
        parts,
        specificity: SvgSpecificity(id_count, class_count, tag_count),
    })
}

fn parse_svg_simple_selector(token: &str) -> Option<SvgSimpleSelector> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    if token.contains(':')
        || token.contains('[')
        || token.contains(']')
        || token.contains('>')
        || token.contains('+')
        || token.contains('~')
    {
        return None;
    }

    let bytes = token.as_bytes();
    let mut i = 0usize;
    let len = bytes.len();
    let mut tag = None;
    let mut id = None;
    let mut classes = Vec::new();

    if bytes[0] == b'*' {
        i = 1;
    } else if is_svg_selector_ident_start(bytes[0]) {
        let start = i;
        i += 1;
        while i < len && is_svg_selector_ident_char(bytes[i]) {
            i += 1;
        }
        if i > start {
            tag = Some(token[start..i].to_ascii_lowercase());
        }
    }

    while i < len {
        match bytes[i] {
            b'.' => {
                i += 1;
                let start = i;
                while i < len && is_svg_selector_ident_char(bytes[i]) {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                classes.push(token[start..i].to_string());
            }
            b'#' => {
                i += 1;
                let start = i;
                while i < len && is_svg_selector_ident_char(bytes[i]) {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                if id.is_some() {
                    return None;
                }
                id = Some(token[start..i].to_string());
            }
            _ => return None,
        }
    }

    if tag.is_none() && id.is_none() && classes.is_empty() {
        return None;
    }

    Some(SvgSimpleSelector { tag, id, classes })
}

fn is_svg_selector_ident_start(ch: u8) -> bool {
    ch.is_ascii_alphabetic() || ch == b'_'
}

fn is_svg_selector_ident_char(ch: u8) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'-' | b':')
}

fn parent_element<'a>(node: XmlNode<'a>) -> Option<XmlNode<'a>> {
    let mut cursor = node.parent();
    while let Some(parent) = cursor {
        if parent.is_element() {
            return Some(parent);
        }
        cursor = parent.parent();
    }
    None
}

fn svg_simple_selector_matches(node: XmlNode<'_>, selector: &SvgSimpleSelector) -> bool {
    if let Some(tag) = &selector.tag {
        if !node.tag_name().name().eq_ignore_ascii_case(tag) {
            return false;
        }
    }
    if let Some(id) = &selector.id {
        if node.attribute("id") != Some(id.as_str()) {
            return false;
        }
    }
    for class_name in &selector.classes {
        let Some(node_classes) = node.attribute("class") else {
            return false;
        };
        if !node_classes
            .split_whitespace()
            .any(|candidate| candidate == class_name)
        {
            return false;
        }
    }
    true
}

fn svg_selector_matches(node: XmlNode<'_>, selector: &SvgSelector) -> bool {
    let Some(last) = selector.parts.last() else {
        return false;
    };
    if !svg_simple_selector_matches(node, last) {
        return false;
    }

    let mut anchor = parent_element(node);
    for part in selector.parts.iter().rev().skip(1) {
        let mut probe = anchor;
        let mut matched = None;
        while let Some(candidate) = probe {
            if svg_simple_selector_matches(candidate, part) {
                matched = Some(candidate);
                break;
            }
            probe = parent_element(candidate);
        }
        let Some(candidate) = matched else {
            return false;
        };
        anchor = parent_element(candidate);
    }

    true
}

fn apply_svg_stylesheet(node: XmlNode<'_>, stylesheet: &SvgStylesheet, style: &mut SvgStyle) {
    if stylesheet.rules.is_empty() {
        return;
    }

    let mut matched: Vec<&SvgCssRule> = stylesheet
        .rules
        .iter()
        .filter(|rule| svg_selector_matches(node, &rule.selector))
        .collect();
    if matched.is_empty() {
        return;
    }

    matched.sort_by(|a, b| {
        a.selector
            .specificity
            .cmp(&b.selector.specificity)
            .then(a.order.cmp(&b.order))
    });

    for rule in &matched {
        apply_svg_declarations(rule.declarations.normal(), style);
    }
    for rule in &matched {
        apply_svg_declarations(rule.declarations.important(), style);
    }
}

fn apply_presentation_and_style(
    node: XmlNode<'_>,
    stylesheet: &SvgStylesheet,
    style: &mut SvgStyle,
) {
    // Presentation attributes are the baseline.
    if let Some(fill) = node.attribute("fill") {
        if let Some(alpha) = apply_svg_paint_value(fill, &mut style.fill) {
            style.fill_opacity *= alpha;
        }
    }
    if let Some(stroke) = node.attribute("stroke") {
        if let Some(alpha) = apply_svg_paint_value(stroke, &mut style.stroke) {
            style.stroke_opacity *= alpha;
        }
    }
    if let Some(sw) = node.attribute("stroke-width") {
        if let Some(v) = parse_number(sw) {
            style.stroke_width = v.max(0.0);
        }
    }
    if let Some(m) = node.attribute("stroke-miterlimit") {
        if let Some(v) = parse_number(m) {
            style.miter_limit = v.max(0.0);
        }
    }
    if let Some(cap) = node.attribute("stroke-linecap") {
        style.line_cap = match cap.trim() {
            "round" => 1,
            "square" => 2,
            _ => 0,
        };
    }
    if let Some(join) = node.attribute("stroke-linejoin") {
        style.line_join = match join.trim() {
            "round" => 1,
            "bevel" => 2,
            _ => 0,
        };
    }
    if let Some(fr) = node.attribute("fill-rule") {
        style.fill_rule_evenodd = fr.trim().eq_ignore_ascii_case("evenodd");
    }
    if let Some(da) = node.attribute("stroke-dasharray") {
        if da.trim().eq_ignore_ascii_case("none") {
            style.dash_pattern.clear();
        } else {
            style.dash_pattern = parse_length_list(da);
            if style.dash_pattern.len() % 2 == 1 {
                let dup = style.dash_pattern.clone();
                style.dash_pattern.extend_from_slice(&dup);
            }
        }
    }
    if let Some(off) = node.attribute("stroke-dashoffset") {
        if let Some(v) = parse_number(off) {
            style.dash_offset = v;
        }
    }
    if let Some(value) = node.attribute("font-family") {
        style.font_family = value.trim().to_string();
    }
    if let Some(value) = node.attribute("font-size") {
        if let Some(size) = parse_svg_font_size(value, style.font_size) {
            style.font_size = size;
        }
    }
    if let Some(value) = node.attribute("font-weight") {
        if let Some(weight) = parse_svg_font_weight(value) {
            style.font_weight = weight;
        }
    }
    if let Some(value) = node.attribute("font-style") {
        style.font_italic = matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "italic" | "oblique"
        );
    }
    if let Some(value) = node.attribute("text-anchor") {
        style.text_anchor = parse_text_anchor(value);
    }
    if let Some(value) = node.attribute("marker") {
        let marker = parse_optional_url_ref(value);
        style.marker_start = marker.clone();
        style.marker_mid = marker.clone();
        style.marker_end = marker;
    }
    if let Some(value) = node.attribute("marker-start") {
        style.marker_start = parse_optional_url_ref(value);
    }
    if let Some(value) = node.attribute("marker-mid") {
        style.marker_mid = parse_optional_url_ref(value);
    }
    if let Some(value) = node.attribute("marker-end") {
        style.marker_end = parse_optional_url_ref(value);
    }

    // Opacity attributes multiply (opacity affects both fill/stroke).
    if let Some(v) = node.attribute("opacity").and_then(parse_number) {
        let o = v.clamp(0.0, 1.0);
        style.fill_opacity *= o;
        style.stroke_opacity *= o;
    }
    if let Some(v) = node.attribute("fill-opacity").and_then(parse_number) {
        style.fill_opacity *= v.clamp(0.0, 1.0);
    }
    if let Some(v) = node.attribute("stroke-opacity").and_then(parse_number) {
        style.stroke_opacity *= v.clamp(0.0, 1.0);
    }

    // Inline/embedded stylesheet rules override presentation attributes.
    apply_svg_stylesheet(node, stylesheet, style);

    // Inline style="" wins over presentation attributes.
    if let Some(s) = node.attribute("style") {
        apply_style_string(s, style);
    }
}

fn apply_style_string(input: &str, style: &mut SvgStyle) {
    if let Ok(declarations) = css_native::parse_declaration_block(input) {
        apply_svg_declarations(declarations.normal(), style);
        apply_svg_declarations(declarations.important(), style);
    }
}

fn apply_svg_declarations<'a>(
    declarations: impl IntoIterator<Item = &'a Declaration>,
    style: &mut SvgStyle,
) {
    for declaration in declarations {
        apply_svg_declaration(declaration, style);
    }
}

fn apply_svg_declaration(declaration: &Declaration, style: &mut SvgStyle) {
    let value = declaration.value.trim();
    match declaration.name.to_ascii_lowercase().as_str() {
        "fill" => {
            if let Some(alpha) = apply_svg_paint_value(value, &mut style.fill) {
                style.fill_opacity *= alpha;
            }
        }
        "stroke" => {
            if let Some(alpha) = apply_svg_paint_value(value, &mut style.stroke) {
                style.stroke_opacity *= alpha;
            }
        }
        "stroke-width" => {
            if let Some(width) = parse_number(value) {
                style.stroke_width = width.max(0.0);
            }
        }
        "stroke-miterlimit" => {
            if let Some(limit) = parse_number(value) {
                style.miter_limit = limit.max(0.0);
            }
        }
        "stroke-linecap" => match value.to_ascii_lowercase().as_str() {
            "butt" => style.line_cap = 0,
            "round" => style.line_cap = 1,
            "square" => style.line_cap = 2,
            _ => {}
        },
        "stroke-linejoin" => match value.to_ascii_lowercase().as_str() {
            "miter" | "miter-clip" | "arcs" => style.line_join = 0,
            "round" => style.line_join = 1,
            "bevel" => style.line_join = 2,
            _ => {}
        },
        "fill-rule" => match value.to_ascii_lowercase().as_str() {
            "nonzero" => style.fill_rule_evenodd = false,
            "evenodd" => style.fill_rule_evenodd = true,
            _ => {}
        },
        "stroke-dasharray" => {
            if value.eq_ignore_ascii_case("none") {
                style.dash_pattern.clear();
            } else {
                let values = parse_length_list(value);
                if !values.is_empty() {
                    style.dash_pattern = values;
                    if style.dash_pattern.len() % 2 == 1 {
                        let duplicate = style.dash_pattern.clone();
                        style.dash_pattern.extend_from_slice(&duplicate);
                    }
                }
            }
        }
        "stroke-dashoffset" => {
            if let Some(offset) = parse_number(value) {
                style.dash_offset = offset;
            }
        }
        "opacity" => {
            if let Some(opacity) = parse_svg_alpha(value) {
                style.fill_opacity *= opacity;
                style.stroke_opacity *= opacity;
            }
        }
        "fill-opacity" => {
            if let Some(opacity) = parse_svg_alpha(value) {
                style.fill_opacity *= opacity;
            }
        }
        "stroke-opacity" => {
            if let Some(opacity) = parse_svg_alpha(value) {
                style.stroke_opacity *= opacity;
            }
        }
        "font-family" => style.font_family = value.to_string(),
        "font-size" => {
            if let Some(size) = parse_svg_font_size(value, style.font_size) {
                style.font_size = size;
            }
        }
        "font-weight" => {
            if let Some(weight) = parse_svg_font_weight(value) {
                style.font_weight = weight;
            }
        }
        "font-style" => {
            let lower = value.to_ascii_lowercase();
            if lower == "normal" {
                style.font_italic = false;
            } else if lower == "italic" || lower == "oblique" || lower.starts_with("oblique ") {
                style.font_italic = true;
            }
        }
        "text-anchor" => match value.to_ascii_lowercase().as_str() {
            "start" | "middle" | "end" => style.text_anchor = parse_text_anchor(value),
            _ => {}
        },
        "marker" => {
            let marker = parse_optional_url_ref(value);
            style.marker_start = marker.clone();
            style.marker_mid = marker.clone();
            style.marker_end = marker;
        }
        "marker-start" => style.marker_start = parse_optional_url_ref(value),
        "marker-mid" => style.marker_mid = parse_optional_url_ref(value),
        "marker-end" => style.marker_end = parse_optional_url_ref(value),
        _ => {}
    }
}

fn apply_svg_paint_value(value: &str, output: &mut Paint) -> Option<f32> {
    if value.eq_ignore_ascii_case("none") {
        output.color = None;
        output.gradient_id = None;
        return Some(1.0);
    }
    if let Some(id) = parse_url_ref(value) {
        output.color = None;
        output.gradient_id = Some(id);
        return Some(1.0);
    }
    if matches!(
        value.to_ascii_lowercase().as_str(),
        "context-fill" | "context-stroke" | "currentcolor"
    ) {
        return None;
    }
    if let Some((color, alpha)) = crate::style::parse_color_string(value) {
        output.color = Some(color);
        output.gradient_id = None;
        return Some(alpha.clamp(0.0, 1.0));
    }
    None
}

fn parse_svg_alpha(value: &str) -> Option<f32> {
    let value = value.trim();
    let alpha = if let Some(percent) = value.strip_suffix('%') {
        percent.trim().parse::<f32>().ok()? / 100.0
    } else {
        value.parse::<f32>().ok()?
    };
    alpha.is_finite().then_some(alpha.clamp(0.0, 1.0))
}

fn parse_svg_font_size(value: &str, inherited: f32) -> Option<f32> {
    let raw = value.trim().to_ascii_lowercase();
    let size = if let Some(percent) = raw.strip_suffix('%') {
        inherited * percent.trim().parse::<f32>().ok()? / 100.0
    } else if let Some(em) = raw.strip_suffix("em") {
        inherited * em.trim().parse::<f32>().ok()?
    } else if let Some(ex) = raw.strip_suffix("ex") {
        inherited * ex.trim().parse::<f32>().ok()? * 0.5
    } else {
        match raw.as_str() {
            "xx-small" => 9.0,
            "x-small" => 10.0,
            "small" => 13.0,
            "medium" => 16.0,
            "large" => 18.0,
            "x-large" => 24.0,
            "xx-large" => 32.0,
            "smaller" => inherited * 0.8,
            "larger" => inherited * 1.2,
            _ => parse_number(&raw)?,
        }
    };
    size.is_finite().then_some(size.max(0.0))
}

fn parse_svg_font_weight(value: &str) -> Option<u16> {
    match value.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(400),
        "bold" => Some(700),
        "bolder" => Some(700),
        "lighter" => Some(300),
        raw => raw.parse::<u16>().ok().map(|weight| weight.clamp(1, 1000)),
    }
}

fn parse_text_anchor(value: &str) -> TextAnchor {
    match value.trim().to_ascii_lowercase().as_str() {
        "middle" => TextAnchor::Middle,
        "end" => TextAnchor::End,
        _ => TextAnchor::Start,
    }
}

fn parse_optional_url_ref(value: &str) -> Option<String> {
    if value.trim().eq_ignore_ascii_case("none") {
        None
    } else {
        parse_url_ref(value)
    }
}

fn parse_length_list(input: &str) -> Vec<f32> {
    input
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .filter_map(parse_number)
        .collect()
}

fn parse_url_ref(input: &str) -> Option<String> {
    let s = input.trim();
    if !s.to_ascii_lowercase().starts_with("url(") {
        return None;
    }
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    if close <= open + 1 {
        return None;
    }
    let inner = s[open + 1..close]
        .trim()
        .trim_matches('"')
        .trim_matches('\'');
    let id = inner.strip_prefix('#')?;
    if id.is_empty() {
        return None;
    }
    Some(id.to_string())
}

#[derive(Debug, Clone, Copy)]
enum GradientUnits {
    ObjectBoundingBox,
    UserSpaceOnUse,
}

#[derive(Debug, Clone, Copy)]
struct Coord {
    v: f32,
    is_percent: bool,
}

#[derive(Debug, Clone)]
enum GradientDef {
    Linear {
        x1: Coord,
        y1: Coord,
        x2: Coord,
        y2: Coord,
        units: GradientUnits,
        transform: Option<Matrix>,
        stops: Vec<crate::types::ShadingStop>,
    },
    Radial {
        cx: Coord,
        cy: Coord,
        r: Coord,
        units: GradientUnits,
        transform: Option<Matrix>,
        stops: Vec<crate::types::ShadingStop>,
    },
}

fn parse_coord(input: Option<&str>, default: Coord) -> Coord {
    let Some(s) = input else { return default };
    let s = s.trim();
    if let Some(p) = s.strip_suffix('%') {
        if let Ok(v) = p.trim().parse::<f32>() {
            return Coord {
                v: (v / 100.0),
                is_percent: true,
            };
        }
        return default;
    }
    if let Some(v) = parse_number(s) {
        return Coord {
            v,
            is_percent: false,
        };
    }
    default
}

fn parse_stop_offset(input: Option<&str>) -> Option<f32> {
    let s = input?.trim();
    if let Some(p) = s.strip_suffix('%') {
        let v = p.trim().parse::<f32>().ok()?;
        return Some((v / 100.0).clamp(0.0, 1.0));
    }
    let v = s.parse::<f32>().ok()?;
    Some(v.clamp(0.0, 1.0))
}

fn parse_stop_color(node: XmlNode<'_>, stylesheet: &SvgStylesheet) -> Option<Color> {
    let mut stop_color = node.attribute("stop-color").and_then(parse_svg_color);

    // Support class/id based declarations in embedded <style> blocks.
    if !stylesheet.rules.is_empty() {
        let mut matched: Vec<&SvgCssRule> = stylesheet
            .rules
            .iter()
            .filter(|rule| svg_selector_matches(node, &rule.selector))
            .collect();
        matched.sort_by(|a, b| {
            a.selector
                .specificity
                .cmp(&b.selector.specificity)
                .then(a.order.cmp(&b.order))
        });
        for rule in &matched {
            if let Some(color) = stop_color_from_declarations(rule.declarations.normal()) {
                stop_color = Some(color);
            }
        }
        for rule in &matched {
            if let Some(color) = stop_color_from_declarations(rule.declarations.important()) {
                stop_color = Some(color);
            }
        }
    }

    if let Some(style_attr) = node.attribute("style") {
        if let Ok(declarations) = css_native::parse_declaration_block(style_attr) {
            if let Some(color) = stop_color_from_declarations(declarations.normal()) {
                stop_color = Some(color);
            }
            if let Some(color) = stop_color_from_declarations(declarations.important()) {
                stop_color = Some(color);
            }
        }
    }
    stop_color
}

fn stop_color_from_declarations<'a>(
    declarations: impl IntoIterator<Item = &'a Declaration>,
) -> Option<Color> {
    let mut color = None;
    for declaration in declarations {
        if declaration.name_eq("stop-color") {
            if let Some(parsed) = parse_svg_color(&declaration.value) {
                color = Some(parsed);
            }
        }
    }
    color
}

fn parse_svg_color(value: &str) -> Option<Color> {
    crate::style::parse_color_string(value)
        .map(|(color, _)| color)
        .or_else(|| parse_color(value))
}

fn extract_gradients(
    doc: &XmlDocument,
    stylesheet: &SvgStylesheet,
) -> std::collections::HashMap<String, GradientDef> {
    // Opinionated SVG 1.1 subset: linearGradient + radialGradient with stop colors.
    // (We ignore per-stop opacity for now; we support element opacity via ExtGState.)
    let mut out: std::collections::HashMap<String, GradientDef> = std::collections::HashMap::new();
    let mut hrefs: Vec<(String, String)> = Vec::new();
    for node in doc.descendants().filter(|n| n.is_element()) {
        let name = node.tag_name().name();
        if name != "linearGradient" && name != "radialGradient" {
            continue;
        }
        let Some(id) = node.attribute("id") else {
            continue;
        };
        if let Some(href) = node
            .attribute("href")
            .or_else(|| node.attribute("xlink:href"))
        {
            let href = href.trim().trim_matches('"').trim_matches('\'');
            if let Some(base) = href.strip_prefix('#') {
                if !base.is_empty() {
                    hrefs.push((id.to_string(), base.to_string()));
                }
            }
        }

        let units = match node
            .attribute("gradientUnits")
            .unwrap_or("objectBoundingBox")
        {
            "userSpaceOnUse" => GradientUnits::UserSpaceOnUse,
            _ => GradientUnits::ObjectBoundingBox,
        };
        let transform = node.attribute("gradientTransform").map(parse_transform);

        let mut stops: Vec<crate::types::ShadingStop> = Vec::new();
        for stop in node
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "stop")
        {
            let Some(offset) = parse_stop_offset(stop.attribute("offset")) else {
                continue;
            };
            let Some(color) = parse_stop_color(stop, stylesheet) else {
                continue;
            };
            stops.push(crate::types::ShadingStop {
                offset,
                color,
                alpha: 1.0,
            });
        }

        let def = if name == "linearGradient" {
            GradientDef::Linear {
                x1: parse_coord(
                    node.attribute("x1"),
                    Coord {
                        v: 0.0,
                        is_percent: true,
                    },
                ),
                y1: parse_coord(
                    node.attribute("y1"),
                    Coord {
                        v: 0.0,
                        is_percent: true,
                    },
                ),
                x2: parse_coord(
                    node.attribute("x2"),
                    Coord {
                        v: 1.0,
                        is_percent: true,
                    },
                ),
                y2: parse_coord(
                    node.attribute("y2"),
                    Coord {
                        v: 0.0,
                        is_percent: true,
                    },
                ),
                units,
                transform,
                stops,
            }
        } else {
            GradientDef::Radial {
                cx: parse_coord(
                    node.attribute("cx"),
                    Coord {
                        v: 0.5,
                        is_percent: true,
                    },
                ),
                cy: parse_coord(
                    node.attribute("cy"),
                    Coord {
                        v: 0.5,
                        is_percent: true,
                    },
                ),
                r: parse_coord(
                    node.attribute("r"),
                    Coord {
                        v: 0.5,
                        is_percent: true,
                    },
                ),
                units,
                transform,
                stops,
            }
        };

        out.insert(id.to_string(), def);
    }

    // Resolve minimal inheritance: if a gradient references another (href) and has no stops,
    // inherit the referenced stops. (Coordinates/units inheritance is intentionally not handled yet.)
    for (id, base) in hrefs {
        let Some(base_def) = out.get(&base).cloned() else {
            continue;
        };
        if let Some(def) = out.get_mut(&id) {
            let base_stops = match base_def {
                GradientDef::Linear { stops, .. } => stops,
                GradientDef::Radial { stops, .. } => stops,
            };
            match def {
                GradientDef::Linear { stops, .. } | GradientDef::Radial { stops, .. } => {
                    if stops.is_empty() {
                        *stops = base_stops;
                    }
                }
            }
        }
    }
    out
}

fn parse_color(input: &str) -> Option<Color> {
    let v = input.trim();
    if v.eq_ignore_ascii_case("none") {
        return None;
    }
    if let Some(hex) = v.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;
            return Some(Color { r, g, b });
        }
    }
    // Minimal named color set (enough for common exports).
    match v.to_ascii_lowercase().as_str() {
        "black" => Some(Color::BLACK),
        "white" => Some(Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
        }),
        "red" => Some(Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
        }),
        "green" => Some(Color {
            r: 0.0,
            g: 0.5,
            b: 0.0,
        }),
        "blue" => Some(Color {
            r: 0.0,
            g: 0.0,
            b: 1.0,
        }),
        _ => None,
    }
}

fn parse_number(input: &str) -> Option<f32> {
    let s = input.trim();
    // Ignore unit suffixes for now (treat user units as-is).
    let s = s
        .trim_end_matches("px")
        .trim_end_matches("pt")
        .trim_end_matches("mm")
        .trim_end_matches("cm")
        .trim_end_matches("in")
        .trim();
    s.parse::<f32>().ok()
}

fn parse_transform(input: &str) -> Matrix {
    let mut out = Matrix::identity();
    let mut s = input.trim();

    while !s.is_empty() {
        // Find function name + (...)
        let Some(open) = s.find('(') else { break };
        let name = s[..open].trim();
        let Some(close) = s[open + 1..].find(')') else {
            break;
        };
        let args_str = &s[open + 1..open + 1 + close];
        let args = parse_number_list(args_str);

        let m = match name {
            "translate" => {
                let tx = args.get(0).copied().unwrap_or(0.0);
                let ty = args.get(1).copied().unwrap_or(0.0);
                Matrix::translate(tx, ty)
            }
            "scale" => {
                let sx = args.get(0).copied().unwrap_or(1.0);
                let sy = args.get(1).copied().unwrap_or(sx);
                Matrix::scale(sx, sy)
            }
            "rotate" => {
                let a = args.get(0).copied().unwrap_or(0.0);
                if args.len() >= 3 {
                    let cx = args[1];
                    let cy = args[2];
                    Matrix::translate(cx, cy)
                        .mul(Matrix::rotate(a))
                        .mul(Matrix::translate(-cx, -cy))
                } else {
                    Matrix::rotate(a)
                }
            }
            "matrix" => {
                if args.len() >= 6 {
                    Matrix {
                        a: args[0],
                        b: args[1],
                        c: args[2],
                        d: args[3],
                        e: args[4],
                        f: args[5],
                    }
                } else {
                    Matrix::identity()
                }
            }
            _ => Matrix::identity(),
        };

        out = out.mul(m);
        s = s[open + 1 + close + 1..].trim_start();
    }

    out
}

fn parse_number_list(input: &str) -> Vec<f32> {
    input
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f32>().ok())
        .collect()
}

fn rect_to_path(node: XmlNode<'_>) -> Option<Vec<PathSeg>> {
    let x = parse_number(node.attribute("x").unwrap_or("0")).unwrap_or(0.0);
    let y = parse_number(node.attribute("y").unwrap_or("0")).unwrap_or(0.0);
    let w = parse_number(node.attribute("width")?)?;
    let h = parse_number(node.attribute("height")?)?;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    Some(vec![
        PathSeg::MoveTo(x, y),
        PathSeg::LineTo(x + w, y),
        PathSeg::LineTo(x + w, y + h),
        PathSeg::LineTo(x, y + h),
        PathSeg::Close,
    ])
}

fn circle_to_path(node: XmlNode<'_>) -> Option<Vec<PathSeg>> {
    let cx = parse_number(node.attribute("cx").unwrap_or("0")).unwrap_or(0.0);
    let cy = parse_number(node.attribute("cy").unwrap_or("0")).unwrap_or(0.0);
    let r = parse_number(node.attribute("r")?)?;
    if r <= 0.0 {
        return None;
    }
    ellipse_to_path_impl(cx, cy, r, r)
}

fn ellipse_to_path(node: XmlNode<'_>) -> Option<Vec<PathSeg>> {
    let cx = parse_number(node.attribute("cx").unwrap_or("0")).unwrap_or(0.0);
    let cy = parse_number(node.attribute("cy").unwrap_or("0")).unwrap_or(0.0);
    let rx = parse_number(node.attribute("rx")?)?;
    let ry = parse_number(node.attribute("ry")?)?;
    if rx <= 0.0 || ry <= 0.0 {
        return None;
    }
    ellipse_to_path_impl(cx, cy, rx, ry)
}

fn ellipse_to_path_impl(cx: f32, cy: f32, rx: f32, ry: f32) -> Option<Vec<PathSeg>> {
    // Approximate with 4 cubic Beziers.
    let k = 0.5522847498f32;
    let ox = rx * k;
    let oy = ry * k;
    Some(vec![
        PathSeg::MoveTo(cx + rx, cy),
        PathSeg::CurveTo(cx + rx, cy + oy, cx + ox, cy + ry, cx, cy + ry),
        PathSeg::CurveTo(cx - ox, cy + ry, cx - rx, cy + oy, cx - rx, cy),
        PathSeg::CurveTo(cx - rx, cy - oy, cx - ox, cy - ry, cx, cy - ry),
        PathSeg::CurveTo(cx + ox, cy - ry, cx + rx, cy - oy, cx + rx, cy),
        PathSeg::Close,
    ])
}

fn line_to_path(node: XmlNode<'_>) -> Option<Vec<PathSeg>> {
    let x1 = parse_number(node.attribute("x1").unwrap_or("0")).unwrap_or(0.0);
    let y1 = parse_number(node.attribute("y1").unwrap_or("0")).unwrap_or(0.0);
    let x2 = parse_number(node.attribute("x2").unwrap_or("0")).unwrap_or(0.0);
    let y2 = parse_number(node.attribute("y2").unwrap_or("0")).unwrap_or(0.0);
    Some(vec![PathSeg::MoveTo(x1, y1), PathSeg::LineTo(x2, y2)])
}

fn poly_points_to_path(node: XmlNode<'_>, close: bool) -> Option<Vec<PathSeg>> {
    let pts = node.attribute("points")?;
    let points = parse_points(pts);
    if points.len() < 2 {
        return None;
    }
    let mut segs = Vec::new();
    segs.push(PathSeg::MoveTo(points[0].0, points[0].1));
    for (x, y) in points.into_iter().skip(1) {
        segs.push(PathSeg::LineTo(x, y));
    }
    if close {
        segs.push(PathSeg::Close);
    }
    Some(segs)
}

fn parse_points(input: &str) -> Vec<(f32, f32)> {
    let nums: Vec<f32> = input
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f32>().ok())
        .collect();
    let mut out = Vec::new();
    let mut it = nums.into_iter();
    while let (Some(x), Some(y)) = (it.next(), it.next()) {
        out.push((x, y));
    }
    out
}

fn parse_path_data(d: &str) -> Vec<PathSeg> {
    // Path parser covering common SVG 1.1 commands; we normalize quadratics/arcs to cubics.
    let mut segs = Vec::new();
    let mut p = PathParser::new(d);
    let mut cmd = ' ';
    let mut cur_x = 0.0;
    let mut cur_y = 0.0;
    let mut start_x = 0.0;
    let mut start_y = 0.0;
    let mut last_cubic_ctrl2: Option<(f32, f32)> = None;
    let mut last_quad_ctrl: Option<(f32, f32)> = None;

    while let Some(c) = p.next_command_or_number(&mut cmd) {
        match c {
            'M' | 'm' => {
                let rel = c == 'm';
                if let Some((x, y)) = p.next_pair() {
                    let (x, y) = if rel { (cur_x + x, cur_y + y) } else { (x, y) };
                    segs.push(PathSeg::MoveTo(x, y));
                    cur_x = x;
                    cur_y = y;
                    start_x = x;
                    start_y = y;
                    last_cubic_ctrl2 = None;
                    last_quad_ctrl = None;

                    // Implicit subsequent pairs are treated as LineTo.
                    while let Some((x2, y2)) = p.next_pair() {
                        let (x2, y2) = if rel {
                            (cur_x + x2, cur_y + y2)
                        } else {
                            (x2, y2)
                        };
                        segs.push(PathSeg::LineTo(x2, y2));
                        cur_x = x2;
                        cur_y = y2;
                    }
                }
            }
            'L' | 'l' => {
                let rel = c == 'l';
                while let Some((x, y)) = p.next_pair() {
                    let (x, y) = if rel { (cur_x + x, cur_y + y) } else { (x, y) };
                    segs.push(PathSeg::LineTo(x, y));
                    cur_x = x;
                    cur_y = y;
                }
                last_cubic_ctrl2 = None;
                last_quad_ctrl = None;
            }
            'H' | 'h' => {
                let rel = c == 'h';
                while let Some(x) = p.next_number() {
                    let x = if rel { cur_x + x } else { x };
                    segs.push(PathSeg::LineTo(x, cur_y));
                    cur_x = x;
                }
                last_cubic_ctrl2 = None;
                last_quad_ctrl = None;
            }
            'V' | 'v' => {
                let rel = c == 'v';
                while let Some(y) = p.next_number() {
                    let y = if rel { cur_y + y } else { y };
                    segs.push(PathSeg::LineTo(cur_x, y));
                    cur_y = y;
                }
                last_cubic_ctrl2 = None;
                last_quad_ctrl = None;
            }
            'C' | 'c' => {
                let rel = c == 'c';
                while let (Some(x1), Some(y1), Some(x2), Some(y2), Some(x), Some(y)) = (
                    p.next_number(),
                    p.next_number(),
                    p.next_number(),
                    p.next_number(),
                    p.next_number(),
                    p.next_number(),
                ) {
                    let (x1, y1, x2, y2, x, y) = if rel {
                        (
                            cur_x + x1,
                            cur_y + y1,
                            cur_x + x2,
                            cur_y + y2,
                            cur_x + x,
                            cur_y + y,
                        )
                    } else {
                        (x1, y1, x2, y2, x, y)
                    };
                    segs.push(PathSeg::CurveTo(x1, y1, x2, y2, x, y));
                    cur_x = x;
                    cur_y = y;
                    last_cubic_ctrl2 = Some((x2, y2));
                    last_quad_ctrl = None;
                }
            }
            'S' | 's' => {
                let rel = c == 's';
                while let (Some(x2), Some(y2), Some(x), Some(y)) = (
                    p.next_number(),
                    p.next_number(),
                    p.next_number(),
                    p.next_number(),
                ) {
                    let (x2, y2, x, y) = if rel {
                        (cur_x + x2, cur_y + y2, cur_x + x, cur_y + y)
                    } else {
                        (x2, y2, x, y)
                    };
                    let (x1, y1) = if let Some((px2, py2)) = last_cubic_ctrl2 {
                        (2.0 * cur_x - px2, 2.0 * cur_y - py2)
                    } else {
                        (cur_x, cur_y)
                    };
                    segs.push(PathSeg::CurveTo(x1, y1, x2, y2, x, y));
                    cur_x = x;
                    cur_y = y;
                    last_cubic_ctrl2 = Some((x2, y2));
                    last_quad_ctrl = None;
                }
            }
            'Q' | 'q' => {
                let rel = c == 'q';
                while let (Some(x1), Some(y1), Some(x), Some(y)) = (
                    p.next_number(),
                    p.next_number(),
                    p.next_number(),
                    p.next_number(),
                ) {
                    let (x1, y1, x, y) = if rel {
                        (cur_x + x1, cur_y + y1, cur_x + x, cur_y + y)
                    } else {
                        (x1, y1, x, y)
                    };
                    let (c1x, c1y, c2x, c2y) = quad_to_cubic(cur_x, cur_y, x1, y1, x, y);
                    segs.push(PathSeg::CurveTo(c1x, c1y, c2x, c2y, x, y));
                    cur_x = x;
                    cur_y = y;
                    last_quad_ctrl = Some((x1, y1));
                    last_cubic_ctrl2 = Some((c2x, c2y));
                }
            }
            'T' | 't' => {
                let rel = c == 't';
                while let Some((x, y)) = p.next_pair() {
                    let (x, y) = if rel { (cur_x + x, cur_y + y) } else { (x, y) };
                    let (qx, qy) = if let Some((px1, py1)) = last_quad_ctrl {
                        (2.0 * cur_x - px1, 2.0 * cur_y - py1)
                    } else {
                        (cur_x, cur_y)
                    };
                    let (c1x, c1y, c2x, c2y) = quad_to_cubic(cur_x, cur_y, qx, qy, x, y);
                    segs.push(PathSeg::CurveTo(c1x, c1y, c2x, c2y, x, y));
                    cur_x = x;
                    cur_y = y;
                    last_quad_ctrl = Some((qx, qy));
                    last_cubic_ctrl2 = Some((c2x, c2y));
                }
            }
            'A' | 'a' => {
                let rel = c == 'a';
                while let (
                    Some(rx),
                    Some(ry),
                    Some(rot),
                    Some(large),
                    Some(sweep),
                    Some(x),
                    Some(y),
                ) = (
                    p.next_number(),
                    p.next_number(),
                    p.next_number(),
                    p.next_arc_flag(),
                    p.next_arc_flag(),
                    p.next_number(),
                    p.next_number(),
                ) {
                    let (x, y) = if rel { (cur_x + x, cur_y + y) } else { (x, y) };
                    let large_arc = large.abs() > 0.5;
                    let sweep_flag = sweep.abs() > 0.5;
                    let curves =
                        arc_to_cubics(cur_x, cur_y, rx, ry, rot, large_arc, sweep_flag, x, y);
                    for seg in &curves {
                        segs.push(seg.clone());
                    }
                    cur_x = x;
                    cur_y = y;
                    // Best-effort: last cubic ctrl2 is the last segment's c2.
                    last_cubic_ctrl2 = curves.iter().rev().find_map(|seg| {
                        if let PathSeg::CurveTo(_, _, x2, y2, _, _) = *seg {
                            Some((x2, y2))
                        } else {
                            None
                        }
                    });
                    last_quad_ctrl = None;
                }
            }
            'Z' | 'z' => {
                segs.push(PathSeg::Close);
                cur_x = start_x;
                cur_y = start_y;
                last_cubic_ctrl2 = None;
                last_quad_ctrl = None;
            }
            _ => {}
        }
    }

    segs
}

pub(crate) fn parse_svg_path_data(d: &str) -> Vec<SvgPathSegment> {
    parse_path_data(d)
}

pub(crate) fn svg_arc_to_cubic_segments(
    x0: f32,
    y0: f32,
    rx: f32,
    ry: f32,
    rotation_deg: f32,
    large_arc: bool,
    sweep: bool,
    x1: f32,
    y1: f32,
) -> Vec<SvgPathSegment> {
    arc_to_cubics(x0, y0, rx, ry, rotation_deg, large_arc, sweep, x1, y1)
}

fn quad_to_cubic(x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> (f32, f32, f32, f32) {
    // Quadratic Bezier -> cubic Bezier controls.
    let c1x = x0 + (2.0 / 3.0) * (x1 - x0);
    let c1y = y0 + (2.0 / 3.0) * (y1 - y0);
    let c2x = x2 + (2.0 / 3.0) * (x1 - x2);
    let c2y = y2 + (2.0 / 3.0) * (y1 - y2);
    (c1x, c1y, c2x, c2y)
}

fn arc_to_cubics(
    x0: f32,
    y0: f32,
    rx_in: f32,
    ry_in: f32,
    x_axis_rotation_deg: f32,
    large_arc: bool,
    sweep: bool,
    x1: f32,
    y1: f32,
) -> Vec<PathSeg> {
    // SVG elliptical arc -> sequence of cubic Beziers.
    // Based on the SVG 1.1 implementation notes (center parameterization).
    use std::f32::consts::PI;

    let mut rx = rx_in.abs();
    let mut ry = ry_in.abs();
    if rx == 0.0 || ry == 0.0 || (x0 == x1 && y0 == y1) {
        return vec![PathSeg::LineTo(x1, y1)];
    }

    let phi = x_axis_rotation_deg.to_radians();
    let (sin_phi, cos_phi) = crate::math::sin_cos(phi);

    // Step 1: compute (x1', y1')
    let dx2 = (x0 - x1) / 2.0;
    let dy2 = (y0 - y1) / 2.0;
    let x1p = cos_phi * dx2 + sin_phi * dy2;
    let y1p = -sin_phi * dx2 + cos_phi * dy2;

    // Step 2: ensure radii are large enough
    let rx2 = rx * rx;
    let ry2 = ry * ry;
    let x1p2 = x1p * x1p;
    let y1p2 = y1p * y1p;
    let lambda = (x1p2 / rx2) + (y1p2 / ry2);
    if lambda > 1.0 {
        let s = crate::math::sqrt(lambda);
        rx *= s;
        ry *= s;
    }

    // Step 3: compute center (cx', cy')
    let rx2 = rx * rx;
    let ry2 = ry * ry;
    let num = rx2 * ry2 - rx2 * y1p2 - ry2 * x1p2;
    let den = rx2 * y1p2 + ry2 * x1p2;
    let mut coef = 0.0;
    if den != 0.0 {
        let sign = if large_arc == sweep { -1.0 } else { 1.0 };
        coef = sign * crate::math::sqrt((num / den).max(0.0));
    }
    let cxp = coef * (rx * y1p / ry);
    let cyp = coef * (-ry * x1p / rx);

    // Step 4: compute center (cx, cy)
    let cx = cos_phi * cxp - sin_phi * cyp + (x0 + x1) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (y0 + y1) / 2.0;

    // Step 5: compute angles
    fn angle(ux: f32, uy: f32, vx: f32, vy: f32) -> f32 {
        let dot = ux * vx + uy * vy;
        let det = ux * vy - uy * vx;
        crate::math::atan2(det, dot)
    }

    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;

    let mut theta1 = angle(1.0, 0.0, ux, uy);
    let mut dtheta = angle(ux, uy, vx, vy);

    if !sweep && dtheta > 0.0 {
        dtheta -= 2.0 * PI;
    } else if sweep && dtheta < 0.0 {
        dtheta += 2.0 * PI;
    }

    // Split into <= 90deg segments.
    let segs_count = crate::math::ceil(dtheta.abs() / (PI / 2.0)).max(1.0) as i32;
    let delta = dtheta / (segs_count as f32);

    let mut out = Vec::new();
    for _ in 0..segs_count {
        let t1 = theta1;
        let t2 = theta1 + delta;
        out.push(arc_segment_to_cubic(
            cx, cy, rx, ry, sin_phi, cos_phi, t1, t2,
        ));
        theta1 = t2;
    }

    // Flatten nested vectors.
    let mut flat = Vec::new();
    for (c1x, c1y, c2x, c2y, ex, ey) in out {
        flat.push(PathSeg::CurveTo(c1x, c1y, c2x, c2y, ex, ey));
    }
    flat
}

fn arc_segment_to_cubic(
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    sin_phi: f32,
    cos_phi: f32,
    t1: f32,
    t2: f32,
) -> (f32, f32, f32, f32, f32, f32) {
    // Convert an ellipse arc segment t1..t2 into a cubic Bezier.
    let dt = t2 - t1;
    let k = (4.0 / 3.0) * crate::math::tan(dt / 4.0);

    let (s1, c1) = crate::math::sin_cos(t1);
    let (s2, c2) = crate::math::sin_cos(t2);

    // Unit circle control points
    let p1x = c1 - k * s1;
    let p1y = s1 + k * c1;
    let p2x = c2 + k * s2;
    let p2y = s2 - k * c2;
    let p3x = c2;
    let p3y = s2;

    // Map unit circle -> ellipse -> rotate -> translate.
    fn map(
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        sin_phi: f32,
        cos_phi: f32,
        x: f32,
        y: f32,
    ) -> (f32, f32) {
        let x = rx * x;
        let y = ry * y;
        let xp = cos_phi * x - sin_phi * y;
        let yp = sin_phi * x + cos_phi * y;
        (cx + xp, cy + yp)
    }

    let (c1x, c1y) = map(cx, cy, rx, ry, sin_phi, cos_phi, p1x, p1y);
    let (c2x, c2y) = map(cx, cy, rx, ry, sin_phi, cos_phi, p2x, p2y);
    let (ex, ey) = map(cx, cy, rx, ry, sin_phi, cos_phi, p3x, p3y);

    (c1x, c1y, c2x, c2y, ex, ey)
}

struct PathParser<'a> {
    bytes: &'a [u8],
    i: usize,
}

impl<'a> PathParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            bytes: input.as_bytes(),
            i: 0,
        }
    }

    fn skip_ws(&mut self) {
        while self.i < self.bytes.len() {
            let b = self.bytes[self.i];
            if b == b' ' || b == b'\n' || b == b'\r' || b == b'\t' || b == b',' {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn next_command_or_number(&mut self, current: &mut char) -> Option<char> {
        self.skip_ws();
        if self.i >= self.bytes.len() {
            return None;
        }
        let b = self.bytes[self.i];
        let c = b as char;
        if c.is_ascii_alphabetic() {
            *current = c;
            self.i += 1;
            return Some(c);
        }
        // No new command; reuse previous.
        Some(*current)
    }

    fn next_number(&mut self) -> Option<f32> {
        self.skip_ws();
        if self.i >= self.bytes.len() {
            return None;
        }
        let start = self.i;
        let mut has = false;

        if matches!(self.bytes[self.i], b'+' | b'-') {
            self.i += 1;
        }
        while self.i < self.bytes.len() && self.bytes[self.i].is_ascii_digit() {
            self.i += 1;
            has = true;
        }
        if self.i < self.bytes.len() && self.bytes[self.i] == b'.' {
            self.i += 1;
            while self.i < self.bytes.len() && self.bytes[self.i].is_ascii_digit() {
                self.i += 1;
                has = true;
            }
        }
        if self.i < self.bytes.len() && matches!(self.bytes[self.i], b'e' | b'E') {
            self.i += 1;
            if self.i < self.bytes.len() && matches!(self.bytes[self.i], b'+' | b'-') {
                self.i += 1;
            }
            while self.i < self.bytes.len() && self.bytes[self.i].is_ascii_digit() {
                self.i += 1;
                has = true;
            }
        }

        if !has {
            self.i = start;
            return None;
        }

        let s = std::str::from_utf8(&self.bytes[start..self.i]).ok()?;
        s.parse::<f32>().ok()
    }

    fn next_arc_flag(&mut self) -> Option<f32> {
        self.skip_ws();
        if self.i >= self.bytes.len() {
            return None;
        }
        match self.bytes[self.i] {
            b'0' => {
                self.i += 1;
                Some(0.0)
            }
            b'1' => {
                self.i += 1;
                Some(1.0)
            }
            _ => self
                .next_number()
                .map(|v| if v.abs() > 0.5 { 1.0 } else { 0.0 }),
        }
    }

    fn next_pair(&mut self) -> Option<(f32, f32)> {
        let x = self.next_number()?;
        let y = self.next_number()?;
        Some((x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Command;
    use crate::image_native::{encode_png_rgba8, load_from_memory};
    use crate::raster::document_to_png_pages;
    use crate::types::Size;

    fn rasterize_native_svg(svg: &str, width: f32, height: f32) -> crate::image_native::RgbaImage {
        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(width),
            height: Pt::from_f32(height),
        });
        render_svg_to_canvas(
            svg,
            &mut canvas,
            Pt::ZERO,
            Pt::ZERO,
            Pt::from_f32(width),
            Pt::from_f32(height),
        );
        let document = canvas.finish();
        let png = document_to_png_pages(&document, 72, None, true)
            .expect("native SVG should rasterize")
            .remove(0);
        load_from_memory(&png)
            .expect("raster output should decode")
            .into_rgba8()
    }

    fn path_bounds(path: &CompiledPath) -> (f32, f32, f32, f32) {
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for segment in &path.segs {
            match *segment {
                PathSeg::MoveTo(x, y) | PathSeg::LineTo(x, y) => {
                    xs.push(x);
                    ys.push(y);
                }
                PathSeg::CurveTo(x1, y1, x2, y2, x3, y3) => {
                    xs.extend([x1, x2, x3]);
                    ys.extend([y1, y2, y3]);
                }
                PathSeg::Close => {}
            }
        }
        (
            xs.iter().copied().fold(f32::INFINITY, f32::min),
            ys.iter().copied().fold(f32::INFINITY, f32::min),
            xs.iter().copied().fold(f32::NEG_INFINITY, f32::max),
            ys.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        )
    }

    #[test]
    fn intrinsic_svg_viewport_scales_when_embedded_without_a_viewbox() {
        let svg = r##"<svg width="280" height="220"><rect width="280" height="220"/><rect x="40" y="40" width="200" height="140"/></svg>"##;
        let compiled = compile_svg(svg, Pt::from_f32(224.0), Pt::from_f32(176.0));
        let paths = compiled
            .iter()
            .filter_map(|item| match item {
                CompiledItem::Path(path) => Some(path),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(path_bounds(paths[0]), (0.0, 0.0, 224.0, 176.0));
        assert_eq!(path_bounds(paths[1]), (32.0, 32.0, 192.0, 144.0));
    }

    #[test]
    fn root_preserve_aspect_ratio_slice_uses_cover_scaling() {
        let svg = r##"<svg viewBox="0 0 120 60" preserveAspectRatio="xMidYMid slice"><rect width="60" height="60"/><rect x="60" width="60" height="60"/></svg>"##;
        let compiled = compile_svg(svg, Pt::from_f32(90.0), Pt::from_f32(90.0));
        let paths = compiled
            .iter()
            .filter_map(|item| match item {
                CompiledItem::Path(path) => Some(path),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(path_bounds(paths[0]), (-45.0, 0.0, 45.0, 90.0));
        assert_eq!(path_bounds(paths[1]), (45.0, 0.0, 135.0, 90.0));
    }

    #[test]
    fn parses_simple_path() {
        let segs = parse_path_data("M 0 0 L 10 0 L 10 10 Z");
        assert!(!segs.is_empty());
        assert!(matches!(segs[0], PathSeg::MoveTo(_, _)));
    }

    #[test]
    fn parses_quadratic_and_arc() {
        let segs = parse_path_data("M 0 0 Q 10 0 10 10 T 20 20 A 5 5 0 0 1 30 30 Z");
        assert!(!segs.is_empty());
        // Quadratic and arc both normalize to cubic CurveTo segments.
        assert!(segs.iter().any(|s| matches!(s, PathSeg::CurveTo(..))));
    }

    #[test]
    fn parses_compact_arc_flags_without_separator() {
        let segs = parse_path_data("M10 10 A5 5 0 01 20 20");
        assert!(
            segs.iter().any(|s| matches!(s, PathSeg::CurveTo(..))),
            "compact arc flag syntax should produce cubic segments"
        );
    }

    #[test]
    fn presentation_attributes_accept_short_hex_colors() {
        let svg = r##"<svg viewBox="0 0 4 4"><rect width="4" height="4" fill="#fff"/></svg>"##;
        let compiled = compile_svg(svg, Pt::from_f32(4.0), Pt::from_f32(4.0));
        let fill = compiled
            .iter()
            .find_map(|item| match item {
                CompiledItem::Path(path) => path.style.fill.color,
                _ => None,
            })
            .expect("rectangle fill");
        assert_eq!(fill, Color::rgb(1.0, 1.0, 1.0));
    }

    #[test]
    fn svg_stylesheet_class_rules_apply_to_shapes() {
        let svg = r##"
        <svg width="220" height="120" viewBox="0 0 220 120">
          <style>
            .bg { fill: #6f85ff; }
            .dot { fill: #9ce2c8; }
            .tri { fill: #202f5f; stroke: #ffffff; stroke-width: 2; }
          </style>
          <rect class="bg" x="8" y="8" width="204" height="104" rx="10" />
          <circle class="dot" cx="56" cy="60" r="24" />
          <path class="tri" d="M96 82 L118 34 L140 82 Z" />
        </svg>
        "##;
        let compiled = compile_svg(svg, Pt::from_f32(220.0), Pt::from_f32(120.0));
        assert!(
            !compiled.is_empty(),
            "expected compiled output from class-based stylesheet"
        );
        let mut had_bg = false;
        let mut had_tri_stroke = false;
        for item in &compiled {
            let CompiledItem::Path(path) = item else {
                continue;
            };
            if let Some(fill) = path.style.fill.color {
                if (fill.r - (111.0 / 255.0)).abs() < 0.01
                    && (fill.g - (133.0 / 255.0)).abs() < 0.01
                    && (fill.b - 1.0).abs() < 0.01
                {
                    had_bg = true;
                }
            }
            if let Some(stroke) = path.style.stroke.color {
                if (stroke.r - 1.0).abs() < 0.01
                    && (stroke.g - 1.0).abs() < 0.01
                    && (stroke.b - 1.0).abs() < 0.01
                    && (path.style.stroke_width - 2.0).abs() < 0.01
                {
                    had_tri_stroke = true;
                }
            }
        }
        assert!(had_bg, "expected stylesheet fill to apply to .bg shape");
        assert!(
            had_tri_stroke,
            "expected stylesheet stroke to apply to .tri shape"
        );
    }

    #[test]
    fn svg_stylesheet_descendant_rules_apply_to_nested_nodes() {
        let svg = r##"
        <svg width="40" height="20" viewBox="0 0 40 20">
          <style>.group .dot { fill: #00ff00; }</style>
          <g class="group"><circle class="dot" cx="10" cy="10" r="8" /></g>
        </svg>
        "##;
        let compiled = compile_svg(svg, Pt::from_f32(40.0), Pt::from_f32(20.0));
        let path = compiled
            .iter()
            .find_map(|item| match item {
                CompiledItem::Path(path) => Some(path),
                _ => None,
            })
            .expect("expected compiled path");
        let fill = path.style.fill.color.expect("expected fill color");
        assert!((fill.r - 0.0).abs() < 0.01);
        assert!((fill.g - 1.0).abs() < 0.01);
        assert!((fill.b - 0.0).abs() < 0.01);
    }

    #[test]
    fn svg_stylesheet_important_beats_later_non_important() {
        let svg = r##"
        <svg width="20" height="10" viewBox="0 0 20 10">
          <style>
            .strong { fill: #ff0000 !important; }
            .weak { fill: #0000ff; }
          </style>
          <rect class="strong weak" x="0" y="0" width="20" height="10" />
        </svg>
        "##;
        let compiled = compile_svg(svg, Pt::from_f32(20.0), Pt::from_f32(10.0));
        let path = compiled
            .iter()
            .find_map(|item| match item {
                CompiledItem::Path(path) => Some(path),
                _ => None,
            })
            .expect("expected compiled path");
        let fill = path.style.fill.color.expect("expected fill color");
        assert!((fill.r - 1.0).abs() < 0.01);
        assert!((fill.g - 0.0).abs() < 0.01);
        assert!((fill.b - 0.0).abs() < 0.01);
    }

    #[test]
    fn svg_supported_features_do_not_force_raster_fallback() {
        let svg = r##"
        <svg width="20" height="10" viewBox="0 0 20 10">
          <style>.x { fill: #ff0000; }</style>
          <path class="x" d="M1 1 A4 4 0 01 9 9" />
        </svg>
        "##;
        assert!(
            !svg_needs_raster_fallback(svg),
            "style/arc-only SVG should stay on vector path"
        );
    }

    #[test]
    fn native_text_symbol_and_affine_image_features_do_not_force_fallback() {
        let svg = r##"
        <svg width="80" height="40" viewBox="0 0 80 40">
          <defs><symbol id="s" viewBox="0 0 10 10"><rect width="10" height="10" /></symbol></defs>
          <use href="#s" width="20" height="20" />
          <text x="2" y="34">native</text>
          <image x="40" y="4" width="10" height="10"
                 href="data:image/png;base64,iVBORw0KGgo=" transform="rotate(12 45 9)" />
          <foreignObject x="0" y="0" width="1" height="1"><div>ignored</div></foreignObject>
        </svg>
        "##;
        assert!(
            !svg_needs_raster_fallback(svg),
            "native features and fallback-equivalent foreignObject handling should stay native"
        );
    }

    #[test]
    fn renders_svg_without_panic() {
        let svg = r##"<svg viewBox="0 0 10 10"><rect x="1" y="1" width="8" height="8" fill="#ff0000"/></svg>"##;
        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(100.0),
            height: Pt::from_f32(100.0),
        });
        render_svg_to_canvas(
            svg,
            &mut canvas,
            Pt::ZERO,
            Pt::ZERO,
            Pt::from_f32(50.0),
            Pt::from_f32(50.0),
        );
    }

    #[test]
    fn gradients_compile_to_shading() {
        let svg = r##"
        <svg width="10" height="10" viewBox="0 0 10 10">
          <defs>
            <linearGradient id="g1">
              <stop offset="0" stop-color="#00ff00"/>
              <stop offset="1" stop-color="#0000ff"/>
            </linearGradient>
          </defs>
          <rect x="0" y="0" width="10" height="10" fill="url(#g1)"/>
        </svg>
        "##;
        let compiled = compile_svg(svg, Pt::from_f32(10.0), Pt::from_f32(10.0));
        assert!(!compiled.is_empty());
        let first_path = compiled
            .iter()
            .find_map(|it| match it {
                CompiledItem::Path(p) => Some(p),
                _ => None,
            })
            .expect("expected at least one path");
        assert!(first_path.style.fill_shading.is_some());
    }

    #[test]
    fn use_references_defs_by_id() {
        let svg = r##"
        <svg width="40" height="20" viewBox="0 0 40 20">
          <defs>
            <g id="icon">
              <rect x="0" y="0" width="10" height="10" fill="#ff0000"/>
            </g>
          </defs>
          <use href="#icon" x="2" y="2"/>
          <use href="#icon" x="20" y="2" transform="scale(1.0)"/>
        </svg>
        "##;
        let compiled = compile_svg(svg, Pt::from_f32(40.0), Pt::from_f32(20.0));
        assert!(!compiled.is_empty());
    }

    #[test]
    fn text_and_tspan_compile_in_mixed_content_order() {
        let svg = r##"
        <svg width="120" height="40" viewBox="0 0 120 40">
          <style>.label { font-family: Arial, sans-serif; font-size: 20px; }</style>
          <text class="label" x="4" y="28" fill="#cc0000">A<tspan fill="#0000cc" font-weight="bold">B</tspan>C</text>
        </svg>
        "##;
        let compiled = compile_svg(svg, Pt::from_f32(120.0), Pt::from_f32(40.0));
        let texts = compiled
            .iter()
            .filter_map(|item| match item {
                CompiledItem::Text(text) => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            texts
                .iter()
                .map(|text| text.text.as_str())
                .collect::<Vec<_>>(),
            ["A", "B", "C"]
        );
        assert_eq!(texts[0].font_name, "Helvetica");
        assert_eq!(texts[1].font_name, "Helvetica-Bold");
        assert!((texts[0].font_size - 20.0).abs() < 0.01);
        assert!(texts[0].x < texts[1].x && texts[1].x < texts[2].x);
        assert!(texts[0].fill.r > 0.7 && texts[0].fill.b < 0.1);
        assert!(texts[1].fill.b > 0.7 && texts[1].fill.r < 0.1);
    }

    #[test]
    fn symbol_use_maps_viewbox_into_requested_viewport() {
        let svg = r##"
        <svg width="100" height="50" viewBox="0 0 100 50">
          <defs>
            <symbol id="tile" viewBox="0 0 10 10">
              <rect x="0" y="0" width="10" height="10" fill="#00aa00" />
            </symbol>
          </defs>
          <use href="#tile" x="8" y="6" width="40" height="20" />
        </svg>
        "##;
        let compiled = compile_svg(svg, Pt::from_f32(100.0), Pt::from_f32(50.0));
        let path = compiled
            .iter()
            .find_map(|item| match item {
                CompiledItem::Path(path) => Some(path),
                _ => None,
            })
            .expect("symbol should compile through use");
        let bounds = bbox_of_segs(&path.segs).expect("symbol path bounds");
        assert!((bounds.0 - 18.0).abs() < 0.01);
        assert!((bounds.1 - 6.0).abs() < 0.01);
        assert!((bounds.2 - 20.0).abs() < 0.01);
        assert!((bounds.3 - 20.0).abs() < 0.01);
    }

    #[test]
    fn transformed_images_and_text_emit_affine_canvas_commands() {
        let svg = r##"
        <svg width="80" height="50" viewBox="0 0 80 50">
          <image x="10" y="8" width="12" height="10"
                 href="data:image/png;base64,iVBORw0KGgo="
                 transform="rotate(20 16 13)" />
          <text x="4" y="42" font-size="12">ok</text>
        </svg>
        "##;
        let compiled = compile_svg(svg, Pt::from_f32(80.0), Pt::from_f32(50.0));
        assert!(compiled.iter().any(|item| {
            matches!(item, CompiledItem::Image(image) if image.transform.is_some())
        }));
        assert!(
            compiled
                .iter()
                .any(|item| matches!(item, CompiledItem::Text(_)))
        );

        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(100.0),
            height: Pt::from_f32(70.0),
        });
        render_compiled_items(&compiled, &mut canvas, Pt::from_f32(5.0), Pt::from_f32(7.0));
        let document = canvas.finish();
        let commands = &document.pages[0].commands;
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, Command::DrawImage { .. }))
        );
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, Command::DrawString { text, .. } if text == "ok"))
        );
        assert!(
            commands
                .iter()
                .filter(|command| matches!(command, Command::ConcatMatrix { .. }))
                .count()
                >= 2
        );
        assert!(commands.iter().any(|command| {
            matches!(
                command,
                Command::ConcatMatrix { a, b, c, d, e, f }
                    if (*a - 1.0).abs() < 0.001
                        && b.abs() < 0.001
                        && c.abs() < 0.001
                        && (*d - 1.0).abs() < 0.001
                        && (*e - Pt::from_f32(5.0)).abs() < Pt::from_f32(0.001)
                        && (*f - Pt::from_f32(7.0)).abs() < Pt::from_f32(0.001)
            )
        }));
    }

    #[test]
    fn scaled_svg_text_matrix_carries_the_absolute_page_origin_phase() {
        let svg = r##"
        <svg width="80" height="50" viewBox="0 0 80 50">
          <text x="4" y="42" font-size="12">ok</text>
        </svg>
        "##;
        let compiled = compile_svg(svg, Pt::from_f32(40.0), Pt::from_f32(25.0));
        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(100.0),
            height: Pt::from_f32(70.0),
        });
        render_compiled_items(&compiled, &mut canvas, Pt::from_f32(5.0), Pt::from_f32(7.0));
        let document = canvas.finish();

        assert!(document.pages[0].commands.iter().any(|command| {
            matches!(
                command,
                Command::ConcatMatrix { a, b, c, d, e, f }
                    if (*a - 0.5).abs() < 0.001
                        && b.abs() < 0.001
                        && c.abs() < 0.001
                        && (*d - 0.5).abs() < 0.001
                        && (*e - Pt::from_f32(5.0)).abs() < Pt::from_f32(0.001)
                        && (*f + Pt::from_f32(28.0)).abs() < Pt::from_f32(0.001)
            )
        }));
    }

    #[test]
    fn native_text_rasterization_paints_requested_fill() {
        let svg = r##"
        <svg width="120" height="48" viewBox="0 0 120 48">
          <text x="5" y="36" font-family="Arial, sans-serif" font-size="32" fill="#cc0000">SVG</text>
        </svg>
        "##;
        let image = rasterize_native_svg(svg, 120.0, 48.0);
        let red_pixels = image
            .pixels()
            .filter(|pixel| pixel[0] > 150 && pixel[1] < 90 && pixel[2] < 90 && pixel[3] > 200)
            .count();
        assert!(
            red_pixels > 80,
            "expected native SVG text pixels, got {red_pixels}"
        );
    }

    #[test]
    fn native_symbol_and_transformed_image_rasterization_paint_pixels() {
        let source =
            encode_png_rgba8(&[220, 0, 0, 255].repeat(16), 4, 4).expect("test PNG should encode");
        let uri = format!(
            "data:image/png;base64,{}",
            crate::base64::encode_standard(&source)
        );
        let svg = format!(
            r##"<svg width="100" height="60" viewBox="0 0 100 60">
              <defs>
                <symbol id="s" viewBox="0 0 10 10">
                  <circle cx="5" cy="5" r="5" fill="#008800" />
                </symbol>
              </defs>
              <use href="#s" x="4" y="4" width="28" height="28" />
              <image x="50" y="12" width="20" height="16" href="{uri}"
                     transform="rotate(25 60 20)" />
            </svg>"##
        );
        let image = rasterize_native_svg(&svg, 100.0, 60.0);
        let green_pixels = image
            .pixels()
            .filter(|pixel| pixel[1] > 80 && pixel[0] < 80 && pixel[2] < 80)
            .count();
        let red_pixels = image
            .pixels()
            .filter(|pixel| pixel[0] > 140 && pixel[1] < 80 && pixel[2] < 80)
            .count();
        assert!(
            green_pixels > 200,
            "expected symbol pixels, got {green_pixels}"
        );
        assert!(
            red_pixels > 150,
            "expected transformed image pixels, got {red_pixels}"
        );
    }

    #[test]
    fn gaussian_filter_compiles_to_native_filtered_group_and_expands_pixels() {
        let svg = r##"
        <svg width="90" height="50" viewBox="0 0 90 50">
          <defs><filter id="blur"><feGaussianBlur stdDeviation="3" /></filter></defs>
          <rect x="25" y="15" width="40" height="20" fill="#cc0000" filter="url(#blur)" />
        </svg>
        "##;
        let compiled = compile_svg(svg, Pt::from_f32(90.0), Pt::from_f32(50.0));
        assert!(compiled.iter().any(|item| {
            matches!(
                item,
                CompiledItem::Group(CompiledGroup {
                    filter: Some(filter),
                    ..
                }) if filter.blur_radius >= Pt::from_f32(2.9)
            )
        }));
        let image = rasterize_native_svg(svg, 90.0, 50.0);
        let painted = image
            .enumerate_pixels()
            .filter(|(_, _, pixel)| pixel[0] > 30 && pixel[1] < 245 && pixel[2] < 245)
            .map(|(x, y, _)| (x, y))
            .collect::<Vec<_>>();
        assert!(!painted.is_empty());
        assert!(painted.iter().any(|(x, _)| *x < 25));
        assert!(painted.iter().any(|(x, _)| *x > 64));
    }

    #[test]
    fn luminance_mask_compiles_to_clip_with_transparent_hole() {
        let svg = r##"
        <svg width="80" height="50" viewBox="0 0 80 50">
          <defs>
            <mask id="cutout">
              <rect x="5" y="5" width="70" height="40" fill="white" />
              <circle cx="40" cy="25" r="10" fill="black" />
            </mask>
          </defs>
          <rect x="5" y="5" width="70" height="40" fill="#0044cc" mask="url(#cutout)" />
        </svg>
        "##;
        let compiled = compile_svg(svg, Pt::from_f32(80.0), Pt::from_f32(50.0));
        assert!(compiled.iter().any(|item| {
            matches!(
                item,
                CompiledItem::Group(CompiledGroup {
                    mask: Some(mask),
                    ..
                }) if mask.paints_anything && mask.evenodd
            )
        }));
        let image = rasterize_native_svg(svg, 80.0, 50.0);
        let painted = image.get_pixel(10, 10);
        let hole = image.get_pixel(40, 25);
        assert!(painted[2] > 150 && painted[0] < 80);
        assert!(hole[0] > 245 && hole[1] > 245 && hole[2] > 245);
    }

    #[test]
    fn path_markers_compile_and_rasterize_with_auto_orientation() {
        let svg = r##"
        <svg width="90" height="60" viewBox="0 0 90 60">
          <defs>
            <marker id="arrow" markerWidth="8" markerHeight="8" refX="8" refY="4"
                    markerUnits="userSpaceOnUse" orient="auto" viewBox="0 0 8 8">
              <path d="M0 0 L8 4 L0 8 Z" fill="#cc0000" />
            </marker>
          </defs>
          <polyline points="10,45 42,15 76,38" fill="none" stroke="#003399"
                    stroke-width="3" marker-start="url(#arrow)" marker-mid="url(#arrow)"
                    marker-end="url(#arrow)" />
        </svg>
        "##;
        let compiled = compile_svg(svg, Pt::from_f32(90.0), Pt::from_f32(60.0));
        assert!(
            compiled
                .iter()
                .filter(|item| matches!(item, CompiledItem::Path(_)))
                .count()
                >= 4
        );
        let image = rasterize_native_svg(svg, 90.0, 60.0);
        let red_pixels = image
            .pixels()
            .filter(|pixel| pixel[0] > 140 && pixel[1] < 80 && pixel[2] < 80)
            .count();
        let blue_pixels = image
            .pixels()
            .filter(|pixel| pixel[2] > 100 && pixel[0] < 80 && pixel[1] < 120)
            .count();
        assert!(red_pixels > 45, "expected marker pixels, got {red_pixels}");
        assert!(
            blue_pixels > 80,
            "expected stroked path pixels, got {blue_pixels}"
        );
    }

    #[test]
    fn user_space_pattern_tiles_are_clipped_to_target_geometry() {
        let svg = r##"
        <svg width="96" height="52" viewBox="0 0 96 52">
          <defs>
            <pattern id="checker" patternUnits="userSpaceOnUse" width="10" height="10">
              <rect x="0" y="0" width="5" height="5" fill="#cc0000" />
              <rect x="5" y="5" width="5" height="5" fill="#cc0000" />
            </pattern>
          </defs>
          <rect x="3" y="4" width="90" height="44" rx="8" fill="url(#checker)" />
        </svg>
        "##;
        let compiled = compile_svg(svg, Pt::from_f32(96.0), Pt::from_f32(52.0));
        assert!(compiled.iter().any(|item| {
            matches!(
                item,
                CompiledItem::Group(CompiledGroup {
                    filter: None,
                    mask: Some(_),
                    ..
                })
            )
        }));
        let image = rasterize_native_svg(svg, 96.0, 52.0);
        let red_pixels = image
            .pixels()
            .filter(|pixel| pixel[0] > 140 && pixel[1] < 80 && pixel[2] < 80)
            .count();
        assert!(
            red_pixels > 1_400,
            "expected tiled pattern pixels, got {red_pixels}"
        );
        let outside = image.get_pixel(1, 1);
        let gap = image.get_pixel(7, 11);
        assert!(outside[0] > 245 && outside[1] > 245 && outside[2] > 245);
        assert!(gap[0] > 245 && gap[1] > 245 && gap[2] > 245);
    }

    #[test]
    fn style_attribute_overrides_presentation_attributes() {
        let svg = r##"
        <svg width="20" height="10" viewBox="0 0 20 10">
          <rect
            x="1"
            y="1"
            width="18"
            height="8"
            fill="#ff0000"
            style="fill:#0000ff; stroke:#00ff00; stroke-width:2; stroke-linecap:round; stroke-linejoin:bevel;"
          />
        </svg>
        "##;
        let compiled = compile_svg(svg, Pt::from_f32(20.0), Pt::from_f32(10.0));
        let path = compiled
            .iter()
            .find_map(|item| match item {
                CompiledItem::Path(path) => Some(path),
                _ => None,
            })
            .expect("expected compiled path");
        let fill = path.style.fill.color.expect("expected fill color");
        let stroke = path.style.stroke.color.expect("expected stroke color");
        assert!((fill.r - 0.0).abs() < 0.001);
        assert!((fill.g - 0.0).abs() < 0.001);
        assert!((fill.b - 1.0).abs() < 0.001);
        assert!((stroke.r - 0.0).abs() < 0.001);
        assert!((stroke.g - 1.0).abs() < 0.001);
        assert!((stroke.b - 0.0).abs() < 0.001);
        assert!((path.style.stroke_width - 2.0).abs() < 0.001);
        assert_eq!(path.style.line_cap, 1);
        assert_eq!(path.style.line_join, 2);
    }

    #[test]
    fn typed_svg_inline_style_parses_important_and_opacity() {
        let svg = r##"
        <svg width="24" height="12" viewBox="0 0 24 12">
          <rect
            x="1"
            y="1"
            width="22"
            height="10"
            style="fill: rgba(255, 0, 0, 0.5); stroke: rgba(0, 128, 0, 0.25); stroke-width: 2px; stroke-dasharray: 2 3 4; fill-rule: evenodd; opacity: 0.5; fill-opacity: 0.8; stroke-opacity: 0.5; stroke-linecap: butt; stroke-linecap: round !important; stroke-linejoin: bevel;"
          />
        </svg>
        "##;
        let compiled = compile_svg(svg, Pt::from_f32(24.0), Pt::from_f32(12.0));
        let path = compiled
            .iter()
            .find_map(|item| match item {
                CompiledItem::Path(path) => Some(path),
                _ => None,
            })
            .expect("expected compiled path");
        let fill = path.style.fill.color.expect("expected fill color");
        let stroke = path.style.stroke.color.expect("expected stroke color");
        assert!((fill.r - 1.0).abs() < 0.001);
        assert!((fill.g - 0.0).abs() < 0.001);
        assert!((fill.b - 0.0).abs() < 0.001);
        assert!((stroke.r - 0.0).abs() < 0.001);
        assert!((stroke.g - (128.0 / 255.0)).abs() < 0.001);
        assert!((stroke.b - 0.0).abs() < 0.001);
        assert!((path.style.stroke_width - 2.0).abs() < 0.001);
        assert_eq!(path.style.line_cap, 1);
        assert_eq!(path.style.line_join, 2);
        assert!(path.style.fill_rule_evenodd);
        assert_eq!(path.style.dash_pattern, vec![2.0, 3.0, 4.0, 2.0, 3.0, 4.0]);
        assert!((path.style.fill_opacity - 0.2).abs() < 0.001);
        assert!((path.style.stroke_opacity - 0.0625).abs() < 0.001);
    }
}
