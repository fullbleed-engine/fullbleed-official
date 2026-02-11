use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::debug::json_escape;

#[derive(Clone)]
pub(crate) struct PerfLogger {
    inner: Arc<Mutex<PerfState>>,
}

struct PerfState {
    writer: BufWriter<File>,
    path: PathBuf,
    span_totals: HashMap<String, f64>,
    span_counts: HashMap<String, u64>,
    count_totals: HashMap<String, u64>,
}

impl PerfLogger {
    pub fn new(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = File::create(&path)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(PerfState {
                writer: BufWriter::new(file),
                path,
                span_totals: HashMap::new(),
                span_counts: HashMap::new(),
                count_totals: HashMap::new(),
            })),
        })
    }

    #[allow(dead_code)]
    pub fn log_json(&self, json: &str) {
        if let Ok(mut state) = self.inner.lock() {
            let _ = writeln!(state.writer, "{json}");
        }
    }

    pub fn log_span_ms(&self, name: &str, doc_id: Option<usize>, ms: f64) {
        let doc = doc_id
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string());
        let json = format!(
            "{{\"type\":\"perf.span\",\"name\":\"{}\",\"doc_id\":{},\"unit\":\"ms\",\"ms\":{:.3}}}",
            json_escape(name),
            doc,
            ms
        );
        if let Ok(mut state) = self.inner.lock() {
            *state.span_totals.entry(name.to_string()).or_insert(0.0) += ms;
            let entry = state.span_counts.entry(name.to_string()).or_insert(0);
            *entry = entry.saturating_add(1);
            let _ = writeln!(state.writer, "{json}");
        }
    }

    pub fn log_counts(&self, name: &str, doc_id: Option<usize>, counts: &[(&str, u64)]) {
        let doc = doc_id
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string());
        let mut out = format!(
            "{{\"type\":\"perf.counts\",\"name\":\"{}\",\"doc_id\":{},\"counts\":{{",
            json_escape(name),
            doc
        );
        for (idx, (key, value)) in counts.iter().enumerate() {
            if idx > 0 {
                out.push(',');
            }
            out.push_str(&format!("\"{}\":{}", json_escape(key), value));
        }
        out.push_str("}}");
        if let Ok(mut state) = self.inner.lock() {
            for (key, value) in counts {
                let full_key = format!("{name}.{key}");
                let entry = state.count_totals.entry(full_key).or_insert(0);
                *entry = entry.saturating_add(*value);
            }
            let _ = writeln!(state.writer, "{out}");
        }
    }

    pub fn flush(&self) {
        if let Ok(mut state) = self.inner.lock() {
            let _ = state.writer.flush();
        }
    }
}

impl Drop for PerfState {
    fn drop(&mut self) {
        let hot_path = hot_path_for(&self.path);
        let Ok(file) = File::create(&hot_path) else {
            return;
        };
        let mut writer = BufWriter::new(file);

        let mut spans: Vec<(&String, &f64)> = self.span_totals.iter().collect();
        spans.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (rank, (name, ms)) in spans.into_iter().take(100).enumerate() {
            let count = *self.span_counts.get(name).unwrap_or(&1);
            let avg = if count == 0 { 0.0 } else { ms / count as f64 };
            let _ = writeln!(
                writer,
                "{{\"type\":\"perf.hot.span\",\"rank\":{},\"name\":\"{}\",\"unit\":\"ms\",\"agg\":\"sum\",\"ms\":{:.3},\"count\":{},\"avg_ms\":{:.3}}}",
                rank + 1,
                json_escape(name),
                ms,
                count,
                avg
            );
        }

        let mut counts: Vec<(&String, &u64)> = self.count_totals.iter().collect();
        counts.sort_by(|a, b| b.1.cmp(a.1));
        for (rank, (name, value)) in counts.into_iter().take(100).enumerate() {
            let _ = writeln!(
                writer,
                "{{\"type\":\"perf.hot.count\",\"rank\":{},\"name\":\"{}\",\"value\":{}}}",
                rank + 1,
                json_escape(name),
                value
            );
        }
    }
}

fn hot_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("fullbleed_perf.log");
    let stem = file_name
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(file_name);
    let hot_name = format!("{stem}_hot.log");
    path.with_file_name(hot_name)
}
