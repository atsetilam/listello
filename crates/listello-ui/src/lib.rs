//! Everything above the pixel line: widgets, layout, and the shell that drives them.
//!
//! D3 says widgets are **pure functions** — an immutable state snapshot and a rectangle in,
//! draw commands and hit regions out. This crate is where that rule is enforced structurally
//! rather than by discipline: it depends on [`listello_surface`] and [`listello_protocol`] and
//! on nothing that can reach the outside world, so a widget *cannot* call D-Bus, touch DRM,
//! read the clock or hold a handle to the core. The borrow checker is not the guard here; the
//! dependency graph is.
//!
//! ```text
//!   config ──► Widget ──DrawCmd + HitRegion──► layout ──Frame──► Surface
//!                 ▲                              │
//!            UiState snapshot ◄──── Shell ◄───────┘ Action
//! ```
//!
//! # Why this is a separate crate from `listello-core`
//!
//! `listello-core` links libinput, uinput, udev and DRM, so it does not compile on macOS and
//! never will. M1's exit criterion is *one command on a Mac* that shows a Touch Bar.
//! Everything that has to be seen therefore lives here, and both binaries —
//! `listello-core` on the panel, `listello-sim` on a desktop — stay thin wrappers that own
//! their platform's file descriptors and their `Surface`, and drive this crate identically.
//! See DESIGN.md's D10 addendum.
//!
//! # Rules
//!
//! - **No Linux-only dependency, ever.** See this crate's `Cargo.toml`.
//! - **No I/O, no clock, no environment.** A widget that calls `Local::now()` is both a D3
//!   violation and an untestable widget; the time arrives in [`UiState`].
//! - **Never clone a `cairo::FontFace`** — it aborts the process in this feature set
//!   (RESEARCH §2.2). Fonts reach this crate as `Rc<FontFace>` inside a
//!   [`listello_surface::TextStyle`], resolved by whichever binary is driving.

pub mod config;
pub mod dock;
pub mod hit;
pub mod layout;
pub mod shell;
pub mod state;
pub mod widget;
pub mod widgets;

pub use config::{
    ConfigError, GeneralConfig, Language, PreviewStyle, StripConfig, UiConfig, WidgetConfig,
    WindowPreviews,
};
pub use dock::DockWidget;
pub use hit::{Action, HitRegion, KeyName, RegionId};
pub use layout::{
    render_scroll, render_strip, scroll_offset_clamped, solve_widths, strip_layout, strip_rects,
};
pub use shell::Shell;
pub use state::{IconKey, Icons, Theme, UiState, WindowControls};
pub use widget::{Ctx, Output, Sizing, Widget};
pub use widgets::{
    BatteryWidget, ButtonWidget, ClockWidget, ControlCenterWidget, FolderEntry, FoldersWidget,
    NetworkWidget, SeparatorWidget,
};
