# Retrieving legacy Dataset / Run data

Rosaray's QKB refactor (spec 003) removed the Dataset, Preview, Run, paper-review
and browser-side designer flows. Existing project files are **not** modified,
migrated or deleted by the new system:

- `backend/rosaray-project/rosaray.sqlite3*`
- `backend/rosaray-project/blobs/`
- `backend/rosaray-project/project.salt`, `project.verifier`

To read them, use the legacy tools:

```bash
git checkout legacy-prototype        # snapshot taken before the refactor
cd backend && cargo run               # legacy service opens rosaray-project/
cd ../frontend && npm install && npm run dev
```

Return with `git checkout main`. Do not run the new service and the legacy
service against the same files at the same time. Only
`rosaray-project/knowledge-base/` is subject to QKB cleanup (non-executable
method versions and drafts are permanently deleted there).
