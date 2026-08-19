//! What hi5 will actually run for `gh`, given this process's environment.
//!
//!     cargo run -p hi5-core --example which
//!     env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin HOME=$HOME SHELL=$SHELL \
//!         cargo run -p hi5-core --example which      # a Finder launch's PATH
//!
//! The second form is the environment an app opened from `/Applications`
//! gets, and it is what the first release ran `gh` under.

fn main() {
    let program = std::env::args().nth(1).unwrap_or_else(|| "gh".to_string());
    let path = std::env::var("PATH").unwrap_or_default();
    println!("PATH = {path}");
    println!(
        "{program} -> {}",
        hi5_core::auth::runner::locate(&program).display()
    );
}
