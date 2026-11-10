//! Alternative browse layouts, chosen in Settings > Appearance.

use crate::api::{Chart, SortKey, strip_tags};
use crate::{App, Art, BOLD, DlStatus, Message, preview_panel};
use iced::widget::{
    button, column, container, grid, image, row, rule, scrollable, space, text,
};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Hash)]
pub enum UiVariant {
    Default,
    Grid,
    Compact,
    Hero,
}

impl UiVariant {
    pub const ALL: [UiVariant; 4] = [Self::Default, Self::Grid, Self::Compact, Self::Hero];

    pub fn from_key(key: &str) -> Self {
        match key {
            "grid" => Self::Grid,
            "compact" => Self::Compact,
            "hero" => Self::Hero,
            _ => Self::Default,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Default => "list",
            Self::Grid => "grid",
            Self::Compact => "compact",
            Self::Hero => "hero",
        }
    }
}

impl std::fmt::Display for UiVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Default => "List with sidebar",
            Self::Grid => "Album art grid",
            Self::Compact => "Compact table",
            Self::Hero => "Banner (no sidebar)",
        })
    }
}

const INSTRUMENT_BADGES: &[(&str, &str)] = &[
    ("guitar", "G"),
    ("bass", "B"),
    ("drums", "D"),
    ("keys", "K"),
    ("vocals", "V"),
    ("guitarghl", "6"),
];

fn dim(t: &Theme) -> Color {
    let mut c = t.extended_palette().background.base.text;
    c.a = 0.6;
    c
}

fn s(o: &Option<String>) -> String {
    o.as_deref().map(strip_tags).unwrap_or_default()
}

impl App {
    fn status_line(&self) -> Option<String> {
        if let Some(e) = &self.error {
            Some(if e.starts_with("No chart sources") { e.clone() } else { format!("Error: {e}") })
        } else if self.searching() {
            Some("Searching…".into())
        } else if self.results.is_empty() {
            Some("No charts found".into())
        } else if !self.more_available() {
            let n = self.results.len();
            Some(format!("{n} chart{}", if n == 1 { "" } else { "s" }))
        } else {
            None
        }
    }

    /// `prefer_full`: use the full-size cover when it's loaded (banner); tiles use thumbnails.
    fn art_or_placeholder(&self, c: &Chart, size: f32, prefer_full: bool) -> Element<'static, Message> {
        // Charter/pack icon in the corner, scaled with the art (64px on a ~328px cover).
        let (icon, pad) = ((size * 0.2).round().max(24.0), (size * 0.035).round().max(4.0));
        match c.album_art_md5.as_ref().and_then(|m| self.art.get(m)) {
            Some(Art::Ready { full: Some(full), .. }) if prefer_full => {
                self.with_icon(image(full.clone()).width(size).height(size).into(), c, icon, pad)
            }
            Some(Art::Ready { thumb, .. }) => {
                self.with_icon(image(thumb.clone()).width(size).height(size).into(), c, icon, pad)
            }
            _ => container(space())
                .width(size)
                .height(size)
                .style(|t: &Theme| container::Style {
                    background: Some(t.extended_palette().background.strong.color.into()),
                    border: Border::default().rounded(6),
                    ..Default::default()
                })
                .into(),
        }
    }

    fn action_buttons(&self, i: usize, c: &Chart) -> Element<'_, Message> {
        let dl_state = self.downloads.iter().rev().find(|d| d.md5 == c.md5).map(|d| &d.status);
        #[cfg(feature = "rhythmverse")]
        let rv_dl = if dl_state.is_none() && !self.in_library(c) { self.rv_buttons(i, c, 14, 9) } else { None };
        #[cfg(not(feature = "rhythmverse"))]
        let rv_dl: Option<Element<'static, Message>> = None;
        let dl: Element<'_, Message> = if let Some(b) = rv_dl { container(b).width(220).into() } else { match dl_state {
            Some(DlStatus::Queued) => button(text("Queued…").size(14)).padding([9, 18]).into(),
            Some(DlStatus::Active { .. }) => button(text("Downloading…").size(14)).padding([9, 18]).into(),
            _ if self.in_library(c) => container(self.library_buttons(i, c, 14, 9)).width(300).into(),
            _ => button(text("Download").size(14)).padding([9, 22]).style(button::primary).on_press(Message::Download(i)).into(),
        } };
        let previewing = self.preview.as_ref().is_some_and(|p| p.md5 == c.md5);
        let pv = button(text(if previewing { "Stop preview" } else { "Preview" }).size(14))
            .padding([9, 18])
            .style(button::secondary)
            .on_press_maybe(
                self.can_preview(c).then_some(if previewing { Message::Preview(preview_panel::Msg::Close) } else { Message::OpenPreview(i) }),
            );
        row![dl, pv].spacing(8).into()
    }

    fn sort_chips(&self) -> Element<'static, Message> {
        use crate::api::SortDir;
        let chip = |label: &'static str, key: SortKey| {
            let active = self.sort.is_some_and(|s| s.key == key);
            let arrow = match self.sort {
                Some(s) if s.key == key && s.direction == SortDir::Asc => " ▲",
                Some(s) if s.key == key => " ▼",
                _ => "",
            };
            button(text(format!("{label}{arrow}")).size(12))
                .padding([4, 12])
                .style(move |t: &Theme, st| {
                    let mut b = if active { button::primary(t, st) } else { button::secondary(t, st) };
                    b.border = b.border.rounded(14);
                    b
                })
                .on_press(Message::SortBy(key))
        };
        row![
            text("Sort").size(12).style(|t: &Theme| text::Style { color: Some(dim(t)) }),
            chip("Name", SortKey::Name),
            chip("Artist", SortKey::Artist),
            chip("Album", SortKey::Album),
            chip("Charter", SortKey::Charter),
            chip("Length", SortKey::Length),
            space().width(Length::Fill),
            text(self.status_line().unwrap_or_else(|| format!("{} of {} charts", self.results.len(), self.total_found())))
                .size(12)
                .style(|t: &Theme| text::Style { color: Some(dim(t)) }),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .padding([8, 14])
        .into()
    }

    fn sidebar_container<'a>(&'a self, width: f32) -> Element<'a, Message> {
        let expanded = self.preview.as_ref().is_some_and(|p| p.expanded());
        container(self.view_sidebar())
            .width(if expanded { 720.0 } else { width })
            .height(Length::Fill)
            .padding(16)
            .style(|t: &Theme| container::Style {
                background: Some(t.extended_palette().background.weakest.color.into()),
                ..Default::default()
            })
            .into()
    }

    // ---------- Grid ----------

    pub fn view_browse_grid(&self) -> Element<'_, Message> {
        row![self.lazy_results(), self.sidebar_container(360.0)].height(Length::Fill).into()
    }

    fn grid_results(&self) -> Element<'static, Message> {
        let tiles = self.results.iter().enumerate().map(|(i, c)| {
            let selected = self.selected == Some(i);
            let checked = self.checked.contains(&i);
            let title: Element<'static, Message> = row![self.check_box(i)]
                .push(self.in_library(c).then(crate::library_mark))
                .push(c.is_rv().then(crate::rv_tag))
                .push(text(s(&c.name)).font(BOLD).size(13).wrapping(text::Wrapping::None))
                .spacing(5)
                .align_y(Alignment::Center)
                .into();
            let card = column![
                self.art_or_placeholder(c, 168.0, false),
                title,
                text(s(&c.artist)).size(12).wrapping(text::Wrapping::None),
                text(
                    [s(&c.charter), c.length_str()].into_iter().filter(|v| !v.is_empty()).collect::<Vec<_>>().join(" · "),
                )
                    .size(11)
                    .wrapping(text::Wrapping::None)
                    .style(|t: &Theme| text::Style { color: Some(dim(t)) }),
            ]
            .spacing(4)
            .width(168)
            .clip(true);
            button(card)
                .padding(8)
                .on_press(Message::Select(i))
                .style(move |t: &Theme, st| {
                    let p = t.extended_palette();
                    button::Style {
                        background: match (selected, st) {
                            (true, _) => Some(Background::Color(p.primary.weak.color)),
                            (false, _) if checked => Some(Background::Color(p.primary.weak.color.scale_alpha(0.45))),
                            (false, button::Status::Hovered) => Some(Background::Color(p.background.weak.color)),
                            _ => None,
                        },
                        text_color: if selected { p.primary.weak.text } else { p.background.base.text },
                        border: Border::default().rounded(10),
                        ..Default::default()
                    }
                })
                .into()
        });
        let g = grid(tiles).fluid(200).spacing(6).height(grid::aspect_ratio(184, 262));
        let results = column![
            self.sort_chips(),
            rule::horizontal(1),
            scrollable(container(g).padding([8, 10])).on_scroll(Message::Scrolled).height(Length::Fill),
        ]
        .width(Length::Fill);
        results.into()
    }

    // ---------- Compact ----------

    pub fn view_browse_compact(&self) -> Element<'_, Message> {
        row![self.lazy_results(), self.sidebar_container(300.0)].height(Length::Fill).into()
    }

    fn compact_results(&self) -> Element<'static, Message> {
        use crate::api::SortDir;
        let head = |label: &'static str, key: Option<SortKey>, w: Length| -> Element<'static, Message> {
            let arrow = match (key, self.sort) {
                (Some(k), Some(s)) if s.key == k && s.direction == SortDir::Asc => " ▲",
                (Some(k), Some(s)) if s.key == k => " ▼",
                _ => "",
            };
            let t = text(format!("{label}{arrow}")).font(BOLD).size(12);
            match key {
                Some(k) => button(t)
                    .padding(0)
                    .width(w)
                    .style(|th: &Theme, _| button::Style { text_color: th.extended_palette().background.base.text, ..Default::default() })
                    .on_press(Message::SortBy(k))
                    .into(),
                None => container(t).width(w).into(),
            }
        };
        let header = row![
            self.check_all_box(),
            head("Name", Some(SortKey::Name), Length::FillPortion(4)),
            head("Artist", Some(SortKey::Artist), Length::FillPortion(3)),
            head("Album", Some(SortKey::Album), Length::FillPortion(3)),
            head("Year", None, Length::Fixed(44.0)),
            head("Genre", None, Length::FillPortion(2)),
            head("Charter", Some(SortKey::Charter), Length::FillPortion(2)),
            head("Instruments", None, Length::Fixed(176.0)),
            head("Length", Some(SortKey::Length), Length::Fixed(52.0)),
        ]
        .spacing(10)
        .padding([7, 14]);

        let rows = self.results.iter().enumerate().map(|(i, c)| {
            let selected = self.selected == Some(i);
            let checked = self.checked.contains(&i);
            let owned = self.in_library(c);
            let cell = |v: String, w: Length| {
                container(text(v).size(12).wrapping(text::Wrapping::None)).width(w).clip(true)
            };
            let badges = INSTRUMENT_BADGES.iter().fold(row![].spacing(3), |r, (key, letter)| {
                let d = c.diff_for(key);
                r.push(
                    container(text(match d { Some(d) => format!("{letter}{d}"), None => letter.to_string() }).size(10).center())
                        .width(26)
                        .padding([1, 0])
                        .style(move |t: &Theme| {
                            let p = t.extended_palette();
                            container::Style {
                                background: Some(if d.is_some() { p.primary.weak.color } else { p.background.weak.color }.into()),
                                text_color: Some(if d.is_some() { p.primary.weak.text } else { dim(t) }),
                                border: Border::default().rounded(4),
                                ..Default::default()
                            }
                        }),
                )
            });
            button(
                row![
                    self.check_box(i),
                    if owned || c.is_rv() {
                        Element::from(
                            row![]
                                .push(owned.then(crate::library_mark))
                                .push(c.is_rv().then(crate::rv_tag))
                                .push(cell(s(&c.name), Length::Fill))
                                .spacing(5)
                                .align_y(Alignment::Center)
                                .width(Length::FillPortion(4)),
                        )
                    } else {
                        cell(s(&c.name), Length::FillPortion(4)).into()
                    },
                    cell(s(&c.artist), Length::FillPortion(3)),
                    cell(s(&c.album), Length::FillPortion(3)),
                    cell(s(&c.year), Length::Fixed(44.0)),
                    cell(s(&c.genre), Length::FillPortion(2)),
                    cell(s(&c.charter), Length::FillPortion(2)),
                    container(badges).width(176),
                    cell(c.length_str(), Length::Fixed(52.0)),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([5, 14])
            .on_press(Message::Select(i))
            .style(move |t: &Theme, st| {
                let p = t.extended_palette();
                let stripe = if i % 2 == 1 { Some(p.background.weakest.color) } else { None };
                button::Style {
                    background: match (selected, st) {
                        (true, _) => Some(p.primary.weak.color),
                        (false, _) if checked => Some(p.primary.weak.color.scale_alpha(0.45)),
                        (false, button::Status::Hovered) => Some(p.background.weak.color),
                        _ => stripe,
                    }
                    .map(Background::Color),
                    text_color: if selected { p.primary.weak.text } else { p.background.base.text },
                    ..Default::default()
                }
            })
            .into()
        });
        let mut list = column(rows);
        if let Some(st) = self.status_line() {
            list = list.push(container(text(st).size(12)).padding(12).center_x(Length::Fill));
        }
        let results = column![
            header,
            rule::horizontal(1),
            scrollable(list).on_scroll(Message::Scrolled).height(Length::Fill),
        ]
        .width(Length::Fill);
        results.into()
    }

    // ---------- Hero ----------

    pub fn view_browse_hero(&self) -> Element<'_, Message> {
        let hero: Element<'_, Message> = match self.selected.and_then(|i| self.results.get(i).map(|c| (i, c))) {
            None => container(
                text("Select a chart to see it here").size(14).style(|t: &Theme| text::Style { color: Some(dim(t)) }),
            )
            .padding(24)
            .center_x(Length::Fill)
            .into(),
            Some((i, c)) => {
                let visual: Element<'_, Message> = match &self.preview {
                    Some(p) if p.md5 == c.md5 => container(p.view(if p.expanded() { 420.0 } else { 236.0 }).map(Message::Preview))
                        .width(if p.expanded() { 760.0 } else { 420.0 })
                        .into(),
                    _ => self.art_or_placeholder(c, 236.0, true),
                };
                let meta = [s(&c.album), s(&c.year), s(&c.genre), c.length_str()]
                    .into_iter()
                    .filter(|v| !v.is_empty())
                    .collect::<Vec<_>>()
                    .join("  ·  ");
                let diffs = INSTRUMENT_BADGES.iter().filter_map(|(k, _)| c.diff_for(k).map(|d| (*k, d))).fold(
                    row![].spacing(14),
                    |r, (k, d)| {
                        let label = crate::api::INSTRUMENTS
                            .iter()
                            .find(|(ik, _)| *ik == k)
                            .map(|(_, l)| *l)
                            .unwrap_or(if k == "vocals" { "Vocals" } else { "?" });
                        let pips = (0..6).fold(row![].spacing(2), |pr, n| {
                            pr.push(container(space()).width(10).height(6).style(move |t: &Theme| container::Style {
                                background: Some(
                                    if n < d { t.extended_palette().primary.base.color } else { t.extended_palette().background.strong.color }
                                        .into(),
                                ),
                                border: Border::default().rounded(2),
                                ..Default::default()
                            }))
                        });
                        r.push(column![text(label).size(11), pips].spacing(3))
                    },
                );
                let info = column![
                    text(s(&c.name)).font(BOLD).size(28),
                    text(s(&c.artist)).size(18),
                    text(meta).size(13).style(|t: &Theme| text::Style { color: Some(dim(t)) }),
                    text(format!("Charted by {}", s(&c.charter))).size(13),
                    space().height(6),
                    diffs,
                    space().height(10),
                    self.action_buttons(i, c),
                    container(self.chart_extras(i, c)).max_width(520),
                ]
                .spacing(4)
                .width(Length::Fill);
                container(row![visual, info].spacing(24).align_y(Alignment::Center))
                    .padding(20)
                    .width(Length::Fill)
                    .style(|t: &Theme| {
                        let p = t.extended_palette();
                        container::Style {
                            background: Some(Background::Gradient(iced::Gradient::Linear(
                                iced::gradient::Linear::new(iced::Radians(std::f32::consts::FRAC_PI_2))
                                    .add_stop(0.0, p.primary.weak.color.scale_alpha(0.35))
                                    .add_stop(1.0, p.background.weakest.color),
                            ))),
                            ..Default::default()
                        }
                    })
                    .into()
            }
        };

        column![hero, rule::horizontal(1), self.lazy_results()].height(Length::Fill).into()
    }

    fn hero_results(&self) -> Element<'static, Message> {
        // The default rows, full width.
        let rows = self.results.iter().enumerate().map(|(i, c)| self.view_row(i, c));
        let mut list = column(rows).spacing(2).padding([0, 6]);
        if let Some(st) = self.status_line() {
            list = list.push(container(text(st).size(13)).padding(14).center_x(Length::Fill));
        }
        column![self.sort_chips(), scrollable(list).on_scroll(Message::Scrolled).height(Length::Fill)]
            .height(Length::Fill)
            .into()
    }

    /// Everything that changes what the results list looks like. The list is only rebuilt when
    /// this changes, so a playing preview (60+ redraws per second) doesn't rebuild every row.
    fn results_key(&self) -> impl std::hash::Hash + use<> {
        (
            self.ui,
            self.generation,
            self.results.len(),
            self.selected,
            self.sort,
            self.found,
            self.searching(),
            self.total_found(),
            self.error.clone(),
            self.art_version,
            self.icon_version,
            self.list_rev,
        )
    }

    /// The results list for the current layout, rebuilt only when `results_key` changes.
    pub fn lazy_results(&self) -> Element<'_, Message> {
        // `lazy` doesn't report its child's size, so give it an explicit fill-sized box.
        container(iced::widget::lazy(self.results_key(), move |_| match self.ui {
            UiVariant::Grid => self.grid_results(),
            UiVariant::Compact => self.compact_results(),
            UiVariant::Hero => self.hero_results(),
            UiVariant::Default => self.list_results(),
        }))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}
