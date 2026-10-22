# `share/listello/` — packaged assets

Installed to `/usr/share/listello/`. Retained from
[tiny-dfr](https://github.com/AsahiLinux/tiny-dfr)'s `share/tiny-dfr/`, Copyright The Asahi
Linux Contributors.

The SVGs are Google's
[material-design-icons](https://github.com/google/material-design-icons), licensed under
Apache-2.0 — see [`LICENSE.material`](../../LICENSE.material). Some are derivatives of
material-icons with edits made by kekrby. They are retained here unchanged and carry no
per-file headers; this note covers the directory.

`config.toml` is the packaged default configuration. **Do not edit it to configure a machine** —
copy it to `/etc/listello/config.toml` and edit that. Only the paths inside it have been
changed from upstream's; its schema is still tiny-dfr's fixed row of buttons, and M1 replaces
it with one that describes widgets and layout (D8).

The icons for lock, folder, download, screenshot and trash (full and empty) were added for
listello from the same Material Symbols set, outlined style, and are covered by the same licence. A config names an
icon with dashes (`trash-full`); the file is the same name with underscores (`trash_full.svg`).

`folder_screenshots.svg`, `folder_downloads.svg`, `bin_full.svg` and `bin_empty.svg` are
listello's own, drawn for it in the look of Pock's Touch Bar folders and bin, and are covered
by listello's licence like its code.

The control-centre glyphs (`cc_*.svg`), the Dock's `pin.svg` and `pin_off.svg`, and the wifi glyphs (`network_0`–`network_3`,
`network_off`) are drawn from [Lucide](https://lucide.dev) 0.460.0, under the ISC licence in
[`LICENSE.lucide`](LICENSE.lucide): white strokes, the speaker and the lock body filled, and the
wifi strength shown by dimming the arcs a weaker signal does not reach.
