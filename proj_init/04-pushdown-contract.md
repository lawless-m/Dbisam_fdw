# 04 — Pushdown contract

What renders to DBISAM SQL, what falls back to Postgres, and the dialect
quirks to compensate. The authoritative grammar is **Dibdog** (Prolog DCG);
the reference implementation of these rules is Delilah's `dbisam_filter_render`.
Port faithfully against Dibdog — do **not** hand-copy from Delilah's C++ and
let the rules drift.

## Principle

Render the predicate subset DBISAM can evaluate correctly and cheaply; hand
everything else back to Postgres as a local recheck. In the single-source
milestone-1 world, a fallback is usually a sign something pushable wasn't
pushed — worth surfacing in testing rather than silently tolerating — because
the goal is that the leaf scan is the only DBISAM interaction.

## Foldable subset (push down)

Mirrors what Delilah already supports:

- **Comparisons:** `=`, `<>`, `<`, `>`, `<=`, `>=`
- **Sets:** `IN`, `NOT IN`
- **Null tests:** `IS NULL`, `IS NOT NULL`
- **Boolean structure:** `AND`, `OR`
- **Pattern:** `LIKE` — **prefix form only** (`col LIKE 'abc%'`). Not arbitrary
  patterns.
- **Whitelisted single-column expressions:** currently `LEFT(col, n)` —
  e.g. `WHERE LEFT(code, 1) IN ('4','6')` runs server-side. Keep the whitelist
  explicit and small; expand only against Dibdog.
- **Projection:** push the requested column list; only those columns travel.
- **Limit:** push as a trailing `TOP n` (see quirks).

Anything outside this — arbitrary functions, multi-column expressions,
non-prefix `LIKE`, anything Dibdog cannot express — stays in Postgres.

## The four dialect quirks to compensate

DBISAM 4 diverges from ANSI in specific places. The renderer must handle these
or it will return wrong data, not just slow data.

1. **`TOP n` is a *trailing* clause.** `SELECT * FROM CUSTOMER TOP 5`, never
   `SELECT TOP 5 * FROM CUSTOMER`. Limit pushdown appends `TOP n`.

2. **`col <> x` includes NULL rows.** DBISAM treats `NULL <> x` as TRUE. To
   preserve ANSI semantics, render `<>` (and `NOT IN`) as
   `(col <> x AND col IS NOT NULL)`. This is a correctness compensation, not
   an optimisation — get it wrong and `<>` silently returns NULL rows PowerBI
   did not ask for.

3. **`LIKE` is prefix-only.** Only `'abc%'`-shaped patterns push down; anything
   with leading or internal wildcards stays local.

4. **Other comparisons are ANSI-safe.** `=`, `<`, `>`, `<=`, `>=`, `IN`,
   `IS [NOT] NULL` exclude NULLs the same way ANSI does — no compensation
   needed. Only `<>` / `NOT IN` need the NULL guard.

## Fallback behaviour

When the renderer cannot express a qual, it must:

- leave that qual for Postgres to apply after the scan (a local recheck), and
- still push down whatever projection / limit / other quals *are* foldable.

This is the same transparent-fallback shape Delilah relies on DuckDB for.
Never push a predicate the renderer is unsure about — correctness over
cleverness.

## Limit edge case

When a non-pushable filter sits *above* the scan, a trailing `TOP n` would cap
the wrong row count. In that case fall back to first-batch sizing rather than
emitting `TOP n` (Delilah's existing behaviour). The renderer must know
whether the limit is safe to push given what sits above it.
