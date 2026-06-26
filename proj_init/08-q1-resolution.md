# 08 — Q1 resolution (the gate)

> Resolves `07-open-questions.md` **Q1**, the gate that had to be settled before
> any other work. Status: **RESOLVED — favorable branch.**

## Answer

**`exportmaster` is cleanly extractable. It is now its own crate.**

Per Q1's decision rule — *"self-contained module with no hard ties to evaluator
types or `IoHost` → dbisam_fdw is its own repo from commit one, depending on the
crate"* — we are in the favorable branch. The protocol module has **zero** ties
to MrsFlow's evaluator or its `IoHost` trait (the trait name appears nowhere in
its 13 files).

This was confirmed not by argument but by doing the extraction: the crate now
lives at `../exportmaster`, **builds standalone**, and passes its 46 unit tests
with no `mrsflow-core` dependency.

## What the coupling actually was

The module (`mrsflow-cli/src/exportmaster/`, ~4,700 lines, 13 files) had exactly
three shallow ties to `mrsflow-core`, all mechanical to re-home:

| Tie | Where | Resolution |
| --- | --- | --- |
| `mrsflow_core::eval::IoError` | 10 files, bare `use` | mirrored as crate-local `IoError` (same 2 variants: `NotSupported`, `Other(String)`) |
| `decode_dbisam_text` (Win-1252→UTF-8) | 1 call site (`row.rs`) | lifted verbatim into `exportmaster::text` |
| `Value` / `Table` (the M value model) | `client.rs` only, at the `query_to_table` *return* | swapped to the Arrow `RecordBatch` that `ColumnBuilders::finish()` already produces |

The decisive structural fact: the wire core (framing, msg, response, crypto,
cursor, schema, **row**, blob — 11 of 13 files) decodes into a protocol-native
`CellValue` enum + Arrow `ColumnBuilders`, **not** into M `Value`s. The
`Value::Table` wrap was a one-line cap at the very top. The crate's natural
public boundary (`Client` + `RecordBatch` / `CellValue` + `Column` + `ConnOpts`)
is exactly what an FDW wants — and the FDW *discards* the `Value` layer the old
code added, replacing it with PG-datum conversion.

## What stayed behind in MrsFlow

Two functions were host glue, not protocol, and remain in MrsFlow:

- `list_tables_as_navigation` — builds an M `Value::Thunk` navigation table.
- `build_lazy_exportmaster_table` — builds a `LazyOdbc` fold plan (`MError`,
  `TableRepr`, `SqlDialect`).

Plus the M-facing entry points `query()` / `database()` / `apply_options`
(parsing an M option record) and `rows_affected_record`. These become a thin
adapter in MrsFlow that calls the crate and maps `RecordBatch` → `Value::Table`.

## Consequences for the build

1. **dbisam_fdw is its own repo from commit one** (as `03-architecture.md`
   already wanted), depending on `exportmaster` via path today, a published
   version later.
2. **MrsFlow now depends on the crate (DONE).** `mrsflow-cli`'s in-tree
   `src/exportmaster/` directory was deleted and replaced with a thin
   `src/exportmaster.rs` adapter that re-exports the crate's surface and keeps
   only the M glue: `RecordBatch → Value::Table`, the nav/lazy-plan builders,
   `apply_options`, and an `IoError → mrsflow_core::eval::IoError` relabel at
   the boundary. The four crypto deps (blowfish/md-5/cbc/flate2) moved out of
   `mrsflow-cli`'s manifest into the crate. Verified: `mrsflow-cli` builds with
   and without `--features exportmaster`, lib tests pass, the `em_smoke` example
   and all integration targets compile. (One pre-existing, unrelated failure:
   the `parquet_roundtrip` test fails identically on the untouched tree — a
   stale `Rc` move, not touched here.) One source of truth achieved.
3. **Q2 (`dbisam-sql` carve-out) is now cheap.** Q1's rule applies: if the
   protocol separated cleanly, the render rules will too. The renderer lands as
   its own plain crate (toolchain-free, unit-testable without pgrx), ported
   faithfully from Delilah against Dibdog per `04-pushdown-contract.md`.

## Verification

```
cd ../exportmaster
cargo build      # clean (pre-existing warnings only, verbatim from MrsFlow)
cargo test       # 46 passed; 0 failed; 1 ignored (live-server integration)
```
