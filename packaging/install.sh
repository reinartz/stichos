#!/usr/bin/env bash
# Generic install script for stichos.
# Usage: curl -sL https://... | sudo bash -s -- --prefix /usr/local

set -euo pipefail

PREFIX="${PREFIX:-/usr/local}"
BINARY="stichos"
VERSION="${VERSION:-0.1.0}"

echo "Installing stichos ${VERSION} to ${PREFIX}/bin/"
install -D -m755 "${BINARY}" "${PREFIX}/bin/${BINARY}"
echo "Done. Run '${BINARY} --help' to get started."
