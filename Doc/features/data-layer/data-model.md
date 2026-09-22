# Data Model: Rosaray Data Layer

Source: [spec.md § Key Entities](./spec.md#key-entities-include-if-feature-involves-data),
grounded by the Functional Requirements referenced inline. Types are given in
Rust-ish notation for the Local Rosaray Service; the same shapes are exposed
to the frontend as JSON via the Local Service API contract (`contracts/local-service-api.md`).

## Entity overview

```
Project 1─* Dataset 1─* DatasetVersion 1─* ImageAsset *─1 ResearchSubject
                                   │              │
                                   │              └─0..1─ ReferenceMask
                                   │
                                   ├─* ValidationFinding
                                   └─1 DatasetFingerprint

DatasetVersion 1─* RunRecord *─1 PipelineSnapshot
RunRecord 1─1 RunInputArtifact
RunRecord 1─* MetricSet
RunRecord *─* Artifact (final outputs)

ImportBatch 0..1─1 MetadataManifest
ImportBatch *─* ImageAsset (candidates, pre-confirmation)

ImageAsset 1─* ThumbnailArtifact (content-identity-keyed, reconstructible)
Project 1─* ExportBundle (selection of DatasetVersions + RunRecords)
```

## Entities

### Project

| Field | Type | Notes |
|---|---|---|
| `id` | UUID | Stable identity. |
| `display_name` | String | Mutable, does not affect any fingerprint. |
| `created_at` | Timestamp | |
| `contract_version` | SemVer string | Data contract version for migration checks (spec Assumptions, migration note). |

**Validation rules**: `contract_version` must be recognized by the running
service before any read/write proceeds (spec Edge Cases: unsupported
contract version must not load blindly).

### Dataset

| Field | Type | Notes |
|---|---|---|
| `id` | UUID | Long-lived identity, independent of any version. |
| `project_id` | UUID (FK) | |
| `display_name` | String | Mutable; FR-007 excludes this from fingerprint. |
| `latest_version_id` | UUID (FK → DatasetVersion, nullable) | |

### DatasetVersion

| Field | Type | Notes |
|---|---|---|
| `id` | UUID | |
| `dataset_id` | UUID (FK) | |
| `fingerprint` | BLAKE3 hash (hex) | Per research.md §4; covers image content identity + patient mapping + split + mask relationships only. |
| `derived_from_version_id` | UUID (FK, nullable) | Set for every version after the first (FR-032). |
| `immutable` | `true` (const) | Enforced at the Data Repository layer: no UPDATE is ever issued against a persisted DatasetVersion row, only INSERTs of new versions. |
| `created_at` | Timestamp | |
| `image_asset_ids` | `Vec<UUID>` | Membership snapshot at this version. |
| `validation_summary` | `{ status: ok \| blocked \| warned, finding_ids: Vec<UUID> }` | Derived from associated ValidationFindings (FR-008). |

**State transitions**: `DatasetVersion` has no in-place transitions — it is
created once, complete, or not at all (FR-044: no partial version on cancel/
failure). "Change" always means "create a new `DatasetVersion` with
`derived_from_version_id` set."

**Validation rules**:
- FR-007: any diff in `image_asset_ids` content identity, patient mapping,
  split, or reference mask relationship vs. `derived_from_version_id` MUST
  produce a different `fingerprint` and thus a new row; a `display_name`-only
  change on the parent `Dataset` MUST NOT.
- FR-032: existing `RunRecord.dataset_version_id` and `ValidationFinding`
  rows referencing an older version are never rewritten to point at a newer
  one.

### ImageAsset

| Field | Type | Notes |
|---|---|---|
| `id` | UUID | Stable identity, independent of dataset version membership. |
| `external_source_uri` | String | Link only — file is never copied (FR-002). |
| `source_content_identity` | BLAKE3 hash | Computed at import time; re-verified before each use (FR-031). |
| `imported_content_identity` | BLAKE3 hash | Grayscale research representation identity, reconstructible from source. |
| `dimensions` | `{ width: u32, height: u32 }` | |
| `source_created_at` | Timestamp (nullable) | From filesystem metadata if available. |
| `imported_at` | Timestamp | |
| `status` | enum: `available \| source_missing \| source_changed` | FR-031. |
| `patient_id` | String (nullable) | De-identified only (FR-025); nullable = incomplete. |
| `split` | enum: `train \| validation \| test` (nullable) | Nullable = incomplete. |
| `reference_mask_id` | UUID (FK → ReferenceMask, nullable) | |
| `metadata_status` | enum: `complete \| incomplete` | Derived from `patient_id`/`split`/`reference_mask_id` presence (FR-004, FR-008). |

**State transitions**:

```
available ──(source file moved/deleted, checked lazily)──▶ source_missing
available ──(source content identity changed on re-check)──▶ source_changed
source_missing/source_changed ──(source restored & re-verified)──▶ available
```

Both `source_missing` and `source_changed` MUST block new Preview/official
Run creation from this `ImageAsset` (FR-031); prior `RunRecord`s remain
readable via their own `RunInputArtifact`.

### ResearchSubject

| Field | Type | Notes |
|---|---|---|
| `id` | UUID | |
| `dataset_id` | UUID (FK) | Scope of the leakage constraint (per-dataset, per spec Assumptions). |
| `deidentified_patient_id` | String | Same value as `ImageAsset.patient_id` for its images. |

**Validation rule**: within one `Dataset`, all `ImageAsset`s sharing a
`ResearchSubject` MUST have the same `split` value; a violation produces a
`ValidationFinding` of category `patient_split_leakage` and blocks the
dataset from being marked usable for official evaluation (spec Acceptance
Scenario US1-2).

### SplitAssignment

Modeled as the `split` field on `ImageAsset` plus its aggregation into
`ResearchSubject` leakage checks — not a separate table, to avoid a
redundant join for a single enum value per image. (Kept as a named concept
in the spec's Key Entities for domain clarity; represented here as an
attribute rather than a row to avoid an unnecessary layer, consistent with
project norms against speculative abstraction.)

### ReferenceMask

| Field | Type | Notes |
|---|---|---|
| `id` | UUID | |
| `content_identity` | BLAKE3 hash | |
| `dimensions` | `{ width: u32, height: u32 }` | |
| `compatible_with_image_id` | UUID (FK → ImageAsset) | |
| `validity` | enum: `valid \| incompatible_dimensions \| unreadable` | Computed at pairing time. |

**Validation rule**: `Dice`/reference-based metrics MUST NOT be computed
unless `validity == valid` and `compatible_with_image_id` matches the image
being scored (FR-038, Edge Cases).

### PipelineSnapshot

| Field | Type | Notes |
|---|---|---|
| `id` | UUID | = pipeline graph identity for this snapshot. |
| `graph_identity` | BLAKE3 hash | Canonical serialization of nodes + typed edges (excludes UI-only state per FR-014). |
| `node_versions` | `Map<node_id, implementation_version>` | |
| `canonical_parameters` | `Map<node_id, Value>` | Only content-affecting parameters (FR-014). |
| `immutable` | `true` (const) | |

### RunInputArtifact

| Field | Type | Notes |
|---|---|---|
| `id` | UUID | |
| `content_identity` | BLAKE3 hash | Of the actual grayscale bytes fed to the pipeline. |
| `source_image_asset_id` | UUID (FK) | Link retained even if the source `ImageAsset` later becomes `source_missing` (FR-019, Edge Cases: existing Run stays reproducible). |
| `created_at` | Timestamp | |

### RunRecord

| Field | Type | Notes |
|---|---|---|
| `id` | UUID | Unique run ID. |
| `status` | enum: `running \| succeeded \| failed \| cancelled` | FR-020/FR-021. |
| `dataset_version_id` | UUID (FK) | Immutable reference (FR-018). |
| `dataset_fingerprint` | BLAKE3 hash | Denormalized copy at run time, for archival stability even if the DB row is later pruned (not rewritten). |
| `image_asset_identity` | BLAKE3 hash | Copy of the `ImageAsset.imported_content_identity` used. |
| `run_input_artifact_id` | UUID (FK) | |
| `pipeline_snapshot_id` | UUID (FK) | |
| `seed` | u64 | |
| `node_versions` | `Map<node_id, implementation_version>` | Copy from `PipelineSnapshot` at run time. |
| `started_at` / `ended_at` | Timestamp / Timestamp (nullable) | `ended_at` is null while `status == running`. |
| `output_artifact_ids` | `Vec<UUID>` | Final artifacts only; intermediate retention governed by `run_policy`. |
| `run_policy` | `{ retain_intermediates: bool }` | Part of the Run Record itself (FR-019). |
| `metric_set_id` | UUID (FK, nullable) | Null while running or if failed before metrics computed. |
| `error_summary` | String (nullable) | Present only when `status == failed`; MUST NOT include raw image content or non-essential subject info (FR-021). |

**Validation rule (atomicity)**: a `RunRecord` transitions to `succeeded`
only inside the same storage transaction that persists
`run_input_artifact_id`, `output_artifact_ids`, and `metric_set_id`; any
interruption leaves it `failed`/`running`, never a partially-populated
`succeeded` row (FR-020, Constitution Principle II).

### Artifact / ArtifactReference

| Field | Type | Notes |
|---|---|---|
| `id` | UUID | |
| `content_identity` | BLAKE3 hash | |
| `kind` | enum: `image \| mask \| measurement \| other` | |
| `persistence` | enum: `preview \| official` | Constitution Principle III boundary lives here. |
| `data_type` / `shape` / `size_bytes` | as applicable | |
| `created_at` | Timestamp | |
| `produced_by` | `{ node_id, run_or_preview_context_id }` | Lineage (FR-016). |

`ArtifactReference` (as seen by callers, including the frontend) is the same
shape minus raw content — it is the identity + metadata used to request
content through the API, never a way to embed pixels directly (spec Key
Entities: Artifact Reference).

### ImageDisplayDescriptor

Read-only, composed on request — not a stored table. Shape:

```
{
  dataset_version_id, image_asset_id,
  display_metadata: { patient_id, split, dimensions },
  source_status: available | source_missing | source_changed,
  validation_summary,
  image_artifact_ref: ArtifactReference,
  reference_mask_ref: ArtifactReference | null,
  reference_mask_unavailable_reason: string | null
}
```

Never includes full pixel data (FR-036).

### ThumbnailArtifact

| Field | Type | Notes |
|---|---|---|
| `source_image_content_identity` | BLAKE3 hash | Binding key (FR-049). |
| `content_identity` | BLAKE3 hash | Of the thumbnail bytes themselves. |
| `state` | enum: `ready \| stale \| generating \| placeholder` | FR-049, Edge Cases (missing/expired → placeholder, never another asset's thumbnail). |

Never referenced by any `RunRecord` or `MetricSet` (Key Entities: explicitly
not usable for official Runs/measurement).

### LocalServiceSession

Not persisted — in-memory per WebSocket/HTTP-session connection.

| Field | Type | Notes |
|---|---|---|
| `session_token` | Opaque string | Issued at service launch (research.md §2). |
| `state` | enum: `connecting \| ready \| unavailable \| access_denied` | FR-042. |

### ImportBatch

| Field | Type | Notes |
|---|---|---|
| `id` | UUID | |
| `source_selection` | enum: `single_file \| multi_file \| folder_scan` | FR-043. |
| `candidates` | `Vec<ImportCandidate>` | See below. |
| `metadata_manifest_id` | UUID (FK, nullable) | |
| `status` | enum: `previewing \| confirmed \| cancelled \| failed` | FR-044. |

`ImportCandidate`: `{ source_ref, classification: importable | skipped | duplicate | unreadable | unsupported, resolved_patient_id, resolved_split, resolved_mask_ref, manifest_match: matched | unmatched | ambiguous }`.

**Validation rule**: no `ImageAsset` or `DatasetVersion` row is written until
`status` becomes `confirmed`; `cancelled`/`failed` leaves zero rows (FR-044,
SC-019).

### MetadataManifest

| Field | Type | Notes |
|---|---|---|
| `id` | UUID | |
| `manifest_version` | SemVer string | research.md §6. |
| `entries` | `Vec<{ source_ref, patient_id, split, reference_mask_ref }>` | Matched to `ImportCandidate.source_ref` only — never inferred from filename/folder (FR-046). |

### MetricSet

| Field | Type | Notes |
|---|---|---|
| `run_record_id` | UUID (FK) | |
| `dice` | f64 (nullable) | Null when no valid reference mask (FR-038). |
| `area_mm2`, `foreground_pixels`, `connected_components` | numeric | |
| `step_timings` | `Map<node_id, duration_ms>` | |

### ValidationFinding

| Field | Type | Notes |
|---|---|---|
| `id` | UUID | |
| `dataset_version_id` | UUID (FK) | |
| `category` | enum: `patient_split_leakage \| missing_patient_id \| missing_or_incompatible_mask \| duplicate_content \| incomplete_metadata` | |
| `severity` | enum: `blocking \| warning` | FR-008. |
| `affected_image_asset_ids` | `Vec<UUID>` | |

### ExportBundle

| Field | Type | Notes |
|---|---|---|
| `id` | UUID | |
| `manifest` | `{ dataset_version_ids, run_record_ids, content_list, integrity_proof }` | FR-026. |
| `credential_key_id` | Opaque reference | Points to a key in the export-specific key hierarchy (research.md §5); never embedded in the bundle itself (FR-035). |
| `excludes_preview` | `true` (const) | Enforced at export-build time. |

## Cross-entity invariants (traceability checklist)

Every row below must be resolvable from any final `MetricSet` or output
`Artifact`, per FR-022 / SC-004:

`MetricSet → RunRecord → {DatasetVersion, RunInputArtifact → ImageAsset, PipelineSnapshot}`
