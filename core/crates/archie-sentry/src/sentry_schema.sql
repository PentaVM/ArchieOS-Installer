CREATE TABLE sentry_intents (
    intent_id VARCHAR(128) PRIMARY KEY,
    operation_id BIGINT NOT NULL UNIQUE CHECK (operation_id > 0),
    resource_id VARCHAR(128) NOT NULL,
    intent_json TEXT NOT NULL CHECK (octet_length(intent_json) <= 1048576),
    intent_sha256 TEXT NOT NULL CHECK (intent_sha256 ~ '^[0-9a-f]{64}$'),
    reserved_at BIGINT NOT NULL CHECK (reserved_at > 0),
    result_json TEXT CHECK (octet_length(result_json) <= 1048576),
    result_sha256 TEXT CHECK (result_sha256 ~ '^[0-9a-f]{64}$'),
    CHECK ((result_json IS NULL) = (result_sha256 IS NULL))
);
REVOKE ALL ON sentry_intents FROM PUBLIC;
