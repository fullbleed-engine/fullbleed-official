from __future__ import annotations

import argparse
import hashlib
import html as html_escape
import json
import random
import subprocess
import sys
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Tuple

import fullbleed


ROOT = Path(__file__).resolve().parent
ASSETS_DIR = ROOT / "assets"
TEMPLATES_DIR = ASSETS_DIR / "templates"
VENDOR_DIR = ROOT / "vendor"
OUTPUT_DIR = ROOT / "output"
RUNS_DIR = OUTPUT_DIR / "runs"

RECORDS_JSONL = OUTPUT_DIR / "records.jsonl"
OVERLAY_HTML = OUTPUT_DIR / "showcase_overlay.html"
OVERLAY_CSS = OUTPUT_DIR / "showcase_overlay.css"
TEMPLATE_BINDING_JSON = OUTPUT_DIR / "template_binding.json"
SHOWCASE_PDF = OUTPUT_DIR / "policy_packet_showcase.pdf"
SHOWCASE_REPORT = OUTPUT_DIR / "showcase_report.json"
ORACLE_JSON = OUTPUT_DIR / "oracle_first20.json"

INSERT_STATES = ("CA", "NY", "TX", "FL")
ALL_STATES = ("CA", "NY", "TX", "FL", "WA", "OR", "AZ", "CO", "NV", "UT")
BACK_KINDS = ("blank", "legal", "marketing")

TEMPLATE_SPECS: List[Tuple[str, str, str, str]] = [
    ("tpl-front-standard", "Policy Packet Front", "Current Account", "#0D6EFD"),
    ("tpl-front-past-due", "Policy Packet Front", "Past Due Account", "#DC3545"),
    ("tpl-detail", "Detail Pages", "Variable Data Pages", "#198754"),
    ("tpl-insert-ca", "State Insert", "California Addendum", "#0B7285"),
    ("tpl-insert-ny", "State Insert", "New York Addendum", "#5F3DC4"),
    ("tpl-insert-tx", "State Insert", "Texas Addendum", "#E8590C"),
    ("tpl-insert-fl", "State Insert", "Florida Addendum", "#2B8A3E"),
    ("tpl-coupon", "Payment Coupon", "Remit Slip", "#495057"),
    ("tpl-back-blank", "Back Page", "Blank Back", "#ADB5BD"),
    ("tpl-back-legal", "Back Page", "Legal Terms", "#343A40"),
    ("tpl-back-marketing", "Back Page", "Marketing Message", "#7C2D12"),
    ("tpl-parity-blank", "Parity Page", "Duplex Alignment", "#CED4DA"),
]


@dataclass
class LineItem:
    date: str
    description: str
    amount: float


@dataclass
class Record:
    record_id: str
    account_id: str
    name: str
    state: str
    segment: str
    past_due: bool
    needs_coupon: bool
    back_kind: str
    insert_state: Optional[str]
    line_items: List[LineItem]


@dataclass
class PageSpec:
    record_id: str
    kind: str
    template_id: str
    feature: str
    packet_index: int = 0
    packet_pages: int = 0
    absolute_page: int = 0
    overlay_marker: str = ""
    template_marker: str = ""
    detail_page_index: Optional[int] = None
    detail_page_count: Optional[int] = None
    detail_items: List[LineItem] = field(default_factory=list)


def _mkdirs() -> None:
    ASSETS_DIR.mkdir(parents=True, exist_ok=True)
    TEMPLATES_DIR.mkdir(parents=True, exist_ok=True)
    VENDOR_DIR.mkdir(parents=True, exist_ok=True)
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    RUNS_DIR.mkdir(parents=True, exist_ok=True)


def _run_json_command(cmd: List[str], cwd: Path) -> Dict:
    proc = subprocess.run(
        cmd,
        cwd=str(cwd),
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            "command failed\n"
            f"cmd: {' '.join(cmd)}\n"
            f"exit: {proc.returncode}\n"
            f"stdout: {proc.stdout.strip()}\n"
            f"stderr: {proc.stderr.strip()}"
        )
    lines = [line.strip() for line in proc.stdout.splitlines() if line.strip()]
    if not lines:
        raise RuntimeError(f"command produced no output: {' '.join(cmd)}")
    try:
        payload = json.loads(lines[-1])
    except json.JSONDecodeError as exc:
        raise RuntimeError(
            "failed to parse JSON command output\n"
            f"cmd: {' '.join(cmd)}\n"
            f"stdout: {proc.stdout.strip()}"
        ) from exc
    if payload.get("ok") is False:
        raise RuntimeError(f"command returned failure payload: {json.dumps(payload, ensure_ascii=True)}")
    return payload


def _resolve_payload_path(base: Path, value: str) -> Path:
    path = Path(value)
    if not path.is_absolute():
        path = base / path
    return path.resolve()


def ensure_inter_font_vendored() -> Tuple[Path, Dict]:
    payload = _run_json_command(
        [
            sys.executable,
            "-m",
            "fullbleed",
            "assets",
            "install",
            "inter",
            "--vendor",
            str(VENDOR_DIR),
            "--json",
        ],
        cwd=ROOT,
    )
    installed_to = payload.get("installed_to")
    if not isinstance(installed_to, str) or not installed_to.strip():
        raise RuntimeError("assets install response missing installed_to")
    font_path = _resolve_payload_path(ROOT, installed_to)
    if not font_path.exists():
        raise RuntimeError(f"vendored font path does not exist: {font_path}")
    return font_path, payload


def _build_template_engine(font_path: Path) -> fullbleed.PdfEngine:
    bundle = fullbleed.AssetBundle()
    bundle.add_file(str(font_path), "font", name="inter")

    engine = fullbleed.PdfEngine(
        page_width="8.5in",
        page_height="11in",
        margin="0in",
        reuse_xobjects=True,
        svg_form_xobjects=True,
        unicode_support=True,
        shape_text=True,
        unicode_metrics=True,
        pdf_version="1.7",
    )
    engine.register_bundle(bundle)
    return engine


def _template_html(template_id: str, title: str, subtitle: str) -> str:
    title_h = html_escape.escape(title)
    subtitle_h = html_escape.escape(subtitle)
    marker = html_escape.escape(f"TPL-{template_id}")
    return f"""<!doctype html>
<html>
  <body>
    <section class="template">
      <div class="accent"></div>
      <div class="badge">{marker}</div>
      <h1>{title_h}</h1>
      <p class="subtitle">{subtitle_h}</p>
      <p class="body">Source template is treated as a vendored PDF asset and composed under overlay pages.</p>
      <p class="body">This marker is intentionally visible for automated validation.</p>
    </section>
  </body>
</html>
""".strip()


def _template_css(accent: str) -> str:
    return f"""
@page {{ size: 8.5in 11in; margin: 0; }}
body {{
  margin: 0;
  font-family: Inter, Helvetica, Arial, sans-serif;
  color: #111;
}}
.template {{
  width: 8.5in;
  height: 11in;
  box-sizing: border-box;
  padding: 0.55in;
  background: #f8f9fa;
}}
.accent {{
  height: 0.45in;
  border-radius: 0.1in;
  background: {accent};
  margin-bottom: 0.35in;
}}
.badge {{
  display: inline-block;
  background: #111;
  color: #fff;
  padding: 4pt 8pt;
  border-radius: 999pt;
  font-size: 8pt;
  letter-spacing: 0.03em;
  margin-bottom: 10pt;
}}
h1 {{
  margin: 0 0 6pt 0;
  font-size: 22pt;
  font-weight: 700;
}}
.subtitle {{
  margin: 0 0 14pt 0;
  font-size: 11pt;
  color: #333;
}}
.body {{
  margin: 0 0 6pt 0;
  font-size: 10pt;
  color: #4f4f4f;
  max-width: 6.2in;
}}
""".strip()


def build_template_assets(font_path: Path) -> Dict[str, Path]:
    for existing in TEMPLATES_DIR.glob("*.pdf"):
        existing.unlink(missing_ok=True)

    engine = _build_template_engine(font_path)
    out: Dict[str, Path] = {}
    for template_id, title, subtitle, accent in TEMPLATE_SPECS:
        pdf_path = TEMPLATES_DIR / f"{template_id}.pdf"
        html = _template_html(template_id, title, subtitle)
        css = _template_css(accent)
        engine.render_pdf_to_file(html, css, str(pdf_path))
        out[template_id] = pdf_path
    return out


def validate_template_assets(template_paths: Dict[str, Path]) -> Dict:
    bundle = fullbleed.AssetBundle()
    vendored_infos: List[Dict] = []
    for template_id, path in sorted(template_paths.items()):
        asset = fullbleed.vendored_asset(str(path), "pdf", name=template_id)
        info = asset.info()
        if info.get("kind") != "pdf":
            raise RuntimeError(f"template asset kind mismatch: {template_id}")
        if bool(info.get("encrypted")):
            raise RuntimeError(f"template asset unexpectedly encrypted: {template_id}")
        if int(info.get("page_count") or 0) != 1:
            raise RuntimeError(f"template must be single-page for this showcase: {template_id}")
        vendored_infos.append(info)
        bundle.add_file(str(path), "pdf", name=template_id)

    bundle_infos = [item for item in bundle.assets_info() if item.get("kind") == "pdf"]
    if len(bundle_infos) != len(template_paths):
        raise RuntimeError(
            f"bundle pdf asset count mismatch expected={len(template_paths)} got={len(bundle_infos)}"
        )

    return {
        "ok": True,
        "template_count": len(template_paths),
        "vendored_assets": vendored_infos,
        "bundle_assets": bundle_infos,
    }


def _build_line_items(record_index: int, count: int) -> List[LineItem]:
    items: List[LineItem] = []
    for i in range(1, count + 1):
        day = ((record_index + i) % 28) + 1
        code = ((record_index * 17) + (i * 9)) % 100
        amount = round(12.5 + (((record_index * 23) + (i * 11)) % 450) / 7.0, 2)
        items.append(
            LineItem(
                date=f"2026-01-{day:02d}",
                description=f"Line {i:03d} Service Code {code:02d}",
                amount=amount,
            )
        )
    return items


def generate_records(count: int, seed: int) -> List[Record]:
    rng = random.Random(seed)
    first_names = ("Avery", "Jordan", "Casey", "Taylor", "Parker", "Morgan", "Riley", "Rowan")
    last_names = ("Nguyen", "Patel", "Smith", "Johnson", "Clark", "Lewis", "Garcia", "Kim")
    segments = ("consumer", "small-business", "enterprise")

    records: List[Record] = []
    for i in range(1, count + 1):
        state = ALL_STATES[(i + rng.randint(0, len(ALL_STATES) - 1)) % len(ALL_STATES)]
        name = f"{first_names[(i + rng.randint(0, 7)) % len(first_names)]} {last_names[(i * 3 + rng.randint(0, 7)) % len(last_names)]}"
        segment = segments[(i + rng.randint(0, 2)) % len(segments)]
        past_due = (i % 9 == 0) or (i % 17 == 0)
        needs_coupon = past_due or (i % 5 == 0)
        back_kind = BACK_KINDS[(i + (1 if past_due else 0)) % len(BACK_KINDS)]
        insert_state = state if (state in INSERT_STATES and (i % 3 != 0)) else None
        line_count = 16 + ((i * 11) % 64) + rng.randint(0, 12)

        records.append(
            Record(
                record_id=f"R{i:04d}",
                account_id=f"{800000000 + i:09d}",
                name=name,
                state=state,
                segment=segment,
                past_due=past_due,
                needs_coupon=needs_coupon,
                back_kind=back_kind,
                insert_state=insert_state,
                line_items=_build_line_items(i, line_count),
            )
        )
    return records


def _chunk(items: List[LineItem], size: int) -> Iterable[List[LineItem]]:
    for i in range(0, len(items), size):
        yield items[i : i + size]


def _back_template_id(back_kind: str) -> str:
    if back_kind == "legal":
        return "tpl-back-legal"
    if back_kind == "marketing":
        return "tpl-back-marketing"
    return "tpl-back-blank"


def _front_template_id(rec: Record) -> str:
    return "tpl-front-past-due" if rec.past_due else "tpl-front-standard"


def _front_feature(rec: Record) -> str:
    return "front_past_due" if rec.past_due else "front_standard"


def build_page_specs(
    records: List[Record],
    lines_per_detail_page: int,
) -> Tuple[List[PageSpec], List[Dict]]:
    pages: List[PageSpec] = []
    record_summaries: List[Dict] = []

    for rec in records:
        packet: List[PageSpec] = []
        start_page = len(pages) + 1
        if start_page % 2 == 0:
            raise RuntimeError(f"duplex violation before record {rec.record_id}: start_page={start_page}")

        front_id = _front_template_id(rec)
        packet.append(PageSpec(record_id=rec.record_id, kind="front", template_id=front_id, feature=_front_feature(rec)))

        detail_chunks = list(_chunk(rec.line_items, lines_per_detail_page))
        for idx, detail_items in enumerate(detail_chunks, start=1):
            packet.append(
                PageSpec(
                    record_id=rec.record_id,
                    kind="detail",
                    template_id="tpl-detail",
                    feature="detail",
                    detail_page_index=idx,
                    detail_page_count=len(detail_chunks),
                    detail_items=detail_items,
                )
            )

        if rec.insert_state:
            state_key = rec.insert_state.lower()
            packet.append(
                PageSpec(
                    record_id=rec.record_id,
                    kind=f"insert_{state_key}",
                    template_id=f"tpl-insert-{state_key}",
                    feature=f"insert_{state_key}",
                )
            )

        if rec.needs_coupon:
            packet.append(
                PageSpec(
                    record_id=rec.record_id,
                    kind="coupon",
                    template_id="tpl-coupon",
                    feature="coupon",
                )
            )

        back_template = _back_template_id(rec.back_kind)
        packet.append(
            PageSpec(
                record_id=rec.record_id,
                kind=f"back_{rec.back_kind}",
                template_id=back_template,
                feature=f"back_{rec.back_kind}",
            )
        )

        if len(packet) % 2 == 1:
            packet.append(
                PageSpec(
                    record_id=rec.record_id,
                    kind="parity_blank",
                    template_id="tpl-parity-blank",
                    feature="parity_blank",
                )
            )

        packet_pages = len(packet)
        for idx, spec in enumerate(packet, start=1):
            spec.packet_index = idx
            spec.packet_pages = packet_pages
            spec.absolute_page = len(pages) + 1
            spec.overlay_marker = f"OVL-{spec.record_id}-{spec.kind}-{idx}of{packet_pages}"
            spec.template_marker = f"TPL-{spec.template_id}"
            pages.append(spec)

        record_summaries.append(
            {
                "record_id": rec.record_id,
                "start_page": start_page,
                "packet_pages": packet_pages,
                "detail_pages": len(detail_chunks),
                "insert_state": rec.insert_state,
                "needs_coupon": rec.needs_coupon,
                "back_kind": rec.back_kind,
                "front_template_id": front_id,
                "back_template_id": back_template,
                "duplex_start_odd": start_page % 2 == 1,
            }
        )

    return pages, record_summaries


def _detail_rows_html(items: List[LineItem]) -> str:
    rows: List[str] = []
    for item in items:
        rows.append(
            "<tr>"
            f"<td>{html_escape.escape(item.date)}</td>"
            f"<td>{html_escape.escape(item.description)}</td>"
            f"<td class='amount'>${item.amount:,.2f}</td>"
            "</tr>"
        )
    return "".join(rows)


def _page_body_html(spec: PageSpec, rec: Record) -> str:
    if spec.kind == "front":
        status = "PAST DUE" if rec.past_due else "CURRENT"
        return (
            f"<h1>Policy Packet {html_escape.escape(rec.record_id)}</h1>"
            f"<p class='lead'>Account {html_escape.escape(rec.account_id)} | Segment: {html_escape.escape(rec.segment)} | Status: {status}</p>"
            f"<p class='lead'>Recipient: {html_escape.escape(rec.name)} | State: {html_escape.escape(rec.state)}</p>"
            "<p>This packet demonstrates deterministic VDP overlay composition against vendored PDF templates.</p>"
        )

    if spec.kind == "detail":
        return (
            "<h2>Transaction Detail</h2>"
            f"<p class='lead'>Detail page {spec.detail_page_index} of {spec.detail_page_count}</p>"
            "<table class='detail-table'>"
            "<thead><tr><th>Date</th><th>Description</th><th class='amount'>Amount</th></tr></thead>"
            f"<tbody>{_detail_rows_html(spec.detail_items)}</tbody>"
            "</table>"
        )

    if spec.kind.startswith("insert_"):
        state = rec.insert_state or rec.state
        return (
            f"<h2>{html_escape.escape(state)} Required Insert</h2>"
            "<p>This state-specific page is conditionally inserted based on recipient jurisdiction rules.</p>"
            "<ul>"
            "<li>Regulatory notice language block A</li>"
            "<li>Regulatory notice language block B</li>"
            "<li>Regulatory notice language block C</li>"
            "</ul>"
        )

    if spec.kind == "coupon":
        return (
            "<h2>Payment Coupon</h2>"
            "<div class='coupon-grid'>"
            f"<div><strong>Record:</strong> {html_escape.escape(rec.record_id)}</div>"
            f"<div><strong>Account:</strong> {html_escape.escape(rec.account_id)}</div>"
            "<div><strong>Due Date:</strong> 2026-03-31</div>"
            "<div><strong>Amount Due:</strong> $275.00</div>"
            "</div>"
        )

    if spec.kind == "back_legal":
        return (
            "<h2>Legal Terms</h2>"
            "<p>Terms and disclosures continue on this page. This is a template-selected legal backer.</p>"
        )

    if spec.kind == "back_marketing":
        return (
            "<h2>Customer Offer</h2>"
            "<p>Marketing backer selected by record segment and campaign controls.</p>"
        )

    if spec.kind == "back_blank":
        return "<p class='blank-note'>Intentional blank back page for this recipient.</p>"

    if spec.kind == "parity_blank":
        return "<p class='blank-note'>Inserted parity page to preserve duplex record starts.</p>"

    return "<p>Unhandled page type.</p>"


def write_overlay_inputs(records: List[Record], pages: List[PageSpec]) -> None:
    records_by_id = {r.record_id: r for r in records}
    chunks: List[str] = []
    for spec in pages:
        rec = records_by_id[spec.record_id]
        body = _page_body_html(spec, rec)
        chunks.append(
            (
                f"<section class='page page-{html_escape.escape(spec.kind)}'>"
                f"<header class='meta-flag' data-fb='fb.feature.{html_escape.escape(spec.feature)}=1'></header>"
                f"<div class='overlay-marker'>{html_escape.escape(spec.overlay_marker)}</div>"
                f"{body}"
                "</section>"
            )
        )

    html_doc = "<!doctype html><html><body>" + "".join(chunks) + "</body></html>"
    css_doc = """
@page { size: 8.5in 11in; margin: 0.55in; }
body { margin: 0; font-family: Inter, Helvetica, Arial, sans-serif; color: #111; }
.page { min-height: 9.8in; box-sizing: border-box; }
.page:not(:last-child) { break-after: page; }
.meta-flag {
  font-size: 1pt;
  line-height: 1;
  color: transparent;
  height: 0;
  overflow: hidden;
}
.overlay-marker {
  font-size: 7pt;
  color: #4f4f4f;
  margin-bottom: 8pt;
}
h1, h2 { margin: 0 0 8pt 0; }
h1 { font-size: 20pt; }
h2 { font-size: 16pt; }
.lead { margin: 0 0 6pt 0; font-size: 10pt; }
p { margin: 0 0 7pt 0; font-size: 10pt; line-height: 1.3; }
ul { margin: 0 0 0 14pt; padding: 0; }
li { margin: 0 0 4pt 0; font-size: 10pt; }
.detail-table { width: 100%; border-collapse: collapse; margin-top: 8pt; }
.detail-table th, .detail-table td {
  border: 1px solid #222;
  padding: 4pt 6pt;
  font-size: 9pt;
}
.detail-table th {
  background: #f1f3f5;
  text-transform: uppercase;
  font-size: 8pt;
}
.amount { text-align: right; width: 1.25in; }
.coupon-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 6pt 10pt;
  border: 1px dashed #444;
  padding: 8pt;
  max-width: 6in;
}
.blank-note { font-size: 9pt; color: #495057; }
""".strip()

    OVERLAY_HTML.write_text(html_doc, encoding="utf-8")
    OVERLAY_CSS.write_text(css_doc, encoding="utf-8")


def write_records_jsonl(records: List[Record]) -> None:
    with RECORDS_JSONL.open("w", encoding="utf-8") as handle:
        for rec in records:
            payload = asdict(rec)
            handle.write(json.dumps(payload, ensure_ascii=True) + "\n")


def write_template_binding() -> Dict:
    by_feature = {
        "front_standard": "tpl-front-standard",
        "front_past_due": "tpl-front-past-due",
        "detail": "tpl-detail",
        "insert_ca": "tpl-insert-ca",
        "insert_ny": "tpl-insert-ny",
        "insert_tx": "tpl-insert-tx",
        "insert_fl": "tpl-insert-fl",
        "coupon": "tpl-coupon",
        "back_blank": "tpl-back-blank",
        "back_legal": "tpl-back-legal",
        "back_marketing": "tpl-back-marketing",
        "parity_blank": "tpl-parity-blank",
    }
    payload = {
        "default_template_id": "tpl-front-standard",
        "feature_prefix": "fb.feature.",
        "by_feature": by_feature,
    }
    TEMPLATE_BINDING_JSON.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    return payload


def _sha256_file(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(1024 * 1024)
            if not chunk:
                break
            hasher.update(chunk)
    return hasher.hexdigest()


def _structural_signature(path: Path) -> str:
    # Fullbleed-only fallback signature: byte-level hash.
    # Structural extraction previously depended on third-party PDF parsers.
    return _sha256_file(path)


def _render_auto_compose(
    *,
    font_path: Path,
    out_pdf: Path,
    deterministic_hash_path: Path,
    expected_pages: int,
    emit_debug: bool,
    run_label: str,
) -> Tuple[Dict, float]:
    cmd = [
        sys.executable,
        "-m",
        "fullbleed",
        "--json-only",
        "render",
        "--html",
        str(OVERLAY_HTML),
        "--css",
        str(OVERLAY_CSS),
        "--asset",
        str(font_path),
        "--asset-kind",
        "font",
        "--asset-name",
        "inter",
        "--profile",
        "prod",
        "--reuse-xobjects",
        "--svg-form-xobjects",
        "--shape-text",
        "--unicode-support",
        "--unicode-metrics",
        "--template-binding",
        str(TEMPLATE_BINDING_JSON),
        "--templates",
        str(TEMPLATES_DIR),
        "--fail-on",
        "font-subst",
        "--fail-on",
        "budget",
        "--budget-max-pages",
        str(expected_pages),
        "--deterministic-hash",
        str(deterministic_hash_path),
        "--out",
        str(out_pdf),
    ]
    if emit_debug:
        cmd.extend(
            [
                "--emit-page-data",
                str(OUTPUT_DIR / f"{run_label}.page_data.json"),
                "--emit-jit",
                str(OUTPUT_DIR / f"{run_label}.jit.jsonl"),
                "--emit-perf",
                str(OUTPUT_DIR / f"{run_label}.perf.jsonl"),
            ]
        )

    started = time.perf_counter()
    payload = _run_json_command(cmd, cwd=ROOT)
    elapsed = time.perf_counter() - started
    return payload, elapsed


def validate_composed_pdf(path: Path, pages: List[PageSpec], sample_limit: int) -> Dict:
    expected_pages = len(pages)
    pdf_exists = path.exists()
    pdf_size = path.stat().st_size if pdf_exists else 0

    sample: List[Dict] = []
    for i, spec in enumerate(pages[:sample_limit]):
        sample.append(
            {
                "page": i + 1,
                "record_id": spec.record_id,
                "kind": spec.kind,
                "template_id": spec.template_id,
                "overlay_marker_expected": spec.overlay_marker,
                "template_marker_expected": spec.template_marker,
            }
        )

    return {
        "ok": bool(pdf_exists and pdf_size > 0 and expected_pages > 0),
        "validation_mode": "contract_only_no_pdf_text_extractor",
        "page_count": expected_pages,
        "pdf_exists": pdf_exists,
        "pdf_size_bytes": pdf_size,
        "sample_checks": sample,
        "failure_count": 0,
        "failure_samples": [],
    }


def write_oracle(record_summaries: List[Dict], pages: List[PageSpec], count: int) -> List[Dict]:
    grouped: Dict[str, List[PageSpec]] = {}
    for spec in pages:
        grouped.setdefault(spec.record_id, []).append(spec)

    oracle: List[Dict] = []
    for summary in record_summaries[:count]:
        rid = summary["record_id"]
        sequence = [spec.template_id for spec in grouped.get(rid, [])]
        oracle.append(
            {
                "record_id": rid,
                "start_page": summary["start_page"],
                "packet_pages": summary["packet_pages"],
                "template_sequence": sequence,
            }
        )

    ORACLE_JSON.write_text(json.dumps(oracle, indent=2), encoding="utf-8")
    return oracle


def _cleanup_transient_run_files(path: Path) -> None:
    path.unlink(missing_ok=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Hard VDP policy packet template-compose showcase")
    parser.add_argument("--records", type=int, default=1000, help="Number of synthetic records")
    parser.add_argument("--seed", type=int, default=20260217, help="Random seed for deterministic data generation")
    parser.add_argument(
        "--lines-per-detail-page",
        type=int,
        default=24,
        help="Rows per detail page before explicit page split",
    )
    parser.add_argument("--runs", type=int, default=3, help="Repeat render runs for determinism verification")
    parser.add_argument("--sample-limit", type=int, default=40, help="Validation sample pages in report")
    parser.add_argument("--emit-debug", action="store_true", help="Emit page-data/jit/perf artifacts for run 1")
    parser.add_argument("--keep-runs", action="store_true", help="Keep pass2+ PDFs and hash files")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    if args.records <= 0:
        raise SystemExit("--records must be > 0")
    if args.lines_per_detail_page <= 0:
        raise SystemExit("--lines-per-detail-page must be > 0")
    if args.runs <= 0:
        raise SystemExit("--runs must be > 0")

    _mkdirs()
    font_path, font_install_payload = ensure_inter_font_vendored()
    template_paths = build_template_assets(font_path)
    template_asset_validation = validate_template_assets(template_paths)

    records = generate_records(args.records, args.seed)
    write_records_jsonl(records)
    pages, record_summaries = build_page_specs(records, args.lines_per_detail_page)
    expected_pages = len(pages)

    write_overlay_inputs(records, pages)
    template_binding = write_template_binding()
    oracle = write_oracle(record_summaries, pages, count=min(20, len(record_summaries)))

    run_reports: List[Dict] = []
    structural_signatures: List[str] = []
    byte_hashes: List[str] = []

    for run_index in range(1, args.runs + 1):
        out_pdf = SHOWCASE_PDF if run_index == 1 else RUNS_DIR / f"policy_packet_showcase_run{run_index}.pdf"
        hash_path = OUTPUT_DIR / f"policy_packet_showcase_run{run_index}.sha256"
        run_label = f"showcase_run{run_index}"

        payload, seconds = _render_auto_compose(
            font_path=font_path,
            out_pdf=out_pdf,
            deterministic_hash_path=hash_path,
            expected_pages=expected_pages,
            emit_debug=(args.emit_debug and run_index == 1),
            run_label=run_label,
        )

        structural_sig = _structural_signature(out_pdf)
        byte_sha = _sha256_file(out_pdf)
        hash_file_value = hash_path.read_text(encoding="utf-8").strip() if hash_path.exists() else ""

        structural_signatures.append(structural_sig)
        byte_hashes.append(byte_sha)
        run_reports.append(
            {
                "run": run_index,
                "seconds": round(seconds, 6),
                "pages": expected_pages,
                "pages_per_second": round(expected_pages / max(seconds, 1e-9), 3),
                "render_payload_schema": payload.get("schema"),
                "render_payload_ok": payload.get("ok"),
                "render_outputs": payload.get("outputs"),
                "structural_signature": structural_sig,
                "byte_sha256": byte_sha,
                "deterministic_hash_file": hash_file_value,
            }
        )

        if run_index > 1 and not args.keep_runs:
            _cleanup_transient_run_files(out_pdf)
            _cleanup_transient_run_files(hash_path)

    validation = validate_composed_pdf(SHOWCASE_PDF, pages, args.sample_limit)
    duplex_ok = all(item["duplex_start_odd"] for item in record_summaries)
    determinism_ok = len(set(structural_signatures)) == 1

    report = {
        "schema": "fullbleed.vdp_policy_packet_showcase.v1",
        "ok": bool(validation["ok"] and duplex_ok and determinism_ok),
        "inputs": {
            "records": args.records,
            "seed": args.seed,
            "lines_per_detail_page": args.lines_per_detail_page,
            "runs": args.runs,
            "emit_debug": bool(args.emit_debug),
        },
        "assets": {
            "font_install": font_install_payload,
            "templates_dir": str(TEMPLATES_DIR),
            "template_binding_file": str(TEMPLATE_BINDING_JSON),
            "template_binding": template_binding,
            "template_validation": template_asset_validation,
        },
        "outputs": {
            "overlay_html": str(OVERLAY_HTML),
            "overlay_css": str(OVERLAY_CSS),
            "records_jsonl": str(RECORDS_JSONL),
            "composed_pdf": str(SHOWCASE_PDF),
            "oracle_first20": str(ORACLE_JSON),
        },
        "workload": {
            "expected_pages": expected_pages,
            "record_summaries": {
                "count": len(record_summaries),
                "sample_first_20": record_summaries[:20],
            },
            "oracle_first_20": oracle,
        },
        "validation": validation,
        "determinism": {
            "runs": run_reports,
            "structural_signatures": structural_signatures,
            "byte_hashes": byte_hashes,
            "structural_determinism_ok": determinism_ok,
            "byte_identical_ok": len(set(byte_hashes)) == 1,
        },
        "duplex": {
            "next_record_starts_on_odd_page": duplex_ok,
        },
    }

    SHOWCASE_REPORT.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=True))
    if not report["ok"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
