-- Schedules are a cron pattern in a time zone; retention is part of the same entry.
ALTER TABLE backup_configs
    ADD COLUMN schedule        text,
    ADD COLUMN timezone        text NOT NULL DEFAULT 'UTC',
    ADD COLUMN next_run_at     timestamptz,
    ADD COLUMN retention_unit  text CHECK (retention_unit IN ('days', 'months', 'gfs')),
    ADD COLUMN retention_value integer,
    ADD COLUMN keep_daily      integer,
    ADD COLUMN keep_weekly     integer,
    ADD COLUMN keep_monthly    integer,
    -- Retention only judges snapshots made after it was last changed.
    ADD COLUMN retention_since timestamptz NOT NULL DEFAULT now();
CREATE INDEX backup_configs_due_idx ON backup_configs (next_run_at) WHERE next_run_at IS NOT NULL;

-- Fixed when the snapshot is made: shortening a retention never deletes older backups.
ALTER TABLE snapshots
    ADD COLUMN expires_at timestamptz,
    -- The agent's MAC over what it stored, checked before a restore.
    ADD COLUMN manifest   text;
CREATE INDEX snapshots_expiry_idx ON snapshots (expires_at) WHERE expires_at IS NOT NULL;
CREATE INDEX snapshots_config_idx ON snapshots (backup_config_id, created_at DESC);
CREATE INDEX jobs_snapshot_idx ON jobs (snapshot_id) WHERE snapshot_id IS NOT NULL;

ALTER TABLE jobs DROP CONSTRAINT jobs_kind_check;
ALTER TABLE jobs ADD CONSTRAINT jobs_kind_check
    CHECK (kind IN ('backup', 'restore', 'test_database', 'test_destination', 'delete_snapshot', 'presign_snapshot'));
-- A download link, encrypted with the master key until the person who asked has fetched it.
ALTER TABLE jobs ADD COLUMN result bytea;

-- Telegram or Mattermost. `secret` (bot token and chat, or webhook URL) is encrypted with
-- the master key; `target` is a harmless label for the list.
CREATE TABLE notification_channels (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    name         text NOT NULL UNIQUE,
    kind         text NOT NULL CHECK (kind IN ('telegram', 'mattermost')),
    target       text NOT NULL,
    secret       bytea NOT NULL,
    events       text[] NOT NULL,
    last_error   text,
    last_sent_at timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now()
);

-- Messages waiting to be sent.
CREATE TABLE notifications (
    id              bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    channel_id      uuid NOT NULL REFERENCES notification_channels (id) ON DELETE CASCADE,
    message         text NOT NULL,
    attempts        integer NOT NULL DEFAULT 0,
    next_attempt_at timestamptz NOT NULL DEFAULT now(),
    sent_at         timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX notifications_waiting_idx ON notifications (next_attempt_at) WHERE sent_at IS NULL;

CREATE FUNCTION queue_alert(event text, message text) RETURNS void LANGUAGE sql AS $$
    INSERT INTO notifications (channel_id, message)
    SELECT id, message FROM notification_channels WHERE event = ANY (events);
$$;

-- A trigger sees every way a job can fail: the agent's report, a silent agent, a lost record.
CREATE FUNCTION alert_failed_job() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM queue_alert(
        CASE NEW.kind WHEN 'restore' THEN 'restore_failed' ELSE 'backup_failed' END,
        format(
            E'%s gagal: %s\nAgent: %s\n%s',
            CASE NEW.kind WHEN 'restore' THEN 'Restore' WHEN 'delete_snapshot' THEN 'Hapus backup lama' ELSE 'Backup' END,
            NEW.target,
            (SELECT name FROM agents WHERE id = NEW.agent_id),
            left(coalesce(NEW.error, ''), 300)
        )
    );
    RETURN NULL;
END $$;
CREATE TRIGGER alert_failed_job AFTER UPDATE OF status ON jobs
    FOR EACH ROW
    WHEN (NEW.status = 'failed' AND OLD.status IS DISTINCT FROM 'failed'
          AND NEW.kind IN ('backup', 'restore', 'delete_snapshot'))
    EXECUTE FUNCTION alert_failed_job();

ALTER TABLE agents ADD COLUMN offline_alerted boolean NOT NULL DEFAULT false;

CREATE TRIGGER notify_change AFTER INSERT OR UPDATE OR DELETE ON notification_channels FOR EACH ROW EXECUTE FUNCTION notify_change();
