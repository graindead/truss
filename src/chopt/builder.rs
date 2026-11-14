// Ported from CHOpt (https://github.com/GenericMadScientist/CHOpt)
// Copyright (C) 2020-2026 Raymond Wright. Rust port for Truss.
// Licensed under the GNU General Public License v3.0 or later.

//! Everything drawn in a path image: rows of measures, notes, Star Power phrases,
//! activations and running scores.

use super::points::PointSet;
use super::processed::{Path, ProcessedSong};
use super::song::{ABSENT, NoteTrack, Song, StarPower};
use super::sp::SpData;
use super::time::{SpTimeMap, TempoMap};

const MAX_BEATS_PER_LINE: f64 = 16.0;

#[derive(Debug, Clone, Copy)]
pub struct DrawnNote {
    pub beat: f64,
    /// Sustain length in beats per colour; negative when the colour isn't in the note.
    pub lengths: [f64; 7],
    pub flags: u32,
    pub is_sp_note: bool,
}

pub type Range = (f64, f64);

#[derive(Debug, Clone, Default)]
pub struct PathImage {
    pub rows: Vec<Range>,
    pub half_beat_lines: Vec<f64>,
    pub beat_lines: Vec<f64>,
    pub measure_lines: Vec<f64>,
    pub bpms: Vec<(f64, f64)>,
    pub time_sigs: Vec<(f64, i32, i32)>,
    pub notes: Vec<DrawnNote>,
    pub base_values: Vec<i32>,
    pub score_values: Vec<i32>,
    pub sp_percent_values: Vec<f64>,
    pub song_name: String,
    pub artist: String,
    pub charter: String,
    pub difficulty: String,
    pub green_ranges: Vec<Range>,
    pub blue_ranges: Vec<Range>,
    pub red_ranges: Vec<Range>,
    pub yellow_ranges: Vec<Range>,
    pub solo_ranges: Vec<Range>,
    pub bre_ranges: Vec<Range>,
    pub unison_ranges: Vec<Range>,
    pub practice_sections: Vec<(f64, String)>,
    pub activation_opacity: f32,
    pub total_score: i32,
    pub lefty: bool,
}

fn time_sig_at(tm: &TempoMap, beat: f64) -> Option<(i32, i32)> {
    let ts = tm.time_sigs();
    let i = ts.partition_point(|t| tm.tick_to_beats(t.position) <= beat);
    (i > 0).then(|| (ts[i - 1].numerator, ts[i - 1].denominator))
}

fn beat_rate(tm: &TempoMap, beat: f64) -> f64 {
    time_sig_at(tm, beat).map_or(4.0, |(n, d)| 4.0 * n as f64 / d as f64)
}

fn drawn_rows(track: &NoteTrack, tm: &TempoMap) -> Vec<Range> {
    let max_pos = track.notes.iter().map(|n| n.position + n.max_length()).max().unwrap_or(0);
    let max_beat = tm.tick_to_beats(max_pos);
    let mut current = 0.0;
    let mut rows = Vec::new();
    while current <= max_beat {
        let mut row_length = 0.0;
        loop {
            let mut contribution = beat_rate(tm, current + row_length);
            if contribution > MAX_BEATS_PER_LINE && row_length == 0.0 {
                // A measure longer than a row is split up.
                while contribution > MAX_BEATS_PER_LINE {
                    rows.push((current, current + MAX_BEATS_PER_LINE));
                    current += MAX_BEATS_PER_LINE;
                    contribution -= MAX_BEATS_PER_LINE;
                }
            }
            if contribution + row_length > MAX_BEATS_PER_LINE {
                break;
            }
            row_length += contribution;
            if current + row_length > max_beat {
                break;
            }
        }
        rows.push((current, current + row_length));
        current += row_length;
    }
    rows
}

fn subtract_video_lag(beat: f64, video_lag: f64, tm: &TempoMap) -> f64 {
    let s = tm.beats_to_seconds(beat) - video_lag;
    if s < 0.0 { 0.0 } else { tm.seconds_to_beats(s) }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
enum DrainEvent {
    Measure,
    ActStart,
    ActEnd,
    WhammyEnd,
    SpPhrase,
    UnisonPhrase,
}

impl PathImage {
    pub fn new(song: &Song, track: &NoteTrack, difficulty: &str, lefty: bool) -> Self {
        let tm = &song.tempo_map;
        let mut img = PathImage { rows: drawn_rows(track, tm), activation_opacity: 0.33, lefty, ..Default::default() };
        img.notes = track
            .notes
            .iter()
            .map(|n| {
                let mut lengths = [-1.0; 7];
                for i in 0..7 {
                    if n.lengths[i] != ABSENT {
                        lengths[i] = tm.tick_to_beats(n.lengths[i]);
                    }
                }
                DrawnNote { beat: tm.tick_to_beats(n.position), lengths, flags: n.flags, is_sp_note: track.is_sp_note(n.position) }
            })
            .collect();
        // Beat lines.
        for &(start, end) in &img.rows.clone() {
            let mut s = start;
            while s < end {
                let len = beat_rate(tm, s);
                let (numer, denom) = time_sig_at(tm, s).map_or((4, 1.0), |(n, d)| (n, 4.0 / d as f64));
                img.measure_lines.push(s);
                img.half_beat_lines.push(s + 0.5 * denom);
                for i in 1..numer {
                    img.beat_lines.push(s + i as f64 * denom);
                    img.half_beat_lines.push(s + (i as f64 + 0.5) * denom);
                }
                s += len;
            }
        }
        if let Some(last) = img.rows.last() {
            img.measure_lines.push(last.1);
        }
        img.song_name = crate::api::strip_tags(&song.name);
        img.artist = crate::api::strip_tags(&song.artist);
        img.charter = crate::api::strip_tags(&song.charter);
        img.difficulty = difficulty.to_string();
        let row_end = img.rows.last().map_or(0.0, |r| r.1);
        let last_note = img.notes.iter().map(|n| n.beat).fold(f64::NEG_INFINITY, f64::max);
        img.practice_sections = song
            .practice_sections
            .iter()
            .map(|s| (tm.tick_to_beats(s.start), s.name.clone()))
            .filter(|(b, _)| *b <= last_note)
            .collect();
        img.bpms = tm.bpms().iter().map(|b| (tm.tick_to_beats(b.position), b.bpm())).filter(|(p, _)| *p < row_end).collect();
        img.time_sigs = tm
            .time_sigs()
            .iter()
            .map(|t| (tm.tick_to_beats(t.position), t.numerator, t.denominator))
            .filter(|(p, _, _)| *p < row_end)
            .collect();
        img.solo_ranges = track.solos().iter().map(|s| (tm.tick_to_beats(s.start), tm.tick_to_beats(s.end))).collect();
        img.bre_ranges = track.bres.iter().map(|b| (tm.tick_to_beats(b.start), tm.tick_to_beats(b.end))).collect();
        img
    }

    fn sp_phrase_bounds(&self, phrase: &StarPower, track: &NoteTrack, tm: &TempoMap) -> Option<Range> {
        let notes = &track.notes;
        let mut i = notes.partition_point(|n| n.position < phrase.position);
        let phrase_end = phrase.position + phrase.length;
        if i >= notes.len() || notes[i].position >= phrase_end {
            return None;
        }
        let start = tm.tick_to_beats(notes[i].position);
        let (mut latest_note, mut latest_sustain_end) = (0, 0);
        while i < notes.len() && notes[i].position < phrase_end {
            latest_note = latest_note.max(notes[i].position);
            for &l in &notes[i].lengths {
                if l > 0 {
                    latest_sustain_end = latest_sustain_end.max(notes[i].position + l);
                }
            }
            i += 1;
        }
        let end = (start + 0.1).max(tm.tick_to_beats(latest_note)).max(tm.tick_to_beats(latest_sustain_end));
        Some((start, end))
    }

    pub fn add_sp_phrases(&mut self, track: &NoteTrack, tm: &TempoMap, unison: &[StarPower]) {
        for phrase in track.sp_phrases() {
            let Some(range) = self.sp_phrase_bounds(phrase, track, tm) else { continue };
            self.green_ranges.push(range);
            if unison.iter().any(|u| u.position == phrase.position && u.length == phrase.length) {
                self.unison_ranges.push(range);
            }
        }
    }

    pub fn add_sp_acts(&mut self, points: &PointSet, tm: &TempoMap, path: &Path) {
        const RED_EPSILON: f64 = 1.0 / 60.0;
        let pts = &points.points;
        let shifted = |b: f64| subtract_video_lag(b, points.video_lag, tm);
        let row_end = self.rows.last().map_or(0.0, |r| r.1);
        let mut no_whammy: Vec<Range> = Vec::new();
        for act in &path.activations {
            let mut blue_start = act.sp_start;
            let mut blue_end = act.sp_end;
            if act.act_start != 0 {
                blue_start = blue_start.max(pts[act.act_start - 1].position.beat);
            }
            if act.act_end + 1 != pts.len() {
                blue_end = blue_end.min(pts[act.act_end + 1].position.beat);
            }
            blue_end = shifted(blue_end).min(row_end);
            self.blue_ranges.push((shifted(blue_start), blue_end));
            let start_beat = pts[act.act_start].position.beat;
            let end_beat = pts[act.act_end].position.beat;
            if act.sp_start - RED_EPSILON > start_beat {
                self.red_ranges.push((shifted(start_beat), shifted(act.sp_start - RED_EPSILON)));
            }
            if act.sp_end + RED_EPSILON < end_beat {
                self.red_ranges.push((shifted(act.sp_end + RED_EPSILON), shifted(end_beat)));
            }
            no_whammy.push((act.whammy_end, act.sp_end));
        }
        let (mut p, mut q) = (0, 0);
        while p < no_whammy.len() && q < self.green_ranges.len() {
            let start = no_whammy[p].0.max(self.green_ranges[q].0);
            let end = no_whammy[p].1.min(self.green_ranges[q].1);
            if start < end {
                self.yellow_ranges.push((start, end));
            }
            if self.green_ranges[q].1 > no_whammy[p].1 {
                p += 1;
            } else {
                q += 1;
            }
        }
    }

    pub fn add_measure_values(&mut self, points: &PointSet, tm: &TempoMap, path: &Path) {
        const MEAS_EPSILON: f64 = 1.0 / 100000.0;
        let m = &self.measure_lines;
        let count = m.len().saturating_sub(1);
        self.base_values = vec![0; count];
        self.score_values = vec![0; count];
        if count == 0 {
            return;
        }
        let pts = &points.points;
        let mut mi = 1; // index into measure_lines; value index is mi - 1
        for p in pts {
            let adj = subtract_video_lag(p.position.beat, points.video_lag, tm);
            while mi < m.len() && mi + 1 < m.len() && m[mi] - MEAS_EPSILON <= adj {
                mi += 1;
            }
            self.base_values[mi - 1] += p.base_value;
            self.score_values[mi - 1] += p.value + p.clean_play_bonus;
        }
        let mut mi = 1;
        for (end, value) in &points.solo_boosts {
            while mi < m.len() && m[mi] <= end.beat {
                mi += 1;
            }
            let idx = (mi - 1).min(count - 1);
            self.score_values[idx] += value;
        }
        for act in &path.activations {
            let mut mi = 1;
            for p in &pts[act.act_start..=act.act_end] {
                let adj = subtract_video_lag(p.position.beat, points.video_lag, tm);
                while mi < m.len() && m[mi] <= adj {
                    mi += 1;
                }
                let idx = (mi - 1).min(count - 1);
                self.score_values[idx] += p.value;
            }
        }
        let mut total = 0;
        for s in &mut self.score_values {
            total += *s;
            *s = total;
        }
    }

    pub fn add_sp_percent_values(&mut self, sp_data: &SpData, tm: &SpTimeMap, points: &PointSet, path: &Path, song: &ProcessedSong) {
        let values = song.values;
        let mut events: Vec<(f64, DrainEvent)> = Vec::new();
        for &m in self.measure_lines.iter().skip(1) {
            events.push((m, DrainEvent::Measure));
        }
        for (i, p) in points.points.iter().enumerate() {
            if !p.is_sp_granting_note {
                continue;
            }
            let mut pos = p.position.beat;
            for act in &path.activations {
                if i > act.act_end {
                    pos = pos.max(act.sp_end);
                }
            }
            events.push((pos, if p.is_unison_sp_granting_note { DrainEvent::UnisonPhrase } else { DrainEvent::SpPhrase }));
        }
        for act in &path.activations {
            if act.whammy_end < act.sp_end {
                events.push((act.whammy_end, DrainEvent::WhammyEnd));
            }
            events.push((act.sp_start, DrainEvent::ActStart));
            events.push((act.sp_end, DrainEvent::ActEnd));
        }
        events.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut active = false;
        let mut whammy_end = f64::INFINITY;
        let mut position = 0.0;
        let mut total = 0.0;
        self.sp_percent_values.clear();
        for (pos, ev) in events {
            if active {
                total = sp_data.propagate_sp_over_whammy_min(tm.pos(position), tm.pos(pos), total, tm.pos(whammy_end));
            } else if whammy_end > position {
                total += sp_data.available_whammy_all(position, pos.min(whammy_end));
            }
            match ev {
                DrainEvent::ActStart => active = true,
                DrainEvent::ActEnd => {
                    active = false;
                    whammy_end = f64::INFINITY;
                    total = 0.0;
                }
                DrainEvent::SpPhrase => total += values.phrase_amount,
                DrainEvent::UnisonPhrase => total += values.unison_phrase_amount,
                DrainEvent::Measure => self.sp_percent_values.push(total.clamp(0.0, 1.0)),
                DrainEvent::WhammyEnd => whammy_end = pos,
            }
            position = pos;
            total = total.clamp(0.0, 1.0);
        }
    }

    pub fn set_total_score(&mut self, song: &ProcessedSong, path: &Path) {
        self.total_score = song.no_sp_score() + path.score_boost;
    }
}
