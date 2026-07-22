# codingest — project instructions

Standalone extraction of KGLite's `code_tree` component: parse polyglot
codebases into queryable [kglite](https://github.com/kkollsga/kglite) knowledge
graphs. The graph engine and MCP server are imported from KGLite as cargo
libraries; this repo owns only the code-tree component. See `README.md` for the
workspace map and `PARITY.md` / `BENCHMARKS.md` for the regression record.

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

## Code analysis — graph-first via the code-review MCP
For any structural question (where is X defined, what calls what, which
node/edge types exist, blast radius), use the **code-review MCP**: `set_root_dir`
to this repo, then `graph_overview` → `cypher_query`. Use `grep`/`read_source`
only for literal text search, never to rediscover structure the graph encodes.
Investigator agents in `phased-plan` run on this MCP.

## Parity discipline (the hard goal — codingest-specific)
codingest was extracted as a **byte-minimal transform** of KGLite's in-tree
`kglite::code_tree`. KGLite **deleted that in-tree builder on 2026-07-16**, so
codingest is now the sole builder and there is no live upstream to cross-check
against. Fidelity to its last-known-good output is still a hard goal, now
enforced against a **frozen record** rather than a second builder. The
rationale + history live in `dev-docs/designs/parity-and-upstream-sync.md`.
- The gate is `cargo test -p codingest --test parity` — it must stay green.
  `golden_parity` rebuilds each corpus with the codingest builder and asserts
  its canonical digest matches the frozen golden captured (2026-07-16) from the
  last in-sync in-tree authority; `rev_self_consistency` covers the multi-rev
  path. `PARITY.md` is the committed record.
- A red `golden_parity` after a builder change is a **conscious decision**: if
  the graph change is intended, regenerate the goldens
  (`--ignored capture_goldens`) in the same commit and note why; never
  regenerate to silence an unexplained diff.

## Performance protocol
Perf claims come from a **release build only** (`--release`), `min` over
`median` for sub-ms work. The harness is `codingest_bench` (now two independent
codingest builds — per-query medians across the two, plus a cross-build
query-result parity check) + `codingest_stats` (build-time, node/edge counts).
Log ad-hoc runs to
`dev-docs/bench/results/results.csv`; heavy artifacts → `dev-docs/bench/out/`.
`BENCHMARKS.md` is the committed published record, refreshed at release time.

## Build / test / lint (no Makefile, no maturin)
- Build: `cargo build --workspace` (`--release` for perf).
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`.
- Test: `cargo test --workspace`; parity: `cargo test -p codingest --test parity`.
- Version: **one line** — `[workspace.package] version` in the root `Cargo.toml`
  (all three crates inherit via `version.workspace = true`). Only the `release`
  skill bumps it.

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
