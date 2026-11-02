//! CLI end-to-end goldens (`trycmd`) against a real spawned `annotary serve`.
//!
//! Ignored under plain `cargo test`; `make e2e` runs it (`cargo test -p annotary --test e2e --
//! --ignored`). Regenerate goldens with `TRYCMD=overwrite make e2e`.
//!
//! Layout: `tests/e2e/<NN>-<group>/*.toml|*.trycmd` at the repo root. Groups run in lexical
//! order; cases inside a group run in parallel and must not depend on each other (use a
//! `.trycmd` file for a dependent sequence). The harness sets `HOME` to a temp dir whose
//! `~/.annotary/config.toml` has profiles `rowan` (owner), `agent` (member @scribe), `down`
//! (unreachable server), `limited` (member with `notes: read,comment` only) and `pw` (a second
//! server whose owner @ana has the local password `hunter2`; no token — `login` fills it), plus
//! `linter` (member @linter) from group `95-agent-flow` on (ticket 14), and `ios` (the second
//! server's owner @ana, whose server has a public URL and a link relay) from `99-ios` on (ticket 18);
//! cases pick one with `-p`. `00-setup` cases use their own fresh `ANNOTARY_HOME` inside
//! the sandbox. Ticket 13: every case runs with `ANNOTARY_LAUNCHD_LABEL=com.annotary.e2e`
//! (the `daemon` goldens look at launchd read-only under that private label).

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::serve::ServeGuard;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_annotary"))
}

fn token_from(stdout: &str) -> String {
    stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("ANNOTARY_TOKEN=").map(|t| t.trim().to_string()))
        .expect("a ANNOTARY_TOKEN= line")
}

fn cli(home: &Path) -> Command {
    let mut c = Command::new(bin());
    c.env("HOME", home)
        .env("USER", "rowan")
        .env_remove("ANNOTARY_HOME")
        .env_remove("ANNOTARY_TOKEN")
        .env_remove("ANNOTARY_SERVER")
        .env_remove("ANNOTARY_PROFILE")
        .env_remove("ANNOTARY_TTY");
    c
}

#[test]
#[ignore = "spawns a real `annotary serve`; run via `make e2e`"]
fn e2e() {
    // Cases inherit this process's environment; scrub anything that would leak in from the
    // developer's shell (case files set what they need themselves).
    for k in [
        "ANNOTARY_HOME",
        "ANNOTARY_TOKEN",
        "ANNOTARY_SERVER",
        "ANNOTARY_PROFILE",
        "ANNOTARY_TTY",
        "NO_COLOR",
        "ANNOTARY_SETUP_HANDLE",
        "ANNOTARY_SETUP_NAME",
        "ANNOTARY_SETUP_PASSWORD",
        "ANNOTARY_PASSWORD",
        // ---- ticket 13 ----
        "ANNOTARY_DAEMON_OS",
        "ANNOTARY_LAUNCHD_LABEL",
        "ANNOTARY_HOUSEKEEPING_INTERVAL",
        // ---- end ticket 13 ----
    ] {
        // SAFETY: single-threaded at this point (start of the only test in this binary).
        unsafe { std::env::remove_var(k) };
    }
    let root = repo_root();
    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path();
    let dd = home.join(".annotary");

    // 1. First-run setup, scripted, then a real server on an ephemeral port.
    let out = cli(home)
        .args(["serve", "--setup-only"])
        .env("ANNOTARY_SETUP_HANDLE", "rowan")
        .env("ANNOTARY_SETUP_NAME", "Rowan Vale")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "setup failed: {}", String::from_utf8_lossy(&out.stderr));
    let owner_token = token_from(&String::from_utf8_lossy(&out.stdout));

    let child = cli(home)
        .args(["serve", "--port", "0"])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut serve = ServeGuard::new(child); // stopped on drop, also when a golden panics
    let server_json = dd.join("server.json");
    let deadline = Instant::now() + Duration::from_secs(15);
    let url = loop {
        if let Some(info) = annotary::server::ServerJson::read(&server_json) {
            break info.url;
        }
        assert!(Instant::now() < deadline, "server.json never appeared");
        std::thread::sleep(Duration::from_millis(20));
    };
    // spec: S01-20 — `--port 0` picked a free port and server.json/log name it.
    assert!(url.starts_with("http://127.0.0.1:"));

    // 2. Seed members with the real CLI (owner token via env), then profiles in config.toml.
    let add = |handle: &str, name: Option<&str>| -> String {
        let mut c = cli(home);
        c.args(["member", "add", handle]).env("ANNOTARY_TOKEN", &owner_token).env("ANNOTARY_SERVER", &url);
        if let Some(n) = name {
            c.args(["--name", n]);
        }
        let out = c.output().unwrap();
        assert!(out.status.success(), "member add {handle}: {}", String::from_utf8_lossy(&out.stderr));
        token_from(&String::from_utf8_lossy(&out.stdout))
    };
    let scribe_token = add("scribe", Some("Scribe"));
    // ticket 14: @linter's token becomes profile `linter` right before `95-agent-flow` (see below).
    let linter_token = add("linter", None);
    // ---- ticket 06: a Member restricted to `notes: read,comment` (exit-4 goldens), and a
    // second server whose owner has a local password (the `login` goldens; profile `pw` names
    // it and gets its token from `login` itself).
    let limited_token = add("limited", Some("Limited Agent"));
    for args in [["member", "grant", "limited", "*", "none"], ["member", "grant", "limited", "notes", "read,comment"]] {
        let out =
            cli(home).args(args).env("ANNOTARY_TOKEN", &owner_token).env("ANNOTARY_SERVER", &url).output().unwrap();
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }
    let home2_dir = tempfile::tempdir().unwrap();
    let home2 = home2_dir.path();
    let out = cli(home2)
        .args(["serve", "--setup-only"])
        .env("ANNOTARY_SETUP_HANDLE", "ana")
        .env("ANNOTARY_SETUP_NAME", "Ana Lima")
        .env("ANNOTARY_SETUP_PASSWORD", "hunter2")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "setup 2 failed: {}", String::from_utf8_lossy(&out.stderr));
    // ticket 18: @ana's first Token becomes profile `ios` right before `99-ios`.
    let ana_token = token_from(&String::from_utf8_lossy(&out.stdout));
    // ticket 18: the second server knows the URL other devices use and the link relay
    // (`99-ios`'s `link` / `pair` / TTY `show` goldens); the main one has neither.
    let child2 = cli(home2)
        .args([
            "serve",
            "--port",
            "0",
            "--url",
            "http://pw-host.example.ts.net:7377",
            "--link-base",
            "https://cd.example.dev",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut serve2 = ServeGuard::new(child2);
    let server_json2 = home2.join(".annotary/server.json");
    let deadline = Instant::now() + Duration::from_secs(15);
    let url2 = loop {
        if let Some(info) = annotary::server::ServerJson::read(&server_json2) {
            break info.url;
        }
        assert!(Instant::now() < deadline, "server.json (2) never appeared");
        std::thread::sleep(Duration::from_millis(20));
    };
    // ---- end ticket 06
    let config = format!(
        "[profiles.rowan]\nserver = \"{url}\"\ntoken = \"{owner_token}\"\n\n\
         [profiles.agent]\nserver = \"{url}\"\ntoken = \"{scribe_token}\"\nvault = \"notes\"\n\n\
         [profiles.down]\nserver = \"http://127.0.0.1:1\"\ntoken = \"ant_unused\"\n\n\
         [profiles.limited]\nserver = \"{url}\"\ntoken = \"{limited_token}\"\n\n\
         [profiles.pw]\nserver = \"{url2}\"\n"
    );
    std::fs::write(dd.join("config.toml"), &config).unwrap();
    // ticket 14: profile `linter` joins before `95-agent-flow` — the CLI prototype's cast (@rowan,
    // @scribe, @linter) plays the golden flows there. Not earlier: `75-access` asserts that
    // @linter's first Token was never used, and `profile list` would stamp it.
    let config_with_linter = format!("{config}\n[profiles.linter]\nserver = \"{url}\"\ntoken = \"{linter_token}\"\n");
    // ticket 18: profile `ios` (the second server's owner @ana) joins before `99-ios` only — any
    // earlier `profile list` golden would list it.
    let config_with_ios =
        format!("{config_with_linter}\n[profiles.ios]\nserver = \"{url2}\"\ntoken = \"{ana_token}\"\n");

    // 3. Golden groups, in order.
    let mut groups: Vec<PathBuf> =
        std::fs::read_dir(root.join("tests/e2e")).unwrap().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    groups.sort();
    assert!(!groups.is_empty());
    // Local iteration only: `E2E_GROUPS=95-agent-flow,96-human-tty` runs just the groups whose
    // directory name contains one of the words (the seeds of earlier groups are then absent —
    // a golden that reads them will fail; `make e2e` always runs everything).
    if let Ok(only) = std::env::var("E2E_GROUPS") {
        let words: Vec<String> = only.split(',').map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect();
        if !words.is_empty() {
            groups.retain(|g| {
                let name = g.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                words.iter().any(|w| name.contains(w.as_str()))
            });
        }
    }
    // ticket 14: the agent golden flow needs one step that runs two commands at once (`wait`
    // parked, then `act` from a second profile) and one that edits a pulled file — `sh -c`
    // does both; the binary's directory goes on PATH so `annotary` resolves inside it.
    let bin_dir = bin().parent().unwrap().to_path_buf();
    let path_env =
        format!("{}:{}", bin_dir.display(), std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string()));
    for group in groups {
        if group.file_name().is_some_and(|n| n == "95-agent-flow") {
            std::fs::write(dd.join("config.toml"), &config_with_linter).unwrap();
        }
        if group.file_name().is_some_and(|n| n == "99-ios") {
            std::fs::write(dd.join("config.toml"), &config_with_ios).unwrap();
        }
        let cases = trycmd::TestCases::new();
        cases
            .case(group.join("*.toml"))
            .case(group.join("*.trycmd"))
            .register_bin("sh", Path::new("/bin/sh"))
            .env("PATH", path_env.clone())
            .env("HOME", home.to_str().unwrap())
            .env("USER", "rowan")
            // ticket 13: `daemon` cases may look at launchd (read-only) — under a private label,
            // so the developer's real com.annotary.server is never touched.
            .env("ANNOTARY_LAUNCHD_LABEL", "com.annotary.e2e")
            // `94-corpus` copies the made-up corpus from here rather than keeping a second copy.
            .env("ANNOTARY_E2E_FIXTURES", repo_root().join("fixtures").to_str().unwrap())
            .insert_var("[URL]", url.clone())
            .unwrap();
        cases.run();
    }

    // 4. spec: S01-11 — a token minted by `member add` works when pasted into an agent's env.
    let out = cli(home)
        .args(["member", "add", "pasted", "--name", "Pasted Agent"])
        .env("ANNOTARY_PROFILE", "rowan")
        .output()
        .unwrap();
    assert!(out.status.success());
    let pasted = token_from(&String::from_utf8_lossy(&out.stdout));
    let out = cli(home)
        .args(["member", "list"])
        .env("ANNOTARY_TOKEN", &pasted)
        .env("ANNOTARY_SERVER", &url)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("@pasted\tPasted Agent\tmember\t*:read,comment,action,write")
    );

    // 5. spec: S01-02 — a second `serve` on the same store is refused (exit 5) while this one runs.
    let out = cli(home).args(["serve", "--port", "0"]).stdin(Stdio::null()).output().unwrap();
    assert_eq!(out.status.code(), Some(5), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("locked by another annotary serve"));

    // ticket 06: the password server is done once the goldens are.
    serve2.sigterm();
    serve2.wait_timeout(Duration::from_secs(10));

    // 6. spec: S01-03 — SIGTERM = clean exit: server.json removed, store unlocked.
    serve.sigterm();
    let status = serve.wait_timeout(Duration::from_secs(10));
    assert!(status.success(), "serve exits 0 on SIGTERM: {status}");
    assert!(!server_json.exists(), "server.json removed on clean exit");
    let out = cli(home).args(["serve", "--setup-only"]).stdin(Stdio::null()).output().unwrap();
    assert!(out.status.success(), "store unlocked after exit: {}", String::from_utf8_lossy(&out.stderr));
    // spec: S01-16 — with the server gone the CLI fails fast with exit 6.
    let out = cli(home).args(["member", "list"]).env("ANNOTARY_PROFILE", "rowan").output().unwrap();
    assert_eq!(out.status.code(), Some(6));
}
