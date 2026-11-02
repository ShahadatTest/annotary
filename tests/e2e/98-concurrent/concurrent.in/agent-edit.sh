#!/bin/sh
# @scribe, in its own checkout, adds WAL mode to Storage.
set -e
sed 's/^SQLite, one file\.$/SQLite, one file, WAL mode.\nBlobs are content-addressed./' agent/cc-plan.md > agent/cc-plan.md.tmp
mv agent/cc-plan.md.tmp agent/cc-plan.md
grep -c 'WAL mode' agent/cc-plan.md
