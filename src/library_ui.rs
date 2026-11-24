//! The Library page: every chart in your library folders, with filters for missing assets and
//! duplicates, search, multi-select and delete (to the Recycle Bin).

use crate::library::{ChartType, LocalChart};
use crate::vlist::VList;
use iced::widget::{button, column, container, pick_list, row, rule, scrollable, space, text, text_input};
use iced::{Alignment, Background, Element, Length, Theme};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    NoVideo,
    NoBackground,
    NoArt,
    NoLyrics,
    Duplicates,
    /// Charts you have a score on, and ones you've full-combo'd.
    Played,
    FullCombo,
    /// Played, not full-combo'd, but with few misses (fewest first).
    AlmostFc,
    /// Not played, at the difficulty most of your full combos are at (or a step up).
    YourLevel,
    /// The same chart file more than once.
    ExactCopies,
}

/// Your level: the Fretwork tier three quarters of your Expert guitar full combos are at or below, so a handful of easy ones don't pull it down.
fn your_level(fc_diffs: &mut [u32]) -> Option<u32> {
    if fc_diffs.len() < 5 {
        return None;
    }
    fc_diffs.sort_unstable();
    Some(fc_diffs[(fc_diffs.len() * 3) / 4])
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KindOpt(pub Option<ChartType>);
impl KindOpt {
    const ALL: [KindOpt; 4] =
        [KindOpt(None), KindOpt(Some(ChartType::Chart)), KindOpt(Some(ChartType::Mid)), KindOpt(Some(ChartType::Sng))];
}
impl std::fmt::Display for KindOpt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            None => "All types",
            Some(ChartType::Chart) => ".chart",
            Some(ChartType::Mid) => ".mid",
            Some(ChartType::Sng) => ".sng",
            Some(ChartType::None) => "No chart file",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Artist,
    Song,
    Charter,
    Newest,
    Oldest,
}

impl SortBy {
    const ALL: [SortBy; 5] = [SortBy::Artist, SortBy::Song, SortBy::Charter, SortBy::Newest, SortBy::Oldest];

    /// How it's saved in the settings.
    fn key(self) -> &'static str {
        match self {
            SortBy::Artist => "artist",
            SortBy::Song => "song",
            SortBy::Charter => "charter",
            SortBy::Newest => "newest",
            SortBy::Oldest => "oldest",
        }
    }
}

impl std::fmt::Display for SortBy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SortBy::Artist => "Artist A–Z",
            SortBy::Song => "Song A–Z",
            SortBy::Charter => "Charter A–Z",
            SortBy::Newest => "Recently added",
            SortBy::Oldest => "Oldest added",
        })
    }
}

/// A chart's date added, as shown in the list.
pub(crate) fn added_date(secs: u64) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub enum Msg {
    Query(String),
    Filter(Filter),
    Kind(KindOpt),
    Sort(SortBy),
    /// Picked from the Sort menu (a header click toggles instead).
    SortPicked(SortBy),
    Toggle(usize),
    /// Opens (or closes) the details below a chart's row.
    Expand(usize),
    SelectAllShown,
    ClearSelection,
    Delete,
    Rescan,
    Open(PathBuf),
    Scrolled(scrollable::Viewport),
    /// Look for loose charts that are also in a pack.
    FindPackDuplicates,
    /// Remove every video background (asks again first).
    RemoveVideos,
    /// Remove the spare exact copies (asks again first).
    RemoveCopies,
    /// The Near FC limit was typed.
    NearFc(String),
}

/// Requests for the app.
pub enum Out {
    None,
    /// The sort order changed (its settings key), to remember it.
    Sorted(String),
    /// The Near FC limit changed, to remember it.
    NearFc(u32),
    Rescan,
    Delete(Vec<PathBuf>),
    Open(PathBuf),
    /// The chart whose details are open below its row changed (`None`: closed).
    Expanded(Option<Arc<LocalChart>>),
    FindPackDuplicates,
    RemoveVideos,
}

/// What the page reads from the app.
pub struct Ctx<'a> {
    pub catalog: &'a [Arc<LocalChart>],
    /// Goes up each time the catalog is replaced.
    pub catalog_rev: u64,
    /// Which charts have lyrics (filled in by a background check).
    pub lyrics: &'a HashMap<PathBuf, bool>,
    pub scanning: bool,
    pub lyrics_scanning: bool,
    pub has_folders: bool,
    pub error: Option<&'a str>,
    /// What the last clean-up did, or that one is running.
    pub notice: Option<&'a str>,
    /// A clean-up is running.
    pub busy: bool,
    /// Your Clone Hero scores, once known, and why they couldn't be fetched.
    pub scores: Option<&'a crate::scores::Scores>,
    pub scores_error: Option<&'a str>,
    /// Missed notes a play can have and still be a Near FC.
    pub near_fc_misses: u32,
    /// Fretwork tiers by chart file, and how far working them out is.
    pub tiers: Option<&'a crate::library_fretwork::Tiers>,
    pub rating: Option<(usize, usize)>,
    /// The library folders (charts directly in one are loose; deeper ones are in packs).
    pub roots: Vec<PathBuf>,
}



#[derive(Default)]
pub struct Page {
    query: String,
    filter: Option<Filter>,
    kind: Option<ChartType>,
    sort: Option<SortBy>,
    selected: BTreeSet<PathBuf>,
    /// The chart whose details are open below its row.
    open: Option<PathBuf>,
    confirm_delete: bool,
    confirm_videos: bool,
    list: VList,
    /// Indices into the catalog that pass the filters, in display order.
    shown: Vec<usize>,
    dup_keys: HashSet<String>,
    /// Lowercased "artist name charter album" per catalog entry, for searching.
    hay: Vec<String>,
    /// Each catalog entry's key for matching scores (see `scores::key`).
    score_keys: Vec<String>,
    /// The catalog the above (and `dup_keys`) were worked out for (its `catalog_rev`, plus one:
    /// 0 is nothing yet).
    built_for: u64,
    /// Your best play of each catalog entry (worked out when the catalog or scores change),
    /// and how many you've played and full-combo'd.
    best: Vec<Option<crate::scores::Best>>,
    played: usize,
    full_combos: usize,
    almost_fc: usize,
    /// Your level (see `your_level`), and how many unplayed charts are at it.
    level: Option<u32>,
    /// Each catalog entry's Fretwork tier (Expert guitar), if it has one.
    tiers: Vec<Option<u32>>,
    /// The Near FC box once it's been typed in (None: shows the setting).
    near_text: Option<String>,
    at_level: usize,
    /// Which catalog entries are exact copies of another, and the spare ones to remove.
    copies: Vec<bool>,
    spares: Vec<usize>,
    confirm_copies: bool,
}

impl Page {
    /// A page that starts in the sort order saved in the settings.
    pub fn with_sort(key: &str) -> Self {
        let sort = SortBy::ALL.into_iter().find(|s| s.key() == key).filter(|s| *s != SortBy::Artist);
        Page { sort, ..Default::default() }
    }

    fn sorted(&self) -> Out {
        Out::Sorted(self.sort.map(|s| s.key()).unwrap_or("").to_string())
    }

    /// Call when the catalog or lyrics data changed.
    pub fn refresh(&mut self, cx: &Ctx) {
        // What depends only on the charts is worked out once for each catalog (a refresh for new
        // scores or lyrics reuses it).
        let catalog_id = cx.catalog_rev + 1;
        if self.built_for != catalog_id || self.hay.len() != cx.catalog.len() {
            self.built_for = catalog_id;
            self.hay = cx
                .catalog
                .iter()
                .map(|c| format!("{} {} {} {}", c.artist, c.name, c.charter, c.album).to_lowercase())
                .collect();
            self.score_keys = cx.catalog.iter().map(|c| crate::scores::key(&c.artist, &c.name, &c.charter)).collect();
            let mut count: HashMap<&str, usize> = HashMap::new();
            for c in cx.catalog {
                if c.key != "||" {
                    *count.entry(c.key.as_str()).or_default() += 1;
                }
            }
            self.dup_keys = count.into_iter().filter(|(_, n)| *n > 1).map(|(k, _)| k.to_string()).collect();
        }
        self.best = self.score_keys.iter().map(|k| cx.scores.and_then(|s| s.best_for_key(k)).cloned()).collect();
        self.played = self.best.iter().flatten().count();
        self.full_combos = self.best.iter().flatten().filter(|b| b.is_fc).count();
        self.almost_fc = self.best.iter().flatten().filter(|b| b.near_fc(cx.near_fc_misses)).count();
        self.tiers = cx.catalog.iter().map(|c| cx.tiers?.get(c.chart_hash.as_deref()?).copied().flatten()).collect();
        let mut fc_diffs: Vec<u32> = self
            .tiers
            .iter()
            .zip(&self.best)
            .filter(|(_, b)| b.as_ref().is_some_and(|b| b.is_fc && b.instrument == "guitar" && b.difficulty == "expert"))
            .filter_map(|(t, _)| *t)
            .collect();
        self.level = your_level(&mut fc_diffs);
        (self.copies, self.spares) = crate::library_tools::exact_copies(cx.catalog, &cx.roots, cx.lyrics);
        self.at_level = (0..cx.catalog.len()).filter(|&i| self.at_your_level(i)).count();
        let existing: HashSet<&PathBuf> = cx.catalog.iter().map(|c| &c.path).collect();
        self.selected.retain(|p| existing.contains(p));
        if self.open.as_ref().is_some_and(|p| !existing.contains(p)) {
            self.open = None;
        }
        self.apply(cx);
    }

    /// The chart whose details are open, if any.
    pub fn open_chart(&self) -> Option<&PathBuf> {
        self.open.as_ref()
    }

    /// An unplayed guitar chart at your level or a step up.
    fn at_your_level(&self, i: usize) -> bool {
        let Some(level) = self.level else { return false };
        self.best.get(i).is_some_and(Option::is_none) && self.tiers.get(i).copied().flatten().is_some_and(|t| t == level || t == level + 1)
    }

    fn passes(&self, i: usize, cx: &Ctx) -> bool {
        let c = &cx.catalog[i];
        let best = self.best.get(i).and_then(Option::as_ref);
        match self.filter.unwrap_or(Filter::All) {
            Filter::All => true,
            Filter::NoVideo => !c.has_video,
            Filter::NoBackground => !c.has_background,
            Filter::NoArt => !c.has_album_art,
            Filter::NoLyrics => cx.lyrics.get(&c.path) == Some(&false),
            Filter::Duplicates => self.dup_keys.contains(&c.key),
            Filter::Played => best.is_some(),
            Filter::FullCombo => best.is_some_and(|b| b.is_fc),
            Filter::AlmostFc => best.is_some_and(|b| b.near_fc(cx.near_fc_misses)),
            Filter::YourLevel => self.at_your_level(i),
            Filter::ExactCopies => self.copies.get(i).copied().unwrap_or(false),
        }
    }

    pub fn scroll_task<T: Send + 'static>(&mut self) -> iced::Task<T> {
        self.list.take_scroll_task()
    }

    fn apply(&mut self, cx: &Ctx) {
        self.list.set_id("library-list");
        let q = self.query.trim().to_lowercase();
        let words: Vec<&str> = q.split_whitespace().collect();
        let mut shown: Vec<usize> = (0..cx.catalog.len())
            .filter(|&i| {
                let c = &cx.catalog[i];
                self.kind.is_none_or(|k| c.chart_type == k)
                    && self.passes(i, cx)
                    && words.iter().all(|w| self.hay.get(i).is_some_and(|h| h.contains(w)))
            })
            .collect();
        let cat = cx.catalog;
        match self.sort {
            // Newest first; the catalog is already artist-sorted, so ties stay in that order.
            Some(SortBy::Newest) => shown.sort_by_key(|&i| std::cmp::Reverse(cat[i].added)),
            Some(SortBy::Oldest) => shown.sort_by_key(|&i| cat[i].added),
            Some(SortBy::Song) => shown.sort_by_cached_key(|&i| (cat[i].name.to_lowercase(), cat[i].artist.to_lowercase())),
            Some(SortBy::Charter) => {
                shown.sort_by_cached_key(|&i| (cat[i].charter.to_lowercase(), cat[i].artist.to_lowercase()))
            }
            // Duplicates sit next to each other.
            _ if self.filter == Some(Filter::Duplicates) => shown.sort_by_cached_key(|&i| cat[i].key.clone()),
            // Copies of a chart sit together.
            _ if self.filter == Some(Filter::ExactCopies) => shown.sort_by_cached_key(|&i| (cat[i].key.clone(), cat[i].chart_hash.clone())),
            // Fewest misses first.
            _ if self.filter == Some(Filter::AlmostFc) => shown.sort_by_key(|&i| {
                self.best.get(i).and_then(Option::as_ref).map_or(u32::MAX, |b| b.misses())
            }),
            _ => {}
        }
        self.shown = shown;
        self.list.reset();
    }

    pub fn update(&mut self, msg: Msg, cx: &Ctx) -> Out {
        if !matches!(msg, Msg::Delete | Msg::Scrolled(_)) {
            self.confirm_delete = false;
        }
        if !matches!(msg, Msg::RemoveVideos | Msg::Scrolled(_)) {
            self.confirm_videos = false;
        }
        if !matches!(msg, Msg::RemoveCopies | Msg::Scrolled(_)) {
            self.confirm_copies = false;
        }
        match msg {
            Msg::Query(q) => {
                self.query = q;
                self.apply(cx);
            }
            Msg::Filter(f) => {
                self.filter = if self.filter == Some(f) || f == Filter::All { None } else { Some(f) };
                self.apply(cx);
            }
            Msg::Kind(k) => {
                self.kind = k.0;
                self.apply(cx);
            }
            Msg::Sort(s) => {
                self.sort = match (self.sort, s) {
                    // The Added heading flips between newest and oldest first.
                    (Some(SortBy::Newest), SortBy::Newest) => Some(SortBy::Oldest),
                    (Some(SortBy::Oldest), SortBy::Newest) => Some(SortBy::Newest),
                    (cur, s) if cur == Some(s) => None,
                    _ => Some(s),
                };
                self.apply(cx);
                return self.sorted();
            }
            Msg::SortPicked(s) => {
                self.sort = (s != SortBy::Artist).then_some(s);
                self.apply(cx);
                return self.sorted();
            }
            Msg::Toggle(i) => {
                if let Some(c) = cx.catalog.get(i)
                    && !self.selected.remove(&c.path)
                {
                    self.selected.insert(c.path.clone());
                }
            }
            Msg::SelectAllShown => {
                for &i in &self.shown {
                    self.selected.insert(cx.catalog[i].path.clone());
                }
            }
            Msg::Expand(i) => {
                let Some(c) = cx.catalog.get(i) else { return Out::None };
                if self.open.as_ref() == Some(&c.path) {
                    self.open = None;
                    return Out::Expanded(None);
                }
                self.open = Some(c.path.clone());
                return Out::Expanded(Some(c.clone()));
            }
            Msg::ClearSelection => self.selected.clear(),
            Msg::Delete => {
                if !self.confirm_delete {
                    self.confirm_delete = true;
                } else {
                    self.confirm_delete = false;
                    let paths: Vec<PathBuf> = std::mem::take(&mut self.selected).into_iter().collect();
                    return Out::Delete(paths);
                }
            }
            Msg::Rescan => return Out::Rescan,
            Msg::FindPackDuplicates => return Out::FindPackDuplicates,
            Msg::RemoveVideos => {
                if !self.confirm_videos {
                    self.confirm_videos = true;
                } else {
                    self.confirm_videos = false;
                    return Out::RemoveVideos;
                }
            }
            Msg::NearFc(t) => {
                let t: String = t.chars().filter(char::is_ascii_digit).take(4).collect();
                let n = t.parse().ok();
                // Empty while typing: the last limit stays until there's a number.
                self.near_text = Some(t);
                return n.map_or(Out::None, Out::NearFc);
            }
            Msg::RemoveCopies => {
                if !self.confirm_copies {
                    self.confirm_copies = true;
                } else {
                    self.confirm_copies = false;
                    return Out::Delete(self.spares.iter().map(|&i| cx.catalog[i].path.clone()).collect());
                }
            }
            Msg::Open(p) => return Out::Open(p),
            Msg::Scrolled(vp) => self.list.on_scroll(vp),
        }
        Out::None
    }

    /// The page; `details` (its height, and the widget) is drawn below the open chart's row.
    pub fn view<'a>(&'a self, cx: &Ctx<'a>, details: Option<(f32, Element<'a, crate::Message>)>) -> Element<'a, crate::Message> {
        self.view_inner(cx, details)
    }

    fn view_inner<'a>(&'a self, cx: &Ctx<'a>, details: Option<(f32, Element<'a, crate::Message>)>) -> Element<'a, crate::Message> {
        use crate::Message::LibraryPage as M;
        if !cx.has_folders {
            return container(
                column![
                    text("No library folders set").size(18),
                    text("Add your Clone Hero / YARG songs folders in Settings.").size(13),
                ]
                .spacing(8)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .into();
        }
        let cat = cx.catalog;
        let open_row = self.open.as_ref().and_then(|p| self.shown.iter().position(|&i| &cat[i].path == p));
        let lyrics_known = !cx.lyrics.is_empty();
        let count = |f: fn(&LocalChart) -> bool| cat.iter().filter(|c| f(c)).count();
        let no_lyrics = cat.iter().filter(|c| cx.lyrics.get(&c.path) == Some(&false)).count();
        let dups = cat.iter().filter(|c| self.dup_keys.contains(&c.key)).count();
        let tile = |label: &'static str, n: String, f: Filter| {
            let active = self.filter.unwrap_or(Filter::All) == f;
            let number = text(n).size(20).style(move |t: &Theme| text::Style {
                color: active.then(|| crate::style::accent(t)),
            });
            let caption = text(label).size(11).style(move |t: &Theme| text::Style {
                color: Some(crate::style::muted(t)),
            });
            button(column![number, caption].align_x(Alignment::Center).width(Length::Fill))
                .width(Length::Fill)
                .padding([8, 6])
                .style(move |t: &Theme, s| {
                    // Outlined; the current filter gets an accent outline and number.
                    let mut st = crate::style::quiet_button(false)(t, s);
                    if active {
                        st.border.color = crate::style::accent(t);
                    }
                    st
                })
                .on_press(Msg::Filter(f))
        };
        let tiles = row![
            tile("Total", cat.len().to_string(), Filter::All),
            tile("No video", count(|c| !c.has_video).to_string(), Filter::NoVideo),
            tile("No background", count(|c| !c.has_background).to_string(), Filter::NoBackground),
            tile("No album art", count(|c| !c.has_album_art).to_string(), Filter::NoArt),
            tile(
                "No lyrics",
                if lyrics_known { no_lyrics.to_string() } else if cx.lyrics_scanning { "…".into() } else { "—".into() },
                Filter::NoLyrics
            ),
            tile("Duplicates", dups.to_string(), Filter::Duplicates),
            tile("Exact copies", self.spares.len().to_string(), Filter::ExactCopies),
            tile(
                "Played",
                if cx.scores.is_some() { self.played.to_string() } else { "—".into() },
                Filter::Played
            ),
            tile(
                "Full combo",
                if cx.scores.is_some() { self.full_combos.to_string() } else { "—".into() },
                Filter::FullCombo
            ),
            tile("Near FC", if cx.scores.is_some() { self.almost_fc.to_string() } else { "—".into() }, Filter::AlmostFc),
            tile("Your level", if cx.rating.is_some() { "…".into() } else if self.level.is_some() { self.at_level.to_string() } else { "—".into() }, Filter::YourLevel),
        ]
        .spacing(8);

        let status = if cx.scanning {
            "Scanning your library…".to_string()
        } else if let Some(e) = cx.error {
            e.to_string()
        } else if let Some(e) = cx.scores_error.filter(|_| matches!(self.filter, Some(Filter::Played | Filter::FullCombo | Filter::AlmostFc | Filter::YourLevel))) {
            e.to_string()
        } else if self.filter == Some(Filter::ExactCopies) {
            format!("{} spare loose copies (packs are left alone)", self.spares.len())
        } else if self.filter == Some(Filter::AlmostFc) {
            format!("{} charts", self.shown.len())
        } else if let (Some(Filter::YourLevel), Some((done, total))) = (self.filter, cx.rating) {
            if total == 0 { "Reading Fretwork tiers…".to_string() } else { format!("Rating your charts: {done} of {total} (once)") }
        } else if let (Some(Filter::YourLevel), Some(l)) = (self.filter, self.level) {
            format!("{} unplayed, Fretwork {l}–{}", self.shown.len(), l + 1)
        } else if self.filter == Some(Filter::YourLevel) {
            "FC five Expert guitar charts to find your level".to_string()
        } else {
            format!("{} of {} charts", self.shown.len(), cat.len())
        };
        // The status takes the room the buttons leave, and is cut short rather than squeezing them.
        let mut bar = row![
            text_input("Search artist, song, charter, album…", &self.query).on_input(Msg::Query).padding(8).width(320),
            pick_list(KindOpt::ALL, Some(KindOpt(self.kind)), Msg::Kind).padding(8),
            pick_list(SortBy::ALL, Some(self.sort.unwrap_or(SortBy::Artist)), Msg::SortPicked).padding(8),
            container(text(status).size(12).wrapping(text::Wrapping::None)).width(Length::Fill).clip(true),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if !self.selected.is_empty() {
            let n = self.selected.len();
            bar = bar
                .push(text(format!("{n} selected")).size(13).wrapping(text::Wrapping::None))
                .push(
                    button(text(if self.confirm_delete { format!("Confirm: delete {n}") } else { format!("Delete {n}") }).size(13).wrapping(text::Wrapping::None))
                        .padding([6, 12])
                        .style(move |t: &Theme, s| if self.confirm_delete { button::danger(t, s) } else { button::secondary(t, s) })
                        .on_press(Msg::Delete),
                )
                .push(button(text("Clear").size(13).wrapping(text::Wrapping::None)).padding([6, 12]).style(crate::style::secondary).on_press(Msg::ClearSelection));
        } else {
            bar = bar.push(
                button(text("Select all shown").size(13).wrapping(text::Wrapping::None)).padding([6, 12]).style(crate::style::secondary).on_press(Msg::SelectAllShown),
            );
        }
        if self.filter == Some(Filter::AlmostFc) {
            let shown = self.near_text.clone().unwrap_or_else(|| cx.near_fc_misses.to_string());
            bar = bar
                .push(text("Misses, up to").size(13).wrapping(text::Wrapping::None))
                .push(text_input("", &shown).on_input(Msg::NearFc).padding(6).size(13).width(52));
        }
        if self.filter == Some(Filter::ExactCopies) {
            let n = self.spares.len();
            bar = bar.push(
                button(text(if self.confirm_copies { format!("Confirm: remove {n}") } else { format!("Remove {n} spare copies") }).size(13).wrapping(text::Wrapping::None))
                    .padding([6, 12])
                    .style(move |t: &Theme, s| if self.confirm_copies { button::danger(t, s) } else { button::secondary(t, s) })
                    .on_press_maybe((n > 0 && !cx.busy && !cx.scanning).then_some(Msg::RemoveCopies)),
            );
        }
        let videos = cat.iter().filter(|c| c.has_video && !c.is_sng()).count();
        let idle = !cx.busy && !cx.scanning;
        bar = bar
            .push(
                button(text("Merge pack duplicates").size(13).wrapping(text::Wrapping::None))
                    .padding([6, 12])
                    .style(crate::style::secondary)
                    .on_press_maybe(idle.then_some(Msg::FindPackDuplicates)),
            )
            .push(
                button(
                    text(if self.confirm_videos { format!("Confirm: remove {videos} videos") } else { "Remove videos".to_string() })
                        .size(13)
                        .wrapping(text::Wrapping::None),
                )
                .padding([6, 12])
                .style(move |t: &Theme, s| if self.confirm_videos { button::danger(t, s) } else { button::secondary(t, s) })
                .on_press_maybe((idle && videos > 0).then_some(Msg::RemoveVideos)),
            );
        bar = bar.push(button(text("Rescan").size(13).wrapping(text::Wrapping::None)).padding([6, 12]).on_press(Msg::Rescan));
        // What the last clean-up did, on its own line.
        let top: Element<'_, Msg> = match cx.notice.filter(|_| !cx.scanning) {
            Some(n) => column![tiles, bar, text(n.to_string()).size(12).style(|t: &Theme| text::Style { color: Some(crate::style::muted(t)) })]
                .spacing(10)
                .into(),
            None => column![tiles, bar].spacing(10).into(),
        };

        // Header and rows share these widths, so every column lines up with its heading.
        const CHECK_W: f32 = 22.0;
        const TYPE_W: f32 = 56.0;
        const ADDED_W: f32 = 84.0;
        const ASSET_W: f32 = 48.0;
        const OPEN_W: f32 = 56.0;
        const SCORE_W: f32 = 92.0;
        const GAP: f32 = 10.0;
        // Right side leaves room for the scrollbar, which floats over the list.
        let row_pad = iced::Padding { top: 0.0, right: 26.0, bottom: 0.0, left: 12.0 };

        let sort_head = |label: &'static str, s: SortBy, width: Length| {
            let arrow = match (self.sort, s) {
                (Some(SortBy::Newest), SortBy::Newest) => " ▼",
                (Some(SortBy::Oldest), SortBy::Newest) => " ▲",
                (cur, s) if cur == Some(s) => " ▲",
                _ => "",
            };
            button(text(format!("{label}{arrow}")).size(12).font(crate::BOLD))
                .padding(0)
                .width(width)
                .style(|t: &Theme, st| button::Style {
                    text_color: if matches!(st, button::Status::Hovered) {
                        t.extended_palette().primary.strong.color
                    } else {
                        t.extended_palette().background.base.text
                    },
                    ..Default::default()
                })
                .on_press(Msg::Sort(s))
        };
        let fixed_head = |label: &'static str, w: f32| text(label).size(12).font(crate::BOLD).width(w).center();
        let head = container(
            row![
                space().width(CHECK_W),
                sort_head("Artist", SortBy::Artist, Length::FillPortion(3)),
                sort_head("Song", SortBy::Song, Length::FillPortion(4)),
                sort_head("Charter", SortBy::Charter, Length::FillPortion(2)),
                sort_head("Added", SortBy::Newest, Length::Fixed(ADDED_W)),
                fixed_head("Type", TYPE_W),
                fixed_head("Art", ASSET_W),
                fixed_head("BG", ASSET_W),
                fixed_head("Video", ASSET_W),
                fixed_head("Lyrics", ASSET_W),
                fixed_head("Score", SCORE_W),
                space().width(OPEN_W),
            ]
            .spacing(GAP)
            .align_y(Alignment::Center),
        )
        .padding(iced::Padding { top: 8.0, bottom: 8.0, ..row_pad });

        let list = self.list.view(
            self.shown.len(),
            |r| {
                let i = self.shown[r];
                let c = &cat[i];
                let sel = self.selected.contains(&c.path);
                let is_open = open_row == Some(r);
                let mark = |b: Option<bool>| {
                    let (glyph, ok) = match b {
                        Some(true) => ("✓", Some(true)),
                        Some(false) => ("✗", Some(false)),
                        None => ("·", None),
                    };
                    text(glyph).size(13).width(ASSET_W).center().style(move |t: &Theme| {
                        let p = t.extended_palette();
                        text::Style {
                            color: Some(match ok {
                                Some(true) => p.success.base.color,
                                _ => p.background.base.text.scale_alpha(0.45),
                            }),
                        }
                    })
                };
                // `raw`: the song.ini text with its colour tags, when it has any.
                let cell = |s: &str, raw: Option<&str>, p: u16| {
                    let content: Element<'_, Msg> = match raw.filter(|r| crate::rich::is_styled(r)) {
                        Some(r) => crate::rich::view(r, 13, iced::Font::DEFAULT, None),
                        None => text(s.to_string()).size(13).wrapping(text::Wrapping::None).into(),
                    };
                    container(content).width(Length::FillPortion(p)).clip(true)
                };
                let tagged = c.tagged.as_deref();
                let dupes = matches!(self.filter, Some(Filter::Duplicates | Filter::ExactCopies));
                let open = if c.is_sng() { c.path.parent().map(|p| p.to_path_buf()).unwrap_or_default() } else { c.path.clone() };
                let song = if dupes {
                    // Show where each copy is, so you can tell them apart.
                    c.path
                        .parent()
                        .and_then(|p| p.file_name())
                        .map(|n| format!("{}   — in {}", c.name, n.to_string_lossy()))
                        .unwrap_or_else(|| c.name.clone())
                } else {
                    c.name.clone()
                };
                let row: Element<'a, Msg> = button(
                    row![
                        button(text(if sel { "☑" } else { "☐" }).size(14).width(CHECK_W).center())
                            .padding(0)
                            .style(button::text)
                            .on_press(Msg::Toggle(i)),
                        cell(&c.artist, tagged.map(|t| t.artist.as_str()), 3),
                        cell(&song, tagged.filter(|_| !dupes && !c.name.is_empty()).map(|t| t.name.as_str()), 4),
                        cell(&c.charter, tagged.map(|t| t.charter.as_str()), 2),
                        text(added_date(c.added)).size(12).width(ADDED_W),
                        text(c.chart_type.label()).size(12).width(TYPE_W).center(),
                        mark(Some(c.has_album_art)),
                        mark(Some(c.has_background)),
                        mark(Some(c.has_video)),
                        mark(cx.lyrics.get(&c.path).copied()),
                        score_cell(self.best.get(i).and_then(Option::as_ref), self.filter == Some(Filter::AlmostFc), SCORE_W),
                        container(button(text("Open").size(11)).padding([2, 8]).style(button::text).on_press(Msg::Open(open)))
                            .width(OPEN_W)
                            .center_x(OPEN_W),
                    ]
                    .spacing(GAP)
                    .height(Length::Fill)
                    .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(row_pad)
                .on_press(Msg::Expand(i))
                .style(move |t: &Theme, s| {
                    let p = t.extended_palette();
                    button::Style {
                        background: match (sel, s) {
                            (true, _) => Some(Background::Color(crate::style::selected(t).scale_alpha(0.5))),
                            _ if is_open => Some(Background::Color(crate::style::weak(t))),
                            (false, button::Status::Hovered) => Some(Background::Color(crate::style::weak(t))),
                            _ if r % 2 == 1 => Some(Background::Color(crate::style::weakest(t))),
                            _ => None,
                        },
                        text_color: p.background.base.text,
                        ..Default::default()
                    }
                })
                .into();
                row.map(M)
            },
            open_row.zip(details).map(|(r, (h, d))| {
                let d = container(d)
                    .padding(iced::Padding { top: 10.0, right: 26.0, bottom: 14.0, left: 12.0 + CHECK_W + GAP })
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(|t: &Theme| container::Style { background: Some(crate::style::weakest(t).into()), ..Default::default() });
                (r, h, Element::from(d))
            }),
            |vp| M(Msg::Scrolled(vp)),
        );

        column![
            Element::from(container(top).padding([12, 14])).map(M),
            Element::from(head).map(M),
            rule::horizontal(1).style(crate::style::rule),
            list,
        ]
        .height(Length::Fill)
        .into()
    }
}

/// Your best play of a chart, in its row: a full-combo badge or the accuracy, the stars, and
/// the instrument when it isn't guitar.
fn score_cell<'a>(best: Option<&crate::scores::Best>, misses: bool, width: f32) -> Element<'a, Msg> {
    let Some(b) = best else {
        return text("·").size(13).width(width).center().style(|t: &Theme| text::Style { color: Some(t.extended_palette().background.base.text.scale_alpha(0.45)) }).into();
    };
    let lead = if b.is_pfc {
        "PFC".to_string()
    } else if b.is_fc {
        "FC".to_string()
    } else if misses {
        format!("{} missed", b.misses())
    } else {
        format!("{}%", b.accuracy)
    };
    let inst = match b.instrument.as_str() {
        "guitar" | "" => String::new(),
        "drums" => "Drums ".into(),
        other => format!("{} ", crate::leaderboard::instrument_label(other)),
    };
    let fc = b.is_fc;
    text(format!("{inst}{lead} ★{}", b.stars))
        .size(12)
        .width(width)
        .center()
        .wrapping(text::Wrapping::None)
        .style(move |t: &Theme| text::Style { color: fc.then(|| t.extended_palette().success.base.color) })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_by_date_added_and_remembers_it() {
        let cat: Vec<Arc<LocalChart>> = [("A", "One", 30), ("B", "Two", 10), ("C", "Three", 20)]
            .into_iter()
            .map(|(a, n, t)| Arc::new(crate::library::test_chart(a, n, t)))
            .collect();
        let lyrics = HashMap::new();
        let cx = Ctx {
            catalog: &cat,
            catalog_rev: 0,
            lyrics: &lyrics,
            scanning: false,
            lyrics_scanning: false,
            has_folders: true,
            error: None,
            notice: None,
            busy: false,
            scores: None,
            scores_error: None,
            near_fc_misses: 5,
            tiers: None,
            rating: None,
            roots: Vec::new(),
        };
        let names = |p: &Page| p.shown.iter().map(|&i| cat[i].name.clone()).collect::<Vec<_>>();

        let mut p = Page::with_sort("newest");
        p.refresh(&cx);
        assert_eq!(names(&p), ["One", "Three", "Two"]);

        // The Added heading flips to oldest first, and the change is reported for saving.
        assert!(matches!(p.update(Msg::Sort(SortBy::Newest), &cx), Out::Sorted(k) if k == "oldest"));
        assert_eq!(names(&p), ["Two", "Three", "One"]);

        assert!(matches!(p.update(Msg::SortPicked(SortBy::Artist), &cx), Out::Sorted(k) if k.is_empty()));
        assert_eq!(names(&p), ["One", "Two", "Three"]);
    }
}
