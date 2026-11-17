-- Phases 2–4: discussions scheduling state, decision details, external agent tokens.

ALTER TABLE discussions
    ADD COLUMN trigger_message_id  uuid REFERENCES messages(id),
    ADD COLUMN chain_root_id       uuid,
    ADD COLUMN depth               integer NOT NULL DEFAULT 0,
    ADD COLUMN turn_index          integer NOT NULL DEFAULT 0,
    ADD COLUMN spoke_this_round    boolean NOT NULL DEFAULT false,
    ADD COLUMN human_interjected   boolean NOT NULL DEFAULT false,
    ADD COLUMN end_reason          text,
    ADD COLUMN finished_at         timestamptz,
    ADD COLUMN updated_at          timestamptz NOT NULL DEFAULT now();
ALTER TABLE discussions
    ADD CONSTRAINT discussions_status_check CHECK (status IN ('running', 'summarizing', 'done', 'cancelled'));
CREATE INDEX discussions_active_idx ON discussions (conversation_id, thread_root_id)
    WHERE status IN ('running', 'summarizing');

ALTER TABLE decisions
    ADD COLUMN source                  text NOT NULL DEFAULT 'decider' CHECK (source IN ('rule', 'decider', 'none')),
    ADD COLUMN reason                  text,
    ADD COLUMN scores                  jsonb,
    ADD COLUMN interaction             text,
    ADD COLUMN interaction_confidence  real;

ALTER TABLE decider_configs
    ADD COLUMN api_key_last4 text;
CREATE INDEX decider_configs_ws_idx ON decider_configs (workspace_id, priority);

ALTER TABLE agents
    ADD COLUMN token_prefix      text,
    ADD COLUMN token_created_at  timestamptz,
    ADD COLUMN last_seen_at      timestamptz;
