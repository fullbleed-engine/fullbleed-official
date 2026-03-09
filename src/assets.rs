use base64::Engine;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Css,
    Font,
    Image,
    Pdf,
    Svg,
    Other,
}

impl AssetKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AssetKind::Css => "css",
            AssetKind::Font => "font",
            AssetKind::Image => "image",
            AssetKind::Pdf => "pdf",
            AssetKind::Svg => "svg",
            AssetKind::Other => "other",
        }
    }

    pub fn from_str(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "css" => Some(AssetKind::Css),
            "font" => Some(AssetKind::Font),
            "image" => Some(AssetKind::Image),
            "pdf" => Some(AssetKind::Pdf),
            "svg" => Some(AssetKind::Svg),
            "other" => Some(AssetKind::Other),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Asset {
    pub name: String,
    pub kind: AssetKind,
    pub data: Vec<u8>,
    pub source: Option<String>,
    pub trusted: bool,
}

impl Asset {
    pub fn new(
        name: String,
        kind: AssetKind,
        data: Vec<u8>,
        source: Option<String>,
        trusted: bool,
    ) -> Self {
        Self {
            name,
            kind,
            data,
            source,
            trusted,
        }
    }

    pub fn bytes_len(&self) -> usize {
        self.data.len()
    }
}

#[derive(Debug, Clone, Default)]
pub struct AssetBundle {
    pub assets: Vec<Asset>,
}

impl AssetBundle {
    pub fn add(&mut self, asset: Asset) {
        self.assets.push(asset);
    }

    pub fn css_text(&self) -> String {
        let mut out = String::new();
        for asset in &self.assets {
            if asset.kind != AssetKind::Css {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
                out.push('\n');
            }
            out.push_str(&String::from_utf8_lossy(&asset.data));
        }
        out
    }

    pub fn font_assets(&self) -> impl Iterator<Item = &Asset> {
        self.assets
            .iter()
            .filter(|asset| asset.kind == AssetKind::Font)
    }

    pub fn image_assets(&self) -> impl Iterator<Item = &Asset> {
        self.assets.iter().filter(|asset| {
            matches!(asset.kind, AssetKind::Image | AssetKind::Svg)
                || asset
                    .source
                    .as_deref()
                    .and_then(infer_image_mime_from_label)
                    .is_some()
                || infer_image_mime_from_label(&asset.name).is_some()
        })
    }
}

#[cfg(any(feature = "python", test))]
pub fn is_supported_font_path(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|v| v.to_str()) else {
        return false;
    };
    matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf")
}

#[cfg_attr(not(feature = "python"), allow(dead_code))]
#[derive(Debug, Clone)]
pub struct AssetResolutionTrace {
    pub source_uri: String,
    pub normalized_uri: Option<String>,
    pub resolver: String,
    pub success: bool,
    pub mime: Option<String>,
    pub content_kind: String,
    pub render_outcome: String,
    pub asset_name: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedAsset {
    pub trace: AssetResolutionTrace,
    pub bytes: Vec<u8>,
}

pub fn parse_data_uri_bytes(uri: &str) -> Option<(String, Vec<u8>)> {
    if !uri.starts_with("data:") {
        return None;
    }
    let (header, payload) = uri.split_once(',')?;
    let mime = header
        .trim_start_matches("data:")
        .split(';')
        .next()
        .filter(|v| !v.is_empty())
        .unwrap_or("application/octet-stream")
        .to_string();
    let data = if header.contains(";base64") {
        base64::engine::general_purpose::STANDARD
            .decode(payload)
            .ok()?
    } else {
        payload.as_bytes().to_vec()
    };
    Some((mime, data))
}

pub fn file_uri_to_path_buf(source: &str) -> Option<PathBuf> {
    if !source.trim_start().starts_with("file://") {
        return None;
    }
    let raw = source.trim();
    let without_scheme = raw.strip_prefix("file://")?;
    let without_fragment = without_scheme
        .split('#')
        .next()
        .unwrap_or(without_scheme)
        .split('?')
        .next()
        .unwrap_or(without_scheme);
    if without_fragment.is_empty() {
        return None;
    }
    let decoded = percent_decode_lossy(without_fragment);
    #[cfg(windows)]
    {
        let value = decoded.trim_start_matches('/');
        if value.len() >= 2 && value.as_bytes()[1] == b':' {
            return Some(PathBuf::from(value));
        }
        if without_fragment.starts_with("//") {
            return Some(PathBuf::from(format!(
                r"\\{}",
                value.trim_start_matches('/')
            )));
        }
        return Some(PathBuf::from(decoded));
    }
    #[cfg(not(windows))]
    {
        if without_fragment.starts_with('/') {
            return Some(PathBuf::from(decoded));
        }
        Some(PathBuf::from(format!(
            "/{}",
            decoded.trim_start_matches('/')
        )))
    }
}

pub fn load_svg_xml_from_image_source(
    bundle: Option<&AssetBundle>,
    source: &str,
) -> Option<String> {
    let resolved = resolve_image_asset(bundle, source);
    if !resolved.trace.success || resolved.trace.content_kind != "svg" {
        return None;
    }
    String::from_utf8(resolved.bytes)
        .ok()
        .map(|xml| xml.trim_start_matches('\u{feff}').to_string())
}

pub fn renderable_image_source(bundle: Option<&AssetBundle>, source: &str) -> Option<String> {
    let resolved = resolve_image_asset(bundle, source);
    if !resolved.trace.success {
        return None;
    }
    match resolved.trace.resolver.as_str() {
        "bundle" => {
            let mime = resolved
                .trace
                .mime
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_string());
            let payload = base64::engine::general_purpose::STANDARD.encode(resolved.bytes);
            Some(format!("data:{mime};base64,{payload}"))
        }
        "file_uri" | "local_path" => resolved.trace.normalized_uri,
        "data_uri" => Some(source.trim().to_string()),
        _ => resolved.trace.normalized_uri,
    }
}

pub fn resolve_image_asset(bundle: Option<&AssetBundle>, source: &str) -> ResolvedAsset {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return ResolvedAsset {
            trace: AssetResolutionTrace {
                source_uri: display_source_uri(trimmed),
                normalized_uri: None,
                resolver: "empty".to_string(),
                success: false,
                mime: None,
                content_kind: "unknown".to_string(),
                render_outcome: "unresolved".to_string(),
                asset_name: None,
                message: Some("image source is empty".to_string()),
            },
            bytes: Vec::new(),
        };
    }

    if let Some((mime, data)) = parse_data_uri_bytes(trimmed) {
        let content_kind = content_kind_for(Some(&mime), trimmed, &data);
        return ResolvedAsset {
            trace: AssetResolutionTrace {
                source_uri: display_source_uri(trimmed),
                normalized_uri: Some(display_source_uri(trimmed)),
                resolver: "data_uri".to_string(),
                success: true,
                mime: Some(mime.clone()),
                content_kind: content_kind.to_string(),
                render_outcome: render_outcome_for(&content_kind).to_string(),
                asset_name: None,
                message: None,
            },
            bytes: data,
        };
    }

    if let Some(bundle) = bundle {
        if let Some(asset) = bundle_lookup(bundle, trimmed) {
            let mime = asset
                .source
                .as_deref()
                .and_then(infer_image_mime_from_label)
                .or_else(|| infer_image_mime_from_label(&asset.name));
            let content_kind = content_kind_for(mime.as_deref(), &asset.name, &asset.data);
            return ResolvedAsset {
                trace: AssetResolutionTrace {
                    source_uri: display_source_uri(trimmed),
                    normalized_uri: Some(format!("bundle://{}", asset.name)),
                    resolver: "bundle".to_string(),
                    success: true,
                    mime,
                    content_kind: content_kind.to_string(),
                    render_outcome: render_outcome_for(&content_kind).to_string(),
                    asset_name: Some(asset.name.clone()),
                    message: None,
                },
                bytes: asset.data.clone(),
            };
        }
    }

    if let Some(path) = file_uri_to_path_buf(trimmed) {
        return resolve_local_path(trimmed, path, "file_uri");
    }

    resolve_local_path(trimmed, PathBuf::from(trimmed), "local_path")
}

fn resolve_local_path(source: &str, path: PathBuf, resolver: &str) -> ResolvedAsset {
    let normalized = path.to_string_lossy().to_string();
    match std::fs::read(&path) {
        Ok(bytes) => {
            let mime = infer_image_mime_from_label(&normalized);
            let content_kind = content_kind_for(mime.as_deref(), &normalized, &bytes);
            ResolvedAsset {
                trace: AssetResolutionTrace {
                    source_uri: display_source_uri(source),
                    normalized_uri: Some(normalized),
                    resolver: resolver.to_string(),
                    success: true,
                    mime,
                    content_kind: content_kind.to_string(),
                    render_outcome: render_outcome_for(&content_kind).to_string(),
                    asset_name: None,
                    message: None,
                },
                bytes,
            }
        }
        Err(err) => ResolvedAsset {
            trace: AssetResolutionTrace {
                source_uri: display_source_uri(source),
                normalized_uri: Some(normalized),
                resolver: resolver.to_string(),
                success: false,
                mime: infer_image_mime_from_label(source),
                content_kind: "unknown".to_string(),
                render_outcome: "unresolved".to_string(),
                asset_name: None,
                message: Some(err.to_string()),
            },
            bytes: Vec::new(),
        },
    }
}

fn bundle_lookup<'a>(bundle: &'a AssetBundle, source: &str) -> Option<&'a Asset> {
    let keys = lookup_keys(source);
    bundle.image_assets().find(|asset| {
        asset_lookup_keys(asset)
            .iter()
            .any(|key| keys.contains(key))
    })
}

fn asset_lookup_keys(asset: &Asset) -> Vec<String> {
    let mut keys = vec![normalize_lookup_key(&asset.name)];
    if let Some(source) = asset.source.as_deref() {
        keys.extend(lookup_keys(source));
    }
    keys.sort();
    keys.dedup();
    keys
}

fn lookup_keys(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return out;
    }
    out.push(normalize_lookup_key(trimmed));
    if let Some(path) = file_uri_to_path_buf(trimmed) {
        let normalized = path.to_string_lossy().to_string();
        out.push(normalize_lookup_key(&normalized));
        if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
            out.push(normalize_lookup_key(name));
        }
    } else {
        let path = Path::new(trimmed);
        if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
            out.push(normalize_lookup_key(name));
        }
    }
    out.sort();
    out.dedup();
    out
}

fn normalize_lookup_key(value: &str) -> String {
    value.trim().replace('\\', "/").to_ascii_lowercase()
}

fn percent_decode_lossy(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'%' && idx + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (from_hex(bytes[idx + 1]), from_hex(bytes[idx + 2])) {
                out.push((hi << 4) | lo);
                idx += 3;
                continue;
            }
        }
        out.push(bytes[idx]);
        idx += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(ch: u8) -> Option<u8> {
    match ch {
        b'0'..=b'9' => Some(ch - b'0'),
        b'a'..=b'f' => Some(ch - b'a' + 10),
        b'A'..=b'F' => Some(ch - b'A' + 10),
        _ => None,
    }
}

fn infer_image_mime_from_label(label: &str) -> Option<String> {
    let lower = label
        .split('#')
        .next()
        .unwrap_or(label)
        .split('?')
        .next()
        .unwrap_or(label)
        .to_ascii_lowercase();
    if lower.ends_with(".png") {
        Some("image/png".to_string())
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        Some("image/jpeg".to_string())
    } else if lower.ends_with(".gif") {
        Some("image/gif".to_string())
    } else if lower.ends_with(".webp") {
        Some("image/webp".to_string())
    } else if lower.ends_with(".bmp") {
        Some("image/bmp".to_string())
    } else if lower.ends_with(".svg") || lower.ends_with(".svgz") {
        Some("image/svg+xml".to_string())
    } else {
        None
    }
}

fn content_kind_for(mime: Option<&str>, label: &str, bytes: &[u8]) -> String {
    if mime.is_some_and(|value| value.contains("svg"))
        || label.to_ascii_lowercase().ends_with(".svg")
    {
        return "svg".to_string();
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        if text
            .trim_start_matches('\u{feff}')
            .trim_start()
            .starts_with("<svg")
        {
            return "svg".to_string();
        }
    }
    if image::guess_format(bytes).is_ok() {
        return "raster_image".to_string();
    }
    "unknown".to_string()
}

fn render_outcome_for(content_kind: &str) -> &'static str {
    match content_kind {
        "svg" => "vector_svg",
        "raster_image" => "raster_image",
        _ => "unsupported",
    }
}

fn display_source_uri(source: &str) -> String {
    if let Some((mime, _)) = parse_data_uri_bytes(source) {
        if source.contains(";base64,") {
            return format!("data:{mime};base64,<payload>");
        }
        return format!("data:{mime},<payload>");
    }
    source.to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        Asset, AssetBundle, AssetKind, file_uri_to_path_buf, parse_data_uri_bytes,
        renderable_image_source, resolve_image_asset,
    };
    use base64::Engine;
    use std::path::Path;

    #[test]
    fn asset_kind_pdf_roundtrip() {
        assert_eq!(AssetKind::from_str("pdf"), Some(AssetKind::Pdf));
        assert_eq!(AssetKind::Pdf.as_str(), "pdf");
    }

    #[test]
    fn supported_font_path_accepts_ttf_and_otf() {
        assert!(super::is_supported_font_path(Path::new("demo.ttf")));
        assert!(super::is_supported_font_path(Path::new("demo.otf")));
        assert!(!super::is_supported_font_path(Path::new("demo.woff2")));
    }

    #[test]
    fn parse_data_uri_decodes_base64_payload() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"png-bytes");
        let uri = format!("data:image/png;base64,{encoded}");
        let (mime, bytes) = parse_data_uri_bytes(&uri).expect("data uri");
        assert_eq!(mime, "image/png");
        assert_eq!(bytes, b"png-bytes");
    }

    #[test]
    fn file_uri_to_path_buf_strips_scheme() {
        let path = file_uri_to_path_buf("file:///tmp/demo.png").expect("file uri");
        assert!(
            path.to_string_lossy().contains("demo.png"),
            "expected demo path, got {}",
            path.to_string_lossy()
        );
    }

    #[test]
    fn bundle_image_resolves_bundle_first() {
        let mut bundle = AssetBundle::default();
        bundle.add(Asset::new(
            "diagram.png".to_string(),
            AssetKind::Image,
            b"fakepng".to_vec(),
            Some("assets/diagram.png".to_string()),
            false,
        ));
        let resolved = resolve_image_asset(Some(&bundle), "diagram.png");
        assert!(resolved.trace.success);
        assert_eq!(resolved.trace.resolver, "bundle");
        assert_eq!(resolved.trace.asset_name.as_deref(), Some("diagram.png"));
        let renderable = renderable_image_source(Some(&bundle), "diagram.png").expect("renderable");
        assert!(renderable.starts_with("data:image/png;base64,"));
    }
}
