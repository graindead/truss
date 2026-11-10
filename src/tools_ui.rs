//! The Tools page: Art Studio, Lyrics, Video Sync, the Scene Switcher and the Issue scanner.
//!
//! Every edit is made only when you click a button for it, and any file it changes or replaces
//! is backed up first (see `backup.rs`).

use crate::art::{self, ArtResult};
use crate::library::LocalChart;
use crate::lyrics::{self, LrcLine, LrcResult};
use crate::video::{self, VideoResult};
use crate::vlist::VList;
use futures::{SinkExt, channel::mpsc::Sender};
use iced::widget::{
    button, column, container, image, progress_bar, row, rule, scrollable, slider, space, text, text_input,
};
use iced::{Alignment, Background, Border, Element, Length, Subscription, Task, Theme};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// The Issue scanner is hidden for now (a full library produces more rows than a spreadsheet
/// can hold). Its code stays so it can be revamped and switched back on.
const ISSUE_SCANNER_ENABLED: bool = false;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Art,
    Lyrics,
    Video,
    Scenes,
    Issues,
}

/// What the page reads from the app.
pub struct Ctx<'a> {
    pub catalog: &'a [Arc<LocalChart>],
    pub lyrics: &'a HashMap<PathBuf, bool>,
    pub settings: &'a crate::settings::Settings,
}

/// Requests for the app.
pub enum Out {
    None,
    /// A chart's files changed; re-read it.
    ChartChanged(PathBuf),
    LyricsChanged(PathBuf, bool),
    Open(PathBuf),
    SetIssuePaths(Option<PathBuf>, Option<PathBuf>),
}

// ---------- Chart picker shared by the tools ----------

#[derive(Default)]
struct Picker {
    query: String,
    only_missing: bool,
    list: VList,
    shown: Vec<usize>,
    selected: Option<PathBuf>,
}

impl Picker {
    fn new() -> Self {
        Self { only_missing: true, ..Default::default() }
    }

    fn refresh(&mut self, cat: &[Arc<LocalChart>], missing: impl Fn(&LocalChart) -> bool) {
        let before = self.shown.len();
        let words: Vec<String> = self.query.to_lowercase().split_whitespace().map(String::from).collect();
        self.shown = (0..cat.len())
            .filter(|&i| {
                let c = &cat[i];
                let hay = format!("{} {} {}", c.artist, c.name, c.charter).to_lowercase();
                (!self.only_missing || missing(c)) && words.iter().all(|w| hay.contains(w.as_str()))
            })
            .collect();
        if self.shown.len() != before {
            self.list.reset();
        }
    }

    fn selected<'a>(&self, cat: &'a [Arc<LocalChart>]) -> Option<&'a Arc<LocalChart>> {
        let p = self.selected.as_ref()?;
        cat.iter().find(|c| &c.path == p)
    }

    fn view<'a, M: Clone + 'a>(
        &'a self,
        cat: &'a [Arc<LocalChart>],
        missing_label: &'static str,
        on: impl Fn(PickMsg) -> M + Copy + 'a,
        badge: impl Fn(&LocalChart) -> &'static str + 'a,
    ) -> Element<'a, M> {
        let head = column![
            text_input("Search…", &self.query).on_input(move |q| on(PickMsg::Query(q))).padding(7),
            row![
                toggle_chip(missing_label, self.only_missing, on(PickMsg::OnlyMissing(true))),
                toggle_chip("All charts", !self.only_missing, on(PickMsg::OnlyMissing(false))),
                space().width(Length::Fill),
                text(format!("{}", self.shown.len())).size(12),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        ]
        .spacing(8)
        .padding(10);
        let list = self.list.view(
            self.shown.len(),
            move |r| {
                let c = &cat[self.shown[r]];
                let sel = self.selected.as_ref() == Some(&c.path);
                button(
                    row![
                        column![
                            text(c.name.clone()).size(13).wrapping(text::Wrapping::None),
                            text(c.artist.clone()).size(11).wrapping(text::Wrapping::None),
                        ]
                        .width(Length::Fill)
                        .clip(true),
                        text(badge(c)).size(11),
                    ]
                    .align_y(Alignment::Center)
                    .spacing(6),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .padding([1, 10])
                .on_press(on(PickMsg::Select(c.path.clone())))
                .style(move |t: &Theme, s| {
                    let p = t.extended_palette();
                    button::Style {
                        background: match (sel, s) {
                            (true, _) => Some(Background::Color(p.primary.weak.color)),
                            (false, button::Status::Hovered) => Some(Background::Color(p.background.weak.color)),
                            _ => None,
                        },
                        text_color: if sel { p.primary.weak.text } else { p.background.base.text },
                        ..Default::default()
                    }
                })
                .into()
            },
            move |vp| on(PickMsg::Scrolled(vp)),
        );
        container(column![head, rule::horizontal(1), list])
            .width(330)
            .height(Length::Fill)
            .style(|t: &Theme| container::Style {
                background: Some(t.extended_palette().background.weakest.color.into()),
                ..Default::default()
            })
            .into()
    }

    /// Returns true when the selection changed.
    fn update(&mut self, m: PickMsg) -> bool {
        match m {
            PickMsg::Query(q) => self.query = q,
            PickMsg::OnlyMissing(b) => {
                self.only_missing = b;
                self.list.reset();
            }
            PickMsg::Select(p) => {
                let changed = self.selected.as_ref() != Some(&p);
                self.selected = Some(p);
                return changed;
            }
            PickMsg::Scrolled(vp) => self.list.on_scroll(vp),
        }
        false
    }
}

#[derive(Debug, Clone)]
pub enum PickMsg {
    Query(String),
    OnlyMissing(bool),
    Select(PathBuf),
    Scrolled(scrollable::Viewport),
}

fn toggle_chip<'a, M: Clone + 'a>(label: &'static str, active: bool, msg: M) -> Element<'a, M> {
    button(text(label).size(12))
        .padding([3, 10])
        .style(move |t: &Theme, s| {
            let mut b = if active { button::primary(t, s) } else { button::secondary(t, s) };
            b.border = Border::default().rounded(12);
            b
        })
        .on_press(msg)
        .into()
}

fn heading<'a, M: 'a>(s: &'a str) -> Element<'a, M> {
    text(s).size(15).font(crate::BOLD).into()
}

fn note<'a, M: 'a>(s: impl Into<String>) -> Element<'a, M> {
    text(s.into()).size(12).into()
}

fn status_line<'a, M: 'a>(s: &'a Option<Result<String, String>>) -> Element<'a, M> {
    match s {
        Some(Ok(m)) => text(m.as_str()).size(12).style(text::success).into(),
        Some(Err(e)) => text(e.as_str()).size(12).style(text::danger).into(),
        None => space().into(),
    }
}

fn sng_note<'a, M: 'a>() -> Element<'a, M> {
    note("This chart is a .sng file. The tools only edit chart folders.")
}

/// Progress of a batch job.
#[derive(Debug, Clone)]
pub enum BatchEvent {
    Item { done: usize, total: usize, ok: bool, path: PathBuf },
    Finished,
}

struct Batch {
    label: String,
    done: usize,
    total: usize,
    failed: usize,
    handle: iced::task::Handle,
}

// ---------- Art Studio ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtMode {
    Album,
    Background,
}

struct ArtTool {
    mode: ArtMode,
    picker: Picker,
    artist: String,
    album: String,
    results: Vec<ArtResult>,
    thumbs: HashMap<String, image::Handle>,
    searching: bool,
    current: Option<image::Handle>,
    blur: u32,
    busy: bool,
    status: Option<Result<String, String>>,
    batch: Option<Batch>,
}

#[derive(Debug, Clone)]
pub enum ArtMsg {
    Mode(ArtModeMsg),
    Pick(PickMsg),
    Artist(String),
    Album(String),
    Search,
    Results(Result<Vec<ArtResult>, String>),
    Thumb(String, Result<bytes::Bytes, String>),
    Current(Result<crate::api::DecodedImage, String>),
    Use(String),
    Blur(u32),
    Generate,
    DeleteBg,
    Done(Result<String, String>),
    BatchStart,
    BatchCancel,
    Batch(BatchEvent),
}

#[derive(Debug, Clone, Copy)]
pub struct ArtModeMsg(bool);

impl ArtTool {
    fn missing(&self) -> impl Fn(&LocalChart) -> bool + use<> {
        let mode = self.mode;
        move |c: &LocalChart| !c.is_sng() && if mode == ArtMode::Album { !c.has_album_art } else { !c.has_background }
    }

    fn load_current(&self, c: &LocalChart) -> Task<ArtMsg> {
        let file = match self.mode {
            ArtMode::Album => art::album_file(&c.path),
            ArtMode::Background => crate::backup::find_files(&c.path, "background", &["png", "jpg", "jpeg"], true).pop(),
        };
        match file {
            Some(f) => Task::perform(art::local_preview(f, 320), ArtMsg::Current),
            None => Task::done(ArtMsg::Current(Err(String::new()))),
        }
    }

    fn update(&mut self, m: ArtMsg, cx: &Ctx) -> (Task<ArtMsg>, Out) {
        let sel = self.picker.selected(cx.catalog).cloned();
        match m {
            ArtMsg::Mode(ArtModeMsg(album)) => {
                self.mode = if album { ArtMode::Album } else { ArtMode::Background };
                self.picker.refresh(cx.catalog, self.missing());
                self.picker.list.reset();
                self.status = None;
                if let Some(c) = &sel {
                    return (self.load_current(c), Out::None);
                }
            }
            ArtMsg::Pick(p) => {
                let changed = self.picker.update(p);
                self.picker.refresh(cx.catalog, self.missing());
                if changed && let Some(c) = self.picker.selected(cx.catalog).cloned() {
                    self.artist = c.artist.clone();
                    self.album = c.album.clone();
                    self.results.clear();
                    self.status = None;
                    self.current = None;
                    let mut tasks = vec![self.load_current(&c)];
                    if self.mode == ArtMode::Album && !c.is_sng() {
                        self.searching = true;
                        tasks.push(Task::perform(art::search(c.artist.clone(), c.album.clone()), ArtMsg::Results));
                    }
                    return (Task::batch(tasks), Out::None);
                }
            }
            ArtMsg::Artist(s) => self.artist = s,
            ArtMsg::Album(s) => self.album = s,
            ArtMsg::Search => {
                self.searching = true;
                self.results.clear();
                return (Task::perform(art::search(self.artist.clone(), self.album.clone()), ArtMsg::Results), Out::None);
            }
            ArtMsg::Results(r) => {
                self.searching = false;
                match r {
                    Ok(v) => {
                        if v.is_empty() {
                            self.status = Some(Err("No album art found. Try editing the artist or album.".into()));
                        }
                        let tasks: Vec<_> = v
                            .iter()
                            .filter(|r| !self.thumbs.contains_key(&r.thumb_url))
                            .map(|r| {
                                let u = r.thumb_url.clone();
                                Task::perform(art::fetch(u.clone()), move |b| ArtMsg::Thumb(u.clone(), b))
                            })
                            .collect();
                        self.results = v;
                        return (Task::batch(tasks), Out::None);
                    }
                    Err(e) => self.status = Some(Err(format!("Search failed: {e}"))),
                }
            }
            ArtMsg::Thumb(u, Ok(b)) => {
                self.thumbs.insert(u, image::Handle::from_bytes(b));
            }
            ArtMsg::Thumb(..) => {}
            ArtMsg::Current(r) => {
                self.current = r.ok().map(|d| image::Handle::from_rgba(d.width, d.height, d.rgba));
            }
            ArtMsg::Use(url) => {
                let Some(c) = sel.filter(|c| !c.is_sng()) else { return (Task::none(), Out::None) };
                self.busy = true;
                return (
                    Task::perform(art::save_album_art(c.path.clone(), url), |r| {
                        ArtMsg::Done(r.map(|_| "Album art saved as album.jpg (any old art was backed up first).".into()))
                    }),
                    Out::None,
                );
            }
            ArtMsg::Blur(b) => self.blur = b,
            ArtMsg::Generate => {
                let Some(c) = sel.filter(|c| !c.is_sng()) else { return (Task::none(), Out::None) };
                self.busy = true;
                return (
                    Task::perform(art::generate_background(c.path.clone(), self.blur), |r| {
                        ArtMsg::Done(r.map(|_| "Background saved as background.jpg (any old one was backed up first).".into()))
                    }),
                    Out::None,
                );
            }
            ArtMsg::DeleteBg => {
                let Some(c) = sel.filter(|c| !c.is_sng()) else { return (Task::none(), Out::None) };
                self.busy = true;
                return (
                    Task::perform(art::delete_background(c.path.clone()), |r| {
                        ArtMsg::Done(r.map(|_| "Background removed (backed up first).".into()))
                    }),
                    Out::None,
                );
            }
            ArtMsg::Done(r) => {
                self.busy = false;
                let ok = r.is_ok();
                self.status = Some(r);
                if ok && let Some(c) = sel {
                    return (self.load_current(&c), Out::ChartChanged(c.path.clone()));
                }
            }
            ArtMsg::BatchStart => {
                let charts: Vec<Arc<LocalChart>> = self.picker.shown.iter().map(|&i| cx.catalog[i].clone()).filter(|c| !c.is_sng()).collect();
                let total = charts.len();
                if total == 0 {
                    return (Task::none(), Out::None);
                }
                let (mode, blur) = (self.mode, self.blur);
                let stream = iced::stream::channel(16, async move |mut tx: Sender<BatchEvent>| {
                    for (i, c) in charts.iter().enumerate() {
                        let ok = match mode {
                            ArtMode::Album => {
                                matches!(art::auto_album_art(c.path.clone(), c.artist.clone(), c.album.clone()).await, Ok(true))
                            }
                            ArtMode::Background => art::generate_background(c.path.clone(), blur).await.is_ok(),
                        };
                        let _ = tx.send(BatchEvent::Item { done: i + 1, total, ok, path: c.path.clone() }).await;
                    }
                    let _ = tx.send(BatchEvent::Finished).await;
                });
                let (task, handle) = Task::run(stream, ArtMsg::Batch).abortable();
                let label = if mode == ArtMode::Album { "Fetching album art" } else { "Generating backgrounds" };
                self.batch = Some(Batch { label: label.into(), done: 0, total, failed: 0, handle });
                return (task, Out::None);
            }
            ArtMsg::BatchCancel => {
                if let Some(b) = self.batch.take() {
                    b.handle.abort();
                    self.status = Some(Ok(format!("Stopped after {} of {}.", b.done, b.total)));
                }
            }
            ArtMsg::Batch(BatchEvent::Item { done, ok, path, .. }) => {
                if let Some(b) = &mut self.batch {
                    b.done = done;
                    if !ok {
                        b.failed += 1;
                    }
                }
                if ok {
                    return (Task::none(), Out::ChartChanged(path));
                }
            }
            ArtMsg::Batch(BatchEvent::Finished) => {
                if let Some(b) = self.batch.take() {
                    self.status = Some(Ok(format!("Done: {} updated, {} not found or failed.", b.total - b.failed, b.failed)));
                }
                self.picker.refresh(cx.catalog, self.missing());
            }
        }
        (Task::none(), Out::None)
    }

    fn view<'a>(&'a self, cx: &Ctx<'a>) -> Element<'a, ArtMsg> {
        let album = self.mode == ArtMode::Album;
        let picker = self.picker.view(
            cx.catalog,
            if album { "Missing album art" } else { "Missing background" },
            ArtMsg::Pick,
            move |c| if c.is_sng() { ".sng" } else if (album && c.has_album_art) || (!album && c.has_background) { "✓" } else { "" },
        );
        let modes = row![
            toggle_chip("Album art", album, ArtMsg::Mode(ArtModeMsg(true))),
            toggle_chip("Backgrounds", !album, ArtMsg::Mode(ArtModeMsg(false))),
        ]
        .spacing(6);
        let mut body = column![modes].spacing(12);
        if let Some(b) = &self.batch {
            body = body.push(batch_view(b, ArtMsg::BatchCancel));
        } else if self.picker.only_missing && !self.picker.shown.is_empty() {
            let n = self.picker.shown.iter().filter(|&&i| !cx.catalog[i].is_sng()).count();
            let label = if album { format!("Fetch art for all {n} shown") } else { format!("Generate all {n} shown") };
            body = body.push(
                row![
                    button(text(label).size(13)).padding([6, 12]).on_press(ArtMsg::BatchStart),
                    note(if album { "Uses the first iTunes match for each chart." } else { "Uses the blur setting below." }),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        }
        match self.picker.selected(cx.catalog) {
            None => body = body.push(note("Pick a chart on the left.")),
            Some(c) => {
                body = body.push(column![text(c.name.clone()).size(20).font(crate::BOLD), text(c.artist.clone()).size(14)].spacing(2));
                let current: Element<_> = match &self.current {
                    Some(h) => image(h.clone()).width(if album { 220 } else { 380 }).into(),
                    None => note(if album { "No album art yet." } else { "No background yet." }),
                };
                body = body.push(column![heading("Current"), current].spacing(6));
                if c.is_sng() {
                    body = body.push(sng_note());
                } else if album {
                    let search = row![
                        text_input("Artist", &self.artist).on_input(ArtMsg::Artist).on_submit(ArtMsg::Search).padding(7),
                        text_input("Album", &self.album).on_input(ArtMsg::Album).on_submit(ArtMsg::Search).padding(7),
                        button(text("Search iTunes").size(13)).padding([7, 12]).on_press(ArtMsg::Search),
                    ]
                    .spacing(8);
                    let mut grid = row![].spacing(10);
                    for r in self.results.iter().take(12) {
                        let thumb: Element<_> = match self.thumbs.get(&r.thumb_url) {
                            Some(h) => image(h.clone()).width(100).height(100).into(),
                            None => container(space()).width(100).height(100).style(container::rounded_box).into(),
                        };
                        let tile = column![
                            thumb,
                            text(r.album.clone()).size(10).width(100).wrapping(text::Wrapping::None),
                        ]
                        .spacing(3)
                        .width(100)
                        .clip(true);
                        grid = grid.push(
                            button(tile)
                                .padding(4)
                                .style(button::text)
                                .on_press_maybe((!self.busy).then(|| ArtMsg::Use(r.url.clone()))),
                        );
                    }
                    body = body.push(column![heading("Find album art"), search].spacing(6));
                    if self.searching {
                        body = body.push(note("Searching…"));
                    } else if !self.results.is_empty() {
                        body = body.push(note("Click a cover to use it (saved as a 512×512 album.jpg)."));
                        body = body.push(grid.wrap());
                    }
                } else {
                    body = body.push(
                        column![
                            heading("Generate"),
                            row![
                                text(format!("Blur {}", self.blur)).size(13).width(70),
                                slider(0..=100u32, self.blur, ArtMsg::Blur).width(240),
                            ]
                            .spacing(10)
                            .align_y(Alignment::Center),
                            note(if c.has_album_art {
                                "Made from the album art, scaled to 1920×1080 and blurred."
                            } else {
                                "No album art, so this makes a dark gradient."
                            }),
                            row![
                                button(text(if c.has_background { "Regenerate" } else { "Generate" }).size(13))
                                    .padding([7, 14])
                                    .on_press_maybe((!self.busy).then_some(ArtMsg::Generate)),
                                button(text("Remove background").size(13))
                                    .padding([7, 14])
                                    .style(button::secondary)
                                    .on_press_maybe((c.has_background && !self.busy).then_some(ArtMsg::DeleteBg)),
                            ]
                            .spacing(8),
                        ]
                        .spacing(8),
                    );
                }
            }
        }
        body = body.push(status_line(&self.status));
        row![picker, scrollable(container(body).padding(16)).width(Length::Fill)].into()
    }
}

fn batch_view<'a, M: Clone + 'a>(b: &'a Batch, cancel: M) -> Element<'a, M> {
    row![
        column![
            text(format!("{}: {} of {}", b.label, b.done, b.total)).size(13),
            progress_bar(0.0..=b.total.max(1) as f32, b.done as f32).girth(6),
        ]
        .spacing(4)
        .width(Length::Fill),
        button(text("Stop").size(13)).padding([6, 12]).style(button::secondary).on_press(cancel),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .into()
}

// ---------- Lyrics ----------

struct LyricsTool {
    picker: Picker,
    artist: String,
    title: String,
    results: Vec<LrcResult>,
    searching: bool,
    chosen: Option<usize>,
    lines: Vec<LrcLine>,
    /// When the first lyric should appear (ms); the offset applied to every line.
    first_ms: f64,
    player: Option<crate::audio::Player>,
    loading_audio: bool,
    position_ms: f64,
    busy: bool,
    confirm_remove: bool,
    status: Option<Result<String, String>>,
    batch: Option<Batch>,
}

pub enum LyricsMsg {
    Pick(PickMsg),
    Artist(String),
    Title(String),
    Search,
    Results(Result<Vec<LrcResult>, String>),
    Choose(usize),
    FirstMs(f64),
    Nudge(f64),
    Preview,
    PlayerReady(Result<crate::audio::Player, String>),
    Stop,
    Tick,
    Apply,
    Remove,
    Done(Result<bool, String>),
    BatchStart,
    BatchCancel,
    Batch(BatchEvent),
}

impl Clone for LyricsMsg {
    fn clone(&self) -> Self {
        match self {
            Self::Pick(p) => Self::Pick(p.clone()),
            Self::Artist(s) => Self::Artist(s.clone()),
            Self::Title(s) => Self::Title(s.clone()),
            Self::Search => Self::Search,
            Self::Results(r) => Self::Results(r.clone()),
            Self::Choose(i) => Self::Choose(*i),
            Self::FirstMs(v) => Self::FirstMs(*v),
            Self::Nudge(v) => Self::Nudge(*v),
            Self::Preview => Self::Preview,
            // A player can't be copied; UI messages are never this variant.
            Self::PlayerReady(_) => Self::PlayerReady(Err("cloned".into())),
            Self::Stop => Self::Stop,
            Self::Tick => Self::Tick,
            Self::Apply => Self::Apply,
            Self::Remove => Self::Remove,
            Self::Done(r) => Self::Done(r.clone()),
            Self::BatchStart => Self::BatchStart,
            Self::BatchCancel => Self::BatchCancel,
            Self::Batch(b) => Self::Batch(b.clone()),
        }
    }
}

impl std::fmt::Debug for LyricsMsg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LyricsMsg")
    }
}

impl LyricsTool {
    fn offset(&self) -> f64 {
        self.lines.first().map(|l| self.first_ms - l.ms).unwrap_or(0.0)
    }

    fn update(&mut self, m: LyricsMsg, cx: &Ctx) -> (Task<LyricsMsg>, Out) {
        let lyr = cx.lyrics.clone();
        let missing = move |c: &LocalChart| !c.is_sng() && lyr.get(&c.path) == Some(&false);
        let sel = self.picker.selected(cx.catalog).cloned();
        if !matches!(m, LyricsMsg::Remove | LyricsMsg::Tick) {
            self.confirm_remove = false;
        }
        match m {
            LyricsMsg::Pick(p) => {
                let changed = self.picker.update(p);
                self.picker.refresh(cx.catalog, missing);
                if changed && let Some(c) = self.picker.selected(cx.catalog).cloned() {
                    self.player = None;
                    self.artist = c.artist.clone();
                    self.title = c.name.clone();
                    self.results.clear();
                    self.chosen = None;
                    self.lines.clear();
                    self.status = None;
                    if !c.is_sng() {
                        self.searching = true;
                        return (Task::perform(lyrics::search(c.artist.clone(), c.name.clone()), LyricsMsg::Results), Out::None);
                    }
                }
            }
            LyricsMsg::Artist(s) => self.artist = s,
            LyricsMsg::Title(s) => self.title = s,
            LyricsMsg::Search => {
                self.searching = true;
                return (Task::perform(lyrics::search(self.artist.clone(), self.title.clone()), LyricsMsg::Results), Out::None);
            }
            LyricsMsg::Results(r) => {
                self.searching = false;
                match r {
                    Ok(v) => {
                        if v.is_empty() {
                            self.status = Some(Err("No synced lyrics found on LRCLIB.".into()));
                        }
                        self.results = v;
                    }
                    Err(e) => self.status = Some(Err(format!("Search failed: {e}"))),
                }
            }
            LyricsMsg::Choose(i) => {
                self.chosen = Some(i);
                self.lines = self.results.get(i).and_then(|r| r.synced_lyrics.as_deref()).map(lyrics::parse_lrc).unwrap_or_default();
                self.first_ms = self.lines.first().map(|l| l.ms).unwrap_or(0.0);
            }
            LyricsMsg::FirstMs(v) => self.first_ms = v.max(0.0),
            LyricsMsg::Nudge(d) => self.first_ms = (self.first_ms + d).max(0.0),
            LyricsMsg::Preview => {
                let Some(c) = sel else { return (Task::none(), Out::None) };
                if let Some(p) = &self.player {
                    p.seek((self.first_ms - 3000.0).max(0.0));
                    p.play();
                    return (Task::none(), Out::None);
                }
                self.loading_audio = true;
                let dir = c.path.clone();
                let start = (self.first_ms - 3000.0).max(0.0);
                return (
                    Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                let stems: Vec<_> = lyrics::audio_stems(&dir)
                                    .into_iter()
                                    .filter_map(|(p, ext)| {
                                        std::fs::read(p).ok().map(|d| (crate::audio::StemData::Memory(d.into()), ext))
                                    })
                                    .collect();
                                if stems.is_empty() {
                                    return Err("No audio files in this chart folder.".to_string());
                                }
                                let p = crate::audio::Player::new(stems, 0.0, 0.0)?;
                                p.set_volume(0.6);
                                p.seek(start);
                                p.play();
                                Ok(p)
                            })
                            .await
                            .map_err(|e| e.to_string())?
                        },
                        LyricsMsg::PlayerReady,
                    ),
                    Out::None,
                );
            }
            LyricsMsg::PlayerReady(r) => {
                self.loading_audio = false;
                match r {
                    Ok(p) => self.player = Some(p),
                    Err(e) => self.status = Some(Err(e)),
                }
            }
            LyricsMsg::Stop => {
                if let Some(p) = &self.player {
                    p.pause();
                }
            }
            LyricsMsg::Tick => {
                if let Some(p) = &self.player {
                    self.position_ms = p.position_ms();
                }
            }
            LyricsMsg::Apply => {
                let Some(file) = sel.as_ref().and_then(|c| c.chart_path()) else { return (Task::none(), Out::None) };
                self.busy = true;
                if let Some(p) = &self.player {
                    p.pause();
                }
                return (
                    Task::perform(lyrics::apply(file, Some(self.lines.clone()), self.offset()), |r| LyricsMsg::Done(r.map(|_| true))),
                    Out::None,
                );
            }
            LyricsMsg::Remove => {
                if !self.confirm_remove {
                    self.confirm_remove = true;
                    return (Task::none(), Out::None);
                }
                self.confirm_remove = false;
                let Some(file) = sel.as_ref().and_then(|c| c.chart_path()) else { return (Task::none(), Out::None) };
                self.busy = true;
                return (Task::perform(lyrics::apply(file, None, 0.0), |r| LyricsMsg::Done(r.map(|_| false))), Out::None);
            }
            LyricsMsg::Done(r) => {
                self.busy = false;
                match r {
                    Ok(has) => {
                        self.status = Some(Ok(if has {
                            "Lyrics added to the chart (the original file was backed up first).".into()
                        } else {
                            "Lyrics removed (the original file was backed up first).".into()
                        }));
                        if let Some(c) = sel {
                            return (Task::none(), Out::LyricsChanged(c.path.clone(), has));
                        }
                    }
                    Err(e) => self.status = Some(Err(e)),
                }
            }
            LyricsMsg::BatchStart => {
                let charts: Vec<Arc<LocalChart>> = self.picker.shown.iter().map(|&i| cx.catalog[i].clone()).filter(|c| !c.is_sng()).collect();
                let total = charts.len();
                if total == 0 {
                    return (Task::none(), Out::None);
                }
                let stream = iced::stream::channel(16, async move |mut tx: Sender<BatchEvent>| {
                    for (i, c) in charts.iter().enumerate() {
                        let ok = async {
                            let file = c.chart_path().ok_or("no chart file")?;
                            let m = lyrics::best_match(c.artist.clone(), c.name.clone(), c.album.clone(), c.song_length_ms as f64 / 1000.0)
                                .await?
                                .ok_or("not found")?;
                            let lines = lyrics::parse_lrc(m.synced_lyrics.as_deref().unwrap_or(""));
                            lyrics::apply(file, Some(lines), 0.0).await?;
                            Ok::<_, String>(())
                        }
                        .await
                        .is_ok();
                        let _ = tx.send(BatchEvent::Item { done: i + 1, total, ok, path: c.path.clone() }).await;
                    }
                    let _ = tx.send(BatchEvent::Finished).await;
                });
                let (task, handle) = Task::run(stream, LyricsMsg::Batch).abortable();
                self.batch = Some(Batch { label: "Adding lyrics".into(), done: 0, total, failed: 0, handle });
                return (task, Out::None);
            }
            LyricsMsg::BatchCancel => {
                if let Some(b) = self.batch.take() {
                    b.handle.abort();
                    self.status = Some(Ok(format!("Stopped after {} of {}.", b.done, b.total)));
                }
            }
            LyricsMsg::Batch(BatchEvent::Item { done, ok, path, .. }) => {
                if let Some(b) = &mut self.batch {
                    b.done = done;
                    if !ok {
                        b.failed += 1;
                    }
                }
                if ok {
                    return (Task::none(), Out::LyricsChanged(path, true));
                }
            }
            LyricsMsg::Batch(BatchEvent::Finished) => {
                if let Some(b) = self.batch.take() {
                    self.status = Some(Ok(format!(
                        "Done: lyrics added to {}, {} not found or failed. Timing isn't adjusted in batch mode.",
                        b.total - b.failed,
                        b.failed
                    )));
                }
            }
        }
        (Task::none(), Out::None)
    }

    fn view<'a>(&'a self, cx: &Ctx<'a>) -> Element<'a, LyricsMsg> {
        let lyr = cx.lyrics;
        let picker = self.picker.view(cx.catalog, "Missing lyrics", LyricsMsg::Pick, move |c| {
            if c.is_sng() {
                ".sng"
            } else {
                match lyr.get(&c.path) {
                    Some(true) => "✓",
                    Some(false) => "",
                    None => "…",
                }
            }
        });
        let mut body = column![note("Synced lyrics from LRCLIB, written into notes.chart / notes.mid.")].spacing(12);
        if lyr.is_empty() {
            body = body.push(note("Checking which charts have lyrics…"));
        }
        if let Some(b) = &self.batch {
            body = body.push(batch_view(b, LyricsMsg::BatchCancel));
        } else if self.picker.only_missing && !self.picker.shown.is_empty() {
            body = body.push(
                row![
                    button(text(format!("Auto-add lyrics to all {} shown", self.picker.shown.len())).size(13))
                        .padding([6, 12])
                        .on_press(LyricsMsg::BatchStart),
                    note("Uses LRCLIB's best match with no timing adjustment."),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        }
        match self.picker.selected(cx.catalog) {
            None => body = body.push(note("Pick a chart on the left.")),
            Some(c) => {
                body = body.push(column![text(c.name.clone()).size(20).font(crate::BOLD), text(c.artist.clone()).size(14)].spacing(2));
                if c.is_sng() {
                    body = body.push(sng_note());
                } else {
                    if lyr.get(&c.path) == Some(&true) {
                        body = body.push(
                            row![
                                note("This chart already has lyrics."),
                                button(text(if self.confirm_remove { "Click again to remove" } else { "Remove lyrics" }).size(12))
                                    .padding([4, 10])
                                    .style(move |t: &Theme, s| if self.confirm_remove { button::danger(t, s) } else { button::secondary(t, s) })
                                    .on_press_maybe((!self.busy).then_some(LyricsMsg::Remove)),
                            ]
                            .spacing(10)
                            .align_y(Alignment::Center),
                        );
                    }
                    body = body.push(
                        row![
                            text_input("Artist", &self.artist).on_input(LyricsMsg::Artist).on_submit(LyricsMsg::Search).padding(7),
                            text_input("Song", &self.title).on_input(LyricsMsg::Title).on_submit(LyricsMsg::Search).padding(7),
                            button(text("Search").size(13)).padding([7, 12]).on_press(LyricsMsg::Search),
                        ]
                        .spacing(8),
                    );
                    if self.searching {
                        body = body.push(note("Searching…"));
                    }
                    let mut results = column![].spacing(4);
                    for (i, r) in self.results.iter().enumerate().take(10) {
                        let chosen = self.chosen == Some(i);
                        let dur = r.duration.map(video::fmt_duration).unwrap_or_default();
                        let len_hint = match r.duration {
                            Some(d) if c.song_length_ms > 0 && (d - c.song_length_ms as f64 / 1000.0).abs() < 5.0 => "  · length matches",
                            _ => "",
                        };
                        results = results.push(
                            button(
                                text(format!(
                                    "{} — {}  ({}{dur}){len_hint}",
                                    r.track_name,
                                    r.artist_name,
                                    r.album_name.as_deref().map(|a| format!("{a}, ")).unwrap_or_default()
                                ))
                                .size(12),
                            )
                            .width(Length::Fill)
                            .padding([5, 8])
                            .style(move |t: &Theme, s| if chosen { button::primary(t, s) } else { button::secondary(t, s) })
                            .on_press(LyricsMsg::Choose(i)),
                        );
                    }
                    body = body.push(results);
                    if !self.lines.is_empty() {
                        body = body.push(self.timing_view());
                    }
                }
            }
        }
        body = body.push(status_line(&self.status));
        row![picker, scrollable(container(body).padding(16)).width(Length::Fill)].into()
    }

    fn timing_view(&self) -> Element<'_, LyricsMsg> {
        let fmt = |ms: f64| format!("{}:{:05.2}", (ms / 60000.0) as u64, (ms / 1000.0) % 60.0);
        let first = &self.lines[0];
        let max = self.lines.last().map(|l| l.ms).unwrap_or(0.0).max(first.ms) + 30000.0;
        let offset = self.offset();
        let playing = self.player.as_ref().is_some_and(|p| p.is_playing());
        let current = if playing {
            self.lines
                .iter()
                .rev()
                .find(|l| l.ms + offset <= self.position_ms)
                .map(|l| l.text.as_str())
                .unwrap_or("♪")
        } else {
            ""
        };
        column![
            heading("Timing"),
            note(format!("First lyric: \"{}\" — LRCLIB has it at {}.", first.text, fmt(first.ms))),
            row![
                text(format!("Shows at {}", fmt(self.first_ms))).size(13).width(130),
                slider(0.0..=max, self.first_ms, LyricsMsg::FirstMs).step(10.0).width(Length::Fill),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            row![
                button(text("−1 s").size(12)).padding([4, 8]).style(button::secondary).on_press(LyricsMsg::Nudge(-1000.0)),
                button(text("−0.1 s").size(12)).padding([4, 8]).style(button::secondary).on_press(LyricsMsg::Nudge(-100.0)),
                button(text("+0.1 s").size(12)).padding([4, 8]).style(button::secondary).on_press(LyricsMsg::Nudge(100.0)),
                button(text("+1 s").size(12)).padding([4, 8]).style(button::secondary).on_press(LyricsMsg::Nudge(1000.0)),
                text(format!("offset {:+.2} s", offset / 1000.0)).size(12),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            row![
                button(text(if self.loading_audio { "Loading…" } else { "Preview from 3 s before" }).size(13))
                    .padding([7, 12])
                    .style(button::secondary)
                    .on_press_maybe((!self.loading_audio).then_some(LyricsMsg::Preview)),
                button(text("Stop").size(13)).padding([7, 12]).style(button::secondary).on_press_maybe(playing.then_some(LyricsMsg::Stop)),
                text(if playing { format!("{}   {current}", fmt(self.position_ms)) } else { String::new() }).size(15),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            button(text("Add lyrics to chart").size(14))
                .padding([8, 16])
                .style(button::primary)
                .on_press_maybe((!self.busy).then_some(LyricsMsg::Apply)),
            note("The chart file is backed up before it's changed, and the result is checked to still load."),
        ]
        .spacing(8)
        .into()
    }
}

// ---------- Video Sync ----------

struct VideoTool {
    picker: Picker,
    tools: Option<(bool, bool)>,
    winget: Option<bool>,
    installing: bool,
    query: String,
    results: Vec<VideoResult>,
    searching: bool,
    url: String,
    progress: Option<(String, f32, iced::task::Handle)>,
    confirm_delete: bool,
    status: Option<Result<String, String>>,
}

#[derive(Debug, Clone)]
pub enum VideoMsg {
    Pick(PickMsg),
    Tools((bool, bool)),
    Winget(bool),
    Recheck,
    Install,
    Installed(Result<(), String>),
    Query(String),
    Search,
    Results(Result<Vec<VideoResult>, String>),
    Url(String),
    Download(String),
    Progress(video::Progress),
    Cancel,
    ChooseFile,
    FileChosen(Option<PathBuf>),
    Delete,
    Done(Result<String, String>),
    OpenLink(String),
}

impl VideoTool {
    fn update(&mut self, m: VideoMsg, cx: &Ctx) -> (Task<VideoMsg>, Out) {
        let missing = |c: &LocalChart| !c.is_sng() && !c.has_video;
        let sel = self.picker.selected(cx.catalog).cloned();
        if !matches!(m, VideoMsg::Delete | VideoMsg::Progress(_)) {
            self.confirm_delete = false;
        }
        match m {
            VideoMsg::Pick(p) => {
                let changed = self.picker.update(p);
                self.picker.refresh(cx.catalog, missing);
                if changed && let Some(c) = self.picker.selected(cx.catalog) {
                    self.query = format!("{} {}", c.artist, c.name);
                    self.results.clear();
                    self.status = None;
                }
            }
            VideoMsg::Tools(t) => {
                self.tools = Some(t);
                if (!t.0 || !t.1) && self.winget.is_none() {
                    return (Task::perform(video::winget_available(), VideoMsg::Winget), Out::None);
                }
            }
            VideoMsg::Winget(w) => self.winget = Some(w),
            VideoMsg::Recheck => return (Task::perform(video::check_tools(), VideoMsg::Tools), Out::None),
            VideoMsg::Install => {
                let Some((yt, ff)) = self.tools else { return (Task::none(), Out::None) };
                self.installing = true;
                self.status = None;
                return (Task::perform(video::install_tools(!yt, !ff), VideoMsg::Installed), Out::None);
            }
            VideoMsg::Installed(r) => {
                self.installing = false;
                if let Err(e) = r {
                    self.status = Some(Err(e));
                }
                return (Task::perform(video::check_tools(), VideoMsg::Tools), Out::None);
            }
            VideoMsg::Query(q) => self.query = q,
            VideoMsg::Search => {
                self.searching = true;
                self.results.clear();
                return (Task::perform(video::search(self.query.clone()), VideoMsg::Results), Out::None);
            }
            VideoMsg::Results(r) => {
                self.searching = false;
                match r {
                    Ok(v) => self.results = v,
                    Err(e) => self.status = Some(Err(e)),
                }
            }
            VideoMsg::Url(u) => self.url = u,
            VideoMsg::Download(url) => {
                let Some(c) = sel.filter(|c| !c.is_sng()) else { return (Task::none(), Out::None) };
                if self.progress.is_some() {
                    return (Task::none(), Out::None);
                }
                let (task, handle) = Task::run(video::download(url, c.path.clone()), VideoMsg::Progress).abortable();
                self.progress = Some(("Starting…".into(), 0.0, handle));
                self.status = None;
                return (task, Out::None);
            }
            VideoMsg::Progress(p) => match p {
                video::Progress::Status(s) => {
                    if let Some(pr) = &mut self.progress {
                        pr.0 = s;
                    }
                }
                video::Progress::Percent(v) => {
                    if let Some(pr) = &mut self.progress {
                        pr.0 = "Downloading".into();
                        pr.1 = v;
                    }
                }
                video::Progress::Finished(r) => {
                    self.progress = None;
                    return self.update(
                        VideoMsg::Done(r.map(|p| format!("Saved as {} (any old video was backed up first).", p.file_name().unwrap_or_default().to_string_lossy()))),
                        cx,
                    );
                }
            },
            VideoMsg::Cancel => {
                if let Some((_, _, h)) = self.progress.take() {
                    h.abort(); // yt-dlp is killed with the task
                    self.status = Some(Ok("Download cancelled.".into()));
                    if let Some(c) = &sel {
                        let _ = std::fs::remove_dir_all(c.path.join(".truss-video-tmp"));
                    }
                }
            }
            VideoMsg::ChooseFile => {
                return (
                    Task::perform(
                        async {
                            rfd::AsyncFileDialog::new()
                                .set_title("Choose a video file")
                                .add_filter("Video", &["webm", "mp4", "mkv", "avi", "ogv", "mpeg", "vp8"])
                                .pick_file()
                                .await
                                .map(|h| h.path().to_path_buf())
                        },
                        VideoMsg::FileChosen,
                    ),
                    Out::None,
                );
            }
            VideoMsg::FileChosen(Some(f)) => {
                let Some(c) = sel.filter(|c| !c.is_sng()) else { return (Task::none(), Out::None) };
                return (
                    Task::perform(video::import_local(f, c.path.clone()), |r| {
                        VideoMsg::Done(r.map(|p| format!("Imported as {} (any old video was backed up first).", p.file_name().unwrap_or_default().to_string_lossy())))
                    }),
                    Out::None,
                );
            }
            VideoMsg::FileChosen(None) => {}
            VideoMsg::Delete => {
                if !self.confirm_delete {
                    self.confirm_delete = true;
                    return (Task::none(), Out::None);
                }
                self.confirm_delete = false;
                let Some(c) = sel.filter(|c| !c.is_sng()) else { return (Task::none(), Out::None) };
                return (
                    Task::perform(video::delete(c.path.clone()), |r| VideoMsg::Done(r.map(|_| "Video removed (backed up first).".into()))),
                    Out::None,
                );
            }
            VideoMsg::Done(r) => {
                let ok = r.is_ok();
                self.status = Some(r);
                if ok && let Some(c) = sel {
                    return (Task::none(), Out::ChartChanged(c.path.clone()));
                }
            }
            VideoMsg::OpenLink(u) => {
                let _ = open::that_detached(u);
            }
        }
        (Task::none(), Out::None)
    }

    fn view<'a>(&'a self, cx: &Ctx<'a>) -> Element<'a, VideoMsg> {
        let picker = self.picker.view(cx.catalog, "Missing video", VideoMsg::Pick, |c| {
            if c.is_sng() { ".sng" } else if c.has_video { "✓" } else { "" }
        });
        let mut body = column![].spacing(12);
        match self.tools {
            None => body = body.push(note("Checking for yt-dlp and ffmpeg…")),
            Some((yt, ff)) => {
                if yt && ff {
                    body = body.push(text("yt-dlp and ffmpeg are installed and ready.").size(12).style(text::success));
                } else {
                    let mut missing = Vec::new();
                    if !yt {
                        missing.push("yt-dlp");
                    }
                    if !ff {
                        missing.push("ffmpeg");
                    }
                    let mut r = row![
                        text(format!("Downloading videos needs {}. Local files work without it.", missing.join(" and ")))
                            .size(12)
                            .style(text::danger),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center);
                    if self.installing {
                        r = r.push(note("Installing… follow the window that opened."));
                    } else {
                        if self.winget == Some(true) {
                            let label = if !yt && !ff { "Install both" } else { "Install" };
                            r = r.push(button(text(label).size(12)).padding([4, 12]).on_press(VideoMsg::Install));
                        }
                        r = r.push(
                            button(text("Check again").size(12)).padding([4, 10]).style(button::secondary).on_press(VideoMsg::Recheck),
                        );
                    }
                    body = body.push(r);
                    if self.winget == Some(false) {
                        body = body.push(note(
                            "winget isn't available, so install them manually: yt-dlp from github.com/yt-dlp/yt-dlp, ffmpeg from ffmpeg.org.",
                        ));
                    }
                }
            }
        }
        let can_download = self.tools.is_some_and(|t| t.0) && self.progress.is_none();
        match self.picker.selected(cx.catalog) {
            None => body = body.push(note("Pick a chart on the left.")),
            Some(c) => {
                body = body.push(column![text(c.name.clone()).size(20).font(crate::BOLD), text(c.artist.clone()).size(14)].spacing(2));
                if c.is_sng() {
                    body = body.push(sng_note());
                } else {
                    if c.has_video {
                        body = body.push(
                            row![
                                note("This chart already has a video."),
                                button(text(if self.confirm_delete { "Click again to remove" } else { "Remove video" }).size(12))
                                    .padding([4, 10])
                                    .style(move |t: &Theme, s| if self.confirm_delete { button::danger(t, s) } else { button::secondary(t, s) })
                                    .on_press(VideoMsg::Delete),
                            ]
                            .spacing(10)
                            .align_y(Alignment::Center),
                        );
                    }
                    if let Some((s, pct, _)) = &self.progress {
                        body = body.push(
                            row![
                                column![text(format!("{s} {pct:.0}%")).size(13), progress_bar(0.0..=100.0, *pct).girth(6)]
                                    .spacing(4)
                                    .width(Length::Fill),
                                button(text("Cancel").size(13)).padding([6, 12]).style(button::secondary).on_press(VideoMsg::Cancel),
                            ]
                            .spacing(10)
                            .align_y(Alignment::Center),
                        );
                    }
                    body = body.push(
                        column![
                            heading("YouTube search"),
                            row![
                                text_input("Search YouTube", &self.query).on_input(VideoMsg::Query).on_submit(VideoMsg::Search).padding(7),
                                button(text("Search").size(13))
                                    .padding([7, 12])
                                    .on_press_maybe(self.tools.is_some_and(|t| t.0).then_some(VideoMsg::Search)),
                            ]
                            .spacing(8),
                        ]
                        .spacing(6),
                    );
                    if self.searching {
                        body = body.push(note("Searching…"));
                    }
                    let mut list = column![].spacing(4);
                    for r in &self.results {
                        let meta = [
                            r.by(),
                            r.duration.map(video::fmt_duration).unwrap_or_default(),
                            r.view_count.map(|v| format!("{v} views")).unwrap_or_default(),
                        ]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ");
                        list = list.push(
                            row![
                                column![text(r.title.clone()).size(13), text(meta).size(11)].width(Length::Fill),
                                button(text("Watch").size(12)).padding([4, 10]).style(button::text).on_press(VideoMsg::OpenLink(r.link())),
                                button(text("Download").size(12))
                                    .padding([4, 10])
                                    .on_press_maybe(can_download.then(|| VideoMsg::Download(r.link()))),
                            ]
                            .spacing(8)
                            .align_y(Alignment::Center),
                        );
                    }
                    body = body.push(list);
                    body = body.push(
                        column![
                            heading("Paste a link"),
                            note("YouTube, Vimeo, Dailymotion, Archive.org or any site yt-dlp supports."),
                            row![
                                text_input("https://…", &self.url).on_input(VideoMsg::Url).padding(7),
                                button(text("Download").size(13))
                                    .padding([7, 12])
                                    .on_press_maybe((can_download && self.url.starts_with("http")).then(|| VideoMsg::Download(self.url.clone()))),
                            ]
                            .spacing(8),
                        ]
                        .spacing(6),
                    );
                    body = body.push(
                        row![
                            heading("Local file"),
                            button(text("Choose video file…").size(13)).padding([7, 12]).style(button::secondary).on_press(VideoMsg::ChooseFile),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                    );
                    body = body.push(note(
                        "Downloads are capped at 1080p, video only; .webm is preferred because .mp4 videos don't play on Linux.",
                    ));
                }
            }
        }
        body = body.push(status_line(&self.status));
        row![picker, scrollable(container(body).padding(16)).width(Length::Fill)].into()
    }
}

// ---------- Issue scanner ----------

#[derive(Default)]
struct IssueTool {
    progress: Option<(usize, usize, iced::task::Handle)>,
    result: Option<Result<(usize, usize, Option<PathBuf>), String>>,
    top: Vec<(String, usize)>,
    rows: Vec<crate::issues::IssueRow>,
    /// Only show this issue type.
    only: Option<String>,
    shown: Vec<usize>,
    list: VList,
}

#[derive(Debug, Clone)]
pub enum IssueMsg {
    PickScan,
    PickOut,
    Picked(bool, Option<PathBuf>),
    Start,
    Cancel,
    Event(ScanEvent),
    OpenReport(PathBuf),
    Only(Option<String>),
    Scrolled(scrollable::Viewport),
}

#[derive(Debug, Clone)]
pub enum ScanEvent {
    Found(usize),
    Scanned(usize),
    Done(Result<(usize, Vec<crate::issues::IssueRow>, Option<PathBuf>), String>),
}

fn scan_stream(root: PathBuf, out_dir: PathBuf) -> impl futures::Stream<Item = ScanEvent> {
    iced::stream::channel(16, async move |mut tx: Sender<ScanEvent>| {
        let root2 = root.clone();
        let charts = tokio::task::spawn_blocking(move || crate::issues::find_charts(&root2)).await.unwrap_or_default();
        let total = charts.len();
        let _ = tx.send(ScanEvent::Found(total)).await;
        let charts = Arc::new(charts);
        let mut rows = Vec::new();
        let mut done = 0;
        const CHUNK: usize = 64;
        while done < total {
            let (cs, start) = (charts.clone(), done);
            let part = tokio::task::spawn_blocking(move || {
                let end = (start + CHUNK).min(cs.len());
                std::thread::scope(|s| {
                    let hs: Vec<_> = cs[start..end]
                        .chunks(CHUNK.div_ceil(4))
                        .map(|c| s.spawn(move || c.iter().flat_map(crate::issues::scan_chart).collect::<Vec<_>>()))
                        .collect();
                    hs.into_iter().filter_map(|h| h.join().ok()).flatten().collect::<Vec<_>>()
                })
            })
            .await
            .unwrap_or_default();
            rows.extend(part);
            done = (done + CHUNK).min(total);
            let _ = tx.send(ScanEvent::Scanned(done)).await;
        }
        let result = if rows.is_empty() {
            Ok((total, rows, None))
        } else {
            let r2 = rows.clone();
            tokio::task::spawn_blocking(move || crate::issues::write_xlsx(&r2, &out_dir))
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r)
                .map(|p| (total, rows, Some(p)))
        };
        let _ = tx.send(ScanEvent::Done(result)).await;
    })
}

impl IssueTool {
    fn update(&mut self, m: IssueMsg, cx: &Ctx) -> (Task<IssueMsg>, Out) {
        let s = cx.settings;
        match m {
            IssueMsg::PickScan | IssueMsg::PickOut => {
                let scan = matches!(m, IssueMsg::PickScan);
                return (
                    Task::perform(
                        async move {
                            rfd::AsyncFileDialog::new()
                                .set_title(if scan { "Folder to scan for chart issues" } else { "Where to save the spreadsheet" })
                                .pick_folder()
                                .await
                                .map(|h| h.path().to_path_buf())
                        },
                        move |p| IssueMsg::Picked(scan, p),
                    ),
                    Out::None,
                );
            }
            IssueMsg::Picked(_, None) => {}
            IssueMsg::Picked(scan, Some(p)) => {
                let (a, b) = if scan {
                    (Some(p), s.spreadsheet_output_path.clone())
                } else {
                    (s.issue_scan_path.clone(), Some(p))
                };
                return (Task::none(), Out::SetIssuePaths(a, b));
            }
            IssueMsg::Start => {
                let (Some(root), Some(out)) = (s.issue_scan_path.clone(), s.spreadsheet_output_path.clone()) else {
                    return (Task::none(), Out::None);
                };
                let (task, handle) = Task::run(scan_stream(root, out), IssueMsg::Event).abortable();
                self.progress = Some((0, 0, handle));
                self.result = None;
                return (task, Out::None);
            }
            IssueMsg::Cancel => {
                if let Some((_, _, h)) = self.progress.take() {
                    h.abort();
                }
            }
            IssueMsg::Event(ScanEvent::Found(n)) => {
                if let Some(p) = &mut self.progress {
                    p.1 = n;
                }
            }
            IssueMsg::Event(ScanEvent::Scanned(n)) => {
                if let Some(p) = &mut self.progress {
                    p.0 = n;
                }
            }
            IssueMsg::Event(ScanEvent::Done(r)) => {
                self.progress = None;
                self.top.clear();
                self.only = None;
                self.result = Some(r.map(|(charts, rows, path)| {
                    let mut kinds: HashMap<String, usize> = HashMap::new();
                    for r in &rows {
                        *kinds.entry(r.issue.clone()).or_default() += 1;
                    }
                    let mut top: Vec<_> = kinds.into_iter().collect();
                    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                    self.top = top;
                    let n = rows.len();
                    // Mandatory fixes first, then by chart.
                    let mut rows = rows;
                    rows.sort_by(|a, b| b.mandatory.cmp(&a.mandatory).then(a.path.cmp(&b.path)));
                    self.rows = rows;
                    (charts, n, path)
                }));
                self.filter_rows();
            }
            IssueMsg::OpenReport(p) => return (Task::none(), Out::Open(p)),
            IssueMsg::Only(k) => {
                self.only = if self.only == k { None } else { k };
                self.filter_rows();
            }
            IssueMsg::Scrolled(vp) => self.list.on_scroll(vp),
        }
        (Task::none(), Out::None)
    }

    fn filter_rows(&mut self) {
        self.list.set_id("issue-list");
        self.shown = (0..self.rows.len()).filter(|&i| self.only.as_ref().is_none_or(|k| &self.rows[i].issue == k)).collect();
        self.list.reset();
    }

    fn view<'a>(&'a self, cx: &Ctx<'a>) -> Element<'a, IssueMsg> {
        let s = cx.settings;
        let path_row = |label: &'static str, p: &Option<PathBuf>, msg: IssueMsg| {
            row![
                text(label).size(13).width(170),
                container(
                    text(p.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "Not chosen".into()))
                        .size(13)
                        .wrapping(text::Wrapping::WordOrGlyph),
                )
                .width(Length::Fill),
                button(text("Choose…").size(13)).padding([6, 12]).style(button::secondary).on_press(msg),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
        };
        let ready = s.issue_scan_path.is_some() && s.spreadsheet_output_path.is_some() && self.progress.is_none();
        let mut col = column![
            heading("Chart issue scanner"),
            note("Checks every chart in a folder for the problems Chorus Encore checks for (missing files, bad metadata, broken notes, sustain gaps…) and saves a spreadsheet. Charts are only read, never changed."),
            path_row("Folder to scan", &s.issue_scan_path, IssueMsg::PickScan),
            path_row("Save spreadsheet in", &s.spreadsheet_output_path, IssueMsg::PickOut),
            button(text("Scan for issues").size(14)).padding([8, 16]).on_press_maybe(ready.then_some(IssueMsg::Start)),
        ]
        .spacing(12)
        .max_width(860);
        if let Some((done, total, _)) = &self.progress {
            let label = if *total == 0 { "Finding charts…".to_string() } else { format!("Scanned {done} of {total} charts…") };
            col = col.push(
                row![
                    column![text(label).size(13), progress_bar(0.0..=(*total).max(1) as f32, *done as f32).girth(6)]
                        .spacing(4)
                        .width(Length::Fill),
                    button(text("Stop").size(13)).padding([6, 12]).style(button::secondary).on_press(IssueMsg::Cancel),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        }
        let mut table: Option<Element<'a, IssueMsg>> = None;
        match &self.result {
            Some(Ok((charts, n, path))) => {
                col = col.push(text(format!("{n} issues found in {charts} charts.")).size(14).style(text::success));
                if let Some(p) = path {
                    let folder = p.parent().map(|f| f.to_path_buf()).unwrap_or_default();
                    col = col.push(
                        row![
                            text(p.display().to_string()).size(12).width(Length::Fill).wrapping(text::Wrapping::WordOrGlyph),
                            button(text("Open spreadsheet").size(13)).padding([6, 12]).on_press(IssueMsg::OpenReport(p.clone())),
                            button(text("Show in folder").size(13)).padding([6, 12]).style(button::secondary).on_press(IssueMsg::OpenReport(folder)),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                    );
                }
                let mut chips = row![toggle_chip("All", self.only.is_none(), IssueMsg::Only(None))].spacing(6);
                for (k, n) in &self.top {
                    let active = self.only.as_deref() == Some(k.as_str());
                    chips = chips.push(
                        button(text(format!("{k} {n}")).size(12))
                            .padding([3, 10])
                            .style(move |t: &Theme, s| {
                                let mut b = if active { button::primary(t, s) } else { button::secondary(t, s) };
                                b.border = Border::default().rounded(12);
                                b
                            })
                            .on_press(IssueMsg::Only(Some(k.clone()))),
                    );
                }
                col = col.push(chips.wrap());
                let rows = &self.rows;
                table = Some(
                    column![
                        row![
                            text("Chart").size(12).font(crate::BOLD).width(Length::FillPortion(3)),
                            text("Issue").size(12).font(crate::BOLD).width(Length::FillPortion(2)),
                            text("Details").size(12).font(crate::BOLD).width(Length::FillPortion(6)),
                            text("Must fix").size(12).font(crate::BOLD).width(60),
                        ]
                        .spacing(10)
                        .padding([6, 20]),
                        rule::horizontal(1),
                        self.list.view(
                            self.shown.len(),
                            move |r| {
                                let row_ = &rows[self.shown[r]];
                                let chart = if row_.artist.is_empty() && row_.name.is_empty() {
                                    row_.path.rsplit(['\\', '/']).next().unwrap_or("").to_string()
                                } else {
                                    format!("{} - {}", row_.artist, row_.name)
                                };
                                let cell = |s: String, p: u16| {
                                    container(text(s).size(12).wrapping(text::Wrapping::None)).width(Length::FillPortion(p)).clip(true)
                                };
                                container(
                                    row![
                                        cell(chart, 3),
                                        cell(row_.issue.clone(), 2),
                                        cell(row_.description.clone(), 6),
                                        text(if row_.mandatory { "yes" } else { "" }).size(12).width(60).style(text::danger),
                                    ]
                                    .spacing(10)
                                    .align_y(Alignment::Center),
                                )
                                .padding([0, 20])
                                .height(Length::Fill)
                                .align_y(Alignment::Center)
                                .style(move |t: &Theme| container::Style {
                                    background: (r % 2 == 1).then(|| t.extended_palette().background.weakest.color.into()),
                                    ..Default::default()
                                })
                                .into()
                            },
                            IssueMsg::Scrolled,
                        ),
                    ]
                    .height(Length::Fill)
                    .into(),
                );
            }
            Some(Err(e)) => col = col.push(text(format!("Scan failed: {e}")).size(13).style(text::danger)),
            None => {}
        }
        let top = container(col).padding(20);
        match table {
            Some(t) => column![top, t].height(Length::Fill).into(),
            None => scrollable(top).height(Length::Fill).into(),
        }
    }
}

// ---------- Page ----------

pub struct Page {
    pub tab: Tab,
    art: ArtTool,
    lyrics: LyricsTool,
    video: VideoTool,
    issues: IssueTool,
    scenes: crate::scenes_ui::ScenesTool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Tab(Tab),
    Art(ArtMsg),
    Lyrics(LyricsMsg),
    Video(VideoMsg),
    Issues(IssueMsg),
    Scenes(crate::scenes_ui::Msg),
}

impl Default for Page {
    fn default() -> Self {
        Self {
            tab: Tab::Art,
            art: ArtTool {
                mode: ArtMode::Album,
                picker: Picker::new(),
                artist: String::new(),
                album: String::new(),
                results: Vec::new(),
                thumbs: HashMap::new(),
                searching: false,
                current: None,
                blur: 50,
                busy: false,
                status: None,
                batch: None,
            },
            lyrics: LyricsTool {
                picker: Picker::new(),
                artist: String::new(),
                title: String::new(),
                results: Vec::new(),
                searching: false,
                chosen: None,
                lines: Vec::new(),
                first_ms: 0.0,
                player: None,
                loading_audio: false,
                position_ms: 0.0,
                busy: false,
                confirm_remove: false,
                status: None,
                batch: None,
            },
            video: VideoTool {
                picker: Picker::new(),
                tools: None,
                winget: None,
                installing: false,
                query: String::new(),
                results: Vec::new(),
                searching: false,
                url: String::new(),
                progress: None,
                confirm_delete: false,
                status: None,
            },
            issues: IssueTool::default(),
            scenes: Default::default(),
        }
    }
}

impl Page {
    /// Re-filters the chart lists (call when the catalog or lyrics data changed).
    pub fn refresh(&mut self, cx: &Ctx) {
        self.art.picker.refresh(cx.catalog, self.art.missing());
        let lyr = cx.lyrics.clone();
        self.lyrics.picker.refresh(cx.catalog, move |c| !c.is_sng() && lyr.get(&c.path) == Some(&false));
        self.video.picker.refresh(cx.catalog, |c| !c.is_sng() && !c.has_video);
    }

    /// Work to start when the page is first shown.
    pub fn on_open(&mut self) -> Task<Msg> {
        if self.video.tools.is_none() {
            return Task::perform(video::check_tools(), |t| Msg::Video(VideoMsg::Tools(t)));
        }
        Task::none()
    }

    pub fn update(&mut self, m: Msg, cx: &Ctx) -> (Task<Msg>, Out) {
        self.art.picker.list.set_id("art-list");
        self.lyrics.picker.list.set_id("lyrics-list");
        self.video.picker.list.set_id("video-list");
        let (task, out) = self.update_inner(m, cx);
        let scroll = Task::batch([
            self.art.picker.list.take_scroll_task(),
            self.lyrics.picker.list.take_scroll_task(),
            self.video.picker.list.take_scroll_task(),
            self.issues.list.take_scroll_task(),
        ]);
        (Task::batch([task, scroll]), out)
    }

    fn update_inner(&mut self, m: Msg, cx: &Ctx) -> (Task<Msg>, Out) {
        match m {
            Msg::Tab(Tab::Issues) if !ISSUE_SCANNER_ENABLED => (Task::none(), Out::None),
            Msg::Tab(t) => {
                self.tab = t;
                if t != Tab::Lyrics
                    && let Some(p) = &self.lyrics.player
                {
                    p.pause();
                }
                (Task::none(), Out::None)
            }
            Msg::Art(m) => {
                let (t, o) = self.art.update(m, cx);
                (t.map(Msg::Art), o)
            }
            Msg::Lyrics(m) => {
                let (t, o) = self.lyrics.update(m, cx);
                (t.map(Msg::Lyrics), o)
            }
            Msg::Video(m) => {
                let (t, o) = self.video.update(m, cx);
                (t.map(Msg::Video), o)
            }
            Msg::Issues(m) => {
                let (t, o) = self.issues.update(m, cx);
                (t.map(Msg::Issues), o)
            }
            Msg::Scenes(m) => (self.scenes.update(m).map(Msg::Scenes), Out::None),
        }
    }

    /// Runs on every page: the scene switcher's OBS connection.
    pub fn background(&self) -> Subscription<Msg> {
        self.scenes.background().map(Msg::Scenes)
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        if self.tab == Tab::Scenes {
            return self.scenes.subscription().map(Msg::Scenes);
        }
        if self.lyrics.player.as_ref().is_some_and(|p| p.is_playing()) {
            iced::time::every(std::time::Duration::from_millis(50)).map(|_| Msg::Lyrics(LyricsMsg::Tick))
        } else {
            Subscription::none()
        }
    }

    /// Stops lyric preview audio (when leaving the page).
    pub fn pause_audio(&self) {
        if let Some(p) = &self.lyrics.player {
            p.pause();
        }
    }

    pub fn view<'a>(&'a self, cx: &Ctx<'a>) -> Element<'a, Msg> {
        let tab = |label: &'static str, t: Tab| {
            let active = self.tab == t;
            button(text(label).size(14))
                .padding([7, 14])
                .style(move |th: &Theme, s| if active { button::primary(th, s) } else { button::subtle(th, s) })
                .on_press(Msg::Tab(t))
        };
        let mut tab_row = row![tab("Art Studio", Tab::Art), tab("Lyrics", Tab::Lyrics), tab("Video Sync", Tab::Video), tab("Scene Switcher", Tab::Scenes)];
        if ISSUE_SCANNER_ENABLED {
            tab_row = tab_row.push(tab("Issue scanner", Tab::Issues));
        }
        let tabs = container(
            tab_row
                .push(space().width(Length::Fill))
                .push(text(if self.tab == Tab::Scenes { "" } else { "Changed files are backed up to your app data folder first." }).size(11))
                .spacing(6)
                .align_y(Alignment::Center),
        )
        .padding([8, 12]);
        let needs_library = !matches!(self.tab, Tab::Issues | Tab::Scenes) && cx.catalog.is_empty();
        let body: Element<'a, Msg> = if needs_library {
            container(note("Add your songs folders in Settings (and let the library scan finish) to use this tool."))
                .center(Length::Fill)
                .into()
        } else {
            match self.tab {
                Tab::Art => self.art.view(cx).map(Msg::Art),
                Tab::Lyrics => self.lyrics.view(cx).map(Msg::Lyrics),
                Tab::Video => self.video.view(cx).map(Msg::Video),
                Tab::Issues => self.issues.view(cx).map(Msg::Issues),
                Tab::Scenes => self.scenes.view().map(Msg::Scenes),
            }
        };
        column![tabs, rule::horizontal(1), body].height(Length::Fill).into()
    }
}
