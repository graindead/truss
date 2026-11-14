// Ported from CHOpt (https://github.com/GenericMadScientist/CHOpt)
// Copyright (C) 2020-2026 Raymond Wright. Rust port for Truss.
// Licensed under the GNU General Public License v3.0 or later.

//! Draws a path image in CHOpt's layout: rows of 16 beats, notes on a five-lane staff, Star
//! Power phrases in green, activations in blue (red where SP can't cover), whammy-free parts
//! of activations in yellow, and running scores under each measure.

use super::builder::{DrawnNote, PathImage, Range};
use super::song::{FLAGS_HOPO, FLAGS_TAP};
use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache};
use std::sync::{Mutex, OnceLock};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

pub const IMAGE_WIDTH: u32 = 1024;
const BEAT_WIDTH: f64 = 60.0;
const FONT_HEIGHT: f32 = 13.0;
const LEFT_MARGIN: i32 = 31;
const MARGIN: i32 = 92;
const MEASURE_HEIGHT: i32 = 61;
const TOP_MARGIN: i32 = 125;
const DIST_BETWEEN_MEASURES: i32 = MEASURE_HEIGHT + MARGIN;
const RANGE_OPACITY: f32 = 0.33333;
const SOLO_HEIGHT: i32 = 10;
/// Pixels per row of measures (for splitting the image into strips).
pub const ROW_HEIGHT: u32 = DIST_BETWEEN_MEASURES as u32;

type Rgb = [u8; 3];
const BLACK: Rgb = [0, 0, 0];
const GREY: Rgb = [160, 160, 160];
const LIGHT_GREY: Rgb = [224, 224, 224];

pub fn image_height(img: &PathImage) -> u32 {
    (TOP_MARGIN + MARGIN + DIST_BETWEEN_MEASURES * img.rows.len() as i32) as u32
}

fn fonts() -> &'static Mutex<(FontSystem, SwashCache)> {
    static FONTS: OnceLock<Mutex<(FontSystem, SwashCache)>> = OnceLock::new();
    FONTS.get_or_init(|| Mutex::new((FontSystem::new(), SwashCache::new())))
}

/// A horizontal strip [y0, y1) of the image.
struct Canvas {
    pix: Pixmap,
    y0: i32,
}

impl Canvas {
    fn h(&self) -> i32 {
        self.pix.height() as i32
    }
    fn visible(&self, top: i32, bottom: i32) -> bool {
        bottom >= self.y0 && top < self.y0 + self.h()
    }
    /// Fills [x0, x1] x [y0, y1] (inclusive, like CImg) blended at `opacity`.
    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgb, opacity: f32) {
        if x1 < x0 || y1 < y0 || !self.visible(y0, y1) {
            return;
        }
        let Some(r) = Rect::from_ltrb(x0 as f32, (y0 - self.y0) as f32, (x1 + 1) as f32, (y1 + 1 - self.y0) as f32) else { return };
        let mut paint = Paint::default();
        paint.set_color_rgba8(c[0], c[1], c[2], (opacity.clamp(0.0, 1.0) * 255.0).round() as u8);
        paint.anti_alias = false;
        self.pix.fill_rect(r, &paint, Transform::identity(), None);
    }
    fn hline(&mut self, x0: i32, x1: i32, y: i32, c: Rgb) {
        self.rect(x0, y, x1, y, c, 1.0);
    }
    fn vline(&mut self, x: i32, y0: i32, y1: i32, c: Rgb) {
        self.rect(x, y0, x, y1, c, 1.0);
    }
    fn blend_px(&mut self, x: i32, y: i32, c: Rgb, a: u8) {
        let (w, yy) = (self.pix.width() as i32, y - self.y0);
        if x < 0 || x >= w || yy < 0 || yy >= self.h() || a == 0 {
            return;
        }
        let i = ((yy * w + x) * 4) as usize;
        let d = self.pix.data_mut();
        let a = a as u32;
        for k in 0..3 {
            d[i + k] = ((c[k] as u32 * a + d[i + k] as u32 * (255 - a)) / 255) as u8;
        }
        d[i + 3] = 255;
    }
    /// Text with its top-left corner at (x, y); returns its width.
    fn text(&mut self, x: i32, y: i32, s: &str, c: Rgb, height: f32) -> i32 {
        self.text_impl(x, y, s, c, height, false)
    }
    /// Text with its top-right corner at (x, y).
    fn text_right(&mut self, x: i32, y: i32, s: &str, c: Rgb, height: f32) {
        self.text_impl(x, y, s, c, height, true);
    }
    fn text_impl(&mut self, x: i32, y: i32, s: &str, c: Rgb, height: f32, right: bool) -> i32 {
        if !self.visible(y - 4, y + height as i32 * 6) {
            return 0;
        }
        let mut guard = fonts().lock().unwrap_or_else(|e| e.into_inner());
        let (fs, cache) = &mut *guard;
        let mut buf = Buffer::new(fs, Metrics::new(height * 0.86, height));
        buf.set_size(fs, None, None);
        buf.set_text(fs, s, &Attrs::new().family(Family::SansSerif), Shaping::Advanced, None);
        buf.shape_until_scroll(fs, false);
        let width = buf.layout_runs().map(|r| r.line_w).fold(0.0, f32::max).ceil() as i32;
        let x = if right { x - width } else { x };
        let mut pixels = Vec::new();
        buf.draw(fs, cache, Color::rgb(c[0], c[1], c[2]), |px, py, _, _, col| pixels.push((px, py, col.a())));
        drop(guard);
        for (px, py, a) in pixels {
            self.blend_px(x + px, y + py, c, a);
        }
        width
    }
}

fn row_of(img: &PathImage, pos: f64) -> usize {
    img.rows.iter().position(|r| r.1 > pos).unwrap_or(img.rows.len().saturating_sub(1))
}

fn get_xy(img: &PathImage, pos: f64) -> (i32, i32) {
    let row = row_of(img, pos);
    let start = img.rows.get(row).map_or(0.0, |r| r.0);
    (LEFT_MARGIN + (BEAT_WIDTH * (pos - start)) as i32, TOP_MARGIN + MARGIN + DIST_BETWEEN_MEASURES * row as i32)
}

fn x_offset_for_row_gap(gap: f64) -> i32 {
    (LEFT_MARGIN as f64 + BEAT_WIDTH * gap + 0.01) as i32
}

fn colour_beat_range(cv: &mut Canvas, img: &PathImage, c: Rgb, range: Range, y_range: (i32, i32), opacity: f32) {
    let Some(last) = img.rows.last() else { return };
    let (mut start, end) = (range.0, range.1.min(super::processed::next_down(last.1)));
    let mut row = row_of(img, start);
    while start < end && row < img.rows.len() {
        let r = img.rows[row];
        let block_end = r.1.min(end);
        let max_x = x_offset_for_row_gap(r.1);
        let x_min = x_offset_for_row_gap(start - r.0);
        let x_max = x_offset_for_row_gap(block_end - r.0).min(max_x);
        if x_min <= x_max {
            let y = TOP_MARGIN + MARGIN + DIST_BETWEEN_MEASURES * row as i32;
            cv.rect(x_min, y + y_range.0, x_max, y + y_range.1, c, opacity);
        }
        start = block_end;
        row += 1;
    }
}

const FRET_COLOURS: [Rgb; 6] = [[0, 190, 0], [220, 20, 20], [235, 215, 0], [0, 120, 255], [255, 140, 0], [128, 0, 128]];
const SUSTAIN_COLOURS: [(Rgb, f32, (i32, i32)); 6] = [
    ([0, 255, 0], 1.0, (-3, 3)),
    ([255, 0, 0], 1.0, (12, 18)),
    ([255, 255, 0], 1.0, (27, 33)),
    ([0, 137, 255], 1.0, (42, 48)),
    ([255, 165, 0], 1.0, (57, 63)),
    ([128, 0, 128], 0.5, (7, 53)),
];

fn lane_y(lane: usize, lefty: bool) -> i32 {
    let y = 15 * lane as i32;
    if lefty { MEASURE_HEIGHT - 1 - y } else { y }
}

fn fill_path(cv: &mut Canvas, pb: PathBuilder, fill: Rgb, outline: Option<Rgb>) {
    let Some(path) = pb.finish() else { return };
    let t = Transform::from_translate(0.0, -cv.y0 as f32);
    let mut paint = Paint::default();
    paint.set_color_rgba8(fill[0], fill[1], fill[2], 255);
    paint.anti_alias = true;
    cv.pix.fill_path(&path, &paint, FillRule::Winding, t, None);
    if let Some(o) = outline {
        paint.set_color_rgba8(o[0], o[1], o[2], 255);
        cv.pix.stroke_path(&path, &paint, &Stroke { width: 1.0, ..Default::default() }, t, None);
    }
}

fn circle(cv: &mut Canvas, cx: f32, cy: f32, r: f32, fill: Rgb, outline: Option<Rgb>) {
    if let Some(p) = PathBuilder::from_circle(cx, cy, r) {
        let mut pb = PathBuilder::new();
        pb.push_path(&p);
        fill_path(cv, pb, fill, outline);
    }
}

fn star(cv: &mut Canvas, cx: f32, cy: f32, r: f32, fill: Rgb) {
    let mut pb = PathBuilder::new();
    for i in 0..10 {
        let rr = if i % 2 == 0 { r } else { r * 0.45 };
        let a = std::f32::consts::PI * (i as f32 / 5.0) - std::f32::consts::FRAC_PI_2;
        let (x, y) = (cx + rr * a.cos(), cy + rr * a.sin());
        if i == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) }
    }
    pb.close();
    fill_path(cv, pb, fill, Some([40, 40, 40]));
}

fn draw_note(cv: &mut Canvas, img: &PathImage, n: &DrawnNote) {
    // Sustains, open first so the frets lie on top.
    for i in (0..6).rev() {
        let len = n.lengths[i];
        if len <= 0.0 {
            continue;
        }
        let (c, opacity, mut range) = SUSTAIN_COLOURS[i];
        if img.lefty {
            range = (MEASURE_HEIGHT - 1 - range.1, MEASURE_HEIGHT - 1 - range.0);
        }
        colour_beat_range(cv, img, c, (n.beat, n.beat + len), range, opacity);
    }
    let (x, y) = get_xy(img, n.beat);
    if !cv.visible(y - 12, y + MEASURE_HEIGHT + 12) {
        return;
    }
    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
    let hopo = n.flags & FLAGS_HOPO != 0;
    let tap = n.flags & FLAGS_TAP != 0;
    if n.lengths[5] >= 0.0 {
        // Open note: a bar across the staff.
        let (top, bottom) = (y + 4, y + MEASURE_HEIGHT - 5);
        let fill = if n.is_sp_note { [235, 235, 235] } else { FRET_COLOURS[5] };
        cv.rect(x - 3, top, x + 3, bottom, [40, 40, 40], 1.0);
        cv.rect(x - 2, top + 1, x + 2, bottom - 1, fill, 1.0);
        if hopo || tap {
            cv.rect(x - 1, top + 2, x + 1, bottom - 2, if tap { [20, 20, 20] } else { [255, 255, 255] }, 1.0);
        }
    }
    for lane in 0..5 {
        if n.lengths[lane] < 0.0 {
            continue;
        }
        let cy = fy + lane_y(lane, img.lefty) as f32;
        let fill = FRET_COLOURS[lane];
        if n.is_sp_note {
            star(cv, fx, cy, 8.0, fill);
        } else {
            circle(cv, fx, cy, 6.0, fill, Some([40, 40, 40]));
        }
        if tap {
            circle(cv, fx, cy, 2.6, [20, 20, 20], None);
        } else if hopo {
            circle(cv, fx, cy, 2.6, [255, 255, 255], None);
        }
    }
}

const QUARTER_NOTE: [(i32, i32); 26] = [
    (4, 0), (4, 1), (4, 2), (4, 3), (4, 4), (4, 5), (4, 6), (4, 7), (4, 8), (4, 9), (4, 10), (4, 11), (3, 9),
    (3, 10), (3, 11), (3, 12), (2, 9), (2, 10), (2, 11), (2, 12), (1, 9), (1, 10), (1, 11), (1, 12), (0, 10), (0, 11),
];

fn draw_measures(cv: &mut Canvas, img: &PathImage) {
    let colour_distance = (MEASURE_HEIGHT - 1) / 4;
    for &pos in &img.beat_lines {
        let (x, y) = get_xy(img, pos);
        cv.vline(x, y, y + MEASURE_HEIGHT - 1, GREY);
    }
    for &pos in &img.half_beat_lines {
        let (x, y) = get_xy(img, pos);
        cv.vline(x, y, y + MEASURE_HEIGHT - 1, LIGHT_GREY);
    }
    for (i, r) in img.rows.iter().enumerate() {
        let y = TOP_MARGIN + DIST_BETWEEN_MEASURES * i as i32 + MARGIN;
        let x_max = LEFT_MARGIN + (BEAT_WIDTH * (r.1 - r.0)) as i32;
        for k in 1..4 {
            cv.hline(LEFT_MARGIN, x_max, y + colour_distance * k, GREY);
        }
        cv.hline(LEFT_MARGIN, x_max, y, BLACK);
        cv.hline(LEFT_MARGIN, x_max, y + MEASURE_HEIGHT - 1, BLACK);
        cv.vline(LEFT_MARGIN, y, y + MEASURE_HEIGHT - 1, BLACK);
        cv.vline(x_max, y, y + MEASURE_HEIGHT - 1, BLACK);
    }
    let n = img.measure_lines.len();
    for (i, &pos) in img.measure_lines.iter().enumerate().take(n.saturating_sub(1)) {
        let (x, y) = get_xy(img, pos);
        cv.vline(x, y, y + MEASURE_HEIGHT - 1, BLACK);
        cv.text(x, y - 18, &(i + 1).to_string(), [140, 0, 0], FONT_HEIGHT);
    }
}

fn draw_score_totals(cv: &mut Canvas, img: &PathImage) {
    for i in 0..img.base_values.len() {
        let pos = super::processed::next_down(img.measure_lines[i + 1]);
        let (x, mut y) = get_xy(img, pos);
        y += 4 + MEASURE_HEIGHT;
        if !cv.visible(y - 2, y + 45) {
            continue;
        }
        cv.text_right(x, y, &img.base_values[i].to_string(), GREY, FONT_HEIGHT);
        y += 13;
        cv.text_right(x, y, &img.score_values[i].to_string(), [0, 100, 0], FONT_HEIGHT);
        y += 13;
        if let Some(sp) = img.sp_percent_values.get(i) {
            cv.text_right(x, y, &format!("{:.2}%", 100.0 * sp), [0, 160, 160], FONT_HEIGHT);
        }
    }
}

/// Draws rows [y0, y1) of the image as RGBA pixels.
pub fn render(img: &PathImage, y0: u32, y1: u32) -> Option<(u32, u32, Vec<u8>)> {
    let h = y1.min(image_height(img)).saturating_sub(y0);
    let mut pix = Pixmap::new(IMAGE_WIDTH, h.max(1))?;
    pix.fill(tiny_skia::Color::WHITE);
    let mut cv = Canvas { pix, y0: y0 as i32 };
    // Header.
    let header = [img.song_name.clone(), img.artist.clone(), img.charter.clone(), img.difficulty.clone(), format!("Total score = {}", img.total_score)];
    for (i, line) in header.iter().enumerate() {
        cv.text(LEFT_MARGIN, LEFT_MARGIN + 23 * i as i32 - 8, line, BLACK, 21.0);
    }
    for (pos, name) in &img.practice_sections {
        let (x, y) = get_xy(img, *pos);
        cv.text(x, y - 31, name, BLACK, FONT_HEIGHT);
    }
    draw_measures(&mut cv, img);
    for (pos, bpm) in &img.bpms {
        let (x, y) = get_xy(img, *pos);
        let y = y - 44;
        cv.text(x + 7, y, &format!("= {bpm:.0}"), GREY, FONT_HEIGHT);
        for (dx, dy) in QUARTER_NOTE {
            cv.blend_px(x + dx, y + dy, GREY, 255);
        }
    }
    for (pos, num, den) in &img.time_sigs {
        let (x, y) = get_xy(img, *pos);
        let gap = MEASURE_HEIGHT / 16;
        cv.text(x + gap + 2, y - gap + 1, &num.to_string(), GREY, 34.0);
        cv.text(x + gap + 2, y - gap + 1 + MEASURE_HEIGHT / 2, &den.to_string(), GREY, 34.0);
    }
    let band = (-SOLO_HEIGHT, MEASURE_HEIGHT - 1 + SOLO_HEIGHT);
    for r in &img.solo_ranges {
        colour_beat_range(&mut cv, img, [0, 51, 128], *r, band, RANGE_OPACITY / 2.0);
    }
    for r in &img.bre_ranges {
        colour_beat_range(&mut cv, img, [127, 0, 0], *r, band, RANGE_OPACITY / 2.0);
    }
    for r in &img.unison_ranges {
        colour_beat_range(&mut cv, img, [255, 255, 0], *r, (-SOLO_HEIGHT, -1), RANGE_OPACITY / 2.0);
        colour_beat_range(&mut cv, img, [255, 255, 0], *r, (MEASURE_HEIGHT, MEASURE_HEIGHT - 1 + SOLO_HEIGHT), RANGE_OPACITY / 2.0);
    }
    for n in &img.notes {
        draw_note(&mut cv, img, n);
    }
    draw_score_totals(&mut cv, img);
    let full = (0, MEASURE_HEIGHT - 1);
    for r in &img.green_ranges {
        colour_beat_range(&mut cv, img, [0, 255, 0], *r, full, RANGE_OPACITY);
    }
    for r in &img.yellow_ranges {
        colour_beat_range(&mut cv, img, [255, 255, 0], *r, full, RANGE_OPACITY);
    }
    for r in &img.red_ranges {
        colour_beat_range(&mut cv, img, [255, 0, 0], *r, full, img.activation_opacity);
    }
    for r in &img.blue_ranges {
        colour_beat_range(&mut cv, img, [0, 0, 255], *r, full, img.activation_opacity);
    }
    let (w, h) = (cv.pix.width(), cv.pix.height());
    Some((w, h, cv.pix.take()))
}

/// Saves the whole image as a PNG.
pub fn save_png(img: &PathImage, path: &std::path::Path) -> Result<(), String> {
    let h = image_height(img);
    let (w, h, data) = render(img, 0, h).ok_or("The image is too big to draw.")?;
    image::save_buffer(path, &data, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())
}
