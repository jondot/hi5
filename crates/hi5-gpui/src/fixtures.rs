//! A queue that exercises the layout, held still.
//!
//! Shared by the `preview` binary and the headless UI tests, so what the
//! tests assert about is what the screenshots show.
//!
//! Chosen rather than sampled: every case the row has to survive appears
//! here — a title that must ellipsize, one that must not, a review
//! request, each CI state, a four-digit PR number, a diff in the
//! thousands, and two repositories so a section boundary is in shot.
//! Real data drifts, and a screenshot of drifting data cannot be
//! compared with last week's.

use hi5_core::github::{Author, CheckState, Label, PullRequest};

/// A pull request, from the handful of facts a row actually renders.
/// `diff` is `(additions, deletions)` — one fact, not two.
fn pr(
    number: u64,
    repo: &str,
    login: &str,
    title: &str,
    age_days: i64,
    diff: (u32, u32),
    checks: CheckState,
) -> PullRequest {
    let (additions, deletions) = diff;
    PullRequest {
        id: format!("{repo}#{number}"),
        number,
        title: title.into(),
        body: String::new(),
        url: format!("https://github.com/{repo}/pull/{number}"),
        repo: repo.into(),
        author: Author {
            login: login.into(),
            avatar_url: String::new(),
        },
        // A fixed clock: "3d" has to keep saying 3d, or every screenshot
        // differs from the last one for no reason anybody can act on.
        created_at: (chrono::DateTime::parse_from_rfc3339("2026-08-18T12:00:00Z")
            .expect("a literal timestamp")
            - chrono::Duration::days(age_days))
        .to_rfc3339(),
        additions,
        deletions,
        changed_files: 1 + (additions / 40),
        labels: Vec::new(),
        head_sha: "0123456789abcdef".into(),
        checks,
        is_draft: false,
        base_ref_name: "main".into(),
        default_branch: "main".into(),
        asked_for_you: false,
    }
}

pub fn pull_requests() -> Vec<PullRequest> {
    let mut prs = vec![
        pr(
            1184,
            "acme-labs/atlas",
            "dependabot",
            "chore(deps): bump node from 24.18.0-alpine to 24.19.0-alpine in /apps/atlas",
            15,
            (1, 1),
            CheckState::Failure,
        ),
        pr(
            920,
            "acme-labs/atlas",
            "mira",
            "Retry the export queue, and pin the stalled jobs to the top",
            7,
            (311, 84),
            CheckState::Success,
        ),
        pr(
            88,
            "acme-labs/atlas",
            "theo",
            "Fix typo",
            2,
            (1, 1),
            CheckState::None,
        ),
        pr(
            1212,
            "acme-labs/atlas",
            "avery",
            "Share one session store between all collaborators",
            1,
            (2451, 190),
            CheckState::Pending,
        ),
        pr(
            134,
            "rusty-ferris-club/rust-starter",
            "kaplanelad",
            "Add .rustfmt.toml file",
            120,
            (26, 16),
            CheckState::Success,
        ),
        pr(
            2,
            "rusty-ferris-club/rust-starter",
            "jondot",
            "setup clap with ArgRequiredElseHelp",
            400,
            (8, 13),
            CheckState::None,
        ),
    ];

    // The row that carries a badge, and the one the detail view shows.
    prs[4].asked_for_you = true;
    prs[4].body = "Adds a `rustfmt.toml` so the formatting is the same on \
                   every machine.\n\n- `max_width = 100`\n- `imports_granularity = \
                   \"Crate\"`\n\nLet me know what you think about this \
                   configuration; you can close the PR if you think it's not \
                   necessary."
        .into();
    prs[4].labels = vec![
        Label {
            name: "enhancement".into(),
            color: "a2eeef".into(),
        },
        Label {
            name: "good first issue".into(),
            color: "7057ff".into(),
        },
    ];
    prs
}

/// The same queue, plus a third repository with enough pull requests
/// (ten) that the list scrolls well past its first two sections — the
/// fixture for anything about scrolling: the pinned section header,
/// the push as the next one arrives, and the rows going under it.
pub fn long_queue() -> Vec<PullRequest> {
    let mut prs = pull_requests();
    let extra = [
        (
            412,
            "hi5-app/hi5",
            "jondot",
            "Sticky section headers in the inbox",
            0,
            (188, 40),
            CheckState::Success,
        ),
        (
            409,
            "hi5-app/hi5",
            "kaplanelad",
            "Tabbed settings: general and repositories",
            1,
            (610, 302),
            CheckState::Pending,
        ),
        (
            401,
            "hi5-app/hi5",
            "mira",
            "Spinner while a refresh is in flight",
            2,
            (22, 4),
            CheckState::Success,
        ),
        (
            398,
            "hi5-app/hi5",
            "theo",
            "Approve gets the same depth as its neighbours",
            3,
            (6, 2),
            CheckState::None,
        ),
        (
            390,
            "hi5-app/hi5",
            "avery",
            "Full-width rules under every row",
            4,
            (14, 9),
            CheckState::Failure,
        ),
        (
            377,
            "hi5-app/hi5",
            "dependabot",
            "chore(deps): bump tokio from 1.47.0 to 1.48.0",
            6,
            (2, 2),
            CheckState::Success,
        ),
        (
            371,
            "hi5-app/hi5",
            "mira",
            "Inset-grouped settings cards",
            8,
            (240, 197),
            CheckState::Success,
        ),
        (
            366,
            "hi5-app/hi5",
            "jondot",
            "System accent for switches",
            9,
            (18, 3),
            CheckState::Success,
        ),
        (
            352,
            "hi5-app/hi5",
            "theo",
            "Drop the separate repositories screen",
            11,
            (31, 92),
            CheckState::None,
        ),
        (
            340,
            "hi5-app/hi5",
            "kaplanelad",
            "Long queue fixture for scroll tests",
            12,
            (44, 0),
            CheckState::Pending,
        ),
    ];
    prs.extend(
        extra
            .into_iter()
            .map(|(n, repo, login, title, age, diff, checks)| {
                pr(n, repo, login, title, age, diff, checks)
            }),
    );
    prs
}

/// Organisations for the Settings screen: two candidates, one watched,
/// so both states of the row are in shot.
pub fn orgs() -> (Vec<String>, Vec<String>) {
    (
        vec!["acme-labs".into(), "rusty-ferris-club".into()],
        vec!["acme-labs".into()],
    )
}

/// The queue the README is photographed with.
///
/// Not the test queue above: that one is built to exercise the layout
/// (a title that must ellipsize, every CI state, a four-digit number)
/// and looks like it. This one is built to look like a Tuesday — one
/// organisation, three repositories, nine pull requests at the sizes
/// and ages real ones have, two of them asked of the reader. Every
/// name, repository and title here is invented.
///
/// Ages are relative to *now*, not the fixed clock, so a pull request
/// opened "3h" ago reads 3h whenever the README is re-shot.
pub fn showcase() -> Vec<PullRequest> {
    fn hours_ago(pr: &mut PullRequest, hours: i64) {
        pr.created_at = (chrono::Utc::now() - chrono::Duration::hours(hours)).to_rfc3339();
    }
    fn label(name: &str, color: &str) -> Label {
        Label {
            name: name.into(),
            color: color.into(),
        }
    }
    let rows = [
        (
            1258,
            "acme-labs/atlas",
            "mira",
            "Retry the export queue, and pin stalled jobs to the top",
            26,
            (311, 84),
            CheckState::Success,
        ),
        (
            1261,
            "acme-labs/atlas",
            "priya",
            "Stream export results instead of buffering the file",
            3,
            (96, 41),
            CheckState::Success,
        ),
        (
            1254,
            "acme-labs/atlas",
            "theo",
            "Rate-limit webhook deliveries per installation",
            49,
            (142, 37),
            CheckState::Success,
        ),
        (
            1249,
            "acme-labs/atlas",
            "dependabot",
            "chore(deps): bump tokio from 1.47.0 to 1.48.0",
            5 * 24,
            (2, 2),
            CheckState::Success,
        ),
        (
            877,
            "acme-labs/atlas-web",
            "avery",
            "Keyboard nav in the palette",
            6,
            (418, 96),
            CheckState::Pending,
        ),
        (
            871,
            "acme-labs/atlas-web",
            "jonas",
            "Empty state for a workspace with no projects yet",
            30,
            (64, 8),
            CheckState::Success,
        ),
        (
            869,
            "acme-labs/atlas-web",
            "priya",
            "Fix the flicker when switching themes",
            3 * 24,
            (9, 14),
            CheckState::Success,
        ),
        (
            233,
            "acme-labs/infra",
            "jonas",
            "Move the staging cluster to spot nodes",
            2 * 24,
            (210, 188),
            CheckState::Failure,
        ),
        (
            230,
            "acme-labs/infra",
            "theo",
            "Rotate the registry pull token",
            4 * 24,
            (3, 3),
            CheckState::Success,
        ),
    ];
    let mut prs: Vec<PullRequest> = rows
        .into_iter()
        .map(|(n, repo, login, title, hours, diff, checks)| {
            let mut p = pr(n, repo, login, title, 0, diff, checks);
            hours_ago(&mut p, hours);
            p
        })
        .collect();

    // The one the detail screen shows, and the first "for you".
    prs[0].asked_for_you = true;
    prs[0].body = "Export jobs that hit a transient S3 error were dropped on the \
                   floor and had to be re-run by hand.\n\n\
                   This retries them three times with backoff, and pins anything \
                   still stalled to the top of the queue so it is picked up first \
                   on the next tick.\n\n\
                   - `ExportQueue::retry` with jittered backoff (250ms → 2s)\n\
                   - stalled jobs carry a `pinned_at` and sort first\n\
                   - one metric per retry, so it shows up in the dashboard\n\n\
                   Tested against the staging bucket with fault injection; no \
                   change to the happy path."
        .into();
    prs[0].labels = vec![label("backend", "0e8a16"), label("area: export", "1d76db")];
    prs[0].changed_files = 6;
    prs[4].asked_for_you = true;
    prs[4].labels = vec![label("frontend", "d4c5f9")];
    prs
}
