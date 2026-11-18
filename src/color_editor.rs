//! Tools > Note colors: edits Clone Hero colour profiles (see color_profile.rs). Guitar and
//! drums are previewed on the chart preview's highway, playing a made-up chart with notes and
//! frets built from Clone Hero's sprites in the profile's colours (see ch_sprites.rs); 6-fret
//! and the other colours have a drawn preview where clicking a part picks its colour.

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
    OpenFolder,
    Kit(Result<Arc<crate::ch_sprites::Kit>, String>),
    SpActive(bool),
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
        self.section < 2
    }

    /// While the highway is showing, it's redrawn about 60 times a second.
    pub fn subscription(&self) -> Subscription<Msg> {
        if self.on_highway() && self.highway.is_some() && self.open.is_some() {
            iced::time::every(std::time::Duration::from_millis(16)).map(|_| Msg::Frame)
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
        match m {
            Msg::Frame => {}
            Msg::Kit(r) => {
                if let Err(e) = &r {
                    log::warn!("note sprites: {e}");
                }
                self.kit = Some(r);
            }
            Msg::SpActive(on) => self.sp_active = on,
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

    pub fn view(&self) -> Element<'_, Msg> {
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
            Some(o) => self.editor(o),
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

    fn editor<'a>(&'a self, o: &'a Open) -> Element<'a, Msg> {
        let muted = |t: &Theme| text::Style { color: Some(crate::style::muted(t)) };
        let sec = self.section();
        let defaults = self.defaults.as_ref().expect("loaded when a profile is opened");
        let changed = o.profile != o.saved;

        let title = row![
            text(if changed { format!("{} •", o.name) } else { o.name.clone() }).size(16).font(crate::BOLD),
            space().width(Length::Fill),
            button(text("Save").size(13)).padding([6, 12]).style(button::primary).on_press_maybe((changed || o.path.is_none()).then_some(Msg::Save)),
            button(text("Undo changes").size(13)).padding([6, 12]).style(crate::style::secondary).on_press_maybe(changed.then_some(Msg::Discard)),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
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
            canvas_widget(Preview { p: &o.profile, d: defaults, section: sec, selected: self.selected.as_deref() }).width(Length::Fill).height(Length::Fill).into()
        };
        let preview: Element<'a, Msg> = if self.on_highway() {
            match (&self.kit, &self.highway, &self.demo) {
                (Some(Ok(_)), Some(tex), Some(demo)) => {
                    let track = if self.section == 0 { &demo.0 } else { &demo.1 };
                    let elapsed = self.started.map_or(0, |s| s.elapsed().as_millis() as u64);
                    let time_ms = 2000.0 + (elapsed as f64) % (crate::ch_sprites::DEMO_MS - 20_000.0);
                    let hw = canvas_widget(crate::preview::Highway { track, textures: tex, time_ms, anim_ms: elapsed }).width(Length::Fill).height(Length::Fill);
                    let stage = iced::widget::stack![
                        container(hw).width(Length::Fill).height(Length::Fill).style(stage_style(Color::BLACK)),
                        container(space()).width(Length::Fill).height(Length::Fill).style(stage_style(Color::TRANSPARENT)),
                    ];
                    column![
                        row![
                            text("Preview").size(12).style(muted).width(Length::Fill),
                            iced::widget::checkbox(self.sp_active).label("Star power active").size(14).text_size(12).on_toggle(Msg::SpActive),
                        ]
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
            column![text("Preview · click a part to edit it").size(12).style(muted), drawn()].spacing(6).into()
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
}

enum Shape {
    Circle(Point, f32),
    Rect(Rectangle, f32),
    Poly(Vec<Point>),
    Label(Point, String),
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
            Shape::Rect(r, _) => *r,
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
            Shape::Label(..) => return None,
        })
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
}

const HIGHWAY: Color = Color { r: 0.07, g: 0.07, b: 0.085, a: 1.0 };
const LANE_LINE: Color = Color { r: 0.2, g: 0.2, b: 0.24, a: 1.0 };

/// The highway's lanes: (left edge, lane width, centre of each lane).
fn lanes(size: Size, n: usize) -> (f32, f32, Vec<f32>) {
    let w = (size.width - 120.0).clamp(160.0, 70.0 * n as f32);
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
            "guitar" => guitar(&mut s, size),
            "drums" => drums(&mut s, size),
            "sixfret" => sixfret(&mut s, size),
            _ => other(&mut s, size),
        }
        s.items
    }
}

fn highway(s: &mut Scene, size: Size, n: usize) -> (f32, f32, Vec<f32>, f32) {
    let (x0, lw, xs) = lanes(size, n);
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
    s.label(Point::new(8.0, strike - 7.0), "Frets");
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
        let items = self.scene(bounds.size());
        for it in &items {
            if let Shape::Label(at, s) = &it.shape {
                f.fill_text(canvas::Text { content: s.clone(), position: *at, color: crate::style::muted(theme), size: 12.0.into(), ..Default::default() });
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
                if let Some(path) = it.shape.path() {
                    f.stroke(&path, Stroke::default().with_color(Color::BLACK.scale_alpha(0.7)).with_width(4.0));
                    f.stroke(&path, Stroke::default().with_color(Color::WHITE).with_width(2.0));
                }
            }
        }
        vec![f.into_geometry()]
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
        self.scene(size).into_iter().rev().find(|it| it.key.is_some() && it.shape.bounds().contains(p)).and_then(|it| it.key)
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
            let pv = Preview { p: &d, d: &d, section: sec, selected: None };
            let items = pv.scene(Size::new(520.0, 640.0));
            assert!(items.iter().any(|i| i.key.is_some()), "{sec}");
            for k in items.iter().filter_map(|i| i.key.as_deref()) {
                assert!(d.get(sec, k).is_some(), "{sec}: {k}");
            }
        }
        // A click on the green fret picks a green fret colour.
        let pv = Preview { p: &d, d: &d, section: "guitar", selected: None };
        let size = Size::new(520.0, 640.0);
        let (_, _, xs) = lanes(size, 5);
        assert!(pv.hit(size, Point::new(xs[0], size.height - 55.0)).is_some_and(|k| k.ends_with("_green")));
    }
}
