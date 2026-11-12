//! Truss's themes. Also compiled into build.rs (with `iced_core` standing in for `iced`), which
//! builds a recoloured taskbar icon for every theme into truss.exe; the icon for `all()[i]` gets
//! resource ID `ICON_ID_BASE + i` (the Rust theme, first, uses the exe's main icon instead).

use iced::theme::Palette;
use iced::{Theme, color};

/// Resource ID of the first themed icon in truss.exe.
pub const ICON_ID_BASE: u16 = 100;

pub fn bridge_dark() -> Theme {
    Theme::custom(
        "Bridge Dark",
        Palette {
            background: color!(0x15161c),
            text: color!(0xe4e4ea),
            primary: color!(0x7c5cff),
            success: color!(0x3ccf8e),
            warning: color!(0xf5b942),
            danger: color!(0xf0506e),
        },
    )
}

/// The default theme: warm charcoal with Rust-orange accents.
pub fn rust_theme() -> Theme {
    Theme::custom(
        "Rust",
        Palette {
            background: color!(0x1b1715),
            text: color!(0xefe6de),
            primary: color!(0xe2622a),
            success: color!(0x7cc47f),
            warning: color!(0xf2b545),
            danger: color!(0xe5484d),
        },
    )
}

/// Deep navy with a clear blue accent.
pub fn kusha_theme() -> Theme {
    Theme::custom(
        "Kusha",
        Palette {
            background: color!(0x131924),
            text: color!(0xe2e9f4),
            primary: color!(0x3b8ef0),
            success: color!(0x5cc89c),
            warning: color!(0xf2c14e),
            danger: color!(0xeb5a6b),
        },
    )
}

/// Deep plum with a vivid purple accent.
pub fn nick_theme() -> Theme {
    Theme::custom(
        "Nick",
        Palette {
            background: color!(0x19141f),
            text: color!(0xece4f4),
            primary: color!(0x9d5cf0),
            success: color!(0x6cc79a),
            warning: color!(0xf2c14e),
            danger: color!(0xeb5a72),
        },
    )
}

/// Every theme, in the order shown in Settings (Rust first).
pub fn all() -> Vec<Theme> {
    [rust_theme(), kusha_theme(), nick_theme(), bridge_dark()].into_iter().chain(Theme::ALL.iter().cloned()).collect()
}

/// A theme's (accent, background) as 0–255 RGB, for recolouring the icon.
pub fn icon_colors(t: &Theme) -> ([f32; 3], [f32; 3]) {
    let p = t.palette();
    let to = |c: iced::Color| [c.r * 255.0, c.g * 255.0, c.b * 255.0];
    (to(p.primary), to(p.background))
}
