#!/bin/sh
# Rowan edits the pulled roadmap.md while @scribe pushes a Version behind his back.
set -e
sed 's/on one machine\./on one machine — no cloud dependency./' roadmap.md > roadmap.md.tmp
mv roadmap.md.tmp roadmap.md
grep -c 'no cloud dependency' roadmap.md
