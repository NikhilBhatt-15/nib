//! Colour themes for the editor and the window chrome. "system" follows the
//! Windows light/dark app setting and accent colour.

use std::mem::size_of;
use std::ptr::null_mut;
use windows_sys::w;
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn hex(v: u32) -> Self {
        Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }
    pub fn colorref(self) -> u32 {
        self.0 as u32 | (self.1 as u32) << 8 | (self.2 as u32) << 16
    }
    /// GDI+ ARGB.
    pub fn argb(self, alpha: u8) -> u32 {
        (alpha as u32) << 24 | (self.0 as u32) << 16 | (self.1 as u32) << 8 | self.2 as u32
    }
    /// Scintilla ColourAlpha (0xAABBGGRR).
    pub fn argb_abgr(self, alpha: u8) -> u32 {
        self.colorref() | (alpha as u32) << 24
    }
    /// `self` over `under` at `t` (0..=1) opacity.
    pub fn mix(self, under: Rgb, t: f32) -> Rgb {
        let m = |a: u8, b: u8| (a as f32 * t + b as f32 * (1.0 - t)).round() as u8;
        Rgb(m(self.0, under.0), m(self.1, under.1), m(self.2, under.2))
    }
}

/// Syntax token categories that lexer styles are mapped onto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Comment,
    Keyword,
    Type,
    String,
    Number,
    Operator,
    Preproc,
    Function,
    Variable,
    Constant,
    Tag,
    Attribute,
    Heading,
    Strong,
    Emphasis,
    Code,
    Link,
    Quote,
    List,
    Added,
    Deleted,
    Error,
}

#[derive(Clone, Copy, Debug)]
pub struct Syntax {
    pub comment: Rgb,
    pub keyword: Rgb,
    pub ty: Rgb,
    pub string: Rgb,
    pub number: Rgb,
    pub operator: Rgb,
    pub preproc: Rgb,
    pub function: Rgb,
    pub variable: Rgb,
    pub constant: Rgb,
    pub tag: Rgb,
    pub attribute: Rgb,
    pub heading: Rgb,
    pub link: Rgb,
    pub added: Rgb,
    pub deleted: Rgb,
    pub error: Rgb,
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    /// Editor background and text.
    pub bg: Rgb,
    pub fg: Rgb,
    /// Window chrome: tab strip, status bar, panels.
    pub chrome: Rgb,
    /// Raised surfaces: active tab, inputs, popups.
    pub surface: Rgb,
    pub border: Rgb,
    pub subtext: Rgb,
    pub accent: Rgb,
    pub selection: Rgb,
    pub gutter_fg: Rgb,
    pub whitespace: Rgb,
    pub match_bg: Rgb,
    pub warning: Rgb,
    pub syntax: Syntax,
}

pub const THEMES: &[(&str, &str)] = &[
    ("system", "System (light/dark)"),
    ("light", "Light"),
    ("dark", "Dark"),
    ("github-light", "GitHub Light"),
    ("solarized-light", "Solarized Light"),
    ("one-dark", "One Dark"),
    ("dracula", "Dracula"),
    ("nord", "Nord"),
    ("monokai", "Monokai"),
    ("gruvbox-dark", "Gruvbox Dark"),
    ("catppuccin-mocha", "Catppuccin Mocha"),
    ("solarized-dark", "Solarized Dark"),
];

const LIGHT_SYNTAX: Syntax = Syntax {
    comment: Rgb::hex(0x6a737d),
    keyword: Rgb::hex(0xaf00db),
    ty: Rgb::hex(0x267f99),
    string: Rgb::hex(0xa31515),
    number: Rgb::hex(0x098658),
    operator: Rgb::hex(0x3b3b3b),
    preproc: Rgb::hex(0x795e26),
    function: Rgb::hex(0x795e26),
    variable: Rgb::hex(0x001080),
    constant: Rgb::hex(0x0070c1),
    tag: Rgb::hex(0x800000),
    attribute: Rgb::hex(0xe50000),
    heading: Rgb::hex(0x0451a5),
    link: Rgb::hex(0x0066cc),
    added: Rgb::hex(0x22863a),
    deleted: Rgb::hex(0xcb2431),
    error: Rgb::hex(0xe51400),
};

const DARK_SYNTAX: Syntax = Syntax {
    comment: Rgb::hex(0x6a9955),
    keyword: Rgb::hex(0xc586c0),
    ty: Rgb::hex(0x4ec9b0),
    string: Rgb::hex(0xce9178),
    number: Rgb::hex(0xb5cea8),
    operator: Rgb::hex(0xd4d4d4),
    preproc: Rgb::hex(0xdcdcaa),
    function: Rgb::hex(0xdcdcaa),
    variable: Rgb::hex(0x9cdcfe),
    constant: Rgb::hex(0x569cd6),
    tag: Rgb::hex(0x569cd6),
    attribute: Rgb::hex(0x9cdcfe),
    heading: Rgb::hex(0x569cd6),
    link: Rgb::hex(0x4fc1ff),
    added: Rgb::hex(0x81b88b),
    deleted: Rgb::hex(0xf48771),
    error: Rgb::hex(0xf44747),
};

impl Theme {
    #[allow(clippy::too_many_arguments)]
    fn build(dark: bool, bg: Rgb, fg: Rgb, chrome: Rgb, accent: Rgb, syntax: Syntax) -> Self {
        let surface = if dark { fg.mix(bg, 0.07) } else { Rgb::hex(0xffffff).mix(bg, 0.6) };
        Theme {
            dark,
            bg,
            fg,
            chrome,
            surface,
            border: fg.mix(chrome, if dark { 0.14 } else { 0.12 }),
            subtext: fg.mix(bg, 0.62),
            accent,
            selection: accent.mix(bg, if dark { 0.38 } else { 0.25 }),
            gutter_fg: fg.mix(bg, 0.38),
            whitespace: fg.mix(bg, 0.25),
            match_bg: if dark { Rgb::hex(0x9e6a03) } else { Rgb::hex(0xffd33d) },
            warning: if dark { Rgb::hex(0xcca700) } else { Rgb::hex(0x9a6700) },
            syntax,
        }
    }

    pub fn named(name: &str) -> Self {
        let s = |v: [u32; 17]| Syntax {
            comment: Rgb::hex(v[0]),
            keyword: Rgb::hex(v[1]),
            ty: Rgb::hex(v[2]),
            string: Rgb::hex(v[3]),
            number: Rgb::hex(v[4]),
            operator: Rgb::hex(v[5]),
            preproc: Rgb::hex(v[6]),
            function: Rgb::hex(v[7]),
            variable: Rgb::hex(v[8]),
            constant: Rgb::hex(v[9]),
            tag: Rgb::hex(v[10]),
            attribute: Rgb::hex(v[11]),
            heading: Rgb::hex(v[12]),
            link: Rgb::hex(v[13]),
            added: Rgb::hex(v[14]),
            deleted: Rgb::hex(v[15]),
            error: Rgb::hex(v[16]),
        };
        match name {
            "light" => Self::build(false, Rgb::hex(0xffffff), Rgb::hex(0x1f1f1f), Rgb::hex(0xf3f3f3), system_accent(false), LIGHT_SYNTAX),
            "dark" => Self::build(true, Rgb::hex(0x1e1e1e), Rgb::hex(0xd4d4d4), Rgb::hex(0x252526), system_accent(true), DARK_SYNTAX),
            "github-light" => Self::build(false,
                Rgb::hex(0xffffff),
                Rgb::hex(0x24292f),
                Rgb::hex(0xf6f8fa),
                Rgb::hex(0x0969da),
                s([0x6e7781, 0xcf222e, 0x953800, 0x0a3069, 0x0550ae, 0x24292f, 0x8250df, 0x8250df, 0x953800, 0x0550ae, 0x116329, 0x0550ae, 0x0550ae, 0x0969da, 0x116329, 0x82071e, 0xcf222e]),
            ),
            "solarized-light" => Self::build(false,
                Rgb::hex(0xfdf6e3),
                Rgb::hex(0x586e75),
                Rgb::hex(0xeee8d5),
                Rgb::hex(0x268bd2),
                s([0x93a1a1, 0x859900, 0xb58900, 0x2aa198, 0xd33682, 0x657b83, 0xcb4b16, 0x268bd2, 0x268bd2, 0x6c71c4, 0x268bd2, 0x93a1a1, 0xcb4b16, 0x268bd2, 0x859900, 0xdc322f, 0xdc322f]),
            ),
            "solarized-dark" => Self::build(true,
                Rgb::hex(0x002b36),
                Rgb::hex(0x93a1a1),
                Rgb::hex(0x073642),
                Rgb::hex(0x2aa198),
                s([0x586e75, 0x859900, 0xb58900, 0x2aa198, 0xd33682, 0x839496, 0xcb4b16, 0x268bd2, 0x268bd2, 0x6c71c4, 0x268bd2, 0x93a1a1, 0xcb4b16, 0x268bd2, 0x859900, 0xdc322f, 0xdc322f]),
            ),
            "one-dark" => Self::build(true,
                Rgb::hex(0x282c34),
                Rgb::hex(0xabb2bf),
                Rgb::hex(0x21252b),
                Rgb::hex(0x61afef),
                s([0x7f848e, 0xc678dd, 0xe5c07b, 0x98c379, 0xd19a66, 0x56b6c2, 0xc678dd, 0x61afef, 0xe06c75, 0xd19a66, 0xe06c75, 0xd19a66, 0xe06c75, 0x61afef, 0x98c379, 0xe06c75, 0xf44747]),
            ),
            "dracula" => Self::build(true,
                Rgb::hex(0x282a36),
                Rgb::hex(0xf8f8f2),
                Rgb::hex(0x21222c),
                Rgb::hex(0xbd93f9),
                s([0x6272a4, 0xff79c6, 0x8be9fd, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0xff79c6, 0x50fa7b, 0xf8f8f2, 0xbd93f9, 0xff79c6, 0x50fa7b, 0xbd93f9, 0x8be9fd, 0x50fa7b, 0xff5555, 0xff5555]),
            ),
            "nord" => Self::build(true,
                Rgb::hex(0x2e3440),
                Rgb::hex(0xd8dee9),
                Rgb::hex(0x272c36),
                Rgb::hex(0x88c0d0),
                s([0x616e88, 0x81a1c1, 0x8fbcbb, 0xa3be8c, 0xb48ead, 0x81a1c1, 0x5e81ac, 0x88c0d0, 0xd8dee9, 0x81a1c1, 0x81a1c1, 0x8fbcbb, 0x88c0d0, 0x88c0d0, 0xa3be8c, 0xbf616a, 0xbf616a]),
            ),
            "monokai" => Self::build(true,
                Rgb::hex(0x272822),
                Rgb::hex(0xf8f8f2),
                Rgb::hex(0x1e1f1c),
                Rgb::hex(0xa6e22e),
                s([0x88846f, 0xf92672, 0x66d9ef, 0xe6db74, 0xae81ff, 0xf92672, 0xf92672, 0xa6e22e, 0xfd971f, 0xae81ff, 0xf92672, 0xa6e22e, 0xa6e22e, 0x66d9ef, 0xa6e22e, 0xf92672, 0xf44747]),
            ),
            "gruvbox-dark" => Self::build(true,
                Rgb::hex(0x282828),
                Rgb::hex(0xebdbb2),
                Rgb::hex(0x1d2021),
                Rgb::hex(0xfabd2f),
                s([0x928374, 0xfb4934, 0xfabd2f, 0xb8bb26, 0xd3869b, 0xfe8019, 0x8ec07c, 0xb8bb26, 0x83a598, 0xd3869b, 0x8ec07c, 0xfabd2f, 0xfabd2f, 0x83a598, 0xb8bb26, 0xfb4934, 0xfb4934]),
            ),
            "catppuccin-mocha" => Self::build(true,
                Rgb::hex(0x1e1e2e),
                Rgb::hex(0xcdd6f4),
                Rgb::hex(0x181825),
                Rgb::hex(0xcba6f7),
                s([0x7f849c, 0xcba6f7, 0xf9e2af, 0xa6e3a1, 0xfab387, 0x89dceb, 0xf5c2e7, 0x89b4fa, 0xcdd6f4, 0xfab387, 0x89b4fa, 0xf9e2af, 0xf38ba8, 0x89b4fa, 0xa6e3a1, 0xf38ba8, 0xf38ba8]),
            ),
            _ => {
                if system_prefers_dark() {
                    Self::named("dark")
                } else {
                    Self::named("light")
                }
            }
        }
    }

    pub fn tok(&self, t: Tok) -> (Rgb, bool, bool) {
        let s = &self.syntax;
        match t {
            Tok::Comment => (s.comment, false, true),
            Tok::Keyword => (s.keyword, false, false),
            Tok::Type => (s.ty, false, false),
            Tok::String => (s.string, false, false),
            Tok::Number => (s.number, false, false),
            Tok::Operator => (s.operator, false, false),
            Tok::Preproc => (s.preproc, false, false),
            Tok::Function => (s.function, false, false),
            Tok::Variable => (s.variable, false, false),
            Tok::Constant => (s.constant, false, false),
            Tok::Tag => (s.tag, false, false),
            Tok::Attribute => (s.attribute, false, false),
            Tok::Heading => (s.heading, true, false),
            Tok::Strong => (self.fg, true, false),
            Tok::Emphasis => (self.fg, false, true),
            Tok::Code => (s.string, false, false),
            Tok::Link => (s.link, false, false),
            Tok::Quote => (s.comment, false, true),
            Tok::List => (s.keyword, false, false),
            Tok::Added => (s.added, false, false),
            Tok::Deleted => (s.deleted, false, false),
            Tok::Error => (s.error, false, false),
        }
    }
}

fn read_dword(key: *const u16, value: *const u16) -> Option<u32> {
    let mut data = 0u32;
    let mut len = size_of::<u32>() as u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, key, value, RRF_RT_REG_DWORD, null_mut(), &mut data as *mut u32 as _, &mut len) };
    (r == 0).then_some(data)
}

pub fn system_prefers_dark() -> bool {
    read_dword(w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"), w!("AppsUseLightTheme")) == Some(0)
}

/// The accent colour from Settings: a lighter shade on dark backgrounds, a
/// darker one on light, as Windows itself does.
pub fn system_accent(dark: bool) -> Rgb {
    let mut palette = [0u8; 32];
    let mut len = palette.len() as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent"),
            w!("AccentPalette"),
            RRF_RT_REG_BINARY,
            null_mut(),
            palette.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if r == 0 && len == 32 {
        let i = if dark { 1 } else { 4 };
        let p = &palette[i * 4..i * 4 + 3];
        return Rgb(p[0], p[1], p[2]);
    }
    if dark {
        Rgb::hex(0x4cc2ff)
    } else {
        Rgb::hex(0x005fb8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(c: Rgb) -> f32 {
        (0.299 * c.0 as f32 + 0.587 * c.1 as f32 + 0.114 * c.2 as f32) / 255.0
    }

    #[test]
    fn every_theme_has_readable_text() {
        for (name, _) in THEMES {
            let t = Theme::named(name);
            assert!((luminance(t.fg) - luminance(t.bg)).abs() > 0.45, "{name}");
            assert_eq!(t.dark, luminance(t.bg) < 0.5, "{name}");
        }
    }

    #[test]
    fn colour_encodings() {
        let c = Rgb::hex(0x112233);
        assert_eq!(c.colorref(), 0x332211);
        assert_eq!(c.argb(0xff), 0xff112233);
        assert_eq!(c.argb_abgr(0x80), 0x80332211);
    }
}
