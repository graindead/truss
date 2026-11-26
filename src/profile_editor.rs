//! Tools > Profile Editor: edits Clone Hero's profiles.ini (each player's name, colour profile,
//! note speed and the rest). The file is edited line by line, so its order and any keys Truss
//! doesn't know about stay as they were, and a backup is made before each save. Clone Hero
//! writes the file itself when it closes, so saving waits until the game isn't running.

use crate::color_profile::{Profile, ch_hex};
use crate::theme_maker::{Picker, PickerInput, parse_hex, to_hsv};
use iced::widget::{button, canvas, checkbox, column, container, pick_list, row, scrollable, space, text, text_input};
use iced::{Alignment, Color, Element, Length, Task, Theme};
use std::path::PathBuf;

/// Settings with their own control (the rest are listed under "Other settings" as they are).
/// (key, label, what it is)
const SWITCHES: [(&str, &str, &str); 2] = [
    ("lefty_flip", "Lefty flip", "The highway is mirrored for left-handed players."),
    ("show_accuracy_display", "Show accuracy", "Accuracy is shown while playing."),
];
const NUMBERS: [(&str, &str, u32, u32); 2] = [("note_speed", "Note speed", 1, 100), ("highway_length", "Highway length (%)", 20, 1000)];
/// Shown by their own controls, or not settings at all.
const OWN: [&str; 7] = ["player_name", "color_profile_name", "highway_name", "lefty_flip", "show_accuracy_display", "note_speed", "highway_length"];

/// Kept apart and folded away, as few players need them: electronic drum kits plugged in over
/// MIDI, and the controller type Clone Hero sets itself.
const DRUM_KIT: [&str; 5] = ["midi_device_id", "midi_device_name", "dynamics_threshold", "default_sample_velocity", "controller_type"];

/// Settings that are on (1) or off (0), shown as tick boxes.
const ON_OFF: [&str; 8] = ["tilt_activation", "alt_taps", "square_tom_notes", "drum_dynamics_hidden", "show_displayname", "is_bot", "gamepad_mode", "color_shuffle"];

/// A tick box for an on/off setting, if it holds 0 or 1 (anything else is left as text).
fn is_switch(key: &str, value: &str) -> bool {
    ON_OFF.contains(&key) && matches!(value, "0" | "1")
}

/// A setting's name to show: plain words for the ones known, else the key made readable
/// ("default_sample_velocity" -> "Default sample velocity").
fn label_for(key: &str) -> String {
    let known = match key {
        "dynamics_threshold" => "Dynamics threshold",
        "midi_device_id" => "MIDI device ID",
        "midi_device_name" => "MIDI device name",
        "highway_name" => "Highway",
        "tilt_activation" => "Tilt activation",
        "alt_taps" => "Alternate taps",
        "square_tom_notes" => "Square tom notes",
        "drum_dynamics_hidden" => "Hide drum dynamics",
        "show_displayname" => "Show display name",
        "is_bot" => "Bot",
        "gamepad_mode" => "Gamepad mode",
        "controller_type" => "Controller type",
        "color_shuffle" => "Color shuffle",
        "default_sample_velocity" => "Default sample velocity",
        _ => "",
    };
    if !known.is_empty() {
        return known.into();
    }
    let words = key.replace('_', " ");
    let mut c = words.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
}

/// How the name is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Plain,
    One,
    Gradient,
    /// The name with tags written by hand (or kept from a style Truss can't make itself).
    Tags,
}

impl std::fmt::Display for Style {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Style::Plain => "No colour",
            Style::One => "One colour",
            Style::Gradient => "Gradient",
            Style::Tags => "Tags by hand",
        })
    }
}

/// The colour being picked: the name's, or a gradient's start or end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    From,
    To,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Pick(usize),
    Name(String),
    Style(Style),
    Tags(String),
    Bold(bool),
    Italic(bool),
    Which(Which),
    Colour(PickerInput),
    ColorProfile(String),
    Highway(String),
    Switch(&'static str, bool),
    Number(&'static str, String),
    Other(String, String),
    /// Show or hide the drum kit settings.
    DrumKit,
    Save,
    Discard,
    Reload,
}

/// The name as Clone Hero stores it: rich-text tags around the letters.
pub fn styled_name(name: &str, style: Style, from: Color, to: Color, bold: bool, italic: bool) -> String {
    let coloured = match style {
        Style::Plain | Style::Tags => name.to_string(),
        Style::One => format!("<color={}>{name}</color>", ch_hex(from)),
        Style::Gradient => {
            // A colour per letter, from the first to the last (spaces don't need one).
            let letters: Vec<char> = name.chars().collect();
            let steps = letters.iter().filter(|c| !c.is_whitespace()).count().saturating_sub(1).max(1) as f32;
            let mut n = 0.0;
            letters
                .iter()
                .map(|c| {
                    if c.is_whitespace() {
                        return c.to_string();
                    }
                    let t = n / steps;
                    n += 1.0;
                    let mix = Color::from_rgb(from.r + (to.r - from.r) * t, from.g + (to.g - from.g) * t, from.b + (to.b - from.b) * t);
                    format!("<color={}>{c}</color>", ch_hex(mix))
                })
                .collect()
        }
    };
    let coloured = if bold { format!("<b>{coloured}</b>") } else { coloured };
    if italic { format!("<i>{coloured}</i>") } else { coloured }
}

/// What a stored name was made with, if Truss could have made it: (name, style, colours, bold,
/// italic). Anything else is shown as tags.
fn read_name(stored: &str) -> (String, Style, Color, Color, bool, bool) {
    let white = Color::WHITE;
    let plain = crate::api::strip_tags(stored);
    let pieces = crate::rich::parse(stored);
    let bold = !pieces.is_empty() && pieces.iter().all(|p| p.bold);
    let italic = !pieces.is_empty() && pieces.iter().all(|p| p.italic);
    // Remade from what was read, it must come out the same (each colour within a shade, as
    // picked colours are rounded when written), or it's kept as it is.
    let try_style = |style: Style, from: Color, to: Color| same_but_shades(&styled_name(&plain, style, from, to, bold, italic), stored).then_some((style, from, to));
    let hex_of = |s: &str| -> Vec<Color> {
        s.match_indices("<color=").filter_map(|(i, _)| parse_hex(s[i + 7..].get(..7)?)).collect()
    };
    let found = hex_of(stored);
    let guess = [
        try_style(Style::Plain, white, white),
        found.first().and_then(|c| try_style(Style::One, *c, *c)),
        found.first().zip(found.last()).and_then(|(a, b)| try_style(Style::Gradient, *a, *b)),
    ];
    match guess.into_iter().flatten().next() {
        Some((style, from, to)) => (plain, style, from, to, bold, italic),
        None => (plain, Style::Tags, white, white, false, false),
    }
}

/// Whether two tagged names are the same, but for colours a shade or two apart.
fn same_but_shades(a: &str, b: &str) -> bool {
    // The text with every colour taken out, and the colours.
    fn split(s: &str) -> (String, Vec<[u8; 3]>) {
        let (mut text, mut colours, mut rest) = (String::new(), Vec::new(), s);
        while let Some(i) = rest.find("<color=#") {
            text.push_str(&rest[..i + 8]);
            let hex = rest.get(i + 8..i + 14).unwrap_or("");
            match u32::from_str_radix(hex, 16) {
                Ok(v) if hex.len() == 6 => {
                    colours.push([(v >> 16) as u8, (v >> 8) as u8, v as u8]);
                    rest = &rest[i + 14..];
                }
                _ => rest = &rest[i + 8..],
            }
        }
        text.push_str(rest);
        (text, colours)
    }
    let ((ta, ca), (tb, cb)) = (split(a), split(b));
    ta == tb && ca.len() == cb.len() && ca.iter().zip(&cb).all(|(x, y)| x.iter().zip(y).all(|(p, q)| p.abs_diff(*q) <= 2))
}

/// Clone Hero is running (it would write over the file when it closes).
fn game_running() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up.
        const NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq Clone Hero.exe", "/NH", "/FO", "CSV"])
            .creation_flags(NO_WINDOW)
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains("clone hero.exe"))
    }
    #[cfg(not(windows))]
    false
}

/// The file: Clone Hero's, or a stand-in for testing (TRUSS_TEST_PROFILES).
fn file() -> Option<PathBuf> {
    match std::env::var_os("TRUSS_TEST_PROFILES") {
        Some(p) => Some(PathBuf::from(p)),
        None => crate::ch_sprites::profiles_file(),
    }
}

/// Writes the profiles: the old file kept as profiles.ini.bak first, the new one written beside
/// it and then put in its place, so a failed save never leaves half a file.
fn write(path: &std::path::Path, text: &str) -> Result<(), String> {
    if path.exists() {
        std::fs::copy(path, path.with_extension("ini.bak")).map_err(|e| format!("Couldn't make a backup: {e}"))?;
    }
    let tmp = path.with_extension("ini.truss-tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

#[derive(Default)]
pub struct ProfileEditor {
    path: Option<PathBuf>,
    /// As read, and as edited.
    saved: Option<Profile>,
    edited: Option<Profile>,
    bom: bool,
    /// The profiles' sections, and the one open.
    sections: Vec<String>,
    open: usize,
    // The name, while it's edited.
    name: String,
    style: Option<Style>,
    tags: String,
    from: Color,
    to: Color,
    bold: bool,
    italic: bool,
    which: Option<Which>,
    hex: String,
    hsv: (f32, f32, f32),
    /// What happened last (good, message).
    status: Option<(bool, String)>,
    colour_profiles: Vec<String>,
    /// The pictures in Clone Hero's Custom\Highways folder.
    highways: Vec<String>,
    /// The drum kit (MIDI) settings are shown.
    drum_kit: bool,
}

impl ProfileEditor {
    /// Reads the file again (the tab was opened, or Reload).
    pub fn opened(&mut self) {
        self.path = file();
        self.status = None;
        let text = self.path.as_ref().and_then(|p| std::fs::read(p).ok());
        let Some(bytes) = text else {
            self.saved = None;
            self.edited = None;
            return;
        };
        self.bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
        let p = Profile::parse(&String::from_utf8_lossy(&bytes));
        let mut sections: Vec<String> = p.sections().into_iter().filter(|s| s.to_ascii_lowercase().starts_with("profile")).collect();
        sections.sort_by_key(|s| s[7..].parse::<u32>().unwrap_or(u32::MAX));
        self.sections = sections;
        self.saved = Some(p.clone());
        self.edited = Some(p);
        self.colour_profiles = crate::color_profile::folder().map(|d| crate::color_profile::list(&d)).unwrap_or_default();
        self.highways = crate::ch_sprites::custom_highways();
        self.open = self.open.min(self.sections.len().saturating_sub(1));
        self.load_name();
    }

    fn section(&self) -> Option<&str> {
        self.sections.get(self.open).map(String::as_str)
    }

    fn get(&self, key: &str) -> String {
        match (&self.edited, self.section()) {
            (Some(p), Some(s)) => p.get(s, key).unwrap_or("").to_string(),
            _ => String::new(),
        }
    }

    fn set(&mut self, key: &str, value: &str) {
        let Some(s) = self.section().map(String::from) else { return };
        if let Some(p) = &mut self.edited {
            p.set(&s, key, value);
        }
    }

    /// The name controls, from the open profile's name.
    fn load_name(&mut self) {
        let stored = self.get("player_name");
        let (name, style, from, to, bold, italic) = read_name(&stored);
        self.name = name;
        self.style = Some(style);
        self.tags = stored;
        (self.from, self.to, self.bold, self.italic) = (from, to, bold, italic);
        self.which = matches!(style, Style::One | Style::Gradient).then_some(Which::From);
        self.load_colour();
    }

    fn load_colour(&mut self) {
        let c = if self.which == Some(Which::To) { self.to } else { self.from };
        self.hex = ch_hex(c);
        self.hsv = to_hsv(c);
    }

    /// Writes the name controls back into the profile.
    fn store_name(&mut self) {
        let style = self.style.unwrap_or(Style::Plain);
        let stored = if style == Style::Tags { self.tags.clone() } else { styled_name(&self.name, style, self.from, self.to, self.bold, self.italic) };
        if style != Style::Tags {
            self.tags = stored.clone();
        }
        self.set("player_name", &stored);
    }

    fn set_colour(&mut self, c: Color) {
        // As it will be written, so a gradient's colours read back the same.
        let c = parse_hex(&ch_hex(c)).unwrap_or(c);
        match self.which {
            Some(Which::To) => self.to = c,
            _ => self.from = c,
        }
        self.store_name();
    }

    fn changed(&self) -> bool {
        self.edited != self.saved
    }

    pub fn update(&mut self, m: Msg) -> Task<Msg> {
        match m {
            Msg::Pick(i) => {
                self.open = i.min(self.sections.len().saturating_sub(1));
                self.load_name();
            }
            Msg::Name(n) => {
                self.name = n;
                self.store_name();
            }
            Msg::Style(s) => {
                // Starting a colour off white would look like nothing happened.
                if s != Style::Plain && s != Style::Tags && self.from == Color::WHITE && self.to == Color::WHITE {
                    self.from = Color::from_rgb8(0x3b, 0x8e, 0xf0);
                    self.to = Color::from_rgb8(0xb6, 0x7c, 0xf2);
                }
                self.style = Some(s);
                self.which = matches!(s, Style::One | Style::Gradient).then_some(Which::From);
                self.load_colour();
                self.store_name();
            }
            Msg::Tags(t) => {
                self.tags = t;
                self.name = crate::api::strip_tags(&self.tags);
                self.store_name();
            }
            Msg::Bold(b) => {
                self.bold = b;
                self.store_name();
            }
            Msg::Italic(b) => {
                self.italic = b;
                self.store_name();
            }
            Msg::Which(w) => {
                self.which = Some(w);
                self.load_colour();
            }
            Msg::Colour(input) => {
                if let Some(c) = crate::theme_maker::pick(&mut self.hsv, &mut self.hex, input) {
                    self.set_colour(c);
                }
            }
            Msg::ColorProfile(n) => self.set("color_profile_name", &n),
            Msg::Highway(n) => self.set("highway_name", &n),
            Msg::Switch(k, on) => self.set(k, if on { "1" } else { "0" }),
            Msg::Number(k, v) => {
                let v: String = v.chars().filter(char::is_ascii_digit).take(4).collect();
                self.set(k, &v);
            }
            Msg::Other(k, v) => self.set(&k, &v),
            Msg::Discard => {
                self.edited = self.saved.clone();
                self.load_name();
                self.status = None;
            }
            Msg::Reload => self.opened(),
            Msg::DrumKit => self.drum_kit = !self.drum_kit,
            Msg::Save => self.save(),
        }
        Task::none()
    }

    fn save(&mut self) {
        let (Some(path), Some(p)) = (self.path.clone(), self.edited.clone()) else { return };
        if self.name.trim().is_empty() {
            self.status = Some((false, "The name can't be empty.".into()));
            return;
        }
        let keys = self.section().and_then(|sec| Some(self.edited.as_ref()?.keys(sec))).unwrap_or_default();
        // (A setting the profile doesn't have is fine: Clone Hero uses its default.)
        if let Some((k, ..)) = NUMBERS.iter().find(|(k, ..)| keys.iter().any(|x| x == k) && self.get(k).parse::<u32>().is_err()) {
            self.status = Some((false, format!("{k} needs a number.")));
            return;
        }
        if game_running() {
            self.status = Some((false, "Close Clone Hero first: it saves its profiles when it closes, which would undo these changes.".into()));
            return;
        }
        // Changed by someone else since it was read: don't write over that.
        let now = std::fs::read(&path).ok().map(|b| Profile::parse(&String::from_utf8_lossy(&b)));
        if now.as_ref() != self.saved.as_ref() {
            self.status = Some((false, "profiles.ini changed since Truss read it (Clone Hero may have saved it). Press Reload, then make your changes again.".into()));
            return;
        }
        let text = if self.bom { format!("\u{feff}{}", p.to_text()) } else { p.to_text() };
        match write(&path, &text) {
            Ok(()) => {
                self.saved = Some(p);
                self.status = Some((true, "Saved. The old file is kept as profiles.ini.bak.".into()));
            }
            Err(e) => self.status = Some((false, format!("Couldn't save: {e}"))),
        }
    }

    /// One setting of "Other settings": a tick box if it's on/off, else its value to edit.
    fn setting_line(&self, k: String) -> Element<'_, Msg> {
        let value = self.get(&k);
        let key = k.clone();
        let name = label_for(&k);
        let tip = |e: Element<'static, Msg>, k: String| -> Element<'static, Msg> {
            iced::widget::tooltip(e, container(text(k).size(11).font(iced::Font::MONOSPACE)).padding([3, 6]).style(crate::style::panel_solid), iced::widget::tooltip::Position::Top).into()
        };
        if is_switch(&k, &value) {
            tip(
                checkbox(value == "1")
                    .label(name)
                    .text_size(13)
                    .on_toggle(move |on| Msg::Other(key.clone(), if on { "1" } else { "0" }.into()))
                    .style(crate::style::visible_checkbox)
                    .into(),
                k,
            )
        } else {
            row![
                tip(text(name).size(13).width(200).into(), k),
                text_input("", &value).on_input(move |v| Msg::Other(key.clone(), v)).size(13).width(Length::Fill).padding([5, 8]),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        }
    }

    pub fn view(&self) -> Element<'_, Msg> {
        let muted = crate::style::muted_text;
        let heading = |s: &'static str| text(s).size(15).font(crate::BOLD);
        let note = |s: String| text(s).size(12).style(muted);
        let Some(_) = &self.edited else {
            let why = match &self.path {
                Some(p) => format!("Couldn't read {}.", p.display()),
                None => "Clone Hero's profiles.ini wasn't found. Choose Clone Hero's folder in Settings, or start Clone Hero once so it makes one.".into(),
            };
            return container(column![heading("Profile Editor"), note(why)].spacing(8)).padding(20).into();
        };

        // The profiles, by name.
        let mut picks = row![].spacing(6);
        for (i, s) in self.sections.iter().enumerate() {
            let name = self.saved.as_ref().and_then(|p| p.get(s, "player_name")).filter(|n| !n.is_empty()).unwrap_or(s);
            picks = picks.push(
                button(crate::rich::view(name, 13, iced::Font::DEFAULT, None))
                    .padding([5, 12])
                    .style(crate::style::quiet_button(i == self.open))
                    .on_press(Msg::Pick(i)),
            );
        }

        // The name.
        let style = self.style.unwrap_or(Style::Plain);
        let preview = container(crate::rich::view(&self.tags, 30, crate::BOLD, None))
            .padding([14, 18])
            .width(Length::Fill)
            .clip(true)
            .style(|t: &Theme| iced::widget::container::Style { background: Some(crate::style::raised(t).into()), ..crate::style::panel(t) });
        let mut name = column![
            heading("Name"),
            preview,
            row![
                text_input("Name", &self.name).on_input(Msg::Name).size(14).padding([7, 10]).width(Length::Fill),
                pick_list([Style::Plain, Style::One, Style::Gradient, Style::Tags], Some(style), Msg::Style).text_size(13).padding([6, 10]),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(10);
        if style == Style::Tags {
            name = name.push(text_input("<color=#ff8800>Name</color>", &self.tags).on_input(Msg::Tags).size(13).padding([7, 10]).font(iced::Font::MONOSPACE)).push(note(
                "Clone Hero's rich-text tags: <color=#RRGGBB>…</color>, <b>…</b>, <i>…</i>.".into(),
            ));
        } else {
            name = name.push(
                row![
                    checkbox(self.bold).label("Bold").text_size(13).on_toggle(Msg::Bold).style(crate::style::visible_checkbox),
                    checkbox(self.italic).label("Italic").text_size(13).on_toggle(Msg::Italic).style(crate::style::visible_checkbox),
                ]
                .spacing(16),
            );
        }
        if let Some(which) = self.which.filter(|_| matches!(style, Style::One | Style::Gradient)) {
            let swatch = |w: Which, label: &'static str, c: Color| {
                button(
                    row![
                        container(space()).width(16).height(16).style(move |_: &Theme| iced::widget::container::Style {
                            background: Some(c.into()),
                            border: iced::Border::default().rounded(3),
                            ..Default::default()
                        }),
                        text(label).size(13),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                )
                .padding([5, 10])
                .style(crate::style::quiet_button(which == w))
                .on_press(Msg::Which(w))
            };
            let mut swatches = row![].spacing(6);
            if style == Style::Gradient {
                swatches = swatches.push(swatch(Which::From, "Start", self.from)).push(swatch(Which::To, "End", self.to));
            }
            let (h, s, v) = self.hsv;
            name = name.push(
                row![swatches, space().width(Length::Fill), text_input("#RRGGBB", &self.hex).on_input(|v| Msg::Colour(PickerInput::Hex(v))).size(13).width(100)]
                    .spacing(8)
                    .align_y(Alignment::Center),
            );
            name = name.push(canvas(Picker { h, s, v, on: |h, s, v| Msg::Colour(PickerInput::Hsv(h, s, v)) }).width(Length::Fill).height(150));
        }

        // Settings with their own controls.
        let colour = self.get("color_profile_name");
        let mut colours = self.colour_profiles.clone();
        if !colour.is_empty() && !colours.contains(&colour) {
            colours.insert(0, colour.clone());
        }
        // The highway in use (one of the game's own, say) stays a choice too.
        let highway = self.get("highway_name");
        let mut highways = self.highways.clone();
        for keep in [highway.as_str(), "Default"] {
            if !keep.is_empty() && !highways.iter().any(|h| h == keep) {
                highways.insert(0, keep.to_string());
            }
        }
        let label = |s: &'static str| text(s).size(13).width(170);
        let mut settings = column![
            heading("Settings"),
            row![label("Color profile"), pick_list(colours, (!colour.is_empty()).then_some(colour), Msg::ColorProfile).text_size(13).padding([5, 10])]
                .spacing(8)
                .align_y(Alignment::Center),
            row![label("Highway"), pick_list(highways, (!highway.is_empty()).then_some(highway), Msg::Highway).text_size(13).padding([5, 10])]
                .spacing(8)
                .align_y(Alignment::Center),
        ]
        .spacing(10);
        for (k, l, lo, hi) in NUMBERS {
            settings = settings.push(
                row![
                    label(l),
                    text_input("", &self.get(k)).on_input(move |v| Msg::Number(k, v)).size(13).width(80).padding([5, 8]),
                    note(format!("{lo} to {hi}")),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        for (k, l, about) in SWITCHES {
            settings = settings.push(
                row![
                    checkbox(self.get(k) == "1").label(l).text_size(13).on_toggle(move |on| Msg::Switch(k, on)).style(crate::style::visible_checkbox).width(170),
                    note(about.into()),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }

        // Everything else: on/off settings as tick boxes, the rest as Clone Hero writes them.
        let mut other = column![heading("Other settings"), note("The rest of this profile. Hover a setting for its name in profiles.ini.".into())].spacing(8);
        let keys = match (&self.edited, self.section()) {
            (Some(p), Some(s)) => p.keys(s),
            _ => Vec::new(),
        };
        let (kit, rest): (Vec<String>, Vec<String>) = keys.into_iter().filter(|k| !OWN.contains(&k.as_str())).partition(|k| DRUM_KIT.contains(&k.as_str()));
        for k in rest {
            other = other.push(self.setting_line(k));
        }
        if !kit.is_empty() {
            let label = format!("{}  Advanced", if self.drum_kit { "▾" } else { "▸" });
            other = other.push(button(text(label).size(13)).padding([6, 0]).style(crate::style::subtle).on_press(Msg::DrumKit));
            if self.drum_kit {
                for k in kit {
                    other = other.push(self.setting_line(k));
                }
            } else {
                other = other.push(note("Drum kit (MIDI) settings, and the controller type Clone Hero sets itself. Most players never need these.".into()));
            }
        }

        let changed = self.changed();
        let mut actions = row![
            button(text("Save").size(14)).padding([8, 18]).style(button::primary).on_press_maybe(changed.then_some(Msg::Save)),
            button(text("Discard changes").size(13)).padding([7, 12]).style(crate::style::secondary).on_press_maybe(changed.then_some(Msg::Discard)),
            button(text("Reload").size(13)).padding([7, 12]).style(crate::style::secondary).on_press(Msg::Reload),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if let Some((good, s)) = &self.status {
            let good = *good;
            actions = actions.push(text(s.clone()).size(12).style(move |t: &Theme| if good { text::success(t) } else { text::danger(t) }));
        }
        let path = self.path.as_ref().map_or(String::new(), |p| p.display().to_string());
        let page = column![
            heading("Profile Editor"),
            note(format!("Edits Clone Hero's profiles ({path}). Close Clone Hero before saving: it saves this file itself when it closes.")),
            picks.wrap(),
            actions,
            row![column![name, settings].spacing(24).width(Length::FillPortion(3)), column![other].width(Length::FillPortion(2))].spacing(32),
        ]
        .spacing(16)
        .max_width(1100);
        scrollable(container(page).padding(20)).height(Length::Fill).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_names() {
        assert_eq!(label_for("is_bot"), "Bot");
        assert_eq!(label_for("some_new_thing"), "Some new thing");
        assert!(is_switch("alt_taps", "1") && !is_switch("alt_taps", "2") && !is_switch("note_speed", "1"));
    }

    #[test]
    fn names_with_colour() {
        let red = Color::from_rgb8(255, 0, 0);
        let blue = Color::from_rgb8(0, 0, 255);
        assert_eq!(styled_name("nick", Style::Plain, red, red, false, false), "nick");
        assert_eq!(styled_name("nick", Style::One, red, red, true, false), "<b><color=#FF0000>nick</color></b>");
        let g = styled_name("a b", Style::Gradient, red, blue, false, false);
        assert_eq!(g, "<color=#FF0000>a</color> <color=#0000FF>b</color>");
        // A gradient a shade off from Truss's own maths (colours rounded) still reads as one.
        assert_eq!(read_name("<color=#9B36EB>a</color><color=#A859EF>b</color><color=#B67CF2>c</color>").1, Style::Gradient);
        // Read back as what made it.
        for (s, style) in [("nick", Style::Plain), ("<b><color=#FF0000>nick</color></b>", Style::One), (g.as_str(), Style::Gradient), ("<color=red>ni</color>ck", Style::Tags)] {
            let (_, got, ..) = read_name(s);
            assert_eq!(got, style, "{s}");
        }
    }

    #[test]
    fn saves_only_what_changed() {
        let dir = crate::test_dir().join("profile-editor");
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("profiles.ini");
        let original = "[profile0]\r\nnote_speed = 13\r\nplayer_name = nick\r\nmystery = 7\r\n\r\n[profile1]\r\nplayer_name = other\r\n";
        std::fs::write(&file, original).unwrap();
        let mut e = ProfileEditor { path: Some(file.clone()), ..Default::default() };
        let p = Profile::parse(original);
        e.sections = vec!["profile0".into(), "profile1".into()];
        (e.saved, e.edited) = (Some(p.clone()), Some(p));
        e.load_name();
        let _ = e.update(Msg::Style(Style::One));
        let _ = e.update(Msg::Colour(PickerInput::Hex("#00ff00".into())));
        let _ = e.update(Msg::Number("note_speed", "15x".into()));
        let _ = e.update(Msg::Save);
        assert!(e.status.as_ref().is_some_and(|s| s.0), "{:?}", e.status);
        let saved = std::fs::read_to_string(&file).unwrap();
        assert_eq!(saved, "[profile0]\r\nnote_speed = 15\r\nplayer_name = <color=#00FF00>nick</color>\r\nmystery = 7\r\n\r\n[profile1]\r\nplayer_name = other\r\n");
        assert_eq!(std::fs::read_to_string(dir.join("profiles.ini.bak")).unwrap(), original);
        // Changed on disk meanwhile: not written over.
        let _ = e.update(Msg::Name("nicky".into()));
        std::fs::write(&file, original).unwrap();
        let _ = e.update(Msg::Save);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
        let _ = std::fs::remove_dir_all(dir);
    }
}
