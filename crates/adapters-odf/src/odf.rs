// adapters-odf: ODF module stubs (deprecated — use ods.rs and odp.rs directly)
use gp_core::diagnostics::Diagnostic;

/// Stub ODF package reader
pub fn odf_parts(_data: &[u8]) -> Result<Vec<PartInfo>, Vec<Diagnostic>> {
    Ok(vec![])
}

pub struct PartInfo {
    pub uri: String,
    pub content_type: String,
}
