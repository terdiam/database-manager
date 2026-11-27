-- A bastion a database is reached through. As with other connections, the secret is
-- sealed to one agent; host, port and username are copies for listing and editing.
CREATE TABLE ssh_configs (
    id         uuid PRIMARY KEY,
    name       text NOT NULL UNIQUE,
    agent_id   uuid NOT NULL REFERENCES agents (id),
    host       text NOT NULL,
    port       integer NOT NULL,
    username   text NOT NULL,
    auth       text NOT NULL CHECK (auth IN ('password', 'key')),
    secret     bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ssh_configs_agent_idx ON ssh_configs (agent_id);

-- In use means it cannot be deleted.
ALTER TABLE database_servers ADD COLUMN ssh_config_id uuid REFERENCES ssh_configs (id);
CREATE INDEX database_servers_ssh_idx ON database_servers (ssh_config_id) WHERE ssh_config_id IS NOT NULL;

ALTER TABLE destinations DROP CONSTRAINT destinations_kind_check;
ALTER TABLE destinations ADD CONSTRAINT destinations_kind_check CHECK (kind IN ('s3', 'sftp', 'ftp', 'smb'));
-- Shown in the edit form; empty for S3, which signs in with keys.
ALTER TABLE destinations ADD COLUMN username text;

ALTER TABLE jobs DROP CONSTRAINT jobs_kind_check;
ALTER TABLE jobs ADD CONSTRAINT jobs_kind_check CHECK (kind IN
    ('backup', 'restore', 'test_database', 'test_destination', 'test_ssh', 'delete_snapshot', 'presign_snapshot'));
ALTER TABLE jobs ADD COLUMN ssh_config_id uuid REFERENCES ssh_configs (id) ON DELETE SET NULL;

CREATE TRIGGER notify_change AFTER INSERT OR UPDATE OR DELETE ON ssh_configs FOR EACH ROW EXECUTE FUNCTION notify_change();
