#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Build compact OFL outline substitutes for native Standard 14 previews.

Development only: requires fontTools==4.65.0. No font tools or downloads are
needed to build or run Fullbleed; the generated fonts are checked in.
"""
from __future__ import annotations

import argparse
from hashlib import sha256
from io import BytesIO
import json
from pathlib import Path
import tarfile
from urllib.request import urlopen

import fontTools
from fontTools.fontBuilder import FontBuilder
from fontTools.misc.roundTools import otRound
from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / "src/preview_fonts"
DATA = ROOT / "tools/data/base14"
INPUTS = ROOT / "tools/data/preview_fonts/sources.json"
EPOCH = 2082844800  # 1970-01-01, in the OpenType epoch; never wall-clock time.

# Adobe Glyph List private-use characters have modern Unicode equivalents.
# These aliases affect preview outlines only, never PDF text or extraction.
ALIASES = {
    "commaaccent": 0x0326,
    "Omega": 0x03A9,
    "mu": 0x03BC,
    "radicalex": 0x23AF,
    "arrowvertex": 0x23D0,
    "arrowhorizex": 0x23AF,
    "registerserif": 0x00AE,
    "copyrightserif": 0x00A9,
    "trademarkserif": 0x2122,
    "registersans": 0x00AE,
    "copyrightsans": 0x00A9,
    "trademarksans": 0x2122,
    "parenlefttp": 0x239B, "parenleftex": 0x239C, "parenleftbt": 0x239D,
    "parenrighttp": 0x239E, "parenrightex": 0x239F, "parenrightbt": 0x23A0,
    "bracketlefttp": 0x23A1, "bracketleftex": 0x23A2, "bracketleftbt": 0x23A3,
    "bracketrighttp": 0x23A4, "bracketrightex": 0x23A5, "bracketrightbt": 0x23A6,
    "bracelefttp": 0x23A7, "braceleftmid": 0x23A8, "braceleftbt": 0x23A9,
    "braceex": 0x23AA,
    "bracerighttp": 0x23AB, "bracerightmid": 0x23AC, "bracerightbt": 0x23AD,
    "integralex": 0x23AE,
}
UNSUPPORTED = {("Symbol", "apple"): "Vendor-private logo U+F8FF; absent from the licensed source fonts and the built-in Symbol encoding."}


def verified(raw, entry, label):
    assert len(raw) == entry["bytes"] and sha256(raw).hexdigest() == entry["sha256"], label
    return raw


def load_sources(cache):
    manifest = json.loads(INPUTS.read_text(encoding="utf-8"))
    archive = manifest["liberation_archive"]
    cache.mkdir(parents=True, exist_ok=True)
    path = cache / "liberation-fonts-ttf-2.1.5.tar.gz"
    if not path.is_file():
        with urlopen(archive["url"], timeout=60) as response:
            raw = response.read(10_000_000)
        path.write_bytes(verified(raw, archive, archive["url"]))
    raw = verified(path.read_bytes(), archive, str(path))
    fonts = {}
    with tarfile.open(fileobj=BytesIO(raw), mode="r:gz") as bundle:
        for entry in manifest["fonts"]:
            if "archive_member" in entry:
                # Read only the named member; never extract archive paths.
                stream = bundle.extractfile(entry["archive_member"])
                assert stream is not None, entry["file"]
                data = stream.read()
            else:
                data = (ROOT / entry["path"]).read_bytes()
            fonts[entry["file"]] = TTFont(BytesIO(verified(data, entry, entry["file"])), recalcTimestamp=False)
    for entry in manifest["notices"]:
        verified((DEST / entry["file"]).read_bytes(), entry, entry["file"])
    return manifest, fonts


def afm_glyphs():
    pins = json.loads((DATA / "sources.json").read_text(encoding="utf-8"))
    for name, entry in pins["files"].items():
        verified((DATA / name).read_bytes(), entry, name)
    mappings = {}
    for filename in ["agl-glyphlist.txt", "agl-zapfdingbats.txt"]:
        for line in (DATA / filename).read_text(encoding="utf-8").splitlines():
            if line and not line.startswith("#"):
                name, codes = line.split(";")
                values = codes.split()
                if len(values) == 1:
                    mappings[name] = int(values[0], 16)
    result = {}
    for path in sorted(DATA.glob("*.afm")):
        glyphs = {}
        for line in path.read_text(encoding="latin-1").splitlines():
            if not line.startswith("C "):
                continue
            fields = dict(part.strip().split(" ", 1) for part in line.split(";") if part.strip())
            name = fields["N"]
            glyphs[name] = dict(code=int(fields["C"]), unicode=mappings[name], width=int(fields["WX"]),
                                bbox=[int(value) for value in fields["B"].split()])
        result[path.stem] = glyphs
    return result


def make_font(pdf_name, glyphs, fonts):
    family = {"Helvetica": "Sans", "Times": "Serif", "Courier": "Mono"}.get(pdf_name.split("-")[0])
    style = {"Roman": "Regular", "Oblique": "Italic", "BoldOblique": "BoldItalic"}.get(
        pdf_name.split("-", 1)[1], pdf_name.split("-", 1)[1]) if "-" in pdf_name else "Regular"
    if family:
        preferred = [f"Liberation{family}-{style}.ttf"]
        upem = 2048
    else:
        family = "Symbol" if pdf_name == "Symbol" else "Dingbats"
        preferred = (["NotoSansMath-Regular.ttf"] if pdf_name == "Symbol" else []) + [
            "NotoSansSymbols2-Regular.ttf", "NotoSansSymbols-Regular.ttf",
            "LiberationSerif-Regular.ttf", "LiberationSans-Regular.ttf", "NotoSans-Regular.ttf"]
        upem = 1000
    family_name = f"Fullbleed Preview {family}"
    ps_name = f"FullbleedPreview{family}-{style}"
    cmap, outlines, metrics, used, substitutions, unsupported = {}, {}, {}, set(), [], []
    blank = TTGlyphPen(None).glyph()
    outlines[".notdef"] = blank
    metrics[".notdef"] = (upem // 2, 0)
    for name, entry in sorted(glyphs.items()):
        if (pdf_name, name) in UNSUPPORTED:
            assert entry["code"] < 0, "Never omit a built-in encoded glyph"
            unsupported.append(dict(glyph=name, unicode=f"U+{entry['unicode']:04X}", reason=UNSUPPORTED[pdf_name, name]))
            continue
        code = ALIASES.get(name, entry["unicode"])
        candidates = preferred
        if name.endswith("serif"):
            candidates = ["LiberationSerif-Regular.ttf"] + preferred
        elif name.endswith("sans"):
            candidates = ["LiberationSans-Regular.ttf"] + preferred
        selected = next((candidate for candidate in candidates if code in fonts[candidate].getBestCmap()), None)
        assert selected is not None, (pdf_name, name, hex(code))
        source = fonts[selected]
        source_name = source.getBestCmap()[code]
        glyph_set = source.getGlyphSet()
        recording = DecomposingRecordingPen(glyph_set)
        glyph_set[source_name].draw(recording)
        scale = upem / source["head"].unitsPerEm
        translate = 0
        if name == "commaaccent":
            # Convert the combining mark's negative bearing into the spacing
            # Adobe accent's bearing, retaining the same outline.
            translate = entry["bbox"][0] * upem / 1000 - source["glyf"][source_name].xMin * scale
        pen = TTGlyphPen(None)
        recording.replay(TransformPen(pen, (scale, 0, 0, scale, translate, 0)))
        glyph = pen.glyph()
        glyph.recalcBounds(None)
        outlines[name] = glyph
        metrics[name] = (otRound(entry["width"] * upem / 1000), getattr(glyph, "xMin", 0))
        cmap[entry["unicode"]] = name
        used.add(selected)
        if code != entry["unicode"]:
            substitutions.append(dict(glyph=name, unicode=f"U+{entry['unicode']:04X}", outline_unicode=f"U+{code:04X}"))

    # Match the runtime's WinAnsi aliases without adding duplicate outlines.
    if "space" in outlines:
        cmap[0x00A0] = "space"
    if "hyphen" in outlines:
        cmap[0x00AD] = "hyphen"

    builder = FontBuilder(upem, isTTF=True)
    builder.setupGlyphOrder(list(outlines))
    builder.setupCharacterMap(cmap)
    builder.setupGlyf(outlines)
    builder.setupHorizontalMetrics(metrics)
    ascent = max(getattr(g, "yMax", 0) for g in outlines.values())
    descent = min(getattr(g, "yMin", 0) for g in outlines.values())
    builder.setupHorizontalHeader(ascent=ascent, descent=descent)
    copyrights = sorted({fonts[name]["name"].getDebugName(0) for name in used})
    builder.setupNameTable(dict(
        familyName=family_name, styleName=style, uniqueFontIdentifier=f"Fullbleed:{ps_name}:1.000",
        fullName=f"{family_name} {style}", psName=ps_name, version="Version 1.000",
        copyright="\n".join(copyrights),
        description="Modified OFL outline substitute for Fullbleed native previews. Subset and renamed; decomposed, unhinted outlines; Adobe AFM advances; AGL preview aliases. Not an original Standard 14 font.",
        licenseDescription="SIL Open Font License, Version 1.1. Complete original notices and license texts accompany this font in src/preview_fonts/LICENSE-*.txt.",
        licenseInfoURL="https://openfontlicense.org/open-font-license-official-text/"))
    bold, italic = "Bold" in style, "Italic" in style
    builder.setupOS2(sTypoAscender=ascent, sTypoDescender=descent, usWinAscent=ascent,
                     usWinDescent=-descent, usWeightClass=700 if bold else 400,
                     fsType=0, fsSelection=(32 if bold else 0) | (1 if italic else 0) | (64 if not bold and not italic else 0))
    builder.setupPost(italicAngle=-12 if italic else 0, isFixedPitch=int(family == "Mono"))
    builder.font["head"].created = builder.font["head"].modified = EPOCH
    builder.font["head"].macStyle = int(bold) | (int(italic) << 1)
    stream = BytesIO()
    builder.save(stream)
    raw = stream.getvalue()
    filename = ps_name + ".ttf"
    return filename, raw, dict(pdf_name=pdf_name, file=filename, bytes=len(raw), sha256=sha256(raw).hexdigest(),
                              modified=True, source_fonts=sorted(used), afm_glyphs=len(glyphs),
                              covered_glyphs=len(outlines)-1, aliases=substitutions, unsupported=unsupported)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--cache", type=Path, default=ROOT / "target/preview-font-sources")
    args = parser.parse_args()
    assert fontTools.__version__ == "4.65.0", "Use fontTools==4.65.0 for reproducible generation"
    manifest, fonts = load_sources(args.cache.resolve())
    report = dict(schema="fullbleed.preview_fonts.v1", license="OFL-1.1", generator="tools/generate_preview_fonts.py",
                  generator_version=fontTools.__version__, inputs_sha256=sha256(INPUTS.read_bytes()).hexdigest(),
                  scope="Fixed outline substitutes, not original Standard 14 designs. All built-in encoded AFM glyphs covered. The unencoded vendor-private Symbol apple logo is unsupported.", fonts=[])
    for pdf_name, glyphs in afm_glyphs().items():
        filename, raw, entry = make_font(pdf_name, glyphs, fonts)
        if args.check:
            assert (DEST / filename).read_bytes() == raw, f"Regenerate {filename}"
        else:
            (DEST / filename).write_bytes(raw)
        report["fonts"].append(entry)
    raw = (json.dumps(report, indent=2) + "\n").encode()
    if args.check:
        assert (DEST / "sources.json").read_bytes() == raw, "Regenerate preview font manifest"
    else:
        (DEST / "sources.json").write_bytes(raw)
    print(json.dumps(dict(ok=True, check=args.check, fonts=len(report["fonts"]), bytes=sum(f["bytes"] for f in report["fonts"]),
                          covered_glyphs=sum(f["covered_glyphs"] for f in report["fonts"]),
                          unsupported=[(f["pdf_name"], g["glyph"]) for f in report["fonts"] for g in f["unsupported"]])))


if __name__ == "__main__":
    main()
