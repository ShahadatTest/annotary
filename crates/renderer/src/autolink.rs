//! GFM extended autolinks (bare `https://…`, `www.…`, `user@example.com` in plain text become
//! links), following cmark-gfm's rules: a valid domain, extension to the next whitespace or `<`,
//! trailing punctuation / unbalanced `)` / a trailing entity trimmed. Applied to text outside
//! code, links and raw HTML.

use crate::escape::{escape_attr, escape_text};

/// One literal link found in a text run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Byte range of the link text within the input.
    pub start: usize,
    pub end: usize,
    /// The `href` (`http://` prefixed for `www.`, `mailto:` for emails).
    pub href: String,
}

/// cmark-gfm's "valid host char" is "not space, not punctuation"; alphanumerics (Unicode) are
/// the useful subset of that.
fn is_hostchar(c: char) -> bool {
    c.is_alphanumeric()
}

/// cmark-gfm `check_domain`: length of the domain part at the start of `s`, or 0.
fn check_domain(s: &str, allow_short: bool) -> usize {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut np = 0usize;
    let mut uscore1 = 0usize;
    let mut uscore2 = 0usize;
    let mut i = 0usize; // index into chars
    let mut end = 0usize; // byte offset past the last accepted char
    while i < chars.len() {
        let (off, c) = chars[i];
        if c == '_' {
            uscore2 += 1;
        } else if c == '.' {
            uscore1 = uscore2;
            uscore2 = 0;
            np += 1;
        } else if !is_hostchar(c) && c != '-' {
            break;
        }
        end = off + c.len_utf8();
        i += 1;
    }
    if uscore1 > 0 || uscore2 > 0 {
        return 0;
    }
    if allow_short || np > 0 { end } else { 0 }
}

/// Extend from `from` to the next whitespace or `<`.
fn extend(s: &str, from: usize) -> usize {
    let mut end = from;
    for (off, c) in s[from..].char_indices() {
        if c.is_whitespace() || c == '<' {
            return from + off;
        }
        end = from + off + c.len_utf8();
    }
    end
}

/// cmark-gfm `autolink_delim`: trim trailing punctuation, an unbalanced `)`, a trailing entity.
fn delim(s: &str, start: usize, mut end: usize) -> usize {
    while end > start {
        let last = s[..end].chars().next_back().unwrap();
        if "?!.,:*_~'\"".contains(last) {
            end -= last.len_utf8();
        } else if last == ';' {
            // `&amp;`-style trailing entity: cut it whole.
            let mut new_end = end - 1;
            while new_end > start && s.as_bytes()[new_end - 1].is_ascii_alphabetic() {
                new_end -= 1;
            }
            if new_end > start && new_end < end - 1 && s.as_bytes()[new_end - 1] == b'&' {
                end = new_end - 1;
            } else {
                end -= 1;
            }
        } else if last == ')' {
            let opening = s[start..end].matches('(').count();
            let closing = s[start..end].matches(')').count();
            if closing <= opening {
                break;
            }
            end -= 1;
        } else {
            break;
        }
    }
    end
}

fn boundary_before(s: &str, at: usize, extra: &str) -> bool {
    match s[..at].chars().next_back() {
        None => true,
        Some(c) => c.is_whitespace() || extra.contains(c),
    }
}

/// Every literal link in `text`, left to right, non-overlapping.
pub fn find(text: &str) -> Vec<Found> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // URL: scheme://
        if bytes[i] == b':' && text[i..].starts_with("://") {
            // Rewind over ASCII letters to the scheme start.
            let mut start = i;
            while start > 0 && bytes[start - 1].is_ascii_alphabetic() {
                start -= 1;
            }
            let scheme = text[start..i].to_ascii_lowercase();
            if matches!(scheme.as_str(), "http" | "https" | "ftp") && start >= last_end(&out) {
                let dom = check_domain(&text[i + 3..], true);
                if dom > 0 {
                    let mut end = extend(text, i + 3 + dom);
                    end = delim(text, start, end);
                    if end > i + 3 {
                        out.push(Found { start, end, href: text[start..end].to_string() });
                        i = end;
                        continue;
                    }
                }
            }
            i += 1;
            continue;
        }
        // www.
        if bytes[i] == b'w' && text[i..].starts_with("www.") && i >= last_end(&out) && boundary_before(text, i, "*_~(")
        {
            let dom = check_domain(&text[i..], false);
            if dom > 0 {
                let mut end = extend(text, i + dom);
                end = delim(text, i, end);
                if end > i {
                    out.push(Found { start: i, end, href: format!("http://{}", &text[i..end]) });
                    i = end;
                    continue;
                }
            }
            i += 1;
            continue;
        }
        // email
        if bytes[i] == b'@' {
            let mut start = i;
            while start > 0 && start > last_end(&out) {
                let c = bytes[start - 1];
                if c.is_ascii_alphanumeric() || b".+-_".contains(&c) {
                    start -= 1;
                } else {
                    break;
                }
            }
            if start < i && (start == 0 || bytes[start - 1] != b'/') {
                let mut end = i + 1;
                let mut np = 0;
                while end < bytes.len() {
                    let c = bytes[end];
                    if c.is_ascii_alphanumeric() {
                    } else if c == b'@' {
                        break;
                    } else if c == b'.' && end + 1 < bytes.len() && bytes[end + 1].is_ascii_alphanumeric() {
                        np += 1;
                    } else if c != b'-' && c != b'_' {
                        break;
                    }
                    end += 1;
                }
                let ok = end >= i + 2 && np > 0 && (bytes[end - 1].is_ascii_alphabetic() || bytes[end - 1] == b'.');
                if ok && (end >= bytes.len() || bytes[end] != b'@') {
                    let end = delim(text, start, end);
                    if end > i + 1 {
                        out.push(Found { start, end, href: format!("mailto:{}", &text[start..end]) });
                        i = end;
                        continue;
                    }
                }
            }
            i += 1;
            continue;
        }
        i += 1;
    }
    out
}

fn last_end(found: &[Found]) -> usize {
    found.last().map(|f| f.end).unwrap_or(0)
}

/// Write `text` escaped, with every literal link wrapped in `<a href="…">…</a>`.
pub fn linkify(text: &str, out: &mut String) {
    let mut at = 0;
    for f in find(text) {
        escape_text(&text[at..f.start], out);
        out.push_str("<a href=\"");
        escape_attr(&f.href, out);
        out.push_str("\">");
        escape_text(&text[f.start..f.end], out);
        out.push_str("</a>");
        at = f.end;
    }
    escape_text(&text[at..], out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(s: &str) -> Vec<(String, String)> {
        find(s).into_iter().map(|f| (s[f.start..f.end].to_string(), f.href)).collect()
    }

    #[test]
    fn urls_www_and_emails_follow_gfm_rules() {
        assert_eq!(
            links("see https://example.com/a?b=1."),
            vec![("https://example.com/a?b=1".to_string(), "https://example.com/a?b=1".to_string())]
        );
        assert_eq!(links("(http://x.y/z)"), vec![("http://x.y/z".to_string(), "http://x.y/z".to_string())]);
        assert_eq!(links("http://x.y/a_(b)"), vec![("http://x.y/a_(b)".to_string(), "http://x.y/a_(b)".to_string())]);
        assert_eq!(links("http://x.y/a&amp;"), vec![("http://x.y/a".to_string(), "http://x.y/a".to_string())]);
        assert_eq!(
            links("http://localhost:7377/d/abc,"),
            vec![("http://localhost:7377/d/abc".to_string(), "http://localhost:7377/d/abc".to_string())]
        );
        assert_eq!(
            links("Www.example.com and www.a.b/c\"."),
            vec![("www.a.b/c".to_string(), "http://www.a.b/c".to_string())]
        );
        assert_eq!(links("xwww.example.com"), vec![], "needs a boundary");
        assert_eq!(links("*www.x.y*"), vec![("www.x.y".to_string(), "http://www.x.y".to_string())]);
        assert_eq!(links("www.no_dot"), vec![]);
        assert_eq!(links("www.a_b.com"), vec![], "underscore in the last two segments");
        assert_eq!(
            links("mail foo.bar+1@example.com, thanks"),
            vec![("foo.bar+1@example.com".to_string(), "mailto:foo.bar+1@example.com".to_string())]
        );
        assert_eq!(links("@rowan and @scribe"), vec![], "mentions are not emails");
        assert_eq!(links("a@b"), vec![]);
        assert_eq!(
            links("ftp://files.example.com/x <b>"),
            vec![("ftp://files.example.com/x".to_string(), "ftp://files.example.com/x".to_string())]
        );
        assert_eq!(links("nothing here"), vec![]);
        assert_eq!(
            links("Ünïcode https://exämple.com/päth."),
            vec![("https://exämple.com/päth".to_string(), "https://exämple.com/päth".to_string())]
        );
        let mut out = String::new();
        linkify("go to https://x.y/?a=1&b=2 now", &mut out);
        assert_eq!(out, "go to <a href=\"https://x.y/?a=1&amp;b=2\">https://x.y/?a=1&amp;b=2</a> now");
    }
}
