//! Font analysis: reference font set, substitution detection, and missing-glyph (tofu) heuristics.
//!
//! LIN-02: font substitution + missing-glyph checks (Wave D, P1-34)

/// Reference font set: fonts known to be available on the reference platform.
/// Fonts outside this set may cause pagination drift on Linux.
pub const REFERENCE_FONTS: &[&str] = &[
    "Calibri",
    "Carlito",
    "Cambria",
    "Caladea",
    "Arial",
    "Liberation Sans",
    "Times New Roman",
    "Liberation Serif",
    "Courier New",
    "Liberation Mono",
    "Consolas",
    "Noto Sans Mono",
    "Noto Sans",
    "Noto Serif",
    "Noto Sans Mono",
];

/// A font substitution mapping: requested font -> actual font used.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FontSubstitution {
    /// The font that was requested but not found in the reference set.
    pub requested: String,
    /// The font that is being used as a substitute.
    pub used: String,
}

/// A missing glyph (tofu) report entry.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MissingGlyphReport {
    /// The font that was requested.
    pub font: String,
    /// Characters that may not render in this font.
    pub missing_chars: Vec<String>,
    /// Text content affected.
    pub text: String,
}

/// Substitution mapping for known non-reference fonts.
/// Maps common Windows/Mac fonts to their Linux metric-compatible substitutes.
const SUBSTITUTION_MAP: &[(&str, &str)] = &[
    ("Segoe UI", "Liberation Sans"),
    ("Georgia", "Liberation Serif"),
    ("Verdana", "Liberation Sans"),
    ("Webdings", "Carlito"),
    ("symbol", "Liberation Serif"),
    ("Tahoma", "Liberation Sans"),
    ("Trebuchet MS", "Liberation Sans"),
    ("Comic Sans MS", "Liberation Sans"),
    ("Impact", "Liberation Sans"),
    ("Lucida Console", "Liberation Mono"),
    ("Consolas", "Consolas"),
    ("Arial Narrow", "Liberation Sans"),
    ("Book Antiqua", "Liberation Serif"),
    ("Palatino Linotype", "Liberation Serif"),
    ("Times New Roman", "Times New Roman"),
    ("Arial", "Arial"),
    ("Calibri", "Calibri"),
    ("Cambria", "Cambria"),
];

/// CJK-capable font prefixes for tofu risk detection.
const CJK_FONT_PATTERNS: &[&str] = &[
    "Noto Sans CJK",
    "Noto Serif CJK",
    "Source Han Sans",
    "Source Han Serif",
    "SimSun",
    "SimHei",
    "Microsoft YaHei",
    "Microsoft JhengHei",
    "PingFang",
    "Hiragino Sans",
    "Yu Gothic",
    "Meiryo",
    "Malgun Gothic",
    "MS Gothic",
    "MS Mincho",
    "Kozuka",
    "Dokyi",
];

/// Arabic-capable font prefixes.
const ARABIC_FONT_PATTERNS: &[&str] = &[
    "Arabic Typesetting",
    "Simplified Arabic",
    "Traditional Arabic",
];

/// Check which fonts from the provided list are NOT in the reference set.
pub fn check_unknown_fonts(used_fonts: &[String]) -> Vec<String> {
    let ref_set: std::collections::HashSet<&str> = REFERENCE_FONTS.iter().copied().collect();
    let mut unknown = Vec::new();
    for font in used_fonts {
        if !ref_set.contains(font.as_str()) && !unknown.contains(font) {
            unknown.push(font.clone());
        }
    }
    unknown
}

/// Detect potential font substitutions: if a non-reference font is used,
/// suggest the reference substitute.
pub fn detect_substitutions(used_fonts: &[String]) -> Vec<FontSubstitution> {
    let ref_set: std::collections::HashSet<&str> = REFERENCE_FONTS.iter().copied().collect();
    let mut substitutions = Vec::new();
    for font in used_fonts {
        if ref_set.contains(font.as_str()) {
            continue;
        }
        // Look up in substitution map
        let used = SUBSTITUTION_MAP
            .iter()
            .find_map(|(known, substitute)| {
                if known.eq_ignore_ascii_case(font.as_str()) {
                    Some(substitute.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| fallback_substitute(font));
        substitutions.push(FontSubstitution {
            requested: font.clone(),
            used,
        });
    }
    substitutions
}

/// Fallback substitute for fonts not in the substitution map.
fn fallback_substitute(font: &str) -> String {
    let lower = font.to_lowercase();
    // Check for CJK fonts
    for pattern in CJK_FONT_PATTERNS {
        if lower.contains(&pattern.to_lowercase()) {
            return "Noto Sans CJK".to_string();
        }
    }
    // Check for Arabic fonts
    for pattern in ARABIC_FONT_PATTERNS {
        if lower.contains(&pattern.to_lowercase()) {
            return "Arial".to_string();
        }
    }
    // Generic fallback: sans-serif -> Liberation Sans, serif -> Liberation Serif
    if lower.contains("sans") {
        return "Liberation Sans".to_string();
    }
    if lower.contains("serif") {
        return "Liberation Serif".to_string();
    }
    if lower.contains("mono") || lower.contains("courier") {
        return "Liberation Mono".to_string();
    }
    // Default fallback
    "Liberation Sans".to_string()
}

/// Check for potential tofu (missing glyph) issues.
/// Heuristic: CJK/Arabic/RTL text rendered with non-CJK/Arabic fonts -> potential tofu.
pub fn detect_tofu_risks(
    text_bboxes: &[gp_core::ir::TextWithBbox],
    fonts: &[String],
) -> Vec<MissingGlyphReport> {
    let mut reports = Vec::new();

    // Determine which fonts are available for each text element (use first non-ref font)
    let available_fonts: Vec<&str> = fonts.iter().map(|s| s.as_str()).collect();

    for tb in text_bboxes {
        let text = &tb.text;
        if text.is_empty() {
            continue;
        }

        // Check if text contains CJK characters
        let cjk_chars: Vec<String> = text
            .chars()
            .filter(|c| is_cjk(*c))
            .map(|c| c.to_string())
            .collect();

        // Check if text contains Arabic characters
        let arabic_chars: Vec<String> = text
            .chars()
            .filter(|c| is_arabic(*c))
            .map(|c| c.to_string())
            .collect();

        // Check if text has RTL characters
        let rtl_chars: Vec<String> = text
            .chars()
            .filter(|c| is_rtl(*c))
            .map(|c| c.to_string())
            .collect();

        // If there are CJK/Arabic/RTL chars, check if any used font can handle them
        let has_cjk = !cjk_chars.is_empty();
        let has_arabic = !arabic_chars.is_empty();
        let has_rtl = !rtl_chars.is_empty();

        if !has_cjk && !has_arabic && !has_rtl {
            continue;
        }

        // Check if any available font is CJK/Arabic capable
        let has_cjk_font = available_fonts.iter().any(|f| is_cjk_capable(f));
        let has_arabic_font = available_fonts.iter().any(|f| is_arabic_capable(f));

        // If CJK text but no CJK font -> risk (as long as we have some fonts to check)
        if has_cjk && !has_cjk_font && !available_fonts.is_empty() {
            reports.push(MissingGlyphReport {
                font: available_fonts[0].to_string(),
                missing_chars: cjk_chars,
                text: text.clone(),
            });
        }

        // If Arabic/RTL text but no Arabic font -> risk (as long as we have some fonts to check)
        if (has_arabic || has_rtl) && !has_arabic_font && !available_fonts.is_empty() {
            reports.push(MissingGlyphReport {
                font: available_fonts[0].to_string(),
                missing_chars: if !arabic_chars.is_empty() {
                    arabic_chars
                } else {
                    rtl_chars
                },
                text: text.clone(),
            });
        }
    }

    reports
}

/// Check if a character is CJK.
fn is_cjk(c: char) -> bool {
    let code = c as u32;
    // CJK Unified Ideographs, Extension A/B/C/D/E/F, Hiragana, Katakana, Hangul
    (0x4E00..=0x9FFF).contains(&code) // CJK Unified
        || (0x3400..=0x4DBF).contains(&code) // Extension A
        || (0x20000..=0x2A6DF).contains(&code) // Extension B
        || (0x2A700..=0x2B73F).contains(&code) // Extension C
        || (0x2B740..=0x2B81F).contains(&code) // Extension D
        || (0x2B820..=0x2CEAF).contains(&code) // Extension E
        || (0x2CEB0..=0x2EBEF).contains(&code) // Extension F
        || (0x3000..=0x303F).contains(&code) // CJK Symbols
        || (0x3040..=0x309F).contains(&code) // Hiragana
        || (0x30A0..=0x30FF).contains(&code) // Katakana
        || (0xAC00..=0xD7AF).contains(&code) // Hangul Syllables
        || (0x1100..=0x11FF).contains(&code) // Hangul Jamo
        || (0x2E80..=0x2EFF).contains(&code) // CJK Radicals
        || (0x3100..=0x312F).contains(&code) // Bopomofo
        || (0x31A0..=0x31BF).contains(&code) // Bopomofo Extended
        || (0xF900..=0xFAFF).contains(&code) // CJK Compatibility Ideographs
        || (0xFE30..=0xFE4F).contains(&code) // CJK Compatibility Forms
}

/// Check if a character is Arabic.
fn is_arabic(c: char) -> bool {
    let code = c as u32;
    (0x0600..=0x06FF).contains(&code) // Arabic
        || (0x0750..=0x077F).contains(&code) // Arabic Extended-B
        || (0x08A0..=0x08FF).contains(&code) // Arabic Extended-A
        || (0xFB50..=0xFDFF).contains(&code) // Arabic Presentation Forms-A
        || (0xFE70..=0xFEFF).contains(&code) // Arabic Presentation Forms-B
}

/// Check if a character is RTL (Right-to-Left) beyond Arabic.
fn is_rtl(c: char) -> bool {
    let code = c as u32;
    // Hebrew
    (0x0590..=0x05FF).contains(&code)
        // Hebrew Extended (for future use)
        || (0x10800..=0x1083F).contains(&code)
        // Thaana
        || (0x0780..=0x07BF).contains(&code)
        // Syriac
        || (0x0700..=0x074F).contains(&code)
        // N'Ko
        || (0x07C0..=0x07FF).contains(&code)
        // Phoenician
        || (0x10900..=0x1091F).contains(&code)
        // Lydian
        || (0x10920..=0x1093F).contains(&code)
        // Mende Kikakad
        || (0x0E800..=0x0EDFF).contains(&code)
        // Adlam
        || (0x1E900..=0x1E95F).contains(&code)
        // Samaritan
        || (0x000FB00..=0x000FB4F).contains(&code)
}

/// Check if a font name indicates CJK capability.
fn is_cjk_capable(font: &str) -> bool {
    let lower = font.to_lowercase();
    CJK_FONT_PATTERNS
        .iter()
        .any(|pattern| lower.contains(&pattern.to_lowercase()))
}

/// Check if a font name indicates Arabic capability.
fn is_arabic_capable(font: &str) -> bool {
    let lower = font.to_lowercase();
    ARABIC_FONT_PATTERNS
        .iter()
        .any(|pattern| lower.contains(&pattern.to_lowercase()))
        || lower.contains("arial")
        || lower.contains("times")
        || lower.contains("calibri")
        || lower.contains("cambria")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_core::ir::{ElementId, TextWithBbox as LinterTextWithBbox};

    fn make_tb(text: &str) -> LinterTextWithBbox {
        LinterTextWithBbox {
            id: ElementId::content_anchored("text", &["text:0"], text),
            text: text.to_string(),
            bbox: [0.0, 0.0, 100.0, 20.0],
        }
    }

    // ------------------------------------------------------------------
    // Reference font tests
    // ------------------------------------------------------------------

    #[test]
    fn test_reference_fonts_are_all_known() {
        // All fonts in REFERENCE_FONTS should be in themselves
        let ref_set: std::collections::HashSet<&str> = REFERENCE_FONTS.iter().copied().collect();
        for font in REFERENCE_FONTS {
            assert!(
                ref_set.contains(font),
                "Reference font '{}' should be in itself",
                font
            );
        }
    }

    #[test]
    fn test_unknown_font_detected() {
        let fonts = vec!["Segoe UI".into(), "Calibri".into(), "Comic Sans MS".into()];
        let unknown = check_unknown_fonts(&fonts);
        assert!(
            unknown.contains(&"Segoe UI".to_string()),
            "Segoe UI should be unknown, got: {:?}",
            unknown
        );
        assert!(
            unknown.contains(&"Comic Sans MS".to_string()),
            "Comic Sans MS should be unknown, got: {:?}",
            unknown
        );
        assert!(
            !unknown.contains(&"Calibri".to_string()),
            "Calibri is a reference font and should not be unknown"
        );
    }

    #[test]
    fn test_substitution_detected() {
        let fonts = vec!["Segoe UI".into()];
        let subs = detect_substitutions(&fonts);
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].requested, "Segoe UI");
        assert_eq!(subs[0].used, "Liberation Sans");
    }

    #[test]
    fn test_cjk_tofu_risk() {
        let text_bboxes = vec![make_tb("こんにちは世界")];
        let fonts = vec!["Calibri".into(), "Arial".into()];
        let reports = detect_tofu_risks(&text_bboxes, &fonts);
        assert!(
            !reports.is_empty(),
            "CJK text with non-CJK fonts should produce tofu risk"
        );
        assert!(
            reports[0]
                .missing_chars
                .iter()
                .any(|c| c.contains('こ') || c.contains('に') || c.contains('ち')),
            "Missing chars should include CJK: {:?}",
            reports[0].missing_chars
        );
    }

    #[test]
    fn test_no_tofu_risk_cjk_font() {
        let text_bboxes = vec![make_tb("こんにちは世界")];
        let fonts = vec!["Noto Sans CJK JP".into(), "Calibri".into()];
        let reports = detect_tofu_risks(&text_bboxes, &fonts);
        assert!(
            reports.is_empty(),
            "CJK text with CJK-capable font should not produce tofu risk, got: {:?}",
            reports
        );
    }

    #[test]
    fn test_latin_no_risk() {
        let text_bboxes = vec![make_tb("Hello, World!")];
        let fonts = vec!["Calibri".into()];
        let reports = detect_tofu_risks(&text_bboxes, &fonts);
        assert!(
            reports.is_empty(),
            "Latin text should not produce tofu risk, got: {:?}",
            reports
        );
    }

    #[test]
    fn test_detect_substitutions_empty() {
        let fonts: Vec<String> = vec![];
        let subs = detect_substitutions(&fonts);
        assert!(subs.is_empty(), "No fonts -> no substitutions");
    }

    #[test]
    fn test_all_known_fonts_no_substitution() {
        let fonts = vec![
            "Calibri".into(),
            "Arial".into(),
            "Times New Roman".into(),
            "Liberation Sans".into(),
        ];
        let subs = detect_substitutions(&fonts);
        assert!(
            subs.is_empty(),
            "All reference fonts should produce no substitution, got: {:?}",
            subs
        );
    }

    #[test]
    fn test_serde_roundtrip_font_substitution() {
        let sub = FontSubstitution {
            requested: "Segoe UI".to_string(),
            used: "Liberation Sans".to_string(),
        };
        let json = serde_json::to_string(&sub).unwrap();
        let back: FontSubstitution = serde_json::from_str(&json).unwrap();
        assert_eq!(sub, back);
        assert_eq!(back.requested, "Segoe UI");
        assert_eq!(back.used, "Liberation Sans");
    }

    #[test]
    fn test_serde_roundtrip_missing_glyph_report() {
        let report = MissingGlyphReport {
            font: "Calibri".to_string(),
            missing_chars: vec!["こ".to_string(), "に".to_string()],
            text: "こんにちは".to_string(),
        };
        let json = serde_json::to_string(&report).unwrap();
        let back: MissingGlyphReport = serde_json::from_str(&json).unwrap();
        assert_eq!(report, back);
        assert_eq!(back.font, "Calibri");
        assert_eq!(back.missing_chars.len(), 2);
        assert_eq!(back.text, "こんにちは");
    }

    // ------------------------------------------------------------------
    // Substitution map tests
    // ------------------------------------------------------------------

    #[test]
    fn test_substitution_map_variants() {
        // Test multiple known Windows fonts
        let fonts = vec![
            "Georgia".into(),
            "Verdana".into(),
            "Webdings".into(),
            "Tahoma".into(),
        ];
        let subs = detect_substitutions(&fonts);
        assert_eq!(subs.len(), 4);

        let requests: Vec<&str> = subs.iter().map(|s| s.requested.as_str()).collect();
        assert!(requests.contains(&"Georgia"));
        assert!(requests.contains(&"Verdana"));
        assert!(requests.contains(&"Webdings"));
        assert!(requests.contains(&"Tahoma"));

        let uses: Vec<&str> = subs.iter().map(|s| s.used.as_str()).collect();
        assert!(uses.contains(&"Liberation Serif"));
        assert!(uses.contains(&"Liberation Sans"));
        assert!(uses.contains(&"Carlito"));
    }

    #[test]
    fn test_fallback_substitute_cjk() {
        let result = fallback_substitute("MS Gothic");
        assert_eq!(result, "Noto Sans CJK");
    }

    #[test]
    fn test_fallback_substitute_generic() {
        let result = fallback_substitute("MyCustomFont");
        assert_eq!(result, "Liberation Sans");
    }

    #[test]
    fn test_fallback_substitute_serif() {
        let result = fallback_substitute("My Serif Font");
        assert_eq!(result, "Liberation Serif");
    }

    #[test]
    fn test_fallback_substitute_mono() {
        let result = fallback_substitute("My Mono Font");
        assert_eq!(result, "Liberation Mono");
    }

    // ------------------------------------------------------------------
    // Character detection tests
    // ------------------------------------------------------------------

    #[test]
    fn test_is_cjk_hiragana() {
        assert!(is_cjk('あ'));
        assert!(is_cjk('こ'));
        assert!(is_cjk('に'));
    }

    #[test]
    fn test_is_cjk_kanji() {
        assert!(is_cjk('世'));
        assert!(is_cjk('界'));
        assert!(is_cjk('日'));
    }

    #[test]
    fn test_is_cjk_katakana() {
        assert!(is_cjk('ア'));
        assert!(is_cjk('カ'));
    }

    #[test]
    fn test_is_cjk_hangul() {
        assert!(is_cjk('한'));
        assert!(is_cjk('글'));
    }

    #[test]
    fn test_is_cjk_latin() {
        assert!(!is_cjk('a'));
        assert!(!is_cjk('Z'));
        assert!(!is_cjk('0'));
    }

    #[test]
    fn test_is_arabic() {
        assert!(is_arabic('ا')); // Arabic Alef
        assert!(is_arabic('ب')); // Arabic Ba
    }

    #[test]
    fn test_is_arabic_latin() {
        assert!(!is_arabic('a'));
        assert!(!is_arabic('A'));
    }

    #[test]
    fn test_is_rtl_hebrew() {
        assert!(is_rtl('א')); // Hebrew Alef
        assert!(is_rtl('ב')); // Hebrew Bet
    }

    #[test]
    fn test_is_rtl_latin() {
        assert!(!is_rtl('a'));
        assert!(!is_rtl('Z'));
    }

    #[test]
    fn test_is_cjk_capable_font() {
        assert!(is_cjk_capable("Noto Sans CJK JP"));
        assert!(is_cjk_capable("Source Han Sans SC"));
        assert!(is_cjk_capable("Microsoft YaHei"));
        assert!(!is_cjk_capable("Calibri"));
        assert!(!is_cjk_capable("Arial"));
    }

    #[test]
    fn test_is_arabic_capable_font() {
        assert!(is_arabic_capable("Arial"));
        assert!(is_arabic_capable("Times New Roman"));
        assert!(is_arabic_capable("Calibri"));
        assert!(is_arabic_capable("Simplified Arabic"));
        assert!(!is_arabic_capable("Noto Sans CJK JP"));
    }

    // ------------------------------------------------------------------
    // Empty/edge case tests
    // ------------------------------------------------------------------

    #[test]
    fn test_tofu_risks_empty_text_bboxes() {
        let text_bboxes: Vec<LinterTextWithBbox> = vec![];
        let fonts = vec!["Calibri".into()];
        let reports = detect_tofu_risks(&text_bboxes, &fonts);
        assert!(reports.is_empty());
    }

    #[test]
    fn test_tofu_risks_empty_fonts() {
        let text_bboxes = vec![make_tb("こんにちは")];
        let fonts: Vec<String> = vec![];
        let reports = detect_tofu_risks(&text_bboxes, &fonts);
        // No fonts available, so has_ref_font is false -> no risk reported
        assert!(reports.is_empty());
    }

    #[test]
    fn test_tofu_risks_empty_text() {
        let text_bboxes = vec![make_tb("")];
        let fonts = vec!["Calibri".into()];
        let reports = detect_tofu_risks(&text_bboxes, &fonts);
        assert!(reports.is_empty());
    }

    #[test]
    fn test_arabic_tofu_risk() {
        let text_bboxes = vec![make_tb("مرحبا بالعالم")];
        let fonts = vec!["Noto Sans CJK JP".into()];
        let reports = detect_tofu_risks(&text_bboxes, &fonts);
        assert!(
            !reports.is_empty(),
            "Arabic text with CJK-only font should produce tofu risk"
        );
    }

    #[test]
    fn test_arabic_no_risk_with_arabic_font() {
        let text_bboxes = vec![make_tb("مرحبا بالعالم")];
        let fonts = vec!["Arial".into(), "Noto Sans CJK JP".into()];
        let reports = detect_tofu_risks(&text_bboxes, &fonts);
        assert!(
            reports.is_empty(),
            "Arabic text with Arial (Arabic-capable) should not produce tofu risk"
        );
    }

    #[test]
    fn test_check_unknown_fonts_duplicates_ignored() {
        let fonts = vec!["Segoe UI".into(), "Segoe UI".into(), "Calibri".into()];
        let unknown = check_unknown_fonts(&fonts);
        assert_eq!(
            unknown.len(),
            1,
            "Duplicate unknown fonts should be deduplicated"
        );
        assert_eq!(unknown[0], "Segoe UI");
    }

    #[test]
    fn test_detect_substitutions_duplicate_unknown_ignored() {
        let fonts = vec!["Georgia".into(), "Georgia".into()];
        let subs = detect_substitutions(&fonts);
        assert_eq!(
            subs.len(),
            2,
            "Each font occurrence gets a substitution entry"
        );
        assert_eq!(subs[0].requested, "Georgia");
        assert_eq!(subs[0].used, "Liberation Serif");
    }

    #[test]
    fn test_reference_fonts_non_empty() {
        assert!(
            !REFERENCE_FONTS.is_empty(),
            "Reference fonts list should not be empty"
        );
        assert_eq!(REFERENCE_FONTS.len(), 15);
    }
}
