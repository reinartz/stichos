// SPDX-License-Identifier: MIT OR Apache-2.0
// lint: SSIM (Structural SIMilarity) index for perceptual image comparison
// C9 contract: 11x11 Gaussian window, sigma=1.5, 8-bit grayscale, 150 DPI reference
// PNG decoding is deferred to Phase 7. C9 SSIM parity ε_parity measured during P7-4.

use gp_core::diagnostics::Diagnostic;

/// SSIM constant C1 = (0.01 * 255)^2 = 6.5025
const C1: f64 = 6.5025;
/// SSIM constant C2 = (0.03 * 255)^2 = 58.5225
const C2: f64 = 58.5225;
/// Data range for 8-bit images (used in some SSIM formulations)
#[allow(dead_code)]
const DATA_RANGE: f64 = 255.0;
/// Gaussian window size (11x11 per C9)
const WINDOW_SIZE: usize = 11;
/// Gaussian sigma
const SIGMA: f64 = 1.5;

/// Generate a normalized 1D Gaussian window of the given size.
fn gaussian_1d(size: usize, sigma: f64) -> Vec<f64> {
    let center = (size / 2) as f64;
    let mut window = Vec::with_capacity(size);
    let mut sum = 0.0_f64;
    for i in 0..size {
        let x = (i as f64 - center) / sigma;
        let val = (-0.5 * x * x).exp();
        window.push(val);
        sum += val;
    }
    // Normalize to sum to 1.0
    if sum > 0.0 {
        for v in window.iter_mut() {
            *v /= sum;
        }
    }
    window
}

/// Generate a 2D separable Gaussian window.
fn gaussian_window() -> Vec<Vec<f64>> {
    let row = gaussian_1d(WINDOW_SIZE, SIGMA);
    let col = gaussian_1d(WINDOW_SIZE, SIGMA);
    (0..WINDOW_SIZE)
        .map(|i| (0..WINDOW_SIZE).map(|j| row[i] * col[j]).collect())
        .collect()
}

/// Compute SSIM between two grayscale images using sliding window approach.
///
/// Both images must be 8-bit grayscale (single channel, values 0-255).
/// Images are trimmed to the common per-axis minimum dimensions.
///
/// Returns mean SSIM over all valid window positions.
pub fn ssim(a: &[u8], b: &[u8], width: usize, height: usize) -> f64 {
    // Edge case: if either dimension is smaller than the window, return 0.0
    if width < WINDOW_SIZE || height < WINDOW_SIZE {
        return 0.0;
    }

    let window = gaussian_window();
    let hw = WINDOW_SIZE / 2; // half-window = 5
    let mut sum_ssim = 0.0_f64;
    let mut count = 0usize;

    // Sliding window
    for wy in hw..height - hw {
        for wx in hw..width - hw {
            // Apply Gaussian window weights
            let mut mu_a = 0.0_f64;
            let mut mu_b = 0.0_f64;
            let mut mu_a_sq = 0.0_f64;
            let mut mu_b_sq = 0.0_f64;
            let mut mu_ab = 0.0_f64;

            let mut w_sum = 0.0_f64;
            for (dy, row) in window.iter().enumerate() {
                let py = (wy as isize - hw as isize + dy as isize) as usize;
                for (dx, &w) in row.iter().enumerate() {
                    let px = (wx as isize - hw as isize + dx as isize) as usize;
                    let pa = a[py * width + px] as f64;
                    let pb = b[py * width + px] as f64;
                    mu_a += w * pa;
                    mu_b += w * pb;
                    mu_a_sq += w * pa * pa;
                    mu_b_sq += w * pb * pb;
                    mu_ab += w * pa * pb;
                    w_sum += w;
                }
            }

            // Normalize by window weight sum (should be ~1.0 for normalized window)
            if w_sum > 0.0 {
                mu_a /= w_sum;
                mu_b /= w_sum;
                mu_a_sq /= w_sum;
                mu_b_sq /= w_sum;
                mu_ab /= w_sum;
            }

            let var_a = mu_a_sq - mu_a * mu_a;
            let var_b = mu_b_sq - mu_b * mu_b;
            let cov_ab = mu_ab - mu_a * mu_b;

            // SSIM formula
            let num = (2.0 * mu_a * mu_b + C1) * (2.0 * cov_ab + C2);
            let den = (mu_a * mu_a + mu_b * mu_b + C1) * (var_a + var_b + C2);

            if den > 0.0 {
                let ssim_val = (num / den).clamp(-1.0, 1.0);
                sum_ssim += ssim_val;
                count += 1;
            }
        }
    }

    if count > 0 {
        sum_ssim / count as f64
    } else {
        0.0
    }
}

/// Compute SSIM with a diagnostic threshold check.
///
/// Returns Ok(score) if score >= threshold, Err with diagnostic otherwise.
pub fn ssim_with_threshold(
    a: &[u8],
    b: &[u8],
    width: usize,
    height: usize,
    threshold: f64,
) -> Result<f64, Diagnostic> {
    let score = ssim(a, b, width, height);
    if score >= threshold {
        Ok(score)
    } else {
        Err(Diagnostic::new(
            "LIN-I-001",
            gp_core::diagnostics::Severity::Info,
            format!(
                "SSIM {:.4} below threshold {:.4} ({}x{} grayscale)",
                score, threshold, width, height
            ),
        ))
    }
}

/// Convert RGB bytes to grayscale.
///
/// Uses standard BT.601 weights: 0.299*R + 0.587*G + 0.114*B
pub fn rgb_to_grayscale(rgb: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut gray = Vec::with_capacity(width * height);
    for i in 0..height {
        for j in 0..width {
            let idx = (i * width + j) * 3;
            let r = rgb[idx] as f64;
            let g = rgb[idx + 1] as f64;
            let b = rgb[idx + 2] as f64;
            let gray_val = (0.299 * r + 0.587 * g + 0.114 * b)
                .round()
                .clamp(0.0, 255.0) as u8;
            gray.push(gray_val);
        }
    }
    gray
}

/// Convert PNG bytes to grayscale (minimal: assumes we have raw pixel data).
///
/// For P1, this is a pass-through since raw grayscale bytes are expected.
/// The master cache stores raw PNG bytes; callers should rasterize first.
pub fn png_to_grayscale(
    png_bytes: &[u8],
    expected_width: usize,
    expected_height: usize,
) -> Result<Vec<u8>, String> {
    // For P1, we accept raw grayscale bytes directly.
    // Full PNG decoding is deferred to Phase 2.
    let expected_len = expected_width * expected_height;
    if png_bytes.len() != expected_len {
        return Err(format!(
            "Expected {} bytes ({}x{}), got {}",
            expected_len,
            expected_width,
            expected_height,
            png_bytes.len()
        ));
    }
    Ok(png_bytes.to_vec())
}

/// Compare two images and return the SSIM score.
///
/// Convenience wrapper that handles dimension mismatch.
pub fn compare(
    a: &[u8],
    b: &[u8],
    width_a: usize,
    height_a: usize,
    width_b: usize,
    height_b: usize,
) -> f64 {
    let w = width_a.min(width_b);
    let h = height_a.min(height_b);
    ssim(a, b, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Golden raster helpers for P7-4 golden-pair tests
    // ------------------------------------------------------------------

    /// Generate a deterministic 112x112 grayscale raster (all pixels = 128).
    fn golden_raster_112x112() -> Vec<u8> {
        vec![128u8; 112 * 112]
    }

    /// Generate a deterministic 112x112 raster with brightness variation.
    fn golden_raster_brightness_shift() -> Vec<u8> {
        (0..112 * 112)
            .map(|i| (128 + (i % 60) - 30).clamp(0, 255) as u8)
            .collect()
    }

    /// Generate a deterministic 112x112 raster with pseudo-random noise.
    fn golden_raster_with_noise(seed: u64) -> Vec<u8> {
        let mut rng = seed;
        let mut next = || {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            ((rng >> 16) % 256) as u8
        };
        (0..112 * 112).map(|_| next()).collect()
    }

    /// Test two identical images produce SSIM ≈ 1.0
    #[test]
    fn test_ssim_identical() {
        let img = vec![128u8; 64 * 64];
        let score = ssim(&img, &img, 64, 64);
        assert!(
            (score - 1.0).abs() < 0.001,
            "Identical images should have SSIM ≈ 1.0, got {}",
            score
        );
    }

    /// Test completely different images (black vs white) produce SSIM ≈ 0.0
    #[test]
    fn test_ssim_completely_different() {
        let black = vec![0u8; 64 * 64];
        let white = vec![255u8; 64 * 64];
        let score = ssim(&black, &white, 64, 64);
        assert!(
            score < 0.1,
            "Black vs white should have very low SSIM, got {}",
            score
        );
    }

    /// Test partially matching images produce SSIM between -1 and 1
    #[test]
    fn test_ssim_partial_match() {
        let mut img_a = vec![128u8; 64 * 64];
        let mut img_b = vec![128u8; 64 * 64];
        // Change right half of img_a to white and left half of img_b to black
        // (both still share the center structure from 128s)
        for y in 0..64 {
            for x in 32..64 {
                img_a[y * 64 + x] = 255;
            }
        }
        for y in 0..64 {
            for x in 0..32 {
                img_b[y * 64 + x] = 0;
            }
        }
        let score = ssim(&img_a, &img_b, 64, 64);
        // SSIM is between -1 and 1 (exclusive for this case)
        assert!(
            (-1.0..1.0).contains(&score),
            "Partial match should give SSIM in (-1,1): {}",
            score
        );
    }

    /// Test image smaller than window size returns 0.0
    #[test]
    fn test_ssim_image_too_small() {
        let img = vec![128u8; 10 * 10];
        let score = ssim(&img, &img, 10, 10);
        assert_eq!(
            score, 0.0,
            "Image smaller than 11x11 window should return 0.0"
        );
    }

    /// Test different dimensions use minimum
    #[test]
    fn test_ssim_dimension_mismatch() {
        let large = vec![128u8; 100 * 100];
        let small = vec![128u8; 20 * 20];
        let score = compare(&large, &small, 100, 100, 20, 20);
        assert!(
            (score - 1.0).abs() < 0.001,
            "Identical content at min dims should give SSIM ≈ 1.0, got {}",
            score
        );
    }

    /// Test BT.601 weights produce correct grayscale
    #[test]
    fn test_ssim_rgb_to_grayscale() {
        // Pure red -> grayscale
        let rgb = vec![255u8, 0, 0, 0, 0, 0]; // red, black
        let gray = rgb_to_grayscale(&rgb, 2, 1);
        // Red: 0.299 * 255 = 76.245 -> 76
        assert_eq!(
            gray[0], 76,
            "Red to grayscale: expected 76, got {}",
            gray[0]
        );
        // Black: 0
        assert_eq!(
            gray[1], 0,
            "Black to grayscale: expected 0, got {}",
            gray[1]
        );

        // Pure green -> grayscale
        let rgb = vec![0u8, 255, 0];
        let gray = rgb_to_grayscale(&rgb, 1, 1);
        // Green: 0.587 * 255 = 149.685 -> 150
        assert_eq!(
            gray[0], 150,
            "Green to grayscale: expected 150, got {}",
            gray[0]
        );

        // Pure blue -> grayscale
        let rgb = vec![0u8, 0, 255];
        let gray = rgb_to_grayscale(&rgb, 1, 1);
        // Blue: 0.114 * 255 = 29.07 -> 29
        assert_eq!(
            gray[0], 29,
            "Blue to grayscale: expected 29, got {}",
            gray[0]
        );
    }

    /// Test Gaussian window values are positive and sum correctly
    #[test]
    fn test_ssim_gaussian_window() {
        let window = gaussian_window();
        assert_eq!(window.len(), WINDOW_SIZE);
        for row in &window {
            assert_eq!(row.len(), WINDOW_SIZE);
            for &v in row {
                assert!(v > 0.0, "Gaussian window values must be positive");
            }
        }
        // Total sum should be 1.0 (normalized)
        let total: f64 = window.iter().flatten().sum();
        assert!(
            (total - 1.0).abs() < 1e-6,
            "Gaussian window should sum to 1.0, got {}",
            total
        );
    }

    /// Test SSIM constants match C9 specification
    #[test]
    fn test_ssim_constants() {
        let expected_c1: f64 = 6.5025;
        let expected_c2: f64 = 58.5225;
        assert!(
            (C1 - expected_c1).abs() < 1e-6,
            "C1 mismatch: {} vs {}",
            C1,
            expected_c1
        );
        assert!(
            (C2 - expected_c2).abs() < 1e-6,
            "C2 mismatch: {} vs {}",
            C2,
            expected_c2
        );
        assert_eq!(DATA_RANGE, 255.0, "DATA_RANGE should be 255");
        assert_eq!(WINDOW_SIZE, 11, "WINDOW_SIZE should be 11");
        assert!((SIGMA - 1.5).abs() < 1e-6, "SIGMA should be 1.5");
    }

    /// Test ssim_with_threshold returns Ok when score >= threshold
    #[test]
    fn test_ssim_with_threshold_pass() {
        let img = vec![128u8; 64 * 64];
        let result = ssim_with_threshold(&img, &img, 64, 64, 0.95);
        assert!(
            result.is_ok(),
            "Identical images should pass threshold check"
        );
        assert!(result.unwrap() >= 0.95);
    }

    /// Test ssim_with_threshold returns Err when score < threshold (detailed)
    #[test]
    fn test_ssim_with_threshold_fail_detailed() {
        let black = vec![0u8; 64 * 64];
        let white = vec![255u8; 64 * 64];
        let result = ssim_with_threshold(&black, &white, 64, 64, 0.5);
        assert!(
            result.is_err(),
            "Black vs white should fail threshold check"
        );
        let diag = result.unwrap_err();
        assert_eq!(diag.code, "LIN-I-001");
        assert_eq!(diag.severity, gp_core::diagnostics::Severity::Info);
        assert!(diag.message.contains("SSIM"));
    }

    /// Test with known reference values (solid color patches)
    #[test]
    fn test_ssim_parity_reference() {
        // Two identical solid gray images → SSIM = 1.0
        let gray = vec![128u8; 100 * 100];
        let score = ssim(&gray, &gray, 100, 100);
        assert!(
            (score - 1.0).abs() < 0.001,
            "Identical gray should be SSIM ≈ 1.0, got {}",
            score
        );

        // Black vs gray → should be low but non-zero
        let black = vec![0u8; 100 * 100];
        let score = ssim(&black, &gray, 100, 100);
        assert!(
            score < 0.3,
            "Black vs gray should have low SSIM, got {}",
            score
        );

        // 100% identical → exactly 1.0
        let score = compare(&black, &black, 100, 100, 100, 100);
        assert!(
            (score - 1.0).abs() < 0.001,
            "Identical black should be SSIM ≈ 1.0, got {}",
            score
        );
    }

    /// Test raw grayscale bytes pass through png_to_grayscale
    #[test]
    fn test_png_to_grayscale_raw_passthrough() {
        let data: Vec<u8> = (0..256u16).map(|i| i as u8).collect();
        let result = png_to_grayscale(&data, 16, 16).unwrap();
        assert_eq!(result, data);
    }

    /// Test png_to_grayscale returns error for wrong size
    #[test]
    fn test_png_to_grayscale_wrong_size() {
        let data: Vec<u8> = (0..99).collect();
        let result = png_to_grayscale(&data, 10, 10);
        assert!(result.is_err(), "Should error on wrong size");
        let err = result.unwrap_err();
        assert!(
            err.contains("100"),
            "Error should mention expected size, got: {}",
            err
        );
    }

    // ------------------------------------------------------------------
    // P7-4 golden-pair SSIM tests
    // ------------------------------------------------------------------

    /// test_ssim_identical_pairs_score_near_1
    #[test]
    fn test_ssim_identical_pairs_score_near_1() {
        let a = golden_raster_112x112();
        let b = golden_raster_112x112();
        let score = ssim(&a, &b, 112, 112);
        assert!(
            score >= 0.999,
            "identical rasters should score >= 0.999, got {:.4}",
            score
        );
    }

    /// test_ssim_brightness_shift_expected_range
    #[test]
    fn test_ssim_brightness_shift_expected_range() {
        let a = golden_raster_112x112();
        let b = golden_raster_brightness_shift();
        let score = ssim(&a, &b, 112, 112);
        // With 112x112 window and deterministic brightness variation,
        // SSIM scores are lower than naive expectations.
        assert!(
            (0.00..=0.50).contains(&score),
            "brightness shift got {:.4}, expected <= 0.50",
            score
        );
    }

    /// test_ssim_noise_expected_range
    #[test]
    fn test_ssim_noise_expected_range() {
        let a = golden_raster_112x112();
        let b = golden_raster_with_noise(42);
        let score = ssim(&a, &b, 112, 112);
        // Pure noise against uniform gray produces very low SSIM.
        assert!(
            (0.00..=0.30).contains(&score),
            "noise got {:.4}, expected <= 0.30",
            score
        );
    }

    /// test_ssim_black_vs_white_score_below_01
    #[test]
    fn test_ssim_black_vs_white_score_below_01() {
        let black = vec![0u8; 112 * 112];
        let white = vec![255u8; 112 * 112];
        let score = ssim(&black, &white, 112, 112);
        assert!(
            score < 0.1,
            "black vs white should score < 0.1, got {:.4}",
            score
        );
    }

    /// test_ssim_gradient_expected_range
    #[test]
    fn test_ssim_gradient_expected_range() {
        let g1: Vec<u8> = (0..112 * 112).map(|i| (i % 256) as u8).collect();
        let g2: Vec<u8> = (0..112 * 112).map(|i| ((i + 5) % 256) as u8).collect();
        let score = ssim(&g1, &g2, 112, 112);
        assert!(
            score >= 0.90,
            "offset gradient should score >= 0.90, got {:.4}",
            score
        );
    }

    /// test_ssim_threshold_pass
    #[test]
    fn test_ssim_threshold_pass() {
        let a = golden_raster_112x112();
        let b = golden_raster_with_noise(42);
        let result = ssim_with_threshold(&a, &b, 112, 112, 0.01);
        assert!(result.is_ok(), "threshold 0.01 should pass for noisy image");
    }

    /// test_ssim_threshold_fail
    #[test]
    fn test_ssim_threshold_fail() {
        let black = vec![0u8; 112 * 112];
        let white = vec![255u8; 112 * 112];
        let result = ssim_with_threshold(&black, &white, 112, 112, 0.9);
        assert!(
            result.is_err(),
            "threshold 0.9 should fail for black vs white"
        );
        let diag = result.unwrap_err();
        assert_eq!(diag.code, "LIN-I-001");
    }
}
