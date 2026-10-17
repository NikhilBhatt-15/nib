//! Pure text transformations: clipboard cleanup, line operations, case
//! changes and JSON formatting. Each takes and returns a String so they are
//! easy to test and to apply to a selection, a document or the clipboard.

use crate::settings::Cleanup;

/// Applies the cleanup steps chosen in settings. Line endings come out as `\n`.
pub fn cleanup(text: &str, c: &Cleanup) -> String {
    let mut s = text.replace("\r\n", "\n").replace('\r', "\n");
    if c.remove_invisible {
        s = remove_invisible(&s);
    }
    if c.straighten_quotes {
        s = straighten_quotes(&s);
    }
    if c.strip_bullets {
        s = s.lines().map(strip_bullet).collect::<Vec<_>>().join("\n");
    }
    if c.trim_trailing_spaces {
        s = trim_trailing(&s);
    }
    if c.dedent {
        s = dedent(&s);
    }
    if c.unwrap_lines {
        s = unwrap_paragraphs(&s);
    }
    if c.collapse_blank_lines {
        s = collapse_blank_lines(&s);
    }
    s.trim_matches('\n').to_string()
}

/// Drops zero-width and bidi control characters, and turns no-break and other
/// exotic spaces into plain spaces.
pub fn remove_invisible(s: &str) -> String {
    s.chars()
        .filter_map(|c| match c {
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{00AD}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' => None,
            '\u{00A0}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => Some(' '),
            '\u{2028}' | '\u{2029}' => Some('\n'),
            c => Some(c),
        })
        .collect()
}

pub fn straighten_quotes(s: &str) -> String {
    s.chars()
        .flat_map(|c| -> Box<dyn Iterator<Item = char>> {
            match c {
                '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{2032}' => Box::new(std::iter::once('\'')),
                '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' | '\u{2033}' | '\u{00AB}' | '\u{00BB}' => Box::new(std::iter::once('"')),
                '\u{2013}' | '\u{2014}' => Box::new(std::iter::once('-')),
                '\u{2026}' => Box::new("...".chars()),
                c => Box::new(std::iter::once(c)),
            }
        })
        .collect()
}

pub fn trim_trailing(s: &str) -> String {
    s.split('\n').map(|l| l.trim_end_matches([' ', '\t'])).collect::<Vec<_>>().join("\n")
}

/// Removes the indentation shared by every non-blank line.
pub fn dedent(s: &str) -> String {
    let indent = s
        .split('\n')
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start_matches([' ', '\t']).len())
        .min()
        .unwrap_or(0);
    if indent == 0 {
        return s.to_string();
    }
    s.split('\n').map(|l| if l.len() >= indent { &l[indent..] } else { l.trim_start() }).collect::<Vec<_>>().join("\n")
}

pub fn collapse_blank_lines(s: &str) -> String {
    let mut out = Vec::new();
    let mut blank = 0;
    for l in s.split('\n') {
        if l.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
            out.push("");
        } else {
            blank = 0;
            out.push(l);
        }
    }
    out.join("\n")
}

/// Joins hard-wrapped lines within each paragraph (as copied from PDFs and emails).
pub fn unwrap_paragraphs(s: &str) -> String {
    let mut out = String::new();
    for (i, para) in s.split("\n\n").enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        let mut first = true;
        for line in para.split('\n') {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            if !first {
                if out.ends_with('-') && !out.ends_with(" -") {
                    out.pop();
                } else {
                    out.push(' ');
                }
            }
            out.push_str(t);
            first = false;
        }
    }
    out
}

fn strip_bullet(l: &str) -> &str {
    let t = l.trim_start();
    for b in ["• ", "◦ ", "▪ ", "- ", "* ", "· ", "– "] {
        if let Some(rest) = t.strip_prefix(b) {
            return rest;
        }
    }
    l
}

pub fn sort_lines(s: &str, descending: bool, case_insensitive: bool) -> String {
    let mut lines: Vec<&str> = s.split('\n').collect();
    let trailing = lines.last() == Some(&"");
    if trailing {
        lines.pop();
    }
    if case_insensitive {
        lines.sort_by_key(|l| l.to_lowercase());
    } else {
        lines.sort();
    }
    if descending {
        lines.reverse();
    }
    let mut out = lines.join("\n");
    if trailing {
        out.push('\n');
    }
    out
}

pub fn unique_lines(s: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    s.split('\n').filter(|l| l.is_empty() || seen.insert(*l)).collect::<Vec<_>>().join("\n")
}

pub fn reverse_lines(s: &str) -> String {
    let mut lines: Vec<&str> = s.split('\n').collect();
    let trailing = lines.last() == Some(&"");
    if trailing {
        lines.pop();
    }
    lines.reverse();
    let mut out = lines.join("\n");
    if trailing {
        out.push('\n');
    }
    out
}

pub fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start = true;
    for c in s.chars() {
        if c.is_alphanumeric() || c == '\'' || c == '’' {
            if start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            start = false;
        } else {
            out.push(c);
            start = true;
        }
    }
    out
}

pub fn sentence_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start = true;
    for c in s.chars() {
        if start && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            start = false;
        } else {
            out.extend(c.to_lowercase());
            if matches!(c, '.' | '!' | '?' | '\n') {
                start = true;
            }
        }
    }
    out
}

pub fn json_format(s: &str, indent: usize) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(&crate::settings::strip_json_comments(s)).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    let pad = vec![b' '; indent];
    let fmt = serde_json::ser::PrettyFormatter::with_indent(&pad);
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    serde::Serialize::serialize(&v, &mut ser).map_err(|e| e.to_string())?;
    Ok(String::from_utf8(buf).unwrap_or_default())
}

pub fn json_minify(s: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(&crate::settings::strip_json_comments(s)).map_err(|e| e.to_string())?;
    serde_json::to_string(&v).map_err(|e| e.to_string())
}

/// For Enter in Markdown: the prefix to continue a list or quote on the next
/// line, or `None` to end it (when the current item is empty).
pub fn list_continuation(line: &str) -> Option<Option<String>> {
    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    let rest = &line[indent_len..];
    // Task items, bullets, quotes.
    for marker in ["- [ ] ", "- [x] ", "* [ ] ", "* [x] ", "- ", "* ", "+ ", "> "] {
        if let Some(body) = rest.strip_prefix(marker) {
            if body.trim().is_empty() {
                return Some(None);
            }
            let next = if marker.contains("[x]") { marker.replace("[x]", "[ ]") } else { marker.to_string() };
            return Some(Some(format!("{indent}{next}")));
        }
    }
    // Numbered: "12. " or "12) ".
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() && digits.len() < 9 {
        let after = &rest[digits.len()..];
        for sep in [". ", ") "] {
            if let Some(body) = after.strip_prefix(sep) {
                if body.trim().is_empty() {
                    return Some(None);
                }
                let n: u64 = digits.parse().unwrap_or(0);
                return Some(Some(format!("{indent}{}{sep}", n + 1)));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_defaults() {
        let messy = "    “Hello”\u{00A0}world\u{200B}  \r\n\r\n\r\n      it’s — done…  \r\n";
        let c = Cleanup::default();
        assert_eq!(cleanup(messy, &c), "\"Hello\" world\n\n  it's - done...");
    }

    #[test]
    fn unwrap_and_bullets() {
        assert_eq!(unwrap_paragraphs("one two\nthree four-\nfive\n\nsix\nseven"), "one two three fourfive\n\nsix seven");
        let c = Cleanup { strip_bullets: true, ..Default::default() };
        assert_eq!(cleanup("• alpha\n• beta", &c), "alpha\nbeta");
    }

    #[test]
    fn lines() {
        assert_eq!(sort_lines("b\nA\nc\n", false, true), "A\nb\nc\n");
        assert_eq!(sort_lines("b\na", true, false), "b\na");
        assert_eq!(unique_lines("a\nb\na\n\n\nb"), "a\nb\n\n");
        assert_eq!(reverse_lines("1\n2\n3\n"), "3\n2\n1\n");
        assert_eq!(dedent("    a\n      b\n\n    c"), "a\n  b\n\nc");
    }

    #[test]
    fn cases() {
        assert_eq!(title_case("hello WORLD it's"), "Hello World It's");
        assert_eq!(sentence_case("HELLO THERE. how ARE you?"), "Hello there. How are you?");
    }

    #[test]
    fn json() {
        assert_eq!(json_format("{\"a\":[1,2], // c\n}", 2).unwrap(), "{\n  \"a\": [\n    1,\n    2\n  ]\n}");
        assert_eq!(json_minify("{ \"a\" : 1 }").unwrap(), "{\"a\":1}");
        assert!(json_format("{", 2).is_err());
    }

    #[test]
    fn markdown_lists() {
        assert_eq!(list_continuation("- item"), Some(Some("- ".into())));
        assert_eq!(list_continuation("  * [x] done"), Some(Some("  * [ ] ".into())));
        assert_eq!(list_continuation("9. nine"), Some(Some("10. ".into())));
        assert_eq!(list_continuation("> quote"), Some(Some("> ".into())));
        assert_eq!(list_continuation("- "), Some(None));
        assert_eq!(list_continuation("plain"), None);
    }
}
