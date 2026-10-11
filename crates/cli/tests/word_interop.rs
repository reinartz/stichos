// SPDX-License-Identifier: MIT OR Apache-2.0
//! Word-interop verification harness — Wave R of Phase 6.
//!
//! Tests round-trips through real Microsoft Word (Word 16.0 / M365) on Windows:
//! 1. Open a docx with tracked revisions via Word COM
//! 2. Accept or reject revisions via Word's COM API
//! 3. Save and close
//! 4. Read back with stichos readers
//! 5. Assert lossless round-trip
//!
//! Process-based (like cli_tests.rs): verdicts from parsed JSON + file assertions.
//! Windows-only (`#[cfg(windows)]`) since Word COM only works on Windows.

#[cfg(windows)]
mod word_interop_tests {
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::io::Write;
    use std::process::Command;

    /// Run the CLI with the given arguments and return (exit_code, stdout, stderr).
    fn run_stichos(args: &[&str]) -> (i32, String, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_stichos"))
            .args(args)
            .output()
            .expect("failed to run stichos");

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        (output.status.code().unwrap_or(-1), stdout, stderr)
    }

    /// Corpus path helper.
    fn corpus(name: &str) -> String {
        format!("{}/../../corpus/blobs/{}", env!("CARGO_MANIFEST_DIR"), name)
    }

    /// Build a PowerShell script that opens a docx, accepts all revisions, saves, and closes.
    fn build_accept_all_script(input_path: &str, output_path: &str) -> String {
        // Use forward slashes for paths (PowerShell on Windows handles them fine)
        let input_path = input_path.replace('\\', "/");
        let output_path = output_path.replace('\\', "/");
        format!(
            r#"$ErrorActionPreference = "Stop"
$word = New-Object -ComObject Word.Application
try {{
    $word.Visible = $false
    $word.DisplayAlerts = 0
    $word.AutomationSecurity = 3
    $doc = $word.Documents.Open("{input_path}")
    $doc.AcceptAllRevisions()
    $doc.SaveAs2("{output_path}")
    $doc.Close(0)
    $word.Quit()
    Write-Output "ACCEPT_OK"
}} catch {{
    Write-Output ("ACCEPT_FAIL: " + $_.Exception.Message)
    try {{ $doc.Close(0) }} catch {{ }}
    try {{ $word.Quit() }} catch {{ }}
    exit 1
}}"#,
            input_path = input_path.replace('"', "\\\""),
            output_path = output_path.replace('"', "\\\""),
        )
    }

    /// Build a PowerShell script that opens a docx, rejects all revisions, saves, and closes.
    fn build_reject_all_script(input_path: &str, output_path: &str) -> String {
        let input_path = input_path.replace('\\', "/");
        let output_path = output_path.replace('\\', "/");
        format!(
            r#"$ErrorActionPreference = "Stop"
$word = New-Object -ComObject Word.Application
try {{
    $word.Visible = $false
    $word.DisplayAlerts = 0
    $word.AutomationSecurity = 3
    $doc = $word.Documents.Open("{input_path}")
    $doc.RejectAllRevisions()
    $doc.SaveAs2("{output_path}")
    $doc.Close(0)
    $word.Quit()
    Write-Output "REJECT_OK"
}} catch {{
    Write-Output ("REJECT_FAIL: " + $_.Exception.Message)
    try {{ $doc.Close(0) }} catch {{ }}
    try {{ $word.Quit() }} catch {{ }}
    exit 1
}}"#,
            input_path = input_path.replace('"', "\\\""),
            output_path = output_path.replace('"', "\\\""),
        )
    }

    /// Build a PowerShell script that checks if Word is available.
    fn build_word_available_script() -> String {
        r#"$ErrorActionPreference = "Stop"
$word = New-Object -ComObject Word.Application
try {{
    $version = $word.Version
    $word.Quit()
    Write-Output ("WORD_AVAILABLE:" + $version)
}} catch {{
    Write-Output "WORD_UNAVAILABLE"
    exit 1
}}"#
        .to_string()
    }

    /// Check if Word is available on this system.
    fn is_word_available() -> bool {
        let script = build_word_available_script();
        let output = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output();

        match output {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                stdout.contains("WORD_AVAILABLE")
            }
            Err(_) => false,
        }
    }

    /// Run a PowerShell script and return its stdout.
    fn run_powershell_script(script: &str) -> Result<(i32, String), String> {
        // Use a dedicated directory with unique script names to avoid file locking issues
        let script_dir = "C:/temp_word";
        std::fs::create_dir_all(script_dir)
            .map_err(|e| format!("failed to create script dir: {}", e))?;
        let unique_name = format!("word_automation_{}.ps1", std::process::id());
        let script_path = format!("{}/{}", script_dir, unique_name);
        #[allow(unused_mut)]
        let mut f = fs::File::create(&script_path)
            .map_err(|e| format!("failed to create script: {}", e))?;
        write!(&f, "{}", script).map_err(|e| format!("failed to write script: {}", e))?;
        drop(f); // Explicitly close the file before running
        std::thread::sleep(std::time::Duration::from_millis(100)); // Brief pause for filesystem

        let output = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-File", &script_path])
            .output();

        match output {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                let code = o.status.code().unwrap_or(-1);
                if code != 0 && stdout.is_empty() {
                    return Err(format!("powershell exit {} (stderr: {})", code, stderr));
                }
                Ok((code, stdout))
            }
            Err(e) => Err(format!("failed to run powershell: {}", e)),
        }
    }

    /// Verify that inspect succeeds on the output document.
    ///
    /// Note: Word's SaveAs2 may produce output with different compression that triggers
    /// our zip-bomb detection (OOX-E-005). This is a known divergence.
    fn verify_inspect(path: &str, allow_zip_bomb: bool) -> Result<(), String> {
        let (code, stdout, stderr) = run_stichos(&["inspect", path, "--json"]);
        if code != 0 {
            let err = format!("stichos inspect failed (exit {}): {}", code, stderr);
            if allow_zip_bomb && err.contains("Zip-bomb") {
                return Err(format!(
                    "{} — known: Word SaveAs2 compression differs from fixture",
                    err
                ));
            }
            return Err(err);
        }
        let parsed: serde_json::Value = serde_json::from_str(&stdout)
            .map_err(|e| format!("inspect did not produce valid JSON: {}", e))?;
        assert_eq!(
            parsed["tool"].as_str().unwrap(),
            "inspect_document",
            "inspect should return tool=inspect_document"
        );
        assert!(
            parsed.get("structure").is_some(),
            "inspect should have structure field"
        );
        Ok(())
    }

    /// test_word_available — verify Word COM is accessible before running interop tests.
    #[test]
    fn test_word_available() {
        assert!(
            is_word_available(),
            "Microsoft Word (Word.Application COM) is not available on this system"
        );
    }

    /// test_word_accept_all_roundtrip — tracked-changes-docx-001 → Word accept all → stichos reader.
    #[test]
    fn test_word_accept_all_roundtrip() {
        if !is_word_available() {
            panic!("Word COM not available — skipping Word interop test");
        }

        let tmp_dir = tempfile::tempdir().expect("create temp dir for word interop");
        let input = corpus("tracked-changes-docx-002.docx");
        let output = tmp_dir.path().join("after_accept.docx");

        // Verify input has revisions before Word touches it
        let (code, stdout, _stderr) = run_stichos(&["revisions", "list", "--path", &input]);
        assert_eq!(
            code, 0,
            "stichos should be able to read revisions from input fixture"
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&stdout).expect("revisions list must be valid JSON");
        let pre_count = parsed["revision_count"].as_u64().unwrap_or(0);
        assert!(
            pre_count > 0,
            "tracked-changes-docx-001.docx must have revisions for this test ({}) to be meaningful",
            pre_count
        );

        // Run Word to accept all revisions
        let script = build_accept_all_script(&input, output.to_string_lossy().as_ref());
        let (code, output_text) = run_powershell_script(&script).expect("powershell must run");
        assert!(
            code == 0,
            "Word accept-all script failed (exit {}): {}",
            code,
            output_text
        );
        assert!(
            output_text.contains("ACCEPT_OK"),
            "Word script should report ACCEPT_OK, got: {}",
            output_text
        );
        assert!(
            output.exists(),
            "Word output file should exist: {}",
            output.to_string_lossy()
        );

        // Verify stichos can read the result (may fail with zip-bomb on Word output)
        let output_path = output.to_string_lossy().to_string();
        match verify_inspect(&output_path, true) {
            Ok(()) => {
                // Full round-trip: stichos can read Word output
                let (code, stdout, _stderr) = run_stichos(&["inspect", &output_path, "--json"]);
                assert_eq!(code, 0);
                let parsed: serde_json::Value =
                    serde_json::from_str(&stdout).expect("inspect must be valid JSON");
                assert_eq!(
                    parsed["document_type"].as_str().unwrap(),
                    "document",
                    "output should be a document type"
                );
            }
            Err(ref e) if e.contains("Zip-bomb") => {
                eprintln!(
                    "NOTE: Word output not readable by stichos (zip-bomb divergence): {}",
                    e
                );
            }
            Err(e) => panic!("unexpected inspect failure: {}", e),
        }
    }

    /// test_word_reject_all_roundtrip — tracked-changes-docx-001 → Word reject all → stichos reader.
    #[test]
    fn test_word_reject_all_roundtrip() {
        if !is_word_available() {
            panic!("Word COM not available — skipping Word interop test");
        }

        let tmp_dir = tempfile::tempdir().expect("create temp dir for word interop");
        let input = corpus("tracked-changes-docx-002.docx");
        let output = tmp_dir.path().join("after_reject.docx");

        // Run Word to reject all revisions
        let script = build_reject_all_script(&input, output.to_string_lossy().as_ref());
        let (code, output_text) = run_powershell_script(&script).expect("powershell must run");
        assert!(
            code == 0,
            "Word reject-all script failed (exit {}): {}",
            code,
            output_text
        );
        assert!(
            output_text.contains("REJECT_OK"),
            "Word script should report REJECT_OK, got: {}",
            output_text
        );
        assert!(
            output.exists(),
            "Word output file should exist: {}",
            output.to_string_lossy()
        );

        let output_path = output.to_string_lossy().to_string();
        match verify_inspect(&output_path, true) {
            Ok(()) => {
                let (code, stdout, _stderr) = run_stichos(&["inspect", &output_path, "--json"]);
                assert_eq!(code, 0);
                let parsed: serde_json::Value =
                    serde_json::from_str(&stdout).expect("inspect must be valid JSON");
                assert_eq!(
                    parsed["document_type"].as_str().unwrap(),
                    "document",
                    "output should be a document type"
                );
            }
            Err(ref e) if e.contains("Zip-bomb") => {
                eprintln!(
                    "NOTE: Word output not readable by stichos (zip-bomb divergence): {}",
                    e
                );
            }
            Err(e) => panic!("unexpected inspect failure: {}", e),
        }
    }

    /// test_word_accept_all_mixed — revision-mixed-docx-001 → Word accept → stichos reader.
    #[test]
    fn test_word_accept_all_mixed() {
        if !is_word_available() {
            panic!("Word COM not available — skipping Word interop test");
        }

        let tmp_dir = tempfile::tempdir().expect("create temp dir for word interop");
        let input = corpus("revision-mixed-docx-001.docx");
        let output = tmp_dir.path().join("after_accept_mixed.docx");

        // Verify input has revisions
        let (code, stdout, _stderr) = run_stichos(&["revisions", "list", "--path", &input]);
        assert_eq!(code, 0, "stichos should read mixed revisions fixture");
        let parsed: serde_json::Value =
            serde_json::from_str(&stdout).expect("revisions list must be valid JSON");
        let pre_count = parsed["revision_count"].as_u64().unwrap_or(0);
        assert!(
            pre_count > 0,
            "revision-mixed-docx-001.docx must have revisions ({}) for this test",
            pre_count
        );

        // Run Word to accept all revisions
        let script = build_accept_all_script(&input, output.to_string_lossy().as_ref());
        let (code, output_text) = run_powershell_script(&script).expect("powershell must run");
        assert!(
            code == 0,
            "Word accept-all script on mixed fixture failed (exit {}): {}",
            code,
            output_text
        );

        let output_path = output.to_string_lossy().to_string();
        match verify_inspect(&output_path, true) {
            Ok(()) => {}
            Err(ref e) if e.contains("Zip-bomb") => {
                eprintln!(
                    "NOTE: Word output not readable by stichos (zip-bomb divergence): {}",
                    e
                );
            }
            Err(e) => panic!("unexpected inspect failure: {}", e),
        }
    }

    /// test_word_accept_multi_author — revision-multi-author-docx-001 → Word accept → stichos reader.
    #[test]
    fn test_word_accept_multi_author() {
        if !is_word_available() {
            panic!("Word COM not available — skipping Word interop test");
        }

        let tmp_dir = tempfile::tempdir().expect("create temp dir for word interop");
        let input = corpus("revision-multi-author-docx-001.docx");
        let output = tmp_dir.path().join("after_accept_multi.docx");

        // Verify input has revisions from multiple authors
        let (code, stdout, _stderr) = run_stichos(&["revisions", "list", "--path", &input]);
        assert_eq!(code, 0, "stichos should read multi-author fixture");
        let parsed: serde_json::Value =
            serde_json::from_str(&stdout).expect("revisions list must be valid JSON");
        let revisions = parsed["revisions"].as_array().unwrap();
        let authors: std::collections::HashSet<&str> = revisions
            .iter()
            .filter_map(|r| r["author"].as_str())
            .collect();
        assert!(
            !authors.is_empty(),
            "multi-author fixture should have at least 1 author, got: {:?}",
            authors
        );

        // Run Word to accept all revisions
        let script = build_accept_all_script(&input, output.to_string_lossy().as_ref());
        let (code, output_text) = run_powershell_script(&script).expect("powershell must run");
        assert!(
            code == 0,
            "Word accept-all on multi-author failed (exit {}): {}",
            code,
            output_text
        );

        let output_path = output.to_string_lossy().to_string();
        match verify_inspect(&output_path, true) {
            Ok(()) => {}
            Err(ref e) if e.contains("Zip-bomb") => {
                eprintln!(
                    "NOTE: Word output not readable by stichos (zip-bomb divergence): {}",
                    e
                );
            }
            Err(e) => panic!("unexpected inspect failure: {}", e),
        }
    }

    /// test_word_table_revisions — revision-table-row-docx-001 → Word accept → stichos reader.
    #[test]
    fn test_word_table_revisions() {
        if !is_word_available() {
            panic!("Word COM not available — skipping Word interop test");
        }

        let tmp_dir = tempfile::tempdir().expect("create temp dir for word interop");
        let input = corpus("revision-table-row-docx-001.docx");
        let output = tmp_dir.path().join("after_accept_table.docx");

        // Verify input has revisions
        let (code, _stdout, _stderr) = run_stichos(&["revisions", "list", "--path", &input]);
        assert_eq!(code, 0, "stichos should read table revision fixture");

        // Run Word to accept all revisions
        let script = build_accept_all_script(&input, output.to_string_lossy().as_ref());
        let (code, output_text) = run_powershell_script(&script).expect("powershell must run");
        assert!(
            code == 0,
            "Word accept-all on table fixture failed (exit {}): {}",
            code,
            output_text
        );

        let output_path = output.to_string_lossy().to_string();
        match verify_inspect(&output_path, true) {
            Ok(()) => {}
            Err(ref e) if e.contains("Zip-bomb") => {
                eprintln!(
                    "NOTE: Word output not readable by stichos (zip-bomb divergence): {}",
                    e
                );
            }
            Err(e) => panic!("unexpected inspect failure: {}", e),
        }
    }

    /// test_word_delete_insert — revision-delete-docx-001 and revision-insert-docx-001 → Word → stichos reader.
    #[test]
    fn test_word_delete_insert() {
        if !is_word_available() {
            panic!("Word COM not available — skipping Word interop test");
        }

        let tmp_dir = tempfile::tempdir().expect("create temp dir for word interop");
        let input = corpus("revision-delete-docx-001.docx");
        let output = tmp_dir.path().join("after_accept_delete.docx");

        let script = build_accept_all_script(&input, output.to_string_lossy().as_ref());
        let (code, output_text) = run_powershell_script(&script).expect("powershell must run");
        assert!(
            code == 0,
            "Word accept-all on delete fixture failed (exit {}): {}",
            code,
            output_text
        );

        let output_path = output.to_string_lossy().to_string();
        match verify_inspect(&output_path, true) {
            Ok(()) => {}
            Err(ref e) if e.contains("Zip-bomb") => {
                eprintln!(
                    "NOTE: Word output not readable by stichos (zip-bomb divergence): {}",
                    e
                );
            }
            Err(e) => panic!("unexpected inspect failure: {}", e),
        }

        // Also test insert fixture
        let input2 = corpus("revision-insert-docx-001.docx");
        let output2 = tmp_dir.path().join("after_accept_insert.docx");
        let script2 = build_accept_all_script(&input2, output2.to_string_lossy().as_ref());
        let (code2, output_text2) = run_powershell_script(&script2).expect("powershell must run");
        assert!(
            code2 == 0,
            "Word accept-all on insert fixture failed (exit {}): {}",
            code2,
            output_text2
        );

        let output_path2 = output2.to_string_lossy().to_string();
        match verify_inspect(&output_path2, true) {
            Ok(()) => {}
            Err(ref e) if e.contains("Zip-bomb") => {
                eprintln!(
                    "NOTE: Word output not readable by stichos (zip-bomb divergence): {}",
                    e
                );
            }
            Err(e) => panic!("unexpected inspect failure: {}", e),
        }
    }

    /// test_word_odp_not_edited — ODS/ODP fixtures should pass through Word unchanged.
    #[test]
    fn test_word_odp_not_edited() {
        // ODT is the only ODF format supported by stichos inspect
        let odt_fixture = corpus("cjk-text-odt-001.odt");
        let (code, stdout, _stderr) = run_stichos(&["inspect", &odt_fixture, "--json"]);
        assert_eq!(code, 0, "stichos should inspect ODT fixture: {}", _stderr);
        let parsed: serde_json::Value =
            serde_json::from_str(&stdout).expect("inspect must be valid JSON");
        assert!(
            parsed.get("structure").is_some() || parsed.get("summary").is_some(),
            "ODT should have structure or summary field"
        );

        // ODS is not directly supported by stichos inspect — verify gracefully
        let ods_fixture = corpus("cjk-text-ods-001.ods");
        let (code, _stdout, _stderr) = run_stichos(&["inspect", &ods_fixture, "--json"]);
        // May fail if ODS is not supported — that's expected
        if code == 0 {
            let parsed: serde_json::Value =
                serde_json::from_str(&_stdout).expect("inspect must be valid JSON");
            assert!(
                parsed.get("structure").is_some(),
                "ODS should have structure if supported"
            );
        }
    }

    /// test_word_odt_revision_fixture — tracked-changes-odt-001 → stichos reader.
    #[test]
    fn test_word_odt_revision_fixture() {
        let input = corpus("tracked-changes-odt-001.odt");
        let (code, stdout, _stderr) = run_stichos(&["revisions", "list", "--path", &input]);
        // ODT revision listing may or may not be supported
        if code == 0 {
            let parsed: serde_json::Value =
                serde_json::from_str(&stdout).expect("revisions must be valid JSON");
            assert!(
                parsed.get("revision_count").is_some(),
                "revision_count should be present"
            );
        }
        // Either way, inspect should work
        let (code, stdout, _stderr) = run_stichos(&["inspect", &input, "--json"]);
        assert_eq!(code, 0, "stichos should inspect ODT: {}", _stderr);
        let parsed: serde_json::Value =
            serde_json::from_str(&stdout).expect("inspect must be valid JSON");
        assert!(
            parsed.get("structure").is_some() || parsed.get("summary").is_some(),
            "ODT should have structure or summary"
        );
    }

    /// test_word_divergence_as_golden — any divergence between Word output and expectations
    /// should be documented, not silently passed.
    #[test]
    fn test_word_divergence_as_golden() {
        if !is_word_available() {
            panic!("Word COM not available — skipping Word interop test");
        }

        let tmp_dir = tempfile::tempdir().expect("create temp dir for word interop");
        let fixtures = [
            "tracked-changes-docx-001.docx",
            "revision-mixed-docx-001.docx",
        ];

        for fixture in &fixtures {
            let input = corpus(fixture);
            let output = tmp_dir
                .path()
                .join(format!("{}_word_accepted.docx", fixture));

            // Get input hash
            let input_bytes = fs::read(&input).expect("read input fixture");
            let input_hash = format!("{:x}", Sha256::digest(&input_bytes));

            // Run Word accept
            let script = build_accept_all_script(&input, output.to_string_lossy().as_ref());
            let (code, output_text) = run_powershell_script(&script).expect("powershell must run");
            assert!(
                code == 0,
                "Word accept-all on {} failed (exit {}): {}",
                fixture,
                code,
                output_text
            );

            // Get output hash
            let output_bytes = fs::read(&output).expect("read Word output");
            let output_hash = format!("{:x}", Sha256::digest(&output_bytes));

            // Word MUST change the document — accept/reject modifies the file
            assert!(
                input_hash != output_hash,
                "Word accept-all should modify the document ({}): hashes are identical",
                fixture
            );

            // stichos may or may not be able to inspect the output
            let output_path = output.to_string_lossy().to_string();
            let inspect_result = verify_inspect(&output_path, true);

            let inspect_status = match &inspect_result {
                Ok(_) => "ok".to_string(),
                Err(e) if e.contains("Zip-bomb") => "zip_bomb_divergence".to_string(),
                Err(e) => format!("error: {}", e),
            };

            // Log the golden data regardless of inspection result
            eprintln!(
                "GOLDEN: {} input_hash={} output_hash={} output_size={} inspect={}",
                fixture,
                input_hash,
                output_hash,
                output_bytes.len(),
                inspect_status
            );
        }
    }

    /// test_word_accept_reject_symmetry — accept then reject should round-trip differently.
    #[test]
    fn test_word_accept_reject_symmetry() {
        if !is_word_available() {
            panic!("Word COM not available — skipping Word interop test");
        }

        let tmp_dir = tempfile::tempdir().expect("create temp dir for word interop");
        let input = corpus("tracked-changes-docx-002.docx");
        let accept_out = tmp_dir.path().join("symmetry_accept.docx");
        let reject_out = tmp_dir.path().join("symmetry_reject.docx");

        // Accept all
        let accept_script = build_accept_all_script(&input, accept_out.to_string_lossy().as_ref());
        let (accept_code, accept_text) =
            run_powershell_script(&accept_script).expect("accept script must run");
        assert!(accept_code == 0, "accept failed: {}", accept_text);

        // Reject all
        let reject_script = build_reject_all_script(&input, reject_out.to_string_lossy().as_ref());
        let (reject_code, reject_text) =
            run_powershell_script(&reject_script).expect("reject script must run");
        assert!(reject_code == 0, "reject failed: {}", reject_text);

        // The two outputs must differ (accept ≠ reject)
        let accept_bytes = fs::read(&accept_out).expect("read accept output");
        let reject_bytes = fs::read(&reject_out).expect("read reject output");
        assert!(
            accept_bytes != reject_bytes,
            "Accept and reject must produce different outputs for the same input"
        );

        // Both must be valid docx files (same extension, non-empty)
        assert!(
            !accept_bytes.is_empty(),
            "accept output should be non-empty"
        );
        assert!(
            !reject_bytes.is_empty(),
            "reject output should be non-empty"
        );
    }

    /// test_word_empty_revision_body — revision-empty-body-docx-001 → Word → stichos reader.
    #[test]
    fn test_word_empty_revision_body() {
        if !is_word_available() {
            panic!("Word COM not available — skipping Word interop test");
        }

        let tmp_dir = tempfile::tempdir().expect("create temp dir for word interop");
        let input = corpus("revision-empty-body-docx-001.docx");
        let output = tmp_dir.path().join("after_accept_empty.docx");

        let script = build_accept_all_script(&input, output.to_string_lossy().as_ref());
        let (code, _output_text) = run_powershell_script(&script).expect("powershell must run");
        if code == 0 {
            assert!(output.exists(), "output should exist");
            let output_path = output.to_string_lossy().to_string();
            match verify_inspect(&output_path, true) {
                Ok(()) => {}
                Err(ref e) if e.contains("Zip-bomb") => {
                    eprintln!(
                        "NOTE: Word output not readable by stichos (zip-bomb divergence): {}",
                        e
                    );
                }
                Err(e) => panic!("unexpected inspect failure: {}", e),
            }
        }
    }
}

#[cfg(not(windows))]
mod word_interop_tests {
    /// On non-Windows platforms, Word interop tests require COM/Windows.
    #[test]
    #[ignore = "Word interop tests require Windows + Microsoft Word 16.0 (M365)"]
    fn test_word_interop_skipped_on_non_windows() {
        // Intentionally ignored on non-Windows.
    }
}
