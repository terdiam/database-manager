-- A token for an existing agent gives it a new credential after the old one was revoked.
ALTER TABLE enrollment_tokens ADD COLUMN agent_id uuid REFERENCES agents (id) ON DELETE CASCADE;
