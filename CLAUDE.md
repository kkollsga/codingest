# codingest — project instructions

Standalone extraction of KGLite's `code_tree` component: parse polyglot
codebases into queryable [kglite](https://github.com/kkollsga/kglite) knowledge
graphs. The graph engine and MCP server are imported from KGLite as cargo
libraries; this repo owns only the code-tree component. See `README.md` for the
workspace map and `PARITY.md` / `BENCHMARKS.md` for the regression record.

**Authority:** `CLAUDE.md` is the authority this repo's conventions are
regenerated from, with `AGENTS.md` as its generated adapter; for the skills the
authority is **tracked `.agents/skills/`**, and `.claude/skills/` is a generated
adapter of it. Edit the authority and regenerate in the same action — never edit
an adapter. (This line is exempt from the `CLAUDE.md`↔`AGENTS.md` substitution —
it names the authority literally in every copy, per doctrine `R7`/`R14`.)

## Working style
- **Evidence over assertion.** For a bug, reproduce it and confirm the **root
  cause with evidence** before fixing. For a behaviour-preserving refactor,
  probe the *actual* graph output first — don't trust your mental model.
- **No bugs left behind.** A defect you notice mid-task gets fixed (in scope) or
  filed via `add-todo` (out of scope) — never silently stepped over. The builder
  now lives only here, so builder bugs are ours to fix; if it traces to the
  shared `kglite` engine (`kglite::api`, Cypher, storage), route it to KGLite via
  `notify` (KGLite is read-only here).
- **Offload, don't print.** Write long output (stats JSON, bench tables, graph
  dumps, big diffs) to `dev-docs/temp/` (>1-day purge) or `dev-docs/bench/out/`
  and **report the path**. Keep responses under ~400 tokens.
- **Tooling discipline.** Don't read a gate's status through a `tail`/`head`
  pipe that can hide a failure — confirm the command actually reported success.
  After any builder/parser change, run `cargo test` (not just a hand-picked
  subset), and always the parity test if transformed `code_tree` source moved.
  Three more shapes of the same trap, each of which failed in the *reassuring*
  direction (doctrine `R2`):
  - **`git add` with one bad pathspec stages NOTHING.** It is all-or-nothing:
    one typo'd or since-renamed path aborts the whole invocation, so the other
    files are not staged either — and the following `git commit` still succeeds,
    on a commit missing the change. Read back `git diff --cached --name-only`.
  - **`grep -c` exits 1 when the count is zero**, so `grep -c … && next` breaks
    the chain on exactly the empty result you needed to act on, and under
    `set -e` it kills the script. Capture it (`n=$(… | grep -c … || true)`) and
    test the number.
  - **A backgrounded command's output must be read from its artifact.** An
    echoed exit status, a "done" line, or the absence of visible errors is not
    the result — open the log the run wrote. This is how a failed background
    build reports green.
- **Testing cadence: targeted per landing, the full battery once at the end.**
  A landing's gate is the suites chosen to catch what *that* change could break
  — its touched surface plus that surface's direct consumers. This does not
  weaken "run `cargo test`, not just a hand-picked subset": the Rust battery is
  cheap and stays the per-phase gate. It applies to the *heavy* `make gate`
  steps (release-gate scripts, bench smoke, wheel + `tests/python`) — those run
  once before a branch's first push and once over the union at the program's
  end, never per phase. Per-phase heavy runs buy nothing a completion-time run
  does not, and their cost is what makes agents quietly stop running them.

## Code analysis — graph-first via the code-review MCP
For any structural question (where is X defined, what calls what, which
node/edge types exist, blast radius), use the **code-review MCP**: `set_root_dir`
to this repo, then `graph_overview` → `cypher_query`. Use `grep`/`read_source`
only for literal text search, never to rediscover structure the graph encodes.
Investigator agents in `phased-plan` run on this MCP.

## Review findings — what counts as one (doctrine `R15`)
- **A review finding names a concrete failure** — the input or state, and the
  wrong outcome it produces: a wrong result, a crash, data loss or corruption, a
  broken contract with a caller or a persisted file (a golden digest, a `.kgl`),
  a security hole, a *measured* performance regression, a gate that cannot fail,
  or a claim the code contradicts. **"No findings" is a valid review**, and a
  good one.
- **Design, structure, naming, "consider using X", "this won't scale" are not
  findings at review — they are mis-staged.** Their venue is **planning**, where
  "I would have designed this differently" is invited, argued and settled before
  the code exists. After plan approval, review measures the implementation
  against *that plan* and against correctness. A design opinion formed mid-diff
  is input to the *next* plan.
- **A finding that cannot state its failure case is removed, not downgraded.**
  Severity labels are the laundering mechanism: "Minor: consider extracting
  this" is a preference wearing a label.
- **One narrow exception:** citing a rule this project declared *before* the
  diff existed — the parity discipline, the six-manifest version rule, a
  documented ceiling, a checklist — naming both the rule and the violating line.
  That is enforcement, not taste.
- **A review tool's effort/confidence level is orthogonal.** A higher level buys
  more *speculative bugs*; it never buys permission to report preferences.
- A graph edge showing coupling is a **fact**, not a defect.

## Parity discipline (the hard goal — codingest-specific)
codingest was extracted as a **byte-minimal transform** of KGLite's in-tree
`kglite::code_tree`. KGLite **deleted that in-tree builder on 2026-07-16**, so
codingest is now the sole builder and there is no live upstream to cross-check
against. Fidelity to its last-known-good output is still a hard goal, now
enforced against a **frozen record** rather than a second builder. The
rationale + history live in `dev-docs/designs/parity-and-upstream-sync.md`.
- The gate is `cargo test -p codingest --test parity` — it must stay green.
  `golden_parity` rebuilds each corpus **three times** with the codingest
  builder and asserts every build's canonical digest matches every other build
  (determinism) and the frozen golden captured (2026-07-16) from the last
  in-sync in-tree authority (behaviour); `rev_self_consistency` covers the
  multi-rev path. `PARITY.md` is the committed record.
- A red `golden_parity` after a builder change is a **conscious decision**: if
  the graph change is intended, regenerate the goldens
  (`--ignored capture_goldens`) in the same commit and note why; never
  regenerate to silence an unexplained diff.

## Performance protocol
Perf claims come from a **release build only** (`--release`), `min` over
`median` for sub-ms work. **A number is meaningless without its corpus.**
`codingest_bench` builds the target's *git-tracked* files copied into a tempdir
and prints a `corpus_sha256`; quote that digest with any number you publish, and
never compare two numbers whose digests differ. `--include-untracked` measures
the directory as-is (the builder ingests gitignored content through the docs
pass) and is for one-off, explicitly-non-reproducible measurement only. The harness is `codingest_bench` (now two independent
codingest builds — per-query medians across the two, plus a cross-build
query-result parity check) + `codingest_stats` (build-time, node/edge counts).
Log ad-hoc runs to
`dev-docs/bench/results/results.csv`; heavy artifacts → `dev-docs/bench/out/`.
`BENCHMARKS.md` is the committed published record, refreshed at release time.

Four methodology rules, each learned by getting it wrong (doctrine `R11`) —
every error produced a plausible, low-concern reading:
- **"Trust min" does not hold for a once-per-event cost.** `min` is right for a
  repeatable inner loop; a cost paid once per build — first parse of a grammar,
  index construction, cold cache fill — has no steady state to find a floor of,
  and `min` over N builds reports the luckiest machine moment. Use the **mean of
  the first events** for those.
- **A heavy-tailed cell is judged by median/mean, not min.** If a cell's `min`
  sits far below its own median, that is not noise around a floor and the floor
  is not what users experience. Check min-vs-median per cell before quoting
  either.
- **A CONTROL cell that regresses means the instrument moved, not the code.**
  Always include a query/corpus the change cannot have touched; if it moves too,
  discard the whole capture rather than choosing which cells to believe. Machine
  load is *not* a precondition for a capture — the control is what carries the
  validity — but a number compared across sessions records the conditions it was
  taken under.
- **Measure the headline quantity two independent ways.** Agreement is cheap
  evidence the harness is sound; disagreement caught a real instrument bug that a
  single route reported as a clean result.

## Build / test / lint (there IS a Makefile; no maturin)
- **`make gate` runs the whole local net** — 9 steps (fmt, clippy, build, test,
  release-gate unit suite, bench-smoke, wheel, pytest). A step that cannot run
  (no `.venv`) is reported **SKIPPED**, never as a pass, and naming `VENV`
  explicitly turns a skip into a failure. Prefer it over hand-picked subsets.
- **Run the CI-only steps once before a long-lived branch's first push.** The
  fast loop (build + clippy + `cargo test` + parity) skips the release-gate
  script suite, the bench smoke and the wheel + `tests/python` acceptance run —
  exactly what CI runs — so a branch that has never run `make gate` accumulates
  several *independent* failures before its first red run (KGLite found **four**
  that way on a branch whose fast gate was green throughout). Once per branch,
  again at completion; never per phase.
- Build: `cargo build --workspace` (`--release` for perf).
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`.
- Test: `cargo test --workspace`; parity: `cargo test -p codingest --test parity`.
- Version: **SIX places, not one.** `[workspace.package] version` in the root
  `Cargo.toml` covers each crate's *own* `package.version` via
  `version.workspace = true`, but **not** the internal dependency requirements:
  `codingest-cli`, `codingest-mcp` and `codingest-py` each pin
  `codingest = { version = "X.Y.Z", path = … }`, and `codingest-py` also pins
  `codingest-cli` and `codingest-mcp` — `cargo publish` rejects a `path`-only
  dependency. "One line, no per-manifest bump" is **false**; it is the belief
  that broke KGLite's 0.15.0 release, and a patch bump only hides it. Only the
  `release` skill bumps, and it bumps all six together.

## Inbox hygiene
`inbox/` (gitignored) is the cross-project channel — operated only by the
`read-inbox` (receive) and `notify` (send) skills, never hand-edited. `unread/`
holds **only what still needs action**; an actioned note gets a `## Status
(codingest, <date>): …` footer and moves to `read/`. Route a note to another
project only if it carries an actionable task for *them*. The most common target
is upstream **KGLite**. Layout map: `inbox/README.md`.

## Skill mandates
- **Large feature / non-trivial refactor →** demand the **`phased-plan`** skill
  (investigate → gated plan → autonomous build/test/commit loop → parity+perf
  gate). Do **not** use generic plan mode for these.
- **Capturing work / findings →** **`add-todo`** (the authority on todo shape;
  lean `todos.md` backlink + detail in `plans/`).
- **Incoming mail →** **`read-inbox`**; **outgoing coordination →** **`notify`**.
- **Tidying the working folder →** **`dev-docs-cleanup`** (before a new
  phased-plan or at end of release).
- **Shipping →** **`release`** (the only place the version bumps).

## dev-docs working folder
Durable plans/designs/bench + a lean `todos.md` + time-boxed scratch live under
the gitignored **`dev-docs/`**. The canonical layout + lifecycle is
**`dev-docs/README.md`** — the skills point there; don't re-describe the folder
elsewhere. `dev-docs/` and `inbox/` are both gitignored local working state.
**Committed files never cite a `dev-docs/` path** — the folder is gitignored and
unbacked, so a citation from source, tests, docs, CI or scripts outlives the file
it names and silently becomes a dangling instruction. Durable rationale goes in
the commit message, in a self-contained comment at the code it constrains, or in
a committed doc (`PARITY.md`, `BENCHMARKS.md`, here).

## Agent worktrees
Agent git worktrees live in **`<repo>-worktrees/<name>`** — a sibling directory
*of the repo* (`Rust/codingest-worktrees/track-a`), never loose in the `Rust/`
parent, where they are indistinguishable at `ls` from the real project repos
(seven such strays, ~46 GB, sat in the estate root on 2026-08-10). The directory
exists only while worktrees are in progress; the `release` skill empties and
deletes it. Per worktree, in order: migrate outstanding actions into
`dev-docs/todos.md` (branch, state, what remains, how to resume) → if dirty, save
its `git diff` under `dev-docs/` **first** → `git worktree remove` +
`git worktree prune`. Removing a worktree never deletes its branch, so unmerged
work always survives. Two traps: a branch whose commits landed by **rebase**
reads as unmerged to `git merge-base --is-ancestor` (`git cherry -v main
<branch>` sees through it — `-` means already upstream), and a fresh worktree
does **not** inherit the repo's build-cache symlink.
