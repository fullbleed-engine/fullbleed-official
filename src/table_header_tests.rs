//! End-to-end regressions: authored HTML associations must reach the PDF target.
use crate::{AuthoringCancellationToken, AuthoringPreviewRequest, FullBleed, pdf};

fn tagged(html: &str, css: &str) -> Vec<u8> {
    let engine = FullBleed::builder().build().unwrap();
    let document = engine.render_to_document(html, css).unwrap();
    pdf::document_to_pdf_with_metrics_and_registry_with_logs(
        &document,
        None,
        Some(engine.font_registry.as_ref()),
        &pdf::PdfOptions {
            pdf_profile: pdf::PdfProfile::Tagged,
            ..Default::default()
        },
        None,
        None,
    )
    .unwrap()
}

#[test]
fn explicit_table_headers_reach_pdf_id_tree_and_table_attributes() {
    let html = "<table><thead><tr><th id='item'>Item</th><th id='amount'>Amount</th></tr></thead><tbody><tr><td headers='item'>Coffee</td><td headers='item amount'>12.50</td></tr></tbody></table>";
    let css = "@page {size:300pt 300pt;margin:12pt;} table {width:100%;} th,td {padding:4pt;}";
    let bytes = tagged(html, css);
    let pdf = String::from_utf8_lossy(&bytes);
    assert!(
        pdf.contains("/IDTree "),
        "Explicit header IDs need a structure ID name tree"
    );
    let cells: Vec<_> = pdf
        .lines()
        .filter(|line| line.contains("/S /TD "))
        .collect();
    assert_eq!(cells.len(), 2);
    assert!(
        cells.iter().all(|line| line.contains("/Headers [")),
        "{cells:#?}"
    );
    assert_eq!(
        bytes,
        tagged(html, css),
        "Process-global table counters must not affect PDF identity"
    );
}

#[test]
fn a_header_on_an_earlier_page_stays_in_one_logical_pdf_table() {
    let rows = (0..12)
        .map(|index| {
            format!(
                "<tr><td headers='group'>Record {index}</td><td headers='amount'>12.50</td></tr>"
            )
        })
        .collect::<String>();
    let html = format!(
        "<table><thead><tr><th>Item</th><th id='amount'>Amount</th></tr></thead><tbody><tr><th id='group' colspan='2' scope='rowgroup'>Account group</th></tr>{rows}</tbody></table>"
    );
    let bytes = tagged(
        &html,
        "@page {size:300pt 150pt;margin:10pt;} body {margin:0;} table {width:100%;} tr {height:24pt;} th,td {font-size:10pt;padding:2pt;}",
    );
    let pdf = String::from_utf8_lossy(&bytes);
    assert!(
        pdf.lines()
            .filter(|line| line.contains("/Type /Page "))
            .count()
            >= 3
    );
    assert_eq!(
        pdf.lines()
            .filter(|line| line.contains("/S /Table "))
            .count(),
        1,
        "A paginated source table must remain one logical table in the compiled structure"
    );
    assert_eq!(
        pdf.lines()
            .filter(|line| line.contains("/S /TD ") && line.contains("/Headers ["))
            .count(),
        24
    );
}

#[test]
fn table_header_ids_are_instance_scoped_even_when_a_caller_reuses_its_record_id() {
    let engine = FullBleed::builder().build().unwrap();
    let document = engine
        .render_to_document(
            "<table><tr><th id='amount'>Amount</th><td headers='amount'>12</td></tr></table>",
            "",
        )
        .unwrap();
    let render = || {
        let mut bytes = Vec::new();
        let mut stream = pdf::PdfStreamWriter::new(
            &mut bytes,
            document.page_size,
            Some(engine.font_registry.as_ref()),
            pdf::PdfOptions {
                pdf_profile: pdf::PdfProfile::Tagged,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();
        stream.add_document(7, &document).unwrap();
        stream.add_document(7, &document).unwrap();
        stream
            .add_compiled_flow_document(
                7,
                &document,
                document.pages.iter().map(|_| Vec::new()).collect(),
                Vec::new(),
                None,
            )
            .unwrap();
        stream.finish().unwrap();
        bytes
    };
    let bytes = render();
    assert_eq!(bytes, render());
    let text = String::from_utf8_lossy(&bytes);
    let ids: Vec<_> = text
        .lines()
        .filter(|line| line.contains("/S /TH "))
        .map(|line| {
            line.split(" /ID ")
                .nth(1)
                .unwrap()
                .split(')')
                .next()
                .unwrap()
                .to_string()
                + ")"
        })
        .collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(
        ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
        3
    );
    let cells: Vec<_> = text
        .lines()
        .filter(|line| line.contains("/S /TD "))
        .collect();
    for (cell, id) in cells.iter().zip(ids) {
        assert!(cell.contains(&format!("/Headers [{id}]")), "{cell}");
    }
    assert_eq!(
        text.lines()
            .filter(|line| line.contains("/S /Table "))
            .count(),
        3
    );
}

#[test]
fn table_header_empty_explicit_and_unemitted_targets_are_not_implicit_guesses() {
    let html = "<table><tr><th id='visible'>Visible</th><th id='hidden' style='display:none'>Hidden</th><td headers='hidden visible'>Bound</td><td headers>Explicit empty</td><td>Implicit</td></tr></table>";
    let engine = FullBleed::builder().build().unwrap();
    let artifact = engine
        .render_authoring_preview(
            AuthoringPreviewRequest {
                html,
                css: "",
                dpi: 72,
            },
            &AuthoringCancellationToken::new(),
            |_| {},
        )
        .unwrap();
    let mut pending: Vec<_> = artifact
        .reading
        .pages
        .iter()
        .flat_map(|page| page.nodes.iter())
        .collect();
    let mut cells = Vec::new();
    while let Some(node) = pending.pop() {
        if node.role == "TD" {
            cells.push(node);
        }
        pending.extend(&node.children);
    }
    let bound = cells.iter().find(|cell| cell.text == "Bound").unwrap();
    let metadata = bound.table_semantics.as_ref().unwrap();
    assert_eq!(metadata.header_cells.as_ref().unwrap().len(), 1);
    assert_eq!(metadata.header_issues, ["unemitted_header_target"]);
    assert!(
        artifact
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "TABLE_HEADERS_UNEMITTED_HEADER_TARGET")
    );
    let bytes = tagged(html, "");
    let text = String::from_utf8_lossy(&bytes);
    let cells: Vec<_> = text
        .lines()
        .filter(|line| line.contains("/S /TD "))
        .collect();
    assert!(cells[0].contains("/Headers [(fb-"));
    assert!(cells[1].contains("/Headers []"));
    assert!(!cells[2].contains("/Headers"));
    assert_eq!(
        text.lines().filter(|line| line.contains("/S /TH ")).count(),
        1
    );
}

#[test]
fn table_header_links_keep_nested_tables_and_distinct_row_groups_separate() {
    let html = "<table><tbody><tr><th id='outer'>Outer</th><td headers='outer'><table><tr><th id='inner'>Inner</th><td headers='inner outer'>Value</td></tr></table></td></tr></tbody><tbody><tr><td headers='outer'>Next group</td></tr></tbody></table>";
    let bytes = tagged(html, "");
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(
        text.lines()
            .filter(|line| line.contains("/S /Table "))
            .count(),
        2
    );
    assert_eq!(
        text.lines()
            .filter(|line| line.contains("/S /TBody "))
            .count(),
        3
    );
    let ids: Vec<_> = text
        .lines()
        .filter(|line| line.contains("/S /TH "))
        .map(|line| {
            line.split(" /ID ")
                .nth(1)
                .unwrap()
                .split(')')
                .next()
                .unwrap()
                .to_string()
                + ")"
        })
        .collect();
    let cells: Vec<_> = text
        .lines()
        .filter(|line| line.contains("/S /TD "))
        .collect();
    assert!(cells[0].contains(&format!("/Headers [{}]", ids[0])));
    assert!(cells[1].contains(&format!("/Headers [{}]", ids[1])));
    assert!(cells[2].contains(&format!("/Headers [{}]", ids[0])));
}

#[test]
fn table_row_span_crossing_a_forced_page_break_uses_complete_emitted_rows() {
    let html = "<table><tbody><tr><th id='group' rowspan='4'>Group</th><td>A</td></tr><tr style='visibility:collapse'><td>Hidden</td></tr><tr style='break-before:page'><td headers='group'>B</td></tr><tr><td headers='group'>C</td></tr></tbody></table>";
    let css = "@page {size:300pt 150pt;margin:10pt;} table {width:100%;} tr {height:24pt;}";
    let bytes = tagged(html, css);
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.lines()
            .filter(|line| line.contains("/Type /Page "))
            .count()
            >= 2
    );
    let header = text.lines().find(|line| line.contains("/S /TH ")).unwrap();
    assert!(header.contains("/RowSpan 3"), "{header}");
    assert_eq!(
        text.lines().filter(|line| line.contains("/S /TR ")).count(),
        3
    );
    assert_eq!(
        text.lines()
            .filter(|line| line.contains("/S /Table "))
            .count(),
        1
    );
}

#[test]
fn table_header_id_tree_uses_bounded_fanout_without_a_cell_limit() {
    let rows = (0..150)
        .map(|index| {
            format!("<tr><th id='h{index}'>H{index}</th><td headers='h{index}'>Value</td></tr>")
        })
        .collect::<String>();
    let bytes = tagged(
        &format!("<table>{rows}</table>"),
        "@page {size:300pt 600pt;} th,td {font-size:8pt;}",
    );
    let text = String::from_utf8_lossy(&bytes);
    let leaves: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("<< /Names [(fb-"))
        .collect();
    assert_eq!(leaves.len(), 3);
    for leaf in leaves {
        assert!(leaf.contains("/Limits ["));
        assert!(leaf.matches(" 0 R").count() <= 64);
    }
    assert_eq!(
        text.lines()
            .filter(|line| line.contains("/S /TH ") && line.contains(" /ID "))
            .count(),
        150
    );
}

#[test]
fn collapsed_row_border_struts_do_not_become_logical_rows_or_cells() {
    let html = "<table><thead><tr><th>Group</th><th>Value</th></tr></thead><tbody><tr><th id='group' rowspan='4'>Group</th><td>A</td></tr><tr style='visibility:collapse'><td>Hidden</td></tr><tr style='break-before:page'><td headers='group'>B</td></tr><tr><td headers='group'>C</td></tr></tbody></table>";
    let css = "@page {size:300pt 150pt;margin:10pt;} table {width:100%;border-collapse:collapse;} th,td {padding:4pt;border:0.6pt solid #888;}";
    let bytes = tagged(html, css);
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(
        text.lines().filter(|line| line.contains("/S /TR ")).count(),
        4
    );
    assert_eq!(
        text.lines().filter(|line| line.contains("/S /TD ")).count(),
        3
    );
    assert!(
        text.lines()
            .any(|line| line.contains("/S /TH ") && line.contains("/RowSpan 3"))
    );
    let engine = FullBleed::builder().build().unwrap();
    let preview = engine
        .render_authoring_preview(
            AuthoringPreviewRequest { html, css, dpi: 72 },
            &AuthoringCancellationToken::new(),
            |_| {},
        )
        .unwrap();
    let mut pending: Vec<_> = preview
        .reading
        .pages
        .iter()
        .flat_map(|page| page.nodes.iter())
        .collect();
    let mut rows = 0;
    let mut data = 0;
    while let Some(node) = pending.pop() {
        rows += usize::from(node.role == "TR");
        data += usize::from(node.role == "TD");
        if node.role == "TH" && node.text == "Group" && node.row_span != Some(1) {
            assert_eq!(node.logical_row_span, Some(3));
        }
        pending.extend(&node.children);
    }
    assert_eq!((rows, data), (4, 3));
}
