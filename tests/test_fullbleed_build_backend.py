from __future__ import annotations

import base64
import csv
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import zipfile

import pytest


REPO_ROOT = Path(__file__).resolve().parents[1]
BACKEND_PATH = REPO_ROOT / "build_backend" / "fullbleed_build_backend.py"


def _load_backend():
    spec = importlib.util.spec_from_file_location(
        "fullbleed_build_backend_test", BACKEND_PATH
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.fixture(scope="module")
def backend():
    return _load_backend()


def test_backend_imports_without_site_packages_and_declares_no_build_requirements() -> (
    None
):
    script = f"""
import importlib.util
spec = importlib.util.spec_from_file_location('fullbleed_build_backend', {str(BACKEND_PATH)!r})
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
assert module.get_requires_for_build_wheel() == []
assert module.get_requires_for_build_editable() == []
assert module.get_requires_for_build_sdist() == []
"""
    subprocess.run([sys.executable, "-S", "-c", script], check=True, cwd=REPO_ROOT)


def test_pyproject_selects_in_tree_backend_with_no_python_requirements(backend) -> None:
    configuration = backend._load_pyproject()
    build_system = configuration["build-system"]
    assert build_system == {
        "requires": [],
        "build-backend": "fullbleed_build_backend",
        "backend-path": ["build_backend"],
    }
    assert configuration["tool"]["fullbleed-build"]["features"] == [
        "python",
        "svg_raster",
    ]
    assert "maturin" not in configuration.get("tool", {})


def test_python_310_toml_fallback_reads_complete_project_configuration(backend) -> None:
    source = (REPO_ROOT / "pyproject.toml").read_text(encoding="utf-8")
    parsed = backend._fallback_toml_loads(source)
    assert parsed["project"]["name"] == "fullbleed"
    assert parsed["project"]["license-files"][-1] == "FONT_LICENSE_AUDIT.json"
    assert parsed["project"]["optional-dependencies"]["api"] == []
    assert parsed["tool"]["fullbleed-build"]["strip"] is True
    assert (
        parsed["tool"]["fullbleed-build"]["wheel-resources"][
            "fullbleed/specs/fullbleed.audit_registry.v1.yaml"
        ]
        == "docs/specs/fullbleed.audit_registry.v1.yaml"
    )


@pytest.mark.parametrize(
    ("target", "compatibility", "expected"),
    [
        ("x86_64-pc-windows-msvc", "off", "win_amd64"),
        ("i686-pc-windows-msvc", "off", "win32"),
        ("aarch64-pc-windows-msvc", "off", "win_arm64"),
        (
            "x86_64-unknown-linux-gnu",
            "manylinux2014",
            "manylinux_2_17_x86_64.manylinux2014_x86_64",
        ),
        (
            "armv7-unknown-linux-gnueabihf",
            "2014",
            "manylinux_2_17_armv7l.manylinux2014_armv7l",
        ),
        ("aarch64-unknown-linux-musl", "musllinux_1_2", "musllinux_1_2_aarch64"),
        ("s390x-unknown-linux-gnu", "off", "linux_s390x"),
        ("powerpc64le-unknown-linux-gnu", "off", "linux_ppc64le"),
    ],
)
def test_platform_tags_cover_release_matrix(
    backend, target, compatibility, expected
) -> None:
    assert (
        backend._platform_tag(
            {"target": target, "compatibility": compatibility, "platform_tag": None}
        )
        == expected
    )


def test_macos_platform_tags_use_supported_deployment_floors(
    backend, monkeypatch
) -> None:
    monkeypatch.delenv("MACOSX_DEPLOYMENT_TARGET", raising=False)
    assert (
        backend._platform_tag(
            {
                "target": "x86_64-apple-darwin",
                "compatibility": "off",
                "platform_tag": None,
            }
        )
        == "macosx_10_12_x86_64"
    )
    assert (
        backend._platform_tag(
            {
                "target": "aarch64-apple-darwin",
                "compatibility": "off",
                "platform_tag": None,
            }
        )
        == "macosx_11_0_arm64"
    )


def test_wheel_writer_is_sorted_reproducible_and_record_is_valid(
    backend, tmp_path: Path, monkeypatch
) -> None:
    monkeypatch.delenv("SOURCE_DATE_EPOCH", raising=False)
    dist_info = backend._dist_info_name()
    entries = {
        "fullbleed/data.bin": b"\x00\x01payload",
        f"{dist_info}/METADATA": b"Metadata-Version: 2.4\n",
        f"{dist_info}/WHEEL": b"Wheel-Version: 1.0\n",
    }
    first = tmp_path / "first.whl"
    second = tmp_path / "second.whl"
    backend._write_wheel(first, entries)
    backend._write_wheel(second, entries)
    assert first.read_bytes() == second.read_bytes()

    with zipfile.ZipFile(first) as archive:
        names = archive.namelist()
        assert names == sorted(names)
        assert all(
            archive.getinfo(name).date_time == (1980, 1, 1, 0, 0, 0) for name in names
        )
        record_name = f"{dist_info}/RECORD"
        rows = csv.reader(io.StringIO(archive.read(record_name).decode("utf-8")))
        records = {name: (digest, size) for name, digest, size in rows}
        assert set(records) == set(names)
        for name in names:
            digest, size = records[name]
            if name == record_name:
                assert (digest, size) == ("", "")
                continue
            payload = archive.read(name)
            encoded = base64.urlsafe_b64encode(hashlib.sha256(payload).digest()).rstrip(
                b"="
            )
            assert digest == f"sha256={encoded.decode('ascii')}"
            assert int(size) == len(payload)


def test_cyclonedx_sbom_is_deterministic_and_covers_optional_build_graph(
    backend, monkeypatch
) -> None:
    monkeypatch.delenv("SOURCE_DATE_EPOCH", raising=False)
    first = backend._sbom_bytes(["python", "svg_raster"])
    second = backend._sbom_bytes(["python", "svg_raster"])
    assert first == second
    document = json.loads(first)
    assert document["bomFormat"] == "CycloneDX"
    assert document["specVersion"] == "1.5"
    assert document["metadata"]["timestamp"] == "1980-01-01T00:00:00Z"
    assert document["metadata"]["component"]["name"] == "fullbleed"
    components = {component["name"]: component for component in document["components"]}
    assert set(components) == {
        "fullbleed_audit_contract",
        "rustc-hash",
        "subsetter",
        "unicode-bidi",
    }
    assert "pyo3" not in components
    assert "kuchiki" not in components
    assert "lopdf" not in components
    assert "resvg" not in components
    assert "rustybuzz" not in components
    assert "tiny-skia" not in components
    assert "time" not in components
    assert "ttf-parser" not in components
    audit_component = components["fullbleed_audit_contract"]
    assert audit_component["hashes"][0]["alg"] == "SHA-256"
    assert len(audit_component["hashes"][0]["content"]) == 64
    assert audit_component["properties"] == [
        {
            "name": "fullbleed:hash-subject",
            "value": "cargo-source-tree-v1",
        }
    ]
    assert len(document["dependencies"]) == len(document["components"]) + 1
    dependencies = {
        row["ref"]: set(row["dependsOn"]) for row in document["dependencies"]
    }
    assert dependencies["pkg:cargo/fullbleed@2.2.3"] == {
        "pkg:cargo/fullbleed_audit_contract@0.1.3",
        "pkg:cargo/subsetter@0.2.6",
        "pkg:cargo/unicode-bidi@0.3.18",
    }
    assert dependencies["pkg:cargo/subsetter@0.2.6"] == {"pkg:cargo/rustc-hash@2.1.3"}


def test_api_compatibility_extra_has_no_third_party_requirements(backend) -> None:
    metadata = backend._metadata_bytes().decode("utf-8")
    headers = metadata.split("\n\n", 1)[0].splitlines()
    assert "Provides-Extra: api" in headers
    assert not any(header.startswith("Requires-Dist:") for header in headers)


def test_python_and_sdist_file_sets_preserve_assets_without_build_caches(
    backend,
) -> None:
    wheel_files = set(backend._python_entries())
    assert "fullbleed_assets/fonts/Inter-Variable.ttf" in wheel_files
    assert (
        "fullbleed_cli/scaffold_templates/new/accessible/output/.gitignore"
        in wheel_files
    )
    assert (
        "fullbleed_cli/scaffold_templates/new/accessible/output/.gitkeep" in wheel_files
    )
    for name in (
        "fullbleed/specs/fullbleed.audit_registry.v1.yaml",
        "fullbleed/specs/fullbleed.a11y.verify.v1.schema.json",
        "fullbleed/specs/fullbleed.pmr.v1.schema.json",
        "fullbleed/specs/wcag20aa_registry.v1.yaml",
        "fullbleed/specs/section508_html_registry.v1.yaml",
    ):
        assert name in wheel_files

    sdist_files = {
        path.relative_to(REPO_ROOT).as_posix() for path in backend._sdist_files()
    }
    assert "build_backend/fullbleed_build_backend.py" in sdist_files
    assert (
        "crates/fullbleed_audit_contract/specs/fullbleed.audit_registry.v1.yaml"
        in sdist_files
    )
    assert "src/jpeg_native.rs" in sdist_files
    assert "docs/specs/fullbleed.a11y.verify.v1.schema.json" in sdist_files
    assert not any("/__pycache__/" in f"/{name}/" for name in sdist_files)
    assert not any("/target/" in f"/{name}/" for name in sdist_files)
    assert not any(
        Path(name).name.startswith("_fullbleed.")
        and Path(name).suffix.lower() in {".dylib", ".pyd", ".so"}
        for name in sdist_files
    )


def test_generated_python_extension_detection_is_platform_independent(backend) -> None:
    for name in (
        "_fullbleed.pyd",
        "_fullbleed.cp311-win_amd64.pyd",
        "_fullbleed.abi3.so",
        "_fullbleed.dylib",
    ):
        assert backend._is_generated_python_extension(Path(name))
    for name in ("_fullbleed.py", "vendor.pyd", "fullbleed.so"):
        assert not backend._is_generated_python_extension(Path(name))


def test_editable_wheel_bootstrap_maps_source_and_native_extension(
    backend, tmp_path: Path
) -> None:
    artifact = tmp_path / "fullbleed.dll"
    artifact.write_bytes(b"native-extension")
    options = {
        "target": "x86_64-pc-windows-msvc",
        "compatibility": "off",
        "platform_tag": None,
    }
    entries = backend._editable_entries(artifact, options)
    assert entries["_fullbleed_editable/_fullbleed.pyd"] == b"native-extension"
    assert entries["_fullbleed_editable.pth"].startswith(b"import _fullbleed_editable")
    bootstrap = entries["_fullbleed_editable.py"].decode("utf-8")
    assert repr(str((REPO_ROOT / "python").resolve())) in bootstrap
    assert 'fullname == "fullbleed._fullbleed"' in bootstrap


def test_msvc_build_uses_reproducible_linker_and_source_epoch(
    backend, tmp_path: Path, monkeypatch
) -> None:
    artifact = tmp_path / "fullbleed.dll"
    artifact.write_bytes(b"native")
    observed = {}

    class FakeProcess:
        stdout = iter(
            [
                json.dumps(
                    {
                        "reason": "compiler-artifact",
                        "target": {
                            "name": "fullbleed",
                            "crate_types": ["rlib", "cdylib"],
                        },
                        "filenames": [str(artifact)],
                    }
                )
            ]
        )

        @staticmethod
        def wait():
            return 0

    def fake_popen(command, **kwargs):
        observed["command"] = command
        observed["environment"] = kwargs["env"]
        return FakeProcess()

    monkeypatch.delenv("SOURCE_DATE_EPOCH", raising=False)
    monkeypatch.setattr(backend.subprocess, "Popen", fake_popen)
    result = backend._build_extension(
        {
            "locked": True,
            "release": True,
            "strip": True,
            "features": ["python", "svg_raster"],
            "target": "x86_64-pc-windows-msvc",
            "cargo_extra_args": None,
        }
    )
    assert result == artifact
    environment = observed["environment"]
    assert environment["SOURCE_DATE_EPOCH"] == "315532800"
    assert environment["CARGO_PROFILE_RELEASE_CODEGEN_UNITS"] == "1"
    assert (
        "-C link-arg=/Brepro"
        in environment["CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS"]
    )
    assert environment["CARGO_PROFILE_RELEASE_STRIP"] == "symbols"
    assert "--message-format=json-render-diagnostics" in observed["command"]


def test_sdist_archive_is_reproducible_and_pep517_complete(
    backend, tmp_path: Path, monkeypatch
) -> None:
    monkeypatch.delenv("SOURCE_DATE_EPOCH", raising=False)
    first = tmp_path / "first"
    second = tmp_path / "second"
    filename = backend.build_sdist(str(first))
    assert backend.build_sdist(str(second)) == filename
    assert (first / filename).read_bytes() == (second / filename).read_bytes()
    with tarfile.open(first / filename, "r:gz") as archive:
        names = set(archive.getnames())
    root = f"fullbleed-{backend._project()['project']['version']}"
    assert f"{root}/PKG-INFO" in names
    assert f"{root}/pyproject.toml" in names
    assert f"{root}/build.rs" in names
    assert f"{root}/build_backend/fullbleed_build_backend.py" in names
    assert not any("/target/" in f"/{name}/" for name in names)
