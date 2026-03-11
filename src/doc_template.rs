use crate::canvas::{Canvas, Document, META_PAGINATION_EVENT_KEY};
use crate::debug::{DebugLogger, json_escape};
use crate::doc_context::DocContext;
use crate::error::FullBleedError;
use crate::flowable::{BreakAfter, BreakBefore, Flowable};
use crate::frame::{AddResult, AddTrace};
use crate::metrics::{DocumentMetrics, PageMetrics};
use crate::page_template::PageTemplate;
use crate::types::Pt;
use base64::Engine;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

fn bool_to_flag(value: bool) -> u8 {
    if value { 1 } else { 0 }
}

fn trace_b64(value: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.as_bytes())
}

fn owner_trace_fields(owner_meta: &[(String, String)]) -> String {
    let get = |key: &str| {
        owner_meta
            .iter()
            .find_map(|(candidate, value)| (candidate == key).then(|| value.as_str()))
    };
    let mut fields: Vec<String> = Vec::new();
    for (field, key) in [
        ("owner_selector_b64", "fb.owner.selector"),
        ("owner_dom_path_b64", "fb.owner.dom_path"),
        ("owner_role_b64", "fb.owner.role"),
        ("owner_component_b64", "fb.owner.component"),
        ("owner_tag_b64", "fb.owner.tag"),
        ("owner_id_b64", "fb.owner.id"),
        ("owner_classes_b64", "fb.owner.classes"),
    ] {
        if let Some(value) = get(key) {
            fields.push(format!("{field}={}", trace_b64(value)));
        }
    }
    if fields.is_empty() {
        String::new()
    } else {
        format!("|{}", fields.join("|"))
    }
}

fn emit_pagination_layout_event(
    canvas: &mut Canvas,
    source_order: usize,
    segment_index: usize,
    flowable_name: &str,
    owner_meta: &[(String, String)],
    frame_index: usize,
    is_last_frame: bool,
    placed_on_page_before: bool,
    trace: AddTrace,
    overflow_severity: Option<&str>,
) {
    let placed = trace.placed_rect.unwrap_or(crate::types::Rect {
        x: Pt::ZERO,
        y: Pt::ZERO,
        width: Pt::ZERO,
        height: Pt::ZERO,
    });
    let value = format!(
        "event=layout|source_order={}|segment_index={}|flowable={}|frame_index={}|is_last_frame={}|placed_on_page_before={}|result={}|reason={}|overflow_severity={}|cursor_y_before={}|avail_w={}|avail_h={}|frame_x={}|frame_y={}|frame_w={}|frame_h={}|wrapped_w={}|wrapped_h={}|placed_x={}|placed_y={}|placed_w={}|placed_h={}{}",
        source_order,
        segment_index,
        flowable_name,
        frame_index,
        bool_to_flag(is_last_frame),
        bool_to_flag(placed_on_page_before),
        trace.disposition.as_str(),
        trace.reason,
        overflow_severity.unwrap_or("none"),
        trace.cursor_y_before.to_milli_i64(),
        trace.avail_width.to_milli_i64(),
        trace.avail_height.to_milli_i64(),
        trace.frame_rect.x.to_milli_i64(),
        trace.frame_rect.y.to_milli_i64(),
        trace.frame_rect.width.to_milli_i64(),
        trace.frame_rect.height.to_milli_i64(),
        trace.wrapped_size.width.to_milli_i64(),
        trace.wrapped_size.height.to_milli_i64(),
        placed.x.to_milli_i64(),
        placed.y.to_milli_i64(),
        placed.width.to_milli_i64(),
        placed.height.to_milli_i64(),
        owner_trace_fields(owner_meta),
    );
    canvas.meta(META_PAGINATION_EVENT_KEY, value);
}

fn emit_pagination_transition_event(
    canvas: &mut Canvas,
    debug: Option<&DebugLogger>,
    debug_doc_id: Option<usize>,
    from_page: usize,
    to_page: usize,
    from_frame_index: usize,
    to_frame_index: usize,
    reason: &str,
    flowable_name: Option<&str>,
    owner_meta: &[(String, String)],
    source_order: Option<usize>,
    segment_index: Option<usize>,
) {
    let flowable = flowable_name.unwrap_or("unknown");
    let source_order = source_order
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-1".to_string());
    let segment_index = segment_index
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-1".to_string());
    let value = format!(
        "event=transition|from_page={}|to_page={}|from_frame_index={}|to_frame_index={}|reason={}|flowable={}|source_order={}|segment_index={}{}",
        from_page,
        to_page,
        from_frame_index,
        to_frame_index,
        reason,
        flowable,
        source_order,
        segment_index,
        owner_trace_fields(owner_meta),
    );
    canvas.meta(META_PAGINATION_EVENT_KEY, value);

    let Some(logger) = debug else {
        return;
    };
    let doc_id = debug_doc_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "null".to_string());
    let name_json = flowable_name
        .map(|name| format!("\"{}\"", json_escape(name)))
        .unwrap_or_else(|| "null".to_string());
    let json = format!(
        "{{\"type\":\"jit.page_break\",\"doc_id\":{},\"code\":\"PAGE_BREAK_TRIGGER\",\"reason\":\"{}\",\"from_page\":{},\"to_page\":{},\"frame_index\":{},\"flowable\":{}}}",
        doc_id, reason, from_page, to_page, from_frame_index, name_json
    );
    logger.log_json(&json);
    logger.increment("jit.page_break.trigger", 1);
}

pub struct DocTemplate {
    page_templates: Vec<PageTemplate>,
    story: Vec<Box<dyn Flowable>>,
    debug: Option<Arc<DebugLogger>>,
    debug_doc_id: Option<usize>,
}

impl DocTemplate {
    pub fn new(page_templates: Vec<PageTemplate>) -> Self {
        Self {
            page_templates,
            story: Vec::new(),
            debug: None,
            debug_doc_id: None,
        }
    }

    pub(crate) fn with_debug(mut self, debug: Arc<DebugLogger>, doc_id: Option<usize>) -> Self {
        self.debug = Some(debug);
        self.debug_doc_id = doc_id;
        self
    }

    pub fn add_flowable(&mut self, flowable: Box<dyn Flowable>) {
        self.story.push(flowable);
    }

    pub fn build(self) -> Result<Document, FullBleedError> {
        Ok(self.build_with_metrics()?.0)
    }

    pub fn build_with_metrics(self) -> Result<(Document, DocumentMetrics), FullBleedError> {
        if self.page_templates.is_empty() {
            return Err(FullBleedError::MissingPageTemplate);
        }

        let debug = self.debug.clone();
        let debug_doc_id = self.debug_doc_id;

        fn select_template<'a>(
            page_templates: &'a [PageTemplate],
            page_number: usize,
        ) -> &'a PageTemplate {
            // Selection rule:
            // - page 1 -> templates[0]
            // - page 2 -> templates[1] (if present)
            // - ...
            // - page n -> templates[min(n-1, templates.len()-1)] (last template repeats)
            let idx = page_number.saturating_sub(1);
            let idx = idx.min(page_templates.len() - 1);
            &page_templates[idx]
        }

        let template = select_template(&self.page_templates, 1);
        let mut canvas = Canvas::new(template.page_size);
        let mut page_number = 1usize;
        let mut frames = template.instantiate_frames();
        let mut frame_index = 0usize;
        let mut placed_on_page = false;
        let mut metrics = DocumentMetrics::default();
        let mut page_start = Instant::now();
        let mut page_flowables = 0usize;
        let mut source_order = 0usize;

        let draw_fixed_overlays =
            |canvas: &mut Canvas, overlays: &[Box<dyn Flowable>], page_flowables: &mut usize| {
                if overlays.is_empty() {
                    return;
                }
                let page_size = canvas.page_size();
                for overlay in overlays {
                    overlay.draw(
                        canvas,
                        Pt::ZERO,
                        Pt::ZERO,
                        page_size.width,
                        page_size.height,
                    );
                    *page_flowables += 1;
                }
            };

        let mut fixed_overlays_back: Vec<Box<dyn Flowable>> = Vec::new();
        let mut fixed_overlays_front: Vec<Box<dyn Flowable>> = Vec::new();
        let mut root_out_of_flow_back: Vec<Box<dyn Flowable>> = Vec::new();
        let mut root_out_of_flow_front: Vec<Box<dyn Flowable>> = Vec::new();
        let mut story: VecDeque<Box<dyn Flowable>> = VecDeque::new();
        for flowable in self.story {
            if flowable.is_fixed_positioned() {
                if flowable.z_index() < 0 {
                    fixed_overlays_back.push(flowable);
                } else {
                    fixed_overlays_front.push(flowable);
                }
            } else if flowable.out_of_flow() {
                // Root-level non-fixed out-of-flow (e.g. position:absolute) is treated as a
                // page-one overlay lane. z-index<0 paints behind flow, z-index>=0 paints above.
                if flowable.z_index() < 0 {
                    root_out_of_flow_back.push(flowable);
                } else {
                    root_out_of_flow_front.push(flowable);
                }
            } else {
                story.push_back(flowable);
            }
        }
        // Keep fixed overlay paint order deterministic and z-index aware.
        // Lower z-index paints first, higher z-index paints later (on top).
        fixed_overlays_back.sort_by(|left, right| left.z_index().cmp(&right.z_index()));
        fixed_overlays_front.sort_by(|left, right| left.z_index().cmp(&right.z_index()));
        root_out_of_flow_back.sort_by(|left, right| left.z_index().cmp(&right.z_index()));
        root_out_of_flow_front.sort_by(|left, right| left.z_index().cmp(&right.z_index()));

        let finish_page = |canvas: &mut Canvas,
                           page_number: usize,
                           page_flowables: &mut usize,
                           metrics: &mut DocumentMetrics,
                           page_start: &mut Instant,
                           fixed_overlays_front: &[Box<dyn Flowable>],
                           root_out_of_flow_front: &[Box<dyn Flowable>]| {
            if canvas.is_current_empty()
                && fixed_overlays_front.is_empty()
                && (page_number != 1 || root_out_of_flow_front.is_empty())
            {
                return;
            }
            if page_number == 1 {
                draw_fixed_overlays(canvas, root_out_of_flow_front, page_flowables);
            }
            draw_fixed_overlays(canvas, fixed_overlays_front, page_flowables);
            if canvas.is_current_empty() {
                return;
            }
            let elapsed = page_start.elapsed().as_secs_f64() * 1000.0;
            metrics.total_render_ms += elapsed;
            metrics.pages.push(PageMetrics {
                page_number,
                render_ms: elapsed,
                command_count: canvas.current_command_count(),
                flowable_count: *page_flowables,
                content_bytes: 0,
            });
            canvas.show_page();
            *page_flowables = 0;
            *page_start = Instant::now();
        };

        if let Some(callback) = template.on_page() {
            callback(&mut canvas, &DocContext::new(page_number, &template.name));
        }
        canvas.meta(
            crate::META_PAGE_TEMPLATE_KEY.to_string(),
            template.name.clone(),
        );
        draw_fixed_overlays(&mut canvas, &fixed_overlays_back, &mut page_flowables);
        draw_fixed_overlays(&mut canvas, &root_out_of_flow_back, &mut page_flowables);

        while let Some(flowable) = story.pop_front() {
            let mut current = flowable;
            let current_source_order = source_order;
            source_order = source_order.saturating_add(1);
            let mut segment_index = 0usize;
            let mut suppress_break_before = false;
            loop {
                let current_name = current.debug_name().to_string();
                let current_owner_meta = current.diagnostic_metadata();
                let pagination = current.pagination();
                if !suppress_break_before
                    && matches!(pagination.break_before, BreakBefore::Page)
                    && (placed_on_page || frame_index > 0)
                {
                    emit_pagination_transition_event(
                        &mut canvas,
                        debug.as_deref(),
                        debug_doc_id,
                        page_number,
                        page_number + 1,
                        frame_index,
                        0,
                        "break_before_page",
                        Some(&current_name),
                        &current_owner_meta,
                        Some(current_source_order),
                        Some(segment_index),
                    );
                    finish_page(
                        &mut canvas,
                        page_number,
                        &mut page_flowables,
                        &mut metrics,
                        &mut page_start,
                        &fixed_overlays_front,
                        &root_out_of_flow_front,
                    );
                    page_number += 1;
                    let template = select_template(&self.page_templates, page_number);
                    frames = template.instantiate_frames();
                    frame_index = 0;
                    placed_on_page = false;
                    if let Some(callback) = template.on_page() {
                        callback(&mut canvas, &DocContext::new(page_number, &template.name));
                    }
                    canvas.meta(
                        crate::META_PAGE_TEMPLATE_KEY.to_string(),
                        template.name.clone(),
                    );
                    draw_fixed_overlays(&mut canvas, &fixed_overlays_back, &mut page_flowables);
                }

                if frame_index >= frames.len() {
                    emit_pagination_transition_event(
                        &mut canvas,
                        debug.as_deref(),
                        debug_doc_id,
                        page_number,
                        page_number + 1,
                        frame_index.saturating_sub(1),
                        0,
                        "frame_exhausted",
                        Some(&current_name),
                        &current_owner_meta,
                        Some(current_source_order),
                        Some(segment_index),
                    );
                    finish_page(
                        &mut canvas,
                        page_number,
                        &mut page_flowables,
                        &mut metrics,
                        &mut page_start,
                        &fixed_overlays_front,
                        &root_out_of_flow_front,
                    );
                    page_number += 1;
                    let template = select_template(&self.page_templates, page_number);
                    frames = template.instantiate_frames();
                    frame_index = 0;
                    placed_on_page = false;
                    if let Some(callback) = template.on_page() {
                        callback(&mut canvas, &DocContext::new(page_number, &template.name));
                    }
                    canvas.meta(
                        crate::META_PAGE_TEMPLATE_KEY.to_string(),
                        template.name.clone(),
                    );
                    draw_fixed_overlays(&mut canvas, &fixed_overlays_back, &mut page_flowables);
                }

                if frames.is_empty() {
                    return Err(FullBleedError::MissingPageTemplate);
                }

                let is_last_frame = frame_index + 1 >= frames.len();
                let frame_rect = frames[frame_index].rect();
                let debug_details = if !placed_on_page && is_last_frame {
                    let size = current.wrap(frame_rect.width, frame_rect.height);
                    let pagination = current.pagination();
                    Some(format!(
                        "{} size={}x{}pt frame={}x{}pt break_inside={:?} break_before={:?} break_after={:?}",
                        current.debug_name(),
                        size.width.to_f32(),
                        size.height.to_f32(),
                        frame_rect.width.to_f32(),
                        frame_rect.height.to_f32(),
                        pagination.break_inside,
                        pagination.break_before,
                        pagination.break_after,
                    ))
                } else {
                    None
                };

                let frame = &mut frames[frame_index];
                match frame.add(current, &mut canvas) {
                    AddResult::Placed(trace) => {
                        emit_pagination_layout_event(
                            &mut canvas,
                            current_source_order,
                            segment_index,
                            &current_name,
                            &current_owner_meta,
                            frame_index,
                            is_last_frame,
                            placed_on_page,
                            trace,
                            None,
                        );
                        placed_on_page = true;
                        page_flowables += 1;
                        if matches!(pagination.break_after, BreakAfter::Page) {
                            emit_pagination_transition_event(
                                &mut canvas,
                                debug.as_deref(),
                                debug_doc_id,
                                page_number,
                                page_number + 1,
                                frame_index,
                                0,
                                "break_after_page",
                                Some(&current_name),
                                &current_owner_meta,
                                Some(current_source_order),
                                Some(segment_index),
                            );
                            finish_page(
                                &mut canvas,
                                page_number,
                                &mut page_flowables,
                                &mut metrics,
                                &mut page_start,
                                &fixed_overlays_front,
                                &root_out_of_flow_front,
                            );
                            page_number += 1;
                            let template = select_template(&self.page_templates, page_number);
                            frames = template.instantiate_frames();
                            frame_index = 0;
                            placed_on_page = false;
                            if let Some(callback) = template.on_page() {
                                callback(
                                    &mut canvas,
                                    &DocContext::new(page_number, &template.name),
                                );
                            }
                            canvas.meta(
                                crate::META_PAGE_TEMPLATE_KEY.to_string(),
                                template.name.clone(),
                            );
                            draw_fixed_overlays(
                                &mut canvas,
                                &fixed_overlays_back,
                                &mut page_flowables,
                            );
                        }
                        break;
                    }
                    AddResult::Split(remaining, trace) => {
                        emit_pagination_layout_event(
                            &mut canvas,
                            current_source_order,
                            segment_index,
                            &current_name,
                            &current_owner_meta,
                            frame_index,
                            is_last_frame,
                            placed_on_page,
                            trace,
                            None,
                        );
                        emit_pagination_transition_event(
                            &mut canvas,
                            debug.as_deref(),
                            debug_doc_id,
                            page_number,
                            page_number + usize::from(is_last_frame),
                            frame_index,
                            if is_last_frame { 0 } else { frame_index + 1 },
                            "flowable_split",
                            Some(&current_name),
                            &current_owner_meta,
                            Some(current_source_order),
                            Some(segment_index),
                        );
                        placed_on_page = true;
                        page_flowables += 1;
                        suppress_break_before = true;
                        current = remaining;
                        segment_index = segment_index.saturating_add(1);
                        frame_index += 1;
                    }
                    AddResult::Overflow(remaining, trace) => {
                        let overflow_severity = if !placed_on_page && is_last_frame {
                            "fatal_unplaceable"
                        } else if is_last_frame {
                            "page_advance"
                        } else {
                            "frame_advance"
                        };
                        emit_pagination_layout_event(
                            &mut canvas,
                            current_source_order,
                            segment_index,
                            &current_name,
                            &current_owner_meta,
                            frame_index,
                            is_last_frame,
                            placed_on_page,
                            trace,
                            Some(overflow_severity),
                        );
                        emit_pagination_transition_event(
                            &mut canvas,
                            debug.as_deref(),
                            debug_doc_id,
                            page_number,
                            page_number + usize::from(is_last_frame),
                            frame_index,
                            if is_last_frame { 0 } else { frame_index + 1 },
                            "frame_overflow",
                            Some(&current_name),
                            &current_owner_meta,
                            Some(current_source_order),
                            Some(segment_index),
                        );
                        if !placed_on_page && is_last_frame {
                            let details = debug_details.unwrap_or_else(|| "unknown".to_string());
                            return Err(FullBleedError::UnplaceableFlowable(details));
                        }
                        current = remaining;
                        frame_index += 1;
                    }
                }
            }
        }

        if !canvas.is_current_empty() || metrics.pages.is_empty() {
            finish_page(
                &mut canvas,
                page_number,
                &mut page_flowables,
                &mut metrics,
                &mut page_start,
                &fixed_overlays_front,
                &root_out_of_flow_front,
            );
        }

        Ok((canvas.finish_without_show(), metrics))
    }
}
