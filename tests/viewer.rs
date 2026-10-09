//! Viewer logic against real PDFs, without a window or a GPU: opening,
//! navigation, marks and jump list, drawing with undo/redo, saving, quit
//! protection, search, outline and password prompts.

use std::path::PathBuf;
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use mizu::config::Settings;
use mizu::input::keys::parse_seq;
use mizu::input::Action;
use mizu::testutil::{write_pdf, Extras, PageSpec};
use mizu::viewer::input::Button;
use mizu::viewer::{LoadPurpose, Tool, UiMode, Viewer};

/// Keep the session file and config out of the real home directory.
fn isolate() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = tempfile::tempdir().expect("tempdir").keep();
        std::env::set_var("HOME", &dir);
        std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
        std::env::set_var("XDG_STATE_HOME", dir.join("state"));
        std::env::set_var("XDG_DATA_HOME", dir.join("data"));
    });
}

fn pages(n: usize) -> Vec<PageSpec> {
    (0..n)
        .map(|i| PageSpec::new(500.0, 700.0, &format!("Page {} hello world", i + 1)))
        .collect()
}

fn sample(dir: &std::path::Path, n: usize) -> PathBuf {
    write_pdf(
        dir,
        "doc.pdf",
        &pages(n),
        &Extras {
            outline: true,
            links: true,
        },
    )
}

fn pump(v: &mut Viewer, what: &str, mut done: impl FnMut(&mut Viewer) -> bool) {
    let end = Instant::now() + Duration::from_secs(15);
    while Instant::now() < end {
        v.poll();
        v.tick(Instant::now());
        if done(v) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for: {what}");
}

fn open(path: &std::path::Path) -> Viewer {
    isolate();
    let mut v = Viewer::new(Settings::default(), None, Arc::new(|| {}));
    v.set_window([1000, 800], 1.0);
    v.open(path.to_path_buf(), LoadPurpose::Open { page: None });
    pump(&mut v, "document to load", |v| v.doc.is_some());
    v
}

fn settle_scroll(v: &mut Viewer) {
    pump(v, "scroll animation", |v| !v.tick(Instant::now()).animating);
}

fn keys(v: &mut Viewer, seq: &str) {
    for k in parse_seq(seq).expect("key sequence") {
        v.on_key(k);
    }
}

#[test]
fn opens_and_fits_width() {
    let dir = tempfile::tempdir().unwrap();
    let v = open(&sample(dir.path(), 5));
    assert_eq!(v.page_count(), 5);
    assert_eq!(v.current_page(), 0);
    // Fit width: the page fills the 1000 px window.
    assert!(
        (v.camera.zoom * 500.0 - 1000.0).abs() < 0.5,
        "zoom {}",
        v.camera.zoom
    );
    assert!(v.title().contains("doc.pdf"));
}

#[test]
fn scrolling_and_page_jumps() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 8));
    let y0 = v.camera.offset[1];
    v.exec(Action::ScrollDown, Some(3));
    settle_scroll(&mut v);
    assert!(v.camera.offset[1] > y0 + 50.0);
    v.exec(Action::ScrollUp, Some(3));
    settle_scroll(&mut v);
    assert!((v.camera.offset[1] - y0).abs() < 1.0);

    v.exec(Action::GotoLast, None);
    assert_eq!(v.current_page(), 7);
    v.exec(Action::GotoFirst, Some(3)); // 3gg: page 3
    assert_eq!(v.top_pos().unwrap().page, 2);
    v.exec(Action::GotoFirst, None);
    assert_eq!(v.top_pos().unwrap().page, 0);

    // The view never leaves the document.
    for _ in 0..40 {
        v.exec(Action::ScrollUp, Some(50));
    }
    settle_scroll(&mut v);
    assert!(v.camera.offset[1] >= -0.01);
}

#[test]
fn jump_list_goes_back_and_forward() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 8));
    v.exec(Action::GotoLast, None); // remembers page 0, lands on 7
    assert_eq!(v.top_pos().unwrap().page, 7);
    v.exec(Action::GotoFirst, Some(4)); // remembers 7, lands on 3
    assert_eq!(v.top_pos().unwrap().page, 3);
    v.exec(Action::JumpBack, None);
    assert_eq!(v.top_pos().unwrap().page, 7);
    v.exec(Action::JumpBack, None);
    assert_eq!(v.top_pos().unwrap().page, 0);
    v.exec(Action::JumpForward, None);
    assert_eq!(v.top_pos().unwrap().page, 7);
    v.exec(Action::JumpForward, None);
    assert_eq!(v.top_pos().unwrap().page, 3);
}

#[test]
fn marks_work_and_unknown_marks_report_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 6));
    v.exec(Action::GotoFirst, Some(2));
    keys(&mut v, "ma"); // set mark a on page 2
    keys(&mut v, "G");
    assert_eq!(v.top_pos().unwrap().page, 5);
    keys(&mut v, "'a");
    assert_eq!(v.top_pos().unwrap().page, 1);
    keys(&mut v, "'z");
    assert!(v
        .message
        .as_ref()
        .map(|m| m.error && m.text.starts_with("E20"))
        .unwrap_or(false));
}

#[test]
fn counts_and_sequences_through_the_key_engine() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 9));
    keys(&mut v, "4G");
    assert_eq!(v.top_pos().unwrap().page, 3);
    keys(&mut v, "gg");
    assert_eq!(v.top_pos().unwrap().page, 0);
    keys(&mut v, "zf");
    assert_eq!(v.camera.mode, mizu::view::ZoomMode::FitPage);
    keys(&mut v, "zw");
    assert_eq!(v.camera.mode, mizu::view::ZoomMode::FitWidth);
    let z = v.camera.zoom;
    keys(&mut v, "+");
    assert!(v.camera.zoom > z * 1.1);
    keys(&mut v, "-");
    assert!((v.camera.zoom - z).abs() / z < 0.01);
}

#[test]
fn zoom_keeps_the_anchor_point() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 4));
    let anchor = [300.0, 250.0];
    let before = v.camera.screen_to_doc(anchor);
    v.zoom_by(1.7, anchor);
    v.zoom_by(0.6, anchor);
    let after = v.camera.screen_to_doc(anchor);
    // Clamping at the document edge may move it, but not in the middle of the page.
    assert!((before[1] - after[1]).abs() < 1.0 || v.camera.offset[1] <= 0.01);
}

fn draw_stroke(v: &mut Viewer, from: [f32; 2], to: [f32; 2]) {
    v.pen_down(from, None, false);
    for i in 1..=10 {
        let t = i as f32 / 10.0;
        v.pen_move(
            [
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t + (t * 6.0).sin() * 3.0,
            ],
            None,
        );
    }
    v.pen_up();
}

#[test]
fn drawing_undo_redo_and_dirty_flag() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 3));
    assert!(!v.is_dirty());
    keys(&mut v, "i");
    assert_eq!(v.mode, UiMode::Draw);
    draw_stroke(&mut v, [100.0, 100.0], [500.0, 160.0]);
    draw_stroke(&mut v, [100.0, 300.0], [600.0, 340.0]);
    assert_eq!(v.doc.as_ref().unwrap().ink.total(), 2);
    assert!(v.is_dirty());
    keys(&mut v, "u");
    assert_eq!(v.doc.as_ref().unwrap().ink.total(), 1);
    keys(&mut v, "u");
    assert!(
        !v.is_dirty(),
        "undoing everything returns to the saved state"
    );
    keys(&mut v, "<C-r>");
    assert_eq!(v.doc.as_ref().unwrap().ink.total(), 1);
    assert!(v.is_dirty());
    keys(&mut v, "<Esc>");
    assert_eq!(v.mode, UiMode::Normal);
}

#[test]
fn eraser_removes_strokes_as_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 2));
    keys(&mut v, "i");
    draw_stroke(&mut v, [100.0, 200.0], [500.0, 200.0]);
    draw_stroke(&mut v, [100.0, 215.0], [500.0, 215.0]);
    assert_eq!(v.doc.as_ref().unwrap().ink.total(), 2);
    keys(&mut v, "e");
    assert_eq!(v.tool, Tool::Eraser);
    // One drag across both strokes.
    v.pen_down([300.0, 150.0], None, true);
    v.pen_move([300.0, 280.0], None);
    v.pen_up();
    assert_eq!(v.doc.as_ref().unwrap().ink.total(), 0);
    keys(&mut v, "u");
    assert_eq!(v.doc.as_ref().unwrap().ink.total(), 2);
    keys(&mut v, "<C-r>");
    assert_eq!(v.doc.as_ref().unwrap().ink.total(), 0);
}

#[test]
fn mouse_drag_draws_and_right_button_erases() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 2));
    keys(&mut v, "i");
    v.on_cursor_moved([200.0, 200.0]);
    v.on_mouse_button(Button::Left, true);
    for i in 0..20 {
        v.on_cursor_moved([
            200.0 + i as f32 * 15.0,
            200.0 + (i as f32 * 0.5).sin() * 20.0,
        ]);
    }
    v.on_mouse_button(Button::Left, false);
    assert_eq!(v.doc.as_ref().unwrap().ink.total(), 1);
    // Right button erases while held, even with the pen tool active.
    v.on_cursor_moved([300.0, 200.0]);
    v.on_mouse_button(Button::Right, true);
    v.on_cursor_moved([320.0, 215.0]);
    v.on_mouse_button(Button::Right, false);
    assert_eq!(v.doc.as_ref().unwrap().ink.total(), 0);
    assert_eq!(v.tool, Tool::Pen);
}

#[test]
fn save_writes_ink_into_the_pdf_and_reload_finds_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path(), 3);
    let mut v = open(&path);
    keys(&mut v, "i");
    draw_stroke(&mut v, [100.0, 100.0], [500.0, 160.0]);
    keys(&mut v, "<Esc>:w<CR>");
    pump(&mut v, "save to finish", |v| !v.is_dirty());
    assert!(v
        .message
        .as_ref()
        .map(|m| m.text.contains("written"))
        .unwrap_or(false));
    // A fresh viewer sees the stroke.
    let again = open(&path);
    assert_eq!(again.doc.as_ref().unwrap().ink.total(), 1);
}

#[test]
fn writing_a_copy_leaves_the_original_and_the_dirty_flag_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path(), 2);
    let original = std::fs::read(&path).unwrap();
    let mut v = open(&path);
    keys(&mut v, "i");
    draw_stroke(&mut v, [100.0, 100.0], [400.0, 130.0]);
    let copy = dir.path().join("copy.pdf");
    v.execute_command(&format!("w {}", copy.display()));
    let c = copy.clone();
    pump(&mut v, "copy to appear", move |_| c.exists());
    pump(&mut v, "save reply", |v| {
        v.message
            .as_ref()
            .map(|m| m.text.contains("written"))
            .unwrap_or(false)
    });
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(v.is_dirty(), "the open file still has unsaved ink");
}

#[test]
fn quitting_with_unsaved_ink_is_refused_like_in_vim() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 2));
    keys(&mut v, "i");
    draw_stroke(&mut v, [100.0, 100.0], [400.0, 130.0]);
    keys(&mut v, "<Esc>");
    v.execute_command("q");
    assert!(!v.quit);
    assert!(v
        .message
        .as_ref()
        .map(|m| m.error && m.text.starts_with("E37"))
        .unwrap_or(false));
    // Closing the window: first attempt warns, second one within 3 s quits.
    v.request_close();
    assert!(!v.quit);
    v.request_close();
    assert!(v.quit);

    let mut v2 = open(&sample(dir.path(), 2));
    keys(&mut v2, "i");
    draw_stroke(&mut v2, [100.0, 100.0], [400.0, 130.0]);
    v2.execute_command("q!");
    assert!(v2.quit);
}

#[test]
fn write_quit_saves_then_quits() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path(), 2);
    let mut v = open(&path);
    keys(&mut v, "i");
    draw_stroke(&mut v, [100.0, 100.0], [400.0, 130.0]);
    keys(&mut v, "<Esc>ZZ");
    assert!(!v.quit, "quits only after the write finished");
    pump(&mut v, "ZZ to finish", |v| v.quit);
    assert_eq!(open(&path).doc.as_ref().unwrap().ink.total(), 1);
}

#[test]
fn colon_commands() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 7));
    v.execute_command("dark");
    assert!(v.dark);
    v.execute_command("light");
    assert!(!v.dark);
    keys(&mut v, "D");
    assert!(v.dark);
    v.execute_command("5");
    assert_eq!(v.top_pos().unwrap().page, 4);
    v.execute_command("999");
    assert_eq!(
        v.top_pos().unwrap().page,
        6,
        "out of range goes to the last page"
    );
    v.execute_command("color #ff0000");
    assert_eq!(v.pen_color, [255, 0, 0]);
    v.execute_command("width 3");
    assert_eq!(v.pen_width, 3.0);
    v.execute_command("nonsense");
    assert!(v
        .message
        .as_ref()
        .map(|m| m.text.starts_with("E492"))
        .unwrap_or(false));
}

#[test]
fn command_line_editing_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 7));
    keys(&mut v, ":3<CR>");
    assert_eq!(v.top_pos().unwrap().page, 2);
    keys(&mut v, ":");
    assert_eq!(v.mode, UiMode::Command);
    keys(&mut v, "<Up>");
    assert_eq!(v.line, "3");
    keys(&mut v, "<BS><BS>");
    assert_eq!(
        v.mode,
        UiMode::Normal,
        "backspace on an empty line leaves command mode"
    );
    keys(&mut v, ":xyz<Esc>");
    assert_eq!(v.mode, UiMode::Normal);
}

#[test]
fn search_finds_hits_and_cycles() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 4));
    keys(&mut v, "/hello<CR>");
    pump(&mut v, "search to finish", |v| {
        v.doc.as_ref().map(|d| !d.search.running).unwrap_or(false)
    });
    let d = v.doc.as_ref().unwrap();
    assert_eq!(d.search.hits.len(), 4);
    let first = d.search.current.expect("a current hit");
    keys(&mut v, "n");
    let second = v.doc.as_ref().unwrap().search.current.unwrap();
    assert_eq!(second, (first + 1) % 4);
    keys(&mut v, "N");
    assert_eq!(v.doc.as_ref().unwrap().search.current.unwrap(), first);
    v.refresh_highlights();
    assert!(!v.highlights.is_empty());
    // Escape hides the highlights.
    keys(&mut v, "<Esc>");
    v.refresh_highlights();
    assert!(v.highlights.is_empty());
}

#[test]
fn search_without_hits_reports_e486() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 2));
    keys(&mut v, "/zzzzqq<CR>");
    pump(&mut v, "error message", |v| {
        v.message
            .as_ref()
            .map(|m| m.text.starts_with("E486"))
            .unwrap_or(false)
    });
}

#[test]
fn smartcase_search() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_pdf(
        dir.path(),
        "case.pdf",
        &[PageSpec::new(500.0, 700.0, "Alpha alpha ALPHA")],
        &Extras::default(),
    );
    let mut v = open(&path);
    keys(&mut v, "/alpha<CR>");
    pump(&mut v, "insensitive search", |v| {
        v.doc.as_ref().map(|d| !d.search.running).unwrap_or(false)
    });
    assert_eq!(v.doc.as_ref().unwrap().search.hits.len(), 3);
    keys(&mut v, "/Alpha<CR>");
    pump(&mut v, "sensitive search", |v| {
        v.doc
            .as_ref()
            .map(|d| !d.search.running && d.search.needle == "Alpha")
            .unwrap_or(false)
    });
    assert_eq!(v.doc.as_ref().unwrap().search.hits.len(), 1);
}

#[test]
fn outline_overlay_jumps() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 6));
    keys(&mut v, "o");
    assert_eq!(v.mode, UiMode::Outline);
    let ui = v.ui_state();
    assert_eq!(ui.overlay.as_ref().unwrap().lines.len(), 6);
    keys(&mut v, "jjj<CR>");
    assert_eq!(v.mode, UiMode::Normal);
    assert_eq!(v.top_pos().unwrap().page, 3);
    // Typing filters the list.
    keys(&mut v, "o");
    keys(&mut v, "5");
    let ui = v.ui_state();
    assert_eq!(ui.overlay.as_ref().unwrap().lines.len(), 1);
    keys(&mut v, "<CR>");
    assert_eq!(v.top_pos().unwrap().page, 4);
    // Esc clears the filter first, then closes.
    keys(&mut v, "o5<Esc>");
    assert_eq!(v.mode, UiMode::Outline);
    keys(&mut v, "<Esc>");
    assert_eq!(v.mode, UiMode::Normal);
}

#[test]
fn clicking_links_follows_them() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 3));
    // Links of page 1 are fetched in the background.
    v.doc
        .as_ref()
        .unwrap()
        .service
        .send(mizu::doc::service::Job::Links { pages: vec![0] });
    pump(&mut v, "links", |v| {
        v.doc
            .as_ref()
            .map(|d| d.links.contains_key(&0))
            .unwrap_or(false)
    });
    // The internal link sits at pdf [20 20 120 60] -> page space y 640..680.
    // Bring the lower part of the page into view first.
    v.goto_pos(mizu::viewer::Pos { page: 0, y: 450.0 }, false);
    let s = v.camera.scale();
    let g = v.doc.as_ref().unwrap().layout.pages[0];
    let screen = v.camera.doc_to_screen([g.x + 70.0, g.y + 660.0]);
    assert!(
        screen[1] < 800.0,
        "link is on screen: {screen:?} (scale {s})"
    );
    v.on_cursor_moved(screen);
    assert!(v.hover_link);
    v.on_mouse_button(Button::Left, true);
    v.on_mouse_button(Button::Left, false);
    assert!(v.current_page() >= 1 || v.top_pos().unwrap().page >= 1);
}

#[test]
fn wrong_pages_and_empty_documents_do_not_crash() {
    isolate();
    let mut v = Viewer::new(Settings::default(), None, Arc::new(|| {}));
    v.set_window([800, 600], 1.0);
    // Nothing open: every action is a harmless no-op.
    for a in [
        Action::ScrollDown,
        Action::GotoLast,
        Action::Undo,
        Action::Outline,
        Action::SearchNext,
        Action::Reload,
    ] {
        v.exec(a, Some(3));
    }
    v.execute_command("w");
    assert!(v.message.is_some());
    keys(&mut v, "i");
    assert_ne!(v.mode, UiMode::Draw);
    v.execute_command("e /definitely/not/here.pdf");
    pump(&mut v, "open error", |v| {
        v.message
            .as_ref()
            .map(|m| m.error && m.text.contains("Cannot open"))
            .unwrap_or(false)
    });
}

#[test]
fn password_protected_pdf_asks_for_the_password() {
    use mupdf::pdf::{Encryption, PdfDocument, PdfWriteOptions};
    let dir = tempfile::tempdir().unwrap();
    let plain = sample(dir.path(), 3);
    let locked = dir.path().join("locked.pdf");
    {
        let doc = PdfDocument::open(plain.to_str().unwrap()).unwrap();
        let mut o = PdfWriteOptions::default();
        o.set_encryption(Encryption::Aes256)
            .set_user_password("secret")
            .set_owner_password("owner");
        doc.save_with_options(locked.to_str().unwrap(), o).unwrap();
    }
    isolate();
    let mut v = Viewer::new(Settings::default(), None, Arc::new(|| {}));
    v.set_window([800, 600], 1.0);
    v.open(locked.clone(), LoadPurpose::Open { page: None });
    pump(&mut v, "password prompt", |v| v.mode == UiMode::Password);
    // Wrong password: asked again.
    keys(&mut v, "nope<CR>");
    pump(&mut v, "second prompt", |v| {
        v.mode == UiMode::Password && v.message.is_some()
    });
    assert!(v.message.as_ref().unwrap().text.contains("Wrong password"));
    // The typed text is masked in the status line.
    keys(&mut v, "abc");
    assert!(v.ui_state().input.unwrap().contains("***"));
    keys(&mut v, "<C-u>"); // not special in password mode: ctrl chars are ignored
    keys(&mut v, "<BS><BS><BS>");
    keys(&mut v, "secret<CR>");
    pump(&mut v, "document to open", |v| v.doc.is_some());
    assert_eq!(v.page_count(), 3);
}

#[test]
fn reload_keeps_the_view_and_refuses_with_unsaved_ink() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path(), 6);
    let mut v = open(&path);
    keys(&mut v, "3G");
    let before = v.top_pos().unwrap();
    keys(&mut v, "r");
    pump(&mut v, "reload", |v| {
        v.message
            .as_ref()
            .map(|m| m.text == "reloaded")
            .unwrap_or(false)
    });
    assert_eq!(v.top_pos().unwrap().page, before.page);
    keys(&mut v, "i");
    draw_stroke(&mut v, [100.0, 100.0], [400.0, 130.0]);
    keys(&mut v, "<Esc>r");
    assert!(v
        .message
        .as_ref()
        .map(|m| m.error && m.text.starts_with("E37"))
        .unwrap_or(false));
}

#[test]
fn external_change_triggers_a_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path(), 3);
    let mut v = open(&path);
    assert_eq!(v.page_count(), 3);
    std::thread::sleep(Duration::from_millis(400)); // let the watcher settle
                                                    // Like latexmk: write a new file and rename it over the old one.
    let tmp = dir.path().join("doc.pdf.tmp");
    std::fs::write(
        &tmp,
        mizu::testutil::make_pdf(&pages(5), &Extras::default()),
    )
    .unwrap();
    std::fs::rename(&tmp, &path).unwrap();
    pump(&mut v, "auto reload", |v| v.page_count() == 5);
}

#[test]
fn session_restores_position_and_marks() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path(), 9);
    {
        let mut v = open(&path);
        keys(&mut v, "6G");
        keys(&mut v, "mb");
        v.set_dark(true);
        v.save_session();
    }
    let v = open(&path);
    assert_eq!(v.top_pos().unwrap().page, 5);
    assert!(v.dark);
    assert!(v.doc.as_ref().unwrap().marks.contains_key(&'b'));
}

#[test]
fn digits_pick_palette_colours_in_light_mode() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 2));
    assert!(!v.dark);
    keys(&mut v, "i2");
    assert_eq!(v.mode, UiMode::Draw);
    assert_eq!(v.pen_color, v.settings.palette[1]);
    draw_stroke(&mut v, [300.0, 300.0], [400.0, 320.0]);
    let d = v.doc.as_ref().unwrap();
    let colour = d.ink.all().next().expect("a stroke").color;
    assert_eq!(colour, v.settings.palette[1]);
    keys(&mut v, "3");
    assert_eq!(v.pen_color, v.settings.palette[2]);
}

#[test]
fn non_latin_layout_uses_the_latin_key() {
    use mizu::input::Key;
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 2));
    // Ukrainian layout: the "i" key types "ш".
    v.on_key_layout(Key::ch('ш'), Some(Key::ch('i')));
    assert_eq!(v.mode, UiMode::Draw);
    // Text entry keeps the real character.
    keys(&mut v, "<Esc>:");
    v.on_key_layout(Key::ch('ш'), Some(Key::ch('i')));
    assert_eq!(v.line, "ш");
}

#[test]
fn command_line_tab_completion_and_editing() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 2));
    // A unique command completes, with a space when it takes an argument.
    keys(&mut v, ":wid<Tab>");
    assert_eq!(v.line, "width ");
    keys(&mut v, "<Esc>:da<Tab>");
    assert_eq!(v.line, "dark");
    // No common prefix to add: Tab cycles, Shift-Tab goes back.
    keys(&mut v, "<Esc>:co<Tab>");
    assert_eq!(v.line, "color ");
    keys(&mut v, "<Tab>");
    assert_eq!(v.line, "color 1");
    keys(&mut v, "<Tab>");
    assert_eq!(v.line, "color 2");
    keys(&mut v, "<S-Tab>");
    assert_eq!(v.line, "color 1");
    keys(&mut v, "<CR>");
    assert_eq!(v.pen_color, v.settings.palette[0]);

    // Cursor editing in the middle of the line.
    keys(&mut v, ":wdth<Left><Left><Left>i");
    assert_eq!(v.line, "width");
    keys(&mut v, "<Home><Del>W<End>X<BS>");
    assert_eq!(v.line, "Width");
    keys(&mut v, "<C-u>");
    assert_eq!(v.line, "");

    // Ghost text from history, accepted with Right.
    keys(&mut v, "<Esc>:width 3<CR>");
    assert!((v.pen_width - 3.0).abs() < 1e-6);
    keys(&mut v, ":wi<Right>");
    assert_eq!(v.line, "width 3");
    // History walks only entries with the typed prefix.
    keys(&mut v, "<Esc>:dark<CR>:light<CR>:w<Up>");
    assert_eq!(v.line, "width 3");
    keys(&mut v, "<Down>");
    assert_eq!(v.line, "w");
}

#[test]
fn file_completion_in_the_command_line() {
    let dir = tempfile::tempdir().unwrap();
    let doc = sample(dir.path(), 1);
    std::fs::create_dir(dir.path().join("lectures")).unwrap();
    std::fs::write(dir.path().join("lectures/linalg.pdf"), b"").unwrap();
    let mut v = open(&doc);
    let base = dir.path().display().to_string();
    keys(&mut v, ":e ");
    for c in base.chars() {
        v.on_key(mizu::input::Key::ch(c));
    }
    keys(&mut v, "/lec<Tab>");
    assert_eq!(v.line, format!("e {base}/lectures/"));
    keys(&mut v, "<Tab>");
    assert_eq!(v.line, format!("e {base}/lectures/linalg.pdf"));
}

#[test]
fn help_and_recent_lists() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 2));
    keys(&mut v, ":help<CR>");
    assert_eq!(v.mode, UiMode::Outline);
    let rows = v.list_visible();
    assert!(rows.iter().any(|(_, t)| t.contains("toggle_dark")));
    assert!(rows.iter().any(|(_, t)| t.contains(":write")));
    keys(&mut v, "<Esc>");
    assert_eq!(v.mode, UiMode::Normal);
    let path = v.path().unwrap().to_path_buf();
    v.session.set(&path, Default::default());
    keys(&mut v, ":recent<CR>");
    assert_eq!(v.mode, UiMode::Outline);
    assert!(v.list_visible()[0].1.contains("doc.pdf"));
}

#[test]
fn pen_settings_go_into_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = open(&sample(dir.path(), 1));
    keys(&mut v, ":color #123456<CR>:width 4<CR>");
    // Not save_session: tests share one session file.
    v.remember_prefs();
    let p = v.session.pen.expect("pen prefs");
    assert_eq!(p.color, [0x12, 0x34, 0x56]);
    assert!((p.width - 4.0).abs() < 1e-6);
    let mut w = Viewer::new(Settings::default(), None, Arc::new(|| {}));
    w.session.pen = Some(p);
    w.restore_prefs();
    assert_eq!(w.pen_color, [0x12, 0x34, 0x56]);
}
