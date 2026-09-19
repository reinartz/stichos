# SPDX-License-Identifier: MIT OR Apache-2.0
import json
import subprocess
import sys
from pathlib import Path

import pytest

from helpers import make_docx, smooth_gray, write_gray_png

PKG_DIR = Path(__file__).resolve().parents[1]


def run(*args: str, stdin: str | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, "-m", "metrics", *args],
        capture_output=True, text=True, cwd=str(PKG_DIR), input=stdin,
    )


def test_page_delta_ok():
    r = run("page-delta", "5", "3")
    assert r.returncode == 0
    out = json.loads(r.stdout)
    assert out["ok"] is True and out["page_delta"] == 2


def test_help_works_for_every_verb():
    for verb in ("ssim", "feature-counts", "retention", "text-f1", "page-delta"):
        r = run(verb, "--help")
        assert r.returncode == 0, verb
        assert "usage" in r.stdout


def test_ssim_cli(tmp_path):
    img = smooth_gray()
    write_gray_png(tmp_path / "a", "page-01.png", img)
    write_gray_png(tmp_path / "b", "page-01.png", img)
    r = run("ssim", str(tmp_path / "a"), str(tmp_path / "b"), "--dpi", "150")
    assert r.returncode == 0
    out = json.loads(r.stdout)
    assert out["ok"] is True and out["mean"] == 1.0 and out["dpi"] == 150


def test_feature_counts_cli(tmp_path):
    path = make_docx(tmp_path / "f.docx")
    r = run("feature-counts", str(path))
    out = json.loads(r.stdout)
    assert out["ok"] is True and out["w:tbl"] == 2 and out["w:comment"] == 1


def test_retention_cli(tmp_path):
    (tmp_path / "before.json").write_text(json.dumps({"w:tbl": 3}))
    (tmp_path / "after.json").write_text(json.dumps({"w:tbl": 1}))
    r = run("retention", str(tmp_path / "before.json"), str(tmp_path / "after.json"))
    out = json.loads(r.stdout)
    assert out["ok"] is True
    assert out["features"]["w:tbl"]["dropped"] == 2
    assert out["pass"] is False


def test_text_f1_dash_stdin(tmp_path):
    ext = tmp_path / "ext.txt"
    ext.write_text("alpha beta gamma")
    r = run("text-f1", "-", str(ext), stdin="alpha beta gamma delta epsilon")
    out = json.loads(r.stdout)
    assert out["ok"] is True
    assert out["f1"] == pytest.approx(0.75)


def test_error_is_json_and_nonzero_exit(tmp_path):
    r = run("feature-counts", str(tmp_path / "missing.docx"))
    assert r.returncode != 0
    out = json.loads(r.stdout)
    assert out["ok"] is False
    assert out["error"]["code"] == "file_not_found"


def test_usage_error_nonzero_exit():
    r = run("page-delta", "not-an-int", "3")
    assert r.returncode == 2


def test_bad_verb_nonzero_exit():
    r = run("frobnicate")
    assert r.returncode == 2
