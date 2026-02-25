# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
"""Project scaffolding commands for fullbleed.

Provides commands for initializing new projects and creating templates.
"""
import json
import hashlib
import os
import shutil
import sys
import tempfile
import urllib.error
import urllib.request
import zipfile
from importlib import resources
from pathlib import Path


# Builtin bootstrap defaults for `fullbleed init`.
DEFAULT_BOOTSTRAP_REF = "bootstrap"
DEFAULT_BOOTSTRAP_VERSION = "5.0.0"
DEFAULT_BOOTSTRAP_FILENAME = "bootstrap.min.css"
DEFAULT_BOOTSTRAP_REL_PATH = "vendor/css/bootstrap.min.css"
DEFAULT_BOOTSTRAP_LICENSE_FILE = "vendor/css/LICENSE.bootstrap.txt"
DEFAULT_BOOTSTRAP_LICENSE_URL = "https://raw.githubusercontent.com/twbs/bootstrap/v5.0.0/LICENSE"
DEFAULT_INIT_FONT_NAME = "inter"
DEFAULT_INIT_FONT_VERSION = "4.0"
DEFAULT_INIT_FONT_SOURCE_REL_PATH = "fonts/Inter-Variable.ttf"
DEFAULT_INIT_FONT_REL_PATH = "vendor/fonts/Inter-Variable.ttf"
DEFAULT_INIT_FONT_LICENSE = "OFL-1.1"
DEFAULT_INIT_FONT_LICENSE_FILE = "vendor/fonts/LICENSE.inter.txt"
DEFAULT_INIT_FONT_LICENSE_URL = "https://raw.githubusercontent.com/google/fonts/main/ofl/inter/OFL.txt"
DEFAULT_INIT_ICON_NAME = "bootstrap-icons"
DEFAULT_INIT_ICON_VERSION = "1.11.3"
DEFAULT_INIT_ICON_SOURCE_REL_PATH = "icons/bootstrap-icons.svg"
DEFAULT_INIT_ICON_REL_PATH = "vendor/icons/bootstrap-icons.svg"
DEFAULT_INIT_ICON_LICENSE = "MIT"
DEFAULT_INIT_ICON_LICENSE_FILE = "vendor/icons/LICENSE.bootstrap-icons.txt"
DEFAULT_INIT_ICON_LICENSE_URL = "https://raw.githubusercontent.com/twbs/icons/v1.11.3/LICENSE"


SCAFFOLD_TEMPLATE_PACKAGE = "fullbleed_cli.scaffold_templates"


def _template_root():
    return resources.files(SCAFFOLD_TEMPLATE_PACKAGE)


def _load_template_tree(relative_dir: str) -> dict[str, str | bytes]:
    root = _template_root().joinpath(relative_dir)
    if not root.is_dir():
        raise RuntimeError(f"scaffold template directory not found: {relative_dir}")

    files: dict[str, str | bytes] = {}
    stack: list[tuple[object, str]] = [(root, "")]
    while stack:
        node, prefix = stack.pop()
        for child in sorted(node.iterdir(), key=lambda x: x.name):
            if child.name == "__pycache__":
                continue
            rel = f"{prefix}/{child.name}" if prefix else child.name
            rel = rel.replace("\\", "/")
            if child.is_dir():
                stack.append((child, rel))
            elif child.is_file():
                if child.suffix.lower() in {".pyc", ".pyo"}:
                    continue
                try:
                    files[rel] = child.read_text(encoding="utf-8")
                except UnicodeDecodeError:
                    files[rel] = child.read_bytes()
    return dict(sorted(files.items()))


def _build_default_init_files(
    bootstrap_sha256: str,
    init_font_sha256: str,
    init_icon_sha256: str,
):
    """Return default init files with locked builtin asset metadata."""
    assets_lock = {
        "schema": 1,
        "packages": [
            {
                "name": "bootstrap",
                "version": DEFAULT_BOOTSTRAP_VERSION,
                "kind": "css",
                "files": [
                    {
                        "path": DEFAULT_BOOTSTRAP_REL_PATH,
                        "sha256": bootstrap_sha256,
                    }
                ],
            },
            {
                "name": DEFAULT_INIT_FONT_NAME,
                "version": DEFAULT_INIT_FONT_VERSION,
                "kind": "font",
                "files": [
                    {
                        "path": DEFAULT_INIT_FONT_REL_PATH,
                        "sha256": init_font_sha256,
                    }
                ],
            },
            {
                "name": DEFAULT_INIT_ICON_NAME,
                "version": DEFAULT_INIT_ICON_VERSION,
                "kind": "svg",
                "files": [
                    {
                        "path": DEFAULT_INIT_ICON_REL_PATH,
                        "sha256": init_icon_sha256,
                    }
                ],
            },
        ],
    }

    init_files = _load_template_tree("init")
    init_files["assets.lock.json"] = json.dumps(assets_lock, ensure_ascii=True, indent=2) + "\n"
    return init_files


def _builtin_seed_info(asset_ref: str, source_rel_path: str, label: str):
    """Resolve source file + metadata for a builtin asset."""
    import fullbleed_assets
    from .assets import _compute_file_hash, _resolve_builtin_asset

    package = _resolve_builtin_asset(asset_ref)
    if not package:
        raise RuntimeError(f"builtin {label} package metadata is missing")

    source_path = Path(fullbleed_assets.asset_path(source_rel_path))
    if not source_path.exists():
        raise RuntimeError(f"builtin {label} asset not found: {source_path}")

    sha256 = _compute_file_hash(source_path)
    return package, source_path, sha256


def _bootstrap_seed_info():
    """Resolve bootstrap source file + metadata for scaffold init."""
    return _builtin_seed_info(
        asset_ref=DEFAULT_BOOTSTRAP_REF,
        source_rel_path=DEFAULT_BOOTSTRAP_FILENAME,
        label="bootstrap",
    )


def _init_font_seed_info():
    """Resolve the default init font source file and hash."""
    import fullbleed_assets
    from .assets import _compute_file_hash

    source_path = Path(fullbleed_assets.asset_path(DEFAULT_INIT_FONT_SOURCE_REL_PATH))
    if not source_path.exists():
        raise RuntimeError(f"init font asset not found: {source_path}")

    return source_path, _compute_file_hash(source_path)


def _init_icon_seed_info():
    """Resolve the default init icon bundle source file and hash."""
    import fullbleed_assets
    from .assets import _compute_file_hash

    source_path = Path(fullbleed_assets.asset_path(DEFAULT_INIT_ICON_SOURCE_REL_PATH))
    if not source_path.exists():
        raise RuntimeError(f"init icon asset not found: {source_path}")

    return source_path, _compute_file_hash(source_path)


def _emit_init_asset_error(exc, is_json):
    """Emit a consistent init error when builtin assets cannot be provisioned."""
    message = f"Failed to provision init vendored assets: {exc}"
    if is_json:
        result = {
            "schema": "fullbleed.error.v1",
            "ok": False,
            "code": "INIT_ASSET_UNAVAILABLE",
            "message": message,
        }
        sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
    else:
        sys.stderr.write(f"[error] {message}\n")


def _emit_init_template_error(exc, is_json):
    """Emit a consistent init error when scaffold template files are unavailable."""
    message = f"Failed to load init scaffold templates: {exc}"
    if is_json:
        result = {
            "schema": "fullbleed.error.v1",
            "ok": False,
            "code": "INIT_TEMPLATE_UNAVAILABLE",
            "message": message,
        }
        sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
    else:
        sys.stderr.write(f"[error] {message}\n")


DEFAULT_INIT_DIRS = ["components", "styles", "output", "vendor"]
DEFAULT_VENDOR_CSS_DIR = "vendor/css"
DEFAULT_VENDOR_FONT_DIR = "vendor/fonts"
DEFAULT_VENDOR_ICON_DIR = "vendor/icons"

DEFAULT_DIRS = DEFAULT_INIT_DIRS


# Sample templates
TEMPLATES = {
    "accessible": {
        "description": "Accessibility-first document scaffold (fullbleed.ui.accessibility)",
        "source_dir": "new/accessible",
    },
    "invoice": {
        "description": "Basic invoice template",
        "source_dir": "new/invoice",
    },
    "statement": {
        "description": "Bank/account statement template",
        "source_dir": "new/statement",
    },
}


DEFAULT_TEMPLATE_REGISTRY_URL = (
    "https://raw.githubusercontent.com/fullbleed-engine/fullbleed-manifest/master/manifest.json"
)
TEMPLATE_REGISTRY_SCHEMA = "fullbleed.template_registry.v1"


def cmd_init(args):
    """Initialize a new fullbleed project in the current directory."""
    target_dir = Path(args.path) if hasattr(args, "path") and args.path else Path.cwd()
    force = getattr(args, "force", False)
    is_json = getattr(args, "json", False)
    
    # Check if already initialized
    report_path = target_dir / "report.py"
    if report_path.exists() and not force:
        if is_json:
            result = {
                "schema": "fullbleed.error.v1",
                "ok": False,
                "code": "ALREADY_INITIALIZED",
                "message": f"Directory already contains report.py. Use --force to overwrite.",
            }
            sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
        else:
            sys.stderr.write(f"[error] Directory already contains report.py. Use --force to overwrite.\n")
        raise SystemExit(1)

    try:
        from .assets import _license_filename, _write_license_notice
        bootstrap, bootstrap_source_path, bootstrap_sha256 = _bootstrap_seed_info()
        init_font_source_path, init_font_sha256 = _init_font_seed_info()
        init_icon_source_path, init_icon_sha256 = _init_icon_seed_info()
    except Exception as exc:
        _emit_init_asset_error(exc, is_json=is_json)
        raise SystemExit(1)
    try:
        init_files = _build_default_init_files(
            bootstrap_sha256=bootstrap_sha256,
            init_font_sha256=init_font_sha256,
            init_icon_sha256=init_icon_sha256,
        )
    except Exception as exc:
        _emit_init_template_error(exc, is_json=is_json)
        raise SystemExit(1)
    
    # Create directories
    created_dirs = []
    for dirname in DEFAULT_DIRS:
        dir_path = target_dir / dirname
        if not dir_path.exists():
            dir_path.mkdir(parents=True, exist_ok=True)
            created_dirs.append(dirname)

    vendor_css_dir = target_dir / DEFAULT_VENDOR_CSS_DIR
    if not vendor_css_dir.exists():
        vendor_css_dir.mkdir(parents=True, exist_ok=True)
        created_dirs.append(DEFAULT_VENDOR_CSS_DIR)
    vendor_font_dir = target_dir / DEFAULT_VENDOR_FONT_DIR
    if not vendor_font_dir.exists():
        vendor_font_dir.mkdir(parents=True, exist_ok=True)
        created_dirs.append(DEFAULT_VENDOR_FONT_DIR)
    vendor_icon_dir = target_dir / DEFAULT_VENDOR_ICON_DIR
    if not vendor_icon_dir.exists():
        vendor_icon_dir.mkdir(parents=True, exist_ok=True)
        created_dirs.append(DEFAULT_VENDOR_ICON_DIR)
    
    # Create files
    created_files = []
    for filename, content in init_files.items():
        file_path = target_dir / filename
        if not file_path.exists() or force:
            file_path.parent.mkdir(parents=True, exist_ok=True)
            if isinstance(content, bytes):
                file_path.write_bytes(content)
            else:
                file_path.write_text(content, encoding="utf-8")
            created_files.append(filename)

    try:
        bootstrap_dest_path = target_dir / DEFAULT_BOOTSTRAP_REL_PATH
        if not bootstrap_dest_path.exists() or force:
            shutil.copy2(bootstrap_source_path, bootstrap_dest_path)
            created_files.append(DEFAULT_BOOTSTRAP_REL_PATH)

        bootstrap_license_name = _license_filename(bootstrap["name"])
        bootstrap_license_path = vendor_css_dir / bootstrap_license_name
        if not bootstrap_license_path.exists() or force:
            written_license = _write_license_notice(
                dest_dir=vendor_css_dir,
                package_name=bootstrap["name"],
                version=bootstrap["version"],
                license_name=bootstrap.get("license", "MIT"),
                license_url=bootstrap.get("license_url", DEFAULT_BOOTSTRAP_LICENSE_URL),
            )
            if written_license:
                created_files.append(DEFAULT_BOOTSTRAP_LICENSE_FILE)

        init_font_dest_path = target_dir / DEFAULT_INIT_FONT_REL_PATH
        if not init_font_dest_path.exists() or force:
            shutil.copy2(init_font_source_path, init_font_dest_path)
            created_files.append(DEFAULT_INIT_FONT_REL_PATH)

        init_font_license_name = _license_filename(DEFAULT_INIT_FONT_NAME)
        init_font_license_path = vendor_font_dir / init_font_license_name
        if not init_font_license_path.exists() or force:
            written_init_font_license = _write_license_notice(
                dest_dir=vendor_font_dir,
                package_name=DEFAULT_INIT_FONT_NAME,
                version=DEFAULT_INIT_FONT_VERSION,
                license_name=DEFAULT_INIT_FONT_LICENSE,
                license_url=DEFAULT_INIT_FONT_LICENSE_URL,
            )
            if written_init_font_license:
                created_files.append(DEFAULT_INIT_FONT_LICENSE_FILE)

        init_icon_dest_path = target_dir / DEFAULT_INIT_ICON_REL_PATH
        if not init_icon_dest_path.exists() or force:
            shutil.copy2(init_icon_source_path, init_icon_dest_path)
            created_files.append(DEFAULT_INIT_ICON_REL_PATH)

        init_icon_license_name = _license_filename(DEFAULT_INIT_ICON_NAME)
        init_icon_license_path = vendor_icon_dir / init_icon_license_name
        if not init_icon_license_path.exists() or force:
            written_init_icon_license = _write_license_notice(
                dest_dir=vendor_icon_dir,
                package_name=DEFAULT_INIT_ICON_NAME,
                version=DEFAULT_INIT_ICON_VERSION,
                license_name=DEFAULT_INIT_ICON_LICENSE,
                license_url=DEFAULT_INIT_ICON_LICENSE_URL,
            )
            if written_init_icon_license:
                created_files.append(DEFAULT_INIT_ICON_LICENSE_FILE)
    except Exception as exc:
        _emit_init_asset_error(exc, is_json=is_json)
        raise SystemExit(1)
    
    result = {
        "path": str(target_dir),
        "created_dirs": created_dirs,
        "created_files": created_files,
    }
    
    if is_json:
        output = {"schema": "fullbleed.init.v1", "ok": True, **result}
        sys.stdout.write(json.dumps(output, ensure_ascii=True) + "\n")
    else:
        sys.stdout.write(f"[ok] Initialized fullbleed project in {target_dir}\n")
        if created_dirs:
            sys.stdout.write(f"  Created directories: {', '.join(created_dirs)}\n")
        if created_files:
            sys.stdout.write(f"  Created files: {', '.join(created_files)}\n")
        sys.stdout.write("\n  Next steps:\n")
        sys.stdout.write("    1. Review SCAFFOLDING.md and COMPLIANCE.md\n")
        sys.stdout.write("    2. Edit components/header.py, components/body.py, components/footer.py, and components/primitives.py\n")
        sys.stdout.write("    3. Tune component styles in components/styles/*.css and composition styles/report.css\n")
        sys.stdout.write("    4. Run: python report.py\n")
        sys.stdout.write("    5. Optional diagnostics: set FULLBLEED_DEBUG=1, FULLBLEED_PERF=1, FULLBLEED_EMIT_PAGE_DATA=1, FULLBLEED_IMAGE_DPI=144\n")


def cmd_new_template(args):
    """Create a new template from a starter template."""
    template_name = args.template
    target_dir = Path(args.path) if hasattr(args, "path") and args.path else Path.cwd()
    force = getattr(args, "force", False)
    
    if template_name not in TEMPLATES:
        available = ", ".join(TEMPLATES.keys())
        if getattr(args, "json", False):
            result = {
                "schema": "fullbleed.error.v1",
                "ok": False,
                "code": "UNKNOWN_TEMPLATE",
                "message": f"Unknown template: {template_name}. Available: {available}",
            }
            sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
        else:
            sys.stderr.write(f"[error] Unknown template: {template_name}\n")
            sys.stderr.write(f"  Available templates: {available}\n")
        raise SystemExit(1)
    
    template = TEMPLATES[template_name]
    try:
        template_files = _load_template_tree(template["source_dir"])
    except Exception as exc:
        message = f"Template files unavailable for {template_name}: {exc}"
        if getattr(args, "json", False):
            result = {
                "schema": "fullbleed.error.v1",
                "ok": False,
                "code": "TEMPLATE_UNAVAILABLE",
                "message": message,
            }
            sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
        else:
            sys.stderr.write(f"[error] {message}\n")
        raise SystemExit(1)

    created_files = []
    
    for filepath, content in template_files.items():
        full_path = target_dir / filepath
        if full_path.exists() and not force:
            if getattr(args, "json", False):
                result = {
                    "schema": "fullbleed.error.v1",
                    "ok": False,
                    "code": "FILE_EXISTS",
                    "message": f"File already exists: {filepath}. Use --force to overwrite.",
                }
                sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
            else:
                sys.stderr.write(f"[error] File already exists: {filepath}. Use --force to overwrite.\n")
            raise SystemExit(1)
        
        full_path.parent.mkdir(parents=True, exist_ok=True)
        if isinstance(content, bytes):
            full_path.write_bytes(content)
        else:
            full_path.write_text(content, encoding="utf-8")
        created_files.append(filepath)
    
    result = {
        "template": template_name,
        "description": template["description"],
        "created_files": created_files,
    }
    
    if getattr(args, "json", False):
        output = {"schema": "fullbleed.new_template.v1", "ok": True, **result}
        sys.stdout.write(json.dumps(output, ensure_ascii=True) + "\n")
    else:
        sys.stdout.write(f"[ok] Created {template_name} template\n")
        for f in created_files:
            sys.stdout.write(f"  - {f}\n")


def cmd_new_template_alias(args):
    """Compatibility shim for `fullbleed new <template>` legacy usage."""
    cmd_new_template(args)


def _emit_new_registry_error(code: str, message: str, *, is_json: bool) -> None:
    if is_json:
        result = {
            "schema": "fullbleed.error.v1",
            "ok": False,
            "code": code,
            "message": message,
        }
        sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
    else:
        sys.stderr.write(f"[error] {code}: {message}\n")
    raise SystemExit(1)


def _resolve_registry_url(args) -> str:
    return (
        getattr(args, "registry", None)
        or os.environ.get("FULLBLEED_TEMPLATE_REGISTRY")
        or DEFAULT_TEMPLATE_REGISTRY_URL
    )


def _fetch_template_registry(url: str) -> dict:
    req = urllib.request.Request(
        url,
        headers={"User-Agent": "fullbleed-cli-template-registry/1"},
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            if getattr(resp, "status", 200) >= 400:
                raise ValueError(f"registry request failed with status {resp.status}")
            payload = json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        raise ValueError(f"registry HTTP error ({exc.code}): {url}") from exc
    except urllib.error.URLError as exc:
        raise ValueError(f"registry network error: {exc.reason}") from exc
    except json.JSONDecodeError as exc:
        raise ValueError(f"registry JSON parse failed: {exc}") from exc

    if not isinstance(payload, dict):
        raise ValueError("registry payload must be a JSON object")
    schema = payload.get("schema")
    if schema != TEMPLATE_REGISTRY_SCHEMA:
        raise ValueError(
            f"unsupported registry schema: {schema!r} (expected {TEMPLATE_REGISTRY_SCHEMA!r})"
        )
    templates = payload.get("templates")
    if not isinstance(templates, list):
        raise ValueError("registry payload missing templates[] list")
    return payload


def _template_summaries(payload: dict) -> list[dict]:
    out: list[dict] = []
    for raw in payload.get("templates", []):
        if not isinstance(raw, dict):
            continue
        template_id = str(raw.get("id", "")).strip()
        if not template_id:
            continue
        tags = raw.get("tags")
        if not isinstance(tags, list):
            tags = []
        tags = [str(tag).strip() for tag in tags if str(tag).strip()]
        releases = raw.get("releases")
        release_count = len(releases) if isinstance(releases, list) else 0
        out.append(
            {
                "id": template_id,
                "name": str(raw.get("name", "")).strip() or template_id,
                "summary": str(raw.get("summary", "")).strip(),
                "description": str(raw.get("description", "")).strip(),
                "tags": tags,
                "maintainer": str(raw.get("maintainer", "")).strip(),
                "license": str(raw.get("license", "")).strip(),
                "homepage": str(raw.get("homepage", "")).strip(),
                "latest": str(raw.get("latest", "")).strip() or None,
                "release_count": release_count,
            }
        )
    out.sort(key=lambda item: (item["name"].lower(), item["id"].lower()))
    return out


def cmd_new_list(args):
    """List remote templates from registry manifest."""
    is_json = getattr(args, "json", False)
    registry_url = _resolve_registry_url(args)
    try:
        payload = _fetch_template_registry(registry_url)
        templates = _template_summaries(payload)
    except Exception as exc:
        _emit_new_registry_error(
            "TEMPLATE_REGISTRY_UNAVAILABLE",
            str(exc),
            is_json=is_json,
        )
        raise

    registry = payload.get("registry")
    if not isinstance(registry, dict):
        registry = {}
    result = {
        "schema": "fullbleed.new_list.v1",
        "ok": True,
        "registry_url": registry_url,
        "registry": {
            "name": registry.get("name"),
            "homepage": registry.get("homepage"),
            "generated_at": registry.get("generated_at"),
        },
        "count": len(templates),
        "templates": templates,
    }
    if is_json:
        sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
        return

    sys.stdout.write(f"[ok] {len(templates)} template(s) available\n")
    for tmpl in templates:
        tags = ", ".join(tmpl["tags"]) if tmpl["tags"] else "-"
        latest = tmpl["latest"] or "n/a"
        sys.stdout.write(
            f"  - {tmpl['id']} ({tmpl['name']}) latest={latest} tags={tags}\n"
        )


def _template_matches_query(template: dict, query: str, tags: list[str]) -> bool:
    q = query.strip().lower()
    if q:
        haystacks = [
            template.get("id", ""),
            template.get("name", ""),
            template.get("summary", ""),
            template.get("description", ""),
            " ".join(template.get("tags", [])),
        ]
        text = "\n".join(str(v).lower() for v in haystacks)
        if q not in text:
            return False
    if tags:
        template_tags = {str(t).lower() for t in template.get("tags", [])}
        for tag in tags:
            if tag not in template_tags:
                return False
    return True


def cmd_new_search(args):
    """Search remote templates from registry manifest."""
    is_json = getattr(args, "json", False)
    registry_url = _resolve_registry_url(args)
    try:
        payload = _fetch_template_registry(registry_url)
        templates = _template_summaries(payload)
    except Exception as exc:
        _emit_new_registry_error(
            "TEMPLATE_REGISTRY_UNAVAILABLE",
            str(exc),
            is_json=is_json,
        )
        raise

    query = str(getattr(args, "query", "") or "")
    tags = getattr(args, "tag", None) or []
    tags = [str(tag).strip().lower() for tag in tags if str(tag).strip()]
    matches = [t for t in templates if _template_matches_query(t, query, tags)]

    result = {
        "schema": "fullbleed.new_search.v1",
        "ok": True,
        "registry_url": registry_url,
        "query": query,
        "tags": tags,
        "count": len(matches),
        "templates": matches,
    }
    if is_json:
        sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
        return

    sys.stdout.write(f"[ok] {len(matches)} match(es)\n")
    for tmpl in matches:
        sys.stdout.write(f"  - {tmpl['id']} ({tmpl['name']})\n")


def _select_template(payload: dict, template_id: str) -> dict | None:
    for raw in payload.get("templates", []):
        if not isinstance(raw, dict):
            continue
        if str(raw.get("id", "")).strip() == template_id:
            return raw
    return None


def _select_release(template: dict, requested_version: str | None) -> dict | None:
    releases = template.get("releases")
    if not isinstance(releases, list) or not releases:
        return None
    if not requested_version or requested_version == "latest":
        latest = str(template.get("latest", "")).strip()
        if latest:
            for rel in releases:
                if isinstance(rel, dict) and str(rel.get("version", "")).strip() == latest:
                    return rel
        for rel in releases:
            if isinstance(rel, dict):
                return rel
        return None
    for rel in releases:
        if not isinstance(rel, dict):
            continue
        if str(rel.get("version", "")).strip() == requested_version:
            return rel
    return None


def _hash_file_sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(1024 * 1024)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


def _download_to_file(url: str, dst: Path) -> None:
    req = urllib.request.Request(
        url,
        headers={"User-Agent": "fullbleed-cli-template-registry/1"},
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        with dst.open("wb") as out:
            while True:
                chunk = resp.read(1024 * 1024)
                if not chunk:
                    break
                out.write(chunk)


def _clean_target_dir(path: Path, *, force: bool) -> None:
    if path.exists():
        if not path.is_dir():
            if not force:
                raise ValueError(
                    f"target path exists and is not a directory: {path}. Use --force to overwrite."
                )
            path.unlink(missing_ok=True)
            path.mkdir(parents=True, exist_ok=True)
            return
        if any(path.iterdir()):
            if not force:
                raise ValueError(
                    f"target directory is not empty: {path}. Use --force to overwrite."
                )
            for child in list(path.iterdir()):
                if child.is_dir():
                    shutil.rmtree(child)
                else:
                    child.unlink(missing_ok=True)
    else:
        path.mkdir(parents=True, exist_ok=True)


def _safe_extract_zip(
    archive_path: Path,
    target_dir: Path,
    *,
    root_dir: str | None,
) -> list[str]:
    extracted: list[str] = []
    root_prefix = None
    if root_dir:
        root_prefix = root_dir.strip().strip("/").replace("\\", "/")
        if root_prefix:
            root_prefix += "/"
        else:
            root_prefix = None

    with zipfile.ZipFile(archive_path, "r") as zf:
        names = zf.namelist()
        if root_prefix and not any(name.startswith(root_prefix) for name in names):
            raise ValueError(f"archive missing expected root_dir prefix: {root_dir}")

        for member in zf.infolist():
            raw_name = member.filename.replace("\\", "/")
            if not raw_name or raw_name.endswith("/"):
                continue
            if raw_name.startswith("/") or raw_name.startswith("../"):
                raise ValueError(f"unsafe archive entry: {raw_name}")

            rel_name = raw_name
            if root_prefix:
                if not rel_name.startswith(root_prefix):
                    continue
                rel_name = rel_name[len(root_prefix) :]
                if not rel_name:
                    continue
            rel_path = Path(rel_name)
            if any(part == ".." for part in rel_path.parts):
                raise ValueError(f"unsafe archive entry: {raw_name}")

            dest_path = target_dir / rel_path
            dest_parent = dest_path.parent.resolve()
            target_resolved = target_dir.resolve()
            if not dest_parent.is_relative_to(target_resolved):
                raise ValueError(f"unsafe archive destination: {raw_name}")
            dest_path.parent.mkdir(parents=True, exist_ok=True)
            with zf.open(member, "r") as src, dest_path.open("wb") as dst:
                shutil.copyfileobj(src, dst)
            extracted.append(str(rel_path).replace("\\", "/"))
    return extracted


def cmd_new_remote(args):
    """Install a remote template project from registry manifest."""
    is_json = getattr(args, "json", False)
    registry_url = _resolve_registry_url(args)
    template_id = str(getattr(args, "template_id", "") or "").strip()
    requested_version = str(getattr(args, "version", "latest") or "latest").strip()
    dry_run = bool(getattr(args, "dry_run", False))
    target_dir = Path(getattr(args, "path", ".") or ".").resolve()
    force = bool(getattr(args, "force", False))

    try:
        payload = _fetch_template_registry(registry_url)
    except Exception as exc:
        _emit_new_registry_error(
            "TEMPLATE_REGISTRY_UNAVAILABLE",
            str(exc),
            is_json=is_json,
        )
        raise

    template = _select_template(payload, template_id)
    if not template:
        available = [
            str(item.get("id", "")).strip()
            for item in payload.get("templates", [])
            if isinstance(item, dict) and str(item.get("id", "")).strip()
        ]
        _emit_new_registry_error(
            "UNKNOWN_REMOTE_TEMPLATE",
            f"unknown template id: {template_id}. available: {', '.join(sorted(available))}",
            is_json=is_json,
        )
        raise RuntimeError("unreachable")

    release = _select_release(template, requested_version)
    if not release:
        _emit_new_registry_error(
            "UNKNOWN_TEMPLATE_VERSION",
            f"no matching release for template={template_id} version={requested_version}",
            is_json=is_json,
        )
        raise RuntimeError("unreachable")

    archive = release.get("archive")
    if not isinstance(archive, dict):
        _emit_new_registry_error(
            "TEMPLATE_RELEASE_INVALID",
            f"release archive block is missing for template={template_id}",
            is_json=is_json,
        )
        raise RuntimeError("unreachable")

    archive_url = str(archive.get("url", "")).strip()
    archive_sha256 = str(archive.get("sha256", "")).strip()
    archive_format = str(archive.get("format", "zip")).strip().lower()
    archive_root = archive.get("root_dir")

    if archive_format != "zip":
        _emit_new_registry_error(
            "UNSUPPORTED_TEMPLATE_ARCHIVE",
            f"unsupported archive format: {archive_format!r} (expected 'zip')",
            is_json=is_json,
        )
        raise RuntimeError("unreachable")

    placeholder_url = ("<" in archive_url) or (">" in archive_url) or not archive_url
    placeholder_hash = (
        ("<" in archive_sha256)
        or (">" in archive_sha256)
        or len(archive_sha256) != 64
    )

    if dry_run:
        result = {
            "schema": "fullbleed.new_remote.v1",
            "ok": True,
            "dry_run": True,
            "registry_url": registry_url,
            "template_id": template_id,
            "version": str(release.get("version", "")).strip() or requested_version,
            "target_path": str(target_dir),
            "archive": {
                "url": archive_url,
                "sha256": archive_sha256,
                "format": archive_format,
                "root_dir": archive_root,
                "placeholder_url": placeholder_url,
                "placeholder_sha256": placeholder_hash,
            },
        }
        if is_json:
            sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
        else:
            sys.stdout.write(
                f"[ok] dry-run template={template_id} version={result['version']} target={target_dir}\n"
            )
        return

    if placeholder_url or placeholder_hash:
        _emit_new_registry_error(
            "TEMPLATE_RELEASE_UNAVAILABLE",
            (
                "release archive is not yet publishable; update manifest archive.url and "
                "archive.sha256 with real values"
            ),
            is_json=is_json,
        )
        raise RuntimeError("unreachable")

    temp_file = None
    try:
        with tempfile.NamedTemporaryFile(
            prefix=f"fullbleed_new_{template_id}_",
            suffix=".zip",
            delete=False,
        ) as tmp:
            temp_file = Path(tmp.name)

        _download_to_file(archive_url, temp_file)
        actual_sha = _hash_file_sha256(temp_file)
        if actual_sha.lower() != archive_sha256.lower():
            raise ValueError(
                f"archive sha256 mismatch expected={archive_sha256} got={actual_sha}"
            )

        _clean_target_dir(target_dir, force=force)
        extracted = _safe_extract_zip(
            temp_file,
            target_dir,
            root_dir=str(archive_root) if archive_root else None,
        )

        result = {
            "schema": "fullbleed.new_remote.v1",
            "ok": True,
            "dry_run": False,
            "registry_url": registry_url,
            "template_id": template_id,
            "version": str(release.get("version", "")).strip() or requested_version,
            "target_path": str(target_dir),
            "files_written": len(extracted),
            "sample_files": extracted[:50],
            "entrypoints": release.get("entrypoints") if isinstance(release.get("entrypoints"), dict) else None,
        }
        if is_json:
            sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
        else:
            sys.stdout.write(
                f"[ok] installed remote template {template_id}@{result['version']} to {target_dir}\n"
            )
            sys.stdout.write(f"  files written: {len(extracted)}\n")
    except Exception as exc:
        _emit_new_registry_error(
            "NEW_REMOTE_FAILED",
            str(exc),
            is_json=is_json,
        )
    finally:
        if temp_file and temp_file.exists():
            temp_file.unlink(missing_ok=True)

