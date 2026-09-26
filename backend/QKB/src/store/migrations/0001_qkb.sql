-- QKB store (spec 003). Independent of the legacy project database: the
-- catalog tables are derived and rebuildable from bundle files; kb_event,
-- qkb_deletion_record, qkb_query, qkb_candidate and qkb_selection are
-- authoritative.

CREATE TABLE kb_catalog_entry (
    path TEXT PRIMARY KEY,
    id TEXT NULL,
    version TEXT NULL,
    kind TEXT NULL CHECK (kind IS NULL OR kind IN ('algonode', 'algopipe')),
    name TEXT NULL,
    summary TEXT NULL,
    purpose TEXT NULL,
    intended_use TEXT NULL,
    domain TEXT NULL,
    status TEXT NOT NULL CHECK (status IN ('draft', 'published', 'invalid')),
    maturity TEXT NULL,
    release_kind TEXT NULL,
    availability TEXT NULL,
    verification TEXT NULL,
    data_kinds TEXT NULL,
    content_id TEXT NULL,
    mtime INTEGER NOT NULL,
    size INTEGER NOT NULL,
    finding_count INTEGER NOT NULL DEFAULT 0,
    indexed_at TEXT NOT NULL,
    dataset_validation TEXT NOT NULL DEFAULT 'none',
    origin TEXT NULL,
    amendment_count INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_kb_catalog_identity ON kb_catalog_entry(kind, id, version);
CREATE INDEX idx_kb_catalog_domain ON kb_catalog_entry(domain);
CREATE INDEX idx_kb_catalog_purpose ON kb_catalog_entry(purpose);

CREATE TABLE kb_dependency (
    from_path TEXT NOT NULL REFERENCES kb_catalog_entry(path) ON DELETE CASCADE,
    to_id TEXT NOT NULL,
    to_version TEXT NOT NULL,
    to_content_id TEXT NULL,
    resolved INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_kb_dependency_from ON kb_dependency(from_path);
CREATE INDEX idx_kb_dependency_to ON kb_dependency(to_id, to_version);

CREATE VIRTUAL TABLE kb_fts USING fts5(
    name, summary, purpose, intended_use,
    path UNINDEXED
);

CREATE TABLE kb_finding (
    finding_id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL,
    bundle_id TEXT NULL,
    bundle_version TEXT NULL,
    severity TEXT NOT NULL CHECK (severity IN ('error', 'warning', 'info')),
    code TEXT NOT NULL,
    subject_json TEXT NOT NULL,
    explanation TEXT NOT NULL,
    action TEXT NOT NULL,
    detected_at TEXT NOT NULL
);

CREATE INDEX idx_kb_finding_path ON kb_finding(path);

-- Append-only audit events.
CREATE TABLE kb_event (
    event_id TEXT PRIMARY KEY,
    at TEXT NOT NULL,
    actor TEXT NOT NULL,
    type TEXT NOT NULL,
    subject TEXT NOT NULL,
    detail_json TEXT NOT NULL
);

CREATE INDEX idx_kb_event_type ON kb_event(type, at);
CREATE INDEX idx_kb_event_subject ON kb_event(subject);

CREATE TRIGGER kb_event_no_update BEFORE UPDATE ON kb_event
BEGIN SELECT RAISE(ABORT, 'kb_event is append-only'); END;
CREATE TRIGGER kb_event_no_delete BEFORE DELETE ON kb_event
BEGIN SELECT RAISE(ABORT, 'kb_event is append-only'); END;

-- Permanent-deletion records (FR-006). `pending` is written before any file is
-- removed so an interrupted deletion can be replayed at startup.
CREATE TABLE qkb_deletion_record (
    deletion_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('algonode', 'algopipe')),
    id TEXT NOT NULL,
    version TEXT NOT NULL,
    content_id TEXT NULL,
    reason_code TEXT NOT NULL,
    root_cause TEXT NULL,
    affected_dependents_json TEXT NOT NULL DEFAULT '[]',
    trigger TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'done')),
    path TEXT NOT NULL,
    recorded_at TEXT NOT NULL
);

CREATE INDEX idx_qkb_deletion_identity ON qkb_deletion_record(kind, id, version);

-- System One exchange records.
CREATE TABLE qkb_query (
    request_id TEXT PRIMARY KEY,
    protocol_version TEXT NOT NULL,
    source_id TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    task_purpose TEXT NOT NULL,
    data_conditions_json TEXT NOT NULL,
    received_at TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('answered', 'no_candidates'))
);

CREATE TABLE qkb_candidate (
    request_id TEXT NOT NULL REFERENCES qkb_query(request_id),
    rank INTEGER NOT NULL,
    pipe_id TEXT NOT NULL,
    pipe_version TEXT NOT NULL,
    content_id TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    applicability_json TEXT NOT NULL,
    PRIMARY KEY (request_id, rank)
);

CREATE TABLE qkb_selection (
    request_id TEXT PRIMARY KEY REFERENCES qkb_query(request_id),
    report_hash TEXT NOT NULL,
    source_id TEXT NOT NULL,
    decision TEXT NOT NULL CHECK (decision IN ('selected', 'abstained')),
    selected_id TEXT NULL,
    selected_version TEXT NULL,
    selected_content_id TEXT NULL,
    reason TEXT NOT NULL,
    validation TEXT NOT NULL CHECK (validation IN ('accepted', 'rejected')),
    rejection_code TEXT NULL,
    contract_snapshot_json TEXT NULL,
    selected_version_deleted INTEGER NOT NULL DEFAULT 0,
    recorded_at TEXT NOT NULL
);
