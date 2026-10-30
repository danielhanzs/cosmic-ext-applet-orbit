// SPDX-License-Identifier: GPL-3.0-only

//! Colors that read: on a bright accent (lime green, yellow…) the theme's
//! white text disappears, so lit surfaces pick black or white by contrast.

use cosmic::{
    iced::{Background, Color},
    theme,
    widget::button,
};

/// How bright a color looks, as WCAG measures it.
fn luminance(c: Color) -> f32 {
    let linear = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
}

/// Black or white, whichever stands out more on `background`.
pub fn readable(background: Color) -> Color {
    let l = luminance(background);
    // Contrast with black is (l + 0.05) / 0.05, with white 1.05 / (l + 0.05).
    if (l + 0.05) * (l + 0.05) > 0.05 * 1.05 {
        Color::from_rgb(0.07, 0.07, 0.07)
    } else {
        Color::WHITE
    }
}

#[derive(Clone, Copy)]
enum State {
    Active,
    Hovered,
    Pressed,
}

fn surface(theme: &cosmic::Theme, lit: bool, state: State, radius: bool) -> button::Style {
    let cosmic = theme.cosmic();
    let component = if lit { &cosmic.accent_button } else { &cosmic.button };
    let background: Color = match state {
        State::Active => component.base,
        State::Hovered => component.hover,
        State::Pressed => component.pressed,
    }
    .into();
    let text = if lit { readable(component.base.into()) } else { component.on.into() };
    let mut style = button::Style::new();
    style.background = Some(Background::Color(background));
    style.text_color = Some(text);
    style.icon_color = Some(text);
    if radius {
        style.border_radius = cosmic.corner_radii.radius_m.into();
    }
    style
}

fn custom(f: impl Fn(&cosmic::Theme, State) -> button::Style + Clone + 'static) -> theme::Button {
    let (a, d, h, p) = (f.clone(), f.clone(), f.clone(), f);
    theme::Button::Custom {
        active: Box::new(move |_, t| a(t, State::Active)),
        disabled: Box::new(move |t| d(t, State::Active)),
        hovered: Box::new(move |_, t| h(t, State::Hovered)),
        pressed: Box::new(move |_, t| p(t, State::Pressed)),
    }
}

/// A tile: the accent when on, the plain button color when off.
pub fn tile(on: bool) -> theme::Button {
    custom(move |t, s| surface(t, on, s, true))
}

/// A picked choice (a calendar day, a layout): the accent, readable, as
/// round as a hovered line.
pub fn picked() -> theme::Button {
    custom(|t, s| {
        let mut style = surface(t, true, s, false);
        style.border_radius = t.cosmic().corner_radii.radius_s.into();
        style
    })
}

/// A whole line that lights on hover, edge to edge (the calendar's head).
pub fn flat() -> theme::Button {
    custom(|t, s| {
        let cosmic = t.cosmic();
        let mut style = button::Style::new();
        style.background = match s {
            State::Active => None,
            State::Hovered => Some(Background::Color(cosmic.text_button.hover.into())),
            State::Pressed => Some(Background::Color(cosmic.text_button.pressed.into())),
        };
        style.text_color = Some(cosmic.background(t.transparent).on.into());
        style.icon_color = style.text_color;
        style.border_radius = cosmic.corner_radii.radius_s.into();
        style
    })
}

/// An icon button inside a tile: no surface of its own, the tile's text color.
pub fn on_tile(on: bool) -> theme::Button {
    custom(move |t, s| {
        let mut style = surface(t, on, s, true);
        let text = style.text_color.unwrap_or(Color::WHITE);
        style.background = match s {
            State::Active => None,
            _ => Some(Background::Color(text.scale_alpha(0.15))),
        };
        style
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_contrast() {
        // Lime green and yellow take black; dark blue and red take white.
        assert_eq!(readable(Color::from_rgb(0.33, 1.0, 0.0)), Color::from_rgb(0.07, 0.07, 0.07));
        assert_eq!(readable(Color::from_rgb(1.0, 0.9, 0.2)), Color::from_rgb(0.07, 0.07, 0.07));
        assert_eq!(readable(Color::from_rgb(0.1, 0.2, 0.6)), Color::WHITE);
        assert_eq!(readable(Color::from_rgb(0.75, 0.1, 0.1)), Color::WHITE);
    }
}
