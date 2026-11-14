// Ported from CHOpt (https://github.com/GenericMadScientist/CHOpt), generate_sprites.py.
// Copyright (C) 2020-2026 Raymond Wright. Licensed under the GNU General Public License v3.0 or later.

//! CHOpt's five-fret note sprites, made here the same way generate_sprites.py makes the PNGs
//! CHOpt draws with: pixel templates for a circle, a star and an open-note bar, filled per lane
//! from a fixed colour table. Later lanes overwrite earlier ones (green ends up on top), exactly as
//! the script's putpixel calls do. `tests::match_chopt_pngs` compares them with CHOpt's own files.

use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shape {
    Circle,
    CircleHopo,
    CircleTap,
    Star,
    StarHopo,
    StarTap,
}

/// An RGBA sprite.
pub struct Sprite {
    pub w: i32,
    pub h: i32,
    pub rgba: Vec<u8>,
}

const CIRCLE: [[u8; 11]; 11] = [
    [0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0],
    [0, 0, 1, 2, 2, 2, 2, 2, 1, 0, 0],
    [0, 1, 2, 2, 2, 2, 2, 2, 2, 1, 0],
    [1, 2, 2, 2, 4, 4, 4, 2, 2, 2, 1],
    [1, 2, 2, 4, 3, 3, 3, 4, 2, 2, 1],
    [1, 2, 2, 4, 3, 3, 3, 4, 2, 2, 1],
    [1, 2, 2, 4, 3, 3, 3, 4, 2, 2, 1],
    [1, 2, 2, 2, 4, 4, 4, 2, 2, 2, 1],
    [0, 1, 2, 2, 2, 2, 2, 2, 2, 1, 0],
    [0, 0, 1, 2, 2, 2, 2, 2, 1, 0, 0],
    [0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0],
];

const STAR: [[u8; 13]; 14] = [
    [0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 1, 2, 1, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 1, 2, 1, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 1, 2, 2, 2, 1, 0, 0, 0, 0],
    [0, 0, 0, 0, 1, 2, 2, 2, 1, 0, 0, 0, 0],
    [1, 1, 1, 1, 2, 4, 4, 4, 2, 1, 1, 1, 1],
    [1, 2, 2, 2, 4, 3, 3, 3, 4, 2, 2, 2, 1],
    [0, 1, 2, 2, 4, 3, 3, 3, 4, 2, 2, 1, 0],
    [0, 0, 1, 2, 4, 3, 3, 3, 4, 2, 1, 0, 0],
    [0, 0, 0, 1, 2, 4, 4, 4, 2, 1, 0, 0, 0],
    [0, 0, 0, 1, 2, 2, 1, 2, 2, 1, 0, 0, 0],
    [0, 0, 1, 2, 2, 1, 0, 1, 2, 2, 1, 0, 0],
    [0, 0, 1, 2, 1, 0, 0, 0, 1, 2, 1, 0, 0],
    [0, 0, 1, 1, 0, 0, 0, 0, 0, 1, 1, 0, 0],
];

/// generate_sprites.py's OPEN_TEMPLATE: an 11-wide bar, 67 rows tall.
fn open_row(i: usize) -> [u8; 11] {
    match i {
        0 | 66 => [0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0],
        1 | 65 => [0, 0, 1, 2, 2, 2, 2, 2, 1, 0, 0],
        _ => [0, 0, 1, 2, 3, 3, 3, 2, 1, 0, 0],
    }
}
const OPEN_ROWS: usize = 67;

type Rgba = (u8, u8, u8, u8);
const fn c(r: u8, g: u8, b: u8) -> Rgba {
    (r, g, b, 255)
}
const BLACK: Rgba = c(0, 0, 0);
const WHITE: Rgba = c(255, 255, 255);
const PURPLE_HALF: Rgba = (128, 0, 128, 128);
const LILAC: Rgba = (212, 170, 212, 192);
/// Green, red, yellow, blue, orange.
const FRETS: [Rgba; 5] = [c(0, 255, 0), c(255, 0, 0), c(255, 255, 0), c(0, 137, 255), c(255, 165, 0)];

/// The colour for template value `pixel` (1-4) of `lane` (0-4 frets, 5 open), per shape.
fn colour(shape: Shape, lane: usize, pixel: u8) -> Rgba {
    let i = (pixel - 1) as usize;
    if lane == 5 {
        return match shape {
            Shape::Circle | Shape::Star => [BLACK, PURPLE_HALF, PURPLE_HALF][i],
            _ => [PURPLE_HALF, PURPLE_HALF, LILAC][i],
        };
    }
    let f = FRETS[lane];
    match shape {
        Shape::Circle | Shape::Star => [BLACK, f, f, f][i],
        Shape::CircleHopo | Shape::StarHopo => [BLACK, f, WHITE, BLACK][i],
        Shape::CircleTap => [BLACK, f, BLACK, BLACK][i],
        Shape::StarTap => [BLACK, f, BLACK, f][i],
    }
}

/// The sprite for a note with lanes `index` (bit 0 green ... bit 4 orange, bit 5 open).
pub fn make(shape: Shape, index: u32, lefty: bool) -> Sprite {
    let star = matches!(shape, Shape::Star | Shape::StarHopo | Shape::StarTap);
    let (w, h) = if star { (13, 75) } else { (11, 71) };
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let mut put = |x: i32, y: i32, col: Rgba| {
        let i = ((y * w + x) * 4) as usize;
        rgba[i..i + 4].copy_from_slice(&[col.0, col.1, col.2, col.3]);
    };
    for lane in (0..6).rev() {
        if index & (1 << lane) == 0 {
            continue;
        }
        if lane == 5 {
            let (xo, yo) = if star { (1, 4) } else { (0, 2) };
            for i in 0..OPEN_ROWS {
                for (j, &p) in open_row(i).iter().enumerate() {
                    if p != 0 {
                        put(xo + j as i32, yo + i as i32, colour(shape, lane, p));
                    }
                }
            }
            continue;
        }
        let yo = 15 * if lefty { 4 - lane as i32 } else { lane as i32 };
        if star {
            for (i, row) in STAR.iter().enumerate() {
                for (j, &p) in row.iter().enumerate() {
                    if p != 0 {
                        put(j as i32, yo + i as i32, colour(shape, lane, p));
                    }
                }
            }
        } else {
            for (i, row) in CIRCLE.iter().enumerate() {
                for (j, &p) in row.iter().enumerate() {
                    if p != 0 {
                        put(j as i32, yo + i as i32, colour(shape, lane, p));
                    }
                }
            }
        }
    }
    Sprite { w, h, rgba }
}

/// Sprites are made once and kept (CHOpt loads each PNG once too).
pub fn get(shape: Shape, index: u32, lefty: bool) -> std::sync::Arc<Sprite> {
    type Cache = Mutex<std::collections::HashMap<(Shape, u32, bool), std::sync::Arc<Sprite>>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let mut m = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    m.entry((shape, index, lefty)).or_insert_with(|| std::sync::Arc::new(make(shape, index, lefty))).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CHOPT_SPRITES=<CHOpt's resources/sprites folder>: every five-fret sprite must equal
    /// CHOpt's PNG pixel for pixel.
    #[test]
    fn match_chopt_pngs() {
        let Ok(dir) = std::env::var("CHOPT_SPRITES") else { return };
        let shapes = [
            (Shape::Circle, "circles"),
            (Shape::CircleHopo, "circle_hopos"),
            (Shape::CircleTap, "circle_taps"),
            (Shape::Star, "stars"),
            (Shape::StarHopo, "star_hopos"),
            (Shape::StarTap, "star_taps"),
        ];
        let mut checked = 0;
        for (lefty, side) in [(false, "righty"), (true, "lefty")] {
            for (shape, name) in shapes {
                for index in 1..64 {
                    let path = std::path::Path::new(&dir).join(side).join(name).join(format!("{index}.png"));
                    let png = image::open(&path).unwrap().to_rgba8();
                    let (pw, ph) = png.dimensions();
                    let ours = make(shape, index, lefty);
                    assert_eq!((ours.w as u32, ours.h as u32), (pw, ph), "{}", path.display());
                    // Transparent pixels may hold any colour; compare what CHOpt blends.
                    for (k, (a, b)) in ours.rgba.chunks(4).zip(png.as_raw().chunks(4)).enumerate() {
                        let same = if a[3] == 0 && b[3] == 0 { true } else { a == b };
                        assert!(same, "{} pixel {k}: ours {a:?}, CHOpt {b:?}", path.display());
                    }
                    checked += 1;
                }
            }
        }
        println!("{checked} sprites match CHOpt's");
    }
}
