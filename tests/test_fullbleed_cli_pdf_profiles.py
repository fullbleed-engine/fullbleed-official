from __future__ import annotations

import json
import re
from pathlib import Path
from types import SimpleNamespace

import pytest

from fullbleed_cli import cli
from tools import validate_pdf_profiles


REPO_ROOT = Path(__file__).resolve().parents[1]
STANDARD_PROFILE_CHOICES = set(cli.PDF_PROFILE_CHOICES) - {"none", "tagged"}
VERAPDF_PROFILE_CHOICES = STANDARD_PROFILE_CHOICES - {"pdfx4", "pdfvt1"}


@pytest.mark.parametrize(
    ("alias", "canonical"),
    [
        ("a", "pdfa2b"),
        ("pdf/a", "pdfa2b"),
        ("pdf/a-1a", "pdfa1a"),
        ("pdf/a-1b", "pdfa1b"),
        ("pdf/a-2a", "pdfa2a"),
        ("pdf/a-2b", "pdfa2b"),
        ("pdf/a-2u", "pdfa2u"),
        ("pdf/a-3a", "pdfa3a"),
        ("pdf/a-3b", "pdfa3b"),
        ("pdf/a-3u", "pdfa3u"),
        ("pdf/a-4", "pdfa4"),
        ("pdf/a-4e", "pdfa4e"),
        ("pdf/a-4f", "pdfa4f"),
        ("pdf/x-4", "pdfx4"),
        ("ua", "pdfua1"),
        ("pdf/ua", "pdfua1"),
        ("pdf/ua-2", "pdfua2"),
        ("vt", "pdfvt1"),
        ("pdf/vt", "pdfvt1"),
        ("wt1r", "wtpdf1r"),
        ("wt1a", "wtpdf1a"),
    ],
)
def test_cli_pdf_profile_aliases_normalize_to_canonical_standards(
    alias: str, canonical: str
) -> None:
    assert cli._normalize_pdf_profile(alias) == canonical


def test_cli_capabilities_expose_asserted_pdf_standards(capsys: pytest.CaptureFixture[str]) -> None:
    cli.cmd_capabilities(SimpleNamespace(json=True))
    payload = json.loads(capsys.readouterr().out)
    build_features_fn = getattr(cli.fullbleed, "build_features", None)
    build_features = dict(build_features_fn()) if callable(build_features_fn) else {}
    svg_raster_available = bool(build_features.get("svg_raster", False))

    assert payload["pdf_profiles"] == cli.PDF_PROFILE_CHOICES
    assert set(payload["pdf_profiles"]) >= STANDARD_PROFILE_CHOICES
    assert payload["pdf_profile_aliases"]["a"] == "pdfa2b"
    assert payload["pdf_profile_aliases"]["ua"] == "pdfua1"
    assert payload["pdf_profile_aliases"]["vt"] == "pdfvt1"
    assert payload["pdf_profile_aliases"]["wt1r"] == "wtpdf1r"
    assert payload["pdf_profile_aliases"]["wt1a"] == "wtpdf1a"
    assert set(payload["pdf_profiles_requiring_output_intent"]) == cli.PDF_PROFILES_REQUIRING_OUTPUT_INTENT
    assert payload["svg"]["engine_flags"]["svg_raster_fallback"] is svg_raster_available
    assert payload["svg"]["build_features"]["svg_raster"] is svg_raster_available
    assert payload["engine"]["compiled_document"] is True
    assert payload["engine"]["compiled_reflow_bindings"] is bool(
        build_features.get("compiled_reflow", False)
    )
    assert payload["engine"]["compiled_flow_compression_modes"] == list(
        build_features.get("compiled_flow_compression_modes", [])
    )
    assert "SVG text and tspan runs" in payload["svg"]["feature_matrix"]["native_vector"]
    assert "symbols with use viewports" in payload["svg"]["feature_matrix"]["native_vector"]
    assert "foreignObject content" in payload["svg"]["feature_matrix"]["unsupported_or_known_loss"]
    assert "basic shapes and paths" in payload["svg"]["feature_matrix"]["native_vector"]
    assert payload["charts"]["engine_owned"] is True
    assert payload["charts"]["kinds"] == ["bar", "line", "sparkline"]
    assert payload["charts"]["outputs"] == ["native_inline_svg", "semantic_html_table"]
    assert payload["charts"]["generated_svg_persisted"] is False
    assert payload["charts"]["browser_runtime_required"] is False


def test_cli_capabilities_without_native_extension_do_not_crash(
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.delattr(cli.fullbleed, "PdfEngine", raising=False)
    monkeypatch.delattr(cli.fullbleed, "build_features", raising=False)

    cli.cmd_capabilities(SimpleNamespace(json=True))
    payload = json.loads(capsys.readouterr().out)

    assert payload["engine"]["batch_render"] is False
    assert payload["engine"]["compiled_document"] is False
    assert payload["engine"]["compiled_reflow_bindings"] is False
    assert payload["engine"]["compiled_flow_compression_modes"] == []
    assert payload["engine"]["template_compose_planner"] is False
    assert payload["svg"]["build_features"]["svg_raster"] is False


def test_release_packaging_enables_asserted_svg_raster_fallback() -> None:
    pyproject = (REPO_ROOT / "pyproject.toml").read_text(encoding="utf-8")
    cargo_manifest = (REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8")
    ci = (REPO_ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")

    feature_match = re.search(r"(?m)^features\s*=\s*\[(?P<features>[^\]]+)\]", pyproject)
    assert feature_match, "pyproject.toml must declare native backend build features"
    features = {
        item.strip().strip('"').strip("'")
        for item in feature_match.group("features").split(",")
    }

    assert {"python", "svg_raster"}.issubset(features)
    assert "build_backend/fullbleed_build_backend.py wheel" in ci
    assert "tests/test_fullbleed_svg_raster_fallback.py" in ci

    include_match = re.search(
        r"(?ms)^include\s*=\s*\[(?P<entries>.*?)^\]",
        cargo_manifest,
    )
    assert include_match, "Cargo.toml must declare a constrained package include list"
    include_entries = [
        item.strip().strip('"').strip("'")
        for item in include_match.group("entries").split(",")
        if item.strip()
    ]
    assert "/LICENSING.md" in include_entries
    assert all(
        entry.startswith("/") for entry in include_entries
    ), "Cargo package include entries must be root-anchored"


@pytest.mark.parametrize(
    ("alias", "canonical"),
    [
        ("a", "pdfa2b"),
        ("ua", "pdfua1"),
        ("vt", "pdfvt1"),
        ("wt1r", "wtpdf1r"),
        ("wt1a", "wtpdf1a"),
        ("pdf/ua-2", "pdfua2"),
    ],
)
def test_cli_manifest_records_requested_alias_and_canonical_profile(
    alias: str, canonical: str, tmp_path: Path
) -> None:
    parser = cli._build_parser()
    args = parser.parse_args(
        [
            "render",
            "--html-str",
            "<html><body><main><p>x</p></main></body></html>",
            "--css-str",
            "body{font-family:Inter}",
            "--pdf-profile",
            alias,
            "--output-intent-icc",
            "data:application/octet-stream;base64,AAAA",
            "--out",
            str(tmp_path / "out.pdf"),
        ]
    )

    manifest = cli._build_manifest(args)

    assert manifest["pdf"]["profile"] == canonical
    assert manifest["pdf"]["profile_requested"] == alias


def test_conformance_harness_covers_every_asserted_standard_profile() -> None:
    assert set(validate_pdf_profiles.DEFAULT_PROFILES) == STANDARD_PROFILE_CHOICES
    assert validate_pdf_profiles.OUTPUT_INTENT_PROFILES == cli.PDF_PROFILES_REQUIRING_OUTPUT_INTENT
    assert validate_pdf_profiles.EMBEDDED_FONT_PROFILES == STANDARD_PROFILE_CHOICES
    assert set(validate_pdf_profiles.VERAPDF_FLAVOURS) == VERAPDF_PROFILE_CHOICES
    assert validate_pdf_profiles.PDFX_PROFILES == {"pdfx4", "pdfvt1"}


@pytest.mark.parametrize("profile", sorted(cli.PDF_PROFILES_REQUIRING_OUTPUT_INTENT))
def test_cli_guards_standard_profiles_requiring_output_intents(profile: str) -> None:
    args = SimpleNamespace(
        pdf_profile=profile,
        output_intent_icc=None,
        output_intent_identifier=None,
        output_intent_info=None,
        output_intent_components=None,
    )

    with pytest.raises(ValueError, match=profile):
        cli._validate_pdf_options(args)


@pytest.mark.parametrize("profile", ["ua", "pdf/ua-2", "wt1r", "wt1a", "tagged"])
def test_cli_does_not_require_output_intents_for_non_output_intent_profiles(
    profile: str,
) -> None:
    args = SimpleNamespace(
        pdf_profile=profile,
        output_intent_icc=None,
        output_intent_identifier=None,
        output_intent_info=None,
        output_intent_components=None,
    )

    cli._validate_pdf_options(args)
