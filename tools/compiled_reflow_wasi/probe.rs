// SPDX-License-Identifier: MIT
// Run by tools/verify_compiled_reflow_wasi.py against the checkout's core.
use fullbleed::{
    Asset, AssetBundle, AssetKind, AuthoringCancellationToken, CompiledBatchReport,
    CompiledFlowCompression, CompiledReflowOptions, FullBleed, FullBleedError, inspect_pdf_bytes,
};
use std::collections::HashMap;

fn report_json(report: &CompiledBatchReport) -> String {
    let records = report
        .records
        .iter()
        .map(|row| {
            let missing = row
                .glyph_report
                .missing()
                .iter()
                .map(|glyph| {
                    format!(
                        "{{\"codepoint\":{},\"count\":{}}}",
                        glyph.codepoint, glyph.count
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "{{\"record_index\":{},\"page_offset\":{},\"page_count\":{},\"missing\":[{}]}}",
                row.record_index, row.page_offset, row.page_count, missing
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"page_count\":{},\"records\":[{}]}}\n",
        report.page_count, records
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut fonts = AssetBundle::default();
    fonts.add(Asset::new(
        "ProbeFont".to_string(),
        AssetKind::Font,
        std::fs::read("NotoSans-Regular.ttf")?,
        None,
        true,
    ));
    let engine = FullBleed::builder().register_bundle(fonts).build()?;
    let template = engine.compile_document(
        "<h1>Account {{account}}</h1><p>{{message}}</p><p>END {{account}}</p>",
        "@page { size: 220pt 160pt; margin: 16pt; } body { font: 10pt ProbeFont; line-height: 12pt; } h1 { font-size: 14pt; }",
    )?;
    assert!(template.reflow_program_ready());
    assert_eq!(template.reflow_binding_slots(), &["account", "message"]);
    let first = HashMap::from([
        (
            "account".to_string(),
            vec!["1042".to_string(), "1043".to_string()],
        ),
        (
            "message".to_string(),
            vec![
                "Active.".to_string(),
                "This longer notice continues onto another page. ".repeat(20),
            ],
        ),
    ]);
    let rebound = HashMap::from([
        (
            "account".to_string(),
            vec!["2042".to_string(), "2043".to_string()],
        ),
        (
            "message".to_string(),
            vec![
                "Žlutý kůň. Łódź café.".to_string(),
                "Ωμέγα résumé. ".repeat(20),
            ],
        ),
    ]);
    let mut summaries = Vec::new();
    for (compression_name, compression) in [
        ("throughput", CompiledFlowCompression::Throughput),
        ("compact", CompiledFlowCompression::Compact),
    ] {
        let options = CompiledReflowOptions { compression };
        let mut original = Vec::new();
        for (index, (name, bindings)) in [
            ("first", &first),
            ("repeat", &first),
            ("rebound", &rebound),
            ("restored", &first),
        ]
        .into_iter()
        .enumerate()
        {
            let pdf = template.render_reflow_bindings_to_buffer_with_options(bindings, options)?;
            let pages = inspect_pdf_bytes(&pdf)?.page_count;
            let (diagnosed, report) = template
                .render_reflow_bindings_to_buffer_with_options_and_report(bindings, options)?;
            assert_eq!(diagnosed, pdf, "reporting changed the PDF");
            assert_eq!(report.page_count, pages);
            assert_eq!(report.records.len(), 2);
            let mut offset = 0;
            for (record, row) in report.records.iter().enumerate() {
                assert_eq!(row.record_index, record);
                assert_eq!(row.page_offset, offset);
                assert!(row.glyph_report.is_empty());
                offset += row.page_count;
            }
            assert_eq!(offset, pages);
            assert!(pages >= 2);
            if index == 0 {
                assert!(pages > 2);
                original = pdf.clone();
            }
            if index == 1 || index == 3 {
                assert_eq!(pdf, original, "replay changed the PDF");
            }
            if index == 0 || index == 2 {
                let stem = format!("{compression_name}-{name}");
                let filename = format!("{stem}.pdf");
                std::fs::write(&filename, &pdf)?;
                std::fs::write(format!("{stem}-report.json"), report_json(&report))?;
                let images = engine.render_finalized_pdf_image_pages(&filename, 72)?;
                assert_eq!(images.len(), pages);
                for (page, png) in images.iter().enumerate() {
                    std::fs::write(format!("{stem}-{}.png", page + 1), png)?;
                }
                summaries.push(format!(
                    "{{\"name\":\"{stem}\",\"pages\":{pages},\"records\":2}}"
                ));
            }
        }
    }
    let cancellation = AuthoringCancellationToken::new();
    cancellation.cancel();
    assert!(matches!(
        template.render_reflow_bindings_to_buffer_with_options_cancellable(
            &first,
            CompiledReflowOptions::default(),
            &cancellation,
        ),
        Err(FullBleedError::Cancelled)
    ));
    assert!(matches!(
        template.render_reflow_bindings_to_buffer_cancellable_with_report(
            &first,
            CompiledReflowOptions::default(),
            &cancellation,
        ),
        Err(FullBleedError::Cancelled)
    ));
    let mut invalid = first.clone();
    invalid.get_mut("account").unwrap().pop();
    assert!(matches!(
        template.render_reflow_bindings_to_buffer(&invalid),
        Err(FullBleedError::InvalidConfiguration(_))
    ));
    assert!(matches!(
        template.render_reflow_bindings_to_buffer_with_report(&invalid),
        Err(FullBleedError::InvalidConfiguration(_))
    ));
    let html_template = engine.compile_document(
        "<div data-fb-bind-html=\"body\"></div>",
        "body { font: 10pt ProbeFont; }",
    )?;
    let bad_fragment = HashMap::from([(
        "body".to_string(),
        vec![
            "<p>Valid first record</p>".to_string(),
            "<p data-fullbleed-compiler-binding-root=\"999\">Invalid second record</p>".to_string(),
        ],
    )]);
    assert!(matches!(
        html_template.render_reflow_bindings_to_buffer(&bad_fragment),
        Err(FullBleedError::InvalidConfiguration(_))
    ));
    assert!(matches!(
        html_template.render_reflow_bindings_to_buffer_with_report(&bad_fragment),
        Err(FullBleedError::InvalidConfiguration(_))
    ));
    let recovered = html_template.render_reflow_bindings_to_buffer(&HashMap::from([(
        "body".to_string(),
        vec!["<p>Recovered</p>".to_string()],
    )]))?;
    assert_eq!(inspect_pdf_bytes(&recovered)?.page_count, 1);
    let missing_template = engine.compile_document(
        "<p>{{name}}</p>",
        "@page { size:400pt 200pt; margin:12pt; } body { font:12pt ProbeFont; }",
    )?;
    let missing_bindings = HashMap::from([(
        "name".to_string(),
        vec!["ASCII".to_string(), "東京".to_string()],
    )]);
    for reflow in [false, true] {
        let (name, bytes, report, ordinary) = if reflow {
            let (bytes, report) =
                missing_template.render_reflow_bindings_to_buffer_with_report(&missing_bindings)?;
            (
                "reflow",
                bytes,
                report,
                missing_template.render_reflow_bindings_to_buffer(&missing_bindings)?,
            )
        } else {
            assert!(matches!(
                missing_template.render_bindings_to_buffer_cancellable_with_report(
                    &missing_bindings,
                    &cancellation
                ),
                Err(FullBleedError::Cancelled)
            ));
            let (bytes, report) =
                missing_template.render_bindings_to_buffer_with_report(&missing_bindings)?;
            (
                "fixed",
                bytes,
                report,
                missing_template.render_bindings_to_buffer(&missing_bindings)?,
            )
        };
        assert_eq!(bytes, ordinary);
        assert_eq!(report.page_count, 2);
        assert_eq!(report.records.len(), 2);
        assert!(report.records[0].glyph_report.is_empty());
        assert_eq!(
            report.records[1]
                .glyph_report
                .missing()
                .iter()
                .map(|glyph| glyph.ch)
                .collect::<Vec<_>>(),
            vec!['京', '東']
        );
        std::fs::write(format!("diagnostics-{name}.pdf"), &bytes)?;
        std::fs::write(format!("diagnostics-{name}.json"), report_json(&report))?;
    }
    std::fs::write(
        "verification.json",
        format!(
            "{{\"cases\":[{}],\"replayChecks\":4,\"preCancelledRejected\":true,\"unequalColumnsRejected\":true,\"invalidFragmentRejected\":true,\"recoveryPages\":1,\"diagnosticCalls\":10,\"diagnosticFailuresRecovered\":true}}\n",
            summaries.join(","),
        ),
    )?;
    println!(
        "compiled reflow: four PDFs, replay, rebound fonts, cancellation, invalid input, and recovery passed"
    );
    Ok(())
}
