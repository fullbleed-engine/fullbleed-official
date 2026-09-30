//! Source-owned table identity and explicit header resolution.
//! Numeric keys are deterministic document preorder, never DOM addresses,
//! process-global layout counters, authored IDs or a VDP record-count ceiling.
use crate::html_dom::{NodeData, NodeRef};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSemanticNode {
    /// Document-local logical table, shared by all its page fragments.
    pub table_key: u64,
    /// None on a table container; stable across repeated cell appearances.
    pub cell_key: Option<u64>,
    pub row_key: Option<u64>,
    pub group_key: Option<u64>,
    /// Last source row covered by the compiler-resolved span, before pagination.
    pub row_span_end: Option<u64>,
    /// None means no explicit association; Some([]) is explicitly empty.
    pub header_cells: Option<Vec<u64>>,
    /// Stable, non-content diagnostic codes. Authored IDs are not exposed here.
    pub header_issues: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct HtmlTableIndex {
    by_node: HashMap<usize, Arc<TableSemanticNode>>,
}

struct SourceCell {
    address: usize,
    table: u64,
    row: Option<u64>,
    group: Option<u64>,
    header: bool,
    empty: bool,
    headers: Option<String>,
}

/// Final emitted identities, shared by PDF and nonvisual reading projections.
/// One catalog represents one document instance, not an entire recipient batch.
#[derive(Default)]
pub(crate) struct CompiledTableCatalog {
    headers: BTreeMap<(u64, u64), usize>,
    rows: BTreeMap<(u64, Option<u64>), Vec<u64>>,
}

impl CompiledTableCatalog {
    pub(crate) fn observe(&mut self, role: &str, node: &TableSemanticNode) {
        if role == "TH" {
            if let Some(cell) = node.cell_key {
                *self.headers.entry((node.table_key, cell)).or_default() += 1;
            }
        }
        if role == "TR" {
            if let Some(row) = node.row_key {
                self.rows
                    .entry((node.table_key, node.group_key))
                    .or_default()
                    .push(row);
            }
        }
    }

    pub(crate) fn finish(&mut self) {
        for rows in self.rows.values_mut() {
            rows.sort_unstable();
            rows.dedup();
        }
    }

    pub(crate) fn from_document(document: &crate::Document) -> Self {
        let mut catalog = Self::default();
        for_each_emitted_table_node(document, |_, _, role, node| catalog.observe(role, node));
        catalog.finish();
        catalog
    }

    pub(crate) fn unique_header(&self, table: u64, cell: u64) -> bool {
        self.headers.get(&(table, cell)) == Some(&1)
    }

    pub(crate) fn resolve(&self, source: &TableSemanticNode) -> TableSemanticNode {
        let mut resolved = source.clone();
        if let Some(headers) = &mut resolved.header_cells {
            headers.retain(|cell| {
                if self.unique_header(source.table_key, *cell) {
                    return true;
                }
                resolved.header_issues.push(
                    if self.headers.contains_key(&(source.table_key, *cell)) {
                        "ambiguous_compiled_header_target"
                    } else {
                        "unemitted_header_target"
                    }
                    .into(),
                );
                false
            });
        }
        resolved.header_issues.sort();
        resolved.header_issues.dedup();
        resolved
    }

    pub(crate) fn logical_row_span(&self, node: &TableSemanticNode) -> Option<u32> {
        let (start, end) = (node.row_key?, node.row_span_end?);
        if end < start {
            return None;
        }
        let rows = self.rows.get(&(node.table_key, node.group_key))?;
        let count =
            rows.partition_point(|row| *row <= end) - rows.partition_point(|row| *row < start);
        u32::try_from(count.max(1)).ok()
    }
}

pub(crate) fn for_each_emitted_table_node(
    document: &crate::Document,
    mut visit: impl FnMut(usize, usize, &str, &TableSemanticNode),
) {
    for (page, commands) in document.pages.iter().enumerate() {
        let mut marked = Vec::new();
        let mut artifact_depth = 0usize;
        for (command, node) in commands.commands.iter().enumerate() {
            match node {
                crate::Command::BeginArtifact { .. } => {
                    marked.push(true);
                    artifact_depth += 1;
                }
                crate::Command::BeginOptionalContent { .. } => marked.push(false),
                crate::Command::EndMarkedContent => {
                    if marked.pop() == Some(true) {
                        artifact_depth = artifact_depth.saturating_sub(1);
                    }
                }
                crate::Command::BeginTag {
                    role,
                    table_semantics: Some(semantics),
                    ..
                } if artifact_depth == 0 => visit(page, command, role, semantics),
                _ => {}
            }
        }
    }
}

impl HtmlTableIndex {
    pub(crate) fn build(document: &NodeRef) -> Self {
        let mut index = Self::default();
        let mut tables = HashMap::<usize, u64>::new();
        let mut groups = HashMap::<usize, u64>::new();
        let mut rows = HashMap::<usize, u64>::new();
        let mut ids = BTreeMap::<String, Vec<u64>>::new();
        let mut cells = BTreeMap::<u64, SourceCell>::new();
        for (ordinal, node) in document.descendants().enumerate() {
            let key = ordinal as u64 + 1;
            let address = node.identity_key();
            let inherited = node
                .parent()
                .and_then(|parent| tables.get(&parent.identity_key()).copied());
            let element = node.as_element();
            let table = if element.is_some_and(|element| element.name.local.as_ref() == "table") {
                index.by_node.insert(
                    address,
                    Arc::new(TableSemanticNode {
                        table_key: key,
                        cell_key: None,
                        row_key: None,
                        group_key: None,
                        row_span_end: None,
                        header_cells: None,
                        header_issues: Vec::new(),
                    }),
                );
                Some(key)
            } else {
                inherited
            };
            if let Some(table) = table {
                tables.insert(address, table);
            }
            let Some(element) = element else {
                continue;
            };
            let attributes = element.attributes.borrow();
            if let Some(id) = attributes.get("id") {
                ids.entry(id.to_string()).or_default().push(key);
            }
            let tag = element.name.local.as_ref();
            let parent = node.parent();
            let group = if matches!(tag, "thead" | "tbody" | "tfoot") {
                Some(key)
            } else {
                parent
                    .as_ref()
                    .and_then(|parent| groups.get(&parent.identity_key()).copied())
            };
            let row = if tag == "tr" {
                Some(key)
            } else {
                parent
                    .as_ref()
                    .and_then(|parent| rows.get(&parent.identity_key()).copied())
            };
            if tag != "table" {
                if let Some(group) = group {
                    groups.insert(address, group);
                }
                if let Some(row) = row {
                    rows.insert(address, row);
                }
            }
            if matches!(tag, "tr" | "thead" | "tbody" | "tfoot") {
                if let Some(table_key) = table {
                    index.by_node.insert(
                        address,
                        Arc::new(TableSemanticNode {
                            table_key,
                            cell_key: None,
                            row_key: row,
                            group_key: group,
                            row_span_end: None,
                            header_cells: None,
                            header_issues: Vec::new(),
                        }),
                    );
                }
            }
            if matches!(tag, "th" | "td") {
                if let Some(table) = table {
                    let empty = node.children().all(|child| match child.data() {
                        NodeData::Element(_) => false,
                        NodeData::Text(text) => text
                            .borrow()
                            .chars()
                            .all(|character| character.is_ascii_whitespace()),
                        _ => true,
                    });
                    cells.insert(
                        key,
                        SourceCell {
                            address,
                            table,
                            row,
                            group,
                            header: tag == "th",
                            empty,
                            headers: attributes.get("headers").map(str::to_string),
                        },
                    );
                }
            }
        }
        let mut resolved = BTreeMap::new();
        for (&key, cell) in &cells {
            let mut issues = BTreeSet::new();
            let headers = cell.headers.as_ref().map(|source| {
                let mut seen = BTreeSet::new();
                let mut targets = Vec::new();
                for label in source.split_ascii_whitespace() {
                    let Some(matches) = ids.get(label) else {
                        issues.insert("missing_header_target");
                        continue;
                    };
                    // Duplicate IDs are invalid source, not permission to link a
                    // repeated recipient to the first recipient's header.
                    if matches.len() != 1 {
                        issues.insert("duplicate_header_id");
                        continue;
                    }
                    let target_key = matches[0];
                    if target_key == key {
                        issues.insert("self_header_reference");
                        continue;
                    }
                    let Some(target) = cells.get(&target_key).filter(|target| target.header) else {
                        issues.insert("non_header_target");
                        continue;
                    };
                    if target.table != cell.table {
                        issues.insert("foreign_table_header");
                        continue;
                    }
                    if !target.empty && seen.insert(target_key) {
                        targets.push(target_key);
                    }
                }
                targets
            });
            resolved.insert(
                key,
                TableSemanticNode {
                    table_key: cell.table,
                    cell_key: Some(key),
                    row_key: cell.row,
                    group_key: cell.group,
                    row_span_end: None,
                    header_cells: headers,
                    header_issues: issues.into_iter().map(str::to_string).collect(),
                },
            );
        }
        remove_header_cycles(&mut resolved);
        for (key, value) in resolved {
            index.by_node.insert(cells[&key].address, Arc::new(value));
        }
        index
    }

    pub(crate) fn node(&self, node: &NodeRef) -> Option<Arc<TableSemanticNode>> {
        self.by_node.get(&node.identity_key()).cloned()
    }
}

/// Iterative strongly-connected components: no recursion or quadratic path
/// expansion for long or heavily linked authored header graphs.
fn remove_header_cycles(nodes: &mut BTreeMap<u64, TableSemanticNode>) {
    let mut visited = BTreeSet::new();
    let mut order = Vec::new();
    for &root in nodes.keys() {
        let mut pending = vec![(root, false)];
        while let Some((key, exiting)) = pending.pop() {
            if exiting {
                order.push(key);
                continue;
            }
            if !visited.insert(key) {
                continue;
            }
            pending.push((key, true));
            for &target in nodes[&key]
                .header_cells
                .as_deref()
                .unwrap_or_default()
                .iter()
                .rev()
            {
                pending.push((target, false));
            }
        }
    }
    let mut reverse = BTreeMap::<u64, Vec<u64>>::new();
    for (&key, node) in nodes.iter() {
        for &target in node.header_cells.as_deref().unwrap_or_default() {
            reverse.entry(target).or_default().push(key);
        }
    }
    let mut components = BTreeMap::new();
    let mut cyclic = BTreeSet::new();
    for root in order.into_iter().rev() {
        if components.contains_key(&root) {
            continue;
        }
        let mut members = Vec::new();
        let mut pending = vec![root];
        while let Some(key) = pending.pop() {
            if components.contains_key(&key) {
                continue;
            }
            components.insert(key, root);
            members.push(key);
            pending.extend(reverse.get(&key).into_iter().flatten().copied());
        }
        if members.len() > 1 {
            cyclic.extend(members);
        }
    }
    for key in cyclic {
        let node = nodes
            .get_mut(&key)
            .expect("component member is a source cell");
        node.header_issues.push("cyclic_header_reference".into());
        if let Some(targets) = &mut node.header_cells {
            targets.retain(|target| components[target] != components[&key]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HtmlTableIndex, TableSemanticNode};
    use crate::html_dom::parse_html;

    fn cells(html: &str) -> Vec<TableSemanticNode> {
        let document = parse_html(html);
        let index = HtmlTableIndex::build(&document);
        document
            .descendants()
            .filter_map(|node| index.node(&node))
            .filter(|node| node.cell_key.is_some())
            .map(|node| (*node).clone())
            .collect()
    }

    #[test]
    fn resolved_ids_use_decoded_engine_attributes_and_ascii_token_boundaries() {
        let html = "<table><tr><th id='a&amp;b'>A</th><th id='\u{00a0}'>B</th><td headers='a&amp;b \u{00a0} a&amp;b'>Value</td><td headers>Empty associations</td><td>Implicit</td></tr></table>";
        let found = cells(html);
        assert_eq!(found, cells(html));
        assert_eq!(
            found[2].header_cells,
            Some(vec![found[0].cell_key.unwrap(), found[1].cell_key.unwrap()])
        );
        assert_eq!(found[3].header_cells, Some(vec![]));
        assert_eq!(found[4].header_cells, None);
        assert!(found.iter().all(|node| node.header_issues.is_empty()));
    }

    #[test]
    fn invalid_references_never_cross_tables_or_ambiguous_recipient_ids() {
        let found = cells(
            "<p id='shadow'>Outside</p><table><tr><th id='same'>A</th><td id='data'>Data</td><td headers='missing same shadow data outer'>Value</td></tr></table><table><tr><th id='same'>B</th><th id='outer'>Other table</th></tr></table>",
        );
        assert_eq!(found[2].header_cells, Some(vec![]));
        assert_eq!(
            found[2].header_issues,
            vec![
                "duplicate_header_id",
                "foreign_table_header",
                "missing_header_target",
                "non_header_target"
            ]
        );
    }

    #[test]
    fn cycles_are_removed_without_erasing_valid_external_edges() {
        let found = cells(
            "<table><tr><th id='a' headers='b c'>A</th><th id='b' headers='a'>B</th><th id='c'>C</th><th id='self' headers='self'>Self</th><td headers='a c'>Value</td></tr></table>",
        );
        assert_eq!(
            found[0].header_cells,
            Some(vec![found[2].cell_key.unwrap()])
        );
        assert_eq!(found[1].header_cells, Some(vec![]));
        assert_eq!(found[0].header_issues, vec!["cyclic_header_reference"]);
        assert!(found[2].header_issues.is_empty());
        assert_eq!(found[3].header_issues, vec!["self_header_reference"]);
        assert_eq!(
            found[4].header_cells,
            Some(vec![found[0].cell_key.unwrap(), found[2].cell_key.unwrap()])
        );
    }

    #[test]
    fn empty_headers_are_omitted_but_element_and_non_ascii_contents_are_not() {
        let found = cells(
            "<table><tr><th id='empty'> \n<!-- comment --></th><th id='element'><span></span></th><th id='space'>\u{2003}</th><td headers='empty element space'>Value</td></tr></table>",
        );
        assert_eq!(
            found[3].header_cells,
            Some(vec![found[1].cell_key.unwrap(), found[2].cell_key.unwrap()])
        );
    }
}
