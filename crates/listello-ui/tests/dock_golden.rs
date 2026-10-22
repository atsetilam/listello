//! The Dock as pixels: pinned and running apps with their dots and a divider, an overflowing
//! strip scrolled part way, and one app expanded into its window entries, with and without
//! window previews.
//!
//! Like `strip_golden`, nothing here draws text: the icons are solid squares sent the way the
//! agent would send them, and every `Text` command is dropped before rasterising (the expanded
//! row's "2 windows" would otherwise be one), so the images do not depend on which font
//! resolved on which machine. The text is asserted on the command list in `dock.rs`.
//!
//! To regenerate after a deliberate rendering change:
//!
//! ```text
//! LISTELLO_GOLDEN_UPDATE=1 cargo test -p listello-ui --test dock_golden
//! ```

use listello_protocol::{AppId, Icon, WindowId, WindowInfo};
use listello_surface::draw::{Color, DrawCmd, Frame};
use listello_surface::geom::Size;
use listello_surface::golden::assert_matches;
use listello_surface::pixel::PixelSurface;
use listello_surface::{Surface, TextStyle};
use listello_ui::{render_strip, DockWidget, Sizing, Theme, UiState, Widget};
use std::rc::Rc;

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

fn window(id: u64, app: &str, active: bool) -> WindowInfo {
    WindowInfo {
        id: WindowId(id),
        app_id: AppId::new(app),
        title: String::new(),
        active,
        minimized: false,
    }
}

/// A solid 40x40 icon whose colour is picked by the app's name, so tiles are tellable apart.
fn icon_for(app: &str) -> Icon {
    let hue = app.bytes().fold(7u32, |h, b| {
        h.wrapping_mul(2654435761).wrapping_add(b as u32)
    });
    // Scramble, so that names differing in one letter still differ in colour.
    let hue = (hue ^ (hue >> 15)).wrapping_mul(2246822519);
    let hue = hue ^ (hue >> 13);
    let channel = |shift: u32| 60 + ((hue >> shift) % 190) as u8;
    let (r, g, b) = (channel(0), channel(8), channel(16));
    let rgba: Vec<u8> = (0..40 * 40).flat_map(|_| [r, g, b, 255]).collect();
    Icon::new(40, 40, rgba).expect("well-formed icon")
}

fn state(windows: Vec<WindowInfo>, apps: &[&str]) -> UiState {
    let mut state =
        UiState::new(chrono::DateTime::parse_from_rfc3339("2026-10-22T09:00:00+02:00").unwrap());
    state.agent_connected = true;
    for app in apps {
        state.icons.insert_app(AppId::new(*app), &icon_for(app));
    }
    state.windows = windows;
    state
}

fn check(name: &str, dock: &DockWidget, state: &UiState) {
    let Some(theme) = theme() else {
        eprintln!("skipping: cairo could not create the toy font");
        return;
    };
    let size = Size::new(400, 60);
    let widgets: [&dyn Widget; 1] = [dock];
    let out = render_strip(&widgets, state, &theme, size.to_rect(), 8.0);
    let mut commands = vec![DrawCmd::Clear(Color::BLACK)];
    commands.extend(
        out.commands()
            .iter()
            .filter(|c| !matches!(c, DrawCmd::Text { .. }))
            .cloned(),
    );
    let mut surface = PixelSurface::new(size).unwrap();
    surface.present(&Frame::full(commands)).unwrap();
    let path = format!("{}/tests/golden/{name}.png", env!("CARGO_MANIFEST_DIR"));
    assert_matches(path, &mut surface).unwrap();
}

fn dock(pinned: &[&str]) -> DockWidget {
    DockWidget {
        pinned: pinned.iter().map(|p| AppId::new(*p)).collect(),
        sizing: Sizing::Flex(1.0),
    }
}

#[test]
fn pinned_and_running_apps_with_a_divider_and_dots() {
    let apps = ["files", "browser", "mail", "terminal"];
    let s = state(
        vec![window(1, "browser", false), window(2, "terminal", true)],
        &apps,
    );
    check("dock_collapsed", &dock(&["files", "browser", "mail"]), &s);
}

#[test]
fn an_overflowing_dock_scrolled_part_way_is_clipped_at_both_ends() {
    let names: Vec<String> = (0..9).map(|i| format!("app{i}")).collect();
    let apps: Vec<&str> = names.iter().map(String::as_str).collect();
    let windows = names
        .iter()
        .enumerate()
        .map(|(i, n)| window(i as u64, n, i == 4))
        .collect();
    let mut s = state(windows, &apps);
    s.scroll.insert(0, 150.0);
    check("dock_scrolled", &dock(&[]), &s);
}

#[test]
fn one_app_expanded_into_its_window_entries() {
    let apps = ["editor", "browser"];
    let mut s = state(
        vec![
            window(1, "editor", false),
            window(2, "editor", true),
            window(3, "browser", false),
        ],
        &apps,
    );
    s.expanded = Some(AppId::new("editor"));
    check("dock_expanded", &dock(&[]), &s);
}

#[test]
fn an_expanded_row_with_window_previews_one_arrived_one_not_yet() {
    let apps = ["editor"];
    let mut s = state(
        vec![window(1, "editor", false), window(2, "editor", true)],
        &apps,
    );
    s.expanded = Some(AppId::new("editor"));
    s.window_previews = Some(listello_ui::PreviewStyle::Whole);
    // A wide window's picture: a light page under a dark bar, as the agent would fit it.
    let (w, h) = (64u32, 40u32);
    let rgba: Vec<u8> = (0..w * h)
        .flat_map(|i| {
            if i / w < 6 {
                [40, 40, 50, 255]
            } else {
                [230, 230, 230, 255]
            }
        })
        .collect();
    let picture = Icon::new(w, h, rgba).unwrap();
    s.previews.insert(
        WindowId(1),
        listello_surface::image_from_rgba(w, h, &picture.rgba).unwrap(),
    );
    check("dock_expanded_previews", &dock(&[]), &s);
}

#[test]
fn an_expanded_row_with_a_picture_filling_one_entry_and_the_other_still_plain() {
    let apps = ["editor"];
    let mut s = state(
        vec![window(1, "editor", false), window(2, "editor", true)],
        &apps,
    );
    s.expanded = Some(AppId::new("editor"));
    s.window_previews = Some(listello_ui::PreviewStyle::Fill);
    // Wider than the entry, as the agent sends it: the entry keeps its middle.
    let (w, h) = (374u32, 54u32);
    let rgba: Vec<u8> = (0..w * h)
        .flat_map(|i| {
            if i % w < w / 2 {
                [230, 230, 230, 255]
            } else {
                [200, 60, 50, 255]
            }
        })
        .collect();
    let picture = Icon::new(w, h, rgba).unwrap();
    s.previews.insert(
        WindowId(1),
        listello_surface::image_from_rgba(w, h, &picture.rgba).unwrap(),
    );
    check("dock_expanded_fill", &dock(&[]), &s);
}
