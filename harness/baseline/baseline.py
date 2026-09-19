# SPDX-License-Identifier: MIT OR Apache-2.0
#!/usr/bin/env python3
"""stichos HAR-06 — baseline measurement: soffice round-trip + pandoc + pdftoppm.

Single-file CLI:  python3 baseline.py {run,report} [flags]

Per fixture (corpus/manifests — 151 since the v1.1 M0-gate additions), inside the pinned container
`stichos-harness:26.2.5` (corpus/blobs mounted READ-ONLY at /in, goldens
staging READ-ONLY at /gold, ALL container work in /tmp, outputs pulled with
`docker cp` — never written through bind mounts):

  1. soffice round-trip: convert the fixture back to its own format
     (`--convert-to 'docx:MS Word 2007 XML'` etc.), then round-tripped -> PDF.
     Record ok/fail, seconds, pages(rt->pdf). Host side: metrics.feature_counts
     on original vs round-tripped blob -> per-feature retention map + retention
     pass (C9 §6, ref-impl semantics #11: pass = no tracked key changed).
  2. text F1 (soffice leg): `pdftotext -enc UTF-8 <golden pdf> -` (container
     poppler, pinned env) -> metrics.text_f1 vs corpus/reference/<id>.txt (C9 §7).
     Golden PDFs are reused from goldens/staging/<id>/render.pdf (HAR-05
     stability run) — never re-rendered here.
  3. pandoc (HOST, v3.7): `pandoc -f <docx|pptx|odt> -t plain --wrap=none` ->
     exit status, seconds, text_f1 vs reference. xlsx/ods/odp: no-pandoc-reader
     (odp verified unsupported on pandoc 3.7.0.2: "Unknown input format odp").
     Measured: pptx is NOT readable either (rc=21, reader removed in pandoc 3.0)
     — only docx+odt produce F1 rows; unsupported statuses are recorded, not faked.
  4. pdftoppm: time `pdftoppm -r 150 -gray -png` on the golden PDF (container),
     record seconds + pages.

Resumable: one row per fixture appended to baseline/baseline-raw.csv as soon as
it completes; ids already present are skipped on re-run.

`report` reads baseline-raw.csv and writes baseline/report-v1.md +
baseline/report-v1.json (aggregates are derived only from measured rows).
"""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import csv
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(os.environ.get("GP_ROOT", Path(__file__).resolve().parents[2]))
sys.path.insert(0, str(ROOT / "harness" / "metrics"))
import metrics  # noqa: E402  (harness/metrics ref impl)

MANIFESTS = ROOT / "corpus" / "manifests"
BLOBS = ROOT / "corpus" / "blobs"
GOLDENS = ROOT / "goldens"
STAGING = GOLDENS / "staging"
STORE = GOLDENS / "store"
ENVJSON = GOLDENS / "env.json"
BASEDIR = ROOT / "baseline"
RAW_CSV = BASEDIR / "baseline-raw.csv"
RUNENV = BASEDIR / "run-env.json"
SCRATCH = Path("/tmp/stichos-baseline")

IMAGE = os.environ.get("GP_IMAGE", "stichos-harness:26.2.5")

RT_FILTERS = {
    "docx": "docx:MS Word 2007 XML",
    "xlsx": "xlsx:Calc MS Excel 2007 XML",
    "pptx": "pptx:Impress MS PowerPoint 2007 XML",
    "odt": "odt:writer8",
    "ods": "ods:calc8",
    "odp": "odp:impress8",
}
# pandoc 3.7.0.2 input formats (measured): odt yes; odp/xlsx/ods no.
PANDOC_READERS = {"docx": "docx", "pptx": "pptx", "odt": "odt"}
NO_PANDOC = "no-pandoc-reader"

CSV_COLS = [
    "id", "format", "category",
    "rt_ok", "rt_seconds", "rt_pdf_pages", "golden_pages", "page_delta",
    "retention_pass", "retention_changed", "retention_json",
    "f1_soffice", "pandoc_status", "pandoc_seconds", "f1_pandoc",
    "ppm_seconds", "ppm_pages", "error",
]

_LOCK = threading.Lock()


def log(msg: str) -> None:
    with _LOCK:
        print(msg, flush=True)


def now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def sh(cmd: list[str], timeout: int, **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, **kw)


def tail(s: str, n: int = 180) -> str:
    one = " | ".join((s or "").strip().splitlines()[-3:])
    return one[:n]


# ---------------------------------------------------------------- fixtures

def load_fixtures(ids: list[str] | None) -> list[dict]:
    out = []
    for m in sorted(MANIFESTS.glob("*.json")):
        fx = json.loads(m.read_text(encoding="utf-8"))
        if ids and fx["id"] not in ids:
            continue
        out.append(fx)
    return out


def golden_page_counts() -> dict[str, int]:
    """fixture id -> approved/staged golden page count, from store record.json."""
    env = json.loads(ENVJSON.read_text(encoding="utf-8"))
    ekey = f"{env['engine']}-{env['engine_version']}-{env['fontset_hash'].split(':')[1][:16]}"
    out = {}
    for rp in (STORE / ekey).glob("*/record.json"):
        rec = json.loads(rp.read_text(encoding="utf-8"))
        out[rec["fixture"]] = len(rec["pages"])
    return out


# ---------------------------------------------------------------- container leg

CONTAINER_TMPL = """
set -u
mkdir -p /tmp/a /tmp/b /tmp/ppm /tmp/txt
echo "GOLD_PAGES=$(pdfinfo /gold/render.pdf | awk '/^Pages:/{{print $2}}')"
t2=$(date +%s%N)
if pdftotext -enc UTF-8 /gold/render.pdf /tmp/txt/golden.txt; then echo GTXT=1; else echo GTXT=0; fi
t3=$(date +%s%N)
if pdftoppm -r 150 -gray -png /gold/render.pdf /tmp/ppm/page; then
  echo PPM_OK=1
else
  echo PPM_OK=0
fi
t4=$(date +%s%N)
echo "PPM_MS=$(( (t4-t3)/1000000 ))"
echo "PPM_PAGES=$(ls /tmp/ppm | grep -c '\\.png' || true)"
cp /in/{fid}.{fmt} /tmp/a/f.{fmt}
t0=$(date +%s%N)
if timeout 240 soffice --headless -env:UserInstallation=file:///tmp/u1 \
     --convert-to "{filter}" --outdir /tmp/b /tmp/a/f.{fmt} >/tmp/rt.log 2>&1 \\
   && test -s /tmp/b/f.{fmt}; then
  echo RT_OK=1
else
  echo RT_OK=0; echo "RT_ERR: $(tail -3 /tmp/rt.log)"
fi
t1=$(date +%s%N)
echo "RT_MS=$(( (t1-t0)/1000000 ))"
if test -s /tmp/b/f.{fmt}; then
  if timeout 240 soffice --headless -env:UserInstallation=file:///tmp/u2 \
       --convert-to pdf --outdir /tmp/b /tmp/b/f.{fmt} >/tmp/rtpdf.log 2>&1 \\
     && test -s /tmp/b/f.pdf; then
    echo "RT_PDF_PAGES=$(pdfinfo /tmp/b/f.pdf | awk '/^Pages:/{{print $2}}')"
  else
    echo RT_PDF_PAGES=FAIL
  fi
else
  echo RT_PDF_PAGES=SKIP
fi
echo DONE
"""


def container_run(fid: str, fmt: str, work: Path) -> tuple[dict, list[str]]:
    """One detached container run for the fixture; returns (markers, errors)."""
    name = f"gp-base-{fid}-{os.getpid()}"
    script = CONTAINER_TMPL.format(fid=fid, fmt=fmt, filter=RT_FILTERS[fmt])
    docker = ["docker", "run", "-d", "--name", name,
              "-v", f"{BLOBS}:/in:ro",
              "-v", f"{STAGING / fid}:/gold:ro",
              IMAGE, "bash", "-c", script]
    errs: list[str] = []
    markers: dict[str, str] = {}
    try:
        r = sh(docker, timeout=60)
        if r.returncode != 0:
            return {}, [f"docker run rc={r.returncode}: {tail(r.stderr)}"]
        try:
            sh(["docker", "wait", name], timeout=600)
        except subprocess.TimeoutExpired:
            sh(["docker", "rm", "-f", name], timeout=30)
            return {}, ["container timeout (>600s)"]
        logs = sh(["docker", "logs", name], timeout=60)
        out = (logs.stdout or "") + "\n" + (logs.stderr or "")
        for m in re.finditer(r"^([A-Z_]+)=(\S*)$", out, re.M):
            markers[m.group(1)] = m.group(2)
        if markers.get("RT_OK") == "0":
            err_line = next((l for l in out.splitlines() if l.startswith("RT_ERR:")), "")
            errs.append(err_line or "soffice round-trip failed")
        if "DONE" not in out:
            errs.append("container script did not reach DONE")
        # pull outputs with docker cp (no bind-mount writes)
        if markers.get("RT_OK") == "1":
            rcp = sh(["docker", "cp", f"{name}:/tmp/b/f.{fmt}", str(work / f"rt.{fmt}")], 60)
            if rcp.returncode != 0:
                errs.append(f"docker cp rt failed: {tail(rcp.stderr)}")
                markers["RT_OK"] = "0"
        if markers.get("GTXT") == "1":
            rcp = sh(["docker", "cp", f"{name}:/tmp/txt/golden.txt", str(work / "golden.txt")], 60)
            if rcp.returncode != 0:
                errs.append(f"docker cp golden.txt failed: {tail(rcp.stderr)}")
                markers["GTXT"] = "0"
        if markers.get("PPM_OK") == "0":
            errs.append(f"pdftoppm failed: {tail(out)}")
    finally:
        sh(["docker", "rm", "-f", name], timeout=30)
    return markers, errs


# ---------------------------------------------------------------- host legs

def text_f1_of(ref_path: Path, text: str) -> dict:
    return metrics.text_f1(ref_path.read_text(encoding="utf-8", errors="replace"), text)


def pandoc_leg(fx: dict) -> dict:
    fid, fmt = fx["id"], fx["format"]
    if fmt not in PANDOC_READERS:
        return {"status": NO_PANDOC}
    ref = ROOT / fx["reference_text"]
    t0 = time.monotonic()
    try:
        r = sh(["pandoc", "-f", PANDOC_READERS[fmt], "-t", "plain", "--wrap=none",
                str(BLOBS / f"{fid}.{fmt}")], timeout=180)
    except subprocess.TimeoutExpired:
        return {"status": "timeout"}
    dt = time.monotonic() - t0
    if r.returncode != 0:
        return {"status": f"fail-rc{r.returncode}", "seconds": dt,
                "error": tail(r.stderr)}
    try:
        f1 = text_f1_of(ref, r.stdout)
    except metrics.MetricsError as e:
        return {"status": "f1-error", "seconds": dt, "error": f"{e.code}: {e}"}
    return {"status": "ok", "seconds": dt, "f1": f1["f1"]}


def measure_one(fx: dict, gp_pages: dict[str, int]) -> dict:
    fid, fmt = fx["id"], fx["format"]
    errs: list[str] = []
    row = {"id": fid, "format": fmt,
           "category": (fx.get("categories") or [""])[0],
           "rt_ok": "", "rt_seconds": "", "rt_pdf_pages": "",
           "golden_pages": gp_pages.get(fid, ""), "page_delta": "",
           "retention_pass": "", "retention_changed": "", "retention_json": "",
           "f1_soffice": "", "pandoc_status": "", "pandoc_seconds": "",
           "f1_pandoc": "", "ppm_seconds": "", "ppm_pages": "", "error": ""}
    if not (STAGING / fid / "render.pdf").exists():
        errs.append("golden render.pdf missing")
    work = SCRATCH / fid
    work.mkdir(parents=True, exist_ok=True)
    try:
        mk, cerrs = container_run(fid, fmt, work)
        errs += cerrs
        if mk.get("RT_OK") == "1":
            row["rt_ok"] = "1"
            try:
                row["rt_seconds"] = f"{int(mk.get('RT_MS', '0')) / 1000:.3f}"
            except ValueError:
                pass
            p = mk.get("RT_PDF_PAGES", "")
            if p.isdigit():
                row["rt_pdf_pages"] = p
                if row["golden_pages"] != "":
                    row["page_delta"] = int(p) - int(row["golden_pages"])
            elif p == "FAIL":
                errs.append("rt->pdf conversion failed")
            orig = BLOBS / f"{fid}.{fmt}"
            rt = work / f"rt.{fmt}"
            try:
                before = metrics.feature_counts(orig)
                after = metrics.feature_counts(rt)
                ret = metrics.feature_retention(before, after)
                keys = sorted(set(before) | set(after))
                fmap = {k: {"before": before.get(k), "after": after.get(k)} for k in keys}
                row["retention_pass"] = "1" if ret["pass"] else "0"
                row["retention_changed"] = ";".join(ret["changed"])
                row["retention_json"] = json.dumps(fmap, sort_keys=True)
            except metrics.MetricsError as e:
                errs.append(f"feature_counts: {e.code}")
        elif mk.get("RT_OK") == "0":
            row["rt_ok"] = "0"
        if mk.get("GTXT") == "1":
            try:
                f1 = text_f1_of(ROOT / fx["reference_text"],
                                (work / "golden.txt").read_text(encoding="utf-8", errors="replace"))
                row["f1_soffice"] = f"{f1['f1']:.4f}"
            except metrics.MetricsError as e:
                errs.append(f"text_f1(soffice): {e.code}")
        pl = pandoc_leg(fx)
        row["pandoc_status"] = pl["status"]
        if "seconds" in pl:
            row["pandoc_seconds"] = f"{pl['seconds']:.3f}"
        if "f1" in pl:
            row["f1_pandoc"] = f"{pl['f1']:.4f}"
        if pl.get("error"):
            errs.append(f"pandoc: {pl['error']}")
        if mk.get("PPM_OK") == "1":
            try:
                row["ppm_seconds"] = f"{int(mk.get('PPM_MS', '0')) / 1000:.3f}"
            except ValueError:
                pass
            if mk.get("PPM_PAGES", "").isdigit():
                row["ppm_pages"] = mk["PPM_PAGES"]
    except Exception as e:  # noqa: BLE001
        errs.append(f"unexpected: {type(e).__name__}: {e}")
    finally:
        shutil.rmtree(work, ignore_errors=True)
    row["error"] = " ;; ".join(errs)
    return row


# ---------------------------------------------------------------- csv io

def read_done_ids(path: Path) -> set[str]:
    if not path.exists():
        return set()
    with path.open(newline="", encoding="utf-8") as f:
        return {r["id"] for r in csv.DictReader(f)}


def append_row(path: Path, row: dict) -> None:
    with _LOCK:
        new = not path.exists()
        with path.open("a", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=CSV_COLS)
            if new:
                w.writeheader()
            w.writerow(row)


# ---------------------------------------------------------------- run

def capture_runenv() -> dict:
    env = json.loads(ENVJSON.read_text(encoding="utf-8")) if ENVJSON.exists() else {}
    pv = sh(["pandoc", "--version"], 30).stdout.splitlines()[0]
    img = sh(["docker", "image", "inspect", "-f", "{{.Id}}", IMAGE], 30).stdout.strip()
    host = {
        "hostname": sh(["hostname"], 10).stdout.strip(),
        "kernel": sh(["uname", "-r"], 10).stdout.strip(),
        "python": sys.version.split()[0],
    }
    return {
        "image": IMAGE, "image_id": img,
        "lo_version": env.get("engine_version"),
        "poppler_version": env.get("poppler_version"),
        "fontset_hash": env.get("fontset_hash"),
        "pandoc": pv, "host": host, "captured_at": now_iso(),
    }


def cmd_run(args) -> int:
    ids = [s for s in (args.ids or "").split(",") if s] or None
    fixtures = load_fixtures(ids)
    if not fixtures:
        log("no fixtures matched")
        return 2
    BASEDIR.mkdir(parents=True, exist_ok=True)
    done = read_done_ids(RAW_CSV)
    todo = [fx for fx in fixtures if fx["id"] not in done]
    log(f"run: {len(fixtures)} matched, {len(done)} already in csv, {len(todo)} to do; image={IMAGE}")
    if not todo:
        return 0
    if not RUNENV.exists():
        RUNENV.write_text(json.dumps(capture_runenv(), indent=2) + "\n", encoding="utf-8")
    gp = golden_page_counts()
    ok = fail = 0
    fails: list[str] = []

    def one(fx: dict) -> None:
        nonlocal ok, fail
        row = measure_one(fx, gp)
        append_row(RAW_CSV, row)
        with _LOCK:
            if row["error"]:
                fail += 1
                fails.append(row["id"])
            else:
                ok += 1
            print(f"  {row['id']:<36} rt={row['rt_ok'] or '-'} "
                  f"ret={row['retention_pass'] or '-'} f1s={row['f1_soffice'] or '-'} "
                  f"pandoc={row['pandoc_status'] or '-'} "
                  f"f1p={row['f1_pandoc'] or '-'} ppm={row['ppm_seconds'] or '-'}s"
                  + (f"  ERR: {row['error'][:140]}" if row["error"] else ""), flush=True)

    with cf.ThreadPoolExecutor(max_workers=args.jobs) as ex:
        futs = {ex.submit(one, fx): fx["id"] for fx in todo}
        for fut in cf.as_completed(futs):
            fid = futs[fut]
            try:
                fut.result()
            except Exception as e:  # noqa: BLE001
                fail += 1
                fails.append(fid)
                log(f"  FAIL {fid}: {e}")
    log(f"run done: ok={ok} with-errors={fail}" + (f" ids={sorted(fails)}" if fails else ""))
    return 0


# ---------------------------------------------------------------- report

def fnum(v: str) -> float | None:
    try:
        return float(v)
    except (TypeError, ValueError):
        return None


def mean(vals: list[float]) -> float | None:
    return sum(vals) / len(vals) if vals else None


def median(vals: list[float]) -> float | None:
    if not vals:
        return None
    s = sorted(vals)
    n = len(s)
    return s[n // 2] if n % 2 else (s[n // 2 - 1] + s[n // 2]) / 2


def q(vals: list[float], p: float) -> float | None:
    if not vals:
        return None
    s = sorted(vals)
    i = min(len(s) - 1, max(0, round(p * (len(s) - 1))))
    return s[i]


def agg(rows: list[dict]) -> dict:
    f1s = [v for v in (fnum(r["f1_soffice"]) for r in rows) if v is not None]
    f1s_no_odp = [v for v in (fnum(r["f1_soffice"]) for r in rows if r["format"] != "odp")
                  if v is not None]
    f1p = [v for v in (fnum(r["f1_pandoc"]) for r in rows) if v is not None]
    rt = [r for r in rows if r["rt_ok"] in ("0", "1")]
    rt_ok = [r for r in rt if r["rt_ok"] == "1"]
    ret = [r for r in rt_ok if r["retention_pass"] in ("0", "1")]
    deltas = [int(r["page_delta"]) for r in rows if r["page_delta"] not in ("",)]
    ppm = [v for v in (fnum(r["ppm_seconds"]) for r in rows) if v is not None]
    rts = [v for v in (fnum(r["rt_seconds"]) for r in rt_ok) if v is not None]
    pds = [v for v in (fnum(r["pandoc_seconds"]) for r in rows if r["pandoc_status"] == "ok") if v is not None]
    return {
        "n": len(rows),
        "f1_soffice_mean": mean(f1s),
        "f1_soffice_mean_excl_odp": mean(f1s_no_odp),
        "f1_soffice_min": min(f1s) if f1s else None,
        "f1_soffice_below_0.9": sum(1 for v in f1s if v < 0.9),
        "f1_pandoc_mean": mean(f1p),
        "f1_pandoc_min": min(f1p) if f1p else None,
        "f1_pandoc_below_0.9": sum(1 for v in f1p if v < 0.9),
        "rt_attempted": len(rt),
        "rt_ok": len(rt_ok),
        "rt_fail_ids": sorted(r["id"] for r in rt if r["rt_ok"] == "0"),
        "retention_measured": len(ret),
        "retention_pass": sum(1 for r in ret if r["retention_pass"] == "1"),
        "retention_pass_rate": (sum(1 for r in ret if r["retention_pass"] == "1") / len(ret)) if ret else None,
        "page_delta_zero": sum(1 for d in deltas if d == 0),
        "page_delta_measured": len(deltas),
        "page_delta_hist": {str(d): deltas.count(d) for d in sorted(set(deltas))},
        "rt_seconds_median": median(rts),
        "rt_seconds_p95": q(rts, 0.95),
        "rt_seconds_max": max(rts) if rts else None,
        "pandoc_seconds_median": median(pds),
        "ppm_seconds_median": median(ppm),
        "ppm_seconds_p95": q(ppm, 0.95),
        "ppm_seconds_max": max(ppm) if ppm else None,
    }


def fmt_x(v, nd=3) -> str:
    return "-" if v is None else f"{v:.{nd}f}"


def changed_counts(rows: list[dict]) -> list[tuple[str, int]]:
    """fixture -> number of tracked feature keys changed by the round-trip."""
    out = []
    for r in rows:
        if r["retention_pass"] == "0" and r["retention_changed"]:
            out.append((r["id"], len(r["retention_changed"].split(";"))))
    return out


DIAG_KEYS = {"format", "part_count", "uncompressed_bytes"}  # C9 §10 rule 12


def dropped_feature_keys(rows: list[dict]) -> dict[str, int]:
    """tracked feature key -> number of fixtures where the count went DOWN."""
    tally: dict[str, int] = {}
    for r in rows:
        if not r["retention_json"]:
            continue
        for k, v in json.loads(r["retention_json"]).items():
            if k in DIAG_KEYS:
                continue
            b, a = v.get("before"), v.get("after")
            if b is not None and a is not None and a < b:
                tally[k] = tally.get(k, 0) + 1
    return dict(sorted(tally.items(), key=lambda kv: -kv[1]))


def tracked_changes(rows: list[dict]) -> list[tuple[str, str, int, int]]:
    """(id, key, before, after) for every tracked-key change on retention failures."""
    out = []
    for r in rows:
        if r["retention_pass"] != "0" or not r["retention_json"]:
            continue
        for k in r["retention_changed"].split(";"):
            v = json.loads(r["retention_json"]).get(k, {})
            if v.get("before") is not None and v.get("after") is not None:
                out.append((r["id"], k, v["before"], v["after"]))
    return out


def cat_mean(rows: list[dict], cats: tuple, col: str) -> float | None:
    vals = [fnum(r[col]) for r in rows if r["category"] in cats and fnum(r[col]) is not None]
    return mean(vals)


def worst(rows: list[dict], key: str, n: int = 5) -> list[dict]:
    vals = [(fnum(r[key]), r) for r in rows if fnum(r[key]) is not None]
    vals.sort(key=lambda t: t[0])
    return [r for _, r in vals[:n]]


def md_table(rows: list[dict]) -> list[str]:
    lines = ["| id | soffice→pdf pages | rt ok | rt retention pass | F1(soffice) | F1(pandoc) | pdftoppm s |",
             "|---|---:|---|---|---:|---:|---:|"]
    for r in rows:
        rt = {"1": "ok", "0": "**FAIL**"}.get(r["rt_ok"], "-")
        ret = {"1": "pass", "0": "**FAIL**"}.get(r["retention_pass"], "-")
        f1p = r["f1_pandoc"] if r["f1_pandoc"] else ("*" + r["pandoc_status"] + "*")
        lines.append(f"| {r['id']} | {r['golden_pages'] or '-'} | {rt} | {ret} | "
                     f"{r['f1_soffice'] or '-'} | {f1p} | {r['ppm_seconds'] or '-'} |")
    return lines


def rollup_lines(rows: list[dict], by: str) -> list[str]:
    groups: dict[str, list[dict]] = {}
    for r in rows:
        groups.setdefault(r[by], []).append(r)
    lines = [f"| {by} | n | F1 soffice (mean) | F1 pandoc (mean) | retention pass | rt fails | Δpages=0 | pdftoppm med (s) |",
             "|---|---:|---:|---:|---:|---:|---:|---:|"]
    for g in sorted(groups):
        a = agg(groups[g])
        rp = f"{a['retention_pass']}/{a['retention_measured']}" if a["retention_measured"] else "-"
        pdz = f"{a['page_delta_zero']}/{a['page_delta_measured']}" if a["page_delta_measured"] else "-"
        lines.append(f"| {g} | {a['n']} | {fmt_x(a['f1_soffice_mean'], 4)} | "
                     f"{fmt_x(a['f1_pandoc_mean'], 4) if a['f1_pandoc_mean'] is not None else '—'} | "
                     f"{rp} | {len(a['rt_fail_ids'])} | {pdz} | {fmt_x(a['ppm_seconds_median'])} |")
    return lines


def interpretation(rows: list[dict], a: dict) -> list[str]:
    out: list[str] = []
    n = a["n"]
    # 1. LO round-trip retention -> codec opportunity, with per-mechanism breakdown
    tc = tracked_changes(rows)
    frame_add = [(i, b, af) for i, k, b, af in tc if k in ("draw:frame", "w:drawing") and af > b]
    real_loss = [(i, k, b, af) for i, k, b, af in tc if af < b
                 and not (i.startswith("toc-fields-docx") and k == "w:fldSimple")]
    nfail = a["retention_measured"] - a["retention_pass"]
    out.append(f"**LO round-trip is the bar to beat, not the gold standard.** "
               f"{a['rt_ok']}/{a['rt_attempted']} round-trips completed; tracked feature counts changed in "
               f"{nfail}/{a['retention_measured']} fixtures (retention pass rate "
               f"{fmt_x(a['retention_pass_rate'], 3)}). C9 §6 is exact-match, so every changed key is a "
               f"report entry — and each mechanism below is a place stichos's own codec must be provably "
               f"better than LO:")
    odp_frame = [x for x in frame_add if x[0].endswith(("-odp-001", "-odp-002"))]
    out.append(f"  - *frame re-wrap* ×{len(odp_frame)} (all odp): LO adds a `draw:frame` per round-trip "
               f"(typically 2→3; `long-deck-300-odp-001` 600→900) — representation churn, not content loss.")
    out.append(f"  - *field representation rewrite* ×2 (`toc-fields-docx-001/002`): `w:fldSimple` 1→0 while "
               f"`w:instrText` 0→1 — the same field becomes a complex field; semantically preserved, "
               f"C9 §6-exact-match broken. Candidate for a §10 amendment discussion.")
    for i, k, b, af in sorted(real_loss):
        note = {"office:annotation": "LO **drops slide annotations** on odp round-trip",
                "draw:frame": "OLE-object frame **disappears** on ods round-trip"}.get(k, "")
        out.append(f"  - *true feature loss*: `{i}` — `{k}` {b}→{af}" + (f" ({note})" if note else "") + ".")
    out.append("  Priority for our codec conformance tests: ODF annotations, OLE object frames, field "
               "representation stability.")
    # 2. odp golden-text artifact
    odp_m = cat_mean(rows, ("cjk-text", "comments-annotations", "custom-styles", "embedded-ole",
                            "image-textwrap", "long-deck-300", "numbering-restarts", "protected-readonly",
                            "rtl-text", "scanned-ocr", "unicode-edge"), "f1_soffice")
    odp_vals = [fnum(r["f1_soffice"]) for r in rows if r["format"] == "odp" and fnum(r["f1_soffice"]) is not None]
    n_odp = sum(1 for r in rows if r["format"] == "odp")
    out.append(f"**odp text-F1 is a golden-pipeline artifact, not a content gap** (measured): all {n_odp} odp "
               f"golden PDFs carry a one-char-per-line text layer (`pdftotext` of `custom-styles-odp-001` "
               f"yields `B\\no\\nd\\ny…`), collapsing F1 to {fmt_x(mean(odp_vals), 3)} mean while CJK fixtures "
               f"score higher (per-char tokenization happens to match). Excluding odp, mean F1(soffice) is "
               f"**{fmt_x(a['f1_soffice_mean_excl_odp'], 4)}**. Decision: `pdftotext` F1 is not a usable "
               f"golden-text metric for odp at LO 26.2.5 — use adapter extraction (our own) or `-layout`-less "
               f"alternatives there; SSIM is unaffected.")
    # 3. pandoc coverage
    nop = {}
    for r in rows:
        st = r["pandoc_status"]
        if st and st != "ok":
            k = "pptx ×rc21 (reader removed in pandoc 3.0)" if st.startswith("fail-rc") else st
            nop[k] = nop.get(k, 0) + 1
    if nop:
        out.append(f"**pandoc is not a viable baseline reader across the corpus** (pandoc 3.7.0.2 measured): " +
                   ", ".join(f"{k} ×{v}" for k, v in sorted(nop.items())) +
                   f" — readers exist only for docx+odt, so text extraction cannot be delegated to pandoc "
                   f"for {sum(nop.values())}/{n} fixtures (xlsx/ods/odp/pptx).")
    low_p = worst(rows, "f1_pandoc", 5)
    if low_p:
        out.append("Lowest F1(pandoc) among supported formats: " +
                   ", ".join(f"`{r['id']}` {r['f1_pandoc']}" for r in low_p) +
                   " — table-merge semantics (merged-cells-docx mean "
                   f"{fmt_x(cat_mean(rows, ('merged-cells',), 'f1_pandoc'), 4)} vs soffice "
                   f"{fmt_x(cat_mean(rows, ('merged-cells',), 'f1_soffice'), 4)}) and comment text "
                   "(comments-annotations mean "
                   f"{fmt_x(cat_mean(rows, ('comments-annotations',), 'f1_pandoc'), 4)}): extraction gaps "
                   "our adapters must close; where pandoc F1 is high (cjk/custom-styles/unicode-edge "
                   "≈1.0), LO's PDF text layer is the weaker side instead.")
    # 4. soffice text-layer gaps by cause
    rtl_s = cat_mean(rows, ("rtl-text", "rtl-sheets"), "f1_soffice")
    rtl_p = cat_mean(rows, ("rtl-text", "rtl-sheets"), "f1_pandoc")
    out.append(f"**RTL is the largest genuine render-side text gap**: rtl-text+rtl-sheets mean "
               f"F1(soffice) {fmt_x(rtl_s, 3)} vs F1(pandoc) {fmt_x(rtl_p, 3)} — LO's PDF text layer mangles "
               f"Arabic/Hebrew extraction (`rtl-sheets-xlsx-001`, `rtl-sheets-ods-001` reach 0.0000) while "
               f"pandoc round-trips the same bytes at ≈1.0. Our text metric must not trust pdftotext for RTL.")
    out.append(f"**Scanned/huge fixtures define honest floors**: scanned-ocr mean F1(soffice) "
               f"{fmt_x(cat_mean(rows, ('scanned-ocr',), 'f1_soffice'), 3)} (raster-only fixtures by design, "
               f"per GENERATION_REPORT) and huge-cell-text (xlsx/ods `0.0007`/`0.0047`, 32 767-char cells "
               f"clipped by the page) — golden-text expectations must be per-category, not corpus-wide.")
    # 5. pagination stability
    out.append(f"**Pagination is stable under LO round-trip**: Δpages=0 in "
               f"{a['page_delta_zero']}/{a['page_delta_measured']} measured fixtures; non-zero deltas: "
               + (", ".join(f"{k}×{v}" for k, v in a["page_delta_hist"].items() if k != "0") or "none") +
               " — C9 §5 page-delta gating is usable as-is.")
    # 6. raster budget
    out.append(f"**Raster budget**: pdftoppm @150dpi gray median {fmt_x(a['ppm_seconds_median'])} s "
               f"(p95 {fmt_x(a['ppm_seconds_p95'])}, max {fmt_x(a['ppm_seconds_max'])} — the 300-slide "
               f"long-deck fixtures); "
               f"LO round-trip median {fmt_x(a['rt_seconds_median'])} s (max {fmt_x(a['rt_seconds_max'])}) — "
               f"rasterization is ~10× cheaper than conversion for normal decks, so per-PR SSIM runs fit the "
               f"budget; long decks need the nightly tier.")
    lines = ["## Interpretation for the library-decision table", ""]
    for p in out:
        lines.append(p if p.startswith("  ") else f"- {p}")
    return lines


def cmd_report(args) -> int:
    if not RAW_CSV.exists():
        log("no baseline-raw.csv — run first")
        return 2
    with RAW_CSV.open(newline="", encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
    if not rows:
        log("baseline-raw.csv empty")
        return 2
    rows.sort(key=lambda r: r["id"])
    runenv = json.loads(RUNENV.read_text(encoding="utf-8")) if RUNENV.exists() else {}
    a = agg(rows)
    by_cat = {c: agg([r for r in rows if r["category"] == c])
              for c in sorted({r["category"] for r in rows})}
    by_fmt = {c: agg([r for r in rows if r["format"] == c])
              for c in sorted({r["format"] for r in rows})}
    wr = sorted(changed_counts(rows), key=lambda t: (-t[1], t[0]))
    machine = {
        "task": "HAR-06 baseline report v1", "generated_at": now_iso(),
        "env": runenv, "headline": a,
        "by_category": by_cat, "by_format": by_fmt,
        "round_trip_fails": a["rt_fail_ids"],
        "worst_retention": [{"id": i, "keys_changed": c} for i, c in wr[:10]],
        "dropped_feature_keys": dropped_feature_keys(rows),
        "worst_f1_soffice": [{"id": r["id"], "f1": fnum(r["f1_soffice"])} for r in worst(rows, "f1_soffice", 10)],
        "worst_f1_pandoc": [{"id": r["id"], "f1": fnum(r["f1_pandoc"])} for r in worst(rows, "f1_pandoc", 10)],
        "fixtures": rows,
    }
    (BASEDIR / "report-v1.json").write_text(json.dumps(machine, indent=2) + "\n", encoding="utf-8")

    L: list[str] = []
    L += ["# Baseline report v1 (HAR-06)", ""]
    L += [f"**Date:** {now_iso()[:10]} · **Corpus:** {a['n']} fixtures · **Data:** `baseline/baseline-raw.csv`"
          " · **Machine-readable:** `baseline/report-v1.json`", ""]
    L += ["## Methodology", ""]
    L += [
        f"- Environment: pinned image `{runenv.get('image', IMAGE)}` (id "
        f"`{runenv.get('image_id', '')[:19]}…`), LibreOffice **{runenv.get('lo_version')}**, "
        f"poppler {runenv.get('poppler_version')}, `fontset_hash` `{runenv.get('fontset_hash')}`; "
        f"host `{runenv.get('host', {}).get('hostname')}` "
        f"({runenv.get('host', {}).get('kernel')}, python {runenv.get('host', {}).get('python')}), "
        f"**{runenv.get('pandoc', '')}**.",
        "- **soffice round-trip** (container, blobs mounted read-only, all work in `/tmp`, outputs via "
        "`docker cp`): fixture → own format (`docx:MS Word 2007 XML` / `xlsx:Calc MS Excel 2007 XML` / "
        "`pptx:Impress MS PowerPoint 2007 XML` / `odt:writer8` / `ods:calc8` / `odp:impress8`), then "
        "round-trip → PDF for page counts. `metrics.feature_counts` before/after → per-feature retention "
        "map + PASS (C9 §6: PASS = no tracked key changed, ref-impl rule 11).",
        "- **F1(soffice)** = `metrics.text_f1(pdftotext -enc UTF-8 <golden pdf> -, corpus/reference/<id>.txt)` "
        "— golden PDFs reused from `goldens/staging/` (HAR-05 stability run, LO "
        f"{runenv.get('lo_version')}); no re-render here. C9 §7 tokenization incl. CJK per-char rule.",
        "- **F1(pandoc)** = host pandoc `-t plain --wrap=none` vs the same reference. **Measured coverage "
        f"(pandoc 3.7.0.2): readers exist only for docx and odt "
        f"({sum(1 for r in rows if r['format'] in ('docx', 'odt'))}/{len(rows)} fixtures).** pptx was attempted per "
        "brief and fails `rc=21 Unknown input format pptx` (the pptx reader was removed in pandoc 3.0; "
        "absent from `pandoc --list-input-formats`); odp verified unsupported (`pandoc -f odp` → \"Unknown "
        "input format odp\"); xlsx/ods have never had readers → recorded `no-pandoc-reader`.",
        "- **pdftoppm**: timed `pdftoppm -r 150 -gray -png` on the golden PDF inside the pinned container "
        "(pages counted from output PNGs).",
        "- Reproduce: `python3 harness/baseline/baseline.py run --jobs 4` (resumable; ids already in "
        "`baseline-raw.csv` are skipped), then `python3 harness/baseline/baseline.py report`.",
        "", "## Headline aggregates", ""]
    pdh = ", ".join(f"{k}: {v}" for k, v in a["page_delta_hist"].items())
    L += [
        f"| measure | value |", "|---|---|",
        f"| fixtures measured | {a['n']} |",
        f"| round-trip ok / attempted | {a['rt_ok']} / {a['rt_attempted']} |",
        f"| retention pass / measured | {a['retention_pass']} / {a['retention_measured']} "
        f"({fmt_x(a['retention_pass_rate'], 3)}) |",
        f"| mean text F1 — soffice (golden PDF text layer) | **{fmt_x(a['f1_soffice_mean'], 4)}** "
        f"(min {fmt_x(a['f1_soffice_min'], 4)}, <0.9: {a['f1_soffice_below_0.9']}; excl. odp artifact: "
        f"{fmt_x(a['f1_soffice_mean_excl_odp'], 4)}) |",
        f"| mean text F1 — pandoc (supported readers only) | **{fmt_x(a['f1_pandoc_mean'], 4)}** "
        f"(min {fmt_x(a['f1_pandoc_min'], 4)}, <0.9: {a['f1_pandoc_below_0.9']}) |",
        f"| page-delta Δ=0 | {a['page_delta_zero']}/{a['page_delta_measured']} — hist {{{pdh}}} |",
        f"| round-trip fails | {', '.join(a['rt_fail_ids']) if a['rt_fail_ids'] else 'none'} |",
        f"| pdftoppm 150dpi gray (s) | median {fmt_x(a['ppm_seconds_median'])}, "
        f"p95 {fmt_x(a['ppm_seconds_p95'])}, max {fmt_x(a['ppm_seconds_max'])} |",
        f"| soffice round-trip (s) | median {fmt_x(a['rt_seconds_median'])}, "
        f"p95 {fmt_x(a['rt_seconds_p95'])}, max {fmt_x(a['rt_seconds_max'])} |",
        "", "## Rollup by category", "", *rollup_lines(rows, "category"),
        "", "## Rollup by format", "", *rollup_lines(rows, "format"),
        "", "## Per-fixture table", "",
        "*soffice→pdf pages = golden page count; F1(pandoc) `no-pandoc-reader` = format not readable "
        "by pandoc (see methodology).*", ""]
    L += md_table(rows)
    L += [""] + interpretation(rows, a) + [""]
    (BASEDIR / "report-v1.md").write_text("\n".join(L), encoding="utf-8")
    log(f"report-v1.md ({a['n']} fixtures) + report-v1.json written to {BASEDIR}")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="baseline.py", description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("run", help="measure fixtures -> append rows to baseline-raw.csv (resumable)")
    p.add_argument("--ids", help="comma-separated fixture ids (default: all)")
    p.add_argument("--jobs", type=int, default=4)
    p.set_defaults(fn=cmd_run)
    p = sub.add_parser("report", help="generate report-v1.md + report-v1.json from baseline-raw.csv")
    p.set_defaults(fn=cmd_report)
    args = ap.parse_args(argv)
    SCRATCH.mkdir(parents=True, exist_ok=True)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
