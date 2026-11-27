# SPDX-License-Identifier: MIT OR Apache-2.0
"""test_cross_check -- pytest tests for pyoracle cross-check modules."""

import json
import os
import pathlib
import sys
import unittest.mock

import pytest

# Ensure harness/pyoracle is on the path
PYORACLE_DIR = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(PYORACLE_DIR))

import corpus_reader
import cross_check
import stichos_reader

# Corpus file paths
CORPUS_DIR = PYORACLE_DIR.parent.parent / "corpus" / "blobs"

# Sample files for testing
DOCX_SAMPLE = CORPUS_DIR / "cjk-text-docx-001.docx"
XLSX_SAMPLE = CORPUS_DIR / "cjk-text-xlsx-001.xlsx"
PPTX_SAMPLE = CORPUS_DIR / "cjk-text-pptx-001.pptx"

# Larger samples for integration tests
DOCX_TABLE_SAMPLE = CORPUS_DIR / "long-tables-docx-001.docx"
XLSX_TABLE_SAMPLE = CORPUS_DIR / "long-tables-xlsx-001.xlsx"


# ------------------------------------------------------------------ corpus_reader


class TestCorpusReaderDetect:
    """Test file format detection."""

    def test_detect_docx(self):
        result = corpus_reader.inspect_corpus_file(str(DOCX_SAMPLE))
        assert result["format"] == "docx"
        assert result["document_type"] == "document"

    def test_detect_xlsx(self):
        result = corpus_reader.inspect_corpus_file(str(XLSX_SAMPLE))
        assert result["format"] == "xlsx"
        assert result["document_type"] == "spreadsheet"

    def test_detect_pptx(self):
        result = corpus_reader.inspect_corpus_file(str(PPTX_SAMPLE))
        assert result["format"] == "pptx"
        assert result["document_type"] == "presentation"

    def test_detect_unsupported(self):
        result = corpus_reader.inspect_corpus_file("test.txt")
        assert result["format"] is None
        assert "Unsupported" in result.get("error", "")


class TestCorpusReaderDocx:
    """Test DOCX-specific extraction."""

    @pytest.fixture(autouse=True)
    def _load(self):
        self.result = corpus_reader.inspect_corpus_file(str(DOCX_SAMPLE))

    def test_has_text(self):
        assert "text" in self.result
        assert isinstance(self.result["text"], list)
        assert len(self.result["text"]) > 0

    def test_has_styles(self):
        assert "styles" in self.result
        assert isinstance(self.result["styles"], list)
        style_names = [s["name"] for s in self.result["styles"]]
        assert "Normal" in style_names

    def test_style_has_fields(self):
        if self.result["styles"]:
            s = self.result["styles"][0]
            assert "name" in s
            assert "type" in s

    def test_has_images(self):
        assert "images" in self.result
        assert isinstance(self.result["images"], dict)
        assert "count" in self.result["images"]

    def test_has_fields(self):
        assert "fields" in self.result
        assert isinstance(self.result["fields"], list)

    def test_has_page_setup(self):
        assert "page_setup" in self.result
        ps = self.result["page_setup"]
        assert "orientation" in ps


class TestCorpusReaderXlsx:
    """Test XLSX-specific extraction."""

    @pytest.fixture(autouse=True)
    def _load(self):
        self.result = corpus_reader.inspect_corpus_file(str(XLSX_SAMPLE))

    def test_has_sheets(self):
        assert "sheets" in self.result
        assert isinstance(self.result["sheets"], dict)
        assert len(self.result["sheets"]) > 0

    def test_sheets_have_cell_values(self):
        for sheet_name, sheet_data in self.result["sheets"].items():
            assert "cell_values" in sheet_data
            assert isinstance(sheet_data["cell_values"], list)

    def test_cell_has_coordinates(self):
        for sheet_data in self.result["sheets"].values():
            for cell in sheet_data["cell_values"]:
                assert "coordinate" in cell
                assert "value" in cell
                assert "cell_type" in cell

    def test_has_number_formats(self):
        for sheet_data in self.result["sheets"].values():
            assert "number_formats" in sheet_data

    def test_has_freeze_panes(self):
        for sheet_data in self.result["sheets"].values():
            assert "freeze_panes" in sheet_data

    def test_has_sheet_views(self):
        for sheet_data in self.result["sheets"].values():
            assert "sheet_views" in sheet_data

    def test_has_autofilter(self):
        for sheet_data in self.result["sheets"].values():
            assert "autofilter" in sheet_data

    def test_has_conditional_formats(self):
        for sheet_data in self.result["sheets"].values():
            assert "conditional_formats" in sheet_data

    def test_has_data_validations(self):
        for sheet_data in self.result["sheets"].values():
            assert "data_validations" in sheet_data

    def test_has_tables(self):
        for sheet_data in self.result["sheets"].values():
            assert "tables" in sheet_data

    def test_has_protection(self):
        for sheet_data in self.result["sheets"].values():
            assert "protection" in sheet_data

    def test_has_page_setup(self):
        for sheet_data in self.result["sheets"].values():
            assert "page_setup" in sheet_data

    def test_has_hyperlinks(self):
        for sheet_data in self.result["sheets"].values():
            assert "hyperlinks" in sheet_data

    def test_defined_names(self):
        assert "defined_names" in self.result

    def test_sheet_count(self):
        assert self.result["sheet_count"] == len(self.result["sheets"])


class TestCorpusReaderPptx:
    """Test PPTX-specific extraction."""

    @pytest.fixture(autouse=True)
    def _load(self):
        self.result = corpus_reader.inspect_corpus_file(str(PPTX_SAMPLE))

    def test_has_slides(self):
        assert "slides" in self.result
        assert isinstance(self.result["slides"], list)
        assert len(self.result["slides"]) > 0

    def test_slides_have_shapes(self):
        for slide in self.result["slides"]:
            assert "shapes" in slide
            assert isinstance(slide["shapes"], list)

    def test_shape_has_position(self):
        for slide in self.result["slides"]:
            for shape in slide["shapes"]:
                assert "left" in shape
                assert "top" in shape
                assert "width" in shape
                assert "height" in shape

    def test_shape_has_text(self):
        for slide in self.result["slides"]:
            for shape in slide["shapes"]:
                if "text" in shape:
                    assert isinstance(shape["text"], str)

    def test_has_placeholders(self):
        for slide in self.result["slides"]:
            assert "placeholders" in slide

    def test_has_notes(self):
        for slide in self.result["slides"]:
            assert "notes" in slide

    def test_has_charts(self):
        for slide in self.result["slides"]:
            assert "charts" in slide

    def test_has_tables(self):
        for slide in self.result["slides"]:
            assert "tables" in slide

    def test_slide_count(self):
        assert self.result["slide_count"] == len(self.result["slides"])

    def test_has_slide_size(self):
        assert "slide_size" in self.result
        ss = self.result["slide_size"]
        assert "width" in ss
        assert "height" in ss


class TestCorpusReaderListFiles:
    """Test list_corpus_files."""

    def test_list_corpus_files(self):
        files = corpus_reader.list_corpus_files(CORPUS_DIR)
        assert len(files) > 0
        # All files should be supported extensions
        for f in files:
            assert pathlib.Path(f).suffix.lower() in corpus_reader._SUPPORTED

    def test_list_filtered(self):
        files = corpus_reader.list_corpus_files(CORPUS_DIR, {".docx"})
        assert all(f.endswith(".docx") for f in files)

    def test_list_nonexistent(self):
        files = corpus_reader.list_corpus_files("/tmp/nonexistent_dir_xyz")
        assert files == []


# ----------------------------------------------------------------- stichos_reader


class TestStichosReader:
    """Test stichos binary detection and output."""

    def test_find_stichos_with_env(self):
        """When STICHOS_BIN env var points to a real binary."""
        bin_path = str(PYORACLE_DIR.parent.parent / "target" / "release" / "stichos")
        if os.path.exists(bin_path):
            output = stichos_reader.get_stichos_output(str(DOCX_SAMPLE), json_path=bin_path)
            assert "error" not in output or output.get("tool") == "inspect_document"
        else:
            pytest.skip("stichos binary not at target/release/stichos")

    def test_find_stichos_fallback(self, monkeypatch):
        """When no stichos binary can be located, get_stichos_output reports stichos_absent."""
        # Deterministically simulate absence regardless of a built binary on disk.
        monkeypatch.setattr(stichos_reader, "_find_stichos", lambda: None)
        output = stichos_reader.get_stichos_output(str(DOCX_SAMPLE))
        assert output.get("stichos_absent") is True
        assert "not found" in output.get("error", "")


class TestStichosReaderIntegration:
    """Integration tests with real stichos binary."""

    @pytest.fixture
    def stichos_bin(self):
        bin_path = str(PYORACLE_DIR.parent.parent / "target" / "release" / "stichos")
        if not os.path.exists(bin_path):
            pytest.skip("stichos binary not found")
        return bin_path

    def test_docx_output(self, stichos_bin):
        output = stichos_reader.get_stichos_output(str(DOCX_SAMPLE), json_path=stichos_bin)
        assert "tool" in output
        assert output.get("tool") == "inspect_document"
        assert output.get("document_type") == "document"

    def test_xlsx_output(self, stichos_bin):
        output = stichos_reader.get_stichos_output(str(XLSX_SAMPLE), json_path=stichos_bin)
        assert "tool" in output
        assert output.get("document_type") == "spreadsheet"

    def test_pptx_output(self, stichos_bin):
        output = stichos_reader.get_stichos_output(str(PPTX_SAMPLE), json_path=stichos_bin)
        assert "tool" in output
        assert output.get("document_type") == "presentation"

    def test_bad_path(self, stichos_bin):
        output = stichos_reader.get_stichos_output("/nonexistent/file.docx", json_path=stichos_bin)
        assert "error" in output
        assert output.get("error") is not None


# ----------------------------------------------------------- cross_check


class TestCrossCheckCompare:
    """Test the compare_file function."""

    def test_compare_docx(self):
        result = cross_check.compare_file(str(DOCX_SAMPLE))
        assert result.fmt == "docx"
        assert result.path == str(DOCX_SAMPLE)

    def test_compare_xlsx(self):
        result = cross_check.compare_file(str(XLSX_SAMPLE))
        assert result.fmt == "xlsx"

    def test_compare_pptx(self):
        result = cross_check.compare_file(str(PPTX_SAMPLE))
        assert result.fmt == "pptx"

    def test_compare_unknown_format(self):
        result = cross_check.compare_file(str(PYORACLE_DIR / "nonexistent.txt"))
        assert result.fmt == "unknown"


class TestCrossCheckSummary:
    """Test the print_summary function."""

    def test_empty_results(self):
        rc = cross_check.print_summary([])
        assert rc == 0

    def test_single_pass(self):
        r = cross_check.Result("test.docx", "docx")
        r.matches.append("test match")
        rc = cross_check.print_summary([r])
        assert rc == 0

    def test_single_fail(self):
        r = cross_check.Result("test.docx", "docx")
        r.mismatches.append("test mismatch")
        rc = cross_check.print_summary([r])
        assert rc == 1

    def test_single_error(self):
        r = cross_check.Result("test.docx", "docx")
        r.errors.append("test error")
        rc = cross_check.print_summary([r])
        assert rc == 1

    def test_mixed_results(self):
        r1 = cross_check.Result("a.docx", "docx")
        r1.matches.append("ok")
        r2 = cross_check.Result("b.xlsx", "xlsx")
        r2.mismatches.append("fail")
        rc = cross_check.print_summary([r1, r2])
        assert rc == 1


# ------------------------------------------------------ integration


@pytest.mark.integration
class TestIntegrationFullCorpus:
    """Full cross-check against a subset of corpus files."""

    @pytest.fixture
    def stichos_bin(self):
        bin_path = str(PYORACLE_DIR.parent.parent / "target" / "release" / "stichos")
        if not os.path.exists(bin_path):
            pytest.skip("stichos binary not found")
        return bin_path

    def test_docx_table_integration(self, stichos_bin):
        """Cross-check long-tables-docx against stichos."""
        # Verify corpus reader
        py_data = corpus_reader.inspect_corpus_file(str(DOCX_TABLE_SAMPLE))
        assert py_data["format"] == "docx"
        assert len(py_data.get("text", [])) > 0

        # Verify stichos reader
        st_data = stichos_reader.get_stichos_output(
            str(DOCX_TABLE_SAMPLE), json_path=stichos_bin
        )
        assert "error" not in st_data or st_data.get("tool") == "inspect_document"

        # Verify comparison works without error
        result = cross_check.compare_file(str(DOCX_TABLE_SAMPLE))
        assert result.fmt == "docx"
        # Should have at least some matches
        assert len(result.matches) >= 1

    def test_xlsx_table_integration(self, stichos_bin):
        """Cross-check long-tables-xlsx against stichos."""
        py_data = corpus_reader.inspect_corpus_file(str(XLSX_TABLE_SAMPLE))
        assert py_data["format"] == "xlsx"
        sheets = py_data.get("sheets", {})
        assert len(sheets) > 0

        st_data = stichos_reader.get_stichos_output(
            str(XLSX_TABLE_SAMPLE), json_path=stichos_bin
        )
        assert "error" not in st_data or st_data.get("tool") == "inspect_document"

        result = cross_check.compare_file(str(XLSX_TABLE_SAMPLE))
        assert result.fmt == "xlsx"
        assert len(result.matches) >= 1


class TestCrossCheckModule:
    """Test the cross_check module-level functions."""

    def test_safe_text_string(self):
        assert cross_check._safe_text("  hello  ") == "hello"

    def test_safe_text_list(self):
        assert cross_check._safe_text(["hello", "world"]) == "hello\nworld"

    def test_safe_text_empty(self):
        assert cross_check._safe_text([]) == ""

    def test_safe_text_none(self):
        assert cross_check._safe_text(None) == "None"

    def test_result_class(self):
        r = cross_check.Result("test.docx", "docx")
        assert r.path == "test.docx"
        assert r.fmt == "docx"
        assert r.passed is True
        r.mismatches.append("fail")
        assert r.passed is False

    def test_result_with_error(self):
        r = cross_check.Result("test.docx", "docx")
        r.errors.append("error")
        assert r.passed is False
