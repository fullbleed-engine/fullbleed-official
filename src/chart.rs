//! Deterministic, browser-independent chart compilation for authored documents.
//!
//! Charts compile to Fullbleed's native inline-SVG subset plus an optional
//! semantic HTML table. The compiler intentionally emits only basic paths,
//! rectangles, lines, circles, and text; it never relies on scripting,
//! animation, filters, masks, markers, or browser layout.

use std::fmt::{Display, Write as _};

pub const CHART_COMPILER_SCHEMA: &str = "fullbleed.chart_compiler.v1";

const DEFAULT_PALETTE: [&str; 8] = [
    "#1f77b4", "#7f7f7f", "#2ca02c", "#d62728", "#9467bd", "#8c564b", "#e377c2", "#17becf",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartKind {
    Bar,
    Line,
    Sparkline,
}

impl ChartKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bar => "bar",
            Self::Line => "line",
            Self::Sparkline => "sparkline",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartTable {
    Visible,
    Hidden,
}

impl ChartTable {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Visible => "visible",
            Self::Hidden => "hidden",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartSeries {
    pub key: String,
    pub label: String,
    pub color: Option<String>,
    pub values: Vec<Option<f64>>,
}

impl ChartSeries {
    pub fn new(key: impl Into<String>, label: impl Into<String>, values: Vec<Option<f64>>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            color: None,
            values,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartSpec {
    pub id: String,
    pub kind: ChartKind,
    pub title: String,
    pub description: String,
    pub width: u32,
    pub height: u32,
    pub categories: Vec<String>,
    pub series: Vec<ChartSeries>,
    pub table: ChartTable,
}

impl ChartSpec {
    pub fn new(
        id: impl Into<String>,
        kind: ChartKind,
        title: impl Into<String>,
        categories: Vec<String>,
        series: Vec<ChartSeries>,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            title: title.into(),
            description: String::new(),
            width: 640,
            height: 320,
            categories,
            series,
            table: ChartTable::Visible,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartDiagnostic {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartTrace {
    pub schema: String,
    pub kind: String,
    pub width: u32,
    pub height: u32,
    pub category_count: usize,
    pub series_count: usize,
    pub data_point_count: usize,
    pub missing_value_count: usize,
    pub primitive_count: usize,
    pub native_vector: bool,
    pub table: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartArtifact {
    pub svg: String,
    pub table_html: String,
    pub diagnostics: Vec<ChartDiagnostic>,
    pub trace: ChartTrace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChartError {
    InvalidDimensions {
        width: u32,
        height: u32,
    },
    NoSeries,
    TooManySeries(usize),
    SeriesLength {
        key: String,
        expected: usize,
        actual: usize,
    },
    NonFiniteValue {
        key: String,
        index: usize,
    },
}

impl Display for ChartError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDimensions { width, height } => write!(
                formatter,
                "chart dimensions must be at least 160 by 100 logical pixels; received {width} by {height}"
            ),
            Self::NoSeries => formatter.write_str("a chart requires at least one series"),
            Self::TooManySeries(count) => {
                write!(
                    formatter,
                    "a chart supports at most 8 series; received {count}"
                )
            }
            Self::SeriesLength {
                key,
                expected,
                actual,
            } => write!(
                formatter,
                "chart series {key:?} has {actual} values but {expected} categories"
            ),
            Self::NonFiniteValue { key, index } => {
                write!(
                    formatter,
                    "chart series {key:?} value {index} is not finite"
                )
            }
        }
    }
}

impl std::error::Error for ChartError {}

/// Compiles a chart into deterministic native inline SVG and semantic HTML.
///
/// # Errors
///
/// Returns [`ChartError`] when dimensions, series shape, or numeric values are
/// invalid. Empty category collections are valid and compile to an explicit
/// observable empty state. Category rows are never implicitly sampled or
/// rejected by a fixed record-count ceiling.
pub fn compile_chart(spec: &ChartSpec) -> Result<ChartArtifact, ChartError> {
    validate_spec(spec)?;
    let missing_value_count = spec
        .series
        .iter()
        .flat_map(|series| &series.values)
        .filter(|value| value.is_none())
        .count();
    let data_point_count = spec.series.iter().map(|series| series.values.len()).sum();
    let mut diagnostics = Vec::new();
    if spec.categories.is_empty() {
        diagnostics.push(ChartDiagnostic {
            code: "CHART_EMPTY_DATA".into(),
            message: "The chart data collection is empty; an explicit empty state was compiled."
                .into(),
        });
    }
    if missing_value_count > 0 {
        diagnostics.push(ChartDiagnostic {
            code: "CHART_MISSING_VALUES".into(),
            message: format!(
                "{missing_value_count} chart value{} missing and omitted from visual marks.",
                if missing_value_count == 1 {
                    " is"
                } else {
                    "s are"
                }
            ),
        });
    }
    if spec.categories.len() > 10_000 {
        diagnostics.push(ChartDiagnostic {
            code: "CHART_DENSE_DATA".into(),
            message: format!(
                "{} chart categories were compiled without sampling or aggregation; the resulting vector and semantic table may be large.",
                spec.categories.len()
            ),
        });
    }

    let safe_id = xml_id(&spec.id);
    let (svg, primitive_count) = render_svg(spec, &safe_id);
    let table_html = if spec.table == ChartTable::Visible {
        render_table(spec, &safe_id)
    } else {
        String::new()
    };
    Ok(ChartArtifact {
        svg,
        table_html,
        diagnostics,
        trace: ChartTrace {
            schema: CHART_COMPILER_SCHEMA.into(),
            kind: spec.kind.as_str().into(),
            width: spec.width,
            height: spec.height,
            category_count: spec.categories.len(),
            series_count: spec.series.len(),
            data_point_count,
            missing_value_count,
            primitive_count,
            native_vector: true,
            table: spec.table.as_str().into(),
        },
    })
}

fn validate_spec(spec: &ChartSpec) -> Result<(), ChartError> {
    if spec.width < 160 || spec.height < 100 {
        return Err(ChartError::InvalidDimensions {
            width: spec.width,
            height: spec.height,
        });
    }
    if spec.series.is_empty() {
        return Err(ChartError::NoSeries);
    }
    if spec.series.len() > DEFAULT_PALETTE.len() {
        return Err(ChartError::TooManySeries(spec.series.len()));
    }
    for series in &spec.series {
        if series.values.len() != spec.categories.len() {
            return Err(ChartError::SeriesLength {
                key: series.key.clone(),
                expected: spec.categories.len(),
                actual: series.values.len(),
            });
        }
        if let Some((index, _)) = series
            .values
            .iter()
            .enumerate()
            .find(|(_, value)| value.is_some_and(|value| !value.is_finite()))
        {
            return Err(ChartError::NonFiniteValue {
                key: series.key.clone(),
                index,
            });
        }
    }
    Ok(())
}

fn render_svg(spec: &ChartSpec, safe_id: &str) -> (String, usize) {
    let width = f64::from(spec.width);
    let height = f64::from(spec.height);
    let title_id = format!("{safe_id}-title");
    let description_id = format!("{safe_id}-description");
    let table_id = format!("{safe_id}-table");
    let mut svg = String::new();
    let described_by = if spec.table == ChartTable::Visible {
        format!("{description_id} {table_id}")
    } else {
        description_id.clone()
    };
    write!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" width=\"{}\" height=\"{}\" role=\"img\" aria-labelledby=\"{}\" aria-describedby=\"{}\" data-fb-chart-kind=\"{}\">",
        spec.width,
        spec.height,
        spec.width,
        spec.height,
        title_id,
        described_by,
        spec.kind.as_str()
    )
    .expect("writing to a string is infallible");
    write!(
        svg,
        "<title id=\"{}\">{}</title><desc id=\"{}\">{}</desc>",
        title_id,
        xml_text(&spec.title),
        description_id,
        xml_text(if spec.description.is_empty() {
            "Data chart"
        } else {
            &spec.description
        })
    )
    .expect("writing to a string is infallible");

    if spec.categories.is_empty() {
        write!(
            svg,
            "<rect x=\"1\" y=\"1\" width=\"{}\" height=\"{}\" fill=\"#f7f9fb\" stroke=\"#a8b3bf\"/><text x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-family=\"Helvetica, sans-serif\" font-size=\"14\" fill=\"#52606d\">No chart data for this record</text></svg>",
            spec.width.saturating_sub(2),
            spec.height.saturating_sub(2),
            fmt_num(width / 2.0),
            fmt_num(height / 2.0)
        )
        .expect("writing to a string is infallible");
        return (svg, 2);
    }

    let compact = spec.kind == ChartKind::Sparkline;
    let left = if compact { 8.0 } else { 52.0 };
    let right = if compact { 8.0 } else { 18.0 };
    let legend = if compact {
        Vec::new()
    } else {
        legend_layout(spec, left, width - right)
    };
    let top = if compact {
        8.0
    } else {
        legend.last().map_or(18.0, |(_, y, _)| y + 22.0).max(18.0)
    };
    let bottom = if compact { 8.0 } else { 42.0 };
    let plot_width = (width - left - right).max(1.0);
    let plot_height = (height - top - bottom).max(1.0);
    let (domain_min, domain_max) = value_domain(spec);
    let baseline = scale_y(0.0, domain_min, domain_max, top, plot_height);
    let mut primitive_count = 0;

    for (series_index, (x, y, _)) in legend.iter().enumerate() {
        let series = &spec.series[series_index];
        write!(
            svg,
            "<rect x=\"{}\" y=\"{}\" width=\"10\" height=\"10\" rx=\"1.5\" fill=\"{}\"/><text x=\"{}\" y=\"{}\" font-family=\"Helvetica, sans-serif\" font-size=\"10\" fill=\"#364452\">{}</text>",
            fmt_num(*x),
            fmt_num(*y),
            xml_attribute(series_color(series, series_index)),
            fmt_num(*x + 15.0),
            fmt_num(*y + 9.0),
            xml_text(&series.label)
        )
        .expect("writing to a string is infallible");
        primitive_count += 2;
    }

    if !compact {
        for tick in 0..=4 {
            let fraction = f64::from(tick) / 4.0;
            let y = top + plot_height * fraction;
            let value = domain_max - (domain_max - domain_min) * fraction;
            write!(
                svg,
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#d9e0e7\" stroke-width=\"1\"/><text x=\"{}\" y=\"{}\" text-anchor=\"end\" font-family=\"Helvetica, sans-serif\" font-size=\"10\" fill=\"#52606d\">{}</text>",
                fmt_num(left),
                fmt_num(y),
                fmt_num(left + plot_width),
                fmt_num(y),
                fmt_num(left - 7.0),
                fmt_num(y + 3.5),
                xml_text(&format_value(value))
            )
            .expect("writing to a string is infallible");
            primitive_count += 2;
        }
        write!(
            svg,
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#647382\" stroke-width=\"1\"/>",
            fmt_num(left),
            fmt_num(baseline),
            fmt_num(left + plot_width),
            fmt_num(baseline)
        )
        .expect("writing to a string is infallible");
        primitive_count += 1;
    }

    match spec.kind {
        ChartKind::Bar => {
            let slot = plot_width / spec.categories.len() as f64;
            let group_width = slot * 0.76;
            let bar_width = (group_width / spec.series.len() as f64).max(0.5);
            for (category_index, category) in spec.categories.iter().enumerate() {
                let group_x = left + slot * category_index as f64 + (slot - group_width) / 2.0;
                for (series_index, series) in spec.series.iter().enumerate() {
                    let Some(value) = series.values[category_index] else {
                        continue;
                    };
                    let value_y = scale_y(value, domain_min, domain_max, top, plot_height);
                    let y = baseline.min(value_y);
                    let bar_height = (baseline - value_y).abs().max(0.4);
                    write!(
                        svg,
                        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\" data-series=\"{}\" data-category=\"{}\"/>",
                        fmt_num(group_x + series_index as f64 * bar_width),
                        fmt_num(y),
                        fmt_num((bar_width - 1.0).max(0.4)),
                        fmt_num(bar_height),
                        xml_attribute(series_color(series, series_index)),
                        xml_attribute(&series.key),
                        xml_attribute(category)
                    )
                    .expect("writing to a string is infallible");
                    primitive_count += 1;
                }
                write_category_label(
                    &mut svg,
                    category,
                    left + slot * (category_index as f64 + 0.5),
                    top + plot_height + 16.0,
                    category_index,
                    spec.categories.len(),
                );
                primitive_count += usize::from(category_label_visible(
                    category_index,
                    spec.categories.len(),
                ));
            }
        }
        ChartKind::Line | ChartKind::Sparkline => {
            let denominator = spec.categories.len().saturating_sub(1).max(1) as f64;
            for (series_index, series) in spec.series.iter().enumerate() {
                let mut path = String::new();
                let mut segment_open = false;
                for (category_index, value) in series.values.iter().enumerate() {
                    let Some(value) = value else {
                        segment_open = false;
                        continue;
                    };
                    let x = left + plot_width * category_index as f64 / denominator;
                    let y = scale_y(*value, domain_min, domain_max, top, plot_height);
                    write!(
                        path,
                        "{}{} {}",
                        if segment_open { " L" } else { "M" },
                        fmt_num(x),
                        fmt_num(y)
                    )
                    .expect("writing to a string is infallible");
                    segment_open = true;
                }
                if !path.is_empty() {
                    write!(
                        svg,
                        "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\" data-series=\"{}\"/>",
                        path,
                        xml_attribute(series_color(series, series_index)),
                        if compact { "2" } else { "2.25" },
                        xml_attribute(&series.key)
                    )
                    .expect("writing to a string is infallible");
                    primitive_count += 1;
                }
                if !compact {
                    for (category_index, value) in series.values.iter().enumerate() {
                        let Some(value) = value else {
                            continue;
                        };
                        let x = left + plot_width * category_index as f64 / denominator;
                        let y = scale_y(*value, domain_min, domain_max, top, plot_height);
                        write!(
                            svg,
                            "<circle cx=\"{}\" cy=\"{}\" r=\"2.75\" fill=\"{}\" data-series=\"{}\"/>",
                            fmt_num(x),
                            fmt_num(y),
                            xml_attribute(series_color(series, series_index)),
                            xml_attribute(&series.key)
                        )
                        .expect("writing to a string is infallible");
                        primitive_count += 1;
                    }
                }
            }
            if !compact {
                for (index, category) in spec.categories.iter().enumerate() {
                    let x = left + plot_width * index as f64 / denominator;
                    write_category_label(
                        &mut svg,
                        category,
                        x,
                        top + plot_height + 16.0,
                        index,
                        spec.categories.len(),
                    );
                    primitive_count +=
                        usize::from(category_label_visible(index, spec.categories.len()));
                }
            }
        }
    }
    svg.push_str("</svg>");
    (svg, primitive_count)
}

fn render_table(spec: &ChartSpec, safe_id: &str) -> String {
    let mut table = String::new();
    write!(
        table,
        "<table id=\"{}-table\" class=\"fb-chart-data\" data-fb-chart-evidence=\"table\"><caption>Data for {}</caption><thead><tr><th scope=\"col\">Category</th>",
        safe_id,
        html_text(&spec.title)
    )
    .expect("writing to a string is infallible");
    for series in &spec.series {
        write!(table, "<th scope=\"col\">{}</th>", html_text(&series.label))
            .expect("writing to a string is infallible");
    }
    table.push_str("</tr></thead><tbody>");
    for (index, category) in spec.categories.iter().enumerate() {
        write!(table, "<tr><th scope=\"row\">{}</th>", html_text(category))
            .expect("writing to a string is infallible");
        for series in &spec.series {
            let value = series.values[index]
                .map(format_value)
                .unwrap_or_else(|| "—".into());
            write!(table, "<td>{}</td>", html_text(&value))
                .expect("writing to a string is infallible");
        }
        table.push_str("</tr>");
    }
    table.push_str("</tbody></table>");
    table
}

fn value_domain(spec: &ChartSpec) -> (f64, f64) {
    let mut minimum = 0.0_f64;
    let mut maximum = 0.0_f64;
    for value in spec
        .series
        .iter()
        .flat_map(|series| &series.values)
        .flatten()
    {
        minimum = minimum.min(*value);
        maximum = maximum.max(*value);
    }
    if (maximum - minimum).abs() < f64::EPSILON {
        if maximum == 0.0 {
            maximum = 1.0;
        } else if maximum > 0.0 {
            minimum = 0.0;
            maximum *= 1.1;
        } else {
            maximum = 0.0;
            minimum *= 1.1;
        }
    }
    (minimum, maximum)
}

fn scale_y(value: f64, minimum: f64, maximum: f64, top: f64, height: f64) -> f64 {
    top + (maximum - value) / (maximum - minimum) * height
}

fn series_color(series: &ChartSeries, index: usize) -> &str {
    series
        .color
        .as_deref()
        .unwrap_or(DEFAULT_PALETTE[index % DEFAULT_PALETTE.len()])
}

fn legend_layout(spec: &ChartSpec, left: f64, right: f64) -> Vec<(f64, f64, f64)> {
    let mut positions = Vec::with_capacity(spec.series.len());
    let mut x = left;
    let mut y = 5.0;
    for series in &spec.series {
        let item_width = (series.label.chars().count() as f64 * 5.8 + 32.0).max(70.0);
        if x > left && x + item_width > right {
            x = left;
            y += 18.0;
        }
        positions.push((x, y, item_width));
        x += item_width;
    }
    positions
}

fn category_label_visible(index: usize, count: usize) -> bool {
    count <= 12 || index % count.div_ceil(12) == 0 || index + 1 == count
}

fn write_category_label(
    svg: &mut String,
    category: &str,
    x: f64,
    y: f64,
    index: usize,
    count: usize,
) {
    if category_label_visible(index, count) {
        write!(
            svg,
            "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-family=\"Helvetica, sans-serif\" font-size=\"10\" fill=\"#364452\">{}</text>",
            fmt_num(x),
            fmt_num(y),
            xml_text(category)
        )
        .expect("writing to a string is infallible");
    }
}

fn fmt_num(value: f64) -> String {
    let mut rendered = format!("{value:.3}");
    while rendered.contains('.') && rendered.ends_with('0') {
        rendered.pop();
    }
    if rendered.ends_with('.') {
        rendered.pop();
    }
    if rendered == "-0" {
        rendered = "0".into();
    }
    rendered
}

fn format_value(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude >= 1_000_000_000.0 {
        return format!("{}B", fmt_num(value / 1_000_000_000.0));
    }
    if magnitude >= 1_000_000.0 {
        return format!("{}M", fmt_num(value / 1_000_000.0));
    }
    if magnitude >= 1_000.0 {
        return format!("{}k", fmt_num(value / 1_000.0));
    }
    fmt_num(value)
}

fn xml_id(value: &str) -> String {
    let mut output = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if output.is_empty() || output.as_bytes()[0].is_ascii_digit() {
        output.insert_str(0, "fb-chart-");
    }
    output
}

fn xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn xml_attribute(value: &str) -> String {
    xml_text(value)
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn html_text(value: &str) -> String {
    xml_text(value).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(kind: ChartKind) -> ChartSpec {
        ChartSpec {
            id: "revenue/chart".into(),
            kind,
            title: "Revenue & target".into(),
            description: "Quarterly comparison".into(),
            width: 640,
            height: 320,
            categories: vec!["Q1".into(), "Q2".into(), "Q3".into(), "Q4".into()],
            series: vec![
                ChartSeries {
                    key: "revenue".into(),
                    label: "Revenue".into(),
                    color: Some("#1668b2".into()),
                    values: vec![Some(90.0), Some(112.0), Some(130.0), Some(148.0)],
                },
                ChartSeries::new(
                    "target",
                    "Target",
                    vec![Some(84.0), Some(99.0), Some(120.0), Some(138.0)],
                ),
            ],
            table: ChartTable::Visible,
        }
    }

    #[test]
    fn bar_chart_is_deterministic_native_svg_with_semantic_table() {
        let first = compile_chart(&fixture(ChartKind::Bar)).unwrap();
        let second = compile_chart(&fixture(ChartKind::Bar)).unwrap();
        assert_eq!(first, second);
        assert!(first.svg.contains("data-fb-chart-kind=\"bar\""));
        assert!(first.svg.contains("<rect"));
        assert!(!first.svg.contains("<script"));
        assert!(!first.svg.contains("<foreignObject"));
        assert!(first.table_html.contains("<th scope=\"col\">Revenue</th>"));
        assert!(first.table_html.contains("<th scope=\"row\">Q1</th>"));
        assert!(first.trace.native_vector);
        assert_eq!(first.trace.data_point_count, 8);
    }

    #[test]
    fn line_and_sparkline_use_basic_paths_without_svg_markers() {
        let line = compile_chart(&fixture(ChartKind::Line)).unwrap();
        let sparkline = compile_chart(&fixture(ChartKind::Sparkline)).unwrap();
        assert!(line.svg.contains("<path"));
        assert!(line.svg.contains("<circle"));
        assert!(sparkline.svg.contains("<path"));
        assert!(!sparkline.svg.contains("<circle"));
        assert!(!line.svg.contains("marker"));
        assert!(!sparkline.svg.contains("marker"));
    }

    #[test]
    fn empty_data_compiles_an_observable_non_failing_state() {
        let mut spec = fixture(ChartKind::Bar);
        spec.categories.clear();
        for series in &mut spec.series {
            series.values.clear();
        }
        let artifact = compile_chart(&spec).unwrap();
        assert!(artifact.svg.contains("No chart data for this record"));
        assert_eq!(artifact.diagnostics[0].code, "CHART_EMPTY_DATA");
        assert_eq!(artifact.trace.category_count, 0);
    }

    #[test]
    fn invalid_series_length_is_refused_before_rendering() {
        let mut spec = fixture(ChartKind::Bar);
        spec.series[0].values.pop();
        assert!(matches!(
            compile_chart(&spec),
            Err(ChartError::SeriesLength {
                expected: 4,
                actual: 3,
                ..
            })
        ));
    }

    #[test]
    fn authored_text_and_attributes_are_escaped() {
        let mut spec = fixture(ChartKind::Bar);
        spec.title = "A < B & C".into();
        spec.categories[0] = "<Q&1>".into();
        spec.series[0].key = "value\" onclick=\"bad".into();
        let artifact = compile_chart(&spec).unwrap();
        assert!(artifact.svg.contains("A &lt; B &amp; C"));
        assert!(artifact.svg.contains("&lt;Q&amp;1&gt;"));
        assert!(!artifact.svg.contains("onclick=\"bad"));
    }

    #[test]
    fn large_category_sets_compile_without_sampling_or_a_fixed_row_limit() {
        let category_count = 100_001;
        let spec = ChartSpec {
            id: "large-source".into(),
            kind: ChartKind::Sparkline,
            title: "Large source".into(),
            description: "Every authored row is retained".into(),
            width: 640,
            height: 160,
            categories: (0..category_count)
                .map(|index| format!("record-{index}"))
                .collect(),
            series: vec![ChartSeries::new(
                "value",
                "Value",
                (0..category_count)
                    .map(|index| Some(index as f64))
                    .collect(),
            )],
            table: ChartTable::Hidden,
        };

        let artifact = compile_chart(&spec).expect("large charts do not have a fixed row limit");
        assert_eq!(artifact.trace.category_count, category_count);
        assert_eq!(artifact.trace.data_point_count, category_count);
        assert!(artifact.svg.contains("data-fb-chart-kind=\"sparkline\""));
        assert!(
            artifact
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "CHART_DENSE_DATA")
        );
    }
}
