# SPDX-License-Identifier: MIT OR Apache-2.0
"""Register pytest markers for pyoracle tests."""

import pytest


def pytest_configure(config):
    config.addinivalue_line("markers", "integration: mark test as integration test")
