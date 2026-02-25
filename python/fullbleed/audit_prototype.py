from __future__ import annotations

import argparse
import hashlib
import json
from dataclasses import dataclass
from datetime import datetime, timezone
from html.parser import HTMLParser
from pathlib import Path
from typing import Any

from fullbleed.audit_wcag import wcag20aa_coverage_from_findings
from fullbleed.audit_section508 import section508_html_coverage_from_findings


def _root() -> Path:
    return Path(__file__).resolve().parents[2]


def _specs() -> Path:
    return _root() / "docs" / "specs"


def _j(path: str | Path) -> dict[str, Any]:
    return json.loads(Path(path).read_text(encoding="utf-8"))


def _j_opt(path: str | Path | None) -> dict[str, Any] | None:
    if path is None:
        return None
    p = Path(path)
    if not p.exists():
        return None
    return _j(p)


def _registry(path: str | Path | None = None) -> dict[str, Any]:
    return _j(Path(path) if path else (_specs() / "fullbleed.audit_registry.v1.yaml"))


def _sha(path: str | Path) -> str:
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        for chunk in iter(lambda: f.read(8192), b""):
            h.update(chunk)
    return f"sha256:{h.hexdigest()}"


def _now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def _i(value: Any, default: int = 0) -> int:
    try:
        return int(value)
    except Exception:
        return default


def _clamp(v: float, lo: float, hi: float) -> float:
    return max(lo, min(hi, v))


def _idrefs(value: str | None) -> list[str]:
    return [t for t in str(value or "").split() if t.strip()]


def _lang_ok(lang: str | None) -> bool:
    if not lang:
        return False
    text = str(lang).strip()
    if not text:
        return False
    allowed = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-")
    return all(ch in allowed for ch in text) and not text.startswith("-") and not text.endswith("-")


def _trueish(value: str | None) -> bool:
    return str(value or "").strip().lower() in {"1", "true", "yes", "on"}


def _invalidish(value: str | None) -> bool:
    text = str(value or "").strip().lower()
    if not text:
        return False
    return text not in {"0", "false", "no", "off"}


@dataclass
class HtmlFacts:
    html_lang: str | None
    title: str
    part_lang_attr_count: int
    invalid_part_lang_attr_count: int
    main_count: int
    ids: dict[str, int]
    dup_ids: list[str]
    missing_idrefs: list[tuple[str, str]]
    has_wrapper: bool
    has_css_link: bool
    css_hrefs: list[str]
    sig_count: int
    empty_heading_count: int
    empty_label_count: int
    empty_aria_label_count: int
    unlabeled_region_count: int
    image_count: int
    image_missing_alt_count: int
    image_title_only_count: int
    image_semantic_conflict_count: int
    form_control_count: int
    unlabeled_form_control_count: int
    invalid_form_control_count: int
    unidentified_error_form_control_count: int
    tabindex_attr_count: int
    positive_tabindex_count: int
    invalid_tabindex_count: int
    link_count: int
    unnamed_link_count: int
    generic_link_text_count: int
    custom_click_handler_count: int
    pointer_only_click_handler_count: int
    script_element_count: int
    embedded_active_content_count: int
    autoplay_media_count: int
    blink_marquee_count: int
    inline_event_handler_attr_count: int
    meta_refresh_count: int
    tables: list[dict[str, Any]]
    body_text: str


class _P(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.html_lang = None
        self.title_chunks: list[str] = []
        self.in_title = False
        self.in_body = False
        self.in_script_style = False
        self.body_chunks: list[str] = []
        self.has_html = False
        self.has_head = False
        self.has_body = False
        self.main_count = 0
        self.part_lang_attr_count = 0
        self.invalid_part_lang_attr_count = 0
        self.ids: dict[str, int] = {}
        self.dup_ids: list[str] = []
        self.idrefs: list[tuple[str, str]] = []
        self.css_hrefs: list[str] = []
        self.sig_count = 0
        self.empty_heading_count = 0
        self.empty_label_count = 0
        self.empty_aria_label_count = 0
        self.unlabeled_region_count = 0
        self.image_count = 0
        self.image_missing_alt_count = 0
        self.image_title_only_count = 0
        self.image_semantic_conflict_count = 0
        self.label_for_targets: set[str] = set()
        self._label_depth = 0
        self._form_controls: list[dict[str, Any]] = []
        self.tabindex_attr_count = 0
        self.positive_tabindex_count = 0
        self.invalid_tabindex_count = 0
        self.link_count = 0
        self.unnamed_link_count = 0
        self.generic_link_text_count = 0
        self.custom_click_handler_count = 0
        self.pointer_only_click_handler_count = 0
        self._text_capture_stack: list[dict[str, Any]] = []
        self.script_element_count = 0
        self.embedded_active_content_count = 0
        self.autoplay_media_count = 0
        self.blink_marquee_count = 0
        self.inline_event_handler_attr_count = 0
        self.meta_refresh_count = 0
        self.table_stack: list[dict[str, Any]] = []
        self.tables: list[dict[str, Any]] = []

    def handle_starttag(self, tag: str, attrs_in: list[tuple[str, str | None]]) -> None:
        self._tag(tag, attrs_in)

    def handle_startendtag(self, tag: str, attrs_in: list[tuple[str, str | None]]) -> None:
        self._tag(tag, attrs_in)

    def handle_endtag(self, tag: str) -> None:
        t = tag.lower()
        if t == "title":
            self.in_title = False
        elif t == "body":
            self.in_body = False
        elif t in {"script", "style"}:
            self.in_script_style = False
        elif t == "table" and self.table_stack:
            self.tables.append(self.table_stack.pop())
        elif t == "label" and self._label_depth > 0:
            self._label_depth -= 1
        if self._text_capture_stack:
            for idx in range(len(self._text_capture_stack) - 1, -1, -1):
                item = self._text_capture_stack[idx]
                if item.get("tag") != t:
                    continue
                text = "".join(item.get("chunks", [])).strip()
                if item.get("kind") == "heading" and not text:
                    self.empty_heading_count += 1
                elif item.get("kind") == "label" and not text:
                    self.empty_label_count += 1
                elif item.get("kind") == "link":
                    aria_label_nonempty = bool(item.get("aria_label_nonempty"))
                    aria_labelledby_nonempty = bool(item.get("aria_labelledby_nonempty"))
                    norm = " ".join(text.split()).strip().lower()
                    text_nonempty = bool(norm)
                    if not (aria_label_nonempty or aria_labelledby_nonempty or text_nonempty):
                        self.unnamed_link_count += 1
                    elif norm in {"click here", "here", "read more", "learn more", "more", "more..."}:
                        self.generic_link_text_count += 1
                self._text_capture_stack.pop(idx)
                break

    def handle_data(self, data: str) -> None:
        if self.in_title:
            self.title_chunks.append(data)
        if self.in_body and not self.in_script_style:
            self.body_chunks.append(data)
        if self._text_capture_stack and not self.in_script_style:
            for item in self._text_capture_stack:
                item["chunks"].append(data)

    def _tag(self, tag: str, attrs_in: list[tuple[str, str | None]]) -> None:
        t = tag.lower()
        attrs = {k.lower(): (v or "") for k, v in attrs_in}
        if t == "html":
            self.has_html = True
            lang = attrs.get("lang", "").strip()
            self.html_lang = lang or None
        elif t == "head":
            self.has_head = True
        elif t == "body":
            self.has_body = True
            self.in_body = True
        elif t == "title":
            self.in_title = True
        elif t in {"script", "style"}:
            self.in_script_style = True
            if t == "script":
                self.script_element_count += 1
        elif t == "main":
            self.main_count += 1
        elif t in {"iframe", "embed", "object", "frame"}:
            self.embedded_active_content_count += 1
        elif t in {"blink", "marquee"}:
            self.blink_marquee_count += 1

        self.inline_event_handler_attr_count += sum(
            1 for name in attrs if len(name) > 2 and name.startswith("on")
        )
        has_onclick = "onclick" in attrs
        has_keyboard_handler = any(name in attrs for name in ("onkeydown", "onkeyup", "onkeypress"))
        has_tabindex_attr = "tabindex" in attrs
        if t != "html" and "lang" in attrs:
            self.part_lang_attr_count += 1
            if not _lang_ok(attrs.get("lang")):
                self.invalid_part_lang_attr_count += 1
        if t in {"audio", "video"} and "autoplay" in attrs:
            self.autoplay_media_count += 1
        if t == "meta" and str(attrs.get("http-equiv", "")).strip().lower() == "refresh":
            self.meta_refresh_count += 1

        if "aria-label" in attrs and not attrs.get("aria-label", "").strip():
            self.empty_aria_label_count += 1
        if "tabindex" in attrs:
            self.tabindex_attr_count += 1
            raw_tabindex = str(attrs.get("tabindex") or "").strip()
            try:
                tabindex_value = int(raw_tabindex)
                if tabindex_value > 0:
                    self.positive_tabindex_count += 1
            except Exception:
                self.invalid_tabindex_count += 1
        role = attrs.get("role", "").strip().lower()
        is_native_keyboard_interactive = (
            (t == "a" and bool(attrs.get("href", "").strip()))
            or t in {"button", "select", "textarea", "summary"}
            or (t == "input" and attrs.get("type", "").strip().lower() != "hidden")
        )
        if has_onclick and not is_native_keyboard_interactive:
            self.custom_click_handler_count += 1
            if not has_keyboard_handler and not has_tabindex_attr:
                self.pointer_only_click_handler_count += 1
        if role == "region":
            aria_label = attrs.get("aria-label", "").strip()
            aria_labelledby = _idrefs(attrs.get("aria-labelledby"))
            if not aria_label and not aria_labelledby:
                self.unlabeled_region_count += 1
        if t == "label":
            self._label_depth += 1
            label_for = attrs.get("for", "").strip()
            if label_for:
                self.label_for_targets.add(label_for)

        node_id = attrs.get("id", "").strip()
        if node_id:
            self.ids[node_id] = self.ids.get(node_id, 0) + 1
            if self.ids[node_id] == 2:
                self.dup_ids.append(node_id)
        for name in ("aria-labelledby", "aria-describedby"):
            for tok in _idrefs(attrs.get(name)):
                self.idrefs.append((name, tok))
        if t == "link":
            rel = {x.strip() for x in attrs.get("rel", "").lower().split()}
            if "stylesheet" in rel:
                href = attrs.get("href", "").strip()
                if href:
                    self.css_hrefs.append(href)
        if attrs.get("data-fb-a11y-signature-status"):
            self.sig_count += 1
        if t in {"img", "svg"}:
            self.image_count += 1
            role_decorative = attrs.get("role", "").strip().lower() in {"presentation", "none"}
            aria_hidden = _trueish(attrs.get("aria-hidden"))
            explicit_decorative = _trueish(attrs.get("data-fb-a11y-decorative"))
            aria_label = attrs.get("aria-label")
            aria_labelledby = attrs.get("aria-labelledby")
            alt_present = "alt" in attrs
            alt_value = attrs.get("alt")
            title_value = attrs.get("title")
            has_informative_name = bool(
                (aria_label is not None and aria_label.strip())
                or _idrefs(aria_labelledby)
                or (alt_value is not None and alt_value.strip())
            )
            alt_empty = alt_present and (alt_value == "")
            decorative = explicit_decorative or aria_hidden or role_decorative or alt_empty
            if decorative and has_informative_name:
                self.image_semantic_conflict_count += 1
            elif not decorative and not has_informative_name:
                if title_value is not None and title_value.strip():
                    self.image_title_only_count += 1
                else:
                    self.image_missing_alt_count += 1
        if t in {"input", "select", "textarea"}:
            input_type = attrs.get("type", "").strip().lower()
            if not (t == "input" and input_type == "hidden"):
                self._form_controls.append(
                    {
                        "id": node_id,
                        "in_label": self._label_depth > 0,
                        "aria_label": attrs.get("aria-label", ""),
                        "aria_labelledby": attrs.get("aria-labelledby", ""),
                        "aria_describedby": attrs.get("aria-describedby", ""),
                        "aria_errormessage": attrs.get("aria-errormessage", ""),
                        "aria_invalid": attrs.get("aria-invalid", ""),
                    }
                )
        if t in {"h1", "h2", "h3", "h4", "h5", "h6"}:
            self._text_capture_stack.append({"tag": t, "kind": "heading", "chunks": []})
        elif t == "label":
            self._text_capture_stack.append({"tag": t, "kind": "label", "chunks": []})
        elif t == "a" and attrs.get("href", "").strip():
            self.link_count += 1
            self._text_capture_stack.append(
                {
                    "tag": t,
                    "kind": "link",
                    "chunks": [],
                    "aria_label_nonempty": bool(attrs.get("aria-label", "").strip()),
                    "aria_labelledby_nonempty": bool(_idrefs(attrs.get("aria-labelledby"))),
                }
            )
        if t == "table":
            self.table_stack.append({"has_caption": False, "th_count": 0, "th_scope_count": 0})
        elif self.table_stack:
            tbl = self.table_stack[-1]
            if t == "caption":
                tbl["has_caption"] = True
            elif t == "th":
                tbl["th_count"] += 1
                if attrs.get("scope", "").strip():
                    tbl["th_scope_count"] += 1


def parse_html_facts(html: str) -> HtmlFacts:
    p = _P()
    p.feed(html)
    p.close()
    ids = set(p.ids)
    missing = [(attr, tok) for attr, tok in p.idrefs if tok not in ids]
    body_text = " ".join(x.strip() for x in p.body_chunks if x.strip())
    unlabeled_controls = 0
    invalid_controls = 0
    unidentified_error_controls = 0
    for ctl in p._form_controls:
        if str(ctl.get("aria_label") or "").strip():
            continue
        if _idrefs(str(ctl.get("aria_labelledby") or "")):
            continue
        if ctl.get("in_label"):
            continue
        ctl_id = str(ctl.get("id") or "").strip()
        if ctl_id and ctl_id in p.label_for_targets:
            continue
        unlabeled_controls += 1
    for ctl in p._form_controls:
        if not _invalidish(str(ctl.get("aria_invalid") or "")):
            continue
        invalid_controls += 1
        describedby_ok = any(tok in ids for tok in _idrefs(str(ctl.get("aria_describedby") or "")))
        errormessage_ok = any(tok in ids for tok in _idrefs(str(ctl.get("aria_errormessage") or "")))
        if not (describedby_ok or errormessage_ok):
            unidentified_error_controls += 1
    return HtmlFacts(
        html_lang=p.html_lang,
        title="".join(p.title_chunks).strip(),
        part_lang_attr_count=p.part_lang_attr_count,
        invalid_part_lang_attr_count=p.invalid_part_lang_attr_count,
        main_count=p.main_count,
        ids=p.ids,
        dup_ids=p.dup_ids,
        missing_idrefs=missing,
        has_wrapper=p.has_html and p.has_head and p.has_body,
        has_css_link=bool(p.css_hrefs),
        css_hrefs=p.css_hrefs,
        sig_count=p.sig_count,
        empty_heading_count=p.empty_heading_count,
        empty_label_count=p.empty_label_count,
        empty_aria_label_count=p.empty_aria_label_count,
        unlabeled_region_count=p.unlabeled_region_count,
        image_count=p.image_count,
        image_missing_alt_count=p.image_missing_alt_count,
        image_title_only_count=p.image_title_only_count,
        image_semantic_conflict_count=p.image_semantic_conflict_count,
        form_control_count=len(p._form_controls),
        unlabeled_form_control_count=unlabeled_controls,
        invalid_form_control_count=invalid_controls,
        unidentified_error_form_control_count=unidentified_error_controls,
        tabindex_attr_count=p.tabindex_attr_count,
        positive_tabindex_count=p.positive_tabindex_count,
        invalid_tabindex_count=p.invalid_tabindex_count,
        link_count=p.link_count,
        unnamed_link_count=p.unnamed_link_count,
        generic_link_text_count=p.generic_link_text_count,
        custom_click_handler_count=p.custom_click_handler_count,
        pointer_only_click_handler_count=p.pointer_only_click_handler_count,
        script_element_count=p.script_element_count,
        embedded_active_content_count=p.embedded_active_content_count,
        autoplay_media_count=p.autoplay_media_count,
        blink_marquee_count=p.blink_marquee_count,
        inline_event_handler_attr_count=p.inline_event_handler_attr_count,
        meta_refresh_count=p.meta_refresh_count,
        tables=p.tables,
        body_text=body_text,
    )


def _indexes(registry: dict[str, Any]) -> tuple[dict[str, dict[str, Any]], dict[str, dict[str, Any]]]:
    return (
        {e["id"]: e for e in registry.get("entries", [])},
        {c["id"]: c for c in registry.get("pmr_categories", [])},
    )


def _profile_override_levels(registry: dict[str, Any], profile: str) -> dict[str, str]:
    out: dict[str, str] = {}
    for item in registry.get("profiles", {}).get(profile, {}).get("overrides", []):
        out[str(item["id"])] = str(item["level"])
    return out


def _gate_level(entry_id: str, entries: dict[str, dict[str, Any]], overrides: dict[str, str]) -> str:
    if entry_id in overrides:
        return overrides[entry_id]
    return str(entries.get(entry_id, {}).get("default_gate_level", "warn"))


def _gate(rows: list[dict[str, Any]], *, id_key: str, mode: str, entries: dict[str, dict[str, Any]], overrides: dict[str, str]) -> dict[str, Any]:
    mode = str(mode or "error").strip().lower()
    if mode not in {"off", "warn", "error"}:
        raise ValueError(f"Unsupported gate mode {mode!r}")
    ec = 0
    wc = 0
    failed: list[str] = []
    for row in rows:
        verdict = str(row.get("verdict") or "").lower()
        if verdict not in {"fail", "warn"}:
            continue
        rid = str(row.get(id_key) or "")
        lvl = _gate_level(rid, entries, overrides)
        if mode == "off" or lvl == "off":
            continue
        if mode == "warn":
            wc += 1
            continue
        if verdict == "warn":
            wc += 1
        elif lvl == "error":
            ec += 1
            failed.append(rid)
        else:
            wc += 1
    suffix = "rule" if id_key == "rule_id" else "audit"
    return {"ok": ec == 0, "mode": mode, "error_count": ec, "warn_count": wc, f"failed_{suffix}_ids": failed}


def _manual_debt(parity_report: dict[str, Any] | None) -> dict[str, Any]:
    review = _i((parity_report or {}).get("coverage", {}).get("review_queue_items"), 0)
    items = []
    if review > 0:
        items.append(
            {
                "id": "manual.transcription_quality.review_queue",
                "reason": f"{review} review-queue item(s) require human verification.",
                "severity": "medium",
            }
        )
    return {"item_count": review, "high_risk_count": 0, "items": items}


def _cav_note_hits(body_text: str) -> list[str]:
    text = (body_text or "").lower()
    needles = [
        "review queue",
        "parity report",
        "source analysis",
        "component validation",
        "a11y validation",
        "transcription sidecar",
        "debug log",
        "remediation note",
    ]
    return [n for n in needles if n in text]


def _sensory_characteristics_hits(body_text: str) -> list[str]:
    text = (body_text or "").lower()
    needles = [
        "see above",
        "see below",
        "shown above",
        "shown below",
        "on the left",
        "on the right",
        "left side",
        "right side",
        "top of the page",
        "bottom of the page",
        "red button",
        "green button",
        "blue button",
    ]
    return [n for n in needles if n in text]


def _focus_visible_css_seed_signals(css_text: str) -> dict[str, int]:
    css_l = (css_text or "").lower()
    return {
        "focus_selector_signal_count": css_l.count(":focus"),
        "outline_suppression_signal_count": (
            css_l.count("outline:none")
            + css_l.count("outline: none")
            + css_l.count("outline:0")
            + css_l.count("outline: 0")
            + css_l.count("outline-width:0")
            + css_l.count("outline-width: 0")
        ),
    }


def _wcag20aa_claim_readiness_scaffold(
    *,
    fail_count: int,
    wcag20aa_coverage: dict[str, Any],
    manual_review_debt_count: int,
) -> dict[str, Any]:
    machine_blocker_count = int(fail_count)
    coverage_gap_count = int(wcag20aa_coverage.get("unmapped_entry_count", 0)) + int(
        wcag20aa_coverage.get("implemented_mapped_entry_pending_count", 0)
    )
    if machine_blocker_count > 0:
        status = "blocked_machine_failures"
    elif coverage_gap_count > 0:
        status = "blocked_coverage_gaps"
    else:
        status = "manual_evidence_required"
    notes: list[str] = []
    if int(wcag20aa_coverage.get("unmapped_entry_count", 0)) > 0:
        notes.append("WCAG target registry still contains unmapped entries.")
    if int(wcag20aa_coverage.get("implemented_mapped_entry_pending_count", 0)) > 0:
        notes.append("Implemented mapped WCAG entries remain unevaluated in this report.")
    notes.append("Manual claim evidence is required for WCAG conformance assertions.")
    return {
        "target": "wcag20aa",
        "status": status,
        "claim_ready": False,
        "manual_review_required": True,
        "manual_review_debt_count": int(manual_review_debt_count),
        "machine_blocker_count": machine_blocker_count,
        "coverage_gap_count": coverage_gap_count,
        "implemented_mapped_entry_count": int(wcag20aa_coverage.get("implemented_mapped_entry_count", 0)),
        "implemented_mapped_entry_evaluated_count": int(
            wcag20aa_coverage.get("implemented_mapped_entry_evaluated_count", 0)
        ),
        "implemented_mapped_entry_pending_count": int(
            wcag20aa_coverage.get("implemented_mapped_entry_pending_count", 0)
        ),
        "unmapped_entry_count": int(wcag20aa_coverage.get("unmapped_entry_count", 0)),
        "notes": notes,
    }


def _contrast_render_seed_analysis(render_preview_png_path: str | Path) -> dict[str, Any]:
    try:
        import fullbleed  # type: ignore

        fn = getattr(fullbleed, "audit_contrast_render_png", None)
        if callable(fn):
            out = fn(str(render_preview_png_path))
            if isinstance(out, dict):
                return out
    except Exception as exc:
        return {
            "schema": "fullbleed.contrast.render_seed.v1",
            "verdict": "manual_needed",
            "confidence": "low",
            "message": f"Render-based contrast seed helper unavailable: {type(exc).__name__}: {exc}",
        }
    return {
        "schema": "fullbleed.contrast.render_seed.v1",
        "verdict": "manual_needed",
        "confidence": "low",
        "message": "Render-based contrast seed helper unavailable.",
    }


def _diag_map(code: str | None) -> str | None:
    return {
        "DOCUMENT_TITLE_MISSING": "fb.a11y.html.title_present_nonempty",
        "ID_DUPLICATE": "fb.a11y.ids.duplicate_id",
        "IDREF_MISSING": "fb.a11y.aria.reference_target_exists",
        "MAIN_MULTIPLE": "fb.a11y.structure.single_main",
        "IMAGE_ALT_MISSING": "fb.a11y.images.alt_or_decorative",
        "IMAGE_ALT_MISSING_TITLE_PRESENT": "fb.a11y.images.alt_or_decorative",
        "IMAGE_SEMANTIC_CONFLICT": "fb.a11y.images.alt_or_decorative",
        "SIGNATURE_STATUS_INVALID": "fb.a11y.signatures.text_semantics_present",
        "SIGNATURE_METHOD_INVALID": "fb.a11y.signatures.text_semantics_present",
        "HEADING_EMPTY": "fb.a11y.headings_labels.present_nonempty",
        "LABEL_EMPTY": "fb.a11y.headings_labels.present_nonempty",
        "ARIA_LABEL_EMPTY": "fb.a11y.headings_labels.present_nonempty",
        "REGION_UNLABELED": "fb.a11y.headings_labels.present_nonempty",
    }.get(str(code or "").strip())


def _vf(
    rule_id: str,
    verdict: str,
    severity: str,
    stage: str,
    source: str,
    message: str,
    *,
    evidence: list[dict[str, Any]] | None = None,
    applicability: str = "applicable",
    verification_mode: str = "machine",
    confidence: str = "certain",
) -> dict[str, Any]:
    d = {
        "rule_id": rule_id,
        "applicability": applicability,
        "verification_mode": verification_mode,
        "verdict": verdict,
        "severity": severity,
        "confidence": confidence,
        "stage": stage,
        "source": source,
        "message": message,
    }
    if evidence:
        d["evidence"] = evidence
    return d


_A11YCONTRACT_DEDUP_RULE_ALLOWLIST = {
    "fb.a11y.html.title_present_nonempty",
    "fb.a11y.structure.single_main",
    "fb.a11y.images.alt_or_decorative",
    "fb.a11y.headings_labels.present_nonempty",
}


def _verdict_rank(v: str) -> int:
    return {
        "fail": 5,
        "warn": 4,
        "manual_needed": 3,
        "pass": 2,
        "not_applicable": 1,
    }.get(str(v or "").strip(), 0)


def _severity_rank(v: str) -> int:
    return {
        "critical": 5,
        "high": 4,
        "medium": 3,
        "low": 2,
        "info": 1,
    }.get(str(v or "").strip(), 0)


def _confidence_rank(v: str) -> int:
    return {
        "certain": 4,
        "high": 3,
        "medium": 2,
        "low": 1,
    }.get(str(v or "").strip(), 0)


def _finding_ref(f: dict[str, Any], idx: int) -> str:
    return f"{f.get('rule_id','unknown')}:{f.get('stage','unknown')}:{f.get('source','unknown')}:{idx}"


def _annotated_correlation_evidence(
    f: dict[str, Any], idx: int, *, primary: bool
) -> list[dict[str, Any]]:
    evid = list(f.get("evidence") or [])
    if not evid:
        evid = [{"values": {}}]
    out: list[dict[str, Any]] = []
    for row in evid:
        item = dict(row)
        values = dict(item.get("values") or {})
        values.setdefault("correlated_origin_stage", str(f.get("stage") or ""))
        values.setdefault("correlated_origin_source", str(f.get("source") or ""))
        values.setdefault("correlated_origin_verdict", str(f.get("verdict") or ""))
        values.setdefault("correlated_primary", primary)
        item["values"] = values
        item.setdefault("diagnostic_ref", _finding_ref(f, idx))
        out.append(item)
    return out


def _count_by_key(rows: list[dict[str, Any]], key: str) -> dict[str, int]:
    out: dict[str, int] = {}
    for row in rows:
        val = str(row.get(key) or "")
        if not val:
            continue
        out[val] = out.get(val, 0) + 1
    return out


def _dedup_and_correlate_findings(
    rows: list[dict[str, Any]],
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    original = [dict(r) for r in rows]
    merge_plan: dict[int, list[int]] = {}
    skip_indexes: set[int] = set()

    by_rule: dict[str, list[int]] = {}
    for idx, row in enumerate(original):
        by_rule.setdefault(str(row.get("rule_id") or ""), []).append(idx)

    for rule_id, idxs in by_rule.items():
        if rule_id not in _A11YCONTRACT_DEDUP_RULE_ALLOWLIST or len(idxs) < 2:
            continue
        pre = [
            i
            for i in idxs
            if str(original[i].get("stage")) == "pre-render"
            and str(original[i].get("source")) == "a11y_contract"
        ]
        if not pre:
            continue
        non_pre = [i for i in idxs if i not in pre]
        if len(non_pre) != 1:
            continue
        primary_idx = non_pre[0]
        merge_plan[primary_idx] = sorted(pre)
        skip_indexes.update(pre)

    merged_rows: list[dict[str, Any]] = []
    dedup_event_count = 0
    dedup_merged_finding_count = 0
    correlation_index: list[dict[str, Any]] = []

    for idx, row in enumerate(original):
        if idx in skip_indexes:
            continue
        peer_idxs = merge_plan.get(idx, [])
        if not peer_idxs:
            merged_rows.append(row)
            continue
        dedup_event_count += 1
        dedup_merged_finding_count += len(peer_idxs)
        primary = dict(row)
        peers = [original[i] for i in peer_idxs]
        group = [row] + peers

        worst_verdict = max((str(g.get("verdict") or "") for g in group), key=_verdict_rank)
        worst_severity = max((str(g.get("severity") or "") for g in group), key=_severity_rank)
        lowest_confidence = min((str(g.get("confidence") or "") for g in group), key=_confidence_rank)

        primary["verdict"] = worst_verdict
        if _severity_rank(worst_severity) > _severity_rank(str(primary.get("severity") or "")):
            primary["severity"] = worst_severity
        if _confidence_rank(lowest_confidence) < _confidence_rank(str(primary.get("confidence") or "")):
            primary["confidence"] = lowest_confidence

        if any(str(g.get("applicability")) == "applicable" for g in group):
            primary["applicability"] = "applicable"
        elif any(str(g.get("applicability")) == "unknown" for g in group):
            primary["applicability"] = "unknown"
        else:
            primary["applicability"] = "not_applicable"

        related_ids = list(primary.get("related_ids") or [])
        related_ids.extend(_finding_ref(original[i], i) for i in peer_idxs)
        if related_ids:
            primary["related_ids"] = related_ids

        merged_evidence: list[dict[str, Any]] = []
        merged_evidence.extend(_annotated_correlation_evidence(row, idx, primary=True))
        for peer_idx in peer_idxs:
            merged_evidence.extend(
                _annotated_correlation_evidence(original[peer_idx], peer_idx, primary=False)
            )
        if merged_evidence:
            primary["evidence"] = merged_evidence

        stage_counts = _count_by_key(group, "stage")
        source_counts = _count_by_key(group, "source")
        primary["message"] = (
            f"{str(primary.get('message') or '').rstrip()} "
            f"(Correlated {len(peer_idxs)} pre-render diagnostic(s) into canonical {primary.get('stage')} finding.)"
        ).strip()
        primary.setdefault("fix_hint", "Review correlated pre-render and post-emit evidence together.")
        # Keep detailed correlation summary in a synthetic evidence row so schema changes are minimal.
        primary.setdefault("evidence", [])
        primary["evidence"].append(
            {
                "diagnostic_ref": f"correlation:{_finding_ref(row, idx)}",
                "values": {
                    "correlation_role": "summary",
                    "merged_pre_render_count": len(peer_idxs),
                    "stage_counts": stage_counts,
                    "source_counts": source_counts,
                },
            }
        )
        correlation_index.append(
            {
                "rule_id": str(primary.get("rule_id") or ""),
                "canonical_stage": str(primary.get("stage") or ""),
                "canonical_source": str(primary.get("source") or ""),
                "canonical_verdict": str(primary.get("verdict") or ""),
                "merged_finding_count": len(peer_idxs),
                "merged_pre_render_count": sum(
                    1
                    for g in group
                    if str(g.get("stage")) == "pre-render"
                    and str(g.get("source")) == "a11y_contract"
                ),
                "merged_stage_counts": stage_counts,
                "merged_source_counts": source_counts,
            }
        )
        merged_rows.append(primary)

    observability = {
        "original_finding_count": len(original),
        "reported_finding_count": len(merged_rows),
        "dedup_event_count": dedup_event_count,
        "dedup_merged_finding_count": dedup_merged_finding_count,
        "correlated_finding_count": sum(1 for r in merged_rows if r.get("related_ids")),
        "stage_counts": _count_by_key(merged_rows, "stage"),
        "source_counts": _count_by_key(merged_rows, "source"),
        "original_stage_counts": _count_by_key(original, "stage"),
        "original_source_counts": _count_by_key(original, "source"),
        "correlation_index": correlation_index,
    }
    return merged_rows, observability


def _claim_bool(claim_evidence: dict[str, Any] | None, *path: str) -> bool:
    cur: Any = claim_evidence
    for part in path:
        if not isinstance(cur, dict):
            return False
        cur = cur.get(part)
    return cur if isinstance(cur, bool) else False


def prototype_verify_accessibility(
    *,
    html_path: str | Path,
    css_path: str | Path,
    profile: str = "strict",
    mode: str = "error",
    a11y_report: dict[str, Any] | None = None,
    parity_report: dict[str, Any] | None = None,
    expected_lang: str | None = None,
    expected_title: str | None = None,
    render_preview_png_path: str | Path | None = None,
    claim_evidence: dict[str, Any] | None = None,
    registry: dict[str, Any] | None = None,
    generated_at: str | None = None,
    fullbleed_version: str = "0.6.0",
) -> dict[str, Any]:
    reg = registry or _registry()
    entries, _cats = _indexes(reg)
    overrides = _profile_override_levels(reg, profile)
    html_p = Path(html_path)
    css_p = Path(css_path)
    facts = parse_html_facts(html_p.read_text(encoding="utf-8"))
    css_text = css_p.read_text(encoding="utf-8")
    findings: list[dict[str, Any]] = []

    lang_pass = _lang_ok(facts.html_lang) and (expected_lang is None or facts.html_lang == expected_lang)
    findings.append(
        _vf(
            "fb.a11y.html.lang_present_valid",
            "pass" if lang_pass else "fail",
            "high",
            "post-emit",
            "fullbleed",
            "HTML lang attribute is present and valid." if lang_pass else "HTML lang missing/invalid or metadata mismatch.",
            evidence=[{"selector": "html", "values": {"lang": facts.html_lang or ""}}],
        )
    )
    if facts.part_lang_attr_count == 0:
        findings.append(
            _vf(
                "fb.a11y.language.parts_declared_valid_seed",
                "not_applicable",
                "low",
                "post-emit",
                "fullbleed",
                "No inline lang declarations on descendant elements detected; language-of-parts rule not applicable for this document.",
                evidence=[
                    {
                        "values": {
                            "part_lang_attr_count": facts.part_lang_attr_count,
                            "invalid_part_lang_attr_count": facts.invalid_part_lang_attr_count,
                        }
                    }
                ],
                applicability="not_applicable",
                verification_mode="hybrid",
                confidence="high",
            )
        )
    else:
        part_lang_fail = facts.invalid_part_lang_attr_count > 0
        findings.append(
            _vf(
                "fb.a11y.language.parts_declared_valid_seed",
                "fail" if part_lang_fail else "pass",
                "medium" if part_lang_fail else "low",
                "post-emit",
                "fullbleed",
                (
                    "Invalid or empty inline language-of-parts declarations detected."
                    if part_lang_fail
                    else "Inline language-of-parts declarations are syntactically valid."
                ),
                evidence=[
                    {
                        "values": {
                            "part_lang_attr_count": facts.part_lang_attr_count,
                            "invalid_part_lang_attr_count": facts.invalid_part_lang_attr_count,
                        }
                    }
                ],
                verification_mode="hybrid",
                confidence="high",
            )
        )
    title_pass = bool(facts.title.strip()) and (expected_title is None or facts.title == expected_title)
    findings.append(
        _vf(
            "fb.a11y.html.title_present_nonempty",
            "pass" if title_pass else "fail",
            "high",
            "post-emit",
            "fullbleed",
            "Document title is present and non-empty." if title_pass else "Document title missing/empty or metadata mismatch.",
            evidence=[{"selector": "head > title", "values": {"title": facts.title}}],
        )
    )
    main_pass = facts.main_count == 1
    findings.append(
        _vf(
            "fb.a11y.structure.single_main",
            "pass" if main_pass else "fail",
            "medium",
            "post-emit",
            "fullbleed",
            "Single primary content root detected." if main_pass else f"Expected exactly one <main>; found {facts.main_count}.",
            evidence=[{"selector": "main", "values": {"count": facts.main_count}}],
        )
    )
    hl_fail = (
        facts.empty_heading_count + facts.empty_label_count + facts.empty_aria_label_count
    ) > 0
    hl_warn = facts.unlabeled_region_count > 0
    findings.append(
        _vf(
            "fb.a11y.headings_labels.present_nonempty",
            "fail" if hl_fail else ("warn" if hl_warn else "pass"),
            "high" if hl_fail else "medium",
            "post-emit",
            "fullbleed",
            (
                "Empty heading/label naming signals detected."
                if hl_fail
                else (
                    "Headings/labels are non-empty, but some region landmarks are unlabeled."
                    if hl_warn
                    else "No empty headings/labels or unlabeled regions detected."
                )
            ),
            evidence=[
                {
                    "values": {
                        "empty_heading_count": facts.empty_heading_count,
                        "empty_label_count": facts.empty_label_count,
                        "empty_aria_label_count": facts.empty_aria_label_count,
                        "unlabeled_region_count": facts.unlabeled_region_count,
                    }
                }
            ],
            verification_mode="hybrid",
            confidence="high" if (hl_fail or hl_warn) else "medium",
        )
    )
    if facts.image_count == 0:
        findings.append(
            _vf(
                "fb.a11y.images.alt_or_decorative",
                "not_applicable",
                "low",
                "post-emit",
                "fullbleed",
                "No img/svg elements detected; non-text-content image rule not applicable.",
                applicability="not_applicable",
                verification_mode="machine",
                confidence="high",
            )
        )
    else:
        img_fail = facts.image_missing_alt_count > 0 or facts.image_semantic_conflict_count > 0
        img_warn = facts.image_title_only_count > 0
        findings.append(
            _vf(
                "fb.a11y.images.alt_or_decorative",
                "fail" if img_fail else ("warn" if img_warn else "pass"),
                "high" if img_fail else "medium",
                "post-emit",
                "fullbleed",
                (
                    "Image text alternative errors detected."
                    if img_fail
                    else (
                        "Some images rely on title without alt/ARIA text alternatives."
                        if img_warn
                        else "Image text alternatives/decorative semantics look consistent."
                    )
                ),
                evidence=[
                    {
                        "values": {
                            "image_count": facts.image_count,
                            "image_missing_alt_count": facts.image_missing_alt_count,
                            "image_title_only_count": facts.image_title_only_count,
                            "image_semantic_conflict_count": facts.image_semantic_conflict_count,
                        }
                    }
                ],
                verification_mode="machine",
                confidence="high",
            )
        )
    if facts.form_control_count == 0:
        findings.append(
            _vf(
                "fb.a11y.forms.labels_or_instructions_present",
                "not_applicable",
                "low",
                "post-emit",
                "fullbleed",
                "No form controls detected; labels/instructions rule not applicable.",
                applicability="not_applicable",
                verification_mode="hybrid",
                confidence="high",
            )
        )
    else:
        ctrl_fail = facts.unlabeled_form_control_count > 0
        findings.append(
            _vf(
                "fb.a11y.forms.labels_or_instructions_present",
                "fail" if ctrl_fail else "pass",
                "high" if ctrl_fail else "medium",
                "post-emit",
                "fullbleed",
                (
                    "Unlabeled form controls detected."
                    if ctrl_fail
                    else "Detected form controls have label/ARIA naming signals."
                ),
                evidence=[
                    {
                        "values": {
                            "form_control_count": facts.form_control_count,
                            "unlabeled_form_control_count": facts.unlabeled_form_control_count,
                        }
                    }
                ],
                verification_mode="hybrid",
                confidence="medium",
            )
        )
    if facts.invalid_form_control_count == 0:
        findings.append(
            _vf(
                "fb.a11y.forms.error_identification_present",
                "not_applicable",
                "low",
                "post-emit",
                "fullbleed",
                "No invalid form controls detected; error-identification rule not applicable.",
                applicability="not_applicable",
                verification_mode="hybrid",
                confidence="high",
            )
        )
    else:
        err_fail = facts.unidentified_error_form_control_count > 0
        findings.append(
            _vf(
                "fb.a11y.forms.error_identification_present",
                "fail" if err_fail else "pass",
                "high" if err_fail else "medium",
                "post-emit",
                "fullbleed",
                (
                    "Invalid form controls without associated error-identification text detected."
                    if err_fail
                    else "Invalid form controls expose associated error-identification text signals."
                ),
                evidence=[
                    {
                        "values": {
                            "invalid_form_control_count": facts.invalid_form_control_count,
                            "unidentified_error_form_control_count": facts.unidentified_error_form_control_count,
                        }
                    }
                ],
                verification_mode="hybrid",
                confidence="medium",
            )
        )
    focus_order_target_count = facts.link_count + facts.form_control_count
    if focus_order_target_count == 0:
        findings.append(
            _vf(
                "fb.a11y.focus.order_seed",
                "not_applicable",
                "medium",
                "post-emit",
                "fullbleed",
                "No interactive links or form controls detected; focus-order seed not applicable.",
                evidence=[
                    {
                        "values": {
                            "interactive_focus_target_count": focus_order_target_count,
                            "link_count": facts.link_count,
                            "form_control_count": facts.form_control_count,
                            "tabindex_attr_count": facts.tabindex_attr_count,
                            "positive_tabindex_count": facts.positive_tabindex_count,
                            "invalid_tabindex_count": facts.invalid_tabindex_count,
                        }
                    }
                ],
                applicability="not_applicable",
                verification_mode="hybrid",
                confidence="high",
            )
        )
    else:
        focus_order_warn = (
            facts.positive_tabindex_count > 0 or facts.invalid_tabindex_count > 0
        )
        findings.append(
            _vf(
                "fb.a11y.focus.order_seed",
                "warn" if focus_order_warn else "pass",
                "medium",
                "post-emit",
                "fullbleed",
                (
                    "Positive tabindex values detected; focus order may diverge from DOM order and requires manual review."
                    if facts.positive_tabindex_count > 0
                    else (
                        "Invalid tabindex values detected; focus order behavior may be inconsistent and requires manual review."
                        if facts.invalid_tabindex_count > 0
                        else "No positive/invalid tabindex focus-order override signals detected for interactive content."
                    )
                ),
                evidence=[
                    {
                        "values": {
                            "interactive_focus_target_count": focus_order_target_count,
                            "link_count": facts.link_count,
                            "form_control_count": facts.form_control_count,
                            "tabindex_attr_count": facts.tabindex_attr_count,
                            "positive_tabindex_count": facts.positive_tabindex_count,
                            "invalid_tabindex_count": facts.invalid_tabindex_count,
                        }
                    }
                ],
                verification_mode="hybrid",
                confidence="medium",
            )
        )
    if facts.link_count == 0:
        findings.append(
            _vf(
                "fb.a11y.links.purpose_in_context",
                "not_applicable",
                "low",
                "post-emit",
                "fullbleed",
                "No links detected; link-purpose rule not applicable.",
                applicability="not_applicable",
                verification_mode="hybrid",
                confidence="high",
            )
        )
    else:
        link_fail = facts.unnamed_link_count > 0
        link_warn = facts.generic_link_text_count > 0
        findings.append(
            _vf(
                "fb.a11y.links.purpose_in_context",
                "fail" if link_fail else ("warn" if link_warn else "pass"),
                "high" if link_fail else "medium",
                "post-emit",
                "fullbleed",
                (
                    "Links without discernible text purpose signals detected."
                    if link_fail
                    else (
                        "Generic link text detected; contextual purpose may require manual review."
                        if link_warn
                        else "Detected links have discernible text purpose signals."
                    )
                ),
                evidence=[
                    {
                        "values": {
                            "link_count": facts.link_count,
                            "unnamed_link_count": facts.unnamed_link_count,
                            "generic_link_text_count": facts.generic_link_text_count,
                        }
                    }
                ],
                verification_mode="hybrid",
                confidence="medium",
            )
        )
    sensory_hits = _sensory_characteristics_hits(facts.body_text)
    findings.append(
        _vf(
            "fb.a11y.instructions.sensory_characteristics_seed",
            "pass" if not sensory_hits else "warn",
            "medium",
            "post-emit",
            "fullbleed",
            (
                "No obvious sensory-characteristics instruction phrases detected."
                if not sensory_hits
                else "Potential sensory-characteristics instruction phrases detected; manual review required."
            ),
            evidence=[
                {
                    "values": {
                        "sensory_phrase_hit_count": len(sensory_hits),
                        "sensory_phrase_hits": "|".join(sensory_hits),
                    }
                }
            ],
            verification_mode="hybrid",
            confidence="high" if not sensory_hits else "medium",
        )
    )
    focus_css = _focus_visible_css_seed_signals(css_text)
    interactive_focus_target_count = facts.link_count + facts.form_control_count
    has_focus_selector_signal = int(focus_css["focus_selector_signal_count"]) > 0
    has_outline_suppression_signal = int(focus_css["outline_suppression_signal_count"]) > 0
    findings.append(
        _vf(
            "fb.a11y.focus.visible_seed",
            (
                "not_applicable"
                if interactive_focus_target_count == 0
                else (
                    "pass"
                    if has_focus_selector_signal
                    else ("warn" if has_outline_suppression_signal else "manual_needed")
                )
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "No interactive links or form controls detected; focus-visible seed not applicable."
                if interactive_focus_target_count == 0
                else (
                    "Focus-style selector signals detected in CSS for interactive content."
                    if has_focus_selector_signal
                    else (
                        "Outline suppression signals detected without focus-style selector signals; focus visibility may be reduced."
                        if has_outline_suppression_signal
                        else "Interactive content detected but no explicit focus-style CSS signals found; manual review required."
                    )
                )
            ),
            evidence=[
                {
                    "values": {
                        "interactive_focus_target_count": interactive_focus_target_count,
                        "link_count": facts.link_count,
                        "form_control_count": facts.form_control_count,
                        "focus_selector_signal_count": int(
                            focus_css["focus_selector_signal_count"]
                        ),
                        "outline_suppression_signal_count": int(
                            focus_css["outline_suppression_signal_count"]
                        ),
                    }
                }
            ],
            applicability="not_applicable"
            if interactive_focus_target_count == 0
            else "applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if interactive_focus_target_count == 0
                else ("medium" if (has_focus_selector_signal or has_outline_suppression_signal) else "low")
            ),
        )
    )
    if facts.dup_ids:
        for dup in facts.dup_ids:
            findings.append(_vf("fb.a11y.ids.duplicate_id", "fail", "critical", "post-emit", "fullbleed", f"Duplicate id {dup!r} detected.", evidence=[{"values": {"id": dup}}]))
    else:
        findings.append(_vf("fb.a11y.ids.duplicate_id", "pass", "critical", "post-emit", "fullbleed", "No duplicate IDs detected."))
    if facts.missing_idrefs:
        for attr, target in facts.missing_idrefs:
            findings.append(_vf("fb.a11y.aria.reference_target_exists", "fail", "critical", "post-emit", "fullbleed", f"{attr} references missing id {target!r}.", evidence=[{"values": {"attr": attr, "target_id": target}}]))
    else:
        findings.append(_vf("fb.a11y.aria.reference_target_exists", "pass", "critical", "post-emit", "fullbleed", "No broken ARIA ID references detected."))

    for diag in (a11y_report or {}).get("diagnostics", []) or []:
        rid = _diag_map(diag.get("code"))
        if not rid:
            continue
        err = str(diag.get("severity")) == "error"
        sev = "critical" if err and rid in {"fb.a11y.ids.duplicate_id", "fb.a11y.aria.reference_target_exists"} else ("high" if err else "medium")
        findings.append(
            _vf(
                rid,
                "fail" if err else "warn",
                sev,
                "pre-render",
                "a11y_contract",
                str(diag.get("message") or diag.get("code") or "A11y diagnostic"),
                evidence=[{"dom_path": str(diag.get("path") or ""), "values": {"code": diag.get("code")}}],
                confidence="high",
            )
        )

    sig_pass = facts.sig_count > 0
    findings.append(
        _vf(
            "fb.a11y.signatures.text_semantics_present",
            "pass" if sig_pass else ("manual_needed" if profile in {"cav", "transactional"} else "not_applicable"),
            "medium",
            "post-emit",
            "fullbleed",
            "Signature fields include text-first semantics." if sig_pass else "Signature semantics could not be confirmed automatically.",
            evidence=[{"values": {"signature_semantic_count": facts.sig_count}}],
            verification_mode="machine" if sig_pass else ("manual" if profile in {"cav", "transactional"} else "machine"),
            applicability="applicable" if profile in {"cav", "transactional"} else "not_applicable",
            confidence="high" if sig_pass else "low",
        )
    )
    non_interference_signal_count = (
        facts.script_element_count
        + facts.embedded_active_content_count
        + facts.autoplay_media_count
        + facts.blink_marquee_count
        + facts.inline_event_handler_attr_count
        + facts.meta_refresh_count
    )
    findings.append(
        _vf(
            "fb.a11y.claim.non_interference_seed",
            "pass" if non_interference_signal_count == 0 else "warn",
            "medium",
            "adapter",
            "adapter",
            (
                "No obvious active-content non-interference risk signals detected in emitted HTML."
                if non_interference_signal_count == 0
                else "Potential non-interference risk signals detected; manual review required."
            ),
            evidence=[
                {
                    "values": {
                        "script_element_count": facts.script_element_count,
                        "embedded_active_content_count": facts.embedded_active_content_count,
                        "autoplay_media_count": facts.autoplay_media_count,
                        "blink_marquee_count": facts.blink_marquee_count,
                        "inline_event_handler_attr_count": facts.inline_event_handler_attr_count,
                        "meta_refresh_count": facts.meta_refresh_count,
                    }
                }
            ],
            verification_mode="hybrid",
            confidence="high" if non_interference_signal_count == 0 else "medium",
        )
    )
    complete_processes_applicable = profile == "transactional"
    findings.append(
        _vf(
            "fb.a11y.claim.complete_processes_scope_seed",
            "manual_needed" if complete_processes_applicable else "not_applicable",
            "medium",
            "adapter",
            "adapter",
            (
                "Transactional/profile process conformance requires complete-process scope evidence; manual review required."
                if complete_processes_applicable
                else "Complete-processes conformance requirement not applicable without a declared multi-step process scope."
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "process_scope_declared": False,
                    }
                }
            ],
            verification_mode="hybrid",
            applicability="applicable" if complete_processes_applicable else "not_applicable",
            confidence="medium" if complete_processes_applicable else "high",
        )
    )
    keyboard_target_count = facts.link_count + facts.form_control_count
    keyboard_custom_click_target_count = facts.custom_click_handler_count
    keyboard_pointer_only_signal_count = facts.pointer_only_click_handler_count
    keyboard_applicable = keyboard_target_count > 0 or keyboard_custom_click_target_count > 0
    keyboard_assessed = _claim_bool(claim_evidence, "wcag20", "keyboard_assessed")
    keyboard_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "keyboard_basis_recorded"
    )
    keyboard_claim_evidence_satisfied = keyboard_assessed and keyboard_basis_recorded
    findings.append(
        _vf(
            "fb.a11y.keyboard.operable_seed",
            (
                "not_applicable"
                if not keyboard_applicable
                else (
                    (
                        "warn"
                        if keyboard_claim_evidence_satisfied
                        else "manual_needed"
                    )
                    if keyboard_pointer_only_signal_count > 0
                    else ("pass" if keyboard_claim_evidence_satisfied else "manual_needed")
                )
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "No interactive links, form controls, or custom click-handlers detected; keyboard-operable seed not applicable."
                if not keyboard_applicable
                else (
                    (
                        "Custom click-handlers without keyboard handlers/tabindex were detected on non-native elements; keyboard review evidence is recorded but manual follow-up remains required."
                        if keyboard_claim_evidence_satisfied
                        else "Custom click-handlers without keyboard handlers/tabindex were detected on non-native elements; keyboard-operability review requires manual evidence."
                    )
                    if keyboard_pointer_only_signal_count > 0
                    else (
                        "Keyboard-operability review evidence is recorded for interactive components."
                        if keyboard_claim_evidence_satisfied
                        else "Interactive components detected; keyboard-operability review requires manual evidence."
                    )
                )
            ),
            evidence=[
                {
                    "values": {
                        "interactive_keyboard_target_count": keyboard_target_count,
                        "custom_click_handler_count": keyboard_custom_click_target_count,
                        "pointer_only_click_handler_count": keyboard_pointer_only_signal_count,
                        "link_count": facts.link_count,
                        "form_control_count": facts.form_control_count,
                        "keyboard_assessed": keyboard_assessed,
                        "keyboard_basis_recorded": keyboard_basis_recorded,
                    }
                }
            ],
            applicability="not_applicable" if not keyboard_applicable else "applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not keyboard_applicable
                else (
                    ("medium" if keyboard_claim_evidence_satisfied else "low")
                    if keyboard_pointer_only_signal_count > 0
                    else ("medium" if keyboard_claim_evidence_satisfied else "low")
                )
            ),
        )
    )
    keyboard_trap_assessed = _claim_bool(claim_evidence, "wcag20", "keyboard_trap_assessed")
    keyboard_trap_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "keyboard_trap_basis_recorded"
    )
    keyboard_trap_claim_evidence_satisfied = (
        keyboard_trap_assessed and keyboard_trap_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.keyboard.no_trap_seed",
            (
                "not_applicable"
                if keyboard_target_count == 0
                else ("pass" if keyboard_trap_claim_evidence_satisfied else "manual_needed")
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "No interactive links or form controls detected; no-keyboard-trap seed not applicable."
                if keyboard_target_count == 0
                else (
                    "No-keyboard-trap review evidence is recorded for interactive components."
                    if keyboard_trap_claim_evidence_satisfied
                    else "Interactive components detected; no-keyboard-trap review requires manual evidence."
                )
            ),
            evidence=[
                {
                    "values": {
                        "interactive_keyboard_target_count": keyboard_target_count,
                        "link_count": facts.link_count,
                        "form_control_count": facts.form_control_count,
                        "keyboard_trap_assessed": keyboard_trap_assessed,
                        "keyboard_trap_basis_recorded": keyboard_trap_basis_recorded,
                    }
                }
            ],
            applicability="not_applicable" if keyboard_target_count == 0 else "applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if keyboard_target_count == 0
                else ("medium" if keyboard_trap_claim_evidence_satisfied else "low")
            ),
        )
    )
    error_suggestion_scope_declared = _claim_bool(
        claim_evidence, "wcag20", "error_suggestion_scope_declared"
    )
    error_suggestion_assessed = _claim_bool(
        claim_evidence, "wcag20", "error_suggestion_assessed"
    )
    error_suggestion_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "error_suggestion_basis_recorded"
    )
    error_suggestion_claim_evidence_satisfied = (
        error_suggestion_assessed and error_suggestion_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.forms.error_suggestion_seed",
            (
                "not_applicable"
                if not error_suggestion_scope_declared
                else (
                    "pass"
                    if error_suggestion_claim_evidence_satisfied
                    else "manual_needed"
                )
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Error-suggestion criterion not applicable without a declared form-flow error-handling scope."
                if not error_suggestion_scope_declared
                else (
                    "Error-suggestion review evidence is recorded for the declared form-flow scope."
                    if error_suggestion_claim_evidence_satisfied
                    else "Error-suggestion criterion is in scope for the declared form-flow; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "error_suggestion_scope_declared": error_suggestion_scope_declared,
                        "error_suggestion_assessed": error_suggestion_assessed,
                        "error_suggestion_basis_recorded": error_suggestion_basis_recorded,
                    }
                }
            ],
            applicability="applicable"
            if error_suggestion_scope_declared
            else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not error_suggestion_scope_declared
                else ("medium" if error_suggestion_claim_evidence_satisfied else "low")
            ),
        )
    )
    error_prevention_scope_declared = _claim_bool(
        claim_evidence, "wcag20", "error_prevention_scope_declared"
    )
    error_prevention_assessed = _claim_bool(
        claim_evidence, "wcag20", "error_prevention_assessed"
    )
    error_prevention_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "error_prevention_basis_recorded"
    )
    error_prevention_claim_evidence_satisfied = (
        error_prevention_assessed and error_prevention_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.forms.error_prevention_legal_financial_data_seed",
            (
                "not_applicable"
                if not error_prevention_scope_declared
                else (
                    "pass"
                    if error_prevention_claim_evidence_satisfied
                    else "manual_needed"
                )
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Error-prevention (legal/financial/data) criterion not applicable without a declared transactional/legal data form-flow scope."
                if not error_prevention_scope_declared
                else (
                    "Error-prevention (legal/financial/data) review evidence is recorded for the declared transactional/legal data form-flow scope."
                    if error_prevention_claim_evidence_satisfied
                    else "Error-prevention (legal/financial/data) criterion is in scope for the declared form-flow; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "error_prevention_scope_declared": error_prevention_scope_declared,
                        "error_prevention_assessed": error_prevention_assessed,
                        "error_prevention_basis_recorded": error_prevention_basis_recorded,
                    }
                }
            ],
            applicability="applicable"
            if error_prevention_scope_declared
            else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not error_prevention_scope_declared
                else ("medium" if error_prevention_claim_evidence_satisfied else "low")
            ),
        )
    )
    on_input_assessed = _claim_bool(claim_evidence, "wcag20", "on_input_assessed")
    on_input_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "on_input_basis_recorded"
    )
    on_input_claim_evidence_satisfied = on_input_assessed and on_input_basis_recorded
    on_input_target_count = facts.form_control_count
    findings.append(
        _vf(
            "fb.a11y.forms.on_input_behavior_seed",
            (
                "not_applicable"
                if on_input_target_count == 0
                else ("pass" if on_input_claim_evidence_satisfied else "manual_needed")
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "No form controls detected; on-input behavior seed not applicable."
                if on_input_target_count == 0
                else (
                    "On-input behavior review evidence is recorded for detected form controls."
                    if on_input_claim_evidence_satisfied
                    else "Form controls detected; on-input behavior review requires manual evidence."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "form_control_count": facts.form_control_count,
                        "on_input_assessed": on_input_assessed,
                        "on_input_basis_recorded": on_input_basis_recorded,
                    }
                }
            ],
            applicability="not_applicable" if on_input_target_count == 0 else "applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if on_input_target_count == 0
                else ("medium" if on_input_claim_evidence_satisfied else "low")
            ),
        )
    )
    on_focus_assessed = _claim_bool(claim_evidence, "wcag20", "on_focus_assessed")
    on_focus_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "on_focus_basis_recorded"
    )
    on_focus_claim_evidence_satisfied = on_focus_assessed and on_focus_basis_recorded
    on_focus_target_count = keyboard_target_count
    findings.append(
        _vf(
            "fb.a11y.focus.on_focus_behavior_seed",
            (
                "not_applicable"
                if on_focus_target_count == 0
                else ("pass" if on_focus_claim_evidence_satisfied else "manual_needed")
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "No interactive links or form controls detected; on-focus behavior seed not applicable."
                if on_focus_target_count == 0
                else (
                    "On-focus behavior review evidence is recorded for interactive components."
                    if on_focus_claim_evidence_satisfied
                    else "Interactive components detected; on-focus behavior review requires manual evidence."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "interactive_focus_target_count": on_focus_target_count,
                        "link_count": facts.link_count,
                        "form_control_count": facts.form_control_count,
                        "on_focus_assessed": on_focus_assessed,
                        "on_focus_basis_recorded": on_focus_basis_recorded,
                    }
                }
            ],
            applicability="not_applicable" if on_focus_target_count == 0 else "applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if on_focus_target_count == 0
                else ("medium" if on_focus_claim_evidence_satisfied else "low")
            ),
        )
    )
    timing_adjustable_scope_declared = _claim_bool(
        claim_evidence, "wcag20", "timing_adjustable_scope_declared"
    )
    timing_adjustable_assessed = _claim_bool(
        claim_evidence, "wcag20", "timing_adjustable_assessed"
    )
    timing_adjustable_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "timing_adjustable_basis_recorded"
    )
    timing_adjustable_claim_evidence_satisfied = (
        timing_adjustable_assessed and timing_adjustable_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.timing.adjustable_seed",
            (
                "not_applicable"
                if not timing_adjustable_scope_declared
                else (
                    "pass"
                    if timing_adjustable_claim_evidence_satisfied
                    else "manual_needed"
                )
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Timing-adjustable criterion not applicable without a declared timed-interaction scope."
                if not timing_adjustable_scope_declared
                else (
                    "Timing-adjustable review evidence is recorded for the declared timed-interaction scope."
                    if timing_adjustable_claim_evidence_satisfied
                    else "Timing-adjustable criterion is in scope for the declared timed-interaction flow; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "timing_adjustable_scope_declared": timing_adjustable_scope_declared,
                        "timing_adjustable_assessed": timing_adjustable_assessed,
                        "timing_adjustable_basis_recorded": timing_adjustable_basis_recorded,
                        "meta_refresh_count": facts.meta_refresh_count,
                    }
                }
            ],
            applicability="applicable"
            if timing_adjustable_scope_declared
            else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not timing_adjustable_scope_declared
                else ("medium" if timing_adjustable_claim_evidence_satisfied else "low")
            ),
        )
    )
    pause_stop_hide_scope_declared = _claim_bool(
        claim_evidence, "wcag20", "pause_stop_hide_scope_declared"
    )
    pause_stop_hide_assessed = _claim_bool(
        claim_evidence, "wcag20", "pause_stop_hide_assessed"
    )
    pause_stop_hide_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "pause_stop_hide_basis_recorded"
    )
    pause_stop_hide_claim_evidence_satisfied = (
        pause_stop_hide_assessed and pause_stop_hide_basis_recorded
    )
    pause_stop_hide_signal_count = facts.autoplay_media_count + facts.blink_marquee_count
    findings.append(
        _vf(
            "fb.a11y.timing.pause_stop_hide_seed",
            (
                "not_applicable"
                if not pause_stop_hide_scope_declared
                else (
                    "pass"
                    if pause_stop_hide_claim_evidence_satisfied
                    else "manual_needed"
                )
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Pause/stop/hide criterion not applicable without a declared moving/blinking/updating content scope."
                if not pause_stop_hide_scope_declared
                else (
                    "Pause/stop/hide review evidence is recorded for the declared moving/blinking/updating content scope."
                    if pause_stop_hide_claim_evidence_satisfied
                    else "Pause/stop/hide criterion is in scope for declared moving/blinking/updating content; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "pause_stop_hide_scope_declared": pause_stop_hide_scope_declared,
                        "pause_stop_hide_assessed": pause_stop_hide_assessed,
                        "pause_stop_hide_basis_recorded": pause_stop_hide_basis_recorded,
                        "autoplay_media_count": facts.autoplay_media_count,
                        "blink_marquee_count": facts.blink_marquee_count,
                        "pause_stop_hide_signal_count": pause_stop_hide_signal_count,
                    }
                }
            ],
            applicability="applicable"
            if pause_stop_hide_scope_declared
            else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not pause_stop_hide_scope_declared
                else ("medium" if pause_stop_hide_claim_evidence_satisfied else "low")
            ),
        )
    )
    three_flashes_scope_declared = _claim_bool(
        claim_evidence, "wcag20", "three_flashes_scope_declared"
    )
    three_flashes_assessed = _claim_bool(
        claim_evidence, "wcag20", "three_flashes_assessed"
    )
    three_flashes_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "three_flashes_basis_recorded"
    )
    three_flashes_claim_evidence_satisfied = (
        three_flashes_assessed and three_flashes_basis_recorded
    )
    flash_signal_count = facts.autoplay_media_count + facts.blink_marquee_count
    findings.append(
        _vf(
            "fb.a11y.seizures.three_flashes_seed",
            (
                "not_applicable"
                if not three_flashes_scope_declared
                else ("pass" if three_flashes_claim_evidence_satisfied else "manual_needed")
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Three-flashes criterion not applicable without a declared flashing-content scope."
                if not three_flashes_scope_declared
                else (
                    "Three-flashes review evidence is recorded for the declared flashing-content scope."
                    if three_flashes_claim_evidence_satisfied
                    else "Three-flashes criterion is in scope for declared flashing content; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "three_flashes_scope_declared": three_flashes_scope_declared,
                        "three_flashes_assessed": three_flashes_assessed,
                        "three_flashes_basis_recorded": three_flashes_basis_recorded,
                        "autoplay_media_count": facts.autoplay_media_count,
                        "blink_marquee_count": facts.blink_marquee_count,
                        "flash_signal_count": flash_signal_count,
                    }
                }
            ],
            applicability="applicable"
            if three_flashes_scope_declared
            else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not three_flashes_scope_declared
                else ("medium" if three_flashes_claim_evidence_satisfied else "low")
            ),
        )
    )
    audio_control_scope_declared = _claim_bool(
        claim_evidence, "wcag20", "audio_control_scope_declared"
    )
    audio_control_assessed = _claim_bool(
        claim_evidence, "wcag20", "audio_control_assessed"
    )
    audio_control_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "audio_control_basis_recorded"
    )
    audio_control_claim_evidence_satisfied = (
        audio_control_assessed and audio_control_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.audio.control_seed",
            (
                "not_applicable"
                if not audio_control_scope_declared
                else ("pass" if audio_control_claim_evidence_satisfied else "manual_needed")
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Audio-control criterion not applicable without a declared autoplay/audio playback scope."
                if not audio_control_scope_declared
                else (
                    "Audio-control review evidence is recorded for the declared autoplay/audio playback scope."
                    if audio_control_claim_evidence_satisfied
                    else "Audio-control criterion is in scope for declared autoplay/audio playback content; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "audio_control_scope_declared": audio_control_scope_declared,
                        "audio_control_assessed": audio_control_assessed,
                        "audio_control_basis_recorded": audio_control_basis_recorded,
                        "autoplay_media_count": facts.autoplay_media_count,
                    }
                }
            ],
            applicability="applicable"
            if audio_control_scope_declared
            else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not audio_control_scope_declared
                else ("medium" if audio_control_claim_evidence_satisfied else "low")
            ),
        )
    )
    use_of_color_scope_declared = _claim_bool(
        claim_evidence, "wcag20", "use_of_color_scope_declared"
    )
    use_of_color_assessed = _claim_bool(
        claim_evidence, "wcag20", "use_of_color_assessed"
    )
    use_of_color_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "use_of_color_basis_recorded"
    )
    use_of_color_claim_evidence_satisfied = (
        use_of_color_assessed and use_of_color_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.color.use_of_color_seed",
            (
                "not_applicable"
                if not use_of_color_scope_declared
                else ("pass" if use_of_color_claim_evidence_satisfied else "manual_needed")
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Use-of-color criterion not applicable without a declared color-only meaning scope."
                if not use_of_color_scope_declared
                else (
                    "Use-of-color review evidence is recorded for the declared color-only meaning scope."
                    if use_of_color_claim_evidence_satisfied
                    else "Use-of-color criterion is in scope for declared color-only meaning content; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "use_of_color_scope_declared": use_of_color_scope_declared,
                        "use_of_color_assessed": use_of_color_assessed,
                        "use_of_color_basis_recorded": use_of_color_basis_recorded,
                    }
                }
            ],
            applicability="applicable"
            if use_of_color_scope_declared
            else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not use_of_color_scope_declared
                else ("medium" if use_of_color_claim_evidence_satisfied else "low")
            ),
        )
    )
    resize_text_scope_declared = _claim_bool(
        claim_evidence, "wcag20", "resize_text_scope_declared"
    )
    resize_text_assessed = _claim_bool(
        claim_evidence, "wcag20", "resize_text_assessed"
    )
    resize_text_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "resize_text_basis_recorded"
    )
    resize_text_claim_evidence_satisfied = (
        resize_text_assessed and resize_text_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.text.resize_seed",
            (
                "not_applicable"
                if not resize_text_scope_declared
                else ("pass" if resize_text_claim_evidence_satisfied else "manual_needed")
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Resize-text criterion not applicable without a declared text-resize review scope."
                if not resize_text_scope_declared
                else (
                    "Resize-text review evidence is recorded for the declared text-resize scope."
                    if resize_text_claim_evidence_satisfied
                    else "Resize-text criterion is in scope for declared text content; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "resize_text_scope_declared": resize_text_scope_declared,
                        "resize_text_assessed": resize_text_assessed,
                        "resize_text_basis_recorded": resize_text_basis_recorded,
                        "link_count": facts.link_count,
                        "form_control_count": facts.form_control_count,
                    }
                }
            ],
            applicability="applicable"
            if resize_text_scope_declared
            else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not resize_text_scope_declared
                else ("medium" if resize_text_claim_evidence_satisfied else "low")
            ),
        )
    )
    images_of_text_scope_declared = _claim_bool(
        claim_evidence, "wcag20", "images_of_text_scope_declared"
    )
    images_of_text_assessed = _claim_bool(
        claim_evidence, "wcag20", "images_of_text_assessed"
    )
    images_of_text_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "images_of_text_basis_recorded"
    )
    images_of_text_claim_evidence_satisfied = (
        images_of_text_assessed and images_of_text_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.images.of_text_seed",
            (
                "not_applicable"
                if not images_of_text_scope_declared
                else ("pass" if images_of_text_claim_evidence_satisfied else "manual_needed")
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Images-of-text criterion not applicable without a declared images-of-text review scope."
                if not images_of_text_scope_declared
                else (
                    "Images-of-text review evidence is recorded for the declared images-of-text scope."
                    if images_of_text_claim_evidence_satisfied
                    else "Images-of-text criterion is in scope for declared content; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "images_of_text_scope_declared": images_of_text_scope_declared,
                        "images_of_text_assessed": images_of_text_assessed,
                        "images_of_text_basis_recorded": images_of_text_basis_recorded,
                        "image_count": facts.image_count,
                        "image_title_only_count": facts.image_title_only_count,
                    }
                }
            ],
            applicability="applicable"
            if images_of_text_scope_declared
            else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not images_of_text_scope_declared
                else ("medium" if images_of_text_claim_evidence_satisfied else "low")
            ),
        )
    )
    prerecorded_av_alternative_scope_declared = _claim_bool(
        claim_evidence, "wcag20", "prerecorded_av_alternative_scope_declared"
    )
    prerecorded_av_alternative_assessed = _claim_bool(
        claim_evidence, "wcag20", "prerecorded_av_alternative_assessed"
    )
    prerecorded_av_alternative_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "prerecorded_av_alternative_basis_recorded"
    )
    prerecorded_av_alternative_claim_evidence_satisfied = (
        prerecorded_av_alternative_assessed
        and prerecorded_av_alternative_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.media.prerecorded_audio_video_alternative_seed",
            (
                "not_applicable"
                if not prerecorded_av_alternative_scope_declared
                else (
                    "pass"
                    if prerecorded_av_alternative_claim_evidence_satisfied
                    else "manual_needed"
                )
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Prerecorded audio-only/video-only alternative criterion not applicable without a declared media-alternative review scope."
                if not prerecorded_av_alternative_scope_declared
                else (
                    "Prerecorded audio-only/video-only alternative review evidence is recorded for the declared scope."
                    if prerecorded_av_alternative_claim_evidence_satisfied
                    else "Prerecorded audio-only/video-only alternative criterion is in scope for declared media content; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "prerecorded_av_alternative_scope_declared": prerecorded_av_alternative_scope_declared,
                        "prerecorded_av_alternative_assessed": prerecorded_av_alternative_assessed,
                        "prerecorded_av_alternative_basis_recorded": prerecorded_av_alternative_basis_recorded,
                        "autoplay_media_count": facts.autoplay_media_count,
                    }
                }
            ],
            applicability="applicable"
            if prerecorded_av_alternative_scope_declared
            else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not prerecorded_av_alternative_scope_declared
                else (
                    "medium"
                    if prerecorded_av_alternative_claim_evidence_satisfied
                    else "low"
                )
            ),
        )
    )
    prerecorded_captions_scope_declared = _claim_bool(
        claim_evidence, "wcag20", "prerecorded_captions_scope_declared"
    )
    prerecorded_captions_assessed = _claim_bool(
        claim_evidence, "wcag20", "prerecorded_captions_assessed"
    )
    prerecorded_captions_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "prerecorded_captions_basis_recorded"
    )
    prerecorded_captions_claim_evidence_satisfied = (
        prerecorded_captions_assessed and prerecorded_captions_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.media.prerecorded_captions_seed",
            (
                "not_applicable"
                if not prerecorded_captions_scope_declared
                else (
                    "pass"
                    if prerecorded_captions_claim_evidence_satisfied
                    else "manual_needed"
                )
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Prerecorded captions criterion not applicable without a declared prerecorded-media captions review scope."
                if not prerecorded_captions_scope_declared
                else (
                    "Prerecorded captions review evidence is recorded for the declared media scope."
                    if prerecorded_captions_claim_evidence_satisfied
                    else "Prerecorded captions criterion is in scope for declared prerecorded media; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "prerecorded_captions_scope_declared": prerecorded_captions_scope_declared,
                        "prerecorded_captions_assessed": prerecorded_captions_assessed,
                        "prerecorded_captions_basis_recorded": prerecorded_captions_basis_recorded,
                        "autoplay_media_count": facts.autoplay_media_count,
                    }
                }
            ],
            applicability="applicable"
            if prerecorded_captions_scope_declared
            else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not prerecorded_captions_scope_declared
                else (
                    "medium"
                    if prerecorded_captions_claim_evidence_satisfied
                    else "low"
                )
            ),
        )
    )
    prerecorded_ad_or_media_alt_scope_declared = _claim_bool(
        claim_evidence,
        "wcag20",
        "prerecorded_audio_description_or_media_alternative_scope_declared",
    )
    prerecorded_ad_or_media_alt_assessed = _claim_bool(
        claim_evidence,
        "wcag20",
        "prerecorded_audio_description_or_media_alternative_assessed",
    )
    prerecorded_ad_or_media_alt_basis_recorded = _claim_bool(
        claim_evidence,
        "wcag20",
        "prerecorded_audio_description_or_media_alternative_basis_recorded",
    )
    prerecorded_ad_or_media_alt_claim_evidence_satisfied = (
        prerecorded_ad_or_media_alt_assessed
        and prerecorded_ad_or_media_alt_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.media.prerecorded_audio_description_or_media_alternative_seed",
            (
                "not_applicable"
                if not prerecorded_ad_or_media_alt_scope_declared
                else (
                    "pass"
                    if prerecorded_ad_or_media_alt_claim_evidence_satisfied
                    else "manual_needed"
                )
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Prerecorded audio-description/media-alternative criterion not applicable without a declared review scope."
                if not prerecorded_ad_or_media_alt_scope_declared
                else (
                    "Prerecorded audio-description/media-alternative review evidence is recorded for the declared media scope."
                    if prerecorded_ad_or_media_alt_claim_evidence_satisfied
                    else "Prerecorded audio-description/media-alternative criterion is in scope for declared prerecorded media; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "prerecorded_audio_description_or_media_alternative_scope_declared": prerecorded_ad_or_media_alt_scope_declared,
                        "prerecorded_audio_description_or_media_alternative_assessed": prerecorded_ad_or_media_alt_assessed,
                        "prerecorded_audio_description_or_media_alternative_basis_recorded": prerecorded_ad_or_media_alt_basis_recorded,
                        "autoplay_media_count": facts.autoplay_media_count,
                    }
                }
            ],
            applicability="applicable"
            if prerecorded_ad_or_media_alt_scope_declared
            else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not prerecorded_ad_or_media_alt_scope_declared
                else (
                    "medium"
                    if prerecorded_ad_or_media_alt_claim_evidence_satisfied
                    else "low"
                )
            ),
        )
    )
    live_captions_scope_declared = _claim_bool(
        claim_evidence, "wcag20", "live_captions_scope_declared"
    )
    live_captions_assessed = _claim_bool(claim_evidence, "wcag20", "live_captions_assessed")
    live_captions_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "live_captions_basis_recorded"
    )
    live_captions_claim_evidence_satisfied = (
        live_captions_assessed and live_captions_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.media.live_captions_seed",
            (
                "not_applicable"
                if not live_captions_scope_declared
                else ("pass" if live_captions_claim_evidence_satisfied else "manual_needed")
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Live-captions criterion not applicable without a declared live-media captions review scope."
                if not live_captions_scope_declared
                else (
                    "Live captions review evidence is recorded for the declared live-media scope."
                    if live_captions_claim_evidence_satisfied
                    else "Live-captions criterion is in scope for declared live media; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "live_captions_scope_declared": live_captions_scope_declared,
                        "live_captions_assessed": live_captions_assessed,
                        "live_captions_basis_recorded": live_captions_basis_recorded,
                        "autoplay_media_count": facts.autoplay_media_count,
                    }
                }
            ],
            applicability="applicable" if live_captions_scope_declared else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not live_captions_scope_declared
                else ("medium" if live_captions_claim_evidence_satisfied else "low")
            ),
        )
    )
    prerecorded_audio_description_scope_declared = _claim_bool(
        claim_evidence, "wcag20", "prerecorded_audio_description_scope_declared"
    )
    prerecorded_audio_description_assessed = _claim_bool(
        claim_evidence, "wcag20", "prerecorded_audio_description_assessed"
    )
    prerecorded_audio_description_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "prerecorded_audio_description_basis_recorded"
    )
    prerecorded_audio_description_claim_evidence_satisfied = (
        prerecorded_audio_description_assessed
        and prerecorded_audio_description_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.media.prerecorded_audio_description_seed",
            (
                "not_applicable"
                if not prerecorded_audio_description_scope_declared
                else (
                    "pass"
                    if prerecorded_audio_description_claim_evidence_satisfied
                    else "manual_needed"
                )
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Prerecorded audio-description criterion not applicable without a declared prerecorded-audio-description review scope."
                if not prerecorded_audio_description_scope_declared
                else (
                    "Prerecorded audio-description review evidence is recorded for the declared media scope."
                    if prerecorded_audio_description_claim_evidence_satisfied
                    else "Prerecorded audio-description criterion is in scope for declared prerecorded media; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "prerecorded_audio_description_scope_declared": prerecorded_audio_description_scope_declared,
                        "prerecorded_audio_description_assessed": prerecorded_audio_description_assessed,
                        "prerecorded_audio_description_basis_recorded": prerecorded_audio_description_basis_recorded,
                        "autoplay_media_count": facts.autoplay_media_count,
                    }
                }
            ],
            applicability="applicable"
            if prerecorded_audio_description_scope_declared
            else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not prerecorded_audio_description_scope_declared
                else (
                    "medium"
                    if prerecorded_audio_description_claim_evidence_satisfied
                    else "low"
                )
            ),
        )
    )
    meaningful_sequence_scope_declared = _claim_bool(
        claim_evidence, "wcag20", "meaningful_sequence_scope_declared"
    )
    meaningful_sequence_assessed = _claim_bool(
        claim_evidence, "wcag20", "meaningful_sequence_assessed"
    )
    meaningful_sequence_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "meaningful_sequence_basis_recorded"
    )
    meaningful_sequence_claim_evidence_satisfied = (
        meaningful_sequence_assessed and meaningful_sequence_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.sequence.meaningful_sequence_seed",
            (
                "not_applicable"
                if not meaningful_sequence_scope_declared
                else ("pass" if meaningful_sequence_claim_evidence_satisfied else "manual_needed")
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Meaningful-sequence criterion not applicable without a declared sequence-dependent content scope."
                if not meaningful_sequence_scope_declared
                else (
                    "Meaningful-sequence review evidence is recorded for the declared content sequence scope."
                    if meaningful_sequence_claim_evidence_satisfied
                    else "Meaningful-sequence criterion is in scope for declared content; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "meaningful_sequence_scope_declared": meaningful_sequence_scope_declared,
                        "meaningful_sequence_assessed": meaningful_sequence_assessed,
                        "meaningful_sequence_basis_recorded": meaningful_sequence_basis_recorded,
                        "table_count": len(facts.tables),
                        "body_text_char_count": len(facts.body_text or ""),
                    }
                }
            ],
            applicability="applicable"
            if meaningful_sequence_scope_declared
            else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not meaningful_sequence_scope_declared
                else ("medium" if meaningful_sequence_claim_evidence_satisfied else "low")
            ),
        )
    )
    multiple_ways_scope_declared = _claim_bool(
        claim_evidence, "wcag20", "multiple_ways_scope_declared"
    )
    multiple_ways_assessed = _claim_bool(claim_evidence, "wcag20", "multiple_ways_assessed")
    multiple_ways_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "multiple_ways_basis_recorded"
    )
    multiple_ways_claim_evidence_satisfied = (
        multiple_ways_assessed and multiple_ways_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.navigation.multiple_ways_seed",
            (
                "not_applicable"
                if not multiple_ways_scope_declared
                else ("pass" if multiple_ways_claim_evidence_satisfied else "manual_needed")
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Multiple-ways criterion not applicable without a declared page-set navigation scope."
                if not multiple_ways_scope_declared
                else (
                    "Multiple-ways navigation/access-path review evidence is recorded for the declared page-set scope."
                    if multiple_ways_claim_evidence_satisfied
                    else "Multiple-ways criterion is in scope for the declared page-set; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "multiple_ways_scope_declared": multiple_ways_scope_declared,
                        "multiple_ways_assessed": multiple_ways_assessed,
                        "multiple_ways_basis_recorded": multiple_ways_basis_recorded,
                    }
                }
            ],
            applicability="applicable" if multiple_ways_scope_declared else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not multiple_ways_scope_declared
                else ("medium" if multiple_ways_claim_evidence_satisfied else "low")
            ),
        )
    )
    consistent_navigation_scope_declared = _claim_bool(
        claim_evidence, "wcag20", "consistent_navigation_scope_declared"
    )
    consistent_navigation_assessed = _claim_bool(
        claim_evidence, "wcag20", "consistent_navigation_assessed"
    )
    consistent_navigation_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "consistent_navigation_basis_recorded"
    )
    consistent_navigation_claim_evidence_satisfied = (
        consistent_navigation_assessed and consistent_navigation_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.navigation.consistent_navigation_seed",
            (
                "not_applicable"
                if not consistent_navigation_scope_declared
                else (
                    "pass"
                    if consistent_navigation_claim_evidence_satisfied
                    else "manual_needed"
                )
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "Consistent-navigation criterion not applicable without a declared page-set navigation scope."
                if not consistent_navigation_scope_declared
                else (
                    "Consistent-navigation review evidence is recorded for the declared page-set scope."
                    if consistent_navigation_claim_evidence_satisfied
                    else "Consistent-navigation criterion is in scope for the declared page-set; manual evidence is required."
                )
            ),
            evidence=[
                {
                    "values": {
                        "profile": profile,
                        "consistent_navigation_scope_declared": consistent_navigation_scope_declared,
                        "consistent_navigation_assessed": consistent_navigation_assessed,
                        "consistent_navigation_basis_recorded": consistent_navigation_basis_recorded,
                    }
                }
            ],
            applicability="applicable"
            if consistent_navigation_scope_declared
            else "not_applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if not consistent_navigation_scope_declared
                else (
                    "medium" if consistent_navigation_claim_evidence_satisfied else "low"
                )
            ),
        )
    )
    ast_signal_count = (
        facts.script_element_count
        + facts.embedded_active_content_count
        + facts.autoplay_media_count
        + facts.blink_marquee_count
        + facts.inline_event_handler_attr_count
        + facts.meta_refresh_count
    )
    tech_support_assessed = _claim_bool(claim_evidence, "technology_support", "assessed")
    tech_support_basis_recorded = _claim_bool(
        claim_evidence, "technology_support", "basis_recorded"
    )
    tech_claim_evidence_satisfied = tech_support_assessed and tech_support_basis_recorded
    findings.append(
        _vf(
            "fb.a11y.claim.accessibility_supported_technologies_seed",
            "warn" if ast_signal_count > 0 else ("pass" if tech_claim_evidence_satisfied else "manual_needed"),
            "medium",
            "adapter",
            "adapter",
            (
                "Potential technology support risk signals detected; accessibility-supported technology claim requires manual evidence."
                if ast_signal_count > 0
                else (
                    "Accessibility-supported technology claim evidence is recorded and no obvious technology-risk signals were detected."
                    if tech_claim_evidence_satisfied
                    else "No obvious technology-support risk signals detected, but accessibility-supported technology claim still requires manual evidence."
                )
            ),
            evidence=[
                {
                    "values": {
                        "embedded_active_content_count": facts.embedded_active_content_count,
                        "script_element_count": facts.script_element_count,
                        "autoplay_media_count": facts.autoplay_media_count,
                        "blink_marquee_count": facts.blink_marquee_count,
                        "inline_event_handler_attr_count": facts.inline_event_handler_attr_count,
                        "meta_refresh_count": facts.meta_refresh_count,
                        "css_linked": facts.has_css_link,
                        "technology_support_assessed": tech_support_assessed,
                        "technology_support_basis_recorded": tech_support_basis_recorded,
                    }
                }
            ],
            verification_mode="hybrid",
            applicability="applicable",
            confidence="medium" if (ast_signal_count > 0 or tech_claim_evidence_satisfied) else "low",
        )
    )
    consistent_identification_target_count = facts.link_count + facts.form_control_count
    consistent_identification_assessed = _claim_bool(
        claim_evidence, "wcag20", "consistent_identification_assessed"
    )
    consistent_identification_basis_recorded = _claim_bool(
        claim_evidence, "wcag20", "consistent_identification_basis_recorded"
    )
    consistent_identification_claim_evidence_satisfied = (
        consistent_identification_assessed and consistent_identification_basis_recorded
    )
    findings.append(
        _vf(
            "fb.a11y.identification.consistent_identification_seed",
            (
                "not_applicable"
                if consistent_identification_target_count == 0
                else (
                    "pass"
                    if consistent_identification_claim_evidence_satisfied
                    else "manual_needed"
                )
            ),
            "medium",
            "adapter",
            "adapter",
            (
                "No interactive links or form controls detected; consistent-identification seed not applicable."
                if consistent_identification_target_count == 0
                else (
                    "Consistent-identification review evidence is recorded for interactive components."
                    if consistent_identification_claim_evidence_satisfied
                    else "Interactive components detected; consistent-identification review requires manual evidence."
                )
            ),
            evidence=[
                {
                    "values": {
                        "interactive_identification_target_count": consistent_identification_target_count,
                        "link_count": facts.link_count,
                        "form_control_count": facts.form_control_count,
                        "consistent_identification_assessed": consistent_identification_assessed,
                        "consistent_identification_basis_recorded": consistent_identification_basis_recorded,
                    }
                }
            ],
            applicability="not_applicable"
            if consistent_identification_target_count == 0
            else "applicable",
            verification_mode="hybrid",
            confidence=(
                "high"
                if consistent_identification_target_count == 0
                else ("medium" if consistent_identification_claim_evidence_satisfied else "low")
            ),
        )
    )
    s508_scope_declared = _claim_bool(claim_evidence, "section508", "scope_declared")
    s508_public_recorded = _claim_bool(
        claim_evidence, "section508", "public_facing_determination_recorded"
    )
    s508_official_recorded = _claim_bool(
        claim_evidence, "section508", "official_communications_determination_recorded"
    )
    s508_nara_recorded = _claim_bool(
        claim_evidence, "section508", "nara_exception_determination_recorded"
    )
    findings.append(
        _vf(
            "fb.a11y.claim.section508.public_facing_content_applicability_seed",
            "pass" if (s508_scope_declared and s508_public_recorded) else "manual_needed",
            "medium",
            "adapter",
            "adapter",
            (
                "Section 508 E205.2 public-facing applicability decision evidence is recorded."
                if (s508_scope_declared and s508_public_recorded)
                else "Section 508 E205.2 public-facing applicability requires agency/content scope evidence; manual review required."
            ),
            evidence=[
                {
                    "values": {
                        "delivery_target": "html",
                        "section508_scope_declared": s508_scope_declared,
                        "public_facing_determination_recorded": s508_public_recorded,
                    }
                }
            ],
            verification_mode="hybrid",
            applicability="applicable",
            confidence="medium" if (s508_scope_declared and s508_public_recorded) else "low",
        )
    )
    findings.append(
        _vf(
            "fb.a11y.claim.section508.official_communications_applicability_seed",
            "pass" if (s508_scope_declared and s508_official_recorded) else "manual_needed",
            "medium",
            "adapter",
            "adapter",
            (
                "Section 508 E205.3 official communications applicability decision evidence is recorded."
                if (s508_scope_declared and s508_official_recorded)
                else "Section 508 E205.3 agency official communications applicability requires agency communication-scope evidence; manual review required."
            ),
            evidence=[
                {
                    "values": {
                        "delivery_target": "html",
                        "section508_scope_declared": s508_scope_declared,
                        "official_communications_determination_recorded": s508_official_recorded,
                    }
                }
            ],
            verification_mode="hybrid",
            applicability="applicable",
            confidence="medium" if (s508_scope_declared and s508_official_recorded) else "low",
        )
    )
    findings.append(
        _vf(
            "fb.a11y.claim.section508.nara_exception_applicability_seed",
            "pass" if s508_nara_recorded else "manual_needed",
            "low",
            "adapter",
            "adapter",
            (
                "Section 508 E205.3 NARA exception applicability decision evidence is recorded."
                if s508_nara_recorded
                else "Section 508 E205.3 NARA exception applicability requires organization/content stewardship evidence; manual review required."
            ),
            evidence=[
                {
                    "values": {
                        "delivery_target": "html",
                        "nara_exception_determination_recorded": s508_nara_recorded,
                    }
                }
            ],
            verification_mode="hybrid",
            applicability="applicable",
            confidence="medium" if s508_nara_recorded else "low",
        )
    )
    findings.append(
        _vf(
            "fb.a11y.claim.section508.non_web_document_exceptions_html_seed",
            "not_applicable",
            "low",
            "adapter",
            "adapter",
            "Section 508 E205.4 Exception and E205.4.1 word-substitution rules are not applicable to HTML deliverables (non-web document path not in scope).",
            evidence=[
                {
                    "values": {
                        "delivery_target": "html",
                        "non_web_document_path": False,
                    }
                }
            ],
            verification_mode="hybrid",
            applicability="not_applicable",
            confidence="high",
        )
    )
    if render_preview_png_path is not None:
        contrast = _contrast_render_seed_analysis(render_preview_png_path)
        findings.append(
            _vf(
                "fb.a11y.contrast.minimum_render_seed",
                str(contrast.get("verdict") or "manual_needed"),
                "medium",
                "post-render",
                "adapter",
                str(contrast.get("message") or "Render-based contrast seed analysis."),
                evidence=[
                    {
                        "values": {
                            "render_preview_png_path": str(render_preview_png_path),
                            "width": contrast.get("width", ""),
                            "height": contrast.get("height", ""),
                            "opaque_pixel_count": contrast.get("opaque_pixel_count", ""),
                            "ink_pixel_count": contrast.get("ink_pixel_count", ""),
                            "background_luminance": contrast.get("background_luminance", ""),
                            "foreground_luminance": contrast.get("foreground_luminance", ""),
                            "estimated_contrast_ratio": contrast.get("estimated_contrast_ratio", ""),
                        }
                    }
                ],
                verification_mode="hybrid",
                confidence=str(contrast.get("confidence") or "low"),
                applicability="applicable",
            )
        )
    if profile == "cav":
        hits = _cav_note_hits(facts.body_text)
        findings.append(
            _vf(
                "fb.a11y.cav.document_only_content",
                "fail" if hits else "pass",
                "critical",
                "post-emit",
                "fullbleed",
                "No remediation notes detected in CAV deliverable body." if not hits else "Potential remediation/provenance note leakage detected in CAV deliverable body.",
                evidence=[{"values": {"hits": hits}}],
                confidence="high" if not hits else "medium",
            )
        )
    else:
        findings.append(_vf("fb.a11y.cav.document_only_content", "not_applicable", "low", "post-emit", "fullbleed", "CAV-only rule not applicable.", applicability="not_applicable"))

    manual = _manual_debt(parity_report)
    if manual["item_count"] > 0:
        findings.append(
            _vf(
                "fb.a11y.cav.manual_transcription_quality_review",
                "manual_needed",
                "medium",
                "post-render",
                "manual",
                f"{manual['item_count']} item(s) require manual transcription review.",
                verification_mode="manual",
                applicability="unknown",
                confidence="low",
            )
        )

    counts_pre = {k: 0 for k in ("pass", "fail", "warn", "manual_needed", "not_applicable")}
    for f in findings:
        counts_pre[str(f.get("verdict"))] = counts_pre.get(str(f.get("verdict")), 0) + 1
    wcag20aa_coverage_pre = wcag20aa_coverage_from_findings(findings)
    claim_readiness_pre = _wcag20aa_claim_readiness_scaffold(
        fail_count=counts_pre["fail"],
        wcag20aa_coverage=wcag20aa_coverage_pre,
        manual_review_debt_count=manual["item_count"],
    )
    findings.append(
        _vf(
            "fb.a11y.claim.wcag20aa_level_readiness",
            "warn"
            if claim_readiness_pre["status"] in {"blocked_machine_failures", "blocked_coverage_gaps"}
            else "manual_needed",
            "high"
            if claim_readiness_pre["status"] == "blocked_machine_failures"
            else "medium",
            "adapter",
            "adapter",
            f"WCAG 2.0 AA conformance-level claim scaffold status: {claim_readiness_pre['status']}.",
            evidence=[
                {
                    "values": {
                        "status": claim_readiness_pre["status"],
                        "machine_blocker_count": claim_readiness_pre["machine_blocker_count"],
                        "coverage_gap_count": claim_readiness_pre["coverage_gap_count"],
                    }
                }
            ],
            verification_mode="hybrid",
            confidence="high",
        )
    )
    findings, observability = _dedup_and_correlate_findings(findings)
    gate = _gate(findings, id_key="rule_id", mode=mode, entries=entries, overrides=overrides)
    counts = {k: 0 for k in ("pass", "fail", "warn", "manual_needed", "not_applicable")}
    for f in findings:
        counts[str(f.get("verdict"))] = counts.get(str(f.get("verdict")), 0) + 1
    reg_rules = [e["id"] for e in reg.get("entries", []) if e.get("system") == "a11y_verifier"]
    evaluated = {f["rule_id"] for f in findings}
    conformance_status = {
        "status": "fail_machine_subset" if counts["fail"] else ("manual_review_required" if (manual["item_count"] or counts["manual_needed"]) else "pass_machine_subset"),
        "claim_scope": "manual_required" if (manual["item_count"] or counts["manual_needed"]) else "machine_subset",
        "manual_review_required": bool(manual["item_count"] or counts["manual_needed"]),
    }
    wcag20aa_coverage = wcag20aa_coverage_from_findings(findings)
    section508_coverage = section508_html_coverage_from_findings(findings)
    wcag20aa_claim_readiness = _wcag20aa_claim_readiness_scaffold(
        fail_count=counts["fail"],
        wcag20aa_coverage=wcag20aa_coverage,
        manual_review_debt_count=manual["item_count"],
    )
    return {
        "schema": "fullbleed.a11y.verify.v1",
        "target": {"html_path": str(html_p), "css_path": str(css_p), "target_hash": _sha(html_p)},
        "profile": profile,
        "conformance_status": conformance_status,
        "gate": gate,
        "summary": {
            "pass_count": counts["pass"],
            "fail_count": counts["fail"],
            "warn_count": counts["warn"],
            "manual_needed_count": counts["manual_needed"],
            "not_applicable_count": counts["not_applicable"],
        },
        "findings": findings,
        "observability": observability,
        "coverage": {
            "evaluated_rule_count": len(evaluated),
            "applicable_rule_count": sum(1 for f in findings if f["applicability"] == "applicable"),
            "machine_rule_count": sum(1 for f in findings if f["verification_mode"] == "machine"),
            "manual_rule_count": sum(1 for f in findings if f["verification_mode"] == "manual"),
            "manual_needed_count": counts["manual_needed"],
            "not_evaluated_rule_count": max(0, len(reg_rules) - len(evaluated & set(reg_rules))),
            "rule_pack_coverage": [
                {
                    "pack_id": "fullbleed.a11y_verifier.registry.v1",
                    "evaluated": len(evaluated & set(reg_rules)),
                    "total": len(reg_rules),
                },
                {
                    "pack_id": "wcag20aa.implemented_map.v1",
                    "evaluated": wcag20aa_coverage["implemented_mapped_entry_evaluated_count"],
                    "total": wcag20aa_coverage["implemented_mapped_entry_count"],
                },
                {
                    "pack_id": "section508_html.implemented_map.v1",
                    "evaluated": section508_coverage["implemented_mapped_entry_evaluated_count"],
                    "total": section508_coverage["implemented_mapped_entry_count"],
                },
            ],
            "wcag20aa": wcag20aa_coverage,
            "section508": section508_coverage,
        },
        "wcag20aa_claim_readiness": wcag20aa_claim_readiness,
        "tooling": {"fullbleed_version": fullbleed_version, "report_schema_version": "1.0.0-draft", "generated_at": generated_at or _now()},
        "artifacts": {
            "html_hash": _sha(html_p),
            "css_hash": _sha(css_p),
            "css_linked": facts.has_css_link,
            "packaging_mode": "linked-css" if facts.has_css_link else "separate-files",
        },
        "manual_review_debt": manual,
    }


def _pmr_score(verdict: str) -> float | None:
    return {"pass": 1.0, "warn": 0.5, "fail": 0.0}.get(verdict)


def _pmr_band(score: float) -> str:
    if score >= 95:
        return "excellent"
    if score >= 85:
        return "good"
    if score >= 70:
        return "watch"
    return "poor"


def _pa(
    audit_id: str,
    *,
    category: str,
    weight: float,
    audit_class: str,
    verification_mode: str,
    severity: str,
    stage: str,
    source: str,
    verdict: str,
    message: str,
    scored: bool,
    evidence: list[dict[str, Any]] | None = None,
    fix_hint: str | None = None,
) -> dict[str, Any]:
    d = {
        "audit_id": audit_id,
        "category": category,
        "weight": float(weight),
        "class": audit_class,
        "verification_mode": verification_mode,
        "severity": severity,
        "stage": stage,
        "source": source,
        "verdict": verdict,
        "scored": bool(scored),
        "message": message,
    }
    if scored:
        score = _pmr_score(verdict)
        if score is not None:
            d["score"] = score
    if evidence:
        d["evidence"] = evidence
    if fix_hint:
        d["fix_hint"] = fix_hint
    return d


def prototype_verify_paged_media_rank(
    *,
    html_path: str | Path,
    css_path: str | Path,
    profile: str = "strict",
    mode: str = "error",
    a11y_report: dict[str, Any] | None = None,
    component_validation: dict[str, Any] | None = None,
    parity_report: dict[str, Any] | None = None,
    run_report: dict[str, Any] | None = None,
    expected_lang: str | None = None,
    expected_title: str | None = None,
    registry: dict[str, Any] | None = None,
    generated_at: str | None = None,
    fullbleed_version: str = "0.6.0",
) -> dict[str, Any]:
    reg = registry or _registry()
    entries, cats = _indexes(reg)
    overrides = _profile_override_levels(reg, profile)
    html_p = Path(html_path)
    css_p = Path(css_path)
    facts = parse_html_facts(html_p.read_text(encoding="utf-8"))
    comp = component_validation or {}
    audits: list[dict[str, Any]] = []

    def E(aid: str) -> dict[str, Any]:
        return entries[aid]

    # Document semantics
    e = E("pmr.doc.lang_present_valid")
    lang_pass = _lang_ok(facts.html_lang) and (expected_lang is None or facts.html_lang == expected_lang)
    audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode=e["verification_mode"], severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="pass" if lang_pass else "fail", message="HTML lang is present and valid." if lang_pass else "HTML lang missing/invalid or metadata mismatch.", scored=e.get("scored", True), evidence=[{"selector": "html", "values": {"lang": facts.html_lang or ""}}]))
    e = E("pmr.doc.title_present_nonempty")
    title_pass = bool(facts.title.strip()) and (expected_title is None or facts.title == expected_title)
    audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode=e["verification_mode"], severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="pass" if title_pass else "fail", message="Document title is present and non-empty." if title_pass else "Document title missing/empty or metadata mismatch.", scored=e.get("scored", True), evidence=[{"selector": "head > title", "values": {"title": facts.title}}]))
    e = E("pmr.doc.metadata_engine_persistence")
    if expected_lang is None and expected_title is None:
        audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode="manual", severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="manual_needed", message="Expected metadata not supplied; cannot verify engine persistence.", scored=False))
    else:
        audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode=e["verification_mode"], severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="pass" if (lang_pass and title_pass) else "fail", message="Engine metadata persisted into emitted HTML." if (lang_pass and title_pass) else "Engine metadata persistence check failed.", scored=e.get("scored", True)))

    # Paged layout integrity
    e = E("pmr.layout.overflow_none")
    overflow = _i(comp.get("overflow_count"), 0)
    audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode=e["verification_mode"], severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="pass" if overflow == 0 else "fail", message="No overflow placements detected." if overflow == 0 else f"Overflow placements detected ({overflow}).", scored=e.get("scored", True), evidence=[{"diagnostic_ref": "component_validation.overflow_count", "values": {"overflow_count": overflow}}]))
    e = E("pmr.layout.known_loss_none_critical")
    known_loss = _i(comp.get("known_loss_count"), 0)
    audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode=e["verification_mode"], severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="pass" if known_loss == 0 else "fail", message="No critical known-loss events detected." if known_loss == 0 else f"Known-loss events detected ({known_loss}).", scored=e.get("scored", True), evidence=[{"diagnostic_ref": "component_validation.known_loss_count", "values": {"known_loss_count": known_loss}}]))
    e = E("pmr.layout.page_count_target")
    src_pages = None
    rnd_pages = None
    if run_report:
        m = run_report.get("metrics", {})
        src_pages = m.get("source_page_count")
        rnd_pages = m.get("render_page_count")
    if src_pages is None and parity_report:
        src_pages = parity_report.get("source_characteristics", {}).get("page_count")
    if src_pages is None or rnd_pages is None:
        audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode="manual", severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="manual_needed", message="Page-count target could not be evaluated.", scored=False))
    else:
        pp = _i(src_pages) == _i(rnd_pages)
        audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode=e["verification_mode"], severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="pass" if pp else "fail", message="Page-count target satisfied." if pp else f"Page-count parity mismatch (source={src_pages}, render={rnd_pages}).", scored=e.get("scored", True), evidence=[{"values": {"source_page_count": src_pages, "render_page_count": rnd_pages}}]))

    # Field/table/form integrity
    e = E("pmr.forms.id_ref_integrity")
    ids_ok = (not facts.dup_ids) and (not facts.missing_idrefs)
    audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode=e["verification_mode"], severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="pass" if ids_ok else "fail", message="ID and IDREF integrity checks passed." if ids_ok else "Duplicate IDs or missing IDREF targets detected.", scored=e.get("scored", True), evidence=[{"values": {"duplicate_ids": facts.dup_ids, "missing_idrefs": facts.missing_idrefs}}]))
    e = E("pmr.tables.semantic_table_headers")
    if not facts.tables:
        audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode=e["verification_mode"], severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="not_applicable", message="No table elements detected.", scored=False))
    else:
        ok = True
        ev = []
        for idx, tbl in enumerate(facts.tables):
            if _i(tbl.get("th_count")) > 0:
                this_ok = bool(tbl.get("has_caption")) or _i(tbl.get("th_scope_count")) > 0
                ok = ok and this_ok
                ev.append({"values": {"table_index": idx, **tbl}})
        audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode=e["verification_mode"], severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="pass" if ok else "fail", message="Semantic table header checks passed." if ok else "Semantic table header checks failed.", scored=e.get("scored", True), evidence=ev or [{"values": {"table_count": len(facts.tables)}}]))
    e = E("pmr.signatures.text_semantics_present")
    if profile in {"cav", "transactional"}:
        sig_ok = facts.sig_count > 0
        audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode=e["verification_mode"], severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="pass" if sig_ok else "fail", message="Text signature semantics detected." if sig_ok else "No text signature semantics detected.", scored=e.get("scored", True), evidence=[{"values": {"signature_semantic_count": facts.sig_count}}]))
    else:
        audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode=e["verification_mode"], severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="not_applicable", message="Not applicable for this profile.", scored=False))
    e = E("pmr.cav.document_only_content")
    if profile == "cav":
        hits = _cav_note_hits(facts.body_text)
        audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode=e["verification_mode"], severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="pass" if not hits else "fail", message="CAV deliverable body contains document-only content." if not hits else "Potential remediation/provenance note leakage detected in CAV deliverable body.", scored=e.get("scored", True), evidence=[{"values": {"hits": hits}}]))
    else:
        audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode=e["verification_mode"], severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="not_applicable", message="Not a CAV profile.", scored=False))

    # Artifact packaging
    e = E("pmr.artifacts.html_emitted")
    html_ok = html_p.exists() and html_p.stat().st_size > 0
    audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode=e["verification_mode"], severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="pass" if html_ok else "fail", message="HTML artifact emitted." if html_ok else "HTML artifact missing or empty.", scored=e.get("scored", True)))
    e = E("pmr.artifacts.css_emitted")
    css_ok = css_p.exists() and css_p.stat().st_size > 0
    audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode=e["verification_mode"], severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="pass" if css_ok else "fail", message="CSS artifact emitted." if css_ok else "CSS artifact missing or empty.", scored=e.get("scored", True)))
    e = E("pmr.artifacts.linked_css_reference")
    audits.append(_pa(e["id"], category=e["category"], weight=e["weight"], audit_class=e["class"], verification_mode=e["verification_mode"], severity=e["severity"], stage=e["stage"], source="fullbleed", verdict="pass" if facts.has_css_link else "warn", message="HTML artifact includes linked CSS reference." if facts.has_css_link else "HTML artifact does not include linked CSS reference (separate artifact mode).", scored=False, evidence=[{"selector": "link[rel~=stylesheet]", "values": {"hrefs": facts.css_hrefs}}], fix_hint=None if facts.has_css_link else "Enable CSS link injection packaging mode for standalone HTML artifacts."))

    manual = _manual_debt(parity_report)
    cat_rows: list[dict[str, Any]] = []
    for cat in reg.get("pmr_categories", []):
        cid = cat["id"]
        subset = [a for a in audits if a["category"] == cid]
        scored = [(float(a.get("score")), float(a["weight"])) for a in subset if a.get("scored") and a.get("score") is not None]
        denom = sum(w for _, w in scored) or 1.0
        cat_score = 100.0 * (sum(s * w for s, w in scored) / denom) if scored else 100.0
        warn_n = sum(1 for a in subset if a["verdict"] == "warn")
        fail_n = sum(1 for a in subset if a["verdict"] == "fail")
        manual_n = sum(1 for a in subset if a["verdict"] == "manual_needed")
        conf = _clamp(100.0 - (10.0 * manual_n) - (3.0 * warn_n) - (5.0 * fail_n), 0.0, 100.0)
        cat_rows.append({"id": cid, "name": cat["name"], "weight": float(cat["weight"]), "score": round(cat_score, 2), "confidence": round(conf, 2), "audit_count": len(subset), "fail_count": fail_n, "warn_count": warn_n})
    denom = sum(float(c["weight"]) for c in cat_rows) or 1.0
    score = sum(float(c["score"]) * float(c["weight"]) for c in cat_rows) / denom
    conf = sum(float(c["confidence"]) * float(c["weight"]) for c in cat_rows) / denom
    if manual["item_count"] > 0:
        conf = _clamp(conf - min(25.0, 3.0 * manual["item_count"]), 0.0, 100.0)

    gate = _gate(audits, id_key="audit_id", mode=mode, entries=entries, overrides=overrides)
    failed_ids = set(gate.get("failed_audit_ids") or [])
    correlation_index = []
    for audit in audits:
        verdict = str(audit.get("verdict") or "")
        fix_hint = audit.get("fix_hint")
        if verdict not in {"fail", "warn", "manual_needed"} and not fix_hint:
            continue
        row = {
            "audit_id": str(audit.get("audit_id") or ""),
            "category": str(audit.get("category") or ""),
            "class": str(audit.get("class") or ""),
            "verdict": verdict,
            "severity": str(audit.get("severity") or ""),
            "stage": str(audit.get("stage") or ""),
            "source": str(audit.get("source") or ""),
            "gate_failed": str(audit.get("audit_id") or "") in failed_ids,
            "gate_relevant": verdict in {"fail", "warn"},
            "opportunity": bool(fix_hint) or str(audit.get("class") or "") == "opportunity",
            "scored": bool(audit.get("scored")),
            "has_fix_hint": bool(fix_hint),
        }
        if "score" in audit:
            row["score"] = audit.get("score")
        correlation_index.append(row)
    observability = {
        "original_audit_count": len(audits),
        "reported_audit_count": len(audits),
        "dedup_event_count": 0,
        "dedup_merged_audit_count": 0,
        "correlated_audit_count": len(correlation_index),
        "stage_counts": _count_by_key(audits, "stage"),
        "source_counts": _count_by_key(audits, "source"),
        "category_counts": _count_by_key(audits, "category"),
        "class_counts": _count_by_key(audits, "class"),
        "verdict_counts": _count_by_key(audits, "verdict"),
        "correlation_index": correlation_index,
    }
    return {
        "schema": "fullbleed.pmr.v1",
        "target": {"html_path": str(html_p), "css_path": str(css_p)},
        "profile": profile,
        "rank": {"score": round(score, 2), "confidence": round(conf, 2), "band": _pmr_band(score), "raw_score": round(score, 2)},
        "gate": gate,
        "categories": cat_rows,
        "audits": audits,
        "observability": observability,
        "manual_debt": manual,
        "coverage": {
            "evaluated_audit_count": len(audits),
            "applicable_audit_count": sum(1 for a in audits if a["verdict"] != "not_applicable"),
            "scored_audit_count": sum(1 for a in audits if a.get("scored")),
            "manual_needed_count": sum(1 for a in audits if a["verdict"] == "manual_needed"),
            "not_evaluated_audit_count": 0,
        },
        "tooling": {"fullbleed_version": fullbleed_version, "report_schema_version": "1.0.0-draft", "generated_at": generated_at or _now()},
        "artifacts": {"html_hash": _sha(html_p), "css_hash": _sha(css_p), "css_linked": facts.has_css_link, "packaging_mode": "linked-css" if facts.has_css_link else "separate-files"},
    }


def run_prototype_bundle(
    *,
    html_path: str | Path,
    css_path: str | Path,
    profile: str = "strict",
    mode: str = "error",
    a11y_report_path: str | Path | None = None,
    component_validation_path: str | Path | None = None,
    parity_report_path: str | Path | None = None,
    run_report_path: str | Path | None = None,
    expected_lang: str | None = None,
    expected_title: str | None = None,
) -> tuple[dict[str, Any], dict[str, Any]]:
    a11y_report = _j_opt(a11y_report_path)
    component_validation = _j_opt(component_validation_path)
    parity_report = _j_opt(parity_report_path)
    run_report = _j_opt(run_report_path)
    render_preview_png_path = None
    if run_report:
        try:
            previews = list((run_report.get("deliverables") or {}).get("render_preview_pngs") or [])
            if previews:
                render_preview_png_path = previews[0]
        except Exception:
            render_preview_png_path = None
    verifier = prototype_verify_accessibility(
        html_path=html_path,
        css_path=css_path,
        profile=profile,
        mode=mode,
        a11y_report=a11y_report,
        parity_report=parity_report,
        expected_lang=expected_lang,
        expected_title=expected_title,
        render_preview_png_path=render_preview_png_path,
    )
    pmr = prototype_verify_paged_media_rank(
        html_path=html_path,
        css_path=css_path,
        profile=profile,
        mode=mode,
        a11y_report=a11y_report,
        component_validation=component_validation,
        parity_report=parity_report,
        run_report=run_report,
        expected_lang=expected_lang,
        expected_title=expected_title,
    )
    return verifier, pmr


def _write_json(path: str | Path, obj: dict[str, Any]) -> None:
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(obj, indent=2), encoding="utf-8")


def _validate_outputs(verifier: dict[str, Any], pmr: dict[str, Any]) -> None:
    import jsonschema  # type: ignore

    jsonschema.Draft202012Validator(_j(_specs() / "fullbleed.a11y.verify.v1.schema.json")).validate(verifier)
    jsonschema.Draft202012Validator(_j(_specs() / "fullbleed.pmr.v1.schema.json")).validate(pmr)


def _parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description="FullBleed prototype accessibility verifier + PMR")
    p.add_argument("--html", required=True)
    p.add_argument("--css", required=True)
    p.add_argument("--profile", default="strict")
    p.add_argument("--mode", default="error", choices=["off", "warn", "error"])
    p.add_argument("--a11y-report")
    p.add_argument("--component-validation")
    p.add_argument("--parity-report")
    p.add_argument("--run-report")
    p.add_argument("--expected-lang")
    p.add_argument("--expected-title")
    p.add_argument("--verifier-out")
    p.add_argument("--pmr-out")
    p.add_argument("--validate-schema", action="store_true")
    p.add_argument("--print-summary", action="store_true")
    return p


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    verifier, pmr = run_prototype_bundle(
        html_path=args.html,
        css_path=args.css,
        profile=args.profile,
        mode=args.mode,
        a11y_report_path=args.a11y_report,
        component_validation_path=args.component_validation,
        parity_report_path=args.parity_report,
        run_report_path=args.run_report,
        expected_lang=args.expected_lang,
        expected_title=args.expected_title,
    )
    if args.validate_schema:
        _validate_outputs(verifier, pmr)
    if args.verifier_out:
        _write_json(args.verifier_out, verifier)
    if args.pmr_out:
        _write_json(args.pmr_out, pmr)
    if args.print_summary or (not args.verifier_out and not args.pmr_out):
        print(
            json.dumps(
                {
                    "verifier_gate_ok": verifier["gate"]["ok"],
                    "conformance_status": verifier["conformance_status"]["status"],
                    "pmr_gate_ok": pmr["gate"]["ok"],
                    "pmr_score": pmr["rank"]["score"],
                    "pmr_confidence": pmr["rank"]["confidence"],
                    "pmr_band": pmr["rank"]["band"],
                },
                indent=2,
            )
        )
    return 0 if verifier["gate"]["ok"] and pmr["gate"]["ok"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
