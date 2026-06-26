# 02 — Scope, milestone 1

This is the load-bearing document. Its job is to stop join and aggregate
pushdown creeping back into the design mid-build. Read it before writing code.

## The boundary

**Scan pushdown down. Rich SQL up. Single source.**

Push down to DBISAM only the four things DBISAM is genuinely good at and that
Delilah already proves:

- **Scan** of a single DBISAM table.
- **Projection** — only requested columns travel over the wire.
- **Filter** — the foldable WHERE subset (see `04-pushdown-contract.md`).
- **Limit** — emitted as a trailing DBISAM `TOP n`.

Everything else stays in Postgres:

- **Joins** — Postgres joins the streams. DBISAM's join engine is old and
  slow; a PG hash join over already-filtered streams will usually beat it.
  Do **not** push joins down.
- **CTEs, subqueries, derived tables, window functions** — Postgres handles
  these natively; DBISAM cannot express them at all. This is the whole point.
- **Aggregates** — Postgres groups. (Single-table aggregate pushdown is a
  *possible second pass* — see below — but is explicitly out of milestone 1.)

## Why this is the right line, not a compromise

It is less to build *and* more capable. Letting Postgres own joins and
nesting deletes the entire "deparse joins into the DBISAM dialect" problem —
a dialect that, by Dibdog, can barely express joins — so we never attempt it.
And it matches the model already trusted in Delilah. The earlier instinct to
fold joins/aggregates "purely into exportmaster" was a misreading of
"purely"; purely means single-source, not all-operations-server-side.

## Explicitly out of scope for milestone 1

- **Writes.** Read-only. See "Read-only as structure" below. ExportKing
  proves the protocol *can* write, so this is a deliberate scope choice, not
  a capability gap — see `07-open-questions.md` for the contract argument.
- **Federation.** No DBISAM-table-joined-to-native-PG-table in the first use
  case. Everything is single-source DBISAM.
- **Cost estimation / `AnalyzeForeignTable`.** Because milestone 1 is
  single-source, the planner never has to estimate DBISAM cardinality to
  choose a cross-source join order — there is no cross-source join. Stats
  only matter the day federation arrives; defer entirely.
- **Join pushdown** (`GetForeignJoinPaths`) and **aggregate pushdown**
  (`GetForeignUpperPaths`).

## Read-only as structure, not as a flag

Enforce read-only by **not implementing the write callbacks**
(`PlanForeignModify`, `BeginForeignModify`, `ExecForeignInsert/Update/Delete`,
`AddForeignUpdateTargets`). With those absent, Postgres rejects any DML at
plan time with a clean "cannot modify foreign table" error, before a byte
reaches the server. This is structurally stronger than Delilah's runtime
throw or an `ATTACH READ_ONLY` flag: there is no code path to writes to get
wrong, and no future commit can introduce one without deliberately adding the
callbacks. The read-only contract *is* the absence.

## The possible second pass (not milestone 1)

Single-table `GROUP BY` pushdown is the one operation beyond scans that may
later earn its keep. Postgres is a row store and a weaker aggregator over
large inputs than DuckDB's vectorised engine; when a visual wants a small
grouped summary off a big DBISAM table, pulling the whole filtered table up
for PG to crunch is the one place the let-the-engine-do-it model strains.
Collapsing that `GROUP BY` into a server-side aggregate so DBISAM returns a
handful of rows is worth doing — **later**, as a deliberate second milestone,
and only for single-table aggregates. Joins still never go down.

## Refined rule of thumb

Scans always go down. Single-table aggregates *optionally* go down (phase 2).
Joins stay up in Postgres. Always.
