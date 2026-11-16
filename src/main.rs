#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod advanced;
mod api;
mod art_cache;
mod rich;
mod audio;
mod chart;
mod chopt;
mod curves;
mod graph;
mod download;
mod extras;
mod icons;
mod issues;
mod leaderboard;
mod leaderboard_ui;
mod library;
mod library_tools;
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
mod theme_maker;
mod backdrop;
mod icon_paint;
mod fretwork;
mod fretwork_filter;
mod tools_ui;
mod path_ui;
mod packs;
mod pack_index;
#[cfg(feature = "rhythmverse")]
mod mega;
mod variants;
mod vlist;

use api::{Chart, DecodedImage, DIFFICULTIES, DRUM_TYPES, INSTRUMENTS, strip_tags};
use iced::widget::{
    button, center, checkbox, column, container, image, pick_list, progress_bar, row, rule, scrollable, space, text,
    text_input, toggler,
};
use iced::{Alignment, Border, Color, Element, Font, Length, Subscription, Task, Theme};
use extras::{library_mark, other_version_mark, rv_tag};
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
    // Keep the album art cache under its size limit, without holding up start-up.
    std::thread::spawn(art_cache::trim);

    let (initial_size, initial_position) = initial_window();
    iced::application(App::boot, App::update, App::view)
        .title("Truss")
        .theme(App::theme)
        .style(App::app_style)
        .subscription(App::subscription)
        .window(iced::window::Settings {
            size: initial_size,
            position: initial_position,
            icon: app_icon(&theme_by_name(&settings.theme, &settings.custom_themes))
                .or_else(|| iced::window::icon::from_file_data(include_bytes!("../assets/truss-256.png"), None).ok()),
            transparent,
            ..Default::default()
        })
        .antialiasing(false)
        .run()
}

/// The window's first size and place: 1280x800 at most, shrunk to fit the screen's usable
/// area (not under the taskbar) and centred in it.
fn initial_window() -> (iced::Size, iced::window::Position) {
    let want: iced::Size<f32> = iced::Size::new(1280.0, 800.0);
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::HiDpi::GetDpiForSystem;
        use windows::Win32::UI::WindowsAndMessaging::{SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW};
        let mut wa = RECT::default();
        // SAFETY: SPI_GETWORKAREA writes one RECT to the pointer given.
        let ok = unsafe {
            SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut wa as *mut RECT as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)).is_ok()
        };
        if ok && wa.right > wa.left && wa.bottom > wa.top {
            // The work area is in this process's pixels: physical if it's DPI aware yet, else
            // already scaled to 96 DPI. Either way this gives logical units.
            let scale = (unsafe { GetDpiForSystem() }.max(96)) as f32 / 96.0;
            let (left, top) = (wa.left as f32 / scale, wa.top as f32 / scale);
            let (aw, ah) = ((wa.right - wa.left) as f32 / scale, (wa.bottom - wa.top) as f32 / scale);
            // Room for the title bar and borders, plus a small margin.
            const FRAME_W: f32 = 16.0;
            const FRAME_H: f32 = 40.0;
            const MARGIN: f32 = 16.0;
            let size = iced::Size::new(
                want.width.min(aw - FRAME_W - 2.0 * MARGIN).max(640.0),
                want.height.min(ah - FRAME_H - 2.0 * MARGIN).max(480.0),
            );
            let x = left + ((aw - size.width - FRAME_W) / 2.0).max(0.0);
            let y = top + ((ah - size.height - FRAME_H) / 2.0).max(0.0);
            return (size, iced::window::Position::Specific(iced::Point::new(x, y)));
        }
    }
    (want, iced::window::Position::Centered)
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
/// Covers read from the disk cache per worker task.
const ART_CACHE_CHUNK: usize = 12;

/// Downloads that run at the same time; the rest wait in the queue.
const MAX_ACTIVE_DOWNLOADS: usize = 2;

#[derive(Debug, Clone)]
enum DlStatus {
    Queued,
    Active { status: String, done: u64, total: Option<u64> },
    Done(PathBuf),
    Failed(String),
}

/// The grouped download notices in the corner.
#[derive(Default)]
struct Notices {
    /// Charts downloaded since the "done" notice appeared, and the last one.
    done: usize,
    last_title: String,
    last_path: Option<PathBuf>,
    done_at: Option<std::time::Instant>,
    /// Downloads that failed since the "failed" notice appeared.
    failed: usize,
}

/// How often to look for newly uploaded charts while Truss is open.
const NEW_CHARTS_SECS: u64 = 3 * 60;

/// How long the "done" notice stays after the last download of a batch finishes.
const NOTICE_SECS: u64 = 6;

struct DownloadEntry {
    id: u64,
    md5: String,
    chart: Arc<Chart>,
    /// Old copies to replace once this download succeeds (a redownload).
    replace: Vec<PathBuf>,
    /// A chart pack (Packs page) instead of a chart.
    pack: Option<packs::Job>,
    title: String,
    status: DlStatus,
    handle: Option<iced::task::Handle>,
}

/// An instrument in the difficulty graph's picker.
#[derive(Debug, Clone, PartialEq)]
struct GraphInst(String);

impl std::fmt::Display for GraphInst {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&leaderboard::instrument_label(&self.0))
    }
}

/// A library chart opened on the Library page (`chart` once its song.ini is read).
struct LibDetails {
    path: PathBuf,
    chart: Option<Arc<Chart>>,
    art: Option<image::Handle>,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Screen {
    Browse,
    Packs,
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
    /// The same by chart file hash (Encore's `chartHash`).
    library_hashes: HashMap<String, Vec<PathBuf>>,
    /// Songs in the library, for spotting other versions (see `library::Library::songs`).
    library_songs: HashMap<String, Vec<(Vec<String>, PathBuf)>>,
    /// Track hashes of library charts, read when a search result might be the same notes with
    /// different metadata (see `same_notes_copies`).
    local_tracks: std::sync::Mutex<HashMap<PathBuf, Vec<String>>>,
    library_charts: usize,
    /// Chart (md5) whose Delete button was clicked once and is waiting for the confirming click.
    confirm_delete: Option<String>,
    delete_error: Option<String>,
    library_scanning: bool,
    /// Charts added to (path) or removed from (paths) the library while a scan was running.
    /// The scan may have missed them, so they're applied again to its result.
    library_changes: Vec<(String, Option<PathBuf>, Vec<PathBuf>)>,
    /// Charts found so far by the running library scan, and how many the last scan found.
    library_scan_found: usize,
    library_scan_expected: Option<usize>,
    library_scan_handle: Option<iced::task::Handle>,
    /// The library chart whose details are open on the Library page.
    lib_details: Option<LibDetails>,
    /// The instrument picked for the Library details' difficulty graph.
    lib_graph_inst: Option<String>,
    /// Every chart in the library folders (Library page and tools).
    catalog: Vec<Arc<library::LocalChart>>,
    /// Which local charts have lyrics; filled in the first time Library/Tools is opened.
    lyrics_map: HashMap<PathBuf, bool>,
    lyrics_scanning: bool,
    /// Downloads/deletes happened since the last scan.
    catalog_stale: bool,
    library_error: Option<String>,
    /// What the last library clean-up did (shown on the Library page).
    library_notice: Option<String>,
    /// A library clean-up (duplicate search, video removal) is running.
    library_busy: bool,
    /// Loose charts found to be in a pack too, waiting for the user to decide.
    merge_offer: Option<MergeOffer>,
    /// Folders of packs just downloaded, checked for loose duplicates once the library is read.
    pack_dup_check: Vec<PathBuf>,
    library_page: library_ui::Page,
    tools_page: tools_ui::Page,
    packs_page: packs::Page,
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
    /// Download notices (finished / failed), grouped so a batch shows one of each.
    notices: Notices,

    preview: Option<preview_panel::Panel>,
    update: UpdateState,
    ui: variants::UiVariant,
}

#[derive(Debug, Clone)]
enum Message {
    QueryChanged(String),
    Debounced(u64),
    /// An Advanced search filter changed a moment ago (this generation): search again.
    AdvancedDebounced(u64),
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
    ToggleFretwork(bool),
    ToggleTextColors(bool),
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
    LibraryScan(library::ScanEvent),
    /// A library chart's details were read (its folder, the chart, its album art).
    LibraryDetails(PathBuf, Arc<Chart>, Option<image::Handle>),
    /// Preview the library chart whose details are open.
    OpenLocalPreview,
    /// The instrument shown in the Library details' difficulty graph.
    LibraryGraphInstrument(String),
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
    PacksPage(packs::Msg),
    LyricsScanned(HashMap<PathBuf, bool>),
    LibraryDeleted(Result<(), String>),
    /// Loose copies of pack charts were found (`Some`: the pack just downloaded).
    PackDuplicates(Vec<library_tools::Duplicate>, Option<String>),
    MergeToggle(usize),
    MergeConfirm,
    MergeCancel,
    VideosRemoved(Result<usize, String>),
    RescanLibrary,
    AddLibraryFolder,
    PickPackFolder,
    PackFolderPicked(Option<PathBuf>),
    ClearPackFolder,
    LibraryFolderAdded(Option<PathBuf>),
    RemoveLibraryFolder(usize),
    MakeDefaultLibrary(usize),
    SortBy(api::SortKey),
    ToggleAdvanced,
    Advanced(advanced::Msg),
    SearchDone { generation: u64, append: bool, result: Result<api::SearchResponse, String> },
    /// Every few minutes: look for charts uploaded since the list loaded.
    CheckNewCharts,
    /// The first page of Encore results again, to compare with what's shown.
    NewChartsChecked(Result<api::SearchResponse, String>),
    Scrolled(scrollable::Viewport),
    Select(usize),
    ArtLoaded(String, Result<(DecodedImage, bytes::Bytes), String>),
    /// Covers looked up in the disk cache; misses go on to download.
    ArtCached(Vec<(String, art_cache::Cached)>),
    IconList(Result<Arc<Vec<icons::IconEntry>>, String>),
    IconLoaded(String, Result<bytes::Bytes, String>),

    Download(usize),
    DlProgress(u64, download::Progress),
    CancelDownload(u64),
    ClearFinished,
    ToggleDownloads,
    /// Retries every failed download.
    RetryAllFailed,
    /// Closes a notice: true the "failed" one, false the "done" one.
    DismissNotice(bool),
    /// Once a second while a notice shows, to let it time out.
    NoticeTick,
    ToggleDownloadNotices(bool),
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

/// The built-in themes, then the ones made in Tools > Themes.
fn all_themes(custom: &[theme_maker::CustomTheme]) -> Vec<ThemeChoice> {
    themes::all().into_iter().chain(custom.iter().map(|t| t.theme())).map(ThemeChoice).collect()
}

/// The transparency to use this session (none with CPU rendering, which can't show it).
fn active_backdrop(s: &Settings) -> backdrop::Backdrop {
    if s.cpu_rendering {
        return backdrop::Backdrop::Off;
    }
    match theme_transparency(s) {
        Some(t) if t > 0.0 => backdrop::Backdrop::Frosted,
        Some(_) => backdrop::Backdrop::Off,
        None => backdrop::Backdrop::from_key(&s.window_backdrop),
    }
}

/// The transparency the current theme sets (made in Tools > Themes): 0 for solid, otherwise
/// the frosted glass see-through amount. None when it leaves it to Settings.
fn theme_transparency(s: &Settings) -> Option<f32> {
    s.custom_themes.iter().find(|t| t.name == s.theme).and_then(|t| t.transparency)
}

/// Points this exe's taskbar pins at the theme's icon embedded in truss.exe (see build.rs), or
/// back at the main icon for the first (Rust) theme.
fn pin_icon_task(theme: &Theme) -> Task<Message> {
    let name = theme.to_string();
    // Made-in-Truss themes have no icon in the exe, so their pins get the main icon.
    let i = themes::all().iter().position(|t| t.to_string() == name).unwrap_or(0);
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

fn theme_by_name(name: &str, custom: &[theme_maker::CustomTheme]) -> Theme {
    all_themes(custom).into_iter().map(|t| t.0).find(|t| t.to_string() == name).unwrap_or_else(themes::rust_theme)
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let settings = Settings::load();
        rich::set_enabled(settings.text_colors);
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
            library_hashes: HashMap::new(),
            library_songs: HashMap::new(),
            local_tracks: Default::default(),
            confirm_delete: None,
            delete_error: None,
            library_charts: 0,
            library_scanning: false,
            library_changes: Vec::new(),
            library_scan_found: 0,
            library_scan_expected: None,
            library_scan_handle: None,
            lib_details: None,
            lib_graph_inst: None,
            catalog: Vec::new(),
            lyrics_map: HashMap::new(),
            lyrics_scanning: false,
            catalog_stale: false,
            library_error: None,
            library_notice: None,
            library_busy: false,
            merge_offer: None,
            pack_dup_check: Vec::new(),
            library_page: library_ui::Page::with_sort(&settings.library_sort),
            tools_page: Default::default(),
            packs_page: Default::default(),
            cleared_generation: 0,
            encore_seen: 0,
            #[cfg(feature = "rhythmverse")]
            rv: Default::default(),
            theme: theme_by_name(&settings.theme, &settings.custom_themes),
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
            notices: Notices::default(),
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
        let a = match theme_transparency(&self.settings) {
            _ if self.backdrop == backdrop::Backdrop::Off => 1.0,
            Some(t) if t > 0.0 => self.backdrop.alpha(t),
            Some(_) => 1.0,
            None => self.backdrop.alpha(self.settings.glass_transparency),
        };
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
            if self.notices.done > 0 {
                iced::time::every(std::time::Duration::from_secs(1)).map(|_| Message::NoticeTick)
            } else {
                Subscription::none()
            },
            iced::time::every(std::time::Duration::from_secs(NEW_CHARTS_SECS)).map(|_| Message::CheckNewCharts),
        ]);
        match &self.preview {
            Some(p) => {
                Subscription::batch([modifiers, tools, p.subscription(self.settings.preview_fps).map(Message::Preview)])
            }
            None => Subscription::batch([modifiers, tools]),
        }
    }

    /// Whether the results can be refreshed without getting in your way: on Browse, at the top
    /// of the list, nothing open or ticked, and no search or filter typed in.
    pub fn list_idle(&self) -> bool {
        self.screen == Screen::Browse
            && self.query.is_empty()
            && self.active_advanced.is_none()
            && self.selected.is_none()
            && self.checked.is_empty()
            && self.art_center == 0
            && !self.fw_active()
            && !self.loading
    }

    /// Looks for new uploads: RhythmVerse into the local copy (which refreshes the list when it
    /// finds some), and Encore by comparing its first page with what's shown.
    fn check_new_charts(&mut self) -> Task<Message> {
        #[cfg(feature = "rhythmverse")]
        let rv = if self.rv_enabled() && self.rv.catalog.ready { self.rv_start_sync() } else { Task::none() };
        #[cfg(not(feature = "rhythmverse"))]
        let rv = Task::none();
        if !self.settings.source_encore || !self.list_idle() {
            return rv;
        }
        let mut common = api::Common::new(1, self.instrument.key.map(String::from), self.difficulty.key.map(String::from), self.sort);
        if self.instrument.key == Some("drums") {
            common.drum_type = self.drum_type.key.map(String::from);
            common.drums_reviewed = self.drums_reviewed.key.is_some();
        }
        Task::batch([rv, Task::perform(api::search(String::new(), common), Message::NewChartsChecked)])
    }

    fn library_ctx(&self) -> library_ui::Ctx<'_> {
        library_ui::Ctx {
            catalog: &self.catalog,
            lyrics: &self.lyrics_map,
            scanning: self.library_scanning,
            lyrics_scanning: self.lyrics_scanning,
            has_folders: !self.settings.all_library_folders().is_empty(),
            error: self.library_error.as_deref(),
            notice: self.library_notice.as_deref(),
            busy: self.library_busy,
        }
    }

    /// Saves, deletes or imports themes made in Tools > Themes.
    fn custom_theme_change(&mut self, o: theme_maker::Out) -> Task<Message> {
        let list = &mut self.settings.custom_themes;
        let mut switch_to = None;
        match o {
            theme_maker::Out::None => return Task::none(),
            theme_maker::Out::Save { theme, old_name, apply } => {
                let was = old_name.as_deref().and_then(|n| list.iter().position(|t| t.name == n));
                let in_use = old_name.as_deref().is_some_and(|n| self.settings.theme == n);
                match was {
                    Some(i) => list[i] = theme.clone(),
                    None => list.push(theme.clone()),
                }
                // Editing the theme in use (or asking to use it) shows the change straight away.
                if apply || in_use {
                    switch_to = Some(theme.theme());
                }
            }
            theme_maker::Out::Delete(name) => {
                list.retain(|t| t.name != name);
                if self.settings.theme == name {
                    switch_to = Some(themes::rust_theme());
                }
            }
            theme_maker::Out::Import(added) => list.extend(added),
        }
        match switch_to {
            Some(t) => self.update(Message::ThemePicked(ThemeChoice(t))),
            None => self.save_settings(),
        }
    }

    fn tools_ctx(&self) -> tools_ui::Ctx<'_> {
        tools_ui::Ctx { settings: &self.settings, catalog: &self.catalog, theme: &self.theme, glass_window: self.backdrop != backdrop::Backdrop::Off }
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
        let folders = self.settings.scan_folders();
        if folders.is_empty() {
            self.library.clear();
            self.library_hashes.clear();
            self.library_songs.clear();
            self.library_charts = 0;
            self.list_rev += 1;
            return Task::none();
        }
        // A rescan replaces any scan still running.
        if let Some(h) = self.library_scan_handle.take() {
            h.abort();
        }
        self.library_scanning = true;
        self.library_scan_found = 0;
        self.library_scan_expected = (self.library_charts > 0).then_some(self.library_charts);
        // Show the library saved by the last scan while this one runs (at start-up).
        let use_cache = self.catalog.is_empty() && self.library.is_empty();
        let (task, handle) = Task::run(library::scan(folders, use_cache), Message::LibraryScan).abortable();
        self.library_scan_handle = Some(handle);
        task
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
        !self.library_matches(c).is_empty()
    }

    /// Library charts of the same song and charter as this one (another version, perhaps).
    fn same_song(&self, c: &Chart) -> Vec<PathBuf> {
        let Some(k) = library::song_key(c.name.as_deref().unwrap_or(""), c.charter.as_deref().unwrap_or("")) else { return Vec::new() };
        let Some(have) = self.library_songs.get(&k) else { return Vec::new() };
        let artist = library::artist_words(c.artist.as_deref().unwrap_or(""));
        have.iter().filter(|(a, _)| library::same_artist(a, &artist)).map(|(_, p)| p.clone()).collect()
    }

    /// True when the library has a different version of this chart (same song and charter,
    /// different notes) but not this one.
    pub fn other_version_in_library(&self, c: &Chart) -> bool {
        !self.same_song(c).is_empty() && !self.in_library(c)
    }

    /// Library copies of the same song whose notes match this chart's track for track, though
    /// the files differ (a pack's copy with its metadata written differently, say).
    fn same_notes_copies(&self, c: &Chart) -> Vec<PathBuf> {
        let Some(nd) = &c.notes_data else { return Vec::new() };
        let mut want: Vec<String> =
            nd.track_hashes.iter().map(|t| format!("{}/{}/{}", t.instrument, t.difficulty, t.hash.trim_end_matches('='))).collect();
        if want.is_empty() {
            return Vec::new();
        }
        want.sort();
        let mut cache = self.local_tracks.lock().unwrap_or_else(|e| e.into_inner());
        self.same_song(c)
            .into_iter()
            .filter(|p| *cache.entry(p.clone()).or_insert_with(|| library::track_hashes(p)) == want)
            .collect()
    }

    /// Library copies of a chart: with the same chart file (Encore's `chartHash`), the same
    /// artist, name and charter, or the same notes.
    fn library_matches(&self, c: &Chart) -> Vec<PathBuf> {
        let by_hash = c.chart_hash.as_ref().and_then(|h| self.library_hashes.get(h)).filter(|p| !p.is_empty());
        match by_hash.or_else(|| self.library.get(&c.library_key()).filter(|p| !p.is_empty())) {
            Some(p) => p.clone(),
            None => self.same_notes_copies(c),
        }
    }

    /// A copy of the chart already on this computer (in the library), to read instead of
    /// downloading it again for previews, leaderboards and Fretwork ratings.
    pub fn library_copy(&self, c: &Chart) -> Option<PathBuf> {
        self.library_matches(c).first().cloned()
    }

    /// The chart whose details are showing: the open library chart on the Library page, else
    /// the selected search result.
    pub fn current_chart(&self) -> Option<&Chart> {
        if self.screen == Screen::Library {
            return self.lib_details.as_ref().and_then(|d| d.chart.as_deref());
        }
        self.selected.and_then(|i| self.results.get(i)).map(|c| &**c)
    }

    /// Opens (or with `None` closes) a library chart's details: reads its song.ini and art.
    fn open_library_details(&mut self, c: Option<Arc<library::LocalChart>>) -> Task<Message> {
        let close = match &self.preview {
            Some(p) if p.md5.starts_with(library::LOCAL_MD5) => self.close_preview(),
            _ => Task::none(),
        };
        let Some(c) = c else {
            self.lib_details = None;
            return close;
        };
        let path = c.path.clone();
        self.lib_details = Some(LibDetails { path: path.clone(), chart: None, art: None });
        let read = Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let (chart, art) = library::details(&c);
                    (Arc::new(chart), art.map(image::Handle::from_bytes))
                })
                .await
                .unwrap_or_else(|_| (Arc::new(Chart::default()), None))
            },
            move |(chart, art)| Message::LibraryDetails(path.clone(), chart, art),
        );
        Task::batch([close, read])
    }

    /// Height of the details below the open row: taller while an expanded preview shows.
    fn library_details_height(&self) -> f32 {
        let expanded = self.preview.as_ref().is_some_and(|p| p.md5.starts_with(library::LOCAL_MD5) && p.expanded());
        if expanded { 660.0 } else { 440.0 }
    }

    /// The details below the open row on the Library page.
    fn library_details_view(&self) -> Option<Element<'_, Message>> {
        let d = self.lib_details.as_ref()?;
        if self.library_page.open_chart() != Some(&d.path) {
            return None;
        }
        let Some(c) = d.chart.as_deref() else {
            return Some(center(text("Reading the chart…").size(13)).into());
        };
        let expanded = self.preview.as_ref().is_some_and(|p| p.md5 == c.md5 && p.expanded());
        // The preview's highway is square like the album art it replaces (bigger when expanded).
        let left_w: f32 = if expanded { 520.0 } else { 300.0 };
        let h = self.library_details_height() - 24.0;
        let previewing = self.preview.as_ref().filter(|p| p.md5 == c.md5);
        let left: Element<'_, Message> = match (previewing, &d.art) {
            (Some(p), _) => p.view(left_w).map(Message::Preview),
            (None, Some(art)) => image(art.clone()).width(left_w).height(left_w).into(),
            (None, None) => container(center(text("No album art").size(13))).width(left_w).height(left_w).style(style::panel).into(),
        };

        let raw = |o: &Option<String>| o.clone().unwrap_or_default();
        let meta_line = |label: &'static str, v: String, link: bool| -> Element<'_, Message> {
            if strip_tags(&v).is_empty() {
                return space().into();
            }
            row![
                text(label).size(13).width(58).style(|t: &Theme| text::Style {
                    color: Some(style::muted(t)),
                }),
                if link { search_link(v, 13) } else { tagged(&v, 13, false) },
            ]
            .spacing(8)
            .into()
        };
        let is_sng = d.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("sng"));
        let open_path = if is_sng { d.path.parent().map(PathBuf::from).unwrap_or_default() } else { d.path.clone() };
        let buttons = row![
            button(text(if previewing.is_some() { "Stop preview" } else { "Preview" }).size(14))
                .padding([8, 16])
                .style(button::primary)
                .on_press(if previewing.is_some() { Message::Preview(preview_panel::Msg::Close) } else { Message::OpenLocalPreview }),
            button(text("Open folder").size(14)).padding([8, 16]).style(style::secondary).on_press(Message::OpenPath(open_path)),
        ]
        .spacing(8);
        let middle = scrollable(
            column![
                column![
                    container(rich::view(c.name.as_deref().unwrap_or(""), 19, BOLD, None)).clip(true),
                    search_link(raw(&c.artist), 15),
                ]
                .spacing(2),
                column![
                    meta_line("Album", raw(&c.album), true),
                    meta_line("Genre", raw(&c.genre), true),
                    meta_line("Charter", raw(&c.charter), true),
                    meta_line("Year", raw(&c.year), true),
                    meta_line("Length", c.length_str(), false),
                ]
                .spacing(4),
                // One instrument per line, so the Fretwork tier always has room.
                self.difficulty_rows(c, 0.0),
                chart_tags(c),
                buttons,
            ]
            .spacing(10)
            .padding(iced::Padding { right: 12.0, ..Default::default() }),
        )
        .width(330)
        .height(h);
        const GRAPH_H: f32 = 220.0;
        let board = scrollable(container(self.leaderboard_section(c)).padding(iced::Padding { right: 12.0, ..Default::default() }))
            .width(Length::Fill)
            .height(h - GRAPH_H - 34.0 - 12.0);
        let right = column![self.library_graph(c, GRAPH_H), board].spacing(12).width(Length::Fill);
        Some(row![container(left).width(left_w), middle, right].spacing(24).height(h).into())
    }

    /// Fretwork's difficulty graph of the open library chart, with an instrument picker.
    fn library_graph(&self, c: &Chart, height: f32) -> Element<'_, Message> {
        let muted = |t: &Theme| text::Style { color: Some(style::muted(t)) };
        let title = || text("Difficulty over the song").size(13).font(BOLD);
        let body = |e: Element<'static, Message>| -> Element<'_, Message> {
            column![row![title()].height(34).align_y(Alignment::Center), container(e).height(height).width(Length::Fill)].into()
        };
        let curves: Vec<(String, Arc<curves::Curves>)> = match self.hashes.get(&c.md5) {
            Some(leaderboard::Hashes::Done(Ok(cp))) => cp.curves.clone(),
            Some(leaderboard::Hashes::Done(Err(e))) => {
                return body(text(format!("Couldn't read this chart: {e}")).size(12).style(muted).into());
            }
            _ => return body(text("Reading the chart…").size(12).style(muted).into()),
        };
        if curves.is_empty() {
            return body(text("Fretwork doesn't graph this chart's instruments (it rates 5-fret and drums).").size(12).style(muted).into());
        }
        let pick = self
            .lib_graph_inst
            .as_ref()
            .and_then(|k| curves.iter().position(|(i, _)| i == k))
            .or_else(|| curves.iter().position(|(i, _)| i == "guitar"))
            .unwrap_or(0);
        let opts: Vec<GraphInst> = curves.iter().map(|(k, _)| GraphInst(k.clone())).collect();
        let picker = pick_list(opts, Some(GraphInst(curves[pick].0.clone())), |g: GraphInst| Message::LibraryGraphInstrument(g.0))
            .text_size(12)
            .padding([3, 8]);
        let graph = iced::widget::canvas(graph::Graph { curves: curves[pick].1.clone() }).width(Length::Fill).height(height);
        column![
            row![title(), space().width(Length::Fill), picker].height(34).align_y(Alignment::Center),
            graph,
        ]
        .into()
    }

    /// Where to read a chart's notes from: the library copy, then (RhythmVerse) a copy kept from
    /// an earlier preview, then the internet.
    pub fn chart_source(&self, c: &Chart) -> Option<leaderboard::Source> {
        // A library chart opened on the Library page: that exact copy.
        if let Some(p) = library::local_path(c) {
            return Some(leaderboard::Source::Local(p));
        }
        if let Some(p) = self.library_copy(c) {
            return Some(leaderboard::Source::Local(p));
        }
        #[cfg(feature = "rhythmverse")]
        if c.is_rv()
            && let Some(p) = crate::preview::rv_cache::any_cached(c)
        {
            return Some(leaderboard::Source::Local(p));
        }
        leaderboard::source(c)
    }

    /// Records a chart added to the library (`added`, replacing `removed`) or removed from it.
    fn apply_library_change(&mut self, (key, added, removed): (String, Option<PathBuf>, Vec<PathBuf>)) {
        if self.library_scanning {
            self.library_changes.push((key.clone(), added.clone(), removed.clone()));
        }
        let entry = self.library.entry(key.clone()).or_default();
        let before = entry.len();
        entry.retain(|p| !removed.contains(p));
        if let Some(p) = added
            && !entry.contains(&p)
        {
            entry.push(p);
        }
        let after = entry.len();
        if after == 0 {
            self.library.remove(&key);
        }
        if !removed.is_empty() {
            self.library_hashes.retain(|_, ps| {
                ps.retain(|p| !removed.contains(p));
                !ps.is_empty()
            });
            self.library_songs.retain(|_, ps| {
                ps.retain(|(_, p)| !removed.contains(p));
                !ps.is_empty()
            });
        }
        self.library_charts = (self.library_charts + after).saturating_sub(before);
        self.catalog_stale = true;
        self.list_rev += 1;
    }

    /// The library copies of a chart that may safely be removed.
    pub fn library_paths(&self, c: &Chart) -> Vec<PathBuf> {
        let roots = self.settings.scan_folders();
        self.library_matches(c).iter().filter(|p| library::is_deletable_chart(p, &roots)).cloned().collect()
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
            pack: None,
            title,
            status: DlStatus::Queued,
            handle: None,
        });
    }

    /// Adds a chart pack to the download queue (ignored if it's already queued or downloading).
    fn enqueue_pack(&mut self, job: packs::Job) {
        let key = format!("pack:{}", job.url);
        if self.downloads.iter().any(|d| d.md5 == key && matches!(d.status, DlStatus::Active { .. } | DlStatus::Queued)) {
            return;
        }
        let id = self.next_dl_id;
        self.next_dl_id += 1;
        self.downloads.push(DownloadEntry {
            id,
            md5: key,
            chart: Arc::new(Chart::default()),
            replace: Vec::new(),
            title: format!("{} (pack)", job.folder),
            pack: Some(job),
            status: DlStatus::Queued,
            handle: None,
        });
    }

    /// Each pack download's state, by link, for the Packs page.
    fn pack_states(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for d in &self.downloads {
            let Some(job) = &d.pack else { continue };
            let s = match &d.status {
                DlStatus::Queued => "Queued".to_string(),
                DlStatus::Active { status, done, total } => match total {
                    Some(t) if *t > 0 && status == "Downloading" => format!("Downloading {}%", done * 100 / t),
                    _ => status.clone(),
                },
                DlStatus::Done(_) => "Downloaded".to_string(),
                DlStatus::Failed(e) => format!("Failed: {e}"),
            };
            out.insert(job.url.clone(), s);
        }
        out
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
            let (task, handle) = match &d.pack {
                #[cfg(feature = "rhythmverse")]
                Some(job) => Task::run(packs::download(job.clone(), self.settings.clone()), move |p| Message::DlProgress(id, p)),
                #[cfg(not(feature = "rhythmverse"))]
                Some(_) => Task::done(Message::DlProgress(id, download::Progress::Finished(Err("This build of Truss can't download packs.".into())))),
                None => Task::run(download::start(d.chart.clone(), self.settings.clone(), d.replace.clone()), move |p| {
                    Message::DlProgress(id, p)
                }),
            }
            .abortable();
            d.status = DlStatus::Active { status: "Starting...".into(), done: 0, total: None };
            d.handle = Some(handle);
            tasks.push(task);
            active += 1;
        }
        Task::batch(tasks)
    }

    /// Searches with the Advanced filters. A filter that can't be read yet (half typed) keeps
    /// the current results; `show_error` (the Search button) says what's wrong with it.
    fn run_advanced(&mut self, show_error: bool) -> Task<Message> {
        match self.advanced.to_filters().and_then(|f| Ok((f, self.advanced.fretwork_filter()?))) {
            Ok((filters, fw)) => {
                // Nothing filled in and no Advanced search showing: nothing to change.
                if !show_error && self.advanced.active_count() == 0 && self.active_advanced.is_none() {
                    return Task::none();
                }
                self.advanced.error = None;
                self.fw.filter = fw;
                // An empty form means a normal search (which includes RhythmVerse).
                self.active_advanced = (self.advanced.active_count() > 0).then(|| Arc::new(filters));
                self.query.clear();
                self.search(false)
            }
            Err(e) => {
                if show_error {
                    self.advanced.error = Some(e);
                }
                Task::none()
            }
        }
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
        for m in &fresh {
            self.art.insert(m.clone(), Art::Loading);
        }
        // Look on disk first (a few at a time, nearest first); only misses wait to download.
        let px = self.thumb_px();
        let mut tasks: Vec<Task<Message>> = fresh
            .chunks(ART_CACHE_CHUNK)
            .map(|chunk| {
                let keys = chunk.to_vec();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            keys.into_iter().map(|k| {
                                let c = art_cache::read(&k, px);
                                (k, c)
                            }).collect::<Vec<_>>()
                        })
                        .await
                        .unwrap_or_default()
                    },
                    Message::ArtCached,
                )
            })
            .collect();
        tasks.push(self.pump_art());
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
            Message::Advanced(advanced::Msg::Search) => self.run_advanced(true),
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
                // Results follow the filters as they're filled in, like the main search box.
                self.generation += 1;
                let g = self.generation;
                Task::perform(tokio::time::sleep(Duration::from_millis(350)), move |_| Message::AdvancedDebounced(g))
            }
            Message::AdvancedDebounced(g) if g == self.generation => self.run_advanced(false),
            Message::AdvancedDebounced(_) => Task::none(),
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
            Message::CheckNewCharts => self.check_new_charts(),
            Message::NewChartsChecked(Ok(page)) => {
                // Charts already loaded, counting older versions and ones held back to mix
                // with RhythmVerse by date.
                let mut seen: std::collections::HashSet<&str> =
                    self.versions.values().flatten().map(|c| c.md5.as_str()).collect();
                #[cfg(feature = "rhythmverse")]
                seen.extend(self.rv.encore_buffer.iter().map(|c| c.md5.as_str()));
                let new = page.data.iter().take(20).any(|c| !seen.contains(c.md5.as_str()));
                if new && self.list_idle() { self.search(false) } else { Task::none() }
            }
            Message::NewChartsChecked(Err(_)) => Task::none(),
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
            Message::ArtCached(found) => {
                let mut shown = false;
                for (md5, c) in found {
                    // Skip covers whose rows scrolled away (or that loaded another way) meanwhile.
                    if !matches!(self.art.get(&md5), Some(Art::Loading)) || self.art_inflight.contains(&md5) {
                        continue;
                    }
                    match c {
                        art_cache::Cached::Thumb(t) => {
                            let thumb = image::Handle::from_rgba(t.width, t.height, t.rgba);
                            self.art.insert(md5, Art::Ready { thumb, full: None });
                            shown = true;
                        }
                        art_cache::Cached::Missing => {
                            self.art.insert(md5, Art::Failed);
                            shown = true;
                        }
                        art_cache::Cached::Miss => {
                            if !self.art_queue.contains(&md5) {
                                self.art_queue.push_back(md5);
                            }
                        }
                    }
                }
                let mut tasks = vec![self.pump_art()];
                if shown {
                    self.art_version += 1;
                    // The selected chart's big cover isn't cached; fetch it once its thumbnail is in.
                    let sel = self.selected.and_then(|i| self.results.get(i)).and_then(|c| c.album_art_md5.clone());
                    tasks.push(self.ensure_full_art(sel.as_ref()));
                }
                Task::batch(tasks)
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
                    Ok(()) => self.apply_library_change((key, None, removed)),
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
            Message::ToggleTextColors(b) => {
                self.settings.text_colors = b;
                rich::set_enabled(b);
                self.list_rev += 1; // redraw the cached result rows
                self.save_settings()
            }
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
                            let dest = match &d.status {
                                DlStatus::Done(p) => Some(p.clone()),
                                _ => None,
                            };
                            let (key, replace, title, is_pack) = (d.chart.library_key(), d.replace.clone(), d.title.clone(), d.pack.is_some());
                            if is_pack {
                                // A pack adds many charts: read the library again, then look for
                                // loose copies of its charts.
                                if let Some(d) = &dest {
                                    self.pack_dup_check.push(d.clone());
                                }
                                self.note_download(title, dest.clone());
                                if dest.is_some() {
                                    return Task::batch([self.scan_library(), self.pump_downloads()]);
                                }
                                return self.pump_downloads();
                            }
                            if let Some(dest) = &dest {
                                self.apply_library_change((key, Some(dest.clone()), replace));
                            }
                            self.note_download(title, dest);
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
                let local = self.library_copy(&chart);
                let (panel, task) = preview_panel::Panel::open(
                    chart,
                    self.instrument.key.map(String::from),
                    self.difficulty.key.map(String::from),
                    self.settings.preview_volume,
                    self.settings.download_videos,
                    local,
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
            Message::RetryAllFailed => {
                for d in self.downloads.iter_mut().filter(|d| matches!(d.status, DlStatus::Failed(_))) {
                    d.status = DlStatus::Queued;
                }
                self.notices.failed = 0;
                self.pump_downloads()
            }
            Message::DismissNotice(failed) => {
                if failed {
                    self.notices.failed = 0;
                } else {
                    self.notices.done = 0;
                }
                Task::none()
            }
            Message::NoticeTick => {
                let busy = self.downloads.iter().any(|d| matches!(d.status, DlStatus::Active { .. } | DlStatus::Queued));
                if !busy && self.notices.done_at.is_some_and(|t| t.elapsed().as_secs() >= NOTICE_SECS) {
                    self.notices.done = 0;
                }
                Task::none()
            }
            Message::ToggleDownloadNotices(b) => {
                self.settings.download_notices = b;
                if !b {
                    self.notices = Notices::default();
                }
                self.save_settings()
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
            Message::LibraryScan(library::ScanEvent::Found(n)) => {
                self.library_scan_found = n;
                Task::none()
            }
            Message::LibraryScan(library::ScanEvent::Cached(lib)) => {
                if !self.library_scanning || !self.catalog.is_empty() {
                    return Task::none();
                }
                let lib = Arc::unwrap_or_clone(lib);
                self.library_scan_expected = Some(lib.charts);
                self.library_charts = lib.charts;
                self.library = lib.keys;
                self.library_hashes = lib.hashes;
                self.library_songs = lib.songs;
                self.catalog = lib.catalog;
                // Downloads and deletions since start-up are replayed onto the cached library.
                // (Each is recorded again for the scan's result.)
                for change in std::mem::take(&mut self.library_changes) {
                    self.apply_library_change(change);
                }
                self.list_rev += 1;
                self.refresh_pages();
                Task::none()
            }
            Message::LibraryScan(library::ScanEvent::Done(lib)) => {
                self.library_scan_handle = None;
                self.library_scanning = false;
                self.catalog_stale = false;
                let lib = Arc::unwrap_or_clone(lib);
                self.library_charts = lib.charts;
                self.library = lib.keys;
                self.library_hashes = lib.hashes;
                self.library_songs = lib.songs;
                self.local_tracks.lock().unwrap_or_else(|e| e.into_inner()).clear();
                self.catalog = lib.catalog;
                self.list_rev += 1;
                // Downloads and deletions that finished during the scan may be missing from it.
                for (key, added, removed) in std::mem::take(&mut self.library_changes) {
                    if added.as_ref().is_some_and(|p| !p.exists()) || removed.iter().any(|p| p.exists() && Some(p) != added.as_ref()) {
                        continue; // undone since (or the scan already saw it)
                    }
                    let entry = self.library.get(&key);
                    let seen = added.as_ref().is_none_or(|p| entry.is_some_and(|e| e.contains(p)))
                        && removed.iter().all(|p| entry.is_none_or(|e| !e.contains(p)));
                    if !seen {
                        self.apply_library_change((key, added, removed));
                    }
                }
                self.refresh_pages();
                let dup_check = if self.pack_dup_check.is_empty() { Task::none() } else {
                    let packs = std::mem::take(&mut self.pack_dup_check);
                    self.find_pack_duplicates(Some(packs))
                };
                if matches!(self.screen, Screen::Library | Screen::Tools) || !self.lyrics_map.is_empty() {
                    return Task::batch([dup_check, self.scan_lyrics()]);
                }
                dup_check
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
                    library_ui::Out::Sorted(key) => {
                        self.settings.library_sort = key;
                        self.save_settings()
                    }
                    library_ui::Out::Rescan => {
                        self.lyrics_map.clear();
                        self.scan_library()
                    }
                    library_ui::Out::Open(p) => {
                        let _ = open::that_detached(p);
                        Task::none()
                    }
                    library_ui::Out::Expanded(c) => self.open_library_details(c),
                    library_ui::Out::FindPackDuplicates => {
                        self.library_busy = true;
                        self.library_notice = Some("Comparing your loose charts with your packs' copies…".into());
                        self.find_pack_duplicates(None)
                    }
                    library_ui::Out::RemoveVideos => {
                        self.library_busy = true;
                        self.library_notice = Some("Moving video backgrounds to the Recycle Bin…".into());
                        let files = library_tools::video_files(&self.catalog);
                        let n = files.len();
                        Task::perform(library::recycle(files), move |r| Message::VideosRemoved(r.map(|_| n)))
                    }
                    library_ui::Out::Delete(paths) => {
                        // Only charts inside the library folders can be removed (to the Recycle Bin).
                        let roots = self.settings.scan_folders();
                        let ok: Vec<PathBuf> = paths.into_iter().filter(|p| library::is_deletable_chart(p, &roots)).collect();
                        Task::perform(library::recycle(ok), Message::LibraryDeleted)
                    }
                };
                Task::batch([task, scroll])
            }
            Message::LibraryDetails(path, chart, art) => {
                let Some(d) = self.lib_details.as_mut().filter(|d| d.path == path) else { return Task::none() };
                d.chart = Some(chart);
                d.art = art;
                self.load_board()
            }
            Message::LibraryGraphInstrument(i) => {
                self.lib_graph_inst = Some(i);
                Task::none()
            }
            Message::OpenLocalPreview => {
                let Some(chart) = self.lib_details.as_ref().and_then(|d| d.chart.clone()) else { return Task::none() };
                if self.preview.as_ref().is_some_and(|p| p.md5 == chart.md5) {
                    return Task::none();
                }
                let save = self.close_preview();
                let local = library::local_path(&chart);
                let (panel, task) = preview_panel::Panel::open(
                    chart,
                    self.instrument.key.map(String::from),
                    self.difficulty.key.map(String::from),
                    self.settings.preview_volume,
                    self.settings.download_videos,
                    local,
                );
                self.preview = Some(panel);
                Task::batch([save, task.map(Message::Preview)])
            }
            Message::PackDuplicates(dups, pack) => {
                self.library_busy = false;
                if dups.is_empty() {
                    if pack.is_none() {
                        self.library_notice = Some("No loose charts match a pack's copy exactly.".into());
                    }
                    return Task::none();
                }
                self.library_notice = None;
                let keep = vec![true; dups.len()];
                self.merge_offer = Some(MergeOffer { dups, keep, pack, confirm: false });
                Task::none()
            }
            Message::MergeToggle(i) => {
                if let Some(o) = &mut self.merge_offer
                    && let Some(k) = o.keep.get_mut(i)
                {
                    *k = !*k;
                    o.confirm = false;
                }
                Task::none()
            }
            Message::MergeCancel => {
                self.merge_offer = None;
                Task::none()
            }
            Message::MergeConfirm => {
                let Some(o) = &mut self.merge_offer else { return Task::none() };
                if !o.confirm {
                    o.confirm = true;
                    return Task::none();
                }
                let o = self.merge_offer.take().unwrap();
                let roots = self.settings.scan_folders();
                let paths: Vec<PathBuf> = o
                    .dups
                    .iter()
                    .zip(&o.keep)
                    .filter(|(d, k)| **k && d.pack.exists() && library::is_deletable_chart(&d.loose, &roots))
                    .map(|(d, _)| d.loose.clone())
                    .collect();
                self.library_notice = Some(format!("Moved {} loose copies to the Recycle Bin; the pack copies stay.", paths.len()));
                Task::perform(library::recycle(paths), Message::LibraryDeleted)
            }
            Message::VideosRemoved(r) => {
                self.library_busy = false;
                match r {
                    Ok(n) => {
                        let sng = library_tools::video_counts(&self.catalog).1;
                        let mut s = format!("Moved {n} video files to the Recycle Bin.");
                        if sng > 0 {
                            s.push_str(&format!(" {sng} .sng charts keep theirs (the video is inside the file)."));
                        }
                        self.library_notice = Some(s);
                    }
                    Err(e) => self.library_error = Some(e),
                }
                self.scan_library()
            }
            Message::LibraryDeleted(r) => {
                self.library_error = r.err();
                self.scan_library()
            }
            Message::PacksPage(m) => {
                let (task, out) = self.packs_page.update(m);
                let task = task.map(Message::PacksPage);
                match out {
                    packs::Out::None => task,
                    packs::Out::Download(job) => {
                        self.enqueue_pack(job);
                        Task::batch([task, self.pump_downloads()])
                    }
                }
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
                    tools_ui::Out::SetVideoLag(v) => {
                        self.settings.chopt_video_lag_ms = v;
                        self.save_settings()
                    }
                    tools_ui::Out::Theme(o) => self.custom_theme_change(o),
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
            Message::PickPackFolder => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Choose where packs are downloaded")
                        .pick_folder()
                        .await
                        .map(|h| h.path().to_path_buf())
                },
                Message::PackFolderPicked,
            ),
            Message::PackFolderPicked(p) => {
                let Some(p) = p else { return Task::none() };
                self.set_pack_folder(Some(p))
            }
            Message::ClearPackFolder => self.set_pack_folder(None),
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
                // A library chart's preview only shows on the Library page.
                if s != Screen::Library && self.preview.as_ref().is_some_and(|p| p.md5.starts_with(library::LOCAL_MD5)) {
                    tasks.push(self.close_preview());
                }
                if s == Screen::Packs {
                    tasks.push(self.packs_page.opened().map(Message::PacksPage));
                }
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
                    self.art_queue.clear();
                    self.full_art_lru.clear();
                    self.art_version += 1;
                    tasks.push(self.refresh_art_window());
                    let sel = self.selected.and_then(|i| self.results.get(i)).and_then(|c| c.album_art_md5.clone());
                    tasks.push(self.load_art(sel.as_ref()));
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

    /// Looks for loose charts whose notes match a pack's copy exactly; with `only`, just copies
    /// of the charts in those (newly downloaded) pack folders.
    fn find_pack_duplicates(&self, only: Option<Vec<PathBuf>>) -> Task<Message> {
        let catalog = self.catalog.clone();
        let roots = self.settings.scan_folders();
        let label = only.as_ref().map(|ps| {
            ps.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ")
        });
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let mut d = library_tools::find_pack_duplicates(&catalog, &roots);
                    if let Some(only) = &only {
                        d.retain(|x| only.iter().any(|p| x.pack.starts_with(p)));
                    }
                    d
                })
                .await
                .unwrap_or_default()
            },
            move |d| Message::PackDuplicates(d, label.clone()),
        )
    }

    /// The list of loose copies to remove, shown above the Library or Packs page.
    fn view_merge_offer(&self) -> Option<Element<'_, Message>> {
        let o = self.merge_offer.as_ref()?;
        let muted = |t: &Theme| text::Style { color: Some(style::muted(t)) };
        let n = o.keep.iter().filter(|k| **k).count();
        let title = match &o.pack {
            Some(p) => format!("{} charts from {p} were already in your library outside the pack", o.dups.len()),
            None => format!("{} loose charts are also in a pack", o.dups.len()),
        };
        let rows = o.dups.iter().enumerate().fold(column![].spacing(4), |col, (i, d)| {
            let short = |p: &std::path::Path| {
                let parts: Vec<String> = p.components().rev().take(3).map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                parts.into_iter().rev().collect::<Vec<_>>().join("\\")
            };
            col.push(
                row![
                    button(text(if o.keep[i] { "☑" } else { "☐" }).size(14)).padding(0).style(button::text).on_press(Message::MergeToggle(i)),
                    column![
                        text(format!("{} - {} ({})", d.artist, d.name, d.charter)).size(13),
                        text(format!("Remove: …\\{}", short(&d.loose))).size(11).style(muted),
                        text(format!("Keep: …\\{}", short(&d.pack))).size(11).style(muted),
                    ]
                    .spacing(1),
                ]
                .spacing(8)
                .align_y(Alignment::Start),
            )
        });
        let confirm = button(
            text(if o.confirm { format!("Confirm: remove {n} loose copies") } else { format!("Remove {n} loose copies") }).size(13),
        )
        .padding([6, 12])
        .style(move |t: &Theme, s| if o.confirm { button::danger(t, s) } else { button::primary(t, s) })
        .on_press_maybe((n > 0).then_some(Message::MergeConfirm));
        let panel = column![
            text(title).size(15),
            text("Their notes match the pack's copies exactly, track for track. Only charts sitting straight in your songs folder are listed; nothing inside a pack or other folder is ever removed. Removed copies go to the Recycle Bin.")
                .size(12)
                .style(muted),
            container(scrollable(rows).height(Length::Shrink)).max_height(220),
            row![confirm, button(text("Keep both").size(13)).padding([6, 12]).style(style::secondary).on_press(Message::MergeCancel)]
                .spacing(8),
        ]
        .spacing(8);
        Some(
            container(panel)
                .padding(12)
                .width(Length::Fill)
                .style(|t: &Theme| container::Style {
                    background: Some(style::weakest(t).into()),
                    border: Border { color: style::accent(t), width: 1.0, radius: 6.0.into() },
                    ..Default::default()
                })
                .into(),
        )
    }

    fn view(&self) -> Element<'_, Message> {
        let body = match self.screen {
            Screen::Browse => self.view_browse(),
            Screen::Settings => self.view_settings(),
            Screen::Library => self.library_page.view(&self.library_ctx(), self.library_details_view().map(|d| (self.library_details_height(), d))),
            Screen::Tools => self.tools_page.view(&self.tools_ctx()).map(Message::ToolsPage),
            Screen::Packs => {
                let status = self.pack_states();
                let downloads: Vec<packs::DlRow> = self
                    .downloads
                    .iter()
                    .filter_map(|d| {
                        let job = d.pack.as_ref()?;
                        let state = match &d.status {
                            DlStatus::Queued => packs::DlState::Queued,
                            DlStatus::Active { status, done, total } => {
                                packs::DlState::Active { status: status.clone(), done: *done, total: *total }
                            }
                            DlStatus::Done(_) => packs::DlState::Done,
                            DlStatus::Failed(_) => packs::DlState::Failed,
                        };
                        Some(packs::DlRow { page: job.page.clone(), state })
                    })
                    .collect();
                let pack_folder = self.settings.pack_folder();
                let lib = &self.library;
                let in_library = |k: &str| lib.get(k).is_some_and(|p| !p.is_empty());
                let cx = packs::Ctx { status: &status, downloads: &downloads, library: pack_folder.as_deref(), in_library: &in_library };
                // The page borrows `status` only while it's built.
                let page: Element<'_, packs::Msg> = self.packs_page.view(&cx);
                page.map(Message::PacksPage)
            }
        };
        let body: Element<'_, Message> = match self.view_merge_offer().filter(|_| matches!(self.screen, Screen::Library | Screen::Packs)) {
            Some(offer) => column![container(offer).padding([10, 14]), body].into(),
            None => body,
        };
        let mut layout = column![self.view_topbar()];
        if self.advanced_open && self.screen == Screen::Browse {
            layout = layout.push(self.advanced.view().map(Message::Advanced));
        }
        let base = layout.push(body);
        match self.view_corner() {
            Some(corner) => iced::widget::stack![base, corner].into(),
            None => base.into(),
        }
    }

    fn view_topbar(&self) -> Element<'_, Message> {
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
            nav("Packs", Screen::Packs),
            nav("Library", Screen::Library),
            nav("Tools", Screen::Tools),
            nav("Settings", Screen::Settings),
            space().width(14),
            search,
        ]
        .push(self.update_button())
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
        let sidebar = container(self.view_sidebar()).width(if expanded { 720.0_f32.min(self.win.width * 0.6) } else { self.sidebar_width() }).height(Length::Fill).padding([18, 20]);
        row![self.lazy_results(), style::divider(true), sidebar].height(Length::Fill).into()
    }

    /// Default layout's results list (header + rows), built with owned data for `lazy`.
    fn list_results(&self) -> Element<'static, Message> {
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

    fn view_row(&self, i: usize, c: &Chart) -> Element<'static, Message> {
        let selected = self.selected == Some(i);
        let checked = self.checked.contains(&i);
        let owned = self.in_library(c);
        let cell = |v: &Option<String>, w: Length, quiet: bool| {
            container(tagged(v.as_deref().unwrap_or(""), if quiet { 13 } else { 14 }, quiet)).width(w).clip(true)
        };
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
            .push((!owned && self.other_version_in_library(c)).then(other_version_mark))
            .push(c.is_rv().then(rv_tag))
            .push(cell(&c.name, Length::Fill, false))
            .spacing(6)
            .align_y(Alignment::Center)
            .width(Length::FillPortion(4))
            .into();
        let name = with_progress(name, self.dl_progress(&c.md5), 4);
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
                cell(&c.artist, Length::FillPortion(3), false),
            ]
            .push((self.list_cols() < 1).then(|| cell(&c.album, Length::FillPortion(3), true)))
            .push((self.list_cols() < 2).then(|| cell(&c.charter, Length::FillPortion(2), true)))
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
        checkbox(self.checked.contains(&i)).size(14).on_toggle(move |_| Message::ToggleChecked(i)).into()
    }

    /// The header tick box: ticks every loaded result, or clears them all.
    pub fn check_all_box(&self) -> Element<'static, Message> {
        // "All" means every chart you don't already have (those can't be downloaded again from here).
        let wanted: Vec<usize> = self.results.iter().enumerate().filter(|(_, c)| !self.in_library(c)).map(|(i, _)| i).collect();
        let all = !wanted.is_empty() && wanted.iter().all(|i| self.checked.contains(i));
        checkbox(all).size(14).on_toggle(Message::CheckAll).into()
    }

    /// A chart's difficulties (with Fretwork tiers), in two columns when `inner` px allows.
    fn difficulty_rows(&self, c: &Chart, inner: f32) -> Element<'_, Message> {
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
                let pips = (0..6).fold(row![].spacing(2), |r, n| {
                    let on = n < d.min(6);
                    r.push(container(space()).width(if two_cols { pip_w } else { 12.0 }).height(5).style(
                        move |t: &Theme| container::Style {
                            background: Some(if on { t.extended_palette().primary.base.color } else { style::hairline(t) }.into()),
                            border: Border::default().rounded(1),
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
        diffs.into()
    }

    fn view_sidebar(&self) -> Element<'_, Message> {
        let Some((i, c)) = self.selected.and_then(|i| self.results.get(i).map(|c| (i, c))) else {
            return center(text("Select a chart to see details").size(14)).into();
        };

        // Album art fills the sidebar width at most, and shrinks in short windows.
        let inner = self.sidebar_width() - 40.0 - 16.0;
        let art_size = inner.min((self.win.height * 0.38).max(120.0));
        let icon_size = (art_size * 0.2).clamp(36.0, 64.0);
        let sized_art = |h: iced::widget::image::Handle| -> Element<'static, Message> {
            let art = self.with_icon(image(h).width(art_size).height(art_size).into(), c, icon_size, icon_size * 0.19);
            container(art).center_x(Length::Fill).into()
        };
        let art: Element<_> = match (&self.preview, c.album_art_md5.as_ref().and_then(|m| self.art.get(m))) {
            // The preview replaces the album art while it's open.
            (Some(p), _) if p.md5 == c.md5 => {
                p.view(if p.expanded() { 500.0 } else { 296.0 }).map(Message::Preview)
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

        // Raw values keep their colour tags; the widgets below draw them.
        let raw = |o: &Option<String>| o.clone().unwrap_or_default();
        let meta_line = |label: &'static str, v: String, link: bool| -> Element<'_, Message> {
            if strip_tags(&v).is_empty() {
                space().into()
            } else {
                row![
                    text(label).size(13).width(58).style(|t: &Theme| text::Style {
                        color: Some(style::muted(t)),
                    }),
                    if link { search_link(v, 13) } else { tagged(&v, 13, false) },
                ]
                    .spacing(8)
                    .into()
            }
        };

        let diffs = self.difficulty_rows(c, inner);
        let tag_row = chart_tags(c);

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
                button(text("Waiting to download…").size(15).width(Length::Fill).center()).width(Length::Fill).padding(10).into()
            }
            Some(DlStatus::Active { .. }) => {
                let p = self.dl_progress(&c.md5).unwrap_or(0.0);
                column![
                    text(format!("Downloading… {:.0}%", p * 100.0)).size(14).width(Length::Fill).center(),
                    progress_bar(0.0..=1.0, p).girth(6),
                ]
                .spacing(6)
                .width(Length::Fill)
                .into()
            }
            _ if self.in_library(c) => self.library_buttons(i, c, 15, 10),
            _ => button(text("Download").size(15).width(Length::Fill).center())
                .width(Length::Fill)
                .padding(10)
                .style(button::primary)
                .on_press(Message::Download(i))
                .into(),
        } };
        let dl_button: Element<_> = if dl_state.is_none() && self.other_version_in_library(c) {
            column![
                text("You have another version of this chart. Downloading adds this one beside it.")
                    .size(12)
                    .style(|t: &Theme| text::Style { color: Some(t.extended_palette().warning.base.color) }),
                dl_button,
            ]
            .spacing(6)
            .width(Length::Fill)
            .into()
        } else {
            dl_button
        };
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
                    container(rich::view(c.name.as_deref().unwrap_or(""), 19, Font { weight: iced::font::Weight::Semibold, ..Font::DEFAULT }, None)).clip(true),
                    search_link(raw(&c.artist), 15),
                ]
                .spacing(2),
                column![
                    meta_line("Album", raw(&c.album), true),
                    meta_line("Genre", raw(&c.genre), true),
                    meta_line("Charter", raw(&c.charter), true),
                    row![
                        container(meta_line("Year", raw(&c.year), true)).width(Length::Fill),
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

    /// Sets where packs are saved. A folder outside the library is scanned too, so the charts
    /// in its packs are marked as downloaded.
    fn set_pack_folder(&mut self, p: Option<PathBuf>) -> Task<Message> {
        let before = self.settings.scan_folders();
        self.settings.pack_path = p;
        if self.settings.scan_folders() != before { Task::batch([self.save_settings(), self.scan_library()]) } else { self.save_settings() }
    }

    /// Settings > Downloads: where chart packs (Packs page) are saved.
    fn pack_folder_setting(&self) -> Element<'static, Message> {
        let (path, note) = match (&self.settings.pack_path, &self.settings.library_path) {
            (Some(p), _) => (p.display().to_string(), None),
            (None, Some(l)) => (l.display().to_string(), Some("Same as your songs folder")),
            (None, None) => ("Not set".to_string(), Some("Same as your songs folder, once you choose one")),
        };
        let mut info = column![text("Pack downloads folder").size(13), text(path).size(12).wrapping(text::Wrapping::WordOrGlyph)].spacing(2);
        if let Some(n) = note {
            info = info.push(text(n).size(11).style(|t: &Theme| text::Style { color: Some(style::muted(t)) }));
        }
        let mut r = row![info.width(Length::Fill), button(text("Choose…").size(12)).padding([4, 10]).style(crate::style::secondary).on_press(Message::PickPackFolder)]
            .spacing(10)
            .align_y(Alignment::Center);
        if self.settings.pack_path.is_some() {
            r = r.push(button(text("Use songs folder").size(12)).padding([4, 10]).style(crate::style::subtle).on_press(Message::ClearPackFolder));
        }
        r.into()
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
            format!("Scanning your library… {} charts found so far.", self.library_scan_found)
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
        lib_col = lib_col.push(rule::horizontal(1).style(crate::style::rule)).push(self.pack_folder_setting());

        let section = |title: &'static str, content: Element<'static, Message>| {
            container(column![text(title).font(BOLD).size(15), content].spacing(10))
                .padding(16)
                .width(Length::Fill)
                .style(crate::style::panel)
        };

        let themes = all_themes(&self.settings.custom_themes);
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
                    toggler(self.settings.download_notices)
                        .label("Show a notice when downloads finish or fail (a batch shares one notice)")
                        .on_toggle(Message::ToggleDownloadNotices),
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
                    toggler(self.settings.show_fretwork)
                        .label("Show Fretwork difficulty tiers next to difficulties (otherwise hover a difficulty to see them)")
                        .on_toggle(Message::ToggleFretwork),
                    toggler(self.settings.text_colors)
                        .label("Show coloured song names, artists and charters (as charters styled them for Clone Hero)")
                        .on_toggle(Message::ToggleTextColors),
                    row![
                        text("Transparency (beta)").size(13),
                        pick_list(
                            backdrop::Backdrop::ALL,
                            Some(backdrop::Backdrop::from_key(&self.settings.window_backdrop)),
                            Message::BackdropPicked,
                        ),
                        text(if self.settings.cpu_rendering {
                            "Needs low memory mode off. Applies after restart."
                        } else if active_backdrop(&self.settings) != self.backdrop {
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
                            iced::widget::slider(0.1..=1.0, self.settings.glass_transparency, Message::GlassTransparency)
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
                        let note = theme_transparency(&self.settings).is_some().then(|| {
                            text(format!("Your theme \u{201c}{}\u{201d} sets its own transparency, so these don't apply while you use it.", self.settings.theme))
                                .size(12)
                                .style(|t: &Theme| text::Style { color: Some(style::muted(t)) })
                        });
                        column![amount].push(note).spacing(8)
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

    /// How far a chart's download has got (0 while queued), if it's queued or downloading.
    pub fn dl_progress(&self, md5: &str) -> Option<f32> {
        self.downloads.iter().rev().find(|d| d.md5 == md5).and_then(|d| match &d.status {
            DlStatus::Queued => Some(0.0),
            DlStatus::Active { done, total, .. } => Some(total.map(|t| *done as f32 / t.max(1) as f32).unwrap_or(0.0).clamp(0.0, 1.0)),
            _ => None,
        })
    }

    /// What the results rows show of the downloads (for the list's redraw key): each queued or
    /// running chart and its progress in whole percent.
    pub fn dl_rows_key(&self) -> Vec<(String, u8)> {
        self.downloads
            .iter()
            .filter_map(|d| self.dl_progress(&d.md5).map(|p| (d.md5.clone(), (p * 100.0) as u8)))
            .collect()
    }

    /// Records a finished download for the corner notices.
    fn note_download(&mut self, title: String, dest: Option<PathBuf>) {
        if !self.settings.download_notices {
            return;
        }
        match dest {
            Some(p) => {
                self.notices.done += 1;
                self.notices.last_title = title;
                self.notices.last_path = Some(p);
                self.notices.done_at = Some(std::time::Instant::now());
            }
            None => self.notices.failed += 1,
        }
    }

    /// "Update to vX" in the top bar while an update is waiting.
    fn update_button(&self) -> Option<Element<'_, Message>> {
        match &self.update {
            UpdateState::Available(rel) => Some(
                button(text(format!("Update to {}", rel.version)).size(12))
                    .padding([5, 10])
                    .style(button::primary)
                    .on_press(Message::InstallUpdate)
                    .into(),
            ),
            UpdateState::Installing(_, p) => Some(button(text(format!("Updating… {:.0}%", p * 100.0)).size(12)).padding([5, 10]).into()),
            UpdateState::Ready(_) => Some(
                button(text("Restart to update").size(12)).padding([5, 10]).style(button::primary).on_press(Message::RestartForUpdate).into(),
            ),
            _ => None,
        }
    }

    /// The bar below the results while several are ticked: "Download N".
    pub fn selection_bar(&self) -> Option<Element<'_, Message>> {
        if self.checked.is_empty() {
            return None;
        }
        let n = self.checked.len();
        let charts = if n == 1 { "1 chart".to_string() } else { format!("{n} charts") };
        // Both buttons the same size and shape; Download in the accent colour.
        let download = button(text(format!("Download {charts}")).size(13))
            .padding([6, 16])
            .style(|t: &Theme, s| {
                let hover = matches!(s, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(if hover { style::accent_tint(t, 0.85) } else { style::accent(t) }.into()),
                    text_color: Color::WHITE,
                    border: Border::default().rounded(6),
                    ..Default::default()
                }
            })
            .on_press(Message::DownloadChecked);
        let bar = container(
            row![
                text(format!("{charts} selected")).size(13),
                space().width(Length::Fill),
                button(text("Clear").size(13)).padding([6, 14]).style(style::quiet_button(false)).on_press(Message::ClearChecked),
                download,
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([8, 16])
        .width(Length::Fill);
        Some(column![style::divider(false), bar].into())
    }

    /// The bottom-right corner: the downloads button, its list, and the download notices.
    fn view_corner(&self) -> Option<Element<'_, Message>> {
        let pill = self.downloads_pill();
        let notices = self.notice_cards();
        if pill.is_none() && notices.is_empty() {
            return None;
        }
        let open = self.show_downloads && !self.downloads.is_empty();
        let mut col = column![].spacing(8).align_x(Alignment::End);
        if open {
            col = col.push(self.downloads_popover());
        }
        for n in notices {
            col = col.push(n);
        }
        if let Some(p) = pill {
            col = col.push(p);
        }
        let corner = container(col).width(Length::Fill).height(Length::Fill).align_right(Length::Fill).align_bottom(Length::Fill).padding(16);
        if open {
            // A click anywhere else closes the list.
            let away = iced::widget::mouse_area(container(space()).width(Length::Fill).height(Length::Fill)).on_press(Message::ToggleDownloads);
            return Some(iced::widget::stack![away, corner].into());
        }
        Some(corner.into())
    }

    /// The floating button: overall download progress (or the library scan).
    fn downloads_pill(&self) -> Option<Element<'_, Message>> {
        let total = self.downloads.len();
        let active = self.downloads.iter().filter(|d| matches!(d.status, DlStatus::Active { .. } | DlStatus::Queued)).count();
        let failed = self.downloads.iter().filter(|d| matches!(d.status, DlStatus::Failed(_))).count();
        let finished = self.downloads.iter().filter(|d| matches!(d.status, DlStatus::Done(_))).count();
        let (label, progress): (String, Option<f32>) = if active > 0 {
            // Each chart is one step; running ones count by their bytes so far.
            let running: f32 = self
                .downloads
                .iter()
                .filter(|d| matches!(d.status, DlStatus::Active { .. }))
                .filter_map(|d| self.dl_progress(&d.md5))
                .sum();
            let done = total - active;
            let p = ((done as f32 + running) / total.max(1) as f32).min(1.0);
            (format!("↓  {done} / {total}  ·  {:.0}%", p * 100.0), Some(p))
        } else if total > 0 {
            let mut s = format!("✓  {finished} downloaded");
            if failed > 0 {
                s.push_str(&format!("  ·  {failed} failed"));
            }
            (s, None)
        } else if self.library_scanning {
            let found = self.library_scan_found;
            match self.library_scan_expected {
                Some(n) => (
                    format!("Scanning library… {} / {}", thousands(found), thousands(n)),
                    Some((found as f32 / n.max(1) as f32).min(1.0)),
                ),
                None => (format!("Scanning library… {} charts", thousands(found)), None),
            }
        } else {
            return None;
        };
        let scanning_only = total == 0;
        let mut content = column![text(label).size(13).wrapping(text::Wrapping::None)].spacing(5);
        if let Some(p) = progress {
            content = content.push(progress_bar(0.0..=1.0, p).length(180).girth(4));
        }
        let pill = button(content)
            .padding([8, 14])
            .style(move |t: &Theme, s| {
                let hovered = matches!(s, button::Status::Hovered) && !scanning_only;
                button::Style {
                    background: Some(if hovered { style::accent_tint(t, 0.25) } else { style::raised(t) }.into()),
                    text_color: style::fg(t),
                    border: Border { color: style::hairline(t), width: 1.0, radius: 18.0.into() },
                    shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.45), offset: iced::Vector::new(0.0, 4.0), blur_radius: 14.0 },
                    ..Default::default()
                }
            })
            .on_press_maybe((!scanning_only).then_some(Message::ToggleDownloads));
        Some(pill.into())
    }

    /// The downloads list, opened from the floating button.
    fn downloads_popover(&self) -> Element<'_, Message> {
        let active = self.downloads.iter().filter(|d| matches!(d.status, DlStatus::Active { .. })).count();
        let queued = self.downloads.iter().filter(|d| matches!(d.status, DlStatus::Queued)).count();
        let failed = self.downloads.iter().filter(|d| matches!(d.status, DlStatus::Failed(_))).count();
        let finished = self.downloads.iter().filter(|d| matches!(d.status, DlStatus::Done(_))).count();
        let mut summary = if active + queued > 0 { format!("{active} downloading") } else { format!("{finished} finished") };
        if queued > 0 {
            summary.push_str(&format!(", {queued} waiting"));
        }
        if failed > 0 {
            summary.push_str(&format!(", {failed} failed"));
        }
        let header = row![
            column![
                text("Downloads").size(15).font(BOLD),
                text(summary).size(12).style(|t: &Theme| text::Style { color: Some(style::muted(t)) }),
            ]
            .spacing(2),
            space().width(Length::Fill),
        ]
        .push((failed > 0).then(|| button(text("Retry failed").size(12)).style(crate::style::secondary).padding([4, 10]).on_press(Message::RetryAllFailed)))
        .push(button(text("Clear finished").size(12)).style(crate::style::subtle).padding([4, 10]).on_press(Message::ClearFinished))
        .push(button(text("✕").size(12)).style(crate::style::subtle).padding([4, 8]).on_press(Message::ToggleDownloads))
        .spacing(8)
        .align_y(Alignment::Center);
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
        let list = scrollable(column(items).spacing(10).padding(iced::Padding { right: 12.0, ..Default::default() }));
        container(column![header, rule::horizontal(1).style(crate::style::rule), container(list).max_height(340)].spacing(10))
            .padding(14)
            .width(420)
            .style(|t: &Theme| container::Style {
                background: Some(style::raised(t).into()),
                text_color: Some(style::fg(t)),
                border: Border { color: style::hairline(t), width: 1.0, radius: 12.0.into() },
                shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.5), offset: iced::Vector::new(0.0, 6.0), blur_radius: 24.0 },
                ..Default::default()
            })
            .into()
    }

    /// The grouped "downloaded" and "failed" notices.
    fn notice_cards(&self) -> Vec<Element<'_, Message>> {
        let card = |body: Element<'static, Message>, actions: Vec<Element<'static, Message>>, failed: bool| -> Element<'static, Message> {
            let mut r = row![body].spacing(8).align_y(Alignment::Center);
            for a in actions {
                r = r.push(a);
            }
            r = r.push(button(text("✕").size(11)).style(crate::style::subtle).padding([3, 7]).on_press(Message::DismissNotice(failed)));
            container(r)
                .padding([8, 12])
                .max_width(460)
                .style(move |t: &Theme| container::Style {
                    background: Some(style::raised(t).into()),
                    text_color: Some(style::fg(t)),
                    border: Border {
                        color: if failed { t.extended_palette().danger.base.color } else { style::hairline(t) },
                        width: 1.0,
                        radius: 10.0.into(),
                    },
                    shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.4), offset: iced::Vector::new(0.0, 4.0), blur_radius: 12.0 },
                    ..Default::default()
                })
                .into()
        };
        let mut out = Vec::new();
        let n = &self.notices;
        if n.failed > 0 {
            let msg = if n.failed == 1 { "1 download failed".to_string() } else { format!("{} downloads failed", n.failed) };
            out.push(card(
                text(msg).size(13).style(text::danger).into(),
                vec![
                    button(text("Retry").size(12)).style(crate::style::secondary).padding([3, 10]).on_press(Message::RetryAllFailed).into(),
                    button(text("Show").size(12)).style(crate::style::subtle).padding([3, 10]).on_press(Message::ToggleDownloads).into(),
                ],
                true,
            ));
        }
        if n.done > 0 {
            let waiting = self.downloads.iter().filter(|d| matches!(d.status, DlStatus::Active { .. } | DlStatus::Queued)).count();
            let mut msg = if n.done == 1 { format!("Downloaded {}", n.last_title) } else { format!("Downloaded {} charts", n.done) };
            if waiting > 0 {
                msg.push_str(&format!("  ·  {waiting} to go"));
            }
            let action: Element<'static, Message> = match (&n.last_path, n.done) {
                (Some(p), 1) => {
                    button(text("Open folder").size(12)).style(crate::style::subtle).padding([3, 10]).on_press(Message::OpenPath(p.clone())).into()
                }
                _ => button(text("Show").size(12)).style(crate::style::subtle).padding([3, 10]).on_press(Message::ToggleDownloads).into(),
            };
            out.push(card(text(msg).size(13).wrapping(text::Wrapping::None).into(), vec![action], false));
        }
        out
    }
}

/// A results cell with a thin download bar under it while the chart is queued or downloading.
fn with_progress(cell: Element<'static, Message>, progress: Option<f32>, portion: u16) -> Element<'static, Message> {
    match progress {
        Some(p) => column![cell, progress_bar(0.0..=1.0, p).girth(3)].spacing(3).width(Length::FillPortion(portion)).into(),
        None => cell,
    }
}

/// Tags for what a chart has (solos, lyrics, video…).
fn chart_tags<'a>(c: &Chart) -> Element<'a, Message> {
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
            style::outline_tag(th)
        }))
    });
    tag_row.into()
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

/// Loose copies of pack charts, waiting for the user to remove them or keep them.
struct MergeOffer {
    dups: Vec<library_tools::Duplicate>,
    /// Which to remove (all at first).
    keep: Vec<bool>,
    /// The pack just downloaded, when the check followed a download.
    pack: Option<String>,
    /// The remove button was clicked once and waits for the confirming click.
    confirm: bool,
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
        ("tiny-skia", "https://github.com/linebender/tiny-skia"),
        ("cosmic-text", "https://github.com/pop-os/cosmic-text"),
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
        heading("Chart Sources"),
        entry(
            "Chorus Encore",
            "https://www.enchor.us",
            "by Geomitron. Browse's search, chart downloads, previews, chart details and the files Truss checks charts against.",
        ),
        entry("RhythmVerse", "https://rhythmverse.co", "Extra search results (marked RV) and their downloads."),
        entry(
            "Custom Songs Central",
            "https://customsongscentral.com",
            "The Packs tab: the list of packs, their pages and their download links.",
        ),
        text("Every chart and pack is the work of its charters and pack makers. Truss only finds and downloads them; please support the sites that host them.")
            .size(12)
            .style(muted),
        heading("Ported from"),
        entry("Bridge", "https://github.com/Geomitron/Bridge", "by Geomitron (GPL-3.0). Truss is a rewrite of Bridge; its search, downloads and folder naming follow it."),
        entry("scan-chart", "https://github.com/Geomitron/scan-chart", "by Geomitron (MIT). Chart reading, the issue checks, and the track hashes leaderboards use."),
        entry("chart-preview", "https://github.com/Geomitron/chart-preview", "by Geomitron (MIT). The note highway preview."),
        entry("Fretwork", "https://github.com/Staycation44/fretwork", "by Staycation (MIT). The Fretwork difficulty tiers."),
        entry("CHOpt", "https://github.com/GenericMadScientist/CHOpt", "by Raymond Wright (GPL-3.0). The Star Power path optimiser and path images."),
        entry("SngFileFormat", "https://github.com/mdsitton/SngFileFormat", "by mdsitton. The .sng file format."),
        heading("Related"),
        entry("CH Scene Switcher", "https://github.com/graindead/chswitcher", "Shares its settings with the Scene Switcher tool."),
        heading("Built with"),
        libs,
        text("Scores come from the Clone Hero leaderboards; song requests from Ouvert.")
            .size(12)
            .style(muted),
    ]
    .spacing(8)
    .into()
}

/// Text in the details that searches for itself when clicked (artist, charter, genre…).
/// Text that may carry colour tags: drawn styled when it has any, else plain (`quiet`: muted).
fn tagged<'a>(v: &str, size: u32, quiet: bool) -> Element<'a, Message> {
    if rich::is_styled(v) {
        return rich::view(v, size, Font::DEFAULT, None);
    }
    let t = text(strip_tags(v)).size(size).wrapping(text::Wrapping::None);
    if quiet { t.style(|th: &Theme| text::Style { color: Some(style::muted(th)) }).into() } else { t.into() }
}

/// A value that searches for itself when clicked. `v` may carry colour tags (searched without).
fn search_link<'a>(v: String, size: u32) -> Element<'a, Message> {
    let content: Element<'a, Message> =
        if rich::is_styled(&v) { rich::view(&v, size, Font::DEFAULT, None) } else { text(strip_tags(&v)).size(size).into() };
    let v = strip_tags(&v);
    button(content)
        .padding(0)
        .style(move |t: &Theme, st| button::Style {
            background: None,
            text_color: if matches!(st, button::Status::Hovered | button::Status::Pressed) {
                style::accent(t)
            } else if size > 13 {
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

/// 30987 -> "30,987".
fn thousands(n: usize) -> String {
    let d = n.to_string();
    let mut out = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
