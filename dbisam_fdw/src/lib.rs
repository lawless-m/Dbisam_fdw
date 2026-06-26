//! `dbisam_fdw` — a read-only PostgreSQL Foreign Data Wrapper for DBISAM 4,
//! speaking the native Exportmaster TCP protocol (no ODBC in the path).
//!
//! Milestone 1 (see `proj_init/02-scope-v1.md`): **scan pushdown down, rich SQL
//! up, single source.** Only scan / projection / filter / limit are pushed to
//! DBISAM; Postgres owns joins, CTEs, subqueries, window functions and
//! aggregates. Read-only is enforced *structurally* by the absence of the write
//! callbacks — there is no `PlanForeignModify` / `ExecForeignInsert` here, so
//! Postgres rejects DML at plan time and no code path to writes exists.
//!
//! Architecture (`proj_init/03-architecture.md`):
//! - [`exportmaster`] — the protocol client. Reused as-is; zero new wire code.
//! - [`dbisam_sql`](dbisam_sql) — quals → DBISAM `WHERE` renderer + the four
//!   dialect quirks. Shared rules, kept off the pgrx types so they unit-test
//!   without a Postgres toolchain.
//! - this crate — the Supabase Wrappers glue: catalog import, scan with
//!   projection/filter/limit pushdown, type mapping, connection handling.
//!
//! STATUS: skeleton. The trait method bodies are `todo!()` and the exact
//! Supabase Wrappers trait signatures must be reconciled against the version
//! `cargo pgrx init` installs (see README.md) — this file fixes the *shape*,
//! not the final API surface.

use pgrx::prelude::*;

mod connection;
mod schema_import;
mod typemap;

pgrx::pg_module_magic!();

/// The FDW instance. One per foreign server; holds a borrowed/owned
/// Exportmaster session for the backend it runs in (per-backend session reuse
/// is mandatory — `proj_init/06-connection-broker.md`).
pub(crate) struct DbisamFdw {
    /// Connection parameters resolved from the foreign-server / user-mapping
    /// options. The live [`exportmaster::Client`] is opened lazily and reused
    /// across the scans this backend serves.
    conn: connection::DbisamConn,
    /// The SQL built for the current scan (SELECT + projection + WHERE + TOP),
    /// and the row cursor over the returned Arrow batch.
    scan: Option<connection::ScanState>,
}

// The `#[wrappers_fdw]` macro + `ForeignDataWrapper` impl go here once the
// Wrappers version is pinned. Sketch of the milestone-1 surface:
//
//   #[wrappers_fdw(version = "0.1.0", author = "...", ...)]
//   impl ForeignDataWrapper<DbisamFdwError> for DbisamFdw {
//       fn new(server: ForeignServer) -> Result<Self, _> { connection::open(server) }
//
//       fn begin_scan(&mut self, quals, columns, _sorts, limit, _options) {
//           // 1. map Wrappers quals -> dbisam_sql::Pred  (typemap/adapter)
//           // 2. let where_ = dbisam_sql::render_where(&preds);   // foldable subset
//           // 3. let top = limit.map(|l| dbisam_sql::top_clause(l.count))  // quirk #1,
//           //    only when no non-pushable qual sits above the scan (04 §Limit edge case)
//           // 4. build "SELECT <columns> FROM <tbl> [WHERE where_] [TOP n]"
//           // 5. self.conn.client().query_to_table_streaming(sql, target)
//       }
//
//       fn iter_scan(&mut self, row: &mut Row) -> Option<()> {
//           // pull next decoded row from the Arrow batch; map CellValue ->
//           // Cell via typemap; None at end.
//       }
//
//       fn end_scan(&mut self) { self.scan = None; }
//
//       fn import_foreign_schema(&mut self, stmt) -> Vec<String> {
//           schema_import::import(&mut self.conn, stmt)  // CREATE FOREIGN TABLE DDL
//       }
//
//       // No PlanForeignModify / ExecForeignInsert/Update/Delete — read-only
//       // by absence (02 §"Read-only as structure").
//   }
