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

## Root-cause hypotheses (ranked)

1. **The server's session is single-statement: a completed statement lifecycle
   drops the connection back to pre-login.** DBISAM's server appears to bind the
   logged-in state to one prepare/execute/cursor cycle; once it's reset/released,
   the connection is "logged out" for the purposes of a new `PrepareStatement`.
   This matches "request before login" exactly and matches the fact that nothing
   in our teardown is a logout. **Most consistent with the evidence.**

2. **One of the teardown messages de-registers the session.**
   `RemoveAllRemoteMemoryTables` (0x0029) or `ResetStatement` may do more than
   intended and tear down login state. Plausible but weaker — a "remove temp
   tables" op logging you out would be odd.

3. **A per-statement re-attach is required.** The catalog attach (0x003c) or part
   of session-setup must be re-issued before each statement, and skipping it
   reads to the server as "not logged in."

## Is reuse achievable? — two experiments

Neither is done here (the deliverable is this doc; reuse is not blocking). Both
are cheap to run against rivsem04 with the existing `run_query`/`find_memo`
examples:

- **E1 — lighter teardown.** Run query 1 omitting `RemoveAllRemoteMemoryTables`
  (and/or `ResetStatement`), then issue query 2's `PrepareStatement` on the same
  `Client`. If query 2 succeeds, hypothesis 2 holds and reuse is a one-line
  teardown change. If it still returns `0x2C2C`, hypothesis 2 is out.

- **E2 — re-login on the same socket.** After query 1's teardown, re-issue
  `login (0x0014) + session-setup` on the *existing* socket (no TCP reconnect)
  before query 2's `PrepareStatement`. If accepted, hypothesis 1 holds: the
  server requires a fresh login per statement, but you can do it without a new
  TCP connection. If it's rejected too, the connection itself is spent and only a
  full reconnect works.

The result that matters for Q4 is **E2**: does avoiding the TCP reconnect still
require a login (so every query is a login regardless), or can a socket carry
many logins cheaply?

## Recommendation for doc 06 (Q4: broker vs serialise)

This sharpens Q4 against the "per-backend session reuse is cheap" assumption:

- If hypothesis 1 holds (likely), **a DBISAM connection cannot amortise its login
  across queries** — N queries ≈ N logins no matter what. A connection *pool* of
  pre-authenticated sessions then buys little, because each pooled session still
  only serves one statement before needing re-login. The real lever becomes
  **serialising / rate-limiting logins** so the bursty DirectQuery fan-out never
  presents a concurrent login storm — i.e. lean toward the **serialise** option
  (or a broker whose job is to *serialise logins*, not to *hold reusable
  sessions*).

- If E1 succeeds (hypothesis 2), reuse is nearly free: fix the teardown and a
  single backend can run many queries on one login — which makes per-backend
  reuse real and de-prioritises the broker.

**Action:** run E1 then E2 (a ~1 hour experiment) before committing to a Q4
design. Until then, the FDW's current one-login-per-scan behaviour is correct and
safe; the open risk is purely the login-storm rate under a real multi-visual
DirectQuery page, which the serialise/broker decision must address.

## Pointers

- `exportmaster/src/response.rs` — `check_body_reqcode`, the `0x2Cxx` family.
- `exportmaster/src/client.rs` — `connect_and_login` (login + setup) and the two
  query paths' teardown blocks.
- `proj_init/06-connection-broker.md` §"Live finding" — where this was first seen.
