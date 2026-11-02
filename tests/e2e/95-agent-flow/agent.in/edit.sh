#!/bin/sh
# "…the agent edits plan.md with its normal file tools…" — item 3 of the order of work gains
# "and its skill file"; nothing under an anchored span changes.
set -e
sed 's/and the skill file\./and its skill file — shipped together./' plan.md > plan.md.tmp
mv plan.md.tmp plan.md
grep -n 'skill file' plan.md
