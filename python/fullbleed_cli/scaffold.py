# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
"""Project scaffolding commands for fullbleed.

Provides commands for initializing new projects and creating templates.
"""
import json
import shutil
import sys
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


def _load_template_tree(relative_dir: str) -> dict[str, str]:
    root = _template_root().joinpath(relative_dir)
    if not root.is_dir():
        raise RuntimeError(f"scaffold template directory not found: {relative_dir}")

    files: dict[str, str] = {}
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
                files[rel] = child.read_text(encoding="utf-8")
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
    "invoice": {
        "description": "Basic invoice template",
        "source_dir": "new/invoice",
    },
    "statement": {
        "description": "Bank/account statement template",
        "source_dir": "new/statement",
    },
}


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

