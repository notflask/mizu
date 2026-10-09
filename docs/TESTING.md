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

| Platform | Compositor / driver | Result |
| --- | --- | --- |
| Linux, Wayland | Niri, Intel Iris Xe, Vulkan | **black window reported, being investigated** |
| Linux, Wayland | Hyprland | not tested yet |
| Linux, X11 | | not tested yet |
| Windows 11 | | compiles in CI, not run |
| macOS | | compiles in CI, not run |
