#!/bin/sh
# `token revoke` needs a Token id that only `token list --json` can give — mint one for
# @par-agent, read its id back, revoke it on a TTY and again non-TTY (idempotent, with a note).
set -e
annotary -p rowan token mint --member par-agent --name throwaway >/dev/null 2>&1
id=$(annotary -p rowan token list --member par-agent --json id,name | grep throwaway | sed 's/.*"id":"\([^"]*\)".*/\1/')
ANNOTARY_TTY=1 NO_COLOR=1 annotary -p rowan token revoke "$id" 2>&1 | sed "s/$id/[ID]/"
annotary -p rowan token revoke "$id" 2>&1 | sed "s/$id/[ID]/"
