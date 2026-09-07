# Document-context chart typography

Charts are engine-owned Rust compilation, not browser graphics. A frontend
resolves the chart's typed data into `ChartSpec`. The context-free
`compile_chart(&spec)` API remains available with 10-unit plot labels; it cannot
know the containing document's CSS or registered fonts.

For document authoring, use `PreparedChart` and
`FullBleed::compile_chart_document` before final rendering:

```rust
use fullbleed::{ChartKind, ChartSeries, ChartSpec, FullBleed, PreparedChart};

let engine = FullBleed::builder()
    .register_font_file("assets/Inter-Regular.ttf")
    .build()?;
let spec = ChartSpec::new(
    "revenue", ChartKind::Bar, "Quarterly revenue",
    vec!["Q1".into(), "Q2".into()],
    vec![ChartSeries::new("revenue", "Revenue", vec![Some(10.0), Some(20.0)])],
);
let chart = PreparedChart::new("revenue-slot", spec)?;
let html = format!("<figure>{}</figure>", chart.placeholder_html());
let css = "figure { font-family: Inter; font-size: 12pt; }";
let compiled = engine.compile_chart_document(&html, css, &[chart])?;
let pdf = engine.render_to_buffer(&compiled.html, css)?;
```

The pending marker is an in-memory compiler input, not the saved authoring
format. Complete bindings and repeat expansion before lowering. Each occurrence
gets its actual CSS ancestor/sibling context, registered face, label size and
independent generated semantic IDs. Unchanged neighboring HTML remains byte
identical. Altered, ambiguous or reordered marker contexts are refused.

Use the same engine/assets for resolution, PNG preview, PDF and HTML export.
Reusing the engine across records shares font registration and parsed CSS caches.
Plain `data-fb-chart-*` source/data binding remains a frontend responsibility;
this API accepts resolved typed chart specifications, not arbitrary chart HTML.

## CSS and layout

`font-family`, `font-size`, `font-weight` and `font-style` come from Fullbleed's
normal cascade. CSS variables work through those properties, for example
`font-size: var(--chart-label-size)`. A custom property alone changes nothing.
An 18pt label becomes 24 SVG user units at the chart's intrinsic dimensions;
resizing the resulting SVG scales its contents normally.

Legend widths use the same selected-face measurement as native SVG. Labels wrap
at whitespace without truncating words. An unbreakable label that cannot fit is
diagnosed rather than split arbitrarily. Font ascent/descent and line height
reserve legend and category space; axis tick density adapts without shrinking
the selected font. Category labels may be sparse, with their exact count in the
trace. Marks and semantic table values are never sampled or capped by row count.

Without registered font metrics, the native SVG fallback estimate is explicit:
`CHART_FONT_METRICS_FALLBACK`. Vendor the face to obtain project-font measurements.
This is not a claim of exact Base-14 or ambient system-font measurement.

If the requested font/labels leave insufficient plot space, the artifact contains
an explicit diagnostic placeholder and retains its semantic data table when
enabled. `CHART_LAYOUT_INSUFFICIENT_SPACE` tells the author to enlarge the chart
or reduce font size. Frontends must not silently publish that as a finished plot;
Studio keeps the preview editable and blocks PDF/HTML/batch publication on this
compiler error. Preview artifacts remain available for diagnosis.

Each chart entry retains font/metric source, label size, legend rows/lines,
category label count, layout validity, data/primitive counts and diagnostics.
No font discovery, network access, browser measurement or source rewriting is
introduced. These checks do not establish PDF/UA or other profile conformance.
