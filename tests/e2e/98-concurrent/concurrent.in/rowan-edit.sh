#!/bin/sh
# Rowan, in his checkout (still on v1), fills in Later.
set -e
sed 's/^Nothing yet\.$/Search, later./' cc-plan.md > cc-plan.md.tmp
mv cc-plan.md.tmp cc-plan.md
grep -c 'Search, later' cc-plan.md
