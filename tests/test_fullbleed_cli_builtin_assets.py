"""Bundled assets work offline through the actual CLI and saved PDF output."""
from hashlib import sha256
import json
from pathlib import Path
import socket
import urllib.request

import fullbleed_assets
from fullbleed_cli import assets, cache, cli
from pypdf import PdfReader
import pytest


@pytest.fixture(autouse=True)
def offline_workspace(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(cache, "get_cache_dir", lambda: tmp_path / "isolated-cache")

    def no_network(*args, **kwargs):
        raise AssertionError("bundled assets must not access the network")

    monkeypatch.setattr(socket, "create_connection", no_network)
    monkeypatch.setattr(urllib.request, "urlopen", no_network)
    monkeypatch.setattr(assets, "_download_file", no_network)
    monkeypatch.setattr(assets, "_download_license", no_network)


def invoke(capsys, *args, expected=0):
    assert cli.main(["--json", *map(str, args)]) == expected
    captured = capsys.readouterr()
    assert not captured.err
    return json.loads(captured.out)


@pytest.mark.parametrize("reference", ["@inter", "@INTER", "@inter@4.0"])
def test_inter_cli_matches_explicit_font_and_embeds_it(tmp_path, capsys, reference):
    html = tmp_path / "my invoice.html"
    css = tmp_path / "my invoice.css"
    html.write_text("<h1>Invoice 1042</h1><p>Résumé — EUR 1,234.50</p>", encoding="utf-8")
    css.write_text("@page { size: A5; margin: 15mm } body { font-family: Inter }", encoding="utf-8")
    pdf = tmp_path / "bundled.pdf"
    report = tmp_path / "glyphs.json"
    common = ["render", "--html", html, "--css", css]
    result = invoke(capsys, *common, "--asset", reference, "--out", pdf,
                    "--emit-glyph-report", report, "--fail-on", "missing-glyphs", "--fail-on", "font-subst")
    assert result["ok"]
    explicit = tmp_path / "explicit.pdf"
    invoke(capsys, *common, "--asset", fullbleed_assets.asset_path("fonts/Inter-Variable.ttf"),
           "--asset-name", "Inter", "--out", explicit)
    assert pdf.read_bytes() == explicit.read_bytes()
    reader = PdfReader(pdf)
    assert len(reader.pages) == 1
    text = reader.pages[0].extract_text()
    assert "Invoice 1042" in text and "Résumé" in text and "1,234.50" in text
    fonts = [font.get_object() for font in reader.pages[0]["/Resources"]["/Font"].values()]
    assert fonts and all("Inter" in str(font["/BaseFont"]) for font in fonts)
    for font in fonts:
        face = font["/DescendantFonts"][0].get_object() if "/DescendantFonts" in font else font
        descriptor = face["/FontDescriptor"]
        assert any(key in descriptor for key in ("/FontFile", "/FontFile2", "/FontFile3"))
    assert json.loads(report.read_bytes()) == []


@pytest.mark.parametrize("reference,path,name", [
    ("@bootstrap", "bootstrap.min.css", "bootstrap-5.0.0"),
    ("@bootstrap5", "bootstrap.min.css", "bootstrap-5.0.0"),
    ("@bootstrap5.0.0", "bootstrap.min.css", "bootstrap-5.0.0"),
    ("@bootstrap@5.0.0", "bootstrap.min.css", "bootstrap-5.0.0"),
    ("@bootstrap-icons", "icons/bootstrap-icons.svg", "bootstrap-icons-1.11.3"),
    ("@bootstrapicons", "icons/bootstrap-icons.svg", "bootstrap-icons-1.11.3"),
    ("@bootstrap-icons1.11.3", "icons/bootstrap-icons.svg", "bootstrap-icons-1.11.3"),
    ("@noto-sans", "fonts/NotoSans-Regular.ttf", "NotoSans-Regular"),
    ("@noto-sans-regular", "fonts/NotoSans-Regular.ttf", "NotoSans-Regular"),
])
def test_existing_aliases_render_identically_to_explicit_assets(tmp_path, capsys, reference, path, name):
    actual, explicit = tmp_path / "actual.pdf", tmp_path / "explicit.pdf"
    body = '<h1 class="text-primary">Bundled assets</h1><p>Invoice 1042</p>'
    if path.endswith(".svg"):
        body += '<svg width="24" height="24"><use href="#check-circle" /></svg>'
    common = ["render", "--html-str", body, "--css-str", 'body { font-family: "Noto Sans" }']
    invoke(capsys, *common, "--asset", reference, "--out", actual)
    invoke(capsys, *common, "--asset", fullbleed_assets.asset_path(path), "--asset-name", name, "--out", explicit)
    assert actual.read_bytes() == explicit.read_bytes()
    info = invoke(capsys, "assets", "info", reference)
    assert info["source"] == "builtin"
    assert info["files"][0]["sha256"] == sha256(Path(fullbleed_assets.asset_path(path)).read_bytes()).hexdigest()
    lock = tmp_path / "assets.lock.json"
    invoke(capsys, "assets", "lock", "--add", reference, "--output", lock)
    verified = invoke(capsys, "assets", "verify", reference, "--lock", lock, "--strict")
    assert verified["ok"] and verified["lock_ok"]
    assert verified["name"] == reference.lstrip("@")
    assert verified["sha256"] == info["files"][0]["sha256"]


def test_inter_discovery_install_lock_and_verify_use_bundled_bytes(tmp_path, capsys):
    listed = invoke(capsys, "assets", "list", "--available")
    inter = [row for row in listed["packages"] if row["name"] == "inter"]
    assert len(inter) == 1 and inter[0]["source"] == "builtin"
    assert inter[0]["render_refs"] == ["@inter"] and inter[0]["install_refs"] == ["inter", "@inter"]
    assert not any(row["name"] == "inter" for row in listed["available"])
    info = invoke(capsys, "assets", "info", "inter")
    vendor = tmp_path / "my vendor"
    result = invoke(capsys, "assets", "install", "@inter@4.0", "--vendor", vendor)
    assert result["source"] == "builtin" and result["install_scope"] == "custom_vendor"
    font = Path(fullbleed_assets.asset_path("fonts/Inter-Variable.ttf"))
    assert Path(result["installed_to"]).read_bytes() == font.read_bytes()
    assert result["sha256"] == info["files"][0]["sha256"] == sha256(font.read_bytes()).hexdigest()
    repeated = invoke(capsys, "assets", "install", "inter", "--vendor", vendor)
    assert repeated["sha256"] == result["sha256"]
    license = Path(result["license_file"]).read_bytes()
    assert license == Path(fullbleed_assets.asset_path("licenses/Inter-OFL.txt")).read_bytes()
    assert b"Copyright 2020 The Inter Project Authors" in license and b"PERMISSION & CONDITIONS" in license
    lock = tmp_path / "assets.lock.json"
    invoke(capsys, "assets", "lock", "--add", "@inter@4.0", "--output", lock)
    locked = json.loads(lock.read_bytes())["packages"]
    assert len(locked) == 1 and locked[0]["name"] == "inter"
    assert locked[0]["files"][0]["sha256"] == result["sha256"]
    verified = invoke(capsys, "assets", "verify", "@inter@4.0", "--lock", lock, "--strict")
    assert verified["ok"] and verified["lock_ok"] and verified["name"] == "inter@4.0"
    assert not (tmp_path / "isolated-cache").exists()


@pytest.mark.parametrize("command", ["info", "install", "verify", "lock", "render"])
def test_unavailable_builtin_version_fails_without_writing_files(tmp_path, capsys, command):
    reference = "@inter@999"
    if command == "render":
        args = ["render", "--html-str", "<p>hello</p>", "--asset", reference, "--out", tmp_path / "bad.pdf"]
    elif command == "lock":
        args = ["assets", "lock", "--add", reference, "--output", tmp_path / "bad.lock.json"]
    elif command == "install":
        args = ["assets", command, reference, "--vendor", tmp_path / "vendor"]
    else:
        args = ["assets", command, reference]
    result = invoke(capsys, *args, expected=3)
    assert result["code"] == "CLI_ERROR" and "includes version 4.0" in result["message"]
    assert list(tmp_path.iterdir()) == []


def test_project_install_updates_lock_and_detects_hash_mismatch(tmp_path, capsys):
    lock = tmp_path / "assets.lock.json"
    lock.write_text('{"schema": 1, "packages": []}', encoding="utf-8")
    installed = invoke(capsys, "assets", "install", "inter")
    assert installed["install_scope"] == "project_vendor"
    assert Path(installed["installed_to"]) == tmp_path / "vendor/fonts/Inter-Variable.ttf"
    verified = invoke(capsys, "assets", "verify", "inter", "--lock", lock, "--strict")
    assert verified["lock_ok"]
    data = json.loads(lock.read_bytes())
    data["packages"][0]["files"][0]["sha256"] = "0" * 64
    lock.write_text(json.dumps(data), encoding="utf-8")
    with pytest.raises(SystemExit) as exc:
        cli.main(["--json", "assets", "verify", "inter", "--lock", str(lock), "--strict"])
    assert exc.value.code == 1
    failed = json.loads(capsys.readouterr().out)
    assert not failed["ok"] and not failed["lock_ok"] and failed["violations"]


@pytest.mark.parametrize("name,relative", [("inter", "fonts/Inter-Variable.ttf"), ("bootstrap", "css/bootstrap.min.css")])
def test_install_preserves_existing_different_asset_and_license(tmp_path, capsys, name, relative):
    vendor = tmp_path / "vendor"
    destination = vendor / relative
    destination.parent.mkdir(parents=True)
    destination.write_bytes(b"synthetic user-owned content")
    license = destination.parent / ("LICENSE." + name + ".txt")
    license.write_bytes(b"synthetic user-owned license notice")
    before = {str(path.relative_to(vendor)):path.read_bytes() for path in vendor.rglob("*") if path.is_file()}
    result = invoke(capsys, "assets", "install", name, "--vendor", vendor, expected=3)
    assert "existing asset has different contents" in result["message"]
    after = {str(path.relative_to(vendor)):path.read_bytes() for path in vendor.rglob("*") if path.is_file()}
    assert before == after


def test_unknown_render_asset_error_lists_usable_references(tmp_path, capsys):
    result = invoke(capsys, "render", "--html-str", "<p>hello</p>", "--asset", "@missing",
                    "--out", tmp_path / "missing.pdf", expected=3)
    assert "@inter" in result["message"] and "@noto-sans" in result["message"]
    assert not (tmp_path / "missing.pdf").exists()
