#!/bin/sh
# Rowan puts @scribe's v4 change (the one `push` just showed him) into his file.
set -e
sed 's/harness plug-ins\.$/harness plug-ins. Sync from local./' roadmap.md > roadmap.md.tmp
mv roadmap.md.tmp roadmap.md
grep -c 'Sync from local' roadmap.md
