//! Keyboard chords, availability, execution and documentation share this table.
use super::*;

mod catalog;
mod dispatch;
mod docs;
mod labels;
pub(super) use dispatch::{dispatch, dispatch_scope};
pub(super) use dispatch::{focused, scopes};
pub(super) use docs::{help_controls, help_text};
pub(crate) use docs::{keymap, markdown};
pub(super) fn all() -> impl Iterator<Item = &'static Action> {
    catalog::TABLE.iter().flat_map(|group| group.iter())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyChord {
    pub code: KeyCode,
    pub mods: Modifiers,
}
impl KeyChord {
    pub const fn plain(code: KeyCode) -> Self {
        Self {
            code,
            mods: Modifiers::NONE,
        }
    }
    pub const fn ctrl(code: KeyCode) -> Self {
        Self {
            code,
            mods: Modifiers::CTRL,
        }
    }
    pub fn matches(self, event: KeyEvent) -> bool {
        // Terminals report an uppercase printable code with or without SHIFT.
        let mods = if matches!(event.code, KeyCode::Char(_) | KeyCode::BackTab) {
            event.modifiers - Modifiers::SHIFT
        } else {
            event.modifiers
        };
        self.code == event.code && self.mods == mods
    }
    pub fn label(self) -> String {
        let key = match self.code {
            KeyCode::Char(' ') => "Space".into(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "Enter".into(),
            KeyCode::Escape => "Esc".into(),
            KeyCode::Up => "↑".into(),
            KeyCode::Down => "↓".into(),
            KeyCode::Left => "←".into(),
            KeyCode::Right => "→".into(),
            KeyCode::Home => "Home".into(),
            KeyCode::End => "End".into(),
            KeyCode::PageUp => "PgUp".into(),
            KeyCode::PageDown => "PgDn".into(),
            KeyCode::Tab => "Tab".into(),
            KeyCode::BackTab => "Shift+Tab".into(),
            KeyCode::Delete => "Delete".into(),
            KeyCode::F(n) => format!("F{n}"),
            _ => format!("{:?}", self.code),
        };
        if self.mods.contains(Modifiers::CTRL) {
            format!("Ctrl+{}", key.to_uppercase())
        } else {
            key
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Query,
    Results,
    Browse,
    Feeds,
    Items,
    Shows,
    Releases,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    Files,
    Providers,
    Settings,
    Help,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Screen(Screen),
    Pane(Screen, Pane),
    Modal(ModalKind),
    Prompt,
}
impl Scope {
    pub fn name(self) -> String {
        fn screen(s: Screen) -> &'static str {
            match s {
                Screen::Torrents => "torrents",
                Screen::Search => "search",
                Screen::Rss => "rss",
                Screen::Shows => "shows",
            }
        }
        match self {
            Self::Global => "global".into(),
            Self::Screen(s) => format!("screen:{}", screen(s)),
            Self::Pane(s, p) => format!("pane:{}.{}", screen(s), format!("{p:?}").to_lowercase()),
            Self::Modal(m) => format!("modal:{}", format!("{m:?}").to_lowercase()),
            Self::Prompt => "prompt".into(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Tui,
    Dashboard,
    Popup,
}
impl Surface {
    pub fn name(self) -> &'static str {
        match self {
            Self::Tui => "tui",
            Self::Dashboard => "dashboard",
            Self::Popup => "popup",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Intent {
    Down,
    Up,
    First,
    Last,
    Primary,
    Cancel,
    Left,
    Right,
    Pause,
    Sequential,
    FirstLast,
    Seed,
    Remove,
    DeleteFiles,
    Open,
    Imdb,
    Wiki,
    Follow,
    Providers,
    Browse,
    Query,
    Source,
    New,
    Refresh,
    Reload,
    Mark,
    MarkAll,
    Undo,
    Toggle,
    Include,
    High,
    Max,
    All,
    None,
    CloseUi,
    Confirm,
}

pub struct Action {
    pub id: &'static str,
    pub keys: &'static [KeyChord],
    pub scope: Scope,
    pub label: fn(&TuiApp) -> Option<String>,
    pub enabled: fn(&TuiApp) -> bool,
    pub priority: u8,
    pub run: Option<fn(&mut TuiApp) -> Cmd<Msg>>,
    pub surfaces: &'static [Surface],
    pub surface_note: Option<&'static str>,
    pub doc_label: &'static str,
    pub dashboard_keys: Option<&'static [KeyChord]>,
    pub popup_keys: Option<&'static [KeyChord]>,
}
impl Action {
    pub fn keys_for(&self, surface: Surface) -> Option<&'static [KeyChord]> {
        if !self.surfaces.contains(&surface) {
            return None;
        }
        Some(match surface {
            Surface::Tui => self.keys,
            Surface::Dashboard => self.dashboard_keys.unwrap_or(self.keys),
            Surface::Popup => self.popup_keys.unwrap_or(self.keys),
        })
    }
}

#[cfg(test)]
mod tests;
