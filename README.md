# dbisam_fdw

A **read-only PostgreSQL Foreign Data Wrapper** for **DBISAM 4**, speaking the
native Exportmaster TCP protocol — no ODBC driver in the path. It exposes DBISAM
tables as Postgres foreign tables so a competent SQL engine sits in front of a
dumb, fast leaf: DBISAM hands back filtered/projected rows; Postgres absorbs
joins, CTEs, subqueries, window functions and aggregates. First use case is
PowerBI DirectQuery pointed purely at Exportmaster.

The thesis, scope, and contracts are fixed in [`proj_init/`](proj_init/) — read
`01`→`07` in order, then `08` for the resolved gate. **Do not re-litigate the
milestone-1 boundary** (`02-scope-v1.md`): scans go down, joins stay up.

## Layout

```
proj_init/      design docs (01–07) + 08-q1-resolution.md (the gate, resolved)
dbisam-sql/     quals → DBISAM SQL renderer + the 4 dialect quirks (plain lib)
dbisam_fdw/     the pgrx + Supabase Wrappers extension (the .so)  [excluded until bootstrap]
../exportmaster sibling crate: the native DBISAM wire-protocol client (Arrow output)
```

The protocol layer lives in its own repo — `../exportmaster`, extracted from
MrsFlow once Q1 confirmed it was cleanly separable (see
`proj_init/08-q1-resolution.md`).

## Build / test

```sh
cargo test -p dbisam-sql        # renderer — no Postgres toolchain needed
```

The extension crate needs the pgrx toolchain — see
[`dbisam_fdw/README.md`](dbisam_fdw/README.md) for bootstrap.

## Status

- **Gate (Q1) resolved** — `exportmaster` extracted to a standalone crate that
  builds + passes 46 tests. MrsFlow rewired onto it (its in-tree module → a thin
  adapter); one source of truth.
- **`dbisam-sql` renderer** — foldable subset + all four quirks, 14 tests green.
- **`dbisam_fdw`** — `ForeignDataWrapper` implemented; compiles against system
  PG15 (pgrx 0.16.1 / supabase-wrappers 0.1.28) and `cargo pgrx schema` emits a
  valid extension. Scan/projection/filter/limit pushdown + import wired; live
  end-to-end scan against a real server is the remaining step.
