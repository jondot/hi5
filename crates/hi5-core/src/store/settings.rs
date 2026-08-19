use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

/// hi5's inbox is now built from two GitHub search queries (see
/// `query::build`), not a set of independently toggleable rules: the
/// "anyone can review" query (org-scoped, `review:none`) and the
/// "asked for you" query (`review-requested:@me`) are both always run --
/// the entire point of the product is a shared queue that also
/// highlights direct asks, not an opt-in list of personal filters. Only
/// the two modifiers that make sense applied *after* either query
/// remain here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Rules {
    /// Applied to the "asked for you" query only -- the "anyone can
    /// review" query already implies `review:none`, so appending
    /// `-reviewed-by:@me` to it would be a redundant no-op.
    pub hide_already_reviewed: bool,
    /// Applied to both queries.
    pub hide_drafts: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            hide_already_reviewed: true,
            hide_drafts: true,
        }
    }
}

/// `muted` is the only repo-level concept left: subtractive, applied
/// client-side after fetching, over every query's results. The old
/// `all_open_repos` additive list and the rule it fed
/// (`all_open_in_selected_repos`) were retired -- nothing in the app
/// could ever populate that list, and org-scoping (`Settings::watched_orgs`) now covers the same need
/// properly.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RepoConfig {
    pub muted: BTreeSet<String>,
}

/// "Branches we care about", per `inbox::resolve_watched_branches`.
///
/// A PR merging one feature branch into another, where neither is
/// protected, gates nothing -- it's noise in a shared review queue (the
/// alma repo example this exists for: 22 of 23 open PRs target `main`,
/// one targets a co-worker's feature branch). Detection alone can't be
/// trusted: `GET /repos/{owner}/{repo}/branches?protected=true` returns
/// an empty array both when a repo genuinely has no protected branches
/// and when the token lacks permission to see them -- verified live,
/// there is no way to tell the two apart (`rusty-ferris-club/rust-starter`
/// vs. a from-scratch repo would look identical). `global` is the
/// user-editable fallback for exactly that ambiguity, and `per_repo` is
/// the outright override for a repo detection gets wrong.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BranchWatch {
    /// Fallback branch names, used when a repo has no override and its
    /// protected branches can't be detected (empty result or a failed
    /// lookup). Never empty by default -- an empty list here is a
    /// deliberate user choice that falls through further, to the repo's
    /// own default branch (see `resolve_watched_branches`).
    pub global: Vec<String>,
    /// repo (`nameWithOwner`) -> explicit branch list, winning outright
    /// over detection. The escape hatch for the case detection can never
    /// resolve: a repo whose protection status genuinely can't be told
    /// apart from "not protected at all".
    pub per_repo: HashMap<String, Vec<String>>,
}

impl Default for BranchWatch {
    fn default() -> Self {
        Self {
            global: vec!["main".into(), "master".into(), "develop".into()],
            per_repo: HashMap::new(),
        }
    }
}

/// Which appearance the popover renders in.
///
/// `System` is the default and means "don't decide" -- the webview
/// follows the macOS appearance live via `prefers-color-scheme`, with no
/// JavaScript involved, and the native window chrome follows through
/// Tauri's own theme API. The two explicit values exist because a
/// menu-bar utility is often used against one fixed backdrop (a dark
/// editor, a light browser) where following the system is the wrong
/// answer.
///
/// Serialized lowercase (`"system"`, `"dark"`, `"light"`) so the value
/// in settings.json reads the same as the CSS `data-theme` attribute it
/// drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

/// How the panel was last left: which scope segment was showing, and
/// which repos the queue was focused on.
///
/// This started out deliberately session-only -- "sit on one repo for an
/// hour" is a posture, not a preference -- and that reasoning was wrong
/// in one specific way. hi5 is a menu-bar utility with no window to
/// leave open: it is quit and relaunched by a logout, a reboot, an
/// update, or a crash, none of which are the user deciding they are done
/// focusing. Dropping the focus on every one of those turns a posture
/// into something you re-set several times a day, and -- worse -- it
/// does so silently, which is the same failure mode as the empty inbox
/// with no explanation.
///
/// Kept as its own struct rather than two loose fields so it reads as
/// what it is in settings.json: restored view state, not configuration.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Session {
    /// True when the "For you" segment was active.
    pub for_you: bool,
    /// Repos the queue was focused on. Empty means every repo, which is
    /// also what a settings file written before this field existed
    /// loads as.
    pub focus_repos: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub rules: Rules,
    pub repos: RepoConfig,
    pub branch_watch: BranchWatch,
    /// Organization logins (plus, conventionally, the viewer's own
    /// personal login) that the "anyone can review" query is scoped to
    /// -- one `org:` search per entry. Never used unscoped: an empty
    /// list means zero "anyone can review" queries run, rather than one
    /// unscoped query matching a large fraction of GitHub. Auto-populated
    /// from `user/orgs` on the first successful poll once empty; the user
    /// can toggle individual entries off afterwards in Settings.
    pub watched_orgs: Vec<String>,
    /// Whether org discovery (`poller::discover_watched_orgs`) has ever
    /// been run to completion. An empty `watched_orgs` is genuinely
    /// ambiguous on its own -- it means both "never discovered yet" and
    /// "the user unwatched every org on purpose" -- and driving discovery
    /// off `watched_orgs.is_empty()` conflated the two: unwatching
    /// everything silently re-triggered discovery on the very next poll
    /// and repopulated the list the user had just emptied. This flag
    /// disambiguates: discovery runs only while it's `false`, and is set
    /// `true` the moment a discovery attempt completes -- whatever it
    /// found, including nothing -- so an empty list afterwards is
    /// respected as a deliberate choice rather than re-run. Defaults to
    /// `false` via `#[serde(default)]`, so a settings file written before
    /// this field existed re-runs discovery exactly once on next load --
    /// the correct one-time migration, not a bug.
    pub orgs_discovered: bool,
    pub poll_interval_secs: u64,
    pub notifications_enabled: bool,
    pub hotkey: String,
    pub launch_at_login: bool,
    /// False until the user has passed the first-run screen once.
    /// Gates the welcome screen so an already-authenticated user does
    /// not see it again on every launch.
    pub has_completed_first_run: bool,
    /// Dark / light / system. Defaults to `System`, and an older
    /// settings.json with no `appearance` key at all loads as `System`
    /// too (`#[serde(default)]` on the struct) -- which is the same
    /// behaviour those users already had, since the app followed nothing
    /// and was simply always dark.
    pub appearance: Appearance,
    /// The user signed out of hi5. hi5 then neither polls nor uses the
    /// GitHub CLI's credential until they sign back in -- the CLI itself
    /// is left exactly as it was, because it is not hi5's to sign out.
    /// Defaults to `false`, so a settings file from before this field
    /// loads as signed in, which is what those users were.
    pub signed_out: bool,
    /// Where to run `gh`, when the user has said. `None` — the default,
    /// and what a settings file from before this field loads as — means
    /// find it: see `auth::runner::locate`. Set from Settings ▸ Connection
    /// when the automatic search comes up empty or picks the wrong one.
    pub gh_path: Option<String>,
    /// Restored view state -- see [`Session`].
    pub session: Session,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            rules: Rules::default(),
            repos: RepoConfig::default(),
            branch_watch: BranchWatch::default(),
            watched_orgs: Vec::new(),
            orgs_discovered: false,
            poll_interval_secs: 30,
            notifications_enabled: true,
            hotkey: "Alt+Cmd+A".into(),
            launch_at_login: true,
            has_completed_first_run: false,
            appearance: Appearance::default(),
            signed_out: false,
            gh_path: None,
            session: Session::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_zero_config_usable() {
        let s = Settings::default();
        assert!(s.rules.hide_already_reviewed);
        assert!(s.rules.hide_drafts);
        assert!(s.repos.muted.is_empty());
        assert!(s.watched_orgs.is_empty());
        assert!(!s.orgs_discovered);
        assert_eq!(s.poll_interval_secs, 30);
        assert_eq!(s.branch_watch.global, vec!["main", "master", "develop"]);
        assert!(s.branch_watch.per_repo.is_empty());
    }

    #[test]
    fn a_settings_file_from_before_branch_watch_existed_still_loads_with_the_default() {
        // Migration: an older settings.json has no `branchWatch` key at
        // all, and must default to the non-empty global list rather than
        // an empty one -- an empty list here would fall all the way
        // through to per-repo default-branch detection for every repo
        // with no override, which is a much bigger behavior change than
        // "add this field".
        let json = r#"{"pollIntervalSecs": 45}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.branch_watch.global, vec!["main", "master", "develop"]);
    }

    #[test]
    fn unknown_fields_are_ignored_and_missing_fields_default() {
        // Forward compatibility: an older binary reading a newer file,
        // and a newer binary reading an older file, must both survive.
        let json = r#"{"pollIntervalSecs": 45, "somethingFromTheFuture": true}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.poll_interval_secs, 45);
        assert_eq!(s.hotkey, "Alt+Cmd+A");
        assert!(s.rules.hide_already_reviewed);
        assert!(s.watched_orgs.is_empty());
        // The migration this field exists for: an older settings file has
        // no `orgsDiscovered` key at all, and must default to `false` so
        // discovery runs exactly once more rather than silently never
        // running for accounts that already went through the old
        // `watched_orgs.is_empty()` path.
        assert!(!s.orgs_discovered);
    }

    #[test]
    fn appearance_defaults_to_system_and_round_trips_lowercase() {
        // The wire value is what the frontend writes into `data-theme`
        // and hands to Tauri's window theme API, so the exact spelling
        // is load-bearing, not cosmetic.
        let s = Settings::default();
        assert_eq!(s.appearance, Appearance::System);
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""appearance":"system""#), "{json}");
        let back: Settings = serde_json::from_str(r#"{"appearance":"light"}"#).unwrap();
        assert_eq!(back.appearance, Appearance::Light);
    }

    #[test]
    fn a_settings_file_from_before_appearance_existed_loads_as_system() {
        // Migration: the app was dark-only before this field, so the
        // absence of the key must mean "follow the system" rather than
        // failing to parse.
        let s: Settings = serde_json::from_str(r#"{"pollIntervalSecs": 45}"#).unwrap();
        assert_eq!(s.appearance, Appearance::System);
    }

    #[test]
    fn a_settings_file_from_before_the_session_existed_loads_as_no_focus() {
        // Migration: the app kept scope and repo focus in memory only
        // before this field, so its absence must mean "All, every repo"
        // -- the state those users already relaunched into -- rather
        // than failing to parse or restoring a focus nobody set.
        let s: Settings = serde_json::from_str(r#"{"pollIntervalSecs": 45}"#).unwrap();
        assert!(!s.session.for_you);
        assert!(s.session.focus_repos.is_empty());
    }

    #[test]
    fn the_session_round_trips_through_settings_json() {
        let s = Settings {
            session: Session {
                for_you: true,
                focus_repos: vec!["acme-labs/atlas".into()],
            },
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""session":{"forYou":true"#), "{json}");
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.session, s.session);
    }
}
