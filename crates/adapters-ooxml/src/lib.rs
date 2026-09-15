//! adapters-ooxml: OOXML (.docx, .xlsx, .pptx) readers for stichos.
//!
//! Implements OPC (Open Packaging Conventions) parsing, XML content extraction,
//! and conversion to the gp-core intermediate representation.

pub mod census;
pub mod commit;
pub mod docx;
pub mod docx_writer;
pub mod executor;
pub mod opc;
pub mod pptx;
pub mod pptx_writer;
pub mod revision_writer;
pub mod span_mut;
pub mod validate;
pub mod writer;
pub mod xlsx;
pub mod xlsx_writer;
pub mod xml;
