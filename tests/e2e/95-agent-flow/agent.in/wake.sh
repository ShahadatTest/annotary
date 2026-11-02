#!/bin/sh
# The agent's `wait` parks on `plan`; the owner approves v5 from a second profile; the wait
# wakes and prints the Action (TTY shape — the `wait: watching` note is the sync point, as in
# crates/annotary/tests/cli_wait.rs). Then the same event read back from the cursor `status`
# gave before the Action — the non-TTY line and the `--json` record an agent would parse.
set -e
seq=$(annotary -p agent status plan --json seq | tr -dc 0-9)
ANNOTARY_TTY=1 annotary -p agent wait plan --on action,mention --timeout 30s --no-color >wake.out 2>wake.err &
pid=$!
until grep -q 'wait: watching' wake.err 2>/dev/null; do sleep 0.05; done
annotary -p rowan act plan approve >/dev/null
wait "$pid"
cat wake.out
annotary -p agent wait plan --on action,mention --cursor "$seq"
annotary -p agent wait plan --on action,mention --cursor "$seq" --json
