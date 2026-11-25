//! Reads players from the Clone Hero leaderboard API (ported from Truss Stats), gently: one request at a time, with pauses,
//! keeping to the server's rate-limit headers, and everything kept on disk.
//!
//! Uses open-chl-api's types and parsing; the requests are made here because its client doesn't
//! expose response headers.

use open_chl_api::{API_URL, AccountScores, AccountScoresQuery, Profile, Score, Search, UserHit};
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

const GAP: Duration = Duration::from_millis(400);
const MAX_WAIT: Duration = Duration::from_secs(120);
/// 50 a page: 20,000 charts.
const MAX_PAGES: u32 = 400;
const MAX_BODY: usize = 8 << 20;
/// Where profile pictures come from (linked Discord and Google sign-ins).
const AVATAR_HOSTS: [&str; 2] = ["cdn.discordapp.com", "lh3.googleusercontent.com"];
const MAX_AVATAR: usize = 1 << 20;

/// Everything fetched for one player.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Player {
    pub fetched_at: String,
    pub profile: Profile,
    /// One row per track: the player's best run.
    pub scores: Vec<Score>,
    /// More charts than we read.
    pub truncated: bool,
}

/// The one fetcher (so there's only ever one request at a time).
static FETCHER: LazyLock<Result<Fetcher, String>> =
    LazyLock::new(|| Fetcher::new(crate::settings::cache_dir().ok_or("There's no folder to keep scores in.")?.join("players")));

pub fn fetcher() -> Result<&'static Fetcher, String> {
    FETCHER.as_ref().map_err(Clone::clone)
}

pub struct Fetcher {
    http: reqwest::Client,
    cache: PathBuf,
    /// When the next request may go out. Held while one is running, so there is only ever one.
    next: Mutex<Instant>,
}

/// How long the rate-limit headers ask us to wait before the next request.
pub fn wait_from_headers(h: &HeaderMap, limited: bool, now: i64) -> Option<Duration> {
    let num = |k: &str| h.get(k)?.to_str().ok()?.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    let remaining = num("x-ratelimit-remaining").or_else(|| num("ratelimit-remaining"));
    if !limited && remaining.is_none_or(|r| r > 2.0) {
        return None;
    }
    let secs = num("retry-after").or_else(|| {
        // Seconds from now, or a moment in seconds or milliseconds since 1970.
        num("x-ratelimit-reset").or_else(|| num("ratelimit-reset")).map(|r| match r {
            r if r > 1e11 => r / 1000.0 - now as f64,
            r if r > 1e8 => r - now as f64,
            r => r,
        })
    });
    let wait = secs.unwrap_or(if limited { 30.0 } else { 10.0 }).max(1.0) + 0.5;
    Some(Duration::from_secs_f64(wait.min(MAX_WAIT.as_secs_f64())))
}

/// Whether a profile picture address is one we fetch.
pub fn avatar_allowed(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            && u.port().is_none()
            && u.username().is_empty()
            && u.password().is_none()
            && u.host_str().is_some_and(|h| AVATAR_HOSTS.contains(&h))
    })
}

/// Discord sends a small picture unless asked for a size; the saved card draws it at 256.
fn sharp(url: &str) -> String {
    if url.starts_with("https://cdn.discordapp.com/avatars/") && !url.contains('?') {
        format!("{url}?size=256")
    } else {
        url.to_string()
    }
}

/// Whether it's a picture Truss can show, from its first bytes (what the server says isn't trusted).
fn is_image(b: &[u8]) -> bool {
    matches!(
        b,
        [0x89, b'P', b'N', b'G', ..] | [0xff, 0xd8, 0xff, ..] | [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..]
    )
}

async fn body(mut r: reqwest::Response, max: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    while let Some(c) = r.chunk().await.map_err(|_| "The connection dropped. Try again.".to_string())? {
        if out.len() + c.len() > max {
            return Err("The answer was too large.".into());
        }
        out.extend_from_slice(&c);
    }
    Ok(out)
}

impl Fetcher {
    fn new(cache: PathBuf) -> Result<Fetcher, String> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .user_agent(concat!("truss/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Fetcher { http, cache, next: Mutex::new(Instant::now()) })
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T, String> {
        let mut next = self.next.lock().await;
        for _ in 0..4 {
            tokio::time::sleep_until(*next).await;
            let sent = self.http.get(format!("{API_URL}{path}")).query(query).send().await;
            *next = Instant::now() + GAP;
            let r = sent.map_err(|e| {
                if e.is_timeout() { "Clone Hero's leaderboards took too long to answer." } else { "Couldn't reach Clone Hero's leaderboards." }
                    .to_string()
            })?;
            let status = r.status();
            if let Some(w) = wait_from_headers(r.headers(), status.as_u16() == 429, chrono::Utc::now().timestamp()) {
                *next = Instant::now() + w;
            }
            match status.as_u16() {
                429 => continue,
                404 => return Err("Clone Hero's leaderboards don't know that player.".into()),
                s if !status.is_success() => return Err(format!("Clone Hero's leaderboards answered with an error ({s}).")),
                _ => return open_chl_api::parse(&body(r, MAX_BODY).await?),
            }
        }
        Err("Clone Hero's leaderboards are asking us to slow down. Try again in a few minutes.".into())
    }

    /// Players whose name matches, best matches first as the server orders them.
    pub async fn search(&self, name: &str) -> Result<Vec<UserHit>, String> {
        let q = [
            ("q", name.to_string()),
            ("songsPage", "1".to_string()),
            ("usersPage", "1".to_string()),
            ("perPage", "20".to_string()),
        ];
        let mut users = self.get::<Search>("/search", &q).await?.users;
        // An exact name first.
        users.sort_by_key(|u| u.username.to_lowercase() != name.to_lowercase());
        Ok(users)
    }

    fn file(&self, id: u64) -> PathBuf {
        self.cache.join(format!("{id}.json"))
    }

    pub async fn cached(&self, id: u64) -> Option<Player> {
        serde_json::from_slice(&tokio::fs::read(self.file(id)).await.ok()?).ok()
    }

    /// A player's profile and best scores from the server; `progress(done, total)` per page.
    pub async fn player(&self, id: u64, progress: impl Fn(u32, u32)) -> Result<Player, String> {
        let profile: Profile = self.get(&format!("/account/{id}/profile"), &[]).await?;
        if profile.deleted_at.is_some() {
            return Err("That account has been deleted.".into());
        }
        let mut q = AccountScoresQuery { best_per_chart: true, ..Default::default() };
        let (mut scores, mut seen) = (Vec::new(), HashSet::new());
        let mut truncated = false;
        loop {
            let p: AccountScores = self.get(&format!("/account/{id}/scores"), &q.pairs()).await?;
            let last = p.is_last_page();
            progress(q.page, p.total_count.div_ceil(u64::from(p.page_size.max(1))) as u32);
            for s in p.scores {
                // A score sent while we page shifts rows onto the next page.
                if s.track_hash.is_empty() || seen.insert((s.track_hash.clone(), s.instrument.clone(), s.difficulty.clone())) {
                    scores.push(s);
                }
            }
            if last {
                break;
            }
            if q.page >= MAX_PAGES {
                truncated = true;
                break;
            }
            q.page += 1;
        }
        let player = Player { fetched_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true), profile, scores, truncated };
        // A failed save only means fetching again next time.
        let _ = self.save(id, &player).await;
        Ok(player)
    }

    async fn save(&self, id: u64, p: &Player) -> std::io::Result<()> {
        tokio::fs::create_dir_all(&self.cache).await?;
        let tmp = self.cache.join(format!("{id}.tmp"));
        tokio::fs::write(&tmp, serde_json::to_vec(p)?).await?;
        tokio::fs::rename(&tmp, self.file(id)).await
    }

    /// A player's picture.
    pub async fn avatar(&self, url: &str) -> Option<Vec<u8>> {
        if !avatar_allowed(url) {
            return None;
        }
        let r = self.http.get(sharp(url)).send().await.ok().filter(|r| r.status().is_success())?;
        let b = body(r, MAX_AVATAR).await.ok()?;
        is_image(&b).then_some(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(h: &[(&'static str, &str)]) -> HeaderMap {
        h.iter().map(|(k, v)| (reqwest::header::HeaderName::from_static(k), v.parse().unwrap())).collect()
    }

    #[test]
    fn waits_as_told() {
        let w = |h: &[(&'static str, &str)], lim| wait_from_headers(&headers(h), lim, 1_800_000_000).map(|d| d.as_secs_f64());
        assert_eq!(w(&[], false), None);
        assert_eq!(w(&[("x-ratelimit-remaining", "40"), ("x-ratelimit-reset", "9")], false), None);
        assert_eq!(w(&[("x-ratelimit-remaining", "1"), ("x-ratelimit-reset", "9")], false), Some(9.5));
        assert_eq!(w(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "1800000020")], false), Some(20.5));
        assert_eq!(w(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "1800000020000")], false), Some(20.5));
        assert_eq!(w(&[], true), Some(30.5));
        assert_eq!(w(&[("retry-after", "3")], true), Some(3.5));
        assert_eq!(w(&[("retry-after", "99999")], true), Some(120.0));
    }

    #[test]
    fn avatars() {
        assert!(avatar_allowed("https://cdn.discordapp.com/avatars/1/a.png"));
        for bad in [
            "http://cdn.discordapp.com/a.png",
            "https://cdn.discordapp.com.evil.test/a.png",
            "https://u@cdn.discordapp.com/a.png",
            "https://cdn.discordapp.com:8443/a.png",
            "https://127.0.0.1/a.png",
            "file:///c:/a.png",
            "",
        ] {
            assert!(!avatar_allowed(bad), "{bad}");
        }
        assert!(is_image(b"\x89PNG\r\n"));
        assert!(!is_image(b"<svg onload=x>"));
    }
}
