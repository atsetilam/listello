# Coming from Pock

listello does for the Touch Bar on Linux what Pock does on macOS: a Dock, status widgets and
controls, with the function row one finger away. It is a reimplementation of Pock's
behaviour, not a port of its code, so it is not identical. This page says what carries over
and what does not.

Install steps are in [INSTALL.md](INSTALL.md).

## Where Pock's widgets went

| In Pock | In listello |
|---|---|
| Dock — running and pinned apps, tap to focus or launch | `kind = "dock"`, with `pinned = [...]` as the starting set; pins added or removed from the Touch Bar are kept per user in `~/.config/listello/pins.toml`, which from then on replaces that list. The tile of an app that is starting hops until its window opens. |
| Dock — several windows of one app | Tapping the tile opens a row of the windows' titles, which takes the whole Touch Bar until you pick one or tap the back tile. With `[strip] window_previews = "snapshots"` or `"live"` each entry also shows a picture of its window, kept fresh while the row is open (see below). |
| Control Center — brightness, volume | `kind = "control_center"`, with the buttons you list. They are buttons (dim, bright, mute, volume down, volume up, lock), not sliders. |
| Status — battery, wifi, clock | `kind = "battery"`, `kind = "network"`, `kind = "clock"`. The clock's `format` is a strftime string, so it shows any mix of weekday, day, month, year and 12- or 24-hour time; `locale = "it_IT"` (or any other) sets the language of the names. |
| Escape key | Not needed on a MacBook Pro with a physical Esc. Not provided. |
| (not in Pock) | Hold an open app's icon (or a window in its row of windows) for half a second: it wiggles and close, minimise and maximise buttons slide out beside it, with a pin that fixes the app to the Dock or, if it is fixed already, takes it off. Holding a fixed app that is not running shows only the pin. While the buttons are up the Dock has the whole Touch Bar: the other widgets step aside and the other apps fade. Close does what the window's own close button does. `[strip] hold_seconds` and `controls_seconds` set the hold and how long the buttons stay up. |
| (not in Pock) | `kind = "folders"`: one-tap shortcuts to Downloads, screenshots, Trash and so on; the Trash icon shows full or empty. |
| Now Playing | Not in 1.0. |
| Weather, third-party `.pock` widgets | Not in 1.0. There is no plugin system; every widget is part of listello. |
| Window previews on the Dock | Optional, off unless turned on: `[strip] window_previews = "snapshots"` or `"live"`. On KDE Plasma the installed agent may capture windows without a prompt. `preview_style` picks how the picture sits in a window's entry: `"fill"` (a band through the middle of the window behind its title, the default), `"whole"` (the whole window, small, beside the title) or `"stretch"` (the whole window squeezed behind the title). Pictures are taken when the row opens, every 1.5 s for 10 s, then every 5 s, and not at all after a minute. With `"live"` each window is a video stream instead, at most ten pictures a second and only when the window changes, for as long as the row is open; a window that cannot be streamed gets the pictures above. Nothing is captured while the row is closed. Other desktops show titles only. |
| Holding Fn | Same: the whole strip is replaced by F1–F12 while Fn is held. |

## Where Pock keeps its settings, and where listello does

Pock has a preferences window, and so does listello: "listello" in the application menu,
with a picture of the strip you can click and drag. Underneath it is a text file, per user in
`~/.config/listello/config.toml` (the machine-wide default is `/etc/listello/config.toml`),
reloaded when saved, if you would rather edit it by hand. The order of widgets in the file is the
order on the bar, left to right. `width` fixes a widget's size in pixels; `flex` shares out
whatever is left.

`/usr/share/listello/config.example-dock.toml` is a layout close to Pock's default: the Dock
on the left taking the free space, then folders, controls, wifi and battery.

## Differences you will notice

- **It needs KDE Plasma on Wayland for the Dock.** Other desktops are not supported yet.
- **Pinned apps are named by their desktop-file id** (`org.kde.dolphin`, `firefox`), not by a
  drag from Finder. `ls /usr/share/applications` lists them.
