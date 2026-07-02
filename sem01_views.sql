-- Mirror the em.* DBISAM foreign tables as plain views in schema `sem01`,
-- so PowerBI's PostgreSQL connector lists them (its Navigator skips foreign
-- tables, relkind='f', but shows views, relkind='v'). The schema name mirrors
-- the DuckDB `sem01` catalog so models port across both.
--
-- Idempotent: re-run after any IMPORT FOREIGN SCHEMA to reconcile new/changed
-- tables. Every existing view in sem01 is dropped and recreated — `CREATE OR
-- REPLACE VIEW` can't drop/rename/retype columns, so it errors on exactly the
-- tables that changed shape; recreating sidesteps that, and also removes stale
-- views whose foreign table has gone. The schema is dedicated to these mirrors:
-- don't put hand-written views in sem01, the next run will drop them.
-- Run with psql (NOT pgAdmin/DBeaver — they split the DO block on its
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
    WHERE n.nspname = 'sem01' AND c.relkind = 'v'
  LOOP
    EXECUTE format('DROP VIEW sem01.%I', t);
  END LOOP;

  FOR t IN
    SELECT c.relname
    FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'em' AND c.relkind = 'f'
  LOOP
    EXECUTE format('CREATE VIEW sem01.%I AS SELECT * FROM em.%I', t, t);
  END LOOP;
END $$;

GRANT SELECT ON ALL TABLES IN SCHEMA sem01 TO bi;
