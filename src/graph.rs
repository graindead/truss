//! Fretwork's difficulty-over-time graph (the same one Truss Index shows): the D line and what
//! it's made of, with the values under the mouse.

use crate::curves::Curves;
use iced::widget::canvas::{self, Frame, Path, Stroke, Text};
use iced::widget::text::Alignment as TextAlign;
use iced::{Color, Point, Rectangle, Renderer, Theme, alignment, mouse};
use std::sync::Arc;

/// Seconds between samples (Fretwork's 250 ms step).
const STEP_S: f64 = crate::fretwork::STEP_MS / 1000.0;

struct Series {
    key: &'static str,
    label: &'static str,
    width: f32,
}

const SERIES: &[Series] = &[
    Series { key: "d", label: "D", width: 2.4 },
    Series { key: "d2x", label: "D with 2x kick", width: 1.6 },
    Series { key: "notes", label: "Notes", width: 1.4 },
    Series { key: "movement", label: "Movement", width: 1.4 },
    Series { key: "hands", label: "Hands", width: 1.4 },
    Series { key: "travel", label: "Travel", width: 1.4 },
    Series { key: "kicks", label: "Kicks", width: 1.4 },
];

fn color(key: &str, t: &Theme) -> Color {
    let p = t.extended_palette();
    match key {
        "d" => crate::style::accent(t),
        "d2x" => Color::from_rgb8(0xe0, 0x7b, 0xd8),
        "notes" | "hands" => p.success.base.color,
        "movement" | "travel" => Color::from_rgb8(0xe8, 0xb3, 0x3c),
        _ => p.danger.base.color,
    }
}

/// "m:ss".
fn clock(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The first step giving at most `want` ticks over `span`.
fn nice_step(span: f64, want: f64, steps: &[f64]) -> f64 {
    steps.iter().copied().find(|s| span / s <= want).unwrap_or(*steps.last().unwrap_or(&1.0))
}

pub struct Graph {
    pub curves: Arc<Curves>,
}

impl<M> canvas::Program<M> for Graph {
    type State = Option<Point>;

    fn update(&self, state: &mut Self::State, event: &canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<M>> {
        if let canvas::Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) = event {
            let now = cursor.position_in(bounds);
            if now != *state {
                *state = now;
                return Some(canvas::Action::request_redraw());
            }
        }
        None
    }

    fn draw(&self, state: &Self::State, renderer: &Renderer, theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<canvas::Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let series: Vec<(&Series, &[f64])> = SERIES
            .iter()
            .filter_map(|s| self.curves.series.iter().find(|(k, _)| *k == s.key).map(|(_, v)| (s, v.as_slice())))
            .collect();
        let n = series.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
        if n < 2 {
            return vec![f.into_geometry()];
        }
        let fg = crate::style::fg(theme);
        let muted = crate::style::muted(theme);
        let (w, h) = (bounds.width, bounds.height);
        // Legend / readout line on top, axes around the plot.
        let (ml, mr, mt, mb) = (34.0, 8.0, 26.0, 20.0);
        let (iw, ih) = ((w - ml - mr).max(10.0), (h - mt - mb).max(10.0));
        let top = series.iter().flat_map(|(_, v)| v.iter().copied()).fold(1.0f64, f64::max);
        let ymax = top * 1.08;
        let y_step = nice_step(ymax, 4.0, &[1.0, 2.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0, 200.0]);
        let dur = ((n - 1) as f64 * STEP_S).max(1.0);
        let x_step = nice_step(dur, (iw / 70.0).max(2.0) as f64, &[10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0]);
        let x = |i: usize| ml + (i as f64 * STEP_S / dur) as f32 * iw;
        let y = |v: f64| mt + ih - (v / ymax) as f32 * ih;
        let label = |f: &mut Frame, s: String, p: Point, ax: TextAlign, ay: alignment::Vertical, c: Color| {
            f.fill_text(Text { content: s, position: p, color: c, size: 11.0.into(), align_x: ax, align_y: ay, ..Default::default() });
        };

        let grid = crate::style::hairline(theme);
        let mut v = 0.0;
        while v <= ymax {
            let yy = y(v);
            f.stroke(&Path::line(Point::new(ml, yy), Point::new(w - mr, yy)), Stroke::default().with_color(grid).with_width(1.0));
            label(&mut f, format!("{v}"), Point::new(ml - 6.0, yy), TextAlign::Right, alignment::Vertical::Center, muted);
            v += y_step;
        }
        let mut t = 0.0;
        while t <= dur {
            let xx = ml + (t / dur) as f32 * iw;
            label(&mut f, clock(t), Point::new(xx, h - 2.0), TextAlign::Center, alignment::Vertical::Bottom, muted);
            t += x_step;
        }

        // About two points per pixel; the D lines go on top.
        let every = ((n as f32 / (iw * 2.0)).floor() as usize).max(1);
        for (s, vals) in series.iter().rev() {
            let path = Path::new(|b| {
                let mut first = true;
                for i in (0..vals.len()).step_by(every) {
                    let p = Point::new(x(i), y(vals[i]));
                    if first {
                        b.move_to(p);
                        first = false;
                    } else {
                        b.line_to(p);
                    }
                }
            });
            f.stroke(&path, Stroke::default().with_color(color(s.key, theme)).with_width(s.width));
        }

        // Legend, or the values at the mouse.
        let hover = state.filter(|p| p.x >= ml && p.x <= w - mr).map(|p| {
            let i = (((p.x - ml) / iw) as f64 * dur / STEP_S).round() as usize;
            i.min(n - 1)
        });
        let mut lx = ml;
        let ly = 9.0;
        if let Some(i) = hover {
            let xx = x(i);
            f.stroke(&Path::line(Point::new(xx, mt), Point::new(xx, mt + ih)), Stroke::default().with_color(muted).with_width(1.0));
            label(&mut f, clock(i as f64 * STEP_S), Point::new(lx, ly), TextAlign::Left, alignment::Vertical::Center, fg);
            lx += 40.0;
        }
        for (s, vals) in &series {
            let c = color(s.key, theme);
            f.fill_rectangle(Point::new(lx, ly - 4.0), iced::Size::new(8.0, 8.0), c);
            let text = match hover {
                Some(i) => {
                    let unit = if s.key.starts_with('d') { "" } else { "/s" };
                    format!("{} {:.1}{unit}", s.label, vals.get(i).copied().unwrap_or(0.0))
                }
                None => s.label.to_string(),
            };
            let tw = text.chars().count() as f32 * 6.2;
            label(&mut f, text, Point::new(lx + 12.0, ly), TextAlign::Left, alignment::Vertical::Center, fg);
            lx += 12.0 + tw + 14.0;
        }
        if hover.is_none()
            && let Some((_, d)) = series.iter().find(|(s, _)| s.key == "d")
        {
            let (pi, pv) = d.iter().copied().enumerate().fold((0, f64::MIN), |a, (i, v)| if v > a.1 { (i, v) } else { a });
            label(&mut f, format!("Peak D {pv:.1} at {}", clock(pi as f64 * STEP_S)), Point::new(lx, ly), TextAlign::Left, alignment::Vertical::Center, muted);
        }
        vec![f.into_geometry()]
    }
}
