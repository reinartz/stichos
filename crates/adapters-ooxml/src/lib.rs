//! adapters-ooxml: OOXML (.docx, .xlsx, .pptx) readers and writers for stichos.
//!
//! Implements OPC (Open Packaging Conventions) parsing, XML content extraction,
//! and conversion to the gp-core intermediate representation. Also includes
//! docx creation from IR documents.

pub mod census;
pub mod comment_writer;
pub mod commit;
pub mod create;
pub mod create_pptx;
pub mod create_template;
pub mod create_xlsx;
pub mod docx;
pub mod docx_writer;
pub mod executor;
pub mod md_ir;
pub mod opc;
pub mod pptx;
pub mod pptx_writer;
pub mod revision_executor;
pub mod revision_ops;
pub mod revision_reader;
pub mod revision_writer;
pub mod span_mut;
pub mod validate;
pub mod writer;
pub mod xlsx;
pub mod xlsx_writer;
pub mod xml;
