# pyoracle -- Dev-only IR cross-check suite

Cross-check scripts that compare stichos IR output against values extracted by
the corresponding Python OOXML libraries:

- **python-docx** for `.docx` (WordprocessingML)
- **openpyxl** for `.xlsx` (SpreadsheetML)
- **python-pptx** for `.pptx` (PresentationML)

## What it does

For each corpus file in `corpus/blobs/`, pyoracle:

1. Runs `stichos inspect --json --styles --media` and parses the IR JSON.
2. Reads the same file with the appropriate Python library.
3. Compares common fields: text content, cell values, shape positions, style
   names, table data, and other structural elements.
4. Reports matches, mismatches, and stichos-only / python-only findings.

## Usage

```bash
# Check a single file
python3 harness/pyoracle/cross_check.py corpus/blobs/cjk-text-docx-001.docx

# Check an entire directory of corpus files
python3 harness/pyoracle/cross_check.py corpus/blobs/

# Run pytest tests
python3 -m pytest harness/pyoracle/test_cross_check.py -v
```

## Design notes

- **Dev-only**: this tool has no role in the stichos product.  The product is
  a Rust binary with zero hard external dependencies.
- **Graceful fallback**: if the `stichos` binary is not found at
  `target/release/stichos`, the cross-check still runs the Python-reader side
  and reports the stichos result as `absent`.
- **Tolerance**: small differences in floating-point coordinates and internal
  ID values are expected and reported as mismatches without causing a non-zero
  exit.
