#!/usr/bin/env bash
# FONTSET_HASH (INV-4, D14/D15): deterministic fingerprint of the harness font set.
#
# Method (runs inside the pinned container, so the hash describes the image's
# filesystem, not the host's):
#   for each font file under /usr/share/fonts (plus ~/.fonts if present), emit
#   "<relative-path> <sha256-of-file-bytes>", sort the lines, sha256 the aggregate.
#
# Determinism notes:
#   - Covers file BYTES and relative paths only — never mtimes, permissions, or
#     ownership. Same image ⇒ same hash across `docker run` invocations.
#   - Relative paths are rooted at `/` (e.g. usr/share/fonts/...), so the two
#     search roots cannot collide.
#   - build-info.json (built_at) is NOT part of the hash; it is provenance only.
#
# Usage: tools/fontset_hash.sh [image]   (default stichos-harness:26.2.5)
set -euo pipefail

IMAGE="${1:-stichos-harness:26.2.5}"

docker run --rm "${IMAGE}" bash -eu -o pipefail -c '
  tmp="$(mktemp)"
  for root in /usr/share/fonts "${HOME}/.fonts"; do
    [ -d "${root}" ] || continue
    ( cd "${root}" && find . -type f -print | while IFS= read -r f; do
        printf "%s %s\n" "${root#/}/${f#./}" "$(sha256sum "${root}/${f}" | cut -d" " -f1)"
      done ) >> "${tmp}"
  done
  hash="$(sort "${tmp}" | sha256sum | cut -d" " -f1)"
  rm -f "${tmp}"
  echo "fontset_hash=sha256:${hash}"
  echo "lo_version=$(jq -r .lo_version /opt/stichos/build-info.json)"
  echo "poppler_version=$(jq -r .poppler_version /opt/stichos/build-info.json)"
'
