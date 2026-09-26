# KB test fixtures

Placeholders created in T004; filled in T024 (and T070 for the PDFs).

| Fixture | Purpose (quickstart.md Prerequisites) |
|---|---|
| `valid-spec-only-node/` | Valid specification-only AlgoNode bundle |
| `draft-pipe/` | AlgoPipe draft referencing fixture nodes |
| `corrupt-bundle/` | Malformed bundle → indexed as `invalid` with findings |
| `conflicting-duplicate/` | Same `id@version`, different content → `identity_conflict` |
| `patient-copy-bundle/` | Bundle containing a copy of a fixture Image Asset → blocked by patient-data guard |
| `text-layer.pdf` | PDF with a text layer for paper extraction |
| `scanned.pdf` | Image-only PDF → `pages_without_text`, zero candidates |

## Notes

- The bundle-shaped fixtures above are draft trees; tests copy them into a
  temporary `knowledge-base/` and publish them through the service.
- `patient-copy-bundle` and the `conflicting-duplicate` archive are built in
  the tests (a byte-copy of a project Image Asset and a same-`id@version`
  bundle with different content), because their identity depends on the
  project database.
- The PDFs arrive with user story 5 (T070).
