#!/usr/bin/env python3
"""Verify a seeded-random sample of the NEW TypeScript File→File IMPORTS edges
against the source at the pinned rev.

Raising an edge count is trivially achievable by manufacturing wrong edges, so
the count is not evidence on its own. This opens each sampled edge's source
file, finds the import/export-from specifier the edge must have come from, and
reports whether that specifier plausibly designates the target under TS
resolution rules — relative, `paths` alias, or workspace package.

Also enforces the structural cap: an edge per specifier is the maximum, so
File→File IMPORTS must not exceed the number of captured import/export-from
statements.

Usage:
  trackC_verify_imports.py <repo> <edges.tsv> [--seed N] [--sample N]

`edges.tsv` is `source<TAB>target`, one edge per line (a `codingest query`
export).
"""
import json
import random
import re
import subprocess
import sys
from pathlib import Path

PINNED_REV = "1e17856ba4b5b052650c8115060852f3f023844e"
SEED = 20260801
SAMPLE = 30
TS_EXT = (".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts")
SPEC_RE = re.compile(
    r"""^\s*(?:import|export)\b[^;\n]*?\bfrom\s*['"]([^'"]+)['"]|^\s*import\s*['"]([^'"]+)['"]""",
    re.MULTILINE,
)


def check_pin(repo: Path) -> None:
    head = subprocess.run(["git", "-C", str(repo), "rev-parse", "HEAD"],
                          capture_output=True, text=True, check=True).stdout.strip()
    if head != PINNED_REV:
        sys.exit(f"PIN FAIL: {repo} is at {head}, expected {PINNED_REV}")
    if subprocess.run(["git", "-C", str(repo), "status", "--porcelain"],
                      capture_output=True, text=True, check=True).stdout.strip():
        sys.exit(f"PIN FAIL: {repo} has a dirty tree")
    print(f"PIN OK {head} clean", file=sys.stderr)


def specifiers(text: str) -> list[str]:
    return [a or b for a, b in SPEC_RE.findall(text)]


def strip_ext(p: str) -> str:
    for e in TS_EXT:
        if p.endswith(e):
            return p[: -len(e)]
    return p


def norm(base: str, rel: str) -> str:
    parts = [p for p in base.split("/") if p]
    for part in rel.split("/"):
        if part in ("", "."):
            continue
        if part == "..":
            if parts:
                parts.pop()
        else:
            parts.append(part)
    return "/".join(parts)


def packages(repo: Path) -> dict[str, str]:
    out: dict[str, str] = {}
    for pj in repo.rglob("package.json"):
        rel = pj.relative_to(repo)
        if any(part == "node_modules" or part.startswith(".") for part in rel.parts):
            continue
        try:
            name = json.loads(pj.read_text()).get("name")
        except Exception:
            continue
        if name:
            out.setdefault(name, str(rel.parent) if str(rel.parent) != "." else "")
    return out


def main() -> None:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if len(args) < 2:
        sys.exit(__doc__)
    repo, tsv = Path(args[0]), Path(args[1])
    check_pin(repo)

    edges = []
    for line in tsv.read_text().splitlines():
        if "\t" not in line:
            continue
        src, tgt = line.split("\t")[:2]
        if src.endswith(TS_EXT) and tgt.endswith(TS_EXT):
            edges.append((src, tgt))
    print(f"TS→TS File→File IMPORTS edges: {len(edges)}", file=sys.stderr)

    # Structural cap: one edge per specifier is the maximum.
    total_specs = 0
    src_files = {s for s, _ in edges}
    for f in src_files:
        try:
            total_specs += len(specifiers((repo / f).read_text(errors="replace")))
        except OSError:
            pass
    print(f"specifiers in those {len(src_files)} source files: {total_specs}")
    print(f"structural cap  edges <= specifiers: {len(edges)} <= {total_specs} -> "
          f"{'OK' if len(edges) <= total_specs else 'VIOLATED'}")

    pkgs = packages(repo)
    rng = random.Random(SEED)
    sample = rng.sample(edges, min(SAMPLE, len(edges)))
    ok = 0
    print(f"\nseeded sample (seed={SEED}, n={len(sample)}):")
    for src, tgt in sample:
        text = (repo / src).read_text(errors="replace")
        base = src.rsplit("/", 1)[0] if "/" in src else ""
        want = strip_ext(tgt)
        want_index = want[: -len("/index")] if want.endswith("/index") else None
        matched = None
        for spec in specifiers(text):
            cands: list[str] = []
            if spec.startswith("."):
                cands.append(strip_ext(norm(base, spec)))
            else:
                for name, d in pkgs.items():
                    if spec == name or spec.startswith(name + "/"):
                        rest = spec[len(name):].lstrip("/")
                        cands += ([d, f"{d}/src", f"{d}/src/index"] if not rest
                                  else [f"{d}/{rest}", f"{d}/src/{rest}"])
                # `paths` aliases are directory-scoped; approximate by matching
                # the alias tail against the target's tail.
                if spec.startswith("@/") or spec.startswith("#"):
                    cands.append(spec)
            for c in cands:
                c = strip_ext(c)
                if c == want or c == want_index or (want_index and c == want_index):
                    matched = spec
                    break
                if (spec.startswith("@/") or spec.startswith("#")) and want.endswith(
                    spec.split("/", 1)[-1]
                ):
                    matched = spec
                    break
            if matched:
                break
        if matched:
            ok += 1
            print(f"  OK   {src}\n         -> {tgt}   via {matched!r}")
        else:
            print(f"  MISS {src}\n         -> {tgt}   (no specifier reproduced it)")
    print(f"\nverified {ok}/{len(sample)} (bar: >= 28/30)")


if __name__ == "__main__":
    main()
