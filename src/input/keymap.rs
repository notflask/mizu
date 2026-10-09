//! Modal key bindings with counts, multi-key sequences and `m{char}` style
//! character arguments. No timeouts: a prefix simply waits for the next key.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::actions::Action;
use super::keys::{parse_seq, Key, KeyCode, Mods};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Draw,
}

#[derive(Clone, Debug, Default)]
struct Table {
    bindings: HashMap<Vec<Key>, Action>,
    prefixes: HashSet<Vec<Key>>,
}

impl Table {
    fn bind(&mut self, seq: Vec<Key>, action: Action) {
        for n in 1..seq.len() {
            self.prefixes.insert(seq[..n].to_vec());
        }
        self.bindings.insert(seq, action);
    }

    fn unbind(&mut self, seq: &[Key]) {
        self.bindings.remove(seq);
        self.rebuild_prefixes();
    }

    fn rebuild_prefixes(&mut self) {
        self.prefixes.clear();
        for seq in self.bindings.keys() {
            for n in 1..seq.len() {
                self.prefixes.insert(seq[..n].to_vec());
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Keymaps {
    normal: Table,
    draw: Table,
}

fn k(s: &str) -> Vec<Key> {
    parse_seq(s).expect("built-in key sequence")
}

impl Keymaps {
    pub fn defaults() -> Self {
        use Action::*;
        let mut normal = Table::default();
        let n: &[(&str, Action)] = &[
            ("j", ScrollDown),
            ("<Down>", ScrollDown),
            ("k", ScrollUp),
            ("<Up>", ScrollUp),
            ("h", ScrollLeft),
            ("<Left>", ScrollLeft),
            ("l", ScrollRight),
            ("<Right>", ScrollRight),
            ("<C-d>", HalfPageDown),
            ("<C-u>", HalfPageUp),
            ("<C-f>", PageDown),
            ("<PageDown>", PageDown),
            ("<Space>", PageDown),
            ("<C-b>", PageUp),
            ("<PageUp>", PageUp),
            ("<S-Space>", PageUp),
            ("J", NextPage),
            ("K", PrevPage),
            ("gg", GotoFirst),
            ("<Home>", GotoFirst),
            ("G", GotoLast),
            ("<End>", GotoLast),
            ("+", ZoomIn),
            ("-", ZoomOut),
            ("=", FitWidth),
            ("zw", FitWidth),
            ("zf", FitPage),
            ("z0", Zoom100),
            ("/", SearchForward),
            ("?", SearchBackward),
            ("n", SearchNext),
            ("N", SearchPrev),
            ("m", SetMark('\0')),
            ("'", JumpMark('\0')),
            ("`", JumpMark('\0')),
            ("<C-o>", JumpBack),
            ("<C-i>", JumpForward),
            ("<Tab>", JumpForward),
            ("o", Outline),
            ("<F1>", Help),
            ("D", ToggleDark),
            ("i", EnterDraw),
            ("u", Undo),
            ("<C-r>", Redo),
            ("r", Reload),
            (":", CommandMode),
            ("ZZ", WriteQuit),
            ("ZQ", QuitForce),
            ("<Esc>", ClearMessage),
        ];
        for (s, a) in n {
            normal.bind(k(s), *a);
        }

        // Drawing mode inherits navigation from normal mode.
        let mut draw = normal.clone();
        draw.unbind(&k("i"));
        let d: &[(&str, Action)] = &[
            ("<Esc>", ExitDraw),
            ("e", ToggleEraser),
            ("p", PenTool),
            ("1", SelectColor(1)),
            ("2", SelectColor(2)),
            ("3", SelectColor(3)),
            ("4", SelectColor(4)),
            ("5", SelectColor(5)),
            ("6", SelectColor(6)),
            ("7", SelectColor(7)),
            ("8", SelectColor(8)),
            ("9", SelectColor(9)),
            ("[", WidthDown),
            ("]", WidthUp),
        ];
        for (s, a) in d {
            draw.bind(k(s), *a);
        }
        Keymaps { normal, draw }
    }

    /// Apply `[keys.normal]` / `[keys.draw]` overrides from the config.
    /// Returns human readable warnings for entries that could not be used.
    pub fn apply_overrides(
        &mut self,
        normal: &BTreeMap<String, String>,
        draw: &BTreeMap<String, String>,
    ) -> Vec<String> {
        let mut warnings = Vec::new();
        for (table, entries, name) in [
            (&mut self.normal, normal, "normal"),
            (&mut self.draw, draw, "draw"),
        ] {
            for (seq, act) in entries {
                let keys = match parse_seq(seq) {
                    Ok(keys) => keys,
                    Err(e) => {
                        warnings.push(format!("keys.{name}: {seq:?}: {e}"));
                        continue;
                    }
                };
                match Action::from_name(act) {
                    Some(Action::None) => table.unbind(&keys),
                    Some(a) => table.bind(keys, a),
                    None => warnings.push(format!("keys.{name}: unknown action {act:?}")),
                }
            }
        }
        warnings
    }

    /// All bindings of a mode as `(keys, action)`, sorted by action.
    pub fn bindings(&self, mode: Mode) -> Vec<(String, Action)> {
        let mut v: Vec<(String, Action)> = self
            .table(mode)
            .bindings
            .iter()
            .filter(|(_, a)| **a != Action::None)
            .map(|(k, a)| (crate::input::keys::format_seq(k), *a))
            .collect();
        v.sort_by(|a, b| a.1.name().cmp(&b.1.name()).then_with(|| a.0.cmp(&b.0)));
        v
    }

    /// True when `key` appears anywhere in a binding of `mode`.
    pub fn uses_key(&self, mode: Mode, key: Key) -> bool {
        self.table(mode)
            .bindings
            .keys()
            .any(|seq| seq.contains(&key))
    }

    fn table(&self, mode: Mode) -> &Table {
        match mode {
            Mode::Normal => &self.normal,
            Mode::Draw => &self.draw,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fired {
    pub action: Action,
    pub count: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feed {
    /// More keys are needed.
    Pending,
    /// Nothing is pending any more.
    Idle,
}

/// Incremental parser for key sequences.
#[derive(Debug, Default)]
pub struct KeyEngine {
    pending: Vec<Key>,
    count: Option<u32>,
    /// A binding that needs one more key as argument.
    waiting_char: Option<(Action, Option<u32>)>,
    /// The pending sequence is a complete binding *and* a prefix of another.
    ambiguous: Option<Action>,
}

impl KeyEngine {
    pub fn reset(&mut self) {
        self.pending.clear();
        self.count = None;
        self.waiting_char = None;
        self.ambiguous = None;
    }

    pub fn is_pending(&self) -> bool {
        !self.pending.is_empty() || self.count.is_some() || self.waiting_char.is_some()
    }

    /// Text shown in the status line while a command is being typed.
    pub fn pending_text(&self) -> String {
        let mut s = String::new();
        if let Some(c) = self.count {
            s.push_str(&c.to_string());
        }
        for key in &self.pending {
            s.push_str(&key.to_string());
        }
        s
    }

    /// Feed one key. Completed actions are pushed to `out`.
    pub fn feed(&mut self, maps: &Keymaps, mode: Mode, key: Key, out: &mut Vec<Fired>) -> Feed {
        let table = maps.table(mode);

        if let Some((action, count)) = self.waiting_char.take() {
            self.pending.clear();
            if let KeyCode::Char(c) = key.code {
                if !key.mods.ctrl {
                    out.push(Fired {
                        action: action.with_char(c),
                        count,
                    });
                }
            }
            return Feed::Idle;
        }

        // A Escape always aborts a half-typed sequence.
        if key.code == KeyCode::Esc && self.is_pending() {
            self.reset();
            return Feed::Idle;
        }

        // Counts: digits that are not bound at the root start / extend one.
        if self.pending.is_empty() && self.ambiguous.is_none() {
            if let (KeyCode::Char(c), false, false) = (key.code, key.mods.ctrl, key.mods.alt) {
                if let Some(d) = c.to_digit(10) {
                    let bound = table.bindings.contains_key(&vec![key]);
                    if (d != 0 || self.count.is_some()) && !bound {
                        let next = self.count.unwrap_or(0).saturating_mul(10).saturating_add(d);
                        self.count = Some(next.min(9_999_999));
                        return Feed::Pending;
                    }
                }
            }
        }

        self.step(table, key, out)
    }

    fn step(&mut self, table: &Table, key: Key, out: &mut Vec<Fired>) -> Feed {
        let mut seq = self.pending.clone();
        seq.push(key);

        if let Some(&action) = table.bindings.get(&seq) {
            if table.prefixes.contains(&seq) {
                // Complete, but could also grow into a longer binding.
                self.pending = seq;
                self.ambiguous = Some(action);
                return Feed::Pending;
            }
            return self.complete(action, out);
        }
        if table.prefixes.contains(&seq) {
            self.pending = seq;
            self.ambiguous = None;
            return Feed::Pending;
        }

        // Mismatch. If the previous prefix was a complete binding, fire it
        // and retry this key from scratch.
        if let Some(action) = self.ambiguous.take() {
            self.pending.clear();
            self.complete(action, out);
            return self.step(table, key, out);
        }
        self.reset();
        Feed::Idle
    }

    fn complete(&mut self, action: Action, out: &mut Vec<Fired>) -> Feed {
        let count = self.count.take();
        self.pending.clear();
        self.ambiguous = None;
        if action.needs_char() {
            self.waiting_char = Some((action, count));
            return Feed::Pending;
        }
        if action != Action::None {
            out.push(Fired { action, count });
        }
        Feed::Idle
    }
}

#[allow(dead_code)]
fn _assert_mods_used(_: Mods) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(mode: Mode, input: &str) -> Vec<Fired> {
        let maps = Keymaps::defaults();
        let mut eng = KeyEngine::default();
        let mut out = Vec::new();
        for key in parse_seq(input).unwrap() {
            eng.feed(&maps, mode, key, &mut out);
        }
        out
    }

    fn actions(v: &[Fired]) -> Vec<Action> {
        v.iter().map(|f| f.action).collect()
    }

    #[test]
    fn single_keys() {
        assert_eq!(
            actions(&run(Mode::Normal, "jjk")),
            vec![Action::ScrollDown, Action::ScrollDown, Action::ScrollUp]
        );
    }

    #[test]
    fn counts() {
        let out = run(Mode::Normal, "5j");
        assert_eq!(
            out,
            vec![Fired {
                action: Action::ScrollDown,
                count: Some(5)
            }]
        );
        let out = run(Mode::Normal, "42G");
        assert_eq!(
            out,
            vec![Fired {
                action: Action::GotoLast,
                count: Some(42)
            }]
        );
        let out = run(Mode::Normal, "12gg");
        assert_eq!(
            out,
            vec![Fired {
                action: Action::GotoFirst,
                count: Some(12)
            }]
        );
        let out = run(Mode::Normal, "<C-d>");
        assert_eq!(out[0].count, None);
    }

    #[test]
    fn zero_is_not_a_count_start() {
        // `0` is unbound at the root and not a count start: nothing happens.
        assert!(run(Mode::Normal, "0").is_empty());
        let out = run(Mode::Normal, "10j");
        assert_eq!(out[0].count, Some(10));
    }

    #[test]
    fn sequences() {
        assert_eq!(actions(&run(Mode::Normal, "gg")), vec![Action::GotoFirst]);
        assert_eq!(actions(&run(Mode::Normal, "zw")), vec![Action::FitWidth]);
        assert_eq!(actions(&run(Mode::Normal, "z0")), vec![Action::Zoom100]);
        assert_eq!(actions(&run(Mode::Normal, "ZZ")), vec![Action::WriteQuit]);
        assert_eq!(actions(&run(Mode::Normal, "ZQ")), vec![Action::QuitForce]);
    }

    #[test]
    fn invalid_sequences_are_dropped() {
        // `gx` is invalid; `j` afterwards works again.
        assert_eq!(actions(&run(Mode::Normal, "gxj")), vec![Action::ScrollDown]);
    }

    #[test]
    fn char_arguments() {
        assert_eq!(
            actions(&run(Mode::Normal, "ma")),
            vec![Action::SetMark('a')]
        );
        assert_eq!(
            actions(&run(Mode::Normal, "'b")),
            vec![Action::JumpMark('b')]
        );
        // The key after `m` is the argument even when it is bound otherwise.
        assert_eq!(
            actions(&run(Mode::Normal, "mj")),
            vec![Action::SetMark('j')]
        );
    }

    #[test]
    fn escape_cancels() {
        assert_eq!(
            actions(&run(Mode::Normal, "g<Esc>j")),
            vec![Action::ScrollDown]
        );
        assert_eq!(
            actions(&run(Mode::Normal, "5<Esc>j")),
            vec![Action::ScrollDown]
        );
        assert_eq!(run(Mode::Normal, "5<Esc>j")[0].count, None);
    }

    #[test]
    fn draw_mode_digits_are_colours() {
        assert_eq!(actions(&run(Mode::Draw, "3")), vec![Action::SelectColor(3)]);
        assert_eq!(actions(&run(Mode::Draw, "e")), vec![Action::ToggleEraser]);
        assert_eq!(actions(&run(Mode::Draw, "<Esc>")), vec![Action::ExitDraw]);
        // Navigation still works, `i` is not re-entering draw mode.
        assert_eq!(actions(&run(Mode::Draw, "j")), vec![Action::ScrollDown]);
        assert!(run(Mode::Draw, "i").is_empty());
        assert_eq!(actions(&run(Mode::Draw, "u")), vec![Action::Undo]);
    }

    #[test]
    fn overrides_add_and_remove() {
        let mut maps = Keymaps::defaults();
        let mut n = BTreeMap::new();
        n.insert("<C-n>".to_string(), "toggle_dark".to_string());
        n.insert("D".to_string(), "none".to_string());
        n.insert("bogus!".to_string(), "no_such_action".to_string());
        let warnings = maps.apply_overrides(&n, &BTreeMap::new());
        assert_eq!(warnings.len(), 1);

        let mut eng = KeyEngine::default();
        let mut out = Vec::new();
        for key in parse_seq("<C-n>").unwrap() {
            eng.feed(&maps, Mode::Normal, key, &mut out);
        }
        assert_eq!(actions(&out), vec![Action::ToggleDark]);
        out.clear();
        for key in parse_seq("D").unwrap() {
            eng.feed(&maps, Mode::Normal, key, &mut out);
        }
        assert!(out.is_empty());
    }

    #[test]
    fn ambiguous_prefix_resolves_on_next_key() {
        let mut maps = Keymaps::defaults();
        let mut n = BTreeMap::new();
        n.insert("g".to_string(), "scroll_up".to_string());
        maps.apply_overrides(&n, &BTreeMap::new());
        let mut eng = KeyEngine::default();
        let mut out = Vec::new();
        // `g` then `j`: fires `g` (scroll_up), then `j` (scroll_down).
        for key in parse_seq("gj").unwrap() {
            eng.feed(&maps, Mode::Normal, key, &mut out);
        }
        assert_eq!(actions(&out), vec![Action::ScrollUp, Action::ScrollDown]);
        // `gg` still reaches the longer binding.
        out.clear();
        for key in parse_seq("gg").unwrap() {
            eng.feed(&maps, Mode::Normal, key, &mut out);
        }
        assert_eq!(actions(&out), vec![Action::GotoFirst]);
    }

    #[test]
    fn pending_text_for_status_line() {
        let maps = Keymaps::defaults();
        let mut eng = KeyEngine::default();
        let mut out = Vec::new();
        for key in parse_seq("12z").unwrap() {
            eng.feed(&maps, Mode::Normal, key, &mut out);
        }
        assert_eq!(eng.pending_text(), "12z");
        assert!(eng.is_pending());
    }
}
