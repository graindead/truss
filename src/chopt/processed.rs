// Ported from CHOpt (https://github.com/GenericMadScientist/CHOpt)
// Copyright (C) 2020-2026 Raymond Wright. Rust port for Truss.
// Licensed under the GNU General Public License v3.0 or later.

//! A track prepared for optimising: which activations are possible and where they end.

use super::engine::SpEngineValues;
use super::points::{PathingSettings, PointSet};
use super::song::{NoteTrack, StarPower};
use super::sp::{SpBar, SpData};
use super::time::{SpPosition, SpTimeMap};

const MEASURES_PER_BAR: f64 = 8.0;

#[derive(Debug, Clone, Copy)]
pub struct ActivationCandidate {
    pub act_start: usize,
    pub act_end: usize,
    pub earliest_activation_point: SpPosition,
    pub sp_bar: SpBar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProtoActivation {
    pub act_start: usize,
    pub act_end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Activation {
    pub act_start: usize,
    pub act_end: usize,
    pub whammy_end: f64,
    pub sp_start: f64,
    pub sp_end: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActValidity {
    Success,
    InsufficientSp,
    SurplusSp,
}

#[derive(Debug, Clone, Copy)]
pub struct ActResult {
    pub ending_position: SpPosition,
    pub validity: ActValidity,
}

#[derive(Debug, Clone, Default)]
pub struct Path {
    pub activations: Vec<Activation>,
    pub score_boost: i32,
}

struct SpStatus {
    position: SpPosition,
    sp: f64,
    values: SpEngineValues,
    last_burst_position: f64,
}

impl SpStatus {
    fn new(position: SpPosition, sp: f64, values: SpEngineValues) -> Self {
        SpStatus { position, sp, values, last_burst_position: f64::NEG_INFINITY }
    }
    fn add_phrase(&mut self) {
        self.sp = (self.sp + self.values.phrase_amount).min(1.0);
    }
    fn add_unison_phrase(&mut self) {
        self.sp = (self.sp + self.values.unison_phrase_amount).min(1.0);
    }
    fn advance_whammy_max(&mut self, end: SpPosition, sp_data: &SpData) {
        self.sp = sp_data.propagate_sp_over_whammy_max(self.position, end, self.sp, self.last_burst_position);
        self.position = end;
        // A burst exactly at `end` isn't added, so keep it available.
        self.last_burst_position = next_down(end.beat);
    }
    fn update_early_end(&mut self, sp_note_start: SpPosition, sp_data: &SpData, required_whammy_end: SpPosition) {
        self.sp = sp_data.propagate_sp_over_whammy_min(self.position, sp_note_start, self.sp, required_whammy_end);
        if sp_note_start.beat > self.position.beat {
            self.position = sp_note_start;
        }
    }
    fn update_late_end(&mut self, mut sp_note_start: SpPosition, sp_note_end: SpPosition, sp_data: &SpData) {
        if sp_note_start.beat < self.position.beat {
            sp_note_start = self.position;
        }
        self.advance_whammy_max(sp_note_start, sp_data);
        if self.sp < 0.0 {
            return;
        }
        let new_sp = sp_data.propagate_sp_over_whammy_max(sp_note_start, sp_note_end, self.sp, self.last_burst_position);
        if new_sp >= 0.0 {
            self.sp = new_sp;
            self.position = sp_note_end;
        }
    }
}

/// The largest double below `x` (C's nextafter towards -inf).
pub fn next_down(x: f64) -> f64 {
    if x.is_nan() || x == f64::NEG_INFINITY {
        return x;
    }
    if x == 0.0 {
        return -f64::from_bits(1);
    }
    let bits = x.to_bits();
    f64::from_bits(if x > 0.0 { bits - 1 } else { bits + 1 })
}

pub struct ProcessedSong {
    pub time_map: SpTimeMap,
    pub points: PointSet,
    pub sp_data: SpData,
    pub values: SpEngineValues,
    total_clean_play_boost: i32,
    total_solo_boost: i32,
    base_score: i32,
    /// Star Power from phrases before each point (exact: the amounts are 1/4 and 1/2).
    phrase_sp_before: Vec<f64>,
}

impl ProcessedSong {
    pub fn new(track: &NoteTrack, time_map: &SpTimeMap, unison_phrases: &[StarPower], s: &PathingSettings) -> Self {
        let points = PointSet::new(track, time_map, unison_phrases, s);
        let sp_data = SpData::new(track, time_map, s);
        let total_clean_play_boost = points.points.iter().map(|p| p.clean_play_bonus).sum();
        let total_solo_boost = track.solos().iter().map(|s| s.value).sum();
        let values = s.engine.sp_engine_values();
        let mut phrase_sp_before = Vec::with_capacity(points.len() + 1);
        let mut total = 0.0;
        phrase_sp_before.push(0.0);
        for p in &points.points {
            if p.is_sp_granting_note {
                total += if p.is_unison_sp_granting_note { values.unison_phrase_amount } else { values.phrase_amount };
            }
            phrase_sp_before.push(total);
        }
        ProcessedSong {
            time_map: time_map.clone(),
            points,
            sp_data,
            values: s.engine.sp_engine_values(),
            total_clean_play_boost,
            total_solo_boost,
            base_score: track.base_score(),
            phrase_sp_before,
        }
    }

    /// SP from phrases in [begin, end). Same as adding each phrase in turn capped at a full
    /// bar, as every amount is positive.
    fn sp_from_phrases(&self, begin: usize, end: usize) -> SpBar {
        let n = self.points.len();
        let (b, e) = (begin.min(n), end.min(n));
        let sp = if e > b { (self.phrase_sp_before[e] - self.phrase_sp_before[b]).min(1.0) } else { 0.0 };
        SpBar::new(sp, sp, self.values)
    }

    /// The SP available between `start` and `act_start` (not counting `act_start`), with all
    /// whammy before `required_whammy_end` mandatory.
    pub fn total_available_sp(&self, start: f64, first_point: usize, act_start: usize, required_whammy_end: f64) -> SpBar {
        let mut bar = self.sp_from_phrases(first_point, act_start);
        let act_beat = self.points.get(act_start).position.beat;
        if start >= required_whammy_end {
            bar.max = (bar.max + self.sp_data.available_whammy_all(start, act_beat)).min(1.0);
        } else if required_whammy_end >= act_beat {
            bar.min = (bar.min + self.sp_data.available_whammy_all(start, act_beat)).min(1.0);
            bar.max = bar.min;
        } else {
            bar.min = (bar.min + self.sp_data.available_whammy_all(start, required_whammy_end)).min(1.0);
            bar.max = bar.min;
            bar.max = (bar.max + self.sp_data.available_whammy_all(required_whammy_end, act_beat)).min(1.0);
        }
        bar
    }

    /// Like `total_available_sp`, but if half a bar is reachable, also the earliest position
    /// at or after `earliest_potential_pos` where it is.
    pub fn total_available_sp_with_earliest_pos(
        &self,
        start: f64,
        first_point: usize,
        act_start: usize,
        earliest_potential_pos: SpPosition,
    ) -> (SpBar, SpPosition) {
        const EPS: f64 = 0.0001;
        let act_beat = self.points.get(act_start).position.beat;
        let note_tick = self.time_map.beats_to_ticks(act_beat);
        let mut bar = self.sp_from_phrases(first_point, act_start);
        bar.max = (bar.max + self.sp_data.available_whammy(start, earliest_potential_pos.beat, note_tick)).min(1.0);
        if bar.full_enough_to_activate() {
            return (bar, earliest_potential_pos);
        }
        let extra = self.values.minimum_to_activate - bar.max;
        let (mut first, mut last) = (earliest_potential_pos.beat, act_beat);
        if self.sp_data.available_whammy(first, last, note_tick) < extra {
            return (bar, earliest_potential_pos);
        }
        while last - first > EPS {
            let mid = (first + last) * 0.5;
            if self.sp_data.available_whammy(earliest_potential_pos.beat, mid, note_tick) < extra {
                first = mid;
            } else {
                last = mid;
            }
        }
        bar.max = (bar.max + self.sp_data.available_whammy(earliest_potential_pos.beat, last, note_tick)).min(1.0);
        (bar, self.time_map.pos(last))
    }

    pub fn adjusted_hit_window_start(&self, point: usize, squeeze: f64) -> SpPosition {
        let p = self.points.get(point);
        if squeeze == 1.0 {
            return p.hit_window_start;
        }
        let start = self.time_map.beats_to_seconds(p.hit_window_start.beat);
        let mid = self.time_map.beats_to_seconds(p.position.beat);
        self.time_map.pos(self.time_map.seconds_to_beats(start + (mid - start) * (1.0 - squeeze)))
    }

    pub fn adjusted_hit_window_end(&self, point: usize, squeeze: f64) -> SpPosition {
        let p = self.points.get(point);
        if squeeze == 1.0 {
            return p.hit_window_end;
        }
        let mid = self.time_map.beats_to_seconds(p.position.beat);
        let end = self.time_map.beats_to_seconds(p.hit_window_end.beat);
        self.time_map.pos(self.time_map.seconds_to_beats(mid + (end - mid) * squeeze))
    }

    pub fn is_candidate_valid(&self, act: &ActivationCandidate, squeeze: f64, required_whammy_end: SpPosition) -> ActResult {
        let fail = |v| ActResult { ending_position: SpPosition::ZERO, validity: v };
        if !act.sp_bar.full_enough_to_activate() {
            return fail(ActValidity::InsufficientSp);
        }
        let mut ending_pos = self.adjusted_hit_window_start(act.act_end, squeeze);
        if ending_pos.beat < act.earliest_activation_point.beat {
            ending_pos = act.earliest_activation_point;
        }
        let mut late_end_position = self.adjusted_hit_window_end(act.act_start, squeeze);
        if late_end_position.beat > ending_pos.beat {
            late_end_position = ending_pos;
        }
        let start_beat = self.points.get(act.act_start).position.beat;
        let late_end_sp = (act.sp_bar.max + self.sp_data.available_whammy_all(act.earliest_activation_point.beat, start_beat)).min(1.0);
        let mut early = SpStatus::new(act.earliest_activation_point, act.sp_bar.min.max(self.values.minimum_to_activate), self.values);
        let mut late = SpStatus::new(late_end_position, late_end_sp, self.values);
        let mut p = self.points.next_sp_granting_note(act.act_start);
        while p < act.act_end {
            let mut p_start = self.adjusted_hit_window_start(p, squeeze);
            if p_start.beat < act.earliest_activation_point.beat {
                p_start = act.earliest_activation_point;
            }
            let mut p_end = self.adjusted_hit_window_end(p, squeeze);
            if p_end.beat > ending_pos.beat {
                p_end = ending_pos;
            }
            late.update_late_end(p_start, p_end, &self.sp_data);
            if late.sp < 0.0 {
                return fail(ActValidity::InsufficientSp);
            }
            early.update_early_end(p_start, &self.sp_data, required_whammy_end);
            if self.points.get(p).is_unison_sp_granting_note {
                early.add_unison_phrase();
                late.add_unison_phrase();
            } else {
                early.add_phrase();
                late.add_phrase();
            }
            p = self.points.next_sp_granting_note(p + 1);
        }
        late.advance_whammy_max(ending_pos, &self.sp_data);
        if late.sp < 0.0 {
            return fail(ActValidity::InsufficientSp);
        }
        early.update_early_end(ending_pos, &self.sp_data, required_whammy_end);
        let end_point = self.points.get(act.act_end);
        if end_point.is_sp_granting_note {
            if end_point.is_unison_sp_granting_note {
                early.add_unison_phrase();
            } else {
                early.add_phrase();
            }
        }
        let end_meas = early.position.sp_measure + early.sp * MEASURES_PER_BAR;
        let next = act.act_end + 1;
        if next < self.points.len() && end_meas >= self.adjusted_hit_window_end(next, squeeze).sp_measure {
            return fail(ActValidity::SurplusSp);
        }
        ActResult { ending_position: self.time_map.pos_from_measure(end_meas), validity: ActValidity::Success }
    }

    // ---------- Summary text ----------

    fn act_summaries(&self, path: &Path) -> Vec<String> {
        let pts = &self.points.points;
        let mut out = Vec::new();
        let mut start = 0;
        for act in &path.activations {
            let sp_before = pts[start..act.act_start].iter().filter(|p| p.is_sp_granting_note).count();
            let mut s = sp_before.to_string();
            let overlapped = pts[act.act_start..=act.act_end].iter().filter(|p| p.is_sp_granting_note).count();
            if overlapped > 0 {
                s += &format!("(+{overlapped})");
            }
            out.push(s);
            start = act.act_end + 1;
        }
        let spare = pts[start.min(pts.len())..].iter().filter(|p| p.is_sp_granting_note).count();
        if spare != 0 {
            out.push(format!("ES{spare}"));
        }
        out
    }

    fn activation_line(&self, act: &Activation, summary: &str) -> String {
        let pts = &self.points.points;
        let label = summary.split('-').next().unwrap_or(summary);
        let mut s = format!("{label}: ");
        if summary.starts_with('0') {
            s += "See image";
            return s;
        }
        let act_start = act.act_start;
        let mut prev_sp = act_start.saturating_sub(1);
        while prev_sp > 0 && !pts[prev_sp].is_sp_granting_note {
            prev_sp -= 1;
        }
        let count = pts[prev_sp + 1..=act_start].iter().filter(|p| !p.is_hold_point).count();
        if pts[act_start].is_hold_point {
            let mut starting = act_start;
            while starting > 0 && pts[starting].is_hold_point {
                starting -= 1;
            }
            let gap = pts[act_start].position.beat - pts[starting].position.beat;
            if count > 0 {
                s += &format!("{gap:.2} beats after ");
            } else {
                s += &format!("After {gap:.2} beats");
            }
        }
        if count > 1 {
            let mut prev_note = act_start;
            while prev_note > 0 && pts[prev_note].is_hold_point {
                prev_note -= 1;
            }
            let colour = self.points.colour_set(prev_note);
            let same = 1 + (prev_sp + 1..prev_note).filter(|&i| !pts[i].is_hold_point && self.points.colour_set(i) == colour).count();
            s += &format!("{} {}", ordinal(same), colour);
        } else if count == 1 {
            s += "NN";
        }
        if !pts[act.act_end].is_hold_point {
            s += &format!(" ({})", self.points.colour_set(act.act_end));
        }
        s
    }

    pub fn no_sp_score(&self) -> i32 {
        self.points.points.iter().map(|p| p.value).sum::<i32>() + self.total_clean_play_boost + self.total_solo_boost
    }

    /// The text CHOpt prints for a path.
    pub fn path_summary(&self, path: &Path) -> String {
        let summaries = self.act_summaries(path);
        let mut s = String::from("Path: ");
        if summaries.is_empty() {
            s += "None";
        } else {
            s += &summaries.join("-");
        }
        let no_sp = self.no_sp_score();
        s += &format!("\nNo SP score: {no_sp}");
        let total = no_sp + path.score_boost;
        s += &format!("\nTotal score: {total}");
        let mut avg = 0.0;
        if self.base_score != 0 {
            let mut int_avg: i64 = 0;
            let mut stars = (total - self.total_solo_boost - self.total_clean_play_boost) as i64;
            let base = self.base_score as i64;
            for _ in 0..4 {
                int_avg *= 10;
                int_avg += stars / base;
                stars %= base;
                stars *= 10;
            }
            avg = int_avg as f64 / 1000.0;
        }
        s += &format!("\nAverage multiplier: {avg:.3}x");
        for (i, act) in path.activations.iter().enumerate() {
            s += "\n";
            s += &self.activation_line(act, &summaries[i]);
        }
        s
    }
}

pub fn ordinal(n: usize) -> String {
    let suffix = if (11..=13).contains(&(n % 100)) {
        "th"
    } else {
        match n % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    };
    format!("{n}{suffix}")
}
