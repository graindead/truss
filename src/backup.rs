//! Copies of chart files the tools are about to change or replace, kept in
//! `%LOCALAPPDATA%\truss\data\backups\<date>\` so any edit can be undone by hand.

use std::path::{Path, PathBuf};

pub fn backup_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("TRUSS_BACKUP_DIR") {
        return Some(PathBuf::from(d));
    }
    crate::settings::project_dirs().map(|d| d.data_local_dir().join("backups"))
}

/// Copies `file` into today's backup folder (as `<chart folder> - <file name>`), returning the copy.
pub fn backup(file: &Path) -> Result<PathBuf, String> {
    let root = backup_dir().ok_or("No app data folder available for backups")?;
    let day = root.join(chrono::Local::now().format("%Y-%m-%d").to_string());
    std::fs::create_dir_all(&day).map_err(|e| format!("Couldn't create backup folder: {e}"))?;
    let folder = file.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let base = crate::download::sanitize_filename(&format!("{folder} - {name}"));
    let mut dest = day.join(&base);
    let mut n = 2;
    while dest.exists() {
        dest = day.join(format!("{n} {base}"));
        n += 1;
    }
    std::fs::copy(file, &dest).map_err(|e| format!("Couldn't back up {}: {e}", file.display()))?;
    Ok(dest)
}

/// Backs up and then removes `file` (used when a tool replaces album art, backgrounds, video).
pub fn backup_and_remove(file: &Path) -> Result<(), String> {
    backup(file)?;
    std::fs::remove_file(file).map_err(|e| format!("Couldn't remove {}: {e}", file.display()))
}

/// Files in `dir` whose name (case-insensitively) is `stem.<one of exts>`, or starts with `stem`
/// when `prefix` is set.
pub fn find_files(dir: &Path, stem: &str, exts: &[&str], prefix: bool) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            let Some((s, ext)) = n.rsplit_once('.') else { return false };
            exts.contains(&ext) && if prefix { s.starts_with(stem) } else { s == stem }
        })
        .map(|e| e.path())
        .collect()
}
