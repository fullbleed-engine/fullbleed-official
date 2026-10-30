use super::*;

const CSS: &str = "@page { size:400pt 200pt; margin:12pt; } body { font:12pt Latin; }";

fn engine(fallback: bool) -> FullBleed {
    let mut fonts = AssetBundle::default();
    fonts.add(Asset::new(
        "Latin".into(),
        AssetKind::Font,
        std::fs::read(tests::repo_font_path("NotoSans-Regular.ttf")).unwrap(),
        None,
        true,
    ));
    if fallback {
        fonts.add(Asset::new(
            "CJK".into(),
            AssetKind::Font,
            std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/table-wrapping/FullbleedCJKTest-Regular.ttf"),
            )
            .unwrap(),
            None,
            true,
        ));
    }
    FullBleed::builder().register_bundle(fonts).build().unwrap()
}

fn columns(values: &[&str]) -> HashMap<String, Vec<String>> {
    HashMap::from([(
        "name".into(),
        values.iter().map(|value| (*value).into()).collect(),
    )])
}

fn missing(report: &CompiledBatchReport) -> Vec<Vec<char>> {
    report
        .records
        .iter()
        .map(|row| {
            row.glyph_report
                .missing()
                .iter()
                .map(|glyph| glyph.ch)
                .collect()
        })
        .collect()
}

fn ranges(report: &CompiledBatchReport, pdf: &[u8]) {
    let mut offset = 0;
    for (index, row) in report.records.iter().enumerate() {
        assert_eq!(row.record_index, index);
        assert_eq!(row.page_offset, offset);
        assert!(row.page_count > 0);
        offset += row.page_count;
    }
    assert_eq!(offset, report.page_count);
    assert_eq!(offset, inspect_pdf_bytes(pdf).unwrap().page_count);
}

#[test]
fn compiled_report_fixed_checks_bound_text_without_changing_bytes() {
    let template = engine(false)
        .compile_document("<p>Marker {{name}} end</p>", CSS)
        .unwrap();
    for values in [["ASCII", "東京", "Café"], ["東京", "ASCII", "東京"]] {
        let bindings = columns(&values);
        let expected = template.render_bindings_to_buffer(&bindings).unwrap();
        let (bytes, report) = template
            .render_bindings_to_buffer_with_report(&bindings)
            .unwrap();
        assert!(bytes == expected, "diagnostics must preserve the PDF");
        ranges(&report, &bytes);
        for (value, observed) in values.iter().zip(missing(&report)) {
            assert_eq!(
                observed,
                if *value == "東京" {
                    vec!['京', '東']
                } else {
                    vec![]
                }
            );
        }
    }
}

#[test]
fn compiled_report_fixed_cannot_borrow_coverage_from_an_unselected_fallback() {
    let engine = engine(true);
    let template = engine.compile_document("<p>{{name}}</p>", CSS).unwrap();
    let bindings = columns(&["ABC", "東京"]);
    let (fixed, fixed_report) = template
        .render_bindings_to_buffer_with_report(&bindings)
        .unwrap();
    assert_eq!(missing(&fixed_report), vec![vec![], vec!['京', '東']]);
    assert!(fixed == template.render_bindings_to_buffer(&bindings).unwrap());
    let (reflow, reflow_report) = template
        .render_reflow_bindings_to_buffer_with_report(&bindings)
        .unwrap();
    assert_eq!(missing(&reflow_report), vec![Vec::<char>::new(), vec![]]);
    assert!(
        reflow
            == template
                .render_reflow_bindings_to_buffer(&bindings)
                .unwrap()
    );
}

#[test]
fn compiled_report_reflow_handles_cache_hits_rebinding_and_replay() {
    for threads in [1, 4] {
        parallel::with_thread_count(threads, || {
            let template = engine(false)
                .compile_document("<p>Marker {{name}} end</p>", CSS)
                .unwrap();
            for values in [
                ["ALPHA", "東京", "BRAVO"],
                ["東京", "CHARLIE", "東京"],
                ["ALPHA", "東京", "BRAVO"],
            ] {
                let bindings = columns(&values);
                let ordinary = template
                    .render_reflow_bindings_to_buffer(&bindings)
                    .unwrap();
                let (bytes, report) = template
                    .render_reflow_bindings_to_buffer_with_report(&bindings)
                    .unwrap();
                assert!(bytes == ordinary);
                ranges(&report, &bytes);
                for (value, observed) in values.iter().zip(missing(&report)) {
                    assert_eq!(
                        observed,
                        if *value == "東京" {
                            vec!['京', '東']
                        } else {
                            vec![]
                        }
                    );
                }
            }
        });
    }
}

#[test]
fn compiled_report_includes_static_generated_and_form_text() {
    let template = engine(false)
        .compile_document(
            "<div style='opacity:.5'><p>水</p></div><p class='bound'>{{name}}</p>",
            &format!("{CSS} .bound::before {{ content:'京'; }}"),
        )
        .unwrap();
    let bindings = columns(&["ALPHA", "東京"]);
    let (pdf, report) = template
        .render_reflow_bindings_to_buffer_with_report(&bindings)
        .unwrap();
    assert_eq!(
        missing(&report),
        vec![vec!['京', '水'], vec!['京', '東', '水']]
    );
    assert!(
        pdf == template
            .render_reflow_bindings_to_buffer(&bindings)
            .unwrap()
    );
}

#[test]
fn compiled_report_records_variable_page_counts_in_input_order() {
    let template = engine(false)
        .compile_document("<p>{{name}}</p>", CSS)
        .unwrap();
    let long = "A long document record. ".repeat(600);
    let bindings = columns(&["SHORT", &long, "END"]);
    let (pdf, report) = template
        .render_reflow_bindings_to_buffer_with_report(&bindings)
        .unwrap();
    ranges(&report, &pdf);
    assert_eq!(report.records.len(), 3);
    assert_eq!(report.records[0].page_count, 1);
    assert!(report.records[1].page_count > 1);
    assert_eq!(report.records[2].page_count, 1);
}

#[test]
fn compiled_report_cancellation_and_invalid_columns_leave_template_reusable() {
    let template = engine(false)
        .compile_document("<p>{{name}}</p>", CSS)
        .unwrap();
    let bindings = columns(&["ASCII", "東京"]);
    let cancellation = AuthoringCancellationToken::new();
    cancellation.cancel();
    assert!(matches!(
        template.render_bindings_to_buffer_cancellable_with_report(&bindings, &cancellation),
        Err(FullBleedError::Cancelled)
    ));
    assert!(matches!(
        template.render_reflow_bindings_to_buffer_cancellable_with_report(
            &bindings,
            CompiledReflowOptions::default(),
            &cancellation
        ),
        Err(FullBleedError::Cancelled)
    ));
    assert!(
        template
            .render_bindings_to_buffer_with_report(&HashMap::new())
            .is_err()
    );
    assert!(
        template
            .render_reflow_bindings_to_buffer_with_report(&HashMap::new())
            .is_err()
    );
    let (fixed, _) = template
        .render_bindings_to_buffer_with_report(&bindings)
        .unwrap();
    let (reflow, _) = template
        .render_reflow_bindings_to_buffer_with_report(&bindings)
        .unwrap();
    assert!(fixed == template.render_bindings_to_buffer(&bindings).unwrap());
    assert!(
        reflow
            == template
                .render_reflow_bindings_to_buffer(&bindings)
                .unwrap()
    );
}

#[test]
fn compiled_report_cancel_during_output_recovers_with_both_worker_counts() {
    struct CancellingWriter<'a> {
        written: usize,
        token: &'a AuthoringCancellationToken,
    }
    impl std::io::Write for CancellingWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.written += bytes.len();
            if self.written > 512 {
                self.token.cancel();
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    for threads in [1, 4] {
        parallel::with_thread_count(threads, || {
            let template = engine(false)
                .compile_document("<p>{{name}}</p>", CSS)
                .unwrap();
            let bindings = columns(&["ASCII", "東京", "END"]);
            let (count, columns) = template.ordered_binding_columns(&bindings).unwrap();
            for reflow in [false, true] {
                let token = AuthoringCancellationToken::new();
                let mut writer = CancellingWriter {
                    written: 0,
                    token: &token,
                };
                let mut report = CompiledBatchReport::default();
                let result = if reflow {
                    template.render_reflow_bindings_to_writer_ordered_reporting(
                        template.compiled_reflow_plan().unwrap(),
                        count,
                        &columns,
                        &mut writer,
                        CompiledReflowOptions::default(),
                        Some(&token),
                        Some(&mut report),
                    )
                } else {
                    template.render_bindings_to_writer_ordered_reporting(
                        count,
                        &columns,
                        &mut writer,
                        Some(&token),
                        Some(&mut report),
                    )
                };
                assert!(writer.written > 512 && token.is_cancelled());
                assert!(matches!(result, Err(FullBleedError::Cancelled)));
            }
            let (pdf, report) = template
                .render_reflow_bindings_to_buffer_with_report(&bindings)
                .unwrap();
            assert_eq!(missing(&report), vec![vec![], vec!['京', '東'], vec![]]);
            assert!(
                pdf == template
                    .render_reflow_bindings_to_buffer(&bindings)
                    .unwrap()
            );
        });
    }
}

#[test]
fn compiled_report_keeps_a_reusable_flow_program_for_covered_values() {
    let template = engine(false)
        .compile_document("<p>{{name}}</p>", CSS)
        .unwrap();
    for value in ["ALPHA", "BRAVO", "Ωμέγα", "12345", "67890"] {
        let (_, report) = template
            .render_reflow_bindings_to_buffer_with_report(&columns(&[value]))
            .unwrap();
        assert!(report.records[0].glyph_report.is_empty());
    }
    let plan = template.compiled_reflow_plan().unwrap();
    let entries = plan.flow_programs.lock().unwrap();
    let count: usize = entries
        .values()
        .map(|entry| entry.state.lock().unwrap().programs.len())
        .sum();
    assert_eq!(count, 1, "diagnostics must retain guarded program reuse");
}
