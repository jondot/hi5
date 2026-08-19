//! The two things GPUI cannot do to a window, and nothing else.
//!
//! Everything that *is* available was left to GPUI: `WindowKind::PopUp`
//! gives an `NSPanel` with `NSWindowStyleMaskNonactivatingPanel`
//! (gpui-0.2.2 `src/platform/mac/window.rs:622`), `titlebar: None` gives
//! a chromeless window with no traffic lights (`:616`), and
//! `WindowBackgroundAppearance::Transparent` is a plain `WindowOptions`
//! field. Hand-rolling `setStyleMask`/`setOpaque`/`setLevel` over the top
//! of those would be writing a worse version of what the framework
//! already does.
//!
//! What is genuinely missing, with the evidence:
//!
//! 1. **Moving and hiding one window.** GPUI's `PlatformWindow` trait
//!    (`src/platform.rs:461-489`) has `bounds()`, `activate()`,
//!    `minimize()`, `toggle_fullscreen()` — and no position setter, no
//!    per-window `hide`, no `set_level`. Its `hide()` (`:172`) belongs to
//!    `Platform` and hides the entire application. A window's position
//!    can only be chosen in `WindowOptions` at open time.
//!
//!    The alternative was to close and reopen the window on every
//!    toggle. That is genuinely GPUI-native, but it means fighting the
//!    display-relative origin arithmetic at `window.rs:655` and mapping
//!    an OS display id onto GPUI's opaque `DisplayId` — more code than
//!    this, and more fragile, to end up in the same place.
//!
//! 2. **Not having a Dock icon.** `did_finish_launching`
//!    (`src/platform/mac/platform.rs:1390`) hardcodes
//!    `setActivationPolicy_(NSApplicationActivationPolicyRegular)`. There
//!    is no option for it, and because it runs at launch it also
//!    overrides `LSUIElement` in Info.plist, so there is no way to do
//!    this from outside the process either.

use display_info::DisplayInfo;
use hi5_core::geometry::{compute_position, monitor_containing, Rect2, Size2};
use objc2::runtime::AnyObject;
use objc2::{msg_send, MainThreadMarker};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// The panel's size, in points. Fixed — this is a popover, not a
/// resizable window.
pub const PANEL_WIDTH: f64 = 392.0;
pub const PANEL_HEIGHT: f64 = 544.0;

/// Every attached monitor in *global, top-left* coordinates.
///
/// This is the space `tray-icon` reports its click rect in, and the
/// space `hi5_core::geometry` does its arithmetic in, so nothing here
/// converts anything. `display-info` reads `CGDisplayBounds` and keeps
/// the origin, which is exactly the field GPUI discards.
pub fn monitors() -> Vec<Rect2> {
    DisplayInfo::all()
        .unwrap_or_default()
        .into_iter()
        .map(|d| Rect2 {
            x: d.x as f64,
            y: d.y as f64,
            w: d.width as f64,
            h: d.height as f64,
        })
        .collect()
}

/// Height of the primary display, the pivot for the one coordinate flip
/// this file performs. AppKit measures window frames from the
/// bottom-left of the primary screen; everything else here is top-left.
fn primary_height() -> f64 {
    DisplayInfo::all()
        .unwrap_or_default()
        .into_iter()
        .find(|d| d.is_primary)
        .map(|d| d.height as f64)
        .unwrap_or(0.0)
}

/// A handle to the `NSWindow` GPUI draws into.
///
/// `Copy` and pointer-sized because it is captured by the tray click
/// handler and the hotkey handler, and neither should own the window.
#[derive(Clone, Copy)]
pub struct Panel {
    window: *mut AnyObject,
}

// Every method asserts `MainThreadMarker`, and AppKit windows are
// main-thread-only, so the pointer is never touched off it.
unsafe impl Send for Panel {}
unsafe impl Sync for Panel {}

impl Panel {
    /// Adopt the `NSWindow` behind a GPUI window. Returns `None` if the
    /// handle is not an AppKit window, which cannot happen on macOS.
    pub fn adopt(handle: &impl HasWindowHandle) -> Option<Self> {
        let raw = handle.window_handle().ok()?;
        let RawWindowHandle::AppKit(h) = raw.as_raw() else {
            return None;
        };
        let view: *mut AnyObject = h.ns_view.as_ptr().cast();
        let window: *mut AnyObject = unsafe { msg_send![view, window] };
        (!window.is_null()).then_some(Self { window })
    }

    pub fn is_visible(&self, _mtm: MainThreadMarker) -> bool {
        unsafe { msg_send![self.window, isVisible] }
    }

    pub fn hide(&self, _mtm: MainThreadMarker) {
        unsafe {
            let _: () = msg_send![self.window, orderOut: std::ptr::null::<AnyObject>()];
        }
    }

    /// Show the panel centred under `icon`, clamped onto whichever
    /// monitor that icon is on.
    pub fn show_under(&self, icon: Rect2, mtm: MainThreadMarker) {
        let monitors = monitors();
        let monitor = monitor_containing((icon.x, icon.y), &monitors).or(monitors.first().copied());
        let size = Size2 {
            w: PANEL_WIDTH,
            h: PANEL_HEIGHT,
        };
        let (x, y) = compute_position(icon, size, monitor);
        self.show_at(x, y, mtm);
    }

    /// Show the panel at a top-left position in global coordinates.
    pub fn show_at(&self, x: f64, y: f64, _mtm: MainThreadMarker) {
        // The one flip: `setFrameOrigin:` wants the bottom-left corner in
        // AppKit's bottom-left-origin global space.
        let cocoa_y = primary_height() - y - PANEL_HEIGHT;
        unsafe {
            let _: () = msg_send![self.window, setFrameOrigin: CGPoint { x, y: cocoa_y }];
            let _: () = msg_send![self.window, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
        }
    }

    /// Show the panel wherever it already is — the hotkey has no icon
    /// rect to anchor to.
    pub fn show(&self, _mtm: MainThreadMarker) {
        unsafe {
            let _: () = msg_send![self.window, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
        }
    }
}

/// `NSPoint`/`CGPoint`, declared here rather than pulling in
/// `objc2-foundation` for one struct.
#[repr(C)]
struct CGPoint {
    x: f64,
    y: f64,
}
unsafe impl objc2::encode::Encode for CGPoint {
    const ENCODING: objc2::encode::Encoding =
        objc2::encode::Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}

/// Drop the Dock icon: hi5 lives in the menu bar, and "accessory" is
/// what that means to AppKit.
///
/// Must run *after* GPUI's own `applicationDidFinishLaunching`, which
/// sets the policy back to `Regular` — see this module's header.
pub fn become_accessory(mtm: MainThreadMarker) {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    NSApplication::sharedApplication(mtm)
        .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}
