//! The "Leaderboard" section in the chart details: the top 50 Clone Hero scores for the
//! picked instrument and difficulty (see leaderboard.rs).

use crate::api::Chart;
use crate::leaderboard::{self, Hashes, Track};
use crate::{App, Message, style};
use iced::widget::{button, column, container, pick_list, row, scrollable, text};
use iced::{Alignment, Border, Color, Element, Length, Task, Theme};

/// A picker entry: API key shown by its label.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Opt {
    key: String,
    label: String,
}

impl std::fmt::Display for Opt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// 1234567 → "1,234,567"
fn thousands(n: i64) -> String {
    let s = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 { format!("-{out}") } else { out }
}

fn muted(t: &Theme) -> text::Style {
    text::Style {
        color: Some(if style::refined() { style::muted(t) } else { Color { a: 0.6, ..style::fg(t) } }),
    }
}

impl App {
    /// The leaderboard to show for a chart: the one last picked if the chart has it, else the
    /// instrument/difficulty you search with, else lead guitar on Expert, else the first.
    fn board_track(&self, c: &Chart) -> Option<Track> {
        let tracks = leaderboard::tracks(c, self.computed_hashes(c));
        let same_inst = |want: &str, t: &Track| {
            t.instrument == want || (want == "drums" && t.instrument == "prodrums") || (want == "prodrums" && t.instrument == "drums")
        };
        let mut wants: Vec<(String, Option<String>)> = Vec::new();
        if let Some((i, d)) = &self.board_pick {
            wants.push((i.clone(), Some(d.clone())));
            wants.push((i.clone(), None));
        }
        if let Some(i) = &self.settings.instrument {
            wants.push((i.clone(), self.settings.difficulty.clone()));
            wants.push((i.clone(), None));
        }
        wants.push(("guitar".into(), Some("expert".into())));
        for (i, d) in &wants {
            // Exact instrument key first, then its drums/pro drums twin.
            let exact = tracks.iter().find(|t| t.instrument == *i && d.as_ref().is_none_or(|d| t.difficulty == *d));
            let twin = || tracks.iter().find(|t| same_inst(i, t) && d.as_ref().is_none_or(|d| t.difficulty == *d));
            if let Some(t) = exact.or_else(twin) {
                return Some(t.clone());
            }
        }
        tracks.into_iter().next()
    }

    /// The hashes Truss worked out for a chart, if it has.
    fn computed_hashes(&self, c: &Chart) -> Option<&crate::trackhash::Computed> {
        match self.hashes.get(&c.md5) {
            Some(Hashes::Done(Ok(cp))) => Some(cp),
            _ => None,
        }
    }

    /// Reads the selected chart's notes to work out its leaderboard hashes.
    pub fn find_scores(&mut self, md5: String) -> Task<Message> {
        let Some(c) = self.selected.and_then(|i| self.results.get(i)).filter(|c| c.md5 == md5) else { return Task::none() };
        let Some(src) = self.chart_source(c) else { return Task::none() };
        self.hashes.insert(md5.clone(), Hashes::Working);
        Task::perform(leaderboard::compute(src), move |r| Message::HashesFound(md5.clone(), r))
    }

    /// The Fretwork rating of a chart's instrument (Expert), and the text shown on hover.
    pub fn fretwork_tip(&self, c: &Chart, instrument: &str) -> (Option<crate::fretwork::Rating>, String) {
        if !matches!(instrument, "guitar" | "guitarcoop" | "rhythm" | "bass" | "keys" | "drums") {
            return (None, "Fretwork doesn't rate this instrument.".into());
        }
        match self.hashes.get(&c.md5) {
            Some(Hashes::Done(Ok(cp))) => match cp.ratings.iter().find(|(i, _)| i == instrument) {
                Some((_, r)) => (
                    Some(*r),
                    format!("Fretwork tier {} \u{b7} {} on the 0\u{2013}6 scale \u{b7} score {:.1}", r.tier, r.remap, r.d),
                ),
                None => (None, "Fretwork: this chart has no Expert notes for it.".into()),
            },
            Some(Hashes::Done(Err(_))) => (None, "Fretwork: couldn't read this chart.".into()),
            Some(Hashes::Working) => (None, "Fretwork: reading the chart\u{2026}".into()),
            None if c.notes_data.is_none() => {
                (None, "Fretwork: open Leaderboard below and click Find scores to read this chart.".into())
            }
            None => (None, "Fretwork: not rated yet.".into()),
        }
    }

    /// Reads the selected chart's notes (for leaderboard hashes and Fretwork tiers) where that's
    /// cheap, then fetches its leaderboard if the section is open and it isn't loaded.
    pub fn load_board(&mut self) -> Task<Message> {
        let Some(c) = self.selected.and_then(|i| self.results.get(i)) else { return Task::none() };
        // Encore's listed hashes are often in an old layout, so read the chart first (just its
        // header and notes). RhythmVerse charts wait for "Find scores".
        match self.hashes.get(&c.md5) {
            None if self.chart_source(c).is_some_and(|s| s.is_cheap()) => {
                let md5 = c.md5.clone();
                return self.find_scores(md5);
            }
            Some(Hashes::Working) => return Task::none(),
            _ => {}
        }
        if !self.settings.show_leaderboard {
            return Task::none();
        }
        let Some(track) = self.board_track(c) else { return Task::none() };
        match self.boards.get(&track) {
            Some(Ok(_)) => return Task::none(),
            // Failed before: try again.
            Some(Err(_)) => {
                self.boards.remove(&track);
            }
            None => {}
        }
        Task::perform(leaderboard::top(track.clone()), move |r| Message::LeaderboardLoaded(track.clone(), r))
    }

    pub fn leaderboard_section(&self, c: &Chart) -> Element<'_, Message> {
        let open = self.settings.show_leaderboard;
        let header = button(text(format!("{} Leaderboard", if open { "▾" } else { "▸" })).size(13))
            .padding([4, 0])
            .style(button::text)
            .on_press(Message::ToggleLeaderboard);
        if !open {
            return header.into();
        }
        let mut col = column![header].spacing(8);

        let state = self.hashes.get(&c.md5);
        if matches!(state, Some(Hashes::Working)) {
            let what = if c.notes_data.is_some() { "Loading scores…" } else { "Reading the chart to find its leaderboards…" };
            return col.push(text(what).size(12).style(muted)).into();
        }
        let tracks = leaderboard::tracks(c, self.computed_hashes(c));
        let Some(current) = self.board_track(c) else {
            let source = self.chart_source(c);
            let find = |label: &'static str| {
                button(text(label).size(12)).padding([3, 10]).style(style::secondary).on_press(Message::FindScores(c.md5.clone()))
            };
            return match state {
                Some(Hashes::Done(Err(e))) => col
                    .push(text(format!("Couldn't read this chart: {e}")).size(12).style(muted))
                    .push(find("Try again")),
                None if source.is_some() => col
                    .push(
                        text("RhythmVerse doesn't list what leaderboards need, so Truss reads it from the chart's notes. That means fetching its files (often just a few hundred KB, but some hosts only offer the whole song).")
                            .size(12)
                            .style(muted),
                    )
                    .push(find("Find scores")),
                _ => col.push(text("This chart has no tracks Clone Hero keeps scores for.").size(12).style(muted)),
            }
            .into();
        };

        // Instrument and difficulty pickers, from the tracks this chart has.
        let mut insts: Vec<Opt> = Vec::new();
        for t in &tracks {
            if !insts.iter().any(|o| o.key == t.instrument) {
                insts.push(Opt { key: t.instrument.clone(), label: leaderboard::instrument_label(&t.instrument) });
            }
        }
        let diffs: Vec<Opt> = tracks
            .iter()
            .filter(|t| t.instrument == current.instrument)
            .map(|t| Opt { key: t.difficulty.clone(), label: leaderboard::difficulty_label(&t.difficulty) })
            .collect();
        let cur_inst = insts.iter().find(|o| o.key == current.instrument).cloned();
        let cur_diff = diffs.iter().find(|o| o.key == current.difficulty).cloned();
        let diff_now = current.difficulty.clone();
        let inst_now = current.instrument.clone();
        let picker_style = |t: &Theme, s| if style::refined() { style::quiet_pick_list(t, s) } else { pick_list::default(t, s) };
        col = col.push(
            row![
                pick_list(insts, cur_inst, move |o: Opt| Message::PickLeaderboard(o.key, diff_now.clone()))
                    .text_size(13)
                    .padding([4, 8])
                    .width(Length::Fill)
                    .style(picker_style),
                pick_list(diffs, cur_diff, move |o: Opt| Message::PickLeaderboard(inst_now.clone(), o.key))
                    .text_size(13)
                    .padding([4, 8])
                    .width(Length::Fixed(100.0))
                    .style(picker_style),
            ]
            .spacing(6),
        );

        let board = match self.boards.get(&current) {
            None => return col.push(text("Loading scores…").size(12).style(muted)).into(),
            Some(Err(e)) => {
                return col
                    .push(text(e.clone()).size(12).style(muted))
                    .push(
                        button(text("Try again").size(12))
                            .padding([3, 10])
                            .style(style::secondary)
                            .on_press(Message::PickLeaderboard(current.instrument.clone(), current.difficulty.clone())),
                    )
                    .into();
            }
            Some(Ok(b)) => b,
        };
        if board.scores.is_empty() {
            return col.push(text("No scores yet for this chart version.").size(12).style(muted)).into();
        }

        let mut list = column![].spacing(2);
        for s in &board.scores {
            let badge: Element<_> = if s.is_pfc || s.is_fc {
                container(text(if s.is_pfc { "PFC" } else { "FC" }).size(10))
                    .padding([0, 4])
                    .style(move |t: &Theme| {
                        let gold = Color::from_rgb8(0xe0, 0xb0, 0x40);
                        container::Style {
                            text_color: Some(gold),
                            border: Border { color: gold.scale_alpha(0.6), width: 1.0, radius: 3.0.into() },
                            background: Some(style::bg(t).into()),
                            ..Default::default()
                        }
                    })
                    .into()
            } else {
                iced::widget::space().into()
            };
            let mut who = row![text(s.username.clone()).size(12).wrapping(text::Wrapping::None)]
                .spacing(6)
                .align_y(Alignment::Center);
            if !s.modifiers.is_empty() {
                who = who.push(text(s.modifiers.join(", ")).size(10).style(muted).wrapping(text::Wrapping::None));
            }
            let acc = s.accuracy.as_deref().map(|a| format!("{}%", a.trim_end_matches(".0"))).unwrap_or_default();
            list = list.push(
                row![
                    text(format!("{}.", s.placement)).size(12).width(24).style(muted),
                    container(who).width(Length::Fill).clip(true),
                    container(badge).width(30).center_x(30),
                    text(acc).size(12).width(44).align_x(iced::alignment::Horizontal::Right).style(muted),
                    text(thousands(s.total_score)).size(12).width(64).align_x(iced::alignment::Horizontal::Right),
                ]
                .spacing(4)
                .align_y(Alignment::Center)
                .padding(iced::Padding { top: 3.0, bottom: 3.0, left: 4.0, right: 10.0 }),
            );
        }
        col = col.push(
            container(scrollable(list).spacing(4).height(Length::Shrink))
                .max_height(300)
                .padding(4)
                .style(style::panel),
        );

        // The player count covers every speed and modifier, the list only normal runs.
        let players = format!(
            "Top {} · {} {} on all modes",
            board.scores.len(),
            thousands(board.players as i64),
            if board.players == 1 { "player" } else { "players" }
        );
        let mut footer = row![text(players).size(11).style(muted).width(Length::Fill)].align_y(Alignment::Center);
        if let Some(page) = &board.page {
            footer = footer.push(
                button(text("View on website ↗").size(11)).padding([2, 6]).style(style::subtle).on_press(Message::OpenUrl(page.clone())),
            );
        }
        col.push(footer).into()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn thousands() {
        assert_eq!(super::thousands(426378), "426,378");
        assert_eq!(super::thousands(1_000), "1,000");
        assert_eq!(super::thousands(999), "999");
        assert_eq!(super::thousands(0), "0");
    }
}
