# Research notes

Everything established during the 2026-08-03 scoping, with sources, plus an explicit list
of what is **not** yet established. Read the open questions (§6) before planning: several of
them change the shape of a milestone.

Confidence tags: `[verified]` read from a primary source; `[reported]` from a search summary
or secondary source, plausible but unconfirmed; `[assumption]` reasoning, not evidence.

---

## 1. The Touch Bar on Linux

The Touch Bar is two devices: **a small DRM display** and **a HID multitouch input device**.
It is not a special API — you draw to it like any other screen.

- On Intel/T2 Macs the display driver is **`appletbdrm`**; on Apple Silicon it is **`adp`**.
  Both were merged for **Linux 6.15**. `[reported]`
  — https://www.phoronix.com/news/Apple-Touch-Bar-Display-Linux
  — https://lkml.iu.edu/2502.3/06190.html (PATCH v7, "Touch Bar DRM driver for x86 Macs")
- Touch input on x86 Macs goes through **`HID: multitouch`**, with Touch Bar support added
  upstream. `[reported]` — https://lwn.net/Articles/1022733/
- The older out-of-tree stack is **`apple-ibridge` / `apple-ib-tb` / `apple-ib-als`** from
  t2linux, which enables the Touch Bar and handles Fn-key mode switching. `[verified, repo]`
  — https://github.com/t2linux/apple-ib-drv
- Overview of Touch Bar support landing for T2: https://lwn.net/Articles/986150/

**Known fragility.** `appletbdrm` has had binding failures on some kernel revisions
(loaded fine on 6.12.21, failed to bind on 6.12.31 after DRM API changes), and module *reload*
is broken independently of initial load. Suspend/resume has needed fixes as recently as
mid-2026. `[reported]`
— https://github.com/t2linux/wiki/issues/635
— https://github.com/basecamp/omarchy/discussions/5862

*Implication:* treat "the Touch Bar came back wrong after suspend / after a kernel update" as
an expected condition to handle and document, not an edge case. Whatever tiny-dfr already does
about this, keep.

**Do not hardcode the resolution.** The panel geometry differs between models and between the
T2 and Apple Silicon drivers, and part of the panel is physically obscured on some models.
Read the mode from DRM. The simulator (D10) should take its size from config, defaulting to
whatever the reference hardware reports once measured. `[assumption — but the safe one]`

## 2. tiny-dfr — the fork base

- Repo: https://github.com/WhatAmISupposedToPutHere/tiny-dfr (AsahiLinux). "The most basic
  dynamic function row daemon possible." Shows the function row and media keys (brightness,
  volume, backlight, play). Supports **Apple Silicon and T2**. `[verified]`
- License: **MIT AND Apache-2.0** — MIT for the code, Apache-2.0 for embedded Google
  material-design-icons. `[verified, crates.io + README]`
- Packaged in Debian (experimental), Ubuntu, Fedora, Nix, Arch. `sudo apt install tiny-dfr`.
  Config at `/usr/share/tiny-dfr/config.toml`, user override at `/etc/tiny-dfr/config.toml`.
  `[reported]` — https://packages.debian.org/experimental/main/tiny-dfr
- Dependencies (`Cargo.toml`, `master`) `[verified]`:

  | crate | what it gives us |
  |---|---|
  | `drm` 0.14 | the Touch Bar display |
  | `input` 0.9, `input-linux` 0.7, `input-linux-sys` 0.9 | libinput touch events, uinput key injection |
  | `cairo-rs` 0.22, `freetype-rs` 0.38 | 2D drawing and text |
  | `librsvg-rebind` 0.3 | SVG icons (needs librsvg ≥ 2.59) |
  | `freedesktop-icons` 0.4.0 | **XDG icon theme lookup — this is how we get app icons** |
  | `udev` 0.9, `nix` 0.29, `privdrop` 0.5.3 | device discovery, privilege dropping |
  | `serde` 1, `toml` 0.8, `chrono` 0.4, `anyhow` 1, `libc`, `rand` | plumbing |

  System deps: cairo, libinput, freetype, fontconfig, librsvg ≥ 2.59, `uinput` enabled in the
  kernel.

*Notable:* `freedesktop-icons` and `cairo` mean the icon-rendering and drawing problems are
already solved in the fork base. That is a large head start on the Dock widget.

### 2.1 The source, read (2026-08-03) `[verified — read in full]`

1,882 lines across six files at `v0.3.7`. **Verdict: roughly 35% fork, 65% new code — and the
fork covers essentially all of the hardware-specific risk.** The seam falls almost exactly on
file boundaries, which makes this a clean fork rather than a messy one.

| File | Lines | Fate |
|---|---|---|
| `display.rs` | 213 | **Keep.** This *is* `DrmSurface`: card discovery, atomic modeset, dumb buffer, damage-rect `dirty_framebuffer`. Needs a trait wrapper, nothing more. |
| `backlight.rs` | 150 | **Keep whole.** Touch Bar backlight, adaptive brightness from the display backlight, dim/off timeouts, lid switch. |
| `pixel_shift.rs` | 107 | **Keep whole.** OLED burn-in mitigation. Nobody would think to write this. |
| `fonts.rs` | 138 | **Keep whole.** fontconfig FFI wrapper. |
| `config.rs` | 217 | **Rewrite**, steal the pattern (see below). |
| `main.rs` | 1057 | **Split**: ~700 lines replaced, the uinput/key-emission path kept. |

Also worth keeping: `etc/` (systemd unit, and **udev rules that put the Touch Bar on its own
libinput seat, `seat-touchbar`** — undocumented tribal knowledge, and non-obvious), and
`share/*.svg` (the Apache-2.0 battery and backlight icons).

**What must be rewritten, and why.** `Button` / `ButtonImage` / `FunctionLayer` in `main.rs` is
not a widget system. It is a six-variant enum (`Text`, `Svg`, `Bitmap`, `Time`, `Battery`,
`Spacer`) that fuses rendering, hit-testing, mutable state, *and data acquisition* —
`get_battery_state()` reads sysfs from inside `render()`. Layout is N equal-width virtual slots
with a `stretch` multiplier to span several. There is no clipping, no scrolling, no separators,
no variable-width region. **The M5 layout is not expressible in it.** D3 replaces all of
it. `config.rs` hardcodes exactly two layers of `ButtonConfig`; the *pattern* is worth stealing
— base config in `/usr/share`, user override in `/etc`, live reload via inotify — but every
type changes.

**The D10 violation to fix in the first commit.** `FunctionLayer::draw()` takes a
`cairo::Surface` and returns `Vec<drm::control::ClipRect>`. DRM types are in the draw
signature, and the 90° rotation is applied inside `draw`. That is exactly the line D10 says
must never be crossed: introduce the `Surface` trait immediately, replace `ClipRect` with our
own damage-rect type, and push rotation down into the DRM backend.

> **Done, M1 step 1 (2026-08-03).** `FunctionLayer::draw()` is now
> `FunctionLayer::frame()`, returning a `listello_surface::Frame` — a draw-command list plus
> damage rectangles, all in logical coordinates. `listello-core` names neither DRM nor a
> rotation anywhere. The 90° transform lives in `listello_surface::Rotation::Cw90`, and a unit
> test asserts it reproduces tiny-dfr's damage arithmetic exactly.

**Good news for the simulator (D10, M1).** Everything renders into a plain `cairo::ImageSurface`
which is then `memcpy`'d into the DRM dumb buffer. `SimulatorSurface` is therefore the same
`ImageSurface` blitted into a window, and cairo runs on macOS. M1's rendering half is cheap;
the cost is the macOS window and input plumbing, not the drawing.

**Two geometry facts the simulator must reproduce, or golden images will drift:**

- The panel reports a **portrait** DRM mode (~60 × 2170 on the 13"; **measured 60 × 2008, framebuffer 64 × 2008 on MacBookPro16,1**, ROADMAP M2) and is rotated 90° in software
  (`c.translate(height, 0.0); c.rotate(90°)`).
- The dumb buffer is allocated **64 px wide** while the visible mode is narrower — the
  framebuffer is wider than the panel, and the cairo surface is created at *framebuffer*
  dimensions (`fb_info().size()`), not mode dimensions. Match the framebuffer, not the mode.

**Redraw model, and what M6 needs.** The loop wakes only on input or a timer (10 s, or per-second
when a clock widget demands it) and redraws only buttons whose `changed` flag is set, pushing
damage rects. There is no frame clock and no double buffering. The bounce animation (M6) and
Dock scrolling (M4) need one — **design the frame clock into M1's loop rather than retrofitting
it later.**

**Error handling.** tiny-dfr `.unwrap()`s and `.expect("failed to load icon")`s liberally,
catching the fallout by painting an embedded crash bitmap to the panel and waiting for SIGTERM.
That is defensible for twelve static, packaged icons. It is not defensible for a Dock loading
arbitrary third-party application icons, where a malformed SVG becomes a guaranteed crash.
**All external-data paths must return errors, not panic.** Easy to gate with tests.

**Privileges — see [D11](DESIGN.md#d11-two-processes-a-privileged-renderer-and-a-session-agent).**
`real_main` drops to `nobody`:`input`,`video` before the loop. This is why the project needs two
processes; it was the most consequential finding of the source read.

### 2.2 `cairo::FontFace::clone()` is a double-free here `[verified — reproduced 2026-08-03]`

Found while building the `Surface` layer (M1 step 1), on cairo-rs 0.22.0 on macOS.

We inherit tiny-dfr's cairo-rs configuration: `default-features = false`, features `freetype`
and `png`. That leaves **`use_glib` off**, and without it cairo-rs's `impl Clone for FontFace`
is

```rust
fn clone(&self) -> FontFace { unsafe { FontFace::from_raw_none(self.to_raw_none()) } }
```

where the non-glib `from_raw_none` just wraps the pointer — it never calls
`cairo_font_face_reference`. `Drop` still calls `cairo_font_face_destroy`. So cloning a font
face and dropping both halves over-releases it and cairo aborts the process:

```
Assertion failed: (CAIRO_REFERENCE_COUNT_HAS_REFERENCE (&font_face->ref_count)),
function cairo_font_face_destroy, file cairo-font-face.c, line 176
```

Reproduced with both `FontFace::toy_create` and `FontFace::create_from_ft`, so it is the clone
and not the constructor. tiny-dfr never triggers it: it stores exactly one face in `Config` and
only ever passes `&FontFace` down into `render()`.

*Implication, and it is permanent until we change the feature flags:* **never clone a
`cairo::FontFace`.** listello has to share one, because draw commands are owned data — so
`listello_surface::TextStyle` holds an `Rc<FontFace>` and `Config::font_face` is an
`Rc<FontFace>`. A later "simplification" that unwraps either of those `Rc`s reintroduces a
process abort that no test will catch until a second widget draws text. Do not remove the `Rc`.

The alternative fix — enabling cairo-rs's `use_glib` feature, which routes clone through
`from_glib_none` and does reference properly — was rejected: it adds a glib build dependency to
`listello-surface`, which has to build on macOS for the simulator, and it makes
correctness depend on a feature flag whose silent removal is fatal. `Rc` is ours and cannot be
switched off.

## 3. Enumerating and focusing windows (the Dock widget's hard problem)

There is no portable Wayland way to list other clients' windows. Per backend:

- **KDE Plasma / KWin** — `org_kde_plasma_window_management`. Long-standing, gives window
  list, `app_id`, title, icon, minimised state, **and activation**. This is why KDE is
  target #1. `[assumption — strongly expected, verify]`
  KWin additionally supports `ext-foreign-toplevel-list-v1`. `[reported]`
  — https://bugs.kde.org / kwin bug 483227

  > **ANSWERED 2026-08-03 by measurement, and the answer is good — but it comes with a
  > condition that changes how listello must be installed.** `[verified — measured, and read
  > from KWin 6.7.3's source]`
  >
  > On a CachyOS machine (KWin 6.7.3, full Plasma session), a plain client sees **66
  > globals and no way to list windows**: no `org_kde_plasma_window_management`, no
  > `ext_foreign_toplevel_list_v1`, no wlroots equivalent. The protocol is **not gone — it is
  > gated**, and KWin says so plainly in `src/wayland_server.cpp`:
  >
  > ```cpp
  > const QSet<QByteArray> interfacesBlackList = {
  >     QByteArrayLiteral("org_kde_plasma_window_management"),
  >     QByteArrayLiteral("org_kde_kwin_fake_input"),
  >     QByteArrayLiteral("zkde_screencast_unstable_v1"),
  >     QByteArrayLiteral("org_kde_plasma_activation_feedback"),
  >     QByteArrayLiteral("kde_lockscreen_overlay_v1"),
  > };
  > ```
  >
  > A blacklisted global is advertised only if the client's **executable has a `.desktop` file
  > declaring the interface** in `X-KDE-Wayland-Interfaces` (`src/utils/serviceutils.h`;
  > sandboxed clients are matched by security-context app id instead). The lookup is
  > `KApplicationTrader` matching the **canonical path of the first token of `Exec=`** against
  > the client's `/proc/<pid>/exe`. No `.desktop` file, or a mismatched path, means no window
  > list — silently, with the protocol simply absent from the registry.
  >
  > **Proven end to end**, not merely read: dropping a `.desktop` file naming
  > `/usr/bin/wayland-info` into `~/.local/share/applications` and running `kbuildsycoca6`
  > took that client from 66 globals to 68, with `org_kde_plasma_window_management` (name 25)
  > and `org_kde_plasma_activation_feedback` (name 27) appearing — the exact two gaps in the
  > global name-id sequence that the denied run had shown. Removing the file put it back to 66.
  >
  > **What this means for listello:**
  >
  > 1. **D4's KDE-first ordering stands, and D11's agent is the right shape.** KDE does give
  >    us the complete answer — list, `app_id`, title, icon, minimised state and activation.
  >    **D11 was amended on 2026-08-03 to record the install-time constraint** — read that
  >    amendment, it is the binding version of points 2 and 3 below.
  > 2. **`listello-agent` must ship a `.desktop` file** declaring
  >    `X-KDE-Wayland-Interfaces=org_kde_plasma_window_management,org_kde_plasma_activation_feedback`,
  >    with `Exec=` pointing at its **installed** path.
  > 3. **Packaging becomes load-bearing for function, not just convenience.** `cargo run` from
  >    a build directory will never get the protocol, because the executable path will not
  >    match any `.desktop` file. Anyone debugging the Dock will hit this and it will look
  >    like a bug in our code. The development escape hatch is
  >    `KWIN_WAYLAND_NO_PERMISSION_CHECKS=1` — set on **KWin**, requiring a compositor
  >    restart, not on our client.
  > 4. **Bonus for M6.** `org_kde_plasma_activation_feedback` is the launch-feedback protocol
  >    and is behind the same gate, so declaring it costs nothing extra now — see §6.4, the
  >    bounce animation.
  > 5. **Correction:** the `[reported]` claim above that KWin also supports
  >    `ext-foreign-toplevel-list-v1` is **false for 6.7.3** — `grep -r foreign_toplevel src/`
  >    in KWin's tree returns nothing. `plasma-window-management` is the only option on KDE.
  >
  > Version caveat: measured on 6.7.3 only. The blacklist is a KWin implementation detail and
  > could change; the `X-KDE-Wayland-Interfaces` mechanism is the stable, documented part.
- **wlroots (Sway, Hyprland)** — `zwlr_foreign_toplevel_management_v1`: list **and** activate,
  in one protocol. `[reported]`
  — https://wayland.app/protocols/wlr-foreign-toplevel-management-unstable-v1
  — reference client: https://github.com/swaywm/wlroots/blob/master/examples/foreign-toplevel.c
- **`ext-foreign-toplevel-list-v1`** — the standardised, cross-compositor protocol. In wlroots
  since 0.18.0. **It is list-only: it does not provide activation.** So it can never be the
  whole answer by itself. `[reported]`
  — https://wayland.app/protocols/ext-foreign-toplevel-list-v1
- **GNOME / Mutter** — exposes no toplevel list to ordinary clients. Requires a **companion
  GNOME Shell extension** publishing a D-Bus interface. This is a whole second deliverable
  (JavaScript, GNOME's review process if distributed via extensions.gnome.org). `[reported]`
- **X11** — EWMH: `_NET_CLIENT_LIST`, `_NET_WM_PID`, `_NET_ACTIVE_WINDOW`. Trivial by
  comparison. `[verified — long-standing standard]`
- **Launching** apps (as opposed to focusing them) is `xdg-activation-v1` plus `.desktop` file
  handling; KWin implements xdg-activation. `[reported]`

**Design consequence:** the `WindowSource` trait needs *at least* list, subscribe-to-changes,
and activate. Because `ext-foreign-toplevel-list-v1` cannot activate, do not design around it
as the primary path — design around the per-compositor protocols and treat `ext-` as a
partial fallback.

### 3.1 Window pictures on KDE (2026-10-22) `[verified — read from the Mac's installed KWin 6.8.0 and its .desktop files; capture itself not yet exercised]`

KWin offers two ways to get another client's window as pixels, and **both are gated by the
same `.desktop` mechanism as the window list above — no prompt, ever**, once the installed
executable declares them:

- **`org.kde.KWin.ScreenShot2`** (D-Bus), method `CaptureWindow(handle, options, pipe fd)`:
  one still, full resolution, written raw into the pipe the caller passes. `handle` is the
  window's UUID — the same one `window_with_uuid` already gives the agent. Allowed when the
  caller's executable has a `.desktop` file with
  `X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2`. Spectacle, plasmashell and the
  xdg portal backend declare exactly this. KWin identifies the caller by its process, so
  shelling out to `gdbus`/`qdbus` would be checked against *their* executables and refused:
  the call must come from `listello-agent` itself.
- **`zkde_screencast_unstable_v1`** (Wayland), `stream_window(uuid)`: a PipeWire stream that
  delivers a frame whenever the window changes. Gated by `X-KDE-Wayland-Interfaces`. This is
  what Plasma's task-manager tooltips use. Needs a PipeWire client — a new dependency and far
  more code.

**Exercised 2026-10-22 on the Mac** (KWin 6.8.0): with the `.desktop` line, `CaptureWindow`
answers at once with no prompt. Chrome windows came back as format 18
(`RGBA8888_Premultiplied`), Konsole as a native-word format; the agent handles the 4/5/6 and
16/17/18 families. Without the line the error is `org.kde.KWin.ScreenShot2.Error.NoAuthorized`.
Minimised windows are not asked for at all (KWin does not paint them); they get a placeholder.

**Cost of `"snapshots"`, measured** `[verified — 2026-10-22, MacBook T2, three Chrome
windows, row open for a minute]`. One capture of a ~2000×1800 window is ≈16 ms, nearly all of
it KWin rendering the window and writing ≈14 MB into the pipe; the agent reduces straight from
those bytes to the strip-sized picture. A first version that decoded and scaled the whole frame
with cairo took ≈60 ms per window and kept the agent near 40 % of a core with the row open.
Measured per phase, as % of one core:

| Row | KWin | agent | core | GPU busy |
|---|---|---|---|---|
| closed | 1.0 | 0 | 0 | 0 |
| first 10 s (every 1.5 s) | 3.1 | 2.6 | 0.7 | 1.1 |
| 10–60 s (every 5 s) | 0.8 | 0.5 | 0.1 | 0 |
| after 60 s (no refresh) | 0.2 | 0.2 | 0.1 | 0 |

So a row open for a few seconds costs a few percent of one core for those seconds: not a
battery concern.

**Cost of `"live"`, measured** `[verified — 2026-10-22, MacBook T2, KWin 6.8.0, PipeWire
1.6.9, a row of three Chrome windows]`. With `zkde_screencast_unstable_v1` declared in the
`.desktop` file, KWin binds the global with no prompt and `stream_window(uuid)` answers with a
PipeWire node at once. The agent offers BGRx/BGRA/RGBx/RGBA in shared memory (no modifier, so
no DMA-BUF) with `maxFramerate` 10/1, and reduces each frame from the mapped buffer with the
same code as the stills. KWin sends a frame only when the window changes. As % of one core,
2-second samples:

| Row | KWin | agent | core |
|---|---|---|---|
| closed | 0–1 | 0 | 0 |
| open, windows still | ≈1 | 0 | 0 |
| open, windows changing | 5–18 | 2–8 | 1–3.5 |

So a still row costs nothing, and a changing one several times what snapshots cost, nearly all
of it KWin reading the window back. Closing the row ends every stream on both sides (checked
with `pw-dump`).

## 4. Mapping windows to apps and icons

A window gives you an `app_id` (Wayland) or WM_CLASS (X11). Turning that into a name and an
icon means matching it to a `.desktop` file in the XDG data dirs, then resolving the icon name
through the XDG icon theme (`freedesktop-icons`, already a tiny-dfr dependency).

**This match is imperfect and it is a known, notorious source of bugs** — `app_id` frequently
does not equal the `.desktop` basename (Electron apps, Flatpaks, Snaps, browsers, JetBrains
IDEs). Expect to need a fallback chain and a user-overridable mapping table in config.
`[assumption — but this is well-established folklore across every Linux dock project]`

## 5. The other widgets' Linux equivalents

| Pock feature | Linux mechanism | Difficulty |
|---|---|---|
| Now Playing / media info | **MPRIS** over D-Bus (`org.mpris.MediaPlayer2`) | easy |
| Volume | PipeWire (`wpctl`) or the PipeWire API | easy |
| Screen brightness | logind D-Bus `SetBrightness`, or sysfs `/sys/class/backlight` | easy; **verify T2 backlight works under Linux** |
| Battery | UPower over D-Bus, or `/sys/class/power_supply` | easy |
| WiFi status | NetworkManager over D-Bus | easy |
| Lock the session | logind D-Bus `LockSession`, or the DE's screensaver D-Bus interface | easy |
| Dock badge counts | Unity launcher D-Bus API (`com.canonical.Unity.LauncherEntry`) | partial parity at best — only some apps implement it |
| Open a folder | `xdg-open` / `gio open` on the path | easy |
| Trash | XDG trash spec, `~/.local/share/Trash` (count files for a full/empty indicator) | easy |
| App launch "bounce" | needs launch-in-progress tracking: xdg-activation token issued → window appears | **the fiddliest 1.0 item**; see open questions |

## 6. Open questions — resolve these before or during planning

1. ~~**What does tiny-dfr's source actually look like?**~~ **ANSWERED — see §2.1.** ~35% fork,
   ~65% new; the fork covers the hardware risk. Produced two new decisions: D2a (upstream asks
   AI agents to stay out) and D11 (two processes).
2. **Real panel geometry and DPI on MacBookPro16,1 under Linux.** Affects layout, icon sizes,
   and the simulator's default. Cannot be answered without hardware.
3. **Does KWin's `org_kde_plasma_window_management` really give us icons and activation, and
   is there a usable Rust binding** (`wayland-client` + generated protocol bindings), or do we
   generate them ourselves from the XML?
   **ANSWERED for the protocol, still open for the bindings.** On KWin 6.7.3 the protocol is
   gated behind a `.desktop` file declaring `X-KDE-Wayland-Interfaces` — see §3 for the
   measurement, the source, and the end-to-end proof. It gives list, `app_id`, title, icon,
   minimised state and activation, so D4's KDE-first ordering is confirmed rather than
   overturned.
   **Bindings answered too, 2026-08-03.** `wayland-protocols-plasma` (crates.io, MIT, part of
   the Smithay `wayland-rs` family) ships pre-generated bindings for `plasma_window_management`
   — and 26 other Plasma protocol extensions, including `plasma_activation_feedback` for
   M6 — on top of `wayland-client`/`wayland-server` 0.31.x. Neither hand-writing nor a
   project-local `wayland-scanner` build step over a vendored XML earns its keep against an
   already-maintained crate in the same ecosystem `listello-agent` will use for the Wayland
   connection itself. `listello-agent` depends on it directly when M4 starts; no XML is
   vendored into this repo. Issue #13 closed.
4. **How do we detect "an app is launching" for the bounce animation?** Candidates: hold the
   xdg-activation token we issued and animate until a matching window appears; time out after
   N seconds. Needs verification that KWin's behaviour makes this observable.
   **A better candidate turned up on 2026-08-03:** KWin has
   `org_kde_plasma_activation_feedback`, which is exactly this — the protocol Plasma's own
   task manager uses to bounce a launcher icon. It sits behind the same
   `X-KDE-Wayland-Interfaces` gate as `plasma-window-management` (§3), so the agent gets it
   by declaring one extra name in a `.desktop` file it has to ship anyway. Verified present
   and bindable on KWin 6.7.3 (global name 27). Its shape has not been read yet — that is
   M6's job, not a blocker. `[verified — measured]`
5. **Does screen backlight control work on T2 Macs under Linux?** **LARGELY ANSWERED.**
   tiny-dfr's `backlight.rs` already reads and writes `/sys/class/backlight`, probing
   `apple-panel-bl`, `gmux_backlight`, `intel_backlight` and `acpi_video0` for the display, and
   `appletb_backlight` for the Touch Bar itself. This is shipping, packaged code, so sysfs
   brightness control on T2 is evidence rather than hope. `[verified — read from source]`
   *Residual:* which of those four nodes the MacBookPro16,1 actually exposes, and whether
   writing to it as `nobody`:`video` works, are hardware questions for M2. Note that under D11
   the *renderer* keeps this sysfs path — brightness does not need the session agent.
6. **Kernel/distro baseline.** Which kernel does the chosen distro ship, does it have
   `appletbdrm` working, and does it need the t2linux patched kernel? Affects install docs.
7. **Multi-window titles from a Wayland toplevel** — confirm title changes are pushed as
   events so the row stays live.
8. **Which distro will actually be installed on the MacBook?** **Leaning Fedora**, via
   t2linux, with KDE. Not final and not yet installed — a large amount of data has to move off
   the Mac before it can dual-boot, so M2 is deliberately deferred until that is done.
   Plan for Fedora (kernel version, packaging in M6, install docs) but do not hardcode it
   anywhere. (t2linux also documents Ubuntu/Debian, Arch and NixOS paths if this changes.)

## 7. Sources

Pock:
- https://github.com/pock/pock — Swift, MIT, ~10.1k stars, latest release v0.9.0-22
- https://pock.app/ and https://pock.app/docs/ — PockKit widget development docs
- Default widgets: Dock (with badges and multi-window), Now Playing, ESC, Status
  (time/WiFi/battery), Control Centre (brightness/volume sliders), Weather

Touch Bar on Linux:
- https://wiki.t2linux.org/guides/postinstall/ — the T2 install/post-install guide
- https://github.com/t2linux/apple-ib-drv
- https://lwn.net/Articles/986150/ — Touch Bar support for T2 Macs
- https://lwn.net/Articles/1022733/ — HID multitouch for x86 Touch Bars
- https://www.phoronix.com/news/Apple-Touch-Bar-Display-Linux — appletbdrm + adp in 6.15
- https://github.com/t2linux/wiki/issues/635 — appletbdrm bind failures
- https://github.com/basecamp/omarchy/discussions/5862 — suspend/resume + tiny-dfr fix

tiny-dfr:
- https://github.com/WhatAmISupposedToPutHere/tiny-dfr
- https://crates.io/crates/tiny-dfr
- https://packages.debian.org/experimental/main/tiny-dfr

Wayland protocols:
- https://wayland.app/protocols/ext-foreign-toplevel-list-v1
- https://wayland.app/protocols/wlr-foreign-toplevel-management-unstable-v1
- https://github.com/swaywm/wlroots/blob/master/examples/foreign-toplevel.c
