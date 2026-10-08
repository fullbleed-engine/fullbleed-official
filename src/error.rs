use std::fmt;

/// Failure to configure, lay out, or write a Fullbleed document.
///
/// Rendering and builder methods return this error. [`Self::Io`] retains its
/// underlying [`std::io::Error`] as the standard error source.
#[derive(Debug)]
pub enum FullBleedError {
    /// No page template was available for the requested layout.
    MissingPageTemplate,
    /// A content item cannot fit in any available page frame.
    UnplaceableFlowable(String),
    /// A batch or compiled-binding call received no documents or records.
    EmptyDocumentSet,
    /// Documents being combined have incompatible page sizes.
    InconsistentPageSize,
    /// Settings or binding columns do not satisfy the requested operation.
    InvalidConfiguration(String),
    /// An asset could not be parsed or used; the string describes the failure.
    Asset(String),
    /// A cancellable operation observed a cancellation request.
    Cancelled,
    /// Reading or writing a resource failed.
    Io(std::io::Error),
}

impl fmt::Display for FullBleedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FullBleedError::MissingPageTemplate => write!(f, "no page template available"),
            FullBleedError::UnplaceableFlowable(message) => {
                write!(f, "flowable cannot fit on any page: {}", message)
            }
            FullBleedError::EmptyDocumentSet => write!(f, "no documents provided to merge"),
            FullBleedError::InconsistentPageSize => {
                write!(f, "documents have inconsistent page sizes")
            }
            FullBleedError::InvalidConfiguration(message) => {
                write!(f, "invalid configuration: {}", message)
            }
            FullBleedError::Asset(message) => write!(f, "asset error: {}", message),
            FullBleedError::Cancelled => write!(f, "operation cancelled"),
            FullBleedError::Io(err) => write!(f, "io error: {}", err),
        }
    }
}

impl std::error::Error for FullBleedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FullBleedError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for FullBleedError {
    fn from(value: std::io::Error) -> Self {
        FullBleedError::Io(value)
    }
}
