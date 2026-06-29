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

`cargo pgrx run` is a **sandbox** (`~/.pgrx/data-15`) — recreated on demand, not
the database clients connect to. To use the wrapper for real, install it into
the system PG15.

## Deploy to the system PG15 (the real database)

There are two PostgreSQL 15s on this box: the pgrx sandbox above, and the system
server (`/usr/bin/pg_config` → `/usr/lib/postgresql/15/lib`,
`/usr/share/postgresql/15`). The DDL below only works once the extension's `.so`
+ `.control` + SQL live in the **system** tree. `cargo pgrx install` puts them
there; rerun it after every code change:

```sh
cargo pgrx install --release --no-default-features --features pg15 \
  -c /usr/bin/pg_config        # add --sudo if the PG dirs aren't user-writable
```

This writes `dbisam_fdw.so` → `/usr/lib/postgresql/15/lib/` and the
`.control` + SQL → `/usr/share/postgresql/15/extension/`. No restart needed; a
fresh `psql` session picks it up. Then connect to your target database
(`psql -d <yourdb>`) and run the DDL below.

```sql
CREATE EXTENSION dbisam_fdw;

-- The extension ships the handler/validator functions; declare the wrapper:
CREATE FOREIGN DATA WRAPPER dbisam_fdw
  HANDLER dbisam_fdw_handler VALIDATOR dbisam_fdw_validator;

-- Credentials go in the SERVER options. supabase-wrappers 0.1.28 exposes only
-- server options to the FDW (its ForeignServer has no user-mapping field), so
-- USER MAPPING options are NOT seen by the wrapper. Reading per-user mappings
-- would need a manual pg_sys catalog lookup — a future enhancement.
CREATE SERVER em FOREIGN DATA WRAPPER dbisam_fdw
  OPTIONS (host '...', port '12005', catalog 'NISAINT_CS',
           user '...', password '...');

-- One per table:
CREATE FOREIGN TABLE miketest ("Mike1" text, "Mike2" text)
  SERVER em OPTIONS (table 'MikeTest');

SELECT * FROM miketest;

-- ...or bulk-import, CURATED to a daily Parquet dump's filenames. Never import
-- the whole catalogue: it's mostly volatile wk<hex> scratch tables (≈357 of 626
-- on NISAINT_CS). `parquet_dir` restricts the import to tables that have a
-- <name>.parquet there; the real table set comes from the live catalogue,
-- columns from a live WHERE 1=0 probe. All PG identifiers are lowercased
-- (DBISAM is case-insensitive, so its stored case is arbitrary noise — folding
-- to lowercase lets callers write natural unquoted SQL); each table with its
-- `pk` option set.
IMPORT FOREIGN SCHEMA dbisam FROM SERVER em INTO public
  OPTIONS (parquet_dir '/mnt/RIVSPROD02_RI_SERVICES/Outputs/Parquets/em');
-- (the PG server process must be able to read parquet_dir; only filenames are
-- read, not contents. Omit the option to import the full, volatile catalogue.)
```

Verified end-to-end against a live DBISAM server (PG15 → dbisam_fdw →
exportmaster → DBISAM): `SELECT * FROM miketest` returned all rows, matching a
direct protocol-level query.

## Status

The `ForeignDataWrapper` trait is implemented and **compiles against PG15**;
`cargo pgrx schema` emits a valid extension (handler + validator). Wired up:

- `begin_scan` — builds `SELECT <proj> FROM <table> [WHERE …] [TOP n]`, pushing
  the foldable qual subset via `dbisam-sql` and `TOP n` only when every qual was
  pushed (`04 §Limit edge case`).
- `quals.rs` — Wrappers `Qual` → `dbisam_sql::Pred` (comparisons, IN/NOT IN,
  prefix LIKE, IS [NOT] NULL, and date/timestamp predicates rendered as DBISAM's
  quoted-string literals; params fall back to Postgres). All pushdown is an
  optimisation — Postgres re-applies every qual locally (see "Null handling").
- `iter_scan` / `typemap.rs` — Arrow `RecordBatch` → Wrappers `Cell`.
- `import_foreign_schema` — probes each table (`WHERE 1=0`) and emits
  `CREATE FOREIGN TABLE` DDL.
- Read-only: `begin_modify`/`insert`/`update`/`delete` return an error (the
  Wrappers-shaped form of doc 02's read-only contract — Wrappers' modify
  methods default to silent no-ops, so absence isn't enough here).

Verified live (PG → dbisam_fdw → exportmaster → DBISAM):
- scalar scans (`MikeTest`);
- **memo resolution as `text`** (`ARCVCFG.ACMemo`, via `OpenBlob`/`FreeBlob`):
  exportmaster tags each column with its DBISAM `FieldType` in the Arrow field
  metadata, so the FDW maps Memo→`text` (Win-1252→UTF-8) and binary Blob/Graphic
  →`bytea`;
- **PK auto-injection**: a `pk` table option lets the FDW prepend the PK to the
  DBISAM projection (so the blob resolver's `columns[0]`-is-PK rule holds even
  for `SELECT <memo>` alone); the injected PK is dropped from output. IMPORT
  FOREIGN SCHEMA sets `pk` automatically.
- **Currency → `numeric`** (lossless): `PRODUCT.PRICE` returns `79.3400` typed
  as PG `numeric`, and arithmetic works (`PRICE * 1.2`). Fixing this surfaced a
  real exportmaster bug — DBISAM stores `ftCurrency` on disk as an IEEE-754
  *double* (not a scaled Int64); it's now read as f64 and rounded into
  `Decimal128(38, 4)`.

## Null handling / pushdown correctness

Pushdown is **never** the final authority: Supabase Wrappers passes all
`scan_clauses` to `make_foreignscan` as local qual, so Postgres re-applies every
WHERE condition (with PG semantics) on the rows the FDW returns. The renderer's
only obligation is therefore to never push a predicate that returns *fewer* rows
than PG wants (a subset) — a superset is always corrected by the recheck.

- DBISAM's `NULL <> x` is TRUE (returns NULLs PG would exclude) → the renderer
  guards `<>`/`NOT IN` with `AND col IS NOT NULL`. Under recheck this is an
  efficiency win (fewer rows on the wire); it also keeps the SQL exact for
  non-rechecking consumers.
- `IS [NOT] NULL` is exact: exportmaster decodes NULL from DBISAM's per-field
  null-flag, and DBISAM SQL `IS NULL` keys off that same flag — so the pushed
  predicate selects exactly PG's NULL rows.
- Partial-`AND` (drop unfoldable conjuncts) and `OR`-all-or-nothing both keep the
  pushed result a superset, so they're safe.

## Known gaps

- **Identifier case & reserved words** (handled — noted for the residual).
  `IMPORT FOREIGN SCHEMA` folds every PG-facing identifier to lowercase, since
  DBISAM is case-insensitive and stores an arbitrary case; `iter_scan` and the
  `pk` dedup match results case-insensitively to compensate. Wire emission flows
  through one `dbisam_sql::quote_ident` matching Dibdog's `gen_ident_atom`:
  simple names bare, character-odd *and reserved* names double-quoted (the
  keyword list sourced from Dibdog), which DBISAM resolves case-insensitively
  too. Residual: Postgres itself still makes callers double-quote a reserved-word
  column (e.g. `"group"`) — inherent to PG, not the FDW; and `INT`-style names
  absent from DBISAM's keyword set go bare.
- **Session reuse fails.** Sequential queries on one Exportmaster session error
  (2nd `PrepareStatement` → `0x2C2C`); each query needs a fresh login. Sharpens
  the broker decision (`06`, Q4) — per-backend reuse needs protocol work.
- Live end-to-end is via the pgrx-managed PG15; streaming (vs materialising the
  whole batch in `begin_scan`) is a later refinement.
