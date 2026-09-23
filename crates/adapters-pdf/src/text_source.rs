// SPDX-License-Identifier: MIT OR Apache-2.0
//! PdfTextSource implementation for adapters-pdf.

use gp_core::diagnostics::Diagnostic;
use gp_core::ir::TextWithBbox;
use gp_core::traits::PdfTextSource;

/// The default [`PdfTextSource`] implementation backed by
/// `adapters_pdf::text::extract_text_bytes`.
pub struct AdaptersPdfTextSource;

impl PdfTextSource for AdaptersPdfTextSource {
    fn extract_text(&self, pdf_bytes: &[u8]) -> Result<Vec<TextWithBbox>, Vec<Diagnostic>> {
        super::text::extract_text_bytes(pdf_bytes)
    }
}
