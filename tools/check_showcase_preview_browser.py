"""Verify automatic refresh, errors and real downloads in a fresh browser."""
from __future__ import annotations

import argparse
from hashlib import sha256
import json
from pathlib import Path
import shutil
import sys

from playwright.sync_api import expect, sync_playwright
from check_showcase_preview import Server, edit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--project", type=Path, default=Path(__file__).resolve().parents[1] / "examples/design_showcase")
    parser.add_argument("--python", type=Path, default=Path(sys.executable), help="Python with the installed component alpha")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--channel", help="Optional Playwright browser channel, e.g. chrome")
    args = parser.parse_args()
    # Make the command independent of cwd without following a venv's interpreter symlink.
    args.python = args.python.absolute()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    project = out / "project"
    shutil.copytree(args.project, project, ignore=shutil.ignore_patterns("output", "__pycache__", "*.pyc"))
    server = Server(project, out / "server-invoice", python=args.python)
    page_errors, screenshots, downloads = [], [], []

    def capture(page, name):
        page.locator("#pages img").evaluate_all("images => Promise.all(images.map(image => image.decode()))")
        assert page.evaluate("document.documentElement.scrollWidth <= innerWidth + 1"), "Horizontal page overflow"
        page.screenshot(path=str(out / name), animations="disabled")
        screenshots.append(name)

    try:
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(headless=True, **({"channel": args.channel} if args.channel else {}))
            context = browser.new_context(accept_downloads=True, viewport={"width": 1440, "height": 1000})
            page = context.new_page()
            page.on("pageerror", lambda error: page_errors.append(str(error)))
            initial = server.wait()
            page.goto(server.url)
            expect(page.get_by_role("status")).to_have_text("Preview up to date")
            expect(page.locator("#pages img")).to_have_count(1)
            for label, width, height in [("desktop", 1440, 1000), ("mobile", 390, 844), ("narrow", 320, 740)]:
                page.set_viewport_size({"width": width, "height": height})
                capture(page, f"invoice-{label}.png")
            page.set_viewport_size({"width": 1440, "height": 1000})
            for label, name in [("Download PDF", "document.pdf"), ("Styled HTML", "document.styled.html"),
                                ("Unstyled HTML", "document.unstyled.html")]:
                with page.expect_download() as pending:
                    page.get_by_role("link", name=label, exact=True).click()
                download = pending.value
                assert download.suggested_filename == name
                destination = out / name
                download.save_as(destination)
                expected = server.fetch(f"revisions/{initial['revision']}/{name}")[0]
                assert destination.read_bytes() == expected
                downloads.append({"file": name, "sha256": sha256(expected).hexdigest()})

            component = project / "invoice_components.py"
            edit(component, 'Strong("Maple & Finch")', 'Strong("Maple & Cedar")')
            changed = server.wait(after=initial["attempt"])
            expect(page.locator("#pages img")).to_have_attribute("src", f"revisions/{changed['revision']}/page-1.png")
            capture(page, "invoice-edited.png")

            good = component.read_text(encoding="utf-8")
            component.write_text(good + "\nthis is a syntax error !!!\n", encoding="utf-8")
            failure = server.wait("error", after=changed["attempt"])
            expect(page.get_by_role("status")).to_have_text("This edit could not be rendered")
            expect(page.get_by_role("alert")).to_contain_text("SyntaxError")
            expect(page.get_by_role("link", name="Download last PDF", exact=True)).to_be_visible()
            expect(page.locator("#pages img")).to_have_attribute("src", f"revisions/{changed['revision']}/page-1.png")
            page.set_viewport_size({"width": 390, "height": 844})
            capture(page, "error-mobile.png")
            component.write_text(good, encoding="utf-8")
            recovered = server.wait(after=failure["attempt"])
            expect(page.get_by_role("status")).to_have_text("Preview up to date")
            expect(page.get_by_role("alert")).to_be_hidden()
            expect(page.locator("#pages img")).to_have_attribute("src", f"revisions/{recovered['revision']}/page-1.png")
            capture(page, "recovered-mobile.png")

            server.close()
            server = Server(project, out / "server-report", document="report", python=args.python)
            report = server.wait()
            page.goto(server.url)
            expect(page.locator("#pages img")).to_have_count(3)
            page.set_viewport_size({"width": 1440, "height": 1000})
            capture(page, "report-three-pages.png")
            edit(project / "report_components.py", "CoverPage(), ReachPage(report), ResourcesPage(report)", "CoverPage()")
            reduced = server.wait(after=report["attempt"])
            expect(page.locator("#pages img")).to_have_count(1)
            expect(page.locator("#meta")).to_contain_text("1 page")
            expect(page.locator("#pages img")).to_have_attribute("src", f"revisions/{reduced['revision']}/page-1.png")
            capture(page, "report-one-page.png")

            server.close()
            expect(page.get_by_role("status")).to_have_text("Preview server disconnected", timeout=15000)
            for identifier in ["pdf", "source", "extra"]:
                expect(page.locator("#" + identifier)).to_have_attribute("aria-disabled", "true")
            capture(page, "disconnected.png")
            context.close()
            browser.close()
        assert not page_errors, page_errors
    finally:
        server.close()
    result = dict(ok=True, screenshots=screenshots, downloads=downloads, page_errors=page_errors,
                  widths=[1440, 390, 320], automatic_refresh=True, error_recovery=True,
                  stale_preview_label=True, page_count_reduction=True, disconnected_downloads_disabled=True)
    (out / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
