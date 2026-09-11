CREATE TABLE cognitive_events (
    sequence BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    event_id TEXT NOT NULL UNIQUE,
    recorded_at BIGINT NOT NULL CHECK (recorded_at > 0),
    focus_id TEXT,
    focus_revision BIGINT CHECK (focus_revision > 0),
    payload_json TEXT NOT NULL CHECK (octet_length(payload_json) <= 65536),
    payload_sha256 TEXT NOT NULL CHECK (length(payload_sha256) = 64),
    CHECK ((focus_id IS NULL) = (focus_revision IS NULL)),
    UNIQUE (focus_id, focus_revision)
);
CREATE INDEX cognitive_focus_time ON cognitive_events(focus_id, recorded_at DESC, sequence DESC);
CREATE TABLE cortex_graph_snapshots (
    sequence BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    snapshot_id TEXT NOT NULL UNIQUE,
    captured_at BIGINT NOT NULL CHECK (captured_at > 0),
    payload_json TEXT NOT NULL CHECK (octet_length(payload_json) <= 67108864),
    payload_sha256 TEXT NOT NULL CHECK (length(payload_sha256) = 64)
);
CREATE INDEX cortex_graph_time ON cortex_graph_snapshots(captured_at DESC, sequence DESC);
CREATE TABLE shared_thought_snapshots (
    thought_id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    recorded_at BIGINT NOT NULL CHECK (recorded_at > 0),
    focus_id TEXT NOT NULL,
    payload TEXT NOT NULL CHECK (octet_length(payload) <= 65536),
    hash TEXT NOT NULL CHECK (length(hash) = 64),
    PRIMARY KEY (thought_id, revision)
);
CREATE INDEX shared_thought_time ON shared_thought_snapshots(thought_id, recorded_at DESC, revision DESC);
CREATE TABLE cognitive_outbox (
    sequence BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    topic TEXT NOT NULL CHECK (topic IN ('event', 'graph', 'shared_thought', 'execution')),
    object_id TEXT NOT NULL,
    payload_sha256 TEXT NOT NULL CHECK (length(payload_sha256) = 64),
    recorded_at BIGINT NOT NULL CHECK (recorded_at > 0),
    acknowledged BOOLEAN NOT NULL DEFAULT FALSE,
    UNIQUE (topic, object_id)
);
CREATE TABLE execution_checkpoints (
    sequence BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    checkpoint_id TEXT NOT NULL UNIQUE,
    focus_id TEXT NOT NULL,
    branch_id TEXT NOT NULL,
    parent_id TEXT,
    recorded_at BIGINT NOT NULL CHECK (recorded_at > 0),
    payload_json TEXT NOT NULL CHECK (octet_length(payload_json) <= 131072),
    payload_sha256 TEXT NOT NULL CHECK (length(payload_sha256) = 64)
);
CREATE INDEX execution_focus_time ON execution_checkpoints(focus_id,recorded_at DESC,sequence DESC);
CREATE INDEX execution_branch ON execution_checkpoints(focus_id,branch_id);
CREATE TABLE cortex_correction_trajectories (
    trajectory_id TEXT PRIMARY KEY,
    correction_id TEXT NOT NULL UNIQUE,
    model_id TEXT NOT NULL,
    captured_at BIGINT NOT NULL CHECK (captured_at > 0),
    source_snapshot_id TEXT NOT NULL REFERENCES cortex_graph_snapshots(snapshot_id),
    resulting_snapshot_id TEXT NOT NULL UNIQUE REFERENCES cortex_graph_snapshots(snapshot_id),
    payload_json TEXT NOT NULL CHECK (octet_length(payload_json) <= 2097152),
    payload_sha256 TEXT NOT NULL CHECK (length(payload_sha256) = 64)
);
