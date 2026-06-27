# 11 — Aggregate performance (Q5, measured)

> The Q5 trigger was "measured evidence that pulling filtered tables up for PG to
> aggregate is too slow." Here's the measurement — and it points somewhere other
> than aggregate pushdown.

## The test

`Analysis` — the biggest fact table, **4,238,476 rows** (271 MB in the dump). A
representative single-table aggregate, run live against rivsem04:

| | What | Result |
| --- | --- | --- |
| **A — push down** | `SELECT SAGROUP, COUNT(*), SUM(SAVALNET) FROM Analysis GROUP BY SAGROUP` (server aggregates) | **1,138 rows in 96.2 s** |
| **B — current FDW** | `SELECT SAGROUP, SAVALNET FROM Analysis` (ship rows, PG aggregates) | 500k rows in 9.8 s ≈ 51k rows/s → **~83 s** for all 4.2M, + PG's aggregate |

## The finding: it's scan-bound, not aggregate-bound

Both paths are **~85–96 s, and it's all the full table scan.** DBISAM allows only
**4 indexes per table**, so a `GROUP BY` (or filter) on a non-indexed column is a
full scan of all 4.2M rows *regardless of where the aggregation happens*. Pushing
the `GROUP BY` down doesn't help wall-clock — it even costs a touch more (the
server does the scan *and* the aggregate). What pushdown *does* save is real but
secondary: **wire transfer** (1,138 rows vs 4.2M) and **PG memory/CPU** (no need
to materialise + aggregate 4.2M rows in Postgres).

So aggregate pushdown is a **data-volume / resource** optimisation, not a
**latency** one — on a LAN where transfer runs ~51k rows/s, the scan dominates
and the two are a wash.

## Consequences

1. **Don't build Q5 (aggregate pushdown) for speed — it won't make big-table
   aggregates interactive.** The ~90 s floor is the DBISAM full scan, which the
   FDW cannot remove. (It's still worth doing *if/when* the wire link is the
   bottleneck — e.g. a slow WAN where shipping 4.2M rows ≫ 1,138 — or to spare PG
   from buffering millions of rows. Re-evaluate per deployment, not as a default.)
2. **Big aggregates belong on the daily Parquet dump, not live DBISAM.** Measured:
   DuckDB over `analysis.parquet` (4.81M rows, columnar, **no DBISAM / no
   rivsem04**) runs this same `GROUP BY` in **0.36 s cold / 0.52 s warm** —
   ~260× faster than DBISAM's 96 s, and zero load on the production server. The
   live FDW cannot compete with a columnar snapshot for full-table analytics —
   and shouldn't try.
3. **The live FDW's sweet spot is small / selective / recent queries** — anything
   that hits one of the ≤4 indexes or a tight filter, so DBISAM scans a bounded
   slice rather than the whole table. There, filter+limit pushdown (already
   implemented) keeps both the scan and the transfer small, and you get *live*
   data the daily snapshot can't give.

## Net for Q5

Measured answer: **the bottleneck is DBISAM full-scan latency (≈90 s for 4.2M
rows), which aggregate pushdown does not address.** So Q5 stays deferred — not
"until it's slow" (it already is), but because pushdown isn't the fix. The real
routing rule is **freshness-vs-size**: live FDW for small/recent, Parquet dump
for big analytics. See `09-session-reuse.md` (the live-vs-snapshot boundary) and
`01-overview.md` (the DBISAM-as-dumb-leaf thesis — which holds, with the caveat
that the "leaf" is only *fast* on bounded scans).
