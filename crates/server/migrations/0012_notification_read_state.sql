-- Up to when this person has seen finished jobs. Existing and new accounts start with
-- nothing unread.
ALTER TABLE users ADD COLUMN notices_read_at timestamptz NOT NULL DEFAULT now();
