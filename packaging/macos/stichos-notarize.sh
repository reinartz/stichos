#!/usr/bin/env bash
# macOS notarization script.
# Owner-bound: requires Apple Developer account + certificates.
# For M1, notarization is flagged as owner-bound (not completed by agent).
echo "Notarization: owner-bound (requires Apple Developer account)"
echo "Command would be: xcrun notary store-submit stichos.pkg --keychain-profile stichos"
