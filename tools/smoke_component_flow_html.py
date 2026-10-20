# SPDX-License-Identifier: MIT
"""Measure component Flow spacing in standalone browser HTML from an installed wheel."""
from __future__ import annotations

import argparse
from hashlib import sha256
from importlib import metadata
import json
from pathlib import Path


def generate(folder):
    import fullbleed
    from fullbleed.ui.experimental import (
        Document, Flow, Fragment, PageBreak, Paragraph, Theme, component,
        pct, pt, style, styled, token,
    )

    root = Path(__file__).resolve().parents[1]
    assert root / "python" not in Path(fullbleed.__file__).parents, "Use an installed wheel with python -I"
    assert not (folder / "fixtures.json").exists(), "Use a new fixture directory"
    folder.mkdir(parents=True, exist_ok=True)
    theme = Theme({"space": pt(20)}, text=style(font_family="sans-serif", font_size=pt(12), line_height=1))
    cases = []

    def paragraph(name, **properties):
        return Paragraph(name, attrs={"id": name}, style=style(**properties))

    def gap(before, after, pixels):
        return {"before": f"#{before}", "after": f"#{after}", "pixels": pixels}

    def add(name, node, checks, *, selectors=None, fragment=False, hidden=()):
        document = Document(node, theme=theme, title=f"Flow HTML: {name}", styles=selectors)
        print_source = document.compile(target="pdf")
        html = document.to_html(include_styles=True)
        plain = document.to_html(include_styles=False)
        if fragment:
            html = ('<!doctype html><html lang="en"><head><meta charset="utf-8">'
                    '<meta name="viewport" content="width=device-width, initial-scale=1">'
                    '<title>Flow fragment</title></head><body>'
                    + node.to_html(include_styles=True, theme=theme) + "</body></html>")
        assert print_source == document.compile(target="pdf"), "HTML export mutated PDF source"
        folder.joinpath(name + ".html").write_text(html, encoding="utf-8", newline="\n")
        folder.joinpath(name + ".unstyled.html").write_text(plain, encoding="utf-8", newline="\n")
        cases.append({
            "name": name, "checks": checks, "hidden": list(hidden),
            "html_sha256": sha256(html.encode()).hexdigest(),
            "unstyled_sha256": sha256(plain.encode()).hexdigest(),
            "print_html_sha256": sha256(print_source.html.encode()).hexdigest(),
            "print_css_sha256": sha256(print_source.css.encode()).hexdigest(),
        })

    children = (paragraph("first"), paragraph("second"))
    add("default", Flow(*children), [gap("first", "second", 8 * 4 / 3)])
    add("explicit", Flow(*children, gap=pt(18)), [gap("first", "second", 24)])
    add("zero", Flow(*children, gap=0), [gap("first", "second", 0)])
    add("margins", Flow(
        paragraph("first", margin_top=pt(6), margin_bottom=pt(9)),
        paragraph("second", margin_top=pt(24), margin_bottom=pt(12)), gap=pt(12),
    ), [gap("first", "second", (9 + 12 + 24) * 4 / 3)])

    @component
    def Pair():
        return Fragment(paragraph("second"), Fragment(paragraph("third")))

    add("fragments", Flow(Fragment(), paragraph("first"), Pair(), Fragment(), gap=pt(9)),
        [gap("first", "second", 12), gap("second", "third", 12)])
    add("nested", Flow(paragraph("first"),
        Flow(paragraph("second"), paragraph("third"), gap=pt(6), attrs={"id": "nested"}),
        paragraph("fourth"), gap=pt(12)),
        [gap("first", "nested", 16), gap("second", "third", 8), gap("nested", "fourth", 16)])
    add("page-break", Flow(children[0], PageBreak(), children[1], gap=pt(18)),
        [gap("first", "second", 24)], hidden=('[aria-hidden="true"]',))
    add("hidden-child", Flow(children[0], Paragraph("Hidden", attrs={"hidden": True}),
        children[1], gap=pt(18)), [gap("first", "second", 24)], hidden=("[hidden]",))

    Recipe = styled(Flow, row_gap=pt(12))
    add("recipe", Recipe(*children), [gap("first", "second", 16)])
    add("prop-over-recipe", Recipe(*children, gap=pt(3)), [gap("first", "second", 4)])
    add("style-over-prop", Recipe(*children, gap=pt(3)).with_style(row_gap=pt(7)),
        [gap("first", "second", 7 * 4 / 3)])
    add("token", styled(Flow, row_gap=token("space"))(*children),
        [gap("first", "second", 20 * 4 / 3)])

    @component(parts=("body",))
    def Panel(*content):
        return Flow(*content, part="body")

    add("named-part", Panel(*children).with_parts(body=style(row_gap=pt(5))),
        [gap("first", "second", 5 * 4 / 3)])
    add("class-selector", Flow(*children, class_name="reader"),
        [gap("first", "second", 18)], selectors={".reader": {"row_gap": "18px"}})
    add("selector-display", Flow(*children, class_name="reader"),
        [gap("first", "second", 18)],
        selectors={".reader": {"display": "grid", "grid_template_columns": "1fr", "row_gap": "18px"}})
    add("fragment-export", Flow(*children, gap=pt(12)),
        [gap("first", "second", 16)], fragment=True)
    add("font-gap", Flow(*children, gap="1.5em"), [gap("first", "second", 24)])
    add("percent-auto", Flow(*children, gap=pct(10)), [gap("first", "second", 0)])
    add("percent-definite", Flow(*children, gap=pct(10), style=style(height=pt(120))),
        [gap("first", "second", 16)])
    result = {
        "version": metadata.version("fullbleed"), "runtime": str(fullbleed.__file__),
        "cases": cases, "reused_children_unchanged": children == (paragraph("first"), paragraph("second")),
    }
    assert result["reused_children_unchanged"]
    (folder / "fixtures.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return result


def check(folder, browser_name, channel):
    from playwright.sync_api import sync_playwright

    fixtures = json.loads((folder / "fixtures.json").read_text(encoding="utf-8"))
    out = folder / (channel or browser_name)
    out.mkdir(exist_ok=False)
    records = []
    with sync_playwright() as playwright:
        browser = getattr(playwright, browser_name).launch(**({"channel": channel} if channel else {}))
        try:
            for width in (1000, 390, 320):
                context = browser.new_context(viewport={"width": width, "height": 700})
                try:
                    page = context.new_page()
                    requests, errors = [], []
                    page.on("request", lambda request: requests.append(request.url))
                    page.on("pageerror", lambda error: errors.append(str(error)))
                    for fixture in fixtures["cases"]:
                        name = fixture["name"]
                        html = folder / f"{name}.html"
                        assert sha256(html.read_bytes()).hexdigest() == fixture["html_sha256"]
                        page.goto(html.as_uri())
                        assert page.locator("script").count() == 0
                        assert page.locator("style").count() == 1
                        measurements = []
                        for expected in fixture["checks"]:
                            first = page.locator(expected["before"]).bounding_box()
                            second = page.locator(expected["after"]).bounding_box()
                            assert first and second, (name, expected)
                            actual = second["y"] - first["y"] - first["height"]
                            assert abs(actual - expected["pixels"]) < 0.1, (name, width, actual, expected)
                            measurements.append({**expected, "actual_pixels": actual})
                        for selector in fixture["hidden"]:
                            assert page.locator(selector).is_hidden(), (name, selector)
                        assert page.evaluate("document.documentElement.scrollWidth <= innerWidth + 1"), name
                        if width in (1000, 320):
                            page.screenshot(path=str(out / f"{name}-{width}.png"), full_page=True)
                        plain = folder / f"{name}.unstyled.html"
                        assert sha256(plain.read_bytes()).hexdigest() == fixture["unstyled_sha256"]
                        page.goto(plain.as_uri())
                        assert page.locator("style, [style], link[rel=stylesheet], script").count() == 0
                        assert page.locator("#first").count() == 1 and page.locator("#second").count() == 1
                        records.append({"name": name, "width": width, "gaps": measurements})
                    assert not errors, errors
                    assert not any(url.startswith(("http:", "https:")) for url in requests), requests
                finally:
                    context.close()
            result = {
                "ok": True, "version": fixtures["version"], "browser": browser_name,
                "channel": channel, "browser_version": browser.version, "cases": records,
                "visual_review_pending": True,
                "scope": "Real browser gap geometry, margins, fragments, nested Flow, PDF-only page breaks, styling precedence, relative units, unstyled export and no network dependencies at three viewport widths. PDF pagination and browser print equivalence are not asserted.",
            }
            (out / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
            return result
        finally:
            browser.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--browser", choices=("chromium", "firefox", "webkit"), default="chromium")
    parser.add_argument("--channel")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--generate-only", action="store_true")
    mode.add_argument("--check-only", action="store_true")
    args = parser.parse_args()
    folder = args.out.resolve()
    if not args.check_only:
        result = generate(folder)
    if not args.generate_only:
        result = check(folder, args.browser, args.channel)
    print(json.dumps({"ok": True, "cases": len(result["cases"]), "version": result["version"]}))


if __name__ == "__main__":
    main()
