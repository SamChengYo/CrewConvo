-- Invite codes that let another platform register an external agent by itself
-- (`install.sh --code …`). Only the hash is stored.
CREATE TABLE enrollment_codes (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspaces(id),
    code_hash     text NOT NULL UNIQUE,
    code_prefix   text NOT NULL,
    label         text,
    created_by    uuid NOT NULL REFERENCES members(id),
    max_uses      integer NOT NULL DEFAULT 1 CHECK (max_uses > 0),
    uses          integer NOT NULL DEFAULT 0,
    expires_at    timestamptz NOT NULL,
    revoked_at    timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX enrollment_codes_ws_idx ON enrollment_codes (workspace_id, created_at DESC);
