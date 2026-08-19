//! Hiding the panel when the click that should dismiss it lands in
//! another process.
//!
//! The window's own activation handler covers most of this: click
//! another *window* and hi5 resigns active and hides. It does not cover
//! the case that matters most in the menu bar — clicking a different
//! status item. `NSMenu` tracking runs in the other app's process
//! without ever making hi5 resign active, so the panel simply stayed up
//! next to the menu that had just opened over it.
//!
//! A global mouse monitor is the standard answer and, unlike a keyboard
//! monitor, needs no accessibility permission. It sees clicks that
//! landed outside this process — which unavoidably includes the click on
//! hi5's *own* menu-bar icon, because at the moment that icon is pressed
//! hi5 is not yet the active app. Left alone, the mouse-up of the very
//! click that opened the panel closed it again.
//!
//! So the location travels with the event and the caller ignores clicks
//! inside the icon's own rect. An earlier version compared timestamps
//! instead — ignore anything within 400ms of a tray press — and that
//! worked on an idle app and failed while it was parsing a poll, because
//! the monitor's events arrive on the same starved main thread. A rect
//! test does not care how late it runs.

use block2::RcBlock;
use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask, NSScreen};

/// A live monitor. Dropping it removes the hook, so the caller has to
/// hold it for as long as the panel can be open — the life of the app.
pub struct OutsideClicks(#[allow(dead_code)] Option<Retained<AnyObject>>);

/// Where a click landed, in global top-left points — the same space
/// `tray-icon` reports the status item's rect in.
#[derive(Debug, Clone, Copy)]
pub struct ClickAt {
    pub x: f64,
    pub y: f64,
}

/// Start watching, and return a stream of clicks landing outside hi5.
///
/// A channel rather than a direct callback for the same reason
/// `tray::Tray` uses one: the handler is a `'static` AppKit block that
/// fires outside any GPUI context, and the receiving end is awaited
/// inside a GPUI task, which is where an `App` is reachable.
pub fn watch() -> (OutsideClicks, UnboundedReceiver<ClickAt>) {
    let (tx, rx) = unbounded();
    let block = RcBlock::new(move |_event: std::ptr::NonNull<NSEvent>| {
        let point = NSEvent::mouseLocation();
        // AppKit reports screen coordinates bottom-left-origin against
        // the *primary* display; everything else here is top-left.
        let flip = primary_height();
        let _ = tx.unbounded_send(ClickAt {
            x: point.x,
            y: flip - point.y,
        });
    });
    let token = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown,
        &block,
    );
    (OutsideClicks(token), rx)
}

fn primary_height() -> f64 {
    objc2::MainThreadMarker::new()
        .and_then(|mtm| {
            NSScreen::screens(mtm)
                .iter()
                .next()
                .map(|s| s.frame().size.height)
        })
        .unwrap_or(0.)
}
