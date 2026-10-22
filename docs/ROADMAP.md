# Roadmap

**Definition of 1.0: a MacBook Pro used on Linux daily, by someone who does not miss Pock.**

That is the release gate (D9).

**Milestone numbers are dependency order, not calendar order.** They are stable identifiers —
other documents reference them — so they are never renumbered. The order they appear in this
file is the order to *start* them.

**M2 is listed second because it gates more than its original position suggested**, not because
it starts first. Everything from M3 onward blocks on it, and two answers M1 wants (the real
panel geometry, and which backlight node the machine exposes) can only come from hardware. In
practice it was deferred until data could be cleared off the Mac — so M0 and M1 had runway,
and M1 being the simulator is what kept work going in the meantime.

---

## M0 — Foundations

Repository scaffolding, before any application code. Ordered — steps 1–3 bake
names and structure into everything after them.

**1. Fork tiny-dfr**, merging upstream history in so authorship is preserved (that history is
part of the attribution D2a requires). Then, in our first commit:

- Replace upstream's README with ours, attribution above the fold. `CONTRIBUTING`. Keep
  `LICENSE` and `LICENSE.material` untouched.

**2. Restructure into a workspace** — the shape D11 and D10 need. This is the only part of M0
that is not mechanical:

```
crates/
  listello-core/       renderer: DRM, input, uinput, backlight, loop, layout, widgets
  listello-agent/      session agent (stub until M4)
  listello-protocol/   the message enum: D3's loop messages AND the D11 wire format
  listello-surface/    Surface trait + DrmSurface + SimulatorSurface
```

`listello-protocol` existing as a real crate from day one is what stops the D11 boundary
rotting back into a single process.

**3. Retain-and-mark** the files that survive the fork (RESEARCH §2.1) — `display.rs`,
`backlight.rs`, `pixel_shift.rs`, `fonts.rs`, plus `etc/` and `share/` — into their new homes,
each with a provenance header. `main.rs` and `config.rs` stay put temporarily so the tree keeps
building; M1 dismantles them.

**4. CI** — `cargo build`, `cargo clippy -D warnings`, `cargo test`, `cargo fmt --check` on
Linux, plus an aarch64 build (Apple Silicon under Asahi) and a macOS build of the simulator.

**5. GitHub** — private repo `listello`, and the post-1.0 backlog below filed as issues
immediately so it stops competing for attention with 1.0.

## M2 — Linux on the MacBook *(deferred, but gates M3 onward)*

Listed before M1 because of what it gates, not because it starts first. Everything from M3 is
blocked on it, and M1 wants two of its measurements.

- Choose a distro. **Leaning Fedora + KDE via t2linux** (RESEARCH §6.8), not final.

**Timing, 2026-08-03:** this was deferred until a large amount of data was moved off the Mac,
which had to happen before it could dual-boot. So M2 did *not* start
immediately, and M0/M1 had more runway than the critical path implied. M3 onward could not
begin until M2 landed.
- Install per t2linux. Confirm the Touch Bar comes up and that stock `tiny-dfr` works — if it
  does, the whole hardware layer we are forking is proven on this exact machine before we
  change a line of it. **Record the kernel version and what `appletbdrm` reports.**

**Measured 2026-10-22, MacBookPro16,1, Arch `linux-t2` 7.2.11, stock `tiny-dfr` 0.3.8 from
`arch-mact2`:**

- **DRM mode 60 × 2008, framebuffer 64 × 2008** (`XR24`, pitch 256), read from
  `/sys/kernel/debug/dri/0/` while tiny-dfr was driving it. Not 2170: the simulator, the golden
  images and every test that uses 2170 describe a different (13") panel. Harmless for the
  daemon, which reads the size from DRM; wrong for any golden image meant to match this panel.
- **Backlights:** `appletb_backlight` (Touch Bar) and `gmux_backlight` (display).
- **`seat-touchbar` is set up by tiny-dfr's udev rules unaided.** One more rule in that file
  matters: the Touch Bar boots in USB configuration 1 (HID keyboard only, firmware-drawn F-keys)
  and only exposes the `appletbdrm` interface after switching to configuration 2. Without
  tiny-dfr's rules installed there is no Touch Bar DRM card at all, and `appletbdrm` is never
  loaded although it is built.
- `appletbdrm` reports `Initialized appletbdrm 1.0.0`, minor 0 (`/dev/dri/card0`).


1. **Both panel geometries, not one** (RESEARCH §2.1) — the DRM *mode* size and the
   *framebuffer* size, which differ. The framebuffer is over-allocated wider than the visible
   panel, and the simulator and every golden image must match the framebuffer. Getting only the
   mode size back means redoing every golden image later. *Wanted by: M1.*
2. **Which backlight sysfs nodes exist** under `/sys/class/backlight` (RESEARCH §6.5) — expect
   `appletb_backlight` for the Touch Bar plus one of `apple-panel-bl` / `gmux_backlight` /
   `intel_backlight` / `acpi_video0` for the display. *Wanted by: M5's brightness buttons.*
3. **Whether the Touch Bar's own libinput seat (`seat-touchbar`) is set up** by tiny-dfr's udev
   rules on this distro, or needs manual help. *Wanted by: M3.*

Nothing in M0 or M1 is *blocked* on M2 — that is what the simulator is for. But M2 finishing
late pushes M3 and everything after it out by the same amount, so it should not idle.

## M1 — The simulator, and everything under it

The largest and most important milestone, because it defines every interface the rest of the
project uses, and because until it exists nothing can be seen.

- `Surface` abstraction: draw commands in, pixels out. Two implementations —
  `SimulatorSurface` (a window, runs on macOS and Linux) and `DrmSurface` (tiny-dfr's existing
  path). **No DRM specifics above this line, ever.**
- Input abstraction: touch/tap/scroll events, with libinput and mouse/scroll-wheel backends.
- The single-threaded event loop and message passing (D3). **The message enum lives in the
  `protocol` crate and doubles as the D11 wire format** — M1 ships no agent, but the boundary
  must exist in the types from the start.
- **A frame clock.** tiny-dfr's loop wakes only on input or a 10 s timer and has nothing that
  can drive an animation (RESEARCH §2.1). Dock scrolling (M4) and the bounce (M6) both need
  one, and retrofitting it into a finished loop is far more expensive than designing it in.
- The `Widget` trait: immutable state snapshot + geometry in, draw commands + hit regions out.
- The layout engine: a horizontal strip of widgets, fixed and flexible regions, separators, a
  scrollable region that clips (needed by the Dock — see M4 layout spec).
- TOML config: which widgets, in what order, with what settings.
- **Golden-image test harness.** Render a widget against a fixed state snapshot, compare to a
  committed PNG. This is the gate for all rendering work and needs to exist early.
- One or two trivial widgets (clock, a static button) to prove the whole chain.

**Exit criterion:** one command on the Mac shows a Touch Bar-shaped window, and
clicking things in it does something.

## M3 — Real hardware and Fn row parity

- The DRM surface and libinput path working on the real Touch Bar.
- **Holding Fn shows F1–F12** (D6). Anyone replacing tiny-dfr with this must not lose their
  function keys.
- Suspend/resume survival, and recovery when the Touch Bar comes back wrong (RESEARCH §1).
- systemd unit, privilege dropping preserved from tiny-dfr.

**Exit criterion:** this daemon replaces tiny-dfr on the reference machine without regression.

**Status 2026-10-22, MacBookPro16,1:** installed as the service, started by udev, drops to
`nobody` (+ `input`, `video`). Fn layer built (`[[fn_widget]]`) and checked on the panel;
suspend/resume survived without a restart (the kernel logs a warning from `appletbdrm` on
resume — its own, not ours). Holding a media key repeats it, as under tiny-dfr (keys go down on
touch, up on release). Cold boot checked: after a real power-on (2026-10-22 09:01) the socket
and service came up by themselves within six seconds and the agent connected half a minute
later, so the unit needs no `BindsTo=` drop-in. **M3 is closed.**

## M4 — The Dock widget

The signature Pock feature and the hardest part.

- **The session agent process itself** (D11) — this is where it first exists and first earns
  its keep. `WindowSource` and all D-Bus work live in it, not in the renderer.
- `WindowSource` trait + **KDE/KWin backend** (D4).
- **The agent's own `.desktop` file, and an installed path to run it from** (D11 amendment).
  KWin advertises `org_kde_plasma_window_management` only to a client whose executable has a
  `.desktop` file declaring it in `X-KDE-Wayland-Interfaces`, so this is a functional
  dependency of everything else in this milestone, not packaging to be tidied up in M6.
  Ship it in the same commit as the KDE backend, and make the agent **say why** when the
  global is missing — `cargo run` from a build directory sees no windows, silently.
- `.desktop` and XDG icon resolution, with a user-overridable mapping table (RESEARCH §4).
- Pinned vs running app grouping.
- Tap to focus; tap a multi-window app to get a row of labelled window entries (D7 — titles,
  not thumbnails); tap an entry to focus that window.
- Horizontal scroll of the app strip when it overflows.
- Launch an app that is pinned but not running.

**Status 2026-10-22:** built on branch `m4-dock` and installed on the MacBook. Done: the agent
process and its reconnecting socket (both ends share one `Link` in `listello-protocol`), the
KWin `WindowSource` with the `NoWindowManagement` message and the agent's `.desktop` file,
`.desktop` and icon resolution with `~/.config/listello/apps.toml` as the user's override
table, pinned/running grouping, tap to focus, the expanded row of titled entries, drag to
scroll, and launching a pinned app that is not running. Not done: the fold behind separator 2
(M5's layout — the Dock simply clips for now), kinetic scrolling, and a person's check on the
real panel.

**Verified on the panel 2026-10-22:** the Dock works. Its looks were a placeholder, deliberately
left for after the functionality.

## M5 — The reference layout

The concrete parity target. **None of this may be hardcoded** (D8) — it is one configuration,
and it must be expressible by anyone else writing a different one.

Left to right, a typical macOS Pock strip:

```
[ pinned running apps │ other running apps ⇠scrollable⇢ ]│[ folders ][ control centre ][ wifi ][ battery ]
                      ▲                                  ▲
                 separator 1                        separator 2
```

- **Separator 1** divides pinned apps (left) from unpinned running apps (right).
- **Separator 2** marks where the scrollable app strip is clipped. An app that would only be
  partially visible is folded behind it; scrolling horizontally with a finger brings it out.
- **Folders widget** — a configurable list of folders opened with the system file manager. Here:
  Screenshots, Downloads (in place of macOS's Applications), and
  **Trash** (which should indicate full/empty).
- **Control centre widget** — a configurable set of buttons. Here: lock, brightness down,
  brightness up, mute, volume down, volume up. *Buttons, not sliders* — note this differs from
  Pock's macOS control centre, on purpose.
- **Status** — WiFi symbol, and a battery indicator showing **both** the percentage and the
  graphical bar.

**Status 2026-10-22:** every widget above exists, is unit-tested and has a golden, and the
packaged icons load in both the daemon and the simulator. The agent supplies battery, wifi
and folder state; the layout is `share/listello/config.example-dock.toml`. Battery and wifi
come from `/sys` and `/proc` rather than UPower and NetworkManager (RESEARCH §6 allows
either for battery): no session bus, no new dependency, and the wifi symbol does not need the
network name. Not yet seen on the panel by a person. Still open: the separator-2 fold (an
app only partly visible is folded behind the clip edge) and finger-scrolling that strip as
its own widget.

## M6 — 1.0 polish

- **App launch bounce animation** (explicitly in 1.0). **Built, 2026-10-22,
  not yet seen on the panel.** It uses the agent's own `LaunchStarted` message rather than
  KWin's `org_kde_plasma_activation_feedback` (RESEARCH §6.4): the Dock's tile hops for as long
  as an app the agent launched has no window, up to ten seconds. That covers launches from the
  Touch Bar; an app started from the desktop menu does not bounce. The activation-feedback
  protocol (already declared in the agent's `.desktop` file) is the way to cover those, if
  wanted.
- Packaging: **built, 2026-10-22.** Arch package (`packaging/arch/`), AUR entry
  (`packaging/aur/`, `listello-git`, builds from the repository so it works only once the
  repository is public), `.deb` (`packaging/debian/`, built in a Debian 13 container and
  installed in a clean one), and build-from-source (`make`, `sudo make install`). All of them
  share the file list in the top-level `Makefile`, and none turns anything on for any user.
  The AUR entry has not been pushed to the AUR; the README is separate work.
- Install and migration docs: **written** — `docs/INSTALL.md`, `docs/FROM-TINY-DFR.md`,
  `docs/FROM-POCK.md`.
- Make the repo public (D9).

---

## Post-1.0 backlog

Track these as GitHub issues labelled `post-1.0` from the moment they come up, so nothing is
lost and nothing derails 1.0.

**Desktop support**
- GNOME (Wayland) — needs a companion GNOME Shell extension. A second deliverable in a second
  language, plus possibly extensions.gnome.org review.
- wlroots compositors (Sway, Hyprland) — cheap once `WindowSource` exists.
- X11 / EWMH fallback.

**Widgets**
- Now Playing / MPRIS media controls, with swipe gestures. A headline Pock feature and cheap
  on Linux; deliberately not in 1.0.
- Weather widget.
- App notification badges (Unity launcher D-Bus API — partial parity only, few apps implement
  it).
- ESC widget — pointless on the MacBookPro16,1 (physical Esc key) but needed if we ever
  support the 13"/15" Touch Bar models that lack one.

**Platform**
- Plugin API: Lua or WASM widgets, as a backend behind the existing widget trait (D3).
- Graphical settings app (D8). With it, an optional system-tray icon (a setting, off unless
  the user wants it) offering: reload listello, open the settings, check for updates, and
  later whatever else earns a place. Everything the tray offers is also in the settings
  window. **Designed 2026-10-22 (D8 amendment): per-user layouts, one language
  setting (Italian, English), an interactive picture of the strip with Apply. Next to build,
  in this order:** (1) per-user layout and the language setting, end to end through agent and
  renderer, verifiable by hand-editing `~/.config/listello/config.toml` — **done 2026-10-22**;
  (2) the window — **done 2026-10-22**;
  (3) the tray icon — built 2026-10-22; then (4) start and stop as a user (D8, second
  amendment): Stop listello, the application menu starting listello, the first-start question,
  "Start at login".
- Live window thumbnails where the compositor permits (D7). **Done on KDE, 2026-10-22.**
- Per-application layouts (different widgets depending on the focused app) — Pock does not do
  this; macOS's own Touch Bar does.
