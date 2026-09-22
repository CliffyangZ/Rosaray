CREATE TABLE metadata_manifests (
    id TEXT PRIMARY KEY,
    manifest_version TEXT NOT NULL,
    entries_json TEXT NOT NULL
);

CREATE TABLE import_batches (
    id TEXT PRIMARY KEY,
    source_selection TEXT NOT NULL CHECK (source_selection IN ('single_file', 'multi_file', 'folder_scan')),
    metadata_manifest_id TEXT NULL REFERENCES metadata_manifests(id),
    status TEXT NOT NULL CHECK (status IN ('previewing', 'confirmed', 'cancelled', 'failed')),
    created_at TEXT NOT NULL,
    candidates_json TEXT NOT NULL
);

CREATE TABLE validation_findings (
    id TEXT PRIMARY KEY,
    dataset_version_id TEXT NOT NULL REFERENCES dataset_versions(id),
    category TEXT NOT NULL CHECK (category IN (
        'patient_split_leakage', 'missing_patient_id', 'missing_or_incompatible_mask',
        'duplicate_content', 'incomplete_metadata'
    )),
    severity TEXT NOT NULL CHECK (severity IN ('blocking', 'warning')),
    affected_image_asset_ids_json TEXT NOT NULL
);

CREATE INDEX idx_validation_findings_dataset_version_id ON validation_findings(dataset_version_id);
