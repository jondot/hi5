//! Layout probes: named measurements taken from inside the layout engine.
//!
//! The question "does the rule span the row" or "is the back button
//! where the click lands" has an exact answer in the bounds gpui
//! computed, and this is how that answer is read — instead of
//! screenshotting the window and counting ink, which measures a
//! projection of the layout through the renderer and the display, and
//! guessed wrong about both more than once.
//!
//! A probe is an element with no visual effect. Two shapes:
//!
//! - [`mark`] is an invisible, absolutely positioned child that reports
//!   its parent's padding box. Put it inside the thing you want measured.
//! - [`children`] is a prepaint listener for a container that reports
//!   each of the container's children, indexed in order. Put it on the
//!   thing that holds the controls you want to hit.
//!
//! Records accumulate per thread until [`reset`]; the headless test
//! harness resets before the frame it reads, and the preview binary
//! dumps them beside each screenshot. Both are the *same* frame the user
//! would see, because these are the same `ui::*` functions the app runs.

use std::cell::RefCell;

use gpui::*;

/// One measurement.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub name: &'static str,
    /// Which child, for [`children`]; always 0 for [`mark`].
    pub index: usize,
    pub bounds: Bounds<Pixels>,
}

thread_local! {
    static RECORDS: RefCell<Vec<Probe>> = const { RefCell::new(Vec::new()) };
}

/// Forget every measurement so far.
pub fn reset() {
    RECORDS.with(|r| r.borrow_mut().clear());
}

/// Every measurement since [`reset`], in prepaint order.
pub fn all() -> Vec<Probe> {
    RECORDS.with(|r| r.borrow().clone())
}

/// The bounds recorded under `name`, in prepaint order. For a
/// [`children`] probe that is one entry per child; for a [`mark`] that is
/// one entry per instance rendered (one per visible list row, say).
pub fn get(name: &str) -> Vec<Bounds<Pixels>> {
    RECORDS.with(|r| {
        r.borrow()
            .iter()
            .filter(|p| p.name == name)
            .map(|p| p.bounds)
            .collect()
    })
}

fn record(name: &'static str, index: usize, bounds: Bounds<Pixels>) {
    RECORDS.with(|r| {
        r.borrow_mut().push(Probe {
            name,
            index,
            bounds,
        })
    });
}

/// An invisible child that reports its parent's padding box as `name`.
///
/// Absolutely positioned, so it takes no part in the parent's flow
/// layout; a canvas, so it paints nothing.
pub fn mark(name: &'static str) -> impl IntoElement {
    canvas(move |bounds, _, _| record(name, 0, bounds), |_, _, _, _| {})
        .absolute()
        .inset_0()
}

/// A listener for `Div::on_children_prepainted` that reports each child
/// of the container as `name` with its index.
pub fn children(
    name: &'static str,
) -> impl Fn(Vec<Bounds<Pixels>>, &mut Window, &mut App) + 'static {
    move |bounds, _, _| {
        for (index, b) in bounds.into_iter().enumerate() {
            record(name, index, b);
        }
    }
}
