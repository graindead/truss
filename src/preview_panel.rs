//! Chart preview panel: state, messages, controls and keyboard shortcuts.

use crate::api::Chart;
use crate::audio::Player;
use crate::chart::{self, InstrumentType, instrument_type};
use crate::preview::{Highway, LoadEvent, Loaded, RenderTrack, load, prepare_track};
use crate::textures::TextureSet;
use iced::widget::{button, canvas, center, column, container, pick_list, progress_bar, row, slider, text};
use iced::{Alignment, Color, Element, Length, Subscription, Task, Theme};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub key: &'static str,
    pub label: &'static str,
}
impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label)
    }
}

fn instrument_label(i: &str) -> &'static str {
    crate::api::INSTRUMENTS.iter().find(|(k, _)| *k == i).map(|(_, l)| *l).unwrap_or("Unknown")
}
fn difficulty_label(d: &str) -> &'static str {
    crate::api::DIFFICULTIES.iter().find(|(k, _)| *k == d).map(|(_, l)| *l).unwrap_or("Unknown")
}

enum Status {
    Loading(f32),
    Ready,
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum Msg {
    Load(LoadEvent),
    Textures(InstrumentType, Result<Arc<TextureSet>, String>),
    TogglePlay,
    Scrub(f64),
    ScrubEnd,
    Volume(f32),
    Instrument(Choice),
    Difficulty(Choice),
    Frame,
    Key(iced::keyboard::Event),
    ToggleExpand,
    Close,
}

pub struct Panel {
    pub md5: String,
    title: String,
    status: Status,
    loaded: Option<Arc<Loaded>>,
    instrument: &'static str,
    difficulty: &'static str,
    track: Option<RenderTrack>,
    textures: Option<Arc<TextureSet>>,
    volume: f32,
    muted: bool,
    expanded: bool,
    scrub: Option<f64>,
    anim_origin: Instant,
    pref_instrument: Option<String>,
    pref_difficulty: Option<String>,
    load_handle: iced::task::Handle,
}

impl Drop for Panel {
    fn drop(&mut self) {
        self.load_handle.abort();
    }
}

impl Panel {
    pub fn open(
        chart: Arc<Chart>,
        pref_instrument: Option<String>,
        pref_difficulty: Option<String>,
        volume: f32,
        keep_video: bool,
    ) -> (Self, Task<Msg>) {
        let title = format!(
            "{} — {}",
            crate::api::strip_tags(chart.name.as_deref().unwrap_or("?")),
            crate::api::strip_tags(chart.artist.as_deref().unwrap_or("?"))
        );
        // Fetch textures for the likely instrument while the chart downloads.
        let guess = instrument_type(pref_instrument.as_deref().unwrap_or("guitar"));
        let (load_task, load_handle) = Task::run(load(chart.clone(), keep_video), Msg::Load).abortable();
        let tex = Task::perform(crate::textures::get(guess), move |r| Msg::Textures(guess, r));
        let panel = Self {
            md5: chart.md5.clone(),
            title,
            status: Status::Loading(0.0),
            loaded: None,
            instrument: "guitar",
            difficulty: "expert",
            track: None,
            textures: crate::textures::cached(guess),
            volume,
            muted: false,
            expanded: false,
            scrub: None,
            anim_origin: Instant::now(),
            pref_instrument,
            pref_difficulty,
            load_handle,
        };
        (panel, Task::batch([load_task, tex]))
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    fn player(&self) -> Option<&Player> {
        self.loaded.as_ref().map(|l| &l.player)
    }

    pub fn is_playing(&self) -> bool {
        self.player().is_some_and(|p| p.is_playing())
    }

    fn available(&self) -> Vec<(&'static str, &'static str)> {
        let Some(l) = &self.loaded else { return Vec::new() };
        l.parsed.tracks.iter().filter(|t| !t.note_groups.is_empty()).map(|t| (t.instrument, t.difficulty)).collect()
    }

    /// Selects a track and makes sure textures for its instrument type are loaded.
    fn select(&mut self, instrument: &'static str, difficulty: &'static str) -> Task<Msg> {
        self.instrument = instrument;
        self.difficulty = difficulty;
        self.track = self.loaded.as_ref().and_then(|l| prepare_track(&l.parsed, instrument, difficulty));
        let itype = instrument_type(instrument);
        if self.textures.as_ref().is_some_and(|t| t.itype == itype) {
            return Task::none();
        }
        self.textures = crate::textures::cached(itype);
        if self.textures.is_some() {
            Task::none()
        } else {
            Task::perform(crate::textures::get(itype), move |r| Msg::Textures(itype, r))
        }
    }

    fn apply_volume(&self) {
        if let Some(p) = self.player() {
            p.set_volume(if self.muted { 0.0 } else { self.volume });
        }
    }

    fn seek_relative(&self, delta_ms: f64) {
        if let Some(p) = self.player() {
            p.seek(p.position_ms() + delta_ms);
        }
    }

    /// Returns `true` as the second value when the panel should close.
    pub fn update(&mut self, msg: Msg) -> (Task<Msg>, bool) {
        match msg {
            Msg::Load(LoadEvent::Progress(p)) => {
                if let Status::Loading(_) = self.status {
                    self.status = Status::Loading(p);
                }
            }
            Msg::Load(LoadEvent::Done(Ok(loaded))) => {
                self.loaded = Some(loaded);
                let avail = self.available();
                let instr = self
                    .pref_instrument
                    .as_deref()
                    .and_then(|p| avail.iter().find(|(i, _)| *i == p))
                    .or_else(|| chart::INSTRUMENTS.iter().find_map(|i| avail.iter().find(|(ai, _)| ai == i)))
                    .map(|(i, _)| *i);
                let Some(instr) = instr else {
                    self.status = Status::Failed("This chart has no playable tracks".into());
                    return (Task::none(), false);
                };
                let diff = self
                    .pref_difficulty
                    .as_deref()
                    .and_then(|p| avail.iter().find(|(i, d)| *i == instr && *d == p))
                    .or_else(|| {
                        chart::DIFFICULTIES.iter().find_map(|d| avail.iter().find(|(i, ad)| *i == instr && ad == d))
                    })
                    .map(|(_, d)| *d)
                    .unwrap_or("expert");
                let task = self.select(instr, diff);
                self.status = Status::Ready;
                self.anim_origin = Instant::now();
                self.apply_volume();
                if let Some(p) = self.player() {
                    p.play();
                }
                return (task, false);
            }
            Msg::Load(LoadEvent::Done(Err(e))) => self.status = Status::Failed(e),
            Msg::Textures(itype, Ok(t)) => {
                if instrument_type(self.instrument) == itype {
                    self.textures = Some(t);
                }
            }
            Msg::Textures(itype, Err(e)) => {
                if instrument_type(self.instrument) == itype && self.textures.is_none() {
                    self.status = Status::Failed(format!("Couldn't load preview graphics: {e}"));
                }
            }
            Msg::TogglePlay => {
                if let Some(p) = self.player() {
                    if p.is_playing() {
                        p.pause()
                    } else {
                        p.play()
                    }
                }
            }
            Msg::Scrub(ms) => self.scrub = Some(ms),
            Msg::ScrubEnd => {
                if let (Some(ms), Some(p)) = (self.scrub.take(), self.player()) {
                    p.seek(ms);
                }
            }
            Msg::Volume(v) => {
                self.volume = v;
                self.muted = false;
                self.apply_volume();
            }
            Msg::Instrument(c) => {
                let avail = self.available();
                let diff = if avail.contains(&(c.key, self.difficulty)) {
                    self.difficulty
                } else {
                    chart::DIFFICULTIES.iter().copied().find(|d| avail.contains(&(c.key, *d))).unwrap_or("expert")
                };
                return (self.select(c.key, diff), false);
            }
            Msg::Difficulty(c) => return (self.select(self.instrument, c.key), false),
            Msg::Frame => {}
            Msg::Key(iced::keyboard::Event::KeyPressed { key, .. }) => {
                use iced::keyboard::{Key, key::Named};
                match key.as_ref() {
                    Key::Named(Named::Space) => return self.update(Msg::TogglePlay),
                    Key::Named(Named::ArrowLeft) => self.seek_relative(-5000.0),
                    Key::Named(Named::ArrowRight) => self.seek_relative(5000.0),
                    Key::Named(Named::ArrowUp) => return self.update(Msg::Volume((self.volume + 0.1).min(1.0))),
                    Key::Named(Named::ArrowDown) => return self.update(Msg::Volume((self.volume - 0.1).max(0.0))),
                    Key::Named(Named::Escape) => return (Task::none(), true),
                    Key::Character("m") => {
                        self.muted = !self.muted;
                        self.apply_volume();
                    }
                    _ => {}
                }
            }
            Msg::Key(_) => {}
            Msg::ToggleExpand => self.expanded = !self.expanded,
            Msg::Close => return (Task::none(), true),
        }
        (Task::none(), false)
    }

    /// `max_fps`: frame-rate cap while playing (0 = every display refresh). Redrawing at a
    /// 240 Hz monitor's rate costs 4x the CPU of 60 FPS for little visible benefit.
    pub fn subscription(&self, max_fps: u32) -> Subscription<Msg> {
        let keys = iced::keyboard::listen().map(Msg::Key);
        if !self.is_playing() {
            return keys;
        }
        let frames = if max_fps == 0 {
            iced::window::frames().map(|_| Msg::Frame)
        } else {
            iced::time::every(std::time::Duration::from_secs_f64(1.0 / max_fps as f64)).map(|_| Msg::Frame)
        };
        Subscription::batch([keys, frames])
    }

    pub fn expanded(&self) -> bool {
        self.expanded
    }

    /// Compact player that sits in the sidebar in place of the album art.
    pub fn view(&self, stage_height: f32) -> Element<'_, Msg> {
        let stage: Element<'_, Msg> = match (&self.status, &self.track, &self.textures, self.player()) {
            (Status::Failed(e), ..) => center(text(e.clone()).size(13).style(text::danger).center()).padding(12).into(),
            (Status::Ready, Some(track), Some(tex), Some(player)) => {
                let time_ms = self.scrub.unwrap_or_else(|| player.position_ms());
                let anim_ms = self.anim_origin.elapsed().as_millis() as u64;
                canvas(Highway { track, textures: tex, time_ms, anim_ms }).width(Length::Fill).height(Length::Fill).into()
            }
            (Status::Loading(p), ..) => center(
                column![
                    text(if *p < 1.0 { "Downloading chart…" } else { "Preparing…" }).size(13),
                    progress_bar(0.0..=1.0, *p).girth(4).length(180),
                ]
                .spacing(10)
                .align_x(Alignment::Center),
            )
            .into(),
            _ => center(text("Loading graphics…").size(13)).into(),
        };
        let stage = container(stage)
            .width(Length::Fill)
            .height(stage_height)
            .style(|_: &Theme| container::Style { background: Some(Color::BLACK.into()), ..Default::default() });

        let small = |label: &'static str, msg: Msg| {
            button(text(label).size(12)).padding([4, 8]).style(crate::style::secondary).on_press(msg)
        };
        let window_buttons = row![
            small(if self.expanded { "Shrink" } else { "Expand" }, Msg::ToggleExpand),
            small("Close", Msg::Close),
        ]
        .spacing(6);

        let controls: Element<'_, Msg> = match self.player() {
            Some(player) => {
                let end = player.end_ms();
                let pos = self.scrub.unwrap_or_else(|| player.position_ms());
                let avail = self.available();
                let instruments: Vec<Choice> = chart::INSTRUMENTS
                    .iter()
                    .filter(|i| avail.iter().any(|(ai, _)| ai == *i))
                    .map(|i| Choice { key: i, label: instrument_label(i) })
                    .collect();
                let difficulties: Vec<Choice> = chart::DIFFICULTIES
                    .iter()
                    .filter(|d| avail.contains(&(self.instrument, **d)))
                    .map(|d| Choice { key: d, label: difficulty_label(d) })
                    .collect();
                let cur_i = Choice { key: self.instrument, label: instrument_label(self.instrument) };
                let cur_d = Choice { key: self.difficulty, label: difficulty_label(self.difficulty) };
                column![
                    row![
                        button(text(if player.is_playing() { "Pause" } else { "Play" }).size(13).width(44).center())
                            .padding([5, 10])
                            .style(button::primary)
                            .on_press(Msg::TogglePlay),
                        text(format!("{} / {}", fmt_time(pos), fmt_time(end))).size(12).width(Length::Fill),
                        window_buttons,
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                    slider(0.0..=end, pos, Msg::Scrub).on_release(Msg::ScrubEnd).step(10.0),
                    row![
                        pick_list(instruments, Some(cur_i), Msg::Instrument)
                            .text_size(12)
                            .padding([4, 8])
                            .width(Length::FillPortion(3)),
                        pick_list(difficulties, Some(cur_d), Msg::Difficulty)
                            .text_size(12)
                            .padding([4, 8])
                            .width(Length::FillPortion(2)),
                    ]
                    .spacing(6),
                    row![
                        text(if self.muted { "Muted" } else { "Volume" }).size(12).width(48),
                        slider(0.0..=1.0, self.volume, Msg::Volume).step(0.01f32),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                ]
                .spacing(6)
                .into()
            }
            None => row![iced::widget::space().width(Length::Fill), window_buttons].into(),
        };

        column![stage, controls].spacing(8).into()
    }
}

fn fmt_time(ms: f64) -> String {
    let s = (ms.max(0.0) / 1000.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}
