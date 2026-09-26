-- Paper-derived bundles stay distinguishable in the catalog (FR-027).
ALTER TABLE kb_catalog_entry ADD COLUMN origin TEXT NULL;
