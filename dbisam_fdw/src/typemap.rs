//! DBISAM/Arrow → PostgreSQL type mapping — `proj_init/05-type-mapping.md`.
//!
//! `exportmaster` decodes DBISAM rows into Arrow arrays; this module maps those
//! into Wrappers [`Cell`]s for `iter_scan`, and into PG type names for
//! `IMPORT FOREIGN SCHEMA`. The Arrow types `exportmaster` actually emits
//! (see its `row.rs`) are: Utf8, Int32, Int64, Float64, Decimal128(_,4)
//! (Currency), Boolean, Date32, Time64(µs), Timestamp(µs), Binary.
//!
//! Memo vs binary Blob, and Currency vs Float, are disambiguated via the
//! `exportmaster::DBISAM_TYPE_KEY` field metadata (Currency now arrives as its
//! own Decimal128 type, so it needs no tag).

use arrow::array::{
    Array, ArrayRef, BinaryArray, BooleanArray, Date32Array, Decimal128Array, Float64Array,
    Int32Array, Int64Array, StringArray, Time64MicrosecondArray, TimestampMicrosecondArray,
};
use arrow::datatypes::{DataType, Field};
use pgrx::datum::{AnyNumeric, Date, IntoDatum, Time, Timestamp};
use supabase_wrappers::prelude::Cell;

/// The DBISAM type tag for a column, from the Arrow field metadata exportmaster
/// attaches (`exportmaster::DBISAM_TYPE_KEY`). `None` if absent.
fn dbisam_tag(field: &Field) -> Option<&str> {
    field.metadata().get(exportmaster::DBISAM_TYPE_KEY).map(String::as_str)
}

/// Days between the Unix epoch (Arrow Date32 origin, 1970-01-01) and the
/// Postgres epoch (2000-01-01). Arrow stores days since 1970; pgrx `Date`
/// counts from 2000.
const UNIX_TO_PG_EPOCH_DAYS: i32 = 10_957;
/// Microseconds between the Unix and Postgres epochs.
const UNIX_TO_PG_EPOCH_MICROS: i64 = 10_957 * 86_400 * 1_000_000;

/// One value from an Arrow column at `row`, as a Wrappers cell (`None` = NULL).
/// `field` carries the DBISAM type tag, used to split Memo (text) from a binary
/// Blob/Graphic — both arrive as Arrow Binary.
pub fn array_cell(field: &Field, array: &ArrayRef, row: usize) -> Option<Cell> {
    if array.is_null(row) {
        return None;
    }
    match array.data_type() {
        DataType::Utf8 => downcast::<StringArray>(array).map(|a| Cell::String(a.value(row).to_string())),
        DataType::Boolean => downcast::<BooleanArray>(array).map(|a| Cell::Bool(a.value(row))),
        DataType::Int32 => downcast::<Int32Array>(array).map(|a| Cell::I32(a.value(row))),
        DataType::Int64 => downcast::<Int64Array>(array).map(|a| Cell::I64(a.value(row))),
        DataType::Float64 => downcast::<Float64Array>(array).map(|a| Cell::F64(a.value(row))),
        // DBISAM Currency → Decimal128(_, 4); to PG numeric, losslessly, via the
        // decimal's own string form (no f64 round-trip).
        DataType::Decimal128(_, _) => downcast::<Decimal128Array>(array)
            .and_then(|a| AnyNumeric::try_from(a.value_as_string(row).as_str()).ok())
            .map(Cell::Numeric),
        DataType::Date32 => downcast::<Date32Array>(array).and_then(|a| {
            // Arrow days-since-1970 → pgrx Date (days-since-2000).
            Date::try_from(a.value(row) - UNIX_TO_PG_EPOCH_DAYS).ok().map(Cell::Date)
        }),
        DataType::Timestamp(_, _) => downcast::<TimestampMicrosecondArray>(array).and_then(|a| {
            Timestamp::try_from(a.value(row) - UNIX_TO_PG_EPOCH_MICROS).ok().map(Cell::Timestamp)
        }),
        // DBISAM Time → microseconds since midnight (Arrow Time64), which is
        // exactly pgrx Time's TimeADT representation.
        DataType::Time64(_) => downcast::<Time64MicrosecondArray>(array)
            .and_then(|a| Time::try_from(a.value(row)).ok())
            .map(Cell::Time),
        // Blob/Memo/Graphic resolve to raw bytes (Arrow Binary). A *Memo* is
        // text (doc 05): decode Win-1252 → UTF-8 and surface as a string.
        // Everything else binary stays bytea (lossless).
        DataType::Binary => {
            let a = downcast::<BinaryArray>(array)?;
            let bytes = a.value(row);
            if dbisam_tag(field) == Some("memo") {
                Some(Cell::String(exportmaster::decode_dbisam_text(bytes)))
            } else {
                bytes.into_datum().map(|d| Cell::Bytea(d.cast_mut_ptr()))
            }
        }
        _ => None,
    }
}

fn downcast<T: 'static>(array: &ArrayRef) -> Option<&T> {
    array.as_any().downcast_ref::<T>()
}

/// PostgreSQL type name for a result column, used to emit `CREATE FOREIGN TABLE`
/// DDL during `IMPORT FOREIGN SCHEMA`. Uses the DBISAM tag to split Memo (text)
/// from binary blobs. `None` for an Arrow type we don't map — `array_cell`
/// would only ever yield NULL for it, so the importer skips the table (with a
/// warning) rather than fabricate an all-NULL text column.
pub fn arrow_pg_type(field: &Field) -> Option<&'static str> {
    Some(match field.data_type() {
        DataType::Utf8 => "text",
        DataType::Boolean => "boolean",
        DataType::Int32 => "integer",
        DataType::Int64 => "bigint",
        DataType::Float64 => "double precision",
        DataType::Decimal128(_, _) => "numeric", // DBISAM Currency, lossless
        DataType::Date32 => "date",
        DataType::Time64(_) => "time",
        DataType::Timestamp(_, _) => "timestamp",
        DataType::Binary | DataType::LargeBinary => {
            if dbisam_tag(field) == Some("memo") {
                "text"
            } else {
                "bytea"
            }
        }
        _ => return None,
    })
}
