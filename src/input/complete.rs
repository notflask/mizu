//! Suggestions for the `:` command line: command names, file paths and pen
//! colours, plus fish-style ghost text. Pure functions; the viewer keeps
//! the state (selection, Tab cycling).

use super::command::{
    common_prefix, format_hex, list_paths, lookup, ArgKind, ListDir, COLOR_NAMES, COMMANDS,
};

/// One suggestion. `text` replaces the word being completed.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub text: String,
    /// What the list shows (the name, without the directory part).
    pub label: String,
    /// Argument hint (`[file]`), shown dimmed after the label.
    pub hint: String,
    /// Description, shown dimmed.
    pub help: String,
    pub swatch: Option<[u8; 3]>,
    /// A directory: completing it continues into it.
    pub dir: bool,
    /// A command that takes an argument: completing it adds a space.
    pub wants_arg: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Suggestions {
    /// Byte offset in the line where the completed word starts.
    pub word_start: usize,
    pub items: Vec<Candidate>,
    /// Shown after the cursor in grey; accepted with Right / Ctrl-E.
    pub ghost: String,
}

pub struct Ctx<'a> {
    /// Oldest first.
    pub history: &'a [String],
    pub palette: &'a [[u8; 3]],
    pub list_dir: &'a ListDir<'a>,
}

const MAX_ITEMS: usize = 50;

pub fn suggest(line: &str, cursor: usize, ctx: &Ctx) -> Suggestions {
    let cursor = cursor.min(line.len());
    let before = &line[..cursor];
    let at_end = cursor == line.len();
    let mut s = match before.find(' ') {
        None => commands(before, ctx),
        Some(i) => arguments(before, i, ctx),
    };
    if !at_end {
        s.ghost.clear();
    }
    s.items.truncate(MAX_ITEMS);
    s
}

fn commands(word: &str, ctx: &Ctx) -> Suggestions {
    let mut items = Vec::new();
    if word.is_empty() {
        // Recent commands first, then everything.
        let mut seen: Vec<&str> = Vec::new();
        for h in ctx.history.iter().rev() {
            if seen.len() >= 3 {
                break;
            }
            if !seen.contains(&h.as_str()) {
                seen.push(h);
                items.push(Candidate {
                    text: h.clone(),
                    label: h.clone(),
                    hint: String::new(),
                    help: "recent".into(),
                    swatch: None,
                    dir: false,
                    wants_arg: false,
                });
            }
        }
    }
    let name = word.trim_end_matches('!');
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) {
        return Suggestions {
            word_start: 0,
            items: vec![Candidate {
                text: word.to_string(),
                label: word.to_string(),
                hint: String::new(),
                help: format!("go to page {name}"),
                swatch: None,
                dir: false,
                wants_arg: false,
            }],
            ghost: String::new(),
        };
    }
    let mut prefixed = Vec::new();
    let mut fuzzy = Vec::new();
    for spec in COMMANDS {
        let names = std::iter::once(spec.name).chain(spec.aliases.iter().copied());
        // Best way this command matches: by name, then by an alias.
        let mut best: Option<(u8, &str)> = None;
        for n in names {
            let rank = if n.starts_with(name) {
                0
            } else if name.len() >= 2 && subsequence(name, n) {
                1
            } else {
                continue;
            };
            if best.map(|(r, _)| rank < r).unwrap_or(true) {
                best = Some((rank, n));
            }
        }
        let Some((rank, _)) = best else { continue };
        let c = Candidate {
            text: spec.name.to_string(),
            label: spec.name.to_string(),
            hint: spec.arg_hint.to_string(),
            help: spec.help.to_string(),
            swatch: None,
            dir: false,
            wants_arg: spec.arg != ArgKind::None,
        };
        if rank == 0 {
            prefixed.push(c);
        } else {
            fuzzy.push(c);
        }
    }
    // An exact name or alias goes first (`w` -> write).
    if let Some(spec) = lookup(name) {
        if let Some(i) = prefixed.iter().position(|c| c.text == spec.name) {
            let c = prefixed.remove(i);
            prefixed.insert(0, c);
        }
    }
    // Fuzzy matches only help when nothing starts with what was typed.
    if prefixed.is_empty() {
        items.extend(fuzzy);
    } else {
        items.extend(prefixed);
    }

    // Ghost: the newest history entry that continues what is typed (fish),
    // otherwise the first command that does.
    let ghost = if word.is_empty() {
        String::new()
    } else if let Some(h) = ctx
        .history
        .iter()
        .rev()
        .find(|h| h.len() > word.len() && h.starts_with(word))
    {
        h[word.len()..].to_string()
    } else {
        items
            .iter()
            .find(|c| c.text.starts_with(word) && c.text.len() > word.len())
            .map(|c| c.text[word.len()..].to_string())
            .unwrap_or_default()
    };
    Suggestions {
        word_start: 0,
        items,
        ghost,
    }
}

fn arguments(before: &str, space: usize, ctx: &Ctx) -> Suggestions {
    let cmd = &before[..space];
    let arg_start = space
        + before[space..]
            .find(|c: char| c != ' ')
            .unwrap_or(before.len() - space);
    let arg = &before[arg_start..];
    let Some(spec) = lookup(cmd.trim_end_matches('!')) else {
        return Suggestions::default();
    };
    let ghost_from = |items: &[Candidate], typed: &str| {
        items
            .first()
            .filter(|c| c.text.len() > typed.len() && c.text.starts_with(typed))
            .map(|c| c.text[typed.len()..].to_string())
            .unwrap_or_default()
    };
    match spec.arg {
        ArgKind::Path { docs } => {
            let Some(found) = list_paths(arg, docs, ctx.list_dir) else {
                return Suggestions {
                    word_start: arg_start,
                    ..Default::default()
                };
            };
            let items: Vec<Candidate> = found
                .names
                .into_iter()
                .map(|(name, dir)| Candidate {
                    text: format!("{}{}{}", found.dir_typed, name, if dir { "/" } else { "" }),
                    label: format!("{}{}", name, if dir { "/" } else { "" }),
                    hint: String::new(),
                    help: String::new(),
                    swatch: None,
                    dir,
                    wants_arg: false,
                })
                .collect();
            // Ghost only when the choice is clear.
            let ghost = if items.len() == 1 {
                ghost_from(&items, arg)
            } else {
                String::new()
            };
            Suggestions {
                word_start: arg_start,
                items,
                ghost,
            }
        }
        ArgKind::Color => {
            let lower = arg.to_ascii_lowercase();
            let mut items = Vec::new();
            for (i, c) in ctx.palette.iter().enumerate().take(9) {
                let num = (i + 1).to_string();
                let hex = format_hex(*c);
                if num.starts_with(&lower) || hex.starts_with(&lower) {
                    items.push(Candidate {
                        text: num.clone(),
                        label: format!("{num}  {hex}"),
                        hint: String::new(),
                        help: "palette".into(),
                        swatch: Some(*c),
                        dir: false,
                        wants_arg: false,
                    });
                }
            }
            for (name, c) in COLOR_NAMES {
                if *name == "gray" && !lower.starts_with("gra") {
                    continue;
                }
                if name.starts_with(&lower) {
                    items.push(Candidate {
                        text: name.to_string(),
                        label: name.to_string(),
                        hint: String::new(),
                        help: format_hex(*c),
                        swatch: Some(*c),
                        dir: false,
                        wants_arg: false,
                    });
                }
            }
            let ghost = if arg.is_empty() {
                String::new()
            } else {
                ghost_from(&items, &lower)
            };
            Suggestions {
                word_start: arg_start,
                items,
                ghost,
            }
        }
        ArgKind::Choice(words) => {
            let items: Vec<Candidate> = words
                .iter()
                .filter(|w| w.starts_with(arg))
                .map(|w| Candidate {
                    text: w.to_string(),
                    label: w.to_string(),
                    hint: String::new(),
                    help: String::new(),
                    swatch: None,
                    dir: false,
                    wants_arg: false,
                })
                .collect();
            let ghost = if arg.is_empty() {
                String::new()
            } else {
                ghost_from(&items, arg)
            };
            Suggestions {
                word_start: arg_start,
                items,
                ghost,
            }
        }
        ArgKind::None | ArgKind::Number => Suggestions {
            word_start: arg_start,
            items: Vec::new(),
            ghost: String::new(),
        },
    }
}

/// `needle`'s characters appear in `hay` in order (`wq` in `write-quit`).
fn subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// What Tab should insert: the longest common prefix of the candidates, if
/// it adds anything to `typed`.
pub fn tab_prefix(items: &[Candidate], typed: &str) -> Option<String> {
    let common = common_prefix(items.iter().map(|c| c.text.as_str()));
    (common.len() > typed.len() && common.starts_with(typed)).then_some(common)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn no_files(_: &Path) -> Option<Vec<(String, bool)>> {
        None
    }

    fn ctx<'a>(history: &'a [String], list: &'a ListDir<'a>) -> Ctx<'a> {
        Ctx {
            history,
            palette: &[[0x1a, 0x1a, 0x1a], [0xe0, 0x31, 0x31]],
            list_dir: list,
        }
    }

    fn texts(s: &Suggestions) -> Vec<&str> {
        s.items.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn prefix_matches_win_over_fuzzy_ones() {
        let c = ctx(&[], &no_files);
        let s = suggest("wi", 2, &c);
        assert_eq!(texts(&s), vec!["width"]);
        assert_eq!(s.ghost, "dth");
        // Nothing starts with "wrt": fuzzy finds write.
        let s = suggest("wrt", 3, &c);
        assert_eq!(texts(&s), vec!["write"]);
    }

    #[test]
    fn alias_finds_the_command() {
        let c = ctx(&[], &no_files);
        let s = suggest("w", 1, &c);
        assert_eq!(texts(&s)[0], "write");
        let s = suggest("e", 1, &c);
        assert_eq!(texts(&s)[0], "edit");
        let s = suggest("exi", 3, &c);
        assert_eq!(texts(&s)[0], "xit");
    }

    #[test]
    fn empty_line_shows_history_then_commands() {
        let h = vec!["dark".to_string(), "w".to_string(), "dark".to_string()];
        let c = ctx(&h, &no_files);
        let s = suggest("", 0, &c);
        assert_eq!(&texts(&s)[..2], &["dark", "w"]);
        assert!(s.items.len() > COMMANDS.len());
        assert!(s.ghost.is_empty());
    }

    #[test]
    fn ghost_prefers_history() {
        let h = vec!["e ~/notes/linalg.pdf".to_string()];
        let c = ctx(&h, &no_files);
        let s = suggest("e ~", 3, &c);
        // In argument position the history is not used; only paths.
        assert!(s.ghost.is_empty());
        let s = suggest("e", 1, &c);
        assert_eq!(s.ghost, " ~/notes/linalg.pdf");
    }

    #[test]
    fn no_ghost_with_the_cursor_inside() {
        let c = ctx(&[], &no_files);
        let s = suggest("wid x", 3, &c);
        assert!(s.ghost.is_empty());
        assert_eq!(texts(&s)[0], "width");
    }

    #[test]
    fn paths_with_directories_first() {
        let list = |p: &Path| -> Option<Vec<(String, bool)>> {
            assert_eq!(p, Path::new("docs/"));
            Some(vec![
                ("b.pdf".into(), false),
                ("a b.pdf".into(), false),
                ("sub".into(), true),
                ("x.txt".into(), false),
                (".hidden.pdf".into(), false),
            ])
        };
        let c = ctx(&[], &list);
        let s = suggest("e docs/", 7, &c);
        assert_eq!(s.word_start, 2);
        assert_eq!(texts(&s), vec!["docs/sub/", "docs/a b.pdf", "docs/b.pdf"]);
        assert_eq!(s.items[0].label, "sub/");
        assert!(s.items[0].dir);
        // :w offers any file.
        let s = suggest("w docs/x", 8, &c);
        assert_eq!(texts(&s), vec!["docs/x.txt"]);
        assert_eq!(s.ghost, ".txt");
    }

    #[test]
    fn colours_from_palette_and_names() {
        let c = ctx(&[], &no_files);
        let s = suggest("color ", 6, &c);
        assert_eq!(&texts(&s)[..2], &["1", "2"]);
        assert_eq!(s.items[1].swatch, Some([0xe0, 0x31, 0x31]));
        assert!(texts(&s).contains(&"red"));
        assert!(!texts(&s).contains(&"gray"));
        let s = suggest("color re", 8, &c);
        assert_eq!(texts(&s), vec!["red"]);
        assert_eq!(s.ghost, "d");
    }

    #[test]
    fn page_numbers() {
        let c = ctx(&[], &no_files);
        let s = suggest("42", 2, &c);
        assert_eq!(s.items[0].help, "go to page 42");
    }

    #[test]
    fn tab_inserts_the_common_prefix() {
        let c = ctx(&[], &no_files);
        let s = suggest("da", 2, &c);
        assert_eq!(texts(&s), vec!["dark"]);
        assert_eq!(tab_prefix(&s.items, "da").as_deref(), Some("dark"));
        // "d": dark and direction share only the "d".
        let s = suggest("d", 1, &c);
        assert_eq!(tab_prefix(&s.items, "d"), None);
        let s = suggest("", 0, &c);
        assert_eq!(tab_prefix(&s.items, ""), None);
    }
}
