#!/usr/bin/env bash
# Verifies CHANGELOG.md contains a section for the given release tag (e.g. v0.1.0 → 0.1.0).
set -euo pipefail

tag="${1:?usage: verify-changelog.sh <tag> (e.g. v0.1.0)}"
version="${tag#v}"
changelog="${2:-CHANGELOG.md}"

if [[ ! -f "$changelog" ]]; then
  echo "error: $changelog not found" >&2
  exit 1
fi

if grep -qE "^## \[${version}\]" "$changelog" || grep -qE "^## ${version}" "$changelog"; then
  echo "ok: $changelog has entry for ${version}"
  exit 0
fi

echo "error: $changelog has no section for tag ${tag} (expected '## [${version}]' or '## ${version}')" >&2
exit 1
