-- Who it was beyond a display name (names repeat and change), and what they used.
ALTER TABLE audit_logs
    ADD COLUMN user_email text NOT NULL DEFAULT '',
    ADD COLUMN user_agent text NOT NULL DEFAULT '';
