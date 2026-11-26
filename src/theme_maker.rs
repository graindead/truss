//! Tools > Themes: make your own themes, and export or import them as .json files.

use iced::theme::Palette;
use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::widget::{button, canvas as canvas_widget, column, container, pick_list, row, scrollable, slider, space, text, text_input};
use iced::{Alignment, Border, Color, Element, Length, Point, Rectangle, Renderer, Size, Task, Theme, mouse};
use serde::{Deserialize, Serialize};

/// A theme someone made, as saved in settings and in exported files. Colours are "#rrggbb".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomTheme {
    pub name: String,
    pub background: String,
    pub text: String,
    pub primary: String,
    pub success: String,
    pub warning: String,
    pub danger: String,
    /// Frosted glass see-through amount (0.1-1.0), 0 for a solid window, or none to follow
    /// Settings > Appearance > Transparency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transparency: Option<f32>,
}

impl CustomTheme {
    pub fn from_palette(name: &str, p: &Palette) -> Self {
        CustomTheme {
            name: name.to_string(),
            background: hex(p.background),
            text: hex(p.text),
            primary: hex(p.primary),
            success: hex(p.success),
            warning: hex(p.warning),
            danger: hex(p.danger),
            transparency: None,
        }
    }

    pub fn palette(&self) -> Palette {
        let c = |s: &str, fallback: Color| parse_hex(s).unwrap_or(fallback);
        Palette {
            background: c(&self.background, Color::from_rgb8(0x1b, 0x17, 0x15)),
            text: c(&self.text, Color::from_rgb8(0xef, 0xe6, 0xde)),
            primary: c(&self.primary, Color::from_rgb8(0xe2, 0x62, 0x2a)),
            success: c(&self.success, Color::from_rgb8(0x7c, 0xc4, 0x7f)),
            warning: c(&self.warning, Color::from_rgb8(0xf2, 0xb5, 0x45)),
            danger: c(&self.danger, Color::from_rgb8(0xe5, 0x48, 0x4d)),
        }
    }

    pub fn theme(&self) -> Theme {
        Theme::custom(self.name.clone(), self.palette())
    }

    fn get(&self, k: Slot) -> &str {
        match k {
            Slot::Background => &self.background,
            Slot::Text => &self.text,
            Slot::Primary => &self.primary,
            Slot::Success => &self.success,
            Slot::Warning => &self.warning,
            Slot::Danger => &self.danger,
        }
    }

    fn set(&mut self, k: Slot, v: String) {
        *match k {
            Slot::Background => &mut self.background,
            Slot::Text => &mut self.text,
            Slot::Primary => &mut self.primary,
            Slot::Success => &mut self.success,
            Slot::Warning => &mut self.warning,
            Slot::Danger => &mut self.danger,
        } = v;
    }

    fn valid(&self) -> bool {
        Slot::ALL.iter().all(|k| parse_hex(self.get(*k)).is_some())
    }
}

/// Reads a theme file: one theme, or a list of them.
pub fn parse_file(bytes: &[u8]) -> Result<Vec<CustomTheme>, String> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum File {
        One(CustomTheme),
        Many(Vec<CustomTheme>),
    }
    let list = match serde_json::from_slice::<File>(bytes).map_err(|_| "This isn't a Truss theme file.".to_string())? {
        File::One(t) => vec![t],
        File::Many(v) => v,
    };
    let list: Vec<CustomTheme> = list.into_iter().filter(|t| !t.name.trim().is_empty() && t.valid()).collect();
    if list.is_empty() {
        return Err("This theme file has no usable themes in it.".into());
    }
    Ok(list)
}

pub fn hex(c: Color) -> String {
    let [r, g, b, _] = c.into_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// A hex code typed into a colour picker: the colour, if it reads as one (the sliders follow it).
pub fn typed_hex(hsv: &mut (f32, f32, f32), hex: &mut String, typed: String) -> Option<Color> {
    let c = parse_hex(&typed);
    if let Some(c) = c {
        *hsv = to_hsv(c);
    }
    *hex = typed;
    c
}

/// A colour picker's sliders moved: the new colour (the hex code follows it).
pub fn moved_hsv(hsv: &mut (f32, f32, f32), hex: &mut String, (h, s, v): (f32, f32, f32)) -> Color {
    *hsv = (h, s, v);
    let c = from_hsv(h, s, v);
    *hex = crate::color_profile::ch_hex(c);
    c
}

pub fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    let s = match s.len() {
        3 => s.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => s.to_string(),
        _ => return None,
    };
    let v = u32::from_str_radix(&s, 16).ok()?;
    Some(Color::from_rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Hue (0-360), saturation and value (0-1).
pub(crate) fn to_hsv(c: Color) -> (f32, f32, f32) {
    let (r, g, b) = (c.r, c.g, c.b);
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let h = if d < 1e-6 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max < 1e-6 { 0.0 } else { d / max };
    (h, s, max)
}

pub(crate) fn from_hsv(h: f32, s: f32, v: f32) -> Color {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h.rem_euclid(360.0) / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color::from_rgb((r + m).clamp(0.0, 1.0), (g + m).clamp(0.0, 1.0), (b + m).clamp(0.0, 1.0))
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgb(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Background,
    Text,
    Primary,
    Success,
    Warning,
    Danger,
}

impl Slot {
    const ALL: [Slot; 6] = [Slot::Background, Slot::Text, Slot::Primary, Slot::Success, Slot::Warning, Slot::Danger];

    fn label(self) -> &'static str {
        match self {
            Slot::Background => "Background",
            Slot::Text => "Text",
            Slot::Primary => "Accent",
            Slot::Success => "Success",
            Slot::Warning => "Warning",
            Slot::Danger => "Error",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Slot::Background => "The window",
            Slot::Text => "Words, and mixed into panels and dividers",
            Slot::Primary => "Buttons, tabs, toggles, the icon",
            Slot::Success => "✓ marks and things that worked",
            Slot::Warning => "Notes and cautions",
            Slot::Danger => "Errors and Delete",
        }
    }
}

#[derive(Default)]
pub struct ThemeMaker {
    /// The theme being edited, and the saved name it came from (None for a new one).
    draft: Option<(CustomTheme, Option<String>)>,
    /// The colour whose picker is open.
    slot: Option<Slot>,
    /// Hue/saturation/value of the open colour, kept so hue isn't lost at grey or black.
    hsv: (f32, f32, f32),
    status: Option<(bool, String)>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    New,
    Edit(String),
    Name(String),
    Hex(Slot, String),
    Pick(Slot),
    Hsv(f32, f32, f32),
    Glass(Glass),
    GlassAmount(f32),
    Save { apply: bool },
    Cancel,
    Delete,
    Export,
    Exported(Result<Option<String>, String>),
    Import,
    Imported(Result<Option<Vec<CustomTheme>>, String>),
}

/// What the page asks of the app.
pub enum Out {
    None,
    /// Save (replacing `old_name` if given) and maybe switch to it.
    Save { theme: CustomTheme, old_name: Option<String>, apply: bool },
    Delete(String),
    Import(Vec<CustomTheme>),
}

impl ThemeMaker {
    /// `saved` is the custom themes in settings, `current` the theme in use, `taken` every
    /// built-in theme name.
    pub fn update(&mut self, m: Msg, saved: &[CustomTheme], current: &Theme, taken: &[String]) -> (Task<Msg>, Out) {
        match m {
            Msg::New => {
                let name = unique_name("My Theme", saved, taken, None);
                self.draft = Some((CustomTheme::from_palette(&name, &current.palette()), None));
                self.slot = None;
                self.status = None;
            }
            Msg::Edit(name) => {
                if let Some(t) = saved.iter().find(|t| t.name == name) {
                    self.draft = Some((t.clone(), Some(name)));
                    self.slot = None;
                    self.status = None;
                }
            }
            Msg::Name(n) => {
                if let Some((d, _)) = &mut self.draft {
                    d.name = n;
                }
            }
            Msg::Hex(k, v) => {
                if let Some((d, _)) = &mut self.draft {
                    if let Some(c) = parse_hex(&v)
                        && self.slot == Some(k)
                    {
                        self.hsv = to_hsv(c);
                    }
                    d.set(k, v);
                }
            }
            Msg::Pick(k) => {
                if self.slot == Some(k) {
                    self.slot = None;
                } else if let Some((d, _)) = &self.draft {
                    self.slot = Some(k);
                    self.hsv = parse_hex(d.get(k)).map(to_hsv).unwrap_or((0.0, 0.0, 0.5));
                }
            }
            Msg::Hsv(h, sat, v) => {
                self.hsv = (h, sat, v);
                if let (Some(k), Some((d, _))) = (self.slot, &mut self.draft) {
                    d.set(k, hex(from_hsv(h, sat, v)));
                }
            }
            Msg::Glass(g) => {
                if let Some((d, _)) = &mut self.draft {
                    d.transparency = match g {
                        Glass::Setting => None,
                        Glass::Solid => Some(0.0),
                        Glass::Frosted => Some(d.transparency.filter(|t| *t > 0.0).unwrap_or(0.5)),
                    };
                }
            }
            Msg::GlassAmount(v) => {
                if let Some((d, _)) = &mut self.draft {
                    d.transparency = Some(v.clamp(0.1, 1.0));
                }
            }
            Msg::Save { apply } => {
                let Some((d, old)) = &self.draft else { return (Task::none(), Out::None) };
                if !d.valid() {
                    self.status = Some((false, "Every colour needs to be a hex code like #1b1715.".into()));
                    return (Task::none(), Out::None);
                }
                let mut t = d.clone();
                let base = if t.name.trim().is_empty() { "My Theme".to_string() } else { t.name.trim().to_string() };
                t.name = unique_name(&base, saved, taken, old.as_deref());
                for k in Slot::ALL {
                    let v = hex(parse_hex(t.get(k)).unwrap_or(Color::BLACK));
                    t.set(k, v);
                }
                let old = old.clone();
                self.draft = Some((t.clone(), Some(t.name.clone())));
                self.status = Some((true, if apply { format!("Saved and switched to {}.", t.name) } else { format!("Saved {}.", t.name) }));
                return (Task::none(), Out::Save { theme: t, old_name: old, apply });
            }
            Msg::Cancel => {
                self.draft = None;
                self.slot = None;
                self.status = None;
            }
            Msg::Delete => {
                if let Some((_, Some(name))) = self.draft.take() {
                    self.slot = None;
                    self.status = Some((true, format!("Deleted {name}.")));
                    return (Task::none(), Out::Delete(name));
                }
                self.draft = None;
            }
            Msg::Export => {
                let Some((d, _)) = &self.draft else { return (Task::none(), Out::None) };
                if !d.valid() {
                    self.status = Some((false, "Fix the colours before exporting.".into()));
                    return (Task::none(), Out::None);
                }
                let t = d.clone();
                let json = serde_json::to_string_pretty(&t).unwrap_or_default();
                let file_name = format!("{}.truss-theme.json", sanitize(&t.name));
                return (
                    Task::perform(
                        async move {
                            let Some(f) = rfd::AsyncFileDialog::new()
                                .set_title("Export theme")
                                .set_file_name(file_name)
                                .add_filter("Truss theme", &["json"])
                                .save_file()
                                .await
                            else {
                                return Ok(None);
                            };
                            let path = f.path().to_path_buf();
                            tokio::fs::write(&path, json).await.map_err(|e| e.to_string())?;
                            Ok(Some(path.display().to_string()))
                        },
                        Msg::Exported,
                    ),
                    Out::None,
                );
            }
            Msg::Exported(r) => {
                self.status = match r {
                    Ok(Some(p)) => Some((true, format!("Exported to {p}"))),
                    Ok(None) => None,
                    Err(e) => Some((false, format!("Couldn't export: {e}"))),
                };
            }
            Msg::Import => {
                return (
                    Task::perform(
                        async {
                            let Some(files) = rfd::AsyncFileDialog::new()
                                .set_title("Import themes")
                                .add_filter("Truss theme", &["json"])
                                .pick_files()
                                .await
                            else {
                                return Ok(None);
                            };
                            let mut all = Vec::new();
                            for f in files {
                                all.extend(parse_file(&f.read().await)?);
                            }
                            Ok(Some(all))
                        },
                        Msg::Imported,
                    ),
                    Out::None,
                );
            }
            Msg::Imported(r) => match r {
                Ok(Some(list)) => {
                    // Names that clash get a number, both with each other and with what's saved.
                    let mut names: Vec<CustomTheme> = saved.to_vec();
                    let mut added = Vec::new();
                    for mut t in list {
                        t.name = unique_name(t.name.trim(), &names, taken, None);
                        names.push(t.clone());
                        added.push(t);
                    }
                    self.status = Some((
                        true,
                        match added.as_slice() {
                            [one] => format!("Imported {}.", one.name),
                            many => format!("Imported {} themes.", many.len()),
                        },
                    ));
                    if let Some(first) = added.first() {
                        self.draft = Some((first.clone(), Some(first.name.clone())));
                        self.slot = None;
                    }
                    return (Task::none(), Out::Import(added));
                }
                Ok(None) => {}
                Err(e) => self.status = Some((false, e)),
            },
        }
        (Task::none(), Out::None)
    }

    /// `glass_window`: whether this window was opened see-through (frosted glass can only be
    /// turned on or off with a restart).
    pub fn view<'a>(&'a self, saved: &'a [CustomTheme], current: &Theme, glass_window: bool) -> Element<'a, Msg> {
        let muted = crate::style::muted_text;

        // Left: your themes.
        let mut list = column![text("Your themes").size(15).font(crate::BOLD)].spacing(6);
        if saved.is_empty() {
            list = list.push(text("None yet. Make one, or import a theme file someone shared.").size(12).style(muted));
        }
        let editing = self.draft.as_ref().and_then(|(_, o)| o.clone());
        for t in saved {
            let p = t.palette();
            let dots = row(
                [p.background, p.primary, p.text, p.success, p.danger]
                    .into_iter()
                    .map(|c| swatch(c, 12.0).into()),
            )
            .spacing(3);
            let active = current.to_string() == t.name;
            let label = if active { format!("{}  (in use)", t.name) } else { t.name.clone() };
            list = list.push(
                button(row![dots, text(label).size(13)].spacing(10).align_y(Alignment::Center))
                    .width(Length::Fill)
                    .padding([6, 10])
                    .style(crate::style::quiet_button(editing.as_deref() == Some(t.name.as_str())))
                    .on_press(Msg::Edit(t.name.clone())),
            );
        }
        list = list.push(space().height(6)).push(
            row![
                button(text("New theme").size(13)).padding([6, 12]).style(button::primary).on_press(Msg::New),
                button(text("Import…").size(13)).padding([6, 12]).style(crate::style::secondary).on_press(Msg::Import),
            ]
            .spacing(8),
        );
        list = list.push(
            text("New themes start from the one you're using. Exported themes are small .json files you can share.")
                .size(12)
                .style(muted),
        );
        let left = container(scrollable(list.padding(iced::Padding { right: 10.0, ..Default::default() })))
            .width(300)
            .height(Length::Fill);

        // Right: the editor and preview.
        let right: Element<'a, Msg> = match &self.draft {
            None => container(text("Pick a theme on the left to edit it, or make a new one.").size(13).style(muted))
                .center(Length::Fill)
                .into(),
            Some((d, old)) => self.editor(d, old.is_some(), glass_window),
        };

        crate::style::list_and_editor(left.into(), right, &self.status)
    }

    fn editor<'a>(&'a self, d: &'a CustomTheme, saved_before: bool, glass_window: bool) -> Element<'a, Msg> {
        let muted = crate::style::muted_text;
        let mut colours = column![].spacing(4);
        for k in Slot::ALL {
            let value = d.get(k);
            let ok = parse_hex(value).is_some();
            let c = parse_hex(value).unwrap_or(Color::TRANSPARENT);
            let open = self.slot == Some(k);
            let head = row![
                button(swatch(c, 28.0).style(move |t: &Theme| container::Style {
                    background: Some(c.into()),
                    border: Border {
                        color: if open { t.palette().text } else { crate::style::hairline(t) },
                        width: if open { 2.0 } else { 1.0 },
                        radius: 7.0.into(),
                    },
                    ..Default::default()
                }))
                .padding(0)
                .style(button::text)
                .on_press(Msg::Pick(k)),
                column![text(k.label()).size(13), text(k.hint()).size(11).style(muted)].spacing(1).width(Length::Fill),
                text_input("#rrggbb", value)
                    .on_input(move |v| Msg::Hex(k, v))
                    .size(13)
                    .width(100)
                    .style(move |t: &Theme, s| {
                        let mut st = text_input::default(t, s);
                        if !ok {
                            st.border.color = t.palette().danger;
                        }
                        st
                    }),
            ]
            .spacing(10)
            .align_y(Alignment::Center);
            let mut block = column![head].spacing(8);
            if open {
                let (h, sat, v) = self.hsv;
                block = block.push(
                    container(canvas_widget(Picker { h, s: sat, v, on: Msg::Hsv }).width(Length::Fill).height(150))
                        .padding(iced::Padding { left: 38.0, right: 0.0, top: 0.0, bottom: 6.0 }),
                );
            }
            colours = colours.push(block);
        }

        let glass = Glass::of(d.transparency);
        let mut window = row![
            text("Window").size(13).width(60),
            pick_list(Glass::ALL, Some(glass), Msg::Glass).text_size(13),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if let Some(t) = d.transparency.filter(|t| *t > 0.0) {
            window = window
                .push(slider(0.1..=1.0, t, Msg::GlassAmount).step(0.05_f32).width(140))
                .push(text(format!("{:.0}% see-through", t * 100.0)).size(12).wrapping(text::Wrapping::None));
        }
        let glass_note = match glass {
            Glass::Frosted if !glass_window => Some("Frosted glass shows after Truss restarts."),
            Glass::Solid if glass_window => Some("Fully solid now; frosted glass switches off when Truss restarts."),
            Glass::Setting => Some("Uses Settings > Appearance > Transparency."),
            _ => None,
        };

        let mut actions = row![
            button(text("Save and use").size(13)).padding([6, 12]).style(button::primary).on_press(Msg::Save { apply: true }),
            button(text("Save").size(13)).padding([6, 12]).style(crate::style::secondary).on_press(Msg::Save { apply: false }),
            button(text("Export…").size(13)).padding([6, 12]).style(crate::style::secondary).on_press(Msg::Export),
            space().width(Length::Fill),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if saved_before {
            actions = actions.push(button(text("Delete").size(13)).padding([6, 12]).style(button::danger).on_press(Msg::Delete));
        }
        actions = actions.push(button(text("Close").size(13)).padding([6, 12]).style(crate::style::subtle).on_press(Msg::Cancel));

        let editor = column![
            row![text("Name").size(13).width(60), text_input("My Theme", &d.name).on_input(Msg::Name).size(14)]
                .spacing(10)
                .align_y(Alignment::Center),
            colours,
            column![window].push(glass_note.map(|n| text(n).size(11).style(muted))).spacing(4),
            actions,
        ]
        .spacing(14)
        .width(Length::FillPortion(5));

        let preview = column![text("Preview").size(13).style(muted), preview(&d.palette())].spacing(6).width(Length::FillPortion(4));
        scrollable(row![editor, preview].spacing(20).padding(iced::Padding { right: 12.0, ..Default::default() })).into()
    }
}

/// Whether a theme sets the window's transparency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glass {
    Setting,
    Solid,
    Frosted,
}

impl Glass {
    const ALL: [Glass; 3] = [Glass::Setting, Glass::Solid, Glass::Frosted];

    fn of(t: Option<f32>) -> Self {
        match t {
            None => Glass::Setting,
            Some(t) if t > 0.0 => Glass::Frosted,
            Some(_) => Glass::Solid,
        }
    }
}

impl std::fmt::Display for Glass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Glass::Setting => "Use my transparency setting",
            Glass::Solid => "Solid",
            Glass::Frosted => "Frosted glass",
        })
    }
}

/// The colour picker: saturation across and brightness down a square, hue in a bar beside it.
/// `on` turns a picked hue/saturation/value into the page's message.
pub(crate) struct Picker<M> {
    pub h: f32,
    pub s: f32,
    pub v: f32,
    pub on: fn(f32, f32, f32) -> M,
}

const HUE_W: f32 = 22.0;
const GAP: f32 = 12.0;

#[derive(Default, Clone, Copy, PartialEq)]
pub(crate) enum Drag {
    #[default]
    None,
    Square,
    Hue,
}

impl<M> Picker<M> {
    fn square_w(bounds: Rectangle) -> f32 {
        (bounds.width - HUE_W - GAP).max(10.0)
    }

    fn pick(&self, part: Drag, p: Point, bounds: Rectangle) -> Option<M> {
        let sw = Self::square_w(bounds);
        let x = (p.x / sw).clamp(0.0, 1.0);
        let y = (p.y / bounds.height).clamp(0.0, 1.0);
        match part {
            Drag::Square => Some((self.on)(self.h, x, 1.0 - y)),
            Drag::Hue => Some((self.on)((y * 360.0).min(359.9), self.s, self.v)),
            Drag::None => None,
        }
    }
}

impl<M> canvas::Program<M> for Picker<M> {
    type State = Drag;

    fn update(&self, drag: &mut Drag, event: &canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<M>> {
        let rel = cursor.position().map(|p| Point::new(p.x - bounds.x, p.y - bounds.y));
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_in(bounds)?;
                let sw = Self::square_w(bounds);
                *drag = if p.x <= sw {
                    Drag::Square
                } else if p.x >= sw + GAP / 2.0 {
                    Drag::Hue
                } else {
                    Drag::None
                };
                self.pick(*drag, p, bounds).map(|m| canvas::Action::publish(m).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if *drag != Drag::None => {
                self.pick(*drag, rel?, bounds).map(|m| canvas::Action::publish(m).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if *drag != Drag::None => {
                *drag = Drag::None;
                Some(canvas::Action::capture())
            }
            _ => None,
        }
    }

    fn draw(&self, _drag: &Drag, renderer: &Renderer, theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<canvas::Geometry> {
        use canvas::gradient::Linear;
        let mut f = Frame::new(renderer, bounds.size());
        let sw = Self::square_w(bounds);
        let h = bounds.height;
        let fill = |lin: Linear| canvas::Fill { style: canvas::Style::Gradient(lin.into()), ..Default::default() };
        // Saturation: white to the pure hue; brightness: a layer fading to black.
        let square = Path::rounded_rectangle(Point::ORIGIN, Size::new(sw, h), 6.0.into());
        f.fill(
            &square,
            fill(Linear::new(Point::new(0.0, 0.0), Point::new(sw, 0.0)).add_stop(0.0, Color::WHITE).add_stop(1.0, from_hsv(self.h, 1.0, 1.0))),
        );
        f.fill(&square, fill(Linear::new(Point::new(0.0, 0.0), Point::new(0.0, h)).add_stop(0.0, Color::TRANSPARENT).add_stop(1.0, Color::BLACK)));
        f.stroke(&square, Stroke::default().with_color(crate::style::hairline(theme)).with_width(1.0));
        // Hue bar.
        let x = sw + GAP;
        let bar = Path::rounded_rectangle(Point::new(x, 0.0), Size::new(HUE_W, h), 6.0.into());
        let mut lin = Linear::new(Point::new(0.0, 0.0), Point::new(0.0, h));
        for i in 0..=6 {
            lin = lin.add_stop(i as f32 / 6.0, from_hsv(i as f32 * 60.0 % 360.0, 1.0, 1.0));
        }
        f.fill(&bar, fill(lin));
        f.stroke(&bar, Stroke::default().with_color(crate::style::hairline(theme)).with_width(1.0));
        // Markers.
        let at = Point::new(self.s * sw, (1.0 - self.v) * h);
        let ring = Path::circle(at, 7.0);
        f.stroke(&ring, Stroke::default().with_color(Color::BLACK.scale_alpha(0.6)).with_width(4.0));
        f.stroke(&ring, Stroke::default().with_color(Color::WHITE).with_width(2.0));
        let hy = (self.h / 360.0 * h).clamp(2.0, h - 2.0);
        let knob = Path::rounded_rectangle(Point::new(x - 2.0, hy - 3.0), Size::new(HUE_W + 4.0, 6.0), 3.0.into());
        f.stroke(&knob, Stroke::default().with_color(Color::BLACK.scale_alpha(0.6)).with_width(3.0));
        f.stroke(&knob, Stroke::default().with_color(Color::WHITE).with_width(1.5));
        vec![f.into_geometry()]
    }

    fn mouse_interaction(&self, drag: &Drag, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if *drag != Drag::None || cursor.is_over(bounds) { mouse::Interaction::Crosshair } else { mouse::Interaction::default() }
    }
}

/// A name no other theme has: `base`, or `base 2`, `base 3`…
fn unique_name(base: &str, saved: &[CustomTheme], taken: &[String], keep: Option<&str>) -> String {
    first_free(base, |n| {
        Some(n) == keep || (!taken.iter().any(|t| t.eq_ignore_ascii_case(n)) && !saved.iter().any(|t| t.name.eq_ignore_ascii_case(n)))
    })
}

/// `base` if `free` says it's free, else the first free one of `base 2`, `base 3`…
pub(crate) fn first_free(base: &str, free: impl Fn(&str) -> bool) -> String {
    if free(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| free(n)).unwrap()
}

fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    let s = s.trim().to_string();
    if s.is_empty() { "theme".into() } else { s }
}

pub(crate) fn swatch<'a, M: 'a>(c: Color, size: f32) -> container::Container<'a, M> {
    container(space()).width(size).height(size).style(move |t: &Theme| container::Style {
        background: Some(c.into()),
        border: Border { color: crate::style::hairline(t), width: 1.0, radius: (size / 4.0).into() },
        ..Default::default()
    })
}

/// A small mock-up of Truss in a palette: top bar, a few results, buttons and notices.
fn preview<'a, M: 'a>(p: &Palette) -> Element<'a, M> {
    let p = *p;
    let dark = p.background.relative_luminance() < 0.5;
    let raised = mix(p.background, p.text, if dark { 0.07 } else { 0.04 });
    let line = mix(p.background, p.text, 0.14);
    let muted = mix(p.background, p.text, 0.6);
    let on_primary = if p.primary.relative_luminance() > 0.45 { Color::BLACK } else { Color::WHITE };
    let txt = move |s: &'static str, size: u32, c: Color| text(s).size(size).color(c);
    let pill = move |s: &'static str, bg: Color, fg: Color| {
        container(text(s).size(11).color(fg)).padding([3, 9]).style(move |_: &Theme| container::Style {
            background: Some(bg.into()),
            border: Border { radius: 5.0.into(), ..Default::default() },
            ..Default::default()
        })
    };
    let tab = move |s: &'static str, active: bool| {
        column![
            txt(s, 12, if active { p.text } else { muted }),
            container(space()).width(Length::Fill).height(2).style(move |_: &Theme| container::Style {
                background: active.then(|| p.primary.into()),
                ..Default::default()
            }),
        ]
        .spacing(3)
        .width(Length::Shrink)
    };
    let result = move |title: &'static str, artist: &'static str, owned: bool, selected: bool| {
        container(
            row![
                container(space()).width(22).height(22).style(move |_: &Theme| container::Style {
                    background: Some(mix(p.primary, p.background, 0.45).into()),
                    border: Border { radius: 3.0.into(), ..Default::default() },
                    ..Default::default()
                }),
                if owned { txt("✓", 12, p.success) } else { txt(" ", 12, p.text) },
                txt(title, 12, p.text).width(Length::FillPortion(3)),
                txt(artist, 12, muted).width(Length::FillPortion(2)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([5, 8])
        .style(move |_: &Theme| container::Style {
            background: selected.then(|| mix(p.background, p.primary, 0.18).into()),
            border: Border { radius: 5.0.into(), ..Default::default() },
            ..Default::default()
        })
    };
    let bar = column![
        row![
            tab("Browse", true),
            tab("Library", false),
            tab("Tools", false),
            space().width(Length::Fill),
            container(txt("Search…", 11, muted)).padding([3, 10]).width(110).style(move |_: &Theme| container::Style {
                background: Some(raised.into()),
                border: Border { color: line, width: 1.0, radius: 5.0.into() },
                ..Default::default()
            }),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
        container(space()).width(Length::Fill).height(1).style(move |_: &Theme| container::Style { background: Some(line.into()), ..Default::default() }),
        result("Through the Fire", "Some Band", false, false),
        result("Second Song", "Another Artist", true, true),
        result("Third Song", "Someone Else", false, false),
        container(space()).width(Length::Fill).height(1).style(move |_: &Theme| container::Style { background: Some(line.into()), ..Default::default() }),
        row![pill("Download", p.primary, on_primary), pill("Preview", raised, p.text), space().width(Length::Fill), txt("Tier 7", 12, p.warning)]
            .spacing(8)
            .align_y(Alignment::Center),
        container(
            row![txt("Downloaded 12 charts", 12, p.text), space().width(Length::Fill), txt("1 failed", 12, p.danger)].align_y(Alignment::Center)
        )
        .padding([6, 10])
        .style(move |_: &Theme| container::Style {
            background: Some(raised.into()),
            border: Border { color: line, width: 1.0, radius: 8.0.into() },
            ..Default::default()
        }),
    ]
    .spacing(8);
    container(bar)
        .padding(12)
        .width(Length::Fill)
        .style(move |_: &Theme| container::Style {
            background: Some(p.background.into()),
            border: Border { color: line, width: 1.0, radius: 10.0.into() },
            ..Default::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        assert_eq!(hex(parse_hex("#1B1715").unwrap()), "#1b1715");
        assert_eq!(hex(parse_hex("fff").unwrap()), "#ffffff");
        assert!(parse_hex("#12345").is_none());
        for h in ["#e2622a", "#3b8ef0", "#7cc47f", "#000000", "#ffffff", "#808080"] {
            let (a, b, c) = to_hsv(parse_hex(h).unwrap());
            assert_eq!(hex(from_hsv(a, b, c)), h);
        }
    }

    #[test]
    fn files() {
        let t = CustomTheme::from_palette("Mine", &crate::themes::rust_theme().palette());
        let one = serde_json::to_vec(&t).unwrap();
        assert_eq!(parse_file(&one).unwrap(), vec![t.clone()]);
        let many = serde_json::to_vec(&vec![t.clone(), t.clone()]).unwrap();
        assert_eq!(parse_file(&many).unwrap().len(), 2);
        assert!(parse_file(b"{\"hello\":1}").is_err());
        assert_eq!(unique_name("Rust", &[], &["Rust".into()], None), "Rust 2");
    }
}
