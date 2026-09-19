// SPDX-License-Identifier: MIT OR Apache-2.0
//! `stichos sign` — PAdES-B-B sign-later for PDF documents (C13 — P5-13).
//!
//! Signs an unsigned PDF document using PAdES-B-B sign-later.
//! Uses the signature module to create a self-signed PKCS#7 signature.

use anyhow::{bail, Context, Result};
use gp_core::signature::{self, SignAlgorithm, SigningIdentity};

/// Execute the sign command.
pub fn cmd_sign(
    path: &std::path::Path,
    subject: Option<String>,
    algorithm: Option<String>,
    output: Option<std::path::PathBuf>,
) -> Result<()> {
    let source = std::fs::read(path).with_context(|| "failed to read input PDF")?;

    let identity = SigningIdentity {
        algorithm: parse_algorithm(algorithm.as_deref())?,
        subject_cn: subject.unwrap_or_else(|| "stichos".to_string()),
    };

    let result = signature::sign_pdf(&source, &identity);

    if !result.success {
        let reasons: Vec<String> = result
            .diagnostics
            .iter()
            .map(|d| format!("{}: {}", d.code, d.message))
            .collect();
        bail!("signing failed: {}", reasons.join("; "));
    }

    // Determine output path
    let out_path = match output {
        Some(p) => p,
        None => {
            let mut p = path.to_path_buf();
            p.set_extension("signed.pdf");
            p
        }
    };

    std::fs::write(&out_path, &result.output)
        .with_context(|| format!("failed to write signed PDF to {:?}", out_path))?;

    // Print summary
    let sig = &result.signature;
    eprintln!("Signed: {}", path.display());
    eprintln!("  Subject: {}", sig.subject);
    eprintln!("  Algorithm: {}", sig.algorithm);
    eprintln!("  Digest: {}", sig.digest);
    eprintln!("  Time: {}", sig.signing_time);
    eprintln!("  Output: {}", out_path.display());

    // Write JSON envelope to stdout
    let json = serde_json::json!({
        "success": true,
        "output_path": out_path.to_string_lossy().to_string(),
        "signature": {
            "subject": sig.subject,
            "algorithm": sig.algorithm,
            "digest": sig.digest,
            "signing_time": sig.signing_time,
            "byte_range": sig.byte_range,
        },
        "diagnostics": result.diagnostics,
    });
    println!("{}", serde_json::to_string_pretty(&json).unwrap());

    Ok(())
}

fn parse_algorithm(s: Option<&str>) -> Result<SignAlgorithm> {
    match s {
        None | Some("rsa") | Some("rsa_sha256") => Ok(SignAlgorithm::RsaSha256),
        Some("ecdsa") | Some("ecdsa_p384") => Ok(SignAlgorithm::EcdsaP384),
        Some(other) => bail!("unsupported algorithm: {} (use 'rsa' or 'ecdsa')", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_algorithm_none() {
        assert!(matches!(
            parse_algorithm(None),
            Ok(SignAlgorithm::RsaSha256)
        ));
    }

    #[test]
    fn test_parse_algorithm_rsa() {
        assert!(matches!(
            parse_algorithm(Some("rsa")),
            Ok(SignAlgorithm::RsaSha256)
        ));
    }

    #[test]
    fn test_parse_algorithm_ecdsa() {
        assert!(matches!(
            parse_algorithm(Some("ecdsa_p384")),
            Ok(SignAlgorithm::EcdsaP384)
        ));
    }

    #[test]
    fn test_parse_algorithm_invalid() {
        assert!(parse_algorithm(Some("blowfish")).is_err());
    }
}
