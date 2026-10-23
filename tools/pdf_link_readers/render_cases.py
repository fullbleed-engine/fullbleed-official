"""Render small link specimens using the installed Fullbleed engine."""
import argparse
from hashlib import sha256
from importlib.metadata import version
import json
from pathlib import Path

import fullbleed
import fullbleed._fullbleed


BASE_CSS = """@page{size:200pt 200pt;margin:0}
body{margin:0;font-family:Helvetica;font-size:12pt}
a{position:absolute;left:40pt;top:40pt;width:100pt;height:100pt;
display:block;box-sizing:border-box;padding:15pt;background:#cceeff;color:#123456;}
"""
CASES = {
    "rectangle": "",
    "rotated": "a{height:40pt;transform-origin:0 0;transform:rotate(30deg)}",
    "triangle": "a{clip-path:polygon(0 0,100% 0,0 100%)}",
    "circle": "a{clip-path:circle(40% at 50% 50%)}",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True, help="New artifact directory")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    engine = fullbleed.PdfEngine()
    native = Path(fullbleed._fullbleed.__file__)
    manifest = {
        "schema": "fullbleed.link-reader-inputs.v1",
        "runtime": {
            "version": version("fullbleed"),
            "native_sha256": sha256(native.read_bytes()).hexdigest(),
            "build_features": fullbleed.build_features(),
        },
        "generator_sha256": sha256(Path(__file__).read_bytes()).hexdigest(),
        "cases": [],
    }
    for name, extra in CASES.items():
        html = f'<a href="https://example.invalid/{name}">Read</a>'
        css = BASE_CSS + extra
        pdf = bytes(engine.render_pdf(html, css))
        images = engine.render_image_pages(html, css, 144)
        if len(images) != 1:
            raise RuntimeError(f"{name}: expected one preview page")
        artifacts = {"html": html.encode(), "css": css.encode(), "pdf": pdf,
                     "png": bytes(images[0])}
        hashes = {}
        for extension, data in artifacts.items():
            path = args.out / f"{name}.{extension}"
            path.write_bytes(data)
            hashes[path.name] = sha256(data).hexdigest()
        manifest["cases"].append({"name": name, "files": hashes})
    (args.out / "inputs.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"out": str(args.out), "runtime": manifest["runtime"], "cases": len(CASES)}))


if __name__ == "__main__":
    main()
