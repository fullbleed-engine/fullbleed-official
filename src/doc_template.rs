use crate::canvas::{
    Canvas, Command, Document, META_NAMED_STRING_PREFIX, META_PAGINATION_EVENT_KEY,
    META_RUNNING_ELEMENT_PREFIX,
};
use crate::debug::{DebugLogger, json_escape};
use crate::doc_context::{DocContext, GeneratedContentValues, RunningElementValue};
use crate::error::FullBleedError;
use crate::flowable::{BreakBefore, Flowable, FootnoteContinuationFlowable};
use crate::frame::{AddResult, AddTrace};
use crate::metrics::{DocumentMetrics, PageMetrics};
use crate::page_template::{PageSelector, PageTemplate};
use crate::types::Pt;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

fn bool_to_flag(value: bool) -> u8 {
    if value { 1 } else { 0 }
}

#[derive(Debug, Clone, Default)]
struct PageGeneratedContent {
    running_elements: HashMap<String, GeneratedContentValues<RunningElementValue>>,
    named_strings: HashMap<String, GeneratedContentValues<String>>,
}

fn resolve_page_generated_values<T: Clone>(
    occurrences: &HashMap<String, Vec<T>>,
    carried: &mut HashMap<String, T>,
) -> HashMap<String, GeneratedContentValues<T>> {
    let names = carried
        .keys()
        .chain(occurrences.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut resolved = HashMap::with_capacity(names.len());
    for name in names {
        let previous = carried.get(&name).cloned();
        let on_page = occurrences.get(&name).map(Vec::as_slice).unwrap_or(&[]);
        let first_on_page = on_page.first().cloned();
        let last_on_page = on_page.last().cloned();
        let values = GeneratedContentValues {
            // `start` retains a value already established before the page;
            // on the first page, its first assignment establishes the value.
            start: previous.clone().or_else(|| first_on_page.clone()),
            first: first_on_page.clone().or_else(|| previous.clone()),
            last: last_on_page.clone().or_else(|| previous.clone()),
            first_except: on_page.is_empty().then_some(previous.clone()).flatten(),
        };
        if let Some(value) = last_on_page {
            carried.insert(name.clone(), value);
        }
        resolved.insert(name, values);
    }
    resolved
}

fn collect_page_generated_content(pages: &[crate::canvas::Page]) -> Vec<PageGeneratedContent> {
    let mut form_sizes = HashMap::<String, (Pt, Pt)>::new();
    for page in pages {
        for command in &page.commands {
            if let Command::DefineForm {
                resource_id,
                width,
                height,
                ..
            }
            | Command::DefineIsolatedForm {
                resource_id,
                width,
                height,
                ..
            } = command
            {
                form_sizes
                    .entry(resource_id.clone())
                    .or_insert((*width, *height));
            }
        }
    }

    let mut carried_running = HashMap::<String, RunningElementValue>::new();
    let mut carried_strings = HashMap::<String, String>::new();
    let mut pages_resolved = Vec::with_capacity(pages.len());
    for page in pages {
        let mut running = HashMap::<String, Vec<RunningElementValue>>::new();
        let mut strings = HashMap::<String, Vec<String>>::new();
        for command in &page.commands {
            let Command::Meta { key, value } = command else {
                continue;
            };
            if let Some(name) = key.strip_prefix(META_RUNNING_ELEMENT_PREFIX) {
                if let Some((width, height)) = form_sizes.get(value) {
                    running
                        .entry(name.to_string())
                        .or_default()
                        .push(RunningElementValue {
                            resource_id: value.clone(),
                            width: *width,
                            height: *height,
                        });
                }
            } else if let Some(name) = key.strip_prefix(META_NAMED_STRING_PREFIX) {
                strings
                    .entry(name.to_string())
                    .or_default()
                    .push(value.clone());
            }
        }
        pages_resolved.push(PageGeneratedContent {
            running_elements: resolve_page_generated_values(&running, &mut carried_running),
            named_strings: resolve_page_generated_values(&strings, &mut carried_strings),
        });
    }
    pages_resolved
}

fn page_satisfies_break_before(page_number: usize, value: BreakBefore) -> bool {
    match value {
        BreakBefore::Left | BreakBefore::Verso => page_number % 2 == 0,
        BreakBefore::Right | BreakBefore::Recto => page_number % 2 == 1,
        _ => true,
    }
}

fn trace_b64(value: &str) -> String {
    crate::base64::encode_url_safe_no_pad(value.as_bytes())
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
            blank: bool,
            named_page: Option<u64>,
        ) -> &'a PageTemplate {
            let uses_page_selectors = page_templates
                .iter()
                .any(|template| template.page_selector() != PageSelector::Sequence);
            if uses_page_selectors {
                if let Some(named_page) = named_page {
                    let selector = if blank && page_number % 2 == 0 {
                        PageSelector::NamedBlankLeft(named_page)
                    } else if blank {
                        PageSelector::NamedBlankRight(named_page)
                    } else if page_number == 1 {
                        PageSelector::NamedFirst(named_page)
                    } else if page_number % 2 == 0 {
                        PageSelector::NamedLeft(named_page)
                    } else {
                        PageSelector::NamedRight(named_page)
                    };
                    if let Some(template) = page_templates
                        .iter()
                        .find(|template| template.page_selector() == selector)
                        .or_else(|| {
                            page_templates.iter().find(|template| {
                                template.page_selector() == PageSelector::NamedAny(named_page)
                            })
                        })
                    {
                        return template;
                    }
                }
                let selector = if blank && page_number % 2 == 0 {
                    PageSelector::BlankLeft
                } else if blank {
                    PageSelector::BlankRight
                } else if page_number == 1 {
                    PageSelector::First
                } else if page_number % 2 == 0 {
                    PageSelector::Left
                } else {
                    PageSelector::Right
                };
                return page_templates
                    .iter()
                    .find(|template| template.page_selector() == selector)
                    .or_else(|| {
                        page_templates
                            .iter()
                            .find(|template| template.page_selector() == PageSelector::Any)
                    })
                    .unwrap_or(&page_templates[0]);
            }
            // Selection rule:
            // - page 1 -> templates[0]
            // - page 2 -> templates[1] (if present)
            // - ...
            // - page n -> templates[min(n-1, templates.len()-1)] (last template repeats)
            let idx = page_number.saturating_sub(1);
            let idx = idx.min(page_templates.len() - 1);
            &page_templates[idx]
        }

        let mut active_named_page = None;
        let template = select_template(&self.page_templates, 1, false, active_named_page);
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

        macro_rules! advance_page {
            ($blank:expr) => {{
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
                let template =
                    select_template(&self.page_templates, page_number, $blank, active_named_page);
                canvas.set_page_size(template.page_size);
                canvas.set_page_presentation(template.page_presentation());
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
            }};
        }

        canvas.set_page_presentation(template.page_presentation());
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
                let ends_with_vertical_fragmentainer = current.ends_with_vertical_fragmentainer();
                let mut named_page_advanced = false;
                if pagination.page_name != active_named_page {
                    active_named_page = pagination.page_name;
                    if placed_on_page || frame_index > 0 {
                        emit_pagination_transition_event(
                            &mut canvas,
                            debug.as_deref(),
                            debug_doc_id,
                            page_number,
                            page_number + 1,
                            frame_index,
                            0,
                            "named_page_transition",
                            Some(&current_name),
                            &current_owner_meta,
                            Some(current_source_order),
                            Some(segment_index),
                        );
                        advance_page!(false);
                        named_page_advanced = true;
                    } else {
                        let template = select_template(
                            &self.page_templates,
                            page_number,
                            false,
                            active_named_page,
                        );
                        canvas.restart_current_page(template.page_size);
                        canvas.set_page_presentation(template.page_presentation());
                        frames = template.instantiate_frames();
                        frame_index = 0;
                        if let Some(callback) = template.on_page() {
                            callback(&mut canvas, &DocContext::new(page_number, &template.name));
                        }
                        canvas.meta(
                            crate::META_PAGE_TEMPLATE_KEY.to_string(),
                            template.name.clone(),
                        );
                        draw_fixed_overlays(&mut canvas, &fixed_overlays_back, &mut page_flowables);
                        if page_number == 1 {
                            draw_fixed_overlays(
                                &mut canvas,
                                &root_out_of_flow_back,
                                &mut page_flowables,
                            );
                        }
                    }
                }
                if !named_page_advanced
                    && !suppress_break_before
                    && pagination.break_before.forces_page()
                    && ((placed_on_page || frame_index > 0)
                        || !page_satisfies_break_before(page_number, pagination.break_before))
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
                    let next_is_blank =
                        !page_satisfies_break_before(page_number + 1, pagination.break_before);
                    advance_page!(next_is_blank);
                    while !page_satisfies_break_before(page_number, pagination.break_before) {
                        emit_pagination_transition_event(
                            &mut canvas,
                            debug.as_deref(),
                            debug_doc_id,
                            page_number,
                            page_number + 1,
                            frame_index,
                            0,
                            "break_before_side_blank",
                            Some(&current_name),
                            &current_owner_meta,
                            Some(current_source_order),
                            Some(segment_index),
                        );
                        advance_page!(false);
                    }
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
                    let next_is_blank = suppress_break_before
                        && !page_satisfies_break_before(page_number + 1, pagination.break_before);
                    advance_page!(next_is_blank);
                    if suppress_break_before {
                        while !page_satisfies_break_before(page_number, pagination.break_before) {
                            emit_pagination_transition_event(
                                &mut canvas,
                                debug.as_deref(),
                                debug_doc_id,
                                page_number,
                                page_number + 1,
                                frame_index,
                                0,
                                "fragment_side_blank",
                                Some(&current_name),
                                &current_owner_meta,
                                Some(current_source_order),
                                Some(segment_index),
                            );
                            advance_page!(false);
                        }
                    }
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
                let add_result = frame.add(current, &mut canvas);
                let deferred_footnotes = frame.take_deferred_footnotes();
                match add_result {
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
                        if !deferred_footnotes.is_empty() {
                            story.push_front(Box::new(FootnoteContinuationFlowable::new(
                                deferred_footnotes,
                            )));
                        }
                        if pagination.break_after.forces_page()
                            && (!story.is_empty() || ends_with_vertical_fragmentainer)
                        {
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
                            let target = pagination.break_after.continuation_break_before();
                            let next_is_blank = target.is_some_and(|target| {
                                !page_satisfies_break_before(page_number + 1, target)
                            });
                            advance_page!(next_is_blank);
                            if let Some(target) = target {
                                while !page_satisfies_break_before(page_number, target) {
                                    emit_pagination_transition_event(
                                        &mut canvas,
                                        debug.as_deref(),
                                        debug_doc_id,
                                        page_number,
                                        page_number + 1,
                                        frame_index,
                                        0,
                                        "break_after_side_blank",
                                        Some(&current_name),
                                        &current_owner_meta,
                                        Some(current_source_order),
                                        Some(segment_index),
                                    );
                                    advance_page!(false);
                                }
                            }
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
                        if !deferred_footnotes.is_empty() {
                            story.push_front(remaining);
                            story.push_front(Box::new(FootnoteContinuationFlowable::new(
                                deferred_footnotes,
                            )));
                            break;
                        }
                        suppress_break_before = true;
                        current = remaining;
                        segment_index = segment_index.saturating_add(1);
                        frame_index += 1;
                    }
                    AddResult::Overflow(remaining, trace) => {
                        debug_assert!(deferred_footnotes.is_empty());
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

        let mut document = canvas.finish_without_show();
        let total_pages = document.pages.len();
        let generated_content = collect_page_generated_content(&document.pages);
        let mut page_counter = 0i32;
        for (index, page) in document.pages.iter_mut().enumerate() {
            let template_name = page.commands.iter().find_map(|command| match command {
                crate::Command::Meta { key, value } if key == crate::META_PAGE_TEMPLATE_KEY => {
                    Some(value.as_str())
                }
                _ => None,
            });
            let template = template_name
                .and_then(|name| {
                    self.page_templates
                        .iter()
                        .find(|template| template.name == name)
                })
                .unwrap_or(&self.page_templates[0]);
            match (
                template.page_counter_reset(),
                template.page_counter_increment(),
            ) {
                (Some(reset), Some(increment)) => {
                    page_counter = reset.saturating_add(increment);
                }
                (Some(reset), None) => page_counter = reset,
                (None, Some(increment)) => {
                    page_counter = page_counter.saturating_add(increment);
                }
                (None, None) => page_counter = page_counter.saturating_add(1),
            }
            let Some(callback) = template.on_page_finalize() else {
                continue;
            };
            let mut overlay = Canvas::new(template.page_size);
            callback(
                &mut overlay,
                &DocContext::finalized(
                    index + 1,
                    total_pages,
                    page_counter,
                    &template.name,
                    generated_content
                        .get(index)
                        .map(|content| content.running_elements.clone())
                        .unwrap_or_default(),
                    generated_content
                        .get(index)
                        .map(|content| content.named_strings.clone())
                        .unwrap_or_default(),
                ),
            );
            let mut overlay_document = overlay.finish();
            let Some(overlay_page) = overlay_document.pages.pop() else {
                continue;
            };
            let overlay_command_count = overlay_page.commands.len();
            let mut finalized_commands = Vec::with_capacity(
                page.commands
                    .len()
                    .saturating_add(overlay_command_count)
                    .saturating_add(4),
            );
            // Both streams were compiled from a fresh Canvas and may omit
            // assignments for default paint state. Scope the existing page,
            // restore the physical page's initial graphics state, then scope
            // the late-bound overlay independently so neither stream can
            // inherit or leak fill/stroke/transform state.
            finalized_commands.push(crate::Command::SaveState);
            finalized_commands.append(&mut page.commands);
            finalized_commands.push(crate::Command::RestoreState);
            finalized_commands.push(crate::Command::SaveState);
            finalized_commands.extend(overlay_page.commands);
            finalized_commands.push(crate::Command::RestoreState);
            page.commands = finalized_commands;
            if let Some(page_metrics) = metrics.pages.get_mut(index) {
                page_metrics.command_count = page_metrics
                    .command_count
                    .saturating_add(overlay_command_count)
                    .saturating_add(4);
            }
        }

        Ok((document, metrics))
    }
}
