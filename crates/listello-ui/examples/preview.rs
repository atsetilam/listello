//! Draw the example layout as the panel would show it, into a PNG, with no panel attached.
//!
//! ```text
//! cargo run -p listello-ui --example preview -- out.png [expanded] [fn] [controls] [charging]
//!     [previews] [style=whole|fill|stretch] [picture=FILE.png]
//! ```
//!
//! The strip is the one in `share/listello/config.example-dock.toml`, drawn at the real
//! panel's size and font size, with packaged icons from `share/listello/` and app icons from the
//! machine's icon themes. The window list is made up: three pinned apps, two Chrome windows, a
//! terminal and Telegram. `expanded` opens Chrome's window row; `fn` shows the Fn layer;
//! `controls` shows a window's close, minimise and maximise lights; `charging` plugs it in.
//! `previews` turns window previews on: the first Chrome window gets a picture (`picture=` a
//! PNG, or a made-up one) and the second the placeholder of a window with none yet.
//!
//! This is for judging looks on a desk instead of on the Touch Bar. It is not a test and it
//! asserts nothing; the goldens are what hold the looks still.

use cairo::{Context, Format, ImageSurface};
use librsvg_rebind::prelude::*;
use listello_protocol::{
    AgentToCore, AppId, BatteryState, FolderState, Icon, NetworkState, WindowId, WindowInfo,
};
use listello_surface::{packaged, Color, PixelSurface, Size, Surface, TextStyle};
use listello_ui::{IconKey, Shell, Theme, UiConfig};
use std::path::Path;
use std::rc::Rc;

const PANEL: Size = Size::new(2008, 60);

/// What `listello-core`'s `build_theme` builds, with cairo's toy font standing in for the
/// fontconfig lookup.
fn theme() -> Theme {
    let face = |weight| {
        Rc::new(
            cairo::FontFace::toy_create("Inter", cairo::FontSlant::Normal, weight)
                .expect("cairo has a toy font"),
        )
    };
    Theme {
        text: TextStyle::new(face(cairo::FontWeight::Normal), 23.0),
        background: Color::BLACK,
        foreground: Color::WHITE,
        button: Color::grey(0.2),
        button_pressed: Color::grey(0.4),
        accent: Color::rgb(0.3, 0.85, 0.4),
        corner_radius: 8.0,
    }
}

/// An app icon as the agent would send it: square, `size` pixels, straight RGBA.
fn app_icon(path: &str, size: u32) -> Option<Icon> {
    let target = ImageSurface::create(Format::ARgb32, size as i32, size as i32).ok()?;
    let cr = Context::new(&target).ok()?;
    if path.ends_with(".svg") {
        let handle = librsvg_rebind::Handle::from_file(path).ok()?;
        let viewport = librsvg_rebind::Rectangle::new(0.0, 0.0, size as f64, size as f64);
        handle.render_document(&cr, &viewport).ok()?;
    } else {
        let source = ImageSurface::create_from_png(&mut std::fs::File::open(path).ok()?).ok()?;
        let scale = size as f64 / source.width().max(source.height()) as f64;
        cr.scale(scale, scale);
        cr.set_source_surface(&source, 0.0, 0.0).ok()?;
        cr.paint().ok()?;
    }
    drop(cr);
    let mut target = target;
    let stride = target.stride() as usize;
    let data = target.data().ok()?;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size as usize {
        for x in 0..size as usize {
            let p = &data[y * stride + x * 4..][..4];
            let (b, g, r, a) = (p[0], p[1], p[2], p[3]);
            let un = |c: u8| {
                if a == 0 {
                    0
                } else {
                    (c as u32 * 255 / a as u32) as u8
                }
            };
            rgba.extend_from_slice(&[un(r), un(g), un(b), a]);
        }
    }
    Icon::new(size, size, rgba)
}

/// A window's picture as the agent would send it, made to `height` × `max_width` as `fit` says
/// (the agent's own rules, repeated). From a PNG when given one, otherwise a made-up window: a
/// light page under a dark title bar.
fn window_picture(
    path: Option<&str>,
    height: u32,
    max_width: u32,
    fit: listello_protocol::PreviewFit,
) -> Option<Icon> {
    let source = match path {
        Some(path) => ImageSurface::create_from_png(&mut std::fs::File::open(path).ok()?).ok()?,
        None => {
            let s = ImageSurface::create(Format::ARgb32, 1600, 1000).ok()?;
            let cr = Context::new(&s).ok()?;
            cr.set_source_rgb(0.95, 0.95, 0.95);
            cr.paint().ok()?;
            cr.set_source_rgb(0.2, 0.22, 0.25);
            cr.rectangle(0.0, 0.0, 1600.0, 120.0);
            cr.fill().ok()?;
            cr.set_source_rgb(0.85, 0.25, 0.2);
            cr.rectangle(80.0, 220.0, 700.0, 500.0);
            cr.fill().ok()?;
            drop(cr);
            s
        }
    };
    use listello_protocol::PreviewFit;
    let (w, h) = (source.width() as f64, source.height() as f64);
    let (bw, bh) = (max_width as f64, height as f64);
    let (ow, oh, sx, sy, dx, dy) = match fit {
        PreviewFit::Whole => {
            let s = (bh / h).min(bw / w).min(1.0);
            ((w * s).round(), (h * s).round(), s, s, 0.0, 0.0)
        }
        PreviewFit::Stretch => (bw, bh, bw / w, bh / h, 0.0, 0.0),
        PreviewFit::Crop => {
            let s = (bw / w).max(bh / h);
            (
                bw,
                bh,
                s,
                s,
                ((bw - w * s) / 2.0).round(),
                ((bh - h * s) / 2.0).round(),
            )
        }
    };
    let (ow, oh) = (ow as u32, oh as u32);
    let target = ImageSurface::create(Format::ARgb32, ow as i32, oh as i32).ok()?;
    let cr = Context::new(&target).ok()?;
    cr.translate(dx, dy);
    cr.scale(sx, sy);
    cr.set_source_surface(&source, 0.0, 0.0).ok()?;
    cr.source().set_filter(cairo::Filter::Good);
    cr.paint().ok()?;
    drop(cr);
    let mut target = target;
    let stride = target.stride() as usize;
    let data = target.data().ok()?;
    let mut rgba = Vec::new();
    for y in 0..oh as usize {
        for x in 0..ow as usize {
            let p = &data[y * stride + x * 4..][..4];
            // Opaque, so no un-premultiplying to do.
            rgba.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    Icon::new(ow, oh, rgba)
}

fn window(id: u64, app: &str, title: &str, active: bool) -> WindowInfo {
    WindowInfo {
        id: WindowId(id),
        app_id: AppId::new(app),
        title: title.to_string(),
        active,
        minimized: false,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().map(String::as_str).unwrap_or("preview.png");
    let expanded = args.iter().any(|a| a == "expanded");
    let fn_held = args.iter().any(|a| a == "fn");
    let controls = args.iter().any(|a| a == "controls");
    let charging = args.iter().any(|a| a == "charging");
    let previews = args.iter().any(|a| a == "previews");
    let picture = args.iter().find_map(|a| a.strip_prefix("picture="));

    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let text = std::fs::read_to_string(root.join("share/listello/config.example-dock.toml"))
        .expect("the example layout is readable");
    let cfg = UiConfig::parse(&text).expect("the example layout parses");
    let now = chrono::DateTime::parse_from_rfc3339("2026-10-22T20:48:00+02:00").unwrap();
    let mut shell = Shell::new(&cfg, theme(), now);
    for (name, image) in packaged::load_icons(&root.join("share/listello"), &cfg.icon_names()) {
        shell.state_mut().icons.insert(IconKey::Named(name), image);
    }

    shell.set_agent_connected(true);
    let apps = [
        (
            "org.kde.dolphin",
            "/usr/share/icons/hicolor/scalable/apps/org.kde.dolphin.svg",
        ),
        (
            "google-chrome",
            "/usr/share/icons/hicolor/256x256/apps/google-chrome.png",
        ),
        (
            "org.kde.konsole",
            "/usr/share/icons/breeze/apps/48/utilities-terminal.svg",
        ),
        (
            "org.telegram.desktop",
            "/usr/share/icons/hicolor/256x256/apps/org.telegram.desktop.png",
        ),
    ];
    for (app, path) in apps {
        match app_icon(path, 40) {
            Some(icon) => {
                shell.apply_agent(&AgentToCore::Icon {
                    app_id: AppId::new(app),
                    icon: Some(icon),
                });
            }
            None => eprintln!("no icon for {app} at {path}"),
        }
    }
    shell.apply_agent(&AgentToCore::Windows(vec![
        window(
            1,
            "google-chrome",
            "Inbox (3) - someone@example.com - Gmail - Google Chrome",
            false,
        ),
        window(
            2,
            "google-chrome",
            "listello: the Touch Bar for Linux - Google Chrome",
            false,
        ),
        window(3, "org.kde.konsole", "~/listello : fish — Konsole", true),
        window(4, "org.telegram.desktop", "Telegram", false),
    ]));
    shell.apply_agent(&AgentToCore::Battery(Some(BatteryState {
        charge: if charging { 1.0 } else { 0.89 },
        charging,
        time_remaining_secs: None,
    })));
    shell.apply_agent(&AgentToCore::Network(Some(NetworkState {
        connected: true,
        signal: 0.9,
        ssid: None,
    })));
    shell.apply_agent(&AgentToCore::Folder(FolderState {
        key: "trash".into(),
        empty: false,
    }));
    if expanded {
        shell.state_mut().expanded = Some(AppId::new("google-chrome"));
    }
    if previews {
        let style = match args.iter().find_map(|a| a.strip_prefix("style=")) {
            Some("whole") => listello_ui::PreviewStyle::Whole,
            Some("stretch") => listello_ui::PreviewStyle::Stretch,
            _ => listello_ui::PreviewStyle::Fill,
        };
        shell.state_mut().window_previews = Some(style);
        if let Some(listello_protocol::CoreToAgent::WatchPreviews {
            height_px,
            max_width_px,
            fit,
            ..
        }) = shell.preview_request(PANEL)
        {
            let image = window_picture(picture, height_px, max_width_px, fit);
            shell.apply_agent(&AgentToCore::Preview {
                id: WindowId(1),
                image,
            });
        }
    }
    shell.state_mut().fn_held = fn_held;
    if controls {
        // The terminal's controls, or the second Chrome window's in the expanded row, fully out
        // and mid-wiggle.
        let (app, window) = if expanded {
            ("google-chrome", 2)
        } else {
            ("org.kde.konsole", 3)
        };
        shell.state_mut().controls = Some(listello_ui::WindowControls {
            app: AppId::new(app),
            window: Some(WindowId(window)),
            age: 1.06,
        });
    }

    let mut surface = PixelSurface::new(PANEL).expect("a pixel surface");
    surface
        .present(&shell.frame(PANEL))
        .expect("the frame draws");
    surface.write_png(out).expect("the PNG writes");
    eprintln!("wrote {out}");
}
