use super::*;

fn extracted(pdf: &[u8]) -> (usize, String) {
    let inspection = inspect_pdf_bytes(pdf).expect("inspect PDF");
    let parsed = crate::pdf_native::Document::load_mem(pdf).expect("parse PDF");
    let pages = parsed.get_pages().keys().copied().collect::<Vec<_>>();
    let text = parsed
        .extract_text_chunks(&pages)
        .into_iter()
        .collect::<crate::pdf_native::Result<Vec<_>>>()
        .expect("extract text")
        .join("\n");
    (inspection.page_count, text)
}

fn compact(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn css(white_space: &str, wrapping: &str) -> String {
    format!(
        "@page {{ size:120pt 90pt; margin:10pt; }}
         body {{ margin:0; font-family:Courier; font-size:10pt; line-height:12pt; }}
         pre {{ margin:0; white-space:{white_space}; {wrapping} }}"
    )
}

#[test]
fn preformatted_css_wrapping_preserves_text_across_pages() {
    let engine = FullBleed::builder().build().expect("engine");
    let text = format!("  PREFIX {}\n  SUFFIX", "abcdefghij".repeat(30));
    let html = format!("<pre>{text}</pre>");
    for (white_space, wrapping) in [
        ("pre-wrap", "overflow-wrap:anywhere"),
        ("pre-wrap", "overflow-wrap:break-word"),
        ("pre-wrap", "word-break:break-all"),
        ("pre-line", "overflow-wrap:anywhere"),
        ("break-spaces", "overflow-wrap:anywhere"),
        ("normal", "overflow-wrap:anywhere"),
    ] {
        let style = css(white_space, wrapping);
        let direct = engine.render_to_buffer(&html, &style).expect("direct PDF");
        let compiled = engine.compile_document(&html, &style).expect("compile");
        let cached = compiled.render_to_buffer().expect("compiled PDF");
        for (path, pdf) in [("direct", direct), ("compiled", cached)] {
            let (pages, actual) = extracted(&pdf);
            assert!(
                pages > 1,
                "{path}: {white_space}; {wrapping} must wrap and paginate"
            );
            assert_eq!(
                compact(&actual),
                compact(&text),
                "{path}: {white_space}; {wrapping}"
            );
        }
    }
}

#[test]
fn preformatted_css_wraps_at_word_boundaries_without_overflow_wrapping() {
    let engine = FullBleed::builder().build().expect("engine");
    let text = format!("  {}\n  SUFFIX", "word ".repeat(70));
    for white_space in ["pre-wrap", "pre-line", "normal", "break-spaces"] {
        let pdf = engine
            .render_to_buffer(&format!("<pre>{text}</pre>"), &css(white_space, ""))
            .expect("PDF");
        let (pages, actual) = extracted(&pdf);
        assert!(pages > 1, "{white_space} must soft-wrap words");
        assert_eq!(compact(&actual), compact(&text));
    }
}

#[test]
fn preformatted_default_and_explicit_nowrap_remain_unwrapped() {
    let engine = FullBleed::builder().build().expect("engine");
    let text = format!("  PREFIX {}\n  SUFFIX", "abcdefghij".repeat(30));
    for style in [
        css("pre", "overflow-wrap:anywhere"),
        css("nowrap", "overflow-wrap:anywhere"),
        css("pre-wrap", "text-wrap:nowrap; overflow-wrap:anywhere"),
    ] {
        let pdf = engine
            .render_to_buffer(&format!("<pre>{text}</pre>"), &style)
            .expect("PDF");
        let (pages, actual) = extracted(&pdf);
        assert_eq!(pages, 1, "An explicit no-wrap policy must be respected");
        assert_eq!(compact(&actual), compact(&text));
    }
    let default_css = "@page { size:120pt 90pt; margin:10pt; } body { margin:0; font-size:10pt; line-height:12pt; } pre { margin:0; overflow-wrap:anywhere; }";
    let pdf = engine
        .render_to_buffer(&format!("<pre>{text}</pre>"), default_css)
        .expect("default pre PDF");
    assert_eq!(extracted(&pdf).0, 1);
}

#[test]
fn preformatted_reflow_bindings_follow_computed_wrapping() {
    let engine = FullBleed::builder().build().expect("engine");
    let text = format!("  RECORD {}\n  END", "abcdefghij".repeat(30));
    for white_space in ["pre-wrap", "pre-line", "break-spaces"] {
        let template = engine
            .compile_document(
                "<pre>{{content}}</pre>",
                &css(white_space, "overflow-wrap:anywhere"),
            )
            .expect("compile reflow");
        let bindings =
            std::collections::HashMap::from([("content".to_string(), vec![text.clone()])]);
        let pdf = template
            .render_reflow_bindings_to_buffer(&bindings)
            .expect("reflow PDF");
        let (pages, actual) = extracted(&pdf);
        assert!(pages > 1, "reflow {white_space} must wrap");
        assert_eq!(compact(&actual), compact(&text));
    }
}
