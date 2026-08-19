//! One type scale.
//!
//! gpui-component's components set their text in the library's own
//! steps — `text_xs`, `text_sm`, `text_base` — off a 16px rem, and set
//! line height through `Label`. hi5's screens must use the same steps,
//! or the app renders four sizes from two systems that do not know about
//! each other: stock buttons and menus at 14, group titles at 16, and
//! hi5's own text at whatever absolute number was typed. That is what
//! "the typography is all over the place" was.
//!
//! This is a lint, run as a test so it cannot be forgotten: no screen
//! sets an absolute text size, an absolute line height, or a font
//! family. Text goes through `Label` (or a step helper) and inherits the
//! rest.

use std::fs;
use std::path::Path;

const FORBIDDEN: &[(&str, &str)] = &[
    (
        "text_size(",
        "use text_xs / text_sm / text_base — the library's steps",
    ),
    (
        "line_height(px(",
        "use Label, which carries the library's line height",
    ),
    (
        "font_family(",
        "one face per line: mixed faces do not share a baseline",
    ),
    (
        "font_size = ",
        "Theme::font_size is the rem; do not touch it",
    ),
    (
        ".font_size(",
        "Theme::font_size is the rem; do not touch it",
    ),
];

#[test]
fn screens_use_only_the_librarys_type_steps() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files: Vec<_> = fs::read_dir(root.join("ui"))
        .expect("src/ui")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.push(root.join("theme.rs"));
    files.sort();

    let mut offences = Vec::new();
    for path in &files {
        let text = fs::read_to_string(path).unwrap();
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for (needle, why) in FORBIDDEN {
                if code.contains(needle) {
                    offences.push(format!(
                        "{}:{}: `{}` — {why}",
                        path.strip_prefix(root.parent().unwrap()).unwrap().display(),
                        n + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        offences.is_empty(),
        "absolute typography in the screens:\n  {}",
        offences.join("\n  ")
    );
}
