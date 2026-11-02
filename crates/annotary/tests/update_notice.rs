//! The update notice (`crate::update`), through the real binary against a fake Releases API on
//! loopback. A build from a checkout reports `0.0.0`, so with `ANNOTARY_UPDATE_URL` set any real
//! release is newer. `ANNOTARY_TTY=1` stands in for a terminal.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use common::serve::ServeGuard;

const TAG: &str = r#"{"tag_name":"v9.8.7"}"#;

/// What the fake Releases API does with every request.
#[derive(Clone, Copy)]
enum Answer {
    Respond(u16, &'static str),
    Never,
}

struct FakeApi {
    url: String,
    requests: Arc<AtomicUsize>,
}

impl FakeApi {
    fn start(answer: Answer) -> FakeApi {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/releases/latest", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming().flatten() {
                count.fetch_add(1, Ordering::SeqCst);
                let mut stream = stream;
                match answer {
                    Answer::Never => held.push(stream),
                    Answer::Respond(status, body) => {
                        let mut buf = [0u8; 2048];
                        let _ = stream.read(&mut buf);
                        let _ = write!(
                            stream,
                            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                    }
                }
            }
        });
        FakeApi { url, requests }
    }

    fn asked(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

fn annotary(home: &Path, url: Option<&str>) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_annotary"));
    c.env("HOME", home)
        .env("ANNOTARY_HOME", home.join("data"))
        .env("ANNOTARY_TTY", "1")
        .env("USER", "rowan")
        .env_remove("ANNOTARY_UPDATE_CHECK")
        .env_remove("ANNOTARY_UPDATE_URL")
        .env_remove("ANNOTARY_TOKEN")
        .env_remove("ANNOTARY_SERVER")
        .env_remove("ANNOTARY_PROFILE")
        .stdin(Stdio::null());
    if let Some(url) = url {
        c.env("ANNOTARY_UPDATE_URL", url);
    }
    c
}

fn version(home: &Path, url: Option<&str>) -> Output {
    annotary(home, url).arg("-V").output().unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn cache(home: &Path) -> PathBuf {
    home.join("data/update-check.json")
}

fn write_cache(home: &Path, checked_at: u64, latest: &str) {
    std::fs::create_dir_all(home.join("data")).unwrap();
    std::fs::write(cache(home), format!(r#"{{"checked_at":{checked_at},"latest":"{latest}"}}"#)).unwrap();
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

// spec: S34-01
#[test]
fn a_newer_release_is_a_note_on_stderr_and_stdout_stays_the_version() {
    let home = tempfile::tempdir().unwrap();
    let api = FakeApi::start(Answer::Respond(200, TAG));
    let out = version(home.path(), Some(&api.url));
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), format!("annotary {}\n", annotary::VERSION));
    let note = stderr(&out);
    assert!(note.contains("annotary 9.8.7 is available"), "{note}");
    assert!(note.contains(&format!("you have {}", annotary::VERSION)), "{note}");
    assert!(note.contains("docs/install.md#upgrade"), "{note}");
    assert_eq!(api.asked(), 1);
}

// spec: S34-01
#[test]
fn the_same_or_an_older_release_says_nothing() {
    let home = tempfile::tempdir().unwrap();
    let api = FakeApi::start(Answer::Respond(200, r#"{"tag_name":"v0.0.0"}"#));
    let out = version(home.path(), Some(&api.url));
    assert_eq!(api.asked(), 1);
    assert_eq!(stderr(&out), "");
}

// spec: S34-02
#[test]
fn it_asks_once_a_day_and_the_cached_answer_still_gives_the_notice() {
    let home = tempfile::tempdir().unwrap();
    let api = FakeApi::start(Answer::Respond(200, TAG));
    version(home.path(), Some(&api.url));
    let again = version(home.path(), Some(&api.url));
    assert_eq!(api.asked(), 1, "the second run asks nothing");
    assert!(stderr(&again).contains("9.8.7"), "{}", stderr(&again));

    // A day later it asks again; so it does when the clock has gone back past the last check.
    write_cache(home.path(), now() - 24 * 60 * 60, "1.0.0");
    let later = version(home.path(), Some(&api.url));
    assert_eq!(api.asked(), 2);
    assert!(stderr(&later).contains("9.8.7"), "the new answer replaces the old: {}", stderr(&later));
    write_cache(home.path(), now() + 3 * 24 * 60 * 60, "1.0.0");
    version(home.path(), Some(&api.url));
    assert_eq!(api.asked(), 3);
}

// spec: S34-03
#[test]
fn off_with_the_environment_means_no_request_no_file_and_no_notice() {
    let home = tempfile::tempdir().unwrap();
    let api = FakeApi::start(Answer::Respond(200, TAG));
    let out = annotary(home.path(), Some(&api.url)).env("ANNOTARY_UPDATE_CHECK", "off").arg("-V").output().unwrap();
    assert!(out.status.success());
    assert_eq!(stderr(&out), "");
    assert_eq!(api.asked(), 0);
    assert!(!cache(home.path()).exists());

    write_cache(home.path(), now(), "9.9.9");
    let out = annotary(home.path(), Some(&api.url)).env("ANNOTARY_UPDATE_CHECK", "OFF").arg("-V").output().unwrap();
    assert_eq!(stderr(&out), "", "an old answer is not shown either");
}

// spec: S34-03
#[test]
fn off_with_config_toml_means_no_request_no_file_and_no_notice_and_the_cli_keeps_the_setting() {
    let home = tempfile::tempdir().unwrap();
    let api = FakeApi::start(Answer::Respond(200, TAG));
    let data = home.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("config.toml"), "update_check = false\n").unwrap();
    let out = version(home.path(), Some(&api.url));
    assert_eq!(stderr(&out), "");
    assert_eq!(api.asked(), 0);
    assert!(!cache(home.path()).exists());
    write_cache(home.path(), now(), "9.9.9");
    assert_eq!(stderr(&version(home.path(), Some(&api.url))), "", "an old answer is not shown either");

    // Writing the file back, as `profile add` does, must not switch it on again.
    let out = annotary(home.path(), None)
        .args(["profile", "add", "rowan", "--server", "http://127.0.0.1:1", "--token", "ant_x"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let text = std::fs::read_to_string(data.join("config.toml")).unwrap();
    assert!(text.contains("update_check = false") && text.contains("[profiles.rowan]"), "{text}");
}

// spec: S34-04
#[test]
fn a_failed_check_is_silent_and_counts_for_the_day() {
    for answer in [
        Answer::Respond(404, r#"{"message":"Not Found"}"#),
        Answer::Respond(500, "oops"),
        Answer::Respond(200, "not json"),
        Answer::Respond(200, r#"{"name":"no tag"}"#),
        Answer::Respond(200, r#"{"tag_name":"nightly"}"#),
    ] {
        let home = tempfile::tempdir().unwrap();
        let api = FakeApi::start(answer);
        let out = version(home.path(), Some(&api.url));
        assert!(out.status.success());
        assert_eq!(stderr(&out), "");
        version(home.path(), Some(&api.url));
        assert_eq!(api.asked(), 1, "no second try the same day");
    }
}

// spec: S34-04
#[test]
fn no_server_at_all_is_silent() {
    let home = tempfile::tempdir().unwrap();
    // A port that was just free: nothing listens there.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let out = version(home.path(), Some(&format!("http://127.0.0.1:{port}/releases/latest")));
    assert!(out.status.success());
    assert_eq!(stderr(&out), "");
    assert!(cache(home.path()).exists(), "the attempt is recorded");
}

// spec: S34-04
#[test]
fn a_failed_check_keeps_the_last_answer() {
    let home = tempfile::tempdir().unwrap();
    let api = FakeApi::start(Answer::Respond(404, "{}"));
    write_cache(home.path(), now() - 2 * 24 * 60 * 60, "9.0.0");
    let out = version(home.path(), Some(&api.url));
    assert_eq!(api.asked(), 1);
    assert!(stderr(&out).contains("9.0.0"), "{}", stderr(&out));
}

// spec: S34-05
#[test]
fn with_no_terminal_it_never_asks_and_only_says_what_it_knows() {
    let home = tempfile::tempdir().unwrap();
    let api = FakeApi::start(Answer::Respond(200, TAG));
    let quiet = annotary(home.path(), Some(&api.url)).env("ANNOTARY_TTY", "0").arg("-V").output().unwrap();
    assert_eq!(stderr(&quiet), "");
    assert_eq!(api.asked(), 0);
    assert!(!cache(home.path()).exists());
    write_cache(home.path(), 1, "9.0.0");
    let told = annotary(home.path(), Some(&api.url)).env("ANNOTARY_TTY", "0").arg("-V").output().unwrap();
    assert_eq!(api.asked(), 0, "due, but no terminal");
    assert!(stderr(&told).contains("9.0.0"), "{}", stderr(&told));
}

// spec: S34-05
#[test]
fn other_verbs_neither_ask_nor_print_it() {
    let home = tempfile::tempdir().unwrap();
    let api = FakeApi::start(Answer::Respond(200, TAG));
    write_cache(home.path(), 1, "9.0.0");
    let out = annotary(home.path(), Some(&api.url)).args(["profile", "list"]).output().unwrap();
    assert!(!stderr(&out).contains("9.0.0"), "{}", stderr(&out));
    assert!(!String::from_utf8_lossy(&out.stdout).contains("9.0.0"));
    assert_eq!(api.asked(), 0);
}

// spec: S34-05
#[test]
fn a_build_with_no_release_version_never_asks() {
    if annotary::VERSION != "0.0.0" {
        return; // a stamped build compares; this is about one that has nothing to compare
    }
    let home = tempfile::tempdir().unwrap();
    let out = version(home.path(), None);
    assert_eq!(stderr(&out), "");
    assert!(!cache(home.path()).exists(), "no attempt, so nothing recorded");
}

/// Starts `serve` on a free port with the given check, returning it and the health URL.
fn serve(home: &Path, url: &str) -> (ServeGuard, String) {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let child = annotary(home, Some(url))
        .env("ANNOTARY_SETUP_HANDLE", "rowan")
        .args(["serve", "--port", &port.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    (ServeGuard::new(child), format!("http://127.0.0.1:{port}/api/v1/health"))
}

fn wait_healthy(health: &str) -> Duration {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        if ureq::get(health).call().is_ok() {
            return start.elapsed();
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("serve did not answer {health}");
}

/// Waits until `f` holds, or fails the test after `secs`.
fn wait_for(what: &str, secs: u64, f: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !f() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

// spec: S34-06
#[test]
fn serve_logs_what_it_learns_after_it_starts() {
    let home = tempfile::tempdir().unwrap();
    let api = FakeApi::start(Answer::Respond(200, TAG));
    let (guard, health) = serve(home.path(), &api.url);
    wait_healthy(&health);
    wait_for("serve asks for the latest release", 15, || cache(home.path()).exists() && api.asked() == 1);
    wait_for("the answer is written", 15, || {
        std::fs::read_to_string(cache(home.path())).is_ok_and(|c| c.contains("9.8.7"))
    });
    let out = guard.stop_with_output(Duration::from_secs(10));
    assert!(stderr(&out).contains("annotary 9.8.7 is available"), "{}", stderr(&out));
}

// spec: S34-06
#[test]
fn serve_logs_the_cached_notice_at_start_without_asking() {
    let home = tempfile::tempdir().unwrap();
    let api = FakeApi::start(Answer::Respond(200, TAG));
    write_cache(home.path(), now(), "9.0.0");
    let (guard, health) = serve(home.path(), &api.url);
    wait_healthy(&health);
    let out = guard.stop_with_output(Duration::from_secs(10));
    assert!(stderr(&out).contains("annotary 9.0.0 is available"), "{}", stderr(&out));
    assert_eq!(api.asked(), 0, "checked today already");
}

// spec: S34-06
#[test]
fn a_release_endpoint_that_never_answers_does_not_hold_up_the_server() {
    let home = tempfile::tempdir().unwrap();
    let api = FakeApi::start(Answer::Never);
    let (guard, health) = serve(home.path(), &api.url);
    let took = wait_healthy(&health);
    assert!(took < Duration::from_secs(3), "serve answered only after {took:?}");
    wait_for("the check started", 15, || api.asked() == 1);
    // The request is still hanging and the server still answers, then stops cleanly.
    assert!(ureq::get(&health).call().is_ok());
    let start = Instant::now();
    let out = guard.stop_with_output(Duration::from_secs(10));
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(start.elapsed() < Duration::from_secs(5), "a stop is not held up by the check");
}
