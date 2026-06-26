# 05 — Type mapping

DBISAM → PostgreSQL. In milestone 1 this is not "good enough to read" — it is
a **correctness requirement**, because the data exits Postgres through Npgsql
to PowerBI, which expects clean, standard PG types on the wire and will render
coercion surprises silently. A wrong numeric in a measure is worse than a
query that fails.

## Fidelity requirements (the reason this doc exists)

- **Lossless numeric.** Carry over MrsFlow's lossless-NUMERIC care. DBISAM
  BCD / currency types map to PG `numeric` with full precision and scale —
  never to a float that drops digits.
- **Text encoding.** DBISAM stores Win-1252. Transcode **Win-1252 → UTF-8 at
  the protocol boundary**, the same point Delilah does it, so PG sees clean
  UTF-8 text and PowerBI renders it correctly.
- **Dates / datetimes.** Map to PG `date` / `timestamp` with correct
  semantics; verify round-trip through Npgsql, not just through `psql`.
- **Null handling.** Distinguish real NULLs from decode failures. Delilah's
  default raises on a row/blob that fails to decode (silent NULLs are
  indistinguishable from real data); a `lenient_decode` mode turns failures
  into NULLs with a per-batch stderr summary. Mirror this — default strict,
  lenient opt-in.

## Mapping table (fill against the live schema + Dibdog/Derek)

This is the skeleton; confirm exact DBISAM type tags against the protocol
schema decode in `exportmaster` and the Derek protocol notes before relying on it.

| DBISAM type | PostgreSQL type | Notes |
| --- | --- | --- |
| Integer family (byte/word/int/autoinc) | `int2` / `int4` / `int8` as width dictates | Autoinc is just an integer on read. |
| BCD / currency | `numeric` | Lossless — precision/scale preserved. MrsFlow has prior art. |
| Float / double | `float4` / `float8` | Only where the source is genuinely floating point — never for BCD. |
| Boolean | `bool` | Note the `<>`-includes-NULL quirk interacts with boolean filters (see 04). |
| String / char | `text` (or `varchar(n)`) | Win-1252 → UTF-8 at the boundary. |
| Memo | `text` | Per-row resolve via `OpenBlob`/`FreeBlob`, PK auto-injected. |
| BLOB / graphic | `bytea` | Per-row resolve; PK auto-injected; binary passthrough, no transcode. |
| Date | `date` | Verify via Npgsql round-trip. |
| Time | `time` | " |
| DateTime / timestamp | `timestamp` | " |

## BLOB / Memo handling

Carried over from Delilah: BLOB and Memo columns are resolved per row via the
protocol's `OpenBlob` / `FreeBlob`, with the primary key auto-injected into
the projection so each row can be targeted. This is already solved in the
protocol layer; the FDW just needs to surface the resolved values as `bytea` /
`text`.

## Schema introspection: lazy vs eager

Same trade-off Delilah resolved. Listing the catalogue needs only table names,
so probe columns lazily by default — enumeration stays instant on catalogues
with hundreds of tables. But catalogue-*wide* column introspection
(`information_schema.columns`, anything a GUI uses to populate a column
browser) then reports empty until a table is queried. Offer an eager mode
(`EAGER_SCHEMA`-equivalent) that probes every table once on first catalogue
access and caches for the session — **serial**, because the server rejects
concurrent login storms (~15 s for ~600 tables in Delilah). PowerBI's modelling
view does up-front introspection, so eager will likely be wanted for the
DirectQuery setup path even though lazy is the better default for query work.
