# Annotary: running a server on Linux, health check and playbooks

For whoever runs a server under systemd with `deploy/install-linux.sh`. Contributors do not need any
of this. `deploy/healthcheck.sh` runs on the host as root and prints one JSON verdict. `make
server-health` feeds it to `ssh "$SERVER_SSH" 'sudo -n bash -s'`; the deploy installs a copy at
`/opt/annotary/app/healthcheck.sh` (`sudo bash /opt/annotary/app/healthcheck.sh` there).
`make server-health` and the deploy gate pass on a warning and fail on critical; a scheduled run
reads the script's own exit code (0 healthy, 1 warning, 2 critical). Each problem it reports
names a playbook below. A playbook is a short list of steps; every one ends at the word
**stop**. If a playbook does not fix the problem, stop and report: never improvise further.

The service: the systemd unit `annotary.service` (`deploy/annotary.service`) runs
`/opt/annotary/app/annotary serve --data-dir /srv/annotary/data --port 7377 --bind 127.0.0.1 --url
${ANNOTARY_URL} --link-base ${ANNOTARY_LINK_BASE}` as the user `annotary`, with those two values from
the root-only `/etc/annotary/env`; logs are in the journal (`journalctl -u annotary.service`). It
listens on loopback only, so what the host puts in front of `127.0.0.1:7377` (a Tailscale Service, a
reverse proxy) is how anyone else reaches it, at `$ANNOTARY_URL`.

## Install

On the host, as root, with a release's tag and the URL other devices will use:

```sh
curl -fsSL https://raw.githubusercontent.com/josh-andrews-dev/annotary/main/deploy/install-linux.sh -o install-linux.sh
sudo bash install-linux.sh vX.Y.Z --url https://annotary.example.ts.net
```

It downloads that release's tarball, refuses it unless the SHA-256 matches, and installs the binary,
the unit and the health check; its header has the steps. `--url none` is for a server nothing else
reaches. It writes `/etc/annotary/env` the first time and only adds a key the file lacks, so a later
install never changes what the file says: to change the URL, edit the file and restart the unit.

## Settings

`make deploy`, `make server-health` and `make verify-live` are for a maintainer deploying from a
checkout. Where they deploy is not in the repo: `deploy/with_settings.py` reads `KEY=value` lines from
`~/.config/annotary/deploy.env` (`ANNOTARY_DEPLOY_ENV` names another file; the environment wins). A
target refuses before it does anything when a key it needs is missing, and a key not listed here is
refused as a typo.

| Key | Needed by | What it is |
|---|---|---|
| `SERVER_SSH` | deploy, server-health; verify-live when the snapshots are on another host | The `ssh` target, with passwordless sudo there |
| `DEPLOY_SOURCE` | deploy | `release` installs the published release of HEAD's tag; `build` builds HEAD on the host |
| `ANNOTARY_URL` | deploy; server-health checks it when set | The URL other devices use, or `none` |
| `ANNOTARY_LINK_BASE` | deploy | The link relay's base, or `none` |
| `ANNOTARY_OWNER_HANDLE`, `ANNOTARY_OWNER_NAME` | deploy, optional | The owner first-run setup makes on an empty store |
| `ANNOTARY_TAILSCALE_SERVICE` | deploy, optional | `svc:NAME`: the Tailscale Service to serve to the server |
| `VERIFY_SNAPSHOT_GLOB` | verify-live | The live snapshots, such as `/srv/annotary/data/snapshots/annotary-*.tar` |
| `VERIFY_UNIT` | verify-live | The unit it checks was not restarted, such as `annotary.service` |

With `DEPLOY_SOURCE=release`, tag first: `make deploy` installs the release of the tag on HEAD and
refuses if HEAD has none, so push `vX.Y.Z` and wait for the Release workflow before you deploy.
`DEPLOY_SOURCE=build` is for a fix that cannot wait for one.

## Readings

What the check reads, and the thresholds for warning and critical.

| Check | Reads | Warning | Critical |
|---|---|---|---|
| service | `systemctl is-active` / `is-enabled annotary.service` | not enabled | not active |
| api | `http://127.0.0.1:7377/api/v1/health` | | no answer, not `"ok":true`, `"set_up"` not true; the data dir missing |
| password | the health answer's `password_set` | false: no browser off the host can sign in | |
| public | `ANNOTARY_URL` in `/etc/annotary/env`; the health answer's `public_url`; the settings' `ANNOTARY_URL`; `<public_url>/api/v1/health` | the URL does not answer, or not `"ok":true` | no env file, no `ANNOTARY_URL` in it or an empty one, the server answering with another URL or none, the settings disagreeing with the file |
| disk | free space on the volume holding `/srv/annotary/data` | under 10 GB | under 2 GB |
| snapshot | mtime of the newest `/srv/annotary/data/snapshots/annotary-*.tar` | none, or over 1 hour old | over 24 hours old |

## Never do

- Never edit the unit on the host or anything in `/srv/annotary/data` by hand (P7's manual
  restore, when the owner asks for it, is the one exception), and never delete anything there (the
  store: SQLite and blobs; `snapshots/pre-schema-*.tar`; `store.failed-*`).
- Never touch a Tailscale Serve config other than the Service `ANNOTARY_TAILSCALE_SERVICE` names: other programs on the host may use theirs.
- Never run `purge` as a fix, and never set, reset or print the owner's password.

## Playbooks

### P1: the service is not running

1. `ssh "$SERVER_SSH" 'systemctl status annotary.service --no-pager; sudo journalctl -u annotary.service -n 50 --no-pager'`.
2. If the journal shows a crash since the last deploy, report it with the lines: a rollback is a
   revert pull request, merged and deployed (`make deploy` ships only `origin/main`). A deploy
   whose new binary does not come up already put the previous one back
   (`deploy/install-linux.sh`), with the unit and the health check's copy; the binary the last deploy
   replaced is `/opt/annotary/app/annotary.prev`. A rollback across a schema migration is P7.
3. If it is not enabled, `ssh "$SERVER_SSH" 'sudo systemctl enable annotary.service'`.
4. Otherwise restart it once, `ssh "$SERVER_SSH" 'sudo systemctl restart annotary.service'`,
   then run `make server-health` again.
5. **stop**

### P2: the API does not answer, or the store is not set up

1. `ssh "$SERVER_SSH" 'sudo journalctl -u annotary.service -n 50 --no-pager; sudo ss -ltnp "sport = :7377"'`.
2. If another process holds port 7377, or the store is locked (`store/LOCK` held by a foreground
   `serve`), report which and **stop**.
3. If `set_up` is false, report it: setup runs only from `deploy/install-linux.sh` (a re-run of
   `make deploy`), never by hand under the unit, whose journal would then hold the owner Token.
4. Otherwise restart it once, `ssh "$SERVER_SSH" 'sudo systemctl restart annotary.service'`, then
   run `make server-health` again.
5. **stop**

### P3: the public URL is missing, wrong or does not answer

1. If the verdict is critical, the URL setting itself is at fault:
   `ssh "$SERVER_SSH" 'sudo grep -c ^ANNOTARY_URL= /etc/annotary/env; curl -sS http://127.0.0.1:7377/api/v1/health'`.
   No key, an empty one, or no file: `make deploy` adds it from the settings, and refuses with nothing
   changed if the settings differ from what the store holds; report both values. The file and the server disagree: run
   `sudo systemctl restart annotary.service` once. The file and the settings disagree: report both
   values; the file is the host's and an agent does not change it. **stop**
2. Otherwise the URL does not answer from outside, and what fronts it is at fault. With a reverse
   proxy, report it. **stop** With a Tailscale Service, which needs the host to be a tagged node, the
   Service defined and the host approved (the admin-console steps below):
   `ssh "$SERVER_SSH" 'tailscale status --peers=false --json | grep -A3 "\"Tags\""; sudo tailscale serve status --json'`
   and `curl -sS "$ANNOTARY_URL/api/v1/health"`.
3. No `Tags`: the console steps are not done yet; report it with the steps. **stop**
4. Tagged but the Service is not in the serve status: run the deploy's step again,
   `ssh "$SERVER_SSH" "sudo tailscale serve --service=$ANNOTARY_TAILSCALE_SERVICE --https=443 --bg --yes http://127.0.0.1:7377"`,
   then check again.
5. In the serve status but no answer: the host may be waiting for approval (admin console,
   Services, the Service, Service hosts, Approve) or Tailscale may be logged out; report which.
6. **stop**

The console steps (the tailnet's admin, once; https://login.tailscale.com/admin), for a Service `svc:annotary` and a tag `tag:annotary`:

1. **Access controls**: add `"tag:annotary": ["autogroup:admin"]` to `tagOwners`, and, so the
   host needs no manual approval, `"services": {"svc:annotary": ["tag:annotary"]}` to
   `autoApprovers`. If the policy is not the default allow-all, also grant tailnet members the
   Service: `{"src": ["autogroup:member"], "dst": ["svc:annotary"], "ip": ["443"]}` in
   `grants`. Save.
2. **Machines**: the host, the row's menu, **Edit ACL tags**, add `tag:annotary`, **Save**.
   This makes the host a tagged device: it loses its user identity and its key expiry, and a
   rule that names your user (or `autogroup:self`) as the host's owner no longer matches it.
3. **Services**: **Advertise**, **Define a Service**: name `annotary`, endpoint `tcp:443`,
   **Add service**.
4. Then `make deploy` (or step 4 above) advertises it; without the auto-approver, **Services**,
   `annotary`, **Service hosts**, the host, **Approve**.

### P4: the store's volume is filling up

1. `ssh "$SERVER_SSH" 'sudo du -sh /srv/annotary/data/store /srv/annotary/data/snapshots; df -h /srv'`.
2. Report the sizes. Housekeeping (GC, stale renders) already runs hourly; nothing is deleted by
   hand.
3. **stop**

### P5: the store snapshot is missing or old

The server writes a consistent tar of the store to `/srv/annotary/data/snapshots/` at start
and every 15 minutes (CMD-1), for the host's backup to copy whole; it keeps the newest four.

1. `ssh "$SERVER_SSH" 'sudo ls -la /srv/annotary/data/snapshots; sudo journalctl -u annotary.service --no-pager | grep snapshot | tail -n 20'`.
2. If the journal shows `snapshot failed: ...` (disk full, permissions), report the line: never
   write, move or delete anything in `/srv/annotary/data` by hand.
3. If the service is not running, follow P1 instead.
4. Otherwise restart it once, `ssh "$SERVER_SSH" 'sudo systemctl restart annotary.service'` (a
   snapshot is written right after start), then run `make server-health` again.
5. **stop**

### P6: no local password is set

Until the owner sets one, a browser on another device sees "This server has no local password
yet" and cannot sign in (CMD-17: requests through the Tailscale Service are never trusted as
loopback). The password is the owner's; an agent never sets or prints it.

1. Report it to the owner with these steps: on their own computer, `ssh -N -L 7377:127.0.0.1:7377 "$SERVER_SSH"`;
   open `http://127.0.0.1:7377/settings/server/general` in a browser there (a loopback browser on a server
   with no password is signed in as the owner); set the password; close the tunnel.
   Then `$ANNOTARY_URL/login` takes that password on any device.
2. **stop**

### P7: a rollback across a schema migration

A binary with a newer store schema migrates the store when it starts, and the previous binary
then refuses that store (`SchemaTooNew`), so `annotary.prev` alone cannot come back (CMD-23).
Before such a binary starts, `deploy/install-linux.sh` stops the service and has the previous
binary write `export` to `/srv/annotary/data/snapshots/pre-schema-N.tar` (N = the store's
schema before; an earlier file of that name is kept as `pre-schema-N.<UTC stamp>.tar`). The
server's snapshot rotation never touches these files, and nothing deletes them. If the new
binary does not come up, the installer's rollback moves the migrated `store/` aside as
`store.failed-<UTC stamp>`, untars the export into a fresh `store/` (750, `annotary`) and
starts `annotary.prev` on it. Anything written between the export and the rollback is only in
`store.failed-…`.

1. Read the deploy's output: `restored store/ from …pre-schema-N.tar` and `rolled back to
   annotary X.Y.Z; it is running` mean it is done; run `make server-health` and report the
   deploy's failure with `ssh "$SERVER_SSH" 'sudo journalctl -u annotary.service -n 50 --no-pager'`.
2. If it says `rollback across the migration failed`, or the previous binary did not come up,
   the service is stopped: report it with the output and
   `ssh "$SERVER_SSH" 'sudo ls -la /srv/annotary/data /srv/annotary/data/snapshots'`.
3. Only when the owner asks for a manual restore from `pre-schema-N.tar` (the installed binary must be
   one that runs schema N: `sudo /opt/annotary/app/annotary serve --print-schema-version`
   prints N, or put `annotary.prev` back with `sudo cp -p annotary.prev annotary`
   in `/opt/annotary/app`): on the host,
   `sudo systemctl stop annotary.service`;
   `sudo -u annotary mkdir /srv/annotary/data/.restore`;
   `sudo -u annotary tar -xf /srv/annotary/data/snapshots/pre-schema-N.tar -C /srv/annotary/data/.restore`;
   `sudo mv /srv/annotary/data/store /srv/annotary/data/store.failed-$(date -u +%Y%m%dT%H%M%SZ)`;
   `sudo mv /srv/annotary/data/.restore/store /srv/annotary/data/store && sudo rmdir /srv/annotary/data/.restore`;
   `sudo chmod 750 /srv/annotary/data/store`; `sudo systemctl start annotary.service`;
   then `make server-health`. Never delete `store.failed-…` or the export.
4. **stop**

## Forgotten local password

The owner's step, never an agent's (the password is theirs). On the host: `sudo systemctl stop
annotary.service`, then `sudo -u annotary /opt/annotary/app/annotary password --set
--data-dir /srv/annotary/data` (prompts, no echo), then `sudo systemctl start
annotary.service`. `password` is refused while the service holds the store's `LOCK`, so it
cannot change a running store. Agents use `password --clear` only on a throwaway copy
(`make verify-live`).

## Restore from a snapshot

Into a fresh data dir, never over `/srv/annotary/data`: on the host, `mkdir /tmp/cd-restore
&& sudo tar -xf /srv/annotary/data/snapshots/<newest>.tar -C /tmp/cd-restore`, then
`/opt/annotary/app/annotary serve --data-dir /tmp/cd-restore --port 7390` and check it
(for example `ANNOTARY_SERVER=http://127.0.0.1:7390 annotary list`). A restore of
`/srv` brings the snapshots folder back with it; use the tar in it, not the restored `store/`
files.
