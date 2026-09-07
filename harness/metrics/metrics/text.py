"""Text-extraction F1 (C9 §7).

Tokenization per C9: NFC-normalize, Unicode whitespace (incl. U+00A0)
boundaries, maximal non-whitespace runs, case-sensitive, punctuation
attaches to the adjacent token; tokens are a multiset. Deviation
(documented in README): C9 defines no CJK rule, so CJK codepoints are
tokenized per character.
"""
from __future__ import annotations

import unicodedata
from collections import Counter

_CJK_RANGES = (
    (0x3040, 0x30FF),    # Hiragana + Katakana
    (0x3400, 0x4DBF),    # CJK Unified Ideographs Extension A
    (0x4E00, 0x9FFF),    # CJK Unified Ideographs
    (0xF900, 0xFAFF),    # CJK Compatibility Ideographs
    (0x20000, 0x2A6DF),  # CJK Extension B
    (0x2A700, 0x2EBEF),  # CJK Extensions C–F
)


def _is_cjk(ch: str) -> bool:
    cp = ord(ch)
    return any(lo <= cp <= hi for lo, hi in _CJK_RANGES)


def tokenize(text: str) -> list[str]:
    """C9 §7 tokenization plus per-character CJK splitting (see README).

    ``str.split()`` maps every Unicode whitespace codepoint (incl. U+00A0,
    U+3000) to a boundary and collapses runs, matching C9 exactly for
    non-CJK text. Within each whitespace-delimited run, CJK codepoints
    become one token each and every other character (letters, digits,
    punctuation) stays grouped — punctuation still attaches to the
    adjacent non-CJK token.
    """
    normalized = unicodedata.normalize("NFC", text)
    tokens: list[str] = []
    for run in normalized.split():
        buf: list[str] = []
        buf_is_cjk: bool | None = None
        for ch in run:
            cjk = _is_cjk(ch)
            if buf_is_cjk is None or cjk == buf_is_cjk:
                buf.append(ch)
                buf_is_cjk = cjk
            else:
                if buf_is_cjk:
                    tokens.extend(buf)
                else:
                    tokens.append("".join(buf))
                buf = [ch]
                buf_is_cjk = cjk
        if buf:
            if buf_is_cjk:
                tokens.extend(buf)
            else:
                tokens.append("".join(buf))
    return tokens


def text_f1(reference_text: str, extracted_text: str) -> dict:
    """Bag-of-tokens F1 per C9 §7.

    Returns ``{"f1", "precision", "recall", "reference_tokens",
    "extracted_tokens", "common"}``. F1 = 1.0 iff both sides empty,
    0.0 iff exactly one side empty; precision/recall are 0.0 on the
    empty side.
    """
    ref = Counter(tokenize(reference_text))
    cand = Counter(tokenize(extracted_text))
    n_ref = sum(ref.values())
    n_cand = sum(cand.values())
    common = sum((ref & cand).values())

    if n_ref == 0 and n_cand == 0:
        precision = recall = f1 = 1.0
    else:
        precision = common / n_cand if n_cand else 0.0
        recall = common / n_ref if n_ref else 0.0
        f1 = (2 * precision * recall / (precision + recall)
              if (precision + recall) else 0.0)

    return {
        "f1": f1,
        "precision": precision,
        "recall": recall,
        "reference_tokens": n_ref,
        "extracted_tokens": n_cand,
        "common": common,
    }
