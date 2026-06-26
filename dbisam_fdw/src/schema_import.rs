//! `IMPORT FOREIGN SCHEMA` — reflect the DBISAM catalogue into foreign tables.
//! See `proj_init/05-type-mapping.md` §"Schema introspection".
//!
//! Import is inherently eager: emitting `CREATE FOREIGN TABLE` needs each
//! table's column list, so we probe every table with a zero-row
//! `SELECT * … WHERE 1=0` (returns the schema without driving the cursor).
//! Probes run serially — the server rejects concurrent login storms
//! (`06-connection-broker.md`).

use exportmaster::{Client, ConnOpts};
use supabase_wrappers::prelude::ImportForeignSchemaStmt;

use crate::{DbisamFdwError, DbisamFdwResult};

pub fn import(opts: &ConnOpts, stmt: &ImportForeignSchemaStmt) -> DbisamFdwResult<Vec<String>> {
    let mut client = Client::connect_and_login(opts)?;
    let names = client.list_tables()?;
    drop(client);

    let local = &stmt.local_schema;
    let server = &stmt.server_name;

    let mut ddl = Vec::with_capacity(names.len());
    for table in names {
        // Fresh session per probe (serial) — matches the proven usage pattern.
        let mut probe = Client::connect_and_login(opts)?;
        let batch = probe
            .query_to_table_capped(&format!("SELECT * FROM \"{table}\" WHERE 1=0"), 0)
            .map_err(|e| DbisamFdwError::Protocol(format!("probe {table}: {e}")))?;
        let schema = batch.schema();
        let cols = schema
            .fields()
            .iter()
            .map(|f| format!("\"{}\" {}", f.name(), crate::typemap::arrow_pg_type(f.data_type())))
            .collect::<Vec<_>>()
            .join(", ");
        ddl.push(format!(
            "CREATE FOREIGN TABLE IF NOT EXISTS {local}.\"{table}\" ({cols}) \
             SERVER {server} OPTIONS (table '{table}')"
        ));
    }
    Ok(ddl)
}
