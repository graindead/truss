//! Updates from GitHub: Truss checks the latest release of graindead/truss, and can download
//! its truss.exe, check it against the release's SHA-256, swap it in and restart.
//!
//! Releases are built by GitHub Actions (.github/workflows/release.yml) when a version tag such
//! as `v0.2.0` is pushed. Each one has `truss.exe` and `truss.exe.sha256`.

use futures::{SinkExt, Stream, StreamExt, channel::mpsc::Sender};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

const REPO: &str = "graindead/truss";
const EXE_ASSET: &str = "truss.exe";
const SHA_ASSET: &str = "truss.exe.sha256";

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent(concat!("truss/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .expect("http client")
});

/// This build's version. TRUSS_FAKE_VERSION pretends to be another one (for testing updates).
pub fn current_version() -> String {
    std::env::var("TRUSS_FAKE_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string())
}

#[derive(Debug, Clone)]
pub struct Release {
    pub version: String,
    pub page_url: String,
    exe_url: String,
    sha_url: String,
    pub size: u64,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// "v1.2.3" / "1.2.3" -> (1, 2, 3). Anything after a '-' or '+' is ignored.
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let core = v.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((parse_version(candidate), parse_version(current)), (Some(a), Some(b)) if a > b)
}

/// The latest release if it's newer than this build (None when up to date or there are no
/// releases yet).
pub async fn check() -> Result<Option<Release>, String> {
    let r = HTTP
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("Couldn't reach GitHub: {e}"))?;
    if r.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None); // no releases yet
    }
    if !r.status().is_success() {
        return Err(format!("GitHub returned {}", r.status()));
    }
    let rel: GhRelease = r.json().await.map_err(|e| format!("Unexpected answer from GitHub: {e}"))?;
    if rel.draft || rel.prerelease || !is_newer(&rel.tag_name, &current_version()) {
        return Ok(None);
    }
    let asset = |n: &str| rel.assets.iter().find(|a| a.name.eq_ignore_ascii_case(n));
    let (Some(exe), Some(sha)) = (asset(EXE_ASSET), asset(SHA_ASSET)) else {
        return Err(format!("Release {} has no {EXE_ASSET} yet (it may still be building).", rel.tag_name));
    };
    Ok(Some(Release {
        version: rel.tag_name.trim_start_matches(['v', 'V']).to_string(),
        page_url: rel.html_url.clone(),
        exe_url: exe.browser_download_url.clone(),
        sha_url: sha.browser_download_url.clone(),
        size: exe.size,
    }))
}

#[derive(Debug, Clone)]
pub enum InstallEvent {
    Progress(f32),
    /// The new version is in place; restarting runs it.
    Done(Result<(), String>),
}

/// Downloads and installs a release in place of the running exe.
pub fn install(rel: Release) -> impl Stream<Item = InstallEvent> {
    iced::stream::channel(16, async move |mut tx: Sender<InstallEvent>| {
        let r = install_inner(&rel, &mut tx).await;
        let _ = tx.send(InstallEvent::Done(r)).await;
    })
}

fn old_path(exe: &Path) -> PathBuf {
    exe.with_extension("exe.old")
}

async fn install_inner(rel: &Release, tx: &mut Sender<InstallEvent>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // Next to the exe, so the swap is a rename on the same drive.
    let new = exe.with_extension("exe.new");

    let want = HTTP
        .get(&rel.sha_url)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Couldn't get the checksum: {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let want = want.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    if want.len() != 64 {
        return Err("The release's checksum file is malformed.".into());
    }

    let resp = HTTP
        .get(&rel.exe_url)
        .timeout(Duration::from_secs(30 * 60))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Download failed: {e}"))?;
    let total = resp.content_length().unwrap_or(rel.size).max(1);
    let mut hasher = Sha256::new();
    {
        use std::io::Write;
        let file = std::fs::File::create(&new).map_err(|e| format!("Can't write next to truss.exe: {e}"))?;
        let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
        let mut stream = resp.bytes_stream();
        let mut done = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| {
                let _ = std::fs::remove_file(&new);
                format!("Connection lost: {e}")
            })?;
            hasher.update(&chunk);
            out.write_all(&chunk).map_err(|e| e.to_string())?;
            done += chunk.len() as u64;
            let _ = tx.try_send(InstallEvent::Progress(done as f32 / total as f32));
        }
        out.flush().map_err(|e| e.to_string())?;
    }
    let got: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != want {
        let _ = std::fs::remove_file(&new);
        return Err("The download didn't match the release's checksum, so it wasn't installed.".into());
    }
    swap(&exe, &new)
}

/// Puts `new` in place of the running exe. Windows lets a running exe be renamed (not
/// deleted), so the old one is moved aside and deleted on the next start.
fn swap(exe: &Path, new: &Path) -> Result<(), String> {
    let old = old_path(exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(|e| {
        let _ = std::fs::remove_file(new);
        format!("Couldn't replace truss.exe (is it in a protected folder?): {e}")
    })?;
    if let Err(e) = std::fs::rename(new, exe) {
        let _ = std::fs::rename(&old, exe); // put the old version back
        let _ = std::fs::remove_file(new);
        return Err(format!("Couldn't install the update: {e}"));
    }
    Ok(())
}

/// Starts the (updated) exe and exits this one.
pub fn restart() -> ! {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
    std::process::exit(0)
}

/// Deletes the previous version left by an update (it may still be closing for a moment).
pub fn cleanup_old() {
    let Ok(exe) = std::env::current_exe() else { return };
    let old = old_path(&exe);
    let new = exe.with_extension("exe.new");
    std::thread::spawn(move || {
        for _ in 0..20 {
            let _ = std::fs::remove_file(&new);
            if !old.exists() || std::fs::remove_file(&old).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(is_newer("v0.1.1", "0.1.0"));
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.0.9", "0.1.0"));
        assert!(!is_newer("nightly", "0.1.0"));
        assert_eq!(parse_version("v1.2.3-beta"), Some((1, 2, 3)));
    }

    #[test]
    fn swaps_and_keeps_old() {
        let dir = std::env::temp_dir().join(format!("truss-upd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("truss.exe");
        let new = dir.join("truss.exe.new");
        std::fs::write(&exe, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        swap(&exe, &new).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(old_path(&exe)).unwrap(), "old");
        assert!(!new.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Network test: TRUSS_TEST_UPDATE=1 checks GitHub for a release newer than 0.0.1.
    #[test]
    fn live_check() {
        if std::env::var("TRUSS_TEST_UPDATE").is_err() {
            return;
        }
        let _ = rustls::crypto::ring::default_provider().install_default();
        unsafe { std::env::set_var("TRUSS_FAKE_VERSION", "0.0.1") };
        let r = tokio::runtime::Runtime::new().unwrap().block_on(check());
        println!("{r:?}");
        assert!(r.unwrap().is_some());
    }
}
