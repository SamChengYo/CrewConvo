-- Invite codes are replaced by a browser-approval flow (`crewconvo connect <server>`):
-- the CLI asks for a short code, a signed-in human approves it on the web, the CLI receives a token.
DROP TABLE IF EXISTS enrollment_codes;

CREATE TABLE connect_requests (
    id                uuid PRIMARY KEY,
    device_code_hash  text NOT NULL UNIQUE,
    user_code         text NOT NULL UNIQUE,
    client            jsonb NOT NULL DEFAULT '{}'::jsonb,
    status            text NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending', 'approved', 'denied', 'consumed')),
    workspace_id      uuid REFERENCES workspaces(id),
    agent_id          uuid REFERENCES members(id) ON DELETE SET NULL,
    token_sealed      text,
    decided_by        uuid REFERENCES members(id) ON DELETE SET NULL,
    expires_at        timestamptz NOT NULL,
    created_at        timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX connect_requests_expires_idx ON connect_requests (expires_at);
