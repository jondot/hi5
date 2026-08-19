//! Asset source: hi5's own glyphs, with gpui-component's behind them.
//!
//! The component library resolves its icons through the same
//! `AssetSource`, so replacing it outright would leave every stock
//! component (menus, inputs, scrollbars) without a glyph. This tries
//! hi5's embedded set first and falls through to theirs, which is the
//! only arrangement where both work.

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

/// Embedded rather than read from disk: the app is a single binary in
/// the menu bar, and an icon that fails to load at runtime is a blank
/// square with no error anywhere.
const ICONS: &[(&str, &str)] = &[
    (
        "icons/filter.svg",
        include_str!("../assets/icons/filter.svg"),
    ),
    (
        "icons/refresh.svg",
        include_str!("../assets/icons/refresh.svg"),
    ),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, svg)) = ICONS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(svg.as_bytes())));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut names: Vec<SharedString> = ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect();
        names.extend(gpui_component_assets::Assets.list(path)?);
        Ok(names)
    }
}

/// The two glyphs hi5 still draws itself.
///
/// Everything else comes from `IconName`. These two do not: the icon set
/// gpui-component ships has no filter and no refresh, and a menu-bar
/// panel whose two most-used controls are unlabelled squares is worse
/// than two custom SVGs.
///
/// Both are drawn to Lucide's metrics — 24x24 box, stroke-width 2, round
/// caps — because that is what the rest of the set is, and an icon whose
/// glyph fills a different fraction of its box reads as the wrong size
/// however carefully the box is sized. The first pair filled 42% of
/// their height where Lucide's fill 75%, which is the whole of "the
/// icons are way too small".
pub mod icon {
    pub const FILTER: &str = "icons/filter.svg";
    pub const REFRESH: &str = "icons/refresh.svg";
}
