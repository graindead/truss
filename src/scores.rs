//! Your own Clone Hero scores, for the Library page. Since Clone Hero 1.1 they're kept on its
//! leaderboard server, not on this computer: Truss finds your account numbers in the game's
//! logs (they're in every leaderboard request the game makes), remembers them, and asks the
//! server for your best score on each chart, the same list your player page on the leaderboard
//! site shows. A copy is kept for each account, so they show while offline.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// One of your best scores: a chart, on one instrument and difficulty.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Best {
    pub name: String,
    pub artist: String,
    pub charter: String,
    pub instrument: String,
    pub difficulty: String,
    pub speed: u32,
    pub stars: u8,
    pub is_fc: bool,
    pub is_pfc: bool,
    pub total_score: u64,
    /// As the server gives it ("99.4").
    pub accuracy: String,
    pub notes_hit: u32,
    pub note_count: u32,
}

impl Best {
    /// Notes missed (0 without a full combo: only overstrums broke it).
    pub fn misses(&self) -> u32 {
        self.note_count.saturating_sub(self.notes_hit)
    }

    /// Close to a full combo: not one, but with at most this many misses.
    pub fn near_fc(&self, misses: u32) -> bool {
        !self.is_fc && self.note_count > 0 && self.misses() <= misses
    }

    pub fn accuracy(&self) -> f32 {
        self.accuracy.parse().unwrap_or(0.0)
    }

    /// Higher is better: harder difficulty first, then a full combo, then accuracy, score.
    fn rank(&self) -> (u8, bool, bool, u32, u64) {
        let diff = match self.difficulty.as_str() {
            "expert" => 4,
            "hard" => 3,
            "medium" => 2,
            "easy" => 1,
            _ => 0,
        };
        (diff, self.is_pfc, self.is_fc, (self.accuracy() * 10.0) as u32, self.total_score)
    }
}

/// Your best scores, by chart (see `key`).
#[derive(Debug, Default)]
pub struct Scores {
    by_chart: HashMap<String, Vec<Best>>,
    /// How many scores there are in all.
    pub count: usize,
}

impl Scores {
    fn new(list: Vec<Best>) -> Self {
        let count = list.len();
        let mut by_chart: HashMap<String, Vec<Best>> = HashMap::new();
        // Slowed-down plays aren't counted (as the leaderboards don't by default).
        for b in list.into_iter().filter(|b| b.speed >= 100) {
            by_chart.entry(key(&b.artist, &b.name, &b.charter)).or_default().push(b);
        }
        Scores { by_chart, count }
    }

    /// Your best play of a chart: on its hardest difficulty you've played, the best of those.
    pub fn best(&self, artist: &str, name: &str, charter: &str) -> Option<&Best> {
        self.best_for_key(&key(artist, name, charter))
    }

    /// The same, for a chart's `key` worked out already.
    pub fn best_for_key(&self, key: &str) -> Option<&Best> {
        self.by_chart.get(key)?.iter().max_by_key(|b| b.rank())
    }
}

/// A chart as scores are matched to it: artist, name and charter without colour tags, in lower
/// case, with spaces evened out.
pub fn key(artist: &str, name: &str, charter: &str) -> String {
    let norm = |s: &str| crate::api::strip_tags(s).to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{}|{}|{}", norm(artist), norm(name), norm(charter))
}

/// Clone Hero's own folder of logs and caches (AppData\LocalLow\srylain Inc_\Clone Hero).
fn game_data_dir() -> Option<PathBuf> {
    let home = directories::UserDirs::new()?.home_dir().to_path_buf();
    Some(home.join("AppData").join("LocalLow").join("srylain Inc_").join("Clone Hero"))
}

/// Every account number in the game's last two logs, with how often each comes up (most
/// first): more than one when several accounts play on this computer.
pub fn log_accounts() -> Vec<(u64, usize)> {
    let Some(dir) = game_data_dir() else { return Vec::new() };
    let mut seen: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
    for f in ["Player.log", "Player-prev.log"] {
        let Ok(text) = std::fs::read_to_string(dir.join(f)) else { continue };
        for (at, m) in text.match_indices("accountId=") {
            let digits: String = text[at + m.len()..].chars().take_while(char::is_ascii_digit).collect();
            if let Ok(id) = digits.parse() {
                *seen.entry(id).or_default() += 1;
            }
        }
    }
    let mut v: Vec<(u64, usize)> = seen.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

/// An account's name, from its public profile.
pub async fn account_name(account: u64) -> Result<String, String> {
    crate::leaderboard::ch_api().profile(account).await.map(|p| p.username)
}

/// The kept copy of your scores.
#[derive(Serialize, Deserialize, Default)]
struct Kept {
    account: u64,
    scores: Vec<Best>,
}

fn kept_path(account: u64) -> Option<PathBuf> {
    crate::settings::cache_dir().map(|d| d.join("scores").join(format!("{account}.json")))
}

/// An account's scores, kept from the last time they were fetched.
pub fn kept(account: u64) -> Option<Scores> {
    let k: Kept = serde_json::from_slice(&std::fs::read(kept_path(account)?).ok()?).ok()?;
    (k.account == account).then(|| Scores::new(k.scores))
}

impl From<open_chl_api::Score> for Best {
    fn from(s: open_chl_api::Score) -> Self {
        Best {
            name: s.name,
            artist: s.artist,
            charter: s.charter,
            instrument: s.instrument,
            difficulty: s.difficulty,
            speed: s.speed,
            stars: s.stars,
            is_fc: s.is_fc,
            is_pfc: s.is_pfc,
            total_score: s.total_score.max(0) as u64,
            accuracy: s.accuracy.unwrap_or_default(),
            notes_hit: s.notes_hit,
            note_count: s.note_count,
        }
    }
}

/// An account's best score on every chart, fresh from the server (and kept).
pub async fn fetch(account: u64) -> Result<Scores, String> {
    let q = open_chl_api::AccountScoresQuery { best_per_chart: true, ..Default::default() };
    let all: Vec<Best> = crate::leaderboard::ch_api().account_scores_all(account, &q, 200).await?.into_iter().map(Best::from).collect();
    if let Some(path) = kept_path(account)
        && let Ok(bytes) = serde_json::to_vec(&Kept { account, scores: all.clone() })
    {
        let _ = path.parent().map(std::fs::create_dir_all);
        let _ = std::fs::write(path, bytes);
    }
    Ok(Scores::new(all))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn best(diff: &str, acc: &str, fc: bool) -> Best {
        Best { name: "Song".into(), artist: "Band".into(), charter: "Someone".into(), difficulty: diff.into(), accuracy: acc.into(), is_fc: fc, speed: 100, ..Default::default() }
    }

    #[test]
    fn near_full_combos_by_misses() {
        let b = |hit: u32, fc: bool| Best { notes_hit: hit, note_count: 100, is_fc: fc, ..Default::default() };
        assert!(b(95, false).near_fc(5) && !b(94, false).near_fc(5));
        // Every note hit but the combo broken (an overstrum): near. A full combo isn't.
        assert!(b(100, false).near_fc(0) && !b(100, true).near_fc(5));
        // Unknown note counts don't count.
        assert!(!Best::default().near_fc(5));
    }

    #[test]
    fn matches_charts_and_picks_the_best_play() {
        let mut slow = best("expert", "100.0", true);
        slow.speed = 75;
        let s = Scores::new(vec![best("hard", "100.0", true), best("expert", "97.5", false), slow]);
        // Tags, case and spacing don't stop a match.
        let b = s.best("<color=#ff0000>BAND</color>", "song", "  Someone ").unwrap();
        // The hardest difficulty played wins over an easier full combo; slowed plays don't count.
        assert_eq!((b.difficulty.as_str(), b.accuracy()), ("expert", 97.5));
        assert!(s.best("Band", "Other", "Someone").is_none());
    }
}
