//! Two players on the charts both have played.

use super::ratings::{is_hidden, strip_tags};
use std::cmp::Ordering;
use std::collections::HashMap;

/// A player's best run of a chart, with what's needed to compare it.
#[derive(Debug, Clone, Default)]
pub struct Run {
    pub name: String,
    pub artist: String,
    pub score: i64,
    pub accuracy: f64,
    pub fc: bool,
    /// Song speed, and modifiers (sorted): only like is compared with like.
    speed: u32,
    modifiers: Vec<String>,
}

/// Best runs by track (hash, instrument, difficulty).
pub type Runs = HashMap<(String, String, String), Run>;

pub fn runs(scores: &[open_chl_api::Score]) -> Runs {
    let text = |t: &str| if is_hidden(t) { String::new() } else { strip_tags(t) };
    scores
        .iter()
        .filter(|s| !s.track_hash.is_empty() && !s.failed_in_run)
        .filter_map(|s| {
            let mut modifiers = s.modifiers.clone();
            modifiers.sort();
            let run = Run {
                name: text(&s.name),
                artist: text(&s.artist),
                score: s.total_score,
                accuracy: super::stats::accuracy(s)?,
                fc: s.is_fc,
                speed: s.speed,
                modifiers,
            };
            Some(((s.track_hash.clone(), s.instrument.clone(), s.difficulty.clone()), run))
        })
        .collect()
}

/// Who did better on a chart: `Greater` if `a` did, `Less` if `b` did, `Equal` for a tie.
/// The way players judge a run: a full combo first, then accuracy as the leaderboard shows it
/// (to a tenth of a percent, so 99.41% and 99.38% are level), then score.
fn better(a: &Run, b: &Run) -> Ordering {
    let shown = |r: &Run| (r.accuracy * 10.0).round() as i64;
    a.fc.cmp(&b.fc).then_with(|| shown(a).cmp(&shown(b))).then_with(|| a.score.cmp(&b.score))
}

/// A chart both have played.
#[derive(Debug, Clone)]
pub struct Shared {
    pub name: String,
    pub artist: String,
    pub instrument: String,
    pub difficulty: String,
    /// Each one's accuracy and whether it's a full combo.
    pub a: (f64, bool),
    pub b: (f64, bool),
}

#[derive(Debug, Default)]
pub struct Versus {
    pub shared: usize,
    /// Played by both, but at other speeds or with other modifiers.
    pub unlike: usize,
    pub a_wins: usize,
    pub b_wins: usize,
    pub ties: usize,
    /// Charts each is furthest ahead on, by accuracy.
    pub a_leads: Vec<Shared>,
    pub b_leads: Vec<Shared>,
}

pub fn compare(a: &Runs, b: &Runs, list: usize) -> Versus {
    let mut v = Versus::default();
    for (k, ra) in a {
        let Some(rb) = b.get(k) else { continue };
        if ra.speed != rb.speed || ra.modifiers != rb.modifiers {
            v.unlike += 1;
            continue;
        }
        v.shared += 1;
        let s = || Shared {
            name: ra.name.clone(),
            artist: ra.artist.clone(),
            instrument: k.1.clone(),
            difficulty: k.2.clone(),
            a: (ra.accuracy, ra.fc),
            b: (rb.accuracy, rb.fc),
        };
        match better(ra, rb) {
            Ordering::Greater => {
                v.a_wins += 1;
                v.a_leads.push(s());
            }
            Ordering::Less => {
                v.b_wins += 1;
                v.b_leads.push(s());
            }
            Ordering::Equal => v.ties += 1,
        }
    }
    // Furthest ahead first; by name where that's the same, so the order never changes.
    let order = |sign: f64| move |x: &Shared, y: &Shared| ((y.a.0 - y.b.0) * sign).total_cmp(&((x.a.0 - x.b.0) * sign)).then_with(|| x.name.cmp(&y.name));
    v.a_leads.sort_by(order(1.0));
    v.b_leads.sort_by(order(-1.0));
    v.a_leads.truncate(list);
    v.b_leads.truncate(list);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(hash: &str, total: i64, hit: u32, speed: u32) -> open_chl_api::Score {
        open_chl_api::Score {
            track_hash: hash.into(),
            instrument: "guitar".into(),
            difficulty: "expert".into(),
            name: hash.into(),
            total_score: total,
            notes_hit: hit,
            note_count: 100,
            speed,
            ..Default::default()
        }
    }

    #[test]
    fn head_to_head() {
        let a = runs(&[score("x", 900, 99, 100), score("y", 500, 90, 100), score("z", 1, 50, 100), score("fast", 9, 90, 120), score("same", 7, 70, 100)]);
        let b = runs(&[score("x", 800, 95, 100), score("y", 700, 97, 100), score("fast", 9, 90, 100), score("same", 7, 70, 100), score("only", 5, 50, 100)]);
        let v = compare(&a, &b, 10);
        assert_eq!((v.shared, v.unlike, v.a_wins, v.b_wins, v.ties), (3, 1, 1, 1, 1));
        assert_eq!((v.a_leads[0].name.as_str(), v.b_leads[0].name.as_str()), ("x", "y"));
        // A full combo beats a higher score; accuracy is compared as shown, then score decides.
        let run = |fc, accuracy, score| Run { fc, accuracy, score, ..Default::default() };
        assert_eq!(better(&run(true, 100.0, 1), &run(false, 99.9, 9)), Ordering::Greater);
        assert_eq!(better(&run(false, 99.41, 1), &run(false, 99.38, 9)), Ordering::Less);
        assert_eq!(better(&run(false, 99.5, 1), &run(false, 99.3, 9)), Ordering::Greater);
        // The other way round, it's the mirror image.
        let w = compare(&b, &a, 10);
        assert_eq!((w.a_wins, w.b_wins, w.ties), (v.b_wins, v.a_wins, v.ties));
    }
}
