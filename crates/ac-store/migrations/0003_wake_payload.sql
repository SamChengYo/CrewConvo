-- Snapshot of the exact payload delivered to the agent ("what did the agent see?").
ALTER TABLE wakes ADD COLUMN payload jsonb;
CREATE INDEX wakes_created_idx ON wakes (created_at DESC);
