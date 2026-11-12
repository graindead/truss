#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod advanced;
mod api;
mod audio;
mod chart;
mod download;
mod extras;
mod icons;
mod issues;
mod leaderboard;
mod leaderboard_ui;
mod library;
mod library_ui;
mod lyrics;
mod preview;
mod preview_panel;
#[cfg(feature = "rhythmverse")]
mod rhythmverse;
#[cfg(feature = "rhythmverse")]
mod rv_ui;
#[cfg(feature = "rhythmverse")]
mod rv_catalog;
#[cfg(feature = "rhythmverse")]
mod zip_ranges;
mod scenes;
mod scenes_ui;
mod settings;
mod style;
mod updater;
mod setlists;
mod sng;
mod textures;
mod trackhash;
mod taskbar_icon;
mod themes;
mod backdrop;
mod icon_paint;
mod fretwork;
mod fretwork_filter;
mod tools_ui;
mod variants;
mod vlist;

use api::{Chart, DecodedImage, DIFFICULTIES, DRUM_TYPES, INSTRUMENTS, strip_tags};
use iced::widget::{
    button, center, checkbox, column, container, image, pick_list, progress_bar, row, rule, scrollable, space, text,
    text_input, toggler,
};
use iced::{Alignment, Background, Border, Color, Element, Font, Length, Subscription, Task, Theme};
use extras::{library_mark, rv_tag};
use settings::Settings;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const SEARCH_ID: &str = "search";
const BOLD: Font = Font { weight: iced::font::Weight::Bold, ..Font::DEFAULT };

/// TRUSS_LOG=<level> prints library log messages (e.g. the graphics backend's) to stderr.
struct StderrLog;
impl log::Log for StderrLog {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        eprintln!("[{} {}] {}", r.level(), r.target(), r.args());
    }
    fn flush(&self) {}
}

fn main() -> iced::Result {
    if let Some(level) = std::env::var("TRUSS_LOG").ok().and_then(|l| l.parse::<log::LevelFilter>().ok()) {
        static LOGGER: StderrLog = StderrLog;
        if log::set_logger(&LOGGER).is_ok() {
            log::set_max_level(level);
        }
    }
    settings::migrate_old_data();
    // Pick the renderer before iced starts. SAFETY: no other threads exist yet.
    let settings = Settings::load();
    unsafe {
        if settings.cpu_rendering && std::env::var_os("ICED_BACKEND").is_none() {
            std::env::set_var("ICED_BACKEND", "tiny-skia");
        }
        // The async runtime otherwise starts one worker per CPU core; this app is network-bound
        // and only ever needs a handful.
        if std::env::var_os("TOKIO_WORKER_THREADS").is_none() {
            std::env::set_var("TOKIO_WORKER_THREADS", "4");
        }
        // DX12 starts faster and uses ~40% less memory than Vulkan/GL on Windows.
        #[cfg(windows)]
        if std::env::var_os("WGPU_BACKEND").is_none() {
            std::env::set_var("WGPU_BACKEND", "dx12");
        }
        // A transparent window needs DX12 to present through a composition visual.
        #[cfg(windows)]
        if active_backdrop(&settings) != backdrop::Backdrop::Off && std::env::var_os("WGPU_DX12_PRESENTATION_SYSTEM").is_none() {
            std::env::set_var("WGPU_DX12_PRESENTATION_SYSTEM", "Visual");
        }
    }
    let transparent = active_backdrop(&settings) != backdrop::Backdrop::Off;

    iced::application(App::boot, App::update, App::view)
        .title("Truss")
        .theme(App::theme)
        .style(App::app_style)
        .subscription(App::subscription)
        .window(iced::window::Settings {
            size: iced::Size::new(1280.0, 800.0),
            icon: app_icon(&theme_by_name(&settings.theme))
                .or_else(|| iced::window::icon::from_file_data(include_bytes!("../assets/truss-256.png"), None).ok()),
            transparent,
            ..Default::default()
        })
        .antialiasing(false)
        .run()
}

/// Where the GitHub update is at.
#[derive(Debug, Clone)]
enum UpdateState {
    Idle,
    Checking { manual: bool },
    UpToDate,
    Available(updater::Release),
    Installing(updater::Release, f32),
    /// Installed; takes effect on restart.
    Ready(String),
    Failed(String),
}

// ---------- Filter option wrappers for pick lists ----------

#[derive(Debug, Clone, PartialEq)]
struct Opt {
    key: Option<&'static str>,
    label: &'static str,
}
impl std::fmt::Display for Opt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label)
    }
}

fn instrument_opts() -> Vec<Opt> {
    std::iter::once(Opt { key: None, label: "Any Instrument" })
        .chain(INSTRUMENTS.iter().map(|(k, l)| Opt { key: Some(k), label: l }))
        .collect()
}
fn difficulty_opts() -> Vec<Opt> {
    std::iter::once(Opt { key: None, label: "Any Difficulty" })
        .chain(DIFFICULTIES.iter().map(|(k, l)| Opt { key: Some(k), label: l }))
        .collect()
}
fn drum_type_opts() -> Vec<Opt> {
    std::iter::once(Opt { key: None, label: "Any Drum Type" })
        .chain(DRUM_TYPES.iter().map(|(k, l)| Opt { key: Some(k), label: l }))
        .collect()
}
fn reviewed_opts() -> Vec<Opt> {
    vec![Opt { key: None, label: "Any Quality" }, Opt { key: Some("reviewed"), label: "Quality Reviewed" }]
}
fn find_opt(opts: &[Opt], key: Option<&str>) -> Opt {
    opts.iter().find(|o| o.key == key).cloned().unwrap_or_else(|| opts[0].clone())
}

// ---------- State ----------

enum Art {
    Loading,
    /// `full` (compressed JPEG) is kept only for the most recently loaded/selected charts.
    Ready { thumb: image::Handle, full: Option<image::Handle> },
    Failed,
}

/// How many full-size album covers stay in memory.
const FULL_ART_KEEP: usize = 24;
/// Thumbnails are kept for this many results on either side of what's on screen.
const ART_WINDOW: usize = 150;
/// Album art downloads at once; the rest wait in `art_queue`, nearest the view first.
const ART_PARALLEL: usize = 8;

/// Downloads that run at the same time; the rest wait in the queue.
const MAX_ACTIVE_DOWNLOADS: usize = 2;

#[derive(Debug, Clone)]
enum DlStatus {
    Queued,
    Active { status: String, done: u64, total: Option<u64> },
    Done(PathBuf),
    Failed(String),
}

struct DownloadEntry {
    id: u64,
    md5: String,
    chart: Arc<Chart>,
    /// Old copies to replace once this download succeeds (a redownload).
    replace: Vec<PathBuf>,
    title: String,
    status: DlStatus,
    handle: Option<iced::task::Handle>,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Screen {
    Browse,
    Library,
    Tools,
    Settings,
}

struct App {
    settings: Settings,
    screen: Screen,
    theme: Theme,
    /// The window transparency in effect (chosen at start; Settings changes apply on restart).
    backdrop: backdrop::Backdrop,

    query: String,
    instrument: Opt,
    difficulty: Opt,
    instrument_opts: Vec<Opt>,
    difficulty_opts: Vec<Opt>,
    sort: Option<api::Sort>,

    advanced_open: bool,
    advanced: advanced::AdvancedForm,
    /// Filters of the advanced search currently shown (used for paging). `None` = quick search.
    active_advanced: Option<Arc<api::AdvancedFilters>>,

    /// One entry per song (the version currently chosen for it).
    results: Vec<Arc<Chart>>,
    /// Every version of each song, newest first, by `Chart::group_key`.
    versions: HashMap<i64, Vec<Arc<Chart>>>,
    found: u64,
    page: u32,
    loading: bool,
    error: Option<String>,
    /// Incremented on every new search so stale responses and debounces are ignored.
    generation: u64,
    selected: Option<usize>,
    /// Results ticked with Ctrl/Shift-click for a batch download.
    checked: BTreeSet<usize>,
    /// Where the last click was, for Shift-click ranges.
    anchor: Option<usize>,
    modifiers: iced::keyboard::Modifiers,
    /// Window size, for fitting the layout to small windows.
    win: iced::Size,
    /// Bumped when row contents change without the list length changing (lazy list key).
    list_rev: u64,
    drum_type: Opt,
    drums_reviewed: Opt,
    drum_type_opts: Vec<Opt>,
    reviewed_opts: Vec<Opt>,
    show_issues: bool,
    show_versions: bool,
    /// The changelog under Settings > Updates is open.
    show_changelog: bool,
    /// The Fretwork tier filter of Advanced search and its rating queue.
    fw: fretwork_filter::FwSearch,
    /// Leaderboards fetched this session, by track.
    boards: HashMap<leaderboard::Track, Result<leaderboard::Board, String>>,
    /// The leaderboard last picked (instrument, difficulty), kept as you move between charts.
    board_pick: Option<(String, String)>,
    /// Leaderboard hashes Truss worked out from charts' notes, by chart md5.
    hashes: HashMap<String, leaderboard::Hashes>,

    /// Charts already in the library folders (see `library::Library`).
    library: HashMap<String, Vec<PathBuf>>,
    library_charts: usize,
    /// Chart (md5) whose Delete button was clicked once and is waiting for the confirming click.
    confirm_delete: Option<String>,
    delete_error: Option<String>,
    library_scanning: bool,
    /// Every chart in the library folders (Library page and tools).
    catalog: Vec<Arc<library::LocalChart>>,
    /// Which local charts have lyrics; filled in the first time Library/Tools is opened.
    lyrics_map: HashMap<PathBuf, bool>,
    lyrics_scanning: bool,
    /// Downloads/deletes happened since the last scan.
    catalog_stale: bool,
    library_error: Option<String>,
    library_page: library_ui::Page,
    tools_page: tools_ui::Page,
    /// Search generation whose old results were already cleared (results are replaced by the
    /// first page to arrive from any source).
    cleared_generation: u64,
    /// Encore songs listed so far in this search (to know whether Encore has more pages).
    encore_seen: u64,
    #[cfg(feature = "rhythmverse")]
    rv: rv_ui::RvState,

    art: HashMap<String, Art>,
    full_art_lru: std::collections::VecDeque<String>,
    /// Album art waiting to be fetched, nearest the visible rows first.
    art_queue: std::collections::VecDeque<String>,
    /// Album art being fetched now.
    art_inflight: HashSet<String>,
    icon_list: Option<Arc<Vec<icons::IconEntry>>>,
    /// Icon images by content hash; `None` while loading or if unavailable.
    icons: HashMap<String, Option<image::Handle>>,
    icon_version: u64,
    /// Approximate index of the result currently on screen (drives the thumbnail window).
    art_center: usize,
    /// Bumped whenever album art finishes loading (part of the results list's lazy key).
    art_version: u64,
    downloads: Vec<DownloadEntry>,
    next_dl_id: u64,
    show_downloads: bool,

    preview: Option<preview_panel::Panel>,
    update: UpdateState,
    ui: variants::UiVariant,
}

#[derive(Debug, Clone)]
enum Message {
    QueryChanged(String),
    Debounced(u64),
    SubmitSearch,
    /// Clicked an artist, charter, genre… in the details: search for it.
    SearchFor(String),
    InstrumentPicked(Opt),
    DifficultyPicked(Opt),
    DrumTypePicked(Opt),
    DrumsReviewedPicked(Opt),
    ModifiersChanged(iced::keyboard::Modifiers),
    WindowResized(iced::Size),
    PickVersion(usize, usize),
    ToggleIssues,
    ToggleLeaderboard,
    /// (instrument, difficulty)
    PickLeaderboard(String, String),
    LeaderboardLoaded(leaderboard::Track, Result<leaderboard::Board, String>),
    /// Read this chart (md5) to work out its leaderboard hashes.
    FindScores(String),
    HashesFound(String, Result<trackhash::Computed, String>),
    /// A search result rated for the Fretwork filter (generation, md5, reading).
    FwRated(u64, String, Result<trackhash::Computed, String>),
    ToggleVersions,
    ToggleChangelog,
    DownloadChecked,
    ClearChecked,
    /// A result's tick box was clicked.
    ToggleChecked(usize),
    CheckUpdates,
    UpdateChecked(Result<Option<updater::Release>, String>),
    InstallUpdate,
    UpdateEvent(updater::InstallEvent),
    RestartForUpdate,
    ToggleCheckUpdates(bool),
    ToggleRefined(bool),
    ToggleFretwork(bool),
    BackdropPicked(backdrop::Backdrop),
    /// Dragging the see-through slider (applies live; saved on release).
    GlassTransparency(f32),
    SaveSettings,
    /// The tick box in the list header: tick every result, or none.
    CheckAll(bool),
    RetryDownload(u64),
    Redownload(usize),
    DeleteChart(usize),
    Deleted(String, Vec<PathBuf>, Result<(), String>),
    CopyText(String),
    OpenUrl(String),
    LibraryScanned(Arc<library::Library>),
    #[cfg(feature = "rhythmverse")]
    RvDone { generation: u64, append: bool, result: Result<Arc<rhythmverse::RvPage>, String> },
    #[cfg(feature = "rhythmverse")]
    RvSync(rv_catalog::SyncEvent),
    #[cfg(feature = "rhythmverse")]
    RvSyncNow,
    #[cfg(feature = "rhythmverse")]
    RvRebuild,
    #[cfg(feature = "rhythmverse")]
    ToggleEncore(bool),
    #[cfg(feature = "rhythmverse")]
    ToggleRhythmVerse(bool),
    LibraryPage(library_ui::Msg),
    ToolsPage(tools_ui::Msg),
    LyricsScanned(HashMap<PathBuf, bool>),
    LibraryDeleted(Result<(), String>),
    RescanLibrary,
    AddLibraryFolder,
    LibraryFolderAdded(Option<PathBuf>),
    RemoveLibraryFolder(usize),
    MakeDefaultLibrary(usize),
    SortBy(api::SortKey),
    ToggleAdvanced,
    Advanced(advanced::Msg),
    SearchDone { generation: u64, append: bool, result: Result<api::SearchResponse, String> },
    Scrolled(scrollable::Viewport),
    Select(usize),
    ArtLoaded(String, Result<(DecodedImage, bytes::Bytes), String>),
    IconList(Result<Arc<Vec<icons::IconEntry>>, String>),
    IconLoaded(String, Result<bytes::Bytes, String>),

    Download(usize),
    DlProgress(u64, download::Progress),
    CancelDownload(u64),
    ClearFinished,
    ToggleDownloads,
    OpenPreview(usize),
    Preview(preview_panel::Msg),
    OpenPath(PathBuf),

    ShowScreen(Screen),
    FolderTemplateChanged(String),
    ToggleSng(bool),
    ToggleVideos(bool),
    ThemePicked(ThemeChoice),
    /// The taskbar pin's icon was updated (or not); nothing to do.
    PinIconDone,
    LayoutPicked(variants::UiVariant),
    FpsPicked(FpsChoice),
    ToggleCpuRendering(bool),
    Saved(Result<(), String>),
}

/// Chart preview frame-rate option (0 = match display).
#[derive(Debug, Clone, Copy, PartialEq)]
struct FpsChoice(u32);
impl FpsChoice {
    const ALL: [FpsChoice; 3] = [FpsChoice(60), FpsChoice(120), FpsChoice(0)];
}
impl std::fmt::Display for FpsChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            0 => f.write_str("Match display (smoothest)"),
            n => write!(f, "{n} FPS"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ThemeChoice(Theme);
impl std::fmt::Display for ThemeChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

fn all_themes() -> Vec<ThemeChoice> {
    themes::all().into_iter().map(ThemeChoice).collect()
}

/// The transparency to use this session (none with CPU rendering, which can't show it).
fn active_backdrop(s: &Settings) -> backdrop::Backdrop {
    if s.cpu_rendering { backdrop::Backdrop::Off } else { backdrop::Backdrop::from_key(&s.window_backdrop) }
}

/// Points this exe's taskbar pins at the theme's icon embedded in truss.exe (see build.rs), or
/// back at the main icon for the first (Rust) theme.
fn pin_icon_task(theme: &Theme) -> Task<Message> {
    let name = theme.to_string();
    let Some(i) = themes::all().iter().position(|t| t.to_string() == name) else { return Task::none() };
    let id = (i > 0).then(|| themes::ICON_ID_BASE + i as u16);
    Task::perform(async move { tokio::task::spawn_blocking(move || taskbar_icon::apply(id)).await }, |_| {
        Message::PinIconDone
    })
}

/// The window icon in a theme's colours (see icon_paint.rs).
fn app_icon(theme: &Theme) -> Option<iced::window::Icon> {
    let (rgba, w, h) = app_icon_rgba(theme)?;
    iced::window::icon::from_rgba(rgba, w, h).ok()
}

fn app_icon_rgba(theme: &Theme) -> Option<(Vec<u8>, u32, u32)> {
    let (accent, bg) = themes::icon_colors(theme);
    icon_paint::recolor(accent, bg)
}

fn theme_by_name(name: &str) -> Theme {
    all_themes().into_iter().map(|t| t.0).find(|t| t.to_string() == name).unwrap_or_else(themes::rust_theme)
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let settings = Settings::load();
        style::set_refined(settings.refined_ui);
        let instrument_opts = instrument_opts();
        let difficulty_opts = difficulty_opts();
        let drum_type_opts = drum_type_opts();
        let reviewed_opts = reviewed_opts();
        let mut app = App {
            drum_type: find_opt(&drum_type_opts, settings.drum_type.as_deref()),
            drums_reviewed: reviewed_opts[settings.drums_reviewed as usize].clone(),
            drum_type_opts,
            reviewed_opts,
            versions: HashMap::new(),
            checked: BTreeSet::new(),
            anchor: None,
            modifiers: Default::default(),
            win: iced::Size::new(1280.0, 800.0),
            list_rev: 0,
            show_issues: false,
            show_versions: false,
            show_changelog: false,
            fw: Default::default(),
            boards: HashMap::new(),
            board_pick: None,
            hashes: HashMap::new(),
            library: HashMap::new(),
            confirm_delete: None,
            delete_error: None,
            library_charts: 0,
            library_scanning: false,
            catalog: Vec::new(),
            lyrics_map: HashMap::new(),
            lyrics_scanning: false,
            catalog_stale: false,
            library_error: None,
            library_page: Default::default(),
            tools_page: Default::default(),
            cleared_generation: 0,
            encore_seen: 0,
            #[cfg(feature = "rhythmverse")]
            rv: Default::default(),
            theme: theme_by_name(&settings.theme),
            backdrop: active_backdrop(&settings),
            instrument: find_opt(&instrument_opts, settings.instrument.as_deref()),
            difficulty: find_opt(&difficulty_opts, settings.difficulty.as_deref()),
            instrument_opts,
            difficulty_opts,
            sort: None,
            advanced_open: false,
            advanced: Default::default(),
            active_advanced: None,
            screen: if settings.library_path.is_none() { Screen::Settings } else { Screen::Browse },
            settings,
            query: String::new(),
            results: Vec::new(),
            found: 0,
            page: 1,
            loading: false,
            error: None,
            generation: 0,
            selected: None,
            art: HashMap::new(),
            full_art_lru: Default::default(),
            art_queue: Default::default(),
            art_inflight: Default::default(),
            icon_list: None,
            icons: HashMap::new(),
            icon_version: 0,
            art_center: 0,
            art_version: 0,
            downloads: Vec::new(),
            next_dl_id: 0,
            show_downloads: false,
            preview: None,
            update: UpdateState::Idle,
            ui: variants::UiVariant::Default,
        };
        app.ui = variants::UiVariant::from_key(&app.settings.layout);
        // Clear preview temp files left by a crash, off the UI thread.
        std::thread::spawn(preview::cleanup_temp_files);
        updater::cleanup_old();
        let update_check = if app.settings.check_updates {
            app.update = UpdateState::Checking { manual: false };
            Task::perform(updater::check(), Message::UpdateChecked)
        } else {
            Task::none()
        };
        let icon_list = Task::perform(icons::fetch_list(), Message::IconList);
        let task = Task::batch([app.search(false), icon_list, app.scan_library(), update_check]);
        #[cfg(feature = "rhythmverse")]
        let task = Task::batch([task, app.rv_auto_sync(), pin_icon_task(&app.theme), app.backdrop_task()]);
        (app, Task::batch([task, iced::widget::operation::focus(SEARCH_ID)]))
    }

    fn theme(&self) -> Theme {
        self.theme.clone()
    }

    /// The window background: see-through over a backdrop when transparency is on.
    fn app_style(&self, theme: &Theme) -> iced::theme::Style {
        let p = theme.extended_palette();
        let mut bg = p.background.base.color;
        // The window presents pre-multiplied alpha, so the colour is scaled by it too.
        let a = self.backdrop.alpha(self.settings.glass_transparency);
        bg = Color { r: bg.r * a, g: bg.g * a, b: bg.b * a, a };
        iced::theme::Style { background_color: bg, text_color: p.background.base.text }
    }

    /// Asks Windows to blur what's behind the window, tinted with the theme's background.
    fn backdrop_task(&self) -> Task<Message> {
        let b = self.backdrop;
        if b == backdrop::Backdrop::Off {
            return Task::none();
        }
        let c = self.theme.palette().background;
        let tint = [c.r, c.g, c.b].map(|v| (v * 255.0).round() as u8);
        // Applied again shortly after, in case the window wasn't fully set up the first time.
        let at = |delay_ms: u64| {
            Task::perform(tokio::time::sleep(std::time::Duration::from_millis(delay_ms)), |_| ())
                .then(|_| iced::window::oldest())
                .and_then(iced::window::raw_id::<Message>)
                .map(move |hwnd| {
                    backdrop::apply(hwnd, b, tint);
                    Message::PinIconDone
                })
        };
        Task::batch([at(0), at(500)])
    }

    fn subscription(&self) -> Subscription<Message> {
        let modifiers = iced::event::listen_with(|e, _, _| match e {
            iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(m)) => Some(Message::ModifiersChanged(m)),
            iced::Event::Window(iced::window::Event::Resized(size)) => Some(Message::WindowResized(size)),
            _ => None,
        });
        let tools = Subscription::batch([
            if self.screen == Screen::Tools { self.tools_page.subscription().map(Message::ToolsPage) } else { Subscription::none() },
            self.tools_page.background().map(Message::ToolsPage),
        ]);
        match &self.preview {
            Some(p) => {
                Subscription::batch([modifiers, tools, p.subscription(self.settings.preview_fps).map(Message::Preview)])
            }
            None => Subscription::batch([modifiers, tools]),
        }
    }

    fn library_ctx(&self) -> library_ui::Ctx<'_> {
        library_ui::Ctx {
            catalog: &self.catalog,
            lyrics: &self.lyrics_map,
            scanning: self.library_scanning,
            lyrics_scanning: self.lyrics_scanning,
            has_folders: !self.settings.all_library_folders().is_empty(),
            error: self.library_error.as_deref(),
        }
    }

    fn tools_ctx(&self) -> tools_ui::Ctx<'_> {
        tools_ui::Ctx { settings: &self.settings }
    }

    /// Re-filters the Library list after the catalog or lyrics data changed.
    fn refresh_pages(&mut self) {
        let mut lp = std::mem::take(&mut self.library_page);
        lp.refresh(&self.library_ctx());
        self.library_page = lp;
    }

    /// Starts the (cached) check for which charts have lyrics, once.
    fn scan_lyrics(&mut self) -> Task<Message> {
        if self.lyrics_scanning || self.catalog.is_empty() {
            return Task::none();
        }
        self.lyrics_scanning = true;
        Task::perform(lyrics::scan(self.catalog.clone()), Message::LyricsScanned)
    }

    fn scan_library(&mut self) -> Task<Message> {
        let folders = self.settings.all_library_folders();
        if folders.is_empty() {
            self.library.clear();
            self.library_charts = 0;
            self.list_rev += 1;
            return Task::none();
        }
        self.library_scanning = true;
        Task::perform(library::scan(folders), Message::LibraryScanned)
    }

    /// Encore charts always; RhythmVerse charts when Truss can download them.
    pub fn can_preview(&self, c: &Chart) -> bool {
        #[cfg(feature = "rhythmverse")]
        if let Some(rv) = &c.rv {
            return rhythmverse::can_download(rv);
        }
        let _ = c;
        true
    }

    pub fn in_library(&self, c: &Chart) -> bool {
        !self.library.is_empty() && self.library.get(&c.library_key()).is_some_and(|p| !p.is_empty())
    }

    /// The library copies of a chart that may safely be removed.
    pub fn library_paths(&self, c: &Chart) -> Vec<PathBuf> {
        let roots = self.settings.all_library_folders();
        self.library
            .get(&c.library_key())
            .map(|ps| ps.iter().filter(|p| library::is_deletable_chart(p, &roots)).cloned().collect())
            .unwrap_or_default()
    }

    /// Adds search results, folding extra versions of a song into its existing row.
    fn ingest(&mut self, data: Vec<Arc<Chart>>) {
        let mut touched = Vec::new();
        for c in data {
            let k = c.group_key();
            match self.versions.get_mut(&k) {
                Some(v) => {
                    v.push(c);
                    touched.push(k);
                }
                None => {
                    self.versions.insert(k, vec![c.clone()]);
                    self.results.push(c);
                }
            }
        }
        if touched.is_empty() {
            return;
        }
        for k in &touched {
            if let Some(v) = self.versions.get_mut(k) {
                // ISO timestamps sort correctly as strings.
                v.sort_by(|a, b| b.modified_time.cmp(&a.modified_time));
            }
        }
        // Show the newest version of each song, unless it's the one open in the sidebar.
        for (i, r) in self.results.iter_mut().enumerate() {
            let k = r.group_key();
            if Some(i) != self.selected && touched.contains(&k) {
                *r = self.versions[&k][0].clone();
            }
        }
        self.list_rev += 1;
    }

    pub fn versions_of(&self, c: &Chart) -> &[Arc<Chart>] {
        self.versions.get(&c.group_key()).map(Vec::as_slice).unwrap_or_default()
    }

    /// Adds a chart to the download queue (ignored if it's already queued or downloading).
    fn enqueue(&mut self, chart: Arc<Chart>, replace: Vec<PathBuf>) {
        if self
            .downloads
            .iter()
            .any(|d| d.md5 == chart.md5 && matches!(d.status, DlStatus::Active { .. } | DlStatus::Queued))
        {
            return;
        }
        let id = self.next_dl_id;
        self.next_dl_id += 1;
        let title = format!(
            "{} — {}",
            strip_tags(chart.name.as_deref().unwrap_or("?")),
            strip_tags(chart.artist.as_deref().unwrap_or("?"))
        );
        self.downloads.push(DownloadEntry {
            id,
            md5: chart.md5.clone(),
            chart,
            replace,
            title,
            status: DlStatus::Queued,
            handle: None,
        });
        self.show_downloads = true;
    }

    /// Starts queued downloads while fewer than `MAX_ACTIVE_DOWNLOADS` are running.
    fn pump_downloads(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        let mut active = self.downloads.iter().filter(|d| matches!(d.status, DlStatus::Active { .. })).count();
        for d in self.downloads.iter_mut() {
            if active >= MAX_ACTIVE_DOWNLOADS {
                break;
            }
            if !matches!(d.status, DlStatus::Queued) {
                continue;
            }
            let id = d.id;
            let (task, handle) = Task::run(download::start(d.chart.clone(), self.settings.clone(), d.replace.clone()), move |p| {
                Message::DlProgress(id, p)
            })
            .abortable();
            d.status = DlStatus::Active { status: "Starting...".into(), done: 0, total: None };
            d.handle = Some(handle);
            tasks.push(task);
            active += 1;
        }
        Task::batch(tasks)
    }

    fn search(&mut self, append: bool) -> Task<Message> {
        if !append {
            self.generation += 1;
            self.page = 1;
            self.encore_seen = 0;
            self.fw_reset();
        } else {
            self.page += 1;
        }
        #[cfg(feature = "rhythmverse")]
        let rv_task = self.rv_search(append);
        #[cfg(feature = "rhythmverse")]
        if !self.settings.source_encore {
            self.loading = false;
            if !append {
                self.found = 0;
                if !self.rv_enabled() {
                    let clear = self.clear_results_if_new();
                    self.error = Some("No chart sources are turned on (Settings > Chart sources).".into());
                    return clear;
                }
            }
            return rv_task;
        }
        self.loading = true;
        let generation = self.generation;
        let mut common = api::Common::new(
            self.page,
            self.instrument.key.map(String::from),
            self.difficulty.key.map(String::from),
            self.sort,
        );
        if self.instrument.key == Some("drums") {
            common.drum_type = self.drum_type.key.map(String::from);
            common.drums_reviewed = self.drums_reviewed.key.is_some();
        }
        let done = move |result| Message::SearchDone { generation, append, result };
        let encore = match &self.active_advanced {
            Some(filters) => Task::perform(api::advanced_search(filters.clone(), common), done),
            None => Task::perform(api::search(self.query.clone(), common), done),
        };
        #[cfg(feature = "rhythmverse")]
        return Task::batch([encore, rv_task]);
        #[cfg(not(feature = "rhythmverse"))]
        encore
    }

    /// Replaces the old results the first time any source answers a new search.
    pub fn clear_results_if_new(&mut self) -> Task<Message> {
        if self.cleared_generation == self.generation {
            return Task::none();
        }
        self.cleared_generation = self.generation;
        self.results.clear();
        self.versions.clear();
        self.checked.clear();
        self.anchor = None;
        self.selected = None;
        self.art_center = 0;
        self.close_preview()
    }

    /// Total results across sources (for "N charts").
    pub fn total_found(&self) -> u64 {
        #[cfg(feature = "rhythmverse")]
        return self.found + if self.rv_enabled() { self.rv.found } else { 0 };
        #[cfg(not(feature = "rhythmverse"))]
        self.found
    }

    /// Whether more results can be loaded by scrolling.
    pub fn more_available(&self) -> bool {
        let encore = self.encore_seen < self.found || self.fw_pending();
        #[cfg(feature = "rhythmverse")]
        return encore || self.rv_more() || self.rv_pending();
        #[cfg(not(feature = "rhythmverse"))]
        encore
    }

    /// A search is still waiting on a source.
    pub fn searching(&self) -> bool {
        #[cfg(feature = "rhythmverse")]
        return self.loading || self.rv.loading;
        #[cfg(not(feature = "rhythmverse"))]
        self.loading
    }

    /// Thumbnail resolution for the current layout (grid tiles are much larger than list rows).
    fn thumb_px(&self) -> u32 {
        if self.ui == variants::UiVariant::Grid { 192 } else { 88 }
    }

    fn fetch_art(&self, md5: String) -> Task<Message> {
        let px = self.thumb_px();
        Task::perform(api::fetch_album_art(md5.clone(), px), move |r| Message::ArtLoaded(md5.clone(), r))
    }

    /// Queues a cover to load, ahead of the others (e.g. the selected chart's).
    fn load_art(&mut self, md5: Option<&String>) -> Task<Message> {
        let Some(md5) = md5 else { return Task::none() };
        if self.art.contains_key(md5) {
            return Task::none();
        }
        self.art.insert(md5.clone(), Art::Loading);
        self.art_queue.push_front(md5.clone());
        self.pump_art()
    }

    /// Starts queued art downloads up to `ART_PARALLEL` at once.
    fn pump_art(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        while self.art_inflight.len() < ART_PARALLEL {
            let Some(md5) = self.art_queue.pop_front() else { break };
            if !matches!(self.art.get(&md5), Some(Art::Loading)) || !self.art_inflight.insert(md5.clone()) {
                continue;
            }
            tasks.push(self.fetch_art(md5));
        }
        Task::batch(tasks)
    }

    /// Loads thumbnails for results near the visible area and frees the ones far away, so memory
    /// stays flat no matter how far a search is scrolled.
    fn refresh_art_window(&mut self) -> Task<Message> {
        if self.results.is_empty() {
            return Task::none();
        }
        let center = self.art_center.min(self.results.len() - 1);
        let lo = center.saturating_sub(ART_WINDOW);
        let hi = (center + ART_WINDOW).min(self.results.len());
        let wanted: Vec<String> = self.results[lo..hi].iter().filter_map(|c| c.album_art_md5.clone()).collect();

        let selected = self.selected.and_then(|i| self.results.get(i)).and_then(|c| c.album_art_md5.clone());
        {
            let keep: std::collections::HashSet<&String> = wanted.iter().chain(selected.as_ref()).collect();
            // Covers queued for rows that have scrolled away aren't needed any more.
            let stale: Vec<String> = self.art_queue.iter().filter(|m| !keep.contains(m)).cloned().collect();
            for m in &stale {
                if !self.art_inflight.contains(m) {
                    self.art.remove(m);
                }
            }
            self.art_queue.retain(|m| keep.contains(m));
            if self.art.len() > 2 * ART_WINDOW + 50 {
                let before = self.art.len();
                // Keep in-flight loads so their results aren't wasted.
                let inflight = &self.art_inflight;
                self.art.retain(|k, _| keep.contains(k) || inflight.contains(k));
                if self.art.len() != before {
                    self.full_art_lru.retain(|m| self.art.contains_key(m));
                    self.art_version += 1;
                }
            }
        }
        // Queue the visible rows' covers first, then outwards.
        let mut order: Vec<(usize, &String)> = self.results[lo..hi]
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.album_art_md5.as_ref().map(|m| ((lo + i).abs_diff(center), m)))
            .collect();
        order.sort_by_key(|(d, _)| *d);
        let fresh: Vec<String> = order.into_iter().map(|(_, m)| m.clone()).filter(|m| !self.art.contains_key(m)).collect();
        for m in fresh {
            self.art.insert(m.clone(), Art::Loading);
            self.art_queue.push_back(m);
        }
        let mut tasks = vec![self.pump_art()];
        // Grid tiles show icons too; other layouts only show the selected chart's icon.
        if self.ui == variants::UiVariant::Grid {
            let charts: Vec<Arc<Chart>> = self.results[lo..hi].to_vec();
            tasks.extend(charts.iter().map(|c| self.load_icon(c)));
        }
        Task::batch(tasks)
    }

    /// Starts loading the charter/pack icon for `chart` if it has one and it isn't loaded yet.
    fn load_icon(&mut self, chart: &Chart) -> Task<Message> {
        let Some(list) = &self.icon_list else { return Task::none() };
        let Some(entry) = icons::resolve(list, chart) else { return Task::none() };
        if self.icons.contains_key(&entry.sha1) {
            return Task::none();
        }
        let entry = entry.clone();
        let sha1 = entry.sha1.clone();
        self.icons.insert(sha1.clone(), None);
        Task::perform(icons::fetch_icon(entry), move |r| Message::IconLoaded(sha1.clone(), r))
    }

    /// The loaded icon image for a chart, if any.
    fn icon_for(&self, chart: &Chart) -> Option<image::Handle> {
        let list = self.icon_list.as_ref()?;
        let entry = icons::resolve(list, chart)?;
        self.icons.get(&entry.sha1)?.clone()
    }

    /// Overlays the chart's charter/pack icon in the bottom-left corner of `art`.
    fn with_icon(&self, art: Element<'static, Message>, chart: &Chart, size: f32, pad: f32) -> Element<'static, Message> {
        let Some(icon) = self.icon_for(chart) else { return art };
        let badge = iced::widget::tooltip(
            image(icon).width(size).height(size),
            container(text(icons::tooltip(chart)).size(12)).padding([4, 8]).style(crate::style::panel_solid),
            iced::widget::tooltip::Position::Top,
        );
        iced::widget::stack![
            art,
            container(badge).padding(pad).align_left(Length::Fill).align_bottom(Length::Fill),
        ]
        .into()
    }

    /// Makes sure the full-size cover is available (re-fetching it if it was evicted).
    fn ensure_full_art(&mut self, md5: Option<&String>) -> Task<Message> {
        match md5.and_then(|m| self.art.get(m)) {
            Some(Art::Ready { full: None, .. }) => self.fetch_art(md5.unwrap().clone()),
            Some(_) => Task::none(),
            None => self.load_art(md5),
        }
    }

    fn remember_full_art(&mut self, md5: &str) {
        self.full_art_lru.retain(|m| m != md5);
        self.full_art_lru.push_back(md5.to_string());
        while self.full_art_lru.len() > FULL_ART_KEEP {
            if let Some(old) = self.full_art_lru.pop_front()
                && let Some(Art::Ready { full, .. }) = self.art.get_mut(&old)
            {
                *full = None;
            }
        }
    }

    /// Closes the preview (stopping audio) and remembers its volume.
    fn close_preview(&mut self) -> Task<Message> {
        if let Some(p) = self.preview.take() {
            if (p.volume() - self.settings.preview_volume).abs() > f32::EPSILON {
                self.settings.preview_volume = p.volume();
                return self.save_settings();
            }
        }
        Task::none()
    }

    fn save_settings(&self) -> Task<Message> {
        Task::perform(self.settings.clone().save(), Message::Saved)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SortBy(key) => {
                use api::{Sort, SortDir};
                // Cycle: ascending -> descending -> relevance
                self.sort = match self.sort {
                    Some(Sort { key: k, direction: SortDir::Asc }) if k == key => {
                        Some(Sort { key, direction: SortDir::Desc })
                    }
                    Some(Sort { key: k, direction: SortDir::Desc }) if k == key => None,
                    _ => Some(Sort { key, direction: SortDir::Asc }),
                };
                self.search(false)
            }
            Message::ToggleAdvanced => {
                self.advanced_open = !self.advanced_open;
                self.screen = Screen::Browse;
                Task::none()
            }
            Message::Advanced(advanced::Msg::Close) => {
                self.advanced_open = false;
                Task::none()
            }
            Message::Advanced(advanced::Msg::Search) => match self.advanced.to_filters().and_then(|f| Ok((f, self.advanced.fretwork_filter()?))) {
                Ok((filters, fw)) => {
                    self.advanced.error = None;
                    self.fw.filter = fw;
                    // An empty form means a normal search (which includes RhythmVerse).
                    self.active_advanced = (self.advanced.active_count() > 0).then(|| Arc::new(filters));
                    self.query.clear();
                    self.search(false)
                }
                Err(e) => {
                    self.advanced.error = Some(e);
                    Task::none()
                }
            },
            // Reset also ends an Advanced search that's showing, back to the normal results.
            Message::Advanced(advanced::Msg::Reset) if self.active_advanced.is_some() => {
                self.advanced.update(advanced::Msg::Reset);
                self.active_advanced = None;
                self.fw.filter = None;
                self.search(false)
            }
            Message::Advanced(msg) => {
                self.advanced.update(msg);
                self.advanced.error = None;
                Task::none()
            }
            Message::QueryChanged(q) => {
                self.query = q;
                self.screen = Screen::Browse;
                // Typing in the main box switches back to quick search.
                self.active_advanced = None;
                self.generation += 1;
                let g = self.generation;
                Task::perform(tokio::time::sleep(Duration::from_millis(220)), move |_| Message::Debounced(g))
            }
            Message::Debounced(g) if g == self.generation => self.search(false),
            Message::Debounced(_) => Task::none(),
            Message::SubmitSearch => {
                self.active_advanced = None;
                self.search(false)
            }
            Message::SearchFor(q) => {
                self.query = q;
                self.screen = Screen::Browse;
                self.active_advanced = None;
                self.generation += 1;
                self.search(false)
            }
            Message::InstrumentPicked(o) => {
                self.settings.instrument = o.key.map(String::from);
                self.instrument = o;
                Task::batch([self.search(false), self.save_settings()])
            }
            Message::DifficultyPicked(o) => {
                self.settings.difficulty = o.key.map(String::from);
                self.difficulty = o;
                Task::batch([self.search(false), self.save_settings()])
            }
            Message::SearchDone { generation, append, result } => {
                if generation != self.generation {
                    return Task::none();
                }
                self.loading = false;
                match result {
                    Ok(resp) => {
                        self.error = None;
                        self.found = resp.found;
                        let save = if append { Task::none() } else { self.clear_results_if_new() };
                        if self.fw_active() {
                            let rate = self.fw_take_page(resp.data);
                            return Task::batch([self.refresh_art_window(), save, rate]);
                        }
                        #[cfg(feature = "rhythmverse")]
                        if self.merging() {
                            self.rv_encore_page(resp.data);
                            return Task::batch([self.refresh_art_window(), save]);
                        }
                        let before = self.results.len();
                        self.ingest(resp.data);
                        self.encore_seen += (self.results.len() - before) as u64;
                        #[cfg(feature = "rhythmverse")]
                        self.rv_flush();
                        Task::batch([self.refresh_art_window(), save])
                    }
                    Err(e) => {
                        if append {
                            self.page -= 1;
                        }
                        self.error = Some(e);
                        // Stop waiting on Encore so RhythmVerse's results aren't held back.
                        #[cfg(feature = "rhythmverse")]
                        if self.merging() {
                            self.found = self.encore_seen;
                            self.rv_flush();
                        }
                        Task::none()
                    }
                }
            }
            Message::Scrolled(vp) => {
                // Roughly which result is on screen (works for every layout).
                let rel = vp.relative_offset().y;
                self.art_center = if rel.is_finite() { (rel * self.results.len() as f32) as usize } else { 0 };
                let art = self.refresh_art_window();
                if rel > 0.85 && !self.searching() && self.more_available() {
                    if self.fw_active() {
                        // Charts are still being rated, or another run of pages may follow.
                        self.fw_more();
                        if self.fw_pending() || self.encore_seen >= self.found {
                            return art;
                        }
                    }
                    if self.encore_seen < self.found {
                        return Task::batch([art, self.search(true)]);
                    }
                    #[cfg(feature = "rhythmverse")]
                    return Task::batch([art, self.rv_search(true)]);
                }
                art
            }
            Message::Select(i) if self.modifiers.control() || self.modifiers.shift() => {
                // Ctrl-click ticks one result, Shift-click a range; the open chart joins in.
                if self.checked.is_empty()
                    && let Some(s) = self.selected
                    && s != i
                {
                    self.checked.insert(s);
                }
                match self.anchor.or(self.selected) {
                    Some(a) if self.modifiers.shift() => {
                        self.checked.extend(a.min(i)..=a.max(i));
                    }
                    _ => {
                        if !self.checked.remove(&i) {
                            self.checked.insert(i);
                        }
                        self.anchor = Some(i);
                    }
                }
                self.list_rev += 1;
                Task::none()
            }
            Message::Select(i) => {
                self.selected = Some(i);
                self.anchor = Some(i);
                self.show_versions = false;
                self.confirm_delete = None;
                self.delete_error = None;
                // The preview lives in the sidebar, so it belongs to the selected chart only.
                let selected_md5 = self.results.get(i).map(|c| c.md5.clone());
                let close = if self.preview.as_ref().is_some_and(|p| Some(&p.md5) != selected_md5.as_ref()) {
                    self.close_preview()
                } else {
                    Task::none()
                };
                let md5 = self.results.get(i).and_then(|c| c.album_art_md5.clone());
                if let Some(m) = &md5 {
                    self.remember_full_art(m);
                }
                let icon = match self.results.get(i).cloned() {
                    Some(c) => self.load_icon(&c),
                    None => Task::none(),
                };
                Task::batch([close, self.ensure_full_art(md5.as_ref()), icon, self.load_board()])
            }
            Message::IconList(r) => {
                let Ok(list) = r else { return Task::none() };
                self.icon_list = Some(list);
                self.icon_version += 1;
                let mut tasks = vec![self.refresh_art_window()];
                if let Some(c) = self.selected.and_then(|i| self.results.get(i)).cloned() {
                    tasks.push(self.load_icon(&c));
                }
                Task::batch(tasks)
            }
            Message::IconLoaded(sha1, r) => {
                self.icons.insert(sha1, r.ok().map(image::Handle::from_bytes));
                self.icon_version += 1;
                Task::none()
            }
            Message::ArtLoaded(md5, r) => {
                self.art_inflight.remove(&md5);
                match r {
                    Ok((t, f)) => {
                        let art = Art::Ready {
                            thumb: image::Handle::from_rgba(t.width, t.height, t.rgba),
                            full: Some(image::Handle::from_bytes(f)),
                        };
                        self.art.insert(md5.clone(), art);
                        self.remember_full_art(&md5);
                    }
                    // The chart has no cover.
                    Err(e) if e == api::ART_MISSING => {
                        self.art.insert(md5.clone(), Art::Failed);
                    }
                    // A failed request: forget it, so it's tried again when the row is next shown.
                    Err(_) => {
                        self.art.remove(&md5);
                    }
                }
                self.art_version += 1;
                self.pump_art()
            }

            Message::Download(i) => {
                let Some(chart) = self.results.get(i).cloned() else { return Task::none() };
                self.enqueue(chart, Vec::new());
                self.pump_downloads()
            }
            Message::Redownload(i) => {
                let Some(chart) = self.results.get(i).cloned() else { return Task::none() };
                self.confirm_delete = None;
                let paths = self.library_paths(&chart);
                self.enqueue(chart, paths);
                self.pump_downloads()
            }
            Message::DeleteChart(i) => {
                let Some(chart) = self.results.get(i).cloned() else { return Task::none() };
                self.delete_error = None;
                if self.confirm_delete.as_deref() != Some(chart.md5.as_str()) {
                    self.confirm_delete = Some(chart.md5.clone());
                    return Task::none();
                }
                self.confirm_delete = None;
                let paths = self.library_paths(&chart);
                let key = chart.library_key();
                let removed = paths.clone();
                Task::perform(library::recycle(paths), move |r| Message::Deleted(key.clone(), removed.clone(), r))
            }
            Message::Deleted(key, removed, r) => {
                match r {
                    Ok(()) => {
                        if let Some(entry) = self.library.get_mut(&key) {
                            let before = entry.len();
                            entry.retain(|p| !removed.contains(p));
                            self.library_charts -= before - entry.len();
                            if entry.is_empty() {
                                self.library.remove(&key);
                            }
                        }
                        self.catalog_stale = true;
                        self.list_rev += 1;
                    }
                    Err(e) => self.delete_error = Some(e),
                }
                Task::none()
            }
            Message::DownloadChecked => {
                let charts: Vec<_> = self.checked.iter().filter_map(|&i| self.results.get(i).cloned()).collect();
                for c in charts {
                    self.enqueue(c, Vec::new());
                }
                self.checked.clear();
                self.list_rev += 1;
                self.pump_downloads()
            }
            Message::ClearChecked => {
                self.checked.clear();
                self.list_rev += 1;
                Task::none()
            }
            Message::CheckUpdates => {
                self.update = UpdateState::Checking { manual: true };
                Task::perform(updater::check(), Message::UpdateChecked)
            }
            Message::UpdateChecked(r) => {
                let manual = matches!(self.update, UpdateState::Checking { manual: true });
                self.update = match r {
                    Ok(Some(rel)) => UpdateState::Available(rel),
                    Ok(None) => UpdateState::UpToDate,
                    // A failed check on start isn't worth bothering anyone about.
                    Err(e) if manual => UpdateState::Failed(e),
                    Err(_) => UpdateState::Idle,
                };
                Task::none()
            }
            Message::InstallUpdate => {
                let UpdateState::Available(rel) = &self.update else { return Task::none() };
                let rel = rel.clone();
                self.update = UpdateState::Installing(rel.clone(), 0.0);
                Task::run(updater::install(rel), Message::UpdateEvent)
            }
            Message::UpdateEvent(e) => {
                match e {
                    updater::InstallEvent::Progress(p) => {
                        if let UpdateState::Installing(_, f) = &mut self.update {
                            *f = p;
                        }
                    }
                    updater::InstallEvent::Done(r) => {
                        let version = match &self.update {
                            UpdateState::Installing(rel, _) => rel.version.clone(),
                            _ => String::new(),
                        };
                        self.update = match r {
                            Ok(()) => UpdateState::Ready(version),
                            Err(e) => UpdateState::Failed(e),
                        };
                    }
                }
                Task::none()
            }
            // Settings are saved as they change, so there's nothing to flush first.
            Message::RestartForUpdate => updater::restart(),
            Message::ToggleFretwork(b) => {
                self.settings.show_fretwork = b;
                self.save_settings()
            }
            Message::BackdropPicked(b) => {
                self.settings.window_backdrop = b.key().into();
                self.save_settings()
            }
            Message::GlassTransparency(v) => {
                self.settings.glass_transparency = v;
                Task::none()
            }
            Message::SaveSettings => self.save_settings(),
            Message::ToggleRefined(b) => {
                self.settings.refined_ui = b;
                style::set_refined(b);
                self.list_rev += 1;
                self.save_settings()
            }
            Message::ToggleChangelog => {
                self.show_changelog = !self.show_changelog;
                Task::none()
            }
            Message::ToggleCheckUpdates(b) => {
                self.settings.check_updates = b;
                self.save_settings()
            }
            Message::ToggleChecked(i) => {
                if !self.checked.remove(&i) {
                    self.checked.insert(i);
                }
                self.anchor = Some(i);
                self.list_rev += 1;
                Task::none()
            }
            Message::CheckAll(all) => {
                // Charts already in your library are left out.
                self.checked = if all {
                    self.results.iter().enumerate().filter(|(_, c)| !self.in_library(c)).map(|(i, _)| i).collect()
                } else {
                    BTreeSet::new()
                };
                self.list_rev += 1;
                Task::none()
            }
            Message::RetryDownload(id) => {
                if let Some(d) = self.downloads.iter_mut().find(|d| d.id == id) {
                    d.status = DlStatus::Queued;
                }
                self.pump_downloads()
            }
            Message::DlProgress(id, p) => {
                if let Some(d) = self.downloads.iter_mut().find(|d| d.id == id) {
                    match p {
                        download::Progress::Status(s) => {
                            if let DlStatus::Active { status, .. } = &mut d.status {
                                *status = s;
                            }
                        }
                        download::Progress::Bytes { done: n, total: t } => {
                            if let DlStatus::Active { status, done, total } = &mut d.status {
                                *done = n;
                                *total = t;
                                *status = "Downloading".into();
                            }
                        }
                        download::Progress::Finished(r) => {
                            d.handle = None;
                            d.status = match r {
                                Ok(p) => DlStatus::Done(p),
                                Err(e) => DlStatus::Failed(e),
                            };
                            if let DlStatus::Done(dest) = &d.status {
                                self.catalog_stale = true;
                                let entry = self.library.entry(d.chart.library_key()).or_default();
                                let before = entry.len();
                                entry.retain(|p| !d.replace.contains(p));
                                entry.push(dest.clone());
                                self.library_charts = self.library_charts + entry.len() - before;
                                self.list_rev += 1;
                            }
                            return self.pump_downloads();
                        }
                    }
                }
                Task::none()
            }
            Message::CancelDownload(id) => {
                if let Some(pos) = self.downloads.iter().position(|d| d.id == id) {
                    if let Some(h) = self.downloads[pos].handle.take() {
                        h.abort();
                    }
                    self.downloads.remove(pos);
                }
                self.pump_downloads()
            }
            Message::ClearFinished => {
                self.downloads.retain(|d| matches!(d.status, DlStatus::Active { .. } | DlStatus::Queued));
                Task::none()
            }
            Message::OpenPreview(i) => {
                let Some(chart) = self.results.get(i).cloned().filter(|c| self.can_preview(c)) else { return Task::none() };
                if self.preview.as_ref().is_some_and(|p| p.md5 == chart.md5) {
                    return Task::none();
                }
                let save = self.close_preview();
                let (panel, task) = preview_panel::Panel::open(
                    chart,
                    self.instrument.key.map(String::from),
                    self.difficulty.key.map(String::from),
                    self.settings.preview_volume,
                    self.settings.download_videos,
                );
                self.preview = Some(panel);
                self.screen = Screen::Browse;
                Task::batch([save, task.map(Message::Preview)])
            }
            Message::Preview(msg) => {
                let Some(panel) = self.preview.as_mut() else { return Task::none() };
                let (task, close) = panel.update(msg);
                if close {
                    return self.close_preview();
                }
                task.map(Message::Preview)
            }
            Message::ToggleDownloads => {
                self.show_downloads = !self.show_downloads;
                Task::none()
            }
            Message::OpenPath(p) => {
                let _ = open::that_detached(p);
                Task::none()
            }

            Message::DrumTypePicked(o) => {
                self.settings.drum_type = o.key.map(String::from);
                self.drum_type = o;
                Task::batch([self.search(false), self.save_settings()])
            }
            Message::DrumsReviewedPicked(o) => {
                self.settings.drums_reviewed = o.key.is_some();
                self.drums_reviewed = o;
                Task::batch([self.search(false), self.save_settings()])
            }
            Message::WindowResized(size) => {
                self.win = size;
                Task::none()
            }
            Message::ModifiersChanged(m) => {
                self.modifiers = m;
                Task::none()
            }
            Message::PickVersion(i, v) => {
                let Some(cur) = self.results.get(i) else { return Task::none() };
                let Some(chart) = self.versions_of(cur).get(v).cloned() else { return Task::none() };
                let close = if self.preview.is_some() { self.close_preview() } else { Task::none() };
                let art = chart.album_art_md5.clone();
                self.results[i] = chart.clone();
                self.list_rev += 1;
                if let Some(m) = &art {
                    self.remember_full_art(m);
                }
                Task::batch([close, self.ensure_full_art(art.as_ref()), self.load_icon(&chart), self.load_board()])
            }
            Message::ToggleLeaderboard => {
                self.settings.show_leaderboard = !self.settings.show_leaderboard;
                Task::batch([self.load_board(), self.save_settings()])
            }
            Message::PickLeaderboard(inst, diff) => {
                self.board_pick = Some((inst, diff));
                self.load_board()
            }
            Message::LeaderboardLoaded(track, result) => {
                self.boards.insert(track, result);
                Task::none()
            }
            Message::FindScores(md5) => self.find_scores(md5),
            Message::FwRated(generation, md5, result) => self.fw_rated(generation, md5, result),
            Message::HashesFound(md5, result) => {
                self.hashes.insert(md5, leaderboard::Hashes::Done(result));
                self.load_board()
            }
            Message::ToggleIssues => {
                self.show_issues = !self.show_issues;
                Task::none()
            }
            Message::ToggleVersions => {
                self.show_versions = !self.show_versions;
                Task::none()
            }
            Message::CopyText(s) => iced::clipboard::write(s),
            Message::OpenUrl(u) => {
                let _ = open::that_detached(u);
                Task::none()
            }
            #[cfg(feature = "rhythmverse")]
            Message::RvDone { generation, append, result } => {
                let clear = if !append && !self.loading && generation == self.generation && result.is_ok() {
                    self.clear_results_if_new()
                } else {
                    Task::none()
                };
                Task::batch([clear, self.rv_done(generation, append, result)])
            }
            #[cfg(feature = "rhythmverse")]
            Message::RvSync(e) => self.rv_sync_event(e),
            #[cfg(feature = "rhythmverse")]
            Message::RvSyncNow => self.rv_start_sync(),
            #[cfg(feature = "rhythmverse")]
            Message::RvRebuild => {
                if self.rv.syncing.is_some() {
                    return Task::none();
                }
                let _ = rv_catalog::reset();
                self.rv.catalog = rv_catalog::status();
                self.rv_start_sync()
            }
            #[cfg(feature = "rhythmverse")]
            Message::ToggleEncore(b) => {
                self.settings.source_encore = b;
                self.error = None;
                Task::batch([self.save_settings(), self.search(false)])
            }
            #[cfg(feature = "rhythmverse")]
            Message::ToggleRhythmVerse(b) => {
                self.settings.source_rhythmverse = b;
                self.error = None;
                let sync = if b { self.rv_auto_sync() } else { Task::none() };
                Task::batch([self.save_settings(), self.search(false), sync])
            }
            Message::LibraryScanned(lib) => {
                self.library_scanning = false;
                self.catalog_stale = false;
                let lib = Arc::unwrap_or_clone(lib);
                self.library_charts = lib.charts;
                self.library = lib.keys;
                self.catalog = lib.catalog;
                self.list_rev += 1;
                self.refresh_pages();
                if matches!(self.screen, Screen::Library | Screen::Tools) || !self.lyrics_map.is_empty() {
                    return self.scan_lyrics();
                }
                Task::none()
            }
            Message::LyricsScanned(map) => {
                self.lyrics_scanning = false;
                self.lyrics_map = map;
                self.refresh_pages();
                Task::none()
            }
            Message::LibraryPage(m) => {
                let mut page = std::mem::take(&mut self.library_page);
                let out = page.update(m, &self.library_ctx());
                let scroll: Task<Message> = page.scroll_task();
                self.library_page = page;
                let task = match out {
                    library_ui::Out::None => Task::none(),
                    library_ui::Out::Rescan => {
                        self.lyrics_map.clear();
                        self.scan_library()
                    }
                    library_ui::Out::Open(p) => {
                        let _ = open::that_detached(p);
                        Task::none()
                    }
                    library_ui::Out::Delete(paths) => {
                        // Only charts inside the library folders can be removed (to the Recycle Bin).
                        let roots = self.settings.all_library_folders();
                        let ok: Vec<PathBuf> = paths.into_iter().filter(|p| library::is_deletable_chart(p, &roots)).collect();
                        Task::perform(library::recycle(ok), Message::LibraryDeleted)
                    }
                };
                Task::batch([task, scroll])
            }
            Message::LibraryDeleted(r) => {
                self.library_error = r.err();
                self.scan_library()
            }
            Message::ToolsPage(m) => {
                let mut page = std::mem::take(&mut self.tools_page);
                let (task, out) = page.update(m, &self.tools_ctx());
                self.tools_page = page;
                let task = task.map(Message::ToolsPage);
                let extra = match out {
                    tools_ui::Out::None => Task::none(),
                    tools_ui::Out::Open(p) => {
                        let _ = open::that_detached(p);
                        Task::none()
                    }
                    tools_ui::Out::SetIssuePaths(a, b) => {
                        self.settings.issue_scan_path = a;
                        self.settings.spreadsheet_output_path = b;
                        self.save_settings()
                    }
                };
                Task::batch([task, extra])
            }
            Message::RescanLibrary => self.scan_library(),
            Message::AddLibraryFolder => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Add a songs folder")
                        .pick_folder()
                        .await
                        .map(|h| h.path().to_path_buf())
                },
                Message::LibraryFolderAdded,
            ),
            Message::LibraryFolderAdded(Some(p)) => {
                if !self.settings.all_library_folders().contains(&p) {
                    if self.settings.library_path.is_none() {
                        self.settings.library_path = Some(p);
                    } else {
                        self.settings.extra_library_folders.push(p);
                    }
                }
                Task::batch([self.save_settings(), self.scan_library()])
            }
            Message::LibraryFolderAdded(None) => Task::none(),
            Message::RemoveLibraryFolder(idx) => {
                if idx == 0 {
                    self.settings.library_path = if self.settings.extra_library_folders.is_empty() {
                        None
                    } else {
                        Some(self.settings.extra_library_folders.remove(0))
                    };
                } else if idx - 1 < self.settings.extra_library_folders.len() {
                    self.settings.extra_library_folders.remove(idx - 1);
                }
                Task::batch([self.save_settings(), self.scan_library()])
            }
            Message::MakeDefaultLibrary(idx) => {
                if idx > 0 && idx - 1 < self.settings.extra_library_folders.len() {
                    let new_default = self.settings.extra_library_folders.remove(idx - 1);
                    if let Some(old) = self.settings.library_path.replace(new_default) {
                        self.settings.extra_library_folders.insert(0, old);
                    }
                }
                self.save_settings()
            }
            Message::ShowScreen(s) => {
                self.screen = s;
                let mut tasks = Vec::new();
                if s == Screen::Library {
                    if self.catalog_stale && !self.library_scanning {
                        tasks.push(self.scan_library());
                    } else if self.lyrics_map.is_empty() {
                        tasks.push(self.scan_lyrics());
                    }
                }
                Task::batch(tasks)
            }
            Message::FolderTemplateChanged(s) => {
                self.settings.chart_folder_name = s;
                self.save_settings()
            }
            Message::ToggleSng(b) => {
                self.settings.is_sng = b;
                self.save_settings()
            }
            Message::ToggleVideos(b) => {
                self.settings.download_videos = b;
                self.save_settings()
            }
            Message::FpsPicked(f) => {
                self.settings.preview_fps = f.0;
                self.save_settings()
            }
            Message::LayoutPicked(l) => {
                let old_px = self.thumb_px();
                self.ui = l;
                self.settings.layout = l.key().to_string();
                let mut tasks = vec![self.save_settings()];
                if self.thumb_px() != old_px {
                    // Thumbnails are decoded at a layout-specific size; reload them.
                    self.art.clear();
                    self.full_art_lru.clear();
                    let md5s: Vec<String> = self.results.iter().filter_map(|c| c.album_art_md5.clone()).collect();
                    tasks.extend(md5s.iter().map(|m| self.load_art(Some(m))));
                }
                Task::batch(tasks)
            }
            Message::ThemePicked(t) => {
                self.settings.theme = t.0.to_string();
                let icon = match app_icon(&t.0) {
                    Some(icon) => iced::window::oldest().and_then(move |id| iced::window::set_icon(id, icon.clone())),
                    None => Task::none(),
                };
                let pin = pin_icon_task(&t.0);
                self.theme = t.0;
                Task::batch([self.save_settings(), icon, pin, self.backdrop_task()])
            }
            Message::PinIconDone => Task::none(),
            Message::ToggleCpuRendering(b) => {
                self.settings.cpu_rendering = b;
                self.save_settings()
            }
            Message::Saved(r) => {
                if let Err(e) = r {
                    self.error = Some(format!("Failed to save settings: {e}"));
                }
                Task::none()
            }
        }
    }

    // ---------- View ----------

    fn view(&self) -> Element<'_, Message> {
        let body = match self.screen {
            Screen::Browse => self.view_browse(),
            Screen::Settings => self.view_settings(),
            Screen::Library => self.library_page.view(&self.library_ctx()).map(Message::LibraryPage),
            Screen::Tools => self.tools_page.view(&self.tools_ctx()).map(Message::ToolsPage),
        };
        let mut layout = column![self.view_topbar()];
        if self.advanced_open && self.screen == Screen::Browse {
            layout = layout.push(self.advanced.view().map(Message::Advanced));
        }
        layout.push(body).push(self.view_downloads()).into()
    }

    fn view_topbar(&self) -> Element<'_, Message> {
        if style::refined() {
            return self.view_topbar_refined();
        }
        let placeholder = if self.active_advanced.is_some() {
            "Showing advanced search results — type here to go back to quick search"
        } else {
            "Search songs, artists, albums, charters…"
        };
        let search = text_input(placeholder, &self.query)
            .id(SEARCH_ID)
            .on_input(Message::QueryChanged)
            .on_submit(Message::SubmitSearch)
            .padding([9, 14])
            .size(15)
            .style(|t: &Theme, s| {
                let mut st = text_input::default(t, s);
                st.border.radius = 8.0.into();
                st
            });

        let nav = |label: &'static str, screen: Screen| {
            let active = self.screen == screen;
            button(text(label).size(14))
                .padding([8, 14])
                .style(move |t: &Theme, s| if active { button::primary(t, s) } else { button::subtle(t, s) })
                .on_press(Message::ShowScreen(screen))
        };

        let filters = row![
                {
                    let n = self.advanced.active_count();
                    let label = if n > 0 { format!("Advanced ({n})") } else { "Advanced".to_string() };
                    let highlighted = self.advanced_open || self.active_advanced.is_some();
                    button(text(label).size(14))
                        .padding([8, 14])
                        .style(move |t: &Theme, s| {
                            if highlighted { button::primary(t, s) } else { button::secondary(t, s) }
                        })
                        .on_press(Message::ToggleAdvanced)
                },
                pick_list(self.instrument_opts.as_slice(), Some(&self.instrument), Message::InstrumentPicked)
                    .padding([8, 12])
                    .text_size(14),
                pick_list(self.difficulty_opts.as_slice(), Some(&self.difficulty), Message::DifficultyPicked)
                    .padding([8, 12])
                    .text_size(14),
            ]
            .extend((self.instrument.key == Some("drums")).then(|| -> Element<'_, Message> {
                pick_list(self.drum_type_opts.as_slice(), Some(&self.drum_type), Message::DrumTypePicked)
                    .padding([8, 12])
                    .text_size(14)
                    .into()
            }))
            .extend((self.instrument.key == Some("drums")).then(|| -> Element<'_, Message> {
                pick_list(self.reviewed_opts.as_slice(), Some(&self.drums_reviewed), Message::DrumsReviewedPicked)
                    .padding([8, 12])
                    .text_size(14)
                    .into()
            }))
            .spacing(10)
            .align_y(Alignment::Center);
        let main = row![
            text("Truss").font(BOLD).size(20),
            nav("Browse", Screen::Browse),
            nav("Library", Screen::Library),
            nav("Tools", Screen::Tools),
            nav("Settings", Screen::Settings),
            search,
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let bar: Element<'_, Message> = if self.narrow_topbar() {
            column![main, filters].spacing(8).into()
        } else {
            main.push(filters).into()
        };
        container(bar)
        .padding([10, 14])
        .style(|t: &Theme| container::Style {
            background: Some(crate::style::weak(t).into()),
            ..Default::default()
        })
        .into()
    }

    fn view_topbar_refined(&self) -> Element<'_, Message> {
        let placeholder = if self.active_advanced.is_some() {
            "Showing advanced search results — type here to go back to quick search"
        } else {
            "Search songs, artists, albums, charters"
        };
        let search = text_input(placeholder, &self.query)
            .id(SEARCH_ID)
            .on_input(Message::QueryChanged)
            .on_submit(Message::SubmitSearch)
            .padding([7, 12])
            .size(14)
            .style(style::search_input);

        // Text tabs; the current page gets a thin accent line under it.
        let nav = |label: &'static str, screen: Screen| -> Element<'_, Message> {
            let active = self.screen == screen;
            column![
                button(text(label).size(14)).padding([6, 2]).style(style::nav_button(active)).on_press(Message::ShowScreen(screen)),
                container(space()).height(2).width(Length::Fill).style(move |t: &Theme| container::Style {
                    background: active.then(|| style::accent(t).into()),
                    ..Default::default()
                }),
            ]
            .width(Length::Shrink)
            .into()
        };
        fn pick<'a>(p: iced::widget::PickList<'a, Opt, &'a [Opt], &'a Opt, Message>) -> iced::widget::PickList<'a, Opt, &'a [Opt], &'a Opt, Message> {
            p.padding([6, 10]).text_size(13).style(style::quiet_pick_list)
        }
        let advanced = {
            let n = self.advanced.active_count();
            let label = if n > 0 { format!("Advanced ({n})") } else { "Advanced".to_string() };
            let highlighted = self.advanced_open || self.active_advanced.is_some();
            button(text(label).size(13)).padding([6, 12]).style(style::quiet_button(highlighted)).on_press(Message::ToggleAdvanced)
        };
        let bar = row![
            text("Truss").font(Font { weight: iced::font::Weight::Semibold, ..Font::DEFAULT }).size(17),
            space().width(10),
            nav("Browse", Screen::Browse),
            nav("Library", Screen::Library),
            nav("Tools", Screen::Tools),
            nav("Settings", Screen::Settings),
            space().width(14),
            search,
        ]
        .spacing(14)
        .align_y(Alignment::Center);
        let mut filters = row![
            advanced,
            pick(pick_list(self.instrument_opts.as_slice(), Some(&self.instrument), Message::InstrumentPicked)),
            pick(pick_list(self.difficulty_opts.as_slice(), Some(&self.difficulty), Message::DifficultyPicked)),
        ]
        .spacing(14)
        .align_y(Alignment::Center);
        if self.instrument.key == Some("drums") {
            filters = filters
                .push(pick(pick_list(self.drum_type_opts.as_slice(), Some(&self.drum_type), Message::DrumTypePicked)))
                .push(pick(pick_list(self.reviewed_opts.as_slice(), Some(&self.drums_reviewed), Message::DrumsReviewedPicked)));
        }
        let bar: Element<'_, Message> = if self.narrow_topbar() {
            column![bar, filters].spacing(8).into()
        } else {
            bar.push(filters).into()
        };
        column![container(bar).padding([8, 18]).width(Length::Fill), style::divider(false)].into()
    }

    /// Width of the details sidebar: narrower in small windows.
    fn sidebar_width(&self) -> f32 {
        let w = self.win.width;
        if w >= 1200.0 { 360.0 } else if w >= 950.0 { 320.0 } else { 280.0 }
    }

    /// Which results columns fit: 0 all, 1 without Album, 2 without Album and Charter.
    pub fn list_cols(&self) -> u8 {
        let list = self.win.width - self.sidebar_width();
        if list >= 640.0 { 0 } else if list >= 500.0 { 1 } else { 2 }
    }

    /// Put the search filters on their own row so the search box keeps its room.
    fn narrow_topbar(&self) -> bool {
        self.win.width < if self.instrument.key == Some("drums") { 1400.0 } else { 1150.0 }
    }

    fn view_browse(&self) -> Element<'_, Message> {
        match self.ui {
            variants::UiVariant::Grid => return self.view_browse_grid(),
            variants::UiVariant::Compact => return self.view_browse_compact(),
            variants::UiVariant::Hero => return self.view_browse_hero(),
            variants::UiVariant::Default => {}
        }
        let expanded = self.preview.as_ref().is_some_and(|p| p.expanded());
        if style::refined() {
            let sidebar = container(self.view_sidebar()).width(if expanded { 720.0_f32.min(self.win.width * 0.6) } else { self.sidebar_width() }).height(Length::Fill).padding([18, 20]);
            return row![self.lazy_results(), style::divider(true), sidebar].height(Length::Fill).into();
        }
        let sidebar = container(self.view_sidebar())
            .width(if expanded { 720.0_f32.min(self.win.width * 0.6) } else { self.sidebar_width() })
            .height(Length::Fill)
            .padding(16)
            .style(|t: &Theme| container::Style {
                background: Some(crate::style::weakest(t).into()),
                ..Default::default()
            });
        row![self.lazy_results(), sidebar].height(Length::Fill).into()
    }

    /// Default layout's results list (header + rows), built with owned data for `lazy`.
    fn list_results(&self) -> Element<'static, Message> {
        use api::{SortDir, SortKey};
        if style::refined() {
            return self.list_results_refined();
        }
        let col = |label: &'static str, key: SortKey, width: Length| {
            let arrow = match self.sort {
                Some(s) if s.key == key && s.direction == SortDir::Asc => " ▲",
                Some(s) if s.key == key => " ▼",
                _ => "",
            };
            button(text(format!("{label}{arrow}")).font(BOLD).size(13))
                .padding(0)
                .width(width)
                .style(|t: &Theme, s| {
                    let p = t.extended_palette();
                    button::Style {
                        text_color: if matches!(s, button::Status::Hovered) {
                            p.primary.strong.color
                        } else {
                            p.background.base.text
                        },
                        ..Default::default()
                    }
                })
                .on_press(Message::SortBy(key))
        };
        let cols = self.list_cols();
        let header = row![
            self.check_all_box(),
            space().width(44),
            col("Name", SortKey::Name, Length::FillPortion(4)),
            col("Artist", SortKey::Artist, Length::FillPortion(3)),
        ]
        .push((cols < 1).then(|| col("Album", SortKey::Album, Length::FillPortion(3))))
        .push((cols < 2).then(|| col("Charter", SortKey::Charter, Length::FillPortion(2))))
        .push(col("Length", SortKey::Length, Length::Fixed(72.0)))
        .spacing(12)
        .padding([6, 12]);

        let rows = self.results.iter().enumerate().map(|(i, c)| self.view_row(i, c));
        let mut list = column(rows).spacing(2).padding([0, 6]);

        let status: Option<String> = if let Some(e) = &self.error {
            Some(if e.starts_with("No chart sources") { e.clone() } else { format!("Error: {e}") })
        } else if self.searching() {
            Some("Searching…".into())
        } else if self.results.is_empty() {
            Some("No charts found".into())
        } else if !self.more_available() {
            let n = self.results.len();
            Some(format!("{n} chart{}", if n == 1 { "" } else { "s" }))
        } else {
            None
        };
        if let Some(s) = status {
            list = list.push(container(text(s).size(13)).padding(14).center_x(Length::Fill));
        }

        column![
            header,
            rule::horizontal(1).style(crate::style::rule),
            scrollable(list).on_scroll(Message::Scrolled).height(Length::Fill).width(Length::Fill),
        ]
        .width(Length::Fill)
        .into()
    }

    fn list_results_refined(&self) -> Element<'static, Message> {
        use api::{SortDir, SortKey};
        let col = |label: &'static str, key: SortKey, width: Length, right: bool| -> Element<'static, Message> {
            let sorted = self.sort.filter(|s| s.key == key);
            let arrow = match sorted {
                Some(s) if s.direction == SortDir::Asc => " ↑",
                Some(_) => " ↓",
                None => "",
            };
            let t = text(format!("{label}{arrow}")).size(12);
            let t = if right { t.width(Length::Fill).align_x(iced::alignment::Horizontal::Right) } else { t };
            button(t)
                .padding(0)
                .width(width)
                .style(move |th: &Theme, s| button::Style {
                    text_color: if sorted.is_some() || matches!(s, button::Status::Hovered) { style::fg(th) } else { style::muted(th) },
                    ..Default::default()
                })
                .on_press(Message::SortBy(key))
                .into()
        };
        let header = row![
            space().width(3),
            container(self.check_all_box()).width(20).center_x(20),
            space().width(40),
            col("Title", SortKey::Name, Length::FillPortion(4), false),
            col("Artist", SortKey::Artist, Length::FillPortion(3), false),
        ]
        .push((self.list_cols() < 1).then(|| col("Album", SortKey::Album, Length::FillPortion(3), false)))
        .push((self.list_cols() < 2).then(|| col("Charter", SortKey::Charter, Length::FillPortion(2), false)))
        .push(col("Length", SortKey::Length, Length::Fixed(56.0), true))
        .spacing(12)
        // Right padding: the rows' own 16 px plus the scrollbar beside them (10 + 4 gap).
        .padding(iced::Padding { top: 8.0, bottom: 8.0, left: 16.0, right: 30.0 })
        .align_y(Alignment::Center);

        let rows = self.results.iter().enumerate().map(|(i, c)| self.view_row(i, c));
        let mut list = column(rows).spacing(1).padding([4, 6]);
        let status: Option<String> = if let Some(e) = &self.error {
            Some(if e.starts_with("No chart sources") { e.clone() } else { format!("Error: {e}") })
        } else if self.searching() {
            Some("Searching…".into())
        } else if self.results.is_empty() {
            Some("No charts found".into())
        } else if !self.more_available() {
            let n = self.results.len();
            Some(format!("{n} chart{}", if n == 1 { "" } else { "s" }))
        } else {
            None
        };
        if let Some(s) = status {
            list = list.push(
                container(text(s).size(12).style(|t: &Theme| text::Style { color: Some(style::muted(t)) }))
                    .padding(16)
                    .center_x(Length::Fill),
            );
        }
        column![
            header,
            style::divider(false),
            scrollable(list).on_scroll(Message::Scrolled).spacing(4).height(Length::Fill).width(Length::Fill),
        ]
        .width(Length::Fill)
        .into()
    }

    fn view_row_refined(&self, i: usize, c: &Chart) -> Element<'static, Message> {
        let selected = self.selected == Some(i);
        let checked = self.checked.contains(&i);
        let owned = self.in_library(c);
        let cell = |s: String, w: Length, quiet: bool| {
            let t = text(s).size(if quiet { 13 } else { 14 }).wrapping(text::Wrapping::None);
            let t = if quiet { t.style(|th: &Theme| text::Style { color: Some(style::muted(th)) }) } else { t };
            container(t).width(w).clip(true)
        };
        let s = |o: &Option<String>| o.as_deref().map(strip_tags).unwrap_or_default();
        let thumb: Element<_> = match c.album_art_md5.as_ref().and_then(|m| self.art.get(m)) {
            Some(Art::Ready { thumb, .. }) => image(thumb.clone()).width(40).height(40).into(),
            _ => container(space())
                .width(40)
                .height(40)
                .style(|t: &Theme| container::Style { background: Some(style::raised(t).into()), ..Default::default() })
                .into(),
        };
        let name: Element<_> = row![]
            .push(owned.then(library_mark))
            .push(c.is_rv().then(rv_tag))
            .push(cell(s(&c.name), Length::Fill, false))
            .spacing(6)
            .align_y(Alignment::Center)
            .width(Length::FillPortion(4))
            .into();
        // A slim accent bar marks the selected row.
        let bar = container(space()).width(3).height(28).style(move |t: &Theme| container::Style {
            background: selected.then(|| style::accent(t).into()),
            border: Border::default().rounded(2),
            ..Default::default()
        });
        button(
            row![
                bar,
                container(self.check_box(i)).width(20).center_x(20),
                thumb,
                name,
                cell(s(&c.artist), Length::FillPortion(3), false),
            ]
            .push((self.list_cols() < 1).then(|| cell(s(&c.album), Length::FillPortion(3), true)))
            .push((self.list_cols() < 2).then(|| cell(s(&c.charter), Length::FillPortion(2), true)))
            .push(
                container(text(c.length_str()).size(13).style(|th: &Theme| text::Style { color: Some(style::muted(th)) }))
                    .width(56)
                    .align_x(iced::alignment::Horizontal::Right),
            )
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding([5, 10])
        .on_press(Message::Select(i))
        .style(style::row_button(selected, checked))
        .into()
    }

    /// The tick box that marks a result for "Download N".
    pub fn check_box(&self, i: usize) -> Element<'static, Message> {
        checkbox(self.checked.contains(&i)).size(if style::refined() { 14 } else { 16 }).on_toggle(move |_| Message::ToggleChecked(i)).into()
    }

    /// The header tick box: ticks every loaded result, or clears them all.
    pub fn check_all_box(&self) -> Element<'static, Message> {
        // "All" means every chart you don't already have (those can't be downloaded again from here).
        let wanted: Vec<usize> = self.results.iter().enumerate().filter(|(_, c)| !self.in_library(c)).map(|(i, _)| i).collect();
        let all = !wanted.is_empty() && wanted.iter().all(|i| self.checked.contains(i));
        checkbox(all).size(if style::refined() { 14 } else { 16 }).on_toggle(Message::CheckAll).into()
    }

    fn view_row(&self, i: usize, c: &Chart) -> Element<'static, Message> {
        if style::refined() {
            return self.view_row_refined(i, c);
        }
        let selected = self.selected == Some(i);
        let checked = self.checked.contains(&i);
        let owned = self.in_library(c);
        let cell = |s: &Option<String>, portion: u16| {
            container(text(s.as_deref().map(strip_tags).unwrap_or_default()).size(14).wrapping(text::Wrapping::None))
                .width(Length::FillPortion(portion))
                .clip(true)
        };

        let thumb: Element<_> = match c.album_art_md5.as_ref().and_then(|m| self.art.get(m)) {
            Some(Art::Ready { thumb, .. }) => image(thumb.clone()).width(44).height(44).into(),
            _ => container(space())
                .width(44)
                .height(44)
                .style(|t: &Theme| container::Style {
                    background: Some(crate::style::strong(t).into()),
                    border: Border::default().rounded(4),
                    ..Default::default()
                })
                .into(),
        };

        let name: Element<_> = match (owned, c.is_rv()) {
            (false, false) => cell(&c.name, 4).into(),
            (o, rv) => row![]
                .push(o.then(library_mark))
                .push(rv.then(rv_tag))
                .push(cell(&c.name, 1))
                .spacing(6)
                .align_y(Alignment::Center)
                .width(Length::FillPortion(4))
                .into(),
        };
        button(
            row![
                self.check_box(i),
                thumb,
                name,
                cell(&c.artist, 3),
            ]
            .push((self.list_cols() < 1).then(|| cell(&c.album, 3)))
            .push((self.list_cols() < 2).then(|| cell(&c.charter, 2)))
            .push(text(c.length_str()).size(14).width(72))
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding([4, 6])
        .on_press(Message::Select(i))
        .style(move |t: &Theme, s| {
            let p = t.extended_palette();
            let bg = match (selected, s) {
                (true, _) => Some(crate::style::selected(t)),
                (false, _) if checked => Some(crate::style::selected(t).scale_alpha(0.45)),
                (false, button::Status::Hovered) => Some(crate::style::weak(t)),
                _ => None,
            };
            button::Style {
                background: bg.map(Background::Color),
                text_color: if selected { crate::style::selected_text(t) } else { p.background.base.text },
                border: Border::default().rounded(6),
                ..Default::default()
            }
        })
        .into()
    }

    fn view_sidebar(&self) -> Element<'_, Message> {
        let Some((i, c)) = self.selected.and_then(|i| self.results.get(i).map(|c| (i, c))) else {
            return center(text("Select a chart to see details").size(14)).into();
        };

        // Album art fills the sidebar width at most, and shrinks in short windows.
        let inner = self.sidebar_width() - if style::refined() { 40.0 } else { 32.0 } - 16.0;
        let art_size = inner.min((self.win.height * 0.38).max(120.0));
        let icon_size = (art_size * 0.2).clamp(36.0, 64.0);
        let sized_art = |h: iced::widget::image::Handle| -> Element<'static, Message> {
            let art = self.with_icon(image(h).width(art_size).height(art_size).into(), c, icon_size, icon_size * 0.19);
            container(art).center_x(Length::Fill).into()
        };
        let art: Element<_> = match (&self.preview, c.album_art_md5.as_ref().and_then(|m| self.art.get(m))) {
            // The preview replaces the album art while it's open.
            (Some(p), _) if p.md5 == c.md5 => {
                p.view(if p.expanded() { 500.0 } else { 328.0 }).map(Message::Preview)
            }
            (_, Some(Art::Ready { full: Some(full), .. })) => sized_art(full.clone()),
            (_, Some(Art::Ready { thumb, .. })) => sized_art(thumb.clone()),
            (_, Some(Art::Loading)) => container(center(text("Loading art…").size(13))).height(art_size).into(),
            _ => container(center(text("No album art").size(13)))
                .height(art_size)
                .width(Length::Fill)
                .style(crate::style::panel)
                .into(),
        };

        let s = |o: &Option<String>| o.as_deref().map(strip_tags).unwrap_or_default();
        let meta_line = |label: &'static str, v: String, link: bool| -> Element<'_, Message> {
            if v.is_empty() {
                space().into()
            } else {
                row![
                    text(label).size(13).width(58).style(|t: &Theme| text::Style {
                        color: Some(if style::refined() { style::muted(t) } else { Color { a: 0.6, ..Color::WHITE } }),
                    }),
                    if link { search_link(v, 13) } else { text(v).size(13).into() },
                ]
                    .spacing(8)
                    .into()
            }
        };

        let mut diff_items: Vec<Element<'_, Message>> = Vec::new();
        let instruments: &[(&str, &str)] = &[
            ("guitar", "Guitar"),
            ("guitarcoop", "Co-op"),
            ("rhythm", "Rhythm"),
            ("bass", "Bass"),
            ("drums", "Drums"),
            ("keys", "Keys"),
            ("vocals", "Vocals"),
            ("guitarghl", "GHL Guitar"),
            ("guitarcoopghl", "GHL Co-op"),
            ("rhythmghl", "GHL Rhythm"),
            ("bassghl", "GHL Bass"),
        ];
        let shown = instruments.iter().filter(|(k, _)| c.diff_for(k).is_some()).count();
        // Two per row only when half the panel still fits a label, five-pixel pips and the number.
        // The Fretwork tier, when shown inline, takes a little more room after the number.
        let fw_w = if self.settings.show_fretwork { 30.0 } else { 0.0 };
        let two_cols = shown > 1 && (inner - 10.0) / 2.0 >= 62.0 + 12.0 + fw_w + 14.0 + 10.0 + 6.0 * 5.0;
        // Pip width that fits half the panel (label 62, number 12, gaps).
        let pip_w = if two_cols { (((inner - 10.0) / 2.0 - 62.0 - 12.0 - fw_w - 14.0 - 10.0) / 6.0).clamp(5.0, 12.0) } else { 0.0 };
        for (key, label) in instruments {
            if let Some(d) = c.diff_for(key) {
                let refined = style::refined();
                let pips = (0..6).fold(row![].spacing(if refined { 2 } else { 3 }), |r, n| {
                    let on = n < d.min(6);
                    r.push(container(space()).width(if two_cols { pip_w } else if refined { 12.0 } else { 14.0 }).height(if refined { 5 } else { 7 }).style(
                        move |t: &Theme| container::Style {
                            background: Some(
                                if on {
                                    t.extended_palette().primary.base.color
                                } else if refined {
                                    style::hairline(t)
                                } else {
                                    crate::style::strong(t)
                                }
                                .into(),
                            ),
                            border: Border::default().rounded(if refined { 1 } else { 2 }),
                            ..Default::default()
                        },
                    ))
                });
                let (rating, tip) = self.fretwork_tip(c, key);
                let mut line = row![text(*label).size(12).width(if two_cols { 62 } else { 84 }).wrapping(text::Wrapping::None), pips, text(d.to_string()).size(12)]
                    .spacing(7)
                    .align_y(Alignment::Center);
                if self.settings.show_fretwork {
                    line = line.push(
                        text(rating.map(|r| format!("FW {}", r.tier)).unwrap_or_default())
                            .size(11)
                            .wrapping(text::Wrapping::None)
                            .style(|t: &Theme| text::Style { color: Some(style::muted(t)) }),
                    );
                }
                diff_items.push(
                    iced::widget::tooltip(
                        line,
                        container(text(tip).size(12)).padding([4, 8]).style(crate::style::panel_solid),
                        iced::widget::tooltip::Position::Top,
                    )
                    .into(),
                );
            }
        }

        let mut diffs = column![].spacing(5);
        let mut items = diff_items.into_iter();
        while let Some(a) = items.next() {
            diffs = diffs.push(if two_cols {
                let b: Element<'_, Message> = items.next().unwrap_or_else(|| space().into());
                Element::from(row![container(a).width(Length::Fill), container(b).width(Length::Fill)].spacing(10))
            } else {
                a
            });
        }

        let mut tags = Vec::new();
        if let Some(nd) = &c.notes_data {
            if nd.has_solo_sections { tags.push("Solos"); }
            if nd.has_lyrics { tags.push("Lyrics"); }
            if nd.has_tap_notes { tags.push("Taps"); }
            if nd.has_open_notes { tags.push("Opens"); }
            if nd.has_2x_kick { tags.push("2x Kick"); }
        }
        if c.has_video_background { tags.push("Video"); }
        if c.modchart { tags.push("Modchart"); }
        let tag_row = tags.into_iter().fold(row![].spacing(6), |r, t| {
            r.push(container(text(t).size(11)).padding([2, 8]).style(|th: &Theme| {
                if style::refined() {
                    style::outline_tag(th)
                } else {
                    container::Style {
                        background: Some(crate::style::strong(th).into()),
                        border: Border::default().rounded(10),
                        ..Default::default()
                    }
                }
            }))
        });

        let dl_state = self.downloads.iter().rev().find(|d| d.md5 == c.md5).map(|d| &d.status);
        #[cfg(feature = "rhythmverse")]
        let rv_dl = if dl_state.is_none() && !self.in_library(c) { self.rv_buttons(i, c, 15, 10) } else { None };
        #[cfg(not(feature = "rhythmverse"))]
        let rv_dl: Option<Element<'static, Message>> = None;
        // Redownload + Delete, or a long "Open on <host>" label, leave no room for Preview beside them.
        let busy = matches!(dl_state, Some(DlStatus::Queued) | Some(DlStatus::Active { .. }));
        #[cfg(feature = "rhythmverse")]
        let opens_link = rv_dl.is_some() && c.rv.as_ref().is_some_and(|rv| !rhythmverse::can_download(rv));
        #[cfg(not(feature = "rhythmverse"))]
        let opens_link = false;
        let crowded = (!busy && self.in_library(c)) || opens_link;
        let dl_button: Element<_> = if let Some(b) = rv_dl { b } else { match dl_state {
            Some(DlStatus::Queued) => {
                button(text("Queued…").size(15).width(Length::Fill).center()).width(Length::Fill).padding(10).into()
            }
            Some(DlStatus::Active { .. }) => {
                button(text("Downloading…").size(15).width(Length::Fill).center()).width(Length::Fill).padding(10).into()
            }
            _ if self.in_library(c) => self.library_buttons(i, c, 15, 10),
            _ => button(text("Download").size(15).width(Length::Fill).center())
                .width(Length::Fill)
                .padding(10)
                .style(button::primary)
                .on_press(Message::Download(i))
                .into(),
        } };
        let previewing = self.preview.as_ref().is_some_and(|p| p.md5 == c.md5);
        let preview_button = button(
            text(if previewing { "Stop preview" } else { "Preview" })
                .size(15)
                .width(if crowded { Length::Fill } else { Length::Shrink })
                .center()
                .wrapping(text::Wrapping::None),
        )
        .width(if crowded { Length::Fill } else { Length::Shrink })
        .padding([10, 16])
        .style(crate::style::secondary)
        .on_press_maybe(self.can_preview(c).then_some(if previewing {
            Message::Preview(preview_panel::Msg::Close)
        } else {
            Message::OpenPreview(i)
        }));
        let dl_button: Element<_> = if crowded {
            column![dl_button, preview_button].spacing(6).into()
        } else {
            row![dl_button, preview_button].spacing(8).into()
        };

        scrollable(
            column![
                art,
                column![
                    text(s(&c.name)).font(if style::refined() { Font { weight: iced::font::Weight::Semibold, ..Font::DEFAULT } } else { BOLD }).size(19),
                    search_link(s(&c.artist), 15),
                ]
                .spacing(2),
                column![
                    meta_line("Album", s(&c.album), true),
                    meta_line("Genre", s(&c.genre), true),
                    meta_line("Charter", s(&c.charter), true),
                    row![
                        container(meta_line("Year", s(&c.year), true)).width(Length::Fill),
                        container(meta_line("Length", c.length_str(), false)).width(Length::Fill),
                    ]
                    .spacing(10),
                ]
                .spacing(4),
                diffs,
                tag_row,
                self.chart_extras(i, c),
                dl_button,
                rule::horizontal(1).style(crate::style::rule),
                self.leaderboard_section(c),
            ]
            .spacing(10),
        )
        .spacing(6) // keep the scrollbar beside the content instead of over it
        .into()
    }

    fn view_settings(&self) -> Element<'_, Message> {
        let folders = self.settings.all_library_folders();
        let mut lib_col = column![].spacing(8);
        if folders.is_empty() {
            lib_col = lib_col.push(text("Not set — downloads need a songs folder").size(14));
        }
        for (idx, f) in folders.iter().enumerate() {
            let default: Element<_> = if idx == 0 {
                container(text("Downloads go here").size(12)).padding([4, 10]).style(crate::style::panel).into()
            } else {
                button(text("Make default").size(12)).padding([4, 10]).style(crate::style::secondary).on_press(Message::MakeDefaultLibrary(idx)).into()
            };
            lib_col = lib_col.push(
                row![
                    text(f.display().to_string()).size(14).width(Length::Fill).wrapping(text::Wrapping::WordOrGlyph),
                    default,
                    button(text("Remove").size(12)).padding([4, 10]).style(crate::style::subtle).on_press(Message::RemoveLibraryFolder(idx)),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        }
        let lib_status = if self.library_scanning {
            "Scanning your library…".to_string()
        } else if folders.is_empty() {
            String::new()
        } else {
            format!("{} charts found. Charts you already have are marked with ✓.", self.library_charts)
        };
        lib_col = lib_col.push(
            row![
                text(lib_status).size(12).width(Length::Fill),
                button(text("Rescan").size(13)).padding([6, 12]).style(crate::style::secondary).on_press(Message::RescanLibrary),
                button(text("Add folder…").size(13)).padding([6, 12]).on_press(Message::AddLibraryFolder),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        );

        let section = |title: &'static str, content: Element<'static, Message>| {
            container(column![text(title).font(BOLD).size(15), content].spacing(10))
                .padding(16)
                .width(Length::Fill)
                .style(crate::style::panel)
        };

        let themes = all_themes();
        let current = ThemeChoice(self.theme.clone());

        let content = column![
            text("Settings").font(BOLD).size(24),
            section("Library folders", lib_col.into()),
        ];
        #[cfg(feature = "rhythmverse")]
        let content = content.push(section("Chart sources", self.rv_settings_section()));
        let content = content.extend([
            section(
                "Chart folder name",
                column![
                    text_input("{artist} - {name} ({charter})", &self.settings.chart_folder_name)
                        .on_input(Message::FolderTemplateChanged)
                        .padding(8),
                    text("Placeholders: {name} {artist} {album} {genre} {year} {charter}. Use / for subfolders.")
                        .size(12),
                ]
                .spacing(6)
                .into(),
            ),
            section(
                "Downloads",
                column![
                    toggler(self.settings.is_sng)
                        .label("Save as .sng file (don't extract)")
                        .on_toggle(Message::ToggleSng),
                    toggler(self.settings.download_videos)
                        .label("Download background videos")
                        .on_toggle(Message::ToggleVideos),
                ]
                .spacing(10)
                .into(),
            ),
            section(
                "Appearance",
                column![
                    row![
                        text("Theme").size(13).width(70),
                        pick_list(themes, Some(current), Message::ThemePicked),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                    row![
                        text("Layout").size(13).width(70),
                        pick_list(variants::UiVariant::ALL, Some(self.ui), Message::LayoutPicked),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                    row![
                        text("Preview").size(13).width(70),
                        pick_list(FpsChoice::ALL, Some(FpsChoice(self.settings.preview_fps)), Message::FpsPicked),
                        text("Higher frame rates are smoother but use more CPU.").size(12),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                    toggler(self.settings.refined_ui)
                        .label("Beta UI")
                        .on_toggle(Message::ToggleRefined),
                    toggler(self.settings.cpu_rendering)
                        .label("Low memory mode: render on the CPU (~60 MB instead of ~140 MB; applies after restart)")
                        .on_toggle(Message::ToggleCpuRendering),
                    toggler(self.settings.show_fretwork)
                        .label("Show Fretwork difficulty tiers next to difficulties (otherwise hover a difficulty to see them)")
                        .on_toggle(Message::ToggleFretwork),
                    row![
                        text("Transparency (beta)").size(13),
                        pick_list(
                            backdrop::Backdrop::ALL,
                            Some(backdrop::Backdrop::from_key(&self.settings.window_backdrop)),
                            Message::BackdropPicked,
                        ),
                        text(if self.settings.cpu_rendering {
                            "Needs low memory mode off. Applies after restart."
                        } else if backdrop::Backdrop::from_key(&self.settings.window_backdrop) != self.backdrop {
                            "Restart Truss to apply."
                        } else {
                            "Applies after restart."
                        })
                        .size(12),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                    {
                        let frosted = backdrop::Backdrop::from_key(&self.settings.window_backdrop) == backdrop::Backdrop::Frosted;
                        let pct = (self.settings.glass_transparency * 100.0).round() as u32;
                        let mut amount = row![
                            text("See-through").size(13),
                            iced::widget::slider(0.1..=0.8, self.settings.glass_transparency, Message::GlassTransparency)
                                .step(0.05_f32)
                                .on_release(Message::SaveSettings)
                                .width(220),
                            text(format!("{pct}%")).size(12).width(40),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center);
                        if !frosted {
                            amount = amount.push(text("Turn on frosted glass to use this.").size(12));
                        }
                        amount
                    },
                ]
                .spacing(12)
                .into(),
            ),
            section("Updates", self.update_section()),
            section("Credits", credits_section()),
        ].map(Element::from))
        .spacing(14)
        .max_width(720);

        scrollable(container(content).padding(24).center_x(Length::Fill)).height(Length::Fill).into()
    }

    fn update_section(&self) -> Element<'static, Message> {
        let current = updater::current_version();
        let (status, action): (String, Option<Element<'static, Message>>) = match &self.update {
            UpdateState::Idle => (format!("Truss {current}"), None),
            UpdateState::Checking { .. } => (format!("Truss {current}. Checking for updates…"), None),
            UpdateState::UpToDate => (format!("Truss {current} is up to date."), None),
            UpdateState::Available(rel) => (
                format!("Truss {} is available (you have {current}).", rel.version),
                Some(
                    row![
                        button(text("What's new ↗").size(13))
                            .padding([6, 12])
                            .style(crate::style::secondary)
                            .on_press(Message::OpenUrl(rel.page_url.clone())),
                        button(text(format!("Update to {}", rel.version)).size(13)).padding([6, 12]).on_press(Message::InstallUpdate),
                    ]
                    .spacing(8)
                    .into(),
                ),
            ),
            UpdateState::Installing(rel, p) => (
                format!("Downloading Truss {}… {:.0}%", rel.version, p * 100.0),
                Some(progress_bar(0.0..=1.0, *p).length(160).girth(8).into()),
            ),
            UpdateState::Ready(v) => (
                format!("Truss {v} is installed. Restart to start using it."),
                Some(button(text("Restart now").size(13)).padding([6, 12]).on_press(Message::RestartForUpdate).into()),
            ),
            UpdateState::Failed(e) => (format!("Update failed: {e}"), None),
        };
        let busy = matches!(self.update, UpdateState::Checking { .. } | UpdateState::Installing(..));
        let mut r = row![text(status).size(13).width(Length::Fill)].spacing(10).align_y(Alignment::Center);
        if let Some(a) = action {
            r = r.push(a);
        }
        if !matches!(self.update, UpdateState::Available(_) | UpdateState::Ready(_) | UpdateState::Installing(..)) {
            r = r.push(
                button(text("Check now").size(13))
                    .padding([6, 12])
                    .style(crate::style::secondary)
                    .on_press_maybe((!busy).then_some(Message::CheckUpdates)),
            );
        }
        column![
            r,
            toggler(self.settings.check_updates).label("Check for updates when Truss starts").on_toggle(Message::ToggleCheckUpdates),
            changelog_view(self.show_changelog, &current),
        ]
        .spacing(10)
        .into()
    }

    fn view_downloads(&self) -> Element<'_, Message> {
        let active = self.downloads.iter().filter(|d| matches!(d.status, DlStatus::Active { .. })).count();
        let queued = self.downloads.iter().filter(|d| matches!(d.status, DlStatus::Queued)).count();
        let failed = self.downloads.iter().filter(|d| matches!(d.status, DlStatus::Failed(_))).count();
        let summary = if self.downloads.is_empty() {
            "No downloads — tip: Ctrl- or Shift-click results to download several at once".to_string()
        } else {
            let mut s = format!("{active} active, {} total", self.downloads.len());
            if queued > 0 {
                s.push_str(&format!(", {queued} queued"));
            }
            if failed > 0 {
                s.push_str(&format!(", {failed} failed"));
            }
            s
        };

        let bar = row![
            button(text(if self.show_downloads { "▾ Downloads" } else { "▸ Downloads" }).size(13))
                .style(button::text)
                .on_press(Message::ToggleDownloads),
            text(summary).size(13),
            space().width(Length::Fill),
        ]
        .extend((!self.checked.is_empty()).then(|| -> Element<'_, Message> {
            let n = self.checked.len();
            row![
                text(format!("{n} selected")).size(13),
                button(text(format!("Download {n}")).size(12))
                    .padding([4, 12])
                    .style(button::primary)
                    .on_press(Message::DownloadChecked),
                button(text("Clear selection").size(12)).padding([4, 10]).style(crate::style::secondary).on_press(Message::ClearChecked),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        }))
        .push(match &self.update {
            UpdateState::Available(rel) => Some(
                button(text(format!("Update to {}", rel.version)).size(12))
                    .padding([4, 10])
                    .style(button::primary)
                    .on_press(Message::InstallUpdate),
            ),
            UpdateState::Installing(_, p) => Some(button(text(format!("Updating… {:.0}%", p * 100.0)).size(12)).padding([4, 10])),
            UpdateState::Ready(_) => Some(
                button(text("Restart to update").size(12)).padding([4, 10]).style(button::primary).on_press(Message::RestartForUpdate),
            ),
            _ => None,
        })
        .push(
            button(text("Clear finished").size(12)).style(crate::style::subtle).padding([4, 10]).on_press(Message::ClearFinished),
        )
        .spacing(10)
        .align_y(Alignment::Center);

        let mut col = column![bar].spacing(6);
        if self.show_downloads && !self.downloads.is_empty() {
            let items = self.downloads.iter().rev().map(|d| {
                let (status_el, pct): (Element<_>, f32) = match &d.status {
                    DlStatus::Queued => (text("Waiting in queue").size(12).into(), 0.0),
                    DlStatus::Active { status, done, total } => {
                        let pct = total.map(|t| *done as f32 / t.max(1) as f32 * 100.0).unwrap_or(0.0);
                        let mb = *done as f64 / 1_048_576.0;
                        let label = match total {
                            Some(t) => format!("{status} {:.1} / {:.1} MB", mb, *t as f64 / 1_048_576.0),
                            None => format!("{status} {mb:.1} MB"),
                        };
                        (text(label).size(12).into(), pct)
                    }
                    DlStatus::Done(p) => (
                        button(text("Done — open folder").size(12))
                            .style(button::text)
                            .padding(0)
                            .on_press(Message::OpenPath(p.clone()))
                            .into(),
                        100.0,
                    ),
                    DlStatus::Failed(e) => (text(e.clone()).size(12).style(text::danger).into(), 0.0),
                };
                row![
                    column![
                        text(d.title.clone()).size(13).wrapping(text::Wrapping::None),
                        progress_bar(0.0..=100.0, pct).girth(4),
                        status_el,
                    ]
                    .spacing(3)
                    .width(Length::Fill),
                ]
                .extend(matches!(d.status, DlStatus::Failed(_)).then(|| -> Element<'_, Message> {
                    button(text("Retry").size(12)).style(crate::style::secondary).padding([4, 10]).on_press(Message::RetryDownload(d.id)).into()
                }))
                .push(button(text("✕").size(12)).style(crate::style::subtle).padding([4, 8]).on_press(Message::CancelDownload(d.id)))
                .spacing(10)
                .align_y(Alignment::Center)
                .into()
            });
            col = col.push(container(scrollable(column(items).spacing(10).padding([0, 12]))).max_height(180));
        }

        if style::refined() {
            return column![style::divider(false), container(col).padding([5, 18]).width(Length::Fill)].into();
        }
        container(col)
            .padding([6, 14])
            .width(Length::Fill)
            .style(|t: &Theme| container::Style {
                background: Some(crate::style::weak(t).into()),
                ..Default::default()
            })
            .into()
    }
}

/// Each version's notes from CHANGELOG.md (built into the app): (version, items), newest first.
fn changelog() -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for line in include_str!("../CHANGELOG.md").lines() {
        if let Some(v) = line.strip_prefix("## ") {
            out.push((v.trim().to_string(), Vec::new()));
        } else if let Some((_, items)) = out.last_mut() {
            if let Some(item) = line.strip_prefix("- ") {
                items.push(item.trim().to_string());
            } else if line.starts_with("  ")
                && let Some(last) = items.last_mut()
            {
                // A wrapped line of the item above.
                last.push(' ');
                last.push_str(line.trim());
            }
        }
    }
    for (_, items) in &mut out {
        for i in items.iter_mut() {
            *i = i.replace("**", "").replace('`', "");
        }
    }
    out
}

/// Settings > Updates: a collapsible list of what changed in each version.
fn changelog_view(open: bool, current: &str) -> Element<'static, Message> {
    let toggle = button(text(format!("{} Changelog", if open { "\u{25be}" } else { "\u{25b8}" })).size(13))
        .padding([4, 0])
        .style(button::text)
        .on_press(Message::ToggleChangelog);
    if !open {
        return toggle.into();
    }
    let muted = |t: &Theme| text::Style { color: Some(style::muted(t)) };
    let mut list = column![].spacing(14);
    for (version, items) in changelog() {
        let this = version.trim_start_matches('v') == current.trim_start_matches('v');
        let mut head = row![text(version.clone()).size(14).font(BOLD)].spacing(8).align_y(Alignment::Center);
        if this {
            head = head.push(text("(this version)").size(12).style(muted));
        }
        let mut block = column![head].spacing(4);
        for item in items {
            block = block.push(row![text("\u{2022}").size(12).style(muted), text(item).size(12)].spacing(6));
        }
        list = list.push(block);
    }
    column![toggle, container(list).padding(iced::Padding { left: 14.0, ..Default::default() })].spacing(8).into()
}

/// Settings > Credits: the projects Truss is ported from, uses, or is built with.
fn credits_section() -> Element<'static, Message> {
    let link = |name: &'static str, url: &'static str| {
        button(text(format!("{name} ↗")).size(13).wrapping(text::Wrapping::None))
            .padding(0)
            .style(|t: &Theme, st| button::Style {
                background: None,
                text_color: if matches!(st, button::Status::Hovered | button::Status::Pressed) { style::fg(t) } else { style::accent(t) },
                ..Default::default()
            })
            .on_press(Message::OpenUrl(url.to_string()))
    };
    let muted = |t: &Theme| text::Style { color: Some(style::muted(t)) };
    let entry = |name: &'static str, url: &'static str, what: &'static str| -> Element<'static, Message> {
        column![link(name, url), text(what).size(12).style(muted)].spacing(1).into()
    };
    let heading = |s: &'static str| -> Element<'static, Message> {
        container(text(s).size(12).font(BOLD)).padding(iced::Padding { top: 6.0, ..Default::default() }).into()
    };

    let libraries: &[(&str, &str)] = &[
        ("iced", "https://github.com/iced-rs/iced"),
        ("tokio", "https://github.com/tokio-rs/tokio"),
        ("reqwest", "https://github.com/seanmonstar/reqwest"),
        ("rustls", "https://github.com/rustls/rustls"),
        ("serde", "https://github.com/serde-rs/serde"),
        ("Symphonia", "https://github.com/pdeljanov/Symphonia"),
        ("cpal", "https://github.com/RustAudio/cpal"),
        ("midly", "https://github.com/kovaxis/midly"),
        ("image", "https://github.com/image-rs/image"),
        ("zip", "https://github.com/zip-rs/zip2"),
        ("sevenz-rust2", "https://github.com/hasenbanck/sevenz-rust2"),
        ("unrar", "https://github.com/muja/unrar.rs"),
        ("rusqlite", "https://github.com/rusqlite/rusqlite"),
        ("BLAKE3", "https://github.com/BLAKE3-team/BLAKE3"),
        ("rfd", "https://github.com/PolyMeilex/rfd"),
        ("trash", "https://github.com/Byron/trash-rs"),
        ("rust_xlsxwriter", "https://github.com/jmcnamara/rust_xlsxwriter"),
        ("tokio-tungstenite", "https://github.com/snapview/tokio-tungstenite"),
    ];
    let libs = libraries.iter().fold(row![].spacing(12), |r, (n, u)| r.push(link(n, u))).wrap().vertical_spacing(6);

    column![
        heading("Ported from"),
        entry("Bridge", "https://github.com/Geomitron/Bridge", "by Geomitron (GPL-3.0). Truss is a rewrite of Bridge; its search, downloads and folder naming follow it."),
        entry("scan-chart", "https://github.com/Geomitron/scan-chart", "by Geomitron (MIT). Chart reading, the issue checks, and the track hashes leaderboards use."),
        entry("chart-preview", "https://github.com/Geomitron/chart-preview", "by Geomitron (MIT). The note highway preview."),
        entry("Fretwork", "https://github.com/Staycation44/fretwork", "by Staycation (MIT). The Fretwork difficulty tiers."),
        entry("SngFileFormat", "https://github.com/mdsitton/SngFileFormat", "by mdsitton. The .sng file format."),
        heading("Related"),
        entry("CH Scene Switcher", "https://github.com/graindead/chswitcher", "Shares its settings with the Scene Switcher tool."),
        heading("Built with"),
        libs,
        text("Charts and scores come from Chorus Encore, RhythmVerse and the Clone Hero leaderboards; song requests from Ouvert.")
            .size(12)
            .style(muted),
    ]
    .spacing(8)
    .into()
}

/// Text in the details that searches for itself when clicked (artist, charter, genre…).
fn search_link<'a>(v: String, size: u32) -> Element<'a, Message> {
    let refined = style::refined();
    button(text(v.clone()).size(size))
        .padding(0)
        .style(move |t: &Theme, st| button::Style {
            background: None,
            text_color: if matches!(st, button::Status::Hovered | button::Status::Pressed) {
                style::accent(t)
            } else if refined && size > 13 {
                style::muted(t)
            } else {
                style::fg(t)
            },
            ..Default::default()
        })
        .on_press(Message::SearchFor(v))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TRUSS_TEST_ICON_DIR=<dir>: write the app icon in a few themes as PNGs to look at.
    #[test]
    fn themed_icons() {
        let rust = app_icon_rgba(&themes::rust_theme()).unwrap();
        let orig = ::image::load_from_memory(include_bytes!("../assets/truss-256.png")).unwrap().to_rgba8().into_raw();
        // The Rust theme gives back (nearly) the original icon.
        // (Only pixels you can see: the corners' near-transparent pixels hold junk colours.)
        let worst = rust.0.chunks(4).zip(orig.chunks(4)).filter(|(_, o)| o[3] >= 200).map(|(a, b)| (0..3).map(|i| a[i].abs_diff(b[i])).max().unwrap()).max().unwrap();
        assert!(worst <= 12, "Rust icon differs by {worst}");
        let Ok(dir) = std::env::var("TRUSS_TEST_ICON_DIR") else { return };
        for t in [themes::rust_theme(), themes::kusha_theme(), themes::nick_theme(), themes::bridge_dark(), Theme::Light, Theme::Dracula] {
            let (px, w, h) = app_icon_rgba(&t).unwrap();
            ::image::RgbaImage::from_raw(w, h, px).unwrap().save(format!("{dir}/icon-{t}.png")).unwrap();
        }
    }
}
