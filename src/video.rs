//! Video Sync: background videos via yt-dlp (YouTube search, pasted URLs) or a local file.
//! yt-dlp (and ffmpeg, which yt-dlp uses to merge streams) must be installed, like in Bridge.

use crate::backup::{backup_and_remove, find_files};
use futures::{SinkExt, Stream, channel::mpsc::Sender};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, BufReader};

const VIDEO_EXTS: &[&str] = &["mp4", "avi", "webm", "vp8", "ogv", "mpeg", "mkv"];

/// Full paths of yt-dlp and ffmpeg once found.
static TOOL_PATHS: std::sync::Mutex<(Option<PathBuf>, Option<PathBuf>)> = std::sync::Mutex::new((None, None));
/// PATH re-read from the registry, handed to yt-dlp so it also finds helpers installed after
/// Truss started (it uses Deno for YouTube and ffmpeg for merging).
static FRESH_PATH: std::sync::Mutex<Option<std::ffi::OsString>> = std::sync::Mutex::new(None);

/// Runs a tool by the path found by `check_tools`, falling back to its bare name.
fn command(program: &str) -> tokio::process::Command {
    let found = {
        let paths = TOOL_PATHS.lock().unwrap();
        match program {
            "yt-dlp" => paths.0.clone(),
            "ffmpeg" => paths.1.clone(),
            _ => None,
        }
    };
    let mut c = tokio::process::Command::new(found.as_deref().map(Path::as_os_str).unwrap_or(program.as_ref()));
    #[cfg(windows)]
    {
        // Don't flash a console window.
        c.creation_flags(0x0800_0000);
    }
    if let Some(path) = FRESH_PATH.lock().unwrap().clone() {
        c.env("PATH", path);
    }
    c.kill_on_drop(true);
    c
}

/// Folders to search for tools: this process's PATH plus the *current* user and system PATH from
/// the registry (so tools installed while Truss is running are found without a restart), and
/// winget's links folder.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        for key in [r"HKCU\Environment", r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment"] {
            let Ok(out) = std::process::Command::new("reg")
                .args(["query", key, "/v", "Path"])
                .creation_flags(0x0800_0000)
                .output()
            else {
                continue;
            };
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                let line = line.trim();
                let Some(rest) = line.strip_prefix("Path").or_else(|| line.strip_prefix("PATH")) else { continue };
                let value = rest.trim_start().splitn(2, char::is_whitespace).nth(1).unwrap_or("").trim();
                for d in value.split(';').filter(|d| !d.is_empty()) {
                    dirs.push(PathBuf::from(expand_env(d)));
                }
            }
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join(r"Microsoft\WinGet\Links"));
        }
    }
    dirs
}

/// Expands %VAR% references (registry PATH entries may contain them).
fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        match after.find('%') {
            Some(b) => {
                let name = &after[..b];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => out.push_str(&format!("%{name}%")),
                }
                rest = &after[b + 1..];
            }
            None => {
                out.push_str(&rest[a..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn find_tool(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    dirs.iter().map(|d| d.join(&file)).find(|p| p.is_file())
}

/// (yt-dlp available, ffmpeg available)
pub async fn check_tools() -> (bool, bool) {
    let (yt, ff, path) = tokio::task::spawn_blocking(|| {
        let mut dirs = search_dirs();
        let tools = (find_tool(&dirs, "yt-dlp"), find_tool(&dirs, "ffmpeg"));
        let mut seen = std::collections::HashSet::new();
        dirs.retain(|d| seen.insert(d.clone()));
        (tools.0, tools.1, std::env::join_paths(dirs).ok())
    })
    .await
    .unwrap_or((None, None, None));
    *TOOL_PATHS.lock().unwrap() = (yt, ff);
    *FRESH_PATH.lock().unwrap() = path;
    let ok = |p: &'static str, arg: &'static str| async move {
        command(p).arg(arg).output().await.is_ok_and(|o| o.status.success())
    };
    tokio::join!(ok("yt-dlp", "--version"), ok("ffmpeg", "-version"))
}

/// True if winget (Windows' package manager) is available.
pub async fn winget_available() -> bool {
    command("winget").arg("--version").output().await.is_ok_and(|o| o.status.success())
}

/// Installs the missing tools with winget, in a visible window so you can follow along and
/// answer any prompts (e.g. agreeing to winget's source terms the first time).
pub async fn install_tools(yt_dlp: bool, ffmpeg: bool) -> Result<(), String> {
    let mut failed = Vec::new();
    for (wanted, id, name) in [(yt_dlp, "yt-dlp.yt-dlp", "yt-dlp"), (ffmpeg, "Gyan.FFmpeg", "ffmpeg")] {
        if !wanted {
            continue;
        }
        // `cmd /c start /wait` gives winget its own console window.
        let status = tokio::process::Command::new("cmd")
            .args(["/c", "start", &format!("Installing {name}"), "/wait", "winget", "install", "--id", id, "-e", "--source", "winget"])
            .status()
            .await
            .map_err(|e| format!("Couldn't start winget: {e}"))?;
        if !status.success() {
            failed.push(name);
        }
    }
    if failed.is_empty() { Ok(()) } else { Err(format!("winget couldn't install {}.", failed.join(" and "))) }
}

#[derive(Debug, Clone, Deserialize)]
pub struct VideoResult {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub uploader: Option<String>,
    /// Seconds.
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub view_count: Option<u64>,
    #[serde(default)]
    pub url: Option<String>,
}

impl VideoResult {
    pub fn link(&self) -> String {
        self.url.clone().filter(|u| u.starts_with("http")).unwrap_or_else(|| format!("https://www.youtube.com/watch?v={}", self.id))
    }
    pub fn by(&self) -> String {
        self.channel.clone().or_else(|| self.uploader.clone()).unwrap_or_default()
    }
}

pub async fn search(query: String) -> Result<Vec<VideoResult>, String> {
    let out = command("yt-dlp")
        .args(["--dump-json", "--flat-playlist", "--no-warnings", "--ignore-errors"])
        .arg(format!("ytsearch12:{query}"))
        .output()
        .await
        .map_err(|e| format!("Couldn't run yt-dlp ({e}). Is it installed?"))?;
    let results: Vec<VideoResult> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if results.is_empty() && !out.status.success() {
        return Err(format!("yt-dlp search failed: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("")));
    }
    Ok(results)
}

#[derive(Debug, Clone)]
pub enum Progress {
    Status(String),
    Percent(f32),
    Finished(Result<PathBuf, String>),
}

/// Downloads `url` as the chart's video (≤1080p, video only; .webm preferred because .mp4 videos
/// don't play on Linux). The old video is backed up and replaced only after a successful download.
pub fn download(url: String, dir: PathBuf) -> impl Stream<Item = Progress> {
    iced::stream::channel(64, async move |mut tx: Sender<Progress>| {
        let r = run_download(&url, &dir, &mut tx).await;
        let _ = tx.send(Progress::Finished(r)).await;
    })
}

async fn run_download(url: &str, dir: &Path, tx: &mut Sender<Progress>) -> Result<PathBuf, String> {
    let tmp = dir.join(".truss-video-tmp");
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    tokio::fs::create_dir_all(&tmp).await.map_err(|e| e.to_string())?;
    let _ = tx.send(Progress::Status("Starting yt-dlp…".into())).await;
    let mut child = command("yt-dlp")
        .args([
            "-f",
            "bv*[height<=1080][ext=webm]/bv*[height<=1080]/b[height<=1080]/b",
            "--no-playlist",
            "--newline",
            "--no-warnings",
        ])
        .args(ffmpeg_location())
        .arg("-o")
        .arg(tmp.join("video.%(ext)s"))
        .arg(url)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't run yt-dlp ({e}). Is it installed?"))?;
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        // "[download]  42.3% of ~ 12.34MiB at ..."
        if let Some(rest) = line.strip_prefix("[download]")
            && let Some(pct) = rest.trim().split('%').next().and_then(|p| p.trim().parse::<f32>().ok())
        {
            let _ = tx.try_send(Progress::Percent(pct));
        } else if line.starts_with("[Merger]") || line.starts_with("[VideoConvertor]") {
            let _ = tx.send(Progress::Status("Processing…".into())).await;
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    let mut stderr = String::new();
    if let Some(mut e) = child.stderr.take() {
        use tokio::io::AsyncReadExt;
        let _ = e.read_to_string(&mut stderr).await;
    }
    let result = (|| {
        if !status.success() {
            let msg = stderr.lines().filter(|l| l.contains("ERROR")).last().unwrap_or(stderr.trim()).to_string();
            return Err(if msg.is_empty() { "yt-dlp failed".into() } else { msg });
        }
        let got = find_files(&tmp, "video", VIDEO_EXTS, false).into_iter().next().ok_or("yt-dlp didn't produce a video file")?;
        install(&got, dir)
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

fn ffmpeg_location() -> Vec<std::ffi::OsString> {
    match &TOOL_PATHS.lock().unwrap().1 {
        Some(p) => vec!["--ffmpeg-location".into(), p.clone().into_os_string()],
        None => Vec::new(),
    }
}

/// Puts `file` into `dir` as `video.<ext>`, backing up and removing any existing video.
fn install(file: &Path, dir: &Path) -> Result<PathBuf, String> {
    let ext = file.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_else(|| "mp4".into());
    for old in find_files(dir, "video", VIDEO_EXTS, false) {
        backup_and_remove(&old)?;
    }
    let dest = dir.join(format!("video.{ext}"));
    std::fs::rename(file, &dest).or_else(|_| std::fs::copy(file, &dest).map(|_| ())).map_err(|e| e.to_string())?;
    Ok(dest)
}

/// Copies a local video file into the chart folder as its video.
pub async fn import_local(src: PathBuf, dir: PathBuf) -> Result<PathBuf, String> {
    tokio::task::spawn_blocking(move || {
        let ext = src.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if !VIDEO_EXTS.contains(&ext.as_str()) {
            return Err(format!("\".{ext}\" isn't a supported video type"));
        }
        let tmp = dir.join(format!(".truss-import.{ext}"));
        std::fs::copy(&src, &tmp).map_err(|e| format!("Couldn't copy the video: {e}"))?;
        let r = install(&tmp, &dir);
        let _ = std::fs::remove_file(&tmp);
        r
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Backs up and removes the chart's video.
pub async fn delete(dir: PathBuf) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        for f in find_files(&dir, "video", VIDEO_EXTS, false) {
            backup_and_remove(&f)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

pub fn fmt_duration(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}
