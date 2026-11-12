//! RhythmVerse in the app (only in the separate `truss-rv.exe` build): searching it alongside
//! Chorus Encore, and the buttons shown for RhythmVerse charts.

use crate::api::Chart;
use crate::rhythmverse::{self, RvPage};
use crate::rv_catalog::{self, SyncEvent};
use crate::{App, Message};
use iced::widget::{button, column, row, text, toggler};
use iced::{Element, Length, Task};
use std::collections::HashSet;
use std::sync::Arc;

/// RhythmVerse search state for the current search.
#[derive(Default)]
pub struct RvState {
    pub found: u64,
    pub seen: u64,
    pub page: u32,
    pub loading: bool,
    /// Results that arrived before Chorus Encore's page; added after it (minus duplicates).
    pub buffer: Vec<Arc<Chart>>,
    /// Chorus Encore results waiting to be interleaved by date (see `merging`).
    pub encore_buffer: Vec<Arc<Chart>>,
    /// Why RhythmVerse charts are left out of the current Advanced search, if they are.
    pub hidden_note: Option<String>,
    /// Whose turn it is when undated RhythmVerse charts are mixed in (see `rv_undated`).
    pub alternate: bool,
    /// Local catalogue status, and sync progress (page, pages) while one runs.
    pub catalog: rv_catalog::Status,
    pub syncing: Option<(u32, u32)>,
    pub sync_error: Option<String>,
}

impl Default for rv_catalog::Status {
    fn default() -> Self {
        Self { charts: 0, ready: false, last_sync: None }
    }
}

/// RhythmVerse dates that don't say when a chart was uploaded: blank, or 3-6 November 2023,
/// when ~45,000 charts (about half the list) all got the same few dates, apparently from a
/// move to RhythmVerse's current site. Sorting by these would show tens of thousands of
/// RhythmVerse charts in a row once the list reached that date.
fn rv_undated(date: &str) -> bool {
    let day = &date[..date.len().min(10)];
    day.is_empty() || ("2023-11-03"..="2023-11-06").contains(&day)
}

/// Results per page when searching the local catalogue.
const LOCAL_PAGE: u32 = 50;
/// Re-sync on start when the catalogue is older than this.
const STALE_SECS: i64 = 6 * 3600;

impl App {
    pub fn rv_enabled(&self) -> bool {
        self.settings.source_rhythmverse
    }

    /// Starts the RhythmVerse part of a search (or the next page of it).
    pub fn rv_search(&mut self, append: bool) -> Task<Message> {
        // Advanced search: RhythmVerse's chart list only has names, length, difficulties and
        // dates, so it joins in only when every filter set is one it can answer.
        let skip = match &self.active_advanced {
            _ if !self.rv_enabled() => Some(None),
            None => None,
            Some(_) if self.fw.filter.is_some() => Some(Some("the Fretwork filter only works on Encore charts".to_string())),
            Some(a) => match a.rv_unsupported() {
                Some(what) => Some(Some(format!("{what} isn't available for them"))),
                None if !self.rv.catalog.ready => Some(Some("their chart list hasn't finished copying yet".into())),
                None => None,
            },
        };
        if !append {
            self.rv.hidden_note = skip.clone().flatten();
        }
        if skip.is_some() {
            if !append {
                self.reset_rv_search(0);
            }
            return Task::none();
        }
        if append {
            if self.rv.loading || self.rv.seen >= self.rv.found {
                return Task::none();
            }
            self.rv.page += 1;
        } else {
            self.reset_rv_search(1);
        }
        self.rv.loading = true;
        let generation = self.generation;
        if self.rv.catalog.ready {
            // Instant local search once the catalogue has been copied.
            let (query, sort, inst, page) = (self.query.clone(), self.sort, self.instrument.key.map(String::from), self.rv.page);
            let advanced = self.active_advanced.clone();
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        rv_catalog::search(&query, sort, inst.as_deref(), advanced.as_deref(), page, LOCAL_PAGE)
                            .map(|(charts, total)| RvPage { charts, total })
                    })
                    .await
                    .map_err(|e| e.to_string())?
                },
                move |result| Message::RvDone { generation, append, result: result.map(Arc::new) },
            );
        }
        Task::perform(
            rhythmverse::search(
                self.query.clone(),
                self.rv.page,
                self.sort,
                self.instrument.key.map(String::from),
                self.difficulty.key.map(String::from),
            ),
            move |result| Message::RvDone { generation, append, result: result.map(Arc::new) },
        )
    }

    fn reset_rv_search(&mut self, page: u32) {
        self.rv.found = 0;
        self.rv.seen = 0;
        self.rv.page = page;
        self.rv.loading = false;
        self.rv.buffer.clear();
        self.rv.encore_buffer.clear();
        self.rv.alternate = false;
    }

    /// Both sources list newest-first (blank search, default sort), so their results are
    /// interleaved by upload date like Encore's own listing. Typed searches keep Encore's
    /// relevance order, with RhythmVerse's results after.
    pub fn merging(&self) -> bool {
        self.rv_enabled()
            && self.settings.source_encore
            && self.sort.is_none()
            && self.query.trim().is_empty()
            && self.active_advanced.is_none()
    }

    /// Encore results arrived while merging: hold them until they can be placed by date.
    pub fn rv_encore_page(&mut self, data: Vec<Arc<Chart>>) {
        if data.is_empty() {
            self.encore_seen = self.found;
        }
        self.encore_seen += data.len() as u64;
        self.rv.encore_buffer.extend(data);
        self.rv_flush();
    }

    /// Whether held-back results are still waiting to be shown.
    pub fn rv_pending(&self) -> bool {
        !self.rv.buffer.is_empty() || !self.rv.encore_buffer.is_empty()
    }

    /// Updates the local catalogue on start if it's missing or stale.
    pub fn rv_auto_sync(&mut self) -> Task<Message> {
        self.rv.catalog = rv_catalog::status();
        let stale = self.rv.catalog.last_sync.is_none_or(|t| {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
            now - t > STALE_SECS
        });
        if self.rv_enabled() && (!self.rv.catalog.ready || stale) { self.rv_start_sync() } else { Task::none() }
    }

    pub fn rv_start_sync(&mut self) -> Task<Message> {
        if self.rv.syncing.is_some() {
            return Task::none();
        }
        self.rv.syncing = Some((0, 0));
        self.rv.sync_error = None;
        Task::run(iced::stream::channel(16, async move |tx| rv_catalog::sync(tx).await), Message::RvSync)
    }

    pub fn rv_sync_event(&mut self, e: SyncEvent) -> Task<Message> {
        match e {
            SyncEvent::Progress { page, pages } => {
                self.rv.syncing = Some((page, pages));
                Task::none()
            }
            SyncEvent::Finished(r) => {
                self.rv.syncing = None;
                let was_ready = self.rv.catalog.ready;
                self.rv.catalog = rv_catalog::status();
                if let Err(e) = r {
                    self.rv.sync_error = Some(e);
                }
                // The first full copy just finished: show RhythmVerse results from it.
                if !was_ready && self.rv.catalog.ready { self.search(false) } else { Task::none() }
            }
        }
    }

    pub fn rv_done(&mut self, generation: u64, append: bool, result: Result<Arc<RvPage>, String>) -> Task<Message> {
        if generation != self.generation {
            return Task::none();
        }
        self.rv.loading = false;
        match result {
            Ok(page) => {
                self.rv.found = page.total;
                self.rv.seen += page.charts.len() as u64;
                if page.charts.is_empty() {
                    self.rv.seen = self.rv.found;
                }
                self.rv.buffer.extend(page.charts.iter().cloned());
                if !self.loading || self.merging() {
                    self.rv_flush();
                }
                self.refresh_art_window()
            }
            Err(e) => {
                if append {
                    self.rv.page -= 1;
                }
                if self.merging() {
                    // Stop waiting on RhythmVerse so Encore's results aren't held back.
                    self.rv.found = self.rv.seen;
                    self.rv_flush();
                }
                if self.results.is_empty() && !self.loading {
                    self.error = Some(e);
                }
                Task::none()
            }
        }
    }

    /// Adds buffered RhythmVerse results, skipping charts Chorus Encore already listed.
    pub fn rv_flush(&mut self) {
        if self.merging() {
            return self.merge_flush();
        }
        if self.rv.buffer.is_empty() {
            return;
        }
        let have: HashSet<String> = self.results.iter().map(|c| c.library_key()).collect();
        let fresh: Vec<Arc<Chart>> = std::mem::take(&mut self.rv.buffer).into_iter().filter(|c| !have.contains(&c.library_key())).collect();
        if !fresh.is_empty() {
            self.error = None;
            self.ingest(fresh);
        }
    }

    /// Shows held results newest-first, as far as both sources' pages agree: a chart is only
    /// placed once the other source has a page reaching back to its date (or has no more).
    fn merge_flush(&mut self) {
        // Encore uses "…T12:00:00.000Z", RhythmVerse "…T12:00:00" (also UTC): compare to the second.
        fn date(c: &Chart) -> &str {
            c.modified_time.as_deref().map(|d| &d[..d.len().min(19)]).unwrap_or("")
        }
        let encore_done = !self.loading && self.encore_seen >= self.found;
        let rv_done = !self.rv.loading && self.rv.seen >= self.rv.found;
        let (mut e, mut r) = (std::mem::take(&mut self.rv.encore_buffer).into_iter().peekable(), std::mem::take(&mut self.rv.buffer).into_iter().peekable());
        let mut have: HashSet<String> = self.results.iter().map(|c| c.library_key()).collect();
        let mut out = Vec::new();
        loop {
            let take_rv = match (e.peek(), r.peek()) {
                // Undated RhythmVerse charts take turns with Encore's instead of piling up.
                (Some(_), Some(b)) if rv_undated(date(b)) => {
                    self.rv.alternate = !self.rv.alternate;
                    self.rv.alternate
                }
                (Some(a), Some(b)) => date(b) > date(a),
                (Some(_), None) if rv_done => false,
                (None, Some(_)) if encore_done => true,
                _ => break,
            };
            if take_rv {
                let c = r.next().unwrap();
                // Skip charts Encore also has (Encore's copy supports preview).
                if have.insert(c.library_key()) {
                    out.push(c);
                }
            } else {
                let c = e.next().unwrap();
                have.insert(c.library_key());
                out.push(c);
            }
        }
        let shown: HashSet<String> = out.iter().map(|c| c.library_key()).collect();
        self.rv.encore_buffer = e.collect();
        // RhythmVerse charts held back whose Encore copy is already showing are dropped.
        self.rv.buffer = r.filter(|c| !shown.contains(&c.library_key())).collect();
        if !out.is_empty() {
            self.error = None;
            self.ingest(out);
        }
    }

    pub fn rv_more(&self) -> bool {
        self.rv_enabled() && self.rv.seen < self.rv.found
    }

    pub fn rv_settings_section(&self) -> Element<'static, Message> {
        let cat = &self.rv.catalog;
        let status = match (self.rv.syncing, &self.rv.sync_error) {
            (Some((page, pages)), _) if pages > 0 => {
                format!("Copying RhythmVerse's chart list: page {page} of {pages}. Searches use the live site until it's done.")
            }
            (Some(_), _) => "Updating RhythmVerse's chart list…".to_string(),
            (None, Some(e)) => format!("Couldn't update the chart list: {e}"),
            (None, None) if cat.ready => {
                let ago = cat.last_sync.map(|t| {
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(t);
                    let m = (now - t).max(0) / 60;
                    if m < 1 { "just now".to_string() } else if m < 60 { format!("{m} min ago") } else if m < 2880 { format!("{} h ago", m / 60) } else { format!("{} days ago", m / 1440) }
                });
                format!("Local copy: {} charts, updated {}. Searches are instant.", cat.charts, ago.unwrap_or_else(|| "—".into()))
            }
            (None, None) => "No local copy yet: searches use the live site (slower).".to_string(),
        };
        let busy = self.rv.syncing.is_some();
        column![
            row![
                text(status).size(12).width(Length::Fill),
                button(text("Update now").size(12)).padding([4, 10]).style(crate::style::secondary).on_press_maybe((!busy).then_some(Message::RvSyncNow)),
                button(text("Rebuild").size(12)).padding([4, 10]).style(crate::style::secondary).on_press_maybe((!busy).then_some(Message::RvRebuild)),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            toggler(self.settings.source_encore)
                .label("Chorus Encore: search, preview and download charts")
                .on_toggle(Message::ToggleEncore),
            toggler(self.settings.source_rhythmverse)
                .label("RhythmVerse (Experimental)")
                .on_toggle(Message::ToggleRhythmVerse),
        ]
        .spacing(10)
        .into()
    }

    /// The download button for a RhythmVerse chart (or `None` for Encore charts).
    pub fn rv_buttons(&self, i: usize, c: &Chart, size: u32, pad: u16) -> Option<Element<'static, Message>> {
        let rv = c.rv.as_ref()?;
        let dl: Element<'static, Message> = if rhythmverse::can_download(rv) {
            button(text("Download").size(size).width(Length::Fill).center())
                .width(Length::Fill)
                .padding(pad)
                .style(button::primary)
                .on_press(Message::Download(i))
                .into()
        } else {
            let link = rv.external_url.clone().unwrap_or_else(|| rv.page_url.clone());
            let host = rhythmverse::host_name(&link);
            button(text(format!("Open on {host} ↗")).size(size).width(Length::Fill).center())
                .width(Length::Fill)
                .padding(pad)
                .style(crate::style::secondary)
                .on_press(Message::OpenUrl(link))
                .into()
        };
        Some(dl)
    }

    /// RhythmVerse link buttons for the chart details (replaces Encore's links).
    pub fn rv_links(&self, c: &Chart) -> Option<Element<'static, Message>> {
        let rv = c.rv.as_ref()?;
        let mut col = column![text("From RhythmVerse").size(11)].spacing(6);
        let link_text = |s: String| text(s).size(12).wrapping(text::Wrapping::None);
        let mut links = iced::widget::row![
            button(link_text("RhythmVerse page ↗".into())).padding([4, 10]).style(crate::style::secondary).on_press(Message::OpenUrl(rv.page_url.clone())),
            button(link_text("Copy link".into())).padding([4, 10]).style(crate::style::secondary).on_press(Message::CopyText(rv.page_url.clone())),
        ]
        .spacing(6);
        if let Some(ext) = rv.external_url.as_ref().filter(|u| !rhythmverse::is_rv_page(u)) {
            // Friendly names for the common hosts, the bare domain otherwise.
            let host = rhythmverse::host_name(ext);
            let label = match host.as_str() {
                "drive.google.com" | "docs.google.com" => "Google Drive".to_string(),
                "mediafire.com" => "MediaFire".to_string(),
                "mega.nz" | "mega.co.nz" => "MEGA".to_string(),
                "dropbox.com" | "dl.dropboxusercontent.com" => "Dropbox".to_string(),
                _ => host,
            };
            links = links.push(
                button(link_text(format!("{label} ↗")))
                    .padding([4, 10])
                    .style(crate::style::secondary)
                    .on_press(Message::OpenUrl(ext.clone())),
            );
        }
        col = col.push(links.wrap().vertical_spacing(6));
        Some(col.into())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn undated_rhythmverse_dates() {
        assert!(super::rv_undated(""));
        assert!(super::rv_undated("2023-11-05T14:02:11"));
        assert!(super::rv_undated("2023-11-03"));
        assert!(super::rv_undated("2023-11-06T23:59:59"));
        assert!(!super::rv_undated("2023-11-07T00:00:00"));
        assert!(!super::rv_undated("2023-11-02T23:59:59"));
        assert!(!super::rv_undated("2026-11-12T10:35:08"));
    }
}
