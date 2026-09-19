# SPDX-License-Identifier: MIT OR Apache-2.0
"""Machine-readable error type shared by the metrics API and CLI."""


class MetricsError(Exception):
    """Error carrying a stable machine-readable ``code``.

    Codes: ``file_not_found``, ``no_such_dir``, ``unreadable_raster``,
    ``unsupported_format``, ``not_a_zip``, ``encrypted_zip``,
    ``compressed_too_large``, ``zip_bomb``, ``xml_parse_error``,
    ``zip_read_error``, ``bad_page_count``, ``bad_json``.
    """

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code
