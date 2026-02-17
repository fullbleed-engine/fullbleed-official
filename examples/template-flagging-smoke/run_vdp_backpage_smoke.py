from __future__ import annotations

import json
from pathlib import Path

import fitz
import fullbleed


ROOT = Path(__file__).resolve().parent
OUT = ROOT / "output"
OUT.mkdir(parents=True, exist_ok=True)

FRONT_TEMPLATE = OUT / "vdp_front_template.pdf"
BACK_BLUE_TEMPLATE = OUT / "vdp_back_blue_template.pdf"
BACK_GREEN_TEMPLATE = OUT / "vdp_back_green_template.pdf"
BACK_BLANK_TEMPLATE = OUT / "vdp_back_blank_template.pdf"
OVERLAY_PDF = OUT / "vdp_overlay_10x2.pdf"
COMPOSED_PDF = OUT / "vdp_composed_10x2.pdf"
REPORT_JSON = OUT / "vdp_backpage_smoke_report.json"

PAGE_HEIGHT = 792
PAGE_WIDTH_BY_TEMPLATE = {
    "tpl-front": 612,
    "tpl-back-blue": 620,
    "tpl-back-green": 628,
    "tpl-back-blank": 636,
}


def _make_single_template(path: Path, rgb: tuple[float, float, float] | None, width: float) -> None:
    doc = fitz.open()
    page = doc.new_page(width=width, height=PAGE_HEIGHT)
    if rgb is not None:
        page.draw_rect(
            fitz.Rect(36, 36, width - 36, PAGE_HEIGHT - 36),
            color=rgb,
            fill=rgb,
            width=0,
            overlay=True,
        )
    doc.save(path)
    doc.close()


def build_templates() -> None:
    _make_single_template(FRONT_TEMPLATE, (0.95, 0.95, 0.95), PAGE_WIDTH_BY_TEMPLATE["tpl-front"])
    _make_single_template(BACK_BLUE_TEMPLATE, (0.72, 0.84, 1.0), PAGE_WIDTH_BY_TEMPLATE["tpl-back-blue"])
    _make_single_template(BACK_GREEN_TEMPLATE, (0.79, 0.94, 0.79), PAGE_WIDTH_BY_TEMPLATE["tpl-back-green"])
    _make_single_template(BACK_BLANK_TEMPLATE, None, PAGE_WIDTH_BY_TEMPLATE["tpl-back-blank"])


def records() -> list[dict[str, str]]:
    back_kinds = ["blue", "blank", "green", "blank", "blue", "green", "blank", "blue", "green", "blank"]
    out: list[dict[str, str]] = []
    for i, back in enumerate(back_kinds, start=1):
        out.append({"record_id": f"R{i:02d}", "name": f"Recipient {i:02d}", "back_kind": back})
    return out


def html_for_records(items: list[dict[str, str]]) -> str:
    pages: list[str] = []
    for rec in items:
        rid = rec["record_id"]
        back = rec["back_kind"]
        pages.append(
            f"""
<section class="page front">
  <header data-fb="fb.feature.front=1"></header>
  <footer></footer>
  <h1>FRONT RECORD {rid}</h1>
  <p>Name: {rec["name"]}</p>
  <p>Back policy: {back}</p>
</section>
""".strip()
        )
        if back == "blank":
            pages.append(
                """
<section class="page back">
  <header data-fb="fb.feature.back_blank=1"></header>
  <footer></footer>
</section>
""".strip()
            )
        else:
            pages.append(
                f"""
<section class="page back">
  <header data-fb="fb.feature.back_{back}=1"></header>
  <footer></footer>
  <h2>BACK DATA {rid}</h2>
  <p>Terms packet: {back.upper()}</p>
</section>
""".strip()
            )
    return "<!doctype html><html><body>" + "\n".join(pages) + "</body></html>"


def css() -> str:
    return """
@page { size: 8.5in 11in; margin: 0.6in; }
body { margin: 0; font-family: Helvetica, Arial, sans-serif; color: #111; }
.page { min-height: 9.8in; }
.page:not(:last-child) { break-after: page; }
h1 { margin: 0 0 8pt 0; font-size: 20pt; }
h2 { margin: 0 0 8pt 0; font-size: 16pt; }
p { margin: 0 0 6pt 0; font-size: 11pt; }
header, footer { height: 0; margin: 0; padding: 0; overflow: hidden; }
""".strip()


def build_engine() -> fullbleed.PdfEngine:
    return fullbleed.PdfEngine(
        template_binding={
            "default_template_id": "tpl-front",
            "by_feature": {
                "front": "tpl-front",
                "back_blue": "tpl-back-blue",
                "back_green": "tpl-back-green",
                "back_blank": "tpl-back-blank",
            },
            "feature_prefix": "fb.feature.",
        }
    )


def expected_template_for_back_kind(back_kind: str) -> str:
    if back_kind == "blue":
        return "tpl-back-blue"
    if back_kind == "green":
        return "tpl-back-green"
    return "tpl-back-blank"


def validate_pdf_asset_bundle(template_paths: list[Path]) -> dict:
    bundle = fullbleed.AssetBundle()
    vendored_infos: list[dict] = []
    for i, path in enumerate(template_paths, start=1):
        name = f"tpl-{i}"
        vendored = fullbleed.vendored_asset(str(path), "pdf", name=name)
        v_info = vendored.info()
        if v_info.get("kind") != "pdf":
            raise RuntimeError(f"vendored_asset kind mismatch for {path}")
        if bool(v_info.get("encrypted")):
            raise RuntimeError(f"encrypted template asset unexpectedly accepted: {path}")
        if int(v_info.get("page_count") or 0) < 1:
            raise RuntimeError(f"template asset has no pages: {path}")
        vendored_infos.append(v_info)
        bundle.add_file(str(path), "pdf", name=name)

    bundled_infos = [info for info in bundle.assets_info() if info.get("kind") == "pdf"]
    if len(bundled_infos) != len(template_paths):
        raise RuntimeError(
            f"asset bundle pdf count mismatch expected={len(template_paths)} got={len(bundled_infos)}"
        )
    for info in bundled_infos:
        if bool(info.get("encrypted")):
            raise RuntimeError("encrypted pdf reported in bundle metadata")
        if int(info.get("page_count") or 0) < 1:
            raise RuntimeError("invalid page_count reported in bundle metadata")

    # Validation smoke: ensure non-PDF content is rejected as a PDF asset.
    invalid_probe = OUT / "_invalid_pdf_probe.txt"
    invalid_probe.write_text("not a pdf", encoding="utf-8")
    invalid_pdf_rejected = False
    invalid_error = None
    try:
        fullbleed.vendored_asset(str(invalid_probe), "pdf")
    except Exception as exc:
        invalid_pdf_rejected = True
        invalid_error = str(exc)
    finally:
        invalid_probe.unlink(missing_ok=True)
    if not invalid_pdf_rejected:
        raise RuntimeError("invalid non-pdf asset was unexpectedly accepted as kind=pdf")

    return {
        "ok": True,
        "vendored_assets": vendored_infos,
        "bundle_assets": bundled_infos,
        "invalid_pdf_rejected": invalid_pdf_rejected,
        "invalid_pdf_error": invalid_error,
    }


def run() -> dict:
    build_templates()
    recs = records()
    asset_bundle = validate_pdf_asset_bundle(
        [FRONT_TEMPLATE, BACK_BLUE_TEMPLATE, BACK_GREEN_TEMPLATE, BACK_BLANK_TEMPLATE]
    )

    engine = build_engine()
    overlay_bytes, _page_data, bindings = engine.render_pdf_with_page_data_and_template_bindings(
        html_for_records(recs),
        css(),
    )
    OVERLAY_PDF.write_bytes(overlay_bytes)

    templates = [
        ("tpl-front", str(FRONT_TEMPLATE)),
        ("tpl-back-blue", str(BACK_BLUE_TEMPLATE)),
        ("tpl-back-green", str(BACK_GREEN_TEMPLATE)),
        ("tpl-back-blank", str(BACK_BLANK_TEMPLATE)),
    ]

    plan: list[tuple[str, int, int, float, float]] = []
    if not isinstance(bindings, list):
        raise RuntimeError("bindings payload missing")
    binding_template_ids: list[str] = []
    for b in bindings:
        page_index = int(b["page_index"])
        template_id = str(b["template_id"])
        binding_template_ids.append(template_id)
        plan.append((template_id, 0, page_index, 0.0, 0.0))

    compose = fullbleed.finalize_compose_pdf(templates, plan, str(OVERLAY_PDF), str(COMPOSED_PDF))

    doc = fitz.open(COMPOSED_PDF)
    checks: list[dict] = []
    try:
        expected_pages = len(recs) * 2
        if doc.page_count != expected_pages:
            raise RuntimeError(f"page count mismatch expected={expected_pages} got={doc.page_count}")

        for i, rec in enumerate(recs):
            front_ix = i * 2
            back_ix = front_ix + 1

            front_text = doc[front_ix].get_text("text")
            front_ok = f"FRONT RECORD {rec['record_id']}" in front_text
            front_width = int(round(float(doc[front_ix].rect.width)))
            front_width_ok = front_width == PAGE_WIDTH_BY_TEMPLATE["tpl-front"]

            back_text = doc[back_ix].get_text("text")
            back_kind = rec["back_kind"]
            expected_back_template = expected_template_for_back_kind(back_kind)
            expected_back_width = PAGE_WIDTH_BY_TEMPLATE[expected_back_template]
            back_width = int(round(float(doc[back_ix].rect.width)))
            back_width_ok = back_width == expected_back_width
            back_binding_ok = binding_template_ids[back_ix] == expected_back_template
            if back_kind == "blank":
                back_ok = ("BACK DATA" not in back_text) and back_width_ok and back_binding_ok
            else:
                back_ok = (f"BACK DATA {rec['record_id']}" in back_text) and back_width_ok and back_binding_ok

            checks.append(
                {
                    "record_id": rec["record_id"],
                    "front_page": front_ix + 1,
                    "front_ok": front_ok,
                    "front_width": front_width,
                    "front_width_ok": front_width_ok,
                    "back_page": back_ix + 1,
                    "back_kind": back_kind,
                    "expected_back_template": expected_back_template,
                    "bound_template": binding_template_ids[back_ix],
                    "back_width": back_width,
                    "back_width_ok": back_width_ok,
                    "back_binding_ok": back_binding_ok,
                    "back_ok": back_ok,
                }
            )

        ok = all(item["front_ok"] and item["front_width_ok"] and item["back_ok"] for item in checks)
        return {
            "schema": "fullbleed.vdp_backpage_smoke.v1",
            "ok": ok,
            "record_count": len(recs),
            "output_page_count": doc.page_count,
            "asset_bundle": asset_bundle,
            "overlay_pdf": str(OVERLAY_PDF),
            "composed_pdf": str(COMPOSED_PDF),
            "compose": compose,
            "checks": checks,
        }
    finally:
        doc.close()


def main() -> None:
    report = run()
    REPORT_JSON.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=True))
    if not report.get("ok"):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
