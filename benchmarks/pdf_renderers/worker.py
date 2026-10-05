"""One renderer per process. Validation and RSS sampling live in the controller."""
from __future__ import annotations

import argparse
import json
import time
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", choices=["fullbleed", "weasyprint", "chromium"], required=True)
    parser.add_argument("--inputs", type=Path, required=True)
    parser.add_argument("--font", type=Path, required=True)
    parser.add_argument("--fixture", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--count", type=int, required=True)
    parser.add_argument("--warmups", type=int, default=0)
    parser.add_argument("--start-token", type=int, required=True)
    args = parser.parse_args()
    template = (args.inputs / f"{args.fixture}.html").read_text(encoding="utf-8")
    css = (args.inputs / "shared.css").read_text(encoding="utf-8")
    close = lambda: None
    metadata = {}
    if args.engine == "fullbleed":
        import fullbleed
        engine = fullbleed.PdfEngine(font_files=[str(args.font)])
        render = lambda html: bytes(engine.render_pdf(html, css))
    elif args.engine == "weasyprint":
        from weasyprint import CSS, HTML
        from weasyprint.text.fonts import FontConfiguration
        fonts = FontConfiguration()
        style = CSS(string=css, font_config=fonts)
        render = lambda html: HTML(string=html).write_pdf(stylesheets=[style], font_config=fonts)
    else:
        from playwright.sync_api import sync_playwright
        playwright = sync_playwright().start()
        browser = playwright.chromium.launch()
        page = browser.new_page()
        page.emulate_media(media="print")
        metadata["browser_version"] = browser.version

        def render(html):
            page.set_content(html.replace("</head>", f"<style>{css}</style></head>"), wait_until="load")
            page.evaluate("document.fonts.ready")
            return page.pdf(prefer_css_page_size=True, print_background=True, tagged=False,
                            display_header_footer=False)

        def close():
            browser.close()
            playwright.stop()

    print(json.dumps({"metadata": metadata}), flush=True)
    try:
        for i in range(args.warmups + args.count):
            token = f"RUN-{args.start_token + i:06d}"
            html = template.replace("RUN-000000", token)
            output = args.out / f"{token}.pdf"
            started = time.perf_counter_ns()
            pdf = render(html)
            output.write_bytes(pdf)
            elapsed = (time.perf_counter_ns() - started) / 1_000_000
            print(json.dumps({"token": token, "path": str(output), "render_write_ms": elapsed,
                              "bytes": len(pdf), "warmup": i < args.warmups}), flush=True)
    finally:
        close()


if __name__ == "__main__":
    main()
