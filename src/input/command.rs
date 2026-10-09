//! `:` commands: the registry, the parser and path completion.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Write {
        path: Option<PathBuf>,
    },
    Quit {
        force: bool,
    },
    WriteQuit,
    /// `:x`: write only when something changed, then quit.
    Exit,
    Edit {
        path: Option<PathBuf>,
        force: bool,
    },
    Goto(usize),
    Dark,
    Light,
    Color([u8; 3]),
    /// `:color 3`: a palette entry (1-based).
    ColorIndex(u8),
    /// `:width` (show) or `:width 2` (set).
    Width(Option<f32>),
    Help,
    Recent,
    FontSize(f32),
    /// `:spread [on|off|auto]`.
    Spread(Option<&'static str>),
    /// `:direction [rtl|ltr]` (`None` toggles).
    Direction(Option<bool>),
}

/// What a command takes after its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgKind {
    None,
    /// A file; `docs` limits suggestions to documents mizu can open.
    Path {
        docs: bool,
    },
    Color,
    Number,
    /// One of a few words.
    Choice(&'static [&'static str]),
}

/// One entry of the registry: the single source of truth for parsing,
/// completion, suggestions and `:help`.
#[derive(Clone, Copy, Debug)]
pub struct CommandSpec {
    pub name: &'static str,
    /// Other names that are accepted exactly (`w` for `write`).
    pub aliases: &'static [&'static str],
    pub bang: bool,
    pub arg: ArgKind,
    /// Shown after the name in suggestions (`{file}`).
    pub arg_hint: &'static str,
    pub help: &'static str,
}

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "write",
        aliases: &["w"],
        bang: false,
        arg: ArgKind::Path { docs: false },
        arg_hint: "[file]",
        help: "write the file with its ink (or a copy to [file])",
    },
    CommandSpec {
        name: "quit",
        aliases: &["q"],
        bang: true,
        arg: ArgKind::None,
        arg_hint: "",
        help: "quit (q! discards unsaved ink)",
    },
    CommandSpec {
        name: "wq",
        aliases: &[],
        bang: false,
        arg: ArgKind::None,
        arg_hint: "",
        help: "write and quit",
    },
    CommandSpec {
        name: "xit",
        aliases: &["x", "exit"],
        bang: false,
        arg: ArgKind::None,
        arg_hint: "",
        help: "write if changed, then quit",
    },
    CommandSpec {
        name: "edit",
        aliases: &["e"],
        bang: true,
        arg: ArgKind::Path { docs: true },
        arg_hint: "[file]",
        help: "open a file (e! reloads and drops unsaved ink)",
    },
    CommandSpec {
        name: "dark",
        aliases: &[],
        bang: false,
        arg: ArgKind::None,
        arg_hint: "",
        help: "dark pages",
    },
    CommandSpec {
        name: "light",
        aliases: &[],
        bang: false,
        arg: ArgKind::None,
        arg_hint: "",
        help: "light pages",
    },
    CommandSpec {
        name: "color",
        aliases: &["colour"],
        bang: false,
        arg: ArgKind::Color,
        arg_hint: "{#rrggbb|name|1-9}",
        help: "pen colour",
    },
    CommandSpec {
        name: "width",
        aliases: &[],
        bang: false,
        arg: ArgKind::Number,
        arg_hint: "[pt]",
        help: "pen width in points (no argument: show it)",
    },
    CommandSpec {
        name: "fontsize",
        aliases: &[],
        bang: false,
        arg: ArgKind::Number,
        arg_hint: "{pt}",
        help: "text size of EPUB books",
    },
    CommandSpec {
        name: "spread",
        aliases: &[],
        bang: false,
        arg: ArgKind::Choice(&["on", "off", "auto"]),
        arg_hint: "[on|off|auto]",
        help: "two pages side by side (books: auto)",
    },
    CommandSpec {
        name: "direction",
        aliases: &["dir"],
        bang: false,
        arg: ArgKind::Choice(&["rtl", "ltr"]),
        arg_hint: "[rtl|ltr]",
        help: "page order in spreads (manga: rtl)",
    },
    CommandSpec {
        name: "recent",
        aliases: &[],
        bang: false,
        arg: ArgKind::None,
        arg_hint: "",
        help: "recently opened files",
    },
    CommandSpec {
        name: "help",
        aliases: &["h"],
        bang: false,
        arg: ArgKind::None,
        arg_hint: "",
        help: "keys and commands",
    },
];

/// The spec for a typed command name (without `!`).
pub fn lookup(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS
        .iter()
        .find(|c| c.name == name || c.aliases.contains(&name))
}

/// Named pen colours. Where a default palette colour exists, it is used.
pub const COLOR_NAMES: &[(&str, [u8; 3])] = &[
    ("black", [0x1a, 0x1a, 0x1a]),
    ("red", [0xe0, 0x31, 0x31]),
    ("blue", [0x19, 0x71, 0xc2]),
    ("green", [0x2f, 0x9e, 0x44]),
    ("orange", [0xf0, 0x8c, 0x00]),
    ("purple", [0x9c, 0x36, 0xb5]),
    ("yellow", [0xfa, 0xb0, 0x05]),
    ("grey", [0x86, 0x8e, 0x96]),
    ("gray", [0x86, 0x8e, 0x96]),
    ("white", [0xff, 0xff, 0xff]),
];

pub fn parse(line: &str) -> Result<Option<Command>, String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    if let Ok(n) = line.parse::<usize>() {
        return Ok(Some(Command::Goto(n)));
    }
    let (name, rest) = match line.find(char::is_whitespace) {
        Some(i) => (&line[..i], line[i..].trim()),
        None => (line, ""),
    };
    let (name, bang) = match name.strip_suffix('!') {
        Some(n) => (n, true),
        None => (name, false),
    };
    let Some(spec) = lookup(name) else {
        return Err(format!("E492: Not an editor command: {name}"));
    };
    if bang && !spec.bang {
        return Err(format!("E477: No ! allowed: {name}!"));
    }
    let path = || -> Option<PathBuf> {
        if rest.is_empty() {
            None
        } else {
            Some(expand_tilde(rest))
        }
    };
    let number = || -> Result<f32, String> {
        let v: f32 = rest
            .parse()
            .map_err(|_| "E474: Invalid argument: expected a number".to_string())?;
        if !(v.is_finite() && v > 0.0) {
            return Err(format!(
                "E474: Invalid argument: {} must be positive",
                spec.name
            ));
        }
        Ok(v)
    };
    let cmd = match spec.name {
        "write" => Command::Write { path: path() },
        "quit" => Command::Quit { force: bang },
        "wq" => Command::WriteQuit,
        "xit" => Command::Exit,
        "edit" => Command::Edit {
            path: path(),
            force: bang,
        },
        "dark" => Command::Dark,
        "light" => Command::Light,
        "color" => parse_color(rest).ok_or_else(|| {
            "E474: Invalid argument: expected #rrggbb, a colour name or 1-9".to_string()
        })?,
        "width" if rest.is_empty() => Command::Width(None),
        "width" => Command::Width(Some(number()?)),
        "fontsize" => Command::FontSize(number()?),
        "recent" => Command::Recent,
        "spread" => match rest {
            "" => Command::Spread(None),
            "on" => Command::Spread(Some("on")),
            "off" => Command::Spread(Some("off")),
            "auto" => Command::Spread(Some("auto")),
            _ => return Err("E474: Invalid argument: expected on, off or auto".into()),
        },
        "direction" => match rest {
            "" => Command::Direction(None),
            "rtl" => Command::Direction(Some(true)),
            "ltr" => Command::Direction(Some(false)),
            _ => return Err("E474: Invalid argument: expected rtl or ltr".into()),
        },
        "help" => Command::Help,
        _ => return Err(format!("E492: Not an editor command: {name}")),
    };
    Ok(Some(cmd))
}

fn parse_color(s: &str) -> Option<Command> {
    let s = s.trim();
    if let Ok(n) = s.parse::<u8>() {
        return (1..=9).contains(&n).then_some(Command::ColorIndex(n));
    }
    let lower = s.to_ascii_lowercase();
    if let Some((_, c)) = COLOR_NAMES.iter().find(|(n, _)| *n == lower) {
        return Some(Command::Color(*c));
    }
    parse_hex(s).map(Command::Color)
}

/// `#rrggbb` or `#rgb` (the `#` is optional).
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if !s.is_ascii() {
        return None;
    }
    match s.len() {
        6 => {
            let v = u32::from_str_radix(s, 16).ok()?;
            Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
        }
        3 => {
            let v = u16::from_str_radix(s, 16).ok()?;
            let d = |x: u16| (x as u8 & 0xf) * 0x11;
            Some([d(v >> 8), d(v >> 4), d(v)])
        }
        _ => None,
    }
}

pub fn format_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

pub fn expand_tilde(s: &str) -> PathBuf {
    if s == "~" || s.starts_with("~/") {
        if let Some(home) = directories::UserDirs::new().map(|d| d.home_dir().to_path_buf()) {
            return home.join(s.trim_start_matches('~').trim_start_matches('/'));
        }
    }
    PathBuf::from(s)
}

/// File extensions mizu can open.
pub fn is_document(name: &str) -> bool {
    Path::new(name)
        .extension()
        .map(|x| x.eq_ignore_ascii_case("pdf") || x.eq_ignore_ascii_case("epub"))
        .unwrap_or(false)
}

/// Complete the path argument of `:e` / `:w`. Returns the completed command
/// line (extended by the longest common prefix of all candidates).
pub fn complete(line: &str) -> Option<String> {
    let (cmd, arg) = line.split_once(' ')?;
    let spec = lookup(cmd.trim_end_matches('!'))?;
    let ArgKind::Path { docs } = spec.arg else {
        return None;
    };
    let arg = arg.trim_start();
    let found = list_paths(arg, docs, &read_dir)?;
    if found.names.is_empty() {
        return None;
    }
    let common = common_prefix(found.names.iter().map(|(n, _)| n.as_str()));
    let mut completed = format!("{}{common}", found.dir_typed);
    if found.names.len() == 1 && found.names[0].1 {
        completed.push('/');
    }
    Some(format!("{cmd} {completed}"))
}

/// Entries of a directory as `(name, is_dir)`, or `None` if unreadable.
pub type ListDir<'a> = dyn Fn(&Path) -> Option<Vec<(String, bool)>> + 'a;

pub fn read_dir(dir: &Path) -> Option<Vec<(String, bool)>> {
    Some(
        std::fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                // Follow symlinks so a link to a directory completes as one.
                let is_dir = std::fs::metadata(e.path())
                    .map(|m| m.is_dir())
                    .unwrap_or(false);
                (name, is_dir)
            })
            .collect(),
    )
}

pub struct PathMatches {
    /// The directory part as typed (with `~`), ending in `/` or empty.
    pub dir_typed: String,
    /// Sorted matching entries: `(name, is_dir)`.
    pub names: Vec<(String, bool)>,
}

/// Entries matching a partly typed path. Directories come first.
pub fn list_paths(arg: &str, docs: bool, list: &ListDir) -> Option<PathMatches> {
    let expanded = expand_tilde(arg).to_string_lossy().into_owned();
    let expanded = if arg == "~" {
        format!("{expanded}/")
    } else {
        expanded
    };
    let (dir_typed, file_prefix) = match arg.rfind('/') {
        Some(i) => (&arg[..=i], &arg[i + 1..]),
        None if arg == "~" => ("~/", ""),
        None => ("", arg),
    };
    let dir_path = match expanded.rfind('/') {
        Some(i) => PathBuf::from(&expanded[..=i]),
        None => PathBuf::from("."),
    };
    let mut names: Vec<(String, bool)> = list(&dir_path)?
        .into_iter()
        .filter(|(name, is_dir)| {
            if !name.starts_with(file_prefix) {
                return false;
            }
            if !file_prefix.starts_with('.') && name.starts_with('.') {
                return false;
            }
            *is_dir || !docs || is_document(name)
        })
        .collect();
    names.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Some(PathMatches {
        dir_typed: dir_typed.to_string(),
        names,
    })
}

pub fn common_prefix<'a>(mut it: impl Iterator<Item = &'a str>) -> String {
    let first = match it.next() {
        Some(f) => f.to_string(),
        None => return String::new(),
    };
    let mut len = first.len();
    for s in it {
        len = len.min(
            first
                .chars()
                .zip(s.chars())
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| a.len_utf8())
                .sum(),
        );
    }
    first[..len].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_commands() {
        assert_eq!(parse("q").unwrap(), Some(Command::Quit { force: false }));
        assert_eq!(parse("q!").unwrap(), Some(Command::Quit { force: true }));
        assert_eq!(parse("wq").unwrap(), Some(Command::WriteQuit));
        assert_eq!(parse("x").unwrap(), Some(Command::Exit));
        assert_eq!(parse("exit").unwrap(), Some(Command::Exit));
        assert_eq!(parse("w").unwrap(), Some(Command::Write { path: None }));
        assert_eq!(parse("42").unwrap(), Some(Command::Goto(42)));
        assert_eq!(parse("dark").unwrap(), Some(Command::Dark));
        assert_eq!(parse("light").unwrap(), Some(Command::Light));
        assert_eq!(parse("help").unwrap(), Some(Command::Help));
        assert_eq!(parse("recent").unwrap(), Some(Command::Recent));
        assert_eq!(parse("  ").unwrap(), None);
        assert!(parse("dark!").unwrap_err().starts_with("E477"));
    }

    #[test]
    fn paths() {
        assert_eq!(
            parse("w out.pdf").unwrap(),
            Some(Command::Write {
                path: Some(PathBuf::from("out.pdf"))
            })
        );
        assert_eq!(
            parse("e! /tmp/a b.pdf").unwrap(),
            Some(Command::Edit {
                path: Some(PathBuf::from("/tmp/a b.pdf")),
                force: true
            })
        );
        assert_eq!(
            parse("e!").unwrap(),
            Some(Command::Edit {
                path: None,
                force: true
            })
        );
    }

    #[test]
    fn pen_commands() {
        assert_eq!(
            parse("color #ff8000").unwrap(),
            Some(Command::Color([255, 128, 0]))
        );
        assert_eq!(
            parse("color #f80").unwrap(),
            Some(Command::Color([255, 136, 0]))
        );
        assert_eq!(
            parse("color Red").unwrap(),
            Some(Command::Color([0xe0, 0x31, 0x31]))
        );
        assert_eq!(parse("color 3").unwrap(), Some(Command::ColorIndex(3)));
        assert!(parse("color 0").is_err());
        assert_eq!(parse("width 2.5").unwrap(), Some(Command::Width(Some(2.5))));
        assert_eq!(parse("width").unwrap(), Some(Command::Width(None)));
        assert!(parse("color mauve").is_err());
        assert!(parse("width -1").is_err());
        assert!(parse("width abc").is_err());
        assert_eq!(parse("fontsize 12").unwrap(), Some(Command::FontSize(12.0)));
        assert_eq!(
            parse("spread auto").unwrap(),
            Some(Command::Spread(Some("auto")))
        );
        assert_eq!(parse("spread").unwrap(), Some(Command::Spread(None)));
        assert_eq!(
            parse("dir rtl").unwrap(),
            Some(Command::Direction(Some(true)))
        );
        assert!(parse("spread sideways").is_err());
    }

    #[test]
    fn unknown_command_has_vim_error() {
        let e = parse("frobnicate").unwrap_err();
        assert!(e.starts_with("E492"));
    }

    #[test]
    fn hex_colours() {
        assert_eq!(parse_hex("#000000"), Some([0, 0, 0]));
        assert_eq!(parse_hex("ffffff"), Some([255, 255, 255]));
        assert_eq!(parse_hex("#fff"), Some([255, 255, 255]));
        assert_eq!(parse_hex("#gggggg"), None);
        assert_eq!(parse_hex("#ffff"), None);
        assert_eq!(format_hex([0xe0, 0x31, 0x31]), "#e03131");
    }

    #[test]
    fn every_command_has_a_unique_name() {
        let mut all: Vec<&str> = COMMANDS
            .iter()
            .flat_map(|c| std::iter::once(c.name).chain(c.aliases.iter().copied()))
            .collect();
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n);
    }

    #[test]
    fn completion_extends_common_prefix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lecture-01.pdf"), b"").unwrap();
        std::fs::write(dir.path().join("lecture-02.pdf"), b"").unwrap();
        std::fs::write(dir.path().join("book.epub"), b"").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"").unwrap();
        let base = dir.path().display();
        let done = complete(&format!("e {base}/lec")).unwrap();
        assert_eq!(done, format!("e {base}/lecture-0"));
        let done = complete(&format!("e {base}/lecture-01")).unwrap();
        assert_eq!(done, format!("e {base}/lecture-01.pdf"));
        let done = complete(&format!("e {base}/bo")).unwrap();
        assert_eq!(done, format!("e {base}/book.epub"));
        assert!(complete(&format!("e {base}/nothing")).is_none());
        // txt files are not offered for :e
        assert!(complete(&format!("e {base}/notes")).is_none());
        assert!(complete("q foo").is_none());
    }
}
