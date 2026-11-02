# Monday notes, 9 March

Present: Rowan, Mei, Layla, Ana. Notes by Layla. Tomasz is on the coast, and sent a voice memo about
a fence.

## Last week

- The memory tier passes its unit tests. Mei found an accounting bug in eviction when a tile is
  replaced by a smaller one, and fixed it before anyone else saw it.
- The cliff symbol is done, and looks like a cliff for the first time since 2022.
- Rowan's call with the orienteering federation went well. They renewed for two years, and asked
  again for everything to be pre-rendered. They were told no again.

## This week

- Mei adds the object-store tier and starts the failure drills in staging.
- Layla checks the labels on the outdoor style at z13, where the new sprites crowd the summit names.
- Ana is on holiday from Wednesday. Ask Rowan about anything to do with accounts.

## Decisions

1. We publish the new outdoor style next Tuesday, and not on a Friday, ever.
2. The warm job reads aggregated counts only. See the privacy section of the
   [RFC](rfc-tile-cache.md#security-and-privacy).

## Numbers

| Metric | This week | Last week |
|---|---:|---:|
| Requests (billions) | 2.3 | 2.1 |
| p95 tile latency | 182 ms | 190 ms |
| Pages out of hours | 0 | 1 |
| Support tickets | 14 | 22 |

## Parking lot

The Muscat z17 question is still here. Mei looked for ten minutes: the coastline there has an
absurd number of vertices. A data build problem, then. Over to Tomasz, when he comes back.

Next Monday: the first failure-drill results, read aloud, with feeling.
