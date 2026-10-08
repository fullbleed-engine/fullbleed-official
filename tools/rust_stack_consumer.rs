//! Regression consumer for tools/smoke_rust_stack.py. Run on the real main
//! thread: the Rust test harness uses a larger, separately configured stack.
use fullbleed::{Asset, AssetBundle, AssetKind, FullBleed};
use std::{collections::HashMap, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    assert_eq!(args.len(), 2, "font and output directory required");
    assert!(std::env::var_os("RUST_MIN_STACK").is_none());
    let mut assets = AssetBundle::default();
    assets.add(Asset::new(
        "Inter".into(),
        AssetKind::Font,
        fs::read(&args[0])?,
        None,
        true,
    ));
    let engine = FullBleed::builder().register_bundle(assets).build()?;
    let css = "@page { size: A4; margin: 20mm; }
        body { font: 12pt Inter; counter-reset: n; }
        table { border-collapse: collapse; width: 100%; }
        td, th { border: 1pt solid #175c52; padding: 5pt; }
        .box { padding: 2pt; border: 1pt solid #ddd; }
        .counter::before { content: 'CNT' counter(n) 'END'; }
        .card { padding: 12pt; border: 2pt solid #175c52; border-radius: 8pt;
            background: linear-gradient(135deg, #eef6f2, #ffffff);
            box-shadow: 2pt 3pt 4pt #cccccc; }
        .card h2 { color: #175c52; margin: 0 0 6pt; }";
    let mut cases = vec![
        (
            "boxless-table",
            "<table><tr><td style='display:contents;counter-increment:n 5'>
                <span class='counter'>Alpha</span></td></tr></table>"
                .to_owned(),
        ),
        (
            "table-40-rows",
            "<table><thead><tr><th>Item</th><th>Amount</th></tr></thead><tbody>".to_owned()
                + &(1..=40)
                    .map(|i| format!("<tr><td>Line {i}</td><td>{i}.00</td></tr>"))
                    .collect::<String>()
                + "</tbody></table>",
        ),
        (
            "flex",
            "<div style='display:flex;gap:10pt'><div class='box'>First</div>
                <div class='box'>Second</div></div>"
                .to_owned(),
        ),
        (
            "grid",
            "<div style='display:grid;grid-template-columns:1fr 1fr;gap:10pt'>
                <div class='box'>First</div><div class='box'>Second</div></div>"
                .to_owned(),
        ),
        (
            "styled-container",
            "<section class='card'><h2>Delivery summary</h2><p>Ready to print.</p></section>"
                .repeat(14),
        ),
    ];
    for (name, depth) in [("nested-blocks-8", 8), ("nested-blocks-16", 16)] {
        cases.push((
            name,
            "<div class='box'>".repeat(depth) + "<p>Nested content</p>" + &"</div>".repeat(depth),
        ));
    }
    for (name, depth) in [("nested-tables-2", 2), ("nested-tables-3", 3)] {
        cases.push((
            name,
            "<table><tr><td>".repeat(depth)
                + "<p>Nested table content</p>"
                + &"</td></tr></table>".repeat(depth),
        ));
    }
    let bindings = HashMap::from([("name".into(), vec!["Ada".into(), "Bea".into()])]);
    for (name, body) in cases {
        // The 16-level block probe intentionally exercises recursion just once;
        // measuring nested boxes in an unoptimized build is expensive.
        let deep_blocks = name == "nested-blocks-16";
        let template = format!("<html><body><p>Record {{{{name}}}}</p>{body}</body></html>");
        let html = template.replace("{{name}}", "Ada");
        fs::write(args[1].join(format!("{name}.html")), &template)?;
        fs::write(args[1].join(format!("{name}.css")), css)?;
        eprintln!("Rendering {name} on the main thread");
        let (direct, glyphs) = engine.render_with_glyph_report(&html, css)?;
        assert!(glyphs.is_empty(), "Missing glyphs in {name}");
        assert!(direct.starts_with(b"%PDF-"));
        fs::write(args[1].join(format!("{name}-direct.pdf")), &direct)?;
        if !deep_blocks {
            assert_eq!(direct, engine.render_to_buffer(&html, css)?);
            let compiled = engine.compile_document(&html, css)?;
            let compiled_pdf = compiled.render_to_buffer()?;
            assert_eq!(direct, compiled_pdf, "Direct/compiled differ for {name}");
            fs::write(args[1].join(format!("{name}-compiled.pdf")), compiled_pdf)?;
            fs::write(
                args[1].join(format!("{name}-repeat.pdf")),
                compiled.render_many_to_buffer(2)?,
            )?;
            fs::write(
                args[1].join(format!("{name}-reflow.pdf")),
                engine
                    .compile_document(&template, css)?
                    .render_reflow_bindings_to_buffer(&bindings)?,
            )?;
        }
        println!("PASS {name}");
    }
    Ok(())
}
