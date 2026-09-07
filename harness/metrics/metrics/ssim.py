"""Per-page SSIM vs golden (C9 §4).

Normative parameters: grayscale 8-bit rasters, 150 DPI, 11x11 Gaussian
window sigma=1.5, c1=(0.01*255)^2, c2=(0.03*255)^2, data_range=255,
skimage flags ``gaussian_weights=True, use_sample_covariance=False``.
Sizes differing: both images are trimmed to the common per-axis minimum
dimension (C9 §4 "Window" row) and the page is flagged ``size_mismatch``.
Pages present on only one side get ``ssim = 0.0`` and the ``missing``
flag; per C9 §4 the unaligned tail is not scored, so ``min``/``mean``
are aggregates over aligned pages only.
"""
from __future__ import annotations

import re
from pathlib import Path

import numpy as np
from PIL import Image
from skimage.metrics import structural_similarity as _structural_similarity

from .errors import MetricsError

_RASTER_EXTS = frozenset({".png", ".pgm", ".pbm", ".ppm", ".pnm", ".tif", ".tiff"})
_WIN_SIZE = 11
_SIGMA = 1.5
_DATA_RANGE = 255


def _natural_key(name: str) -> list:
    parts = re.split(r"(\d+)", name)
    return [int(p) if p.isdigit() else p.lower() for p in parts]


def _list_pages(directory: Path) -> list[Path]:
    if not directory.is_dir():
        raise MetricsError("no_such_dir", f"not a directory: {directory}")
    pages = [p for p in directory.iterdir() if p.is_file() and p.suffix.lower() in _RASTER_EXTS]
    return sorted(pages, key=lambda p: _natural_key(p.name))


def _load_gray(path: Path) -> tuple[np.ndarray, bool]:
    try:
        with Image.open(path) as img:
            converted = img.mode != "L"
            if converted:
                img = img.convert("L")
            return np.asarray(img, dtype=np.uint8), converted
    except OSError as exc:
        raise MetricsError("unreadable_raster", f"{path}: {exc}") from exc


def _size_of(path: Path) -> tuple[int, int]:
    try:
        with Image.open(path) as img:
            return int(img.width), int(img.height)
    except OSError as exc:
        raise MetricsError("unreadable_raster", f"{path}: {exc}") from exc


def ssim_pages(a_dir, b_dir, dpi: int = 150) -> dict:
    """Per-page SSIM of ``a_dir`` vs ``b_dir`` aligned by page index (C9 §4).

    Returns ``{"dpi", "aligned", "min", "mean", "pages": [{page, ssim,
    width, height, flags}, ...]}``. ``min``/``mean`` are None when no page
    pair is aligned.
    """
    a_pages = _list_pages(Path(a_dir))
    b_pages = _list_pages(Path(b_dir))
    total = max(len(a_pages), len(b_pages))

    entries: list[dict] = []
    scores: list[float] = []
    for i in range(total):
        page = i + 1
        fa = a_pages[i] if i < len(a_pages) else None
        fb = b_pages[i] if i < len(b_pages) else None

        if fa is None or fb is None:
            width, height = _size_of(fa if fa is not None else fb)
            entries.append({"page": page, "ssim": 0.0, "width": width,
                            "height": height, "flags": ["missing"]})
            continue

        img_a, conv_a = _load_gray(fa)
        img_b, conv_b = _load_gray(fb)
        flags: list[str] = []
        if conv_a or conv_b:
            flags.append("converted_to_grayscale")
        if img_a.shape != img_b.shape:
            flags.append("size_mismatch")
            h = min(img_a.shape[0], img_b.shape[0])
            w = min(img_a.shape[1], img_b.shape[1])
            img_a = img_a[:h, :w]
            img_b = img_b[:h, :w]
        h, w = img_a.shape
        if h < _WIN_SIZE or w < _WIN_SIZE:
            raise MetricsError(
                "image_too_small",
                f"page {page}: common size {w}x{h} below the 11x11 SSIM window",
            )
        score = float(_structural_similarity(
            img_a, img_b,
            win_size=_WIN_SIZE,
            sigma=_SIGMA,
            data_range=_DATA_RANGE,
            gaussian_weights=True,
            use_sample_covariance=False,
        ))
        scores.append(score)
        entries.append({"page": page, "ssim": score, "width": int(w),
                        "height": int(h), "flags": flags})

    return {
        "dpi": int(dpi),
        "aligned": len(scores),
        "min": min(scores) if scores else None,
        "mean": (sum(scores) / len(scores)) if scores else None,
        "pages": entries,
    }
