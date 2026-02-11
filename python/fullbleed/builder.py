# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
import sys
import os
import importlib.util
from pathlib import Path
from typing import Optional
import json

import fullbleed
import fullbleed_assets
from .config import Config

def cmd_build(args):
    """Build the project based on fullbleed.toml."""
    config_path = Path(args.config) if args.config else None
    
    try:
        config = Config.load(config_path)
    except Exception as e:
        sys.stderr.write(f"[error] Failed to load config: {e}\n")
        sys.exit(1)
        
    if not args.json:
        sys.stdout.write(f"[build] Loading config from {config.path}\n")

    # 1. Create Asset Bundler
    bundle = fullbleed.AssetBundle()
    
    # Built-ins / Dependencies from config
    assets_config = config.assets
    for name, info in assets_config.items():
        # TODO: Implement robust asset resolution/downloading here
        # For now, simplistic mapping for testing
        if name == "bootstrap":
             bundle.add_file(str(fullbleed_assets.asset_path("bootstrap.min.css")), "css", name="bootstrap")
        elif name == "noto-sans":
             bundle.add_file(str(fullbleed_assets.asset_path("fonts/NotoSans-Regular.ttf")), "font", name="noto-sans")
        else:
             # Assume local file if not a known package? 
             # Or maybe config.assets is strictly for packages?
             pass
    
    # 2. Load Engine
    entrypoint = config.get_entrypoint()
    if ":" not in entrypoint:
        sys.stderr.write(f"[error] Invalid entrypoint: {entrypoint}\n")
        sys.exit(1)
        
    module_path_str, engine_name = entrypoint.rsplit(":", 1)
    
    # Resolve module path relative to config root
    module_path = config.resolve_path(module_path_str)
    
    if not module_path.exists():
         sys.stderr.write(f"[error] Entrypoint module not found: {module_path}\n")
         sys.exit(1)

    try:
        spec = importlib.util.spec_from_file_location("_fb_build_module", module_path)
        if spec is None or spec.loader is None:
             raise ImportError(f"Could not load spec for {module_path}")
        module = importlib.util.module_from_spec(spec)
        sys.modules["_fb_build_module"] = module
        spec.loader.exec_module(module)
    except Exception as e:
        sys.stderr.write(f"[error] Failed to load entrypoint module: {e}\n")
        sys.exit(1)
    
    if not hasattr(module, engine_name):
        sys.stderr.write(f"[error] '{engine_name}' not found in {module_path}\n")
        sys.exit(1)
        
    try:
        factory = getattr(module, engine_name)
        engine = factory() if callable(factory) else factory
    except Exception as e:
        sys.stderr.write(f"[error] Failed to create engine: {e}\n")
        sys.exit(1)
    
    # Register assets
    engine.register_bundle(bundle)
    
    # 3. Collect Inputs
    html_path = config.get_html_path()
    if html_path:
        html = html_path.read_text(encoding="utf-8")
    else:
        # Fallback? Or maybe the engine factory handles it?
        # For now require HTML in config for build command
         sys.stderr.write(f"[error] 'build.html' not specified in fullbleed.toml\n")
         sys.exit(1)

    css_paths = config.get_css_paths()
    css_content = "\n".join([p.read_text(encoding="utf-8") for p in css_paths])
    
    # 4. Render
    out_path = Path(args.out) if args.out else config.get_output_path()
    
    # Ensure output dir exists
    out_path.parent.mkdir(parents=True, exist_ok=True)
    
    bytes_written = engine.render_pdf_to_file(html, css_content, str(out_path))
    
    if args.json:
        result = {
            "schema": "fullbleed.build.v1",
            "ok": True,
            "output": str(out_path),
            "bytes": bytes_written
        }
        sys.stdout.write(json.dumps(result) + "\n")
    else:
        sys.stdout.write(f"[ok] Built {out_path} ({bytes_written} bytes)\n")
