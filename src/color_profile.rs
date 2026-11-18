//! Clone Hero colour profiles (PlayerData/Custom/Colors/*.ini): `[guitar]`, `[drums]`,
//! `[sixfret]` and `[other]` sections of `key = #RRGGBB` lines.
//!
//! A profile is edited line by line, so a saved file keeps its order, its comments and any
//! keys Truss doesn't know about; only the values that were changed are rewritten.

use iced::Color;
use std::path::PathBuf;

/// Clone Hero's own default colours (its DefaultColors.ini).
pub const DEFAULT_INI: &str = include_str!("../data/ch-default-colors.ini");

pub const SECTIONS: [&str; 4] = ["guitar", "drums", "sixfret", "other"];

#[derive(Debug, Clone, PartialEq)]
enum Line {
    Section(String),
    Entry { key: String, value: String },
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    lines: Vec<Line>,
    crlf: bool,
}

impl Profile {
    pub fn parse(text: &str) -> Self {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let lines = text
            .lines()
            .map(|raw| {
                let t = raw.trim();
                if t.starts_with('[') && t.ends_with(']') && t.len() > 2 {
                    Line::Section(t[1..t.len() - 1].trim().to_string())
                } else if !t.starts_with(';') && !t.starts_with('#')
                    && let Some((k, v)) = t.split_once('=')
                    && !k.trim().is_empty()
                {
                    Line::Entry { key: k.trim().to_string(), value: v.trim().to_string() }
                } else {
                    Line::Other(raw.to_string())
                }
            })
            .collect();
        Profile { lines, crlf: text.contains("\r\n") }
    }

    pub fn defaults() -> Self {
        Self::parse(DEFAULT_INI)
    }

    /// Where each line sits: (section, line index) for entries.
    fn entries(&self) -> impl Iterator<Item = (Option<&str>, usize, &str, &str)> {
        let mut section: Option<&str> = None;
        self.lines.iter().enumerate().filter_map(move |(i, l)| match l {
            Line::Section(s) => {
                section = Some(s.as_str());
                None
            }
            Line::Entry { key, value } => Some((section, i, key.as_str(), value.as_str())),
            Line::Other(_) => None,
        })
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.entries()
            .find(|(s, _, k, _)| s.is_some_and(|s| s.eq_ignore_ascii_case(section)) && *k == key)
            .map(|(_, _, _, v)| v)
    }

    pub fn color(&self, section: &str, key: &str) -> Option<Color> {
        self.get(section, key).and_then(crate::theme_maker::parse_hex)
    }

    /// The keys of a section, in file order.
    pub fn keys(&self, section: &str) -> Vec<String> {
        self.entries().filter(|(s, ..)| s.is_some_and(|s| s.eq_ignore_ascii_case(section))).map(|(_, _, k, _)| k.to_string()).collect()
    }

    /// Sets a value: in place if the key is there, else at the end of its section (which is
    /// added at the end of the file if it's missing).
    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        let at = self.entries().find(|(s, _, k, _)| s.is_some_and(|s| s.eq_ignore_ascii_case(section)) && *k == key).map(|(_, i, ..)| i);
        let entry = Line::Entry { key: key.to_string(), value: value.to_string() };
        if let Some(i) = at {
            self.lines[i] = entry;
            return;
        }
        // The section's last entry, or its header.
        let mut current = false;
        let mut last = None;
        for (i, l) in self.lines.iter().enumerate() {
            match l {
                Line::Section(s) => {
                    current = s.eq_ignore_ascii_case(section);
                    if current {
                        last = Some(i);
                    }
                }
                Line::Entry { .. } if current => last = Some(i),
                _ => {}
            }
        }
        match last {
            Some(i) => self.lines.insert(i + 1, entry),
            None => {
                if self.lines.last().is_some_and(|l| !matches!(l, Line::Other(s) if s.trim().is_empty())) {
                    self.lines.push(Line::Other(String::new()));
                }
                self.lines.push(Line::Section(section.to_string()));
                self.lines.push(entry);
            }
        }
    }

    pub fn to_text(&self) -> String {
        let nl = if self.crlf { "\r\n" } else { "\n" };
        let mut out = String::new();
        for l in &self.lines {
            match l {
                Line::Section(s) => out.push_str(&format!("[{s}]")),
                Line::Entry { key, value } => out.push_str(&format!("{key} = {value}")),
                Line::Other(s) => out.push_str(s),
            }
            out.push_str(nl);
        }
        out
    }
}

/// A colour as Clone Hero writes it: "#RRGGBB".
pub fn ch_hex(c: Color) -> String {
    crate::theme_maker::hex(c).to_uppercase()
}

/// Clone Hero's colour profile folder (Documents\Clone Hero\PlayerData\Custom\Colors), if
/// there is one.
pub fn folder() -> Option<PathBuf> {
    let user = directories::UserDirs::new()?;
    let mut tries = Vec::new();
    if let Some(d) = user.document_dir() {
        tries.push(d.join(r"Clone Hero\PlayerData\Custom\Colors"));
        tries.push(d.join(r"Clone Hero\Custom\Colors"));
    }
    tries.push(user.home_dir().join(r"OneDrive\Documents\Clone Hero\PlayerData\Custom\Colors"));
    tries.into_iter().find(|p| p.is_dir())
}

/// The profiles in a folder (file stems), sorted.
pub fn list(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ini")).then(|| p.file_stem().unwrap_or_default().to_string_lossy().into_owned())
        })
        .collect();
    v.sort_by_key(|s| s.to_lowercase());
    v
}

// ---------- What the keys are ----------

/// A colour lane, for the grids.
#[derive(Debug, Clone, Copy)]
pub struct Lane {
    pub id: &'static str,
    pub label: &'static str,
}

pub const GUITAR_LANES: [Lane; 6] = [
    Lane { id: "green", label: "Green" },
    Lane { id: "red", label: "Red" },
    Lane { id: "yellow", label: "Yellow" },
    Lane { id: "blue", label: "Blue" },
    Lane { id: "orange", label: "Orange" },
    Lane { id: "open", label: "Open" },
];

pub const DRUM_LANES: [Lane; 5] = [
    Lane { id: "red", label: "Red" },
    Lane { id: "yellow", label: "Yellow" },
    Lane { id: "blue", label: "Blue" },
    Lane { id: "green", label: "Green" },
    Lane { id: "kick", label: "Kick" },
];

/// A row of a grid: its label, its key with `{}` for the lane, and the lanes it has.
pub struct GridRow {
    pub label: &'static str,
    pub key: &'static str,
    pub lanes: &'static [&'static str],
}

const G5: &[&str] = &["green", "red", "yellow", "blue", "orange"];
const G6: &[&str] = &["green", "red", "yellow", "blue", "orange", "open"];
const D4: &[&str] = &["red", "yellow", "blue", "green"];
const D5: &[&str] = &["red", "yellow", "blue", "green", "kick"];
const KICK: &[&str] = &["kick"];

pub const GUITAR_GRID: &[GridRow] = &[
    GridRow { label: "Note", key: "note_{}", lanes: G6 },
    GridRow { label: "Note detail", key: "note_anim_{}", lanes: G6 },
    GridRow { label: "Sustain", key: "sustain_{}", lanes: G6 },
    GridRow { label: "Fret", key: "striker_cover_{}", lanes: G5 },
    GridRow { label: "Fret top", key: "striker_head_cover_{}", lanes: G5 },
    GridRow { label: "Fret light", key: "striker_head_light_{}", lanes: G6 },
    GridRow { label: "Fret base", key: "striker_base_{}", lanes: G5 },
];

pub const DRUM_GRID: &[GridRow] = &[
    GridRow { label: "Tom", key: "tom_{}", lanes: D4 },
    GridRow { label: "Tom detail", key: "tom_anim_{}", lanes: D4 },
    GridRow { label: "Cymbal", key: "cym_{}", lanes: D4 },
    GridRow { label: "Cymbal detail", key: "cym_anim_{}", lanes: D4 },
    GridRow { label: "Kick note", key: "note_{}", lanes: KICK },
    GridRow { label: "Kick detail", key: "note_anim_{}", lanes: KICK },
    GridRow { label: "Pad", key: "drums_striker_cover_{}", lanes: D4 },
    GridRow { label: "Pad top", key: "drums_striker_head_cover_{}", lanes: D4 },
    GridRow { label: "Pad light", key: "drums_striker_head_light_{}", lanes: D5 },
    GridRow { label: "Pad base", key: "drums_striker_base_{}", lanes: D4 },
];

pub fn grid(section: &str) -> (&'static [Lane], &'static [GridRow]) {
    match section {
        "guitar" => (&GUITAR_LANES, GUITAR_GRID),
        "drums" => (&DRUM_LANES, DRUM_GRID),
        _ => (&[], &[]),
    }
}

pub fn grid_key(row: &GridRow, lane: &str) -> String {
    row.key.replace("{}", lane)
}

/// The keys of a section that aren't in its grid (star power, six-fret, combo...), from
/// Clone Hero's defaults and the profile, in that order.
pub fn other_keys(section: &str, p: &Profile, defaults: &Profile) -> Vec<String> {
    let (_, rows) = grid(section);
    let in_grid = |k: &str| rows.iter().any(|r| r.lanes.iter().any(|l| grid_key(r, l) == k));
    let mut out: Vec<String> = Vec::new();
    for k in defaults.keys(section).into_iter().chain(p.keys(section)) {
        if !in_grid(&k) && !out.contains(&k) {
            out.push(k);
        }
    }
    // Star power and similar keys together, the way they read.
    out.sort_by_key(|k| (k.contains("sp_") || k.contains("_sp"), label(k)));
    out
}

/// A key as words: "note_anim_sp_phrase_active" is "Note detail · SP phrase (active)".
pub fn label(key: &str) -> String {
    let k = key.strip_prefix("sf_").unwrap_or(key);
    let (base, sp) = if let Some(b) = k.strip_suffix("_sp_phrase_active") {
        (b, Some("star power phrase, while active"))
    } else if let Some(b) = k.strip_suffix("_sp_phrase") {
        (b, Some("star power phrase"))
    } else if let Some(b) = k.strip_suffix("_sp_active") {
        (b, Some("star power active"))
    } else {
        (k, None)
    };
    let words: Vec<String> = base
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|w| match w {
            "anim" => "detail".to_string(),
            "striker" => "fret".to_string(),
            "cym" => "cymbal".to_string(),
            "hopo" => "HOPO".to_string(),
            "sp" => "star power".to_string(),
            "elec" => "electricity".to_string(),
            "act" => "activation".to_string(),
            "drums" => String::new(),
            w => w.to_string(),
        })
        .filter(|w| !w.is_empty())
        .collect();
    let mut s = words.join(" ");
    if let Some(c) = s.get(0..1) {
        s = c.to_uppercase() + &s[1..];
    }
    match sp {
        Some(sp) => format!("{s} · {sp}"),
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVILISH: &str = "[other]\r\ncombo_one = #3827DB\r\n\r\n[guitar]\r\nnote_green = #CF81F9\r\nnote_red = #8349F3\r\n";

    #[test]
    fn edits_keep_the_file() {
        let mut p = Profile::parse(DEVILISH);
        assert_eq!(p.get("guitar", "note_green"), Some("#CF81F9"));
        assert_eq!(p.to_text(), DEVILISH, "unchanged files are written back the same");
        p.set("guitar", "note_green", "#00FF00");
        p.set("guitar", "note_open", "#BA00FF");
        p.set("drums", "tom_red", "#FF0000");
        let t = p.to_text();
        assert!(t.contains("note_green = #00FF00\r\nnote_red = #8349F3\r\nnote_open = #BA00FF\r\n"));
        assert!(t.ends_with("\r\n[drums]\r\ntom_red = #FF0000\r\n"));
        assert_eq!(Profile::parse(&t).get("drums", "tom_red"), Some("#FF0000"));
    }

    /// Every profile in Clone Hero's folder reads and writes back byte for byte (reads only).
    #[test]
    #[ignore]
    fn real_profiles_round_trip() {
        let Some(dir) = folder() else { return };
        for name in list(&dir) {
            let text = std::fs::read_to_string(dir.join(format!("{name}.ini"))).unwrap();
            assert_eq!(Profile::parse(&text).to_text(), text, "{name}");
            println!("{name}: ok");
        }
    }

    #[test]
    fn defaults_and_labels() {
        let d = Profile::defaults();
        for s in SECTIONS {
            assert!(!d.keys(s).is_empty(), "{s}");
        }
        // Every grid key is one Clone Hero has.
        for s in ["guitar", "drums"] {
            let (_, rows) = grid(s);
            for r in rows {
                for l in r.lanes {
                    assert!(d.get(s, &grid_key(r, l)).is_some(), "{s} {}", grid_key(r, l));
                }
            }
        }
        assert_eq!(label("note_anim_sp_phrase_active"), "Note detail · star power phrase, while active");
        assert_eq!(label("striker_hit_flame_kick"), "Fret hit flame kick");
        assert_eq!(ch_hex(Color::from_rgb8(0xcf, 0x81, 0xf9)), "#CF81F9");
        assert!(other_keys("guitar", &d, &d).iter().all(|k| k.contains("sp_")));
    }
}
