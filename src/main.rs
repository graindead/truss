#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod advanced;
mod art;
mod backup;
mod api;
mod audio;
mod chart;
mod download;
mod extras;
mod icons;
mod issues;
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
mod updater;
mod setlists;
mod sng;
mod textures;
mod tools_ui;
mod variants;
mod video;
mod vlist;

use api::{Chart, DecodedImage, DIFFICULTIES, DRUM_TYPES, INSTRUMENTS, strip_tags};
use iced::widget::{
    button, center, checkbox, column, container, image, pick_list, progress_bar, row, rule, scrollable, space, text,
    text_input, toggler,
};
use iced::{Alignment, Background, Border, Color, Element, Font, Length, Subscription, Task, Theme, color};
use extras::{library_mark, rv_tag};
use settings::Settings;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const SEARCH_ID: &str = "search";
const BOLD: Font = Font { weight: iced::font::Weight::Bold, ..Font::DEFAULT };

fn main() -> iced::Result {
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
    }

    iced::application(App::boot, App::update, App::view)
        .title("Truss")
        .theme(App::theme)
        .subscription(App::subscription)
        .window(iced::window::Settings {
            size: iced::Size::new(1280.0, 800.0),
            icon: iced::window::icon::from_file_data(include_bytes!("../assets/truss-256.png"), None).ok(),
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
    /// Bumped when row contents change without the list length changing (lazy list key).
    list_rev: u64,
    drum_type: Opt,
    drums_reviewed: Opt,
    drum_type_opts: Vec<Opt>,
    reviewed_opts: Vec<Opt>,
    show_issues: bool,
    show_versions: bool,

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
    InstrumentPicked(Opt),
    DifficultyPicked(Opt),
    DrumTypePicked(Opt),
    DrumsReviewedPicked(Opt),
    ModifiersChanged(iced::keyboard::Modifiers),
    PickVersion(usize, usize),
    ToggleIssues,
    ToggleVersions,
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
    ChartReread(PathBuf, Option<Arc<library::LocalChart>>),
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

fn bridge_dark() -> Theme {
    Theme::custom(
        "Bridge Dark",
        iced::theme::Palette {
            background: color!(0x15161c),
            text: color!(0xe4e4ea),
            primary: color!(0x7c5cff),
            success: color!(0x3ccf8e),
            warning: color!(0xf5b942),
            danger: color!(0xf0506e),
        },
    )
}

/// The default theme: warm charcoal with Rust-orange accents.
fn rust_theme() -> Theme {
    Theme::custom(
        "Rust",
        iced::theme::Palette {
            background: color!(0x1b1715),
            text: color!(0xefe6de),
            primary: color!(0xe2622a),
            success: color!(0x7cc47f),
            warning: color!(0xf2b545),
            danger: color!(0xe5484d),
        },
    )
}

fn all_themes() -> Vec<ThemeChoice> {
    [rust_theme(), bridge_dark()].into_iter().chain(Theme::ALL.iter().cloned()).map(ThemeChoice).collect()
}

fn theme_by_name(name: &str) -> Theme {
    all_themes().into_iter().map(|t| t.0).find(|t| t.to_string() == name).unwrap_or_else(rust_theme)
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let settings = Settings::load();
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
            list_rev: 0,
            show_issues: false,
            show_versions: false,
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
        let task = Task::batch([task, app.rv_auto_sync()]);
        (app, Task::batch([task, iced::widget::operation::focus(SEARCH_ID)]))
    }

    fn theme(&self) -> Theme {
        self.theme.clone()
    }

    fn subscription(&self) -> Subscription<Message> {
        let modifiers = iced::event::listen_with(|e, _, _| match e {
            iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(m)) => Some(Message::ModifiersChanged(m)),
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
        tools_ui::Ctx { catalog: &self.catalog, lyrics: &self.lyrics_map, settings: &self.settings }
    }

    /// Re-filters the Library and Tools lists after the catalog or lyrics data changed.
    fn refresh_pages(&mut self) {
        let mut lp = std::mem::take(&mut self.library_page);
        lp.refresh(&self.library_ctx());
        self.library_page = lp;
        let mut tp = std::mem::take(&mut self.tools_page);
        tp.refresh(&self.tools_ctx());
        self.tools_page = tp;
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
        let encore = self.encore_seen < self.found;
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

    fn load_art(&mut self, md5: Option<&String>) -> Task<Message> {
        let Some(md5) = md5 else { return Task::none() };
        if self.art.contains_key(md5) {
            return Task::none();
        }
        self.art.insert(md5.clone(), Art::Loading);
        self.fetch_art(md5.clone())
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

        if self.art.len() > 2 * ART_WINDOW + 50 {
            let mut keep: std::collections::HashSet<&String> = wanted.iter().collect();
            let selected = self.selected.and_then(|i| self.results.get(i)).and_then(|c| c.album_art_md5.as_ref());
            keep.extend(selected);
            let before = self.art.len();
            // Keep in-flight loads so their results aren't wasted.
            self.art.retain(|k, v| keep.contains(k) || matches!(v, Art::Loading));
            if self.art.len() != before {
                self.full_art_lru.retain(|m| self.art.contains_key(m));
                self.art_version += 1;
            }
        }
        let mut tasks: Vec<_> = wanted.iter().map(|m| self.load_art(Some(m))).collect();
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
            container(text(icons::tooltip(chart)).size(12)).padding([4, 8]).style(container::rounded_box),
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
            Message::Advanced(advanced::Msg::Search) => match self.advanced.to_filters() {
                Ok(filters) => {
                    self.advanced.error = None;
                    self.active_advanced = Some(Arc::new(filters));
                    self.query.clear();
                    self.search(false)
                }
                Err(e) => {
                    self.advanced.error = Some(e);
                    Task::none()
                }
            },
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
                Task::batch([close, self.ensure_full_art(md5.as_ref()), icon])
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
                let art = match r {
                    Ok((t, f)) => Art::Ready {
                        thumb: image::Handle::from_rgba(t.width, t.height, t.rgba),
                        full: Some(image::Handle::from_bytes(f)),
                    },
                    Err(_) => Art::Failed,
                };
                self.art.insert(md5.clone(), art);
                self.remember_full_art(&md5);
                self.art_version += 1;
                Task::none()
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
                self.checked = if all { (0..self.results.len()).collect() } else { BTreeSet::new() };
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
                Task::batch([close, self.ensure_full_art(art.as_ref()), self.load_icon(&chart)])
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
                    tools_ui::Out::ChartChanged(p) => {
                        let path = p.clone();
                        Task::perform(
                            async move {
                                tokio::task::spawn_blocking(move || library::read_chart(&path).map(Arc::new))
                                    .await
                                    .ok()
                                    .flatten()
                            },
                            move |c| Message::ChartReread(p.clone(), c),
                        )
                    }
                    tools_ui::Out::LyricsChanged(p, has) => {
                        self.lyrics_map.insert(p, has);
                        self.refresh_pages();
                        Task::none()
                    }
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
            Message::ChartReread(p, c) => {
                if let Some(c) = c
                    && let Some(slot) = self.catalog.iter_mut().find(|x| x.path == p)
                {
                    *slot = c;
                    self.refresh_pages();
                }
                Task::none()
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
                if self.screen == Screen::Tools && s != Screen::Tools {
                    self.tools_page.pause_audio();
                }
                self.screen = s;
                let mut tasks = Vec::new();
                if matches!(s, Screen::Library | Screen::Tools) {
                    if self.catalog_stale && !self.library_scanning {
                        tasks.push(self.scan_library());
                    } else if self.lyrics_map.is_empty() {
                        tasks.push(self.scan_lyrics());
                    }
                }
                if s == Screen::Tools {
                    tasks.push(self.tools_page.on_open().map(Message::ToolsPage));
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
                self.theme = t.0;
                self.save_settings()
            }
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

        container(
            row![
                text("Truss").font(BOLD).size(20),
                nav("Browse", Screen::Browse),
                nav("Library", Screen::Library),
                nav("Tools", Screen::Tools),
                nav("Settings", Screen::Settings),
                search,
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
            .align_y(Alignment::Center),
        )
        .padding([10, 14])
        .style(|t: &Theme| container::Style {
            background: Some(t.extended_palette().background.weak.color.into()),
            ..Default::default()
        })
        .into()
    }

    fn view_browse(&self) -> Element<'_, Message> {
        match self.ui {
            variants::UiVariant::Grid => return self.view_browse_grid(),
            variants::UiVariant::Compact => return self.view_browse_compact(),
            variants::UiVariant::Hero => return self.view_browse_hero(),
            variants::UiVariant::Default => {}
        }
        let expanded = self.preview.as_ref().is_some_and(|p| p.expanded());
        let sidebar = container(self.view_sidebar())
            .width(if expanded { 720 } else { 360 })
            .height(Length::Fill)
            .padding(16)
            .style(|t: &Theme| container::Style {
                background: Some(t.extended_palette().background.weakest.color.into()),
                ..Default::default()
            });
        row![self.lazy_results(), sidebar].height(Length::Fill).into()
    }

    /// Default layout's results list (header + rows), built with owned data for `lazy`.
    fn list_results(&self) -> Element<'static, Message> {
        use api::{SortDir, SortKey};
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
        let header = row![
            self.check_all_box(),
            space().width(44),
            col("Name", SortKey::Name, Length::FillPortion(4)),
            col("Artist", SortKey::Artist, Length::FillPortion(3)),
            col("Album", SortKey::Album, Length::FillPortion(3)),
            col("Charter", SortKey::Charter, Length::FillPortion(2)),
            col("Length", SortKey::Length, Length::Fixed(72.0)),
        ]
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
            rule::horizontal(1),
            scrollable(list).on_scroll(Message::Scrolled).height(Length::Fill).width(Length::Fill),
        ]
        .width(Length::Fill)
        .into()
    }

    /// The tick box that marks a result for "Download N".
    pub fn check_box(&self, i: usize) -> Element<'static, Message> {
        checkbox(self.checked.contains(&i)).size(16).on_toggle(move |_| Message::ToggleChecked(i)).into()
    }

    /// The header tick box: ticks every loaded result, or clears them all.
    pub fn check_all_box(&self) -> Element<'static, Message> {
        let all = !self.results.is_empty() && self.checked.len() == self.results.len();
        checkbox(all).size(16).on_toggle(Message::CheckAll).into()
    }

    fn view_row(&self, i: usize, c: &Chart) -> Element<'static, Message> {
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
                    background: Some(t.extended_palette().background.strong.color.into()),
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
                cell(&c.album, 3),
                cell(&c.charter, 2),
                text(c.length_str()).size(14).width(72),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding([4, 6])
        .on_press(Message::Select(i))
        .style(move |t: &Theme, s| {
            let p = t.extended_palette();
            let bg = match (selected, s) {
                (true, _) => Some(p.primary.weak.color),
                (false, _) if checked => Some(p.primary.weak.color.scale_alpha(0.45)),
                (false, button::Status::Hovered) => Some(p.background.weak.color),
                _ => None,
            };
            button::Style {
                background: bg.map(Background::Color),
                text_color: if selected { p.primary.weak.text } else { p.background.base.text },
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

        let art: Element<_> = match (&self.preview, c.album_art_md5.as_ref().and_then(|m| self.art.get(m))) {
            // The preview replaces the album art while it's open.
            (Some(p), _) if p.md5 == c.md5 => {
                p.view(if p.expanded() { 500.0 } else { 328.0 }).map(Message::Preview)
            }
            (_, Some(Art::Ready { full: Some(full), .. })) => {
                self.with_icon(image(full.clone()).width(Length::Fill).into(), c, 64.0, 12.0)
            }
            (_, Some(Art::Ready { thumb, .. })) => {
                self.with_icon(image(thumb.clone()).width(Length::Fill).into(), c, 64.0, 12.0)
            }
            (_, Some(Art::Loading)) => container(center(text("Loading art…").size(13))).height(328).into(),
            _ => container(center(text("No album art").size(13)))
                .height(328)
                .width(Length::Fill)
                .style(container::rounded_box)
                .into(),
        };

        let s = |o: &Option<String>| o.as_deref().map(strip_tags).unwrap_or_default();
        let meta_line = |label: &'static str, v: String| -> Element<'_, Message> {
            if v.is_empty() {
                space().into()
            } else {
                row![text(label).size(13).width(70).color(Color { a: 0.6, ..Color::WHITE }), text(v).size(13)]
                    .spacing(8)
                    .into()
            }
        };

        let mut diffs = column![].spacing(4);
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
        for (key, label) in instruments {
            if let Some(d) = c.diff_for(key) {
                let pips = (0..6).fold(row![].spacing(3), |r, n| {
                    let on = n < d.min(6);
                    r.push(container(space()).width(14).height(8).style(move |t: &Theme| container::Style {
                        background: Some(
                            if on { t.extended_palette().primary.base.color } else { t.extended_palette().background.strong.color }
                                .into(),
                        ),
                        border: Border::default().rounded(2),
                        ..Default::default()
                    }))
                });
                diffs = diffs.push(
                    row![text(*label).size(13).width(84), pips, text(d.to_string()).size(12)]
                        .spacing(8)
                        .align_y(Alignment::Center),
                );
            }
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
            r.push(container(text(t).size(11)).padding([2, 8]).style(|th: &Theme| container::Style {
                background: Some(th.extended_palette().background.strong.color.into()),
                border: Border::default().rounded(10),
                ..Default::default()
            }))
        });

        let dl_state = self.downloads.iter().rev().find(|d| d.md5 == c.md5).map(|d| &d.status);
        #[cfg(feature = "rhythmverse")]
        let rv_dl = if dl_state.is_none() && !self.in_library(c) { self.rv_buttons(i, c, 15, 10) } else { None };
        #[cfg(not(feature = "rhythmverse"))]
        let rv_dl: Option<Element<'static, Message>> = None;
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
        let preview_button = button(text(if previewing { "Stop preview" } else { "Preview" }).size(15).center())
            .padding([10, 16])
            .style(button::secondary)
            .on_press_maybe(self.can_preview(c).then_some(if previewing {
                Message::Preview(preview_panel::Msg::Close)
            } else {
                Message::OpenPreview(i)
            }));
        let dl_button = row![dl_button, preview_button].spacing(8);

        scrollable(
            column![
                art,
                text(s(&c.name)).font(BOLD).size(20),
                text(s(&c.artist)).size(15),
                meta_line("Album", s(&c.album)),
                meta_line("Year", s(&c.year)),
                meta_line("Genre", s(&c.genre)),
                meta_line("Charter", s(&c.charter)),
                meta_line("Length", c.length_str()),
                tag_row,
                self.chart_extras(i, c),
                dl_button,
                rule::horizontal(1),
                diffs,
            ]
            .spacing(8),
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
                container(text("Downloads go here").size(12)).padding([4, 10]).style(container::rounded_box).into()
            } else {
                button(text("Make default").size(12)).padding([4, 10]).style(button::secondary).on_press(Message::MakeDefaultLibrary(idx)).into()
            };
            lib_col = lib_col.push(
                row![
                    text(f.display().to_string()).size(14).width(Length::Fill).wrapping(text::Wrapping::WordOrGlyph),
                    default,
                    button(text("Remove").size(12)).padding([4, 10]).style(button::subtle).on_press(Message::RemoveLibraryFolder(idx)),
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
                button(text("Rescan").size(13)).padding([6, 12]).style(button::secondary).on_press(Message::RescanLibrary),
                button(text("Add folder…").size(13)).padding([6, 12]).on_press(Message::AddLibraryFolder),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        );

        let section = |title: &'static str, content: Element<'static, Message>| {
            container(column![text(title).font(BOLD).size(15), content].spacing(10))
                .padding(16)
                .width(Length::Fill)
                .style(container::rounded_box)
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
                    toggler(self.settings.cpu_rendering)
                        .label("Low memory mode: render on the CPU (~60 MB instead of ~140 MB; applies after restart)")
                        .on_toggle(Message::ToggleCpuRendering),
                ]
                .spacing(12)
                .into(),
            ),
            section("Updates", self.update_section()),
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
                            .style(button::secondary)
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
                    .style(button::secondary)
                    .on_press_maybe((!busy).then_some(Message::CheckUpdates)),
            );
        }
        column![
            r,
            toggler(self.settings.check_updates).label("Check for updates when Truss starts").on_toggle(Message::ToggleCheckUpdates),
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
                button(text("Clear selection").size(12)).padding([4, 10]).style(button::secondary).on_press(Message::ClearChecked),
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
            button(text("Clear finished").size(12)).style(button::subtle).padding([4, 10]).on_press(Message::ClearFinished),
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
                    button(text("Retry").size(12)).style(button::secondary).padding([4, 10]).on_press(Message::RetryDownload(d.id)).into()
                }))
                .push(button(text("✕").size(12)).style(button::subtle).padding([4, 8]).on_press(Message::CancelDownload(d.id)))
                .spacing(10)
                .align_y(Alignment::Center)
                .into()
            });
            col = col.push(container(scrollable(column(items).spacing(10).padding([0, 12]))).max_height(180));
        }

        container(col)
            .padding([6, 14])
            .width(Length::Fill)
            .style(|t: &Theme| container::Style {
                background: Some(t.extended_palette().background.weak.color.into()),
                ..Default::default()
            })
            .into()
    }
}
