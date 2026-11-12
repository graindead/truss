//! Advanced search form: editable state, validation, and view.

use crate::api::{AdvancedFilters, TextFilter};
use iced::widget::{button, checkbox, column, container, pick_list, row, text, text_input};
use iced::{Alignment, Border, Element, Font, Length, Theme};

const BOLD: Font = Font { weight: iced::font::Weight::Bold, ..Font::DEFAULT };

const TEXT_LABELS: [&str; 6] = ["Name", "Artist", "Album", "Genre", "Year", "Charter"];
const RANGE_LABELS: [&str; 4] = ["Length (min)", "Intensity", "Average NPS", "Max NPS"];
const FLAG_LABELS: [&str; 11] = [
    "Solo sections",
    "Forced notes",
    "Open notes",
    "Tap notes",
    "Lyrics",
    "Vocals",
    "Roll lanes",
    "2x kick",
    "Chart issues",
    "Video background",
    "Modchart",
];

/// Instruments the Fretwork tier filter can rate (Expert): (key, label).
pub const FW_INSTRUMENTS: [(&str, &str); 6] =
    [("guitar", "Guitar"), ("bass", "Bass"), ("drums", "Drums"), ("keys", "Keys"), ("rhythm", "Rhythm"), ("guitarcoop", "Co-op")];

/// Cut-offs for "even" and "spiky" charts on Fretwork's spread (variation of the note rate):
/// roughly the lowest and highest third of official Rock Band charts. (5-fret, drums)
/// (From 400 official charts: 5-fret thirds at 0.15-0.17 / 0.21-0.24, drums 0.12 / 0.17.)
const EVEN_BELOW: (f64, f64) = (0.16, 0.12);
const SPIKY_ABOVE: (f64, f64) = (0.23, 0.17);

/// Choices for the Fretwork filter's drop-downs.
const VS_LISTED: [&str; 5] = [
    "Any",
    "Harder than it says",
    "Much harder (2+ tiers)",
    "Easier than it says",
    "Much easier (2+ tiers)",
];
const PACING: [&str; 3] = ["Any", "Steady throughout", "Has hard spikes"];
const MOVEMENT: [&str; 4] = ["Any", "Low", "Medium", "High"];
const KICK_SPEED: [&str; 4] = ["Any", "Light", "Medium", "Heavy"];

/// Hand movement (average fret or pad changes per second) splitting official charts into
/// thirds, per instrument: (low below, high above).
fn movement_cuts(instrument: &str) -> (f64, f64) {
    match instrument {
        "bass" => (1.09, 1.71),
        "keys" => (1.11, 2.11),
        "drums" => (1.50, 1.95),
        _ => (1.97, 2.76),
    }
}
/// Kicks per second, the same way (drums).
const KICK_CUTS: (f64, f64) = (1.61, 2.10);

/// Whether `v` falls in the chosen third: 1 low, 2 middle, 3 high (0 any).
fn in_third(v: f64, level: usize, (low, high): (f64, f64)) -> bool {
    match level {
        1 => v < low,
        2 => (low..=high).contains(&v),
        3 => v > high,
        _ => true,
    }
}

/// Charts rated by Truss itself (see fretwork_filter.rs), not by Encore's search.
#[derive(Debug, Clone, PartialEq)]
pub struct FwFilter {
    pub instrument: &'static str,
    pub min: Option<u32>,
    pub max: Option<u32>,
    /// 0 any, 1 harder than the chart's listed difficulty, 2 easier, by at least `margin`.
    pub vs_listed: usize,
    pub margin: i32,
    /// Hand movement: 0 any, 1 low, 2 medium, 3 high.
    pub moves: usize,
    /// 0 any, 1 steady, 2 has spikes.
    pub shape: usize,
    /// Drums: kick speed (0 any, 1 light, 2 medium, 3 heavy), and the double-kick tier.
    pub kicks: usize,
    pub tier_2x: (Option<u32>, Option<u32>),
}

impl FwFilter {
    pub fn tier_only(instrument: &'static str, min: Option<u32>, max: Option<u32>) -> Self {
        Self { instrument, min, max, vs_listed: 0, margin: 1, moves: 0, shape: 0, kicks: 0, tier_2x: (None, None) }
    }

    /// Whether a chart's rating for `instrument` passes, given its listed difficulty.
    pub fn accepts(&self, r: &crate::fretwork::Rating, listed: Option<i32>) -> bool {
        fn within<T: PartialOrd + Copy>(v: T, (lo, hi): (Option<T>, Option<T>)) -> bool {
            lo.is_none_or(|m| v >= m) && hi.is_none_or(|m| v <= m)
        }
        let drums = self.instrument == "drums";
        let gap = listed.map(|l| r.remap as i32 - l);
        let (even, spiky) = if drums { (EVEN_BELOW.1, SPIKY_ABOVE.1) } else { (EVEN_BELOW.0, SPIKY_ABOVE.0) };
        within(r.tier, (self.min, self.max))
            && match self.vs_listed {
                1 => gap.is_some_and(|g| g >= self.margin),
                2 => gap.is_some_and(|g| g <= -self.margin),
                _ => true,
            }
            && in_third(r.movement, self.moves, movement_cuts(self.instrument))
            && match self.shape {
                1 => r.spread < even,
                2 => r.spread > spiky,
                _ => true,
            }
            && (!drums || self.kicks == 0 || r.kicks.is_some_and(|k| in_third(k, self.kicks, KICK_CUTS)))
            && (!drums || self.tier_2x == (None, None) || r.tier_2x.is_some_and(|t| within(t, self.tier_2x)))
    }
}

/// An entry of one of the Fretwork drop-downs (`choices[index]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice(&'static [&'static str], usize);

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0[self.1])
    }
}

fn choices(list: &'static [&'static str]) -> Vec<Choice> {
    (0..list.len()).map(|i| Choice(list, i)).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FwInstrument(usize);

impl std::fmt::Display for FwInstrument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(FW_INSTRUMENTS[self.0].1)
    }
}

#[derive(Debug, Clone, Default)]
struct TextRow {
    value: String,
    exact: bool,
    exclude: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AdvancedForm {
    texts: [TextRow; 6],
    ranges: [(String, String); 4],
    modified_after: String,
    hash: String,
    flags: [Option<bool>; 11],
    /// Fretwork tier range and the instrument it applies to.
    fw_min: String,
    fw_max: String,
    fw_instrument: usize,
    fw_vs: usize,
    fw_moves: usize,
    fw_shape: usize,
    fw_kicks: usize,
    fw_2x: (String, String),
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Text(usize, String),
    Exact(usize, bool),
    Exclude(usize, bool),
    Min(usize, String),
    Max(usize, String),
    Modified(String),
    Hash(String),
    CycleFlag(usize),
    FwMin(String),
    FwMax(String),
    FwInstrument(FwInstrument),
    FwVs(Choice),
    FwMoves(Choice),
    FwShape(Choice),
    FwKicks(Choice),
    /// Double-kick tier: (is it the max, text)
    Fw2x(bool, String),
    Reset,
    Search,
    Close,
}

impl AdvancedForm {
    /// Applies an edit. `Search` and `Close` are handled by the caller.
    pub fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Text(i, v) => self.texts[i].value = v,
            Msg::Exact(i, b) => self.texts[i].exact = b,
            Msg::Exclude(i, b) => self.texts[i].exclude = b,
            Msg::Min(i, v) => self.ranges[i].0 = v,
            Msg::Max(i, v) => self.ranges[i].1 = v,
            Msg::Modified(v) => self.modified_after = v,
            Msg::Hash(v) => self.hash = v,
            Msg::FwMin(v) => self.fw_min = v,
            Msg::FwMax(v) => self.fw_max = v,
            Msg::FwInstrument(i) => self.fw_instrument = i.0,
            Msg::FwVs(c) => self.fw_vs = c.1,
            Msg::FwMoves(c) => self.fw_moves = c.1,
            Msg::FwShape(c) => self.fw_shape = c.1,
            Msg::FwKicks(c) => self.fw_kicks = c.1,
            Msg::Fw2x(is_max, v) => {
                if is_max { self.fw_2x.1 = v } else { self.fw_2x.0 = v }
            }
            Msg::CycleFlag(i) => {
                self.flags[i] = match self.flags[i] {
                    None => Some(true),
                    Some(true) => Some(false),
                    Some(false) => None,
                }
            }
            Msg::Reset => *self = Self::default(),
            Msg::Search | Msg::Close => {}
        }
    }

    /// Number of filters that are set, for the button badge.
    pub fn active_count(&self) -> usize {
        self.texts.iter().filter(|t| !t.value.trim().is_empty()).count()
            + self.ranges.iter().map(|(a, b)| (!a.trim().is_empty()) as usize + (!b.trim().is_empty()) as usize).sum::<usize>()
            + (!self.modified_after.trim().is_empty()) as usize
            + (!self.hash.trim().is_empty()) as usize
            + self.flags.iter().filter(|f| f.is_some()).count()
            + [&self.fw_min, &self.fw_max].iter().filter(|s| !s.trim().is_empty()).count()
            + if self.fw_drums() {
                (self.fw_kicks != 0) as usize + [&self.fw_2x.0, &self.fw_2x.1].iter().filter(|s| !s.trim().is_empty()).count()
            } else {
                0
            }
            + (self.fw_vs != 0) as usize
            + (self.fw_moves != 0) as usize
            + (self.fw_shape != 0) as usize
    }

    fn fw_drums(&self) -> bool {
        FW_INSTRUMENTS[self.fw_instrument].0 == "drums"
    }

    /// The Fretwork filter, if any part of it is set.
    pub fn fretwork_filter(&self) -> Result<Option<FwFilter>, String> {
        fn whole(s: &str, what: &str) -> Result<Option<u32>, String> {
            let s = s.trim();
            if s.is_empty() {
                return Ok(None);
            }
            s.parse::<u32>().map(Some).map_err(|_| format!("{what}: \"{s}\" is not a whole number"))
        }
        fn ordered<T: PartialOrd>(r: (Option<T>, Option<T>), what: &str) -> Result<(Option<T>, Option<T>), String> {
            match &r {
                (Some(a), Some(b)) if a > b => Err(format!("{what}: the minimum is above the maximum")),
                _ => Ok(r),
            }
        }
        let drums = self.fw_drums();
        let tier = ordered((whole(&self.fw_min, "Fretwork tier")?, whole(&self.fw_max, "Fretwork tier")?), "Fretwork tier")?;
        let (kicks, tier_2x) = if drums {
            (self.fw_kicks, ordered((whole(&self.fw_2x.0, "Double-kick tier")?, whole(&self.fw_2x.1, "Double-kick tier")?), "Double-kick tier")?)
        } else {
            (0, (None, None))
        };
        // "Harder" / "much harder" / "easier" / "much easier" than the listed difficulty.
        let (vs_listed, margin) = match self.fw_vs {
            1 => (1, 1),
            2 => (1, 2),
            3 => (2, 1),
            4 => (2, 2),
            _ => (0, 1),
        };
        let f = FwFilter {
            instrument: FW_INSTRUMENTS[self.fw_instrument].0,
            min: tier.0,
            max: tier.1,
            vs_listed,
            margin,
            moves: self.fw_moves,
            shape: self.fw_shape,
            kicks,
            tier_2x,
        };
        let any = f != FwFilter::tier_only(f.instrument, None, None);
        Ok(any.then_some(f))
    }

    pub fn to_filters(&self) -> Result<AdvancedFilters, String> {
        let t = |i: usize| TextFilter {
            value: self.texts[i].value.trim().to_string(),
            exact: self.texts[i].exact,
            exclude: self.texts[i].exclude,
        };
        let num = |s: &str, label: &str| -> Result<Option<f64>, String> {
            let s = s.trim();
            if s.is_empty() {
                return Ok(None);
            }
            s.parse::<f64>()
                .ok()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .map(Some)
                .ok_or_else(|| format!("{label}: \"{s}\" is not a valid number"))
        };
        let r = |i: usize| -> Result<(Option<f64>, Option<f64>), String> {
            Ok((num(&self.ranges[i].0, RANGE_LABELS[i])?, num(&self.ranges[i].1, RANGE_LABELS[i])?))
        };

        let modified_after = self.modified_after.trim().to_string();
        if !modified_after.is_empty() && !is_date(&modified_after) {
            return Err("Modified after: use the format YYYY-MM-DD".into());
        }

        let hash = self
            .hash
            .split(',')
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .collect::<Vec<_>>();
        if let Some(bad) = hash.iter().find(|h| h.len() != 32 || !h.bytes().all(|b| b.is_ascii_hexdigit())) {
            return Err(format!("Hash: \"{bad}\" is not an MD5 hash (32 hex characters)"));
        }

        let (min_length, max_length) = r(0)?;
        let (min_intensity, max_intensity) = r(1)?;
        let (min_average_nps, max_average_nps) = r(2)?;
        let (min_max_nps, max_max_nps) = r(3)?;
        let f = &self.flags;

        Ok(AdvancedFilters {
            name: t(0),
            artist: t(1),
            album: t(2),
            genre: t(3),
            year: t(4),
            charter: t(5),
            min_length,
            max_length,
            min_intensity,
            max_intensity,
            min_average_nps,
            max_average_nps,
            min_max_nps,
            max_max_nps,
            modified_after,
            hash: hash.join(","),
            has_solo_sections: f[0],
            has_forced_notes: f[1],
            has_open_notes: f[2],
            has_tap_notes: f[3],
            has_lyrics: f[4],
            has_vocals: f[5],
            has_roll_lanes: f[6],
            has_2x_kick: f[7],
            has_issues: f[8],
            has_video_background: f[9],
            modchart: f[10],
        })
    }

    pub fn view<'a>(&'a self) -> Element<'a, Msg> {
        let heading = |s: &'static str| text(s).font(BOLD).size(13);
        let hint = |s: &'static str| text(s).size(11).style(|t: &Theme| text::Style { color: Some(crate::style::muted(t)) });
        let input = |placeholder: &'static str, value: &str| {
            text_input(placeholder, value).size(13).padding([5, 8]).on_submit(Msg::Search)
        };
        let range = |min: &str, max: &str, on_min: fn(String) -> Msg, on_max: fn(String) -> Msg| {
            row![
                input("Min", min).on_input(on_min).width(70),
                text("–").size(13),
                input("Max", max).on_input(on_max).width(70),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
        };

        // Song: text filters
        let label = |s: &'static str, w: f32| text(s).size(13).width(w);
        let texts = self.texts.iter().enumerate().fold(column![heading("Song")].spacing(6), |col, (i, t)| {
            let has_value = !t.value.trim().is_empty();
            col.push(
                row![
                    label(TEXT_LABELS[i], 62.0),
                    input("", &t.value).on_input(move |v| Msg::Text(i, v)).width(Length::Fill),
                    checkbox(t.exact)
                        .label("Exact")
                        .text_size(12)
                        .size(14)
                        .style(crate::style::visible_checkbox)
                        .on_toggle_maybe(has_value.then_some(move |b| Msg::Exact(i, b))),
                    checkbox(t.exclude)
                        .label("Exclude")
                        .text_size(12)
                        .size(14)
                        .style(crate::style::visible_checkbox)
                        .on_toggle_maybe(has_value.then_some(move |b| Msg::Exclude(i, b))),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
        });

        // Chart: number ranges, date and hash
        let ranges = self.ranges.iter().enumerate().fold(column![heading("Chart")].spacing(6), |col, (i, (min, max))| {
            col.push(
                row![
                    label(RANGE_LABELS[i], 100.0),
                    input("Min", min).on_input(move |v| Msg::Min(i, v)).width(70),
                    text("–").size(13),
                    input("Max", max).on_input(move |v| Msg::Max(i, v)).width(70),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
        });
        let ranges = ranges
            .push(
                row![
                    label("Modified after", 100.0),
                    input("YYYY-MM-DD", &self.modified_after).on_input(Msg::Modified).width(Length::Fill)
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .push(
                row![label("Hash (MD5)", 100.0), input("a, b, …", &self.hash).on_input(Msg::Hash).width(Length::Fill)]
                    .spacing(6)
                    .align_y(Alignment::Center),
            );

        // Fretwork: rated by Truss, not Encore
        // Each label explains itself on hover (dotted underline style via a "ⓘ").
        let fw_label = |s: &'static str, tip: &'static str| -> Element<'a, Msg> {
            if tip.is_empty() {
                return text(s).size(13).width(122).into();
            }
            iced::widget::tooltip(
                row![text(s).size(13), text("ⓘ").size(11).style(|t: &Theme| text::Style { color: Some(crate::style::muted(t)) })]
                    .spacing(4)
                    .align_y(Alignment::Center)
                    .width(122),
                container(text(tip).size(12)).padding([6, 10]).max_width(300).style(crate::style::panel_solid),
                iced::widget::tooltip::Position::Bottom,
            )
            .into()
        };
        let pick = |list: &'static [&'static str], at: usize, msg: fn(Choice) -> Msg| {
            pick_list(choices(list), Some(Choice(list, at)), msg).text_size(13).padding([5, 10]).width(Length::Fill)
        };
        let drums = self.fw_drums();
        let line =
            |l: &'static str, tip: &'static str, w: Element<'a, Msg>| row![fw_label(l, tip), w].spacing(6).align_y(Alignment::Center);
        let mut fretwork = column![
            heading("Fretwork"),
            line(
                "Instrument",
                "",
                pick_list(
                    (0..FW_INSTRUMENTS.len()).map(FwInstrument).collect::<Vec<_>>(),
                    Some(FwInstrument(self.fw_instrument)),
                    Msg::FwInstrument,
                )
                .text_size(13)
                .padding([5, 10])
                .width(Length::Fill)
                .into(),
            ),
            line(
                "Difficulty tier",
                "How hard Fretwork rates the chart on Expert. 0–6 is like the official difficulty dots; very hard charts go above 6.",
                row![
                    input("Min", &self.fw_min).on_input(Msg::FwMin).width(Length::Fill),
                    text("–").size(13),
                    input("Max", &self.fw_max).on_input(Msg::FwMax).width(Length::Fill),
                ]
                .spacing(6)
                .align_y(Alignment::Center)
                .into(),
            ),
            line(
                "Vs listed difficulty",
                "Charts Fretwork rates harder or easier than the difficulty the charter gave them (both on the 0–6 scale). Good for spotting under- or overrated charts.",
                pick(&VS_LISTED, self.fw_vs, Msg::FwVs).into(),
            ),
            line(
                "Hand movement",
                if drums {
                    "How much your hands travel between pads. High means lots of moving around the kit; low means grooves that stay put."
                } else {
                    "How much your fretting hand moves between notes. High means lots of chord changes, runs and zig-zags; low means notes that stay in place."
                },
                pick(&MOVEMENT, self.fw_moves, Msg::FwMoves).into(),
            ),
            line(
                "Pacing",
                "Steady: about as hard all the way through. Has hard spikes: easier parts with much harder sections, like a solo or a fill-heavy bridge.",
                pick(&PACING, self.fw_shape, Msg::FwShape).into(),
            ),
        ]
        .spacing(6);
        if drums {
            fretwork = fretwork
                .push(line(
                    "Kick speed",
                    "How fast the kick pedal notes come on average (single pedal).",
                    pick(&KICK_SPEED, self.fw_kicks, Msg::FwKicks).into(),
                ))
                .push(line(
                    "Double-kick tier",
                    "Fretwork's tier when double-kick (2x pedal) notes are played too. Only charts that have them.",
                    row![
                        input("Min", &self.fw_2x.0).on_input(|v| Msg::Fw2x(false, v)).width(Length::Fill),
                        text("–").size(13),
                        input("Max", &self.fw_2x.1).on_input(|v| Msg::Fw2x(true, v)).width(Length::Fill),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .into(),
                ));
        }
        let fretwork = fretwork.push(hint("Encore charts only. Hover ⓘ for what each one means."));

        // Tri-state feature flags: Any -> Yes -> No
        let flags = self.flags.iter().enumerate().fold(row![].spacing(6), |r, (i, state)| {
            let state = *state;
            let label = match state {
                None => FLAG_LABELS[i].to_string(),
                Some(true) => format!("✓ {}", FLAG_LABELS[i]),
                Some(false) => format!("✕ {}", FLAG_LABELS[i]),
            };
            r.push(
                button(text(label).size(12))
                    .padding([4, 10])
                    .on_press(Msg::CycleFlag(i))
                    .style(move |t: &Theme, s| {
                        let mut st = match state {
                            None => button::secondary(t, s),
                            Some(true) => button::success(t, s),
                            Some(false) => button::danger(t, s),
                        };
                        if state.is_none() {
                            let p = t.extended_palette();
                            st.background = Some(
                                if matches!(s, button::Status::Hovered) { p.background.strong.color } else { p.background.weak.color }
                                    .into(),
                            );
                            st.text_color = p.background.base.text;
                            st.border = Border::default().rounded(if crate::style::refined() { 5 } else { 12 }).width(1).color(p.background.strong.color);
                        } else {
                            st.border = st.border.rounded(if crate::style::refined() { 5 } else { 12 });
                        }
                        st
                    }),
            )
        });
        let flags = column![
            row![heading("Features"), hint("Click to cycle: any → must have → must not have")].spacing(10).align_y(Alignment::Center),
            flags.wrap().vertical_spacing(6),
        ]
        .spacing(8);

        let mut footer = row![].spacing(8).align_y(Alignment::Center);
        if let Some(e) = &self.error {
            footer = footer.push(text(e.clone()).size(12).style(text::danger));
        }
        footer = footer.push(iced::widget::space().width(Length::Fill)).push(
            button(text("Reset").size(13)).padding([6, 14]).style(crate::style::secondary).on_press(Msg::Reset),
        );
        footer = footer
            .push(button(text("Close").size(13)).padding([6, 14]).style(crate::style::secondary).on_press(Msg::Close))
            .push(button(text("Search").size(13)).padding([6, 18]).style(button::primary).on_press(Msg::Search));

        // Each group in its own outlined card; Features spans the width underneath.
        let card = |content: Element<'a, Msg>| container(content).padding([10, 14]).width(Length::Fill).style(crate::style::panel);
        container(
            column![
                row![
                    container(card(texts.into())).width(Length::FillPortion(5)),
                    container(card(ranges.into())).width(Length::FillPortion(4)),
                    container(card(fretwork.into())).width(Length::FillPortion(4)),
                ]
                .spacing(12),
                card(flags.into()),
                footer,
            ]
            .spacing(12),
        )
        .padding([12, 16])
        .width(Length::Fill)
        .style(|t: &Theme| container::Style {
            background: Some(t.extended_palette().background.weaker.color.into()),
            ..Default::default()
        })
        .into()
    }
}

fn is_date(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 || !parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
        return false;
    }
    let (m, d) = (parts[1].parse::<u32>().unwrap_or(0), parts[2].parse::<u32>().unwrap_or(0));
    parts[1].len() == 2 && parts[2].len() == 2 && (1..=12).contains(&m) && (1..=31).contains(&d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fretwork_tier_filter() {
        let mut f = AdvancedForm::default();
        assert_eq!(f.fretwork_filter(), Ok(None));
        f.update(Msg::FwMin("5".into()));
        f.update(Msg::FwInstrument(FwInstrument(2)));
        let fw = f.fretwork_filter().unwrap().unwrap();
        assert_eq!((fw.instrument, fw.min, fw.max), ("drums", Some(5), None));
        let r = |tier| crate::fretwork::Rating { tier, ..Default::default() };
        assert!(fw.accepts(&r(5), None) && fw.accepts(&r(9), None) && !fw.accepts(&r(4), None));
        f.update(Msg::FwMax("3".into()));
        assert!(f.fretwork_filter().is_err());
        f.update(Msg::FwMax("x".into()));
        assert!(f.fretwork_filter().is_err());
        assert_eq!(f.active_count(), 2);
    }

    #[test]
    fn other_fretwork_filters() {
        use crate::fretwork::Rating;
        let mut f = AdvancedForm::default();
        // Much harder than it says (2+ tiers), on guitar.
        f.update(Msg::FwVs(Choice(&VS_LISTED, 2)));
        let fw = f.fretwork_filter().unwrap().unwrap();
        let r = Rating { remap: 5, ..Default::default() };
        assert!(fw.accepts(&r, Some(3)) && !fw.accepts(&r, Some(4)) && !fw.accepts(&r, None));
        // Easier than it says.
        f.update(Msg::FwVs(Choice(&VS_LISTED, 3)));
        let fw = f.fretwork_filter().unwrap().unwrap();
        assert!(fw.accepts(&r, Some(6)) && !fw.accepts(&r, Some(5)));
        // High hand movement on guitar (above 2.76 changes/s) with hard spikes.
        f.update(Msg::FwVs(Choice(&VS_LISTED, 0)));
        f.update(Msg::FwMoves(Choice(&MOVEMENT, 3)));
        f.update(Msg::FwShape(Choice(&PACING, 2)));
        let fw = f.fretwork_filter().unwrap().unwrap();
        assert!(fw.accepts(&Rating { movement: 3.0, spread: 10.0, ..Default::default() }, None));
        assert!(!fw.accepts(&Rating { movement: 2.0, spread: 10.0, ..Default::default() }, None));
        assert!(!fw.accepts(&Rating { movement: 3.0, spread: 0.0, ..Default::default() }, None));
        // Drum-only filters are ignored for other instruments, and need the numbers on drums.
        f = AdvancedForm::default();
        f.update(Msg::FwKicks(Choice(&KICK_SPEED, 3)));
        assert_eq!(f.fretwork_filter(), Ok(None));
        f.update(Msg::FwInstrument(FwInstrument(2)));
        f.update(Msg::Fw2x(false, "6".into()));
        let fw = f.fretwork_filter().unwrap().unwrap();
        assert!(fw.accepts(&Rating { kicks: Some(2.5), tier_2x: Some(7), ..Default::default() }, None));
        assert!(!fw.accepts(&Rating { kicks: Some(1.8), tier_2x: Some(7), ..Default::default() }, None));
        assert!(!fw.accepts(&Rating { kicks: Some(2.5), tier_2x: None, ..Default::default() }, None));
    }
}
