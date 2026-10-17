//! A failure must name the actual rejected constraint, not an inferred owner.
use crate::{Command, FullBleed};

fn failures(html: &str) -> Vec<(usize, String, Vec<String>)> {
    let document = FullBleed::builder().build().unwrap().render_to_document(
        html,
        "@page{size:240pt 180pt;margin:12pt;}body{margin:0;}p{margin:0;line-height:14pt;}table{width:100%;border-collapse:collapse;}td,th{padding:3pt;vertical-align:top;}",
    ).unwrap();
    document
        .pages
        .iter()
        .enumerate()
        .flat_map(|(page, sheet)| {
            sheet.commands.iter().filter_map(move |command| {
                let Command::Meta { key, value } = command else {
                    return None;
                };
                if key != crate::canvas::META_PAGINATION_EVENT_KEY
                    || !value.contains("reason=forced_unsplittable_full_frame|")
                {
                    return None;
                }
                let field = |name: &str| value.split('|').find_map(|part| part.strip_prefix(name));
                let decode = |text: &str| {
                    String::from_utf8(crate::base64::decode_url_safe_no_pad(text).unwrap()).unwrap()
                };
                let reason = decode(
                    field("split_failure_reason_b64=")
                        .expect("a forced placement must retain its split failure"),
                );
                let sources = field("split_failure_sources_b64=")
                    .unwrap()
                    .split(',')
                    .filter(|id| !id.is_empty())
                    .map(decode)
                    .collect();
                Some((page + 1, reason, sources))
            })
        })
        .collect()
}

#[test]
fn split_failure_reports_table_cell_alignment_through_authored_wrappers() {
    let text = "<p>Long content</p>".repeat(30);
    let html = format!(
        "<div data-fb-id='outer'><div data-fb-id='inner'><table data-fb-id='table'><thead><tr><th>Heading</th></tr></thead><tbody><tr><td data-fb-id='bad-cell' style='vertical-align:bottom'>{text}</td></tr></tbody></table></div></div>"
    );
    assert_eq!(
        failures(&html),
        vec![(1, "table_cell_alignment".into(), vec!["bad-cell".into()])]
    );
}

#[test]
fn split_failure_reports_all_incompatible_cells_without_choosing_one() {
    let text = "<p>Long content</p>".repeat(30);
    let html = format!(
        "<table data-fb-id='table'><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td data-fb-id='a' style='vertical-align:bottom'>{text}</td><td data-fb-id='b' style='vertical-align:middle'>Short</td></tr></tbody></table>"
    );
    assert_eq!(
        failures(&html),
        vec![(
            1,
            "table_cell_alignment".into(),
            vec!["a".into(), "b".into()]
        )]
    );
}

#[test]
fn split_failure_retains_the_original_cell_after_earlier_rows_paginate() {
    let text = "<p>Long content</p>".repeat(30);
    let html = format!(
        "<table data-fb-id='table'><thead><tr><th>A</th></tr></thead><tbody><tr><td data-fb-id='good-cell'>Fits first</td></tr><tr><td data-fb-id='later-cell' style='vertical-align:bottom'>{text}</td></tr></tbody></table>"
    );
    assert_eq!(
        failures(&html),
        vec![(2, "table_cell_alignment".into(), vec!["later-cell".into()])]
    );
}

#[test]
fn split_failure_reports_only_remaining_unsplittable_cell_content() {
    let html = "<table data-fb-id='table'><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td data-fb-id='blocked'><div style='height:500pt;overflow:hidden'>Tall</div></td><td data-fb-id='completed'>Short</td></tr></tbody></table>";
    assert_eq!(
        failures(html),
        vec![(2, "table_cell_content".into(), vec!["blocked".into()])]
    );
}

#[test]
fn split_failure_reports_clipping_owner_and_preserves_arbitrary_ids() {
    let html = "<div data-fb-id='outer'><div data-fb-id='clip,|=&quot;é' style='height:500pt;overflow:hidden'><p data-fb-id='text'>Clipped</p></div></div>";
    assert_eq!(
        failures(html),
        vec![(1, "clipped_container".into(), vec!["clip,|=\"é".into()])]
    );
}

#[test]
fn split_failure_is_absent_for_fitting_and_successfully_continued_content() {
    let fit = "<table><tr><td style='vertical-align:bottom'>Fits</td></tr></table>";
    assert!(failures(fit).is_empty());
    let text = "<p>Long content</p>".repeat(30);
    assert!(failures(&format!("<table><thead><tr><th>A</th></tr></thead><tbody><tr><td data-fb-id='cell'>{text}</td></tr></tbody></table>")).is_empty());
}

#[test]
fn split_failure_reports_the_spanning_cell_that_prevents_continuation() {
    let text = "<p>Long content</p>".repeat(45);
    let html = format!(
        "<table data-fb-id='table'><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td data-fb-id='spanning' rowspan='2'>{text}</td><td>Short</td></tr><tr><td>Following</td></tr></tbody></table>"
    );
    assert_eq!(
        failures(&html),
        vec![(1, "table_rowspan".into(), vec!["spanning".into()])]
    );
}

#[test]
fn split_failure_does_not_blame_body_cells_for_an_oversized_table_header() {
    let html = "<table data-fb-id='table'><thead><tr><th style='height:300pt'>Tall heading</th></tr></thead><tbody><tr><td data-fb-id='body-cell'>Short</td></tr></tbody></table>";
    assert_eq!(
        failures(html),
        vec![(1, "table_fixed_height".into(), vec!["table".into()])]
    );
}

#[test]
fn split_failure_does_not_blame_children_for_parent_padding() {
    let html = "<div data-fb-id='padding-owner' style='padding-top:300pt'><p data-fb-id='child'>Short</p></div>";
    assert_eq!(
        failures(html),
        vec![(
            1,
            "container_decoration_height".into(),
            vec!["padding-owner".into()]
        )]
    );
}

#[test]
fn split_failure_does_not_blame_content_in_a_collapsed_column() {
    let html = "<table data-fb-id='table'><colgroup><col style='visibility:collapse'><col></colgroup><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td data-fb-id='collapsed'>Hidden content</td><td data-fb-id='blocked'><div style='height:500pt;overflow:hidden'>Tall</div></td></tr></tbody></table>";
    assert_eq!(
        failures(html),
        vec![(1, "table_cell_content".into(), vec!["blocked".into()])]
    );
}
