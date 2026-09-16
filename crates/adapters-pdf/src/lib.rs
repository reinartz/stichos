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

pub mod annot;
pub mod form;
pub mod incremental;
pub mod metadata;
pub mod outline;
pub mod parser;
pub mod qpdf;
pub mod text;
