// Ported from CHOpt / SightRead (https://github.com/GenericMadScientist/CHOpt)
// Copyright (C) 2020-2026 Raymond Wright. Rust port for Truss.
// Licensed under the GNU General Public License v3.0 or later.

//! Conversions between ticks, beats, seconds, measures and fretbars. (The full set, as other
//! engines than Clone Hero's use fretbars and overdrive beats.)
#![allow(dead_code)]

/// A strictly increasing piecewise linear function, evaluated both ways.
#[derive(Debug, Clone)]
pub struct TimeConversionMap {
    initial_gradient: f64,
    /// (position, gradient, value)
    pinned: Vec<(f64, f64, f64)>,
}

impl TimeConversionMap {
    pub fn new(initial_gradient: f64, changes: &[(f64, f64)]) -> Self {
        let mut pinned: Vec<(f64, f64, f64)> = changes.iter().map(|&(p, g)| (p, g, 0.0)).collect();
        pinned.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut map = TimeConversionMap { initial_gradient, pinned };
        if map.pinned.is_empty() {
            return map;
        }
        let mut value = map.pinned[0].0 * initial_gradient;
        map.pinned[0].2 = value;
        for i in 0..map.pinned.len() - 1 {
            let dx = map.pinned[i + 1].0 - map.pinned[i].0;
            value += map.pinned[i].1 * dx;
            map.pinned[i + 1].2 = value;
        }
        let at_zero = map.eval(0.0);
        for p in &mut map.pinned {
            p.2 -= at_zero;
        }
        map
    }

    pub fn eval(&self, x: f64) -> f64 {
        if self.pinned.is_empty() {
            return self.initial_gradient * x;
        }
        let next = self.pinned.partition_point(|p| p.0 <= x);
        if next == 0 {
            let p = self.pinned[0];
            return p.2 + (x - p.0) * self.initial_gradient;
        }
        let p = self.pinned[next - 1];
        p.2 + (x - p.0) * p.1
    }

    pub fn inverse(&self, y: f64) -> f64 {
        if self.pinned.is_empty() {
            return y / self.initial_gradient;
        }
        let next = self.pinned.partition_point(|p| p.2 <= y);
        if next == 0 {
            let p = self.pinned[0];
            return p.0 + (y - p.2) / self.initial_gradient;
        }
        let p = self.pinned[next - 1];
        p.0 + (y - p.2) / p.1
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeSignature {
    pub position: i32,
    pub numerator: i32,
    pub denominator: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bpm {
    pub position: i32,
    pub millibeats_per_minute: f64,
}

impl Bpm {
    pub fn bpm(&self) -> f64 {
        self.millibeats_per_minute / 1000.0
    }
}

#[derive(Debug, Clone)]
pub struct TempoMap {
    time_sigs: Vec<TimeSignature>,
    bpms: Vec<Bpm>,
    od_beats: Vec<i32>,
    resolution: i32,
    beats_to_seconds: TimeConversionMap,
    beats_to_fretbars: TimeConversionMap,
    beats_to_measures: TimeConversionMap,
    beats_to_od_beats: TimeConversionMap,
}

/// Later events at the same position win, and a default is added at tick 0.
fn normalise<T: Copy>(events: &mut Vec<T>, default: T, pos: impl Fn(&T) -> i32) {
    events.reverse();
    events.push(default);
    events.sort_by_key(|e| pos(e)); // stable
    events.dedup_by_key(|e| pos(e));
}

impl Default for TempoMap {
    fn default() -> Self {
        TempoMap::new(Vec::new(), Vec::new(), Vec::new(), 192)
    }
}

impl TempoMap {
    pub fn new(time_sigs: Vec<TimeSignature>, bpms: Vec<Bpm>, od_beats: Vec<i32>, resolution: i32) -> Self {
        let resolution = resolution.max(1);
        let res = resolution as f64;
        let seconds: Vec<(f64, f64)> = bpms.iter().map(|b| (b.position as f64 / res, 60000.0 / b.millibeats_per_minute)).collect();
        let fretbars: Vec<(f64, f64)> = time_sigs.iter().map(|t| (t.position as f64 / res, t.denominator as f64 / 4.0)).collect();
        let measures: Vec<(f64, f64)> =
            time_sigs.iter().map(|t| (t.position as f64 / res, t.denominator as f64 * 0.25 / t.numerator as f64)).collect();
        let od: Vec<(f64, f64)> = (0..od_beats.len())
            .map(|i| {
                let beat = od_beats[i] as f64 / res;
                let diff = if i + 1 < od_beats.len() { (od_beats[i + 1] - od_beats[i]) as f64 / res } else { 1.0 };
                (beat, 0.25 / diff)
            })
            .collect();
        let mut map = TempoMap {
            beats_to_seconds: TimeConversionMap::new(0.5, &seconds),
            beats_to_fretbars: TimeConversionMap::new(1.0, &fretbars),
            beats_to_measures: TimeConversionMap::new(0.25, &measures),
            beats_to_od_beats: TimeConversionMap::new(0.25, &od),
            time_sigs,
            bpms,
            od_beats,
            resolution,
        };
        normalise(&mut map.bpms, Bpm { position: 0, millibeats_per_minute: 120000.0 }, |b| b.position);
        normalise(&mut map.time_sigs, TimeSignature { position: 0, numerator: 4, denominator: 4 }, |t| t.position);
        map
    }

    pub fn time_sigs(&self) -> &[TimeSignature] {
        &self.time_sigs
    }
    pub fn bpms(&self) -> &[Bpm] {
        &self.bpms
    }
    pub fn od_beats(&self) -> &[i32] {
        &self.od_beats
    }
    pub fn resolution(&self) -> i32 {
        self.resolution
    }

    pub fn speedup(&self, speed: i32) -> TempoMap {
        let bpms = self.bpms.iter().map(|b| Bpm { millibeats_per_minute: b.millibeats_per_minute * speed as f64 / 100.0, ..*b }).collect();
        TempoMap::new(self.time_sigs.clone(), bpms, self.od_beats.clone(), self.resolution)
    }

    pub fn with_od_beats(&self, od_beats: Vec<i32>) -> TempoMap {
        TempoMap::new(self.time_sigs.clone(), self.bpms.clone(), od_beats, self.resolution)
    }

    pub fn tick_to_beats(&self, ticks: i32) -> f64 {
        ticks as f64 / self.resolution as f64
    }
    pub fn beats_to_ticks(&self, beats: f64) -> i32 {
        (beats * self.resolution as f64) as i32
    }
    pub fn fretbars_to_beats(&self, f: f64) -> f64 {
        self.beats_to_fretbars.inverse(f)
    }
    pub fn measures_to_beats(&self, m: f64) -> f64 {
        self.beats_to_measures.inverse(m)
    }
    pub fn od_beats_to_beats(&self, o: f64) -> f64 {
        self.beats_to_od_beats.inverse(o)
    }
    pub fn seconds_to_beats(&self, s: f64) -> f64 {
        self.beats_to_seconds.inverse(s)
    }
    pub fn beats_to_fretbars(&self, b: f64) -> f64 {
        self.beats_to_fretbars.eval(b)
    }
    pub fn beats_to_measures(&self, b: f64) -> f64 {
        self.beats_to_measures.eval(b)
    }
    pub fn beats_to_od_beats(&self, b: f64) -> f64 {
        self.beats_to_od_beats.eval(b)
    }
    pub fn beats_to_seconds(&self, b: f64) -> f64 {
        self.beats_to_seconds.eval(b)
    }
    pub fn ticks_to_seconds(&self, t: i32) -> f64 {
        self.beats_to_seconds(self.tick_to_beats(t))
    }
    pub fn seconds_to_ticks(&self, s: f64) -> i32 {
        self.beats_to_ticks(self.seconds_to_beats(s))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpMode {
    Measure,
    OdBeat,
}

/// A position as both a beat and the unit Star Power drains in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpPosition {
    pub beat: f64,
    pub sp_measure: f64,
}

impl SpPosition {
    pub const NEG_INF: SpPosition = SpPosition { beat: f64::NEG_INFINITY, sp_measure: f64::NEG_INFINITY };
    pub const POS_INF: SpPosition = SpPosition { beat: f64::INFINITY, sp_measure: f64::INFINITY };
    pub const ZERO: SpPosition = SpPosition { beat: 0.0, sp_measure: 0.0 };
}

#[derive(Debug, Clone)]
pub struct SpTimeMap {
    pub tempo_map: TempoMap,
    pub sp_mode: SpMode,
}

impl SpTimeMap {
    pub fn new(tempo_map: TempoMap, sp_mode: SpMode) -> Self {
        SpTimeMap { tempo_map, sp_mode }
    }
    pub fn sp_measures_to_beats(&self, m: f64) -> f64 {
        match self.sp_mode {
            SpMode::Measure => self.tempo_map.measures_to_beats(m),
            SpMode::OdBeat => self.tempo_map.od_beats_to_beats(m),
        }
    }
    pub fn beats_to_sp_measures(&self, b: f64) -> f64 {
        match self.sp_mode {
            SpMode::Measure => self.tempo_map.beats_to_measures(b),
            SpMode::OdBeat => self.tempo_map.beats_to_od_beats(b),
        }
    }
    pub fn seconds_to_sp_measures(&self, s: f64) -> f64 {
        self.beats_to_sp_measures(self.tempo_map.seconds_to_beats(s))
    }
    pub fn beats_to_seconds(&self, b: f64) -> f64 {
        self.tempo_map.beats_to_seconds(b)
    }
    pub fn seconds_to_beats(&self, s: f64) -> f64 {
        self.tempo_map.seconds_to_beats(s)
    }
    pub fn tick_to_beats(&self, t: i32) -> f64 {
        self.tempo_map.tick_to_beats(t)
    }
    pub fn beats_to_ticks(&self, b: f64) -> i32 {
        self.tempo_map.beats_to_ticks(b)
    }
    pub fn beats_to_fretbars(&self, b: f64) -> f64 {
        self.tempo_map.beats_to_fretbars(b)
    }
    pub fn fretbars_to_beats(&self, f: f64) -> f64 {
        self.tempo_map.fretbars_to_beats(f)
    }
    pub fn pos(&self, beat: f64) -> SpPosition {
        SpPosition { beat, sp_measure: self.beats_to_sp_measures(beat) }
    }
    pub fn pos_from_measure(&self, sp_measure: f64) -> SpPosition {
        SpPosition { beat: self.sp_measures_to_beats(sp_measure), sp_measure }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tempo_is_120_bpm() {
        let t = TempoMap::default();
        assert_eq!(t.beats_to_seconds(4.0), 2.0);
        assert_eq!(t.seconds_to_beats(1.0), 2.0);
        assert_eq!(t.beats_to_measures(8.0), 2.0);
    }

    #[test]
    fn tempo_changes() {
        let t = TempoMap::new(
            vec![TimeSignature { position: 0, numerator: 4, denominator: 4 }, TimeSignature { position: 768, numerator: 3, denominator: 4 }],
            vec![Bpm { position: 0, millibeats_per_minute: 150000.0 }, Bpm { position: 800, millibeats_per_minute: 200000.0 }],
            vec![],
            192,
        );
        assert!((t.beats_to_seconds(4.0) - 1.6).abs() < 1e-9);
        let change = 800.0 / 192.0;
        assert!((t.beats_to_seconds(6.0) - (change * 0.4 + (6.0 - change) * 0.3)).abs() < 1e-9);
        assert!((t.seconds_to_beats(t.beats_to_seconds(6.0)) - 6.0).abs() < 1e-9);
        assert!((t.beats_to_measures(7.0) - 2.0).abs() < 1e-9);
        assert!((t.measures_to_beats(2.0) - 7.0).abs() < 1e-9);
    }
}
