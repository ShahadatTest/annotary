//! Ticket 16's plumbing: `make check` is the definition of done (perf included, `PERF=0` /
//! `check-ci` without; the recipes live in `project.mk`, which the template's `Makefile`
//! includes), every workflow runs on GitHub-hosted runners (`cargo test` on Linux and macOS, `make e2e`
//! on Linux, `make perf` nowhere: perf/README.md has where), the release workflow keeps what the
//! pipeline relies on (docs/dev/releases.md), and the Phase 1 definition of done is recorded and linked. These read the files — the commands themselves are what `make check` runs.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The recipe lines of one Makefile target (`name:` … up to the next target / blank line).
fn target<'a>(makefile: &'a str, name: &str) -> &'a str {
    let start = makefile.find(&format!("\n{name}:")).unwrap_or_else(|| panic!("a `{name}:` target")) + 1;
    let rest = &makefile[start..];
    let end = rest[1..].find("\n\n").map(|i| i + 1).unwrap_or(rest.len());
    &rest[..end]
}

// spec: S16-06
#[test]
fn make_check_is_check_ci_plus_perf_and_perf_0_leaves_perf_out() {
    let mk = read("project.mk");
    let ci = target(&mk, "check-ci");
    for dep in ["fmt-check", "clippy", "test", "e2e", "spec-check", "wasm-check"] {
        assert!(ci.lines().next().unwrap().contains(dep), "check-ci depends on {dep}: {ci}");
    }
    assert!(!ci.lines().next().unwrap().contains("perf"), "check-ci runs no perf: {ci}");
    // `check` = check-ci + perf, unless PERF=0.
    assert!(mk.contains("PERF ?= 1"), "PERF defaults on");
    assert!(mk.contains("ifeq ($(PERF),0)\ncheck: check-ci\nelse\ncheck: check-ci perf\nendif"), "{mk}");
    // e2e = trycmd (the `e2e` recipe) + Playwright (`e2e: web`).
    assert!(target(&mk, "e2e").contains("--test e2e -- --ignored"));
    assert!(mk.contains("\ne2e: web"), "Playwright is a prerequisite of e2e");
    assert!(target(&mk, "spec-check").contains("--test spec_ids"));
    // `make -n` agrees (the Makefile parses and the conditional resolves).
    let dry = |perf: &str| {
        let out = std::process::Command::new("make")
            .args(["-n", "check", &format!("PERF={perf}")])
            .current_dir(root())
            .output()
            .expect("make on PATH");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    let with = dry("1");
    let without = dry("0");
    assert!(with.contains("perf/playwright.config.ts"), "make check runs the perf suite:\n{with}");
    assert!(!without.contains("perf/playwright.config.ts"), "PERF=0 make check skips it:\n{without}");
    for step in [
        "fmt --all -- --check",
        "clippy",
        "cargo test --workspace",
        "--test e2e -- --ignored",
        "--test spec_ids",
        "--test wasm",
    ] {
        assert!(without.contains(step), "{step} in make check:\n{without}");
    }
}

// One job's block: from `  name:` to the next line indented by exactly two spaces.
fn job(workflow: &str, name: &str) -> String {
    let mut lines = workflow.lines().skip_while(|l| *l != format!("  {name}:"));
    let head = lines.next().unwrap_or_else(|| panic!("a job {name}")).to_string();
    let body: Vec<&str> = lines.take_while(|l| !(l.starts_with("  ") && !l.starts_with("   "))).collect();
    format!("{head}\n{}", body.join("\n"))
}

// spec: S16-06, S15-07
#[test]
fn ci_runs_on_github_hosted_runners_and_perf_is_in_no_workflow() {
    let ci = read(".github/workflows/checks.yml");
    let test = job(&ci, "test");
    assert!(test.contains("runner: ubuntu-latest") && test.contains("runner: macos-latest"), "{test}");
    assert!(test.contains("make test") && test.contains("make clippy") && test.contains("make wasm-check"), "{test}");
    let e2e = job(&ci, "e2e");
    assert!(e2e.contains("runs-on: ubuntu-latest"), "{e2e}");
    assert!(e2e.contains("make e2e") && e2e.contains("playwright install --with-deps chromium"), "{e2e}");
    assert!(!ci.contains("\n  perf:"), "no perf job");
    // On pull requests and pushes to main.
    assert!(ci.contains("\non:\n  pull_request:\n  push:\n    branches: [main]\n"), "{ci}");
    let required = read(".github/workflows/ci.yml");
    assert!(job(&required, "ci").contains("runs-on: ubuntu-latest"), "{required}");

    // Across every workflow: no self-hosted runner, a fork's pull request could reach it, and no
    // `make perf`, whose absolute budgets mean nothing on a shared runner.
    let mut names = Vec::new();
    for entry in std::fs::read_dir(root().join(".github/workflows")).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        names.push(path.file_name().unwrap().to_string_lossy().to_string());
        for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
            assert!(!line.contains("self-hosted"), "{}: a self-hosted runner: {line}", path.display());
            assert!(!line.contains("make perf"), "{}: perf on a hosted runner: {line}", path.display());
        }
    }
    names.sort();
    assert_eq!(names, ["checks.yml", "ci.yml", "publish-tap.yml", "release.yml"]);

    // Every action is pinned to a commit, with what it is in a comment: a tag or a branch can move.
    for name in &names {
        let text = read(&format!(".github/workflows/{name}"));
        for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
            let Some((_, used)) = line.split_once("uses: ") else { continue };
            if used.starts_with("./") {
                continue; // this repo's own workflow, at this commit
            }
            let (action, comment) = used.split_once(" # ").unwrap_or((used, ""));
            let sha = action.rsplit_once('@').map_or("", |(_, r)| r);
            assert!(
                sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "{name}: not a 40-character SHA: {line}"
            );
            assert!(!comment.trim().is_empty(), "{name}: no version in a comment: {line}");
        }
    }

    // perf/README.md's contract is what happens instead.
    let readme = read("perf/README.md");
    for needle in
        ["Never on a GitHub-hosted runner", "dedicated Mac", "nightly", "make perf", "playwright install chromium"]
    {
        assert!(readme.contains(needle), "{needle} in perf/README.md");
    }
}

// spec: S34-07
#[test]
fn the_release_workflow_keeps_what_the_pipeline_relies_on() {
    let release = read(".github/workflows/release.yml");
    let manifest = read("Cargo.toml");
    // The installer in the workflow is the version the workspace pins, and is checked before it runs,
    // in both jobs that install it: never piped to a shell, and never the plan's unchecked command.
    let pinned = manifest.split("cargo-dist-version = \"").nth(1).unwrap().split('"').next().unwrap();
    let installer = format!("cargo-dist/releases/download/v{pinned}/cargo-dist-installer.sh");
    assert!(release.contains(&installer), "{pinned}");
    assert!(!release.contains("install_dist.run"), "dist's unchecked install command");
    assert!(!release.contains("cargo-dist-installer.sh | sh"), "the installer piped to a shell");
    for name in ["plan", "build-local-artifacts"] {
        let block = job(&release, name);
        let (fetch, check, run) = (
            block.find(&installer).unwrap_or_else(|| panic!("{name} fetches the installer")),
            block.find("| shasum -a 256 -c -").unwrap_or_else(|| panic!("{name} checks it")),
            block.find("sh \"$RUNNER_TEMP/dist-installer.sh\"").unwrap_or_else(|| panic!("{name} runs it")),
        );
        assert!(fetch < check && check < run, "{name} checks the installer before it runs it:\n{block}");
    }
    // Read-only unless a job says otherwise, and only the job that makes the release does.
    assert!(release.contains("\npermissions:\n  \"contents\": \"read\"\n"), "read-only at the top");
    assert_eq!(release.matches("\"contents\": \"write\"").count(), 1, "one job writes");
    let host = job(&release, "host");
    assert!(host.contains("\"contents\": \"write\"") && host.contains("gh release create"), "{host}");
    // A release is made on a tag, and a pull request still plans.
    assert!(release.contains("pull_request:\n  push:\n    tags:"), "{release}");
    assert!(job(&release, "plan").contains("|| 'plan' }}"), "dist plan on a pull request");
    // dist refuses a tag that is not the package's version, and the repo keeps 0.0.0: every job that
    // calls dist stamps the tag first.
    for name in ["plan", "build-local-artifacts", "build-global-artifacts", "host"] {
        let block = job(&release, name);
        let stamp = block.find("deploy/stamp_version.py").unwrap_or_else(|| panic!("{name} stamps"));
        let first_call = ["dist $", "dist build", "dist host"].iter().filter_map(|c| block.find(c)).min().unwrap();
        assert!(stamp < first_call, "{name} stamps before it calls dist:\n{block}");
    }
    // Notes from the pull request titles, which the `ci` job holds to Conventional Commits.
    assert!(release.contains("--generate-notes"), "{release}");
    // The tap's job goes through ours, with this repo's token read-only.
    let tap_job = job(&release, "custom-publish-tap");
    assert!(tap_job.contains("uses: ./.github/workflows/publish-tap.yml"), "{tap_job}");
    assert!(tap_job.contains("\"contents\": \"read\""), "{tap_job}");
    assert!(!tap_job.contains("write"), "{tap_job}");
    // It gets the one secret it needs, by name.
    assert!(!release.contains("secrets: inherit"), "no job gets every secret");
    assert!(tap_job.contains("HOMEBREW_TAP_DEPLOY_KEY: ${{ secrets.HOMEBREW_TAP_DEPLOY_KEY }}"), "{tap_job}");
    assert!(job(&release, "announce").contains("custom-publish-tap"), "the announcement waits for the tap");

    let tap = read(".github/workflows/publish-tap.yml");
    // The deploy key is only in the job that pushes, which runs no third-party code and only after
    // the job that does has passed.
    let (check, push) = (job(&tap, "check"), job(&tap, "push"));
    assert!(push.contains("ssh-key: ${{ secrets.HOMEBREW_TAP_DEPLOY_KEY }}"), "{push}");
    assert!(push.contains("needs: check"), "{push}");
    assert!(!check.contains("secrets.") && !check.contains("ssh-key"), "no secret where Homebrew runs: {check}");
    for line in push.lines().filter(|l| l.contains("uses:")) {
        assert!(line.contains("uses: actions/"), "only GitHub's actions beside the key: {line}");
    }
    for needle in ["brew ", "setup-homebrew", "deploy/"] {
        assert!(!push.contains(needle), "{needle} beside the key: {push}");
    }
    // The formula is put aside before anything from Homebrew runs.
    assert!(check.find("checked-formula").unwrap() < check.find("setup-homebrew").unwrap(), "{check}");
    // The push runs git, and nothing after it.
    assert!(push.trim_end().ends_with("git -C tap push"), "{push}");
    assert!(!tap.contains("HOMEBREW_TAP_TOKEN") && !tap.contains("token:"), "no personal token: {tap}");
    assert!(tap.contains("repository: josh-andrews-dev/homebrew-tap"), "{tap}");
    // Nothing reaches users until the formula has its test and has passed.
    let order = [
        "deploy/tap_formula.py",
        "brew audit --strict --online",
        "brew install --formula",
        "brew test",
        "git -C tap push",
    ];
    let at: Vec<usize> = order.iter().map(|o| tap.find(o).unwrap_or_else(|| panic!("{o}"))).collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{order:?} in that order: {at:?}");

    // What the README tells people to run is what these produce.
    let readme = read("README.md");
    assert!(readme.contains("brew install josh-andrews-dev/tap/annotary"), "the tap's name");
    assert!(readme.contains("releases/latest/download/annotary-installer.sh"), "the installer's name");
    assert!(manifest.contains("tap = \"josh-andrews-dev/homebrew-tap\""));
}

// spec: S16-07
#[test]
fn the_phase_1_definition_of_done_is_recorded_and_linked() {
    let dod = read("docs/PHASE1-DONE.md");
    for needle in ["16", "make check", "perf/BUDGETS.md", "Raised budgets", "tests/SPEC.md", "checks.yml"] {
        assert!(dod.contains(needle), "{needle} in docs/PHASE1-DONE.md");
    }
    // The budget record it points at: no budget raised at the close of Phase 1.
    let budgets = read("perf/BUDGETS.md");
    let raised = budgets.split("### Raised budgets").nth(1).expect("a `### Raised budgets` heading");
    assert!(
        raised.trim_start().starts_with("None."),
        "no budget raised: {}",
        raised.lines().take(3).collect::<Vec<_>>().join(" / ")
    );
    // Linked from AGENTS.md (the entry point for agents) and the DoD links the rule.
    let agents = read("AGENTS.md");
    assert!(agents.contains("docs/PHASE1-DONE.md"), "AGENTS.md links the definition of done");
    assert!(dod.contains("budget-raise rule"));
}
