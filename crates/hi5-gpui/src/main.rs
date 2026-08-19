//! The menu-bar app.

use gpui::*;
use gpui_component::Root;
use objc2::MainThreadMarker;

use hi5_gpui::app::Hi5;
use hi5_gpui::platform::panel::{Panel, PANEL_HEIGHT, PANEL_WIDTH};
use hi5_gpui::{actions, assets, backend, config_dir, platform, theme};

fn main() {
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);
            let mtm = MainThreadMarker::new().expect("gpui runs this on the main thread");
            // After GPUI's own applicationDidFinishLaunching, which sets
            // the policy back to Regular — see platform::panel.
            platform::panel::become_accessory(mtm);
            theme::install(cx);
            actions::bind(cx);

            let (backend, mut messages) =
                backend::Backend::start(config_dir()).expect("start the backend");
            let (tray, mut clicks) = platform::tray::Tray::new().expect("create the tray icon");
            backend.check_auth();

            let window = cx
                .open_window(panel_window_options(), |window, cx| {
                    let view = cx.new(|cx| {
                        let mut this = Hi5::new(backend.clone(), tray, window, cx);
                        // The cached queue, pushed into the list before
                        // the first frame — filling the delegates needs a
                        // `&mut App`, which `new` does not have while it
                        // is still building the value it returns.
                        this.prime(cx);
                        this
                    });
                    cx.new(|cx| Root::new(view, window, cx))
                })
                .expect("open the panel window");

            let panel = window
                .update(cx, |_, window, _| Panel::adopt(window))
                .ok()
                .flatten()
                .expect("a gpui window is backed by an NSWindow on macOS");

            // Everything the backend has to say, folded into the view.
            let root = window
                .read_with(cx, |root, _| root.view().clone())
                .expect("root view")
                .downcast::<Hi5>()
                .unwrap_or_else(|_| unreachable!("the root view is the one just built above"));
            // Hide the moment the panel stops being active.
            let _ = window.update(cx, |view, window, cx| {
                view.view()
                    .clone()
                    .downcast::<Hi5>()
                    .map(|v| v.update(cx, |this, cx| this.hide_on_blur(panel, window, cx)))
                    .ok();
            });

            let msg_root = root.clone();
            cx.spawn(async move |cx| {
                use futures::StreamExt;
                while let Some(msg) = messages.next().await {
                    let _ = msg_root.update(cx, |this, cx| this.handle(msg, cx));
                }
            })
            .detach();

            if let Ok((hotkey, mut presses)) = platform::hotkey::register() {
                let hotkey_root = root.clone();
                cx.spawn(async move |cx| {
                    use futures::StreamExt;
                    while presses.next().await.is_some() {
                        let mtm = MainThreadMarker::new().expect("gpui task on the main thread");
                        if panel.is_visible(mtm) {
                            panel.hide(mtm);
                            let _ = hotkey_root.update(cx, |this, _| this.tray.set_active(false));
                            continue;
                        }
                        panel.show(mtm);
                        let _ = cx.update(|cx| cx.activate(true));
                        let _ = window.update(cx, |_, w, cx| {
                            w.focus(&hotkey_root.read(cx).focus);
                        });
                        let _ = hotkey_root.update(cx, |this, _| this.tray.set_active(true));
                    }
                })
                .detach();
                std::mem::forget(hotkey);
            } else {
                // Never fatal: another app may already own ⌥⌘A, and the
                // menu-bar icon still works.
                eprintln!("hi5: could not register ⌥⌘A");
            }

            // Clicking a *different* menu-bar item never makes hi5
            // resign active, so the blur handler alone left the panel
            // sitting next to the menu that had just opened over it.
            //
            // The monitor cannot tell hi5's own status item apart from
            // anyone else's: at the moment its icon is pressed hi5 is not
            // yet the active app, so that click is "outside" too. Left
            // alone, the mouse-*up* of the very click that opened the
            // panel closed it again, and the icon stopped working. The
            // icon's rect arrives with every press; clicks inside it
            // belong to hi5.
            let icon_rect: std::rc::Rc<std::cell::Cell<Option<hi5_core::geometry::Rect2>>> =
                std::rc::Rc::new(std::cell::Cell::new(None));
            let (monitor, mut outside) = platform::dismiss::watch();
            std::mem::forget(monitor);
            let dismiss_root = root.clone();
            let dismiss_icon = icon_rect.clone();
            cx.spawn(async move |cx| {
                use futures::StreamExt;
                while let Some(at) = outside.next().await {
                    let mtm = MainThreadMarker::new().expect("gpui task on the main thread");
                    if !panel.is_visible(mtm) {
                        continue;
                    }
                    if dismiss_icon.get().is_some_and(|r| {
                        at.x >= r.x && at.x <= r.x + r.w && at.y >= r.y && at.y <= r.y + r.h
                    }) {
                        continue;
                    }
                    panel.hide(mtm);
                    let _ = dismiss_root.update(cx, |this, _| this.tray.set_active(false));
                }
            })
            .detach();

            let focus_root = root.clone();
            cx.spawn(async move |cx| {
                use futures::StreamExt;
                while let Some(press) = clicks.next().await {
                    let mtm = MainThreadMarker::new().expect("gpui task on the main thread");
                    let icon = match press {
                        // The up edge only re-asserts the highlight:
                        // `tray-icon` clears it in its own `mouseUp:`,
                        // which runs after this loop sees the down edge.
                        platform::tray::Press::Up => {
                            let visible = panel.is_visible(mtm);
                            let _ = focus_root.update(cx, |this, _| this.tray.set_active(visible));
                            continue;
                        }
                        platform::tray::Press::Down(icon) => {
                            icon_rect.set(Some(icon));
                            icon
                        }
                    };
                    if panel.is_visible(mtm) {
                        panel.hide(mtm);
                        let _ = focus_root.update(cx, |this, _| this.tray.set_active(false));
                        continue;
                    }
                    panel.show_under(icon, mtm);
                    // `WindowKind::PopUp` is an NSPanel with
                    // NSWindowStyleMaskNonactivatingPanel, which is what
                    // stops a menu-bar panel stealing the foreground when
                    // you merely click it — and also what stops it ever
                    // becoming key on its own, so ↑/↓/↵ reached nothing.
                    // Activating the app first is the missing half; the
                    // panel then hides itself again on blur.
                    let _ = cx.update(|cx| cx.activate(true));
                    let _ = window.update(cx, |_, w, cx| {
                        w.focus(&focus_root.read(cx).focus);
                    });
                    let _ = focus_root.update(cx, |this, _| this.tray.set_active(true));
                }
            })
            .detach();
        });
}

fn panel_window_options() -> WindowOptions {
    WindowOptions {
        show: false,
        focus: false,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        kind: WindowKind::PopUp,
        titlebar: None,
        window_background: WindowBackgroundAppearance::Transparent,
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(PANEL_WIDTH as f32), px(PANEL_HEIGHT as f32)),
        })),
        ..Default::default()
    }
}
