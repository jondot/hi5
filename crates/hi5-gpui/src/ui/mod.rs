//! The panel's screens, and the three pieces of chrome they share.
//!
//! Everything here composes gpui-component. There is no hi5 widget
//! vocabulary any more — no hand-built segmented control, menu, inset
//! table, check row or separator. The previous version had all of those,
//! each with a comment justifying why the stock component was not quite
//! right, and the sum of those small departures was an app that looked
//! hand-drawn: buttons with no radius, icons sized to nothing, text off
//! its baseline, a list padded on one side and not the other. A
//! component library's polish lives in the relationships between its
//! parts, and it only survives if you take the parts.

pub mod approve_all;
pub mod auth;
pub mod detail;
pub mod format;
pub mod inbox;
pub mod probe;
pub mod repo_filter;
pub mod settings;

use gpui::*;
use gpui_component::alert::Alert;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::label::Label;
use gpui_component::{h_flex, ActiveTheme as _, IconName, StyledExt as _};

use crate::actions::Back;

/// The header on every screen that is not the inbox: a back button, a
/// title, and whatever the screen puts on the trailing edge.
///
/// The same height as the inbox toolbar — the same `py_1p5` around a
/// default-size icon button — so the bar does not step down by eight
/// points when a row is opened. `tests/layout.rs` holds the two equal.
pub fn screen_header(
    title: impl Into<SharedString>,
    trailing: Option<AnyElement>,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .flex_shrink_0()
        .w_full()
        .gap_1()
        .px_2()
        .py_1p5()
        .bg(cx.theme().title_bar)
        .border_b_1()
        .border_color(cx.theme().title_bar_border)
        // `header[0]` is the back button, `[1]` the title, `[2]` the
        // trailing control when there is one.
        .on_children_prepainted(probe::children("header"))
        .child(
            Button::new("back")
                .ghost()
                .icon(IconName::ChevronLeft)
                .on_click(|_, window, cx| window.dispatch_action(Box::new(Back), cx)),
        )
        .child(
            Label::new(title)
                .flex_1()
                .min_w_0()
                .text_sm()
                .font_semibold(),
        )
        .children(trailing)
        // Last, so it does not shift the indices above.
        .child(probe::mark("screen.header"))
}

/// The strip between the toolbar and the body, when something is wrong.
///
/// `Alert::banner()` is exactly this shape: full-bleed, an icon, one
/// line, tinted by variant. Amber rather than red for a lost connection
/// — the menu-bar badge already reports the same state, and red here
/// would make one condition read as two problems.
pub fn status_strip(strip: &crate::app::Strip) -> impl IntoElement {
    use crate::app::Strip;

    let alert = match strip {
        Strip::Disconnected(msg) => Alert::warning("strip", msg.clone()),
        Strip::RateLimited(reset) => {
            let mins = ((reset - chrono::Utc::now().timestamp()) / 60).max(1);
            Alert::warning(
                "strip",
                format!("GitHub rate limit — retrying in about {mins} minutes"),
            )
        }
        Strip::Stale(msg) => Alert::info("strip", msg.clone()),
    };
    div().flex_shrink_0().child(alert.banner())
}

/// The footer that confirms an action landed. Hidden until something has
/// actually happened — it exists to confirm, not to narrate.
pub fn action_bar(action: &crate::app::LastAction, cx: &App) -> impl IntoElement {
    use crate::app::LastAction;

    // Status-bar text, in the status bar's colour: the confirmation is
    // the sentence, not a green one. Only a failure is coloured, because
    // that one is an error and reads as one.
    let (fg, text) = match action {
        LastAction::Approved { repo, number } => (
            cx.theme().muted_foreground,
            format!("Approved {repo} #{number}"),
        ),
        LastAction::ApproveFailed { repo, number } => (
            cx.theme().danger,
            format!("Could not approve {repo} #{number}"),
        ),
        LastAction::Skipped { repo, number } => (
            cx.theme().muted_foreground,
            format!("Skipped {repo} #{number}"),
        ),
        LastAction::ApprovedAll {
            repo,
            approved,
            total,
        } if approved == total => (
            cx.theme().muted_foreground,
            format!("Approved all {approved} in {repo}"),
        ),
        LastAction::ApprovedAll {
            repo,
            approved,
            total,
        } => (
            cx.theme().danger,
            format!("Approved {approved} of {total} in {repo}"),
        ),
    };
    h_flex()
        .flex_shrink_0()
        .w_full()
        .px_3()
        .py_1()
        .bg(cx.theme().title_bar)
        .border_t_1()
        .border_color(cx.theme().title_bar_border)
        .child(Label::new(text).text_xs().text_color(fg))
}

/// The line every piece of text in the app sits on: `Label`'s
/// `rems(1.25)`, 20pt at the library's rem.
///
/// Set once on the app root, so it also reaches the text *inside* stock
/// controls. That matters for a reason that was measured rather than
/// seen: gpui's default line height is φ, so a 14pt button label is a
/// 22.65pt line box inside a small button's 22pt interior; the overflow
/// is centred at a half-pixel, the layout rounds it, and the label lands
/// a pixel low in every 24pt button at 1x. A 20pt line fits with a whole
/// pixel to spare on each side, and the label sits where the box says.
pub const LINE: Rems = Rems(1.25);

/// A button whose content is a text label.
///
/// Two points of bottom padding, and only here: an optical correction,
/// measured rather than felt. gpui centres a text run's ascent+descent
/// band in its line box. For the system font at 14pt that band's centre
/// is 0.4pt below the cap-height centre, and the *visual* centre of a
/// mixed-case word — between its x-height and its caps — is another
/// 0.6pt below that; so "Open" and "Skip" measured a full pixel low in
/// a 24pt button. gpui rounds layout to whole pixels (taffy rounding is
/// on), so one point of padding rounds away to nothing; two lifts the
/// label by exactly one pixel, which is the correction. Icon buttons are
/// left alone: their glyphs are centred by their own box and measure
/// half a pixel *high*, so this would push them further.
pub fn text_button(id: impl Into<ElementId>) -> Button {
    Button::new(id).pb(px(2.))
}

/// A label that ends in an ellipsis when it does not fit `width`.
///
/// Two things have to be true for gpui to draw an ellipsis, and both are
/// easy to lose:
///
/// 1. The text needs a *definite* width to truncate against
///    (`elements/text.rs:357`): a `flex_1` column is sized by flex
///    resolution, so the text measures at max-content, keeps that size,
///    and gets clipped mid-word with no ellipsis.
/// 2. That measurement is cached for the frame (`:373`), and the *first*
///    measurement wins. A flex item's automatic minimum size is computed
///    by laying it out at min-content with its own `width` ignored
///    (`SizingMode::ContentSize`), so a fixed-width element that is
///    itself the flex item is first measured as if it had no width —
///    the full text goes into the cache, and the later, definite pass
///    reads it back. Wrapping the fixed-width label in an auto-sized
///    `flex_shrink_0` div makes *that* the flex item; its child is then
///    laid out with the width already known, and the first measurement
///    is the truncated one.
///
/// So: this, not `Label::new(..).w(..).truncate()`.
pub fn truncated(label: Label, width: Pixels) -> Div {
    div().flex_shrink_0().child(label.w(width).truncate())
}

/// One inline fact in a [`facts`] line.
pub struct Fact {
    pub text: SharedString,
    pub color: Option<Hsla>,
    pub weight: Option<FontWeight>,
}

impl Fact {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            color: None,
            weight: None,
        }
    }

    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    pub fn weight(mut self, weight: FontWeight) -> Self {
        self.weight = Some(weight);
        self
    }
}

/// A run of inline facts — `#134 · 3d · +26 −16` — as *one* piece of
/// text.
///
/// One `StyledText` with a highlight per fact, not a flex row of one
/// element per fact. Every fact then shares one font, one size and one
/// baseline by construction, and a long run wraps the way a sentence
/// does. The flex-row version could be — and was — one element in a
/// different face, sitting a point above its neighbours, with nothing
/// in the layout able to say so.
///
/// Facts are joined with an interpunct in the muted colour; the caller
/// sets the size and the resting colour on the returned element, the
/// same way it would on a `Label`. The line height is `Label`'s.
pub fn facts(items: impl IntoIterator<Item = Fact>, cx: &App) -> Div {
    let dot = " · ";
    let dim = cx.theme().muted_foreground.opacity(0.6);
    let mut text = String::new();
    let mut highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
    for (i, fact) in items.into_iter().enumerate() {
        if i > 0 {
            let start = text.len();
            text.push_str(dot);
            highlights.push((
                start..text.len(),
                HighlightStyle {
                    color: Some(dim),
                    ..Default::default()
                },
            ));
        }
        let start = text.len();
        text.push_str(&fact.text);
        if fact.color.is_some() || fact.weight.is_some() {
            highlights.push((
                start..text.len(),
                HighlightStyle {
                    color: fact.color,
                    font_weight: fact.weight,
                    ..Default::default()
                },
            ));
        }
    }
    // A `Label` with more than one colour, on the same line as one.
    div()
        .line_height(LINE)
        .child(StyledText::new(text).with_highlights(highlights))
}
