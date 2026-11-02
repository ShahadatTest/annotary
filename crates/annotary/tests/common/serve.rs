//! `ServeGuard`: a spawned `annotary serve` that never outlives its test (CMD-28).
//!
//! `std::process::Child` is not killed on drop, so a test that panics between spawning `serve`
//! and stopping it leaves the server running (holding a deleted temp store and a port) long
//! after the test binary exits. Every test that spawns a real `serve` wraps the child in a
//! `ServeGuard` at once. On drop it sends SIGTERM, waits up to [`GRACE`], then SIGKILLs, and
//! always reaps the child. Stopping the server on purpose (SIGTERM, SIGKILL) goes through the
//! guard as well.

use std::io::Read;
use std::process::{Child, ChildStderr, ExitStatus, Output};
use std::time::{Duration, Instant};

/// How long a dropped guard waits after SIGTERM before it sends SIGKILL.
pub const GRACE: Duration = Duration::from_secs(2);

/// Owns a spawned server process; stops and reaps it on drop.
pub struct ServeGuard(Option<Child>);

impl ServeGuard {
    pub fn new(child: Child) -> Self {
        Self(Some(child))
    }

    fn child(&mut self) -> &mut Child {
        self.0.as_mut().expect("the guard still owns its child")
    }

    pub fn id(&self) -> u32 {
        self.0.as_ref().expect("the guard still owns its child").id()
    }

    /// The child's piped stderr, for a test that reads it while the server runs.
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child().stderr.take()
    }

    /// Whether the process is still running (not yet exited).
    pub fn running(&mut self) -> bool {
        matches!(self.child().try_wait(), Ok(None))
    }

    /// Send SIGTERM (a clean shutdown) while the child runs; does not wait. A no-op once it has
    /// exited, so a reaped pid that the OS may have reused is never signalled.
    pub fn sigterm(&mut self) {
        if self.running() {
            // SAFETY: a signal to our own child, which `try_wait` just saw running (not reaped).
            unsafe {
                libc::kill(self.id() as libc::pid_t, libc::SIGTERM);
            }
        }
    }

    /// SIGKILL and reap: the hard crash a test may want.
    pub fn kill(&mut self) {
        let c = self.child();
        let _ = c.kill();
        let _ = c.wait();
    }

    /// Wait for the process to exit, at most `max`; panics if it does not (the drop then stops it).
    pub fn wait_timeout(&mut self, max: Duration) -> ExitStatus {
        let deadline = Instant::now() + max;
        loop {
            if let Some(st) = self.child().try_wait().expect("try_wait") {
                return st;
            }
            assert!(Instant::now() < deadline, "serve (pid {}) did not exit within {max:?}", self.id());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// SIGTERM, then wait up to `max` for the exit and collect what the piped stdout and stderr
    /// held. The pipes are drained while the server stops, so it never blocks on a full one.
    pub fn stop_with_output(mut self, max: Duration) -> Output {
        let out = drain(self.child().stdout.take());
        let err = drain(self.child().stderr.take());
        self.sigterm();
        let status = self.wait_timeout(max);
        Output { status, stdout: out.join().unwrap(), stderr: err.join().unwrap() }
    }
}

/// Read a pipe to its end on its own thread.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    })
}

impl Drop for ServeGuard {
    fn drop(&mut self) {
        let Some(mut c) = self.0.take() else { return };
        if let Ok(None) = c.try_wait() {
            // SAFETY: a signal to our own, not yet reaped, child.
            unsafe {
                libc::kill(c.id() as libc::pid_t, libc::SIGTERM);
            }
            let deadline = Instant::now() + GRACE;
            while matches!(c.try_wait(), Ok(None)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            if matches!(c.try_wait(), Ok(None)) {
                let _ = c.kill();
            }
        }
        let _ = c.wait();
    }
}
