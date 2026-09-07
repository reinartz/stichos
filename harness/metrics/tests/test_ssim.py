import numpy as np

from helpers import blank_with_text, smooth_gray, write_gray_png

from metrics import ssim_pages


def _dirs(tmp_path, a_images, b_images):
    for i, arr in enumerate(a_images, 1):
        write_gray_png(tmp_path / "a", f"page-{i:02d}.png", arr)
    for i, arr in enumerate(b_images, 1):
        write_gray_png(tmp_path / "b", f"page-{i:02d}.png", arr)
    return tmp_path / "a", tmp_path / "b"


def test_identical_images_score_one(tmp_path):
    img = smooth_gray()
    a, b = _dirs(tmp_path, [img, img], [img, img])
    r = ssim_pages(a, b)
    assert r["aligned"] == 2
    assert all(p["ssim"] == 1.0 for p in r["pages"])
    assert r["mean"] == 1.0
    assert r["min"] == 1.0
    assert all(p["flags"] == [] for p in r["pages"])
    assert r["dpi"] == 150


def test_three_pixel_shift_high_but_below_one(tmp_path):
    img = smooth_gray()
    shifted = np.zeros_like(img)
    shifted[:, 3:] = img[:, :-3]
    a, b = _dirs(tmp_path, [img], [shifted])
    r = ssim_pages(a, b)
    s = r["pages"][0]["ssim"]
    assert 0.9 < s < 1.0


def test_blank_vs_text_is_low(tmp_path):
    blank = np.full((128, 128), 255, dtype=np.uint8)
    text = blank_with_text()
    a, b = _dirs(tmp_path, [blank], [text])
    r = ssim_pages(a, b)
    assert r["pages"][0]["ssim"] < 0.5


def test_size_mismatch_trimmed_and_flagged(tmp_path):
    img = smooth_gray()
    crop = img[:, :100]
    a, b = _dirs(tmp_path, [img], [crop])
    r = ssim_pages(a, b)
    page = r["pages"][0]
    assert "size_mismatch" in page["flags"]
    assert page["ssim"] == 1.0
    assert page["width"] == 100 and page["height"] == 128


def test_missing_page_zero_and_flag(tmp_path):
    img = smooth_gray()
    other = smooth_gray(seed=11)
    a, b = _dirs(tmp_path, [img, other], [img])
    r = ssim_pages(a, b)
    assert len(r["pages"]) == 2
    assert r["aligned"] == 1
    assert r["pages"][1]["ssim"] == 0.0
    assert r["pages"][1]["flags"] == ["missing"]
    assert r["mean"] == r["pages"][0]["ssim"] == 1.0
    assert r["min"] == 1.0


def test_empty_directories(tmp_path):
    (tmp_path / "a").mkdir()
    (tmp_path / "b").mkdir()
    r = ssim_pages(tmp_path / "a", tmp_path / "b")
    assert r["aligned"] == 0
    assert r["pages"] == []
    assert r["mean"] is None and r["min"] is None
