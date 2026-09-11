-- Run with psql -X --set ON_ERROR_STOP=1 --file preflight.sql using a configured
-- PGSERVICE/PGPASSFILE. Never put credentials in command arguments.
-- Read-only capability probe; creates no extensions, schemas, roles or tables.
BEGIN READ ONLY;
SET LOCAL statement_timeout = '5s';
DO $$
BEGIN
  IF current_setting('server_version_num')::integer / 10000 <> 18 THEN
    RAISE EXCEPTION 'Archie data-plane candidate requires PostgreSQL major 18';
  END IF;
  IF pg_is_in_recovery() THEN
    RAISE EXCEPTION 'Candidate is a recovery/replica endpoint; not a writable authority';
  END IF;
  IF NOT EXISTS (SELECT FROM pg_extension WHERE extname='timescaledb' AND extversion='2.29.2') THEN
    RAISE EXCEPTION 'Reviewed TimescaleDB 2.29.2 extension is unavailable';
  END IF;
  IF NOT EXISTS (SELECT FROM pg_extension WHERE extname='vector' AND extversion='0.8.6') THEN
    RAISE EXCEPTION 'Reviewed pgvector 0.8.6 extension is unavailable';
  END IF;
END $$;
SELECT json_build_object(
  'status','COMPATIBILITY_ONLY',
  'postgres_version_num',current_setting('server_version_num'),
  'extensions',(SELECT json_object_agg(extname,extversion) FROM pg_extension WHERE extname IN ('timescaledb','vector')),
  'backend_switched',false,
  'runtime_validated',false
);
ROLLBACK;
