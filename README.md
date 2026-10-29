# listello

**The Touch Bar, made useful again — on Linux.** A widget manager for the MacBook Pro Touch
Bar: a Dock with your apps and windows, folders, a control centre, Wi-Fi, battery and a clock,
all in the strip above your keyboard. Hold Fn and you get F1–F12, as you always did. Built on
tiny-dfr and inspired by Pock.

![The listello strip: the Dock, folders, control centre, Wi-Fi, battery and clock](docs/strip.png)

A *listello* (*lee-STEL-loh*) is a thin decorative strip — in tiling, the narrow border
course.

## Built on tiny-dfr

listello is a fork of **[tiny-dfr](https://github.com/AsahiLinux/tiny-dfr)**, by
**The Asahi Linux Contributors**, and would not exist without it.

tiny-dfr solved the genuinely hard, hardware-specific half of the job: opening the Touch
Bar's display and drawing to it, reading its touch input, driving the Touch Bar and display
backlights, protecting the panel from burn-in, dropping privileges, and doing all of it on both
T2 Intel Macs and Apple Silicon under Asahi Linux. listello keeps that work substantially
unchanged and builds a widget system on top. Files retained from tiny-dfr say so in a header,
and tiny-dfr's git history is preserved in this repository.

tiny-dfr's maintainers have asked that AI agents not take part in their repository, and
listello respects that by staying out of it entirely: it sends nothing back, and **bugs in
listello belong here, not there** (see [CONTRIBUTING](CONTRIBUTING.md)). If all you want is a
reliable function row, tiny-dfr itself is smaller, simpler, packaged in most distributions, and
may be exactly what you need.

## Inspired by Pock

What listello draws is **[Pock](https://github.com/pock/pock)**'s idea, by
**[Pierluigi Galdi](https://github.com/pigigaldi)**: the macOS app that turned the Touch Bar
from a curiosity into the most useful strip on the laptop. I am probably one of the very few
people who loved the Touch Bar, and that is entirely Pock's doing — Apple should have shipped Pock
as standard.

Then Apple announced the end of macOS on Intel Macs. The only way to keep my 2019 16-inch
MacBook Pro alive was to move to Linux — and [t2linux](https://wiki.t2linux.org/) makes it run beautifully. But
on Linux the Touch Bar goes back to a fixed row of keys, and I would not have lasted a minute
without Pock. So this is Pock's behaviour, reimplemented from scratch in Rust for Linux. It is
not a port (Pock is Swift and AppKit and cannot run here), and it has no connection to the Pock
project.

**If Pock made your Touch Bar worth having, [throw some money at its
author](https://www.paypal.com/paypalme/pigigaldi).**

## What works today

- **The Dock**: pinned and running apps, tap to switch, a row of an app's windows (with live
  previews on KDE), launch with a bounce, pin and unpin from the Touch Bar itself.
- **Folders** (Screenshots, Downloads, Trash showing full or empty), a **control centre**
  (lock, brightness, volume), **Wi-Fi**, **battery**, **clock**.
- **Fn** swaps in F1–F12. With listello stopped, the Touch Bar goes back to the Mac's own keys.
- A **settings window** with a live picture of the strip, per-user layouts, English and
  Italian, an optional tray icon, and start at login if you want it.

Tested on **Arch Linux, KDE Plasma 6 (Wayland), MacBookPro16,1 (T2)**. Other T2 Macs should
work, as they share tiny-dfr's panel code. Apple Silicon under Asahi Linux builds, but nobody
has tried it on a real machine yet — reports are very welcome. The Dock needs KDE Plasma for
now; on other desktops the Dock stays empty, and everything else should work.

## What's missing

- **Pock widgets not in listello yet:** Now Playing (what is playing, with play, pause and
  skip), Weather, the keyboard language in the status area, and Sleep, Screensaver and Do Not
  Disturb in the control centre.
- **Sliders.** Brightness and volume are buttons; Pock opens a slider.
- **Notification badges on Dock icons.** Only some Linux apps report them, so this would be
  partial at best.
- **An automatic Escape key.** The 13- and 15-inch Touch Bar models have no physical Esc.
  tiny-dfr adds one to the strip on those, but listello does not yet, and none of those models
  has been tested. Until then, add a button that sends Esc in the settings window or the config
  file (`keys = ["Esc"]`).
- **Widgets by other people.** Pock can load third-party widgets; listello has no plugin system,
  and every widget is built in.
- **The Dock on other desktops.** It needs KDE Plasma; GNOME, Sway, Hyprland and X11 get an
  empty Dock.
- **Packages for more systems.** Arch (AUR) and Debian 13 only so far: nothing yet for Fedora,
  Ubuntu or NixOS, and no ARM `.deb`.
- **Testing on more Macs.** Only the MacBookPro16,1 has been tried.
- **More languages.** English and Italian only.
- **Per-app layouts**, different buttons depending on the app in front. Pock does not do this
  either, but Apple's own Touch Bar did.

## Install

On Arch, from the AUR:

```sh
git clone https://aur.archlinux.org/listello-git.git && cd listello-git && makepkg -si
```

Debian (`.deb`), building from source, and the first start: see
**[docs/INSTALL.md](docs/INSTALL.md)**. Coming from tiny-dfr or from Pock?
[FROM-TINY-DFR.md](docs/FROM-TINY-DFR.md) and [FROM-POCK.md](docs/FROM-POCK.md) say what
changes.

## How it is put together

| Part | What it does |
|---|---|
| `listello-core` | The renderer. Owns the Touch Bar display and input, and runs as a system service with dropped privileges. On T2 Macs it starts when you start listello and stops when you stop it, handing the Touch Bar back to the Mac's own keys. |
| `listello-agent` | Runs as you, inside your desktop session, and supplies everything that needs your session: the window list, app icons, battery and Wi-Fi, and the actions buttons trigger. |
| `listello-settings` | The settings window and the tray icon. |

There are two processes because the renderer needs device access your desktop session must not
have, and the desktop session has access the renderer must not need. The reasoning behind this and every
other decision is in [docs/DESIGN.md](docs/DESIGN.md).

## Licence

listello is under the MIT licence, like Pock and tiny-dfr. Some icons are under Apache-2.0 (see
below). Both upstream notices ship verbatim and unmodified:
`LICENSE` (MIT) and `LICENSE.material` (Apache-2.0).

* Copyright The Asahi Linux Contributors — see the git history for authorship
  information. That history is merged into this repository, so `git log` and `git blame` on
  retained files attribute upstream's work to upstream.
* tiny-dfr embeds Google's
  [material-design-icons](https://github.com/google/material-design-icons), licensed under
  [Apache License Version 2.0](LICENSE.material). Some icons are derivatives of
  material-icons, with edits made by [kekrby](https://github.com/kekrby). Those icons are
  retained here unchanged.
* Other icons are from [Lucide](https://lucide.dev), under the licence in
  `share/listello/LICENSE.lucide`.
* Text on the strip is set in [Inter](https://rsms.me/inter/), by Rasmus Andersson, installed
  as a dependency. A copy ships with the tests (`crates/listello-ui/tests/fonts/`), under the
  SIL Open Font License included next to it.

## Disclaimer

listello was written almost entirely by AI: I decided what to build and how it should
behave, and tested it on my own Touch Bar. It comes with no warranty of any kind (see [LICENSE](LICENSE)). If it makes you cry
with joy or with pain, or your MacBook explodes, that is not on me.
