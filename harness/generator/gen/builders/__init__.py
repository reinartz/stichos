"""Category builders. Each module registers via gen.core.builder(cat, fmt).

Modules are optional: a missing module only means its (category, format)
builders are absent (`gen status` lists the gaps)."""

for _mod in ("docx", "xlsx", "pptx", "odf"):
    try:
        __import__(f"{__name__}.{_mod}")
    except ImportError as e:  # builder module not written yet / deps missing
        import sys

        print(f"gen: builder module {_mod} not loaded: {e}", file=sys.stderr)
