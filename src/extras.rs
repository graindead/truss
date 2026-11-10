//! Per-chart details shown with the selected chart: "Already in your library", detected issues,
//! version picker and where the chart lives on Google Drive.

use crate::api::{Chart, strip_tags};
use crate::{App, Message};
use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Border, Element, Length, Theme};

/// Small green check shown on results you already have.
pub fn library_mark<'a>() -> Element<'a, Message> {
    if crate::style::refined() {
        return text("✓").size(13).style(|t: &Theme| text::Style { color: Some(t.extended_palette().success.base.color) }).into();
    }
    container(text("✓").size(11))
        .padding([0, 5])
        .style(|t: &Theme| {
            let p = t.extended_palette();
            container::Style {
                background: Some(p.success.weak.color.into()),
                text_color: Some(p.success.weak.text),
                border: Border::default().rounded(4),
                ..Default::default()
            }
        })
        .into()
}

/// Small "RV" tag shown on RhythmVerse results.
pub fn rv_tag<'a>() -> Element<'a, Message> {
    if crate::style::refined() {
        // A small blue tag (its own colour, so it doesn't read as an accent or a warning).
        let tag = container(text("RV").size(10).font(crate::BOLD)).padding([1, 5]).style(|t: &Theme| {
            let blue = iced::Color::from_rgb8(0x5a, 0xa9, 0xe6);
            let bg = crate::style::bg(t);
            let mix = |a: f32| iced::Color { r: bg.r + (blue.r - bg.r) * a, g: bg.g + (blue.g - bg.g) * a, b: bg.b + (blue.b - bg.b) * a, a: 1.0 };
            container::Style {
                background: Some(mix(0.22).into()),
                text_color: Some(mix(0.9)),
                border: Border::default().rounded(3),
                ..Default::default()
            }
        });
        return iced::widget::tooltip(
            tag,
            container(text("From RhythmVerse").size(12)).padding([4, 8]).style(crate::style::panel_solid),
            iced::widget::tooltip::Position::Top,
        )
        .into();
    }
    container(text("RV").size(10))
        .padding([0, 5])
        .style(|t: &Theme| {
            let p = t.extended_palette();
            container::Style {
                background: Some(p.secondary.strong.color.into()),
                text_color: Some(p.secondary.strong.text),
                border: Border::default().rounded(4),
                ..Default::default()
            }
        })
        .into()
}

fn expander<'a>(label: String, open: bool, msg: Message) -> Element<'a, Message> {
    button(text(format!("{} {label}", if open { "▾" } else { "▸" })).size(13))
        .padding([4, 0])
        .style(button::text)
        .on_press(msg)
        .into()
}

fn date(c: &Chart) -> String {
    c.modified_time.as_deref().map(|t| t.get(..10).unwrap_or(t).replace('-', "/")).unwrap_or_default()
}

impl App {
    /// Half "Redownload" (replaces the library copy once the new one has downloaded), half
    /// "Delete" (sends it to the Recycle Bin; needs a second click to confirm).
    pub fn library_buttons(&self, i: usize, c: &Chart, size: u32, pad: u16) -> Element<'_, Message> {
        let armed = self.confirm_delete.as_deref() == Some(c.md5.as_str());
        let can_delete = !self.library_paths(c).is_empty();
        let redownload = button(text("Redownload").size(size).width(Length::Fill).center())
            .width(Length::Fill)
            .padding(pad)
            .style(button::primary)
            .on_press(Message::Redownload(i));
        let delete = button(text(if armed { "Confirm" } else { "Delete" }).size(size).width(Length::Fill).center())
            .width(Length::Fill)
            .padding(pad)
            .style(move |t: &Theme, s| if armed { button::danger(t, s) } else { button::secondary(t, s) })
            .on_press_maybe(can_delete.then_some(Message::DeleteChart(i)));
        let mut col = column![row![redownload, delete].spacing(6)].spacing(4);
        if armed {
            col = col.push(text("Click again to move it to the Recycle Bin.").size(11));
        }
        if let Some(e) = &self.delete_error {
            col = col.push(text(e.clone()).size(11).style(text::danger));
        }
        col.into()
    }

    pub fn chart_extras(&self, i: usize, c: &Chart) -> Element<'_, Message> {
        let mut col = column![].spacing(6);

        if self.in_library(c) {
            let paths = self.library_paths(c);
            let folder = paths.first().map(|p| if p.is_file() { p.parent().unwrap_or(p).to_path_buf() } else { p.clone() });
            let copies = if paths.len() > 1 { format!(" ({} copies)", paths.len()) } else { String::new() };
            let mut banner = row![text(format!("✓  Already in your library{copies}")).size(13).width(Length::Fill)]
                .align_y(Alignment::Center);
            if let Some(f) = folder {
                banner = banner.push(
                    button(text("Show in folder").size(12))
                        .padding([2, 8])
                        .style(button::text)
                        .on_press(Message::OpenPath(f)),
                );
            }
            col = col.push(
                container(banner)
                    .padding([6, 10])
                    .width(Length::Fill)
                    .style(|t: &Theme| {
                        let p = t.extended_palette();
                        container::Style {
                            background: Some(p.success.weak.color.scale_alpha(0.35).into()),
                            text_color: Some(p.success.strong.color),
                            border: Border::default().rounded(6),
                            ..Default::default()
                        }
                    }),
            );
        }

        let issues = c.issue_groups();
        if !issues.is_empty() {
            let n: usize = issues.iter().map(|(_, l)| l.len()).sum();
            col = col.push(
                button(
                    text(format!("{} ⚠ Issues detected ({n})", if self.show_issues { "▾" } else { "▸" }))
                        .size(13),
                )
                .padding([4, 0])
                .style(|t: &Theme, s| button::Style {
                    text_color: t.extended_palette().warning.strong.color,
                    ..button::text(t, s)
                })
                .on_press(Message::ToggleIssues),
            );
            if self.show_issues {
                for (heading, lines) in issues {
                    col = col.push(text(heading).size(12).font(crate::BOLD));
                    for l in lines {
                        col = col.push(row![text("•").size(12), text(l).size(12)].spacing(6));
                    }
                }
            }
        }

        let versions = self.versions_of(c);
        if versions.len() > 1 {
            col = col.push(expander(format!("{} versions", versions.len()), self.show_versions, Message::ToggleVersions));
            if self.show_versions {
                for (vi, v) in versions.iter().enumerate() {
                    let current = std::sync::Arc::ptr_eq(v, &self.results[i]);
                    let label = format!(
                        "{}  ·  {}  ·  {}",
                        date(v),
                        &v.md5[..v.md5.len().min(7)],
                        strip_tags(v.charter.as_deref().unwrap_or("?"))
                    );
                    col = col.push(
                        button(text(label).size(12))
                            .width(Length::Fill)
                            .padding([4, 8])
                            .style(move |t: &Theme, s| {
                                if current { button::primary(t, s) } else { button::secondary(t, s) }
                            })
                            .on_press(Message::PickVersion(i, vi)),
                    );
                }
            }
        }

        #[cfg(feature = "rhythmverse")]
        if let Some(rv_links) = self.rv_links(c) {
            return col.push(rv_links).into();
        }
        let mut links = row![].spacing(6).align_y(Alignment::Center);
        if let Some((label, url)) = c.drive_location() {
            links = links.push(
                button(text("Google Drive ↗").size(12))
                    .padding([4, 10])
                    .style(crate::style::secondary)
                    .on_press(Message::OpenUrl(url)),
            );
            col = col.push(text(label).size(11).style(|t: &Theme| text::Style {
                color: Some(t.extended_palette().background.base.text.scale_alpha(0.6)),
            }));
        }
        links = links
            .push(
                button(text("Copy link").size(12))
                    .padding([4, 10])
                    .style(crate::style::secondary)
                    .on_press(Message::CopyText(format!("https://enchor.us/chart/{}", c.md5))),
            )
            .push(
                button(text("Copy hash").size(12))
                    .padding([4, 10])
                    .style(crate::style::secondary)
                    .on_press(Message::CopyText(c.md5.clone())),
            );
        col.push(links).into()
    }
}
