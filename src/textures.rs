//! Highway / strikeline / note textures for the chart preview (the same artwork the
//! original player loads from static.enchor.us). Files are cached on disk after the first
//! download, and decoded (all animation frames) in parallel off the UI thread.

use crate::chart::{InstrumentType, nf, nt};
use iced::widget::image::Handle;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const BASE_URL: &str = "https://static.enchor.us";
/// Smallest width kept when making the smaller copies of a texture.
const MIN_MIP_WIDTH: u32 = 8;

pub const SP_FLAG: u32 = 1 << 31;
pub const BARRE1: u8 = 101;
pub const BARRE2: u8 = 102;
pub const BARRE3: u8 = 103;

/// One image at several sizes (each half the one before), so it can be drawn small without the
/// shimmer and grain of squeezing a big image into a few pixels.
pub struct Mips {
    /// Width and height of each level, largest first.
    sizes: Vec<(u32, u32)>,
    handles: Vec<Handle>,
}

impl Mips {
    pub fn new(img: image::RgbaImage) -> Self {
        let mut sizes = Vec::new();
        let mut handles = Vec::new();
        for level in mip_chain(img) {
            sizes.push(level.dimensions());
            handles.push(Handle::from_rgba(level.width(), level.height(), level.into_raw()));
        }
        Self { sizes, handles }
    }

    /// The smallest copy that is still at least `px` wide.
    pub fn pick(&self, px: f32) -> &Handle {
        let i = self.sizes.iter().rposition(|&(w, _)| w as f32 >= px).unwrap_or(0);
        &self.handles[i]
    }
}

/// Halves an image repeatedly (in premultiplied alpha, so transparent edges don't darken).
fn mip_chain(img: image::RgbaImage) -> Vec<image::RgbaImage> {
    let mut out = vec![img];
    loop {
        let last = out.last().unwrap();
        let (w, h) = (last.width() / 2, last.height() / 2);
        if w < MIN_MIP_WIDTH || h == 0 {
            break;
        }
        let next = shrink(last, w, h);
        out.push(next);
    }
    out
}

/// Area-averaged resize with premultiplied alpha.
pub fn shrink(img: &image::RgbaImage, w: u32, h: u32) -> image::RgbaImage {
    let mut pre = image::Rgba32FImage::new(img.width(), img.height());
    for (d, s) in pre.pixels_mut().zip(img.pixels()) {
        let a = s[3] as f32 / 255.0;
        let k = a / 255.0; // the resizer works in 0..1
        *d = image::Rgba([s[0] as f32 * k, s[1] as f32 * k, s[2] as f32 * k, a]);
    }
    let small = image::imageops::resize(&pre, w, h, image::imageops::FilterType::Triangle);
    let mut out = image::RgbaImage::new(w, h);
    for (d, s) in out.pixels_mut().zip(small.pixels()) {
        let a = s[3].clamp(0.0, 1.0);
        let c = |v: f32| if a > 0.0 { (v / a * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
        *d = image::Rgba([c(s[0]), c(s[1]), c(s[2]), (a * 255.0).round() as u8]);
    }
    out
}

pub struct AnimTexture {
    pub frames: Vec<Mips>,
    /// Cumulative end time of each frame in ms.
    pub frame_ends: Vec<u32>,
    pub total_ms: u32,
    pub aspect: f32,
}

impl AnimTexture {
    pub fn frame(&self, t_ms: u64) -> &Mips {
        if self.frames.len() <= 1 || self.total_ms == 0 {
            return &self.frames[0];
        }
        let t = (t_ms % self.total_ms as u64) as u32;
        let i = self.frame_ends.partition_point(|&e| e <= t).min(self.frames.len() - 1);
        &self.frames[i]
    }
}

impl std::fmt::Debug for TextureSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextureSet({:?}, {} notes)", self.itype, self.notes.len())
    }
}

pub struct TextureSet {
    /// Small solid-colour images for sustains, keyed by 0xRRGGBB.
    pub solid: HashMap<u32, Handle>,
    pub itype: InstrumentType,
    /// The highway texture at several sizes, each cut into one-pixel rows (repeated three times
    /// so smooth sampling never picks up a neighbouring row).
    pub highway: Vec<HighwayLevel>,
    pub strikeline: Mips,
    /// Vertical black-to-clear gradient for fading out the far end of the highway.
    pub fade: Handle,
    pub strikeline_aspect: f32,
    pub notes: HashMap<(u8, u32), Arc<AnimTexture>>,
}

pub struct HighwayLevel {
    pub width: u32,
    pub rows: Vec<Handle>,
}

impl TextureSet {
    /// Texture for a note, tolerating flag bits that have no artwork (e.g. flam), which the
    /// original player would silently skip.
    pub fn note(&self, ty: u8, flags: u32) -> Option<&Arc<AnimTexture>> {
        self.notes.get(&(ty, flags)).or_else(|| {
            let known = nf::STRUM | nf::HOPO | nf::TAP | nf::DOUBLE_KICK | nf::TOM | nf::CYMBAL | nf::GHOST | nf::ACCENT | SP_FLAG;
            let f = flags & known;
            self.notes
                .get(&(ty, f))
                .or_else(|| self.notes.get(&(ty, f & !(nf::GHOST | nf::ACCENT))))
                .or_else(|| self.notes.get(&(ty, f & SP_FLAG)))
        })
    }
}

/// (note type, flags, file stem) for every texture the instrument needs.
fn note_texture_list(itype: InstrumentType) -> Vec<(u8, u32, String)> {
    let mut list = Vec::new();
    let sp_flags = [(0u32, ""), (SP_FLAG, "-sp")];
    match itype {
        InstrumentType::Drums => {
            for (sp, sps) in sp_flags {
                list.push((nt::KICK, sp, format!("drums-kick{sps}")));
                list.push((nt::KICK, nf::DOUBLE_KICK | sp, format!("drums-kick{sps}")));
            }
            let colors = [(nt::RED_DRUM, "red"), (nt::YELLOW_DRUM, "yellow"), (nt::BLUE_DRUM, "blue"), (nt::GREEN_DRUM, "green")];
            let dynamics = [(0u32, ""), (nf::GHOST, "-ghost"), (nf::ACCENT, "-accent")];
            for (ty, color) in colors {
                for (dy, dys) in dynamics {
                    for (sp, sps) in sp_flags {
                        list.push((ty, sp | dy | nf::TOM, format!("drums-{color}-tom{dys}{sps}")));
                        if ty != nt::RED_DRUM {
                            list.push((ty, sp | dy | nf::CYMBAL, format!("drums-{color}-cymbal{dys}{sps}")));
                        }
                    }
                }
            }
        }
        InstrumentType::SixFret => {
            let lanes: [(&[u8], &str); 4] = [
                (&[nt::OPEN], "open"),
                (&[nt::BLACK1, nt::BLACK2, nt::BLACK3], "black"),
                (&[nt::WHITE1, nt::WHITE2, nt::WHITE3], "white"),
                (&[BARRE1, BARRE2, BARRE3], "barre"),
            ];
            for (types, name) in lanes {
                for (m, ms) in [(nf::STRUM, "-strum"), (nf::HOPO, "-hopo"), (nf::TAP, "-tap")] {
                    for (sp, sps) in sp_flags {
                        for &ty in types {
                            list.push((ty, m | sp, format!("6fret-{name}{ms}{sps}")));
                        }
                    }
                }
            }
        }
        InstrumentType::FiveFret => {
            let lanes = [
                (nt::OPEN, "open"),
                (nt::GREEN, "green"),
                (nt::RED, "red"),
                (nt::YELLOW, "yellow"),
                (nt::BLUE, "blue"),
                (nt::ORANGE, "orange"),
            ];
            for (ty, name) in lanes {
                for (m, ms) in [(nf::STRUM, "-strum"), (nf::HOPO, "-hopo"), (nf::TAP, "-tap")] {
                    for (sp, sps) in sp_flags {
                        // Open notes can't be taps; the original maps them to the hopo artwork.
                        let (m2, ms2) = if ty == nt::OPEN && m == nf::TAP { (nf::HOPO, "-hopo") } else { (m, ms) };
                        list.push((ty, m2 | sp, format!("5fret-{name}{ms2}{sps}")));
                    }
                }
            }
        }
    }
    list
}

fn cache_dir() -> Option<PathBuf> {
    crate::settings::project_dirs().map(|d| d.cache_dir().join("preview-textures"))
}

async fn fetch(file: String) -> Result<(String, bytes::Bytes), String> {
    let path = cache_dir().map(|d| d.join(&file));
    if let Some(p) = &path
        && let Ok(b) = tokio::fs::read(p).await
    {
        return Ok((file, b.into()));
    }
    let bytes = crate::api::CLIENT
        .get(format!("{BASE_URL}/{file}"))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Failed to download {file}: {e}"))?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    if let Some(p) = path {
        if let Some(parent) = p.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = tokio::fs::write(p, &bytes).await;
    }
    Ok((file, bytes))
}

fn decode_anim(bytes: &[u8]) -> Result<AnimTexture, String> {
    use image::AnimationDecoder;
    let decoder = image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut frames = Vec::new();
    let mut frame_ends = Vec::new();
    let mut total = 0u32;
    let mut aspect = 1.0;
    if decoder.has_animation() {
        for f in decoder.into_frames() {
            let f = f.map_err(|e| e.to_string())?;
            let (n, d) = f.delay().numer_denom_ms();
            total += (n / d.max(1)).max(1);
            frame_ends.push(total);
            let buf = f.into_buffer();
            aspect = buf.width() as f32 / buf.height() as f32;
            frames.push(Mips::new(buf));
        }
    } else {
        let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
        aspect = img.width() as f32 / img.height() as f32;
        frames.push(Mips::new(img));
        frame_ends.push(0);
    }
    if frames.is_empty() {
        return Err("empty texture".into());
    }
    Ok(AnimTexture { frames, frame_ends, total_ms: total, aspect })
}

static CACHE: std::sync::LazyLock<std::sync::Mutex<HashMap<InstrumentType, Arc<TextureSet>>>> =
    std::sync::LazyLock::new(Default::default);

/// Already-decoded textures for this instrument type, if any.
pub fn cached(itype: InstrumentType) -> Option<Arc<TextureSet>> {
    CACHE.lock().ok()?.get(&itype).cloned()
}

/// Decoded textures for this instrument type (from memory, disk cache, or the network).
pub async fn get(itype: InstrumentType) -> Result<Arc<TextureSet>, String> {
    if let Some(t) = cached(itype) {
        return Ok(t);
    }
    let set = load(itype).await?;
    if let Ok(mut c) = CACHE.lock() {
        c.insert(itype, set.clone());
    }
    Ok(set)
}

async fn load(itype: InstrumentType) -> Result<Arc<TextureSet>, String> {
    let strikeline = match itype {
        InstrumentType::Drums => "preview-drums-strikeline.png",
        InstrumentType::SixFret => "preview-6fret-strikeline.png",
        InstrumentType::FiveFret => "preview-5fret-strikeline.png",
    };
    let list = note_texture_list(itype);
    let mut files: Vec<String> = list.iter().map(|(_, _, s)| format!("preview-{s}.webp")).collect();
    files.sort();
    files.dedup();
    files.push("preview-highway.png".into());
    files.push(strikeline.into());

    let fetched = futures::future::join_all(files.into_iter().map(fetch)).await;
    let mut raw: HashMap<String, bytes::Bytes> = HashMap::new();
    for r in fetched {
        let (name, b) = r?;
        raw.insert(name, b);
    }

    tokio::task::spawn_blocking(move || {
        // Decode distinct note files in parallel.
        let names: Vec<String> = raw.keys().filter(|k| k.ends_with(".webp")).cloned().collect();
        let decoded: HashMap<String, Arc<AnimTexture>> = std::thread::scope(|s| {
            let handles: Vec<_> = names
                .iter()
                .map(|n| {
                    let bytes = &raw[n];
                    s.spawn(move || decode_anim(bytes).map(|t| (n.clone(), Arc::new(t))))
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()?.ok()).collect()
        });

        let mut notes = HashMap::new();
        for (ty, flags, stem) in list {
            if let Some(t) = decoded.get(&format!("preview-{stem}.webp")) {
                notes.insert((ty, flags), t.clone());
            }
        }

        let hw = image::load_from_memory(&raw["preview-highway.png"]).map_err(|e| e.to_string())?.to_rgba8();
        let highway = mip_chain(hw)
            .into_iter()
            .map(|level| {
                let w = level.width();
                let rows = level
                    .rows()
                    .map(|row| {
                        let line: Vec<u8> = row.flat_map(|p| p.0).collect();
                        Handle::from_rgba(w, 3, line.repeat(3))
                    })
                    .collect();
                HighwayLevel { width: w, rows }
            })
            .collect();

        let sl = image::load_from_memory(&raw[strikeline]).map_err(|e| e.to_string())?.to_rgba8();
        let strikeline_aspect = sl.width() as f32 / sl.height() as f32;
        let strikeline = Mips::new(sl);

        let solid = crate::preview::SUSTAIN_HEXES
            .iter()
            .map(|&hex| {
                let px = [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 255];
                (hex, Handle::from_rgba(4, 4, px.repeat(16)))
            })
            .collect();

        let fade_px: Vec<u8> = (0..64u32)
            .flat_map(|i| {
                let t = i as f32 / 63.0;
                // Smooth ease so the fade has no visible edge.
                let a = 1.0 - t * t * (3.0 - 2.0 * t);
                [0, 0, 0, (a * 255.0).round() as u8]
            })
            .collect();
        let fade = Handle::from_rgba(1, 64, fade_px);

        Ok(Arc::new(TextureSet { solid, itype, highway, strikeline, strikeline_aspect, fade, notes }))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrinking_keeps_colours() {
        // Left half opaque red, right half fully transparent black: the red must stay red
        // (not darkened by the transparent pixels) and the image must not come out black.
        let img = image::RgbaImage::from_fn(16, 8, |x, _| if x < 8 { image::Rgba([200, 20, 20, 255]) } else { image::Rgba([0, 0, 0, 0]) });
        let small = shrink(&img, 8, 4);
        let p = small.get_pixel(1, 1);
        assert_eq!((p[0], p[1], p[2], p[3]), (200, 20, 20, 255));
        let edge = small.get_pixel(4, 1);
        assert!(edge[3] > 0 && edge[3] < 255 && edge[0] >= 195, "{edge:?}");
        assert_eq!(mip_chain(img).iter().map(|m| m.width()).collect::<Vec<_>>(), [16, 8]);
    }
}
