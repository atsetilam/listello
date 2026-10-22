//! The example layout, drawn at the real panel's font size with the fullest state each widget
//! can show: no text may be wider than the box it is drawn in, and nothing may stick out past
//! the panel. Written after the battery's "%" was clipped twice by a change that was harmless
//! on its own — a layout edit that breaks this fails here, not on someone's Touch Bar.

use listello_protocol::{AgentToCore, BatteryState, FolderState, NetworkState};
use listello_surface::draw::{Color, DrawCmd};
use listello_surface::geom::Size;
use listello_surface::{measure_text, packaged, TextStyle};
use listello_ui::{IconKey, Shell, Theme, UiConfig};
use std::rc::Rc;

const PANEL: Size = Size::new(2008, 60);
/// The size the daemon draws text at (`FONT_SIZE` in listello-core).
const FONT_SIZE: f64 = 23.0;

#[test]
fn every_text_in_the_example_layout_fits_its_box_and_the_panel() {
    // The daemon's face (`FONT_TEMPLATE`); fontconfig substitutes when Inter is not installed.
    let Ok(font) =
        cairo::FontFace::toy_create("Inter", cairo::FontSlant::Normal, cairo::FontWeight::Normal)
    else {
        eprintln!("skipping: cairo could not create the toy font");
        return;
    };
    let theme = Theme {
        text: TextStyle::new(Rc::new(font), FONT_SIZE),
        background: Color::BLACK,
        foreground: Color::WHITE,
        button: Color::grey(0.25),
        button_pressed: Color::grey(0.45),
        accent: Color::grey(0.8),
        corner_radius: 10.0,
    };
    let text = include_str!("../../../share/listello/config.example-dock.toml");
    let cfg = UiConfig::parse(text).expect("the example parses");
    // A wide date: a three-letter day and month with wide letters, and two-digit numbers.
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-30T20:48:00+02:00").unwrap();

    // With the packaged icons loaded, as installed. (With none, folder and button labels
    // replace the icons and are wider than their boxes; the per-slot clip contains that, and it
    // is a degraded state, not the one this guards.)
    for with_icons in [true] {
        let mut shell = Shell::new(&cfg, theme.clone(), now);
        if with_icons {
            let share =
                std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../share/listello"));
            for (name, image) in packaged::load_icons(share, &cfg.icon_names()) {
                shell.state_mut().icons.insert(IconKey::Named(name), image);
            }
        }
        shell.set_agent_connected(true);
        shell.apply_agent(&AgentToCore::Battery(Some(BatteryState {
            charge: 1.0,
            charging: true,
            time_remaining_secs: None,
        })));
        shell.apply_agent(&AgentToCore::Network(Some(NetworkState {
            connected: true,
            signal: 1.0,
            ssid: None,
        })));
        shell.apply_agent(&AgentToCore::Folder(FolderState {
            key: "trash".into(),
            empty: false,
        }));

        let frame = shell.frame(PANEL);
        // Full and charging is the widest the battery gets: its bar, with the bolt on it, must
        // still be there next to "100%", not given up for room.
        assert!(
            frame.commands.iter().any(|c| matches!(
                c,
                DrawCmd::Image { rect, .. } if rect.width == 22.0 && rect.height == 22.0
            )),
            "the charging battery lost its bar"
        );
        for cmd in &frame.commands {
            let DrawCmd::Text {
                rect, text, style, ..
            } = cmd
            else {
                continue;
            };
            let needed = measure_text(style, text).advance;
            assert!(
                needed <= rect.width + 0.5,
                "icons={with_icons}: {text:?} needs {needed:.1}px but its box is {:.1}px",
                rect.width
            );
            assert!(
                rect.x >= 0.0 && rect.right() <= PANEL.width as f64 + 0.5,
                "icons={with_icons}: {text:?} sticks out of the panel: {rect:?}"
            );
        }
    }
}
