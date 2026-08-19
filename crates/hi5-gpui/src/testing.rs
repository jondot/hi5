//! The headless harness: the real app, laid out by gpui, driven by
//! synthetic events, with nothing on screen and nothing of yours touched.
//!
//! This is what a UI test in this crate is built on. `TestAppContext`
//! runs gpui's own layout engine over the same `ui::*` functions the app
//! renders, against a stub text system with fixed glyph metrics — so
//! bounds are exact and deterministic, clicks land where the layout says
//! a control is, and keystrokes go through the same key dispatch the
//! window uses. What it does *not* do is rasterise: real fonts, real
//! wrapping and real colour are what `cargo run --bin preview` is for.
//!
//! The backend is `Backend::null`, which records commands and performs
//! none of them; the tray is `Tray::null`, which puts nothing in the
//! menu bar. A test can therefore click Approve as hard as it likes.
//!
//! ```ignore
//! #[gpui::test]
//! fn back_returns_to_the_inbox(cx: &mut TestAppContext) {
//!     let mut h = Harness::with_queue(cx);
//!     h.click("inbox.row", 0);
//!     assert!(matches!(h.screen(), Screen::Detail(_)));
//!     h.click("header", 0);
//!     assert!(matches!(h.screen(), Screen::Inbox));
//! }
//! ```

use gpui::*;
use gpui_component::Root;
use hi5_core::github::PullRequest;
use hi5_core::poller::{InboxUpdate, PollEvent};

use crate::app::{Hi5, Screen};
use crate::backend::{Backend, Command, CommandResult, Msg};
use crate::platform::panel::{PANEL_HEIGHT, PANEL_WIDTH};
use crate::platform::tray::Tray;
use crate::ui::probe;
use crate::{actions, fixtures, theme};

pub struct Harness {
    pub cx: VisualTestContext,
    pub app: Entity<Hi5>,
    pub backend: Backend,
    _dir: tempfile::TempDir,
}

impl Harness {
    /// The app with an empty queue, on the inbox, focused, drawn once.
    pub fn new(cx: &mut TestAppContext) -> Self {
        let dir = tempfile::tempdir().expect("a scratch config dir");
        let (backend, _rx) = Backend::null(dir.path().to_path_buf());

        cx.update(|cx| {
            gpui_component::init(cx);
            theme::install(cx);
            actions::bind(cx);
        });

        let b = backend.clone();
        let window = cx
            .update(|cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: point(px(0.), px(0.)),
                            size: size(px(PANEL_WIDTH as f32), px(PANEL_HEIGHT as f32)),
                        })),
                        ..Default::default()
                    },
                    |window, cx| {
                        let view = cx.new(|cx| Hi5::new(b, Tray::null(), window, cx));
                        cx.new(|cx| Root::new(view, window, cx))
                    },
                )
            })
            .expect("open the test window");

        let app = window
            .read_with(cx, |root, _| root.view().clone())
            .expect("root view")
            .downcast::<Hi5>()
            .unwrap_or_else(|_| unreachable!("the root view is the Hi5 just built"));

        let mut cx = VisualTestContext::from_window(*window, cx);
        // What `main.rs` does when the panel is shown: activate, and
        // give the panel's own handle the focus.
        cx.update(|window, cx| {
            window.activate_window();
            let focus = app.read(cx).focus.clone();
            window.focus(&focus);
            app.update(cx, |this, cx| this.prime(cx));
        });
        cx.run_until_parked();

        let mut h = Self {
            cx,
            app,
            backend,
            _dir: dir,
        };
        h.draw();
        h
    }

    /// The app showing the fixture queue — the same six pull requests
    /// the preview binary photographs.
    pub fn with_queue(cx: &mut TestAppContext) -> Self {
        let mut h = Self::new(cx);
        h.receive(fixtures::pull_requests());
        h
    }

    /// The app showing `fixtures::long_queue` — sixteen rows in three
    /// sections, so the list scrolls.
    pub fn with_long_queue(cx: &mut TestAppContext) -> Self {
        let mut h = Self::new(cx);
        h.receive(fixtures::long_queue());
        h
    }

    /// Feed the app an inbox update, as a poll cycle would.
    pub fn receive(&mut self, prs: Vec<PullRequest>) {
        self.app.update(&mut self.cx, |this, cx| {
            this.handle(
                Msg::Poll(PollEvent::InboxUpdated(InboxUpdate { prs, anomaly: None })),
                cx,
            );
        });
        self.draw();
    }

    /// Hand the app a backend result, as the null backend never does.
    pub fn result(&mut self, result: CommandResult) {
        self.app.update(&mut self.cx, |this, cx| {
            this.handle(Msg::Command(result), cx);
        });
        self.draw();
    }

    /// Render one frame and collect its probes.
    pub fn draw(&mut self) {
        probe::reset();
        self.cx.update(|window, _| window.refresh());
        self.cx.run_until_parked();
    }

    /// The bounds recorded under `name` in the last drawn frame.
    pub fn bounds(&mut self, name: &str) -> Vec<Bounds<Pixels>> {
        self.draw();
        probe::get(name)
    }

    /// Click the centre of the `index`th probe named `name`.
    pub fn click(&mut self, name: &str, index: usize) {
        let all = self.bounds(name);
        let b = *all.get(index).unwrap_or_else(|| {
            panic!(
                "no probe {name}[{index}] in the last frame; have {} of {name}: {:?}",
                all.len(),
                probe::all().iter().map(|p| p.name).collect::<Vec<_>>()
            )
        });
        self.cx.simulate_click(b.center(), Modifiers::default());
        self.cx.run_until_parked();
        self.draw();
    }

    /// Turn the wheel over the centre of the `index`th probe named
    /// `name`: `by` points downward, as a finger drags the page up.
    pub fn scroll(&mut self, name: &str, index: usize, by: Pixels) {
        let all = self.bounds(name);
        let b = *all
            .get(index)
            .unwrap_or_else(|| panic!("no probe {name}[{index}] to scroll over"));
        self.cx.simulate_event(ScrollWheelEvent {
            position: b.center(),
            delta: ScrollDelta::Pixels(point(px(0.), -by)),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        });
        self.cx.run_until_parked();
        self.draw();
    }

    /// Type keystrokes, in gpui's notation: `"escape"`, `"cmd-enter"`,
    /// `"down down enter"`.
    pub fn keys(&mut self, keystrokes: &str) {
        self.cx.simulate_keystrokes(keystrokes);
        self.cx.run_until_parked();
        self.draw();
    }

    pub fn screen(&mut self) -> Screen {
        self.app.read_with(&self.cx, |this, _| this.screen.clone())
    }

    /// Read a field off the app.
    pub fn read<T>(&mut self, f: impl FnOnce(&Hi5) -> T) -> T {
        self.app.read_with(&self.cx, |this, _| f(this))
    }

    /// What the UI has asked the (null) backend to do so far.
    pub fn commands(&self) -> Vec<Command> {
        self.backend.commands()
    }
}
