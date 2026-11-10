//! Art Studio: finds album art on iTunes and makes blurred backgrounds from album art.

use crate::api::CLIENT;
use crate::backup::{backup_and_remove, find_files};
use image::{DynamicImage, GenericImageView, imageops::FilterType};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ArtResult {
    /// 100x100 preview.
    pub thumb_url: String,
    /// 600x600 version that gets saved.
    pub url: String,
    pub album: String,
    pub artist: String,
}

#[derive(Deserialize)]
struct ItunesResponse {
    #[serde(default)]
    results: Vec<ItunesItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItunesItem {
    artwork_url100: Option<String>,
    #[serde(default)]
    collection_name: String,
    #[serde(default)]
    artist_name: String,
}

async fn itunes(term: &str) -> Result<Vec<ArtResult>, String> {
    let resp: ItunesResponse = CLIENT
        .get("https://itunes.apple.com/search")
        .query(&[("term", term), ("entity", "album"), ("limit", "12")])
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp
        .results
        .into_iter()
        .filter_map(|i| {
            let t = i.artwork_url100?;
            Some(ArtResult { url: t.replace("100x100", "600x600"), thumb_url: t, album: i.collection_name, artist: i.artist_name })
        })
        .collect())
}

/// Album covers for `artist` + `album`, falling back to the artist alone (like Bridge).
pub async fn search(artist: String, album: String) -> Result<Vec<ArtResult>, String> {
    let mut results = itunes(format!("{artist} {album}").trim()).await?;
    if results.is_empty() && !album.trim().is_empty() {
        results = itunes(artist.trim()).await?;
    }
    Ok(results)
}

pub async fn fetch(url: String) -> Result<bytes::Bytes, String> {
    CLIENT
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())
}

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg"];

/// The chart's album art file, if any.
pub fn album_file(dir: &Path) -> Option<PathBuf> {
    let mut files = find_files(dir, "album", IMAGE_EXTS, false);
    files.sort();
    files.pop()
}

/// Downloads `url` and saves it as the chart's `album.jpg` (512x512, what Clone Hero expects),
/// backing up and replacing any existing album art.
pub async fn save_album_art(dir: PathBuf, url: String) -> Result<(), String> {
    let bytes = fetch(url).await?;
    tokio::task::spawn_blocking(move || {
        let img = image::load_from_memory(&bytes).map_err(|e| format!("Couldn't read the image: {e}"))?;
        let img = img.resize_to_fill(512, 512, FilterType::Lanczos3).to_rgb8();
        for old in find_files(&dir, "album", IMAGE_EXTS, false) {
            backup_and_remove(&old)?;
        }
        let out = std::fs::File::create(dir.join("album.jpg")).map_err(|e| format!("Couldn't save album art: {e}"))?;
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(out), 92);
        enc.encode_image(&img).map_err(|e| format!("Couldn't save album art: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Finds and saves art for a chart automatically (batch mode): the first iTunes result.
pub async fn auto_album_art(dir: PathBuf, artist: String, album: String) -> Result<bool, String> {
    let results = search(artist, album).await?;
    let Some(first) = results.into_iter().next() else { return Ok(false) };
    save_album_art(dir, first.url).await?;
    Ok(true)
}

/// Makes a 1920x1080 `background.jpg`: the album art scaled to fill the screen and blurred
/// (`blur` 0-100), or a dark gradient when there's no album art. Replaces any old background.
pub async fn generate_background(dir: PathBuf, blur: u32) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let (w, h) = (1920u32, 1080u32);
        let img = match album_file(&dir).and_then(|p| image::open(p).ok()) {
            Some(art) => blurred_cover(&art, w, h, blur),
            None => gradient(w, h),
        };
        for old in find_files(&dir, "background", IMAGE_EXTS, true) {
            backup_and_remove(&old)?;
        }
        let out = std::fs::File::create(dir.join("background.jpg")).map_err(|e| format!("Couldn't save background: {e}"))?;
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(out), 90);
        enc.encode_image(&img).map_err(|e| format!("Couldn't save background: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn blurred_cover(art: &DynamicImage, w: u32, h: u32, blur: u32) -> image::RgbImage {
    if blur == 0 {
        return art.resize_to_fill(w, h, FilterType::Lanczos3).to_rgb8();
    }
    // Blur at quarter size: indistinguishable for a heavy blur and ~16x faster.
    let small = art.resize_to_fill(w / 4, h / 4, FilterType::Triangle);
    let sigma = (blur as f32 / 4.0).max(0.5);
    let blurred = image::imageops::fast_blur(&small.to_rgb8(), sigma);
    let mut big = image::imageops::resize(&blurred, w, h, FilterType::Triangle);
    // Darken a little so notes stay readable over bright covers.
    for p in big.pixels_mut() {
        for c in p.0.iter_mut() {
            *c = (*c as f32 * 0.8) as u8;
        }
    }
    big
}

fn gradient(w: u32, h: u32) -> image::RgbImage {
    let (a, b) = ([0x1a, 0x1a, 0x2e], [0x16, 0x21, 0x3e]);
    image::RgbImage::from_fn(w, h, |x, y| {
        let t = (x as f32 / w as f32 + y as f32 / h as f32) / 2.0;
        image::Rgb([0, 1, 2].map(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t) as u8))
    })
}

/// Backs up and removes the chart's background image(s).
pub async fn delete_background(dir: PathBuf) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        for f in find_files(&dir, "background", IMAGE_EXTS, true) {
            backup_and_remove(&f)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Small preview of a local image (for showing current art / background).
pub async fn local_preview(path: PathBuf, px: u32) -> Result<crate::api::DecodedImage, String> {
    tokio::task::spawn_blocking(move || {
        let img = image::open(&path).map_err(|e| e.to_string())?;
        let (iw, ih) = img.dimensions();
        let t = img.thumbnail(px, px * ih / iw.max(1)).to_rgba8();
        Ok(crate::api::DecodedImage { width: t.width(), height: t.height(), rgba: t.into_raw() })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_from_art_and_gradient() {
        let dir = std::env::temp_dir().join("truss-art-test");
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: tests that touch backups set the same value.
        unsafe { std::env::set_var("TRUSS_BACKUP_DIR", std::env::temp_dir().join("truss-test-backups")) };
        std::fs::create_dir_all(&dir).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        // No album art: gradient.
        rt.block_on(generate_background(dir.clone(), 50)).unwrap();
        assert_eq!(image::image_dimensions(dir.join("background.jpg")).unwrap(), (1920, 1080));
        // With album art (a small test image), replacing the gradient.
        image::RgbImage::from_fn(300, 300, |x, _| image::Rgb([x as u8, 80, 200])).save(dir.join("album.png")).unwrap();
        let t = std::time::Instant::now();
        rt.block_on(generate_background(dir.clone(), 50)).unwrap();
        println!("background in {:?}", t.elapsed());
        assert_eq!(image::image_dimensions(dir.join("background.jpg")).unwrap(), (1920, 1080));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
