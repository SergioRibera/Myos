#!/usr/bin/env bash
# Put the current published catalog (catalog.db + manifest) into
# assets/catalog/ so the app bundles it, after checking the manifest
# signature against the app's public key and the file against the
# manifest. No key in the app or nothing published: build without a
# bundled catalog. Needs GH_TOKEN.
set -euo pipefail

dest=assets/catalog
pub=$(bash .github/scripts/catalog-sig.sh public-key)
if [ -z "$pub" ]; then
  echo "::warning::PUBLIC_KEY unset in src/reference.rs: building without a bundled catalog"
  exit 0
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
if ! gh release download catalog --repo "$GITHUB_REPOSITORY" \
    --pattern manifest --pattern manifest.sig --dir "$tmp"; then
  echo "::warning::no published catalog: building without a bundled catalog"
  exit 0
fi
# The manifest names the immutable release of this version: files and
# manifest come from one publish even while the moving release updates.
version=$(awk '$1 == "catalog" { print $4 }' "$tmp/manifest")
gh release download "catalog-v$version" --repo "$GITHUB_REPOSITORY" \
  --pattern catalog.db --pattern manifest --pattern manifest.sig --dir "$tmp" --clobber
bash .github/scripts/catalog-sig.sh verify "$tmp" "$pub"
cp "$tmp/catalog.db" "$tmp/manifest" "$tmp/manifest.sig" "$dest/"
# Flakes only see files git knows: stage the ignored downloads.
git add --force "$dest/catalog.db" "$dest/manifest" "$dest/manifest.sig"
