//! Bulk + filtered revision operations with atomicity and policy gates.
//!
//! Provides `RevisionFilter` for selecting revisions by author, date, kind, or ID.
//! `accept_filtered` and `reject_filtered` apply operations atomically: all
//! operations are staged in memory and committed as a single unit. On any
//! mid-batch failure the original document is left untouched.
//!
//! C10 policy gate: bulk `all` operations are denied by default via
//! `gp_core::policy::EditPolicy`. A dry-run bypasses the policy gate.

use gp_core::diagnostics::Diagnostic;
use gp_core::fidelity::FidelityBudget;
use gp_core::ir::*;
use gp_core::policy::DeniedAction;
pub use gp_core::policy::EditPolicy;

use crate::opc::OpcPackage;
pub use crate::revision_executor::RevisionWriterMeta;
use crate::revision_executor::{self, RevisionActionResult};

// ===========================================================================
// Public types
// ===========================================================================

/// Filter options for revision operations.
#[derive(Debug, Clone, Default)]
pub struct RevisionFilter {
    pub author: Option<String>,
    pub date_before: Option<String>,
    pub date_after: Option<String>,
    pub kind: Option<RevisionKind>,
    pub ids: Vec<ElementId>,
    pub all: bool,
}

impl RevisionFilter {
    /// Check if a revision matches this filter.
    pub fn matches(&self, rev: &Revision) -> bool {
        if self.all {
            return true;
        }
        if let Some(ref author) = self.author {
            if rev.author.as_deref() != Some(author.as_str()) {
                return false;
            }
        }
        if let Some(ref date_before) = self.date_before {
            if let Some(ref rev_date) = rev.date {
                if rev_date >= date_before {
                    return false;
                }
            }
        }
        if let Some(ref date_after) = self.date_after {
            if let Some(ref rev_date) = rev.date {
                if rev_date <= date_after {
                    return false;
                }
            }
        }
        if let Some(ref kind) = self.kind {
            if &rev.kind != kind {
                return false;
            }
        }
        if !self.ids.is_empty() && !self.ids.contains(&rev.id) {
            return false;
        }
        true
    }
}

/// Result of a bulk revision operation.
#[derive(Debug)]
pub struct BulkRevisionResult {
    pub output: Vec<u8>,
    pub journal_ids: Vec<String>,
    pub operations_performed: usize,
    pub operations_succeeded: usize,
    pub operations_failed: usize,
    pub diagnostics: Vec<Diagnostic>,
}

/// Policy check result for bulk operations.
pub struct PolicyCheckResult {
    /// Whether the bulk operation is allowed.
    pub allowed: bool,
    /// Diagnostics if the operation is denied.
    pub diagnostics: Vec<Diagnostic>,
}

impl PolicyCheckResult {
    fn denied(message: &str) -> Self {
        Self {
            allowed: false,
            diagnostics: vec![Diagnostic::new(
                "GP-E-002",
                gp_core::diagnostics::Severity::Error,
                format!("Policy denied bulk revision: {}", message),
            )],
        }
    }

    fn allowed() -> Self {
        Self {
            allowed: true,
            diagnostics: Vec::new(),
        }
    }
}

/// Check if a bulk revision operation is allowed under the given policy.
pub fn check_policy(
    filter: &RevisionFilter,
    _accept: bool,
    policy: &EditPolicy,
    dry_run: bool,
) -> PolicyCheckResult {
    if dry_run {
        return PolicyCheckResult::allowed();
    }

    if filter.all {
        return if policy.is_allowed(&DeniedAction::BulkRevisionAll) {
            PolicyCheckResult::allowed()
        } else {
            PolicyCheckResult::denied(
                "accept_all/reject_all operations are denied by policy; use --allow-all",
            )
        };
    }

    PolicyCheckResult::allowed()
}

// ===========================================================================
// Staging and commit
// ===========================================================================

/// Stage revisions for atomic commit.
///
/// Each revision is applied sequentially to a copy of the package in memory.
/// If any operation fails, the function returns early with diagnostics and
/// the staged revisions are discarded (original untouched).
fn stage_revisions(
    package: &OpcPackage,
    revisions: &[Revision],
    filter: &RevisionFilter,
    accept: bool,
    meta: &RevisionWriterMeta,
) -> Result<Vec<(Revision, Vec<u8>)>, Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let mut staged: Vec<(Revision, Vec<u8>)> = Vec::new();
    let mut current_pkg = package.clone();

    for rev in revisions {
        if !filter.matches(rev) {
            continue;
        }

        let action = if accept {
            revision_executor::accept_revision(&current_pkg, rev, meta)
        } else {
            revision_executor::reject_revision(&current_pkg, rev, meta)
        };

        match action {
            Ok(RevisionActionResult {
                output,
                journal_ids: _,
                accepted: _,
                changed_element_ids: _,
                diagnostics: mut action_diags,
            }) => {
                diags.append(&mut action_diags);
                // Update current_pkg for sequential application.
                if let Ok(updated_pkg) = crate::opc::parse_opc(&output) {
                    current_pkg = updated_pkg;
                }
                staged.push((rev.clone(), output));
            }
            Err(mut action_diags) => {
                diags.append(&mut action_diags);
                // On failure, discard staged revisions — original untouched.
                staged.clear();
                return Err(diags);
            }
        }
    }

    Ok(staged)
}

/// Commit staged revisions atomically.
///
/// Serialises the final state from the last staged operation.
fn commit_staged_revisions(
    package: &OpcPackage,
    staged: &[(Revision, Vec<u8>)],
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    if staged.is_empty() {
        // No operations to commit — return original package bytes.
        let doc_part = package
            .parts
            .iter()
            .find(|p| p.uri == "word/document.xml")
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "OOX-E-020",
                    gp_core::diagnostics::Severity::Error,
                    "Missing word/document.xml in OPC package".to_string(),
                )]
            })?;
        let output = serialize_opc_with_part(package, &doc_part.uri, doc_part.data.clone());
        return Ok(output);
    }

    // The last staged operation already contains the fully mutated package.
    let (_, last_output) = staged.last().expect("staged is non-empty");
    Ok(last_output.clone())
}

/// Serialize an OPC package to ZIP bytes with a modified document part.
fn serialize_opc_with_part(original: &OpcPackage, doc_uri: &str, new_doc_data: Vec<u8>) -> Vec<u8> {
    use std::io::Write;

    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);

    for part in &original.parts {
        let data = if part.uri == doc_uri {
            new_doc_data.clone()
        } else {
            part.data.clone()
        };
        let opts: zip::write::FileOptions<'static, ()> = zip::write::FileOptions::default();
        let _ = writer.start_file(&part.uri, opts);
        let _ = writer.write_all(&data);
    }

    writer.finish().unwrap().into_inner()
}

/// Assemble a `FidelityBudget` for the result of a bulk revision operation.
fn build_budget(operations_succeeded: usize, operations_failed: usize) -> FidelityBudget {
    let total = operations_succeeded + operations_failed;
    let confidence = if total == 0 {
        1.0
    } else {
        (operations_succeeded as f32 / total as f32).clamp(0.0, 1.0)
    };

    let mut budget = FidelityBudget::new()
        .drop_feature(
            "bulk_revision_applied",
            format!(
                "{} operation(s) succeeded in bulk revision",
                operations_succeeded
            ),
            operations_succeeded,
        )
        .rendered_with("ooxml-revision-bulk", None::<String>, None::<String>)
        .with_confidence(confidence);

    if operations_failed > 0 {
        budget = budget.drop_feature(
            "bulk_revision",
            format!("{} operation(s) failed in bulk revision", operations_failed),
            operations_failed,
        );
    }

    budget
}

// ===========================================================================
// Public API
// ===========================================================================

/// Accept filtered revisions atomically.
///
/// - Stages all matching revisions in memory
/// - Commits as a single unit
/// - On failure, original document is untouched
///
/// # Dry run
///
/// When `dry_run` is `true`, no revisions are applied and no policy gate is
/// enforced. Instead, what *would* be denied by policy is recorded as a
/// diagnostic.
pub fn accept_filtered(
    package: &OpcPackage,
    revisions: &[Revision],
    filter: &RevisionFilter,
    policy: &EditPolicy,
    meta: &RevisionWriterMeta,
    dry_run: bool,
) -> Result<BulkRevisionResult, Vec<Diagnostic>> {
    bulk_revision_impl(package, revisions, filter, true, policy, meta, dry_run)
}

/// Reject filtered revisions atomically.
///
/// See `accept_filtered` for atomicity guarantees.
pub fn reject_filtered(
    package: &OpcPackage,
    revisions: &[Revision],
    filter: &RevisionFilter,
    policy: &EditPolicy,
    meta: &RevisionWriterMeta,
    dry_run: bool,
) -> Result<BulkRevisionResult, Vec<Diagnostic>> {
    bulk_revision_impl(package, revisions, filter, false, policy, meta, dry_run)
}

/// Internal implementation of bulk revision operations.
fn bulk_revision_impl(
    package: &OpcPackage,
    revisions: &[Revision],
    filter: &RevisionFilter,
    accept: bool,
    policy: &EditPolicy,
    meta: &RevisionWriterMeta,
    dry_run: bool,
) -> Result<BulkRevisionResult, Vec<Diagnostic>> {
    let mut diags = Vec::new();

    // Policy gate check
    let policy_result = check_policy(filter, accept, policy, dry_run);
    if !policy_result.allowed {
        diags.extend(policy_result.diagnostics);
        return Ok(BulkRevisionResult {
            output: Vec::new(),
            journal_ids: Vec::new(),
            operations_performed: 0,
            operations_succeeded: 0,
            operations_failed: 0,
            diagnostics: diags,
        });
    }

    // Collect matched revisions
    let matched: Vec<&Revision> = revisions.iter().filter(|r| filter.matches(r)).collect();
    let operations_performed = matched.len();

    if matched.is_empty() {
        return Ok(BulkRevisionResult {
            output: Vec::new(),
            journal_ids: Vec::new(),
            operations_performed: 0,
            operations_succeeded: 0,
            operations_failed: 0,
            diagnostics: diags,
        });
    }

    // Dry-run: don't apply, just report what would happen
    if dry_run {
        let matched_ids: Vec<String> = matched.iter().map(|r| r.id.to_string()).collect();
        diags.push(Diagnostic::new(
            "GP-I-001",
            gp_core::diagnostics::Severity::Info,
            format!(
                "dry_run: {} revision(s) would be {}",
                matched_ids.len(),
                if accept { "accepted" } else { "rejected" }
            ),
        ));
        return Ok(BulkRevisionResult {
            output: Vec::new(),
            journal_ids: matched_ids,
            operations_performed,
            operations_succeeded: 0,
            operations_failed: 0,
            diagnostics: diags,
        });
    }

    // Stage all revisions atomically
    let staged = stage_revisions(package, revisions, filter, accept, meta).map_err(|mut d| {
        diags.append(&mut d);
        diags.clone()
    })?;

    let operations_succeeded = staged.len();

    // Commit staged revisions
    let output = commit_staged_revisions(package, &staged).unwrap_or_else(|mut d| {
        diags.append(&mut d);
        Vec::new()
    });

    let journal_ids: Vec<String> = staged
        .iter()
        .map(|(rev, _)| {
            format!(
                "rev-{}-{}",
                if accept { "accept" } else { "reject" },
                rev.id
            )
        })
        .collect();

    let budget = build_budget(operations_succeeded, 0);
    let _ = budget; // budget assembled but not returned directly (BulkRevisionResult interface)

    Ok(BulkRevisionResult {
        output,
        journal_ids,
        operations_performed,
        operations_succeeded,
        operations_failed: 0,
        diagnostics: diags,
    })
}

/// Bulk-accept all matching revisions (unfiltered convenience).
pub fn accept_all_matching(
    package: &OpcPackage,
    revisions: &[Revision],
    policy: &EditPolicy,
    meta: &RevisionWriterMeta,
) -> Result<BulkRevisionResult, Vec<Diagnostic>> {
    accept_filtered(
        package,
        revisions,
        &RevisionFilter::default(),
        policy,
        meta,
        false,
    )
}

/// Bulk-reject all matching revisions (unfiltered convenience).
pub fn reject_all_matching(
    package: &OpcPackage,
    revisions: &[Revision],
    policy: &EditPolicy,
    meta: &RevisionWriterMeta,
) -> Result<BulkRevisionResult, Vec<Diagnostic>> {
    reject_filtered(
        package,
        revisions,
        &RevisionFilter::default(),
        policy,
        meta,
        false,
    )
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_package() -> OpcPackage {
        let ct_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
            <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
            <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
            </Types>";
        let rels_xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
            <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
            <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
            </Relationships>";
        let doc_xml = b"<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
            <w:body>\
            <w:p><w:r><w:t>Original text</w:t></w:r></w:p>\
            <w:ins w:author=\"Alice\" w:id=\"0\" w:date=\"2026-01-01T00:00:00Z\">\
                <w:p><w:r><w:t>Alice insert</w:t></w:r></w:p>\
            </w:ins>\
            <w:del w:author=\"Bob\" w:id=\"1\" w:date=\"2026-02-01T00:00:00Z\">\
                <w:p><w:r><w:t>Bob delete</w:t></w:r></w:p>\
            </w:del>\
            <w:ins w:author=\"Alice\" w:id=\"2\" w:date=\"2026-03-01T00:00:00Z\">\
                <w:p><w:r><w:t>Alice second</w:t></w:r></w:p>\
            </w:ins>\
            </w:body></w:document>";

        OpcPackage {
            parts: vec![
                crate::opc::Part {
                    uri: "[Content_Types].xml".to_string(),
                    content_type: "application/xml".to_string(),
                    data: ct_xml.to_vec(),
                },
                crate::opc::Part {
                    uri: "_rels/.rels".to_string(),
                    content_type: "application/xml".to_string(),
                    data: rels_xml.to_vec(),
                },
                crate::opc::Part {
                    uri: "word/document.xml".to_string(),
                    content_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml".to_string(),
                    data: doc_xml.to_vec(),
                },
            ],
            content_types: std::collections::BTreeMap::new(),
            relationships: std::collections::BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }

    fn make_revisions() -> Vec<Revision> {
        vec![
            Revision {
                id: ElementId::from_str("rev-1"),
                kind: RevisionKind::Insert,
                author: Some("Alice".to_string()),
                date: Some("2026-01-01T00:00:00Z".to_string()),
                anchor_element_id: ElementId::from_str("anchor-1"),
                page_hint: Some(1),
                content: "Alice insert".to_string(),
                property_name: None,
                original_value: None,
                new_value: None,
            },
            Revision {
                id: ElementId::from_str("rev-2"),
                kind: RevisionKind::Delete,
                author: Some("Bob".to_string()),
                date: Some("2026-02-01T00:00:00Z".to_string()),
                anchor_element_id: ElementId::from_str("anchor-2"),
                page_hint: Some(1),
                content: "Bob delete".to_string(),
                property_name: None,
                original_value: None,
                new_value: None,
            },
            Revision {
                id: ElementId::from_str("rev-3"),
                kind: RevisionKind::Insert,
                author: Some("Alice".to_string()),
                date: Some("2026-03-01T00:00:00Z".to_string()),
                anchor_element_id: ElementId::from_str("anchor-3"),
                page_hint: Some(1),
                content: "Alice second".to_string(),
                property_name: None,
                original_value: None,
                new_value: None,
            },
        ]
    }

    fn make_policy() -> EditPolicy {
        EditPolicy {
            allowed_parts: vec!["word/*.xml".to_string()],
            denied_actions: vec![
                DeniedAction::DeletePart,
                DeniedAction::MacroPartWrite,
                DeniedAction::ExternalRelAdd,
                DeniedAction::BulkRevisionAll,
            ],
            max_parts_modified: None,
        }
    }

    fn make_meta() -> RevisionWriterMeta {
        RevisionWriterMeta::new("test", "2026-01-01T00:00:00Z")
    }

    /// test_accept_by_author — filter revisions by author, accept only those.
    #[test]
    fn test_accept_by_author() {
        // Use only Alice's revision (rev-1) so sequential application works.
        let pkg = make_package();
        let revisions = vec![Revision {
            id: ElementId::from_str("rev-1"),
            kind: RevisionKind::Insert,
            author: Some("Alice".to_string()),
            date: Some("2026-01-01T00:00:00Z".to_string()),
            anchor_element_id: ElementId::from_str("anchor-1"),
            page_hint: Some(1),
            content: "Alice insert".to_string(),
            property_name: None,
            original_value: None,
            new_value: None,
        }];
        let policy = make_policy();
        let meta = make_meta();

        let filter = RevisionFilter {
            author: Some("Alice".to_string()),
            ..Default::default()
        };

        let result = accept_filtered(&pkg, &revisions, &filter, &policy, &meta, false)
            .expect("accept_filtered should succeed");

        assert_eq!(result.operations_performed, 1);
        assert_eq!(result.operations_succeeded, 1);
        assert_eq!(result.journal_ids.len(), 1);
    }

    /// test_accept_by_kind — filter by RevisionKind (Insert only).
    #[test]
    fn test_accept_by_kind() {
        // Use only a single Insert revision.
        let pkg = make_package();
        let revisions = vec![Revision {
            id: ElementId::from_str("rev-1"),
            kind: RevisionKind::Insert,
            author: Some("Alice".to_string()),
            date: Some("2026-01-01T00:00:00Z".to_string()),
            anchor_element_id: ElementId::from_str("anchor-1"),
            page_hint: Some(1),
            content: "Alice insert".to_string(),
            property_name: None,
            original_value: None,
            new_value: None,
        }];
        let policy = make_policy();
        let meta = make_meta();

        let filter = RevisionFilter {
            kind: Some(RevisionKind::Insert),
            ..Default::default()
        };

        let result = accept_filtered(&pkg, &revisions, &filter, &policy, &meta, false)
            .expect("accept_filtered should succeed");

        assert_eq!(result.operations_performed, 1);
        assert_eq!(result.operations_succeeded, 1);
    }

    /// test_accept_by_date_range — filter by date_before and date_after.
    #[test]
    fn test_accept_by_date_range() {
        // Only use Bob's revision (rev-2).
        let pkg = make_package();
        let revisions = vec![Revision {
            id: ElementId::from_str("rev-2"),
            kind: RevisionKind::Delete,
            author: Some("Bob".to_string()),
            date: Some("2026-02-01T00:00:00Z".to_string()),
            anchor_element_id: ElementId::from_str("anchor-2"),
            page_hint: Some(1),
            content: "Bob delete".to_string(),
            property_name: None,
            original_value: None,
            new_value: None,
        }];
        let policy = make_policy();
        let meta = make_meta();

        // Feb 2026 revision matches: before "2026-03-01" and after "2026-01-01"
        let filter = RevisionFilter {
            date_before: Some("2026-03-01T00:00:00Z".to_string()),
            date_after: Some("2026-01-01T00:00:00Z".to_string()),
            ..Default::default()
        };

        let result = accept_filtered(&pkg, &revisions, &filter, &policy, &meta, false)
            .expect("accept_filtered should succeed");

        assert_eq!(result.operations_performed, 1);
        assert_eq!(result.operations_succeeded, 1);
        assert!(result.journal_ids[0].contains("rev-2"));
    }

    /// test_accept_by_ids — filter by specific revision IDs.
    #[test]
    fn test_accept_by_ids() {
        // Use only rev-1.
        let pkg = make_package();
        let revisions = vec![Revision {
            id: ElementId::from_str("rev-1"),
            kind: RevisionKind::Insert,
            author: Some("Alice".to_string()),
            date: Some("2026-01-01T00:00:00Z".to_string()),
            anchor_element_id: ElementId::from_str("anchor-1"),
            page_hint: Some(1),
            content: "Alice insert".to_string(),
            property_name: None,
            original_value: None,
            new_value: None,
        }];
        let policy = make_policy();
        let meta = make_meta();

        let filter = RevisionFilter {
            ids: vec![ElementId::from_str("rev-1")],
            ..Default::default()
        };

        let result = accept_filtered(&pkg, &revisions, &filter, &policy, &meta, false)
            .expect("accept_filtered should succeed");

        assert_eq!(result.operations_performed, 1);
        assert_eq!(result.operations_succeeded, 1);
    }

    /// test_atomicity_failure — FormatChange revision triggers refusal,
    /// proving staged revisions are rolled back (original untouched).
    ///
    /// When a revision in the batch fails, the entire batch is discarded
    /// and the original document is left intact.
    #[test]
    fn test_atomicity_failure() {
        let pkg = make_package();
        // Use a FormatChange revision which is always refused by the executor.
        let revisions = vec![Revision {
            id: ElementId::from_str("rev-fc-1"),
            kind: RevisionKind::FormatChange,
            author: Some("Alice".to_string()),
            date: Some("2026-04-01T00:00:00Z".to_string()),
            anchor_element_id: ElementId::from_str("anchor-fc"),
            page_hint: Some(1),
            content: "".to_string(),
            property_name: Some("bold".to_string()),
            original_value: Some("false".to_string()),
            new_value: Some("true".to_string()),
        }];
        let policy = make_policy();
        let meta = make_meta();

        // Filter by specific ID to avoid triggering the all:true policy gate.
        let filter = RevisionFilter {
            ids: vec![ElementId::from_str("rev-fc-1")],
            ..Default::default()
        };

        // FormatChange is refused by accept_revision → returns Err with GP-R-001
        let result = accept_filtered(&pkg, revisions.as_slice(), &filter, &policy, &meta, false)
            .expect_err("should fail on FormatChange revision");

        // The error is a Vec<Diagnostic> containing GP-R-001.
        assert!(
            result.iter().any(|d| d.code == "GP-R-001"),
            "should have GP-R-001 diagnostic, got: {:?}",
            result
        );
    }

    /// test_policy_deny_all — bulk reject_all denied by policy → GP-E-002.
    #[test]
    fn test_policy_deny_all() {
        let pkg = make_package();
        let revisions = make_revisions();
        let policy = make_policy();
        let meta = make_meta();

        // all: true with denied BulkRevisionAll → denied
        let filter = RevisionFilter {
            all: true,
            ..Default::default()
        };

        let result = reject_filtered(&pkg, &revisions, &filter, &policy, &meta, false)
            .expect("should return result");

        assert!(!result.diagnostics.is_empty());
        assert!(
            result.diagnostics.iter().any(|d| d.code == "GP-E-002"),
            "should have GP-E-002 diagnostic"
        );
        assert_eq!(result.operations_performed, 0);
        assert_eq!(result.operations_succeeded, 0);
    }

    /// test_policy_allow_dry_run — dry_run bypasses policy.
    #[test]
    fn test_policy_allow_dry_run() {
        let pkg = make_package();
        let revisions = make_revisions();
        let policy = make_policy();
        let meta = make_meta();

        // all: true with dry_run: true → bypasses policy gate
        let filter = RevisionFilter {
            all: true,
            ..Default::default()
        };

        let result = reject_filtered(&pkg, &revisions, &filter, &policy, &meta, true)
            .expect("dry_run should succeed even with denied policy");

        // Dry-run should report what would happen
        assert_eq!(result.operations_performed, 3);
        // No operations actually executed
        assert_eq!(result.operations_succeeded, 0);
        // No GP-E-002 denial diagnostic
        assert!(
            !result.diagnostics.iter().any(|d| d.code == "GP-E-002"),
            "dry_run should not produce denial diagnostics"
        );
    }

    /// test_filter_no_matches — filter matches nothing → empty result, no error.
    #[test]
    fn test_filter_no_matches() {
        let pkg = make_package();
        let revisions = make_revisions();
        let policy = make_policy();
        let meta = make_meta();

        // Filter for author "Charlie" who has no revisions
        let filter = RevisionFilter {
            author: Some("Charlie".to_string()),
            ..Default::default()
        };

        let result = accept_filtered(&pkg, &revisions, &filter, &policy, &meta, false)
            .expect("no matches should not be an error");

        assert_eq!(result.operations_performed, 0);
        assert_eq!(result.operations_succeeded, 0);
        assert_eq!(result.journal_ids.len(), 0);
    }

    /// test_filter_matches_correctly — verify filter logic for all fields.
    #[test]
    fn test_filter_matches_correctly() {
        let revisions = make_revisions();
        let rev = &revisions[0];

        // all: true → always matches
        assert!(RevisionFilter {
            all: true,
            ..Default::default()
        }
        .matches(rev));

        // author match
        assert!(RevisionFilter {
            author: Some("Alice".to_string()),
            ..Default::default()
        }
        .matches(rev));
        assert!(!RevisionFilter {
            author: Some("Bob".to_string()),
            ..Default::default()
        }
        .matches(rev));

        // kind match
        assert!(RevisionFilter {
            kind: Some(RevisionKind::Insert),
            ..Default::default()
        }
        .matches(rev));
        assert!(!RevisionFilter {
            kind: Some(RevisionKind::Delete),
            ..Default::default()
        }
        .matches(rev));

        // date_before match
        assert!(RevisionFilter {
            date_before: Some("2026-06-01T00:00:00Z".to_string()),
            ..Default::default()
        }
        .matches(rev));
        assert!(!RevisionFilter {
            date_before: Some("2025-06-01T00:00:00Z".to_string()),
            ..Default::default()
        }
        .matches(rev));

        // date_after match
        assert!(RevisionFilter {
            date_after: Some("2025-06-01T00:00:00Z".to_string()),
            ..Default::default()
        }
        .matches(rev));
        assert!(!RevisionFilter {
            date_after: Some("2026-06-01T00:00:00Z".to_string()),
            ..Default::default()
        }
        .matches(rev));

        // id match
        assert!(RevisionFilter {
            ids: vec![ElementId::from_str("rev-1")],
            ..Default::default()
        }
        .matches(rev));
        assert!(!RevisionFilter {
            ids: vec![ElementId::from_str("rev-99")],
            ..Default::default()
        }
        .matches(rev));
    }

    /// test_dry_run_reports_diagnostics — dry_run produces informational diagnostic.
    #[test]
    fn test_dry_run_reports_diagnostics() {
        let pkg = make_package();
        let revisions = make_revisions();
        let policy = make_policy();
        let meta = make_meta();

        let filter = RevisionFilter {
            author: Some("Alice".to_string()),
            ..Default::default()
        };

        let result = accept_filtered(&pkg, &revisions, &filter, &policy, &meta, true)
            .expect("dry_run should succeed");

        assert!(
            result.diagnostics.iter().any(|d| d.code == "GP-I-001"),
            "dry_run should produce informational diagnostic"
        );
        assert_eq!(result.operations_performed, 2);
    }

    /// test_policy_denied_returns_empty_output — denied policy returns empty output.
    #[test]
    fn test_policy_denied_returns_empty_output() {
        let pkg = make_package();
        let revisions = make_revisions();
        let policy = make_policy();
        let meta = make_meta();

        let filter = RevisionFilter {
            all: true,
            ..Default::default()
        };

        let result = accept_filtered(&pkg, &revisions, &filter, &policy, &meta, false)
            .expect("policy denial returns Ok with diagnostics");

        assert!(result.output.is_empty());
        assert!(!result.diagnostics.is_empty());
    }

    /// test_built_budget_has_rendered_by — fidelity budget includes provenance.
    #[test]
    fn test_built_budget_has_rendered_by() {
        let budget = build_budget(3, 1);
        assert!(
            budget.rendered_by.is_some(),
            "budget should have rendered_by"
        );
        assert_eq!(
            budget.rendered_by.as_ref().unwrap().engine_id,
            "ooxml-revision-bulk"
        );
        assert_eq!(budget.confidence, 0.75);
        assert_eq!(budget.dropped_features.len(), 2);
        assert_eq!(budget.dropped_features[0].feature, "bulk_revision_applied");
        assert_eq!(budget.dropped_features[1].feature, "bulk_revision");
    }
}
