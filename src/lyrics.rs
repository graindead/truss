//! Lyrics tool: synced lyrics from LRCLIB, written into notes.chart / notes.mid.
//!
//! Unlike the original Bridge (which converts times with only the first tempo marker), lyric
//! times are converted with the chart's full tempo map, so they stay in sync through tempo
//! changes.

use crate::api::CLIENT;
use crate::library::LocalChart;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const LRCLIB: &str = "https://lrclib.net/api";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LrcResult {
    pub id: i64,
    #[serde(default)]
    pub track_name: String,
    #[serde(default)]
    pub artist_name: String,
    #[serde(default)]
    pub album_name: Option<String>,
    /// Seconds.
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub instrumental: bool,
    #[serde(default)]
    pub synced_lyrics: Option<String>,
}

/// LRCLIB results that have synced (timestamped) lyrics.
pub async fn search(artist: String, title: String) -> Result<Vec<LrcResult>, String> {
    let q = format!("{artist} {title}");
    let r: Vec<LrcResult> = CLIENT
        .get(format!("{LRCLIB}/search"))
        .query(&[("q", q.trim())])
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(r.into_iter().filter(|r| r.synced_lyrics.as_deref().is_some_and(|s| !s.is_empty()) && !r.instrumental).collect())
}

/// Best automatic match (batch mode): LRCLIB's exact lookup with the song length, then search.
pub async fn best_match(artist: String, title: String, album: String, duration_s: f64) -> Result<Option<LrcResult>, String> {
    let mut req = CLIENT
        .get(format!("{LRCLIB}/get"))
        .query(&[("artist_name", artist.as_str()), ("track_name", title.as_str())]);
    if !album.is_empty() {
        req = req.query(&[("album_name", album.as_str())]);
    }
    if duration_s > 0.0 {
        req = req.query(&[("duration", (duration_s.round() as i64).to_string())]);
    }
    if let Ok(resp) = req.send().await
        && resp.status().is_success()
        && let Ok(r) = resp.json::<LrcResult>().await
        && r.synced_lyrics.as_deref().is_some_and(|s| !s.is_empty())
    {
        return Ok(Some(r));
    }
    // Fall back to search, preferring a close duration.
    let mut results = search(artist, title).await?;
    if duration_s > 0.0 {
        results.retain(|r| r.duration.is_none_or(|d| (d - duration_s).abs() < 8.0));
    }
    Ok(results.into_iter().next())
}

#[derive(Debug, Clone, PartialEq)]
pub struct LrcLine {
    pub ms: f64,
    pub text: String,
}

/// Parses `[mm:ss.xx] text` lines (several timestamps per line allowed), sorted by time.
pub fn parse_lrc(s: &str) -> Vec<LrcLine> {
    let mut out = Vec::new();
    for line in s.lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while let Some(r) = rest.strip_prefix('[') {
            let Some(end) = r.find(']') else { break };
            let stamp = &r[..end];
            let parsed = (|| {
                let (m, sec) = stamp.split_once(':')?;
                let (s, frac) = sec.split_once(['.', ':'])?;
                let (m, s): (u64, u64) = (m.parse().ok()?, s.parse().ok()?);
                let frac_ms: u64 = match frac.len() {
                    2 => frac.parse::<u64>().ok()? * 10,
                    3 => frac.parse().ok()?,
                    _ => return None,
                };
                Some(((m * 60 + s) * 1000 + frac_ms) as f64)
            })();
            match parsed {
                Some(t) => times.push(t),
                None => break, // e.g. [ar:Artist] tags
            }
            rest = &r[end + 1..];
        }
        let text = rest.trim();
        if !text.is_empty() {
            for t in times {
                out.push(LrcLine { ms: t, text: text.to_string() });
            }
        }
    }
    out.sort_by(|a, b| a.ms.total_cmp(&b.ms));
    out
}

// ---------- Tempo maps ----------

struct TempoMap {
    resolution: f64,
    /// (tick, ms, bpm)
    tempos: Vec<(f64, f64, f64)>,
}

impl TempoMap {
    fn new(resolution: f64, mut raw: Vec<(i64, f64)>) -> Self {
        raw.sort_by_key(|t| t.0);
        if raw.first().is_none_or(|t| t.0 != 0) {
            raw.insert(0, (0, 120.0));
        }
        let mut tempos: Vec<(f64, f64, f64)> = Vec::new();
        for (tick, bpm) in raw {
            let ms = match tempos.last() {
                Some(&(pt, pms, pbpm)) => pms + (tick as f64 - pt) * 60000.0 / (pbpm * resolution),
                None => 0.0,
            };
            tempos.push((tick as f64, ms, bpm));
        }
        Self { resolution, tempos }
    }

    fn tick_at(&self, ms: f64) -> i64 {
        let ms = ms.max(0.0);
        let &(tick, tms, bpm) = self.tempos.iter().rev().find(|t| t.1 <= ms).unwrap_or(&self.tempos[0]);
        (tick + (ms - tms) * bpm * self.resolution / 60000.0).round() as i64
    }
}

fn chart_tempo_map(text: &str) -> TempoMap {
    let section = |name: &str| -> Vec<&str> {
        let Some(start) = text.find(&format!("[{name}]")) else { return Vec::new() };
        let body = &text[start..];
        let Some(open) = body.find('{') else { return Vec::new() };
        let close = body[open..].find('}').map(|c| open + c).unwrap_or(body.len());
        body[open + 1..close].lines().map(str::trim).filter(|l| !l.is_empty()).collect()
    };
    let resolution = section("Song")
        .iter()
        .find_map(|l| l.strip_prefix("Resolution").and_then(|r| r.trim_start().strip_prefix('=')).map(|v| v.trim()))
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(192.0);
    let tempos = section("SyncTrack")
        .iter()
        .filter_map(|l| {
            let (tick, rest) = l.split_once('=')?;
            let v = rest.trim().strip_prefix("B ")?;
            Some((tick.trim().parse().ok()?, v.trim().parse::<f64>().ok()? / 1000.0))
        })
        .collect();
    TempoMap::new(resolution, tempos)
}

/// Splits lyric lines into word events: (tick, word, length ticks, first-in-line, line end tick).
fn word_events(lines: &[LrcLine], offset_ms: f64, map: &TempoMap) -> Vec<(i64, String, i64, bool, i64)> {
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let start = line.ms + offset_ms;
        let end = lines.get(i + 1).map(|n| n.ms + offset_ms).unwrap_or(start + 3000.0);
        let dur = (end - start).max(200.0);
        let words: Vec<&str> = line.text.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        let per = dur * 0.8 / words.len() as f64;
        let line_end = map.tick_at(start + dur * 0.95);
        for (j, w) in words.iter().enumerate() {
            let ws = start + j as f64 * per;
            let t = map.tick_at(ws);
            let len = (map.tick_at(ws + per * 0.9) - t).max((map.resolution / 4.0) as i64);
            out.push((t, w.to_string(), len, j == 0, line_end));
        }
    }
    out
}

// ---------- .chart ----------

fn is_lyric_line(line: &str) -> bool {
    line.contains("E \"lyric ") || line.contains("E \"phrase_start\"") || line.contains("E \"phrase_end\"")
}

/// Replaces the [Events] lyric/phrase lines of a .chart. `None` lines = remove lyrics.
fn rewrite_chart(text: &str, lines: Option<&[LrcLine]>, offset_ms: f64) -> String {
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut new_events: Vec<(i64, String)> = Vec::new();
    if let Some(lines) = lines {
        let map = chart_tempo_map(text);
        for (tick, word, _, first, line_end) in word_events(lines, offset_ms, &map) {
            if first {
                new_events.push((tick, "E \"phrase_start\"".into()));
                new_events.push((line_end, "E \"phrase_end\"".into()));
            }
            // Quotes would end the event early in .chart files.
            new_events.push((tick, format!("E \"lyric {}\"", word.replace('"', "'"))));
        }
    }
    let to_line = |(tick, ev): &(i64, String)| format!("  {tick} = {ev}");
    match text.find("[Events]") {
        Some(start) => {
            let open = start + text[start..].find('{').unwrap_or(0);
            let close = open + text[open..].find('}').unwrap_or(text.len() - open);
            let mut events: Vec<(i64, String)> = text[open + 1..close]
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !is_lyric_line(l))
                .filter_map(|l| {
                    let (t, ev) = l.split_once(" = ")?;
                    Some((t.trim().parse().ok()?, ev.to_string()))
                })
                .collect();
            events.extend(new_events);
            // Stable: existing events keep their order before new ones at the same tick.
            events.sort_by_key(|e| e.0);
            let body: Vec<String> = events.iter().map(to_line).collect();
            format!("{}{{{nl}{}{nl}{}", &text[..open], body.join(nl), &text[close..])
        }
        None => {
            new_events.sort_by_key(|e| e.0);
            let body: Vec<String> = new_events.iter().map(to_line).collect();
            let section = format!("[Events]{nl}{{{nl}{}{nl}}}{nl}", body.join(nl));
            // After [SyncTrack], or at the end.
            let at = text
                .find("[SyncTrack]")
                .and_then(|s| text[s..].find('}').map(|c| s + c + 1))
                .unwrap_or(text.len());
            let rest = text[at..].trim_start_matches(['\r', '\n']);
            format!("{}{nl}{section}{rest}", &text[..at])
        }
    }
}

// ---------- .mid ----------

fn rewrite_mid(data: &[u8], lines: Option<&[LrcLine]>, offset_ms: f64) -> Result<Vec<u8>, String> {
    use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind, num::u28};
    let smf = Smf::parse(data).map_err(|e| format!("Couldn't read notes.mid: {e}"))?;
    let resolution = match smf.header.timing {
        Timing::Metrical(t) => t.as_int() as f64,
        Timing::Timecode(..) => return Err("SMPTE-timed .mid files aren't supported".into()),
    };
    let mut tempos = Vec::new();
    if let Some(t0) = smf.tracks.first() {
        let mut tick = 0i64;
        for e in t0 {
            tick += e.delta.as_int() as i64;
            if let TrackEventKind::Meta(MetaMessage::Tempo(us)) = e.kind
                && us.as_int() > 0
            {
                tempos.push((tick, 60_000_000.0 / us.as_int() as f64));
            }
        }
    }
    let map = TempoMap::new(resolution, tempos);
    let is_vocals = |t: &[TrackEvent]| {
        t.iter().take_while(|e| e.delta.as_int() == 0).any(|e| {
            matches!(e.kind, TrackEventKind::Meta(MetaMessage::TrackName(n)) if n == b"PART VOCALS")
        })
    };

    let words = lines.map(|l| word_events(l, offset_ms, &map)).unwrap_or_default();
    let texts: Vec<Vec<u8>> = words.iter().map(|w| w.1.as_bytes().to_vec()).collect();
    let mut new_track: Option<Vec<TrackEvent>> = None;
    if lines.is_some() {
        // (tick, order, kind): note-offs before note-ons at the same tick.
        let mut evs: Vec<(i64, u8, TrackEventKind)> = Vec::new();
        let on = |key: u8| TrackEventKind::Midi {
            channel: 0.into(),
            message: MidiMessage::NoteOn { key: key.into(), vel: 100.into() },
        };
        let off = |key: u8| TrackEventKind::Midi {
            channel: 0.into(),
            message: MidiMessage::NoteOff { key: key.into(), vel: 0.into() },
        };
        for (i, (tick, _, len, first, line_end)) in words.iter().enumerate() {
            if *first {
                evs.push((*tick, 1, on(105)));
                evs.push((*line_end, 0, off(105)));
            }
            evs.push((*tick, 2, TrackEventKind::Meta(MetaMessage::Lyric(&texts[i]))));
            evs.push((*tick, 3, on(60)));
            evs.push((tick + len, 0, off(60)));
        }
        evs.sort_by_key(|e| (e.0, e.1));
        let mut track = vec![TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::TrackName(b"PART VOCALS")) }];
        let mut last = 0i64;
        for (tick, _, kind) in evs {
            track.push(TrackEvent { delta: u28::new((tick - last).max(0) as u32), kind });
            last = tick;
        }
        track.push(TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) });
        new_track = Some(track);
    }

    let mut out = Smf::new(smf.header);
    let mut replaced = false;
    for t in &smf.tracks {
        if is_vocals(t) {
            if let Some(nt) = new_track.take() {
                out.tracks.push(nt);
            }
            replaced = true;
            continue; // removing lyrics drops the old PART VOCALS track
        }
        out.tracks.push(t.clone());
    }
    if !replaced && let Some(nt) = new_track {
        out.tracks.push(nt);
    }
    let mut buf = Vec::new();
    out.write_std(&mut buf).map_err(|e| format!("Couldn't write notes.mid: {e}"))?;
    Ok(buf)
}

// ---------- Applying ----------

/// Writes `lines` (shifted by `offset_ms`) into the chart's notes file, after backing it up.
/// `None` removes the lyrics instead.
pub async fn apply(chart_file: PathBuf, lines: Option<Vec<LrcLine>>, offset_ms: f64) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let data = std::fs::read(&chart_file).map_err(|e| format!("Couldn't read the chart: {e}"))?;
        let is_mid = chart_file.extension().is_some_and(|e| e.eq_ignore_ascii_case("mid"));
        let out = if is_mid {
            rewrite_mid(&data, lines.as_deref(), offset_ms)?
        } else {
            let bom = data.starts_with(&[0xEF, 0xBB, 0xBF]);
            let text = crate::library::decode(&data);
            let mut out = if bom { vec![0xEF, 0xBB, 0xBF] } else { Vec::new() };
            out.extend(rewrite_chart(&text, lines.as_deref(), offset_ms).into_bytes());
            out
        };
        // Make sure the result still parses before touching the original.
        crate::chart::parse_chart_file(&out, is_mid, &Default::default())
            .map_err(|e| format!("Not saved: the edited chart wouldn't load ({e})"))?;
        crate::backup::backup(&chart_file)?;
        std::fs::write(&chart_file, out).map_err(|e| format!("Couldn't save the chart: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

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

/// Audio stems of a folder chart, for the timing preview.
pub fn audio_stems(dir: &Path) -> Vec<(PathBuf, String)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            let (stem, ext) = name.rsplit_once('.')?;
            (["ogg", "mp3", "wav", "opus"].contains(&ext) && stem != "preview" && stem != "crowd")
                .then(|| (e.path(), ext.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LRC: &str = "[ar:Someone]\n[00:12.50] Hello there world\n[00:15.00][01:00.123] Again\n\n";

    #[test]
    fn lrc_parsing() {
        let l = parse_lrc(LRC);
        assert_eq!(
            l,
            vec![
                LrcLine { ms: 12500.0, text: "Hello there world".into() },
                LrcLine { ms: 15000.0, text: "Again".into() },
                LrcLine { ms: 60123.0, text: "Again".into() },
            ]
        );
    }

    #[test]
    fn tempo_map_handles_changes() {
        // 120 BPM for 4 beats (2 s), then 60 BPM.
        let m = TempoMap::new(192.0, vec![(0, 120.0), (768, 60.0)]);
        assert_eq!(m.tick_at(1000.0), 384);
        assert_eq!(m.tick_at(3000.0), 768 + 192);
    }

    #[test]
    fn chart_roundtrip() {
        let chart = "[Song]\r\n{\r\n  Resolution = 192\r\n}\r\n[SyncTrack]\r\n{\r\n  0 = TS 4\r\n  0 = B 120000\r\n}\r\n[Events]\r\n{\r\n  0 = E \"section Intro\"\r\n  10 = E \"lyric old\"\r\n}\r\n[ExpertSingle]\r\n{\r\n  0 = N 0 0\r\n}\r\n";
        let lines = parse_lrc(LRC);
        let out = rewrite_chart(chart, Some(&lines), 0.0);
        assert!(!out.contains("lyric old"));
        assert!(out.contains("4800 = E \"phrase_start\"") && out.contains("4800 = E \"lyric Hello\""));
        assert!(out.contains("0 = E \"section Intro\""));
        let p = crate::chart::parse_chart_file(out.as_bytes(), false, &Default::default()).unwrap();
        assert!(p.has_lyrics && p.sections.len() == 1);
        let removed = rewrite_chart(&out, None, 0.0);
        assert!(!removed.contains("lyric") && removed.contains("section Intro"));
        // No [Events] section yet.
        let no_events = chart.replace("[Events]\r\n{\r\n  0 = E \"section Intro\"\r\n  10 = E \"lyric old\"\r\n}\r\n", "");
        let out = rewrite_chart(&no_events, Some(&lines), 0.0);
        assert!(crate::chart::parse_chart_file(out.as_bytes(), false, &Default::default()).unwrap().has_lyrics);
    }

    #[test]
    fn mid_roundtrip() {
        use midly::{Format, Header, MetaMessage, Smf, Timing, TrackEvent, TrackEventKind, num::u28};
        let end = TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) };
        let mut smf = Smf::new(Header::new(Format::Parallel, Timing::Metrical(480.into())));
        smf.tracks.push(vec![
            TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::Tempo(500_000.into())) },
            end,
        ]);
        smf.tracks.push(vec![TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::TrackName(b"PART GUITAR")) }, end]);
        let mut data = Vec::new();
        smf.write_std(&mut data).unwrap();
        let out = rewrite_mid(&data, Some(&parse_lrc(LRC)), 0.0).unwrap();
        assert!(mid_has_lyrics(&out));
        let p = crate::chart::parse_chart_file(&out, true, &Default::default()).unwrap();
        assert!(p.has_vocals && p.has_lyrics);
        let removed = rewrite_mid(&out, None, 0.0).unwrap();
        assert!(!mid_has_lyrics(&removed));
    }
}
