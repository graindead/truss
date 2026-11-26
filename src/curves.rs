//! Difficulty-over-time curves for the Library page's graphs (the same as Truss Index's): a
//! port of Fretwork's render curves (functions/curves.py, by Staycation, MIT).
//!
//! The 1 s windows stepped every 250 ms that the rating uses are turned into per-second rates
//! and smoothed by a single-pole low-pass (time constant 2 s) run forwards then backwards, so
//! peaks don't lag. The D line is the sum of the curves, without the song-level stamina and
//! variation factors, as in Fretwork.
//!
//! Notes are read the same way as for the ratings (`fretwork::frets` / `drums`).

use crate::chart::Track;
use crate::fretwork::{STEP_MS, WINDOW_MS, fret_changes, windowed};

const TAU_MS: f64 = 2000.0;

/// Named per-second series on a 250 ms grid starting at 0.
#[derive(Debug, Clone)]
pub struct Curves {
    pub series: Vec<(&'static str, Vec<f64>)>,
}

fn ema_forward(samples: &[f64], decay: f64) -> Vec<f64> {
    let gain = 1.0 - decay;
    let mut acc = 0.0;
    samples
        .iter()
        .map(|&raw| {
            acc = acc * decay + raw * gain;
            acc
        })
        .collect()
}

/// Forward then backward pass, like Fretwork's `_ema_smooth`.
fn smooth(samples: &[f64]) -> Vec<f64> {
    let decay = (-STEP_MS / TAU_MS).exp();
    let fwd = ema_forward(samples, decay);
    let rev: Vec<f64> = fwd.into_iter().rev().collect();
    let mut back = ema_forward(&rev, decay);
    back.reverse();
    back
}

fn pad(mut v: Vec<f64>, n: usize) -> Vec<f64> {
    v.resize(n.max(v.len()), 0.0);
    v
}

fn per_second(v: Vec<f64>) -> Vec<f64> {
    let s = WINDOW_MS / 1000.0;
    v.into_iter().map(|x| x / s).collect()
}

fn sum(a: &[f64], b: &[f64]) -> Vec<f64> {
    a.iter().zip(b).map(|(x, y)| x + y).collect()
}

pub fn curves(instrument: &str, expert: &Track) -> Option<Curves> {
    match instrument {
        "guitar" | "guitarcoop" | "rhythm" | "bass" | "keys" => fret_curves(expert),
        "drums" => drum_curves(expert),
        _ => None,
    }
}

fn fret_curves(t: &Track) -> Option<Curves> {
    let f = crate::fretwork::frets(t);
    let times = f.times;
    let last = *times.last()?;
    let changes = fret_changes(&f.masks);
    let nps = smooth(&per_second(windowed(&times, &vec![1.0; times.len()], last)));
    let vps = smooth(&per_second(windowed(&times, &changes, last)));
    let d = sum(&nps, &vps);
    Some(Curves { series: vec![("d", d), ("notes", nps), ("movement", vps)] })
}

fn drum_curves(t: &Track) -> Option<Curves> {
    let d = crate::fretwork::drums(t);
    let has_2x = d.has_2x();
    let last = *d.hand_t.last()?;
    let (hps, tps) = crate::fretwork::hand_rates(&d, &crate::fretwork::hands(t, &d), last);
    let kicks = |times: &[f64]| match times.last() {
        Some(&kl) => windowed(times, &vec![1.0; times.len()], kl),
        None => Vec::new(),
    };
    let k1 = kicks(&d.kick_t);
    let k2 = if has_2x { kicks(&d.kick2_t) } else { Vec::new() };
    // Hand and kick streams end at different times; pad them all to the longest.
    let n = hps.len().max(k1.len()).max(k2.len());
    let hps = smooth(&per_second(pad(hps, n)));
    let tps = smooth(&per_second(pad(tps, n)));
    let k1 = smooth(&per_second(pad(k1, n)));
    let mut series = vec![("d", sum(&sum(&hps, &tps), &k1))];
    if has_2x {
        let k2 = smooth(&per_second(pad(k2, n)));
        series.push(("d2x", sum(&sum(&hps, &tps), &k2)));
        series.push(("kicks2x", k2));
    }
    series.extend([("hands", hps), ("travel", tps), ("kicks", k1)]);
    Some(Curves { series })
}
