//! What a click or a key actually asks the backend to do — recorded by
//! `Backend::null`, which performs none of it. Approve on the live
//! backend posts a public review, so this is the only place it is ever
//! exercised.

use std::time::Duration;

use gpui::TestAppContext;

use hi5_core::auth::AuthState;
use hi5_core::poller::PollEvent;

use crate::actions::{OpenSettings, SignOut};
use crate::app::{LastAction, Screen, SettingsTab, ARMING};
use crate::backend::{Command, CommandResult, Msg};
use crate::testing::Harness;

fn open_first(h: &mut Harness) {
    h.click("inbox.row", 0);
    assert!(matches!(h.screen(), Screen::Detail(_)));
}

/// Approve is inert for `ARMING` after the detail appears, so a fast
/// double-click on a row cannot carry through into a public review. The
/// button shows a spinner meanwhile and swallows the click.
#[gpui::test]
fn approve_is_inert_until_armed_and_then_reaches_the_backend(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    open_first(&mut h);

    // `detail.footer[2]` is Approve.
    h.click("detail.footer", 2);
    assert!(
        !h.commands()
            .iter()
            .any(|c| matches!(c, Command::Approve { .. })),
        "an immediate click must not approve"
    );

    std::thread::sleep(ARMING + Duration::from_millis(30));
    h.click("detail.footer", 2);
    let approves: Vec<_> = h
        .commands()
        .into_iter()
        .filter(|c| matches!(c, Command::Approve { .. }))
        .collect();
    assert_eq!(approves.len(), 1, "one click, one approve: {approves:?}");
}

#[gpui::test]
fn cmd_enter_approves_once_armed(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    open_first(&mut h);
    h.keys("cmd-enter");
    assert!(!h
        .commands()
        .iter()
        .any(|c| matches!(c, Command::Approve { .. })));
    std::thread::sleep(ARMING + Duration::from_millis(30));
    h.keys("cmd-enter");
    assert!(h
        .commands()
        .iter()
        .any(|c| matches!(c, Command::Approve { .. })));
}

#[gpui::test]
fn skip_reaches_the_backend_with_the_head_sha(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    open_first(&mut h);
    let Screen::Detail(pr) = h.screen() else {
        unreachable!()
    };
    // `detail.footer[1]` is Skip.
    h.click("detail.footer", 1);
    assert!(h.commands().contains(&Command::Skip {
        id: pr.id.clone(),
        head_sha: pr.head_sha.clone(),
        repo: pr.repo.clone(),
        number: pr.number,
    }));
}

#[gpui::test]
fn open_goes_to_the_pull_request_on_github(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    open_first(&mut h);
    let Screen::Detail(pr) = h.screen() else {
        unreachable!()
    };
    // `detail.footer[0]` is Open.
    h.click("detail.footer", 0);
    assert_eq!(h.cx.opened_url().as_deref(), Some(pr.url.as_str()));
}

/// Settings is one screen with two pages, chosen in its header. The page
/// is remembered for the session; ‹ leaves the screen from either.
#[gpui::test]
fn settings_has_two_pages_and_remembers_which(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    h.cx.dispatch_action(OpenSettings);
    h.draw();
    assert!(matches!(h.screen(), Screen::Settings));
    assert_eq!(h.read(|t| t.settings_tab), SettingsTab::General);
    assert!(
        h.bounds("settings.row").len() >= 6,
        "the general page has rows"
    );

    // `settings.tab[1]` is Repositories.
    h.click("settings.tab", 1);
    assert_eq!(h.read(|t| t.settings_tab), SettingsTab::Repositories);
    let repo_rows = h.bounds("settings.row");
    assert!(!repo_rows.is_empty(), "the repositories page has rows");

    h.click("header", 0);
    assert!(matches!(h.screen(), Screen::Inbox));
    h.cx.dispatch_action(OpenSettings);
    h.draw();
    assert_eq!(h.read(|t| t.settings_tab), SettingsTab::Repositories);
}

/// A refresh asks the backend once and shows as in flight until the
/// poller answers with any event — every cycle ends in exactly one.
#[gpui::test]
fn refresh_is_in_flight_until_the_poller_answers(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    assert!(!h.read(|t| t.refreshing));
    h.keys("cmd-r");
    assert!(h.commands().contains(&Command::Refresh));
    assert!(h.read(|t| t.refreshing), "spinning while the poller works");
    h.receive(crate::fixtures::pull_requests());
    assert!(!h.read(|t| t.refreshing), "the answer stops the spinner");
}

/// Signing out is hi5's own state, and it is reflected everywhere at
/// once: the setting is saved (which is what stops the poller), the
/// queue is dropped, the badge goes quiet, and the panel shows the
/// signed-out screen. Signing back in from that screen reverses the
/// setting and asks the backend for the credential again — nothing
/// about the GitHub CLI's own session is touched on either path.
#[gpui::test]
fn sign_out_and_back_in_are_reflected_in_every_state(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    // Connected, past first run: the inbox.
    h.app.update(&mut h.cx, |this, cx| {
        this.settings.has_completed_first_run = true;
        this.handle(
            Msg::Poll(PollEvent::AuthChanged(AuthState::Connected {
                login: "jondot".into(),
                source: "gh".into(),
                scopes: vec!["repo".into()],
                scopes_adequate: true,
                verified: true,
            })),
            cx,
        );
    });
    h.draw();
    assert!(!h.read(|t| t.needs_auth()));
    assert!(!h.bounds("inbox.row").is_empty());

    h.cx.dispatch_action(SignOut);
    h.draw();
    assert!(
        h.read(|t| t.settings.signed_out),
        "the setting is what the poller reads"
    );
    assert!(
        h.commands().contains(&Command::SaveSettings),
        "and it was saved"
    );
    assert!(matches!(
        h.read(|t| t.auth.clone()),
        Some(AuthState::SignedOut)
    ));
    assert!(h.read(|t| t.needs_auth()), "the signed-out screen shows");
    assert!(h.read(|t| t.prs.is_empty()), "the queue is dropped");
    assert!(h.bounds("inbox.row").is_empty());
    assert_eq!(
        crate::decisions::badge(h.read(|t| t.auth.clone()).as_ref(), 0),
        crate::decisions::Badge::Quiet
    );

    // `auth.action[0]` is the screen's one button: "Sign in".
    let before = h.commands().len();
    h.click("auth.action", 0);
    assert!(!h.read(|t| t.settings.signed_out));
    assert!(
        h.read(|t| t.checking_auth),
        "the button spins until the backend answers"
    );
    assert!(
        h.read(|t| t.refreshing),
        "and so does refresh: the save woke a cycle"
    );
    let after: Vec<_> = h.commands().into_iter().skip(before).collect();
    assert!(
        after.contains(&Command::SaveSettings),
        "signing in is saved: {after:?}"
    );
    assert!(
        after.contains(&Command::CheckAuth),
        "and the credential re-checked"
    );
    // The answer comes back connected, and the inbox is where we land.
    h.app.update(&mut h.cx, |this, cx| {
        this.handle(
            Msg::Poll(PollEvent::AuthChanged(AuthState::Connected {
                login: "jondot".into(),
                source: "gh".into(),
                scopes: vec!["repo".into()],
                scopes_adequate: true,
                verified: true,
            })),
            cx,
        );
    });
    assert!(!h.read(|t| t.checking_auth));
    // Until the cycle lands the inbox is loading, not empty.
    assert!(h.read(|t| t.loading()));
    h.receive(crate::fixtures::pull_requests());
    assert!(!h.read(|t| t.needs_auth()));
    assert!(!h.read(|t| t.loading() || t.refreshing));
    assert!(matches!(h.screen(), Screen::Inbox));
}

/// The ids of every `Approve` the backend has been asked for, in order.
fn approves(h: &Harness) -> Vec<String> {
    h.commands()
        .into_iter()
        .filter_map(|c| match c {
            Command::Approve { id, .. } => Some(id),
            _ => None,
        })
        .collect()
}

/// The first section as drawn: the fixture queue's four `acme-labs/atlas`
/// pull requests, in list order.
fn first_section(h: &mut Harness) -> Vec<hi5_core::github::PullRequest> {
    h.read(|t| {
        let view = hi5_core::view::inbox_view(&t.prs, &t.focus_repos, t.scope);
        let repo = view.visible[0].repo.clone();
        view.visible
            .into_iter()
            .filter(|p| p.repo == repo)
            .cloned()
            .collect()
    })
}

#[gpui::test]
fn approve_all_asks_first_and_then_approves_the_section_in_order(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    let section = first_section(&mut h);
    assert_eq!(section.len(), 4, "the fixture's first section");

    // The header button opens a confirmation that lists the section.
    // Nothing has been approved.
    h.click("inbox.approve-all", 0);
    assert!(approves(&h).is_empty(), "the click only asks");
    assert_eq!(
        h.bounds("dialog.pr").len(),
        4,
        "every PR of the section is listed"
    );
    assert!(h.read(|t| t.batch.is_none()));

    // OK: the first request goes at once, the rest wait their turn.
    h.click("dialog.footer", 1);
    assert!(h.bounds("dialog.pr").is_empty(), "the dialog closed");
    assert_eq!(approves(&h), vec![section[0].id.clone()]);
    assert!(h.read(|t| t.busy));
    assert_eq!(h.read(|t| t.batch.as_ref().map(|b| b.queue.len())), Some(3));
    assert_eq!(
        h.bounds("inbox.approve-all").len(),
        3,
        "the header buttons stay put"
    );

    // Each answer sends the next; the approved PR leaves the list.
    for (i, pr) in section.iter().enumerate() {
        h.result(CommandResult::Approved {
            id: pr.id.clone(),
            repo: pr.repo.clone(),
            number: pr.number,
        });
        let sent = approves(&h);
        assert_eq!(
            sent.len(),
            (i + 2).min(4),
            "one in flight at a time: {sent:?}"
        );
        assert!(h.read(|t| !t.prs.iter().any(|p| p.id == pr.id)));
    }
    let expected: Vec<String> = section.iter().map(|p| p.id.clone()).collect();
    assert_eq!(
        approves(&h),
        expected,
        "the section, in the order it was shown"
    );
    assert!(h.read(|t| t.batch.is_none()), "the batch is over");
    assert!(h.read(|t| !t.busy));
    assert!(matches!(
        h.read(|t| t.last_action.clone()),
        Some(LastAction::ApprovedAll {
            approved: 4,
            total: 4,
            ..
        })
    ));
    assert!(h.read(|t| t.action_error.is_none()));
    assert_eq!(h.bounds("inbox.header").len(), 1, "one section left");
}

#[gpui::test]
fn approve_all_cancelled_approves_nothing(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    let before = h.bounds("inbox.row").len();

    h.click("inbox.approve-all", 0);
    assert_eq!(h.bounds("dialog.pr").len(), 4);
    h.click("dialog.footer", 0);
    assert!(h.bounds("dialog.pr").is_empty(), "Cancel closed it");
    assert!(approves(&h).is_empty());
    assert!(h.read(|t| t.batch.is_none() && !t.busy));

    // Escape is Cancel too — and it is the dialog's Escape, not the
    // panel's, which would have left the dialog standing.
    h.click("inbox.approve-all", 0);
    assert_eq!(h.bounds("dialog.pr").len(), 4);
    h.keys("escape");
    assert!(h.bounds("dialog.pr").is_empty(), "Escape closed it");
    assert!(approves(&h).is_empty());
    assert_eq!(h.bounds("inbox.row").len(), before, "nothing left the list");
    assert!(matches!(h.screen(), Screen::Inbox));
}

#[gpui::test]
fn enter_in_the_confirmation_confirms_and_does_not_open_a_row(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    let section = first_section(&mut h);
    // A selected row, so a stray Enter would have somewhere to go.
    h.keys("down");
    h.click("inbox.approve-all", 0);
    assert_eq!(h.bounds("dialog.pr").len(), 4);
    h.keys("enter");
    assert!(h.bounds("dialog.pr").is_empty(), "Enter is the dialog's OK");
    assert!(
        matches!(h.screen(), Screen::Inbox),
        "and not the row's Open"
    );
    assert_eq!(approves(&h), vec![section[0].id.clone()]);
}

#[gpui::test]
fn a_failure_inside_approve_all_is_counted_and_the_rest_go_on(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    let section = first_section(&mut h);
    h.click("inbox.approve-all", 0);
    h.click("dialog.footer", 1);

    for (i, pr) in section.iter().enumerate() {
        if i == 1 {
            h.result(CommandResult::ApproveFailed {
                repo: pr.repo.clone(),
                number: pr.number,
                message: "merged since".into(),
            });
            assert!(
                h.read(|t| t.prs.iter().any(|p| p.id == pr.id)),
                "the one that failed stays in the list"
            );
        } else {
            h.result(CommandResult::Approved {
                id: pr.id.clone(),
                repo: pr.repo.clone(),
                number: pr.number,
            });
        }
    }
    assert_eq!(approves(&h).len(), 4, "every one was attempted");
    assert!(h.read(|t| t.batch.is_none()));
    assert!(matches!(
        h.read(|t| t.last_action.clone()),
        Some(LastAction::ApprovedAll {
            approved: 3,
            total: 4,
            ..
        })
    ));
    let error = h
        .read(|t| t.action_error.clone())
        .expect("the tally names the failure");
    assert!(error.starts_with("1 of 4"), "{error}");
}

#[gpui::test]
fn check_again_says_when_it_answered_and_the_screen_has_a_way_out(cx: &mut TestAppContext) {
    let mut h = Harness::new(cx);
    let not_installed = || AuthState::GhNotInstalled {
        homebrew_available: true,
    };
    h.app.update(&mut h.cx, |this, cx| {
        this.handle(Msg::Poll(PollEvent::AuthChanged(not_installed())), cx)
    });
    h.draw();
    assert!(h.read(|t| t.needs_auth()));
    assert!(
        h.read(|t| t.auth_checked_at.is_none()),
        "nothing asked for yet"
    );
    assert_eq!(h.bounds("auth.quit").len(), 1, "Quit is on the screen");

    // Check again: the button spins, the backend is asked …
    h.click("auth.action", 0);
    assert!(h.read(|t| t.checking_auth));
    assert!(h.commands().contains(&Command::CheckAuth));

    // … and the same answer coming back still leaves a visible trace.
    h.app.update(&mut h.cx, |this, cx| {
        this.handle(Msg::Poll(PollEvent::AuthChanged(not_installed())), cx)
    });
    h.draw();
    assert!(!h.read(|t| t.checking_auth));
    assert!(
        h.read(|t| t.auth_checked_at.is_some()),
        "the caption says when the check answered"
    );
}

#[gpui::test]
fn settings_is_reachable_without_a_credential_and_the_gh_path_field_commits(
    cx: &mut TestAppContext,
) {
    let mut h = Harness::new(cx);
    h.app.update(&mut h.cx, |this, cx| {
        this.handle(
            Msg::Poll(PollEvent::AuthChanged(AuthState::GhNotInstalled {
                homebrew_available: false,
            })),
            cx,
        )
    });
    h.draw();
    assert!(h.read(|t| t.needs_auth()));
    assert!(
        h.bounds("settings.gh-input").is_empty(),
        "the connect screen, not settings"
    );

    // Settings… on the connect screen opens Settings despite needs_auth.
    h.click("auth.settings", 0);
    assert!(matches!(h.screen(), Screen::Settings));
    assert_eq!(
        h.bounds("settings.gh-input").len(),
        1,
        "the gh path field is on the page"
    );

    // Type a path and press Enter: it is saved and re-checked through.
    let before = h.commands().len();
    h.click("settings.gh-input", 0);
    h.keys("/ o p t / x / g h enter");
    assert_eq!(
        h.read(|t| t.settings.gh_path.clone()),
        Some("/opt/x/gh".to_string()),
        "Enter committed the field, not the panel's OpenSelected"
    );
    let after: Vec<_> = h.commands().into_iter().skip(before).collect();
    assert!(after.contains(&Command::SaveSettings), "{after:?}");
    assert!(after.contains(&Command::CheckAuth), "{after:?}");
    assert!(h.read(|t| t.checking_auth));

    // Back returns to the connect screen, which now knows where it looked.
    h.app.update(&mut h.cx, |this, cx| {
        this.handle(
            Msg::GhResolved(hi5_core::auth::runner::Resolution {
                path: "/opt/x/gh".into(),
                runnable: false,
                overridden: true,
            }),
            cx,
        );
        this.handle(
            Msg::Poll(PollEvent::AuthChanged(AuthState::GhNotInstalled {
                homebrew_available: false,
            })),
            cx,
        );
    });
    h.cx.dispatch_action(crate::actions::Back);
    h.draw();
    assert!(matches!(h.screen(), Screen::Inbox));
    assert_eq!(
        h.bounds("auth.settings").len(),
        1,
        "back on the connect screen"
    );
    assert!(h.bounds("settings.gh-input").is_empty());
}
