# SPDX-License-Identifier: MIT OR Apache-2.0
import pytest

from metrics import text_f1, tokenize


def test_identical_scores_one():
    ref = "Hello, World! the the quick."
    r = text_f1(ref, ref)
    assert r["f1"] == 1.0 and r["precision"] == 1.0 and r["recall"] == 1.0
    assert r["reference_tokens"] == r["extracted_tokens"] == r["common"] == 5


def test_both_empty_scores_one():
    r = text_f1("", "   \u00a0 \n")
    assert r["f1"] == 1.0


def test_empty_extracted_scores_zero():
    r = text_f1("alpha beta gamma", "")
    assert r["f1"] == 0.0
    assert r["precision"] == 0.0
    assert r["recall"] == 0.0
    assert r["common"] == 0


def test_empty_reference_scores_zero():
    assert text_f1("", "alpha")["f1"] == 0.0


def test_two_token_drop_hand_computed():
    # ref 5 tokens, cand 3, common 3 -> P=1.0, R=0.6, F1=2*1*0.6/1.6=0.75
    r = text_f1("alpha beta gamma delta epsilon", "alpha beta gamma")
    assert r["precision"] == 1.0
    assert r["recall"] == pytest.approx(0.6)
    assert r["f1"] == pytest.approx(0.75)
    assert r["common"] == 3


def test_multiset_order_insensitive():
    # bag semantics: common = min-counts = {a:1, b:1} = 2
    r = text_f1("a a b", "a b b")
    assert r["common"] == 2
    assert r["f1"] == pytest.approx(2 / 3)


def test_unicode_whitespace_and_nbsp():
    assert text_f1("a\u00a0b\u2003c\nd", "a b c d")["f1"] == 1.0


def test_case_sensitive():
    assert text_f1("The", "the")["f1"] == 0.0


def test_cjk_per_char_tokenization():
    assert tokenize("中文abc") == ["中", "文", "abc"]
    # punctuation still attaches to the adjacent non-CJK token
    assert tokenize("数据 2024。") == ["数", "据", "2024。"]
    r = text_f1("中文测试", "中文")
    assert r["reference_tokens"] == 4 and r["extracted_tokens"] == 2
    assert r["precision"] == 1.0 and r["recall"] == pytest.approx(0.5)
    assert r["f1"] == pytest.approx(2 / 3)


def test_nfc_normalization():
    assert text_f1("café", "café")["f1"] == 1.0
