//! Turns a player's best runs into the numbers the page shows (ported from Truss Stats).
//!
//! Everything here is about the best-scoring run per chart, not every attempt, and the page says
//! so. Comparisons against Fretwork ratings use only normal-speed, unmodified Expert runs that
//! matched a rating; nothing unmatched is treated as easy.

use super::fetch::Player;
use super::ratings::{INSTRUMENTS, Match, Miss, Ratings, is_hidden, rated_instrument, strip_tags};
use open_chl_api::Score;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

/// Runs a comparison needs before it is made at all.
pub const MIN_MODEL: usize = 40;
/// Runs a group needs before it is called a strong suit or a weak spot.
pub const MIN_GROUP: usize = 15;
/// How many standard errors, and how many points of accuracy, a group must stand out by.
const SIGNIFICANT: f64 = 2.5;
const MIN_EFFECT: f64 = 0.3;
const NEAR_FC: u32 = 5;
/// Modifiers that don't make the notes easier: a run with these can still be a "hardest FC".
const NOT_EASIER: [&str; 6] = ["mirrorMode", "lightsOut", "precisionMode", "brutalMode", "deadlyDynamics", "noFretGhosting"];

struct Run<'a> {
    s: &'a Score,
    /// Accuracy in percent: the server's, or worked out from the notes.
    acc: Option<f64>,
    rating: Option<Match>,
    pct: f64,
}

impl Run<'_> {
    /// Played as charted: comparable with the rating.
    fn plain(&self) -> bool {
        self.s.speed == 100 && self.s.modifiers.is_empty() && !self.s.failed_in_run
    }

    /// At least as hard as charted.
    fn at_least(&self) -> bool {
        self.s.speed >= 100 && !self.s.failed_in_run && self.s.modifiers.iter().all(|m| NOT_EASIER.contains(&m.as_str()))
    }

    fn missed(&self) -> u32 {
        self.s.note_count.saturating_sub(self.s.notes_hit)
    }

    fn row(&self) -> Value {
        let s = self.s;
        let text = |t: &str| if is_hidden(t) { String::new() } else { strip_tags(t) };
        json!({
            "name": text(&s.name), "artist": text(&s.artist), "charter": text(&s.charter),
            "instrument": s.instrument, "difficulty": s.difficulty,
            "accuracy": self.acc, "missed": self.missed(), "notes": s.note_count, "excess": s.excess_hits,
            "speed": s.speed, "modifiers": s.modifiers, "stars": s.stars, "score": s.total_score, "fc": s.is_fc,
            "failed": s.failed_in_run, "date": s.created_at.get(..10).unwrap_or(""),
            "tier": self.rating.map(|m| m.rating.tier), "d": self.rating.map(|m| m.rating.d),
            "harderThan": self.rating.map(|_| self.pct), "byHash": self.rating.map(|m| m.by_hash),
            "url": link(s),
        })
    }
}

/// The song's leaderboard page, when every part of the address is well formed.
fn link(s: &Score) -> Option<String> {
    let hash_ok = s.song_hash.len() == 43 && s.song_hash.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    (hash_ok && open_chl_api::INSTRUMENTS.contains(&s.instrument.as_str()) && open_chl_api::DIFFICULTIES.contains(&s.difficulty.as_str()))
        .then(|| open_chl_api::site::song_page(&s.song_hash, &s.instrument, &s.difficulty, s.speed, "score"))
}

pub fn accuracy(s: &Score) -> Option<f64> {
    s.accuracy
        .as_deref()
        .and_then(|a| a.parse::<f64>().ok())
        .filter(|a| a.is_finite())
        .or_else(|| (s.note_count > 0).then(|| 100.0 * f64::from(s.notes_hit) / f64::from(s.note_count)))
        .map(|a| a.clamp(0.0, 100.0))
}

pub fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

/// Simple and note-weighted averages, full combos and the rest for a set of runs.
fn summary<'a, 'b: 'a>(runs: impl Iterator<Item = &'a Run<'b>>) -> Value {
    let runs: Vec<&Run> = runs.collect();
    let accs: Vec<f64> = runs.iter().filter_map(|r| r.acc).collect();
    let (hit, notes) = runs
        .iter()
        .filter(|r| r.s.note_count > 0)
        .fold((0u64, 0u64), |(h, n), r| (h + u64::from(r.s.notes_hit.min(r.s.note_count)), n + u64::from(r.s.note_count)));
    let fcs = runs.iter().filter(|r| r.s.is_fc).count();
    json!({
        "charts": runs.len(), "withAccuracy": accs.len(), "simple": mean(&accs),
        "weighted": (notes > 0).then(|| 100.0 * hit as f64 / notes as f64),
        "fcs": fcs, "fcRate": (!runs.is_empty()).then(|| 100.0 * fcs as f64 / runs.len() as f64),
        // Charts full-comboed before, whose best-scoring run isn't itself one.
        "earlierFcs": runs.iter().filter(|r| !r.s.is_fc && r.s.has_previous_fc).count(),
    })
}

/// The accuracy expected at difficulty `x`, from the player's own other runs `(x, accuracy)`:
/// a kernel-weighted average that leaves run `skip` out.
pub fn expected(points: &[(f64, f64)], x: f64, width: f64, skip: usize) -> Option<f64> {
    let (mut sum, mut weight) = (0.0, 0.0);
    for (i, (px, acc)) in points.iter().enumerate() {
        if i != skip {
            let w = (-0.5 * ((px - x) / width).powi(2)).exp();
            sum += w * acc;
            weight += w;
        }
    }
    // Fewer than about five comparable runs: nothing to expect.
    (weight >= 5.0).then(|| sum / weight)
}

#[derive(Debug, PartialEq)]
pub enum Verdict {
    Strong,
    Weak,
    Average,
    NotEnough,
}

/// A group's mean difference from expected, and whether that says anything.
pub fn verdict(diffs: &[f64]) -> (f64, Verdict) {
    let n = diffs.len();
    let m = mean(diffs).unwrap_or(0.0);
    if n < MIN_GROUP {
        return (m, Verdict::NotEnough);
    }
    let var = diffs.iter().map(|d| (d - m).powi(2)).sum::<f64>() / (n - 1) as f64;
    let se = (var / n as f64).sqrt();
    if m.abs() < MIN_EFFECT || m.abs() < SIGNIFICANT * se {
        (m, Verdict::Average)
    } else if m > 0.0 {
        (m, Verdict::Strong)
    } else {
        (m, Verdict::Weak)
    }
}

/// Splits `items` into low / middle / high thirds of `f`. Equal values stay together.
fn thirds<T: Copy>(items: &[T], f: impl Fn(T) -> f64) -> (f64, f64) {
    let mut v: Vec<f64> = items.iter().map(|i| f(*i)).collect();
    v.sort_by(f64::total_cmp);
    (v[v.len() / 3], v[v.len() * 2 / 3])
}

fn instrument_name(i: u8) -> &'static str {
    ["Guitar", "Co-op guitar", "Rhythm", "Bass", "Drums", "Keys"][i as usize]
}

/// Strong suits and weak spots: accuracy against what the player's own runs at the same
/// difficulty predict, for groups of charts.
fn suits(runs: &[Run]) -> Value {
    struct P<'a> {
        r: &'a Run<'a>,
        m: Match,
        acc: f64,
        diff: f64,
    }
    let all: Vec<(&Run, Match, f64)> =
        runs.iter().filter(|r| r.plain()).filter_map(|r| Some((r, r.rating?, r.acc?))).collect();
    let used = all.len();

    // Within one instrument, by D; the width is a fifth of the runs' D (about one tier).
    let mut points: Vec<P> = Vec::new();
    for inst in 0..INSTRUMENTS.len() as u8 {
        let mine: Vec<_> = all.iter().filter(|(_, m, _)| m.rating.instrument == inst).collect();
        if mine.len() < MIN_MODEL {
            continue;
        }
        let xy: Vec<(f64, f64)> = mine.iter().map(|(_, m, a)| (m.rating.d.max(0.1).ln(), *a)).collect();
        for (i, (r, m, acc)) in mine.iter().enumerate() {
            if let Some(e) = expected(&xy, xy[i].0, 0.1, i) {
                points.push(P { r, m: *m, acc: *acc, diff: acc - e });
            }
        }
    }
    if points.len() < MIN_MODEL {
        return json!({ "enough": false, "used": used, "need": MIN_MODEL, "groups": [] });
    }

    let mut groups: Vec<Value> = Vec::new();
    let mut add = |dimension: &str, label: String, detail: String, pts: Vec<&P>| {
        if pts.is_empty() {
            return;
        }
        let (m, v) = verdict(&pts.iter().map(|p| p.diff).collect::<Vec<_>>());
        groups.push(json!({
            "dimension": dimension, "label": label, "detail": detail, "count": pts.len(), "difference": m,
            "accuracy": mean(&pts.iter().map(|p| p.acc).collect::<Vec<_>>()),
            "verdict": match v { Verdict::Strong => "strong", Verdict::Weak => "weak", Verdict::Average => "average", Verdict::NotEnough => "notEnough" },
        }));
    };
    for inst in 0..INSTRUMENTS.len() as u8 {
        let mine: Vec<&P> = points.iter().filter(|p| p.m.rating.instrument == inst).collect();
        if mine.is_empty() {
            continue;
        }
        let who = instrument_name(inst);
        let drums = inst == 4;
        // Movement compared with D, so "more movement" isn't just "harder".
        let share = |p: &P| p.m.rating.movement / p.m.rating.d.max(0.1);
        let (a, b) = thirds(&mine, share);
        let part = |lo: f64, hi: f64, f: &dyn Fn(&P) -> f64| -> Vec<&P> {
            mine.iter().copied().filter(|p| f(p) >= lo && f(p) < hi).collect()
        };
        let (inf, ninf) = (f64::INFINITY, f64::NEG_INFINITY);
        let (more, less) = if drums {
            ("Lots of moving around the kit", "Mostly the same pads")
        } else {
            ("Lots of fret movement", "Little fret movement")
        };
        if a < b {
            add("Movement", format!("{who}: {more}"), "top third of these runs, for how hard they are".into(), part(b, inf, &share));
            add("Movement", format!("{who}: {less}"), "bottom third of these runs, for how hard they are".into(), part(ninf, a, &share));
        }
        let spread = |p: &P| p.m.rating.spread;
        let (a, b) = thirds(&mine, spread);
        if a < b {
            add("Spread", format!("{who}: bursts and breaks"), format!("spread {b:.2} or more"), part(b, inf, &spread));
            add("Spread", format!("{who}: steady charts"), format!("spread under {a:.2}"), part(ninf, a, &spread));
        }
        let length = |p: &P| if p.r.s.duration > 0 { p.r.s.duration as f64 / 60000.0 } else { f64::NAN };
        for (label, detail, lo, hi) in [
            ("short songs", "under 3 minutes", 0.0, 3.0),
            ("medium songs", "3 to 5 minutes", 3.0, 5.0),
            ("long songs", "5 minutes or more", 5.0, inf),
        ] {
            add("Length", format!("{who}: {label}"), detail.into(), part(lo, hi, &length));
        }
        let mut genres: BTreeMap<String, Vec<&P>> = BTreeMap::new();
        for p in &mine {
            let g = strip_tags(&p.r.s.genre);
            if !g.is_empty() && !is_hidden(&g) {
                genres.entry(g.to_lowercase()).or_default().push(p);
            }
        }
        for (_, v) in genres.into_iter().filter(|(_, v)| v.len() >= MIN_GROUP) {
            add("Genre", format!("{who}: {}", strip_tags(&v[0].r.s.genre)), "by the chart's genre".into(), v);
        }
    }
    // Instruments have separate D scales, so they are compared by how many rated charts on the
    // same instrument are easier.
    let xy: Vec<(f64, f64)> = all.iter().map(|(r, _, a)| (r.pct, *a)).collect();
    let mut by_inst: BTreeMap<u8, Vec<P>> = BTreeMap::new();
    for (i, (r, m, acc)) in all.iter().enumerate() {
        if let Some(e) = expected(&xy, r.pct, 7.0, i) {
            by_inst.entry(m.rating.instrument).or_default().push(P { r, m: *m, acc: *acc, diff: acc - e });
        }
    }
    if by_inst.len() > 1 {
        for (i, v) in &by_inst {
            add("Instrument", instrument_name(*i).into(), "against equally hard charts on every instrument played".into(), v.iter().collect());
        }
    }
    groups.sort_by(|a, b| {
        let k = |v: &Value| v["difference"].as_f64().unwrap_or(0.0).abs();
        k(b).total_cmp(&k(a))
    });
    json!({ "enough": true, "used": points.len(), "minGroup": MIN_GROUP, "groups": groups })
}

/// Per instrument: accuracy by Fretwork tier, comfort zone and wall.
fn tiers(runs: &[Run]) -> Value {
    let mut out: Vec<Value> = Vec::new();
    for inst in 0..INSTRUMENTS.len() as u8 {
        let mut by: BTreeMap<u8, Vec<&Run>> = BTreeMap::new();
        for r in runs.iter().filter(|r| r.plain() && r.acc.is_some()) {
            if let Some(m) = r.rating.filter(|m| m.rating.instrument == inst) {
                by.entry(m.rating.tier).or_default().push(r);
            }
        }
        if by.is_empty() {
            continue;
        }
        let avg = |v: &Vec<&Run>| mean(&v.iter().filter_map(|r| r.acc).collect::<Vec<_>>()).unwrap_or(0.0);
        let comfort = by.iter().rfind(|(_, v)| v.len() >= 5 && avg(v) >= 95.0).map(|(t, _)| *t);
        let wall = by.iter().find(|(t, v)| v.len() >= 5 && avg(v) < 90.0 && comfort.is_none_or(|c| **t > c)).map(|(t, _)| *t);
        let fc = by.iter().rfind(|(_, v)| v.len() >= 5 && v.iter().filter(|r| r.s.is_fc).count() * 4 >= v.len()).map(|(t, _)| *t);
        out.push(json!({
            "instrument": INSTRUMENTS[inst as usize], "comfort": comfort, "wall": wall, "fcZone": fc,
            "tiers": by.iter().map(|(t, v)| json!({
                "tier": t, "count": v.len(), "accuracy": avg(v), "fcs": v.iter().filter(|r| r.s.is_fc).count(),
            })).collect::<Vec<_>>(),
        }));
    }
    out.sort_by_key(|v| std::cmp::Reverse(v["tiers"].as_array().unwrap().iter().map(|t| t["count"].as_u64().unwrap()).sum::<u64>()));
    json!(out)
}

/// When the standing best runs were set: accuracy and difficulty by month, and the newest
/// against the ones before.
fn form(runs: &[Run]) -> Value {
    let mut months: BTreeMap<String, Vec<&Run>> = BTreeMap::new();
    let mut plain: Vec<(&Run, f64)> = Vec::new();
    for r in runs.iter().filter(|r| r.s.created_at.len() >= 10) {
        if let Some(a) = r.acc {
            months.entry(r.s.created_at[..].chars().take(7).collect()).or_default().push(r);
            if r.plain() && r.rating.is_some() {
                plain.push((r, a));
            }
        }
    }
    // Difficulty-adjusted: against the player's own runs at the same difficulty.
    plain.sort_by(|a, b| b.0.s.created_at.cmp(&a.0.s.created_at));
    let xy: Vec<(f64, f64)> = plain.iter().map(|(r, a)| (r.pct, *a)).collect();
    let diffs: Vec<Option<f64>> = plain.iter().enumerate().map(|(i, (r, a))| expected(&xy, r.pct, 7.0, i).map(|e| a - e)).collect();
    let recent: Vec<f64> = diffs.iter().take(30).flatten().copied().collect();
    let before: Vec<f64> = diffs.iter().skip(30).flatten().copied().collect();
    let change = (recent.len() >= MIN_GROUP && before.len() >= MIN_GROUP).then(|| {
        let (r, b) = (mean(&recent).unwrap(), mean(&before).unwrap());
        let var = |v: &[f64], m: f64| v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64 / v.len() as f64;
        let se = (var(&recent, r) + var(&before, b)).sqrt();
        json!({ "change": r - b, "recent": recent.len(), "before": before.len(), "clear": (r - b).abs() > 2.0 * se })
    });
    json!({
        "months": months.iter().map(|(m, v)| json!({
            "month": m, "count": v.len(), "accuracy": mean(&v.iter().filter_map(|r| r.acc).collect::<Vec<_>>()),
            "fcs": v.iter().filter(|r| r.s.is_fc).count(),
        })).collect::<Vec<_>>(),
        "recent": change,
    })
}

pub fn build(p: &Player, ratings: &Ratings) -> Value {
    let mut misses: HashMap<Miss, u32> = HashMap::new();
    let (mut by_hash, mut by_name) = (0, 0);
    let runs: Vec<Run> = p
        .scores
        .iter()
        .map(|s| {
            let rating = match ratings.find(s) {
                Ok(m) => {
                    if m.by_hash { by_hash += 1 } else { by_name += 1 }
                    Some(m)
                }
                Err(e) => {
                    *misses.entry(e).or_default() += 1;
                    None
                }
            };
            Run { s, acc: accuracy(s), rating, pct: rating.map_or(0.0, |m| ratings.percentile(&m.rating)) }
        })
        .collect();
    let miss = |m: Miss| misses.get(&m).copied().unwrap_or(0);
    let ratable = runs.len() as u32 - miss(Miss::NotExpert) - miss(Miss::Instrument);

    // Hardest first; instruments are ranked by their own difficulty scale.
    let hardest = |v: &mut Vec<&Run>| v.sort_by(|a, b| b.pct.total_cmp(&a.pct).then(b.rating.map(|m| m.rating.d).unwrap_or(0.0).total_cmp(&a.rating.map(|m| m.rating.d).unwrap_or(0.0))));
    let top = |v: Vec<&Run>, n: usize| v.into_iter().take(n).map(|r| r.row()).collect::<Vec<_>>();

    let mut fcs: Vec<&Run> = runs.iter().filter(|r| r.s.is_fc && r.rating.is_some() && r.at_least()).collect();
    hardest(&mut fcs);
    let fcs_unrated = runs.iter().filter(|r| r.s.is_fc && r.rating.is_none()).count();
    let mut passes: Vec<&Run> = runs.iter().filter(|r| r.rating.is_some() && r.at_least() && r.acc.is_some_and(|a| a >= 80.0)).collect();
    hardest(&mut passes);
    let mut near: Vec<&Run> = runs
        .iter()
        .filter(|r| {
            !r.s.is_fc && !r.s.has_previous_fc && !r.s.failed_in_run && r.s.note_count > 0 && r.s.notes_hit > 0 && r.missed() + r.s.excess_hits <= NEAR_FC
        })
        .collect();
    near.sort_by(|a, b| (a.missed() + a.s.excess_hits).cmp(&(b.missed() + b.s.excess_hits)).then(b.pct.total_cmp(&a.pct)));
    let near_count = near.len();
    let mut longest: Vec<&Run> = runs.iter().filter(|r| r.s.is_fc && r.at_least()).collect();
    longest.sort_by_key(|r| std::cmp::Reverse(r.s.note_count));

    let mut by_board: BTreeMap<(String, String), Vec<&Run>> = BTreeMap::new();
    for r in &runs {
        by_board.entry((r.s.instrument.clone(), r.s.difficulty.clone())).or_default().push(r);
    }
    let mut boards: Vec<Value> = by_board
        .iter()
        .map(|((i, d), v)| {
            let mut j = summary(v.iter().copied());
            j["instrument"] = json!(i);
            j["difficulty"] = json!(d);
            j
        })
        .collect();
    boards.sort_by_key(|b| std::cmp::Reverse(b["charts"].as_u64()));

    let mut mods: BTreeMap<&str, u32> = BTreeMap::new();
    for m in runs.iter().flat_map(|r| &r.s.modifiers) {
        *mods.entry(m).or_default() += 1;
    }
    let count = |f: &dyn Fn(&Run) -> bool| runs.iter().filter(|r| f(r)).count();

    let mut overall = summary(runs.iter());
    overall["expert"] = summary(runs.iter().filter(|r| r.s.difficulty == "expert"));

    let pr = &p.profile;
    json!({
        "player": {
            "id": pr.account_id, "username": strip_tags(&pr.username), "since": pr.created_at.get(..10).unwrap_or(""),
            "avatar": pr.profile_picture_url.as_deref().is_some_and(super::fetch::avatar_allowed),
            "page": open_chl_api::site::player_page(pr.account_id),
            "submissions": pr.submission_count, "stats": pr.stats, "ranks": pr.ranks,
            "topArtists": pr.top_artists.iter().filter(|a| !is_hidden(&a.artist) && !a.artist.is_empty()).take(5)
                .map(|a| json!({ "artist": strip_tags(&a.artist), "plays": a.plays })).collect::<Vec<_>>(),
            "instruments": pr.top_instruments,
        },
        "fetchedAt": p.fetched_at, "truncated": p.truncated,
        "match": {
            "charts": runs.len(), "ratable": ratable, "matched": by_hash + by_name, "byHash": by_hash, "byName": by_name,
            "notExpert": miss(Miss::NotExpert), "instrument": miss(Miss::Instrument), "hidden": miss(Miss::Hidden),
            "ambiguous": miss(Miss::Ambiguous), "notFound": miss(Miss::NotFound),
            "compared": count(&|r| r.plain() && r.rating.is_some() && r.acc.is_some()),
        },
        "overall": overall, "boards": boards,
        "speeds": { "normal": count(&|r| r.s.speed == 100), "faster": count(&|r| r.s.speed > 100), "slower": count(&|r| r.s.speed < 100),
            "failed": count(&|r| r.s.failed_in_run), "modified": count(&|r| !r.s.modifiers.is_empty()), "modifiers": mods },
        "hardestFcs": top(fcs, 10), "fcsUnrated": fcs_unrated,
        "bestPasses": top(passes, 10),
        "nearFcs": top(near, 10), "nearFcCount": near_count, "nearFcLimit": NEAR_FC,
        "longestFcs": top(longest, 5),
        "tiers": tiers(&runs), "suits": suits(&runs), "form": form(&runs),
        "ratedInstrument": pr.top_instruments.first().and_then(|i| rated_instrument(&i.instrument)).is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::ratings::{Rating, chart_key};

    fn score(name: &str, hit: u32, notes: u32) -> Score {
        Score {
            name: name.into(),
            artist: "A".into(),
            charter: "C".into(),
            genre: "Rock".into(),
            duration: 200_000,
            instrument: "guitar".into(),
            difficulty: "expert".into(),
            speed: 100,
            notes_hit: hit,
            note_count: notes,
            is_fc: hit == notes,
            created_at: "2026-11-01T00:00:00.000Z".into(),
            ..Default::default()
        }
    }

    fn rating(name: &str, d: f64, movement: f64) -> Rating {
        Rating {
            hash: [0; 9],
            key: chart_key(name, "A", "C").unwrap(),
            length_ms: 200_000,
            instrument: 0,
            tier: (d / 3.0) as u8,
            d,
            movement,
            spread: 0.2,
            rhythmverse: false,
        }
    }

    fn player(scores: Vec<Score>) -> Player {
        Player { scores, ..Default::default() }
    }

    #[test]
    fn accuracy_sources() {
        let mut s = score("x", 95, 100);
        assert_eq!(accuracy(&s), Some(95.0));
        s.accuracy = Some("94.7".into());
        assert_eq!(accuracy(&s), Some(94.7));
        s.accuracy = Some("NaN".into());
        assert_eq!(accuracy(&s), Some(95.0));
        s.accuracy = None;
        s.note_count = 0;
        assert_eq!(accuracy(&s), None);
    }

    #[test]
    fn averages() {
        let mut none = score("c", 0, 0);
        none.is_fc = false;
        let v = build(&player(vec![score("a", 100, 100), score("b", 800, 1000), none]), &Ratings::default());
        let o = &v["overall"];
        assert_eq!(o["simple"], 90.0); // the run with no accuracy counts in neither
        assert!((o["weighted"].as_f64().unwrap() - 100.0 * 900.0 / 1100.0).abs() < 1e-9);
        assert_eq!((o["charts"].as_u64(), o["withAccuracy"].as_u64(), o["fcs"].as_u64()), (Some(3), Some(2), Some(1)));
    }

    #[test]
    fn hardest_fcs_are_honest() {
        let rs = Ratings::new(vec![rating("easy", 5.0, 1.0), rating("hard", 30.0, 1.0), rating("slow", 50.0, 1.0), rating("mod", 60.0, 1.0)]);
        let mut slow = score("slow", 10, 10);
        slow.speed = 80;
        let mut modded = score("mod", 10, 10);
        modded.modifiers = vec!["allTaps".into()];
        let mut fast = score("hard", 10, 10);
        fast.speed = 125;
        let v = build(&player(vec![score("easy", 10, 10), slow, modded, fast, score("unrated", 10, 10)]), &rs);
        let names: Vec<_> = v["hardestFcs"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["hard", "easy"]);
        assert_eq!(v["hardestFcs"][0]["speed"], 125);
        assert_eq!(v["fcsUnrated"], 1);
        assert_eq!((v["match"]["matched"].as_u64(), v["match"]["notFound"].as_u64()), (Some(4), Some(1)));
        // Only the plain run is compared with its rating.
        assert_eq!(v["match"]["compared"], 1);
    }

    #[test]
    fn near_fcs() {
        let mut over = score("over", 99, 100);
        over.excess_hits = 9;
        let mut done = score("done", 99, 100);
        done.has_previous_fc = true;
        let v = build(&player(vec![score("two", 98, 100), score("one", 99, 100), over, done, score("far", 80, 100)]), &Ratings::default());
        let names: Vec<_> = v["nearFcs"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["one", "two"]);
        assert_eq!(v["overall"]["earlierFcs"], 1);
    }

    #[test]
    fn verdicts_need_evidence() {
        assert_eq!(verdict(&[5.0; 5]).1, Verdict::NotEnough);
        assert_eq!(verdict(&[1.0; 20]).1, Verdict::Strong);
        assert_eq!(verdict(&[-1.0; 20]).1, Verdict::Weak);
        assert_eq!(verdict(&[0.1; 40]).1, Verdict::Average); // steady but tiny
        let noisy: Vec<f64> = (0..20).map(|i| if i % 2 == 0 { 11.0 } else { -9.0 }).collect();
        assert_eq!(verdict(&noisy).1, Verdict::Average);
    }

    #[test]
    fn baseline_leaves_the_run_out() {
        let p: Vec<(f64, f64)> = (0..10).map(|i| (1.0, if i == 0 { 0.0 } else { 90.0 })).collect();
        assert_eq!(expected(&p, 1.0, 0.1, 0), Some(90.0));
        assert_eq!(expected(&p, 9.0, 0.1, 0), None);
    }

    #[test]
    fn finds_a_planted_strong_suit() {
        // Same difficulty spread throughout; the player is two points better on high movement.
        let (mut scores, mut rated) = (Vec::new(), Vec::new());
        for i in 0..90u32 {
            let name = format!("song{i}");
            let d = 10.0 + f64::from(i / 3) * 0.3;
            let high = i % 3 == 0;
            rated.push(rating(&name, d, d * if high { 0.4 } else { 0.1 + 0.05 * f64::from(i % 3) }));
            scores.push(score(&name, if high { 970 } else { 950 } + i % 4, 1000));
        }
        let v = build(&player(scores), &Ratings::new(rated));
        let g = v["suits"]["groups"].as_array().unwrap();
        let strong: Vec<_> = g.iter().filter(|g| g["verdict"] == "strong").collect();
        assert_eq!(strong.len(), 1, "{g:?}");
        assert_eq!(strong[0]["label"], "Guitar: Lots of fret movement");
        assert_eq!(strong[0]["count"], 30);
    }

    #[test]
    fn too_few_runs_says_so() {
        let v = build(&player(vec![score("a", 9, 10)]), &Ratings::new(vec![rating("a", 5.0, 1.0)]));
        assert_eq!(v["suits"]["enough"], false);
        assert!(v["form"]["recent"].is_null());
        build(&Player::default(), &Ratings::default()); // nothing at all
    }
}
