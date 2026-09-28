"""The installed native catalog is authoritative for CLI and Python parsing."""
import fullbleed
import pytest
from fullbleed_cli import cli


def test_native_profile_catalog_matches_cli_and_has_unique_aliases():
    catalog = fullbleed.pdf_profile_catalog()
    assert len(catalog) == 19
    assert [p["name"] for p in catalog] == cli.PDF_PROFILE_CHOICES
    assert cli._capabilities_payload()["pdf_profile_catalog"] == catalog
    assert cli._agent_contract_payload()["capabilities"]["pdf_profile_catalog"] == catalog
    assert len([p for p in catalog if p["requires_output_intent"]]) == 13
    aliases = [alias for p in catalog for alias in p["aliases"]]
    assert len(aliases) == len(set(aliases))
    for profile in catalog:
        assert profile["fixed_bindings_supported"] is not profile["emits_tagged_structure"]
        for alias in profile["aliases"]:
            assert cli._normalize_pdf_profile(alias) == profile["name"]
            try:
                fullbleed.PdfEngine(pdf_profile=f" {alias.upper()} ")
            except ValueError as error:
                # ICC-required targets can fail construction, but never name parsing.
                assert profile["requires_output_intent"], str(error)
                assert "output intent" in str(error).lower() or "output_intent" in str(error).lower()


def test_profile_catalog_is_detached_and_blank_defaults_remain_compatible():
    first = fullbleed.pdf_profile_catalog()
    first[0]["name"] = "tampered"
    assert fullbleed.pdf_profile_catalog()[0]["name"] == "none"
    for blank in [None, "", " \t", "none"]:
        fullbleed.PdfEngine(pdf_profile=blank)
    with pytest.raises(ValueError, match="Invalid pdf_profile"):
        fullbleed.PdfEngine(pdf_profile="pdfua99")
