#!/bin/sh
# A profile on the harness server whose Vault is e2e-drafts — the golden cannot spell the
# server's URL, so this reads it from the config (the first `server` that is not profile `down`'s).
set -e
url=$(sed -n 's/^server = "\(http:\/\/127\.0\.0\.1:[0-9]*\)"$/\1/p' "$HOME/.annotary/config.toml" | grep -v ':1$' | head -n 1)
annotary -p rowan profile add e2e --server "$url" --token ant_e2e --vault e2e-drafts
