#!/usr/bin/env bash
# ed25519 signing and verification of the catalog manifest with the
# OpenSSL CLI, so the jobs holding the signing key or vetting downloads run
# no Rust build (no third-party build scripts next to the secret).
#
#   catalog-sig.sh public-key            → hex key from src/reference.rs ("" if unset)
#   catalog-sig.sh sign DIR              → DIR/manifest.sig (seed in $CATALOG_SIGNING_KEY)
#   catalog-sig.sh verify DIR PUBKEY     → checks DIR/manifest.sig and every
#                                          manifest file present in DIR
set -euo pipefail

der_public() { echo "302a300506032b6570032100$1" | xxd -r -p; }

case "${1:-}" in
  public-key)
    sed -n 's/^const PUBLIC_KEY: &str = "\([0-9a-f]*\)";$/\1/p' src/reference.rs
    ;;
  sign)
    dir=$2
    seed=${CATALOG_SIGNING_KEY:?CATALOG_SIGNING_KEY is not set}
    key=$(mktemp)
    trap 'rm -f "$key"' EXIT
    echo "302e020100300506032b657004220420$seed" | xxd -r -p > "$key"
    openssl pkeyutl -sign -inkey "$key" -keyform DER -rawin \
      -in "$dir/manifest" -out "$dir/manifest.sig.bin"
    xxd -p -c128 "$dir/manifest.sig.bin" > "$dir/manifest.sig"
    rm "$dir/manifest.sig.bin"
    # Public half, for the caller to compare with the app's key.
    openssl pkey -inform DER -in "$key" -pubout -outform DER | tail -c32 | xxd -p -c64
    ;;
  verify)
    dir=$2
    pub=$3
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    der_public "$pub" > "$tmp/key.der"
    xxd -r -p "$dir/manifest.sig" > "$tmp/sig.bin"
    openssl pkeyutl -verify -pubin -inkey "$tmp/key.der" -keyform DER -rawin \
      -in "$dir/manifest" -sigfile "$tmp/sig.bin" > /dev/null
    head -1 "$dir/manifest" | grep -qx 'myos-catalog-manifest 1'
    # catalog <file> <schema> <version> <size> <sha>
    # food-facts <cc> <file> <schema> <version> <size> <sha>
    tail -n +2 "$dir/manifest" | while read -r kind a b _ d e f; do
      case "$kind" in
        catalog) file=$a size=$d sha=$e ;;
        food-facts) file=$b size=$e sha=$f ;;
        *) echo "unknown manifest line: $kind" >&2; exit 1 ;;
      esac
      [ -f "$dir/$file" ] || continue
      [ "$(stat -c %s "$dir/$file")" = "$size" ] || { echo "$file: size mismatch" >&2; exit 1; }
      echo "$sha  $dir/$file" | sha256sum --check --quiet
    done
    echo "manifest and files verified"
    ;;
  *)
    echo "usage: catalog-sig.sh public-key | sign DIR | verify DIR PUBKEY" >&2
    exit 2
    ;;
esac
