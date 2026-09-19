# SPDX-License-Identifier: MIT OR Apache-2.0
"""stichos Phase-0 fidelity metrics — C9 reference implementation.

Public API (C9 §4-§7): ssim_pages, page_count_delta, feature_counts,
feature_retention, text_f1. CLI: ``python -m metrics <verb>``.
"""
from .errors import MetricsError
from .features import feature_counts, feature_retention
from .pages import page_count_delta
from .ssim import ssim_pages
from .text import text_f1, tokenize

__version__ = "0.1.0"

__all__ = [
    "MetricsError",
    "feature_counts",
    "feature_retention",
    "page_count_delta",
    "ssim_pages",
    "text_f1",
    "tokenize",
]
