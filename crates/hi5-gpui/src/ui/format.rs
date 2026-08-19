//! Text that has to be right regardless of how it is drawn.
//!
//! Ages, error sentences and truncation are logic, not layout: they have
//! edge cases, they have tests, and they survived the UI being rebuilt
//! twice. Everything else that used to live beside them — a palette, row
//! geometry, a hand-built menu vocabulary — is gone, replaced by stock
//! components.

/// Shorten to a character budget, with a real ellipsis.
///
/// gpui's own `truncate()` needs a *definite* width to ellipsize
/// against (`elements/text.rs:357`), and the only place that width can
/// come from is a fixed `w()` — which then reserves the full column even
/// for a short string, leaving a hole in the middle of an inline run of
/// facts. For one line at a known size, counting characters is both
/// exact enough and testable, which a layout quirk is not.
pub fn ellipsize(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    // One character of the budget belongs to the ellipsis itself, or the
    // result is longer than what was asked for.
    let keep = max_chars.saturating_sub(1);
    s.chars().take(keep).collect::<String>() + "…"
}

/// Compact relative age: `12m`, `4h`, `9d`, `6mo`, `3y`.
///
/// Rolls up past days deliberately. An old PR rendering as `1423d` is
/// technically correct and unreadable; recent ones — the common case —
/// are untouched.
pub fn relative_age(iso: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let Ok(created) = chrono::DateTime::parse_from_rfc3339(iso) else {
        return String::new();
    };
    let mins = (now - created.with_timezone(&chrono::Utc)).num_minutes();
    if mins < 60 {
        return format!("{}m", mins.max(1));
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = hours / 24;
    if days < 30 {
        return format!("{days}d");
    }
    let months = days / 30;
    if months < 12 {
        return format!("{months}mo");
    }
    // Floored and clamped to 1: a PR at 360 days is past the months
    // branch already and should read "1y", not "0y".
    format!("{}y", (days / 365).max(1))
}

const SHOWING_LAST: &str = "showing your last results";

/// Turn a backend poll failure into something a person can act on.
///
/// `PollEvent::PollError` carries `AppError::to_string()` verbatim —
/// strings like `github api error: http 503`. That is a stack trace
/// wearing a banner: it tells a user nothing and makes hi5 look broken
/// when GitHub is the one having a bad night. The raw text is never
/// discarded; Settings ▸ Connection still prints it in full, because an
/// inbox that silently emptied itself was a real shipped bug and
/// softening the wording must not soften the signal.
pub fn humanize_poll_error(raw: &str) -> String {
    let detail = raw.trim();
    let lower = detail.to_lowercase();

    if let Some(rest) = lower.strip_prefix("github api error: http ") {
        if let Ok(code) = rest
            .trim()
            .chars()
            .take(3)
            .collect::<String>()
            .parse::<u16>()
        {
            return match code {
                500..=599 => format!("GitHub is having trouble — {SHOWING_LAST}"),
                429 => format!("GitHub is throttling hi5 — {SHOWING_LAST}"),
                401 | 403 => format!("GitHub refused that request — {SHOWING_LAST}"),
                _ => format!("GitHub couldn't answer that request — {SHOWING_LAST}"),
            };
        }
    }
    if lower.starts_with("github api error: forbidden") {
        return format!("GitHub refused that request — {SHOWING_LAST}");
    }
    // GraphQL-level errors and parse failures: hi5 reached GitHub,
    // GitHub answered, the answer was not usable.
    if lower.starts_with("github api error:") {
        return format!("GitHub sent something hi5 couldn't read — {SHOWING_LAST}");
    }
    if lower.starts_with("authentication failed") || lower.contains("token is invalid") {
        return format!("GitHub wouldn't accept your credentials — {SHOWING_LAST}");
    }
    // Transport errors — offline, DNS, TLS, timeout.
    const TRANSPORT: [&str; 6] = [
        "error sending request",
        "dns",
        "connection",
        "timed out",
        "timeout",
        "os error",
    ];
    if TRANSPORT.iter().any(|n| lower.contains(n)) {
        return format!("Can't reach GitHub right now — {SHOWING_LAST}");
    }
    // Anything already written as a sentence for a human — the poller's
    // response-shape anomaly is the live example — says itself better
    // than a generic rewrite would.
    if detail.starts_with(|c: char| c.is_uppercase()) {
        return detail.to_string();
    }
    format!("Couldn't refresh from GitHub — {SHOWING_LAST}")
}

/// The colours the six-hue avatar palette assigns from a login.
///
/// White initials clear 4.5:1 on every one of these — that is why the
/// palette is these six values and not a prettier set. gpui-component's
/// `Avatar` tints its own placeholder at 20% opacity and draws the
/// initials in the same hue, which is a soft, decorative treatment; in a
/// dense queue it left two-letter initials barely legible against their
/// own background.
const AVATAR_COLORS: [u32; 6] = [0xdc2727, 0x1074c7, 0x7551e0, 0x178161, 0xaf5c0e, 0xd32e68];

pub fn avatar_color(login: &str) -> gpui::Hsla {
    let mut h: i32 = 0;
    for c in login.chars() {
        h = h.wrapping_mul(31).wrapping_add(c as i32);
    }
    gpui::rgb(AVATAR_COLORS[(h.unsigned_abs() as usize) % AVATAR_COLORS.len()]).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_string_inside_the_budget_is_untouched() {
        assert_eq!(ellipsize("acme-labs/atlas", 26), "acme-labs/atlas");
        assert_eq!(ellipsize("", 3), "");
    }

    #[test]
    fn an_overlong_string_ends_in_an_ellipsis_and_fits_the_budget() {
        // The bug this replaces: gpui clipped the repo mid-word with no
        // ellipsis at all, so "rusty-ferris-club/rust-starter" read as
        // the real name "rusty-ferris-club/rust-starte".
        let out = ellipsize("rusty-ferris-club/rust-starter", 26);
        assert_eq!(out, "rusty-ferris-club/rust-st…");
        assert_eq!(out.chars().count(), 26);
    }

    #[test]
    fn it_counts_characters_rather_than_bytes() {
        // Repo names are ASCII in practice, but slicing by byte would
        // panic on the first one that is not.
        assert_eq!(ellipsize("ααααα", 3), "αα…");
    }

    #[test]
    fn a_degenerate_budget_does_not_panic() {
        assert_eq!(ellipsize("abcdef", 1), "…");
        assert_eq!(ellipsize("abcdef", 0), "…");
    }

    // A fixed anchor rather than "now", so every case is deterministic.
    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-08-17T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    fn ago(minutes: i64) -> String {
        (now() - chrono::Duration::minutes(minutes)).to_rfc3339()
    }

    const HOUR: i64 = 60;
    const DAY: i64 = 24 * HOUR;

    #[test]
    fn floors_sub_minute_ages_up_to_one_rather_than_showing_zero() {
        assert_eq!(relative_age(&ago(0), now()), "1m");
    }

    #[test]
    fn renders_minutes_under_an_hour() {
        assert_eq!(relative_age(&ago(45), now()), "45m");
        assert_eq!(relative_age(&ago(59), now()), "59m");
    }

    #[test]
    fn renders_hours_under_a_day() {
        assert_eq!(relative_age(&ago(HOUR), now()), "1h");
        assert_eq!(relative_age(&ago(23 * HOUR), now()), "23h");
    }

    #[test]
    fn renders_days_under_a_month() {
        assert_eq!(relative_age(&ago(DAY), now()), "1d");
        assert_eq!(relative_age(&ago(9 * DAY), now()), "9d");
        assert_eq!(relative_age(&ago(29 * DAY), now()), "29d");
    }

    #[test]
    fn rolls_up_to_months_between_thirty_days_and_roughly_a_year() {
        assert_eq!(relative_age(&ago(30 * DAY), now()), "1mo");
        assert_eq!(relative_age(&ago(90 * DAY), now()), "3mo");
        assert_eq!(relative_age(&ago(200 * DAY), now()), "6mo");
    }

    #[test]
    fn rolls_up_to_years_rather_than_showing_a_raw_day_count() {
        // The motivating regression: a 2022 PR rendering as `1423d`.
        assert_eq!(relative_age(&ago(1423 * DAY), now()), "3y");
    }

    #[test]
    fn never_regresses_to_zero_years_just_under_a_year() {
        assert_eq!(relative_age(&ago(360 * DAY), now()), "1y");
        assert_eq!(relative_age(&ago(364 * DAY), now()), "1y");
    }

    #[test]
    fn handles_multi_year_pull_requests() {
        assert_eq!(relative_age(&ago(5 * 365 * DAY), now()), "5y");
    }

    #[test]
    fn an_unparseable_timestamp_renders_nothing_rather_than_a_wrong_age() {
        assert_eq!(relative_age("not a date", now()), "");
    }

    #[test]
    fn turns_a_5xx_stack_trace_into_a_sentence_about_github_not_hi5() {
        let msg = humanize_poll_error("github api error: http 503");
        assert_eq!(msg, "GitHub is having trouble — showing your last results");
        assert!(!msg.contains("http"));
        assert!(!msg.contains("error:"));
    }

    #[test]
    fn distinguishes_throttling_refusal_and_unreadable_answers() {
        assert!(humanize_poll_error("github api error: http 429").contains("throttling"));
        assert!(humanize_poll_error("github api error: http 403").contains("refused"));
        assert!(humanize_poll_error("github api error: http 401").contains("refused"));
        assert!(humanize_poll_error("github api error: expected data").contains("couldn't read"));
    }

    #[test]
    fn names_an_unreachable_network_as_unreachable() {
        let msg =
            humanize_poll_error("error sending request for url (https://api.github.com/graphql)");
        assert!(msg.contains("Can't reach GitHub"), "{msg}");
    }

    #[test]
    fn passes_an_already_human_anomaly_through_verbatim() {
        // The poller's response-shape warning is written for a person
        // already; rewriting it would lose the only detail it carries.
        let anomaly = "GitHub returned 30 PRs but none could be parsed — the API response shape may have changed";
        assert_eq!(humanize_poll_error(anomaly), anomaly);
    }

    #[test]
    fn falls_back_to_a_plain_sentence_for_anything_unrecognised() {
        assert_eq!(
            humanize_poll_error("kaboom"),
            "Couldn't refresh from GitHub — showing your last results"
        );
    }
}
