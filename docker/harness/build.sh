#!/usr/bin/env bash
# Build + tag the pinned stichos harness image (HAR-03, D15).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

LO_VERSION="26.2.5"
IMAGE="stichos-harness:${LO_VERSION}"

docker build --file Dockerfile --tag "${IMAGE}" .

echo "tag: ${IMAGE}"
# Locally built images carry the content id digest; the registry RepoDigest
# (sha256 manifest digest) only exists after a push.
echo "image_digest=$(docker image inspect --format '{{.Id}}' "${IMAGE}")"
echo "size_bytes=$(docker image inspect --format '{{.Size}}' "${IMAGE}")"
