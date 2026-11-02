# The made-up test corpus

Realistic Documents about Fernway Maps, a fictional small mapping company, written for the test
suite. Nothing here is about a real person or a real company. Use it when a test needs the mix a
real Document has; the tiny per-test Documents under `tests/` remain the right tool for testing one
thing.

| Document | About | Exercises |
|---|---|---|
| `rfc-tile-cache.md` | 20 KB | the main "plan for review": headings, code, tables, footnotes, links |
| `api-reference.md` | 66 KB | the large one: deep headings, long tables, many code blocks |
| `onboarding.md` | 8 KB | nested and task lists, block quotes, relative links to other corpus docs |
| `field-notes.md` | 4 KB | images and video from `fixtures/assets/` (Asset Threads), one missing picture |
| `glossary-multilingual.md` | 7 KB | CJK, Arabic and Hebrew (RTL), emoji, combining marks |
| `math-and-diagrams.md` | 5 KB | math, mermaid fences, raw HTML blocks, scripts that must not run |
| `meeting-notes-*.md` (3) | 2 KB each | short docs for lists, search and the activity feed |
| `release-checklist.md` | 2 KB | task lists, front matter |
| `edge-cases.md` | 4 KB | a very long line, no trailing newline, odd headings, markup that never closes |

## Version histories

`rfc-tile-cache/`, `onboarding/` and `glossary-multilingual/` hold Versions v1 to v4 with realistic
edits (a paragraph rewritten, a section moved, a section deleted, typo fixes), plus `threads.json`
and `expected.json` in the format of `fixtures/anchoring/` (see its README). `v4.md` is a link to
the top-level Document, so a history's last Version is the Document everyone else reads.

They are replayed by `crates/diff/tests/fixtures.rs` (pure) and `crates/annotary/tests/reanchor_api.rs`
(through the server, with placements computed at the edge and by the server).

## What is written by hand

`<name>/v1.md` and the Documents without a history are written by hand. `make.py` generates the rest:
`api-reference.md` from its tables, and each history's later Versions, `threads.json` and
`expected.json` from the edits and expectations it declares. After changing either, run
`python3 fixtures/corpus/make.py` and read the diff.

`expected.json` is declared from the Anchor model (`crates/diff/src/lib.rs`), never computed by the
crate. If a replay disagrees, work out which is wrong; a real anchoring failure is a bug to fix or
report, and its expectation stays as it is.

## Who uses it

- `tests/e2e/94-corpus/`: every Document and one history through the real CLI.
- `tests/web/corpus.spec.ts`: rendering, commenting and RTL selection in a browser.

The people are fictional too: the owner `@rowan` (Rowan Vale), the agents `@scribe` and `@linter`,
and the staff named in the Documents. Addresses are at `example.com` and `example.org`.
