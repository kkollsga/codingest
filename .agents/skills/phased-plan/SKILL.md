---
name: phased-plan
description: Run a large feature or refactor as a gated, phased project. Starts with an investigation phase (Explore agents on the code-review MCP map scale and impacted paths) — NOT standard plan mode — then builds a custom gated phased plan, creates a branch + draft PR for CI tracking, and executes each phase autonomously (code → build → clippy → test → commit) with checkpoint pushes until done. Keeps the parity + perf net green. Ships only via the release skill.
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

## Doctrine sync — first action of the run, before Phase −1
The estate's rules live in the sibling `doctrine` repo and are versioned. **Pull
them forward before planning anything**, so a plan is never built on doctrine
this repo has already been told is superseded.

1. Read **`../doctrine/VERSION`** (a date serial, e.g. `2026.08.10`) and
   **`dev-docs/.doctrine-synced`**. If the marker is absent, create it with the
   current version and note in the report-out that this was a first sync (there
   is nothing to replay — the marker's job starts now).
2. **Versions equal → done.** That is the normal case and it costs **one file
   read**; it is never worth skipping to save time, and "we're probably current"
   is not the check.
3. **Doctrine ahead → read `../doctrine/CHANGELOG.md` forward from the marker**
   and act on every entry newer than it. Each item carries exactly one action
   class:
   - **`[skills-update]`** — merge the change into this repo's **declared
     authority** (per the Authority line at the top of AGENTS.md: the conventions
     file itself, and the tracked `.agents/skills/` tree) and regenerate the
     adapters from it in the same action. Never hand-port into an adapter — that
     is what doctrine `R7` measures.
   - **`[local-sweep]`** — run the check command the entry states. If it comes
     back clean, say so and move on. **If it fails, the sweep becomes Phase 0
     work of *this* plan** — scoped, listed, and visible in the plan doc — never
     a silent side-task folded into an unrelated phase.
   - **`[info]`** — nothing to do.
4. **Write the new version to `dev-docs/.doctrine-synced` only after those
   actions completed.** A marker written first permanently hides the entry it
   skipped: the next run compares against it and sees nothing. If an entry could
   not be actioned, the marker advances **only** once that item is in the plan —
   the plan is the record, not the marker.

Doctrine `R14`: read the oracle before the local copy, and cite the oracle
version the adaptation read. Every divergence between `../doctrine` and this
repo's installed copy is named as one of exactly two things — a **local
improvement** (upstream it) or **staleness** (fix it from the oracle).

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
  The parity test (`crates/codingest/tests/parity.rs`) pins each corpus's graph to a frozen
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
- **Challenge the plan once before presenting it** (doctrine 0.1.5). List the
  factual claims it rests on (paths, call sites, API behaviour, cost
  attributions) and verify each against the code, recording the evidence —
  Phase 0's attributions are hypotheses until re-checked *as written into the
  plan*, where a stale or misquoted one reads as settled. Then run one
  pre-mortem: "this plan shipped and failed — why?", 2–3 concrete scenarios. A
  scenario that names a real failure changes a phase, adds a test, or becomes a
  stop rule; one that cannot is a design preference — argue it in the approval
  loop, unlabelled. **No severity tiers** (R15 names them as the laundering
  mechanism, and the same agents work review an hour later).
- Present the plan, then **invite revision: ask the user to revise or approve,
  and loop on their feedback until they approve.**
- **This is the stage where "I would have designed this differently" belongs —
  say so explicitly when presenting.** Structure, naming, factoring, "consider
  using X", scale worries: raise them now or hold them. Review will **refuse**
  them later (AGENTS.md "Review findings", doctrine `R15`), because once the plan
  is approved review measures the implementation against *that plan* and against
  correctness — not against a design the reviewer would have preferred. The
  invitation and the refusal are a pair: written in only one place, design
  critique lands wherever the reviewer is standing, which is review, because
  that is where the code is.
- **Hard stop — wait for an explicit go-ahead.** Do not create the branch or
  write any code until the user says proceed (e.g. "proceed", "go ahead",
  "approved", "ship it"). A simple proceed is enough. Until then, stay
  read-only.
- Once approved, **do not pause between phases.**

## Phase 2 — Branch + draft PR (the CI tracking handle)
- Create a feature branch: `feat/<slug>` or `refactor/<slug>` (never work the
  project directly on `main`).
- **Exactly one branch + one draft PR per plan. Phases are commits, never
  sub-branches** — no per-phase or per-workstream branches merged back later
  (KGLite's 0.14.2 cycle left 8 stale branches that way). When the plan ships,
  the release skill deletes the branch local + remote.
- Push the branch and **open a draft PR against `main`**. This is what makes CI
  run on the branch: `ci.yml` triggers on `pull_request: [main]`, so every push
  to the branch runs the full suite — while **nothing publishes** (publishing
  only triggers on a `v*` tag push, at release time).
- Put the phased plan into the **PR description as a checklist** (one box per
  phase). The PR tab then shows plan + progress + CI status in one place.
- **Run the CI-only steps once locally before the branch's first push.** The
  Phase 3 loop runs `build` + `clippy` + `cargo test` + parity; `make gate`
  additionally runs the release-gate script suite, the bench parity smoke, and
  the **wheel + `tests/python`** acceptance run — exactly the steps CI runs and
  the phase loop skips, so they stay ungated until CI sees them, and a branch
  that has never run them accumulates several *independent* failures before its
  first red run (KGLite's 2026-08-09 program found **four** that way, on a
  branch whose fast gate was green throughout). Run `make gate` once here, then
  rely on the phase loop.

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
   **Choose the per-phase gate by what this change could BREAK, not by what it
   touches.** The suite to run is the touched surface *plus its direct
   consumers* — the phase that edits a parser also runs whatever asserts on the
   graph that parser feeds. The Rust battery here is cheap enough that
   `cargo test --workspace` + parity is that gate for almost every phase, and
   AGENTS.md's "not just a hand-picked subset" rule still binds. The heavy
   `make gate` steps are not per-phase work: they run once before the first push
   (Phase 2) and once over the union at plan completion (Phase 4). Per-phase
   heavy runs buy nothing a completion-time run does not, and their cost is
   what makes agents quietly stop running them at all.
   Observe AGENTS.md "Tooling discipline": don't read a gate's status through a
   `tail`/`head` pipe; confirm the command actually reported success.
   **A NEW GATE IS NOT TRUSTED UNTIL YOU HAVE SEEN IT FAIL.** If the phase adds
   or changes a check — a test, a CI step, an assertion in a script — break the
   thing it guards, confirm it goes red, then restore. Reading a gate cannot tell
   you whether it works: every vacuous gate found on 2026-07-28 looked correct,
   and the only thing that separated the live ones from the dead ones was
   mutation. Four ways a gate is born dead:
   - **Substring subsumption.** `assert "cmd" in block` also matches
     `cmd --self-test`, so deleting the real invocation stays green. Compare
     whole stripped lines, not `in`.
   - **Comment subsumption.** The words you assert on usually also appear in the
     comment explaining them. Strip comment lines before matching.
   - **`exit` inside `$( )`.** A shell guard that exits inside command
     substitution kills only the subshell; the caller reads the empty output as 0
     and passes. Return a sentinel the caller checks.
   - **A guard skippable by the condition it guards.** A check placed inside a
     module or step that is skipped wholesale when its subject is absent checks
     nothing — a binary-resolution test inside the module that skips when no
     binary exists can never fail. Put the guard where the skip cannot reach it.
   **Verify the probe, not just the result.** A mutation that silently edited the
   wrong text makes a working gate look broken, and an unchanged file makes a dead
   gate look alive. Confirm the subject actually changed before believing either
   verdict.
   **A pipeline reports its LAST stage's status**, so `cmd … | tail` says 0 for a
   `cmd` that exited non-zero. Never declare a phase green off piped output.
   **Three more shapes of the same trap, all found on KGLite's 2026-08-09/10
   program — each lied in the reassuring direction:**
   - **`git add` with one bad pathspec stages NOTHING.** It is all-or-nothing:
     a single typo'd or since-renamed path aborts the whole invocation, so the
     other five files you named are not staged either. This skill's
     commit step names files by path deliberately — so read back
     `git status --porcelain` (or `git diff --cached --name-only`) and confirm
     the staged set is what you intended. A commit that "succeeded" can be empty
     of the change you meant to ship.
   - **`grep -c` exits 1 when the count is zero**, so `grep -c … && next` breaks
     the chain on the one result you most need to act on, and under `set -e` it
     kills the script. A zero count is a legitimate answer, not an error:
     capture it (`n=$(… | grep -c … || true)`) and test the number.
   - **A backgrounded command's output must be read from its artifact.** An
     echoed exit status, a "done" line, or the absence of visible errors is not
     the result — open the log/output file the run actually wrote. Inferring
     success from the wrapper is how a failed background build gets reported as
     a passing one.
3. Update `CHANGELOG.md` `[Unreleased]` for user-visible changes (not the
   version block). Create it if missing.
4. **Commit** the phase (`feat(...)` / `refactor(...)` / `fix(...)`), one commit
   per phase.
5. **Push at checkpoints, not per commit.** Every branch push starts a full CI
   run; the habit is batching: push every 2–3 quick phases, at a risky
   milestone worth CI confirmation, or before stepping away — and always once
   at plan completion. Tick completed phases' checkboxes in the PR description
   when you push.
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
7. Continue into the next phase. If a checkpoint push's CI comes back red, fold
   the fix into the loop before the plan completes — don't leave the PR red.

Stop mid-plan only for a genuine blocker (unfixable test, architectural surprise
invalidating a later phase). Surface it; don't push through.

**The run ends in a finished plan or a named blocker — not a status report**
(doctrine [[R12]]). The approval in Phase 1 authorizes the *whole* build loop,
not permission to begin it.
- **Completion condition:** every phase committed, Phase 4's parity (+ perf, if
  it applied) green, the checkpoint push landed, and the report-out written.
- **Non-endings:** "CI is running", "the phase is committed", "the agent is
  working", "next I will…". Each is a natural pause point that reads, from the
  inside, like a reasonable place to check in — that is the whole difficulty.
  The failure is not choosing to stop; it is not noticing that continuing is an
  option.
- **Waiting is not a checkpoint.** A checkpoint push's CI, or a subagent still
  running, is something to poll or background — not a reason to hand control
  back. A red checkpoint CI is a task: diagnose, fix, fold it in, re-poll.
- **Do not pause between phases** (Phase 1 already says this). This is the same
  rule, stated where the loop can actually stall.

**Bugs that surface mid-plan — fix them as they surface, don't step over them**
(AGENTS.md "no bugs left behind"; doctrine 0.1.2 — fixing is the default):
- **Classify first.** A *bug* — wrong result, crash, data loss, broken
  contract, measured regression, dead gate, contradicted claim — is **fixed,
  never filed**. Only a *missing capability* (a route never built, an
  optimization never attempted) may go to the backlog/parking lot.
- **"Out of scope" changes the commit boundary, not the decision to fix**: an
  in-scope bug folds into the phase's work as its **own bisectable phase**
  (`Phase Nb`) with its own test + commit (+ CHANGELOG if user-visible); an
  out-of-scope bug still gets fixed, just as a separately-scoped phase. Don't
  fold a behaviour change into a mechanical refactor commit.
- **Filing a bug is allowed only when fixing-now is genuinely blocked** (e.g.
  it needs its own parity decision, a corpus that doesn't exist, or an
  upstream change), and the report-out must then say **why** — "out of scope"
  is a location, not a reason. Reproduce + confirm first, file via `add-todo`
  with the blocking reason in the plan doc, and add a cheap regression/parity
  assertion pinning the current behaviour if one fits.
- **A suspected perf bug is measured in-plan** to confirm it before its fix
  counts — never deferred unmeasured to a someday-backlog.
- **A break that traces to the shared engine** (the fix belongs in
  `kglite::api` / Cypher / storage, not the builder): KGLite is read-only here —
  capture it and `notify` KGLite's inbox (type `bug`/`request`); don't patch
  upstream locally.
Either way, record it in the **report-out** — a discovered bug never vanishes.

## Phase 4 — Full battery + parity + perf gate
Before declaring done:
- **Full battery — once, here, over the union of everything the plan touched:**
  `make gate` (all 9 steps, including the ones the phase loop skips). The phase
  loop's targeted gates catch per-landing breakage; this run is what catches the
  interaction between phases. Per AGENTS.md, a **SKIPPED** step is not a pass —
  either name `VENV=` explicitly so a skip becomes a failure, or state in the
  report-out which steps did not run.
- **Parity — always, unconditionally:** `cargo test -p codingest --test parity`
  green (`golden_parity` + `rev_self_consistency`). An intended digest change
  gets its goldens regenerated in the same commit with a recorded reason.
- **Perf — only if the plan touched perf-sensitive paths** (parser hot loops,
  the builder walk/partition/resolve stages, anything on the per-file path):
  run `codingest_bench` (release build, min over median) against a
  representative repo (two codingest builds; per-query medians + cross-build
  query parity) per AGENTS.md "Performance protocol". Log rows to
  `bench/results/results.csv`; heavy artifacts → `bench/out/`. Fix regressions
  now, not in a follow-up. For plans that never touched perf-sensitive code,
  skip the bench with a note — the release-time `BENCHMARKS.md` refresh covers
  it. Refresh `BENCHMARKS.md` only at release time either way.
  **Four methodology rules, each learned by getting it wrong (KGLite,
  2026-08-09/10) — every error produced a plausible, low-concern reading:**
  - **"Trust min" does not hold for a once-per-event cost.** `min` is the right
    statistic for a repeatable inner loop; a cost paid once per build — first
    parse of a grammar, index construction, cold cache fill — has no
    steady-state to find a floor of, and `min` over N builds just reports the
    luckiest machine moment. Use the **mean of the first events** for those.
  - **A heavy-tailed cell is judged by median/mean, not min.** If a
    measurement's `min` sits far below its own median, the distribution is not
    "noise around a floor" and the floor is not the thing users experience.
    Check min-vs-median per cell before quoting either.
  - **A CONTROL cell that regresses is your instrument, not the code.** Always
    include a query/corpus the change cannot possibly have touched. If that one
    moves too, you measured the machine (thermals, a background build, a
    different corpus digest) — throw the whole capture away rather than
    reasoning about which cells to believe.
  - **Measure the headline quantity by two independent routes.** Agreement is
    cheap evidence the harness is sound; disagreement caught a real instrument
    bug that a single route reported as a clean result. Pairs with the existing
    rule that a number is meaningless without its `corpus_sha256`.

## Report out (when the plan completes, before Ship)
Keep it under the 400-token rule; link the plan doc for detail:
- **Phases** done (one line each) + the PR link / final commit shas.
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
the plan, gates, bumps the version across all six manifest sites (the workspace
table plus the five internal path-dependency pins), promotes the CHANGELOG,
refreshes `PARITY.md` / `BENCHMARKS.md` if they moved, and commits. This skill
never bumps the version.

## Notes
- Keep responses under 400 tokens; write long diffs/logs/stat dumps to a file
  under `dev-docs/temp/` and report the path.
- Branch pushes during the loop are routine (CI only, no publish). Publishing
  only triggers on a `v*` tag push — the `release` skill's approval-gated step.
- Touched a transformed code_tree source? Re-run the parity test and note in
  `designs/parity-and-upstream-sync.md` whether the transform stayed mechanical.
