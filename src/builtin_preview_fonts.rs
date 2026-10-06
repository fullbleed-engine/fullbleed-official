//! Fixed, open-licensed outline substitutes for unembedded PDF Standard 14 fonts.
//!
//! These programs are used by the native rasterizer only. They do not replace
//! registered/embedded fonts or change the font resources written into a PDF.

#[derive(Clone, Copy)]
pub(crate) struct PreviewFont {
    pub(crate) data: &'static [u8],
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) file_name: &'static str,
}

pub(crate) fn resolve(name: &str) -> Option<PreviewFont> {
    let name = name.trim().trim_matches('"').trim_matches('\'');
    let name = match name.split_once('+') {
        Some((prefix, rest))
            if prefix.len() == 6 && prefix.bytes().all(|b| b.is_ascii_uppercase()) =>
        {
            rest
        }
        _ => name,
    };
    macro_rules! face {
        ($file:literal) => {
            Some(PreviewFont {
                data: include_bytes!(concat!("preview_fonts/", $file)),
                file_name: $file,
            })
        };
    }
    match name.to_ascii_lowercase().as_str() {
        "helvetica" => face!("FullbleedPreviewSans-Regular.ttf"),
        "helvetica-bold" => face!("FullbleedPreviewSans-Bold.ttf"),
        "helvetica-oblique" => face!("FullbleedPreviewSans-Italic.ttf"),
        "helvetica-boldoblique" => face!("FullbleedPreviewSans-BoldItalic.ttf"),
        "times-roman" => face!("FullbleedPreviewSerif-Regular.ttf"),
        "times-bold" => face!("FullbleedPreviewSerif-Bold.ttf"),
        "times-italic" => face!("FullbleedPreviewSerif-Italic.ttf"),
        "times-bolditalic" => face!("FullbleedPreviewSerif-BoldItalic.ttf"),
        "courier" => face!("FullbleedPreviewMono-Regular.ttf"),
        "courier-bold" => face!("FullbleedPreviewMono-Bold.ttf"),
        "courier-oblique" => face!("FullbleedPreviewMono-Italic.ttf"),
        "courier-boldoblique" => face!("FullbleedPreviewMono-BoldItalic.ttf"),
        "symbol" => face!("FullbleedPreviewSymbol-Regular.ttf"),
        "zapfdingbats" => face!("FullbleedPreviewDingbats-Regular.ttf"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_faces_have_usable_programs_and_correct_styles() {
        for (pdf_name, file_name) in [
            ("Helvetica", "FullbleedPreviewSans-Regular.ttf"),
            ("Helvetica-Bold", "FullbleedPreviewSans-Bold.ttf"),
            ("Helvetica-Oblique", "FullbleedPreviewSans-Italic.ttf"),
            (
                "Helvetica-BoldOblique",
                "FullbleedPreviewSans-BoldItalic.ttf",
            ),
            ("Times-Roman", "FullbleedPreviewSerif-Regular.ttf"),
            ("Times-Bold", "FullbleedPreviewSerif-Bold.ttf"),
            ("Times-Italic", "FullbleedPreviewSerif-Italic.ttf"),
            ("Times-BoldItalic", "FullbleedPreviewSerif-BoldItalic.ttf"),
            ("Courier", "FullbleedPreviewMono-Regular.ttf"),
            ("Courier-Bold", "FullbleedPreviewMono-Bold.ttf"),
            ("Courier-Oblique", "FullbleedPreviewMono-Italic.ttf"),
            ("Courier-BoldOblique", "FullbleedPreviewMono-BoldItalic.ttf"),
            ("Symbol", "FullbleedPreviewSymbol-Regular.ttf"),
            ("ZapfDingbats", "FullbleedPreviewDingbats-Regular.ttf"),
        ] {
            let font = resolve(pdf_name).unwrap();
            assert_eq!(font.file_name, file_name);
            assert!(crate::sfnt::Face::parse(font.data, 0).is_ok(), "{pdf_name}");
        }
    }

    #[test]
    fn resolution_does_not_match_arbitrary_names_or_family_prefixes() {
        assert_eq!(
            resolve("ABCDEF+Helvetica-Bold").unwrap().file_name,
            "FullbleedPreviewSans-Bold.ttf"
        );
        assert_eq!(
            resolve("'courier-oblique'").unwrap().file_name,
            "FullbleedPreviewMono-Italic.ttf"
        );
        for name in [
            "HelveticaWorld",
            "Helvetica Neue",
            "Arial",
            "Inter",
            "UnknownFont",
            "sans-serif",
            "BAD+Helvetica",
        ] {
            assert!(resolve(name).is_none(), "{name}");
        }
    }

    #[test]
    fn every_builtin_encoded_standard_glyph_has_an_outline_mapping() {
        for name in [
            "Helvetica",
            "Helvetica-Bold",
            "Helvetica-Oblique",
            "Helvetica-BoldOblique",
            "Times-Roman",
            "Times-Bold",
            "Times-Italic",
            "Times-BoldItalic",
            "Courier",
            "Courier-Bold",
            "Courier-Oblique",
            "Courier-BoldOblique",
            "Symbol",
            "ZapfDingbats",
        ] {
            let font = resolve(name).unwrap();
            let face = crate::sfnt::Face::parse(font.data, 0).unwrap();
            let metrics = crate::base14_metrics::font(name).unwrap();
            for code in 0..=255u8 {
                if let Some(glyph) = metrics.builtin_glyph(code) {
                    for ch in glyph.text.chars() {
                        assert!(
                            face.glyph_index(ch as u32).is_some(),
                            "{name}: code {code}, {ch:?}"
                        );
                    }
                }
            }
        }
    }
}
