-- Secrets are sealed by the browser to one agent's key: the server stores only ciphertext.
-- host/port/username and endpoint/bucket are copies for listing; agents use the sealed values.
CREATE TABLE database_servers (
    id         uuid PRIMARY KEY,
    name       text NOT NULL UNIQUE,
    engine     text NOT NULL CHECK (engine IN ('postgresql')),
    agent_id   uuid NOT NULL REFERENCES agents (id),
    host       text NOT NULL,
    port       integer NOT NULL,
    username   text NOT NULL,
    secret     bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX database_servers_agent_idx ON database_servers (agent_id);

CREATE TABLE destinations (
    id         uuid PRIMARY KEY,
    name       text NOT NULL UNIQUE,
    kind       text NOT NULL CHECK (kind IN ('s3')),
    agent_id   uuid NOT NULL REFERENCES agents (id),
    endpoint   text NOT NULL,
    bucket     text NOT NULL,
    prefix     text NOT NULL,
    secret     bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX destinations_agent_idx ON destinations (agent_id);

CREATE TABLE backup_configs (
    id             uuid PRIMARY KEY DEFAULT uuidv7(),
    name           text NOT NULL UNIQUE,
    server_id      uuid NOT NULL REFERENCES database_servers (id),
    database_name  text NOT NULL,
    destination_id uuid NOT NULL REFERENCES destinations (id),
    compression    text NOT NULL,
    level          integer NOT NULL,
    created_at     timestamptz NOT NULL DEFAULT now()
);

-- age keypairs for backup files. `secret` is encrypted with DBM_MASTER_KEY.
-- Keys are never deleted: old backups need them.
CREATE TABLE backup_keys (
    id         integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    recipient  text NOT NULL,
    secret     bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE jobs (
    id               uuid PRIMARY KEY DEFAULT uuidv7(),
    kind             text NOT NULL CHECK (kind IN ('backup', 'restore', 'test_database', 'test_destination', 'delete_snapshot')),
    status           text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    agent_id         uuid NOT NULL REFERENCES agents (id) ON DELETE CASCADE,
    -- What the job is about, as text, so history survives deleted records.
    target           text NOT NULL,
    backup_config_id uuid REFERENCES backup_configs (id) ON DELETE SET NULL,
    server_id        uuid REFERENCES database_servers (id) ON DELETE SET NULL,
    destination_id   uuid REFERENCES destinations (id) ON DELETE SET NULL,
    snapshot_id      uuid,
    database_name    text,
    object_key       text,
    compression      text,
    level            integer,
    key_id           integer REFERENCES backup_keys (id),
    bytes            bigint NOT NULL DEFAULT 0,
    error            text,
    lease            uuid,
    lease_expires_at timestamptz,
    cancel_requested boolean NOT NULL DEFAULT false,
    created_by       text NOT NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    started_at       timestamptz,
    finished_at      timestamptz
);
-- The queue: only waiting rows are indexed.
CREATE INDEX jobs_queue_idx ON jobs (agent_id, created_at) WHERE status = 'pending';
CREATE INDEX jobs_running_idx ON jobs (lease_expires_at) WHERE status = 'running';
CREATE INDEX jobs_time_idx ON jobs (created_at DESC, id DESC);
-- One backup of a configuration at a time.
CREATE UNIQUE INDEX jobs_one_backup_idx ON jobs (backup_config_id)
    WHERE kind = 'backup' AND status IN ('pending', 'running');

CREATE TABLE job_logs (
    job_id     uuid NOT NULL REFERENCES jobs (id) ON DELETE CASCADE,
    id         bigint GENERATED ALWAYS AS IDENTITY,
    created_at timestamptz NOT NULL DEFAULT now(),
    level      text NOT NULL,
    message    text NOT NULL,
    PRIMARY KEY (job_id, id)
);

CREATE TABLE snapshots (
    id               uuid PRIMARY KEY DEFAULT uuidv7(),
    job_id           uuid NOT NULL,
    backup_config_id uuid REFERENCES backup_configs (id) ON DELETE SET NULL,
    server_name      text NOT NULL,
    database_name    text NOT NULL,
    destination_id   uuid NOT NULL REFERENCES destinations (id),
    object_key       text NOT NULL,
    size             bigint NOT NULL,
    sha256           text NOT NULL,
    compression      text NOT NULL,
    key_id           integer NOT NULL REFERENCES backup_keys (id),
    created_at       timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX snapshots_time_idx ON snapshots (created_at DESC, id DESC);
CREATE INDEX snapshots_server_idx ON snapshots (server_name, created_at DESC);
CREATE INDEX snapshots_destination_idx ON snapshots (destination_id);
