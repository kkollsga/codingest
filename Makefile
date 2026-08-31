# codingest — CI-equivalent local gate
#
# `make gate` mirrors codingest CI (cargo fmt --check, clippy with -D warnings,
# workspace build, workspace test) and adds codingest-specific checks:
#   * the release-gate script unit tests (tests/release/test_release_gates.py —
#     the same suite ci.yml's `release-gates` job runs),
#   * a codingest_bench parity smoke against this workspace's own source, and
#   * the Python-wheel gate — build the `codingest` wheel via maturin and run
#     the tests/python acceptance suite (the .kgl-bytes handoff proof).
#
# Run `make gate` before pushing. Individual steps are also runnable
# (`make clippy`, `make bench-smoke`, …).
#
# A SKIPPED STEP IS NEVER REPORTED AS A PASS. Three steps can legitimately not
# apply on a given machine (they need a venv, or pytest). Each one records
# itself in $(GATE_SKIPS) instead of exiting 0 in silence, and the summary line
# prints "N/M PASSED, K SKIPPED" naming them — `ALL … STEPS PASSED` is printed
# only when every step actually returned a verdict. A step that runs and fails
# still fails the gate with a non-zero exit, as before.
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
# the wheel step prints the absolute path it is about to write into either way.
VENV ?= $(CURDIR)/.venv

# A DEFAULT venv that isn't there is "not applicable" (fresh checkout) and
# SKIPs. A venv named explicitly — `make gate VENV=…` or `VENV=… make gate` —
# is a request to run those steps, so a missing/incomplete one is a FAILURE,
# not a skip. Nobody passes VENV= in order to be told it was ignored.
ifneq ($(filter command line environment,$(origin VENV)),)
VENV_REQUIRED := 1
else
VENV_REQUIRED :=
endif

# Where steps that could not run record themselves, so `gate` can tell
# "everything passed" apart from "everything that ran passed". `gate` truncates
# it up front (gate-reset) and reads it in the summary.
GATE_STEPS  := 9
GATE_SKIPS  := $(CURDIR)/target/.gate-skips

# $(call record-skip,<reason>) — reason must not contain a comma ($(call)
# splits on them).
record-skip = mkdir -p "$(dir $(GATE_SKIPS))" && printf '%s\n' "$(1)" >> "$(GATE_SKIPS)"

# The gate's steps share state through $(GATE_SKIPS) and are ordered on
# purpose; never run them concurrently.
.NOTPARALLEL:

.PHONY: gate gate-reset fmt fmt-check clippy build test release-gates \
	bench-smoke wheel pytest-py determinism-soak clean check-dev-docs

## Full CI-equivalent gate — the single entry point. Runs every step below
## in order and stops at the first failure.
gate: gate-reset check-dev-docs fmt-check clippy build test release-gates bench-smoke wheel pytest-py
	@echo ""
	@echo "=================================================="
	@if [ -s "$(GATE_SKIPS)" ]; then \
		n=$$(wc -l < "$(GATE_SKIPS)" | tr -d ' '); \
		echo " gate: $$(( $(GATE_STEPS) - n ))/$(GATE_STEPS) STEPS PASSED, $$n SKIPPED"; \
		echo " NOT A FULL GATE — these steps returned no verdict:"; \
		sed 's/^/   - /' "$(GATE_SKIPS)"; \
		echo "=================================================="; \
	else \
		echo " gate: ALL $(GATE_STEPS) STEPS PASSED"; \
		echo "=================================================="; \
	fi

# Clears the skip ledger so the summary describes THIS run. First prerequisite
# of `gate`; not meant to be run on its own.
gate-reset:
	@mkdir -p "$(dir $(GATE_SKIPS))"
	@: > "$(GATE_SKIPS)"

## 1. Formatting must be clean (matches KGLite `cargo fmt -- --check`).
fmt-check:
	@echo "== [2/9] cargo fmt --check =="
	cargo fmt --check

## Auto-format (convenience; not part of the gate).
fmt:
	cargo fmt

## 2. Clippy with warnings-as-errors (matches KGLite
##    `cargo clippy --all-targets -- -D warnings`, widened to --workspace).
clippy:
	@echo "== [3/9] cargo clippy --workspace --all-targets -- -D warnings =="
	cargo clippy --workspace --all-targets -- -D warnings

## 3. Build every crate + binary in the workspace.
build:
	@echo "== [4/9] cargo build --workspace --all-targets =="
	cargo build --workspace --all-targets

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
	@echo "== [5/9] cargo test --workspace =="
	cargo test --workspace

## 5. Release-gate script unit tests — the same suite ci.yml's `release-gates`
##    job runs. `.github/workflows/release.yml` fires only on a `v*` tag push,
##    so its publish-path logic can never be exercised (or seen to fail) by
##    branch CI; that logic therefore lives in `scripts/release_gates.sh` and
##    this suite drives every function through both its pass and its FAIL path.
##    Pure shell + a stubbed curl, so it is offline and takes <1s.
##
##    Interpreter: $(VENV)'s python if it has pytest, else the system python3.
##    If neither has pytest the step is SKIPPED (and says so in the summary) —
##    unless VENV was named explicitly, which makes it a failure.
release-gates:
	@echo "== [6/9] pytest tests/release (scripts/release_gates.sh) =="
	@set -e; \
	py=""; \
	for cand in "$(VENV)/bin/python" python3; do \
		if command -v "$$cand" >/dev/null 2>&1 \
			&& "$$cand" -c 'import pytest' >/dev/null 2>&1; then py="$$cand"; break; fi; \
	done; \
	if [ -n "$$py" ]; then \
		echo "  interpreter: $$py"; \
		"$$py" -m pytest tests/release -q; \
	elif [ -n "$(VENV_REQUIRED)" ]; then \
		echo "  FAIL: no pytest in $(VENV) or python3, and VENV was set explicitly"; \
		exit 1; \
	else \
		echo "  SKIP: no pytest in $(VENV) or python3 (pip install pytest to run)"; \
		$(call record-skip,[6/9] release-gate script unit tests (no pytest available)); \
	fi

## 6. Bench smoke: run codingest_bench against this workspace's own Rust
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
	@echo "== [7/9] codingest_bench parity smoke (crates/codingest/src) =="
	cargo build --release --workspace --bin codingest_bench
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

## 7. Build the `codingest` Python wheel into $(VENV) via `maturin develop`.
##    This is the extension `import codingest` resolves to. Release build so
##    the pytest suite's build-then-load handoff runs at native speed.
##
##    A missing DEFAULT venv is SKIPPED — recorded in $(GATE_SKIPS) so the gate
##    summary reports it instead of counting it as a pass. It used to just
##    `exit 0`, which is why `make gate` printed "ALL STEPS PASSED" on a machine
##    where 2 of its steps had never run. An explicitly-named VENV that is
##    missing is a FAILURE (see VENV_REQUIRED).
wheel:
	@echo "== [8/9] maturin develop the codingest wheel into $(VENV) =="
	@echo "  writing into: $(abspath $(VENV))"
	@if [ -x "$(VENV)/bin/maturin" ]; then \
		echo "  running: maturin develop --release"; \
		env -u CONDA_PREFIX VIRTUAL_ENV=$(VENV) $(VENV)/bin/maturin develop --release; \
	elif [ -n "$(VENV_REQUIRED)" ]; then \
		echo "  FAIL: $(VENV)/bin/maturin not present, and VENV was set explicitly"; \
		exit 1; \
	else \
		echo "  SKIP: $(VENV)/bin/maturin not present (pass VENV=... to run)"; \
		$(call record-skip,[8/9] maturin develop the codingest wheel (no venv)); \
	fi

## 8. Run the codingest-py acceptance suite (tests/python) — the .kgl-bytes
##    handoff proof + the resurrected build API. Requires step 7's wheel and a
##    kglite install in $(VENV). Same SKIP-vs-FAIL rule as step 7.
pytest-py: wheel
	@echo "== [9/9] pytest tests/python =="
	@if [ -x "$(VENV)/bin/python" ]; then \
		$(VENV)/bin/python -m pytest tests/python -q; \
	elif [ -n "$(VENV_REQUIRED)" ]; then \
		echo "  FAIL: $(VENV)/bin/python not present, and VENV was set explicitly"; \
		exit 1; \
	else \
		echo "  SKIP: $(VENV)/bin/python not present (pass VENV=... to run)"; \
		$(call record-skip,[9/9] pytest tests/python (no venv)); \
	fi

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
	cargo build --release --workspace --bin codingest_stats
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

## Mechanical bound on the gitignored dev-docs/ working folder — the one
## accumulation with no reviewer, no CI and no remote watching it grow. The
## gate itself NEVER deletes: which tier a file belongs in, and whether
## it is reproducible, is a judgement call, so the gate FAILS and hands the
## decision back. Stale purge-tier entries are reported as a warning (temp/bin
## churn is normal working state; failing on it would only teach people to
## bypass the gate). Tier lifecycles: dev-docs/README.md.
DEV_DOCS_MAX_MB := 256
.PHONY: check-dev-docs
check-dev-docs:
	@echo "== [1/9] dev-docs/ size bound =="
	@[ -d dev-docs ] || { echo "no dev-docs/ — nothing to bound"; exit 0; }; \
	mb=$$(du -sm dev-docs | cut -f1); \
	stale=$$( { find dev-docs/bench/out -mindepth 1 -maxdepth 1 -mtime +14; \
	            find dev-docs/temp      -mindepth 1 -maxdepth 1 -mtime +1;  \
	            find dev-docs/bin       -mindepth 1 -maxdepth 1 -mtime +7;  \
	          } 2>/dev/null ); \
	if [ "$${mb:-0}" -ge $(DEV_DOCS_MAX_MB) ]; then \
		echo "FAIL: dev-docs/ is $${mb} MB (>= $(DEV_DOCS_MAX_MB) MB)"; \
		echo "  largest tiers:"; \
		du -sm dev-docs/* dev-docs/bench/* 2>/dev/null | sort -rn | head -8 | sed 's/^/    /'; \
		[ -z "$$stale" ] || { echo "  past their documented lifetime:"; echo "$$stale" | sed 's/^/    /'; }; \
		echo "  -> reclaim, or move anything irreproducible to a durable tier (dev-docs/README.md)"; \
		exit 1; \
	fi; \
	echo "dev-docs/ is $${mb} MB (limit $(DEV_DOCS_MAX_MB) MB)"; \
	[ -z "$$stale" ] || { echo "WARN: past their documented lifetime (dev-docs/README.md):"; \
	                      echo "$$stale" | sed 's/^/    /'; }
