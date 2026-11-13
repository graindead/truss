//! Clone Hero's online leaderboards (leaderboards.clonehero.net). Scores are kept per track
//! (one instrument + difficulty of one exact chart version), keyed by scan-chart's track hash.
//! Encore lists those hashes, but many in an older layout Clone Hero doesn't use, and
//! RhythmVerse lists none, so Truss reads the chart and works them out itself (trackhash.rs).

use crate::api::Chart;
use crate::trackhash::Computed;
use serde::Deserialize;

const API: &str = "https://api.clonehero.net/v1";
pub const SITE: &str = "https://leaderboards.clonehero.net";
const TOP: u32 = 50;

/// One leaderboard you can pick for a chart.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Track {
    /// Leaderboard instrument key ("guitar", "prodrums", …).
    pub instrument: String,
    pub difficulty: String,
    /// Hashes to try in order (base64url, no padding): the one Truss worked out, then Encore's.
    pub hashes: Vec<String>,
}

/// Where a chart's hashes stand.
#[derive(Debug, Clone)]
pub enum Hashes {
    Working,
    Done(Result<Computed, String>),
}

/// Where to read a chart's notes from.
pub enum Source {
    /// An Encore .sng (read by byte range).
    Encore(String),
    #[cfg(feature = "rhythmverse")]
    RhythmVerse(std::sync::Arc<crate::rhythmverse::RvFile>),
    /// A chart folder or .sng already on this computer (only read).
    Local(std::path::PathBuf),
}

impl Source {
    /// RhythmVerse charts can mean downloading a whole archive, so they wait for a click.
    pub fn is_cheap(&self) -> bool {
        matches!(self, Source::Encore(_) | Source::Local(_))
    }
}

pub fn source(c: &Chart) -> Option<Source> {
    #[cfg(feature = "rhythmverse")]
    if let Some(rv) = &c.rv
        && c.notes_data.is_none()
    {
        return Some(Source::RhythmVerse(rv.clone()));
    }
    if c.md5.is_empty() || c.md5.starts_with("rv-") {
        return None;
    }
    Some(Source::Encore(format!(
        "{}/{}{}.sng",
        crate::api::FILES_URL,
        c.md5,
        if c.has_video_background { "_novideo" } else { "" }
    )))
}

/// Reads a chart's notes and works out its track hashes.
pub async fn compute(src: Source) -> Result<Computed, String> {
    match src {
        Source::Encore(url) => {
            let (meta, files) = crate::sng::fetch_remote_notes(&url).await?;
            tokio::task::spawn_blocking(move || crate::trackhash::hash_files(&files, Some(&meta))).await.map_err(|e| e.to_string())?
        }
        Source::Local(path) => tokio::task::spawn_blocking(move || {
            let (files, meta) = local_note_files(&path)?;
            crate::trackhash::hash_files(&files, meta.as_deref())
        })
        .await
        .map_err(|e| e.to_string())?,
        #[cfg(feature = "rhythmverse")]
        Source::RhythmVerse(rv) => {
            let files = crate::rhythmverse::fetch_note_files(&rv).await?;
            tokio::task::spawn_blocking(move || crate::trackhash::hash_files(&files, None)).await.map_err(|e| e.to_string())?
        }
    }
}

type NoteFiles = (Vec<(String, Vec<u8>)>, Option<Vec<(String, String)>>);

/// The notes and song.ini of a chart on disk: a folder's files, or a .sng's entries and metadata.
fn local_note_files(path: &std::path::Path) -> Result<NoteFiles, String> {
    let wanted = |n: &str| {
        let l = n.to_ascii_lowercase();
        l.ends_with(".chart") || l.ends_with(".mid") || l.ends_with(".ini")
    };
    if path.is_dir() {
        let mut files = Vec::new();
        for e in std::fs::read_dir(path).map_err(|e| e.to_string())?.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.file_type().is_ok_and(|t| t.is_file()) && wanted(&name) {
                files.push((name, std::fs::read(e.path()).map_err(|e| e.to_string())?));
            }
        }
        return Ok((files, None));
    }
    let idx = crate::sng::read_index(path)?;
    let mut files = Vec::new();
    for f in idx.files.iter().filter(|f| wanted(&f.name)) {
        files.push((f.name.clone(), crate::sng::read_entry(path, &idx, f)?));
    }
    Ok((files, Some(idx.metadata.clone())))
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Score {
    pub placement: u32,
    pub username: String,
    pub total_score: i64,
    /// "99.8"; missing on some runs.
    pub accuracy: Option<String>,
    pub is_fc: bool,
    pub is_pfc: bool,
    pub notes_hit: u32,
    pub note_count: u32,
    pub speed: u32,
    pub modifiers: Vec<String>,
    pub controller_type: String,
    pub song_hash: String,
}

#[derive(Debug, Clone, Default)]
pub struct Board {
    pub scores: Vec<Score>,
    pub players: u64,
    /// The chart's page on the leaderboards website, when it has any scores.
    pub page: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Resp {
    summary: Summary,
    top: Top,
}
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Summary {
    total_player_count: u64,
}
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Top {
    score_data: Vec<Score>,
}

/// Display name for a leaderboard instrument key.
pub fn instrument_label(key: &str) -> String {
    match key {
        "drums" => "Drums".into(),
        "prodrums" => "Pro Drums".into(),
        k => crate::api::INSTRUMENTS.iter().find(|(a, _)| *a == k).map(|(_, l)| l.to_string()).unwrap_or_else(|| k.into()),
    }
}

pub fn difficulty_label(key: &str) -> String {
    crate::api::DIFFICULTIES.iter().find(|(a, _)| *a == key).map(|(_, l)| l.to_string()).unwrap_or_else(|| key.into())
}

/// The leaderboards a chart has, in the usual instrument and difficulty order: from the hashes
/// Truss worked out (`computed`) and/or the ones Encore lists. Clone Hero keeps separate boards
/// for drums and pro drums on the same drum track.
pub fn tracks(c: &Chart, computed: Option<&Computed>) -> Vec<Track> {
    // (instrument, difficulty) -> hashes, in first-seen order.
    let mut found: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut add = |i: &str, d: &str, h: &str| {
        let h = h.trim_end_matches('=');
        if h.is_empty() {
            return;
        }
        match found.iter_mut().find(|(fi, fd, _)| fi == i && fd == d) {
            Some((_, _, hs)) => {
                if !hs.iter().any(|x| x == h) {
                    hs.push(h.to_string())
                }
            }
            None => found.push((i.to_string(), d.to_string(), vec![h.to_string()])),
        }
    };
    if let Some(cp) = computed {
        for (i, d, h) in &cp.tracks {
            add(i, d, h);
        }
    }
    if let Some(nd) = &c.notes_data {
        for t in &nd.track_hashes {
            add(&t.instrument, &t.difficulty, &t.hash);
        }
    }
    let pro_first = computed.map(|c| c.pro_drums).unwrap_or(c.notes_data.as_ref().is_some_and(|n| n.drum_type == Some(1)));
    let mut out = Vec::new();
    for (inst, diff, hashes) in found {
        let keys = match inst.as_str() {
            "drums" if pro_first => vec!["prodrums", "drums"],
            "drums" => vec!["drums", "prodrums"],
            i => vec![i],
        };
        for k in keys {
            out.push(Track { instrument: k.to_string(), difficulty: diff.clone(), hashes: hashes.clone() });
        }
    }
    let inst_order = |k: &str| match k {
        "prodrums" => crate::api::INSTRUMENTS.iter().position(|(a, _)| *a == "drums").unwrap_or(99),
        k => crate::api::INSTRUMENTS.iter().position(|(a, _)| *a == k).unwrap_or(99),
    };
    let diff_order = |d: &str| crate::api::DIFFICULTIES.iter().position(|(a, _)| *a == d).unwrap_or(99);
    // Stable sort keeps pro drums next to (and in the chosen order with) drums.
    out.sort_by_key(|t| (inst_order(&t.instrument), diff_order(&t.difficulty)));
    out
}

/// The top 50 scores at normal speed (as the website shows by default), from the first of the
/// track's hashes that has any.
pub async fn top(track: Track) -> Result<Board, String> {
    let mut last = Ok(Board::default());
    for hash in &track.hashes {
        last = top_for(hash, &track).await;
        if last.as_ref().is_ok_and(|b| !b.scores.is_empty()) {
            break;
        }
    }
    last
}

async fn top_for(hash: &str, track: &Track) -> Result<Board, String> {
    let url = format!(
        "{API}/scores?trackHash={hash}&instrument={}&difficulty={}&minSpeed=100&maxSpeed=100&topScores={TOP}&sort=score",
        track.instrument, track.difficulty
    );
    let resp = crate::api::CLIENT.get(url).send().await.map_err(|e| format!("Couldn't reach the leaderboards: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("The leaderboards answered {}", resp.status()));
    }
    let r: Resp = resp.json().await.map_err(|e| format!("Unexpected leaderboard reply: {e}"))?;
    let page = r.top.score_data.first().filter(|s| !s.song_hash.is_empty()).map(|s| {
        format!("{SITE}/scores/{}?instrument={}&difficulty={}&minSpeed=100&maxSpeed=100&sort=score", s.song_hash, track.instrument, track.difficulty)
    });
    let mut scores = r.top.score_data;
    scores.truncate(TOP as usize);
    Ok(Board { scores, players: r.summary.total_player_count, page })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_notes_from_a_local_folder() {
        let d = std::env::temp_dir().join(format!("truss-local-notes-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        for (n, b) in [("notes.chart", &b"[Song]"[..]), ("song.ini", b"[song]"), ("guitar.ogg", b"OggS"), ("album.png", b"x")] {
            std::fs::write(d.join(n), b).unwrap();
        }
        let (files, meta) = local_note_files(&d).unwrap();
        let mut names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        names.sort();
        assert_eq!(names, ["notes.chart", "song.ini"]);
        assert!(meta.is_none());
        assert!(Source::Local(d.clone()).is_cheap());
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Metallica - Enter Sandman (oBLINKxONCEo), lead guitar on Expert.
    #[tokio::test]
    #[ignore = "network"]
    async fn live_top() {
        let b = top(Track {
            instrument: "guitar".into(),
            difficulty: "expert".into(),
            hashes: vec!["-Nku724NgARbyAgbfSHvqhPBeskhr5rF-m_FTuv9B5s".into()],
        })
        .await
        .unwrap();
        assert!(!b.scores.is_empty());
        assert_eq!(b.scores[0].placement, 1);
        assert!(b.scores.windows(2).all(|w| w[0].total_score >= w[1].total_score));
        assert!(b.page.unwrap().contains("/scores/"));
    }

    #[test]
    fn drum_tracks_get_both_boards() {
        let mut c = Chart::default();
        c.notes_data = Some(crate::api::NotesData {
            drum_type: Some(1),
            track_hashes: vec![
                crate::api::TrackHash { instrument: "drums".into(), difficulty: "expert".into(), hash: "abc=".into() },
                crate::api::TrackHash { instrument: "guitar".into(), difficulty: "hard".into(), hash: "g".into() },
                crate::api::TrackHash { instrument: "guitar".into(), difficulty: "expert".into(), hash: "G".into() },
            ],
            ..Default::default()
        });
        let t: Vec<_> = tracks(&c, None).into_iter().map(|t| format!("{}/{}/{}", t.instrument, t.difficulty, t.hashes.join("+"))).collect();
        assert_eq!(t, ["guitar/expert/G", "guitar/hard/g", "prodrums/expert/abc", "drums/expert/abc"]);

        // Worked-out hashes come first; Encore's stay as a fallback.
        let cp = Computed {
            tracks: vec![("drums".into(), "expert".into(), "new".into()), ("guitar".into(), "expert".into(), "G".into())],
            pro_drums: false,
            ratings: Vec::new(),
        };
        let t: Vec<_> = tracks(&c, Some(&cp)).into_iter().map(|t| format!("{}/{}/{}", t.instrument, t.difficulty, t.hashes.join("+"))).collect();
        assert_eq!(t, ["guitar/expert/G", "guitar/hard/g", "drums/expert/new+abc", "prodrums/expert/new+abc"]);
    }

    /// The leaderboard's own hash for Natasha Bedingfield - Unwritten (Hyperbolium), worked out
    /// from the RhythmVerse copy on Google Drive.
    #[cfg(feature = "rhythmverse")]
    #[tokio::test]
    #[ignore = "network"]
    async fn live_rhythmverse_hash() {
        let rv = crate::rhythmverse::RvFile {
            external_url: Some("https://drive.google.com/drive/u/0/folders/1T4GBsK5DhnNs6zRY0hViY1zeDj1cCZLV".into()),
            ..Default::default()
        };
        let cp = compute(Source::RhythmVerse(std::sync::Arc::new(rv))).await.unwrap();
        assert!(cp.tracks.iter().any(|(i, d, h)| i == "guitar" && d == "expert" && h == "l_zt2Xyf4HKaBaSXYtTO4OytHpGUwNO4WtOjTCmgmYk"));
    }

    /// Metallica - Nothing Else Matters: Encore lists its guitar hash in the old layout; reading
    /// the .sng by byte range gives the one Clone Hero uses, which has scores.
    #[tokio::test]
    #[ignore = "network"]
    async fn live_encore_rehash() {
        let mut found = None;
        for suffix in ["", "_novideo"] {
            let url = format!("{}/0882accdc2285bf0f7e5f98a3c6fadc6{suffix}.sng", crate::api::FILES_URL);
            if let Ok(cp) = compute(Source::Encore(url)).await {
                found = Some(cp);
                break;
            }
        }
        let cp = found.expect("couldn't read the .sng");
        let (_, _, h) = cp.tracks.iter().find(|(i, d, _)| i == "guitar" && d == "expert").unwrap();
        assert_eq!(h, "t7PGaIlqeeI0vrzQWVRTS77VVvRS1UDF3vBPGfBXiTM");
        let b = top(Track { instrument: "guitar".into(), difficulty: "expert".into(), hashes: vec![h.clone()] }).await.unwrap();
        assert!(!b.scores.is_empty());
    }
}
