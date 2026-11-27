CREATE TABLE users (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    name          text NOT NULL,
    email         text NOT NULL UNIQUE CHECK (email = lower(email)),
    password_hash text NOT NULL,
    role          text NOT NULL CHECK (role IN ('admin', 'operator', 'viewer')),
    created_at    timestamptz NOT NULL DEFAULT now()
);

-- Only SHA-256 hashes of tokens are stored.
CREATE TABLE sessions (
    token_hash bytea PRIMARY KEY,
    user_id    uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    expires_at timestamptz NOT NULL
);
CREATE INDEX sessions_user_idx ON sessions (user_id);
CREATE INDEX sessions_expires_idx ON sessions (expires_at);

CREATE TABLE agents (
    id           uuid PRIMARY KEY DEFAULT uuidv7(),
    name         text NOT NULL UNIQUE,
    token_hash   bytea NOT NULL UNIQUE,
    version      text NOT NULL,
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE enrollment_tokens (
    token_hash bytea PRIMARY KEY,
    agent_name text NOT NULL,
    expires_at timestamptz NOT NULL,
    used_at    timestamptz
);

-- No foreign key to users: the record must outlive the account.
CREATE TABLE audit_logs (
    id         bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    created_at timestamptz NOT NULL DEFAULT now(),
    user_id    uuid,
    user_name  text NOT NULL,
    action     text NOT NULL,
    target     text NOT NULL,
    ip         text NOT NULL,
    ok         boolean NOT NULL
);
CREATE INDEX audit_logs_time_idx ON audit_logs (created_at DESC, id DESC);
CREATE INDEX audit_logs_user_idx ON audit_logs (user_id, created_at DESC);

CREATE FUNCTION audit_logs_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'audit_logs is append-only';
END $$;

CREATE TRIGGER audit_logs_no_change BEFORE UPDATE OR DELETE ON audit_logs
    FOR EACH ROW EXECUTE FUNCTION audit_logs_append_only();
