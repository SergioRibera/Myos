#!/usr/bin/env bash
# Put the newest published catalog (catalog.db + manifest) into
# assets/catalog/ so the app bundles it. Nothing published yet: the app
# starts without a bundled catalog. Needs GH_TOKEN.
set -euo pipefail

dest=assets/catalog
if ! gh release download catalog --repo "$GITHUB_REPOSITORY" \
    --pattern catalog.db --pattern manifest --dir "$dest" --clobber; then
  echo "::warning::no published catalog: building without a bundled catalog"
  exit 0
fi
cat "$dest/manifest"
# Flakes only see files git knows: stage the ignored downloads.
git add --force --intent-to-add "$dest/catalog.db" "$dest/manifest"
