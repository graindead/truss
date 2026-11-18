//! Note and fret sprites for the colour editor's highway preview, tinted with a colour
//! profile.
//!
//! The sprites are Clone Hero's greyscale layers as published with the open-source Clone Hero
//! Color Editor (github.com/saadbruno/clone-hero-color-editor). They aren't shipped with
//! Truss: they're downloaded from that repository the first time the editor needs them and
//! cached. Each layer is multiplied by a profile colour and the layers are stacked, the way
//! that editor (and Clone Hero) colours them.

use crate::chart::{InstrumentType, nf, nt};
use crate::color_profile::Profile;
use crate::preview::{RNote, RenderTrack};
use crate::textures::{AnimTexture, Mips, SP_FLAG, Striker, TextureSet};
use iced::Color;
use iced::widget::image::Handle;
use image::RgbaImage;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const BASE_URL: &str = "https://raw.githubusercontent.com/saadbruno/clone-hero-color-editor/main/sprites";

/// Every sprite the preview uses (paths in the repository's sprites folder, without ".png").
fn files() -> Vec<String> {
    let mut v: Vec<String> = [
        "guitar/guitarNote_Base_Strum",
        "guitar/guitarNote_Base_Hopo",
        "guitar/guitarNote_Base_Tap",
        "guitar/guitarNote_Body_Strum",
        "guitar/guitarNote_Body_Tap",
        "guitar/guitarNote_Strum_Shine12",
        "guitar/guitarNote_SP_Base9",
        "guitar/guitarNote_SP_Body9",
        "guitar/guitarNote_SP_BodyTap9",
        "guitar/guitarNote_SP_StrumCap",
        "guitar/guitarNote_SP_HopoCap1",
        "guitar/guitarNote_SP_TapCap",
        "guitar/guitarNote_Open_Base",
        "guitar/guitarNote_Open_Body1",
        "guitar/guitarNote_Open_Shine1",
        "drums/drumNote_Tom_Regular_Base",
        "drums/drumNote_Tom_Regular_Body",
        "drums/drumNote_Tom_SP_Body1",
        "drums/drumNote_Tom_Shine4",
        "drums/drumNote_Cymbal_Regular_Base",
        "drums/drumNote_Cymbal_Regular_Body",
        "drums/drumNote_Cymbal_SP_Glow1",
        "drums/drumNote_Cymbal_Shine12",
        "drums/drumNote_Kick_Base",
        "drums/drumNote_Kick_Body",
        "drums/drumNote_Kick_Shine8",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for n in 1..=5 {
        for l in ["striker_GuitarUnderlay", "striker_Ring", "striker_RingCover", "striker_GuitarHead", "striker_HeadColor", "striker_HeadGlow"] {
            v.push(format!("guitar/{l}{n}"));
        }
    }
    for (ring, head) in DRUM_STRIKERS {
        for l in ["striker_Ring", "striker_RingCover", "striker_HeadColor", "striker_HeadGlow"] {
            v.push(format!("drums/{l}{ring}"));
        }
        v.push(format!("drums/drumStriker_Head{head}"));
        v.push(format!("drums/drumStriker_Underlay{head}"));
    }
    v
}

/// The drum pads' sprite numbers (ring and colour layers, head and underlay), red to green.
const DRUM_STRIKERS: [(u32, u32); 4] = [(1, 1), (2, 2), (4, 3), (5, 4)];

/// The decoded sprites, and the chart preview's highway textures they're drawn over.
pub struct Kit {
    sprites: HashMap<String, RgbaImage>,
    guitar: Arc<TextureSet>,
    drums: Arc<TextureSet>,
}

impl std::fmt::Debug for Kit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Kit({} sprites)", self.sprites.len())
    }
}

fn cache_dir() -> Option<PathBuf> {
    crate::settings::cache_dir().map(|d| d.join("ch-sprites"))
}

async fn fetch(name: String) -> Result<(String, Vec<u8>), String> {
    let path = cache_dir().map(|d| d.join(format!("{name}.png")));
    if let Some(p) = &path
        && let Ok(b) = tokio::fs::read(p).await
    {
        return Ok((name, b));
    }
    let bytes = crate::api::CLIENT
        .get(format!("{BASE_URL}/{name}.png"))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Couldn't download the note sprites ({name}): {e}"))?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    if let Some(p) = path {
        if let Some(parent) = p.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = tokio::fs::write(p, &bytes).await;
    }
    Ok((name, bytes.to_vec()))
}

/// Downloads (or reads from the cache) the sprites and the highway textures.
pub async fn load() -> Result<Arc<Kit>, String> {
    let fetched = futures::future::join_all(files().into_iter().map(fetch)).await;
    let raw: Vec<(String, Vec<u8>)> = fetched.into_iter().collect::<Result<_, _>>()?;
    let guitar = crate::textures::get(InstrumentType::FiveFret).await?;
    let drums = crate::textures::get(InstrumentType::Drums).await?;
    tokio::task::spawn_blocking(move || {
        let mut sprites = HashMap::new();
        for (name, b) in raw {
            let img = image::load_from_memory(&b).map_err(|e| format!("{name}: {e}"))?.to_rgba8();
            sprites.insert(name, img);
        }
        Ok(Arc::new(Kit { sprites, guitar, drums }))
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------- Tinting and stacking ----------

fn rgb(c: Color) -> [f32; 3] {
    [c.r, c.g, c.b]
}

/// Layers of one picture: (sprite, tint or none, y offset), bottom first.
struct Stack<'a> {
    kit: &'a Kit,
    out: RgbaImage,
}

impl<'a> Stack<'a> {
    fn new(kit: &'a Kit, w: u32, h: u32) -> Self {
        Stack { kit, out: RgbaImage::new(w, h) }
    }

    /// Draws a sprite over the picture, multiplied by `tint` (as Clone Hero's colours are),
    /// scaled to the picture's width and moved down by `dy` pixels.
    fn layer(mut self, name: &str, tint: Option<Color>, dy: i64) -> Self {
        let Some(src) = self.kit.sprites.get(name) else { return self };
        let src = if src.width() != self.out.width() {
            let h = (src.height() as f32 * self.out.width() as f32 / src.width() as f32).round().max(1.0) as u32;
            std::borrow::Cow::Owned(image::imageops::resize(src, self.out.width(), h, image::imageops::FilterType::Triangle))
        } else {
            std::borrow::Cow::Borrowed(src)
        };
        let t = tint.map(rgb).unwrap_or([1.0; 3]);
        for (x, y, p) in src.enumerate_pixels() {
            let ty = y as i64 + dy;
            if ty < 0 || ty >= self.out.height() as i64 {
                continue;
            }
            let a = p[3] as f32 / 255.0;
            if a <= 0.0 {
                continue;
            }
            let d = self.out.get_pixel_mut(x, ty as u32);
            let da = d[3] as f32 / 255.0;
            let oa = a + da * (1.0 - a);
            for i in 0..3 {
                let s = p[i] as f32 * t[i];
                let v = (s * a + d[i] as f32 * da * (1.0 - a)) / oa.max(1e-6);
                d[i] = v.round().clamp(0.0, 255.0) as u8;
            }
            d[3] = (oa * 255.0).round() as u8;
        }
        self
    }

    fn done(self) -> RgbaImage {
        self.out
    }
}

/// Note, fret and sustain pictures, before they're turned into textures.
pub struct Built {
    pub notes: HashMap<(u8, u32), RgbaImage>,
    /// (note type, resting, lit, lit by an open note or kick, width on the highway).
    pub strikers: Vec<(u8, [RgbaImage; 3], f32)>,
    pub sustains: HashMap<(u8, bool), Color>,
}

impl Built {
    fn textures(self, base: &TextureSet) -> TextureSet {
        let notes = self.notes.into_iter().map(|(k, img)| (k, still(img))).collect();
        let strikers = self
            .strikers
            .into_iter()
            .map(|(ty, [idle, lit, lit_open], width)| {
                let aspect = idle.width() as f32 / idle.height() as f32;
                Striker { ty, idle: Mips::new(idle), lit: Mips::new(lit), lit_open: Mips::new(lit_open), aspect, width }
            })
            .collect();
        let sustains = self.sustains.into_iter().map(|(k, c)| (k, solid(c))).collect();
        with_base(base, notes, strikers, sustains)
    }
}

fn still(img: RgbaImage) -> Arc<AnimTexture> {
    let aspect = img.width() as f32 / img.height() as f32;
    Arc::new(AnimTexture { frames: vec![Mips::new(img)], frame_ends: vec![0], total_ms: 0, aspect })
}

/// Trims fully transparent rows off the top and bottom (the kick sprites have a lot).
fn trim_rows(img: RgbaImage) -> RgbaImage {
    let rows: Vec<u32> = (0..img.height()).filter(|&y| (0..img.width()).any(|x| img.get_pixel(x, y)[3] > 8)).collect();
    match (rows.first(), rows.last()) {
        (Some(&a), Some(&b)) => image::imageops::crop_imm(&img, 0, a, img.width(), b - a + 1).to_image(),
        _ => img,
    }
}

fn solid(c: Color) -> Handle {
    let [r, g, b, _] = c.into_rgba8();
    Handle::from_rgba(4, 4, [r, g, b, 255].repeat(16))
}

/// Profile colours, with Clone Hero's defaults for keys the profile doesn't set.
struct Colors<'a> {
    p: &'a Profile,
    d: &'a Profile,
    section: &'a str,
}

impl Colors<'_> {
    fn get(&self, key: &str) -> Color {
        self.p.color(self.section, key).or_else(|| self.d.color(self.section, key)).unwrap_or(Color::WHITE)
    }
}

const GUITAR_LANES: [(u8, &str); 5] = [(nt::GREEN, "green"), (nt::RED, "red"), (nt::YELLOW, "yellow"), (nt::BLUE, "blue"), (nt::ORANGE, "orange")];
const DRUM_LANES: [(u8, &str); 4] = [(nt::RED_DRUM, "red"), (nt::YELLOW_DRUM, "yellow"), (nt::BLUE_DRUM, "blue"), (nt::GREEN_DRUM, "green")];

/// Textures for the guitar highway in a profile's colours. `sp_active`: draw the notes as
/// they look while star power is on.
pub fn guitar(kit: &Kit, p: &Profile, d: &Profile, sp_active: bool) -> TextureSet {
    guitar_images(kit, p, d, sp_active).textures(&kit.guitar)
}

fn guitar_images(kit: &Kit, p: &Profile, d: &Profile, sp_active: bool) -> Built {
    let c = Colors { p, d, section: "guitar" };
    let mut notes = HashMap::new();
    // (note key, detail key) for a lane's note, plain or in a star power phrase.
    let keys = |lane: &str, sp: bool| -> (String, String) {
        match (sp, sp_active) {
            (true, true) => ("note_sp_phrase_active".into(), "note_anim_sp_phrase_active".into()),
            (true, false) => ("note_sp_phrase".into(), "note_anim_sp_phrase".into()),
            (false, true) => ("note_sp_active".into(), "note_anim_sp_active".into()),
            (false, false) => (format!("note_{lane}"), format!("note_anim_{lane}")),
        }
    };
    for (ty, lane) in GUITAR_LANES {
        for sp in [false, true] {
            let (body, detail) = keys(lane, sp);
            let (body, detail) = (c.get(&body), c.get(&detail));
            let flag = if sp { SP_FLAG } else { 0 };
            for (kind, base) in [(nf::STRUM, "guitarNote_Base_Strum"), (nf::HOPO, "guitarNote_Base_Hopo"), (nf::TAP, "guitarNote_Base_Tap")] {
                let img = if sp {
                    let (glow, cap) = match kind {
                        nf::TAP => ("guitarNote_SP_BodyTap9", "guitarNote_SP_TapCap"),
                        nf::HOPO => ("guitarNote_SP_Body9", "guitarNote_SP_HopoCap1"),
                        _ => ("guitarNote_SP_Body9", "guitarNote_SP_StrumCap"),
                    };
                    Stack::new(kit, 128, 64)
                        .layer("guitar/guitarNote_SP_Base9", Some(detail), 0)
                        .layer(&format!("guitar/{glow}"), Some(body), 0)
                        .layer(&format!("guitar/{cap}"), None, 0)
                        .done()
                } else {
                    let body_sprite = if kind == nf::TAP { "guitar/guitarNote_Body_Tap" } else { "guitar/guitarNote_Body_Strum" };
                    Stack::new(kit, 128, 64)
                        .layer(&format!("guitar/{base}"), None, 0)
                        .layer(body_sprite, Some(body), 0)
                        .layer("guitar/guitarNote_Strum_Shine12", Some(detail), -56)
                        .done()
                };
                notes.insert((ty, kind | flag), img);
            }
        }
    }
    // Open notes.
    for sp in [false, true] {
        let (body, detail) = if sp || sp_active { keys("open", sp) } else { ("note_open".to_string(), "note_anim_open".to_string()) };
        let img = Stack::new(kit, 410, 51)
            .layer("guitar/guitarNote_Open_Base", None, 0)
            .layer("guitar/guitarNote_Open_Body1", Some(c.get(&body)), 0)
            .layer("guitar/guitarNote_Open_Shine1", Some(c.get(&detail)), -5)
            .done();
        let flag = if sp { SP_FLAG } else { 0 };
        for kind in [nf::STRUM, nf::HOPO] {
            notes.insert((nt::OPEN, kind | flag), img.clone());
        }
    }
    // Frets.
    let strikers = GUITAR_LANES
        .iter()
        .enumerate()
        .map(|(i, (ty, lane))| {
            let n = i + 1;
            // The coloured ring goes over the head and the silver cover, so it shows all the
            // way round, as in the game.
            let fret = |light: Option<Color>| {
                let mut s = Stack::new(kit, 128, 64)
                    .layer(&format!("guitar/striker_GuitarUnderlay{n}"), None, 0)
                    .layer(&format!("guitar/striker_GuitarHead{n}"), Some(c.get(&format!("striker_base_{lane}"))), 0)
                    .layer(&format!("guitar/striker_RingCover{n}"), None, 0)
                    .layer(&format!("guitar/striker_Ring{n}"), Some(c.get(&format!("striker_cover_{lane}"))), 0)
                    .layer(&format!("guitar/striker_HeadColor{n}"), Some(c.get(&format!("striker_head_cover_{lane}"))), 0);
                if let Some(l) = light {
                    s = s.layer(&format!("guitar/striker_HeadGlow{n}"), Some(l), 0);
                }
                s.done()
            };
            (*ty, [fret(None), fret(Some(c.get(&format!("striker_head_light_{lane}")))), fret(Some(c.get("striker_head_light_open")))], 0.19)
        })
        .collect();
    let mut sustains = HashMap::new();
    for (ty, lane) in GUITAR_LANES.iter().copied().chain([(nt::OPEN, "open")]) {
        let plain = if sp_active { c.get("sustain_sp_active") } else { c.get(&format!("sustain_{lane}")) };
        let sp = if sp_active { c.get("sustain_sp_phrase_active") } else { c.get("sustain_sp_phrase") };
        sustains.insert((ty, false), plain);
        sustains.insert((ty, true), sp);
    }
    Built { notes, strikers, sustains }
}

/// Textures for the drum highway in a profile's colours.
pub fn drums(kit: &Kit, p: &Profile, d: &Profile, sp_active: bool) -> TextureSet {
    drum_images(kit, p, d, sp_active).textures(&kit.drums)
}

fn drum_images(kit: &Kit, p: &Profile, d: &Profile, sp_active: bool) -> Built {
    let c = Colors { p, d, section: "drums" };
    let mut notes = HashMap::new();
    let pick = |plain: String, sp: bool, kind: &str| -> String {
        match (sp, sp_active) {
            (true, true) => format!("{kind}_sp_phrase_active"),
            (true, false) => format!("{kind}_sp_phrase"),
            (false, true) => format!("{kind}_sp_active"),
            (false, false) => plain,
        }
    };
    for (ty, lane) in DRUM_LANES {
        for sp in [false, true] {
            let flag = if sp { SP_FLAG } else { 0 };
            let tom_body = c.get(&pick(format!("tom_{lane}"), sp, "tom"));
            let tom_detail = c.get(&pick(format!("tom_anim_{lane}"), sp, "tom_anim"));
            let body_sprite = if sp { "drums/drumNote_Tom_SP_Body1" } else { "drums/drumNote_Tom_Regular_Body" };
            let tom = Stack::new(kit, 128, 64)
                .layer("drums/drumNote_Tom_Regular_Base", None, 0)
                .layer(body_sprite, Some(tom_body), 0)
                .layer("drums/drumNote_Tom_Shine4", Some(tom_detail), 0)
                .done();
            notes.insert((ty, nf::TOM | flag), tom);
            let cym_body = c.get(&pick(format!("cym_{lane}"), sp, "cym"));
            let cym_detail = c.get(&pick(format!("cym_anim_{lane}"), sp, "cym_anim"));
            let mut s = Stack::new(kit, 128, 64).layer("drums/drumNote_Cymbal_Regular_Base", None, 0);
            if sp {
                s = s.layer("drums/drumNote_Cymbal_SP_Glow1", Some(cym_body), 0);
            }
            let cym = s.layer("drums/drumNote_Cymbal_Regular_Body", Some(cym_body), 0).layer("drums/drumNote_Cymbal_Shine12", Some(cym_detail), 0).done();
            notes.insert((ty, nf::CYMBAL | flag), cym);
        }
    }
    for sp in [false, true] {
        let flag = if sp { SP_FLAG } else { 0 };
        let body = c.get(&pick("note_kick".into(), sp, "note_kick"));
        let detail = c.get(&pick("note_anim_kick".into(), sp, "note_anim_kick"));
        let kick = trim_rows(
            Stack::new(kit, 1024, 128)
                .layer("drums/drumNote_Kick_Base", None, 0)
                .layer("drums/drumNote_Kick_Body", Some(body), 0)
                .layer("drums/drumNote_Kick_Shine8", Some(detail), 0)
                .done(),
        );
        notes.insert((nt::KICK, flag), kick.clone());
        notes.insert((nt::KICK, nf::DOUBLE_KICK | flag), kick);
    }
    let strikers = DRUM_LANES
        .iter()
        .zip(DRUM_STRIKERS)
        .map(|((ty, lane), (ring, head))| {
            let pad = |light: Option<Color>| {
                let mut s = Stack::new(kit, 128, 64)
                    .layer(&format!("drums/drumStriker_Underlay{head}"), None, 0)
                    .layer(&format!("drums/drumStriker_Head{head}"), Some(c.get(&format!("drums_striker_base_{lane}"))), 0)
                    .layer(&format!("drums/striker_RingCover{ring}"), None, 0)
                    .layer(&format!("drums/striker_Ring{ring}"), Some(c.get(&format!("drums_striker_cover_{lane}"))), 0)
                    .layer(&format!("drums/striker_HeadColor{ring}"), Some(c.get(&format!("drums_striker_head_cover_{lane}"))), 0);
                if let Some(l) = light {
                    s = s.layer(&format!("drums/striker_HeadGlow{ring}"), Some(l), 0);
                }
                s.done()
            };
            (*ty, [pad(None), pad(Some(c.get(&format!("drums_striker_head_light_{lane}")))), pad(Some(c.get("drums_striker_head_light_kick")))], 0.21)
        })
        .collect();
    Built { notes, strikers, sustains: HashMap::new() }
}

/// The chart preview's highway with these notes, frets and sustains.
fn with_base(base: &TextureSet, notes: HashMap<(u8, u32), Arc<AnimTexture>>, strikers: Vec<Striker>, sustains: HashMap<(u8, bool), Handle>) -> TextureSet {
    TextureSet {
        solid: base.solid.clone(),
        itype: base.itype,
        highway: base.highway.clone(),
        strikeline: base.strikeline.clone(),
        fade: base.fade.clone(),
        strikeline_aspect: base.strikeline_aspect,
        notes,
        strikers,
        sustains,
    }
}

// ---------- The demo chart ----------

/// How long the demo loops for before starting again.
pub const DEMO_MS: f64 = 600_000.0;

/// A made-up chart that shows every kind of note: (guitar, drums).
pub fn demo_tracks() -> (RenderTrack, RenderTrack) {
    let mut g = Vec::new();
    let mut d = Vec::new();
    let note = |ms: f64, len: f64, ty: u8, flags: u32| RNote { ms, ms_len: len, ty, flags };
    // Guitar: an 8-second phrase, every other one in star power.
    let mut bar = 0.0;
    let mut i = 0;
    while bar < DEMO_MS {
        let sp = if i % 2 == 1 { SP_FLAG } else { 0 };
        let s = nf::STRUM | sp;
        let h = nf::HOPO | sp;
        let t = nf::TAP | sp;
        g.extend([
            note(bar, 600.0, nt::GREEN, s),
            note(bar + 750.0, 0.0, nt::RED, s),
            note(bar + 1000.0, 0.0, nt::YELLOW, h),
            note(bar + 1250.0, 0.0, nt::BLUE, h),
            note(bar + 1500.0, 0.0, nt::ORANGE, h),
            note(bar + 2000.0, 0.0, nt::GREEN, s),
            note(bar + 2000.0, 0.0, nt::YELLOW, s),
            note(bar + 2500.0, 700.0, nt::OPEN, s),
            note(bar + 3500.0, 0.0, nt::RED, s),
            note(bar + 3500.0, 0.0, nt::BLUE, s),
            note(bar + 4000.0, 0.0, nt::YELLOW, t),
            note(bar + 4250.0, 0.0, nt::BLUE, t),
            note(bar + 4500.0, 0.0, nt::ORANGE, t),
            note(bar + 4750.0, 0.0, nt::BLUE, t),
            note(bar + 5250.0, 900.0, nt::ORANGE, s),
            note(bar + 6500.0, 0.0, nt::GREEN, s),
            note(bar + 6750.0, 0.0, nt::RED, h),
            note(bar + 7000.0, 500.0, nt::YELLOW, s),
            note(bar + 7000.0, 500.0, nt::BLUE, s),
        ]);
        bar += 8000.0;
        i += 1;
    }
    // Drums: a 4-second groove with a tom fill, every other one in star power.
    let mut bar = 0.0;
    let mut i = 0;
    while bar < DEMO_MS {
        let sp = if i % 2 == 1 { SP_FLAG } else { 0 };
        let cym = nf::CYMBAL | sp;
        let tom = nf::TOM | sp;
        d.push(note(bar, 0.0, nt::GREEN_DRUM, cym));
        for k in [0.0, 1000.0, 1250.0, 2000.0] {
            d.push(note(bar + k, 0.0, nt::KICK, sp));
        }
        for k in [500.0, 1500.0, 2500.0] {
            d.push(note(bar + k, 0.0, nt::RED_DRUM, tom));
        }
        for k in (1..12).map(|j| j as f64 * 250.0) {
            d.push(note(bar + k, 0.0, nt::YELLOW_DRUM, cym));
        }
        d.push(note(bar + 2250.0, 0.0, nt::BLUE_DRUM, cym));
        d.push(note(bar + 3000.0, 0.0, nt::YELLOW_DRUM, tom));
        d.push(note(bar + 3250.0, 0.0, nt::BLUE_DRUM, tom));
        d.push(note(bar + 3500.0, 0.0, nt::GREEN_DRUM, tom));
        d.push(note(bar + 3750.0, 0.0, nt::KICK, sp));
        bar += 4000.0;
        i += 1;
    }
    let sort = |v: &mut Vec<RNote>| v.sort_by(|a, b| a.ms.total_cmp(&b.ms));
    sort(&mut g);
    sort(&mut d);
    let max = |v: &Vec<RNote>| v.iter().map(|n| n.ms_len).fold(0.0, f64::max);
    (
        RenderTrack { itype: InstrumentType::FiveFret, max_len: max(&g), notes: g },
        RenderTrack { itype: InstrumentType::Drums, max_len: max(&d), notes: d },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Network (first run): the sprites download, and a profile's colours end up in them.
    #[test]
    #[ignore]
    fn tints_sprites() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let kit = rt.block_on(load()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(kit.sprites.len(), files().len());
        let d = Profile::defaults();
        let mut p = d.clone();
        p.set("guitar", "note_green", "#FF00FF");
        let g = guitar(&kit, &p, &d, false);
        assert!(g.notes.contains_key(&(nt::GREEN, nf::STRUM)) && g.notes.contains_key(&(nt::OPEN, nf::HOPO | SP_FLAG)));
        assert_eq!(g.strikers.len(), 5);
        let dr = drums(&kit, &d, &d, true);
        assert!(dr.notes.contains_key(&(nt::KICK, 0)) && dr.strikers.len() == 4);
        // The green body really is magenta now: a body pixel has red and blue, no green.
        let img = Stack::new(&kit, 128, 64).layer("guitar/guitarNote_Body_Strum", Some(Color::from_rgb(1.0, 0.0, 1.0)), 0).done();
        let p = img.pixels().filter(|p| p[3] > 200).max_by_key(|p| p[0] as u32 + p[2] as u32).unwrap();
        assert!(p[1] == 0 && p[0] > 100 && p[2] > 100, "{p:?}");
        // A sheet to look at, when asked: notes, star power notes, frets (resting and lit), drums.
        if let Some(out) = std::env::var_os("TRUSS_SPRITE_SHEET") {
            let gi = guitar_images(&kit, &Profile::defaults(), &d, false);
            let di = drum_images(&kit, &Profile::defaults(), &d, false);
            let mut sheet = RgbaImage::from_pixel(128 * 5, 64 * 6 + 60, image::Rgba([30, 30, 36, 255]));
            let mut put = |img: &RgbaImage, x: u32, y: u32| image::imageops::overlay(&mut sheet, img, x as i64, y as i64);
            for (i, (t, _)) in GUITAR_LANES.iter().enumerate() {
                put(&gi.notes[&(*t, nf::STRUM)], i as u32 * 128, 0);
                put(&gi.notes[&(*t, nf::HOPO | SP_FLAG)], i as u32 * 128, 64);
                put(&gi.strikers[i].1[0], i as u32 * 128, 128);
                put(&gi.strikers[i].1[1], i as u32 * 128, 192);
            }
            for (i, (t, _)) in DRUM_LANES.iter().enumerate() {
                put(&di.notes[&(*t, nf::TOM)], i as u32 * 128, 256);
                put(&di.notes[&(*t, nf::CYMBAL)], i as u32 * 128, 320);
            }
            let open = image::imageops::resize(&gi.notes[&(nt::OPEN, nf::STRUM)], 320, 40, image::imageops::FilterType::Triangle);
            put(&open, 0, 390);
            let kick = &di.notes[&(nt::KICK, 0)];
            let kick = image::imageops::resize(kick, 320, (kick.height() * 320 / kick.width()).max(1), image::imageops::FilterType::Triangle);
            put(&kick, 320, 400);
            sheet.save(std::path::PathBuf::from(out)).unwrap();
        }
    }

    #[test]
    fn demo_is_sorted_and_long() {
        let (g, d) = demo_tracks();
        for t in [&g, &d] {
            assert!(t.notes.windows(2).all(|w| w[0].ms <= w[1].ms));
            assert!(t.notes.last().unwrap().ms > DEMO_MS - 10_000.0);
        }
        assert!(g.notes.iter().any(|n| n.ty == nt::OPEN) && d.notes.iter().any(|n| n.ty == nt::KICK));
    }
}
