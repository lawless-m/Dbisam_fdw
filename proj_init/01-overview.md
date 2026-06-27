# 01 — Overview

## What dbisam_fdw is

A read-only PostgreSQL Foreign Data Wrapper that exposes DBISAM 4 tables as
Postgres foreign tables, reading over the native Exportmaster TCP protocol —
no ODBC driver in the path.

## The thesis, in one line

**DBISAM is the dumb, fast leaf; Postgres is the engine on top.**

DBISAM 4's SQL dialect is impoverished — no CTEs, no subqueries, no window
functions, a quirky `TOP` clause, weak join execution. The winning move is
not to make DBISAM do more; it is to put a competent SQL engine in front of
it and let DBISAM do only what it is genuinely good at: hand back filtered,
projected rows, fast. Everything richer — joins, CTEs, subqueries,
aggregates, window functions — is absorbed by Postgres one layer up.

This is the same model Delilah already runs in production with DuckDB as the
engine. dbisam_fdw reproduces it with Postgres as the engine instead. The
advantage carries over for free: any SQL Postgres can parse, it can serve,
and only the innermost table reads turn into DBISAM scans.

## First use case — PowerBI **refresh**

The primary use case is **refreshing PowerBI datasets** (Import mode) from
Exportmaster — PowerBI pulls the curated tables in on a schedule and aggregates
them itself (VertiPaq). Everything is *single-source*: every table comes from
DBISAM, nothing federated alongside it — **not** "every operation runs on the
DBISAM box."

What the FDW gives a refresh is **correct + live + secure**, not fast:

- **Correct.** A clean Postgres endpoint with faithful types — lossless
  `numeric`/currency, real `date`/`time`/`timestamp`, memo→`text` (Win-1252→
  UTF-8), blobs as `bytea` — and no silent row loss (the ODBC driver's bug). The
  type-fidelity work *is* the product here.
- **Live.** A refresh off the FDW sees *current* DBISAM data. This is the one
  thing the daily Parquet dump (`/mnt/.../Outputs/Parquets/em`) structurally
  cannot be. Parquet already owns fast + day-old analytics — that's why the dumps
  exist; the FDW owns the fresher-than-daily / live path, and a boring SQL
  endpoint nobody has to be re-taught to use.
- **Secure.** This may be the deepest reason. Exportmaster has **no TLS and no
  real user/password management** — a fixed Blowfish login (the shared
  `elevatesoft` key) over plaintext TCP; it's a trust-the-LAN protocol that can't
  be safely exposed. Fronting it with Postgres collapses the external surface to a
  *single* endpoint: **PG on 5432, listening on SSL, with real roles and per-table
  GRANTs**. PowerBI (via the on-prem gateway) connects there with proper auth and
  never touches the credential-less DBISAM protocol, which stays internal (PG host
  → DBISAM, on the trusted network). The FDW is as much a security boundary as a
  data path.

Speed is explicitly *not* the goal: a refresh is a background batch, so a 96 s
full scan is fine. Big interactive analytics belong on the Parquet snapshot, not
the live FDW (see `11-aggregate-perf.md`). A refresh that's fine with day-old
data should just import the Parquet and skip DBISAM; the FDW earns its place only
when the refresh needs live data.

**DirectQuery is a secondary, supported mode**, not the primary one. It works for
the same reason: PowerBI emits layered, nested SQL that DBISAM could never parse;
pointed at Postgres, PG swallows the whole structure and only the leaf table reads
become DBISAM scans (exactly as DuckDB does for Delilah). But interactive latency
is bounded by DBISAM's full scans, so DirectQuery is for small/selective/recent
queries, not full-table dashboards.

## Family

| Project | Language | Role |
| --- | --- | --- |
| **MrsFlow** | Rust | Power Query M evaluator; owns the `exportmaster` protocol client and DBISAM SQL emission. |
| **Delilah** | C++ | DuckDB extension; the closest structural analogue to this FDW. |
| **ExportKing** | .NET / ADO.NET | Third protocol implementation; the one that does writes. |
| **Derek** | — | Where the wire protocol was reverse-engineered and documented. |
| **Dibdog** | Prolog DCG | Authoritative DBISAM SQL grammar; the oracle for what is emittable. |

dbisam_fdw is a fourth consumer of the same protocol. It shares the protocol
core and the grammar; it does **not** share a codebase with any of them.

## Why native protocol, not ODBC

The Elevate Software DBISAM ODBC driver is Windows-only, has compounding bugs in
its bulk-fetch path that cause silent row loss (see MrsFlow `KNOWN_BUGS.md §B1`),
**and is slower** than the native cursor protocol (confirmed in practice — the
ODBC fetch path takes longer than the exportmaster path the FDW uses). So native
wins on three axes: correctness, deployment, and speed. Delilah and dbisam_fdw
both bypass ODBC entirely by speaking the wire protocol over TCP. The deployment
win: the FDW just opens a socket to the DBISAM server (default port 12005), so
Postgres on Debian is fine — there is no in-process Windows driver to host.
