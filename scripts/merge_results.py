#!/usr/bin/env python3
"""Combine the per-machine files written by `report --json` into the one file
the results page loads.

Usage: merge_results.py OUT.json RUN.json [RUN.json ...]
"""

import json
import sys

# machines the page should list first, by `meta.arch`
ORDER = {"x86_64": 0, "aarch64": 1}


def main() -> int:
    out, paths = sys.argv[1], sys.argv[2:]
    runs = []
    for path in paths:
        with open(path, encoding="utf-8") as f:
            run = json.load(f)
        if not run.get("subjects"):
            print(f"skipping {path}: no results", file=sys.stderr)
            continue
        runs.append(run)
    if not runs:
        print("no runs to publish", file=sys.stderr)
        return 1
    runs.sort(key=lambda r: (ORDER.get(r["meta"].get("arch"), 9), r["meta"].get("label") or ""))
    with open(out, "w", encoding="utf-8") as f:
        json.dump({"runs": runs}, f)
    print(f"wrote {out}: " + ", ".join(r["meta"].get("label") or r["meta"].get("arch", "?") for r in runs))
    return 0


if __name__ == "__main__":
    sys.exit(main())
