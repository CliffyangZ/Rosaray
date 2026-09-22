CREATE TABLE datasets (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    display_name TEXT NOT NULL,
    latest_version_id TEXT NULL
);

CREATE TABLE dataset_versions (
    id TEXT PRIMARY KEY,
    dataset_id TEXT NOT NULL REFERENCES datasets(id),
    fingerprint TEXT NOT NULL,
    derived_from_version_id TEXT NULL REFERENCES dataset_versions(id),
    created_at TEXT NOT NULL,
    validation_status TEXT NOT NULL CHECK (validation_status IN ('ok', 'blocked', 'warned'))
);

CREATE INDEX idx_dataset_versions_dataset_id ON dataset_versions(dataset_id);

CREATE TABLE reference_masks (
    id TEXT PRIMARY KEY,
    content_identity TEXT NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    compatible_with_image_id TEXT NOT NULL,
    validity TEXT NOT NULL CHECK (validity IN ('valid', 'incompatible_dimensions', 'unreadable'))
);

CREATE TABLE image_assets (
    id TEXT PRIMARY KEY,
    external_source_uri TEXT NOT NULL,
    source_content_identity TEXT NOT NULL,
    imported_content_identity TEXT NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    source_created_at TEXT NULL,
    imported_at TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('available', 'source_missing', 'source_changed')),
    patient_id TEXT NULL,
    split TEXT NULL CHECK (split IS NULL OR split IN ('train', 'validation', 'test')),
    reference_mask_id TEXT NULL REFERENCES reference_masks(id),
    metadata_status TEXT NOT NULL CHECK (metadata_status IN ('complete', 'incomplete'))
);

CREATE INDEX idx_image_assets_imported_content_identity ON image_assets(imported_content_identity);
CREATE INDEX idx_image_assets_source_content_identity ON image_assets(source_content_identity);

CREATE TABLE dataset_version_images (
    dataset_version_id TEXT NOT NULL REFERENCES dataset_versions(id),
    image_asset_id TEXT NOT NULL REFERENCES image_assets(id),
    PRIMARY KEY (dataset_version_id, image_asset_id)
);

CREATE TABLE research_subjects (
    id TEXT PRIMARY KEY,
    dataset_id TEXT NOT NULL REFERENCES datasets(id),
    deidentified_patient_id TEXT NOT NULL,
    UNIQUE (dataset_id, deidentified_patient_id)
);
