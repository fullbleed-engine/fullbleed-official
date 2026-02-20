use crate::canvas::{Canvas, Document};
use crate::debug::{DebugLogger, json_escape};
use crate::doc_context::DocContext;
use crate::error::FullBleedError;
use crate::flowable::{BreakAfter, BreakBefore, Flowable};
use crate::frame::AddResult;
use crate::metrics::{DocumentMetrics, PageMetrics};
use crate::page_template::PageTemplate;
use crate::types::Pt;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

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
        let log_page_break = |from_page: usize,
                              to_page: usize,
                              reason: &str,
                              flowable_name: Option<&str>,
                              frame_index: usize| {
            let Some(logger) = debug.as_deref() else {
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
                doc_id, reason, from_page, to_page, frame_index, name_json
            );
            logger.log_json(&json);
            logger.increment("jit.page_break.trigger", 1);
        };

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

        let draw_fixed_overlays = |canvas: &mut Canvas,
                                   overlays: &[Box<dyn Flowable>],
                                   page_flowables: &mut usize| {
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

        let mut fixed_overlays: Vec<Box<dyn Flowable>> = Vec::new();
        let mut story: VecDeque<Box<dyn Flowable>> = VecDeque::new();
        for flowable in self.story {
            if flowable.is_fixed_positioned() {
                fixed_overlays.push(flowable);
            } else {
                story.push_back(flowable);
            }
        }

        let finish_page = |canvas: &mut Canvas,
                           page_number: usize,
                           page_flowables: &mut usize,
                           metrics: &mut DocumentMetrics,
                           page_start: &mut Instant,
                           fixed_overlays: &[Box<dyn Flowable>]| {
            if canvas.is_current_empty() && fixed_overlays.is_empty() {
                return;
            }
            draw_fixed_overlays(canvas, fixed_overlays, page_flowables);
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

        while let Some(flowable) = story.pop_front() {
            let mut current = flowable;
            let mut suppress_break_before = false;
            loop {
                let current_name = current.debug_name().to_string();
                let pagination = current.pagination();
                if !suppress_break_before
                    && matches!(pagination.break_before, BreakBefore::Page)
                    && (placed_on_page || frame_index > 0)
                {
                    log_page_break(
                        page_number,
                        page_number + 1,
                        "break_before_page",
                        Some(&current_name),
                        frame_index,
                    );
                    finish_page(
                        &mut canvas,
                        page_number,
                        &mut page_flowables,
                        &mut metrics,
                        &mut page_start,
                        &fixed_overlays,
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
                }

                if frame_index >= frames.len() {
                    log_page_break(
                        page_number,
                        page_number + 1,
                        "frame_exhausted",
                        Some(&current_name),
                        frame_index,
                    );
                    finish_page(
                        &mut canvas,
                        page_number,
                        &mut page_flowables,
                        &mut metrics,
                        &mut page_start,
                        &fixed_overlays,
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
                    AddResult::Placed => {
                        placed_on_page = true;
                        page_flowables += 1;
                        if matches!(pagination.break_after, BreakAfter::Page) {
                            log_page_break(
                                page_number,
                                page_number + 1,
                                "break_after_page",
                                Some(&current_name),
                                frame_index,
                            );
                            finish_page(
                                &mut canvas,
                                page_number,
                                &mut page_flowables,
                                &mut metrics,
                                &mut page_start,
                                &fixed_overlays,
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
                        }
                        break;
                    }
                    AddResult::Split(remaining) => {
                        placed_on_page = true;
                        page_flowables += 1;
                        log_page_break(
                            page_number,
                            page_number + usize::from(is_last_frame),
                            "flowable_split",
                            Some(&current_name),
                            frame_index,
                        );
                        suppress_break_before = true;
                        current = remaining;
                        frame_index += 1;
                    }
                    AddResult::Overflow(remaining) => {
                        log_page_break(
                            page_number,
                            page_number + usize::from(is_last_frame),
                            "frame_overflow",
                            Some(&current_name),
                            frame_index,
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
                &fixed_overlays,
            );
        }

        Ok((canvas.finish_without_show(), metrics))
    }
}
