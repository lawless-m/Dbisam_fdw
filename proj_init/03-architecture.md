# 03 — Architecture

## Shape

dbisam_fdw is a **standalone repository**, not a member of the MrsFlow
workspace. It is a pgrx-based PostgreSQL extension producing a versioned
`.so` that loads into a Postgres backend.

It stands apart from MrsFlow deliberately:

- **Build target mismatch.** MrsFlow is a pure synchronous evaluator with IO
  quarantined behind `IoHost`, kept trivial to compile to WASM. A pgrx FDW is
  the opposite: it links the Postgres C ABI, pins to a PG major version, and
  can only load inside a backend. Folding that into MrsFlow would put a
  Postgres version pin and C linkage into a workspace that has carefully
  avoided both.
- **Lineage.** The FDW is structurally a sibling of **Delilah** (catalog
  attach, scan with projection/filter/limit pushdown, filter renderer,
  read-only), retargeted from DuckDB's extension API to Postgres's FDW API.
  It is "Delilah for Postgres," not "MrsFlow with an FDW bolted on."
- **Contract legibility.** Read-only-by-absent-write-callbacks wants to be
  visible at the repo boundary. In its own repo, "the read-only Postgres FDW
  for DBISAM" is the whole identity. Inside MrsFlow it is a feature-gated
  corner and the guarantee gets quieter.

## Dependencies

dbisam_fdw depends on two MrsFlow-originated crates and contributes nothing
back into MrsFlow's build:

| Crate | Origin | What it provides |
| --- | --- | --- |
| `exportmaster` | MrsFlow | Native DBISAM wire protocol client: framing, crypto, cursor, schema, row/blob decode. The data-access layer. **Reused as-is.** |
| `dbisam-sql` | carved out of MrsFlow | The DBISAM SQL render rules + quirk compensations. Shared so the FDW's renderer cannot drift from MrsFlow's. See note below. |

The protocol layer is **zero new code** — link and use. The render crate is a
small extraction whose value is keeping the rules DRY across the Rust
implementations; the C++ (Delilah) and .NET (ExportKing) ones cannot share it
and stay anchored to **Dibdog** as the common grammar oracle instead.

## Framework: Supabase Wrappers

Milestone 1 is scan / projection / filter / limit pushdown with everything
else handled by Postgres. That is exactly Supabase Wrappers' sweet spot — it
surfaces `quals`, `columns`, `sorts`, and `limit` as structured Rust values
and handles the C callback boilerplate. Use it.

Raw pgrx was considered and is **not** needed for milestone 1. It only becomes
relevant if the phase-2 single-table aggregate pushdown turns out to need a
planner path (`GetForeignUpperPaths`) that Wrappers cannot advertise. Revisit
then, not now. Do not start on raw pgrx "to be safe" — it is a larger floor
for no milestone-1 benefit.

## New code, in scope

The build is narrow. The genuinely new pieces:

1. **quals → DBISAM WHERE renderer.** Ported from Delilah's
   `dbisam_filter_render`, expressed against the PG qual shapes Wrappers
   hands you, gated by Dibdog. See `04-pushdown-contract.md`.
2. **`ImportForeignSchema`.** Reflect the DBISAM catalogue into foreign
   tables. Mirrors Delilah's catalog enumeration and the lazy-vs-eager schema
   trade-off (instant `SHOW TABLES` vs. populating `information_schema.columns`
   for GUI/tool introspection).
3. **DBISAM → PG type mapping.** See `05-type-mapping.md`.
4. **Connection handling.** See `06-connection-broker.md` — not FDW-internal
   but mandated by the use case.

Everything else is reuse.

## The gate before first commit

Whether dbisam_fdw begins as its own repo depending on a published
`exportmaster` crate, or starts in-tree in MrsFlow and splits out later,
depends entirely on how cleanly `exportmaster` separates from MrsFlow's
evaluator and `IoHost` today. This is unresolved. See `07-open-questions.md`
Q1 and settle it first.
