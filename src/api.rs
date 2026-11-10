//! Client for the Chorus Encore API (api.enchor.us / files.enchor.us).

use serde::{Deserialize, Serialize};
use std::sync::{Arc, LazyLock};

pub const API_URL: &str = "https://api.enchor.us";
pub const FILES_URL: &str = "https://files.enchor.us";
pub const PER_PAGE: u32 = 50;

/// One shared HTTP client: keeps TLS sessions and HTTP/2 connections warm.
pub static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .user_agent(concat!("truss/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(15))
        .pool_max_idle_per_host(8)
        .build()
        .expect("failed to build HTTP client")
});

/// Drum type filter: (API key, display name).
pub const DRUM_TYPES: &[(&str, &str)] =
    &[("fourLane", "Four Lane"), ("fourLanePro", "Four Lane Pro"), ("fiveLane", "Five Lane")];

/// (API key, display name)
pub const INSTRUMENTS: &[(&str, &str)] = &[
    ("guitar", "Lead Guitar"),
    ("guitarcoop", "Co-op Guitar"),
    ("rhythm", "Rhythm Guitar"),
    ("bass", "Bass"),
    ("drums", "Drums"),
    ("keys", "Keys"),
    ("guitarghl", "GHL Guitar"),
    ("guitarcoopghl", "GHL Co-op"),
    ("rhythmghl", "GHL Rhythm"),
    ("bassghl", "GHL Bass"),
];

pub const DIFFICULTIES: &[(&str, &str)] = &[
    ("expert", "Expert"),
    ("hard", "Hard"),
    ("medium", "Medium"),
    ("easy", "Easy"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum SortKey {
    #[serde(rename = "name")]
    Name,
    #[serde(rename = "artist")]
    Artist,
    #[serde(rename = "album")]
    Album,
    #[serde(rename = "charter")]
    Charter,
    #[serde(rename = "length")]
    Length,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Sort {
    #[serde(rename = "type")]
    pub key: SortKey,
    pub direction: SortDir,
}

/// Fields shared by the quick and advanced search requests.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Common {
    #[serde(rename = "per_page")]
    pub per_page: u32,
    pub page: u32,
    pub instrument: Option<String>,
    pub difficulty: Option<String>,
    pub drum_type: Option<String>,
    pub drums_reviewed: bool,
    pub sort: Option<Sort>,
    pub source: &'static str,
}

impl Common {
    pub fn new(page: u32, instrument: Option<String>, difficulty: Option<String>, sort: Option<Sort>) -> Self {
        Self {
            per_page: PER_PAGE,
            page,
            instrument,
            difficulty,
            drum_type: None,
            drums_reviewed: false,
            sort,
            source: "bridge",
        }
    }
}

#[derive(Debug, Serialize)]
struct SearchRequest<'a> {
    search: &'a str,
    #[serde(flatten)]
    common: Common,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TextFilter {
    pub value: String,
    pub exact: bool,
    pub exclude: bool,
}

/// Body of `POST /search/advanced`. `None` means "don't filter".
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvancedFilters {
    pub name: TextFilter,
    pub artist: TextFilter,
    pub album: TextFilter,
    pub genre: TextFilter,
    pub year: TextFilter,
    pub charter: TextFilter,
    /// Minutes.
    pub min_length: Option<f64>,
    pub max_length: Option<f64>,
    pub min_intensity: Option<f64>,
    pub max_intensity: Option<f64>,
    #[serde(rename = "minAverageNPS")]
    pub min_average_nps: Option<f64>,
    #[serde(rename = "maxAverageNPS")]
    pub max_average_nps: Option<f64>,
    #[serde(rename = "minMaxNPS")]
    pub min_max_nps: Option<f64>,
    #[serde(rename = "maxMaxNPS")]
    pub max_max_nps: Option<f64>,
    /// YYYY-MM-DD, or empty.
    pub modified_after: String,
    /// Comma separated MD5s, or empty.
    pub hash: String,
    pub has_solo_sections: Option<bool>,
    pub has_forced_notes: Option<bool>,
    pub has_open_notes: Option<bool>,
    pub has_tap_notes: Option<bool>,
    pub has_lyrics: Option<bool>,
    pub has_vocals: Option<bool>,
    pub has_roll_lanes: Option<bool>,
    #[serde(rename = "has2xKick")]
    pub has_2x_kick: Option<bool>,
    pub has_issues: Option<bool>,
    pub has_video_background: Option<bool>,
    pub modchart: Option<bool>,
}

#[derive(Debug, Serialize)]
struct AdvancedRequest<'a> {
    #[serde(flatten)]
    filters: &'a AdvancedFilters,
    #[serde(flatten)]
    common: Common,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResponse {
    pub found: u64,
    pub data: Vec<Arc<Chart>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotesData {
    pub instruments: Vec<String>,
    pub has_solo_sections: bool,
    pub has_lyrics: bool,
    pub has_vocals: bool,
    pub has_forced_notes: bool,
    pub has_tap_notes: bool,
    pub has_open_notes: bool,
    #[serde(rename = "has2xKick")]
    pub has_2x_kick: bool,
    pub note_counts: Vec<NoteCount>,
    pub track_hashes: Vec<TrackHash>,
    pub chart_issues: Vec<ChartIssue>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChartIssue {
    pub instrument: Option<String>,
    pub difficulty: Option<String>,
    pub note_issue: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FolderIssue {
    pub folder_issue: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MetadataIssue {
    pub metadata_issue: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TrackHash {
    pub instrument: String,
    pub difficulty: String,
    pub hash: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct NoteCount {
    pub instrument: String,
    pub difficulty: String,
    pub count: u64,
}

/// Only the fields this app uses; everything else in the response is skipped by serde.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Chart {
    pub name: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub year: Option<String>,
    pub charter: Option<String>,
    #[serde(rename = "chartId")]
    pub chart_id: u64,
    pub md5: String,
    #[serde(rename = "albumArtMd5")]
    pub album_art_md5: Option<String>,
    pub song_length: Option<i64>,
    pub diff_band: Option<i32>,
    pub diff_guitar: Option<i32>,
    pub diff_guitar_coop: Option<i32>,
    pub diff_rhythm: Option<i32>,
    pub diff_bass: Option<i32>,
    pub diff_drums: Option<i32>,
    pub diff_keys: Option<i32>,
    pub diff_guitarghl: Option<i32>,
    pub diff_guitar_coop_ghl: Option<i32>,
    pub diff_rhythm_ghl: Option<i32>,
    pub diff_bassghl: Option<i32>,
    pub diff_vocals: Option<i32>,
    pub loading_phrase: Option<String>,
    /// Pack/setlist icon code; empty means "use the charter's icon".
    pub icon: Option<String>,
    pub preview_start_time: Option<i64>,
    /// Seconds.
    pub chart_offset: Option<f64>,
    #[serde(rename = "hasVideoBackground")]
    pub has_video_background: bool,
    #[serde(rename = "modifiedTime")]
    pub modified_time: Option<String>,
    #[serde(rename = "notesData")]
    pub notes_data: Option<NotesData>,
    pub modchart: bool,
    #[serde(rename = "songId")]
    pub song_id: Option<i64>,
    #[serde(rename = "chartHash")]
    pub chart_hash: Option<String>,
    #[serde(rename = "folderIssues")]
    pub folder_issues: Vec<FolderIssue>,
    #[serde(rename = "metadataIssues")]
    pub metadata_issues: Vec<MetadataIssue>,
    #[serde(rename = "packName")]
    pub pack_name: Option<String>,
    #[serde(rename = "applicationUsername")]
    pub application_username: Option<String>,
    #[serde(rename = "applicationDriveId")]
    pub application_drive_id: Option<String>,
    #[serde(rename = "parentFolderId")]
    pub parent_folder_id: Option<String>,
    #[serde(rename = "drivePath")]
    pub drive_path: Option<String>,
    /// Set for results from RhythmVerse (experimental source); `None` for Chorus Encore.
    #[cfg(feature = "rhythmverse")]
    #[serde(skip)]
    pub rv: Option<Arc<crate::rhythmverse::RvFile>>,
}

impl Chart {
    pub fn diff_for(&self, instrument: &str) -> Option<i32> {
        let d = match instrument {
            "guitar" => self.diff_guitar,
            "guitarcoop" => self.diff_guitar_coop,
            "rhythm" => self.diff_rhythm,
            "bass" => self.diff_bass,
            "drums" => self.diff_drums,
            "keys" => self.diff_keys,
            "guitarghl" => self.diff_guitarghl,
            "guitarcoopghl" => self.diff_guitar_coop_ghl,
            "rhythmghl" => self.diff_rhythm_ghl,
            "bassghl" => self.diff_bassghl,
            "vocals" => self.diff_vocals,
            "band" => self.diff_band,
            _ => None,
        };
        d.filter(|d| *d >= 0)
    }

    /// True for RhythmVerse results (only possible in the RhythmVerse build).
    pub fn is_rv(&self) -> bool {
        #[cfg(feature = "rhythmverse")]
        return self.rv.is_some();
        #[cfg(not(feature = "rhythmverse"))]
        false
    }

    /// Results with the same key are versions of one song (same grouping as Bridge).
    pub fn group_key(&self) -> i64 {
        self.song_id.unwrap_or(-(self.chart_id as i64))
    }

    /// "artist|name|charter", lowercased: how Bridge matches search results to the library.
    pub fn library_key(&self) -> String {
        library_key(
            self.artist.as_deref().unwrap_or(""),
            self.name.as_deref().unwrap_or(""),
            self.charter.as_deref().unwrap_or(""),
        )
    }

    /// Where this chart lives on Google Drive: (label, folder link).
    pub fn drive_location(&self) -> Option<(String, String)> {
        let id = self.parent_folder_id.as_ref().or(self.application_drive_id.as_ref())?;
        let source = match (&self.pack_name, &self.application_username) {
            (Some(p), _) => p.clone(),
            (None, Some(u)) => format!("{u}'s Charts"),
            _ => "Google Drive".into(),
        };
        let label = match self.drive_path.as_deref().filter(|p| !p.is_empty()) {
            Some(p) if self.parent_folder_id != self.application_drive_id => format!("{source} / {p}"),
            _ => source,
        };
        Some((label, format!("https://drive.google.com/open?id={id}")))
    }

    /// Issue descriptions grouped under headings, filtered and worded like Bridge's sidebar.
    pub fn issue_groups(&self) -> Vec<(String, Vec<String>)> {
        let mut out = Vec::new();
        let meta: Vec<String> = self
            .metadata_issues
            .iter()
            .filter(|i| i.metadata_issue != "extraValue")
            .map(|i| i.description.clone())
            .collect();
        if !meta.is_empty() {
            out.push(("Metadata issues".to_string(), meta));
        }
        let mut folder: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for i in &self.folder_issues {
            if !["albumArtSize", "invalidIni", "multipleVideo", "badIniLine"].contains(&i.folder_issue.as_str())
                && seen.insert(i.folder_issue.clone())
            {
                folder.push(i.description.clone());
            }
        }
        if !folder.is_empty() {
            out.push(("Chart folder issues".to_string(), folder));
        }
        let Some(nd) = &self.notes_data else { return out };
        // (instrument, difficulty) -> [(noteIssue, count, first description)]
        type Group = ((Option<String>, Option<String>), Vec<(String, usize, String)>);
        let mut groups: Vec<Group> = Vec::new();
        for i in &nd.chart_issues {
            let skip = if i.instrument.is_none() {
                i.note_issue == "isDefaultBPM"
            } else {
                ["badEndEvent", "emptyStarPower", "emptySoloSection", "emptyFlexLane", "babySustain"]
                    .contains(&i.note_issue.as_str())
            };
            if skip {
                continue;
            }
            let key = (i.instrument.clone(), i.difficulty.clone());
            let gi = match groups.iter().position(|(k, _)| *k == key) {
                Some(p) => p,
                None => {
                    groups.push((key, Vec::new()));
                    groups.len() - 1
                }
            };
            let list = &mut groups[gi].1;
            match list.iter_mut().find(|(n, _, _)| *n == i.note_issue) {
                Some(e) => e.1 += 1,
                None => list.push((i.note_issue.clone(), 1, i.description.clone())),
            }
        }
        for ((inst, diff), issues) in groups {
            let heading = match inst {
                None => "Chart issues".to_string(),
                Some(inst) => {
                    let name = INSTRUMENTS.iter().find(|(k, _)| *k == inst).map(|(_, l)| l.to_string()).unwrap_or(inst);
                    let d = diff.map(|d| capitalize(&d)).unwrap_or_else(|| "(all difficulties)".into());
                    format!("{name} - {d} issues")
                }
            };
            let lines = issues.into_iter().map(|(kind, n, desc)| issue_text(&kind, n, desc)).collect();
            out.push((heading, lines));
        }
        out
    }

    pub fn length_str(&self) -> String {
        match self.song_length {
            Some(ms) if ms > 0 => {
                let s = ms / 1000;
                format!("{}:{:02}", s / 60, s % 60)
            }
            _ => String::new(),
        }
    }
}

/// Formatting tags (`<color=#A7CEED>NEVER ENOUGH</color>`) and extra spaces are ignored, since a
/// song.ini often has them where Chorus lists the plain text.
pub fn library_key(artist: &str, name: &str, charter: &str) -> String {
    let norm = |s: &str| strip_tags(s).split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    format!("{}|{}|{}", norm(artist), norm(name), norm(charter))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Bridge's summarised wording for repeated issues.
fn issue_text(kind: &str, n: usize, desc: String) -> String {
    let one = n == 1;
    let (is, s) = if one { ("is", "") } else { ("are", "s") };
    match kind {
        "misalignedTimeSignature" => format!("There {is} {n} misaligned time signature marker{s} in this chart."),
        "badEndEvent" => format!("There {is} {n} invalid \"end\" event{s} in this chart."),
        "badStarPower" => {
            format!("There {is} {n} ignored star power event{s} due to the .ini \"multiplier_note\" setting.")
        }
        "difficultyForbiddenNote" => format!("There {is} {n} note{s} that {is}n't allowed on this track's difficulty."),
        "invalidChord" => format!("There {is} {n} chord{s} that {is}n't allowed for this instrument type."),
        "brokenNote" => format!("There {is} {n} broken note{s} on this track."),
        "badSustainGap" if one => "There is 1 note that has a sustain gap that is too small.".into(),
        "badSustainGap" => format!("There are {n} notes that have sustain gaps that are too small."),
        _ => desc,
    }
}

/// Strips Clone Hero rich-text tags like `<color=#FF0000>` or `<b>` for display.
pub fn strip_tags(s: &str) -> String {
    if !s.contains('<') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        match rest[start..].find('>') {
            Some(end) => rest = &rest[start + end + 1..],
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

pub async fn search(query: String, common: Common) -> Result<SearchResponse, String> {
    let body = SearchRequest { search: if query.trim().is_empty() { "*" } else { query.trim() }, common };
    post_search("search", &body).await
}

pub async fn advanced_search(filters: Arc<AdvancedFilters>, common: Common) -> Result<SearchResponse, String> {
    post_search("search/advanced", &AdvancedRequest { filters: &filters, common }).await
}

async fn post_search(path: &str, body: &impl Serialize) -> Result<SearchResponse, String> {
    let mut last_err = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(800 * attempt)).await;
        }
        let resp = CLIENT.post(format!("{API_URL}/{path}")).json(body).send().await;
        match resp {
            Ok(r) if r.status().is_success() => {
                let bytes = r.bytes().await.map_err(|e| e.to_string())?;
                return serde_json::from_slice(&bytes).map_err(|e| format!("Bad response: {e}"));
            }
            Ok(r) if r.status().as_u16() == 400 => {
                let msg = r.text().await.unwrap_or_default();
                return Err(format!("Invalid search: {}", msg.chars().take(300).collect::<String>()));
            }
            Ok(r) => last_err = format!("Server returned {}", r.status()),
            Err(e) => last_err = e.to_string(),
        }
    }
    Err(last_err)
}

/// Downloads album art and decodes a small thumbnail off the UI thread.
/// The full image is kept compressed (~50-150 KB) and decoded on demand only
/// for the selected chart, instead of holding 1 MB of RGBA per result.
pub async fn fetch_album_art(md5: String, thumb_px: u32) -> Result<(DecodedImage, bytes::Bytes), String> {
    // RhythmVerse results carry their album art's full address instead of an Encore hash.
    let url = if md5.starts_with("https://") { md5.clone() } else { format!("{FILES_URL}/{md5}.jpg") };
    let bytes = CLIENT
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;

    tokio::task::spawn_blocking(move || {
        let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
        let thumb = img.thumbnail(thumb_px, thumb_px).to_rgba8();
        Ok((DecodedImage { width: thumb.width(), height: thumb.height(), rgba: thumb.into_raw() }, bytes))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn library_key_ignores_tags_and_spaces() {
        assert_eq!(
            library_key("Turnstile", "<color=#A7CEED>NEVER ENOUGH</color>", "Pix_"),
            library_key("Turnstile ", "NEVER  ENOUGH", "pix_")
        );
    }

    use super::*;
    #[test]
    fn advanced_body_matches_api() {
        let mut f = AdvancedFilters::default();
        f.artist.value = "dragonforce".into();
        f.min_length = Some(6.0);
        f.has_solo_sections = Some(true);
        let c = Common::new(1, None, None, Some(Sort { key: SortKey::Length, direction: SortDir::Desc }));
        let json = serde_json::to_string(&AdvancedRequest { filters: &f, common: c }).unwrap();
        for key in [
            r#""artist":{"value":"dragonforce","exact":false,"exclude":false}"#,
            r#""minLength":6.0"#,
            r#""hasSoloSections":true"#,
            r#""has2xKick":null"#,
            r#""minAverageNPS":null"#,
            r#""per_page":50"#,
            r#""drumsReviewed":false"#,
            r#""sort":{"type":"length","direction":"desc"}"#,
        ] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
    }
}
