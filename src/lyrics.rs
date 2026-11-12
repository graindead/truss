//! Which library charts have lyrics (shown on the Library page), with a disk cache.

use crate::library::LocalChart;
use std::collections::HashMap;
use std::path::PathBuf;

// ---------- Detection (with a disk cache) ----------

/// True if the chart's notes file contains lyrics.
pub fn detect(c: &LocalChart) -> bool {
    let data = if c.is_sng() {
        let Ok(idx) = crate::sng::read_index(&c.path) else { return false };
        let Some(e) = idx.files.iter().find(|f| {
            let n = f.name.to_ascii_lowercase();
            n.ends_with(".chart") || n.ends_with(".mid")
        }) else {
            return false;
        };
        let is_mid = e.name.to_ascii_lowercase().ends_with(".mid");
        let Ok(d) = crate::sng::read_entry(&c.path, &idx, e) else { return false };
        return if is_mid { mid_has_lyrics(&d) } else { memchr_find(&d, b"E \"lyric ") };
    } else {
        match c.chart_path().and_then(|p| std::fs::read(p).ok()) {
            Some(d) => d,
            None => return false,
        }
    };
    if c.chart_type == crate::library::ChartType::Mid { mid_has_lyrics(&data) } else { memchr_find(&data, b"E \"lyric ") }
}

fn memchr_find(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn mid_has_lyrics(data: &[u8]) -> bool {
    use midly::{MetaMessage, Smf, TrackEventKind};
    let Ok(smf) = Smf::parse(data) else { return false };
    smf.tracks.iter().any(|t| {
        let vocals = t.iter().take_while(|e| e.delta.as_int() == 0).any(|e| {
            matches!(e.kind, TrackEventKind::Meta(MetaMessage::TrackName(n)) if n == b"PART VOCALS")
        });
        vocals
            && t.iter().any(|e| {
                matches!(e.kind, TrackEventKind::Meta(MetaMessage::Lyric(b) | MetaMessage::Text(b))
                    if !b.is_empty() && b[0] != b'[')
            })
    })
}

fn cache_file() -> Option<PathBuf> {
    crate::settings::project_dirs().map(|d| d.cache_dir().join("lyrics-scan.json"))
}

/// Which charts have lyrics, by path. Uses a cache keyed on the chart file's modified time, so
/// only new or changed charts are read.
pub async fn scan(charts: Vec<std::sync::Arc<LocalChart>>) -> HashMap<PathBuf, bool> {
    tokio::task::spawn_blocking(move || {
        let cached: HashMap<PathBuf, (u64, bool)> = cache_file()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Vec<(PathBuf, u64, bool)>>(&b).ok())
            .map(|v| v.into_iter().map(|(p, m, h)| (p, (m, h))).collect())
            .unwrap_or_default();
        let todo: Vec<&std::sync::Arc<LocalChart>> =
            charts.iter().filter(|c| cached.get(&c.path).is_none_or(|(m, _)| *m != c.modified)).collect();
        let fresh: Vec<(PathBuf, u64, bool)> = std::thread::scope(|s| {
            let chunk = todo.len().div_ceil(4).max(1);
            let hs: Vec<_> = todo
                .chunks(chunk)
                .map(|part| s.spawn(move || part.iter().map(|c| (c.path.clone(), c.modified, detect(c))).collect::<Vec<_>>()))
                .collect();
            hs.into_iter().filter_map(|h| h.join().ok()).flatten().collect()
        });
        let mut all: HashMap<PathBuf, (u64, bool)> = cached;
        for (p, m, h) in fresh {
            all.insert(p, (m, h));
        }
        let keep: std::collections::HashSet<&PathBuf> = charts.iter().map(|c| &c.path).collect();
        all.retain(|p, _| keep.contains(p));
        if let Some(f) = cache_file() {
            let v: Vec<(&PathBuf, u64, bool)> = all.iter().map(|(p, (m, h))| (p, *m, *h)).collect();
            if let Some(parent) = f.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(f, serde_json::to_vec(&v).unwrap_or_default());
        }
        all.into_iter().map(|(p, (_, h))| (p, h)).collect()
    })
    .await
    .unwrap_or_default()
}
