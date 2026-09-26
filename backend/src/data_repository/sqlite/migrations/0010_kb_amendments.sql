-- Amendments (own + those affecting a pipe's dependencies) are shown in the
-- catalog (FR-051); the count is derived from the append-only chains.
ALTER TABLE kb_catalog_entry ADD COLUMN amendment_count INTEGER NOT NULL DEFAULT 0;
