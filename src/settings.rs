use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Folder that downloaded charts are saved into (the default library folder).
    pub library_path: Option<PathBuf>,
    /// More song folders that are checked for charts you already have.
    pub extra_library_folders: Vec<PathBuf>,
    /// Where chart packs (Packs page) are saved; the default songs folder when not set.
    pub pack_path: Option<PathBuf>,
    /// Drum type filter (API key, e.g. "fourLanePro"); only used when searching drums.
    pub drum_type: Option<String>,
    /// Only show drum charts that passed Encore's quality review.
    pub drums_reviewed: bool,
    /// Chart sources (only the RhythmVerse build shows these switches).
    pub source_encore: bool,
    pub source_rhythmverse: bool,
    /// Tools > Issue scanner: folder to scan, and where the spreadsheet goes.
    pub issue_scan_path: Option<PathBuf>,
    pub spreadsheet_output_path: Option<PathBuf>,
    /// Relative path template for each chart. Supports {name} {artist} {album} {genre} {year} {charter}.
    pub chart_folder_name: String,
    /// Save the raw .sng file instead of extracting it into a folder.
    pub is_sng: bool,
    /// Download background videos when a chart has one.
    pub download_videos: bool,
    /// Corner notices when downloads finish or fail.
    pub download_notices: bool,
    /// Default instrument filter (API key, e.g. "guitar").
    pub instrument: Option<String>,
    /// Default difficulty filter (API key, e.g. "expert").
    pub difficulty: Option<String>,
    /// Name of the iced theme.
    pub theme: String,
    /// Themes made in Tools > Themes (or imported).
    pub custom_themes: Vec<crate::theme_maker::CustomTheme>,
    /// Chart preview volume, 0..=1.
    pub preview_volume: f32,
    /// Browse layout: "list", "grid", "compact" or "hero".
    pub layout: String,
    /// Chart preview frame-rate cap; 0 = match the display's refresh rate.
    pub preview_fps: u32,
    /// Note speed and highway length for previews in Clone Hero's layout (as its settings);
    /// none: the player's own, from their Clone Hero profile.
    pub preview_note_speed: Option<f32>,
    pub preview_highway_length: Option<f32>,
    /// The colour profile the chart preview uses (by name, in Clone Hero's Colors folder);
    /// none: the player's own.
    pub preview_color_profile: Option<String>,
    /// Clone Hero's folder, if picked (otherwise it's looked for).
    pub ch_folder: Option<PathBuf>,
    /// The version of the folder setup screen last seen (it's shown again when there's
    /// something new on it).
    pub setup_seen: u32,
    /// Look for a new version on GitHub when Truss starts.
    pub check_updates: bool,
    /// The Leaderboard section in the details is open.
    pub show_leaderboard: bool,
    /// The Library page's sort order (see `library_ui::SortBy::key`); empty for the default.
    pub library_sort: String,
    /// Show Fretwork tiers beside the difficulties (they're always on hover).
    pub show_fretwork: bool,
    /// Draw the colours and bold/italic tags in song names, artists and charters.
    pub text_colors: bool,
    /// Window transparency (beta): "off", "mica", "acrylic" or "clear". Applies on start.
    pub window_backdrop: String,
    /// How see-through the window is with frosted glass on, 0.1 (barely) to 0.8 (very).
    pub glass_transparency: f32,
    /// Tools > Star Power paths: the player's video calibration (ms), kept between runs.
    pub chopt_video_lag_ms: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            library_path: None,
            extra_library_folders: Vec::new(),
            pack_path: None,
            drum_type: None,
            drums_reviewed: false,
            source_encore: true,
            source_rhythmverse: true,
            issue_scan_path: None,
            spreadsheet_output_path: None,
            chart_folder_name: "{artist} - {name} ({charter})".into(),
            is_sng: false,
            download_videos: false,
            instrument: None,
            difficulty: None,
            theme: "Rust".into(),
            preview_volume: 0.5,
            layout: "list".into(),
            preview_fps: 60,
            preview_note_speed: None,
            preview_highway_length: None,
            preview_color_profile: None,
            ch_folder: None,
            setup_seen: 0,
            check_updates: true,
            show_leaderboard: false,
            library_sort: String::new(),
            show_fretwork: true,
            text_colors: true,
            download_notices: true,
            // Frosted glass needs Windows' blur; elsewhere it would only be see-through.
            window_backdrop: if cfg!(windows) { "frosted" } else { "off" }.into(),
            glass_transparency: 0.5,
            chopt_video_lag_ms: 0,
            custom_themes: Vec::new(),
        }
    }
}

/// The app's settings, cache and data folders (%APPDATA%\truss, %LOCALAPPDATA%\truss).
pub fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("", "", "truss")
}

/// Truss's cache folder. TRUSS_CACHE_DIR gives a second copy (for testing, say) its own, the
/// way TRUSS_CONFIG_DIR gives it its own settings.
/// Truss's own folder for short-lived files (in its cache, which only this user can reach,
/// unlike the system's shared temp folder).
pub fn temp_dir() -> PathBuf {
    match cache_dir() {
        Some(d) => d.join("temp"),
        // nosemgrep: only when there's no user folder to put Truss's cache in
        None => std::env::temp_dir().join("truss"),
    }
}

pub fn cache_dir() -> Option<PathBuf> {
    match std::env::var_os("TRUSS_CACHE_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None => project_dirs().map(|d| d.cache_dir().to_path_buf()),
    }
}

/// Truss's data folder (repair history); under TRUSS_CACHE_DIR when that's set.
pub fn data_dir() -> Option<PathBuf> {
    match std::env::var_os("TRUSS_CACHE_DIR") {
        Some(d) => Some(PathBuf::from(d).join("data")),
        None => project_dirs().map(|d| d.data_dir().to_path_buf()),
    }
}

/// The app used to be called "Bridge RS". Moves its settings, caches and backups over to the
/// new folders the first time Truss starts (nothing is moved if Truss already has its own).
pub fn migrate_old_data() {
    let (Some(old), Some(new)) = (directories::ProjectDirs::from("", "", "bridge-rs"), project_dirs()) else { return };
    for (from, to) in [
        (old.config_dir(), new.config_dir()),
        (old.cache_dir(), new.cache_dir()),
        (old.data_local_dir(), new.data_local_dir()),
    ] {
        if from.exists() && !to.exists() {
            if let Some(parent) = to.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::rename(from, to);
        }
    }
    // Remove the old top-level folders if they're now empty.
    for dir in [old.config_dir().parent(), old.cache_dir().parent()].into_iter().flatten() {
        let _ = std::fs::remove_dir(dir);
    }
}

fn settings_file() -> Option<PathBuf> {
    // Lets a second copy (e.g. for testing) keep its own settings.
    if let Some(dir) = std::env::var_os("TRUSS_CONFIG_DIR") {
        return Some(PathBuf::from(dir).join("settings.json"));
    }
    project_dirs().map(|d| d.config_dir().join("settings.json"))
}

impl Settings {
    /// Every library folder, the default one first.
    pub fn all_library_folders(&self) -> Vec<PathBuf> {
        self.library_path.iter().chain(self.extra_library_folders.iter()).cloned().collect()
    }

    /// Where chart packs are saved: the pack folder if one is set, else the default songs folder.
    pub fn pack_folder(&self) -> Option<PathBuf> {
        self.pack_path.clone().or_else(|| self.library_path.clone())
    }

    /// The folders scanned for charts you have: the library folders, plus the pack folder when
    /// it isn't inside one of them.
    pub fn scan_folders(&self) -> Vec<PathBuf> {
        let mut out = self.all_library_folders();
        if let Some(p) = &self.pack_path {
            let low = |p: &std::path::Path| p.to_string_lossy().to_lowercase().replace('/', "\\").trim_end_matches('\\').to_string();
            let pl = low(p);
            let inside = out.iter().any(|f| {
                let fl = low(f);
                pl == fl || pl.starts_with(&format!("{fl}\\"))
            });
            if !inside {
                out.push(p.clone());
            }
        }
        out
    }

    pub fn load() -> Self {
        settings_file()
            .and_then(|p| std::fs::read(p).ok())
            // Hand-edited files (e.g. saved by PowerShell or Notepad) can start with a BOM.
            .and_then(|b| serde_json::from_slice(b.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&b)).ok())
            .unwrap_or_default()
    }

    /// Writes settings to disk. Called off the UI thread.
    pub async fn save(self) -> Result<(), String> {
        let path = settings_file().ok_or("No config directory available")?;
        let json = serde_json::to_vec_pretty(&self).map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
        }
        tokio::fs::write(path, json).await.map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn new_install_defaults() {
        let s = Settings::default();
        assert!(!s.is_sng && !s.download_videos);
        assert!(s.show_fretwork);
        assert_eq!((s.theme.as_str(), s.layout.as_str(), s.preview_fps), ("Rust", "list", 60));
        #[cfg(windows)]
        assert_eq!(s.window_backdrop, "frosted");
        // A settings file from an older version keeps its values; only missing ones get defaults.
        let old: Settings = serde_json::from_str(r#"{"refined_ui": false, "download_videos": true}"#).unwrap();
        assert!(old.download_videos && old.show_fretwork && old.text_colors);
        assert!(old.pack_path.is_none());
    }
}

#[cfg(test)]
mod pack_folder_tests {
    use super::*;

    #[test]
    fn pack_folder_defaults_to_songs() {
        let mut s = Settings { library_path: Some(PathBuf::from("C:\\Songs")), ..Default::default() };
        assert_eq!(s.pack_folder(), Some(PathBuf::from("C:\\Songs")));
        assert_eq!(s.scan_folders().len(), 1);
        s.pack_path = Some(PathBuf::from("C:\\Songs\\Packs"));
        assert_eq!(s.pack_folder(), Some(PathBuf::from("C:\\Songs\\Packs")));
        assert_eq!(s.scan_folders().len(), 1, "inside the songs folder: already scanned");
        s.pack_path = Some(PathBuf::from("D:\\Packs"));
        assert_eq!(s.scan_folders(), vec![PathBuf::from("C:\\Songs"), PathBuf::from("D:\\Packs")]);
        s.pack_path = Some(PathBuf::from("C:\\SongsPacks"));
        assert_eq!(s.scan_folders().len(), 2, "a folder whose name only starts the same isn't inside");
    }
}
