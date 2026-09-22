CREATE TABLE thumbnail_artifacts (
    source_content_identity TEXT PRIMARY KEY,
    content_identity TEXT NULL,
    state TEXT NOT NULL CHECK (state IN ('ready', 'stale', 'generating', 'placeholder'))
);
