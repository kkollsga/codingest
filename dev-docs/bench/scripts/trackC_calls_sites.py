#!/usr/bin/env python3
"""Extract the call-site source text behind every CALLS edge in a
`codingest_stats --dump-calls` dump, so each edge can be labeled true/false by
reading what the caller actually wrote.

The dump gives (caller, callee, caller_file, callee_file, call_lines). This
script opens each caller file *in the pinned checkout* and pulls the token
immediately before the callee's short name at each recorded line — the receiver
expression. That receiver is what decides the verdict: a bare `fetch(` is the
web global (edge false), `catalog.fetch(` where `catalog` is the imported
module is the project function (edge true).

Usage:
  trackC_calls_sites.py <dump.json> <repo> <name1,name2,...> <out.csv>

Refuses to run against a repo whose HEAD is not the pinned rev, or whose tree is
dirty — a label set is only reusable if the source it was read from is fixed.
"""
import csv
import json
import re
import subprocess
import sys
from pathlib import Path

PINNED_REV = "1e17856ba4b5b052650c8115060852f3f023844e"


def short_name(qname: str) -> str:
    cut = 0
    for sep in ("::", ".", "/"):
        i = qname.rfind(sep)
        if i >= 0 and i + len(sep) > cut:
            cut = i + len(sep)
    return qname[cut:]


def check_pin(repo: Path) -> None:
    head = subprocess.run(
        ["git", "-C", str(repo), "rev-parse", "HEAD"],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    if head != PINNED_REV:
        sys.exit(f"PIN FAIL: {repo} is at {head}, expected {PINNED_REV}")
    dirty = subprocess.run(
        ["git", "-C", str(repo), "status", "--porcelain"],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    if dirty:
        sys.exit(f"PIN FAIL: {repo} has a dirty tree ({len(dirty.splitlines())} entries)")
    print(f"PIN OK {head} clean", file=sys.stderr)


def main() -> None:
    if len(sys.argv) != 5:
        sys.exit(__doc__)
    dump_path, repo, names_arg, out_path = sys.argv[1:]
    repo = Path(repo)
    check_pin(repo)

    names = {n.strip() for n in names_arg.split(",") if n.strip()}
    rows = [r for r in json.load(open(dump_path)) if short_name(r["callee"]) in names]

    cache: dict[str, list[str]] = {}

    def lines_of(path: str) -> list[str]:
        if path not in cache:
            try:
                cache[path] = (repo / path).read_text(errors="replace").splitlines()
            except OSError:
                cache[path] = []
        return cache[path]

    out = []
    for r in rows:
        name = short_name(r["callee"])
        src = lines_of(r["caller_file"])
        sites, receivers = [], set()
        for raw in r["call_lines"].split(","):
            if not raw.strip():
                continue
            n = int(raw)
            text = src[n - 1].strip() if 0 < n <= len(src) else "<line unavailable>"
            sites.append(f"{n}:{text}")
            # Receiver = the dotted expression immediately preceding `name(`.
            for m in re.finditer(r"([\w$.\[\]\"']*?)\.?\b" + re.escape(name) + r"\s*[(<]", text):
                receivers.add(m.group(1))
            if not any(
                re.search(r"\b" + re.escape(name) + r"\s*[(<]", text) for _ in [0]
            ):
                receivers.add("<name-not-on-line>")
        out.append({
            "callee": r["callee"],
            "caller": r["caller"],
            "caller_file": r["caller_file"],
            "callee_file": r["callee_file"],
            "call_lines": r["call_lines"],
            "receivers": "|".join(sorted(x for x in receivers if x != "")) or "<bare>",
            "sites": " ⏎ ".join(sites),
            "verdict": "",
        })

    out.sort(key=lambda d: (d["callee"], d["caller"]))
    with open(out_path, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=list(out[0].keys()))
        w.writeheader()
        w.writerows(out)
    print(f"{len(out)} rows -> {out_path}", file=sys.stderr)


if __name__ == "__main__":
    main()
