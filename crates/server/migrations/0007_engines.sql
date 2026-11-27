ALTER TABLE database_servers DROP CONSTRAINT database_servers_engine_check;
ALTER TABLE database_servers ADD CONSTRAINT database_servers_engine_check
    CHECK (engine IN ('postgresql', 'mysql', 'mariadb', 'mongodb', 'sqlite'));
-- A backup can only be restored into the engine that made it.
ALTER TABLE snapshots ADD COLUMN engine text NOT NULL DEFAULT 'postgresql';
