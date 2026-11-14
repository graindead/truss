// Ported from CHOpt (https://github.com/GenericMadScientist/CHOpt)
// Copyright (C) 2020-2026 Raymond Wright. Rust port for Truss.
// Licensed under the GNU General Public License v3.0 or later.

//! Star Power drain and whammy gain.

use super::engine::SpEngineValues;
use super::points::PathingSettings;
use super::song::{Note, NoteTrack};
use super::time::{SpPosition, SpTimeMap};
use std::collections::BTreeSet;

const MEASURES_PER_BAR: f64 = 8.0;

/// The least and most Star Power possible at some time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpBar {
    pub min: f64,
    pub max: f64,
    pub values: SpEngineValues,
}

impl SpBar {
    pub fn new(min: f64, max: f64, values: SpEngineValues) -> Self {
        SpBar { min, max, values }
    }
    pub fn full_enough_to_activate(&self) -> bool {
        self.max >= self.values.minimum_to_activate
    }
    pub fn minimum_sufficient_to_activate(&self) -> bool {
        self.min >= self.values.minimum_to_activate
    }
}

#[derive(Debug, Clone, Copy)]
struct SpSustain {
    note_position: i32,
    whammy_start: SpPosition,
    whammy_end: SpPosition,
    burst_position: SpPosition,
    releasable_for_burst: bool,
}

#[derive(Debug, Clone, Copy)]
struct BeatRate {
    position: f64,
    net_sp_gain_rate: f64,
}

pub struct SpData {
    beat_rates: Vec<BeatRate>,
    sustains: Vec<SpSustain>,
    last_whammy_point: f64,
    initial_guesses: Vec<usize>,
    sp_gain_rate: f64,
    default_net_sp_gain_rate: f64,
}

fn is_note_part_of_phrase(track: &NoteTrack, note: &Note) -> bool {
    track.is_sp_note(note.position)
}

fn note_end(n: &Note) -> i32 {
    n.position + n.max_length()
}

/// Groups of overlapping sustains, to know which note ends each group.
struct ExtendedSustains(Vec<(i32, i32, i32)>);

impl ExtendedSustains {
    fn new(notes: &[Note]) -> Self {
        let mut v: Vec<(i32, i32, i32)> = Vec::new(); // (start, end, terminating note)
        let mut cur: Option<(i32, i32, i32)> = None;
        for n in notes {
            if cur.is_none_or(|c| n.position >= c.1) {
                if let Some(c) = cur {
                    v.push(c);
                }
                cur = Some((n.position, note_end(n), n.position));
            }
            let e = note_end(n);
            if let Some(c) = cur.as_mut()
                && e > c.1
            {
                c.1 = e;
                c.2 = n.position;
            }
        }
        if let Some(c) = cur {
            v.push(c);
        }
        ExtendedSustains(v)
    }
    fn is_ender(&self, n: &Note) -> bool {
        let mut i = self.0.partition_point(|s| s.0 < n.position);
        if i != 0 && (i == self.0.len() || self.0[i].0 > n.position) {
            i -= 1;
        }
        self.0.get(i).is_some_and(|s| s.2 == n.position)
    }
}

fn round_back_to_resolution(beat: f64, resolution: i32) -> f64 {
    (beat * resolution as f64).floor() / resolution as f64
}

fn whammy_spans(track: &NoteTrack, s: &PathingSettings, tm: &SpTimeMap) -> Vec<SpSustain> {
    let engine = &s.engine;
    let ext = ExtendedSustains::new(&track.notes);
    let notes = &track.notes;
    let mut spans = Vec::new();
    for (i, note) in notes.iter().enumerate() {
        if !is_note_part_of_phrase(track, note) {
            continue;
        }
        let t = tm.beats_to_seconds(tm.tick_to_beats(note.position));
        let early_gap = if i > 0 { t - tm.beats_to_seconds(tm.tick_to_beats(notes[i - 1].position)) } else { f64::INFINITY };
        let late_gap = if i + 1 < notes.len() { tm.beats_to_seconds(tm.tick_to_beats(notes[i + 1].position)) - t } else { f64::INFINITY };
        let lengths: BTreeSet<i32> = note.lengths.iter().copied().filter(|l| *l > 0).collect();
        let longest = lengths.iter().next_back().copied();
        for &length in lengths.iter().rev() {
            let early_window = if engine.has_early_whammy() { engine.early_timing_window(early_gap, late_gap) * s.early_whammy } else { 0.0 };
            let mut start_beat = tm.seconds_to_beats(t - early_window);
            if engine.resolution_rounded_whammy() {
                start_beat = round_back_to_resolution(start_beat, track.resolution);
            }
            let whammy_start = tm.pos(start_beat);
            let whammy_end = tm.pos(tm.tick_to_beats(note.position + length));
            let mut burst = whammy_end;
            if engine.has_whammy_bursts() {
                burst.beat = (burst.beat - engine.burst_size()).max(whammy_start.beat);
                burst.sp_measure = tm.beats_to_sp_measures(burst.beat);
            }
            spans.push(SpSustain {
                note_position: note.position,
                whammy_start,
                whammy_end,
                burst_position: burst,
                releasable_for_burst: Some(length) == longest && ext.is_ender(note),
            });
        }
    }
    spans
}

impl SpData {
    pub fn new(track: &NoteTrack, time_map: &SpTimeMap, s: &PathingSettings) -> Self {
        let engine = &s.engine;
        let gain = engine.sp_gain_rate();
        let tm = &time_map.tempo_map;
        let beat_rates = tm
            .time_sigs()
            .iter()
            .map(|ts| {
                let measure_rate = ts.numerator as f64 * 4.0 / ts.denominator as f64;
                BeatRate { position: tm.tick_to_beats(ts.position), net_sp_gain_rate: gain - 1.0 / (MEASURES_PER_BAR * measure_rate) }
            })
            .collect();
        let mut sustains = whammy_spans(track, s, time_map);
        for sus in &mut sustains {
            let secs = time_map.beats_to_seconds(sus.whammy_start.beat) + s.lazy_whammy + s.video_lag;
            sus.whammy_start = time_map.pos(time_map.seconds_to_beats(secs));
        }
        sustains.retain(|s| s.whammy_start.beat < s.whammy_end.beat);
        let mut d = SpData {
            beat_rates,
            sustains,
            last_whammy_point: f64::NEG_INFINITY,
            initial_guesses: Vec::new(),
            sp_gain_rate: gain,
            default_net_sp_gain_rate: gain - 1.0 / 32.0,
        };
        if d.sustains.is_empty() {
            return d;
        }
        d.last_whammy_point = d.sustains.iter().map(|s| s.whammy_end.beat).fold(f64::NEG_INFINITY, f64::max);
        let mut p = 0;
        let mut pos = 0i64;
        while (pos as f64) < d.last_whammy_point {
            while p < d.sustains.len() && d.sustains[p].whammy_end.beat <= pos as f64 {
                p += 1;
            }
            d.initial_guesses.push(p);
            pos += 1;
        }
        d
    }

    fn first_sp_sustain_after(&self, pos: f64) -> usize {
        if self.last_whammy_point <= pos {
            return self.sustains.len();
        }
        let begin = if pos < 0.0 { 0 } else { self.initial_guesses.get(pos as usize).copied().unwrap_or(0) };
        let mut i = begin;
        while i < self.sustains.len() && self.sustains[i].whammy_end.beat <= pos {
            i += 1;
        }
        i
    }

    /// (beat-rate index, position, sp)
    fn initial_prop_state(&self, mut start: f64, end: f64, mut sp: f64) -> (usize, f64, f64) {
        let mut p = self.beat_rates.partition_point(|r| r.position < start);
        if p != 0 {
            p -= 1;
        } else {
            let sub_end = end.min(self.beat_rates.first().map_or(end, |r| r.position));
            sp += (sub_end - start) * self.default_net_sp_gain_rate;
            sp = sp.min(1.0);
            start = sub_end;
        }
        (p, start, sp)
    }

    fn add_reserved_burst(&self, mut sp: f64, reserved: &mut f64) -> f64 {
        if *reserved > 0.0 {
            sp += *reserved * self.sp_gain_rate;
            sp = sp.min(1.0);
        }
        *reserved = 0.0;
        sp
    }

    /// The most SP left at `end` with whammy, or a negative number if it runs out.
    pub fn propagate_sp_over_whammy_max(&self, mut start: SpPosition, end: SpPosition, mut sp: f64, mut last_burst: f64) -> f64 {
        let mut reserved = 0.0;
        let mut end_of_reserved = SpPosition::ZERO;
        let mut p = self.first_sp_sustain_after(start.beat);
        while p < self.sustains.len() && self.sustains[p].whammy_start.beat < end.beat {
            let s = self.sustains[p];
            p += 1;
            if s.burst_position.beat <= last_burst {
                continue;
            }
            if s.whammy_start.beat > start.beat {
                sp -= (s.whammy_start.sp_measure - start.sp_measure) / MEASURES_PER_BAR;
                if sp < 0.0 {
                    sp = self.add_reserved_burst(sp, &mut reserved);
                    if sp < 0.0 {
                        return sp;
                    }
                }
                start = s.whammy_start;
            }
            let sustain_end = if s.releasable_for_burst { s.burst_position } else { s.whammy_end };
            let range_end = end.beat.min(sustain_end.beat);
            sp = self.add_reserved_burst(sp, &mut reserved);
            if start.beat < range_end {
                sp = self.propagate_over_whammy_range(start.beat, range_end, sp);
            }
            if sp < 0.0 || sustain_end.beat >= end.beat {
                return sp;
            }
            start = sustain_end;
            reserved = s.whammy_end.beat - sustain_end.beat;
            end_of_reserved = s.whammy_end;
            last_burst = s.burst_position.beat;
        }
        if reserved > 0.0 && end_of_reserved.sp_measure > start.sp_measure {
            sp -= (end_of_reserved.sp_measure - start.sp_measure) / MEASURES_PER_BAR;
            sp = self.add_reserved_burst(sp, &mut reserved);
            start = end_of_reserved;
        }
        sp - (end.sp_measure - start.sp_measure) / MEASURES_PER_BAR
    }

    /// The least SP left at `end`, with whammy required up to `required_whammy_end`.
    pub fn propagate_sp_over_whammy_min(&self, mut start: SpPosition, end: SpPosition, mut sp: f64, required_whammy_end: SpPosition) -> f64 {
        if required_whammy_end.beat > start.beat {
            let whammy_end = if required_whammy_end.beat < end.beat { required_whammy_end } else { end };
            sp = self.propagate_sp_over_whammy_max(start, whammy_end, sp, f64::NEG_INFINITY);
            start = required_whammy_end;
        }
        if start.beat < end.beat {
            sp -= (end.sp_measure - start.sp_measure) / MEASURES_PER_BAR;
        }
        sp.max(0.0)
    }

    fn propagate_over_whammy_range(&self, start: f64, end: f64, sp: f64) -> f64 {
        let (mut rate, mut pos, mut sp) = self.initial_prop_state(start, end, sp);
        while pos < end {
            let sub_end = match self.beat_rates.get(rate + 1) {
                Some(n) => end.min(n.position),
                None => end,
            };
            sp += (sub_end - pos) * self.beat_rates[rate].net_sp_gain_rate;
            if sp < 0.0 {
                return -1.0;
            }
            sp = sp.min(1.0);
            pos = sub_end;
            rate += 1;
        }
        sp
    }

    pub fn is_in_whammy_ranges(&self, beat: f64) -> bool {
        let p = self.first_sp_sustain_after(beat);
        p < self.sustains.len() && self.sustains[p].whammy_start.beat <= beat
    }

    fn sp_from_whammying_range(&self, start: f64, end: f64) -> f64 {
        if start >= end {
            return 0.0;
        }
        (end - start) * self.sp_gain_rate
    }

    /// Whammy available in [start, end) from notes before `note_pos`.
    pub fn available_whammy(&self, mut start: f64, end: f64, note_pos: i32) -> f64 {
        let mut total = 0.0;
        let mut last_burst = f64::NEG_INFINITY;
        let mut p = self.first_sp_sustain_after(start);
        while p < self.sustains.len() && self.sustains[p].whammy_start.beat < end && self.sustains[p].note_position < note_pos && start < end {
            let s = self.sustains[p];
            p += 1;
            if s.burst_position.beat <= last_burst {
                continue;
            }
            total += self.sp_from_whammying_range(s.whammy_start.beat.max(start), s.whammy_end.beat.min(end));
            start = if s.releasable_for_burst { start.max(s.burst_position.beat) } else { start.max(s.whammy_end.beat) };
            last_burst = s.burst_position.beat;
        }
        total
    }

    pub fn available_whammy_all(&self, start: f64, end: f64) -> f64 {
        self.available_whammy(start, end, i32::MAX)
    }

    pub fn next_whammy_point(&self, pos: f64) -> f64 {
        let p = self.first_sp_sustain_after(pos);
        match self.sustains.get(p) {
            Some(s) => s.whammy_start.beat.max(pos),
            None => f64::INFINITY,
        }
    }
}
