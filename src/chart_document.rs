//! Document-context chart lowering. Pending markers are ephemeral compiler
//! inputs, not a persistence format or a second chart source language.

use std::collections::BTreeMap;

use crate::chart::{self, ChartTypography};
use crate::{ChartDiagnostic, ChartError, ChartSpec, ChartTable, ChartTrace, FullBleed};

/// A validated chart whose data is resolved but whose typography is not yet
/// known. Insert `placeholder_html()` into derived HTML, finish bindings and
/// repeat expansion, then call `FullBleed::compile_chart_document`.
#[derive(Debug, Clone)]
pub struct PreparedChart {
    key: String,
    spec: ChartSpec,
    placeholder: String,
}

impl PreparedChart {
    pub fn new(key: impl Into<String>, spec: ChartSpec) -> Result<Self, ChartError> {
        let key = key.into();
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(ChartError::InvalidDocumentContext("Chart placeholder keys must be nonempty ASCII letters, digits, hyphens or underscores".into()));
        }
        chart::validate_spec(&spec)?;
        let id = chart::xml_id(&spec.id);
        let mut placeholder = chart::svg_header(&spec, &id);
        let header_end = placeholder.find('>').expect("generated SVG header");
        placeholder.insert_str(header_end, &format!(" data-fb-chart-pending=\"{key}\""));
        placeholder.push_str("</svg>");
        if spec.table == ChartTable::Visible {
            // Include the real table now so structural CSS selectors see the
            // same siblings/descendants before and after lowering.
            placeholder.push_str(&chart::render_table(&spec, &id));
        }
        Ok(Self {
            key,
            spec,
            placeholder,
        })
    }

    pub fn placeholder_html(&self) -> &str {
        &self.placeholder
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartDocumentEntry {
    pub key: String,
    pub occurrence: usize,
    pub diagnostics: Vec<ChartDiagnostic>,
    pub trace: ChartTrace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartDocumentArtifact {
    pub html: String,
    pub charts: Vec<ChartDocumentEntry>,
}

impl FullBleed {
    /// Compiles prepared charts with this engine's actual CSS cascade, page
    /// context and registered font bytes. All other HTML bytes are preserved.
    /// Repeated occurrences receive their own computed style and derived IDs.
    /// Empty repeat collections may legitimately remove a prepared marker.
    ///
    /// # Errors
    /// Refuses duplicate keys, ambiguous/reordered/non-element markers, or
    /// altered placeholders instead of guessing which source bytes to replace.
    pub fn compile_chart_document(
        &self,
        html: &str,
        css: &str,
        prepared: &[PreparedChart],
    ) -> Result<ChartDocumentArtifact, ChartError> {
        if prepared.is_empty() {
            return Ok(ChartDocumentArtifact {
                html: html.into(),
                charts: Vec::new(),
            });
        }
        let mut keys = BTreeMap::new();
        let mut spans = Vec::new();
        for (index, chart) in prepared.iter().enumerate() {
            if keys.insert(chart.key.as_str(), index).is_some() {
                return Err(ChartError::InvalidDocumentContext(
                    "Duplicate chart placeholder key".into(),
                ));
            }
            spans.extend(
                html.match_indices(&chart.placeholder)
                    .map(|(start, matched)| (start, start + matched.len(), index)),
            );
        }
        spans.sort_unstable_by_key(|span| span.0);
        if spans.windows(2).any(|pair| pair[0].1 > pair[1].0) {
            return Err(ChartError::InvalidDocumentContext(
                "Overlapping chart placeholders".into(),
            ));
        }
        let context = self.build_render_context(css, None);
        let styles = crate::html::chart_marker_text_styles(html, &context.resolver)
            .into_iter()
            .filter(|(key, _)| keys.contains_key(key.as_str()))
            .collect::<Vec<_>>();
        if spans.len() != styles.len()
            || spans
                .iter()
                .zip(&styles)
                .any(|(span, (key, _))| prepared[span.2].key != *key)
        {
            return Err(ChartError::InvalidDocumentContext(
                "Chart placeholders must remain intact HTML elements in source order".into(),
            ));
        }
        let mut output = String::with_capacity(html.len());
        let mut charts = Vec::with_capacity(spans.len());
        let mut occurrences = vec![0; prepared.len()];
        let mut cursor = 0;
        for ((start, end, index), (_, style)) in spans.into_iter().zip(styles) {
            let pending = &prepared[index];
            let occurrence = occurrences[index];
            occurrences[index] += 1;
            let repeated_spec;
            let spec = if occurrence == 0 {
                &pending.spec
            } else {
                repeated_spec = ChartSpec {
                    id: format!("{}-instance-{}", pending.spec.id, occurrence + 1),
                    ..pending.spec.clone()
                };
                &repeated_spec
            };
            let typography = ChartTypography::new(&self.font_registry, &style);
            let artifact = chart::compile_chart_with_typography(spec, &typography)?;
            output.push_str(&html[cursor..start]);
            output.push_str(&artifact.svg);
            output.push_str(&artifact.table_html);
            cursor = end;
            charts.push(ChartDocumentEntry {
                key: pending.key.clone(),
                occurrence,
                diagnostics: artifact.diagnostics,
                trace: artifact.trace,
            });
        }
        output.push_str(&html[cursor..]);
        Ok(ChartDocumentArtifact {
            html: output,
            charts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChartKind, ChartSeries, Command, Pt};

    fn spec(labels: &[&str]) -> ChartSpec {
        let mut spec = ChartSpec::new(
            "typography",
            ChartKind::Bar,
            "Chart typography",
            vec!["First quarter".into(), "Second quarter".into()],
            labels
                .iter()
                .enumerate()
                .map(|(index, label)| {
                    ChartSeries::new(
                        format!("series-{index}"),
                        *label,
                        vec![Some(10.0), Some(20.0)],
                    )
                })
                .collect(),
        );
        spec.width = 400;
        spec.height = 400;
        spec.table = ChartTable::Hidden;
        spec
    }

    #[test]
    fn document_chart_size_cascade_and_warm_output_match_the_final_text_commands() {
        let pending = PreparedChart::new("proof", spec(&["Revenue"])).unwrap();
        let html = format!(
            "<!-- exact trivia -->\n<main><p>REFERENCE</p><figure style='font-size:inherit'>{}</figure></main>",
            pending.placeholder_html()
        );
        let css = "@page { size: 800pt 700pt; margin: 20pt; } main { --label-size:18pt; font-size:var(--label-size); font-family:'Noto Sans'; } figure { margin:0; }";
        let engine = FullBleed::builder()
            .register_font_file(crate::tests::repo_font_path("NotoSans-Regular.ttf"))
            .svg_form_xobjects(false)
            .build()
            .unwrap();
        let first = engine
            .compile_chart_document(&html, css, std::slice::from_ref(&pending))
            .unwrap();
        let second = engine
            .compile_chart_document(&html, css, &[pending])
            .unwrap();
        assert_eq!(first, second);
        assert!(first.html.starts_with("<!-- exact trivia -->\n<main>"));
        assert_eq!(first.charts[0].trace.label_font_size, "24");
        assert!(
            first.charts[0]
                .trace
                .label_font_family
                .to_lowercase()
                .contains("noto")
        );
        assert!(first.charts[0].trace.layout_valid);
        let document = engine.render_to_document(&first.html, css).unwrap();
        let mut size = Pt::ZERO;
        let mut sizes = BTreeMap::new();
        for command in document.pages.iter().flat_map(|page| &page.commands) {
            match command {
                Command::SetFontSize(value) => size = *value,
                Command::DrawString { text, .. } => {
                    sizes.insert(text.clone(), size);
                }
                _ => {}
            }
        }
        assert_eq!(sizes.get("REFERENCE"), Some(&Pt::from_f32(18.0)));
        // SVG keeps font size in its 400px viewBox and applies a 0.75
        // user-unit -> point transform, unlike point-native paragraph text.
        assert_eq!(sizes.get("Revenue"), Some(&Pt::from_f32(24.0)));
        assert_eq!(
            engine.render_to_buffer(&first.html, css).unwrap(),
            engine.render_to_buffer(&second.html, css).unwrap()
        );
    }

    #[test]
    fn legend_layout_uses_font_widths_not_equal_character_counts() {
        let engine = FullBleed::builder()
            .register_font_file(crate::tests::repo_font_path("NotoSans-Regular.ttf"))
            .build()
            .unwrap();
        let compile = |label| {
            let pending = PreparedChart::new("legend", spec(&[label, label])).unwrap();
            engine
                .compile_chart_document(
                    &format!("<figure>{}</figure>", pending.placeholder_html()),
                    "figure {font-family:'Noto Sans'; font-size:12pt;}",
                    &[pending],
                )
                .unwrap()
        };
        let narrow = compile("iiiiiiiiiiiiiiii");
        let wide = compile("WWWWWWWWWWWWWWWW");
        assert_eq!(narrow.charts[0].trace.legend_row_count, 1);
        assert_eq!(wide.charts[0].trace.legend_row_count, 2);
        assert!(narrow.charts[0].trace.layout_valid && wide.charts[0].trace.layout_valid);
    }

    #[test]
    fn long_legend_wraps_without_losing_words_or_overlapping_the_next_series() {
        let label = "Revenue from residential accounts for the current reporting period";
        let pending = PreparedChart::new("long-label", spec(&[label, "Budget"])).unwrap();
        let engine = FullBleed::builder().build().unwrap();
        let compiled = engine
            .compile_chart_document(
                &format!("<figure>{}</figure>", pending.placeholder_html()),
                "figure {font-family:Courier; font-size:12pt;}",
                &[pending],
            )
            .unwrap();
        assert!(compiled.charts[0].trace.layout_valid);
        assert!(compiled.charts[0].trace.legend_line_count > 2);
        let svg = crate::xml::Document::parse(
            &compiled.html
                [compiled.html.find("<svg").unwrap()..compiled.html.find("</svg>").unwrap() + 6],
        )
        .unwrap();
        let labels = svg
            .descendants()
            .filter(|node| node.attribute("data-fb-chart-label") == Some("legend"))
            .collect::<Vec<_>>();
        let text = labels
            .iter()
            .filter_map(|node| node.text())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(text, format!("{label} Budget"));
        let ys = labels
            .iter()
            .map(|node| node.attribute("y").unwrap().parse::<f64>().unwrap())
            .collect::<Vec<_>>();
        assert!(
            ys.windows(2).all(|pair| pair[1] > pair[0]),
            "wrapped legend rows must progress: {ys:?}"
        );
    }

    #[test]
    fn repeated_chart_markers_use_each_occurrences_css_and_unique_semantic_ids() {
        let mut chart = spec(&["Revenue"]);
        chart.table = ChartTable::Visible;
        let pending = PreparedChart::new("repeated", chart).unwrap();
        let html = format!(
            "<main><figure>{0}</figure><figure>{0}</figure></main>",
            pending.placeholder_html()
        );
        let engine = FullBleed::builder().build().unwrap();
        let compiled = engine
            .compile_chart_document(
                &html,
                "figure {font-family:Helvetica; font-size:9pt;} figure + figure {font-size:15pt;}",
                std::slice::from_ref(&pending),
            )
            .unwrap();
        assert_eq!(compiled.charts.len(), 2);
        assert_eq!(compiled.charts[0].trace.label_font_size, "12");
        assert_eq!(compiled.charts[1].trace.label_font_size, "20");
        assert!(compiled.html.contains("id=\"typography-instance-2-table\""));
        assert!(!compiled.html.contains("data-fb-chart-pending"));
        let empty = engine
            .compile_chart_document("<main></main>", "", &[pending])
            .unwrap();
        assert!(empty.charts.is_empty());
    }

    #[test]
    fn tiny_chart_reports_an_actionable_error_without_discarding_semantic_values() {
        let mut chart = spec(&["Revenue"]);
        chart.width = 160;
        chart.height = 100;
        chart.table = ChartTable::Visible;
        let pending = PreparedChart::new("tiny", chart).unwrap();
        let engine = FullBleed::builder().build().unwrap();
        let compiled = engine
            .compile_chart_document(
                &format!("<figure>{}</figure>", pending.placeholder_html()),
                "figure {font-size:48pt;}",
                &[pending],
            )
            .unwrap();
        assert!(!compiled.charts[0].trace.layout_valid);
        assert!(
            compiled.charts[0]
                .diagnostics
                .iter()
                .any(|item| item.code == "CHART_LAYOUT_INSUFFICIENT_SPACE")
        );
        let document = crate::xml::Document::parse(&compiled.html).unwrap();
        let message = document
            .descendants()
            .filter(|node| node.attribute("data-fb-chart-label") == Some("state"))
            .filter_map(|node| node.text())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(message, "Chart labels need more space");
        assert!(compiled.html.contains("<td>10</td>") && compiled.html.contains("<td>20</td>"));
    }

    #[test]
    fn chart_context_rejects_ambiguous_and_altered_markers() {
        let pending = PreparedChart::new("guard", spec(&["Revenue"])).unwrap();
        let engine = FullBleed::builder().build().unwrap();
        let commented = format!("<!-- {} -->", pending.placeholder_html());
        assert!(
            engine
                .compile_chart_document(&commented, "", std::slice::from_ref(&pending))
                .is_err()
        );
        let altered = pending
            .placeholder_html()
            .replace("width=\"400\"", "width=\"401\"");
        assert!(
            engine
                .compile_chart_document(&altered, "", std::slice::from_ref(&pending))
                .is_err()
        );
        assert!(
            engine
                .compile_chart_document(
                    pending.placeholder_html(),
                    "",
                    &[pending.clone(), pending.clone()]
                )
                .is_err()
        );
    }
}
