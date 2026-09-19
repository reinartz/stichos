# Implementation Guide: Context Budget & Response Size Management

**Contract:** [`C14-context-budget.md`](./contracts/C14-context-budget.md)
**Version:** 1.0.0
**Status:** Draft

This guide provides implementation details for the changes described in C14.

---

## 1. New Types and Enums

### 1.1 Render Output Mode

```rust
// In crates/gp-core/src/traits.rs or a new module

/// How to return rendered images.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderOutput {
    /// Return base64-encoded image blocks (current behavior).
    Base64,
    /// Write images to temp files and return URLs/paths.
    Urls,
    /// Return both URLs and inline base64.
    Both,
}
```

### 1.2 Inspection Detail Level

```rust
/// How much detail to return from inspect_document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectDetail {
    /// Summary only: page counts, element counts, IDs. (~150 tokens)
    Summary,
    /// Elements mode: up to max_elements full element records. (~5000 tokens)
    Elements,
    /// All: full coordinates, styles, fidelity budget per element. (unlimited)
    All,
}
```

### 1.3 Budget Metadata

```rust
/// Token budget metadata included in every response.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ResponseBudget {
    /// Estimated token cost of this response.
    pub tokens_est: usize,
    /// Number of elements/data items returned.
    pub elements_count: usize,
    /// Number of images in the response.
    pub image_count: usize,
}

/// Common response envelope — add to all tool responses.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EnvelopedResponse {
    /// Tool-specific data.
    #[serde(flatten)]
    pub inner: serde_json::Value,
    /// Token budget metadata.
    pub __budget: ResponseBudget,
    /// C3-coded diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// Engine identifier.
    pub rendered_by: String,
}
```

---

## 2. Temp File Management

### 2.1 Temp Directory

Images written via `output="urls"` go to a project-local temp directory:

```
$XDG_RUNTIME_DIR/stichos/renders/   (or /tmp/stichos/ on systems without XDG)
```

### 2.2 File Naming Convention

```
{prefix}_{hash}_{page}.png
```

- `prefix`: `render`, `region`, `annot_m` (marked), `annot_c` (clean)
- `hash`: First 12 characters of the SHA-256 of `path + page_range + dpi + seed`
- `page`: Page number

Example: `render_abc123def456_1.png`

### 2.3 Cleanup

Two cleanup mechanisms:

1. **TTL-based**: Files older than 1 hour are eligible for deletion.
2. **Startup cleanup**: On server start, delete all files in the temp directory.

**Implementation:**

```rust
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub struct TempFileManager {
    base_dir: PathBuf,
    cleanup_interval: Duration,
    last_cleanup: Instant,
}

impl TempFileManager {
    pub fn new(base_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&base_dir).ok();
        Self {
            base_dir,
            cleanup_interval: Duration::from_secs(3600), // 1 hour
            last_cleanup: Instant::now(),
        }
    }

    /// Create a temp file path with a unique name.
    pub fn create(&self, prefix: &str, page: usize) -> PathBuf {
        let hash = format!("{:x}", std::panic::AssertUnwindSafe(
            &std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap()
        )).chars().take(12).collect::<String>();
        
        let filename = format!("{}_{}_{}.png", prefix, hash, page);
        self.base_dir.join(filename)
    }

    /// Cleanup files older than 1 hour.
    pub fn maybe_cleanup(&mut self) {
        if self.last_cleanup.elapsed() < self.cleanup_interval {
            return;
        }
        self.last_cleanup = Instant::now();
        
        if let Ok(entries) = std::fs::read_dir(&self.base_dir) {
            let cutoff = std::time::SystemTime::now() - Duration::from_secs(3600);
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if let Ok(created) = metadata.created() {
                        if created < cutoff {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
            }
        }
    }
}
```

### 2.4 Expiration in Responses

Every URL response includes:

```json
{
  "url": "/tmp/stichos/abc123.png",
  "expiresAt": "2026-09-19T15:00:00Z"
}
```

The `expiresAt` is `now + 1 hour`.

---

## 3. Handler Implementation

### 3.1 `inspect_document` Handler

```rust
async fn handle_inspect_document(args: &Value) -> Result<Value, String> {
    let path = args["path"].as_str().ok_or("missing path")?;
    let detail: InspectDetail = args["detail"]
        .as_str()
        .map(|s| s.parse().unwrap_or(InspectDetail::Summary))
        .unwrap_or(InspectDetail::Summary);
    let max_elements: usize = args["max_elements"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(100);

    // Parse the document
    let doc = parse_document(path)?;
    
    match detail {
        InspectDetail::Summary => {
            // Build summary response
            let page_stats = doc.pages.iter().map(|p| {
                json!({
                    "page": p.number,
                    "elementCount": p.elements.len(),
                    "hasImages": p.elements.iter().any(|e| matches!(e, Element::Image(_))),
                    "hasTables": p.elements.iter().any(|e| matches!(e, Element::Table(_))),
                })
            }).collect::<Vec<_>>();
            
            let element_ids: Vec<String> = doc.elements.iter()
                .take(max_elements)
                .map(|e| e.id.clone())
                .collect();
            
            Ok(json!({
                "summary": {
                    "pageCount": doc.pages.len(),
                    "wordCount": doc.word_count(),
                    "elementCount": doc.elements.len(),
                    "hasImages": doc.elements.iter().any(|e| matches!(e, Element::Image(_))),
                    "hasTables": doc.elements.iter().any(|e, matches!(e, Element::Table(_))),
                    "hasComments": false, // TODO: implement
                    "hasTrackedRevisions": false, // TODO: implement
                },
                "pages": page_stats,
                "elementIds": element_ids,
                "hasMore": doc.elements.len() > max_elements,
            }))
        }
        InspectDetail::Elements => {
            // Build elements response
            let elements: Vec<Value> = doc.elements.iter()
                .take(max_elements)
                .map(|e| element_to_json(e))
                .collect();
            
            Ok(json!({
                "summary": {
                    "pageCount": doc.pages.len(),
                    "wordCount": doc.word_count(),
                    "elementCount": doc.elements.len(),
                },
                "elements": elements,
                "hasMore": doc.elements.len() > max_elements,
            }))
        }
        InspectDetail::All => {
            // Current behavior: full response
            build_full_inspect_response(&doc)
        }
    }
}

fn element_to_json(e: &Element) -> Value {
    match e {
        Element::Paragraph(p) => json!({
            "id": p.id,
            "kind": "paragraph",
            "text": p.text,
            "page": p.page,
            "bbox": p.bbox,
        }),
        Element::Image(i) => json!({
            "id": i.id,
            "kind": "image",
            "page": i.page,
            "bbox": i.bbox,
            "width": i.width,
            "height": i.height,
        }),
        // ... other element kinds
        _ => json!({ "id": e.id, "kind": "unknown" }),
    }
}
```

### 3.2 `render_document` Handler

```rust
async fn handle_render_document(args: &Value) -> Result<Value, String> {
    let path = args["path"].as_str().ok_or("missing path")?;
    let dpi: u32 = args["dpi"].as_u64().map(|n| n as u32).unwrap_or(150);
    let output: RenderOutput = args["output"]
        .as_str()
        .map(|s| s.parse().unwrap_or(RenderOutput::Urls))
        .unwrap_or(RenderOutput::Urls);
    let max_pages: usize = args["max_pages"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(3);
    let page_range: Option<Vec<usize>> = args["page_range"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_u64().map(|n| n as usize)).collect());

    let doc = parse_document(path)?;
    let total_pages = doc.pages.len();
    
    // Determine which pages to render
    let pages_to_render = match page_range {
        Some(range) => range
            .into_iter()
            .filter(|p| *p >= 1 && *p <= total_pages)
            .take(max_pages)
            .collect::<Vec<_>>(),
        None => (1..=total_pages.min(max_pages)).collect(),
    };

    let temp_mgr = get_temp_manager();
    let mut rendered_pages = Vec::new();
    
    for page_num in &pages_to_render {
        let page = doc.pages.iter().find(|p| p.number == *page_num)
            .ok_or(format!("page {} not found", page_num))?;
        
        let rendered = render_page(page, dpi)?;
        
        match output {
            RenderOutput::Base64 | RenderOutput::Both => {
                rendered_pages.push(json!({
                    "page": page_num,
                    "width": rendered.width,
                    "height": rendered.height,
                    "base64": rendered.to_base64(),
                }));
            }
            RenderOutput::Urls | RenderOutput::Both => {
                let url = temp_mgr.create("render", *page_num);
                std::fs::write(&url, rendered.as_bytes())?;
                
                rendered_pages.push(json!({
                    "page": page_num,
                    "width": rendered.width,
                    "height": rendered.height,
                    "url": url.to_string_lossy().to_string(),
                    "expiresAt": now_plus_1hour(),
                }));
            }
        }
    }

    Ok(json!({
        "pages": rendered_pages,
        "totalPages": total_pages,
        "renderedPages": rendered_pages.len(),
        "dpi": dpi,
        "hint": if rendered_pages.len() < total_pages {
            format!("Use page_range to render additional pages. Max {} rendered.", max_pages)
        } else {
            String::new()
        },
    }))
}
```

### 3.3 `diff_documents` Handler

```rust
async fn handle_diff_documents(args: &Value) -> Result<Value, String> {
    let path_a = args["path_a"].as_str().ok_or("missing path_a")?;
    let path_b = args["path_b"].as_str().ok_or("missing path_b")?;
    let max_ops: usize = args["max_ops"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(20);  // NEW: default is 20, not unlimited

    let doc_a = parse_document(path_a)?;
    let doc_b = parse_document(path_b)?;
    
    // Compute full diff
    let full_diff = compute_ir_diff(&doc_a, &doc_b)?;
    let total_ops = full_diff.operations.len();
    
    // Take up to max_ops
    let ops_to_return = full_diff.operations.iter().take(max_ops).cloned().collect::<Vec<_>>();
    let has_more = total_ops > max_ops;
    
    // Build summary
    let summary = DiffSummary {
        pages_added: full_diff.pages_added,
        pages_removed: full_diff.pages_removed,
        elements_modified: full_diff.operations.iter()
            .filter(|op| matches!(op, DiffOp::Replace { .. }))
            .count(),
        elements_added: full_diff.operations.iter()
            .filter(|op| matches!(op, DiffOp::Insert { .. }))
            .count(),
        elements_deleted: full_diff.operations.iter()
            .filter(|op| matches!(op, DiffOp::Delete { .. }))
            .count(),
        total_operations: total_ops,
    };
    
    Ok(json!({
        "summary": summary,
        "operations": ops_to_return,
        "hasMore": has_more,
        "totalOperations": total_ops,
    }))
}
```

### 3.4 `apply_patch` Handler (dry_run default change)

```rust
async fn handle_apply_patch(args: &Value) -> Result<Value, String> {
    let path = args["path"].as_str().ok_or("missing path")?;
    let ops: Vec<PatchOp> = serde_json::from_value(args["ops"].clone())?;
    let dry_run: bool = args["dry_run"]
        .as_bool()
        .unwrap_or(true);  // NEW: default is true for LLM safety
    
    // ... rest of implementation
    
    if dry_run {
        // Build preview (limited to 20 operations)
        let preview_ops: Vec<Value> = ops.iter().take(20).map(|op| {
            patch_op_to_json(op)
        }).collect();
        
        let estimated_tokens = preview_ops.len() * 15; // rough estimate
        
        Ok(json!({
            "preview": {
                "estimatedTokenChange": format!("+{}", estimated_tokens),
                "elementsAffected": ops.len(),
                "diffSummary": format!("{} operations", ops.len()),
                "operations": preview_ops,
            },
            "fidelity_budget": compute_fidelity_budget(&doc_before, &doc_after)?,
        }))
    } else {
        // Apply patch and return full document
        apply_and_return(path, ops)
    }
}
```

---

## 4. CLI Integration

### 4.1 CLI Arguments

The CLI should expose the new parameters with sensible defaults that match the tool defaults:

```bash
# inspect_document
stichos inspect path/to/doc.docx --detail summary    # default
stichos inspect path/to/doc.docx --detail elements    # opt-in
stichos inspect path/to/doc.docx --detail all         # full (old behavior)

# render_document
stichos render path/to/doc.docx                       # default: urls, 3 pages
stichos render path/to/doc.docx --output base64       # old behavior
stichos render path/to/doc.docx --max-pages 10        # more pages

# diff_documents
stichos diff doc-a.docx doc-b.docx                   # default: max 20 ops
stichos diff doc-a.docx doc-b.docx --max-ops 100     # more ops

# edit_text (dry_run default)
stichos edit --kind text --dry-run                   # default: preview only
stichos edit --kind text --apply                     # apply the change
```

### 4.2 CLI Enum Parsing

```rust
#[derive(Debug, Clone, clap::ValueEnum)]
enum Detail {
    Summary,
    Elements,
    All,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum Output {
    Base64,
    Urls,
    Both,
}
```

---

## 5. Testing

### 5.1 Unit Tests

```rust
#[test]
fn test_inspect_summary_fits_in_budget() {
    // Create a 50-page document
    let doc = create_large_doc(50, 200_elements_per_page);
    let result = handle_inspect_document(&json!({
        "path": doc.path,
        "detail": "summary",
    })).unwrap();
    
    // Verify response fits in ~150 tokens
    let json_str = serde_json::to_string(&result).unwrap();
    assert!(json_str.len() < 1000); // ~150 tokens * 7 chars/token
}

#[test]
fn test_render_urls_no_base64() {
    let doc = create_doc_with_pages(5);
    let result = handle_render_document(&json!({
        "path": doc.path,
        "output": "urls",
    })).unwrap();
    
    // Verify no base64 in response
    let json_str = serde_json::to_string(&result).unwrap();
    assert!(!json_str.contains("base64"));
    assert!(json_str.contains("url"));
    assert!(json_str.contains("expiresAt"));
}

#[test]
fn test_diff_default_caps_at_20() {
    let doc_a = create_doc();
    let doc_b = create_modified_doc(doc_a.path(), 100_changes);
    let result = handle_diff_documents(&json!({
        "path_a": doc_a.path,
        "path_b": doc_b.path,
    })).unwrap();
    
    // Verify only 20 operations returned
    let ops: Vec<Value> = result["operations"].clone().as_array().unwrap().clone();
    assert_eq!(ops.len(), 20);
    assert_eq!(result["hasMore"].as_bool().unwrap(), true);
    assert_eq!(result["totalOperations"].as_u64().unwrap(), 100);
}

#[test]
fn test_apply_patch_dry_run_default() {
    let doc = create_doc();
    let result = handle_apply_patch(&json!({
        "path": doc.path,
        "ops": [{ "op": "replace", "path": "/body/para[1]", "value": "new text" }],
    })).unwrap();
    
    // Verify dry_run was true (preview returned, not full doc)
    assert!(result["preview"].is_object());
    assert!(!result["document"].is_object()); // No full doc in response
}

#[test]
fn test_temp_file_cleanup() {
    let mut mgr = TempFileManager::new(temp_dir());
    
    // Create some files
    let f1 = mgr.create("test", 1);
    let f2 = mgr.create("test", 2);
    assert!(f1.exists());
    assert!(f2.exists());
    
    // Manually age files beyond TTL
    set_file_time(&f1, SystemTime::now() - Duration::from_secs(3601));
    set_file_time(&f2, SystemTime::now() - Duration::from_secs(3601));
    
    // Cleanup should remove them
    mgr.maybe_cleanup();
    assert!(!f1.exists());
    assert!(!f2.exists());
}
```

### 5.2 Integration Tests

```bash
#!/bin/bash
# tests/integration/context_budget.sh

set -euo pipefail

# Test 1: inspect_document summary mode
echo "Test 1: inspect_document summary mode"
result=$(stichos inspect tests/fixtures/large.docx --detail summary)
echo "$result" | python3 -c "
import json, sys
data = json.load(sys.stdin)
assert 'summary' in data
assert 'elementIds' in data
assert 'pages' in data
# Verify no full element records
assert 'coordinates' not in data
print('PASS')
"

# Test 2: render_document URL mode
echo "Test 2: render_document URL mode"
result=$(stichos render tests/fixtures/simple.docx --output urls)
echo "$result" | python3 -c "
import json, sys
data = json.load(sys.stdin)
assert 'pages' in data
assert 'url' in data['pages'][0]
assert 'base64' not in str(data)
print('PASS')
"

# Test 3: diff_documents default cap
echo "Test 3: diff_documents default cap"
result=$(stichos diff tests/fixtures/a.docx tests/fixtures/b.docx)
echo "$result" | python3 -c "
import json, sys
data = json.load(sys.stdin)
assert 'summary' in data
assert 'operations' in data
assert len(data['operations']) <= 20
assert 'hasMore' in data
print('PASS')
"
```

---

## 6. Phase Gate Checklist

Before merging, verify:

- [ ] All new parameters are optional (additive only)
- [ ] Default values match C14 specification
- [ ] `__budget` field present on all modified tool responses
- [ ] `rendered_by` field present on all responses
- [ ] `diagnostics` field present on all responses
- [ ] Temp file cleanup works (TTL and startup)
- [ ] No base64 in responses when `output="urls"`
- [ ] `dry_run` defaults to `true` for mutation tools
- [ ] `max_ops` defaults to `20` for diff_documents
- [ ] All unit tests pass
- [ ] All integration tests pass
- [ ] C7 contract updated with v1.6.0
- [ ] CHANGELOG.md updated
- [ ] No breaking changes (verified by schema diff)
