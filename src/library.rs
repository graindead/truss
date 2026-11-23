//! Scans the user's library folders. Produces a catalog of every local chart (used by the
//! Library page and the tools) and the "artist|name|charter" keys used to mark search results
//! "Already in your library" (matched the same way as the original Bridge).

use crate::api::library_key;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use futures::{SinkExt, channel::mpsc::Sender};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Worker threads for reading folders.
const THREADS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tagged {
    pub name: String,
    pub artist: String,
    pub charter: String,
}

/// One chart in the user's library.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Modified time and size of the song.ini (or .sng) this was read from; a rescan reuses the
    /// chart instead of reading the file again while it's unchanged. 0 when unknown.
    #[serde(default)]
    pub stamp: u64,
    /// The chart file's hash as Chorus Encore lists it (`chartHash`), so a copy whose song.ini
    /// reads differently (from a pack, say) still counts as this chart.
    #[serde(default)]
    pub chart_hash: Option<String>,
    /// Lead guitar's difficulty rating from song.ini (0 to 6, sometimes more), if it has one.
    #[serde(default)]
    pub diff_guitar: Option<i8>,
    /// The chart folder's own modified time when it was read (nanoseconds; 0 unknown). It
    /// changes when a file in it is added, removed or renamed, so while it's the same a quick
    /// scan reuses the chart without opening the folder.
    #[serde(default)]
    pub folder_stamp: u64,
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

/// A chart's folder (or .sng file) as the library's lookups hold it: one copy, shared by all of
/// them (they'd otherwise hold three copies of every path; with tens of thousands of charts
/// that's megabytes).
pub type LibPath = Arc<Path>;

#[derive(Debug, Default, Clone)]
pub struct Library {
    /// Library key -> every chart folder / .sng file with that key.
    pub keys: HashMap<String, Vec<LibPath>>,
    /// Chart file hash -> every chart folder / .sng file with that chart.
    pub hashes: HashMap<String, Vec<LibPath>>,
    /// `song_key` -> the artist of each chart with it (its words, joined by spaces): another
    /// version of a song (an updated chart, or a pack's copy with its song.ini written
    /// differently).
    pub songs: HashMap<String, Vec<(Box<str>, LibPath)>>,
    pub charts: usize,
    pub catalog: Vec<Arc<LocalChart>>,
}

impl Library {
    fn from_catalog(catalog: Vec<Arc<LocalChart>>) -> Self {
        let mut lib = Library::default();
        for c in &catalog {
            lib.add(c);
        }
        lib.charts = catalog.len();
        lib.catalog = catalog;
        lib
    }

    fn add(&mut self, c: &LocalChart) {
        // Almost every key has one chart: room for one, not the four a Vec starts with.
        let path: LibPath = Arc::from(c.path.as_path());
        self.keys.entry(c.key.clone()).or_insert_with(|| Vec::with_capacity(1)).push(path.clone());
        if let Some(h) = &c.chart_hash {
            self.hashes.entry(h.clone()).or_insert_with(|| Vec::with_capacity(1)).push(path.clone());
        }
        let raw = |t: &Option<Box<Tagged>>, f: fn(&Tagged) -> &str, plain: &str| t.as_deref().map(f).unwrap_or(plain).to_string();
        let name = raw(&c.tagged, |t| &t.name, &c.name);
        let charter = raw(&c.tagged, |t| &t.charter, &c.charter);
        if let Some(k) = song_key(&name, &charter) {
            self.songs.entry(k).or_insert_with(|| Vec::with_capacity(1)).push((artist_words(&c.artist).join(" ").into_boxed_str(), path));
        }
    }
}

/// A song's title and charter with the details versions of a chart tend to disagree on
/// dropped: formatting tags, "feat." credits, punctuation, and the order of the charters.
/// `None` without a title or charter.
pub fn song_key(name: &str, charter: &str) -> Option<String> {
    let title = words(&drop_feat(&crate::api::strip_tags(name).to_lowercase())).join(" ");
    let charter = crate::api::strip_tags(charter).to_lowercase().replace(" and ", ",");
    let mut charters: Vec<String> =
        charter.split([',', '&', '/', '+']).map(|c| words(c).join(" ")).filter(|c| !c.is_empty()).collect();
    charters.sort();
    charters.dedup();
    (!title.is_empty() && !charters.is_empty()).then(|| format!("{title}|{}", charters.join("+")))
}

/// The words of an artist without any "feat." credit, for a loose comparison ("Hikaru Utada"
/// and "Utada Hikaru (feat. Skrillex)" share theirs).
pub fn artist_words(artist: &str) -> Vec<String> {
    words(&drop_feat(&crate::api::strip_tags(artist).to_lowercase()))
}

/// Whether two charts' artists (from `artist_words`) could be the same: they share a word, or
/// one is blank.
pub fn same_artist(a: &[String], b: &[String]) -> bool {
    a.is_empty() || b.is_empty() || a.iter().any(|w| b.contains(w))
}

/// Lower-case text without "(feat. X)" / "[ft. X]" groups or a trailing "feat. X".
fn drop_feat(s: &str) -> String {
    let is_feat = |t: &str| ["feat.", "feat ", "ft.", "ft ", "featuring "].iter().any(|p| t.trim_start().starts_with(p));
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(['(', '[']) {
        let close = if rest.as_bytes()[i] == b'(' { ')' } else { ']' };
        out.push_str(&rest[..i]);
        match rest[i + 1..].find(close) {
            Some(j) => {
                let inner = &rest[i + 1..i + 1 + j];
                if !is_feat(inner) {
                    out.push_str(&rest[i..i + 2 + j]);
                }
                rest = &rest[i + 2 + j..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    for p in [" feat. ", " feat ", " ft. ", " ft ", " featuring "] {
        if let Some(i) = out.find(p) {
            out.truncate(i);
        }
    }
    out
}

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// The note hashes of every track of a library chart (as Encore lists them in `trackHashes`,
/// without padding), sorted; empty if it can't be read.
pub fn track_hashes(path: &Path) -> Vec<String> {
    let computed = if path.is_dir() {
        let files: Vec<(String, Vec<u8>)> = std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                let l = n.to_ascii_lowercase();
                (l.ends_with(".chart") || l.ends_with(".mid") || l.ends_with(".ini")).then_some(n)
            })
            .filter_map(|n| Some((n.clone(), std::fs::read(path.join(&n)).ok()?)))
            .collect();
        crate::trackhash::hash_tracks(&files, None)
    } else {
        crate::sng::read_index(path).and_then(|idx| {
            let files = idx
                .files
                .iter()
                .filter(|f| {
                    let l = f.name.to_ascii_lowercase();
                    l.ends_with(".chart") || l.ends_with(".mid")
                })
                .map(|f| Ok((f.name.clone(), crate::sng::read_entry(path, &idx, f)?)))
                .collect::<Result<Vec<_>, String>>()?;
            crate::trackhash::hash_tracks(&files, Some(&idx.metadata))
        })
    };
    // instrument/difficulty/hash, so two charts only match track for track.
    let mut out: Vec<String> =
        computed.map(|c| c.into_iter().map(|(i, d, h)| format!("{i}/{d}/{}", h.trim_end_matches('='))).collect()).unwrap_or_default();
    out.sort();
    out
}

/// Encore's `chartHash`: blake3 of the chart file, in padded URL-safe base64.
pub fn chart_hash(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE.encode(blake3::hash(bytes).as_bytes())
}

#[derive(Debug, Clone)]
pub enum ScanEvent {
    /// The library as it was at the end of the last scan (shown while this one runs) and its
    /// chart count, used as the expected total for progress.
    Cached(Arc<Library>),
    /// Charts found so far.
    Found(usize),
    Done(Arc<Library>),
}

/// Scans the library folders, reporting progress. With `use_cache`, first sends the library
/// saved by the last scan of the same folders so it shows straight away.
/// Scans the library folders. A quick scan (`full` false) opens only chart folders that are new
/// or whose date changed, reusing the rest from the last scan; it becomes a full one when the
/// last full scan is a week old, the cache is from another version, or there's no cache.
pub fn scan(folders: Vec<PathBuf>, use_cache: bool, full: bool) -> impl futures::Stream<Item = ScanEvent> {
    iced::stream::channel(8, async move |mut tx: Sender<ScanEvent>| {
        // The last scan's charts: shown straight away at start-up, and reused by this scan for
        // every chart whose song.ini (or .sng) hasn't changed.
        let cache = tokio::task::spawn_blocking(load_cache).await.ok().flatten();
        let now = now_secs();
        let quick = !full
            && cache.as_ref().is_some_and(|c| c.version == CACHE_VERSION && c.folders == folders && now.saturating_sub(c.full_scan) < FULL_SCAN_EVERY);
        let full_scan = if quick { cache.as_ref().map_or(0, |c| c.full_scan) } else { now };
        let prev: HashMap<PathBuf, Arc<LocalChart>> = match cache {
            Some(c) => {
                if use_cache && c.folders == folders {
                    let _ = tx.send(ScanEvent::Cached(Arc::new(Library::from_catalog(c.charts.clone())))).await;
                }
                // Charts read by an older version have their song.ini read again, in case the
                // reading changed (their chart files aren't hashed again while unchanged).
                let reuse = c.version == CACHE_VERSION;
                // Every chart (even one without a song.ini, which a quick scan still reuses by its
                // folder's date; a full scan checks the song.ini's own stamp before reusing).
                c.charts
                    .into_iter()
                    .map(|c| {
                        let c = if reuse {
                            c
                        } else {
                            let mut old = LocalChart::clone(&c);
                            old.stamp = STALE_STAMP;
                            Arc::new(old)
                        };
                        (c.path.clone(), c)
                    })
                    .collect()
            }
            None => HashMap::new(),
        };
        let found = Arc::new(AtomicUsize::new(0));
        let counter = found.clone();
        let mut job = tokio::task::spawn_blocking(move || {
            let lib = scan_with(&folders, &counter, &prev, quick);
            // Saved only when something changed: a quick scan that reused every chart as it was
            // leaves the (19 MB or so) file alone.
            let unchanged = quick && lib.catalog.len() == prev.len() && lib.catalog.iter().all(|c| prev.get(&c.path).is_some_and(|o| Arc::ptr_eq(o, c)));
            drop(prev);
            if !unchanged {
                save_cache(&folders, &lib, full_scan);
            }
            lib
        });
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(120));
        loop {
            tokio::select! {
                lib = &mut job => {
                    let _ = tx.send(ScanEvent::Done(Arc::new(lib.unwrap_or_default()))).await;
                    break;
                }
                _ = tick.tick() => {
                    let _ = tx.send(ScanEvent::Found(found.load(Ordering::Relaxed))).await;
                }
            }
        }
    })
}

#[derive(Serialize, Deserialize)]
struct Cache {
    version: u32,
    folders: Vec<PathBuf>,
    charts: Vec<Arc<LocalChart>>,
    /// When the last full scan was (seconds since the epoch).
    #[serde(default)]
    full_scan: u64,
}

/// How long quick scans are trusted before a full one (which opens every chart folder, to
/// catch a file changed in place, which doesn't change its folder's date).
const FULL_SCAN_EVERY: u64 = 7 * 24 * 3600;

fn now_secs() -> u64 {
    secs(Ok(std::time::SystemTime::now())).unwrap_or(0)
}
/// Bumped when the way charts are read changes, so the next scan reads them all again.
/// 2: a line like `[song] =` no longer starts a new section of song.ini.
const CACHE_VERSION: u32 = 3;

/// A kept chart's stamp when an older version read it: matches no file, so it's read again.
const STALE_STAMP: u64 = u64::MAX;

fn cache_path() -> Option<PathBuf> {
    crate::settings::cache_dir().map(|d| d.join("library.msgpack"))
}

/// Where versions before 0.1.27 kept it (read once, to carry it over).
fn old_cache_path() -> Option<PathBuf> {
    crate::settings::cache_dir().map(|d| d.join("library.json"))
}

fn load_cache() -> Option<Cache> {
    if let Some(c) = cache_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| rmp_serde::from_slice(&b).ok()) {
        return Some(c);
    }
    serde_json::from_slice(&std::fs::read(old_cache_path()?).ok()?).ok()
}

fn save_cache(folders: &[PathBuf], lib: &Library, full_scan: u64) {
    let Some(path) = cache_path() else { return };
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
    let c = Cache { version: CACHE_VERSION, folders: folders.to_vec(), charts: lib.catalog.clone(), full_scan };
    // Written beside it and renamed, so a crash can't leave half a file.
    let tmp = path.with_extension("msgpack.tmp");
    // With field names, so a file written before a field was added still reads.
    let ok = std::fs::File::create(&tmp)
        .ok()
        .and_then(|f| {
            let mut w = std::io::BufWriter::new(f);
            c.serialize(&mut rmp_serde::Serializer::new(&mut w).with_struct_map()).ok()?;
            std::io::Write::flush(&mut w).ok()
        })
        .is_some();
    if ok {
        if std::fs::rename(&tmp, &path).is_ok()
            && let Some(old) = old_cache_path()
        {
            let _ = std::fs::remove_file(old);
        }
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
}

pub fn scan_blocking(folders: &[PathBuf]) -> Library {
    scan_with(folders, &AtomicUsize::new(0), &HashMap::new(), false)
}

/// A directory to read: when it was created and its modified time (nanoseconds), from its
/// parent's listing (none and 0 for the library folders themselves).
type Dir = (PathBuf, Option<u64>, u64);

/// Lowers this thread's processor and disk priority, so a scan never slows what you're doing
/// (or other programs) on a slow drive.
fn lower_priority() {
    #[cfg(windows)]
    // nosemgrep: a Windows API call (Rust requires unsafe for these)
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}

fn scan_with(folders: &[PathBuf], found: &AtomicUsize, prev: &HashMap<PathBuf, Arc<LocalChart>>, quick: bool) -> Library {
    let mut lib = Library::default();
    let mut frontier: Vec<Dir> = folders.iter().filter(|f| f.is_dir()).map(|f| (f.clone(), None, 0)).collect();
    // Breadth-first, one directory level at a time, each level split across a few threads.
    while !frontier.is_empty() {
        let chunk = frontier.len().div_ceil(THREADS).max(1);
        let results: Vec<(Vec<Dir>, Vec<Arc<LocalChart>>)> = std::thread::scope(|s| {
            let handles: Vec<_> = frontier
                .chunks(chunk)
                .map(|dirs| {
                    s.spawn(move || {
                        lower_priority();
                        dirs.iter().fold((Vec::new(), Vec::new()), |mut acc, d| {
                            let before = acc.1.len();
                            visit(d, prev, quick, &mut acc.0, &mut acc.1);
                            found.fetch_add(acc.1.len() - before, Ordering::Relaxed);
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
                lib.add(&c);
                lib.catalog.push(c);
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
pub const VIDEO_EXT: &[&str] = &["mp4", "avi", "webm", "vp8", "ogv", "mpeg"];

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
/// `.sng` files are read wherever they are. Charts in `prev` whose file is unchanged are reused
/// instead of read again.
fn visit((dir, created, folder_stamp): &Dir, prev: &HashMap<PathBuf, Arc<LocalChart>>, quick: bool, subdirs: &mut Vec<Dir>, charts: &mut Vec<Arc<LocalChart>>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut dirs = Vec::new();
    // File names, and their modified time and size (free from the listing on Windows).
    let mut files: Vec<(String, Option<std::fs::Metadata>)> = Vec::new();
    let mut sngs = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            // Unfinished downloads (".bridge-rs-tmp-" from before the rename).
            if !name.starts_with(".truss-tmp-") && !name.starts_with(".bridge-rs-tmp-") {
                let meta = e.metadata().ok();
                let created = meta.as_ref().and_then(|m| secs(m.created()));
                let stamp = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64);
                // A chart folder whose date hasn't changed: the chart as it was, without
                // opening the folder.
                if quick
                    && stamp != 0
                    && let Some(old) = prev.get(&e.path()).filter(|o| o.folder_stamp == stamp)
                {
                    charts.push(old.clone());
                    continue;
                }
                dirs.push((e.path(), created, stamp));
            }
        } else if ext_of(&name) == "sng" {
            sngs.push((e.path(), e.metadata().ok()));
        } else {
            let meta = e.metadata().ok();
            files.push((name, meta));
        }
    }
    let ini = files.iter().find(|(n, _)| n.eq_ignore_ascii_case("song.ini"));
    let assets = assets_from_names(files.iter().map(|(n, _)| n.as_str()));
    if ini.is_some() || assets.chart_file.is_some() {
        // A chart folder: its subfolders don't hold other charts.
        let meta_of = |name: &str| files.iter().find(|(n, _)| n == name).and_then(|(_, m)| m.as_ref());
        let modified = assets.chart_file.as_deref().and_then(meta_of).and_then(|m| secs(m.modified())).unwrap_or(0);
        let added = created
            .or_else(|| std::fs::metadata(dir).ok().and_then(|m| secs(m.created())))
            .unwrap_or(modified);
        let stamp = ini.and_then(|(_, m)| m.as_ref()).map(stamp_of).unwrap_or(0);
        // The chart file is hashed again only when it changed.
        let hash = match prev.get(dir.as_path()) {
            Some(old) if old.chart_hash.is_some() && old.modified == modified && modified != 0 && old.chart_file == assets.chart_file => {
                old.chart_hash.clone()
            }
            _ => assets.chart_file.as_ref().and_then(|f| std::fs::read(dir.join(f)).ok()).map(|b| chart_hash(&b)),
        };
        let mut chart = match prev.get(dir.as_path()) {
            Some(old) if stamp != 0 && old.stamp == stamp => {
                // Same song.ini: keep what was read from it, refresh what comes from the listing.
                let mut c = LocalChart::clone(old);
                c.chart_type = assets.chart_type;
                c.chart_file = assets.chart_file;
                c.has_video = assets.has_video;
                c.has_background = assets.has_background;
                c.has_album_art = assets.has_album_art;
                c.has_audio = assets.has_audio;
                c.modified = modified;
                c.added = added;
                c
            }
            _ => {
                let pairs = ini
                    .and_then(|(i, _)| std::fs::read(dir.join(i)).ok())
                    .map(|b| parse_ini(&decode(&b)))
                    .unwrap_or_default();
                let mut c = make_chart(dir.to_path_buf(), &pairs, assets, modified, added);
                c.stamp = stamp;
                c
            }
        };
        chart.chart_hash = hash;
        chart.folder_stamp = *folder_stamp;
        charts.push(Arc::new(chart));
    } else {
        subdirs.extend(dirs);
    }
    for (sng, meta) in sngs {
        let stamp = meta.as_ref().map(stamp_of).unwrap_or(0);
        let modified = meta.as_ref().and_then(|m| secs(m.modified())).unwrap_or(0);
        let added = meta.as_ref().and_then(|m| secs(m.created())).unwrap_or(modified);
        if let Some(old) = prev.get(&sng).filter(|o| stamp != 0 && o.stamp == stamp && o.chart_hash.is_some()) {
            let mut c = LocalChart::clone(old);
            c.added = added;
            charts.push(Arc::new(c));
            continue;
        }
        // Read again only for its song.ini (after an update): its chart's hash still holds.
        let kept_hash = prev.get(&sng).filter(|o| o.modified == modified && modified != 0).and_then(|o| o.chart_hash.clone());
        if let Ok(index) = crate::sng::read_index(&sng) {
            let mut assets = assets_from_names(index.files.iter().map(|f| f.name.as_str()));
            assets.chart_type = ChartType::Sng;
            let hash = kept_hash.or_else(|| {
                assets
                    .chart_file
                    .as_ref()
                    .and_then(|n| index.files.iter().find(|f| &f.name == n))
                    .and_then(|e| crate::sng::read_entry(&sng, &index, e).ok())
                    .map(|b| chart_hash(&b))
            });
            let mut c = make_chart(sng, &index.metadata, assets, modified, added);
            c.stamp = stamp;
            c.chart_hash = hash;
            charts.push(Arc::new(c));
        }
    }
}

/// A file's modified time and size, packed into one number (never 0).
fn stamp_of(m: &std::fs::Metadata) -> u64 {
    let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64).unwrap_or(0);
    (t ^ m.len().rotate_left(40)).max(1)
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

/// The key a chart with this song.ini is matched on (as `LocalChart::key`): artist, name and
/// charter. Used by the pack index.
#[cfg_attr(not(test), allow(dead_code))] // used by the pack index builder
pub fn ini_key(bytes: &[u8]) -> String {
    pairs_key(&parse_ini(&decode(bytes)))
}

/// The same from song.ini values (a .sng file's metadata, say).
#[cfg_attr(not(test), allow(dead_code))] // used by the pack index builder
pub fn pairs_key(pairs: &[(String, String)]) -> String {
    let get = |k: &str| pairs.iter().find(|(pk, _)| pk.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str()).unwrap_or("");
    let charter = if get("charter").is_empty() { get("frets") } else { get("charter") };
    library_key(get("artist"), get("name"), charter)
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
        folder_stamp: 0,
        diff_guitar: get("diff_guitar").trim().parse::<f64>().ok().filter(|d| *d >= 0.0).map(|d| d.min(100.0) as i8),
        modified,
        added,
        stamp: 0,
        chart_hash: None,
        path,
    }
}

/// Prefix of the stand-in md5 given to library charts shown as a `Chart` (see `details`).
pub const LOCAL_MD5: &str = "local:";

/// The library copy a stand-in chart from `details` was read from.
pub fn local_path(c: &crate::api::Chart) -> Option<PathBuf> {
    c.md5.strip_prefix(LOCAL_MD5).map(PathBuf::from)
}

/// A library chart's song.ini read into a `Chart` (so the Browse details widgets can show it),
/// and its album art. Reads only; nothing is written.
pub fn details(c: &LocalChart) -> (crate::api::Chart, Option<Vec<u8>>) {
    let is_image = |n: &str| {
        let n = n.to_ascii_lowercase();
        IMAGE_EXT.iter().any(|e| n == format!("album.{e}"))
    };
    let (pairs, art) = if c.is_sng() {
        match crate::sng::read_index(&c.path) {
            Ok(idx) => {
                let art = idx.files.iter().find(|e| is_image(&e.name)).and_then(|e| crate::sng::read_entry(&c.path, &idx, e).ok());
                (idx.metadata.clone(), art)
            }
            Err(_) => (Vec::new(), None),
        }
    } else {
        let mut ini = None;
        let mut art = None;
        if let Ok(rd) = std::fs::read_dir(&c.path) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.eq_ignore_ascii_case("song.ini") {
                    ini = Some(e.path());
                } else if art.is_none() && is_image(&n) {
                    art = Some(e.path());
                }
            }
        }
        let pairs = ini.and_then(|p| std::fs::read(p).ok()).map(|b| parse_ini(&decode(&b))).unwrap_or_default();
        (pairs, art.and_then(|p| std::fs::read(p).ok()))
    };
    let get = |k: &str| pairs.iter().find(|(pk, _)| pk.eq_ignore_ascii_case(k)).map(|(_, v)| v.trim().to_string()).filter(|v| !v.is_empty());
    let num = |k: &str| get(k).and_then(|v| v.parse::<f64>().ok());
    let diff = |k: &str| num(k).map(|d| d as i32).filter(|d| *d >= 0);
    let t = c.tagged.as_deref();
    let chart = crate::api::Chart {
        name: Some(t.map(|t| t.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| c.name.clone())),
        artist: Some(t.map(|t| t.artist.clone()).unwrap_or_else(|| c.artist.clone())),
        charter: Some(t.map(|t| t.charter.clone()).unwrap_or_else(|| c.charter.clone())),
        album: get("album"),
        genre: get("genre"),
        year: get("year"),
        md5: format!("{LOCAL_MD5}{}", c.path.display()),
        song_length: (c.song_length_ms > 0).then_some(c.song_length_ms as i64),
        diff_band: diff("diff_band"),
        diff_guitar: diff("diff_guitar"),
        diff_guitar_coop: diff("diff_guitar_coop"),
        diff_rhythm: diff("diff_rhythm"),
        diff_bass: diff("diff_bass"),
        diff_drums: diff("diff_drums").or_else(|| diff("diff_drums_real")),
        diff_keys: diff("diff_keys"),
        diff_guitarghl: diff("diff_guitarghl"),
        diff_guitar_coop_ghl: diff("diff_guitar_coop_ghl"),
        diff_rhythm_ghl: diff("diff_rhythm_ghl"),
        diff_bassghl: diff("diff_bassghl"),
        diff_vocals: diff("diff_vocals"),
        loading_phrase: get("loading_phrase"),
        preview_start_time: num("preview_start_time").map(|v| v as i64),
        has_video_background: c.has_video,
        modchart: get("modchart").is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")),
        ..Default::default()
    };
    (chart, art)
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
        // Like Clone Hero, only `[name]` starts a section; a stray `[song] =` is skipped.
        if line.starts_with('[') && line.ends_with(']') {
            in_song = line[1..line.len() - 1].trim().eq_ignore_ascii_case("song");
        } else if line.starts_with('[') {
            continue;
        } else if in_song && let Some((k, v)) = line.split_once('=') {
            out.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

/// This process's private memory in MB (Windows; for the memory tests).
#[cfg(test)]
pub(crate) fn memory_private_mb() -> f64 {
    memory::private_mb()
}

#[cfg(test)]
mod memory {
    use super::*;

    /// This process's private memory in MB (Windows).
    pub(crate) fn private_mb() -> f64 {
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &format!("(Get-Process -Id {}).PrivateMemorySize64", std::process::id())])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().unwrap_or(0.0) / 1048576.0
    }

    /// What the library costs in memory, from the real cache (TRUSS_MEM_TEST=1).
    #[test]
    #[ignore]
    fn library_memory() {
        if std::env::var_os("TRUSS_MEM_TEST").is_none() {
            return;
        }
        let base = private_mb();
        let c = load_cache().expect("a library cache");
        let after_load = private_mb();
        let lib = Library::from_catalog(c.charts);
        let after_index = private_mb();
        eprintln!("{} charts: catalog {:.1} MB, lookup tables {:.1} MB", lib.charts, after_load - base, after_index - after_load);
        std::hint::black_box(&lib);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_keeps_charts_and_reads_older_files() {
        let mut c = test_chart("Band", "Song", 5);
        c.chart_hash = Some("h".into());
        c.folder_stamp = 42;
        let cache = Cache { version: CACHE_VERSION, folders: vec![PathBuf::from("Songs")], charts: vec![Arc::new(c)], full_scan: 7 };
        let mut bytes = Vec::new();
        cache.serialize(&mut rmp_serde::Serializer::new(&mut bytes).with_struct_map()).unwrap();
        let back: Cache = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!((back.full_scan, back.charts[0].folder_stamp, back.charts[0].chart_hash.as_deref()), (7, 42, Some("h")));
        // A file from before `full_scan` and `folder_stamp` existed still reads (as 0).
        let mut old = serde_json::to_value(&cache).unwrap();
        old.as_object_mut().unwrap().remove("full_scan");
        old["charts"][0].as_object_mut().unwrap().remove("folder_stamp");
        let mut bytes = Vec::new();
        old.serialize(&mut rmp_serde::Serializer::new(&mut bytes).with_struct_map()).unwrap();
        let back: Cache = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!((back.full_scan, back.charts[0].folder_stamp, back.charts[0].name.as_str()), (0, 0, "Song"));
    }

    #[test]
    fn ini_key() {
        let ini = "\u{feff}[Song]\r\nname = Through the Fire and Flames\r\nartist=DragonForce\r\nfrets = Neversoft\r\n[other]\r\nname = nope\r\n";
        let c = make_chart(PathBuf::from("x"), &parse_ini(ini), assets_from_names(["notes.mid", "song.ogg"].into_iter()), 0, 0);
        assert_eq!(c.key, "dragonforce|through the fire and flames|neversoft");
        assert_eq!(c.chart_type, ChartType::Mid);
        assert!(c.has_audio && !c.has_album_art);
    }

    /// A stray `[song] =` line (seen in real charts) doesn't end the [song] section.
    #[test]
    fn other_versions() {
        let fears_drive = song_key("Face My Fears (feat. Skrillex) (English Version)", "Angevil & Stickheadz32");
        let fears_pack = song_key("Face My Fears (English Version)", "Angevil & <color=#FF0>Stickheadz32</color>");
        assert_eq!(fears_drive, fears_pack);
        assert!(same_artist(&artist_words("Utada Hikaru"), &artist_words("Hikaru Utada (feat. Skrillex)")));
        assert_eq!(
            song_key("Herald of Darkness (feat. Alan Wake & Mr. Door)", "Angevil & ZetaPhoenix"),
            song_key("Herald of Darkness", "ZetaPhoenix, Angevil")
        );
        assert_ne!(song_key("Herald of Darkness - Radio Edit", "Angevil"), song_key("Herald of Darkness", "Angevil"));
        assert_eq!(artist_words("Old Gods of Asgard feat. Alan Wake"), ["old", "gods", "of", "asgard"]);
        assert!(!same_artist(&artist_words("Metallica"), &artist_words("Megadeth")));
        assert_eq!(song_key("", "Angevil"), None);
    }

    #[test]
    fn ini_stray_heading() {
        let ini = "[song]\r\n[song] = \r\nname = <color=#2e84c9>Summer</color>\r\nartist = Oakwood\r\ncharter = Whip\r\n";
        let c = make_chart(PathBuf::from("x"), &parse_ini(ini), assets_from_names(["notes.chart"].into_iter()), 0, 0);
        assert_eq!(c.key, "oakwood|summer|whip");
    }

    /// Times loading a saved library cache (TRUSS_TEST_CACHE = path to library.json).
    #[test]
    fn cache_load_timing() {
        let Some(p) = std::env::var_os("TRUSS_TEST_CACHE") else { return };
        let t = std::time::Instant::now();
        let f = std::fs::File::open(&p).unwrap();
        let c: Cache = serde_json::from_reader(std::io::BufReader::new(f)).unwrap();
        println!("from_reader: {} charts in {:?}", c.charts.len(), t.elapsed());
        let t = std::time::Instant::now();
        let b = std::fs::read(&p).unwrap();
        let c: Cache = serde_json::from_slice(&b).unwrap();
        println!("from_slice: {} charts in {:?}", c.charts.len(), t.elapsed());
        let t = std::time::Instant::now();
        let lib = Library::from_catalog(c.charts.clone());
        println!("from_catalog: {} in {:?}", lib.charts, t.elapsed());
        // A full scan, then a rescan reusing it (as at start-up with a cache).
        let t = std::time::Instant::now();
        let first = scan_blocking(&c.folders);
        println!("full scan: {} in {:?}", first.charts, t.elapsed());
        let prev: HashMap<PathBuf, Arc<LocalChart>> = first.catalog.iter().map(|c| (c.path.clone(), c.clone())).collect();
        let t = std::time::Instant::now();
        let again = scan_with(&c.folders, &AtomicUsize::new(0), &prev, false);
        println!("rescan with cache: {} in {:?}", again.charts, t.elapsed());
        assert_eq!(first.charts, again.charts);
        for (a, b) in first.catalog.iter().zip(again.catalog.iter()) {
            assert_eq!(format!("{a:?}"), format!("{b:?}"));
        }
    }

    /// Scans a real library when TRUSS_TEST_LIBRARY is set and prints timing.
    #[test]
    fn scan_real_library() {
        let Some(dir) = std::env::var_os("TRUSS_TEST_LIBRARY") else { return };
        let t = std::time::Instant::now();
        let root = PathBuf::from(dir);
        let lib = scan_blocking(std::slice::from_ref(&root));
        let t2 = std::time::Instant::now();
        let json = serde_json::to_vec(&lib.catalog).unwrap();
        let back: Vec<Arc<LocalChart>> = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.len(), lib.charts);
        println!("cache {} KB, round trip {:?}", json.len() / 1024, t2.elapsed());
        println!("{} chart hashes, e.g. {:?}", lib.hashes.len(), lib.hashes.keys().next());
        let some = lib.keys.values().next().unwrap()[0].to_path_buf();
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
