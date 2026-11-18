//! Chart preview: loading pipeline and the highway renderer.
//!
//! Rendering reproduces the original three.js scene (camera, highway, strikeline, sprite
//! notes, sustain planes, clipping planes) with the projection done on the CPU and the
//! result drawn through an iced canvas, so it works with both GPU and CPU renderers.

use crate::api::{CLIENT, Chart, FILES_URL};
use crate::audio::Player;
use crate::chart::{self, InstrumentType, Modifiers, ParsedChart, instrument_type, nf, nt};
use crate::textures::{BARRE1, BARRE2, BARRE3, SP_FLAG, TextureSet};
use futures::{SinkExt, Stream, StreamExt, channel::mpsc::Sender};
use iced::widget::canvas::{self, Frame, Geometry};
use iced::{Point, Rectangle, Renderer, Size, Theme, mouse};
use std::sync::Arc;

pub const HIGHWAY_DURATION_MS: f64 = 1500.0;
const SCALE: f32 = 0.105;
/// How long a hit flame shows: a fret's, and the open note's (or kick's) along the strike line.
const NOTE_FLAME_MS: f64 = 170.0;
const WIDE_FLAME_MS: f64 = 220.0;
/// The game's hit effects (from its Guitar Player prefab): each frame of the hit flame, the
/// star power spark burst and the held-sustain sparks; how long the hit particles last; and
/// the width of its frets, which its effects' sizes are measured against.
const FLAME_FRAME_MS: f64 = 8.0;
const SPARK_FRAME_MS: f64 = 32.0;
const HOLD_FRAME_MS: u64 = 30;
const PARTICLE_MS: f64 = 300.0;
const GAME_FRET: f32 = 0.197;
/// How far back a hit can still be showing a flame.
const FLAME_MS: f64 = 600.0;
const NOTE_SPAN_WIDTH: f32 = 0.95;

// ---------- Loading ----------

pub struct Loaded {
    pub parsed: ParsedChart,
    /// The audio streams from the temp .sng file; the player's readers keep it alive and it is
    /// deleted once the player (and its audio thread) is gone.
    pub player: Player,
    /// Chart time to start at (the chart's preview point).
    pub start_ms: f64,
}

impl std::fmt::Debug for Loaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Loaded")
    }
}

#[derive(Debug, Clone)]
pub enum LoadEvent {
    Progress(f32),
    Done(Result<Arc<Loaded>, String>),
}

const AUDIO_EXTS: [&str; 4] = ["ogg", "mp3", "wav", "opus"];

fn ext(name: &str) -> String {
    name.rsplit('.').next().unwrap_or("").to_ascii_lowercase()
}

fn temp_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("truss-preview")
}

/// Removes preview files left behind by a crash or forced shutdown. Files still in use by
/// another running instance stay readable (they're opened with delete sharing).
pub fn cleanup_temp_files() {
    if let Ok(entries) = std::fs::read_dir(temp_dir()) {
        for e in entries.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }
    // RhythmVerse charts downloaded for preview in an earlier session.
    let _ = std::fs::remove_dir_all(temp_dir().join("rv"));
    // Folder used before the app was renamed from Bridge RS.
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("bridge-rs-preview"));
}

/// Downloads the chart (never the video), parses notes, and opens the audio, all off the UI thread.
/// `keep_video` only matters for RhythmVerse charts (see `rv_cache`). With `local` (the copy in
/// the library) nothing is downloaded, unless that copy can't be read.
pub fn load(chart: Arc<Chart>, keep_video: bool, local: Option<std::path::PathBuf>) -> impl Stream<Item = LoadEvent> {
    iced::stream::channel(16, async move |mut tx: Sender<LoadEvent>| {
        if let Some(path) = local {
            let fallback_preview = chart.preview_start_time.unwrap_or(-1) as f64;
            let r = tokio::task::spawn_blocking(move || load_local(&path, fallback_preview)).await;
            if let Ok(Ok(loaded)) = r {
                let _ = tx.send(LoadEvent::Done(Ok(Arc::new(loaded)))).await;
                return;
            }
        }
        #[cfg(feature = "rhythmverse")]
        if chart.is_rv() {
            let r = rv_cache::load(&chart, keep_video, &mut tx).await;
            let _ = tx.send(LoadEvent::Done(r.map(Arc::new))).await;
            return;
        }
        let _ = keep_video;
        let r = load_inner(&chart, &mut tx).await;
        let _ = tx.send(LoadEvent::Done(r.map(Arc::new))).await;
    })
}

/// Song settings that change how the notes are read.
fn modifiers(num: &dyn Fn(&str) -> Option<f64>, flag: &dyn Fn(&str) -> bool) -> Modifiers {
    Modifiers {
        hopo_frequency: num("hopo_frequency").unwrap_or(0.0) as i64,
        eighthnote_hopo: flag("eighthnote_hopo"),
        multiplier_note: num("multiplier_note").unwrap_or(0.0) as i64,
        sustain_cutoff_threshold: num("sustain_cutoff_threshold").unwrap_or(-1.0) as i64,
        chord_snap_threshold: num("chord_snap_threshold").unwrap_or(0.0) as i64,
        five_lane_drums: flag("five_lane_drums"),
        pro_drums: flag("pro_drums"),
    }
}

fn is_audio(n: &str) -> bool {
    let l = n.to_ascii_lowercase();
    let stem = l.rsplit_once('.').map(|(s, _)| s).unwrap_or(&l);
    AUDIO_EXTS.contains(&ext(&l).as_str()) && !stem.contains("preview") && !stem.contains("crowd")
}

/// The chart time at which the audio starts playing (negative: the chart begins partway into
/// the audio). A .chart "Offset" (seconds) is where in the audio the chart begins, so the audio
/// starts that much earlier; song.ini "delay" (ms) moves the notes later; a chart with an Offset
/// ignores the delay. Measured against the audio of charts that use each. Encore's own preview
/// adds both with the opposite signs, which puts such charts out of sync by twice the amount.
fn audio_start_ms(chart_offset_secs: f64, delay_ms: f64) -> f64 {
    if chart_offset_secs != 0.0 { -chart_offset_secs * 1000.0 } else { -delay_ms }
}

/// Parses the notes and opens the audio. `meta` reads song.ini values.
fn build(
    meta: &dyn Fn(&str) -> Option<String>,
    chart_bytes: &[u8],
    is_mid: bool,
    stems: Vec<(crate::audio::StemData, String)>,
    fallback_preview: f64,
) -> Result<Loaded, String> {
    let num = |k: &str| meta(k).and_then(|v| v.trim().parse::<f64>().ok());
    let flag = |k: &str| meta(k).is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1"));
    let mods = modifiers(&num, &flag);
    let parsed = chart::parse_chart_file(chart_bytes, is_mid, &mods)?;
    if parsed.tracks.iter().all(|t| t.note_groups.is_empty()) {
        return Err("This chart has no playable tracks".to_string());
    }
    let last_note =
        parsed.tracks.iter().flat_map(|t| t.note_groups.iter().flatten()).map(|n| n.ms + n.ms_length).fold(0.0, f64::max);
    let song_length = num("song_length").filter(|l| *l > 0.0).unwrap_or(last_note + 5000.0);
    let start_delay = audio_start_ms(parsed.chart_offset_secs, num("delay").unwrap_or(0.0));
    let preview_start = num("preview_start_time").filter(|p| *p > 0.0).unwrap_or(fallback_preview).max(0.0);
    if stems.is_empty() {
        return Err("This chart has no audio".to_string());
    }
    let player = Player::new(stems, start_delay, song_length)?;
    let start_ms = preview_start.min(player.end_ms() - 1000.0).max(0.0);
    player.seek(start_ms);
    Ok(Loaded { parsed, player, start_ms })
}

/// RhythmVerse charts: downloaded whole into a temp folder (RhythmVerse can't send part of a
/// file), then previewed from there. Downloading the chart afterwards moves that copy into the
/// library instead of fetching it again.
#[cfg(feature = "rhythmverse")]
pub mod rv_cache {
    use super::*;
    use std::path::PathBuf;

    /// How many previewed charts to keep for a later Download.
    const KEEP: usize = 6;

    fn root() -> PathBuf {
        temp_dir().join("rv")
    }

    fn tag(chart: &Chart) -> String {
        chart.md5.chars().filter(|c| c.is_ascii_alphanumeric()).take(24).collect()
    }

    /// The folder for a chart, with or without its video (a download with videos on can't use
    /// a copy made without).
    fn dir(chart: &Chart, keep_video: bool) -> PathBuf {
        root().join(format!("{}-{}", tag(chart), if keep_video { "v" } else { "nv" }))
    }

    /// A finished copy of the chart from an earlier preview, if there is one.
    pub fn cached(chart: &Chart, keep_video: bool) -> Option<PathBuf> {
        let d = dir(chart, keep_video);
        d.is_dir().then_some(d)
    }

    /// Any finished copy from an earlier preview (with or without video).
    pub fn any_cached(chart: &Chart) -> Option<PathBuf> {
        cached(chart, false).or_else(|| cached(chart, true))
    }

    /// Deletes all but the newest few cached charts.
    fn trim() {
        let Ok(rd) = std::fs::read_dir(root()) else { return };
        let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = rd
            .flatten()
            .filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'))
            .map(|e| (e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), e.path()))
            .collect();
        dirs.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, d) in dirs.into_iter().skip(KEEP) {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    async fn fetch(chart: &Chart, keep_video: bool, tx: &mut Sender<LoadEvent>) -> Result<PathBuf, String> {
        let final_dir = dir(chart, keep_video);
        if final_dir.is_dir() {
            return Ok(final_dir);
        }
        let rv = chart.rv.as_ref().ok_or("Not a RhythmVerse chart")?;
        let root = root();
        std::fs::create_dir_all(&root).map_err(|e| format!("Can't create temp folder: {e}"))?;
        let tag = tag(chart);
        let staged = root.join(format!(".{tag}-{}-staged", std::process::id()));
        let mut guard = crate::download::TempGuard(Some(staged.clone()));
        let _ = std::fs::remove_dir_all(&staged);
        // The downloader reports progress on its own channel; pass it on as preview progress.
        let (mut ptx, mut prx) = futures::channel::mpsc::channel(16);
        let work = async {
            let r = crate::rhythmverse::download(rv, &staged, &root, &tag, keep_video, &mut ptx).await;
            ptx.close_channel();
            r
        };
        let forward = async {
            while let Some(p) = prx.next().await {
                if let crate::download::Progress::Bytes { done, total: Some(t) } = p {
                    let _ = tx.try_send(LoadEvent::Progress(done as f32 / t.max(1) as f32));
                }
            }
        };
        let (r, ()) = futures::join!(work, forward);
        r?;
        crate::download::move_path(&staged, &final_dir)?;
        guard.0 = None;
        trim();
        Ok(final_dir)
    }

    pub async fn load(chart: &Chart, keep_video: bool, tx: &mut Sender<LoadEvent>) -> Result<Loaded, String> {
        let folder = fetch(chart, keep_video, tx).await?;
        let _ = tx.send(LoadEvent::Progress(1.0)).await;
        let fallback_preview = chart.preview_start_time.unwrap_or(-1) as f64;
        tokio::task::spawn_blocking(move || load_folder(&folder, fallback_preview))
            .await
            .map_err(|e| e.to_string())?
    }

    pub use super::load_folder;
}

/// Loads a chart folder: song.ini, the notes file and the audio stems (read into memory, so
/// the folder can be moved into the library while the preview plays).
pub fn load_folder(folder: &std::path::Path, fallback_preview: f64) -> Result<Loaded, String> {
    let names: Vec<String> = std::fs::read_dir(folder)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let pairs = names
        .iter()
        .find(|n| n.eq_ignore_ascii_case("song.ini"))
        .and_then(|n| std::fs::read(folder.join(n)).ok())
        .map(|b| crate::library::parse_ini(&crate::library::decode(&b)))
        .unwrap_or_default();
    let meta = |k: &str| pairs.iter().find(|(pk, _)| pk.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
    let pick = |want: &str| names.iter().find(|n| n.to_ascii_lowercase().ends_with(want)).cloned();
    let chart_name = pick(".mid").or_else(|| pick(".chart")).ok_or("This chart has no .chart or .mid file")?;
    let chart_bytes = std::fs::read(folder.join(&chart_name)).map_err(|e| e.to_string())?;
    let mut stems = Vec::new();
    for n in names.iter().filter(|n| is_audio(n)) {
        let data = std::fs::read(folder.join(n)).map_err(|e| e.to_string())?;
        stems.push((crate::audio::StemData::Memory(data.into()), ext(n)));
    }
    let is_mid = chart_name.to_ascii_lowercase().ends_with(".mid");
    build(&meta, &chart_bytes, is_mid, stems, fallback_preview)
}

/// Loads a chart that's already on disk (a chart folder or a .sng file); nothing is downloaded,
/// and the files are only read.
pub fn load_local(path: &std::path::Path, fallback_preview: f64) -> Result<Loaded, String> {
    if path.is_dir() { load_folder(path, fallback_preview) } else { load_sng(path, None, fallback_preview) }
}

/// Loads a .sng file. `temp` keeps a temporary download alive while its audio plays.
fn load_sng(path: &std::path::Path, temp: Option<Arc<crate::sng::TempFile>>, fallback_preview: f64) -> Result<Loaded, String> {
    let sng = crate::sng::read_index(path)?;
    let meta = |k: &str| sng.meta(k).map(String::from);
    let chart_entry = sng
        .files
        .iter()
        .find(|f| f.name.to_ascii_lowercase().ends_with(".mid"))
        .or_else(|| sng.files.iter().find(|f| f.name.to_ascii_lowercase().ends_with(".chart")))
        .ok_or("This chart has no .chart or .mid file")?;
    let chart_bytes = crate::sng::read_entry(path, &sng, chart_entry)?;
    let mut stems = Vec::new();
    for f in sng.files.iter().filter(|f| is_audio(&f.name)) {
        let slice = crate::sng::MaskedSlice::open(path, &sng, f, temp.clone()).map_err(|e| e.to_string())?;
        stems.push((crate::audio::StemData::Sng(slice), ext(&f.name)));
    }
    drop(temp); // from here on the audio readers own the temp file
    let is_mid = chart_entry.name.to_ascii_lowercase().ends_with(".mid");
    build(&meta, &chart_bytes, is_mid, stems, fallback_preview)
}

async fn load_inner(chart: &Chart, tx: &mut Sender<LoadEvent>) -> Result<Loaded, String> {
    let url = format!("{FILES_URL}/{}{}.sng", chart.md5, if chart.has_video_background { "_novideo" } else { "" });
    let resp = CLIENT
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Download failed: {e}"))?;
    let total = resp.content_length();

    // Stream to a temp file instead of RAM: charts can be hours long. The guard deletes the
    // file if loading is cancelled or fails, and later once playback no longer needs it.
    let dir = temp_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Can't create temp folder: {e}"))?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let path = dir.join(format!("{}-{}-{nanos}.sng", chart.md5, std::process::id()));
    let temp = Arc::new(crate::sng::TempFile(path.clone()));
    {
        use std::io::Write;
        let file = crate::sng::open_shared(&path, true).map_err(|e| format!("Can't write temp file: {e}"))?;
        let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
        let mut done: u64 = 0;
        let mut stream = resp.bytes_stream();
        let mut last = std::time::Instant::now();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Connection lost: {e}"))?;
            out.write_all(&chunk).map_err(|e| format!("Can't write temp file: {e}"))?;
            done += chunk.len() as u64;
            if let Some(t) = total
                && last.elapsed().as_millis() > 30
            {
                last = std::time::Instant::now();
                let _ = tx.try_send(LoadEvent::Progress(done as f32 / t.max(1) as f32));
            }
        }
        out.flush().map_err(|e| format!("Can't write temp file: {e}"))?;
    }
    let _ = tx.send(LoadEvent::Progress(1.0)).await;
    let fallback_preview = chart.preview_start_time.unwrap_or(-1) as f64;

    tokio::task::spawn_blocking(move || load_sng(&path, Some(temp), fallback_preview)).await.map_err(|e| e.to_string())?
}

// ---------- Note preparation (adjustParsedChart) ----------

#[derive(Debug, Clone, Copy)]
pub struct RNote {
    pub ms: f64,
    pub ms_len: f64,
    pub ty: u8,
    pub flags: u32,
}

pub struct RenderTrack {
    pub itype: InstrumentType,
    pub notes: Vec<RNote>,
    pub max_len: f64,
}

pub fn prepare_track(parsed: &ParsedChart, instrument: &str, difficulty: &str) -> Option<RenderTrack> {
    let track = parsed.tracks.iter().find(|t| t.instrument == instrument && t.difficulty == difficulty)?;
    let itype = instrument_type(instrument);
    let sp = &track.star_power;
    let mut sp_i = 0;
    let mut notes = Vec::new();
    for group in &track.note_groups {
        let tick = group[0].tick;
        while sp_i < sp.len() && sp[sp_i].tick + sp[sp_i].length < tick {
            sp_i += 1;
        }
        let in_sp = sp_i < sp.len() && tick >= sp[sp_i].tick && tick < sp[sp_i].tick + sp[sp_i].length;
        let mut g: Vec<RNote> = group
            .iter()
            .map(|n| RNote { ms: n.ms, ms_len: n.ms_length, ty: n.ty, flags: n.flags | if in_sp { SP_FLAG } else { 0 } })
            .collect();

        match itype {
            InstrumentType::SixFret => {
                // Two notes in the same lane (black + white) become a barre note.
                for (a, b, barre) in [(nt::BLACK1, nt::WHITE1, BARRE1), (nt::BLACK2, nt::WHITE2, BARRE2), (nt::BLACK3, nt::WHITE3, BARRE3)] {
                    if g.iter().filter(|n| n.ty == a || n.ty == b).count() > 1 {
                        let first = g.iter().position(|n| n.ty == a || n.ty == b).unwrap();
                        let mut keep = g[first];
                        keep.ty = barre;
                        g.retain(|n| n.ty != a && n.ty != b);
                        g.push(keep);
                    }
                }
            }
            InstrumentType::Drums => {
                for n in g.iter_mut() {
                    n.flags &= !nf::DISCO_NOFLIP;
                    if n.flags & nf::DISCO != 0 {
                        n.flags &= !nf::DISCO;
                        if n.ty == nt::RED_DRUM {
                            n.ty = nt::YELLOW_DRUM;
                            n.flags = (n.flags & !nf::TOM) | nf::CYMBAL;
                        } else if n.ty == nt::YELLOW_DRUM {
                            n.ty = nt::RED_DRUM;
                            n.flags = (n.flags & !nf::CYMBAL) | nf::TOM;
                        }
                    }
                }
            }
            InstrumentType::FiveFret => {}
        }
        notes.extend(g);
    }
    let max_len = notes.iter().map(|n| n.ms_len).fold(0.0, f64::max);
    Some(RenderTrack { itype, notes, max_len })
}

fn lane(ty: u8) -> f32 {
    match ty {
        nt::GREEN | nt::RED_DRUM | nt::BLACK1 | nt::WHITE1 | BARRE1 => 0.0,
        nt::RED | nt::YELLOW_DRUM | nt::BLACK2 | nt::WHITE2 | BARRE2 => 1.0,
        nt::YELLOW | nt::BLUE_DRUM | nt::OPEN | nt::KICK | nt::BLACK3 | nt::WHITE3 | BARRE3 => 2.0,
        nt::BLUE | nt::GREEN_DRUM => 3.0,
        nt::ORANGE => 4.0,
        _ => 0.0,
    }
}

fn note_x(itype: InstrumentType, ty: u8) -> f32 {
    let left = match itype {
        InstrumentType::Drums => 0.135,
        InstrumentType::SixFret if ty != nt::OPEN => 0.2,
        _ => 0.035,
    };
    left - NOTE_SPAN_WIDTH / 2.0 + SCALE + ((NOTE_SPAN_WIDTH - SCALE) / 5.0) * lane(ty)
}

/// Clone Hero's lane positions (from its player prefabs): frets 0.2 apart, drum pads at
/// ±0.122 and ±0.365; open notes and kicks across the middle.
fn game_x(itype: InstrumentType, ty: u8) -> f32 {
    match (itype, ty) {
        (_, nt::KICK) | (_, nt::OPEN) => 0.0,
        (InstrumentType::Drums, _) => [-0.365, -0.122, 0.122, 0.365][(lane(ty) as usize).min(3)],
        _ => -0.4 + 0.2 * lane(ty),
    }
}

/// Sustain colour (as 0xRRGGBB) for a note type, same as the original player.
pub fn sustain_hex(ty: u8) -> u32 {
    match ty {
        nt::GREEN | nt::GREEN_DRUM => 0x01B11A,
        nt::RED | nt::RED_DRUM => 0xDD2214,
        nt::YELLOW | nt::YELLOW_DRUM => 0xDEEB52,
        nt::BLUE | nt::BLUE_DRUM => 0x006CAF,
        nt::OPEN => 0x8A0BB5,
        nt::ORANGE => 0xF8B272,
        _ => 0xFFFFFF,
    }
}

pub const SUSTAIN_HEXES: [u32; 7] = [0x01B11A, 0xDD2214, 0xDEEB52, 0x006CAF, 0x8A0BB5, 0xF8B272, 0xFFFFFF];

// ---------- Camera (ChartCamera: fov 90, position (0,-1.3,0.8), rotation.x 60°) ----------

/// Smaller copies of a texture are picked so they're at least this many times bigger than
/// where they're drawn, which keeps them sharp on high-DPI screens.
const OVERSAMPLE: f32 = 1.4;

/// The original player's camera, zoomed so the strikeline fits the stage with a margin and
/// anchored to the bottom; any spare height shows more of the highway ahead.
struct Camera {
    w: f32,
    /// Pixels per unit of the projection plane (focal length 1).
    f: f32,
    /// Screen y of the horizon line through the camera axis.
    oy: f32,
    cos: f32,
    sin: f32,
}

const CAM_Y: f32 = -1.3;
const CAM_Z: f32 = 0.8;
/// Furthest point of the highway that is ever shown (the original shows up to 0.9).
const MAX_Y_TOP: f32 = 2.4;
/// Notes and sustains fade in over this much of the far end of the highway (world units).
const FADE_IN: f32 = 0.35;
/// Corner radius of the stage the preview is drawn in; the highway is kept inside it.
pub const STAGE_RADIUS: f32 = 8.0;

/// How far in from each side a row of the stage must stay to remain inside its rounded outline
/// (plus a pixel for the border).
fn stage_inset(sy: f32, height: f32) -> f32 {
    let r = STAGE_RADIUS;
    let d = [r - sy, sy - (height - r)].into_iter().fold(0.0f32, f32::max).min(r);
    1.0 + r - (r * r - d * d).max(0.0).sqrt()
}

impl Camera {
    /// `half_width`: how far left/right of centre (world units, at the strikeline) must fit.
    /// `below`: how far below the strikeline centre the strikeline sprite reaches.
    fn new(size: Size, half_width: f32, below: f32) -> Self {
        let r = 60f32.to_radians();
        let mut cam = Self { w: size.width, f: 1.0, oy: 0.0, cos: r.cos(), sin: r.sin() };
        let (u, v_line, depth) = cam.plane(half_width, -1.0);
        let v_bottom = v_line - below / depth;
        let v_top = cam.plane(0.0, 0.9).1;
        let pad_x = (size.width * 0.035).max(8.0);
        let (pad_top, pad_bottom) = (6.0, (size.height * 0.03).max(8.0));
        let fit_w = (size.width - 2.0 * pad_x) / (2.0 * u);
        let fit_h = (size.height - pad_top - pad_bottom) / (v_top - v_bottom);
        cam.f = fit_w.min(fit_h).max(1.0);
        cam.oy = size.height - pad_bottom + cam.f * v_bottom;
        cam
    }

    /// World point on the highway plane (z = 0) -> projection plane (x, y) and depth.
    fn plane(&self, x: f32, y: f32) -> (f32, f32, f32) {
        let dy = y - CAM_Y;
        let dz = -CAM_Z;
        let yc = self.cos * dy + self.sin * dz;
        let depth = (self.sin * dy - self.cos * dz).max(0.01);
        (x / depth, yc / depth, depth)
    }

    /// World point on the highway plane (z = 0) -> (screen x, screen y, depth).
    fn project(&self, x: f32, y: f32) -> (f32, f32, f32) {
        let (u, v, depth) = self.plane(x, y);
        (self.w * 0.5 + self.f * u, self.oy - self.f * v, depth)
    }

    /// Screen y -> world y on the highway plane.
    fn unproject_y(&self, sy: f32) -> f32 {
        // Stay below the horizon (where the plane would be infinitely far away).
        let ndc = ((self.oy - sy) / self.f).min(self.cos / self.sin - 0.02);
        let dz = -CAM_Z;
        let dy = dz * (self.sin + ndc * self.cos) / (ndc * self.sin - self.cos);
        dy + CAM_Y
    }

    /// Pixels per world unit for a sprite at `depth`.
    fn px_per_unit(&self, depth: f32) -> f32 {
        self.f / depth
    }
}

/// Clone Hero's camera (from its player prefabs): at (0, 1, -3.97) behind the strikeline,
/// tilted 6° down, with a narrow 20° field of view, looking along the highway. The highway is
/// the ground (y = 0) and runs along z; the strikeline is at z = 0.09 (drums 0.106). Fitted to
/// the stage like the game's own view: from just below the strikeline to the far end of the
/// highway, rails included. (A narrower stage than the game's screen just shows less around it.)
struct GameCam {
    f: f32,
    cx: f32,
    cy: f32,
    cos: f32,
    sin: f32,
}

const GAME_CAM_Y: f32 = 1.0;
const GAME_CAM_Z: f32 = -3.97;
/// The highway's width, and the rails' (just outside it, from ±0.475 out).
const GAME_NECK: f32 = 0.975;
const GAME_RAIL_IN: f32 = 0.475;
const GAME_RAIL_W: f32 = 0.107;

impl GameCam {
    fn new(size: Size, half_width: f32, strike: f32, z_far: f32) -> Self {
        let t = 6f32.to_radians();
        let mut c = GameCam { f: 1.0, cx: 0.0, cy: 0.0, cos: t.cos(), sin: t.sin() };
        // The bottom of the strike rings (they stand partly below the line), and the top of a
        // note at the far end.
        let (u_near, v_near, _) = c.view(half_width, -0.05, strike);
        let (_, v_far, _) = c.view(0.0, 0.12, z_far);
        let pad_x = (size.width * 0.035).max(8.0);
        let (pad_top, pad_bottom) = (6.0, (size.height * 0.03).max(8.0));
        let fit_w = (size.width - 2.0 * pad_x) / (2.0 * u_near);
        let fit_h = (size.height - pad_top - pad_bottom) / (v_far - v_near).max(1e-4);
        c.f = fit_w.min(fit_h).max(1.0);
        c.cx = size.width * 0.5;
        c.cy = size.height - pad_bottom + c.f * v_near;
        c
    }

    /// A point -> (x / depth, up / depth, depth) in the camera's view.
    fn view(&self, x: f32, y: f32, z: f32) -> (f32, f32, f32) {
        let (dy, dz) = (y - GAME_CAM_Y, z - GAME_CAM_Z);
        let depth = (-dy * self.sin + dz * self.cos).max(0.01);
        let up = dy * self.cos + dz * self.sin;
        (x / depth, up / depth, depth)
    }

    /// A point on the highway -> (screen x, screen y, depth).
    fn project(&self, x: f32, z: f32) -> (f32, f32, f32) {
        let (u, v, d) = self.view(x, 0.0, z);
        (self.cx + self.f * u, self.cy - self.f * v, d)
    }

    /// Screen y -> how far along the highway (z) that row of it is.
    fn unproject_y(&self, sy: f32) -> f32 {
        // Below the horizon, where the highway would be infinitely far away.
        let v = ((self.cy - sy) / self.f).min(self.sin / self.cos - 0.002);
        let dy = -GAME_CAM_Y;
        let dz = dy * (self.cos + v * self.sin) / (v * self.cos - self.sin);
        dz + GAME_CAM_Z
    }
}

/// The original player's camera, or Clone Hero's.
enum Cam {
    Original(Camera),
    Game(GameCam),
}

impl Cam {
    fn project(&self, x: f32, y: f32) -> (f32, f32, f32) {
        match self {
            Cam::Original(c) => c.project(x, y),
            Cam::Game(c) => c.project(x, y),
        }
    }
    fn unproject_y(&self, sy: f32) -> f32 {
        match self {
            Cam::Original(c) => c.unproject_y(sy),
            Cam::Game(c) => c.unproject_y(sy),
        }
    }
    fn px_per_unit(&self, depth: f32) -> f32 {
        match self {
            Cam::Original(c) => c.px_per_unit(depth),
            Cam::Game(c) => c.f / depth,
        }
    }
}

// ---------- Canvas program ----------

pub struct Highway<'a> {
    pub track: &'a RenderTrack,
    pub textures: &'a TextureSet,
    pub time_ms: f64,
    pub anim_ms: u64,
    /// In Clone Hero's layout: the note speed and highway length to show (otherwise the
    /// player's own, from their profile).
    pub game_speed: Option<GameSpeed>,
}

/// Clone Hero's note speed and highway length (percent) settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameSpeed {
    pub note_speed: f32,
    pub highway_length: f32,
}

/// The game's settings: (lowest, highest, step), and their defaults (from its profile class's
/// constructor in GameAssembly.dll).
pub const NOTE_SPEED_RANGE: (f32, f32, f32) = (1.0, 100.0, 1.0);
pub const HIGHWAY_LENGTH_RANGE: (f32, f32, f32) = (20.0, 1000.0, 10.0);
pub const NOTE_SPEED_DEFAULT: f32 = 7.0;
pub const HIGHWAY_LENGTH_DEFAULT: f32 = 100.0;

impl GameSpeed {
    /// Highway units a note moves per millisecond: the game's note renderer moves notes at the
    /// note speed setting in units per second (`noteSpeed = (float)setting`).
    fn per_ms(self) -> f32 {
        self.note_speed.max(1.0) / 1000.0
    }
    /// Where the track fades out, and over how long: the track materials' 7.75 and 2.75, both
    /// times highway length / 100 (as the game's track fade manager scales them). Notes never
    /// show past where they appear (7.87).
    fn fade(self) -> (f32, f32) {
        let k = self.highway_length.max(1.0) / 100.0;
        (GAME_FAR_LIMIT * k, GAME_FADE * k)
    }
}

const GAME_FAR_LIMIT: f32 = 7.75;
const GAME_FADE: f32 = 2.75;
/// Where notes appear, and where the highway itself ends.
const GAME_NOTE_SPAWN: f32 = 7.87;
const GAME_NECK_END: f32 = 7.97;

impl<Message> canvas::Program<Message> for Highway<'_> {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        use iced::widget::image::FilterMethod;
        let mut frame = Frame::new(renderer, bounds.size());
        let itype = self.track.itype;

        // Clone Hero's layout when its sprites are read from the install, else the original's.
        let game = self.textures.game.as_ref();
        // Highway: textured plane, width depends on instrument, scrolling texture.
        let hw_width = match itype {
            _ if game.is_some() => GAME_NECK,
            InstrumentType::Drums => 0.9,
            InstrumentType::SixFret => 0.7,
            InstrumentType::FiveFret => 1.0,
        };
        // Strikeline sprite size in world units, centred at y = -1.
        let (sl_w, sl_h) = {
            let s = if itype == InstrumentType::SixFret { 0.141 } else { 0.19 };
            let a = self.textures.strikeline_aspect;
            if a > 1.0 { (a * s, s) } else { (s, s / a) }
        };
        // Where the strikeline is along the highway, and how far a note moves per millisecond.
        let strike = match (game, itype) {
            (None, _) => -1.0,
            (Some(_), InstrumentType::Drums) => 0.106,
            (Some(_), _) => 0.09,
        };
        let speed = game.map(|g| self.game_speed.unwrap_or(GameSpeed { note_speed: g.note_speed, highway_length: g.highway_length }));
        let per_ms = speed.map_or((2.0 / HIGHWAY_DURATION_MS) as f32, GameSpeed::per_ms);
        let (fade_at, fade_len) = speed.map_or((MAX_Y_TOP, 1.0), GameSpeed::fade);
        // What shows of the highway: up to where it fades out (or ends).
        let far = fade_at.min(GAME_NECK_END);
        // The game's tracks fade out over their last stretch (everything on them does too).
        let track_fade = |z: f32| if game.is_some() { ((fade_at - z) / fade_len.max(1e-3)).clamp(0.0, 1.0) } else { 1.0 };
        // Notes are timed to cross z = 0 (the game's strikeLine), just in front of the strike
        // rings; in the original, to cross the strikeline.
        let hit = if game.is_some() { 0.0 } else { strike };
        let cam = match game {
            Some(_) => Cam::Game(GameCam::new(bounds.size(), GAME_RAIL_IN + GAME_RAIL_W, strike, far)),
            None => Cam::Original(Camera::new(bounds.size(), (sl_w / 2.0).max(hw_width / 2.0) + 0.01, sl_h / 2.0)),
        };
        let nx = |ty: u8| if game.is_some() { game_x(itype, ty) } else { note_x(itype, ty) };

        let now = self.time_ms;
        let y_bottom = -1.1f32;
        // Show as much of the highway as fits (but at least the original's 0.9); in the game's
        // layout, up to the end of its highway.
        let y_top = match game {
            Some(_) => cam.unproject_y(0.0).clamp(strike + 0.5, far),
            None => cam.unproject_y(0.0).clamp(0.9, MAX_Y_TOP),
        };
        let end = now + ((y_top.min(if game.is_some() { GAME_NOTE_SPAWN } else { y_top }) - hit) / per_ms) as f64;
        let to_y = |ms: f64| hit + ((ms - now) as f32) * per_ms;
        // The highway texture repeats every this much along it: its own proportions (1:2) in
        // the game's layout, every unit in the original's.
        let tile = if game.is_some() {
            // As the game's highway scroller sizes it: the picture 0.96 wide, the neck stretched
            // 9.2 times along, repeating six times over it.
            let t = &self.textures.highway[0];
            0.96 * (t.rows.len() as f32 / t.width as f32) * 9.2 / 6.0
        } else {
            1.0
        };

        let sy_top = cam.project(0.0, y_top).1.max(0.0).floor();
        let sy_bottom = bounds.height - 1.0;
        let offset = (0.9 * (now / 1000.0) * (HIGHWAY_DURATION_MS / 1000.0)) as f32;
        let full = &self.textures.highway[0];
        let full_rows = full.rows.len() as f32;
        let mut sy = sy_top;
        while sy < sy_bottom {
            let sy2 = sy + 1.0;
            let wy = cam.unproject_y(sy + 0.5);
            let inset = stage_inset(sy + 0.5, bounds.height);
            let lx = cam.project(-hw_width / 2.0, wy).0.max(inset);
            let rx = cam.project(hw_width / 2.0, wy).0.min(bounds.width - inset);
            // Texture texels per screen pixel, across and along the highway; use a copy that's
            // about the right size so it doesn't shimmer.
            let across = full.width as f32 / (rx - lx).max(1.0);
            let along = (cam.unproject_y(sy) - cam.unproject_y(sy2)).abs() * full_rows / tile;
            let level = (across.max(along * 0.5) / OVERSAMPLE).max(1.0).log2().floor() as usize;
            let level = &self.textures.highway[level.min(self.textures.highway.len() - 1)];
            let tex_v = if game.is_some() { ((wy + now as f32 * per_ms) / tile).rem_euclid(1.0) } else { (wy - y_bottom + offset).rem_euclid(1.0) };
            let n = level.rows.len();
            let row = (((1.0 - tex_v) * n as f32) as usize).min(n - 1);
            let mut img = canvas::Image::new(level.rows[row].clone());
            // Nearest sampling on whole pixels: smooth sampling of these one-pixel rows gave the odd
            // bright speck. The copy picked above is already close to the drawn size.
            img.snap = true;
            img.filter_method = FilterMethod::Nearest;
            img.opacity = track_fade(wy);
            frame.draw_image(Rectangle::new(Point::new(lx, sy), Size::new(rx - lx, sy2 - sy + 0.5)), img);
            // The game's rails along each side.
            if let Some((left, right)) = game.and_then(|g| g.rail.as_ref()) {
                for (img, from, to) in [(left, -GAME_RAIL_IN - GAME_RAIL_W, -GAME_RAIL_IN), (right, GAME_RAIL_IN, GAME_RAIL_IN + GAME_RAIL_W)] {
                    let (a, b) = (cam.project(from, wy).0.max(inset), cam.project(to, wy).0.min(bounds.width - inset));
                    if b > a {
                        let mut r = canvas::Image::new(img.clone());
                        r.opacity = track_fade(wy);
                        frame.draw_image(Rectangle::new(Point::new(a, sy), Size::new(b - a, sy2 - sy + 0.5)), r);
                    }
                }
            }
            sy = sy2;
        }

        // Visible notes: heads within the highway window, plus sustains still playing.
        let notes = &self.track.notes;
        let start = notes.partition_point(|n| n.ms < now - self.track.max_len);
        let visible: Vec<&RNote> = notes[start..]
            .iter()
            .take_while(|n| n.ms < end)
            .filter(|n| n.ms >= now || n.ms + n.ms_len > now)
            .collect();

        let clip_lo = hit;
        let clip_hi = y_top;

        let draw_sprite = |frame: &mut Frame, n: &RNote| {
            let Some(tex) = self.textures.note(n.ty, n.flags) else { return };
            let x = nx(n.ty);
            let y = to_y(n.ms);
            // In the game's layout a note that's been hit is gone.
            if game.is_some() && y < hit - 0.005 {
                return;
            }
            let (sx, sy, depth) = cam.project(x, y);
            let k = cam.px_per_unit(depth);
            let (ws, hs, cx, cy) = if game.is_some() {
                // The game's notes stand upright on the highway: their widths, and the height
                // of the point they stand on, from its sprites.
                // Times the note renderer's scale: 1.1 on guitar, 1.2 on drums.
                let scale = if itype == InstrumentType::Drums { 1.2 } else { 1.1 };
                let w = scale
                    * match n.ty {
                        nt::KICK => 0.8093,
                        nt::OPEN => 0.8939,
                        _ if itype == InstrumentType::Drums => 0.2169,
                        _ => 0.2226,
                    };
                let pivot = match n.ty {
                    nt::KICK => 0.039,
                    _ if itype == InstrumentType::Drums => 0.206,
                    _ => 0.16,
                };
                (w, w / tex.aspect, 0.5, pivot)
            } else {
                let scale = if n.ty == nt::KICK {
                    0.045
                } else if n.ty == nt::OPEN && itype == InstrumentType::SixFret {
                    0.04
                } else {
                    SCALE
                };
                let (cx, cy) = if n.ty == nt::KICK { (0.62, -0.5) } else { (0.5, 0.0) };
                (scale * tex.aspect, scale, cx, cy)
            };
            let (w, h) = (ws * k, hs * k);
            let rect = Rectangle::new(Point::new(sx - cx * w, sy - (1.0 - cy) * h), Size::new(w, h));
            // Notes fade in over the far end of the highway instead of popping in.
            let opacity = ((clip_hi - y) / FADE_IN).clamp(0.0, 1.0);
            if opacity <= 0.0 {
                return;
            }
            let mut image = canvas::Image::new(tex.frame(self.anim_ms).pick(w * OVERSAMPLE).clone());
            image.opacity = opacity;
            if game.is_some() {
                // Fading out with the track instead (no flash of a note at the far end).
                image.opacity = track_fade(y);
                if image.opacity > 0.0 {
                    frame.draw_image(rect, image);
                }
                return;
            }
            // Clipping planes y in [-1, y_top]: the sprite faces the camera, whose up vector has
            // world-y component cos(60°) = 0.5, so a vertical offset v changes world y by 0.5·v.
            // Also never above the far end of the highway on screen, where a note's top would
            // otherwise show before the note does.
            let top = (sy - (clip_hi - y) / 0.5 * k).max(sy_top);
            let bottom = sy - (clip_lo - y) / 0.5 * k;
            if rect.y >= top && rect.y + rect.height <= bottom {
                frame.draw_image(rect, image);
            } else if bottom > top {
                let clip = Rectangle::new(Point::new(rect.x - 1.0, top), Size::new(rect.width + 2.0, bottom - top));
                frame.with_clip(clip, |f| f.draw_image(rect, image));
            }
        };

        // Kicks (render order 1, drawn far to near).
        for n in visible.iter().rev().filter(|n| n.ty == nt::KICK) {
            draw_sprite(&mut frame, n);
        }

        // Sustains (render order 2): planes on the highway, clipped to [-1, y_top].
        // Drawn as thin solid-colour image slices (like the highway) rather than filled paths:
        // iced draws a canvas's paths underneath all of its images, which hid them entirely.
        for n in visible.iter().filter(|n| n.ms_len > 0.0) {
            let sp = n.flags & SP_FLAG != 0;
            // Once its note is hit, a sustain being held looks different (in the game, it glows).
            let held = n.ms <= now;
            let Some(solid) = held
                .then(|| self.textures.sustains_held.get(&(n.ty, sp)))
                .flatten()
                .or_else(|| self.textures.sustains.get(&(n.ty, sp)))
                .or_else(|| self.textures.solid.get(&sustain_hex(n.ty)))
            else {
                continue;
            };
            let x = nx(n.ty);
            let half_w = match (game, n.ty) {
                // The game's sustain pictures: an open note's 0.85 wide. The others are sized by
                // their shader; measured against the game, the body (a third of the picture) is
                // about 0.07 wide, under a third of a note.
                (Some(_), nt::OPEN) => 0.425,
                (Some(_), _) => 0.105,
                (None, nt::OPEN) => SCALE * 5.0 / 2.0,
                (None, _) => SCALE * 0.3 / 2.0,
            };
            let lift = if game.is_some() { 0.0 } else { 0.03 };
            let y0 = (to_y(n.ms) + lift).max(clip_lo);
            let tail = to_y(n.ms) + lift + n.ms_len as f32 * per_ms;
            let y1 = tail.min(clip_hi);
            // The game's sustains end in a rounded tip: the last stretch of the sustain (as long
            // as its picture's top is, for its width) is drawn from that, row by row.
            let cap = self.textures.sustain_caps.get(&(n.ty, sp)).map(|(rest, hold)| if held { hold } else { rest });
            let cap_len = self.textures.sustain_cap_ratio.get(&n.ty).map_or(0.0, |r| r * half_w * 2.0).min(n.ms_len as f32 * per_ms);
            let cap_from = tail - cap_len;
            if y1 <= y0 {
                continue;
            }
            let sy_far = cam.project(x, y1).1;
            let sy_near = cam.project(x, y0).1;
            // The game's sustains can be see-through: whole-pixel rows that don't overlap, or the
            // overlaps show as bright lines that crawl as the sustain moves.
            let (mut sy, rows) = if game.is_some() { (sy_far.floor(), 1.0) } else { (sy_far, 2.0) };
            while sy < sy_near {
                let sy2 = (sy + rows).min(if game.is_some() { sy_near.ceil() } else { sy_near });
                let wy = cam.unproject_y((sy + sy2) * 0.5);
                let (lx, _, _) = cam.project(x - half_w, wy);
                let (rx, _, _) = cam.project(x + half_w, wy);
                let pick = match cap {
                    Some(rows) if wy > cap_from && cap_len > 0.0 && !rows.is_empty() => {
                        let r = (((tail - wy) / cap_len) * rows.len() as f32) as usize;
                        rows[r.min(rows.len() - 1)].clone()
                    }
                    _ => solid.clone(),
                };
                let mut img = canvas::Image::new(pick).filter_method(if game.is_some() { FilterMethod::Linear } else { FilterMethod::Nearest });
                img.opacity = if game.is_some() { track_fade(wy) } else { ((clip_hi - wy) / FADE_IN).clamp(0.0, 1.0) };
                let overlap = if game.is_some() { 0.0 } else { 0.6 };
                frame.draw_image(Rectangle::new(Point::new(lx, sy), Size::new((rx - lx).max(1.0), sy2 - sy + overlap)), img);
                sy = sy2;
            }
        }

        // Strikeline (render order 3): sprite centered at y = -1, or the frets one by one, lit
        // while a note of their lane is being hit.
        if !self.textures.strikers.is_empty() {
            let hit = |n: &RNote| (n.ms <= now && now - n.ms < 120.0) || (n.ms <= now && now < n.ms + n.ms_len);
            let recent = &notes[notes.partition_point(|n| n.ms < now - self.track.max_len.max(FLAME_MS))..notes.partition_point(|n| n.ms <= now)];
            let open = recent.iter().any(|n| (n.ty == nt::OPEN || n.ty == nt::KICK) && hit(n));
            for s in &self.textures.strikers {
                let (sx, sy, depth) = cam.project(nx(s.ty), strike);
                let k = cam.px_per_unit(depth);
                let (w, h) = (s.width * k, s.width / s.aspect * k);
                // The game's stand on the strikeline at 39% of their height; the original's are
                // centred on it.
                let up = if game.is_some() { 1.0 - 0.39 } else { 0.5 };
                let mips = if recent.iter().any(|n| n.ty == s.ty && hit(n)) {
                    &s.lit
                } else if open {
                    &s.lit_open
                } else {
                    &s.idle
                };
                frame.draw_image(Rectangle::new(Point::new(sx - w / 2.0, sy - h * up), Size::new(w, h)), canvas::Image::new(mips.pick(w * OVERSAMPLE).clone()));
            }
        } else {
            let (sx, sy, depth) = cam.project(0.0, -1.0);
            let k = cam.px_per_unit(depth);
            let (w, h) = (sl_w * k, sl_h * k);
            frame.draw_image(
                Rectangle::new(Point::new(sx - w / 2.0, sy - h / 2.0), Size::new(w, h)),
                canvas::Image::new(self.textures.strikeline.pick(w * OVERSAMPLE).clone()),
            );
        }

        // Notes (render order 4), far to near.
        for n in visible.iter().rev().filter(|n| n.ty != nt::KICK) {
            draw_sprite(&mut frame, n);
        }

        // Hit flames, over the notes: one on each fret just hit, and one along the strike line
        // for an open note or a kick.
        if let Some(fl) = &self.textures.flames {
            let recent = &notes[notes.partition_point(|n| n.ms < now - FLAME_MS)..notes.partition_point(|n| n.ms <= now)];
            let held = &notes[notes.partition_point(|n| n.ms < now - self.track.max_len)..notes.partition_point(|n| n.ms <= now)];
            for s in &self.textures.strikers {
                let (sx, sy, depth) = cam.project(nx(s.ty), strike);
                let k = cam.px_per_unit(depth);
                let fw = s.width * k;
                let base = sy + fw / s.aspect * 0.15;
                // As the game has them (sizes against its 0.197-wide fret; anchored at the
                // fret's centre by each sprite's pivot, as fractions of it from left and bottom).
                let game = |size: f32| fw * size / GAME_FRET;
                let at = |z: f32, pivot: (f32, f32)| Rectangle::new(Point::new(sx - z * pivot.0, sy - z * (1.0 - pivot.1)), Size::new(z, z));
                // Sparks while a sustain of this lane is held, looping.
                if !fl.hold.is_empty() && held.iter().any(|n| n.ty == s.ty && n.ms_len > 0.0 && now < n.ms + n.ms_len) {
                    let i = (self.anim_ms / HOLD_FRAME_MS) as usize % fl.hold.len();
                    frame.draw_image(at(game(0.32 * 0.9), (0.5, 0.27)), canvas::Image::new(fl.hold[i].clone()));
                }
                let Some(n) = recent.iter().rev().find(|n| n.ty == s.ty) else { continue };
                let since = now - n.ms;
                if !fl.note_frames.is_empty() {
                    // The burst of particles: three, up in a narrow cone, slowing, fading.
                    if since < PARTICLE_MS {
                        let t = (since / 1000.0) as f32;
                        let mut seed = (n.ms as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ s.ty as u64;
                        for _ in 0..3 {
                            seed ^= seed << 13;
                            seed ^= seed >> 7;
                            seed ^= seed << 17;
                            let ang = ((seed % 1000) as f32 / 1000.0 - 0.5) * 2.0 * 20f32.to_radians();
                            let v = game(0.7);
                            let px = sx + ang.sin() * v * t;
                            let py = sy - ang.cos() * v * t + 0.5 * game(0.4 * 9.81) * t * t;
                            let z = game(0.05) * 3.0;
                            let mut img = canvas::Image::new(fl.glow.clone());
                            img.opacity = 1.0 - (since / PARTICLE_MS) as f32;
                            frame.draw_image(Rectangle::new(Point::new(px - z / 2.0, py - z / 2.0), Size::new(z, z)), img);
                        }
                    }
                    // Star power's spark burst, then the hit flame on top.
                    let frame_at = |per: f64| (since / per) as usize;
                    let i = frame_at(SPARK_FRAME_MS);
                    if i < fl.sparks.len() {
                        frame.draw_image(at(game(0.384), (0.509, 0.04)), canvas::Image::new(fl.sparks[i].clone()));
                    }
                    let i = frame_at(FLAME_FRAME_MS);
                    if i < fl.note_frames.len() {
                        frame.draw_image(at(game(0.369), (0.5, 0.2)), canvas::Image::new(fl.note_frames[i].clone()));
                    }
                    continue;
                }
                let t = (since / NOTE_FLAME_MS) as f32;
                if t >= 1.0 {
                    continue;
                }
                let g = fw * (1.1 + 0.5 * t);
                let mut glow = canvas::Image::new(fl.glow.clone());
                glow.opacity = (1.0 - t) * 0.7;
                frame.draw_image(Rectangle::new(Point::new(sx - g / 2.0, base - g * 0.6), Size::new(g, g)), glow);
                let w = fw * (0.75 + 0.35 * t);
                let h = w / fl.note_aspect;
                let mut img = canvas::Image::new(fl.note.clone());
                img.opacity = 1.0 - t * t;
                frame.draw_image(Rectangle::new(Point::new(sx - w / 2.0, base - h), Size::new(w, h)), img);
            }
            if let (Some(n), Some(first), Some(last)) = (
                recent.iter().rev().find(|n| (n.ty == nt::OPEN || n.ty == nt::KICK) && now - n.ms < WIDE_FLAME_MS),
                self.textures.strikers.first(),
                self.textures.strikers.last(),
            ) && !fl.wide.is_empty()
            {
                let t = ((now - n.ms) / WIDE_FLAME_MS) as f32;
                let i = ((t * fl.wide.len() as f32) as usize).min(fl.wide.len() - 1);
                let (l, sy, depth) = cam.project(nx(first.ty) - first.width / 2.0, strike);
                let (r, _, _) = cam.project(nx(last.ty) + last.width / 2.0, strike);
                let h = first.width * cam.px_per_unit(depth) * 0.75;
                let mut img = canvas::Image::new(fl.wide[i].clone());
                img.opacity = 1.0 - t * t;
                frame.draw_image(Rectangle::new(Point::new(l, sy - h * 0.85), Size::new(r - l, h)), img);
            }
        }

        // Fade the far end of the highway (and notes arriving there) into the background.
        let sy_line = cam.project(0.0, strike).1;
        let fade_h = ((sy_line - sy_top) * 0.22).max(0.0);
        // Only as wide as the highway where the fade ends, so it never covers the stage's border.
        let fade_y = cam.unproject_y(sy_top + fade_h);
        let fade_l = (cam.project(-hw_width / 2.0, fade_y).0 - 2.0).max(1.0);
        let fade_r = (cam.project(hw_width / 2.0, fade_y).0 + 2.0).min(bounds.width - 1.0);
        // (The game's layout fades the highway out itself.)
        if fade_h > 1.0 && game.is_none() {
            frame.draw_image(
                Rectangle::new(Point::new(fade_l, sy_top - 1.0), Size::new(fade_r - fade_l, fade_h + 1.0)),
                canvas::Image::new(self.textures.fade.clone()),
            );
        }

        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod sync_tests {
    use super::audio_start_ms;

    #[test]
    fn offsets_and_delays() {
        // "Remix 10": Offset = -2, delay = -2000 -> the audio starts 2 s in (Offset only).
        assert_eq!(audio_start_ms(-2.0, -2000.0), 2000.0);
        // "Of Your Heart": Offset = -1.045 -> notes line up 1.045 s into the audio.
        assert_eq!(audio_start_ms(-1.045, 0.0), 1045.0);
        // "1HP": delay = 2000 -> notes 2 s later, i.e. the audio starts 2 s before chart time 0.
        assert_eq!(audio_start_ms(0.0, 2000.0), -2000.0);
        assert_eq!(audio_start_ms(0.0, 0.0), 0.0);
    }
}
