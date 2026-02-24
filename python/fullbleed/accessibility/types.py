from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


@dataclass
class AccessibilityRunResult:
    ok: bool
    pdf_ua_targeted: bool
    paths: dict[str, str] = field(default_factory=dict)
    verifier_report: dict[str, Any] | None = None
    pmr_report: dict[str, Any] | None = None
    pdf_ua_seed_report: dict[str, Any] | None = None
    reading_order_trace: dict[str, Any] | None = None
    pdf_structure_trace: dict[str, Any] | None = None
    reading_order_trace_render: dict[str, Any] | None = None
    pdf_structure_trace_render: dict[str, Any] | None = None
    run_report: dict[str, Any] = field(default_factory=dict)
    contract_fingerprint: str | None = None
    warnings: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        return {
            "ok": self.ok,
            "pdf_ua_targeted": self.pdf_ua_targeted,
            "paths": dict(self.paths),
            "verifier_report": self.verifier_report,
            "pmr_report": self.pmr_report,
            "pdf_ua_seed_report": self.pdf_ua_seed_report,
            "reading_order_trace": self.reading_order_trace,
            "pdf_structure_trace": self.pdf_structure_trace,
            "reading_order_trace_render": self.reading_order_trace_render,
            "pdf_structure_trace_render": self.pdf_structure_trace_render,
            "run_report": self.run_report,
            "contract_fingerprint": self.contract_fingerprint,
            "warnings": list(self.warnings),
        }
