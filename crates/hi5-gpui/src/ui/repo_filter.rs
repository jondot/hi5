//! The repo focus filter: "let me sit on these repos for a while".
//!
//! A `List` rather than a `PopupMenu`, for one reason: a menu has no
//! search field, and 28 watched organisations produce more repositories
//! than anyone wants to scroll past. `ListState::searchable` brings the
//! field, the prefix icon, the clear button and `perform_search` — and
//! the list underneath is the same virtualised one the inbox uses.
//!
//! This is a *view*, not a rule. It survives a relaunch (see
//! `store::settings::Session`) because a menu-bar app gets relaunched by
//! logouts and updates, but the persistent "never show me this repo" is
//! Settings ▸ Repositories, which mutes a repo out of the queue.

use gpui::*;
use gpui_component::label::Label;
use gpui_component::list::{ListDelegate, ListItem, ListState};
use gpui_component::{h_flex, ActiveTheme as _, Icon, IconName, IndexPath, Sizable as _};

use crate::app::Hi5;
use crate::ui;

pub struct RepoFilterDelegate {
    all: Vec<(String, usize)>,
    matched: Vec<(String, usize)>,
    focused: Vec<String>,
    selected: Option<IndexPath>,
    query: String,
    app: WeakEntity<Hi5>,
}

impl RepoFilterDelegate {
    pub fn new(app: WeakEntity<Hi5>) -> Self {
        Self {
            all: Vec::new(),
            matched: Vec::new(),
            focused: Vec::new(),
            selected: None,
            query: String::new(),
            app,
        }
    }

    /// Refresh the repo list and which of them are focused.
    pub fn set_repos(&mut self, repos: Vec<(String, usize)>, focused: Vec<String>) {
        self.all = repos;
        self.focused = focused;
        self.refilter();
    }

    fn refilter(&mut self) {
        let needle = self.query.trim().to_lowercase();
        self.matched = if needle.is_empty() {
            self.all.clone()
        } else {
            self.all
                .iter()
                .filter(|(repo, _)| repo.to_lowercase().contains(&needle))
                .cloned()
                .collect()
        };
    }
}

impl ListDelegate for RepoFilterDelegate {
    type Item = ListItem;

    fn items_count(&self, _: usize, _: &App) -> usize {
        self.matched.len()
    }

    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        self.query = query.to_string();
        self.refilter();
        Task::ready(())
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let (repo, n) = self.matched.get(ix.row)?;
        let on = self.focused.contains(repo);
        Some(
            // `text_sm`: `ListItem` sets `text_base` for itself, the
            // library's picker size, but this picker sits in an app whose
            // body — the queue, settings, the ⋯ menu — is `text_sm`.
            ListItem::new(ix)
                .selected(Some(ix) == self.selected)
                .text_sm()
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        // A fixed leading column for the mark, so labels
                        // line up whether or not they are ticked — the
                        // one detail that separates a list of checkable
                        // things from a list with some ticks in it.
                        .child(div().flex_shrink_0().w_4().children(on.then(|| {
                            Icon::new(IconName::Check)
                                .xsmall()
                                .text_color(cx.theme().primary)
                        })))
                        // A definite width, for the same reason the queue's
                        // title carries one: GPUI ellipsizes only against a
                        // width the layout already knows, and a `flex_1`
                        // column measures at max-content and gets clipped
                        // mid-word instead. These are repo names — every one
                        // of them was cut.
                        .child(ui::truncated(Label::new(repo.clone()), px(LABEL_WIDTH)))
                        .child(
                            Label::new(format!("{n}"))
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground),
                        ),
                ),
        )
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

    /// Toggling deliberately leaves the list open: focusing "a repo or
    /// a few" is usually more than one click.
    fn confirm(&mut self, _: bool, _: &mut Window, cx: &mut Context<ListState<Self>>) {
        let Some(ix) = self.selected else { return };
        let Some((repo, _)) = self.matched.get(ix.row).cloned() else {
            return;
        };
        let app = self.app.clone();
        cx.defer(move |cx| {
            let _ = app.update(cx, |this, cx| {
                this.toggle_repo_focus_by_name(&repo, cx);
            });
        });
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        div()
            .p_3()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(if self.all.is_empty() {
                "No repositories in the queue."
            } else {
                "Nothing matches that search."
            })
    }
}

/// How big the filter panel is. Wide, because repo names are; tall,
/// because it is a picker and a picker that scrolls after five rows is
/// a picker you search instead of read.
pub const MENU_WIDTH: Pixels = px(328.);
pub const MENU_HEIGHT: Pixels = px(400.);

/// The label column, derived from the panel: the popover has no padding
/// of its own (see `inbox::filter_popover`), `ListItem` pads by 12 a
/// side, and the row spends 16 on the check column, 8 on gaps and 22 on
/// the trailing count.
const LABEL_WIDTH: f32 = 328. - 2. * 12. - 16. - 8. - 22.;
