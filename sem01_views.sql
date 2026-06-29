-- Mirror the em.* DBISAM foreign tables as plain views in schema `sem01`,
-- so PowerBI's PostgreSQL connector lists them (its Navigator skips foreign
-- tables, relkind='f', but shows views, relkind='v'). The schema name mirrors
-- the DuckDB `sem01` catalog so models port across both.
--
-- Idempotent: re-run after any IMPORT FOREIGN SCHEMA to reconcile new/changed
-- tables. Run with psql (NOT pgAdmin/DBeaver — they split the DO block on its
-- internal semicolons):
--     psql -U postgres -d em -f sem01_views.sql

CREATE SCHEMA IF NOT EXISTS sem01;
GRANT USAGE ON SCHEMA sem01 TO bi;

DO $$
DECLARE t text;
BEGIN
  FOR t IN
    SELECT c.relname
    FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'em' AND c.relkind = 'f'
  LOOP
    EXECUTE format('CREATE OR REPLACE VIEW sem01.%I AS SELECT * FROM em.%I', t, t);
  END LOOP;
END $$;

GRANT SELECT ON ALL TABLES IN SCHEMA sem01 TO bi;
