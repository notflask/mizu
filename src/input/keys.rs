//! Key representation and Vim-style notation (`<C-d>`, `<Esc>`, `gg`).

use std::fmt;

use winit::keyboard::{Key as WKey, ModifiersState, NamedKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Char(char),
    Esc,
    Enter,
    Tab,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    F(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: KeyCode,
    pub mods: Mods,
}

impl Key {
    pub fn new(code: KeyCode, mods: Mods) -> Self {
        Key { code, mods }.normalized()
    }

    pub fn ch(c: char) -> Self {
        Key {
            code: KeyCode::Char(c),
            mods: Mods::NONE,
        }
    }

    /// Characters carry their own case, so `shift` is dropped for them
    /// (except for space). `<C-D>` is the same as `<C-d>`.
    fn normalized(mut self) -> Self {
        if let KeyCode::Char(c) = self.code {
            if c != ' ' {
                self.mods.shift = false;
            }
            if self.mods.ctrl && c.is_ascii_uppercase() {
                self.code = KeyCode::Char(c.to_ascii_lowercase());
            }
        }
        self
    }

    /// Printable text typed with this key, for text-entry modes.
    pub fn text(&self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if !self.mods.ctrl && !self.mods.alt => Some(c),
            _ => None,
        }
    }

    /// Convert a winit logical key. Returns `None` for pure modifier keys and
    /// anything we do not map.
    pub fn from_winit(key: &WKey, mods: ModifiersState) -> Option<Key> {
        let m = Mods {
            ctrl: mods.control_key(),
            alt: mods.alt_key(),
            shift: mods.shift_key(),
        };
        let code = match key {
            WKey::Character(s) => {
                let mut it = s.chars();
                let c = it.next()?;
                if it.next().is_some() {
                    return None;
                }
                KeyCode::Char(c)
            }
            WKey::Named(n) => match n {
                NamedKey::Escape => KeyCode::Esc,
                NamedKey::Enter => KeyCode::Enter,
                NamedKey::Tab => KeyCode::Tab,
                NamedKey::Backspace => KeyCode::Backspace,
                NamedKey::Delete => KeyCode::Delete,
                NamedKey::Space => KeyCode::Char(' '),
                NamedKey::ArrowUp => KeyCode::Up,
                NamedKey::ArrowDown => KeyCode::Down,
                NamedKey::ArrowLeft => KeyCode::Left,
                NamedKey::ArrowRight => KeyCode::Right,
                NamedKey::PageUp => KeyCode::PageUp,
                NamedKey::PageDown => KeyCode::PageDown,
                NamedKey::Home => KeyCode::Home,
                NamedKey::End => KeyCode::End,
                NamedKey::F1 => KeyCode::F(1),
                NamedKey::F2 => KeyCode::F(2),
                NamedKey::F3 => KeyCode::F(3),
                NamedKey::F4 => KeyCode::F(4),
                NamedKey::F5 => KeyCode::F(5),
                NamedKey::F6 => KeyCode::F(6),
                NamedKey::F7 => KeyCode::F(7),
                NamedKey::F8 => KeyCode::F(8),
                NamedKey::F9 => KeyCode::F(9),
                NamedKey::F10 => KeyCode::F(10),
                NamedKey::F11 => KeyCode::F(11),
                NamedKey::F12 => KeyCode::F(12),
                _ => return None,
            },
            _ => return None,
        };
        // Shift+Tab arrives as its own logical key on some layouts; keep it simple.
        Some(Key::new(code, m))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name: Option<String> = match self.code {
            KeyCode::Char(' ') => Some("Space".into()),
            KeyCode::Char('<') => Some("lt".into()),
            KeyCode::Char(_) => None,
            KeyCode::Esc => Some("Esc".into()),
            KeyCode::Enter => Some("CR".into()),
            KeyCode::Tab => Some("Tab".into()),
            KeyCode::Backspace => Some("BS".into()),
            KeyCode::Delete => Some("Del".into()),
            KeyCode::Up => Some("Up".into()),
            KeyCode::Down => Some("Down".into()),
            KeyCode::Left => Some("Left".into()),
            KeyCode::Right => Some("Right".into()),
            KeyCode::PageUp => Some("PageUp".into()),
            KeyCode::PageDown => Some("PageDown".into()),
            KeyCode::Home => Some("Home".into()),
            KeyCode::End => Some("End".into()),
            KeyCode::F(n) => Some(format!("F{n}")),
        };
        let special = name.is_some() || self.mods.ctrl || self.mods.alt || self.mods.shift;
        if !special {
            if let KeyCode::Char(c) = self.code {
                return write!(f, "{c}");
            }
        }
        write!(f, "<")?;
        if self.mods.ctrl {
            write!(f, "C-")?;
        }
        if self.mods.alt {
            write!(f, "A-")?;
        }
        if self.mods.shift {
            write!(f, "S-")?;
        }
        match (name, self.code) {
            (Some(n), _) => write!(f, "{n}")?,
            (None, KeyCode::Char(c)) => write!(f, "{c}")?,
            _ => {}
        }
        write!(f, ">")
    }
}

/// Parse a Vim-style sequence like `gg`, `<C-d>`, `<S-Space>` or `zw`.
pub fn parse_seq(s: &str) -> Result<Vec<Key>, String> {
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' {
            if let Some(end) = chars[i..].iter().position(|&c| c == '>') {
                // `<>` and a lone `<` followed by non-key text are plain characters.
                let inner: String = chars[i + 1..i + end].iter().collect();
                if !inner.is_empty() {
                    out.push(parse_bracketed(&inner)?);
                    i += end + 1;
                    continue;
                }
            }
        }
        out.push(Key::ch(chars[i]));
        i += 1;
    }
    if out.is_empty() {
        return Err("empty key sequence".into());
    }
    Ok(out)
}

fn parse_bracketed(inner: &str) -> Result<Key, String> {
    let mut mods = Mods::NONE;
    let mut rest = inner;
    loop {
        let lower = rest.to_ascii_lowercase();
        if lower.starts_with("c-") && rest.len() > 2 {
            mods.ctrl = true;
            rest = &rest[2..];
        } else if lower.starts_with("a-") && rest.len() > 2
            || lower.starts_with("m-") && rest.len() > 2
        {
            mods.alt = true;
            rest = &rest[2..];
        } else if lower.starts_with("s-") && rest.len() > 2 {
            mods.shift = true;
            rest = &rest[2..];
        } else {
            break;
        }
    }
    let code = match rest.to_ascii_lowercase().as_str() {
        "esc" => KeyCode::Esc,
        "cr" | "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "bs" | "backspace" => KeyCode::Backspace,
        "del" | "delete" => KeyCode::Delete,
        "space" => KeyCode::Char(' '),
        "lt" => KeyCode::Char('<'),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        f if f.starts_with('f') && f[1..].parse::<u8>().is_ok() => {
            let n: u8 = f[1..].parse().unwrap_or(1);
            if !(1..=12).contains(&n) {
                return Err(format!("unknown key <{inner}>"));
            }
            KeyCode::F(n)
        }
        _ => {
            let mut it = rest.chars();
            match (it.next(), it.next()) {
                (Some(c), None) => KeyCode::Char(c),
                _ => return Err(format!("unknown key <{inner}>")),
            }
        }
    };
    Ok(Key::new(code, mods))
}

pub fn format_seq(seq: &[Key]) -> String {
    seq.iter().map(|k| k.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_chars() {
        let s = parse_seq("gg").unwrap();
        assert_eq!(s, vec![Key::ch('g'), Key::ch('g')]);
        assert_eq!(format_seq(&s), "gg");
    }

    #[test]
    fn ctrl_and_named() {
        let s = parse_seq("<C-d>").unwrap();
        assert_eq!(s.len(), 1);
        assert!(s[0].mods.ctrl);
        assert_eq!(s[0].code, KeyCode::Char('d'));
        assert_eq!(format_seq(&s), "<C-d>");
        assert_eq!(parse_seq("<C-D>").unwrap(), s);
        assert_eq!(parse_seq("<Esc>").unwrap()[0].code, KeyCode::Esc);
        assert_eq!(parse_seq("<CR>").unwrap()[0].code, KeyCode::Enter);
    }

    #[test]
    fn shift_space() {
        let k = parse_seq("<S-Space>").unwrap()[0];
        assert!(k.mods.shift);
        assert_eq!(k.code, KeyCode::Char(' '));
        assert_eq!(k.to_string(), "<S-Space>");
        assert_eq!(Key::ch(' ').to_string(), "<Space>");
    }

    #[test]
    fn mixed_sequences() {
        let s = parse_seq("z<C-w>x").unwrap();
        assert_eq!(s.len(), 3);
        assert_eq!(format_seq(&s), "z<C-w>x");
    }

    #[test]
    fn literal_angle_brackets() {
        assert_eq!(parse_seq("<").unwrap(), vec![Key::ch('<')]);
        assert_eq!(parse_seq("<lt>").unwrap(), vec![Key::ch('<')]);
        assert_eq!(parse_seq(">").unwrap(), vec![Key::ch('>')]);
    }

    #[test]
    fn function_keys() {
        assert_eq!(parse_seq("<F5>").unwrap()[0].code, KeyCode::F(5));
        assert!(parse_seq("<F99>").is_err());
        assert!(parse_seq("<Bogus>").is_err());
        assert!(parse_seq("").is_err());
    }

    #[test]
    fn uppercase_char_drops_shift() {
        let k = Key::new(
            KeyCode::Char('G'),
            Mods {
                shift: true,
                ..Mods::NONE
            },
        );
        assert_eq!(k, Key::ch('G'));
    }
}
