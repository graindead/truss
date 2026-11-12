//! Downloads a chart from files.enchor.us and extracts it into the library.

use crate::api::{CLIENT, Chart, FILES_URL, strip_tags};
use crate::settings::Settings;
use crate::sng::SngExtractor;
use futures::{SinkExt, Stream, StreamExt, channel::mpsc::Sender};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum Progress {
    Status(String),
    Bytes { done: u64, total: Option<u64> },
    Finished(Result<PathBuf, String>),
}

/// Deletes the temporary download location if the download is cancelled
/// (task dropped) or fails.
pub struct TempGuard(pub Option<PathBuf>);

impl Drop for TempGuard {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            if p.is_dir() {
                let _ = std::fs::remove_dir_all(&p);
            } else {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
}

/// `replace`: existing copies of this chart to move to the Recycle Bin once the new copy has
/// downloaded successfully (a redownload). They're only removed after the download succeeds.
pub fn start(chart: Arc<Chart>, settings: Settings, replace: Vec<PathBuf>) -> impl Stream<Item = Progress> {
    iced::stream::channel(32, async move |mut tx: Sender<Progress>| {
        let result = run(&chart, &settings, &replace, &mut tx).await;
        let _ = tx.send(Progress::Finished(result)).await;
    })
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

async fn run(
    chart: &Chart,
    settings: &Settings,
    replace: &[PathBuf],
    tx: &mut Sender<Progress>,
) -> Result<PathBuf, String> {
    let library = settings
        .library_path
        .clone()
        .ok_or("No library folder set. Choose one in Settings.")?;
    if !library.is_dir() {
        return Err(format!("Library folder not found: {}", library.display()));
    }

    let mut rel = resolve_chart_folder_name(&settings.chart_folder_name, chart);
    // RhythmVerse charts come as folders (zips), so they're always saved as folders.
    if settings.is_sng && !chart.is_rv() {
        rel.push_str(".sng");
    }
    let dest = library.join(&rel);
    if dest.exists() && !replace.iter().any(|p| same_path(p, &dest)) {
        return Err(format!("Already in library: {}", dest.display()));
    }

    #[cfg(feature = "rhythmverse")]
    if let Some(rv) = &chart.rv {
        let tag: String = chart.md5.chars().filter(|c| c.is_ascii_alphanumeric()).take(24).collect();
        let staged = library.join(format!(".truss-tmp-{tag}-staged"));
        let mut guard = TempGuard(Some(staged.clone()));
        match crate::preview::rv_cache::cached(chart, settings.download_videos) {
            // Previewed earlier: use that copy instead of downloading again.
            Some(cached) => {
                let _ = tx.send(Progress::Status("Using the previewed copy...".into())).await;
                let _ = std::fs::remove_dir_all(&staged);
                move_or_copy(&cached, &staged)?;
            }
            None => crate::rhythmverse::download(rv, &staged, &library, &tag, settings.download_videos, tx).await?,
        }
        if !replace.is_empty() {
            let _ = tx.send(Progress::Status("Removing old copy...".into())).await;
            crate::library::recycle(replace.to_vec()).await?;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create folder: {e}"))?;
        }
        move_path(&staged, &dest)?;
        guard.0 = None;
        return Ok(dest);
    }

    let novideo = chart.has_video_background && !settings.download_videos;
    let url = format!("{FILES_URL}/{}{}.sng", chart.md5, if novideo { "_novideo" } else { "" });

    let _ = tx.send(Progress::Status("Connecting...".into())).await;
    let resp = CLIENT
        .get(&url)
        .timeout(Duration::from_secs(60 * 30))
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(match resp.status().as_u16() {
            404 | 410 => "This chart is no longer available: Encore no longer has its file (it was removed or replaced). Try another version of this song.".into(),
            429 => "Encore is limiting downloads right now. Wait a bit and try again.".into(),
            code => format!("Download failed: Encore returned an error ({code}). Try again later."),
        });
    }
    let total = resp.content_length();

    // Temp location inside the library so the final move is an instant rename.
    let tmp = library.join(format!(".truss-tmp-{}-{}", chart.md5, std::process::id()));
    let mut guard = TempGuard(Some(tmp.clone()));

    enum Sink {
        Raw(std::io::BufWriter<std::fs::File>),
        Extract(SngExtractor),
    }
    let mut sink = if settings.is_sng {
        let f = std::fs::File::create(&tmp).map_err(|e| format!("Cannot write to library: {e}"))?;
        Sink::Raw(std::io::BufWriter::with_capacity(1 << 20, f))
    } else {
        std::fs::create_dir_all(&tmp).map_err(|e| format!("Cannot write to library: {e}"))?;
        Sink::Extract(SngExtractor::new(&tmp))
    };

    let mut stream = resp.bytes_stream();
    let mut done: u64 = 0;
    let mut last_report = Instant::now() - Duration::from_secs(1);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Connection lost: {e}"))?;
        done += chunk.len() as u64;
        match &mut sink {
            Sink::Raw(w) => w.write_all(&chunk).map_err(|e| format!("Write failed: {e}"))?,
            Sink::Extract(x) => x.feed(&chunk)?,
        }
        // Throttle UI updates to ~30/s.
        if last_report.elapsed() >= Duration::from_millis(33) {
            last_report = Instant::now();
            let _ = tx.try_send(Progress::Bytes { done, total });
        }
    }
    let _ = tx.send(Progress::Bytes { done, total }).await;

    match sink {
        Sink::Raw(mut w) => w.flush().map_err(|e| format!("Write failed: {e}"))?,
        Sink::Extract(x) => {
            if !x.is_done() {
                return Err("Download ended early (incomplete .sng)".into());
            }
        }
    }

    if !replace.is_empty() {
        let _ = tx.send(Progress::Status("Removing old copy...".into())).await;
        crate::library::recycle(replace.to_vec()).await?;
    }
    let _ = tx.send(Progress::Status("Moving into library...".into())).await;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create folder: {e}"))?;
    }
    move_path(&tmp, &dest)?;
    guard.0 = None;
    Ok(dest)
}

pub fn move_path(from: &Path, to: &Path) -> Result<(), String> {
    // Windows can briefly lock freshly written files (indexers/AV); retry a few times.
    let mut last = None;
    for i in 0..10 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(50 * (i + 1)));
            }
        }
    }
    Err(format!("Failed to move chart into library: {}", last.unwrap()))
}

/// Moves a folder, copying it when it's on another drive (the preview cache is in the temp
/// folder, which may be on a different drive from the library).
#[cfg(feature = "rhythmverse")]
fn move_or_copy(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            let dest = to.join(e.file_name());
            if e.file_type()?.is_dir() {
                copy_dir(&e.path(), &dest)?;
            } else {
                std::fs::copy(e.path(), dest)?;
            }
        }
        Ok(())
    }
    copy_dir(from, to).map_err(|e| format!("Failed to copy the previewed chart: {e}"))?;
    let _ = std::fs::remove_dir_all(from);
    Ok(())
}

/// Port of Bridge's `resolveChartFolderName`.
pub fn resolve_chart_folder_name(template: &str, chart: &Chart) -> String {
    let template = if template.split('/').all(|p| p.is_empty()) { "{artist} - {name} ({charter})" } else { template };
    let f = |o: &Option<String>| o.as_deref().map(strip_tags).unwrap_or_default();
    let (name, artist, album, genre, year, charter) =
        (f(&chart.name), f(&chart.artist), f(&chart.album), f(&chart.genre), f(&chart.year), f(&chart.charter));

    let parts: Vec<String> = template
        .split('/')
        .map(|part| {
            sanitize_filename(
                &part
                    .replace("{name}", &name)
                    .replace("{artist}", &artist)
                    .replace("{album}", &album)
                    .replace("{genre}", &genre)
                    .replace("{year}", &year)
                    .replace("{charter}", &charter),
            )
        })
        .filter(|p| !p.is_empty())
        .collect();

    if parts.is_empty() { chart.md5.clone() } else { parts.join(std::path::MAIN_SEPARATOR_STR) }
}

/// Mirrors the `sanitize-filename` npm package with Bridge's look-alike replacements.
pub fn sanitize_filename(input: &str) -> String {
    let mut s: String = input
        .chars()
        .map(|c| match c {
            '<' => '❮',
            '>' => '❯',
            ':' => '꞉',
            '"' => '\'',
            '/' => '／',
            '\\' => '⧵',
            '|' => '⏐',
            '?' => '？',
            '*' => '⁎',
            c if (c as u32) < 0x20 || (0x80..=0x9f).contains(&(c as u32)) => '_',
            c => c,
        })
        .collect();

    if !s.is_empty() && s.chars().all(|c| c == '.') {
        s = "_".repeat(s.len());
    }
    let stem = s.split('.').next().unwrap_or("").to_ascii_lowercase();
    let reserved = matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4 && (stem.starts_with("com") || stem.starts_with("lpt")) && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        s.insert(0, '_');
    }
    // Windows strips trailing dots/spaces, which would break paths.
    let trimmed_len = s.trim_end_matches(['.', ' ']).len();
    if trimmed_len < s.len() {
        s.truncate(trimmed_len);
    }
    // Limit to 255 bytes on a char boundary.
    if s.len() > 255 {
        let mut end = 255;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

#[cfg(all(test, feature = "rhythmverse"))]
mod tests {
    use super::*;

    /// A previewed chart in the temp folder (C:) moves into a library on another drive.
    #[test]
    fn move_or_copy_across_drives() {
        let from = std::env::temp_dir().join(format!("truss-moc-{}", std::process::id()));
        let to = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("truss-moc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&to);
        std::fs::create_dir_all(from.join("sub")).unwrap();
        std::fs::write(from.join("song.ini"), "[song]\nname = x\n").unwrap();
        std::fs::write(from.join("sub").join("a.txt"), "hi").unwrap();
        move_or_copy(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read_to_string(to.join("sub").join("a.txt")).unwrap(), "hi");
        assert!(to.join("song.ini").is_file());
        std::fs::remove_dir_all(&to).unwrap();
    }
}
