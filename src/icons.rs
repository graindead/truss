//! Charter / pack icons shown in the corner of album art (same source and matching rules as the
//! original Bridge: https://clonehero.gitlab.io/sources/icons.json).

use crate::api::{Chart, strip_tags};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

const BASE: &str = "https://clonehero.gitlab.io/sources";

#[derive(Debug, Clone, Deserialize)]
pub struct IconEntry {
    pub name: String,
    pub sha1: String,
}

fn cache_dir() -> Option<PathBuf> {
    crate::settings::cache_dir().map(|d| d.join("icons"))
}

/// Downloads the icon list, falling back to the last saved copy when offline.
pub async fn fetch_list() -> Result<Arc<Vec<IconEntry>>, String> {
    let cached = cache_dir().map(|d| d.join("icons.json"));
    let fresh = crate::api::get_bytes(&format!("{BASE}/icons.json")).await.map_err(|e| e.to_string());
    let bytes = match fresh {
        Ok(b) => {
            if let Some(p) = &cached {
                crate::api::save_quietly(p, &b).await;
            }
            b
        }
        Err(e) => match &cached {
            Some(p) => tokio::fs::read(p).await.map_err(|_| e)?.into(),
            None => return Err(e),
        },
    };
    serde_json::from_slice(&bytes).map(Arc::new).map_err(|e| e.to_string())
}

/// The icon for a chart: its `icon` field (packs/setlists) or else its charter's name, matched
/// case-insensitively against icon file names, exactly like the original Bridge.
pub fn resolve<'a>(list: &'a [IconEntry], chart: &Chart) -> Option<&'a IconEntry> {
    let base = match chart.icon.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(icon) => icon.to_lowercase(),
        None => strip_tags(chart.charter.as_deref().unwrap_or("N/A")).to_lowercase(),
    };
    if base == "unknown charter" {
        return None;
    }
    let prefix = format!("{base}.");
    list.iter().find(|i| i.name.to_lowercase().starts_with(&prefix))
}

/// Tooltip text: the pack's full name for pack icons, otherwise the charter.
pub fn tooltip(chart: &Chart) -> String {
    match chart.icon.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(icon) => match crate::setlists::setlist_name(icon) {
            Some(name) => format!("Pack: {name}"),
            None => format!("Icon: {icon}"),
        },
        None => format!("Charter: {}", strip_tags(chart.charter.as_deref().unwrap_or(""))),
    }
}

/// Icon image bytes, from the disk cache (keyed by content hash, so updated icons refresh
/// automatically) or the network.
pub async fn fetch_icon(entry: IconEntry) -> Result<bytes::Bytes, String> {
    let ext = entry.name.rsplit('.').next().unwrap_or("png").to_ascii_lowercase();
    let path = cache_dir().map(|d| d.join(format!("{}.{ext}", entry.sha1)));
    crate::api::cached_get(&format!("{BASE}/icons/{}", entry.name), path).await.map_err(|e| e.to_string())
}
