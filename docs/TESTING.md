# Testing

What has been checked automatically, what has been checked by hand, and what still needs a human
on real hardware. Dates are when I last did it; "headless" means a compositor without a monitor
(Weston or Sway with the headless backend) and the software Vulkan driver (lavapipe).

## Automatic

`cargo test` runs about 135 tests:

- unit tests for the key parser, key map, commands, config, session store, layout and camera
  maths, stroke smoothing, eraser, undo history, and the CPU reference of the dark-mode colour
  function;
- `tests/pdf_roundtrip.rs`: ink saved into real PDFs (also rotated pages) and read back, other
  people's annotations stay untouched, failed saves keep the original, search, links, outline,
  tile rendering;
- `tests/viewer.rs`: the viewer without a window: scrolling, zoom, marks, `:w` / `:q` rules,
  external changes, search, the command line (suggestions, Tab, editing, history), keyboard
  layouts, palette colours, help and recent lists;
- `tests/epub.rs`: a generated EPUB: layout and re-layout, outline, search, links, tiles, and the
  read-only rules in the viewer.

CI (`.github/workflows/ci.yml`) builds and tests on Linux, Windows and macOS.

## End to end, headless (Sway 1.9 and Weston, lavapipe, 2026-10)

`MIZU_TEST_SCRIPT` drives the real window and takes screenshots of it. Checked this way: open,
scroll, zoom, dark and light, search with highlights, outline overlay, password prompt, drawing with undo/redo,
erasing, `:w` and reload, output scales 1, 1.25 and 2, the Wayland pen/pinch backend binding
(`tablet-v2` and `pointer-gestures` both offered by Sway), and the Vulkan validation layer
(no messages).

Measured numbers (software renderer on a small cloud VM, so only the CPU side is meaningful):

| What | Result |
| --- | --- |
| render one 512x512 tile (MuPDF, CPU) | about 1.2 ms |
| load the page list of a 300-page PDF | 7 ms |
| save 1000 strokes into a PDF | 125 ms |
| hit-test the eraser against 10 000 strokes | 21 us |
| CPU use while idle | 0 (no timers, no polling) |
| release binary | 16 MB; 19.1 MB with EPUB support (2026-10-09) |

## Plan 2 (2026-10-09, headless Sway + lavapipe, and Xvfb)

Checked with `MIZU_TEST_SCRIPT` screenshots: palette colours change the ink in light mode
(`i`, `2`, draw, `3`, draw: red then blue strokes), the palette strip and the suggestion list of the
command line, the help overlay, EPUB books in light and dark mode and after `:fontsize`, drawing
refused in EPUBs, and the title strip (simulated with `MIZU_TITLEBAR_INSET=28`) in both themes.
Under Xvfb the X11 backend starts (`x11: 0 tablet tool(s), gestures yes`).

The macOS and Windows platform code (`src/platform/macos.rs`, `src/platform/windows.rs`, and the
macOS calls in `app.rs` / `main.rs`) is type-checked and linted for `aarch64-apple-darwin` and
`x86_64-pc-windows-msvc` in a small crate that includes those files; it has not run on a Mac or a
Windows machine yet.

| What | Result |
| --- | --- |
| first frame of a wheel notch after 1.5 s idle | moves 17 % of the notch (5.4 of 32.5 pt), then 8.3, 6.8, 4.7 … (before: 89 % in the first frame) |
| frame time while scrolling (lavapipe, CPU only) | 3–8 ms |
| release binary with EPUB | 19.1 MB (limit 30 MB) |

## Manual checklist on real hardware

Not everything can be tested without a screen, a GPU and a pen. Please tick these off on each
platform before calling a release good. `mizu --diag file.pdf` (see the README) helps when
something is black.

- [ ] The page shows up at once, in light and dark mode (`D`).
- [ ] Scrolling with the wheel and the touchpad is smooth; `Ctrl` + wheel and pinch zoom keep the
      point under the cursor fixed; text is sharp after zooming has settled.
- [ ] HiDPI / fractional scale (1.25, 1.5, 2): text and the status line are sharp, not blurry.
- [ ] Resizing the window and moving it between monitors with different scales.
- [ ] Search (`/`, `n`, `N`), outline (`o`), links, marks (`m`, `'`), `:N` to jump to a page.
- [ ] Draw (`i`), change colour and width, erase, undo and redo, `:w`, reopen: ink is still there
      and the PDF opens in another viewer with the ink visible.
- [ ] Pen: pressure changes the width; the eraser end of the pen erases.
- [ ] Editing the PDF in another program (for example re-running LaTeX) reloads the page.
- [ ] Quit protection: `:q` with unsaved ink refuses, `:q!` does not.
- [ ] Scrolling right after the window was idle starts smoothly (no jump on the first frame), with
      the wheel, held `j`, and the touchpad.
- [ ] Your keyboard layout: `i`, digits `1`–`9` (colour changes, shown in the status line), `[` `]`
      (width), also with a German (QWERTZ) and a Ukrainian layout active.
- [ ] `:` shows suggestions; `Tab` completes and cycles; `→` takes the grey suggestion; `:e ~/`
      lists files.
- [ ] An EPUB opens, reads in light and dark mode, `/` search and `o` outline work, `:fontsize 14`
      keeps the place.
- [ ] `scripts/install.sh` installs, the app shows up in the launcher with its icon, a second run
      upgrades, `--uninstall` removes it.
- [ ] macOS: the title strip (light and dark), dragging and double-clicking it, full screen,
      opening a PDF from Finder (mizu closed and open), pen pressure and the eraser end.
- [ ] X11: touchpad pinch, pen pressure.

| Platform | Compositor / driver | Result |
| --- | --- | --- |
| Linux, Wayland | Niri, Intel Iris Xe, Vulkan | **black window reported, being investigated** |
| Linux, Wayland | Hyprland | not tested yet |
| Linux, X11 | | not tested yet |
| Windows 11 | | compiles in CI, not run |
| macOS | | compiles in CI, not run (title bar, Finder, pen are new) |
