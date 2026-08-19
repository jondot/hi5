//! Launch at login.
//!
//! `auto-launch` rather than writing a LaunchAgent plist by hand. The
//! last implementation of this shipped a plist pointing at a debug build
//! in `target/`, which would have launched at the next login and broken
//! the moment the directory was cleaned.
use std::sync::OnceLock;

use auto_launch::AutoLaunch;

fn launcher() -> Option<&'static AutoLaunch> {
    static LAUNCHER: OnceLock<Option<AutoLaunch>> = OnceLock::new();
    LAUNCHER
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            Some(AutoLaunch::new(
                "hi5",
                &exe.to_string_lossy(),
                false,
                &[] as &[&str],
            ))
        })
        .as_ref()
}

pub fn set(enabled: bool) {
    let Some(l) = launcher() else { return };
    let _ = if enabled { l.enable() } else { l.disable() };
}
