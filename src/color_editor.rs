//! Tools > Note colors: edits Clone Hero colour profiles (see color_profile.rs). Guitar and
//! drums are previewed on the chart preview's highway, playing a made-up chart with notes and
//! frets built from Clone Hero's sprites in the profile's colours (see ch_sprites.rs); 6-fret
//! and the other colours have a preview where clicking a part picks its colour, made of the
//! game's own pictures when they're read from the install (otherwise drawn shapes).

use crate::color_profile::{self as cp, Profile};
use crate::theme_maker::{Picker, from_hsv, parse_hex, swatch, to_hsv};
use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::widget::{button, canvas as canvas_widget, column, container, row, scrollable, space, text, text_input};
use iced::{Alignment, Border, Color, Element, Length, Point, Rectangle, Renderer, Size, Subscription, Task, Theme, mouse};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

const SECTION_LABELS: [&str; 4] = ["Guitar", "Drums", "6-fret", "Other"];

struct Open {
    /// File stem, or the name it'll be saved as.
    name: String,
    path: Option<PathBuf>,
    profile: Profile,
    /// As last saved, for "unsaved changes".
    saved: Profile,
}

#[derive(Default)]
pub struct ColorTool {
    dir: Option<PathBuf>,
    files: Vec<String>,
    defaults: Option<Profile>,
    open: Option<Open>,
    section: usize,
    selected: Option<String>,
    hsv: (f32, f32, f32),
    hex: String,
    save_as: String,
    status: Option<(bool, String)>,
    /// Asking whether to delete the open profile.
    confirm_delete: bool,
    /// Clone Hero's sprites for the highway preview (downloaded once), and what's built from
    /// them for the section shown: (section, star power on, profile text) it was built for.
    kit: Option<Result<Arc<crate::ch_sprites::Kit>, String>>,
    kit_requested: bool,
    highway: Option<Arc<crate::textures::TextureSet>>,
    built: Option<(usize, bool, String)>,
    demo: Option<Arc<(crate::preview::RenderTrack, crate::preview::RenderTrack)>>,
    started: Option<Instant>,
    /// Show the notes as they look while star power is on.
    sp_active: bool,
    /// The game's pictures for the previews, when read from the install.
    art: Option<Art>,
    /// Guitar and drums: show the chart preview's highway instead of every piece laid out.
    highway_view: bool,
    /// The player's own note speed and highway length, from their Clone Hero profile (Settings
    /// can set others).
    game_speed: Option<crate::preview::GameSpeed>,
}

/// The game's pictures for the previews, tinted as they're needed (and kept).
struct Art {
    kit: Arc<crate::ch_sprites::Kit>,
    raw: std::cell::RefCell<std::collections::HashMap<String, Option<Arc<image::RgbaImage>>>>,
    tinted: std::cell::RefCell<std::collections::HashMap<(String, [u8; 3]), iced::widget::image::Handle>>,
}

/// A picture by name: one of `ch_sprites::ART`, or a note's or fret's layer named as
/// `layer_name` makes them.
fn source(kit: &crate::ch_sprites::Kit, name: &str) -> Option<image::RgbaImage> {
    if let Some(rest) = name.strip_prefix("layer:") {
        let mut it = rest.rsplitn(4, ':');
        let dy = it.next()?.parse().ok()?;
        let h = it.next()?.parse().ok()?;
        let w = it.next()?.parse().ok()?;
        return crate::ch_sprites::layer_image(kit, it.next()?, w, h, dy);
    }
    kit.art.get(name).cloned()
}

/// The name of one layer of a note or fret (see `ch_sprites::layer_image`).
fn layer_name(sprite: &str, (w, h): (u32, u32), dy: i64) -> String {
    format!("layer:{sprite}:{w}:{h}:{dy}")
}

impl Art {
    fn raw(&self, name: &str) -> Option<Arc<image::RgbaImage>> {
        if let Some(r) = self.raw.borrow().get(name) {
            return r.clone();
        }
        let r = source(&self.kit, name).map(Arc::new);
        self.raw.borrow_mut().insert(name.to_string(), r.clone());
        r
    }

    /// Whether a point in a picture drawn at `rect` lands on something (not a clear part).
    fn solid(&self, name: &str, rect: Rectangle, p: Point) -> bool {
        let Some(img) = self.raw(name) else { return rect.contains(p) };
        let x = ((p.x - rect.x) / rect.width * img.width() as f32) as i64;
        let y = ((p.y - rect.y) / rect.height * img.height() as f32) as i64;
        x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() && img.get_pixel(x as u32, y as u32)[3] > 60
    }

    fn get(&self, name: &str, c: Color) -> Option<iced::widget::image::Handle> {
        let [r, g, b, _] = c.into_rgba8();
        let k = (name.to_string(), [r, g, b]);
        if let Some(h) = self.tinted.borrow().get(&k) {
            return Some(h.clone());
        }
        let mut img = (*self.raw(name)?).clone();
        for p in img.pixels_mut() {
            p[0] = (p[0] as u32 * r as u32 / 255) as u8;
            p[1] = (p[1] as u32 * g as u32 / 255) as u8;
            p[2] = (p[2] as u32 * b as u32 / 255) as u8;
        }
        let h = iced::widget::image::Handle::from_rgba(img.width(), img.height(), img.into_raw());
        let mut t = self.tinted.borrow_mut();
        // Dragging through colours makes a lot of these; start again now and then.
        if t.len() > 800 {
            t.clear();
        }
        t.insert(k, h.clone());
        Some(h)
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    OpenFile(String),
    New,
    Section(usize),
    Select(String),
    Hex(String),
    Hsv(f32, f32, f32),
    Default,
    SaveAsName(String),
    Save,
    SaveAs,
    Discard,
    Delete,
    DeleteConfirmed,
    DeleteCancelled,
    OpenFolder,
    Kit(Result<Arc<crate::ch_sprites::Kit>, String>),
    SpActive(bool),
    HighwayView(bool),
    Frame,
}

impl ColorTool {
    fn defaults(&mut self) -> &Profile {
        self.defaults.get_or_insert_with(Profile::defaults)
    }

    fn section(&self) -> &'static str {
        cp::SECTIONS[self.section]
    }

    /// Reads the profile list (when the tab is opened).
    pub fn refresh(&mut self) {
        self.dir = cp::folder();
        self.files = self.dir.as_deref().map(cp::list).unwrap_or_default();
    }

    /// The tab was opened: reads the profile list, and starts getting the sprites.
    pub fn opened(&mut self) -> Task<Msg> {
        self.refresh();
        if self.kit_requested {
            return Task::none();
        }
        self.kit_requested = true;
        Task::perform(crate::ch_sprites::load(), Msg::Kit)
    }

    /// Guitar and drums get the highway; the others the drawn preview.
    fn on_highway(&self) -> bool {
        // Without the game's pictures there's no laid-out view, so the highway it is.
        self.section < 2 && (self.highway_view || self.art.is_none())
    }

    /// While the highway is showing, it's redrawn about 60 times a second.
    /// At the preview frame rate set in Settings (0: whenever the display draws).
    pub fn subscription(&self, fps: u32) -> Subscription<Msg> {
        if self.on_highway() && self.highway.is_some() && self.open.is_some() {
            match fps {
                0 => iced::window::frames().map(|_| Msg::Frame),
                n => iced::time::every(std::time::Duration::from_secs_f64(1.0 / n as f64)).map(|_| Msg::Frame),
            }
        } else {
            Subscription::none()
        }
    }

    /// Builds the highway's notes and frets again if the colours (or what's shown) changed.
    fn rebuild(&mut self) {
        let (Some(Ok(kit)), Some(o), true) = (&self.kit, &self.open, self.on_highway()) else { return };
        let key = (self.section, self.sp_active, o.profile.to_text());
        if self.built.as_ref() == Some(&key) {
            return;
        }
        let d = self.defaults.get_or_insert_with(Profile::defaults);
        let set = if self.section == 0 {
            crate::ch_sprites::guitar(kit, &o.profile, d, self.sp_active)
        } else {
            crate::ch_sprites::drums(kit, &o.profile, d, self.sp_active)
        };
        self.highway = Some(Arc::new(set));
        self.built = Some(key);
        if self.demo.is_none() {
            self.demo = Some(Arc::new(crate::ch_sprites::demo_tracks()));
        }
        self.started.get_or_insert_with(Instant::now);
    }

    fn value(&self, key: &str) -> Option<Color> {
        let sec = self.section();
        let o = self.open.as_ref()?;
        o.profile.color(sec, key).or_else(|| self.defaults.as_ref().and_then(|d| d.color(sec, key)))
    }

    fn select(&mut self, key: String) {
        let c = self.value(&key).unwrap_or(Color::WHITE);
        self.hsv = to_hsv(c);
        self.hex = cp::ch_hex(c);
        self.selected = Some(key);
    }

    fn set_selected(&mut self, c: Color) {
        let sec = self.section();
        if let (Some(k), Some(o)) = (self.selected.clone(), self.open.as_mut()) {
            o.profile.set(sec, &k, &cp::ch_hex(c));
        }
    }

    pub fn update(&mut self, m: Msg) -> Task<Msg> {
        if matches!(m, Msg::Frame) {
            return Task::none();
        }
        let task = self.apply(m);
        self.rebuild();
        task
    }

    fn apply(&mut self, m: Msg) -> Task<Msg> {
        // Anything else done meanwhile drops a pending "Delete?" (it's only for what was asked).
        if !matches!(m, Msg::Delete | Msg::DeleteConfirmed | Msg::DeleteCancelled | Msg::Frame) {
            self.confirm_delete = false;
        }
        match m {
            Msg::Frame => {}
            Msg::Kit(r) => {
                if let Err(e) = &r {
                    log::warn!("note sprites: {e}");
                }
                if let Some(install) = r.as_ref().ok().and_then(|k| k.game.as_ref()) {
                    let (speed, length) = crate::ch_sprites::profile_settings(install);
                    self.game_speed = Some(crate::preview::GameSpeed {
                        note_speed: speed.unwrap_or(crate::preview::NOTE_SPEED_DEFAULT),
                        highway_length: length.unwrap_or(crate::preview::HIGHWAY_LENGTH_DEFAULT),
                    });
                }
                if let Ok(kit) = &r
                    && !kit.art.is_empty()
                {
                    self.art = Some(Art { kit: kit.clone(), raw: Default::default(), tinted: Default::default() });
                }
                self.kit = Some(r);
            }
            Msg::SpActive(on) => self.sp_active = on,
            Msg::HighwayView(on) => self.highway_view = on,
            Msg::OpenFile(name) => {
                let Some(path) = self.dir.as_ref().map(|d| d.join(format!("{name}.ini"))) else { return Task::none() };
                match std::fs::read(&path) {
                    Ok(b) => {
                        let p = Profile::parse(&String::from_utf8_lossy(&b));
                        self.defaults();
                        self.open = Some(Open { name: name.clone(), path: Some(path), profile: p.clone(), saved: p });
                        self.save_as = format!("{name} copy");
                        self.status = None;
                        self.selected = None;
                    }
                    Err(e) => self.status = Some((false, format!("Couldn't read {name}.ini: {e}"))),
                }
            }
            Msg::New => {
                let p = self.defaults().clone();
                self.open = Some(Open { name: "New profile".into(), path: None, profile: p.clone(), saved: p });
                self.save_as = unique(&self.files, "My colors");
                self.status = None;
                self.selected = None;
            }
            Msg::Section(i) => {
                self.section = i.min(cp::SECTIONS.len() - 1);
                self.selected = None;
            }
            Msg::Select(k) => {
                if self.selected.as_deref() == Some(k.as_str()) {
                    self.selected = None;
                } else {
                    self.select(k);
                }
            }
            Msg::Hex(v) => {
                if let Some(c) = parse_hex(&v) {
                    self.hsv = to_hsv(c);
                    self.set_selected(c);
                }
                self.hex = v;
            }
            Msg::Hsv(h, s, v) => {
                self.hsv = (h, s, v);
                let c = from_hsv(h, s, v);
                self.hex = cp::ch_hex(c);
                self.set_selected(c);
            }
            Msg::Default => {
                let sec = self.section();
                if let Some(k) = self.selected.clone()
                    && let Some(c) = self.defaults().color(sec, &k)
                {
                    self.set_selected(c);
                    self.select(k);
                }
            }
            Msg::SaveAsName(n) => self.save_as = n,
            Msg::Save => {
                let Some(o) = self.open.as_mut() else { return Task::none() };
                let Some(path) = o.path.clone() else { return self.apply(Msg::SaveAs) };
                self.status = Some(match save(&path, &o.profile) {
                    Ok(()) => {
                        o.saved = o.profile.clone();
                        (true, format!("Saved {}.ini. Pick it in Clone Hero under Settings > Gameplay > Color profile (restart Clone Hero if it was already in use).", o.name))
                    }
                    Err(e) => (false, e),
                });
            }
            Msg::SaveAs => {
                let name = self.save_as.trim().to_string();
                let Some(dir) = self.dir.clone() else {
                    self.status = Some((false, "Clone Hero's Colors folder wasn't found.".into()));
                    return Task::none();
                };
                if name.is_empty() || name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
                    self.status = Some((false, "Give the profile a file name without \\ / : * ? \" < > |.".into()));
                    return Task::none();
                }
                let path = dir.join(format!("{name}.ini"));
                let Some(o) = self.open.as_mut() else { return Task::none() };
                if path.exists() && o.path.as_ref() != Some(&path) {
                    self.status = Some((false, format!("There's already a profile called {name}. Pick another name.")));
                    return Task::none();
                }
                self.status = Some(match save(&path, &o.profile) {
                    Ok(()) => {
                        o.saved = o.profile.clone();
                        o.path = Some(path);
                        o.name = name.clone();
                        (true, format!("Saved {name}.ini. Pick it in Clone Hero under Settings > Gameplay > Color profile."))
                    }
                    Err(e) => (false, e),
                });
                self.refresh();
            }
            Msg::Delete => self.confirm_delete = true,
            Msg::DeleteCancelled => self.confirm_delete = false,
            Msg::DeleteConfirmed => {
                self.confirm_delete = false;
                if let Some(o) = self.open.take() {
                    match o.path.as_ref().map(trash::delete) {
                        Some(Ok(())) => self.status = Some((true, format!("Moved {} to the Recycle Bin.", o.name))),
                        Some(Err(e)) => {
                            self.status = Some((false, format!("Couldn't delete {}: {e}", o.name)));
                            self.open = Some(o);
                        }
                        None => {}
                    }
                }
                self.refresh();
            }
            Msg::Discard => {
                if let Some(o) = self.open.as_mut() {
                    o.profile = o.saved.clone();
                }
                if let Some(k) = self.selected.clone() {
                    self.select(k);
                }
            }
            Msg::OpenFolder => {
                if let Some(d) = &self.dir {
                    let _ = open::that_detached(d);
                }
            }
        }
        Task::none()
    }

    pub fn view(&self, settings: &crate::settings::Settings) -> Element<'_, Msg> {
        // Note speed and highway length: Settings', else the player's profile's.
        let speed = self.game_speed.map(|p| crate::preview::GameSpeed {
            note_speed: settings.preview_note_speed.unwrap_or(p.note_speed),
            highway_length: settings.preview_highway_length.unwrap_or(p.highway_length),
        });
        let muted = |t: &Theme| text::Style { color: Some(crate::style::muted(t)) };

        // Left: the profiles in Clone Hero's folder.
        let mut list = column![text("Color profiles").size(15).font(crate::BOLD)].spacing(6);
        match &self.dir {
            None => list = list.push(text("Clone Hero's Colors folder (Documents\\Clone Hero\\PlayerData\\Custom\\Colors) wasn't found.").size(12).style(muted)),
            Some(_) => {
                if self.files.is_empty() {
                    list = list.push(text("No profiles yet.").size(12).style(muted));
                }
                let current = self.open.as_ref().and_then(|o| o.path.as_ref()).and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned());
                for f in &self.files {
                    list = list.push(
                        button(text(f.clone()).size(13))
                            .width(Length::Fill)
                            .padding([5, 10])
                            .style(crate::style::quiet_button(current.as_deref() == Some(f.as_str())))
                            .on_press(Msg::OpenFile(f.clone())),
                    );
                }
            }
        }
        list = list.push(space().height(6)).push(
            row![
                button(text("New profile").size(13)).padding([6, 12]).style(button::primary).on_press(Msg::New),
                button(text("Open folder").size(13)).padding([6, 12]).style(crate::style::secondary).on_press_maybe(self.dir.as_ref().map(|_| Msg::OpenFolder)),
            ]
            .spacing(8),
        );
        list = list.push(text("New profiles start from Clone Hero's default colors.").size(12).style(muted));
        let left = container(scrollable(list.padding(iced::Padding { right: 10.0, ..Default::default() }))).width(230).height(Length::Fill);

        let right: Element<'_, Msg> = match &self.open {
            None => container(text("Pick a profile on the left to edit it, or make a new one.").size(13).style(muted)).center(Length::Fill).into(),
            Some(o) => self.editor(o, speed),
        };
        let mut body = column![row![left, crate::style::divider(true), right].spacing(16).height(Length::Fill)].spacing(10);
        if let Some((ok, s)) = &self.status {
            let ok = *ok;
            body = body.push(text(s.clone()).size(12).style(move |t: &Theme| text::Style {
                color: Some(if ok { t.palette().success } else { t.palette().danger }),
            }));
        }
        container(body).padding(16).width(Length::Fill).height(Length::Fill).into()
    }

    fn editor<'a>(&'a self, o: &'a Open, speed: Option<crate::preview::GameSpeed>) -> Element<'a, Msg> {
        let muted = |t: &Theme| text::Style { color: Some(crate::style::muted(t)) };
        let sec = self.section();
        let defaults = self.defaults.as_ref().expect("loaded when a profile is opened");
        let changed = o.profile != o.saved;

        let title = row![
            text(if changed { format!("{} •", o.name) } else { o.name.clone() }).size(16).font(crate::BOLD),
            space().width(Length::Fill),
            button(text("Save").size(13)).padding([6, 12]).style(button::primary).on_press_maybe((changed || o.path.is_none()).then_some(Msg::Save)),
            button(text("Undo changes").size(13)).padding([6, 12]).style(crate::style::secondary).on_press_maybe(changed.then_some(Msg::Discard)),
            button(text("Delete").size(13)).padding([6, 12]).style(crate::style::secondary).on_press_maybe((o.path.is_some() && !self.confirm_delete).then_some(Msg::Delete)),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        // Deleting asks first (the file goes to the Recycle Bin).
        let title: Element<'a, Msg> = if self.confirm_delete {
            column![
                title,
                row![
                    text(format!("Delete \"{}\"? It goes to the Recycle Bin.", o.name)).size(13).width(Length::Fill),
                    button(text("Delete").size(13)).padding([5, 12]).style(button::danger).on_press(Msg::DeleteConfirmed),
                    button(text("Cancel").size(13)).padding([5, 12]).style(crate::style::secondary).on_press(Msg::DeleteCancelled),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8)
            .into()
        } else {
            title.into()
        };
        let save_as = row![
            text_input("Profile name", &self.save_as).on_input(Msg::SaveAsName).on_submit(Msg::SaveAs).size(13).width(200),
            button(text("Save as new").size(13)).padding([6, 12]).style(crate::style::secondary).on_press(Msg::SaveAs),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let tabs = row(SECTION_LABELS.iter().enumerate().map(|(i, l)| crate::style::tab(l, self.section == i, 13, Msg::Section(i)))).spacing(6);

        let cell = |key: String| -> Element<'a, Msg> {
            let c = o.profile.color(sec, &key).or_else(|| defaults.color(sec, &key)).unwrap_or(Color::TRANSPARENT);
            let on = self.selected.as_deref() == Some(key.as_str());
            button(swatch(c, 26.0).style(move |t: &Theme| container::Style {
                background: Some(c.into()),
                border: Border { color: if on { t.palette().text } else { crate::style::hairline(t) }, width: if on { 2.5 } else { 1.0 }, radius: 6.0.into() },
                ..Default::default()
            }))
            .padding(0)
            .style(button::text)
            .on_press(Msg::Select(key))
            .into()
        };

        let mut parts = column![].spacing(6);
        let (lanes, rows) = cp::grid(sec);
        if !rows.is_empty() {
            let mut head = row![space().width(110)].spacing(8);
            for l in lanes {
                head = head.push(text(l.label).size(11).style(muted).width(40).align_x(iced::alignment::Horizontal::Center));
            }
            parts = parts.push(head);
            for r in rows {
                let mut line = row![text(r.label).size(12).width(110)].spacing(8).align_y(Alignment::Center);
                for l in lanes {
                    line = line.push(if r.lanes.contains(&l.id) {
                        container(cell(cp::grid_key(r, l.id))).width(40).align_x(iced::alignment::Horizontal::Center).into()
                    } else {
                        Element::from(space().width(40))
                    });
                }
                parts = parts.push(line);
            }
            parts = parts.push(space().height(8));
        }
        for k in cp::other_keys(sec, &o.profile, defaults) {
            parts = parts.push(row![cell(k.clone()), text(cp::label(&k)).size(12)].spacing(10).align_y(Alignment::Center));
        }

        // The colour being edited.
        let picker: Element<'a, Msg> = match &self.selected {
            None => text("Click a swatch, or a part of the preview, to change its color.").size(12).style(muted).into(),
            Some(k) => {
                let ok = parse_hex(&self.hex).is_some();
                let (h, s, v) = self.hsv;
                column![
                    row![
                        column![text(cp::label(k)).size(13), text(k.clone()).size(11).style(muted)].spacing(1).width(Length::Fill),
                        text_input("#RRGGBB", &self.hex).on_input(Msg::Hex).size(13).width(100).style(move |t: &Theme, st| {
                            let mut style = text_input::default(t, st);
                            if !ok {
                                style.border.color = t.palette().danger;
                            }
                            style
                        }),
                        button(text("Default").size(12)).padding([5, 10]).style(crate::style::secondary).on_press(Msg::Default),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                    canvas_widget(Picker { h, s, v, on: Msg::Hsv }).width(Length::Fill).height(150),
                ]
                .spacing(8)
                .into()
            }
        };

        let editor = column![title, save_as, tabs, picker, scrollable(parts.padding(iced::Padding { right: 12.0, ..Default::default() })).height(Length::Fill)]
            .spacing(12)
            .width(Length::FillPortion(5));
        let drawn = || -> Element<'a, Msg> {
            canvas_widget(Preview { p: &o.profile, d: defaults, section: sec, selected: self.selected.as_deref(), art: self.art.as_ref(), art_layout: self.art.is_some() })
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };
        // Guitar and drums: every piece laid out, or the highway.
        let toggle = || -> Element<'a, Msg> {
            if self.section >= 2 || self.art.is_none() {
                return space().into();
            }
            let b = |label: &'a str, on: bool| {
                button(text(label).size(12)).padding([3, 10]).style(crate::style::quiet_button(on == self.on_highway())).on_press(Msg::HighwayView(on))
            };
            row![b("Pieces", false), b("Highway", true)].spacing(4).into()
        };
        let preview: Element<'a, Msg> = if self.on_highway() {
            match (&self.kit, &self.highway, &self.demo) {
                (Some(Ok(_)), Some(tex), Some(demo)) => {
                    let track = if self.section == 0 { &demo.0 } else { &demo.1 };
                    let elapsed = self.started.map_or(0, |s| s.elapsed().as_millis() as u64);
                    let time_ms = 2000.0 + (elapsed as f64) % (crate::ch_sprites::DEMO_MS - 20_000.0);
                    let hw = canvas_widget(crate::preview::Highway { track, textures: tex, time_ms, anim_ms: elapsed, game_speed: speed }).width(Length::Fill).height(Length::Fill);
                    let stage = iced::widget::stack![
                        container(hw).width(Length::Fill).height(Length::Fill).style(stage_style(Color::BLACK)),
                        container(space()).width(Length::Fill).height(Length::Fill).style(stage_style(Color::TRANSPARENT)),
                    ];
                    column![
                        row![
                            space().width(Length::Fill),
                            iced::widget::checkbox(self.sp_active).label("Star power active").size(14).text_size(12).on_toggle(Msg::SpActive),
                            toggle(),
                        ]
                        .spacing(12)
                        .align_y(Alignment::Center),
                        stage,
                    ]
                    .spacing(6)
                    .into()
                }
                (Some(Err(e)), ..) => column![
                    text(format!("Couldn't get Clone Hero's note sprites, so this is a simpler preview: {e}")).size(12).style(muted),
                    drawn(),
                ]
                .spacing(6)
                .into(),
                _ => column![text("Getting Clone Hero's note sprites…").size(12).style(muted), drawn()].spacing(6).into(),
            }
        } else {
            column![
                row![text("Preview · click a part to edit it").size(12).style(muted).width(Length::Fill), toggle()].align_y(Alignment::Center),
                drawn()
            ]
            .spacing(6)
            .into()
        };
        let preview = container(preview).width(Length::FillPortion(4));
        row![editor, preview].spacing(20).height(Length::Fill).into()
    }
}

/// The highway's stage: black behind it, and a thin border drawn over it.
fn stage_style(background: Color) -> impl Fn(&Theme) -> container::Style {
    move |_: &Theme| container::Style {
        background: Some(background.into()),
        border: Border { radius: crate::preview::STAGE_RADIUS.into(), width: 1.0, color: Color::from_rgba(1.0, 1.0, 1.0, 0.08) },
        ..Default::default()
    }
}

fn save(path: &std::path::Path, p: &Profile) -> Result<(), String> {
    // The first time Truss writes over a profile, the original is kept in its cache.
    if path.exists()
        && let Some(dir) = crate::settings::cache_dir().map(|d| d.join("color-profile-backups"))
        && let Some(name) = path.file_name()
        && !dir.join(name).exists()
    {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::copy(path, dir.join(name));
    }
    std::fs::write(path, p.to_text()).map_err(|e| format!("Couldn't save {}: {e}", path.display()))
}

fn unique(files: &[String], base: &str) -> String {
    let free = |n: &str| !files.iter().any(|f| f.eq_ignore_ascii_case(n));
    if free(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| free(n)).unwrap()
}

// ---------- Preview ----------

/// The highway drawn with a profile's colours. Each shape knows the key it's coloured by, so
/// a click picks that colour.
struct Preview<'a> {
    p: &'a Profile,
    d: &'a Profile,
    section: &'a str,
    selected: Option<&'a str>,
    /// The game's pictures, and whether to lay the preview out with them.
    art: Option<&'a Art>,
    art_layout: bool,
}

enum Shape {
    Circle(Point, f32),
    Rect(Rectangle, f32),
    Poly(Vec<Point>),
    Label(Point, String),
    /// One of the game's pictures (see `source`), tinted by the item's fill.
    Img(Rectangle, String),
    /// Not drawn: somewhere a click picks the item's colour (on top of what's under it).
    Hit(Rectangle),
}

struct Item {
    shape: Shape,
    fill: Option<Color>,
    stroke: Option<(Color, f32)>,
    key: Option<String>,
}

impl Shape {
    fn bounds(&self) -> Rectangle {
        match self {
            Shape::Circle(c, r) => Rectangle::new(Point::new(c.x - r, c.y - r), Size::new(r * 2.0, r * 2.0)),
            Shape::Rect(r, _) | Shape::Img(r, _) | Shape::Hit(r) => *r,
            Shape::Poly(pts) => {
                let (mut a, mut b) = (Point::new(f32::MAX, f32::MAX), Point::new(f32::MIN, f32::MIN));
                for p in pts {
                    a = Point::new(a.x.min(p.x), a.y.min(p.y));
                    b = Point::new(b.x.max(p.x), b.y.max(p.y));
                }
                Rectangle::new(a, Size::new(b.x - a.x, b.y - a.y))
            }
            Shape::Label(..) => Rectangle::new(Point::ORIGIN, Size::ZERO),
        }
    }

    fn path(&self) -> Option<Path> {
        Some(match self {
            Shape::Circle(c, r) => Path::circle(*c, *r),
            Shape::Rect(r, rad) => Path::rounded_rectangle(r.position(), r.size(), (*rad).into()),
            Shape::Poly(pts) => Path::new(|b| {
                if let Some(first) = pts.first() {
                    b.move_to(*first);
                    for p in &pts[1..] {
                        b.line_to(*p);
                    }
                    b.close();
                }
            }),
            Shape::Label(..) | Shape::Img(..) | Shape::Hit(..) => return None,
        })
    }

    /// The outline drawn round the shape when its colour is being edited.
    fn outline(&self) -> Option<Path> {
        match self {
            Shape::Img(r, _) => Some(Path::rounded_rectangle(r.position(), r.size(), 6.0.into())),
            Shape::Hit(..) => None,
            _ => self.path(),
        }
    }
}

/// Builds the scene: shapes coloured by keys of the section being shown.
struct Scene<'a> {
    pv: &'a Preview<'a>,
    items: Vec<Item>,
}

impl Scene<'_> {
    fn color(&self, key: &str) -> Color {
        let s = self.pv.section;
        self.pv.p.color(s, key).or_else(|| self.pv.d.color(s, key)).unwrap_or(Color::from_rgb(0.5, 0.5, 0.5))
    }
    fn fill(&mut self, shape: Shape, key: &str) {
        let c = self.color(key);
        self.items.push(Item { shape, fill: Some(c), stroke: None, key: Some(key.to_string()) });
    }
    fn ring(&mut self, shape: Shape, key: &str, w: f32) {
        let c = self.color(key);
        self.items.push(Item { shape, fill: None, stroke: Some((c, w)), key: Some(key.to_string()) });
    }
    fn plain(&mut self, shape: Shape, c: Color) {
        self.items.push(Item { shape, fill: Some(c), stroke: None, key: None });
    }
    fn label(&mut self, at: Point, s: &str) {
        self.items.push(Item { shape: Shape::Label(at, s.to_string()), fill: None, stroke: None, key: None });
    }
    /// A game picture tinted by a key's colour (clicking it picks that colour).
    fn img(&mut self, r: Rectangle, name: &str, key: &str) {
        let c = self.color(key);
        self.items.push(Item { shape: Shape::Img(r, name.to_string()), fill: Some(c), stroke: None, key: Some(key.to_string()) });
    }
    /// A game picture as it is.
    /// A place a click picks `key`'s colour, drawn as nothing; later ones win where they overlap.
    fn area(&mut self, r: Rectangle, key: &str) {
        self.items.push(Item { shape: Shape::Hit(r), fill: None, stroke: None, key: Some(key.to_string()) });
    }
    fn img_plain(&mut self, r: Rectangle, name: &str) {
        self.items.push(Item { shape: Shape::Img(r, name.to_string()), fill: Some(Color::WHITE), stroke: None, key: None });
    }
    /// A note or fret built up from its layers, as on the highway: (sprite, key or none, dy).
    fn stacked(&mut self, r: Rectangle, canvas: (u32, u32), layers: &[(String, Option<String>, i64)]) {
        for (sprite, key, dy) in layers {
            let name = layer_name(sprite, canvas, *dy);
            match key {
                Some(k) => self.img(r, &name, k),
                None => self.img_plain(r, &name),
            }
        }
    }
}

/// A rectangle `w` by `h` centred on (`cx`, `cy`).
fn rc(cx: f32, cy: f32, w: f32, h: f32) -> Rectangle {
    Rectangle::new(Point::new(cx - w / 2.0, cy - h / 2.0), Size::new(w, h))
}

const HIGHWAY: Color = Color { r: 0.07, g: 0.07, b: 0.085, a: 1.0 };
const LANE_LINE: Color = Color { r: 0.2, g: 0.2, b: 0.24, a: 1.0 };

/// The highway's lanes: (left edge, lane width, centre of each lane).
fn lanes(size: Size, n: usize) -> (f32, f32, Vec<f32>) {
    lanes_up_to(size, n, 70.0)
}

/// As `lanes`, with lanes up to `max` wide.
fn lanes_up_to(size: Size, n: usize, max: f32) -> (f32, f32, Vec<f32>) {
    let w = (size.width - 120.0).clamp(160.0, max * n as f32);
    let lw = w / n as f32;
    let x0 = 110.0 + ((size.width - 110.0) - w) / 2.0;
    (x0, lw, (0..n).map(|i| x0 + lw * (i as f32 + 0.5)).collect())
}

/// Note rows above the strikeline, evenly spaced.
fn rows(strike: f32, n: usize) -> Vec<f32> {
    let top = 40.0;
    let gap = ((strike - 60.0 - top) / n as f32).max(30.0);
    (0..n).map(|i| strike - 70.0 - gap * i as f32).collect()
}

impl<'a> Preview<'a> {
    fn scene(&'a self, size: Size) -> Vec<Item> {
        let mut s = Scene { pv: self, items: Vec::new() };
        match self.section {
            "guitar" if self.art_layout => guitar_pieces(&mut s, size),
            "drums" if self.art_layout => drum_pieces(&mut s, size),
            "guitar" => guitar(&mut s, size),
            "drums" => drums(&mut s, size),
            "sixfret" if self.art_layout => sixfret_art(&mut s, size),
            "sixfret" => sixfret(&mut s, size),
            _ if self.art_layout => other_art(&mut s, size),
            _ => other(&mut s, size),
        }
        s.items
    }
}

fn highway(s: &mut Scene, size: Size, n: usize) -> (f32, f32, Vec<f32>, f32) {
    highway_up_to(s, size, n, 70.0)
}

fn highway_up_to(s: &mut Scene, size: Size, n: usize, max: f32) -> (f32, f32, Vec<f32>, f32) {
    let (x0, lw, xs) = lanes_up_to(size, n, max);
    s.plain(Shape::Rect(Rectangle::new(Point::new(x0, 0.0), Size::new(lw * n as f32, size.height)), 6.0), HIGHWAY);
    for i in 1..n {
        s.plain(Shape::Rect(Rectangle::new(Point::new(x0 + lw * i as f32 - 0.5, 0.0), Size::new(1.0, size.height)), 0.0), LANE_LINE);
    }
    (x0, lw, xs, size.height - 55.0)
}

fn gem(s: &mut Scene, at: Point, r: f32, fill: &str, detail: &str) {
    s.fill(Shape::Circle(at, r), fill);
    s.ring(Shape::Circle(at, r * 0.72), detail, (r * 0.22).max(2.0));
    s.plain(Shape::Circle(at, r * 0.28), Color::WHITE);
}

fn sustain(s: &mut Scene, x: f32, from: f32, to: f32, w: f32, key: &str) {
    s.fill(Shape::Rect(Rectangle::new(Point::new(x - w / 2.0, to), Size::new(w, from - to)), w / 2.0), key);
}

fn guitar(s: &mut Scene, size: Size) {
    let lanes5 = ["green", "red", "yellow", "blue", "orange"];
    let (x0, lw, xs, strike) = highway(s, size, 5);
    let y = rows(strike, 4);
    let r = (lw * 0.3).min(22.0);
    // Open notes light the whole strikeline.
    s.fill(Shape::Rect(Rectangle::new(Point::new(x0, strike - 4.0), Size::new(lw * 5.0, 8.0)), 4.0), "striker_head_light_open");
    for (i, c) in lanes5.iter().enumerate() {
        let at = Point::new(xs[i], strike);
        s.fill(Shape::Circle(at, lw * 0.42), &format!("striker_base_{c}"));
        s.fill(Shape::Circle(at, lw * 0.35), &format!("striker_cover_{c}"));
        s.fill(Shape::Circle(at, lw * 0.24), &format!("striker_head_cover_{c}"));
        s.fill(Shape::Circle(at, lw * 0.12), &format!("striker_head_light_{c}"));
    }
    // Notes, two with sustains.
    for (i, c) in lanes5.iter().enumerate() {
        if i == 0 || i == 3 {
            sustain(s, xs[i], y[0], y[0] - (y[0] - y[1]) * 0.75, r * 0.55, &format!("sustain_{c}"));
        }
        gem(s, Point::new(xs[i], y[0]), r, &format!("note_{c}"), &format!("note_anim_{c}"));
    }
    // An open note.
    sustain(s, xs[2], y[1], y[1] - (y[1] - y[2]) * 0.7, r * 0.55, "sustain_open");
    s.fill(Shape::Rect(Rectangle::new(Point::new(x0 + 8.0, y[1] - 8.0), Size::new(lw * 5.0 - 16.0, 16.0)), 8.0), "note_open");
    s.ring(Shape::Rect(Rectangle::new(Point::new(x0 + 14.0, y[1] - 3.0), Size::new(lw * 5.0 - 28.0, 6.0)), 3.0), "note_anim_open", 2.0);
    // Star power.
    for (row, (kind, label)) in [("sp_phrase", "Star power phrase"), ("sp_active", "Star power active")].iter().enumerate() {
        let yy = y[2 + row];
        sustain(s, xs[2], yy, yy - (y[1] - y[2]) * 0.6, r * 0.55, &format!("sustain_{kind}"));
        for i in [1, 2] {
            gem(s, Point::new(xs[i], yy), r, &format!("note_{kind}"), &format!("note_anim_{kind}"));
        }
        s.label(Point::new(8.0, yy - 7.0), label);
    }
    s.label(Point::new(8.0, y[0] - 7.0), "Notes");
    s.label(Point::new(8.0, y[1] - 7.0), "Open note");
    s.label(Point::new(8.0, strike - 7.0), "Strikers");
}

fn diamond(c: Point, r: f32) -> Shape {
    Shape::Poly(vec![Point::new(c.x, c.y - r * 0.7), Point::new(c.x + r * 1.2, c.y), Point::new(c.x, c.y + r * 0.7), Point::new(c.x - r * 1.2, c.y)])
}

fn drums(s: &mut Scene, size: Size) {
    let pads = ["red", "yellow", "blue", "green"];
    let (x0, lw, xs, strike) = highway(s, size, 4);
    let y = rows(strike, 5);
    let r = (lw * 0.3).min(24.0);
    let full = lw * 4.0;
    s.fill(Shape::Rect(Rectangle::new(Point::new(x0, strike - 4.0), Size::new(full, 8.0)), 4.0), "drums_striker_head_light_kick");
    for (i, c) in pads.iter().enumerate() {
        let at = Point::new(xs[i], strike);
        s.fill(Shape::Circle(at, lw * 0.4), &format!("drums_striker_base_{c}"));
        s.fill(Shape::Circle(at, lw * 0.33), &format!("drums_striker_cover_{c}"));
        s.fill(Shape::Circle(at, lw * 0.23), &format!("drums_striker_head_cover_{c}"));
        s.fill(Shape::Circle(at, lw * 0.11), &format!("drums_striker_head_light_{c}"));
    }
    let kick = |s: &mut Scene, yy: f32, fill: &str, detail: &str| {
        s.fill(Shape::Rect(Rectangle::new(Point::new(x0 + 6.0, yy - 5.0), Size::new(full - 12.0, 10.0)), 5.0), fill);
        s.ring(Shape::Rect(Rectangle::new(Point::new(x0 + 12.0, yy - 2.0), Size::new(full - 24.0, 4.0)), 2.0), detail, 1.5);
    };
    let tom = |s: &mut Scene, x: f32, yy: f32, fill: &str, detail: &str| {
        s.fill(Shape::Rect(Rectangle::new(Point::new(x - r * 1.2, yy - r * 0.55), Size::new(r * 2.4, r * 1.1)), r * 0.4), fill);
        s.ring(Shape::Rect(Rectangle::new(Point::new(x - r * 0.85, yy - r * 0.25), Size::new(r * 1.7, r * 0.5)), r * 0.2), detail, 2.0);
    };
    let cym = |s: &mut Scene, x: f32, yy: f32, fill: &str, detail: &str| {
        s.fill(diamond(Point::new(x, yy), r), fill);
        s.ring(diamond(Point::new(x, yy), r * 0.6), detail, 2.0);
    };
    for (i, c) in pads.iter().enumerate() {
        tom(s, xs[i], y[0], &format!("tom_{c}"), &format!("tom_anim_{c}"));
    }
    for (i, c) in pads.iter().enumerate() {
        cym(s, xs[i], y[1], &format!("cym_{c}"), &format!("cym_anim_{c}"));
    }
    kick(s, y[2], "note_kick", "note_anim_kick");
    for (row, (kind, label)) in [("sp_phrase", "Star power phrase"), ("sp_active", "Star power active")].iter().enumerate() {
        let yy = y[3 + row];
        kick(s, yy + r * 0.9, &format!("note_kick_{kind}"), &format!("note_anim_kick_{kind}"));
        tom(s, xs[0], yy, &format!("tom_{kind}"), &format!("tom_anim_{kind}"));
        cym(s, xs[2], yy, &format!("cym_{kind}"), &format!("cym_anim_{kind}"));
        s.label(Point::new(8.0, yy - 7.0), label);
    }
    s.label(Point::new(8.0, y[0] - 7.0), "Toms");
    s.label(Point::new(8.0, y[1] - 7.0), "Cymbals");
    s.label(Point::new(8.0, y[2] - 7.0), "Kick");
    s.label(Point::new(8.0, strike - 7.0), "Pads");
}

fn sixfret(s: &mut Scene, size: Size) {
    let pos = ["left", "mid", "right"];
    let (x0, lw, xs, strike) = highway(s, size, 3);
    let y = rows(strike, 6);
    let r = (lw * 0.22).min(20.0);
    for (i, p) in pos.iter().enumerate() {
        s.fill(Shape::Rect(Rectangle::new(Point::new(xs[i] - lw * 0.4, strike - lw * 0.32), Size::new(lw * 0.8, lw * 0.64)), 8.0), &format!("sf_striker_background_{p}"));
        s.fill(Shape::Circle(Point::new(xs[i], strike - lw * 0.15), r), &format!("sf_striker_base_black_{p}"));
        s.fill(Shape::Circle(Point::new(xs[i], strike + lw * 0.15), r), &format!("sf_striker_base_white_{p}"));
    }
    let note = |s: &mut Scene, x: f32, yy: f32, key: &str| {
        s.fill(Shape::Rect(Rectangle::new(Point::new(x - r * 1.5, yy - r * 0.6), Size::new(r * 3.0, r * 1.2)), r * 0.6), key);
    };
    for (i, p) in pos.iter().enumerate() {
        if i == 1 {
            sustain(s, xs[i], y[0], y[0] - (y[0] - y[1]) * 0.7, r * 0.6, &format!("sf_sustain_{p}"));
        }
        note(s, xs[i], y[0], &format!("sf_note_white_{p}"));
        note(s, xs[i], y[1], &format!("sf_note_black_{p}"));
        note(s, xs[i], y[3], &format!("sf_note_tap_black_{p}"));
        note(s, xs[i], y[3] - r * 1.5, &format!("sf_note_tap_white_{p}"));
    }
    sustain(s, xs[1], y[2], y[2] - (y[2] - y[3]) * 0.5, r * 0.6, "sf_sustain_open");
    s.fill(Shape::Rect(Rectangle::new(Point::new(x0 + 8.0, y[2] - 7.0), Size::new(lw * 3.0 - 16.0, 14.0)), 7.0), "sf_note_open");
    s.ring(Shape::Rect(Rectangle::new(Point::new(x0 + 8.0, y[3] + r * 1.4), Size::new(lw * 3.0 - 16.0, 10.0)), 5.0), "sf_note_tap_open", 2.0);
    s.ring(Shape::Rect(Rectangle::new(Point::new(xs[0] - r * 1.6, y[1] - r * 0.7), Size::new(r * 3.2, r * 1.4)), r * 0.7), "sf_note_hopo", 2.0);
    for (row, (kind, label)) in [("sp_phrase", "Star power phrase"), ("sp_active", "Star power active")].iter().enumerate() {
        let yy = y[4 + row];
        sustain(s, xs[2], yy, yy - 30.0, r * 0.6, &format!("sf_sustain_{kind}"));
        note(s, xs[0], yy, &format!("sf_note_{kind}"));
        note(s, xs[2], yy, &format!("sf_note_{kind}"));
        s.label(Point::new(8.0, yy - 7.0), label);
    }
    s.label(Point::new(8.0, y[0] - 7.0), "White notes");
    s.label(Point::new(8.0, y[1] - 7.0), "Black · HOPO");
    s.label(Point::new(8.0, y[2] - 7.0), "Open note");
    s.label(Point::new(8.0, y[3] - 7.0), "Taps");
    s.label(Point::new(8.0, strike - 7.0), "Frets");
}

/// Lays out a section's pieces: titled groups of tiles, each a picture on a dark tile.
struct Pieces {
    x: f32,
    w: f32,
    y: f32,
    /// Height of a note tile; everything is sized from it so it all fits.
    u: f32,
}

impl Pieces {
    fn new(size: Size, rows: f32, titles: f32) -> Self {
        let w = (size.width - 16.0).min(500.0);
        // Tiles are half as tall as wide; leave room for each group's title.
        let u = ((size.height - 12.0 - titles * 30.0) / rows).clamp(18.0, 48.0).min(w / 12.0);
        Pieces { x: (size.width - w) / 2.0, w, y: 12.0, u }
    }
    fn title(&mut self, s: &mut Scene, t: &str) {
        s.label(Point::new(self.x, self.y), t);
        self.y += 20.0;
    }
    /// A row of `n` tiles (`span` columns of a 5-column grid each), returned as rectangles.
    fn row(&mut self, s: &mut Scene, n: usize, span: f32) -> Vec<Rectangle> {
        let cw = self.w / 5.0;
        let (tw, th) = (cw * span - 6.0, self.u * 1.15);
        let out = (0..n)
            .map(|i| {
                let r = Rectangle::new(Point::new(self.x + i as f32 * cw * span, self.y), Size::new(tw, th));
                s.plain(Shape::Rect(r, 6.0), HIGHWAY);
                r
            })
            .collect();
        self.y += th + 6.0;
        out
    }
    fn gap(&mut self) {
        self.y += 8.0;
    }
}

/// A picture of aspect `aspect` (width / height) fitted in a tile, centred.
fn fit(tile: Rectangle, aspect: f32) -> Rectangle {
    let (mut w, mut h) = (tile.width - 6.0, tile.height - 4.0);
    if w / h > aspect {
        w = h * aspect;
    } else {
        h = w / aspect;
    }
    rc(tile.center_x(), tile.center_y(), w, h)
}

type Layers = Vec<(String, Option<String>, i64)>;

fn lay(sprite: &str, key: Option<String>, dy: i64) -> (String, Option<String>, i64) {
    (sprite.to_string(), key, dy)
}

/// A guitar note's layers: `kind` 0 strum, 1 HOPO, 2 tap; `sp` the star power phrase shape.
fn guitar_note(kind: usize, sp: bool, body: String, detail: String) -> Layers {
    if sp {
        let (glow, cap) = match kind {
            2 => ("guitar/guitarNote_SP_BodyTap9", "guitar/guitarNote_SP_TapCap"),
            1 => ("guitar/guitarNote_SP_Body9", "guitar/guitarNote_SP_HopoCap1"),
            _ => ("guitar/guitarNote_SP_Body9", "guitar/guitarNote_SP_StrumCap"),
        };
        vec![lay("guitar/guitarNote_SP_Base9", Some(detail), 0), lay(glow, Some(body), 0), lay(cap, None, 0)]
    } else {
        let base = ["guitar/guitarNote_Base_Strum", "guitar/guitarNote_Base_Hopo", "guitar/guitarNote_Base_Tap"][kind];
        let b = if kind == 2 { "guitar/guitarNote_Body_Tap" } else { "guitar/guitarNote_Body_Strum" };
        vec![lay(base, None, 0), lay(b, Some(body), 0), lay("guitar/guitarNote_Strum_Shine12", Some(detail), -56)]
    }
}

/// A strike ring's layers (`n` 1-5 left to right, or a drum pad's sprite numbers), lit by `light`.
fn ring(prefix: &str, under: String, head: String, ring_n: u32, keys: [String; 3], light: Option<String>) -> Layers {
    let [base, cover, head_cover] = keys;
    let mut v = vec![
        lay(&under, None, 0),
        lay(&head, Some(base), 0),
        lay(&format!("{prefix}/striker_Ring{ring_n}"), Some(cover.clone()), 0),
        lay(&format!("{prefix}/striker_RingCover{ring_n}"), Some(cover), 0),
        lay(&format!("{prefix}/striker_HeadColor{ring_n}"), Some(head_cover), 0),
    ];
    if let Some(l) = light {
        v.push(lay(&format!("{prefix}/striker_HeadGlow{ring_n}"), Some(l), 0));
    }
    v
}

/// Guitar's pieces with the game's pictures: notes, the open note, star power, sustains, and
/// the strike rings resting and lit. Clicking a layer picks its colour.
fn guitar_pieces(s: &mut Scene, size: Size) {
    const LANES: [&str; 5] = ["green", "red", "yellow", "blue", "orange"];
    const NOTE: (u32, u32) = (128, 64);
    let mut p = Pieces::new(size, 13.5, 6.0);
    p.title(s, "Notes: strum, HOPO, tap");
    for kind in 0..3 {
        for (i, t) in p.row(s, 5, 1.0).into_iter().enumerate() {
            let c = LANES[i];
            s.stacked(fit(t, 2.0), NOTE, &guitar_note(kind, false, format!("note_{c}"), format!("note_anim_{c}")));
        }
    }
    p.gap();
    p.title(s, "Open note");
    let t = p.row(s, 1, 5.0)[0];
    s.stacked(
        fit(t, 410.0 / 51.0),
        (410, 51),
        &[
            lay("guitar/guitarNote_Open_Base", None, 0),
            lay("guitar/guitarNote_Open_Body1", Some("note_open".into()), 0),
            lay("guitar/guitarNote_Open_Shine1", Some("note_anim_open".into()), -5),
        ],
    );
    p.gap();
    p.title(s, "Star power: phrase · notes while active · phrase while active (strum, HOPO, tap)");
    let t = p.row(s, 3, 1.0);
    for kind in 0..3 {
        s.stacked(fit(t[kind], 2.0), NOTE, &guitar_note(kind, true, "note_sp_phrase".into(), "note_anim_sp_phrase".into()));
    }
    let t2 = p.row(s, 3, 1.0);
    for kind in 0..3 {
        s.stacked(fit(t2[kind], 2.0), NOTE, &guitar_note(kind, false, "note_sp_active".into(), "note_anim_sp_active".into()));
    }
    let t3 = p.row(s, 3, 1.0);
    for kind in 0..3 {
        s.stacked(fit(t3[kind], 2.0), NOTE, &guitar_note(kind, true, "note_sp_phrase_active".into(), "note_anim_sp_phrase_active".into()));
    }
    p.gap();
    p.title(s, "Sustains: lanes, open, star power phrase, active, phrase while active");
    let keys = ["green", "red", "yellow", "blue", "orange", "open", "sp_phrase", "sp_active", "sp_phrase_active"];
    let bw = p.w / keys.len() as f32;
    for (i, k) in keys.iter().enumerate() {
        let r = Rectangle::new(Point::new(p.x + i as f32 * bw + bw * 0.3, p.y), Size::new(bw * 0.4, p.u * 1.2));
        s.fill(Shape::Rect(r, bw * 0.2), &format!("sustain_{k}"));
    }
    p.y += p.u * 1.2 + 6.0;
    p.gap();
    p.title(s, "Strike rings: resting, lit (and lit by an open note)");
    for lit in [false, true] {
        let tiles = p.row(s, 5, 1.0);
        for (i, t) in tiles.into_iter().enumerate() {
            let (c, n) = (LANES[i], i as u32 + 1);
            let light = lit.then(|| format!("striker_head_light_{c}"));
            s.stacked(
                fit(t, 2.0),
                NOTE,
                &ring(
                    "guitar",
                    format!("guitar/striker_GuitarUnderlay{n}"),
                    format!("guitar/striker_GuitarHead{n}"),
                    n,
                    [format!("striker_base_{c}"), format!("striker_cover_{c}"), format!("striker_head_cover_{c}")],
                    light,
                ),
            );
        }
    }
    let t = p.row(s, 1, 1.0)[0];
    s.stacked(
        fit(t, 2.0),
        NOTE,
        &ring(
            "guitar",
            "guitar/striker_GuitarUnderlay3".into(),
            "guitar/striker_GuitarHead3".into(),
            3,
            ["striker_base_yellow".into(), "striker_cover_yellow".into(), "striker_head_cover_yellow".into()],
            Some("striker_head_light_open".into()),
        ),
    );
}

/// Drums' pieces with the game's pictures: toms, cymbals, the kick, star power, and the pads
/// resting and lit.
fn drum_pieces(s: &mut Scene, size: Size) {
    const PADS: [&str; 4] = ["red", "yellow", "blue", "green"];
    const RINGS: [(u32, u32); 4] = [(1, 1), (2, 2), (4, 3), (5, 4)];
    const NOTE: (u32, u32) = (128, 64);
    const KICK: (u32, u32) = (1024, 64);
    let tom = |body: String, detail: String, sp: bool| -> Layers {
        let b = if sp { "drums/drumNote_Tom_SP_Body1" } else { "drums/drumNote_Tom_Regular_Body" };
        vec![lay("drums/drumNote_Tom_Regular_Base", None, 0), lay(b, Some(body), 0), lay("drums/drumNote_Tom_Shine4", Some(detail), 0)]
    };
    let cym = |body: String, detail: String, sp: bool| -> Layers {
        let mut v = vec![lay("drums/drumNote_Cymbal_Regular_Base", None, 0)];
        if sp {
            v.push(lay("drums/drumNote_Cymbal_SP_Glow1", Some(body.clone()), 0));
        }
        v.push(lay("drums/drumNote_Cymbal_Regular_Body", Some(body), 0));
        v.push(lay("drums/drumNote_Cymbal_Shine12", Some(detail), 0));
        v
    };
    let kick = |body: String, detail: String| -> Layers {
        vec![lay("drums/drumNote_Kick_Base", None, 0), lay("drums/drumNote_Kick_Body", Some(body), 0), lay("drums/drumNote_Kick_Shine8", Some(detail), 0)]
    };
    let mut p = Pieces::new(size, 13.0, 5.0);
    p.title(s, "Toms and cymbals");
    for (i, t) in p.row(s, 4, 1.25).into_iter().enumerate() {
        let c = PADS[i];
        s.stacked(fit(t, 2.0), NOTE, &tom(format!("tom_{c}"), format!("tom_anim_{c}"), false));
    }
    for (i, t) in p.row(s, 4, 1.25).into_iter().enumerate() {
        let c = PADS[i];
        s.stacked(fit(t, 2.0), NOTE, &cym(format!("cym_{c}"), format!("cym_anim_{c}"), false));
    }
    p.gap();
    p.title(s, "Kick");
    let t = p.row(s, 1, 5.0)[0];
    s.stacked(fit(t, 16.0), KICK, &kick("note_kick".into(), "note_anim_kick".into()));
    p.gap();
    p.title(s, "Star power: phrase · notes while active · phrase while active (tom, cymbal, kick)");
    for (kind, sp) in [("sp_phrase", true), ("sp_active", false), ("sp_phrase_active", true)] {
        let t = p.row(s, 3, 1.0);
        s.stacked(fit(t[0], 2.0), NOTE, &tom(format!("tom_{kind}"), format!("tom_anim_{kind}"), sp));
        s.stacked(fit(t[1], 2.0), NOTE, &cym(format!("cym_{kind}"), format!("cym_anim_{kind}"), sp));
        let k = Rectangle::new(t[2].position(), Size::new(p.w - t[2].x + p.x - 6.0, t[2].height));
        s.plain(Shape::Rect(k, 6.0), HIGHWAY);
        s.stacked(fit(k, 16.0), KICK, &kick(format!("note_kick_{kind}"), format!("note_anim_kick_{kind}")));
    }
    p.gap();
    p.title(s, "Pads: resting, lit (and lit by a kick)");
    for lit in [false, true] {
        for (i, t) in p.row(s, 4, 1.25).into_iter().enumerate() {
            let c = PADS[i];
            let (ring_n, head) = RINGS[i];
            s.stacked(
                fit(t, 2.0),
                NOTE,
                &ring(
                    "drums",
                    format!("drums/drumStriker_Underlay{head}"),
                    format!("drums/drumStriker_Head{head}"),
                    ring_n,
                    [format!("drums_striker_base_{c}"), format!("drums_striker_cover_{c}"), format!("drums_striker_head_cover_{c}")],
                    lit.then(|| format!("drums_striker_head_light_{c}")),
                ),
            );
        }
    }
    let t = p.row(s, 1, 1.25)[0];
    s.stacked(
        fit(t, 2.0),
        NOTE,
        &ring(
            "drums",
            "drums/drumStriker_Underlay1".into(),
            "drums/drumStriker_Head1".into(),
            1,
            ["drums_striker_base_red".into(), "drums_striker_cover_red".into(), "drums_striker_head_cover_red".into()],
            Some("drums_striker_head_light_kick".into()),
        ),
    );
}

/// 6-fret with the game's pictures: the strikeline and its buttons, white ("up") and black
/// ("down") notes, HOPOs and taps, the open (bar) note, and star power.
fn sixfret_art(s: &mut Scene, size: Size) {
    const BUTTONS: [&str; 3] = ["6fret_strikeline_buttons_0", "6fret_strikeline_buttons_1", "6fret_strikeline_buttons_2"];
    let pos = ["left", "mid", "right"];
    // Wider lanes than the drawn preview, so the notes are big enough to make out.
    let (x0, lw, xs, strike) = highway_up_to(s, size, 3, 110.0);
    let y = rows(strike, 8);
    let gap = (y[0] - y[1]).abs();
    let n = (lw * 0.62).min(gap * 1.05).min(72.0);
    let full = lw * 3.0;
    // Strikeline: the bar, a button plate per lane, and its black and white buttons.
    s.img_plain(rc(x0 + full / 2.0, strike, full, full / 13.77), "6fret_strikeline_base");
    for (i, p) in pos.iter().enumerate() {
        let (w, h) = (lw * 0.9, lw * 0.9 / 4.59);
        s.img(rc(xs[i], strike, w, h), BUTTONS[i], &format!("sf_striker_background_{p}"));
        let k = (h * 1.6).min(n * 0.7);
        s.img(rc(xs[i] - k * 0.6, strike - h * 0.2, k, k), "down-body", &format!("sf_striker_base_black_{p}"));
        s.img(rc(xs[i] + k * 0.6, strike - h * 0.2, k, k), "up-body", &format!("sf_striker_base_white_{p}"));
    }
    // A note: body in its colour, then an inside colour (taps), the rim, and a glow (HOPOs).
    let note = |s: &mut Scene, white: bool, x: f32, yy: f32, body: &str, inside: Option<&str>, glow: Option<&str>| {
        let (b, i, r, g) = if white {
            ("up-body", "up-inside-overlay", "up-ring", "up-overlay-glow")
        } else {
            ("down-body", "down-inside-overlay", "down-ring", "down-overlay-glow")
        };
        let rect = rc(x, yy, n, n);
        s.img(rect, b, body);
        if let Some(k) = inside {
            s.img(rect, i, k);
        }
        s.img_plain(rect, r);
        if let Some(k) = glow {
            s.img(rect, g, k);
        }
    };
    let bar = |s: &mut Scene, yy: f32, body: &str, glow: Option<&str>| {
        let rect = rc(x0 + full / 2.0, yy, full * 0.94, n * 0.8);
        s.img(rect, "bar-chord-body", body);
        s.img_plain(rect, "bar-chord-ring");
        if let Some(k) = glow {
            s.img(rect, "bar-chord-overlay-glow", k);
        }
    };
    for (i, p) in pos.iter().enumerate() {
        if i == 1 {
            sustain(s, xs[i], y[0], y[0] - gap * 0.7, n * 0.3, &format!("sf_sustain_{p}"));
        }
        note(s, true, xs[i], y[0], &format!("sf_note_white_{p}"), None, None);
        note(s, false, xs[i], y[1], &format!("sf_note_black_{p}"), None, (i == 0).then_some("sf_note_hopo"));
        note(s, true, xs[i], y[3], &format!("sf_note_white_{p}"), Some(&format!("sf_note_tap_white_{p}")), None);
        note(s, false, xs[i], y[4], &format!("sf_note_black_{p}"), Some(&format!("sf_note_tap_black_{p}")), None);
    }
    sustain(s, xs[1], y[2], y[2] - gap * 0.6, n * 0.3, "sf_sustain_open");
    bar(s, y[2], "sf_note_open", None);
    bar(s, y[5], "sf_note_open", Some("sf_note_tap_open"));
    for (row, (kind, label)) in [("sp_phrase", "Star power phrase"), ("sp_active", "Star power active")].iter().enumerate() {
        let yy = y[6 + row];
        sustain(s, xs[2], yy, yy - gap * 0.6, n * 0.3, &format!("sf_sustain_{kind}"));
        note(s, true, xs[0], yy, &format!("sf_note_{kind}"), None, None);
        note(s, false, xs[2], yy, &format!("sf_note_{kind}"), None, None);
        s.label(Point::new(8.0, yy - 7.0), label);
    }
    s.label(Point::new(8.0, y[0] - 7.0), "White notes");
    s.label(Point::new(8.0, y[1] - 7.0), "Black · HOPO");
    s.label(Point::new(8.0, y[2] - 7.0), "Open note");
    s.label(Point::new(8.0, y[3] - 7.0), "White taps");
    s.label(Point::new(8.0, y[4] - 7.0), "Black taps");
    s.label(Point::new(8.0, y[5] - 7.0), "Open tap");
    s.label(Point::new(8.0, strike - 7.0), "Strikers");
}

/// The other colours with the game's pictures: the combo meter at each level, the star power
/// bar, star power's effects, hit flames and sparks, and the leaderboard.
fn other_art(s: &mut Scene, size: Size) {
    // A margin each side, so a selection box round the widest pieces has room.
    let w = (size.width - 16.0).min(480.0);
    let x = (size.width - w) / 2.0;
    let mut y = 16.0;
    // Combo meter: ×1 to ×4, and star power.
    s.label(Point::new(x, y), "Combo meter");
    y += 20.0;
    let combos = [
        ("combo_one", None, "multi-font-overlay_2"),
        ("combo_two", Some("combo_two_glow"), "multi-font-overlay_3"),
        ("combo_three", Some("combo_three_glow"), "multi-font-overlay_4"),
        ("combo_four", Some("combo_four_glow"), "multi-font-overlay_5"),
        ("combo_sp_active", Some("combo_sp_active_glow"), "multi-font-overlay_9"),
    ];
    let cw = w / 5.0;
    let d = (cw - 6.0).min(86.0);
    for (i, (k, glow, digit)) in combos.iter().enumerate() {
        let cx = x + cw * (i as f32 + 0.5);
        let cy = y + d / 2.0;
        s.img_plain(rc(cx, cy, d * 0.83, d * 0.84), "multi-bg");
        s.img(rc(cx, cy, d, d), "multi-overlay", k);
        let dr = rc(cx + d * 0.06, cy, d * 0.36, d * 0.39);
        match glow {
            Some(g) => s.img(dr, digit, g),
            None => s.img_plain(dr, digit),
        }
        s.img_plain(rc(cx - d * 0.2, cy + d * 0.05, d * 0.3, d * 0.3), "multi-x");
        // Anywhere on the meter: its colour; the middle (the number): the number's glow.
        s.area(rc(cx, cy, d, d), k);
        if let Some(g) = glow {
            s.area(rc(cx, cy, d * 0.56, d * 0.56), g);
        }
    }
    y += d + 18.0;
    // Star power bar, partly filled.
    s.label(Point::new(x, y), "Star power bar");
    y += 22.0;
    let (bw, bh) = (w, w / 4.76);
    let bar = Rectangle::new(Point::new(x, y), Size::new(bw, bh));
    s.img_plain(bar, "power_meter_basepng");
    let fill = Rectangle::new(Point::new(x + bw * 0.078, y + bh * 0.226), Size::new(bw * 0.845 * 0.62, bh * 0.548));
    s.img(fill, "power_meter_fill", "sp_bar_color");
    s.img(Rectangle::new(fill.position(), Size::new(fill.width, fill.height)), "lightning_merged_16_final_0", "sp_bar_elec");
    s.img(rc(fill.x + fill.width, y + bh / 2.0, bh * 0.9, bh * 0.79), "arrow_glow", "sp_bar_arrow");
    y += bh + 16.0;
    // Star power's effects, each on its own dark tile.
    s.label(Point::new(x, y), "Star power: general, active, activation, flash, gain lightning");
    y += 22.0;
    let fx: [(&str, &'static str); 6] = [
        ("general_sp", "SP_highlight_0"),
        ("general_sp_active", "sp_color_anim_0"),
        ("sp_act_animation", "activationfade2"),
        ("sp_act_flash", "pulse"),
        ("sp_gain_lightning", "lightning_merged_16_final_0"),
        ("sp_gain_lightning_secondary", "Bolt"),
    ];
    let tw = (w - 5.0 * 8.0) / 6.0;
    for (i, (k, pic)) in fx.iter().enumerate() {
        let r = Rectangle::new(Point::new(x + i as f32 * (tw + 8.0), y), Size::new(tw, tw * 0.75));
        s.plain(Shape::Rect(r, 6.0), HIGHWAY);
        s.img(Rectangle::new(Point::new(r.x + 4.0, r.y + 4.0), Size::new(r.width - 8.0, r.height - 8.0)), pic, k);
        s.area(r, k);
    }
    y += tw * 0.75 + 18.0;
    // Hit effects.
    s.label(Point::new(x, y), "Hit flames, particles and hold sparks (normal, star power, kick, open)");
    y += 22.0;
    let fw = w / 4.0;
    let fs = (fw - 10.0).min(96.0);
    let flames = [
        ("striker_hit_flame", Some(("striker_hit_particles", "striker_hold_spark"))),
        ("striker_hit_flame_sp_active", Some(("striker_hit_particles_sp_active", "striker_hold_spark_sp_active"))),
        ("striker_hit_flame_kick", None),
        ("striker_hit_flame_open", None),
    ];
    for (i, (flame, extra)) in flames.iter().enumerate() {
        let cx = x + fw * (i as f32 + 0.5);
        let r = Rectangle::new(Point::new(cx - fw / 2.0 + 4.0, y), Size::new(fw - 8.0, fs * 1.25));
        s.plain(Shape::Rect(r, 6.0), HIGHWAY);
        match extra {
            Some((parts, spark)) => {
                s.img(rc(cx - fs * 0.22, y + fs * 0.5, fs * 0.8, fs * 0.8), "hit_spark_5", parts);
                s.img(rc(cx - fs * 0.15, y + fs * 0.45, fs * 0.7, fs * 0.7), "hitflame_anim_16_5", flame);
                s.img(rc(cx + fs * 0.3, y + fs * 0.75, fs * 0.55, fs * 0.55), "sustain_spark_anim_color_3", spark);
                // The tile in parts: the flame on the left, particles top right, hold sparks
                // bottom right.
                let split = r.x + r.width * 0.6;
                s.area(Rectangle::new(r.position(), Size::new(split - r.x, r.height)), flame);
                s.area(Rectangle::new(Point::new(split, r.y), Size::new(r.x + r.width - split, r.height * 0.5)), parts);
                s.area(Rectangle::new(Point::new(split, r.y + r.height * 0.5), Size::new(r.x + r.width - split, r.height * 0.5)), spark);
            }
            None => {
                let pic = if *flame == "striker_hit_flame_kick" { "open_flame_drum" } else { "open_flame" };
                s.img(rc(cx, y + fs * 0.6, fw - 16.0, (fw - 16.0) / 1.6), pic, flame);
                s.area(r, flame);
            }
        }
    }
    y += fs * 1.25 + 18.0;
    // Leaderboard.
    s.label(Point::new(x, y), "Leaderboard: 1st, 2nd, 3rd");
    y += 20.0;
    for (i, k) in ["leaderboard_first", "leaderboard_second", "leaderboard_third"].iter().enumerate() {
        let r = rc(x + 22.0 + i as f32 * 50.0, y + 22.0, 44.0, 44.0);
        s.img(r, "score_star_grey_back", k);
        s.img(r, "score_star_outline_grey", k);
        s.area(rc(r.center_x(), r.center_y(), 50.0, 50.0), k);
    }
}

fn other(s: &mut Scene, size: Size) {
    let w = size.width.min(460.0);
    let x = (size.width - w) / 2.0;
    let mut y = 16.0;
    // Combo meter.
    s.label(Point::new(x, y), "Combo meter");
    y += 22.0;
    let combos = [("combo_one", None), ("combo_two", Some("combo_two_glow")), ("combo_three", Some("combo_three_glow")), ("combo_four", Some("combo_four_glow")), ("combo_sp_active", Some("combo_sp_active_glow"))];
    let bw = (w - 4.0 * 10.0) / 5.0;
    for (i, (k, glow)) in combos.iter().enumerate() {
        let r = Rectangle::new(Point::new(x + i as f32 * (bw + 10.0), y), Size::new(bw, 40.0));
        s.fill(Shape::Rect(r, 8.0), k);
        if let Some(g) = glow {
            s.ring(Shape::Rect(Rectangle::new(Point::new(r.x - 3.0, r.y - 3.0), Size::new(r.width + 6.0, r.height + 6.0)), 10.0), g, 3.0);
        }
    }
    y += 64.0;
    // Star power bar.
    s.label(Point::new(x, y), "Star power bar");
    y += 22.0;
    s.plain(Shape::Rect(Rectangle::new(Point::new(x, y), Size::new(w, 26.0)), 6.0), HIGHWAY);
    s.fill(Shape::Rect(Rectangle::new(Point::new(x + 3.0, y + 3.0), Size::new(w * 0.62, 20.0)), 4.0), "sp_bar_color");
    let zig: Vec<Point> = (0..=12).map(|i| Point::new(x + 6.0 + i as f32 * (w * 0.6 / 12.0), y + if i % 2 == 0 { 8.0 } else { 18.0 })).collect();
    let mut zz = zig.clone();
    zz.extend(zig.iter().rev().map(|p| Point::new(p.x, p.y + 1.5)));
    s.fill(Shape::Poly(zz), "sp_bar_elec");
    let ax = x + w * 0.62 + 3.0;
    s.fill(Shape::Poly(vec![Point::new(ax, y - 4.0), Point::new(ax + 12.0, y + 13.0), Point::new(ax, y + 30.0)]), "sp_bar_arrow");
    y += 50.0;
    // Star power, general.
    let swatches = |s: &mut Scene, y: f32, title: &str, keys: &[&str]| {
        s.label(Point::new(x, y), title);
        let bw = (w - (keys.len() as f32 - 1.0) * 10.0) / keys.len() as f32;
        for (i, k) in keys.iter().enumerate() {
            s.fill(Shape::Rect(Rectangle::new(Point::new(x + i as f32 * (bw + 10.0), y + 22.0), Size::new(bw, 26.0)), 6.0), k);
        }
    };
    swatches(s, y, "Star power: general, active, activation, flash, gain lightning", &["general_sp", "general_sp_active", "sp_act_animation", "sp_act_flash", "sp_gain_lightning", "sp_gain_lightning_secondary"]);
    y += 66.0;
    // Hit effects.
    s.label(Point::new(x, y), "Hit flames, particles and hold sparks (normal, star power, kick, open)");
    y += 30.0;
    let flames = [("striker_hit_flame", Some("striker_hit_particles"), Some("striker_hold_spark")), ("striker_hit_flame_sp_active", Some("striker_hit_particles_sp_active"), Some("striker_hold_spark_sp_active")), ("striker_hit_flame_kick", None, None), ("striker_hit_flame_open", None, None)];
    let fw = w / 4.0;
    for (i, (flame, parts, spark)) in flames.iter().enumerate() {
        let cx = x + fw * (i as f32 + 0.5);
        let base = y + 46.0;
        s.fill(Shape::Poly(vec![Point::new(cx - 18.0, base), Point::new(cx - 8.0, base - 30.0), Point::new(cx, base - 46.0), Point::new(cx + 8.0, base - 30.0), Point::new(cx + 18.0, base)]), flame);
        if let Some(p) = parts {
            for (dx, dy) in [(-26.0, -20.0), (24.0, -28.0), (-14.0, -44.0), (18.0, -6.0)] {
                s.fill(Shape::Circle(Point::new(cx + dx, base + dy), 3.0), p);
            }
        }
        if let Some(k) = spark {
            s.fill(Shape::Poly(vec![Point::new(cx - 4.0, base + 6.0), Point::new(cx + 4.0, base + 6.0), Point::new(cx + 1.0, base + 22.0), Point::new(cx - 1.0, base + 22.0)]), k);
        }
    }
    y += 90.0;
    // Leaderboard.
    s.label(Point::new(x, y), "Leaderboard: 1st, 2nd, 3rd");
    y += 22.0;
    for (i, k) in ["leaderboard_first", "leaderboard_second", "leaderboard_third"].iter().enumerate() {
        s.fill(Shape::Circle(Point::new(x + 16.0 + i as f32 * 44.0, y + 16.0), 16.0), k);
    }
}

impl canvas::Program<Msg> for Preview<'_> {
    type State = ();

    fn update(&self, _: &mut (), event: &canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<Msg>> {
        if let canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            let p = cursor.position_in(bounds)?;
            let key = self.hit(bounds.size(), p)?;
            return Some(canvas::Action::publish(Msg::Select(key)).and_capture());
        }
        None
    }

    fn draw(&self, _: &(), renderer: &Renderer, theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<canvas::Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        // Drawn after the pictures (iced puts a frame's pictures over its shapes).
        let mut top = Frame::new(renderer, bounds.size());
        let items = self.scene(bounds.size());
        for it in &items {
            if let Shape::Label(at, s) = &it.shape {
                f.fill_text(canvas::Text { content: s.clone(), position: *at, color: crate::style::muted(theme), size: 12.0.into(), ..Default::default() });
                continue;
            }
            if let Shape::Img(r, name) = &it.shape {
                if let Some(h) = self.art.and_then(|a| a.get(name.as_str(), it.fill.unwrap_or(Color::WHITE))) {
                    f.draw_image(*r, canvas::Image::new(h));
                }
                continue;
            }
            let Some(path) = it.shape.path() else { continue };
            if let Some(c) = it.fill {
                f.fill(&path, c);
            }
            if let Some((c, w)) = it.stroke {
                f.stroke(&path, Stroke::default().with_color(c).with_width(w));
            }
        }
        // The colour being edited, outlined wherever it's used.
        if let Some(sel) = self.selected {
            for it in items.iter().filter(|it| it.key.as_deref() == Some(sel)) {
                // A picture: a box round the whole thing it's part of (every picture overlapping
                // it), with some room.
                if let Shape::Img(r, _) = &it.shape {
                    // The dark tile it sits on, if it has one; otherwise every picture it
                    // overlaps (pictures can spill past their tile).
                    let centre = Point::new(r.center_x(), r.center_y());
                    let tile = items.iter().find_map(|o| match &o.shape {
                        Shape::Rect(q, _) if o.key.is_none() && q.contains(centre) && q.width < bounds.width * 0.9 && q.height < bounds.height * 0.5 => Some(*q),
                        _ => None,
                    });
                    let mut all = *r;
                    if let Some(t) = tile {
                        all = t;
                    } else {
                        for o in &items {
                            if let Shape::Img(q, _) = &o.shape
                                && q.intersects(&all)
                            {
                                all = all.union(q);
                            }
                        }
                    }
                    let pad = 4.0;
                    // Kept inside the preview, so none of it is cut off.
                    let (x0, y0) = ((all.x - pad).max(2.0), (all.y - pad).max(2.0));
                    let (x1, y1) = ((all.x + all.width + pad).min(bounds.width - 2.0), (all.y + all.height + pad).min(bounds.height - 2.0));
                    let at = Rectangle::new(Point::new(x0, y0), Size::new((x1 - x0).max(1.0), (y1 - y0).max(1.0)));
                    let path = Path::rounded_rectangle(at.position(), at.size(), 8.0.into());
                    top.stroke(&path, Stroke::default().with_color(Color::BLACK.scale_alpha(0.7)).with_width(4.0));
                    top.stroke(&path, Stroke::default().with_color(Color::WHITE).with_width(2.0));
                    continue;
                }
                if let Some(path) = it.shape.outline() {
                    top.stroke(&path, Stroke::default().with_color(Color::BLACK.scale_alpha(0.7)).with_width(4.0));
                    top.stroke(&path, Stroke::default().with_color(Color::WHITE).with_width(2.0));
                }
            }
        }
        vec![f.into_geometry(), top.into_geometry()]
    }

    fn mouse_interaction(&self, _: &(), bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        match cursor.position_in(bounds) {
            Some(p) if self.hit(bounds.size(), p).is_some() => mouse::Interaction::Pointer,
            _ => mouse::Interaction::default(),
        }
    }
}

impl Preview<'_> {
    /// The key of the topmost coloured shape under a point.
    fn hit(&self, size: Size, p: Point) -> Option<String> {
        // On a picture, only where it isn't see-through, so a click goes to the layer you see.
        let on = |it: &Item| match (&it.shape, self.art) {
            (Shape::Img(r, name), Some(a)) => r.contains(p) && a.solid(name, *r, p),
            _ => it.shape.bounds().contains(p),
        };
        self.scene(size).into_iter().rev().find(|it| it.key.is_some() && on(it)).and_then(|it| it.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shape in every preview is coloured by a key Clone Hero has.
    #[test]
    fn preview_keys_exist() {
        let d = Profile::defaults();
        for sec in cp::SECTIONS {
            // Drawn, and with the game's pictures (every one of which Truss reads from it).
            for art_layout in [false, true] {
                let pv = Preview { p: &d, d: &d, section: sec, selected: None, art: None, art_layout };
                let items = pv.scene(Size::new(520.0, 640.0));
                assert!(items.iter().any(|i| i.key.is_some()), "{sec}");
                for k in items.iter().filter_map(|i| i.key.as_deref()) {
                    assert!(d.get(sec, k).is_some(), "{sec}: {k}");
                }
                for it in &items {
                    if let Shape::Img(_, name) = &it.shape {
                        let ok = match name.strip_prefix("layer:") {
                            Some(rest) => crate::ch_sprites::files().iter().any(|f| rest.starts_with(&format!("{f}:"))),
                            None => crate::ch_sprites::ART.contains(&name.as_str()),
                        };
                        assert!(ok, "{name}");
                    }
                }
            }
        }
        // On the combo meter, the number picks its glow and the ring the meter's colour.
        let pv = Preview { p: &d, d: &d, section: "other", selected: None, art: None, art_layout: true };
        let size = Size::new(520.0, 640.0);
        let items = pv.scene(size);
        let meter = items.iter().find_map(|it| match &it.shape {
            Shape::Hit(r) if it.key.as_deref() == Some("combo_two") => Some(*r),
            _ => None,
        });
        let m = meter.expect("the x2 meter");
        assert_eq!(pv.hit(size, Point::new(m.center_x(), m.center_y())).as_deref(), Some("combo_two_glow"));
        assert_eq!(pv.hit(size, Point::new(m.x + 3.0, m.center_y())).as_deref(), Some("combo_two"));
        // A click on the green fret picks a green fret colour.
        let pv = Preview { p: &d, d: &d, section: "guitar", selected: None, art: None, art_layout: false };
        let size = Size::new(520.0, 640.0);
        let (_, _, xs) = lanes(size, 5);
        assert!(pv.hit(size, Point::new(xs[0], size.height - 55.0)).is_some_and(|k| k.ends_with("_green")));
    }
}

#[cfg(test)]
mod art_sheet {
    use super::*;

    /// Roughly renders the 6-fret and other previews with the game's pictures from a real
    /// install, to TRUSS_ART_SHEET (a PNG) to look at.
    #[test]
    #[ignore]
    fn art_preview_sheet() {
        let Some(out) = std::env::var_os("TRUSS_ART_SHEET") else { return };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let kit = rt.block_on(crate::ch_sprites::load()).unwrap();
        assert!(!kit.art.is_empty(), "no Clone Hero install found");
        let d = Profile::defaults();
        let (w, h) = (520u32, 720u32);
        let mut sheet = image::RgbaImage::from_pixel(w * 2, h, image::Rgba([24, 24, 28, 255]));
        for (n, sec) in ["guitar", "drums"].iter().enumerate() {
            let pv = Preview { p: &d, d: &d, section: sec, selected: None, art: None, art_layout: true };
            for it in pv.scene(Size::new(w as f32, h as f32)) {
                let c = it.fill.unwrap_or(Color::WHITE).into_rgba8();
                match &it.shape {
                    Shape::Rect(r, _) => {
                        for y in r.y.max(0.0) as u32..((r.y + r.height) as u32).min(h) {
                            for x in r.x.max(0.0) as u32..((r.x + r.width) as u32).min(w) {
                                sheet.put_pixel(x + n as u32 * w, y, image::Rgba(c));
                            }
                        }
                    }
                    Shape::Img(r, name) => {
                        let Some(src) = source(&kit, name) else { continue };
                        let mut img = image::imageops::resize(&src, r.width.max(1.0) as u32, r.height.max(1.0) as u32, image::imageops::FilterType::Triangle);
                        for p in img.pixels_mut() {
                            for i in 0..3 {
                                p[i] = (p[i] as u32 * c[i] as u32 / 255) as u8;
                            }
                        }
                        image::imageops::overlay(&mut sheet, &img, r.x as i64 + (n as u32 * w) as i64, r.y as i64);
                    }
                    _ => {}
                }
            }
        }
        sheet.save(std::path::PathBuf::from(out)).unwrap();
    }
}
