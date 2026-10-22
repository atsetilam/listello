# Coming from tiny-dfr

listello is built on tiny-dfr and does what it does by default: with the packaged config you
get the media keys on the Touch Bar, and F1–F12 while you hold Fn. If you install it and change
nothing, nothing is lost. (Install steps are in [INSTALL.md](INSTALL.md); the package
replaces tiny-dfr.)

What listello adds is a Dock, a battery, wifi, folders and a control centre, and a config
that lists widgets instead of a fixed row of buttons.

## Your old config

tiny-dfr read `/etc/tiny-dfr/config.toml`. listello reads `/etc/listello/config.toml` and does
**not** read the old file. The format is different, so a copy will be refused with an error
in the log.

| tiny-dfr | listello |
|---|---|
| `PrimaryLayerKeys = [...]` and `MediaLayerKeys = [...]` | `[[widget]]` entries (the strip you see) and `[[fn_widget]]` entries (the strip shown while Fn is held). For tiny-dfr's default `MediaLayerDefault = true`, put the media keys under `[[widget]]` and F1–F12 under `[[fn_widget]]`. For `false`, swap them. |
| `{ Text = "F1", Action = "F1" }` | `kind = "button"`, `label = "F1"`, `keys = ["F1"]` |
| `Action = ["LeftCtrl", "LeftAlt", "F1"]` | `keys = ["LeftCtrl", "LeftAlt", "F1"]` |
| `Icon = "volume_up"` | `icon = "volume-up"` (dashes) |
| `{ Time = "24hr" }` | `kind = "clock"`, `format = "%H:%M"` |
| `{ Battery = "both" }` | `kind = "battery"` (needs the agent) |
| `Stretch = 2` | `flex = 2.0`; or `width = 120.0` for a fixed size in pixels |
| an empty `{ Stretch = 3 }` spacer | no direct equivalent; `[strip] gap` sets the space between all widgets |
| `AdaptiveBrightness`, `ActiveBrightness` | `[core]` table, same names in lower case |

Key names are the same ones tiny-dfr used (`"F5"`, `"VolumeUp"`, `"BrightnessDown"`).

### Not carried over yet

`ShowButtonOutlines`, `EnablePixelShift`, `FontTemplate` and `DoublePressSwitchLayers`
have no equivalent today. The Touch Bar keeps working without
them; they are on the list.

## Things that behave differently

- **Two programs now.** `listello-core` is the old daemon's job (panel, touch, keys,
  brightness) and runs without a desktop. `listello-agent` runs in your session and supplies
  windows, icons, battery and wifi. If the agent is not running you get tiny-dfr's behaviour:
  keys and clock work, the Dock, battery and wifi stay blank.
- **Your config is checked strictly.** An unknown key or a misspelt widget is an error, not
  something silently ignored. A bad edit is reported and the previous layout stays on screen.

## Going back

See the last section of [INSTALL.md](INSTALL.md). Your old `/etc/tiny-dfr/config.toml` is
untouched by listello.
