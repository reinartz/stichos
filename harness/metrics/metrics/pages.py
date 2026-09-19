# SPDX-License-Identifier: MIT OR Apache-2.0
"""Page-count delta (C9 §5)."""
from __future__ import annotations

from .errors import MetricsError


def page_count_delta(a_n: int, b_n: int) -> dict:
    """Signed ``page_delta = a_n - b_n`` per C9 §5 (a = candidate, b = golden).

    Returns ``{"a_pages", "b_pages", "page_delta", "abs_page_delta", "pass"}``;
    PASS iff ``page_delta == 0``. Manifest ``expected.page_count_min/max``
    tolerance, when present, is applied by the caller against ``b_pages``.
    """
    if not isinstance(a_n, int) or not isinstance(b_n, int):
        raise MetricsError("bad_page_count", "page counts must be integers")
    if a_n < 0 or b_n < 0:
        raise MetricsError("bad_page_count", "page counts must be >= 0")
    delta = a_n - b_n
    return {
        "a_pages": a_n,
        "b_pages": b_n,
        "page_delta": delta,
        "abs_page_delta": abs(delta),
        "pass": delta == 0,
    }
