//! Every command the panel can perform, as a gpui action.
//!
//! Two things need these, and that is the point of having them:
//!
//!   * **The keyboard.** Bound once in [`bind`] and dispatched through
//!     the focus tree, instead of one `on_key_down` match reading raw
//!     keystroke strings. The bindings are now the keymap — there is one
//!     place to read what ⌘↵ does, and it cannot drift from what the
//!     menu says it does.
//!   * **`PopupMenu`.** Its items *are* actions, so the ⋯ menu and the
//!     repo filter can be stock menus rather than hand-drawn overlays
//!     with their own hit-testing and dismissal.
//!
//! The payload actions carry `String`s rather than indices deliberately:
//! a repo can leave the queue between the frame that drew the menu and
//! the click that lands on it, and an index would then toggle whichever
//! repo slid into that position.

use gpui::{actions, Action};
use serde::Deserialize;

actions!(
    hi5,
    [
        /// Move the keyboard selection down the drawn list.
        SelectNext,
        /// Move the keyboard selection up.
        SelectPrev,
        /// Open the selected pull request.
        OpenSelected,
        /// Clear the selection, close a menu, or go back a screen —
        /// whichever applies. There is no parent screen above the inbox
        /// and the panel hides itself on blur, so at the top level this
        /// only clears the selection.
        Cancel,
        /// Back to the inbox.
        Back,
        /// Ask the poller for a cycle now.
        Refresh,
        OpenSettings,
        /// Sign out of hi5: stop polling and stop using the GitHub CLI's
        /// credential until signed back in. The CLI itself is untouched.
        SignOut,
        /// Show every repo again.
        ClearRepoFocus,
        /// Open the current pull request on github.com.
        OpenExternal,
        /// Mute the current pull request until its head commit changes.
        Skip,
        /// Post a public approving review. The one irreversible thing
        /// hi5 does; see `Hi5::approve` for the guards around it.
        Approve,
        /// Quit. The only way out — hi5 has no Dock icon and no app
        /// menu.
        Quit,
    ]
);

/// Which slice of the queue to show.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize, schemars::JsonSchema, Action)]
#[action(namespace = hi5)]
pub struct SetScope {
    pub for_you: bool,
}

/// Approve every visible pull request of one repository — after a
/// confirmation that lists them. See `Hi5::approve_all`.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize, schemars::JsonSchema, Action)]
#[action(namespace = hi5)]
pub struct ApproveAll {
    pub repo: String,
}

/// Toggle one repo in the session's focus filter.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize, schemars::JsonSchema, Action)]
#[action(namespace = hi5)]
pub struct ToggleRepoFocus {
    pub repo: String,
}

/// Mute or unmute one repo, persistently.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize, schemars::JsonSchema, Action)]
#[action(namespace = hi5)]
pub struct ToggleRepoMute {
    pub repo: String,
}

/// Watch or unwatch one organization.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize, schemars::JsonSchema, Action)]
#[action(namespace = hi5)]
pub struct ToggleOrg {
    pub org: String,
}

/// dark / light / system.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize, schemars::JsonSchema, Action)]
#[action(namespace = hi5)]
pub struct SetAppearance {
    /// `"system"`, `"light"` or `"dark"` — the same spelling
    /// settings.json uses, so the two can't disagree.
    pub mode: String,
}

/// Seconds between poll cycles.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize, schemars::JsonSchema, Action)]
#[action(namespace = hi5)]
pub struct SetPollInterval {
    pub secs: u64,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, schemars::JsonSchema, Action)]
#[action(namespace = hi5)]
pub struct ToggleRule {
    /// `"hide_already_reviewed"`, `"hide_drafts"`, `"notifications"` or
    /// `"launch_at_login"`.
    pub which: String,
}

/// The panel's keymap.
///
/// One context, `!Dialog && !Input`: the panel is one window with one
/// focus target, so there is nothing else to scope against — except a
/// dialog, and a text field. gpui-component's confirmation (the
/// Approve-all one) binds Enter and Escape in its own `Dialog` context
/// and its inputs bind Enter, the arrows and more in `Input`; gpui ranks
/// a context-less binding as deepest of all and, on a tie, the one
/// bound last — which is this keymap, bound after
/// `gpui_component::init`. Left unscoped, Escape in the dialog reached
/// `Hi5::cancel` and the dialog stayed; Enter would have opened the
/// selected row underneath it, and Enter in the gh-path field would
/// have done the same instead of committing the field. What each
/// action *does* still depends on the screen — see `Hi5`'s handlers —
/// but which key fires it does not.
pub fn bind(cx: &mut gpui::App) {
    use gpui::KeyBinding;
    const CONTEXT: Option<&str> = Some("!Dialog && !Input");
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, CONTEXT),
        KeyBinding::new("up", SelectPrev, CONTEXT),
        KeyBinding::new("enter", OpenSelected, CONTEXT),
        KeyBinding::new("escape", Cancel, CONTEXT),
        KeyBinding::new("cmd-enter", Approve, CONTEXT),
        KeyBinding::new("cmd-o", OpenExternal, CONTEXT),
        KeyBinding::new("cmd-r", Refresh, CONTEXT),
        KeyBinding::new("cmd-,", OpenSettings, CONTEXT),
        KeyBinding::new("cmd-q", Quit, CONTEXT),
    ]);
}
