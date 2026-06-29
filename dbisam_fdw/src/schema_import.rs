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
use supabase_wrappers::prelude::ImportForeignSchemaStmt;

use crate::{DbisamFdwError, DbisamFdwResult};

pub fn import(opts: &ConnOpts, stmt: &ImportForeignSchemaStmt) -> DbisamFdwResult<Vec<String>> {
    let names = select_tables(opts, stmt)?;
    let local = &stmt.local_schema;
    let server = &stmt.server_name;

    let mut ddl = Vec::with_capacity(names.len());
    for table in names {
        ddl.push(table_ddl(opts, local, server, &table)?);
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
fn select_tables(opts: &ConnOpts, stmt: &ImportForeignSchemaStmt) -> DbisamFdwResult<Vec<String>> {
    let mut client = Client::connect_and_login(opts)?;
    let all = client.list_tables()?;
    drop(client);

    let Some(dir) = stmt.options.get("parquet_dir") else {
        return Ok(all); // no curation requested — whole catalogue
    };

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

    Ok(all
        .into_iter()
        .filter(|t| stems.contains(&t.to_ascii_lowercase()))
        .collect())
}

/// Probe one table's live schema and render its `CREATE FOREIGN TABLE` DDL.
fn table_ddl(
    opts: &ConnOpts,
    local: &str,
    server: &str,
    table: &str,
) -> DbisamFdwResult<String> {
    // Fresh session per probe (serial) — matches the proven usage pattern.
    let mut probe = Client::connect_and_login(opts)?;
    let batch = probe
        .query_to_table_capped(&format!("SELECT * FROM \"{table}\" WHERE 1=0"), 0)
        .map_err(|e| DbisamFdwError::Protocol(format!("probe {table}: {e}")))?;
    let schema = batch.schema();
    // DBISAM is case-insensitive and never committed to an identifier scheme, so
    // the probed case is arbitrary noise. Fold every PG-facing identifier (table,
    // columns, pk) to lowercase so callers write natural unquoted SQL instead of
    // quoting the captured case. The `table` option keeps the probed name —
    // DBISAM ignores its case, and the FDW resolves results case-insensitively.
    let cols = schema
        .fields()
        .iter()
        .map(|f| format!("\"{}\" {}", f.name().to_ascii_lowercase(), crate::typemap::arrow_pg_type(f.as_ref())))
        .collect::<Vec<_>>()
        .join(", ");
    // Column 0 is the DBISAM PK (protocol §4); emit it as the `pk` option so the
    // FDW can auto-inject it for blob/memo resolution.
    let pk_opt = schema
        .fields()
        .first()
        .map(|f| format!(", pk '{}'", f.name().to_ascii_lowercase()))
        .unwrap_or_default();
    let pg_table = table.to_ascii_lowercase();
    Ok(format!(
        "CREATE FOREIGN TABLE IF NOT EXISTS {local}.\"{pg_table}\" ({cols}) \
         SERVER {server} OPTIONS (table '{table}'{pk_opt})"
    ))
}
