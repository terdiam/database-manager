ALTER TABLE database_servers DROP CONSTRAINT database_servers_engine_check;
ALTER TABLE database_servers ADD CONSTRAINT database_servers_engine_check
    CHECK (engine IN ('postgresql', 'mysql', 'mariadb', 'mongodb', 'sqlite', 'redis', 'elasticsearch', 'opensearch'));
