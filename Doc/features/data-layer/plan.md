# Implementation Plan: Rosaray Data Layer

**Branch**: `main` | **Date**: 2026-09-22 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/001-data-layer-design/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

Design the Data Layer that is the single source of truth for images, masks,
artifacts, metrics, and Runs across Rosaray's Local Rosaray Service, so the
Execution Layer and the browser-based Presentation Layer never depend on
where data actually lives. The layer links externally-stored PNG/JPEG source
images (never copying them) into immutable, fingerprinted Dataset Versions;
separates volatile Preview artifacts from durable, fully-traceable official
Run Records; serves lightweight read descriptors to the frontend without
requiring full-pixel loads; and protects all Rosaray-managed persistent data
with default encryption, including self-contained encrypted Export Bundles.
The technical approach (Phase 0) is a single-binary Rust Local Rosaray
Service exposing a typed local HTTP+WebSocket API to the existing Vite/
vanilla-JS frontend, backed by SQLite for relational metadata and a
content-addressed encrypted blob store for pixel/artifact content.

## Technical Context

**Language/Version**: Rust 1.75+ (Local Rosaray Service: Core, Data Layer, Execution Layer, Algorithm Layer/Runtime); existing vanilla JS + Vite 7 (Browser Frontend, extended not replaced)
**Primary Dependencies**: `axum` (local HTTP + WebSocket API), `rusqlite` (SQLite metadata store), `image` (PNG/JPEG decode), `blake3` (content identity & fingerprints), `aes-gcm` + `argon2` (encryption at rest), `serde`/`serde_json` (typed API payloads, Metadata Manifest)
**Storage**: SQLite (Projects, Datasets, Dataset Versions, Image Assets, Research Subjects, Split Assignments, Reference Masks, Pipeline Snapshots, Run Records, Metric Sets, Validation Findings, Import Batches, Metadata Manifests) + content-addressed encrypted filesystem blob store (grayscale research representations, Run Input Artifacts, Thumbnail Artifacts, mask pixel data, Export Bundles)
**Testing**: `cargo test` (unit + integration) for the Local Rosaray Service; manual/exploratory verification for the existing frontend prototype (no frontend test runner exists yet; adding one is out of scope for this data-layer feature)
**Target Platform**: Single-process local service on the researcher's own workstation (macOS/Linux/Windows), bound to `127.0.0.1` only, paired with the existing browser frontend running on the same machine
**Project Type**: Local service (new `backend/`) + existing browser frontend (`frontend/`) — not a multi-tenant web application; see research.md §2 and §8
**Performance Goals**: 95% of cached image/result reads visible ≤250ms, 99% ≤1s (SC-001); Explorer list or explicit loading state ≤1s, selected image ≤1s (SC-015); local service `ready` or actionable error ≤2s (SC-017)
**Constraints**: Offline-only, no automatic network egress of research data (FR-024); encrypted-at-rest by default for all Rosaray-managed persistent data (FR-033, Constitution Principle VI); single-user, single-process, single-workstation (Assumptions); frontend may reach persisted data only through the local service API, never via local file paths (FR-040/FR-052)
**Scale/Scope**: Up to 1,000 images (≤50 megapixels each) and 500 official Runs in a representative project (spec Assumptions, SC-001)

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|---|---|---|
| I. Local-First, Offline-by-Default | PASS | Local service binds to loopback only; no dependency introduced requires network access (research.md §1–§2); FR-024 carried through unchanged. |
| II. Data Integrity & Immutability | PASS | Dataset Version fingerprinting (research.md §4) and SQLite transactional writes (research.md §3) directly implement immutable-version-on-change and all-or-nothing Run completion (FR-007, FR-020, FR-032). |
| III. Preview/Official Separation | PASS | Thumbnail/Preview caching design (research.md §7) keeps Preview and Thumbnail artifacts reconstructible and excluded from official storage and Export Bundles by construction. |
| IV. Privacy by Default | PASS | No design decision here introduces direct-identifier storage; Metadata Manifest (research.md §6) carries only de-identified `patient_id` fields, matching FR-025. |
| V. Traceability & Reproducibility | PASS | Content-addressed blob store + BLAKE3 identities (research.md §3–§4) give every artifact and Run a verifiable content identity chain per FR-018/FR-022. |
| VI. Encryption & Confidentiality | PASS | Envelope encryption design (research.md §5) implements default-at-rest encryption and a structurally separate Export Bundle credential (FR-033–FR-035). |

No violations identified; Complexity Tracking table below is not required at
this stage and is left empty pending Phase 1 design review.

**Post-Phase 1 re-check**: PASS — see [Post-Phase 1 Constitution Re-check](#post-phase-1-constitution-re-check) below. The data model, contracts, and quickstart produced in Phase 1 did not introduce any new dependency, storage location, or API surface beyond what this table already evaluated.

## Project Structure

### Documentation (this feature)

```
specs/001-data-layer-design/
├── plan.md              # This file (/speckit.plan command output)
├── research.md          # Phase 0 output (/speckit.plan command)
├── data-model.md         # Phase 1 output (/speckit.plan command)
├── quickstart.md         # Phase 1 output (/speckit.plan command)
├── contracts/            # Phase 1 output (/speckit.plan command)
│   ├── local-service-api.md    # Typed Command/Query HTTP API (frontend ↔ service)
│   └── event-bus.md            # Async WebSocket notification contract
└── tasks.md              # Phase 2 output (/speckit.tasks command - NOT created by /speckit.plan)
```

### Source Code (repository root)

This feature introduces a new local service alongside the existing frontend
prototype; it does not restructure either into a different project type.

```
backend/                        # NEW — Local Rosaray Service (Rust)
├── Cargo.toml
├── src/
│   ├── main.rs                 # Service bootstrap, session token issuance
│   ├── api/                    # axum routes: typed Command/Query handlers
│   ├── events/                 # WebSocket Event Bus (non-persistent)
│   ├── data_engine/             # Import, Dataset Version creation, validation, export
│   │   ├── import.rs
│   │   ├── dataset_version.rs
│   │   ├── validation.rs
│   │   └── export.rs
│   ├── data_repository/         # Query/persistence abstraction only (FR-051)
│   │   ├── sqlite/              # Relational metadata store
│   │   └── blob_store/          # Content-addressed encrypted artifact store
│   ├── crypto/                  # Envelope encryption (project key, export key)
│   └── domain/                  # Entities from data-model.md (shared types)
└── tests/                       # cargo integration tests per quickstart.md scenario

frontend/                       # EXISTING — extended, not replaced
└── src/
    ├── service_client.js       # NEW — typed HTTP client + WebSocket subscriber
    ├── main.js                 # EXTENDED — consume Image Display Descriptors
    └── registry.js             # EXTENDED — Preview requests via service_client
```

**Structure decision**: Two sibling top-level directories (`backend/`,
`frontend/`) rather than a single project, because the Data Layer's
Constitution-mandated boundary (frontend never touches storage directly,
FR-040/FR-052) only holds if the service is a separately-built, separately-
run process — not a library linked into the frontend build. Within
`backend/`, `data_engine/` vs. `data_repository/` is a direct implementation
of FR-051's split between business rules and persistence abstraction, not a
speculative layering choice.

## Complexity Tracking

*Only fill out if the Constitution Check has violations that must be justified.*

No entries — the Constitution Check above found no violations requiring
justification.

## Post-Phase 1 Constitution Re-check

Re-evaluated after producing `data-model.md`, `contracts/local-service-api.md`,
`contracts/event-bus.md`, and `quickstart.md`:

| Principle | Status | Notes |
|---|---|---|
| I. Local-First, Offline-by-Default | PASS | `local-service-api.md` confirms every endpoint is loopback-bound; no external calls appear in any contract. |
| II. Data Integrity & Immutability | PASS | `data-model.md` Dataset Version and Run Record entities carry `immutable: true` fields and derivation links (`derived_from_version_id`), matching FR-007/FR-032. |
| III. Preview/Official Separation | PASS | `local-service-api.md` puts Preview and official Run endpoints on distinct resource paths (`/preview/*` vs `/runs/*`) with different persistence semantics; Export contract explicitly excludes Preview/Thumbnail content. |
| IV. Privacy by Default | PASS | No entity in `data-model.md` has a direct-identifier field; `patient_id` is documented as researcher-supplied de-identified text in every entity that references it. |
| V. Traceability & Reproducibility | PASS | Run Record entity carries the full identity chain (dataset version fingerprint, image identity, run input artifact identity, pipeline snapshot identity, seed, node versions) required by FR-018/FR-022. |
| VI. Encryption & Confidentiality | PASS | `contracts/local-service-api.md` error taxonomy includes `credential_required`/`credential_invalid` responses that refuse partial disclosure, matching FR-034. |

No new violations were introduced during Phase 1 design; Complexity Tracking
remains empty.
