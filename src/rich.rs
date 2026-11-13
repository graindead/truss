//! Song names, artists, charters and so on can carry Clone Hero's rich-text tags
//! (`<color=#ff8800>`, `<b>`, `<i>`, …). Charters use them to colour their names. This turns
//! them into styled text instead of showing them stripped.

use iced::font::{Style as FontStyle, Weight};
use iced::widget::{rich_text, span, text};
use iced::{Color, Element, Font};
use std::sync::atomic::{AtomicBool, Ordering};

/// Settings > Appearance > text colours.
static ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub text: String,
    pub color: Option<Color>,
    pub bold: bool,
    pub italic: bool,
}

/// Splits tagged text into styled pieces. Unknown tags are dropped, like Clone Hero does;
/// a lone `<` that doesn't start a tag is kept as text.
pub fn parse(s: &str) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    let mut colors: Vec<Option<Color>> = Vec::new();
    let (mut bold, mut italic) = (0u32, 0u32);
    let mut rest = s;
    let mut push = |t: &str, color: Option<Color>, bold: bool, italic: bool| {
        if t.is_empty() {
            return;
        }
        match out.last_mut() {
            Some(p) if p.color == color && p.bold == bold && p.italic == italic => p.text.push_str(t),
            _ => out.push(Piece { text: t.to_string(), color, bold, italic }),
        }
    };
    while !rest.is_empty() {
        let color = colors.last().copied().flatten();
        let Some(start) = rest.find('<') else {
            push(rest, color, bold > 0, italic > 0);
            break;
        };
        push(&rest[..start], color, bold > 0, italic > 0);
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else {
            push(&rest[start..], color, bold > 0, italic > 0);
            break;
        };
        let tag = after[..end].trim();
        let lower = tag.to_ascii_lowercase();
        if !is_tag(&lower) {
            push("<", color, bold > 0, italic > 0);
            rest = after;
            continue;
        }
        match lower.as_str() {
            "b" => bold += 1,
            "/b" => bold = bold.saturating_sub(1),
            "i" => italic += 1,
            "/i" => italic = italic.saturating_sub(1),
            "/color" => {
                colors.pop();
            }
            t if t.starts_with("color") => colors.push(parse_color(tag[5..].trim_start_matches(['=', ' ']).trim_matches('"'))),
            _ => {}
        }
        rest = &after[end + 1..];
    }
    // Trim the outer whitespace, as strip_tags does.
    if let Some(first) = out.first_mut() {
        first.text = first.text.trim_start().to_string();
    }
    if let Some(last) = out.last_mut() {
        last.text = last.text.trim_end().to_string();
    }
    out.retain(|p| !p.text.is_empty());
    out
}

/// Whether `<...>` is a rich-text tag (so "a < b > c" stays as written).
fn is_tag(lower: &str) -> bool {
    let name = lower.trim_start_matches('/');
    let name = name.split(['=', ' ']).next().unwrap_or("");
    matches!(name, "b" | "i" | "u" | "s" | "color" | "size" | "alpha" | "mark" | "sup" | "sub" | "noparse" | "font" | "material" | "voffset" | "cspace" | "mspace" | "lowercase" | "uppercase" | "smallcaps" | "br")
}

fn parse_color(v: &str) -> Option<Color> {
    let named = |n: &str| -> Option<u32> {
        Some(match n {
            "red" => 0xFF0000,
            "green" => 0x00FF00,
            "blue" => 0x0000FF,
            "yellow" => 0xFFFF00,
            "orange" => 0xFFA500,
            "purple" => 0xA020F0,
            "white" => 0xFFFFFF,
            "black" => 0x000000,
            "cyan" | "aqua" => 0x00FFFF,
            "magenta" | "fuchsia" => 0xFF00FF,
            "lime" => 0x00FF00,
            "grey" | "gray" => 0x808080,
            "brown" => 0xA52A2A,
            "pink" => 0xFFC0CB,
            _ => return None,
        })
    };
    let rgb = match v.strip_prefix('#') {
        Some(h) if h.chars().all(|c| c.is_ascii_hexdigit()) => match h.len() {
            3 | 4 => {
                let d = |i: usize| u32::from_str_radix(&h[i..i + 1], 16).map(|x| x * 17);
                (d(0).ok()? << 16) | (d(1).ok()? << 8) | d(2).ok()?
            }
            6 | 8 => u32::from_str_radix(&h[..6], 16).ok()?,
            _ => return None,
        },
        _ => named(&v.to_ascii_lowercase())?,
    };
    Some(readable(Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)))
}

/// Lightens colours too dark to read on Truss's dark background.
fn readable(c: Color) -> Color {
    let lum = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    const MIN: f32 = 0.32;
    if lum >= MIN {
        return c;
    }
    let t = (MIN - lum) / (1.0 - lum).max(0.001);
    Color::from_rgb(c.r + (1.0 - c.r) * t, c.g + (1.0 - c.g) * t, c.b + (1.0 - c.b) * t)
}

/// Whether the text has any styling worth drawing (otherwise plain text is used).
pub fn is_styled(s: &str) -> bool {
    ENABLED.load(Ordering::Relaxed) && s.contains('<') && parse(s).iter().any(|p| p.color.is_some() || p.bold || p.italic)
}

/// Tagged text as a widget: styled when it has colours or bold/italic, plain otherwise.
/// `base` and `color` are the font and colour for unstyled parts (`None`: the usual colour).
pub fn view<'a, M: 'a>(s: &str, size: u32, base: Font, color: Option<Color>) -> Element<'a, M> {
    if !is_styled(s) {
        return text(crate::api::strip_tags(s)).size(size).font(base).color_maybe(color).wrapping(text::Wrapping::None).into();
    }
    let spans: Vec<text::Span<'a, (), Font>> = parse(s)
        .into_iter()
        .map(|p| {
            let font = Font {
                weight: if p.bold { Weight::Bold } else { base.weight },
                style: if p.italic { FontStyle::Italic } else { base.style },
                ..base
            };
            span(p.text).font(font).color_maybe(p.color)
        })
        .collect();
    rich_text(spans).size(size).color_maybe(color).wrapping(text::Wrapping::None).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colours_and_bold() {
        let s = "<b><color=#c96cff>j</color><color=#996cff>d</color>urand</b>TV";
        let p = parse(s);
        assert_eq!(p.iter().map(|p| p.text.as_str()).collect::<String>(), "jdurandTV");
        assert_eq!(p[0].color, Some(Color::from_rgb8(0xc9, 0x6c, 0xff)));
        assert!(p[0].bold && p[2].bold && !p[3].bold);
        assert_eq!(p[2].color, None);
        assert!(is_styled(s));
    }

    #[test]
    fn plain_and_odd_text() {
        assert!(!is_styled("Rock & Roll"));
        assert!(!is_styled("<size=20>Big</size>"));
        let p = parse("x <not a tag> y");
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].text, "x <not a tag> y");
        assert_eq!(parse("<color=red>x</color> <unknown>y")[1].text, " <unknown>y");
        // Too dark to read on the dark background: lightened.
        let dark = parse("<color=#000000>x</color>")[0].color.unwrap();
        assert!(dark.r > 0.2);
        assert_eq!(parse("<color=#FFF>x</color>")[0].color, Some(Color::WHITE));
    }
}
