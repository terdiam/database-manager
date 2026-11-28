-- An incremental entry follows a full one and takes its database, destination and retention.
ALTER TABLE backup_configs ADD COLUMN base_id uuid REFERENCES backup_configs (id);
CREATE INDEX backup_configs_base_idx ON backup_configs (base_id) WHERE base_id IS NOT NULL;

-- A chain is a full snapshot and the increments after it, each continuing its parent.
-- Positions say where in the database's log a file starts and ends.
ALTER TABLE snapshots
    ADD COLUMN chain_id      uuid,
    ADD COLUMN parent_id     uuid REFERENCES snapshots (id),
    ADD COLUMN position_from text,
    ADD COLUMN position_to   text;
CREATE INDEX snapshots_parent_idx ON snapshots (parent_id) WHERE parent_id IS NOT NULL;
CREATE INDEX snapshots_chain_idx ON snapshots (chain_id) WHERE chain_id IS NOT NULL;

ALTER TABLE jobs
    -- The snapshot an incremental backup continues.
    ADD COLUMN parent_id     uuid,
    ADD COLUMN restore_until timestamptz;
CREATE INDEX jobs_parent_idx ON jobs (parent_id) WHERE parent_id IS NOT NULL;
