# UI verification

1. **Diagnose and prove in headless tests.** `cargo test -p hi5-gpui` runs
   the real screens through gpui's layout engine with no window
   (`crates/hi5-gpui/src/testing.rs`). Bounds come from `ui::probe`;
   clicks and keys go through gpui's own dispatch; the backend records
   commands instead of performing them.
2. **Change everything that needs changing.**
3. **Render once, look at every screen.**

       cargo run --release --bin preview -- target/preview

   One PNG and one `.json` layout dump per pose per appearance, in about ten seconds.
   The window is shown at (80,80) for the duration and never takes focus.

## `probe.py` — measuring a picture

Answers questions about *pixels* — the things the layout engine cannot
see, like whether two runs of text share a baseline after real fonts have
been applied, or whether a corner is round.

    # Do these runs sit on one line? Same top/bottom band = same baseline.
    uv run --with pillow probe.py align target/preview/detail.png --scale 1 \
        --y0 140 --y1 156 --columns 'into:16:36,main:41:66,checks:78:116'

    # Where are the text rows, and are they evenly spaced?
    uv run --with pillow probe.py bands shot.png --y0 40 --y1 200

    # Is that corner actually round?
    uv run --with pillow probe.py corner shot.png --corner tl

    # 4x an area to inspect a hairline or a glyph edge.
    uv run --with pillow probe.py zoom shot.png out.png --x0 300 --y0 8 --w 92 --y1 34

The preview writes a `.json` beside each PNG; `probe.py` reads the row
width from it to learn the capture's backing scale. Otherwise pass
`--scale 1` for a 1x display, `--scale 2` for Retina.
