//! The Library page: every chart in your library folders, with filters for missing assets and
//! duplicates, search, multi-select and delete (to the Recycle Bin).

use crate::library::{ChartType, LocalChart};
use crate::vlist::VList;
use iced::widget::{button, column, container, pick_list, row, rule, scrollable, space, text, text_input};
use iced::{Alignment, Background, Border, Element, Length, Theme};
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
}

#[derive(Debug, Clone)]
pub enum Msg {
    Query(String),
    Filter(Filter),
    Kind(KindOpt),
    Sort(SortBy),
    Toggle(usize),
    SelectAllShown,
    ClearSelection,
    Delete,
    Rescan,
    Open(PathBuf),
    Scrolled(scrollable::Viewport),
}

/// Requests for the app.
pub enum Out {
    None,
    Rescan,
    Delete(Vec<PathBuf>),
    Open(PathBuf),
}

/// What the page reads from the app.
pub struct Ctx<'a> {
    pub catalog: &'a [Arc<LocalChart>],
    /// Which charts have lyrics (filled in by a background check).
    pub lyrics: &'a HashMap<PathBuf, bool>,
    pub scanning: bool,
    pub lyrics_scanning: bool,
    pub has_folders: bool,
    pub error: Option<&'a str>,
}

#[derive(Default)]
pub struct Page {
    query: String,
    filter: Option<Filter>,
    kind: Option<ChartType>,
    sort: Option<SortBy>,
    selected: BTreeSet<PathBuf>,
    confirm_delete: bool,
    list: VList,
    /// Indices into the catalog that pass the filters, in display order.
    shown: Vec<usize>,
    dup_keys: HashSet<String>,
    /// Lowercased "artist name charter album" per catalog entry, for searching.
    hay: Vec<String>,
}

impl Page {
    /// Call when the catalog or lyrics data changed.
    pub fn refresh(&mut self, cx: &Ctx) {
        self.hay = cx
            .catalog
            .iter()
            .map(|c| format!("{} {} {} {}", c.artist, c.name, c.charter, c.album).to_lowercase())
            .collect();
        let mut count: HashMap<&str, usize> = HashMap::new();
        for c in cx.catalog {
            if c.key != "||" {
                *count.entry(c.key.as_str()).or_default() += 1;
            }
        }
        self.dup_keys = count.into_iter().filter(|(_, n)| *n > 1).map(|(k, _)| k.to_string()).collect();
        let existing: HashSet<&PathBuf> = cx.catalog.iter().map(|c| &c.path).collect();
        self.selected.retain(|p| existing.contains(p));
        self.apply(cx);
    }

    fn passes(&self, c: &LocalChart, cx: &Ctx) -> bool {
        match self.filter.unwrap_or(Filter::All) {
            Filter::All => true,
            Filter::NoVideo => !c.has_video,
            Filter::NoBackground => !c.has_background,
            Filter::NoArt => !c.has_album_art,
            Filter::NoLyrics => cx.lyrics.get(&c.path) == Some(&false),
            Filter::Duplicates => self.dup_keys.contains(&c.key),
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
                    && self.passes(c, cx)
                    && words.iter().all(|w| self.hay.get(i).is_some_and(|h| h.contains(w)))
            })
            .collect();
        let key = |c: &LocalChart| match self.sort {
            Some(SortBy::Song) => (c.name.to_lowercase(), c.artist.to_lowercase()),
            Some(SortBy::Charter) => (c.charter.to_lowercase(), c.artist.to_lowercase()),
            _ => (c.artist.to_lowercase(), c.name.to_lowercase()),
        };
        if self.filter == Some(Filter::Duplicates) || self.sort.is_some() {
            // Duplicates sit next to each other; other sorts on request (catalog is artist-sorted).
            shown.sort_by_cached_key(|&i| {
                let c = &cx.catalog[i];
                if self.filter == Some(Filter::Duplicates) && self.sort.is_none() { (c.key.clone(), String::new()) } else { key(c) }
            });
        }
        self.shown = shown;
        self.list.reset();
    }

    pub fn update(&mut self, msg: Msg, cx: &Ctx) -> Out {
        if !matches!(msg, Msg::Delete | Msg::Scrolled(_)) {
            self.confirm_delete = false;
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
                self.sort = if self.sort == Some(s) { None } else { Some(s) };
                self.apply(cx);
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
            Msg::Open(p) => return Out::Open(p),
            Msg::Scrolled(vp) => self.list.on_scroll(vp),
        }
        Out::None
    }

    pub fn view<'a>(&'a self, cx: &Ctx<'a>) -> Element<'a, Msg> {
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
        let lyrics_known = !cx.lyrics.is_empty();
        let count = |f: fn(&LocalChart) -> bool| cat.iter().filter(|c| f(c)).count();
        let no_lyrics = cat.iter().filter(|c| cx.lyrics.get(&c.path) == Some(&false)).count();
        let dups = cat.iter().filter(|c| self.dup_keys.contains(&c.key)).count();
        let tile = |label: &'static str, n: String, f: Filter| {
            let active = self.filter.unwrap_or(Filter::All) == f;
            let refined = crate::style::refined();
            let number = text(n).size(20).style(move |t: &Theme| text::Style {
                color: (refined && active).then(|| crate::style::accent(t)),
            });
            let caption = text(label).size(11).style(move |t: &Theme| text::Style {
                color: refined.then(|| crate::style::muted(t)),
            });
            button(column![number, caption].align_x(Alignment::Center).width(Length::Fill))
                .width(Length::Fill)
                .padding([8, 6])
                .style(move |t: &Theme, s| {
                    if refined {
                        // Outlined; the current filter gets an accent outline and number.
                        let mut st = crate::style::quiet_button(false)(t, s);
                        if active {
                            st.border.color = crate::style::accent(t);
                        }
                        return st;
                    }
                    let mut st = if active { button::primary(t, s) } else { button::secondary(t, s) };
                    st.border = Border::default().rounded(8);
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
        ]
        .spacing(8);

        let status = if cx.scanning {
            "Scanning your library…".to_string()
        } else if let Some(e) = cx.error {
            e.to_string()
        } else {
            format!("{} of {} charts", self.shown.len(), cat.len())
        };
        let mut bar = row![
            text_input("Search artist, song, charter, album…", &self.query).on_input(Msg::Query).padding(8).width(320),
            pick_list(KindOpt::ALL, Some(KindOpt(self.kind)), Msg::Kind).padding(8),
            text(status).size(12),
            space().width(Length::Fill),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if !self.selected.is_empty() {
            let n = self.selected.len();
            bar = bar
                .push(text(format!("{n} selected")).size(13))
                .push(
                    button(text(if self.confirm_delete { format!("Confirm: delete {n}") } else { format!("Delete {n}") }).size(13))
                        .padding([6, 12])
                        .style(move |t: &Theme, s| if self.confirm_delete { button::danger(t, s) } else { button::secondary(t, s) })
                        .on_press(Msg::Delete),
                )
                .push(button(text("Clear").size(13)).padding([6, 12]).style(crate::style::secondary).on_press(Msg::ClearSelection));
        } else {
            bar = bar.push(
                button(text("Select all shown").size(13)).padding([6, 12]).style(crate::style::secondary).on_press(Msg::SelectAllShown),
            );
        }
        bar = bar.push(button(text("Rescan").size(13)).padding([6, 12]).on_press(Msg::Rescan));

        // Header and rows share these widths, so every column lines up with its heading.
        const CHECK_W: f32 = 22.0;
        const TYPE_W: f32 = 56.0;
        const ASSET_W: f32 = 48.0;
        const OPEN_W: f32 = 56.0;
        const GAP: f32 = 10.0;
        // Right side leaves room for the scrollbar, which floats over the list.
        let row_pad = iced::Padding { top: 0.0, right: 26.0, bottom: 0.0, left: 12.0 };

        let sort_head = |label: &'static str, s: SortBy, portion: u16| {
            let arrow = match self.sort == Some(s) {
                true => " ▲",
                false => "",
            };
            button(text(format!("{label}{arrow}")).size(12).font(crate::BOLD))
                .padding(0)
                .width(Length::FillPortion(portion))
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
                sort_head("Artist", SortBy::Artist, 3),
                sort_head("Song", SortBy::Song, 4),
                sort_head("Charter", SortBy::Charter, 2),
                fixed_head("Type", TYPE_W),
                fixed_head("Art", ASSET_W),
                fixed_head("BG", ASSET_W),
                fixed_head("Video", ASSET_W),
                fixed_head("Lyrics", ASSET_W),
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
                let cell = |s: &str, p: u16| {
                    container(text(s.to_string()).size(13).wrapping(text::Wrapping::None))
                        .width(Length::FillPortion(p))
                        .clip(true)
                };
                let open = if c.is_sng() { c.path.parent().map(|p| p.to_path_buf()).unwrap_or_default() } else { c.path.clone() };
                let song = if self.filter == Some(Filter::Duplicates) {
                    // Show where each copy is, so you can tell them apart.
                    c.path
                        .parent()
                        .and_then(|p| p.file_name())
                        .map(|n| format!("{}   — in {}", c.name, n.to_string_lossy()))
                        .unwrap_or_else(|| c.name.clone())
                } else {
                    c.name.clone()
                };
                button(
                    row![
                        text(if sel { "☑" } else { "☐" }).size(14).width(CHECK_W).center(),
                        cell(&c.artist, 3),
                        cell(&song, 4),
                        cell(&c.charter, 2),
                        text(c.chart_type.label()).size(12).width(TYPE_W).center(),
                        mark(Some(c.has_album_art)),
                        mark(Some(c.has_background)),
                        mark(Some(c.has_video)),
                        mark(cx.lyrics.get(&c.path).copied()),
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
                .on_press(Msg::Toggle(i))
                .style(move |t: &Theme, s| {
                    let p = t.extended_palette();
                    button::Style {
                        background: match (sel, s) {
                            (true, _) => Some(Background::Color(crate::style::selected(t).scale_alpha(0.5))),
                            (false, button::Status::Hovered) => Some(Background::Color(crate::style::weak(t))),
                            _ if r % 2 == 1 => Some(Background::Color(crate::style::weakest(t))),
                            _ => None,
                        },
                        text_color: p.background.base.text,
                        ..Default::default()
                    }
                })
                .into()
            },
            Msg::Scrolled,
        );

        column![
            container(column![tiles, bar].spacing(10)).padding([12, 14]),
            head,
            rule::horizontal(1).style(crate::style::rule),
            list,
        ]
        .height(Length::Fill)
        .into()
    }
}
