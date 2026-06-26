//! `IMPORT FOREIGN SCHEMA` — reflect the DBISAM catalogue into foreign tables.
//! Mirrors Delilah's catalog enumeration; see `proj_init/05-type-mapping.md`
//! §"Schema introspection: lazy vs eager".
//!
//! Lazy by default: listing the catalogue needs only table names
//! (`exportmaster::Client::list_tables`), so column probing is deferred —
//! enumeration stays instant on catalogues with hundreds of tables. The cost:
//! catalogue-wide column introspection (`information_schema.columns`, a GUI
//! column browser) reports empty until a table is queried.
//!
//! Eager opt-in (`EAGER_SCHEMA`-equivalent): probe every table once on first
//! catalogue access and cache for the session. Must stay **serial** — the
//! server rejects concurrent login storms (~15 s for ~600 tables in Delilah).
//! PowerBI's modelling view does up-front introspection, so the DirectQuery
//! setup path will likely want eager even though lazy is the better default
//! for query work (open question Q3).

use crate::connection::DbisamConn;

/// Produce the `CREATE FOREIGN TABLE` statements for the imported schema.
/// Each table's columns are typed via [`crate::typemap`] from a zero-row
/// `SELECT * FROM <t> WHERE 1=0` probe (cheap — returns the schema without
/// driving the cursor).
pub(crate) fn import(_conn: &mut DbisamConn, _stmt: ()) -> Vec<String> {
    // TODO: list_tables() -> for each (eager) or on demand (lazy): probe
    // schema -> typemap::schema_type per column -> emit CREATE FOREIGN TABLE.
    todo!("emit CREATE FOREIGN TABLE DDL from the DBISAM catalogue")
}
