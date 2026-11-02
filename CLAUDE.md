@AGENTS.md

---

# Claude Code mechanics

Everything above is the shared policy. This section is only what needs a Claude Code feature.

- `.claude/settings.json` (committed): the template's permissions, plus SessionStart and
  SessionEnd hooks that run `scripts/trim-target.sh --quiet` (removes `target/` when the
  toolchain changed, nothing was built for 14 days, or it passed 15 GB; it skips while a build or
  a `target/` binary is running, and always exits 0).
- `.claude/rules` → `.agents/rules` and `.claude/skills` → `.agents/skills` are folder links. The
  committed skill source is `crates/annotary/skill/SKILL.md`; `annotary skill install` at the
  root writes an ignored copy to `.agents/skills/annotary/`.
- Sub-agents that change code run with the `Agent` tool's `isolation: "worktree"`; read-only
  research may share the checkout. Start `make build` or `make perf` with `run_in_background`
  in a fresh worktree (a cold release build takes minutes).
