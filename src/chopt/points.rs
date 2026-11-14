// Ported from CHOpt (https://github.com/GenericMadScientist/CHOpt)
// Copyright (C) 2020-2026 Raymond Wright. Rust port for Truss.
// Licensed under the GNU General Public License v3.0 or later.

//! Every scoring event in a track (notes and sustain ticks), with its timing window.

use super::engine::Engine;
use super::song::{ABSENT, Note, NoteTrack, StarPower};
use super::time::{SpPosition, SpTimeMap};

#[derive(Debug, Clone)]
pub struct PathingSettings {
    pub engine: Engine,
    /// 0..1 (CHOpt clamps to at least 0.001).
    pub squeeze: f64,
    pub early_whammy: f64,
    /// Seconds.
    pub lazy_whammy: f64,
    pub video_lag: f64,
    pub whammy_delay: f64,
}

impl PathingSettings {
    pub fn new(engine: Engine) -> Self {
        PathingSettings { engine, squeeze: 1.0, early_whammy: 1.0, lazy_whammy: 0.0, video_lag: 0.0, whammy_delay: 0.0 }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Point {
    pub position: SpPosition,
    pub hit_window_start: SpPosition,
    pub hit_window_end: SpPosition,
    pub max_sqz_hit_window_start: SpPosition,
    pub value: i32,
    pub base_value: i32,
    pub clean_play_bonus: i32,
    pub is_hold_point: bool,
    pub is_sp_granting_note: bool,
    pub is_unison_sp_granting_note: bool,
}

fn phrase_contains(p: &StarPower, pos: i32) -> bool {
    pos >= p.position && pos < p.position + p.length
}

fn song_tick_gap(resolution: i32, engine: &Engine) -> f64 {
    let mut q = resolution as f64 / engine.sust_points_per_beat() as f64;
    if engine.round_tick_gap() {
        q = q.floor();
    }
    q.max(1.0)
}

fn hold_point(position: SpPosition, value: i32) -> Point {
    Point {
        position,
        hit_window_start: position,
        hit_window_end: position,
        max_sqz_hit_window_start: position,
        value,
        base_value: value,
        clean_play_bonus: 0,
        is_hold_point: true,
        is_sp_granting_note: false,
        is_unison_sp_granting_note: false,
    }
}

fn append_sustain_points(out: &mut Vec<Point>, position: i32, sust_length: i32, resolution: i32, time_map: &SpTimeMap, engine: &Engine) {
    let res = resolution as f64;
    let pos = position as f64;
    let tick_gap = song_tick_gap(resolution, engine);
    let float_len = sust_length as f64;
    let mut sust_ticks = (float_len / tick_gap).ceil() as i32; // RoundUp
    let burst_offset = (float_len - engine.burst_size() * res).max(0.0);
    let mut i = 1;
    while (i as f64) * tick_gap < burst_offset && sust_ticks > 0 {
        let beat = (pos + i as f64 * tick_gap) / res;
        out.push(hold_point(time_map.pos(beat), 1));
        i += 1;
        sust_ticks -= 1;
    }
    if sust_ticks > 0 {
        let beat = (pos + burst_offset) / res;
        out.push(hold_point(time_map.pos(beat), sust_ticks));
    }
}

fn min_max_lengths(note: &Note) -> (i32, i32) {
    let ls = note.lengths.iter().copied().filter(|l| *l != ABSENT);
    (ls.clone().min().unwrap_or(0), ls.max().unwrap_or(0))
}

fn clean_bonus_multiplier(note: &Note) -> i32 {
    let (min, max) = min_max_lengths(note);
    if min == max { 1 } else { note.chord_size() }
}

#[allow(clippy::too_many_arguments)]
fn append_note_points(
    idx: usize,
    notes: &[Note],
    out: &mut Vec<Point>,
    time_map: &SpTimeMap,
    resolution: i32,
    is_sp_ender: bool,
    is_unison_sp_ender: bool,
    s: &PathingSettings,
) {
    let note = &notes[idx];
    let engine = &s.engine;
    let note_value = engine.base_note_value();
    let chord_size = note.chord_size();
    let beat = time_map.tick_to_beats(note.position);
    let note_seconds = time_map.beats_to_seconds(beat);
    let early_gap = if idx > 0 { note_seconds - time_map.beats_to_seconds(time_map.tick_to_beats(notes[idx - 1].position)) } else { f64::INFINITY };
    let late_gap =
        if idx + 1 < notes.len() { time_map.beats_to_seconds(time_map.tick_to_beats(notes[idx + 1].position)) - note_seconds } else { f64::INFINITY };
    let early_window = engine.early_timing_window(early_gap, late_gap) * s.squeeze;
    let max_sqz_early_window = engine.early_timing_window(early_gap, late_gap);
    let late_window = engine.late_timing_window(early_gap, late_gap) * s.squeeze;
    let at = |secs: f64| time_map.pos(time_map.seconds_to_beats(secs));
    out.push(Point {
        position: time_map.pos(beat),
        hit_window_start: at(note_seconds - early_window),
        hit_window_end: at(note_seconds + late_window),
        max_sqz_hit_window_start: at(note_seconds - max_sqz_early_window),
        value: note_value * chord_size,
        base_value: note_value * chord_size,
        clean_play_bonus: engine.clean_play_bonus() * clean_bonus_multiplier(note),
        is_hold_point: false,
        is_sp_granting_note: is_sp_ender,
        is_unison_sp_granting_note: is_unison_sp_ender,
    });
    let (min, max) = min_max_lengths(note);
    if min == max {
        append_sustain_points(out, note.position, min, resolution, time_map, engine);
    } else {
        for &l in note.lengths.iter().filter(|l| **l != ABSENT) {
            append_sustain_points(out, note.position, l, resolution, time_map, engine);
        }
    }
}

fn colours_string(note: &Note) -> String {
    const NAMES: [&str; 6] = ["G", "R", "Y", "B", "O", "open"];
    (0..6).filter(|&i| note.lengths[i] != ABSENT).map(|i| NAMES[i]).collect()
}

/// For each index, the first index at or after it matching `pred` (or `len` if none), plus a
/// final entry for `len` itself.
fn next_matching(points: &[Point], pred: impl Fn(&Point) -> bool) -> Vec<usize> {
    let n = points.len();
    let mut out = vec![n; n + 1];
    let mut next = n;
    for i in (0..n).rev() {
        if pred(&points[i]) {
            next = i;
        }
        out[i] = next;
    }
    out
}

pub struct PointSet {
    pub points: Vec<Point>,
    next_non_hold: Vec<usize>,
    next_sp_granting: Vec<usize>,
    pub solo_boosts: Vec<(SpPosition, i32)>,
    cumulative: Vec<i32>,
    pub video_lag: f64,
    colours: Vec<String>,
}

impl PointSet {
    pub fn new(track: &NoteTrack, time_map: &SpTimeMap, unison_phrases: &[StarPower], s: &PathingSettings) -> Self {
        let notes = &track.notes;
        let phrases = track.sp_phrases();
        let mut points: Vec<Point> = Vec::new();
        let mut current = 0;
        for idx in 0..notes.len() {
            let p = &notes[idx];
            let q = notes.get(idx + 1);
            while current < phrases.len() && p.position >= phrases[current].position + phrases[current].length {
                current += 1;
            }
            let (mut ender, mut unison) = (false, false);
            if let Some(ph) = phrases.get(current)
                && phrase_contains(ph, p.position)
                && q.is_none_or(|q| !phrase_contains(ph, q.position))
            {
                ender = true;
                unison = unison_phrases.iter().any(|u| u.position == ph.position && u.length == ph.length);
            }
            append_note_points(idx, notes, &mut points, time_map, track.resolution, ender, unison, s);
        }
        points.sort_by(|a, b| a.position.beat.total_cmp(&b.position.beat)); // stable
        // Multiplier.
        let mut combo = 0;
        for p in &mut points {
            if !p.is_hold_point {
                combo += 1;
            }
            let mult = (combo / 10 + 1).min(s.engine.max_multiplier());
            p.value *= mult;
        }
        // Video lag.
        // Always applied (even with no lag), as the round trip through seconds can move a point
        // by a hair and CHOpt's output depends on that.
        let shift = |pos: &mut SpPosition| {
            let secs = time_map.beats_to_seconds(pos.beat) + s.video_lag;
            *pos = time_map.pos(time_map.seconds_to_beats(secs));
        };
        for p in points.iter_mut().filter(|p| !p.is_hold_point) {
            shift(&mut p.position);
            shift(&mut p.hit_window_start);
            shift(&mut p.hit_window_end);
        }
        let next_non_hold = next_matching(&points, |p| !p.is_hold_point);
        let next_sp_granting = next_matching(&points, |p| p.is_sp_granting_note);
        let solo_boosts = track
            .solos()
            .iter()
            .map(|s| {
                let b = time_map.tick_to_beats(s.end);
                (time_map.pos(b), s.value)
            })
            .collect();
        let mut cumulative = Vec::with_capacity(points.len() + 1);
        let mut sum = 0;
        cumulative.push(0);
        for p in &points {
            sum += p.value;
            cumulative.push(sum);
        }
        let mut note_iter = notes.iter();
        let colours = points
            .iter()
            .map(|p| if p.is_hold_point { String::new() } else { note_iter.next().map(colours_string).unwrap_or_default() })
            .collect();
        PointSet { points, next_non_hold, next_sp_granting, solo_boosts, cumulative, video_lag: s.video_lag, colours }
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }
    pub fn get(&self, i: usize) -> &Point {
        &self.points[i]
    }
    /// With overlapping Star Power (Clone Hero), simply the next point.
    pub fn first_after_current_phrase(&self, i: usize) -> usize {
        i + 1
    }
    pub fn next_non_hold_point(&self, i: usize) -> usize {
        self.next_non_hold[i.min(self.points.len())]
    }
    pub fn next_sp_granting_note(&self, i: usize) -> usize {
        self.next_sp_granting[i.min(self.points.len())]
    }
    pub fn colour_set(&self, i: usize) -> &str {
        &self.colours[i]
    }
    /// Score of points in [start, end).
    pub fn range_score(&self, start: usize, end: usize) -> i32 {
        self.cumulative[end] - self.cumulative[start]
    }
}
