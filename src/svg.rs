use crate::Canvas;
use crate::types::{Color, Pt};
use lightningcss::printer::PrinterOptions;
use lightningcss::properties::Property;
use lightningcss::properties::svg::{
    SVGPaint, SVGPaintFallback, StrokeDasharray, StrokeLinecap, StrokeLinejoin,
};
use lightningcss::rules::CssRule;
use lightningcss::stylesheet::{ParserOptions, StyleAttribute, StyleSheet};
use lightningcss::traits::ToCss;
use lightningcss::values::alpha::AlphaValue;
use lightningcss::values::color::{CssColor, SRGB};
use lightningcss::values::shape::FillRule;

// Lightweight detection for SVG features that our vector compiler does not support yet.
// When present, we can optionally fall back to rasterization.
pub(crate) fn svg_needs_raster_fallback(svg_xml: &str) -> bool {
    let Ok(doc) = roxmltree::Document::parse(svg_xml) else {
        return false;
    };

    for node in doc.descendants().filter(|n| n.is_element()) {
        let name = node.tag_name().name();
        match name {
            // Text and HTML-in-SVG are not supported in our vector subset.
            "text" | "foreignObject" => return true,
            // Filter/mask pipelines are raster-only for us.
            "filter" | "mask" => return true,
            // Pattern/marker/symbol are not implemented in our subset.
            "pattern" | "marker" | "symbol" => return true,
            _ => {}
        }

        if node.attribute("mask").is_some() || node.attribute("filter").is_some() {
            return true;
        }

        if name == "image" {
            if let Some(transform) = node.attribute("transform") {
                // Any rotation/skew/matrix on <image> would require a general image matrix draw.
                let t = transform.to_ascii_lowercase();
                if t.contains("rotate") || t.contains("skew") || t.contains("matrix") {
                    return true;
                }
            }
        }
    }

    false
}

#[cfg(feature = "svg_raster")]
pub(crate) fn rasterize_svg_to_data_uri(svg_xml: &str, width: Pt, height: Pt) -> Option<String> {
    use base64::Engine;
    use image::ColorType;
    use image::codecs::png::PngEncoder;
    use resvg::{tiny_skia, usvg};

    let mut opt = usvg::Options::default();
    opt.keep_named_groups = false;

    let tree = usvg::Tree::from_str(svg_xml, &opt).ok()?;

    let mut w = width.to_f32().round().max(1.0) as u32;
    let mut h = height.to_f32().round().max(1.0) as u32;
    if w == 0 || h == 0 {
        let size = tree.size();
        w = size.width().ceil().max(1.0) as u32;
        h = size.height().ceil().max(1.0) as u32;
    }

    let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
    resvg::render(&tree, usvg::FitTo::Size(w, h), pixmap.as_mut())?;

    let data = pixmap.data().to_vec();
    let mut png = Vec::new();
    let encoder = PngEncoder::new(&mut png);
    use image::ImageEncoder;
    encoder
        .write_image(&data, w, h, ColorType::Rgba8.into())
        .ok()?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
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
// Not supported (yet):
// - <text>, <clipPath>, <mask>, <filter>, <foreignObject>, arcs (A/a), gradients

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
        let s = libm::sinf(rad);
        let c = libm::cosf(rad);
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
        libm::sqrtf(det.abs()).max(0.0)
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
    declarations: String,
    order: usize,
}

#[derive(Debug, Clone, Default)]
struct SvgStylesheet {
    rules: Vec<SvgCssRule>,
}

#[derive(Debug, Clone)]
enum PathSeg {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    CurveTo(f32, f32, f32, f32, f32, f32),
    Close,
}

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
}

#[derive(Debug, Clone)]
pub(crate) enum CompiledItem {
    Path(CompiledPath),
    Image(CompiledImage),
}

pub(crate) fn compile_svg(svg_xml: &str, width: Pt, height: Pt) -> Vec<CompiledItem> {
    let Ok(doc) = roxmltree::Document::parse(svg_xml) else {
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
    let view_box = parse_viewbox(root.attribute("viewBox"));
    let viewport = viewbox_to_viewport_matrix(view_box, width.to_f32(), height.to_f32());
    let base = viewport;

    let style = SvgStyle::default();
    let mut out = Vec::new();
    compile_element(
        &mut out,
        root,
        base,
        &style,
        &gradients,
        &id_map,
        &stylesheet,
    );
    out
}

pub(crate) fn render_compiled_items(items: &[CompiledItem], canvas: &mut Canvas, x: Pt, y: Pt) {
    for it in items {
        match it {
            CompiledItem::Path(path) => draw_compiled_path(canvas, path, x, y),
            CompiledItem::Image(img) => {
                canvas.draw_image(
                    x + Pt::from_f32(img.x),
                    y + Pt::from_f32(img.y),
                    Pt::from_f32(img.width),
                    Pt::from_f32(img.height),
                    img.source.clone(),
                );
            }
        }
    }
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
    node: roxmltree::Node<'_, '_>,
    ctm: Matrix,
    style: &SvgStyle,
    gradients: &std::collections::HashMap<String, GradientDef>,
    id_map: &std::collections::HashMap<String, roxmltree::Node<'_, '_>>,
    stylesheet: &SvgStylesheet,
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
                );
            }
        }
        "use" => {
            // Minimal <use> support: href/xlink:href with "#id", optional x/y.
            if let Some(id) = href_id(node) {
                if let Some(target) = id_map.get(&id).copied() {
                    let x = parse_number(node.attribute("x").unwrap_or("0")).unwrap_or(0.0);
                    let y = parse_number(node.attribute("y").unwrap_or("0")).unwrap_or(0.0);
                    let use_ctm = local_ctm.mul(Matrix::translate(x, y));
                    compile_element(
                        out,
                        target,
                        use_ctm,
                        &local_style,
                        gradients,
                        id_map,
                        stylesheet,
                    );
                }
            }
        }
        "path" => {
            if let Some(d) = node.attribute("d") {
                let segs = parse_path_data(d);
                let clip = compile_clip_for_node(node, local_ctm, id_map);
                push_compiled_path(out, &segs, &local_style, local_ctm, gradients, clip);
            }
        }
        "rect" => {
            if let Some(segs) = rect_to_path(node) {
                let clip = compile_clip_for_node(node, local_ctm, id_map);
                push_compiled_path(out, &segs, &local_style, local_ctm, gradients, clip);
            }
        }
        "circle" => {
            if let Some(segs) = circle_to_path(node) {
                let clip = compile_clip_for_node(node, local_ctm, id_map);
                push_compiled_path(out, &segs, &local_style, local_ctm, gradients, clip);
            }
        }
        "ellipse" => {
            if let Some(segs) = ellipse_to_path(node) {
                let clip = compile_clip_for_node(node, local_ctm, id_map);
                push_compiled_path(out, &segs, &local_style, local_ctm, gradients, clip);
            }
        }
        "line" => {
            if let Some(segs) = line_to_path(node) {
                let clip = compile_clip_for_node(node, local_ctm, id_map);
                push_compiled_path(out, &segs, &local_style, local_ctm, gradients, clip);
            }
        }
        "polyline" => {
            if let Some(segs) = poly_points_to_path(node, false) {
                let clip = compile_clip_for_node(node, local_ctm, id_map);
                push_compiled_path(out, &segs, &local_style, local_ctm, gradients, clip);
            }
        }
        "polygon" => {
            if let Some(segs) = poly_points_to_path(node, true) {
                let clip = compile_clip_for_node(node, local_ctm, id_map);
                push_compiled_path(out, &segs, &local_style, local_ctm, gradients, clip);
            }
        }
        "image" => {
            // Raster image inside SVG (PNG/JPEG/data URI). We only support axis-aligned transforms for now.
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

            // If the CTM includes rotation/shear, we'd need a matrix-based image draw.
            // For now, only accept near-axis-aligned matrices.
            if local_ctm.b.abs() > 1e-4 || local_ctm.c.abs() > 1e-4 {
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
            }));
        }
        _ => {
            // Ignore unknown tags in our subset.
        }
    }
}

fn build_id_map<'a>(
    doc: &'a roxmltree::Document<'a>,
) -> std::collections::HashMap<String, roxmltree::Node<'a, 'a>> {
    let mut out = std::collections::HashMap::new();
    for node in doc.descendants().filter(|n| n.is_element()) {
        if let Some(id) = node.attribute("id") {
            // First wins (matches common SVG authoring expectations).
            out.entry(id.to_string()).or_insert(node);
        }
    }
    out
}

fn href_id(node: roxmltree::Node<'_, '_>) -> Option<String> {
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
        } => Shading::Radial {
            x0: x0 + dx,
            y0: y0 + dy,
            r0: *r0,
            x1: x1 + dx,
            y1: y1 + dy,
            r1: *r1,
            stops: stops.clone(),
        },
    }
}

fn compile_clip_for_node(
    node: roxmltree::Node<'_, '_>,
    ctm: Matrix,
    id_map: &std::collections::HashMap<String, roxmltree::Node<'_, '_>>,
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
    node: roxmltree::Node<'_, '_>,
    ctm: Matrix,
    id_map: &std::collections::HashMap<String, roxmltree::Node<'_, '_>>,
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
    let Some((min_x, min_y, vb_w, vb_h)) = view_box else {
        return Matrix::identity();
    };

    // Opinionated default: "meet" preserveAspectRatio and center.
    let sx = if vb_w > 0.0 { w / vb_w } else { 1.0 };
    let sy = if vb_h > 0.0 { h / vb_h } else { 1.0 };
    let s = sx.min(sy);
    let tx = (w - vb_w * s) * 0.5 - min_x * s;
    let ty = (h - vb_h * s) * 0.5 - min_y * s;
    Matrix::translate(tx, ty).mul(Matrix::scale(s, s))
}

fn extract_svg_stylesheet(doc: &roxmltree::Document<'_>) -> SvgStylesheet {
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
        let Ok(sheet) = StyleSheet::parse(css, ParserOptions::default()) else {
            continue;
        };
        collect_svg_style_rules(sheet.rules, &mut out.rules, &mut order);
    }

    out
}

fn collect_svg_style_rules(
    rules: lightningcss::rules::CssRuleList,
    out: &mut Vec<SvgCssRule>,
    order: &mut usize,
) {
    for rule in rules.0 {
        match rule {
            CssRule::Style(style_rule) => {
                let selectors = style_rule
                    .selectors
                    .to_css_string(PrinterOptions::default())
                    .unwrap_or_default();
                let declarations = style_rule
                    .declarations
                    .to_css_string(PrinterOptions::default())
                    .unwrap_or_default();
                if declarations.trim().is_empty() {
                    *order += 1;
                    continue;
                }
                for selector_raw in selectors.split(',') {
                    if let Some(selector) = parse_svg_selector(selector_raw) {
                        out.push(SvgCssRule {
                            selector,
                            declarations: declarations.clone(),
                            order: *order,
                        });
                    }
                }
                *order += 1;
            }
            CssRule::Media(media) => {
                collect_svg_style_rules(media.rules, out, order);
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

fn parent_element<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
) -> Option<roxmltree::Node<'a, 'input>> {
    let mut cursor = node.parent();
    while let Some(parent) = cursor {
        if parent.is_element() {
            return Some(parent);
        }
        cursor = parent.parent();
    }
    None
}

fn svg_simple_selector_matches(
    node: roxmltree::Node<'_, '_>,
    selector: &SvgSimpleSelector,
) -> bool {
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

fn svg_selector_matches(node: roxmltree::Node<'_, '_>, selector: &SvgSelector) -> bool {
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

fn apply_style_string_normal_only(input: &str, style: &mut SvgStyle) -> bool {
    if let Ok(style_attr) = StyleAttribute::parse(input, ParserOptions::default()) {
        apply_svg_property_list(&style_attr.declarations.declarations, style);
        return true;
    }
    false
}

fn apply_style_string_important_only(input: &str, style: &mut SvgStyle) -> bool {
    if let Ok(style_attr) = StyleAttribute::parse(input, ParserOptions::default()) {
        apply_svg_property_list(&style_attr.declarations.important_declarations, style);
        return true;
    }
    false
}

fn apply_svg_stylesheet(
    node: roxmltree::Node<'_, '_>,
    stylesheet: &SvgStylesheet,
    style: &mut SvgStyle,
) {
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
        if !apply_style_string_normal_only(&rule.declarations, style) {
            apply_style_string_legacy(&rule.declarations, style);
        }
    }
    for rule in &matched {
        let _ = apply_style_string_important_only(&rule.declarations, style);
    }
}

fn apply_presentation_and_style(
    node: roxmltree::Node<'_, '_>,
    stylesheet: &SvgStylesheet,
    style: &mut SvgStyle,
) {
    // Presentation attributes are the baseline.
    if let Some(fill) = node.attribute("fill") {
        parse_paint_into(fill, &mut style.fill);
    }
    if let Some(stroke) = node.attribute("stroke") {
        parse_paint_into(stroke, &mut style.stroke);
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
    if let Ok(style_attr) = StyleAttribute::parse(input, ParserOptions::default()) {
        apply_svg_property_list(&style_attr.declarations.declarations, style);
        apply_svg_property_list(&style_attr.declarations.important_declarations, style);
        return;
    }
    apply_style_string_legacy(input, style);
}

fn apply_svg_property_list(props: &[Property], style: &mut SvgStyle) {
    for prop in props {
        apply_svg_property(prop, style);
    }
}

fn apply_svg_property(prop: &Property<'_>, style: &mut SvgStyle) {
    match prop {
        Property::Fill(paint) => {
            if let Some(alpha) = apply_svg_paint(paint, &mut style.fill) {
                style.fill_opacity *= alpha;
            }
        }
        Property::Stroke(paint) => {
            if let Some(alpha) = apply_svg_paint(paint, &mut style.stroke) {
                style.stroke_opacity *= alpha;
            }
        }
        Property::StrokeWidth(value) => {
            if let Some(raw) = value.to_css_string(PrinterOptions::default()).ok() {
                if let Some(v) = parse_number(&raw) {
                    style.stroke_width = v.max(0.0);
                }
            }
        }
        Property::StrokeMiterlimit(value) => {
            style.miter_limit = value.max(0.0);
        }
        Property::StrokeLinecap(value) => {
            style.line_cap = match value {
                StrokeLinecap::Round => 1,
                StrokeLinecap::Square => 2,
                StrokeLinecap::Butt => 0,
            };
        }
        Property::StrokeLinejoin(value) => {
            style.line_join = match value {
                StrokeLinejoin::Round => 1,
                StrokeLinejoin::Bevel => 2,
                StrokeLinejoin::Miter | StrokeLinejoin::MiterClip | StrokeLinejoin::Arcs => 0,
            };
        }
        Property::FillRule(value) => {
            style.fill_rule_evenodd = matches!(value, FillRule::Evenodd);
        }
        Property::StrokeDasharray(value) => match value {
            StrokeDasharray::None => {
                style.dash_pattern.clear();
            }
            StrokeDasharray::Values(values) => {
                style.dash_pattern = values
                    .iter()
                    .filter_map(|v| v.to_css_string(PrinterOptions::default()).ok())
                    .filter_map(|raw| parse_number(&raw))
                    .collect();
                if style.dash_pattern.len() % 2 == 1 {
                    let dup = style.dash_pattern.clone();
                    style.dash_pattern.extend_from_slice(&dup);
                }
            }
        },
        Property::StrokeDashoffset(value) => {
            if let Some(raw) = value.to_css_string(PrinterOptions::default()).ok() {
                if let Some(v) = parse_number(&raw) {
                    style.dash_offset = v;
                }
            }
        }
        Property::Opacity(value) => {
            let o = alpha_value(value);
            style.fill_opacity *= o;
            style.stroke_opacity *= o;
        }
        Property::FillOpacity(value) => {
            style.fill_opacity *= alpha_value(value);
        }
        Property::StrokeOpacity(value) => {
            style.stroke_opacity *= alpha_value(value);
        }
        _ => {}
    }
}

fn apply_svg_paint(paint: &SVGPaint<'_>, out: &mut Paint) -> Option<f32> {
    match paint {
        SVGPaint::None => {
            out.color = None;
            out.gradient_id = None;
            Some(1.0)
        }
        SVGPaint::Color(color) => {
            if let Some((mapped, alpha)) = css_color_to_svg_color(color) {
                out.color = Some(mapped);
                out.gradient_id = None;
                Some(alpha)
            } else {
                None
            }
        }
        SVGPaint::Url { url, fallback } => {
            let raw = url.url.as_ref().trim();
            if let Some(id) = raw.strip_prefix('#') {
                if !id.is_empty() {
                    out.color = None;
                    out.gradient_id = Some(id.to_string());
                    return Some(1.0);
                }
            }
            match fallback {
                Some(SVGPaintFallback::Color(color)) => {
                    if let Some((mapped, alpha)) = css_color_to_svg_color(color) {
                        out.color = Some(mapped);
                        out.gradient_id = None;
                        Some(alpha)
                    } else {
                        None
                    }
                }
                Some(SVGPaintFallback::None) => {
                    out.color = None;
                    out.gradient_id = None;
                    Some(1.0)
                }
                None => None,
            }
        }
        SVGPaint::ContextFill | SVGPaint::ContextStroke => None,
    }
}

fn css_color_to_svg_color(color: &CssColor) -> Option<(Color, f32)> {
    if let CssColor::RGBA(rgba) = color {
        let alpha = (rgba.alpha as f32 / 255.0).clamp(0.0, 1.0);
        let r = rgba.red as f32 / 255.0;
        let g = rgba.green as f32 / 255.0;
        let b = rgba.blue as f32 / 255.0;
        return Some((Color::rgb(r, g, b), alpha));
    }
    if let Ok(srgb) = SRGB::try_from(color) {
        return Some((Color::rgb(srgb.r, srgb.g, srgb.b), 1.0));
    }
    None
}

fn alpha_value(value: &AlphaValue) -> f32 {
    value.0.clamp(0.0, 1.0)
}

fn apply_style_string_legacy(input: &str, style: &mut SvgStyle) {
    for decl in input.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        let Some((k, v)) = decl.split_once(':') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        let val = v.trim();
        match key.as_str() {
            "fill" => {
                parse_paint_into(val, &mut style.fill);
            }
            "stroke" => {
                parse_paint_into(val, &mut style.stroke);
            }
            "stroke-width" => {
                if let Some(v) = parse_number(val) {
                    style.stroke_width = v.max(0.0);
                }
            }
            "stroke-miterlimit" => {
                if let Some(v) = parse_number(val) {
                    style.miter_limit = v.max(0.0);
                }
            }
            "stroke-linecap" => {
                style.line_cap = match val {
                    "round" => 1,
                    "square" => 2,
                    _ => 0,
                };
            }
            "stroke-linejoin" => {
                style.line_join = match val {
                    "round" => 1,
                    "bevel" => 2,
                    _ => 0,
                };
            }
            "fill-rule" => {
                style.fill_rule_evenodd = val.eq_ignore_ascii_case("evenodd");
            }
            "stroke-dasharray" => {
                if val.eq_ignore_ascii_case("none") {
                    style.dash_pattern.clear();
                } else {
                    style.dash_pattern = parse_length_list(val);
                    if style.dash_pattern.len() % 2 == 1 {
                        let dup = style.dash_pattern.clone();
                        style.dash_pattern.extend_from_slice(&dup);
                    }
                }
            }
            "stroke-dashoffset" => {
                if let Some(v) = parse_number(val) {
                    style.dash_offset = v;
                }
            }
            "opacity" => {
                if let Some(v) = parse_number(val) {
                    let o = v.clamp(0.0, 1.0);
                    style.fill_opacity *= o;
                    style.stroke_opacity *= o;
                }
            }
            "fill-opacity" => {
                if let Some(v) = parse_number(val) {
                    style.fill_opacity *= v.clamp(0.0, 1.0);
                }
            }
            "stroke-opacity" => {
                if let Some(v) = parse_number(val) {
                    style.stroke_opacity *= v.clamp(0.0, 1.0);
                }
            }
            _ => {}
        }
    }
}

fn parse_length_list(input: &str) -> Vec<f32> {
    input
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .filter_map(parse_number)
        .collect()
}

fn parse_paint_into(input: &str, out: &mut Paint) {
    let v = input.trim();
    if v.eq_ignore_ascii_case("none") {
        out.color = None;
        out.gradient_id = None;
        return;
    }

    if let Some(id) = parse_url_ref(v) {
        out.color = None;
        out.gradient_id = Some(id);
        return;
    }

    if let Some(c) = parse_color(v) {
        out.color = Some(c);
        out.gradient_id = None;
        return;
    }

    // Unknown paint (e.g. currentColor): ignore and keep inherited/current.
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

fn parse_stop_color(node: roxmltree::Node<'_, '_>, stylesheet: &SvgStylesheet) -> Option<Color> {
    let mut stop_color = node.attribute("stop-color").and_then(parse_color);

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
            if let Some(color) = parse_named_stop_color_decl(&rule.declarations, false) {
                stop_color = Some(color);
            }
        }
        for rule in &matched {
            if let Some(color) = parse_named_stop_color_decl(&rule.declarations, true) {
                stop_color = Some(color);
            }
        }
    }

    if let Some(style_attr) = node.attribute("style") {
        for decl in style_attr.split(';') {
            let decl = decl.trim();
            let Some((k, v)) = decl.split_once(':') else {
                continue;
            };
            if k.trim().eq_ignore_ascii_case("stop-color") {
                stop_color = parse_color(v.trim());
            }
        }
    }
    stop_color
}

fn parse_named_stop_color_decl(input: &str, important_only: bool) -> Option<Color> {
    let mut out = None;
    for decl in input.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        let Some((raw_key, raw_value)) = decl.split_once(':') else {
            continue;
        };
        if !raw_key.trim().eq_ignore_ascii_case("stop-color") {
            continue;
        }
        let mut value = raw_value.trim();
        let has_important = value.to_ascii_lowercase().contains("!important");
        if important_only != has_important {
            continue;
        }
        if has_important {
            value = value
                .rsplit_once("!important")
                .map(|(v, _)| v.trim())
                .unwrap_or(value);
        }
        if let Some(color) = parse_color(value) {
            out = Some(color);
        }
    }
    out
}

fn extract_gradients(
    doc: &roxmltree::Document<'_>,
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
            stops.push(crate::types::ShadingStop { offset, color });
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

fn rect_to_path(node: roxmltree::Node<'_, '_>) -> Option<Vec<PathSeg>> {
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

fn circle_to_path(node: roxmltree::Node<'_, '_>) -> Option<Vec<PathSeg>> {
    let cx = parse_number(node.attribute("cx").unwrap_or("0")).unwrap_or(0.0);
    let cy = parse_number(node.attribute("cy").unwrap_or("0")).unwrap_or(0.0);
    let r = parse_number(node.attribute("r")?)?;
    if r <= 0.0 {
        return None;
    }
    ellipse_to_path_impl(cx, cy, r, r)
}

fn ellipse_to_path(node: roxmltree::Node<'_, '_>) -> Option<Vec<PathSeg>> {
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

fn line_to_path(node: roxmltree::Node<'_, '_>) -> Option<Vec<PathSeg>> {
    let x1 = parse_number(node.attribute("x1").unwrap_or("0")).unwrap_or(0.0);
    let y1 = parse_number(node.attribute("y1").unwrap_or("0")).unwrap_or(0.0);
    let x2 = parse_number(node.attribute("x2").unwrap_or("0")).unwrap_or(0.0);
    let y2 = parse_number(node.attribute("y2").unwrap_or("0")).unwrap_or(0.0);
    Some(vec![PathSeg::MoveTo(x1, y1), PathSeg::LineTo(x2, y2)])
}

fn poly_points_to_path(node: roxmltree::Node<'_, '_>, close: bool) -> Option<Vec<PathSeg>> {
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
    let sin_phi = libm::sinf(phi);
    let cos_phi = libm::cosf(phi);

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
        let s = libm::sqrtf(lambda);
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
        coef = sign * libm::sqrtf((num / den).max(0.0));
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
        libm::atan2f(det, dot)
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
    let segs_count = libm::ceilf(dtheta.abs() / (PI / 2.0)).max(1.0) as i32;
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
    let k = (4.0 / 3.0) * libm::tanf(dt / 4.0);

    let s1 = libm::sinf(t1);
    let c1 = libm::cosf(t1);
    let s2 = libm::sinf(t2);
    let c2 = libm::cosf(t2);

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
    use crate::types::Size;

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
