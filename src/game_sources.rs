//! Where each chart of the official game setlists (`game_setlists`) can be downloaded on its own:
//! Chorus Encore first, then RhythmVerse. Installing a game this way gets its charts one at a
//! time from those sites instead of the spreadsheet's large Google Drive archives, which often
//! hit Drive's download limit.
//!
//! The list (`data/game-sources.json`) is worked out ahead of time by the `build` test below,
//! from the pack index (which charts each setlist download has), Chorus's official-charter charts
//! (with the pack each is filed under) and RhythmVerse's, plus hand-made fixes in
//! `data/game-source-overrides.json` for names the rules can't match. Like the pack index, Truss
//! also fetches the published copy, so fixes reach everyone without a new release.

use crate::api::Chart;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Where Truss gets the latest list.
pub const URL: &str = "https://raw.githubusercontent.com/graindead/truss/main/data/game-sources.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Sources {
    pub version: u32,
    /// When it was built (seconds since 1970).
    pub built: u64,
    /// By game title, then download label ("Main setlist", "DLC"...), as in `game_setlists`.
    pub games: BTreeMap<String, BTreeMap<String, Vec<Source>>>,
}

/// One chart of a setlist, and where to get it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    /// "encore" or "rv".
    pub from: String,
    /// The Encore chart's md5, or the RhythmVerse file id.
    pub id: String,
    pub artist: String,
    pub name: String,
    pub charter: String,
    /// RhythmVerse only: its download, the outside host it links to, and its page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
}

impl Source {
    /// The key Truss matches library charts on.
    pub fn key(&self) -> String {
        crate::api::library_key(&self.artist, &self.name, &self.charter)
    }

    /// A RhythmVerse source as a chart Truss can download.
    fn rv_chart(&self) -> Chart {
        Chart {
            chart_id: 9_000_000_000u64 + self.id.bytes().fold(0u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64)) % 1_000_000_000,
            md5: format!("rv-{}", self.id),
            name: Some(self.name.clone()),
            artist: Some(self.artist.clone()),
            charter: Some(self.charter.clone()),
            rv: Some(Arc::new(crate::rhythmverse::RvFile {
                download_url: self.download_url.clone(),
                external_url: self.external_url.clone(),
                page_url: self.page_url.clone().unwrap_or_default(),
                title: Some(self.name.clone()),
            })),
            ..Default::default()
        }
    }
}

impl Sources {
    pub fn parse(bytes: &[u8]) -> Option<Sources> {
        serde_json::from_slice(bytes).ok()
    }

    /// A setlist download's charts, if they all have a source.
    pub fn get(&self, game: &str, label: &str) -> Option<&[Source]> {
        self.games.get(game)?.get(label).map(Vec::as_slice).filter(|s| !s.is_empty())
    }
}

fn cache_path() -> Option<std::path::PathBuf> {
    crate::settings::project_dirs().map(|d| d.cache_dir().join("packs").join("game-sources.json"))
}

/// The list: the newest of the copy built into Truss, the last one downloaded, and the one
/// published now.
pub async fn load() -> Option<Sources> {
    let built_in = Sources::parse(include_bytes!("../data/game-sources.json"));
    let cached = tokio::task::spawn_blocking(|| cache_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| Sources::parse(&b)))
        .await
        .ok()
        .flatten();
    let fresh = async {
        let r = crate::api::CLIENT.get(URL).timeout(std::time::Duration::from_secs(20)).send().await.ok()?;
        let b = r.error_for_status().ok()?.bytes().await.ok()?;
        let s = Sources::parse(&b)?;
        if let Some(p) = cache_path() {
            let _ = std::fs::create_dir_all(p.parent()?);
            let _ = std::fs::write(p, &b);
        }
        Some(s)
    }
    .await;
    [built_in, cached, fresh].into_iter().flatten().max_by_key(|s| s.built)
}

/// The charts to download for `sources`: Encore's looked up by md5 (50 at a time), RhythmVerse's
/// made from the list. Also returns how many Encore no longer has (replaced or removed since the
/// list was built).
pub async fn resolve(sources: Vec<Source>) -> Result<(Vec<Arc<Chart>>, usize), String> {
    let mut out: Vec<Arc<Chart>> = sources.iter().filter(|s| s.from == "rv").map(|s| Arc::new(s.rv_chart())).collect();
    let encore: Vec<&Source> = sources.iter().filter(|s| s.from == "encore").collect();
    let mut gone = 0;
    for (i, batch) in encore.chunks(50).enumerate() {
        // Encore limits how fast it's asked.
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        }
        let filters = crate::api::AdvancedFilters { hash: batch.iter().map(|s| s.id.as_str()).collect::<Vec<_>>().join(","), ..Default::default() };
        let r = crate::api::advanced_search(Arc::new(filters), crate::api::Common::new(1, None, None, None)).await?;
        let found: std::collections::HashSet<&str> = r.data.iter().map(|c| c.md5.as_str()).collect();
        gone += batch.iter().filter(|s| !found.contains(s.id.as_str())).count();
        for c in r.data.iter().filter(|c| batch.iter().any(|s| s.id == c.md5)) {
            if !out.iter().any(|o| o.md5 == c.md5) {
                out.push(c.clone());
            }
        }
    }
    Ok((out, gone))
}

// ---------- Building the list ----------

/// A song's artist or title, loosely: no tags, accents, punctuation, "The ", "&" vs "and", or
/// notes like "(Live)", "(WaveGroup)" and "(2x Bass Pedal Expert+)" that sites add or drop.
pub fn loose(s: &str) -> String {
    let s = crate::api::strip_tags(s).to_lowercase();
    let mut out = String::new();
    let mut depth = 0;
    let mut paren = String::new();
    for c in s.chars() {
        match c {
            // Only the outermost brackets count: "Snow ((Hey Oh))" is "Snow (Hey Oh)".
            '(' | '[' => {
                if depth == 0 {
                    paren.clear();
                }
                depth += 1;
            }
            ')' | ']' if depth > 1 => depth -= 1,
            ')' | ']' if depth == 1 => {
                depth = 0;
                let p = paren.trim();
                let dropped = ["live", "wavegroup", "2x", "bass pedal", "version", "feat", "ft.", "explicit", "clean"].iter().any(|w| p.contains(w));
                if !dropped {
                    out.push_str(p);
                }
            }
            _ if depth > 0 => paren.push(c),
            _ => out.push(c),
        }
    }
    let out = out.replace('&', " and ").replace('+', " plus ");
    let out = out.trim_start();
    let out = out.strip_prefix("the ").unwrap_or(out);
    out.chars()
        .filter_map(|c| {
            let c = match c {
                'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a',
                'è' | 'é' | 'ê' | 'ë' => 'e',
                'ì' | 'í' | 'î' | 'ï' => 'i',
                'ò' | 'ó' | 'ô' | 'ö' | 'õ' | 'ø' => 'o',
                'ù' | 'ú' | 'û' | 'ü' => 'u',
                'ñ' => 'n',
                'ç' => 'c',
                c => c,
            };
            c.is_ascii_alphanumeric().then_some(c)
        })
        .collect()
}

#[cfg(all(test, feature = "rhythmverse"))]
mod build {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Overrides {
        /// Download labels containing one of these are left out ("Prototype", "Endless").
        skip_labels: Vec<String>,
        /// Games left out entirely.
        skip_games: Vec<String>,
        /// Charter names that mark an official chart (any part of the charter field).
        charters: Vec<String>,
        /// Each game's packs on Chorus (their packName), preferred over the same song elsewhere.
        packs: BTreeMap<String, Vec<String>>,
        /// "artist - title" in the spreadsheet → "artist - title" on Chorus or RhythmVerse.
        names: BTreeMap<String, String>,
    }

    struct Cand {
        source: Source,
        pack: String,
        loose_key: String,
        /// The charter field is only official names (not a community co-charter).
        pure: bool,
    }

    fn drive_id(u: &str) -> Option<String> {
        let i = ["/file/d/", "/folders/", "id="].iter().find_map(|m| u.find(m).map(|i| i + m.len()))?;
        Some(u[i..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect())
    }

    fn now() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }

    /// Builds `data/game-sources.json` (TRUSS_BUILD_GAME_SOURCES = where to write it). Reads the
    /// pack index and overrides from `data/`, Chorus over the network, and RhythmVerse from
    /// Truss's local copy of its catalogue (Settings > RhythmVerse, kept up to date by Truss).
    /// Prints the charts it couldn't place, for `names` in the overrides.
    #[test]
    #[ignore]
    fn build_game_sources() {
        let Some(out_path) = std::env::var_os("TRUSS_BUILD_GAME_SOURCES") else { return };
        let ov: Overrides = serde_json::from_slice(include_bytes!("../data/game-source-overrides.json")).expect("overrides");
        let games: Vec<crate::game_setlists::Game> = serde_json::from_str(crate::game_setlists::SNAPSHOT).unwrap();
        let index = crate::pack_index::PackIndex::parse(include_bytes!("../data/pack-index.json")).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let official = |charter: &str| {
            let c = charter.to_lowercase();
            ov.charters.iter().any(|w| c.contains(&w.to_lowercase()))
        };
        let pure = |charter: &str| {
            charter.split([',', '&', '/']).map(str::trim).filter(|p| !p.is_empty()).all(|p| ov.charters.iter().any(|w| p.to_lowercase().contains(&w.to_lowercase())))
        };
        // Chorus: every chart by an official charter, with its pack.
        let mut cands: Vec<Cand> = Vec::new();
        let mut seen = HashSet::new();
        for w in &ov.charters {
            for page in 1.. {
                let f = crate::api::AdvancedFilters {
                    charter: crate::api::TextFilter { value: w.clone(), exact: false, exclude: false },
                    ..Default::default()
                };
                let mut common = crate::api::Common::new(page, None, None, None);
                common.per_page = 250;
                let r = match rt.block_on(crate::api::advanced_search(Arc::new(f), common)) {
                    Ok(r) => r,
                    Err(e) => panic!("Chorus search for {w}, page {page}: {e}"),
                };
                if r.data.is_empty() {
                    break;
                }
                for c in &r.data {
                    let charter = c.charter.clone().unwrap_or_default();
                    if !official(&charter) || !seen.insert(c.md5.clone()) {
                        continue;
                    }
                    let (artist, name) = (c.artist.clone().unwrap_or_default(), c.name.clone().unwrap_or_default());
                    cands.push(Cand {
                        loose_key: format!("{}|{}", loose(&artist), loose(&name)),
                        pack: c.pack_name.clone().unwrap_or_default(),
                        pure: pure(&charter),
                        source: Source {
                            from: "encore".into(),
                            id: c.md5.clone(),
                            artist: crate::api::strip_tags(&artist),
                            name: crate::api::strip_tags(&name),
                            charter: crate::api::strip_tags(&charter),
                            download_url: None,
                            external_url: None,
                            page_url: None,
                        },
                    });
                }
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
        }
        let encore_count = cands.len();
        // RhythmVerse, from Truss's local catalogue.
        let words: Vec<&str> = ov.charters.iter().map(String::as_str).collect();
        for c in crate::rv_catalog::by_charter(&words).expect("RhythmVerse catalogue") {
            let charter = c.charter.clone().unwrap_or_default();
            let Some(rv) = &c.rv else { continue };
            if !official(&charter) || (rv.download_url.is_none() && rv.external_url.is_none()) {
                continue;
            }
            let (artist, name) = (c.artist.clone().unwrap_or_default(), c.name.clone().unwrap_or_default());
            cands.push(Cand {
                loose_key: format!("{}|{}", loose(&artist), loose(&name)),
                pack: String::new(),
                pure: pure(&charter),
                source: Source {
                    from: "rv".into(),
                    id: c.md5.trim_start_matches("rv-").to_string(),
                    artist: crate::api::strip_tags(&artist),
                    name: crate::api::strip_tags(&name),
                    charter: crate::api::strip_tags(&charter),
                    download_url: rv.download_url.clone(),
                    external_url: rv.external_url.clone(),
                    page_url: Some(rv.page_url.clone()),
                },
            });
        }
        println!("{encore_count} official charts on Chorus, {} on RhythmVerse", cands.len() - encore_count);
        let mut by_key: HashMap<&str, Vec<&Cand>> = HashMap::new();
        for c in &cands {
            by_key.entry(c.loose_key.as_str()).or_default().push(c);
        }
        let names: HashMap<String, String> = ov.names.iter().map(|(k, v)| (k.to_lowercase(), v.clone())).collect();
        // Each setlist download's charts, from the pack index.
        let mut out = Sources { version: 1, built: now(), games: BTreeMap::new() };
        let (mut placed, mut from_rv, mut missing) = (0, 0, Vec::new());
        for g in &games {
            if ov.skip_games.contains(&g.title) {
                continue;
            }
            let game_packs: Vec<String> = ov.packs.get(&g.title).cloned().unwrap_or_default();
            for l in &g.links {
                if ov.skip_labels.iter().any(|s| l.label.contains(s.as_str())) {
                    continue;
                }
                let id = drive_id(&l.url);
                let Some(entry) = index.links.get(&l.url).or_else(|| index.links.iter().find(|(u, _)| id.is_some() && drive_id(u) == id).map(|(_, e)| e))
                else {
                    println!("not in the pack index: {} - {}", g.title, l.label);
                    continue;
                };
                let mut list: Vec<Source> = Vec::new();
                let mut keys: Vec<&str> = entry.charts.iter().map(|(k, _)| k.as_str()).collect();
                keys.sort();
                keys.dedup();
                for k in keys {
                    let mut parts = k.splitn(3, '|');
                    let (a, n, ch) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                    let wanted = names.get(&format!("{a} - {n}")).cloned();
                    let lk = match &wanted {
                        Some(w) => {
                            let (wa, wn) = w.split_once(" - ").unwrap_or(("", w));
                            format!("{}|{}", loose(wa), loose(wn))
                        }
                        None => format!("{}|{}", loose(a), loose(n)),
                    };
                    // Best first: this game's Chorus pack, then Chorus, then RhythmVerse; the
                    // spreadsheet's charter; only official names in the charter field.
                    let best = by_key.get(lk.as_str()).and_then(|cs| {
                        cs.iter().min_by_key(|c| {
                            (
                                !(c.source.from == "encore" && game_packs.contains(&c.pack)),
                                c.source.from != "encore",
                                c.source.charter.to_lowercase() != ch,
                                !c.pure,
                                c.source.name.to_lowercase() != n,
                            )
                        })
                    });
                    match best {
                        Some(c) => {
                            placed += 1;
                            from_rv += (c.source.from == "rv") as usize;
                            if !list.contains(&c.source) {
                                list.push(c.source.clone());
                            }
                        }
                        None => missing.push(format!("{} - {}: {a} - {n} [{ch}]", g.title, l.label)),
                    }
                }
                list.sort_by(|a, b| (a.artist.to_lowercase(), a.name.to_lowercase()).cmp(&(b.artist.to_lowercase(), b.name.to_lowercase())));
                out.games.entry(g.title.clone()).or_default().insert(l.label.clone(), list);
            }
        }
        for m in &missing {
            println!("MISSING {m}");
        }
        println!("placed {placed} ({from_rv} from RhythmVerse), {} missing", missing.len());
        let mut bytes = serde_json::to_vec_pretty(&out).unwrap();
        bytes.push(b'\n');
        std::fs::write(out_path, bytes).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loose_names() {
        assert_eq!(loose("The Jimi Hendrix Experience"), loose("Jimi Hendrix Experience"));
        assert_eq!(loose("Afterlife (2x Bass Pedal Expert+)"), loose("Afterlife"));
        assert_eq!(loose("Simon & Garfunkel"), loose("Simon and Garfunkel"));
        assert_eq!(loose("Back in Black (Live)"), loose("Back In Black"));
        assert_eq!(loose("Maxïmo Park"), loose("Maximo Park"));
        assert_eq!(loose("Snow ((Hey Oh))"), loose("Snow (Hey Oh)"));
        assert_ne!(loose("Laughtrack (Co-op)"), loose("Laughtrack"), "co-op versions are separate charts");
    }

    #[test]
    fn built_in_list_parses() {
        let s = Sources::parse(include_bytes!("../data/game-sources.json")).unwrap();
        assert!(s.get("Rock Band 2", "DLC").is_some_and(|l| l.len() > 900));
    }

    /// Network: every source in the built-in list still resolves to a chart Truss can
    /// download (Encore's by md5).
    #[test]
    #[ignore]
    fn sources_resolve() {
        let s = Sources::parse(include_bytes!("../data/game-sources.json")).unwrap();
        let all: Vec<Source> = s.games.values().flat_map(|l| l.values().flatten()).cloned().collect();
        let mut unique = all.clone();
        unique.sort_by(|a, b| (&a.from, &a.id).cmp(&(&b.from, &b.id)));
        unique.dedup_by(|a, b| a.from == b.from && a.id == b.id);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (charts, gone) = rt.block_on(resolve(unique.clone())).unwrap_or_else(|e| panic!("{e}"));
        println!("{} sources: {} charts, {gone} gone from Encore", unique.len(), charts.len());
        assert!(charts.iter().filter(|c| c.rv.is_some()).all(|c| c.rv.as_ref().is_some_and(|r| r.download_url.is_some() || r.external_url.is_some())));
        assert_eq!(gone, 0);
    }

    /// Network: downloads one Chorus and one RhythmVerse chart of the list into a scratch
    /// "game folder" (TRUSS_TEST_GAME_DIR), the way Install does.
    #[test]
    #[ignore]
    fn installs_one_of_each() {
        let Some(dir) = std::env::var_os("TRUSS_TEST_GAME_DIR").map(std::path::PathBuf::from) else { return };
        std::fs::create_dir_all(&dir).unwrap();
        let s = Sources::parse(include_bytes!("../data/game-sources.json")).unwrap();
        let all: Vec<&Source> = s.games.values().flat_map(|l| l.values().flatten()).collect();
        let pick = vec![all.iter().find(|s| s.from == "encore").copied().cloned().unwrap(), all.iter().find(|s| s.from == "rv").copied().cloned().unwrap()];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (charts, _) = rt.block_on(resolve(pick)).unwrap();
        let settings = crate::settings::Settings { library_path: Some(dir.clone()), ..Default::default() };
        for c in charts {
            let mut st = Box::pin(crate::download::start(c.clone(), settings.clone(), Vec::new()));
            let r = rt.block_on(async {
                use futures::StreamExt;
                let mut last = None;
                while let Some(p) = st.next().await {
                    if let crate::download::Progress::Finished(r) = p {
                        last = Some(r);
                    }
                }
                last.unwrap()
            });
            println!("{} - {} ({}): {:?}", c.artist.clone().unwrap_or_default(), c.name.clone().unwrap_or_default(), if c.rv.is_some() { "RhythmVerse" } else { "Chorus" }, r);
            assert!(r.unwrap().starts_with(&dir));
        }
    }
}
