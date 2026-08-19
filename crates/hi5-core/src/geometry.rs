//! Where the popover goes, as arithmetic.
//!
//! Pure geometry with no window handle in sight, so the placement rules
//! — centred under the menu-bar icon, clamped inside whichever monitor
//! it lands on — can be tested without a display attached. The shell
//! supplies the three rectangles and applies the answer.
//!
//! **Everything here is in logical points.** Mixing units is not a
//! hypothetical: the Tauri implementation converted the tray rect with
//! the window's scale factor, read the window size in physical pixels
//! of whichever monitor it was last shown on, and handed the result
//! back as a physical position that macOS divided by that scale again.
//! On a 1x ultrawide beside a 2x laptop the panel landed 1,400px from
//! the icon it was supposed to hang under.

/// Margin, in logical points, kept between the popover and the edge of
/// whatever monitor it's clamped to.
const EDGE_MARGIN: f64 = 8.0;

/// A rectangle in screen space, in logical points. Used for both the
/// tray icon's rect and a monitor's bounds — origins can be negative (a
/// monitor to the left of or above the primary display).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect2 {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A window size in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size2 {
    pub w: f64,
    pub h: f64,
}

/// Pure geometry, no Tauri handles: where should the popover's top-left
/// corner land, given the tray icon's rect, the popover's own size, and
/// (if known) the bounds of the monitor the icon is on?
///
/// Horizontally centred beneath the icon, clamped on both axes to stay
/// inside the monitor's frame (minus [`EDGE_MARGIN`]) so the panel never
/// runs off screen — including on a monitor with a negative-origin
/// coordinate space, or one smaller than the popover itself. With no
/// monitor info at all, returns the unclamped centred position as a
/// best-effort fallback; callers should prefer passing at least the
/// primary monitor's bounds rather than `None` when possible.
pub fn compute_position(icon: Rect2, win: Size2, monitor: Option<Rect2>) -> (f64, f64) {
    let mut x = icon.x + icon.w / 2.0 - win.w / 2.0;
    let mut y = icon.y + icon.h;

    if let Some(m) = monitor {
        let min_x = m.x + EDGE_MARGIN;
        let max_x = m.x + m.w - win.w - EDGE_MARGIN;
        x = x.clamp(min_x, max_x.max(min_x));

        let min_y = m.y + EDGE_MARGIN;
        let max_y = m.y + m.h - win.h - EDGE_MARGIN;
        y = y.clamp(min_y, max_y.max(min_y));
    }

    (x, y)
}

/// Which monitor a point falls on, or `None` if it falls on no monitor
/// at all. Split out of `position_under` as pure geometry so the
/// multi-monitor case is testable without a display attached.
///
/// All rects must be in the same units (see `position_under`: logical
/// points). Ties are impossible — monitor frames don't overlap — so the
/// first hit wins.
pub fn monitor_containing(point: (f64, f64), monitors: &[Rect2]) -> Option<Rect2> {
    monitors
        .iter()
        .copied()
        .find(|m| point.0 >= m.x && point.0 < m.x + m.w && point.1 >= m.y && point.1 < m.y + m.h)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIN: Size2 = Size2 { w: 392.0, h: 544.0 };
    const PRIMARY: Rect2 = Rect2 {
        x: 0.0,
        y: 0.0,
        w: 1920.0,
        h: 1080.0,
    };

    // Two monitors side by side in logical points: a 3440-wide 1x panel
    // at the origin and a 1470-wide 2x laptop to its right. These are the
    // real numbers from the setup that exposed the mixed-DPI bug.
    const WIDE: Rect2 = Rect2 {
        x: 0.0,
        y: 0.0,
        w: 3440.0,
        h: 1440.0,
    };
    const LAPTOP: Rect2 = Rect2 {
        x: 3440.0,
        y: 0.0,
        w: 1470.0,
        h: 956.0,
    };

    #[test]
    fn monitor_containing_picks_the_screen_the_point_is_on() {
        let all = [WIDE, LAPTOP];
        assert_eq!(monitor_containing((2737.0, 0.0), &all), Some(WIDE));
        assert_eq!(monitor_containing((3500.0, 100.0), &all), Some(LAPTOP));
    }

    #[test]
    fn monitor_containing_is_none_off_every_screen() {
        // Below the laptop's 956pt height but inside the wide monitor's
        // x-range would be a hit; this is past the right edge of both.
        assert_eq!(monitor_containing((9000.0, 0.0), &[WIDE, LAPTOP]), None);
        assert_eq!(monitor_containing((0.0, 0.0), &[]), None);
    }

    #[test]
    fn monitor_containing_treats_the_far_edge_as_the_next_screen() {
        // x == m.x + m.w belongs to the neighbour, not to this monitor --
        // otherwise a point on the seam matches two screens.
        assert_eq!(
            monitor_containing((3440.0, 0.0), &[WIDE, LAPTOP]),
            Some(LAPTOP)
        );
    }

    #[test]
    fn tray_on_the_wide_monitor_anchors_under_the_icon() {
        // The regression this replaces: with the icon at x=2737 (w=64) the
        // panel must land at 2737 + 32 - 196 = 2573, on the same monitor.
        // The buggy version mixed a 2x window size into 1x icon
        // coordinates and produced 1188.
        let icon = Rect2 {
            x: 2737.0,
            y: 0.0,
            w: 64.0,
            h: 24.0,
        };
        let (x, y) = compute_position(icon, WIN, Some(WIDE));
        assert_eq!((x, y), (2573.0, 24.0));
    }

    #[test]
    fn centered_under_icon_on_normal_screen_no_clamp() {
        // Icon comfortably in the middle of the menu bar: nothing should clamp.
        let icon = Rect2 {
            x: 900.0,
            y: 0.0,
            w: 24.0,
            h: 24.0,
        };
        let (x, y) = compute_position(icon, WIN, Some(PRIMARY));
        // centred: icon.x + icon.w/2 - win.w/2 = 900 + 12 - 196 = 716
        assert_eq!((x, y), (716.0, 24.0));
    }

    #[test]
    fn clamped_at_right_edge() {
        // Icon near the right edge of the monitor: centred position would
        // run the panel off the right, so it must clamp to the margin.
        let icon = Rect2 {
            x: 1900.0,
            y: 0.0,
            w: 24.0,
            h: 24.0,
        };
        let (x, y) = compute_position(icon, WIN, Some(PRIMARY));
        // max_x = 0 + 1920 - 392 - 8 = 1520
        assert_eq!(x, 1520.0);
        assert_eq!(y, 24.0);
    }

    #[test]
    fn clamped_at_left_edge() {
        // Icon flush against the left edge: centred position goes negative,
        // must clamp to the left margin instead.
        let icon = Rect2 {
            x: 0.0,
            y: 0.0,
            w: 24.0,
            h: 24.0,
        };
        let (x, _y) = compute_position(icon, WIN, Some(PRIMARY));
        // min_x = 0 + 8 = 8
        assert_eq!(x, 8.0);
    }

    #[test]
    fn second_monitor_with_negative_origin_is_not_clamped_to_primary() {
        // A monitor placed to the left of the primary display, e.g. in
        // System Settings' arrangement panel, has a negative x origin.
        // The result must be computed relative to *that* monitor's frame,
        // not accidentally clamped against a 0-based (primary) assumption.
        let monitor = Rect2 {
            x: -1920.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };
        let icon = Rect2 {
            x: -1000.0,
            y: 0.0,
            w: 24.0,
            h: 24.0,
        };
        let (x, y) = compute_position(icon, WIN, Some(monitor));
        // centred: -1000 + 12 - 196 = -1184, well within
        // [-1912, -400] for this monitor, so it should be unclamped.
        // If this were wrongly clamped against a 0-based primary monitor
        // (min_x = 8), x would come back positive instead.
        assert_eq!((x, y), (-1184.0, 24.0));
        assert!(
            x < 0.0,
            "expected the popover to stay on the negative-origin monitor"
        );
    }

    #[test]
    fn second_monitor_negative_origin_right_edge_still_clamps() {
        let monitor = Rect2 {
            x: -1920.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };
        let icon = Rect2 {
            x: -100.0,
            y: 0.0,
            w: 24.0,
            h: 24.0,
        };
        let (x, _y) = compute_position(icon, WIN, Some(monitor));
        // max_x = -1920 + 1920 - 392 - 8 = -400
        assert_eq!(x, -400.0);
    }

    #[test]
    fn monitor_narrower_than_window_pins_left_without_panic_or_nan() {
        // A monitor narrower than the popover itself would make max_x < min_x
        // without the max_x.max(min_x) guard, which panics f64::clamp.
        let monitor = Rect2 {
            x: 100.0,
            y: 0.0,
            w: 200.0, // narrower than WIN.w (392)
            h: 1080.0,
        };
        let icon = Rect2 {
            x: 150.0,
            y: 0.0,
            w: 24.0,
            h: 24.0,
        };
        let (x, _y) = compute_position(icon, WIN, Some(monitor));
        assert!(x.is_finite(), "position must not be NaN/infinite");
        // min_x = 100 + 8 = 108; guard makes max_x collapse to min_x, so we
        // pin to the left margin rather than getting an inverted clamp.
        assert_eq!(x, 108.0);
    }

    #[test]
    fn clamped_at_bottom_edge() {
        // Icon near the bottom of a normal monitor: nothing stopped y from
        // running off the bottom before the vertical clamp was added.
        let icon = Rect2 {
            x: 900.0,
            y: 1050.0,
            w: 24.0,
            h: 24.0,
        };
        let (_x, y) = compute_position(icon, WIN, Some(PRIMARY));
        // max_y = 0 + 1080 - 544 - 8 = 528
        assert_eq!(y, 528.0);
    }

    #[test]
    fn no_monitor_returns_unclamped_raw_position() {
        // With no monitor info at all (e.g. both monitor_from_point and
        // primary_monitor came back None), fall back to the raw centred
        // position rather than panicking or guessing at bounds.
        let icon = Rect2 {
            x: 900.0,
            y: 0.0,
            w: 24.0,
            h: 24.0,
        };
        let (x, y) = compute_position(icon, WIN, None);
        assert_eq!((x, y), (716.0, 24.0));
    }
}
