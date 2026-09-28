//! Document metadata facts from the same recovered HTML tree used by layout.
use crate::html_dom::{NodeRef, parse_html};

/// Decoded authored metadata. `None` means absent; `Some("")` is an explicitly
/// empty value and must not be confused with permission to invent a fallback.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthoringDocumentMetadata {
    pub title: Option<String>,
    pub language: Option<String>,
}

/// Inspect document metadata without rendering, fetching assets or mutating HTML.
/// Recovery and entity decoding are engine semantics, not browser DOM behavior.
pub fn inspect_document_metadata(html: &str) -> AuthoringDocumentMetadata {
    let document = parse_html(html);
    metadata_from_document(&document)
}

pub(crate) fn metadata_from_document(document: &NodeRef) -> AuthoringDocumentMetadata {
    let language = document.select("html").ok().and_then(|mut nodes| {
        nodes.next().and_then(|node| {
            node.attributes
                .borrow()
                .get("lang")
                .map(|value| value.trim().to_owned())
        })
    });
    let title = document
        .select("html > head > title")
        .ok()
        .and_then(|mut nodes| {
            nodes
                .next()
                .map(|node| node.text_contents().trim().to_owned())
        });
    AuthoringDocumentMetadata { title, language }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figure_and_body_titles_are_not_document_titles() {
        let html = "<html lang='en'><head></head><body><svg><title>Figure name</title></svg><title>Body title</title></body></html>";
        assert_eq!(inspect_document_metadata(html).title, None);
        let html = html.replace("<head></head>", "<head><title>Document</title></head>");
        assert_eq!(
            inspect_document_metadata(&html).title.as_deref(),
            Some("Document")
        );
    }

    #[test]
    fn metadata_uses_native_decoding_and_preserves_absent_vs_empty() {
        assert_eq!(
            inspect_document_metadata("<p>Body</p>"),
            AuthoringDocumentMetadata::default()
        );
        assert_eq!(
            inspect_document_metadata("<html lang=''><head><title> </title></head></html>"),
            AuthoringDocumentMetadata {
                title: Some(String::new()),
                language: Some(String::new())
            }
        );
        assert_eq!(
            inspect_document_metadata(
                "<!doctype html><HTML LANG='fr&#45;CA'><head><title>Crédit &amp; résumé &#x1f4c4;</title></head><body><p>Bonjour</p></body></HTML>"
            ),
            AuthoringDocumentMetadata {
                title: Some("Crédit & résumé 📄".into()),
                language: Some("fr-CA".into())
            }
        );
    }

    #[test]
    fn metadata_ignores_comment_and_script_title_lookalikes() {
        let html = "<html lang='en'><head><!-- <title>False</title> --><script>const x = '<title>False</title>';</script><title>Real &copy; title</title></head><body><p>Body</p></body></html>";
        assert_eq!(
            inspect_document_metadata(html).title.as_deref(),
            Some("Real © title")
        );
    }
}
