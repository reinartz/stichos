//! Payload budgeter: enforces Claude vision constraints on image payloads.
//!
//! Per C8 contract: ≤20 image blocks, pre-resized to known sizes,
//! estimator ⌈w/28⌉ × ⌈h/28⌉, ≤2000 px/side.

use gp_core::diagnostics::Diagnostic;

/// Maximum number of image blocks per request (Claude vision limit).
pub const MAX_IMAGE_BLOCKS: usize = 20;

/// Maximum pixel dimension per image side (Claude vision limit).
pub const MAX_PIXELS_PER_SIDE: usize = 2000;

/// Visual token grid size (Claude vision token model).
pub const TOKEN_GRID: usize = 28;

/// Maximum visual tokens for standard tier (768px long edge).
pub const MAX_STANDARD_TOKENS: usize = 1568;

/// Maximum visual tokens for high-res tier (2576px long edge).
pub const MAX_HIGHRES_TOKENS: usize = 4784;

/// Compute visual tokens for an image of given dimensions.
/// Formula: ⌈width/TOKEN_GRID⌉ × ⌈height/TOKEN_GRID⌉.
pub fn compute_visual_tokens(width: usize, height: usize) -> usize {
    let tw = (width as f64 / TOKEN_GRID as f64).ceil() as usize;
    let th = (height as f64 / TOKEN_GRID as f64).ceil() as usize;
    tw * th
}

/// A single image block in the payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImageBlock {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// Visual tokens consumed.
    pub visual_tokens: usize,
    /// Byte size of the encoded image.
    pub byte_size: usize,
    /// Format identifier.
    pub format: String,
}

impl ImageBlock {
    pub fn new(width: usize, height: usize, byte_size: usize, format: String) -> Self {
        Self {
            width,
            height,
            visual_tokens: compute_visual_tokens(width, height),
            byte_size,
            format,
        }
    }
}

/// Budget report for a set of image blocks.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BudgetReport {
    /// Total image blocks.
    pub block_count: usize,
    /// Total visual tokens.
    pub total_tokens: usize,
    /// Total byte size.
    pub total_bytes: usize,
    /// Whether the budget is respected.
    pub within_budget: bool,
    /// List of violations (empty if within budget).
    pub violations: Vec<String>,
}

impl BudgetReport {
    pub fn new(
        block_count: usize,
        total_tokens: usize,
        total_bytes: usize,
        within_budget: bool,
    ) -> Self {
        Self {
            block_count,
            total_tokens,
            total_bytes,
            within_budget,
            violations: Vec::new(),
        }
    }
}

/// Check if a single image block fits within the per-image budget.
/// Returns Ok(()) if valid, Err with diagnostic otherwise.
pub fn check_single_block(width: usize, height: usize) -> Result<(), Diagnostic> {
    if width > MAX_PIXELS_PER_SIDE {
        return Err(Diagnostic::new(
            "LIN-B-001",
            gp_core::diagnostics::Severity::Error,
            format!(
                "image width {} exceeds maximum {}px",
                width, MAX_PIXELS_PER_SIDE
            ),
        ));
    }
    if height > MAX_PIXELS_PER_SIDE {
        return Err(Diagnostic::new(
            "LIN-B-002",
            gp_core::diagnostics::Severity::Error,
            format!(
                "image height {} exceeds maximum {}px",
                height, MAX_PIXELS_PER_SIDE
            ),
        ));
    }
    Ok(())
}

/// Check if a list of image blocks fits within the payload budget.
///
/// Enforces:
/// - No more than MAX_IMAGE_BLOCKS (20) blocks
/// - No image exceeds MAX_PIXELS_PER_SIDE (2000) per side
/// - Returns a BudgetReport with total tokens and violations
pub fn check_payload_budget(blocks: &[ImageBlock]) -> BudgetReport {
    let mut report = BudgetReport::new(
        blocks.len(),
        blocks.iter().map(|b| b.visual_tokens).sum(),
        blocks.iter().map(|b| b.byte_size).sum(),
        true,
    );

    if blocks.len() > MAX_IMAGE_BLOCKS {
        report.within_budget = false;
        report.violations.push(format!(
            "block count {} exceeds maximum {}",
            blocks.len(),
            MAX_IMAGE_BLOCKS
        ));
    }

    for block in blocks {
        if block.width > MAX_PIXELS_PER_SIDE || block.height > MAX_PIXELS_PER_SIDE {
            report.within_budget = false;
            report.violations.push(format!(
                "block {}x{} exceeds {}px per side",
                block.width, block.height, MAX_PIXELS_PER_SIDE
            ));
        }
    }

    report
}

/// Check if adding a block would exceed the budget.
/// Returns Ok(new_total_tokens) if it fits, Err with diagnostic if not.
pub fn check_can_add(
    current_blocks: usize,
    current_tokens: usize,
    new_block: &ImageBlock,
) -> Result<usize, Diagnostic> {
    if current_blocks + 1 > MAX_IMAGE_BLOCKS {
        return Err(Diagnostic::new(
            "LIN-B-001",
            gp_core::diagnostics::Severity::Error,
            format!(
                "adding block would exceed {} image block limit ({} current + 1)",
                MAX_IMAGE_BLOCKS, current_blocks
            ),
        ));
    }

    check_single_block(new_block.width, new_block.height)?;

    Ok(current_tokens + new_block.visual_tokens)
}

/// Pre-resize an image dimension to fit within MAX_PIXELS_PER_SIDE.
pub fn clamp_dimension(size: usize) -> usize {
    size.min(MAX_PIXELS_PER_SIDE)
}

/// Compute the resize target for an image that preserves aspect ratio
/// while fitting within MAX_PIXELS_PER_SIDE × MAX_PIXELS_PER_SIDE.
pub fn resize_target(width: usize, height: usize) -> (usize, usize) {
    if width <= MAX_PIXELS_PER_SIDE && height <= MAX_PIXELS_PER_SIDE {
        return (width, height);
    }

    let ratio = width as f64 / height as f64;
    if ratio >= 1.0 {
        // Wider than tall: clamp width, scale height
        (
            MAX_PIXELS_PER_SIDE,
            (MAX_PIXELS_PER_SIDE as f64 / ratio) as usize,
        )
    } else {
        // Taller than wide: clamp height, scale width
        (
            (MAX_PIXELS_PER_SIDE as f64 * ratio) as usize,
            MAX_PIXELS_PER_SIDE,
        )
    }
}

/// Enforce the full payload budget: clamp sizes, check counts, report budget.
pub fn enforce_budget(blocks: Vec<ImageBlock>) -> Result<BudgetReport, Diagnostic> {
    let clamped: Vec<ImageBlock> = blocks
        .iter()
        .map(|b| {
            let (cw, ch) = resize_target(b.width, b.height);
            let mut clamped = b.clone();
            clamped.width = cw;
            clamped.height = ch;
            clamped.visual_tokens = compute_visual_tokens(cw, ch);
            clamped
        })
        .collect();

    let report = check_payload_budget(&clamped);

    if !report.within_budget {
        return Err(Diagnostic::new(
            "LIN-B-002",
            gp_core::diagnostics::Severity::Error,
            format!("payload exceeds budget: {}", report.violations.join("; ")),
        ));
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // test_compute_visual_tokens — table-driven
    // ------------------------------------------------------------------

    #[test]
    fn test_compute_visual_tokens() {
        let table: [(usize, usize, usize); 5] = [
            (768, 1024, 28 * 37),  // ceil(768/28)=28, ceil(1024/28)=37
            (2000, 2000, 72 * 72), // ceil(2000/28)=72
            (1240, 1754, 45 * 63), // ceil(1240/28)=45, ceil(1754/28)=63
            (100, 100, 4 * 4),     // ceil(100/28)=4
            (0, 0, 0),
        ];
        for &(w, h, expected) in &table {
            let result = compute_visual_tokens(w, h);
            assert_eq!(
                result, expected,
                "compute_visual_tokens({}, {}) = {}, expected {}",
                w, h, result, expected
            );
        }
    }

    // ------------------------------------------------------------------
    // test_check_single_block_ok
    // ------------------------------------------------------------------

    #[test]
    fn test_check_single_block_ok() {
        let result = check_single_block(1000, 1000);
        assert!(result.is_ok(), "1000x1000 should be within limits");
    }

    // ------------------------------------------------------------------
    // test_check_single_block_wide — width > 2000
    // ------------------------------------------------------------------

    #[test]
    fn test_check_single_block_wide() {
        let diag = check_single_block(2001, 1000).expect_err("width > 2000 should fail");
        assert_eq!(diag.code, "LIN-B-001");
        assert!(diag.message.contains("width"));
    }

    // ------------------------------------------------------------------
    // test_check_single_block_tall — height > 2000
    // ------------------------------------------------------------------

    #[test]
    fn test_check_single_block_tall() {
        let diag = check_single_block(1000, 2001).expect_err("height > 2000 should fail");
        assert_eq!(diag.code, "LIN-B-002");
        assert!(diag.message.contains("height"));
    }

    // ------------------------------------------------------------------
    // test_check_payload_budget_ok — 5 blocks, within limits
    // ------------------------------------------------------------------

    #[test]
    fn test_check_payload_budget_ok() {
        let blocks: Vec<ImageBlock> = (0..5)
            .map(|i| {
                ImageBlock::new(
                    500 + i * 100,
                    500 + i * 100,
                    1000 + i * 500,
                    "jpeg".to_string(),
                )
            })
            .collect();
        let report = check_payload_budget(&blocks);
        assert!(report.within_budget, "5 blocks within limits should pass");
        assert_eq!(report.block_count, 5);
        assert!(!report.violations.is_empty() || report.within_budget);
    }

    // ------------------------------------------------------------------
    // test_check_payload_budget_exceeds_blocks — 21 blocks
    // ------------------------------------------------------------------

    #[test]
    fn test_check_payload_budget_exceeds_blocks() {
        let blocks: Vec<ImageBlock> = (0..21)
            .map(|_| ImageBlock::new(500, 500, 1000, "jpeg".to_string()))
            .collect();
        let report = check_payload_budget(&blocks);
        assert!(!report.within_budget, "21 blocks should exceed budget");
        assert!(
            report.violations.iter().any(|v| v.contains("block count")),
            "should have block count violation"
        );
    }

    // ------------------------------------------------------------------
    // test_check_payload_budget_exceeds_size — one block > 2000px
    // ------------------------------------------------------------------

    #[test]
    fn test_check_payload_budget_exceeds_size() {
        let mut blocks: Vec<ImageBlock> = (0..3)
            .map(|i| ImageBlock::new(500 + i * 100, 500 + i * 100, 1000, "jpeg".to_string()))
            .collect();
        blocks.push(ImageBlock::new(3000, 3000, 10000, "jpeg".to_string()));
        let report = check_payload_budget(&blocks);
        assert!(!report.within_budget, "block > 2000px should fail budget");
        assert!(
            report.violations.iter().any(|v| v.contains("per side")),
            "should have size violation"
        );
    }

    // ------------------------------------------------------------------
    // test_check_can_add_ok — within limits
    // ------------------------------------------------------------------

    #[test]
    fn test_check_can_add_ok() {
        let block = ImageBlock::new(500, 500, 1000, "jpeg".to_string());
        let result = check_can_add(5, 100, &block);
        assert!(
            result.is_ok(),
            "should be able to add block when under limit"
        );
        let new_tokens = result.unwrap();
        assert_eq!(new_tokens, 100 + block.visual_tokens);
    }

    // ------------------------------------------------------------------
    // test_check_can_add_exceeds — at limit
    // ------------------------------------------------------------------

    #[test]
    fn test_check_can_add_exceeds() {
        let block = ImageBlock::new(500, 500, 1000, "jpeg".to_string());
        let result = check_can_add(20, 100, &block);
        let diag = result.expect_err("should reject when at block limit");
        assert_eq!(diag.code, "LIN-B-001");
        assert!(diag.message.contains("limit"));
    }

    // ------------------------------------------------------------------
    // test_resize_target_no_change — already within limits
    // ------------------------------------------------------------------

    #[test]
    fn test_resize_target_no_change() {
        let (w, h) = resize_target(1000, 800);
        assert_eq!(w, 1000);
        assert_eq!(h, 800);
    }

    // ------------------------------------------------------------------
    // test_resize_target_wide — wider image
    // ------------------------------------------------------------------

    #[test]
    fn test_resize_target_wide() {
        let (w, h) = resize_target(4000, 2000);
        assert_eq!(w, 2000);
        assert_eq!(h, 1000);
    }

    // ------------------------------------------------------------------
    // test_resize_target_tall — taller image
    // ------------------------------------------------------------------

    #[test]
    fn test_resize_target_tall() {
        let (w, h) = resize_target(2000, 4000);
        assert_eq!(w, 1000);
        assert_eq!(h, 2000);
    }

    // ------------------------------------------------------------------
    // test_resize_target_square_over — square over limit
    // ------------------------------------------------------------------

    #[test]
    fn test_resize_target_square_over() {
        let (w, h) = resize_target(3000, 3000);
        assert_eq!(w, 2000);
        assert_eq!(h, 2000);
    }

    // ------------------------------------------------------------------
    // test_enforce_budget_ok — all within limits
    // ------------------------------------------------------------------

    #[test]
    fn test_enforce_budget_ok() {
        let blocks = vec![
            ImageBlock::new(500, 500, 1000, "jpeg".to_string()),
            ImageBlock::new(800, 600, 2000, "webp".to_string()),
        ];
        let report = enforce_budget(blocks);
        assert!(report.is_ok(), "should succeed for blocks within limits");
    }

    // ------------------------------------------------------------------
    // test_enforce_budget_reject_too_many — too many blocks
    // ------------------------------------------------------------------

    #[test]
    fn test_enforce_budget_reject_too_many() {
        let blocks: Vec<ImageBlock> = (0..21)
            .map(|_| ImageBlock::new(500, 500, 1000, "jpeg".to_string()))
            .collect();
        let result = enforce_budget(blocks);
        let diag = result.expect_err("should reject too many blocks");
        assert_eq!(diag.code, "LIN-B-002");
        assert!(diag.message.contains("exceeds budget"));
    }

    // ------------------------------------------------------------------
    // test_enforce_budget_clamps_size — oversized blocks clamped
    // ------------------------------------------------------------------

    #[test]
    fn test_enforce_budget_clamps_size() {
        let blocks = vec![ImageBlock::new(4000, 2000, 5000, "jpeg".to_string())];
        let report = enforce_budget(blocks).expect("clamp should succeed");
        assert!(report.within_budget);
        // After clamping: width=2000, height=2000*(2000/4000)=1000
        assert_eq!(report.block_count, 1);
    }

    // ------------------------------------------------------------------
    // test_clamp_dimension — table-driven
    // ------------------------------------------------------------------

    #[test]
    fn test_clamp_dimension() {
        let table: [(usize, usize); 3] = [(100, 100), (2000, 2000), (3000, 2000)];
        for &(input, expected) in &table {
            let result = clamp_dimension(input);
            assert_eq!(
                result, expected,
                "clamp_dimension({}) = {}, expected {}",
                input, result, expected
            );
        }
    }

    // ------------------------------------------------------------------
    // test_budget_report_serde — roundtrip
    // ------------------------------------------------------------------

    #[test]
    fn test_budget_report_serde() {
        let report = BudgetReport::new(5, 1064, 100000, true);
        let json = serde_json::to_string(&report).unwrap();
        let back: BudgetReport = serde_json::from_str(&json).unwrap();
        assert_eq!(report.block_count, back.block_count);
        assert_eq!(report.total_tokens, back.total_tokens);
        assert_eq!(report.total_bytes, back.total_bytes);
        assert_eq!(report.within_budget, back.within_budget);
        assert_eq!(report.violations, back.violations);
    }

    // ------------------------------------------------------------------
    // test_image_block_serde — roundtrip
    // ------------------------------------------------------------------

    #[test]
    fn test_image_block_serde() {
        let block = ImageBlock::new(768, 1024, 50000, "jpeg".to_string());
        let json = serde_json::to_string(&block).unwrap();
        let back: ImageBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block.width, back.width);
        assert_eq!(block.height, back.height);
        assert_eq!(block.visual_tokens, back.visual_tokens);
        assert_eq!(block.byte_size, back.byte_size);
        assert_eq!(block.format, back.format);
    }

    // ------------------------------------------------------------------
    // test_enforce_budget_with_clamped_tokens
    // ------------------------------------------------------------------

    #[test]
    fn test_enforce_budget_with_clamped_tokens() {
        // 3000x3000 -> clamped to 2000x2000 -> tokens = 72*72 = 5184
        let blocks = vec![ImageBlock::new(3000, 3000, 100000, "jpeg".to_string())];
        let report = enforce_budget(blocks).expect("should clamp and succeed");
        assert!(report.within_budget);
        // Check that visual_tokens is recomputed after clamping
        assert_eq!(report.total_tokens, 72 * 72);
    }

    // ------------------------------------------------------------------
    // test_check_single_block_at_boundary — exactly at 2000
    // ------------------------------------------------------------------

    #[test]
    fn test_check_single_block_at_boundary() {
        let result = check_single_block(2000, 2000);
        assert!(
            result.is_ok(),
            "2000x2000 should be at the boundary and pass"
        );
    }

    // ------------------------------------------------------------------
    // test_enforce_budget_empty
    // ------------------------------------------------------------------

    #[test]
    fn test_enforce_budget_empty() {
        let blocks: Vec<ImageBlock> = vec![];
        let report = enforce_budget(blocks).expect("empty list should succeed");
        assert!(report.within_budget);
        assert_eq!(report.block_count, 0);
        assert_eq!(report.total_tokens, 0);
    }
}
