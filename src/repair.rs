//! Tools > Repair bad songs: reads Clone Hero's badsongs.txt and fixes what can be fixed without
//! changing how a chart plays. Truss never edits a notes.chart / notes.mid: the fixes only
//! rename files and folders, write song.ini metadata (never the values that change how notes
//! read, like HOPO frequency), or move an exact duplicate to the Recycle Bin. After each fix the
//! chart's leaderboard track hashes are read again, and the fix is undone if they changed.

use crate::library;
use futures::{SinkExt, channel::mpsc::Sender};
use iced::widget::{button, column, container, progress_bar, row, scrollable, text, text_input};
use iced::{Alignment, Element, Length, Task, Theme};
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

// ---------- Reading badsongs.txt ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    LyricsEncoding,
    LyricsUnvalidated,
    NoMetadata,
    NoChart,
    Corrupt,
    Duplicate,
    NoInstruments,
    Other,
}

impl Kind {
    fn is_warning(self) -> bool {
        matches!(self, Kind::LyricsEncoding | Kind::LyricsUnvalidated)
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub kind: Kind,
    pub path: PathBuf,
    /// For a duplicate: the copy Clone Hero kept.
    pub other: Option<PathBuf>,
}

/// The folders listed in badsongs.txt, by the problem Clone Hero gave.
pub fn parse(text: &str) -> Vec<Entry> {
    let mut kind = Kind::Other;
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        let low = l.to_lowercase();
        if low.starts_with("warning:") || low.starts_with("error:") {
            kind = if low.contains("not valid utf-8") {
                Kind::LyricsEncoding
            } else if low.contains("matching vocal note") {
                Kind::LyricsUnvalidated
            } else if low.contains("metadata") {
                Kind::NoMetadata
            } else if low.contains("no notes.mid or notes.chart") {
                Kind::NoChart
            } else if low.contains("corrupt") {
                Kind::Corrupt
            } else if low.contains("duplicate") {
                Kind::Duplicate
            } else if low.contains("supported instruments") {
                Kind::NoInstruments
            } else {
                Kind::Other
            };
            continue;
        }
        if !is_path(l) {
            continue; // an explanation line under a heading
        }
        if kind == Kind::Duplicate
            && let Some(i) = l.match_indices(" (").map(|(i, _)| i).filter(|&i| is_path(&l[i + 2..])).last()
            && l.ends_with(')')
        {
            out.push(Entry { kind, path: PathBuf::from(&l[..i]), other: Some(PathBuf::from(&l[i + 2..l.len() - 1])) });
            continue;
        }
        out.push(Entry { kind, path: PathBuf::from(l), other: None });
    }
    out
}

fn is_path(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

// ---------- Working out fixes ----------

/// One change to a chart folder.
#[derive(Debug, Clone)]
pub enum Step {
    /// Shorten the chart folder's own name so its files fit Windows' 260-character limit.
    ShortenFolder(PathBuf),
    /// Rename the chart file Clone Hero didn't find (`note.mid`, `Song.chart`…) to notes.*.
    RenameChart { from: String, to: String },
    /// Write a song.ini with these values (metadata only).
    WriteIni(Vec<(String, String)>),
    /// Add the missing song name to song.ini.
    AddName(String),
    /// Move this copy to the Recycle Bin; the other copy (with identical notes) stays.
    RemoveCopy { keep: PathBuf },
}

impl Step {
    fn describe(&self) -> String {
        match self {
            Step::ShortenFolder(to) => {
                format!("Shorten its folder name to \"{}\" (the path is too long for Clone Hero)", to.file_name().unwrap_or_default().to_string_lossy())
            }
            Step::RenameChart { from, to } => format!("Rename {from} to {to}"),
            Step::WriteIni(p) => {
                let name = p.iter().find(|(k, _)| k == "name").map(|(_, v)| v.as_str()).unwrap_or("");
                format!("Write a song.ini (name \"{name}\")")
            }
            Step::AddName(n) => format!("Add the song name \"{n}\" to song.ini"),
            Step::RemoveCopy { keep } => format!("Move this copy to the Recycle Bin; keep {}", short(keep)),
        }
    }
}

#[derive(Debug, Clone)]
pub enum State {
    /// Can be repaired (ticked or not).
    Ready,
    Done(String),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct Item {
    pub entry: Entry,
    /// The steps that fix it, or why it can't be fixed.
    pub fix: Result<Vec<Step>, String>,
    pub on: bool,
    pub state: State,
}

/// Works out a fix for each entry (reads the folders; changes nothing).
pub fn diagnose(entries: Vec<Entry>, roots: &[PathBuf]) -> Vec<Item> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
    let chunk = entries.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = entries
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    part.iter()
                        .map(|e| {
                            // badsongs.txt lowercases most paths: use the real names.
                            let mut e = e.clone();
                            e.path = real_path(&e.path);
                            e.other = e.other.as_deref().map(real_path);
                            let fix = fix_for(&e, roots);
                            let on = fix.is_ok();
                            Item { entry: e, fix, on, state: State::Ready }
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).flatten().collect()
    })
}

const MAX_PATH: usize = 259;

/// A path with each part spelled as it is on disk (it's unchanged if it can't be found).
fn real_path(p: &Path) -> PathBuf {
    match std::fs::canonicalize(p) {
        Ok(c) => {
            let s = c.to_string_lossy();
            match s.strip_prefix(r"\\?\UNC\") {
                Some(rest) => PathBuf::from(format!(r"\\{rest}")),
                None => PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s)),
            }
        }
        Err(_) => p.to_path_buf(),
    }
}

fn fix_for(e: &Entry, roots: &[PathBuf]) -> Result<Vec<Step>, String> {
    match e.kind {
        Kind::LyricsEncoding => Err("Lyrics warning only: the chart plays fine.".into()),
        Kind::LyricsUnvalidated => Err("Lyrics warning only: the chart plays fine.".into()),
        Kind::Duplicate => duplicate_fix(e, roots),
        Kind::NoMetadata | Kind::NoChart | Kind::Corrupt | Kind::NoInstruments | Kind::Other => folder_fix(e, roots),
    }
}

fn folder_fix(e: &Entry, roots: &[PathBuf]) -> Result<Vec<Step>, String> {
    let dir = &e.path;
    if !dir.is_dir() {
        return Err("This folder isn't there any more.".into());
    }
    let files: Vec<(String, u64)> = std::fs::read_dir(dir)
        .map_err(|e| format!("Couldn't read the folder: {e}"))?
        .flatten()
        .filter(|f| f.file_type().is_ok_and(|t| t.is_file()))
        .map(|f| (f.file_name().to_string_lossy().into_owned(), f.metadata().map(|m| m.len()).unwrap_or(0)))
        .collect();
    let lower = |n: &str| n.to_ascii_lowercase();
    let is_chart = |n: &str| {
        let l = lower(n);
        l.ends_with(".chart") || l.ends_with(".mid")
    };
    let mut steps = Vec::new();

    // A path too long for Windows' usual limit.
    let longest = files.iter().map(|(n, _)| n.len()).max().unwrap_or(0).max("notes.chart".len());
    let dir_len = dir.as_os_str().len();
    if dir_len + 1 + longest > MAX_PATH {
        if !crate::library_tools::is_loose(dir, roots) {
            return Err("Its path is longer than Windows allows (260 characters). It's inside a pack, so Truss won't rename it; moving the pack closer to the Songs folder fixes it.".into());
        }
        let parent = dir.parent().ok_or("No parent folder")?;
        let budget = MAX_PATH.saturating_sub(parent.as_os_str().len() + 2 + longest);
        if budget < 16 {
            return Err("Its path is too long and its folder name can't be shortened enough.".into());
        }
        let name = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let mut cut: String = name.chars().take(budget - 1).collect();
        while cut.len() > budget - 3 {
            cut.pop();
        }
        let cut = cut.trim_end_matches([' ', '.', ',', '(', '[', '-', '&']).to_string() + "…";
        let mut to = parent.join(&cut);
        let mut n = 2;
        while to.exists() {
            to = parent.join(format!("{} ({n})", cut.trim_end_matches('…')));
            n += 1;
        }
        steps.push(Step::ShortenFolder(to));
    }

    let charts: Vec<&(String, u64)> = files.iter().filter(|(n, _)| is_chart(n)).collect();
    if charts.is_empty() {
        return Err("There's no chart file in this folder. Download it again.".into());
    }
    let has_notes = charts.iter().any(|(n, _)| lower(n) == "notes.chart" || lower(n) == "notes.mid");
    if charts.iter().filter(|(n, _)| lower(n).starts_with("notes.")).all(|(_, len)| *len == 0) && has_notes {
        return Err("Its chart file is empty (a broken download or unpack). Download it again.".into());
    }
    if !has_notes {
        // The file scan-chart would pick: .mid first, then names with "note" in them, then not copies.
        let mut c: Vec<&&(String, u64)> = charts.iter().filter(|(_, len)| *len > 0).collect();
        c.sort_by_key(|(n, len)| {
            let l = lower(n);
            (!l.ends_with(".mid"), !l.contains("note"), l.contains("cop"), std::cmp::Reverse(*len))
        });
        let Some((from, _)) = c.first() else { return Err("Its chart file is empty. Download it again.".into()) };
        let to = if lower(from).ends_with(".mid") { "notes.mid" } else { "notes.chart" };
        steps.push(Step::RenameChart { from: from.clone(), to: to.into() });
    }

    let ini = files.iter().find(|(n, _)| lower(n) == "song.ini");
    match ini {
        None => steps.push(Step::WriteIni(ini_from(dir, &files))),
        Some((n, _)) => {
            let pairs = library::parse_ini(&library::decode(&std::fs::read(dir.join(n)).unwrap_or_default()));
            let named = pairs.iter().any(|(k, v)| k.eq_ignore_ascii_case("name") && !v.trim().is_empty());
            if !named {
                steps.push(Step::AddName(name_from_folder(dir).1));
            }
        }
    }
    if steps.is_empty() {
        return Err(match e.kind {
            Kind::Corrupt => "Clone Hero can't read its notes file. Repairing it would mean changing the chart, so Truss leaves it: download a fresh copy, or tell its charter.".into(),
            Kind::NoInstruments => "Its notes file has no tracks Clone Hero recognises (often an old MIDI layout). Converting it would change the chart, so Truss leaves it.".into(),
            _ => "Truss found nothing it can safely fix here. Downloading it again may help.".into(),
        });
    }
    Ok(steps)
}

/// "Artist - Name (Charter)" from a folder name: (artist, name, charter).
fn name_from_folder(dir: &Path) -> (String, String, String) {
    let n = dir.file_name().unwrap_or_default().to_string_lossy().trim_end_matches('…').trim().to_string();
    let n = n.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ' || c == '_').to_string();
    let (artist, rest) = n.split_once(" - ").map(|(a, r)| (a.trim().to_string(), r.trim().to_string())).unwrap_or((String::new(), n.clone()));
    let (name, charter) = match rest.rfind(['(', '[']) {
        Some(i) if rest.ends_with([')', ']']) && i > 0 => (rest[..i].trim().to_string(), rest[i + 1..rest.len() - 1].trim().to_string()),
        _ => (rest, String::new()),
    };
    (artist, if name.is_empty() { n } else { name }, charter)
}

/// song.ini values for a folder without one: from a .chart's [Song] section where it has them,
/// else from the folder name. Only metadata, so nothing about how the notes read changes.
fn ini_from(dir: &Path, files: &[(String, u64)]) -> Vec<(String, String)> {
    let (artist, name, charter) = name_from_folder(dir);
    let mut out = vec![("name".to_string(), name), ("artist".to_string(), artist), ("charter".to_string(), charter)];
    let chart = files.iter().map(|(n, _)| n).find(|n| n.to_ascii_lowercase().ends_with(".chart"));
    if let Some(text) = chart.and_then(|n| std::fs::read(dir.join(n)).ok()).map(|b| library::decode(&b)) {
        let mut in_song = false;
        for line in text.lines() {
            let l = line.trim();
            if l.starts_with('[') {
                if in_song {
                    break;
                }
                in_song = l.eq_ignore_ascii_case("[song]");
                continue;
            }
            let Some((k, v)) = l.split_once('=') else { continue };
            if !in_song {
                continue;
            }
            let v = v.trim().trim_start_matches(',').trim().trim_matches('"').trim().to_string();
            let key = match k.trim() {
                "Name" => "name",
                "Artist" => "artist",
                "Album" => "album",
                "Year" => "year",
                "Genre" => "genre",
                "Charter" => "charter",
                _ => continue,
            };
            if v.is_empty() {
                continue;
            }
            match out.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = v,
                None => out.push((key.to_string(), v)),
            }
        }
    }
    out.retain(|(_, v)| !v.is_empty());
    out
}

fn duplicate_fix(e: &Entry, roots: &[PathBuf]) -> Result<Vec<Step>, String> {
    let other = e.other.clone().ok_or("Clone Hero didn't say which chart it duplicates.")?;
    if !e.path.exists() || !other.exists() {
        return Err("One of the copies isn't there any more.".into());
    }
    // Clone Hero skipped the first and kept the second; never remove a copy inside a pack.
    let (remove, keep) = match (crate::library_tools::is_loose(&e.path, roots), crate::library_tools::is_loose(&other, roots)) {
        (true, _) => (e.path.clone(), other),
        (false, true) => (other, e.path.clone()),
        (false, false) => return Err("Both copies are inside packs or other folders, which Truss leaves alone.".into()),
    };
    if !library::is_deletable_chart(&remove, roots) {
        return Err("This copy isn't inside your library folders.".into());
    }
    let a = library::track_hashes(&remove);
    if a.is_empty() || a != library::track_hashes(&keep) {
        return Err("Clone Hero calls these duplicates, but their notes aren't identical, so both stay.".into());
    }
    Ok(vec![Step::RemoveCopy { keep }])
}

// ---------- Applying fixes ----------

/// Applies an item's steps (undoing them all if the chart's track hashes change), and adds what
/// was done to the repair history so it can be undone later.
pub async fn apply(item: Item) -> Result<String, String> {
    let steps = item.fix.clone()?;
    tokio::task::spawn_blocking(move || {
        let path = item.entry.path.clone();
        let (msg, changes) = apply_blocking(&path, &steps)?;
        let what = steps.iter().map(|s| s.describe()).collect::<Vec<_>>().join("; ");
        record(Record { id: now_id(), when: now_secs(), folder: path, what, changes, undone: false });
        Ok(msg)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// One change Truss made, with what's needed to reverse it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Change {
    Renamed { from: PathBuf, to: PathBuf },
    Created { path: PathBuf },
    /// A file rewritten; its old contents (base64).
    Rewrote { path: PathBuf, old: String },
    /// Moved to the Recycle Bin.
    Recycled { path: PathBuf },
}

fn b64(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn unb64(s: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).unwrap_or_default()
}

/// Reverses changes, newest first. Stops at the first that can't be reversed.
fn reverse(changes: &[Change]) -> Result<(), String> {
    for c in changes.iter().rev() {
        match c {
            Change::Renamed { from, to } => {
                if from.exists() {
                    return Err(format!("{} is back already (or something else is there now).", short(from)));
                }
                std::fs::rename(to, from).map_err(|e| format!("Couldn't rename {} back: {e}", short(to)))?;
            }
            Change::Created { path } => {
                if path.exists() {
                    std::fs::remove_file(path).map_err(|e| format!("Couldn't remove {}: {e}", short(path)))?;
                }
            }
            Change::Rewrote { path, old } => std::fs::write(path, unb64(old)).map_err(|e| format!("Couldn't restore {}: {e}", short(path)))?,
            Change::Recycled { path } => {
                if path.exists() {
                    return Err(format!("{} is already back.", short(path)));
                }
                let want = path.to_string_lossy().to_lowercase();
                let items = trash::os_limited::list().map_err(|e| format!("Couldn't read the Recycle Bin: {e}"))?;
                let item = items
                    .into_iter()
                    .filter(|i| i.original_path().to_string_lossy().to_lowercase() == want)
                    .max_by_key(|i| i.time_deleted)
                    .ok_or_else(|| format!("{} isn't in the Recycle Bin any more.", short(path)))?;
                trash::os_limited::restore_all([item]).map_err(|e| format!("Couldn't restore {}: {e}", short(path)))?;
            }
        }
    }
    Ok(())
}

fn apply_blocking(path: &Path, steps: &[Step]) -> Result<(String, Vec<Change>), String> {
    if let [Step::RemoveCopy { keep }] = steps {
        let a = library::track_hashes(path);
        if a.is_empty() || a != library::track_hashes(keep) {
            return Err("The copies' notes no longer match; nothing was removed.".into());
        }
        trash::delete(path).map_err(|e| format!("Couldn't move it to the Recycle Bin: {e}"))?;
        return Ok(("Moved to the Recycle Bin".into(), vec![Change::Recycled { path: path.to_path_buf() }]));
    }
    // What the chart reads as now (with the chart file it will have), to compare afterwards.
    let expected = expected_hashes(path, steps);
    let mut dir = path.to_path_buf();
    let mut done = Vec::new();
    let result = (|| -> Result<(), String> {
        for s in steps {
            match s {
                Step::ShortenFolder(to) => {
                    std::fs::rename(&dir, to).map_err(|e| format!("Couldn't rename the folder: {e}"))?;
                    done.push(Change::Renamed { from: dir.clone(), to: to.clone() });
                    dir = to.clone();
                }
                Step::RenameChart { from, to } => {
                    let (a, b) = (dir.join(from), dir.join(to));
                    std::fs::rename(&a, &b).map_err(|e| format!("Couldn't rename {from}: {e}"))?;
                    done.push(Change::Renamed { from: a, to: b });
                }
                Step::WriteIni(pairs) => {
                    let p = dir.join("song.ini");
                    let mut s = String::from("[song]\r\n");
                    for (k, v) in pairs {
                        s.push_str(&format!("{k} = {v}\r\n"));
                    }
                    std::fs::write(&p, s).map_err(|e| format!("Couldn't write song.ini: {e}"))?;
                    done.push(Change::Created { path: p });
                }
                Step::AddName(name) => {
                    let p = std::fs::read_dir(&dir)
                        .map_err(|e| e.to_string())?
                        .flatten()
                        .map(|f| f.path())
                        .find(|f| f.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("song.ini")))
                        .ok_or("song.ini is gone")?;
                    let old = std::fs::read(&p).map_err(|e| e.to_string())?;
                    let text = library::decode(&old);
                    // Right under the [song] heading (or at the top, adding one).
                    let at = text.lines().position(|l| l.trim().eq_ignore_ascii_case("[song]"));
                    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
                    match at {
                        Some(i) => lines.insert(i + 1, format!("name = {name}")),
                        None => {
                            lines.insert(0, format!("name = {name}"));
                            lines.insert(0, "[song]".into());
                        }
                    }
                    std::fs::write(&p, lines.join("\r\n") + "\r\n").map_err(|e| format!("Couldn't update song.ini: {e}"))?;
                    done.push(Change::Rewrote { path: p, old: b64(&old) });
                }
                Step::RemoveCopy { .. } => return Err("A duplicate can't be removed alongside other fixes.".into()),
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = reverse(&done);
        return Err(e);
    }
    let after = library::track_hashes(&dir);
    if after.is_empty() {
        let _ = reverse(&done);
        return Err("After the fix Truss still couldn't read the chart, so it put everything back.".into());
    }
    if expected.as_ref().is_some_and(|x| *x != after) {
        let _ = reverse(&done);
        return Err("The fix would have changed how the chart reads, so Truss put everything back.".into());
    }
    Ok(("Repaired".into(), done))
}

/// The track hashes the chart should have after the steps: its chart file as it is, read with
/// its song.ini as it is (a new song.ini only adds metadata, which changes nothing here).
fn expected_hashes(dir: &Path, steps: &[Step]) -> Option<Vec<String>> {
    let rename = steps.iter().find_map(|s| match s {
        Step::RenameChart { from, to } => Some((from.clone(), to.clone())),
        _ => None,
    });
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for f in std::fs::read_dir(dir).ok()?.flatten() {
        let n = f.file_name().to_string_lossy().into_owned();
        let l = n.to_ascii_lowercase();
        match &rename {
            Some((from, to)) if &n == from => files.push((to.clone(), std::fs::read(f.path()).ok()?)),
            _ if l == "notes.chart" || l == "notes.mid" || l == "song.ini" => files.push((n, std::fs::read(f.path()).ok()?)),
            _ => {}
        }
    }
    let mut out: Vec<String> = crate::trackhash::hash_tracks(&files, None)
        .ok()?
        .into_iter()
        .map(|(i, d, h)| format!("{i}/{d}/{}", h.trim_end_matches('=')))
        .collect();
    out.sort();
    (!out.is_empty()).then_some(out)
}

// ---------- History ----------

/// One repair, remembered so it can be undone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub id: u64,
    /// Seconds since the epoch.
    pub when: u64,
    pub folder: PathBuf,
    pub what: String,
    pub changes: Vec<Change>,
    pub undone: bool,
}

/// Kept in Truss's data folder (not its cache, which may be cleared).
fn history_path() -> Option<PathBuf> {
    crate::settings::data_dir().map(|d| d.join("repair-history.json"))
}

static HISTORY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn load_history() -> Vec<Record> {
    let _g = HISTORY_LOCK.lock();
    history_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_history(h: &[Record]) {
    let Some(p) = history_path() else { return };
    let _ = std::fs::create_dir_all(p.parent().unwrap_or(Path::new(".")));
    let tmp = p.with_extension("json.tmp");
    if serde_json::to_vec_pretty(h).ok().and_then(|b| std::fs::write(&tmp, b).ok()).is_some() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

fn record(r: Record) {
    let mut h = load_history();
    let _g = HISTORY_LOCK.lock();
    h.push(r);
    save_history(&h);
}

/// Undoes the repairs with these ids, newest first; returns how many were undone and the
/// problems with the rest.
pub fn undo(ids: &[u64]) -> (usize, Vec<String>) {
    let mut h = load_history();
    let _g = HISTORY_LOCK.lock();
    let (mut n, mut errors) = (0, Vec::new());
    for r in h.iter_mut().rev().filter(|r| !r.undone && ids.contains(&r.id)) {
        match reverse(&r.changes) {
            Ok(()) => {
                r.undone = true;
                n += 1;
            }
            Err(e) => errors.push(format!("{}: {e}", r.folder.file_name().unwrap_or_default().to_string_lossy())),
        }
    }
    save_history(&h);
    (n, errors)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn now_id() -> u64 {
    static LAST: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    // Unique even when two land in the same tick.
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    *last = t.max(*last + 1);
    *last
}

fn short(p: &Path) -> String {
    let parts: Vec<String> = p.components().rev().take(3).map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    format!("…\\{}", parts.into_iter().rev().collect::<Vec<_>>().join("\\"))
}

// ---------- The tool ----------

#[derive(Default)]
pub struct RepairTool {
    path: String,
    loading: bool,
    items: Vec<Item>,
    error: Option<String>,
    confirm: bool,
    /// Repairs running: (done, total).
    running: Option<(usize, usize)>,
    show_unfixable: bool,
    history: Vec<Record>,
    history_loaded: bool,
    show_history: bool,
    confirm_undo_all: bool,
    undo_note: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Path(String),
    Read,
    Read2(Result<Vec<Item>, String>),
    Toggle(usize),
    All(bool),
    ShowUnfixable(bool),
    Repair,
    Progress(usize, Result<String, String>),
    Finished,
    ShowHistory(bool),
    History(Vec<Record>),
    Undo(Vec<u64>),
    UndoAll,
    Undone((usize, Vec<String>)),
}

/// Where Clone Hero writes badsongs.txt: beside the Songs folder.
fn default_path(settings: &crate::settings::Settings) -> String {
    settings
        .library_path
        .as_ref()
        .and_then(|p| p.parent())
        .map(|p| p.join("badsongs.txt"))
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

impl RepairTool {
    /// The repairs made so far, each with an Undo button.
    fn view_history<'a>(&'a self) -> Element<'a, Msg> {
        let muted = |t: &Theme| text::Style { color: Some(crate::style::muted(t)) };
        let open = self.show_history;
        let live = self.history.iter().filter(|r| !r.undone).count();
        let head = button(
            text(format!(
                "{} Repair history{}",
                if open { "▾" } else { "▸" },
                if self.history_loaded { format!(" ({live} can be undone)") } else { String::new() }
            ))
            .size(13)
            .font(crate::BOLD),
        )
        .padding([4, 0])
        .style(button::text)
        .on_press(Msg::ShowHistory(!open));
        let mut col = column![head].spacing(8);
        if !open {
            return col.into();
        }
        let mut bar = row![
            text("Every repair is remembered (renames, new song.ini files, the old song.ini text, recycled copies), so it can be put back exactly.")
                .size(12)
                .style(muted)
                .width(Length::Fill)
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if live > 0 {
            bar = bar.push(
                button(text(if self.confirm_undo_all { format!("Confirm: undo all {live}") } else { "Undo all".to_string() }).size(12).wrapping(text::Wrapping::None))
                    .padding([4, 10])
                    .style(move |t: &Theme, s| if self.confirm_undo_all { button::danger(t, s) } else { crate::style::secondary(t, s) })
                    .on_press_maybe(self.running.is_none().then_some(Msg::UndoAll)),
            );
        }
        col = col.push(bar);
        if let Some(n) = &self.undo_note {
            col = col.push(text(n.clone()).size(12));
        }
        if self.history.is_empty() {
            return col.push(text("No repairs yet.").size(12).style(muted)).into();
        }
        for r in self.history.iter().rev().take(300) {
            let when = crate::library_ui::added_date(r.when);
            let action: Element<'a, Msg> = if r.undone {
                text("Undone").size(11).style(muted).into()
            } else {
                button(text("Undo").size(11)).padding([2, 8]).style(crate::style::secondary).on_press_maybe(self.running.is_none().then_some(Msg::Undo(vec![r.id]))).into()
            };
            col = col.push(
                row![
                    column![
                        text(r.folder.file_name().unwrap_or_default().to_string_lossy().into_owned()).size(12),
                        text(format!("{when} · {}", r.what)).size(11).style(muted),
                    ]
                    .spacing(1)
                    .width(Length::Fill),
                    action,
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        col.into()
    }

    /// `rescan`: true when charts changed and the library should be read again.
    pub fn update(&mut self, m: Msg, settings: &crate::settings::Settings) -> (Task<Msg>, bool) {
        if !matches!(m, Msg::Repair | Msg::Progress(..)) {
            self.confirm = false;
        }
        if !matches!(m, Msg::UndoAll) {
            self.confirm_undo_all = false;
        }
        match m {
            Msg::ShowHistory(b) => {
                self.show_history = b;
                if b {
                    return (Task::perform(async { tokio::task::spawn_blocking(load_history).await.unwrap_or_default() }, Msg::History), false);
                }
            }
            Msg::History(h) => {
                self.history = h;
                self.history_loaded = true;
            }
            Msg::UndoAll => {
                if !self.confirm_undo_all {
                    self.confirm_undo_all = true;
                    return (Task::none(), false);
                }
                let ids: Vec<u64> = self.history.iter().filter(|r| !r.undone).map(|r| r.id).collect();
                return self.update(Msg::Undo(ids), settings);
            }
            Msg::Undo(ids) => {
                if self.running.is_some() {
                    return (Task::none(), false);
                }
                self.undo_note = Some("Undoing…".into());
                return (Task::perform(async move { tokio::task::spawn_blocking(move || undo(&ids)).await.unwrap_or((0, vec![])) }, Msg::Undone), false);
            }
            Msg::Undone((n, errors)) => {
                self.undo_note = Some(if errors.is_empty() {
                    format!("Undid {n} repair{}.", if n == 1 { "" } else { "s" })
                } else {
                    format!("Undid {n}; couldn't undo {}: {}", errors.len(), errors.join(" · "))
                });
                self.history = load_history();
                return (Task::none(), n > 0);
            }
            Msg::Path(p) => self.path = p,
            Msg::Read => {
                if self.path.trim().is_empty() {
                    self.path = default_path(settings);
                }
                let path = PathBuf::from(self.path.trim());
                let roots = settings.scan_folders();
                self.loading = true;
                self.error = None;
                return (
                    Task::perform(
                        async move {
                            let text = std::fs::read(&path).map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
                            let entries = parse(&library::decode(&text));
                            tokio::task::spawn_blocking(move || diagnose(entries, &roots)).await.map_err(|e| e.to_string())
                        },
                        Msg::Read2,
                    ),
                    false,
                );
            }
            Msg::Read2(r) => {
                self.loading = false;
                match r {
                    Ok(items) => self.items = items,
                    Err(e) => self.error = Some(e),
                }
            }
            Msg::Toggle(i) => {
                if let Some(it) = self.items.get_mut(i).filter(|it| it.fix.is_ok() && matches!(it.state, State::Ready)) {
                    it.on = !it.on;
                }
            }
            Msg::All(on) => {
                for it in self.items.iter_mut().filter(|it| it.fix.is_ok() && matches!(it.state, State::Ready)) {
                    it.on = on;
                }
            }
            Msg::ShowUnfixable(b) => self.show_unfixable = b,
            Msg::Repair => {
                if self.running.is_some() {
                    return (Task::none(), false);
                }
                if !self.confirm {
                    self.confirm = true;
                    return (Task::none(), false);
                }
                self.confirm = false;
                let jobs: Vec<(usize, Item)> = self
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, it)| it.on && it.fix.is_ok() && matches!(it.state, State::Ready))
                    .map(|(i, it)| (i, it.clone()))
                    .collect();
                if jobs.is_empty() {
                    return (Task::none(), false);
                }
                self.running = Some((0, jobs.len()));
                // One at a time, so two fixes never touch the same folder at once.
                let stream = iced::stream::channel(16, async move |mut tx: Sender<Msg>| {
                    for (i, it) in jobs {
                        let r = apply(it).await;
                        let _ = tx.send(Msg::Progress(i, r)).await;
                    }
                    let _ = tx.send(Msg::Finished).await;
                });
                return (Task::run(stream, |m| m), false);
            }
            Msg::Progress(i, r) => {
                if let Some(it) = self.items.get_mut(i) {
                    it.state = match r {
                        Ok(s) => State::Done(s),
                        Err(e) => State::Failed(e),
                    };
                }
                if let Some((d, _)) = &mut self.running {
                    *d += 1;
                }
            }
            Msg::Finished => {
                self.running = None;
                self.history = load_history();
                self.history_loaded = true;
                return (Task::none(), true);
            }
        }
        (Task::none(), false)
    }

    pub fn view<'a>(&'a self, settings: &crate::settings::Settings) -> Element<'a, Msg> {
        let muted = |t: &Theme| text::Style { color: Some(crate::style::muted(t)) };
        let placeholder = default_path(settings);
        let read_label = if self.items.is_empty() { "Read badsongs.txt" } else { "Read again" };
        let top = column![
            text("Repair bad songs").size(15).font(crate::BOLD),
            text("Clone Hero lists the songs it couldn't load in badsongs.txt (beside your Songs folder) after a scan. Truss fixes what it can without changing how any chart plays: it renames files and folders, writes missing song.ini details, and moves exact duplicates to the Recycle Bin. It never edits notes, and it undoes any fix that would change a chart's leaderboard hashes.")
                .size(12)
                .style(muted),
            row![
                text_input(if placeholder.is_empty() { "Path to badsongs.txt" } else { &placeholder }, &self.path)
                    .on_input(Msg::Path)
                    .on_submit(Msg::Read)
                    .padding(7)
                    .size(13),
                button(text(if self.loading { "Reading…" } else { read_label }).size(13).wrapping(text::Wrapping::None))
                    .padding([6, 14])
                    .style(button::primary)
                    .on_press_maybe((!self.loading && self.running.is_none()).then_some(Msg::Read)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(8);
        let mut col = column![top].spacing(12);
        if let Some(e) = &self.error {
            col = col.push(text(e.clone()).size(12).style(|t: &Theme| text::Style { color: Some(t.extended_palette().danger.base.color) }));
        }
        col = col.push(self.view_history());
        if self.items.is_empty() {
            return container(scrollable(col)).padding([14, 18]).height(Length::Fill).into();
        }

        let fixable: Vec<usize> = (0..self.items.len()).filter(|&i| self.items[i].fix.is_ok()).collect();
        let warnings = self.items.iter().filter(|it| it.entry.kind.is_warning()).count();
        let unfixable: Vec<usize> = (0..self.items.len()).filter(|&i| self.items[i].fix.is_err() && !self.items[i].entry.kind.is_warning()).collect();
        let picked = fixable.iter().filter(|&&i| self.items[i].on && matches!(self.items[i].state, State::Ready)).count();
        let repaired = self.items.iter().filter(|it| matches!(it.state, State::Done(_))).count();
        let mut summary = format!("{} can be repaired · {} can't · {} lyrics warnings (harmless, nothing to fix)", fixable.len(), unfixable.len(), warnings);
        if repaired > 0 {
            summary.push_str(&format!(" · {repaired} repaired"));
        }
        let mut actions = row![text(summary).size(12).width(Length::Fill)].spacing(8).align_y(Alignment::Center);
        if let Some((d, t)) = self.running {
            actions = actions.push(column![text(format!("Repairing {d} of {t}…")).size(12), progress_bar(0.0..=1.0, d as f32 / t.max(1) as f32).length(160).girth(4)].spacing(3));
        } else {
            actions = actions
                .push(button(text("Tick all").size(12)).padding([4, 10]).style(crate::style::secondary).on_press(Msg::All(true)))
                .push(button(text("Untick all").size(12)).padding([4, 10]).style(crate::style::secondary).on_press(Msg::All(false)))
                .push(
                    button(text(if self.confirm { format!("Confirm: repair {picked}") } else { format!("Repair {picked}") }).size(13).wrapping(text::Wrapping::None))
                        .padding([6, 14])
                        .style(move |t: &Theme, s| if self.confirm { button::danger(t, s) } else { button::primary(t, s) })
                        .on_press_maybe((picked > 0).then_some(Msg::Repair)),
                );
        }
        col = col.push(actions);

        let row_for = |i: usize| -> Element<'a, Msg> {
            let it = &self.items[i];
            let name = it.entry.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let detail: String = match (&it.fix, &it.state) {
                (_, State::Done(s)) => s.clone(),
                (_, State::Failed(e)) => e.clone(),
                (Ok(steps), _) => steps.iter().map(|s| s.describe()).collect::<Vec<_>>().join("; "),
                (Err(why), _) => why.clone(),
            };
            let state_color = match &it.state {
                State::Done(_) => Some(true),
                State::Failed(_) => Some(false),
                State::Ready => None,
            };
            let mark: Element<'a, Msg> = match (&it.fix, &it.state) {
                (Ok(_), State::Ready) => button(text(if it.on { "☑" } else { "☐" }).size(14)).padding(0).style(button::text).on_press(Msg::Toggle(i)).into(),
                (_, State::Done(_)) => text("✓").size(13).style(|t: &Theme| text::Style { color: Some(t.extended_palette().success.base.color) }).into(),
                (_, State::Failed(_)) => text("✗").size(13).style(|t: &Theme| text::Style { color: Some(t.extended_palette().danger.base.color) }).into(),
                _ => text("·").size(13).style(muted).into(),
            };
            row![
                container(mark).width(20),
                column![
                    text(name).size(13),
                    text(short(&it.entry.path)).size(11).style(muted),
                    text(detail).size(11).style(move |t: &Theme| text::Style {
                        color: Some(match state_color {
                            Some(true) => t.extended_palette().success.base.color,
                            Some(false) => t.extended_palette().danger.base.color,
                            None => crate::style::muted(t),
                        })
                    }),
                ]
                .spacing(1),
            ]
            .spacing(8)
            .align_y(Alignment::Start)
            .into()
        };
        let mut list = column![].spacing(10);
        if !fixable.is_empty() {
            list = list.push(text("Can be repaired").size(13).font(crate::BOLD));
            for &i in &fixable {
                list = list.push(row_for(i));
            }
        }
        if !unfixable.is_empty() {
            list = list.push(
                button(text(format!("{} Can't be repaired by Truss ({})", if self.show_unfixable { "▾" } else { "▸" }, unfixable.len())).size(13).font(crate::BOLD))
                    .padding([4, 0])
                    .style(button::text)
                    .on_press(Msg::ShowUnfixable(!self.show_unfixable)),
            );
            if self.show_unfixable {
                for &i in &unfixable {
                    list = list.push(row_for(i));
                }
            }
        }
        col = col.push(scrollable(container(list).padding(iced::Padding { right: 16.0, ..Default::default() })).height(Length::Fill));
        container(col).padding([14, 18]).height(Length::Fill).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_badsongs() {
        let t = "Warning: These folders contain lyrics that are not valid UTF-8, and may display missing or incorrect characters in-game.\r\n\
c:\\songs\\a\r\n\r\n\
Warning: These folders contain lyrics that do not have matching vocal note events, and have fallen back to non validated parsing.\r\n\
This warning has no effect on charts currently.\r\n\
c:\\songs\\b\r\n\r\n\
ERROR: These folders contain charts that another song has (duplicate charts)!\r\n\
c:\\songs\\x (y) (z) (c:\\songs\\x (z))\r\n\
ERROR: These folders have no notes.mid or notes.chart files!\r\n\
c:\\songs\\pack\\c\r\n";
        let e = parse(t);
        assert_eq!(e.len(), 4);
        assert_eq!(e[0].kind, Kind::LyricsEncoding);
        assert_eq!(e[1].kind, Kind::LyricsUnvalidated);
        assert_eq!(e[2].kind, Kind::Duplicate);
        assert_eq!(e[2].path, PathBuf::from(r"c:\songs\x (y) (z)"));
        assert_eq!(e[2].other.as_deref(), Some(Path::new(r"c:\songs\x (z)")));
        assert_eq!(e[3].kind, Kind::NoChart);
    }

    #[test]
    fn names_from_folders() {
        assert_eq!(name_from_folder(Path::new(r"C:\S\Artist - Song (Me)")), ("Artist".into(), "Song".into(), "Me".into()));
        assert_eq!(name_from_folder(Path::new(r"C:\S\013. Sabado de Sol - Mamonas Assassinas")).1, "Mamonas Assassinas");
        assert_eq!(name_from_folder(Path::new(r"C:\S\Just A Name")).1, "Just A Name");
    }

    /// Fixes a scratch copy: a misnamed chart, no song.ini, and a too-long folder name, with the
    /// track hashes checked before and after.
    #[test]
    fn repairs_scratch_folder() {
        let Some(src) = std::env::var_os("TRUSS_TEST_REPAIR_CHART") else { return };
        // nosemgrep: a test's own scratch folder
        let root = std::env::temp_dir().join(format!("truss-repair-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let long = "x".repeat(260 - root.as_os_str().len() - 20);
        let dir = root.join(format!("Artist - {long} (Me)"));
        std::fs::create_dir_all(&dir).unwrap();
        let src = PathBuf::from(src);
        let before = library::track_hashes(&src);
        assert!(!before.is_empty());
        for f in std::fs::read_dir(&src).unwrap().flatten() {
            let n = f.file_name().to_string_lossy().to_lowercase();
            if n == "notes.chart" || n == "notes.mid" {
                std::fs::copy(f.path(), dir.join(if n.ends_with(".mid") { "note.mid" } else { "Song.chart" })).unwrap();
            }
        }
        let item = diagnose(vec![Entry { kind: Kind::NoMetadata, path: dir.clone(), other: None }], std::slice::from_ref(&root)).remove(0);
        println!("{:?}", item.fix);
        let steps = item.fix.clone().unwrap();
        assert!(steps.iter().any(|s| matches!(s, Step::ShortenFolder(_))));
        assert!(steps.iter().any(|s| matches!(s, Step::RenameChart { .. })));
        assert!(steps.iter().any(|s| matches!(s, Step::WriteIni(_))));
        let (msg, changes) = apply_blocking(&dir, &steps).unwrap();
        println!("{msg}: {changes:?}");
        let fixed = std::fs::read_dir(&root).unwrap().flatten().next().unwrap().path();
        // Hashes read without a song.ini modifier: the same notes as the source.
        let after = library::track_hashes(&fixed);
        println!("{} tracks; folder now {}", after.len(), fixed.display());
        assert!(!after.is_empty());
        // Undo puts everything back as it was.
        reverse(&changes).unwrap();
        assert!(dir.is_dir() && !fixed.exists());
        let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|f| f.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        println!("after undo: {names:?}");
        assert!(!names.iter().any(|n| n == "song.ini" || n.starts_with("notes.")));
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod dry_run {
    /// TRUSS_TEST_BADSONGS=<badsongs.txt> TRUSS_TEST_LIBRARY=<Songs>: what the tool would do (reads only).
    #[test]
    fn diagnose_real_badsongs() {
        let (Some(f), Some(lib)) = (std::env::var_os("TRUSS_TEST_BADSONGS"), std::env::var_os("TRUSS_TEST_LIBRARY")) else { return };
        let entries = super::parse(&crate::library::decode(&std::fs::read(f).unwrap()));
        let items = super::diagnose(entries, &[std::path::PathBuf::from(lib)]);
        let mut why: std::collections::BTreeMap<String, usize> = Default::default();
        let mut fixes: std::collections::BTreeMap<String, usize> = Default::default();
        for it in &items {
            match &it.fix {
                Ok(steps) => {
                    for s in steps {
                        *fixes.entry(format!("{:?}", std::mem::discriminant(s))).or_default() += 1;
                    }
                    if !matches!(steps[0], super::Step::RemoveCopy { .. }) {
                        println!("FIX {} => {}", it.entry.path.display(), steps.iter().map(|s| s.describe()).collect::<Vec<_>>().join("; "));
                    }
                }
                Err(e) => *why.entry(e.clone()).or_default() += 1,
            }
        }
        println!("{} items; fixes {fixes:?}", items.len());
        for (w, n) in why {
            println!("{n:5}  {w}");
        }
    }
}
