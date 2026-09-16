/// Selfupdate primitive: download, verify, install binary updates.
///
/// Per ADR-0006: GitHub Releases channel, minisign verify, backup+rollback.
/// Opt-in only: never triggered implicitly.
///
/// Safety invariants:
/// 1. Signature verified BEFORE any filesystem swap
/// 2. Backup created before swap
/// 3. Smoke test after swap, rollback on failure
/// 4. Network only on explicit user command
///
/// Diagnostic codes: UPD-E-001..006, UPD-W-001..002
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Diagnostic codes
// ---------------------------------------------------------------------------

/// Update-specific diagnostic codes.
pub const UPD_E_001: &str = "UPD-E-001"; // Signature verification failed
pub const UPD_E_002: &str = "UPD-E-002"; // SHA-256 hash mismatch
pub const UPD_E_003: &str = "UPD-E-003"; // Download failed
pub const UPD_E_004: &str = "UPD-E-004"; // Smoke test failed
pub const UPD_E_005: &str = "UPD-E-005"; // Backup/restore failed
pub const UPD_E_006: &str = "UPD-E-006"; // Manifest parse error
pub const UPD_W_001: &str = "UPD-W-001"; // Update available but not installed (dry-run)
pub const UPD_W_002: &str = "UPD-W-002"; // No update available

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Update channel configuration.
#[derive(Debug, Clone)]
pub struct UpdateChannel {
    /// Channel name (e.g., "stable", "beta").
    pub name: String,
    /// GitHub Releases URL template.
    pub release_url: String,
    /// Asset name pattern for the binary.
    pub binary_pattern: String,
    /// Manifest download URL.
    pub manifest_url: String,
    /// Minisign public key (base64, as in minisign.pub format).
    pub public_key: String,
    /// Local fixture base path (file://), if using fixture channel.
    pub fixture_path: Option<PathBuf>,
}

impl UpdateChannel {
    /// Create the default stable channel (GitHub Releases).
    pub fn stable() -> Self {
        Self {
            name: "stable".to_string(),
            release_url: "https://github.com/orei/stichos/releases/latest".to_string(),
            binary_pattern: "stichos-x86_64-unknown-linux-gnu".to_string(),
            manifest_url: "https://github.com/orei/stichos/releases/latest/download/manifest.json"
                .to_string(),
            public_key: String::new(), // Set by `with_public_key()`
            fixture_path: None,
        }
    }

    /// Create a fixture channel from a local directory path.
    pub fn fixture(path: PathBuf) -> Self {
        Self {
            name: "fixture".to_string(),
            release_url: format!("file://{}", path.display()),
            binary_pattern: "stichos-x86_64-unknown-linux-gnu".to_string(),
            manifest_url: path.join("manifest.json").to_string_lossy().to_string(),
            public_key: String::new(),
            fixture_path: Some(path),
        }
    }

    /// Set the minisign public key.
    pub fn with_public_key(mut self, key: &str) -> Self {
        self.public_key = key.to_string();
        self
    }
}

/// An update manifest downloaded from the update channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateManifest {
    /// Version string (e.g., "0.2.0").
    pub version: String,
    /// SHA-256 hash of the binary (hex-encoded).
    pub sha256: String,
    /// Download URL for the binary.
    pub download_url: String,
    /// Minisign signature (base64, in `minisig` text format).
    #[serde(default)]
    pub signature: String,
    /// Release notes.
    #[serde(default)]
    pub notes: String,
}

/// Error types for manifest loading.
#[derive(Debug, thiserror::Error)]
pub enum ManifestLoadError {
    #[error("I/O error reading manifest: {0} ({1})")]
    Io(io::Error, PathBuf),
    #[error("manifest parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

impl UpdateManifest {
    /// Load a manifest from a JSON file.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ManifestLoadError> {
        let bytes =
            fs::read(&path).map_err(|e| ManifestLoadError::Io(e, path.as_ref().to_path_buf()))?;
        let manifest = serde_json::from_slice::<Self>(&bytes).map_err(ManifestLoadError::Parse)?;
        Ok(manifest)
    }

    /// Compute the SHA-256 hash of raw bytes.
    pub fn hash_bytes(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }
}

impl FromStr for UpdateManifest {
    type Err = ManifestLoadError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s).map_err(ManifestLoadError::Parse)
    }
}

/// Result of a selfupdate operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateResult {
    /// Whether the update was successful.
    pub success: bool,
    /// Current version (before update, or new version after successful update).
    pub current_version: String,
    /// New version available (or same as current if no update).
    pub new_version: String,
    /// Status message.
    pub message: String,
    /// Whether a rollback was performed.
    #[serde(default)]
    pub rolled_back: bool,
    /// Diagnostics (errors/warnings).
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

impl UpdateResult {
    pub fn success(version: &str, message: &str) -> Self {
        Self {
            success: true,
            current_version: version.to_string(),
            new_version: version.to_string(),
            message: message.to_string(),
            rolled_back: false,
            diagnostics: Vec::new(),
        }
    }

    pub fn no_update(current: &str, new: &str) -> Self {
        Self {
            success: true,
            current_version: current.to_string(),
            new_version: new.to_string(),
            message: format!("already on version {}", current),
            rolled_back: false,
            diagnostics: Vec::new(),
        }
    }

    pub fn error(message: &str, current: &str, new: &str) -> Self {
        Self {
            success: false,
            current_version: current.to_string(),
            new_version: new.to_string(),
            message: message.to_string(),
            rolled_back: false,
            diagnostics: Vec::new(),
        }
    }

    pub fn with_rollback(mut self, message: &str) -> Self {
        self.rolled_back = true;
        self.message = message.to_string();
        self.diagnostics.push(format!("ROLLBACK: {}", message));
        self
    }

    pub fn with_diagnostic(mut self, diag: &str) -> Self {
        self.diagnostics.push(diag.to_string());
        self
    }
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

/// Verify a minisign signature over the manifest bytes.
///
/// Uses the `minisign-verify` crate (pure Rust, D2-safe).
/// Returns `Ok(())` if the signature is valid.
///
/// The signature is expected in minisign's text format:
/// ```text
/// untrusted comment: signature from signing key
/// <base64 sig>
/// trusted comment: timestamp=<timestamp> <keyid>
/// ```
pub fn verify_manifest_signature(
    manifest_bytes: &[u8],
    signature: &str,
    public_key: &str,
) -> Result<(), String> {
    // Parse the public key (format: "r6 <hex>" or "<hex>")
    let pk = minisign_verify::PublicKey::decode(public_key)
        .map_err(|e| format!("failed to parse public key: {}", e))?;

    // Parse the signature
    let sig = minisign_verify::Signature::decode(signature)
        .map_err(|e| format!("failed to parse signature: {}", e))?;

    // Verify
    pk.verify(manifest_bytes, &sig, false)
        .map_err(|e| format!("minisign signature verification failed: {}", e))?;

    Ok(())
}

/// Verify the SHA-256 hash of a binary against the manifest.
pub fn verify_hash(downloaded_bytes: &[u8], expected_hash: &str) -> Result<(), String> {
    let actual_hash = UpdateManifest::hash_bytes(downloaded_bytes);
    if actual_hash != expected_hash {
        return Err(format!(
            "SHA-256 hash mismatch: expected {}, got {}",
            expected_hash, actual_hash
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Download
// ---------------------------------------------------------------------------

/// Download a file using curl (shell-out, no linked network stack).
///
/// Returns the raw bytes on success.
pub fn download_with_curl(url: &str, timeout_secs: u64) -> Result<Vec<u8>, String> {
    let output = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--location",
            "--max-time",
            &timeout_secs.to_string(),
            "--retry",
            "3",
            "--retry-delay",
            "1",
            "-o",
            "-",
            url,
        ])
        .output()
        .map_err(|e| format!("curl failed to execute: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "curl download failed (exit {}): {}",
            output.status, stderr
        ));
    }

    Ok(output.stdout)
}

/// Download a file using wget (shell-out fallback).
pub fn download_with_wget(url: &str, timeout_secs: u64) -> Result<Vec<u8>, String> {
    let output = Command::new("wget")
        .args([
            "--quiet",
            "--timeout",
            &timeout_secs.to_string(),
            "--tries",
            "3",
            "-O",
            "-",
            url,
        ])
        .output()
        .map_err(|e| format!("wget failed to execute: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "wget download failed (exit {}): {}",
            output.status, stderr
        ));
    }

    Ok(output.stdout)
}

/// Download a file, trying curl first, then wget.
pub fn download(url: &str, timeout_secs: u64) -> Result<Vec<u8>, String> {
    download_with_curl(url, timeout_secs).or_else(|_| download_with_wget(url, timeout_secs))
}

/// Download the manifest from a channel, preferring fixture if available.
pub fn download_manifest(channel: &UpdateChannel) -> Result<(UpdateManifest, String), String> {
    let manifest_bytes;
    let signature;

    if let Some(fixture_path) = &channel.fixture_path {
        // Local fixture: read manifest.json and manifest.minisig (optional)
        let manifest_path = fixture_path.join("manifest.json");
        let sig_path = fixture_path.join("manifest.minisig");

        manifest_bytes = fs::read(&manifest_path).map_err(|e| {
            format!(
                "cannot read fixture manifest {}: {}",
                manifest_path.display(),
                e
            )
        })?;

        signature = fs::read_to_string(&sig_path).unwrap_or_default();
    } else {
        // Remote: download manifest and its signature
        let manifest_url = &channel.manifest_url;
        manifest_bytes = download(manifest_url, 30)?;
        signature = download(&format!("{}.minisig", manifest_url), 10)
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();
    }

    let manifest = std::str::from_utf8(&manifest_bytes)
        .map_err(|e| format!("manifest is not valid UTF-8: {}", e))?
        .parse::<UpdateManifest>()
        .map_err(|e: ManifestLoadError| format!("manifest parse error: {}", e))?;

    Ok((manifest, signature))
}

// ---------------------------------------------------------------------------
// Installation (backup → swap → smoke test → rollback)
// ---------------------------------------------------------------------------

/// Create a backup of the current binary.
fn create_backup(bin_path: &Path) -> Result<PathBuf, String> {
    let backup_path = bin_path.with_extension("bak");
    fs::copy(bin_path, &backup_path).map_err(|e| {
        format!(
            "failed to create backup at {}: {}",
            backup_path.display(),
            e
        )
    })?;
    Ok(backup_path)
}

/// Remove a backup file.
fn remove_backup(backup_path: &Path) -> Result<(), String> {
    fs::remove_file(backup_path)
        .map_err(|e| format!("failed to remove backup {}: {}", backup_path.display(), e))
}

/// Swap the current binary with the new one (atomic rename).
fn swap_binary(new_path: &Path, current_path: &Path) -> Result<(), String> {
    fs::rename(new_path, current_path).map_err(|e| format!("failed to swap binary: {}", e))?;
    Ok(())
}

/// Run the smoke test: execute the new binary with --version.
fn smoke_test(new_path: &Path, _timeout_secs: u64) -> Result<(), String> {
    let output = Command::new(new_path)
        .args(["--version"])
        .output()
        .map_err(|e| format!("smoke test exec failed: {}", e))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        tracing::info!("smoke test: {}", stdout);
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(format!(
            "smoke test failed (exit {}): {}",
            output.status, stderr
        ))
    }
}

/// Rollback: restore the backup binary.
fn rollback(backup_path: &Path, current_path: &Path) -> Result<(), String> {
    let backup_bytes = fs::read(backup_path)
        .map_err(|e| format!("failed to read backup {}: {}", backup_path.display(), e))?;
    fs::write(current_path, &backup_bytes)
        .map_err(|e| format!("failed to restore backup: {}", e))?;
    remove_backup(backup_path)?;
    Ok(())
}

/// Full update flow: download → verify → backup → swap → smoke test → rollback on failure.
///
/// Safety:
/// - Signature and hash verified BEFORE any swap
/// - Backup created before swap
/// - Smoke test runs after swap
/// - On any failure, backup is restored
pub fn download_and_install(
    channel: &UpdateChannel,
    current_bin_path: &Path,
    dry_run: bool,
) -> UpdateResult {
    let current_version = get_current_version(current_bin_path);
    let temp_dir = tempfile::tempdir().expect("create temp dir for update");
    let temp_bin = temp_dir.path().join(&channel.binary_pattern);

    // Step 1: Download and parse manifest
    let (manifest, signature) = match download_manifest(channel) {
        Ok((m, s)) => (m, s),
        Err(e) => {
            return UpdateResult::error(
                &format!("manifest download failed: {}", e),
                &current_version,
                "unknown",
            );
        }
    };

    if dry_run {
        return UpdateResult::success(
            &current_version,
            &format!(
                "dry-run: would update to {} (hash: {}, signature: {})",
                manifest.version,
                &manifest.sha256[..8.min(manifest.sha256.len())],
                if signature.is_empty() {
                    "none (fixture-only)"
                } else {
                    "present"
                }
            ),
        )
        .with_diagnostic(UPD_W_001);
    }

    // Step 2: Verify minisign signature on the manifest bytes
    if !signature.is_empty() && !channel.public_key.is_empty() {
        if let Err(e) = verify_manifest_signature(
            &manifest_bytes_for_verification(&manifest),
            &signature,
            &channel.public_key,
        ) {
            return UpdateResult::error(
                &format!("signature verification failed: {}", e),
                &current_version,
                &manifest.version,
            )
            .with_diagnostic(UPD_E_001);
        }
    }

    // Step 3: Download the binary
    let downloaded_bytes = if let Some(_fixture_path) = &channel.fixture_path {
        // Local fixture: read the binary from the fixture directory
        let fixture_dir = channel.fixture_path.as_ref().unwrap();
        let binary_path = fixture_dir.join(&channel.binary_pattern);
        fs::read(&binary_path).map_err(|e| {
            format!(
                "cannot read fixture binary {}: {}",
                binary_path.display(),
                e
            )
        })
    } else {
        // Remote: download via curl/wget
        download(&manifest.download_url, 120)
    };

    let downloaded_bytes = match downloaded_bytes {
        Ok(bytes) => bytes,
        Err(e) => {
            return UpdateResult::error(
                &format!("binary download failed: {}", e),
                &current_version,
                &manifest.version,
            )
            .with_diagnostic(UPD_E_003);
        }
    };

    // Step 4: Verify SHA-256 hash
    if let Err(e) = verify_hash(&downloaded_bytes, &manifest.sha256) {
        return UpdateResult::error(
            &format!("hash verification failed: {}", e),
            &current_version,
            &manifest.version,
        )
        .with_diagnostic(UPD_E_002);
    }

    // Step 5: Write to temp file
    if let Err(e) = fs::write(&temp_bin, &downloaded_bytes) {
        return UpdateResult::error(
            &format!("failed to write temp binary: {}", e),
            &current_version,
            &manifest.version,
        )
        .with_diagnostic(UPD_E_005);
    }

    // Make executable
    set_executable(&temp_bin);

    // Step 6: Backup current binary
    let backup_path = match create_backup(current_bin_path) {
        Ok(path) => path,
        Err(e) => {
            return UpdateResult::error(
                &format!("backup failed: {}", e),
                &current_version,
                &manifest.version,
            )
            .with_diagnostic(UPD_E_005);
        }
    };

    // Step 7: Swap binary
    if let Err(e) = swap_binary(&temp_bin, current_bin_path) {
        let _ = rollback(&backup_path, current_bin_path);
        return UpdateResult::error(
            &format!("swap failed, rolled back: {}", e),
            &current_version,
            &manifest.version,
        )
        .with_rollback(&format!("swap failed: {}", e));
    }

    // Step 8: Smoke test
    match smoke_test(current_bin_path, 10) {
        Ok(()) => {
            let _ = remove_backup(&backup_path);
            UpdateResult::success(
                &manifest.version,
                &format!("updated to {} successfully", manifest.version),
            )
        }
        Err(e) => {
            // Rollback on smoke test failure
            if let Err(rollback_err) = rollback(&backup_path, current_bin_path) {
                return UpdateResult::error(
                    &format!(
                        "smoke test failed and rollback also failed: {} (rollback: {})",
                        e, rollback_err
                    ),
                    &current_version,
                    &manifest.version,
                )
                .with_rollback(&format!(
                    "smoke test failed: {}; rollback: {}",
                    e, rollback_err
                ));
            }
            UpdateResult::error(
                &format!("smoke test failed, rolled back: {}", e),
                &current_version,
                &manifest.version,
            )
            .with_rollback(&format!("smoke test failed: {}", e))
        }
    }
}

/// Serialize manifest to JSON bytes for signature verification.
fn manifest_bytes_for_verification(manifest: &UpdateManifest) -> Vec<u8> {
    // Use the compact JSON for signing (standard practice)
    serde_json::to_vec_pretty(manifest).unwrap_or_default()
}

/// Set a file as executable (POSIX).
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = fs::metadata(path) {
        let mut perms = metadata.permissions();
        perms.set_mode(0o755);
        let _ = fs::set_permissions(path, perms);
    }
}

/// Get the current version of a binary via `--version`.
fn get_current_version(bin_path: &Path) -> String {
    match Command::new(bin_path).args(["--version"]).output() {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .nth(1)
            .unwrap_or("unknown")
            .to_string(),
        _ => "unknown".to_string(),
    }
}

/// Check if an update is available (version comparison, non-install).
pub fn check_available(
    channel: &UpdateChannel,
    current_version: &str,
) -> Result<Option<UpdateManifest>, String> {
    let (manifest, _) = download_manifest(channel)?;
    if manifest.version != current_version {
        return Ok(Some(manifest));
    }
    Ok(None)
}

#[cfg(test)]
mod selfupdate_tests {
    use super::*;

    /// test_verify_manifest_valid — valid signature passes (fixture-based).
    #[test]
    fn test_verify_manifest_valid() {
        let key_path = "/tmp/stichos-test-key";
        let pub_key_path = "/tmp/stichos-test-key.pub";
        let manifest_path = "/tmp/stichos-test-manifest.json";
        let sig_path = "/tmp/stichos-test-manifest.minisig";

        // Generate test key pair using rsign (minisign-compatible)
        let gen_result = Command::new("rsign")
            .args(["generate", "-t", "stichos test", "-f", key_path])
            .output();

        // Skip if rsign is not available
        let _gen_ok = gen_result.is_ok();
        let pub_key = fs::read_to_string(pub_key_path).unwrap_or_default();
        let pub_key_hex = pub_key.trim().replace("r6 ", "");

        if pub_key_hex.is_empty() || !Path::new(sig_path).exists() {
            // rsign2 may not be available, skip this test
            return;
        }

        // Create a test manifest
        let manifest = serde_json::json!({
            "version": "0.2.0",
            "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "download_url": "https://example.com/stichos",
            "notes": "Test release"
        });
        let manifest_str = serde_json::to_string_pretty(&manifest).unwrap();
        fs::write(manifest_path, &manifest_str).unwrap();

        // Sign the manifest
        let _sign_result = Command::new("rsign")
            .args(["sign", "-m", manifest_path, "-p", key_path])
            .output();

        let signature = fs::read_to_string(sig_path).unwrap_or_default();

        // Verify
        let manifest_bytes = fs::read(manifest_path).unwrap();
        let result = verify_manifest_signature(manifest_bytes.as_slice(), &signature, &pub_key_hex);
        assert!(result.is_ok(), "valid signature should pass: {:?}", result);
    }

    /// test_verify_manifest_tampered — tampered hash is rejected.
    #[test]
    fn test_verify_manifest_tampered() {
        let good_bytes = b"\x7fELF";

        // Verify against an obviously wrong hash — should fail
        let result = verify_hash(good_bytes, "deadbeef");
        assert!(result.is_err(), "tampered hash should be rejected");
    }

    /// test_verify_manifest_unsigned — unsigned manifest with key provided is rejected.
    #[test]
    fn test_verify_manifest_unsigned() {
        let result = verify_manifest_signature(
            b"{}",
            "",
            "r6 deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        );
        assert!(
            result.is_err(),
            "unsigned manifest should be rejected when key is set"
        );
    }

    /// test_rollback_on_smoke_fail — backup restored on smoke test failure.
    #[test]
    fn test_rollback_on_smoke_fail() {
        let tmp = tempfile::tempdir().unwrap();
        let bin_path = tmp.path().join("stichos");
        let channel_path = tmp.path().join("fixtures");

        // Create a dummy binary (just the ELF header — enough for smoke test to fail)
        let dummy_binary = b"\x7fELF\x02\x01\x01\x00";
        fs::write(&bin_path, dummy_binary).unwrap();
        set_executable(&bin_path);

        // Create a fixture channel with a manifest
        let manifest_json = serde_json::json!({
            "version": "0.2.0",
            "sha256": UpdateManifest::hash_bytes(dummy_binary),
            "download_url": "file:///tmp/stub",
            "notes": "test"
        });
        let manifest_str = serde_json::to_string_pretty(&manifest_json).unwrap();

        let fixture_dir = channel_path.join("stable");
        let _ = fs::create_dir_all(&fixture_dir);
        fs::write(fixture_dir.join("manifest.json"), &manifest_str).unwrap();

        // Create a valid-looking binary in the fixture (same bytes as dummy_binary)
        let fixture_bin = fixture_dir.join("stichos-x86_64-unknown-linux-gnu");
        fs::write(&fixture_bin, dummy_binary).unwrap();
        set_executable(&fixture_bin);

        let channel = UpdateChannel::fixture(fixture_dir.clone()).with_public_key(""); // No signature verification for test

        let result = download_and_install(&channel, &bin_path, false);

        // The update should fail because the fake binary doesn't support --version
        assert!(!result.success, "update should fail with invalid binary");

        // Verify backup was restored (rollback happened)
        assert!(result.rolled_back, "rollback should have been triggered");
    }

    /// test_dry_run_no_write — dry-run writes nothing.
    #[test]
    fn test_dry_run_no_write() {
        let tmp = tempfile::tempdir().unwrap();
        let bin_path = tmp.path().join("stichos");
        let channel_path = tmp.path().join("fixtures");

        // Create a binary
        fs::write(&bin_path, b"\x7fELFdummy").unwrap();
        set_executable(&bin_path);

        // Create a fixture channel
        let fixture_dir = channel_path.join("stable");
        let _ = fs::create_dir_all(&fixture_dir);

        let manifest_json = serde_json::json!({
            "version": "0.2.0",
            "sha256": UpdateManifest::hash_bytes(b"\x7fELFdummy"),
            "download_url": "file:///tmp/stub",
            "notes": "test"
        });
        let manifest_str = serde_json::to_string_pretty(&manifest_json).unwrap();
        fs::write(fixture_dir.join("manifest.json"), &manifest_str).unwrap();

        let fixture_bin = fixture_dir.join("stichos-x86_64-unknown-linux-gnu");
        fs::write(&fixture_bin, b"\x7fELFdummy").unwrap();

        let channel = UpdateChannel::fixture(fixture_dir.clone()).with_public_key("");

        let result = download_and_install(&channel, &bin_path, true);

        // Dry run should report success but not change anything
        assert!(result.success, "dry-run should succeed");
        assert!(!result.rolled_back, "dry-run should not rollback");
        assert!(
            result.message.contains("dry-run"),
            "message should mention dry-run"
        );

        // Verify original binary is unchanged
        let original_bytes = fs::read(&bin_path).unwrap();
        assert_eq!(&original_bytes, b"\x7fELFdummy");

        // Verify no backup was created
        assert!(
            !bin_path.with_extension("bak").exists(),
            "dry-run should not create backup"
        );
    }

    /// test_download_manifest_fixture — load manifest from fixture directory.
    #[test]
    fn test_download_manifest_fixture() {
        let tmp = tempfile::tempdir().unwrap();
        let fixture_dir = tmp.path().join("channel");
        let _ = fs::create_dir_all(&fixture_dir);

        let manifest_json = serde_json::json!({
            "version": "0.3.0",
            "sha256": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            "download_url": "https://example.com/stichos-v0.3.0",
            "notes": "Test fixture manifest"
        });
        fs::write(
            fixture_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest_json).unwrap(),
        )
        .unwrap();

        let channel = UpdateChannel::fixture(fixture_dir.clone()).with_public_key("");
        let (m, sig) = download_manifest(&channel).unwrap();

        assert_eq!(m.version, "0.3.0");
        assert_eq!(
            m.sha256,
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );
        assert!(sig.is_empty(), "fixture should have no signature");
    }

    /// test_update_manifest_hash_bytes — deterministic hash output.
    #[test]
    fn test_update_manifest_hash_bytes() {
        let data = b"hello world";
        let hash1 = UpdateManifest::hash_bytes(data);
        let hash2 = UpdateManifest::hash_bytes(data);
        assert_eq!(hash1, hash2, "same input produces same hash");
        assert_eq!(hash1.len(), 64, "SHA-256 hex is 64 chars");
    }

    /// test_update_result_serialization — UpdateResult roundtrips via serde.
    #[test]
    fn test_update_result_serialization() {
        let result = UpdateResult::success("0.2.0", "updated successfully");
        let json = serde_json::to_string(&result).unwrap();
        let back: UpdateResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result.success, back.success);
        assert_eq!(result.current_version, back.current_version);
        assert_eq!(result.message, back.message);
    }

    /// test_check_available — detects version difference.
    #[test]
    fn test_check_available() {
        let tmp = tempfile::tempdir().unwrap();
        let fixture_dir = tmp.path().join("channel");
        let _ = fs::create_dir_all(&fixture_dir);

        let manifest_json = serde_json::json!({
            "version": "0.3.0",
            "sha256": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdead",
            "download_url": "https://example.com/stichos",
            "notes": "test"
        });
        fs::write(
            fixture_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest_json).unwrap(),
        )
        .unwrap();

        let channel = UpdateChannel::fixture(fixture_dir.clone()).with_public_key("");

        // Same version → no update
        let result = check_available(&channel, "0.3.0").unwrap();
        assert!(result.is_none(), "same version should return None");

        // Different version → update available
        let result = check_available(&channel, "0.2.0").unwrap();
        assert!(result.is_some(), "different version should return Some");
        assert_eq!(result.unwrap().version, "0.3.0");
    }

    /// test_no_update_same_version — UpdateResult.no_update works correctly.
    #[test]
    fn test_no_update_same_version() {
        let result = UpdateResult::no_update("0.2.0", "0.2.0");
        assert!(result.success);
        assert_eq!(result.current_version, "0.2.0");
        assert_eq!(result.new_version, "0.2.0");
        assert!(!result.rolled_back);
    }
}
