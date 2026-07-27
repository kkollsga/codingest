# codingest — CI-equivalent local gate
#
# `make gate` mirrors codingest CI (cargo fmt --check, clippy with -D warnings,
# workspace build, workspace test) and adds codingest-specific checks:
#   * a determinism reproducer for the DEFINES-edge nondeterminism bug,
#   * a codingest_bench parity smoke against this workspace's own source, and
#   * the Python-wheel gate — build the `codingest` wheel via maturin and run
#     the tests/python acceptance suite (the .kgl-bytes handoff proof).
#
# Run `make gate` before pushing. Individual steps are also runnable
# (`make clippy`, `make determinism`, …).

SHELL := /bin/bash

# The determinism reproducer target. Override on the command line if the
# checkout lives elsewhere: `make determinism DISTILLPDF=/path/to/repo`.
DISTILLPDF ?= /Volumes/EksternalHome/Koding/Rust/distillPDF
# Post-fix canonical edge count on distillPDF (BTreeMap + within-pair
# consolidation of DEFINES edges). Stability across runs is the real
# invariant; this pins the known-good value too.
EXPECTED_EDGES ?= 24317

# The venv used for the Python-wheel gate steps (steps 7 & 8). It must already
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
# step 7 prints the absolute path it is about to write into either way.
VENV ?= $(CURDIR)/.venv

.PHONY: gate fmt fmt-check clippy build test determinism bench-smoke wheel pytest-py clean

## Full CI-equivalent gate — the single entry point. Runs every step below
## in order and stops at the first failure.
gate: fmt-check clippy build test determinism bench-smoke wheel pytest-py
	@echo ""
	@echo "=================================================="
	@echo " gate: ALL STEPS PASSED"
	@echo "=================================================="

## 1. Formatting must be clean (matches KGLite `cargo fmt -- --check`).
fmt-check:
	@echo "== [1/8] cargo fmt --check =="
	cargo fmt --check

## Auto-format (convenience; not part of the gate).
fmt:
	cargo fmt

## 2. Clippy with warnings-as-errors (matches KGLite
##    `cargo clippy --all-targets -- -D warnings`, widened to --workspace).
clippy:
	@echo "== [2/8] cargo clippy --workspace --all-targets -- -D warnings =="
	cargo clippy --workspace --all-targets -- -D warnings

## 3. Build every crate + binary in the workspace.
build:
	@echo "== [3/8] cargo build --workspace =="
	cargo build --workspace

## 4. Test the workspace — includes tests/parity.rs: the golden oracle
##    (golden_parity + rev_self_consistency), which verifies each corpus's
##    frozen SHA-256 digest using only the codingest builder. (The live
##    two-builder sweep corpus_parity / rev_path_parity was removed when
##    KGLite deleted its in-tree builder on 2026-07-16 — cross-builder
##    comparison is no longer possible; the frozen goldens carry the
##    authority forward.) Runs under `cargo test --workspace` automatically.
test:
	@echo "== [4/8] cargo test --workspace =="
	cargo test --workspace

## 5. Determinism: codingest_stats must report an identical `edges` count on
##    three consecutive builds of the same tree. This is the original
##    nondeterminism bug's reproducer (randomized HashMap iteration over
##    DEFINES pairs flapped the edge total run-to-run). Skips cleanly if the
##    reproducer repo is not present on this machine.
determinism:
	@echo "== [5/8] determinism: edges stable over 3 runs on $(DISTILLPDF) =="
	@if [ ! -d "$(DISTILLPDF)" ]; then \
		echo "  SKIP: $(DISTILLPDF) not present (pass DISTILLPDF=... to run)"; \
		exit 0; \
	fi
	cargo build --release -p codingest --bin codingest_stats
	@set -e; prev=""; \
	for i in 1 2 3; do \
		e=$$(./target/release/codingest_stats "$(DISTILLPDF)" \
			| python3 -c "import sys,json;print(json.load(sys.stdin)['edges'])"); \
		echo "  run $$i: edges=$$e"; \
		if [ -n "$$prev" ] && [ "$$e" != "$$prev" ]; then \
			echo "  FAIL: edges changed between runs ($$prev != $$e) — nondeterminism regression"; \
			exit 1; \
		fi; \
		prev=$$e; \
	done; \
	echo "  edges=$$prev (stable across 3 runs)"; \
	if [ "$$prev" != "$(EXPECTED_EDGES)" ]; then \
		echo "  FAIL: edges=$$prev, expected $(EXPECTED_EDGES) — value drifted from canonical baseline"; \
		exit 1; \
	fi; \
	echo "  edges match canonical baseline ($(EXPECTED_EDGES))"

## 6. Bench smoke: run codingest_bench against this workspace's own Rust
##    source (small, ~2s). It builds the tree TWICE with the codingest
##    builder and asserts query-result parity across the two independent
##    builds (a determinism check); any MISMATCH fails the gate. (Heavy
##    targets are out of scope for the smoke — point the binary at a large
##    repo by hand for real numbers.)
bench-smoke:
	@echo "== [6/8] codingest_bench parity smoke (crates/codingest/src) =="
	cargo build --release -p codingest --bin codingest_bench
	@set -e; \
	out=$$(./target/release/codingest_bench crates/codingest/src); \
	echo "$$out" | tail -1; \
	mismatch=$$(echo "$$out" | sed -n 's/.*, \([0-9]*\) MISMATCH$$/\1/p'); \
	if [ -z "$$mismatch" ]; then \
		echo "  FAIL: could not parse bench summary"; exit 1; \
	fi; \
	if [ "$$mismatch" != "0" ]; then \
		echo "  FAIL: $$mismatch query mismatch(es) between builders"; exit 1; \
	fi; \
	echo "  bench parity OK (0 mismatches)"

## 7. Build the `codingest` Python wheel into $(VENV) via `maturin develop`.
##    This is the extension `import codingest` resolves to. Release build so
##    the pytest suite's build-then-load handoff runs at native speed. Skips
##    cleanly if the venv (or its maturin) is not present.
wheel:
	@echo "== [7/8] maturin develop the codingest wheel into $(VENV) =="
	@echo "  writing into: $(abspath $(VENV))"
	@if [ ! -x "$(VENV)/bin/maturin" ]; then \
		echo "  SKIP: $(VENV)/bin/maturin not present (pass VENV=... to run)"; \
		exit 0; \
	fi
	env -u CONDA_PREFIX VIRTUAL_ENV=$(VENV) $(VENV)/bin/maturin develop --release

## 8. Run the codingest-py acceptance suite (tests/python) — the .kgl-bytes
##    handoff proof + the resurrected build API. Requires step 7's wheel and a
##    kglite install in $(VENV). Skips cleanly if the venv is not present.
pytest-py: wheel
	@echo "== [8/8] pytest tests/python =="
	@if [ ! -x "$(VENV)/bin/python" ]; then \
		echo "  SKIP: $(VENV)/bin/python not present (pass VENV=... to run)"; \
		exit 0; \
	fi
	$(VENV)/bin/python -m pytest tests/python -q

## Remove build artifacts.
clean:
	cargo clean
