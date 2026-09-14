//! adapters-ooxml: OOXML (.docx, .xlsx, .pptx) readers for stichos.
//!
//! Implements OPC (Open Packaging Conventions) parsing, XML content extraction,
//! and conversion to the gp-core intermediate representation.

pub mod census;
pub mod docx;
pub mod opc;
pub mod pptx;
pub mod validate;
pub mod xlsx;
pub mod xml;
