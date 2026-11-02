//! Exit codes (the only place they are defined) and the CLI error type.
//!
//! `0 ok · 1 usage · 2 not found/ambiguous · 3 no credential · 4 missing right ·
//! 5 conflict/refused · 6 server unreachable · 7 wait timeout`

/// Process exit codes. Agents branch on these; never invent new ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Exit {
    Ok = 0,
    /// Bad arguments, unknown field, malformed input, or an unexpected/internal failure.
    Usage = 1,
    /// Document/Thread/Member not found, or an ambiguous Name.
    NotFound = 2,
    /// No credential configured, or the server rejected it (401).
    NoCredential = 3,
    /// The Member lacks a Right / owner-only (403).
    MissingRight = 4,
    /// Conflict or refused: head guard, duplicate handle, LOCK held, `--strict`, caps (409).
    Refused = 5,
    /// Server not reachable.
    Unreachable = 6,
    /// `wait` timed out.
    WaitTimeout = 7,
}

impl Exit {
    pub fn code(self) -> i32 {
        self as i32
    }

    /// One-line legend, as printed in `--help`.
    pub const LEGEND: &'static str = "0 ok · 1 usage · 2 not found/ambiguous · 3 no credential · 4 missing right · 5 conflict/refused · 6 server unreachable · 7 wait timed out";
}

/// A CLI failure: printed as `error: MESSAGE — HINT` (+ detail lines) on stderr, or as
/// `{"error","message","hint"}` under `--json`; the process exits with `exit`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliError {
    pub exit: Exit,
    /// Machine-readable kind (`no_credential`, `unreachable`, `forbidden`, …) — the JSON `error`.
    pub kind: String,
    pub message: String,
    pub hint: Option<String>,
    /// Extra lines printed after the error line (text mode); folded into `hint` under `--json`.
    pub detail: Vec<String>,
    // ---- ticket 08 ----
    /// Text mode prints `message` verbatim — no `error:` prefix, no hint (`wait`'s
    /// `wait: nothing after 30m (seq 812)`); `--json` is unchanged.
    pub bare: bool,
    /// Extra fields merged into the `--json` object (`seq` on a wait timeout). Boxed to keep
    /// `Result<_, CliError>` small.
    pub extra: Option<Box<serde_json::Map<String, serde_json::Value>>>,
    // ---- end ticket 08 ----
}

impl CliError {
    pub fn new(exit: Exit, kind: impl Into<String>, message: impl Into<String>) -> CliError {
        CliError {
            exit,
            kind: kind.into(),
            message: message.into(),
            hint: None,
            detail: Vec::new(),
            bare: false,
            extra: None,
        }
    }

    pub fn hint(mut self, hint: impl Into<String>) -> CliError {
        let h = hint.into();
        self.hint = if h.is_empty() { None } else { Some(h) };
        self
    }

    pub fn detail(mut self, line: impl Into<String>) -> CliError {
        self.detail.push(line.into());
        self
    }

    pub fn usage(message: impl Into<String>) -> CliError {
        CliError::new(Exit::Usage, "usage", message)
    }

    pub fn internal(message: impl Into<String>) -> CliError {
        CliError::new(Exit::Usage, "internal", message)
    }

    pub fn not_found(message: impl Into<String>) -> CliError {
        CliError::new(Exit::NotFound, "not_found", message)
    }

    pub fn no_credential(message: impl Into<String>) -> CliError {
        CliError::new(Exit::NoCredential, "no_credential", message)
    }

    pub fn missing_right(message: impl Into<String>) -> CliError {
        CliError::new(Exit::MissingRight, "forbidden", message)
    }

    pub fn refused(message: impl Into<String>) -> CliError {
        CliError::new(Exit::Refused, "refused", message)
    }

    pub fn unreachable(url: &str) -> CliError {
        CliError::new(Exit::Unreachable, "unreachable", format!("server at {url} not reachable"))
            .hint("run `annotary serve` or `annotary daemon install`")
    }

    // ---- ticket 08 ----
    /// `wait` gave up: `wait: nothing after DUR (seq N)` on stderr, exit 7; `--json`
    /// `{error: "wait_timeout", message, seq}`.
    pub fn wait_timeout(duration: &str, seq: Option<i64>) -> CliError {
        let cursor = seq.map(|s| format!(" (seq {s})")).unwrap_or_default();
        let mut e = CliError::new(Exit::WaitTimeout, "wait_timeout", format!("wait: nothing after {duration}{cursor}"));
        e.bare = true;
        if let Some(s) = seq {
            let mut extra = serde_json::Map::new();
            extra.insert("seq".into(), s.into());
            e.extra = Some(Box::new(extra));
        }
        e
    }
    // ---- end ticket 08 ----

    /// The single-line text form (without the detail lines).
    pub fn text_line(&self) -> String {
        if self.bare {
            return self.message.clone();
        }
        match &self.hint {
            Some(h) => format!("error: {} — {}", self.message, h),
            None => format!("error: {}", self.message),
        }
    }

    /// The `{error, message, hint}` object (+ `extra` fields).
    pub fn json(&self) -> serde_json::Value {
        let mut hint = self.hint.clone().unwrap_or_default();
        for d in &self.detail {
            if !hint.is_empty() {
                hint.push_str("; ");
            }
            hint.push_str(d);
        }
        let mut obj = serde_json::Map::new();
        obj.insert("error".into(), self.kind.clone().into());
        obj.insert("message".into(), self.message.clone().into());
        if !hint.is_empty() {
            obj.insert("hint".into(), hint.into());
        }
        if let Some(extra) = &self.extra {
            for (k, v) in extra.iter() {
                obj.insert(k.clone(), v.clone());
            }
        }
        serde_json::Value::Object(obj)
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text_line())
    }
}

impl std::error::Error for CliError {}

pub type CliResult<T> = Result<T, CliError>;

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::internal(e.to_string())
    }
}

impl From<annotary_store::StoreError> for CliError {
    fn from(e: annotary_store::StoreError) -> Self {
        use annotary_store::StoreError as S;
        match e {
            S::Locked { path, pid } => CliError::refused(format!(
                "store {} is locked by another annotary serve{}",
                crate::paths::display(&path),
                pid.map(|p| format!(" (pid {p})")).unwrap_or_default()
            ))
            .hint("stop it first, or use --data-dir / ANNOTARY_HOME for a separate store"),
            S::SchemaTooNew { .. } => CliError::refused(e.to_string()).hint("upgrade annotary"),
            S::HandleTaken(_) => CliError::refused(e.to_string()),
            S::InvalidHandle { .. } => CliError::usage(e.to_string()),
            S::MemberNotFound | S::TokenNotFound => CliError::not_found(e.to_string()),
            other => CliError::internal(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_the_spec_table() {
        assert_eq!(Exit::Ok.code(), 0);
        assert_eq!(Exit::Usage.code(), 1);
        assert_eq!(Exit::NotFound.code(), 2);
        assert_eq!(Exit::NoCredential.code(), 3);
        assert_eq!(Exit::MissingRight.code(), 4);
        assert_eq!(Exit::Refused.code(), 5);
        assert_eq!(Exit::Unreachable.code(), 6);
        assert_eq!(Exit::WaitTimeout.code(), 7);
    }

    #[test]
    fn text_and_json_forms() {
        let e = CliError::no_credential("no credential").hint("set ANNOTARY_TOKEN").detail("profiles: a, b");
        assert_eq!(e.text_line(), "error: no credential — set ANNOTARY_TOKEN");
        assert_eq!(
            e.json(),
            serde_json::json!({"error":"no_credential","message":"no credential","hint":"set ANNOTARY_TOKEN; profiles: a, b"})
        );
        let u = CliError::unreachable("http://127.0.0.1:7377");
        assert_eq!(
            u.text_line(),
            "error: server at http://127.0.0.1:7377 not reachable — run `annotary serve` or `annotary daemon install`"
        );
    }

    // spec: S08-08
    #[test]
    fn wait_timeout_is_bare_text_and_carries_seq_in_json() {
        let e = CliError::wait_timeout("30m", Some(812));
        assert_eq!(e.exit, Exit::WaitTimeout);
        assert_eq!(e.text_line(), "wait: nothing after 30m (seq 812)");
        assert_eq!(
            e.json(),
            serde_json::json!({"error":"wait_timeout","message":"wait: nothing after 30m (seq 812)","seq":812})
        );
        assert_eq!(CliError::wait_timeout("10s", None).text_line(), "wait: nothing after 10s");
    }
}
