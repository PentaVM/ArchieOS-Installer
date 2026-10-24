-- Additive migration. schema.sql and its recorded v1 hash remain unchanged.
ALTER TABLE cognitive_outbox DROP CONSTRAINT cognitive_outbox_topic_check;
ALTER TABLE cognitive_outbox ADD CONSTRAINT cognitive_outbox_topic_check
    CHECK (topic IN ('event', 'graph', 'shared_thought', 'execution', 'inference'));

CREATE TABLE inference_focus_domains (
    focus_id TEXT PRIMARY KEY,
    domain TEXT NOT NULL CHECK (octet_length(domain) <= 192)
);
CREATE TABLE inference_bindings (
    task_id TEXT PRIMARY KEY,
    focus_id TEXT NOT NULL REFERENCES inference_focus_domains(focus_id),
    domain TEXT NOT NULL CHECK (octet_length(domain) <= 192),
    payload_json TEXT NOT NULL CHECK (octet_length(payload_json) <= 4096),
    payload_sha256 TEXT NOT NULL CHECK (length(payload_sha256) = 64)
);
CREATE INDEX inference_binding_focus ON inference_bindings(focus_id);
CREATE TABLE inference_receipts (
    sequence BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    receipt_id TEXT NOT NULL UNIQUE,
    task_id TEXT NOT NULL REFERENCES inference_bindings(task_id),
    checkpoint_id TEXT NOT NULL UNIQUE REFERENCES execution_checkpoints(checkpoint_id),
    observed_at BIGINT NOT NULL CHECK (observed_at > 0),
    scope_sha256 TEXT NOT NULL CHECK (length(scope_sha256) = 64),
    session_sha256 TEXT CHECK (length(session_sha256) = 64),
    payload_json TEXT NOT NULL CHECK (octet_length(payload_json) <= 65536),
    payload_sha256 TEXT NOT NULL CHECK (length(payload_sha256) = 64)
);
CREATE INDEX inference_receipt_task ON inference_receipts(task_id,observed_at DESC,sequence DESC);
CREATE INDEX inference_session_scope ON inference_receipts(scope_sha256,observed_at DESC,sequence DESC)
    WHERE session_sha256 IS NOT NULL;
CREATE INDEX inference_session_identity ON inference_receipts(session_sha256)
    WHERE session_sha256 IS NOT NULL;
