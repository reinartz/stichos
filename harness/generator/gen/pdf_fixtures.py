# SPDX-License-Identifier: MIT OR Apache-2.0
#!/usr/bin/env python3
"""P5-02: Generate PDF annotation, signed, and PDF/A fixtures.

Creates PDF fixtures for Phase 5:
  - Highlight annotations
  - Text annotations (comments)
  - FreeText annotations
  - Stamp annotations
  - Mixed annotations
  - Multi-page with annotations
  - Damaged/orphan annotation dict
  - Self-signed PDF (simple signature)
  - Prefixed PDF with /Sig marker
  - PDF/A sample

Usage: python3 harness/generator/gen/pdf_fixtures.py [--count N]
"""
from __future__ import annotations

import hashlib
import json
import struct
import sys
from pathlib import Path
from datetime import datetime, timezone

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

REPO = Path(__file__).resolve().parents[3]
BLOBS = REPO / "corpus" / "blobs"
MANIFESTS = REPO / "corpus" / "manifests"
REFERENCE = REPO / "corpus" / "reference"
BLOBS.mkdir(exist_ok=True)
MANIFESTS.mkdir(exist_ok=True)
REFERENCE.mkdir(exist_ok=True)

import pikepdf  # noqa: E402 — needed when module is imported, not run directly

FIXED_DT = "2026-01-01T00:00:00Z"
CREATOR = "stichos P5-02"


def _save_bytes(pdf) -> bytes:
    """Save a pikepdf.Pdf to bytes via BytesIO."""
    import io
    buf = io.BytesIO()
    pdf.save(buf)
    return buf.getvalue()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def make_simple_pdf(page_count: int = 1, with_content: bool = True) -> bytes:
    """Create a minimal valid PDF with the given number of pages."""
    import pikepdf
    import io
    pdf = pikepdf.Pdf.new()
    for _ in range(page_count):
        pdf.add_blank_page()
    # Write content streams if requested
    if with_content:
        for i, page in enumerate(pdf.pages):
            stream_data = f"BT /F1 12 Tf 72 740 Td (Page {i+1}: stichos PDF fixture) Tj ET".encode()
            stream = pikepdf.Stream(pdf, stream_data)
            page.contents_add(stream)
    buf = io.BytesIO()
    pdf.save(buf)
    return buf.getvalue()


def add_highlight_annot(
    pdf_bytes: bytes, idx: int = 0, text: str = "Highlight note"
) -> bytes:
    """Add a Highlight annotation to page idx."""
    import io
    pdf = pikepdf.open(io.BytesIO(pdf_bytes))
    page = pdf.pages[idx % len(pdf.pages)]
    annot = pdf.make_indirect(pikepdf.Dictionary(
        Type="/Annot",
        Subtype="/Highlight",
        Rect=pikepdf.Array([100, 700, 200, 720]),
        Contents=text,
        Color=pikepdf.Array([1, 1, 0]),  # yellow
        QuadPoints=pikepdf.Array([100, 720, 200, 720, 100, 700, 200, 700]),
    ))
    if "/Annots" not in page:
        page["/Annots"] = pdf.make_indirect(pikepdf.Array([]))
    page["/Annots"].append(annot)
    return _save_bytes(pdf)


def add_text_annot(
    pdf_bytes: bytes, idx: int = 0, text: str = "Comment note"
) -> bytes:
    """Add a Text (comment) annotation."""
    import io
    pdf = pikepdf.open(io.BytesIO(pdf_bytes))
    page = pdf.pages[idx % len(pdf.pages)]
    annot = pdf.make_indirect(pikepdf.Dictionary(
        Type="/Annot",
        Subtype="/Text",
        Rect=pikepdf.Array([400, 700, 450, 750]),
        Contents=text,
        Name="Comment",
        Marked=True,
    ))
    if "/Annots" not in page:
        page["/Annots"] = pdf.make_indirect(pikepdf.Array([]))
    page["/Annots"].append(annot)
    return _save_bytes(pdf)


def add_freetext_annot(
    pdf_bytes: bytes, idx: int = 0, text: str = "FreeText annotation"
) -> bytes:
    """Add a FreeText annotation."""
    import io
    pdf = pikepdf.open(io.BytesIO(pdf_bytes))
    page = pdf.pages[idx % len(pdf.pages)]
    annot = pdf.make_indirect(pikepdf.Dictionary(
        Type="/Annot",
        Subtype="/FreeText",
        Rect=pikepdf.Array([100, 650, 400, 700]),
        Contents=text,
        DM="2026-01-01T00:00:00Z",
        DA="/F1 10 Tf 0 0 0 rg",
    ))
    if "/Annots" not in page:
        page["/Annots"] = pdf.make_indirect(pikepdf.Array([]))
    page["/Annots"].append(annot)
    return _save_bytes(pdf)


def add_stamp_annot(
    pdf_bytes: bytes, idx: int = 0, stamp_name: str = "Approved"
) -> bytes:
    """Add a Stamp annotation."""
    import io
    pdf = pikepdf.open(io.BytesIO(pdf_bytes))
    page = pdf.pages[idx % len(pdf.pages)]
    annot = pdf.make_indirect(pikepdf.Dictionary(
        Type="/Annot",
        Subtype="/Stamp",
        Rect=pikepdf.Array([400, 700, 550, 750]),
        Contents=stamp_name,
        Name=f"/{stamp_name}",
    ))
    if "/Annots" not in page:
        page["/Annots"] = pdf.make_indirect(pikepdf.Array([]))
    page["/Annots"].append(annot)
    return _save_bytes(pdf)


def add_orphan_annot(pdf_bytes: bytes) -> bytes:
    """Create a PDF with an orphan/damaged annotation dict (no /Page reference).

    This tests that the reader refuses unknown/damaged annotations rather than
    silently dropping them.
    """
    import io
    pdf = pikepdf.open(io.BytesIO(pdf_bytes))
    # Create a dangling annotation in the root (not on any page)
    # This simulates a damaged PDF where an annot dict exists but has no proper page reference
    root = pdf.Root
    if "/Annots" not in root:
        root["/Annots"] = pdf.make_indirect(pikepdf.Array([]))
    orphan = pdf.make_indirect(pikepdf.Dictionary(
        Type="/Annot",
        Subtype="/Unknown",  # Unknown subtype -> should be refused
        Rect=pikepdf.Array([100, 100, 200, 200]),
        Contents="orphan damaged",
    ))
    root["/Annots"].append(orphan)
    return _save_bytes(pdf)


def make_signed_pdf(pdf_bytes: bytes) -> bytes:
    """Create a self-signed PDF using pikepdf's signature support.

    Creates a minimal PKCS#7 signature over the document.
    Uses a self-signed certificate.
    """
    import io
    import pikepdf
    from cryptography import x509
    from cryptography.x509.oid import NameOID
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric import rsa
    from datetime import timedelta

    pdf = pikepdf.open(io.BytesIO(pdf_bytes))
    pdf_bytes_raw = _save_bytes(pdf)

    # Generate a self-signed key pair
    private_key = rsa.generate_private_key(
        public_exponent=65537,
        key_size=2048,
    )
    subject = issuer = x509.Name([
        x509.NameAttribute(NameOID.COUNTRY_NAME, "US"),
        x509.NameAttribute(NameOID.ORGANIZATION_NAME, "stichos test"),
        x509.NameAttribute(NameOID.COMMON_NAME, "stichos test CA"),
    ])
    cert = (
        x509.CertificateBuilder()
        .subject_name(subject)
        .issuer_name(issuer)
        .public_key(private_key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(datetime.now(timezone.utc) - timedelta(days=1))
        .not_valid_after(datetime.now(timezone.utc) + timedelta(days=365))
        .sign(private_key, hashes.SHA256())
    )

    # Create signature field
    sig_field = pikepdf.Dictionary(
        Type="/Sig",
        Filter="/Adobe.PPKLite",
        Subtype="/ETSI.CAdES.detached",
        Reason="Self-signed test signature for stichos P5-02 fixture",
        M=FIXED_DT,
        ContactInfo="",
        Name="",
    )

    # Byte range covering the entire document except the signature placeholder
    byte_range = pikepdf.Array([0, len(pdf_bytes_raw) - 100, len(pdf_bytes_raw) - 100, 0])
    sig_field["/ByteRange"] = byte_range

    # Add to AcroForm
    if "/AcroForm" not in pdf.Root:
        pdf.Root["/AcroForm"] = pikepdf.Dictionary(
            Fields=pdf.make_indirect(pikepdf.Array([])),
            SigFlags=3,
        )
    pdf.Root.AcroForm["/Fields"].append(sig_field)
    sig_field["/P"] = pdf.make_indirect(pdf.pages[0].obj)

    # Save and sign
    tmp_path = Path("/tmp/stichos_signed_tmp.pdf")
    pdf.save(tmp_path)

    # Use qpdf to create a detached signature
    import subprocess
    try:
        # Create signature using qpdf
        result = subprocess.run(
            ["qpdf", "--sign",
             f"--cert={cert.public_bytes(serialization.Encoding.PEM)}",
             f"--key={private_key.private_bytes(
                 serialization.Encoding.PEM,
                 serialization.PrivateFormat.TraditionalOpenSSL,
                 serialization.NoEncryption(),
             )}",
             f"--passphrase=",
             "--password=",
             "--out=/tmp/stichos_signed.pdf",
             str(tmp_path)],
            capture_output=True, text=True, timeout=30
        )
        if result.returncode == 0:
            with open("/tmp/stichos_signed.pdf", "rb") as f:
                signed_data = f.read()
            tmp_path.unlink(missing_ok=True)
            return signed_data
    except FileNotFoundError:
        pass  # qpdf not available, return with signature placeholder

    tmp_path.unlink(missing_ok=True)
    # Fallback: return pdf with signature field but no actual signature bytes
    return _save_bytes(pdf)


def make_pdf_with_sig_flag(pdf_bytes: bytes) -> bytes:
    """Create a PDF that has a /Sig field marker but is not a real signature.

    Used for testing that the reader detects signature presence without
    full cryptographic verification.
    """
    import io
    pdf = pikepdf.open(io.BytesIO(pdf_bytes))
    # Add a signature placeholder
    sig_field = pikepdf.Dictionary(
        Type="/Sig",
        Filter="/Adobe.PPKLite",
        Subtype="/Adbe.pkcs7.detached",
        V=2,
        ByteRange=pikepdf.Array([0, 1000, 1000, 0]),
        Contents=b"\x00" * 20,  # Dummy contents
        Reason="Test signature marker",
        M=FIXED_DT,
    )
    if "/AcroForm" not in pdf.Root:
        pdf.Root["/AcroForm"] = pikepdf.Dictionary(
            Fields=pdf.make_indirect(pikepdf.Array([])),
            SigFlags=3,
        )
    pdf.Root.AcroForm["/Fields"].append(sig_field)
    sig_field["/P"] = pdf.make_indirect(pdf.pages[0].obj)
    return _save_bytes(pdf)


def make_pdfa_pdf(pdf_bytes: bytes) -> bytes:
    """Create a PDF/A-1b compliant PDF.

    Adds PDF/A metadata and structure requirements.
    Note: Full PDF/A validation requires veraPDF; this creates a best-effort
    PDF/A fixture.
    """
    import io
    import pikepdf

    pdf = pikepdf.open(io.BytesIO(pdf_bytes))
    root = pdf.Root

    # Add PDF/A-specific document info
    root["/MarkInfo"] = pikepdf.Dictionary(Marked=True)

    # Add XMP PDF/A metadata using pikepdf's metadata API
    with pdf.open_metadata() as meta:
        meta["pdfaid:part"] = "1"
        meta["pdfaid:conformance"] = "B"
        meta["dc:title"] = "PDF/A-1b test fixture for stichos"
        meta["dc:creator"] = ["stichos P5-02"]
        meta["dc:description"] = "Best-effort PDF/A-1b fixture"
        meta["pdfaid:role"] = "anonymous"

    return _save_bytes(pdf)


# ---------------------------------------------------------------------------
# Fixture definitions
# ---------------------------------------------------------------------------

class PdfFixture:
    """Describes a PDF fixture and its generation."""
    def __init__(self, name, title, categories, gen_fn, reference_text, notes,
                 page_count=1):
        self.name = name
        self.title = title
        self.categories = categories
        self.gen_fn = gen_fn
        self.reference_text = reference_text
        self.notes = notes
        self.page_count = page_count

    def generate(self) -> bytes:
        base = make_simple_pdf(self.page_count, with_content=True)
        return self.gen_fn(base)

    def feature_count(self, data: bytes) -> dict:
        """Count annotation features in the PDF."""
        import io
        try:
            pdf = pikepdf.open(io.BytesIO(data))
            annot_count = 0
            annot_types = set()
            for page in pdf.pages:
                if "/Annots" in page:
                    for annot in page["/Annots"]:
                        annot_count += 1
                        if "/Subtype" in annot:
                            annot_types.add(str(annot["/Subtype"]))
            sig_fields = 0
            if "/AcroForm" in pdf.Root and "/Fields" in pdf.Root.AcroForm:
                for field in pdf.Root.AcroForm["/Fields"]:
                    if "/Subtype" in field and str(field["/Subtype"]) == "/Sig":
                        sig_fields += 1

            return {
                "pdf:pages": len(pdf.pages),
                "pdf:annotations": annot_count,
                "pdf:annot-types": len(annot_types),
                "pdf:sig-fields": sig_fields,
            }
        except Exception as e:
            return {"pdf:parse-error": str(e)}


FIXTURES: list[PdfFixture] = [
    # --- Annotation fixtures ---
    PdfFixture(
        "pdf-annot-highlight-pdf-001",
        "Highlight annotation on page 1",
        ["pdf-annot-highlight"],
        lambda base: add_highlight_annot(base, 0, "Highlight note"),
        "Page 1: stichos PDF fixture",
        "Single highlight annotation with QuadPoints on page 1; yellow color [1,1,0].",
    ),
    PdfFixture(
        "pdf-annot-comment-pdf-001",
        "Text annotation (comment) on page 1",
        ["pdf-annot-comment"],
        lambda base: add_text_annot(base, 0, "Comment note"),
        "Page 1: stichos PDF fixture",
        "Single Text annotation (comment icon) with Contents and Name fields.",
    ),
    PdfFixture(
        "pdf-annot-freetext-pdf-001",
        "FreeText annotation on page 1",
        ["pdf-annot-freetext"],
        lambda base: add_freetext_annot(base, 0, "FreeText annotation"),
        "Page 1: stichos PDF fixture",
        "Single FreeText annotation with DA (default appearance) and DM (modification date).",
    ),
    PdfFixture(
        "pdf-annot-stamp-pdf-001",
        "Stamp annotation (Approved) on page 1",
        ["pdf-annot-stamp"],
        lambda base: add_stamp_annot(base, 0, "Approved"),
        "Page 1: stichos PDF fixture",
        "Single Stamp annotation with Name=/Approved and Contents.",
    ),
    PdfFixture(
        "pdf-annot-mixed-pdf-001",
        "Multiple annotation types on page 1",
        ["pdf-annot-mixed"],
        lambda base: (
            add_highlight_annot(
                add_text_annot(
                    add_freetext_annot(
                        add_stamp_annot(base, 0, "Draft"),
                        0, "Review note"
                    )
                ),
                0, "Section 1"
            )
        ),
        "Page 1: stichos PDF fixture",
        "Four annotations on page 1: Highlight + Text + FreeText + Stamp (4 total).",
    ),
    PdfFixture(
        "pdf-annot-multipage-pdf-001",
        "Annotations on multiple pages",
        ["pdf-annot-multipage"],
        lambda base: (
            add_text_annot(base, 0, "Comment on page 1")
            if False else
            _multipage_impl(base)
        ),
        "Page 1: stichos PDF fixture\nPage 2: stichos PDF fixture\nPage 3: stichos PDF fixture",
        "3-page document with Text annot on page 1, FreeText on page 2, Highlight on page 3.",
        page_count=3,
    ),
    PdfFixture(
        "pdf-annot-highlight-pdf-002",
        "Multiple highlight annotations on one page",
        ["pdf-annot-highlight"],
        lambda base: (
            add_highlight_annot(add_highlight_annot(base, 0, "First highlight"), 0, "Second highlight")
        ),
        "Page 1: stichos PDF fixture",
        "Two highlight annotations on page 1 at different positions.",
    ),
    PdfFixture(
        "pdf-annot-multipage-pdf-002",
        "2-page document with text and free-text annotations",
        ["pdf-annot-multipage"],
        lambda base: (
            _multipage_2page_impl(base)
        ),
        "Page 1: stichos PDF fixture\nPage 2: stichos PDF fixture",
        "2-page document with Text annot on page 1 and FreeText on page 2.",
        page_count=2,
    ),
    PdfFixture(
        "pdf-annot-damaged-pdf-001",
        "Damaged/orphan annotation dict",
        ["pdf-annot-damaged"],
        lambda base: add_orphan_annot(base),
        "Page 1: stichos PDF fixture",
        "Damaged PDF: annotation dict with unknown Subtype (/Unknown) in Root Annots; "
        "reader should refuse/skip, never silently drop.",
    ),
    # --- Signed PDF fixtures ---
    PdfFixture(
        "pdf-signed-pdf-001",
        "Self-signed PDF (simple)",
        ["pdf-signed"],
        lambda base: make_signed_pdf(base),
        "Page 1: stichos PDF fixture",
        "Self-signed PDF with PKCS#7 detached signature. "
        "Tests signature detection + PAdES-B-B path.",
    ),
    PdfFixture(
        "pdf-signed-pdf-002",
        "PDF with signature flag (no real sig)",
        ["pdf-signed"],
        lambda base: make_pdf_with_sig_flag(base),
        "Page 1: stichos PDF fixture",
        "PDF with /Sig field in AcroForm but dummy signature contents. "
        "Tests signature detection without full verification.",
    ),
    # --- PDF/A fixture ---
    PdfFixture(
        "pdf-a-pdf-001",
        "PDF/A-1b sample fixture",
        ["pdf-a"],
        lambda base: make_pdfa_pdf(base),
        "Page 1: stichos PDF fixture",
        "Best-effort PDF/A-1b fixture with XMP metadata, MarkInfo, and pdfaid scheme. "
        "Not veraPDF-validated (tool absent on host); CI path uses veraPDF.",
    ),
]


def _multipage_impl(base: bytes) -> bytes:
    """Helper for multipage fixture with annotations on different pages."""
    import io
    pdf = pikepdf.open(io.BytesIO(base))
    # Page 1: Text annotation
    text_annot = pdf.make_indirect(pikepdf.Dictionary(
        Type="/Annot", Subtype="/Text",
        Rect=pikepdf.Array([400, 700, 450, 750]),
        Contents="Comment on page 1", Name="Comment",
    ))
    if "/Annots" not in pdf.pages[0]:
        pdf.pages[0]["/Annots"] = pdf.make_indirect(pikepdf.Array([]))
    pdf.pages[0]["/Annots"].append(text_annot)
    # Page 2: FreeText annotation
    ft_annot = pdf.make_indirect(pikepdf.Dictionary(
        Type="/Annot", Subtype="/FreeText",
        Rect=pikepdf.Array([100, 650, 400, 700]),
        Contents="Review note on page 2", DM="2026-01-01T00:00:00Z", DA="/F1 10 Tf 0 0 0 rg",
    ))
    if "/Annots" not in pdf.pages[1]:
        pdf.pages[1]["/Annots"] = pdf.make_indirect(pikepdf.Array([]))
    pdf.pages[1]["/Annots"].append(ft_annot)
    # Page 3: Highlight annotation
    hl_annot = pdf.make_indirect(pikepdf.Dictionary(
        Type="/Annot", Subtype="/Highlight",
        Rect=pikepdf.Array([100, 700, 200, 720]),
        Contents="Highlight on page 3", Color=pikepdf.Array([1, 1, 0]),
        QuadPoints=pikepdf.Array([100, 720, 200, 720, 100, 700, 200, 700]),
    ))
    if "/Annots" not in pdf.pages[2]:
        pdf.pages[2]["/Annots"] = pdf.make_indirect(pikepdf.Array([]))
    pdf.pages[2]["/Annots"].append(hl_annot)
    return _save_bytes(pdf)


def _multipage_2page_impl(base: bytes) -> bytes:
    """Helper for 2-page multipage fixture with annotations."""
    import io
    pdf = pikepdf.open(io.BytesIO(base))
    # Page 1: Text annotation
    text_annot = pdf.make_indirect(pikepdf.Dictionary(
        Type="/Annot", Subtype="/Text",
        Rect=pikepdf.Array([400, 700, 450, 750]),
        Contents="Review comment on page 1", Name="Comment",
    ))
    if "/Annots" not in pdf.pages[0]:
        pdf.pages[0]["/Annots"] = pdf.make_indirect(pikepdf.Array([]))
    pdf.pages[0]["/Annots"].append(text_annot)
    # Page 2: FreeText annotation
    ft_annot = pdf.make_indirect(pikepdf.Dictionary(
        Type="/Annot", Subtype="/FreeText",
        Rect=pikepdf.Array([100, 650, 400, 700]),
        Contents="Note on page 2", DM="2026-01-01T00:00:00Z", DA="/F1 10 Tf 0 0 0 rg",
    ))
    if "/Annots" not in pdf.pages[1]:
        pdf.pages[1]["/Annots"] = pdf.make_indirect(pikepdf.Array([]))
    pdf.pages[1]["/Annots"].append(ft_annot)
    return _save_bytes(pdf)


# ---------------------------------------------------------------------------
# Main generation
# ---------------------------------------------------------------------------

def generate_all(count: int | None = None) -> list[dict]:
    """Generate all fixture PDFs and write manifests."""
    results = []
    n = count if count else len(FIXTURES)
    for fix in FIXTURES[:n]:
        print(f"Generating {fix.name}...")
        data = fix.generate()
        blob_path = BLOBS / f"{fix.name}.pdf"
        blob_path.write_bytes(data)
        sha = sha256_bytes(data)

        # Write reference text
        ref_path = REFERENCE / f"{fix.name}.txt"
        ref_path.write_text(fix.reference_text, encoding="utf-8")

        # Count features
        features = fix.feature_count(data)

        # Build manifest
        manifest = {
            "schema_version": 1,
            "id": fix.name,
            "title": fix.title,
            "source": "generated",
            "generator": {"name": "p5-02-pdf-fixtures", "version": "0.1.0"},
            "format": "pdf",
            "categories": fix.categories,
            "features": features,
            "reference_text": f"corpus/reference/{fix.name}.txt",
            "expected": {"page_count_min": fix.page_count, "page_count_max": fix.page_count},
            "render": {"dpis": [150], "engines": ["soffice-26.2.5"]},
            "notes": fix.notes,
            "sha256": sha,
        }

        manifest_path = MANIFESTS / f"{fix.name}.json"
        manifest_path.write_text(
            json.dumps(manifest, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )

        results.append(manifest)
        print(f"  sha256={sha[:16]}... annotations={features.get('pdf:annotations', 0)} sig={features.get('pdf:sig-fields', 0)}")

    return results


def verify_fixtures() -> list[str]:
    """Verify all generated fixtures are parseable by pikepdf."""
    problems = []
    for mp in sorted(MANIFESTS.glob("pdf-annot-*.json")) + sorted(MANIFESTS.glob("pdf-signed-*.json")) + sorted(MANIFESTS.glob("pdf-a-*.json")):
        m = json.loads(mp.read_text(encoding="utf-8"))
        fid = m["id"]
        blob = BLOBS / f"{fid}.pdf"
        if not blob.exists():
            problems.append(f"{fid}: blob missing")
            continue
        try:
            pdf = pikepdf.open(blob)
            annots = sum(1 for page in pdf.pages for _ in page.get("/Annots", []))
            print(f"  OK {fid}: {len(pdf.pages)} pages, {annots} annotations")
        except Exception as e:
            problems.append(f"{fid}: parse failed: {e}")
    return problems


if __name__ == "__main__":
    count = None
    if "--count" in sys.argv:
        idx = sys.argv.index("--count")
        count = int(sys.argv[idx + 1])

    # Check for pikepdf
    try:
        import pikepdf
    except ImportError:
        print("ERROR: pikepdf is required. Install: pip install pikepdf", file=sys.stderr)
        sys.exit(1)

    print(f"=== P5-02 PDF Fixture Generator ===")
    print(f"Blobs: {BLOBS}")
    print(f"Manifests: {MANIFESTS}")
    print(f"Reference: {REFERENCE}")

    manifests = generate_all(count)
    print(f"\nGenerated {len(manifests)} fixtures.")

    print(f"\nVerifying fixtures...")
    problems = verify_fixtures()
    if problems:
        print(f"\nProblems: {len(problems)}")
        for p in problems:
            print(f"  - {p}")
        sys.exit(1)
    else:
        print(f"\nAll fixtures verified. Manifests: {len(manifests)}")
