# SPDX-License-Identifier: MIT OR Apache-2.0
"""stichos_reader -- run the stichos CLI inspect command and parse IR JSON output."""

import json
import os
import pathlib
import shutil
import subprocess
from typing import Any, Dict, Optional


# Default path relative to the repo root
_STICHOS_BIN = "target/release/stichos"


def _find_stichos() -> Optional[str]:
    """Locate the stichos binary.

    Search order:
    1. Explicit STICHOS_BIN env var
    2. Relative path ``target/release/stichos`` from CWD
    3. Same-relative path from the script's parent directory
    4. PATH lookup
    """
    explicit = os.environ.get("STICHOS_BIN")
    if explicit and pathlib.Path(explicit).is_file():
        return explicit

    from_env = shutil.which("stichos")
    if from_env:
        return from_env

    # Check relative to CWD
    cwd_bin = pathlib.Path(_STICHOS_BIN)
    if cwd_bin.is_file():
        return str(cwd_bin)

    # Check relative to this file's parent (harness/pyoracle -> repo root)
    script_dir = pathlib.Path(__file__).resolve().parent
    repo_root = script_dir.parent.parent
    rel_bin = repo_root / _STICHOS_BIN
    if rel_bin.is_file():
        return str(rel_bin)

    return None


def get_stichos_output(path: str, json_path: Optional[str] = None) -> Dict[str, Any]:
    """Run ``stichos inspect --json --styles --media`` and return parsed JSON.

    Args:
        path: Path to the document file (absolute or relative).
        json_path: Override the stichos binary path.

    Returns:
        Dict with at least "path", "error" (on failure), and IR fields.
    """
    bin_path = json_path or _find_stichos()
    if bin_path is None:
        return {
            "path": path,
            "error": "stichos binary not found",
            "stichos_absent": True,
        }

    cmd = [bin_path, "inspect", "--json", "--styles", "--media", path]
    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=60,
        )
    except FileNotFoundError:
        return {
            "path": path,
            "error": f"stichos binary not found at {bin_path}",
            "stichos_absent": True,
        }
    except subprocess.TimeoutExpired:
        return {
            "path": path,
            "error": "stichos inspect timed out (60s)",
            "stichos_absent": True,
        }

    if result.returncode != 0:
        return {
            "path": path,
            "error": f"stichos exited {result.returncode}: {result.stderr.strip()}",
            "stichos_absent": False,
        }

    try:
        data = json.loads(result.stdout)
        return data
    except json.JSONDecodeError as exc:
        return {
            "path": path,
            "error": f"JSON parse error: {exc}",
            "raw_output": result.stdout[:500],
        }
