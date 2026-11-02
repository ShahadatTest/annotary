//! CMD-23: what `deploy/install-linux.sh` relies on to cross a schema migration safely, and the
//! installer's own stubbed test (`deploy/test_install_linux.sh`, also `make deploy-test`).
//!
//! - `serve --print-schema-version` prints `SCHEMA_VERSION` and never touches the data dir;
//! - `export` of a store older than the binary leaves it at its schema (export never migrates),
//!   so the pre-migration tar holds the store the previous binary can open;
//! - the installer writes `snapshots/pre-schema-N.tar` before a newer schema starts and restores
//!   it when the new binary does not come up.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use annotary_store::migrations::{MIGRATIONS, current_version};
use annotary_store::{Meta, SCHEMA_VERSION};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_annotary"))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn cli(data_dir: &Path) -> Command {
    let mut c = Command::new(bin());
    c.arg("--data-dir")
        .arg(data_dir)
        .env_remove("ANNOTARY_HOME")
        .env_remove("ANNOTARY_TTY")
        .env_remove("ANNOTARY_TOKEN")
        .env_remove("ANNOTARY_SERVER")
        .env_remove("ANNOTARY_PROFILE")
        .stdin(Stdio::null());
    c
}

// spec: S29-40
#[test]
fn print_schema_version_prints_the_binary_schema_and_never_touches_the_data_dir() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let out = cli(&data).args(["serve", "--print-schema-version"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout), format!("{SCHEMA_VERSION}\n"));
    assert!(!data.exists(), "--print-schema-version must not create the data dir");
}

// spec: S29-40
#[test]
fn export_of_an_older_store_never_migrates_it() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let store = data.join("store");
    std::fs::create_dir_all(&store).unwrap();
    let older = SCHEMA_VERSION - 1;
    {
        let mut conn = rusqlite::Connection::open(store.join(annotary_store::DB_FILE)).unwrap();
        for sql in MIGRATIONS.iter().take(older as usize) {
            let tx = conn.transaction().unwrap();
            tx.execute_batch(sql).unwrap();
            tx.commit().unwrap();
        }
        conn.pragma_update(None, "user_version", older).unwrap();
    }
    Meta { schema_version: older, tenant_id: None, created_at: "2026-10-30T00:00:00Z".into() }.write(&store).unwrap();

    let tar = dir.path().join("pre-schema.tar");
    let out = cli(&data).arg("export").arg(&tar).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains(&format!("schema v{older}")));

    let conn = rusqlite::Connection::open(store.join(annotary_store::DB_FILE)).unwrap();
    assert_eq!(current_version(&conn).unwrap(), older, "export must not migrate the store");
    assert_eq!(Meta::read(&store).unwrap().unwrap().schema_version, older);
    assert!(!store.join(annotary_store::LOCK_FILE).exists(), "export takes no LOCK");

    // The tar restores to a store at the older schema.
    let restored = dir.path().join("restored");
    std::fs::create_dir_all(&restored).unwrap();
    let st = Command::new("tar").arg("-xf").arg(&tar).arg("-C").arg(&restored).status().unwrap();
    assert!(st.success());
    let conn = rusqlite::Connection::open(restored.join("store").join(annotary_store::DB_FILE)).unwrap();
    assert_eq!(current_version(&conn).unwrap(), older);
}

/// Whether a deploy test script passed, with its stdout and stderr.
#[cfg(unix)]
type ScriptRun = (bool, String, String);

/// `test_install_linux.sh` takes a while and two tests read it, so each script runs once.
#[cfg(unix)]
fn script_output(script: &'static str) -> &'static ScriptRun {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static RUNS: OnceLock<Mutex<HashMap<&'static str, &'static ScriptRun>>> = OnceLock::new();
    let mut runs = RUNS.get_or_init(Default::default).lock().unwrap();
    runs.entry(script).or_insert_with(|| {
        let out = Command::new("bash").arg(repo_root().join(script)).output().unwrap();
        Box::leak(Box::new((
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )))
    })
}

/// Runs one of the deploy test scripts and checks the named lines passed.
#[cfg(unix)]
fn script_passes(script: &'static str, lines: &[&str]) {
    let (passed, stdout, stderr) = script_output(script);
    assert!(*passed, "{script} failed:\n{stdout}\n{stderr}");
    for line in lines {
        assert!(stdout.contains(line), "missing {line:?} in:\n{stdout}");
    }
}

// spec: S29-40
#[cfg(unix)]
#[test]
fn install_linux_exports_before_a_migration_and_restores_it_on_rollback() {
    script_passes(
        "deploy/test_install_linux.sh",
        &[
            "ok   a schema bump writes pre-schema-4.tar before setup",
            "ok   no schema bump writes no pre-schema tar",
            "ok   a failed start after a bump restores the export",
            "ok   a failed start after a bump keeps the migrated store aside",
            "ok   a failed start after a bump starts annotary.prev",
        ],
    );
}

// spec: S33-01, S33-02, S33-03, S33-09, S33-10, S33-11, S33-12
#[cfg(unix)]
#[test]
fn install_linux_checks_a_release_and_keeps_the_hosts_env_file() {
    script_passes(
        "deploy/test_install_linux.sh",
        &[
            "ok   a release installs its binary",
            "ok   a release upgrade keeps .prev",
            "ok   a tarball that does not match its SHA-256 refuses",
            "ok   a tarball that does not match its SHA-256 leaves the installed binary",
            "ok   a missing checksum refuses",
            "ok   a tarball holding another version refuses",
            "ok   an empty env file gets the URL",
            "ok   what the host's file has is kept",
            "ok   the env file is mode 600 whatever it was",
            "ok   no URL anywhere refuses",
            "ok   a server answering with another URL rolls back",
            "ok   no --tailscale-service: nothing is served",
            "ok   an empty ANNOTARY_URL with no --url refuses",
            "ok   an empty ANNOTARY_URL given --url holds it",
            "ok   ANNOTARY_URL=none needs no --url",
            "ok   a keyless file given --url installs",
            "ok   a URL that goes empty before the start check rolls back",
            "ok   the store's own values fill an empty file with no override",
            "ok   a mistyped URL refuses",
            "ok   a mistyped link base refuses",
            "ok   a link base left to its default of none refuses",
            "ok   --accept-new-url changes the URL on purpose",
            "ok   none matches a store with no URL and no link base",
            "ok   a first install has nothing to compare with",
            "ok   a build in a folder only its owner can read installs",
            "ok   an upgrade asks the store schema as nobody",
            "ok   a rollback runs nothing as root",
            "ok   a release runs nothing as root",
            "ok   a rollback puts the previous check back",
            "ok   a failed first install keeps no previous check",
        ],
    );
}

// spec: S33-04
#[cfg(unix)]
#[test]
fn the_unit_takes_its_url_from_the_env_file_and_names_no_host() {
    let unit = std::fs::read_to_string(repo_root().join("deploy/annotary.service")).unwrap();
    let exec = unit.lines().find(|l| l.starts_with("ExecStart=")).unwrap();
    assert!(exec.contains("--url ${ANNOTARY_URL} --link-base ${ANNOTARY_LINK_BASE}"), "{exec}");
    // No dash: without the file the unit does not start, rather than running with no URL.
    assert!(unit.lines().any(|l| l == "EnvironmentFile=/etc/annotary/env"), "{unit}");
}

// spec: S33-05, S33-09
#[cfg(unix)]
#[test]
fn health_catches_a_lost_url() {
    script_passes(
        "deploy/test_healthcheck.sh",
        &[
            "ok   no env file is critical",
            "ok   an env file without the URL is critical, though the store remembers one",
            "ok   a server with no URL, with one in the file, is critical",
            "ok   settings disagreeing with the file are critical",
            "ok   none says no URL on purpose",
            "ok   an empty ANNOTARY_URL is critical, with no URL on the server",
        ],
    );
}

// spec: S33-06, S33-07
#[cfg(unix)]
#[test]
fn the_host_is_a_setting_and_targets_refuse_clearly_without_it() {
    script_passes(
        "deploy/test_deploy.sh",
        &[
            "ok   no settings file: a required key refuses",
            "ok   the environment beats the file",
            "ok   an unknown key is refused",
            "ok   release with no tag on HEAD refuses",
            "ok   release runs the installer over stdin as root with the tag",
            "ok   build installs the built binary with the settings",
            "ok   the settings' URL goes to the check",
            "ok   make deploy's check names the keys",
            "ok   make verify-live's check names the keys",
        ],
    );
}

// spec: S34-07
#[cfg(unix)]
#[test]
fn the_release_tools_do_what_the_pipeline_relies_on() {
    let out = Command::new("python3").arg(repo_root().join("deploy/test_release_tools.py")).output().unwrap();
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

// spec: S34-07
#[test]
fn the_workspace_keeps_the_placeholder_version_and_dist_can_release_the_binary() {
    let manifest = std::fs::read_to_string(repo_root().join("Cargo.toml")).unwrap();
    assert!(manifest.contains("[workspace.package]"));
    assert!(manifest.contains("version = \"0.0.0\""), "the version comes from the tag");
    let dist = manifest.split("[workspace.metadata.dist]").nth(1).expect("the dist config");
    let dist = dist.split("\n[").next().unwrap();
    for needle in [
        "installers = [\"shell\", \"homebrew\"]",
        "tap = \"josh-andrews-dev/homebrew-tap\"",
        "publish-jobs = [\"./publish-tap\"]",
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        "checksum = \"sha256\"",
        "deploy/annotary.service",
        "deploy/healthcheck.sh",
    ] {
        assert!(dist.contains(needle), "{needle} in {dist}");
    }
    let krate = std::fs::read_to_string(repo_root().join("crates/annotary/Cargo.toml")).unwrap();
    assert!(krate.contains("[package.metadata.dist]\ndist = true"), "{krate}");
}
