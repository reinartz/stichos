# SPDX-License-Identifier: MIT OR Apache-2.0
import argparse
import json
import sys

from . import builders  # noqa: F401  (registers builders)
from .core import BUDGET, BUILDERS, build_all, select, verify_all


def main() -> int:
    ap = argparse.ArgumentParser(prog="gen", description="stichos fixture generator (HAR-02)")
    sub = ap.add_subparsers(dest="cmd", required=True)

    b = sub.add_parser("build", help="build fixtures into corpus/")
    b.add_argument("--format", choices=["docx", "xlsx", "pptx", "odt", "ods", "odp"])
    b.add_argument("--category")

    v = sub.add_parser("verify", help="schema + sha + feature + determinism check")
    v.add_argument("--no-blobs", action="store_true",
                   help="manifest-only mode (blobs gitignored): schema + id/category/budget/reference consistency")

    s = sub.add_parser("status", help="budget vs builders vs built manifests")
    args = ap.parse_args()

    if args.cmd == "build":
        build_all(args.format, args.category)
        return 0
    if args.cmd == "verify":
        problems = verify_all(no_blobs=args.no_blobs)
        for p in problems:
            print("FAIL", p)
        print(f"verify: {len(problems)} problem(s)")
        return 1 if problems else 0
    if args.cmd == "status":
        import pathlib

        built = {p.stem for p in (pathlib.Path(__file__).resolve().parents[3] / "corpus" / "manifests").glob("*.json")}
        missing_impl = []
        for (c, f), n in sorted(BUDGET.items()):
            have = sum(1 for i in range(1, n + 1) if f"{c}-{f}-{i:03d}" in built)
            impl = (c, f) in BUILDERS
            if have < n:
                missing_impl.append(f"{c}/{f}: {have}/{n} built{'' if impl else ' (NO BUILDER)'}")
        print(json.dumps({"budget_total": sum(BUDGET.values()), "built": len(built), "gaps": missing_impl}, indent=2))
        return 0
    return 2


if __name__ == "__main__":
    sys.exit(main())
