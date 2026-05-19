// Key binding map loaded from ~/.pixel-agents/keymap.toml with hard-coded defaults.
//
// Toml format:
//   [bindings]
//   quit         = ["ctrl+alt+q"]          # one or more chords per action
//   toggle_layout = ["ctrl+alt+l"]
//   focus_office  = ["ctrl+alt+o"]

use std::collections::HashMap;

use directories::BaseDirs;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

/// Client-level actions that are reserved across all focus modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    ToggleLayout,
    FocusOffice,
}

pub struct Keymap {
    bindings: HashMap<(KeyModifiers, KeyCode), Action>,
}

impl Keymap {
    /// Load keymap: defaults first, then override from `~/.pixel-agents/keymap.toml`
    /// if it exists. Missing file → silently use defaults. Parse errors → silently
    /// ignore the file and use defaults.
    pub fn load() -> Self {
        let mut km = Self::defaults();
        if let Some(base) = BaseDirs::new() {
            let path = base.home_dir().join(".pixel-agents").join("keymap.toml");
            if let Ok(s) = std::fs::read_to_string(&path) {
                km.apply_toml(&s);
            }
        }
        km
    }

    /// Look up the action (if any) for a key event.
    pub fn matches(&self, mods: KeyModifiers, code: KeyCode) -> Option<Action> {
        self.bindings.get(&(mods, code)).copied()
    }

    fn defaults() -> Self {
        let mut b: HashMap<(KeyModifiers, KeyCode), Action> = HashMap::new();
        let ca = KeyModifiers::CONTROL | KeyModifiers::ALT;
        b.insert((ca, KeyCode::Char('q')), Action::Quit);
        b.insert((ca, KeyCode::Char('l')), Action::ToggleLayout);
        b.insert((ca, KeyCode::Char('o')), Action::FocusOffice);
        Self { bindings: b }
    }

    /// Parse a `[bindings]` toml block and merge into existing bindings.
    /// Unknown action names and un-parseable chords are silently skipped.
    fn apply_toml(&mut self, s: &str) {
        let table = match s.parse::<toml::Table>() {
            Ok(t) => t,
            Err(_) => return,
        };
        let Some(bindings) = table.get("bindings").and_then(|v| v.as_table()) else {
            return;
        };
        for (action_name, value) in bindings {
            let action = match action_name.as_str() {
                "quit" => Action::Quit,
                "toggle_layout" => Action::ToggleLayout,
                "focus_office" => Action::FocusOffice,
                _ => continue,
            };
            let chords: Vec<&str> = match value {
                toml::Value::String(s) => vec![s.as_str()],
                toml::Value::Array(arr) => arr.iter().filter_map(|v| v.as_str()).collect(),
                _ => continue,
            };
            // Remove old bindings for this action before installing new ones
            self.bindings.retain(|_, a| *a != action);
            for chord in chords {
                if let Some(key) = parse_chord(chord) {
                    self.bindings.insert(key, action);
                }
            }
        }
    }
}

/// Parse a chord string like `"ctrl+alt+q"` into `(KeyModifiers, KeyCode)`.
/// Returns `None` for unknown modifier or key names.
pub fn parse_chord(s: &str) -> Option<(KeyModifiers, KeyCode)> {
    let parts: Vec<&str> = s.split('+').collect();
    let (key_str, mod_strs) = parts.split_last()?;
    let mut mods = KeyModifiers::NONE;
    for m in mod_strs {
        match m.to_lowercase().as_str() {
            "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
            "alt" => mods |= KeyModifiers::ALT,
            "shift" => mods |= KeyModifiers::SHIFT,
            _ => return None,
        }
    }
    let code = match key_str.to_lowercase().as_str() {
        "tab" => KeyCode::Tab,
        "esc" | "escape" => KeyCode::Esc,
        "enter" | "return" => KeyCode::Enter,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        s if s.len() == 1 => KeyCode::Char(s.chars().next().unwrap()),
        _ => return None,
    };
    Some((mods, code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_include_quit() {
        let km = Keymap::defaults();
        let ca = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(km.matches(ca, KeyCode::Char('q')), Some(Action::Quit));
    }

    #[test]
    fn defaults_include_toggle_layout() {
        let km = Keymap::defaults();
        let ca = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(km.matches(ca, KeyCode::Char('l')), Some(Action::ToggleLayout));
    }

    #[test]
    fn defaults_include_focus_office() {
        let km = Keymap::defaults();
        let ca = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(km.matches(ca, KeyCode::Char('o')), Some(Action::FocusOffice));
    }

    #[test]
    fn unknown_key_returns_none() {
        let km = Keymap::defaults();
        assert_eq!(km.matches(KeyModifiers::NONE, KeyCode::Char('z')), None);
    }

    #[test]
    fn parse_chord_ctrl_alt_q() {
        let (mods, code) = parse_chord("ctrl+alt+q").unwrap();
        assert_eq!(mods, KeyModifiers::CONTROL | KeyModifiers::ALT);
        assert_eq!(code, KeyCode::Char('q'));
    }

    #[test]
    fn parse_chord_tab() {
        let (mods, code) = parse_chord("tab").unwrap();
        assert_eq!(mods, KeyModifiers::NONE);
        assert_eq!(code, KeyCode::Tab);
    }

    #[test]
    fn parse_chord_unknown_modifier() {
        assert!(parse_chord("super+q").is_none());
    }

    #[test]
    fn parse_chord_unknown_key() {
        assert!(parse_chord("ctrl+f13").is_none());
    }

    #[test]
    fn toml_override_replaces_quit() {
        let mut km = Keymap::defaults();
        km.apply_toml("[bindings]\nquit = [\"ctrl+q\"]");
        // new binding present
        assert_eq!(
            km.matches(KeyModifiers::CONTROL, KeyCode::Char('q')),
            Some(Action::Quit)
        );
        // old binding gone
        let ca = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(km.matches(ca, KeyCode::Char('q')), None);
    }

    #[test]
    fn toml_bad_parse_ignored() {
        let mut km = Keymap::defaults();
        km.apply_toml("!!! not valid toml !!!");
        // defaults still intact
        let ca = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(km.matches(ca, KeyCode::Char('q')), Some(Action::Quit));
    }

    #[test]
    fn toml_unknown_action_skipped() {
        let mut km = Keymap::defaults();
        km.apply_toml("[bindings]\nunknown_action = [\"ctrl+x\"]");
        // no crash; ctrl+x not mapped
        assert_eq!(km.matches(KeyModifiers::CONTROL, KeyCode::Char('x')), None);
    }
}
