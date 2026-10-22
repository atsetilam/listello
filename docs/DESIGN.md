# Design decisions

The decisions listello is built on, starting from the initial scoping (2026-08-03). Each
carries its reasoning, so a later change can be told apart as a correction or a drift.
Reversing one is a deliberate decision, not an implementation choice. Facts established later
— a measurement, a proof, a constraint the hardware imposed — are recorded under the decision
they belong to, dated.

---

## D1. This is a reimplementation, not a port

Pock is Swift + AppKit. Its widgets are compiled `.pock` bundles linking `PockKit`, which
links AppKit. There is no realistic path to running those binaries on Linux — it would mean
reimplementing AppKit. Binary plugin compatibility was **explicitly rejected**.

We reimplement Pock's *behaviour*, and we read Pock's source and docs as a specification of
what that behaviour is.

**Rejected alternatives:** binary compatibility (months-to-years, likely a dead end); mimicking
PockKit's API shape for easy porting by widget authors (no benefit once the ABI is gone).

## D2. Rust, forking tiny-dfr

[tiny-dfr](https://github.com/WhatAmISupposedToPutHere/tiny-dfr) (MIT AND Apache-2.0, Asahi
Linux project) already solves the hard, hardware-specific half: opening the Touch Bar DRM
device, drawing to it, reading libinput multitouch events, dropping privileges, handling
suspend/resume, and doing all of that on **both** T2 Intel Macs and Apple Silicon. It is Rust,
small, and proven.

We fork it and grow a widget system on top, rather than starting from scratch or depending on
it as an external process. Only one process can own the Touch Bar DRM device, so coexisting
with tiny-dfr is not possible — we **replace** it.

Rust was chosen for fitness for purpose. The Linux plumbing this needs — DRM/KMS,
evdev/libinput, Wayland protocols, D-Bus — has
mature first-class Rust crates and effectively nothing in Go or Python. It compiles to a small
always-running daemon with no GC, which matters on a laptop battery.

**Rejected alternatives:** Go (simpler to write, but no tiny-dfr fork and we would
hand-roll DRM/Wayland/evdev bindings); C++ (viable, weaker ecosystem for these interfaces);
Python (bad fit for a low-latency graphics daemon).

### D2a. Upstream asks AI agents to stay out; we fork, and stay out

tiny-dfr's repository asks that automated agents and AI systems not modify, analyse or
contribute to it. listello is written almost entirely with AI. That request is not a licensing
restriction — MIT and Apache-2.0 grant the right to fork, read and modify, and a file in a
repository cannot revoke a granted licence — so listello forks tiny-dfr, knowingly, and
respects the request in the only way left open: it never takes part in tiny-dfr's repository.
The obligations that follow are not optional:

1. **Attribution louder than MIT requires.** The README credits The Asahi Linux Contributors
   and tiny-dfr prominently and above the fold, not only in `LICENSE`. Both licence files
   (`LICENSE` MIT, `LICENSE.material` Apache-2.0) ship verbatim. Every file retained
   substantially unchanged from tiny-dfr carries a header saying so, and upstream's git history
   is merged into this repository and never rewritten.
2. **We never upstream.** No issues, no pull requests, no comments, no email, no upstreamed
   fixes. Any fix made to the DRM, backlight or input paths stays in this fork permanently.
3. **The provenance is stated plainly.** The README describes the fork honestly, including
   that listello is AI-written and what tiny-dfr asked.

**Accepted cost:** no upstream channel, and possibly ill will from tiny-dfr's maintainers. **Why accepted:** the alternative is hand-rolling DRM
atomic modeset, libinput seat handling and T2 backlight quirks — months of work on the one part
of the project that is already solved.

## D3. Single-threaded event loop, widgets as pure functions

This is the load-bearing architectural decision, and it was chosen to satisfy three
constraints at once.

- **One event loop, single-threaded, message passing over channels.** Sources of change
  (window list updates, MPRIS events, battery/wifi polls, touch input, timers) send messages;
  the loop owns all state and is the only thing that mutates it.
- **Widgets are pure functions: state in, draw commands out.** A widget does not hold a handle
  to the core, does not call D-Bus itself, does not know about DRM. It receives an immutable
  snapshot of the state it declared interest in, plus its allotted geometry, and returns draw
  commands and hit regions.

Why this shape:

1. **It is the most borrow-checker-friendly shape available.** Almost no shared mutability
   means almost no `Arc<Mutex<...>>`.
2. **It makes a widget cheap to add and to test.** Each widget is a new file with no callers,
   testable by rendering it against a fixed state snapshot and comparing to a golden image.
3. **It keeps the plugin-API door open.** The widget trait is deliberately shaped like a
   scripting boundary. Adding Lua or WASM widgets post-1.0 becomes a new backend behind an
   existing interface rather than a rewrite.

**Explicitly deferred:** scripting-language widgets (Lua/WASM) from day one were rejected for
1.0. Rationale: it would freeze the public plugin ABI before a single widget exists to tell us
what the ABI needs (Pock got PockKit right by extracting it from its own built-in widgets
*after* they existed); the boundary is rich, not thin (window lists, live icon pixel data,
scroll state, click routing, badge counts); and it optimises the wrong half — the genuinely
hard Rust is the core (DRM, input, Wayland backends, text and icon rendering, layout,
compositing), which stays in Rust either way, while widgets are the easy half.

## D4. Pluggable window sources, staged by desktop environment

The Dock widget must enumerate running applications and their windows, and focus a chosen
window. Linux has no single way to do this. So: **one `WindowSource` trait, several backends,
selected at runtime by probing what the session offers.**

Target order:

1. **KDE Plasma (Wayland)** — first testing target and the reference implementation.
2. **GNOME (Wayland)** — needs a companion GNOME Shell extension; it is the only major desktop
   that exposes no window list to ordinary clients.
3. **wlroots compositors (Sway, Hyprland)** — nearly free once the trait exists.
4. **X11 (any DE)** — legacy fallback, easiest of all technically, lowest priority.

The aim is any desktop, primarily on Wayland. X11 is a fallback, not a design centre.

See [RESEARCH.md](RESEARCH.md) §3 for which protocol each backend uses and what is still
unverified.

## D5. Hardware: design for both, target T2 first

The reference machine is a **MacBookPro16,1** (16-inch 2019, Intel, T2 chip) — that is the
reference hardware and the first target. Apple Silicon (M1/M2 under Asahi Linux) must remain
supported because tiny-dfr already abstracts both and the Apple Silicon audience is larger and
growing. Do not write anything that assumes T2 unless the abstraction genuinely has to differ.

Consequence of the specific model: **the MacBookPro16,1 has a physical Esc key and physical
Touch ID**, so Pock's ESC widget is unnecessary here. Do not port it. (Earlier 13/15-inch
Touch Bar models lack the physical Esc — if we ever support them, an ESC widget becomes
relevant again. Post-1.0.)

## D6. Fn row behaviour

Widgets are shown normally. **Holding Fn temporarily swaps in F1–F12**, exactly as both Pock
and tiny-dfr do. This is the behaviour we inherit from tiny-dfr and must not regress: a user
who installs this in place of tiny-dfr must not lose their function keys.

*Recorded 2026-10-22, not a change of decision:* the layer that is shown by default is a config
choice. `[[widget]]` is the default strip and `[[fn_widget]]` replaces it while Fn is held, so
media keys first and F1–F12 under Fn (tiny-dfr's `MediaLayerDefault = true`) is just the two
lists swapped.

## D7. No live window thumbnails

Pock shows live previews of an app's windows when you tap a multi-window app. On Wayland,
capturing other clients' windows is deliberately restricted and permission-gated.

**Decision: icon + window title is fine.** Tapping a multi-window app shows a row of labelled
entries; selecting one focuses that window. This works on every desktop, needs no screen
capture permission, and costs no battery. Live thumbnails are post-1.0 at best.

**Amendment 2026-10-22: the window row takes the whole Touch Bar.** Where Pock keeps
its other widgets beside the previews, listello's row of windows replaces the entire strip
until it is closed or a window is picked; the back tile brings everything back. The row's
look is settled: the app's icon once, on the back tile, then its name and window count, then
one entry per window with the app's name trimmed from the titles and long titles shortened in
the middle. Live thumbnails, if they come, are an option on top of this, not a replacement.

**Amendment 2026-10-22, second: thumbnails become an option, in three modes.**
Reading KWin (RESEARCH §3.1) removed the premise above: on KDE an installed agent gets window
pictures with no permission prompt. So the window row gains a preview per window, beside its
title, chosen by `[strip] window_previews`:

- `"off"` — the row exactly as it is today. **The default**, until the measurement below says
  otherwise.
- `"snapshots"` — a still per window when the row opens, refreshed every 1–2 s while it stays
  open (KWin `ScreenShot2`). Built first.
- `"live"` — a video stream per window (KWin screencast + PipeWire). Built the same day; see
  below.

Nothing is captured while the row is closed, in any mode. Before the settings app ships, the
modes are measured (CPU and GPU load, row open) so it can say which one costs battery. Other
desktops, and KDE when the agent is not allowed to capture, fall back to the row without
previews.

**Built the same day (`"snapshots"`), with the choices made trying it on the panel.** A
whole window shrunk to strip height is only shapes, and windows of one app look alike, so
`[strip] preview_style` places the picture: `"fill"` (default) fills the entry
with a full-width band through the middle of the window, 6:1 like the entry, darkened under
a readable title — a reminder of the window, not text to read; `"whole"` keeps the small
whole-window picture beside the title; `"stretch"` squeezes the whole window behind the
title. Refreshes are every 1.5 s for 10 s, every 5 s to a minute, then none. Measured cost
(RESEARCH §3.1): a few percent of one core while a row is open, nothing when closed.

**`"live"` built and approved on the panel the same day.** One PipeWire stream per window
while the row is open, a frame only when the window changes, **at most ten a second**,
for as long as the row stays open; every stream ends when it closes. The row
opens with one still of each window so it is never blank while streams start, and a window
whose stream cannot be had, fails or breaks gets stills as in `"snapshots"`, so live is never
worse. Minimised windows lose their stream and get the placeholder. Cost (RESEARCH §3.1):
nothing for still windows, up to about a fifth of a core (mostly KWin) while they change.
`clang` becomes a build dependency (the PipeWire bindings) and `libpipewire` a runtime one.

## D8. Configuration: file first, GUI later

TOML config, in tiny-dfr's tradition and extending its existing config. A graphical settings
app comes after the core works, and is not part of 1.0.

Nothing about anyone's personal layout may be hardcoded. The author's layout (see ROADMAP M5)
was the first implementation target — a *test case*, not a default that other users are stuck
with.

**Amendment 2026-10-22: the settings window, per-user layouts, one language, the tray icon.**
The GUI is now designed. What follows is the decision; how to build it is not.

*The window.* A standalone app, `listello-settings`, in the application menu, looking like a
KDE app (Qt/Kirigami is the obvious fit on Plasma). At the top, **one interactive picture of
the strip** as it will look, drawn by the same shared rasteriser that draws the panel — never a
second imitation of it. Click a widget in the picture to edit its options in the area below;
drag it with the mouse to move it; an "Add" row offers the widget kinds; each widget's options
have a Remove. Three sections: **Strip**, **Fn keys** (the same editor for the Fn layer) and
**General**. There is no before/after view: the Touch Bar itself is the "before".
**Nothing reaches the Touch Bar until Apply.** Beside Apply: Cancel (drop the edits) and
"Restore the default" (drop this user's layout so the machine's applies again). The window
refuses to Apply a layout that does not validate.

*Per user, no password.* A user's layout lives in `~/.config/listello/config.toml`, in the same
format as `/etc/listello/config.toml`. The agent reads it and hands it to the renderer; the
renderer shows it while that user's agent is connected, and shows the machine's layout
(`/etc`, else `/usr/share`) at the login screen, after logout, or whenever no agent is there —
so the "never black" rule (D11) is untouched. A user who never opened the settings sees the
machine's layout. A broken user file leaves the machine's layout up and is reported, never a
crash. The Dock's pinned list in the window *is* the user's `pins.toml`, the same list the
Touch Bar's own pin button edits. `[core]` (backlight and other hardware settings) stays
machine-only: it belongs to the device, not to a user.

*One language setting.* `language` in General: **follow the system** (the default), Italian
or English — the two the app speaks for now. It governs the settings window, the tray menu
and the text listello itself puts on the strip (the clock's day and month names: "gio", not
"thu"). A clock's own `locale` stays accepted in the file as an override for that clock, so
existing configs keep working; the window does not show it. In the file it is
`[general] language = "system" | "italian" | "english"`. *Follow the system*
(2026-10-22): the clock follows the system's date-and-time format (`LC_TIME`, else `LANG` — KDE's
"Time" setting), while the window and the tray follow the display language; the two can differ
(English display, Italian time format). "The system" is the connected
user's session; with no agent it is the machine's `/etc/locale.conf`.

*The tray icon.* Off unless the user turns it on in General. Its menu: **Open settings**,
**Reload listello** (makes the Touch Bar start over, for when it misbehaves), and **Check for
updates** — which stays hidden while there is no public release to check against (D9), and
appears when there is. Everything the tray offers is also in the window.

**Second amendment 2026-10-22: listello is something a user starts and stops.**
Seeing the tray, it was decided that listello must be stoppable as a whole, and must not start by
itself just because it was installed or started once.

*Stop listello* (tray menu, and General beside Reload) stops **everything** of listello's: the
agent, the tray, the settings window and the renderer. The Touch Bar then goes back to the keys
the Mac's own T2 chip draws (special keys; F1–F12 with Fn), which is what Arch shows with no
Touch Bar program. The only way back is the application menu, or the next login when starting
at login is on. There is no "Start" item: a stopped listello has no tray.

*The application menu entry* starts listello if it is stopped and opens the settings window.

*Starting at login* is the user's choice, never assumed. The very first time a user starts
listello, a small question comes before the settings window: "Start listello every time you
log in?" with **Yes, every time** / **Only now**. Afterwards it is a checkbox in General
("Start at login"). Installing the package turns nothing on for any user.

*No user running listello* (the login screen, after logout, after Stop, or a session that did
not start it) means the renderer is not running and the T2 draws its own keys. This replaces
"the machine's layout at the login screen" above: the machine's layout (`/etc`, else
`/usr/share`) is now what a user running listello sees until they have a layout of their own.

*Measured on a MacBookPro16,1 (T2), 2026-10-22:* with the renderer and agent stopped and the
panel left in USB configuration 2 (what tiny-dfr's udev rule, retained in
`99-touchbar-listello.rules`, selects so that `appletbdrm` gets the panel), the Touch Bar is
**black**, also with `appletbdrm` unloaded, also with Fn held. Writing `1` to the panel's
`bConfigurationValue` (USB `05ac:8302`, at `/sys/bus/usb/devices/7-6` on that machine) brings the
T2's own keys back within two seconds, `hid-appletb-kbd` binds, and Fn shows F1–F12. So
stopping the renderer must also put the panel in configuration 1, and starting it must select
2 first. Apple Silicon Touch Bars have no firmware fallback; there the renderer has to stay up
as before (D11).

*How it is built (2026-10-22).* The agent's unit is never enabled by the package; "Start at
login" is `systemctl --user enable/disable listello-agent`, and the first-start question leaves
a marker, `asked-start-at-login`, in `$XDG_STATE_HOME/listello/`. Opening the settings window
(what the menu entry does) asks that question if the marker is missing, starts the agent if it
is stopped, and closes itself when the agent's unit is stopped. The agent starts the tray as its
own child when `[general] tray = true`, so stopping the unit takes the tray with it. The unit's
`KillSignal` is `SIGUSR2`: the agent then sends the renderer `AgentToCore::Quit` (protocol 9)
and exits, so the T2's keys are back at once on Stop and on logout, instead of after the grace
period. The renderer (`listello-core`) is started by the agent connecting to
`listello-core.socket`; `ExecStartPre=+` / `ExecStopPost=+` run `listello-core --panel-mode`,
which selects USB configuration 2 and waits for the panel's devices before it starts, and
configuration 1 after it ends (except on a reload, which exits 75 and restarts at once). With
no agent connected for 8 seconds the renderer exits. A machine with no `05ac:8302` (Apple
Silicon) keeps the old behaviour: udev starts it and it stays up. `BindsTo` on the display device
is gone, since that device only exists after configuration 2.

## D9. Repository and release

- **Private until it runs on the reference machine**, then public.
- **GitHub is the source of truth.**
- **The release gate is a human test on real hardware.** Automated tests and golden images
  gate every change, but a release is something that was used on a Touch Bar.
- Intended license: MIT (matches both Pock and tiny-dfr; tiny-dfr's embedded
  material-design-icons are Apache-2.0, so a fork must carry both notices).

## D10. The simulator is milestone 1

When the project started, the reference MacBook still ran macOS, and Linux was not going on it
for a while. Without a simulator nothing the project did could be seen before then.

So the first milestone is a **Touch Bar simulator**: a window on the desktop that renders
exactly what the real Touch Bar would, driven by the same widget and layout code, with mouse
clicks and scroll standing in for touch. It runs on macOS and Linux.

This is not a nicety. It is:

- the only way to look at the strip before Linux is on the Mac;
- the gate for all rendering work (render to buffer → compare golden image);
- what makes most widget work testable on any machine, including a headless one.

Consequence: the rendering layer must be **backend-agnostic from the first commit**. Draw
commands go to a `Surface`; `DrmSurface` and `SimulatorSurface` are both implementations.
Never let DRM specifics leak into widget or layout code.

### D10 addendum — what the simulator is made of, and the crates M1 splits out

*Added 2026-08-03, while reviewing the M1 plan. The toolchain choice is an implementation call
made inside D10, not a new decision; the measurements under it are facts and are recorded as
such. The crate split below is a **consequence** of D10 meeting the Mac's limits, not a reinterpretation of it.*

**`SimulatorSurface` is `winit` 0.30 + `softbuffer` 0.4.** CPU blit, no GPU dependency,
matching the `cairo::ImageSurface` → `memcpy` shape RESEARCH §2.1 found in tiny-dfr's real
path. Both are cross-platform (macOS, X11, Wayland).

The load-bearing question was ownership: winit's normal mode *takes over* the main thread and
calls back, which inverts D3's single-threaded loop. It does not have to.
`EventLoopExtPumpEvents::pump_app_events(Some(timeout), &mut app)` polls and returns, so
**our** loop keeps ownership and winit becomes one more source drained per iteration, exactly
like the libinput fds. `[verified — winit 0.30.13 + softbuffer 0.4.8, macOS, 2026-08-03: a
scratch program pumped the loop 40× with a 50 ms timeout, got a window and a working
`softbuffer` present, and returned control every time]`.

Two measured details worth knowing:

- **The window does not exist on the first pump.** `ApplicationHandler::resumed` fires
  asynchronously; a pump with a zero timeout returns before any window is created. Code must
  treat "no window yet" as normal, not as an error.
- **`Window::inner_size()` is in *physical* pixels**, so on a Retina Mac a default window came
  back 1600×1200. The simulator sizes its buffer from the physical size and must map mouse
  coordinates through the scale factor, or every hit test is off by 2× on a Retina display.

*What would reverse this:* `pump_app_events` losing macOS support, or the panel-sized window
proving unusable on Wayland. Either would reopen D10 itself.

**Two new crates, because `listello-core` cannot build on the Mac and never will.**

```
crates/listello-ui/    Widget trait, hit regions, layout, widgets, config schema, the Shell
crates/listello-sim/   the simulator binary: owns the winit pump, assembles ui + surface
```

`listello-ui` depends on `listello-surface` and `listello-protocol` and **nothing else that is
Linux-only** — never `input-linux`, `udev`, `drm` or `libinput`. That is what makes it build,
test and gate on the Mac, and it turns D3's "a widget never holds a handle to the core" from a
rule anyone can break by accident into one the crate graph refuses to compile.
`[verified — `input-linux` 0.7 fails to build on macOS: `input-linux-sys` cannot find the
Linux uapi types. Checked 2026-08-03.]`

M1's exit criterion is *"one command on the Mac shows the strip"*. Putting that binary in
`listello-core` — which needs libinput, uinput, udev and DRM headers — would make the
criterion unreachable by construction. `listello-core` and `listello-sim` are both thin: each
owns its platform's fds and its `Surface`, and both drive the same `Shell` in `listello-ui`.

## D11. Two processes: a privileged renderer and a session agent

*Added 2026-08-03, after reading tiny-dfr's `src/`.*

**The problem this solves.** tiny-dfr drops privileges to user `nobody`, groups `input` and
`video`, before entering its loop. That is correct for tiny-dfr, which needs only the DRM
device and libinput. It is **fatal for the Dock (M4) and for a full layout (M5)**: `nobody`
cannot talk to the user's Wayland compositor, cannot reach the session D-Bus (UPower,
NetworkManager, logind, MPRIS), cannot `xdg-open` a folder as the user, and cannot read
user-installed `.desktop` files or icon themes under `~/.local/share`.

**The decision.** Two processes.

| | **Renderer** (`-core`) | **Session agent** (`-agent`) |
|---|---|---|
| Runs as | system service; drops to `nobody`:`input`,`video` (tiny-dfr's existing path, kept) | the user, in their graphical session (`systemd --user`) |
| Owns | the Touch Bar DRM device, libinput touch, uinput, backlight, pixel shift, the event loop, layout, widget rendering | Wayland compositor protocols, session D-Bus, `.desktop` and icon resolution, `xdg-open` |
| Knows nothing about | the desktop session | DRM, drawing, layout |

**Why this costs less than it looks.** D3 already says: sources of change send messages; the
loop owns all state and is the only thing that mutates it. The session agent is simply **one
more message source that happens to live in another process**. The event loop's shape does not
change. Widgets do not know the agent exists — they still receive an immutable state snapshot.

Consequences that are binding:

- **The message enum is the wire protocol.** Define it once in a shared crate. Two directions:
  `AgentToCore` carries state (window lists, battery, wifi, icon data); `CoreToAgent` carries
  actions (activate this window, launch this app, open this folder, lock the session).
- **Icons cross the boundary as pixels, not paths.** The agent resolves an `app_id` to an icon
  and ships decoded pixel data; the renderer caches it keyed by `app_id`. `nobody` can read
  `/usr/share/icons` but not the user's own icon theme, so paths are not portable across the
  boundary. Do not design a protocol that sends paths.
- **The renderer must run, and be useful, with no agent connected.** Function keys, media keys,
  and the Fn layer are the renderer's own job and must never depend on the session. If the
  agent dies or the user logs out, the Touch Bar degrades to tiny-dfr's behaviour — it does not
  go black. This is also what makes M1 and M3 possible before any agent exists.
  *(Narrowed on T2 Macs by D8's second amendment, 2026-10-22: with no user running listello
  the renderer does not run at all, and the T2 chip's own keys take the panel back. While a
  user runs listello and the agent only blips, a crash or a reload, this rule stands.)*
- **Transport:** a Unix socket at a fixed path, systemd socket activation, length-prefixed
  serialised messages, peer credentials checked on connect. Not D-Bus — putting the renderer on
  the system bus as `nobody` means shipping bus policy files, and buys nothing here.
  *Settled in M1 step 2, when the types were written:* the codec is
  [postcard](https://docs.rs/postcard) behind a little-endian `u32` length prefix, and the
  prefix is capped at 8 MiB **before** any buffer is grown — the peer chooses that number, so
  it must not be allowed to choose an allocation in a privileged process. Postcard is not
  self-describing and has no schema evolution, which is why `PROTOCOL_VERSION` is exchanged
  first and a mismatch closes the socket instead of negotiating: both binaries ship from this
  repository and are installed together, so skew is a bug to surface, not a case to tolerate.
- **The control centre is key events, not agent actions.** M5's mute, volume and brightness
  buttons go out through uinput from `listello-core`, exactly as tiny-dfr already emits media
  keys — routing them through the agent would make half the control centre stop working when
  the user logs out, which is the failure this decision exists to prevent. `SystemAction` has
  one variant, `LockSession`, because logind is a session service and there is no key event
  for it. Keep it close to one.
- **The agent must be installed to work at all, and must ship a `.desktop` file.** See the
  amendment below.

**Amendment, 2026-08-03: the agent's privileges come from being installed.**

Measured on KWin 6.7.3 and proven end to end (RESEARCH §3): KWin keeps
`org_kde_plasma_window_management` — and `org_kde_plasma_activation_feedback`, which M6's
bounce animation needs — on a blacklist, and advertises them only to a client whose executable
has a `.desktop` file declaring them in `X-KDE-Wayland-Interfaces`. The match is the canonical
path of `Exec=`'s first token against the client's `/proc/<pid>/exe`.

So the agent does not earn the window list by being a well-written Wayland client. It earns it
by **being installed where its `.desktop` file says it is**. Binding consequences:

- **`listello-agent` ships `/usr/share/applications/org.listello.Agent.desktop`**, with
  `Exec=` pointing at the installed binary and
  `X-KDE-Wayland-Interfaces=org_kde_plasma_window_management,org_kde_plasma_activation_feedback`.
  That file is not packaging metadata that can be added later — it is a functional dependency
  of the Dock, and it belongs in the same commit as the KDE `WindowSource`.
- **A developer running `cargo run` gets no windows, silently.** The global is simply absent
  from the registry; there is no error and no permission denial to log. This will be mistaken
  for our bug — by us. The agent should therefore **detect the absence and say so plainly**
  ("no window-management protocol: listello-agent is running from an uninstalled path, or its
  .desktop file is missing") rather than reporting an empty window list. Development escape
  hatch: start KWin with `KWIN_WAYLAND_NO_PERMISSION_CHECKS=1`.
- **Packaging (M6) is therefore not purely post-1.0 polish.** A working installed path has to
  exist by M4, when the agent first does anything.
- **This does not change the D11 split**, and it does not change what crosses the socket. The
  renderer still runs with no agent, and the agent still knows nothing about DRM. What changes
  is that the agent's *deployment* is part of its contract with the compositor, which is a
  constraint the original decision did not anticipate.

The mechanism is KDE-specific but the shape may not be: GNOME already needs a shell extension
the compositor trusts (D4), and both are the same underlying fact — **a compositor decides
which clients may see other clients' windows, and the answer is never "any client that asks"**.
Expect every `WindowSource` backend to carry some install-time requirement, and do not design
one that assumes otherwise.

**Consequence for milestones:** M1 defines the message types and builds the loop around them,
but ships **no agent** — the simulator feeds stub state directly into the loop. The agent
process itself first appears in M4, where it is needed. The boundary must exist in M1's types;
the second binary need not.

**Settled in M4 (2026-10-22), not a change of decision:**

- *One agent at a time.* The renderer accepts one connection, serves it, and closes any other
  at once. The peer must be an ordinary user (uid ≥ 1000) or root, and must say `Hello` within
  five seconds. The socket itself is mode 0666 under `/run/listello`, because the peer check
  is what decides who may talk.
- *`Windows` is in the order windows first appeared, not stacking order.* Stacking order
  reshuffles on every focus change, and a Dock whose tiles jump when tapped is unusable. The
  agent also resolves each window's `app_id` to its `.desktop` entry's id where there is one,
  so a pinned `firefox-esr` and a window reporting `firefox` are the same app.
- *The socket is a systemd socket unit* (`listello-core.socket`, enabled on `sockets.target`);
  the service started by udev inherits it. Started by hand, with no unit, the renderer binds
  the path itself before dropping privileges.

**Rejected alternative:** a single service running as the user, with udev rules granting the
user access to the Touch Bar DRM and input devices. Simpler, no IPC — rejected because it
weakens the security posture tiny-dfr deliberately established, and because retrofitting the
split later would cut through the middle of a finished project.

## D12. The project is called `listello`

*Decided 2026-08-03.* `pock-on-linux` was the working directory name and is
retired. A **listello** is a thin decorative strip — in carpentry a batten, in tiling the
narrow border course — which is literally the shape of a Touch Bar. It is a real Italian word
for the actual object rather than a metaphor, it is free on crates.io, and it collides with no
existing project. Naming it after Pock was rejected: this is not a port of Pock (D1), and
borrowing the name invites confusion with a separate, still-maintained project.

**Every name below is fixed. Do not improvise variants.**

| Thing | Name |
|---|---|
| Repository, workspace, config directory | `listello` |
| Renderer binary and crate (D11, privileged) | `listello-core` |
| Session agent binary and crate (D11, user) | `listello-agent` |
| Shared message/wire-format crate | `listello-protocol` |
| `Surface` abstraction crate (D10) | `listello-surface` |
| System unit | `listello-core.service` |
| User unit (`systemd --user`) | `listello-agent.service` |
| Packaged default config | `/usr/share/listello/config.toml` |
| User config override | `/etc/listello/config.toml` |
| Per-user layout (D8 amendment) | `~/.config/listello/config.toml` |
| Settings app binary and crate (D8 amendment) | `listello-settings` |
| Settings app desktop entry | `/usr/share/applications/org.listello.Settings.desktop` |
| IPC socket | `/run/listello/listello.sock` |
| Agent desktop entry (D11 amendment — load-bearing, not metadata) | `/usr/share/applications/org.listello.Agent.desktop` |

Note there is no `listellod`. The renderer is `listello-core`, because the `-core` / `-agent`
pair states the D11 split in the names themselves, and `listellod` is unpronounceable.

Pronunciation, for the README: *lee-STEL-loh*.
