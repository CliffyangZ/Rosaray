-- Derived catalog: dataset-validation status is its own dimension (FR-011).
ALTER TABLE kb_catalog_entry ADD COLUMN dataset_validation TEXT NOT NULL DEFAULT 'none';
