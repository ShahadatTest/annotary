//! The strict reader. Accepts one JSON object in the Sidecar shape; rejects YAML, JSONL and
//! minified JSON with a typed [`ParseError`] so push can say exactly what is wrong. Unknown keys
//! are ignored (hand edits outside the read fields are not an error).

use alloc::string::{String, ToString};

use serde_json::Value;

use crate::model::Sidecar;

/// Why a text is not a Sidecar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// Nothing but whitespace.
    Empty,
    /// Not JSON at all. `looks_like` is `Some("YAML")` when the text has YAML's `key: value` shape.
    NotJson { looks_like: Option<&'static str>, detail: String },
    /// Several JSON values one after another (JSONL).
    MultipleValues,
    /// One JSON object squeezed onto a single line while it holds nested containers.
    Minified,
    /// Valid JSON but not an object.
    NotAnObject,
    /// `format` missing or not the supported value.
    Format { found: Option<u64> },
    /// The JSON is in shape but a field has the wrong type or a required field is missing.
    Invalid(String),
    /// Ticket 17: the two sides of a git conflict name different Document ids or Vaults
    /// (`what` = `"Document ids"` | `"Vaults"`) — one side has to be kept by hand.
    Conflict { what: &'static str },
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ParseError::Empty => f.write_str("file is empty"),
            ParseError::NotJson { looks_like: Some(kind), .. } => write!(f, "not JSON (looks like {kind})"),
            ParseError::NotJson { looks_like: None, detail } => write!(f, "not JSON ({detail})"),
            ParseError::MultipleValues => f.write_str("several JSON values (JSONL) — a Sidecar is one JSON object"),
            ParseError::Minified => f.write_str("minified JSON — a Sidecar is written in the canonical layout"),
            ParseError::NotAnObject => f.write_str("JSON is not an object"),
            ParseError::Format { found: Some(n) } => {
                write!(f, "format {n} is not supported (this annotary writes format {})", crate::FORMAT)
            }
            ParseError::Format { found: None } => f.write_str("missing `format`"),
            ParseError::Invalid(msg) => write!(f, "invalid field: {msg}"),
            ParseError::Conflict { what } => write!(f, "the two sides of the git conflict have different {what}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseError {}

/// Parse a canonical Sidecar into its typed shape.
pub fn parse(text: &str) -> Result<Sidecar, ParseError> {
    parse_object(text)?;
    // Typed parse straight from the text (not via `Value`, whose object keeps no order) so the
    // `members` table comes back in file order.
    serde_json::from_str::<Sidecar>(text.trim()).map_err(|e| ParseError::Invalid(e.to_string()))
}

/// The shared front half: reject non-JSON / JSONL / minified / non-object / wrong `format`, and
/// return the parsed object for either the typed reader or the lenient [`crate::Binding`].
pub(crate) fn parse_object(text: &str) -> Result<Value, ParseError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(ParseError::Empty);
    }
    let value: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(e) => {
            if looks_like_yaml(trimmed) {
                return Err(ParseError::NotJson { looks_like: Some("YAML"), detail: e.to_string() });
            }
            if looks_like_jsonl(trimmed) {
                return Err(ParseError::MultipleValues);
            }
            return Err(ParseError::NotJson { looks_like: None, detail: e.to_string() });
        }
    };
    let Value::Object(map) = &value else {
        return Err(ParseError::NotAnObject);
    };
    let has_container = map.values().any(|v| matches!(v, Value::Object(_) | Value::Array(_)));
    if has_container && !trimmed.contains('\n') {
        return Err(ParseError::Minified);
    }
    match map.get("format") {
        Some(Value::Number(n)) if n.as_u64() == Some(u64::from(crate::FORMAT)) => {}
        Some(Value::Number(n)) => return Err(ParseError::Format { found: n.as_u64() }),
        _ => return Err(ParseError::Format { found: None }),
    }
    Ok(value)
}

/// `key: value` on the first content line, or a `---` document marker.
fn looks_like_yaml(text: &str) -> bool {
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim_start();
    if first.starts_with("---") || first.starts_with("- ") {
        return true;
    }
    if first.starts_with('{') || first.starts_with('[') || first.starts_with('"') {
        return false;
    }
    let ok_start = first.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_');
    ok_start
        && first.split_once(':').is_some_and(|(k, rest)| {
            k.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') && (rest.starts_with(' ') || rest.is_empty())
        })
}

/// The first content line is a complete JSON value and more content follows.
fn looks_like_jsonl(text: &str) -> bool {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let Some(first) = lines.next() else { return false };
    lines.next().is_some() && serde_json::from_str::<Value>(first).is_ok()
}
