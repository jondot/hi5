//! Settings: two pages, each a column of grouped cards.
//!
//! The pages are the split the settings actually have — what hi5 does
//! (appearance, hotkey, which pull requests it hides, how it polls and
//! notifies, who it is signed in as) and which repositories it does it
//! to (organizations, the mute list, base branches). They are chosen
//! with a segmented control in the header, the way a macOS toolbar
//! switches panes.
//!
//! Each page is inset-grouped, the way System Settings and every iOS
//! settings screen are: a muted section title, then a card — the
//! panel's `background` colour on the page's `secondary`, ruled with the
//! theme's `border` — whose rows are separated by hairlines that run the
//! card's full width. The previous version drew filled grey boxes on a
//! white page, which is the same idea inverted, and read as a page of
//! disabled controls. The card and its rows are hi5's own layout because
//! the library keeps its own settings rows to itself
//! (`setting::SettingItem` is `pub(crate)`, reachable only through the
//! sidebar-and-pages `Settings` component, which does not fit a 392pt
//! panel); the controls in them are all stock — `Switch`, `TabBar`,
//! `Label`.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::input::Input;
use gpui_component::label::Label;
use gpui_component::switch::Switch;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::{h_flex, v_flex, ActiveTheme as _, Sizable as _};
use hi5_core::store::settings::Appearance;

use crate::actions::{
    SetAppearance, SetPollInterval, SignOut, ToggleOrg, ToggleRepoMute, ToggleRule,
};
use crate::app::{Hi5, SettingsTab};
use crate::ui;
use crate::ui::probe;

/// The poll intervals offered, in the order they are shown.
const INTERVALS: [(&str, u64); 3] = [("30s", 30), ("60s", 60), ("5 min", 300)];
const APPEARANCES: [(&str, Appearance); 3] = [
    ("System", Appearance::System),
    ("Light", Appearance::Light),
    ("Dark", Appearance::Dark),
];
/// The pages, in the order the control shows them.
const TABS: [(&str, SettingsTab); 2] = [
    ("General", SettingsTab::General),
    ("Repositories", SettingsTab::Repositories),
];

pub fn render(this: &mut Hi5, _window: &mut Window, cx: &mut Context<Hi5>) -> impl IntoElement {
    let tab = this.settings_tab;
    let sections = match tab {
        SettingsTab::General => general(this, cx),
        SettingsTab::Repositories => repositories(this, cx),
    };
    let tabs = TabBar::new("settings-tabs")
        .segmented()
        .small()
        .selected_index(TABS.iter().position(|(_, t)| *t == tab).unwrap_or(0))
        // `settings.tab[i]` is the ith page's segment; the probe rides
        // inside the segment's own box, so its centre is where a click
        // lands.
        .children(TABS.map(|(label, _)| Tab::new().label(label).child(probe::mark("settings.tab"))))
        .on_click(cx.listener(|this, ix: &usize, _, cx| {
            let tab = TABS.get(*ix).map_or(SettingsTab::General, |(_, t)| *t);
            this.set_settings_tab(tab, cx);
        }));

    v_flex()
        .size_full()
        .bg(cx.theme().secondary)
        .child(ui::screen_header(
            "Settings",
            Some(tabs.into_any_element()),
            cx,
        ))
        .child(
            v_flex()
                .id("settings-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_3()
                .gap_4()
                // Body size for the whole screen. Row titles and the
                // controls beside them inherit it, so a row, its switch
                // label and its segmented control are all the one step.
                .text_sm()
                .children(sections),
        )
}

/// What hi5 does.
fn general(this: &Hi5, cx: &App) -> Vec<AnyElement> {
    let s = &this.settings;

    // Cost of the current configuration, stated plainly rather than as a
    // fixed "~1%" — the query count scales with how many orgs are
    // watched, so a fixed number would go stale the moment someone
    // watches a second one.
    let queries_per_poll = s.watched_orgs.len() + 1;
    let polls_per_hour = 3600 / s.poll_interval_secs.max(1);
    let points_per_hour = queries_per_poll as u64 * polls_per_hour;

    vec![
        // First, because it is what the connect screen sends you here
        // for: who you are signed in as, and where gh is.
        section("Connection", connection(this, cx), None, cx),
        section(
            "Application",
            vec![
                row(
                    "Appearance",
                    None,
                    TabBar::new("appearance")
                        .segmented()
                        .small()
                        .selected_index(
                            APPEARANCES
                                .iter()
                                .position(|(_, a)| *a == s.appearance)
                                .unwrap_or(0),
                        )
                        .children(APPEARANCES.map(|(label, _)| Tab::new().label(label)))
                        .on_click(|ix: &usize, window, cx| {
                            let mode = match APPEARANCES.get(*ix) {
                                Some((_, Appearance::Light)) => "light",
                                Some((_, Appearance::Dark)) => "dark",
                                _ => "system",
                            };
                            window.dispatch_action(
                                Box::new(SetAppearance { mode: mode.into() }),
                                cx,
                            )
                        })
                        .into_any_element(),
                    cx,
                ),
                switch_row(
                    "launch-at-login",
                    "Launch at login",
                    Some("Only works from a built hi5.app"),
                    s.launch_at_login,
                    "launch_at_login",
                    cx,
                ),
                value_row("Global hotkey", format_hotkey(&s.hotkey), cx),
            ],
            None,
            cx,
        ),
        section(
            "Inbox",
            vec![
                switch_row(
                    "hide-reviewed",
                    "Hide PRs I already reviewed",
                    Some("Drops anything you approved or commented on"),
                    s.rules.hide_already_reviewed,
                    "hide_already_reviewed",
                    cx,
                ),
                switch_row(
                    "hide-drafts",
                    "Hide drafts",
                    Some("Skip PRs still marked as draft"),
                    s.rules.hide_drafts,
                    "hide_drafts",
                    cx,
                ),
            ],
            None,
            cx,
        ),
        section(
            "Notifications & polling",
            vec![
                switch_row(
                    "notifications",
                    "Banner for each new PR",
                    Some("One per newly-visible pull request"),
                    s.notifications_enabled,
                    "notifications",
                    cx,
                ),
                stacked_row(
                    "Check interval",
                    Some(format!(
                        "{queries_per_poll} quer{} per poll — about {points_per_hour} GraphQL points an hour of the 5000 budget",
                        if queries_per_poll == 1 { "y" } else { "ies" }
                    )),
                    TabBar::new("interval")
                        .segmented()
                        .small()
                        .selected_index(
                            INTERVALS
                                .iter()
                                .position(|(_, secs)| *secs == s.poll_interval_secs)
                                .unwrap_or(1),
                        )
                        .children(INTERVALS.map(|(label, _)| Tab::new().label(label)))
                        .on_click(|ix: &usize, window, cx| {
                            let secs = INTERVALS.get(*ix).map_or(60, |(_, s)| *s);
                            window.dispatch_action(Box::new(SetPollInterval { secs }), cx)
                        })
                        .into_any_element(),
                    cx,
                ),
            ],
            None,
            cx,
        ),
    ]
}

/// Which repositories it does it to.
fn repositories(this: &Hi5, cx: &App) -> Vec<AnyElement> {
    let s = &this.settings;

    // Unioned with the enabled list rather than shown raw, so an org the
    // user has toggled off still appears (unchecked) instead of
    // vanishing the moment it is unwatched — and so a failed discovery
    // still shows whatever was already enabled.
    let mut orgs: Vec<String> = this.org_candidates.clone();
    orgs.extend(s.watched_orgs.iter().cloned());
    orgs.sort_by_key(|o| o.to_lowercase());
    orgs.dedup();

    // Muted repos are unioned back in because the inbox has already
    // stripped their PRs: deriving this list from the inbox alone made
    // muting a one-way door — switch a repo off and it disappeared from
    // the list, with no way to switch it back on short of hand-editing
    // settings.json. They stay listed, just off.
    let muted = &s.repos.muted;
    let mut repos: Vec<String> = this.prs.iter().map(|p| p.repo.clone()).collect();
    repos.extend(muted.iter().cloned());
    repos.sort_by_key(|r| r.to_lowercase());
    repos.dedup();

    let branch_status = hi5_core::inbox::branch_watch_status(s, &this.backend.state());

    vec![
        section(
            "Watched organizations",
            if orgs.is_empty() {
                vec![note_row(
                    "hi5 is still discovering which organizations you belong to.",
                    cx,
                )]
            } else {
                orgs.iter()
                    .map(|org| {
                        let on = s.watched_orgs.contains(org);
                        let key = org.clone();
                        row(
                            org.clone(),
                            None,
                            Switch::new(SharedString::from(format!("org-{org}")))
                                .checked(on)
                                .small()
                                .on_click(move |_, window, cx| {
                                    window.dispatch_action(
                                        Box::new(ToggleOrg { org: key.clone() }),
                                        cx,
                                    )
                                })
                                .into_any_element(),
                            cx,
                        )
                    })
                    .collect()
            },
            Some("Pull requests are searched in every organization switched on."),
            cx,
        ),
        // Distinct from the toolbar's repo focus, and deliberately so:
        // this edits `settings.repos.muted`, the subtractive list that
        // drops a repo's PRs from every query's results for good. The
        // toolbar filter is "let me sit on these repos for a while";
        // this is "I never want to see that repo". Two different jobs,
        // two different places.
        section(
            "Repositories",
            if repos.is_empty() {
                vec![note_row(
                    "Repositories appear here once they show up in your inbox.",
                    cx,
                )]
            } else {
                repos
                    .into_iter()
                    .map(|repo| {
                        let on = !muted.contains(&repo);
                        let key = repo.clone();
                        row(
                            repo.clone(),
                            None,
                            Switch::new(SharedString::from(format!("mute-{repo}")))
                                .checked(on)
                                .small()
                                .on_click(move |_, window, cx| {
                                    window.dispatch_action(
                                        Box::new(ToggleRepoMute { repo: key.clone() }),
                                        cx,
                                    )
                                })
                                .into_any_element(),
                            cx,
                        )
                    })
                    .collect()
            },
            Some(
                "Switch a repository off to mute it: its pull requests leave the inbox \
                 until it is switched back on. The toolbar filter is a temporary focus; \
                 this is for good.",
            ),
            cx,
        ),
        // A PR from one feature branch into another gates nothing and is
        // dropped from the inbox entirely — not dimmed, not label-only.
        // Silent hiding is the failure mode this project has been bitten
        // by, so this section is the one place that explains, per repo,
        // which branches count and where that answer came from.
        section(
            "Base branches",
            std::iter::once(fact(
                "Fallback branches",
                s.branch_watch.global.join(", "),
                cx,
            ))
            .chain(branch_status.into_iter().map(|info| {
                let watching = if info.branches.is_empty() {
                    "every branch".to_string()
                } else {
                    info.branches.join(", ")
                };
                fact(
                    info.repo.clone(),
                    format!("{watching} — {}", source_label(info.source)),
                    cx,
                )
            }))
            .collect(),
            Some(
                "Only PRs targeting a watched branch appear in the inbox. hi5 detects each \
                 repo's protected branches automatically; the fallback list applies when \
                 that can't be determined.",
            ),
            cx,
        ),
    ]
}

/// One titled card of rows, with an optional footnote under it.
///
/// The title and the footnote are inset to the rows' text (`px_3`, the
/// rows' own padding), so the three left edges align; the rows are
/// separated by hairlines the full width of the card.
fn section(
    title: &'static str,
    rows: Vec<AnyElement>,
    footer: Option<&'static str>,
    cx: &App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    v_flex()
        .w_full()
        .gap_1p5()
        .child(Label::new(title).px_3().text_xs().text_color(muted))
        .child(
            v_flex()
                .w_full()
                .bg(cx.theme().background)
                .border_1()
                .border_color(cx.theme().border)
                .rounded(cx.theme().radius_lg)
                .overflow_hidden()
                .on_children_prepainted(probe::children("settings.row"))
                .children(rows.into_iter().enumerate().map(|(i, row)| {
                    div()
                        .w_full()
                        .when(i > 0, |this| {
                            this.border_t_1().border_color(cx.theme().border)
                        })
                        .child(row)
                })),
        )
        .children(footer.map(|text| Label::new(text).px_3().text_xs().text_color(muted)))
        .into_any_element()
}

/// A settings row: a title, an optional sub-line, and a trailing
/// control.
fn row(
    title: impl Into<SharedString>,
    sub: Option<String>,
    control: AnyElement,
    cx: &App,
) -> AnyElement {
    h_flex()
        .w_full()
        .min_h(px(40.))
        .px_3()
        .py_2()
        .gap_3()
        .items_center()
        .justify_between()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(Label::new(title))
                .children(sub.map(|sub| {
                    Label::new(sub)
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                })),
        )
        .child(div().flex_shrink_0().child(control))
        .into_any_element()
}

/// A settings row whose control needs room: the control sits under the
/// text rather than beside it.
///
/// The three-segment interval picker and a three-line explanation cannot
/// share 344 points. Side by side, the sentence wrapped to three lines
/// and squeezed the control against the trailing edge; stacked, both are
/// full width and legible.
fn stacked_row(
    title: impl Into<SharedString>,
    sub: Option<String>,
    control: AnyElement,
    cx: &App,
) -> AnyElement {
    v_flex()
        .w_full()
        .px_3()
        .py_2()
        .gap_2()
        .child(
            v_flex()
                .w_full()
                .child(Label::new(title))
                .children(sub.map(|sub| {
                    Label::new(sub)
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                })),
        )
        .child(control)
        .into_any_element()
}

fn switch_row(
    id: &'static str,
    title: &'static str,
    sub: Option<&'static str>,
    on: bool,
    which: &'static str,
    cx: &App,
) -> AnyElement {
    row(
        title,
        sub.map(str::to_string),
        Switch::new(id)
            .checked(on)
            .small()
            .on_click(move |_, window, cx| {
                window.dispatch_action(
                    Box::new(ToggleRule {
                        which: which.to_string(),
                    }),
                    cx,
                )
            })
            .into_any_element(),
        cx,
    )
}

/// A read-only row: the value sits at the trailing edge, muted, where a
/// control would. For values short enough to fit there — see [`fact`]
/// for the ones that are not.
fn value_row(title: &'static str, value: impl Into<SharedString>, cx: &App) -> AnyElement {
    row(
        title,
        None,
        Label::new(value)
            .text_color(cx.theme().muted_foreground)
            .into_any_element(),
        cx,
    )
}

/// A read-only pair: a label and the value under it, for values that
/// run long — a list of scopes, an error message, a list of branches.
fn fact(title: impl Into<SharedString>, value: impl Into<SharedString>, cx: &App) -> AnyElement {
    v_flex()
        .w_full()
        .px_3()
        .py_2()
        .child(Label::new(title))
        .child(
            Label::new(value)
                .text_xs()
                .text_color(cx.theme().muted_foreground),
        )
        .into_any_element()
}

/// A card's only row, when there is nothing to list yet: says why.
fn note_row(text: &'static str, cx: &App) -> AnyElement {
    div()
        .w_full()
        .px_3()
        .py_2()
        .child(
            Label::new(text)
                .text_xs()
                .text_color(cx.theme().muted_foreground),
        )
        .into_any_element()
}

/// The raw backend string lives here, in full.
///
/// The status strip shows a humanised version; this is where the actual
/// diagnostic stays readable. Softening the wording up there must not
/// soften the signal down here — an inbox that silently emptied itself
/// was a real shipped bug.
fn connection(this: &Hi5, cx: &App) -> Vec<AnyElement> {
    let (login, scopes) = match &this.auth {
        Some(hi5_core::auth::AuthState::Connected { login, scopes, .. }) => {
            (login.clone(), scopes.join(", "))
        }
        _ => (String::new(), String::new()),
    };
    let mut rows = vec![value_row(
        "Signed in",
        if login.is_empty() {
            "not connected".to_string()
        } else {
            login
        },
        cx,
    )];
    if !scopes.is_empty() {
        rows.push(fact("Token scopes", scopes, cx));
    }
    if let Some(err) = this.poll_error.clone() {
        rows.push(fact("Last poll error", err, cx));
    }
    // Where gh is. The field is the user's override (blank: find it);
    // the line under it is what hi5 actually resolved on the last check
    // — the answer to "which gh is it running", and to "why can't it
    // find mine". Reachable from the connect screen for exactly that
    // case, see `ui::auth`.
    rows.push(stacked_row(
        "GitHub CLI",
        Some("Where hi5 runs `gh`. Leave empty to find it automatically.".into()),
        v_flex()
            .w_full()
            .gap_1()
            .child(
                div()
                    .w_full()
                    // `settings.gh-input[0]` is the field.
                    .on_children_prepainted(ui::probe::children("settings.gh-input"))
                    .child(Input::new(&this.gh_path_input).small().w_full()),
            )
            .child(
                Label::new(gh_readout(this))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground),
            )
            .into_any_element(),
        cx,
    ));
    // Signing out is hi5's own: polling stops and the credential goes
    // unused until "Sign in" on the screen that follows. The CLI's
    // session is left alone — see `Hi5::sign_out`.
    if matches!(this.auth, Some(hi5_core::auth::AuthState::Connected { .. })) {
        rows.push(row(
            "Sign out of hi5",
            Some("Stops polling. The GitHub CLI stays signed in.".into()),
            ui::text_button("sign-out")
                .outline()
                .small()
                .label("Sign out")
                .on_click(|_, window, cx| window.dispatch_action(Box::new(SignOut), cx))
                .into_any_element(),
            cx,
        ));
    }
    rows
}

/// The line under the gh path field.
fn gh_readout(this: &Hi5) -> String {
    use hi5_core::auth::runner::Resolution;
    match &this.gh_resolution {
        None => "Checking…".into(),
        Some(Resolution {
            path,
            runnable: true,
            ..
        }) => format!("Using {}", path.display()),
        Some(Resolution {
            path,
            overridden: true,
            ..
        }) => format!("Nothing runnable at {}", path.display()),
        Some(_) => "Not found in your PATH, Homebrew, MacPorts, nix, or your login shell.".into(),
    }
}

fn source_label(source: &str) -> &'static str {
    match source {
        "override" => "your override",
        "detected" => "detected",
        "global" => "global default",
        "default" => "repo's default branch",
        _ => "not filtered",
    }
}

/// The raw accelerator (`Alt+Cmd+A`) as the glyphs macOS shows (⌥⌘A).
fn format_hotkey(hotkey: &str) -> String {
    hotkey
        .split('+')
        .map(|part| match part.trim() {
            "Cmd" | "Command" => "⌘",
            "Alt" | "Option" => "⌥",
            "Shift" => "⇧",
            "Ctrl" | "Control" => "⌃",
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{format_hotkey, source_label};

    #[test]
    fn renders_the_accelerator_as_macos_glyphs() {
        assert_eq!(format_hotkey("Alt+Cmd+A"), "⌥⌘A");
        assert_eq!(format_hotkey("Ctrl+Shift+K"), "⌃⇧K");
    }

    #[test]
    fn names_every_branch_source_in_words() {
        assert_eq!(source_label("detected"), "detected");
        assert_eq!(source_label("override"), "your override");
        assert_eq!(source_label("anything else"), "not filtered");
    }
}
