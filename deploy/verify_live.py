#!/usr/bin/env python3
"""make verify-live (CMD-22): test a build end to end on a throwaway copy of the live store.

Reads the live server's snapshots where they are: on this machine, or on the host SERVER_SSH names
(over ssh, with sudo there). `make verify-live` gives it the settings (deploy/with_settings.py). One
run:

1. sweeps copies an earlier, crashed run left behind (`cd-verify-live-*` folders of this user
   whose run is no longer alive);
2. extracts the newest snapshot matching VERIFY_SNAPSHOT_GLOB (read with `sudo -n` when this user
   cannot read it, always over ssh) into a fresh private folder (mode 700) and chowns it to this
   user;
3. clears every local password in the copy (`annotary password --clear`), so a browser on
   this machine is trusted as the owner;
4. serves the copy with the build under test on a free loopback port (never 7377, never
   beyond 127.0.0.1, no link relay);
5. signs in over loopback, mints a Token in the copy and writes a private CLI home whose only
   profile, `copy`, points at the copy;
6. runs the Playwright scenarios in `verify/` (unless --no-scenarios) with VERIFY_URL,
   VERIFY_CLI_HOME, VERIFY_BIN and VERIFY_OUT (where scenarios save PR screenshots);
7. always (also on failure or Ctrl-C) stops the server's and Playwright's process groups and
   deletes the folder, Playwright's traces included; on Linux both also get SIGTERM if this
   script dies, and the next run's sweep stops whatever a hard-killed run left;
8. prints one JSON verdict on stdout; exit 0 when every check and scenario passed, else 1.

It never writes to the live data dir and never touches the live unit (VERIFY_UNIT); the verdict
checks that the unit was not restarted, the snapshot it copied is unchanged, and the live server refuses
the copy's Token.
"""

from __future__ import annotations

import argparse
import fnmatch
import glob
import hashlib
import http.client
import json
import os
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import tarfile
import tempfile
import time

# No default: where the live server keeps these is a setting, not something this repo knows.
LIVE_SNAPSHOT_GLOB = os.environ.get("VERIFY_SNAPSHOT_GLOB") or ""
LIVE_PORT = 7377
UNIT = os.environ.get("VERIFY_UNIT") or ""
SERVER_SSH = os.environ.get("SERVER_SSH") or ""
PREFIX = "cd-verify-live-"
REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def note(msg: str) -> None:
    print(f"verify-live: {msg}", file=sys.stderr, flush=True)


def over_ssh(args: list[str], stdin: bytes | None = None, stdout=subprocess.PIPE) -> subprocess.CompletedProcess:
    """Run `args` as root on the SERVER_SSH host. Arguments are quoted for its shell."""
    remote = " ".join(shlex.quote(a) for a in ["sudo", "-n", *args])
    return subprocess.run(
        ["ssh", "-o", "ConnectTimeout=10", "-o", "BatchMode=yes", SERVER_SSH, remote],
        input=stdin, stdout=stdout, stderr=subprocess.PIPE,
    )


def sudo_ok() -> bool:
    return subprocess.run(["sudo", "-n", "true"], capture_output=True).returncode == 0


def newest_snapshot(pattern: str) -> str | None:
    """The newest snapshot matching `pattern` (by name: they sort by time), via sudo if needed."""
    folder, glob_name = os.path.split(pattern)
    if SERVER_SSH:
        out = over_ssh(["ls", folder])
        if out.returncode != 0:
            return None
        names = out.stdout.decode().split()
    else:
        try:
            names = os.listdir(folder)
        except PermissionError:
            if not sudo_ok():
                return None
            out = subprocess.run(["sudo", "-n", "ls", folder], capture_output=True, text=True)
            names = out.stdout.split()
        except FileNotFoundError:
            return None
    tars = sorted(n for n in names if n.endswith(".tar") and fnmatch.fnmatchcase(n, glob_name))
    return os.path.join(folder, tars[-1]) if tars else None


def snapshot_mtime(path: str, remote: bool = False) -> float | None:
    if remote:
        out = over_ssh(["stat", "-c", "%Y", path])
        return float(out.stdout) if out.returncode == 0 else None
    try:
        return os.stat(path).st_mtime
    except PermissionError:
        out = subprocess.run(["sudo", "-n", "stat", "-c", "%Y", path], capture_output=True, text=True)
        return float(out.stdout.strip()) if out.returncode == 0 else None
    except FileNotFoundError:
        return None


def snapshot_hash(path: str, remote: bool = False) -> str | None:
    """sha256 of the snapshot (the live server moves an unchanged snapshot's mtime every tick)."""
    if remote:
        out = over_ssh(["sha256sum", path])
        return out.stdout.decode().split()[0] if out.returncode == 0 and out.stdout else None
    try:
        h = hashlib.sha256()
        with open(path, "rb") as f:
            for chunk in iter(lambda: f.read(1 << 20), b""):
                h.update(chunk)
        return h.hexdigest()
    except PermissionError:
        out = subprocess.run(["sudo", "-n", "sha256sum", path], capture_output=True, text=True)
        return out.stdout.split()[0] if out.returncode == 0 and out.stdout else None
    except FileNotFoundError:
        return None


def unit_started(remote: bool = False) -> str | None:
    show = ["systemctl", "show", UNIT, "-p", "ExecMainStartTimestamp", "--value"]
    if remote:
        out = over_ssh(show)
        return out.stdout.decode().strip() or None if out.returncode == 0 else None
    if not UNIT or not shutil.which("systemctl"):
        return None
    out = subprocess.run(show, capture_output=True, text=True)
    if out.returncode != 0:
        return None
    return out.stdout.strip() or None


STALE_NO_PID_S = 3600  # a copy folder with no readable pid is a crashed run only after this


def alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True  # someone else's process: alive, and not ours to touch
    out = subprocess.run(["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True)
    return not out.stdout.strip().startswith("Z")  # an exited, unreaped child is not running


def cmdline(pid: int) -> str:
    out = subprocess.run(["ps", "-ww", "-o", "command=", "-p", str(pid)], capture_output=True, text=True)
    return out.stdout.strip() if out.returncode == 0 else ""


def read_pid(path: str) -> int | None:
    try:
        return int(open(path).read().strip())
    except (OSError, ValueError):
        return None


def group_exists(pgid: int) -> bool:
    """True while any process is left in group `pgid` (unreaped zombies aside)."""
    try:
        os.killpg(pgid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    out = subprocess.run(["ps", "-A", "-o", "pgid=,stat="], capture_output=True, text=True)
    if out.returncode != 0:
        return True
    return any(f[0] == str(pgid) and not f[1].startswith("Z") for f in (l.split() for l in out.stdout.splitlines()) if len(f) == 2)


def kill_group(pgid: int, wait_s: float = 5) -> bool:
    """SIGTERM the whole group, then SIGKILL it while anything in it is left (the leader dying
    first is not enough: a member may ignore SIGTERM). True when the group is gone."""
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(pgid, sig)
        except ProcessLookupError:
            return True
        except PermissionError:
            return False
        end = time.time() + wait_s
        while time.time() < end and group_exists(pgid):
            time.sleep(0.1)
        if not group_exists(pgid):
            return True
    return not group_exists(pgid)


def environ_names(pid: int, folder: str) -> bool:
    """True when process `pid` was started for the copy in `folder` (its VERIFY_TMP, Linux)."""
    try:
        env = open(f"/proc/{pid}/environ", "rb").read().split(b"\0")
    except OSError:
        return False
    return f"VERIFY_TMP={folder}".encode() in env


def sweep(tmp_root: str) -> list[str]:
    """Delete copies of earlier runs that are certainly over. Returns what was removed.

    A folder is swept only when its run's pid is readable and that process is gone (or, with no
    readable pid, when the folder is over an hour old: a run that crashed before writing it).
    Before deleting, the server and Playwright groups that run left are stopped, after checking
    their command lines name that copy, so a password-cleared copy is never left being served.
    """
    removed = []
    for d in glob.glob(os.path.join(tmp_root, PREFIX + "*")):
        try:
            st = os.lstat(d)
        except OSError:
            continue
        if st.st_uid != os.getuid() or not os.path.isdir(d) or os.path.islink(d):
            continue
        pid = read_pid(os.path.join(d, "pid"))
        if pid is None:
            if time.time() - st.st_mtime < STALE_NO_PID_S:
                continue
        elif pid == os.getpid() or alive(pid):
            continue
        # A leader's group is ours only when the leader names this copy; a group that will not
        # die keeps its folder (and the folder is not reported as swept).
        survivors = False
        srv = read_pid(os.path.join(d, "server.pid"))
        if srv and alive(srv) and os.path.join(d, "data") in cmdline(srv):
            survivors |= not kill_group(srv)
        pw = read_pid(os.path.join(d, "playwright.pid"))
        if pw and alive(pw) and "verify/playwright.config.ts" in cmdline(pw) and environ_names(pw, d):
            survivors |= not kill_group(pw)
        if survivors:
            continue
        shutil.rmtree(d, ignore_errors=True)
        removed.append(d)
    return removed


def die_with_parent() -> None:
    """Child setup: its own process group, and (Linux) SIGTERM when this script dies."""
    os.setsid()
    if sys.platform.startswith("linux"):
        import ctypes

        ctypes.CDLL("libc.so.6", use_errno=True).prctl(1, signal.SIGTERM)  # PR_SET_PDEATHSIG


def extract(snapshot: str, data: str, remote: bool = False) -> None:
    os.makedirs(data, mode=0o700)
    if remote:  # a local file beside `data`, in the private folder; deleted once it is extracted
        local = data + ".tar"
        with open(local, "wb") as f:
            out = over_ssh(["cat", snapshot], stdout=f)
        if out.returncode != 0:
            raise RuntimeError(f"could not copy {snapshot} from {SERVER_SSH}: {out.stderr.decode().strip()}")
        snapshot = local
    if os.access(snapshot, os.R_OK):
        with tarfile.open(snapshot) as t:
            try:
                t.extractall(data, filter="data")
            except TypeError:  # Python before 3.12 has no filter; our own snapshots only
                t.extractall(data)
        if remote:
            os.remove(snapshot)
        return
    subprocess.run(["sudo", "-n", "tar", "-xf", snapshot, "-C", data], check=True)
    subprocess.run(["sudo", "-n", "chown", "-R", f"{os.getuid()}:{os.getgid()}", data], check=True)


def free_port() -> int:
    while True:
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            port = s.getsockname()[1]
        if port != LIVE_PORT:
            return port


def request(port: int, method: str, path: str, body: dict | None = None, cookie: str | None = None):
    c = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
    headers = {"Content-Type": "application/json"} if body is not None else {}
    if cookie:
        headers["Cookie"] = cookie
    c.request(method, path, body=json.dumps(body) if body is not None else None, headers=headers)
    r = c.getresponse()
    data = r.read()
    return r.status, dict(r.getheaders()), data


def wait_healthy(port: int, proc: subprocess.Popen, seconds: float) -> bool:
    end = time.time() + seconds
    while time.time() < end:
        if proc.poll() is not None:
            return False
        try:
            if request(port, "GET", "/api/v1/health")[0] == 200:
                return True
        except OSError:
            pass
        time.sleep(0.2)
    return False


def cli_home(port: int, home: str) -> str:
    """Sign in over loopback, mint a Token in the copy, write a config with only `copy`."""
    status, headers, _ = request(port, "GET", "/")
    cookie = next((v.split(";")[0] for k, v in headers.items() if k.lower() == "set-cookie" and "cd_session" in v), None)
    if not cookie:
        raise RuntimeError(f"loopback sign-in gave no session cookie (GET / → {status}); is a password still set?")
    status, _, data = request(port, "POST", "/api/v1/tokens", {"name": "verify-live"}, cookie)
    if status != 201:
        raise RuntimeError(f"minting a Token in the copy failed: {status} {data[:200]!r}")
    token = json.loads(data)["token"]
    os.makedirs(home, mode=0o700)
    with open(os.path.join(home, "config.toml"), "w") as f:
        f.write(f'[profiles.copy]\nserver = "http://127.0.0.1:{port}"\ntoken = "{token}"\n')
    return token


def local_addresses() -> list[str]:
    """Every non-loopback address of this host (`ip -o addr` on Linux, `ifconfig` elsewhere)."""
    addrs = set()
    for cmd in (["ip", "-o", "addr", "show"], ["ifconfig"]):
        if not shutil.which(cmd[0]):
            continue
        out = subprocess.run(cmd, capture_output=True, text=True).stdout
        for word in out.replace("addr:", " ").split():
            ip = word.split("/")[0].split("%")[0]
            try:
                socket.inet_pton(socket.AF_INET6 if ":" in ip else socket.AF_INET, ip)
            except OSError:
                continue
            if not (ip.startswith("127.") or ip == "::1" or ip.startswith("fe80:")):
                addrs.add(ip)
        break
    return sorted(addrs)


def answers_beyond_loopback(port: int) -> list[str]:
    """The host's own non-loopback addresses on which `port` answers (there must be none)."""
    hits = []
    for ip in local_addresses():
        family = socket.AF_INET6 if ":" in ip else socket.AF_INET
        try:
            with socket.socket(family, socket.SOCK_STREAM) as s:
                s.settimeout(2)
                s.connect((ip, port))
                hits.append(ip)
        except OSError:
            pass
    return hits


def live_rejects(token: str) -> bool:
    """True when the live server on its host's 127.0.0.1:7377 answers 401 to the copy's Token."""
    if SERVER_SSH:  # the Token goes over stdin, never on a command line
        config = f'header = "Authorization: Bearer {token}"\n'.encode()
        out = over_ssh(["curl", "-sS", "-o", "/dev/null", "-w", "%{http_code}", "--max-time", "5", "-K", "-",
                        f"http://127.0.0.1:{LIVE_PORT}/api/v1/me"], stdin=config)
        return out.returncode == 0 and out.stdout.strip() == b"401"
    try:
        c = http.client.HTTPConnection("127.0.0.1", LIVE_PORT, timeout=5)
        c.request("GET", "/api/v1/me", headers={"Authorization": f"Bearer {token}"})
        return c.getresponse().status == 401
    except OSError:
        return False


def clean_env(extra: dict[str, str]) -> dict[str, str]:
    env = {k: v for k, v in os.environ.items() if not k.startswith("ANNOTARY_")}
    env.update(extra)
    return env


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--bin", default=os.path.join(REPO, "target/debug/annotary"), help="the build under test")
    ap.add_argument("--snapshot", help="a snapshot tar (default: the live server's newest)")
    ap.add_argument("--no-scenarios", action="store_true", help="set up, check and tear down only")
    ap.add_argument("--tmp", default=tempfile.gettempdir(), help="where the copy goes")
    ap.add_argument("--out", default=os.path.join(REPO, "target/verify-live"), help="screenshots for the PR")
    ap.add_argument("playwright_args", nargs="*", help="passed to playwright test (after --)")
    a = ap.parse_args()

    verdict: dict = {"status": "fail", "problems": []}
    problems: list[str] = verdict["problems"]
    live = a.snapshot is None  # copying the live server's newest: production's checks are required
    remote = live and bool(SERVER_SSH)
    if live and not (LIVE_SNAPSHOT_GLOB and UNIT):
        problems.append("VERIFY_SNAPSHOT_GLOB and VERIFY_UNIT are not set (make verify-live reads them from "
                        "~/.config/annotary/deploy.env; docs/ops-health-check.md, Settings); or pass --snapshot")
        print(json.dumps(verdict))
        return 1
    snapshot = a.snapshot or newest_snapshot(LIVE_SNAPSHOT_GLOB)
    if not snapshot:
        where = f" on {SERVER_SSH}" if SERVER_SSH else " (set SERVER_SSH if it is on another host)"
        problems.append(f"no snapshot: none matches {LIVE_SNAPSHOT_GLOB}{where}, or no sudo; pass --snapshot")
        print(json.dumps(verdict))
        return 1
    if not os.path.exists(a.bin):
        problems.append(f"no build at {a.bin}: run make build")
        print(json.dumps(verdict))
        return 1
    # The live server may write a newer snapshot meanwhile; the one we copy must not change.
    mtime = snapshot_mtime(snapshot, remote)
    before = {"unit_started": unit_started(remote), "snapshot_sha256": snapshot_hash(snapshot, remote)}
    if live and None in before.values():
        problems.append(f"cannot read production's state (unit start, snapshot hash): {before}")
    verdict.update(snapshot=snapshot, snapshot_age_s=int(time.time() - mtime) if mtime else None)
    verdict["swept"] = sweep(a.tmp)

    work = tempfile.mkdtemp(prefix=PREFIX, dir=a.tmp)  # mode 700
    with open(os.path.join(work, "pid"), "w") as f:
        f.write(f"{os.getpid()}\n")
    data, home = os.path.join(work, "data"), os.path.join(work, "cli")
    procs: list[subprocess.Popen] = []  # the server and Playwright, each leading its own group
    token = None

    def teardown(*_):
        for p in reversed(procs):
            for sig in (signal.SIGTERM, signal.SIGKILL):
                try:
                    os.killpg(p.pid, sig)
                except (ProcessLookupError, PermissionError):
                    break
                try:
                    p.wait(10)
                    break
                except subprocess.TimeoutExpired:
                    continue
            kill_group(p.pid, 5)  # whatever else is left in its group (Chromium under Playwright)
        procs.clear()
        shutil.rmtree(work, ignore_errors=True)

    def on_signal(signum, _frame):
        teardown()
        sys.exit(128 + signum)

    signal.signal(signal.SIGINT, on_signal)
    signal.signal(signal.SIGTERM, on_signal)
    try:
        extract(snapshot, data, remote)
        env = clean_env({"HOME": work})
        r = subprocess.run([a.bin, "-q", "--data-dir", data, "password", "--clear"], env=env, capture_output=True, text=True)
        if r.returncode != 0:
            raise RuntimeError(f"password --clear on the copy failed: {r.stderr.strip()}")
        port = free_port()
        verdict["port"] = port
        log = open(os.path.join(work, "serve.log"), "w")
        server = subprocess.Popen(
            [a.bin, "serve", "--data-dir", data, "--port", str(port), "--bind", "127.0.0.1", "--link-base", "none"],
            env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=log, preexec_fn=die_with_parent,
        )
        procs.append(server)
        with open(os.path.join(work, "server.pid"), "w") as f:
            f.write(f"{server.pid}\n")
        if not wait_healthy(port, server, 30):
            raise RuntimeError("the copy's server did not come up: " + open(os.path.join(work, "serve.log")).read()[-500:])
        token = cli_home(port, home)
        cfg = open(os.path.join(home, "config.toml")).read()
        checks = {
            "port_not_live": port != LIVE_PORT,
            "loopback_only": not answers_beyond_loopback(port),
            "cli_home_has_only_the_copy": cfg.count("[profiles.") == 1 and f"127.0.0.1:{port}" in cfg,
        }
        verdict["checks"] = checks
        verdict["addresses_probed"] = len(local_addresses())
        problems += [f"check failed: {k}" for k, ok in checks.items() if not ok]
        # A CLI round trip against the copy, through the private profile.
        r = subprocess.run([a.bin, "-p", "copy", "list", "--json", "id"],
                           env=clean_env({"HOME": work, "ANNOTARY_HOME": home}), capture_output=True, text=True)
        if r.returncode != 0:
            problems.append(f"CLI list on the copy failed: {r.stderr.strip()}")
        else:
            verdict["documents"] = len(json.loads(r.stdout or "[]"))
        if not a.no_scenarios and not problems:
            shutil.rmtree(a.out, ignore_errors=True)
            os.makedirs(a.out, exist_ok=True)
            # HOME is the copy's folder too, so nothing a scenario starts finds the real profiles.
            browsers = os.environ.get("PLAYWRIGHT_BROWSERS_PATH") or os.path.expanduser("~/.cache/ms-playwright")
            pw_env = clean_env({"VERIFY_URL": f"http://127.0.0.1:{port}", "VERIFY_CLI_HOME": home,
                                "VERIFY_BIN": os.path.abspath(a.bin), "VERIFY_OUT": a.out,
                                "VERIFY_TMP": work, "HOME": work, "PLAYWRIGHT_BROWSERS_PATH": browsers})
            report = os.path.join(work, "report.json")  # written by verify/playwright.config.ts
            pw = subprocess.Popen(
                [os.path.join(REPO, "node_modules/.bin/playwright"), "test", "-c", "verify/playwright.config.ts",
                 *a.playwright_args],
                cwd=REPO, env=pw_env, stdin=subprocess.DEVNULL, preexec_fn=die_with_parent,
            )
            procs.append(pw)
            with open(os.path.join(work, "playwright.pid"), "w") as f:
                f.write(f"{pw.pid}\n")
            pw.wait()
            r = pw
            stats = json.load(open(report)).get("stats", {}) if os.path.exists(report) else {}
            verdict["scenarios"] = {"passed": stats.get("expected", 0), "failed": stats.get("unexpected", 0),
                                    "flaky": stats.get("flaky", 0), "skipped": stats.get("skipped", 0)}
            if r.returncode != 0:
                problems.append("a scenario failed (rerun with the scenario name after --; traces are deleted with the copy)")
    except Exception as e:  # noqa: BLE001 - every failure becomes the verdict
        problems.append(str(e))
    finally:
        teardown()
    verdict["deleted"] = not os.path.exists(work)
    if not verdict["deleted"]:
        problems.append(f"the copy at {work} was not deleted")
    # What these prove: the live unit was not restarted, the copied snapshot was not changed, and
    # the copy's credential is refused by the live server (so the private profile cannot write
    # production). They do not prove no one else wrote the live store meanwhile (its owner may).
    after = {"unit_started": unit_started(remote), "snapshot_sha256": snapshot_hash(snapshot, remote)}
    prod = {
        "unit_not_restarted": before["unit_started"] == after["unit_started"],
        "copied_snapshot_unchanged": before["snapshot_sha256"] == after["snapshot_sha256"],
    }
    if live:
        prod["copy_token_rejected_by_live"] = token is not None and live_rejects(token)
    verdict["production"] = prod
    problems += [f"production check failed: {k}" for k, ok in prod.items() if not ok]
    if not problems:
        verdict["status"] = "ok"
    print(json.dumps(verdict))
    return 0 if verdict["status"] == "ok" else 1


if __name__ == "__main__":
    sys.exit(main())
