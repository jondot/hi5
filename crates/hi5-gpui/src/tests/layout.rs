//! Geometry the eye checks and gets wrong. Measured from gpui's own
//! layout instead.

use gpui::{px, TestAppContext};

use crate::platform::panel::PANEL_WIDTH;
use crate::testing::Harness;
use crate::ui::inbox::{HEADER_HEIGHT, ROW_HEIGHT};

/// A list control's rows tile the list: every row is the full width of
/// the list and starts exactly where the one above ends. That is what
/// makes a row's bottom border read as *the* separator — one hairline,
/// edge to edge, at the boundary — rather than a decoration inset into
/// the row above.
#[gpui::test]
fn rows_tile_the_full_width_of_the_list(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    let rows = h.bounds("inbox.row");
    assert!(
        rows.len() >= 4,
        "expected the fixture rows, got {}",
        rows.len()
    );

    for (i, row) in rows.iter().enumerate() {
        assert_eq!(
            row.origin.x,
            px(0.),
            "row {i} does not start at the left edge"
        );
        assert_eq!(
            row.size.width,
            px(PANEL_WIDTH as f32),
            "row {i} is not the full width of the panel"
        );
    }
    // Rows within one section abut. (Between sections a header
    // intervenes, so only consecutive neighbours are compared: a gap of
    // exactly a header is expected there.)
    let mut abutting = 0;
    for w in rows.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b.origin.y == a.origin.y + a.size.height {
            abutting += 1;
        }
    }
    assert!(
        abutting >= 3,
        "rows in a section should be contiguous; only {abutting} pairs abut"
    );
}

/// The row's own bottom edge carries the rule, so the rule is exactly
/// as wide as the row: `inbox.rule` is the hairline, `inbox.row` its
/// row. And *every* row carries one — the last of a section included,
/// so a block of rows is closed before the next header rather than
/// trailing off into it.
#[gpui::test]
fn every_row_carries_a_rule_that_runs_edge_to_edge(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    let rows = h.bounds("inbox.row");
    let rules = h.bounds("inbox.rule");
    assert!(!rules.is_empty(), "no rules were drawn");
    assert_eq!(
        rules.len(),
        rows.len(),
        "one rule per row, the last row of each section included"
    );
    for rule in &rules {
        let row = rows
            .iter()
            .find(|r| r.origin.y <= rule.origin.y && rule.origin.y < r.origin.y + r.size.height)
            .unwrap_or_else(|| panic!("rule at {:?} is not inside any row", rule.origin));
        assert_eq!(rule.origin.x, row.origin.x, "rule is inset on the left");
        assert_eq!(
            rule.size.width, row.size.width,
            "rule is narrower than its row"
        );
        assert_eq!(
            rule.origin.y + rule.size.height,
            row.origin.y + row.size.height,
            "rule does not sit on the row's bottom edge"
        );
    }
}

/// The back button is where the header says it is, and the click that
/// lands there is the one that navigates. (`header[0]` is the button —
/// see `ui::screen_header`.)
#[gpui::test]
fn the_back_button_is_the_first_thing_in_the_header(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    h.click("inbox.row", 0);
    let header = h.bounds("header");
    assert!(header.len() >= 2, "header should hold ‹ and a title");
    let back = header[0];
    assert!(back.origin.x < px(16.), "‹ should sit at the leading edge");
    assert!(back.size.width >= px(20.) && back.size.height >= px(20.));
}

/// The section header at the top of the viewport is pinned there while
/// its rows scroll under it, and pushed off by the next section's header
/// as that arrives — the way a grouped table view behaves.
///
/// The overlay is placed by arithmetic over `ROW_HEIGHT` and
/// `HEADER_HEIGHT` (see `InboxDelegate::pinned`); this holds that
/// arithmetic to the positions the list actually lays the flow headers
/// out at, so the two cannot drift apart by a pixel.
#[gpui::test]
fn the_section_header_pins_and_is_pushed_by_the_next(cx: &mut TestAppContext) {
    let mut h = Harness::with_long_queue(cx);

    // At rest the overlay sits exactly over the first header: same box.
    let flow = h.bounds("inbox.header");
    let pinned = h.bounds("inbox.pinned");
    assert_eq!(pinned.len(), 1, "one pinned header");
    assert_eq!(
        pinned[0], flow[0],
        "at rest the overlay coincides with the first header"
    );
    let list_top = pinned[0].origin.y;
    assert_eq!(pinned[0].size.height, px(HEADER_HEIGHT));

    // The first section is four rows: its header, then 4 × ROW_HEIGHT.
    // Scroll a hundred points into it — the flow has gone up by a
    // hundred, the pinned header has not moved.
    h.scroll("inbox.row", 1, px(100.));
    let flow = h.bounds("inbox.header");
    let pinned = h.bounds("inbox.pinned");
    // The first header is off the top and no longer drawn; the second
    // (one header and four rows down) has come up by a hundred.
    let second = px(HEADER_HEIGHT) + px(ROW_HEIGHT) * 4.;
    assert_eq!(
        flow[0].origin.y,
        list_top + second - px(100.),
        "the flow scrolled"
    );
    assert_eq!(pinned[0].origin.y, list_top, "the pinned header held");

    // Scroll to within 14 points of the second header: the pinned header
    // is pushed up by exactly that.
    let target = second - px(HEADER_HEIGHT) + px(14.);
    h.scroll("inbox.row", 1, target - px(100.));
    let flow = h.bounds("inbox.header");
    let pinned = h.bounds("inbox.pinned");
    let arriving = flow
        .iter()
        .find(|b| b.origin.y > list_top)
        .expect("the second header is in view");
    assert_eq!(arriving.origin.y, list_top + px(HEADER_HEIGHT) - px(14.));
    assert_eq!(
        pinned[0].origin.y,
        list_top - px(14.),
        "the pinned header is pushed by the arriving one"
    );
    assert_eq!(
        pinned[0].origin.y + pinned[0].size.height,
        arriving.origin.y,
        "pushed exactly to the arriving header's top edge"
    );

    // Past it: the second section's header is now the pinned one, back
    // at the top, and standing in for a header that is off-screen —
    // every header still drawn in the flow is below it.
    h.scroll("inbox.row", 1, px(60.));
    let flow = h.bounds("inbox.header");
    let pinned = h.bounds("inbox.pinned");
    assert_eq!(pinned[0].origin.y, list_top, "the next header took over");
    assert!(
        flow.iter()
            .all(|b| b.origin.y > list_top + px(HEADER_HEIGHT)),
        "no flow header is at the top; the overlay stands in: {flow:?}"
    );
}

/// The bar at the top is one height on every screen: opening a row must
/// not step it down. Both are `py_1p5` around a default-size icon
/// button; this holds them to the same number.
#[gpui::test]
fn the_screen_header_is_as_tall_as_the_toolbar(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    let toolbar = h.bounds("inbox.toolbar")[0];
    h.click("inbox.row", 0);
    let header = h.bounds("screen.header")[0];
    assert_eq!(header.size.height, toolbar.size.height);
    assert_eq!(header.origin, toolbar.origin);
}

/// Before the first cycle lands the empty inbox is a spinner, not the
/// words "nothing waiting on you" — those are a claim about GitHub, and
/// nothing has been asked yet. Once a cycle answers, even with nothing,
/// the words are right.
#[gpui::test]
fn an_unfetched_inbox_shows_a_spinner_not_the_empty_state(cx: &mut TestAppContext) {
    let mut h = Harness::new(cx);
    assert_eq!(
        h.bounds("inbox.loading").len(),
        1,
        "spinner before any poll"
    );
    h.receive(Vec::new());
    assert!(
        h.bounds("inbox.loading").is_empty(),
        "the answer was: nothing"
    );
}
