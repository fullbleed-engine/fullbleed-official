//! Text shaping boundary.
//!
//! Keeping the shaping contract in one module lets layout, measurement, and rasterization share
//! identical glyph positions through FullBleed's native OpenType implementation.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TextDirection {
    LeftToRight,
    RightToLeft,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ShapedGlyph {
    pub(crate) glyph_id: u16,
    pub(crate) cluster: u32,
    pub(crate) x_advance: i32,
    pub(crate) y_advance: i32,
    pub(crate) x_offset: i32,
    pub(crate) y_offset: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShapedText {
    pub(crate) units_per_em: u16,
    pub(crate) glyphs: Vec<ShapedGlyph>,
}

pub(crate) fn shape(font_data: &[u8], text: &str) -> Option<ShapedText> {
    crate::native_shape::shape(font_data, text)
}

pub(crate) fn detect_direction(text: &str) -> TextDirection {
    if text.chars().any(|ch| {
        matches!(
            ch as u32,
            0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x1EE00..=0x1EEFF
        )
    }) {
        TextDirection::RightToLeft
    } else {
        TextDirection::LeftToRight
    }
}

#[cfg(test)]
mod tests {
    use super::{ShapedGlyph, TextDirection, detect_direction, shape};
    use fullbleed_audit_contract::sha256::Sha256;

    const INTER: &[u8] = include_bytes!("../python/fullbleed_assets/fonts/Inter-Variable.ttf");
    const NOTO: &[u8] = include_bytes!("../python/fullbleed_assets/fonts/NotoSans-Regular.ttf");
    const MATH: &[u8] = include_bytes!("../python/fullbleed_assets/fonts/NotoSansMath-Regular.ttf");

    fn update_shape_contract(hasher: &mut Sha256, text: &str, shaped: &Option<super::ShapedText>) {
        hasher.update(&(text.len() as u32).to_be_bytes());
        hasher.update(text.as_bytes());
        let Some(shaped) = shaped else {
            hasher.update(&[0]);
            return;
        };
        hasher.update(&[1]);
        hasher.update(&shaped.units_per_em.to_be_bytes());
        hasher.update(&(shaped.glyphs.len() as u32).to_be_bytes());
        for glyph in &shaped.glyphs {
            hasher.update(&glyph.glyph_id.to_be_bytes());
            hasher.update(&glyph.cluster.to_be_bytes());
            hasher.update(&glyph.x_advance.to_be_bytes());
            hasher.update(&glyph.y_advance.to_be_bytes());
            hasher.update(&glyph.x_offset.to_be_bytes());
            hasher.update(&glyph.y_offset.to_be_bytes());
        }
    }

    fn hex_digest(hasher: Sha256) -> String {
        hasher
            .finalize()
            .into_iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn assert_shape(
        label: &str,
        font: &[u8],
        text: &str,
        units_per_em: u16,
        expected: &[(u16, u32, i32, i32, i32, i32)],
    ) {
        let shaped = shape(font, text).expect("shape");
        assert_eq!(shaped.units_per_em, units_per_em, "{label} units/em");
        let actual: Vec<_> = shaped
            .glyphs
            .into_iter()
            .map(
                |ShapedGlyph {
                     glyph_id,
                     cluster,
                     x_advance,
                     y_advance,
                     x_offset,
                     y_offset,
                 }| {
                    (glyph_id, cluster, x_advance, y_advance, x_offset, y_offset)
                },
            )
            .collect();
        assert_eq!(actual, expected, "{label} glyph contract");
    }

    #[test]
    fn bundled_font_shaping_contract_is_frozen() {
        assert_shape(
            "latin",
            INTER,
            "office AVATAR caf\u{e9}",
            2048,
            &[
                (790, 0, 1228, 0, 0, 0),
                (647, 1, 678, 0, 0, 0),
                (647, 2, 758, 0, 0, 0),
                (689, 3, 496, 0, 0, 0),
                (586, 4, 1170, 0, 0, 0),
                (614, 5, 1194, 0, 0, 0),
                (1777, 6, 576, 0, 0, 0),
                (2, 7, 1273, 0, 0, 0),
                (456, 8, 1273, 0, 0, 0),
                (2, 9, 1239, 0, 0, 0),
                (411, 10, 1148, 0, 0, 0),
                (2, 11, 1413, 0, 0, 0),
                (384, 12, 1318, 0, 0, 0),
                (1777, 13, 576, 0, 0, 0),
                (586, 14, 1190, 0, 0, 0),
                (507, 15, 1150, 0, 0, 0),
                (647, 16, 698, 0, 0, 0),
                (618, 17, 1194, 0, 0, 0),
            ],
        );
        assert_shape(
            "greek",
            INTER,
            "\u{391}\u{3b2}\u{3b3}\u{3ac}\u{3c0}\u{3b7}",
            2048,
            &[
                (33, 0, 1413, 0, 0, 0),
                (1136, 2, 1220, 0, 0, 0),
                (1137, 4, 1151, 0, 0, 0),
                (1111, 6, 1360, 0, 0, 0),
                (1199, 8, 1239, 0, 0, 0),
                (1154, 10, 1211, 0, 0, 0),
            ],
        );
        assert_shape(
            "cyrillic",
            INTER,
            "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}",
            2048,
            &[
                (2434, 0, 1522, 0, 0, 0),
                (848, 2, 1254, 0, 0, 0),
                (1262, 4, 1211, 0, 0, 0),
                (1254, 6, 1157, 0, 0, 0),
                (641, 8, 1194, 0, 0, 0),
                (1280, 10, 954, 0, 0, 0),
            ],
        );
        assert_shape(
            "devanagari",
            NOTO,
            "\u{915}\u{94d}\u{937}\u{93f}",
            1000,
            &[(4036, 0, 259, 0, 0, 0), (4154, 0, 717, 0, 0, 0)],
        );
        assert_shape(
            "latin marks",
            INTER,
            "x\u{301}\u{323}",
            2048,
            &[
                (993, 0, 1118, 0, 0, 0),
                (1775, 0, 0, 0, -803, 0),
                (1770, 0, 0, 0, -806, 0),
            ],
        );
        assert_shape(
            "devanagari marks",
            NOTO,
            "\u{915}\u{93f}\u{902}",
            1000,
            &[
                (4046, 0, 259, 0, 0, 0),
                (3942, 0, 768, 0, 0, 0),
                (4042, 0, 0, 0, 0, 0),
            ],
        );
        assert_shape(
            "math",
            MATH,
            "\u{221e}+\u{2211}\u{3c0}",
            1000,
            &[
                (795, 0, 739, 0, 0, 0),
                (1335, 3, 572, 0, 0, 0),
                (1710, 4, 677, 0, 0, 0),
                (1328, 7, 659, 0, 0, 0),
            ],
        );

        assert_eq!(detect_direction("hello"), TextDirection::LeftToRight);
        assert_eq!(
            detect_direction("\u{5e9}\u{5dc}\u{5d5}\u{5dd}"),
            TextDirection::RightToLeft
        );
        assert_eq!(
            detect_direction("\u{645}\u{631}\u{62d}\u{628}\u{627}"),
            TextDirection::RightToLeft
        );
    }

    #[test]
    fn native_single_character_sweep_matches_frozen_contract() {
        let cases = [
            ("inter", INTER, &[(0x20u32, 0x052f), (0x1e00, 0x218f)][..]),
            ("noto", NOTO, &[(0x20u32, 0x10ff), (0x1e00, 0x218f)][..]),
            ("math", MATH, &[(0x20u32, 0x052f), (0x2000, 0x22ff)][..]),
        ];
        let mut contract = Sha256::new();
        for (font_name, font, ranges) in cases {
            contract.update(font_name.as_bytes());
            for &(start, end) in ranges {
                for codepoint in start..=end {
                    let Some(character) = char::from_u32(codepoint) else {
                        continue;
                    };
                    let text = character.to_string();
                    let shaped = shape(font, &text);
                    update_shape_contract(&mut contract, &text, &shaped);
                }
            }
        }
        assert_eq!(
            hex_digest(contract),
            "117efed9fbab694815b51ef2b32302deb1b5372543e898aea6e012a2ff48faf7",
            "single-character shaping contract"
        );
    }

    #[test]
    fn native_external_script_fonts_match_frozen_contracts() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("native-shape-oracles");
        let cases = [
            (
                "arabic",
                "NotoSansArabic.ttf",
                &[
                    "\u{645}\u{631}\u{62d}\u{628}\u{627} \u{628}\u{627}\u{644}\u{639}\u{627}\u{644}\u{645}",
                    "\u{644}\u{627}",
                    "\u{627}\u{644}\u{633}\u{644}\u{627}\u{645} \u{639}\u{644}\u{64a}\u{643}\u{645}",
                    "\u{627}\u{644}\u{639}\u{64e}\u{631}\u{64e}\u{628}\u{650}\u{64a}\u{64e}\u{651}\u{629}\u{64f}",
                    "(\u{645}\u{631}\u{62d}\u{628}\u{627}) [\u{633}\u{644}\u{627}\u{645}]",
                ][..],
                &[(0x0600u32, 0x06ff), (0x0750, 0x077f), (0x08a0, 0x08ff)][..],
            ),
            (
                "hebrew",
                "NotoSansHebrew.ttf",
                &[
                    "\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{5e2}\u{5d5}\u{5dc}\u{5dd}",
                    "\u{5e9}\u{5b8}\u{5c1}\u{5dc}\u{5d5}\u{5b9}\u{5dd}",
                    "(\u{5e9}\u{5dc}\u{5d5}\u{5dd}) [\u{5e2}\u{5d5}\u{5dc}\u{5dd}]",
                ][..],
                &[(0x0590u32, 0x05ff)][..],
            ),
            (
                "thai",
                "NotoSansThai.ttf",
                &[
                    "\u{e20}\u{e32}\u{e29}\u{e32}\u{e44}\u{e17}\u{e22}",
                    "\u{e01}\u{e33}\u{e25}\u{e31}\u{e07}\u{e17}\u{e14}\u{e2a}\u{e2d}\u{e1a}",
                ][..],
                &[(0x0e00u32, 0x0e7f)][..],
            ),
            (
                "nastaliq",
                "NotoNastaliqUrdu.ttf",
                &[
                    "\u{646}\u{633}\u{62a}\u{639}\u{644}\u{6cc}\u{642}",
                    "\u{67e}\u{627}\u{6a9}\u{633}\u{62a}\u{627}\u{646}",
                    "\u{627}\u{631}\u{62f}\u{648}",
                    "\u{639}\u{64e}\u{631}\u{64e}\u{628}\u{650}\u{6cc}",
                ][..],
                &[][..],
            ),
            (
                "korean",
                "NotoSansKR.ttf",
                &[
                    "\u{c548}\u{b155}\u{d558}\u{c138}\u{c694} \u{c138}\u{acc4}",
                    "\u{1100}\u{1161}\u{11a8}",
                    "\u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}",
                ][..],
                &[][..],
            ),
            (
                "japanese",
                "NotoSansJP.ttf",
                &[
                    "\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{7d44}\u{7248}\u{3001}\u{304b}\u{306a}\u{30ab}\u{30ca}\u{3002}",
                    "\u{304b}\u{306a}\u{3068}\u{30ab}\u{30ca}",
                    "\u{300c}\u{65e5}\u{672c}\u{300d} \u{6771}\u{4eac}",
                ][..],
                &[][..],
            ),
            (
                "simplified Chinese",
                "NotoSansSC.ttf",
                &[
                    "\u{4f60}\u{597d}\u{ff0c}\u{4e16}\u{754c}\u{3002}",
                    "\u{4e2d}\u{6587}\u{6392}\u{7248} \u{5168}\u{89d2}\u{5b57}\u{7b26}",
                ][..],
                &[][..],
            ),
            (
                "emoji",
                "NotoColorEmoji.ttf",
                &[
                    "\u{1f600}\u{1f642}\u{1f680}",
                    "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}",
                    "\u{1f469}\u{200d}\u{1f4bb}",
                    "\u{1f44d}\u{1f3fd}",
                    "\u{1f1fa}\u{1f1f8}",
                    "1\u{fe0f}\u{20e3}",
                ][..],
                &[][..],
            ),
        ];
        for (label, filename, strings, ranges) in cases {
            let path = root.join(filename);
            let Ok(font) = std::fs::read(&path) else {
                return;
            };
            let mut contract = Sha256::new();
            contract.update(label.as_bytes());
            for text in strings {
                let shaped = shape(&font, text);
                update_shape_contract(&mut contract, text, &shaped);
            }
            for &(start, end) in ranges {
                for codepoint in start..=end {
                    let Some(character) = char::from_u32(codepoint) else {
                        continue;
                    };
                    let text = character.to_string();
                    let shaped = shape(&font, &text);
                    update_shape_contract(&mut contract, &text, &shaped);
                }
            }
            let expected = match label {
                "arabic" => "9f2727f2097f2a2e4c94e46dc60e284b64c8000c79ecf7b60c0b8d661b77adb5",
                "hebrew" => "cec8f02ba457f6ed66f316a795603ac4a2e72cf8a5efcd727552a28237f241a2",
                "thai" => "4804fb49a4d7a3487991b38ac1d57d3d1f6851a7708911df956afdde73a183b2",
                "nastaliq" => "5ce644865c96fc02ecff27fa0af6c39e7c06cf3feb72ec9aba85a4c5817247bf",
                "korean" => "cb84e32d00ae00941d91ebabd0331de6204434d52e998897d74bf0b34e3cce93",
                "japanese" => "08e7f47189c578365c53f86b3a83acc8b997920c4c72c2a11c8cfbc5c9b79fec",
                "simplified Chinese" => {
                    "b10c7577bad7715a5a51876eec06b683dd38d41b49945f8b6c7bc3582ca6e42b"
                }
                "emoji" => "e52e6c8cb4526e0e4f7138742239ff0db341edb1c24e40110289bc5cffbc3a11",
                _ => unreachable!("known external shaping fixture"),
            };
            assert_eq!(hex_digest(contract), expected, "{label} shaping contract");
        }
    }

    #[test]
    fn native_sequence_sweep_matches_frozen_contract() {
        fn check(label: &str, font: &[u8], text: &str, contract: &mut Sha256) {
            let shaped = shape(font, text);
            contract.update(label.as_bytes());
            update_shape_contract(contract, text, &shaped);
        }

        let mut contract = Sha256::new();
        for first in ' '..='~' {
            for second in ' '..='~' {
                check(
                    "inter ASCII pair",
                    INTER,
                    &format!("{first}{second}"),
                    &mut contract,
                );
            }
        }
        for base in 'A'..='z' {
            for mark in [
                '\u{300}', '\u{301}', '\u{308}', '\u{323}', '\u{327}', '\u{342}',
            ] {
                check(
                    "inter mark sequence",
                    INTER,
                    &format!("{base}{mark}"),
                    &mut contract,
                );
            }
        }
        let greek: Vec<_> = (0x0391..=0x03c9).filter_map(char::from_u32).collect();
        for pair in greek.windows(2) {
            check(
                "inter Greek pair",
                INTER,
                &pair.iter().collect::<String>(),
                &mut contract,
            );
        }
        let cyrillic: Vec<_> = (0x0410..=0x044f).filter_map(char::from_u32).collect();
        for pair in cyrillic.windows(2) {
            check(
                "inter Cyrillic pair",
                INTER,
                &pair.iter().collect::<String>(),
                &mut contract,
            );
        }
        let consonants: Vec<_> = (0x0915..=0x0939).filter_map(char::from_u32).collect();
        for &first in &consonants {
            for &second in &consonants {
                check(
                    "Devanagari conjunct",
                    NOTO,
                    &format!("{first}\u{94d}{second}"),
                    &mut contract,
                );
            }
            for matra in [
                '\u{93e}', '\u{93f}', '\u{940}', '\u{941}', '\u{947}', '\u{94b}',
            ] {
                check(
                    "Devanagari matra",
                    NOTO,
                    &format!("{first}{matra}"),
                    &mut contract,
                );
            }
        }
        assert_eq!(
            hex_digest(contract),
            "6695cb4ff67ea7fd803ea17709a459e76bb84300e8af42c706f4f08c9f5a02f9",
            "multi-character shaping contract"
        );
    }

    #[test]
    fn native_default_feature_sequences_match_frozen_contract() {
        let mut contract = Sha256::new();
        for (label, font, text) in [
            ("inter fraction", INTER, "1\u{2044}2"),
            ("inter long fraction", INTER, "12\u{2044}345"),
            ("noto fraction", NOTO, "3\u{2044}8"),
        ] {
            let shaped = shape(font, text);
            contract.update(label.as_bytes());
            update_shape_contract(&mut contract, text, &shaped);
        }
        assert_eq!(
            hex_digest(contract),
            "92ec4f6c693c70f17ef2180e035a1994464f65efbf7950c62a4ee3212bc03660",
            "default-feature shaping contract"
        );
    }
}
