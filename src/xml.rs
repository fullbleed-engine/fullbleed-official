//! Owned, dependency-free XML DOM for SVG compilation.

use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub(crate) struct Document {
    elements: Vec<Element>,
    root: usize,
}

#[derive(Debug)]
struct Element {
    name: String,
    attributes: Vec<(String, String)>,
    parent: Option<usize>,
    children: Vec<usize>,
    content: Vec<ElementContent>,
    first_text: Option<String>,
}

#[derive(Debug)]
enum ElementContent {
    Element(usize),
    Text(String),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Node<'a> {
    document: &'a Document,
    index: usize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TagName<'a> {
    name: &'a str,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ContentNode<'a> {
    Element(Node<'a>),
    Text(&'a str),
}

impl<'a> TagName<'a> {
    pub(crate) fn name(self) -> &'a str {
        self.name
    }
}

impl Document {
    pub(crate) fn parse(input: &str) -> Result<Self, ParseError> {
        Parser::new(input).parse()
    }

    pub(crate) fn descendants(&self) -> Descendants<'_> {
        Descendants {
            document: self,
            next: 0,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn root(&self) -> Node<'_> {
        Node {
            document: self,
            index: self.root,
        }
    }
}

impl<'a> Node<'a> {
    pub(crate) const fn is_element(&self) -> bool {
        true
    }

    pub(crate) fn tag_name(&self) -> TagName<'a> {
        TagName {
            name: local_name(&self.element().name),
        }
    }

    pub(crate) fn attribute(&self, name: &str) -> Option<&'a str> {
        let element = self.element();
        if let Some((_, value)) = element
            .attributes
            .iter()
            .find(|(candidate, _)| candidate == name)
        {
            return Some(value);
        }
        if name.contains(':') {
            return None;
        }
        element
            .attributes
            .iter()
            .find(|(candidate, _)| local_name(candidate) == name)
            .map(|(_, value)| value.as_str())
    }

    pub(crate) fn children(&self) -> Children<'a> {
        Children {
            document: self.document,
            indices: self.element().children.iter(),
        }
    }

    pub(crate) fn content(&self) -> Content<'a> {
        Content {
            document: self.document,
            entries: self.element().content.iter(),
        }
    }

    pub(crate) fn parent(&self) -> Option<Node<'a>> {
        self.element().parent.map(|index| Node {
            document: self.document,
            index,
        })
    }

    pub(crate) fn text(&self) -> Option<&'a str> {
        self.element().first_text.as_deref()
    }

    fn element(&self) -> &'a Element {
        &self.document.elements[self.index]
    }
}

pub(crate) struct Descendants<'a> {
    document: &'a Document,
    next: usize,
}

impl<'a> Iterator for Descendants<'a> {
    type Item = Node<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.document.elements.len() {
            return None;
        }
        let index = self.next;
        self.next += 1;
        Some(Node {
            document: self.document,
            index,
        })
    }
}

pub(crate) struct Children<'a> {
    document: &'a Document,
    indices: std::slice::Iter<'a, usize>,
}

impl<'a> Iterator for Children<'a> {
    type Item = Node<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.indices.next().copied().map(|index| Node {
            document: self.document,
            index,
        })
    }
}

pub(crate) struct Content<'a> {
    document: &'a Document,
    entries: std::slice::Iter<'a, ElementContent>,
}

impl<'a> Iterator for Content<'a> {
    type Item = ContentNode<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.next().map(|entry| match entry {
            ElementContent::Element(index) => ContentNode::Element(Node {
                document: self.document,
                index: *index,
            }),
            ElementContent::Text(value) => ContentNode::Text(value.as_str()),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ParseError {
    offset: usize,
    reason: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid XML at byte {}: {}",
            self.offset, self.reason
        )
    }
}

impl Error for ParseError {}

struct Parser<'a> {
    input: &'a str,
    offset: usize,
    elements: Vec<Element>,
    stack: Vec<usize>,
    root: Option<usize>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            offset: 0,
            elements: Vec::new(),
            stack: Vec::new(),
            root: None,
        }
    }

    fn parse(mut self) -> Result<Document, ParseError> {
        if self.input.starts_with('\u{feff}') {
            self.offset = '\u{feff}'.len_utf8();
        }
        while self.offset < self.input.len() {
            if !self.remaining().starts_with('<') {
                self.parse_text()?;
            } else if self.remaining().starts_with("<!--") {
                self.parse_comment()?;
            } else if self.remaining().starts_with("<![CDATA[") {
                self.parse_cdata()?;
            } else if self.remaining().starts_with("<?") {
                self.parse_processing_instruction()?;
            } else if self.remaining().starts_with("<!DOCTYPE") {
                self.parse_doctype()?;
            } else if self.remaining().starts_with("</") {
                self.parse_end_tag()?;
            } else if self.remaining().starts_with("<!") {
                return Err(self.error("unsupported declaration"));
            } else {
                self.parse_start_tag()?;
            }
        }

        if !self.stack.is_empty() {
            return Err(self.error("unclosed element"));
        }
        let root = self
            .root
            .ok_or_else(|| self.error("document has no root element"))?;
        Ok(Document {
            elements: self.elements,
            root,
        })
    }

    fn parse_start_tag(&mut self) -> Result<(), ParseError> {
        self.expect("<", "expected start tag")?;
        let name = self.parse_name()?.to_string();
        let parent = self.stack.last().copied();
        if parent.is_none() && self.root.is_some() {
            return Err(self.error("document has more than one root element"));
        }

        let mut attributes = Vec::new();
        let self_closing;
        loop {
            let had_whitespace = self.skip_whitespace();
            if self.take("/>") {
                self_closing = true;
                break;
            }
            if self.take(">") {
                self_closing = false;
                break;
            }
            if !had_whitespace {
                return Err(self.error("attributes must be separated by whitespace"));
            }
            let attribute_name = self.parse_name()?.to_string();
            if attributes
                .iter()
                .any(|(existing, _): &(String, String)| existing == &attribute_name)
            {
                return Err(self.error("duplicate attribute"));
            }
            self.skip_whitespace();
            self.expect("=", "expected '=' after attribute name")?;
            self.skip_whitespace();
            let value = self.parse_attribute_value()?;
            attributes.push((attribute_name, value));
        }

        let index = self.elements.len();
        self.elements.push(Element {
            name,
            attributes,
            parent,
            children: Vec::new(),
            content: Vec::new(),
            first_text: None,
        });
        if let Some(parent) = parent {
            self.elements[parent].children.push(index);
            self.elements[parent]
                .content
                .push(ElementContent::Element(index));
        } else {
            self.root = Some(index);
        }
        if !self_closing {
            self.stack.push(index);
        }
        Ok(())
    }

    fn parse_end_tag(&mut self) -> Result<(), ParseError> {
        self.offset += 2;
        let name = self.parse_name()?.to_string();
        self.skip_whitespace();
        self.expect(">", "expected '>' after end tag")?;
        let index = self
            .stack
            .pop()
            .ok_or_else(|| self.error("end tag has no matching start tag"))?;
        if self.elements[index].name != name {
            return Err(self.error("end tag does not match start tag"));
        }
        Ok(())
    }

    fn parse_text(&mut self) -> Result<(), ParseError> {
        let end = self.remaining().find('<').unwrap_or(self.remaining().len());
        let start = self.offset;
        self.offset += end;
        let raw = &self.input[start..self.offset];
        if raw.contains("]]>") {
            return Err(self.error("']]>' is only valid as a CDATA terminator"));
        }
        if self.stack.is_empty() {
            if !raw.trim().is_empty() {
                return Err(self.error("text is not allowed outside the root element"));
            }
            return Ok(());
        }
        let decoded = decode_entities(raw, start)?;
        self.append_text(decoded);
        Ok(())
    }

    fn parse_comment(&mut self) -> Result<(), ParseError> {
        self.offset += 4;
        let Some(length) = self.remaining().find("-->") else {
            return Err(self.error("unterminated comment"));
        };
        if self.remaining()[..length].contains("--") {
            return Err(self.error("'--' is not valid inside an XML comment"));
        }
        self.offset += length + 3;
        Ok(())
    }

    fn parse_cdata(&mut self) -> Result<(), ParseError> {
        if self.stack.is_empty() {
            return Err(self.error("CDATA is not allowed outside the root element"));
        }
        self.offset += 9;
        let Some(length) = self.remaining().find("]]>") else {
            return Err(self.error("unterminated CDATA section"));
        };
        let value = self.remaining()[..length].to_string();
        self.offset += length + 3;
        self.append_text(value);
        Ok(())
    }

    fn parse_processing_instruction(&mut self) -> Result<(), ParseError> {
        self.offset += 2;
        let Some(length) = self.remaining().find("?>") else {
            return Err(self.error("unterminated processing instruction"));
        };
        self.offset += length + 2;
        Ok(())
    }

    fn parse_doctype(&mut self) -> Result<(), ParseError> {
        if self.root.is_some() || !self.stack.is_empty() {
            return Err(self.error("DOCTYPE must precede the root element"));
        }
        self.offset += "<!DOCTYPE".len();
        let mut quote = None;
        let mut subset_depth = 0usize;
        while let Some(character) = self.peek_char() {
            self.offset += character.len_utf8();
            if let Some(expected) = quote {
                if character == expected {
                    quote = None;
                }
                continue;
            }
            match character {
                '\'' | '"' => quote = Some(character),
                '[' => subset_depth += 1,
                ']' => subset_depth = subset_depth.saturating_sub(1),
                '>' if subset_depth == 0 => return Ok(()),
                _ => {}
            }
        }
        Err(self.error("unterminated DOCTYPE"))
    }

    fn parse_attribute_value(&mut self) -> Result<String, ParseError> {
        let quote = match self.peek_char() {
            Some('\'' | '"') => self.peek_char().expect("quoted attribute"),
            _ => return Err(self.error("attribute value must be quoted")),
        };
        self.offset += 1;
        let start = self.offset;
        while let Some(character) = self.peek_char() {
            if character == quote {
                let raw = &self.input[start..self.offset];
                self.offset += 1;
                return decode_entities(raw, start);
            }
            if character == '<' {
                return Err(self.error("'<' is not valid in an attribute value"));
            }
            self.offset += character.len_utf8();
        }
        Err(self.error("unterminated attribute value"))
    }

    fn parse_name(&mut self) -> Result<&'a str, ParseError> {
        let start = self.offset;
        let Some(first) = self.peek_char() else {
            return Err(self.error("expected XML name"));
        };
        if !is_name_start(first) {
            return Err(self.error("invalid first character in XML name"));
        }
        self.offset += first.len_utf8();
        while let Some(character) = self.peek_char() {
            if !is_name_continue(character) {
                break;
            }
            self.offset += character.len_utf8();
        }
        Ok(&self.input[start..self.offset])
    }

    fn append_text(&mut self, value: String) {
        let Some(index) = self.stack.last().copied() else {
            return;
        };
        if self.elements[index].first_text.is_none() {
            self.elements[index].first_text = Some(value.clone());
        }
        self.elements[index]
            .content
            .push(ElementContent::Text(value));
    }

    fn skip_whitespace(&mut self) -> bool {
        let start = self.offset;
        while matches!(self.peek_char(), Some(' ' | '\t' | '\r' | '\n')) {
            self.offset += 1;
        }
        self.offset != start
    }

    fn take(&mut self, value: &str) -> bool {
        if self.remaining().starts_with(value) {
            self.offset += value.len();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, value: &str, reason: &'static str) -> Result<(), ParseError> {
        if self.take(value) {
            Ok(())
        } else {
            Err(self.error(reason))
        }
    }

    fn remaining(&self) -> &'a str {
        &self.input[self.offset..]
    }

    fn peek_char(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    fn error(&self, reason: &'static str) -> ParseError {
        ParseError {
            offset: self.offset,
            reason,
        }
    }
}

fn local_name(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

fn is_name_start(character: char) -> bool {
    character == ':' || character == '_' || character.is_alphabetic()
}

fn is_name_continue(character: char) -> bool {
    is_name_start(character)
        || character.is_ascii_digit()
        || matches!(character, '-' | '.')
        || (!character.is_ascii() && character.is_alphanumeric())
}

fn decode_entities(input: &str, base_offset: usize) -> Result<String, ParseError> {
    if !input.contains('&') {
        return Ok(input.to_string());
    }
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;
    while let Some(relative) = input[cursor..].find('&') {
        let ampersand = cursor + relative;
        output.push_str(&input[cursor..ampersand]);
        let entity_start = ampersand + 1;
        let Some(relative_end) = input[entity_start..].find(';') else {
            return Err(ParseError {
                offset: base_offset + ampersand,
                reason: "unterminated entity reference",
            });
        };
        let entity_end = entity_start + relative_end;
        let entity = &input[entity_start..entity_end];
        let character = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "apos" => '\'',
            "quot" => '"',
            value if value.starts_with("#x") => u32::from_str_radix(&value[2..], 16)
                .ok()
                .and_then(char::from_u32)
                .ok_or(ParseError {
                    offset: base_offset + ampersand,
                    reason: "invalid hexadecimal character reference",
                })?,
            value if value.starts_with('#') => value[1..]
                .parse::<u32>()
                .ok()
                .and_then(char::from_u32)
                .ok_or(ParseError {
                    offset: base_offset + ampersand,
                    reason: "invalid decimal character reference",
                })?,
            _ => {
                return Err(ParseError {
                    offset: base_offset + ampersand,
                    reason: "unknown entity reference",
                });
            }
        };
        if is_forbidden_xml_character(character) {
            return Err(ParseError {
                offset: base_offset + ampersand,
                reason: "character reference resolves to a forbidden XML character",
            });
        }
        output.push(character);
        cursor = entity_end + 1;
    }
    output.push_str(&input[cursor..]);
    Ok(output)
}

fn is_forbidden_xml_character(character: char) -> bool {
    let value = character as u32;
    !matches!(value, 0x09 | 0x0a | 0x0d | 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_svg_structure_namespaces_and_entities() {
        let document = Document::parse(
            r##"<?xml version="1.0"?><svg xmlns="urn:svg" xmlns:xlink="urn:xlink"><g id="a&amp;b"><xlink:image xlink:href="#asset"/></g></svg>"##,
        )
        .expect("valid XML");
        let names: Vec<&str> = document
            .descendants()
            .map(|node| node.tag_name().name())
            .collect();
        assert_eq!(names, ["svg", "g", "image"]);
        let group = document.root().children().next().expect("group");
        assert_eq!(group.attribute("id"), Some("a&b"));
        let image = group.children().next().expect("image");
        assert_eq!(image.attribute("xlink:href"), Some("#asset"));
        assert_eq!(image.attribute("href"), Some("#asset"));
        assert_eq!(image.parent().expect("parent").tag_name().name(), "g");
    }

    #[test]
    fn parses_cdata_unicode_and_numeric_references() {
        let document = Document::parse(
            "<svg><style><![CDATA[.λ { fill: red; }]]></style><title>&#x1f680; &#955;</title></svg>",
        )
        .expect("valid XML");
        let mut nodes = document.descendants();
        let _root = nodes.next();
        assert_eq!(
            nodes.next().expect("style").text(),
            Some(".λ { fill: red; }")
        );
        assert_eq!(nodes.next().expect("title").text(), Some("🚀 λ"));
    }

    #[test]
    fn preserves_mixed_text_and_element_order() {
        let document = Document::parse("<text>A<tspan>B</tspan>C</text>").expect("valid XML");
        let content = document.root().content().collect::<Vec<_>>();
        assert!(matches!(content[0], ContentNode::Text("A")));
        assert!(matches!(
            content[1],
            ContentNode::Element(node) if node.tag_name().name() == "tspan"
        ));
        assert!(matches!(content[2], ContentNode::Text("C")));
    }

    #[test]
    fn rejects_structural_and_entity_errors() {
        for invalid in [
            "<svg><g></svg>",
            "<svg a='1' a='2'/>",
            "<svg>&unknown;</svg>",
            "<svg/><svg/>",
            "<svg>",
            "text<svg/>",
        ] {
            assert!(
                Document::parse(invalid).is_err(),
                "input should be rejected: {invalid}"
            );
        }
    }
}
