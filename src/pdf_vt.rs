//! Source-level PDF/VT job structure. Each document consumes one input document
//! (or one compiled binding row/copy); page ranges come from final pagination.

use std::collections::{BTreeMap, BTreeSet};
use std::io;

#[derive(Debug, Clone)]
pub enum DpmValue {
    String(String),
    Integer(i64),
    Real(f64),
    Boolean(bool),
    Array(Vec<DpmValue>),
    Dictionary(BTreeMap<String, DpmValue>),
}

// Bitwise real equality keeps inspection reports reflexive (and Eq-compatible),
// including values constructed directly by callers before validation.
impl PartialEq for DpmValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Integer(a), Self::Integer(b)) => a == b,
            (Self::Real(a), Self::Real(b)) => a.to_bits() == b.to_bits(),
            (Self::Boolean(a), Self::Boolean(b)) => a == b,
            (Self::Array(a), Self::Array(b)) => a == b,
            (Self::Dictionary(a), Self::Dictionary(b)) => a == b,
            _ => false,
        }
    }
}
impl Eq for DpmValue {}

pub type DpmMetadata = BTreeMap<String, DpmValue>;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PdfVtDocument {
    pub id: String,
    pub metadata: DpmMetadata,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PdfVtRecord {
    pub id: String,
    pub metadata: DpmMetadata,
    pub documents: Vec<PdfVtDocument>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PdfVtJob {
    pub id: String,
    pub metadata: DpmMetadata,
    /// Empty means one automatically named record/document per input.
    pub records: Vec<PdfVtRecord>,
}

pub(crate) fn invalid(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("PDF_VT_JOB_INVALID: {message}"),
    )
}

impl PdfVtJob {
    pub(crate) fn validate(&self) -> io::Result<()> {
        check_id(&self.id)?;
        validate_metadata(&self.metadata)?;
        let mut records = BTreeSet::new();
        for record in &self.records {
            check_id(&record.id)?;
            if !records.insert(&record.id) {
                return Err(invalid("record IDs must be unique"));
            }
            validate_metadata(&record.metadata)?;
            if record.documents.is_empty() {
                return Err(invalid("each record requires at least one document"));
            }
            let mut documents = BTreeSet::new();
            for document in &record.documents {
                check_id(&document.id)?;
                if !documents.insert(&document.id) {
                    return Err(invalid("document IDs must be unique within a record"));
                }
                validate_metadata(&document.metadata)?;
            }
        }
        Ok(())
    }
}

fn check_id(id: &str) -> io::Result<()> {
    if id.trim().is_empty() || id.len() > 4096 {
        return Err(invalid(
            "IDs must be nonempty strings of at most 4096 bytes",
        ));
    }
    Ok(())
}

fn validate_metadata(metadata: &DpmMetadata) -> io::Result<()> {
    fn dictionary(items: &DpmMetadata, depth: usize, remaining: &mut usize) -> io::Result<()> {
        for (key, item) in items {
            if key.is_empty() || key.len() > 127 {
                return Err(invalid("DPM keys must contain 1 through 127 UTF-8 bytes"));
            }
            value(item, depth + 1, remaining)?;
        }
        Ok(())
    }
    fn value(v: &DpmValue, depth: usize, remaining: &mut usize) -> io::Result<()> {
        if depth > 16 || *remaining == 0 {
            return Err(invalid("DPM exceeds depth 16 or 10000 values per node"));
        }
        *remaining -= 1;
        match v {
            DpmValue::Real(v)
                if !v.is_finite()
                    || !(*v as f32).is_finite()
                    || (*v != 0.0 && *v as f32 == 0.0) =>
            {
                Err(invalid(
                    "DPM real values must be finite and within the PDF reader's single-precision range",
                ))
            }
            DpmValue::String(v) if v.len() > 65536 => {
                Err(invalid("DPM strings must be at most 65536 bytes"))
            }
            DpmValue::Array(items) => {
                for item in items {
                    value(item, depth + 1, remaining)?;
                }
                Ok(())
            }
            DpmValue::Dictionary(items) => dictionary(items, depth, remaining),
            _ => Ok(()),
        }
    }
    dictionary(metadata, 0, &mut 9999)
}

fn value_pdf(value: &DpmValue) -> String {
    match value {
        DpmValue::String(value) => crate::pdf::pdf_text_string(value),
        DpmValue::Integer(value) => value.to_string(),
        DpmValue::Real(value) => {
            let text = value.to_string();
            if text.contains('.') {
                text
            } else {
                format!("{text}.0")
            }
        }
        DpmValue::Boolean(value) => value.to_string(),
        DpmValue::Array(items) => format!(
            "[{}]",
            items.iter().map(value_pdf).collect::<Vec<_>>().join(" ")
        ),
        DpmValue::Dictionary(items) => dictionary_pdf(items),
    }
}

fn dictionary_pdf(metadata: &DpmMetadata) -> String {
    format!(
        "<< {} >>",
        metadata
            .iter()
            .map(|(key, value)| format!(
                "/{} {}",
                crate::pdf::escape_pdf_name(key),
                value_pdf(value)
            ))
            .collect::<Vec<_>>()
            .join(" ")
    )
}

/// Private Fullbleed DPM vocabulary: ID and user-supplied production metadata.
/// It does not assert CIP4, JDF or ISO 21812 semantics.
pub(crate) fn dpm(id: &str, metadata: &DpmMetadata) -> String {
    format!(
        " /DPM << /Fullbleed << /ID {} /Metadata {} >> >>",
        crate::pdf::pdf_text_string(id),
        dictionary_pdf(metadata)
    )
}

pub(crate) struct RecordState {
    pub object_id: usize,
    pub id: String,
    pub metadata: DpmMetadata,
    pub documents: Vec<usize>,
}

pub(crate) struct DocumentState {
    pub object_id: usize,
    pub parent_id: usize,
    pub start: usize,
    pub end: usize,
    pub source: PdfVtDocument,
}
