CREATE TABLE pipeline_snapshots (
    id TEXT PRIMARY KEY,
    graph_identity TEXT NOT NULL,
    node_versions_json TEXT NOT NULL,
    canonical_parameters_json TEXT NOT NULL
);

CREATE TABLE run_input_artifacts (
    id TEXT PRIMARY KEY,
    content_identity TEXT NOT NULL,
    source_image_asset_id TEXT NOT NULL REFERENCES image_assets(id),
    created_at TEXT NOT NULL
);

CREATE TABLE run_records (
    id TEXT PRIMARY KEY,
    status TEXT NOT NULL CHECK (status IN ('running', 'succeeded', 'failed', 'cancelled')),
    dataset_version_id TEXT NOT NULL REFERENCES dataset_versions(id),
    dataset_fingerprint TEXT NOT NULL,
    image_asset_id TEXT NOT NULL REFERENCES image_assets(id),
    image_asset_identity TEXT NOT NULL,
    run_input_artifact_id TEXT NOT NULL REFERENCES run_input_artifacts(id),
    pipeline_snapshot_id TEXT NOT NULL REFERENCES pipeline_snapshots(id),
    target_node_id TEXT NOT NULL,
    seed INTEGER NOT NULL,
    node_versions_json TEXT NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT NULL,
    output_content_identities_json TEXT NOT NULL,
    retain_intermediates INTEGER NOT NULL,
    metric_set_id TEXT NULL,
    error_summary TEXT NULL,
    failed_stage TEXT NULL
);

CREATE INDEX idx_run_records_dataset_version_id ON run_records(dataset_version_id);
CREATE INDEX idx_run_records_image_asset_id ON run_records(image_asset_id);
CREATE INDEX idx_run_records_reproducibility_lookup
    ON run_records(dataset_version_id, image_asset_id, pipeline_snapshot_id, seed, target_node_id);

CREATE TABLE metric_sets (
    id TEXT PRIMARY KEY,
    run_record_id TEXT NOT NULL REFERENCES run_records(id),
    dice REAL NULL,
    area_mm2 REAL NULL,
    foreground_pixels INTEGER NULL,
    connected_components INTEGER NULL,
    step_timings_json TEXT NOT NULL
);
