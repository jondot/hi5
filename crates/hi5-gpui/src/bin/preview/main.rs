//! Render every screen from fixtures and photograph it. No clicking.
//!
//! This replaces a harness that drove the real app through synthetic
//! mouse events in the menu bar. That approach took four minutes a
//! sweep, needed a live GitHub poll before it could see anything, could
//! not be trusted to land a click on the control it aimed at — it twice
//! changed real settings — and spent most of its failures inside the
//! harness rather than the app.
//!
//! This takes seconds, needs no network, touches no configuration, and
//! renders exactly the same `ui::*` functions the app does. What it
//! deliberately does *not* cover is interaction — that is what the
//! headless tests in `src/tests/` are for, on the same fixtures. Layout
//! and typography are worth *looking at* on every change, with real
//! fonts and real pixels, which is what this is for.
//!
//!     cargo run --release --bin preview -- target/preview
//!     cargo run --release --bin preview -- --readme docs/screenshots
//!
//! The second form shoots the README's handful of screens from
//! `fixtures::showcase` — invented data that looks like a working day
//! rather than like a layout test. Both forms photograph on the
//! sharpest attached display, so a Retina screen gives 2× pixels.
//!
//! One PNG per screen per appearance, and beside each a `.json` of every
//! `ui::probe` measurement in that frame — the layout engine's own
//! numbers for the frame in the picture, so "is that rule really edge to
//! edge" is a lookup rather than a squint. The window is shown at a
//! fixed origin for about half a second per screen and never takes
//! keyboard focus.

use std::path::PathBuf;

use display_info::DisplayInfo;
use gpui::*;
use gpui_component::Root;

use hi5_gpui::app::{Hi5, Screen, SettingsTab};
use hi5_gpui::{actions, assets, backend, fixtures, platform, theme};

/// Where the preview window sits while it is being photographed,
/// relative to the top-left of the display it is shown on.
const ORIGIN: Point<Pixels> = point(px(80.), px(80.));

type Pose = fn(&mut Hi5, &mut Window, &mut Context<Hi5>);
type Queue = fn() -> Vec<hi5_core::github::PullRequest>;

/// Which screens get shot, and how to put the view into each one.
const SHOTS: &[(&str, Pose)] = &[
    ("inbox", |_, _, _| {}),
    ("inbox-selected", |this, window, cx| {
        this.preview_select(3, window, cx)
    }),
    // A queue long enough to scroll, scrolled into its second section:
    // the pinned header is that section's, pushed by nothing yet, and
    // the first section's rows have gone under it.
    ("inbox-scrolled", |this, _, cx| {
        this.prs = fixtures::long_queue();
        this.invalidate(cx);
        this.preview_scroll(px(200.), cx);
    }),
    ("inbox-empty", |this, _, cx| {
        this.prs.clear();
        this.invalidate(cx);
    }),
    // Nothing fetched yet: the spinner, not the empty state.
    ("inbox-loading", |this, _, cx| {
        this.prs.clear();
        this.last_updated = None;
        this.refreshing = true;
        this.invalidate(cx);
    }),
    // The repo focus popover, open, with one repo focused.
    ("inbox-filter", |this, _, cx| {
        this.focus_repos = vec!["acme-labs/atlas".into()];
        this.preview_filter_open = true;
        this.menu_generation += 1;
        this.invalidate(cx);
    }),
    // "Approve all" on the first section: the confirmation, listing it.
    ("inbox-approve-all", |this, window, cx| {
        this.preview_approve_all(window, cx);
    }),
    ("detail", |this, _, cx| {
        // Through `go`, not by assigning the screen: `go` is what starts
        // the 250ms arming timer, and without it Approve sits on its
        // spinner for ever — a screenshot of a state the app never
        // actually shows.
        if let Some(pr) = this.prs.iter().find(|p| p.asked_for_you).cloned() {
            this.go(Screen::Detail(Box::new(pr)), cx);
        }
    }),
    ("settings", |this, _, cx| {
        this.set_settings_tab(SettingsTab::General, cx);
        this.screen = Screen::Settings;
    }),
    ("settings-repos", |this, _, cx| {
        let (candidates, watched) = fixtures::orgs();
        this.org_candidates = candidates;
        this.settings.watched_orgs = watched;
        this.settings.repos.muted = ["acme-labs/atlas".to_string()].into();
        this.set_settings_tab(SettingsTab::Repositories, cx);
        this.screen = Screen::Settings;
    }),
    ("auth", |this, _, _| {
        this.auth = Some(hi5_core::auth::AuthState::GhNotAuthenticated);
    }),
    // gh not found: where hi5 looked, and the way to Settings.
    ("auth-no-gh", |this, _, _| {
        this.auth = Some(hi5_core::auth::AuthState::GhNotInstalled {
            homebrew_available: true,
        });
        this.gh_resolution = Some(hi5_core::auth::runner::Resolution {
            path: "gh".into(),
            runnable: false,
            overridden: false,
        });
    }),
    ("auth-signed-out", |this, _, _| {
        this.auth = Some(hi5_core::auth::AuthState::SignedOut);
    }),
];

/// The README's screens: the inbox, one pull request, the Approve-all
/// confirmation and Settings, each in both appearances. Signed in, so
/// Settings reads as a working installation rather than a blank one.
const README_SHOTS: &[(&str, Pose)] = &[
    ("inbox", |_, _, _| {}),
    ("detail", |this, _, cx| {
        if let Some(pr) = this.prs.iter().find(|p| p.asked_for_you).cloned() {
            this.go(Screen::Detail(Box::new(pr)), cx);
        }
    }),
    ("approve-all", |this, window, cx| {
        this.preview_approve_all(window, cx);
    }),
    ("settings", |this, _, cx| {
        this.auth = Some(hi5_core::auth::AuthState::Connected {
            login: "dana".into(),
            source: "gh".into(),
            scopes: vec!["repo".into(), "read:org".into()],
            scopes_adequate: true,
            verified: true,
        });
        this.set_settings_tab(SettingsTab::General, cx);
        this.screen = Screen::Settings;
    }),
];

/// The display to photograph on — the sharpest one attached — and where
/// its top-left sits in the global coordinates `screencapture -R`
/// takes. GPUI's own `Display::bounds()` drops the origin, so this
/// reads it through `display-info` like `platform::panel` does, and
/// matches the two by the display id both expose.
fn stage(cx: &App) -> (Option<DisplayId>, Point<Pixels>) {
    let Some(best) = DisplayInfo::all()
        .unwrap_or_default()
        .into_iter()
        .max_by(|a, b| {
            a.scale_factor
                .total_cmp(&b.scale_factor)
                .then(b.is_primary.cmp(&a.is_primary))
        })
    else {
        return (None, ORIGIN);
    };
    let id = cx
        .displays()
        .into_iter()
        .map(|d| d.id())
        .find(|id| u32::from(*id) == best.id);
    let global = point(px(best.x as f32) + ORIGIN.x, px(best.y as f32) + ORIGIN.y);
    println!(
        "  on {} ({}x{} @{}x)",
        best.friendly_name, best.width, best.height, best.scale_factor
    );
    (id, global)
}

/// A throwaway config, so a preview run can never disturb a real one —
/// and an *empty* one each run, so what the settings screens show comes
/// from the fixtures and the defaults, not from whatever a previous run
/// left behind.
fn config_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("hi5-preview-config");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let readme = args.iter().any(|a| a == "--readme");
    let out = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/preview"));
    let _ = std::fs::create_dir_all(&out);
    let shots: &[(&str, Pose)] = if readme { README_SHOTS } else { SHOTS };
    let queue: Queue = if readme {
        fixtures::showcase
    } else {
        fixtures::pull_requests
    };

    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);
            theme::install(cx);
            actions::bind(cx);

            // Nothing live: no poller, no `gh`, no menu-bar item. The
            // queue comes from the fixtures below.
            let (backend, _messages) = backend::Backend::null(config_dir());
            let tray = platform::tray::Tray::null();

            let (display_id, global_origin) = stage(cx);
            let window = cx
                .open_window(options(display_id), |window, cx| {
                    let view = cx.new(|cx| Hi5::new(backend.clone(), tray, window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                })
                .expect("open the preview window");

            let root = window
                .read_with(cx, |root, _| root.view().clone())
                .expect("root view")
                .downcast::<Hi5>()
                .unwrap_or_else(|_| unreachable!("the root view is the one just built above"));

            cx.spawn(async move |cx| {
                for dark in [false, true] {
                    for (name, pose) in shots {
                        // `update_window`, not `WindowHandle::update`:
                        // the latter leases the `Root` view for the
                        // duration, and a pose that opens a dialog needs
                        // `Root` itself (`window.open_dialog`).
                        let _ = cx.update_window(window.into(), |_, window, cx| {
                            root.update(cx, |this, cx| {
                                this.preview_reset(dark, queue(), window, cx);
                                pose(this, window, cx);
                                cx.notify();
                            })
                        });
                        // Two frames: one to lay the new screen out, one
                        // to be sure it has been presented before the
                        // shutter. A single frame caught the *previous*
                        // screen often enough to be useless.
                        for _ in 0..2 {
                            hi5_gpui::ui::probe::reset();
                            let _ = cx.update(|cx| cx.refresh_windows());
                            // The frame is drawn while this task is
                            // parked here — so the probes read below are
                            // this frame's, and the shutter fires after
                            // it has been presented.
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(180))
                                .await;
                        }
                        let suffix = if dark { "-dark" } else { "" };
                        capture(global_origin, &out.join(format!("{name}{suffix}.png")));
                        dump_probes(&out.join(format!("{name}{suffix}.json")));
                    }
                }
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        });
}

/// Photograph the window's own rectangle.
///
/// By position rather than by window id: the position is chosen here, so
/// it needs no lookup, and `screencapture -R` is the same tool the old
/// harness used — the part of it that always worked. On a Retina
/// display the PNG comes back at 2× the point size.
fn capture(origin: Point<Pixels>, path: &std::path::Path) {
    let w = platform::panel::PANEL_WIDTH;
    let h = platform::panel::PANEL_HEIGHT;
    let status = std::process::Command::new("screencapture")
        .args([
            "-x",
            "-o",
            "-R",
            &format!(
                "{},{},{},{}",
                f32::from(origin.x),
                f32::from(origin.y),
                w,
                h
            ),
        ])
        .arg(path)
        .status();
    match status {
        Ok(s) if s.success() => println!("  {}", path.display()),
        _ => eprintln!("  FAILED {}", path.display()),
    }
}

/// Every probe in the frame that was just photographed, as JSON:
/// `[{"name":"inbox.row","index":0,"x":0,"y":87,"w":392,"h":56}, …]`.
fn dump_probes(path: &std::path::Path) {
    let rows: Vec<serde_json::Value> = hi5_gpui::ui::probe::all()
        .into_iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "index": p.index,
                "x": f32::from(p.bounds.origin.x),
                "y": f32::from(p.bounds.origin.y),
                "w": f32::from(p.bounds.size.width),
                "h": f32::from(p.bounds.size.height),
            })
        })
        .collect();
    if let Ok(text) = serde_json::to_string_pretty(&rows) {
        let _ = std::fs::write(path, text);
    }
}

fn options(display_id: Option<DisplayId>) -> WindowOptions {
    WindowOptions {
        display_id,
        titlebar: None,
        // Shown, but never key: the preview must not take the keyboard
        // away from whatever you were typing into.
        focus: false,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        kind: WindowKind::PopUp,
        window_background: WindowBackgroundAppearance::Transparent,
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: ORIGIN,
            size: size(
                px(platform::panel::PANEL_WIDTH as f32),
                px(platform::panel::PANEL_HEIGHT as f32),
            ),
        })),
        ..Default::default()
    }
}
