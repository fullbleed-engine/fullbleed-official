//! Engine-owned profile discovery and parsing. These are emission requirements,
//! not an assertion of conformance by an authored document or resulting PDF.

use crate::{PdfProfile, PdfVersion};
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfProfileDescriptor {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub emits_tagged_structure: bool,
    pub requires_output_intent: bool,
    pub requires_embedded_fonts: bool,
    pub uses_pdfx_page_boxes: bool,
    pub default_pdf_version: &'static str,
    pub fixed_bindings_supported: bool,
}

impl PdfProfile {
    pub const ALL: [Self; 19] = [
        Self::None,
        Self::PdfA1a,
        Self::PdfA1b,
        Self::PdfA2a,
        Self::PdfA2b,
        Self::PdfA2u,
        Self::PdfA3a,
        Self::PdfA3b,
        Self::PdfA3u,
        Self::PdfA4,
        Self::PdfA4e,
        Self::PdfA4f,
        Self::PdfX4,
        Self::PdfUa1,
        Self::PdfUa2,
        Self::PdfVt1,
        Self::Wtpdf1r,
        Self::Wtpdf1a,
        Self::Tagged,
    ];

    /// Accepted nonempty names. Blank input retains the legacy `none` default.
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::None => &["none"],
            Self::PdfA1a => &["pdfa1a", "pdfa-1a", "pdfa_1a", "pdf/a-1a", "pdf/a1a"],
            Self::PdfA1b => &["pdfa1b", "pdfa-1b", "pdfa_1b", "pdf/a-1b", "pdf/a1b"],
            Self::PdfA2a => &["pdfa2a", "pdfa-2a", "pdfa_2a", "pdf/a-2a", "pdf/a2a"],
            Self::PdfA2b => &[
                "pdfa2b", "pdfa-2b", "pdfa_2b", "pdf/a-2b", "pdf/a2b", "a", "pdfa", "pdf/a",
            ],
            Self::PdfA2u => &["pdfa2u", "pdfa-2u", "pdfa_2u", "pdf/a-2u", "pdf/a2u"],
            Self::PdfA3a => &["pdfa3a", "pdfa-3a", "pdfa_3a", "pdf/a-3a", "pdf/a3a"],
            Self::PdfA3b => &["pdfa3b", "pdfa-3b", "pdfa_3b", "pdf/a-3b", "pdf/a3b"],
            Self::PdfA3u => &["pdfa3u", "pdfa-3u", "pdfa_3u", "pdf/a-3u", "pdf/a3u"],
            Self::PdfA4 => &["pdfa4", "pdfa-4", "pdfa_4", "pdf/a-4", "pdf/a4"],
            Self::PdfA4e => &["pdfa4e", "pdfa-4e", "pdfa_4e", "pdf/a-4e", "pdf/a4e"],
            Self::PdfA4f => &["pdfa4f", "pdfa-4f", "pdfa_4f", "pdf/a-4f", "pdf/a4f"],
            Self::PdfX4 => &["pdfx4", "pdfx-4", "pdfx_4", "pdf/x-4", "pdf/x4"],
            Self::PdfUa1 => &["pdfua1", "pdfua-1", "pdf/ua-1", "ua", "pdfua", "pdf/ua"],
            Self::PdfUa2 => &["pdfua2", "pdfua-2", "pdf/ua-2"],
            Self::PdfVt1 => &["pdfvt1", "pdfvt-1", "pdf/vt-1", "vt", "pdfvt", "pdf/vt"],
            Self::Wtpdf1r => &["wtpdf1r", "wtpdf-1r", "wtpdf_1r", "wt1r", "wt-1r"],
            Self::Wtpdf1a => &["wtpdf1a", "wtpdf-1a", "wtpdf_1a", "wt1a", "wt-1a"],
            Self::Tagged => &["tagged"],
        }
    }

    pub fn descriptor(self) -> PdfProfileDescriptor {
        PdfProfileDescriptor {
            name: self.as_str(),
            aliases: self.aliases(),
            emits_tagged_structure: self.emits_tagged_structure(),
            requires_output_intent: self.requires_output_intent(),
            requires_embedded_fonts: self.requires_embedded_fonts(),
            uses_pdfx_page_boxes: self.uses_pdfx_page_boxes(),
            default_pdf_version: match self.effective_pdf_version(PdfVersion::Pdf17) {
                PdfVersion::Pdf17 => "1.7",
                PdfVersion::Pdf20 => "2.0",
            },
            fixed_bindings_supported: !self.emits_tagged_structure(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsePdfProfileError(pub String);

impl fmt::Display for ParsePdfProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Invalid pdf_profile: {:?}. Expected one of: {}",
            self.0,
            PdfProfile::ALL.map(PdfProfile::as_str).join(", ")
        )
    }
}

impl std::error::Error for ParsePdfProfileError {}

impl FromStr for PdfProfile {
    type Err = ParsePdfProfileError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let name = value.trim().to_ascii_lowercase();
        if name.is_empty() {
            return Ok(Self::None);
        }
        Self::ALL
            .into_iter()
            .find(|profile| profile.aliases().contains(&name.as_str()))
            .ok_or_else(|| ParsePdfProfileError(value.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn profile_catalog_aliases_and_emission_rules_agree() {
        let mut aliases = BTreeSet::new();
        assert_eq!(PdfProfile::ALL.len(), 19);
        assert_eq!(
            PdfProfile::ALL
                .iter()
                .filter(|p| p.requires_output_intent())
                .count(),
            13
        );
        for profile in PdfProfile::ALL {
            let descriptor = profile.descriptor();
            assert_eq!(descriptor.name.parse::<PdfProfile>().unwrap(), profile);
            for alias in descriptor.aliases {
                assert!(aliases.insert(*alias), "duplicate alias {alias}");
                assert_eq!(
                    format!(" {} ", alias.to_uppercase())
                        .parse::<PdfProfile>()
                        .unwrap(),
                    profile
                );
            }
            assert_eq!(
                descriptor.fixed_bindings_supported,
                !profile.emits_tagged_structure()
            );
        }
        assert_eq!(" \t".parse::<PdfProfile>().unwrap(), PdfProfile::None);
        assert_eq!(
            "pdf/a-2b".parse::<PdfProfile>().unwrap(),
            PdfProfile::PdfA2b
        );
        assert_eq!("pdf/a2b".parse::<PdfProfile>().unwrap(), PdfProfile::PdfA2b);
        assert!("PDF/UA-3".parse::<PdfProfile>().is_err());
    }
}
