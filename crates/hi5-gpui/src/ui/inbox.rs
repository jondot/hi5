//! The queue.
//!
//! The list is gpui-component's `List`: a virtualised, sectioned table
//! with its own keyboard navigation, selection and empty state. Sections
//! are repositories, rows are pull requests. Nothing here measures a row
//! or tracks a scroll offset by hand — the previous version did both,
//! which is what made scrolling expensive.
//!
//! The one thing `List` does not do is pin a section's header while its
//! rows scroll under it, and a queue grouped by repository wants that:
//! twelve rows down, "which repo am I in" is the question. So the header
//! of the section at the top of the viewport is drawn a second time, in
//! an overlay on the list, at the top — and pushed up by the next
//! section's header as it arrives. That is arithmetic over the offset
//! `List` already exposes and heights that are constants here (every row
//! is `ROW_HEIGHT`; every header `HEADER_HEIGHT`), not a measurement:
//! see [`InboxDelegate::pinned`].
//!
//! The toolbar is `TabBar` for the scope and `Button` for the three
//! actions, and both menus are `PopupMenu`s hung off a button. Because
//! a `PopupMenu`'s items *are* actions, the repo filter and the ⋯ menu
//! dispatch the same `hi5` actions the keyboard does.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::divider::Divider;
use gpui_component::label::Label;
use gpui_component::list::{List, ListDelegate, ListState};
use gpui_component::menu::DropdownMenu as _;
use gpui_component::popover::Popover;
use gpui_component::scroll::Scrollbar;
use gpui_component::spinner::Spinner;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::tag::Tag;
use gpui_component::{
    avatar::Avatar, h_flex, v_flex, ActiveTheme as _, Icon, IconName, IndexPath, Selectable,
    Sizable as _,
};
use hi5_core::github::{CheckState, PullRequest};
use hi5_core::view::{group_by_repo, inbox_view, Scope};

use crate::actions::{ApproveAll, ClearRepoFocus, OpenSettings, Quit, Refresh, SetScope};
use crate::app::{Hi5, Screen};
use crate::assets::icon;
use crate::ui;
use crate::ui::format::{ellipsize, relative_age};
use crate::ui::repo_filter;
use crate::ui::{facts, Fact};

/// Row geometry, in the library's own steps, written out because the
/// title needs a *definite* width to ellipsize against (see `Row`) and
/// that width is arithmetic over the rest of the row. The panel is a
/// fixed 392pt that never resizes.
const PANEL_WIDTH: f32 = 392.;
/// `p_3` — the row's own horizontal padding, both edges.
const ROW_PADDING: f32 = 12.;
/// `gap_3` — between the avatar, the text column and the chevron.
const GAP: f32 = 12.;
/// `size_8` — the avatar. Sized to the two-line text column beside it
/// (40pt) rather than to one line of it: it is the row's anchor.
const AVATAR: f32 = 32.;
/// `Icon::large()`, `size_6` — the disclosure chevron.
const CHEVRON: f32 = 24.;
/// Two `Label` lines (`rems(1.25)` each, 20pt) plus `py_2` above and
/// below. Every row is this tall whether or not it carries a badge:
/// `ListDelegate` measures one row and assumes the rest match ("NOTE:
/// Every item should have same height"), so a row that grew would put
/// every later row's hit target out of step with where it is drawn.
pub const ROW_HEIGHT: f32 = 2. * 20. + 2. * 8.;
/// A section header: one `text_xs` line, centred in a band `p_3` tall
/// on each side of it — and a *constant*, because the pinned copy is
/// placed by arithmetic that has to agree with the one in the flow.
pub const HEADER_HEIGHT: f32 = 32.;
/// The FOR YOU tag and the gap before it, when a row carries one.
const BADGE: f32 = 72.;

const TITLE_WIDTH: f32 = PANEL_WIDTH - 2. * ROW_PADDING - AVATAR - GAP - GAP - CHEVRON;

/// One repository's run of pull requests — a section of the list.
pub struct Section {
    pub repo: SharedString,
    pub prs: Vec<PullRequest>,
}

/// What the list draws, and the only place the queue's display order
/// lives.
pub struct InboxDelegate {
    sections: Vec<Section>,
    selected: Option<IndexPath>,
    /// Set when the queue is empty *because* the user narrowed it, so
    /// the placeholder can say which of the two empties this is.
    filtered: bool,
    /// How many pull requests exist before the repo focus is applied —
    /// the difference between "nothing to review" and "nothing here".
    unfiltered_total: usize,
    /// Nothing has been fetched yet — see `Hi5::loading`. Written by
    /// `inbox::render` each frame rather than read back from the app,
    /// which is mid-render and cannot be borrowed.
    pub loading: bool,
    /// The repository whose "Approve all" is running, if one is — its
    /// header button spins for the duration. Written like `loading`.
    pub batching: Option<SharedString>,
    app: WeakEntity<Hi5>,
}

impl InboxDelegate {
    pub fn new(app: WeakEntity<Hi5>) -> Self {
        Self {
            sections: Vec::new(),
            selected: None,
            filtered: false,
            unfiltered_total: 0,
            loading: false,
            batching: None,
            app,
        }
    }

    /// Rebuild the sections from the queue as it now stands.
    ///
    /// Called when the data, the scope or the repo focus changes — never
    /// per frame. The list keeps its own scroll offset across this, so a
    /// poll landing while the user reads does not move the page.
    pub fn set_queue(&mut self, prs: &[PullRequest], focus: &[String], scope: Scope) {
        let view = inbox_view(prs, focus, scope);
        self.unfiltered_total = inbox_view(prs, &[], scope).visible.len();
        self.filtered = !focus.is_empty();
        self.sections = group_by_repo(&view.visible)
            .into_iter()
            .map(|(repo, group)| Section {
                repo: repo.into(),
                prs: group.into_iter().cloned().collect(),
            })
            .collect();
        self.selected = self.selected.filter(|ix| self.pr_at(*ix).is_some());
    }

    pub fn pr_at(&self, ix: IndexPath) -> Option<&PullRequest> {
        self.sections.get(ix.section)?.prs.get(ix.row)
    }

    pub fn selected_pr(&self) -> Option<&PullRequest> {
        self.pr_at(self.selected?)
    }

    /// Which section's header is pinned at the top of a viewport
    /// scrolled down by `scrolled`, and where to draw it: `(section,
    /// top, floating)`. `top` is 0 or negative — the amount the *next*
    /// header, arriving from below, has pushed this one off the top.
    /// `floating` is whether it is standing in for a header that has
    /// scrolled away (draw the hairline under it) or sitting exactly
    /// over its own (draw nothing the flow does not).
    ///
    /// Section `i` starts at `Σ_{j<i} (HEADER_HEIGHT + n_j × ROW_HEIGHT)`
    /// — the same sum the virtual list makes from the sizes it measured,
    /// which are these constants. `tests/layout.rs` holds the two to the
    /// same pixel.
    pub fn pinned(&self, scrolled: Pixels) -> Option<(usize, Pixels, bool)> {
        let scrolled = scrolled.max(px(0.));
        let mut top = px(0.);
        let mut current: Option<(usize, Pixels)> = None;
        for (ix, section) in self.sections.iter().enumerate() {
            if top > scrolled {
                // The first header still below the viewport's top edge:
                // it pushes the pinned one up as it approaches.
                let push = (top - scrolled - px(HEADER_HEIGHT)).min(px(0.));
                return current.map(|(c, start)| (c, push, scrolled > start));
            }
            current = Some((ix, top));
            top += px(HEADER_HEIGHT) + px(ROW_HEIGHT) * section.prs.len() as f32;
        }
        current.map(|(c, start)| (c, px(0.), scrolled > start))
    }

    /// The header for section `ix`, as the list draws it and as the
    /// overlay draws it again — measured under `probe` so a test can
    /// tell the two apart (`inbox.header` in the flow, `inbox.pinned`
    /// in the overlay).
    fn header(&self, ix: usize, probe: &'static str, cx: &App) -> Option<AnyElement> {
        let s = self.sections.get(ix)?;
        let busy = self.batching.as_ref() == Some(&s.repo);
        Some(section_header(s.repo.clone(), s.prs.len(), busy, probe, cx).into_any_element())
    }
}

/// A repository's name, "Approve all", and how many of its pull requests
/// follow, in a band exactly `HEADER_HEIGHT` tall.
///
/// A grey band, in the flow and pinned alike — the way a macOS list
/// draws its group rows — so the pinned copy is the same thing seen
/// twice rather than a strip that changes colour the moment it sticks.
///
/// "Approve all" is a plain outline button, not a green one: it opens a
/// confirmation that lists the section, and the green belongs to the
/// step that actually approves. It dispatches `ApproveAll` like every
/// other control, so the flow-header copy and the pinned copy cannot
/// differ in what they do.
fn section_header(
    repo: SharedString,
    count: usize,
    busy: bool,
    probe: &'static str,
    cx: &App,
) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    let action = ApproveAll {
        repo: repo.to_string(),
    };
    h_flex()
        .w_full()
        .h(px(HEADER_HEIGHT))
        .px_3()
        .items_center()
        .gap_2()
        .bg(cx.theme().secondary)
        .child(ui::probe::mark(probe))
        // Shortened by characters, like the detail header's repo (see
        // `format::ellipsize`), so the button and the count keep their
        // places against a long name.
        .child(
            Label::new(ellipsize(&repo, 34))
                .text_xs()
                .text_color(muted)
                .flex_1(),
        )
        .child(
            div()
                .on_children_prepainted(ui::probe::children("inbox.approve-all"))
                .child(
                    ui::text_button(SharedString::from(format!("approve-all-{probe}")))
                        .outline()
                        .small()
                        .compact()
                        .label("Approve all")
                        .loading(busy)
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(Box::new(action.clone()), cx)
                        }),
                ),
        )
        .child(Label::new(format!("{count}")).text_xs().text_color(muted))
}

impl ListDelegate for InboxDelegate {
    type Item = Row;

    fn sections_count(&self, _: &App) -> usize {
        self.sections.len()
    }

    fn items_count(&self, section: usize, _: &App) -> usize {
        self.sections.get(section).map_or(0, |s| s.prs.len())
    }

    fn render_section_header(
        &mut self,
        section: usize,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<impl IntoElement> {
        self.header(section, "inbox.header", cx)
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let pr = self.pr_at(ix)?;
        Some(Row {
            ix,
            pr: pr.clone(),
            selected: false,
        })
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        self.selected = ix;
        cx.notify();
    }

    /// Click or ↵ opens the pull request. Approve deliberately does not
    /// live in the list: it is irreversible and publicly visible, so it
    /// exists only on the detail screen behind its own arming delay.
    fn confirm(&mut self, _: bool, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        let Some(pr) = self.selected_pr().cloned() else {
            return;
        };
        let app = self.app.clone();
        cx.defer(move |cx| {
            let _ = app.update(cx, |this, cx| {
                this.go(Screen::Detail(Box::new(pr)), cx);
            });
        });
    }

    fn cancel(&mut self, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        self.selected = None;
        cx.notify();
    }

    fn loading(&self, _: &App) -> bool {
        self.loading
    }

    /// The first cycle since launch or sign-in has not landed. A spinner
    /// and a sentence, in the empty state's place — never its words,
    /// which promise there is nothing on GitHub.
    fn render_loading(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .p_6()
            .text_color(cx.theme().muted_foreground)
            .child(ui::probe::mark("inbox.loading"))
            .child(Spinner::new().large())
            .child(div().text_sm().child("Checking GitHub…"))
    }

    /// Two different empties, which must not look the same: a queue with
    /// nothing in it, and a queue the user has narrowed to nothing. The
    /// second one names the filter and offers the way out, because a
    /// silently short inbox was a real shipped bug.
    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        let narrowed = self.filtered && self.unfiltered_total > 0;
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .p_6()
            .text_color(cx.theme().muted_foreground)
            .child(Icon::new(IconName::Inbox).size_8())
            .child(div().text_sm().child(if narrowed {
                "No pull requests in the repositories you're focused on."
            } else {
                "Nothing waiting on you."
            }))
            .when(narrowed, |this| {
                this.child(
                    ui::text_button("show-all")
                        .outline()
                        .small()
                        .label(format!("Show all {}", self.unfiltered_total))
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(ClearRepoFocus), cx)
                        }),
                )
            })
    }
}

/// One row of the queue.
///
/// hi5's own element rather than `ListItem`, and not because `ListItem`
/// is unpolished — because it is a *picker* row: a label, a check mark,
/// a suffix, centred in a `py_1 px_3` box. Its children live in a
/// vertically-centred inner band, so a two-line cell placed in it never
/// owns the row's box: a rule drawn at the cell's bottom edge sat ten
/// points above the row boundary, and no amount of padding arithmetic on
/// the outside could reach it. (That was measured, from the layout, not
/// guessed from a screenshot; see `ui::probe`.)
///
/// So the row is the box. Selection and hover use the theme's own list
/// tokens — `list_hover`, `list_active`, `list_active_border` — exactly
/// as `ListItem` does, so a selected row here looks like a selected row
/// anywhere else in the library. `List` still owns the virtualisation,
/// the scrolling, the selection index and the keyboard.
pub struct Row {
    ix: IndexPath,
    pr: PullRequest,
    selected: bool,
}

impl Selectable for Row {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl IntoElement for Row {
    type Element = gpui::Component<Self>;

    fn into_element(self) -> Self::Element {
        gpui::Component::new(self)
    }
}

impl RenderOnce for Row {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let pr = self.pr;
        let selected = self.selected;
        let muted = cx.theme().muted_foreground;
        let title_width = TITLE_WIDTH - if pr.asked_for_you { BADGE } else { 0. };

        let mut meta = vec![
            Fact::new(format!("#{}", pr.number)),
            Fact::new(relative_age(&pr.created_at, chrono::Utc::now())),
            Fact::new(format!("+{}", pr.additions)).color(cx.theme().green),
            Fact::new(format!("−{}", pr.deletions)).color(cx.theme().red),
        ];
        if let Some((text, color)) = checks_fact(pr.checks, cx) {
            meta.push(Fact::new(text).color(color));
        }

        h_flex()
            .id(self.ix)
            .relative()
            .w_full()
            .h(px(ROW_HEIGHT))
            .px_3()
            .gap_3()
            .items_center()
            .overflow_hidden()
            .child(ui::probe::mark("inbox.row"))
            .child(
                // A solid fill with white initials. `Avatar`'s own
                // placeholder tints at 20% and draws the initials in the
                // same hue — a soft treatment that disappeared at this
                // size. The component still does the work; only the two
                // colours are hi5's, and they are the pair that was
                // contrast-checked.
                Avatar::new()
                    .name(pr.author.login.clone())
                    // A *named* size, then the box set by style. Passing
                    // `Size::Size(32)` makes `avatar_text_size` size the
                    // inner text div to half the avatar (avatar/mod.rs:33)
                    // rather than setting a font size, which clipped
                    // every login to a single initial.
                    .small()
                    .size_8()
                    .bg(crate::ui::format::avatar_color(&pr.author.login))
                    .text_color(white())
                    .border_0(),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .items_center()
                            // A *definite* width, not `flex_1` — see
                            // `ui::truncated` for both halves of why. The
                            // panel never resizes, so this is arithmetic.
                            .child(ui::truncated(
                                Label::new(pr.title.clone()).text_sm(),
                                px(title_width),
                            ))
                            .when(pr.asked_for_you, |this| {
                                // A stock `Tag`, and an *informational*
                                // one: the theme's `primary` is near-black
                                // in light mode, so tinting a badge with it
                                // produced a grey pill that read as
                                // disabled rather than as "this one is
                                // yours".
                                this.child(
                                    div()
                                        .flex_shrink_0()
                                        .child(Tag::info().small().child("FOR YOU")),
                                )
                            }),
                    )
                    .child(facts(meta, cx).text_xs().text_color(muted)),
            )
            .child(
                Icon::new(IconName::ChevronRight)
                    .large()
                    .flex_shrink_0()
                    .text_color(muted.opacity(0.75)),
            )
            .map(|this| {
                if selected {
                    // `ListItem`'s selected treatment, token for token.
                    this.bg(cx.theme().list_active).child(
                        div()
                            .absolute()
                            .inset_0()
                            .border_1()
                            .border_color(cx.theme().list_active_border),
                    )
                } else {
                    this.hover(|this| this.bg(cx.theme().list_hover))
                }
            })
            // The separator, as a table view draws it: one hairline on
            // the row's own bottom edge, from the panel's leading edge
            // to its trailing edge. The row is the box, so `left_0
            // right_0 bottom_0` *is* that edge — there is no padding
            // between this and the boundary for it to be inset by.
            //
            // On *every* row, the last of a section included: the rule
            // under the last row is what closes the block before the
            // next header's whitespace. Without it a section ended in
            // nothing and the next repo's name floated in the gap.
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(1.))
                    .bg(cx.theme().border)
                    .child(ui::probe::mark("inbox.rule")),
            )
    }
}

/// The CI state as a fact, when there is anything to say about it.
fn checks_fact(checks: CheckState, cx: &App) -> Option<(&'static str, Hsla)> {
    match checks {
        CheckState::Success => Some(("checks pass", cx.theme().green)),
        CheckState::Failure => Some(("checks failing", cx.theme().red)),
        CheckState::Pending => Some(("checks running", cx.theme().yellow)),
        CheckState::None => None,
    }
}

pub fn render(this: &mut Hi5, _window: &mut Window, cx: &mut Context<Hi5>) -> impl IntoElement {
    let view = inbox_view(&this.prs, &this.focus_repos, this.scope);
    let (all, for_you) = (view.scoped.len(), view.for_you);
    drop(view);

    let strip = this.strip();
    let last_action = this.last_action.clone();

    // Tell the list whether an empty queue means "not yet" (spinner) or
    // "nothing" (the empty state). Written, not notified: this is the
    // frame that draws it.
    let loading = this.loading();
    let batching: Option<SharedString> = this.batch.as_ref().map(|b| b.repo.clone().into());
    this.inbox.update(cx, |list, _| {
        list.delegate_mut().loading = loading;
        list.delegate_mut().batching = batching;
    });

    // The pinned header, from this frame's offset. Read, not tracked:
    // `List` owns the offset and the wheel; this only asks where it is.
    let list = this.inbox.read(cx);
    let scrolled = -list.scroll_handle().base_handle().offset().y;
    let pinned = list
        .delegate()
        .pinned(scrolled)
        .and_then(|(ix, top, floating)| {
            let header = list.delegate().header(ix, "inbox.pinned", cx)?;
            Some((header, top, floating))
        });

    v_flex()
        .size_full()
        .bg(cx.theme().background)
        .child(toolbar(this, all, for_you, cx))
        .children(strip.as_ref().map(ui::status_strip))
        .child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                // The list's own scrollbar would run under the overlay
                // for its first `HEADER_HEIGHT`; it is drawn here instead,
                // last, over both.
                .child(List::new(&this.inbox).scrollbar_visible(false).size_full())
                .children(pinned.map(|(header, top, floating)| {
                    div()
                        .id("pinned-header")
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(top)
                        // Standing in for a header that scrolled away:
                        // rule it off from the rows sliding beneath.
                        // Sitting exactly over its own: nothing the flow
                        // does not draw, so the two are one.
                        .when(floating, |this| {
                            this.border_b_1().border_color(cx.theme().border)
                        })
                        // The rows under it must not light up or take a
                        // click through it; the wheel must still reach
                        // them.
                        .block_mouse_except_scroll()
                        .child(header)
                }))
                .child(Scrollbar::vertical(this.inbox.read(cx).scroll_handle())),
        )
        .children(last_action.as_ref().map(|a| ui::action_bar(a, cx)))
}

fn toolbar(this: &Hi5, all: usize, for_you: usize, cx: &mut Context<Hi5>) -> impl IntoElement {
    let scope = this.scope;
    let focus = this.focus_repos.clone();
    let filtered = !focus.is_empty();
    let action_context = this.focus.clone();
    // See `Hi5::menu_generation`: a menu's open flag lives in window
    // element state keyed by id and outlives the panel being ordered
    // out, so hiding the panel bumps the generation and every menu comes
    // back as a new, closed one.
    let gen = this.menu_generation;

    h_flex()
        .flex_shrink_0()
        .w_full()
        .gap_1()
        .px_2()
        .py_1p5()
        .bg(cx.theme().title_bar)
        .border_b_1()
        .border_color(cx.theme().title_bar_border)
        .child(ui::probe::mark("inbox.toolbar"))
        .child(
            // Not stretched: a segmented control is as wide as its
            // segments, and `flex_1` turned the recessed track into a
            // grey band running the width of the toolbar. The spacer
            // after it takes the slack instead.
            TabBar::new("scope")
                .segmented()
                .small()
                .selected_index(if scope == Scope::ForYou { 1 } else { 0 })
                // `child`, not `suffix`: a `Tab`'s children go inside its
                // raised inner box, where its suffix sits outside — which
                // left the count stranded on the track next to the pill.
                .child(Tab::new().label("All").child(count_badge(all, cx)))
                .child(Tab::new().label("For you").child(count_badge(for_you, cx)))
                .on_click(|ix: &usize, window, cx| {
                    window.dispatch_action(Box::new(SetScope { for_you: *ix == 1 }), cx)
                }),
        )
        .child(div().flex_1().min_w_0())
        .child(filter_popover(this, filtered, focus.len(), cx))
        .child(
            Button::new("refresh")
                .ghost()
                .icon(Icon::empty().path(icon::REFRESH))
                .tooltip("Refresh now")
                // A spinner in the icon's place, dimmed, and the click
                // swallowed, until the poller answers: a refresh that
                // showed nothing between the click and the new list got
                // clicked three times.
                .loading(this.refreshing)
                .on_click(|_, window, cx| window.dispatch_action(Box::new(Refresh), cx)),
        )
        .child(
            Button::new(SharedString::from(format!("more-{gen}")))
                .ghost()
                .icon(IconName::Ellipsis)
                .dropdown_menu_with_anchor(Corner::TopRight, move |menu, _, _| {
                    menu.action_context(action_context.clone())
                        .menu("Settings…", Box::new(OpenSettings))
                        .separator()
                        // The only way out of the app: no Dock icon, no
                        // app menu, and the tray click toggles the panel
                        // rather than opening a menu.
                        .menu("Quit hi5", Box::new(Quit))
                }),
        )
}

/// The count that rides inside a scope segment.
fn count_badge(n: usize, cx: &App) -> impl IntoElement {
    // A `Tab`'s inner box lays its label and children out with no gap
    // between them, so the count needs its own. The *same* size as the
    // tab's label, muted rather than smaller: a smaller run beside a
    // larger one is centred on its own em box, which puts its baseline
    // a pixel above the label's — "For you 1" read as a superscript.
    // Same size, same box, same baseline.
    Label::new(format!("{n}"))
        .ml_1()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
}

/// The repo focus control.
///
/// The trigger is not decoration: an engaged filter is the only thing
/// standing between the user and a silently short inbox, so it becomes
/// an accented pill carrying its own count rather than a glyph that
/// looks identical either way.
fn filter_popover(
    this: &Hi5,
    filtered: bool,
    count: usize,
    _cx: &mut Context<Hi5>,
) -> impl IntoElement {
    let state = this.repo_filter.clone();
    Popover::new(SharedString::from(format!(
        "repo-filter-{}",
        this.menu_generation
    )))
    .anchor(Corner::TopRight)
    // Only the preview sets this: a `Popover`'s open flag is element
    // state, and this is the one hook it offers for the initial value.
    .default_open(this.preview_filter_open)
    // No padding of its own: the search field, the rows and the footer
    // run edge to edge inside the panel's border, the way a macOS
    // popover with a list in it does. The stock 12pt inset around a
    // list left a table floating in a frame.
    .p_0()
    .overflow_hidden()
    .trigger(
        Button::new("filter")
            .ghost()
            // `info`, not `primary`: the theme's primary is #171717 in
            // light mode, so an engaged filter came out as a heavy black
            // pill. This is a *state*, not a call to action — it wants
            // the accent, which is what info is (#0ea5e9). And no
            // `small()`: it sits between two default-sized buttons, and
            // was a size down from both.
            .when(filtered, |b| b.info())
            .icon(Icon::empty().path(icon::FILTER))
            .when(filtered, |b| b.label(format!("{count}"))),
    )
    .content(move |_, _, _| {
        v_flex()
            .w(repo_filter::MENU_WIDTH)
            .h(repo_filter::MENU_HEIGHT)
            .child(
                List::new(&state)
                    .small()
                    .search_placeholder("Search repositories")
                    .size_full(),
            )
            // Ruled off from the list: without it "All repositories"
            // read as one more repository, and an unlucky click cleared
            // the filter instead of adding to it.
            .child(Divider::horizontal())
            .child(
                div().p_1p5().child(
                    ui::text_button("all-repos")
                        .ghost()
                        .small()
                        .w_full()
                        .label("Show all repositories")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(ClearRepoFocus), cx)
                        }),
                ),
            )
    })
}

#[cfg(test)]
mod tests {
    // Named imports: `super::*` would bring in `gpui::test`, which
    // shadows `#[test]` with itself.
    use gpui::{px, WeakEntity};
    use hi5_core::view::Scope;

    use super::{InboxDelegate, HEADER_HEIGHT, ROW_HEIGHT};
    use crate::fixtures;

    fn delegate() -> InboxDelegate {
        let mut d = InboxDelegate::new(WeakEntity::new_invalid());
        d.set_queue(&fixtures::long_queue(), &[], Scope::All);
        d
    }

    /// Sections in queue order — 4, 2 and 10 rows: the second starts
    /// after one header and four rows, the third after two headers and
    /// six.
    #[test]
    fn the_pinned_header_is_the_section_at_the_top_pushed_by_the_next() {
        let d = delegate();
        assert_eq!(
            d.sections.iter().map(|s| s.prs.len()).collect::<Vec<_>>(),
            vec![4, 2, 10]
        );
        let h = px(HEADER_HEIGHT);
        let r = px(ROW_HEIGHT);
        let second = h + r * 4.;
        let third = second + h + r * 2.;

        assert_eq!(d.pinned(px(0.)), Some((0, px(0.), false)));
        assert_eq!(
            d.pinned(px(-30.)),
            Some((0, px(0.), false)),
            "overscroll is rest"
        );
        assert_eq!(
            d.pinned(px(1.)),
            Some((0, px(0.), true)),
            "a pixel in, it floats"
        );
        assert_eq!(
            d.pinned(second - h),
            Some((0, px(0.), true)),
            "touching, not pushed"
        );
        assert_eq!(d.pinned(second - h + px(14.)), Some((0, px(-14.), true)));
        assert_eq!(d.pinned(second - px(1.)), Some((0, h * -1. + px(1.), true)));
        assert_eq!(
            d.pinned(second),
            Some((1, px(0.), false)),
            "the next takes over"
        );
        assert_eq!(d.pinned(second + px(50.)), Some((1, px(0.), true)));
        assert_eq!(d.pinned(third - px(10.)), Some((1, px(-22.), true)));
        assert_eq!(
            d.pinned(third + px(999.)),
            Some((2, px(0.), true)),
            "the last never pushed"
        );
    }

    #[test]
    fn an_empty_queue_pins_nothing() {
        let mut d = InboxDelegate::new(WeakEntity::new_invalid());
        d.set_queue(&[], &[], Scope::All);
        assert_eq!(d.pinned(px(0.)), None);
    }
}
