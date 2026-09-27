// SPDX-License-Identifier: MIT OR Apache-2.0
//! `stichos sign` — PAdES-B-B sign-later for PDF documents (C13 — P5-13).
//!
//! Signs an unsigned PDF document using PAdES-B-B sign-later.
//! Uses the signature module to create a self-signed PKCS#7 signature.
//! Supports user-provided PKCS#12 certificate chains via --pkcs12 flag.

use anyhow::{bail, Context, Result};
use gp_core::signature::{self, verify_signature, CertSource, SignAlgorithm, SigningIdentity};

/// Execute the sign command.
pub fn cmd_sign(
    path: &std::path::Path,
    subject: Option<String>,
    algorithm: Option<String>,
    output: Option<std::path::PathBuf>,
    verify: bool,
    pkcs12_path: Option<String>,
    pkcs12_password: Option<String>,
) -> Result<()> {
    let source = std::fs::read(path).with_context(|| "failed to read input PDF")?;

    let identity = if let Some(p12_path) = pkcs12_path {
        let password = pkcs12_password.unwrap_or_default();
        SigningIdentity {
            algorithm: parse_algorithm(algorithm.as_deref())?,
            subject_cn: subject.unwrap_or_default(),
            cert_source: CertSource::Pkcs12 {
                path: p12_path,
                password,
            },
        }
    } else {
        SigningIdentity {
            algorithm: parse_algorithm(algorithm.as_deref())?,
            subject_cn: subject.unwrap_or_else(|| "stichos".to_string()),
            cert_source: CertSource::SelfSigned,
        }
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

    // Verify the signature if requested
    if verify {
        let vr = verify_signature(&result.output);
        if !vr.valid {
            eprintln!("WARNING: Signature verification failed:");
            for error in &vr.errors {
                eprintln!("  ERROR: {}", error);
            }
            for warning in &vr.warnings {
                eprintln!("  WARNING: {}", warning);
            }
        } else {
            eprintln!("Signature verification: OK");
            eprintln!("  Subject: {:?}", vr.subject);
            eprintln!("  Algorithm: {:?}", vr.algorithm);
            eprintln!("  Signing time: {:?}", vr.signing_time);
        }
    }

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
            "signature_value": sig.signature_value,
            "signature_length": sig.signature_length,
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

/// Execute the verify command.
pub fn cmd_verify(path: &std::path::Path) -> Result<()> {
    let source = std::fs::read(path).with_context(|| "failed to read PDF")?;
    let vr = verify_signature(&source);

    println!("{}", serde_json::to_string_pretty(&vr).unwrap());

    if !vr.valid {
        eprintln!("Signature verification: FAILED");
        for error in &vr.errors {
            eprintln!("  ERROR: {}", error);
        }
    } else {
        eprintln!("Signature verification: OK");
    }

    Ok(())
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
