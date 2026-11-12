use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Folder that downloaded charts are saved into (the default library folder).
    pub library_path: Option<PathBuf>,
    /// More song folders that are checked for charts you already have.
    pub extra_library_folders: Vec<PathBuf>,
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
    /// Default instrument filter (API key, e.g. "guitar").
    pub instrument: Option<String>,
    /// Default difficulty filter (API key, e.g. "expert").
    pub difficulty: Option<String>,
    /// Name of the iced theme.
    pub theme: String,
    /// Render on the CPU (tiny-skia) instead of the GPU. Uses far less RAM.
    pub cpu_rendering: bool,
    /// Chart preview volume, 0..=1.
    pub preview_volume: f32,
    /// Browse layout: "list", "grid", "compact" or "hero".
    pub layout: String,
    /// Chart preview frame-rate cap; 0 = match the display's refresh rate.
    pub preview_fps: u32,
    /// Look for a new version on GitHub when Truss starts.
    pub check_updates: bool,
    /// The quieter "Refined look" (see style.rs).
    pub refined_ui: bool,
    /// The Leaderboard section in the details is open.
    pub show_leaderboard: bool,
    /// The Library page's sort order (see `library_ui::SortBy::key`); empty for the default.
    pub library_sort: String,
    /// Show Fretwork tiers beside the difficulties (they're always on hover).
    pub show_fretwork: bool,
    /// Window transparency (beta): "off", "mica", "acrylic" or "clear". Applies on start.
    pub window_backdrop: String,
    /// How see-through the window is with frosted glass on, 0.1 (barely) to 0.8 (very).
    pub glass_transparency: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            library_path: None,
            extra_library_folders: Vec::new(),
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
            cpu_rendering: false,
            preview_volume: 0.5,
            layout: "list".into(),
            preview_fps: 60,
            check_updates: true,
            refined_ui: true,
            show_leaderboard: false,
            library_sort: String::new(),
            show_fretwork: true,
            // Frosted glass needs Windows' blur; elsewhere it would only be see-through.
            window_backdrop: if cfg!(windows) { "frosted" } else { "off" }.into(),
            glass_transparency: 0.5,
        }
    }
}

/// The app's settings, cache and data folders (%APPDATA%\truss, %LOCALAPPDATA%\truss).
pub fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("", "", "truss")
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
        assert!(!s.is_sng && !s.download_videos && !s.cpu_rendering);
        assert!(s.refined_ui && s.show_fretwork);
        assert_eq!((s.theme.as_str(), s.layout.as_str(), s.preview_fps), ("Rust", "list", 60));
        #[cfg(windows)]
        assert_eq!(s.window_backdrop, "frosted");
        // A settings file from an older version keeps its values; only missing ones get defaults.
        let old: Settings = serde_json::from_str(r#"{"refined_ui": false, "download_videos": true}"#).unwrap();
        assert!(!old.refined_ui && old.download_videos && old.show_fretwork);
    }
}
