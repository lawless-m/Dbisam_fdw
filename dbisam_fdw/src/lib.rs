//! `dbisam_fdw` — a read-only PostgreSQL Foreign Data Wrapper for DBISAM 4,
//! speaking the native Exportmaster TCP protocol (no ODBC in the path).
//!
//! Milestone 1 (`proj_init/02-scope-v1.md`): scan / projection / filter / limit
//! are pushed down to DBISAM; Postgres owns joins, CTEs, subqueries, window
//! functions and aggregates. See `03-architecture.md` for the crate split
//! (`exportmaster` protocol client + `dbisam-sql` renderer + this glue).
//!
//! Read-only: the write callbacks (`begin_modify`/`insert`/`update`/`delete`)
//! are overridden to error. Under Supabase Wrappers the modify methods have
//! no-op default impls, so "read-only by absence" (the raw-FDW contract in doc
//! 02) is expressed here as explicit rejection — same guarantee, Wrappers-shaped.

use std::collections::HashMap;

use pgrx::pg_sys::panic::ErrorReport;
use pgrx::PgSqlErrorCode;
use supabase_wrappers::prelude::*;

use exportmaster::{Client, ConnOpts};

mod quals;
mod schema_import;
mod typemap;

pgrx::pg_module_magic!();

/// FDW error. Converts into a Postgres `ErrorReport` with the FDW error code.
#[derive(Debug)]
enum DbisamFdwError {
    /// A required/invalid foreign-server or table option.
    Options(String),
    /// A failure from the Exportmaster protocol client.
    Protocol(String),
    /// A write was attempted against this read-only FDW.
    ReadOnly,
}

impl From<exportmaster::IoError> for DbisamFdwError {
    fn from(e: exportmaster::IoError) -> Self {
        DbisamFdwError::Protocol(e.to_string())
    }
}

impl From<DbisamFdwError> for ErrorReport {
    fn from(e: DbisamFdwError) -> Self {
        let msg = match &e {
            DbisamFdwError::Options(s) => format!("dbisam_fdw option error: {s}"),
            DbisamFdwError::Protocol(s) => format!("dbisam_fdw protocol error: {s}"),
            DbisamFdwError::ReadOnly => {
                "dbisam_fdw is read-only; DML is not supported".to_string()
            }
        };
        ErrorReport::new(PgSqlErrorCode::ERRCODE_FDW_ERROR, msg, "")
    }
}

type DbisamFdwResult<T> = Result<T, DbisamFdwError>;

/// One FDW instance per foreign scan. The whole result set is materialised in
/// `begin_scan` as an Arrow `RecordBatch` (DBISAM returns it fast once filtered
/// and projected) and walked row-by-row in `iter_scan`. Per-backend session
/// reuse / streaming is a later refinement (`06-connection-broker.md`).
#[wrappers_fdw(
    version = "0.1.0",
    author = "Ramsden International",
    website = "https://github.com/lawless-m/Dbisam_fdw",
    error_type = "DbisamFdwError"
)]
pub(crate) struct DbisamFdw {
    opts: ConnOpts,
    batch: Option<arrow::record_batch::RecordBatch>,
    row_idx: usize,
    tgt_cols: Vec<Column>,
}

impl DbisamFdw {
    /// Build `ConnOpts` from the merged foreign-server + user-mapping options.
    fn conn_opts(options: &HashMap<String, String>) -> DbisamFdwResult<ConnOpts> {
        let host = options
            .get("host")
            .ok_or_else(|| DbisamFdwError::Options("server option `host` is required".into()))?;
        let user = options.get("user").cloned().unwrap_or_default();
        let password = options.get("password").cloned().unwrap_or_default();
        let mut opts = ConnOpts::new(host, user, password);
        if let Some(p) = options.get("port").and_then(|s| s.parse().ok()) {
            opts.port = p;
        }
        if let Some(c) = options.get("catalog") {
            opts.catalog = c.clone();
        }
        if let Some(c) = options.get("compression").and_then(|s| s.parse().ok()) {
            opts.compression = c;
        }
        if let Some(b) = options.get("batch_size").and_then(|s| s.parse().ok()) {
            opts.batch_size = b;
        }
        Ok(opts)
    }
}

impl ForeignDataWrapper<DbisamFdwError> for DbisamFdw {
    fn new(server: ForeignServer) -> DbisamFdwResult<Self> {
        let opts = Self::conn_opts(&server.options)?;
        Ok(Self {
            opts,
            batch: None,
            row_idx: 0,
            tgt_cols: Vec::new(),
        })
    }

    fn begin_scan(
        &mut self,
        quals: &[Qual],
        columns: &[Column],
        _sorts: &[Sort],
        limit: &Option<Limit>,
        options: &HashMap<String, String>,
    ) -> DbisamFdwResult<()> {
        let table = options
            .get("table")
            .ok_or_else(|| DbisamFdwError::Options("foreign table option `table` is required".into()))?;

        // Projection: only the requested columns travel the wire.
        //
        // PK auto-injection (doc 05): blob/memo resolution reconstructs each
        // row's OpenBlob slot from the *first* projected column, which must be
        // the table's PK. So when a `pk` table option is set we prepend it
        // (deduped). If the PK wasn't requested it's fetched but ignored on
        // output (iter_scan looks columns up by name). IMPORT FOREIGN SCHEMA
        // sets `pk` automatically; set it by hand for tables with memo/blob
        // columns created manually.
        let pk = options.get("pk").map(String::as_str);
        let projection = if columns.is_empty() {
            "*".to_string()
        } else {
            let mut names: Vec<&str> = Vec::with_capacity(columns.len() + 1);
            if let Some(pk) = pk {
                names.push(pk);
            }
            for c in columns {
                if Some(c.name.as_str()) != pk {
                    names.push(c.name.as_str());
                }
            }
            names.iter().map(|n| dbisam_sql::quote_ident(n)).collect::<Vec<_>>().join(", ")
        };

        // Filter: render the foldable subset; the rest is rechecked by Postgres.
        let preds = quals::to_preds(quals);
        let where_clause = dbisam_sql::render_where(&preds);
        // TOP is only safe to push when *every* qual was pushed — a non-foldable
        // qual rechecked above the scan would make TOP cap the wrong count
        // (04 §"Limit edge case").
        let all_pushed = preds.iter().all(|p| p.render().is_some());

        let mut sql = format!("SELECT {projection} FROM {}", dbisam_sql::quote_ident(table));
        if let Some(w) = &where_clause {
            sql.push_str(" WHERE ");
            sql.push_str(w);
        }
        if let Some(lim) = limit {
            if all_pushed {
                // Fetch offset+count; Postgres applies the OFFSET itself.
                let n = (lim.offset + lim.count).max(0) as u64;
                sql.push(' ');
                sql.push_str(&dbisam_sql::top_clause(n));
            }
        }

        // The exact DBISAM SQL we push (projection + foldable WHERE + TOP).
        // Visible with `SET client_min_messages = 'debug1'`.
        pgrx::debug1!("dbisam_fdw push: {sql}");

        let mut client = Client::connect_and_login(&self.opts)?;
        let batch = client.query_to_table(&sql)?;
        self.batch = Some(batch);
        self.row_idx = 0;
        self.tgt_cols = columns.to_vec();
        Ok(())
    }

    fn iter_scan(&mut self, row: &mut Row) -> DbisamFdwResult<Option<()>> {
        let Some(batch) = &self.batch else {
            return Ok(None);
        };
        if self.row_idx >= batch.num_rows() {
            return Ok(None);
        }
        let r = self.row_idx;
        let schema = batch.schema();
        for col in &self.tgt_cols {
            let cell = match schema.index_of(&col.name) {
                Ok(idx) => typemap::array_cell(schema.field(idx), batch.column(idx), r),
                Err(_) => None,
            };
            row.push(&col.name, cell);
        }
        self.row_idx += 1;
        Ok(Some(()))
    }

    fn end_scan(&mut self) -> DbisamFdwResult<()> {
        self.batch = None;
        self.tgt_cols.clear();
        Ok(())
    }

    fn import_foreign_schema(
        &mut self,
        stmt: ImportForeignSchemaStmt,
    ) -> DbisamFdwResult<Vec<String>> {
        schema_import::import(&self.opts, &stmt)
    }

    // ---- read-only: reject every write path explicitly ----

    fn begin_modify(&mut self, _options: &HashMap<String, String>) -> DbisamFdwResult<()> {
        Err(DbisamFdwError::ReadOnly)
    }

    fn insert(&mut self, _row: &Row) -> DbisamFdwResult<()> {
        Err(DbisamFdwError::ReadOnly)
    }

    fn update(&mut self, _rowid: &Cell, _new_row: &Row) -> DbisamFdwResult<()> {
        Err(DbisamFdwError::ReadOnly)
    }

    fn delete(&mut self, _rowid: &Cell) -> DbisamFdwResult<()> {
        Err(DbisamFdwError::ReadOnly)
    }
}
