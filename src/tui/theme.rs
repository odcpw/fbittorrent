//! Colors from the active Omarchy theme (XDG state, then ~/.local/state),
//! with a neutral fallback when not on Omarchy.

use ftui::{PackedRgba, Style};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Theme {
    pub fg: PackedRgba,
    pub bg: PackedRgba,
    pub accent_text: PackedRgba,
    pub accent_fallback: bool,
    pub low_chroma: bool,
    pub dim: PackedRgba,
    pub accent: PackedRgba,
    pub selection: PackedRgba,
    pub muted: PackedRgba,
    pub red: PackedRgba,
    pub green: PackedRgba,
    pub yellow: PackedRgba,
    pub blue: PackedRgba,
    pub cyan: PackedRgba,
    pub magenta: PackedRgba,
    pub name: String,
}

impl Default for Theme {
    fn default() -> Self {
        let mut theme = Self {
            bg: PackedRgba::rgb(0, 0, 0),
            accent_text: PackedRgba::rgb(0xca, 0xcc, 0xcc),
            accent_fallback: true,
            low_chroma: false,
            fg: PackedRgba::rgb(0xca, 0xcc, 0xcc),
            dim: PackedRgba::rgb(0x70, 0x78, 0x80),
            accent: PackedRgba::rgb(0xca, 0xcc, 0xcc),
            selection: PackedRgba::rgb(0x2a, 0x2e, 0x33),
            muted: PackedRgba::rgb(0x70, 0x78, 0x80),
            red: PackedRgba::rgb(0xa5, 0x55, 0x55),
            green: PackedRgba::rgb(0x8a, 0xa8, 0x72),
            yellow: PackedRgba::rgb(0xc9, 0xa5, 0x54),
            blue: PackedRgba::rgb(0x6c, 0x8e, 0xbf),
            cyan: PackedRgba::rgb(0x6f, 0xa8, 0xa8),
            magenta: PackedRgba::rgb(0xa0, 0x7a, 0xb0),
            name: "default".into(),
        };
        theme.roles();
        theme
    }
}

fn colors_path_for(state: Option<PathBuf>, home: Option<PathBuf>) -> PathBuf {
    state
        .unwrap_or_else(|| home.unwrap_or_default().join(".local/state"))
        .join("omarchy/current/theme/colors.toml")
}
pub fn colors_path() -> PathBuf {
    colors_path_for(
        std::env::var_os("XDG_STATE_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from),
        dirs::home_dir(),
    )
}

pub fn parse_hex(s: &str) -> Option<PackedRgba> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some(PackedRgba::rgb(r, g, b))
}

impl Theme {
    fn roles(&mut self) {
        self.dim = super::palette::dim(self.fg, self.bg).0;
        self.accent_fallback = super::palette::accent_text(self.accent, self.fg, self.bg);
        self.accent_text = if self.accent_fallback {
            self.fg
        } else {
            self.accent
        };
        self.low_chroma = super::palette::low_chroma(self);
    }
    pub fn load() -> Self {
        let Ok(raw) = std::fs::read_to_string(colors_path()) else {
            return Self::default();
        };
        let mut theme = Self::from_toml(&raw).unwrap_or_default();
        theme.name = std::fs::read_link(colors_path().parent().unwrap())
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "omarchy".into());
        theme
    }
    pub(super) fn from_toml(raw: &str) -> Option<Self> {
        let mut t = Self::default();
        let v = raw.parse::<toml::Table>().ok()?;
        let get = |k: &str| v.get(k).and_then(|x| x.as_str()).and_then(parse_hex);
        if let Some(c) = get("foreground") {
            t.fg = c;
        }
        if let Some(c) = get("background") {
            t.bg = c;
        }
        if let Some(c) = get("accent") {
            t.accent = c;
        }
        if let Some(c) = get("selection") {
            t.selection = c;
        }
        if let Some(c) = get("muted") {
            t.muted = c;
        }
        if let Some(c) = get("red") {
            t.red = c;
        }
        if let Some(c) = get("green") {
            t.green = c;
        }
        if let Some(c) = get("yellow") {
            t.yellow = c;
        }
        if let Some(c) = get("blue") {
            t.blue = c;
        }
        if let Some(c) = get("cyan") {
            t.cyan = c;
        }
        if let Some(c) = get("magenta") {
            t.magenta = c;
        }
        t.roles();
        Some(t)
    }

    pub fn text(&self) -> Style {
        Style::new().fg(self.fg)
    }
    pub fn dimmed(&self) -> Style {
        Style::new().fg(self.dim)
    }
    pub fn accented(&self) -> Style {
        Style::new().fg(self.accent_text).bold()
    }
    pub fn selected(&self) -> Style {
        Style::new().bg(self.selection).fg(self.fg).bold()
    }
    pub fn border(&self) -> Style {
        Style::new().fg(self.dim)
    }
    pub fn border_focused(&self) -> Style {
        if self.accent_fallback {
            self.text().bold()
        } else {
            Style::new().fg(self.accent_text)
        }
    }
    pub fn title(&self) -> Style {
        Style::new().fg(self.fg).bold()
    }
    pub fn ok(&self) -> Style {
        if self.low_chroma {
            self.text()
        } else {
            Style::new().fg(self.green)
        }
    }
    pub fn warn(&self) -> Style {
        if self.low_chroma {
            self.text()
        } else {
            Style::new().fg(self.yellow)
        }
    }
    pub fn err(&self) -> Style {
        if self.low_chroma {
            self.text().bold().reverse()
        } else {
            Style::new().fg(self.red)
        }
    }
    pub(super) fn error_message(&self, message: &str) -> String {
        if self.low_chroma && !message.starts_with('!') {
            format!("! {message}")
        } else {
            message.to_owned()
        }
    }
    pub fn key(&self) -> Style {
        Style::new().fg(self.accent_text).bold()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_home_precedes_the_home_fallback() {
        let state = PathBuf::from("/private/state");
        let home = PathBuf::from("/private/home");
        assert_eq!(
            colors_path_for(Some(state.clone()), Some(home.clone())),
            state.join("omarchy/current/theme/colors.toml")
        );
        assert_eq!(
            colors_path_for(None, Some(home.clone())),
            home.join(".local/state/omarchy/current/theme/colors.toml")
        );
    }
    #[test]
    fn muted_and_dark_foreground_are_never_secondary_text() {
        let theme=Theme::from_toml("foreground='#ffffff'\nbackground='#000000'\nmuted='#111111'\ndark_foreground='#222222'\naccent='#8d8d8d'").unwrap();
        assert_eq!(theme.dim, PackedRgba::rgb(182, 182, 182));
        assert_ne!(theme.dim, theme.muted);
        for input in ["白白", "#fff", "#ffffffff", "not a color"] {
            assert!(parse_hex(input).is_none());
        }
    }
}
