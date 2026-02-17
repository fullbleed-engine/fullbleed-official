use lopdf::Document as LoDocument;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfInspectErrorCode {
    PdfParseFailed,
    PdfEncryptedUnsupported,
    PdfEmptyOrNoPages,
    PdfIoError,
}

impl PdfInspectErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            PdfInspectErrorCode::PdfParseFailed => "PDF_PARSE_FAILED",
            PdfInspectErrorCode::PdfEncryptedUnsupported => "PDF_ENCRYPTED_UNSUPPORTED",
            PdfInspectErrorCode::PdfEmptyOrNoPages => "PDF_EMPTY_OR_NO_PAGES",
            PdfInspectErrorCode::PdfIoError => "PDF_IO_ERROR",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfInspectError {
    pub code: PdfInspectErrorCode,
    pub message: String,
}

impl std::fmt::Display for PdfInspectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for PdfInspectError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfInspectWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfInspectReport {
    pub pdf_version: String,
    pub page_count: usize,
    pub encrypted: bool,
    pub file_size_bytes: usize,
    pub warnings: Vec<PdfInspectWarning>,
}

pub fn inspect_pdf_bytes(bytes: &[u8]) -> Result<PdfInspectReport, PdfInspectError> {
    let pdf = LoDocument::load_mem(bytes).map_err(|err| PdfInspectError {
        code: PdfInspectErrorCode::PdfParseFailed,
        message: err.to_string(),
    })?;

    Ok(PdfInspectReport {
        pdf_version: pdf.version.clone(),
        page_count: pdf.get_pages().len(),
        encrypted: pdf.is_encrypted(),
        file_size_bytes: bytes.len(),
        warnings: Vec::new(),
    })
}

pub fn inspect_pdf_path(path: &Path) -> Result<PdfInspectReport, PdfInspectError> {
    let data = std::fs::read(path).map_err(|err| PdfInspectError {
        code: PdfInspectErrorCode::PdfIoError,
        message: err.to_string(),
    })?;
    inspect_pdf_bytes(&data)
}

pub fn composition_compatibility_issues(report: &PdfInspectReport) -> Vec<PdfInspectErrorCode> {
    let mut issues = Vec::new();
    if report.encrypted {
        issues.push(PdfInspectErrorCode::PdfEncryptedUnsupported);
    }
    if report.page_count == 0 {
        issues.push(PdfInspectErrorCode::PdfEmptyOrNoPages);
    }
    issues
}

pub fn require_pdf_composition_compatibility(
    report: &PdfInspectReport,
) -> Result<(), PdfInspectError> {
    for issue in composition_compatibility_issues(report) {
        match issue {
            PdfInspectErrorCode::PdfEncryptedUnsupported => {
                return Err(PdfInspectError {
                    code: PdfInspectErrorCode::PdfEncryptedUnsupported,
                    message: "encrypted pdf assets are not supported".to_string(),
                });
            }
            PdfInspectErrorCode::PdfEmptyOrNoPages => {
                return Err(PdfInspectError {
                    code: PdfInspectErrorCode::PdfEmptyOrNoPages,
                    message: "pdf has no pages".to_string(),
                });
            }
            PdfInspectErrorCode::PdfParseFailed | PdfInspectErrorCode::PdfIoError => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Object as LoObject, Stream as LoStream, dictionary};
    use std::io::Write;

    fn make_single_page_pdf_bytes(text: &str) -> Vec<u8> {
        let mut doc = LoDocument::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content = format!("BT /F1 18 Tf 72 720 Td ({}) Tj ET", text).into_bytes();
        let content_id = doc.add_object(LoStream::new(dictionary! {}, content));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        };
        doc.objects.insert(pages_id, LoObject::Dictionary(pages));
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        doc.compress();

        let mut out = Vec::new();
        doc.save_to(&mut out).expect("save");
        out
    }

    #[test]
    fn inspect_pdf_bytes_reads_version_and_page_count() {
        let bytes = make_single_page_pdf_bytes("HELLO");
        let report = inspect_pdf_bytes(&bytes).expect("inspect");
        assert_eq!(report.page_count, 1);
        assert!(!report.encrypted);
        assert_eq!(report.file_size_bytes, bytes.len());
        assert!(!report.pdf_version.is_empty());
    }

    #[test]
    fn inspect_pdf_bytes_rejects_malformed_data() {
        let err = inspect_pdf_bytes(b"not a pdf").expect_err("invalid");
        assert_eq!(err.code, PdfInspectErrorCode::PdfParseFailed);
    }

    #[test]
    fn inspect_pdf_path_reports_io_error_for_missing_file() {
        let missing = std::env::temp_dir().join(format!(
            "fullbleed_pdfinspect_missing_{}_{}.pdf",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let err = inspect_pdf_path(&missing).expect_err("missing");
        assert_eq!(err.code, PdfInspectErrorCode::PdfIoError);
    }

    #[test]
    fn composition_compatibility_rejects_encrypted() {
        let report = PdfInspectReport {
            pdf_version: "1.7".to_string(),
            page_count: 1,
            encrypted: true,
            file_size_bytes: 0,
            warnings: Vec::new(),
        };
        let issues = composition_compatibility_issues(&report);
        assert!(issues.contains(&PdfInspectErrorCode::PdfEncryptedUnsupported));

        let err = require_pdf_composition_compatibility(&report).expect_err("must fail");
        assert_eq!(err.code, PdfInspectErrorCode::PdfEncryptedUnsupported);
    }

    #[test]
    fn composition_compatibility_rejects_empty_page_count() {
        let report = PdfInspectReport {
            pdf_version: "1.7".to_string(),
            page_count: 0,
            encrypted: false,
            file_size_bytes: 0,
            warnings: Vec::new(),
        };
        let issues = composition_compatibility_issues(&report);
        assert_eq!(issues, vec![PdfInspectErrorCode::PdfEmptyOrNoPages]);
        let err = require_pdf_composition_compatibility(&report).expect_err("must fail");
        assert_eq!(err.code, PdfInspectErrorCode::PdfEmptyOrNoPages);
    }

    #[test]
    fn inspect_pdf_path_matches_bytes_report() {
        let bytes = make_single_page_pdf_bytes("PATH");
        let temp_dir = std::env::temp_dir().join(format!(
            "fullbleed_pdfinspect_path_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_dir).expect("mkdir");
        let path = temp_dir.join("one.pdf");
        let mut f = std::fs::File::create(&path).expect("create");
        f.write_all(&bytes).expect("write");

        let from_path = inspect_pdf_path(&path).expect("inspect path");
        let from_bytes = inspect_pdf_bytes(&bytes).expect("inspect bytes");
        assert_eq!(from_path.page_count, from_bytes.page_count);
        assert_eq!(from_path.encrypted, from_bytes.encrypted);
        assert_eq!(from_path.pdf_version, from_bytes.pdf_version);
    }
}
