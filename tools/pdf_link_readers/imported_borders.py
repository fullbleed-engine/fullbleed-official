"""Reproduce border painting at an imported link's cropped edge.

Generate with an installed Fullbleed candidate, then check in the independent
reader environment. Exit 0 means these observations pass, 2 retains a visual
difference, and 1 means the check could not complete. No viewer parity claim.
"""
import argparse
from datetime import datetime, timezone
from hashlib import sha256
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from importlib.metadata import version
import json
import mimetypes
from pathlib import Path
from threading import Thread
from urllib.parse import urlsplit

from check import check_files, write_json

HERE = Path(__file__).resolve().parent
SCHEMA = "fullbleed.imported-border-inputs.v1"
CASES = {"solid": ("S", "I"), "dashed": ("D", "I"), "rounded": ("S", "I"),
         "beveled": ("B", "I"), "inset": ("I", "I"), "underline": ("U", "I"),
         "solid-outline": ("S", "O"), "appearance": ("S", "I")}
VARIANTS = ("source", "compose", "stamp")


def generate(args):
    import fullbleed
    import fullbleed._fullbleed as native
    from pypdf import PdfWriter
    from pypdf.generic import (ArrayObject, DecodedStreamObject, DictionaryObject,
                              NameObject, NumberObject, RectangleObject, TextStringObject)

    blank = PdfWriter()
    blank.add_blank_page(200, 200)
    template = args.out / "template.pdf"
    blank.write(template)
    manifest = dict(schema=SCHEMA, runtime=dict(version=version("fullbleed"),
        native_sha256=sha256(Path(native.__file__).read_bytes()).hexdigest(), pypdf=version("pypdf")),
        generator_sha256=sha256(Path(__file__).read_bytes()).hexdigest(), cases=[])
    for name, (style, highlight) in CASES.items():
        writer = PdfWriter()
        writer.pdf_header = "%PDF-1.7"
        page = writer.add_blank_page(200, 200)
        page.cropbox = RectangleObject([40, 0, 200, 200])
        annotation = DictionaryObject({
            NameObject("/Type"): NameObject("/Annot"), NameObject("/Subtype"): NameObject("/Link"),
            NameObject("/Rect"): RectangleObject([10, 20, 70, 60]),
            NameObject("/QuadPoints"): ArrayObject([NumberObject(v) for v in [10, 20, 70, 20, 70, 60, 10, 60]]),
            NameObject("/F"): NumberObject(4), NameObject("/H"): NameObject("/" + highlight),
            NameObject("/C"): ArrayObject([NumberObject(1), NumberObject(0), NumberObject(0)]),
            NameObject("/A"): DictionaryObject({NameObject("/S"): NameObject("/URI"),
                NameObject("/URI"): TextStringObject("https://example.invalid/cut-border/" + name)})})
        if name == "rounded":
            annotation[NameObject("/Border")] = ArrayObject([NumberObject(v) for v in [8, 8, 3]])
        else:
            annotation[NameObject("/BS")] = DictionaryObject({NameObject("/W"): NumberObject(3),
                NameObject("/S"): NameObject("/" + style),
                NameObject("/D"): ArrayObject([NumberObject(4), NumberObject(2)])})
        if name == "appearance":
            appearance = DecodedStreamObject()
            appearance.set_data(b"0 0 1 rg 40 15 20 10 re f")
            appearance.update({NameObject("/Type"): NameObject("/XObject"),
                NameObject("/Subtype"): NameObject("/Form"), NameObject("/BBox"): RectangleObject([0, 0, 60, 40]),
                NameObject("/Resources"): DictionaryObject()})
            ref = writer._add_object(appearance)
            annotation[NameObject("/AP")] = DictionaryObject({NameObject(key): ref for key in ("/N", "/R", "/D")})
        writer.add_annotation(0, annotation)
        source = args.out / f"{name}-source.pdf"
        writer.write(source)
        fullbleed.finalize_compose_pdf([("tpl", str(template))], [("tpl", 0, 0, 12.0, 18.0)],
            str(source), str(args.out / f"{name}-compose.pdf"), annotation_mode="none")
        fullbleed.finalize_stamp_pdf(str(template), str(source), str(args.out / f"{name}-stamp.pdf"), dx=12.0, dy=18.0)
        manifest["cases"].append(dict(name=name, files={f"{name}-{variant}.pdf":
            sha256((args.out / f"{name}-{variant}.pdf").read_bytes()).hexdigest() for variant in VARIANTS}))
    write_json(args.out / "inputs.json", manifest)
    return dict(status="generated", runtime=manifest["runtime"], cases=len(CASES))


def nonwhite(image, box=None):
    piece = image.crop(box) if box else image
    return sum(piece.getpixel((x, y)) != (255, 255, 255)
               for y in range(piece.height) for x in range(piece.width))


def observe(args, report):
    from PIL import Image
    from pypdf import PdfReader
    import pypdfium2 as pdfium
    from pypdfium2 import raw
    from playwright.sync_api import sync_playwright

    manifest = json.loads((args.inputs / "inputs.json").read_text(encoding="utf-8"))
    lock = json.loads((HERE / "pdfjs-lock.json").read_text(encoding="utf-8"))
    if manifest["schema"] != SCHEMA or [case["name"] for case in manifest["cases"]] != list(CASES):
        raise ValueError("The eight documented input cases are required, in order")
    check_files(args.pdfjs, lock["files"])
    assets = {"/viewer.html": HERE / "viewer.html"}
    assets.update({"/pdfjs/" + name: args.pdfjs / name for name in lock["files"]})
    for case in manifest["cases"]:
        expected = {f"{case['name']}-{variant}.pdf" for variant in VARIANTS}
        if set(case["files"]) != expected:
            raise ValueError("Incomplete PDF specimens")
        check_files(args.inputs, case["files"])
        for name in expected:
            assets["/inputs/" + name] = args.inputs / name
            (args.out / name).write_bytes((args.inputs / name).read_bytes())
    report.update(input_manifest=manifest, readers=dict(pdfium=str(pdfium.PDFIUM_INFO), pdfjs=lock["version"],
        python_packages={name: version(name) for name in ("pypdf", "pypdfium2", "Pillow", "playwright")}),
        harness_sha256={name: sha256((HERE / name).read_bytes()).hexdigest()
                        for name in ("imported_borders.py", "check.py", "viewer.html", "pdfjs-lock.json")})
    write_json(args.out / "inputs.json", manifest)
    for name in report["harness_sha256"]:
        (args.out / name).write_bytes((HERE / name).read_bytes())

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            path = assets.get(urlsplit(self.path).path)
            if path is None:
                self.send_error(404)
                return
            data = path.read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "text/javascript" if path.suffix == ".mjs" else
                             mimetypes.guess_type(path)[0] or "application/octet-stream")
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
                context = browser.new_context(viewport={"width": 1000, "height": 900}, device_scale_factor=1)
                context.route("**/*", lambda route: route.continue_()
                              if route.request.url.startswith(origin + "/") else route.abort())
                page = context.new_page()
                page.on("pageerror", lambda error: report["errors"].append(str(error)))
                page.goto(origin + "/viewer.html")
                page.wait_for_function("window.ready === true")
                for name, (_style, highlight) in CASES.items():
                    url = "https://example.invalid/cut-border/" + name
                    for variant in VARIANTS:
                        source = variant == "source"
                        path = args.inputs / f"{name}-{variant}.pdf"
                        document = PdfReader(path, strict=True)
                        annotation, = [ref.get_object() for ref in document.pages[0]["/Annots"]]
                        rect = [10, 20, 70, 60] if source else [52, 38, 82, 78]
                        if (len(document.pages) != 1 or list(map(float, annotation["/Rect"])) != rect or
                                annotation["/H"] != "/" + highlight or annotation["/A"]["/URI"] != url or
                                annotation["/Subtype"] != "/Link"):
                            raise ValueError(f"{name}-{variant}: changed link geometry, action, or highlight mode")
                        inside = (55, 45) if source else (67, 63)
                        with pdfium.PdfDocument(path) as pdf:
                            view = pdf[0]
                            try:
                                bitmap = view.render(scale=2, draw_annots=True)
                                try:
                                    raster = bitmap.to_pil().convert("RGB").copy()
                                finally:
                                    bitmap.close()
                                if not raw.FPDFLink_GetLinkAtPoint(view, *inside):
                                    raise ValueError("The retained link is not active in PDFium")
                                if not source and raw.FPDFLink_GetLinkAtPoint(view, 51.5, 63):
                                    raise ValueError("The clipped-away area remains active in PDFium")
                            finally:
                                view.close()
                        raster.save(args.out / f"{name}-{variant}-pdfium.png")
                        drawn = page.evaluate("url => window.draw(url)", origin + "/inputs/" + path.name)
                        if drawn["version"] != lock["version"]:
                            raise ValueError("Unexpected PDF.js version")
                        page.mouse.move(800, 50)
                        screenshot = args.out / f"{name}-{variant}-pdfjs.png"
                        page.locator("#sheet").screenshot(path=screenshot)
                        browser_raster = Image.open(screenshot).convert("RGB")
                        point = page.evaluate("p => window.point(p[0],p[1])", inside)
                        page.evaluate("window.clicks = []")
                        page.mouse.click(point["x"], point["y"])
                        clicks = page.evaluate("window.clicks")
                        if clicks != [url]:
                            raise ValueError("The retained link does not activate in PDF.js")
                        if not source and page.evaluate("window.point(51.5, 63).href"):
                            raise ValueError("The clipped-away area remains active in PDF.js")
                        box = (0, 310, 2, 330) if source else (104, 274, 106, 294)
                        paint = dict(pdfium=nonwhite(raster, box), pdfjs=nonwhite(browser_raster, box))
                        if source and any(paint.values()):
                            raise ValueError("Invalid source: the intended cut band already contains artwork")
                        # A blank result cannot pass merely by removing the unwanted edge.
                        visible = nonwhite(raster)
                        if not source and not visible:
                            raise ValueError("Composition discarded all border/appearance artwork")
                        if name == "appearance" and not visible:
                            raise ValueError("The explicit source appearance is not a functioning control")
                        report["cases"].append(dict(name=name, variant=variant, cut_band_pixels=40,
                            cut_band_nonwhite=paint, pdfium_visible_pixels=visible, clicked=clicks,
                            highlight=str(annotation["/H"])))
                        write_json(args.out / "report.json", report)
                context.close()
            finally:
                browser.close()
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
    if report["errors"]:
        raise RuntimeError("PDF.js reported browser errors")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    render = commands.add_parser("generate", help="Create independent source PDFs and candidate outputs")
    render.add_argument("--out", type=Path, required=True)
    check = commands.add_parser("check", help="Inspect the actual PDF.js annotation layer and PDFium")
    check.add_argument("--inputs", type=Path, required=True)
    check.add_argument("--pdfjs", type=Path, required=True)
    check.add_argument("--out", type=Path, required=True)
    check.add_argument("--chromium", type=Path)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    if args.command == "generate":
        print(json.dumps(generate(args)))
        return 0
    report = dict(schema="fullbleed.imported-border-readers.v1", status="running", cases=[], errors=[],
        started_at=datetime.now(timezone.utc).isoformat(),
        scope="Eight imported-link specimens: cut-edge paint, retained artwork, target activation, and stored H modes. Not complete border-style, mouse-highlight, accessibility, or reader parity.")
    try:
        observe(args, report)
        different = any(any(case["cut_band_nonwhite"].values()) for case in report["cases"])
        report["status"] = "differences" if different else "passed"
        code = 2 if different else 0
    except Exception as error:
        report["status"] = "incomplete"
        report["errors"].append(str(error))
        code = 1
    report["completed_at"] = datetime.now(timezone.utc).isoformat()
    write_json(args.out / "report.json", report)
    print(json.dumps(dict(status=report["status"], cases=len(report["cases"]), errors=report["errors"])))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
