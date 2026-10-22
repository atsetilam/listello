# listello

**The Touch Bar, made useful again — on Linux.** A widget manager for the MacBook Pro Touch
Bar: a Dock with your apps and windows, folders, a control centre, wifi, battery and a clock,
all in the strip above your keyboard. Hold Fn and you get F1–F12, as you always did.

![The listello strip: the Dock, folders, control centre, wifi, battery and clock](docs/strip.png)

A *listello* (*lee-STEL-loh*) is a thin decorative strip — in tiling, the narrow border
course. Which is exactly the shape of the thing this drives.

## Built on tiny-dfr

listello is a fork of **[tiny-dfr](https://github.com/AsahiLinux/tiny-dfr)**, by
**The Asahi Linux Contributors**, and would not exist without it.

tiny-dfr solved the genuinely hard, hardware-specific half of this problem: opening the Touch
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
people who loved the Touch Bar, and that is entirely Pock's doing — Apple should have shipped it
as standard.

Then Apple stopped supporting Intel Macs. To keep a 2019 16-inch MacBook Pro alive, moving to
Linux was the only way — and [t2linux](https://wiki.t2linux.org/) makes it run beautifully. But
on Linux the Touch Bar goes back to a fixed row of keys, and I would not have lasted a minute
without mine. So this is Pock's behaviour, reimplemented from scratch in Rust for Linux. It is
not a port (Pock is Swift and AppKit and cannot run here), and it has no connection to the Pock
project.

**If Pock made your Touch Bar worth having, [throw some money at its
author](https://www.paypal.com/paypalme/pigigaldi).**

## What works today

- **The Dock**: pinned and running apps, tap to switch, a row of an app's windows (with live
  previews on KDE), launch with a bounce, pin and unpin from the Touch Bar itself.
- **Folders** (Screenshots, Downloads, Trash showing full or empty), a **control centre**
  (lock, brightness, volume), **wifi**, **battery**, **clock**.
- **Fn** swaps in F1–F12. With listello stopped, the Touch Bar goes back to the Mac's own keys.
- A **settings window** with a live picture of the strip, per-user layouts, English and
  Italian, an optional tray icon, and start at login if you want it.

Tested on **Arch Linux, KDE Plasma 6 (Wayland), MacBookPro16,1 (T2)**. Other T2 Macs and Apple
Silicon share tiny-dfr's panel code and should work, but nobody has tried yet — reports are very
welcome. The Dock needs KDE Plasma for now; on other desktops everything else works and the
Dock stays empty.

## Install

On Arch, from the AUR:

```sh
git clone https://aur.archlinux.org/listello-git.git && cd listello-git && makepkg -si
```

Debian and Ubuntu (`.deb`), building from source, and turning it on afterwards: see
**[docs/INSTALL.md](docs/INSTALL.md)**. Coming from tiny-dfr or from Pock?
[FROM-TINY-DFR.md](docs/FROM-TINY-DFR.md) and [FROM-POCK.md](docs/FROM-POCK.md) say what
changes.

## How it is put together

| | |
|---|---|
| `listello-core` | The renderer. Owns the Touch Bar display and input, runs as a system service with dropped privileges. Works on its own: without the agent you get function keys and media keys, i.e. tiny-dfr's behaviour. |
| `listello-agent` | Runs as you, inside your desktop session, and supplies what needs it: the window list, app icons, battery and wifi, and actions. |
| `listello-settings` | The settings window and the tray icon. |

Two processes because the renderer needs device access your desktop session must not have, and
the desktop session has access the renderer must not need. The reasoning behind this and every
other decision is in [docs/DESIGN.md](docs/DESIGN.md).

## License

MIT, matching both Pock and tiny-dfr. Both upstream notices ship verbatim and unmodified:
`LICENSE` (MIT) and `LICENSE.material` (Apache-2.0).

* Copyright The Asahi Linux Contributors — please see the git history for authorship
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
  as a dependency rather than shipped here.

## Disclaimer

listello was written almost entirely by AI: I decided what to build and tested it on my own
Touch Bar. It comes with no warranty of any kind (see [LICENSE](LICENSE)). If it makes you cry
with joy or with pain, or your MacBook explodes, that is not on me.
