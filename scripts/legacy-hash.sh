#!/usr/bin/env bash
# Prints sha256 of legacy Dataset/Run files (read-only). Usage: scripts/legacy-hash.sh [project-dir] > before.txt
set -euo pipefail
dir="${1:-backend/rosaray-project}"
[ -d "$dir" ] || exit 0
find "$dir" -type f \( -name 'rosaray.sqlite3*' -o -name project.salt -o -name project.verifier -o -path '*/blobs/*' \) -print0 \
  | sort -z | xargs -0 shasum -a 256
