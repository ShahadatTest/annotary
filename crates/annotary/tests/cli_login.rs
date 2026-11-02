//! The real binary against the in-process server for what trycmd cannot drive: `login`
//! prompting on stdin (server, password), what it writes into
//! `config.toml`, `logout` revoking the Token, and the `profile` verbs — a server with a local
//! password (set through the store) and one without.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use annotary::config::Config;
use annotary::testing::TestServer;

fn run(home: &Path, args: &[&str], stdin: Option<&str>, env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_annotary"));
    cmd.args(args)
        .env("HOME", home)
        .env("ANNOTARY_HOME", home.join(".annotary"))
        .env_remove("ANNOTARY_TOKEN")
        .env_remove("ANNOTARY_SERVER")
        .env_remove("ANNOTARY_PROFILE")
        .env_remove("ANNOTARY_PASSWORD")
        .env_remove("ANNOTARY_TTY")
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
    if let Some(s) = stdin {
        child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
    }
    child.wait_with_output().unwrap()
}

fn text(o: &Output) -> (String, String) {
    (String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string())
}

fn config(home: &Path) -> Config {
    Config::load(&home.join(".annotary/config.toml")).unwrap()
}

// spec: S06-12, S06-13, S06-11
#[test]
fn login_prompts_saves_a_profile_and_logout_revokes() {
    let ts = TestServer::start();
    ts.store.set_password(&ts.owner_id, &ts.owner_id, Some("hunter2")).unwrap();
    let home = tempfile::tempdir().unwrap();
    let home = home.path();

    // 1. Prompts on stdin: server (typed), password — and nothing about a default.
    let out = run(home, &["login"], Some(&format!("{}\nhunter2\n", ts.url)), &[]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    assert_eq!(so, format!("Signed in as @rowan (owner). Saved profile \"rowan\" → {}\n", display(home)));
    assert!(se.contains("server [http://127.0.0.1:7377]: "), "{se}");
    assert!(se.contains("password: ********\n"), "{se}");
    assert!(!se.contains("default"), "{se}");
    assert!(se.contains("use it with -p rowan"), "{se}");
    assert!(!se.contains("hunter2"), "the password is never echoed: {se}");
    let c = config(home);
    let p = &c.profiles["rowan"];
    assert_eq!(p.server.as_deref(), Some(ts.url.as_str()));
    assert!(p.token.as_deref().is_some_and(|t| t.starts_with("ant_")));
    // The saved token works as @rowan with -p; without it nobody is assumed (exit 3).
    let out = run(home, &["member", "list"], None, &[]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out).1);
    let out = run(home, &["-p", "rowan", "member", "list"], None, &[]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert!(text(&out).0.starts_with("@rowan\tRowan Vale\towner\t"));
    // Its name on the server is `login`.
    let toks = ts.store.list_tokens(Some(&ts.owner_id)).unwrap();
    assert!(toks.iter().any(|t| t.info.name.as_deref() == Some("login")));

    // 2. `--server` skips the prompt; ANNOTARY_PASSWORD answers the password; -p names the
    //    profile.
    let out = run(home, &["-p", "work", "login", "--server", &ts.url], None, &[("ANNOTARY_PASSWORD", "hunter2")]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    assert!(so.starts_with("Signed in as @rowan (owner). Saved profile \"work\""), "{so}");
    assert!(!se.contains("server ["), "{se}");
    assert!(config(home).profiles.contains_key("work"));
    // The old flags are gone.
    let out = run(home, &["-p", "work", "login", "--server", &ts.url, "--default"], Some("hunter2\n"), &[]);
    assert_eq!(out.status.code(), Some(1));

    // 3. --json; ANNOTARY_PROFILE names the profile too.
    let out = run(home, &["login", "--server", &ts.url, "--json"], Some("hunter2\n"), &[("ANNOTARY_PROFILE", "again")]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    let v: serde_json::Value = serde_json::from_str(so.trim()).unwrap();
    assert_eq!(v["profile"], "again");
    assert_eq!(v["handle"], "rowan");
    assert_eq!(v["role"], "owner");
    assert!(v.get("default").is_none(), "{v}");
    assert_eq!(v["server"], ts.url);

    // 4. Wrong password ⇒ exit 3; empty ⇒ exit 1; unreachable server ⇒ exit 6.
    let out = run(home, &["login", "--server", &ts.url], Some("nope\n"), &[]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out).1);
    assert!(text(&out).1.contains("error: wrong password"), "{}", text(&out).1);
    let out = run(home, &["login", "--server", &ts.url], Some("\n"), &[]);
    assert_eq!(out.status.code(), Some(1));
    let out = run(home, &["login", "--server", "http://127.0.0.1:1"], Some("x\n"), &[]);
    assert_eq!(out.status.code(), Some(6), "{}", text(&out).1);
    let out = run(home, &["login", "--server", "ftp://x"], None, &[]);
    assert_eq!(out.status.code(), Some(1));

    // 5. `--json ?` lists fields without touching the server.
    let out = run(home, &["login", "--json", "?"], None, &[]);
    assert_eq!(text(&out).0.trim(), "profile server handle role config");

    // 6. logout: the Token is revoked on the server and removed from the profile (server kept);
    //    the profile is then unusable until the next login; idempotent-ish.
    let work_token = config(home).profiles["work"].token.clone().unwrap();
    assert!(ts.store.authenticate(&work_token).unwrap().is_some());
    let out = run(home, &["-p", "work", "logout"], None, &[]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    assert_eq!(so, "Signed out (@rowan): profile \"work\" — token revoked and removed\n");
    assert!(ts.store.authenticate(&work_token).unwrap().is_none(), "revoked server-side");
    let c = config(home);
    assert_eq!(c.profiles["work"].token, None);
    assert_eq!(c.profiles["work"].server.as_deref(), Some(ts.url.as_str()));
    let out = run(home, &["-p", "work", "member", "list"], None, &[]);
    assert_eq!(out.status.code(), Some(3));
    assert!(text(&out).1.contains("has no token"), "{}", text(&out).1);
    let out = run(home, &["-p", "work", "logout"], None, &[]);
    assert_eq!(out.status.code(), Some(0));
    assert!(text(&out).1.contains("nothing to forget"));
    // ANNOTARY_PROFILE names the profile for a bare `logout`; a token the server no longer
    // knows is still forgotten locally.
    let again_token = config(home).profiles["again"].token.clone().unwrap();
    let toks = ts.store.list_tokens(Some(&ts.owner_id)).unwrap();
    let row = toks.iter().find(|t| t.info.hash == annotary_store::token::hash(&again_token)).unwrap();
    ts.store.revoke_token(&ts.owner_id, &row.info.id).unwrap();
    let out = run(home, &["logout"], None, &[("ANNOTARY_PROFILE", "again")]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    assert!(se.contains("server did not revoke the token"), "{se}");
    assert_eq!(so, "Signed out: profile \"again\" — token removed\n");
    let out = run(home, &["-p", "ghost", "logout"], None, &[]);
    assert_eq!(out.status.code(), Some(2));
    // No profile named: usage error naming the fix.
    let out = run(home, &["logout"], None, &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out).1.contains("pass -p NAME"));
}

/// The path as the CLI prints it (`HOME` is the temp dir, so `~`).
fn display(_home: &Path) -> String {
    "~/.annotary/config.toml".to_string()
}

// spec: S06-12
#[test]
fn login_refuses_when_the_server_has_no_password() {
    let ts = TestServer::start();
    let home = tempfile::tempdir().unwrap();
    // The refusal comes before any password prompt (stdin is not even read).
    let out = run(home.path(), &["login", "--server", &ts.url], Some("whatever\n"), &[]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(5), "{se}");
    assert!(so.is_empty());
    assert!(se.starts_with(&format!("error: no local password is set on {} — ", ts.url)), "{se}");
    assert!(se.contains("member add"), "{se}");
    assert!(!se.contains("password:"), "no password prompt: {se}");
    assert!(!home.path().join(".annotary/config.toml").exists(), "nothing saved");
    // The CLI never became the owner: no session, no token, no profile.
    assert!(ts.store.list_tokens(Some(&ts.owner_id)).unwrap().len() == 1, "only the first-run token");
}

// spec: S06-13
#[test]
fn profile_verbs_manage_config_toml_without_printing_tokens() {
    let ts = TestServer::start();
    let (_, agent) = ts.add_member("agent");
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    let out = run(home, &["profile", "list"], None, &[]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(text(&out).0, "", "no profiles: empty non-TTY output");
    let out =
        run(home, &["profile", "add", "a", "--server", &ts.url, "--token", &agent, "--vault", "notes"], None, &[]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    assert_eq!(so, format!("profile \"a\" saved → {}\n", display(home)));
    let out = run(home, &["profile", "add", "a", "--server", &ts.url], None, &[]);
    assert_eq!(out.status.code(), Some(5), "exists");
    let out = run(home, &["profile", "add", "b", "--server", &ts.url, "--token", &ts.owner_token], None, &[]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let out = run(home, &["profile", "add", "down", "--server", "http://127.0.0.1:1", "--token", "ant_x"], None, &[]);
    assert_eq!(out.status.code(), Some(0));
    let out = run(home, &["profile", "add", "empty"], None, &[]);
    assert_eq!(out.status.code(), Some(0));
    assert!(text(&out).1.contains("has no token yet"));
    let out = run(home, &["profile", "add", "bad name"], None, &[]);
    assert_eq!(out.status.code(), Some(1));
    // list: NAME SERVER MEMBER; MEMBER resolved live, `-` when the server cannot say.
    let out = run(home, &["profile", "list"], None, &[]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    assert_eq!(
        so,
        format!(
            "a\t{u}\t@agent\nb\t{u}\t@rowan\ndown\thttp://127.0.0.1:1\t-\nempty\thttp://127.0.0.1:7377\t-\n",
            u = ts.url
        )
    );
    assert!(!so.contains("ant_"), "tokens are never printed");
    let out = run(home, &["profile", "list", "--json"], None, &[]);
    let v: serde_json::Value = serde_json::from_str(text(&out).0.trim()).unwrap();
    assert_eq!(v[0]["name"], "a");
    assert_eq!(v[0]["member"], "agent");
    assert_eq!(v[0]["vault"], "notes");
    assert!(v[1].get("default").is_none());
    assert!(!text(&out).0.contains("ant_"));
    let out = run(home, &["profile", "list", "--json", "?"], None, &[]);
    assert_eq!(text(&out).0.trim(), "name server member role vault has_token");
    // TTY parity: header + aligned columns, same fields.
    let out = run(home, &["profile", "list", "--no-color"], None, &[("ANNOTARY_TTY", "1")]);
    let so = text(&out).0;
    assert!(so.starts_with("NAME   SERVER"), "{so}");
    assert!(so.contains("@rowan"), "{so}");
    // There is no `profile default` any more.
    let out = run(home, &["profile", "default", "a"], None, &[]);
    assert_eq!(out.status.code(), Some(1));
    // remove; unknown ⇒ exit 2.
    let out = run(home, &["profile", "remove", "b"], None, &[]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    assert_eq!(so, format!("profile \"b\" removed → {}\n", display(home)));
    assert!(!config(home).profiles.contains_key("b"));
    let out = run(home, &["profile", "remove", "b"], None, &[]);
    assert_eq!(out.status.code(), Some(2));
    // config.toml stays owner-readable only.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(home.join(".annotary/config.toml")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

// spec: S01-14, S01-15
#[test]
fn an_agent_never_falls_back_to_the_owners_profile() {
    // The bug: config.toml had `default = "rowan"` (the owner) next to an `agent` profile; an agent
    // that ran `annotary thread reply …` without -p commented as Rowan.
    let ts = TestServer::start();
    let (_, agent) = ts.add_member("agent");
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    std::fs::create_dir_all(home.join(".annotary")).unwrap();
    std::fs::write(
        home.join(".annotary/config.toml"),
        format!(
            "default = \"rowan\"\n[profiles.rowan]\nserver = \"{u}\"\ntoken = \"{o}\"\n[profiles.agent]\nserver = \"{u}\"\ntoken = \"{agent}\"\n",
            u = ts.url,
            o = ts.owner_token
        ),
    )
    .unwrap();
    // No -p: refused (exit 3), the old default named as the way to act as the owner on purpose.
    let out = run(home, &["member", "list"], None, &[]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(3), "{se}");
    assert!(so.is_empty());
    assert!(se.contains("say who you are: pass -p NAME"), "{se}");
    assert!(se.contains("`default = \"rowan\"` is no longer used — pass -p rowan"), "{se}");
    // `profile list` tells the agent which profile is its own.
    let out = run(home, &["profile", "list"], None, &[]);
    assert_eq!(text(&out).0, format!("agent\t{u}\t@agent\nrowan\t{u}\t@rowan\n", u = ts.url));
    // -p agent acts as @agent — even when the human's shell exported the owner's token and
    // profile. `token list` shows the caller's own Tokens, so it names who the call was.
    let who = |args: &[&str], env: &[(&str, &str)]| {
        let out = run(home, args, None, env);
        assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
        let v: serde_json::Value = serde_json::from_str(text(&out).0.trim()).unwrap();
        let members: Vec<String> =
            v.as_array().unwrap().iter().map(|t| t["member"].as_str().unwrap().to_string()).collect();
        members
    };
    let exported = [("ANNOTARY_TOKEN", ts.owner_token.as_str()), ("ANNOTARY_PROFILE", "rowan")];
    assert_eq!(who(&["-p", "agent", "token", "list", "--json", "member"], &exported), vec!["agent"]);
    // …while the human, who names nothing, is who their environment says (the owner sees every
    // Member's Tokens, the agent only its own).
    assert!(who(&["token", "list", "--json", "member"], &exported).contains(&"rowan".to_string()));
    assert_eq!(who(&["token", "list", "--json", "member"], &[("ANNOTARY_PROFILE", "agent")]), vec!["agent"]);
}
