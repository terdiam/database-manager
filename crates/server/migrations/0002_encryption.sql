-- Keypairs the server owns. `secret` is encrypted with DBM_MASTER_KEY.
CREATE TABLE server_keys (
    name       text PRIMARY KEY,
    public_key bytea NOT NULL,
    secret     bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Secrets for an agent are sealed to this key. Agents enrolled before it exist without one.
ALTER TABLE agents ADD COLUMN public_key bytea;
