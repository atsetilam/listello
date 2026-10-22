# `etc/` — system integration

Retained from [tiny-dfr](https://github.com/AsahiLinux/tiny-dfr), Copyright The Asahi Linux
Contributors, MIT. Every file in this directory is upstream's work with names changed to
listello's (D12): the unit is `listello-core.service`, the binary is `/usr/bin/listello-core`,
and the udev device aliases are `/dev/listello_display`, `/dev/listello_backlight` and
`/dev/listello_display_backlight`. Nothing else about them has been touched.

Two things differ from upstream since the start/stop work (D8, second amendment): on a T2
MacBook the renderer is no longer started by udev at boot, and the udev rule that selected USB
configuration 2 is gone. `listello-core.service` selects configuration 2 itself before it runs
and configuration 1 (the T2's own keys) after it ends, so a machine with nobody running listello
shows the T2's keys; it is started by the agent connecting to `listello-core.socket` and leaves
a few seconds after the last agent has. Apple Silicon keeps udev starting it and it stays up.

That includes the parts that are easy to mistake for boilerplate and are not:

- **`99-touchbar-seat.rules`** puts the Touch Bar's input and DRM devices on their own libinput
  seat, `seat-touchbar`. Without it the Touch Bar's touchpad is treated as an ordinary pointer
  and the cursor jumps around the screen when you touch the bar.
- **`99-touchbar-listello.rules`** matches the panel across several machine generations —
  `appletbdrm` on T2 Intel, `adp` on Apple Silicon — and names its devices for systemd. On
  Apple Silicon it also starts the service when the panel appears; on a T2 it does not.
- **`systemd-backlight@*.service`** drop-ins stop systemd from restoring a saved brightness to
  the Touch Bar panel on boot, which fights with the daemon's own backlight management.

## Installing next to tiny-dfr

Installing listello-core in place of the distro's tiny-dfr package means: put the unit and
rules from this directory in `/etc`, mask `tiny-dfr.service`, and shadow tiny-dfr's own
`99-touchbar-tiny-dfr.rules` with an empty file of the same name in `/etc/udev/rules.d/`
(its USB-mode rule is in `99-touchbar-listello.rules`, and running both would switch the
Touch Bar's USB configuration twice).

`listello-core.socket` is the agent's socket and, on a T2, what starts the renderer; enable it
(`systemctl enable --now listello-core.socket`) **while the service is stopped** — a socket will
not start while its service is running. `etc/systemd/user/listello-agent.service` is the agent's `systemd --user`
unit, and `share/applications/org.listello.Agent.desktop` must be installed next to the agent
binary: without it KWin shows the agent no windows (D11 amendment).
