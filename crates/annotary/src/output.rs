//! Output conventions shared by every verb.
//!
//! - **non-TTY** (agents): one record per line, tab-separated, no header, no colour.
//! - **TTY** (humans): the same fields in the same order, aligned columns, a dim header,
//!   colour where it helps. `ANNOTARY_TTY=1|0` forces the mode (tests use it; no pty needed).
//! - **`--json [FIELDS]`**: bare = the verb's compact default set; `a,b` narrows; `?` lists every
//!   field. Minified, one object (or one array) on stdout, empty/null/false fields omitted.
//! - Notes/progress/hints go to stderr (`-q` silences them); results go to stdout; errors are one
//!   line on stderr (`error: MSG — HINT`) or `{error,message,hint}` under `--json`, also on stderr.

use std::io::{IsTerminal, Write};

use serde_json::{Map, Value};

use crate::exit::{CliError, CliResult};

/// How `--json` was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsonMode {
    /// `--json` bare: the verb's default field set.
    Default,
    /// `--json a,b`: exactly these fields.
    Fields(Vec<String>),
    /// `--json ?`: print the field list and exit 0.
    ListFields,
}

impl JsonMode {
    /// Parse the raw `--json` value (`""` = bare).
    pub fn parse(raw: &str) -> JsonMode {
        let raw = raw.trim();
        if raw.is_empty() {
            JsonMode::Default
        } else if raw == "?" {
            JsonMode::ListFields
        } else {
            JsonMode::Fields(raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        }
    }
}

/// The fields a verb can emit under `--json`, and its compact default set.
#[derive(Clone, Copy, Debug)]
pub struct FieldSpec {
    pub all: &'static [&'static str],
    pub default: &'static [&'static str],
}

impl FieldSpec {
    /// Resolve the requested fields, or a usage error naming the valid ones.
    pub fn select(&self, mode: &JsonMode) -> CliResult<Vec<&'static str>> {
        match mode {
            JsonMode::Default => Ok(self.default.to_vec()),
            JsonMode::ListFields => Ok(self.all.to_vec()),
            JsonMode::Fields(want) => {
                let mut out = Vec::new();
                for w in want {
                    match self.all.iter().find(|f| **f == w) {
                        Some(f) => out.push(*f),
                        None => {
                            return Err(CliError::usage(format!("unknown --json field '{w}'"))
                                .hint(format!("fields: {}", self.all.join(" "))));
                        }
                    }
                }
                Ok(out)
            }
        }
    }
}

/// Column alignment for TTY tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// Output settings resolved once per invocation.
#[derive(Clone, Debug)]
pub struct Out {
    pub tty: bool,
    pub color: bool,
    pub quiet: bool,
    pub json: Option<JsonMode>,
}

impl Out {
    /// Resolve from flags + environment (`ANNOTARY_TTY`, `NO_COLOR`) + whether stdout is a terminal.
    pub fn resolve(json: Option<&str>, quiet: bool, no_color: bool) -> Out {
        let forced = std::env::var("ANNOTARY_TTY").ok();
        let tty = match forced.as_deref().map(str::trim) {
            Some("1") | Some("true") | Some("yes") => true,
            Some("0") | Some("false") | Some("no") => false,
            _ => std::io::stdout().is_terminal(),
        };
        let no_color_env = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        let json = json.map(JsonMode::parse);
        Out { tty, color: tty && !no_color && !no_color_env && json.is_none(), quiet, json }
    }

    /// Plain settings for tests and non-CLI callers.
    pub fn plain() -> Out {
        Out { tty: false, color: false, quiet: false, json: None }
    }

    pub fn is_json(&self) -> bool {
        self.json.is_some()
    }

    /// Dim text (header rows) when colour is on.
    pub fn dim(&self, s: &str) -> String {
        if self.color { format!("\x1b[2m{s}\x1b[0m") } else { s.to_string() }
    }

    /// A note/hint/progress line on stderr — silenced by `-q`.
    pub fn note(&self, line: &str) {
        if !self.quiet {
            let _ = writeln!(std::io::stderr(), "{line}");
        }
    }

    /// Print a result line on stdout.
    pub fn line(&self, line: &str) {
        let mut so = std::io::stdout().lock();
        let _ = writeln!(so, "{line}");
    }

    /// Print an error on stderr in the mode's format.
    pub fn error(&self, err: &CliError) {
        let mut se = std::io::stderr().lock();
        if self.is_json() {
            let _ = writeln!(se, "{}", err.json());
        } else {
            let _ = writeln!(se, "{}", err.text_line());
            for d in &err.detail {
                let _ = writeln!(se, "{d}");
            }
        }
    }

    /// Render a table: TTY = aligned columns + dim header; non-TTY = tab-separated, no header.
    /// `rows` are already-formatted cells; `align` may be shorter than the column count (Left).
    pub fn table(&self, header: &[&str], rows: &[Vec<String>], align: &[Align]) -> String {
        let mut out = String::new();
        if !self.tty {
            for r in rows {
                out.push_str(&r.join("\t"));
                out.push('\n');
            }
            return out;
        }
        let ncols = header.len().max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
        let mut widths = vec![0usize; ncols];
        for (i, h) in header.iter().enumerate() {
            widths[i] = widths[i].max(h.chars().count());
        }
        for r in rows {
            for (i, c) in r.iter().enumerate() {
                widths[i] = widths[i].max(c.chars().count());
            }
        }
        let fmt_row = |cells: &[String]| -> String {
            let mut line = String::new();
            for (i, width) in widths.iter().enumerate() {
                let cell = cells.get(i).map(String::as_str).unwrap_or("");
                let a = align.get(i).copied().unwrap_or(Align::Left);
                let last = i + 1 == ncols;
                let pad = width.saturating_sub(cell.chars().count());
                match a {
                    Align::Right => {
                        line.push_str(&" ".repeat(pad));
                        line.push_str(cell);
                    }
                    Align::Left => {
                        line.push_str(cell);
                        if !last {
                            line.push_str(&" ".repeat(pad));
                        }
                    }
                }
                if !last {
                    line.push_str("  ");
                }
            }
            line.trim_end().to_string()
        };
        let head: Vec<String> = header.iter().map(|h| h.to_string()).collect();
        out.push_str(&self.dim(&fmt_row(&head)));
        out.push('\n');
        for r in rows {
            out.push_str(&fmt_row(r));
            out.push('\n');
        }
        out
    }

    /// Emit records under `--json` (`?` prints the field list instead). `single` = one object
    /// rather than an array. Values are pruned of empty/null/false fields.
    pub fn json_records(&self, spec: &FieldSpec, records: Vec<Map<String, Value>>, single: bool) -> CliResult<()> {
        let mode = self.json.clone().unwrap_or(JsonMode::Default);
        if mode == JsonMode::ListFields {
            self.line(&spec.all.join(" "));
            return Ok(());
        }
        let fields = spec.select(&mode)?;
        let mut items: Vec<Value> = Vec::with_capacity(records.len());
        for rec in records {
            let mut obj = Map::new();
            for f in &fields {
                if let Some(v) = rec.get(*f)
                    && !is_empty(v)
                {
                    obj.insert((*f).to_string(), prune(v.clone()));
                }
            }
            items.push(Value::Object(obj));
        }
        let value =
            if single { items.into_iter().next().unwrap_or(Value::Object(Map::new())) } else { Value::Array(items) };
        self.line(&value.to_string());
        Ok(())
    }
}

/// True for null, "", [], {}, false — the values `--json` omits.
pub fn is_empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Bool(b) => !*b,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        Value::Number(_) => false,
    }
}

/// Recursively drop empty fields from objects (arrays keep their elements).
pub fn prune(v: Value) -> Value {
    match v {
        Value::Object(o) => {
            Value::Object(o.into_iter().filter(|(_, v)| !is_empty(v)).map(|(k, v)| (k, prune(v))).collect())
        }
        Value::Array(a) => Value::Array(a.into_iter().map(prune).collect()),
        other => other,
    }
}

/// Relative age for text output: `2m`, `3h`, `4d`, else the (local) date. Input is a stored
/// timestamp.
pub fn relative_age(ts: &str, now: jiff::Timestamp) -> String {
    let Some(t) = annotary_store::time::parse(ts) else {
        return ts.to_string();
    };
    let secs = (now.as_second() - t.as_second()).max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else if secs < 86_400 * 30 {
        format!("{}d", secs / 86_400)
    } else {
        t.to_zoned(jiff::tz::TimeZone::system()).strftime("%Y-%m-%d").to_string()
    }
}

/// Trim a stored millisecond timestamp to seconds for human/JSON display (`…:02Z`).
pub fn iso_seconds(ts: &str) -> String {
    annotary_store::time::to_seconds(ts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<Vec<String>> {
        vec![
            vec!["@rowan".into(), "Rowan Vale".into(), "owner".into(), "*:read,comment,action,write".into()],
            vec!["@linter".into(), "linter".into(), "member".into(), "*:read".into()],
        ]
    }

    // spec: S01-12
    #[test]
    fn non_tty_is_tsv_without_header() {
        let out = Out::plain();
        let t = out.table(&["HANDLE", "NAME", "ROLE", "GRANTS"], &rows(), &[]);
        assert_eq!(t, "@rowan\tRowan Vale\towner\t*:read,comment,action,write\n@linter\tlinter\tmember\t*:read\n");
    }

    #[test]
    fn tty_is_aligned_with_dim_header_and_same_fields() {
        let out = Out { tty: true, color: true, quiet: false, json: None };
        let t = out.table(&["HANDLE", "NAME", "ROLE", "GRANTS"], &rows(), &[]);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines[0], "\x1b[2mHANDLE   NAME        ROLE    GRANTS\x1b[0m");
        assert_eq!(lines[1], "@rowan   Rowan Vale  owner   *:read,comment,action,write");
        assert_eq!(lines[2], "@linter  linter      member  *:read");
        let plain = Out { tty: true, color: false, quiet: false, json: None };
        let t = plain.table(&["HANDLE", "NAME", "ROLE", "GRANTS"], &rows(), &[]);
        assert!(t.starts_with("HANDLE   NAME        ROLE    GRANTS\n"), "no escape codes without colour: {t:?}");
    }

    #[test]
    fn right_alignment() {
        let out = Out { tty: true, color: false, quiet: false, json: None };
        let t = out.table(
            &["N", "OPEN"],
            &[vec!["a".into(), "4".into()], vec!["bb".into(), "12".into()]],
            &[Align::Left, Align::Right],
        );
        assert_eq!(t, "N   OPEN\na      4\nbb    12\n");
    }

    // spec: S01-17
    #[test]
    fn json_modes() {
        assert_eq!(JsonMode::parse(""), JsonMode::Default);
        assert_eq!(JsonMode::parse("?"), JsonMode::ListFields);
        assert_eq!(JsonMode::parse("a, b"), JsonMode::Fields(vec!["a".into(), "b".into()]));
        let spec = FieldSpec { all: &["a", "b", "c"], default: &["a", "b"] };
        assert_eq!(spec.select(&JsonMode::Default).unwrap(), vec!["a", "b"]);
        assert_eq!(spec.select(&JsonMode::Fields(vec!["c".into()])).unwrap(), vec!["c"]);
        let err = spec.select(&JsonMode::Fields(vec!["zz".into()])).unwrap_err();
        assert_eq!(err.exit, crate::exit::Exit::Usage);
        assert!(err.hint.unwrap().contains("a b c"));
    }

    // spec: S01-17
    #[test]
    fn prune_drops_empty_null_false() {
        let v =
            serde_json::json!({"a":1,"b":null,"c":"","d":[],"e":{},"f":false,"g":{"h":null,"i":2},"j":[{"k":null}]});
        assert_eq!(prune(v), serde_json::json!({"a":1,"g":{"i":2},"j":[{}]}));
    }

    #[test]
    fn ages_and_seconds() {
        let now = annotary_store::time::parse("2026-08-18T10:00:00.000Z").unwrap();
        assert_eq!(relative_age("2026-08-18T09:58:30.000Z", now), "1m");
        assert_eq!(relative_age("2026-08-16T09:58:30.000Z", now), "2d");
        assert_eq!(relative_age("2026-01-01T12:00:00.000Z", now), "2026-01-01", "old = local date");
        assert_eq!(iso_seconds("2026-08-18T09:41:02.123Z"), "2026-08-18T09:41:02Z");
    }
}
