//! Source identities for authored links and their local destinations.
//!
//! Addresses are lookup keys only. Retained commands use document-order ordinals
//! so recompilation is deterministic and page fragments keep the same identity.

use crate::html_dom::NodeRef;
use std::collections::{HashMap, HashSet};

pub(crate) const META_LINK_ID: &str = "__fb_link_id";
pub(crate) const META_LINK_TARGET: &str = "__fb_link_target";
pub(crate) const META_LINK_LABEL: &str = "__fb_link_label";
pub(crate) const META_DESTINATION: &str = "__fb_link_destination";
pub(crate) const META_NAVIGATION_SUPPRESSED: &str = "__fb_navigation_suppressed";

#[derive(Debug, Clone, Default)]
pub(crate) struct HtmlLinkIndex {
    metadata: HashMap<usize, Vec<(String, String)>>,
    ancestors_with_boundaries: HashSet<usize>,
}

/// A fragment-only reference is local to one input document, including when
/// that document is repeated or linked into a multi-recipient PDF job.
pub(crate) fn local_fragment(href: &str) -> Option<String> {
    let href = href.trim_matches(|ch: char| ch <= '\u{20}');
    let fragment = href.strip_prefix('#')?;
    let bytes = fragment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = (bytes[index + 1] as char).to_digit(16);
            let low = (bytes[index + 2] as char).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                decoded.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    Some(String::from_utf8_lossy(&decoded).into_owned())
}

impl HtmlLinkIndex {
    pub(crate) fn build(document: &NodeRef) -> Self {
        let mut index = Self::default();
        let mut references = HashSet::new();
        let mut ids = HashMap::<String, NodeRef>::new();
        let mut names = HashMap::<String, NodeRef>::new();
        for (ordinal, node) in document.descendants().enumerate() {
            let Some(element) = node.as_element() else {
                continue;
            };
            let attributes = element.attributes.borrow();
            if let Some(id) = attributes.get("id").filter(|id| !id.is_empty()) {
                ids.entry(id.to_owned()).or_insert_with(|| node.clone());
            }
            if element.name.local.as_ref() != "a" {
                continue;
            }
            if let Some(name) = attributes.get("name").filter(|name| !name.is_empty()) {
                names.entry(name.to_owned()).or_insert_with(|| node.clone());
            }
            let Some(href) = attributes.get("href") else {
                continue;
            };
            if let Some(fragment) = local_fragment(href).filter(|value| !value.is_empty()) {
                references.insert(fragment);
            }
            let mut metadata = vec![
                (META_LINK_ID.into(), (ordinal + 1).to_string()),
                (META_LINK_TARGET.into(), href.to_owned()),
            ];
            if let Some(label) = attributes
                .get("aria-label")
                .filter(|s| !s.trim().is_empty())
            {
                metadata.push((META_LINK_LABEL.into(), label.to_owned()));
            }
            index.add(&node, metadata);
        }
        // An ID takes precedence over a legacy named anchor. Duplicate IDs and
        // names resolve to their first source occurrence, not their paint order.
        let mut references = references.into_iter().collect::<Vec<_>>();
        references.sort();
        for name in references {
            if let Some(node) = ids.get(&name).or_else(|| names.get(&name)) {
                index.add(node, vec![(META_DESTINATION.into(), name)]);
            }
        }
        index
    }

    fn add(&mut self, node: &NodeRef, metadata: Vec<(String, String)>) {
        self.metadata
            .entry(node.identity_key())
            .or_default()
            .extend(metadata);
        // Only descendants prevent flattening. A target paragraph may still
        // use its ordinary text layout when its children have no boundaries.
        for ancestor in node.ancestors().skip(1) {
            self.ancestors_with_boundaries
                .insert(ancestor.identity_key());
        }
    }

    pub(crate) fn metadata(&self, node: &NodeRef) -> &[(String, String)] {
        self.metadata
            .get(&node.identity_key())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn has_descendant_boundary(&self, node: &NodeRef) -> bool {
        self.ancestors_with_boundaries
            .contains(&node.identity_key())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{
        Command, Document, META_DIAGNOSTIC_SCOPE_BEGIN_KEY, META_DIAGNOSTIC_SCOPE_END_KEY,
    };
    use crate::html_dom::parse_html;

    fn compile(html: &str) -> crate::CompiledDocument {
        crate::FullBleed::builder().build().unwrap().compile_document(
            html,
            "@page { size: 180pt 180pt; margin: 12pt; } body { font-family: Helvetica; font-size: 12pt; line-height: 16pt; } p { margin: 0; }",
        ).unwrap()
    }

    fn painted_targets(
        document: &Document,
    ) -> Vec<(usize, Option<String>, Option<String>, String)> {
        let mut result = Vec::new();
        for (page_index, page) in document.pages.iter().enumerate() {
            let mut scope = (None::<String>, None::<String>);
            let mut stack = Vec::new();
            for command in &page.commands {
                match command {
                    Command::Meta { key, .. } if key == META_DIAGNOSTIC_SCOPE_BEGIN_KEY => {
                        stack.push(scope.clone())
                    }
                    Command::Meta { key, .. } if key == META_DIAGNOSTIC_SCOPE_END_KEY => {
                        scope = stack.pop().expect("balanced semantic scope")
                    }
                    Command::Meta { key, value } if key == META_LINK_ID => {
                        scope.0 = Some(value.clone())
                    }
                    Command::Meta { key, value } if key == META_LINK_TARGET => {
                        scope.1 = Some(value.clone())
                    }
                    Command::DrawString { text, .. }
                    | Command::DrawStringTransformed { text, .. } => {
                        result.push((page_index, scope.0.clone(), scope.1.clone(), text.clone()));
                    }
                    Command::DrawImage { .. } | Command::DrawForm { .. } => {
                        result.push((
                            page_index,
                            scope.0.clone(),
                            scope.1.clone(),
                            "[image]".into(),
                        ));
                    }
                    _ => {}
                }
            }
            assert!(stack.is_empty());
            assert_eq!(scope, (None, None));
        }
        result
    }

    #[test]
    fn links_retain_distinct_deterministic_source_identities() {
        let html = r#"<p><a href="https://example.invalid/a">Same</a><a href="https://example.invalid/b">Same</a><a>Plain</a></p>"#;
        let inspect = || {
            let document = parse_html(html);
            let index = HtmlLinkIndex::build(&document);
            let links = document
                .select("a")
                .unwrap()
                .map(|node| index.metadata(node.as_node()).to_vec())
                .collect::<Vec<_>>();
            let paragraph = document.select_first("p").unwrap();
            assert!(index.has_descendant_boundary(paragraph.as_node()));
            assert!(links[2].is_empty());
            assert_ne!(links[0][0].1, links[1][0].1);
            links
        };
        assert_eq!(inspect(), inspect());
    }

    #[test]
    fn only_referenced_destinations_block_text_flattening() {
        let document = parse_html(
            r##"<a href="#target">Go</a><p><span id="target">Here</span></p><div><span id="unused">Text</span></div>"##,
        );
        let index = HtmlLinkIndex::build(&document);
        let paragraph = document.select_first("p").unwrap();
        let div = document.select_first("div").unwrap();
        let target = document.select_first("#target").unwrap();
        assert!(index.has_descendant_boundary(paragraph.as_node()));
        assert!(!index.has_descendant_boundary(div.as_node()));
        assert_eq!(
            index.metadata(target.as_node()),
            &[(META_DESTINATION.into(), "target".into())]
        );
    }

    #[test]
    fn local_destination_prefers_first_id_over_named_anchor() {
        let document = parse_html(
            r##"<a href="#café">Go</a><a name="café">Legacy</a><p id="café">First</p><p id="café">Second</p>"##,
        );
        let index = HtmlLinkIndex::build(&document);
        let targets = document
            .descendants()
            .filter(|node| {
                index
                    .metadata(node)
                    .iter()
                    .any(|(key, _)| key == META_DESTINATION)
            })
            .collect::<Vec<_>>();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].text_contents(), "First");
    }

    #[test]
    fn fragment_decoding_does_not_confuse_external_urls_with_local_targets() {
        assert_eq!(local_fragment("#caf%C3%A9"), Some("café".into()));
        assert_eq!(
            local_fragment("#literal%bad%"),
            Some("literal\u{fffd}d%".into())
        );
        assert_eq!(local_fragment("#"), Some(String::new()));
        assert_eq!(local_fragment("https://example.invalid/#target"), None);
        assert_eq!(local_fragment("other.pdf#target"), None);
    }

    #[test]
    fn compiled_adjacent_links_keep_targets_out_of_unlinked_text() {
        let compiled = compile(
            r#"<p>Before <a href="https://example.invalid/first">First</a> middle <a href="https://example.invalid/second">Second</a> after <a>Plain</a></p>"#,
        );
        let paint = painted_targets(&compiled.document);
        let first = paint.iter().find(|row| row.3.contains("First")).unwrap();
        let second = paint.iter().find(|row| row.3.contains("Second")).unwrap();
        assert_eq!(first.2.as_deref(), Some("https://example.invalid/first"));
        assert_eq!(second.2.as_deref(), Some("https://example.invalid/second"));
        assert_ne!(first.1, second.1);
        for row in paint.iter().filter(|row| {
            ["Before", "middle", "after", "Plain"]
                .iter()
                .any(|text| row.3.contains(text))
        }) {
            assert_eq!(row.2, None, "unlinked text acquired a target: {row:?}");
        }
    }

    #[test]
    fn link_identity_survives_line_wrapping_and_page_fragmentation() {
        let html = format!(
            "<p><a href='https://example.invalid/wrapped'>{}</a></p>",
            "Document details ".repeat(100)
        );
        let compiled = compile(&html);
        assert!(compiled.page_count() > 1);
        let paint = painted_targets(&compiled.document);
        let keys = paint
            .iter()
            .filter(|row| !row.3.trim().is_empty())
            .map(|row| {
                assert_eq!(row.2.as_deref(), Some("https://example.invalid/wrapped"));
                row.1.clone().expect("source link identity")
            })
            .collect::<HashSet<_>>();
        assert_eq!(keys.len(), 1);
        assert_eq!(
            paint
                .iter()
                .map(|row| row.3.split_whitespace().count())
                .sum::<usize>(),
            200
        );
        assert_eq!(
            paint.iter().map(|row| row.0).collect::<HashSet<_>>().len(),
            compiled.page_count()
        );
    }

    #[test]
    fn links_survive_contents_lists_tables_and_inline_styles() {
        for html in [
            "<p><a style='display:contents' href='https://example.invalid/target'>Document <b>details</b></a></p>",
            "<ul style='list-style-position:inside'><li><a href='https://example.invalid/target'>Document details</a></li></ul>",
            "<ul><li><a href='https://example.invalid/target'>Document details</a></li></ul>",
            "<table><tr><td><a href='https://example.invalid/target'>Document details</a></td></tr></table>",
            "<p><a href='https://example.invalid/target'>Document <span style='color:red'>details</span></a></p>",
        ] {
            let compiled = compile(html);
            let paint = painted_targets(&compiled.document);
            let text = paint
                .iter()
                .filter(|row| row.3.contains("Document") || row.3.contains("details"))
                .collect::<Vec<_>>();
            assert!(!text.is_empty(), "{html}");
            assert!(
                text.iter()
                    .all(|row| row.2.as_deref() == Some("https://example.invalid/target")),
                "{html}: {text:?}"
            );
        }
    }

    #[test]
    fn hidden_links_and_plain_anchors_do_not_mark_visible_paint() {
        let compiled = compile(
            r#"<p><a>Plain</a><a href="https://example.invalid/hidden" style="display:none">Hidden</a><a href="https://example.invalid/invisible" style="visibility:hidden">Invisible</a></p>"#,
        );
        let paint = painted_targets(&compiled.document);
        assert!(paint.iter().any(|row| row.3.contains("Plain")));
        assert!(paint.iter().all(|row| row.2.is_none()), "{paint:?}");
    }

    #[test]
    fn referenced_inline_destinations_survive_parent_flattening() {
        let compiled = compile(
            r##"<p><a href="#details">Go</a></p><p>Before <span id="details">Here</span> after</p>"##,
        );
        assert!(compiled.document.pages.iter().flat_map(|page| &page.commands).any(|command| matches!(command, Command::Meta { key, value } if key == META_DESTINATION && value == "details")));
    }

    #[test]
    fn hidden_link_does_not_change_visible_text_layout() {
        let plain = compile("<p>Before after</p>").render_to_buffer().unwrap();
        let hidden = compile("<p>Before<a href='https://example.invalid/hidden' style='display:none'>Hidden</a> after</p>").render_to_buffer().unwrap();
        assert_eq!(plain, hidden);
    }

    #[test]
    fn link_semantics_preserve_advances_within_a_source_text_run() {
        let engine = crate::FullBleed::builder()
            .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
            .build()
            .unwrap();
        let compiled = engine.compile_document(
            "<p><a href='https://example.invalid/target'>Documents</a> for this order.</p>",
            "@page{size:240pt 240pt;margin:20pt}body{font-family:'Noto Sans';font-size:12pt;line-height:16pt}p{margin:0 0 12pt}",
        ).unwrap();
        let origins = compiled.document.pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::DrawString { x, text, .. } => Some((text.as_str(), x.to_f32())),
                _ => None,
            })
            .collect::<Vec<_>>();
        // Chromium rounds at DOM text-run boundaries, but does not round
        // each word inside that run. The bundled font's advances therefore
        // stay exact between these words, including the intervening space.
        let origin = |word: &str| origins.iter().find(|(text, _)| *text == word).unwrap().1;
        assert_eq!(origin("Documents"), 20.0);
        for (left, right, expected) in [("for", "this", 19.464), ("this", "order.", 23.712)] {
            let actual = origin(right) - origin(left);
            assert!(
                (actual - expected).abs() < 0.0006,
                "{left} -> {right}: {actual}, expected {expected}; {origins:?}"
            );
        }
    }

    #[test]
    fn ordinary_styled_inline_text_paginates_without_losing_words() {
        let html = format!(
            "<p><span style='color:red'>{}</span></p>",
            "Document details ".repeat(100)
        );
        let compiled = compile(&html);
        assert!(compiled.page_count() > 1);
        let paint = painted_targets(&compiled.document);
        assert_eq!(
            paint
                .iter()
                .map(|row| row.3.split_whitespace().count())
                .sum::<usize>(),
            200
        );
        assert!(paint.iter().all(|row| row.2.is_none()));
    }

    #[test]
    fn link_semantics_keep_run_advances_across_wrapped_pages() {
        let engine = crate::FullBleed::builder()
            .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
            .build()
            .unwrap();
        let html = format!(
            "<p><a href='https://example.invalid/target'>{}</a></p>",
            "Documents for this order. ".repeat(40)
        );
        let compiled = engine.compile_document(&html,
            "@page{size:180pt 180pt;margin:20pt}body{font-family:'Noto Sans';font-size:12pt;line-height:16pt}p{margin:0}").unwrap();
        assert!(compiled.document.pages.len() > 2);
        let mut checked = 0;
        let mut word_count = 0;
        for page in &compiled.document.pages {
            let words = page
                .commands
                .iter()
                .filter_map(|command| match command {
                    Command::DrawString { x, y, text } => Some((text.as_str(), *x, *y)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            word_count += words.len();
            for pair in words.windows(2) {
                if pair[0].2 != pair[1].2 {
                    continue;
                }
                let expected = match (pair[0].0, pair[1].0) {
                    ("Documents", "for") => 67.800,
                    ("for", "this") => 19.464,
                    ("this", "order.") => 23.712,
                    _ => continue,
                };
                let advance = (pair[1].1 - pair[0].1).to_f32();
                assert!((advance - expected).abs() < 0.0006, "{pair:?}: {advance}");
                checked += 1;
            }
        }
        assert_eq!(word_count, 160);
        assert!(checked > 40);
    }

    #[test]
    fn link_semantics_paint_origin_does_not_leak_or_enter_serialized_output() {
        let engine = crate::FullBleed::builder()
            .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
            .build()
            .unwrap();
        let html = "<p><a href='https://example.invalid/target'>Documents</a> for this order.</p><p>Next plain paragraph.</p>";
        let css = "@page{size:240pt 240pt;margin:20pt}body{font-family:'Noto Sans';font-size:12pt;line-height:16pt}p{margin:0 0 12pt}";
        let first = engine.compile_document(html, css).unwrap();
        let second = engine.compile_document(html, css).unwrap();
        assert_eq!(
            first.render_to_buffer().unwrap(),
            second.render_to_buffer().unwrap()
        );
        let plain = first
            .document
            .pages
            .iter()
            .flat_map(|page| &page.commands)
            .find_map(|command| match command {
                Command::DrawString { x, text, .. } if text == "Next plain paragraph." => Some(*x),
                _ => None,
            })
            .expect("following ordinary paragraph");
        assert_eq!(plain, crate::types::Pt::from_f32(20.0));
    }

    #[test]
    fn link_semantics_reflow_uses_the_shared_paint_origin_and_repositions_changed_words() {
        let engine = crate::FullBleed::builder()
            .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
            .build()
            .unwrap();
        let template =
            "<p><a href='https://example.invalid/target'>Documents</a> {{word}} this order.</p>";
        let css = "@page{size:240pt 240pt;margin:20pt}body{font-family:'Noto Sans';font-size:12pt;line-height:16pt}p{margin:0 0 12pt}";
        let compiled = engine.compile_document(template, css).unwrap();
        // Reuse one compiled program across equal, different, and wrapped
        // values. Its optimized path must preserve ordinary-render geometry.
        for word in ["for", "with", "for", "with a longer description for"] {
            let bindings = HashMap::from([("word".to_owned(), vec![word.to_owned()])]);
            let reflow = compiled
                .render_reflow_bindings_to_buffer(&bindings)
                .unwrap();
            let direct = engine
                .render_to_buffer(&template.replace("{{word}}", word), css)
                .unwrap();
            assert_eq!(reflow, direct, "reflow mismatch for {word:?}");
        }
    }
}
