#!/usr/bin/env python3
"""Score the Phase-1 closure-scoped-definitions spike.

Reads the per-corpus artifacts `nested_spike_run.sh` produced in OUTDIR and
writes the numbers the plan's go/no-go rests on:

  * node-count growth under D1, **with and without** the factory unwrap
    (the abandonment section's narrowing fallback needs the two separated),
  * the size distribution of captured nested functions (median LOC is the
    go/no-go number),
  * naive name-collision growth in the global `name_lookup`, top-20 bare
    names before/after (naive = pre-D3; D3's same-file gate is what actually
    neutralises this, and the point of measuring it is to size what D3 has
    to hold back),
  * how many `<anonL{line}>` positional markers D2 would have to mint,
  * a fidelity check of the prototype against the real graph.

Usage: nested_spike_report.py OUTDIR label=/abs/repo [label=/abs/repo ...]
"""

from __future__ import annotations

import csv
import json
import statistics
import sys
from collections import Counter
from pathlib import Path

# Plan thresholds (dev-docs/plans/closure-scoped-definitions.md, Phase 1
# "Exit criteria"). Not tunable from the command line on purpose: the point
# of a go/no-go is that the bar is fixed before the numbers arrive.
NODE_GROWTH_CEILING = {"opencode": 0.12}
NODE_GROWTH_CEILING_DEFAULT = 0.10
MEDIAN_LOC_FLOOR = 3


# ── candidate narrowings of D1-3 (the factory unwrap) ────────────────────
#
# All of them keep D1-3's "exactly one function literal across the chain"
# precondition and differ only in which *call shapes* they accept. They exist
# because the rule as written in the plan also accepts `const names =
# users.map(u => u.name)` — the binding there is the call's *result*, an
# array, not a function.
FACTORY_RULES: dict[str, callable] = {
    # D1-3 verbatim.
    "F0_as_planned": lambda f: True,
    # Curried application only: `Effect.fn("n")(function*…)`.
    "F1_curried_only": lambda f: f["curried"],
    # The literal is a generator — the Effect-TS / redux-saga shape.
    "F2_generator_literal": lambda f: False,  # replaced below (needs the capture)
    # The callee is not a method on a *value*: rejects `arr.map`,
    # `results.filter`, `this[k].map`; keeps `Effect.fn`, `Layer.effect`,
    # `defineStore`, `memoize`.
    "F3_non_value_receiver": lambda f: f["callee_shape"]
    in ("identifier", "member.Capitalized", "curried"),
    "F4_curried_or_generator": lambda f: False,  # replaced below
    # F3 ∩ F4 — the narrowest that still covers the whole opencode motivator.
    "F5_non_value_receiver_and_fnlike": lambda f: False,  # replaced below
}


def factory_rule_matches(rule: str, cap: dict) -> bool:
    f = cap.get("factory") or {}
    gen = cap.get("literal_kind") == "generator_function"
    if rule == "F0_as_planned":
        return True
    if rule == "F1_curried_only":
        return bool(f.get("curried"))
    if rule == "F2_generator_literal":
        return gen
    if rule == "F3_non_value_receiver":
        return f.get("callee_shape") in ("identifier", "member.Capitalized", "curried")
    if rule == "F4_curried_or_generator":
        return bool(f.get("curried")) or gen
    if rule == "F5_non_value_receiver_and_fnlike":
        return (
            f.get("callee_shape") in ("identifier", "member.Capitalized", "curried")
        ) and (bool(f.get("curried")) or gen)
    raise KeyError(rule)


# ── candidate narrowings of the *scope* rule (D1 items 1/2) ──────────────
#
# `all` is D1 as written: a named binding at any depth, including one that
# sits inside an anonymous callback. `named_chain_only` additionally requires
# every enclosing scope to be named — which is also what makes D2's
# `<anonL{line}>` positional marker unnecessary, so the determinism question
# stops being something to manage and becomes something that cannot arise.
SCOPE_RULES = {
    "all": lambda c: True,
    "named_chain_only": lambda c: not c["anon_in_chain"],
    "depth_le_1": lambda c: c["depth"] <= 1,
    "named_chain_and_depth_le_1": lambda c: not c["anon_in_chain"] and c["depth"] <= 1,
}


def read_csv(path: Path) -> list[dict]:
    with path.open(newline="") as fh:
        return list(csv.DictReader(fh))


def pct(x: float) -> str:
    return f"{x * 100:.2f} %"


def quantiles(values: list[int]) -> dict:
    if not values:
        return {"n": 0}
    s = sorted(values)
    return {
        "n": len(s),
        "min": s[0],
        "p25": s[max(0, (len(s) - 1) // 4)],
        "median": statistics.median(s),
        "p75": s[min(len(s) - 1, (3 * (len(s) - 1)) // 4)],
        "max": s[-1],
        "mean": round(statistics.fmean(s), 2),
        "under_3_loc": sum(1 for v in s if v < MEDIAN_LOC_FLOOR),
        "under_3_loc_share": round(
            sum(1 for v in s if v < MEDIAN_LOC_FLOOR) / len(s), 4
        ),
    }


def score(outdir: Path, label: str) -> dict:
    corpus = json.loads((outdir / f"{label}.corpus.json").read_text())
    spike = json.loads((outdir / f"{label}.spike.json").read_text())
    node_counts = {
        r["node_type"].strip('[]"'): int(r["n"])
        for r in read_csv(outdir / f"{label}.nodecounts.csv")
    }
    total_nodes = sum(node_counts.values())
    docs_counts_path = outdir / f"{label}.nodecounts-docs.csv"
    total_nodes_with_docs = (
        sum(int(r["n"]) for r in read_csv(docs_counts_path))
        if docs_counts_path.exists()
        else total_nodes
    )
    graph_fns = read_csv(outdir / f"{label}.functions.csv")
    graph_consts = read_csv(outdir / f"{label}.constants.csv")
    caps = spike["captures"]

    # ── node growth ──────────────────────────────────────────────────────
    # today == "none"     -> a brand-new node (+1)
    # today == "constant" -> the Constant this binding already produces is
    #                        replaced by a Function: net 0, not a growth term
    # today == "function" -> already a Function node: net 0
    new_all = [c for c in caps if c["today"] == "none"]
    new_no_factory = [c for c in new_all if c["bucket"] != "factory"]
    new_factory_only = [c for c in new_all if c["bucket"] == "factory"]
    swaps = [c for c in caps if c["today"] == "constant"]
    swaps_factory = [c for c in swaps if c["bucket"] == "factory"]

    growth = {
        "d1_full": len(new_all) / total_nodes,
        "d1_without_factory_unwrap": len(new_no_factory) / total_nodes,
        "narrowed_factory_only": len(new_factory_only) / total_nodes,
    }
    ceiling = NODE_GROWTH_CEILING.get(label, NODE_GROWTH_CEILING_DEFAULT)

    # ── candidate narrowings of the factory unwrap ───────────────────────
    # Scored as "D1 items 1/2/4 (which every variant keeps) + this factory
    # rule", so each row is a whole, shippable inclusion criterion.
    factory_caps = [c for c in caps if c["bucket"] == "factory"]
    factory_rules = {}
    for rule in FACTORY_RULES:
        kept = [c for c in factory_caps if factory_rule_matches(rule, c)]
        kept_new = [c for c in kept if c["today"] == "none"]
        combined_new = new_no_factory + kept_new
        factory_rules[rule] = {
            "factory_captures_kept": len(kept),
            "factory_captures_rejected": len(factory_caps) - len(kept),
            "factory_new_nodes": len(kept_new),
            "factory_constant_swaps": sum(1 for c in kept if c["today"] == "constant"),
            "factory_loc": quantiles([c["loc"] for c in kept_new]),
            "combined_new_nodes": len(combined_new),
            "combined_growth": round(len(combined_new) / total_nodes, 5),
            "combined_loc": quantiles([c["loc"] for c in combined_new]),
            "top_wrappers": Counter(
                c["wrapped_by"] for c in kept if c["wrapped_by"]
            ).most_common(10),
        }

    # ── the full criterion matrix: factory rule x scope rule ─────────────
    matrix = []
    for frule in ("none", "F0_as_planned", "F5_non_value_receiver_and_fnlike"):
        for srule, spred in SCOPE_RULES.items():
            sel = []
            for c in new_all:
                if c["bucket"] == "factory":
                    if frule == "none":
                        continue
                    if not factory_rule_matches(frule, c):
                        continue
                if not spred(c):
                    continue
                sel.append(c)
            swapped = [
                c
                for c in swaps
                if spred(c)
                and (
                    c["bucket"] != "factory"
                    or (frule != "none" and factory_rule_matches(frule, c))
                )
            ]
            locs = quantiles([c["loc"] for c in sel])
            g = len(sel) / total_nodes
            qn = Counter(c["qualified_name"] for c in sel + swapped)
            matrix.append(
                {
                    "factory_rule": frule,
                    "scope_rule": srule,
                    "new_nodes": len(sel),
                    "growth": round(g, 5),
                    "constant_swaps": len(swapped),
                    "median_loc": locs.get("median"),
                    "under_3_loc_share": locs.get("under_3_loc_share"),
                    "anon_markers_required": 0
                    if srule.startswith("named_chain")
                    else len({c["parent_scope"] for c in sel if c["anon_in_chain"]}),
                    "duplicate_qnames": sum(1 for v in qn.values() if v > 1),
                    "growth_ok": g <= ceiling,
                    "median_ok": (locs.get("median") or 0) >= MEDIAN_LOC_FLOOR,
                    "passes": g <= ceiling
                    and (locs.get("median") or 0) >= MEDIAN_LOC_FLOOR,
                }
            )

    # ── size distribution of the captured nested functions ───────────────
    loc = {
        "d1_full_new_nodes": quantiles([c["loc"] for c in new_all]),
        "d1_without_factory_unwrap": quantiles([c["loc"] for c in new_no_factory]),
        "narrowed_factory_only": quantiles([c["loc"] for c in new_factory_only]),
        "all_captures": quantiles([c["loc"] for c in caps]),
    }

    # ── naive name_lookup collision growth ───────────────────────────────
    before = Counter(r["name"] for r in graph_fns)
    after = Counter(before)
    after.update(c["name"] for c in new_all)
    collisions = {
        "distinct_names_before": len(before),
        "distinct_names_after": len(after),
        "multi_candidate_names_before": sum(1 for v in before.values() if v > 1),
        "multi_candidate_names_after": sum(1 for v in after.values() if v > 1),
        "unique_names_that_become_ambiguous": sum(
            1 for n, v in before.items() if v == 1 and after[n] > 1
        ),
        "max_bucket_before": max(before.values(), default=0),
        "max_bucket_after": max(after.values(), default=0),
        "mean_bucket_before": round(
            sum(before.values()) / len(before), 3) if before else 0,
        "mean_bucket_after": round(
            sum(after.values()) / len(after), 3) if after else 0,
        "top20_before": before.most_common(20),
        "top20_after": [(n, after[n]) for n, _ in after.most_common(20)],
    }

    # ── D2 anon markers ──────────────────────────────────────────────────
    # Without the factory unwrap every factory binding's literal turns into
    # an anonymous scope instead, so the marker count is a floor, not a total.
    anon = {
        "markers_used_d1_full": len(spike["anon_markers_used"]),
        "captures_with_anon_in_chain": sum(1 for c in caps if c["anon_in_chain"]),
        "anon_scopes_entered_by_parent": spike["anon_scopes_entered_by_parent"],
        "extra_markers_if_factory_unwrap_dropped": len(swaps_factory)
        + len(new_factory_only),
    }

    # ── fidelity: does the prototype's model of "today" match the graph? ──
    fn_exact = {(r["file_path"], r["name"], r["line_number"]) for r in graph_fns}
    fn_loose = {(r["file_path"], r["name"]) for r in graph_fns}
    const_loose = {(r["file_path"], r["name"]) for r in graph_consts}
    claim_fn = [c for c in caps if c["today"] == "function"]
    claim_const = [c for c in caps if c["today"] == "constant"]
    fidelity = {
        "claimed_existing_function": len(claim_fn),
        "matched_exact_file_name_line": sum(
            1
            for c in claim_fn
            if (c["file"], c["name"], str(c["start_line"])) in fn_exact
        ),
        "matched_file_name": sum(
            1 for c in claim_fn if (c["file"], c["name"]) in fn_loose
        ),
        "claimed_existing_constant": len(claim_const),
        "constant_matched_file_name": sum(
            1 for c in claim_const if (c["file"], c["name"]) in const_loose
        ),
        # A capture the prototype calls new must NOT already be a graph node.
        "new_nodes_colliding_with_an_existing_function": sum(
            1 for c in new_all if (c["file"], c["name"], str(c["start_line"])) in fn_exact
        ),
    }

    by_bucket = Counter(c["bucket"] for c in caps)
    by_depth = Counter(min(c["depth"], 6) for c in new_all)

    return {
        "label": label,
        "commit": corpus["commit"],
        "dirty_paths": corpus["dirty_paths"],
        "files_walked": spike["files_walked"],
        "files_unreadable": spike["files_unreadable"],
        "baseline_nodes_total": total_nodes,
        "baseline_nodes_total_with_docs": total_nodes_with_docs,
        "baseline_node_counts": node_counts,
        "captures_total": len(caps),
        "captures_by_bucket": dict(by_bucket),
        "new_nodes_by_depth": {str(k): v for k, v in sorted(by_depth.items())},
        "new_nodes_d1_full": len(new_all),
        "new_nodes_without_factory_unwrap": len(new_no_factory),
        "new_nodes_narrowed_factory_only": len(new_factory_only),
        "constant_to_function_swaps": len(swaps),
        "in_namespace_captures": sum(1 for c in caps if c["in_namespace"]),
        "fidelity": fidelity,
        "factory_rule_candidates": factory_rules,
        "criterion_matrix": matrix,
        "factory_wrapper_histogram": Counter(
            c["wrapped_by"] for c in factory_caps if c["wrapped_by"]
        ).most_common(25),
        "node_growth": {k: round(v, 5) for k, v in growth.items()},
        "node_growth_ceiling": ceiling,
        "loc_distribution": loc,
        "collisions": collisions,
        "anon_markers": anon,
        "duplicate_qnames": len(spike["duplicate_qnames"]),
        "duplicate_qname_examples": list(spike["duplicate_qnames"].items())[:10],
        "excluded_shapes": spike["excluded"],
        "verdict_inputs": {
            "growth_full_within_ceiling": growth["d1_full"] <= ceiling,
            "growth_no_factory_within_ceiling": growth["d1_without_factory_unwrap"]
            <= ceiling,
            "growth_narrowed_within_ceiling": growth["narrowed_factory_only"]
            <= ceiling,
            "median_loc_full": loc["d1_full_new_nodes"].get("median"),
            "median_loc_meets_floor": (loc["d1_full_new_nodes"].get("median") or 0)
            >= MEDIAN_LOC_FLOOR,
            "median_loc_narrowed": loc["narrowed_factory_only"].get("median"),
            "median_loc_narrowed_meets_floor": (
                loc["narrowed_factory_only"].get("median") or 0
            )
            >= MEDIAN_LOC_FLOOR,
        },
    }


def summary_table(results: list[dict]) -> str:
    lines = []
    lines.append("# Phase-1 spike — closure-scoped definitions (D1/D2)")
    lines.append("")
    lines.append(
        "Corpora are `codingest build --no-tests` (docs off) — the same "
        "configuration `codingest_stats` measures, and the *smaller* node "
        "denominator of the two available, so every growth figure below is "
        "the conservative one."
    )
    lines.append("")
    lines.append("## Corpora")
    lines.append("")
    lines.append(
        "| corpus | commit | ts/js files walked | baseline nodes (docs off, "
        "the denominator used) | baseline nodes (docs on, sensitivity) |"
    )
    lines.append("|---|---|---:|---:|---:|")
    for r in results:
        lines.append(
            f"| {r['label']} | `{r['commit']}` | {r['files_walked']} | "
            f"{r['baseline_nodes_total']} | "
            f"{r['baseline_nodes_total_with_docs']} |"
        )
    lines.append("")
    lines.append("## Node-count growth (the ceiling test)")
    lines.append("")
    lines.append(
        "| corpus | ceiling | D1 full | D1 w/o factory unwrap | narrowed: "
        "factory only | Constant→Function swaps (net 0) |"
    )
    lines.append("|---|---:|---:|---:|---:|---:|")
    for r in results:
        g = r["node_growth"]
        lines.append(
            f"| {r['label']} | {pct(r['node_growth_ceiling'])} | "
            f"{r['new_nodes_d1_full']} ({pct(g['d1_full'])}) | "
            f"{r['new_nodes_without_factory_unwrap']} "
            f"({pct(g['d1_without_factory_unwrap'])}) | "
            f"{r['new_nodes_narrowed_factory_only']} "
            f"({pct(g['narrowed_factory_only'])}) | "
            f"{r['constant_to_function_swaps']} |"
        )
    lines.append("")
    lines.append("## Size of the captured functions (the noise test)")
    lines.append("")
    lines.append(
        "| corpus | population | n | p25 | **median** | p75 | < 3 LOC | share < 3 LOC |"
    )
    lines.append("|---|---|---:|---:|---:|---:|---:|---:|")
    for r in results:
        for key, name in (
            ("d1_full_new_nodes", "D1 full"),
            ("d1_without_factory_unwrap", "D1 w/o factory"),
            ("narrowed_factory_only", "factory only"),
        ):
            d = r["loc_distribution"][key]
            if not d.get("n"):
                lines.append(f"| {r['label']} | {name} | 0 | | | | | |")
                continue
            lines.append(
                f"| {r['label']} | {name} | {d['n']} | {d['p25']} | "
                f"**{d['median']}** | {d['p75']} | {d['under_3_loc']} | "
                f"{pct(d['under_3_loc_share'])} |"
            )
    lines.append("")
    lines.append("## Candidate narrowings of the factory unwrap (D1-3)")
    lines.append("")
    lines.append(
        "Every row keeps D1 items 1/2/4 unchanged and swaps only the factory "
        "rule, so each row is a complete inclusion criterion. `combined` = "
        "the whole criterion's new-node count / growth / median LOC."
    )
    lines.append("")
    lines.append(
        "| corpus | factory rule | factory kept | factory new nodes | factory "
        "median LOC | combined new nodes | combined growth | ceiling | "
        "combined median LOC |"
    )
    lines.append("|---|---|---:|---:|---:|---:|---:|---:|---:|")
    for r in results:
        for rule, d in r["factory_rule_candidates"].items():
            ok = "✅" if d["combined_growth"] <= r["node_growth_ceiling"] else "❌"
            lines.append(
                f"| {r['label']} | `{rule}` | {d['factory_captures_kept']} | "
                f"{d['factory_new_nodes']} | "
                f"{d['factory_loc'].get('median', '—')} | "
                f"{d['combined_new_nodes']} | {pct(d['combined_growth'])} {ok} | "
                f"{pct(r['node_growth_ceiling'])} | "
                f"{d['combined_loc'].get('median', '—')} |"
            )
    lines.append("")
    for r in results:
        lines.append(f"### {r['label']} — factory wrappers matched by D1-3 as written")
        lines.append("")
        lines.append("| wrapper | n |")
        lines.append("|---|---:|")
        for name, n in r["factory_wrapper_histogram"]:
            flat = " ".join(str(name).split())
            lines.append(f"| `{flat}` | {n} |")
        lines.append("")
    lines.append("## Criterion matrix — factory rule x scope rule")
    lines.append("")
    lines.append(
        "The full go/no-go surface. A row PASSES only if node growth is "
        f"within the corpus ceiling AND the median captured function is "
        f">= {MEDIAN_LOC_FLOOR} LOC. `named_chain_only` drops bindings whose "
        "scope chain passes through an anonymous callback — which is also "
        "what drives `anon markers` to 0, retiring D2's line-numbered "
        "positional marker entirely."
    )
    lines.append("")
    lines.append(
        "| corpus | factory rule | scope rule | new nodes | growth | median "
        "LOC | < 3 LOC | anon markers | dup qnames | PASS |"
    )
    lines.append("|---|---|---|---:|---:|---:|---:|---:|---:|---|")
    for r in results:
        for m in r["criterion_matrix"]:
            lines.append(
                f"| {r['label']} | `{m['factory_rule']}` | "
                f"`{m['scope_rule']}` | {m['new_nodes']} | "
                f"{pct(m['growth'])} | {m['median_loc']} | "
                f"{pct(m['under_3_loc_share'] or 0)} | "
                f"{m['anon_markers_required']} | {m['duplicate_qnames']} | "
                f"{'✅' if m['passes'] else '❌'} |"
            )
    lines.append("")
    lines.append("## Naive `name_lookup` collision growth (pre-D3)")
    lines.append("")
    lines.append(
        "| corpus | distinct names | multi-candidate names | unique→ambiguous "
        "| max bucket | mean bucket |"
    )
    lines.append("|---|---|---|---:|---|---|")
    for r in results:
        c = r["collisions"]
        lines.append(
            f"| {r['label']} | {c['distinct_names_before']} → "
            f"{c['distinct_names_after']} | "
            f"{c['multi_candidate_names_before']} → "
            f"{c['multi_candidate_names_after']} | "
            f"{c['unique_names_that_become_ambiguous']} | "
            f"{c['max_bucket_before']} → {c['max_bucket_after']} | "
            f"{c['mean_bucket_before']} → {c['mean_bucket_after']} |"
        )
    lines.append("")
    for r in results:
        c = r["collisions"]
        lines.append(f"### {r['label']} — top-20 bare names")
        lines.append("")
        lines.append("| # | before | n | after | n |")
        lines.append("|---:|---|---:|---|---:|")
        for i in range(20):
            b = c["top20_before"][i] if i < len(c["top20_before"]) else ("", "")
            a = c["top20_after"][i] if i < len(c["top20_after"]) else ("", "")
            lines.append(f"| {i + 1} | `{b[0]}` | {b[1]} | `{a[0]}` | {a[1]} |")
        lines.append("")
    lines.append("## D2 positional markers + determinism")
    lines.append("")
    lines.append(
        "| corpus | `<anonL{line}>` markers needed | captures under an anon "
        "scope | duplicate qnames | walk byte-stable |"
    )
    lines.append("|---|---:|---:|---:|---|")
    for r in results:
        a = r["anon_markers"]
        lines.append(
            f"| {r['label']} | {a['markers_used_d1_full']} | "
            f"{a['captures_with_anon_in_chain']} | {r['duplicate_qnames']} | "
            f"{r['determinism']} |"
        )
    lines.append("")
    lines.append("## Prototype fidelity against the real graph")
    lines.append("")
    lines.append(
        "| corpus | claims 'already a Function' | matched file+name+line | "
        "claims 'already a Constant' | matched file+name | new nodes that "
        "collide with an existing Function |"
    )
    lines.append("|---|---:|---:|---:|---:|---:|")
    for r in results:
        f = r["fidelity"]
        lines.append(
            f"| {r['label']} | {f['claimed_existing_function']} | "
            f"{f['matched_exact_file_name_line']} | "
            f"{f['claimed_existing_constant']} | "
            f"{f['constant_matched_file_name']} | "
            f"{f['new_nodes_colliding_with_an_existing_function']} |"
        )
    lines.append("")
    lines.append("## Shapes D1 declines (informational)")
    lines.append("")
    keys = sorted({k for r in results for k in r["excluded_shapes"]})
    lines.append("| corpus | " + " | ".join(keys) + " |")
    lines.append("|---|" + "---:|" * len(keys))
    for r in results:
        lines.append(
            f"| {r['label']} | "
            + " | ".join(str(r["excluded_shapes"].get(k, 0)) for k in keys)
            + " |"
        )
    lines.append("")
    lines.append("## Verdict inputs")
    lines.append("")
    lines.append(
        "GO requires node growth within the ceiling AND captured-function "
        f"median >= {MEDIAN_LOC_FLOOR} LOC, on every corpus."
    )
    lines.append("")
    lines.append(
        "| corpus | growth full ok | growth w/o factory ok | growth narrowed "
        "ok | median LOC full | median ok | median LOC narrowed | median "
        "narrowed ok |"
    )
    lines.append("|---|---|---|---|---:|---|---:|---|")
    for r in results:
        v = r["verdict_inputs"]
        lines.append(
            f"| {r['label']} | {v['growth_full_within_ceiling']} | "
            f"{v['growth_no_factory_within_ceiling']} | "
            f"{v['growth_narrowed_within_ceiling']} | "
            f"{v['median_loc_full']} | {v['median_loc_meets_floor']} | "
            f"{v['median_loc_narrowed']} | "
            f"{v['median_loc_narrowed_meets_floor']} |"
        )
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    outdir = Path(sys.argv[1])
    labels = [spec.split("=", 1)[0] for spec in sys.argv[2:]]
    results = []
    for label in labels:
        r = score(outdir, label)
        det = (outdir / f"{label}.determinism.txt").read_text().strip()
        r["determinism"] = "OK" if det.endswith("two runs)") else det
        results.append(r)
    (outdir / "summary.md").write_text(summary_table(results))
    (outdir / "summary.json").write_text(json.dumps(results, indent=2))
    print(f"wrote {outdir / 'summary.md'}")
    print(f"wrote {outdir / 'summary.json'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
