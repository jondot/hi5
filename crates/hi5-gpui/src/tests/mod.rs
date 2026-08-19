//! Headless UI tests. See `testing.rs` for the harness.
//!
//! Each file is one concern. A test here asserts something the user can
//! see or do — a screen changes, a control sits where a click lands, a
//! command reaches the backend — never the shape of the element tree.

mod actions;
mod layout;
mod navigation;
mod typography;
