//! Your own Clone Hero scores, for the Library page. Since Clone Hero 1.1 they're kept on its
//! leaderboard server, not on this computer: Truss finds your account number in the game's log
//! (it's in every leaderboard request the game makes) and asks the server for your best score
//! on each chart, the same list your player page on the leaderboard site shows. A copy is kept,
//! so they show while offline.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

const API: &str = "https://api.clonehero.net/v1";

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
        self.by_chart.get(&key(artist, name, charter))?.iter().max_by_key(|b| b.rank())
    }
}

/// A chart as scores are matched to it: artist, name and charter without colour tags, in lower
/// case, with spaces evened out.
fn key(artist: &str, name: &str, charter: &str) -> String {
    let norm = |s: &str| crate::api::strip_tags(s).to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{}|{}|{}", norm(artist), norm(name), norm(charter))
}

/// Clone Hero's own folder of logs and caches (AppData\LocalLow\srylain Inc_\Clone Hero).
fn game_data_dir() -> Option<PathBuf> {
    let home = directories::UserDirs::new()?.home_dir().to_path_buf();
    Some(home.join("AppData").join("LocalLow").join("srylain Inc_").join("Clone Hero"))
}

/// Your account number, from the game's last two logs (the newest mention).
pub fn account_id() -> Option<u64> {
    let dir = game_data_dir()?;
    ["Player.log", "Player-prev.log"].iter().find_map(|f| {
        let text = std::fs::read_to_string(dir.join(f)).ok()?;
        text.rmatch_indices("accountId=").find_map(|(at, m)| {
            let digits: String = text[at + m.len()..].chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
    })
}

/// The kept copy of your scores.
#[derive(Serialize, Deserialize, Default)]
struct Kept {
    account: u64,
    scores: Vec<Best>,
}

fn kept_path() -> Option<PathBuf> {
    crate::settings::cache_dir().map(|d| d.join("my-scores.json"))
}

/// The scores kept from the last time they were fetched (for this account).
pub fn kept() -> Option<Scores> {
    let account = account_id()?;
    let k: Kept = serde_json::from_slice(&std::fs::read(kept_path()?).ok()?).ok()?;
    (k.account == account).then(|| Scores::new(k.scores))
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Page {
    scores: Vec<Best>,
    total_count: usize,
    page_size: usize,
}

/// Your best score on every chart, fresh from the server (and kept).
pub async fn fetch() -> Result<Scores, String> {
    let account = account_id().ok_or("Play a song in Clone Hero while signed in, so Truss can find your account")?;
    let mut all = Vec::new();
    for page in 1..=200 {
        let url = format!("{API}/account/{account}/scores?page={page}&bestPerChart=true");
        let resp = crate::api::CLIENT.get(&url).send().await.map_err(|e| format!("Couldn't reach Clone Hero's leaderboards: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("Clone Hero's leaderboards answered {}", resp.status()));
        }
        let p: Page = resp.json().await.map_err(|e| format!("Unexpected answer from the leaderboards: {e}"))?;
        let done = p.scores.is_empty() || page * p.page_size.max(1) >= p.total_count;
        all.extend(p.scores);
        if done {
            break;
        }
    }
    if let Some(path) = kept_path()
        && let Ok(bytes) = serde_json::to_vec(&Kept { account, scores: all.clone() })
    {
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
