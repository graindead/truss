//! Difficulty-over-time curves for the Library page's graphs (the same as Truss Index's): a
//! port of Fretwork's render curves (functions/curves.py, by Staycation, MIT).
//!
//! The 1 s windows stepped every 250 ms that the rating uses are turned into per-second rates
//! and smoothed by a single-pole low-pass (time constant 2 s) run forwards then backwards, so
//! peaks don't lag. The D line is the sum of the curves, without the song-level stamina and
//! variation factors, as in Fretwork.
//!
//! The note extraction repeats `fretwork::rate_frets` / `rate_drums` on purpose, so the ratings
//! code stays exactly as ported.

use crate::chart::{Track, nf, nt};
use crate::fretwork::{ROLL_CAP_HPS, STEP_MS, WINDOW_MS, windowed};

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
    let mut times = Vec::new();
    let mut masks: Vec<u8> = Vec::new();
    for g in &t.note_groups {
        let mut m = 0u8;
        for n in g {
            m |= match n.ty {
                nt::OPEN => 1 << 7,
                ty @ nt::GREEN..=nt::ORANGE => 1 << (ty - nt::GREEN),
                _ => 0,
            };
        }
        if m != 0 {
            times.push(g[0].ms);
            masks.push(m);
        }
    }
    let last = *times.last()?;
    let mut changes = Vec::with_capacity(masks.len());
    let mut prev = 0u8;
    for &m in &masks {
        changes.push((prev & !m).count_ones().max((m & !prev).count_ones()) as f64);
        prev = m;
    }
    let nps = smooth(&per_second(windowed(&times, &vec![1.0; times.len()], last)));
    let vps = smooth(&per_second(windowed(&times, &changes, last)));
    let d = sum(&nps, &vps);
    Some(Curves { series: vec![("d", d), ("notes", nps), ("movement", vps)] })
}

fn drum_curves(t: &Track) -> Option<Curves> {
    let (mut hand_t, mut hand_m, mut kick_t, mut kick2_t) = (Vec::new(), Vec::<u8>::new(), Vec::new(), Vec::new());
    for g in &t.note_groups {
        let mut m = 0u8;
        let (mut kick, mut kick2) = (false, false);
        for n in g {
            match n.ty {
                nt::KICK if n.flags & nf::DOUBLE_KICK == 0 => kick = true,
                nt::KICK => kick2 = true,
                ty @ nt::RED_DRUM..=nt::GREEN_DRUM => m |= 1 << (ty - nt::RED_DRUM),
                _ => {}
            }
        }
        if m != 0 {
            hand_t.push(g[0].ms);
            hand_m.push(m);
        }
        if kick {
            kick_t.push(g[0].ms);
        }
        if kick || kick2 {
            kick2_t.push(g[0].ms);
        }
    }
    let has_2x = kick2_t.len() > kick_t.len();
    let last = *hand_t.last()?;

    let rolls: Vec<(f64, f64)> = t.flex.iter().map(|(s, _)| (s.ms, s.ms + s.ms_length)).collect();
    let in_roll = |x: f64| rolls.iter().any(|&(a, b)| x >= a && x < b);
    let (mut normal, mut rolled, mut travel) = (Vec::new(), Vec::new(), Vec::new());
    let mut prev = 0u8;
    for (i, &m) in hand_m.iter().enumerate() {
        let hits = m.count_ones() as f64;
        let added = (m & !prev).count_ones() as f64;
        prev = m;
        if in_roll(hand_t[i]) {
            normal.push(0.0);
            rolled.push(hits);
            travel.push(0.0);
        } else {
            normal.push(hits);
            rolled.push(0.0);
            travel.push(added);
        }
    }
    let hn = windowed(&hand_t, &normal, last);
    let hr = windowed(&hand_t, &rolled, last);
    let hps: Vec<f64> = hn.iter().zip(&hr).map(|(a, b)| a + b.min(ROLL_CAP_HPS)).collect();
    let tps = windowed(&hand_t, &travel, last);
    let kicks = |times: &[f64]| match times.last() {
        Some(&kl) => windowed(times, &vec![1.0; times.len()], kl),
        None => Vec::new(),
    };
    let k1 = kicks(&kick_t);
    let k2 = if has_2x { kicks(&kick2_t) } else { Vec::new() };
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
