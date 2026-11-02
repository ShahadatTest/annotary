#!/bin/sh
# Rowan puts the agent's v2 change (the hunk `push` printed) into his file.
set -e
sed 's/^SQLite, one file\.$/SQLite, one file, WAL mode.\nBlobs are content-addressed./' cc-plan.md > cc-plan.md.tmp
mv cc-plan.md.tmp cc-plan.md
grep -c 'WAL mode' cc-plan.md
