//! The macOS half of hi5.
//!
//! Almost all of it is maintained crates rather than hand-written
//! AppKit: `tray-icon` for the status item, `global-hotkey` for the
//! summon, `notify-rust` for banners, `display-info` for monitor
//! geometry. What remains in `panel` is the small set of things GPUI
//! 0.2.2 provably cannot do — see that module's header for the source
//! lines.

pub mod autostart;
pub mod dismiss;
pub mod hotkey;
pub mod notify;
pub mod panel;
pub mod tray;
