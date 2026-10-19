//! Render-local layouts retained for experimental, slot-sized running content.
//!
//! Ordinary CSS running elements still use their captured forms. This registry
//! is owned by one layout pass and is never stored in the stylesheet cache.

use crate::canvas::{Canvas, Command, META_RUNNING_ELEMENT_PREFIX};
use crate::debug::json_escape;
use crate::flowable::Flowable;
use crate::font::FontRegistry;
use crate::pdf::PdfOptions;
use crate::types::{Pt, Rect, Size};
use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, Mutex};

pub(crate) const LAYOUT_ATTRIBUTE: &str = "data-fb-running-layout";
pub(crate) const FIT_ERROR_META: &str = "__fb_page_margin_fit_error";
pub(crate) const FIT_ERROR_PREFIX: &str = "page-margin layout: ";
pub(crate) const REFLOW_GUARD_META: &str = "__fb_page_margin_requires_reflow";
const KEY_PREFIX: &str = "fb-running-layout-";

pub(crate) fn with_record_index(error: crate::FullBleedError, row: usize) -> crate::FullBleedError {
    match error {
        crate::FullBleedError::UnplaceableFlowable(message) => {
            let message = if let Some(payload) = message
                .strip_prefix(FIT_ERROR_PREFIX)
                .and_then(|payload| payload.strip_prefix('{'))
            {
                format!("{FIT_ERROR_PREFIX}{{\"record\":{},{}", row + 1, payload)
            } else {
                message
            };
            crate::FullBleedError::UnplaceableFlowable(message)
        }
        other => other,
    }
}

pub(crate) struct RunningLayout {
    pub child: Box<dyn Flowable>,
    pub source_ids: Vec<String>,
    managed: bool,
    paint: Arc<PaintContext>,
}

struct PaintContext {
    fonts: Arc<FontRegistry>,
    options: PdfOptions,
}

impl RunningLayout {
    fn fit_error(
        &self,
        page: usize,
        slot: &str,
        available: Rect,
        measured: Option<Size>,
        reason: &str,
    ) -> String {
        let sources = self
            .source_ids
            .iter()
            .map(|id| format!("\"{}\"", json_escape(id)))
            .collect::<Vec<_>>()
            .join(",");
        let measured = measured
            .map(|size| {
                format!(
                    "{{\"width\":{},\"height\":{}}}",
                    size.width.to_milli_i64(),
                    size.height.to_milli_i64()
                )
            })
            .unwrap_or_else(|| "null".to_string());
        format!(
            "{{\"schema\":\"fullbleed.page_margin_fit.v1\",\"code\":\"layout.page_margin_size\",\"reason\":\"{reason}\",\"page\":{page},\"slot\":\"{slot}\",\"units\":\"millipoints\",\"available\":{{\"x\":{},\"y\":{},\"width\":{},\"height\":{}}},\"measured\":{measured},\"source_ids\":[{sources}]}}",
            available.x.to_milli_i64(),
            available.y.to_milli_i64(),
            available.width.to_milli_i64(),
            available.height.to_milli_i64()
        )
    }

    pub fn materialize(
        &self,
        key: &str,
        slot: &str,
        page: usize,
        available: Rect,
        reserved: Rect,
        canvas: &mut Canvas,
    ) -> Result<crate::doc_context::RunningElementValue, String> {
        // Even a currently empty band must be laid out again if its bindings
        // change. Page-local text patches cannot update its retained forms.
        canvas.meta(REFLOW_GUARD_META, "true");
        if available.width <= Pt::ZERO || available.height <= Pt::ZERO {
            return Err(self.fit_error(page, slot, available, None, "empty_slot"));
        }
        let measured = self.child.wrap(available.width, Pt::from_i32(1_000_000));
        let tolerance = Pt::from_milli_i64(1);
        if self.managed
            && (available.x < reserved.x - tolerance
                || available.y < reserved.y - tolerance
                || available.x + available.width > reserved.x + reserved.width + tolerance
                || available.y + available.height > reserved.y + reserved.height + tolerance)
        {
            return Err(self.fit_error(
                page,
                slot,
                available,
                Some(measured),
                "slot_outside_reserved_margin",
            ));
        }
        if measured.width > available.width + tolerance
            || measured.height > available.height + tolerance
        {
            return Err(self.fit_error(
                page,
                slot,
                available,
                Some(measured),
                "measured_box_overflow",
            ));
        }
        let width = available.width;
        let height = measured.height.max(Pt::ZERO);
        if height <= Pt::ZERO && !self.managed {
            return Ok(crate::doc_context::RunningElementValue {
                resource_id: key.to_string(),
                width,
                height,
            });
        }
        let mut surface = Canvas::new(Size { width, height });
        if self.managed {
            // A Form inherits the caller's graphics state. Make the state used
            // by the paint checker explicit instead of relying on what body
            // text or a neighboring margin box happened to leave behind.
            surface.set_line_width(Pt::from_i32(1));
            surface.set_line_cap(0);
            surface.set_line_join(0);
            surface.set_miter_limit(Pt::from_i32(10));
            surface.set_opacity(1.0, 1.0);
            surface.set_text_rendering_mode(0);
        }
        self.child
            .draw(&mut surface, Pt::ZERO, Pt::ZERO, width, height);
        let mut document = surface.finish();
        let commands = document
            .pages
            .pop()
            .map(|page| page.commands)
            .unwrap_or_default();
        if commands.iter().any(|command| matches!(command, Command::Meta { key, .. } if key.starts_with(META_RUNNING_ELEMENT_PREFIX))) {
            return Err(self.fit_error(page, slot, available, Some(measured), "nested_running_content"));
        }
        if self.managed {
            match crate::paint_bounds::measure(
                &commands,
                Size { width, height },
                &self.paint.fonts,
                &self.paint.options,
            ) {
                Ok(Some(bounds)) if !bounds.fits(width, height) => {
                    let [left, top, right, bottom] = bounds.page_rect(height);
                    let mut error =
                        self.fit_error(page, slot, available, Some(measured), "paint_overflow");
                    error.pop();
                    error.push_str(&format!(",\"paint\":{{\"left\":{left},\"top\":{top},\"right\":{right},\"bottom\":{bottom}}}}}"));
                    return Err(error);
                }
                Err(unknown) => {
                    let mut error = self.fit_error(
                        page,
                        slot,
                        available,
                        Some(measured),
                        "paint_bounds_unknown",
                    );
                    error.pop();
                    let forms = unknown
                        .forms
                        .iter()
                        .map(|form| format!("\"{}\"", json_escape(form)))
                        .collect::<Vec<_>>()
                        .join(",");
                    error.push_str(&format!(",\"paint_check\":{{\"reason\":\"{}\",\"command\":{},\"forms\":[{forms}]}}}}",unknown.reason,unknown.command));
                    return Err(error);
                }
                _ => {}
            }
        }
        // Managed zero-height layouts were drawn and checked above: positioned
        // descendants must not disappear through this formerly early return.
        if height <= Pt::ZERO {
            return Ok(crate::doc_context::RunningElementValue {
                resource_id: key.to_string(),
                width,
                height,
            });
        }
        let resource_id = form_id(key, width, height, &commands);
        canvas.define_form(resource_id.clone(), width, height, commands);
        Ok(crate::doc_context::RunningElementValue {
            resource_id,
            width,
            height,
        })
    }
}

#[derive(Clone)]
pub(crate) struct RunningLayouts {
    layouts: Arc<Mutex<Vec<Arc<RunningLayout>>>>,
    paint: Arc<PaintContext>,
}

impl Default for RunningLayouts {
    fn default() -> Self {
        Self::new(Arc::new(FontRegistry::new()), PdfOptions::default())
    }
}

impl fmt::Debug for RunningLayouts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunningLayouts")
            .field(
                "count",
                &self.layouts.lock().unwrap_or_else(|e| e.into_inner()).len(),
            )
            .finish()
    }
}

impl RunningLayouts {
    pub fn new(fonts: Arc<FontRegistry>, options: PdfOptions) -> Self {
        Self {
            layouts: Arc::new(Mutex::new(Vec::new())),
            paint: Arc::new(PaintContext { fonts, options }),
        }
    }

    pub fn register(
        &self,
        child: Box<dyn Flowable>,
        source_id: Option<String>,
        managed: bool,
    ) -> String {
        let mut seen = BTreeSet::new();
        let source_ids = source_id
            .into_iter()
            .chain(child.diagnostic_source_ids())
            .filter(|value| !value.is_empty() && seen.insert(value.clone()))
            .collect();
        let mut layouts = self.layouts.lock().unwrap_or_else(|e| e.into_inner());
        let key = format!("{KEY_PREFIX}{}", layouts.len());
        layouts.push(Arc::new(RunningLayout {
            child,
            source_ids,
            managed,
            paint: self.paint.clone(),
        }));
        key
    }

    pub fn get(&self, key: &str) -> Option<Arc<RunningLayout>> {
        let index = key.strip_prefix(KEY_PREFIX)?.parse::<usize>().ok()?;
        self.layouts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(index)
            .cloned()
    }
}

pub(crate) fn form_id(name: &str, width: Pt, height: Pt, commands: &[Command]) -> String {
    // Keep ordinary running-element resource identities byte-compatible.
    let mut hash = 0xcbf29ce484222325u64;
    let mut absorb = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    };
    absorb(name.as_bytes());
    absorb(&width.to_milli_i64().to_le_bytes());
    absorb(&height.to_milli_i64().to_le_bytes());
    for command in commands {
        absorb(format!("{command:?}").as_bytes());
    }
    format!("css-running-{hash:016x}")
}

#[cfg(test)]
mod paint_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FullBleed, FullBleedError};
    use std::collections::HashMap;

    const BASE: &str = "@page { size:300pt 220pt; margin:80pt 20pt 20pt; @top-left { content:element(head); } } * { margin:0; box-sizing:border-box; } html { font:10pt/12pt Helvetica; } .band { position:running(head); background:#ffe100; }";

    fn html(text: &str, tail: &str) -> String {
        format!(
            "<html><body><div class='band' data-fb-running-layout='margin' data-fb-id='header'>{text}</div><p>body</p>{tail}</body></html>"
        )
    }

    fn running_forms(document: &crate::Document) -> Vec<(usize, Pt, Pt, Vec<Command>)> {
        document
            .pages
            .iter()
            .enumerate()
            .flat_map(|(page, value)| {
                value
                    .commands
                    .iter()
                    .filter_map(move |command| match command {
                        Command::DefineForm {
                            resource_id,
                            width,
                            height,
                            commands,
                        } if resource_id.starts_with("css-running-") => {
                            Some((page + 1, *width, *height, commands.clone()))
                        }
                        _ => None,
                    })
            })
            .collect()
    }

    fn form_text(commands: &[Command]) -> String {
        commands
            .iter()
            .filter_map(|command| match command {
                Command::DrawString { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn deferred_running_layout_reflows_for_each_named_page_width() {
        let engine = FullBleed::builder().build().unwrap();
        let css = format!(
            "{BASE} @page narrow {{ size:180pt 220pt; }} .next {{ page:narrow; break-before:page; }}"
        );
        let document = engine
            .render_to_document(
                &html(
                    "Alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima",
                    "<p class='next'>narrow body</p>",
                ),
                &css,
            )
            .unwrap();
        assert_eq!(document.pages.len(), 2);
        let forms = running_forms(&document);
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].1, Pt::from_i32(260));
        assert_eq!(forms[1].1, Pt::from_i32(140));
        assert!(forms[1].2 > forms[0].2);
        assert!(
            forms
                .iter()
                .all(|entry| form_text(&entry.3).contains("Alpha"))
        );
    }

    #[test]
    fn deferred_running_layout_uses_slot_width_instead_of_body_box_width() {
        let engine = FullBleed::builder().build().unwrap();
        let css = format!(
            "{BASE} @page {{ @top-left {{ width:100pt; }} }} body {{ width:40pt; padding:3pt; }} .band {{ padding:4pt; }}"
        );
        let document = engine
            .render_to_document(&html("Alpha bravo charlie delta echo foxtrot", ""), &css)
            .unwrap();
        let forms = running_forms(&document);
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].1, Pt::from_i32(100));
        assert!(forms[0].2 > Pt::from_i32(12));
        assert!(form_text(&forms[0].3).contains("foxtrot"));
    }

    #[test]
    fn deferred_running_layout_reports_source_and_final_page_for_overflow() {
        let engine = FullBleed::builder().build().unwrap();
        let css = format!(
            "{BASE} @page narrow {{ margin-top:20pt; }} .next {{ page:narrow; break-before:page; }} .band {{ height:40pt; }}"
        );
        let error = engine
            .render_to_document(&html("Header", "<p class='next'>next</p>"), &css)
            .unwrap_err();
        assert!(matches!(error, FullBleedError::UnplaceableFlowable(_)));
        let text = error.to_string();
        for part in [
            "\"page\":2",
            "\"slot\":\"top-left\"",
            "\"reason\":\"measured_box_overflow\"",
            "\"source_ids\":[\"header\"]",
            "\"height\":40000",
        ] {
            assert!(text.contains(part), "{text}");
        }
    }

    #[test]
    fn deferred_running_layout_rejects_a_fixed_child_wider_than_its_slot() {
        let engine = FullBleed::builder().build().unwrap();
        let css =
            format!("{BASE} @page {{ @top-left {{ width:80pt; }} }} .band {{ width:200pt; }}");
        let error = engine
            .render_to_document(&html("Header", ""), &css)
            .unwrap_err()
            .to_string();
        assert!(error.contains("measured_box_overflow"), "{error}");
        assert!(error.contains("\"width\":200000"), "{error}");
    }

    #[test]
    fn deferred_running_layout_rejects_nested_running_content() {
        let engine = FullBleed::builder().build().unwrap();
        let source = html(
            "Outer <div style='position:running(nested)'>Inner</div>",
            "",
        );
        let error = engine
            .render_to_document(&source, BASE)
            .unwrap_err()
            .to_string();
        assert!(error.contains("nested_running_content"), "{error}");
    }

    #[test]
    fn deferred_running_layout_respects_first_page_suppression_and_empty_margins() {
        let engine = FullBleed::builder().build().unwrap();
        let css = format!(
            "{BASE} @page:first {{ @top-left {{ content:none; }} }} .next {{ break-before:page; }}"
        );
        let document = engine
            .render_to_document(&html("Header", "<p class='next'>next</p>"), &css)
            .unwrap();
        let forms = running_forms(&document);
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].0, 2);
        let zero = format!("{BASE} @page {{ margin-top:0; }}");
        let error = engine
            .render_to_document(&html("Header", ""), &zero)
            .unwrap_err()
            .to_string();
        assert!(error.contains("empty_slot"), "{error}");
    }

    #[test]
    fn ordinary_running_content_keeps_original_capture_and_overflow_behavior() {
        let engine = FullBleed::builder().build().unwrap();
        let css = format!(
            "{BASE} @page {{ margin-top:20pt; @top-left {{ width:80pt; }} }} .band {{ width:200pt; height:40pt; }}"
        );
        let ordinary = html("Header", "").replace(" data-fb-running-layout='margin'", "");
        let document = engine.render_to_document(&ordinary, &css).unwrap();
        let forms = running_forms(&document);
        assert_eq!(forms.len(), 1);
        assert_eq!(
            (forms[0].1, forms[0].2),
            (Pt::from_i32(200), Pt::from_i32(40))
        );
    }

    #[test]
    fn ordinary_empty_margin_boxes_keep_noop_output() {
        let engine = FullBleed::builder().build().unwrap();
        let css = "@page {size:300pt 220pt;margin:60pt 20pt 20pt 0} body {margin:0;font:10pt Helvetica} .band {position:running(head)}";
        let source = html("Margin", "").replace(" data-fb-running-layout='margin'", "");
        let expected = engine.render_to_buffer(&source, css).unwrap();
        for content in ["\"Margin\"", "element(head)"] {
            let actual = engine
                .render_to_buffer(
                    &source,
                    &format!("{css} @page {{ @top-left-corner {{content:{content};width:40pt;background:#d9ede3}} }}"),
                )
                .unwrap();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn deferred_running_layout_is_isolated_between_cached_and_parallel_renders() {
        let engine = FullBleed::builder().build().unwrap();
        std::thread::scope(|scope| {
            let jobs = (0..8)
                .map(|index| {
                    let engine = &engine;
                    scope.spawn(move || {
                        let label = format!("RECORD{index}");
                        let document = engine.render_to_document(&html(&label, ""), BASE).unwrap();
                        let forms = running_forms(&document);
                        assert_eq!(forms.len(), 1);
                        assert_eq!(form_text(&forms[0].3), label);
                    })
                })
                .collect::<Vec<_>>();
            for job in jobs {
                job.join().unwrap();
            }
        });
    }

    #[test]
    fn running_layout_rechecks_a_binding_shared_with_body_text() {
        let engine = FullBleed::builder().build().unwrap();
        let template = html("{{name}}", "").replace("<p>body</p>", "<p>{{name}}</p>");
        for source in [
            template.clone(),
            template.replace(" data-fb-running-layout='margin'", ""),
        ] {
            let compiled = engine.compile_document(&source, BASE).unwrap();
            let plan = compiled.compiled_reflow_plan().unwrap();
            let mut document = plan.template.instantiate();
            let mut glyphs = crate::CompiledFlowWorkerGlyphs::new();
            let names = ["Ada", "Beatrice", "Ada"].map(str::to_string);
            for (row, name) in names.iter().enumerate() {
                let record = plan
                    .render_compiled_flow_record(row, &mut document, &mut glyphs, &[&names])
                    .unwrap();
                let forms = running_forms(&record.program.document);
                assert_eq!(forms.len(), 1);
                assert_eq!(form_text(&forms[0].3), *name);
            }
        }
    }

    #[test]
    fn deferred_running_layout_is_checked_again_for_compiled_reflow_bindings() {
        let engine = FullBleed::builder().build().unwrap();
        let css = format!("{BASE} @page {{ @top-left {{ width:100pt; }} }}");
        let compiled = engine
            .compile_document(&html("{{name}}", ""), &css)
            .unwrap();
        let valid = HashMap::from([(
            "name".to_string(),
            vec!["Ada".to_string(), "Beatrice".to_string()],
        )]);
        let first = compiled.render_reflow_bindings_to_buffer(&valid).unwrap();
        assert_eq!(
            first,
            compiled.render_reflow_bindings_to_buffer(&valid).unwrap()
        );
        let invalid = HashMap::from([(
            "name".to_string(),
            vec!["Ada".to_string(), "words ".repeat(300)],
        )]);
        let error = compiled
            .render_reflow_bindings_to_buffer(&invalid)
            .unwrap_err()
            .to_string();
        assert!(error.contains("measured_box_overflow"), "{error}");
        assert!(error.contains("\"source_ids\":[\"header\"]"), "{error}");
        assert!(error.contains("\"record\":2"), "{error}");
        assert_eq!(
            first,
            compiled.render_reflow_bindings_to_buffer(&valid).unwrap()
        );
    }
}
