//! Everything a key can trigger.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    None,
    // scrolling
    ScrollDown,
    ScrollUp,
    ScrollLeft,
    ScrollRight,
    HalfPageDown,
    HalfPageUp,
    PageDown,
    PageUp,
    NextPage,
    PrevPage,
    /// `gg`: first page, or page `count`.
    GotoFirst,
    /// `G`: last page, or page `count`.
    GotoLast,
    // zoom
    ZoomIn,
    ZoomOut,
    FitWidth,
    FitPage,
    Zoom100,
    // search
    SearchForward,
    SearchBackward,
    SearchNext,
    SearchPrev,
    // marks and jumps
    SetMark(char),
    JumpMark(char),
    JumpBack,
    JumpForward,
    // misc
    Outline,
    ToggleDark,
    EnterDraw,
    ExitDraw,
    Undo,
    Redo,
    Reload,
    CommandMode,
    WriteQuit,
    QuitForce,
    ClearMessage,
    // drawing
    ToggleEraser,
    PenTool,
    SelectColor(u8),
    WidthDown,
    WidthUp,
}

impl Action {
    /// Actions that consume one more key (`m{a-z}`, `'{a-z}`).
    pub fn needs_char(&self) -> bool {
        matches!(self, Action::SetMark(_) | Action::JumpMark(_))
    }

    pub fn with_char(self, c: char) -> Action {
        match self {
            Action::SetMark(_) => Action::SetMark(c),
            Action::JumpMark(_) => Action::JumpMark(c),
            other => other,
        }
    }

    /// Name used in `config.toml`.
    pub fn from_name(name: &str) -> Option<Action> {
        use Action::*;
        Some(match name {
            "none" => None,
            "scroll_down" => ScrollDown,
            "scroll_up" => ScrollUp,
            "scroll_left" => ScrollLeft,
            "scroll_right" => ScrollRight,
            "half_page_down" => HalfPageDown,
            "half_page_up" => HalfPageUp,
            "page_down" => PageDown,
            "page_up" => PageUp,
            "next_page" => NextPage,
            "prev_page" => PrevPage,
            "goto_first" => GotoFirst,
            "goto_last" => GotoLast,
            "zoom_in" => ZoomIn,
            "zoom_out" => ZoomOut,
            "fit_width" => FitWidth,
            "fit_page" => FitPage,
            "zoom_100" => Zoom100,
            "search_forward" => SearchForward,
            "search_backward" => SearchBackward,
            "search_next" => SearchNext,
            "search_prev" => SearchPrev,
            "set_mark" => SetMark('\0'),
            "jump_mark" => JumpMark('\0'),
            "jump_back" => JumpBack,
            "jump_forward" => JumpForward,
            "outline" => Outline,
            "toggle_dark" => ToggleDark,
            "enter_draw" => EnterDraw,
            "exit_draw" => ExitDraw,
            "undo" => Undo,
            "redo" => Redo,
            "reload" => Reload,
            "command_mode" => CommandMode,
            "write_quit" => WriteQuit,
            "quit_force" => QuitForce,
            "clear_message" => ClearMessage,
            "toggle_eraser" => ToggleEraser,
            "pen_tool" => PenTool,
            "width_down" => WidthDown,
            "width_up" => WidthUp,
            other => {
                // select_color_1 .. select_color_9
                let n = other.strip_prefix("select_color_")?.parse::<u8>().ok()?;
                if (1..=9).contains(&n) {
                    SelectColor(n)
                } else {
                    return Option::None;
                }
            }
        })
    }

    /// Every action name, for the README table.
    pub const NAMES: &'static [&'static str] = &[
        "none",
        "scroll_down",
        "scroll_up",
        "scroll_left",
        "scroll_right",
        "half_page_down",
        "half_page_up",
        "page_down",
        "page_up",
        "next_page",
        "prev_page",
        "goto_first",
        "goto_last",
        "zoom_in",
        "zoom_out",
        "fit_width",
        "fit_page",
        "zoom_100",
        "search_forward",
        "search_backward",
        "search_next",
        "search_prev",
        "set_mark",
        "jump_mark",
        "jump_back",
        "jump_forward",
        "outline",
        "toggle_dark",
        "enter_draw",
        "exit_draw",
        "undo",
        "redo",
        "reload",
        "command_mode",
        "write_quit",
        "quit_force",
        "clear_message",
        "toggle_eraser",
        "pen_tool",
        "select_color_1 … select_color_9",
        "width_down",
        "width_up",
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_roundtrip() {
        assert_eq!(Action::from_name("toggle_dark"), Some(Action::ToggleDark));
        assert_eq!(Action::from_name("none"), Some(Action::None));
        assert_eq!(
            Action::from_name("select_color_3"),
            Some(Action::SelectColor(3))
        );
        assert_eq!(Action::from_name("select_color_0"), Option::None);
        assert_eq!(Action::from_name("nope"), Option::None);
    }

    #[test]
    fn char_actions() {
        assert!(Action::SetMark('\0').needs_char());
        assert_eq!(Action::JumpMark('\0').with_char('a'), Action::JumpMark('a'));
        assert!(!Action::ScrollDown.needs_char());
    }
}
