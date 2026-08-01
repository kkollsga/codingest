#!/usr/bin/env python3
"""Score Track C's pre-registered Phase 5 decision rule against the labeled
CALLS truth set.

The rule (registered in the plan BEFORE it was measured, deliberately, so the
threshold could not be tuned to the result):

  Candidate policy — DROP a name-resolved CROSS-FILE edge when
      resolution in {unique_name, lang_group, global_fallback}
      AND import_backed == false
      AND the caller's language group is go_ts_js
  Adopt the drop IFF, on the labeled set:
      >= 80% of labeled-FALSE edges are removed
      AND 100% of labeled-TRUE edges are retained.
  Otherwise ship annotation-only. That outcome is a valid completion, not a
  failure.

Usage: trackC_score_rule.py <labels.csv> <dump.json>
"""
import csv
import json
import sys

GATED_TIERS = {"unique_name", "lang_group", "global_fallback"}


def lang_group(qname: str) -> str:
    """Mirror of `call_edges::infer_lang_group`."""
    if "::" in qname:
        return "rust_cpp"
    if "/" in qname:
        return "go_ts_js"
    return "python_java"


def dropped(row: dict) -> bool:
    if row["caller_file"] == row["callee_file"]:
        return False
    if row["resolution"] not in GATED_TIERS:
        return False
    if row["import_backed"]:
        return False
    return lang_group(row["caller"]) == "go_ts_js"


def main() -> None:
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    labels = {(r["caller"], r["callee"]): r["verdict"] for r in csv.DictReader(open(sys.argv[1]))}
    rows = {(r["caller"], r["callee"]): r for r in json.load(open(sys.argv[2]))}

    tp_total = tp_kept = fp_total = fp_dropped = 0
    kept_false: list[dict] = []
    dropped_true: list[dict] = []
    for key, verdict in labels.items():
        row = rows.get(key)
        if row is None:
            # An edge the labeled build had and this build does not: score it
            # as removed, per the plan.
            is_dropped = True
            row = {"caller": key[0], "callee": key[1], "resolution": "<absent>",
                   "candidates": 0, "import_backed": False, "caller_file": "", "callee_file": ""}
        else:
            is_dropped = dropped(row)
        if verdict == "true":
            tp_total += 1
            if is_dropped:
                dropped_true.append(row)
            else:
                tp_kept += 1
        else:
            fp_total += 1
            if is_dropped:
                fp_dropped += 1
            else:
                kept_false.append(row)

    fp_rate = fp_dropped / fp_total if fp_total else 0.0
    tp_rate = tp_kept / tp_total if tp_total else 1.0
    print(f"labeled set: {tp_total} true / {fp_total} false ({len(labels)} rows)")
    print(f"labeled-FALSE removed : {fp_dropped}/{fp_total} = {fp_rate:.1%}   (rule needs >= 80%)")
    print(f"labeled-TRUE  retained: {tp_kept}/{tp_total} = {tp_rate:.1%}   (rule needs 100%)")
    verdict_ok = fp_rate >= 0.80 and tp_rate == 1.0
    print()
    print("DECISION:", "ADOPT the drop" if verdict_ok else "DO NOT adopt — annotation only")

    if dropped_true:
        print(f"\n{len(dropped_true)} labeled-TRUE edge(s) the rule would DESTROY:")
        for r in dropped_true:
            print(f"  {r['caller']}\n    -> {r['callee']}   [{r['resolution']}, "
                  f"candidates={r['candidates']}, import_backed={r['import_backed']}]")

    if kept_false:
        from collections import Counter
        why = Counter()
        for r in kept_false:
            if r["caller_file"] == r["callee_file"]:
                why["same file"] += 1
            elif r["resolution"] not in GATED_TIERS:
                why[f"tier {r['resolution']} not gated"] += 1
            elif r["import_backed"]:
                why["import_backed=true"] += 1
            else:
                why["not go_ts_js"] += 1
        print(f"\n{len(kept_false)} labeled-FALSE edge(s) the rule would KEEP, by reason:")
        for reason, n in why.most_common():
            print(f"  {n:5d}  {reason}")

    sys.exit(0 if verdict_ok else 2)


if __name__ == "__main__":
    main()
