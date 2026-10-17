# SPDX-License-Identifier: MIT
"""Check generated HTML with an isolated, headless Playwright browser."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from playwright.sync_api import sync_playwright


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True, help="Output directory from smoke_component_bootstrap.py")
    parser.add_argument("--browser-executable", type=Path)
    args = parser.parse_args()
    folder = args.out.resolve()
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(headless=True, executable_path=str(args.browser_executable) if args.browser_executable else None)
        context = browser.new_context(viewport={"width": 1000, "height": 900}, device_scale_factor=1)
        page = context.new_page()
        requests, errors = [], []
        page.on("request", lambda request: requests.append(request.url))
        page.on("pageerror", lambda error: errors.append(str(error)))
        page.goto((folder / "catalog.html").as_uri())
        assert page.locator("script").count() == 0
        assert page.locator("style").count() == 1
        assert page.locator("[id^=sample-]").count() == 27
        assert page.locator("#company").input_value() == "Northstar Studio"
        assert page.locator("#archive").is_checked()
        assert page.locator("#sample-forms select option:checked").inner_text() == "Standard stock"
        assert page.locator("#record-item-1-panel").is_visible()
        assert page.locator("#record-item-2-panel").is_hidden()
        assert page.locator("#sample-carousel .carousel-item").nth(1).is_hidden()
        for selector in ("#review", "#details", "#sample-popovers .popover", "#sample-toasts .toast", "#sample-tooltips .tooltip"):
            assert page.locator(selector).is_visible(), selector
        assert page.locator(".fb-bs-spinner").first.evaluate("e => getComputedStyle(e).animationName") == "none"
        family_boxes = {}
        for sample in page.locator("[id^=sample-]").all():
            family_boxes[sample.get_attribute("id")] = sample.bounding_box()
        page.screenshot(path=str(folder / "catalog.browser.png"), full_page=True)
        page.locator("#sample-forms").screenshot(path=str(folder / "forms.browser.png"))
        page.set_viewport_size({"width": 390, "height": 844})
        narrow = page.evaluate("({width:innerWidth,scroll:document.documentElement.scrollWidth})")
        assert narrow["scroll"] <= narrow["width"], narrow
        page.screenshot(path=str(folder / "catalog.mobile.png"), full_page=True)
        page.goto((folder / "catalog.unstyled.html").as_uri())
        assert page.locator("style, [style], link[rel=stylesheet], script").count() == 0
        assert page.locator("#record-item-2-panel").is_hidden()
        assert page.locator("#company").input_value() == "Northstar Studio"
        page.set_viewport_size({"width": 1000, "height": 1200})
        page.goto((folder / "report.styled.html").as_uri())
        assert page.get_by_text("Studio notebooks", exact=True).is_visible()
        page.screenshot(path=str(folder / "report.browser.png"), full_page=True)
        assert not errors, errors
        assert not any(url.startswith(("http:", "https:")) for url in requests), requests
        result = {"ok": True, "browser": browser.version, "family_boxes": family_boxes,
                  "mobile_width": narrow, "page_errors": errors, "requests": requests,
                  "scope": "Static browser HTML, authored visibility and form states, no scripts, no external network requests; 1000px and 390px catalog viewports."}
        (folder / "browser-verification.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
        context.close()
        browser.close()
    print(json.dumps({"ok": True, "specimens": len(family_boxes)}))


if __name__ == "__main__":
    main()
