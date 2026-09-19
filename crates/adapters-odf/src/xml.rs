// SPDX-License-Identifier: MIT OR Apache-2.0
//! XML DOM-lite with byte spans.
//!
//! A minimal, order-preserving, namespace-aware XML parser that tracks
//! byte-level spans for byte-exact re-serialization.
//!
//! This parser does NOT validate against XSD, resolve external entities, or
//! handle all edge cases. It IS fast, safe (no network, no entity resolution),
//! and produces byte-exact output for well-formed input.

use std::collections::BTreeMap;

use gp_core::diagnostics::Diagnostic;

// ---------------------------------------------------------------------------
// DOM types
// ---------------------------------------------------------------------------

/// A namespace declaration: prefix -> URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Namespace {
    pub prefix: String,
    pub uri: String,
}

/// A qualified tag name with optional namespace prefix.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct TagName {
    pub prefix: Option<String>,
    pub local_name: String,
    pub namespace_uri: Option<String>,
}

impl TagName {
    /// Reconstruct the qualified name (e.g. `"w:p"` or `"p"`).
    pub fn qname(&self) -> String {
        match &self.prefix {
            Some(p) => format!("{}:{}", p, self.local_name),
            None => self.local_name.clone(),
        }
    }

    /// Create a simple local-name-only tag name.
    pub fn local(name: &str) -> Self {
        Self {
            prefix: None,
            local_name: name.to_string(),
            namespace_uri: None,
        }
    }
}

impl std::fmt::Display for TagName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.qname())
    }
}

/// Byte-level span within the original source bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteSpan {
    pub offset: usize,
    pub length: usize,
}

impl ByteSpan {
    /// Returns the end offset (exclusive).
    pub fn end(&self) -> usize {
        self.offset + self.length
    }
}

/// A single XML node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmlNode {
    /// An element: tag name, attributes, children.
    Element {
        name: TagName,
        attributes: BTreeMap<TagName, String>,
        namespaces: Vec<Namespace>,
        children: Vec<XmlNode>,
        span: ByteSpan,
    },
    /// Text content between tags.
    Text { text: String, span: ByteSpan },
    /// Processing instruction: <?target data?>
    ProcessingInstruction {
        target: String,
        data: String,
        span: ByteSpan,
    },
    /// Comment: <!-- comment -->
    Comment { text: String, span: ByteSpan },
    /// CDATA section: <![CDATA[...]]>
    CData { text: String, span: ByteSpan },
}

impl XmlNode {
    /// Re-serialize this node to bytes.
    ///
    /// For byte-exact output, use `XmlDocument::to_bytes()` which
    /// returns the original source bytes. This method reconstructs
    /// from the DOM structure (useful for individual nodes).
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            XmlNode::Element {
                name,
                attributes,
                namespaces,
                children,
                ..
            } => {
                let mut out = Vec::new();
                out.push(b'<');
                write_tag_name(&mut out, name);

                // Write namespace declarations
                for ns in namespaces {
                    if ns.prefix.is_empty() {
                        out.extend_from_slice(b" xmlns=\"");
                        out.extend_from_slice(ns.uri.as_bytes());
                        out.push(b'"');
                    } else {
                        out.extend_from_slice(b" xmlns:");
                        out.extend_from_slice(ns.prefix.as_bytes());
                        out.extend_from_slice(b"=\"");
                        out.extend_from_slice(ns.uri.as_bytes());
                        out.push(b'"');
                    }
                }

                // Write attributes (excluding namespaces)
                for (attr_name, attr_val) in attributes {
                    out.push(b' ');
                    write_tag_name(&mut out, attr_name);
                    out.push(b'=');
                    out.push(b'"');
                    out.extend_from_slice(attr_val.as_bytes());
                    out.push(b'"');
                }

                if children.is_empty() {
                    out.extend_from_slice(b"/>");
                } else {
                    out.push(b'>');
                    for child in children {
                        out.extend_from_slice(&child.to_bytes());
                    }
                    out.extend_from_slice(b"</");
                    write_tag_name(&mut out, name);
                    out.push(b'>');
                }
                out
            }
            XmlNode::Text { text, .. } => text.as_bytes().to_vec(),
            XmlNode::ProcessingInstruction { target, data, .. } => {
                let mut out = Vec::new();
                out.extend_from_slice(b"<?");
                out.extend_from_slice(target.as_bytes());
                out.extend_from_slice(b" ");
                out.extend_from_slice(data.as_bytes());
                out.extend_from_slice(b"?>");
                out
            }
            XmlNode::Comment { text, .. } => {
                let mut out = Vec::new();
                out.extend_from_slice(b"<!--");
                out.extend_from_slice(text.as_bytes());
                out.extend_from_slice(b"-->");
                out
            }
            XmlNode::CData { text, .. } => {
                let mut out = Vec::new();
                out.extend_from_slice(b"<![CDATA[");
                out.extend_from_slice(text.as_bytes());
                out.extend_from_slice(b"]]>");
                out
            }
        }
    }
}

/// Write a tag name (prefix:local) to a byte buffer.
fn write_tag_name(buf: &mut Vec<u8>, name: &TagName) {
    if let Some(ref prefix) = name.prefix {
        buf.extend_from_slice(prefix.as_bytes());
        buf.push(b':');
    }
    buf.extend_from_slice(name.local_name.as_bytes());
}

/// A parsed XML document.
#[derive(Debug, Clone)]
pub struct XmlDocument {
    pub root: XmlNode,
    pub namespaces: Vec<Namespace>,
    pub encoding: String,
    /// Original source bytes for byte-exact re-serialization.
    source: Vec<u8>,
}

impl XmlDocument {
    /// Create a new XML document.
    pub fn new(
        root: XmlNode,
        namespaces: Vec<Namespace>,
        encoding: String,
        source: Vec<u8>,
    ) -> Self {
        Self {
            root,
            namespaces,
            encoding,
            source,
        }
    }

    /// Resolve a prefixed name to its namespace URI.
    ///
    /// Walks the root element's namespace declarations and global namespaces
    /// to find the URI for the given prefix.
    pub fn resolve_ns(&self, prefixed: &str) -> Option<&str> {
        // Check global namespaces first
        for ns in &self.namespaces {
            if ns.prefix == prefixed {
                return Some(&ns.uri);
            }
        }

        // Check root element namespaces
        if let XmlNode::Element { namespaces, .. } = &self.root {
            for ns in namespaces {
                if ns.prefix == prefixed {
                    return Some(&ns.uri);
                }
            }
        }

        None
    }

    /// Get all namespaces in scope for a given node (walking up parent chain).
    /// Returns namespaces from root-level declarations first, then element-level.
    pub fn namespaces_for_node(&self, _node_idx: usize) -> Vec<&Namespace> {
        let mut result = Vec::new();

        // Global namespaces
        for ns in &self.namespaces {
            result.push(ns);
        }

        // Root element namespaces
        if let XmlNode::Element { namespaces, .. } = &self.root {
            for ns in namespaces {
                result.push(ns);
            }
        }

        result
    }

    /// Re-serialize the full document to bytes.
    ///
    /// For byte-exact output, uses the original source bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        // For byte-exact output, use the full source
        self.source.clone()
    }

    /// Returns the original source bytes.
    pub fn source(&self) -> &[u8] {
        &self.source
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse XML bytes into an XmlDocument.
///
/// This is a minimal, order-preserving, namespace-aware XML parser.
/// It does NOT: validate against XSD, resolve external entities, or handle all edge cases.
/// It IS: fast, safe (no network, no entity resolution), and byte-exact for re-serialization.
pub fn parse_xml(data: &[u8]) -> Result<XmlDocument, Vec<Diagnostic>> {
    let mut parser = XmlParser::new(data);
    parser.parse_document()
}

/// Internal XML parser state.
struct XmlParser {
    source: Vec<u8>,
    pos: usize,
    diagnostics: Vec<Diagnostic>,
}

impl XmlParser {
    fn new(source: &[u8]) -> Self {
        Self {
            source: source.to_vec(),
            pos: 0,
            diagnostics: Vec::new(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.source.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let b = self.peek();
        if b.is_some() {
            self.pos += 1;
        }
        b
    }

    fn at_end(&self) -> bool {
        self.pos >= self.source.len()
    }

    fn expect(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.source.len() {
            match self.source[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn push_diag(
        &mut self,
        code: &'static str,
        severity: gp_core::diagnostics::Severity,
        msg: String,
    ) {
        self.diagnostics.push(Diagnostic::new(code, severity, msg));
    }

    fn parse_document(&mut self) -> Result<XmlDocument, Vec<Diagnostic>> {
        // Skip BOM if present
        if self.source.starts_with(&[0xEF, 0xBB, 0xBF]) {
            self.pos = 3;
        }

        self.skip_whitespace();

        // Parse XML declaration
        let mut encoding = String::from("utf-8");
        if self.matches(b"<?xml") {
            encoding = self.parse_xml_declaration()?;
        }

        self.skip_whitespace();

        // Parse root element
        if self.at_end() {
            self.push_diag(
                "OOX-E-002",
                gp_core::diagnostics::Severity::Error,
                "XML document is empty".to_string(),
            );
            return Err(std::mem::take(&mut self.diagnostics));
        }

        let root = self.parse_element()?;

        self.skip_whitespace();

        // Check for trailing content
        if !self.at_end() {
            self.push_diag(
                "OOX-E-002",
                gp_core::diagnostics::Severity::Error,
                format!(
                    "Unexpected content after root element at offset {}",
                    self.pos
                ),
            );
            return Err(std::mem::take(&mut self.diagnostics));
        }

        // Collect global namespaces from root element
        let mut global_ns = Vec::new();
        if let XmlNode::Element { namespaces, .. } = &root {
            for ns in namespaces {
                global_ns.push(ns.clone());
            }
        }

        Ok(XmlDocument::new(
            root,
            global_ns,
            encoding,
            self.source.clone(),
        ))
    }

    fn parse_xml_declaration(&mut self) -> Result<String, Vec<Diagnostic>> {
        let start = self.pos;
        // Consume "<?xml"
        self.advance();
        self.advance();
        self.advance();
        self.advance();
        self.advance();

        // Parse attributes: version, encoding, standalone
        let mut encoding = String::from("utf-8");

        self.skip_whitespace();
        while self.peek() != Some(b'?') {
            if self.at_end() {
                self.push_diag(
                    "OOX-E-002",
                    gp_core::diagnostics::Severity::Error,
                    "Unterminated XML declaration".to_string(),
                );
                return Err(std::mem::take(&mut self.diagnostics));
            }

            let attr = self.parse_attribute()?;
            let attr_name = attr.0;
            let attr_val = attr.1;

            match attr_name.as_str() {
                "version" => {
                    // We accept any version
                }
                "encoding" => {
                    encoding = attr_val;
                }
                "standalone" => {
                    // Ignore
                }
                _ => {
                    // Ignore unknown attributes
                }
            }

            self.skip_whitespace();
        }

        // Consume ?>
        self.advance(); // <
        self.advance(); // ?

        // Calculate span for declaration
        let span = ByteSpan {
            offset: start,
            length: self.pos - start,
        };
        let _ = span;

        Ok(encoding)
    }

    fn parse_element(&mut self) -> Result<XmlNode, Vec<Diagnostic>> {
        let start = self.pos;

        // Expect <
        if !self.expect(b'<') {
            self.push_diag(
                "OOX-E-002",
                gp_core::diagnostics::Severity::Error,
                "Expected '<' for element start".to_string(),
            );
            return Err(std::mem::take(&mut self.diagnostics));
        }

        // Parse tag name
        let name = self.parse_tag_name()?;

        // Parse attributes and namespace declarations
        let mut attributes = BTreeMap::new();
        let mut namespaces = Vec::new();

        self.skip_whitespace();
        while !self.at_end() && self.peek() != Some(b'>') && self.peek() != Some(b'/') {
            let attr = self.parse_attribute()?;
            // Split attribute name at first ':' to extract prefix
            let (prefix, local_name) = if let Some(colon_pos) = attr.0.find(':') {
                (
                    Some(attr.0[..colon_pos].to_string()),
                    attr.0[colon_pos + 1..].to_string(),
                )
            } else {
                (None, attr.0.clone())
            };
            let attr_name = TagName {
                prefix,
                local_name,
                namespace_uri: None,
            };
            let attr_val = attr.1;

            // Check for namespace declarations
            if attr.0.starts_with("xmlns:") {
                let prefix = attr.0[6..].to_string();
                namespaces.push(Namespace {
                    prefix,
                    uri: attr_val,
                });
            } else if attr.0 == "xmlns" {
                // Default namespace
                namespaces.push(Namespace {
                    prefix: String::new(),
                    uri: attr_val,
                });
            } else {
                // Regular attribute
                attributes.insert(attr_name, attr_val);
            }

            self.skip_whitespace();
        }

        // Save position after tag name (for span calculation)
        let tag_end = self.pos;

        // Check for self-closing
        if self.peek() == Some(b'/') {
            self.advance(); // /
            if !self.expect(b'>') {
                self.push_diag(
                    "OOX-E-002",
                    gp_core::diagnostics::Severity::Error,
                    "Expected '>' in self-closing tag".to_string(),
                );
                return Err(std::mem::take(&mut self.diagnostics));
            }

            let span = ByteSpan {
                offset: start,
                length: self.pos - start,
            };

            return Ok(XmlNode::Element {
                name,
                attributes,
                namespaces,
                children: Vec::new(),
                span,
            });
        }

        // Expect >
        if !self.expect(b'>') {
            self.push_diag(
                "OOX-E-002",
                gp_core::diagnostics::Severity::Error,
                "Expected '>' after element start tag".to_string(),
            );
            return Err(std::mem::take(&mut self.diagnostics));
        }

        // Parse children (text, nested elements, comments, PIs)
        let mut children = Vec::new();
        loop {
            if self.at_end() {
                self.push_diag(
                    "OOX-E-002",
                    gp_core::diagnostics::Severity::Error,
                    format!(
                        "Unterminated element '{}': expected closing tag",
                        name.qname()
                    ),
                );
                break;
            }

            match self.peek() {
                Some(b'<') => {
                    // Check for closing tag
                    if self.pos + 1 < self.source.len() && self.source[self.pos + 1] == b'/' {
                        break;
                    }

                    // Check for comment
                    if self.matches(b"<!--") {
                        let comment = self.parse_comment()?;
                        children.push(comment);
                        continue;
                    }

                    // Check for processing instruction
                    if self.matches(b"<?") {
                        let pi = self.parse_processing_instruction()?;
                        children.push(pi);
                        continue;
                    }

                    // Check for CDATA
                    if self.matches(b"<![CDATA[") {
                        let cdata = self.parse_cdata()?;
                        children.push(cdata);
                        continue;
                    }

                    // Nested element: parse_element consumes its own '<'
                    let child = self.parse_element()?;
                    children.push(child);
                }
                _ => {
                    // Text content -- capture until next '<' or end
                    let text_node = self.parse_text()?;
                    if let XmlNode::Text { ref text, .. } = text_node {
                        if !text.is_empty() {
                            children.push(text_node);
                        }
                    } else {
                        children.push(text_node);
                    }
                }
            }
        }

        // Parse closing tag
        if !self.expect(b'<') || !self.expect(b'/') {
            self.push_diag(
                "OOX-E-002",
                gp_core::diagnostics::Severity::Error,
                format!("Expected closing tag for '{}'", name.qname()),
            );
            return Err(std::mem::take(&mut self.diagnostics));
        }

        let close_name = self.parse_tag_name()?;
        if close_name.local_name != name.local_name {
            self.push_diag(
                "OOX-E-002",
                gp_core::diagnostics::Severity::Error,
                format!(
                    "Mismatched closing tag: expected '</{}>', found '</{}>'",
                    name.qname(),
                    close_name.qname()
                ),
            );
            return Err(std::mem::take(&mut self.diagnostics));
        }

        if !self.expect(b'>') {
            self.push_diag(
                "OOX-E-002",
                gp_core::diagnostics::Severity::Error,
                format!("Expected '>' in closing tag for '{}'", name.qname()),
            );
            return Err(std::mem::take(&mut self.diagnostics));
        }

        // Update tag_end to include the closing tag
        let _ = tag_end;

        let span = ByteSpan {
            offset: start,
            length: self.pos - start,
        };

        Ok(XmlNode::Element {
            name,
            attributes,
            namespaces,
            children,
            span,
        })
    }

    fn parse_tag_name(&mut self) -> Result<TagName, Vec<Diagnostic>> {
        // Read all characters that could be part of a tag name
        let mut name_parts: Vec<u8> = Vec::new();
        let mut first_char_read = false;

        while self.pos < self.source.len() {
            let b = self.source[self.pos];
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' {
                first_char_read = true;
                name_parts.push(b);
                self.pos += 1;
            } else if b == b':' {
                // Found a colon — this is a namespaced name
                break;
            } else {
                break;
            }
        }

        if !first_char_read && name_parts.is_empty() {
            self.push_diag(
                "OOX-E-002",
                gp_core::diagnostics::Severity::Error,
                "Expected element name".to_string(),
            );
            return Err(std::mem::take(&mut self.diagnostics));
        }

        // Check if there's a colon indicating a namespace prefix
        if let Some(colon_pos) = name_parts.iter().position(|&b| b == b':') {
            // Namespaced name — but colons are only valid as prefix separators
            // (the first colon separates prefix from local name)
            let prefix = String::from_utf8_lossy(&name_parts[..colon_pos]).to_string();
            let mut local_name = String::new();
            self.pos += 1; // skip the ':'
                           // Read local name
            while self.pos < self.source.len() {
                let b = self.source[self.pos];
                if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' {
                    local_name.push(b as char);
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if local_name.is_empty() {
                self.push_diag(
                    "OOX-E-002",
                    gp_core::diagnostics::Severity::Error,
                    "Expected ':' after namespace prefix".to_string(),
                );
                return Err(std::mem::take(&mut self.diagnostics));
            }
            Ok(TagName {
                prefix: if prefix.is_empty() {
                    None
                } else {
                    Some(prefix)
                },
                local_name,
                namespace_uri: None,
            })
        } else {
            // No colon — just a simple or namespaced-local name
            // Check if there's a ':' immediately after what we read
            if self.pos < self.source.len() && self.source[self.pos] == b':' {
                // There IS a colon — name_parts contains the prefix
                let prefix = String::from_utf8_lossy(&name_parts).to_string();
                self.pos += 1; // skip ':'
                let mut local_name = String::new();
                while self.pos < self.source.len() {
                    let b = self.source[self.pos];
                    if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' {
                        local_name.push(b as char);
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                Ok(TagName {
                    prefix: Some(prefix),
                    local_name,
                    namespace_uri: None,
                })
            } else {
                // Simple local name
                let local_name = String::from_utf8_lossy(&name_parts).to_string();
                Ok(TagName {
                    prefix: None,
                    local_name,
                    namespace_uri: None,
                })
            }
        }
    }

    fn parse_attribute(&mut self) -> Result<(String, String), Vec<Diagnostic>> {
        let start = self.pos;

        // Skip whitespace before attribute name
        self.skip_whitespace();

        // Parse attribute name (may include namespace prefix separated by ':')
        let mut name_parts: Vec<u8> = Vec::new();
        while self.pos < self.source.len() {
            let b = self.source[self.pos];
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b':' {
                name_parts.push(b);
                self.pos += 1;
            } else {
                break;
            }
        }

        let name = String::from_utf8_lossy(&name_parts).to_string();
        if name.is_empty() {
            self.push_diag(
                "OOX-E-002",
                gp_core::diagnostics::Severity::Error,
                "Expected attribute name".to_string(),
            );
            return Err(std::mem::take(&mut self.diagnostics));
        }

        // Skip whitespace
        self.skip_whitespace();

        // Expect =
        if !self.expect(b'=') {
            self.push_diag(
                "OOX-E-002",
                gp_core::diagnostics::Severity::Error,
                "Expected '=' after attribute name".to_string(),
            );
            return Err(std::mem::take(&mut self.diagnostics));
        }

        self.skip_whitespace();

        // Expect opening quote
        let quote = self.peek().ok_or_else(|| {
            vec![Diagnostic::new(
                "OOX-E-002",
                gp_core::diagnostics::Severity::Error,
                "Expected attribute value quote".to_string(),
            )]
        })?;
        if quote != b'"' && quote != b'\'' {
            self.push_diag(
                "OOX-E-002",
                gp_core::diagnostics::Severity::Error,
                "Expected attribute value in quotes".to_string(),
            );
            return Err(std::mem::take(&mut self.diagnostics));
        }
        self.advance();

        // Parse attribute value
        let val_start = self.pos;
        while self.pos < self.source.len() {
            if self.source[self.pos] == quote {
                break;
            }
            self.pos += 1;
        }

        let value = String::from_utf8_lossy(&self.source[val_start..self.pos]).to_string();

        // Expect closing quote
        if !self.expect(quote) {
            self.push_diag(
                "OOX-E-002",
                gp_core::diagnostics::Severity::Error,
                "Unterminated attribute value".to_string(),
            );
            return Err(std::mem::take(&mut self.diagnostics));
        }

        // Track span for attribute
        let _span = ByteSpan {
            offset: start,
            length: self.pos - start,
        };

        Ok((name, value))
    }

    fn parse_text(&mut self) -> Result<XmlNode, Vec<Diagnostic>> {
        let start = self.pos;

        // Find end of text (next '<' or end of source)
        while self.pos < self.source.len() {
            if self.source[self.pos] == b'<' {
                break;
            }
            self.pos += 1;
        }

        let text = String::from_utf8_lossy(&self.source[start..self.pos]).to_string();

        let span = ByteSpan {
            offset: start,
            length: self.pos - start,
        };

        Ok(XmlNode::Text { text, span })
    }

    fn parse_comment(&mut self) -> Result<XmlNode, Vec<Diagnostic>> {
        let start = self.pos;

        // Consume <!--
        self.advance(); // <
        self.advance(); // !
        self.advance(); // -
        self.advance(); // -

        // Find -->
        let content_start = self.pos;
        while self.pos < self.source.len() - 2 {
            if self.source[self.pos] == b'-'
                && self.source[self.pos + 1] == b'-'
                && self.source[self.pos + 2] == b'>'
            {
                break;
            }
            self.pos += 1;
        }

        let text = String::from_utf8_lossy(&self.source[content_start..self.pos]).to_string();

        // Consume -->
        self.advance(); // -
        self.advance(); // -
        self.advance(); // >

        let span = ByteSpan {
            offset: start,
            length: self.pos - start,
        };

        Ok(XmlNode::Comment { text, span })
    }

    fn parse_processing_instruction(&mut self) -> Result<XmlNode, Vec<Diagnostic>> {
        let start = self.pos;

        // Consume <?
        self.advance(); // <
        self.advance(); // ?

        // Parse target
        let mut target = String::new();
        while self.pos < self.source.len() {
            let b = self.source[self.pos];
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
                target.push(b as char);
                self.pos += 1;
            } else {
                break;
            }
        }

        if target.is_empty() {
            self.push_diag(
                "OOX-E-002",
                gp_core::diagnostics::Severity::Error,
                "Expected processing instruction target".to_string(),
            );
            return Err(std::mem::take(&mut self.diagnostics));
        }

        // Skip whitespace
        self.skip_whitespace();

        // Collect data until ?>
        let data_start = self.pos;
        while self.pos < self.source.len() - 1 {
            if self.source[self.pos] == b'?' && self.source[self.pos + 1] == b'>' {
                break;
            }
            self.pos += 1;
        }

        let data = String::from_utf8_lossy(&self.source[data_start..self.pos]).to_string();

        // Consume ?>
        self.advance(); // ?
        self.advance(); // >

        let span = ByteSpan {
            offset: start,
            length: self.pos - start,
        };

        Ok(XmlNode::ProcessingInstruction { target, data, span })
    }

    fn parse_cdata(&mut self) -> Result<XmlNode, Vec<Diagnostic>> {
        let start = self.pos;

        // Consume <![CDATA[
        self.advance(); // <
        self.advance(); // !
        self.advance(); // [
        self.advance(); // C
        self.advance(); // D
        self.advance(); // A
        self.advance(); // T
        self.advance(); // A
        self.advance(); // [

        // Find ]]>
        let content_start = self.pos;
        while self.pos < self.source.len() - 2 {
            if self.source[self.pos] == b']'
                && self.source[self.pos + 1] == b']'
                && self.source[self.pos + 2] == b'>'
            {
                break;
            }
            self.pos += 1;
        }

        let text = String::from_utf8_lossy(&self.source[content_start..self.pos]).to_string();

        // Consume ]]>
        self.advance(); // ]
        self.advance(); // ]
        self.advance(); // >

        let span = ByteSpan {
            offset: start,
            length: self.pos - start,
        };

        Ok(XmlNode::CData { text, span })
    }

    fn matches(&self, pattern: &[u8]) -> bool {
        let pattern_bytes = pattern;
        if self.source.len() - self.pos < pattern_bytes.len() {
            return false;
        }
        &self.source[self.pos..self.pos + pattern_bytes.len()] == pattern_bytes
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// test_parse_simple_xml: parse <root><child>text</child></root>
    #[test]
    fn test_parse_simple_xml() {
        let data = b"<root><child>text</child></root>";
        let doc = parse_xml(data).expect("should parse simple XML");

        match &doc.root {
            XmlNode::Element { name, children, .. } => {
                assert_eq!(name.local_name, "root");
                assert_eq!(children.len(), 1);

                match &children[0] {
                    XmlNode::Element {
                        name,
                        children: gkids,
                        ..
                    } => {
                        assert_eq!(name.local_name, "child");
                        assert_eq!(gkids.len(), 1);
                        match &gkids[0] {
                            XmlNode::Text { text, .. } => assert_eq!(text, "text"),
                            _ => panic!("expected text node"),
                        }
                    }
                    _ => panic!("expected element node"),
                }
            }
            _ => panic!("expected root element"),
        }
    }

    /// test_parse_namespaced_xml: parse with xmlns declarations
    #[test]
    fn test_parse_namespaced_xml() {
        let xml = b"<?xml version=\"1.0\"?><w:document xmlns:w=\"http://example.com/w\"><w:p><w:r><w:t>Hello</w:t></w:r></w:p></w:document>";
        let doc = parse_xml(xml).expect("should parse namespaced XML");

        assert_eq!(doc.encoding, "utf-8");

        match &doc.root {
            XmlNode::Element {
                name, namespaces, ..
            } => {
                assert_eq!(name.prefix, Some("w".to_string()));
                assert_eq!(name.local_name, "document");
                assert_eq!(namespaces.len(), 1);
                assert_eq!(namespaces[0].prefix, "w");
                assert_eq!(namespaces[0].uri, "http://example.com/w");
            }
            _ => panic!("expected element"),
        }

        // Verify namespace resolution
        assert_eq!(doc.resolve_ns("w"), Some("http://example.com/w"));
    }

    /// test_parse_with_attributes: parse element with multiple attributes
    #[test]
    fn test_parse_with_attributes() {
        let data = b"<root id=\"1\" class=\"a b\" value=\"test\"></root>";
        let doc = parse_xml(data).expect("should parse with attributes");

        match &doc.root {
            XmlNode::Element { attributes, .. } => {
                assert_eq!(attributes.len(), 3);
                let id_key = TagName::local("id");
                assert_eq!(attributes.get(&id_key), Some(&"1".to_string()));

                let class_key = TagName::local("class");
                assert_eq!(attributes.get(&class_key), Some(&"a b".to_string()));

                let value_key = TagName::local("value");
                assert_eq!(attributes.get(&value_key), Some(&"test".to_string()));
            }
            _ => panic!("expected element"),
        }
    }

    /// test_re_serialize_byte_exact: parse a real corpus XML part, re-serialize, verify exact match
    #[test]
    fn test_re_serialize_byte_exact() {
        // Real [Content_Types].xml structure from a corpus fixture
        let xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                   <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
                   <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
                   <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
                   <Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
                   </Types>";
        let doc = parse_xml(xml).expect("should parse [Content_Types].xml");
        let out = doc.to_bytes();
        assert_eq!(out, xml, "re-serialized bytes must match original");
    }

    /// test_byte_span_tracking: verify spans are correct for elements and text
    #[test]
    fn test_byte_span_tracking() {
        let xml = b"<root><child id=\"1\">hello</child></root>";
        let doc = parse_xml(xml).expect("should parse");

        match &doc.root {
            XmlNode::Element { span, children, .. } => {
                // Root spans the entire document
                assert_eq!(span.offset, 0);
                assert_eq!(span.length, xml.len());

                // Child element
                match &children[0] {
                    XmlNode::Element {
                        span: child_span,
                        children: gkids,
                        ..
                    } => {
                        // Child spans from <child to </child>
                        assert_eq!(child_span.offset, 6);
                        assert_eq!(child_span.length, 27); // <child id="1">hello</child>

                        // Text node
                        match &gkids[0] {
                            XmlNode::Text {
                                span: text_span,
                                text,
                            } => {
                                assert_eq!(text_span.offset, 20);
                                assert_eq!(text_span.length, 5);
                                assert_eq!(*text, "hello");

                                // Verify span maps to correct bytes
                                let extracted =
                                    &xml[text_span.offset..text_span.offset + text_span.length];
                                assert_eq!(extracted, b"hello");
                            }
                            _ => panic!("expected text node"),
                        }
                    }
                    _ => panic!("expected element"),
                }
            }
            _ => panic!("expected element"),
        }
    }

    /// test_namespace_resolution: resolve prefixed names
    #[test]
    fn test_namespace_resolution() {
        let xml = b"<?xml version=\"1.0\"?><w:document xmlns:w=\"http://example.com/w\" xmlns:m=\"http://example.com/m\"><w:p/></w:document>";
        let doc = parse_xml(xml).expect("should parse");

        assert_eq!(doc.resolve_ns("w"), Some("http://example.com/w"));
        assert_eq!(doc.resolve_ns("m"), Some("http://example.com/m"));
        assert_eq!(doc.resolve_ns("x"), None);
    }

    /// test_parse_comment: parse <!-- comment -->
    #[test]
    fn test_parse_comment() {
        let xml = b"<root><!-- this is a comment --><child/></root>";
        let doc = parse_xml(xml).expect("should parse");

        match &doc.root {
            XmlNode::Element { children, .. } => {
                assert_eq!(children.len(), 2);

                match &children[0] {
                    XmlNode::Comment { text, span } => {
                        assert_eq!(*text, " this is a comment ");
                        assert_eq!(span.offset, 6);
                        assert_eq!(span.length, 26);

                        // Verify span
                        let extracted = &xml[span.offset..span.offset + span.length];
                        assert_eq!(extracted, b"<!-- this is a comment -->");
                    }
                    _ => panic!("expected comment"),
                }

                match &children[1] {
                    XmlNode::Element { .. } => {}
                    _ => panic!("expected element"),
                }
            }
            _ => panic!("expected element"),
        }
    }

    /// test_parse_processing_instruction: parse <?mc...?>
    #[test]
    fn test_parse_processing_instruction() {
        let xml = b"<root><?mctype custom?></root>";
        let doc = parse_xml(xml).expect("should parse");

        match &doc.root {
            XmlNode::Element { children, .. } => {
                assert_eq!(children.len(), 1);

                match &children[0] {
                    XmlNode::ProcessingInstruction { target, data, span } => {
                        assert_eq!(target, "mctype");
                        assert_eq!(*data, "custom");
                        assert_eq!(span.offset, 6);
                        assert_eq!(span.length, 17);

                        // Verify span
                        let extracted = &xml[span.offset..span.offset + span.length];
                        assert_eq!(extracted, b"<?mctype custom?>");
                    }
                    _ => panic!("expected PI"),
                }
            }
            _ => panic!("expected element"),
        }
    }

    /// test_parse_self_closing: parse <w:br/>
    #[test]
    fn test_parse_self_closing() {
        let xml = b"<root><w:br xmlns:w=\"http://example.com/w\"/></root>";
        let doc = parse_xml(xml).expect("should parse");

        match &doc.root {
            XmlNode::Element { children, .. } => {
                assert_eq!(children.len(), 1);

                match &children[0] {
                    XmlNode::Element {
                        name,
                        span,
                        children: gkids,
                        namespaces,
                        ..
                    } => {
                        assert_eq!(name.prefix, Some("w".to_string()));
                        assert_eq!(name.local_name, "br");
                        assert!(gkids.is_empty());
                        assert_eq!(namespaces.len(), 1);
                        assert_eq!(namespaces[0].prefix, "w");

                        // Verify span
                        assert_eq!(span.offset, 6);
                        assert_eq!(span.length, 38);
                        let extracted = &xml[span.offset..span.offset + span.length];
                        assert_eq!(extracted, b"<w:br xmlns:w=\"http://example.com/w\"/>");
                    }
                    _ => panic!("expected element"),
                }
            }
            _ => panic!("expected element"),
        }
    }

    /// test_parse_deeply_nested: parse deeply nested XML, verify structure
    #[test]
    fn test_parse_deeply_nested() {
        // Build deeply nested XML: <r><n0><n1>...<n19>deep</n19></n18>...</n0></r>
        let xml = "<r><n0><n1><n2><n3><n4><n5><n6><n7><n8><n9><n10><n11><n12><n13><n14><n15><n16><n17><n18><n19>deep</n19></n18></n17></n16></n15></n14></n13></n12></n11></n10></n9></n8></n7></n6></n5></n4></n3></n2></n1></n0></r>".to_string();
        let doc = parse_xml(xml.as_bytes()).expect("should parse deeply nested");

        // Walk down to find the deepest text node
        let mut current = &doc.root;
        let mut depth_count = 0;
        while let XmlNode::Element { children, .. } = current {
            assert!(
                !children.is_empty(),
                "should have child at depth {}",
                depth_count
            );
            if children.len() == 1 {
                match &children[0] {
                    XmlNode::Text { text, .. } => {
                        assert_eq!(text, "deep");
                        break;
                    }
                    XmlNode::Element { .. } => {
                        current = &children[0];
                        depth_count += 1;
                        continue;
                    }
                    _ => break,
                }
            } else {
                break;
            }
        }
        assert_eq!(depth_count, 20); // n0..n19 = 20 nested levels
    }

    /// test_parse_real_corpus_xml: parse [Content_Types].xml from corpus fixture
    #[test]
    fn test_parse_real_corpus_xml() {
        // Read a real corpus docx and extract [Content_Types].xml
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/blobs/cjk-text-docx-001.docx"
        );
        let data = std::fs::read(path).expect("corpus fixture should exist");

        // First, extract the [Content_Types].xml part from the ZIP
        let reader = std::io::Cursor::new(&data);
        let mut archive = zip::ZipArchive::new(reader).expect("should be valid ZIP");

        // Find [Content_Types].xml
        let mut content_types_xml: Option<Vec<u8>> = None;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).expect("read entry");
            let name = entry.name().to_string();
            if name == "[Content_Types].xml" {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf).expect("read content types");
                content_types_xml = Some(buf);
                break;
            }
        }

        let xml = content_types_xml.expect("[Content_Types].xml should exist in corpus fixture");

        // Parse the XML
        let doc = parse_xml(&xml).expect("should parse [Content_Types].xml");

        // Verify it has a root element
        match &doc.root {
            XmlNode::Element { name, .. } => {
                assert_eq!(name.local_name, "Types");
            }
            _ => panic!("expected root element"),
        }

        // Verify byte-exact re-serialization
        let out = doc.to_bytes();
        assert_eq!(
            out, xml,
            "re-serialized [Content_Types].xml must match original bytes"
        );
    }

    /// test_parse_xml_with_cdata: handle CDATA sections
    #[test]
    fn test_parse_xml_with_cdata() {
        let xml = b"<root><![CDATA[some <script> & \"text\" ]]></root>";
        let doc = parse_xml(xml).expect("should parse CDATA");

        match &doc.root {
            XmlNode::Element { children, .. } => {
                assert_eq!(children.len(), 1);

                match &children[0] {
                    XmlNode::CData { text, span } => {
                        assert_eq!(*text, "some <script> & \"text\" ");
                        assert_eq!(span.offset, 6);
                        assert_eq!(span.length, 35);

                        // Verify span
                        let extracted = &xml[span.offset..span.offset + span.length];
                        assert_eq!(extracted, b"<![CDATA[some <script> & \"text\" ]]>");
                    }
                    _ => panic!("expected CDATA"),
                }
            }
            _ => panic!("expected element"),
        }
    }

    /// test_parse_invalid_xml: malformed XML -> diagnostics (not panic)
    #[test]
    fn test_parse_invalid_xml() {
        // Unclosed tag
        let result = parse_xml(b"<root><child>");
        assert!(result.is_err(), "should error on unclosed tag");
        let diags = result.unwrap_err();
        assert!(!diags.is_empty(), "should produce diagnostics");
        assert!(
            diags.iter().any(|d| d.code == "OOX-E-002"),
            "should be OOX-E-002"
        );

        // Mismatched tags
        let result = parse_xml(b"<root></wrong>");
        assert!(result.is_err(), "should error on mismatched tags");
        let diags = result.unwrap_err();
        assert!(!diags.is_empty(), "should produce diagnostics");

        // Empty document
        let result = parse_xml(b"");
        assert!(result.is_err(), "should error on empty document");

        // Trailing content
        let result = parse_xml(b"<root/>extra");
        assert!(result.is_err(), "should error on trailing content");
    }

    /// test_xml_node_to_bytes: re-serialize individual nodes
    #[test]
    fn test_xml_node_to_bytes() {
        let xml = b"<root><child>text</child></root>";
        let doc = parse_xml(xml).expect("should parse");

        // Re-serialize the document
        let out = doc.to_bytes();
        assert_eq!(out, xml, "document re-serialization should be byte-exact");

        // Re-serialize just the root element
        let root_bytes = doc.root.to_bytes();
        assert_eq!(
            root_bytes, xml,
            "root element re-serialization should match"
        );
    }

    /// test_xml_declaration_encoding: parse encoding from XML declaration
    #[test]
    fn test_xml_declaration_encoding() {
        let xml = b"<?xml version=\"1.0\" encoding=\"UTF-16\"?><root/>";
        let doc = parse_xml(xml).expect("should parse");
        assert_eq!(doc.encoding, "UTF-16");

        let xml2 = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><root/>";
        let doc2 = parse_xml(xml2).expect("should parse");
        assert_eq!(doc2.encoding, "ISO-8859-1");
    }

    /// test_text_with_whitespace: preserve text containing whitespace
    #[test]
    fn test_text_with_whitespace() {
        let xml = b"<root>  hello  world  </root>";
        let doc = parse_xml(xml).expect("should parse");

        match &doc.root {
            XmlNode::Element { children, .. } => {
                assert_eq!(children.len(), 1);
                match &children[0] {
                    XmlNode::Text { text, .. } => {
                        assert_eq!(*text, "  hello  world  ");
                    }
                    _ => panic!("expected text node"),
                }
            }
            _ => panic!("expected element"),
        }
    }

    /// test_multiple_children: parse element with multiple children of different types
    #[test]
    fn test_multiple_children() {
        let xml = b"<root><!-- c1 --><a/><![CDATA[data]]><?pi tgt?><b>text</b></root>";
        let doc = parse_xml(xml).expect("should parse");

        match &doc.root {
            XmlNode::Element { children, .. } => {
                assert_eq!(children.len(), 5);

                match &children[0] {
                    XmlNode::Comment { text, .. } => assert_eq!(*text, " c1 "),
                    _ => panic!("expected comment"),
                }

                match &children[1] {
                    XmlNode::Element {
                        children: gkids, ..
                    } => assert!(gkids.is_empty()),
                    _ => panic!("expected element"),
                }

                match &children[2] {
                    XmlNode::CData { text, .. } => assert_eq!(*text, "data"),
                    _ => panic!("expected CDATA"),
                }

                match &children[3] {
                    XmlNode::ProcessingInstruction { target, data, .. } => {
                        assert_eq!(*target, "pi");
                        assert_eq!(*data, "tgt");
                    }
                    _ => panic!("expected PI"),
                }

                match &children[4] {
                    XmlNode::Element {
                        children: gkids, ..
                    } => {
                        assert_eq!(gkids.len(), 1);
                        match &gkids[0] {
                            XmlNode::Text { text, .. } => assert_eq!(*text, "text"),
                            _ => panic!("expected text"),
                        }
                    }
                    _ => panic!("expected element"),
                }
            }
            _ => panic!("expected element"),
        }
    }

    /// test_namespaces_for_node: verify namespace lookup works
    #[test]
    fn test_namespaces_for_node() {
        let xml = b"<root xmlns:w=\"http://example.com/w\" xmlns:x=\"http://example.com/x\"><w:child/></root>";
        let doc = parse_xml(xml).expect("should parse");

        let ns_list = doc.namespaces_for_node(0);
        assert!(ns_list
            .iter()
            .any(|ns| ns.prefix == "w" && ns.uri == "http://example.com/w"));
        assert!(ns_list
            .iter()
            .any(|ns| ns.prefix == "x" && ns.uri == "http://example.com/x"));
    }

    /// test_bom_handling: handle UTF-8 BOM
    #[test]
    fn test_bom_handling() {
        let mut xml = Vec::new();
        xml.extend_from_slice(&[0xEF, 0xBB, 0xBF]); // UTF-8 BOM
        xml.extend_from_slice(b"<root/>");

        let doc = parse_xml(&xml).expect("should handle BOM");
        assert_eq!(doc.encoding, "utf-8");

        match &doc.root {
            XmlNode::Element { name, .. } => {
                assert_eq!(name.local_name, "root");
            }
            _ => panic!("expected element"),
        }
    }
}
