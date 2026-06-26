//! Connection handling — `proj_init/06-connection-broker.md`.
//!
//! Per-backend session reuse is non-negotiable and cheap: a backend opens one
//! Exportmaster session and reuses it across every scan it serves, rather than
//! reconnecting per scan. Build this from the start.
//!
//! The broker-vs-serialise decision for DirectQuery login-storm survival
//! (06 §"Options to evaluate", open question Q4) is deferred to a measured
//! experiment and does not block milestone 1.

use exportmaster::{Client, ConnOpts};

/// Resolved connection parameters for one foreign server, plus the lazily
/// opened, reused session.
pub(crate) struct DbisamConn {
    opts: ConnOpts,
    client: Option<Client>,
}

impl DbisamConn {
    /// Build from foreign-server / user-mapping options (`host`, `port`,
    /// `user`, `password`, `catalog`, `compression`, `batch_size`, …).
    pub(crate) fn from_options(_opts: ()) -> Self {
        // TODO: parse Wrappers `Vec<Option<String>>` / server options into
        // ConnOpts. Placeholder shape only.
        todo!("map foreign-server options -> exportmaster::ConnOpts")
    }

    /// The reused session, opened on first use (06 §"Per-backend reuse").
    pub(crate) fn client(&mut self) -> &mut Client {
        if self.client.is_none() {
            self.client = Some(
                Client::connect_and_login(&self.opts).expect("exportmaster login"),
            );
        }
        self.client.as_mut().unwrap()
    }
}

/// Per-scan cursor state: the SQL issued and the Arrow batch being walked.
pub(crate) struct ScanState {
    pub sql: String,
    pub batch: arrow::record_batch::RecordBatch,
    pub next_row: usize,
}
