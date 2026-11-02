---
name: annotary
description: Read, reply to and resolve review Threads on Markdown Documents shared via annotary; push new Versions; take Actions (approve, send back); wait for @mentions; send the human a document to review. Use when a task mentions annotary, a Document id like k7q2m9xd, a Thread id like t7, asks you to address review comments, or you wrote Markdown for the human to decide on.
metadata:
  version: 0.0.0
  cli: annotary
---

# annotary

`annotary` is a CLI: humans and agents comment on shared Markdown with equal standing. No default
identity: pass `-p NAME` on **every** command.
- **Your profile** (`annotary profile list`) is your agent Member, never the human's. None? Ask the human.
- **Post as the human** only when they ask ("comment this as me"): their profile, that command only.

## Do this

- **Read through the CLI**: `show DOC`. **Never open `*.annotary`**.
- **Cite Threads by id + quoted text** (`t7 "testable in isolation"`), never by line number.
- **Pull, edit, push.** `pull DOC` writes `NAME.md`; `push NAME.md -m "…"` uploads a Version. Refused with a
  diff (exit 5)? Someone pushed first: add their changes, push again. Never `--discard-head-changes` unasked.
- **Reply, then resolve**: `thread reply DOC t7 --resolve -m "Done in v5: …"`; `@handle` mentions.
- **Links:** give a person only the URL `annotary link DOC [t7]` prints, as a plain Markdown link (in code it
  can't be clicked), never a URL you built, and only if the person can reach it: 127.0.0.1 only with them on that machine. T3 Code: `preview_open` it too.
- **Guard `act` and `thread new` with `--version N`**: refused (exit 5) if head moved.
- **Wait, don't poll.** `wait DOC` blocks until a new Version, Action or @mention; resume: `--cursor SEQ` (`status`).
- **`--json`** to parse (`--json id,anchor,body` narrows, `--json ?` lists).

## A document for the human's review

Markdown you wrote for them to read and decide on: plans (`docs/plans/`), requirements, design notes,
research, handoffs needing sign-off, proposed standards. Not mock boards, PR bodies, TODO.md, tracker items,
docs nobody asked for. The repo file stays the source; push a copy:
1. `D=~/.cache/annotary/review; mkdir -p $D; cp docs/plans/x.md $D/REPO:docs:plans:x.md; push` that file
   (Name `REPO:docs:plans:x`: a Name has no `/`). Give its link, then `wait` it.
2. Per Thread: fix the repo file, copy it over again, push, reply naming the new Version, resolve.
3. Their `approve` Action is their agreement: `pull DOC --version N --out $D --force`, copy it to the repo
   path, commit it in the PR.

## Commands (each also takes `-p NAME`)

```
show DOC                          header + open Threads (start here)
status DOC                        version · action · open · mentions-me · seq
thread list DOC [--all|--status S|--mentions-me|--since 2h|--sort recent|--version N]
thread show DOC t7                full Thread
thread new DOC "L:C-L:C"|--document|--asset REF -m MSG [--version N]
thread reply DOC t7 -m MSG [--resolve]      thread resolve|reopen|edit|delete DOC t7
mentions [--since 2h]             open Threads mentioning you, all Documents
pull DOC [--version N]            push FILE.md [-m MSG] [--strict]
act DOC approve|send-back [--version N]     actions DOC     (`show DOC` lists the Action ids)
wait [DOC] [--on version,action,mention] [--timeout 30m] [--cursor SEQ]
list [NAME] [--mentions-me|--action ID|none|--since 2h]
```

DOC = id, unique Name, or path to a pulled `.md`. Anchor `refined`: text changed (`thread show` has it);
`orphaned`: span gone, still replyable. Exit: 0 ok · 1 usage · 2 not found · 3 no credential (pass `-p`) ·
4 missing right · 5 conflict / refused · 6 unreachable · 7 wait timed out.

`annotary --version` ≠ `metadata.version` above → `annotary skill install`, reload. More: `<command> --help`.
