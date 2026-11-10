//! Advanced search form: editable state, validation, and view.

use crate::api::{AdvancedFilters, TextFilter};
use iced::widget::{button, checkbox, column, container, row, text, text_input};
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

    pub fn view(&self) -> Element<'_, Msg> {
        let label = |s: &'static str| text(s).size(13).width(92);
        let input = |placeholder: &'static str, value: &str| {
            text_input(placeholder, value).size(13).padding([5, 8]).on_submit(Msg::Search)
        };

        // Text filters
        let texts = self.texts.iter().enumerate().fold(column![].spacing(6), |col, (i, t)| {
            let has_value = !t.value.trim().is_empty();
            col.push(
                row![
                    label(TEXT_LABELS[i]),
                    input("", &t.value).on_input(move |v| Msg::Text(i, v)).width(Length::Fill),
                    checkbox(t.exact)
                        .label("Exact")
                        .text_size(12)
                        .size(14)
                        .on_toggle_maybe(has_value.then_some(move |b| Msg::Exact(i, b))),
                    checkbox(t.exclude)
                        .label("Exclude")
                        .text_size(12)
                        .size(14)
                        .on_toggle_maybe(has_value.then_some(move |b| Msg::Exclude(i, b))),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
        });

        // Numeric ranges + misc
        let ranges = self.ranges.iter().enumerate().fold(column![].spacing(6), |col, (i, (min, max))| {
            col.push(
                row![
                    label(RANGE_LABELS[i]),
                    input("Min", min).on_input(move |v| Msg::Min(i, v)).width(64),
                    text("–").size(13),
                    input("Max", max).on_input(move |v| Msg::Max(i, v)).width(64),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
        });
        let ranges = ranges
            .push(
                row![
                    label("Modified after"),
                    input("YYYY-MM-DD", &self.modified_after).on_input(Msg::Modified).width(Length::Fill)
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .push(
                row![label("Hash (MD5)"), input("a, b, …", &self.hash).on_input(Msg::Hash).width(Length::Fill)]
                    .spacing(6)
                    .align_y(Alignment::Center),
            );

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
                            st.border = Border::default().rounded(12).width(1).color(p.background.strong.color);
                        } else {
                            st.border = st.border.rounded(12);
                        }
                        st
                    }),
            )
        });
        let flags = column![
            text("Features").font(BOLD).size(13),
            text("Click to cycle: any → must have → must not have").size(11),
            flags.wrap().vertical_spacing(6),
        ]
        .spacing(6);

        let mut footer = row![].spacing(8).align_y(Alignment::Center);
        if let Some(e) = &self.error {
            footer = footer.push(text(e.clone()).size(12).style(text::danger));
        }
        footer = footer.push(iced::widget::space().width(Length::Fill)).push(
            button(text("Reset").size(13)).padding([6, 14]).style(button::secondary).on_press(Msg::Reset),
        );
        footer = footer
            .push(button(text("Close").size(13)).padding([6, 14]).style(button::secondary).on_press(Msg::Close))
            .push(button(text("Search").size(13)).padding([6, 18]).style(button::primary).on_press(Msg::Search));

        container(
            column![
                row![
                    container(texts).width(Length::FillPortion(5)),
                    container(ranges).width(Length::FillPortion(3)),
                    container(flags).width(Length::FillPortion(4)),
                ]
                .spacing(24),
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
