
CREATE TABLE boots (
    id               BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    node_id          TEXT    NOT NULL,
    started_at       BIGINT NOT NULL,
    archieos_version TEXT    NOT NULL,
    capabilities     TEXT    NOT NULL,
    descriptor       TEXT    NOT NULL
);
CREATE INDEX idx_boots_node ON boots (node_id, id DESC);

CREATE TABLE tasks (
    task_id     TEXT    PRIMARY KEY,
    trace_id    TEXT    NOT NULL,
    status      TEXT    NOT NULL,
    queue_class TEXT    NOT NULL,
    created_at  BIGINT NOT NULL,
    updated_at  BIGINT NOT NULL,
    payload     TEXT    NOT NULL
);
CREATE INDEX idx_tasks_status ON tasks (status, created_at);
CREATE INDEX idx_tasks_trace ON tasks (trace_id);

CREATE TABLE peers (
    node_id    TEXT    PRIMARY KEY,
    public_key TEXT    NOT NULL,
    first_seen BIGINT NOT NULL,
    last_seen  BIGINT NOT NULL,
    trust      TEXT    NOT NULL,
    descriptor TEXT
);

CREATE TABLE audit (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    recorded_at BIGINT NOT NULL,
    node_id     TEXT    NOT NULL,
    action      TEXT    NOT NULL,
    outcome     TEXT    NOT NULL,
    payload     TEXT    NOT NULL
);
CREATE INDEX idx_audit_time ON audit (recorded_at DESC);
CREATE INDEX idx_audit_action ON audit (action);


CREATE TABLE devices (
    device_reference TEXT    PRIMARY KEY,
    public_key       TEXT    NOT NULL,
    class            TEXT    NOT NULL,
    first_seen       BIGINT NOT NULL,
    last_seen        BIGINT NOT NULL,
    pairing_state    TEXT    NOT NULL
);
CREATE INDEX idx_devices_pairing ON devices (pairing_state, first_seen);


CREATE TABLE device_operations (
    operation_id    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    device_reference TEXT    NOT NULL REFERENCES devices(device_reference),
    trace_id        TEXT    NOT NULL,
    operation_sha256 TEXT   NOT NULL,
    prepared_at     BIGINT NOT NULL,
    state           TEXT    NOT NULL
);
CREATE INDEX idx_device_operations_recent ON device_operations (operation_id DESC);
CREATE INDEX idx_device_operations_device ON device_operations (device_reference, operation_id DESC);


CREATE TABLE device_adapter_proposals (
    proposal_id     BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    adapter_id      TEXT    NOT NULL,
    module_id       TEXT    NOT NULL,
    manifest_json   TEXT    NOT NULL,
    manifest_sha256 TEXT    NOT NULL,
    proposed_at     BIGINT NOT NULL
);
CREATE INDEX idx_device_adapter_proposals_recent
    ON device_adapter_proposals (proposal_id DESC);

CREATE TABLE cortex_receipt_history(trajectory_id TEXT PRIMARY KEY,correction_id TEXT NOT NULL UNIQUE,captured_at BIGINT NOT NULL,payload_json TEXT NOT NULL,payload_sha256 TEXT NOT NULL);
CREATE TABLE metric_samples(recorded_at BIGINT NOT NULL,metric TEXT NOT NULL,node_id TEXT NOT NULL,value DOUBLE PRECISION NOT NULL,provenance_ref TEXT NOT NULL,PRIMARY KEY(recorded_at,metric,node_id));
SELECT public.create_hypertable('metric_samples', public.by_range('recorded_at',86400000));
CREATE TABLE memory_vectors(object_id TEXT NOT NULL,model_id TEXT NOT NULL,dimensions INTEGER NOT NULL,embedding public.vector NOT NULL,sensitivity INTEGER NOT NULL CHECK(sensitivity BETWEEN 0 AND 5),provenance_ref TEXT NOT NULL,recorded_at BIGINT NOT NULL,PRIMARY KEY(object_id,model_id,recorded_at),CHECK(public.vector_dims(embedding)=dimensions));
CREATE TABLE database_snapshots(snapshot_id TEXT PRIMARY KEY,captured_at BIGINT NOT NULL,manifest_json TEXT NOT NULL,manifest_sha256 TEXT NOT NULL);
CREATE FUNCTION check_archived_correction() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF EXISTS(SELECT 1 FROM cortex_receipt_history WHERE (correction_id=NEW.correction_id OR trajectory_id=NEW.trajectory_id)
  AND (payload_sha256<>NEW.payload_sha256 OR correction_id<>NEW.correction_id OR trajectory_id<>NEW.trajectory_id)) THEN
  RAISE EXCEPTION USING ERRCODE='23514', MESSAGE='correction receipt conflict';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER check_archived_correction BEFORE INSERT ON cortex_correction_trajectories FOR EACH ROW EXECUTE FUNCTION check_archived_correction();
