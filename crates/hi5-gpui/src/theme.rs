//! Appearance: which of gpui-component's two themes is installed, and
//! the two dimensions a 392pt panel has to change about them.
//!
//! There is deliberately no hi5 palette here any more. The previous
//! version of this file was 533 lines defining every surface, hairline
//! and accent by hand and then feeding them into gpui-component's theme
//! so the stock widgets would match. It worked, in the sense that the
//! colours agreed — and it was also the direct cause of most of the
//! app's "unpolished" complaints, because a component library's polish
//! lives in the *relationships* between its tokens, not in any one of
//! them. Override `background` alone and `TabBar::segmented`, which
//! draws the raised segment in `background` against a `tab_bar_segmented`
//! track, silently loses its only contrast. That happened. So: the stock
//! themes, with exactly one token swapped — the accent, see [`accent`],
//! which is a colour the library uses on its own terms rather than a
//! relationship it depends on.
//!
//! Beyond that, what is set here is the one thing that genuinely differs
//! for a menu-bar panel rather than a window — its corner radius. The
//! type scale is the library's: `text_xs` / `text_sm` / `text_base` off a
//! 16px rem, with `Label`'s line height. Not because 16 is a menu-bar
//! number — it is not; macOS's is 13 — but because gpui-component's
//! spacing, control heights and icon sizes are all rem-derived, and
//! `Root` sets the rem from `Theme::font_size` (`root.rs:398`). Setting
//! that to 13 did not choose a text size; it scaled the entire design
//! system to 81% and left every stock control slightly small and slightly
//! tight. Stating hi5's own text in absolute points beside it produced
//! four sizes from two systems. So: the library's rem, the library's
//! steps, and `tests/typography.rs` to keep it that way.

use gpui::{px, rgb, App, Hsla, Pixels, Window, WindowAppearance};
use gpui_component::{Theme, ThemeMode};
use hi5_core::store::settings::Appearance;

/// The corner radius of the panel itself.
///
/// Measured, not chosen: a real macOS menu was screenshotted on a 1x
/// display and its top-left corner profile fitted to a circle — left
/// edge at x=5.12, radius 7.2px, mean squared error 0.0013px². Seven is
/// that, rounded. `tools/ui/probe.py corner` re-runs the same fit
/// against hi5's own captures.
pub const WINDOW_RADIUS: Pixels = px(7.);

/// Install the base theme. Called once, at startup, after
/// `gpui_component::init`.
///
/// Deliberately leaves `Theme::font_size` alone — see the module doc for
/// what changing it actually does.
pub fn install(cx: &mut App) {
    Theme::change(ThemeMode::Light, None, cx);
    accent(cx);
}

/// The one token hi5 overrides: `primary`, and the three that travel
/// with it.
///
/// The stock light theme's primary is #171717 — near-black. That is a
/// fine colour for a document app's default button and a poor one for
/// the *on* state of a switch: black-on-grey reads as "disabled" where
/// every macOS control the user has ever flipped is blue. So `primary`
/// is the system's own accent blue, in the light and dark values macOS
/// uses, and everything the library draws in `primary` — switches,
/// the underline of a selected tab, the one filled button on the
/// welcome screen — follows. Nothing else is touched: `info`, which
/// hi5's tags and filter pill already use, keeps the theme's sky, and
/// `ring` stays the theme's focus colour.
///
/// Reapplied after every `Theme::change`, because that call rewrites the
/// whole palette from the theme file.
fn accent(cx: &mut App) {
    let theme = Theme::global_mut(cx);
    let (base, hover, active): (u32, u32, u32) = if theme.mode.is_dark() {
        (0x0A84FF, 0x2B93FF, 0x4DA3FF)
    } else {
        (0x007AFF, 0x0071EC, 0x0066D6)
    };
    theme.colors.primary = rgb(base).into();
    theme.colors.primary_hover = rgb(hover).into();
    theme.colors.primary_active = rgb(active).into();
    theme.colors.primary_foreground = Hsla::white();
}

/// Whether the requested appearance resolves to dark right now.
///
/// `System` is re-resolved from the window on every frame rather than
/// stored, so a live macOS appearance switch is picked up with no
/// listener and no state to go stale.
pub fn is_dark(appearance: Appearance, window: WindowAppearance) -> bool {
    match appearance {
        Appearance::Light => false,
        Appearance::Dark => true,
        Appearance::System => matches!(
            window,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ),
    }
}

/// Switch light and dark.
pub fn set_mode(dark: bool, window: &mut Window, cx: &mut App) {
    Theme::change(
        if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        Some(window),
        cx,
    );
    accent(cx);
}
