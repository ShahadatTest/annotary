#!/usr/bin/env python3
"""Regenerates the re-anchoring fixture directories (ticket 07): one directory per row of the
empirical fixture table (`fixtures/anchoring/TABLE.md`, Google
Docs D1–D12 and GitHub G0–G14) plus the ticket-05 additions, each `{v1.md, v2.md, …,
[vN.manifest.json], threads.json, expected.json}`.

The expectations here are *declared*, not computed by the crate: every entry says which text
survives on which line (positions are then found with `str.find`, like the sidecar prototype's
`pos()`), or `orphaned`. The decisions they encode are the Anchor-model ones — sticky inside /
non-sticky at the edges; partial deletion shrinks and insertion grows (refined); type-over,
swallowing replace, moved or gone lines orphan; whitespace-only edits keep anchored; orphaned is
sticky. Divergences from the products are listed in README.md. Run from this directory:
python3 make.py
"""
import json, os, shutil

HERE = os.path.dirname(os.path.abspath(__file__))

def line_of(text, n):
    return text.split("\n")[n - 1]

def find_range(text, line, exact, nth=0):
    """`L:C-L:C` of the nth occurrence of `exact` starting on `line` (may span lines)."""
    lines = text.split("\n")
    start_line_text = lines[line - 1]
    first, rest = exact.split("\n", 1) if "\n" in exact else (exact, None)
    if rest is None:
        col = -1
        for _ in range(nth + 1):
            col = start_line_text.find(exact, col + 1)
            assert col >= 0, f"{exact!r} not on line {line}: {start_line_text!r}"
        return f"{line}:{col}-{line}:{col + len(exact)}"
    col = start_line_text.find(first)
    assert col >= 0 and start_line_text[col:] == first, f"{first!r} must end line {line}"
    parts = exact.split("\n")
    end_line = line + len(parts) - 1
    assert lines[end_line - 1].startswith(parts[-1]), f"{parts[-1]!r} must start line {end_line}"
    for i, mid in enumerate(parts[1:-1], start=1):
        assert lines[line + i - 1] == mid, f"line {line + i} must be {mid!r}"
    return f"{line}:{col}-{end_line}:{len(parts[-1])}"

FIXTURES = {}

def fixture(id, versions, threads, manifests=None):
    """threads: list of dicts {id, subject, version, (line, exact | ref), expected: {version: (line, exact) | 'orphaned' | 'anchored'}}."""
    FIXTURES[id] = (versions, threads, manifests or {})

def write_all():
    for id, (versions, threads, manifests) in FIXTURES.items():
        d = os.path.join(HERE, id)
        if os.path.isdir(d):
            for f in os.listdir(d):
                os.remove(os.path.join(d, f))
        os.makedirs(d, exist_ok=True)
        for n, text in enumerate(versions, start=1):
            with open(os.path.join(d, f"v{n}.md"), "w") as f:
                f.write(text)
        for n, m in manifests.items():
            with open(os.path.join(d, f"v{n}.manifest.json"), "w") as f:
                json.dump(m, f, indent=2)
                f.write("\n")
        tj, ej = [], {}
        for t in threads:
            created = t["version"]
            entry = {"id": t["id"], "subject": t["subject"], "version": created}
            hist = []
            if t["subject"] == "span":
                line, exact = t["at"]
                r = find_range(versions[created - 1], line, exact, t.get("nth", 0))
                entry["range"] = r
                orphaned = False
                prev = (line, exact)
                for n in range(created, len(versions) + 1):
                    exp = t.get("expected", {}).get(n, "same")
                    if exp == "orphaned" or orphaned:
                        assert exp == "orphaned" or exp == "same", f"{id}/{t['id']}: orphaned is sticky"
                        orphaned = True
                        hist.append({"version": n, "status": "orphaned"})
                        continue
                    if exp == "same":
                        exp = prev
                    if exp == "anchored":
                        exp = (line, exact)
                    l, e = exp
                    rr = find_range(versions[n - 1], l, e, t.get("nth", 0))
                    status = "anchored" if e == exact else "refined"
                    h = {"version": n, "status": status, "range": rr, "exact": e}
                    hist.append(h)
                    prev = exp
            elif t["subject"] == "asset":
                entry["ref"] = t["ref"]
                orphaned = False
                prev = t["expected"][created]
                for n in range(created, len(versions) + 1):
                    exp = t["expected"].get(n, "same")
                    if exp == "orphaned" or orphaned:
                        orphaned = True
                        hist.append({"version": n, "status": "orphaned"})
                        continue
                    if exp == "same":
                        exp = prev
                    hist.append({"version": n, "status": "anchored", "line": exp})
                    prev = exp
            else:
                for n in range(created, len(versions) + 1):
                    hist.append({"version": n, "status": "anchored"})
            tj.append(entry)
            ej[t["id"]] = hist
        with open(os.path.join(d, "threads.json"), "w") as f:
            json.dump(tj, f, indent=2)
            f.write("\n")
        with open(os.path.join(d, "expected.json"), "w") as f:
            json.dump(ej, f, indent=2)
            f.write("\n")

# ---------------------------------------------------------------- Google Docs rows (span S = "bb cc dd")

DOC = "# Fixture\n\nIntro paragraph that never changes.\n\nXxx: aa bb cc dd ee ff gg.\n\nOutro paragraph that never changes.\n"
def doc(mid):
    return DOC.replace("Xxx: aa bb cc dd ee ff gg.", mid)
S = {"id": "t1", "subject": "span", "version": 1, "at": (5, "bb cc dd")}

fixture("D1", [DOC, doc("Xxx: aa ee ff gg."), DOC],
        [dict(S, expected={2: "orphaned", 3: "orphaned"})])  # undo (v3 = v1) does not revive: divergence
fixture("D2", [DOC, doc("Xxx: aa cc dd ee ff gg.")], [dict(S, expected={2: (5, "cc dd")})])
fixture("D3", [DOC, doc("Xxx: aa bb dd ee ff gg.")], [dict(S, expected={2: (5, "bb dd")})])
fixture("D4", [DOC, doc("Xxx: aa bb cc ee ff gg.")], [dict(S, expected={2: (5, "bb cc")})])
fixture("D5", [DOC, doc("Xxx: aa bb cc XX dd ee ff gg.")], [dict(S, expected={2: (5, "bb cc XX dd")})])
fixture("D6", [DOC, doc("Xxx: aa ZZ bb cc dd ee ff gg.")], [dict(S, expected={2: "anchored"})])
fixture("D7", [DOC, doc("Xxx: aa bb cc dd YY ee ff gg.")], [dict(S, expected={2: "anchored"})])
fixture("D8", [DOC, doc("Xxx: aa QQ QQ QQ ee ff gg."), DOC],
        [dict(S, expected={2: "orphaned", 3: "orphaned"})])
PASTED = DOC.replace("Outro paragraph that never changes.", "Outro paragraph that never changes. bb cc dd")
fixture("D9a", [DOC, PASTED], [dict(S, expected={2: "anchored"})])  # single range: the copy is not the anchor
fixture("D9b", [DOC, PASTED, PASTED.replace("Xxx: aa bb cc dd ee ff gg.", "Xxx: aa ee ff gg.")],
        [dict(S, expected={2: "anchored", 3: "orphaned"})])  # net cut & paste = moved ⇒ orphaned
fixture("D10", [DOC, doc("Xxx: aa bb cc RR ee ff gg.")], [dict(S, expected={2: (5, "bb cc")})])  # overlap ⇒ refined
fixture("D10-prime", [DOC, doc("Xxx: aa bb RR ee ff gg.")],
        [dict(S, id="t1", at=(5, "cc dd"), expected={2: "orphaned"}),  # the match swallows S ⇒ orphaned (divergence)
         dict(S, id="t2", expected={2: (5, "bb")})])                    # S = "bb cc dd" partly survives ⇒ refined
fixture("D11", [DOC, DOC.replace("Xxx: aa bb cc dd ee ff gg.\n\n", ""), DOC],
        [dict(S, expected={2: "orphaned", 3: "orphaned"})])
D12_V2 = doc("Xxx: aa ZZ bb cc XX dd ee ff gg.")
fixture("D12", [DOC, D12_V2, D12_V2],  # a reload (v3 = v2) shows what was persisted: nothing recomputed, nothing moves
        [dict(S, expected={2: (5, "bb cc XX dd")}),
         dict(S, id="t2", at=(3, "never changes"), expected={2: "anchored"})])

# ---------------------------------------------------------------- GitHub rows (line comments on doc.md, 30 lines)

WORDS = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet",
         "kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo", "sierra", "tango",
         "uniform", "victor", "whiskey", "xray", "yankee", "zulu", "amber", "blue", "coral", "dune"]
def gh_lines():
    return [f"Line {n}: {WORDS[n - 1]} {WORDS[(n * 7) % 30]} {WORDS[(n * 11) % 30]}" for n in range(1, 31)]
def gh(lines):
    return "\n".join(lines) + "\n"
GH = gh(gh_lines())
def whole(lines, a, b):
    """The exact text of whole lines a..b (1-based, inclusive)."""
    return "\n".join(lines[a - 1:b])
L = gh_lines()
A = {"id": "A", "subject": "span", "version": 1, "at": (5, whole(L, 5, 6))}
B = {"id": "B", "subject": "span", "version": 1, "at": (9, whole(L, 9, 10))}
C = {"id": "C", "subject": "span", "version": 1, "at": (13, whole(L, 13, 14))}
D = {"id": "D", "subject": "span", "version": 1, "at": (17, whole(L, 17, 18))}
E = {"id": "E", "subject": "span", "version": 1, "at": (21, whole(L, 21, 21))}
F = {"id": "F", "subject": "span", "version": 1, "at": (24, whole(L, 24, 25))}
G = {"id": "G", "subject": "span", "version": 1, "at": (28, whole(L, 28, 29))}
H = {"id": "H", "subject": "span", "version": 1, "at": (2, whole(L, 2, 2))}
ALL = [A, B, C, D, E, F, G, H]

fixture("G0", [GH], [dict(t) for t in ALL])
fixture("G1", [GH, gh(["Line 0a: inserted at top", "Line 0b: inserted at top"] + L)],
        [dict(t, expected={2: (t["at"][0] + 2, t["at"][1])}) for t in ALL])
L2 = list(L); L2[4] = "Line 5: alpha CHANGED " + WORDS[(5 * 11) % 30]
fixture("G2", [GH, gh(L2)], [dict(A, expected={2: (5, whole(L2, 5, 6))})])  # edited inside ⇒ refined (GitHub: outdated)
L3 = list(L); L3[7] = "Line 8: edited just above B"
fixture("G3", [GH, gh(L3)], [dict(B, expected={2: "anchored"})])
L4 = L[:12] + L[14:]
fixture("G4", [GH, gh(L4)], [dict(C, expected={2: "orphaned"})])
L5 = L[:16] + L[17:]
fixture("G5", [GH, gh(L5)], [dict(D, expected={2: (17, whole(L, 18, 18))})])  # partial deletion shrinks ⇒ refined (GitHub: outdated)
L6 = L[:24] + ["Line 24a: inserted between F's lines"] + L[24:]
fixture("G6", [GH, gh(L6)], [dict(F, expected={2: (24, whole(L6, 24, 26))})])  # grows to 3 lines ⇒ refined
L7 = list(L); L7[27] = "Line 28: " + WORDS[27] + " EDITED " + WORDS[(28 * 11) % 30]
fixture("G7", [GH, gh(L7)], [dict(G, expected={2: (28, whole(L7, 28, 29))})])  # refined (GitHub: outdated)
L8 = list(L); L8[27] = "Twenty-eight was retyped completely here"; L8[28] = "and so was twenty-nine, nothing kept"
fixture("G8", [GH, gh(L8), GH], [dict(G, expected={2: "orphaned", 3: "orphaned"})])  # byte-for-byte revert never revives
fixture("G9", [GH, gh(L8), gh(L8 + ["Line 31: appended at EOF"])],
        [dict(G, expected={2: "orphaned", 3: "orphaned"}), dict(B, expected={2: "anchored", 3: "anchored"}),
         dict(E, expected={2: "anchored", 3: "anchored"})])  # a later unrelated push: outdated stays, the rest untouched
REWRITE = [f"Row {n} - {WORDS[(n * 13) % 30]}ish {WORDS[(n * 17) % 30]}ish {WORDS[(n * 19) % 30]}ish" for n in range(1, 31)]
fixture("G10", [GH, gh(REWRITE)], [dict(t, expected={2: "orphaned"}) for t in ALL])
L11 = L[:10] + L[12:] + L[10:12]  # cut lines 11–12 (block I) and paste them at EOF
I = {"id": "I", "subject": "span", "version": 1, "at": (11, whole(L, 11, 12))}
fixture("G11", [GH, gh(L11)], [dict(I, expected={2: "orphaned"}), dict(D, expected={2: (15, whole(L, 17, 18))})])
L12 = list(L); L12[4] = L[4] + "   "
J = {"id": "J", "subject": "span", "version": 1, "at": (5, whole(L, 5, 5))}
fixture("G12", [GH, gh(L12)], [dict(J, expected={2: "anchored"})])  # whitespace-only ⇒ anchored (GitHub: outdated)
fixture("G13", [GH, GH], [dict(D, expected={2: "anchored"})])  # a rename is a new Version with the same text
L14 = L[:20] + ["Line 20a: inserted between L's lines"] + L[20:]
LL = {"id": "L", "subject": "span", "version": 1, "at": (20, whole(L, 20, 21))}
fixture("G14", [GH, gh(L14)], [dict(LL, expected={2: (20, whole(L14, 20, 22))})])

# ---------------------------------------------------------------- ticket-05 additions

fixture("whitespace-only", [DOC, doc("Xxx:   aa bb cc dd ee ff gg.   ")], [dict(S, expected={2: "anchored"})])
fixture("type-over", [DOC, doc("Xxx: hello world and more.")], [dict(S, expected={2: "orphaned"})])
fixture("replace-swallows-span", [DOC, doc("Xxx: aa RR ee ff gg.")], [dict(S, expected={2: "orphaned"})])
fixture("overlapping-replace", [DOC, doc("Xxx: aa bb cc RR ee ff gg.")], [dict(S, expected={2: (5, "bb cc")})])
fixture("quote-restored", [DOC, doc("Xxx: aa ee ff gg."), DOC, doc("Xxx: aa bb cc dd ee ff gg. and bb cc dd again")],
        [dict(S, expected={2: "orphaned", 3: "orphaned", 4: "orphaned"})])
PARAS = ["# Moved", "First paragraph stays where it is.", "Second paragraph stays too.", "Xxx: aa bb cc dd ee ff gg.",
         "Fourth paragraph stays as well.", "Fifth paragraph closes the document."]
FIVE = "\n\n".join(PARAS) + "\n"
MOVED = "\n\n".join(PARAS[:3] + PARAS[4:] + [PARAS[3]]) + "\n"
fixture("moved-block", [FIVE, MOVED], [dict(S, at=(7, "bb cc dd"), expected={2: "orphaned"})])
fixture("whitespace-survivor", [DOC.replace("aa bb cc dd ee", "aa bb, cc, dd ee"), doc("Xxx: aa xx, yy, zz ee ff gg.")],
        [dict(S, at=(5, "bb, cc, dd"), expected={2: "orphaned"})])  # only ", " survives ⇒ orphaned
MULTI = "# Multi\n\nfirst line of the span\nsecond line in the middle\nthird line ends the span\n\nafter.\n"
MULTI2 = MULTI.replace("second line in the middle", "second line REWORDED in the middle")
fixture("multiline-inner-edit", [MULTI, MULTI2],
        [{"id": "t1", "subject": "span", "version": 1, "at": (3, "line of the span\nsecond line in the middle\nthird line"),
          "expected": {2: (3, "line of the span\nsecond line REWORDED in the middle\nthird line")}}])
BOUND = MULTI.replace("first line of the span", "ABOVE inserted line\nfirst line of the span").replace(
    "third line ends the span", "third line ends the span\nBELOW inserted line")
BOUND = BOUND.replace("first line of the span", "PRE first line of the span").replace("third line ends the span", "third line ends the span POST")
fixture("boundary-insertion", [MULTI, BOUND],
        [{"id": "t1", "subject": "span", "version": 1, "at": (3, "first line of the span\nsecond line in the middle\nthird line ends the span"),
          "expected": {2: (4, "first line of the span\nsecond line in the middle\nthird line ends the span")}}])
fixture("copy-paste", [DOC, PASTED], [dict(S, expected={2: "anchored"})])

IMG = "# Doc\n\n![architecture](img/architecture.png)\n\ntext\n\nmore text\n"
IMG_MOVED = "# Doc\n\ntext\n\nmore text\n\n![architecture](img/architecture.png)\n"
H1, H2 = "1111111111111111111111111111111111111111111111111111111111111111", "2222222222222222222222222222222222222222222222222222222222222222"
fixture("asset-hash-moved", [IMG, IMG_MOVED],
        [{"id": "t1", "subject": "asset", "version": 1, "ref": "img/architecture.png", "expected": {1: 3, 2: 7}}],
        manifests={1: [{"ref": "img/architecture.png", "hash": H1}], 2: [{"ref": "img/architecture.png", "hash": H1}]})
fixture("asset-hash-replaced", [IMG, IMG, IMG],
        [{"id": "t1", "subject": "asset", "version": 1, "ref": "img/architecture.png", "expected": {1: 3, 2: "orphaned", 3: "orphaned"}}],
        manifests={1: [{"ref": "img/architecture.png", "hash": H1}], 2: [{"ref": "img/architecture.png", "hash": H2}],
                   3: [{"ref": "img/architecture.png", "hash": H1}]})  # the old bytes back ⇒ still orphaned (sticky)
fixture("document-level", [GH, gh(REWRITE), "# Rewritten again\n"],
        [{"id": "t1", "subject": "document", "version": 1, "expected": {}}])
DET1 = "# Det\n\none two three four\nfive six seven eight\nnine ten eleven twelve\n\nthirteen fourteen fifteen\n"
DET2 = "# Det\n\nzero one two three four\nfive six SEVEN eight\nnine ten eleven twelve\n\nthirteen fourteen fifteen sixteen\n"
DET3 = "# Det\n\nzero one two three four\n\nnine ten eleven twelve\n\nthirteen fourteen fifteen sixteen\n\nnew paragraph at the end\n"
fixture("determinism", [DET1, DET2, DET3],
        [{"id": "t1", "subject": "span", "version": 1, "at": (3, "two three"), "expected": {2: "anchored", 3: "anchored"}},
         {"id": "t2", "subject": "span", "version": 1, "at": (4, "six seven eight"), "expected": {2: (4, "six SEVEN eight"), 3: "orphaned"}},
         {"id": "t3", "subject": "span", "version": 1, "at": (5, "ten eleven"), "expected": {2: "anchored", 3: "anchored"}},
         {"id": "t4", "subject": "span", "version": 1, "at": (7, "fourteen fifteen"), "expected": {2: "anchored", 3: "anchored"}},
         {"id": "t5", "subject": "document", "version": 1, "expected": {}},
         {"id": "t6", "subject": "span", "version": 2, "at": (3, "zero one"), "expected": {3: "anchored"}}])

write_all()
print(f"wrote {len(FIXTURES)} fixture directories")
