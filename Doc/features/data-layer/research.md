# Phase 0 Research: Rosaray Data Layer

**Input**: [spec.md](./spec.md), [System Overview](../../architecture/overview.md), existing frontend prototype (`frontend/`)

This document resolves every `NEEDS CLARIFICATION` left open by the Technical
Context and records the key design decisions the Data Layer plan depends on.
Format per decision: **Decision / Rationale / Alternatives considered**.

## 1. Local Rosaray Service language & runtime

- **Decision**: Rust (stable, 1.75+), compiled to a single native binary that
  hosts the entire Local Rosaray Service (Core, Data Layer, Execution Layer,
  Algorithm Layer/Runtime per `system_architecture.canvas`).
- **Rationale**: The service must run entirely offline on a researcher's own
  workstation, manage encrypted persistent state, decode/process
  medium-to-large images (up to 50MP) with predictable latency (SC-001,
  SC-015), and eventually host ONNX inference (`ONNX-Runtime` node in the
  canvas). Rust gives memory-safety without a GC pause budget to manage,
  strong crates for cryptography, SQLite, and image decoding, and a single
  static binary that is trivial to ship alongside the existing Vite frontend
  with no separate runtime install. This also matches the project's own
  forward plan captured for the future Rust backend.
- **Alternatives considered**: Node.js/TypeScript (would unify language with
  the frontend, but weaker for CPU-bound image validation/fingerprinting and
  for eventual native GPU/ONNX integration; leaves memory-safety and
  crypto-misuse risks to userland libraries). Python (fastest to prototype,
  but poor single-binary distribution story for a local-first desktop tool
  and weaker compile-time guarantees for the immutability/integrity rules in
  Constitution Principle II). Go (viable, but weaker first-party ecosystem
  for the encrypted embedded-database + ONNX runtime bindings this service
  will need later).

## 2. Local service transport (frontend ↔ service boundary)

- **Decision**: A local-only HTTP API (bound to `127.0.0.1`, random ephemeral
  port advertised to the frontend at launch) for typed Command/Query
  request-response calls, plus a single WebSocket connection per browser tab
  for the Event Bus's async status/lifecycle notification stream. Every
  request carries a per-launch session token (delivered out-of-band, e.g. via
  a local file or the process that launches the browser) so an unrelated
  local process cannot silently query the service.
- **Rationale**: Directly implements FR-040/FR-041/FR-050: typed
  request-response for Commands/Queries, a separate non-persistent channel
  for async notifications, and no filesystem/path leakage to the frontend.
  HTTP+WebSocket is well supported by both Rust (`axum`) and the existing
  vanilla-JS frontend (`fetch`/`WebSocket`) with no new frontend framework
  dependency. Binding to loopback only and requiring a session token
  satisfies the `connecting` / `ready` / `unavailable` / `access_denied`
  Local Service Session states (FR-042) without needing OS-level IPC.
- **Alternatives considered**: gRPC (stronger typing, but adds a codegen
  toolchain to a vanilla-JS frontend for little benefit at this scale). Raw
  Unix domain socket / named pipe (avoids any network exposure, but
  complicates the browser side, which cannot open sockets directly; would
  still need an HTTP shim). Exposing Event Bus messages directly over the
  same request-response channel (rejected — violates FR-050's separation of
  typed API vs. non-persistent notification stream).

## 3. Persistent storage engine

- **Decision**: Embedded SQLite (via `rusqlite`) for all structured,
  relational metadata (Projects, Datasets, Dataset Versions, Image Assets,
  Research Subjects, Split Assignments, Reference Masks, Pipeline Snapshots,
  Run Records, Metric Sets, Validation Findings, Import Batches, Metadata
  Manifests), plus a content-addressed encrypted blob store on the local
  filesystem for large binary content (grayscale research representations,
  Run Input Artifacts, Thumbnail Artifacts, reference mask pixel data,
  Export Bundle payloads).
- **Rationale**: SQLite is transactional, requires no separate server
  process (keeps the offline/single-process constraint), and gives the
  read-your-writes consistency needed for Constitution Principle II
  (immutability) and FR-020 (a Run's completion state and its data must
  become durable together, inside one transaction). Large pixel content does
  not belong in relational rows; a content-addressed blob store keyed by the
  content identity already required everywhere in the spec (FR-003, FR-010,
  FR-031) gives natural deduplication and makes integrity checks (FR-023) a
  simple hash-recompute-and-compare.
- **Alternatives considered**: A single embedded KV/document store (e.g.
  `sled`) for everything — rejected because the relational shape (datasets →
  versions → images → splits/masks → runs → metrics) and the need for
  leakage-validation queries (FR-008, joins across subjects/splits) fit SQL
  far better than a KV store. Storing blobs as SQLite BLOBs — rejected for
  images up to 50MP × up to 1000 per project; filesystem storage keeps the
  metadata DB small, fast to back up, and lets partial/streamed reads avoid
  loading full pixel buffers just to serve a list (FR-036).

## 4. Content identity & dataset fingerprint

- **Decision**: BLAKE3 as the content-hash function for image content
  identity, artifact content identity, and as the leaf hash in a Merkle-style
  combination for the Dataset Version fingerprint (hash of the sorted list of
  `{image content identity, patient id, split, reference mask content
  identity}` tuples that affect research interpretation, per FR-007).
- **Rationale**: BLAKE3 is fast enough to hash up to 50MP images without
  becoming the bottleneck implied by SC-001's 250ms/1s targets, is
  cryptographically strong enough for tamper-evidence (SC-007, SC-013), and
  has a mature, audited Rust crate. Restricting the fingerprint inputs to
  fields the spec explicitly says affect interpretation (content, patient
  mapping, split, mask relationship — not display name) directly satisfies
  FR-007's "display-only changes must not create a new version" rule.
- **Alternatives considered**: SHA-256 (equally strong, but slower on large
  images with no offsetting benefit here). Including all metadata fields
  (including display name) in the fingerprint — rejected, contradicts FR-007
  and would create spurious Dataset Versions on cosmetic renames.

## 5. Encryption at rest

- **Decision**: Envelope encryption. Each project has a master key derived
  from a user-supplied passphrase via Argon2id; the master key wraps
  per-artifact/per-row data-encryption keys (AES-256-GCM) so individual
  blobs and metadata pages can be re-keyed or selectively revoked without
  re-encrypting the whole project. Export Bundles use a wholly separate
  key hierarchy derived from an export-specific credential that is never
  written into the bundle (FR-035).
- **Rationale**: Directly implements Constitution Principle VI and FR-033/
  FR-034/FR-035: default-encrypted managed data, safe refusal without partial
  disclosure when a credential can't be verified, and a bundle credential
  that is structurally incapable of being embedded in its own bundle because
  it belongs to an unrelated key hierarchy.
- **Alternatives considered**: A single static, non-derived project key
  stored on disk — rejected, does not meet "encrypted at rest" in any
  meaningful sense since compromise of the disk trivially compromises the
  key. Full-disk encryption as the only protection — rejected per FR-033,
  which explicitly scopes Rosaray's own encryption responsibility to data it
  persists, independent of OS/disk-level protection the researcher may or
  may not have.

## 6. Metadata Manifest format (Import Batch)

- **Decision**: A versioned JSON document (`manifest_version`, then an array
  of `{source_ref, patient_id, split, reference_mask_ref}` entries), matched
  to candidate images by `source_ref` (relative path or filename as scanned
  in the Import Batch) rather than by any inferred pattern.
- **Rationale**: FR-045/FR-046 require an explicit, versioned, exchangeable
  mapping format and forbid guessing patient/split/mask from filenames or
  folder names. JSON with an explicit version field lets future manifest
  schema changes be detected and rejected safely rather than silently
  mis-mapped, and matching purely by explicit `source_ref` keeps every
  mapping traceable to a manifest line rather than a heuristic.
- **Alternatives considered**: CSV — viable and considered as an additional
  accepted format in a later iteration, but JSON was chosen as the required
  baseline since it can represent the mapping unambiguously without a
  column-dialect problem (delimiters, quoting) that could introduce silent
  mis-parses in a compliance-sensitive mapping step.

## 7. Thumbnail generation & caching

- **Decision**: Thumbnails are derived on-demand per Image Asset content
  identity, generated by the Data Engine from the grayscale research
  representation, written to the Memory Cache first and only persisted to
  disk as a reconstructible, non-authoritative cache entry (never a Run
  input). Generation requests are scoped to the currently visible/adjacent
  Explorer range and are cancellable (FR-047/FR-048).
- **Rationale**: Matches the spec's explicit rule that Thumbnail Artifacts
  are reconstructible, content-identity-bound, and never usable for official
  Runs or measurements (Key Entities: Thumbnail Artifact) while keeping
  Explorer list-open cheap (FR-036/FR-047: no full-image or full-thumbnail
  load on list open).
- **Alternatives considered**: Eagerly generating all thumbnails at import
  time — rejected, contradicts FR-047 and would regress SC-018's need for a
  fast Import Batch Preview independent of thumbnail cost. Storing
  thumbnails as first-class persistent artifacts equivalent to Run Input
  Artifacts — rejected, blurs the Preview/official separation the Key
  Entities section explicitly draws for this artifact type.

## 8. Frontend integration boundary

- **Decision**: The existing Vite/vanilla-JS frontend (`frontend/src/`)
  gains a new local-service client module rather than a framework migration;
  it calls the typed HTTP API for Commands/Queries and subscribes to the
  WebSocket Event Bus stream for async status. Presentation/rendering
  components (`main.js`, `registry.js`) are extended to consume Image
  Display Descriptors instead of the in-browser sample data they use today.
- **Rationale**: FR-052 requires Presentation Engine/Image Viewer/Renderers
  to run in the browser frontend and reach persisted data only through the
  local service API — this is achievable by adding a client module to the
  current codebase without introducing a new frontend framework or build
  toolchain, minimizing unrelated churn to a prototype the team already
  built.
- **Alternatives considered**: Rewriting the frontend in a framework (React/
  Vue) as part of this feature — rejected as out of scope; the spec's User
  Story 2 acceptance criteria are about data/state contracts, not UI
  framework, and a rewrite would be an unjustified scope increase per the
  project's "no speculative abstraction" working norm.

## Summary of resolved Technical Context

| Field | Resolution |
|---|---|
| Language/Version | Rust 1.75+ (Local Rosaray Service); existing vanilla JS + Vite 7 (frontend, extended not replaced) |
| Primary Dependencies | `axum` (HTTP+WebSocket), `rusqlite` (SQLite), `image` (PNG/JPEG decode), `blake3`, `aes-gcm`, `argon2`, `serde`/`serde_json` |
| Storage | SQLite (metadata) + content-addressed encrypted filesystem blob store (pixel/artifact content) |
| Testing | `cargo test` (unit + integration) for the service; manual/exploratory verification for the existing frontend prototype (no test runner present yet — out of scope to add one here) |
| Target Platform | Single-process local service on the researcher's workstation (macOS/Linux/Windows), loopback-only, paired with the existing browser frontend |
| Project Type | Local service ("backend/") + existing browser frontend ("frontend/") — not a multi-tenant web app |
| Performance Goals | Per SC-001/SC-015/SC-017: 250ms p95 / 1s p99 cached reads; Explorer list ≤1s; service reconnect ready ≤2s |
| Constraints | Offline-only (FR-024), encrypted-at-rest by default (FR-033), single-user single-process, loopback-only API |
| Scale/Scope | Up to 1,000 images (≤50MP each) and 500 Runs per project (spec Assumptions) |
