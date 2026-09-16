#!/usr/bin/env bash
# Build + verify the stichos profile image (P3-30, DIS-06).
#
# Usage:  bash docker/profile/build.sh
#
# Builds the Docker image tagged stichos/profile:latest, runs
# `stichos doctor --json` inside the container to verify all tools,
# and exits 0 if all tools are available, 1 otherwise.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IMAGE="stichos/profile:latest"

echo "==> Building ${IMAGE} ..."
docker build --file "${SCRIPT_DIR}/Dockerfile" --tag "${IMAGE}" "${SCRIPT_DIR}/.."

echo "==> Running doctor inside container ..."
DOCTOR_JSON=$(docker run --rm "${IMAGE}" stichos doctor --json)
echo "${DOCTOR_JSON}"

# Check doctor result: exit 0 = all tools green
echo "${DOCTOR_JSON}" | python3 -c "
import json, sys
data = json.load(sys.stdin)
engines = data.get('engines', [])
ok = all(e.get('available', False) for e in engines if e.get('id') != 'mock')
if ok:
    print('doctor: all tools green')
    sys.exit(0)
else:
    unavailable = [e['id'] for e in engines if not e.get('available', False)]
    print(f'doctor: unavailable engines: {unavailable}', file=sys.stderr)
    sys.exit(1)
"
