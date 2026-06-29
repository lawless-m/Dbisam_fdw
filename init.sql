CREATE EXTENSION dbisam_fdw;

CREATE FOREIGN DATA WRAPPER dbisam_fdw
    HANDLER dbisam_fdw_handler VALIDATOR dbisam_fdw_validator;

  CREATE SERVER sem01 FOREIGN DATA WRAPPER dbisam_fdw
    OPTIONS (
      host     '___HOST___',          -- dev DBISAM box; use FQDN/IP if name doesn't resolve from the PG host
      port     '12005',             -- dbsrvr.exe TCP port (exportmaster default)
      catalog  'NISAINT_CS',        -- the only catalog tested against
      user     '___USER___',
      password '___PASS___'
    );

CREATE FOREIGN TABLE miketest ("Mike1" text, "Mike2" text)
    SERVER sem01 OPTIONS (table 'MikeTest');

CREATE SCHEMA em;

IMPORT FOREIGN SCHEMA dbisam FROM SERVER sem01 INTO em
    OPTIONS (parquet_dir '/mnt/RIVSPROD02_RI_SERVICES/Outputs/Parquets/em');

	
