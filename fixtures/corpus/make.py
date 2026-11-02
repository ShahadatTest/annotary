#!/usr/bin/env python3
"""Regenerates the generated part of the made-up test corpus (see README.md). Run from anywhere:
python3 fixtures/corpus/make.py

- `api-reference.md`, the large Document, from the tables below.
- For each of the three Documents with a Version history: `<name>/v2.md` to `v3.md` and
  `<name>.md` (= v4, which `<name>/v4.md` links to) from the hand-written `<name>/v1.md` and the
  edits declared here, plus `threads.json` and `expected.json` in the anchoring fixtures' format
  (`fixtures/anchoring/README.md`).

The expectations are *declared* from the Anchor model, never computed by the crate: each Thread
says which text it should sit on at each Version, or `ORPHANED`. Lines and columns are then found
in the Version's text. When a replay disagrees, the question is whether the engine or the
declaration is wrong; never paste the engine's answer in here without knowing which.
"""
import json
import os

HERE = os.path.dirname(os.path.abspath(__file__))
ORPHANED = "orphaned"


def replace_once(text, old, new):
    assert text.count(old) == 1, f"{old!r} occurs {text.count(old)} times"
    return text.replace(old, new)


def section(text, heading):
    """The whole of a section, from its heading line to the next heading of the same or a higher level."""
    level = len(heading) - len(heading.lstrip("#"))
    start = text.index("\n" + heading + "\n") + 1
    lines = text[start:].split("\n")
    fenced = False
    length = len(lines[0]) + 1
    for line in lines[1:]:
        if line.startswith("```"):
            fenced = not fenced
        hashes = len(line) - len(line.lstrip("#"))
        if not fenced and 0 < hashes <= level and line[hashes:hashes + 1] == " ":
            break
        length += len(line) + 1
    return text[start:start + length]


def delete_section(text, heading):
    return replace_once(text, section(text, heading), "")


def move_section_before(text, heading, before):
    block = section(text, heading)
    text = replace_once(text, block, "")
    return replace_once(text, "\n" + before + "\n", "\n" + block + before + "\n")


def move_section_to_end(text, heading):
    block = section(text, heading)
    return replace_once(text, block, "").rstrip("\n") + "\n\n" + block


def range_of(text, exact):
    """`L:C-L:C` of the one occurrence of `exact`; columns are code points, like the Sidecar's."""
    assert text.count(exact) == 1, f"{exact!r} occurs {text.count(exact)} times"
    before = text[: text.index(exact)]
    line = before.count("\n") + 1
    col = len(before) - (before.rfind("\n") + 1)
    parts = exact.split("\n")
    if len(parts) == 1:
        return f"{line}:{col}-{line}:{col + len(exact)}"
    return f"{line}:{col}-{line + len(parts) - 1}:{len(parts[-1])}"


def span(id, version, exact, **expected_at):
    """A span Thread created on `version` over `exact`. `expected_at` maps `v3=` to the text it sits
    on from that Version on, or to ORPHANED; the text carries over until the next entry."""
    expected = {int(k[1:]): v for k, v in expected_at.items()}
    return {"id": id, "subject": "span", "version": version, "exact": exact, "expected": expected}


def document_thread(id, version):
    return {"id": id, "subject": "document", "version": version}


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def write_history(name, versions, threads):
    d = os.path.join(HERE, name)
    write(os.path.join(HERE, f"{name}.md"), versions[3])
    for n in (2, 3):
        write(os.path.join(d, f"v{n}.md"), versions[n - 1])
    link = os.path.join(d, "v4.md")
    if os.path.lexists(link):
        os.remove(link)
    os.symlink(f"../{name}.md", link)
    tj, ej = [], {}
    for t in threads:
        created = t["version"]
        entry = {"id": t["id"], "subject": t["subject"], "version": created}
        hist = []
        if t["subject"] == "document":
            hist = [{"version": n, "status": "anchored"} for n in range(created, len(versions) + 1)]
        else:
            entry["range"] = range_of(versions[created - 1], t["exact"])
            current = t["exact"]
            for n in range(created, len(versions) + 1):
                current = t["expected"].get(n, current)
                if current == ORPHANED:
                    hist.append({"version": n, "status": "orphaned"})
                    continue
                status = "anchored" if current == t["exact"] else "refined"
                hist.append(
                    {"version": n, "status": status, "range": range_of(versions[n - 1], current), "exact": current}
                )
        tj.append(entry)
        ej[t["id"]] = hist
    write(os.path.join(d, "threads.json"), json.dumps(tj, indent=2, ensure_ascii=False) + "\n")
    write(os.path.join(d, "expected.json"), json.dumps(ej, indent=2, ensure_ascii=False) + "\n")


def read_v1(name):
    with open(os.path.join(HERE, name, "v1.md")) as f:
        return f.read()


# ---------------------------------------------------------------- rfc-tile-cache


def rfc():
    v1 = read_v1("rfc-tile-cache")
    # v2, addressing review: two typos, a paragraph rewritten, a table row and a question added.
    v2 = replace_once(v1, "clients recieve stale tiles", "clients receive stale tiles")
    v2 = replace_once(
        v2,
        "The cost side is less dramatic but steady. Redis holds about 380 GB of tiles at any moment, most of\n"
        "them never read twice, and memory is the most expensive thing we buy. The object store we already\n"
        "pay for holds the rendered archive at a twentieth of the price per gigabyte.\n",
        "Spending tells a similar story. Our Redis cluster keeps 380 GB resident, largely single-use tiles,\n"
        "at a rate per gigabyte no other line on our bill comes close to. Object storage costs one twentieth\n"
        "as much, and already holds every rendered archive.\n",
    )
    v2 = replace_once(
        v2,
        "| INC-224 | 2 February |",
        "| INC-221 | 20 January | expired certificate on the poller | 12,900 | 18 |\n| INC-224 | 2 February |",
    )
    v2 = replace_once(
        v2,
        "3. Who owns the warm job after the rollout: the tile team, or whoever runs data builds?\n",
        "3. Who owns the warm job after the rollout: the tile team, or whoever runs data builds?\n"
        "4. Should a customer be able to see which tier served a tile? An `X-Fernway-Cache` header would be\n"
        "   cheap to add, and support would love it.\n",
    )
    # v3: the invalidation section moves to the end of Rollout; small edits on two lines.
    v3 = move_section_before(v2, "### Cache invalidation", "## Cost")
    v3 = replace_once(v3, "2. The hiking clubs, who mostly", "2. The walking and hiking clubs, who mostly")
    v3 = replace_once(v3, "header would be\n   cheap to add", "header would be\n   very cheap to add")
    # v4: an update note at the top, Option C dropped, a sentence reworded, a clause deleted.
    v4 = replace_once(
        v3,
        "**Decision wanted by:** the end of the spring planning cycle.",
        "**Update, 16 March:** the failure drills have run, and their results are in the Monday notes.\n\n"
        "**Decision wanted by:** the end of the spring planning cycle.",
    )
    v4 = delete_section(v4, "### Option C: pre-render everything")
    v4 = replace_once(
        v4,
        "We ship this behind a per-customer flag and move customers in the order that hurts least if it goes\n",
        "We ship this behind a per-customer flag, moving customers in whichever order would hurt least if it goes\n",
    )
    v4 = replace_once(
        v4, "then the object store, and only then the renderer. The renderer\n", "then the object store. The renderer\n"
    )
    threads = [
        document_thread("t1", 1),
        span("t2", 1, "versioned key, so publishing a style is cheap"),  # shifts only
        span("t3", 1, "memory is the most expensive thing we buy", v2=ORPHANED),  # type-over
        span("t4", 1, "clients recieve stale tiles", v2="clients receive stale tiles"),  # a typo fixed inside
        span("t5", 1, "bulk re-render of contours"),  # a table row added above
        span("t6", 1, "it refuses to run without one", v3=ORPHANED),  # its section moved
        span("t7", 1, "let Some((_, evicted)) = self.entries.pop_lru()"),  # inside a code block
        span("t8", 1, "takes around nine days on the current fleet", v4=ORPHANED),  # its section deleted
        span(
            "t9",
            1,
            "checks memory first, then the object store, and only then the renderer. The renderer\n"
            "writes through both tiers",
            v4="checks memory first, then the object store. The renderer\nwrites through both tiers",
        ),  # two lines, a clause deleted inside
        span("t10", 1, "sampled by a hash of the tile coordinates"),  # a footnote definition
        span("t11", 1, "Request path"),  # a heading
        span("t12", 1, "who mostly read at weekends"),  # the column moves when v3 edits the line before it
        span("t13", 1, "Serving yesterday's style"),  # in a section the move jumps over
        span(
            "t14", 1, "in the order that hurts least", v4="in whichever order would hurt least"
        ),  # reworded, words survive at both ends
        span(
            "t15",
            2,
            "An `X-Fernway-Cache` header would be\n   cheap to add",
            v3="An `X-Fernway-Cache` header would be\n   very cheap to add",
        ),  # created on v2, grows on v3
    ]
    write_history("rfc-tile-cache", [v1, v2, v3, v4], threads)


# ---------------------------------------------------------------- onboarding


def onboarding():
    v1 = read_v1("onboarding")
    # v2: tasks ticked, a nested item added, a nested list re-indented, one chat channel renamed.
    v2 = replace_once(v1, "- [ ] Payroll forms returned to Ana", "- [x] Payroll forms returned to Ana")
    v2 = replace_once(
        v2,
        "  - [ ] The on-call tool, read-only until your second month\n",
        "  - [ ] The on-call tool, read-only until your second month\n"
        "  - [ ] The survey-data drive, for reading field notes\n",
    )
    v2 = replace_once(
        v2,
        "     - Read its `README` first, then the [cache RFC](rfc-tile-cache.md).\n"
        "     - Its test suite takes four minutes and needs no network.\n",
        "       - Read its `README` first, then the [cache RFC](rfc-tile-cache.md).\n"
        "       - Its test suite takes four minutes and needs no network.\n",
    )
    v2 = replace_once(v2, "| `#styles` |", "| `#cartography` |")
    # v3: on-call moves to the end; a link's target is updated.
    v3 = move_section_before(v2, "## Being on call", "Welcome aboard. We are glad you are here.")
    v3 = replace_once(v3, "meeting-notes-2026-03-02.md", "meeting-notes-2026-03-16.md")
    # v4: a section deleted, a clause typed over.
    v4 = delete_section(v3, "### Watch it land")
    v4 = replace_once(
        v4,
        "The company buys any book you want, and pays for one field trip",
        "Books are on us, whatever the subject, and we pay for one field trip",
    )
    threads = [
        document_thread("t1", 1),
        span("t2", 1, "Payroll forms returned"),  # the checkbox before it changes
        span("t3", 1, "Its test suite takes four minutes"),  # whitespace-only: the column moves
        span("t4", 1, "take the next morning off", v3=ORPHANED),  # its section moved
        span("t5", 1, "a map is a promise"),  # in a block quote, stable
        span(
            "t6",
            1,
            "[the meeting notes](meeting-notes-2026-03-02.md) from any recent Monday",
            v3="[the meeting notes](meeting-notes-2026-03-16.md) from any recent Monday",
        ),  # a word inside the link's target changes
        span("t7", 1, "Deploys take about twelve minutes", v4=ORPHANED),  # its section deleted
        span("t8", 1, "The company buys any book you want", v4=ORPHANED),  # typed over
        span("t9", 1, "Styles, cartography, fonts"),  # a table row; another cell of it changes
        span("t10", 1, "Render your first tile locally"),  # in a nested ordered list
        span("t11", 1, "fernway-tiles render --style outdoor"),  # a code block inside a list
        span("t12", 2, "The survey-data drive"),  # created on v2
    ]
    write_history("onboarding", [v1, v2, v3, v4], threads)


# ---------------------------------------------------------------- glossary-multilingual


def glossary():
    v1 = read_v1("glossary-multilingual")
    # v2: a word inserted in a Japanese sentence, a vowel mark dropped in Arabic, a table row added.
    v2 = replace_once(v1, "地図の品質にとても厳しいです", "地図の品質と更新にとても厳しいです")
    v2 = replace_once(v2, "شركات العبّارات في الخليج", "شركات العبارات في الخليج")
    v2 = replace_once(
        v2,
        "| contour | 等高線 |",
        "| ferry | フェリー | عبّارة | מעבורת | a customer, usually |\n| contour | 等高線 |",
    )
    v2 = replace_once(
        v2,
        "タイルの再描画は夜間に行い、昼間の利用者に影響を与えないようにします。\n",
        "タイルの再描画は夜間に行い、昼間の利用者に影響を与えないようにします。\n\n"
        "北海道の冬季閉鎖の情報も毎月確認します。\n",
    )
    # v3: a Hebrew word inserted before a span, another clause typed over, the emoji section moved.
    v3 = replace_once(v2, "מועדון הטיולים בחיפה", "מועדון הטיולים הוותיק בחיפה")
    v3 = replace_once(v3, "אנחנו מעדכנים את הנתונים תוך יום אחד", "הצוות משנה אותם בתוך יממה")
    v3 = move_section_to_end(v3, "## Emoji in the wild")
    # v4: the Greek section deleted, a number changed beside a span.
    v4 = delete_section(v3, "## Greek, with combining marks")
    v4 = replace_once(v4, "ואורכו 12.5 ק״מ", "ואורכו 12.8 ק״מ")
    threads = [
        document_thread("t1", 1),
        span("t2", 1, "Fernway glossary 🗺️"),  # an emoji in a heading: stable
        span(
            "t3", 1, "地図の品質にとても厳しい", v2="地図の品質と更新にとても厳しい"
        ),  # inserted inside, one token per ideograph
        span(
            "t4", 1, "تعتمد شركات العبّارات في الخليج", v2="تعتمد شركات العبارات في الخليج"
        ),  # one RTL word changed inside
        span("t5", 1, "המפות שלנו"),  # RTL: a word inserted before it moves the column
        span("t6", 1, "מעדכנים את הנתונים", v3=ORPHANED),  # RTL: typed over
        span("t7", 1, "👩‍👩‍👧‍👦 Family Walk", v3=ORPHANED),  # a joined emoji, its section moved
        span("t8", 1, "Ha\u0300 No\u0323\u0302i", v4=ORPHANED),  # combining marks, its section deleted
        span("t9", 1, "خط كنتور"),  # a table row added above
        span("t10", 1, "مَكْتَبَةُ الْخَرَائِطِ"),  # vocalised Arabic: stable
        span("t11", 1, "דרך Mount Carmel ואורכו"),  # mixed direction, the number after it changes
        span("t12", 1, "リビジョン"),  # the move jumps over it
        span("t13", 2, "冬季閉鎖の情報"),  # created on v2
    ]
    write_history("glossary-multilingual", [v1, v2, v3, v4], threads)


# ---------------------------------------------------------------- api-reference

COMMON_ERRORS = [
    ("400", "bad_request", "The request could not be parsed.", "Check the parameters against this page."),
    ("401", "invalid_key", "The API key is missing, unknown or revoked.", "Send a live key in `Authorization`."),
    ("403", "key_restricted", "The key is valid but not allowed from this origin.", "Add the origin to the key."),
    ("404", "not_found", "Nothing exists at this path.", "Check the identifier."),
    ("409", "conflict", "The resource changed since you read it.", "Read it again and retry."),
    ("422", "invalid_field", "A field failed validation; `field` names it.", "Correct the field."),
    ("429", "rate_limited", "The key is over its limit.", "Wait for the time in `Retry-After`."),
    ("500", "internal", "Something failed on our side.", "Retry with backoff; quote `request_id`."),
    ("503", "overloaded", "The render queue is full.", "Wait for the time in `Retry-After`."),
]

# (group, intro, [(method, path, summary, [path params], [query params], [response fields])])
P = lambda name, kind, desc: (name, kind, desc)  # noqa: E731

STYLE = P("style", "string", "A style identifier, such as `outdoor`, `nautical` or `atlas-print`.")
Z = P("z", "integer", "The zoom level, from 0 to 18.")
X = P("x", "integer", "The column, counted from the west, from 0 to 2^z - 1.")
Y = P("y", "integer", "The row, counted from the north, from 0 to 2^z - 1.")
LANG = P("lang", "string", "The label language as a BCP 47 tag. Defaults to the customer's setting.")
LIMIT = P("limit", "integer", "How many items to return, from 1 to 200. Defaults to 50.")
CURSOR = P("cursor", "string", "An opaque position from an earlier response's `next_cursor`.")
ID = lambda what: P("id", "string", f"The {what}'s identifier.")  # noqa: E731
SINCE = P("since", "timestamp", "Only items changed after this time, in RFC 3339 form.")
BBOX = P("bbox", "string", "West, south, east and north in degrees, comma-separated.")
F = lambda name, kind, desc: (name, kind, desc)  # noqa: E731

GROUPS = [
    (
        "Tiles",
        "Tiles are the product. The tile URL is stable by contract: customers hard-code it into apps "
        "that may not be updated for years, so nothing in this section changes without a new version prefix.",
        [
            ("GET", "/v2/tiles/{style}/{z}/{x}/{y}.png", "Fetch a raster tile.", [STYLE, Z, X, Y], [LANG, P("scale", "integer", "1 or 2, for high-density screens. Defaults to 1.")], [F("(body)", "image/png", "The tile, 256 by 256 logical pixels.")]),
            ("GET", "/v2/tiles/{style}/{z}/{x}/{y}.mvt", "Fetch a vector tile.", [STYLE, Z, X, Y], [LANG], [F("(body)", "application/vnd.mapbox-vector-tile", "The tile, gzip-encoded.")]),
            ("GET", "/v2/tiles/{style}/tilejson.json", "Describe a style's tiles.", [STYLE], [LANG], [F("tiles", "array", "URL templates."), F("minzoom", "integer", "The lowest zoom served."), F("maxzoom", "integer", "The highest zoom served."), F("attribution", "string", "HTML the customer must display.")]),
            ("GET", "/v2/static/{style}", "Render a static map image.", [STYLE], [BBOX, P("width", "integer", "Pixels, from 64 to 2048."), P("height", "integer", "Pixels, from 64 to 2048."), P("markers", "string", "Up to 50 points as `lon,lat` pairs, separated by `;`.")], [F("(body)", "image/png", "The picture, with attribution drawn in.")]),
            ("POST", "/v2/tiles/purge", "Purge tiles from every cache tier.", [], [P("reason", "string", "Why, for the audit log. Required.")], [F("tiles", "integer", "How many tiles were removed."), F("audit_id", "string", "The audit line's identifier.")]),
        ],
    ),
    (
        "Styles",
        "A style is a set of drawing rules with a revision counter. Publishing bumps the counter; rolling "
        "back moves it down. Old revisions stay available for 30 days.",
        [
            ("GET", "/v2/styles", "List the styles a key can use.", [], [LIMIT, CURSOR], [F("styles", "array", "Style summaries."), F("next_cursor", "string", "Present when there is more.")]),
            ("GET", "/v2/styles/{style}", "Read a style.", [STYLE], [P("rev", "integer", "A specific revision. Defaults to the active one.")], [F("id", "string", "The identifier."), F("rev", "integer", "The revision."), F("published_at", "timestamp", "When it went live."), F("published_by", "string", "Who published it."), F("layers", "array", "The drawing rules.")]),
            ("PUT", "/v2/styles/{style}", "Create or replace a draft.", [STYLE], [], [F("id", "string", "The identifier."), F("draft_rev", "integer", "The new draft's number."), F("warnings", "array", "Lint findings; a draft can have them.")]),
            ("POST", "/v2/styles/{style}/publish", "Publish the current draft.", [STYLE], [P("region", "string", "One region, or all of them when left out.")], [F("rev", "integer", "The revision now active."), F("previous_rev", "integer", "What it replaced.")]),
            ("POST", "/v2/styles/{style}/rollback", "Make an earlier revision active.", [STYLE], [P("to_rev", "integer", "The revision. Defaults to the one before.")], [F("rev", "integer", "The revision now active.")]),
            ("GET", "/v2/styles/{style}/sprites/{name}.png", "Fetch a sprite sheet.", [STYLE, P("name", "string", "`sprite` or `sprite@2x`.")], [], [F("(body)", "image/png", "The sheet.")]),
            ("GET", "/v2/styles/{style}/revisions", "List revisions, newest first.", [STYLE], [LIMIT, CURSOR], [F("revisions", "array", "Each with `rev`, `published_at` and `published_by`.")]),
        ],
    ),
    (
        "Datasets",
        "A dataset is the output of one nightly data build. Customers cannot write to the base datasets, but "
        "can upload overlays, such as a club's own trail network.",
        [
            ("GET", "/v2/datasets", "List datasets.", [], [LIMIT, CURSOR, SINCE], [F("datasets", "array", "Dataset summaries."), F("next_cursor", "string", "Present when there is more.")]),
            ("GET", "/v2/datasets/{id}", "Read a dataset.", [ID("dataset")], [], [F("id", "string", "The identifier."), F("build_date", "date", "When it was built."), F("sources", "array", "Attribution for each source."), F("feature_count", "integer", "How many features.")]),
            ("POST", "/v2/overlays", "Upload an overlay as GeoJSON.", [], [P("name", "string", "A display name.")], [F("id", "string", "The overlay's identifier."), F("features", "integer", "How many were accepted."), F("rejected", "array", "Features refused, with reasons.")]),
            ("GET", "/v2/overlays/{id}", "Read an overlay.", [ID("overlay")], [], [F("id", "string", "The identifier."), F("name", "string", "The display name."), F("features", "integer", "How many.")]),
            ("DELETE", "/v2/overlays/{id}", "Delete an overlay.", [ID("overlay")], [], [F("deleted", "boolean", "Always true.")]),
            ("GET", "/v2/datasets/{id}/changes", "List what changed since the previous build.", [ID("dataset")], [BBOX, LIMIT, CURSOR], [F("changes", "array", "Each with `kind`, `feature` and `geometry`."), F("next_cursor", "string", "Present when there is more.")]),
        ],
    ),
    (
        "Geocoding",
        "Search for places by name and turn coordinates back into names. Results are ranked by distance from "
        "`near` when it is given, and by prominence otherwise.",
        [
            ("GET", "/v2/geocode", "Search for a place by name.", [], [P("q", "string", "The search text, in any script. Required."), P("near", "string", "`lon,lat` to rank around."), LANG, LIMIT], [F("results", "array", "Places, best first."), F("results[].name", "string", "The display name."), F("results[].lon", "number", "Longitude."), F("results[].lat", "number", "Latitude."), F("results[].kind", "string", "`harbour`, `summit`, `trail`, `town` and so on.")]),
            ("GET", "/v2/reverse", "Name the place at a coordinate.", [], [P("lon", "number", "Longitude. Required."), P("lat", "number", "Latitude. Required."), LANG], [F("name", "string", "The display name."), F("distance_m", "number", "How far away it is.")]),
            ("GET", "/v2/elevation", "Read the height at a coordinate.", [], [P("points", "string", "Up to 200 `lon,lat` pairs separated by `;`. Required.")], [F("heights", "array", "Metres above mean sea level, in order.")]),
            ("POST", "/v2/route/profile", "Return a trail's height profile.", [], [P("sample_m", "integer", "Metres between samples. Defaults to 25.")], [F("distance_m", "number", "The total length."), F("ascent_m", "number", "The total climb."), F("samples", "array", "`[distance, height]` pairs.")]),
        ],
    ),
    (
        "Keys",
        "API keys belong to a customer. A `pk_` key is public and restricted by origin; an `sk_` key is secret "
        "and must stay on a server. Only a secret key can manage keys.",
        [
            ("GET", "/v2/keys", "List a customer's keys.", [], [LIMIT, CURSOR], [F("keys", "array", "Keys, with the secret part hidden."), F("next_cursor", "string", "Present when there is more.")]),
            ("POST", "/v2/keys", "Create a key.", [], [P("kind", "string", "`public` or `secret`."), P("name", "string", "A label.")], [F("id", "string", "The identifier."), F("key", "string", "The key itself, shown this once."), F("origins", "array", "Allowed origins; empty means none.")]),
            ("PATCH", "/v2/keys/{id}", "Change a key's name or origins.", [ID("key")], [], [F("id", "string", "The identifier."), F("name", "string", "The label."), F("origins", "array", "Allowed origins.")]),
            ("DELETE", "/v2/keys/{id}", "Revoke a key at once.", [ID("key")], [], [F("revoked_at", "timestamp", "When it stopped working.")]),
            ("POST", "/v2/keys/{id}/rotate", "Replace a key, keeping the old one alive for a while.", [ID("key")], [P("overlap_hours", "integer", "How long both work, up to 168.")], [F("key", "string", "The new key, shown this once."), F("old_key_expires_at", "timestamp", "When the old one stops.")]),
        ],
    ),
    (
        "Usage",
        "Usage is counted per key and per day, in UTC. A day's figures settle about two hours after midnight, so "
        "today's numbers are estimates.",
        [
            ("GET", "/v2/usage", "Read usage per day.", [], [P("from", "date", "The first day."), P("to", "date", "The last day, inclusive."), P("key", "string", "Only this key.")], [F("days", "array", "Each with `date`, `tiles`, `static` and `geocode`."), F("settled_until", "date", "The last settled day.")]),
            ("GET", "/v2/usage/top-tiles", "List the most requested tiles.", [], [P("days", "integer", "How far back, up to 30."), LIMIT], [F("tiles", "array", "Each with `z`, `x`, `y` and a count above the privacy threshold.")]),
            ("GET", "/v2/usage/limits", "Read the key's limits.", [], [], [F("requests_per_minute", "integer", "The burst limit."), F("tiles_per_month", "integer", "The contract's allowance."), F("used_this_month", "integer", "So far.")]),
        ],
    ),
    (
        "Webhooks",
        "A webhook tells a customer's server that something changed. Deliveries are signed, retried for a day "
        "with backoff, and may arrive more than once, so a receiver should use `event_id` to ignore repeats.",
        [
            ("GET", "/v2/webhooks", "List webhooks.", [], [LIMIT, CURSOR], [F("webhooks", "array", "Each with `id`, `url` and `events`."), F("next_cursor", "string", "Present when there is more.")]),
            ("POST", "/v2/webhooks", "Create a webhook.", [], [P("url", "string", "An HTTPS address."), P("events", "array", "`style.published`, `dataset.built` or `overlay.ready`.")], [F("id", "string", "The identifier."), F("secret", "string", "The signing secret, shown this once.")]),
            ("DELETE", "/v2/webhooks/{id}", "Delete a webhook.", [ID("webhook")], [], [F("deleted", "boolean", "Always true.")]),
            ("POST", "/v2/webhooks/{id}/test", "Send a test delivery now.", [ID("webhook")], [], [F("delivered", "boolean", "Whether the server answered 2xx."), F("status", "integer", "What it answered.")]),
            ("GET", "/v2/webhooks/{id}/deliveries", "List recent deliveries.", [ID("webhook")], [LIMIT, CURSOR], [F("deliveries", "array", "Each with `event_id`, `status` and `attempts`.")]),
        ],
    ),
    (
        "Print jobs",
        "A print job turns a map area into a press-ready PDF at a fixed scale. Jobs run in the background and can "
        "take minutes, so the API hands back an identifier and a webhook or a poll says when the file is ready.",
        [
            ("POST", "/v2/print/jobs", "Start a print job.", [], [BBOX, P("style", "string", "The style, usually `atlas-print`."), P("scale", "integer", "The denominator, such as 25000."), P("paper", "string", "`A0` to `A4`."), P("bleed_mm", "integer", "Bleed in millimetres. Defaults to 3.")], [F("id", "string", "The job's identifier."), F("state", "string", "`queued`, `running`, `done` or `failed`."), F("estimated_seconds", "integer", "A rough guess.")]),
            ("GET", "/v2/print/jobs", "List print jobs.", [], [LIMIT, CURSOR, SINCE], [F("jobs", "array", "Job summaries."), F("next_cursor", "string", "Present when there is more.")]),
            ("GET", "/v2/print/jobs/{id}", "Read a print job.", [ID("print job")], [], [F("id", "string", "The identifier."), F("state", "string", "Where it has got to."), F("progress", "number", "From 0 to 1."), F("file_url", "string", "A signed link, once done."), F("expires_at", "timestamp", "When the link stops working.")]),
            ("DELETE", "/v2/print/jobs/{id}", "Cancel a job, or delete its file.", [ID("print job")], [], [F("state", "string", "`cancelled` or `deleted`.")]),
            ("GET", "/v2/print/presets", "List paper and scale presets.", [], [], [F("presets", "array", "Each with `name`, `paper`, `scale` and `margins_mm`.")]),
        ],
    ),
    (
        "Surveys",
        "Survey sessions hold what field teams record: tracks, photos and notes. They feed the next data build once "
        "somebody has reviewed them. Customers with a field-data contract can read their own sessions.",
        [
            ("GET", "/v2/surveys", "List survey sessions.", [], [BBOX, LIMIT, CURSOR, SINCE], [F("sessions", "array", "Session summaries."), F("next_cursor", "string", "Present when there is more.")]),
            ("POST", "/v2/surveys", "Start a survey session.", [], [P("name", "string", "A label, such as `Skåne coast, week 14`."), P("device", "string", "The handheld's serial number.")], [F("id", "string", "The session's identifier."), F("upload_url", "string", "Where to send the tracks.")]),
            ("GET", "/v2/surveys/{id}", "Read a survey session.", [ID("survey session")], [], [F("id", "string", "The identifier."), F("name", "string", "The label."), F("track_km", "number", "The length recorded."), F("photos", "integer", "How many photos."), F("reviewed_by", "string", "Who reviewed it, if anyone.")]),
            ("POST", "/v2/surveys/{id}/tracks", "Upload a track as GPX.", [ID("survey session")], [P("accuracy_m", "number", "The handheld's reported accuracy.")], [F("points", "integer", "How many were accepted."), F("rejected", "integer", "How many were dropped as noise.")]),
            ("POST", "/v2/surveys/{id}/photos", "Attach a geotagged photo.", [ID("survey session")], [P("lon", "number", "Overrides the photo's own tag."), P("lat", "number", "Overrides the photo's own tag.")], [F("photo_id", "string", "The photo's identifier."), F("thumbnail_url", "string", "A small copy.")]),
            ("POST", "/v2/surveys/{id}/review", "Approve a session for the next build.", [ID("survey session")], [P("note", "string", "Anything the build should know.")], [F("state", "string", "`approved`."), F("next_build", "date", "When it will be included.")]),
            ("GET", "/v2/surveys/{id}/diff", "Compare a session with the current data.", [ID("survey session")], [P("buffer_m", "integer", "How far a track may stray before it counts as new. Defaults to 10.")], [F("added", "array", "Features the survey has that the data does not."), F("missing", "array", "Features the data has that the survey did not find."), F("moved", "array", "Features displaced by more than the buffer.")]),
        ],
    ),
    (
        "Customers",
        "Account details for the customer the key belongs to. Only a secret key can read these, and nothing in this "
        "section is ever cached at the edge.",
        [
            ("GET", "/v2/customer", "Read the customer's account.", [], [], [F("id", "string", "The identifier."), F("name", "string", "The organisation."), F("contract", "string", "`club`, `standard` or `enterprise`."), F("default_lang", "string", "The label language."), F("regions", "array", "Where its tiles are served from.")]),
            ("PATCH", "/v2/customer", "Change the account's settings.", [], [P("default_lang", "string", "The label language.")], [F("default_lang", "string", "As stored."), F("updated_at", "timestamp", "When it changed.")]),
            ("GET", "/v2/customer/invoices", "List invoices.", [], [LIMIT, CURSOR], [F("invoices", "array", "Each with `number`, `amount`, `currency`, `paid_at` and `pdf_url`.")]),
            ("GET", "/v2/customer/attribution", "Read what the customer must display.", [], [P("style", "string", "One style, or all of them.")], [F("html", "string", "Ready to paste."), F("text", "string", "For print."), F("sources", "array", "Each source and its licence.")]),
            ("GET", "/v2/customer/members", "List the people on the account.", [], [LIMIT, CURSOR], [F("members", "array", "Each with `name`, `email` and `role`.")]),
            ("POST", "/v2/customer/members", "Invite somebody to the account.", [], [P("email", "string", "Their address, such as `someone@example.org`."), P("role", "string", "`admin`, `billing` or `viewer`.")], [F("invited", "boolean", "Always true."), F("expires_at", "timestamp", "When the invitation stops working.")]),
        ],
    ),
]

REGIONS = [
    ("eu-north", "Stockholm", "Fernway's first region", 14), ("eu-west", "Dublin", "Newspapers", 9),
    ("eu-south", "Milan", "Greek and Italian ferries", 5), ("me-south", "Bahrain", "Gulf ferries", 3),
    ("il-central", "Tel Aviv", "Hiking clubs", 2), ("ap-northeast", "Tokyo", "Japanese clubs", 4),
]

EXAMPLE_VALUES = {
    "string": '"outdoor"', "integer": "48", "number": "57.7", "boolean": "true", "array": "[]",
    "timestamp": '"2026-03-02T09:30:00Z"', "date": '"2026-03-02"',
}


def table(headers, rows):
    out = ["| " + " | ".join(headers) + " |", "|" + "---|" * len(headers)]
    out += ["| " + " | ".join(str(c) for c in row) + " |" for row in rows]
    return "\n".join(out)


def endpoint(method, path, summary, path_params, query_params, fields):
    out = [f"#### {method} `{path}`", "", summary, ""]
    if path_params:
        out += ["##### Path parameters", "", table(["Name", "Type", "Description"], [(f"`{n}`", k, d) for n, k, d in path_params]), ""]
    if query_params:
        label = "Query parameters" if method == "GET" else "Parameters"
        out += [f"##### {label}", "", table(["Name", "Type", "Description"], [(f"`{n}`", k, d) for n, k, d in query_params]), ""]
    url = path.replace("{style}", "outdoor").replace("{z}", "12").replace("{x}", "2189").replace("{y}", "1273")
    url = url.replace("{id}", "ds_01hzy8").replace("{name}", "sprite")
    flag = "" if method == "GET" else f"-X {method} "
    out += ["##### Example request", "", "```bash", f'curl {flag}"https://api.example.com{url}" \\', '  -H "Authorization: Bearer $FERNWAY_KEY"', "```", ""]
    out += ["##### Response", "", table(["Field", "Type", "Description"], [(f"`{n}`", k, d) for n, k, d in fields]), ""]
    body = {n: k for n, k, _ in fields if n != "(body)" and "[]" not in n}
    if body:
        js = ",\n".join(f'  "{n}": {EXAMPLE_VALUES.get(k, chr(34) + k + chr(34))}' for n, k in body.items())
        out += ["###### Example", "", "```json", "{\n" + js + "\n}", "```", ""]
    if method == "GET":
        out += ["Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.", ""]
    else:
        out += ["Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.", ""]
    out += [
        "##### Errors",
        "",
        table(["Status", "Error", "When"], [("`401`", "`invalid_key`", "The key is missing or revoked."), ("`404`", "`not_found`", f"Nothing at `{path}`."), ("`429`", "`rate_limited`", "Over the limit; see `Retry-After`.")]),
        "",
    ]
    return "\n".join(out)


def api_reference():
    out = [
        "# Fernway Maps API reference",
        "",
        "Version 2. Everything here is served from `https://api.example.com`, except the tile endpoints, which "
        "customers may also reach through `https://tiles.example.com`. Questions go to `support@example.com`.",
        "",
        "## Conventions",
        "",
        "Requests and responses are JSON unless the endpoint says otherwise. Timestamps are RFC 3339 in UTC. "
        "Identifiers are opaque strings, and you should not parse them. Unknown fields may be added to any "
        "response at any time, and a client must ignore them.",
        "",
        "### Pagination",
        "",
        "List endpoints take `limit` and `cursor`, and return `next_cursor` while there is more. A cursor stays "
        "valid for 24 hours.",
        "",
        "```bash",
        'curl "https://api.example.com/v2/styles?limit=2&cursor=eyJwIjoxfQ" -H "Authorization: Bearer $FERNWAY_KEY"',
        "```",
        "",
        "### Rate limits",
        "",
        "Every response carries `X-RateLimit-Remaining`. Over the limit, a request gets a 429 with `Retry-After`. "
        "Tile requests have a much higher limit than everything else, because a single screen of map is about twenty of them.",
        "",
        "## Authentication",
        "",
        "Send a key as a bearer token. Tile URLs may also carry it as `?key=`, since an `<img>` tag cannot set "
        "a header; a key used that way should be a public `pk_` key restricted by origin.",
        "",
        "```http",
        "GET /v2/styles HTTP/1.1",
        "Host: api.example.com",
        "Authorization: Bearer sk_test_0000000000000000",
        "```",
        "",
        "## Errors",
        "",
        "Errors share one shape, and `request_id` is what support will ask you for:",
        "",
        "```json",
        '{\n  "error": "invalid_field",\n  "message": "width must be between 64 and 2048",\n  "field": "width",\n  "request_id": "req_01hzy8m4"\n}',
        "```",
        "",
    ]
    rows = [(f"`{c}`", f"`{e}`", m, a) for c, e, m, a in COMMON_ERRORS]
    for group, _intro, endpoints in GROUPS:
        for method, path, _s, _pp, _qp, _f in endpoints:
            slug = group.lower().rstrip("s")
            what = path.split("/")[-1].strip("{}").split(".")[0] or slug
            rows.append(("`404`", f"`{slug}_{what.replace('-', '_')}_missing`", f"`{method} {path}`: nothing found.", "Check the identifier."))
            if method != "GET":
                rows.append(("`409`", f"`{slug}_{what.replace('-', '_')}_conflict`", f"`{method} {path}`: changed meanwhile.", "Read it again and retry."))
    out += [table(["Status", "Error", "Meaning", "What to do"], rows), ""]
    out += ["## Regions", "", table(["Region", "City", "Mostly serves", "Edge nodes"], REGIONS), ""]
    for group, intro, endpoints in GROUPS:
        out += [f"## {group}", "", intro, "", "### Endpoints", ""]
        for e in endpoints:
            out.append(endpoint(*e))
    out += [
        "## Changelog",
        "",
        table(
            ["Date", "Change"],
            [
                ("2026-02-20", "`/v2/route/profile` returns `ascent_m`."),
                ("2025-11-03", "Webhook deliveries are signed with SHA-256; the old scheme stops in June."),
                ("2025-06-17", "Vector tiles are gzip-encoded by default."),
                ("2024-09-30", "Version 2. Version 1's tile URLs keep working, and everything else redirects."),
            ],
        ),
        "",
    ]
    write(os.path.join(HERE, "api-reference.md"), "\n".join(out))


if __name__ == "__main__":
    api_reference()
    rfc()
    onboarding()
    glossary()
