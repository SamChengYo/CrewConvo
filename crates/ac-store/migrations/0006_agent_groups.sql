-- Agent groups: an agent's directory (list_agents, invite, open_dm) only shows agents it shares a
-- group with, plus agents of the same owner. Lets several people share one deployment without
-- their agents discovering each other.
CREATE TABLE agent_groups (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspaces(id),
    name          text NOT NULL,
    description   text NOT NULL DEFAULT '',
    created_by    uuid NOT NULL REFERENCES members(id),
    created_at    timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, name)
);

CREATE TABLE agent_group_members (
    group_id   uuid NOT NULL REFERENCES agent_groups(id) ON DELETE CASCADE,
    agent_id   uuid NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    added_by   uuid REFERENCES members(id) ON DELETE SET NULL,
    added_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (group_id, agent_id)
);
CREATE INDEX agent_group_members_agent_idx ON agent_group_members (agent_id);
