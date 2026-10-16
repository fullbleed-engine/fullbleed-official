# SPDX-License-Identifier: MIT
from importlib.resources import files
import argparse
import json
from pathlib import Path

from fullbleed.ui.experimental import Renderer
from documents import Invoice, OperationsReport


def main():
    parser = argparse.ArgumentParser(description="Render the experimental component examples")
    parser.add_argument("--output", type=Path, default=Path(__file__).parent / "output")
    args = parser.parse_args()
    data = json.loads((Path(__file__).parent.parent / "components/data.json").read_text(encoding="utf-8"))
    font = files("fullbleed_assets").joinpath("fonts/Inter-Variable.ttf")
    renderer = Renderer(font_files=[str(font)], document_timestamp="2026-10-15T00:00:00Z")
    report = {**data, "items": [{**data["items"][index % 3], "name": f"Deliverable {index:03d}"} for index in range(1, 61)]}
    args.output.mkdir(parents=True, exist_ok=True)
    for name, document in (("invoice", Invoice(data)), ("report", OperationsReport(report))):
        (args.output / f"{name}.pdf").write_bytes(document.render_pdf(renderer))
        document.emit_artifacts(html_path=args.output / f"{name}.html", css_path=args.output / f"{name}.css")
        for index, png in enumerate(document.render_image_pages(renderer, dpi=96), 1):
            (args.output / f"{name}-{index}.png").write_bytes(png)


if __name__ == "__main__":
    main()
