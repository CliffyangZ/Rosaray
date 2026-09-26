-- Feature 002 (Quantify KB AlgoDesigner). Derived catalog tables can be
-- dropped and rebuilt from bundle files (FR-040); paper_source,
-- extraction_candidate, review_decision and kb_event are authoritative.

-- ---- derived catalog ------------------------------------------------------

CREATE TABLE kb_catalog_entry (
    path TEXT PRIMARY KEY,                 -- relative to the knowledge-base root
    id TEXT NULL,                          -- NULL only when the bundle is too broken to identify
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
    data_kinds TEXT NULL,                  -- space-separated artifact kinds for the data_kind filter
    content_id TEXT NULL,
    mtime INTEGER NOT NULL,
    size INTEGER NOT NULL,
    finding_count INTEGER NOT NULL DEFAULT 0,
    indexed_at TEXT NOT NULL
);

CREATE INDEX idx_kb_catalog_identity ON kb_catalog_entry(kind, id, version);

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

CREATE TABLE kb_draft_session (
    draft_key TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('algonode', 'algopipe')),
    id TEXT NOT NULL,
    base_revision TEXT NOT NULL,
    opened_at TEXT NOT NULL
);

-- ---- authoritative: audit events (append-only, FR-032) --------------------

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

-- ---- authoritative: paper extraction --------------------------------------

CREATE TABLE paper_source (
    paper_id TEXT PRIMARY KEY,
    content_id TEXT NOT NULL,
    blob_ref TEXT NOT NULL,
    title TEXT NULL,
    authors TEXT NULL,
    year INTEGER NULL,
    doi TEXT NULL,
    page_count INTEGER NOT NULL DEFAULT 0,
    pages_without_text_json TEXT NOT NULL DEFAULT '[]',
    authorization_attested INTEGER NOT NULL CHECK (authorization_attested IN (0, 1)),
    authorization_attested_at TEXT NULL,
    extraction_state TEXT NOT NULL CHECK (extraction_state IN ('imported', 'extracting', 'extracted', 'cancelled', 'failed')),
    imported_at TEXT NOT NULL
);

CREATE INDEX idx_paper_source_content ON paper_source(content_id);

CREATE TABLE extraction_candidate (
    candidate_id TEXT PRIMARY KEY,
    paper_id TEXT NOT NULL REFERENCES paper_source(paper_id),
    run_id TEXT NOT NULL,
    category TEXT NOT NULL,
    sources_json TEXT NOT NULL,
    proposed_json TEXT NOT NULL,
    ambiguities_json TEXT NOT NULL DEFAULT '[]',
    state TEXT NOT NULL CHECK (state IN ('proposed', 'accepted', 'edited', 'mapped', 'non_executable', 'rejected', 'deferred')),
    origin TEXT NOT NULL DEFAULT 'paper_derived' CHECK (origin = 'paper_derived'),
    created_at TEXT NOT NULL
);

CREATE INDEX idx_extraction_candidate_paper ON extraction_candidate(paper_id, run_id);

CREATE TABLE review_decision (
    decision_id TEXT PRIMARY KEY,
    candidate_id TEXT NOT NULL REFERENCES extraction_candidate(candidate_id),
    action TEXT NOT NULL CHECK (action IN (
        'accept', 'edit', 'reject', 'mark_non_executable', 'map_to_node', 'keep_specification_only', 'defer'
    )),
    target_ref TEXT NULL,
    edited_snapshot_json TEXT NULL,
    rationale TEXT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_review_decision_candidate ON review_decision(candidate_id, created_at);

CREATE TRIGGER review_decision_no_update BEFORE UPDATE ON review_decision
BEGIN SELECT RAISE(ABORT, 'review_decision is append-only'); END;
CREATE TRIGGER review_decision_no_delete BEFORE DELETE ON review_decision
BEGIN SELECT RAISE(ABORT, 'review_decision is append-only'); END;

-- ---- additive amendments to feature 001 tables (all nullable) -------------

ALTER TABLE image_assets ADD COLUMN pixel_spacing_mm_x REAL NULL;
ALTER TABLE image_assets ADD COLUMN pixel_spacing_mm_y REAL NULL;
ALTER TABLE image_assets ADD COLUMN spacing_source TEXT NULL
    CHECK (spacing_source IS NULL OR spacing_source IN ('metadata', 'user_entered'));

ALTER TABLE run_records ADD COLUMN algopipe_id TEXT NULL;
ALTER TABLE run_records ADD COLUMN algopipe_version TEXT NULL;
ALTER TABLE run_records ADD COLUMN algopipe_content_id TEXT NULL;
ALTER TABLE run_records ADD COLUMN eligibility_event_id TEXT NULL;

ALTER TABLE pipeline_snapshots ADD COLUMN source_algopipe_content_id TEXT NULL;
