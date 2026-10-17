//! Authoring structure must not change the height of same-font text lines.

use crate::{Command, FullBleed, Pt};

fn engine() -> FullBleed {
    FullBleed::builder()
        .register_font_file(crate::tests::repo_font_path("NotoSans-Regular.ttf"))
        .register_font_file(crate::tests::repo_font_path("Inter-Variable.ttf"))
        .build()
        .expect("engine with bundled fonts")
}

fn rows(variant: &str, count: usize) -> String {
    (0..count)
        .map(|index| {
            let marker = format!("ROW{index:03}");
            let content = match variant {
                "plain" => format!("a {marker}"),
                "span" => format!("<span>a </span>{marker}"),
                "color" => format!("<span class='accent'>a </span>{marker}"),
                "nested" => format!("<span><span class='accent'>a</span> </span>{marker}"),
                "before" | "after" => marker,
                _ => panic!("unknown row variant"),
            };
            format!("<div class='{variant}'>{content}</div>")
        })
        .collect()
}

fn positions(document: &crate::Document, count: usize) -> Vec<(usize, Pt)> {
    (0..count)
        .map(|index| {
            let marker = format!("ROW{index:03}");
            let matches: Vec<_> = document
                .pages
                .iter()
                .enumerate()
                .flat_map(|(page, contents)| {
                    let marker = &marker;
                    contents.commands.iter().filter_map(move |command| {
                        if let Command::DrawString { y, text, .. } = command {
                            text.contains(marker).then_some((page, *y))
                        } else {
                            None
                        }
                    })
                })
                .collect();
            assert_eq!(matches.len(), 1, "{marker} must be drawn exactly once");
            matches[0]
        })
        .collect()
}

fn css(font: &str, size: i32, line_height: &str, page_height: i32) -> String {
    format!(
        "@page {{ size: 500px {page_height}px; margin: 0; }} \
         * {{ margin: 0; padding: 0; }} \
         html, body {{ font-family: '{font}'; font-size: {size}px; line-height: {line_height}; }} \
         .accent, .before::before, .after::after {{ color: #bc4749; }} \
         .before::before {{ content: 'a '; }} .after::after {{ content: ' a'; }}"
    )
}

#[test]
fn inline_text_structure_preserves_authored_line_height() {
    let engine = engine();
    for font in ["Noto Sans", "Inter"] {
        for size in [12, 16, 20, 26, 42] {
            for factor in [1.0, 1.5] {
                let rules = css(font, size, &factor.to_string(), 1000);
                let expected = Pt::from_f32(size as f32 * factor * 0.75);
                for variant in ["plain", "span", "color", "nested", "before", "after"] {
                    let document = engine
                        .render_to_document(&rows(variant, 8), &rules)
                        .expect("render rows");
                    assert_eq!(document.pages.len(), 1);
                    for pair in positions(&document, 8).windows(2) {
                        assert_eq!(
                            pair[1].1 - pair[0].1,
                            expected,
                            "{font} {size}px/{factor} {variant}: inline markup must not add leading"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn inline_text_structure_preserves_page_breaks() {
    let engine = engine();
    for (font, size) in [("Noto Sans", 20), ("Inter", 12)] {
        let rules = css(font, size, "1", size * 8);
        let plain = engine
            .render_to_document(&rows("plain", 24), &rules)
            .expect("render plain text pages");
        let expected = positions(&plain, 24);
        assert_eq!(plain.pages.len(), 3);
        for variant in ["span", "color", "nested", "before", "after"] {
            let document = engine
                .render_to_document(&rows(variant, 24), &rules)
                .expect("render inline text pages");
            assert_eq!(document.pages.len(), plain.pages.len(), "{font} {variant}");
            assert_eq!(
                positions(&document, 24),
                expected,
                "{font} {variant}: same-font inline markup must preserve pagination and baselines"
            );
        }
    }
}
