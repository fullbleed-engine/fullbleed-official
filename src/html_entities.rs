//! WHATWG HTML named character references as checked-in, dependency-free tables.
//!
//! Keys intentionally retain the semicolon. Legacy semicolon-less spellings are separate
//! entries, allowing the tokenizer to enforce the attribute ambiguity rule exactly.

pub(crate) type Entity = (&'static str, u32, u32);

pub(crate) static NAMED_ENTITY_TABLES: &[&[Entity]] = &[
    include!("html_entities_0.rs"),
    include!("html_entities_1.rs"),
    include!("html_entities_2.rs"),
    include!("html_entities_3.rs"),
    include!("html_entities_4.rs"),
    include!("html_entities_5.rs"),
];
