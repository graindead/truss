//! The Fretwork tier filter in Advanced search. Encore's search knows nothing about Fretwork,
//! so its results are rated here as they load (each chart's notes, read from its .sng by byte
//! range; see leaderboard::compute) and only charts in the chosen tier range are shown, in
//! Encore's order. Ratings are kept on disk so a chart is only read once.

use crate::advanced::FwFilter;
use crate::api::Chart;
use crate::leaderboard;
use crate::{App, Message};
use iced::Task;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

/// Charts read at once.
const PARALLEL: usize = 4;
/// Result pages fetched per scroll while looking for matches (25 charts each).
const PAGES_PER_SCROLL: u32 = 20;

/// Expert ratings by instrument for a chart, as stored in the cache.
type Tiers = Vec<(String, crate::fretwork::Rating)>;

#[derive(Default)]
pub struct FwSearch {
    pub filter: Option<FwFilter>,
    /// Results waiting for their rating, in Encore's order.
    queue: VecDeque<Arc<Chart>>,
    started: HashSet<String>,
    inflight: usize,
    /// Charts rated (or found unreadable) in this search.
    pub checked: u64,
    /// Pages still allowed before waiting for the next scroll.
    budget: u32,
    /// md5 -> tiers; `None` when the chart couldn't be read.
    ratings: HashMap<String, Option<Tiers>>,
    loaded: bool,
    unsaved: usize,
}

fn cache_file() -> Option<std::path::PathBuf> {
    // v2: full ratings (v1 kept only the tier).
    crate::settings::project_dirs().map(|d| d.cache_dir().join("fretwork-ratings-v2.json"))
}

impl App {
    /// The tier filter is on for the current (Advanced) search.
    pub fn fw_active(&self) -> bool {
        self.fw.filter.is_some() && self.active_advanced.is_some()
    }

    /// Results still waiting to be rated.
    pub fn fw_pending(&self) -> bool {
        self.fw_active() && !self.fw.queue.is_empty()
    }

    /// Starts a new search: forgets queued results and allows a fresh run of pages.
    pub fn fw_reset(&mut self) {
        self.fw.queue.clear();
        self.fw.started.clear();
        self.fw.checked = 0;
        self.fw.budget = PAGES_PER_SCROLL;
        if !self.fw.loaded {
            self.fw.loaded = true;
            self.fw.ratings = cache_file()
                .and_then(|p| std::fs::read(p).ok())
                .and_then(|b| serde_json::from_slice::<HashMap<String, Option<Tiers>>>(&b).ok())
                .unwrap_or_default();
        }
    }

    /// The user scrolled to the end: allow another run of pages.
    pub fn fw_more(&mut self) {
        self.fw.budget = PAGES_PER_SCROLL;
    }

    /// A page of Encore results arrived: queue it for rating.
    pub fn fw_take_page(&mut self, data: Vec<Arc<Chart>>) -> Task<Message> {
        if data.is_empty() {
            self.encore_seen = self.found;
        }
        self.encore_seen += data.len() as u64;
        self.fw.queue.extend(data);
        self.fw_pump()
    }

    pub fn fw_rated(&mut self, generation: u64, md5: String, result: Result<crate::trackhash::Computed, String>) -> Task<Message> {
        self.fw.inflight = self.fw.inflight.saturating_sub(1);
        let tiers = result.as_ref().ok().map(|cp| cp.ratings.clone());
        self.fw.ratings.insert(md5.clone(), tiers);
        if let Ok(cp) = result {
            // The leaderboard can use the same reading.
            self.hashes.insert(md5, leaderboard::Hashes::Done(Ok(cp)));
        }
        self.fw.unsaved += 1;
        if generation != self.generation {
            return Task::none();
        }
        self.fw.checked += 1;
        self.fw_pump()
    }

    /// Shows rated results in order, starts reading the next ones, and fetches more pages
    /// while there are too few matches.
    fn fw_pump(&mut self) -> Task<Message> {
        let Some(filter) = self.fw.filter.clone() else { return Task::none() };
        // Rated charts at the front of the queue are shown (if they match) or dropped.
        let mut shown = Vec::new();
        while let Some(c) = self.fw.queue.front() {
            let Some(rating) = self.fw.ratings.get(&c.md5) else { break };
            if !self.fw.started.contains(&c.md5) {
                // Rated in an earlier search: count it now.
                self.fw.checked += 1;
            }
            let r = rating.as_ref().and_then(|t| t.iter().find(|(i, _)| i == filter.instrument).map(|(_, r)| *r));
            let listed = c.diff_for(filter.instrument);
            let c = self.fw.queue.pop_front().unwrap();
            if r.is_some_and(|r| filter.accepts(&r, listed)) {
                shown.push(c);
            }
        }
        if !shown.is_empty() {
            self.ingest(shown);
            self.list_rev += 1;
        }
        // Read the next charts.
        let generation = self.generation;
        let mut tasks = Vec::new();
        for c in self.fw.queue.iter() {
            if self.fw.inflight >= PARALLEL {
                break;
            }
            if self.fw.ratings.contains_key(&c.md5) || !self.fw.started.insert(c.md5.clone()) {
                continue;
            }
            let Some(src) = leaderboard::source(c) else {
                self.fw.ratings.insert(c.md5.clone(), None);
                self.fw.checked += 1;
                continue;
            };
            self.fw.inflight += 1;
            let md5 = c.md5.clone();
            tasks.push(Task::perform(leaderboard::compute(src), move |r| Message::FwRated(generation, md5.clone(), r)));
        }
        // Charts marked unreadable just now: show progress past them.
        if tasks.is_empty() && self.fw.queue.front().is_some_and(|c| self.fw.ratings.contains_key(&c.md5)) {
            return self.fw_pump();
        }
        if self.fw.queue.is_empty() {
            if self.fw.unsaved > 0 {
                self.fw_save();
            }
            if self.fw.budget > 0 && !self.loading && self.encore_seen < self.found {
                self.fw.budget -= 1;
                tasks.push(self.search(true));
            }
        }
        Task::batch(tasks)
    }

    fn fw_save(&mut self) {
        self.fw.unsaved = 0;
        let Some(path) = cache_file() else { return };
        let Ok(json) = serde_json::to_vec(&self.fw.ratings) else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, json);
    }

    /// Progress line for the results footer.
    pub fn fw_status(&self) -> Option<String> {
        let f = self.fw.filter.as_ref().filter(|_| self.fw_active())?;
        let label = crate::advanced::FW_INSTRUMENTS.iter().find(|(k, _)| *k == f.instrument).map(|(_, l)| *l).unwrap_or("");
        let n = self.results.len();
        let working = !self.fw.queue.is_empty() || self.loading;
        Some(format!(
            "{n} chart{} match the Fretwork filter ({label}), from {} checked{}",
            if n == 1 { "" } else { "s" },
            self.fw.checked,
            if working { " — rating more…" } else if self.encore_seen < self.found { ". Scroll for more." } else { "" }
        ))
    }
}
