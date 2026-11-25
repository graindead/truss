//! The player card as a picture to share: Truss Stats' card (laid out at 1200 x 630, saved at
//! twice that), in its colours, drawn without a browser (tiny-skia and cosmic-text).

use super::Loaded;
use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use serde_json::Value;
use tiny_skia::{FillRule, FilterQuality, Paint, PathBuilder, Pattern, Pixmap, Rect, SpreadMode, Transform};

const W: f32 = 1200.0;
const H: f32 = 630.0;
/// Saved at twice the size it's laid out at.
const SCALE: f32 = 2.0;

type Rgb = [u8; 3];
// Truss's Kusha theme, as Truss Stats uses.
const BG: Rgb = [0x13, 0x19, 0x24];
const TINT: Rgb = [0x17, 0x25, 0x39];
const ACCENT: Rgb = [0x3b, 0x8e, 0xf0];
const RAISED: Rgb = [0x1a, 0x20, 0x2b];
const LINE: Rgb = [0x2a, 0x30, 0x3b];
const TEXT: Rgb = [0xe2, 0xe9, 0xf4];
const MUTED: Rgb = [0x93, 0x9a, 0xa6];
const CHIP_TEXT: Rgb = [0x13, 0x19, 0x24];
const TIERS: [Rgb; 7] = [
    [0x85, 0x8b, 0x96],
    [0x5c, 0xc8, 0x9c],
    [0xa6, 0xc9, 0x5e],
    [0xf2, 0xc1, 0x4e],
    [0xf0, 0x90, 0x4e],
    [0xeb, 0x5a, 0x6b],
    [0xb6, 0x7c, 0xf2],
];

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Center,
    Right,
}

struct Card {
    pix: Pixmap,
}

fn paint(c: Rgb) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], 255);
    p.anti_alias = true;
    p
}

/// A rectangle with rounded corners, in card units.
fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let (x, y, w, h, r) = (x * SCALE, y * SCALE, w * SCALE, h * SCALE, (r * SCALE).min(w * SCALE / 2.0).min(h * SCALE / 2.0));
    let mut b = PathBuilder::new();
    b.move_to(x + r, y);
    b.line_to(x + w - r, y);
    b.quad_to(x + w, y, x + w, y + r);
    b.line_to(x + w, y + h - r);
    b.quad_to(x + w, y + h, x + w - r, y + h);
    b.line_to(x + r, y + h);
    b.quad_to(x, y + h, x, y + h - r);
    b.line_to(x, y + r);
    b.quad_to(x, y, x + r, y);
    b.close();
    b.finish()
}

impl Card {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Rgb) {
        if let Some(r) = Rect::from_xywh(x * SCALE, y * SCALE, w * SCALE, h * SCALE) {
            self.pix.fill_rect(r, &paint(c), Transform::identity(), None);
        }
    }

    fn rounded(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, fill: Rgb, outline: Option<Rgb>) {
        let Some(path) = rounded(x, y, w, h, r) else { return };
        self.pix.fill_path(&path, &paint(fill), FillRule::Winding, Transform::identity(), None);
        if let Some(o) = outline {
            let stroke = tiny_skia::Stroke { width: 1.5 * SCALE, ..Default::default() };
            self.pix.stroke_path(&path, &paint(o), &stroke, Transform::identity(), None);
        }
    }

    /// A picture cropped to a square and fitted to a rounded box.
    fn picture(&mut self, (pw, ph, rgba): (u32, u32, &[u8]), x: f32, y: f32, side: f32, r: f32) {
        let Some(path) = rounded(x, y, side, side, r) else { return };
        // tiny-skia wants premultiplied colour.
        let mut data = rgba.to_vec();
        for p in data.chunks_exact_mut(4) {
            let a = u16::from(p[3]);
            for c in &mut p[..3] {
                *c = (u16::from(*c) * a / 255) as u8;
            }
        }
        let Some(size) = tiny_skia::IntSize::from_wh(pw, ph) else { return };
        let Some(img) = Pixmap::from_vec(data, size) else { return };
        let crop = pw.min(ph) as f32;
        let k = side * SCALE / crop;
        let t = Transform::from_row(k, 0.0, 0.0, k, x * SCALE - (pw as f32 - crop) / 2.0 * k, y * SCALE - (ph as f32 - crop) / 2.0 * k);
        let shader = Pattern::new(img.as_ref(), SpreadMode::Pad, FilterQuality::Bicubic, 1.0, t);
        let p = Paint { shader, anti_alias: true, ..Default::default() };
        self.pix.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
    }

    /// Text with its baseline at `y`, cut short with "…" to fit `max` wide.
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, s: &str, x: f32, y: f32, size: f32, bold: bool, c: Rgb, max: f32, align: Align) {
        let mut guard = crate::chopt::draw::fonts().lock().unwrap_or_else(|e| e.into_inner());
        let (fs, cache) = &mut *guard;
        let attrs = Attrs::new().family(Family::Name("Segoe UI")).weight(if bold { Weight::BOLD } else { Weight::NORMAL });
        let px = size * SCALE;
        let mut buf = Buffer::new(fs, Metrics::new(px, px * 1.3));
        buf.set_size(fs, None, None);
        let mut shown: Vec<char> = s.chars().collect();
        let mut cut = false;
        let (width, baseline) = loop {
            let t: String = shown.iter().collect::<String>() + if cut { "…" } else { "" };
            buf.set_text(fs, &t, &attrs, Shaping::Advanced, None);
            buf.shape_until_scroll(fs, false);
            let (w, base) = buf.layout_runs().fold((0.0f32, 0.0f32), |(w, b), r| (w.max(r.line_w), if b == 0.0 { r.line_y } else { b }));
            if w <= max * SCALE || shown.is_empty() {
                break (w, base);
            }
            shown.pop();
            cut = true;
        };
        let left = match align {
            Align::Left => x * SCALE,
            Align::Center => x * SCALE - width / 2.0,
            Align::Right => x * SCALE - width,
        };
        let (ox, oy) = (left.round() as i32, (y * SCALE - baseline).round() as i32);
        let mut dots = Vec::new();
        buf.draw(fs, cache, cosmic_text::Color::rgb(c[0], c[1], c[2]), |gx, gy, _, _, col| dots.push((gx, gy, col.a())));
        drop(guard);
        let (w, h) = (self.pix.width() as i32, self.pix.height() as i32);
        let d = self.pix.data_mut();
        for (gx, gy, a) in dots {
            let (px, py) = (ox + gx, oy + gy);
            if a == 0 || px < 0 || py < 0 || px >= w || py >= h {
                continue;
            }
            // The card is opaque underneath, so this is a plain blend.
            let i = ((py * w + px) * 4) as usize;
            let a = u32::from(a);
            for k in 0..3 {
                d[i + k] = ((u32::from(c[k]) * a + u32::from(d[i + k]) * (255 - a)) / 255) as u8;
            }
        }
    }
}

fn num(v: &Value, path: &str) -> Option<f64> {
    v.pointer(path).and_then(Value::as_f64)
}

fn word<'a>(v: &'a Value, path: &str) -> &'a str {
    v.pointer(path).and_then(Value::as_str).unwrap_or("")
}

/// Draws a player's card: RGBA pixels, `(width, height, data)`.
pub fn draw(l: &Loaded) -> Option<(u32, u32, Vec<u8>)> {
    use super::view::{big, hours, instrument, pct, thousands};
    let s = &l.stats;
    let mut c = Card { pix: Pixmap::new((W * SCALE) as u32, (H * SCALE) as u32)? };
    let count = |v: Option<f64>| v.map_or("–".to_string(), thousands);
    c.rect(0.0, 0.0, W, H, BG);
    c.rect(0.0, 0.0, W, 196.0, TINT);
    c.rect(0.0, 0.0, 8.0, 196.0, ACCENT);

    // Picture, or the first letter of the name.
    c.rounded(44.0, 34.0, 128.0, 128.0, 22.0, RAISED, Some(LINE));
    match &l.picture_rgba {
        Some((w, h, px)) => c.picture((*w, *h, px), 44.0, 34.0, 128.0, 22.0),
        None => {
            let first = l.name.chars().next().map_or("?".to_string(), |ch| ch.to_uppercase().to_string());
            c.text(&first, 108.0, 122.0, 70.0, true, ACCENT, 120.0, Align::Center);
        }
    }
    c.text(&l.name, 198.0, 92.0, 52.0, true, TEXT, 640.0, Align::Left);
    let main = s.pointer("/player/instruments/0/instrument").and_then(Value::as_str).map(|i| format!("{} player", instrument(i)));
    let since = chrono::NaiveDate::parse_from_str(word(s, "/player/since"), "%Y-%m-%d").ok().map(|d| d.format("member since %B %Y").to_string());
    let about = [main, since].into_iter().flatten().collect::<Vec<_>>().join("  ·  ");
    c.text(&about, 198.0, 132.0, 22.0, false, MUTED, 640.0, Align::Left);
    if let Some(r) = num(s, "/player/ranks/totalScore") {
        c.text("SCORE RANK", 1156.0, 70.0, 17.0, true, MUTED, 280.0, Align::Right);
        c.text(&format!("#{}", thousands(r)), 1156.0, 130.0, 58.0, true, ACCENT, 280.0, Align::Right);
    }

    let st = |k: &str| num(s, &format!("/player/stats/{k}"));
    let rank = |k: &str| num(s, &format!("/player/ranks/{k}")).map_or(String::new(), |r| format!("  ·  #{}", thousands(r)));
    let tiles = [
        (pct(num(s, "/overall/simple")), "average accuracy".to_string()),
        (count(st("totalFcs")), format!("full combos{}", rank("totalFcs"))),
        (count(st("songsPlayed")), "songs played".into()),
        (st("totalScore").map_or("–".into(), big), "total score".into()),
        (count(st("totalStarsEarned")), format!("stars{}", rank("starsEarned"))),
        (st("notesHit").map_or("–".into(), big), format!("notes hit{}", rank("notesHit"))),
        (hours(st("totalTimePlayedMs")), format!("hours played{}", rank("hoursPlayed"))),
        (count(st("highestCombo")), "highest combo".into()),
    ];
    for (i, (n, label)) in tiles.iter().enumerate() {
        let w = 265.0;
        let (x, y) = (44.0 + (i % 4) as f32 * (w + 17.3), 216.0 + (i / 4) as f32 * 104.0);
        c.rounded(x, y, w, 90.0, 10.0, RAISED, Some(LINE));
        c.text(n, x + 18.0, y + 47.0, 34.0, true, TEXT, w - 36.0, Align::Left);
        c.text(label, x + 18.0, y + 74.0, 17.0, false, MUTED, w - 36.0, Align::Left);
    }

    // Hardest FC and top artists.
    let (half, y) = (547.0, 428.0);
    c.rounded(44.0, y, half, 142.0, 10.0, RAISED, Some(LINE));
    c.text("HARDEST FC", 62.0, y + 32.0, 16.0, true, MUTED, 300.0, Align::Left);
    match s.pointer("/hardestFcs/0") {
        Some(f) => {
            let tier = num(f, "/tier").unwrap_or(0.0);
            c.rounded(62.0, y + 52.0, 64.0, 64.0, 10.0, TIERS[(tier as usize).min(6)], None);
            c.text(&format!("{tier:.0}"), 94.0, y + 98.0, 40.0, true, CHIP_TEXT, 60.0, Align::Center);
            let name = match word(f, "/name") {
                "" => "Hidden song",
                n => n,
            };
            c.text(name, 144.0, y + 80.0, 25.0, true, TEXT, half - 120.0, Align::Left);
            let mut about = vec![word(f, "/artist").to_string(), instrument(word(f, "/instrument")).to_string()];
            if let Some(sp) = num(f, "/speed").filter(|s| *s != 100.0) {
                about.push(format!("{sp:.0}% speed"));
            }
            about.extend(f.pointer("/modifiers").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(String::from));
            about.retain(|t| !t.is_empty());
            c.text(&about.join("  ·  "), 144.0, y + 110.0, 18.0, false, MUTED, half - 120.0, Align::Left);
        }
        None => c.text("No FC on a rated chart yet", 62.0, y + 84.0, 22.0, false, MUTED, half - 40.0, Align::Left),
    }
    let ax = 44.0 + half + 18.0;
    c.rounded(ax, y, half, 142.0, 10.0, RAISED, Some(LINE));
    c.text("TOP ARTISTS", ax + 18.0, y + 32.0, 16.0, true, MUTED, 300.0, Align::Left);
    let artists = s.pointer("/player/topArtists").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    for (i, a) in artists.iter().take(3).enumerate() {
        let ay = y + 64.0 + i as f32 * 30.0;
        c.text(word(a, "/artist"), ax + 18.0, ay, 21.0, i == 0, TEXT, half - 160.0, Align::Left);
        c.text(&format!("{} plays", count(num(a, "/plays"))), ax + half - 18.0, ay, 18.0, false, MUTED, 120.0, Align::Right);
    }
    if artists.is_empty() {
        c.text("None yet", ax + 18.0, y + 84.0, 22.0, false, MUTED, 300.0, Align::Left);
    }

    c.text("Truss", 44.0, 606.0, 17.0, true, TEXT, 200.0, Align::Left);
    let date = word(s, "/fetchedAt").get(..10).unwrap_or("");
    let foot = format!("average accuracy is the best run per chart  ·  ratings by Fretwork  ·  {date}");
    c.text(&foot, 1156.0, 606.0, 16.0, false, MUTED, 900.0, Align::Right);

    let (w, h) = (c.pix.width(), c.pix.height());
    Some((w, h, c.pix.take()))
}

/// Saves a player's card as a PNG.
pub fn save(l: &Loaded, path: &std::path::Path) -> Result<(), String> {
    let (w, h, px) = draw(l).ok_or("Couldn't draw the card.")?;
    let img = image::RgbaImage::from_raw(w, h, px).ok_or("Couldn't draw the card.")?;
    img.save_with_format(path, image::ImageFormat::Png).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TRUSS_TEST_CARD=<a player's kept scores (.json)>: saves their card next to it.
    #[test]
    fn draws_a_kept_player() {
        let Ok(file) = std::env::var("TRUSS_TEST_CARD") else { return };
        let p: super::super::fetch::Player = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        let stats = super::super::stats::build(&p, &super::super::ratings::Ratings::bundled());
        // A stand-in picture (wider than tall, to check the crop).
        let px: Vec<u8> = (0..300u32 * 200).flat_map(|i| [(i % 300) as u8, (i / 300) as u8, 200, 255]).collect();
        let l = Loaded { name: p.profile.username.clone(), stats, picture: None, picture_rgba: Some((300, 200, px)), runs: Default::default() };
        let out = std::path::Path::new(&file).with_extension("png");
        save(&l, &out).unwrap();
        println!("SAVED {}", out.display());
    }
}
