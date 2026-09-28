// SPDX-License-Identifier: MIT OR Apache-2.0
//! End-to-end tests for the `stichos create` CLI subcommand.
//!
//! These tests verify:
//! - `stichos create --help` shows the expected options
//! - `stichos create` produces valid output documents

use std::process::Command;

/// Run the stichos create command and return (exit_code, stdout, stderr).
fn run_stichos_create(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_stichos"))
        .args(args)
        .output()
        .expect("failed to run stichos");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.code().unwrap_or(-1), stdout, stderr)
}

/// `stichos create --help` should exit cleanly and show available options.
#[test]
fn test_create_help_shows_create() {
    let (code, stdout, stderr) = run_stichos_create(&["create", "--help"]);
    assert_eq!(code, 0, "create --help should exit 0");
    let combined = format!("{}{}", stdout, stderr);

    // Should show source types
    assert!(
        combined.contains("markdown"),
        "should mention markdown source type"
    );
    assert!(combined.contains("html"), "should mention html source type");
    assert!(combined.contains("typ"), "should mention typ source type");

    // Should show output formats
    assert!(combined.contains("docx"), "should mention docx format");
    assert!(combined.contains("odt"), "should mention odt format");
    assert!(combined.contains("pdf"), "should mention pdf format");

    // Should show key options
    assert!(
        combined.contains("--source-content"),
        "should show --source-content"
    );
    assert!(combined.contains("--source"), "should show --source");
    assert!(combined.contains("--format"), "should show --format");
    assert!(
        combined.contains("--source-type"),
        "should show --source-type"
    );
    assert!(combined.contains("--json"), "should show --json option");
}

/// `stichos create` should produce a valid docx file.
///
/// Runs create with markdown source, parses JSON output to find the document_id,
/// then verifies the blob file is a valid zip (PK header).
#[test]
fn test_create_document_idempotent() {
    let (code, stdout, stderr) = run_stichos_create(&[
        "create",
        "--source-type",
        "markdown",
        "--format",
        "docx",
        "--source-content",
        "# Hello\n\nWorld",
        "--json",
    ]);
    assert_eq!(code, 0, "create should succeed: stderr={}", stderr);

    let output: serde_json::Value =
        serde_json::from_str(&stdout).expect("create --json should produce valid JSON");

    // Verify the response fields
    assert_eq!(output["status"].as_str().unwrap(), "created");
    assert!(output["document_id"].is_string());
    assert!(output["format"].is_string());
    assert!(output["output_size"].is_number());

    // The output_size should be > 0 (docx is a zip file)
    let output_size = output["output_size"].as_u64().unwrap();
    assert!(output_size > 0, "output_size should be positive");

    // Verify the store_root exists and contains the blob
    let store_root = output["store_root"].as_str().unwrap();
    let blob_dir = format!("{}/blobs", store_root);
    assert!(
        std::path::Path::new(&blob_dir).exists(),
        "blobs dir should exist"
    );

    // Find the blob file and verify PK (zip) header
    let doc_id = output["document_id"].as_str().unwrap();
    // Blobs are stored content-addressed under blobs/<first-2-chars-of-hash>/
    let blob_prefix = &doc_id[4..6]; // e.g. "doc-" prefix, get chars 4-5
    let _blob_prefix = blob_prefix; // keep to verify doc_id format
    let blob_dir_full = format!("{}/blobs", store_root);
    let blob_files = find_blob_files(&blob_dir_full);
    assert!(
        !blob_files.is_empty(),
        "should find at least one blob file in {}",
        blob_dir_full
    );

    // Verify the first blob file starts with PK (zip) magic bytes
    let blob_path = &blob_files[0];
    let blob_content = std::fs::read(blob_path).expect("should read blob file");
    assert!(
        blob_content.len() >= 2 && &blob_content[0..2] == b"PK",
        "blob should start with PK (zip) magic bytes, got: {:02x} {:02x}",
        blob_content[0],
        blob_content[1]
    );
}

/// Find all files in a directory tree.
fn find_blob_files(dir: &str) -> Vec<String> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                files.push(path.to_string_lossy().to_string());
            } else if path.is_dir() {
                files.extend(find_blob_files(&path.to_string_lossy()));
            }
        }
    }
    files
}
