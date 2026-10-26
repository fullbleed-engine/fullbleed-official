"""Reproduce the small OFL font fixture from the pinned upstream font."""
import argparse
from hashlib import sha256
import json
from pathlib import Path

from fontTools import __version__
from fontTools import subset
from fontTools.ttLib import TTFont

SOURCE_URL = "https://raw.githubusercontent.com/bin-utokyo/davis/343d4d27537b7809638140df082901a1d9c7dab9/crates/davis-document/assets/fonts/BIZUDPGothic-Regular.ttf"
SOURCE_SHA256 = "258d7156c165f2ff774b6efee637c22c3b950de0d8a10e501137061bc8085d01"
TEXT = "東京都の交通計画と地域調査の結果を説明する日本語の文章です吾輩は猫である。名前はまだ無い。「小さいゃっ」你好世界"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    args = parser.parse_args()
    assert __version__ == "4.65.0", "Use the recorded FontTools version"
    assert sha256(args.source.read_bytes()).hexdigest() == SOURCE_SHA256
    font = TTFont(args.source, recalcTimestamp=False)
    codepoints = sorted(set(range(32, 127)) | {ord(char) for char in TEXT})
    assert set(codepoints) <= set(font.getBestCmap())
    options = subset.Options()
    options.name_IDs = ["*"]
    options.name_languages = ["*"]
    options.name_legacy = True
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(unicodes=codepoints)
    subsetter.subset(font)
    names = {1: "Fullbleed CJK Test", 2: "Regular", 3: "FullbleedCJKTest-Regular-1",
             4: "Fullbleed CJK Test Regular", 6: "FullbleedCJKTest-Regular",
             16: "Fullbleed CJK Test", 17: "Regular"}
    for record in font["name"].names:
        if record.nameID in names:
            record.string = names[record.nameID].encode(record.getEncoding())
    target = Path(__file__).resolve().parent / "FullbleedCJKTest-Regular.ttf"
    font.save(target, reorderTables=True)
    font.close()
    result = dict(source=SOURCE_URL, sourceSha256=SOURCE_SHA256,
                  subsetSha256=sha256(target.read_bytes()).hexdigest(),
                  bytes=target.stat().st_size, fontToolsVersion=__version__,
                  codepoints=codepoints, license="OFL-1.1",
                  note="Renamed test-only subset of BIZ UDPGothic; no upstream endorsement.")
    target.with_name("font-source.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(dict(bytes=result["bytes"], sha256=result["subsetSha256"])))


if __name__ == "__main__":
    main()
