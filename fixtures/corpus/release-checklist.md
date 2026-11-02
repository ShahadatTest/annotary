---
title: Style release checklist
owner: Layla Haddad
applies_to: [outdoor, nautical, atlas-print]
last_reviewed: 2026-02-20
tags:
  - process
  - styles
review_every_days: 90
---

# Style release checklist

Copy this list into the release's issue, and tick things as they happen. A box you cannot tick is a
reason to stop rather than a reason to write a note. The whole list takes about an afternoon.

## Before the publish

- [ ] Style file passes `fernway-styles lint` with no warnings
- [ ] Sprite sheet rebuilt, and both 1× and 2× checked at z12
- [ ] Label collisions checked on the four reference views
  - [ ] Gothenburg archipelago, z12
  - [ ] Haifa and the Carmel, z13
  - [ ] Muscat harbour, z15
  - [ ] Northern Alps, z11
- [ ] Attribution lines match each data source's current rules
- [ ] A changelog line a customer could read without help
- [ ] Not a Friday

## The publish

- [ ] Publish to staging, and look at it on a phone
- [ ] `fernway-tiles publish outdoor --region eu-north`
- [ ] Wait ten minutes and watch the render queue
- [ ] Publish to the remaining regions, one at a time

## After the publish

- [ ] Check all four reference views in production
- [ ] Post the changelog line in `#customers`
- [x] Keep the previous revision active in the rollback list for 30 days (the tool does this)
- [ ] Note anything this list got wrong, and fix the list

## If it goes wrong

Roll back first, and ask why afterwards. `fernway-tiles rollback outdoor` takes seconds, and because
every revision stays in the bucket, it needs no render at all:

| Symptom | Do this | Then |
|---|---|---|
| Labels missing or overlapping | Roll back | Reopen the checklist at label collisions |
| Render queue climbing | Roll back, and page Mei | Read the [cache RFC](rfc-tile-cache.md) failure table |
| A customer says the map is wrong | Check it against the data first | Roll back only if the style is at fault |
