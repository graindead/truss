// Ported from CHOpt (https://github.com/GenericMadScientist/CHOpt), the Star Power optimiser for
// Clone Hero by Raymond Wright. Copyright (C) 2020-2026 Raymond Wright. Rust port for Truss.
//
// This program is free software: you can redistribute it and/or modify it under the terms of
// the GNU General Public License as published by the Free Software Foundation, either version 3
// of the License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY;
// without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See
// the GNU General Public License for more details.

//! Star Power path optimiser: a Rust port of CHOpt's Clone Hero engine for five-fret
//! instruments. Finds the activations that score the most and draws the path as an image.

mod builder;
pub mod draw;
mod engine;
mod optimiser;
mod points;
mod processed;
mod song;
mod sp;
mod time;

pub use builder::PathImage;
pub use song::{Difficulty, Instrument, Song};

use std::sync::atomic::AtomicBool;

/// Which game rules and how the player plays (all as in CHOpt's options).
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub instrument: Instrument,
    pub difficulty: Difficulty,
    /// Clone Hero's precision mode (tighter timing windows).
    pub precision: bool,
    /// Squeeze, 0-100 %.
    pub squeeze: u32,
    /// Early whammy, 0-100 %.
    pub early_whammy: u32,
    /// Milliseconds of whammy lost at the start of each sustain.
    pub lazy_whammy_ms: u32,
    /// Milliseconds after an activation before whammy counts again.
    pub whammy_delay_ms: u32,
    /// Calibration, -200..200 ms.
    pub video_lag_ms: i32,
    /// Song speed in %, a multiple of 5.
    pub speed: u32,
    pub lefty: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            instrument: Instrument::Guitar,
            difficulty: Difficulty::Expert,
            precision: false,
            squeeze: 100,
            early_whammy: 100,
            lazy_whammy_ms: 0,
            whammy_delay_ms: 0,
            video_lag_ms: 0,
            speed: 100,
            lefty: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Output {
    /// CHOpt's text summary: the path, scores and how to hit each activation.
    pub summary: String,
    pub image: PathImage,
}

/// Reads a chart: `notes` is a notes.chart or notes.mid, `ini` the song.ini (may be empty).
pub fn load(notes: &[u8], is_midi: bool, ini: &[u8]) -> Result<Song, String> {
    let meta = song::parse_ini(ini);
    if is_midi { song::parse_midi(notes, &meta) } else { song::parse_chart(notes, &meta) }
}

/// Finds the best path. `cancel` stops it early (returning an error).
pub fn run(mut song: Song, opts: &Options, cancel: &AtomicBool) -> Result<Output, String> {
    let Some(track) = song.tracks.get(&(opts.instrument, opts.difficulty)).cloned() else {
        return Err(format!("This chart has no {} {} part.", opts.difficulty, opts.instrument));
    };
    let speed = (opts.speed.clamp(5, 5000) / 5 * 5) as i32;
    song.speedup(speed);
    let engine = engine::Engine::clone_hero(opts.precision);
    let mut settings = points::PathingSettings::new(engine);
    settings.squeeze = (opts.squeeze.min(100) as f64 / 100.0).max(0.001);
    // Independent of squeeze, as in CHOpt.
    settings.early_whammy = opts.early_whammy.min(100) as f64 / 100.0;
    settings.lazy_whammy = opts.lazy_whammy_ms as f64 / 1000.0;
    settings.whammy_delay = opts.whammy_delay_ms as f64 / 1000.0;
    settings.video_lag = opts.video_lag_ms.clamp(-200, 200) as f64 / 1000.0;

    let tm = song.tempo_map.clone();
    let time_map = time::SpTimeMap::new(tm.clone(), engine.sp_mode());
    let mut image = PathImage::new(&song, &track, &format!("{} {}", opts.difficulty, opts.instrument), opts.lefty);
    let processed = processed::ProcessedSong::new(&track, &time_map, &[], &settings);
    let path = optimiser::Optimiser::new(&processed, cancel, settings.whammy_delay).optimal_path().ok_or("Cancelled")?;
    let summary = processed.path_summary(&path);
    image.add_sp_phrases(&track, &tm, &[]);
    image.add_sp_acts(&processed.points, &tm, &path);
    image.add_measure_values(&processed.points, &tm, &path);
    image.add_sp_percent_values(&processed.sp_data, &time_map, &processed.points, &path, &processed);
    image.set_total_score(&processed, &path);
    Ok(Output { summary, image })
}

#[cfg(test)]
mod tests;
