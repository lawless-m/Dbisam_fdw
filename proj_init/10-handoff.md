# 10 — Autonomous task-loop handoff

> What the `/task-loop` run completed while Matt was out, and the items that
> still need his decision. All four work tasks completed; none halted.

## Completed (this run)

| # | Outcome | Commits |
| --- | --- | --- |
| 1 | **Dbisam_fdw now depends on the published `exportmaster` git crate**, not a local path — a fresh clone builds standalone. `.cargo/config.toml` sets `git-fetch-with-cli` so the `github-lawless` SSH alias resolves. | Dbisam_fdw `60d2498` |
| 2 | **Reserved-word column quoting.** `dbisam_sql::quote_ident` now double-quotes the ~95 DBISAM keywords (sourced from Dibdog's `keyword//` terminals) even when character-valid; non-reserved names stay bare. 19 renderer tests. | Dbisam_fdw `a799a91` |
| 3 | **Session-reuse (0x2C2C) investigated** → `proj_init/09-session-reuse.md`. `0x2C2C` = "request before login": the server drops login state per statement lifecycle, so each query needs a fresh login. Likely a DBISAM connection can't amortise its login across queries. | Dbisam_fdw `6c920d0` |
| 4 | **DBISAM Time → PG `time`.** exportmaster emits `Time64(µs)` (was `Int64`); the FDW maps it to `Cell::Time`. exportmaster 46 tests pass, FDW builds clean. Live step skipped — no populated `ftTime` column in 14 sampled tables. | exportmaster `043f8ad`, `9a8288b`; Dbisam_fdw `c50f15c` |

State at handoff: all milestone-1 type fidelity gaps from doc 05 are closed
(numeric/currency, text/memo, date, time, blob); pushdown covers comparisons,
IN/NOT IN, prefix LIKE, IS [NOT] NULL, and date/timestamp; identifier quoting is
Dibdog-correct incl. reserved words. The pgrx-managed PG15 instance (port 28815)
has the extension installed and was used for live verification throughout.

## Needs Matt's decision (not autonomously doable)

- **Connection: broker vs serialise (doc 06, Q4).** *The* milestone-1 blocker for
  a real PowerBI DirectQuery page. Sharpened by `09-session-reuse.md`: if a DBISAM
  connection can't reuse a login across queries (likely), a reusable-session pool
  buys little and the lever is **serialising/rate-limiting logins**. Run the two
  experiments in `09` (lighter teardown; re-login on same socket) to confirm
  before committing to a design.
- **Eager-schema default for the DirectQuery profile (doc 05 §schema; Q3 in
  `07`).** PowerBI's modelling view wants up-front column introspection; lazy is
  the better default for query work. Decide whether the connection profile sets
  eager automatically (must stay serial — interacts with the login-storm limit).
- **Aggregate (single-table GROUP BY) pushdown — phase 2 (doc 02; Q5 in `07`).**
  Deferred out of milestone 1. Trigger is *measured* evidence that pulling
  filtered tables up for PG to aggregate is too slow on real visuals. May need
  raw pgrx for `GetForeignUpperPaths`. Joins still never go down.

## Smaller follow-ups (could be looped later)

- **Reserved-word list completeness.** Task #2 used Dibdog's current `keyword//`
  terminals. If Dibdog later adds keywords, re-sync `RESERVED` in
  `dbisam-sql/src/lib.rs` (the real anti-drift home is Dibdog).
- **Live Time verification.** `ftTime` is rare in NISAINT_CS; verify the
  `Time64 → time` path end-to-end if/when a populated time column turns up.
- **Repoint check.** MrsFlow and Dbisam_fdw both now use the git-dep exportmaster;
  the dep is pinned per-repo via Cargo.lock, so bump deliberately when the crate
  advances.
