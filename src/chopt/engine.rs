// Ported from CHOpt (https://github.com/GenericMadScientist/CHOpt)
// Copyright (C) 2021-2026 Raymond Wright. Rust port for Truss.
// Licensed under the GNU General Public License v3.0 or later.

//! Scoring and Star Power rules of the game being optimised for.

use super::time::SpMode;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpEngineValues {
    pub phrase_amount: f64,
    pub unison_phrase_amount: f64,
    pub minimum_to_activate: f64,
}

/// Clone Hero's five-fret guitar rules (standard or precision mode).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Engine {
    pub precision: bool,
}

impl Engine {
    pub fn clone_hero(precision: bool) -> Self {
        Engine { precision }
    }
    pub fn base_note_value(&self) -> i32 {
        50
    }
    pub fn burst_size(&self) -> f64 {
        0.25
    }
    pub fn clean_play_bonus(&self) -> i32 {
        2
    }
    pub fn timing_window(&self, early_gap: f64, late_gap: f64) -> f64 {
        if !self.precision {
            return 0.07;
        }
        let total = early_gap.clamp(0.0, 0.0525) + late_gap.clamp(0.0, 0.0525);
        0.27619 * total + 0.021
    }
    pub fn early_timing_window(&self, early_gap: f64, late_gap: f64) -> f64 {
        self.timing_window(early_gap, late_gap)
    }
    pub fn late_timing_window(&self, early_gap: f64, late_gap: f64) -> f64 {
        self.timing_window(early_gap, late_gap)
    }
    pub fn has_early_whammy(&self) -> bool {
        true
    }
    pub fn has_whammy_bursts(&self) -> bool {
        true
    }
    pub fn max_multiplier(&self) -> i32 {
        4
    }
    pub fn resolution_rounded_whammy(&self) -> bool {
        true
    }
    pub fn round_tick_gap(&self) -> bool {
        true
    }
    pub fn sp_engine_values(&self) -> SpEngineValues {
        SpEngineValues { phrase_amount: 0.25, unison_phrase_amount: 0.5, minimum_to_activate: 0.5 }
    }
    pub fn sp_gain_rate(&self) -> f64 {
        1.0 / 30.0
    }
    pub fn sp_mode(&self) -> SpMode {
        SpMode::Measure
    }
    pub fn sust_points_per_beat(&self) -> i32 {
        25
    }
}
