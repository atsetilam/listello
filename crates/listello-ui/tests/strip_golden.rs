//! The first pixel proof of the whole M1 chain: a strip of button widgets, laid out by
//! `render_strip`, rasterised into a `PixelSurface`, compared against a PNG committed at
//! `tests/golden/ui_strip.png`.
//!
//! The three buttons are deliberately textless. A golden containing rendered text would depend
//! on which font resolved and which freetype rasterised it on each machine this repo gates on
//! (macOS, Linux x86_64, Linux aarch64) and would fail somewhere for a reason that
//! has nothing to do with this code. Geometry and colour are identical everywhere, so that is
//! what the pixel fixture covers; the label path is asserted at the draw-command level in
//! `widgets.rs` instead. This is a rule for the project, not a quirk of this test.
//!
//! To regenerate `ui_strip.png` after a deliberate rendering change:
//!
//! ```text
//! LISTELLO_GOLDEN_UPDATE=1 cargo test -p listello-ui --test strip_golden
//! ```

use listello_protocol::SystemAction;
use listello_surface::draw::{Color, DrawCmd, Frame};
use listello_surface::geom::Size;
use listello_surface::golden::assert_matches;
use listello_surface::image_from_rgba;
use listello_surface::pixel::PixelSurface;
use listello_surface::Surface;
use listello_surface::TextStyle;
use listello_ui::widgets::ControlButton;
use listello_ui::{
    render_strip, Action, ButtonWidget, ControlCenterWidget, IconKey, KeyName, RegionId, Sizing,
    Theme, UiState, Widget,
};
use std::rc::Rc;

/// A theme with distinguishable colours, built from cairo's toy font so it needs no fontconfig.
/// Buttons here have empty labels, so the face is never actually rasterised.
fn theme() -> Option<Theme> {
    let font = cairo::FontFace::toy_create(
        "sans-serif",
        cairo::FontSlant::Normal,
        cairo::FontWeight::Normal,
    )
    .ok()?;
    Some(Theme {
        text: TextStyle::new(Rc::new(font), 14.0),
        background: Color::BLACK,
        foreground: Color::WHITE,
        button: Color::grey(0.25),
        button_pressed: Color::grey(0.6),
        accent: Color::grey(0.8),
        corner_radius: 8.0,
    })
}

#[test]
fn strip_matches_the_committed_golden_image() {
    let Some(theme) = theme() else {
        eprintln!("skipping: cairo could not create the toy font");
        return;
    };
    let mut state =
        UiState::new(chrono::DateTime::parse_from_rfc3339("2026-08-03T14:05:00+02:00").unwrap());
    // The middle button (widget index 1) is held down, so the image proves the pressed path
    // rather than only the idle one.
    state.pressed = Some(RegionId::new(1, 0));

    let bounds = Size::new(320, 60).to_rect();
    let buttons = [
        ButtonWidget {
            label: String::new(),
            icon: None,
            action: Action::None,
            sizing: Sizing::Flex(1.0),
        },
        ButtonWidget {
            label: String::new(),
            icon: None,
            action: Action::None,
            sizing: Sizing::Fixed(60.0),
        },
        ButtonWidget {
            label: String::new(),
            icon: None,
            action: Action::None,
            sizing: Sizing::Flex(1.0),
        },
    ];
    let refs: Vec<&dyn Widget> = buttons.iter().map(|b| b as &dyn Widget).collect();
    let out = render_strip(&refs, &state, &theme, bounds, 8.0);

    // A real frame begins with a clear, so the strip is painted over it the same way a frame
    // presented to a Surface would be.
    let mut commands = vec![DrawCmd::Clear(Color::BLACK)];
    commands.extend(out.commands().iter().cloned());

    let mut surface = PixelSurface::new(Size::new(320, 60)).unwrap();
    surface.present(&Frame::full(commands)).unwrap();

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/ui_strip.png");
    assert_matches(path, &mut surface).unwrap();
}

#[test]
fn control_center_six_button_row_matches_the_committed_golden_image() {
    let Some(theme) = theme() else {
        eprintln!("skipping: cairo could not create the toy font");
        return;
    };
    let mut state =
        UiState::new(chrono::DateTime::parse_from_rfc3339("2026-08-03T14:05:00+02:00").unwrap());

    // One distinct solid-colour icon per button, built in memory so no machine's icon theme and
    // no font stack can leak into the fixture — the same rule the strip golden above follows.
    // The usual set, in order: lock, brightness down/up, mute, volume down/up.
    let row: [(&str, Action, [u8; 3]); 6] = [
        (
            "lock",
            Action::System(SystemAction::LockSession),
            [0xe0, 0x30, 0x30],
        ),
        (
            "brightness-down",
            Action::Keys(vec![KeyName::new("KEY_BRIGHTNESSDOWN").unwrap()]),
            [0xe0, 0x80, 0x20],
        ),
        (
            "brightness-up",
            Action::Keys(vec![KeyName::new("KEY_BRIGHTNESSUP").unwrap()]),
            [0xd0, 0xc0, 0x20],
        ),
        (
            "mute",
            Action::Keys(vec![KeyName::new("KEY_MUTE").unwrap()]),
            [0x30, 0xc0, 0x40],
        ),
        (
            "volume-down",
            Action::Keys(vec![KeyName::new("KEY_VOLUMEDOWN").unwrap()]),
            [0x20, 0xa0, 0xc0],
        ),
        (
            "volume-up",
            Action::Keys(vec![KeyName::new("KEY_VOLUMEUP").unwrap()]),
            [0x40, 0x50, 0xe0],
        ),
    ];

    let buttons: Vec<ControlButton> = row
        .iter()
        .map(|(icon, action, _)| ControlButton {
            icon: (*icon).to_string(),
            label: None,
            action: action.clone(),
        })
        .collect();
    for (icon, _, [r, g, b]) in &row {
        let rgba: Vec<u8> = (0..16 * 16).flat_map(|_| [*r, *g, *b, 0xff]).collect();
        if let Some(surface) = image_from_rgba(16, 16, &rgba) {
            state
                .icons
                .insert(IconKey::Named((*icon).to_string()), surface);
        }
    }

    let widget = ControlCenterWidget {
        buttons,
        sizing: Sizing::Fixed(260.0),
    };
    let refs: Vec<&dyn Widget> = vec![&widget as &dyn Widget];
    // Six 40px buttons and five 4px internal gaps make 260; render the whole strip so the
    // command list is what a real frame would carry.
    let bounds = Size::new(260, 60).to_rect();
    let out = render_strip(&refs, &state, &theme, bounds, 8.0);

    let mut commands = vec![DrawCmd::Clear(Color::BLACK)];
    commands.extend(out.commands().iter().cloned());

    let mut surface = PixelSurface::new(Size::new(260, 60)).unwrap();
    surface.present(&Frame::full(commands)).unwrap();

    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/golden/control_center_six_buttons.png"
    );
    assert_matches(path, &mut surface).unwrap();
}

#[test]
fn battery_bar_at_sixty_two_percent_matches_the_committed_golden_image() {
    let Some(theme) = theme() else {
        eprintln!("skipping: cairo could not create the toy font");
        return;
    };
    let mut state =
        UiState::new(chrono::DateTime::parse_from_rfc3339("2026-08-03T14:05:00+02:00").unwrap());
    state.battery = Some(listello_protocol::BatteryState {
        charge: 0.62,
        charging: false,
        time_remaining_secs: None,
    });

    // The bar only: the percentage is text, and text stays out of goldens (see the header).
    let widget = listello_ui::BatteryWidget {
        show_percentage: false,
        show_bar: true,
        sizing: Sizing::Fixed(60.0),
    };
    let refs: Vec<&dyn Widget> = vec![&widget as &dyn Widget];
    let bounds = Size::new(60, 60).to_rect();
    let out = render_strip(&refs, &state, &theme, bounds, 8.0);

    let mut commands = vec![DrawCmd::Clear(Color::BLACK)];
    commands.extend(out.commands().iter().cloned());

    let mut surface = PixelSurface::new(Size::new(60, 60)).unwrap();
    surface.present(&Frame::full(commands)).unwrap();

    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/golden/battery_sixty_two_percent.png"
    );
    assert_matches(path, &mut surface).unwrap();
}

#[test]
fn three_folders_with_an_empty_trash_match_the_committed_golden_image() {
    let Some(theme) = theme() else {
        eprintln!("skipping: cairo could not create the toy font");
        return;
    };
    let mut state =
        UiState::new(chrono::DateTime::parse_from_rfc3339("2026-08-03T14:05:00+02:00").unwrap());
    state.folders.insert(
        "trash".into(),
        listello_protocol::FolderState {
            key: "trash".into(),
            empty: true,
        },
    );

    // Solid-colour icons built in memory, as in the control-centre golden. Trash is empty, so
    // it must wear the lighter "trash-empty" square rather than the dark "trash-full" one.
    for (name, [r, g, b]) in [
        ("screenshots", [0x40, 0x90, 0xe0]),
        ("downloads", [0x30, 0xc0, 0x60]),
        ("trash-full", [0x60, 0x30, 0x30]),
        ("trash-empty", [0xe0, 0xe0, 0xe0]),
    ] {
        let rgba: Vec<u8> = (0..16 * 16).flat_map(|_| [r, g, b, 0xff]).collect();
        if let Some(surface) = image_from_rgba(16, 16, &rgba) {
            state.icons.insert(IconKey::Named(name.into()), surface);
        }
    }

    let entry = |key: &str, icon: &str, empty_icon: Option<&str>| listello_ui::FolderEntry {
        key: key.into(),
        path: format!("/home/g/{key}"),
        label: None,
        icon: icon.into(),
        empty_icon: empty_icon.map(str::to_string),
    };
    let widget = listello_ui::FoldersWidget {
        folders: vec![
            entry("screenshots", "screenshots", None),
            entry("downloads", "downloads", None),
            entry("trash", "trash-full", Some("trash-empty")),
        ],
        sizing: Sizing::Fixed(200.0),
    };
    let refs: Vec<&dyn Widget> = vec![&widget as &dyn Widget];
    let bounds = Size::new(200, 60).to_rect();
    let out = render_strip(&refs, &state, &theme, bounds, 8.0);

    let mut commands = vec![DrawCmd::Clear(Color::BLACK)];
    commands.extend(out.commands().iter().cloned());

    let mut surface = PixelSurface::new(Size::new(200, 60)).unwrap();
    surface.present(&Frame::full(commands)).unwrap();

    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/golden/folders_three.png"
    );
    assert_matches(path, &mut surface).unwrap();
}
