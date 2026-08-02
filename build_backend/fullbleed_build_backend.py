"""Dependency-free PEP 517/660 backend for FullBleed's Rust extension.

The backend deliberately uses only the Python standard library and Cargo.  It
is kept in the source distribution through ``backend-path`` so pip can build a
wheel from a clean checkout or sdist without downloading a Python build tool.
"""

from __future__ import annotations

import argparse
import ast
import base64
import datetime as _datetime
import gzip
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shlex
import subprocess
import sys
import sysconfig
import tarfile
import urllib.parse
import uuid
import zipfile


_ROOT = Path(__file__).resolve().parent.parent
_PYPROJECT = _ROOT / "pyproject.toml"
_DEFAULT_EPOCH = 315532800  # 1980-01-01T00:00:00Z, the ZIP format minimum.
_NATIVE_SUFFIXES = {".dll", ".dylib", ".so"}
_PYTHON_EXTENSION_SUFFIXES = {".dylib", ".pyd", ".so"}


class BuildBackendError(RuntimeError):
    """A user-facing packaging failure."""


def _is_generated_python_extension(path: Path) -> bool:
    return (
        path.name.startswith("_fullbleed.")
        and path.suffix.lower() in _PYTHON_EXTENSION_SUFFIXES
    )


def _without_comment(line: str) -> str:
    quote = ""
    escaped = False
    for index, char in enumerate(line):
        if escaped:
            escaped = False
            continue
        if quote and char == "\\" and quote == '"':
            escaped = True
            continue
        if char in {'"', "'"}:
            if quote == char:
                quote = ""
            elif not quote:
                quote = char
            continue
        if char == "#" and not quote:
            return line[:index]
    return line


def _value_is_complete(value: str) -> bool:
    quote = ""
    escaped = False
    square = 0
    curly = 0
    for char in value:
        if escaped:
            escaped = False
            continue
        if quote and char == "\\" and quote == '"':
            escaped = True
            continue
        if char in {'"', "'"}:
            if quote == char:
                quote = ""
            elif not quote:
                quote = char
            continue
        if quote:
            continue
        square += (char == "[") - (char == "]")
        curly += (char == "{") - (char == "}")
    return not quote and square == 0 and curly == 0


def _parse_toml_value(value: str):
    value = value.strip()
    if value in {"true", "false"}:
        return value == "true"
    if value.startswith(('"', "'", "[", "{")):
        try:
            return ast.literal_eval(value)
        except (SyntaxError, ValueError) as error:
            raise BuildBackendError(
                f"unsupported pyproject.toml value: {value}"
            ) from error
    try:
        return int(value.replace("_", ""))
    except ValueError as error:
        raise BuildBackendError(f"unsupported pyproject.toml value: {value}") from error


def _fallback_toml_loads(text: str) -> dict:
    """Parse the deliberately small TOML subset used by this pyproject.

    Python 3.10 predates :mod:`tomllib`.  The fallback supports tables, basic
    strings, booleans, integers, and arrays/inline tables composed of those
    values.  That is sufficient for all build and project metadata here.
    """

    root: dict = {}
    table = root
    lines = iter(text.splitlines())
    for raw_line in lines:
        line = _without_comment(raw_line).strip()
        if not line:
            continue
        if line.startswith("[") and line.endswith("]"):
            names = [part.strip() for part in line[1:-1].split(".")]
            if not all(names):
                raise BuildBackendError(f"invalid TOML table: {line}")
            table = root
            for name in names:
                table = table.setdefault(name, {})
            continue
        if "=" not in line:
            raise BuildBackendError(f"invalid pyproject.toml line: {line}")
        key, value = line.split("=", 1)
        key = key.strip().strip('"').strip("'")
        while not _value_is_complete(value):
            try:
                value += "\n" + _without_comment(next(lines)).strip()
            except StopIteration as error:
                raise BuildBackendError(f"unterminated TOML value for {key}") from error
        table[key] = _parse_toml_value(value)
    return root


def _load_pyproject() -> dict:
    text = _PYPROJECT.read_text(encoding="utf-8")
    try:
        import tomllib  # Python 3.11+
    except ImportError:
        return _fallback_toml_loads(text)
    return tomllib.loads(text)


def _project() -> dict:
    data = _load_pyproject()
    project = data.get("project")
    tool = data.get("tool", {}).get("fullbleed-build")
    if not isinstance(project, dict) or not isinstance(tool, dict):
        raise BuildBackendError(
            "pyproject.toml is missing project or tool.fullbleed-build"
        )
    return {"project": project, "tool": tool}


def _distribution_name(name: str) -> str:
    return re.sub(r"[-_.]+", "_", name)


def _dist_info_name() -> str:
    project = _project()["project"]
    return f"{_distribution_name(project['name'])}-{project['version']}.dist-info"


def _source_date_epoch() -> int:
    raw = os.environ.get("SOURCE_DATE_EPOCH")
    if raw is None:
        return _DEFAULT_EPOCH
    try:
        value = int(raw)
    except ValueError as error:
        raise BuildBackendError("SOURCE_DATE_EPOCH must be an integer") from error
    return max(value, _DEFAULT_EPOCH)


def _iso_timestamp() -> str:
    stamp = _datetime.datetime.fromtimestamp(
        _source_date_epoch(), tz=_datetime.timezone.utc
    )
    return stamp.isoformat(timespec="seconds").replace("+00:00", "Z")


def _metadata_bytes() -> bytes:
    project = _project()["project"]
    readme_path = _ROOT / str(project.get("readme", "README.md"))
    lines = [
        "Metadata-Version: 2.4",
        f"Name: {project['name']}",
        f"Version: {project['version']}",
    ]
    for classifier in project.get("classifiers", []):
        lines.append(f"Classifier: {classifier}")
    for extra, requirements in project.get("optional-dependencies", {}).items():
        for requirement in requirements:
            lines.append(f"Requires-Dist: {requirement} ; extra == '{extra}'")
        lines.append(f"Provides-Extra: {extra}")
    for license_file in project.get("license-files", []):
        lines.append(f"License-File: {license_file}")
    if project.get("description"):
        lines.append(f"Summary: {project['description']}")
    if project.get("keywords"):
        lines.append("Keywords: " + ",".join(project["keywords"]))
    homepage = project.get("urls", {}).get("Homepage")
    if homepage:
        lines.append(f"Home-Page: {homepage}")
    license_expression = project.get("license")
    if isinstance(license_expression, str):
        lines.append(f"License-Expression: {license_expression}")
    if project.get("requires-python"):
        lines.append(f"Requires-Python: {project['requires-python']}")
    lines.append("Description-Content-Type: text/markdown; charset=UTF-8; variant=GFM")
    for label, url in sorted(project.get("urls", {}).items()):
        lines.append(f"Project-URL: {label}, {url}")
    description = readme_path.read_text(encoding="utf-8")
    return ("\n".join(lines) + "\n\n" + description.rstrip() + "\n").encode("utf-8")


def _entry_points_bytes() -> bytes:
    scripts = _project()["project"].get("scripts", {})
    if not scripts:
        return b""
    lines = ["[console_scripts]"]
    lines.extend(f"{name}={target}" for name, target in sorted(scripts.items()))
    return ("\n".join(lines) + "\n").encode("utf-8")


def _setting(config_settings: dict | None, *names: str):
    config_settings = config_settings or {}
    for name in names:
        if name in config_settings:
            value = config_settings[name]
            if isinstance(value, list):
                return value[-1] if value else ""
            return value
    return None


def _boolean(value, default: bool) -> bool:
    if value is None:
        return default
    if value is True or value == "":
        return True
    if value is False:
        return False
    normalized = str(value).strip().lower()
    if normalized in {"1", "true", "yes", "on"}:
        return True
    if normalized in {"0", "false", "no", "off"}:
        return False
    raise BuildBackendError(f"invalid boolean build setting: {value}")


def _build_options(config_settings: dict | None) -> dict:
    tool = _project()["tool"]
    features_setting = _setting(config_settings, "features", "--features")
    if features_setting is None:
        features = list(tool.get("features", []))
    else:
        features = [
            part.strip() for part in str(features_setting).split(",") if part.strip()
        ]
    target = _setting(config_settings, "target", "--target")
    target = (
        target
        or os.environ.get("FULLBLEED_BUILD_TARGET")
        or os.environ.get("CARGO_BUILD_TARGET")
    )
    compatibility = _setting(config_settings, "compatibility", "--compatibility")
    compatibility = compatibility or os.environ.get(
        "FULLBLEED_WHEEL_COMPATIBILITY", "off"
    )
    return {
        "release": _boolean(_setting(config_settings, "release", "--release"), True),
        "locked": _boolean(_setting(config_settings, "locked", "--locked"), True),
        "strip": _boolean(
            _setting(config_settings, "strip", "--strip"), bool(tool.get("strip", True))
        ),
        "features": features,
        "target": _resolve_target(str(target), str(compatibility)) if target else None,
        "compatibility": str(compatibility),
        "platform_tag": _setting(config_settings, "plat-name", "--plat-name")
        or os.environ.get("FULLBLEED_WHEEL_PLATFORM_TAG"),
        "cargo_extra_args": _setting(
            config_settings, "cargo-extra-args", "--cargo-extra-args"
        ),
    }


def _resolve_target(target: str, compatibility: str) -> str:
    normalized = target.strip().lower()
    if "-" in normalized and normalized not in {"armv7", "x86-64"}:
        return target
    aliases = {
        "x64": "x86_64",
        "x86-64": "x86_64",
        "amd64": "x86_64",
        "x86": "i686",
        "arm64": "aarch64",
        "armv7l": "armv7",
    }
    architecture = aliases.get(normalized, normalized)
    if sys.platform == "win32":
        triples = {
            "x86_64": "x86_64-pc-windows-msvc",
            "i686": "i686-pc-windows-msvc",
            "aarch64": "aarch64-pc-windows-msvc",
        }
    elif sys.platform == "darwin":
        triples = {
            "x86_64": "x86_64-apple-darwin",
            "aarch64": "aarch64-apple-darwin",
        }
    elif compatibility.startswith("musllinux"):
        triples = {
            "x86_64": "x86_64-unknown-linux-musl",
            "i686": "i686-unknown-linux-musl",
            "aarch64": "aarch64-unknown-linux-musl",
            "armv7": "armv7-unknown-linux-musleabihf",
        }
    else:
        triples = {
            "x86_64": "x86_64-unknown-linux-gnu",
            "i686": "i686-unknown-linux-gnu",
            "aarch64": "aarch64-unknown-linux-gnu",
            "armv7": "armv7-unknown-linux-gnueabihf",
            "s390x": "s390x-unknown-linux-gnu",
            "ppc64le": "powerpc64le-unknown-linux-gnu",
        }
    try:
        return triples[architecture]
    except KeyError as error:
        raise BuildBackendError(f"unsupported build target alias: {target}") from error


def _target_arch(target: str) -> str:
    normalized = target.lower()
    if normalized.startswith(("x86_64", "x64")):
        return "x86_64"
    if normalized.startswith(("i686", "i586", "x86-")) or normalized == "x86":
        return "i686"
    if normalized.startswith(("aarch64", "arm64")):
        return "aarch64"
    if normalized.startswith(("armv7", "armv7l")):
        return "armv7l"
    if normalized.startswith("s390x"):
        return "s390x"
    if normalized.startswith(("powerpc64le", "ppc64le")):
        return "ppc64le"
    raise BuildBackendError(f"cannot derive a wheel architecture from target: {target}")


def _platform_tag(options: dict) -> str:
    explicit = options.get("platform_tag")
    if explicit:
        explicit = str(explicit).replace("-", "_")
        if not re.fullmatch(r"[A-Za-z0-9_.]+", explicit):
            raise BuildBackendError(f"invalid wheel platform tag: {explicit}")
        return explicit

    target = options.get("target")
    compatibility = options.get("compatibility", "off")
    if not target:
        return sysconfig.get_platform().replace("-", "_").replace(".", "_")

    arch = _target_arch(target)
    lowered = target.lower()
    if "windows" in lowered:
        return {"x86_64": "win_amd64", "i686": "win32", "aarch64": "win_arm64"}[arch]
    if "darwin" in lowered or "apple" in lowered:
        default = "11.0" if arch == "aarch64" else "10.12"
        deployment = os.environ.get("MACOSX_DEPLOYMENT_TARGET", default)
        if not re.fullmatch(r"\d+(?:\.\d+){1,2}", deployment):
            raise BuildBackendError(f"invalid MACOSX_DEPLOYMENT_TARGET: {deployment}")
        mac_arch = "arm64" if arch == "aarch64" else arch
        return f"macosx_{deployment.replace('.', '_')}_{mac_arch}"
    if compatibility in {"2014", "manylinux2014", "manylinux_2_17"}:
        return f"manylinux_2_17_{arch}.manylinux2014_{arch}"
    if compatibility.startswith("musllinux_"):
        version = compatibility[len("musllinux_") :]
        if not re.fullmatch(r"\d+_\d+", version):
            raise BuildBackendError(f"invalid musllinux compatibility: {compatibility}")
        return f"musllinux_{version}_{arch}"
    if compatibility not in {"off", "native", "linux"}:
        raise BuildBackendError(f"unsupported wheel compatibility: {compatibility}")
    return f"linux_{arch}"


def _wheel_tag(options: dict) -> str:
    abi_tag = str(_project()["tool"].get("abi-tag", "cp310-abi3"))
    if not re.fullmatch(r"[A-Za-z0-9_.]+-[A-Za-z0-9_.]+", abi_tag):
        raise BuildBackendError(f"invalid Python ABI tag: {abi_tag}")
    return f"{abi_tag}-{_platform_tag(options)}"


def _wheel_bytes(tag: str) -> bytes:
    return (
        "Wheel-Version: 1.0\n"
        "Generator: fullbleed-build 1\n"
        "Root-Is-Purelib: false\n"
        f"Tag: {tag}\n"
    ).encode("utf-8")


def _cargo_command() -> str:
    return os.environ.get("CARGO", "cargo")


def _cargo_metadata(features: list[str]) -> dict:
    command = [
        _cargo_command(),
        "metadata",
        "--format-version",
        "1",
        "--locked",
        "--manifest-path",
        str(_ROOT / "Cargo.toml"),
    ]
    if features:
        command.extend(["--features", ",".join(features)])
    completed = subprocess.run(
        command,
        cwd=_ROOT,
        text=True,
        encoding="utf-8",
        capture_output=True,
        check=False,
    )
    if completed.returncode:
        if completed.stderr:
            sys.stderr.write(completed.stderr)
        raise BuildBackendError("cargo metadata failed")
    try:
        return json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise BuildBackendError("cargo metadata returned invalid JSON") from error


def _cargo_lock_checksums() -> dict[tuple[str, str, str], str]:
    checksums: dict[tuple[str, str, str], str] = {}
    text = (_ROOT / "Cargo.lock").read_text(encoding="utf-8")
    for block in text.split("[[package]]")[1:]:
        values = {}
        for key in ("name", "version", "source", "checksum"):
            match = re.search(rf'(?m)^{key}\s*=\s*("(?:[^"\\]|\\.)*")\s*$', block)
            if match:
                values[key] = json.loads(match.group(1))
        if "name" in values and "version" in values and "checksum" in values:
            checksums[(values["name"], values["version"], values.get("source", ""))] = (
                values["checksum"]
            )
    return checksums


def _package_purl(package: dict) -> str:
    name = urllib.parse.quote(package["name"], safe="._-")
    version = urllib.parse.quote(package["version"], safe="._-+")
    return f"pkg:cargo/{name}@{version}"


def _local_package_source_hash(package: dict) -> str | None:
    manifest = Path(str(package.get("manifest_path") or "")).resolve()
    try:
        package_root = manifest.parent
        package_root.relative_to(_ROOT.resolve())
    except (OSError, ValueError):
        return None
    if not manifest.is_file():
        return None

    excluded_directories = {".git", ".pytest_cache", "__pycache__", "target"}
    files = []
    for path in package_root.rglob("*"):
        relative = path.relative_to(package_root)
        if any(part in excluded_directories for part in relative.parts):
            continue
        if path.is_file() and not path.is_symlink() and path.suffix != ".pyc":
            files.append((relative.as_posix(), path))

    digest = hashlib.sha256(b"fullbleed-cargo-source-tree-v1\0")
    for relative, path in sorted(files):
        name = relative.encode("utf-8")
        payload = path.read_bytes()
        digest.update(len(name).to_bytes(8, "big"))
        digest.update(name)
        digest.update(len(payload).to_bytes(8, "big"))
        digest.update(payload)
    return digest.hexdigest()


def _sbom_bytes(features: list[str]) -> bytes:
    metadata = _cargo_metadata(features)
    packages = {package["id"]: package for package in metadata["packages"]}
    root_id = metadata.get("resolve", {}).get("root")
    if not root_id or root_id not in packages:
        raise BuildBackendError("cargo metadata did not identify the root package")
    checksums = _cargo_lock_checksums()

    purl_counts: dict[str, int] = {}
    for package in packages.values():
        purl = _package_purl(package)
        purl_counts[purl] = purl_counts.get(purl, 0) + 1
    references = {}
    for package_id, package in packages.items():
        purl = _package_purl(package)
        references[package_id] = (
            purl
            if purl_counts[purl] == 1
            else f"{purl}#{hashlib.sha256(package_id.encode()).hexdigest()[:16]}"
        )

    def component(package_id: str) -> dict:
        package = packages[package_id]
        item = {
            "type": "library",
            "bom-ref": references[package_id],
            "name": package["name"],
            "version": package["version"],
            "scope": "required",
            "purl": _package_purl(package),
        }
        for key in ("description",):
            if package.get(key):
                item[key] = package[key]
        if package.get("authors"):
            item["author"] = ", ".join(package["authors"])
        if package.get("license"):
            item["licenses"] = [{"expression": package["license"]}]
        source = package.get("source") or ""
        checksum = checksums.get((package["name"], package["version"], source))
        if checksum and re.fullmatch(r"[0-9a-fA-F]{64}", checksum):
            item["hashes"] = [{"alg": "SHA-256", "content": checksum.lower()}]
        elif package_id != root_id and not source:
            source_hash = _local_package_source_hash(package)
            if source_hash:
                item["hashes"] = [{"alg": "SHA-256", "content": source_hash}]
                item["properties"] = [
                    {
                        "name": "fullbleed:hash-subject",
                        "value": "cargo-source-tree-v1",
                    }
                ]
        external = []
        for reference_type, key in (
            ("documentation", "documentation"),
            ("website", "homepage"),
            ("vcs", "repository"),
        ):
            value = package.get(key)
            if value and not any(entry["url"] == value for entry in external):
                external.append({"type": reference_type, "url": value})
        if external:
            item["externalReferences"] = external
        return item

    dependencies = []
    for node in metadata.get("resolve", {}).get("nodes", []):
        if node["id"] not in references:
            continue
        depends_on = sorted(
            references[dependency]
            for dependency in node.get("dependencies", [])
            if dependency in references
        )
        dependencies.append({"ref": references[node["id"]], "dependsOn": depends_on})
    dependencies.sort(key=lambda item: item["ref"])

    root_component = component(root_id)
    root_component["type"] = "application"
    lock_hash = hashlib.sha256((_ROOT / "Cargo.lock").read_bytes()).hexdigest()
    serial = uuid.uuid5(uuid.NAMESPACE_URL, f"{references[root_id]}:{lock_hash}")
    document = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": f"urn:uuid:{serial}",
        "version": 1,
        "metadata": {
            "timestamp": _iso_timestamp(),
            "tools": {
                "components": [
                    {
                        "type": "application",
                        "name": "fullbleed-build",
                        "version": "1",
                    }
                ]
            },
            "component": root_component,
        },
        "components": [
            component(package_id)
            for package_id in sorted(
                (item for item in packages if item != root_id),
                key=lambda item: references[item],
            )
        ],
        "dependencies": dependencies,
    }
    return (
        json.dumps(document, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
        + "\n"
    ).encode("utf-8")


def _dist_info_entries(tag: str, features: list[str]) -> dict[str, bytes]:
    project = _project()["project"]
    dist_info = _dist_info_name()
    entries = {
        f"{dist_info}/METADATA": _metadata_bytes(),
        f"{dist_info}/WHEEL": _wheel_bytes(tag),
        f"{dist_info}/entry_points.txt": _entry_points_bytes(),
        f"{dist_info}/sboms/{project['name']}.cyclonedx.json": _sbom_bytes(features),
    }
    for license_file in project.get("license-files", []):
        entries[f"{dist_info}/licenses/{license_file}"] = (
            _ROOT / license_file
        ).read_bytes()
    return entries


def _prepared_entries(
    metadata_directory: str | os.PathLike, tag: str
) -> dict[str, bytes]:
    base = Path(metadata_directory)
    if base.name.endswith(".dist-info"):
        dist_info = base
    else:
        dist_info = base / _dist_info_name()
    if not dist_info.is_dir():
        raise BuildBackendError(f"prepared metadata directory is missing: {dist_info}")
    entries = {}
    for path in sorted(dist_info.rglob("*")):
        if path.is_file() and path.name != "RECORD":
            entries[f"{dist_info.name}/{path.relative_to(dist_info).as_posix()}"] = (
                path.read_bytes()
            )
    wheel = entries.get(f"{dist_info.name}/WHEEL", b"").decode(
        "utf-8", errors="replace"
    )
    if f"Tag: {tag}\n" not in wheel:
        raise BuildBackendError("prepared metadata wheel tag does not match this build")
    return entries


def _python_entries() -> dict[str, bytes]:
    tool = _project()["tool"]
    source = _ROOT / str(tool.get("python-source", "python"))
    entries = {}
    for package in tool.get("python-packages", []):
        package_root = source / package
        if not package_root.is_dir():
            raise BuildBackendError(
                f"Python package directory is missing: {package_root}"
            )
        for path in sorted(package_root.rglob("*")):
            if (
                not path.is_file()
                or "__pycache__" in path.parts
                or path.suffix == ".pyc"
            ):
                continue
            if _is_generated_python_extension(path):
                continue
            entries[path.relative_to(source).as_posix()] = path.read_bytes()
    resources = tool.get("wheel-resources", {})
    if not isinstance(resources, dict):
        raise BuildBackendError("tool.fullbleed-build.wheel-resources must be a table")
    root = _ROOT.resolve()
    for target_name, source_name in sorted(resources.items()):
        if not isinstance(target_name, str) or not isinstance(source_name, str):
            raise BuildBackendError("wheel resource paths must be strings")
        target = PurePosixPath(target_name)
        if target.is_absolute() or not target.parts or ".." in target.parts:
            raise BuildBackendError(f"invalid wheel resource target: {target_name}")
        source_path = (_ROOT / source_name).resolve()
        try:
            source_path.relative_to(root)
        except ValueError as error:
            raise BuildBackendError(
                f"wheel resource escapes repository: {source_name}"
            ) from error
        if not source_path.is_file():
            raise BuildBackendError(f"wheel resource is missing: {source_path}")
        normalized_target = target.as_posix()
        if normalized_target in entries:
            raise BuildBackendError(
                f"wheel resource path collision: {normalized_target}"
            )
        entries[normalized_target] = source_path.read_bytes()
    return entries


def _build_extension(options: dict) -> Path:
    command = [
        _cargo_command(),
        "build",
        "--manifest-path",
        str(_ROOT / "Cargo.toml"),
        "--message-format=json-render-diagnostics",
    ]
    if options["locked"]:
        command.append("--locked")
    if options["release"]:
        command.append("--release")
    if options["features"]:
        command.extend(["--features", ",".join(options["features"])])
    if options["target"]:
        command.extend(["--target", options["target"]])
    if options["cargo_extra_args"]:
        command.extend(
            shlex.split(str(options["cargo_extra_args"]), posix=os.name != "nt")
        )

    environment = os.environ.copy()
    environment.setdefault("SOURCE_DATE_EPOCH", str(_source_date_epoch()))
    if options["release"]:
        # Parallel codegen units can reach the linker in different orders even
        # when the source tree is identical. One unit makes checkout and sdist
        # builds byte-reproducible and gives release builds full-crate
        # optimization scope.
        environment.setdefault("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "1")
    if options["release"] and options["strip"]:
        environment.setdefault("CARGO_PROFILE_RELEASE_STRIP", "symbols")
    target = options.get("target") or ""
    if (target and "windows-msvc" in target) or (
        not target and sys.platform == "win32"
    ):
        # MSVC's default PE timestamp and CodeView identifier change at every
        # link even when all inputs are identical. /Brepro derives them from
        # the linked content, making wheels stable across isolated build envs.
        rustflags_key = (
            "CARGO_TARGET_" + target.upper().replace("-", "_") + "_RUSTFLAGS"
            if target
            else "RUSTFLAGS"
        )
        rustflags = environment.get(rustflags_key, "")
        if "link-arg=/Brepro" not in rustflags:
            environment[rustflags_key] = (rustflags + " -C link-arg=/Brepro").strip()
    if "apple" in target and "MACOSX_DEPLOYMENT_TARGET" not in environment:
        environment["MACOSX_DEPLOYMENT_TARGET"] = (
            "11.0" if _target_arch(target) == "aarch64" else "10.12"
        )

    process = subprocess.Popen(
        command,
        cwd=_ROOT,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=None,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    artifact = None
    assert process.stdout is not None
    for line in process.stdout:
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            sys.stderr.write(line)
            continue
        rendered = message.get("message", {}).get("rendered")
        if rendered:
            sys.stderr.write(rendered)
        target_data = message.get("target", {})
        if (
            message.get("reason") == "compiler-artifact"
            and target_data.get("name") == "fullbleed"
            and "cdylib" in target_data.get("crate_types", [])
        ):
            native = [
                Path(filename)
                for filename in message.get("filenames", [])
                if Path(filename).suffix.lower() in _NATIVE_SUFFIXES
            ]
            if native:
                artifact = native[0]
    returncode = process.wait()
    if returncode:
        raise BuildBackendError(
            f"Cargo extension build failed with exit code {returncode}"
        )
    if artifact is None or not artifact.is_file():
        raise BuildBackendError("Cargo did not report the FullBleed cdylib artifact")
    return artifact


def _extension_wheel_path(options: dict) -> str:
    module = str(_project()["tool"].get("module-name", "fullbleed._fullbleed"))
    parts = module.split(".")
    if len(parts) < 2 or not all(part.isidentifier() for part in parts):
        raise BuildBackendError(f"invalid extension module name: {module}")
    suffix = ".pyd" if _platform_tag(options).startswith("win") else ".abi3.so"
    return "/".join(parts[:-1] + [parts[-1] + suffix])


def _editable_entries(artifact: Path, options: dict) -> dict[str, bytes]:
    extension_name = Path(_extension_wheel_path(options)).name
    extension_path = f"_fullbleed_editable/{extension_name}"
    source = str(
        (_ROOT / str(_project()["tool"].get("python-source", "python"))).resolve()
    )
    bootstrap = f"""# Generated by fullbleed-build.\nimport importlib.abc\nimport importlib.util\nfrom pathlib import Path\nimport sys\n\n_SOURCE = {source!r}\n_EXTENSION = Path(__file__).resolve().parent / "_fullbleed_editable" / {extension_name!r}\n\nclass _FullBleedExtensionFinder(importlib.abc.MetaPathFinder):\n    _fullbleed_editable_finder = True\n\n    def find_spec(self, fullname, path=None, target=None):\n        if fullname == "fullbleed._fullbleed":\n            return importlib.util.spec_from_file_location(fullname, _EXTENSION)\n        return None\n\ndef install():\n    if _SOURCE not in sys.path:\n        sys.path.insert(0, _SOURCE)\n    if not any(getattr(finder, "_fullbleed_editable_finder", False) for finder in sys.meta_path):\n        sys.meta_path.insert(0, _FullBleedExtensionFinder())\n"""
    return {
        extension_path: artifact.read_bytes(),
        "_fullbleed_editable.py": bootstrap.encode("utf-8"),
        "_fullbleed_editable.pth": b"import _fullbleed_editable; _fullbleed_editable.install()\n",
    }


def _record(entries: dict[str, bytes]) -> bytes:
    dist_info = _dist_info_name()
    record_path = f"{dist_info}/RECORD"
    lines = []
    for name in sorted(entries):
        digest = base64.urlsafe_b64encode(
            hashlib.sha256(entries[name]).digest()
        ).rstrip(b"=")
        lines.append(f"{name},sha256={digest.decode('ascii')},{len(entries[name])}")
    lines.append(f"{record_path},,")
    return ("\n".join(lines) + "\n").encode("utf-8")


def _zip_timestamp() -> tuple[int, int, int, int, int, int]:
    return tuple(
        _datetime.datetime.fromtimestamp(
            _source_date_epoch(), tz=_datetime.timezone.utc
        ).timetuple()[:6]
    )


def _write_wheel(destination: Path, entries: dict[str, bytes]) -> None:
    entries = dict(entries)
    entries[f"{_dist_info_name()}/RECORD"] = _record(entries)
    destination.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(
        destination, mode="w", compression=zipfile.ZIP_DEFLATED, compresslevel=9
    ) as archive:
        for name in sorted(entries):
            info = zipfile.ZipInfo(name, _zip_timestamp())
            info.create_system = 3
            info.compress_type = zipfile.ZIP_DEFLATED
            executable = name.endswith((".pyd", ".so", ".dylib"))
            info.external_attr = (0o100755 if executable else 0o100644) << 16
            archive.writestr(info, entries[name])


def _wheel_filename(tag: str) -> str:
    project = _project()["project"]
    name = _distribution_name(project["name"])
    version = str(project["version"]).replace("-", "_")
    return f"{name}-{version}-{tag}.whl"


def _build_wheel(
    wheel_directory: str,
    config_settings: dict | None,
    metadata_directory: str | None,
    *,
    editable: bool,
) -> str:
    options = _build_options(config_settings)
    tag = _wheel_tag(options)
    artifact = _build_extension(options)
    if editable:
        entries = _editable_entries(artifact, options)
    else:
        entries = _python_entries()
        entries[_extension_wheel_path(options)] = artifact.read_bytes()
    metadata = (
        _prepared_entries(metadata_directory, tag)
        if metadata_directory
        else _dist_info_entries(tag, options["features"])
    )
    overlap = set(entries).intersection(metadata)
    if overlap:
        raise BuildBackendError(f"wheel path collision: {sorted(overlap)[0]}")
    entries.update(metadata)
    filename = _wheel_filename(tag)
    _write_wheel(Path(wheel_directory) / filename, entries)
    return filename


def get_requires_for_build_wheel(config_settings=None):
    return []


def get_requires_for_build_editable(config_settings=None):
    return []


def get_requires_for_build_sdist(config_settings=None):
    return []


def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
    options = _build_options(config_settings)
    destination = Path(metadata_directory) / _dist_info_name()
    for name, content in _dist_info_entries(
        _wheel_tag(options), options["features"]
    ).items():
        relative = Path(name).relative_to(_dist_info_name())
        path = destination / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
    return _dist_info_name()


def prepare_metadata_for_build_editable(metadata_directory, config_settings=None):
    return prepare_metadata_for_build_wheel(metadata_directory, config_settings)


def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    return _build_wheel(
        wheel_directory, config_settings, metadata_directory, editable=False
    )


def build_editable(wheel_directory, config_settings=None, metadata_directory=None):
    return _build_wheel(
        wheel_directory, config_settings, metadata_directory, editable=True
    )


def _sdist_files() -> list[Path]:
    project = _project()["project"]
    files = {
        _ROOT / name
        for name in (
            "Cargo.toml",
            "Cargo.lock",
            "build.rs",
            "pyproject.toml",
            str(project.get("readme", "README.md")),
            *project.get("license-files", []),
        )
    }
    roots = [
        _ROOT / "src",
        _ROOT / "build_backend",
        _ROOT / "crates" / "fullbleed_audit_contract" / "src",
        _ROOT / "crates" / "fullbleed_audit_contract" / "specs",
    ]
    audit_root = _ROOT / "crates" / "fullbleed_audit_contract"
    files.update(
        audit_root / name
        for name in ("Cargo.toml", "Cargo.lock", "LICENSE", "README.md")
    )
    resources = _project()["tool"].get("wheel-resources", {})
    if not isinstance(resources, dict):
        raise BuildBackendError("tool.fullbleed-build.wheel-resources must be a table")
    files.update(_ROOT / source for source in resources.values())
    python_root = _ROOT / str(_project()["tool"].get("python-source", "python"))
    roots.extend(
        python_root / package
        for package in _project()["tool"].get("python-packages", [])
    )
    for root in roots:
        if not root.is_dir():
            raise BuildBackendError(f"sdist source directory is missing: {root}")
        for path in root.rglob("*"):
            if (
                path.is_file()
                and "__pycache__" not in path.parts
                and path.suffix != ".pyc"
                and not _is_generated_python_extension(path)
            ):
                files.add(path)
    missing = [path for path in files if not path.is_file()]
    if missing:
        raise BuildBackendError(f"sdist source file is missing: {missing[0]}")
    return sorted(files, key=lambda path: path.relative_to(_ROOT).as_posix())


def build_sdist(sdist_directory, config_settings=None):
    del config_settings
    project = _project()["project"]
    stem = f"{project['name']}-{project['version']}"
    filename = f"{stem}.tar.gz"
    destination = Path(sdist_directory) / filename
    destination.parent.mkdir(parents=True, exist_ok=True)
    epoch = _source_date_epoch()
    with destination.open("wb") as raw:
        with gzip.GzipFile(
            filename="", mode="wb", fileobj=raw, mtime=epoch
        ) as compressed:
            with tarfile.open(
                fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT
            ) as archive:
                members = [(Path("PKG-INFO"), _metadata_bytes())]
                members.extend(
                    (path.relative_to(_ROOT), path.read_bytes())
                    for path in _sdist_files()
                )
                for relative, content in sorted(
                    members, key=lambda item: item[0].as_posix()
                ):
                    info = tarfile.TarInfo(f"{stem}/{relative.as_posix()}")
                    info.size = len(content)
                    info.mtime = epoch
                    info.mode = 0o644
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    archive.addfile(info, fileobj=_BytesReader(content))
    return filename


class _BytesReader:
    """The tiny read-only interface tarfile.addfile needs."""

    def __init__(self, value: bytes):
        self._value = value
        self._offset = 0

    def read(self, size: int = -1) -> bytes:
        if size < 0:
            size = len(self._value) - self._offset
        start = self._offset
        self._offset = min(len(self._value), start + size)
        return self._value[start : self._offset]


def _cli_config(arguments) -> dict:
    config = {
        "release": str(not arguments.debug).lower(),
        "locked": str(not arguments.unlocked).lower(),
        "strip": str(not arguments.no_strip).lower(),
    }
    for key in ("target", "compatibility", "features", "plat_name", "cargo_extra_args"):
        value = getattr(arguments, key, None)
        if value is not None:
            config[key.replace("_", "-")] = value
    return config


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    for command in ("wheel", "editable", "metadata"):
        child = subparsers.add_parser(command)
        child.add_argument("--out", default="dist")
        child.add_argument("--target")
        child.add_argument("--compatibility", default="off")
        child.add_argument("--features")
        child.add_argument("--plat-name")
        child.add_argument("--cargo-extra-args")
        child.add_argument("--debug", action="store_true")
        child.add_argument("--unlocked", action="store_true")
        child.add_argument("--no-strip", action="store_true")
    sdist_parser = subparsers.add_parser("sdist")
    sdist_parser.add_argument("--out", default="dist")
    arguments = parser.parse_args(argv)
    output = Path(arguments.out).resolve()
    output.mkdir(parents=True, exist_ok=True)
    if arguments.command == "wheel":
        result = build_wheel(str(output), _cli_config(arguments))
    elif arguments.command == "editable":
        result = build_editable(str(output), _cli_config(arguments))
    elif arguments.command == "metadata":
        result = prepare_metadata_for_build_wheel(str(output), _cli_config(arguments))
    else:
        result = build_sdist(str(output))
    print(output / result)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except BuildBackendError as error:
        print(f"fullbleed-build: error: {error}", file=sys.stderr)
        raise SystemExit(1) from error
