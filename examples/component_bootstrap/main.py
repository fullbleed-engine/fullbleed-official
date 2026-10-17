"""Build a rich report, standalone HTML and a static Bootstrap catalog.

Run with a wheel containing the experimental Bootstrap kit:
    python examples/component_bootstrap/main.py --out target/bootstrap-demo
"""
from __future__ import annotations

import argparse
from importlib.resources import files
import json
from pathlib import Path

from fullbleed.ui.experimental import Document, Page, Renderer, Theme, mm, pt, style
from fullbleed.ui.experimental import bootstrap as bs
from catalog import samples


def report():
    rows = [
        {"name": "Studio notebooks", "units": 240, "amount": "$3,600.00"},
        {"name": "Cotton stationery sets", "units": 120, "amount": "$2,160.00"},
        {"name": "Archival print collection", "units": 48, "amount": "$2,880.00"},
    ]
    metrics = [("408", "Items delivered"), ("$8,640", "Collection revenue"), ("98%", "On-time dispatch")]
    return Document(
        bs.Box(bs.Text("NORTHSTAR / STUDIO", class_name="fw-bold"), bs.Badge("APRIL 2026", variant="light", class_name="ms-3"), class_name="mb-4"),
        bs.Heading("Made with care.\nDelivered with purpose.", level=1, style=style(font_size=pt(32), white_space="pre-line", line_height=1.1)),
        bs.Paragraph("A production report for people who care about the details.", class_name="lead text-secondary mb-4"),
        bs.Row((bs.Col(bs.Card(bs.Heading(value, level=2, class_name="mb-1"), bs.Text(label), variant="light"), span=4)
                for value, label in metrics), gutter=3, class_name="mb-4"),
        bs.Heading("The April collection", level=2, class_name="h4"),
        bs.DataTable(rows, columns=[bs.Column("Collection", lambda row: row["name"]),
                                   bs.Column("Units", lambda row: row["units"], align="right"),
                                   bs.Column("Revenue", lambda row: row["amount"], align="right")],
                     striped=True, caption="Completed orders / April 1-30, 2026"),
        bs.Row(
            bs.Col(bs.Card(bs.Paragraph("Every shipment used recycled packaging. The next collection enters production on May 6."),
                           title="A lighter footprint", footer=bs.Badge("100% recycled packaging", variant="success")), span=7),
            bs.Col(bs.Card(bs.Paragraph("We are three quarters of the way to our spring production target."),
                           bs.Progress(bs.ProgressBar(75, label="75%", variant="success")), title="On the horizon"), span=5),
            gutter=3, class_name="mt-4",
        ),
        bs.Alert("All figures are final. Prepared for the Northstar operations team.", variant="light", class_name="mt-4 small"),
        title="Northstar / April production report", page=Page(size="A4", margin=mm(16)),
        theme=Theme(text=style(font_family="Inter, Arial, sans-serif", font_size=pt(10), color="#152c3a", line_height=1.4)),
        styles={".card": {"border_color": "#d7e1de"}, ".card-title": {"font_size": "15pt"}},
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("target/bootstrap-demo"))
    parser.add_argument("--catalog-pdfs", action="store_true")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    font = files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    renderer = Renderer(font_files=[str(font)], document_timestamp="2026-04-30T00:00:00Z")
    document = report()
    document.emit_artifacts(html_path=args.out / "report.html", css_path=args.out / "report.css")
    (args.out / "report.styled.html").write_text(document.to_html(include_styles=True), encoding="utf-8")
    (args.out / "report.unstyled.html").write_text(document.to_html(include_styles=False), encoding="utf-8")
    prepared = document.prepare(renderer)
    (args.out / "report.pdf").write_bytes(prepared.render_pdf())
    for index, png in enumerate(prepared.render_image_pages(dpi=100), 1):
        (args.out / f"report-{index}.png").write_bytes(png)
    trace = prepared.export_render_time_pagination_trace()
    (args.out / "report.pagination.json").write_text(json.dumps(trace, indent=2), encoding="utf-8")
    catalog = samples()
    theme = Theme(text=style(font_family="Inter, Arial, sans-serif", font_size=pt(10)))
    gallery = Document((bs.Box(bs.Heading(name.replace("-", " ").title(), level=2), node,
                              class_name="mb-5", id="sample-" + name) for name, node in catalog.items()),
                       title="Static Bootstrap 5.0.0 catalog", theme=theme)
    (args.out / "catalog.html").write_text(gallery.to_html(include_styles=True), encoding="utf-8")
    (args.out / "catalog.unstyled.html").write_text(gallery.to_html(include_styles=False), encoding="utf-8")
    if args.catalog_pdfs:
        evidence = {}
        for name, node in catalog.items():
            specimen = Document(bs.Heading(name.replace("-", " ").title(), level=1), node,
                                title=name, theme=theme, page=Page(size="A5", margin=mm(12)))
            try:
                compiled = specimen.prepare(renderer)
                (args.out / f"catalog-{name}.pdf").write_bytes(compiled.render_pdf())
                pages = compiled.render_image_pages(dpi=100)
                for i, png in enumerate(pages, 1):
                    (args.out / f"catalog-{name}-{i}.png").write_bytes(png)
                evidence[name] = {"pages": len(pages), "trace": compiled.export_render_time_pagination_trace()}
            except Exception as error:
                evidence[name] = {"error": str(error), "type": type(error).__name__}
        (args.out / "catalog.pagination.json").write_text(json.dumps(evidence, indent=2), encoding="utf-8")
    print(args.out.resolve())


if __name__ == "__main__":
    main()
