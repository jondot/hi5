//! The confirmation behind "Approve all".
//!
//! Approving is the one irreversible thing hi5 does, and this is the one
//! place it happens more than once per click. So the click on the header
//! does nothing but ask — and the asking shows the exact list that will
//! be approved, by number and title, with the sentence that matters
//! above it. The dialog's OK is what starts the batch (`Hi5::start_batch`);
//! Cancel and Escape do nothing at all.
//!
//! gpui-component's `Dialog`, opened through the window's `Root`, sized
//! for a 392pt panel rather than the 480pt document default.

use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::dialog::DialogButtonProps;
use gpui_component::label::Label;
use gpui_component::{h_flex, v_flex, ActiveTheme as _, WindowExt as _};
use hi5_core::github::PullRequest;

use crate::app::Hi5;
use crate::ui;
use crate::ui::format::ellipsize;

/// The dialog's width. The panel is 392pt; this leaves a margin either
/// side and keeps the overlay visibly an overlay.
const WIDTH: f32 = 352.;
/// The list scrolls past this many points rather than pushing the
/// buttons off the panel — a ten-PR repository must still show its OK.
const LIST_MAX_HEIGHT: f32 = 232.;
/// A title longer than this is cut, by characters (see `format::ellipsize`).
const TITLE_MAX_CHARS: usize = 38;

/// Open the confirmation for `repo`'s `prs`. `app` is who to tell on OK.
pub fn open(
    repo: String,
    prs: Vec<PullRequest>,
    app: WeakEntity<Hi5>,
    window: &mut Window,
    cx: &mut App,
) {
    let repo: SharedString = repo.into();
    let prs = Rc::new(prs);
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let repo = repo.clone();
        let prs = prs.clone();
        let app = app.clone();
        let count = prs.len();
        dialog
            .w(px(WIDTH))
            .margin_top(px(36.))
            // What `.confirm()` sets up, with the two buttons measured:
            // `dialog.footer[0]` is Cancel, `dialog.footer[1]` is OK.
            .footer(|ok, cancel, window, cx| {
                vec![h_flex()
                    .gap_2()
                    .on_children_prepainted(ui::probe::children("dialog.footer"))
                    .child(cancel(window, cx))
                    .child(ok(window, cx))]
            })
            .overlay_closable(false)
            .close_button(false)
            .keyboard(true)
            .button_props(
                DialogButtonProps::default()
                    .ok_text(format!("Approve {count}"))
                    .cancel_text("Cancel"),
            )
            .title(
                div()
                    .text_sm()
                    .child(format!("Approve all in {}", ellipsize(&repo, 30))),
            )
            .child(body(&prs, _cx))
            // `on_ok` returns whether to close; the batch is started
            // and the dialog goes. Nothing happens on cancel.
            .on_ok(move |_, _, cx| {
                let prs = prs.clone();
                let repo = repo.to_string();
                app.update(cx, |this, cx| this.start_batch(repo, &prs, cx))
                    .ok();
                true
            })
    });
}

fn body(prs: &[PullRequest], cx: &App) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    v_flex()
        .gap_3()
        .text_sm()
        .child(
            Label::new(
                "Before approving, confirm you have reviewed all of the pull requests below.",
            )
            .text_sm(),
        )
        .child(
            v_flex()
                .id("approve-all-list")
                .max_h(px(LIST_MAX_HEIGHT))
                .overflow_y_scroll()
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().secondary)
                .child(
                    v_flex()
                        .w_full()
                        // `dialog.pr[i]` is the i-th listed pull request.
                        .on_children_prepainted(ui::probe::children("dialog.pr"))
                        .children(prs.iter().enumerate().map(|(i, pr)| {
                            h_flex()
                                .w_full()
                                .px_2p5()
                                .py_1p5()
                                .gap_2()
                                .items_baseline()
                                .when_some((i > 0).then_some(()), |this, _| {
                                    this.border_t_1().border_color(cx.theme().border)
                                })
                                .child(
                                    Label::new(format!("#{}", pr.number))
                                        .text_xs()
                                        .text_color(muted)
                                        .flex_shrink_0(),
                                )
                                .child(
                                    Label::new(ellipsize(&pr.title, TITLE_MAX_CHARS))
                                        .text_xs()
                                        .flex_1(),
                                )
                        })),
                ),
        )
        .child(
            Label::new(
                "Each approval is posted to GitHub as a public review and cannot be undone here.",
            )
            .text_xs()
            .text_color(muted),
        )
}
