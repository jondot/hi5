use gpui::TestAppContext;

use crate::app::Screen;
use crate::testing::Harness;

#[gpui::test]
fn clicking_a_row_opens_its_pull_request(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    h.click("inbox.row", 0);
    assert!(matches!(h.screen(), Screen::Detail(_)));
}

/// The bug as reported: open a pull request with the mouse, click ‹, and
/// nothing happens. Also holds for Escape — the same dispatch path.
#[gpui::test]
fn back_returns_to_the_inbox_after_a_mouse_click(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    h.click("inbox.row", 0);
    assert!(matches!(h.screen(), Screen::Detail(_)));

    h.click("header", 0);
    assert!(
        matches!(h.screen(), Screen::Inbox),
        "‹ did nothing after a mouse-opened detail"
    );
}

#[gpui::test]
fn escape_leaves_detail_after_a_mouse_click(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    h.click("inbox.row", 0);
    h.keys("escape");
    assert!(
        matches!(h.screen(), Screen::Inbox),
        "Escape did nothing after a mouse-opened detail"
    );
}

#[gpui::test]
fn keyboard_navigation_opens_and_leaves_detail(cx: &mut TestAppContext) {
    let mut h = Harness::with_queue(cx);
    h.keys("down enter");
    assert!(matches!(h.screen(), Screen::Detail(_)));
    h.keys("escape");
    assert!(matches!(h.screen(), Screen::Inbox));
}
