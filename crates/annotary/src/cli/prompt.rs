//! Interactive prompts shared by first-run setup (`serve`) and `login`: a line with a default,
//! a password (no echo on a TTY), a yes/no question.
//!
//! Scriptable by design: when stdin is not a terminal each prompt reads one line from stdin
//! (EOF/blank = default; the effective answer is echoed after the label so transcripts read
//! the same); an env override answers a prompt without reading anything. A non-TTY stdin that
//! is an idle pipe (inherited from a supervisor) counts as EOF after [`STDIN_GRACE`], so nothing
//! ever hangs waiting for a human who is not there.

use std::io::{BufRead, IsTerminal, Write};

/// How long a non-TTY stdin gets to deliver its first byte before we assume nobody is there.
pub const STDIN_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// Where prompts are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sink {
    /// First-run setup: the transcript is the result (stdout).
    Stdout,
    /// Verbs with a result line of their own (`login`): prompts stay off stdout.
    Stderr,
}

pub struct Prompter {
    pub stdin_tty: bool,
    pub stdin_usable: bool,
    sink: Sink,
    indent: &'static str,
}

impl Prompter {
    /// `indent` prefixes every prompt (`"  "` in the setup transcript, `""` for `login`).
    pub fn new(sink: Sink, indent: &'static str) -> Prompter {
        let stdin_tty = std::io::stdin().is_terminal();
        let stdin_usable = stdin_tty || stdin_readable_within(STDIN_GRACE);
        Prompter { stdin_tty, stdin_usable, sink, indent }
    }

    /// True when prompts will take their defaults because nobody can answer them.
    pub fn stdin_idle(&self) -> bool {
        !self.stdin_usable
    }

    fn write(&self, text: &str) {
        match self.sink {
            Sink::Stdout => {
                let mut so = std::io::stdout().lock();
                let _ = so.write_all(text.as_bytes());
                let _ = so.flush();
            }
            Sink::Stderr => {
                let mut se = std::io::stderr().lock();
                let _ = se.write_all(text.as_bytes());
                let _ = se.flush();
            }
        }
    }

    /// `label [default]: ` → the answer (env override → stdin line → default).
    pub fn ask(&self, label: &str, default: &str, env_key: Option<&str>) -> String {
        let shown_default = if default.is_empty() { String::new() } else { format!(" [{default}]") };
        self.write(&format!("{}{label}{shown_default}: ", self.indent));
        if let Some(v) = env_key.and_then(|k| std::env::var(k).ok()) {
            let v = v.trim().to_string();
            let answer = if v.is_empty() { default.to_string() } else { v };
            self.write(&format!("{answer}\n"));
            return answer;
        }
        let typed = if self.stdin_usable { read_line().trim().to_string() } else { String::new() };
        let answer = if typed.is_empty() { default.to_string() } else { typed };
        if !self.stdin_tty {
            self.write(&format!("{answer}\n"));
        }
        answer
    }

    /// `label: ` → the password, never echoed (`********` stands in on a non-TTY transcript);
    /// `None` when blank.
    pub fn ask_password(&self, label: &str, env_key: Option<&str>) -> Option<String> {
        self.write(&format!("{}{label}: ", self.indent));
        let pw = if let Some(v) = env_key.and_then(|k| std::env::var(k).ok()) {
            v
        } else if self.stdin_tty {
            // rpassword prints the newline itself; nothing is echoed.
            let p = rpassword::read_password().unwrap_or_default();
            return if p.is_empty() { None } else { Some(p) };
        } else if self.stdin_usable {
            read_line()
        } else {
            String::new()
        };
        let pw = pw.trim_end_matches(['\r', '\n']).to_string();
        self.write(&format!("{}\n", if pw.is_empty() { "" } else { "********" }));
        if pw.is_empty() { None } else { Some(pw) }
    }

    /// `label [y/N]: ` → true for `y`/`yes` (case-insensitive); blank = `default`.
    pub fn confirm(&self, label: &str, default: bool) -> bool {
        let yn = if default { "[Y/n]" } else { "[y/N]" };
        self.write(&format!("{}{label} {yn} ", self.indent));
        let typed = if self.stdin_usable { read_line().trim().to_ascii_lowercase() } else { String::new() };
        let answer = match typed.as_str() {
            "" => default,
            "y" | "yes" => true,
            _ => false,
        };
        if !self.stdin_tty {
            self.write(&format!("{}\n", if answer { "y" } else { "n" }));
        }
        answer
    }
}

/// One line from stdin (empty at EOF).
pub fn read_line() -> String {
    let mut s = String::new();
    let _ = std::io::stdin().lock().read_line(&mut s);
    s
}

/// Whether a non-TTY stdin will deliver something (data or EOF) within `timeout`. Only a pipe
/// (FIFO) can be idle; regular files, `/dev/null`, sockets etc. answer a read at once.
pub fn stdin_readable_within(timeout: std::time::Duration) -> bool {
    // SAFETY: fstat on fd 0 into a zeroed stat buffer.
    let is_fifo = unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        libc::fstat(0, &mut st) == 0 && (st.st_mode & libc::S_IFMT) == libc::S_IFIFO
    };
    if !is_fifo {
        return true;
    }
    let mut fds = [libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 }];
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: a valid pollfd array of length 1 on fd 0.
    let rc = unsafe { libc::poll(fds.as_mut_ptr(), 1, ms) };
    if rc < 0 {
        // poll failed (e.g. stdin closed): treat as EOF — reads will return 0 immediately.
        return true;
    }
    rc > 0 && (fds[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR)) != 0
}
