#!/usr/bin/env python3
"""Compare two `dump_canonical` output directories section by section.

`canonical_graph_string` renders a graph as five `## <section>` blocks:
node_type_counts, edge_type_counts, node_identities, node_props, edge_props.
A digest change tells you only *that* the graph moved. This tells you *which
section* moved — the difference between "edges gained properties" and "the edge
set silently changed", which a properties-only golden regeneration would
otherwise bless forever.

Exit code 1 if any section outside those named in --allow differs.

Usage:
  trackC_section_diff.py <before_dir> <after_dir> [--allow edge_props,...]
"""
import sys
from pathlib import Path

SECTIONS = [
    "node_type_counts",
    "edge_type_counts",
    "node_identities",
    "node_props",
    "edge_props",
]


def split_sections(text: str) -> dict[str, list[str]]:
    out: dict[str, list[str]] = {}
    current = None
    for line in text.splitlines():
        if line.startswith("## "):
            current = line[3:].strip()
            out[current] = []
        elif current is not None:
            out[current].append(line)
    return out


def main() -> None:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    allow = set()
    for a in sys.argv[1:]:
        if a.startswith("--allow"):
            allow = set(a.split("=", 1)[1].split(",")) if "=" in a else set()
    if len(args) < 2:
        sys.exit(__doc__)
    before, after = Path(args[0]), Path(args[1])
    if "--allow" in sys.argv:
        allow = set(sys.argv[sys.argv.index("--allow") + 1].split(","))

    corpora = sorted({p.stem for p in before.glob("*.txt")} | {p.stem for p in after.glob("*.txt")})
    violations = 0
    print(f"{'corpus':22s} " + " ".join(f"{s:18s}" for s in SECTIONS))
    for corpus in corpora:
        bp, ap = before / f"{corpus}.txt", after / f"{corpus}.txt"
        if not bp.exists():
            print(f"{corpus:22s} NEW (no before dump — additive corpus)")
            continue
        if not ap.exists():
            print(f"{corpus:22s} MISSING AFTER")
            violations += 1
            continue
        b, a = split_sections(bp.read_text()), split_sections(ap.read_text())
        cells = []
        for section in SECTIONS:
            same = b.get(section) == a.get(section)
            if same:
                cells.append(f"{'same':18s}")
            else:
                nb, na = len(b.get(section, [])), len(a.get(section, []))
                cells.append(f"{'DIFF ' + str(nb) + '->' + str(na):18s}")
                if section not in allow:
                    violations += 1
        print(f"{corpus:22s} " + " ".join(cells))

    print()
    if violations:
        print(f"FAIL: {violations} section change(s) outside --allow={sorted(allow)}")
        sys.exit(1)
    print(f"OK: every section change is inside --allow={sorted(allow)}")


if __name__ == "__main__":
    main()
