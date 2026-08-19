//! Connecting to GitHub.
//!
//! Every branch here ends in something the user can actually do. A state
//! that only reports a problem is a dead end, and this screen is the one
//! place hi5 can be entered from a broken state.
//!
//! Two things every branch has that the first release did not: a
//! caption saying when the last Check again answered — because a check
//! that lands on the same state changed nothing on screen and the
//! button read as dead — and a way out. This screen has no toolbar and
//! so no ⋯ menu; without "Quit hi5" here, an app that could not find
//! `gh` could not be quit either.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::button::ButtonVariants as _;
use gpui_component::label::Label;
use gpui_component::{
    h_flex, v_flex, ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
};
use hi5_core::auth::AuthState;

use crate::app::Hi5;
use crate::ui;

pub fn render(this: &mut Hi5, cx: &mut Context<Hi5>) -> impl IntoElement {
    let state = this.auth.clone();
    let checking = this.checking_auth;
    let checked_at = this.auth_checked_at;
    // When gh was not found: where hi5 looked, or what the override
    // pointed at, so the next step (Settings) is obvious.
    let gh_note: Option<String> = match (&state, &this.gh_resolution) {
        (Some(AuthState::GhNotInstalled { .. }), Some(r)) if r.overridden => Some(format!(
            "Nothing runnable at {} — change it in Settings.",
            r.path.display()
        )),
        (Some(AuthState::GhNotInstalled { .. }), _) => Some(
            "Looked in your PATH, Homebrew, MacPorts, nix and your login shell. \
             If gh is somewhere else, set its path in Settings."
                .into(),
        ),
        _ => None,
    };

    let (title, detail, action): (String, String, Option<&'static str>) = match &state {
        Some(AuthState::Connected { login, .. }) => (
            "Signed in to GitHub".into(),
            format!("as {login}"),
            Some("Use this account"),
        ),
        Some(AuthState::GhNotInstalled { homebrew_available }) => (
            "The GitHub CLI isn't installed".into(),
            if *homebrew_available {
                "hi5 signs in through `gh`. Install it with `brew install gh`, then press Check again.".into()
            } else {
                "hi5 signs in through `gh`. Install it from cli.github.com, then press Check again."
                    .into()
            },
            Some("Check again"),
        ),
        Some(AuthState::GhNotAuthenticated) => (
            "The GitHub CLI isn't signed in".into(),
            "Run `gh auth login` in a terminal, then press Check again.".into(),
            Some("Check again"),
        ),
        Some(AuthState::NeedsToken) => (
            "hi5 needs a token".into(),
            "`gh` is signed in but didn't hand over a token. Run `gh auth token` to check.".into(),
            Some("Check again"),
        ),
        Some(AuthState::Disconnected { reason }) => (
            "Lost access to GitHub".into(),
            format!("{reason}. Run `gh auth login` to reconnect."),
            Some("Check again"),
        ),
        Some(AuthState::SignedOut) => (
            "Signed out".into(),
            "hi5 isn't polling or using your GitHub CLI account. Sign in to pick up where you left off."
                .into(),
            Some("Sign in"),
        ),
        None => ("Connecting…".into(), String::new(), None),
    };

    // A classic token without `repo` authenticates fine and then quietly
    // returns public results only — a short inbox with no explanation.
    // Blocking on it is the point.
    let scope_problem = matches!(
        &state,
        Some(AuthState::Connected {
            scopes_adequate: false,
            ..
        })
    );

    v_flex()
        .relative()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .px_8()
        .bg(cx.theme().background)
        .child(
            Icon::new(IconName::GitHub)
                .size_10()
                .text_color(cx.theme().muted_foreground),
        )
        .child(
            Label::new(if scope_problem {
                "Your token can't see private repos".to_string()
            } else {
                title
            })
            .text_base()
            .font_semibold()
            .text_center(),
        )
        .child(
            Label::new(if scope_problem {
                "Run `gh auth refresh -s repo` so hi5 can see private pull requests.".to_string()
            } else {
                detail
            })
            .text_sm()
            .text_center()
            .text_color(cx.theme().muted_foreground),
        )
        .when_some(action, |this, label| {
            this.child(
                // `auth.action[0]` is the button.
                div()
                    .on_children_prepainted(ui::probe::children("auth.action"))
                    .child(
                        ui::text_button("auth-action")
                            .primary()
                            .small()
                            .label(label)
                            .loading(checking)
                            .on_click(cx.listener(|this, _, _, cx| this.retry_auth(cx))),
                    ),
            )
        })
        .when_some(gh_note, |this, note| {
            this.child(
                Label::new(note)
                    .text_xs()
                    .text_center()
                    .text_color(cx.theme().muted_foreground),
            )
        })
        // The answer to "did that do anything": the time the last check
        // came back. Wall clock, so a glance at the menu bar's clock
        // confirms it was just now.
        .when_some(checked_at, |this, at| {
            this.child(
                Label::new(format!("Checked at {}", at.format("%H:%M:%S")))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground),
            )
        })
        // Two ways onward that need no credential: Settings, where the
        // gh path lives (`auth.settings[0]`), and Quit (`auth.quit[0]`).
        .child(
            h_flex()
                .absolute()
                .bottom_3()
                .gap_1()
                .child(
                    div()
                        .on_children_prepainted(ui::probe::children("auth.settings"))
                        .child(
                            ui::text_button("auth-settings")
                                .ghost()
                                .small()
                                .text_color(cx.theme().muted_foreground)
                                .label("Settings…")
                                .on_click(|_, window, cx| {
                                    window
                                        .dispatch_action(Box::new(crate::actions::OpenSettings), cx)
                                }),
                        ),
                )
                .child(
                    div()
                        .on_children_prepainted(ui::probe::children("auth.quit"))
                        .child(
                            ui::text_button("auth-quit")
                                .ghost()
                                .small()
                                .text_color(cx.theme().muted_foreground)
                                .label("Quit hi5")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(crate::actions::Quit), cx)
                                }),
                        ),
                ),
        )
}
