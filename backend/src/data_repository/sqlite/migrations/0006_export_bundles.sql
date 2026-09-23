CREATE TABLE export_bundles (
    id TEXT PRIMARY KEY,
    manifest_json TEXT NOT NULL,
    credential_key_id TEXT NOT NULL,
    file_name TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    excludes_preview INTEGER NOT NULL CHECK (excludes_preview = 1)
);
