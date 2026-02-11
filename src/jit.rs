use crate::canvas::{Command, Document, Page};
use crate::debug::{DebugLogger, json_escape};
use crate::font::FontRegistry;
use crate::page_data::{PageDataContext, PageDataValue};
use crate::types::{Pt, Rect, Size};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

pub type DocId = usize;
pub type PaintableId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitMode {
    Off,
    PlanOnly,
    PlanAndReplay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Background,
    Content,
    Overlay,
}

#[derive(Debug, Clone, Copy)]
pub struct Transform {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Transform {
    pub fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn translate(tx: f32, ty: f32) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    pub fn scale(sx: f32, sy: f32) -> Self {
        Self {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn rotate(angle: f32) -> Self {
        let cos = angle.cos();
        let sin = angle.sin();
        Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn mul(self, other: Self) -> Self {
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
}

#[derive(Debug, Clone)]
pub struct PlacedItem {
    pub paintable_id: PaintableId,
    pub layer: Layer,
    pub bbox: Option<Rect>,
    pub transform: Option<Transform>,
}

#[derive(Debug, Clone)]
pub struct PagePlan {
    pub page_number: usize,
    pub page_count: usize,
    pub page_data: Option<HashMap<String, PageDataValue>>,
    pub placements: Vec<PlacedItem>,
}

#[derive(Debug, Clone)]
pub enum Paintable {
    PageCommands { commands: Vec<Command> },
}

pub type FontFaceId = String;
pub type GlyphSet = BTreeSet<u32>;

#[derive(Debug, Clone)]
pub struct DocPlan {
    pub doc_id: DocId,
    pub page_size: Size,
    pub page_count: usize,
    pub pages: Vec<PagePlan>,
    pub page_data: Option<PageDataContext>,
    pub font_use: HashMap<FontFaceId, GlyphSet>,
    pub paintables: Vec<Paintable>,
}

#[derive(Debug, Clone)]
pub struct PageOps {
    pub commands: Vec<Command>,
}

pub fn plan_document_with_overlay(
    doc_id: DocId,
    document: &Document,
    background: Option<&Document>,
    overlay: Option<&Document>,
    page_data: Option<PageDataContext>,
    debug: Option<Arc<DebugLogger>>,
    font_registry: Option<&FontRegistry>,
) -> DocPlan {
    let mut paintables = Vec::with_capacity(document.pages.len() * 2);
    let mut pages = Vec::with_capacity(document.pages.len());
    let page_count = document.pages.len();
    let page_bbox = Rect {
        x: Pt::ZERO,
        y: Pt::ZERO,
        width: document.page_size.width,
        height: document.page_size.height,
    };

    for (page_index, page) in document.pages.iter().enumerate() {
        let mut placements = Vec::new();
        let page_data_snapshot = page_data
            .as_ref()
            .and_then(|ctx| ctx.pages.get(page_index).cloned());

        if !page.commands.is_empty() {
            let paintable_id = paintables.len();
            let bbox = commands_bbox(&page.commands, font_registry);
            paintables.push(Paintable::PageCommands {
                commands: page.commands.clone(),
            });
            placements.push(PlacedItem {
                paintable_id,
                layer: Layer::Content,
                bbox: bbox.or(Some(page_bbox)),
                transform: None,
            });
        }

        if let Some(bg_doc) = background {
            if let Some(bg_page) = bg_doc.pages.get(page_index) {
                if !bg_page.commands.is_empty() {
                    let paintable_id = paintables.len();
                    let bbox = commands_bbox(&bg_page.commands, font_registry);
                    paintables.push(Paintable::PageCommands {
                        commands: bg_page.commands.clone(),
                    });
                    placements.push(PlacedItem {
                        paintable_id,
                        layer: Layer::Background,
                        bbox: bbox.or(Some(page_bbox)),
                        transform: None,
                    });
                }
            }
        }

        if let Some(overlay_doc) = overlay {
            if let Some(overlay_page) = overlay_doc.pages.get(page_index) {
                if !overlay_page.commands.is_empty() {
                    let paintable_id = paintables.len();
                    let bbox = commands_bbox(&overlay_page.commands, font_registry);
                    paintables.push(Paintable::PageCommands {
                        commands: overlay_page.commands.clone(),
                    });
                    placements.push(PlacedItem {
                        paintable_id,
                        layer: Layer::Overlay,
                        bbox: bbox.or(Some(page_bbox)),
                        transform: None,
                    });
                }
            }
        }

        sort_placements(&mut placements);
        pages.push(PagePlan {
            page_number: page_index + 1,
            page_count,
            page_data: page_data_snapshot,
            placements,
        });
    }

    let plan = DocPlan {
        doc_id,
        page_size: document.page_size,
        page_count: pages.len(),
        pages,
        page_data,
        font_use: HashMap::new(),
        paintables,
    };

    if let Some(logger) = debug.as_deref() {
        let overlay_pages = overlay.map(|doc| doc.pages.len()).unwrap_or(0);
        let background_pages = background.map(|doc| doc.pages.len()).unwrap_or(0);
        let json = format!(
            "{{\"type\":\"jit.plan\",\"doc_id\":{},\"pages\":{},\"paintables\":{},\"overlay_pages\":{},\"background_pages\":{},\"page_data\":{},\"page_bounds\":{}}}",
            plan.doc_id,
            plan.page_count,
            plan.paintables.len(),
            overlay_pages,
            background_pages,
            if plan.page_data.is_some() {
                "true"
            } else {
                "false"
            },
            page_bounds_json(&plan.pages)
        );
        logger.log_json(&json);
        logger.log_json(&plan_to_json(&plan));
    }

    plan
}

pub fn paint_plan(plan: &DocPlan, debug: Option<Arc<DebugLogger>>) -> Vec<PageOps> {
    let mut out = Vec::with_capacity(plan.pages.len());

    for (page_index, page) in plan.pages.iter().enumerate() {
        let mut commands = Vec::new();
        let mut placements = page.placements.clone();
        sort_placements(&mut placements);
        for placement in placements {
            match &plan.paintables[placement.paintable_id] {
                Paintable::PageCommands { commands: cmds } => {
                    commands.extend(cmds.clone());
                }
            }
        }
        out.push(PageOps { commands });

        if let Some(logger) = debug.as_deref() {
            let json = format!(
                "{{\"type\":\"jit.paint\",\"doc_id\":{},\"page\":{},\"commands\":{}}}",
                plan.doc_id,
                page_index + 1,
                out.last().map(|ops| ops.commands.len()).unwrap_or(0)
            );
            logger.log_json(&json);
        }
    }

    out
}

pub fn paint_plan_parallel(plan: &DocPlan, debug: Option<Arc<DebugLogger>>) -> Vec<PageOps> {
    use rayon::prelude::*;

    let mut results: Vec<(usize, PageOps)> = plan
        .pages
        .par_iter()
        .enumerate()
        .map(|(page_index, page)| {
            let mut commands = Vec::new();
            let mut placements = page.placements.clone();
            sort_placements(&mut placements);
            for placement in placements {
                match &plan.paintables[placement.paintable_id] {
                    Paintable::PageCommands { commands: cmds } => {
                        commands.extend(cmds.clone());
                    }
                }
            }
            if let Some(logger) = debug.as_deref() {
                let json = format!(
                    "{{\"type\":\"jit.paint\",\"doc_id\":{},\"page\":{},\"commands\":{}}}",
                    plan.doc_id,
                    page_index + 1,
                    commands.len()
                );
                logger.log_json(&json);
            }
            (page_index, PageOps { commands })
        })
        .collect();

    results.sort_by_key(|(idx, _)| *idx);
    results.into_iter().map(|(_, ops)| ops).collect()
}

fn layer_rank(layer: Layer) -> u8 {
    match layer {
        Layer::Background => 0,
        Layer::Content => 1,
        Layer::Overlay => 2,
    }
}

fn sort_placements(placements: &mut Vec<PlacedItem>) {
    placements.sort_by_key(|p| layer_rank(p.layer));
}

fn page_bounds_json(pages: &[PagePlan]) -> String {
    let mut out = String::from("[");
    for (idx, page) in pages.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        let mut bounds: Option<(f32, f32, f32, f32)> = None;
        for placement in &page.placements {
            if let Some(rect) = placement.bbox {
                let min_x = rect.x.to_f32();
                let min_y = rect.y.to_f32();
                let max_x = min_x + rect.width.to_f32();
                let max_y = min_y + rect.height.to_f32();
                union_bounds(&mut bounds, (min_x, min_y, max_x, max_y));
            }
        }
        if let Some((min_x, min_y, max_x, max_y)) = bounds {
            out.push_str(&format!(
                "{{\"x\":{:.3},\"y\":{:.3},\"w\":{:.3},\"h\":{:.3}}}",
                min_x,
                min_y,
                (max_x - min_x).max(0.0),
                (max_y - min_y).max(0.0)
            ));
        } else {
            out.push_str("null");
        }
    }
    out.push(']');
    out
}

fn plan_to_json(plan: &DocPlan) -> String {
    let mut out = String::from("{\"type\":\"jit.docplan\"");
    out.push_str(&format!(
        ",\"doc_id\":{},\"page_size\":{{\"w\":{:.3},\"h\":{:.3}}},\"page_count\":{},\"font_use\":{},\"pages\":[",
        plan.doc_id,
        plan.page_size.width.to_f32(),
        plan.page_size.height.to_f32(),
        plan.page_count,
        plan.font_use.len()
    ));

    for (idx, page) in plan.pages.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"n\":{},\"page_count\":{},\"placements\":[",
            page.page_number, page.page_count
        ));

        for (pidx, placement) in page.placements.iter().enumerate() {
            if pidx > 0 {
                out.push(',');
            }
            out.push_str("{\"id\":");
            out.push_str(&placement.paintable_id.to_string());
            out.push_str(",\"layer\":");
            out.push_str(match placement.layer {
                Layer::Background => "\"bg\"",
                Layer::Content => "\"content\"",
                Layer::Overlay => "\"overlay\"",
            });
            if let Some(rect) = placement.bbox {
                out.push_str(&format!(
                    ",\"bbox\":{{\"x\":{:.3},\"y\":{:.3},\"w\":{:.3},\"h\":{:.3}}}",
                    rect.x.to_f32(),
                    rect.y.to_f32(),
                    rect.width.to_f32(),
                    rect.height.to_f32()
                ));
            }
            if let Some(tx) = placement.transform {
                out.push_str(&format!(
                    ",\"tx\":[{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}]",
                    tx.a, tx.b, tx.c, tx.d, tx.e, tx.f
                ));
            }
            out.push('}');
        }
        out.push(']');

        if let Some(ref data) = page.page_data {
            out.push_str(",\"data\":");
            out.push_str(&page_data_json(data));
        }
        out.push('}');
    }
    out.push_str("]}");
    out
}

fn page_data_json(data: &HashMap<String, PageDataValue>) -> String {
    let mut keys: Vec<&String> = data.keys().collect();
    keys.sort();
    let mut out = String::from("{");
    for (idx, key) in keys.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&json_escape(key));
        out.push_str("\":");
        if let Some(value) = data.get(*key) {
            out.push_str(&page_value_json(value));
        } else {
            out.push_str("null");
        }
    }
    out.push('}');
    out
}

fn page_value_json(value: &PageDataValue) -> String {
    match value {
        PageDataValue::Count(v) => format!("{{\"count\":{}}}", v),
        PageDataValue::Sum { scale, value } => {
            format!("{{\"sum\":{},\"scale\":{}}}", value, scale)
        }
        PageDataValue::Every(items) => {
            let mut out = String::from("{\"every\":[");
            let limit = 200usize;
            let shown = items.len().min(limit);
            for (idx, item) in items.iter().take(shown).enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                out.push('"');
                out.push_str(&json_escape(item));
                out.push('"');
            }
            out.push(']');
            if items.len() > limit {
                out.push_str(&format!(",\"truncated\":true,\"total\":{}", items.len()));
            }
            out.push('}');
            out
        }
    }
}

fn union_bounds(bounds: &mut Option<(f32, f32, f32, f32)>, rect: (f32, f32, f32, f32)) {
    let (min_x, min_y, max_x, max_y) = rect;
    match bounds {
        None => *bounds = Some((min_x, min_y, max_x, max_y)),
        Some(existing) => {
            existing.0 = existing.0.min(min_x);
            existing.1 = existing.1.min(min_y);
            existing.2 = existing.2.max(max_x);
            existing.3 = existing.3.max(max_y);
        }
    }
}

fn rect_from_bounds(bounds: Option<(f32, f32, f32, f32)>) -> Option<Rect> {
    bounds.map(|(min_x, min_y, max_x, max_y)| Rect {
        x: Pt::from_f32(min_x),
        y: Pt::from_f32(min_y),
        width: Pt::from_f32((max_x - min_x).max(0.0)),
        height: Pt::from_f32((max_y - min_y).max(0.0)),
    })
}

fn commands_bbox(commands: &[Command], font_registry: Option<&FontRegistry>) -> Option<Rect> {
    let mut bounds: Option<(f32, f32, f32, f32)> = None;
    let mut path_points: Vec<(f32, f32)> = Vec::new();
    let mut transform = Transform::identity();
    let mut stack: Vec<Transform> = Vec::new();
    let mut font_name = "Helvetica".to_string();
    let mut font_size = Pt::from_f32(12.0);
    let mut has_meta_bounds = false;

    for cmd in commands {
        if has_meta_bounds {
            if let Command::Meta { key, value } = cmd {
                if key == "__fb_bbox" {
                    if let Some(rect) = parse_bbox_meta(value) {
                        let min_x = rect.x.to_f32();
                        let min_y = rect.y.to_f32();
                        let max_x = min_x + rect.width.to_f32();
                        let max_y = min_y + rect.height.to_f32();
                        union_bounds(&mut bounds, (min_x, min_y, max_x, max_y));
                    }
                }
            }
            continue;
        }

        match cmd {
            Command::SaveState => stack.push(transform),
            Command::RestoreState => {
                transform = stack.pop().unwrap_or_else(Transform::identity);
            }
            Command::Translate(x, y) => {
                transform = transform.mul(Transform::translate(x.to_f32(), y.to_f32()));
            }
            Command::Scale(x, y) => {
                transform = transform.mul(Transform::scale(*x, *y));
            }
            Command::Rotate(angle) => {
                transform = transform.mul(Transform::rotate(*angle));
            }
            Command::SetFontName(name) => font_name = name.clone(),
            Command::SetFontSize(size) => font_size = *size,
            Command::MoveTo { x, y } => {
                let (tx, ty) = transform.apply(x.to_f32(), y.to_f32());
                path_points.push((tx, ty));
            }
            Command::LineTo { x, y } => {
                let (tx, ty) = transform.apply(x.to_f32(), y.to_f32());
                path_points.push((tx, ty));
            }
            Command::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                let p1 = transform.apply(x1.to_f32(), y1.to_f32());
                let p2 = transform.apply(x2.to_f32(), y2.to_f32());
                let p3 = transform.apply(x.to_f32(), y.to_f32());
                path_points.extend([p1, p2, p3]);
            }
            Command::ClosePath => {}
            Command::Fill
            | Command::FillEvenOdd
            | Command::Stroke
            | Command::FillStroke
            | Command::FillStrokeEvenOdd
            | Command::ShadingFill(_) => {
                if !path_points.is_empty() {
                    let mut min_x = path_points[0].0;
                    let mut min_y = path_points[0].1;
                    let mut max_x = path_points[0].0;
                    let mut max_y = path_points[0].1;
                    for (x, y) in &path_points {
                        min_x = min_x.min(*x);
                        min_y = min_y.min(*y);
                        max_x = max_x.max(*x);
                        max_y = max_y.max(*y);
                    }
                    union_bounds(&mut bounds, (min_x, min_y, max_x, max_y));
                    path_points.clear();
                }
            }
            Command::DrawRect {
                x,
                y,
                width,
                height,
            } => {
                let x0 = x.to_f32();
                let y0 = y.to_f32();
                let x1 = x0 + width.to_f32();
                let y1 = y0 + height.to_f32();
                let corners = [
                    transform.apply(x0, y0),
                    transform.apply(x1, y0),
                    transform.apply(x1, y1),
                    transform.apply(x0, y1),
                ];
                let mut min_x = corners[0].0;
                let mut min_y = corners[0].1;
                let mut max_x = corners[0].0;
                let mut max_y = corners[0].1;
                for (cx, cy) in &corners[1..] {
                    min_x = min_x.min(*cx);
                    min_y = min_y.min(*cy);
                    max_x = max_x.max(*cx);
                    max_y = max_y.max(*cy);
                }
                union_bounds(&mut bounds, (min_x, min_y, max_x, max_y));
            }
            Command::DrawImage {
                x,
                y,
                width,
                height,
                ..
            }
            | Command::DrawForm {
                x,
                y,
                width,
                height,
                ..
            } => {
                let x0 = x.to_f32();
                let y0 = y.to_f32();
                let x1 = x0 + width.to_f32();
                let y1 = y0 + height.to_f32();
                let corners = [
                    transform.apply(x0, y0),
                    transform.apply(x1, y0),
                    transform.apply(x1, y1),
                    transform.apply(x0, y1),
                ];
                let mut min_x = corners[0].0;
                let mut min_y = corners[0].1;
                let mut max_x = corners[0].0;
                let mut max_y = corners[0].1;
                for (cx, cy) in &corners[1..] {
                    min_x = min_x.min(*cx);
                    min_y = min_y.min(*cy);
                    max_x = max_x.max(*cx);
                    max_y = max_y.max(*cy);
                }
                union_bounds(&mut bounds, (min_x, min_y, max_x, max_y));
            }
            Command::DrawString { x, y, text } => {
                let width = if let Some(registry) = font_registry {
                    registry.measure_text_width(&font_name, font_size, text)
                } else {
                    let approx = text.chars().count() as f32 * 0.6;
                    Pt::from_f32(font_size.to_f32() * approx)
                };
                let height = font_size;
                let x0 = x.to_f32();
                let y0 = y.to_f32();
                let x1 = x0 + width.to_f32();
                let y1 = y0 + height.to_f32();
                let corners = [
                    transform.apply(x0, y0),
                    transform.apply(x1, y0),
                    transform.apply(x1, y1),
                    transform.apply(x0, y1),
                ];
                let mut min_x = corners[0].0;
                let mut min_y = corners[0].1;
                let mut max_x = corners[0].0;
                let mut max_y = corners[0].1;
                for (cx, cy) in &corners[1..] {
                    min_x = min_x.min(*cx);
                    min_y = min_y.min(*cy);
                    max_x = max_x.max(*cx);
                    max_y = max_y.max(*cy);
                }
                union_bounds(&mut bounds, (min_x, min_y, max_x, max_y));
            }
            Command::Meta { key, value } => {
                if key == "__fb_bbox" {
                    if let Some(rect) = parse_bbox_meta(value) {
                        let min_x = rect.x.to_f32();
                        let min_y = rect.y.to_f32();
                        let max_x = min_x + rect.width.to_f32();
                        let max_y = min_y + rect.height.to_f32();
                        union_bounds(&mut bounds, (min_x, min_y, max_x, max_y));
                        has_meta_bounds = true;
                    }
                }
            }
            Command::ClipRect { .. }
            | Command::ClipPath { .. }
            | Command::SetFillColor(_)
            | Command::SetStrokeColor(_)
            | Command::SetLineWidth(_)
            | Command::SetLineCap(_)
            | Command::SetLineJoin(_)
            | Command::SetMiterLimit(_)
            | Command::SetDash { .. }
            | Command::SetOpacity { .. }
            | Command::DefineForm { .. }
            | Command::BeginTag { .. }
            | Command::EndTag
            | Command::BeginArtifact { .. }
            | Command::BeginOptionalContent { .. }
            | Command::EndMarkedContent => {}
        }
    }

    rect_from_bounds(bounds)
}

fn parse_bbox_meta(raw: &str) -> Option<Rect> {
    let mut parts = raw.split(',');
    let x = parts.next()?.trim().parse::<i64>().ok()?;
    let y = parts.next()?.trim().parse::<i64>().ok()?;
    let w = parts.next()?.trim().parse::<i64>().ok()?;
    let h = parts.next()?.trim().parse::<i64>().ok()?;
    Some(Rect {
        x: Pt::from_milli_i64(x),
        y: Pt::from_milli_i64(y),
        width: Pt::from_milli_i64(w),
        height: Pt::from_milli_i64(h),
    })
}

pub fn ops_to_document(page_size: Size, ops: Vec<PageOps>) -> Document {
    let mut pages = Vec::with_capacity(ops.len());
    for page_ops in ops {
        pages.push(Page {
            commands: page_ops.commands,
        });
    }
    Document { page_size, pages }
}
