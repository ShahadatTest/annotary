//! HTML escaping and the one placeholder shape shared by the Markdown writer and the raw-HTML
//! sanitizer.

/// Escape text content: `& < > "`.
pub fn escape_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

/// Escape an attribute value (double-quoted): `& < > " '`.
pub fn escape_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
}

/// Write ` name="value"` (escaped).
pub fn push_attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    escape_attr(value, out);
    out.push('"');
}

/// The marked placeholder for a `missing` Asset (referenced by the Markdown, not found on push):
/// `<span class="asset-missing" data-ref="REF">missing asset: REF</span>`, the alt text (when
/// any) in parentheses. Inline (it replaces an image / media reference wherever it was).
pub fn push_missing_placeholder(out: &mut String, reference: &str, alt: &str) {
    out.push_str("<span class=\"asset-missing\"");
    push_attr(out, "data-ref", reference);
    out.push_str(">missing asset: ");
    escape_text(reference, out);
    let alt = alt.trim();
    if !alt.is_empty() {
        out.push_str(" (");
        escape_text(alt, out);
        out.push(')');
    }
    out.push_str("</span>");
}

/// The scheme of a URL as written (lower-cased), when it has one before any `/`, `?`, `#`.
pub fn scheme_of(url: &str) -> Option<String> {
    let t = url.trim();
    let colon = t.find(':')?;
    let scheme = &t[..colon];
    if scheme.is_empty() || t[..colon].contains(['/', '?', '#']) {
        return None;
    }
    if !scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    Some(scheme.to_ascii_lowercase())
}

/// Whether a URL written in the source may be emitted at all. `javascript:` / `vbscript:` /
/// `blob:` never; `data:` only as the `src` of media (`data:image/…`, `data:video/…`,
/// `data:audio/…`); everything else (http(s), mailto, tel, custom schemes, relative) yes.
pub fn url_allowed(url: &str, media_src: bool) -> bool {
    match scheme_of(url).as_deref() {
        Some("javascript") | Some("vbscript") | Some("blob") => false,
        Some("data") => {
            let l = url.trim().to_ascii_lowercase();
            media_src && (l.starts_with("data:image/") || l.starts_with("data:video/") || l.starts_with("data:audio/"))
        }
        _ => true,
    }
}
