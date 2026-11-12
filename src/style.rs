//! The "Refined look" (Settings > Appearance): quieter styling with hairline dividers, muted
//! secondary text and the accent colour kept for what you act on. Colours come from the
//! current theme, so it works with every theme.

use iced::widget::{button, container, pick_list, text_input};
use iced::{Background, Border, Color, Theme};
use std::sync::atomic::{AtomicBool, Ordering};

static REFINED: AtomicBool = AtomicBool::new(false);

pub fn refined() -> bool {
    REFINED.load(Ordering::Relaxed)
}

pub fn set_refined(on: bool) {
    REFINED.store(on, Ordering::Relaxed);
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: 1.0 }
}

pub fn bg(t: &Theme) -> Color {
    t.extended_palette().background.base.color
}

pub fn fg(t: &Theme) -> Color {
    t.extended_palette().background.base.text
}

/// Secondary text: album, charter, labels.
pub fn muted(t: &Theme) -> Color {
    mix(bg(t), fg(t), 0.55)
}

/// Dividers and outlines.
pub fn hairline(t: &Theme) -> Color {
    mix(bg(t), fg(t), 0.11)
}

/// A slightly raised surface (inputs, the details panel).
pub fn raised(t: &Theme) -> Color {
    mix(bg(t), fg(t), 0.035)
}

pub fn accent(t: &Theme) -> Color {
    t.extended_palette().primary.base.color
}

/// The page background with the accent mixed in (selected rows).
pub fn accent_tint(t: &Theme, amount: f32) -> Color {
    mix(bg(t), accent(t), amount)
}

/// A one-pixel divider.
pub fn divider<'a, M: 'a>(vertical: bool) -> iced::Element<'a, M> {
    let c = container(iced::widget::space()).style(|t: &Theme| container::Style {
        background: Some(hairline(t).into()),
        ..Default::default()
    });
    if vertical { c.width(1).height(iced::Length::Fill).into() } else { c.height(1).width(iced::Length::Fill).into() }
}

/// Top navigation: muted text, the current page in full colour.
pub fn nav_button(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |t, s| button::Style {
        background: None,
        text_color: if active || matches!(s, button::Status::Hovered) { fg(t) } else { muted(t) },
        ..Default::default()
    }
}

/// Plain outlined button for secondary actions.
pub fn quiet_button(highlighted: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |t, s| {
        let hover = matches!(s, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if hover { raised(t) } else { bg(t) })),
            text_color: if highlighted { accent(t) } else { fg(t) },
            border: Border { color: if highlighted { accent(t) } else { hairline(t) }, width: 1.0, radius: 6.0.into() },
            ..Default::default()
        }
    }
}

pub fn search_input(t: &Theme, s: text_input::Status) -> text_input::Style {
    let mut st = text_input::default(t, s);
    st.background = Background::Color(raised(t));
    st.border = Border {
        color: if matches!(s, text_input::Status::Focused { .. }) { accent(t) } else { hairline(t) },
        width: 1.0,
        radius: 6.0.into(),
    };
    st.placeholder = muted(t);
    st
}

pub fn quiet_pick_list(t: &Theme, s: pick_list::Status) -> pick_list::Style {
    let mut st = pick_list::default(t, s);
    st.background = Background::Color(bg(t));
    st.border = Border {
        color: if matches!(s, pick_list::Status::Hovered | pick_list::Status::Opened { .. }) { muted(t) } else { hairline(t) },
        width: 1.0,
        radius: 6.0.into(),
    };
    st.handle_color = muted(t);
    st
}

/// A results row: a faint accent tint when selected (with an accent bar drawn beside it).
pub fn row_button(selected: bool, checked: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |t, s| {
        let bgc = if selected {
            Some(accent_tint(t, 0.16))
        } else if checked {
            Some(accent_tint(t, 0.08))
        } else if matches!(s, button::Status::Hovered) {
            Some(raised(t))
        } else {
            None
        };
        button::Style {
            background: bgc.map(Background::Color),
            text_color: fg(t),
            border: Border::default().rounded(4),
            ..Default::default()
        }
    }
}

/// Small outlined tag ("RV", "Lyrics").
pub fn outline_tag(t: &Theme) -> container::Style {
    container::Style {
        background: None,
        text_color: Some(muted(t)),
        border: Border { color: hairline(t), width: 1.0, radius: 3.0.into() },
        ..Default::default()
    }
}

/// `button::secondary`, or the quiet outlined button with the refined look on.
pub fn secondary(t: &Theme, s: button::Status) -> button::Style {
    if refined() { quiet_button(false)(t, s) } else { button::secondary(t, s) }
}

/// `container::rounded_box`, or a hairline-outlined panel on the page background.
pub fn panel(t: &Theme) -> container::Style {
    if refined() {
        container::Style {
            background: None,
            border: Border { color: hairline(t), width: 1.0, radius: 6.0.into() },
            ..Default::default()
        }
    } else {
        container::rounded_box(t)
    }
}

/// `button::subtle`, or plain muted text that brightens on hover.
pub fn subtle(t: &Theme, s: button::Status) -> button::Style {
    if !refined() {
        return button::subtle(t, s);
    }
    let hover = matches!(s, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hover.then(|| Background::Color(raised(t))),
        text_color: if hover { fg(t) } else { muted(t) },
        border: Border::default().rounded(4),
        ..Default::default()
    }
}

/// Divider lines in the theme's hairline colour.
pub fn rule(t: &Theme) -> iced::widget::rule::Style {
    let mut st = iced::widget::rule::default(t);
    if refined() {
        st.color = hairline(t);
    }
    st
}

/// Shaded panel backgrounds: flat page background with the refined look.
pub fn weakest(t: &Theme) -> Color {
    if refined() { bg(t) } else { t.extended_palette().background.weakest.color }
}

/// Hover and raised surfaces.
pub fn weak(t: &Theme) -> Color {
    if refined() { raised(t) } else { t.extended_palette().background.weak.color }
}

/// Placeholders and "off" fills.
pub fn strong(t: &Theme) -> Color {
    if refined() { hairline(t) } else { t.extended_palette().background.strong.color }
}

/// Selected list rows.
pub fn selected(t: &Theme) -> Color {
    if refined() { accent_tint(t, 0.16) } else { t.extended_palette().primary.weak.color }
}

pub fn selected_text(t: &Theme) -> Color {
    if refined() { fg(t) } else { t.extended_palette().primary.weak.text }
}

/// A page's tab: a filled button normally, text with an accent underline when refined.
pub fn tab<'a, M: Clone + 'a>(label: &'static str, active: bool, size: u32, msg: M) -> iced::Element<'a, M> {
    if !refined() {
        return button(iced::widget::text(label).size(size))
            .padding([7, 14])
            .style(move |th: &Theme, s| if active { button::primary(th, s) } else { button::subtle(th, s) })
            .on_press(msg)
            .into();
    }
    iced::widget::column![
        button(iced::widget::text(label).size(size)).padding([6, 4]).style(nav_button(active)).on_press(msg),
        container(iced::widget::space()).height(2).width(iced::Length::Fill).style(move |t: &Theme| container::Style {
            background: active.then(|| accent(t).into()),
            ..Default::default()
        }),
    ]
    .width(iced::Length::Shrink)
    .into()
}

/// Tooltips and popups: a raised surface with a hairline outline.
pub fn panel_solid(t: &Theme) -> container::Style {
    container::Style {
        background: Some(raised(t).into()),
        text_color: Some(fg(t)),
        border: Border { color: hairline(t), width: 1.0, radius: 4.0.into() },
        ..Default::default()
    }
}

/// A checkbox with an outline you can see on any theme (iced's default nearly vanishes on dark
/// backgrounds, especially while disabled).
pub fn visible_checkbox(t: &Theme, s: iced::widget::checkbox::Status) -> iced::widget::checkbox::Style {
    use iced::widget::checkbox::Status;
    let p = t.extended_palette();
    let (checked, hovered, disabled) = match s {
        Status::Active { is_checked } => (is_checked, false, false),
        Status::Hovered { is_checked } => (is_checked, true, false),
        Status::Disabled { is_checked } => (is_checked, false, true),
    };
    let accent = p.primary.base.color;
    let outline = if hovered { accent } else { muted(t) };
    let fade = |c: Color| if disabled { c.scale_alpha(0.45) } else { c };
    iced::widget::checkbox::Style {
        background: if checked { fade(accent).into() } else { Color::TRANSPARENT.into() },
        icon_color: p.primary.base.text,
        border: Border { color: fade(if checked { accent } else { outline }), width: 1.5, radius: 3.0.into() },
        text_color: Some(fade(fg(t))),
    }
}
