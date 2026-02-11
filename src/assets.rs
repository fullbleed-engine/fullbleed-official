#[cfg(feature = "python")]
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Css,
    Font,
    Image,
    Svg,
    Other,
}

impl AssetKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AssetKind::Css => "css",
            AssetKind::Font => "font",
            AssetKind::Image => "image",
            AssetKind::Svg => "svg",
            AssetKind::Other => "other",
        }
    }

    pub fn from_str(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "css" => Some(AssetKind::Css),
            "font" => Some(AssetKind::Font),
            "image" => Some(AssetKind::Image),
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
}

#[cfg(feature = "python")]
pub fn is_supported_font_path(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|v| v.to_str()) else {
        return false;
    };
    matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf")
}
