use std::fmt;

#[derive(Debug)]
pub enum FullBleedError {
    MissingPageTemplate,
    UnplaceableFlowable(String),
    EmptyDocumentSet,
    InconsistentPageSize,
    InvalidConfiguration(String),
    Asset(String),
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
