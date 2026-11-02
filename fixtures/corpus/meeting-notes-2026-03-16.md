# Monday notes, 16 March

Present: Rowan, Mei, Tomasz, Layla. Notes by Tomasz, who apologises for the handwriting in advance
of having typed this.

## Last week

- All four failure drills ran. Three behaved as the RFC says they should. In the fourth, blocking
  the object store made the writers queue up and use more memory than we expected, so the write
  queue is now bounded in bytes rather than in items.
- The outdoor style publish moved to Tuesday, and went out in under a minute. Nobody noticed, which
  was the whole idea.
- The field notes from the Skåne coast are up: [field notes](field-notes.md). Comments welcome,
  especially from anyone who has seen the burnt bench.

## This week

- Mei runs the 2× load test, with a publish in the middle of it.
- Tomasz simplifies the Muscat coastline in the data build. Early numbers suggest 40,000 vertices
  can become about 3,000 with no visible change at z17.
- Layla has a first draft of the Japanese label rules. See the [glossary](glossary-multilingual.md).

## Decisions

1. Rollout step one starts on the 30th if the load test passes.
2. The render deadline stays one number for now. A per-customer deadline waits until somebody other
   than the federation asks for it.

## Questions we did not answer

- Who owns the warm job after the rollout?
- Should we tell customers about the cache at all? Rowan says yes, in one sentence, after it has
  worked for a month.

## Parking lot

Empty, for the first time since November. Ana will want to frame this.

Next Monday: Ana is back, so somebody should have tidied the shared drive by then.
