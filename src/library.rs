//! Scans the user's library folders. Produces a catalog of every local chart (used by the
//! Library page and the tools) and the "artist|name|charter" keys used to mark search results
//! "Already in your library" (matched the same way as the original Bridge).

use crate::api::library_key;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Worker threads for reading folders.
const THREADS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChartType {
    Chart,
    Mid,
    Sng,
    None,
}

impl ChartType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Chart => ".chart",
            Self::Mid => ".mid",
            Self::Sng => ".sng",
            Self::None => "—",
        }
    }
}

/// Tagged originals of a library chart's text (see `LocalChart::tagged`).
#[derive(Debug, Clone)]
pub struct Tagged {
    pub name: String,
    pub artist: String,
    pub charter: String,
}

/// One chart in the user's library.
#[derive(Debug, Clone)]
pub struct LocalChart {
    /// Chart folder, or the .sng file.
    pub path: PathBuf,
    pub name: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub year: String,
    pub charter: String,
    /// The name, artist and charter as written in song.ini, kept only when one of them has
    /// colour/formatting tags (drawn styled on the Library page).
    pub tagged: Option<Box<Tagged>>,
    /// Raw (un-stripped) artist|name|charter key.
    pub key: String,
    pub song_length_ms: u64,
    pub chart_type: ChartType,
    /// File name of notes.chart / notes.mid inside the folder (or .sng).
    pub chart_file: Option<String>,
    pub has_video: bool,
    pub has_background: bool,
    pub has_album_art: bool,
    pub has_audio: bool,
    pub has_guitar: bool,
    pub has_bass: bool,
    pub has_drums: bool,
    pub has_keys: bool,
    pub has_vocals: bool,
    /// Last modified time of the chart file (seconds since the epoch), for caches.
    pub modified: u64,
    /// When the chart arrived in the library (seconds since the epoch): when its folder or .sng
    /// was created, which is when it was downloaded or copied in.
    pub added: u64,
}

impl LocalChart {
    pub fn is_sng(&self) -> bool {
        self.chart_type == ChartType::Sng
    }

    /// The notes.chart / notes.mid file of a folder chart.
    pub fn chart_path(&self) -> Option<PathBuf> {
        if self.is_sng() {
            return None;
        }
        self.chart_file.as_ref().map(|f| self.path.join(f))
    }
}

#[derive(Debug, Default, Clone)]
pub struct Library {
    /// Library key -> every chart folder / .sng file with that key.
    pub keys: HashMap<String, Vec<PathBuf>>,
    pub charts: usize,
    pub catalog: Vec<Arc<LocalChart>>,
}

pub async fn scan(folders: Vec<PathBuf>) -> Arc<Library> {
    tokio::task::spawn_blocking(move || Arc::new(scan_blocking(&folders))).await.unwrap_or_default()
}

pub fn scan_blocking(folders: &[PathBuf]) -> Library {
    let mut lib = Library::default();
    let mut frontier: Vec<PathBuf> = folders.iter().filter(|f| f.is_dir()).cloned().collect();
    // Breadth-first, one directory level at a time, each level split across a few threads.
    while !frontier.is_empty() {
        let chunk = frontier.len().div_ceil(THREADS).max(1);
        let results: Vec<(Vec<PathBuf>, Vec<LocalChart>)> = std::thread::scope(|s| {
            let handles: Vec<_> = frontier
                .chunks(chunk)
                .map(|dirs| {
                    s.spawn(move || {
                        dirs.iter().fold((Vec::new(), Vec::new()), |mut acc, d| {
                            visit(d, &mut acc.0, &mut acc.1);
                            acc
                        })
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        frontier = Vec::new();
        for (subdirs, charts) in results {
            frontier.extend(subdirs);
            for c in charts {
                lib.keys.entry(c.key.clone()).or_default().push(c.path.clone());
                lib.catalog.push(Arc::new(c));
            }
        }
    }
    lib.charts = lib.catalog.len();
    // By artist then song; charts without an artist go last.
    lib.catalog.sort_by_cached_key(|c| (c.artist.trim().is_empty(), c.artist.to_lowercase(), c.name.to_lowercase()));
    lib
}

fn ext_of(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}
fn stem_of(name: &str) -> String {
    name.rsplit_once('.').map(|(s, _)| s.to_ascii_lowercase()).unwrap_or_else(|| name.to_ascii_lowercase())
}
const AUDIO_EXT: &[&str] = &["ogg", "mp3", "wav", "opus"];
const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg"];
const VIDEO_EXT: &[&str] = &["mp4", "avi", "webm", "vp8", "ogv", "mpeg"];

struct Assets {
    chart_type: ChartType,
    chart_file: Option<String>,
    has_video: bool,
    has_background: bool,
    has_album_art: bool,
    has_audio: bool,
}

fn assets_from_names<'a>(names: impl Iterator<Item = &'a str>) -> Assets {
    let mut a = Assets {
        chart_type: ChartType::None,
        chart_file: None,
        has_video: false,
        has_background: false,
        has_album_art: false,
        has_audio: false,
    };
    for name in names {
        let (stem, ext) = (stem_of(name), ext_of(name));
        match ext.as_str() {
            "chart" | "mid" => {
                // Prefer notes.mid over notes.chart, like scan-chart.
                let better = a.chart_file.is_none() || (stem == "notes" && (ext == "mid" || a.chart_type != ChartType::Mid));
                if better {
                    a.chart_type = if ext == "mid" { ChartType::Mid } else { ChartType::Chart };
                    a.chart_file = Some(name.to_string());
                }
            }
            e if AUDIO_EXT.contains(&e) && stem != "preview" && stem != "crowd" => a.has_audio = true,
            e if IMAGE_EXT.contains(&e) && stem == "album" => a.has_album_art = true,
            e if IMAGE_EXT.contains(&e) && stem.starts_with("background") => a.has_background = true,
            e if VIDEO_EXT.contains(&e) && stem == "video" => a.has_video = true,
            _ => {}
        }
    }
    a
}

/// Reads one directory: a chart folder yields one chart; otherwise its subfolders are queued.
/// `.sng` files are read wherever they are.
fn visit(dir: &Path, subdirs: &mut Vec<PathBuf>, charts: &mut Vec<LocalChart>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut sngs = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            // Unfinished downloads (".bridge-rs-tmp-" from before the rename).
            if !name.starts_with(".truss-tmp-") && !name.starts_with(".bridge-rs-tmp-") {
                dirs.push(e.path());
            }
        } else if ext_of(&name) == "sng" {
            sngs.push((e.path(), e.metadata().ok()));
        } else {
            files.push(name);
        }
    }
    let ini = files.iter().find(|n| n.eq_ignore_ascii_case("song.ini"));
    let assets = assets_from_names(files.iter().map(String::as_str));
    if ini.is_some() || assets.chart_file.is_some() {
        // A chart folder: its subfolders don't hold other charts.
        let pairs = ini
            .and_then(|i| std::fs::read(dir.join(i)).ok())
            .map(|b| parse_ini(&decode(&b)))
            .unwrap_or_default();
        let modified = assets
            .chart_file
            .as_ref()
            .and_then(|f| std::fs::metadata(dir.join(f)).ok())
            .and_then(|m| secs(m.modified()))
            .unwrap_or(0);
        let added = std::fs::metadata(dir).ok().and_then(|m| secs(m.created())).unwrap_or(modified);
        charts.push(make_chart(dir.to_path_buf(), &pairs, assets, modified, added));
    } else {
        subdirs.extend(dirs);
    }
    for (sng, meta) in sngs {
        if let Ok(index) = crate::sng::read_index(&sng) {
            let mut assets = assets_from_names(index.files.iter().map(|f| f.name.as_str()));
            assets.chart_type = ChartType::Sng;
            let modified = meta.as_ref().and_then(|m| secs(m.modified())).unwrap_or(0);
            let added = meta.as_ref().and_then(|m| secs(m.created())).unwrap_or(modified);
            charts.push(make_chart(sng, &index.metadata, assets, modified, added));
        }
    }
}

/// A chart for other modules' tests.
#[cfg(test)]
pub fn test_chart(artist: &str, name: &str, added: u64) -> LocalChart {
    let pairs = [("artist".to_string(), artist.to_string()), ("name".to_string(), name.to_string())];
    make_chart(PathBuf::from(format!("{artist} - {name}")), &pairs, assets_from_names(["notes.chart"].into_iter()), 0, added)
}

fn secs(t: std::io::Result<std::time::SystemTime>) -> Option<u64> {
    t.ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
}

fn make_chart(path: PathBuf, pairs: &[(String, String)], a: Assets, modified: u64, added: u64) -> LocalChart {
    let get = |k: &str| pairs.iter().find(|(pk, _)| pk.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str()).unwrap_or("");
    let charter = if get("charter").is_empty() { get("frets") } else { get("charter") };
    let has = |k: &str| get(k).trim().parse::<f64>().is_ok_and(|d| d >= 0.0);
    let clean = |s: &str| crate::api::strip_tags(s);
    let fallback_name = path
        .file_name()
        .map(|n| n.to_string_lossy().trim_end_matches(".sng").to_string())
        .unwrap_or_default();
    LocalChart {
        key: library_key(get("artist"), get("name"), charter),
        name: if get("name").is_empty() { fallback_name } else { clean(get("name")) },
        artist: clean(get("artist")),
        album: clean(get("album")),
        genre: clean(get("genre")),
        year: clean(get("year")),
        charter: clean(charter),
        tagged: [get("name"), get("artist"), charter].iter().any(|v| v.contains('<')).then(|| {
            Box::new(Tagged { name: get("name").to_string(), artist: get("artist").to_string(), charter: charter.to_string() })
        }),
        song_length_ms: get("song_length").trim().parse::<f64>().map(|v| v.max(0.0) as u64).unwrap_or(0),
        chart_type: a.chart_type,
        chart_file: a.chart_file,
        has_video: a.has_video,
        has_background: a.has_background,
        has_album_art: a.has_album_art,
        has_audio: a.has_audio,
        has_guitar: has("diff_guitar"),
        has_bass: has("diff_bass"),
        has_drums: has("diff_drums"),
        has_keys: has("diff_keys"),
        has_vocals: has("diff_vocals"),
        modified,
        added,
        path,
    }
}

/// song.ini text: UTF-8 (with or without BOM), UTF-16 with BOM, else Latin-1.
pub fn decode(b: &[u8]) -> String {
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if b.len() >= 2 && (b[..2] == [0xFF, 0xFE] || b[..2] == [0xFE, 0xFF]) {
        let le = b[0] == 0xFF;
        let units: Vec<u16> = b[2..]
            .chunks_exact(2)
            .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
            .collect();
        return String::from_utf16_lossy(&units);
    }
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}

/// True if `path` is a chart inside one of the library `roots`: strictly below a root (never a
/// root itself), and either a `.sng` file or a folder that holds a song.ini or notes file.
/// Guards every delete so a bad path can never take a whole songs folder with it.
pub fn is_deletable_chart(path: &Path, roots: &[PathBuf]) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let p = norm(path);
    let inside = roots.iter().any(|r| {
        let r = norm(r);
        p.len() > r.len() + 1 && p.starts_with(&r) && p.as_bytes()[r.len()] == b'\\'
    });
    if !inside {
        return false;
    }
    if path.is_file() {
        return p.ends_with(".sng");
    }
    path.is_dir()
        && std::fs::read_dir(path)
            .map(|rd| {
                rd.flatten().any(|e| {
                    let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                    n == "song.ini" || n == "notes.chart" || n == "notes.mid"
                })
            })
            .unwrap_or(false)
}

/// Moves files or folders to the Recycle Bin (so a mistaken delete can be undone).
pub async fn recycle(paths: Vec<PathBuf>) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    tokio::task::spawn_blocking(move || trash::delete_all(&paths).map_err(|e| format!("Couldn't delete: {e}")))
        .await
        .map_err(|e| e.to_string())?
}

/// `key = value` pairs from the `[song]` section of a song.ini.
pub fn parse_ini(text: &str) -> Vec<(String, String)> {
    let mut in_song = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.starts_with('[') {
            in_song = line.trim_matches(|c| c == '[' || c == ']').trim().eq_ignore_ascii_case("song");
        } else if in_song && let Some((k, v)) = line.split_once('=') {
            out.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_key() {
        let ini = "\u{feff}[Song]\r\nname = Through the Fire and Flames\r\nartist=DragonForce\r\nfrets = Neversoft\r\n[other]\r\nname = nope\r\n";
        let c = make_chart(PathBuf::from("x"), &parse_ini(ini), assets_from_names(["notes.mid", "song.ogg"].into_iter()), 0, 0);
        assert_eq!(c.key, "dragonforce|through the fire and flames|neversoft");
        assert_eq!(c.chart_type, ChartType::Mid);
        assert!(c.has_audio && !c.has_album_art);
    }

    /// Scans a real library when TRUSS_TEST_LIBRARY is set and prints timing.
    #[test]
    fn scan_real_library() {
        let Some(dir) = std::env::var_os("TRUSS_TEST_LIBRARY") else { return };
        let t = std::time::Instant::now();
        let root = PathBuf::from(dir);
        let lib = scan_blocking(std::slice::from_ref(&root));
        let some = lib.keys.values().next().unwrap()[0].clone();
        assert!(is_deletable_chart(&some, std::slice::from_ref(&root)));
        assert!(!is_deletable_chart(&root, std::slice::from_ref(&root)));
        assert!(!is_deletable_chart(root.parent().unwrap(), std::slice::from_ref(&root)));
        let c = &lib.catalog;
        println!(
            "{} charts ({} unique) in {:?}; no art {}, no bg {}, no video {}, sng {}",
            lib.charts,
            lib.keys.len(),
            t.elapsed(),
            c.iter().filter(|c| !c.has_album_art).count(),
            c.iter().filter(|c| !c.has_background).count(),
            c.iter().filter(|c| !c.has_video).count(),
            c.iter().filter(|c| c.is_sng()).count(),
        );
    }
}
