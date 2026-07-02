//! `IMPORT FOREIGN SCHEMA` — reflect the DBISAM catalogue into foreign tables.
//! See `proj_init/05-type-mapping.md` §"Schema introspection".
//!
//! `list_tables` is NOT a stable schema: the DBISAM catalogue is mostly volatile
//! `wk<hex>` scratch tables (≈357 of 626 on NISAINT_CS, drifting run-to-run).
//! So importing the *whole* catalogue is wrong. With a `parquet_dir` import
//! option, curate the table list to the daily Parquet dump's filenames — the
//! authoritative "useful tables" set — and only those get imported.
//!
//! Either way, each table's *columns* come from the live `SELECT * … WHERE 1=0`
//! probe (the engine's real wire schema), never a registry table like DataDict.
//! Probes run serially (the server refuses concurrent connection storms,
//! `06`/`09`).

use std::collections::HashSet;
use std::fs;

use exportmaster::{Client, ConnOpts};
use supabase_wrappers::prelude::{ImportForeignSchemaStmt, ImportSchemaType};

use crate::{DbisamFdwError, DbisamFdwResult};

pub fn import(opts: &ConnOpts, stmt: &ImportForeignSchemaStmt) -> DbisamFdwResult<Vec<String>> {
    let names = select_tables(opts, stmt)?;
    let local = &stmt.local_schema;
    let server = &stmt.server_name;

    let mut ddl = Vec::with_capacity(names.len());
    for table in names {
        if let Some(stmt_ddl) = table_ddl(opts, local, server, &table)? {
            ddl.push(stmt_ddl);
        }
    }
    Ok(ddl)
}

/// Pick which DBISAM tables to import.
///
/// With `OPTIONS (parquet_dir '…')`, curate to the tables that have a
/// `<name>.parquet` file in that directory (case-insensitive). The real DBISAM
/// name and its case come from `list_tables`, so this both filters to the useful
/// set and drops dump-only synthetic tables (e.g. `ri…std…`) that aren't real
/// DBISAM tables. Without the option, fall back to the full (volatile)
/// catalogue.
///
/// `LIMIT TO` / `EXCEPT` are applied here too (case-insensitively — the PG
/// list is lowercased, DBISAM names are arbitrary-case). Postgres re-filters
/// the DDL we return anyway, but honouring the list up front avoids a serial
/// connect+probe against every excluded table (the `09` storm concern).
fn select_tables(opts: &ConnOpts, stmt: &ImportForeignSchemaStmt) -> DbisamFdwResult<Vec<String>> {
    let mut client = Client::connect_and_login(opts)?;
    let all = client.list_tables()?;
    drop(client);

    let curated = match stmt.options.get("parquet_dir") {
        None => all, // no curation requested — whole catalogue
        Some(dir) => {
            let stems: HashSet<String> = fs::read_dir(dir)
                .map_err(|e| DbisamFdwError::Options(format!("parquet_dir {dir}: {e}")))?
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let p = e.path();
                    if p.extension().and_then(|x| x.to_str()) != Some("parquet") {
                        return None;
                    }
                    p.file_stem().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase())
                })
                .collect();

            all.into_iter()
                .filter(|t| stems.contains(&t.to_ascii_lowercase()))
                .collect()
        }
    };

    let listed = |t: &String| stmt.table_list.iter().any(|n| n.eq_ignore_ascii_case(t));
    Ok(match stmt.list_type {
        ImportSchemaType::FdwImportSchemaAll => curated,
        ImportSchemaType::FdwImportSchemaLimitTo => curated.into_iter().filter(listed).collect(),
        ImportSchemaType::FdwImportSchemaExcept => {
            curated.into_iter().filter(|t| !listed(t)).collect()
        }
    })
}

/// Probe one table's live schema and render its `CREATE FOREIGN TABLE` DDL.
/// `Ok(None)` means the table was skipped (with a warning): the probe failed —
/// the volatile catalogue can drop a table between `list_tables` and its probe,
/// and one stale entry must not fail the whole IMPORT — or a column has an
/// Arrow type we don't map. Connection failures still abort: if the server is
/// unreachable, every remaining probe would fail too.
fn table_ddl(
    opts: &ConnOpts,
    local: &str,
    server: &str,
    table: &str,
) -> DbisamFdwResult<Option<String>> {
    // Fresh session per probe (serial) — matches the proven usage pattern.
    let mut probe = Client::connect_and_login(opts)?;
    let sql = format!("SELECT * FROM {} WHERE 1=0", dbisam_sql::quote_ident(table));
    let batch = match probe.query_to_table_capped(&sql, 0) {
        Ok(batch) => batch,
        Err(e) => {
            pgrx::warning!("dbisam_fdw import: skipping table \"{table}\": probe failed: {e}");
            return Ok(None);
        }
    };
    let schema = batch.schema();
    // DBISAM is case-insensitive and never committed to an identifier scheme, so
    // the probed case is arbitrary noise. Fold every PG-facing identifier (table,
    // columns, pk) to lowercase so callers write natural unquoted SQL instead of
    // quoting the captured case. The `table` option keeps the probed name —
    // DBISAM ignores its case, and the FDW resolves results case-insensitively.
    let mut cols = Vec::with_capacity(schema.fields().len());
    for f in schema.fields() {
        let Some(ty) = crate::typemap::arrow_pg_type(f.as_ref()) else {
            pgrx::warning!(
                "dbisam_fdw import: skipping table \"{table}\": column \"{}\" has unmapped Arrow type {:?}",
                f.name(),
                f.data_type()
            );
            return Ok(None);
        };
        cols.push(format!("{} {ty}", pg_ident(&f.name().to_ascii_lowercase())));
    }
    let cols = cols.join(", ");
    // Column 0 is the DBISAM PK (protocol §4); emit it as the `pk` option so the
    // FDW can auto-inject it for blob/memo resolution.
    let pk_opt = schema
        .fields()
        .first()
        .map(|f| format!(", pk {}", pg_literal(&f.name().to_ascii_lowercase())))
        .unwrap_or_default();
    Ok(Some(format!(
        "CREATE FOREIGN TABLE IF NOT EXISTS {}.{} ({cols}) SERVER {} OPTIONS (table {}{pk_opt})",
        pg_ident(local),
        pg_ident(&table.to_ascii_lowercase()),
        pg_ident(server),
        pg_literal(table),
    )))
}

/// Quote a PostgreSQL identifier (always-quoted form; embedded `"` doubled).
fn pg_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Quote a PostgreSQL string literal (e.g. an OPTIONS value); `'` doubled.
fn pg_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
