# 09 — Exportmaster session reuse (0x2C2C)

> Investigation of why a second query on one Exportmaster session fails, and what
> it means for the connection design (doc 06, Q4). Findings only — no fix applied.

## The symptom

Issuing a second query on a single `exportmaster::Client` fails:

```
PrepareStatement: server error reqcode 0x2C2C
```

The first query succeeds; the second `PrepareStatement` (reqcode `0x0320`) is
rejected. Observed live (rivsem04) while scanning the catalogue in the
`find_memo` example, which is why each query currently needs a fresh
`connect_and_login`.

## What 0x2C2C means

`0x2C2C` is a **"request before login"** session error. From
`exportmaster/src/response.rs` (`check_body_reqcode`, decoded live against
rivsem01):

> `0x2Cxx` session errors — `0x2C17` = login rejected, `0x2C1E` = catalog
> attach failed, **`0x2C2C` = request before login**. (`0x2C14` is the polling
> sentinel, not an error.)

So the server is saying: *you sent a statement request on a connection it does
not consider logged in.* The whole `0x2Cxx` family is the server's session-state
gate; `0x2C2C` is specifically the "no valid login" rung.

## Why this is surprising

The post-query teardown sends **no logout**. There is no logout/disconnect
message anywhere in the protocol implementation — a session ends only by
dropping the TCP socket (`Client` drop). The teardown (`client.rs`, both query
paths) is just:

1. `CloseCursor` (0x00A0) — release the cursor,
2. `ResetStatement` — reset the prepared statement,
3. `RemoveAllRemoteMemoryTables` (0x0029) — drop server-side temp tables.

None of these is a logout. Yet after them, the *same TCP connection* answers the
next `PrepareStatement` with "request before login". So the server drops the
login state as part of the statement lifecycle — not because we asked it to.

The login that establishes the session is `connect → login (0x0014) →
session-setup (C2, C3, catalog-attach 0x003c, C5)` in `connect_and_login`.

## Experiment results (run live against rivsem04)

Driven by `exportmaster/examples/session_reuse_probe.rs` (query =
`SELECT CODE, PRICE FROM PRODUCT TOP 2`):

| Experiment | Query 2 outcome |
| --- | --- |
| **Baseline** — 2 queries on one `Client`, default teardown | **FAIL `0x2C2C`** (confirms the symptom) |
| **E1** — query 1 teardown *minus* `RemoveAllRemoteMemoryTables` (0x0029) | **FAIL `0x2C2C`** — the release is **not** the culprit |
| **E2** — `reauth` (re-login + session-setup) on the *same socket*, then query 2 | **OK (2 rows)** — a socket can carry many queries |

So:

- **Hypothesis 1 (confirmed): the server drops login state per statement
  lifecycle.** A completed prepare/execute/cursor cycle returns the connection to
  pre-login; the next `PrepareStatement` needs a fresh login.
- **Hypothesis 2 (rejected):** skipping the temp-table release changes nothing.
- The decisive new fact: **a fresh login on the *existing socket* (no TCP
  reconnect, no repeat of the one-time Connect handshake) is accepted and lets
  the next query run.** A TCP connection is reusable; a *login* is not.

This is exposed as `Client::reauth(&opts)` in exportmaster.

## What this means

Two costs were conflated. Re-login on a kept socket separates them:

- **TCP connect + Connect handshake** — one-time per socket; **reusable** (E2).
- **Login (0x0014) + session-setup** — required **per query**; *not* reusable.

So **per-backend socket reuse is real and worth doing** (keep the socket, call
`reauth` before each query — saves the TCP connect/handshake per query). But it
does **not** reduce the login *count*: N queries = N logins regardless. The
login-storm constraint is about *concurrent* logins across backends, which socket
reuse does not address.

## Login-storm measurement (live, single run)

`exportmaster/examples/login_storm.rs` ramps concurrency: N threads barrier-synced
to fire `connect_and_login` + a trivial query at once.

| Concurrency | Logins OK | Failure |
| --- | --- | --- |
| 2 | 2/2 | — (first wave wall 10s — cold-start artifact) |
| 4 | 4/4 | — |
| 8 | 6/8 | 2× **Connection refused (os 111)** |
| 16 | 13/16 | 3× Connection refused |
| 32 | 19/32 | 13× Connection refused |

The failure is **TCP connection-refused**, *not* a DBISAM login-rejection
reqcode (no `0x2C17`). So "the server rejects login storms" really means its
TCP accept path (small listen backlog / serial accept) drops connections under a
concurrent-*connect* burst. Clean to ~4 concurrent; failures begin at 8 and
reach ~40% at 32. An 8-visual DirectQuery page that opens 8 fresh connections at
once would already lose ~25%.

(Single run against production — not repeated, to avoid disrupting real users.)

## Recommendation for doc 06 (Q4: broker vs serialise)

The two findings together point to one answer:

- E2: a **warm TCP socket** is reusable for many queries (re-login per query, no
  reconnect).
- Storm: the thing that breaks under load is **opening fresh connections
  concurrently** (TCP refused at ~8+), not the logins themselves.

So the design that avoids the failure mode entirely is a **broker that owns a
small bounded pool of persistent (warm) sockets** — open them once, keep them,
and serve each query by `reauth` + query over a borrowed warm socket. This:

- never opens connections under burst (the pool is pre-warmed), so it never hits
  the connection-refused wall;
- caps concurrent server load at the pool size (set it at/below the clean
  threshold — **~4** from the measurement, tune live);
- needs no reusable *sessions* (which DBISAM doesn't support) — just reusable
  *sockets*, which it does.

**Recommendation: build the out-of-process broker over a warm-socket pool (~4),
not an in-process rate-limiter.** The rate-limiter would still pay a TCP connect
per query and only queues the burst; the warm-socket broker removes the connect
storm at the root. Either way, FDW backends should borrow from the broker rather
than `connect_and_login` per scan.

**Interim (before the broker exists):** the FDW's one-`connect_and_login`-per-scan
is correct; just cap how many PG backends hit DBISAM at once (keep the
On-Premises Gateway / PG connection pool ≤ ~4 to DBISAM-backed work) to stay
under the connection-refused threshold. Joins/scope unchanged.

## Pointers

- `exportmaster/src/response.rs` — `check_body_reqcode`, the `0x2Cxx` family.
- `exportmaster/src/client.rs` — `connect_and_login` (login + setup) and the two
  query paths' teardown blocks.
- `proj_init/06-connection-broker.md` §"Live finding" — where this was first seen.
