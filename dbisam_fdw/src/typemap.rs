//! DBISAM → PostgreSQL type mapping and value conversion —
//! `proj_init/05-type-mapping.md`.
//!
//! In milestone 1 this is a **correctness** requirement, not a convenience:
//! data exits Postgres through Npgsql to PowerBI, which renders coercion
//! surprises silently. The fidelity rules to honour:
//!
//! - **Lossless numeric** — DBISAM BCD / currency → PG `numeric` with full
//!   precision/scale, never a float that drops digits.
//! - **Text** — already transcoded Win-1252 → UTF-8 at the protocol boundary
//!   by `exportmaster` (`decode_dbisam_text`), so PG sees clean UTF-8.
//! - **Dates/times** — `date` / `time` / `timestamp`; verify round-trip
//!   through Npgsql, not just `psql`.
//! - **Null handling** — distinguish real NULLs from decode failures. Default
//!   strict (raise); offer an opt-in `lenient_decode` that turns failures into
//!   NULLs with a per-batch summary (mirrors Delilah).
//!
//! Mapping table skeleton (confirm tags against `exportmaster::schema` +
//! Derek before relying on it):
//!
//! | DBISAM            | PostgreSQL              |
//! |-------------------|-------------------------|
//! | int family        | int2 / int4 / int8      |
//! | BCD / currency    | numeric (lossless)      |
//! | float / double    | float4 / float8         |
//! | boolean           | bool                    |
//! | string / char     | text / varchar(n)       |
//! | memo              | text  (per-row OpenBlob)|
//! | blob / graphic    | bytea (per-row OpenBlob)|
//! | date/time/datetime| date / time / timestamp |
//!
//! Two jobs live here:
//! 1. `schema_type(col: &exportmaster::Column) -> PgType` — for
//!    `IMPORT FOREIGN SCHEMA` DDL and column declarations.
//! 2. `cell_to_datum(cell: &exportmaster::CellValue) -> Cell` — per-value
//!    conversion during `iter_scan`.

// TODO: implement once the Wrappers `Cell` enum and the pg type OIDs are in
// scope (needs the pgrx build). The Arrow `RecordBatch` exportmaster returns
// already carries an Arrow schema, so much of (1) can be derived from there;
// (2) reads the typed Arrow columns / CellValues.
