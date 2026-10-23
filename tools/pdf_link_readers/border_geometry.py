"""Check the declared geometry of the eight imported_borders.py specimens.

This measures Fullbleed's explicit appearances against independently authored
PDF dimensions. It does not treat a reader's fallback border as the oracle.
Exit 0: the bounded checks pass; 2: a geometry difference; 1: invalid inputs or
an incomplete check. Use a new output directory for each observation.
"""
import argparse
from datetime import datetime, timezone
from hashlib import sha256
from importlib.metadata import version
import json
from pathlib import Path

from PIL import Image, ImageChops
from pypdf import PdfReader
from pypdf.generic import ContentStream
import pypdfium2 as pdfium

CASES = ("solid", "dashed", "rounded", "beveled", "inset", "underline",
         "solid-outline", "appearance")
VARIANTS = ("source", "compose", "stamp")
SCALE = 8


class GeometryDifference(Exception):
    pass


def require(condition, message):
    if not condition:
        raise GeometryDifference(message)


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def pixel(image, x, y):
    return image.getpixel((int(x * SCALE), int((200 - y) * SCALE)))


def runs(values):
    result = []
    for value in values:
        if result and result[-1]["painted"] == value:
            result[-1]["pixels"] += 1
        else:
            result.append(dict(painted=value, pixels=1))
    for row in result:
        row["points"] = row["pixels"] / SCALE
    return result


def render(path):
    with pdfium.PdfDocument(path) as document:
        page = document[0]
        try:
            bitmap = page.render(scale=SCALE, draw_annots=True)
            try:
                return bitmap.to_pil().convert("RGB").copy()
            finally:
                bitmap.close()
        finally:
            page.close()


def observe(args, report):
    manifest = json.loads((args.inputs / "inputs.json").read_text(encoding="utf-8"))
    if (manifest["schema"] != "fullbleed.imported-border-inputs.v1" or
            [case["name"] for case in manifest["cases"]] != list(CASES)):
        raise ValueError("The eight documented input cases are required, in order")
    for case in manifest["cases"]:
        expected = {f"{case['name']}-{variant}.pdf" for variant in VARIANTS}
        if set(case["files"]) != expected:
            raise ValueError("Incomplete PDF specimens")
        for name, expected_hash in case["files"].items():
            path = args.inputs / name
            if digest(path) != expected_hash:
                raise ValueError(f"Input hash mismatch: {name}")
            (args.out / name).write_bytes(path.read_bytes())
    report["input_manifest"] = manifest
    (args.out / "inputs.json").write_bytes((args.inputs / "inputs.json").read_bytes())
    (args.out / Path(__file__).name).write_bytes(Path(__file__).read_bytes())

    for name in CASES:
        source_pdf = PdfReader(args.inputs / f"{name}-source.pdf", strict=True)
        source_page = source_pdf.pages[0]
        source = source_page["/Annots"][0].get_object()
        if (len(source_pdf.pages) != 1 or list(source_page.mediabox) != [0, 0, 200, 200] or
                list(source_page.cropbox) != [40, 0, 200, 200] or
                len(source_page["/Annots"]) != 1 or source["/Subtype"] != "/Link" or
                list(source["/C"]) != [1, 0, 0] or
                list(source["/Rect"]) != [10, 20, 70, 60]):
            raise ValueError(f"{name}: unexpected independent source geometry")
        if name == "rounded":
            if list(source["/Border"]) != [8, 8, 3]:
                raise ValueError("Unexpected source corner radii or width")
        else:
            style = {"dashed": "/D", "beveled": "/B", "inset": "/I",
                     "underline": "/U"}.get(name, "/S")
            if float(source["/BS"]["/W"]) != 3 or source["/BS"]["/S"] != style:
                raise ValueError(f"{name}: unexpected source border width or style")
        if name == "dashed" and list(source["/BS"]["/D"]) != [4, 2]:
            raise ValueError("Unexpected source dash pattern")

        case = dict(name=name, variants=[])
        report["cases"].append(case)
        images = []
        for variant in ("compose", "stamp"):
            label = f"{name}-{variant}"
            path = args.inputs / f"{label}.pdf"
            reader = PdfReader(path, strict=True)
            require(len(reader.pages) == 1 and
                    list(reader.pages[0].mediabox) == [0, 0, 200, 200],
                    f"{label}: changed output page")
            annotation, = [ref.get_object() for ref in reader.pages[0]["/Annots"]]
            require(list(annotation["/Rect"]) == [52, 38, 82, 78],
                    f"{label}: changed visible rectangle")
            require(list(annotation["/Border"]) == [0, 0, 0] and "/BS" not in annotation,
                    f"{label}: separate fallback border remains enabled")
            require(annotation["/Subtype"] == "/Link" and
                    annotation["/H"] == source["/H"] and
                    annotation["/A"]["/URI"] == source["/A"]["/URI"],
                    f"{label}: changed target, subtype or stored highlight mode")
            appearance = annotation["/AP"]["/N"].get_object()
            ops = ContentStream(appearance, reader).operations
            image = render(path)
            images.append(image)
            dest = args.out / f"{label}.png"
            image.save(dest)
            bounds = ImageChops.difference(image, Image.new("RGB", image.size, "white")).getbbox()
            row = dict(variant=variant, pdf_sha256=digest(path), png_sha256=digest(dest),
                       paint_bbox_pixels=bounds, target_and_highlight_retained=True)
            case["variants"].append(row)
            require(bounds is not None and bounds[0] >= 52 * SCALE and
                    bounds[1] >= (200 - 78) * SCALE and bounds[2] <= 82 * SCALE and
                    bounds[3] <= (200 - 38) * SCALE,
                    f"{label}: missing paint or paint outside the clipped rectangle")
            row["paint_within_clipped_rect"] = True

            if name in ("solid", "solid-outline", "underline", "beveled", "inset"):
                mismatches = 0
                color_mismatches = 0
                for ix in range(70):
                    x = 50.25 + ix * .5
                    for iy in range(90):
                        y = 36.25 + iy * .5
                        expected = (52 < x < 82 and 38 < y < 78 and
                                    ((y < 41) if name == "underline" else
                                     (y < 41 or y > 75 or x > 79)))
                        color = pixel(image, x, y)
                        mismatches += expected != (color != (255, 255, 255))
                        if expected and name in ("solid", "solid-outline", "underline"):
                            color_mismatches += color != (255, 0, 0)
                row.update(analytic_mask_samples=6300, analytic_mask_mismatches=mismatches,
                           solid_color_mismatches=color_mismatches)
                require(mismatches == 0, f"{label}: {mismatches} analytic mask differences")
                require(color_mismatches == 0, f"{label}: {color_mismatches} solid color differences")
            if name in ("solid", "solid-outline", "dashed", "rounded", "underline"):
                widths = [float(args[0]) for args, operator in ops if operator == b"w"]
                row["declared_stroke_widths_pt"] = widths
                require(widths == [3.0], f"{label}: changed declared stroke width")
            if name == "dashed":
                dash_ops = [([float(x) for x in args[0]], float(args[1]))
                            for args, operator in ops if operator == b"d"]
                require(dash_ops == [([4.0, 2.0], 0.0)], f"{label}: changed dash pattern")
                lengths = runs(pixel(image, 52 + (i + .5) / SCALE, 39.5) != (255, 255, 255)
                               for i in range(26 * SCALE))
                row["measured_bottom_edge_runs"] = lengths
                full = lengths[1:-1]
                require(any(r["painted"] for r in full) and
                        any(not r["painted"] for r in full) and
                        all(abs(r["points"] - (4 if r["painted"] else 2)) <= 1 / SCALE
                            for r in full), f"{label}: rendered dash lengths differ")
            if name == "rounded":
                require(sum(operator == b"c" for _, operator in ops) == 4 and
                        pixel(image, 81.75, 77.75) == (255, 255, 255) and
                        pixel(image, 81.75, 38.25) == (255, 255, 255) and
                        pixel(image, 80.5, 58) == (255, 0, 0) and
                        pixel(image, 60, 76.5) == (255, 0, 0),
                        f"{label}: rounded corner or straight-edge samples differ")
                row["rounded_corner_and_edge_samples_match"] = True
            if name in ("beveled", "inset"):
                top, bottom = pixel(image, 60, 76.5), pixel(image, 60, 39.5)
                require(top != bottom and top[0] > top[1] and bottom[0] > bottom[1] and
                        (sum(top) > sum(bottom)) == (name == "beveled"),
                        f"{label}: opposed shading direction differs")
                row.update(top_edge_rgb=top, bottom_edge_rgb=bottom,
                           opposed_shading_direction_verified=True)
        require(ImageChops.difference(*images).getbbox() is None,
                f"{name}: composition and stamping pixels differ")
        case["compose_and_stamp_pixels_identical"] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inputs", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    report = dict(schema="fullbleed.imported-border-geometry.v1", status="running",
                  started_at=datetime.now(timezone.utc).isoformat(), cases=[], errors=[],
                  harness_sha256=digest(Path(__file__)),
                  readers=dict(pdfium=str(pdfium.PDFIUM_INFO),
                               packages={name: version(name) for name in
                                         ("pypdf", "pypdfium2", "Pillow")}),
                  source_rect_pt=[10, 20, 70, 60], crop_left_pt=40, placement_pt=[12, 18],
                  expected_visible_rect_pt=[52, 38, 82, 78], pixels_per_point=SCALE,
                  sample_step_pt=.5, sample_margin_from_edges_pt=.25,
                  limits=["Eight axis-aligned RGB 3pt specimens; not general imported-PDF parity.",
                          "Rounded borders receive sampled checks, not exact curve-raster proof.",
                          "Bevel/inset checks measure placement and opposed shading, not a prescribed color formula.",
                          "Stored highlight modes are checked; highlight interaction, keyboard navigation and assistive technology are unqualified."])
    try:
        observe(args, report)
        report["status"], code = "passed", 0
    except GeometryDifference as error:
        report["status"], code = "differences", 2
        report["errors"].append(str(error))
    except Exception as error:
        report["status"], code = "incomplete", 1
        report["errors"].append(f"{type(error).__name__}: {error}")
    report["analytic_mask_samples"] = sum(v.get("analytic_mask_samples", 0)
        for case in report["cases"] for v in case["variants"])
    report["analytic_mask_mismatches"] = sum(v.get("analytic_mask_mismatches", 0)
        for case in report["cases"] for v in case["variants"])
    report["completed_at"] = datetime.now(timezone.utc).isoformat()
    (args.out / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: report[key] for key in
                     ("status", "analytic_mask_samples", "analytic_mask_mismatches", "errors")}))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
