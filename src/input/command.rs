//! Parser for `:` commands and path completion.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Write { path: Option<PathBuf> },
    Quit { force: bool },
    WriteQuit,
    /// `:x`: write only when something changed, then quit.
    Exit,
    Edit { path: Option<PathBuf>, force: bool },
    Goto(usize),
    Dark,
    Light,
    Color([u8; 3]),
    Width(f32),
}

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
    let path = || -> Option<PathBuf> {
        if rest.is_empty() {
            None
        } else {
            Some(expand_tilde(rest))
        }
    };
    let cmd = match name {
        "w" | "write" => Command::Write { path: path() },
        "q" | "quit" => Command::Quit { force: bang },
        "wq" => Command::WriteQuit,
        "x" | "xit" | "exit" => Command::Exit,
        "e" | "edit" => Command::Edit {
            path: path(),
            force: bang,
        },
        "dark" => Command::Dark,
        "light" => Command::Light,
        "color" | "colour" => Command::Color(parse_hex(rest).ok_or_else(|| {
            "E474: Invalid argument: expected #rrggbb".to_string()
        })?),
        "width" => {
            let w: f32 = rest
                .parse()
                .map_err(|_| "E474: Invalid argument: expected a number".to_string())?;
            if !(w.is_finite() && w > 0.0) {
                return Err("E474: Invalid argument: width must be positive".into());
            }
            Command::Width(w)
        }
        _ => return Err(format!("E492: Not an editor command: {name}")),
    };
    Ok(Some(cmd))
}

pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub fn expand_tilde(s: &str) -> PathBuf {
    if s == "~" || s.starts_with("~/") {
        if let Some(home) = directories::UserDirs::new().map(|d| d.home_dir().to_path_buf()) {
            return home.join(s.trim_start_matches('~').trim_start_matches('/'));
        }
    }
    PathBuf::from(s)
}

/// Complete the path argument of `:e` / `:w`. Returns the completed command
/// line (extended by the longest common prefix of all candidates).
pub fn complete(line: &str) -> Option<String> {
    let (cmd, arg) = line.split_once(' ')?;
    if !matches!(cmd.trim_end_matches('!'), "e" | "edit" | "w" | "write") {
        return None;
    }
    let arg = arg.trim_start();
    let expanded = expand_tilde(arg).to_string_lossy().into_owned();
    // Keep what the user typed (including ~) as prefix of the result.
    let (dir_typed, file_prefix) = match arg.rfind('/') {
        Some(i) => (&arg[..=i], &arg[i + 1..]),
        None => ("", arg),
    };
    let dir_path = match expanded.rfind('/') {
        Some(i) => PathBuf::from(&expanded[..=i]),
        None => PathBuf::from("."),
    };
    let mut names: Vec<(String, bool)> = std::fs::read_dir(&dir_path)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with(file_prefix) {
                return None;
            }
            if file_prefix.is_empty() && name.starts_with('.') {
                return None;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let is_pdf = Path::new(&name)
                .extension()
                .map(|x| x.eq_ignore_ascii_case("pdf"))
                .unwrap_or(false);
            (is_dir || is_pdf || cmd.starts_with('w')).then_some((name, is_dir))
        })
        .collect();
    if names.is_empty() {
        return None;
    }
    names.sort();
    let common = common_prefix(names.iter().map(|(n, _)| n.as_str()));
    let mut completed = format!("{dir_typed}{common}");
    if names.len() == 1 && names[0].1 {
        completed.push('/');
    }
    Some(format!("{cmd} {completed}"))
}

fn common_prefix<'a>(mut it: impl Iterator<Item = &'a str>) -> String {
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
        assert_eq!(parse("w").unwrap(), Some(Command::Write { path: None }));
        assert_eq!(parse("42").unwrap(), Some(Command::Goto(42)));
        assert_eq!(parse("dark").unwrap(), Some(Command::Dark));
        assert_eq!(parse("light").unwrap(), Some(Command::Light));
        assert_eq!(parse("  ").unwrap(), None);
    }

    #[test]
    fn paths() {
        assert_eq!(
            parse("w out.pdf").unwrap(),
            Some(Command::Write { path: Some(PathBuf::from("out.pdf")) })
        );
        assert_eq!(
            parse("e! /tmp/a b.pdf").unwrap(),
            Some(Command::Edit { path: Some(PathBuf::from("/tmp/a b.pdf")), force: true })
        );
        assert_eq!(parse("e!").unwrap(), Some(Command::Edit { path: None, force: true }));
    }

    #[test]
    fn pen_commands() {
        assert_eq!(parse("color #ff8000").unwrap(), Some(Command::Color([255, 128, 0])));
        assert_eq!(parse("width 2.5").unwrap(), Some(Command::Width(2.5)));
        assert!(parse("color red").is_err());
        assert!(parse("width -1").is_err());
        assert!(parse("width abc").is_err());
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
        assert_eq!(parse_hex("#fff"), None);
        assert_eq!(parse_hex("#gggggg"), None);
    }

    #[test]
    fn completion_extends_common_prefix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lecture-01.pdf"), b"").unwrap();
        std::fs::write(dir.path().join("lecture-02.pdf"), b"").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"").unwrap();
        let base = dir.path().display();
        let done = complete(&format!("e {base}/lec")).unwrap();
        assert_eq!(done, format!("e {base}/lecture-0"));
        let done = complete(&format!("e {base}/lecture-01")).unwrap();
        assert_eq!(done, format!("e {base}/lecture-01.pdf"));
        assert!(complete(&format!("e {base}/nothing")).is_none());
        // txt files are not offered for :e
        assert!(complete(&format!("e {base}/notes")).is_none());
        assert!(complete("q foo").is_none());
    }
}
