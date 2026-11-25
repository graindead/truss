//! What the Player Stats tab looks like.

use super::versus::{self, Shared};
use super::{Loaded, Msg, PlayersTool, Search, Side, Slot, State};
use crate::style;
use iced::widget::canvas::{self, Canvas, Frame, Path, Stroke, Text};
use iced::widget::{Column, button, column, container, image, progress_bar, row, scrollable, space, text, text_input};
use iced::{Alignment, Border, Color, Element, Length, Point, Rectangle, Renderer, Theme, mouse};
use serde_json::Value;

type El<'a> = Element<'a, Msg>;

/// Charts listed for each player in a comparison.
const LEADS: usize = 10;
/// Colours of Fretwork tiers 0 to 6 and up, and of the number on them (as in Truss Stats).
const TIERS: [Color; 7] = [
    Color::from_rgb8(0x85, 0x8b, 0x96),
    Color::from_rgb8(0x5c, 0xc8, 0x9c),
    Color::from_rgb8(0xa6, 0xc9, 0x5e),
    Color::from_rgb8(0xf2, 0xc1, 0x4e),
    Color::from_rgb8(0xf0, 0x90, 0x4e),
    Color::from_rgb8(0xeb, 0x5a, 0x6b),
    Color::from_rgb8(0xb6, 0x7c, 0xf2),
];
const CHIP_TEXT: Color = Color::from_rgb8(0x13, 0x19, 0x24);

// ---------- Numbers and words ----------

pub(super) fn thousands(n: f64) -> String {
    let s = format!("{:.0}", n.abs());
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0.0 && out != "0" { format!("-{out}") } else { out }
}

/// 1.23B, 4.5M, or in full.
pub(super) fn big(n: f64) -> String {
    if n >= 1e9 {
        format!("{:.2}B", n / 1e9)
    } else if n >= 1e6 {
        format!("{:.1}M", n / 1e6)
    } else {
        thousands(n)
    }
}

fn num(v: &Value, path: &str) -> Option<f64> {
    v.pointer(path).and_then(Value::as_f64)
}

fn word<'a>(v: &'a Value, path: &str) -> &'a str {
    v.pointer(path).and_then(Value::as_str).unwrap_or("")
}

fn list<'a>(v: &'a Value, path: &str) -> &'a [Value] {
    v.pointer(path).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

const DASH: &str = "–";

fn show(n: Option<f64>, f: impl Fn(f64) -> String) -> String {
    n.map_or_else(|| DASH.to_string(), f)
}

pub(super) fn pct(n: Option<f64>) -> String {
    show(n, |n| format!("{n:.2}%"))
}

fn pct1(n: Option<f64>) -> String {
    show(n, |n| format!("{n:.1}%"))
}

fn count(n: Option<f64>) -> String {
    show(n, thousands)
}

pub(super) fn hours(ms: Option<f64>) -> String {
    show(ms, |m| thousands((m / 3.6e6).round()))
}

fn plural(n: f64, one: &str) -> String {
    format!("{} {one}{}", thousands(n), if n == 1.0 { "" } else { "s" })
}

fn signed(n: f64) -> String {
    format!("{}{:.1}", if n >= 0.0 { "+" } else { "−" }, n.abs())
}

pub(super) fn instrument(i: &str) -> &str {
    match i {
        "guitar" => "Guitar",
        "guitarcoop" => "Co-op guitar",
        "rhythm" => "Rhythm",
        "bass" => "Bass",
        "drums" => "Drums",
        "prodrums" => "Pro drums",
        "keys" => "Keys",
        "guitarghl" => "6-fret guitar",
        "guitarcoopghl" => "6-fret co-op",
        "rhythmghl" => "6-fret rhythm",
        "bassghl" => "6-fret bass",
        "keysghl" => "6-fret keys",
        other => other,
    }
}

fn difficulty(d: &str) -> &str {
    match d {
        "expert" => "Expert",
        "hard" => "Hard",
        "medium" => "Medium",
        "easy" => "Easy",
        other => other,
    }
}

// ---------- Pieces ----------

fn muted(t: &Theme) -> text::Style {
    text::Style { color: Some(style::muted(t)) }
}

fn accent(t: &Theme) -> text::Style {
    text::Style { color: Some(style::accent(t)) }
}

/// One line of text, cut off at its box's edge.
fn line<'a>(s: impl Into<String>, size: u32) -> text::Text<'a> {
    text(s.into()).size(size).wrapping(text::Wrapping::None)
}

fn cell<'a>(c: impl Into<El<'a>>, width: impl Into<Length>) -> El<'a> {
    container(c).width(width).clip(true).into()
}

fn note<'a>(s: impl Into<String>) -> El<'a> {
    text(s.into()).size(12).style(muted).into()
}

fn panel<'a>(c: impl Into<El<'a>>) -> El<'a> {
    container(c)
        .padding(14)
        .width(Length::Fill)
        .style(|t: &Theme| container::Style { background: Some(style::raised(t).into()), ..style::panel(t) })
        .into()
}

fn section<'a>(title: &'a str, about: impl Into<String>, body: impl Into<El<'a>>) -> El<'a> {
    let about: String = about.into();
    let mut c = column![text(title).size(16).font(crate::BOLD)].spacing(8);
    if !about.is_empty() {
        c = c.push(note(about));
    }
    c.push(body).into()
}

fn tile<'a>(value: String, label: String) -> El<'a> {
    container(column![line(value, 22).font(crate::BOLD), line(label, 12).style(muted)].spacing(2))
        .padding([10, 12])
        .width(Length::Fill)
        .clip(true)
        .style(|t: &Theme| container::Style { background: Some(style::raised(t).into()), ..style::panel(t) })
        .into()
}

/// Tiles in rows of `across`.
fn tiles<'a>(across: usize, all: Vec<El<'a>>) -> El<'a> {
    let mut out = column![].spacing(8);
    let mut all = all.into_iter().peekable();
    while all.peek().is_some() {
        let mut r = row![].spacing(8);
        for _ in 0..across {
            r = r.push(all.next().unwrap_or_else(|| space().width(Length::Fill).into()));
        }
        out = out.push(r);
    }
    out.into()
}

fn chip<'a>(tier: Option<f64>, size: f32) -> El<'a> {
    let label = tier.map_or(DASH.to_string(), |t| format!("{t:.0}"));
    container(line(label, (size * 0.6) as u32).font(crate::BOLD).color(CHIP_TEXT))
        .width(size)
        .height(size)
        .center(size)
        .style(move |t: &Theme| container::Style {
            background: Some(tier.map_or(style::muted(t), |n| TIERS[(n as usize).min(6)]).into()),
            border: Border::default().rounded(5),
            ..Default::default()
        })
        .into()
}

fn small_button<'a>(label: impl Into<String>, msg: Option<Msg>) -> El<'a> {
    button(text(label.into()).size(13)).padding([6, 12]).style(style::secondary).on_press_maybe(msg).into()
}

// ---------- The card ----------

/// The player card. `across`: tiles in a row (fewer when two cards are side by side).
fn card<'a>(l: &'a Loaded, across: usize) -> El<'a> {
    let s = &l.stats;
    let picture: El = match &l.picture {
        Some(h) => image(h.clone()).width(84).height(84).content_fit(iced::ContentFit::Cover).border_radius(12).into(),
        None => {
            let first = l.name.chars().next().map_or("?".to_string(), |c| c.to_uppercase().to_string());
            container(text(first).size(44).font(crate::BOLD).style(accent)).center(84).style(style::panel).into()
        }
    };
    let since = word(s, "/player/since");
    let since = chrono::NaiveDate::parse_from_str(since, "%Y-%m-%d").map(|d| d.format("member since %B %Y").to_string()).unwrap_or_default();
    let main = list(s, "/player/instruments").first().map(|i| format!("{} player", instrument(word(i, "/instrument")))).unwrap_or_default();
    let about = [main, since].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join("  ·  ");
    let mut head = row![
        picture,
        cell(column![line(l.name.clone(), 28).font(crate::BOLD), line(about, 13).style(muted)].spacing(4), Length::Fill),
    ]
    .spacing(16)
    .align_y(Alignment::Center);
    if let Some(r) = num(s, "/player/ranks/totalScore") {
        head = head.push(
            column![line("SCORE RANK", 11).style(muted), line(format!("#{}", thousands(r)), 28).font(crate::BOLD).style(accent)].align_x(Alignment::End),
        );
    }
    let head = container(head).padding(16).width(Length::Fill).style(|t: &Theme| container::Style {
        background: Some(style::accent_tint(t, 0.10).into()),
        border: Border { radius: 8.0.into(), ..Default::default() },
        ..Default::default()
    });
    let st = |k: &str| num(s, &format!("/player/stats/{k}"));
    let rank = |k: &str| num(s, &format!("/player/ranks/{k}")).map_or(String::new(), |r| format!("  ·  #{}", thousands(r)));
    let grid = tiles(
        across,
        vec![
            tile(pct(num(s, "/overall/simple")), "average accuracy".into()),
            tile(count(st("totalFcs")), format!("full combos{}", rank("totalFcs"))),
            tile(count(st("songsPlayed")), "songs played".into()),
            tile(show(st("totalScore"), big), "total score".into()),
            tile(count(st("totalStarsEarned")), format!("stars{}", rank("starsEarned"))),
            tile(show(st("notesHit"), big), format!("notes hit{}", rank("notesHit"))),
            tile(hours(st("totalTimePlayedMs")), format!("hours played{}", rank("hoursPlayed"))),
            tile(count(st("highestCombo")), "highest combo".into()),
        ],
    );
    let hardest: El = match list(s, "/hardestFcs").first() {
        Some(f) => row![
            chip(num(f, "/tier"), 46.0),
            cell(column![line(song(f), 16).font(crate::BOLD), line(song_about(f), 12).style(muted)].spacing(3), Length::Fill),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into(),
        None => note("No FC on a rated chart yet"),
    };
    let artists = list(s, "/player/topArtists");
    let mut top = column![].spacing(4);
    for a in artists.iter().take(3) {
        top = top.push(
            row![cell(line(word(a, "/artist"), 14), Length::Fill), line(plural(num(a, "/plays").unwrap_or(0.0), "play"), 12).style(muted)]
                .spacing(8)
                .align_y(Alignment::Center),
        );
    }
    if artists.is_empty() {
        top = top.push(note("None yet"));
    }
    let label = |s: &'static str| line(s, 11).style(muted);
    let (hardest, top) = (panel(column![label("HARDEST FC"), hardest].spacing(8)), panel(column![label("TOP ARTISTS"), top].spacing(8)));
    let foot: El = if across >= 4 { row![hardest, top].spacing(8).into() } else { column![hardest, top].spacing(8).into() };
    column![head, grid, foot].spacing(8).width(Length::Fill).into()
}

fn song(r: &Value) -> String {
    match word(r, "/name") {
        "" => "Hidden song".into(),
        n => n.into(),
    }
}

/// Artist, instrument, speed and modifiers of a run.
fn song_about(r: &Value) -> String {
    let mut v = vec![word(r, "/artist").to_string(), instrument(word(r, "/instrument")).to_string()];
    if word(r, "/difficulty") != "expert" {
        v.push(difficulty(word(r, "/difficulty")).into());
    }
    if let Some(sp) = num(r, "/speed").filter(|s| *s != 100.0) {
        v.push(format!("{sp:.0}% speed"));
    }
    v.extend(list(r, "/modifiers").iter().filter_map(Value::as_str).map(String::from));
    v.retain(|t| !t.is_empty());
    v.join("  ·  ")
}

// ---------- The stats ----------

fn run_table<'a>(rows: &'a [Value], cols: &[(&'static str, fn(&Value) -> String)]) -> El<'a> {
    if rows.is_empty() {
        return note("None yet.");
    }
    let mut head = row![space().width(26), line("Song", 12).style(muted).width(Length::Fill)].spacing(12);
    for (name, _) in cols {
        head = head.push(line(*name, 12).style(muted).width(90).align_x(Alignment::End));
    }
    let mut out = column![head].spacing(8);
    for r in rows {
        let name: El = match r.pointer("/url").and_then(Value::as_str) {
            Some(u) => button(line(song(r), 14)).padding(0).style(style::nav_button(true)).on_press(Msg::Open(u.to_string())).into(),
            None => line(song(r), 14).into(),
        };
        let mut l = row![chip(num(r, "/tier"), 26.0), cell(column![name, line(song_about(r), 11).style(muted)].spacing(1), Length::Fill)]
            .spacing(12)
            .align_y(Alignment::Center);
        for (_, f) in cols {
            l = l.push(line(f(r), 13).width(90).align_x(Alignment::End));
        }
        out = out.push(l);
    }
    panel(out)
}

fn tier_panel(t: &Value) -> El<'_> {
    let tiers = list(t, "/tiers");
    let lowest = tiers.iter().filter_map(|x| num(x, "/accuracy")).fold(100.0, f64::min);
    let lo = ((lowest / 10.0).floor() * 10.0).min(90.0);
    let mut out = column![text(instrument(word(t, "/instrument"))).size(14).font(crate::BOLD)].spacing(6);
    for x in tiers {
        let acc = num(x, "/accuracy").unwrap_or(0.0);
        out = out.push(
            row![
                chip(num(x, "/tier"), 22.0),
                container(progress_bar(lo as f32..=100.0, acc as f32).girth(8)).width(Length::Fill),
                line(pct1(Some(acc)), 13).width(52).align_x(Alignment::End),
                line(format!("{}, {}", plural(num(x, "/count").unwrap_or(0.0), "chart"), plural(num(x, "/fcs").unwrap_or(0.0), "FC")), 12)
                    .style(muted)
                    .width(140),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        );
    }
    let mut n = format!("Bars start at {lo:.0}%. ");
    if let Some(c) = num(t, "/comfort") {
        n += &format!("Comfortable up to tier {c:.0} (95% or better). ");
    }
    if let Some(c) = num(t, "/fcZone") {
        n += &format!("FCs at least a quarter of charts up to tier {c:.0}. ");
    }
    if let Some(c) = num(t, "/wall") {
        n += &format!("Drops under 90% from tier {c:.0}. ");
    }
    panel(out.push(note(n + "These need 5 charts in a tier.")))
}

fn suits(s: &Value) -> El<'_> {
    if s["enough"] != true {
        return note(format!(
            "Not enough data: this needs about {} normal-speed, unmodified Expert runs on rated charts for one instrument, and there are {} in all.",
            count(num(s, "/need")),
            count(num(s, "/used"))
        ));
    }
    let part = |title: &'static str, verdict: &str, up: bool| {
        let mut c: Column<'_, Msg> = column![text(title).size(14).font(crate::BOLD)].spacing(8);
        let mut any = false;
        for g in list(s, "/groups").iter().filter(|g| g["verdict"] == verdict) {
            any = true;
            let about = format!("{} · {} · {} average", word(g, "/detail"), plural(num(g, "/count").unwrap_or(0.0), "chart"), pct1(num(g, "/accuracy")));
            c = c.push(
                row![
                    column![text(word(g, "/label")).size(13), text(about).size(11).style(muted)].width(Length::Fill),
                    line(format!("{} pts", signed(num(g, "/difference").unwrap_or(0.0))), 14)
                        .font(crate::BOLD)
                        .style(if up { text::success } else { text::danger }),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        if !any {
            c = c.push(note("Nothing stands out clearly."));
        }
        panel(c)
    };
    row![part("Strong suits", "strong", true), part("Weak spots", "weak", false)].spacing(8).into()
}

/// Average accuracy by month.
struct Trend(Vec<(String, f64)>);

impl canvas::Program<Msg> for Trend {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, theme: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<canvas::Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        let (left, right, top, bottom) = (44.0, 30.0, 10.0, 24.0);
        let lo = self.0.iter().map(|m| m.1).fold(100.0, f64::min) - 0.5;
        let hi = (self.0.iter().map(|m| m.1).fold(0.0, f64::max) + 0.5).min(100.0);
        let (lo, hi) = (lo.floor().max(0.0), hi.ceil().max(lo.floor() + 1.0));
        let x = |i: usize| left + (w - left - right) * i as f32 / (self.0.len() - 1) as f32;
        let y = |a: f64| top + (h - top - bottom) * ((hi - a) / (hi - lo)) as f32;
        let label = |f: &mut Frame, s: String, p: Point, align: iced::widget::text::Alignment| {
            f.fill_text(Text { content: s, position: p, color: style::muted(theme), size: 11.0.into(), align_x: align, ..Default::default() });
        };
        for a in [lo, (lo + hi) / 2.0, hi] {
            let yy = y(a);
            f.stroke(&Path::line(Point::new(left, yy), Point::new(w - right, yy)), Stroke::default().with_color(style::hairline(theme)));
            label(&mut f, format!("{a}%"), Point::new(left - 6.0, yy - 7.0), iced::widget::text::Alignment::Right);
        }
        let path = Path::new(|b| {
            for (i, m) in self.0.iter().enumerate() {
                let p = Point::new(x(i), y(m.1));
                if i == 0 { b.move_to(p) } else { b.line_to(p) }
            }
        });
        f.stroke(&path, Stroke::default().with_color(style::accent(theme)).with_width(2.0));
        let step = self.0.len().div_ceil(8);
        for (i, m) in self.0.iter().enumerate() {
            f.fill(&Path::circle(Point::new(x(i), y(m.1)), 3.5), style::accent(theme));
            if i % step == 0 {
                let align = if i == 0 { iced::widget::text::Alignment::Left } else { iced::widget::text::Alignment::Center };
                label(&mut f, m.0.clone(), Point::new(x(i), h - 16.0), align);
            }
        }
        vec![f.into_geometry()]
    }
}

fn form(s: &Value) -> El<'_> {
    let months = list(s, "/form/months");
    let recent: Vec<(String, f64)> = months
        .iter()
        .skip(months.len().saturating_sub(24))
        .filter_map(|m| Some((word(m, "/month").to_string(), num(m, "/accuracy")?)))
        .collect();
    let chart: El = if recent.len() < 2 {
        note("Not enough data: scores from at least two months are needed.")
    } else {
        panel(Canvas::new(Trend(recent)).width(Length::Fill).height(200))
    };
    let n = match num(s, "/form/recent/change") {
        Some(c) => format!(
            "Allowing for difficulty, the newest {} best runs average {} points against the {} before them{}",
            count(num(s, "/form/recent/recent")),
            signed(c),
            count(num(s, "/form/recent/before")),
            if s.pointer("/form/recent/clear") == Some(&Value::Bool(true)) { "." } else { ", which is too close to call." }
        ),
        None => "Not enough rated runs to compare recent form.".into(),
    };
    column![chart, note(n)].spacing(8).into()
}

fn boards(s: &Value) -> El<'_> {
    let head = ["Instrument", "Difficulty", "Charts", "Accuracy", "By notes", "FCs", "FC rate"];
    let widths = |i: usize| if i < 2 { Length::FillPortion(3) } else { Length::FillPortion(2) };
    let line_of = |cells: Vec<String>, dim: bool| {
        let mut r = row![].spacing(12);
        for (i, c) in cells.into_iter().enumerate() {
            let t = line(c, if dim { 12 } else { 13 }).width(widths(i)).align_x(if i < 2 { Alignment::Start } else { Alignment::End });
            r = r.push(if dim { t.style(muted) } else { t });
        }
        r
    };
    let mut out = column![line_of(head.map(String::from).to_vec(), true)].spacing(6);
    for b in list(s, "/boards") {
        out = out.push(line_of(
            vec![
                instrument(word(b, "/instrument")).into(),
                difficulty(word(b, "/difficulty")).into(),
                count(num(b, "/charts")),
                pct(num(b, "/simple")),
                pct(num(b, "/weighted")),
                count(num(b, "/fcs")),
                pct1(num(b, "/fcRate")),
            ],
            false,
        ));
    }
    panel(out)
}

fn details(l: &Loaded) -> El<'_> {
    let s = &l.stats;
    let mut averages = vec![
        tile(pct(num(s, "/overall/simple")), "average (each chart counts once)".into()),
        tile(pct(num(s, "/overall/weighted")), "by notes (long charts count more)".into()),
        tile(
            pct1(num(s, "/overall/fcRate")),
            format!("FC rate: {} of {}", count(num(s, "/overall/fcs")), count(num(s, "/overall/charts"))),
        ),
    ];
    if let Some(n) = num(s, "/overall/expert/charts").filter(|n| *n > 0.0) {
        averages.push(tile(pct(num(s, "/overall/expert/simple")), format!("Expert only, {}", plural(n, "chart"))));
    }
    let (matched, ratable) = (num(s, "/match/matched"), num(s, "/match/ratable"));
    let share = matched.zip(ratable).filter(|(_, r)| *r > 0.0).map(|(m, r)| 100.0 * m / r);
    let rated = format!(
        "{} of {} Expert charts ({}) have a Fretwork rating here: {} matched by their exact notes, {} by name, artist, charter and length (likely, not certain). Unrated charts are left out below, whatever their difficulty. Ratings are from November 2026.",
        count(matched),
        count(ratable),
        show(share, |s| format!("{s:.0}%")),
        count(num(s, "/match/byHash")),
        count(num(s, "/match/byName")),
    );
    let unrated = match num(s, "/fcsUnrated").filter(|n| *n > 0.0) {
        Some(n) => format!(" {} on unrated charts couldn't be ranked.", plural(n, "FC")),
        None => String::new(),
    };
    let notes = |r: &Value| count(num(r, "/notes"));
    let date = |r: &Value| word(r, "/date").to_string();
    let acc = |r: &Value| pct1(num(r, "/accuracy"));
    let harder = |r: &Value| pct1(num(r, "/harderThan"));
    let missed = |r: &Value| count(num(r, "/missed"));
    let over = |r: &Value| count(num(r, "/excess"));
    let tiers = list(s, "/tiers");
    let mut by_tier = column![].spacing(8);
    for t in tiers {
        by_tier = by_tier.push(tier_panel(t));
    }
    if tiers.is_empty() {
        by_tier = by_tier.push(note("Not enough data."));
    }
    column![
        section(
            "Averages",
            "From the best-scoring run on each chart, so they're kinder than an average of every attempt. All instruments, difficulties and speeds.",
            column![tiles(4, averages), boards(s)].spacing(8)
        ),
        section("How much is rated", rated, space()),
        section(
            "Hardest FCs",
            format!("Rated charts only, normal speed or faster, without modifiers that make the notes easier. \u{201c}Harder than\u{201d} is the share of rated charts on that instrument with a lower Fretwork score.{unrated}"),
            run_table(list(s, "/hardestFcs"), &[("Harder than", harder), ("Notes", notes), ("Date", date)])
        ),
        section(
            "Best passes",
            "The hardest rated charts finished without failing, with at least 80% accuracy.",
            run_table(list(s, "/bestPasses"), &[("Harder than", harder), ("Accuracy", acc), ("Date", date)])
        ),
        section(
            "Near-FCs",
            format!(
                "Never full-comboed, with {} or fewer misses and overhits together ({} in all). Closest first.",
                count(num(s, "/nearFcLimit")),
                count(num(s, "/nearFcCount"))
            ),
            run_table(list(s, "/nearFcs"), &[("Missed", missed), ("Overhits", over), ("Accuracy", acc)])
        ),
        section("Longest FCs", "By number of notes, rated or not.", run_table(list(s, "/longestFcs"), &[("Notes", notes), ("Date", date)])),
        section("Accuracy by tier", "Normal speed, no modifiers, Expert, rated charts only. Tiers keep going past 6.", by_tier),
        section(
            "Strong suits and weak spots",
            "Each group of charts is compared with the player's own accuracy on other charts of the same difficulty. A group is only listed when it has at least 15 charts and the gap is bigger than chance would explain. Best runs only.",
            suits(&s["suits"])
        ),
        section(
            "Form",
            "When the standing best runs were set. Older months lose a run each time it's beaten, and months differ in how hard the charts were.",
            form(s)
        ),
    ]
    .spacing(26)
    .into()
}

// ---------- Comparing ----------

fn shared_list<'a>(title: String, rows: Vec<Shared>, flip: bool) -> El<'a> {
    let mut out = column![text(title).size(14).font(crate::BOLD)].spacing(8);
    if rows.is_empty() {
        out = out.push(note("None."));
    }
    for r in rows {
        let (me, them) = if flip { (r.b, r.a) } else { (r.a, r.b) };
        let acc = |(a, fc): (f64, bool)| if fc { "FC".to_string() } else { format!("{a:.1}%") };
        let mut about = vec![r.artist, instrument(&r.instrument).to_string()];
        if r.difficulty != "expert" {
            about.push(difficulty(&r.difficulty).into());
        }
        about.retain(|t| !t.is_empty());
        let name = if r.name.is_empty() { "Hidden song".to_string() } else { r.name };
        out = out.push(
            row![
                cell(column![line(name, 13), line(about.join("  ·  "), 11).style(muted)].spacing(1), Length::Fill),
                line(acc(me), 13).font(crate::BOLD).style(accent).width(52).align_x(Alignment::End),
                line(acc(them), 13).style(muted).width(52).align_x(Alignment::End),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }
    panel(out)
}

/// Both players' numbers, the better one of each lit.
fn table<'a>(a: &Loaded, b: &Loaded) -> El<'a> {
    let (x, y) = (&a.stats, &b.stats);
    // (what, where it is, how it's shown, 1 if higher is better, -1 if lower is, 0 if neither)
    type Row = (&'static str, &'static str, fn(Option<f64>) -> String, i8);
    let bigs: fn(Option<f64>) -> String = |n| show(n, big);
    let rank: fn(Option<f64>) -> String = |n| show(n, |n| format!("#{}", thousands(n)));
    let tier: fn(Option<f64>) -> String = |n| show(n, |n| format!("tier {n:.0}"));
    let rows: [Row; 13] = [
        ("Score rank", "/player/ranks/totalScore", rank, -1),
        ("Total score", "/player/stats/totalScore", bigs, 1),
        ("Average accuracy", "/overall/simple", pct, 1),
        ("Expert accuracy", "/overall/expert/simple", pct, 1),
        ("Accuracy by notes", "/overall/weighted", pct, 1),
        ("Full combos", "/player/stats/totalFcs", count, 1),
        ("FC rate", "/overall/fcRate", pct1, 1),
        ("Hardest FC", "/hardestFcs/0/tier", tier, 1),
        ("Stars", "/player/stats/totalStarsEarned", count, 1),
        ("Notes hit", "/player/stats/notesHit", bigs, 1),
        ("Highest combo", "/player/stats/highestCombo", count, 1),
        ("Songs played", "/player/stats/songsPlayed", count, 0),
        ("Hours played", "/player/stats/totalTimePlayedMs", hours, 0),
    ];
    let mut out = column![row![
        cell(line(a.name.clone(), 14).font(crate::BOLD), Length::FillPortion(3)),
        space().width(Length::FillPortion(2)),
        container(line(b.name.clone(), 14).font(crate::BOLD)).width(Length::FillPortion(3)).align_right(Length::FillPortion(3)).clip(true),
    ]]
    .spacing(9);
    for (label, path, f, good) in rows {
        let (p, q) = (num(x, path), num(y, path));
        let (sp, sq) = (f(p), f(q));
        // Equal as shown counts as equal.
        let lead = match (p, q) {
            (Some(p), Some(q)) if sp != sq => (p - q).signum() * f64::from(good),
            _ => 0.0,
        };
        let side = |s: String, best: bool| {
            let t = line(s, 15);
            if best { t.font(crate::BOLD).style(accent) } else { t }
        };
        out = out.push(
            row![
                side(sp, lead > 0.0).width(Length::FillPortion(3)),
                line(label, 12).style(muted).width(Length::FillPortion(2)).align_x(Alignment::Center),
                side(sq, lead < 0.0).width(Length::FillPortion(3)).align_x(Alignment::End),
            ]
            .align_y(Alignment::Center),
        );
    }
    panel(out)
}

fn comparison<'a>(a: &'a Loaded, b: &'a Loaded) -> El<'a> {
    let v = versus::compare(&a.runs, &b.runs, LEADS);
    let mut about = format!(
        "{} played by both: {} ahead on {}, {} ahead on {}, {} tied. Each one's best run, at the same speed and with the same modifiers.",
        plural(v.shared as f64, "chart"),
        a.name,
        thousands(v.a_wins as f64),
        b.name,
        thousands(v.b_wins as f64),
        thousands(v.ties as f64),
    );
    if v.unlike > 0 {
        about += &format!(" {} more were played at other speeds or with other modifiers, so they aren't counted.", thousands(v.unlike as f64));
    }
    let lists: El = if v.shared == 0 {
        space().into()
    } else {
        row![
            shared_list(format!("{} is furthest ahead", a.name), v.a_leads, false),
            shared_list(format!("{} is furthest ahead", b.name), v.b_leads, true),
        ]
        .spacing(8)
        .into()
    };
    column![
        row![card(a, 2), card(b, 2)].spacing(16),
        section("Side by side", "", table(a, b)),
        section("Head to head", about, lists),
    ]
    .spacing(26)
    .into()
}

// ---------- The page ----------

fn search_box<'a>(s: &'a Search, side: Side, hint: &'a str, label: &'a str) -> El<'a> {
    let ready = !s.running && !s.text.trim().is_empty();
    let mut c = column![
        row![
            text_input(hint, &s.text)
                .on_input(move |t| Msg::Query(side, t))
                .on_submit(Msg::Find(side))
                .size(14)
                .padding([7, 12])
                .width(260)
                .style(style::search_input),
            button(text(if s.running { "Looking…" } else { label }).size(13))
                .padding([7, 14])
                .style(button::primary)
                .on_press_maybe(ready.then_some(Msg::Find(side))),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
    ]
    .spacing(8);
    if let Some(n) = &s.note {
        c = c.push(note(n.clone()));
    }
    if !s.picks.is_empty() {
        let picks = s.picks.iter().map(|(id, n)| small_button(n.clone(), Some(Msg::Pick(side, *id))));
        c = c.push(note("Which player?")).push(row(picks).spacing(6).wrap().vertical_spacing(6));
    }
    c.into()
}

/// A player that isn't ready to show: how far it's got, or what went wrong.
fn waiting<'a>(s: &Slot, side: Side) -> El<'a> {
    match &s.state {
        State::Failed(e) => column![text(e.clone()).size(13).style(text::danger), small_button("Try again", Some(Msg::Refresh(side)))].spacing(8).into(),
        State::Loading(_, 0) => note("Loading…"),
        State::Loading(page, pages) => {
            let mut c = column![
                note(format!("Fetching scores: page {page} of {pages}")),
                container(progress_bar(0.0..=*pages as f32, *page as f32).girth(6)).max_width(420)
            ]
            .spacing(6);
            if *pages > 60 {
                c = c.push(note("That's a lot of scores. They're fetched slowly to go easy on Clone Hero's server, then kept on this computer."));
            }
            c.into()
        }
        State::Ready(_) => space().into(),
    }
}

fn fetched(l: &Loaded) -> String {
    let at = word(&l.stats, "/fetchedAt");
    let at = chrono::DateTime::parse_from_rfc3339(at).map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string());
    let more = if l.stats["truncated"] == true { " (only the newest scores: there are too many to read them all)" } else { "" };
    format!("Fetched {}{more}", at.unwrap_or_default())
}

impl PlayersTool {
    pub fn view<'a>(&'a self, me: Option<u64>) -> El<'a> {
        let is_me = |s: &Slot| Some(s.id) == me;
        let mut top = row![search_box(&self.find, Side::Main, "Clone Hero username", "Find player")].spacing(24);
        if self.main.is_some() {
            top = top.push(search_box(&self.find_other, Side::Other, "Compare with…", "Compare"));
        }
        let mut actions = row![].spacing(8).align_y(Alignment::Center);
        if let Some(m) = &self.main {
            let ready = m.ready();
            if me.is_some() && !is_me(m) {
                if self.other.is_none() {
                    actions = actions.push(small_button("Compare with me", Some(Msg::WithMe)));
                }
                actions = actions.push(small_button("My card", Some(Msg::Me)));
            }
            if self.other.is_some() {
                actions = actions.push(small_button("Stop comparing", Some(Msg::StopCompare)));
                if self.other.as_ref().is_some_and(|o| o.ready().is_some()) {
                    actions = actions.push(small_button("Refresh both", ready.map(|_| Msg::RefreshBoth)));
                }
            } else {
                actions = actions.push(small_button("Refresh", ready.map(|_| Msg::Refresh(Side::Main))));
            }
            if let Some(l) = ready.filter(|_| self.other.is_none()) {
                actions = actions.push(
                    button(text(if self.saving { "Saving…" } else { "Save card as PNG" }).size(13))
                        .padding([6, 12])
                        .style(button::primary)
                        .on_press_maybe((!self.saving).then_some(Msg::SaveCard)),
                );
                actions = actions.push(small_button("Leaderboard page ↗", Some(Msg::Open(word(&l.stats, "/player/page").to_string()))));
                actions = actions.push(note(fetched(l)));
            }
        }
        let body: El = match (&self.main, &self.other) {
            (None, _) => note(
                "Type a Clone Hero username for a player card, averages, hardest FCs and strong suits. Load a leaderboard in Clone Hero while signed in, and your own card shows here by itself.",
            ),
            (Some(m), None) => match m.ready() {
                Some(l) => column![card(l, 4), details(l)].spacing(26).into(),
                None => waiting(m, Side::Main),
            },
            (Some(m), Some(o)) => match (m.ready(), o.ready()) {
                (Some(a), Some(b)) => comparison(a, b),
                _ => column![waiting(m, Side::Main), waiting(o, Side::Other)].spacing(12).into(),
            },
        };
        let credit = note(
            "Scores come from Clone Hero's leaderboards, read by open-chl-api, an unofficial client not affiliated with Clone Hero. Difficulty is worked out with Fretwork by Staycation, for charts on Chorus Encore and RhythmVerse.",
        );
        let saved: Option<El> = self.saved.as_ref().map(|r| match r {
            Ok(p) => row![
                note(format!("Card saved: {}", p.display())),
                small_button("Show in folder", Some(Msg::ShowSaved)),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into(),
            Err(e) => text(format!("Couldn't save the card: {e}")).size(12).style(text::danger).into(),
        });
        let page = column![top, actions].push(saved).push(body).push(credit).spacing(16).max_width(1200);
        scrollable(container(page).padding(20).center_x(Length::Fill)).height(Length::Fill).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(thousands(1234567.0), "1,234,567");
        assert_eq!(thousands(999.0), "999");
        assert_eq!(big(1.5e9), "1.50B");
        assert_eq!(big(2_340_000.0), "2.3M");
        assert_eq!(pct(None), DASH);
        assert_eq!(hours(Some(7.2e6)), "2");
        assert_eq!(plural(1.0, "chart"), "1 chart");
    }
}
