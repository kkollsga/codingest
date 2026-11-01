# AGENTS.deviations.md — local improvements over the doctrine baseline

Ledger of behaviours in `AGENTS.md` and `.agents/skills/` that go beyond
`reference/AGENTS.baseline.md` (format and rules: the baseline's "How to
adopt it"). Adaptations (commands, paths, thresholds) are not listed. Read at
doctrine sync time; no harness loads it.

## Parity gate against a frozen golden

- **Where:** `AGENTS.md` → "Parity discipline"; `.agents/skills/phased-plan` → Phases 0, 3, 4; `.agents/skills/release`
- **Baseline says:** a red baseline after a deliberate change is regenerated in the same commit with a reason (R10); the baseline has no corpus-digest gate.
- **Local rule:** `cargo test --workspace --test parity` must stay green on every builder change; goldens are regenerated only via `--ignored capture_goldens` with a recorded reason, never to silence an unexplained diff.
- **Why it adds value here:** codingest is the sole builder, and its graph output is pinned to a golden captured 2026-07-16 from the last in-tree authority; without the gate a builder change could silently alter the graph every consumer queries.
- **Upstream:** not proposed (codingest-specific)
- **Last reviewed:** doctrine 0.2.0

## Gate-trap and benchmark methodology details beyond R1/R2/R11

- **Where:** `AGENTS.md` → "Testing discipline" (bare-`python` and null-masked `$?` recipe trap) and "Performance protocol" (corpus_sha256 pinning; mean-of-first-events for once-per-event costs; ≥2x control margin and deterministic-control void rule)
- **Baseline says:** R1/R2 name the traps generically; R11 asks for control cells and two agreeing runs.
- **Local rule:** the concrete recipe rules and the four extra benchmark methodology rules listed in those sections.
- **Why it adds value here:** each was learned by getting it wrong (KGLite 2026-08-09/10; the 0.1.6 control corollary) and `codingest_bench` and `scripts/bench_anchor.py` implement them.
- **Upstream:** candidate (the corpus-digest, first-event-mean and control-margin rules would help any repo with a longitudinal benchmark)
- **Last reviewed:** doctrine 0.2.0
