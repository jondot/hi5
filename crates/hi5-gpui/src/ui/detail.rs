//! One pull request, and the decision about it.
//!
//! Content sits directly on `theme.background` — the same surface the
//! list scrolls on — with one `Divider` between the facts and the body.
//! A single-object detail view is not a settings pane: wrapping it in
//! grouped tables is what made this screen read as grey and inert.
//!
//! Approve is the only irreversible thing hi5 does. Singly, it exists
//! only here, behind an arming delay, never on a list row; the one
//! other way to it is a section header's "Approve all", which goes
//! through a confirmation that lists what it is about to do
//! (`ui::approve_all`).

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::button::{ButtonCustomVariant, ButtonVariants as _};
use gpui_component::divider::Divider;
use gpui_component::label::Label;
use gpui_component::tag::Tag;
use gpui_component::text::{TextView, TextViewStyle};
use gpui_component::{
    avatar::Avatar, h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, IconName,
    Sizable as _, StyledExt as _,
};
use hi5_core::github::{CheckState, PullRequest};

use crate::actions::{Approve, OpenExternal, Skip};
use crate::app::Hi5;
use crate::ui;
use crate::ui::format::{ellipsize, relative_age};
use crate::ui::{facts, Fact};

/// How much of the identity line the repo may take, in characters,
/// leaving room for the login, the number and the age beside it.
///
/// Characters rather than pixels — see `format::ellipsize`: a definite
/// pixel width is the only thing gpui will ellipsize against, and it
/// would reserve the full column for short repo names too, putting a
/// hole in the middle of a line meant to read continuously.
const REPO_MAX_CHARS: usize = 24;

pub fn render(
    this: &mut Hi5,
    pr: &PullRequest,
    window: &mut Window,
    cx: &mut Context<Hi5>,
) -> impl IntoElement {
    let strip = this.strip();
    let last_action = this.last_action.clone();
    let error = this.action_error.clone();
    let armed = this.is_armed();
    let busy = this.busy;

    let body = if pr.body.trim().is_empty() {
        "_No description._".to_string()
    } else {
        pr.body.clone()
    };

    v_flex()
        .size_full()
        .bg(cx.theme().background)
        // Nothing on the trailing edge: Open in the footer (and ⌘O) is
        // the way to GitHub, and a second one in the header was a second
        // one.
        .child(ui::screen_header(format!("#{}", pr.number), None, cx))
        .children(strip.as_ref().map(ui::status_strip))
        .child(
            v_flex()
                .id("detail-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(header(pr, cx))
                // Edge to edge, like every rule in the app: a divider
                // that stopped at the text's margin read as a dash
                // under the facts rather than a boundary in the screen.
                .child(div().py_3().child(Divider::horizontal()))
                .child(
                    // Body copy at the app's body size: `TextView` sizes
                    // its paragraphs from the inherited font size, and
                    // at the 16px rem a PR description came out larger
                    // than the PR's own title.
                    div()
                        .px_4()
                        .pb_4()
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child(
                            // PR bodies are untrusted Markdown and
                            // routinely link out. `TextView` renders them
                            // as elements rather than navigating a
                            // webview, so the whole class of "a link in a
                            // PR description replaced the app" bug cannot
                            // arise here at all.
                            TextView::markdown("pr-body", body, window, cx)
                                .selectable(true)
                                .style(body_style()),
                        ),
                ),
        )
        .children(error.map(|message| {
            div()
                .flex_shrink_0()
                .bg(cx.theme().danger.opacity(0.12))
                .px_3()
                .py_2()
                .text_sm()
                .text_color(cx.theme().danger)
                .child(message)
        }))
        .child(footer(armed, busy, cx))
        .children(last_action.as_ref().map(|a| ui::action_bar(a, cx)))
}

/// Markdown at panel scale.
///
/// The defaults are sized for a document window: headings that grow by
/// level and a full rem between paragraphs. In a 392pt popover an `##`
/// in a PR description then renders larger than the PR's own title,
/// which inverts the hierarchy of the screen. Headings here are one step
/// above body copy and flat across levels — a PR description is a note,
/// not a document, and its `###` is not three levels of anything.
fn body_style() -> TextViewStyle {
    TextViewStyle::default()
        .paragraph_gap(rems(0.62))
        .heading_font_size(|_level, base| base * 1.05)
}

fn header(pr: &PullRequest, cx: &App) -> impl IntoElement {
    let base = if pr.base_ref_name.is_empty() {
        "unknown branch".to_string()
    } else {
        pr.base_ref_name.clone()
    };
    let muted = cx.theme().muted_foreground;
    let (checks, checks_color) = checks_value(pr.checks, cx);

    v_flex()
        .px_4()
        .pt_3()
        .gap_2()
        // Who, where, when. The repo is the one field long enough to
        // widen the panel, so it is the only one that shortens — by
        // characters, not pixels (see `format::ellipsize`), so the line
        // still reads as one sentence rather than a column with a hole
        // in it.
        .child(
            h_flex()
                .min_w_0()
                .gap_1p5()
                .items_center()
                .child(
                    Avatar::new()
                        .name(pr.author.login.clone())
                        .xsmall()
                        .bg(crate::ui::format::avatar_color(&pr.author.login))
                        .text_color(white())
                        .border_0(),
                )
                .child(
                    facts(
                        [
                            Fact::new(pr.author.login.clone())
                                .color(cx.theme().foreground)
                                .weight(FontWeight::MEDIUM),
                            Fact::new(ellipsize(&pr.repo, REPO_MAX_CHARS)),
                            Fact::new(relative_age(&pr.created_at, chrono::Utc::now())),
                        ],
                        cx,
                    )
                    .min_w_0()
                    .text_xs()
                    .text_color(muted),
                ),
        )
        .when(pr.asked_for_you, |this| {
            // The same accent the queue row's FOR YOU badge carries.
            // `primary` is #171717 in light mode, which made the one
            // thing on this screen that says "this is yours" look like a
            // disabled chip.
            this.child(h_flex().child(Tag::info().small().child("REVIEW REQUESTED FROM YOU")))
        })
        // The one large thing on the screen.
        .child(Label::new(pr.title.clone()).text_base().font_semibold())
        // The decision line: everything that answers "should I put my
        // name on this". The base branch is always shown here, unlike on
        // a list row — this is where "does this PR gate anything" gets
        // answered for real. One text run, so `main` sits on the same
        // baseline as the words around it: it used to be the only thing
        // on the line in a monospace face, and a different face has a
        // different ascent, which put it a point above its neighbours.
        .child(
            facts(
                [
                    Fact::new(format!("into {base}"))
                        .color(cx.theme().foreground)
                        .weight(FontWeight::MEDIUM),
                    Fact::new(checks).color(checks_color),
                    Fact::new(format!("+{}", pr.additions)).color(cx.theme().green),
                    Fact::new(format!("−{}", pr.deletions)).color(cx.theme().red),
                    Fact::new(format!(
                        "{} file{}",
                        pr.changed_files,
                        if pr.changed_files == 1 { "" } else { "s" }
                    )),
                ],
                cx,
            )
            .text_xs()
            .text_color(muted),
        )
        .when(!pr.labels.is_empty(), |this| {
            this.child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .children(pr.labels.iter().map(|l| label_chip(&l.name, &l.color, cx))),
            )
        })
}

/// GitHub's own label colour as a `Tag`.
///
/// `Tag::custom` takes `(color, foreground, border)`. Getting that order
/// wrong is how these once shipped as empty pills: the label's colour
/// landed on the foreground at 35% alpha, invisible against a fill of
/// the same hue. The label colour tints the chip; the text stays the
/// theme's reading colour, because GitHub label colours are chosen to be
/// seen as fills and not as type.
fn label_chip(name: &str, hex: &str, cx: &App) -> impl IntoElement {
    let base: Hsla = rgb(u32::from_str_radix(hex, 16).unwrap_or(0x8a8a8a)).into();
    Tag::custom(
        base.opacity(0.15),
        cx.theme().foreground,
        base.opacity(0.35),
    )
    .small()
    .child(name.to_string())
}

/// A missing CI status says so in words rather than rendering nothing —
/// a blank slot in a run of inline facts would read as missing data
/// rather than "this repo has no CI".
fn checks_value(state: CheckState, cx: &App) -> (&'static str, Hsla) {
    match state {
        CheckState::Success => ("checks pass", cx.theme().green),
        CheckState::Failure => ("checks failing", cx.theme().red),
        CheckState::Pending => ("checks running", cx.theme().yellow),
        CheckState::None => ("no checks", cx.theme().muted_foreground),
    }
}

/// macOS button placement: the confirming action is rightmost and
/// filled, its alternatives sit to its left in ascending
/// destructiveness, and the consequence is spelled out in the same bar
/// rather than in a dialog nobody reads.
///
/// Every button dispatches an action, so the keyboard path (⌘↵, ⌘O) and
/// the click path are the same code.
fn footer(armed: bool, busy: bool, cx: &App) -> impl IntoElement {
    h_flex()
        .flex_shrink_0()
        .w_full()
        .gap_1p5()
        .px_3()
        .py_2()
        .bg(cx.theme().title_bar)
        .border_t_1()
        .border_color(cx.theme().title_bar_border)
        // Trailing, like a macOS sheet's buttons: the confirming action
        // at the far right, its alternatives to its left. No caption —
        // the tooltip on Approve and the arming delay say what needs
        // saying; a standing "cannot be undone" read as nagging.
        .justify_end()
        // `detail.footer[0..=2]` are Open, Skip, Approve.
        .on_children_prepainted(ui::probe::children("detail.footer"))
        // Open and Skip share one visual weight, clearly subordinate to
        // Approve's filled button. Open is read-only, so unlike Skip it
        // is never disabled by an in-flight request.
        // `outline`, not the bare secondary variant: these sit on the
        // footer's own tinted bar, where a secondary button's fill is
        // near enough to the background that the two read as text rather
        // than as controls.
        .child(
            ui::text_button("open")
                .outline()
                .small()
                .label("Open")
                .tooltip("⌘O")
                .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenExternal), cx)),
        )
        .child(
            ui::text_button("skip")
                .outline()
                .small()
                .label("Skip")
                .disabled(busy)
                .on_click(|_, window, cx| window.dispatch_action(Box::new(Skip), cx)),
        )
        .child(
            ui::text_button("approve")
                // The success variant, built by hand so it carries the
                // same depth as the outline buttons beside it: the same
                // small shadow, and a border that is the fill taken a
                // twelfth of the way to black — the same subtlety as
                // their grey-on-white edge. The stock `.success()` is a
                // flat fill with no shadow and a border the colour of
                // itself, and next to Open and Skip it read as printed
                // on rather than raised; the theme's `success_active` as
                // a rim was a step too far the other way.
                .custom(
                    ButtonCustomVariant::new(cx)
                        .color(cx.theme().success)
                        .foreground(cx.theme().success_foreground)
                        .border(cx.theme().success.blend(black().opacity(0.12)))
                        .hover(cx.theme().success_hover)
                        .active(cx.theme().success_active)
                        .shadow(true),
                )
                .small()
                // The label's lift (see `ui::text_button`) carries the
                // icon with it, and an icon is centred by its box, not
                // its caps: this puts it back on the button's centre.
                .icon(Icon::new(IconName::ThumbsUp).mt(px(2.)))
                .label("Approve")
                .tooltip("⌘↵")
                // Approve is inert for 250ms after this screen appears, so
                // a fast double-click on a list row cannot carry through
                // into a public review. `loading` says so: a disabled
                // button with no explanation reads as broken, a spinner
                // reads as "not yet". It also blocks the click itself,
                // which is the same guarantee `disabled` gave.
                .loading(!armed && !busy)
                .disabled(busy)
                .on_click(|_, window, cx| window.dispatch_action(Box::new(Approve), cx)),
        )
}
