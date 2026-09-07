"""Fixture builders shared by the metrics tests."""
from __future__ import annotations

import struct
import zipfile
from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter

W_NS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
ODF_NS = ('xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" '
          'xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" '
          'xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" '
          'xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"')

DOC_XML = f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="{W_NS}"><w:body>
<w:tbl><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>
<w:tbl/>
<w:p><w:r><w:drawing/></w:r></w:p>
<w:p><w:fldSimple w:instr=" PAGE "/></w:p>
<w:p>
 <w:r><w:fldChar w:fldCharType="begin"/></w:r>
 <w:r><w:instrText xml:space="preserve"> NUMPAGES </w:instrText></w:r>
 <w:r><w:fldChar w:fldCharType="separate"/></w:r>
 <w:r><w:t>1</w:t></w:r>
 <w:r><w:fldChar w:fldCharType="end"/></w:r>
</w:p>
<w:p><w:r><w:commentReference w:id="1"/></w:r></w:p>
</w:body></w:document>'''

COMMENTS_XML = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
                f'<w:comments xmlns:w="{W_NS}">'
                f'<w:comment w:id="1" w:author="gp">note</w:comment>'
                f'</w:comments>')

CONTENT_XML = f'''<?xml version="1.0" encoding="UTF-8"?>
<office:document-content {ODF_NS}>
<office:body><office:text>
<table:table/><table:table/>
<text:p><draw:frame/></text:p>
<text:note><text:note-body><text:p>n</text:p></text:note-body></text:note>
<office:annotation><text:p>a</text:p></office:annotation>
<text:p><text:field/></text:p>
</office:text></office:body></office:document-content>'''


def smooth_gray(seed: int = 7, size: int = 128) -> np.ndarray:
    rng = np.random.default_rng(seed)
    small = rng.integers(0, 256, (4, 4), dtype=np.uint8)
    block = np.kron(small, np.ones((size // 4, size // 4), dtype=np.uint8))
    return np.asarray(Image.fromarray(block).filter(ImageFilter.GaussianBlur(8)),
                      dtype=np.uint8)


def blank_with_text() -> np.ndarray:
    arr = np.full((128, 128), 255, dtype=np.uint8)
    for row in range(20, 110, 12):
        arr[row:row + 3, 15:115] = 0
    return arr


def write_gray_png(directory: Path, name: str, arr: np.ndarray) -> Path:
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / name
    Image.fromarray(arr, mode="L").save(path)
    return path


def make_docx(path: Path) -> Path:
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("[Content_Types].xml", '<?xml version="1.0"?><Types/>')
        zf.writestr("word/document.xml", DOC_XML)
        zf.writestr("word/comments.xml", COMMENTS_XML)
    return path


def make_odt(path: Path) -> Path:
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("mimetype", "application/vnd.oasis.opendocument.text")
        zf.writestr("META-INF/manifest.xml", '<?xml version="1.0"?><manifest/>')
        zf.writestr("content.xml", CONTENT_XML)
    return path


def make_pad_docx(path: Path, approx_member_bytes: int) -> Path:
    """docx whose word/document.xml is a VALID XML doc of ~approx_member_bytes.

    Still contains the 2 tables / 1 drawing / ... from DOC_XML semantics is
    not guaranteed here; size is what matters for guard tests.
    """
    head = DOC_XML.replace("</w:body></w:document>", "")
    filler = "<w:p/>" * max(0, (approx_member_bytes - len(head)) // len("<w:p/>"))
    doc = (head + filler + "</w:body></w:document>").encode("utf-8")
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("[Content_Types].xml", '<?xml version="1.0"?><Types/>')
        zf.writestr("word/document.xml", doc)
    return path


def patch_central_declared_size(path: Path, new_size: int) -> None:
    """Manifest lie: rewrite every central-directory uncompressed size field."""
    data = bytearray(path.read_bytes())
    sig = b"PK\x01\x02"
    start = 0
    while (j := data.find(sig, start)) != -1:
        struct.pack_into("<I", data, j + 24, new_size)
        start = j + 4
    path.write_bytes(bytes(data))
