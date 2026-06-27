# 07 — Open questions

Unresolved decisions, flagged rather than buried. Q1 gates the first commit.

## Q1 — Is `exportmaster` cleanly extractable from MrsFlow? (GATE)

Whether dbisam_fdw begins as its own repo depending on a published
`exportmaster` crate, or starts in-tree in MrsFlow and splits out later,
depends entirely on how cleanly the protocol module separates from MrsFlow's
synchronous evaluator and `IoHost` trait.

- If `exportmaster` is already a self-contained module with no hard ties to
  evaluator types or `IoHost` → dbisam_fdw is its own repo from commit one,
  depending on the crate.
- If it is still woven in → start dbisam_fdw in-tree to avoid blocking on a
  painful extraction, and split out once the protocol is freed.

**Resolve this before any other work.** Everything downstream is plumbing
already mapped in docs 01–06.

## Q2 — Carve out `dbisam-sql`, or duplicate the renderer?

The quals → DBISAM WHERE rules now exist in MrsFlow (Rust), Delilah (C++) and
ExportKing (.NET). A shared `dbisam-sql` Rust crate would keep the FDW's
renderer DRY with MrsFlow's. But it is an extraction cost, and the C++/.NET
siblings cannot share it regardless.

- The *protocol* is the shareable core; the *render rules* keep getting
  re-expressed per host's predicate vocabulary.
- Dibdog is the real anti-drift mechanism across all four — the grammar
  oracle, not co-located code.

Decision: worth carving `dbisam-sql` out *if* it is cheap given Q1's answer
(if `exportmaster` is already separable, the render rules likely are too). If
extraction is costly, port the renderer into the FDW against Dibdog and accept
two Rust copies, kept honest by the grammar. Not a blocker either way.

## Q3 — Eager schema default for the PowerBI setup path? — RESOLVED

Reframed by the live catalogue: `list_tables` is **volatile**, not a schema —
~357 of NISAINT_CS's 626 entries are transient `wk<hex>` scratch tables. So
"import everything" was never the right eager default. The useful set (~198) is
enumerated by the **daily Parquet dump**, so import curates from it:
`IMPORT FOREIGN SCHEMA … OPTIONS (parquet_dir '…')` restricts to tables that have
a `<name>.parquet` file (real names/case from `list_tables`, columns from the
live `WHERE 1=0` probe, `pk` set per table). Implemented + verified live; see
`05-type-mapping.md`, the FDW README, and `dbisam_fdw/src/schema_import.rs`.
Probes are serial, so the login-storm constraint is moot here. Omit the option
to import the full (volatile) catalogue.

## Q4 — Broker vs serialise for Exportmaster concurrency? — MEASURED & PARKED

Resolved by experiment (see `09-session-reuse.md`), then **parked** — not needed
for milestone 1. Findings: a DBISAM *login* can't be reused across queries
(`0x2C2C`), but a warm TCP *socket* can (`Client::reauth`); and the real load
limit is the server's TCP accept path refusing *concurrent connections* (clean
to ~4, failing from 8). Answer when it's needed: an out-of-process **broker over
a ~4 warm-socket pool**, with `reauth` as its per-query step (so no standalone
FDW reuse work). **Interim for milestone 1:** keep concurrent PG→DBISAM backends
≤ ~4 (gateway/PG pool); the FDW's connect-per-scan is fine. Revisit the broker
only if a real DirectQuery page shows it's needed.

## Q5 — Aggregate pushdown: when does phase 2 start? — MEASURED: NOT THE FIX

Measured live (`11-aggregate-perf.md`): a `GROUP BY` over `Analysis` (4.2M rows)
takes **~96 s pushed down vs ~85 s pulled up** — a wash, because both are
dominated by a **DBISAM full scan** (only 4 indexes/table → non-indexed
aggregates always full-scan). Aggregate pushdown saves wire transfer (1,138 vs
4.2M rows) and PG memory, but **not latency** — it does not make big aggregates
interactive, so it is *not* the answer to slow visuals.

Stays deferred, with the reasoning changed: the real rule is **freshness-vs-size**
— route big analytics to the daily Parquet dump (DuckDB does this `GROUP BY` in
0.36 s, columnar, off the snapshot — no DBISAM), and reserve the live FDW for
small/selective/recent queries that
hit an index or a tight filter. Build aggregate pushdown only if a *specific*
deployment is wire-bound (slow WAN) or PG-memory-bound — re-evaluate then, not as
a default. Joins still never go down.

## Settled — do not reopen

For the record, these were decided in discussion and should not be
re-litigated mid-build:

- Read-only, enforced by absent write callbacks. (Despite ExportKing proving
  the protocol can write — read-only is a chosen contract, not a limitation.)
- Joins stay in Postgres; never pushed to DBISAM.
- Native Exportmaster protocol, never ODBC.
- Separate repo from MrsFlow (modulo Q1's in-tree-then-split timing).
- Supabase Wrappers for milestone 1, not raw pgrx.
- pg_duckdb-over-Delilah rejected as a path (SELECT-only dead end and an
  unpleasant split-planner experience in practice).
