# dbisam_fdw — design docs

A read-only PostgreSQL Foreign Data Wrapper for DBISAM 4, speaking the
native Exportmaster TCP protocol. Sibling to **Delilah** (DuckDB / C++) and
**MrsFlow** (Power Query M / Rust); shares the reverse-engineered protocol
documented in **Derek** and the SQL grammar in **Dibdog**.

These are design documents, not code. They exist to fix the decisions the
build should not re-litigate, and to hand Claude Code an unambiguous target.

## Start here

Read in order:

1. **01-overview.md** — what dbisam_fdw is, the core thesis, the first use case.
2. **02-scope-v1.md** — the milestone-1 line. The single most important doc:
   it fixes the scan-down / SQL-up boundary so join pushdown does not creep
   back in mid-build. Read this before writing anything.
3. **03-architecture.md** — repo shape, crate dependencies, framework choice,
   and the one unresolved gate (protocol-crate extraction).
4. **04-pushdown-contract.md** — exactly which predicates render to DBISAM SQL,
   which fall back to Postgres, and the four dialect quirks to compensate.
5. **05-type-mapping.md** — DBISAM → PostgreSQL type table and the fidelity
   requirements the PowerBI/Npgsql path imposes.
6. **06-connection-broker.md** — the concurrent-login constraint and the
   DirectQuery fan-out it has to survive. Not FDW-internal, but build it alongside.
7. **07-open-questions.md** — unresolved decisions, flagged not buried.

## The one gate before first commit

`07-open-questions.md` Q1: is MrsFlow's `exportmaster` module cleanly
extractable as a standalone crate today, or still woven into the evaluator
and `IoHost`? That answer decides whether dbisam_fdw starts as its own repo
depending on a published crate, or starts in-tree and splits out later.
Resolve it first.
