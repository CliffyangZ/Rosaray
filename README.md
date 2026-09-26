# Rosaray

Local-first research software built around two knowledge bases — a
**Quantitative Knowledge Base (QKB)** of Python measurement algorithms and a
**Pathology Knowledge Base (KB)** — a local **System One** model (Laya) that
selects measurement algorithms and scores pathology candidates, a
**research-only pathology inference** flow with mandatory human review, and a
read-only frontend workspace. Research use only; nothing here is a diagnosis,
and model scores are unverified and are not disease probabilities.

Principles: [.specify/memory/constitution.md](.specify/memory/constitution.md) (3.0.0) ·
Active spec: [specs/004-pathology-kb-laya-inference/](specs/004-pathology-kb-laya-inference/)

> **Status**: constitution 3.0.0 is ratified; the code is mid-migration. The
> Rust crate still lives in `backend/QKB/` and QKB is still AlgoNode/AlgoPipe
> based (spec 003) until spec 004 FR-023 lands. The Python CRR algorithm and
> the frontend workspace already follow the new layout.

## Layout

| Path | What |
| --- | --- |
| `backend/QKB/` | target: Python measurement algorithms only (today `CRR/`). Also still holds the Rust crate `rosaray-qkb` (bundles, executability, catalog, System One protocol) that spec 004 moves to `backend/src/` |
| `backend/src/` | crate `rosaray-service`: thin HTTP layer (`/qkb/v1/*`); target home of the whole Rust service |
| `backend/KB/` | target: pathology knowledge entries (spec 004) |
| `model/` | local model weights, not tracked (see `model/README.md`) |
| `frontend/` | read-only workspace: Explorer, Evidence (CRR overlay), pathology inference view, Knowledge base dialog (Cmd/Ctrl+K) |
| `docs/legacy-retrieval.md` | how to read old Dataset/Run files |

## Run

```bash
cd backend && cargo run -- --project-dir ./rosaray-project
# prints {"port":…,"session_token":…}; author / System One credentials are
# owner-only files in ./rosaray-project/qkb-credentials/
cd frontend && npm install && npm run dev
# open http://localhost:5173/?rosarayPort=<port>&rosaraySession=<session_token>
cargo test --workspace   # from backend/
scripts/check-frontend-readonly.sh
```

## Behaviour in one paragraph (current Rust QKB, spec 003)

Authors submit complete AlgoNode/AlgoPipe bundles; a version is stored only if
it is currently executable (resolvable, trusted implementation, technical
verification produced by running the node's own tests, pinned dependencies).
There are no drafts. Whenever executability can change, every version that is
no longer executable — and every pipe depending on it — is permanently
deleted, with a deletion record. System One queries for candidate
AlgoPipes and reports a selection or abstention; the QKB validates it against
that query's candidate set and current eligibility and records it
idempotently. The frontend can only read.

## Legacy data

Dataset, Preview, Run, paper-review and browser-side designer flows were
retired. Existing project files (`rosaray.sqlite3*`, `blobs/`, `project.*`)
are never opened or modified; retrieve them with the `legacy-prototype` git
tag — see [docs/legacy-retrieval.md](docs/legacy-retrieval.md).
