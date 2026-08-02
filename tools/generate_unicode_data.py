"""Regenerate FullBleed's compact Unicode normalization tables.

The checked-in output is runtime dependency-free.  Generation is pinned to the
Unicode 14 database used by the shaping contract being replaced.
"""

from __future__ import annotations

from pathlib import Path
import hashlib
import unicodedata
import urllib.request


UNICODE_VERSION = "14.0.0"
ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "src" / "unicode_data.rs"
ARABIC_SHAPING_URL = "https://www.unicode.org/Public/14.0.0/ucd/ArabicShaping.txt"
ARABIC_SHAPING_SHA256 = "c7698811e9adb6cc98fb996a7de4be2b6532f2ac67e76055cc8afdbf6ee18af3"
BIDI_MIRRORING_URL = "https://www.unicode.org/Public/14.0.0/ucd/BidiMirroring.txt"
BIDI_MIRRORING_SHA256 = "7a5c74cedc1616a9af0a9d22e108ae592d86fe93649c144ae6ba49f193a44122"


def _hex(value: int) -> str:
    return f"0x{value:x}"


def main() -> None:
    if unicodedata.unidata_version != UNICODE_VERSION:
        raise RuntimeError(
            f"expected Unicode {UNICODE_VERSION}, got {unicodedata.unidata_version}"
        )

    class_ranges: list[list[int]] = []
    transparent_ranges: list[list[int]] = []
    decimal_ranges: list[list[int]] = []
    for codepoint in range(0x110000):
        category = unicodedata.category(chr(codepoint))
        if category in {"Mn", "Me", "Cf"}:
            if transparent_ranges and transparent_ranges[-1][1] + 1 == codepoint:
                transparent_ranges[-1][1] = codepoint
            else:
                transparent_ranges.append([codepoint, codepoint])
        if category == "Nd":
            if decimal_ranges and decimal_ranges[-1][1] + 1 == codepoint:
                decimal_ranges[-1][1] = codepoint
            else:
                decimal_ranges.append([codepoint, codepoint])
        combining_class = unicodedata.combining(chr(codepoint))
        if not combining_class:
            continue
        if (
            class_ranges
            and class_ranges[-1][1] + 1 == codepoint
            and class_ranges[-1][2] == combining_class
        ):
            class_ranges[-1][1] = codepoint
        else:
            class_ranges.append([codepoint, codepoint, combining_class])

    with urllib.request.urlopen(ARABIC_SHAPING_URL, timeout=30) as response:
        arabic_shaping_bytes = response.read()
    actual_hash = hashlib.sha256(arabic_shaping_bytes).hexdigest()
    if actual_hash != ARABIC_SHAPING_SHA256:
        raise RuntimeError(f"ArabicShaping.txt hash mismatch: {actual_hash}")
    joining_values: dict[int, int] = {}
    for raw_line in arabic_shaping_bytes.decode("utf-8-sig").splitlines():
        line = raw_line.split("#", 1)[0].strip()
        if not line:
            continue
        fields = [field.strip() for field in line.split(";")]
        codepoint_field, joining_type, joining_group = fields[0], fields[2], fields[3]
        if joining_group == "ALAPH":
            joining_value = 4
        elif joining_group == "DALATH RISH":
            joining_value = 5
        else:
            joining_value = {"U": 0, "L": 1, "R": 2, "D": 3, "C": 3, "T": 7}[joining_type]
        if ".." in codepoint_field:
            start_text, end_text = codepoint_field.split("..", 1)
        else:
            start_text = end_text = codepoint_field
        for codepoint in range(int(start_text, 16), int(end_text, 16) + 1):
            joining_values[codepoint] = joining_value
    joining_ranges: list[list[int]] = []
    for codepoint, joining_type in sorted(joining_values.items()):
        if (
            joining_ranges
            and joining_ranges[-1][1] + 1 == codepoint
            and joining_ranges[-1][2] == joining_type
        ):
            joining_ranges[-1][1] = codepoint
        else:
            joining_ranges.append([codepoint, codepoint, joining_type])

    with urllib.request.urlopen(BIDI_MIRRORING_URL, timeout=30) as response:
        bidi_mirroring_bytes = response.read()
    actual_hash = hashlib.sha256(bidi_mirroring_bytes).hexdigest()
    if actual_hash != BIDI_MIRRORING_SHA256:
        raise RuntimeError(f"BidiMirroring.txt hash mismatch: {actual_hash}")
    bidi_mirrors: list[tuple[int, int]] = []
    for raw_line in bidi_mirroring_bytes.decode("utf-8-sig").splitlines():
        line = raw_line.split("#", 1)[0].strip()
        if not line:
            continue
        source, target = (field.strip() for field in line.split(";", 1))
        bidi_mirrors.append((int(source, 16), int(target, 16)))

    decompositions: list[tuple[int, int, int]] = []
    decomposition_values: list[int] = []
    compositions: list[tuple[int, int]] = []
    for codepoint in range(0x110000):
        raw = unicodedata.decomposition(chr(codepoint))
        if not raw or raw.startswith("<"):
            continue
        values = tuple(int(value, 16) for value in raw.split())
        offset = len(decomposition_values)
        decomposition_values.extend(values)
        decompositions.append((codepoint, offset, len(values)))
        if len(values) == 2:
            key = (values[0] << 21) | values[1]
            compositions.append((key, codepoint))
    compositions.sort()

    lines = [
        "//! Generated compact Unicode normalization data; do not edit by hand.",
        f"//! Unicode version: {UNICODE_VERSION}.",
        f"//! Joining data: {ARABIC_SHAPING_URL}",
        f"//! Joining data SHA-256: {ARABIC_SHAPING_SHA256}.",
        f"//! Bidi mirroring data: {BIDI_MIRRORING_URL}",
        f"//! Bidi mirroring data SHA-256: {BIDI_MIRRORING_SHA256}.",
        "",
        "#[allow(dead_code)]",
        "pub(crate) const UNICODE_VERSION: &str = \"14.0.0\";",
        "",
        "const COMBINING_CLASS_RANGES: &[(u32, u32, u8)] = &[",
    ]
    lines.extend(
        f"    ({_hex(start)}, {_hex(end)}, {combining_class}),"
        for start, end, combining_class in class_ranges
    )
    lines.extend(["] ;".replace(" ", ""), "", "const JOINING_TYPES: &[(u32, u32, u8)] = &["])
    lines.extend(
        f"    ({_hex(start)}, {_hex(end)}, {joining_type}),"
        for start, end, joining_type in joining_ranges
    )
    lines.extend(["] ;".replace(" ", ""), "", "const JOINING_TRANSPARENT_RANGES: &[(u32, u32)] = &["])
    lines.extend(
        f"    ({_hex(start)}, {_hex(end)})," for start, end in transparent_ranges
    )
    lines.extend(["] ;".replace(" ", ""), "", "const DECIMAL_NUMBER_RANGES: &[(u32, u32)] = &["])
    lines.extend(
        f"    ({_hex(start)}, {_hex(end)})," for start, end in decimal_ranges
    )
    lines.extend(["] ;".replace(" ", ""), "", "const BIDI_MIRRORS: &[(u32, u32)] = &["])
    lines.extend(
        f"    ({_hex(source)}, {_hex(target)})," for source, target in bidi_mirrors
    )
    lines.extend(["] ;".replace(" ", ""), "", "const DECOMPOSITIONS: &[(u32, u16, u8)] = &["])
    lines.extend(
        f"    ({_hex(codepoint)}, {offset}, {length}),"
        for codepoint, offset, length in decompositions
    )
    lines.extend(["] ;".replace(" ", ""), "", "const DECOMPOSITION_VALUES: &[u32] = &["])
    for index in range(0, len(decomposition_values), 12):
        values = ", ".join(_hex(value) for value in decomposition_values[index : index + 12])
        lines.append(f"    {values},")
    lines.extend(["] ;".replace(" ", ""), "", "const COMPOSITIONS: &[(u64, u32)] = &["])
    lines.extend(
        f"    ({_hex(key)}, {_hex(codepoint)})," for key, codepoint in compositions
    )
    lines.extend(
        [
            "];",
            "",
            "pub(crate) fn combining_class(codepoint: u32) -> u8 {",
            "    let index = COMBINING_CLASS_RANGES.partition_point(|range| range.1 < codepoint);",
            "    COMBINING_CLASS_RANGES",
            "        .get(index)",
            "        .filter(|range| range.0 <= codepoint)",
            "        .map(|range| range.2)",
            "        .unwrap_or(0)",
            "}",
            "",
            "pub(crate) fn joining_type(codepoint: u32) -> u8 {",
            "    let explicit = JOINING_TYPES.partition_point(|range| range.1 < codepoint);",
            "    if let Some(range) = JOINING_TYPES.get(explicit).filter(|range| range.0 <= codepoint) {",
            "        return range.2;",
            "    }",
            "    let transparent = JOINING_TRANSPARENT_RANGES",
            "        .partition_point(|range| range.1 < codepoint);",
            "    if JOINING_TRANSPARENT_RANGES",
            "        .get(transparent)",
            "        .is_some_and(|range| range.0 <= codepoint)",
            "    {",
            "        7",
            "    } else {",
            "        0",
            "    }",
            "}",
            "",
            "pub(crate) fn is_decimal_number(codepoint: u32) -> bool {",
            "    let index = DECIMAL_NUMBER_RANGES.partition_point(|range| range.1 < codepoint);",
            "    DECIMAL_NUMBER_RANGES",
            "        .get(index)",
            "        .is_some_and(|range| range.0 <= codepoint)",
            "}",
            "",
            "pub(crate) fn bidi_mirror(codepoint: u32) -> Option<u32> {",
            "    BIDI_MIRRORS",
            "        .binary_search_by_key(&codepoint, |entry| entry.0)",
            "        .ok()",
            "        .map(|index| BIDI_MIRRORS[index].1)",
            "}",
            "",
            "pub(crate) fn canonical_decomposition(codepoint: u32) -> Option<&'static [u32]> {",
            "    let index = DECOMPOSITIONS",
            "        .binary_search_by_key(&codepoint, |entry| entry.0)",
            "        .ok()?;",
            "    let (_, offset, length) = DECOMPOSITIONS[index];",
            "    let start = usize::from(offset);",
            "    DECOMPOSITION_VALUES.get(start..start + usize::from(length))",
            "}",
            "",
            "pub(crate) fn canonical_composition(first: u32, second: u32) -> Option<u32> {",
            "    let key = (u64::from(first) << 21) | u64::from(second);",
            "    COMPOSITIONS",
            "        .binary_search_by_key(&key, |entry| entry.0)",
            "        .ok()",
            "        .map(|index| COMPOSITIONS[index].1)",
            "}",
            "",
        ]
    )
    OUTPUT.write_text("\n".join(lines), encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main()
