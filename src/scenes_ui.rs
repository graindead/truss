//! The Scene Switcher tool (Tools > Scene Switcher): CH Scene Switcher inside Truss.

use crate::scenes::{self, Cmd, Config, Conn, DeviceCode, Event, NextResult, Tokens};
use iced::widget::{button, column, container, pick_list, row, rule, scrollable, space, text, text_input};
use iced::{Alignment, Element, Length, Subscription, Task, Theme};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sub {
    Requests,
    Single,
    Multi,
    Settings,
}

enum Twitch {
    Out,
    SigningIn(DeviceCode),
    In,
}

pub struct ScenesTool {
    cfg: Config,
    tx: Option<UnboundedSender<Cmd>>,
    pub sub: Sub,
    conn: Conn,
    conn_text: String,
    status: String,
    scenes: Vec<String>,
    queue: Option<Result<Vec<String>, String>>,
    next_busy: bool,
    next_msg: String,
    twitch: Twitch,
    tw_msg: String,
    tw_busy: bool,
    save_error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Worker(Event),
    Sub(Sub),
    Host(String),
    Port(String),
    Password(String),
    Connect,
    Disconnect,
    SongScene(String),
    MenuScene(String),
    UseMode,
    Delay(String),
    File(String),
    Browse,
    FileChosen(Option<String>),
    OuvertKey(String),
    OuvertPrivate(String),
    ChatCmd(String),
    RefreshQueue,
    Queue(Result<Vec<String>, String>),
    Next,
    NextDone(NextResult),
    QueueSoon,
    TwButton,
    TwStarted(Result<DeviceCode, String>),
    TwTick,
    TwPolled(Result<Option<Tokens>, String>),
    TwSignedOut,
}

const NONE: &str = "(none)";

impl Default for ScenesTool {
    fn default() -> Self {
        let cfg = scenes::load();
        let signed_in = !cfg.twitch_access_token.is_empty();
        let has_scene = !cfg.song_scene.is_empty() || !cfg.mp_song_scene.is_empty();
        Self {
            sub: if has_scene { if cfg.mode == 0 { Sub::Single } else { Sub::Multi } } else { Sub::Settings },
            twitch: if signed_in { Twitch::In } else { Twitch::Out },
            tw_msg: if signed_in { format!("Twitch: signed in as {}", cfg.twitch_login) } else { "Twitch: not signed in".into() },
            cfg,
            tx: None,
            conn: Conn::Off,
            conn_text: "Enter your OBS WebSocket details and click Connect.".into(),
            status: String::new(),
            scenes: Vec::new(),
            queue: None,
            next_busy: false,
            next_msg: String::new(),
            tw_busy: false,
            save_error: None,
        }
    }
}

impl ScenesTool {
    fn send(&self, c: Cmd) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(c);
        }
    }

    fn save(&mut self) {
        self.save_error = scenes::save(&self.cfg).err().map(|e| format!("Couldn't save settings: {e}"));
    }

    /// Saves and hands the new settings to the OBS worker.
    fn changed(&mut self) {
        self.save();
        self.send(Cmd::Update(self.cfg.clone()));
    }

    /// The mode shown on a Singleplayer / Multiplayer page.
    fn view_mode(&self) -> u8 {
        (self.sub == Sub::Multi) as u8
    }

    fn scenes_mut(&mut self, mode: u8) -> (&mut String, &mut String) {
        if mode == 0 { (&mut self.cfg.song_scene, &mut self.cfg.menu_scene) } else { (&mut self.cfg.mp_song_scene, &mut self.cfg.mp_menu_scene) }
    }

    fn refresh_queue(&self) -> Task<Msg> {
        Task::perform(scenes::ouvert_queue(self.cfg.ouvert_key.clone()), Msg::Queue)
    }

    fn set_tokens(&mut self, t: &Tokens) {
        self.cfg.twitch_access_token = t.access.clone();
        self.cfg.twitch_refresh_token = t.refresh.clone();
        self.cfg.twitch_user_id = t.id.clone();
        self.cfg.twitch_login = t.login.clone();
        self.save();
        if t.access.is_empty() {
            self.twitch = Twitch::Out;
        } else {
            self.twitch = Twitch::In;
            self.tw_msg = format!("Twitch: signed in as {}", t.login);
        }
    }

    pub fn update(&mut self, m: Msg) -> Task<Msg> {
        match m {
            Msg::Worker(e) => match e {
                Event::Ready(tx) => {
                    self.tx = Some(tx);
                    // Connected last time: connect again straight away.
                    if self.cfg.truss_connect {
                        self.send(Cmd::Connect(self.cfg.clone()));
                    }
                }
                Event::Conn(c, t) => {
                    self.conn = c;
                    self.conn_text = t;
                    if c != Conn::Connected {
                        self.status.clear();
                    }
                }
                Event::Status(s) => self.status = s,
                Event::Scenes(s) => self.scenes = s,
            },
            Msg::Sub(s) => {
                self.sub = s;
                if s == Sub::Requests {
                    return self.refresh_queue();
                }
            }
            Msg::Host(s) => self.cfg.host = s,
            Msg::Port(s) => self.cfg.port = s.chars().filter(char::is_ascii_digit).take(5).collect(),
            Msg::Password(s) => self.cfg.password = s,
            Msg::Connect => {
                if !self.cfg.port_ok() {
                    self.conn_text = "Port must be a number from 1 to 65535.".into();
                    return Task::none();
                }
                self.cfg.truss_connect = true;
                self.save();
                self.send(Cmd::Connect(self.cfg.clone()));
            }
            Msg::Disconnect => {
                self.cfg.truss_connect = false;
                self.save();
                self.send(Cmd::Disconnect);
            }
            Msg::SongScene(s) => {
                *self.scenes_mut(self.view_mode()).0 = s;
                self.changed();
            }
            Msg::MenuScene(s) => {
                *self.scenes_mut(self.view_mode()).1 = if s == NONE { String::new() } else { s };
                self.changed();
            }
            Msg::UseMode => {
                self.cfg.mode = self.view_mode();
                self.save();
                self.send(Cmd::ModeChanged(self.cfg.clone()));
            }
            Msg::Delay(s) => {
                self.cfg.menu_delay = s;
                if self.cfg.delay_ms().is_some() {
                    self.changed();
                }
            }
            Msg::File(s) => {
                self.cfg.song_file = s.trim().trim_matches('"').to_string();
                self.changed();
            }
            Msg::Browse => {
                let start = std::path::Path::new(&self.cfg.song_file).parent().map(|p| p.to_path_buf());
                return Task::perform(
                    async move {
                        let mut d = rfd::AsyncFileDialog::new().add_filter("Text files", &["txt"]).add_filter("All files", &["*"]);
                        if let Some(p) = start {
                            d = d.set_directory(p);
                        }
                        d.pick_file().await.map(|f| f.path().to_string_lossy().into_owned())
                    },
                    Msg::FileChosen,
                );
            }
            Msg::FileChosen(Some(p)) => return self.update(Msg::File(p)),
            Msg::FileChosen(None) => {}
            Msg::OuvertKey(s) => {
                self.cfg.ouvert_key = s.trim().to_string();
                self.save();
            }
            Msg::OuvertPrivate(s) => {
                self.cfg.ouvert_private_key = s.trim().to_string();
                self.save();
            }
            Msg::ChatCmd(s) => {
                self.cfg.next_chat_command = s;
                self.save();
            }
            Msg::RefreshQueue => return self.refresh_queue(),
            Msg::Queue(r) => self.queue = Some(r),
            Msg::Next => {
                // One at a time until the answer comes, so a double click can't skip two songs.
                if !self.next_busy {
                    self.next_busy = true;
                    self.next_msg.clear();
                    return Task::perform(scenes::next_song(self.cfg.clone()), Msg::NextDone);
                }
            }
            Msg::NextDone(r) => {
                self.next_busy = false;
                self.next_msg = r.message;
                if let Some(t) = &r.tokens {
                    if t.access.is_empty() {
                        self.tw_msg = "Twitch sign-in expired. Sign in again in Settings.".into();
                    }
                    self.set_tokens(t);
                }
                // Give the bot a moment, then show the new queue.
                let wait = if r.via_chat { 1500 } else { 300 };
                return Task::perform(tokio::time::sleep(Duration::from_millis(wait)), |_| Msg::QueueSoon);
            }
            Msg::QueueSoon => return self.refresh_queue(),
            Msg::TwButton => {
                // signed out -> sign in, signing in -> cancel, signed in -> sign out
                match &self.twitch {
                    Twitch::Out => {
                        self.tw_busy = true;
                        return Task::perform(scenes::twitch_start(), Msg::TwStarted);
                    }
                    Twitch::SigningIn(_) => {
                        self.twitch = Twitch::Out;
                        self.tw_msg = "Twitch: not signed in".into();
                    }
                    Twitch::In => {
                        self.tw_busy = true;
                        let access = std::mem::take(&mut self.cfg.twitch_access_token);
                        return Task::perform(scenes::twitch_revoke(access), |_| Msg::TwSignedOut);
                    }
                }
            }
            Msg::TwStarted(r) => {
                self.tw_busy = false;
                match r {
                    Ok(code) => {
                        self.tw_msg = format!("Twitch code: {}", code.user_code);
                        // only ever open Twitch's own site
                        if code.verification_uri.starts_with("https://www.twitch.tv/") {
                            let _ = open::that(&code.verification_uri);
                        }
                        self.twitch = Twitch::SigningIn(code);
                    }
                    Err(_) => self.tw_msg = "Twitch sign-in failed. Try again.".into(),
                }
            }
            Msg::TwTick => {
                if let Twitch::SigningIn(code) = &self.twitch {
                    if Instant::now() >= code.expires {
                        self.twitch = Twitch::Out;
                        self.tw_msg = "Twitch sign-in failed. Try again.".into();
                    } else if !self.tw_busy {
                        self.tw_busy = true;
                        return Task::perform(scenes::twitch_poll(code.device_code.clone()), Msg::TwPolled);
                    }
                }
            }
            Msg::TwPolled(r) => {
                self.tw_busy = false;
                if !matches!(self.twitch, Twitch::SigningIn(_)) {
                    return Task::none(); // cancelled meanwhile
                }
                match r {
                    Ok(Some(t)) => self.set_tokens(&t),
                    Ok(None) => {}
                    Err(e) => {
                        self.twitch = Twitch::Out;
                        self.tw_msg = e;
                    }
                }
            }
            Msg::TwSignedOut => {
                self.tw_busy = false;
                self.set_tokens(&Tokens::default());
                self.tw_msg = "Twitch: not signed in".into();
            }
        }
        Task::none()
    }

    /// The OBS worker (always running, so scenes switch on every page) and the Twitch sign-in
    /// check.
    pub fn background(&self) -> Subscription<Msg> {
        let worker = Subscription::run(scenes::worker).map(Msg::Worker);
        match &self.twitch {
            Twitch::SigningIn(c) => Subscription::batch([worker, iced::time::every(c.interval).map(|_| Msg::TwTick)]),
            _ => worker,
        }
    }

    /// While the request queue is on screen: refresh it every 5 seconds.
    pub fn subscription(&self) -> Subscription<Msg> {
        if self.sub == Sub::Requests {
            iced::time::every(Duration::from_secs(5)).map(|_| Msg::RefreshQueue)
        } else {
            Subscription::none()
        }
    }

    pub fn view(&self) -> Element<'_, Msg> {
        let nav = |label: &'static str, s: Sub| {
            let active = self.sub == s;
            button(text(label).size(13))
                .width(Length::Fill)
                .padding([7, 12])
                .style(move |t: &Theme, st| if active { button::primary(t, st) } else { button::subtle(t, st) })
                .on_press(Msg::Sub(s))
        };
        let left = container(
            column![nav("Requests", Sub::Requests), nav("Singleplayer", Sub::Single), nav("Multiplayer", Sub::Multi), nav("Settings", Sub::Settings)]
                .spacing(4)
                .padding(10),
        )
        .width(160)
        .height(Length::Fill)
        .style(|t: &Theme| container::Style { background: Some(t.extended_palette().background.weakest.color.into()), ..Default::default() });

        let body: Element<'_, Msg> = match self.sub {
            Sub::Requests => self.requests_view(),
            Sub::Single | Sub::Multi => self.mode_view(),
            Sub::Settings => self.settings_view(),
        };
        let right = column![container(body).height(Length::Fill).padding(20), rule::horizontal(1), self.status_row()];
        row![left, rule::vertical(1), right].height(Length::Fill).into()
    }

    fn status_row(&self) -> Element<'_, Msg> {
        let (dot_color, label): (fn(&Theme) -> iced::Color, &str) = match self.conn {
            Conn::Connected => (|t| t.extended_palette().success.base.color, if self.status.is_empty() { "Connected" } else { &self.status }),
            Conn::Connecting => (|t| t.extended_palette().warning.base.color, "Connecting…"),
            Conn::Failed => (|t| t.extended_palette().danger.base.color, "Can't connect"),
            Conn::Off => (|t| t.extended_palette().background.strong.color, "Not connected"),
        };
        let mode = if self.cfg.mode == 0 { "Singleplayer" } else { "Multiplayer" };
        // A drawn circle rather than a "●" character, so it sits centred on the text.
        let dot = container(space()).width(8).height(8).style(move |t: &Theme| container::Style {
            background: Some(dot_color(t).into()),
            border: iced::Border::default().rounded(4),
            ..Default::default()
        });
        container(
            row![
                dot,
                text(label.to_string()).size(12).width(Length::Fill).wrapping(text::Wrapping::None),
                text(format!("Active mode: {mode}")).size(12),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([8, 20])
        .into()
    }

    fn requests_view(&self) -> Element<'_, Msg> {
        let list: Element<'_, Msg> = match &self.queue {
            None => text("Loading the request queue…").size(13).into(),
            Some(Err(e)) => text(e.clone()).size(13).style(text::danger).into(),
            Some(Ok(q)) if q.is_empty() => text("The queue is empty.").size(13).into(),
            Some(Ok(q)) => scrollable(
                column(q.iter().enumerate().map(|(i, s)| text(format!("{}. {s}", i + 1)).size(13).into())).spacing(6).width(Length::Fill),
            )
            .height(Length::Fill)
            .into(),
        };
        column![
            text("Song requests").size(15).font(crate::BOLD),
            container(list).height(Length::Fill),
            row![
                button(text(if self.next_busy { "Working…" } else { "Next song" }).size(13))
                    .padding([6, 14])
                    .on_press_maybe((!self.next_busy).then_some(Msg::Next)),
                text(self.next_msg.clone()).size(12),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        ]
        .spacing(12)
        .into()
    }

    fn mode_view(&self) -> Element<'_, Msg> {
        let mode = self.view_mode();
        let (song, menu) = self.cfg.scenes(mode);
        let mut menu_opts = vec![NONE.to_string()];
        menu_opts.extend(self.scenes.iter().cloned());
        let song_sel = (!song.is_empty()).then(|| song.to_string());
        let menu_sel = Some(if menu.is_empty() { NONE.to_string() } else { menu.to_string() });
        let hint = if self.scenes.is_empty() {
            "Connect to OBS (Settings) to choose from its scenes."
        } else {
            "Truss switches to the first scene when a song starts and to the second when it ends."
        };
        let active = self.cfg.mode == mode;
        column![
            text(if mode == 0 { "Singleplayer" } else { "Multiplayer" }).size(15).font(crate::BOLD),
            text("Song playing").size(12),
            pick_list(self.scenes.clone(), song_sel, Msg::SongScene).placeholder("Pick a scene").width(320),
            text("In menus").size(12),
            pick_list(menu_opts, menu_sel, Msg::MenuScene).width(320),
            text(hint).size(12),
            space().height(8),
            if active {
                Element::from(text("This mode is active.").size(13).style(text::success))
            } else {
                button(text("Use this mode").size(13)).padding([6, 14]).on_press(Msg::UseMode).into()
            },
        ]
        .spacing(8)
        .into()
    }

    fn settings_view(&self) -> Element<'_, Msg> {
        let label = |s: &'static str| text(s).size(12);
        let connected = matches!(self.conn, Conn::Connected | Conn::Connecting);
        let connect = if connected {
            button(text("Disconnect").size(13)).padding([6, 14]).style(button::secondary).on_press(Msg::Disconnect)
        } else {
            button(text("Connect").size(13)).padding([6, 14]).on_press(Msg::Connect)
        };
        let delay_bad = self.cfg.delay_ms().is_none();
        let tw_label = match self.twitch {
            Twitch::Out => "Sign in to Twitch",
            Twitch::SigningIn(_) => "Cancel",
            Twitch::In => "Sign out",
        };
        let mut col = column![
            text("OBS WebSocket").size(15).font(crate::BOLD),
            row![
                column![label("Server IP"), text_input("localhost", &self.cfg.host).on_input(Msg::Host).on_submit(Msg::Connect).padding(6)]
                    .spacing(4)
                    .width(Length::Fill),
                column![label("Port"), text_input("4455", &self.cfg.port).on_input(Msg::Port).on_submit(Msg::Connect).padding(6)]
                    .spacing(4)
                    .width(90),
            ]
            .spacing(10),
            label("Password"),
            row![
                text_input("", &self.cfg.password).secure(true).on_input(Msg::Password).on_submit(Msg::Connect).padding(6).width(Length::Fill),
                connect,
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            text(self.conn_text.clone()).size(12),
            space().height(6),
            text("Clone Hero").size(15).font(crate::BOLD),
            label("Current song file"),
            row![
                text_input("currentsong.txt", &self.cfg.song_file).on_input(Msg::File).padding(6).width(Length::Fill),
                button(text("Browse…").size(13)).padding([6, 12]).style(button::secondary).on_press(Msg::Browse),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            row![
                label("Delay before the menu scene (seconds)").width(Length::Fill),
                text_input("0", &self.cfg.menu_delay).on_input(Msg::Delay).padding(6).width(90),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        ]
        .spacing(8)
        .max_width(560);
        if delay_bad {
            col = col.push(text("Enter a number of seconds, such as 1.5").size(12).style(text::danger));
        }
        col = col.push(space().height(6)).push(text("Song requests").size(15).font(crate::BOLD)).extend([
            label("Ouvert public API key").into(),
            text_input("", &self.cfg.ouvert_key).secure(true).on_input(Msg::OuvertKey).padding(6).into(),
            label("Ouvert private API key (for Next song without Twitch)").into(),
            text_input("", &self.cfg.ouvert_private_key).secure(true).on_input(Msg::OuvertPrivate).padding(6).into(),
            row![
                text(self.tw_msg.clone()).size(12).width(Length::Fill),
                button(text(tw_label).size(13)).padding([6, 12]).style(button::secondary).on_press_maybe((!self.tw_busy).then_some(Msg::TwButton)),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into(),
            row![
                label("Chat command for Next song").width(Length::Fill),
                text_input("!playnext", &self.cfg.next_chat_command).on_input(Msg::ChatCmd).padding(6).width(140),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into(),
            text(
                "Settings are shared with the standalone CH Scene Switcher app. Keys and Twitch sign-in are \
                 encrypted so only your Windows account can read them. Don't run both apps at once.",
            )
            .size(11)
            .into(),
        ]);
        if let Some(e) = &self.save_error {
            col = col.push(text(e.clone()).size(12).style(text::danger));
        }
        scrollable(col).height(Length::Fill).into()
    }
}
