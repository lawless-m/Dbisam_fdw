# dbisam_fdw (pgrx extension crate)

The Supabase Wrappers / pgrx extension itself — the loadable `.so`. This crate
is **excluded from the parent workspace** until the pgrx toolchain is installed,
so `cargo test` on the `dbisam-sql` renderer never pulls the heavy pgrx graph.

## Bootstrap (done on this machine)

Pinned to `pgrx 0.16.1` / `supabase-wrappers 0.1.28`, targeting the system
PostgreSQL 15 (`postgresql-server-dev-15`), so no from-source PG build:

```sh
cargo install cargo-pgrx --version 0.16.1 --locked
cargo pgrx init --pg15 /usr/bin/pg_config   # registers system PG15 in ~/.pgrx
```

To develop against a different PG major, install its `-server-dev` package and
`cargo pgrx init --pgNN $(which pg_config)`, then build with `--features pgNN`.

## Develop / run

```sh
cargo build --features pg15 --no-default-features   # type-check / build the .so
cargo pgrx schema pg15                              # emit the extension SQL
cargo pgrx run pg15                                 # install into a throwaway PG15 + psql
```

```sql
CREATE EXTENSION dbisam_fdw;
CREATE SERVER em FOREIGN DATA WRAPPER dbisam_fdw
  OPTIONS (host '...', catalog 'NISAINT_CS');
CREATE USER MAPPING FOR CURRENT_USER SERVER em OPTIONS (user '...', password '...');
IMPORT FOREIGN SCHEMA dbisam FROM SERVER em INTO public;
```

## Status

The `ForeignDataWrapper` trait is implemented and **compiles against PG15**;
`cargo pgrx schema` emits a valid extension (handler + validator). Wired up:

- `begin_scan` — builds `SELECT <proj> FROM <table> [WHERE …] [TOP n]`, pushing
  the foldable qual subset via `dbisam-sql` and `TOP n` only when every qual was
  pushed (`04 §Limit edge case`).
- `quals.rs` — Wrappers `Qual` → `dbisam_sql::Pred` (comparisons, IN/NOT IN,
  prefix LIKE; params and dates fall back to Postgres).
- `iter_scan` / `typemap.rs` — Arrow `RecordBatch` → Wrappers `Cell`.
- `import_foreign_schema` — probes each table (`WHERE 1=0`) and emits
  `CREATE FOREIGN TABLE` DDL.
- Read-only: `begin_modify`/`insert`/`update`/`delete` return an error (the
  Wrappers-shaped form of doc 02's read-only contract — Wrappers' modify
  methods default to silent no-ops, so absence isn't enough here).

Not yet done / known gaps: live end-to-end scan against a real DBISAM server
(needs credentials); per-backend session reuse + streaming (`06`); pushing
`IS NULL` and date/time predicates (need the DBISAM `#…#` literal pinned vs
Dibdog); currency→`numeric` fidelity (an exportmaster decode gap, see
`typemap.rs`).
