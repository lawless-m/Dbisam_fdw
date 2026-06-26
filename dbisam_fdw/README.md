# dbisam_fdw (pgrx extension crate)

The Supabase Wrappers / pgrx extension itself — the loadable `.so`. This crate
is **excluded from the parent workspace** until the pgrx toolchain is installed,
so `cargo test` on the `dbisam-sql` renderer never pulls the heavy pgrx graph.

## Bootstrap (one-time)

```sh
cargo install cargo-pgrx --locked       # pin to the version supabase-wrappers tracks
cargo pgrx init                         # downloads/builds the PG versions to dev against
```

Then reconcile versions: set `pgrx` / `pgrx-tests` in `Cargo.toml` to exactly
what `cargo-pgrx` installed, and `supabase-wrappers` to the release that depends
on that pgrx. Add `dbisam_fdw` to the parent `Cargo.toml` `members` (and drop it
from `exclude`) if you want it in the workspace.

## Develop / run

```sh
cargo pgrx run pg16      # builds, installs into a throwaway PG16, opens psql
```

```sql
CREATE EXTENSION dbisam_fdw;
CREATE SERVER em FOREIGN DATA WRAPPER dbisam_fdw
  OPTIONS (host '...', catalog 'NISAINT_CS');
CREATE USER MAPPING FOR CURRENT_USER SERVER em OPTIONS (user '...', password '...');
IMPORT FOREIGN SCHEMA dbisam FROM SERVER em INTO public;
```

## Status

Skeleton. `src/lib.rs` fixes the milestone-1 shape; the `ForeignDataWrapper`
trait impl and the `todo!()` module bodies (`connection`, `typemap`,
`schema_import`) are the next work, to be written against the pinned Wrappers
API. Read-only is enforced by **not** implementing the write callbacks
(`proj_init/02-scope-v1.md`).
