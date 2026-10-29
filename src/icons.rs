// SPDX-License-Identifier: GPL-3.0-only

//! Icons drawn here rather than looked up: the icon theme lacks some
//! (dark style, keep awake, power profiles), and its battery reads poorly at
//! panel size. Symbolic: drawn in one color, which the theme then tints.

use cosmic::widget::icon;

fn svg(body: &str) -> icon::Handle {
    svg_with("", body)
}

fn svg_with(defs: &str, body: &str) -> icon::Handle {
    let text = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">{defs}<g fill="#000">{body}</g></svg>"##
    );
    icon::from_svg_bytes(text.into_bytes()).symbolic(true)
}

/// A battery filled as charged; while charging, a bolt with a clear gap
/// around it, which reads on a full battery and an empty one alike.
pub fn battery(percent: f64, charging: bool) -> icon::Handle {
    const BOLT: &str = "M8.6 3.6 4.2 8.7h2.9l-1 3.9 4.6-5.6H7.7z";
    // The inside spans x 2.6..13.1.
    let fill = (percent.clamp(0.0, 100.0) / 100.0 * 10.5).max(1.0);
    let shape = format!(
        r#"<path fill-rule="evenodd" d="M2 3.5h11a1.5 1.5 0 0 1 1.5 1.5v6a1.5 1.5 0 0 1-1.5 1.5H2A1.5 1.5 0 0 1 .5 11V5A1.5 1.5 0 0 1 2 3.5zm0 1.3a.2.2 0 0 0-.2.2v6c0 .1.1.2.2.2h11a.2.2 0 0 0 .2-.2V5a.2.2 0 0 0-.2-.2z"/><rect x="15" y="6.2" width="1" height="3.6" rx=".5"/><rect x="2.6" y="5.6" width="{fill:.2}" height="4.8" rx=".6"/>"#
    );
    if !charging {
        return svg(&shape);
    }
    svg_with(
        &format!(
            r##"<defs><mask id="gap"><rect width="16" height="16" fill="#fff"/><path d="{BOLT}" fill="#000" stroke="black" stroke-width="1.8" stroke-linejoin="round"/></mask></defs>"##
        ),
        &format!(r#"<g mask="url(#gap)">{shape}</g><path d="{BOLT}"/>"#),
    )
}

pub fn dark_style() -> icon::Handle {
    svg(r#"<path d="M6.2 1.3a6.8 6.8 0 1 0 8.5 8.5A5.6 5.6 0 0 1 6.2 1.3z"/>"#)
}

pub fn keep_awake() -> icon::Handle {
    // A cup of coffee, steaming.
    svg(r#"<path d="M1.5 6h10v4a4 4 0 0 1-4 4h-2a4 4 0 0 1-4-4zm10 1h1a2.5 2.5 0 0 1 0 5h-1.2l.4-1.4h.8a1.1 1.1 0 0 0 0-2.2h-1z"/><path d="M4 1c.8.8.8 1.7 0 2.5s-.8 1.7 0 2.5M7 1c.8.8.8 1.7 0 2.5s-.8 1.7 0 2.5" fill="none" stroke="black" stroke-width="1.1" stroke-linecap="round"/>"#)
}

/// A gauge, its needle at the profile's place.
pub fn power_profile(profile: &str) -> icon::Handle {
    let needle = match profile {
        "performance" => r#"<path d="m8 10 4.6-4.4" stroke="black" stroke-width="1.8" stroke-linecap="round"/>"#,
        "power-saver" => r#"<path d="M8 10 3.4 5.6" stroke="black" stroke-width="1.8" stroke-linecap="round"/>"#,
        _ => r#"<path d="M8 10V3.8" stroke="black" stroke-width="1.8" stroke-linecap="round"/>"#,
    };
    svg(&format!(
        r#"<path fill-rule="evenodd" d="M8 2a7 7 0 0 0-6.1 10.4l.3.6h11.6l.3-.6A7 7 0 0 0 8 2zm0 1.5a5.5 5.5 0 0 0-4.9 8h9.8A5.5 5.5 0 0 0 8 3.5z"/><circle cx="8" cy="10" r="1.6"/>{needle}"#
    ))
}
