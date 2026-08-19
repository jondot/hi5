//! hi5's domain logic, with no UI framework anywhere in it.
//!
//! Everything here was written for, and proven by, the Tauri
//! implementation this branch replaces; it moved across unchanged
//! because none of it ever knew what was drawing the panel. That is the
//! point of the split: the GPUI rewrite is a new *shell*, not a new
//! product, and the parts that decide which pull requests are waiting
//! for a review should not be rewritten to change how they are drawn.
//!
//! The shell supplies three things this crate deliberately does not
//! know how to do — showing a notification, setting the menu-bar badge,
//! and telling the UI something changed. They arrive through
//! [`poller::PollHost`].

pub mod auth;
pub mod error;
pub mod geometry;
pub mod github;
pub mod inbox;
pub mod notify_diff;
pub mod poller;
pub mod query;
pub mod store;
pub mod view;

pub use error::{AppError, Result};
