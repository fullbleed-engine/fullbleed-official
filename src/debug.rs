use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(crate) struct DebugLogger {
    inner: Arc<Mutex<DebugState>>,
}

struct DebugState {
    writer: BufWriter<File>,
    counters: HashMap<String, u64>,
}

impl DebugLogger {
    pub fn new(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = File::create(path)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(DebugState {
                writer: BufWriter::new(file),
                counters: HashMap::new(),
            })),
        })
    }

    pub fn log_json(&self, json: &str) {
        if let Ok(mut writer) = self.inner.lock() {
            let _ = writeln!(writer.writer, "{json}");
        }
    }

    pub fn increment(&self, key: &str, amount: u64) {
        if let Ok(mut state) = self.inner.lock() {
            let entry = state.counters.entry(key.to_string()).or_insert(0);
            *entry = entry.saturating_add(amount);
        }
    }

    pub fn emit_summary(&self, context: &str) {
        if let Ok(mut state) = self.inner.lock() {
            let mut counters: Vec<(String, u64)> = state.counters.drain().collect();
            counters.sort_by(|a, b| a.0.cmp(&b.0));
            let counts_json = if counters.is_empty() {
                "{}".to_string()
            } else {
                let mut out = String::from("{");
                for (idx, (key, value)) in counters.iter().enumerate() {
                    if idx > 0 {
                        out.push(',');
                    }
                    out.push_str(&format!("\"{}\":{}", json_escape(key), value));
                }
                out.push('}');
                out
            };
            let json = format!(
                "{{\"type\":\"debug.summary\",\"context\":\"{}\",\"counts\":{}}}",
                json_escape(context),
                counts_json
            );
            let _ = writeln!(state.writer, "{json}");
        }
    }

    pub fn flush(&self) {
        if let Ok(mut state) = self.inner.lock() {
            let _ = state.writer.flush();
        }
    }
}

pub(crate) fn json_escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 8);
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}
