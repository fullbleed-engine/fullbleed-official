//! Logical table continuation and PDF structure identities. No pixel changes.
use super::TagRecord;
use crate::table_semantics::CompiledTableCatalog;
use std::collections::BTreeMap;
use std::sync::Arc;

pub(super) fn normalize(records: Vec<TagRecord>) -> Vec<TagRecord> {
    let mut mapped = Vec::<usize>::with_capacity(records.len());
    let mut containers = BTreeMap::new();
    let mut normalized = Vec::with_capacity(records.len());
    for mut record in records {
        record.parent = record.parent.and_then(|parent| mapped.get(parent).copied());
        let key = record.table_semantics.as_ref().and_then(|node| {
            if record.mcid.is_some() {
                return None;
            }
            let local = match record.role.as_str() {
                "Table" => Some(0),
                "THead" | "TBody" | "TFoot" => node.group_key,
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
