use crate::FullBleed;
use std::collections::HashMap;

const BASE: &str = r#"@page {size:320pt 240pt;margin:80pt 20pt 40pt;
@top-left {content:element(head);width:100pt}}
* {margin:0;box-sizing:border-box}
html,body {font:10pt/12pt "Fullbleed Preview Mono"}
.band {position:running(head);background:#d9ede3;height:30pt}
.child {background:#244c85;color:white;height:20pt;width:80pt}"#;

fn engine() -> FullBleed {
    FullBleed::builder()
        .register_font_file(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/preview_fonts/FullbleedPreviewMono-Regular.ttf"),
        )
        .build()
        .unwrap()
}
fn html(text: &str) -> String {
    format!(
        r#"<html><body><div class="band" data-fb-running-layout="band" data-fb-id="band"><div class="child">{text}</div></div><p>Body control</p></body></html>"#
    )
}

#[test]
fn managed_paint_rejects_the_retained_silent_clipping_probes() {
    let engine = engine();
    for (name, css) in [
        ("wide", ".child {width:150pt}"),
        ("tall", ".child {height:70pt}"),
        ("relative", ".child {position:relative;left:65pt}"),
        ("absolute", ".child {position:absolute;left:65pt}"),
        ("translate", ".child {transform:translateX(65pt)}"),
        ("rotate", ".child {transform:rotate(30deg)}"),
    ] {
        let error = engine
            .render_to_buffer(&html("VISIBLE"), &format!("{BASE} {css}"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("paint_overflow"), "{name}: {error}");
        assert!(
            error.contains("\"page\":1")
                && error.contains("\"slot\":\"top-left\"")
                && error.contains("\"source_ids\":[\"band\"]"),
            "{error}"
        );
        assert!(error.contains("\"paint\":{"), "{error}");
    }
    let error = engine
        .render_to_buffer(
            &html("VISIBLE"),
            &format!("{BASE} .child {{box-shadow:25pt 0 5pt #f00}}"),
        )
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("paint_bounds_unknown") && error.contains("filtered_form_extent"),
        "{error}"
    );
}

#[test]
fn managed_paint_accepts_control_and_explicit_authored_clipping() {
    let engine = engine();
    for css in ["", ".band {overflow:hidden} .child {width:150pt}"] {
        let source = html("VISIBLE");
        let css = format!("{BASE} {css}");
        let managed = engine.render_to_buffer(&source, &css).unwrap();
        let measured = engine
            .render_to_buffer(
                &source.replace("layout=\"band\"", "layout=\"margin\""),
                &css,
            )
            .unwrap();
        assert!(!managed.is_empty() && !measured.is_empty());
        assert_eq!(managed, engine.render_to_buffer(&source, &css).unwrap());
    }
}

#[test]
fn managed_paint_draws_zero_height_before_deciding_it_is_empty() {
    let engine = engine();
    let css = format!("{BASE} .band {{height:0}} .child {{position:absolute;top:0;left:0}}");
    let error = engine
        .render_to_buffer(&html("VISIBLE"), &css)
        .unwrap_err()
        .to_string();
    assert!(error.contains("paint_overflow"), "{error}");
    assert!(error.contains("\"height\":0"), "{error}");
    let empty = html("").replace("<div class=\"child\"></div>", "");
    engine.render_to_buffer(&empty, &css).unwrap();
}

#[test]
fn managed_paint_rechecks_final_named_page_slot_and_reserved_margin() {
    let engine = engine();
    let source = html("VISIBLE").replace("</body>", "<p class='next'>Next</p></body>");
    let css = format!(
        "{BASE} @page narrow {{size:160pt 240pt;@top-left {{width:50pt}}}} .next {{page:narrow;break-before:page}}"
    );
    let error = engine
        .render_to_buffer(&source, &css)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("paint_overflow") && error.contains("\"page\":2"),
        "{error}"
    );
    let css = format!("{BASE} @page {{@top-left {{width:400pt}}}}");
    let error = engine
        .render_to_buffer(&html("VISIBLE"), &css)
        .unwrap_err()
        .to_string();
    assert!(error.contains("slot_outside_reserved_margin"), "{error}");
}

#[test]
fn managed_paint_unknowns_are_source_aware_and_leave_legacy_paths_alone() {
    let engine = FullBleed::builder().build().unwrap();
    let css = format!("{BASE} html,body {{font-family:Helvetica}}");
    let source = html("VISIBLE");
    let error = engine
        .render_to_buffer(&source, &css)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("paint_bounds_unknown") && error.contains("unresolved_paint_font"),
        "{error}"
    );
    for source in [
        source.replace("layout=\"band\"", "layout=\"margin\""),
        source.replace(" data-fb-running-layout=\"band\"", ""),
    ] {
        engine.render_to_buffer(&source, &css).unwrap();
    }
}

#[test]
fn managed_paint_rechecks_each_record_after_empty_binding() {
    let engine = engine();
    let source = html("{{name}}");
    let compiled = engine.compile_document(&source, BASE).unwrap();
    let valid = HashMap::from([(
        "name".to_string(),
        vec!["".to_string(), "VISIBLE".to_string()],
    )]);
    let before = compiled.render_reflow_bindings_to_buffer(&valid).unwrap();
    let invalid = HashMap::from([(
        "name".to_string(),
        vec!["".to_string(), "Overflow words ".repeat(20)],
    )]);
    let error = compiled
        .render_reflow_bindings_to_buffer(&invalid)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("\"record\":2") && error.contains("paint_overflow"),
        "{error}"
    );
    assert_eq!(
        compiled.render_reflow_bindings_to_buffer(&valid).unwrap(),
        before
    );
}
