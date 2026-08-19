//! Running `gh`.
//!
//! An app opened from Finder or `/Applications` does not get the user's
//! shell `PATH`; it gets launchd's — `/usr/bin:/bin:/usr/sbin:/sbin` —
//! and Homebrew's `/opt/homebrew/bin` is not on it. So
//! `Command::new("gh")`, which is how this used to run, found `gh` from
//! `cargo run` and reported "The GitHub CLI isn't installed" from the
//! release bundle on the very same machine. [`locate`] is the fix: the
//! `PATH` this process has, then the directories package managers use,
//! then — once — whatever the user's login shell says.

use std::path::{Path, PathBuf};
use std::process::Output;

/// Indirection over process execution so gh behaviour can be tested
/// without a gh binary present.
pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<Output>;
}

pub struct RealRunner;

impl CommandRunner for RealRunner {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<Output> {
        std::process::Command::new(locate(program))
            .args(args)
            .output()
    }
}

/// Where the package managers put executables on macOS, none of which a
/// launchd `PATH` includes. Homebrew on Apple silicon and Intel,
/// MacPorts, nix (per-user and system profiles), and `~/.local/bin`.
fn well_known_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/local/bin"),
        PathBuf::from("/nix/var/nix/profiles/default/bin"),
        PathBuf::from("/run/current-system/sw/bin"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".nix-profile/bin"));
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join("bin"));
    }
    dirs
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

/// The user's own answer to "where is gh", from `Settings::gh_path`.
/// Consulted before everything else in [`locate`], and never cached, so
/// a change in Settings takes effect on the next run.
fn gh_override() -> &'static std::sync::RwLock<Option<PathBuf>> {
    static OVERRIDE: std::sync::OnceLock<std::sync::RwLock<Option<PathBuf>>> =
        std::sync::OnceLock::new();
    OVERRIDE.get_or_init(Default::default)
}

/// Set (or clear) where `gh` is, from settings. Called wherever settings
/// are loaded — the poller at the top of every cycle, the backend before
/// a check — so the file is the one source of truth and this is a copy.
pub fn set_gh_override(path: Option<&str>) {
    let path = path
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(expand_home);
    if let Ok(mut o) = gh_override().write() {
        *o = path;
    }
}

/// `~/x` → `$HOME/x`; a settings field is typed by a person.
fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(p)
}

/// What `gh` resolves to right now, and whether that is runnable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// What `Command::new` would be handed — the override, a found path,
    /// or the bare name if nothing was found.
    pub path: PathBuf,
    /// Whether `path` is an executable file.
    pub runnable: bool,
    /// Whether `path` came from `Settings::gh_path` rather than the search.
    pub overridden: bool,
}

/// [`Resolution`] for `gh`. For the Settings readout: "Using …", or
/// "nothing runnable at …", or "not found".
pub fn resolve_gh() -> Resolution {
    let overridden = gh_override().read().ok().and_then(|o| o.clone());
    let path = locate("gh");
    let runnable = path.is_absolute() && is_executable(&path);
    Resolution {
        path,
        runnable,
        overridden: overridden.is_some(),
    }
}

/// Resolve `program` to something `Command::new` will actually find.
///
/// `gh` with an override set (see [`set_gh_override`]) is the override,
/// runnable or not — it is the user's word. Otherwise a name with a
/// slash is a path and is used as given, and a bare name is searched:
/// this process's `PATH`, then [`well_known_dirs`], then the login shell
/// (`$SHELL -l -c 'command -v gh'`, which sees `mise`, `asdf`, and
/// whatever a dotfile prepends). Found paths are cached for the life of
/// the process; a miss is not, so the next check after `brew install gh`
/// finds it. When nothing finds it, the bare name goes back, and
/// `Command::new` fails the way it always did.
pub fn locate(program: &str) -> PathBuf {
    if program == "gh" {
        if let Some(over) = gh_override().read().ok().and_then(|o| o.clone()) {
            return over;
        }
    }
    if program.contains('/') {
        return PathBuf::from(program);
    }
    if let Some(hit) = cache().lock().ok().and_then(|c| c.get(program).cloned()) {
        return hit;
    }
    let path: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let found = locate_in(program, &path, &well_known_dirs()).or_else(|| from_login_shell(program));
    match found {
        Some(p) => {
            if let Ok(mut c) = cache().lock() {
                c.insert(program.to_string(), p.clone());
            }
            p
        }
        None => PathBuf::from(program),
    }
}

/// The search itself, over explicit lists, so it can be tested without
/// touching the real `PATH`.
pub fn locate_in(program: &str, path: &[PathBuf], well_known: &[PathBuf]) -> Option<PathBuf> {
    path.iter()
        .chain(well_known.iter())
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
}

/// Ask the user's login shell where `program` is. Non-interactive
/// (`-c`), but a login shell (`-l`), so `.zprofile` / `config.fish` and
/// the version managers they set up have run. `command -v` is POSIX and
/// fish alike.
fn from_login_shell(program: &str) -> Option<PathBuf> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let out = std::process::Command::new(shell)
        .args(["-l", "-c", &format!("command -v {program}")])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout);
    let candidate = PathBuf::from(line.lines().last()?.trim());
    (candidate.is_absolute() && is_executable(&candidate)).then_some(candidate)
}

fn cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, PathBuf>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, PathBuf>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

#[cfg(test)]
mod locate_tests {
    use super::*;

    fn executable(dir: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn a_path_hit_wins_over_the_well_known_dirs() {
        let on_path = tempfile::tempdir().unwrap();
        let brew = tempfile::tempdir().unwrap();
        let a = executable(on_path.path(), "gh");
        executable(brew.path(), "gh");
        let found = locate_in(
            "gh",
            &[on_path.path().to_path_buf()],
            &[brew.path().to_path_buf()],
        );
        assert_eq!(found, Some(a));
    }

    #[test]
    fn a_launchd_path_still_finds_gh_where_homebrew_put_it() {
        // The bug: PATH is /usr/bin:/bin:/usr/sbin:/sbin and gh is not
        // there. The well-known list is what finds it.
        let brew = tempfile::tempdir().unwrap();
        let gh = executable(brew.path(), "gh");
        let launchd: Vec<PathBuf> = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"]
            .iter()
            .map(PathBuf::from)
            .collect();
        assert_eq!(
            locate_in("gh", &launchd, &[brew.path().to_path_buf()]),
            Some(gh)
        );
    }

    #[test]
    fn a_file_that_is_not_executable_does_not_count() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("gh"), "").unwrap();
        assert_eq!(locate_in("gh", &[dir.path().to_path_buf()], &[]), None);
    }

    #[test]
    fn a_path_is_used_as_given() {
        assert_eq!(locate("/nowhere/gh"), PathBuf::from("/nowhere/gh"));
    }

    #[test]
    fn the_override_wins_and_is_reported_even_when_it_is_not_runnable() {
        // One test for the override, so its global state cannot leak
        // between tests: set, check, clear.
        set_gh_override(Some("  /nowhere/at/all/gh "));
        let r = resolve_gh();
        assert_eq!(r.path, PathBuf::from("/nowhere/at/all/gh"));
        assert!(r.overridden);
        assert!(!r.runnable, "the readout says so rather than pretending");
        set_gh_override(Some("~/x/gh"));
        assert!(
            !resolve_gh().path.to_string_lossy().starts_with('~'),
            "~ is expanded"
        );
        set_gh_override(Some("   "));
        assert!(!resolve_gh().overridden, "blank is unset");
        set_gh_override(None);
        assert!(!resolve_gh().overridden);
    }
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::collections::HashMap;
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    #[derive(Default)]
    pub struct MockRunner {
        pub responses: HashMap<String, (i32, String, String)>,
        pub missing: bool,
    }

    impl MockRunner {
        pub fn with(mut self, cmd: &str, code: i32, stdout: &str, stderr: &str) -> Self {
            self.responses
                .insert(cmd.to_string(), (code, stdout.into(), stderr.into()));
            self
        }
        pub fn missing_binary() -> Self {
            Self {
                missing: true,
                ..Default::default()
            }
        }
    }

    impl CommandRunner for MockRunner {
        fn run(&self, program: &str, args: &[&str]) -> std::io::Result<Output> {
            if self.missing {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no such file",
                ));
            }
            let key = format!("{program} {}", args.join(" "));
            let (code, out, err) = self.responses.get(&key).cloned().unwrap_or((
                1,
                String::new(),
                "unexpected command".into(),
            ));
            Ok(Output {
                status: ExitStatus::from_raw(code << 8),
                stdout: out.into_bytes(),
                stderr: err.into_bytes(),
            })
        }
    }
}
