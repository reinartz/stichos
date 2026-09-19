# SPDX-License-Identifier: MIT OR Apache-2.0
import zipfile

import pytest

import metrics.features as F
from helpers import (make_docx, make_odt, make_pad_docx,
                     patch_central_declared_size)
from metrics import MetricsError, feature_counts, feature_retention


def test_docx_exact_counts(tmp_path):
    path = make_docx(tmp_path / "f.docx")
    counts = feature_counts(path)
    with zipfile.ZipFile(path) as zf:
        files = [i for i in zf.infolist() if not i.is_dir()]
    assert counts == {
        "w:tbl": 2,
        "w:drawing": 1,
        "w:fldSimple": 1,
        "w:commentReference": 1,
        "w:comment": 1,
        "w:instrText": 1,
        "format": "ooxml",
        "part_count": 3,
        "uncompressed_bytes": sum(i.file_size for i in files),
    }


def test_odt_exact_counts(tmp_path):
    path = make_odt(tmp_path / "f.odt")
    counts = feature_counts(path)
    assert counts["table:table"] == 2
    assert counts["draw:frame"] == 1
    assert counts["text:note"] == 1
    assert counts["office:annotation"] == 1
    assert counts["text:field"] == 1
    assert counts["format"] == "odf"
    assert counts["part_count"] == 3
    assert "w:tbl" not in counts


def test_zip_bomb_manifest_lie(tmp_path):
    """Tiny zip whose central directory lies about a 3 GB member size."""
    path = make_docx(tmp_path / "lie.docx")
    patch_central_declared_size(path, 3_000_000_000)
    assert path.stat().st_size < 100_000
    with pytest.raises(MetricsError) as exc:
        feature_counts(path)
    assert exc.value.code == "zip_bomb"


def test_zip_bomb_huge_declared_hits_cap(tmp_path, monkeypatch):
    """Tiny compressed file, honest but > cap declared total (cap lowered)."""
    monkeypatch.setattr(F, "MAX_UNCOMPRESSED_BYTES", 2 * 1024 * 1024)
    path = make_pad_docx(tmp_path / "bomb.docx", 4 * 1024 * 1024)
    assert path.stat().st_size < 1024 * 1024
    with pytest.raises(MetricsError) as exc:
        feature_counts(path)
    assert exc.value.code == "zip_bomb"


def test_compressed_cap(tmp_path, monkeypatch):
    monkeypatch.setattr(F, "MAX_COMPRESSED_BYTES", 100)
    path = make_docx(tmp_path / "f.docx")
    with pytest.raises(MetricsError) as exc:
        feature_counts(path)
    assert exc.value.code == "compressed_too_large"


def test_unsupported_format(tmp_path):
    p = tmp_path / "x.txt"
    p.write_text("hello")
    with pytest.raises(MetricsError) as exc:
        feature_counts(p)
    assert exc.value.code == "unsupported_format"


def test_not_a_zip(tmp_path):
    p = tmp_path / "x.docx"
    p.write_bytes(b"not a zip at all")
    with pytest.raises(MetricsError) as exc:
        feature_counts(p)
    assert exc.value.code == "not_a_zip"


def test_file_not_found(tmp_path):
    with pytest.raises(MetricsError) as exc:
        feature_counts(tmp_path / "missing.docx")
    assert exc.value.code == "file_not_found"


def test_retention():
    before = {"w:tbl": 3, "w:drawing": 2, "w:comment": 1, "part_count": 9,
              "uncompressed_bytes": 1234}
    after = {"w:tbl": 2, "w:drawing": 2, "w:comment": 0, "part_count": 7,
             "uncompressed_bytes": 1000}
    r = feature_retention(before, after)
    assert r["features"]["w:tbl"] == {"before": 3, "after": 2, "dropped": 1,
                                      "changed": True}
    assert r["features"]["w:drawing"] == {"before": 2, "after": 2, "dropped": 0,
                                          "changed": False}
    assert r["features"]["w:comment"] == {"before": 1, "after": 0, "dropped": 1,
                                          "changed": True}
    assert "part_count" not in r["features"]
    assert sorted(r["changed"]) == ["w:comment", "w:tbl"]
    assert r["pass"] is False


def test_retention_clean_and_new_key():
    r = feature_retention({"w:tbl": 1}, {"w:tbl": 1, "w:drawing": 4})
    assert r["features"]["w:drawing"] == {"before": 0, "after": 4, "dropped": 0,
                                          "changed": True}
    assert r["pass"] is False
    r2 = feature_retention({"w:tbl": 1}, {"w:tbl": 1})
    assert r2["pass"] is True and r2["changed"] == []
