---
name: phased-plan
description: Run a large feature or refactor as a gated, phased project. Starts with an investigation phase (Explore agents on the code-review MCP map scale and impacted paths) — NOT standard plan mode — then builds a custom gated phased plan, works a local feature branch, and executes each phase autonomously (code → build → clippy → test → commit) until done. Keeps the parity + perf net green. Ships only via the release skill.
---

# Phased plan

For any large feature or non-trivial refactor of codingest. **Demand this
skill** when the user kicks off such work. Do **not** use standard plan mode
(`EnterPlanMode` / `ExitPlanMode`) — this skill builds its own gated phased
plan instead of the harness's generic plan.

## Working dir: `dev-docs/` (gitignored)
All plans, scratch, and intermediates live under **`dev-docs/`** — gitignored
local working state. **The canonical layout + lifecycle is `dev-docs/README.md`
— read it; it's the source of truth, this is just the phased-plan-relevant
subset:**
- This project's plan → **`dev-docs/plans/<slug>.md`** (durable).
- Design choices/trade-offs you weigh (parity/sync/algorithm) →
  **`dev-docs/designs/`** (durable).
- Open threads → a lean one-line backlink in **`dev-docs/todos.md`** (detail in
  the linked durable doc, never inline).
- **Offload large output to `dev-docs/temp/` and report the path** (>1-day
  purge) instead of printing it — stays under the response token gate.
- Parity/perf: harnesses → **`bench/scripts/`**, regression rows →
  **`bench/results/results.csv`**, heavy built graphs/dumps → **`bench/out/`**
  (>14-day purge; never write artifacts next to the script). The committed
  regression record is `PARITY.md` + `tests/parity.rs` and `BENCHMARKS.md`.

## Phase −1 — Start fresh (recommend cleanup first)
Before investigating, **recommend the user run the `dev-docs-cleanup` skill** so
we start from a tidy `dev-docs/` and a current `todos.md`. Relevant carried-over
todos can then be folded into this plan — **only with the user's go-ahead.** If
they decline, proceed without it.

## Phase 0 — Investigation (get a feel for scale before committing to a plan)
- **Do not enter plan mode.** Investigate first, plan second.
- **Read-only until approval.** The main loop makes **zero edits** during Phase
  0 and Phase 1 — no branch, no code, no file writes. All investigation goes
  through **read-only `Explore` agents**; nothing touches the working tree until
  the user approves the plan in Phase 1.
- Kick off **investigator agents** (`Explore`) equipped with the **code-review
  MCP** (graph_overview + cypher_query over the code graph, plus grep). Point it
  at this repo first (`set_root_dir` to the codingest root). Fan them out in
  parallel — one per subsystem / suspected blast-radius area. **Scale the count
  to blast radius:** 1–2 for a medium change, more only for a genuinely large
  one; don't over-spend on investigation. Have them report: structure of the
  affected area, impacted paths / callers, hidden couplings, existing test
  coverage, and a rough size estimate.
- **Is this a builder-behavior change?** codingest is the sole builder now that
  KGLite deleted its in-tree `kglite::code_tree` (2026-07-16), so builder logic
  is ours to change directly. But a change that alters graph output will move a
  corpus's golden digest — that's a conscious decision: regenerate the goldens
  (`--ignored capture_goldens`) with a recorded reason, in the same commit,
  never to silence an unexplained `golden_parity` diff. (Engine-level issues in
  `kglite::api` still go upstream via `notify` — KGLite is read-only here.)
- If this is a bug-driven refactor: reproduce and confirm the **root cause with
  evidence** before planning the fix (AGENTS.md "Working style").
- **For a behaviour-preserving refactor, probe current behaviour first.** Write
  a throwaway scratch script (or a `cargo test` scratch) that exercises the code
  paths you're about to move and capture their *actual* graph output — don't
  trust your mental model.
- **Confirm your intended safety net actually catches *this class* of change.**
  The parity test (`tests/parity.rs`) pins each corpus's graph to a frozen
  golden digest, but the corpus set is small — a regression on a shape no
  corpus exercises slips through. If your change touches an unrepresented
  language/edge kind, add a corpus (and its golden) or a targeted unit
  assertion. Decide the net in Phase 0, not after you've written the wrong one.
- Synthesize their findings into a scale read: small/medium/large, risk hot
  spots, what could invalidate a naive plan.

## Phase 1 — Build the gated phased plan
- Write the plan to **`dev-docs/plans/<slug>.md`** (the durable copy).
- Break the work into numbered phases. Each phase must be independently
  **buildable, testable, committable** (bisectable).
- For each phase spell out: the change, the tests that prove it (name the
  parity/unit test), the green gate.
- No phase touches the workspace `version` / CHANGELOG promotion — shipping is
  the `release` skill's job.
- Present the plan, then **invite revision: ask the user to revise or approve,
  and loop on their feedback until they approve.**
- **Hard stop — wait for an explicit go-ahead.** Do not create the branch or
  write any code until the user says proceed (e.g. "proceed", "go ahead",
  "approved", "ship it"). A simple proceed is enough. Until then, stay
  read-only.
- Once approved, **do not pause between phases.**

## Phase 2 — Branch (the tracking handle)
- **If this repo is a git repo** (`git rev-parse --show-toplevel` succeeds):
  create a feature branch `feat/<slug>` or `refactor/<slug>` — never work
  directly on `main`. If a remote + CI exist, push the branch and open a **draft
  PR against `main`** (CI then runs per push while nothing publishes) and mirror
  the plan into the PR description as a checklist.
- **If it is NOT a git repo yet** (the current state): offer to `git init` so
  the phase loop is bisectable. If the user declines, proceed without commits —
  the plan doc + a running progress note in `dev-docs/temp/` become the tracking
  handle. Say clearly that without git there is no per-phase bisection.

## Phase 3 — Execute each phase (the autonomous loop)
For every phase, in order:
1. Implement the phase's code + its tests.
2. **Local green gate before committing** (release build only for perf work):
   - `cargo build --workspace`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - the relevant test suite: `cargo test --workspace`, and **always**
     `cargo test -p codingest --test parity` if the change touches the builder
     or any transformed code_tree source. A targeted subset that skips the
     parity test will miss graph-equivalence regressions.
   Observe AGENTS.md "Tooling discipline": don't read a gate's status through a
   `tail`/`head` pipe; confirm the command actually reported success.
3. Update `CHANGELOG.md` `[Unreleased]` for user-visible changes (not the
   version block). Create it if missing.
4. **Commit** the phase (`feat(...)` / `refactor(...)` / `fix(...)`), one commit
   per phase (only if git is in play).
5. If a remote+CI PR exists, push → CI runs; tick the phase's checkbox.
6. **Retire any `todos.md` action this phase completed.** If the phase fully
   closes a backlog thread, do the same soft-delete tidy `dev-docs-cleanup`
   performs — at phase-commit time, not a separate pass:
   - **Fully done** → remove the backlink line from `todos.md` and move its
     supporting `plans/<doc>.md` to `dev-docs/bin/` (7-day grace).
   - **Partially done** → leave the doc; trim the entry to only what's left.
   - **Shared doc** → remove only the closed entry; move the doc to `bin/` only
     once no live backlink points at it.
   `dev-docs/` is gitignored, so this is local bookkeeping alongside the commit.
   Note each retirement in the report-out.
7. Continue into the next phase.

Stop mid-plan only for a genuine blocker (unfixable test, architectural surprise
invalidating a later phase). Surface it; don't push through.

**Bugs that surface mid-plan — fix them as they surface, don't step over them**
(AGENTS.md "no bugs left behind"):
- **In scope** (same file/subsystem): reproduce + confirm root cause, then fix
  as its **own bisectable phase** (`Phase Nb`) with its own test + commit
  (+ CHANGELOG if user-visible). Don't fold a behaviour change into a mechanical
  refactor commit.
- **Out of scope** (different subsystem): don't silently leave it. Reproduce,
  confirm, file it to `dev-docs/plans/` with a `todos.md` backlink (via
  `add-todo`), and add a cheap regression/parity assertion if one fits.
- **A break that traces to the shared engine** (the fix belongs in
  `kglite::api` / Cypher / storage, not the builder): KGLite is read-only here —
  capture it and `notify` KGLite's inbox (type `bug`/`request`); don't patch
  upstream locally.
Either way, record it in the **report-out** — a discovered bug never vanishes.

## Phase 4 — Parity + perf gate
Before declaring done:
- **Parity:** `cargo test -p codingest --test parity` green (`golden_parity` +
  `rev_self_consistency`). An intended digest change gets its goldens
  regenerated in the same commit with a recorded reason.
- **Perf:** run `codingest_bench` (release build, min over median) against a
  representative repo (two codingest builds; per-query medians + cross-build
  query parity) per AGENTS.md "Performance protocol". Log rows to
  `bench/results/results.csv`; heavy artifacts →
  `bench/out/`. Fix regressions now, not in a follow-up. Refresh `BENCHMARKS.md`
  only at release time.

## Report out (when the plan completes, before Ship)
Keep it under the 400-token rule; link the plan doc for detail:
- **Phases** done (one line each) + the commit shas (or "no git — plan doc only").
- **Bugs surfaced** during execution and each one's disposition: *fixed in Phase
  Nb* / *filed to backlog* / *routed upstream to KGLite*. Mandatory even if
  empty ("no bugs surfaced").
- **Parity + perf gate** result (parity discrepancy count; pre/post perf min +
  verdict: flat / regression / improved).
- **`todos.md` changes**: actions *retired* and carried-over / out-of-scope
  items *added*.
- **Plan deviations** (inserted phases, re-scopes) and why.

## Phase 5 — Ship (only on request)
When the user asks to ship, run the **`release`** skill. It goal-checks against
the plan, gates, bumps the one workspace `version` line, promotes the CHANGELOG,
refreshes `PARITY.md` / `BENCHMARKS.md` if they moved, and commits. This skill
never bumps the version.

## Notes
- Keep responses under 400 tokens; write long diffs/logs/stat dumps to a file
  under `dev-docs/temp/` and report the path.
- Touched a transformed code_tree source? Re-run the parity test and note in
  `designs/parity-and-upstream-sync.md` whether the transform stayed mechanical.
