"""Compare authored link geometry with PDF paint and two independent readers.

Exit 0: every sampled observation agrees; 2: completed with differences;
1: an incomplete run or invalid input. Reader differences are never waived.
"""
import argparse
from datetime import datetime, timezone
from hashlib import sha256
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from importlib.metadata import version
import json
import math
import mimetypes
from pathlib import Path
from threading import Thread
from urllib.parse import urlsplit


HERE = Path(__file__).resolve().parent
NAMES = ("rectangle", "rotated", "triangle", "circle")


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def inside_quad(quad, x, y):
    crosses = [(b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0])
               for a, b in zip(quad, quad[1:] + quad[:1])]
    return all(c >= -1e-5 for c in crosses) or all(c <= 1e-5 for c in crosses)


def shape_distance(name, x, y):
    # Analytic CSS shapes, independent of Fullbleed's region implementation.
    # PDF points have their origin at the bottom left; CSS starts at the top.
    u, v = x - 40, 160 - y
    if name == "rotated":
        c, s = math.cos(math.pi / 6), math.sin(math.pi / 6)
        u, v = c * u + s * v, -s * u + c * v
        return min(u, 100 - u, v, 40 - v)
    if name == "triangle":
        return min(u, v, (100 - u - v) / math.sqrt(2))
    if name == "circle":
        return 40 - math.hypot(u - 50, v - 50)
    return min(u, 100 - u, v, 100 - v)


def check_files(folder, hashes):
    for name, digest in hashes.items():
        path = (folder / name).resolve()
        if not path.is_relative_to(folder.resolve()):
            raise ValueError(f"Path outside artifact directory: {name}")
        if sha256(path.read_bytes()).hexdigest() != digest:
            raise ValueError(f"Artifact hash mismatch: {name}")


def observe(args, report):
    from pypdf import PdfReader
    import pypdfium2 as pdfium
    from pypdfium2 import raw
    from playwright.sync_api import sync_playwright

    manifest = json.loads((args.inputs / "inputs.json").read_text(encoding="utf-8"))
    lock = json.loads((HERE / "pdfjs-lock.json").read_text(encoding="utf-8"))
    if manifest["schema"] != "fullbleed.link-reader-inputs.v1":
        raise ValueError("Unrecognized input manifest")
    if [case["name"] for case in manifest["cases"]] != list(NAMES):
        raise ValueError("The four documented input cases are required, in order")
    check_files(args.pdfjs, lock["files"])
    assets = {"/viewer.html": HERE / "viewer.html"}
    assets.update({"/pdfjs/" + name: args.pdfjs / name for name in lock["files"]})
    for case in manifest["cases"]:
        name = case["name"]
        if set(case["files"]) != {f"{name}.{ext}" for ext in ("html", "css", "pdf", "png")}:
            raise ValueError(f"Incomplete input artifacts: {name}")
        check_files(args.inputs, case["files"])
        assets[f"/inputs/{name}.pdf"] = args.inputs / f"{name}.pdf"
    report.update(
        input_manifest=manifest,
        readers={"pdfium": str(pdfium.PDFIUM_INFO), "pdfjs": lock["version"],
                 "python_packages": {name: version(name) for name in
                                     ("pypdf", "pypdfium2", "Pillow", "playwright")}},
        harness_sha256={name: sha256((HERE / name).read_bytes()).hexdigest()
                        for name in ("check.py", "viewer.html", "pdfjs-lock.json")},
    )
    write_json(args.out / "inputs.json", manifest)
    for name in ("check.py", "viewer.html", "pdfjs-lock.json"):
        (args.out / name).write_bytes((HERE / name).read_bytes())

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            path = assets.get(urlsplit(self.path).path)
            if path is None:
                self.send_error(404)
                return
            data = path.read_bytes()
            mime = "text/javascript" if path.suffix == ".mjs" else mimetypes.guess_type(path)[0]
            self.send_response(200)
            self.send_header("Content-Type", mime or "application/octet-stream")
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, *_args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    origin = f"http://127.0.0.1:{server.server_port}"
    try:
        with sync_playwright() as playwright:
            launch = {"headless": True}
            if args.chromium:
                launch["executable_path"] = str(args.chromium)
            browser = playwright.chromium.launch(**launch)
            try:
                report["readers"]["chromium"] = browser.version
                context = browser.new_context(viewport={"width": 1000, "height": 900},
                                              device_scale_factor=1, color_scheme="light")
                context.route("**/*", lambda route: route.continue_()
                              if route.request.url.startswith(origin + "/") else route.abort())
                page = context.new_page()
                page.on("pageerror", lambda error: report["errors"].append(str(error)))
                page.goto(origin + "/viewer.html")
                page.wait_for_function("window.ready === true")
                for name in NAMES:
                    path = args.inputs / f"{name}.pdf"
                    (args.out / path.name).write_bytes(path.read_bytes())
                    parsed = PdfReader(path, strict=True)
                    if len(parsed.pages) != 1 or list(map(float, parsed.pages[0].mediabox)) != [0, 0, 200, 200]:
                        raise ValueError(f"{name}: unexpected page geometry")
                    annotations = [ref.get_object() for ref in parsed.pages[0].get("/Annots", [])]
                    if not annotations:
                        raise ValueError(f"{name}: installed engine did not emit PDF links")
                    quads = []
                    for annotation in annotations:
                        if annotation["/Subtype"] != "/Link" or annotation["/A"]["/URI"] != f"https://example.invalid/{name}":
                            raise ValueError(f"{name}: unexpected link action")
                        values = list(map(float, annotation["/QuadPoints"]))
                        if not values or len(values) % 8:
                            raise ValueError(f"{name}: malformed QuadPoints")
                        quads.extend([list(zip(values[i:i+8:2], values[i+1:i+8:2]))
                                      for i in range(0, len(values), 8)])
                    points = [{"x": x, "y": y, "expected": shape_distance(name, x, y) > 0}
                              for y in range(5, 196, 5) for x in range(5, 196, 5)
                              if abs(shape_distance(name, x, y)) > 1.5]
                    with pdfium.PdfDocument(path) as document:
                        view = document[0]
                        try:
                            bitmap = view.render(scale=2, draw_annots=True)
                            try:
                                image = bitmap.to_pil().convert("RGB").copy()
                            finally:
                                bitmap.close()
                            image.save(args.out / f"{name}-pdfium.png")
                            for point in points:
                                x, y = point["x"], point["y"]
                                point["geometry"] = any(inside_quad(q, x, y) for q in quads)
                                point["pdfium"] = bool(raw.FPDFLink_GetLinkAtPoint(view, x, y))
                                point["paint"] = min(image.getpixel((2*x, 400-2*y))) < 240
                        finally:
                            view.close()
                    observed = page.evaluate("url => window.draw(url)", origin + f"/inputs/{name}.pdf")
                    if observed["version"] != lock["version"]:
                        raise ValueError("Unexpected PDF.js version")
                    hits = page.evaluate("points => points.map(p => window.point(p.x,p.y))", points)
                    for point, hit in zip(points, hits):
                        point["pdfjs"] = bool(hit["href"])
                    selected = [next(p for p in points if p["expected"]),
                                next(p for p in points if not p["expected"] and not p["pdfjs"])]
                    mismatch = next((p for p in points if p["pdfjs"] != p["expected"]), None)
                    if mismatch:
                        selected.append(mismatch)
                    clicks = []
                    for point in selected:
                        hit = page.evaluate("p => window.point(p.x,p.y)", point)
                        before = page.evaluate("window.clicks.length")
                        page.mouse.click(hit["x"], hit["y"])
                        clicked = page.evaluate("n => window.clicks.slice(n)", before)
                        if bool(clicked) != point["pdfjs"]:
                            raise RuntimeError("Actual click disagreed with the DOM hit test")
                        clicks.append({"point": [point["x"], point["y"]],
                                       "expected": point["expected"], "clicked": clicked})
                    page.mouse.move(900, 800)
                    page.locator("#sheet").screenshot(path=args.out / f"{name}-pdfjs.png")
                    counts = {key: sum(p[key] != p["expected"] for p in points)
                              for key in ("geometry", "paint", "pdfium", "pdfjs")}
                    row = {"name": name, "samples": len(points), "mismatches": counts,
                           "pdf_sha256": sha256(path.read_bytes()).hexdigest(), "clicks": clicks,
                           "annotations": observed["annotations"]}
                    report["cases"].append(row)
                    write_json(args.out / f"{name}-points.json", points)
                    write_json(args.out / "report.json", report)
                    print(json.dumps({key: row[key] for key in ("name", "samples", "mismatches")}), flush=True)
            finally:
                browser.close()
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inputs", required=True, type=Path)
    parser.add_argument("--pdfjs", required=True, type=Path, help="Pinned pdfjs-dist package directory")
    parser.add_argument("--out", required=True, type=Path, help="New evidence directory")
    parser.add_argument("--chromium", type=Path, help="Optional Chromium executable")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    report = {"schema": "fullbleed.link-readers.v1", "status": "running", "cases": [], "errors": [],
              "started_at": datetime.now(timezone.utc).isoformat(),
              "scope": "Four authored shapes, 5pt grid, excluding points within 1.5pt of boundaries. "
                       "Paint, standard PDF QuadPoints, PDFium hit API and PDF.js DOM/clicks are separate observations. "
                       "This is not an accessibility, conformance, or exact-curve-boundary test."}
    try:
        observe(args, report)
        if report["errors"]:
            raise RuntimeError("Reader page errors; see report.json")
        different = any(count for case in report["cases"] for count in case["mismatches"].values())
        report["status"] = "differences" if different else "passed"
        code = 2 if different else 0
    except Exception as error:
        report.update(status="error", error=f"{type(error).__name__}: {error}")
        code = 1
    finally:
        report["completed_at"] = datetime.now(timezone.utc).isoformat()
        write_json(args.out / "report.json", report)
    print(json.dumps({"status": report["status"], "report": str(args.out / "report.json")}))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
