//! Album art thumbnails saved on disk (`%LOCALAPPDATA%\truss\cache\art`), so covers seen before
//! show up instantly instead of being downloaded and shrunk again. Only the small list
//! thumbnails are kept; the big cover in the details panel is still downloaded when needed.
//!
//! Files are named after the cover (Encore's image hash, or a hash of a RhythmVerse address) and
//! the thumbnail size. An empty file means "this chart has no cover".

use crate::api::DecodedImage;
use std::path::PathBuf;

/// The cache is trimmed back to this size (oldest-used first) when Truss starts.
const MAX_BYTES: u64 = 300 * 1024 * 1024;
/// …down to this, so it isn't trimmed again on every start.
const TRIM_TO: u64 = 250 * 1024 * 1024;

#[derive(Debug, Clone)]
pub enum Cached {
    Thumb(DecodedImage),
    /// Encore said the chart has no cover.
    Missing,
    /// Not saved yet (or the file couldn't be read).
    Miss,
}

fn dir() -> Option<PathBuf> {
    crate::settings::project_dirs().map(|d| d.cache_dir().join("art"))
}

fn path(key: &str, px: u32) -> Option<PathBuf> {
    // Encore keys are already hex hashes; anything else (RhythmVerse addresses) is hashed.
    let name = if key.len() <= 64 && !key.is_empty() && key.bytes().all(|b| b.is_ascii_hexdigit()) {
        key.to_ascii_lowercase()
    } else {
        blake3::hash(key.as_bytes()).to_hex()[..32].to_string()
    };
    dir().map(|d| d.join(format!("{name}-{px}.jpg")))
}

/// Reads a saved thumbnail. Blocking: call from a worker thread.
pub fn read(key: &str, px: u32) -> Cached {
    let Some(p) = path(key, px) else { return Cached::Miss };
    let Ok(bytes) = std::fs::read(&p) else { return Cached::Miss };
    if bytes.is_empty() {
        return Cached::Missing;
    }
    let Ok(img) = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg) else {
        let _ = std::fs::remove_file(&p);
        return Cached::Miss;
    };
    // Mark it as recently used, so trimming removes covers that haven't been seen in a while.
    if let Ok(f) = std::fs::File::options().write(true).open(&p) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
    let rgba = img.to_rgba8();
    Cached::Thumb(DecodedImage { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() })
}

/// Saves a thumbnail. Blocking; failures are ignored (the cover just downloads again next time).
pub fn write(key: &str, px: u32, thumb: &image::RgbaImage) {
    let Some(p) = path(key, px) else { return };
    let rgb = image::DynamicImage::ImageRgba8(thumb.clone()).to_rgb8();
    let mut out = Vec::with_capacity(8 * 1024);
    if image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90).encode_image(&rgb).is_err() {
        return;
    }
    store(&p, &out);
}

/// Remembers that a chart has no cover.
pub fn write_missing(key: &str, px: u32) {
    if let Some(p) = path(key, px) {
        store(&p, &[]);
    }
}

fn store(p: &std::path::Path, bytes: &[u8]) {
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Write to a temporary name first so a half-written file is never read as a cover.
    let tmp = p.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, p).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Deletes the least recently used covers when the cache has grown past its limit. Blocking.
pub fn trim() {
    let Some(d) = dir() else { return };
    trim_dir(&d, MAX_BYTES, TRIM_TO);
}

fn trim_dir(d: &std::path::Path, max: u64, to: u64) {
    let Ok(entries) = std::fs::read_dir(d) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= max {
        return;
    }
    files.sort_by_key(|f| f.0);
    for (_, len, p) in files {
        if total <= to {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        let a = path("ABCdef0123", 88).unwrap();
        assert!(a.ends_with("abcdef0123-88.jpg"));
        let b = path("https://rhythmverse.co/assets/album_art/x/a.png", 192).unwrap();
        let name = b.file_name().unwrap().to_str().unwrap();
        assert_eq!(name.len(), 32 + "-192.jpg".len());
        assert!(!name.contains('/'));
    }

    #[test]
    fn trims_oldest_first() {
        let d = std::env::temp_dir().join(format!("truss-art-trim-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let t0 = std::time::SystemTime::now() - std::time::Duration::from_secs(1000);
        for i in 0..5u64 {
            let p = d.join(format!("{i}.jpg"));
            std::fs::write(&p, vec![0u8; 100]).unwrap();
            let f = std::fs::File::options().write(true).open(&p).unwrap();
            f.set_modified(t0 + std::time::Duration::from_secs(i * 10)).unwrap();
        }
        trim_dir(&d, 350, 250);
        let mut left: Vec<String> =
            std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["3.jpg", "4.jpg"]);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
