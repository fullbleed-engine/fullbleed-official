use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const CONTRACT_ID: &str = "fullbleed.audit_contract";
pub const CONTRACT_VERSION: &str = "1";

const AUDIT_REGISTRY_ID: &str = "fullbleed.audit_registry.v1";
const WCAG20AA_REGISTRY_ID: &str = "wcag20aa_registry.v1";
const SECTION508_HTML_REGISTRY_ID: &str = "section508_html_registry.v1";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PmrCategoryDef {
    pub id: &'static str,
    pub name: &'static str,
    pub weight: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PmrAuditDef {
    pub id: &'static str,
    pub category: &'static str,
    pub weight: f64,
    pub class_name: &'static str,
    pub verification_mode: &'static str,
    pub severity: &'static str,
    pub stage: &'static str,
    pub scored: bool,
}

pub const PMR_CATEGORIES_V1: [PmrCategoryDef; 5] = [
    PmrCategoryDef {
        id: "document-semantics",
        name: "Document Semantics",
        weight: 20.0,
    },
    PmrCategoryDef {
        id: "reading-order-structure",
        name: "Reading Order & Structure",
        weight: 20.0,
    },
    PmrCategoryDef {
        id: "paged-layout-integrity",
        name: "Paged Layout Integrity",
        weight: 25.0,
    },
    PmrCategoryDef {
        id: "field-table-form-integrity",
        name: "Field/Table/Form Integrity",
        weight: 20.0,
    },
    PmrCategoryDef {
        id: "artifact-packaging-reproducibility",
        name: "Artifact Packaging & Reproducibility",
        weight: 15.0,
    },
];

pub const PMR_AUDITS_V1: [PmrAuditDef; 13] = [
    PmrAuditDef { id: "pmr.doc.lang_present_valid", category: "document-semantics", weight: 3.0, class_name: "required", verification_mode: "machine", severity: "high", stage: "post-emit", scored: true },
    PmrAuditDef { id: "pmr.doc.title_present_nonempty", category: "document-semantics", weight: 3.0, class_name: "required", verification_mode: "machine", severity: "high", stage: "post-emit", scored: true },
    PmrAuditDef { id: "pmr.doc.metadata_engine_persistence", category: "document-semantics", weight: 4.0, class_name: "scored", verification_mode: "machine", severity: "high", stage: "post-emit", scored: true },
    PmrAuditDef { id: "pmr.layout.overflow_none", category: "paged-layout-integrity", weight: 6.0, class_name: "required", verification_mode: "machine", severity: "critical", stage: "post-render", scored: true },
    PmrAuditDef { id: "pmr.layout.known_loss_none_critical", category: "paged-layout-integrity", weight: 5.0, class_name: "required", verification_mode: "machine", severity: "high", stage: "post-render", scored: true },
    PmrAuditDef { id: "pmr.layout.page_count_target", category: "paged-layout-integrity", weight: 4.0, class_name: "scored", verification_mode: "machine", severity: "high", stage: "post-render", scored: true },
    PmrAuditDef { id: "pmr.forms.id_ref_integrity", category: "field-table-form-integrity", weight: 4.0, class_name: "required", verification_mode: "machine", severity: "critical", stage: "post-emit", scored: true },
    PmrAuditDef { id: "pmr.tables.semantic_table_headers", category: "field-table-form-integrity", weight: 4.0, class_name: "scored", verification_mode: "machine", severity: "high", stage: "post-emit", scored: true },
    PmrAuditDef { id: "pmr.signatures.text_semantics_present", category: "field-table-form-integrity", weight: 3.0, class_name: "scored", verification_mode: "machine", severity: "medium", stage: "pre-render", scored: true },
    PmrAuditDef { id: "pmr.cav.document_only_content", category: "field-table-form-integrity", weight: 3.0, class_name: "required", verification_mode: "machine", severity: "critical", stage: "post-emit", scored: true },
    PmrAuditDef { id: "pmr.artifacts.html_emitted", category: "artifact-packaging-reproducibility", weight: 3.0, class_name: "required", verification_mode: "machine", severity: "high", stage: "post-emit", scored: true },
    PmrAuditDef { id: "pmr.artifacts.css_emitted", category: "artifact-packaging-reproducibility", weight: 3.0, class_name: "required", verification_mode: "machine", severity: "high", stage: "post-emit", scored: true },
    PmrAuditDef { id: "pmr.artifacts.linked_css_reference", category: "artifact-packaging-reproducibility", weight: 2.0, class_name: "opportunity", verification_mode: "machine", severity: "low", stage: "post-emit", scored: false },
];

// Runtime authority is compiled into the binary. These payloads are currently sourced
// from frozen spec artifacts at compile time; runtime never reads from repo files.
const AUDIT_REGISTRY_V1_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/fullbleed.audit_registry.v1.yaml"
));
const WCAG20AA_REGISTRY_V1_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/wcag20aa_registry.v1.yaml"
));
const SECTION508_HTML_REGISTRY_V1_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/section508_html_registry.v1.yaml"
));

#[derive(Debug, Clone)]
pub struct AuditContractMetadata {
    pub contract_id: &'static str,
    pub contract_version: &'static str,
    pub contract_fingerprint_sha256: String,
    pub audit_registry_id: &'static str,
    pub audit_registry_hash_sha256: String,
    pub wcag20aa_registry_id: &'static str,
    pub wcag20aa_registry_hash_sha256: String,
    pub section508_html_registry_id: &'static str,
    pub section508_html_registry_hash_sha256: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WcagImplementedMappedResultCounts {
    pub pass: usize,
    pub fail: usize,
    pub warn: usize,
    pub manual_needed: usize,
    pub not_applicable: usize,
    pub unknown: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wcag20AaCoverageSummary {
    pub registry_id: String,
    pub registry_version: usize,
    pub wcag_version: String,
    pub target_level: String,
    pub total_entries: usize,
    pub success_criteria_total: usize,
    pub conformance_requirements_total: usize,
    pub mapped_entry_count: usize,
    pub mapped_success_criteria_count: usize,
    pub mapped_conformance_requirement_count: usize,
    pub implemented_mapped_entry_count: usize,
    pub implemented_mapped_entry_evaluated_count: usize,
    pub implemented_mapped_entry_pending_count: usize,
    pub supporting_only_mapped_entry_count: usize,
    pub planned_only_mapped_entry_count: usize,
    pub unmapped_entry_count: usize,
    pub implemented_mapped_result_counts: WcagImplementedMappedResultCounts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section508HtmlCoverageSummary {
    pub registry_id: String,
    pub registry_version: usize,
    pub profile_id: String,
    pub total_entries: usize,
    pub specific_entries_total: usize,
    pub inherited_wcag_entries_total: usize,
    pub mapped_entry_count: usize,
    pub implemented_mapped_entry_count: usize,
    pub implemented_mapped_entry_evaluated_count: usize,
    pub implemented_mapped_entry_pending_count: usize,
    pub supporting_only_mapped_entry_count: usize,
    pub planned_only_mapped_entry_count: usize,
    pub unmapped_entry_count: usize,
    pub specific_mapped_entry_count: usize,
    pub specific_implemented_mapped_entry_count: usize,
    pub specific_implemented_mapped_entry_evaluated_count: usize,
    pub specific_implemented_mapped_entry_pending_count: usize,
    pub specific_unmapped_entry_count: usize,
    pub inherited_wcag_registry_id: String,
    pub inherited_wcag_implemented_mapped_entry_count: usize,
    pub inherited_wcag_implemented_mapped_entry_evaluated_count: usize,
    pub inherited_wcag_unmapped_entry_count: usize,
    pub implemented_mapped_result_counts: WcagImplementedMappedResultCounts,
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{:02x}", b);
    }
    out
}

fn hash_memoized(cell: &OnceLock<String>, text: &str) -> String {
    cell.get_or_init(|| hex_sha256(text.as_bytes())).clone()
}

static AUDIT_REGISTRY_HASH: OnceLock<String> = OnceLock::new();
static WCAG20AA_REGISTRY_HASH: OnceLock<String> = OnceLock::new();
static SECTION508_HTML_REGISTRY_HASH: OnceLock<String> = OnceLock::new();
static CONTRACT_FINGERPRINT: OnceLock<String> = OnceLock::new();
static WCAG20AA_REGISTRY_JSON_VALUE: OnceLock<Value> = OnceLock::new();
static SECTION508_HTML_REGISTRY_JSON_VALUE: OnceLock<Value> = OnceLock::new();

fn wcag20aa_registry_value() -> &'static Value {
    WCAG20AA_REGISTRY_JSON_VALUE.get_or_init(|| {
        serde_json::from_str(WCAG20AA_REGISTRY_V1_JSON)
            .expect("embedded wcag20aa registry must be valid JSON-formatted YAML")
    })
}

fn section508_html_registry_value() -> &'static Value {
    SECTION508_HTML_REGISTRY_JSON_VALUE.get_or_init(|| {
        serde_json::from_str(SECTION508_HTML_REGISTRY_V1_JSON)
            .expect("embedded section508 html registry must be valid JSON-formatted YAML")
    })
}

fn worst_verdict<'a>(verdicts: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut best: Option<(&'a str, i32)> = None;
    for verdict in verdicts {
        let rank = match verdict {
            "fail" => 5,
            "warn" => 4,
            "manual_needed" => 3,
            "pass" => 2,
            "not_applicable" => 1,
            _ => 0,
        };
        if let Some((_, best_rank)) = best {
            if rank > best_rank {
                best = Some((verdict, rank));
            }
        } else {
            best = Some((verdict, rank));
        }
    }
    best.map(|(v, _)| v)
}

pub fn audit_registry_v1_json() -> &'static str {
    AUDIT_REGISTRY_V1_JSON
}

pub fn wcag20aa_registry_v1_json() -> &'static str {
    WCAG20AA_REGISTRY_V1_JSON
}

pub fn audit_registry_v1_hash_sha256() -> String {
    hash_memoized(&AUDIT_REGISTRY_HASH, AUDIT_REGISTRY_V1_JSON)
}

pub fn wcag20aa_registry_v1_hash_sha256() -> String {
    hash_memoized(&WCAG20AA_REGISTRY_HASH, WCAG20AA_REGISTRY_V1_JSON)
}

pub fn section508_html_registry_v1_hash_sha256() -> String {
    hash_memoized(&SECTION508_HTML_REGISTRY_HASH, SECTION508_HTML_REGISTRY_V1_JSON)
}

pub fn contract_fingerprint_sha256() -> String {
    CONTRACT_FINGERPRINT
        .get_or_init(|| {
            let mut hasher = Sha256::new();
            hasher.update(CONTRACT_ID.as_bytes());
            hasher.update(b"\n");
            hasher.update(CONTRACT_VERSION.as_bytes());
            hasher.update(b"\n");
            hasher.update(AUDIT_REGISTRY_ID.as_bytes());
            hasher.update(b"\n");
            hasher.update(audit_registry_v1_hash_sha256().as_bytes());
            hasher.update(b"\n");
            hasher.update(WCAG20AA_REGISTRY_ID.as_bytes());
            hasher.update(b"\n");
            hasher.update(wcag20aa_registry_v1_hash_sha256().as_bytes());
            hasher.update(b"\n");
            hasher.update(SECTION508_HTML_REGISTRY_ID.as_bytes());
            hasher.update(b"\n");
            hasher.update(section508_html_registry_v1_hash_sha256().as_bytes());
            let digest = hasher.finalize();
            let mut out = String::with_capacity(digest.len() * 2);
            for b in digest {
                use std::fmt::Write;
                let _ = write!(&mut out, "{:02x}", b);
            }
            out
        })
        .clone()
}

pub fn registry_json(name: &str) -> Option<&'static str> {
    match name {
        AUDIT_REGISTRY_ID => Some(AUDIT_REGISTRY_V1_JSON),
        WCAG20AA_REGISTRY_ID => Some(WCAG20AA_REGISTRY_V1_JSON),
        SECTION508_HTML_REGISTRY_ID => Some(SECTION508_HTML_REGISTRY_V1_JSON),
        _ => None,
    }
}

pub fn section508_html_registry_v1_json() -> &'static str {
    SECTION508_HTML_REGISTRY_V1_JSON
}

pub fn wcag20aa_coverage_from_rule_verdicts<'a, I>(rule_verdicts: I) -> Wcag20AaCoverageSummary
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let root = wcag20aa_registry_value();
    let entries = root
        .get("entries")
        .and_then(Value::as_array)
        .expect("wcag20aa registry entries array");

    let mut verdict_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (rule_id, verdict) in rule_verdicts {
        let rid = rule_id.trim();
        if rid.is_empty() {
            continue;
        }
        verdict_map
            .entry(rid.to_string())
            .or_default()
            .push(verdict.to_string());
    }

    fn entry_mappings<'a>(entry: &'a Value) -> Vec<&'a Value> {
        entry.get("fullbleed_rule_mapping")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter(|v| v.is_object()).collect())
            .unwrap_or_default()
    }

    let mapped_entries: Vec<&Value> = entries
        .iter()
        .filter(|e| !entry_mappings(e).is_empty())
        .collect();
    let sc_entries: Vec<&Value> = entries
        .iter()
        .filter(|e| e.get("kind").and_then(Value::as_str) == Some("success_criterion"))
        .collect();
    let conf_entries: Vec<&Value> = entries
        .iter()
        .filter(|e| e.get("kind").and_then(Value::as_str) == Some("conformance_requirement"))
        .collect();

    let mut implemented_entries: Vec<&Value> = Vec::new();
    let mut supporting_only_count = 0usize;
    let mut planned_only_count = 0usize;
    for entry in &mapped_entries {
        let mappings = entry_mappings(entry);
        let statuses: std::collections::BTreeSet<String> = mappings
            .iter()
            .filter_map(|m| m.get("status").and_then(Value::as_str))
            .map(|s| s.to_string())
            .collect();
        if statuses.contains("implemented") {
            implemented_entries.push(entry);
        } else if statuses == std::collections::BTreeSet::from(["supporting".to_string()])
            || (statuses.contains("supporting") && !statuses.contains("planned"))
        {
            supporting_only_count += 1;
        } else {
            planned_only_count += 1;
        }
    }

    let mut implemented_evaluated = 0usize;
    let mut implemented_pending = 0usize;
    let mut result_counts = WcagImplementedMappedResultCounts::default();
    for entry in &implemented_entries {
        let implemented_rule_ids: Vec<&str> = entry_mappings(entry)
            .iter()
            .filter(|m| m.get("status").and_then(Value::as_str) == Some("implemented"))
            .filter_map(|m| m.get("id").and_then(Value::as_str))
            .collect();
        let mut verdicts: Vec<&str> = Vec::new();
        for rid in implemented_rule_ids {
            if let Some(rows) = verdict_map.get(rid) {
                verdicts.extend(rows.iter().map(String::as_str));
            }
        }
        if verdicts.is_empty() {
            implemented_pending += 1;
            continue;
        }
        implemented_evaluated += 1;
        match worst_verdict(verdicts.into_iter()).unwrap_or("unknown") {
            "pass" => result_counts.pass += 1,
            "fail" => result_counts.fail += 1,
            "warn" => result_counts.warn += 1,
            "manual_needed" => result_counts.manual_needed += 1,
            "not_applicable" => result_counts.not_applicable += 1,
            _ => result_counts.unknown += 1,
        }
    }

    let mapped_success_criteria_count = sc_entries
        .iter()
        .filter(|e| !entry_mappings(e).is_empty())
        .count();
    let mapped_conformance_requirement_count = conf_entries
        .iter()
        .filter(|e| !entry_mappings(e).is_empty())
        .count();

    let scope = root.get("scope").and_then(Value::as_object);
    let total_entries = scope
        .and_then(|s| s.get("total_entries"))
        .and_then(Value::as_u64)
        .unwrap_or(entries.len() as u64) as usize;
    let success_criteria_total = scope
        .and_then(|s| s.get("total_success_criteria"))
        .and_then(Value::as_u64)
        .unwrap_or(sc_entries.len() as u64) as usize;
    let conformance_requirements_total = scope
        .and_then(|s| s.get("total_conformance_requirements"))
        .and_then(Value::as_u64)
        .unwrap_or(conf_entries.len() as u64) as usize;

    Wcag20AaCoverageSummary {
        registry_id: root
            .get("schema")
            .and_then(Value::as_str)
            .unwrap_or(WCAG20AA_REGISTRY_ID)
            .to_string(),
        registry_version: root
            .get("version")
            .and_then(Value::as_u64)
            .unwrap_or(1) as usize,
        wcag_version: root
            .get("wcag_version")
            .and_then(Value::as_str)
            .unwrap_or("2.0")
            .to_string(),
        target_level: root
            .get("target_level")
            .and_then(Value::as_str)
            .unwrap_or("AA")
            .to_string(),
        total_entries,
        success_criteria_total,
        conformance_requirements_total,
        mapped_entry_count: mapped_entries.len(),
        mapped_success_criteria_count,
        mapped_conformance_requirement_count,
        implemented_mapped_entry_count: implemented_entries.len(),
        implemented_mapped_entry_evaluated_count: implemented_evaluated,
        implemented_mapped_entry_pending_count: implemented_pending,
        supporting_only_mapped_entry_count: supporting_only_count,
        planned_only_mapped_entry_count: planned_only_count,
        unmapped_entry_count: total_entries.saturating_sub(mapped_entries.len()),
        implemented_mapped_result_counts: result_counts,
    }
}

pub fn section508_html_coverage_from_rule_verdicts<'a, I>(
    rule_verdicts: I,
) -> Section508HtmlCoverageSummary
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let owned_pairs: Vec<(String, String)> = rule_verdicts
        .into_iter()
        .map(|(rid, verdict)| (rid.to_string(), verdict.to_string()))
        .collect();
    let wcag_pairs: Vec<(&str, &str)> = owned_pairs
        .iter()
        .map(|(rid, verdict)| (rid.as_str(), verdict.as_str()))
        .collect();
    let wcag = wcag20aa_coverage_from_rule_verdicts(wcag_pairs);

    let root = section508_html_registry_value();
    let entries = root
        .get("entries")
        .and_then(Value::as_array)
        .expect("section508 html registry entries array");

    let mut verdict_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (rule_id, verdict) in &owned_pairs {
        let rid = rule_id.trim();
        if rid.is_empty() {
            continue;
        }
        verdict_map
            .entry(rid.to_string())
            .or_default()
            .push(verdict.to_string());
    }

    fn entry_mappings<'a>(entry: &'a Value) -> Vec<&'a Value> {
        entry.get("fullbleed_rule_mapping")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter(|v| v.is_object()).collect())
            .unwrap_or_default()
    }

    let mapped_entries: Vec<&Value> = entries
        .iter()
        .filter(|e| !entry_mappings(e).is_empty())
        .collect();

    let mut implemented_entries: Vec<&Value> = Vec::new();
    let mut supporting_only_count = 0usize;
    let mut planned_only_count = 0usize;
    for entry in &mapped_entries {
        let mappings = entry_mappings(entry);
        let statuses: std::collections::BTreeSet<String> = mappings
            .iter()
            .filter_map(|m| m.get("status").and_then(Value::as_str))
            .map(|s| s.to_string())
            .collect();
        if statuses.contains("implemented") {
            implemented_entries.push(entry);
        } else if statuses == std::collections::BTreeSet::from(["supporting".to_string()])
            || (statuses.contains("supporting") && !statuses.contains("planned"))
        {
            supporting_only_count += 1;
        } else {
            planned_only_count += 1;
        }
    }

    let mut specific_implemented_evaluated = 0usize;
    let mut specific_implemented_pending = 0usize;
    let mut specific_result_counts = WcagImplementedMappedResultCounts::default();
    for entry in &implemented_entries {
        let implemented_rule_ids: Vec<&str> = entry_mappings(entry)
            .iter()
            .filter(|m| m.get("status").and_then(Value::as_str) == Some("implemented"))
            .filter_map(|m| m.get("id").and_then(Value::as_str))
            .collect();
        let mut verdicts: Vec<&str> = Vec::new();
        for rid in implemented_rule_ids {
            if let Some(rows) = verdict_map.get(rid) {
                verdicts.extend(rows.iter().map(String::as_str));
            }
        }
        if verdicts.is_empty() {
            specific_implemented_pending += 1;
            continue;
        }
        specific_implemented_evaluated += 1;
        match worst_verdict(verdicts.into_iter()).unwrap_or("unknown") {
            "pass" => specific_result_counts.pass += 1,
            "fail" => specific_result_counts.fail += 1,
            "warn" => specific_result_counts.warn += 1,
            "manual_needed" => specific_result_counts.manual_needed += 1,
            "not_applicable" => specific_result_counts.not_applicable += 1,
            _ => specific_result_counts.unknown += 1,
        }
    }

    let scope = root.get("scope").and_then(Value::as_object);
    let specific_entries_total = scope
        .and_then(|s| s.get("total_specific_entries"))
        .and_then(Value::as_u64)
        .unwrap_or(entries.len() as u64) as usize;
    let inherited_wcag_entries_total = scope
        .and_then(|s| s.get("inherited_wcag_entry_count"))
        .and_then(Value::as_u64)
        .unwrap_or(wcag.total_entries as u64) as usize;
    let total_entries = scope
        .and_then(|s| s.get("total_entries"))
        .and_then(Value::as_u64)
        .unwrap_or((specific_entries_total + inherited_wcag_entries_total) as u64)
        as usize;

    let mut combined_result_counts = specific_result_counts.clone();
    combined_result_counts.pass += wcag.implemented_mapped_result_counts.pass;
    combined_result_counts.fail += wcag.implemented_mapped_result_counts.fail;
    combined_result_counts.warn += wcag.implemented_mapped_result_counts.warn;
    combined_result_counts.manual_needed += wcag.implemented_mapped_result_counts.manual_needed;
    combined_result_counts.not_applicable += wcag.implemented_mapped_result_counts.not_applicable;
    combined_result_counts.unknown += wcag.implemented_mapped_result_counts.unknown;

    let specific_unmapped_entry_count = specific_entries_total.saturating_sub(mapped_entries.len());
    let mapped_entry_count = mapped_entries.len() + wcag.mapped_entry_count;
    let implemented_mapped_entry_count =
        implemented_entries.len() + wcag.implemented_mapped_entry_count;
    let implemented_mapped_entry_evaluated_count =
        specific_implemented_evaluated + wcag.implemented_mapped_entry_evaluated_count;
    let implemented_mapped_entry_pending_count =
        specific_implemented_pending + wcag.implemented_mapped_entry_pending_count;

    Section508HtmlCoverageSummary {
        registry_id: root
            .get("schema")
            .and_then(Value::as_str)
            .unwrap_or(SECTION508_HTML_REGISTRY_ID)
            .to_string(),
        registry_version: root
            .get("version")
            .and_then(Value::as_u64)
            .unwrap_or(1) as usize,
        profile_id: root
            .get("profile_id")
            .and_then(Value::as_str)
            .unwrap_or("section508.revised.e205.html")
            .to_string(),
        total_entries,
        specific_entries_total,
        inherited_wcag_entries_total,
        mapped_entry_count,
        implemented_mapped_entry_count,
        implemented_mapped_entry_evaluated_count,
        implemented_mapped_entry_pending_count,
        supporting_only_mapped_entry_count: supporting_only_count + wcag.supporting_only_mapped_entry_count,
        planned_only_mapped_entry_count: planned_only_count + wcag.planned_only_mapped_entry_count,
        unmapped_entry_count: total_entries.saturating_sub(mapped_entry_count),
        specific_mapped_entry_count: mapped_entries.len(),
        specific_implemented_mapped_entry_count: implemented_entries.len(),
        specific_implemented_mapped_entry_evaluated_count: specific_implemented_evaluated,
        specific_implemented_mapped_entry_pending_count: specific_implemented_pending,
        specific_unmapped_entry_count,
        inherited_wcag_registry_id: wcag.registry_id.clone(),
        inherited_wcag_implemented_mapped_entry_count: wcag.implemented_mapped_entry_count,
        inherited_wcag_implemented_mapped_entry_evaluated_count: wcag
            .implemented_mapped_entry_evaluated_count,
        inherited_wcag_unmapped_entry_count: wcag.unmapped_entry_count,
        implemented_mapped_result_counts: combined_result_counts,
    }
}

pub fn pmr_category_defs_v1() -> &'static [PmrCategoryDef] {
    &PMR_CATEGORIES_V1
}

pub fn pmr_audit_defs_v1() -> &'static [PmrAuditDef] {
    &PMR_AUDITS_V1
}

pub fn pmr_audit_def(audit_id: &str) -> Option<&'static PmrAuditDef> {
    PMR_AUDITS_V1.iter().find(|d| d.id == audit_id)
}

pub fn pmr_default_gate_level(audit_id: &str) -> &'static str {
    match audit_id {
        "pmr.layout.page_count_target" => "warn",
        "pmr.signatures.text_semantics_present" => "warn",
        "pmr.artifacts.linked_css_reference" => "warn",
        _ => "error",
    }
}

pub fn pmr_profile_gate_override(profile: &str, audit_id: &str) -> Option<&'static str> {
    let p = profile.to_ascii_lowercase();
    match (p.as_str(), audit_id) {
        ("cav", "pmr.layout.page_count_target") => Some("error"),
        ("cav", "pmr.cav.document_only_content") => Some("error"),
        ("transactional", "pmr.layout.page_count_target") => Some("warn"),
        ("strict", "pmr.artifacts.linked_css_reference") => Some("warn"),
        _ => None,
    }
}

pub fn pmr_effective_gate_level(profile: &str, audit_id: &str) -> &'static str {
    pmr_profile_gate_override(profile, audit_id).unwrap_or_else(|| pmr_default_gate_level(audit_id))
}

pub fn metadata() -> AuditContractMetadata {
    AuditContractMetadata {
        contract_id: CONTRACT_ID,
        contract_version: CONTRACT_VERSION,
        contract_fingerprint_sha256: contract_fingerprint_sha256(),
        audit_registry_id: AUDIT_REGISTRY_ID,
        audit_registry_hash_sha256: audit_registry_v1_hash_sha256(),
        wcag20aa_registry_id: WCAG20AA_REGISTRY_ID,
        wcag20aa_registry_hash_sha256: wcag20aa_registry_v1_hash_sha256(),
        section508_html_registry_id: SECTION508_HTML_REGISTRY_ID,
        section508_html_registry_hash_sha256: section508_html_registry_v1_hash_sha256(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn parse_embedded_audit_registry() -> Value {
        serde_json::from_str(AUDIT_REGISTRY_V1_JSON).expect("embedded audit registry JSON should parse")
    }

    #[test]
    fn contract_fingerprint_is_stable_and_nonempty() {
        let a = contract_fingerprint_sha256();
        let b = contract_fingerprint_sha256();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn registry_lookup_returns_known_payloads() {
        assert!(registry_json(AUDIT_REGISTRY_ID).unwrap().contains("\"schema\": \"fullbleed.audit_registry.v1\""));
        assert!(registry_json(WCAG20AA_REGISTRY_ID)
            .unwrap()
            .contains("\"schema\": \"wcag20aa_registry.v1\""));
        assert!(registry_json(SECTION508_HTML_REGISTRY_ID)
            .unwrap()
            .contains("\"schema\": \"section508_html_registry.v1\""));
        assert!(registry_json("unknown").is_none());
    }

    #[test]
    fn pmr_category_weights_sum_to_100() {
        let sum: f64 = pmr_category_defs_v1().iter().map(|c| c.weight).sum();
        assert!((sum - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn pmr_gate_levels_match_expected_overrides() {
        assert_eq!(
            pmr_effective_gate_level("strict", "pmr.artifacts.linked_css_reference"),
            "warn"
        );
        assert_eq!(
            pmr_effective_gate_level("cav", "pmr.layout.page_count_target"),
            "error"
        );
        assert_eq!(
            pmr_effective_gate_level("transactional", "pmr.layout.page_count_target"),
            "warn"
        );
        assert_eq!(
            pmr_effective_gate_level("strict", "pmr.layout.overflow_none"),
            "error"
        );
    }

    #[test]
    fn pmr_policy_exports_match_embedded_audit_registry() {
        let root = parse_embedded_audit_registry();
        let pmr_categories_json = root
            .get("pmr_categories")
            .and_then(Value::as_array)
            .expect("pmr_categories array");

        assert_eq!(pmr_category_defs_v1().len(), pmr_categories_json.len());
        for (idx, cat) in pmr_categories_json.iter().enumerate() {
            let expected = &pmr_category_defs_v1()[idx];
            assert_eq!(cat.get("id").and_then(Value::as_str), Some(expected.id));
            assert_eq!(cat.get("name").and_then(Value::as_str), Some(expected.name));
            let weight = cat.get("weight").and_then(Value::as_f64).expect("category weight");
            assert!((weight - expected.weight).abs() < f64::EPSILON);
        }

        let category_ids: std::collections::BTreeSet<&str> =
            pmr_category_defs_v1().iter().map(|c| c.id).collect();

        let entries = root
            .get("entries")
            .and_then(Value::as_array)
            .expect("entries array");
        let pmr_entry_ids: Vec<&str> = entries
            .iter()
            .filter(|e| e.get("system").and_then(Value::as_str) == Some("pmr"))
            .map(|e| {
                let id = e.get("id").and_then(Value::as_str).expect("entry id");
                let def = pmr_audit_def(id).unwrap_or_else(|| panic!("missing PMR audit def for {id}"));
                if let Some(category) = e.get("category").and_then(Value::as_str) {
                    assert!(
                        category_ids.contains(category),
                        "unknown PMR category referenced in registry: {category}"
                    );
                    assert_eq!(category, def.category, "category drift for {id}");
                }
                let weight = e.get("weight").and_then(Value::as_f64).expect("weight");
                assert!((weight - def.weight).abs() < f64::EPSILON, "weight drift for {id}");
                assert_eq!(
                    e.get("class").and_then(Value::as_str),
                    Some(def.class_name),
                    "class drift for {id}"
                );
                assert_eq!(
                    e.get("verification_mode").and_then(Value::as_str),
                    Some(def.verification_mode),
                    "verification_mode drift for {id}"
                );
                assert_eq!(
                    e.get("severity").and_then(Value::as_str),
                    Some(def.severity),
                    "severity drift for {id}"
                );
                assert_eq!(
                    e.get("stage").and_then(Value::as_str),
                    Some(def.stage),
                    "stage drift for {id}"
                );
                assert_eq!(
                    e.get("scored").and_then(Value::as_bool),
                    Some(def.scored),
                    "scored drift for {id}"
                );
                let default_gate = e
                    .get("default_gate_level")
                    .and_then(Value::as_str)
                    .expect("default gate level");
                assert_eq!(
                    pmr_default_gate_level(id),
                    default_gate,
                    "default gate level drift for {id}"
                );
                id
            })
            .collect();
        assert_eq!(pmr_entry_ids.len(), pmr_audit_defs_v1().len(), "pmr audit def set size drift");

        let profiles = root
            .get("profiles")
            .and_then(Value::as_object)
            .expect("profiles object");
        for (profile_name, profile_val) in profiles {
            let overrides = profile_val
                .get("overrides")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut expected_overrides: std::collections::BTreeMap<String, String> =
                std::collections::BTreeMap::new();
            for item in overrides {
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .expect("override id")
                    .to_string();
                let level = item
                    .get("level")
                    .and_then(Value::as_str)
                    .expect("override level")
                    .to_string();
                expected_overrides.insert(id, level);
            }
            for audit_id in &pmr_entry_ids {
                let expected = expected_overrides
                    .get(*audit_id)
                    .map(String::as_str)
                    .unwrap_or_else(|| {
                        entries
                            .iter()
                            .find(|e| e.get("id").and_then(Value::as_str) == Some(*audit_id))
                            .and_then(|e| e.get("default_gate_level"))
                            .and_then(Value::as_str)
                            .expect("default gate level present")
                    });
                let actual = pmr_effective_gate_level(profile_name, audit_id);
                assert_eq!(
                    actual, expected,
                    "effective PMR gate level drift for profile={profile_name}, audit={audit_id}"
                );
            }
        }
    }

    #[test]
    fn wcag20aa_coverage_summary_matches_registry_totals() {
        let summary = wcag20aa_coverage_from_rule_verdicts(std::iter::empty::<(&str, &str)>());
        assert_eq!(summary.registry_id, "wcag20aa_registry.v1");
        assert_eq!(summary.registry_version, 1);
        assert_eq!(summary.wcag_version, "2.0");
        assert_eq!(summary.target_level, "AA");
        assert_eq!(summary.total_entries, 43);
        assert_eq!(summary.success_criteria_total, 38);
        assert_eq!(summary.conformance_requirements_total, 5);
        assert_eq!(
            summary.implemented_mapped_entry_count,
            summary.implemented_mapped_entry_evaluated_count
                + summary.implemented_mapped_entry_pending_count
        );
    }

    #[test]
    fn wcag20aa_coverage_aggregates_worst_verdict_for_mapped_entries() {
        let summary = wcag20aa_coverage_from_rule_verdicts([
            ("fb.a11y.html.lang_present_valid", "pass"),
            ("fb.a11y.html.title_present_nonempty", "pass"),
            ("fb.a11y.structure.single_main", "pass"),
            ("fb.a11y.ids.duplicate_id", "fail"),
            ("fb.a11y.aria.reference_target_exists", "pass"),
            ("fb.a11y.signatures.text_semantics_present", "manual_needed"),
        ]);
        assert!(summary.implemented_mapped_result_counts.fail >= 1);
        let counted = summary.implemented_mapped_result_counts.pass
            + summary.implemented_mapped_result_counts.fail
            + summary.implemented_mapped_result_counts.warn
            + summary.implemented_mapped_result_counts.manual_needed
            + summary.implemented_mapped_result_counts.not_applicable
            + summary.implemented_mapped_result_counts.unknown;
        assert_eq!(counted, summary.implemented_mapped_entry_evaluated_count);
        assert!(
            summary.implemented_mapped_entry_evaluated_count >= 2,
            "expected multiple implemented WCAG mappings to be counted"
        );
    }

    #[test]
    fn section508_html_coverage_summary_matches_registry_scope_totals() {
        let summary = section508_html_coverage_from_rule_verdicts(std::iter::empty::<(&str, &str)>());
        assert_eq!(summary.registry_id, "section508_html_registry.v1");
        assert_eq!(summary.registry_version, 1);
        assert_eq!(summary.profile_id, "section508.revised.e205.html");
        assert_eq!(summary.specific_entries_total, 6);
        assert_eq!(summary.inherited_wcag_entries_total, 43);
        assert_eq!(summary.total_entries, 49);
        assert_eq!(
            summary.implemented_mapped_entry_count,
            summary.implemented_mapped_entry_evaluated_count
                + summary.implemented_mapped_entry_pending_count
        );
        assert_eq!(summary.inherited_wcag_registry_id, "wcag20aa_registry.v1");
    }

    #[test]
    fn section508_html_coverage_includes_wcag_inherited_and_specific_mappings() {
        let summary = section508_html_coverage_from_rule_verdicts([
            ("fb.a11y.html.lang_present_valid", "pass"),
            ("fb.a11y.html.title_present_nonempty", "pass"),
            ("fb.a11y.claim.wcag20aa_level_readiness", "warn"),
        ]);
        assert!(summary.implemented_mapped_entry_count >= summary.inherited_wcag_implemented_mapped_entry_count);
        assert!(summary.specific_implemented_mapped_entry_count >= 1);
        assert!(summary.implemented_mapped_result_counts.warn >= 1);
        assert_eq!(
            summary.mapped_entry_count + summary.unmapped_entry_count,
            summary.total_entries
        );
    }
}
