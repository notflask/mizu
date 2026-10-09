# mizu: plan 2 (finish the job)

This is the second implementation plan for mizu. `docs/PLAN.md` (German) is the original spec and
stays valid for everything this document does not change. Where the two disagree, **this
document wins** (for example, EPUB is no longer a non-goal).

It is written for a coding agent (Claude Sonnet 5.5). Read it fully before you start.

---

## 0. Rules for the implementing agent

- Work **milestone by milestone** in the order of section 3. At the end of every milestone run
  `cargo fmt --all`, `cargo clippy --workspace --all-targets --profile fast -- -D warnings` and
  `cargo test --workspace --profile fast`. Commit only when everything is green. Make one commit per
  finished step. Work on a branch (`plan2/<milestone>`), never directly on `master`.
- **Check crate APIs on docs.rs (or in `~/.cargo/registry/src`) before you use them.** The pinned
  versions are winit 0.30.13, wgpu 30, glyphon 0.12, mupdf 0.8.0, objc2 0.6, objc2-app-kit 0.3.
  Do not write winit/objc2 code from memory.
- Never `unwrap()` on user input, files or documents. Errors go to the status line.
- Performance rules from `docs/PLAN.md` §19 still apply: no allocation per frame on the hot path,
  zero CPU when idle, nothing blocking on the UI thread. No milestone may make the numbers in
  `docs/TESTING.md` worse.
- Code and comments in English. Match the existing style: short doc comments, small modules,
  comments only where the *why* is not obvious.
- **Diagnose before you fix.** Two items below (§4.1, §4.2) are bug reports from the user. The
  plan names the likely causes; reproduce each one first and confirm the cause. If the cause turns
  out different, fix the real cause and write down what it was in the commit message.
- You can drive the real window without a human: `MIZU_TEST_SCRIPT=script.txt` (commands such as
  `key jjD`, `move 300 200`, `down left`, `up left`, `capture out.ppm`, `settle`; see
  `src/app.rs`, `Script`). Use it for end-to-end checks and add new script commands when you need
  them (for example `wheel 0 -3`).
- Keep `README.md` and `docs/TESTING.md` up to date in the same commit as the feature.
- Do not invent features beyond this plan. When something is unclear and blocks you, stop and
  write the question into the final report instead of guessing.

---

## 1. Where the project stands

Done and working (do not rewrite): tiled GPU renderer, dark-mode shader, Vim keymap engine,
command line, search, outline, links, marks, jump list, ink with SDF rendering, saving ink as
`/Ink` annotations, undo/redo, auto-reload, session restore, password prompt, Wayland pen and
pinch backend (`src/platform/wayland.rs`), Nix flake, icons, CI, release workflow (Linux tarball,
Windows zip, macOS dmg).

Key files:

| Area | Files |
|---|---|
| Window, event loop, frame pacing | `src/app.rs` |
| Viewer state, tick/animation | `src/viewer/mod.rs`, `src/viewer/actions.rs` |
| Keyboard → keys | `src/input/keys.rs` (`Key::from_winit`), `src/input/keymap.rs` |
| `:` command line | `src/viewer/input.rs` (`on_key_line`), `src/input/command.rs` |
| Status line, overlays | `src/viewer/status.rs`, `src/render/ui.rs` |
| Pen | `src/viewer/draw.rs`, `src/ink/*` |
| Renderer | `src/render/mod.rs`, `src/render/shaders/*.wgsl` |
| Documents, workers | `src/doc/mod.rs`, `src/doc/worker.rs`, `src/doc/service.rs`, `src/doc/annots.rs` |
| Platform input | `src/platform/mod.rs`, `src/platform/wayland.rs` |
| Packaging | `packaging/`, `nix/`, `.github/workflows/release.yml` |

---

## 2. Carried over from PLAN.md (not done yet)

Verify each item against the code before you start it; some may be partly done.

| # | Item | Where it goes |
|---|---|---|
| C1 | **macOS pen pressure and eraser end.** winit does not turn tablet events into `Touch` on macOS, so there is no pressure there today. `objc2`/`objc2-app-kit` are already dependencies but unused. | M6 |
| C2 | **X11 backend**: XInput 2.4 pinch and pen pressure. `x11rb` is a dependency but unused. | M6 |
| C3 | **Windows eraser end** (`WM_POINTER` + `PEN_FLAG_ERASER`/`PEN_FLAG_INVERTED`). Pressure already comes through winit `Touch`. | M6 (lowest priority) |
| C4 | **macOS: open files from Finder** (double-click, "Open With", drop on the Dock icon). winit 0.30 has no event for this. | M5 |
| C5 | **AppImage** in `release.yml`. | M1 |
| C6 | **Windows MSI** (`cargo-wix`) in `release.yml`. | M8 (last, may be skipped if time runs out; say so in the report) |
| C7 | **Live stroke without per-frame allocation.** `Renderer::live_instances` builds a new `Stroke` with `to_vec()` on every frame while drawing. Append only the new segments to the live buffer. | M2 |
| C8 | **`wgpu::PipelineCache`** stored in the cache dir where the backend supports it (Vulkan). | M8 |
| C9 | **README is out of date**: it says the Wayland tablet and pinch protocols are not wired up, but `platform/wayland.rs` does that. | M1 |
| C10 | **Real-hardware numbers** in `docs/TESTING.md` (§19.1 of PLAN.md) for Linux (Niri, Hyprland) and macOS. | M8 |

---

## 3. Milestones

| # | Content | Done when … |
|---|---|---|
| **M1** | One install script for Linux and macOS (§5), shared macOS bundling script, AppImage (C5), desktop/plist updates, README fix (C9) | On a fresh Ubuntu, Fedora, Arch, NixOS and macOS machine, `./scripts/install.sh` builds and installs mizu, it shows up in the launcher / Launchpad with its icon, `mizu file.pdf` works from a new terminal, and `--uninstall` removes every file it installed |
| **M2** | Bug fixes: brush colour and key handling on non-US layouts (§4.1), scroll stutter (§4.2), live-stroke buffer (C7) | The repro steps in §4.1 and §4.2 pass; regression tests exist |
| **M3** | Command line: suggestions, ghost text, Tab completion, cursor editing (§6) | All behaviour in §6 works; unit tests for the completion engine |
| **M4** | QoL improvements (§7) | Every item in §7 works and is in the README |
| **M5** | macOS title bar (§8) and opening files from Finder (C4) | Looks like §8 describes, on light and dark; drag/double-click on the bar behave like a native window; Finder opens files in mizu |
| **M6** | Pen and pinch on the remaining platforms: macOS (C1), X11 (C2), Windows eraser (C3) | macOS: pressure changes the width and the eraser end erases. X11: pinch zooms and pressure works. Without a tablet nothing changes |
| **M7** | EPUB reading (§9) | §9's acceptance list passes |
| **M8** | Optimisation and release pass: C6, C8, C10, binary size check with EPUB, final README/TESTING update | All numbers from PLAN.md §19.1 measured and written down; CI green on all three OSes |

---

## 4. Bug fixes (M2)

### 4.1 Brush colour does not change (light theme)

**User report:** "With the white theme the brush has one colour and it does not change."

The colour path itself looks right when reading the code: `SelectColor(n)` sets
`Viewer::pen_color` (`src/viewer/actions.rs`), `live_stroke()` and `pen_up()` use it, and
`recolor()` is the identity in light mode. So the most likely problem is that the **keys never
reach `SelectColor`**, or the user cannot find out how to change the colour at all.

Reproduce first:
1. Headless: a test script `key i`, `key 2`, drag with the left button, `capture`, then check that
   the captured stroke pixels are red (`#e03131`), not near-black. Do the same with `:color #1971c2`.
2. Add a `tests/viewer.rs` test that feeds the keys `i`, `2` through `Viewer::on_key` and asserts
   `pen_color == palette[1]`.
3. Then test with real keyboard layouts the user is likely to have: **German (QWERTZ) on macOS and
   Linux, and Ukrainian**. Log every key with `RUST_LOG=mizu=debug` (add a debug log in
   `App::window_event` for `KeyboardInput`: logical key, `key_without_modifiers`, physical key,
   modifiers).

Likely causes, in order. Fix all that apply:

- **Option / AltGr keep the `alt` modifier.** `Key::from_winit` keeps `alt` for characters. On a
  German Mac, `[` and `]` are `Option+5` / `Option+6`, so they arrive as `<A-[>` and never match the
  `[` / `]` bindings (width). The same breaks `{`, `@`, `~`, `|` on many layouts.
  Fix: in `app.rs`, use `winit::platform::modifier_supplement::KeyEventExtModifierSupplement::key_without_modifiers()`.
  If the logical character differs from the unmodified one and Ctrl is not held, the character was
  *produced* by Option/AltGr, so drop `alt` (the same way `shift` is already dropped for
  characters). Keep `alt` when the character is unchanged, so real `<A-x>` bindings still work.
  Unit-test the normalisation with a small pure function (do not depend on winit types in tests).
- **Non-Latin layouts.** With a Ukrainian (or Russian, Greek, …) layout active, `i`, `e`, `p`, `j`,
  `k` produce Cyrillic letters and no binding fires. Fix: in Normal and Draw mode, when a character
  key has no binding and is not ASCII, retry with the Latin letter from the **physical** key
  (`KeyCode::KeyA` → `a`, with Shift → `A`). Text entry modes (`:`, `/`, outline filter) keep the
  real character. Unit test with a Cyrillic key.
- **Discoverability.** Nothing in the UI says that digits pick colours. §7.1 adds a visible palette
  in Draw mode. This is part of the fix.

If none of these reproduces the bug, ask the user (in the final report) for their keyboard layout
and the exact keys they pressed, and leave a `RUST_LOG` hint in the README's troubleshooting
section.

### 4.2 Scrolling stutters at the start, then is smooth (Linux and macOS)

**Confirmed cause (from reading the code):** `Viewer::tick` (`src/viewer/mod.rs`) measures `dt`
from `last_tick`, which is only updated when a frame is drawn. After the window has been idle,
the first frame of a new scroll animation gets `dt` clamped to `0.1 s`, so
`k = 1 - exp(-0.1 * 22) ≈ 0.89`: the **first frame jumps 89 % of the way**, and the rest is
eased. That is exactly "a small jerk, then smooth".

Fix:
1. When an animation starts from rest (`anim_target` was `None`), set `last_tick = now` so the first
   step uses a real frame interval. Also clamp `dt` to at most `1/30 s` in general.
2. Replace the exponential ease with a **critically damped spring** (position + velocity) so that
   consecutive wheel notches and held `j`/`k` (key repeat) keep a continuous velocity instead of
   restarting the curve each time. Keep the feel snappy: a single wheel notch must settle within
   about 120–150 ms. Put the spring in a small pure function and unit-test it (first step after idle
   moves at most ~25 % of the distance at 60 Hz; it converges without overshoot; retargeting keeps
   velocity).
3. Time the animation by the frame, not by the event: compute `now` once per `draw_frame` (already
   so) and do not advance animations from input handlers.

Other things that add to the first-frames hitch. Measure each with `--stats` and the trace log
before and after; only keep changes that help:

- Call `window.pre_present_notify()` right before presenting (winit uses it to throttle redraws to
  Wayland frame callbacks).
- `finish_frame` calls `window.set_cursor` and compares/sets the title on **every** frame. Only call
  them when the value changed.
- `view_complete` and `schedule_tiles` both build a `wanted_tiles` list every frame; make them reuse
  one scratch `Vec` (no allocation per frame).
- Tile uploads: the cap is 8 tiles per frame (8 MB). On integrated GPUs and on Metal this can cost
  a frame at the start of a scroll when the prefetch tiles all land together. Change the cap to a
  **byte budget** (start with 4 MB per frame) and upload visible tiles before prefetch tiles.
- Drivers may compile pipelines lazily on first use. If the trace shows the first frame that
  draws ink/highlights/thumbnails being slow, warm the pipelines once after start-up by drawing
  one instance of each into a 1×1 offscreen target.
- `ui_state()` builds several `String`s every frame. Cache it and rebuild only when `dirty` parts
  of the status changed (the glyphon side already only reshapes on change).

Repro / acceptance:
- Add a `trace`-level log line per frame with frame time and camera offset delta. Scroll with single
  wheel notches after 2 s of idle: the first frame's delta must not be much larger than the second.
- Manual on Niri/Hyprland and macOS (60 Hz and 120 Hz ProMotion): one notch, a fast flick, held
  `j`, touchpad two-finger scroll. No visible jump at the start.

### 4.3 Live stroke buffer (C7)

Keep the instances of the live stroke between frames. On each new point append one instance (two
for the first segment), on `pen_up` clear. With pressure, the last instance changes when the
pressure filter updates, so rewrite only the tail. No `Vec` is allocated per frame while drawing.

---

## 5. Install script (M1)

One entry point: **`scripts/install.sh`**, for Linux and macOS. It builds mizu from the checked-out
sources and installs it for the current user. The user should never have to copy files by hand
again.

Write it in **bash that runs on bash 3.2** (the macOS default): no associative arrays, no
`${var,,}`, no `mapfile`. `set -euo pipefail`. It must pass `shellcheck`.

### 5.1 Interface

```
scripts/install.sh [options]

  --prefix DIR     install under DIR (default: ~/.local on Linux; ~/Applications + ~/.local/bin on macOS)
  --system         install for all users (/usr/local, /Applications); uses sudo only for the copy step
  --uninstall      remove everything a previous run installed (reads the manifest)
  --no-build       install the existing target/release/mizu
  --universal      macOS: build arm64 + x86_64 and lipo them
  --nix            force the Nix path (auto on NixOS)
  --no-nix         force the cargo path even on Nix systems
  --default-pdf    also make mizu the default app for PDF (and EPUB); never done without this flag
  --dry-run        print what would happen
  -h, --help
```

It prints one short line per step and a summary at the end (what was installed where, and whether
`PATH` needs a change). Running it twice is safe (idempotent, it upgrades in place). It writes a
manifest of installed files to `<prefix>/share/mizu/install-manifest` (Linux) or
`~/Library/Application Support/io.github.notflask.Mizu/install-manifest` (macOS); `--uninstall`
removes exactly those files and nothing else.

### 5.2 Linux

1. **NixOS / Nix**: if `/etc/NIXOS` exists or `ID=nixos` is in `/etc/os-release` (or `--nix`), run
   `nix profile install "path:$REPO#mizu"` (or `nix profile upgrade` when an entry for this flake
   exists). That gets the wrapper with the right `LD_LIBRARY_PATH`, the desktop file and the icons.
   A plain `cargo build` binary does **not** run on NixOS (it cannot find Vulkan/Wayland libs), so
   never take the cargo path there unless `--no-nix` is given, and then warn. Print the snippet for a
   declarative install (`overlays.default`) as a hint. Done.
2. **Other distros**: check for `cargo`, `clang`/`libclang`, `pkg-config`, and the dev packages
   (Wayland, xkbcommon, fontconfig, freetype). For anything missing, print the exact install command
   for the detected package manager (`apt`, `dnf`, `pacman`, `zypper`), taken from `/etc/os-release`.
   When stdin is a TTY, offer to run it (`[y/N]`); never run `sudo` without asking. If `cargo` is
   missing, point to https://rustup.rs and stop.
3. `cargo build --release --locked`.
4. Install:
   - `bin/mizu` (mode 755)
   - `share/applications/io.github.notflask.Mizu.desktop`; if `<prefix>/bin` is not on `PATH`,
     rewrite `Exec=` to the absolute path
   - `share/icons/hicolor/...` (copy `assets/icons/generated/linux/hicolor`)
   - `share/metainfo/io.github.notflask.Mizu.metainfo.xml` (new, small AppStream file; optional but
     nice for software centres)
5. Refresh caches if the tools exist: `update-desktop-database`, `gtk-update-icon-cache -f -t`,
   `xdg-desktop-menu forceupdate`. Missing tools are not an error.
6. `--default-pdf`: `xdg-mime default io.github.notflask.Mizu.desktop application/pdf application/epub+zip`.

### 5.3 macOS

1. Check `xcode-select -p` (Command Line Tools; offer `xcode-select --install`) and `cargo`.
2. Build for the native arch (`--universal`: `rustup target add` both targets, build both, `lipo`).
3. Make the bundle with a **new shared script `packaging/macos/make-app.sh <binary> <version> <out.app>`**
   (Info.plist with the version from `Cargo.toml`, `mizu.icns`). Change `release.yml` to use the same
   script, so local and CI bundles never drift apart.
4. Ad-hoc sign: `codesign --force --deep --sign - mizu.app`.
5. Install to `~/Applications/mizu.app` (`--system`: `/Applications`). Before replacing an existing
   `mizu.app`, check that its `CFBundleIdentifier` is `io.github.notflask.Mizu`; otherwise stop.
6. `xattr -dr com.apple.quarantine` on the app (harmless if not set), then register it:
   `/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f <app>`.
7. Command line: install a tiny wrapper script at `~/.local/bin/mizu` (`--system`:
   `/usr/local/bin/mizu`) that does `exec "<app>/Contents/MacOS/mizu" "$@"`. Use a wrapper, not a
   symlink, so the process runs from inside the bundle (Dock icon and bundle identity work).
8. `--default-pdf`: only if `duti` is installed (`duti -s io.github.notflask.Mizu com.adobe.pdf viewer`);
   otherwise print how to do it in Finder ("Get Info → Open with → Change All").

### 5.4 Packaging changes that go with it

- `packaging/linux/io.github.notflask.Mizu.desktop`: `MimeType=application/pdf;application/epub+zip;`
  (EPUB lands in M7; add the MIME type then, not before).
- `packaging/macos/Info.plist`: add a document type for `org.idpf.epub-container` in M7.
- **AppImage (C5)** in `release.yml`: build on the oldest supported Ubuntu LTS runner, use
  `linuxdeploy` + `appimagetool`, do **not** bundle `libvulkan`, `libwayland-client`,
  `libxkbcommon`, `libGL` (host libraries).
- CI: a job that runs `scripts/install.sh --prefix "$RUNNER_TEMP/p"` and then `--uninstall` on
  `ubuntu-latest` and `macos-latest`, and checks the prefix is empty afterwards. Run `shellcheck`.
- README: replace the "From source" steps with `./scripts/install.sh`, keep the manual steps below
  it for people who want them.

---

## 6. Command line: suggestions and Tab completion (M3)

Today `:` is a plain text field; `Tab` only completes paths for `:e`/`:w`. Goal: typing a command
shows what is possible, and `Tab` finishes it, like fish/zsh and like Vim's `wildmenu`.

### 6.1 Command registry

Replace the hard-coded `match` in `src/input/command.rs` with a static table that is the single
source of truth for parsing, completion, the suggestion list and `:help`:

```rust
struct CommandSpec {
    name: &'static str,          // "write"
    short: &'static str,         // "w" (minimum abbreviation, Vim style)
    bang: bool,                  // accepts "!"
    arg: ArgKind,                // None, Path { filter }, Color, Number, Page, Choice(&[..])
    help: &'static str,          // "write the file (with ink)"
}
```

Every existing command goes into the table (`w`, `q`, `wq`, `x`, `e`, `<N>`, `dark`, `light`,
`color`, `width`), plus the new ones from §7 and §9. Parsing keeps all current behaviour and error
messages (`E492`, `E474`); the existing tests must pass unchanged.

### 6.2 Behaviour

- **Suggestion popup.** While in Command mode, a small list (max 8 rows) sits directly above the
  status line, left-aligned with the `:`. Each row: the command (matched part bold or brighter),
  its argument hint (`{file}`, `#rrggbb`, `{pt}`), and the help text in grey. Same colours as the
  status line (black/white in dark mode). It is drawn with the existing UI pipeline (`render/ui.rs`),
  like the outline overlay, but anchored bottom-left instead of centred.
- **What is suggested**:
  - before the first space: command names, prefix matches first, then subsequence ("fuzzy")
    matches; aliases count (`wr` → `write`);
  - after `e `/`w `: files and directories (reuse `complete()`; `:e` shows `.pdf` and, after M7,
    `.epub`; dot-files only when the prefix starts with `.`); directories end with `/`;
  - after `color `: the palette entries with a colour swatch and their number, plus the colour
    names from §7.2;
  - after `width `: nothing (just the hint);
  - empty line: the most recent history entries first, then all commands.
- **Ghost text.** The best suggestion's missing part is shown after the cursor in grey (fish
  style). `→` or `<C-e>` at the end of the line accepts it.
- **Tab**: first press completes to the longest common prefix of all candidates. If that adds
  nothing, it selects the first candidate and inserts it; further `Tab` / `<S-Tab>` cycle forward /
  backward through the list (the selected row is highlighted). Any other key keeps the inserted
  text and continues editing. A completed directory continues into the directory on the next Tab.
- **Cursor editing** (needed for all of the above): the line gets a cursor index. `←`/`→`,
  `<Home>`/`<End>`, `<C-a>`/`<C-e>` (when there is no ghost text to accept), `<C-b>`/`<C-f>`,
  `<Del>`, `<C-w>`, `<C-u>` (delete to start), `<BS>` at the cursor. Work on `char` boundaries
  (UTF-8 paths). The same editing applies to `/` search.
- `<Up>`/`<Down>` keep walking the history, filtered by what is typed before the cursor (like Vim).
- Suggestions never touch the disk on the UI thread for more than a directory listing; cache the
  listing of the current directory while the popup is open.

### 6.3 Code layout and tests

- `src/input/complete.rs` (new): pure functions, `fn suggest(line: &str, cursor: usize, ctx: &Ctx) -> Suggestions`
  where `Ctx` gives history, palette and a directory-listing callback (so tests do not need a real
  file system except for the existing tempdir test). Returns candidates, the common prefix and the
  ghost text.
- `Viewer` holds a `CompletionState { items, selected, base_line }`, rebuilt only when the line
  changes.
- Unit tests: prefix and fuzzy order, alias match, common-prefix Tab, Tab cycling and `<S-Tab>`,
  ghost text, path completion with spaces and `~`, colour suggestions, history filter, UTF-8 cursor
  movement.

---

## 7. QoL improvements (M4)

Small things that make daily use nicer. Each is in the README's key/command tables.

### 7.1 Visible palette in Draw mode

In Draw mode the right side of the status line shows the palette as small swatches with their
digit (`1● 2● 3● …`), the current colour framed, then the width (`1.5pt`) or `eraser`. On colour
change, a short message `colour 2  #e03131`. Swatches are drawn in the original colour in light
mode and recoloured like ink in dark mode, so they match what is drawn.

### 7.2 Friendlier `:color` and `:width`

- `:color` accepts `#rgb`, `#rrggbb`, a palette number (`:color 3`) and names: `black`, `white`,
  `red`, `blue`, `green`, `orange`, `purple`, `yellow`, `grey`. Names map to the default palette
  colours where one exists.
- `:width` without an argument shows the current width.
- Selecting a colour with a digit while the eraser is active switches back to the pen (already so);
  make `e` show `eraser` / `pen` as a message too.

### 7.3 Remember pen settings

Pen colour and width are remembered across runs (global, in the session file, not per document).
Old session files without these fields still load (`#[serde(default)]`); add a test.

### 7.4 Cursor ring visible on any page

The Draw-mode cursor ring uses the pen colour. Draw a thin contrasting outline around it (white in
dark mode, black in light mode, about 0.5 px) so a dark pen on a dark area or a white pen on white
stays visible.

### 7.5 `:help` overlay

`:help` (and `<F1>`) opens a list overlay (reuse the outline overlay with its filter) with every
key binding of the current mode (taken from the live keymap, so config overrides show) and every
command from the registry with its help text. `<Esc>` closes it.

### 7.6 Recent files

`:recent` (and the empty start screen) lists the most recently opened files from the session store
(only files that still exist), with their last page. `j`/`k`/Enter opens one, typing filters. The
start screen without a file says `mizu — :e <file>   ·   :recent`.

### 7.7 Smaller items

- `<C-d>`/`<C-u>`, `J`/`K`, `gg`/`G` use the new spring from §4.2 (they already animate; make sure
  they still do and feel the same).
- Window title shows `[+]` when there is unsaved ink: `skript.pdf [+] — mizu`.
- Remember the window size (logical) in the session and restore it on start (Wayland ignores the
  position anyway; do not store a position).

---

## 8. macOS title bar (M5)

Today the window has the default macOS title bar (grey bar, centred title). Goal: a clean,
unified look like Preview, Notes or Zed, where the content goes up to the top edge and the traffic
lights float over it. **Linux stays exactly as it is** (the user's Niri/Hyprland setup draws no
decorations, or its own). All code in this section is `#[cfg(target_os = "macos")]`.

### 8.1 Window attributes

In `App::resumed` (`src/app.rs`), with `winit::platform::macos::WindowAttributesExtMacOS`:

```rust
attrs = attrs
    .with_titlebar_transparent(true)
    .with_title_hidden(true)
    .with_fullsize_content_view(true);
```

Keep `with_title(...)`: the title is still used by Mission Control, the Window menu and
accessibility, it is just not drawn. Option keeps producing characters (do not set
`OptionAsAlt`; §4.1 handles Option-produced characters).

### 8.2 The title strip

- The top `T` logical pixels are the title strip. Get `T` from AppKit (`NSWindow.frame` height minus
  `contentLayoutRect` height, via objc2 on the `NSView` from the raw window handle); fall back to
  `28.0` if that fails. Recompute on resize, scale change and full-screen change.
- mizu draws the strip itself: a solid bar in the window background colour (dark mode `#000000`,
  light mode the current gap grey) with a 1 px hairline at the bottom (`#1a1a1a` in dark,
  `#c4c4c4` in light). Pages scroll **under** the strip and are clipped by it, like Preview.
- In the strip, centred: the file name in the status-line font, grey (`#8a8a8a`), plus a small `●`
  when there is unsaved ink. Nothing else; no buttons.
- The camera gets a top inset of `T`: `gg` puts the first page right below the strip, and
  "current page", fit-page and centring all use the viewport minus the strip. Implement the inset
  generically (a `top_inset` field in the camera, 0 on other platforms) and unit-test fit-page and
  clamping with an inset.
- Native full screen: the strip disappears (`T = 0`) while in full screen; the traffic lights
  appear on hover as usual. Check how winit reports native full screen on macOS
  (`Window::fullscreen()`), and update on `Resized`.

### 8.3 Behaviour of the strip

- Left-drag in the strip moves the window: call `window.drag_window()` on mouse-down there.
  The strip never starts panning or drawing.
- Double-click in the strip does what the system setting says
  (`NSUserDefaults` `AppleActionOnDoubleClick`: `Maximize` → zoom, `Minimize` → miniaturise,
  `None` → nothing; default zoom). Use `NSWindow` `performZoom:` / `performMiniaturize:` via
  objc2.
- Links, pen and the text cursor ignore the strip area.

### 8.4 Appearance

- Follow mizu's dark mode, not the system: on `D` / `:dark` / `:light`, call
  `window.set_theme(Some(Theme::Dark | Theme::Light))`, so the traffic lights and any system UI
  (window menu, tooltips) match the page colours.
- The window background (`NSWindow.backgroundColor`) matches the strip colour, so live resizing
  does not flash grey.
- Keep the system's rounded corners and shadow.

### 8.5 Open files from Finder (C4)

Install an Apple Event handler for `kAEOpenDocuments` (`'aevt'`/`'odoc'`) with
`NSAppleEventManager` via objc2 (this does not replace winit's application delegate). Install it
before the event loop starts processing (`new_events` with `StartCause::Init`), collect the file
URLs, and send them to the app through the `EventLoopProxy` as a new `UserEvent::OpenFile(PathBuf)`,
which goes through the same path as `:e` (with the dirty check). Test: double-click a PDF in Finder
with mizu closed and with mizu open; drop a PDF on the Dock icon.

Acceptance for M5: screenshots of light and dark mode in `docs/screenshots/macos-*.png`; resize,
full screen, drag, double-click and Finder open checked on a real Mac.

---

## 9. EPUB (M7)

EPUB was a non-goal in PLAN.md. It is now a goal: mizu opens `.epub` files and reads them with the
same keys, dark mode, search, outline and links as PDFs.

### 9.1 Engine

- MuPDF can lay out EPUB itself. Enable the `epub` feature of the `mupdf` crate in `Cargo.toml`
  (it also turns on MuPDF's HTML engine). Do **not** turn on `js`, `xps`, `svg`, `cbz`, `img`,
  `tesseract` or bundled font packs. Measure the release binary before and after and write both
  numbers into the README; it must stay below 30 MB.
- `mupdf::Document` gives what is needed: `open`, `is_reflowable`, `layout(w, h, em)`,
  `page_count`, `load_page`, `outlines`, `resolve_link`, and `Page::search` / links. If you need
  `fz_make_bookmark` / `fz_lookup_bookmark` (for keeping the reading position across re-layout),
  add `mupdf-sys` with the same version as a direct dependency and wrap the calls in a small safe
  function in `src/doc/ffi.rs`; all `unsafe` stays in that file.

### 9.2 Opening

- `open_pdf` becomes `open_document`, returning a kind: `Pdf(PdfDocument)` or
  `Reflow(Document)`. Detect EPUB by content (zip with `mimetype` = `application/epub+zip`) and by
  extension. Unknown formats still give a clear error.
- Every place that assumes PDF (annots loading/stripping, saving, the worker's per-page annotation
  strip) checks the kind. Go through `src/doc/*.rs` and `src/viewer/*.rs` for `PdfDocument` uses.

### 9.3 Layout

- A reflowable document is laid out to a virtual page: config
  `[epub] page_width = 480, page_height = 680, font_size = 11` (points; the defaults give a
  comfortable book page at fit-width on a laptop). The page is then shown like any PDF page: tiles,
  zoom, dark mode, all unchanged.
- **Every worker** opens its own `Document` and must call `layout` with exactly the same
  parameters before rendering. Add a `layout_epoch` to render jobs; workers re-layout when the epoch
  changes and drop jobs from older epochs (same idea as `generation`).
- Laying out a large book can take seconds. It must never run on the UI thread: the first worker
  lays out and reports the page count and sizes; the window shows `Laying out…` until then.
- `:fontsize <pt>` changes `font_size` and re-lays out. The reading position is kept with MuPDF
  bookmarks (or, as a fallback, the fraction through the book). `<C-+>`/`<C-->` are **not**
  added; zoom stays zoom.
- Window resize does not re-layout (pages are virtual, like PDF pages).

### 9.4 What works and what does not

- Works: scrolling, zoom, fit modes, dark mode, search (`/`, `n`, smartcase), outline (`o`),
  internal links and external links, marks, jump list, session restore (position stored as page +
  offset together with the layout parameters; if those changed, use the bookmark/fraction),
  auto-reload, `:recent`.
- **No ink in EPUB.** `i` shows `Drawing works only in PDFs`; `:w` shows
  `E: EPUB files are read-only`; `:q` never complains about unsaved changes. (The user may ask for
  EPUB ink later; then it needs a sidecar file, which is out of scope here.)
- Completion (§6), drag and drop, the desktop file and Info.plist accept `.epub`.

### 9.5 Tests

- A tiny EPUB fixture generated in the test helper (`src/testutil.rs`): `mimetype` (stored, first),
  `META-INF/container.xml`, an OPF, a nav document and two XHTML chapters with a heading and a
  link. Use the `zip` crate as a **dev-dependency** with default features off (store only).
- Integration tests in `tests/pdf_roundtrip.rs` (or a new `tests/epub.rs`): opens, page count > 0,
  a smaller page height gives more pages, search finds a word from chapter 2, the outline has both
  chapters, the internal link resolves, a rendered tile is not blank, `:w` is refused, `i` is
  refused.
- Manual: a real novel and a real textbook EPUB (with images and CSS), in light and dark mode.

---

## 10. Platform input, remaining (M6)

Follow PLAN.md §9.5. Short version of what is left:

### 10.1 macOS (C1)

`NSEvent.addLocalMonitorForEventsMatchingMask` for `LeftMouseDown/Dragged/Up`, `TabletPoint`,
`TabletProximity`. For events with `subtype == NSEventSubtypeTabletPoint` read `pressure` and,
from the last proximity event, `pointingDeviceType == NSPointingDeviceTypeEraser`. Convert the
location to window logical coordinates (y flipped). Emit `PlatformEvent::Pen*` through the existing
`Emit`, and **swallow** the matching mouse event (return `nil` from the monitor block) so the stroke
is not drawn twice; non-tablet mouse events pass through unchanged. Put it in
`src/platform/macos.rs`, wired in `platform::init`. Keep the block alive in the backend struct.

### 10.2 X11 (C2)

`src/platform/x11.rs` with `x11rb` (XInput 2.4): select `XI_GesturePinchBegin/Update/End` and, on
tablet devices, motion with the valuator labelled `Abs Pressure` (look up the atom). Only when
running on X11 (check the raw window handle). Its own connection and a thread, like the Wayland
backend. Fall back silently if XI 2.4 is missing.

### 10.3 Windows eraser (C3)

Subclass the window (`SetWindowSubclass`), on `WM_POINTERDOWN/UPDATE/UP` with `PT_PEN` read
`GetPointerPenInfo` for `PEN_FLAG_ERASER` / `PEN_FLAG_INVERTED` and forward pressure. Then pass the
message on to winit. Lowest priority of M6.

---

## 11. Final pass (M8)

- C6 MSI, C8 pipeline cache.
- C10: measure on the user's real machines (Linux with Niri and Hyprland, a Mac) every number in
  PLAN.md §19.1, plus "first frame of a scroll after idle" from §4.2, and write them into
  `docs/TESTING.md` with date and commit. Fill in the platform table there.
- Binary size with EPUB enabled, checked in CI (the existing 30 MiB check).
- README: install section (script), macOS look, EPUB, new keys and commands, troubleshooting for
  keyboard layouts. Remove the "EPUB" line from non-goals in PLAN.md with a note pointing here.

---

## 12. Final report

When done (or blocked), write a short report: what was done per milestone, what was measured,
what was skipped and why, and any open question for the user (for example the keyboard layout if
§4.1 did not reproduce).
