//! Note and fret sprites for the colour editor's highway preview, tinted with a colour
//! profile.
//!
//! The sprites are Clone Hero's greyscale layers. When Clone Hero is installed they're read
//! from its own files on this computer (see unity_assets.rs), so the preview shows the game's
//! current art; otherwise they're the older ones published with the open-source Clone Hero
//! Color Editor (github.com/saadbruno/clone-hero-color-editor), downloaded the first time the
//! editor needs them and cached. Neither is shipped with Truss. Each layer is multiplied by a
//! profile colour and the layers are stacked, the way Clone Hero colours them.

use crate::chart::{InstrumentType, nf, nt};
use crate::color_profile::Profile;
use crate::preview::{RNote, RenderTrack};
use crate::textures::{AnimTexture, Flames, Mips, SP_FLAG, Striker, TextureSet};
use iced::Color;
use iced::widget::image::Handle;
use image::RgbaImage;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const BASE_URL: &str = "https://raw.githubusercontent.com/saadbruno/clone-hero-color-editor/main/sprites";

/// Every sprite the preview uses (paths in the repository's sprites folder, without ".png").
pub fn files() -> Vec<String> {
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
        "drums/drumNote_Kick_Hitflame",
        "guitar/guitarNote_Open_HitFlame",
        "other/note_HitFlame3",
        "other/note_Particle",
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
    /// The Clone Hero folder the sprites were read from (none: downloaded).
    pub game: Option<PathBuf>,
    /// More of the game's own pictures (6-fret notes, the combo meter, star power bar, hit
    /// effects), greyscale to be tinted, by their names in the game; only when read from it.
    pub art: HashMap<String, RgbaImage>,
    guitar: Arc<TextureSet>,
    drums: Arc<TextureSet>,
}

impl Kit {
    fn scale(&self) -> u32 {
        if self.game.is_some() { 2 } else { 1 }
    }
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

/// Each sprite here and where it is in Clone Hero's files: the sprites stacked to make it,
/// and whether it's mirrored. Clone Hero draws three strike rings (left, middle, right) and
/// mirrors them for the right-hand side.
fn game_names() -> Vec<(String, Vec<String>, bool)> {
    let mut v: Vec<(String, Vec<String>, bool)> = [
        ("guitar/guitarNote_Base_Strum", "spr_newnotes_strip4_0"),
        ("guitar/guitarNote_Base_Hopo", "spr_newnotes_strip4_1"),
        ("guitar/guitarNote_Base_Tap", "spr_newnotes_strip4_4"),
        ("guitar/guitarNote_Body_Strum", "spr_newnotes_strip4_2"),
        ("guitar/guitarNote_Body_Tap", "spr_newnotes_strip4_3"),
        ("guitar/guitarNote_Strum_Shine12", "spr_note_anim_strip16_11"),
        ("guitar/guitarNote_SP_Base9", "spr_star_notes_strip4_8"),
        ("guitar/guitarNote_SP_Body9", "spr_star_notes_strip1_8"),
        ("guitar/guitarNote_SP_BodyTap9", "spr_star_notes_strip2_8"),
        ("guitar/guitarNote_SP_StrumCap", "spr_star_notes_cap_strip4_0"),
        ("guitar/guitarNote_SP_HopoCap1", "spr_star_notes_cap_strip4_1"),
        ("guitar/guitarNote_SP_TapCap", "spr_star_notes_cap_strip4_4"),
        ("guitar/guitarNote_Open_Base", "spr_open_notes_strip5_0"),
        ("guitar/guitarNote_Open_Body1", "spr_open_notes_strip5_1"),
        ("guitar/guitarNote_Open_Shine1", "highlight_0"),
        ("guitar/guitarNote_Open_HitFlame", "open_flame"),
        ("drums/drumNote_Tom_Regular_Base", "head"),
        ("drums/drumNote_Tom_Regular_Body", "body"),
        ("drums/drumNote_Tom_SP_Body1", "body0001"),
        ("drums/drumNote_Tom_Shine4", "shine04"),
        ("drums/drumNote_Cymbal_Regular_Base", "cap"),
        ("drums/drumNote_Cymbal_Regular_Body", "body#2"),
        ("drums/drumNote_Cymbal_SP_Glow1", "starcym_01"),
        ("drums/drumNote_Cymbal_Shine12", "shine11"),
        ("drums/drumNote_Kick_Base", "kickbase"),
        ("drums/drumNote_Kick_Body", "Kickbody"),
        ("drums/drumNote_Kick_Shine8", "Kickshine08"),
        ("drums/drumNote_Kick_Hitflame", "open_flame_drum"),
        ("other/note_HitFlame3", "hitflame_anim_16_4"),
        ("other/note_Particle", "partical"),
    ]
    .iter()
    .map(|(a, b)| (a.to_string(), vec![b.to_string()], false))
    .collect();
    // Outside in: the edge ring, its neighbour, the middle one; mirrored right of the middle.
    let ring = |lane: usize, of: usize| -> (usize, bool) {
        let from_edge = lane.min(of - 1 - lane);
        ((from_edge + 1).min(3), lane * 2 + 1 > of)
    };
    for n in 1..=5usize {
        let (g, flip) = ring(n - 1, 5);
        for (here, game) in [
            ("striker_GuitarUnderlay", vec![format!("pick_{g}"), format!("base_{g}")]),
            ("striker_Ring", vec![format!("cover_{g}")]),
            ("striker_RingCover", vec![format!("half_cover_{g}")]),
            ("striker_GuitarHead", vec![format!("head_{g}")]),
            ("striker_HeadColor", vec![format!("inner_color_{g}")]),
            ("striker_HeadGlow", vec![format!("head_light_{g}")]),
        ] {
            v.push((format!("guitar/{here}{n}"), game, flip));
        }
    }
    for (lane, (ring_n, head_n)) in DRUM_STRIKERS.iter().enumerate() {
        let (g, flip) = ring(lane, 4);
        for (here, game) in [
            (format!("drumStriker_Underlay{head_n}"), vec![format!("pick_{g}"), format!("base_{g}")]),
            (format!("striker_Ring{ring_n}"), vec![format!("cover_{g}")]),
            (format!("striker_RingCover{ring_n}"), vec![format!("half_cover_{g}")]),
            (format!("drumStriker_Head{head_n}"), vec![format!("head_{g}")]),
            (format!("striker_HeadColor{ring_n}"), vec![format!("inner_color_{g}")]),
            (format!("striker_HeadGlow{ring_n}"), vec![format!("head_light_{g}")]),
        ] {
            v.push((format!("drums/{here}"), game, flip));
        }
    }
    v
}

/// The Clone Hero folder chosen in Settings (none: found automatically).
static INSTALL: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

pub fn set_install(p: Option<PathBuf>) {
    *INSTALL.lock().unwrap() = p;
}

/// The player's song folders (from Settings): Clone Hero's own Songs folder sits in its install.
static SONG_FOLDERS: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

pub fn set_song_folders(v: Vec<PathBuf>) {
    *SONG_FOLDERS.lock().unwrap() = v;
}

/// The Clone Hero folder chosen in Settings, if it is one.
pub fn chosen_install() -> Option<PathBuf> {
    INSTALL.lock().unwrap().clone().filter(|p| crate::unity_assets::data_folder(p).is_some())
}

/// Clone Hero's install folder: the one chosen in Settings, else above the colour profiles, or
/// in the usual places.
pub fn find_install() -> Option<PathBuf> {
    if let Some(p) = chosen_install() {
        return Some(p);
    }
    let mut tries: Vec<PathBuf> = Vec::new();
    // Above a song folder in its usual place (Clone Hero\Songs), or a couple of levels up.
    for f in SONG_FOLDERS.lock().unwrap().iter() {
        tries.extend(f.ancestors().skip(1).take(3).map(PathBuf::from));
    }
    if let Some(colors) = crate::color_profile::folder() {
        tries.extend(colors.ancestors().skip(1).take(4).map(PathBuf::from));
    }
    if let Some(u) = directories::UserDirs::new() {
        if let Some(d) = u.document_dir() {
            tries.push(d.join("Clone Hero"));
        }
        tries.push(u.home_dir().join(r"OneDrive\Documents\Clone Hero"));
    }
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(p) = std::env::var_os(var) {
            tries.push(PathBuf::from(p).join("Clone Hero"));
        }
    }
    tries.into_iter().find(|p| crate::unity_assets::data_folder(p).is_some())
}

/// One layer of a note or fret on its own (untinted): `sprite` drawn into a `w` by `h`
/// picture the way the highway's are stacked (scaled to the width, moved down `dy`).
pub fn layer_image(kit: &Kit, sprite: &str, w: u32, h: u32, dy: i64) -> Option<RgbaImage> {
    kit.sprites.get(sprite)?;
    Some(Stack::new(kit, w, h).layer(sprite, None, dy).done())
}

/// The game's pictures for the 6-fret and other colours' previews (see `Kit::art`).
pub const ART: &[&str] = &[
    "6fret_strikeline_base",
    "6fret_strikeline_buttons_0",
    "6fret_strikeline_buttons_1",
    "6fret_strikeline_buttons_2",
    "up-body",
    "up-ring",
    "up-overlay-glow",
    "up-inside-overlay",
    "down-body",
    "down-ring",
    "down-overlay-glow",
    "down-inside-overlay",
    "bar-chord-body",
    "bar-chord-ring",
    "bar-chord-overlay-glow",
    "multi-bg",
    "multi-overlay",
    "multi-x",
    "multi-font-overlay_2",
    "multi-font-overlay_3",
    "multi-font-overlay_4",
    "multi-font-overlay_5",
    "multi-font-overlay_9",
    "power_meter_basepng",
    "power_meter_fill",
    "arrow_glow",
    "lightning_merged_16_final_0",
    "SP_highlight_0",
    "sp_color_anim_0",
    "activationfade2",
    "pulse",
    "Bolt",
    "hitflame_anim_16_5",
    "hit_spark_5",
    "sustain_spark_anim_color_3",
    "open_flame",
    "open_flame_drum",
    "score_star_grey_back",
    "score_star_outline_grey",
    "sidebar",
    "beatline",
    "spr_sustain_strip6_0",
    "spr_sustain_strip6_1",
    "open_sustain_glow",
];

/// The game's picture as the preview uses it: greyscale (so a colour tints it), the flame
/// strips cut to their first frame, and the star power bar's pieces turned to lie flat.
fn art_image(name: &str, mut img: RgbaImage) -> RgbaImage {
    for p in img.pixels_mut() {
        let l = (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32).round() as u8;
        p[0] = l;
        p[1] = l;
        p[2] = l;
    }
    let frames = match name {
        "open_flame" => 5,
        "open_flame_drum" => 4,
        _ => 1,
    };
    if frames > 1 {
        img = image::imageops::crop_imm(&img, 0, 0, img.width() / frames, img.height()).to_image();
    }
    if matches!(name, "power_meter_basepng" | "power_meter_fill" | "lightning_merged_16_final_0" | "activationfade2") {
        img = image::imageops::rotate90(&img);
    }
    if name == "arrow_glow" {
        img = image::imageops::rotate90(&img);
    }
    // The star is dark grey, which would hide its colour: brightest part made white.
    if name == "score_star_grey_back" {
        let top = img.pixels().filter(|p| p[3] > 128).map(|p| p[0]).max().unwrap_or(255).max(1) as u32;
        for p in img.pixels_mut() {
            let v = (p[0] as u32 * 255 / top).min(255) as u8;
            p[0] = v;
            p[1] = v;
            p[2] = v;
        }
    }
    img
}

/// Reads the sprites out of Clone Hero's own files, named as the downloaded ones are, and the
/// pictures for the other previews.
fn from_game(install: &std::path::Path) -> Result<(HashMap<String, RgbaImage>, HashMap<String, RgbaImage>), String> {
    let data = crate::unity_assets::data_folder(install).ok_or("not a Clone Hero folder")?;
    let s = crate::unity_assets::Sprites::open(&[data.join("resources.assets")])?;
    let map = game_names();
    let anims = anim_names();
    let mut wanted: Vec<&str> = map.iter().flat_map(|(_, g, _)| g.iter().map(String::as_str)).collect();
    wanted.extend_from_slice(ART);
    wanted.extend(anims.iter().map(String::as_str));
    let got = s.get(&wanted)?;
    let art = ART
        .iter()
        .copied()
        .chain(anims.iter().map(String::as_str))
        .filter_map(|n| Some((n.to_string(), art_image(n, got.get(n)?.clone()))))
        .collect();
    let mut out = HashMap::new();
    for (here, game, flip) in &map {
        let mut img: Option<RgbaImage> = None;
        for g in game {
            let l = got.get(g).ok_or_else(|| format!("Clone Hero's files have no sprite \"{g}\""))?;
            match &mut img {
                None => img = Some(l.clone()),
                Some(i) => image::imageops::overlay(i, l, 0, 0),
            }
        }
        let mut img = img.ok_or("no sprite")?;
        if *flip {
            image::imageops::flip_horizontal_in_place(&mut img);
        }
        if game[0] == "partical" {
            // Drawn added on, so it has no transparency of its own: its brightness is.
            for p in img.pixels_mut() {
                let a = p[0].max(p[1]).max(p[2]);
                *p = image::Rgba([255, 255, 255, a]);
            }
        }
        out.insert(here.clone(), img);
    }
    Ok((out, art))
}

// ---------- The game's pictures, read once and kept ----------

/// Bumped when what's read from the game (or how it's prepared) changes, so it's read again.
const GAME_CACHE_VERSION: u32 = 1;

fn game_cache_dir() -> Option<PathBuf> {
    crate::settings::cache_dir().map(|d| d.join("ch-game"))
}

/// What the kept pictures were read from: the folder, its resources.assets' size and date, and
/// this version of Truss's reading of them. Different (an update, another folder): read again.
fn game_stamp(install: &std::path::Path) -> Option<String> {
    let data = crate::unity_assets::data_folder(install)?;
    let meta = std::fs::metadata(data.join("resources.assets")).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(format!("{GAME_CACHE_VERSION}\n{}\n{} {modified}\n", install.display(), meta.len()))
}

fn cached_name(dir: &std::path::Path, kind: &str, name: &str) -> PathBuf {
    dir.join(kind).join(format!("{name}.png"))
}

/// The pictures kept from an earlier read of this install, if they're still current.
fn from_game_cache(install: &std::path::Path) -> Option<(HashMap<String, RgbaImage>, HashMap<String, RgbaImage>)> {
    let dir = game_cache_dir()?;
    if std::fs::read_to_string(dir.join("stamp.txt")).ok()? != game_stamp(install)? {
        return None;
    }
    let read = |kind: &str, names: Vec<String>| -> Option<HashMap<String, RgbaImage>> {
        names.into_iter().map(|n| Some((n.clone(), image::open(cached_name(&dir, kind, &n)).ok()?.to_rgba8()))).collect()
    };
    let sprites = read("sprites", game_names().into_iter().map(|(n, _, _)| n).collect())?;
    let art = read("art", ART.iter().map(|s| s.to_string()).chain(anim_names()).collect())?;
    Some((sprites, art))
}

/// Keeps the pictures read from the game, so it's only read once.
fn save_game_cache(install: &std::path::Path, sprites: &HashMap<String, RgbaImage>, art: &HashMap<String, RgbaImage>) -> Result<(), String> {
    let dir = game_cache_dir().ok_or("no cache folder")?;
    let _ = std::fs::remove_file(dir.join("stamp.txt"));
    let _ = std::fs::remove_dir_all(dir.join("highways"));
    for (kind, set) in [("sprites", sprites), ("art", art)] {
        for (name, img) in set {
            let p = cached_name(&dir, kind, name);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            img.save(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        }
    }
    let stamp = game_stamp(install).ok_or("can't read the game's files")?;
    std::fs::write(dir.join("stamp.txt"), stamp).map_err(|e| e.to_string())
}

/// The pictures from Clone Hero: kept ones if current, else read from the game (and kept).
fn game_pictures(install: &std::path::Path) -> Result<(HashMap<String, RgbaImage>, HashMap<String, RgbaImage>), String> {
    if let Some(kept) = from_game_cache(install) {
        return Ok(kept);
    }
    let (sprites, art) = from_game(install)?;
    if let Err(e) = save_game_cache(install, &sprites, &art) {
        log::warn!("couldn't keep Clone Hero's pictures: {e}");
    }
    Ok((sprites, art))
}

/// The kit while something's using it (the colour editor, a chart preview being built): shared
/// then, and freed once nothing is (it's quick to make again from the kept pictures).
static KIT: std::sync::Mutex<Option<std::sync::Weak<Kit>>> = std::sync::Mutex::new(None);

/// The sprites (from Clone Hero's own files when it's installed, kept after the first read;
/// otherwise downloaded or read from the cache) and the highway textures.
pub async fn load() -> Result<Arc<Kit>, String> {
    let install = find_install();
    if let Some(k) = KIT.lock().unwrap().as_ref().and_then(std::sync::Weak::upgrade)
        && k.game == install
    {
        return Ok(k);
    }
    let kit = load_new(install).await?;
    *KIT.lock().unwrap() = Some(Arc::downgrade(&kit));
    Ok(kit)
}

async fn load_new(install: Option<PathBuf>) -> Result<Arc<Kit>, String> {
    let guitar = crate::textures::get(InstrumentType::FiveFret).await?;
    let drums = crate::textures::get(InstrumentType::Drums).await?;
    if let Some(install) = install {
        let from = install.clone();
        match tokio::task::spawn_blocking(move || game_pictures(&from)).await.map_err(|e| e.to_string())? {
            Ok((sprites, art)) => return Ok(Arc::new(Kit { sprites, game: Some(install), art, guitar, drums })),
            Err(e) => log::warn!("Clone Hero's sprites from {}: {e}; using the downloaded ones", install.display()),
        }
    }
    let fetched = futures::future::join_all(files().into_iter().map(fetch)).await;
    let raw: Vec<(String, Vec<u8>)> = fetched.into_iter().collect::<Result<_, _>>()?;
    tokio::task::spawn_blocking(move || {
        let mut sprites = HashMap::new();
        for (name, b) in raw {
            let img = image::load_from_memory(&b).map_err(|e| format!("{name}: {e}"))?.to_rgba8();
            sprites.insert(name, img);
        }
        Ok(Arc::new(Kit { sprites, game: None, art: HashMap::new(), guitar, drums }))
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------- The chart preview ----------

/// How the chart preview looks in Clone Hero's layout (from Settings): the colour profile (by
/// name in the game's Colors folder; none: the player's own), note speed and highway length
/// (none: the player's own).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreviewLook {
    pub profile: Option<String>,
    pub note_speed: Option<f32>,
    pub highway_length: Option<f32>,
}

static LOOK: std::sync::Mutex<Option<PreviewLook>> = std::sync::Mutex::new(None);

pub fn set_preview_look(look: PreviewLook) {
    *LOOK.lock().unwrap() = Some(look);
}

pub fn preview_look() -> PreviewLook {
    LOOK.lock().unwrap().clone().unwrap_or_default()
}

/// The colour profile the player uses in Clone Hero (its profiles.ini), if it says.
pub fn player_color_profile() -> Option<String> {
    player_setting("color_profile_name")
}

/// A setting from the first profile in Clone Hero's profiles.ini (the player's own).
fn player_setting(key: &str) -> Option<String> {
    let install = find_install()?;
    let mut tries = vec![install.join("PlayerData").join("profiles.ini")];
    if let Some(c) = crate::color_profile::folder() {
        tries.extend(c.ancestors().skip(2).take(2).map(|d| d.join("profiles.ini")));
    }
    let text = tries.iter().find_map(|p| std::fs::read_to_string(p).ok())?;
    text.lines()
        .take_while(|l| !l.trim_start().starts_with("[profile1]"))
        .find_map(|l| l.split_once('=').filter(|(k, _)| k.trim() == key).map(|(_, v)| v.trim().to_string()))
        .filter(|v| !v.is_empty())
}

/// The player plays lefty (Lefty flip on in their Clone Hero profile).
pub fn player_lefty() -> bool {
    // TRUSS_LEFTY=1 for testing, without changing the game's settings.
    std::env::var_os("TRUSS_LEFTY").is_some_and(|v| v == "1") || player_setting("lefty_flip").is_some_and(|v| v == "1")
}

/// The highway the player has chosen in Clone Hero ("Default" if none).
pub fn player_highway() -> String {
    player_setting("highway_name").unwrap_or_else(|| "Default".into())
}

/// A highway's picture: one of the player's own (an image in Custom\Highways), or one of the
/// game's (kept after the first read). None for one Truss can't show (a video highway).
fn highway_picture(install: &std::path::Path, name: &str) -> Option<RgbaImage> {
    let custom = install.join("PlayerData").join("Custom").join("Highways");
    let own = std::iter::once(custom.join(name)).chain(["png", "jpg", "jpeg"].map(|e| custom.join(format!("{name}.{e}"))));
    if let Some(img) = own.filter(|p| p.is_file()).find_map(|p| image::open(p).ok()) {
        return Some(img.to_rgba8());
    }
    let kept = game_cache_dir().map(|d| cached_name(&d, "highways", name));
    if let Some(img) = kept.as_ref().and_then(|p| image::open(p).ok()) {
        return Some(img.to_rgba8());
    }
    let file = crate::unity_assets::AssetFile::open(&crate::unity_assets::data_folder(install)?.join("resources.assets")).ok()?;
    let img = file.texture(file.texture_named(name)?).ok()?;
    if let Some(p) = kept {
        let _ = p.parent().map(std::fs::create_dir_all);
        let _ = img.save(&p);
    }
    Some(img)
}

/// Lets go of the chart preview's textures (when it closes): what's open keeps its own.
pub fn release_chart_textures() {
    CHART_TEXTURES.lock().unwrap().clear();
}

/// Textures made for the chart preview: (instrument, profile name, profile text) -> textures.
static CHART_TEXTURES: std::sync::Mutex<Vec<((InstrumentType, String, String, String, bool), Arc<TextureSet>)>> = std::sync::Mutex::new(Vec::new());

/// The chart preview's textures: guitar and drums from Clone Hero's own sprites in the chosen
/// colour profile when it's installed (in its layout), otherwise the usual ones.
pub async fn chart_textures(itype: InstrumentType) -> Result<Arc<TextureSet>, String> {
    if itype != InstrumentType::SixFret
        && let Some(t) = game_chart_textures(itype).await
    {
        return Ok(t);
    }
    crate::textures::get(itype).await
}

async fn game_chart_textures(itype: InstrumentType) -> Option<Arc<TextureSet>> {
    find_install()?;
    let kit = load().await.ok()?;
    kit.game.as_ref()?;
    let name = preview_look().profile.or_else(player_color_profile).unwrap_or_else(|| "DefaultColors".into());
    let text = crate::color_profile::folder().and_then(|d| std::fs::read_to_string(d.join(format!("{name}.ini"))).ok()).unwrap_or_default();
    let highway = player_highway();
    let key = (itype, name, text.clone(), highway.clone(), player_lefty());
    if let Some((_, t)) = CHART_TEXTURES.lock().unwrap().iter().find(|(k, _)| *k == key) {
        return Some(t.clone());
    }
    let t = tokio::task::spawn_blocking(move || {
        let p = Profile::parse(&text);
        let d = Profile::defaults();
        let mut set = if itype == InstrumentType::Drums { drums(&kit, &p, &d, false) } else { guitar(&kit, &p, &d, false) };
        // The player's own highway, as Clone Hero shows it.
        if let Some(img) = kit.game.as_deref().and_then(|i| highway_picture(i, &highway)) {
            set.highway = crate::textures::highway_levels(img);
        }
        Arc::new(set)
    })
    .await
    .ok()?;
    let mut cache = CHART_TEXTURES.lock().unwrap();
    cache.retain(|(k, _)| k.0 != itype);
    cache.push((key, t.clone()));
    Some(t)
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
    /// A picture `w` by `h` (in the downloaded sprites' pixels; bigger for the game's own,
    /// which are drawn at twice the size).
    fn new(kit: &'a Kit, w: u32, h: u32) -> Self {
        Stack { kit, out: RgbaImage::new(w * kit.scale(), h * kit.scale()) }
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
        let dy = dy * self.kit.scale() as i64;
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
    /// Hit flames and sparks.
    pub flames: Option<BuiltFlames>,
    /// Clone Hero's layout, when the sprites are its own.
    pub game: Option<crate::textures::GameLook>,
    /// The game's sustain cross-sections (a sustain's at rest, held, an open note's), when its
    /// own sprites.
    pub sustain_shapes: Option<(RgbaImage, RgbaImage, RgbaImage)>,
    /// Their rounded ends (at rest, held, an open note's): the top of each picture.
    pub sustain_ends: Option<(RgbaImage, RgbaImage, RgbaImage)>,
}

/// A row across one of the game's sustain pictures: what a sustain looks like across its
/// width, all along it (three rows of it, so smooth sampling never reaches past it).
fn cross_section(img: &RgbaImage, at: f32) -> RgbaImage {
    let y = ((img.height() as f32 * at) as u32).min(img.height() - 1);
    let row = image::imageops::crop_imm(img, 0, y, img.width(), 1).to_image();
    RgbaImage::from_fn(img.width(), 3, |x, _| *row.get_pixel(x, 0))
}

/// The top of a sustain picture, `rows` tall: its rounded end (the picture's top border).
fn sustain_end(img: &RgbaImage, rows: u32) -> RgbaImage {
    image::imageops::crop_imm(img, 0, 0, img.width(), rows.min(img.height())).to_image()
}

/// The rounded ends of the game's sustains: the top 38 rows of the sustain pictures and the
/// top 55 of the open note's (their sprites' top borders, which stay as drawn however long the
/// sustain is).
fn sustain_ends(kit: &Kit) -> Option<(RgbaImage, RgbaImage, RgbaImage)> {
    kit.game.as_ref()?;
    Some((
        sustain_end(kit.art.get("spr_sustain_strip6_1")?, 38),
        sustain_end(kit.art.get("spr_sustain_strip6_0")?, 38),
        sustain_end(kit.art.get("open_sustain_glow")?, 55),
    ))
}

/// A picture cut into its rows (each three tall, for smooth sampling), tinted.
fn rows_of(img: &RgbaImage, c: Color) -> Vec<Handle> {
    let t = tinted(img, c, false);
    (0..t.height())
        .map(|y| {
            let row = RgbaImage::from_fn(t.width(), 3, |x, _| *t.get_pixel(x, y));
            handle(row)
        })
        .collect()
}

fn sustain_shapes(kit: &Kit) -> Option<(RgbaImage, RgbaImage, RgbaImage)> {
    kit.game.as_ref()?;
    // Frame 1 of the sustain strip is a sustain at rest, frame 0 one being held (it glows).
    Some((
        cross_section(kit.art.get("spr_sustain_strip6_1")?, 0.8),
        cross_section(kit.art.get("spr_sustain_strip6_0")?, 0.8),
        cross_section(kit.art.get("open_sustain_glow")?, 0.75),
    ))
}

/// The note speed and highway length (percent) of the first profile in Clone Hero's
/// profiles.ini, if it has them.
pub fn profile_settings(install: &std::path::Path) -> (Option<f32>, Option<f32>) {
    let mut tries = vec![install.join("PlayerData").join("profiles.ini")];
    if let Some(c) = crate::color_profile::folder() {
        tries.extend(c.ancestors().skip(2).take(2).map(|d| d.join("profiles.ini")));
    }
    let Some(text) = tries.iter().find_map(|p| std::fs::read_to_string(p).ok()) else { return (None, None) };
    let get = |key: &str| {
        text.lines()
            .take_while(|l| !l.trim_start().starts_with("[profile1]"))
            .find_map(|l| l.split_once('=').filter(|(k, _)| k.trim() == key).and_then(|(_, v)| v.trim().parse::<f32>().ok()))
    };
    (get("note_speed"), get("highway_length"))
}

/// Clone Hero's layout for the highway preview: speed and length from the player's profile,
/// and the game's rails.
fn game_look(kit: &Kit) -> Option<crate::textures::GameLook> {
    let install = kit.game.as_ref()?;
    let (speed, length) = profile_settings(install);
    // The rail's cross-section where it's widest (the bottom of the game's sprite).
    let rail = kit.art.get("sidebar").map(|img| {
        let (w, h) = img.dimensions();
        let mut row = RgbaImage::new(w, 1);
        for x in 0..w {
            let mut acc = [0u32; 4];
            let rows = (h - 64)..h;
            for y in rows.clone() {
                let p = img.get_pixel(x, y);
                for i in 0..4 {
                    acc[i] += p[i] as u32;
                }
            }
            let n = rows.len() as u32;
            row.put_pixel(x, 0, image::Rgba([(acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8, (acc[3] / n) as u8]));
        }
        let right = image::imageops::flip_horizontal(&row);
        (handle(row), handle(right))
    });
    Some(crate::textures::GameLook {
        note_speed: speed.unwrap_or(crate::preview::NOTE_SPEED_DEFAULT),
        highway_length: length.unwrap_or(crate::preview::HIGHWAY_LENGTH_DEFAULT),
        rail,
        lefty: player_lefty(),
        beatline: kit
            .art
            .get("beatline")
            .map(|img| [1, 2, 3, 4, 6, 8, 12, 16, 24].map(|h| (h, handle(crate::textures::shrink(img, 512, h)))).to_vec())
            .unwrap_or_default(),
    })
}

/// Hit flames and sparks in a profile's colours.
pub struct BuiltFlames {
    /// A note's flame, and the glow behind it.
    pub note: RgbaImage,
    pub glow: RgbaImage,
    /// The open note's (or kick's) flame along the strike line, frame by frame.
    pub wide: Vec<RgbaImage>,
    /// The game's animations, when read from it: hit flame, spark burst, held sustain sparks.
    pub note_frames: Vec<RgbaImage>,
    pub sparks: Vec<RgbaImage>,
    pub hold: Vec<RgbaImage>,
}

/// The game's hit flame, spark and held-sustain spark animations, frame by frame.
pub fn anim_names() -> Vec<String> {
    let mut v: Vec<String> = (0..15).map(|i| format!("hitflame_anim_16_{i}")).collect();
    v.extend((0..16).map(|i| format!("hit_spark_{i}")));
    v.extend((0..16).map(|i| format!("sustain_spark_anim_color_{i}")));
    v.extend((0..16).map(|i| format!("sustain_spark_anim_overlay_{i}")));
    v
}

/// A sprite the game draws added onto what's behind it (its "FadeSpriteAdd1Intense"
/// material), turned into one that looks about the same drawn the usual way: as light as it
/// adds, see-through where it adds nothing. Exact over black; close over the dark highway.
fn additive(mut img: RgbaImage) -> RgbaImage {
    // "Intense": the game's additive material brightens a little more than plain adding.
    const INTENSE: f32 = 1.25;
    for p in img.pixels_mut() {
        let a = p[3] as f32 / 255.0 * INTENSE;
        let e = [p[0] as f32 / 255.0 * a, p[1] as f32 / 255.0 * a, p[2] as f32 / 255.0 * a];
        let m = e[0].max(e[1]).max(e[2]).min(1.0);
        if m <= 0.0 {
            *p = image::Rgba([0, 0, 0, 0]);
            continue;
        }
        let c = |v: f32| ((v / m).min(1.0) * 255.0).round() as u8;
        *p = image::Rgba([c(e[0]), c(e[1]), c(e[2]), (m * 255.0).round() as u8]);
    }
    img
}

fn handle(img: RgbaImage) -> Handle {
    Handle::from_rgba(img.width(), img.height(), img.into_raw())
}

/// A sprite multiplied by a colour. `grey`: made greyscale first (for sprites that come coloured).
fn tinted(src: &RgbaImage, c: Color, grey: bool) -> RgbaImage {
    let t = rgb(c);
    let mut out = src.clone();
    for p in out.pixels_mut() {
        let v = if grey {
            let l = 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
            [l; 3]
        } else {
            [p[0] as f32, p[1] as f32, p[2] as f32]
        };
        for i in 0..3 {
            p[i] = (v[i] * t[i]).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// The hit flames in a profile's colours: `wide` is the strip for open notes (or kicks), cut
/// into `frames`, tinted `wide_key`.
fn flames(kit: &Kit, p: &Profile, d: &Profile, sp_active: bool, wide: &str, frames: u32, wide_key: &str) -> Option<BuiltFlames> {
    let o = Colors { p, d, section: "other" };
    let sp = if sp_active { "_sp_active" } else { "" };
    let note = tinted(kit.sprites.get("other/note_HitFlame3")?, o.get(&format!("striker_hit_flame{sp}")), true);
    let glow = tinted(kit.sprites.get("other/note_Particle")?, o.get(&format!("striker_hit_particles{sp}")), false);
    // Under star power the open note's (and kick's) flame takes the star power flame colour,
    // like every other flame.
    let strip = tinted(kit.sprites.get(wide)?, o.get(if sp_active { "striker_hit_flame_sp_active" } else { wide_key }), false);
    let fw = strip.width() as f32 / frames as f32;
    let cut = (0..frames)
        .map(|i| {
            let x = (i as f32 * fw).round() as u32;
            let w = (((i + 1) as f32 * fw).round() as u32).min(strip.width()) - x;
            image::imageops::crop_imm(&strip, x, 0, w, strip.height()).to_image()
        })
        .collect();
    // The game's frames, in order, tinted (as the game colours them).
    let frames = |prefix: &str, c: Option<Color>| -> Vec<RgbaImage> {
        let n = anim_names().iter().filter(|n| n.starts_with(prefix)).count();
        (0..n).filter_map(|i| kit.art.get(&format!("{prefix}{i}"))).map(|img| tinted(img, c.unwrap_or(Color::WHITE), false)).collect()
    };
    let color = |key: &str| Some(o.get(&format!("{key}{sp}")));
    // Held sustains: the coloured sparks with the white overlay added on top.
    let hold = frames("sustain_spark_anim_color_", color("striker_hold_spark"))
        .into_iter()
        .zip(frames("sustain_spark_anim_overlay_", None))
        .map(|(mut c, w)| {
            for (p, q) in c.pixels_mut().zip(w.pixels()) {
                let (a, b) = (p[3] as f32 / 255.0, q[3] as f32 / 255.0);
                let oa = (a + b).min(1.0).max(1e-6);
                for i in 0..3 {
                    p[i] = ((p[i] as f32 * a + q[i] as f32 * b) / oa).min(255.0) as u8;
                }
                p[3] = (oa * 255.0) as u8;
            }
            additive(c)
        })
        .collect();
    Some(BuiltFlames {
        note,
        glow,
        wide: cut,
        note_frames: frames("hitflame_anim_16_", color("striker_hit_flame")).into_iter().map(additive).collect(),
        // The burst of sparks only shows while star power is on.
        sparks: if sp_active { frames("hit_spark_", color("striker_hit_particles")).into_iter().map(additive).collect() } else { Vec::new() },
        hold,
    })
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
        let shapes = self.sustain_shapes;
        let ends = self.sustain_ends;
        let mut sustains_held = HashMap::new();
        let mut sustain_caps = HashMap::new();
        let mut sustain_cap_ratio = HashMap::new();
        if let Some((plain, _, open)) = &ends {
            for (k, _) in &self.sustains {
                let img = if k.0 == nt::OPEN { open } else { plain };
                // Rows of the end against the picture's width (the open note's is 512 wide).
                let full_w = if k.0 == nt::OPEN { 512.0 } else { img.width() as f32 };
                sustain_cap_ratio.insert(k.0, img.height() as f32 / full_w);
            }
        }
        for (k, c) in &self.sustains {
            if let Some((plain, held, open)) = &ends {
                let (rest, hold) = if k.0 == nt::OPEN { (open, open) } else { (plain, held) };
                sustain_caps.insert(*k, (rows_of(rest, *c), rows_of(hold, *c)));
            }
        }
        let sustains = self
            .sustains
            .into_iter()
            .map(|(k, c)| match &shapes {
                Some((plain, held, open)) => {
                    if k.0 != nt::OPEN {
                        sustains_held.insert(k, handle(tinted(held, c, false)));
                    }
                    (k, handle(tinted(if k.0 == nt::OPEN { open } else { plain }, c, false)))
                }
                None => (k, solid(c)),
            })
            .collect();
        let flames = self.flames.map(|f| Flames {
            note_aspect: f.note.width() as f32 / f.note.height() as f32,
            note: handle(f.note),
            glow: handle(f.glow),
            wide: f.wide.into_iter().map(handle).collect(),
            note_frames: f.note_frames.into_iter().map(handle).collect(),
            sparks: f.sparks.into_iter().map(handle).collect(),
            hold: f.hold.into_iter().map(handle).collect(),
        });
        let mut set = with_base(base, notes, strikers, sustains);
        set.flames = flames;
        set.game = self.game;
        set.sustains_held = sustains_held;
        set.sustain_caps = sustain_caps;
        set.sustain_cap_ratio = sustain_cap_ratio;
        set
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
            // The art for where it is on screen (the lanes run the other way for lefty), in
            // the lane's own colours.
            let n = if kit.game.is_some() && player_lefty() { GUITAR_LANES.len() - i } else { i + 1 };
            // As the game layers them: the ring and its lower half over the head, both in the
            // ring's colour, so it shows all the way round.
            let fret = |light: Option<Color>| {
                let mut s = Stack::new(kit, 128, 64)
                    .layer(&format!("guitar/striker_GuitarUnderlay{n}"), None, 0)
                    .layer(&format!("guitar/striker_GuitarHead{n}"), Some(c.get(&format!("striker_base_{lane}"))), 0)
                    .layer(&format!("guitar/striker_Ring{n}"), Some(c.get(&format!("striker_cover_{lane}"))), 0)
                    .layer(&format!("guitar/striker_RingCover{n}"), Some(c.get(&format!("striker_cover_{lane}"))), 0)
                    .layer(&format!("guitar/striker_HeadColor{n}"), Some(c.get(&format!("striker_head_cover_{lane}"))), 0);
                if let Some(l) = light {
                    s = s.layer(&format!("guitar/striker_HeadGlow{n}"), Some(l), 0);
                }
                s.done()
            };
            let width = if kit.game.is_some() { 0.197 } else { 0.19 };
            (*ty, [fret(None), fret(Some(c.get(&format!("striker_head_light_{lane}")))), fret(Some(c.get("striker_head_light_open")))], width)
        })
        .collect();
    let mut sustains = HashMap::new();
    for (ty, lane) in GUITAR_LANES.iter().copied().chain([(nt::OPEN, "open")]) {
        let plain = if sp_active { c.get("sustain_sp_active") } else { c.get(&format!("sustain_{lane}")) };
        let sp = if sp_active { c.get("sustain_sp_phrase_active") } else { c.get("sustain_sp_phrase") };
        sustains.insert((ty, false), plain);
        sustains.insert((ty, true), sp);
    }
    let flames = flames(kit, p, d, sp_active, "guitar/guitarNote_Open_HitFlame", 5, "striker_hit_flame_open");
    Built { notes, strikers, sustains, flames, game: game_look(kit), sustain_shapes: sustain_shapes(kit), sustain_ends: sustain_ends(kit) }
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
    let mut shapes = DRUM_STRIKERS;
    if kit.game.is_some() && player_lefty() {
        shapes.reverse();
    }
    let strikers = DRUM_LANES
        .iter()
        .zip(shapes)
        .map(|((ty, lane), (ring, head))| {
            let pad = |light: Option<Color>| {
                let mut s = Stack::new(kit, 128, 64)
                    .layer(&format!("drums/drumStriker_Underlay{head}"), None, 0)
                    .layer(&format!("drums/drumStriker_Head{head}"), Some(c.get(&format!("drums_striker_base_{lane}"))), 0)
                    .layer(&format!("drums/striker_Ring{ring}"), Some(c.get(&format!("drums_striker_cover_{lane}"))), 0)
                    .layer(&format!("drums/striker_RingCover{ring}"), Some(c.get(&format!("drums_striker_cover_{lane}"))), 0)
                    .layer(&format!("drums/striker_HeadColor{ring}"), Some(c.get(&format!("drums_striker_head_cover_{lane}"))), 0);
                if let Some(l) = light {
                    s = s.layer(&format!("drums/striker_HeadGlow{ring}"), Some(l), 0);
                }
                s.done()
            };
            let width = if kit.game.is_some() { 0.197 * 1.22 } else { 0.21 };
            (*ty, [pad(None), pad(Some(c.get(&format!("drums_striker_head_light_{lane}")))), pad(Some(c.get("drums_striker_head_light_kick")))], width)
        })
        .collect();
    let flames = flames(kit, p, d, sp_active, "drums/drumNote_Kick_Hitflame", 4, "striker_hit_flame_kick");
    Built { notes, strikers, sustains: HashMap::new(), flames, game: game_look(kit), sustain_shapes: None, sustain_ends: None }
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
        sustains_held: HashMap::new(),
        sustain_caps: HashMap::new(),
        sustain_cap_ratio: HashMap::new(),
        flames: None,
        game: None,
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
        RenderTrack { itype: InstrumentType::FiveFret, max_len: max(&g), notes: g, beats: Vec::new() },
        RenderTrack { itype: InstrumentType::Drums, max_len: max(&d), notes: d, beats: Vec::new() },
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

    /// Reading a real install keeps its pictures; the next read gets the same ones back from
    /// what was kept (TRUSS_CH_DIR; uses Truss's cache, or TRUSS_CACHE_DIR's).
    #[test]
    #[ignore]
    fn keeps_the_games_pictures() {
        let Some(dir) = std::env::var_os("TRUSS_CH_DIR") else { return };
        let install = PathBuf::from(dir);
        if let Some(c) = game_cache_dir() {
            let _ = std::fs::remove_file(c.join("stamp.txt"));
        }
        let t = std::time::Instant::now();
        let (s1, a1) = game_pictures(&install).unwrap();
        let first = t.elapsed();
        assert!(from_game_cache(&install).is_some(), "kept");
        let t = std::time::Instant::now();
        let (s2, a2) = game_pictures(&install).unwrap();
        eprintln!("read from the game in {first:?}, from what was kept in {:?}", t.elapsed());
        assert_eq!((s1.len(), a1.len()), (s2.len(), a2.len()));
        for (k, v) in &s1 {
            assert_eq!(v.as_raw(), s2[k].as_raw(), "{k}");
        }
        for (k, v) in &a1 {
            assert_eq!(v.as_raw(), a2[k].as_raw(), "{k}");
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
