//! The 🖐 in the menu bar, via `tray-icon`.
//!
//! This is the same crate Tauri wraps for its own tray support, used
//! directly. Its click event already carries `rect` — the icon's screen
//! rect in global, top-left coordinates — which is precisely what the
//! panel anchors to, so nothing here has to reach into `NSStatusItem`.

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use hi5_core::geometry::Rect2;
use tray_icon::menu::{Menu, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::decisions::Badge;

/// One press of the menu-bar icon.
///
/// Both edges are reported, because the panel and the highlight are
/// driven by different ones. The panel opens on `Down` — that is when a
/// menu-bar item opens on macOS, and opening on `Up` is what made the
/// icon light up, go out, and come back. The highlight is re-asserted on
/// `Up`, because `tray-icon` clears it there in its own handler
/// (tray-icon-0.24.2 `src/platform_impl/macos/mod.rs:355`) after ours
/// has already run.
pub enum Press {
    Down(Rect2),
    Up,
}

/// Drawn as a text title rather than a template image: the hand keeps
/// its colour that way, where a template image is flattened to a
/// monochrome mask.
const GLYPH: &str = "🖐";
/// The hand when there is nothing left to review.
const DONE: &str = "🤘";

/// A live tray icon. Dropping it removes the icon from the menu bar, so
/// the caller must hold it for the life of the app.
pub struct Tray {
    /// `None` is the null tray: same interface, no menu-bar item. Used
    /// by headless tests, which must not put anything in your menu bar
    /// (and could not, without a run loop).
    icon: Option<TrayIcon>,
}

impl Tray {
    /// Build the icon and return it alongside a stream of left-click
    /// rects.
    ///
    /// A channel rather than a direct callback because `tray-icon`'s
    /// handler is a global, `'static` hook that fires outside any GPUI
    /// context; the receiving end is awaited inside a GPUI task, which
    /// is where an `App` is reachable.
    pub fn new() -> anyhow::Result<(Self, UnboundedReceiver<Press>)> {
        let (tx, rx) = unbounded();
        set_click_handler(tx);
        // A right-click menu with the one thing that must be reachable
        // from the menu bar itself: Quit. Left click still toggles the
        // panel (`with_menu_on_left_click(false)`), and hi5's own ⋯ menu
        // inside it keeps the rest. Without this, an app stuck on the
        // connect screen — no toolbar, no ⋯ — could not be quit at all.
        // The predefined item is `NSApp terminate:`, the same call
        // gpui's own `quit` makes.
        let menu = Menu::new();
        menu.append(&PredefinedMenuItem::quit(Some("Quit hi5")))?;
        let icon = TrayIconBuilder::new()
            .with_title(GLYPH)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()?;
        Ok((Self { icon: Some(icon) }, rx))
    }

    /// A tray that does nothing. See `icon`.
    pub fn null() -> Self {
        Self { icon: None }
    }

    /// Keep the menu-bar item lit for as long as the panel is open.
    ///
    /// This is what every native menu-bar app does and hi5 did not:
    /// `tray-icon` highlights the button on `mouseDown:` and clears it
    /// again on `mouseUp:` (tray-icon-0.24.2
    /// `src/platform_impl/macos/mod.rs:507` and `:355`), so the icon lit
    /// up for the length of the click and went out the instant the panel
    /// appeared. The panel now opens on the *down* edge and this is
    /// re-asserted on the up edge, which is the last thing to run.
    pub fn set_active(&self, active: bool) {
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return;
        };
        let Some(item) = self.icon.as_ref().and_then(|i| i.ns_status_item()) else {
            return;
        };
        if let Some(button) = item.button(mtm) {
            button.highlight(active);
        }
    }

    /// The menu-bar label. See `decisions::Badge` for the three states
    /// and why each looks the way it does.
    pub fn set_badge(&self, badge: Badge) {
        let title = match badge {
            Badge::Broken => format!("{GLYPH} !"),
            Badge::Quiet => GLYPH.to_string(),
            Badge::Count(0) => DONE.to_string(),
            Badge::Count(n) => format!("{GLYPH} {n}"),
        };
        if let Some(icon) = &self.icon {
            icon.set_title(Some(title));
        }
    }
}

fn set_click_handler(tx: UnboundedSender<Press>) {
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        let TrayIconEvent::Click {
            button: tray_icon::MouseButton::Left,
            button_state,
            rect,
            ..
        } = event
        else {
            return;
        };
        let press = match button_state {
            tray_icon::MouseButtonState::Down => Press::Down(Rect2 {
                x: rect.position.x,
                y: rect.position.y,
                w: rect.size.width as f64,
                h: rect.size.height as f64,
            }),
            tray_icon::MouseButtonState::Up => Press::Up,
        };
        let _ = tx.unbounded_send(press);
    }));
}
