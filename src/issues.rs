//! Chart issue scanner (Tools > Issue scanner): a port of scan-chart's folder, metadata and
//! chart checks, plus the same Excel report the original Bridge writes.

use crate::chart::{self, InstrumentType, Modifiers, ParsedChart, Track, instrument_type, nf, nt};
use std::path::{Path, PathBuf};

/// One row of the report.
#[derive(Debug, Clone)]
pub struct IssueRow {
    pub path: String,
    pub artist: String,
    pub name: String,
    pub charter: String,
    pub issue: String,
    pub description: String,
    pub mandatory: bool,
}

struct Issues {
    folder: Vec<(&'static str, String)>,
    metadata: Vec<(&'static str, String)>,
    /// (instrument, difficulty, type, description)
    chart: Vec<(Option<String>, Option<String>, &'static str, String)>,
}

/// A chart to scan: a folder of files, or a .sng file.
pub struct ChartSource {
    pub path: PathBuf,
    pub sng: Option<PathBuf>,
}

/// Every chart folder / .sng below `root` (scan-chart's `getChartFolders`).
pub fn find_charts(root: &Path) -> Vec<ChartSource> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut subdirs = 0;
        let mut exts = Vec::new();
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                if name != "__MACOSX" {
                    subdirs += 1;
                    stack.push(e.path());
                }
                continue;
            }
            let ext = ext_of(&name);
            if ext == "sng" {
                out.push(ChartSource { path: dir.clone(), sng: Some(e.path()) });
            }
            exts.push(ext);
        }
        let notes = exts.iter().any(|e| e == "chart" || e == "mid");
        let audio = exts.iter().any(|e| AUDIO.contains(&e.as_str()));
        if subdirs == 0 && (notes || audio) {
            out.push(ChartSource { path: dir, sng: None });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path).then(a.sng.cmp(&b.sng)));
    out
}

const AUDIO: &[&str] = &["ogg", "mp3", "wav", "opus"];

fn ext_of(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_string()).unwrap_or_default()
}
fn basename(name: &str) -> &str {
    name.rsplit_once('.').map(|(b, _)| b).unwrap_or(name)
}

/// A file in the chart: its name and (only for the few files that are read) its contents.
struct ChartFile {
    name: String,
    data: Option<Vec<u8>>,
}

fn load_files(src: &ChartSource) -> Result<Vec<ChartFile>, String> {
    let wanted = |n: &str| {
        let e = ext_of(n).to_ascii_lowercase();
        matches!(e.as_str(), "ini" | "chart" | "mid") || matches!(n, "album.jpg" | "album.jpeg" | "album.png")
    };
    if let Some(sng) = &src.sng {
        let idx = crate::sng::read_index(sng)?;
        let mut files: Vec<ChartFile> = idx
            .files
            .iter()
            .map(|f| ChartFile {
                name: f.name.clone(),
                data: if wanted(&f.name) { crate::sng::read_entry(sng, &idx, f).ok() } else { None },
            })
            .collect();
        // Like parse-sng with `generateSongIni`, metadata becomes a song.ini.
        files.push(ChartFile { name: "song.ini".into(), data: Some(crate::sng::generate_ini(&idx.metadata).into_bytes()) });
        Ok(files)
    } else {
        let rd = std::fs::read_dir(&src.path).map_err(|e| e.to_string())?;
        Ok(rd
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let data = if wanted(&name) { std::fs::read(e.path()).ok() } else { None };
                ChartFile { name, data }
            })
            .collect())
    }
}

// ---------- song.ini (scan-ini.ts) ----------

const INTEGER_KEYS: &[&str] = &[
    "song_length",
    "diff_band",
    "diff_guitar",
    "diff_guitar_coop",
    "diff_rhythm",
    "diff_bass",
    "diff_drums",
    "diff_drums_real",
    "diff_keys",
    "diff_guitarghl",
    "diff_guitar_coop_ghl",
    "diff_rhythm_ghl",
    "diff_bassghl",
    "diff_vocals",
    "preview_start_time",
    "album_track",
    "playlist_track",
    "delay",
    "hopo_frequency",
    "multiplier_note",
    "sustain_cutoff_threshold",
    "chord_snap_threshold",
    "video_start_time",
];

fn default_int(key: &str) -> f64 {
    match key {
        "song_length" | "delay" | "hopo_frequency" | "multiplier_note" | "chord_snap_threshold" | "video_start_time" => 0.0,
        "album_track" | "playlist_track" => 16000.0,
        _ => -1.0,
    }
}

/// JavaScript's parseFloat: the longest numeric prefix, or NaN.
fn js_parse_float(s: &str) -> f64 {
    let s = s.trim_start();
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    if s[i..].starts_with("Infinity") {
        return if b.first() == Some(&b'-') { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    let digits_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i == digits_start || (i == digits_start + 1 && b[digits_start] == b'.') {
        return f64::NAN;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let exp_start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_start {
            i = j;
        }
    }
    s[..i].parse().unwrap_or(f64::NAN)
}

/// scan-chart's `removeStyleTags`: drops Unity rich-text tags it recognizes.
pub fn remove_style_tags(text: &str) -> String {
    const TAGS: &[&str] = &[
        "align", "allcaps", "alpha", "b", "br", "color", "cspace", "font", "font-weight", "gradient", "i", "indent",
        "line-height", "line-indent", "link", "lowercase", "margin", "mark", "mspace", "nobr", "noparse", "page", "pos",
        "rotate", "s", "size", "smallcaps", "space", "sprite", "strikethrough", "style", "sub", "sup", "u", "uppercase",
        "voffset", "width",
    ];
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut s = text.to_string();
    loop {
        let mut out = String::with_capacity(s.len());
        let mut rest = s.as_str();
        while let Some(start) = rest.find('<') {
            out.push_str(&rest[..start]);
            let tail = &rest[start + 1..];
            let Some(end) = tail.find('>') else {
                out.push_str(&rest[start..]);
                rest = "";
                break;
            };
            let inner = tail[..end].trim_start();
            let inner = inner.strip_prefix('/').unwrap_or(inner).trim_start();
            let lower = inner.to_lowercase();
            let is_tag = lower.starts_with('#')
                || TAGS.iter().any(|t| {
                    lower.starts_with(t) && !lower[t.len()..].starts_with(word)
                });
            if is_tag {
                rest = &tail[end + 1..];
            } else {
                out.push('<');
                rest = tail;
            }
        }
        out.push_str(rest);
        let out = out.trim().to_string();
        if out == s {
            return out;
        }
        s = out;
    }
}

struct IniResult {
    /// `None` when there's no readable song.ini / [Song] section.
    meta: Option<Vec<(String, String)>>,
    name: String,
    artist: String,
    charter: String,
    mods: Modifiers,
    /// Integer-ish values that differ from the defaults (to check diff_* fields).
    diff_set: Vec<String>,
    song_length: f64,
}

fn decode_ini(b: &[u8]) -> String {
    crate::library::decode(b)
}

fn scan_ini(files: &[ChartFile], issues: &mut Issues) -> IniResult {
    let mut res = IniResult {
        meta: None,
        name: String::new(),
        artist: String::new(),
        charter: String::new(),
        mods: Modifiers::default(),
        diff_set: Vec::new(),
        song_length: 0.0,
    };
    let inis: Vec<&ChartFile> = files.iter().filter(|f| ext_of(&f.name).eq_ignore_ascii_case("ini")).collect();
    for f in &inis {
        if f.name != "song.ini" {
            issues.folder.push(("invalidIni", format!("\"{}\" is not named \"song.ini\".", f.name)));
        }
    }
    if inis.len() > 1 {
        issues.folder.push(("multipleIniFiles", "This chart has multiple .ini files.".into()));
    }
    let Some(ini) = inis.iter().find(|f| f.name == "song.ini").or(inis.last()) else {
        issues.folder.push(("noMetadata", "This chart doesn't have \"song.ini\".".into()));
        return res;
    };
    let text = decode_ini(ini.data.as_deref().unwrap_or_default());
    // parseIni: [section] lines, key = value lines, everything else is a bad line.
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut current: Option<String> = None;
    let mut bad = Vec::new();
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r').trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            if line.len() > 2 && line.ends_with(']') {
                current = Some(line[1..line.len() - 1].trim().to_string());
            } else {
                bad.push(line.to_string());
            }
        } else if let Some((k, v)) = line.split_once('=') {
            let name = current.clone().unwrap_or_default();
            match sections.iter_mut().find(|(n, _)| *n == name) {
                Some((_, kv)) => {
                    let (k, v) = (k.trim().to_string(), v.trim().to_string());
                    match kv.iter_mut().find(|(ek, _)| *ek == k) {
                        Some(e) => e.1 = v,
                        None => kv.push((k, v)),
                    }
                }
                None => sections.push((name, vec![(k.trim().to_string(), v.trim().to_string())])),
            }
        } else {
            bad.push(line.to_string());
        }
    }
    for line in bad.iter().rev().take(5).rev() {
        let shown: String = line.chars().take(197).collect();
        let shown = if line.chars().count() > 200 { format!("{shown}...") } else { line.clone() };
        issues.folder.push(("badIniLine", format!("Unsupported type of line: \"{shown}\"")));
    }
    let Some((_, song)) = sections.iter().find(|(n, _)| n == "song" || n == "Song" || n == "SONG") else {
        issues.folder.push(("invalidMetadata", "\"song.ini\" doesn't have a \"[Song]\" section.".into()));
        return res;
    };
    let raw = |k: &str| song.iter().find(|(sk, _)| sk == k).map(|(_, v)| v.as_str());
    let string = |k: &str, legacy: Option<&str>| -> Option<String> {
        let ok = |v: &str| !v.is_empty() && v != "0" && v != "-1";
        raw(k).filter(|v| ok(v)).or_else(|| legacy.and_then(raw).filter(|v| ok(v))).map(remove_style_tags)
    };
    let int = |k: &str, legacy: Option<&str>| -> f64 {
        let v = raw(k).map(js_parse_float).unwrap_or(f64::NAN);
        if !v.is_nan() && v != -1.0 {
            return v;
        }
        if let Some(l) = legacy {
            let v = raw(l).map(js_parse_float).unwrap_or(f64::NAN);
            if !v.is_nan() && v != -1.0 {
                return v;
            }
        }
        default_int(k)
    };
    let boolean = |k: &str| -> Option<bool> {
        raw(k).and_then(|v| match v.to_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        })
    };
    for key in ["name", "artist", "album", "genre", "year", "charter"] {
        if string(key, if key == "charter" { Some("frets") } else { None }).is_none() {
            // Defaults are "Unknown X"; a value equal to the default counts as missing too.
            issues.metadata.push(("missingValue", format!("Metadata is missing the \"{key}\" property.")));
        }
    }
    // (Reported in scan-chart's order: integer checks come before the required-property checks.)
    let missing: Vec<_> = issues.metadata.drain(..).collect();
    let legacy = |k: &str| match k {
        "album_track" => Some("track"),
        "hopo_frequency" => Some("hopofreq"),
        "multiplier_note" => Some("star_power_note"),
        _ => None,
    };
    let mut multiplier_note = int("multiplier_note", Some("star_power_note"));
    for key in INTEGER_KEYS {
        let v = int(key, legacy(key));
        if v.fract() != 0.0 || !v.is_finite() {
            issues.metadata.push((
                "invalidValue",
                format!("The \"{key}\" value in \"song.ini\" is \"{}\", which is not an integer.", js_num(v)),
            ));
        }
        if key.starts_with("diff_") && v.round() != -1.0 {
            res.diff_set.push(key.to_string());
        }
    }
    issues.metadata.extend(missing);
    if multiplier_note.round() != 0.0 && multiplier_note.round() != 103.0 && multiplier_note.round() != 116.0 {
        issues.metadata.push((
            "invalidValue",
            format!(
                "The \"multiplier_note\" value in \"song.ini\" is \"{}\", which is not a valid value.",
                js_num(multiplier_note.round())
            ),
        ));
        multiplier_note = 0.0;
    }
    let pro = boolean("pro_drums").unwrap_or(false);
    let five = boolean("five_lane_drums").unwrap_or(false);
    if pro && five {
        issues.metadata.push((
            "invalidValue",
            "Metadata contains both the \"pro_drums\" and \"five_lane_drums\" properties, which is not supported.".into(),
        ));
    }
    res.mods = Modifiers {
        hopo_frequency: int("hopo_frequency", Some("hopofreq")).round() as i64,
        eighthnote_hopo: boolean("eighthnote_hopo").unwrap_or(false),
        multiplier_note: multiplier_note.round() as i64,
        sustain_cutoff_threshold: int("sustain_cutoff_threshold", None).round() as i64,
        chord_snap_threshold: int("chord_snap_threshold", None).round() as i64,
        five_lane_drums: five,
        pro_drums: pro,
    };
    res.name = string("name", None).unwrap_or_default();
    res.artist = string("artist", None).unwrap_or_default();
    res.charter = string("charter", Some("frets")).unwrap_or_default();
    res.song_length = int("song_length", None);
    res.meta = Some(song.clone());
    res
}

/// Number formatting like JavaScript's `${n}`.
fn js_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e21 { format!("{}", v as i64) } else { format!("{v}") }
}

// ---------- scan-chart.ts ----------

pub fn scan_chart(src: &ChartSource) -> Vec<IssueRow> {
    let path_str = src.sng.as_ref().unwrap_or(&src.path).display().to_string();
    let mut issues = Issues { folder: Vec::new(), metadata: Vec::new(), chart: Vec::new() };
    let files = match load_files(src) {
        Ok(f) => f,
        Err(e) => {
            issues.folder.push(("badChart", format!("Couldn't read this chart: {e}")));
            Vec::new()
        }
    };
    let ini = scan_ini(&files, &mut issues);

    // findChartData
    let mut charts: Vec<&ChartFile> =
        files.iter().filter(|f| matches!(ext_of(&f.name).to_lowercase().as_str(), "chart" | "mid")).collect();
    charts.sort_by_key(|f| {
        let named = f.name == "notes.chart" || f.name == "notes.mid";
        (!named, ext_of(&f.name).to_lowercase() != "mid")
    });
    for f in &charts {
        if f.name != "notes.chart" && f.name != "notes.mid" {
            issues.folder.push((
                "invalidChart",
                format!("\"{}\" is not named \"notes.{}\".", f.name, ext_of(&f.name).to_lowercase()),
            ));
        }
    }
    if charts.len() > 1 {
        issues.folder.push(("multipleChart", "This chart has multiple .chart/.mid files.".into()));
    }
    let mut parsed: Option<ParsedChart> = None;
    match charts.first() {
        None => issues.folder.push(("noChart", "This chart doesn't have \"notes.chart\"/\"notes.mid\".".into())),
        Some(f) => {
            let is_mid = ext_of(&f.name).eq_ignore_ascii_case("mid");
            match chart::parse_chart_file(f.data.as_deref().unwrap_or_default(), is_mid, &ini.mods) {
                Ok(p) => parsed = Some(p),
                Err(e) => issues.folder.push(("badChart", e)),
            }
        }
    }

    if let Some(p) = &parsed {
        find_chart_issues(p, &mut issues);
        if ini.meta.is_some() {
            let instruments: Vec<&str> = p.tracks.iter().map(|t| t.instrument).collect();
            const DIFFS: &[(&str, &str)] = &[
                ("guitar", "diff_guitar"),
                ("guitarcoop", "diff_guitar_coop"),
                ("rhythm", "diff_rhythm"),
                ("bass", "diff_bass"),
                ("drums", "diff_drums"),
                ("keys", "diff_keys"),
                ("guitarghl", "diff_guitarghl"),
                ("guitarcoopghl", "diff_guitar_coop_ghl"),
                ("rhythmghl", "diff_rhythm_ghl"),
                ("bassghl", "diff_bassghl"),
            ];
            let set = |k: &str| ini.diff_set.iter().any(|d| d == k);
            for (inst, key) in DIFFS {
                if instruments.contains(inst) && !set(key) {
                    issues.metadata.push(("missingValue", format!("Metadata is missing a \"{key}\" value.")));
                }
            }
            if p.has_vocals && !set("diff_vocals") {
                issues.metadata.push(("missingValue", "Metadata is missing a \"diff_vocals\" value.".into()));
            }
            for (inst, key) in DIFFS {
                if set(key) && !instruments.contains(inst) {
                    issues.metadata.push((
                        "extraValue",
                        format!("Metadata contains \"{key}\", but {inst} is not charted."),
                    ));
                }
            }
            if set("diff_vocals") && !p.has_vocals {
                issues
                    .metadata
                    .push(("extraValue", "Metadata contains \"diff_vocals\", but vocals are not charted.".into()));
            }
        }
    }

    scan_image(&files, &mut issues);
    scan_audio(&files, &mut issues);
    scan_video(&files, &mut issues);

    // ExcelBuilder.getChartIssues
    let mut rows = Vec::new();
    let mut add = |issue: &str, description: String, mandatory: bool| {
        rows.push(IssueRow {
            path: path_str.clone(),
            artist: ini.artist.clone(),
            name: ini.name.clone(),
            charter: ini.charter.clone(),
            issue: issue.to_string(),
            description,
            mandatory,
        })
    };
    for (kind, desc) in issues.folder {
        if kind == "albumArtSize" {
            continue; // .sng conversion fixes this
        }
        let mandatory =
            ["noMetadata", "invalidMetadata", "noAudio", "badAudio", "noChart", "invalidChart", "badChart"].contains(&kind);
        add(kind, desc, mandatory);
    }
    for (kind, desc) in issues.metadata {
        let mandatory = ["\"name\"", "\"artist\"", "\"charter\""].iter().any(|p| desc.contains(p));
        add(kind, desc, mandatory);
    }
    for (inst, diff, kind, desc) in issues.chart {
        let prefix = format!(
            "{}{}",
            inst.map(|i| format!("[{i}]")).unwrap_or_default(),
            diff.map(|d| format!("[{d}]")).unwrap_or_default()
        );
        add(kind, format!("{prefix} {desc}"), kind == "noNotes");
    }
    rows
}

fn scan_image(files: &[ChartFile], issues: &mut Issues) {
    let albums: Vec<&ChartFile> =
        files.iter().filter(|f| matches!(f.name.as_str(), "album.jpg" | "album.jpeg" | "album.png")).collect();
    if albums.len() > 1 {
        issues.folder.push(("multipleAlbumArt", "This chart has multiple album art files.".into()));
    }
    let Some(album) = albums.last() else {
        issues.folder.push(("noAlbumArt", "This chart doesn't have album art.".into()));
        return;
    };
    let dims = album.data.as_deref().and_then(|d| {
        image::ImageReader::new(std::io::Cursor::new(d)).with_guessed_format().ok()?.into_dimensions().ok()
    });
    match dims {
        Some((w, h)) => {
            if !((w, h) == (500, 500) || (w, h) == (512, 512)) {
                issues.folder.push((
                    "albumArtSize",
                    format!("This chart's album art is {h}x{w}, and should be 512x512."),
                ));
            }
        }
        None => issues.folder.push(("badAlbumArt", "This chart's album art couldn't be parsed.".into())),
    }
}

fn scan_audio(files: &[ChartFile], issues: &mut Issues) {
    const STEMS: &[&str] = &[
        "song", "guitar", "bass", "rhythm", "keys", "vocals", "vocals_1", "vocals_2", "drums", "drums_1", "drums_2",
        "drums_3", "drums_4", "crowd", "preview",
    ];
    let mut stems = Vec::new();
    let mut audio = 0;
    for f in files {
        let ext = ext_of(&f.name);
        if !AUDIO.contains(&ext.to_lowercase().as_str()) {
            continue;
        }
        let base = basename(&f.name);
        if STEMS.contains(&base) && AUDIO.contains(&ext.as_str()) {
            stems.push(base.to_string());
            if !["preview", "crowd"].contains(&base.to_lowercase().as_str()) {
                audio += 1;
            }
        } else {
            issues.folder.push(("invalidAudio", format!("\"{}\" is not a valid audio stem name.", f.name)));
        }
    }
    let mut uniq = stems.clone();
    uniq.sort();
    uniq.dedup();
    if uniq.len() != stems.len() {
        issues.folder.push(("multipleAudio", "This chart has multiple audio files of the same stem.".into()));
    }
    if audio == 0 {
        issues.folder.push(("noAudio", "This chart doesn't have an audio file.".into()));
    }
}

fn scan_video(files: &[ChartFile], issues: &mut Issues) {
    let mut count = 0;
    for f in files {
        let ext = ext_of(&f.name);
        if basename(&f.name) == "video" && ["mp4", "avi", "webm", "vp8", "ogv", "mpeg"].contains(&ext.as_str()) {
            count += 1;
            if ["mp4", "avi", "mpeg"].contains(&ext.as_str()) {
                issues.folder.push((
                    "badVideo",
                    format!("\"{}\" will not work on Linux and should be converted to .webm.", f.name),
                ));
            }
        }
    }
    if count > 1 {
        issues.folder.push(("multipleVideo", "This chart has multiple video background files.".into()));
    }
}

// ---------- scan-parsed-chart.ts: findChartIssues ----------

const LEADING_SILENCE_THRESHOLD_MS: f64 = 1000.0;
const MIN_SUSTAIN_GAP_MS: f64 = 40.0;
const MIN_SUSTAIN_MS: f64 = 100.0;

fn describe(kind: &str) -> &'static str {
    match kind {
        "misalignedTimeSignature" => {
            "This chart has a time signature marker that doesn't appear at the start of a measure. This can't be interpreted correctly in Clone Hero."
        }
        "noNotes" => "This chart has no notes.",
        "noExpert" => "This instrument has Easy, Medium, or Hard charted but not Expert.",
        "difficultyNotReduced" => "The notes of this difficulty are identical to the notes of the expert chart.",
        "isDefaultBPM" => {
            "This chart has only one 120 BPM marker and only one 4/4 time signature. This usually means the chart wasn't tempo-mapped, but you can ignore this if the song is a constant 120 BPM."
        }
        "noSections" => "This chart has no sections.",
        "badEndEvent" => "This end event is in an invalid location and will be ignored by most games.",
        "smallLeadingSilence" => "This track has a note that is less than 2000ms after the start of the track.",
        "noStarPower" => "This track has no star power.",
        "emptyStarPower" => "This star power phrase contains no notes.",
        "badStarPower" => "This star power is being ignored due to the .ini \"multiplier_note\" setting.",
        "emptySoloSection" => "This solo section contains no notes.",
        "noDrumActivationLanes" => "This drums track has no activation lanes.",
        "emptyFlexLane" => "This flex lane contains no notes.",
        "difficultyForbiddenNote" => "This is a note that isn't allowed on the track's difficulty.",
        "invalidChord" => "The use of this type of chord is strongly discouraged.",
        "brokenNote" => "This note is so close to the previous note that this was likely a charting mistake.",
        "badSustainGap" => "This note is not far enough ahead of the previous sustain.",
        "babySustain" => "The sustain on this note is too short.",
        "invalidLyric" => "This lyric is on the EVENTS track instead of PART VOCALS and will not be displayed.",
        "invalidPhraseStart" => {
            "This phrase_start is on the EVENTS track. Vocal phrase boundaries in .mid charts must be MIDI note 105 (player 1) or 106 (player 2) on PART VOCALS."
        }
        "invalidPhraseEnd" => {
            "This phrase_end is on the EVENTS track. Vocal phrase boundaries in .mid charts must be MIDI note 105 (player 1) or 106 (player 2) on PART VOCALS."
        }
        _ => "",
    }
}

/// scan-chart's msToExactTime: [HH:]MM:SS.ss
pub fn ms_to_exact_time(ms: f64) -> String {
    // lodash's _.floor(x, 2) shifts the decimal point through the string form.
    let x = (ms / 1000.0) % 60.0;
    let shifted = format!("{x}e2").parse::<f64>().unwrap_or(x * 100.0).floor();
    let seconds = format!("{shifted}e-2").parse::<f64>().unwrap_or(shifted / 100.0);
    let minutes = ((ms / 1000.0 / 60.0) % 60.0).floor();
    let hours = ((ms / 1000.0 / 3600.0) % 24.0).floor();
    let h = if hours > 0.0 { format!("{hours}:") } else { String::new() };
    format!("{h}{:02}:{:05.2}", minutes as i64, seconds)
}

fn type_count(group: &[chart::Note], types: &[u8]) -> usize {
    group.iter().filter(|n| types.contains(&n.ty)).count()
}

/// Phrases containing at least one note (track-hasher's pruneEmptyPhrases).
fn empty_phrases<'a>(phrases: impl Iterator<Item = (i64, i64, f64)> + 'a, notes: &'a [Vec<chart::Note>]) -> Vec<f64> {
    let mut out = Vec::new();
    let mut i = 0;
    for (tick, length, ms) in phrases {
        while i < notes.len() && notes[i][0].tick < tick {
            i += 1;
        }
        let nonempty = i < notes.len() && notes[i][0].tick < tick + length.max(1);
        if !nonempty {
            out.push(ms);
        }
    }
    out
}

fn same_notes(a: &Track, b: &Track) -> bool {
    a.note_groups.len() == b.note_groups.len()
        && a.note_groups.iter().zip(&b.note_groups).all(|(x, y)| {
            x.len() == y.len()
                && x.iter().zip(y).all(|(n, m)| n.tick == m.tick && n.ty == m.ty && n.flags == m.flags && n.length == m.length)
        })
        && a.star_power.len() == b.star_power.len()
}

fn find_chart_issues(p: &ParsedChart, issues: &mut Issues) {
    let mut global = |kind: &'static str, ms: Option<f64>| {
        let d = describe(kind);
        let desc = match ms {
            Some(ms) => format!("[{}]: {d}", ms_to_exact_time(ms)),
            None => d.to_string(),
        };
        issues.chart.push((None, None, kind, desc));
    };
    for kind in &p.parse_issues {
        global(kind, None);
    }
    // misalignedTimeSignature
    {
        let mut ts: Vec<(i64, f64, u32, u32)> = p.timed_time_sigs.clone();
        let mut last_beatline = 0f64;
        let mut i = 0usize;
        while i < ts.len() {
            if last_beatline != ts[i].0 as f64 {
                global("misalignedTimeSignature", Some(ts[i].1));
                ts.remove(i);
                if i == 0 {
                    continue;
                }
                i -= 1;
            }
            while i + 1 < ts.len() && last_beatline < ts[i + 1].0 as f64 {
                last_beatline += p.resolution as f64 * 4.0 * (ts[i].2 as f64 / ts[i].3 as f64);
            }
            i += 1;
        }
    }
    // noNotes
    if p.tracks.iter().all(|t| t.note_groups.is_empty()) && !p.has_vocals {
        global("noNotes", None);
    }
    // isDefaultBPM
    if p.tempos.len() == 1
        && (p.tempos[0].1 * 1e12).round() / 1e12 == 120.0
        && p.time_sigs.len() == 1
        && p.time_sigs[0].1 == 4
        && p.time_sigs[0].2 == 4
    {
        global("isDefaultBPM", None);
    }
    // noSections
    if p.sections.is_empty() {
        global("noSections", None);
    }
    // badEndEvent
    for e in p.end_events.iter().skip(1) {
        global("badEndEvent", Some(e.ms));
    }
    let last_note_tick = p.tracks.iter().filter_map(|t| t.note_groups.last().map(|g| g[0].tick)).max();
    if let (Some(last), Some(first_end)) = (last_note_tick, p.end_events.first())
        && last != 0
        && first_end.tick < last
    {
        global("badEndEvent", Some(first_end.ms));
    }

    // noExpert, difficultyNotReduced (grouped by instrument, in track order)
    let mut seen: Vec<&str> = Vec::new();
    for t in &p.tracks {
        if seen.contains(&t.instrument) {
            continue;
        }
        seen.push(t.instrument);
        let group: Vec<&Track> = p.tracks.iter().filter(|x| x.instrument == t.instrument).collect();
        match group.iter().find(|x| x.difficulty == "expert") {
            None => issues.chart.push((Some(t.instrument.into()), None, "noExpert", describe("noExpert").into())),
            Some(expert) => {
                for lower in group.iter().filter(|x| x.difficulty != "expert") {
                    if same_notes(expert, lower) && lower.note_groups.len() > 20 {
                        issues.chart.push((
                            Some(lower.instrument.into()),
                            Some(lower.difficulty.into()),
                            "difficultyNotReduced",
                            describe("difficultyNotReduced").into(),
                        ));
                    }
                }
            }
        }
    }

    for track in &p.tracks {
        track_issues(track, issues);
    }
}

fn track_issues(track: &Track, issues: &mut Issues) {
    let mut add = |kind: &'static str, ms: Option<f64>| {
        let d = describe(kind);
        let desc = match ms {
            Some(ms) => format!("[{}]: {d}", ms_to_exact_time(ms)),
            None => d.to_string(),
        };
        issues.chart.push((Some(track.instrument.into()), Some(track.difficulty.into()), kind, desc));
    };
    let groups = &track.note_groups;
    let span_ms = || match (groups.first(), groups.last()) {
        (Some(a), Some(b)) => b[0].ms - a[0].ms,
        _ => 0.0,
    };
    if groups.first().is_some_and(|g| g[0].ms < LEADING_SILENCE_THRESHOLD_MS) {
        add("smallLeadingSilence", None);
    }
    if track.star_power.is_empty() && track.instrument != "drums" && groups.len() > 50 && span_ms() > 60000.0 {
        add("noStarPower", None);
    }
    for ms in empty_phrases(track.star_power.iter().map(|s| (s.tick, s.length, s.ms)), groups) {
        add("emptyStarPower", Some(ms));
    }
    for s in &track.rejected_star_power {
        add("badStarPower", Some(s.ms));
    }
    for ms in empty_phrases(track.solo.iter().map(|s| (s.tick, s.length, s.ms)), groups) {
        add("emptySoloSection", Some(ms));
    }
    if track.instrument == "drums"
        && track.freestyle.is_empty()
        && !track.star_power.is_empty()
        && groups.len() > 50
        && span_ms() > 60000.0
    {
        add("noDrumActivationLanes", None);
    }
    for ms in empty_phrases(track.flex.iter().map(|(s, _)| (s.tick, s.length, s.ms)), groups) {
        add("emptyFlexLane", Some(ms));
    }

    match instrument_type(track.instrument) {
        InstrumentType::Drums => {
            let non_kick = [nt::GREEN_DRUM, nt::RED_DRUM, nt::YELLOW_DRUM, nt::BLUE_DRUM];
            for g in groups {
                if track.difficulty != "expert" {
                    for n in g {
                        if n.flags & nf::DOUBLE_KICK != 0 {
                            add("difficultyForbiddenNote", Some(g[0].ms));
                        }
                    }
                }
                if track.difficulty == "easy" && type_count(g, &non_kick) == 2 && type_count(g, &[nt::KICK]) > 0 {
                    add("difficultyForbiddenNote", Some(g[0].ms));
                }
                if type_count(g, &non_kick) >= 3 {
                    add("invalidChord", Some(g[0].ms));
                }
            }
        }
        InstrumentType::FiveFret => {
            let five = [nt::GREEN, nt::RED, nt::YELLOW, nt::BLUE, nt::ORANGE];
            for g in groups {
                let forbidden = match track.difficulty {
                    "hard" => type_count(g, &[nt::GREEN, nt::ORANGE]) == 2,
                    "medium" => type_count(g, &[nt::ORANGE]) > 0 || type_count(g, &[nt::GREEN, nt::BLUE]) == 2,
                    "easy" => type_count(g, &[nt::ORANGE, nt::BLUE]) > 0,
                    _ => false,
                };
                if forbidden {
                    add("difficultyForbiddenNote", Some(g[0].ms));
                }
                if type_count(g, &five) == 5 {
                    add("invalidChord", Some(g[0].ms));
                }
            }
        }
        InstrumentType::SixFret => {
            let black = [nt::BLACK1, nt::BLACK2, nt::BLACK3];
            let white = [nt::WHITE1, nt::WHITE2, nt::WHITE3];
            let one = [nt::BLACK1, nt::WHITE1];
            let two = [nt::BLACK2, nt::WHITE2];
            let three = [nt::BLACK3, nt::WHITE3];
            for g in groups {
                let forbidden = match track.difficulty {
                    "hard" => g.len() == 3 && (type_count(g, &black) < 3 || type_count(g, &white) < 3),
                    "medium" => {
                        g.len() == 3
                            || (g.len() == 2
                                && type_count(g, &black) > 0
                                && type_count(g, &white) > 0
                                && type_count(g, &one) < 2
                                && type_count(g, &two) < 2
                                && type_count(g, &three) < 2)
                    }
                    "easy" => {
                        g.len() == 3
                            || (g.len() == 2
                                && !(type_count(g, &[nt::WHITE1, nt::WHITE2]) == 2
                                    || type_count(g, &[nt::WHITE2, nt::WHITE3]) == 2))
                    }
                    _ => false,
                };
                if forbidden {
                    add("difficultyForbiddenNote", Some(g[0].ms));
                }
                if g.len() > 3
                    || (type_count(g, &two) == 2 && type_count(g, &one) > 0)
                    || (type_count(g, &three) == 2 && type_count(g, &one) + type_count(g, &two) > 0)
                {
                    add("invalidChord", Some(g[0].ms));
                }
            }
        }
    }

    // brokenNote
    for i in 1..groups.len() {
        let (note, prev) = (&groups[i], &groups[i - 1]);
        let d = note[0].ms - prev[0].ms;
        if d > 0.0 && d <= 15.0 {
            let open_now = type_count(note, &[nt::OPEN]) > 0;
            let open_prev = type_count(prev, &[nt::OPEN]) > 0;
            if open_now != open_prev {
                continue;
            }
            add("brokenNote", Some(note[0].ms));
        }
    }

    // badSustainGap, babySustain
    if instrument_type(track.instrument) != InstrumentType::Drums {
        let mut gaps: Vec<(f64, f64)> = Vec::new();
        for (i, g) in groups.iter().enumerate() {
            let t = g[0].ms;
            gaps.retain(|r| r.1 > t);
            if gaps.iter().any(|r| t >= r.0 && t < r.1) {
                add("badSustainGap", Some(t));
            }
            for n in g {
                if n.ms_length > 0.0 && n.ty != nt::OPEN {
                    gaps.push((n.ms + n.ms_length, n.ms + n.ms_length + MIN_SUSTAIN_GAP_MS));
                }
                if n.ms_length > 0.0 && n.ms_length < MIN_SUSTAIN_MS {
                    let next_open = groups.get(i + 1).and_then(|ng| ng.iter().find(|m| m.ty == nt::OPEN));
                    if let Some(o) = next_open
                        && o.tick < n.tick + n.length
                        && o.flags & (nf::HOPO | nf::TAP) != 0
                    {
                        continue;
                    }
                    add("babySustain", Some(n.ms));
                }
            }
        }
    }
}

// ---------- Excel report (ExcelBuilder.ts) ----------

pub fn write_xlsx(rows: &[IssueRow], out_dir: &Path) -> Result<PathBuf, String> {
    use rust_xlsxwriter::{ConditionalFormatFormula, Format, FormatBorder, Workbook};
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    ws.set_name("Chart Issues").map_err(|e| e.to_string())?;
    let headers: [(&str, f64); 7] = [
        ("Artist", 160.0),
        ("Name", 400.0),
        ("Charter", 160.0),
        ("Issue Name", 160.0),
        (
            "Issue Description (a more detailed description of issue types can be found at https://drive.google.com/open?id=1UK7GsP4ZHJkOg8uREFRMY72svySaDlf0QRTGlk-ruYQ)",
            650.0,
        ),
        ("Fix Mandatory?", 120.0),
        ("Path", 600.0),
    ];
    let grid = Format::new().set_border(FormatBorder::Thin).set_border_color(0xD0D0D0);
    let head = Format::new()
        .set_bold()
        .set_background_color(0xD3D3D3)
        .set_border(FormatBorder::Thin)
        .set_border_color(0xD0D0D0);
    for (i, (h, w)) in headers.iter().enumerate() {
        ws.write_with_format(0, i as u16, *h, &head).map_err(|e| e.to_string())?;
        ws.set_column_width(i as u16, w / 7.0).map_err(|e| e.to_string())?;
    }
    for (r, row) in rows.iter().enumerate() {
        let r = r as u32 + 1;
        let cells = [
            row.artist.as_str(),
            row.name.as_str(),
            row.charter.as_str(),
            row.issue.as_str(),
            row.description.as_str(),
            if row.mandatory { "yes" } else { "no" },
            row.path.as_str(),
        ];
        for (c, v) in cells.iter().enumerate() {
            ws.write_with_format(r, c as u16, *v, &grid).map_err(|e| e.to_string())?;
        }
    }
    ws.set_freeze_panes(1, 0).map_err(|e| e.to_string())?;
    let last = rows.len() as u32;
    ws.autofilter(0, 0, last, 6).map_err(|e| e.to_string())?;
    if last > 0 {
        let stripe = ConditionalFormatFormula::new()
            .set_rule("=MOD(ROW(),2)=0")
            .set_format(Format::new().set_background_color(0xF7F7F7));
        ws.add_conditional_format(1, 0, last, 6, &stripe).map_err(|e| e.to_string())?;
    }
    let name = format!("chart_issues_{}.xlsx", chrono::Local::now().format("%Y.%m.%d_%H.%M.%S"));
    let path = out_dir.join(name);
    wb.save(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_match_javascript() {
        assert_eq!(ms_to_exact_time(134_350.0), "02:14.34"); // same float quirk as JavaScript
        assert_eq!(ms_to_exact_time(2_000.0), "00:02.00");
        assert_eq!(ms_to_exact_time(3_725_000.0), "1:02:05.00");
        assert!(js_parse_float("abc").is_nan());
        assert_eq!(js_parse_float(" -2.1x"), -2.1);
        assert_eq!(js_parse_float("12"), 12.0);
        assert_eq!(remove_style_tags("<color=#AEFFFF>Aren Eternal</color> & Geo"), "Aren Eternal & Geo");
        assert_eq!(remove_style_tags("a <3 b"), "a <3 b");
    }

    /// Compares chart and metadata issues with the ones Encore reports (it runs scan-chart on
    /// the same files). Network test: TRUSS_TEST_ENCORE=1.
    #[test]
    fn issues_match_encore() {
        if std::env::var("TRUSS_TEST_ENCORE").is_err() {
            return;
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut charts = Vec::new();
            for (query, instrument) in [
                ("through the fire and flames", None),
                ("metallica", Some("drums")),
                ("slipknot", Some("guitarghl")),
                ("queen", Some("bass")),
                ("*", Some("keys")),
                ("rush", None),
            ] {
                let common = crate::api::Common::new(1, instrument.map(String::from), None, None);
                let r = crate::api::search(query.to_string(), common).await.unwrap();
                charts.extend(r.data.into_iter().take(8));
            }
            let dir = std::env::temp_dir().join("truss-issue-test");
            let _ = std::fs::create_dir_all(&dir);
            let (mut same, mut diff) = (0, 0);
            for c in charts {
                let novideo = if c.has_video_background { "_novideo" } else { "" };
                let url = format!("{}/{}{novideo}.sng", crate::api::FILES_URL, c.md5);
                let Ok(resp) = crate::api::CLIENT.get(&url).send().await else { continue };
                let Ok(bytes) = resp.bytes().await else { continue };
                let path = dir.join(format!("{}.sng", c.md5));
                std::fs::write(&path, &bytes).unwrap();
                let rows = scan_chart(&ChartSource { path: dir.clone(), sng: Some(path.clone()) });
                let _ = std::fs::remove_file(&path);
                let count = |v: Vec<String>| {
                    let mut m: std::collections::BTreeMap<String, usize> = Default::default();
                    for k in v {
                        *m.entry(k).or_default() += 1;
                    }
                    m
                };
                let chart_kinds = [
                    "misalignedTimeSignature", "noNotes", "noExpert", "difficultyNotReduced", "isDefaultBPM", "noSections",
                    "badEndEvent", "smallLeadingSilence", "noStarPower", "emptyStarPower", "badStarPower",
                    "emptySoloSection", "noDrumActivationLanes", "emptyFlexLane", "difficultyForbiddenNote",
                    "invalidChord", "brokenNote", "badSustainGap", "babySustain", "invalidLyric",
                ];
                let ours = count(
                    rows.iter()
                        .filter(|r| chart_kinds.contains(&r.issue.as_str()) || r.issue == "missingValue" || r.issue == "extraValue" || r.issue == "invalidValue")
                        .map(|r| format!("{} {}", r.issue, r.description))
                        .collect(),
                );
                let mut theirs_v: Vec<String> = c
                    .metadata_issues
                    .iter()
                    .map(|i| format!("{} {}", i.metadata_issue, i.description))
                    .collect();
                if let Some(nd) = &c.notes_data {
                    for i in &nd.chart_issues {
                        let prefix = format!(
                            "{}{}",
                            i.instrument.as_ref().map(|x| format!("[{x}]")).unwrap_or_default(),
                            i.difficulty.as_ref().map(|x| format!("[{x}]")).unwrap_or_default()
                        );
                        theirs_v.push(format!("{} {prefix} {}", i.note_issue, i.description));
                    }
                }
                let theirs = count(theirs_v);
                if std::env::var("TRUSS_TEST_VERBOSE").is_ok() {
                    for r in rows.iter().filter(|r| !chart_kinds.contains(&r.issue.as_str())) {
                        println!("   ROW {} {}", r.issue, r.description);
                    }
                }
                if ours == theirs {
                    same += 1;
                } else {
                    diff += 1;
                    println!("DIFF {} {:?} ({})", c.md5, c.name.as_deref().unwrap_or(""), c.charter.as_deref().unwrap_or(""));
                    for (k, n) in &ours {
                        if theirs.get(k) != Some(n) {
                            println!("   ours  {n}x {k} (encore {})", theirs.get(k).copied().unwrap_or(0));
                        }
                    }
                    for (k, n) in &theirs {
                        if !ours.contains_key(k) {
                            println!("   encore {n}x {k}");
                        }
                    }
                }
            }
            println!("issues: {same} charts identical, {diff} differ");
        });
    }

    /// Scans a library folder when TRUSS_TEST_ISSUES is set; writes the report to the temp dir.
    #[test]
    fn scan_real_folder() {
        let Some(dir) = std::env::var_os("TRUSS_TEST_ISSUES") else { return };
        let t = std::time::Instant::now();
        let charts = find_charts(Path::new(&dir));
        let rows: Vec<IssueRow> = charts.iter().flat_map(scan_chart).collect();
        let out = write_xlsx(&rows, &std::env::temp_dir()).unwrap();
        println!("{} charts, {} issues in {:?} -> {}", charts.len(), rows.len(), t.elapsed(), out.display());
        let mut kinds: Vec<(&str, usize)> = Vec::new();
        for r in &rows {
            match kinds.iter_mut().find(|(k, _)| *k == r.issue) {
                Some(e) => e.1 += 1,
                None => kinds.push((&r.issue, 1)),
            }
        }
        kinds.sort_by_key(|k| std::cmp::Reverse(k.1));
        println!("{kinds:?}");
    }
}
