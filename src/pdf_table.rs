//! Logical table continuation and PDF structure identities. No pixel changes.
use super::TagRecord;
use crate::table_semantics::CompiledTableCatalog;
use std::collections::BTreeMap;
use std::sync::Arc;

pub(super) fn normalize(records: Vec<TagRecord>) -> Vec<TagRecord> {
    // A logical cell can own content on several pages. Keep its one TH/TD and
    // attach each fragment's content in order, rather than adding extra columns
    // to the merged row. Plain cells need a leaf Span to retain each page's MCID.
    let cell_key = |record: &TagRecord| {
        let node = record.table_semantics.as_ref()?;
        matches!(record.role.as_str(), "TH" | "TD").then_some((
            record.table_document,
            node.table_key,
            record.role.clone(),
            node.cell_key?,
            node.row_key,
            node.group_key,
        ))
    };
    let mut cell_counts = BTreeMap::new();
    for record in &records {
        if let Some(key) = cell_key(record) {
            *cell_counts.entry(key).or_insert(0usize) += 1;
        }
    }
    let mut cells = BTreeMap::<_, usize>::new();
    let mut mapped = Vec::<usize>::with_capacity(records.len());
    let mut containers = BTreeMap::<_, usize>::new();
    let mut normalized = Vec::<TagRecord>::with_capacity(records.len());
    for mut record in records {
        record.parent = record.parent.and_then(|parent| mapped.get(parent).copied());
        if let Some(key) = cell_key(&record).filter(|key| cell_counts[key] > 1) {
            let leaf = record.mcid.map(|_| record.clone());
            let existing = if let Some(&existing) = cells.get(&key) {
                normalized[existing]
                    .destination_sources
                    .extend(record.destination_sources);
                existing
            } else {
                let existing = normalized.len();
                record.mcid = None;
                normalized.push(record);
                cells.insert(key, existing);
                existing
            };
            mapped.push(existing);
            if let Some(mut leaf) = leaf {
                leaf.role = "Span".into();
                leaf.parent = Some(existing);
                leaf.scope = None;
                leaf.column_span = None;
                leaf.row_span = None;
                leaf.table_semantics = None;
                leaf.structure_id = None;
                leaf.destination_sources.clear();
                normalized.push(leaf);
            }
            continue;
        }
        let key = record.table_semantics.as_ref().and_then(|node| {
            if record.mcid.is_some() {
                return None;
            }
            let local = match record.role.as_str() {
                "Table" => Some(0),
                "THead" | "TBody" | "TFoot" | "Caption" => node.group_key,
                "TR" => node.row_key,
                _ => None,
            }?;
            Some((
                record.table_document,
                node.table_key,
                record.role.clone(),
                local,
            ))
        });
        if let Some(key) = key {
            if let Some(&existing) = containers.get(&key) {
                normalized[existing]
                    .destination_sources
                    .extend(record.destination_sources);
                mapped.push(existing);
                continue;
            }
            containers.insert(key, normalized.len());
        }
        mapped.push(normalized.len());
        normalized.push(record);
    }
    let mut catalogs = BTreeMap::<usize, CompiledTableCatalog>::new();
    for record in &normalized {
        if let Some(node) = &record.table_semantics {
            catalogs
                .entry(record.table_document)
                .or_default()
                .observe(&record.role, node);
        }
    }
    for catalog in catalogs.values_mut() {
        catalog.finish();
    }
    for record in &mut normalized {
        if let Some(node) = &record.table_semantics {
            let catalog = &catalogs[&record.table_document];
            if let Some(span) = catalog.logical_row_span(node) {
                record.row_span = Some(span);
            }
            if record.role == "TH"
                && node
                    .cell_key
                    .is_some_and(|cell| catalog.unique_header(node.table_key, cell))
            {
                record.structure_id = node
                    .cell_key
                    .map(|cell| cell_id(record.table_document, node.table_key, cell));
            }
            record.table_semantics = Some(Arc::new(catalog.resolve(node)));
        }
    }
    normalized
}

pub(super) fn cell_id(document: usize, table: u64, cell: u64) -> String {
    // ASCII byte strings with no source IDs or recipient data; ordered only by
    // deterministic document insertion and source preorder, never global IDs.
    format!("fb-d{document}-t{table}-c{cell}")
}
