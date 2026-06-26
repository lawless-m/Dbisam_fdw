//! DBISAM/Arrow → PostgreSQL type mapping — `proj_init/05-type-mapping.md`.
//!
//! `exportmaster` decodes DBISAM rows into Arrow arrays; this module maps those
//! into Wrappers [`Cell`]s for `iter_scan`, and into PG type names for
//! `IMPORT FOREIGN SCHEMA`. The Arrow types `exportmaster` actually emits
//! (see its `row.rs`) are: Utf8, Int32, Int64, Float64, Boolean, Date32,
//! Timestamp(µs), Binary — plus Int64 for DBISAM Time (a current exportmaster
//! quirk).
//!
//! KNOWN FIDELITY GAPS to close against doc 05 (both originate in exportmaster,
//! not here):
//!   - DBISAM Currency/BCD currently decode to Float64 → mapped to `double
//!     precision`. Doc 05 requires lossless `numeric`. Fix in exportmaster
//!     (emit Decimal128), then map to `numeric` here.
//!   - DBISAM Time decodes to Int64 → surfaces as `bigint`, not `time`.

use arrow::array::{
    Array, ArrayRef, BooleanArray, Date32Array, Float64Array, Int32Array, Int64Array, StringArray,
    TimestampMicrosecondArray,
};
use arrow::datatypes::DataType;
use pgrx::datum::{Date, Timestamp};
use supabase_wrappers::prelude::Cell;

/// Days between the Unix epoch (Arrow Date32 origin, 1970-01-01) and the
/// Postgres epoch (2000-01-01). Arrow stores days since 1970; pgrx `Date`
/// counts from 2000.
const UNIX_TO_PG_EPOCH_DAYS: i32 = 10_957;
/// Microseconds between the Unix and Postgres epochs.
const UNIX_TO_PG_EPOCH_MICROS: i64 = 10_957 * 86_400 * 1_000_000;

/// One value from an Arrow column at `row`, as a Wrappers cell (`None` = NULL).
pub fn array_cell(array: &ArrayRef, row: usize) -> Option<Cell> {
    if array.is_null(row) {
        return None;
    }
    match array.data_type() {
        DataType::Utf8 => downcast::<StringArray>(array).map(|a| Cell::String(a.value(row).to_string())),
        DataType::Boolean => downcast::<BooleanArray>(array).map(|a| Cell::Bool(a.value(row))),
        DataType::Int32 => downcast::<Int32Array>(array).map(|a| Cell::I32(a.value(row))),
        DataType::Int64 => downcast::<Int64Array>(array).map(|a| Cell::I64(a.value(row))),
        DataType::Float64 => downcast::<Float64Array>(array).map(|a| Cell::F64(a.value(row))),
        DataType::Date32 => downcast::<Date32Array>(array).and_then(|a| {
            // Arrow days-since-1970 → pgrx Date (days-since-2000).
            Date::try_from(a.value(row) - UNIX_TO_PG_EPOCH_DAYS).ok().map(Cell::Date)
        }),
        DataType::Timestamp(_, _) => downcast::<TimestampMicrosecondArray>(array).and_then(|a| {
            Timestamp::try_from(a.value(row) - UNIX_TO_PG_EPOCH_MICROS).ok().map(Cell::Timestamp)
        }),
        // Unhandled types fall back to NULL rather than a wrong value. Surfacing
        // these belongs in the strict/lenient decode policy (doc 05) — TODO.
        _ => None,
    }
}

fn downcast<T: 'static>(array: &ArrayRef) -> Option<&T> {
    array.as_any().downcast_ref::<T>()
}

/// PostgreSQL type name for an Arrow column type, used to emit
/// `CREATE FOREIGN TABLE` DDL during `IMPORT FOREIGN SCHEMA`.
pub fn arrow_pg_type(dt: &DataType) -> &'static str {
    match dt {
        DataType::Utf8 => "text",
        DataType::Boolean => "boolean",
        DataType::Int32 => "integer",
        DataType::Int64 => "bigint",
        DataType::Float64 => "double precision", // see fidelity gap: currency → numeric
        DataType::Date32 => "date",
        DataType::Timestamp(_, _) => "timestamp",
        DataType::Binary | DataType::LargeBinary => "bytea",
        _ => "text",
    }
}
