//! Library clean-ups: removing loose copies of charts that are also in a pack, and removing
//! video backgrounds. Both move files to the Recycle Bin, and neither touches a pack's folders
//! (only the loose copy, or the video files inside chart folders).

use crate::library::{self, LocalChart};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A chart outside the packs whose notes match a pack's copy exactly.
#[derive(Debug, Clone)]
pub struct Duplicate {
    /// The loose copy, to remove.
    pub loose: PathBuf,
    /// The pack's copy, kept.
    pub pack: PathBuf,
    pub artist: String,
    pub name: String,
    pub charter: String,
}

/// Whether `path` is a loose chart: one sitting straight in a library folder (or the pack
/// downloads folder), the way single downloads land. Anything in a subfolder belongs to a pack
/// or some other set the user arranged (tiers and all), and is never treated as loose.
pub fn is_loose(path: &Path, roots: &[PathBuf]) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let parent = path.parent().map(norm);
    parent.is_some_and(|parent| roots.iter().any(|r| norm(r) == parent))
}

/// Charts that are exact copies of another: the same chart file (by its hash). Returns, for
/// each catalog entry, whether it's in such a group, and the spare copies to remove: every copy
/// but one in each group. The one kept is the one with the most extras (album art, background,
/// video, lyrics), then the oldest. Copies inside packs are never spares (their folders stay as
/// the pack has them); when a pack has one, every loose copy is spare.
pub fn exact_copies(catalog: &[Arc<LocalChart>], roots: &[PathBuf], lyrics: &HashMap<PathBuf, bool>) -> (Vec<bool>, Vec<usize>) {
    let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, c) in catalog.iter().enumerate() {
        if let Some(h) = c.chart_hash.as_deref() {
            groups.entry(h).or_default().push(i);
        }
    }
    let mut in_group = vec![false; catalog.len()];
    let mut spares = Vec::new();
    for members in groups.values().filter(|m| m.len() > 1) {
        for &i in members {
            in_group[i] = true;
        }
        let (loose, packed): (Vec<usize>, Vec<usize>) = members.iter().partition(|&&i| is_loose(&catalog[i].path, roots));
        let extras = |i: usize| {
            let c = &catalog[i];
            [c.has_album_art, c.has_background, c.has_video, lyrics.get(&c.path) == Some(&true)].iter().filter(|b| **b).count()
        };
        let keep = if packed.is_empty() {
            loose.iter().copied().max_by_key(|&i| (extras(i), std::cmp::Reverse(catalog[i].added)))
        } else {
            None
        };
        spares.extend(loose.into_iter().filter(|&i| Some(i) != keep));
    }
    spares.sort_unstable();
    (in_group, spares)
}

/// Finds loose charts that are also in a pack. A pair only counts when every track's notes
/// hash the same (the same tracks, each with identical notes); a title, file or song.ini match
/// alone is never enough.
pub fn find_pack_duplicates(catalog: &[Arc<LocalChart>], roots: &[PathBuf]) -> Vec<Duplicate> {
    let (loose, packed): (Vec<&Arc<LocalChart>>, Vec<&Arc<LocalChart>>) = catalog.iter().partition(|c| is_loose(&c.path, roots));
    // Pack charts that could be the same song, found by file hash or by title and charter.
    let mut by_hash: HashMap<&str, Vec<&Arc<LocalChart>>> = HashMap::new();
    let mut by_song: HashMap<String, Vec<&Arc<LocalChart>>> = HashMap::new();
    for c in &packed {
        if let Some(h) = &c.chart_hash {
            by_hash.entry(h.as_str()).or_default().push(c);
        }
        if let Some(k) = song_key(c) {
            by_song.entry(k).or_default().push(c);
        }
    }
    let mut pairs: Vec<(&Arc<LocalChart>, Vec<&Arc<LocalChart>>)> = Vec::new();
    for c in loose.iter().filter(|c| library::is_deletable_chart(&c.path, roots)) {
        let mut cands: Vec<&Arc<LocalChart>> = c.chart_hash.as_deref().and_then(|h| by_hash.get(h)).cloned().unwrap_or_default();
        let artist = library::artist_words(&c.artist);
        for p in song_key(c).and_then(|k| by_song.get(&k)).into_iter().flatten() {
            if library::same_artist(&library::artist_words(&p.artist), &artist) && !cands.iter().any(|x| x.path == p.path) {
                cands.push(p);
            }
        }
        if !cands.is_empty() {
            pairs.push((c, cands));
        }
    }
    // Hash every chart involved once, a few at a time.
    let mut wanted: Vec<PathBuf> = pairs.iter().flat_map(|(c, cs)| std::iter::once(&c.path).chain(cs.iter().map(|p| &p.path))).cloned().collect();
    wanted.sort();
    wanted.dedup();
    let tracks = hash_all(&wanted);
    pairs
        .into_iter()
        .filter_map(|(c, cands)| {
            let mine = tracks.get(&c.path).filter(|t| !t.is_empty())?;
            let p = cands.into_iter().find(|p| tracks.get(&p.path) == Some(mine))?;
            Some(Duplicate { loose: c.path.clone(), pack: p.path.clone(), artist: c.artist.clone(), name: c.name.clone(), charter: c.charter.clone() })
        })
        .collect()
}

fn song_key(c: &LocalChart) -> Option<String> {
    let t = c.tagged.as_deref();
    library::song_key(t.map(|t| t.name.as_str()).unwrap_or(&c.name), t.map(|t| t.charter.as_str()).unwrap_or(&c.charter))
}

fn hash_all(paths: &[PathBuf]) -> HashMap<PathBuf, Vec<String>> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
    let chunk = paths.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = paths
            .chunks(chunk)
            .map(|part| s.spawn(move || part.iter().map(|p| (p.clone(), library::track_hashes(p))).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).flatten().collect()
    })
}

/// How many video backgrounds there are to remove, and how many are inside .sng files (which
/// Truss leaves alone, since it would have to rewrite the file).
pub fn video_counts(catalog: &[Arc<LocalChart>]) -> (usize, usize) {
    let sng = catalog.iter().filter(|c| c.has_video && c.is_sng()).count();
    (catalog.iter().filter(|c| c.has_video).count() - sng, sng)
}

/// The video background files of every chart folder that has one.
pub fn video_files(catalog: &[Arc<LocalChart>]) -> Vec<PathBuf> {
    catalog
        .iter()
        .filter(|c| c.has_video && !c.is_sng())
        .flat_map(|c| {
            std::fs::read_dir(&c.path).into_iter().flatten().flatten().filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                let (stem, ext) = n.rsplit_once('.')?;
                (stem == "video" && library::VIDEO_EXT.contains(&ext) && e.file_type().is_ok_and(|t| t.is_file())).then(|| e.path())
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_copies_keep_the_best_and_spare_packs() {
        let chart = |path: &str, hash: &str, art: bool, added: u64| {
            let mut c = crate::library::test_chart("Band", "Song", added);
            c.path = PathBuf::from(path);
            c.chart_hash = Some(hash.into());
            c.has_album_art = art;
            Arc::new(c)
        };
        let roots = vec![PathBuf::from(r"C:\Songs")];
        let cat = vec![
            // Two loose copies: the one with album art is kept.
            chart(r"C:\Songs\A", "h1", false, 10),
            chart(r"C:\Songs\A (2)", "h1", true, 20),
            // A loose copy and one in a pack: the pack's stays, the loose one is spare.
            chart(r"C:\Songs\B", "h2", true, 10),
            chart(r"C:\Songs\Pack\Tier 1\B", "h2", false, 30),
            // Two equal loose copies: the older is kept.
            chart(r"C:\Songs\C", "h3", false, 50),
            chart(r"C:\Songs\C (2)", "h3", false, 40),
            // Not a copy of anything.
            chart(r"C:\Songs\D", "h4", true, 10),
        ];
        let (in_group, spares) = exact_copies(&cat, &roots, &HashMap::new());
        assert_eq!(spares, vec![0, 2, 4]);
        assert_eq!(in_group, vec![true, true, true, true, true, true, false]);
    }

    #[test]
    fn only_top_level_charts_are_loose() {
        let roots = [PathBuf::from(r"C:\Songs"), PathBuf::from(r"C:\Songs\!!1 Packs")];
        assert!(is_loose(Path::new(r"C:\Songs\Artist - Song (Me)"), &roots));
        assert!(is_loose(Path::new(r"C:\Songs\Artist - Song (Me).sng"), &roots));
        assert!(!is_loose(Path::new(r"C:\Songs\Rage Quit\Tier 1\Song"), &roots));
        assert!(!is_loose(Path::new(r"C:\Songs\Anti Hero\Tier 40\Song"), &roots));
        assert!(!is_loose(Path::new(r"C:\Songs\!!1 Packs\Rage Quit\Song"), &roots));
        // A single chart in the pack folder itself is loose too.
        assert!(is_loose(Path::new(r"C:\Songs\!!1 Packs\Artist - Song"), &roots));
    }

    /// TRUSS_TEST_DUP_LIBRARY=<folder>: list the pack duplicates in a library (read only).
    #[test]
    fn finds_pack_duplicates() {
        let Some(dir) = std::env::var_os("TRUSS_TEST_DUP_LIBRARY") else { return };
        let root = PathBuf::from(dir);
        let lib = library::scan_blocking(std::slice::from_ref(&root));
        let t = std::time::Instant::now();
        let d = find_pack_duplicates(&lib.catalog, std::slice::from_ref(&root));
        for x in &d {
            assert!(is_loose(&x.loose, std::slice::from_ref(&root)) && !is_loose(&x.pack, std::slice::from_ref(&root)));
        }
        println!("{} duplicates of {} charts in {:?}", d.len(), lib.charts, t.elapsed());
        for x in d.iter().take(40) {
            println!("{} - {} ({})\n   remove {}\n   keep   {}", x.artist, x.name, x.charter, x.loose.display(), x.pack.display());
        }
        let (v, s) = video_counts(&lib.catalog);
        println!("{v} chart folders with videos, {s} .sng with videos");
    }
}
