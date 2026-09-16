//! adapters-pdf: PDF reader for stichos.
//!
//! Parses PDF object streams, the page tree, annotations, AcroForm fields,
//! and extracts text with bounding boxes.
//!
//! Modules:
//! - `parser`: core PDF parser (objects, xref, page tree, streams)
//! - `annot`: annotation extraction from page /Annots arrays
//! - `outline`: outline/tree extraction from /Outlines
//! - `metadata`: XMP metadata parsing from /Metadata stream
//! - `form`: AcroForm (interactive form fields) and XFA detection
//! - `text`: text extraction with word-level bounding boxes
//! - `qpdf`: external `qpdf --check` integration for PDF validation
//! - `incremental`: append-only incremental PDF writer (P3-12)
//! - `patch_executor`: PDF-aware patch executor implementing PatchExecutor trait (P3-14)

pub mod annot;
pub mod form;
pub mod incremental;
pub mod metadata;
pub mod outline;
pub mod parser;
pub mod patch_executor;
pub mod qpdf;
pub mod text;
