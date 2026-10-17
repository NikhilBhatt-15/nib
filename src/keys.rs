//! Key chords as text ("Ctrl+Shift+P") and back, for keybindings and the
//! global scratchpad shortcut.

use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

pub const M_CTRL: u8 = 1;
pub const M_ALT: u8 = 2;
pub const M_SHIFT: u8 = 4;
pub const M_WIN: u8 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chord {
    pub mods: u8,
    pub vk: u16,
}

const NAMED: &[(u16, &str)] = &[
    (VK_SPACE, "Space"),
    (VK_TAB, "Tab"),
    (VK_RETURN, "Enter"),
    (VK_ESCAPE, "Esc"),
    (VK_BACK, "Backspace"),
    (VK_INSERT, "Insert"),
    (VK_DELETE, "Delete"),
    (VK_HOME, "Home"),
    (VK_END, "End"),
    (VK_PRIOR, "PageUp"),
    (VK_NEXT, "PageDown"),
    (VK_UP, "Up"),
    (VK_DOWN, "Down"),
    (VK_LEFT, "Left"),
    (VK_RIGHT, "Right"),
    (VK_OEM_3, "`"),
    (VK_OEM_MINUS, "-"),
    (VK_OEM_PLUS, "="),
    (VK_OEM_4, "["),
    (VK_OEM_6, "]"),
    (VK_OEM_5, "\\"),
    (VK_OEM_1, ";"),
    (VK_OEM_7, "'"),
    (VK_OEM_COMMA, ","),
    (VK_OEM_PERIOD, "."),
    (VK_OEM_2, "/"),
    (VK_ADD, "NumPlus"),
    (VK_SUBTRACT, "NumMinus"),
];

pub fn key_name(vk: u16) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from(vk as u8).to_string(),
        v if (VK_F1..=VK_F24).contains(&v) => format!("F{}", v - VK_F1 + 1),
        v => NAMED.iter().find(|(k, _)| *k == v).map(|(_, n)| n.to_string()).unwrap_or_else(|| format!("0x{v:02X}")),
    }
}

fn key_from_name(name: &str) -> Option<u16> {
    let up = name.to_ascii_uppercase();
    if up.len() == 1 {
        let c = up.as_bytes()[0];
        if c.is_ascii_alphanumeric() {
            return Some(c as u16);
        }
    }
    if let Some(n) = up.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()) {
        if (1..=24).contains(&n) {
            return Some(VK_F1 + n - 1);
        }
    }
    match up.as_str() {
        "PLUS" => return Some(VK_OEM_PLUS),
        "MINUS" => return Some(VK_OEM_MINUS),
        "ESCAPE" => return Some(VK_ESCAPE),
        "RETURN" => return Some(VK_RETURN),
        "DEL" => return Some(VK_DELETE),
        "PGUP" => return Some(VK_PRIOR),
        "PGDN" => return Some(VK_NEXT),
        _ => {}
    }
    NAMED.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).map(|(k, _)| *k)
}

impl Chord {
    pub fn parse(s: &str) -> Option<Chord> {
        let mut mods = 0;
        let mut vk = None;
        let s = s.trim();
        // "Ctrl++" style: a trailing '+' key.
        let (body, plus_key) = match s.strip_suffix("++") {
            Some(b) => (b, true),
            None => (s, false),
        };
        for part in body.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods |= M_CTRL,
                "shift" => mods |= M_SHIFT,
                "alt" => mods |= M_ALT,
                "win" | "windows" | "super" | "meta" => mods |= M_WIN,
                _ => {
                    if vk.is_some() {
                        return None;
                    }
                    vk = Some(key_from_name(part)?);
                }
            }
        }
        if plus_key {
            if vk.is_some() {
                return None;
            }
            vk = Some(VK_OEM_PLUS);
        }
        Some(Chord { mods, vk: vk? })
    }

    pub fn display(&self) -> String {
        let mut parts = Vec::new();
        for (m, n) in [(M_WIN, "Win"), (M_CTRL, "Ctrl"), (M_ALT, "Alt"), (M_SHIFT, "Shift")] {
            if self.mods & m != 0 {
                parts.push(n.to_string());
            }
        }
        parts.push(key_name(self.vk));
        parts.join("+")
    }

    /// The chord being pressed right now for virtual key `vk`.
    pub fn current(vk: u16) -> Chord {
        let down = |k: u16| unsafe { GetKeyState(k as i32) } < 0;
        let mut mods = 0;
        if down(VK_CONTROL) {
            mods |= M_CTRL;
        }
        if down(VK_MENU) {
            mods |= M_ALT;
        }
        if down(VK_SHIFT) {
            mods |= M_SHIFT;
        }
        if down(VK_LWIN) || down(VK_RWIN) {
            mods |= M_WIN;
        }
        Chord { mods, vk }
    }

    /// RegisterHotKey modifiers.
    pub fn hot_mods(&self) -> HOT_KEY_MODIFIERS {
        let mut m = MOD_NOREPEAT;
        if self.mods & M_CTRL != 0 {
            m |= MOD_CONTROL;
        }
        if self.mods & M_ALT != 0 {
            m |= MOD_ALT;
        }
        if self.mods & M_SHIFT != 0 {
            m |= MOD_SHIFT;
        }
        if self.mods & M_WIN != 0 {
            m |= MOD_WIN;
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for s in ["Ctrl+Shift+P", "Win+Alt+N", "F5", "Alt+Up", "Ctrl+/", "Ctrl+\\", "Shift+F3", "Ctrl+Alt+S", "Ctrl+="] {
            assert_eq!(Chord::parse(s).unwrap().display(), s);
        }
        assert_eq!(Chord::parse("ctrl++"), Chord::parse("Ctrl+="));
        assert_eq!(Chord::parse("shift + ctrl + p"), Chord::parse("Ctrl+Shift+P"));
        assert!(Chord::parse("Ctrl+A+B").is_none());
        assert!(Chord::parse("Ctrl+Banana").is_none());
    }
}
