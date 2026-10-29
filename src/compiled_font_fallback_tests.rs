use super::*;

fn engine() -> FullBleed {
    let mut fonts = AssetBundle::default();
    for (name, path) in [
        ("Latin", tests::repo_font_path("NotoSans-Regular.ttf")),
        (
            "CJK",
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/table-wrapping/FullbleedCJKTest-Regular.ttf"),
        ),
    ] {
        fonts.add(Asset::new(
            name.to_string(),
            AssetKind::Font,
            std::fs::read(path).expect("test font"),
            None,
            true,
        ));
    }
    FullBleed::builder().register_bundle(fonts).build().unwrap()
}

fn render(template: &CompiledDocument, value: &str) -> Vec<u8> {
    template
        .render_reflow_bindings_to_buffer(&HashMap::from([(
            "name".to_string(),
            vec![value.to_string()],
        )]))
        .unwrap()
}

fn text(pdf: &[u8]) -> String {
    let document = crate::pdf_native::Document::load_mem(pdf).unwrap();
    let pages = document.get_pages().keys().copied().collect::<Vec<_>>();
    document
        .extract_text_chunks(&pages)
        .into_iter()
        .collect::<crate::pdf_native::Result<Vec<_>>>()
        .unwrap()
        .join("")
        .split_whitespace()
        .collect()
}

#[test]
fn compiled_reflow_font_fallback_matches_a_fresh_template_in_both_directions() {
    let engine = engine();
    for threads in [1, 4] {
        crate::parallel::with_thread_count(threads, || {
            for style in ["normal", "italic"] {
                let css = format!(
                    "@page {{ size: 400pt 200pt; margin: 12pt; }}
                     body {{ font: {style} 12pt Latin, CJK; }}"
                );
                for (first, second) in [
                    ("ABC", "東京"),
                    ("東京", "ABC"),
                    ("ASCII CONTROL", "Before 東京 after"),
                    ("Before 東京 after", "ASCII CONTROL"),
                ] {
                    let template = engine.compile_document("<p>{{name}}</p>", &css).unwrap();
                    let initial = render(&template, first);
                    let rebound = render(&template, second);
                    let fresh = render(
                        &engine.compile_document("<p>{{name}}</p>", &css).unwrap(),
                        second,
                    );
                    assert_eq!(
                        text(&rebound),
                        second.split_whitespace().collect::<String>()
                    );
                    assert!(
                        rebound == fresh,
                        "{threads} workers, {style}, {first:?} -> {second:?}: cached font selection differs"
                    );
                    assert!(
                        initial == render(&template, first),
                        "original record replay"
                    );
                }
            }
        });
    }
}

#[test]
fn compiled_reflow_font_selection_respects_unicode_ranges_after_rebinding() {
    let engine = engine();
    let css = "@font-face { font-family: Ranged; src: url(Latin.ttf); unicode-range: U+0041; }
               @font-face { font-family: Ranged; src: url(CJK.ttf); unicode-range: U+0042; }
               @page { size: 400pt 200pt; margin: 12pt; }
               body { font: 12pt Ranged; }";
    for (first, second) in [("A", "B"), ("B", "A")] {
        let template = engine.compile_document("<p>{{name}}</p>", css).unwrap();
        let initial = render(&template, first);
        let rebound = render(&template, second);
        let fresh = render(
            &engine.compile_document("<p>{{name}}</p>", css).unwrap(),
            second,
        );
        assert_eq!(text(&rebound), second);
        assert!(rebound == fresh, "CSS ranges must reselect the font");
        assert!(initial == render(&template, first));
    }
}

#[test]
fn compiled_reflow_primary_font_rebinding_keeps_the_reusable_program() {
    let engine = engine();
    let template = engine
        .compile_document(
            "<p>{{name}}</p>",
            "@page { size: 400pt 200pt; margin: 12pt; } body { font: 12pt Latin, CJK; }",
        )
        .unwrap();
    for value in ["ALPHA", "BRAVO", "Ωμέγα", "12345", "67890"] {
        assert_eq!(text(&render(&template, value)), value);
    }
    let plan = template.compiled_reflow_plan().unwrap();
    let programs = plan.flow_programs.lock().unwrap();
    let count: usize = programs
        .values()
        .map(|entry| entry.state.lock().unwrap().programs.len())
        .sum();
    assert_eq!(
        count, 1,
        "supported replacement glyphs must retain fast rebinding"
    );
}
