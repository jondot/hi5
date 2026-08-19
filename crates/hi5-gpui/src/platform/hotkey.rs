//! ⌥⌘A, via `global-hotkey`.
//!
//! Same shape as the tray: the crate's handler is a global `'static`
//! hook, so it forwards through a channel into a GPUI task rather than
//! trying to reach an `App` from outside one.

use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

/// A live registration. Dropping the manager unregisters the hotkey, so
/// the caller must hold it for the life of the app.
pub struct Hotkey {
    _manager: GlobalHotKeyManager,
}

/// Registers ⌥⌘A and returns a stream that ticks once per press.
///
/// Failure is not fatal and never should be: another app may already own
/// the combination, and hi5 is still perfectly usable from the menu bar.
pub fn register() -> anyhow::Result<(Hotkey, UnboundedReceiver<()>)> {
    let manager = GlobalHotKeyManager::new()?;
    manager.register(HotKey::new(
        Some(Modifiers::ALT | Modifiers::META),
        Code::KeyA,
    ))?;

    let (tx, rx) = unbounded();
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        // Press only. Without this the panel toggles twice per keypress
        // and appears not to open at all.
        if event.state == HotKeyState::Pressed {
            let _ = tx.unbounded_send(());
        }
    }));

    Ok((Hotkey { _manager: manager }, rx))
}
