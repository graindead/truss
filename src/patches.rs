//! Game setlist patches (Guitar Hero III's Explicit Patch, say): a download of a few audio
//! files that go into charts already in the library, not charts of its own. Applying one
//! moves every file it would overwrite into Truss's cache folder (`patch-backups`) first, and
//! Revert takes the patch's files out and puts those originals back. Nothing of the user's is
//! deleted.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// True for a game setlist download that patches charts instead of being charts.
pub fn is_patch(label: &str) -> bool {
    label.to_ascii_lowercase().contains("patch")
}

/// What a patch did, kept with its backups so Revert can undo it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub url: String,
    pub game: String,
    /// Unix seconds.
    pub applied: u64,
    /// The song folders it went into.
    pub songs: Vec<PathBuf>,
    pub files: Vec<Placed>,
}

/// One file the patch put in place.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Placed {
    pub target: PathBuf,
    /// The file it replaced, moved into the patch's backup folder (relative to it); none when
    /// the patch added a file that wasn't there.
    pub backup: Option<PathBuf>,
}

/// The library, for finding the charts a patch goes into.
pub struct Library {
    /// Every chart's library key (artist|name|charter) and folder.
    pub charts: Vec<(String, PathBuf)>,
    /// The game's own folders start with this (`<packs>/Game Setlists/<game>`, and its
    /// " - DLC" and other downloads): charts there are the game's.
    pub game_prefix: Option<PathBuf>,
    /// Library keys of the game's official charts (from the game sources list), for charts
    /// installed elsewhere.
    pub official: HashSet<String>,
}

fn root() -> Option<PathBuf> {
    crate::settings::cache_dir().map(|d| d.join("patch-backups"))
}

/// A stable folder name for a patch link (FNV-1a of the URL).
fn id(url: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in url.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn dir_for(url: &str) -> Option<PathBuf> {
    root().map(|r| r.join(id(url)))
}

/// The patch's record if it's applied.
pub fn applied(url: &str) -> Option<Manifest> {
    let bytes = std::fs::read(dir_for(url)?.join("manifest.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save(dir: &Path, m: &Manifest) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(m).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("manifest.json"), json).map_err(|e| format!("Couldn't save the patch's record: {e}"))
}

/// Moves a file, copying it when it's on another drive (the cache is usually on C:).
fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(p) = to.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    let n = std::fs::copy(from, to).map_err(|e| format!("Couldn't copy {}: {e}", from.display()))?;
    // Only remove the original once its copy is whole.
    let len = std::fs::metadata(from).map(|m| m.len()).map_err(|e| e.to_string())?;
    if n != len {
        let _ = std::fs::remove_file(to);
        return Err(format!("Couldn't copy {} whole", from.display()));
    }
    std::fs::remove_file(from).map_err(|e| format!("Couldn't move {}: {e}", from.display()))
}

/// The patch's song folders (folders holding files, named "Artist - Title") and their files.
fn song_folders(dir: &Path) -> HashMap<PathBuf, Vec<PathBuf>> {
    let mut out: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push(p);
            } else if d != dir && d.file_name().is_some_and(|n| n.to_string_lossy().contains(" - ")) {
                out.entry(d.clone()).or_default().push(p);
            }
        }
    }
    out
}

/// The library's copies of the song a patch folder is named after: the game's own (in its
/// Game Setlists folder) if there are any, else ones charted by the game's makers. A
/// community chart of the same song is left alone.
fn targets(folder_name: &str, lib: &Library) -> Vec<PathBuf> {
    use crate::game_sources::loose;
    let Some((artist, title)) = folder_name.split_once(" - ") else { return Vec::new() };
    let (artist, title) = (loose(artist), loose(title));
    let same: Vec<&(String, PathBuf)> = lib
        .charts
        .iter()
        .filter(|(k, p)| {
            let mut parts = k.split('|');
            p.is_dir() && parts.next().is_some_and(|a| loose(a) == artist) && parts.next().is_some_and(|n| loose(n) == title)
        })
        .collect();
    // Case and either slash: Windows paths, and "Game Setlists/<game>" is written with '/'.
    let lower = |p: &Path| p.to_string_lossy().to_lowercase().replace('\\', "/");
    let in_game: Vec<PathBuf> = match &lib.game_prefix {
        Some(g) => {
            let g = lower(g);
            same.iter().filter(|(_, p)| lower(p).starts_with(&g)).map(|(_, p)| p.clone()).collect()
        }
        None => Vec::new(),
    };
    if !in_game.is_empty() {
        return in_game;
    }
    same.iter().filter(|(k, _)| lib.official.contains(k)).map(|(_, p)| p.clone()).collect()
}

/// What a patch run did, for the Packs page.
pub struct Report {
    pub songs: usize,
    pub added: usize,
    pub replaced: usize,
    pub missing: Vec<String>,
}

impl Report {
    pub fn message(&self) -> String {
        let mut s = match (self.added, self.replaced) {
            (a, 0) => format!("Patched {} songs ({a} files added).", self.songs),
            (0, r) => format!("Patched {} songs ({r} files replaced; the originals are kept for Revert).", self.songs),
            (a, r) => format!("Patched {} songs ({a} files added, {r} replaced; the originals are kept for Revert).", self.songs),
        };
        if !self.missing.is_empty() {
            s.push_str(&format!(" Not in your library: {}.", self.missing.join(", ")));
        }
        s
    }
}

/// Downloads the patch and puts its files into the library's copies of its songs.
#[cfg(feature = "rhythmverse")]
pub async fn apply(url: String, game: String, lib: Library) -> Result<Report, String> {
    if applied(&url).is_some() {
        return Err("This patch is already applied.".into());
    }
    let dir = dir_for(&url).ok_or("Truss's cache folder isn't available.")?;
    let work = root().ok_or("Truss's cache folder isn't available.")?.join(format!("tmp-{}", std::process::id()));
    let _guard = crate::download::TempGuard(Some(work.clone()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("Couldn't write to Truss's cache: {e}"))?;
    let unpacked = work.join("patch");
    // Nobody shows this progress: the sends just fail.
    let (mut tx, _) = futures::channel::mpsc::channel(1);
    crate::rhythmverse::fetch_pack(&url, &work, &unpacked, false, &mut tx).await?;
    crate::rhythmverse::unpack_nested(&unpacked, false).await?;
    tokio::task::spawn_blocking(move || place(&url, &game, &unpacked, &dir, &lib)).await.map_err(|e| e.to_string())?
}

#[cfg(not(feature = "rhythmverse"))]
pub async fn apply(_url: String, _game: String, _lib: Library) -> Result<Report, String> {
    Err("This build of Truss can't download patches.".into())
}

/// Puts the unpacked patch's files in place, backing up what they replace.
fn place(url: &str, game: &str, unpacked: &Path, dir: &Path, lib: &Library) -> Result<Report, String> {
    let songs = song_folders(unpacked);
    if songs.is_empty() {
        return Err("The patch has no song folders in it.".into());
    }
    let mut names: Vec<(String, &Vec<PathBuf>)> =
        songs.iter().map(|(d, f)| (d.file_name().unwrap_or_default().to_string_lossy().into_owned(), f)).collect();
    names.sort_by(|a, b| a.0.cmp(&b.0));
    let mut report = Report { songs: 0, added: 0, replaced: 0, missing: Vec::new() };
    let plan: Vec<(PathBuf, &Vec<PathBuf>)> = names
        .iter()
        .flat_map(|(name, files)| {
            let t = targets(name, lib);
            if t.is_empty() {
                report.missing.push(name.clone());
            }
            t.into_iter().map(move |t| (t, *files))
        })
        .collect();
    if plan.is_empty() {
        return Err(format!("None of the patch's songs are in your library: {}. Install the game first.", report.missing.join(", ")));
    }
    std::fs::create_dir_all(dir.join("files")).map_err(|e| format!("Couldn't write to Truss's cache: {e}"))?;
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut m = Manifest { url: url.into(), game: game.into(), applied: secs, songs: Vec::new(), files: Vec::new() };
    let mut result = Ok(());
    'songs: for (song, files) in &plan {
        m.songs.push(song.clone());
        report.songs += 1;
        for f in files.iter() {
            let target = song.join(f.file_name().unwrap_or_default());
            let backup = if target.exists() {
                let rel = PathBuf::from("files").join(format!("{}-{}", m.files.len(), f.file_name().unwrap_or_default().to_string_lossy()));
                if let Err(e) = move_file(&target, &dir.join(&rel)) {
                    result = Err(e);
                    break 'songs;
                }
                report.replaced += 1;
                Some(rel)
            } else {
                report.added += 1;
                None
            };
            let copied = std::fs::copy(f, &target).map_err(|e| format!("Couldn't write {}: {e}", target.display()));
            m.files.push(Placed { target, backup });
            if let Err(e) = copied {
                result = Err(e);
                break 'songs;
            }
        }
    }
    // Saved even when something failed part way, so Revert can undo what was done.
    save(dir, &m)?;
    result.map(|()| report).map_err(|e| format!("{e} Revert undoes what was patched."))
}

/// Takes the patch's files out and puts the originals back.
pub async fn revert(url: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || revert_blocking(&url)).await.map_err(|e| e.to_string())?
}

fn revert_blocking(url: &str) -> Result<String, String> {
    revert_in(&dir_for(url).ok_or("Truss's cache folder isn't available.")?)
}

fn revert_in(dir: &Path) -> Result<String, String> {
    let bytes = std::fs::read(dir.join("manifest.json")).map_err(|_| "This patch isn't applied.")?;
    let mut m: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let mut left = Vec::new();
    let mut restored = 0;
    for p in m.files.drain(..).rev() {
        // The patch's own file: it can always be downloaded again.
        if p.target.exists() && std::fs::remove_file(&p.target).is_err() {
            left.push(p);
            continue;
        }
        if let Some(b) = &p.backup {
            if !p.target.parent().is_some_and(Path::is_dir) || move_file(&dir.join(b), &p.target).is_err() {
                left.push(p);
                continue;
            }
            restored += 1;
        }
    }
    if left.is_empty() {
        let _ = std::fs::remove_dir_all(dir);
        Ok(match restored {
            0 => format!("Reverted: the patch's files are out of {} songs.", m.songs.len()),
            n => format!("Reverted: the patch's files are out of {} songs and {n} original files are back.", m.songs.len()),
        })
    } else {
        left.reverse();
        let n = left.len();
        m.files = left;
        save(dir, &m)?;
        Err(format!(
            "{n} files couldn't be put back (is a song folder gone or in use?). Their originals are still in {}.",
            dir.join("files").display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib(charts: &[(&str, &Path)], game: Option<&Path>, official: &[&str]) -> Library {
        Library {
            charts: charts.iter().map(|(k, p)| (k.to_string(), p.to_path_buf())).collect(),
            game_prefix: game.map(Path::to_path_buf),
            official: official.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Network: downloads GH3's Explicit Patch and applies it to a scratch library
    /// (TRUSS_TEST_PATCH_LIB, holding "Game Setlists/<game>/..."), then reverts it.
    #[test]
    #[ignore]
    fn gh3_explicit_patch_live() {
        let Some(base) = std::env::var_os("TRUSS_TEST_PATCH_LIB").map(PathBuf::from) else { return };
        let url = "https://drive.google.com/file/d/1_9gv-mto6YChasYE2nCZcvDxt5UNgyRL/view?usp=drive_link".to_string();
        let game = "Guitar Hero III: Legends of Rock".to_string();
        let scanned = crate::library::scan_blocking(std::slice::from_ref(&base));
        let lib = Library {
            charts: scanned.keys.iter().flat_map(|(k, ps)| ps.iter().map(move |p| (k.clone(), p.to_path_buf()))).collect(),
            game_prefix: Some(base.join(crate::game_setlists::folder(&game, crate::game_setlists::MAIN))),
            official: HashSet::new(),
        };
        println!("{} charts in the scratch library", lib.charts.len());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let r = rt.block_on(apply(url.clone(), game, lib)).unwrap_or_else(|e| panic!("{e}"));
        println!("{}", r.message());
        assert!(r.missing.is_empty() && r.songs == 7);
        let m = applied(&url).unwrap();
        assert!(m.files.iter().all(|p| p.target.exists()));
        println!("{}", rt.block_on(revert(url.clone())).unwrap());
        assert!(applied(&url).is_none());
        assert!(m.files.iter().all(|p| p.backup.is_some() == p.target.exists()));
    }

    #[test]
    fn patches_and_reverts_on_scratch_copies() {
        // nosemgrep: a test's own scratch folder
        let base = std::env::temp_dir().join(format!("truss-patch-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let game = base.join("Game Setlists").join("Guitar Hero III");
        let song = game.join("Quickplay").join("Pearl Jam - Even Flow");
        let other = base.join("Community").join("Pearl Jam - Even Flow");
        let solo = base.join("Elsewhere").join("Beastie Boys - Sabotage");
        for d in [&song, &other, &solo] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join("song.ogg"), b"original song").unwrap();
        }
        let patch = base.join("unpacked").join("GH3").join("Quickplay");
        for (name, files) in [("Pearl Jam - Even Flow", &["vocals_2.opus", "song.ogg"][..]), ("Beastie Boys - Sabotage", &["vocals_2.opus"]), ("Nobody - Nothing", &["x.opus"])] {
            std::fs::create_dir_all(patch.join(name)).unwrap();
            for f in files {
                std::fs::write(patch.join(name).join(f), format!("patched {f}")).unwrap();
            }
        }
        std::fs::write(base.join("unpacked").join("readme.txt"), b"hi").unwrap();
        let l = lib(
            &[
                ("pearl jam|even flow|neversoft", &song),
                ("pearl jam|even flow|someone", &other),
                ("beastie boys|sabotage|neversoft", &solo),
                ("beastie boys|sabotage|fan", &other),
            ],
            Some(&game),
            &["beastie boys|sabotage|neversoft"],
        );
        let backups = base.join("backups");
        let r = place("u", "GH3", &base.join("unpacked"), &backups, &l).unwrap();
        assert_eq!((r.songs, r.added, r.replaced), (2, 2, 1));
        assert_eq!(r.missing, vec!["Nobody - Nothing".to_string()]);
        assert_eq!(std::fs::read(song.join("song.ogg")).unwrap(), b"patched song.ogg");
        assert!(song.join("vocals_2.opus").exists() && solo.join("vocals_2.opus").exists());
        assert!(!other.join("vocals_2.opus").exists(), "a community chart of the song is left alone");
        let m: Manifest = serde_json::from_slice(&std::fs::read(backups.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(m.files.len(), 3);
        assert!(backups.join("files").read_dir().unwrap().count() == 1, "the replaced original is kept");
        revert_in(&backups).unwrap();
        assert!(!backups.exists());
        assert_eq!(std::fs::read(song.join("song.ogg")).unwrap(), b"original song");
        assert!(!song.join("vocals_2.opus").exists() && !solo.join("vocals_2.opus").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
