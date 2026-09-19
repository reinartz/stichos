# SPDX-License-Identifier: MIT OR Apache-2.0
"""Category builders. Each module registers via gen.core.builder(cat, fmt).

Modules are optional: a missing module only means its (category, format)
builders are absent (`gen status` lists the gaps)."""

for _mod in ("docx", "xlsx", "pptx", "odf"):
    try:
        __import__(f"{__name__}.{_mod}")
    except ImportError as e:  # builder module not written yet / deps missing
        import sys

        print(f"gen: builder module {_mod} not loaded: {e}", file=sys.stderr)

# Load the revision builder (P4-07 lossless proof suite fixtures)
try:
    __import__(f"{__name__}.revision_builder")
except ImportError as e:
    import sys
    print(f"gen: builder module revision_builder not loaded: {e}", file=sys.stderr)
