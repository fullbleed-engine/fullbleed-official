//! Dependency-free HTML DOM, tokenizer, recovery tree builder, and query subset.
//!
//! FullBleed does not expose a browser DOM. It needs a deterministic owned tree for layout,
//! accessibility inspection, inline SVG extraction, and a small set of internal queries. This
//! module implements that boundary directly, including the HTML recovery rules that materially
//! affect those consumers.

use crate::html_entities::NAMED_ENTITY_TABLES;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::ops::Deref;
use std::rc::{Rc, Weak};

const HTML_NS: &str = "http://www.w3.org/1999/xhtml";
const SVG_NS: &str = "http://www.w3.org/2000/svg";
const MATHML_NS: &str = "http://www.w3.org/1998/Math/MathML";

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Name(String);

impl Name {
    fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Namespace(String);

impl Namespace {
    fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ExpandedName {
    pub(crate) ns: Namespace,
    pub(crate) local: Name,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Attribute {
    pub(crate) value: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Attributes {
    pub(crate) map: BTreeMap<ExpandedName, Attribute>,
}

impl Attributes {
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.map
            .iter()
            .find(|(key, _)| key.ns.is_empty() && key.local.as_ref().eq_ignore_ascii_case(name))
            .map(|(_, value)| value.value.as_str())
    }

    fn insert_first(&mut self, name: String, value: String) {
        if self.get(&name).is_some() {
            return;
        }
        self.map.insert(
            ExpandedName {
                ns: Namespace::new(""),
                local: Name::new(name),
            },
            Attribute { value },
        );
    }

    fn merge_missing(&mut self, other: &Attributes) {
        for (name, value) in &other.map {
            if self.get(name.local.as_ref()).is_none() {
                self.map.insert(name.clone(), value.clone());
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct QualName {
    pub(crate) ns: Namespace,
    pub(crate) local: Name,
}

#[derive(Debug)]
pub(crate) struct ElementData {
    pub(crate) name: QualName,
    pub(crate) attributes: RefCell<Attributes>,
}

#[derive(Debug)]
pub(crate) enum NodeData {
    Document,
    Doctype(String),
    Text(RefCell<String>),
    Comment(RefCell<String>),
    Element(ElementData),
}

struct Node {
    data: NodeData,
    parent: RefCell<Weak<Node>>,
    children: RefCell<Vec<NodeRef>>,
}

#[derive(Clone)]
pub(crate) struct NodeRef(Rc<Node>);

impl fmt::Debug for NodeRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.data() {
            NodeData::Document => formatter.write_str("NodeRef(Document)"),
            NodeData::Doctype(name) => formatter
                .debug_tuple("NodeRef(Doctype)")
                .field(name)
                .finish(),
            NodeData::Text(text) => formatter
                .debug_tuple("NodeRef(Text)")
                .field(&text.borrow().as_str())
                .finish(),
            NodeData::Comment(text) => formatter
                .debug_tuple("NodeRef(Comment)")
                .field(&text.borrow().as_str())
                .finish(),
            NodeData::Element(element) => formatter
                .debug_tuple("NodeRef(Element)")
                .field(&element.name.local.as_ref())
                .finish(),
        }
    }
}

impl PartialEq for NodeRef {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for NodeRef {}

impl NodeRef {
    fn new(data: NodeData) -> Self {
        Self(Rc::new(Node {
            data,
            parent: RefCell::new(Weak::new()),
            children: RefCell::new(Vec::new()),
        }))
    }

    fn element(name: String, namespace: &str, attributes: Attributes) -> Self {
        Self::new(NodeData::Element(ElementData {
            name: QualName {
                ns: Namespace::new(namespace),
                local: Name::new(name),
            },
            attributes: RefCell::new(attributes),
        }))
    }

    fn text(value: String) -> Self {
        Self::new(NodeData::Text(RefCell::new(value)))
    }

    fn comment(value: String) -> Self {
        Self::new(NodeData::Comment(RefCell::new(value)))
    }

    fn doctype(value: String) -> Self {
        Self::new(NodeData::Doctype(value))
    }

    fn append_child(&self, child: NodeRef) {
        *child.0.parent.borrow_mut() = Rc::downgrade(&self.0);
        self.0.children.borrow_mut().push(child);
    }

    fn insert_before_child(&self, reference: &NodeRef, child: NodeRef) {
        let mut children = self.0.children.borrow_mut();
        let index = children
            .iter()
            .position(|candidate| candidate == reference)
            .unwrap_or(children.len());
        *child.0.parent.borrow_mut() = Rc::downgrade(&self.0);
        children.insert(index, child);
    }

    fn detach(&self) {
        if let Some(parent) = self.parent() {
            parent.0.children.borrow_mut().retain(|child| child != self);
        }
        *self.0.parent.borrow_mut() = Weak::new();
    }

    pub(crate) fn data(&self) -> &NodeData {
        &self.0.data
    }

    pub(crate) fn as_element(&self) -> Option<&ElementData> {
        match self.data() {
            NodeData::Element(element) => Some(element),
            _ => None,
        }
    }

    pub(crate) fn parent(&self) -> Option<NodeRef> {
        self.0.parent.borrow().upgrade().map(NodeRef)
    }

    pub(crate) fn children(&self) -> Children {
        Children {
            inner: self.0.children.borrow().clone().into_iter(),
        }
    }

    pub(crate) fn descendants(&self) -> Descendants {
        let mut stack: Vec<NodeRef> = self.children().collect();
        stack.reverse();
        Descendants { stack }
    }

    pub(crate) fn ancestors(&self) -> Ancestors {
        Ancestors {
            next: Some(self.clone()),
        }
    }

    pub(crate) fn text_contents(&self) -> String {
        fn collect(node: &NodeRef, output: &mut String) {
            if let NodeData::Text(text) = node.data() {
                output.push_str(&text.borrow());
            }
            for child in node.children() {
                collect(&child, output);
            }
        }

        let mut output = String::new();
        collect(self, &mut output);
        output
    }

    pub(crate) fn select(&self, source: &str) -> Result<Select, SelectorError> {
        let selectors = parse_selector_list(source)?;
        let mut nodes = Vec::new();
        let mut candidates = Vec::new();
        if self.as_element().is_some() {
            candidates.push(self.clone());
        }
        candidates.extend(
            self.descendants()
                .filter(|node| node.as_element().is_some()),
        );
        for node in candidates {
            if selectors.iter().any(|selector| selector.matches(&node)) {
                nodes.push(SelectedNode { node });
            }
        }
        Ok(Select {
            inner: nodes.into_iter(),
        })
    }

    pub(crate) fn select_first(&self, source: &str) -> Result<SelectedNode, SelectorError> {
        self.select(source)?
            .next()
            .ok_or_else(|| SelectorError::new("selector matched no elements"))
    }
}

pub(crate) struct Children {
    inner: std::vec::IntoIter<NodeRef>,
}

impl Iterator for Children {
    type Item = NodeRef;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

pub(crate) struct Descendants {
    stack: Vec<NodeRef>,
}

impl Iterator for Descendants {
    type Item = NodeRef;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop()?;
        let children: Vec<NodeRef> = node.children().collect();
        self.stack.extend(children.into_iter().rev());
        Some(node)
    }
}

pub(crate) struct Ancestors {
    next: Option<NodeRef>,
}

impl Iterator for Ancestors {
    type Item = NodeRef;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.next.take()?;
        self.next = node.parent();
        Some(node)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SelectedNode {
    node: NodeRef,
}

impl SelectedNode {
    pub(crate) fn as_node(&self) -> &NodeRef {
        &self.node
    }

    pub(crate) fn text_contents(&self) -> String {
        self.node.text_contents()
    }
}

impl Deref for SelectedNode {
    type Target = ElementData;

    fn deref(&self) -> &Self::Target {
        self.node.as_element().expect("selected node is an element")
    }
}

pub(crate) struct Select {
    inner: std::vec::IntoIter<SelectedNode>,
}

impl Iterator for Select {
    type Item = SelectedNode;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SelectorError {
    reason: String,
}

impl SelectorError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for SelectorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Combinator {
    Descendant,
    Child,
    AdjacentSibling,
    GeneralSibling,
}

#[derive(Clone, Debug, Default)]
struct CompoundSelector {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attributes: Vec<AttributeSelector>,
}

#[derive(Clone, Debug)]
struct AttributeSelector {
    name: String,
    operator: AttributeOperator,
    value: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttributeOperator {
    Present,
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Clone, Debug)]
struct ComplexSelector {
    compounds: Vec<CompoundSelector>,
    combinators: Vec<Combinator>,
}

impl ComplexSelector {
    fn matches(&self, node: &NodeRef) -> bool {
        if self.compounds.is_empty() {
            return false;
        }
        self.matches_at(node, self.compounds.len() - 1)
    }

    fn matches_at(&self, node: &NodeRef, index: usize) -> bool {
        if !self.compounds[index].matches(node) {
            return false;
        }
        if index == 0 {
            return true;
        }
        match self.combinators[index - 1] {
            Combinator::Child => node
                .parent()
                .filter(|parent| parent.as_element().is_some())
                .is_some_and(|parent| self.matches_at(&parent, index - 1)),
            Combinator::Descendant => {
                let mut parent = node.parent();
                while let Some(candidate) = parent {
                    if candidate.as_element().is_some() && self.matches_at(&candidate, index - 1) {
                        return true;
                    }
                    parent = candidate.parent();
                }
                false
            }
            Combinator::AdjacentSibling => previous_element_siblings(node)
                .into_iter()
                .next_back()
                .is_some_and(|sibling| self.matches_at(&sibling, index - 1)),
            Combinator::GeneralSibling => previous_element_siblings(node)
                .into_iter()
                .rev()
                .any(|sibling| self.matches_at(&sibling, index - 1)),
        }
    }
}

impl CompoundSelector {
    fn matches(&self, node: &NodeRef) -> bool {
        let Some(element) = node.as_element() else {
            return false;
        };
        if let Some(tag) = &self.tag {
            if tag != "*" && !element.name.local.as_ref().eq_ignore_ascii_case(tag) {
                return false;
            }
        }
        let attributes = element.attributes.borrow();
        if let Some(id) = &self.id {
            if attributes.get("id") != Some(id.as_str()) {
                return false;
            }
        }
        let class_value = attributes.get("class").unwrap_or("");
        if self.classes.iter().any(|class| {
            !class_value
                .split_ascii_whitespace()
                .any(|candidate| candidate == class)
        }) {
            return false;
        }
        self.attributes.iter().all(|selector| {
            let value = attributes.get(&selector.name);
            match selector.operator {
                AttributeOperator::Present => value.is_some(),
                AttributeOperator::Equals => value == Some(selector.value.as_str()),
                AttributeOperator::Includes => value.is_some_and(|value| {
                    value
                        .split_ascii_whitespace()
                        .any(|candidate| candidate == selector.value)
                }),
                AttributeOperator::DashMatch => value.is_some_and(|value| {
                    value == selector.value
                        || value
                            .strip_prefix(&selector.value)
                            .is_some_and(|tail| tail.starts_with('-'))
                }),
                AttributeOperator::Prefix => {
                    value.is_some_and(|value| value.starts_with(&selector.value))
                }
                AttributeOperator::Suffix => {
                    value.is_some_and(|value| value.ends_with(&selector.value))
                }
                AttributeOperator::Substring => {
                    value.is_some_and(|value| value.contains(&selector.value))
                }
            }
        })
    }
}

fn previous_element_siblings(node: &NodeRef) -> Vec<NodeRef> {
    let Some(parent) = node.parent() else {
        return Vec::new();
    };
    parent
        .children()
        .take_while(|sibling| sibling != node)
        .filter(|sibling| sibling.as_element().is_some())
        .collect()
}

fn parse_selector_list(source: &str) -> Result<Vec<ComplexSelector>, SelectorError> {
    let groups = split_top_level(source, ',');
    if groups.is_empty() {
        return Err(SelectorError::new("empty selector"));
    }
    groups.into_iter().map(parse_complex_selector).collect()
}

fn parse_complex_selector(source: &str) -> Result<ComplexSelector, SelectorError> {
    let chars: Vec<char> = source.trim().chars().collect();
    if chars.is_empty() {
        return Err(SelectorError::new("empty selector group"));
    }
    let mut compounds = Vec::new();
    let mut combinators = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;
    let mut bracket_depth = 0usize;
    let mut quote = None;
    let mut pending_descendant = false;

    while index <= chars.len() {
        let at_end = index == chars.len();
        let ch = if at_end { '\0' } else { chars[index] };
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else if ch == '\\' {
                index = index.saturating_add(1);
            }
            index += 1;
            continue;
        }
        match ch {
            '\'' | '"' if bracket_depth > 0 => quote = Some(ch),
            '[' => bracket_depth += 1,
            ']' if bracket_depth > 0 => bracket_depth -= 1,
            _ => {}
        }
        if bracket_depth == 0
            && (at_end || ch.is_ascii_whitespace() || matches!(ch, '>' | '+' | '~'))
        {
            if start < index {
                let token: String = chars[start..index].iter().collect();
                if pending_descendant
                    && !compounds.is_empty()
                    && combinators.len() < compounds.len()
                {
                    combinators.push(Combinator::Descendant);
                }
                compounds.push(parse_compound_selector(&token)?);
            }
            if at_end {
                break;
            }
            if ch.is_ascii_whitespace() {
                pending_descendant = !compounds.is_empty();
                index += 1;
                while index < chars.len() && chars[index].is_ascii_whitespace() {
                    index += 1;
                }
                start = index;
                continue;
            }
            let combinator = match ch {
                '>' => Combinator::Child,
                '+' => Combinator::AdjacentSibling,
                '~' => Combinator::GeneralSibling,
                _ => unreachable!(),
            };
            if compounds.is_empty() || combinators.len() >= compounds.len() {
                return Err(SelectorError::new("misplaced selector combinator"));
            }
            combinators.push(combinator);
            pending_descendant = false;
            index += 1;
            while index < chars.len() && chars[index].is_ascii_whitespace() {
                index += 1;
            }
            start = index;
            continue;
        }
        index += 1;
    }
    if compounds.is_empty() || combinators.len() + 1 != compounds.len() {
        return Err(SelectorError::new("incomplete selector"));
    }
    Ok(ComplexSelector {
        compounds,
        combinators,
    })
}

fn parse_compound_selector(source: &str) -> Result<CompoundSelector, SelectorError> {
    let bytes = source.as_bytes();
    let mut selector = CompoundSelector::default();
    let mut offset = 0usize;
    if bytes
        .first()
        .is_some_and(|byte| *byte == b'*' || is_css_name_start(*byte))
    {
        let start = offset;
        offset += 1;
        while offset < bytes.len() && is_css_name_char(bytes[offset]) {
            offset += 1;
        }
        selector.tag = Some(source[start..offset].to_ascii_lowercase());
    }
    while offset < bytes.len() {
        match bytes[offset] {
            b'#' | b'.' => {
                let marker = bytes[offset];
                offset += 1;
                let start = offset;
                while offset < bytes.len() && is_css_name_char(bytes[offset]) {
                    offset += 1;
                }
                if start == offset {
                    return Err(SelectorError::new("empty id or class selector"));
                }
                let value = source[start..offset].to_string();
                if marker == b'#' {
                    selector.id = Some(value);
                } else {
                    selector.classes.push(value);
                }
            }
            b'[' => {
                let end = find_selector_bracket_end(source, offset)?;
                selector
                    .attributes
                    .push(parse_attribute_selector(&source[offset + 1..end])?);
                offset = end + 1;
            }
            b':' => {
                return Err(SelectorError::new(
                    "pseudo-classes are outside FullBleed's DOM query subset",
                ));
            }
            _ => return Err(SelectorError::new("invalid compound selector")),
        }
    }
    if selector.tag.is_none()
        && selector.id.is_none()
        && selector.classes.is_empty()
        && selector.attributes.is_empty()
    {
        return Err(SelectorError::new("empty compound selector"));
    }
    Ok(selector)
}

fn find_selector_bracket_end(source: &str, start: usize) -> Result<usize, SelectorError> {
    let bytes = source.as_bytes();
    let mut quote = None;
    let mut offset = start + 1;
    while offset < bytes.len() {
        let ch = bytes[offset] as char;
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else if ch == '\\' {
                offset = offset.saturating_add(1);
            }
        } else if matches!(ch, '\'' | '"') {
            quote = Some(ch);
        } else if ch == ']' {
            return Ok(offset);
        }
        offset += 1;
    }
    Err(SelectorError::new("unclosed attribute selector"))
}

fn parse_attribute_selector(source: &str) -> Result<AttributeSelector, SelectorError> {
    let source = source.trim();
    let operators = ["~=", "|=", "^=", "$=", "*=", "="];
    for operator in operators {
        if let Some(index) = source.find(operator) {
            let name = source[..index].trim().to_ascii_lowercase();
            if name.is_empty() {
                return Err(SelectorError::new("attribute selector has no name"));
            }
            let value = source[index + operator.len()..].trim();
            let value = strip_matching_quotes(value).to_string();
            let operator = match operator {
                "=" => AttributeOperator::Equals,
                "~=" => AttributeOperator::Includes,
                "|=" => AttributeOperator::DashMatch,
                "^=" => AttributeOperator::Prefix,
                "$=" => AttributeOperator::Suffix,
                "*=" => AttributeOperator::Substring,
                _ => unreachable!(),
            };
            return Ok(AttributeSelector {
                name,
                operator,
                value,
            });
        }
    }
    if source.is_empty() {
        return Err(SelectorError::new("attribute selector has no name"));
    }
    Ok(AttributeSelector {
        name: source.to_ascii_lowercase(),
        operator: AttributeOperator::Present,
        value: String::new(),
    })
}

fn strip_matching_quotes(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if matches!(bytes[0], b'\'' | b'"') && bytes[value.len() - 1] == bytes[0] {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn is_css_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'-')
}

fn is_css_name_char(byte: u8) -> bool {
    is_css_name_start(byte) || byte.is_ascii_digit()
}

fn split_top_level(source: &str, separator: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut bracket_depth = 0usize;
    let mut quote = None;
    for (offset, ch) in source.char_indices() {
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '[' | '(' => bracket_depth += 1,
            ']' | ')' => bracket_depth = bracket_depth.saturating_sub(1),
            _ if ch == separator && bracket_depth == 0 => {
                if !source[start..offset].trim().is_empty() {
                    out.push(source[start..offset].trim());
                }
                start = offset + ch.len_utf8();
            }
            _ => {}
        }
    }
    if !source[start..].trim().is_empty() {
        out.push(source[start..].trim());
    }
    out
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Head,
    AfterHead,
    Body,
}

#[derive(Clone, Debug)]
struct StartTag {
    name: String,
    attributes: Attributes,
    self_closing: bool,
}

#[derive(Clone, Debug)]
struct ActiveFormatting {
    tag: String,
    namespace: String,
    attributes: Attributes,
    node: NodeRef,
}

struct HtmlParser<'a> {
    input: &'a str,
    offset: usize,
    document: NodeRef,
    html: NodeRef,
    head: NodeRef,
    body: NodeRef,
    stack: Vec<NodeRef>,
    active_formatting: Vec<ActiveFormatting>,
    form: Option<NodeRef>,
    head_start_seen: bool,
    phase: Phase,
}

pub(crate) fn parse_html(input: &str) -> NodeRef {
    HtmlParser::new(input).parse()
}

impl<'a> HtmlParser<'a> {
    fn new(input: &'a str) -> Self {
        let document = NodeRef::new(NodeData::Document);
        let html = NodeRef::element("html".to_string(), HTML_NS, Attributes::default());
        let head = NodeRef::element("head".to_string(), HTML_NS, Attributes::default());
        let body = NodeRef::element("body".to_string(), HTML_NS, Attributes::default());
        document.append_child(html.clone());
        html.append_child(head.clone());
        html.append_child(body.clone());
        Self {
            input,
            offset: 0,
            document,
            html: html.clone(),
            head,
            body,
            stack: vec![html],
            active_formatting: Vec::new(),
            form: None,
            head_start_seen: false,
            phase: Phase::Head,
        }
    }

    fn parse(mut self) -> NodeRef {
        while self.offset < self.input.len() {
            if let Some((tag, decode_entities)) = self.raw_text_context() {
                if self.consume_raw_text(&tag, decode_entities) {
                    continue;
                }
            }
            if self.remaining().starts_with("<![CDATA[") && self.current_namespace() != HTML_NS {
                self.consume_foreign_cdata();
            } else if self.remaining().starts_with("<!--") {
                self.consume_comment();
            } else if starts_ascii_case_insensitive(self.remaining(), "<!doctype") {
                self.consume_doctype();
            } else if self.remaining().starts_with("<!") || self.remaining().starts_with("<?") {
                self.consume_bogus_comment();
            } else if self.remaining().starts_with("</") {
                if let Some(name) = self.consume_end_tag() {
                    self.handle_end_tag(&name);
                } else {
                    self.consume_text_byte();
                }
            } else if self.remaining().starts_with('<') {
                if let Some(tag) = self.consume_start_tag() {
                    self.handle_start_tag(tag);
                } else {
                    self.consume_text_byte();
                }
            } else {
                self.consume_normal_text();
            }
        }
        self.document
    }

    fn remaining(&self) -> &'a str {
        &self.input[self.offset..]
    }

    fn raw_text_context(&self) -> Option<(String, bool)> {
        let node = self.stack.last()?;
        let element = node.as_element()?;
        if element.name.ns.0 != HTML_NS && element.name.ns.0 != SVG_NS {
            return None;
        }
        let tag = element.name.local.as_ref();
        match tag.to_ascii_lowercase().as_str() {
            "script" | "style" | "xmp" | "iframe" | "noembed" | "noframes" | "noscript" => {
                Some((tag.to_string(), false))
            }
            "title" | "textarea" => Some((tag.to_string(), true)),
            "plaintext" => Some((tag.to_string(), false)),
            _ => None,
        }
    }

    fn consume_raw_text(&mut self, tag: &str, decode_entities: bool) -> bool {
        let close_start = if tag.eq_ignore_ascii_case("plaintext") {
            None
        } else {
            find_raw_end_tag(self.remaining(), tag)
        };
        let content_end = close_start.unwrap_or(self.remaining().len());
        if content_end > 0 {
            let source = &self.remaining()[..content_end];
            let text = if decode_entities {
                decode_character_references(source)
            } else {
                replace_nulls(source)
            };
            self.append_text_to_current(text);
            self.offset += content_end;
            return true;
        }
        if close_start.is_none() {
            self.offset = self.input.len();
            return true;
        }
        false
    }

    fn consume_comment(&mut self) {
        let start = self.offset + 4;
        if let Some(relative_end) = self.input[start..].find("-->") {
            let end = start + relative_end;
            let comment = NodeRef::comment(replace_nulls(&self.input[start..end]));
            self.current_parent().append_child(comment);
            self.offset = end + 3;
        } else {
            let comment = NodeRef::comment(replace_nulls(&self.input[start..]));
            self.current_parent().append_child(comment);
            self.offset = self.input.len();
        }
    }

    fn consume_foreign_cdata(&mut self) {
        let start = self.offset + "<![CDATA[".len();
        let end = self.input[start..]
            .find("]]>")
            .map(|relative| start + relative)
            .unwrap_or(self.input.len());
        self.append_text_to_current(replace_nulls(&self.input[start..end]));
        self.offset = (end + usize::from(end < self.input.len()) * 3).min(self.input.len());
    }

    fn consume_doctype(&mut self) {
        let start = self.offset + 2;
        let end = self.input[start..]
            .find('>')
            .map(|relative| start + relative)
            .unwrap_or(self.input.len());
        let declaration = self.input[start..end].trim();
        let name = declaration
            .strip_prefix("DOCTYPE")
            .or_else(|| declaration.strip_prefix("doctype"))
            .unwrap_or(declaration)
            .split_ascii_whitespace()
            .next()
            .unwrap_or("html")
            .to_ascii_lowercase();
        let node = NodeRef::doctype(name);
        self.document.insert_before_child(&self.html, node);
        self.offset = (end + usize::from(end < self.input.len())).min(self.input.len());
    }

    fn consume_bogus_comment(&mut self) {
        let start = self.offset + 2;
        let end = self.input[start..]
            .find('>')
            .map(|relative| start + relative)
            .unwrap_or(self.input.len());
        self.current_parent()
            .append_child(NodeRef::comment(replace_nulls(&self.input[start..end])));
        self.offset = (end + usize::from(end < self.input.len())).min(self.input.len());
    }

    fn consume_end_tag(&mut self) -> Option<String> {
        let mut cursor = self.offset + 2;
        skip_ascii_whitespace(self.input, &mut cursor);
        let name_start = cursor;
        while cursor < self.input.len() && is_html_name_char(self.input.as_bytes()[cursor]) {
            cursor += 1;
        }
        if cursor == name_start {
            return None;
        }
        let name = self.input[name_start..cursor].to_ascii_lowercase();
        if let Some(relative_end) = self.input[cursor..].find('>') {
            self.offset = cursor + relative_end + 1;
        } else {
            self.offset = self.input.len();
        }
        Some(name)
    }

    fn consume_start_tag(&mut self) -> Option<StartTag> {
        let mut cursor = self.offset + 1;
        if cursor >= self.input.len() || !is_html_name_start(self.input.as_bytes()[cursor]) {
            return None;
        }
        let name_start = cursor;
        cursor += 1;
        while cursor < self.input.len() && is_html_name_char(self.input.as_bytes()[cursor]) {
            cursor += 1;
        }
        let name = self.input[name_start..cursor].to_ascii_lowercase();
        let mut attributes = Attributes::default();
        let mut self_closing = false;
        loop {
            skip_ascii_whitespace(self.input, &mut cursor);
            if cursor >= self.input.len() {
                self.offset = cursor;
                break;
            }
            match self.input.as_bytes()[cursor] {
                b'>' => {
                    cursor += 1;
                    self.offset = cursor;
                    break;
                }
                b'/' if self.input.as_bytes().get(cursor + 1) == Some(&b'>') => {
                    self_closing = true;
                    cursor += 2;
                    self.offset = cursor;
                    break;
                }
                b'/' => {
                    cursor += 1;
                }
                _ => {
                    let attr_start = cursor;
                    while cursor < self.input.len()
                        && !self.input.as_bytes()[cursor].is_ascii_whitespace()
                        && !matches!(self.input.as_bytes()[cursor], b'=' | b'>' | b'/')
                    {
                        cursor += 1;
                    }
                    if cursor == attr_start {
                        cursor += 1;
                        continue;
                    }
                    let attr_name = self.input[attr_start..cursor].to_ascii_lowercase();
                    skip_ascii_whitespace(self.input, &mut cursor);
                    let value = if self.input.as_bytes().get(cursor) == Some(&b'=') {
                        cursor += 1;
                        skip_ascii_whitespace(self.input, &mut cursor);
                        consume_attribute_value(self.input, &mut cursor)
                    } else {
                        String::new()
                    };
                    attributes
                        .insert_first(attr_name, decode_character_references_in_attribute(&value));
                }
            }
        }
        Some(StartTag {
            name,
            attributes,
            self_closing,
        })
    }

    fn consume_normal_text(&mut self) {
        let end = self.remaining().find('<').unwrap_or(self.remaining().len());
        if end == 0 {
            self.consume_text_byte();
            return;
        }
        let text = decode_character_references(&self.remaining()[..end]);
        self.offset += end;
        self.handle_text(text);
    }

    fn skip_template_contents(&mut self) {
        let mut depth = 1usize;
        while self.offset < self.input.len() {
            let Some(relative) = self.input[self.offset..].find('<') else {
                self.offset = self.input.len();
                return;
            };
            let start = self.offset + relative;
            let remaining = &self.input[start..];
            if remaining.starts_with("<!--") {
                self.offset = remaining
                    .find("-->")
                    .map(|end| start + end + 3)
                    .unwrap_or(self.input.len());
                continue;
            }
            let (closing, name_start) = if remaining.starts_with("</") {
                (true, start + 2)
            } else {
                (false, start + 1)
            };
            let mut name_end = name_start;
            while name_end < self.input.len() && is_html_name_char(self.input.as_bytes()[name_end])
            {
                name_end += 1;
            }
            let name = self.input[name_start..name_end].to_ascii_lowercase();
            self.offset = find_markup_end(self.input, name_end)
                .map(|end| end + 1)
                .unwrap_or(self.input.len());
            if name == "template" {
                if closing {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                } else {
                    depth += 1;
                }
            } else if !closing && matches!(name.as_str(), "script" | "style") {
                if let Some(raw_end) = find_raw_end_tag(&self.input[self.offset..], &name) {
                    self.offset += raw_end;
                    self.offset = self.input[self.offset..]
                        .find('>')
                        .map(|end| self.offset + end + 1)
                        .unwrap_or(self.input.len());
                } else {
                    self.offset = self.input.len();
                }
            }
        }
    }

    fn consume_text_byte(&mut self) {
        let Some(ch) = self.remaining().chars().next() else {
            return;
        };
        self.offset += ch.len_utf8();
        self.handle_text(if ch == '\0' {
            "\u{fffd}".to_string()
        } else {
            ch.to_string()
        });
    }

    fn handle_text(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        if self.phase != Phase::Body && text.trim().is_empty() {
            self.append_text_to_current(text);
            return;
        }
        if self.phase != Phase::Body {
            self.enter_body();
        }
        self.reconstruct_active_formatting();
        if self.in_table_insertion_context() && !text.trim().is_empty() {
            self.append_fostered_text(text);
        } else {
            self.append_text_to_current(text);
        }
    }

    fn handle_start_tag(&mut self, mut tag: StartTag) {
        match tag.name.as_str() {
            "html" => {
                self.merge_attributes(&self.html.clone(), &tag.attributes);
                return;
            }
            "head" => {
                if self.phase != Phase::Head || self.head_start_seen {
                    return;
                }
                self.head_start_seen = true;
                self.phase = Phase::Head;
                self.stack.truncate(1);
                self.stack.push(self.head.clone());
                self.merge_attributes(&self.head.clone(), &tag.attributes);
                return;
            }
            "body" => {
                self.enter_body();
                self.merge_attributes(&self.body.clone(), &tag.attributes);
                return;
            }
            _ => {}
        }

        if tag.name == "frameset" && !self.stack_has_html_tag("frameset") {
            let body_has_content = self.body.children().any(|child| match child.data() {
                NodeData::Text(text) => !text.borrow().trim().is_empty(),
                NodeData::Comment(_) => false,
                _ => true,
            });
            if body_has_content {
                return;
            }
            self.body.detach();
            self.stack.truncate(1);
            self.phase = Phase::Body;
        }

        if self.phase != Phase::Body && is_head_element(&tag.name) {
            self.ensure_head_stack();
        } else if self.phase != Phase::Body {
            self.enter_body();
        }

        if self.in_select_context() {
            match tag.name.as_str() {
                "hr" => {
                    self.pop_through_if_present("option");
                    self.pop_through_if_present("optgroup");
                }
                "option" | "optgroup" | "script" | "template" => {}
                "input" | "keygen" | "textarea" => {
                    self.pop_through("select");
                    self.handle_start_tag(tag);
                    return;
                }
                "select" => {
                    self.pop_through("select");
                    return;
                }
                _ => return,
            }
        }

        let mut current_namespace = self.current_namespace().to_string();
        let at_foreign_integration_point = (current_namespace == SVG_NS
            && (self.current_tag_eq("foreignObject")
                || self.current_tag_eq("desc")
                || self.current_tag_eq("title")))
            || (current_namespace == MATHML_NS && self.mathml_html_integration_point(&tag.name));
        if current_namespace != HTML_NS
            && !at_foreign_integration_point
            && is_foreign_content_breakout(&tag.name, &tag.attributes)
        {
            while self.current_namespace() != HTML_NS && self.stack.len() > 1 {
                self.stack.pop();
            }
            current_namespace = self.current_namespace().to_string();
        }
        let namespace = if tag.name == "svg" && current_namespace == HTML_NS {
            SVG_NS.to_string()
        } else if tag.name == "math" && current_namespace == HTML_NS {
            MATHML_NS.to_string()
        } else if current_namespace == SVG_NS
            && (self.current_tag_eq("foreignObject")
                || self.current_tag_eq("desc")
                || self.current_tag_eq("title"))
        {
            HTML_NS.to_string()
        } else if current_namespace == MATHML_NS && self.mathml_html_integration_point(&tag.name) {
            HTML_NS.to_string()
        } else {
            current_namespace
        };
        if namespace == SVG_NS {
            tag.name = adjusted_svg_tag_name(&tag.name).to_string();
            tag.attributes = adjusted_svg_attributes(tag.attributes);
        } else if namespace == HTML_NS && tag.name == "image" {
            tag.name = "img".to_string();
        }

        if namespace == HTML_NS {
            if is_table_structure_element(&tag.name) && !self.stack_has_html_tag("table") {
                return;
            }
            if tag.name == "form" && self.form.is_some() {
                return;
            }
            if is_block_that_closes_p(&tag.name) {
                self.suspend_active_formatting_for_block();
            }
            self.apply_start_tag_recovery(&tag.name);
            if is_formatting_element(&tag.name) {
                self.reconstruct_active_formatting();
            }
        }

        let table_form =
            namespace == HTML_NS && tag.name == "form" && self.in_table_insertion_context();
        let hidden_table_input = namespace == HTML_NS
            && tag.name == "input"
            && self.in_table_insertion_context()
            && tag
                .attributes
                .get("type")
                .is_some_and(|value| value.eq_ignore_ascii_case("hidden"));
        let foster = namespace == HTML_NS
            && self.in_table_insertion_context()
            && !hidden_table_input
            && !is_table_allowed_start(&tag.name);
        let node = NodeRef::element(tag.name.clone(), &namespace, tag.attributes.clone());
        if foster {
            self.append_fostered_node(node.clone());
        } else {
            self.current_parent().append_child(node.clone());
        }

        if namespace == HTML_NS && is_formatting_element(&tag.name) {
            self.active_formatting.push(ActiveFormatting {
                tag: tag.name.clone(),
                namespace: namespace.clone(),
                attributes: tag.attributes.clone(),
                node: node.clone(),
            });
        }
        if namespace == HTML_NS && tag.name == "form" {
            self.form = Some(node.clone());
        }
        if namespace == HTML_NS && tag.name == "template" {
            self.skip_template_contents();
            return;
        }

        let is_void = namespace == HTML_NS && is_void_element(&tag.name);
        let foreign_self_closing = namespace != HTML_NS && tag.self_closing;
        if !is_void && !foreign_self_closing && !table_form {
            self.stack.push(node);
        }
    }

    fn handle_end_tag(&mut self, raw_name: &str) {
        if raw_name.eq_ignore_ascii_case("br") {
            self.handle_start_tag(StartTag {
                name: "br".to_string(),
                attributes: Attributes::default(),
                self_closing: true,
            });
            return;
        }
        if self.in_select_context() {
            match raw_name.to_ascii_lowercase().as_str() {
                "option" => {
                    if self.current_tag_eq("option") {
                        self.stack.pop();
                    }
                    return;
                }
                "optgroup" => {
                    if self.current_tag_eq("option") {
                        self.stack.pop();
                    }
                    if self.current_tag_eq("optgroup") {
                        self.stack.pop();
                    }
                    return;
                }
                "select" => {
                    self.pop_through("select");
                    return;
                }
                "template" => {}
                _ => return,
            }
        }
        let name = if self.current_namespace() == SVG_NS {
            adjusted_svg_tag_name(raw_name)
        } else {
            raw_name
        };
        match name.to_ascii_lowercase().as_str() {
            "html" | "body" => {
                self.stack.truncate(1);
                self.stack.push(self.body.clone());
                self.phase = Phase::Body;
                return;
            }
            "head" => {
                self.stack.truncate(1);
                self.phase = Phase::AfterHead;
                return;
            }
            "form" => {
                if let Some(form) = self.form.take() {
                    self.stack.retain(|node| node != &form);
                }
                return;
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.pop_first_present(&["h1", "h2", "h3", "h4", "h5", "h6"]);
                return;
            }
            "p" if !self.stack_has_html_tag("p") => {
                self.handle_start_tag(StartTag {
                    name: "p".to_string(),
                    attributes: Attributes::default(),
                    self_closing: false,
                });
            }
            _ => {}
        }
        if is_formatting_element(&name.to_ascii_lowercase()) {
            self.close_formatting_element(name);
            return;
        }
        self.pop_through(name);
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "table" | "tbody" | "thead" | "tfoot"
        ) {
            let stack = self.stack.clone();
            self.active_formatting
                .retain(|entry| stack.iter().any(|node| node == &entry.node));
        }
    }

    fn ensure_head_stack(&mut self) {
        self.stack.truncate(1);
        if self.stack.last() != Some(&self.head) {
            self.stack.push(self.head.clone());
        }
    }

    fn enter_body(&mut self) {
        self.phase = Phase::Body;
        self.stack.truncate(1);
        if self.stack.last() != Some(&self.body) {
            self.stack.push(self.body.clone());
        }
    }

    fn current_parent(&self) -> NodeRef {
        self.stack
            .last()
            .cloned()
            .unwrap_or_else(|| self.body.clone())
    }

    fn current_namespace(&self) -> &str {
        self.stack
            .last()
            .and_then(NodeRef::as_element)
            .map(|element| element.name.ns.0.as_str())
            .unwrap_or(HTML_NS)
    }

    fn current_tag_eq(&self, expected: &str) -> bool {
        self.stack
            .last()
            .and_then(NodeRef::as_element)
            .is_some_and(|element| element.name.local.as_ref().eq_ignore_ascii_case(expected))
    }

    fn in_select_context(&self) -> bool {
        for node in self.stack.iter().rev() {
            if node_tag_eq(node, "select") {
                return true;
            }
            if node_tag_eq(node, "template") {
                return false;
            }
        }
        false
    }

    fn mathml_html_integration_point(&self, incoming_tag: &str) -> bool {
        let Some(element) = self.stack.last().and_then(NodeRef::as_element) else {
            return false;
        };
        let current = element.name.local.as_ref().to_ascii_lowercase();
        if matches!(current.as_str(), "mi" | "mo" | "mn" | "ms" | "mtext") {
            return !matches!(incoming_tag, "mglyph" | "malignmark");
        }
        false
    }

    fn merge_attributes(&self, node: &NodeRef, attributes: &Attributes) {
        if let Some(element) = node.as_element() {
            element.attributes.borrow_mut().merge_missing(attributes);
        }
    }

    fn apply_start_tag_recovery(&mut self, name: &str) {
        if name == "p" || is_block_that_closes_p(name) {
            self.pop_through_if_present("p");
        }
        match name {
            "a" | "nobr"
                if self
                    .active_formatting
                    .iter()
                    .any(|entry| entry.tag.eq_ignore_ascii_case(name)) =>
            {
                self.close_formatting_element(name)
            }
            "button" => self.pop_through_if_present("button"),
            "li" => self.pop_list_item_if_in_scope(),
            "dt" | "dd" => self.pop_first_present(&["dt", "dd"]),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.pop_first_present(&["h1", "h2", "h3", "h4", "h5", "h6"])
            }
            "option" => self.pop_through_if_present("option"),
            "rp" | "rt" => self.pop_first_present(&["rp", "rt"]),
            "optgroup" => {
                self.pop_through_if_present("option");
                self.pop_through_if_present("optgroup");
            }
            "tr" => self.prepare_table_row(),
            "td" | "th" => self.prepare_table_cell(),
            "caption" | "tbody" | "thead" | "tfoot" | "colgroup" => {
                self.truncate_to_nearest_table()
            }
            "col" => self.ensure_colgroup(),
            "table" if self.in_table_insertion_context() => self.pop_through("table"),
            _ => {}
        }
    }

    fn ensure_table_section(&mut self) {
        let Some(table_index) = self
            .stack
            .iter()
            .rposition(|node| node_tag_eq(node, "table"))
        else {
            return;
        };
        if self.stack[table_index + 1..].iter().any(|node| {
            node_tag_eq(node, "tbody") || node_tag_eq(node, "thead") || node_tag_eq(node, "tfoot")
        }) {
            return;
        }
        self.stack.truncate(table_index + 1);
        let tbody = NodeRef::element("tbody".to_string(), HTML_NS, Attributes::default());
        self.current_parent().append_child(tbody.clone());
        self.stack.push(tbody);
    }

    fn truncate_to_nearest_table(&mut self) {
        if let Some(table_index) = self
            .stack
            .iter()
            .rposition(|node| node_tag_eq(node, "table"))
        {
            self.stack.truncate(table_index + 1);
        }
    }

    fn prepare_table_row(&mut self) {
        let Some(table_index) = self
            .stack
            .iter()
            .rposition(|node| node_tag_eq(node, "table"))
        else {
            return;
        };
        if let Some(section_index) = self.stack[table_index + 1..]
            .iter()
            .rposition(|node| {
                node_tag_eq(node, "tbody")
                    || node_tag_eq(node, "thead")
                    || node_tag_eq(node, "tfoot")
            })
            .map(|relative| table_index + 1 + relative)
        {
            self.stack.truncate(section_index + 1);
        } else {
            self.stack.truncate(table_index + 1);
            self.ensure_table_section();
        }
    }

    fn prepare_table_cell(&mut self) {
        let Some(table_index) = self
            .stack
            .iter()
            .rposition(|node| node_tag_eq(node, "table"))
        else {
            return;
        };
        if let Some(cell_index) = self.stack[table_index + 1..]
            .iter()
            .rposition(|node| node_tag_eq(node, "td") || node_tag_eq(node, "th"))
            .map(|relative| table_index + 1 + relative)
        {
            self.stack.truncate(cell_index);
        }
        self.ensure_table_row();
    }

    fn ensure_table_row(&mut self) {
        let nearest_table = self
            .stack
            .iter()
            .rposition(|node| node_tag_eq(node, "table"));
        if nearest_table.is_some_and(|index| {
            self.stack[index + 1..]
                .iter()
                .any(|node| node_tag_eq(node, "tr"))
        }) {
            return;
        }
        self.ensure_table_section();
        if nearest_table.is_none() {
            return;
        }
        let row = NodeRef::element("tr".to_string(), HTML_NS, Attributes::default());
        self.current_parent().append_child(row.clone());
        self.stack.push(row);
    }

    fn ensure_colgroup(&mut self) {
        if self.current_tag_eq("colgroup") {
            return;
        }
        let Some(table_index) = self
            .stack
            .iter()
            .rposition(|node| node_tag_eq(node, "table"))
        else {
            return;
        };
        self.stack.truncate(table_index + 1);
        let group = NodeRef::element("colgroup".to_string(), HTML_NS, Attributes::default());
        self.current_parent().append_child(group.clone());
        self.stack.push(group);
    }

    fn in_table_insertion_context(&self) -> bool {
        let Some(table_index) = self
            .stack
            .iter()
            .rposition(|node| node_tag_eq(node, "table"))
        else {
            return false;
        };
        self.stack[table_index + 1..].iter().all(|node| {
            node.as_element().is_some_and(|element| {
                matches!(
                    element.name.local.as_ref().to_ascii_lowercase().as_str(),
                    "tbody" | "thead" | "tfoot" | "tr"
                )
            })
        })
    }

    fn foster_parent(&self) -> Option<(NodeRef, NodeRef)> {
        let table = self
            .stack
            .iter()
            .rev()
            .find(|node| node_tag_eq(node, "table"))?
            .clone();
        table.parent().map(|parent| (parent, table))
    }

    fn append_fostered_node(&self, node: NodeRef) {
        if let Some((parent, table)) = self.foster_parent() {
            parent.insert_before_child(&table, node);
        } else {
            self.current_parent().append_child(node);
        }
    }

    fn append_fostered_text(&self, text: String) {
        if let Some((parent, table)) = self.foster_parent() {
            append_or_coalesce_text_before(&parent, &table, text);
        } else {
            append_or_coalesce_text(&self.current_parent(), text);
        }
    }

    fn append_text_to_current(&self, mut text: String) {
        let parent = self.current_parent();
        if parent.0.children.borrow().is_empty()
            && parent.as_element().is_some_and(|element| {
                matches!(
                    element.name.local.as_ref().to_ascii_lowercase().as_str(),
                    "pre" | "listing" | "textarea"
                )
            })
            && text.starts_with('\n')
        {
            text.remove(0);
        }
        append_or_coalesce_text(&parent, text);
    }

    fn stack_has_html_tag(&self, name: &str) -> bool {
        self.stack.iter().any(|node| {
            node.as_element().is_some_and(|element| {
                element.name.ns.0 == HTML_NS
                    && element.name.local.as_ref().eq_ignore_ascii_case(name)
            })
        })
    }

    fn pop_through_if_present(&mut self, name: &str) {
        if self.stack_has_html_tag(name) {
            self.pop_through(name);
        }
    }

    fn pop_list_item_if_in_scope(&mut self) {
        for node in self.stack.iter().rev() {
            if node_tag_eq(node, "li") {
                self.pop_through("li");
                return;
            }
            // A nested list establishes a new list-item scope. An incoming
            // <li> inside it must not implicitly close an outer list's <li>.
            if ["ol", "ul", "menu", "html", "table", "template"]
                .iter()
                .any(|boundary| node_tag_eq(node, boundary))
            {
                return;
            }
        }
    }

    fn pop_first_present(&mut self, names: &[&str]) {
        if let Some((_, name)) = self
            .stack
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, node)| {
                names
                    .iter()
                    .find(|name| node_tag_eq(node, name))
                    .map(|name| (index, *name))
            })
        {
            self.pop_through(name);
        }
    }

    fn pop_through(&mut self, name: &str) {
        if let Some(index) = self.stack.iter().rposition(|node| node_tag_eq(node, name)) {
            let minimum = if self.phase == Phase::Body { 2 } else { 1 };
            self.stack.truncate(index.max(minimum - 1));
            if self.stack.is_empty() {
                self.stack.push(self.html.clone());
            }
        }
    }

    fn node_is_on_stack(&self, node: &NodeRef) -> bool {
        self.stack.iter().any(|candidate| candidate == node)
    }

    fn reconstruct_active_formatting(&mut self) {
        let start = self
            .active_formatting
            .iter()
            .rposition(|entry| self.node_is_on_stack(&entry.node))
            .map(|index| index + 1)
            .unwrap_or(0);
        for index in start..self.active_formatting.len() {
            let entry = self.active_formatting[index].clone();
            let node = NodeRef::element(
                entry.tag.clone(),
                &entry.namespace,
                entry.attributes.clone(),
            );
            self.current_parent().append_child(node.clone());
            self.stack.push(node.clone());
            self.active_formatting[index].node = node;
        }
    }

    fn suspend_active_formatting_for_block(&mut self) {
        while self.stack.len() > 1
            && self.stack.last().is_some_and(|node| {
                self.active_formatting
                    .iter()
                    .any(|entry| entry.node == *node)
            })
        {
            self.stack.pop();
        }
    }

    fn close_formatting_element(&mut self, name: &str) {
        let Some(active_index) = self
            .active_formatting
            .iter()
            .rposition(|entry| entry.tag.eq_ignore_ascii_case(name))
        else {
            self.pop_through(name);
            return;
        };
        let target = self.active_formatting[active_index].node.clone();
        if let Some(stack_index) = self.stack.iter().rposition(|node| node == &target) {
            self.stack.truncate(stack_index);
            if self.stack.is_empty() {
                self.stack.push(self.html.clone());
            }
        }
        self.active_formatting.remove(active_index);
    }
}

fn append_or_coalesce_text(parent: &NodeRef, text: String) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = parent.0.children.borrow().last() {
        if let NodeData::Text(existing) = last.data() {
            existing.borrow_mut().push_str(&text);
            return;
        }
    }
    parent.append_child(NodeRef::text(text));
}

fn append_or_coalesce_text_before(parent: &NodeRef, reference: &NodeRef, text: String) {
    if text.is_empty() {
        return;
    }
    let children = parent.0.children.borrow();
    let index = children
        .iter()
        .position(|candidate| candidate == reference)
        .unwrap_or(children.len());
    if index > 0 {
        if let NodeData::Text(existing) = children[index - 1].data() {
            existing.borrow_mut().push_str(&text);
            return;
        }
    }
    drop(children);
    parent.insert_before_child(reference, NodeRef::text(text));
}

fn node_tag_eq(node: &NodeRef, name: &str) -> bool {
    node.as_element()
        .is_some_and(|element| element.name.local.as_ref().eq_ignore_ascii_case(name))
}

fn is_head_element(name: &str) -> bool {
    matches!(
        name,
        "base"
            | "basefont"
            | "bgsound"
            | "link"
            | "meta"
            | "noframes"
            | "noscript"
            | "script"
            | "style"
            | "template"
            | "title"
    )
}

fn is_void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "frame"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn is_formatting_element(name: &str) -> bool {
    matches!(
        name,
        "a" | "b"
            | "big"
            | "code"
            | "em"
            | "font"
            | "i"
            | "nobr"
            | "s"
            | "small"
            | "strike"
            | "strong"
            | "tt"
            | "u"
    )
}

fn is_block_that_closes_p(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "details"
            | "dialog"
            | "dir"
            | "div"
            | "dl"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hgroup"
            | "hr"
            | "main"
            | "menu"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "search"
            | "section"
            | "table"
            | "ul"
    )
}

fn is_table_allowed_start(name: &str) -> bool {
    matches!(
        name,
        "caption"
            | "col"
            | "colgroup"
            | "form"
            | "script"
            | "style"
            | "tbody"
            | "td"
            | "template"
            | "tfoot"
            | "th"
            | "thead"
            | "tr"
    )
}

fn is_table_structure_element(name: &str) -> bool {
    matches!(
        name,
        "caption" | "col" | "colgroup" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr"
    )
}

fn is_foreign_content_breakout(name: &str, attributes: &Attributes) -> bool {
    matches!(
        name,
        "b" | "big"
            | "blockquote"
            | "body"
            | "br"
            | "center"
            | "code"
            | "dd"
            | "div"
            | "dl"
            | "dt"
            | "em"
            | "embed"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "head"
            | "hr"
            | "i"
            | "img"
            | "li"
            | "listing"
            | "menu"
            | "meta"
            | "nobr"
            | "ol"
            | "p"
            | "pre"
            | "ruby"
            | "s"
            | "small"
            | "span"
            | "strong"
            | "strike"
            | "sub"
            | "sup"
            | "table"
            | "tt"
            | "u"
            | "ul"
            | "var"
    ) || (name == "font"
        && ["color", "face", "size"]
            .iter()
            .any(|attribute| attributes.get(attribute).is_some()))
}

fn adjusted_svg_tag_name(name: &str) -> &str {
    match name.to_ascii_lowercase().as_str() {
        "altglyph" => "altGlyph",
        "altglyphdef" => "altGlyphDef",
        "altglyphitem" => "altGlyphItem",
        "animatecolor" => "animateColor",
        "animatemotion" => "animateMotion",
        "animatetransform" => "animateTransform",
        "clippath" => "clipPath",
        "feblend" => "feBlend",
        "fecolormatrix" => "feColorMatrix",
        "fecomponenttransfer" => "feComponentTransfer",
        "fecomposite" => "feComposite",
        "feconvolvematrix" => "feConvolveMatrix",
        "fediffuselighting" => "feDiffuseLighting",
        "fedisplacementmap" => "feDisplacementMap",
        "fedistantlight" => "feDistantLight",
        "fedropshadow" => "feDropShadow",
        "feflood" => "feFlood",
        "fefunca" => "feFuncA",
        "fefuncb" => "feFuncB",
        "fefuncg" => "feFuncG",
        "fefuncr" => "feFuncR",
        "fegaussianblur" => "feGaussianBlur",
        "feimage" => "feImage",
        "femerge" => "feMerge",
        "femergenode" => "feMergeNode",
        "femorphology" => "feMorphology",
        "feoffset" => "feOffset",
        "fepointlight" => "fePointLight",
        "fespecularlighting" => "feSpecularLighting",
        "fespotlight" => "feSpotLight",
        "fetile" => "feTile",
        "feturbulence" => "feTurbulence",
        "foreignobject" => "foreignObject",
        "glyphref" => "glyphRef",
        "lineargradient" => "linearGradient",
        "radialgradient" => "radialGradient",
        "textpath" => "textPath",
        _ => name,
    }
}

fn adjusted_svg_attributes(attributes: Attributes) -> Attributes {
    let mut adjusted = Attributes::default();
    for (name, value) in attributes.map {
        let raw = name.local.as_ref();
        let corrected = match raw.to_ascii_lowercase().as_str() {
            "attributename" => "attributeName",
            "attributetype" => "attributeType",
            "basefrequency" => "baseFrequency",
            "baseprofile" => "baseProfile",
            "calcmode" => "calcMode",
            "clippathunits" => "clipPathUnits",
            "diffuseconstant" => "diffuseConstant",
            "edgemode" => "edgeMode",
            "filterunits" => "filterUnits",
            "glyphref" => "glyphRef",
            "gradienttransform" => "gradientTransform",
            "gradientunits" => "gradientUnits",
            "kernelmatrix" => "kernelMatrix",
            "kernelunitlength" => "kernelUnitLength",
            "keypoints" => "keyPoints",
            "keysplines" => "keySplines",
            "keytimes" => "keyTimes",
            "lengthadjust" => "lengthAdjust",
            "limitingconeangle" => "limitingConeAngle",
            "markerheight" => "markerHeight",
            "markerunits" => "markerUnits",
            "markerwidth" => "markerWidth",
            "maskcontentunits" => "maskContentUnits",
            "maskunits" => "maskUnits",
            "numoctaves" => "numOctaves",
            "pathlength" => "pathLength",
            "patterncontentunits" => "patternContentUnits",
            "patterntransform" => "patternTransform",
            "patternunits" => "patternUnits",
            "pointsatx" => "pointsAtX",
            "pointsaty" => "pointsAtY",
            "pointsatz" => "pointsAtZ",
            "preservealpha" => "preserveAlpha",
            "preserveaspectratio" => "preserveAspectRatio",
            "primitiveunits" => "primitiveUnits",
            "refx" => "refX",
            "refy" => "refY",
            "repeatcount" => "repeatCount",
            "repeatdur" => "repeatDur",
            "requiredextensions" => "requiredExtensions",
            "requiredfeatures" => "requiredFeatures",
            "specularconstant" => "specularConstant",
            "specularexponent" => "specularExponent",
            "spreadmethod" => "spreadMethod",
            "startoffset" => "startOffset",
            "stddeviation" => "stdDeviation",
            "stitchtiles" => "stitchTiles",
            "surfacescale" => "surfaceScale",
            "systemlanguage" => "systemLanguage",
            "tablevalues" => "tableValues",
            "targetx" => "targetX",
            "targety" => "targetY",
            "textlength" => "textLength",
            "viewbox" => "viewBox",
            "viewtarget" => "viewTarget",
            "xchannelselector" => "xChannelSelector",
            "ychannelselector" => "yChannelSelector",
            "zoomandpan" => "zoomAndPan",
            _ => raw,
        };
        adjusted.insert_first(corrected.to_string(), value.value);
    }
    adjusted
}

fn consume_attribute_value(input: &str, cursor: &mut usize) -> String {
    if *cursor >= input.len() {
        return String::new();
    }
    let quote = input.as_bytes()[*cursor];
    if matches!(quote, b'\'' | b'"') {
        *cursor += 1;
        let start = *cursor;
        while *cursor < input.len() && input.as_bytes()[*cursor] != quote {
            *cursor += 1;
        }
        let value = input[start..*cursor].to_string();
        if *cursor < input.len() {
            *cursor += 1;
        }
        value
    } else {
        let start = *cursor;
        while *cursor < input.len()
            && !input.as_bytes()[*cursor].is_ascii_whitespace()
            && input.as_bytes()[*cursor] != b'>'
        {
            *cursor += 1;
        }
        input[start..*cursor].to_string()
    }
}

fn skip_ascii_whitespace(input: &str, cursor: &mut usize) {
    while *cursor < input.len() && input.as_bytes()[*cursor].is_ascii_whitespace() {
        *cursor += 1;
    }
}

fn is_html_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic()
}

fn is_html_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.')
}

fn starts_ascii_case_insensitive(source: &str, prefix: &str) -> bool {
    source
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
}

fn find_markup_end(source: &str, mut offset: usize) -> Option<usize> {
    let mut quote = None;
    while offset < source.len() {
        let ch = source.as_bytes()[offset];
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
        } else if matches!(ch, b'\'' | b'"') {
            quote = Some(ch);
        } else if ch == b'>' {
            return Some(offset);
        }
        offset += 1;
    }
    None
}

fn find_raw_end_tag(source: &str, tag: &str) -> Option<usize> {
    let needle = format!("</{}", tag);
    let lower_source = source.to_ascii_lowercase();
    let lower_needle = needle.to_ascii_lowercase();
    let mut search = 0usize;
    while let Some(relative) = lower_source[search..].find(&lower_needle) {
        let offset = search + relative;
        let after = offset + lower_needle.len();
        if lower_source
            .as_bytes()
            .get(after)
            .is_none_or(|byte| byte.is_ascii_whitespace() || *byte == b'>')
        {
            return Some(offset);
        }
        search = after;
    }
    None
}

fn replace_nulls(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\0' => output.push('\u{fffd}'),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                output.push('\n');
            }
            _ => output.push(ch),
        }
    }
    output
}

fn decode_character_references(source: &str) -> String {
    decode_character_references_with_context(source, false)
}

fn decode_character_references_in_attribute(source: &str) -> String {
    decode_character_references_with_context(source, true)
}

fn decode_character_references_with_context(source: &str, in_attribute: bool) -> String {
    let mut output = String::with_capacity(source.len());
    let mut offset = 0usize;
    while offset < source.len() {
        let Some(relative_amp) = source[offset..].find('&') else {
            output.push_str(&replace_nulls(&source[offset..]));
            break;
        };
        let amp = offset + relative_amp;
        output.push_str(&replace_nulls(&source[offset..amp]));
        if let Some((value, consumed)) =
            decode_character_reference(&source[amp + 1..], in_attribute)
        {
            output.push_str(value.as_ref());
            offset = amp + 1 + consumed;
        } else {
            output.push('&');
            offset = amp + 1;
        }
    }
    output
}

fn decode_character_reference(
    source: &str,
    in_attribute: bool,
) -> Option<(std::borrow::Cow<'static, str>, usize)> {
    use std::borrow::Cow;
    if let Some(rest) = source.strip_prefix('#') {
        let (radix, digits_start) = if rest.starts_with('x') || rest.starts_with('X') {
            (16, 1)
        } else {
            (10, 0)
        };
        let digits = &rest[digits_start..];
        let digit_count = digits
            .bytes()
            .take_while(|byte| {
                if radix == 16 {
                    byte.is_ascii_hexdigit()
                } else {
                    byte.is_ascii_digit()
                }
            })
            .count();
        if digit_count == 0 {
            return None;
        }
        let raw = u32::from_str_radix(&digits[..digit_count], radix).ok()?;
        let consumed = 1
            + digits_start
            + digit_count
            + usize::from(digits.as_bytes().get(digit_count) == Some(&b';'));
        let scalar = sanitize_numeric_reference(raw);
        return Some((Cow::Owned(scalar.to_string()), consumed));
    }

    let alphanumeric_len = source
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric())
        .count();
    let candidate_len =
        alphanumeric_len + usize::from(source.as_bytes().get(alphanumeric_len) == Some(&b';'));
    for length in (1..=candidate_len).rev() {
        let name = &source[..length];
        let entity = NAMED_ENTITY_TABLES.iter().find_map(|table| {
            table
                .binary_search_by(|candidate| candidate.0.cmp(name))
                .ok()
                .map(|index| table[index])
        });
        if let Some((_, first, second)) = entity {
            if in_attribute
                && !name.ends_with(';')
                && source
                    .as_bytes()
                    .get(length)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'=')
            {
                return None;
            }
            let mut value = String::with_capacity(8);
            value.push(char::from_u32(first).unwrap_or('\u{fffd}'));
            if second != 0 {
                value.push(char::from_u32(second).unwrap_or('\u{fffd}'));
            }
            return Some((Cow::Owned(value), length));
        }
    }

    None
}

fn sanitize_numeric_reference(value: u32) -> char {
    let value = match value {
        0x80 => 0x20ac,
        0x82 => 0x201a,
        0x83 => 0x0192,
        0x84 => 0x201e,
        0x85 => 0x2026,
        0x86 => 0x2020,
        0x87 => 0x2021,
        0x88 => 0x02c6,
        0x89 => 0x2030,
        0x8a => 0x0160,
        0x8b => 0x2039,
        0x8c => 0x0152,
        0x8e => 0x017d,
        0x91 => 0x2018,
        0x92 => 0x2019,
        0x93 => 0x201c,
        0x94 => 0x201d,
        0x95 => 0x2022,
        0x96 => 0x2013,
        0x97 => 0x2014,
        0x98 => 0x02dc,
        0x99 => 0x2122,
        0x9a => 0x0161,
        0x9b => 0x203a,
        0x9c => 0x0153,
        0x9e => 0x017e,
        0x9f => 0x0178,
        other => other,
    };
    if value == 0 || value > 0x10ffff || (0xd800..=0xdfff).contains(&value) {
        '\u{fffd}'
    } else {
        char::from_u32(value).unwrap_or('\u{fffd}')
    }
}

#[cfg(test)]
mod tests {
    use super::{NodeData, parse_html};

    fn tag(node: &super::NodeRef) -> Option<&str> {
        node.as_element().map(|element| element.name.local.as_ref())
    }

    #[test]
    fn fragments_gain_implied_document_structure_and_head_routing() {
        let document = parse_html("<title>A &amp; B</title><main id=x>Hello</main>");
        assert_eq!(document.select("html > head > title").unwrap().count(), 1);
        assert_eq!(document.select("html > body > main#x").unwrap().count(), 1);
        assert_eq!(
            document.select_first("title").unwrap().text_contents(),
            "A & B"
        );
    }

    #[test]
    fn structural_recovery_closes_paragraphs_lists_and_formatting() {
        let document = parse_html("<p>one<div>two</div>three<ul><li>a<li>b</ul><b><i>x</b>y</i>");
        let body = document.select_first("body").unwrap();
        let element_tags: Vec<String> = body
            .as_node()
            .children()
            .filter_map(|node| tag(&node).map(str::to_string))
            .collect();
        assert_eq!(element_tags[..2], ["p", "div"]);
        assert_eq!(document.select("li").unwrap().count(), 2);
        assert_eq!(document.select("b > i").unwrap().count(), 1);
        assert_eq!(document.select("body > i").unwrap().count(), 1);
    }

    #[test]
    fn table_recovery_inserts_sections_rows_and_fosters_text() {
        let document = parse_html("<table>before<tr><td>A<td>B</table>after");
        assert_eq!(
            document.select("table > tbody > tr > td").unwrap().count(),
            2
        );
        let body = document.select_first("body").unwrap();
        assert_eq!(body.text_contents(), "beforeABafter");
        let first = body.as_node().children().next().expect("fostered text");
        assert!(matches!(first.data(), NodeData::Text(_)));
    }

    #[test]
    fn html5_edge_recovery_preserves_void_table_select_and_frameset_rules() {
        let self_closing = parse_html("<div/>x<svg><g/>y</svg>");
        assert_eq!(
            self_closing.select_first("div").unwrap().text_contents(),
            "xy"
        );
        assert_eq!(
            self_closing
                .select_first("svg > g")
                .unwrap()
                .text_contents(),
            ""
        );

        let hidden_input = parse_html("<table><input type=HiDdEn value=x><tr><td>y</table>");
        assert_eq!(hidden_input.select("table > input").unwrap().count(), 1);
        assert_eq!(
            hidden_input
                .select_first("table > input")
                .unwrap()
                .attributes
                .borrow()
                .get("value"),
            Some("x")
        );

        let orphan = parse_html("<td>x</td><p>y");
        assert_eq!(orphan.select("td").unwrap().count(), 0);
        assert_eq!(orphan.select_first("body").unwrap().text_contents(), "xy");

        let captions = parse_html("<table><caption>a<caption>b</table>");
        assert_eq!(captions.select("table > caption").unwrap().count(), 2);

        let nested_tables = parse_html("<table><tbody><table><tr><td>x</table>");
        assert_eq!(nested_tables.select("body > table").unwrap().count(), 2);

        let select = parse_html("<select><option>a<hr><option>b</select>");
        assert_eq!(select.select("select > option").unwrap().count(), 2);
        assert_eq!(select.select("select > hr").unwrap().count(), 1);

        let framesets = parse_html("<frameset><frameset><frame></frameset></frameset>");
        assert_eq!(
            framesets
                .select("html > frameset > frameset > frame")
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn raw_text_rcdata_entities_and_numeric_replacements_are_deterministic() {
        let document = parse_html(
            "<style>a<b{c:d}</style><textarea>&lt;x&gt;</textarea><p>&#x80; &copy; &#0;</p>",
        );
        assert_eq!(
            document.select_first("style").unwrap().text_contents(),
            "a<b{c:d}"
        );
        assert_eq!(
            document.select_first("textarea").unwrap().text_contents(),
            "<x>"
        );
        assert_eq!(document.select_first("p").unwrap().text_contents(), "€ © �");
    }

    #[test]
    fn complete_named_entity_table_preserves_pairs_and_attribute_ambiguity() {
        let document = parse_html(
            "<p title='&notit; &not= &not; &CounterClockwiseContourIntegral;'>&notit; &NotEqualTilde;</p>",
        );
        let paragraph = document.select_first("p").unwrap();
        assert_eq!(
            paragraph.attributes.borrow().get("title"),
            Some("&notit; &not= ¬ ∳")
        );
        assert_eq!(paragraph.text_contents(), "¬it; ≂̸");
    }

    #[test]
    fn numeric_controls_and_raw_text_states_match_html_tokenization() {
        let references = parse_html("<p>&#13;&#129;&#xD800;&#x110000;</p>");
        assert_eq!(
            references.select_first("p").unwrap().text_contents(),
            "\r\u{81}\u{fffd}\u{fffd}"
        );

        let plaintext = parse_html("<plaintext>a</plaintext><p>b");
        assert_eq!(plaintext.select("p").unwrap().count(), 0);
        assert_eq!(
            plaintext.select_first("plaintext").unwrap().text_contents(),
            "a</plaintext><p>b"
        );

        let noscript = parse_html("<body><noscript><p>fallback</p></noscript><p>live</p>");
        assert_eq!(noscript.select("p").unwrap().count(), 1);
        assert_eq!(
            noscript.select_first("noscript").unwrap().text_contents(),
            "<p>fallback</p>"
        );
    }

    #[test]
    fn foreign_content_restores_svg_names_and_attribute_case() {
        let document = parse_html(
            "<svg viewbox='0 0 4 4'><lineargradient id='g'/><foreignobject><DIV>x</DIV></foreignobject></svg>",
        );
        let svg = document.select_first("svg").unwrap();
        assert!(svg.attributes.borrow().get("viewBox").is_some());
        assert_eq!(tag(svg.as_node()), Some("svg"));
        assert_eq!(
            tag(document.select_first("linearGradient").unwrap().as_node()),
            Some("linearGradient")
        );
        assert_eq!(
            tag(document
                .select_first("foreignObject > div")
                .unwrap()
                .as_node()),
            Some("div")
        );
    }

    #[test]
    fn internal_selector_subset_preserves_document_order_and_combinators() {
        let document = parse_html(
            "<main><p id='a' class='x y' data-k='one two'></p><span></span><p class='x'></p></main>",
        );
        assert_eq!(document.select("p.x, #a").unwrap().count(), 2);
        assert_eq!(
            document.select("main > p[data-k~='two']").unwrap().count(),
            1
        );
        assert_eq!(document.select("span + p").unwrap().count(), 1);
        assert_eq!(document.select("#a ~ p").unwrap().count(), 1);
    }

    fn native_canonical(node: &super::NodeRef, output: &mut String) {
        match node.data() {
            NodeData::Document | NodeData::Doctype(_) | NodeData::Comment(_) => {}
            NodeData::Text(text) => {
                output.push_str("T{");
                output.push_str(&text.borrow());
                output.push('}');
            }
            NodeData::Element(element) => {
                output.push_str("E{");
                output.push_str(element.name.local.as_ref());
                for (name, value) in &element.attributes.borrow().map {
                    output.push('|');
                    output.push_str(name.local.as_ref());
                    output.push('=');
                    output.push_str(&value.value);
                }
                output.push('}');
            }
        }
        for child in node.children() {
            native_canonical(&child, output);
        }
        if let NodeData::Element(element) = node.data() {
            output.push_str("X{");
            output.push_str(element.name.local.as_ref());
            output.push('}');
        }
    }

    #[test]
    fn recovery_matrix_matches_frozen_html5_tree_contract() {
        let cases = [
            "<!doctype html><html lang=en><head><title>x</title></head><body><main><p>A</p></main></body></html>",
            "<title>A &amp; B</title><main id=x>Hello</main>",
            "<p>one<div>two</div>three",
            "<ul><li>a<li>b</ul><dl><dt>x<dd>y<dt>z</dl>",
            "<h1>a<h2>b</h1>c",
            "<b><i>x</b>y</i>",
            "<table><tr><td>A<td>B</table>",
            "<table>before<tr><td>A</table>after",
            "<table><div>x</div><tr><td>y</table>",
            "<style>a<b{c:d}</style><script>if(a<b){x()}</script>",
            "<textarea>&lt;x&gt;</textarea><p>&#x80; &copy; &#0;</p>",
            "<input disabled disabled=x><br/>tail",
            "<svg viewbox='0 0 4 4'><lineargradient id=g/><text>A<tspan>B</tspan>C</text></svg>",
            "<svg><foreignobject><DIV class=X>x</DIV></foreignobject></svg>",
            "<p><p>x</p></p>",
            "<button>a<button>b</button>c",
            "<a href=x>a<a href=y>b</a>c",
            "<nobr>a<nobr>b</nobr>c",
            "<form><div><form>x</form>y</div>",
            "<select><option>a<option>b<optgroup label=x><option>c</select>",
            "<table><caption>c</caption><col><tr><th>h<td>d</table>",
            "<table><tbody><tr><td>A<table><tr><td>B</table>C</table>",
            "<table><tr><td>A</tr>B</table>",
            "<table> \n<tr><td>A</table>",
            "<table><input value=x>y<tr><td>z</table>",
            "<p>a</br>b",
            "<image src=x><p>y",
            "<pre>\nabc</pre><listing>\ndef</listing><textarea>\nghi</textarea>",
            "<script><!-- if(a<b) x() --></script><plaintext>a<b>c",
            "<ruby>a<rt>x<rp>(<rt>y</ruby>",
            "<svg><desc><p>x</p></desc><foreignobject><section>y</section></foreignobject></svg>",
            "<math><mi><b>x</b></mi><annotation-xml encoding='text/html'><p>y</p></annotation-xml></math>",
            "<html><head></head><meta charset=utf-8><body><title>x</title>y",
            "<table><style>.x{color:red}</style><tr><td>x</table>",
            "<p><b>1<i>2</b>3</i>4",
            "<b><p>one</b>two<p>three",
            "<a href=x><b>one<a href=y>two</a>three</b>",
            "<html a=1><html b=2><head c=3><head d=4><body e=5><body f=6>x",
            "<head><meta name=x></head><body><link rel=stylesheet href=x><title>late</title>x",
            "<frameset cols='*'><frame src=x><noframes><p>fallback</p></noframes></frameset>",
            "<template><table><tr><td>x</table><p>y</template>z",
            "<table><form id=f><tr><td>x</form>y</table>",
            "<select><div>x</div><option>a<table><tr><td>b</select>c",
            "<table><select><option>a</select><tr><td>b</table>",
            "<table><colgroup><col><colgroup><col><tbody><tr><td>x</table>",
            "<table><thead><tr><th>x<tbody><tr><td>y<tfoot><tr><td>z</table>",
            "<svg><g><p>x</p></g><circle/></svg>",
            "<svg><![CDATA[x<y & z]]><text>a</text></svg>",
            "<math><annotation-xml><svg><circle/></svg></annotation-xml></math>",
            "<p title='a>b' data-x=a/b/>x",
            "<div>a</span>b</unknown>c</div>",
            "<table><!--c--><tr><!--d--><td>x</table>",
        ];
        let expected = [
            0x0e693a514fac3050,
            0xcc19bb1463576889,
            0xed115cd07a22ec4a,
            0x6ffd26a71a69d611,
            0xa57f14e6fd7620c4,
            0xbf2edd5bdfc4e35e,
            0xa662a4f041d7becc,
            0xaa93cea1e04566d0,
            0xb1a9c2c8ac2b94a4,
            0x9507bf0ed830145a,
            0xbbd9ade9bfc2305c,
            0x80180f2984264485,
            0x7bb71da9d1dfcdb9,
            0x6a5a55573c0ab9f6,
            0xe3f794d38d406465,
            0x5f52dc89d04eb37e,
            0x38b7a0933179a005,
            0xa487a6fd7f73a92a,
            0xa40b80af5e9b3c67,
            0x4050add853814c72,
            0x1b0c306f03a88cab,
            0x044e8bde617a9f80,
            0xd45544e9fb521927,
            0x34a49871f927b951,
            0xfaa1465bbb84af46,
            0xf73d8090604301e1,
            0xff4b476bff9213ac,
            0x58ce4f580a21bc22,
            0x90f48524a20c79e7,
            0xb9ffd74a8d3966b6,
            0xf1c7a40e9a999a6a,
            0xaf680e047e2d4619,
            0x70b937dc6258f664,
            0xb6b2e95d2a5cefaf,
            0x79979281bd07bb5a,
            0xc0825828795cfe76,
            0x4c557fa412481201,
            0xab1e76d8647cf35b,
            0x2ebbe89e3ef71129,
            0x495d397993a4b00c,
            0x31b25d16b66f8583,
            0xca3f02f59f6395ee,
            0xc8b60804b8e24a74,
            0x48f532914d5f845f,
            0x8a23d88a0735d4de,
            0xe7add7a40817e629,
            0xbdae80dc18b5bd34,
            0xd4e4941a1d0acc0e,
            0x8ede266833ecb78e,
            0x392049b9f866c6f4,
            0x996a2e311c810845,
            0x4de9ee93167f1e7e,
        ];
        assert_eq!(cases.len(), expected.len());
        for (source, expected_fingerprint) in cases.into_iter().zip(expected) {
            let native = parse_html(source);
            let mut native_tree = String::new();
            native_canonical(&native, &mut native_tree);
            let fingerprint = native_tree
                .bytes()
                .fold(0xcbf29ce484222325u64, |hash, byte| {
                    (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
                });
            assert_eq!(fingerprint, expected_fingerprint, "source: {source}");
        }
    }
}
