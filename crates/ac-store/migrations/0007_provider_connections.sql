-- Platforms that host many agents (an "agent provider") connect once with a catalog URL and an
-- access token. Their agents are synced as members and woken over the A2A protocol.
CREATE TABLE provider_connections (
    id                       uuid PRIMARY KEY,
    workspace_id             uuid NOT NULL REFERENCES workspaces(id),
    name                     text NOT NULL,
    catalog_url              text NOT NULL,
    token_sealed             text,
    external_workspace_id    text,
    external_workspace_name  text,
    group_id                 uuid REFERENCES agent_groups(id) ON DELETE SET NULL,
    created_by               uuid NOT NULL REFERENCES members(id),
    last_sync_at             timestamptz,
    last_error               text,
    created_at               timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE agents
    ADD COLUMN provider_connection_id uuid REFERENCES provider_connections(id) ON DELETE SET NULL,
    ADD COLUMN external_id  text,
    ADD COLUMN a2a_url      text,
    ADD COLUMN a2a_legacy   boolean NOT NULL DEFAULT false,
    -- Never offered to the decision model: woken only by @mention, DM or a direct follow-up.
    ADD COLUMN mention_only boolean NOT NULL DEFAULT false;
CREATE UNIQUE INDEX agents_provider_external_idx ON agents (provider_connection_id, external_id)
    WHERE provider_connection_id IS NOT NULL;

ALTER TABLE agents DROP CONSTRAINT IF EXISTS agents_delivery_check;
ALTER TABLE agents ADD CONSTRAINT agents_delivery_check CHECK (delivery IN ('websocket', 'webhook', 'a2a'));

-- A2A context ids, so an agent keeps its memory per conversation / thread.
CREATE TABLE a2a_contexts (
    agent_id         uuid NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    conversation_id  uuid NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    thread_key       text NOT NULL,
    context_id       text NOT NULL,
    updated_at       timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (agent_id, conversation_id, thread_key)
);
