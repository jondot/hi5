//! Banners, via `notify-rust`.
//!
//! macOS attributes every notification to a *bundle*, so a bare binary
//! cannot post one: `set_application` has to name a registered bundle
//! identifier, and hi5's only exists inside `hi5.app` (see
//! `scripts/bundle.sh`). Run the binary straight out of `target/` and
//! this is a no-op — deliberately, because the alternative found in most
//! examples is to borrow some other app's identifier and post banners
//! that claim to be from Terminal.
//!
//! Failures are swallowed everywhere else too. A banner that cannot be
//! posted — Do Not Disturb, permission not granted — is not a reason to
//! interrupt anything, and the pull request is in the panel either way.

use std::sync::OnceLock;

/// Matches `CFBundleIdentifier` in the bundle's Info.plist, and the
/// config directory the app has always used.
pub const BUNDLE_ID: &str = "com.hi5.app";

fn ready() -> bool {
    static READY: OnceLock<bool> = OnceLock::new();
    *READY.get_or_init(|| match notify_rust::set_application(BUNDLE_ID) {
        Ok(()) => true,
        Err(e) => {
            eprintln!(
                "hi5: notifications are off — {e}. \
                 This is expected when running the binary outside hi5.app."
            );
            false
        }
    })
}

pub fn banner(title: &str, body: &str) {
    if !ready() {
        return;
    }
    let _ = notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .show();
}
