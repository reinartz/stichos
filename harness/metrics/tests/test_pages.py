import pytest

from metrics import MetricsError, page_count_delta


def test_equal():
    r = page_count_delta(3, 3)
    assert r["page_delta"] == 0
    assert r["abs_page_delta"] == 0
    assert r["pass"] is True


def test_more_candidate_pages():
    r = page_count_delta(5, 3)
    assert r["page_delta"] == 2
    assert r["abs_page_delta"] == 2
    assert r["pass"] is False


def test_fewer_candidate_pages_signed():
    r = page_count_delta(2, 3)
    assert r["page_delta"] == -1
    assert r["abs_page_delta"] == 1
    assert r["pass"] is False


def test_negative_rejected():
    with pytest.raises(MetricsError) as exc:
        page_count_delta(-1, 3)
    assert exc.value.code == "bad_page_count"


def test_non_integer_rejected():
    with pytest.raises(MetricsError):
        page_count_delta(1.5, 3)
