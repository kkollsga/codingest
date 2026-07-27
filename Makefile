# codingest — CI-equivalent local gate
#
# `make gate` mirrors codingest CI (cargo fmt --check, clippy with -D warnings,
# workspace build, workspace test) and adds codingest-specific checks:
#   * a codingest_bench parity smoke against this workspace's own source, and
#   * the Python-wheel gate — build the `codingest` wheel via maturin and run
#     the tests/python acceptance suite (the .kgl-bytes handoff proof).
#
# Run `make gate` before pushing. Individual steps are also runnable
# (`make clippy`, `make bench-smoke`, …).
#
# EVERY STEP HERE MUST HAVE A VERDICT THIS REPOSITORY CONTROLS. The gate
# previously carried a determinism step that ran three builds of a *sibling*
# checkout (distillPDF) and asserted an exact edge count against it. That gate's
# verdict depended on a repository we do not own — including its uncommitted
# working state — so upstream churn turned it red (24,317 -> 24,173) with no
# codingest change, it never ran in CI, and it skipped silently when the sibling
# was absent. The property it actually protected (the builder is deterministic:
# the DEFINES-edge HashMap-iteration bug) now lives in
# `crates/codingest/tests/parity.rs::golden_parity`, which builds each committed
# in-repo corpus three times and asserts all three digests match each other and
# the frozen golden. That runs under step 4 below AND in CI on both OSes.
# `make determinism-soak REPO=…` keeps the large-repo reproducer available as a
# diagnostic; it is deliberately NOT part of the gate.

SHELL := /bin/bash

# Optional target for `make determinism-soak` (a diagnostic, not a gate).
SOAK_RUNS ?= 3

# The venv used for the Python-wheel gate steps (steps 6 & 7). It must already
# have `kglite` + `maturin` installed — the wheel's build-then-load handoff
# returns the installed kglite wheel's KnowledgeGraph. Override on the command
# line: `make pytest-py VENV=/path/to/.venv`.
#
# This defaults to a codingest-LOCAL venv on purpose. It previously defaulted to
# the sibling KGLite checkout's `.venv` (inherited from the initial extraction
# scaffold, where that venv happened to be the one with kglite + maturin already
# installed — never a deliberate requirement). That made `make gate` in this
# repo silently `maturin develop --release` into *another repo's* environment,
# clobbering whatever extension that repo's own conventions require to be
# installed, with nothing in either repo warning about it. Sharing an
# environment is now something you opt *into* explicitly via `VENV=...`, and
# the wheel step prints the absolute path it is about to write into either way.
VENV ?= $(CURDIR)/.venv

.PHONY: gate fmt fmt-check clippy build test bench-smoke wheel pytest-py \
	determinism-soak clean

## Full CI-equivalent gate — the single entry point. Runs every step below
## in order and stops at the first failure.
gate: fmt-check clippy build test bench-smoke wheel pytest-py
	@echo ""
	@echo "=================================================="
	@echo " gate: ALL STEPS PASSED"
	@echo "=================================================="

## 1. Formatting must be clean (matches KGLite `cargo fmt -- --check`).
fmt-check:
	@echo "== [1/7] cargo fmt --check =="
	cargo fmt --check

## Auto-format (convenience; not part of the gate).
fmt:
	cargo fmt

## 2. Clippy with warnings-as-errors (matches KGLite
##    `cargo clippy --all-targets -- -D warnings`, widened to --workspace).
clippy:
	@echo "== [2/7] cargo clippy --workspace --all-targets -- -D warnings =="
	cargo clippy --workspace --all-targets -- -D warnings

## 3. Build every crate + binary in the workspace.
build:
	@echo "== [3/7] cargo build --workspace =="
	cargo build --workspace

## 4. Test the workspace — includes tests/parity.rs: the golden oracle
##    (golden_parity + rev_self_consistency), which verifies each corpus's
##    frozen SHA-256 digest using only the codingest builder. (The live
##    two-builder sweep corpus_parity / rev_path_parity was removed when
##    KGLite deleted its in-tree builder on 2026-07-16 — cross-builder
##    comparison is no longer possible; the frozen goldens carry the
##    authority forward.) Runs under `cargo test --workspace` automatically.
##
##    THIS IS ALSO THE BUILDER-DETERMINISM GATE: golden_parity builds each
##    corpus three times and requires all three digests to agree with each
##    other (nondeterminism) as well as with the golden (behaviour change).
##    tests/corpus/dup_minified_assets is the reproducer for the DEFINES-edge
##    HashMap-iteration bug.
test:
	@echo "== [4/7] cargo test --workspace =="
	cargo test --workspace

## 5. Bench smoke: run codingest_bench against this workspace's own Rust
##    source (small, ~2s). It builds the tree TWICE with the codingest
##    builder and asserts query-result parity across the two independent
##    builds (a determinism check); any MISMATCH fails the gate. (Heavy
##    targets are out of scope for the smoke — point the binary at a large
##    repo by hand for real numbers.)
##
##    The step also asserts the harness resolved a `tracked-only` corpus. That
##    is the reproducibility contract: codingest_bench copies the target's
##    git-tracked files into a tempdir and builds THAT, because the builder
##    ingests untracked/git-ignored content (`dev-docs/`, `inbox/`) through the
##    docs pass and would otherwise measure whatever local state happens to
##    exist. Falling back to `working-tree` silently would restore exactly that
##    hazard, so it fails the gate instead.
bench-smoke:
	@echo "== [5/7] codingest_bench parity smoke (crates/codingest/src) =="
	cargo build --release -p codingest --bin codingest_bench
	@set -e; \
	out=$$(./target/release/codingest_bench crates/codingest/src); \
	echo "$$out" | grep '^corpus :' -A1; \
	echo "$$out" | tail -1; \
	if ! echo "$$out" | grep -q '^corpus : tracked-only'; then \
		echo "  FAIL: bench did not resolve a tracked-only corpus — the measured"; \
		echo "        input would include untracked/git-ignored files"; exit 1; \
	fi; \
	mismatch=$$(echo "$$out" | sed -n 's/.*, \([0-9]*\) MISMATCH$$/\1/p'); \
	if [ -z "$$mismatch" ]; then \
		echo "  FAIL: could not parse bench summary"; exit 1; \
	fi; \
	if [ "$$mismatch" != "0" ]; then \
		echo "  FAIL: $$mismatch query mismatch(es) between builders"; exit 1; \
	fi; \
	echo "  bench parity OK (0 mismatches, tracked-only corpus)"

## 6. Build the `codingest` Python wheel into $(VENV) via `maturin develop`.
##    This is the extension `import codingest` resolves to. Release build so
##    the pytest suite's build-then-load handoff runs at native speed. Skips
##    cleanly if the venv (or its maturin) is not present.
wheel:
	@echo "== [6/7] maturin develop the codingest wheel into $(VENV) =="
	@echo "  writing into: $(abspath $(VENV))"
	@if [ ! -x "$(VENV)/bin/maturin" ]; then \
		echo "  SKIP: $(VENV)/bin/maturin not present (pass VENV=... to run)"; \
		exit 0; \
	fi
	env -u CONDA_PREFIX VIRTUAL_ENV=$(VENV) $(VENV)/bin/maturin develop --release

## 7. Run the codingest-py acceptance suite (tests/python) — the .kgl-bytes
##    handoff proof + the resurrected build API. Requires step 6's wheel and a
##    kglite install in $(VENV). Skips cleanly if the venv is not present.
pytest-py: wheel
	@echo "== [7/7] pytest tests/python =="
	@if [ ! -x "$(VENV)/bin/python" ]; then \
		echo "  SKIP: $(VENV)/bin/python not present (pass VENV=... to run)"; \
		exit 0; \
	fi
	$(VENV)/bin/python -m pytest tests/python -q

## Determinism soak (DIAGNOSTIC — deliberately not part of `make gate`).
##
## The authoritative determinism gate is
## `crates/codingest/tests/parity.rs::golden_parity`: hermetic, in-repo corpora,
## run by `cargo test` and by CI. This target exists for the other question —
## "does the builder stay deterministic at real-repository scale?" — and takes
## any checkout you point it at:
##
##     make determinism-soak REPO=/path/to/some/checkout [SOAK_RUNS=5]
##
## It asserts only that `edges` is IDENTICAL across the runs. It deliberately
## does NOT compare against a pinned expected value: the target is a repository
## this project does not own, so any pinned number encodes a snapshot of
## somebody else's working tree and goes stale the moment they commit. If the
## soak target's own tree changes between runs, the edge count moves for that
## reason and the result is void — soak a quiescent checkout.
determinism-soak:
	@if [ -z "$(REPO)" ]; then \
		echo "usage: make determinism-soak REPO=/path/to/checkout [SOAK_RUNS=N]"; \
		exit 2; \
	fi
	@if [ ! -d "$(REPO)" ]; then echo "no such directory: $(REPO)"; exit 2; fi
	@echo "== determinism soak: edges stable over $(SOAK_RUNS) runs on $(REPO) =="
	cargo build --release -p codingest --bin codingest_stats
	@set -e; prev=""; \
	for i in $$(seq 1 $(SOAK_RUNS)); do \
		e=$$(./target/release/codingest_stats "$(REPO)" 2>/dev/null \
			| python3 -c "import sys,json;print(json.load(sys.stdin)['edges'])"); \
		echo "  run $$i: edges=$$e"; \
		if [ -n "$$prev" ] && [ "$$e" != "$$prev" ]; then \
			echo "  FAIL: edges changed between runs ($$prev != $$e) — nondeterminism"; \
			exit 1; \
		fi; \
		prev=$$e; \
	done; \
	echo "  edges=$$prev (stable across $(SOAK_RUNS) runs)"

## Remove build artifacts.
clean:
	cargo clean
