//! "More like this": which charts Encore is asked for, and how alike two charts are. The search
//! itself runs through the Fretwork filter's rating queue (see fretwork_filter.rs).

use crate::api::{AdvancedFilters, Chart};
use crate::fretwork::Rating;
use std::collections::HashMap;

/// Charts less alike than this (0 to 1) aren't shown.
pub const ALIKE_ENOUGH: f64 = 0.8;
/// Notes per second and length may differ from the chart's by this share.
const NPS_WITHIN: f64 = 0.2;
const LENGTH_WITHIN: f64 = 0.4;

/// A "More like this" search.
pub struct Like {
    pub name: String,
    pub instrument: &'static str,
    of: Rating,
    group: i64,
    song: (String, String),
    /// How alike each chart found is, by md5.
    pub scores: HashMap<String, f64>,
}

fn song(c: &Chart) -> (String, String) {
    let t = |s: &Option<String>| crate::strip_tags(s.as_deref().unwrap_or("")).trim().to_lowercase();
    (t(&c.artist), t(&c.name))
}

impl Like {
    pub fn new(c: &Chart, instrument: &'static str, of: Rating) -> Self {
        let name = crate::strip_tags(c.name.as_deref().unwrap_or("")).to_string();
        Like { name, instrument, of, group: c.group_key(), song: song(c), scores: HashMap::new() }
    }

    /// What Encore narrows its charts by: note rate and length near this chart's.
    pub fn filters(&self, c: &Chart) -> AdvancedFilters {
        let near = |v: Option<f64>, within: f64| {
            let v = v.filter(|v| *v > 0.0);
            let round = |x: f64| (x * 100.0).round() / 100.0;
            (v.map(|v| round(v * (1.0 - within))), v.map(|v| round(v * (1.0 + within))))
        };
        // (Like Truss, Encore counts a chord as one note. Its own count is used when it has one: on
        // drums Truss counts each pad.)
        let secs = c.song_length.filter(|l| *l > 0).map(|l| l as f64 / 1000.0);
        let counted = c.notes_data.as_ref().and_then(|n| {
            let count = n.note_counts.iter().find(|n| n.instrument == self.instrument && n.difficulty == "expert")?.count;
            Some(count as f64 / secs?)
        });
        let nps = near(counted.or(self.of.nps), NPS_WITHIN);
        let len = near(secs.map(|s| s / 60.0), LENGTH_WITHIN);
        AdvancedFilters { min_average_nps: nps.0, max_average_nps: nps.1, min_length: len.0, max_length: len.1, ..Default::default() }
    }

    pub fn tiers(&self) -> (u32, u32) {
        (self.of.tier.saturating_sub(1), self.of.tier + 1)
    }

    /// Whether `c` is alike enough to show (remembering how alike). Never the same song.
    pub fn takes(&mut self, c: &Chart, r: &Rating) -> bool {
        if c.group_key() == self.group || song(c) == self.song {
            return false;
        }
        let a = alike(&self.of, r);
        self.scores.insert(c.md5.clone(), a);
        a >= ALIKE_ENOUGH
    }
}

/// How far apart two amounts are, from 0 (the same) to 1 (one is nothing beside the other).
fn gap(a: f64, b: f64) -> f64 {
    let most = a.abs().max(b.abs());
    if most > 0.0 { (a - b).abs() / most } else { 0.0 }
}

/// How alike two charts are to play: 1 the same, 0 nothing alike.
fn alike(a: &Rating, b: &Rating) -> f64 {
    // How it's played counts most: tier, note rate and length are already held close by the
    // search. Shares are compared as they are (1% and 2% tapped are much the same).
    let style = 0.25 * (a.taps - b.taps).abs() + 0.20 * (a.strums - b.strums).abs() + 0.15 * (a.chords - b.chords).abs();
    let pace = 0.15 * gap(a.nps.unwrap_or(0.0), b.nps.unwrap_or(0.0)) + 0.10 * gap(a.movement, b.movement);
    let hard = 0.10 * gap(a.d, b.d) + 0.05 * gap(a.spread, b.spread);
    1.0 - style - pace - hard
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_is_most_alike() {
        let a = Rating { d: 10.0, tier: 2, nps: Some(6.0), movement: 4.0, spread: 0.2, strums: 0.6, taps: 0.1, chords: 0.3, ..Default::default() };
        assert!((alike(&a, &a) - 1.0).abs() < 1e-9);
        let b = Rating { d: 14.0, nps: Some(9.0), strums: 0.1, taps: 0.8, ..a };
        assert!(alike(&a, &b) < 1.0);
        assert!((0.0..=1.0).contains(&alike(&a, &b)));
        assert!((alike(&a, &b) - alike(&b, &a)).abs() < 1e-9);
    }

    #[test]
    fn leaves_out_the_same_song() {
        let c = Chart { name: Some("A".into()), artist: Some("B".into()), md5: "1".into(), chart_id: 1, song_length: Some(180_000), ..Default::default() };
        let r = Rating { d: 10.0, nps: Some(5.0), strums: 0.9, ..Default::default() };
        let mut like = Like::new(&c, "guitar", r);
        let f = like.filters(&c);
        assert_eq!((f.min_average_nps, f.max_average_nps), (Some(4.0), Some(6.0)));
        let again = Chart { name: Some("a".into()), chart_id: 2, ..c.clone() };
        assert!(!like.takes(&again, &r));
        let other = Chart { name: Some("C".into()), chart_id: 3, ..c };
        assert!(like.takes(&other, &r));
        // Just as hard, but tapped instead of strummed.
        assert!(!like.takes(&other, &Rating { strums: 0.1, taps: 0.8, ..r }));
    }
}
