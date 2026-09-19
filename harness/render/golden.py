# SPDX-License-Identifier: MIT OR Apache-2.0
#!/usr/bin/env python3
"""stichos HAR-05 — golden render pipeline + content-addressed approved store.

Single-file CLI:  python3 golden.py {render,stability,approve,list,verify} [flags]

Pipeline per fixture (C9 §2 `rendered` state):
  pinned container (corpus/blobs mounted READ-ONLY at /in)
    -> soffice --headless --convert-to pdf   (cwd /tmp, timeout 240)
    -> pdftoppm -r <dpi> -gray               (C9 §4 normative rasters)
    -> PDF + page PNGs pulled to goldens/staging/<id>/ (RW mount at /out)
  sha256 of PDF + each page -> record.json under
    goldens/store/<engine-key>/<fixture-sha>/   (state: candidate)
  bytes copied content-addressed to goldens/objects/<sha[:2]>/<sha>.{pdf,png} (dedup via exists()).

engine-key = f"{engine}-{engine_version}-{fontset_hash_hex[:16]}"  (INV-4: font set is
part of the cache key; a new fontset_hash means a new key, old goldens stay queryable).

Approval (C9 §2 `golden-approved`) is a human gate: `approve` only promotes when the
fixture's stability entry is ok, and records the reviewer. `--reviewer` is mandatory in
spirit; the M0 default "owner-pending" is allowed but recorded as-is.
"""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import threading
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(os.environ.get("GP_ROOT", Path(__file__).resolve().parents[2]))
BLOBS = ROOT / "corpus" / "blobs"
MANIFESTS = ROOT / "corpus" / "manifests"
GOLDENS = ROOT / "goldens"
STAGING = GOLDENS / "staging"
OBJECTS = GOLDENS / "objects"
STORE = GOLDENS / "store"
ENVJSON = GOLDENS / "env.json"
STABILITY_JSON = GOLDENS / "stability.json"
APPROVALS_JSON = GOLDENS / "approvals.json"

IMAGE = os.environ.get("GP_IMAGE", "stichos-harness:26.2.5")
ENGINE = "soffice-26.2.5"
DEFAULT_DPI = 150

_PRINT_LOCK = threading.Lock()
_STAB_LOCK = threading.Lock()


def log(msg: str) -> None:
    with _PRINT_LOCK:
        print(msg, flush=True)


def now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def page_sort_key(p: Path) -> tuple:
    m = re.search(r"(\d+)", p.stem)
    return (int(m.group(1)) if m else 0, p.name)  # natural sort, C9 §10.8


# ---------------------------------------------------------------- environment

def env_info(force: bool = False) -> dict:
    """fontset_hash (tools/fontset_hash.sh) + engine_version (soffice --version)."""
    if not force and ENVJSON.exists():
        info = json.loads(ENVJSON.read_text(encoding="utf-8"))
        if info.get("image") == IMAGE:
            return info
    r = subprocess.run(["bash", str(ROOT / "tools" / "fontset_hash.sh"), IMAGE],
                       capture_output=True, text=True, check=True, timeout=600)
    m = re.search(r"^fontset_hash=(sha256:[0-9a-f]{64})$", r.stdout, re.M)
    if not m:
        raise RuntimeError(f"could not parse fontset_hash from:\n{r.stdout}")
    rv = subprocess.run(
        ["docker", "run", "--rm", IMAGE, "bash", "-c",
         "soffice --version 2>/dev/null | head -1; jq -r .poppler_version /opt/stichos/build-info.json"],
        capture_output=True, text=True, check=True, timeout=300)
    lines = rv.stdout.strip().splitlines()
    mv = re.search(r"LibreOffice\s+([0-9][0-9.]*)", lines[0])
    if not mv:
        raise RuntimeError(f"could not parse soffice version from: {lines[0]!r}")
    info = {
        "image": IMAGE,
        "engine": ENGINE,
        "engine_version": mv.group(1),
        "poppler_version": lines[1].strip() if len(lines) > 1 else None,
        "fontset_hash": m.group(1),
        "measured_at": now_iso(),
    }
    GOLDENS.mkdir(parents=True, exist_ok=True)
    ENVJSON.write_text(json.dumps(info, indent=2) + "\n", encoding="utf-8")
    return info


def fontset_hex(info: dict) -> str:
    return info["fontset_hash"].split(":", 1)[1]


def engine_key(info: dict) -> str:
    return f"{ENGINE}-{info['engine_version']}-{fontset_hex(info)[:16]}"


# ---------------------------------------------------------------- fixtures

def load_fixtures(ids: list[str] | None, category: str | None) -> list[dict]:
    out = []
    for m in sorted(MANIFESTS.glob("*.json")):
        fx = json.loads(m.read_text(encoding="utf-8"))
        if ids and fx["id"] not in ids:
            continue
        if category and category not in fx.get("categories", []):
            continue
        out.append(fx)
    return out


# ---------------------------------------------------------------- render

def render_once(fx: dict, dpi: int, stage: Path) -> dict:
    """One fresh container run. Returns {pdf_sha, page_shas, pdf_path, page_paths}."""
    fid, fmt = fx["id"], fx["format"]
    blob = BLOBS / f"{fid}.{fmt}"
    if not blob.exists():
        raise RuntimeError(f"blob missing: {blob}")
    if stage.exists():
        shutil.rmtree(stage)
    (stage / "pages").mkdir(parents=True)
    # container runs as uid 1001 (harvester); host uid differs -> open the RW mount.
    os.chmod(stage, 0o777)
    os.chmod(stage / "pages", 0o777)
    script = (
        f"cp /in/{fid}.{fmt} /tmp/f.{fmt}\n"
        "cd /tmp\n"
        f"timeout 240 soffice --headless -env:UserInstallation=file:///tmp/u --convert-to pdf f.{fmt}\n"
        "test -s /tmp/f.pdf\n"
        f"pdftoppm -r {dpi} -gray -png /tmp/f.pdf /tmp/page\n"
        "cp /tmp/f.pdf /out/render.pdf\n"
        "cp /tmp/page-*.png /out/pages/\n"
    )
    cmd = ["docker", "run", "--rm",
           "-v", f"{BLOBS}:/in:ro",
           "-v", f"{stage}:/out",
           IMAGE, "bash", "-eu", "-o", "pipefail", "-c", script]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=330)
    if r.returncode != 0:
        tail = (r.stderr or r.stdout or "").strip().splitlines()[-3:]
        raise RuntimeError(f"container render failed rc={r.returncode}: {' | '.join(tail)}")
    pdf = stage / "render.pdf"
    pages = sorted((stage / "pages").glob("page-*.png"), key=page_sort_key)
    if not pages:
        raise RuntimeError("no page PNGs produced")
    return {
        "pdf_sha": sha256_file(pdf),
        "page_shas": [sha256_file(p) for p in pages],
        "pdf_path": pdf,
        "page_paths": pages,
    }


def put_object(sha: str, src: Path, ext: str) -> None:
    d = OBJECTS / sha[:2]
    d.mkdir(parents=True, exist_ok=True)
    dst = d / f"{sha}.{ext}"
    if not dst.exists():  # content-addressed dedup
        shutil.copyfile(src, dst)


def store_path(info: dict, fixture_sha: str) -> Path:
    return STORE / engine_key(info) / fixture_sha


def publish(fx: dict, fixture_sha: str, res: dict, dpi: int, info: dict, state: str) -> Path:
    put_object(res["pdf_sha"], res["pdf_path"], "pdf")
    for sha, p in zip(res["page_shas"], res["page_paths"]):
        put_object(sha, p, "png")
    rec = {
        "fixture": fx["id"],
        "fixture_sha": fixture_sha,
        "engine": ENGINE,
        "engine_version": info["engine_version"],
        "fontset_hash": info["fontset_hash"],
        "dpi": dpi,
        "pdf_sha": res["pdf_sha"],
        "pages": res["page_shas"],
        "state": state,
        "staged_at": now_iso(),
    }
    d = store_path(info, fixture_sha)
    d.mkdir(parents=True, exist_ok=True)
    (d / "record.json").write_text(json.dumps(rec, indent=2) + "\n", encoding="utf-8")
    return d / "record.json"


def fixture_blob_sha(fx: dict) -> str:
    blob = BLOBS / f"{fx['id']}.{fx['format']}"
    sha = sha256_file(blob)
    if fx.get("sha256") and fx["sha256"] != sha:
        raise RuntimeError(f"blob sha {sha} != manifest sha {fx['sha256']}")
    return sha


# ---------------------------------------------------------------- commands

def cmd_render(args) -> int:
    info = env_info()
    fixtures = load_fixtures([args.id] if args.id else None, args.category)
    if not fixtures:
        log("no fixtures matched")
        return 2
    log(f"render: {len(fixtures)} fixtures, dpi={args.dpi}, engine-key={engine_key(info)}")
    ok, fail = 0, []

    def one(fx):
        fid = fx["id"]
        t0 = datetime.now()
        fixture_sha = fixture_blob_sha(fx)
        res = render_once(fx, args.dpi, STAGING / fid)
        publish(fx, fixture_sha, res, args.dpi, info, "candidate")
        dt = (datetime.now() - t0).total_seconds()
        log(f"  ok   {fid} {len(res['page_shas'])}p {dt:.1f}s")

    with cf.ThreadPoolExecutor(max_workers=args.jobs) as ex:
        futs = {ex.submit(one, fx): fx["id"] for fx in fixtures}
        for fut in cf.as_completed(futs):
            fid = futs[fut]
            try:
                fut.result()
                ok += 1
            except Exception as e:  # noqa: BLE001
                fail.append(fid)
                log(f"  FAIL {fid}: {e}")
    log(f"render done: ok={ok} fail={len(fail)}" + (f" failed={sorted(fail)}" if fail else ""))
    return 0 if not fail else 1


def _load_stability() -> dict:
    if STABILITY_JSON.exists():
        return json.loads(STABILITY_JSON.read_text(encoding="utf-8"))
    return {}


def _save_stability(doc: dict) -> None:
    with _STAB_LOCK:
        tmp = STABILITY_JSON.with_suffix(".tmp")
        tmp.write_text(json.dumps(doc, indent=2) + "\n", encoding="utf-8")
        tmp.replace(STABILITY_JSON)


def cmd_stability(args) -> int:
    info = env_info()
    ids = [s for s in (args.ids or "").split(",") if s] or None
    fixtures = load_fixtures(ids, args.category)
    if not fixtures:
        log("no fixtures matched")
        return 2
    n = args.repeats
    log(f"stability: {len(fixtures)} fixtures x {n} fresh re-render(s), dpi={args.dpi}")
    doc = _load_stability()
    doc.update({"generated_at": now_iso(), "engine_key": engine_key(info),
                "engine": ENGINE, "engine_version": info["engine_version"],
                "fontset_hash": info["fontset_hash"], "dpi": args.dpi, "repeats": n})
    results = doc.setdefault("results", {})
    tmp_root = GOLDENS / ".stability"

    def one(fx):
        fid = fx["id"]
        fixture_sha = fixture_blob_sha(fx)
        runs = []
        pdfs = []
        for k in range(n):
            res = render_once(fx, args.dpi, tmp_root / fid / f"run{k}")
            runs.append(res["page_shas"])
            pdfs.append(res["pdf_sha"])
        seq_ok = all(r == runs[0] for r in runs)
        if n == 1:
            # 1 re-render: compare against the stored candidate/approved record.
            rp = store_path(info, fixture_sha) / "record.json"
            stored = json.loads(rp.read_text(encoding="utf-8")) if rp.exists() else None
            stored_match = (stored is not None and stored["pages"] == runs[0])
            seq_ok = bool(stored_match)
        else:
            stored = None
            stored_match = None
        entry = {"ok": seq_ok, "fixture_sha": fixture_sha, "runs": runs,
                 "pdf_shas": pdfs, "pdf_seq_ok": all(p == pdfs[0] for p in pdfs),
                 "stored_pages_match": stored_match, "checked_at": now_iso()}
        with _STAB_LOCK:
            results[fid] = entry
        _save_stability(doc)
        log(f"  {'stable  ' if seq_ok else 'UNSTABLE'} {fid} {len(runs[0])}p"
            + ("" if entry["pdf_seq_ok"] else " (pdf_sha differs — PDF metadata churn)"))

    fail = []
    with cf.ThreadPoolExecutor(max_workers=args.jobs) as ex:
        futs = {ex.submit(one, fx): fx["id"] for fx in fixtures}
        for fut in cf.as_completed(futs):
            fid = futs[fut]
            try:
                fut.result()
            except Exception as e:  # noqa: BLE001
                fail.append(fid)
                with _STAB_LOCK:
                    results[fid] = {"ok": False, "error": str(e), "checked_at": now_iso()}
                _save_stability(doc)
                log(f"  ERROR   {fid}: {e}")
    if not args.keep_temp and tmp_root.exists():
        shutil.rmtree(tmp_root, ignore_errors=True)
    ok_n = sum(1 for e in results.values() if e.get("ok"))
    unstable = sorted(fid for fid, e in results.items() if not e.get("ok"))
    doc["summary"] = {"ok": ok_n, "fail": len(unstable), "failed_ids": unstable}
    _save_stability(doc)
    log(f"stability done: ok={ok_n} fail={len(unstable)}" + (f" unstable={unstable}" if unstable else ""))
    return 0 if not fail else 1


def _load_records(info: dict) -> dict[str, dict]:
    out = {}
    root = STORE / engine_key(info)
    if not root.exists():
        return out
    for rp in root.glob("*/record.json"):
        rec = json.loads(rp.read_text(encoding="utf-8"))
        out[rec["fixture"]] = rec
    return out


def cmd_approve(args) -> int:
    info = env_info()
    records = _load_records(info)
    stab = _load_stability()
    stab_res = stab.get("results", {})
    if args.all_staged:
        ids = sorted(fid for fid, r in records.items() if r["state"] == "candidate")
    else:
        ids = [args.id]
    if not ids:
        log("nothing to approve")
        return 2
    approvals = json.loads(APPROVALS_JSON.read_text(encoding="utf-8")) if APPROVALS_JSON.exists() else []
    known = {a["fixture"] for a in approvals if a.get("engine_key") == engine_key(info)}
    promoted, refused = [], []
    for fid in ids:
        rec = records.get(fid)
        if rec is None:
            refused.append((fid, "no staged record"))
            continue
        if rec["state"] == "approved":
            refused.append((fid, "already approved"))
            continue
        st = stab_res.get(fid)
        if st is None or not st.get("ok"):
            refused.append((fid, f"stability not ok ({'missing' if st is None else 'failed'})"))
            continue
        rec["state"] = "approved"
        (store_path(info, rec["fixture_sha"]) / "record.json").write_text(
            json.dumps(rec, indent=2) + "\n", encoding="utf-8")
        golden_key = "sha256:" + hashlib.sha256(
            ("\n".join(rec["pages"])).encode()).hexdigest()  # content hash of approved raster set
        approvals.append({
            "fixture": fid,
            "fixture_sha": rec["fixture_sha"],
            "engine": rec["engine"],
            "engine_version": rec["engine_version"],
            "fontset_hash": rec["fontset_hash"],
            "dpi": rec["dpi"],
            "pdf_sha": rec["pdf_sha"],
            "golden_key": golden_key,
            "engine_key": engine_key(info),
            "reviewer": args.reviewer,
            "when": now_iso(),
        })
        promoted.append(fid)
    tmp = APPROVALS_JSON.with_suffix(".tmp")
    tmp.write_text(json.dumps(approvals, indent=2) + "\n", encoding="utf-8")
    tmp.replace(APPROVALS_JSON)
    for fid, why in refused:
        log(f"  refused {fid}: {why}")
    log(f"approve done: approved={len(promoted)} refused={len(refused)} reviewer={args.reviewer!r}")
    return 0 if not refused else 1


def cmd_list(args) -> int:
    info = env_info()
    records = _load_records(info)
    rows = [(fid, r) for fid, r in sorted(records.items())
            if not args.state or r["state"] == args.state]
    print(f"engine-key: {engine_key(info)}")
    print(f"{'fixture':<36} {'fixture_sha':<14} {'pdf_sha':<14} {'pages':>5}  {'state':<10} staged_at")
    for fid, r in rows:
        print(f"{fid:<36} {r['fixture_sha'][:12]:<14} {r['pdf_sha'][:12]:<14} "
              f"{len(r['pages']):>5}  {r['state']:<10} {r['staged_at']}")
    print(f"{len(rows)} record(s)")
    return 0


def cmd_verify(args) -> int:
    info = env_info()
    fixtures = {fx["id"]: fx for fx in load_fixtures([args.id], None)}
    if args.id not in fixtures:
        print(json.dumps({"fixture": args.id, "ok": False, "error": "fixture not found"}, indent=2))
        return 2
    fx = fixtures[args.id]
    fixture_sha = fixture_blob_sha(fx)
    rp = store_path(info, fixture_sha) / "record.json"
    if not rp.exists():
        print(json.dumps({"fixture": args.id, "ok": False,
                          "error": f"no stored record under {engine_key(info)}"}, indent=2))
        return 2
    rec = json.loads(rp.read_text(encoding="utf-8"))
    res = render_once(fx, rec["dpi"], GOLDENS / ".verify" / args.id)
    pages_match = res["page_shas"] == rec["pages"]
    pdf_match = res["pdf_sha"] == rec["pdf_sha"]
    first_diff = next((i for i, (a, b) in enumerate(zip(rec["pages"], res["page_shas"]))
                       if a != b), None)
    verdict = {
        "fixture": args.id,
        "ok": pages_match,
        "check": "INV-4 re-render: page-hash sequence must equal stored record",
        "engine_key": engine_key(info),
        "state": rec["state"],
        "pages_stored": len(rec["pages"]),
        "pages_now": len(res["page_shas"]),
        "pages_match": pages_match,
        "first_differing_page": first_diff,
        "pdf_sha_match": pdf_match,
        "verified_at": now_iso(),
    }
    print(json.dumps(verdict, indent=2))
    shutil.rmtree(GOLDENS / ".verify" / args.id, ignore_errors=True)
    return 0 if pages_match else 1


# ---------------------------------------------------------------- CLI

def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="golden.py", description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("render", help="render fixtures -> goldens/staging + store (state=candidate)")
    p.add_argument("--id")
    p.add_argument("--category")
    p.add_argument("--dpi", type=int, default=DEFAULT_DPI)
    p.add_argument("--jobs", type=int, default=4)
    p.set_defaults(fn=cmd_render)

    p = sub.add_parser("stability", help="fresh re-renders; identical page-hash sequences required")
    p.add_argument("--ids", help="comma-separated fixture ids (default: all)")
    p.add_argument("--category")
    p.add_argument("--repeats", type=int, default=2,
                   help="fresh renders per fixture; with 1, compares against the stored record")
    p.add_argument("--dpi", type=int, default=DEFAULT_DPI)
    p.add_argument("--jobs", type=int, default=4)
    p.add_argument("--keep-temp", action="store_true")
    p.set_defaults(fn=cmd_stability)

    p = sub.add_parser("approve", help="promote candidate -> approved (human gate, stability required)")
    p.add_argument("id", nargs="?", help="fixture id (or use --all-staged)")
    p.add_argument("--all-staged", action="store_true")
    p.add_argument("--reviewer", default="owner-pending")
    p.set_defaults(fn=cmd_approve)

    p = sub.add_parser("list", help="table of stored records")
    p.add_argument("--state", choices=["candidate", "approved"])
    p.set_defaults(fn=cmd_list)

    p = sub.add_parser("verify", help="re-render once, compare page hashes to stored record (INV-4)")
    p.add_argument("id")
    p.set_defaults(fn=cmd_verify)

    args = ap.parse_args(argv)
    GOLDENS.mkdir(parents=True, exist_ok=True)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
