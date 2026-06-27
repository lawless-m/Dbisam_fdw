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

## First use case

DirectQuery connections from **app.powerbi.com** pointed purely into
Exportmaster. "Purely" means *single-source* — every table in the report
comes from DBISAM, nothing federated alongside it — **not** "every operation
runs on the DBISAM box."

This use case is why the FDW is the right shape. PowerBI DirectQuery does not
emit tidy SQL; it generates layered subqueries and derived tables, often
nested several levels deep. Pointed straight at DBISAM that SQL fails on
contact — DBISAM cannot parse it. Pointed at Postgres, PG swallows the whole
nested structure and only the leaf table reads become DBISAM scans. The
complexity PowerBI generates that DBISAM could never handle gets absorbed by
the engine above, exactly as it does in DuckDB today.

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
