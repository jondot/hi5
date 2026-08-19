//! hi5's GPUI application, as a library.
//!
//! A library rather than one binary's module tree because there are two
//! binaries: the app, and `preview`, which renders every screen from
//! fixtures and photographs itself. The preview used to include these
//! modules by `#[path]`, which compiled the whole crate a second time
//! and reported two thirds of it as dead code.

pub mod actions;
pub mod app;
pub mod assets;
pub mod backend;
pub mod decisions;
pub mod fixtures;
pub mod platform;
pub mod theme;
pub mod ui;

#[cfg(test)]
pub mod testing;
#[cfg(test)]
mod tests;

/// Where settings.json and state.json live.
///
/// `~/Library/Application Support/com.hi5.app` — the same path the
/// previous implementation used, so an existing install keeps its
/// skips, its watched orgs and its appearance.
///
/// `HI5_CONFIG_DIR` overrides it, so a run of the app can be pointed at
/// a throwaway copy — a debugging session must not be able to touch the
/// install you actually use. (The `preview` binary and the headless
/// tests do not go through this at all: they hand a scratch directory
/// to `Backend` directly.)
pub fn config_dir() -> std::path::PathBuf {
    let dir = match std::env::var("HI5_CONFIG_DIR") {
        Ok(path) if !path.is_empty() => std::path::PathBuf::from(path),
        _ => {
            let home = std::env::var("HOME").unwrap_or_default();
            std::path::PathBuf::from(home)
                .join("Library/Application Support")
                .join("com.hi5.app")
        }
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}
