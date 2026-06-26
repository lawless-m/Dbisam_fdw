# 06 — Connection handling

Not FDW-internal, but the PowerBI DirectQuery use case mandates it. Design it
alongside the FDW, not after the first multi-visual report falls over.

## The constraint

The Exportmaster server **rejects concurrent login storms**. This is why
Delilah's eager schema probe had to be serial. The same constraint bites far
harder under DirectQuery.

## The fan-out problem

DirectQuery is bursty and parallel. One PowerBI report page with eight visuals
fires eight-plus queries. In Postgres each becomes a separate backend. A pgrx
FDW runs *in-backend*, so naively each backend opens its own Exportmaster
session — eight-plus simultaneous logins into a server that rejects exactly
that. Sharing sessions across backends is not free, because they are separate
processes.

This is a genuine design component, not a footnote.

## The full path

```
app.powerbi.com
      │  DirectQuery (bursty, parallel)
      ▼
On-premises Data Gateway        ← pools PG connections, sits on-prem
      │  PostgreSQL wire (Npgsql)
      ▼
PostgreSQL  ── backend ──┐
            ── backend ──┤  each runs dbisam_fdw in-process
            ── backend ──┘
                         │  Exportmaster TCP (login per session)
                         ▼
                  DBISAM server (rejects concurrent login storms)
```

Two pooling layers already exist in the path (the gateway in front of PG; PG's
own backend model). The new risk is purely at the bottom hop: backend →
Exportmaster.

## Options to evaluate

- **Out-of-process connection broker** in front of Exportmaster: a small
  daemon that owns a bounded pool of authenticated DBISAM sessions; FDW
  backends borrow rather than log in. Decouples PG backend count from DBISAM
  login count. Most robust; most new code.
- **Serialising / rate-limiting layer**: cap concurrent logins and queue,
  accepting added latency under burst. Simpler; may not meet DirectQuery
  responsiveness.
- **Per-backend session reuse**: at minimum, a backend should reuse one
  Exportmaster session across the queries it serves rather than reconnecting
  per scan. Necessary regardless of the broker decision.

## Recommendation

Per-backend reuse is non-negotiable and cheap — do it from the start. The
broker-vs-serialise decision depends on measured DirectQuery concurrency
against the real server's login tolerance; treat it as a sized experiment
early, because it shapes whether the milestone-1 demo survives a real report
page rather than a single-visual preview.
