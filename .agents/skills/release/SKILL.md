---
name: release
description: Cut and publish a codingest release — goal-check, gate (build/clippy/test incl. parity), bump the workspace version, refresh regression records, rebuild binaries, promote CHANGELOG, commit, push, tag, publish crates and Python artifacts, verify CI/registries/GitHub Release, then tidy dev-docs and released branches.
---

# Release

## Preconditions
- **Git state.** If this repo is not yet a git repo, a "release" is just the
  gated version bump + CHANGELOG promotion committed locally once `git init` is
  done — offer to init if the user wants a tagged history; otherwise the bump +
  CHANGELOG edit stand as the release marker. Say which path you're on.
- **No double-stage.** If git is in play, check no release is already staged:
  `git log --oneline -20 | grep -E "release\("`. If a `release(x.y.z)` sits
  unpushed, **keep that version** — fold work into the same `[x.y.z]` block (one
  version bump per release). If that version is already tagged, published, and
  there is no new releasable work, stop as an idempotent no-op — never create an
  empty patch release.
- **Surgical staging.** If there's unrelated uncommitted work in the tree, don't
  sweep it in: **stage every release file explicitly by path** (`git add <file>
  …`, never `git add -A`/`.`) and verify with `git status --porcelain` that only
  release files are staged.

## Steps
1. **Goal check — did we achieve what we set out to do?** If this release ships
   a `phased-plan` project, read its plan (`dev-docs/plans/<slug>.md`) and
   confirm every planned phase actually shipped. List any phase **dropped,
   deferred, or only partially done** and surface the gaps before bumping — each
   gap is a conscious choice: finish it now, or carry it to `dev-docs/todos.md`.
   Don't let it vanish silently.
2. **Gate.** All green before continuing:
   - `cargo build --workspace --release`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo test --workspace` — **and confirm `cargo test -p codingest --test
     parity` is in the run and green.** The golden-digest parity
     (`golden_parity` + `rev_self_consistency`, against digests frozen from the
     now-deleted in-tree `kglite::code_tree`) is the hard release gate; a subset
     that skips it can ship a graph-equivalence regression.
3. **Bump version — patch by default** (`x.y.Z` → `x.y.Z+1`). If the changes
   warrant a **minor/major** bump (new feature, breaking change, scope
   expansion), STOP and ask one quick clarification before starting; otherwise
   proceed with the patch bump. **One line:** `[workspace.package] version` in
   the root `Cargo.toml` — every crate inherits via `version.workspace = true`,
   so there is no per-manifest bump. Run `cargo metadata --no-deps --format-version 1
   | grep -o '"version":"[^"]*"' | sort -u` (or `cargo metadata --no-deps`) to
   confirm all three members resolved to the new version.
4. **Refresh the regression record (codingest's "captured constants").** These
   are the committed files that gate the project — refresh them off the fresh
   `--release` build so they reflect the shipped state:
   - **Parity** (`PARITY.md`): re-run `cargo test -p codingest --test parity`;
     if the verdict or corpus set changed, update `PARITY.md` (date + result).
     A green `golden_parity` is the invariant — if a corpus digest drifts,
     STOP: either it's a real regression to fix, or an intended builder change
     whose goldens you regenerate (`--ignored capture_goldens`) with a recorded
     reason, in the same commit.
   - **Benchmarks** (`BENCHMARKS.md`): re-run `codingest_bench` (release, min
     over median) against the reference repo(s); if timing moved beyond noise,
     update `BENCHMARKS.md` with the new codingest numbers and the date (the
     us-vs-in-tree tables are a frozen historical snapshot).
   Only touch these when they actually moved — a no-op release leaves them as-is.
5. **Rebuild the binaries** at the new version:
   `cargo build -p codingest-cli -p codingest-mcp --release` (a version bump
   stales any prebuilt binaries). Confirm both built clean.
6. **Promote CHANGELOG** `[Unreleased]` → `[x.y.z]` (with the date). Create
   `CHANGELOG.md` from the Keep-a-Changelog skeleton if it doesn't exist yet.
7. **Commit** as the final phase: `release(x.y.z): ...` — version bump +
   CHANGELOG promotion + any refreshed `PARITY.md`/`BENCHMARKS.md` in one commit
   (only the release files staged).
8. **Push and tag — invoking `/release` is the authorization.** If `origin` is
   wired, push `main` only after the safeguards pass: green gates, zero parity
   discrepancy, surgical staging, and ff-clean remote state. Poll branch CI to
   green, folding a shipped-code/infra fix (`fix(...)`/`ci(...)`) into the same
   release loop without re-asking (stop after ~3 iterations or a release-shape
   change). Then resolve the exact release commit, confirm `vX.Y.Z` is absent,
   create the lightweight tag used by this repo, and push that exact tag ref.
   Never move or replace an existing published tag. If there is no remote, stop
   at the local commit and report that publication cannot run.
9. **Publish and verify.** The tag triggers `.github/workflows/release.yml`.
   Monitor the entire Release run to green. Verify the version independently on
   crates.io (`codingest`, `codingest-cli`, `codingest-mcp`), PyPI (all expected
   wheels plus sdist), and the non-draft GitHub Release. A successful branch CI
   run is not publication. The workflow is idempotent-safe, so retry a partial
   failure only after diagnosing it; stop after ~3 repair iterations.
10. **Tidy dev-docs — perform directly, no prompt** (the `/release` invocation
    is the authorization). Follow the **`dev-docs-cleanup`** logic (todos.md-
    driven): auto-purge the time-boxed dirs, then read **only `todos.md`** —
    archive the now-shipped plan to `dev-docs/bin/` and prune its `todos.md`
    entry, move any other completed/stale docs to `bin/`, trim the entries (read
    a backlinked doc only to confirm it shipped). Carry the step-1 gaps into
    `todos.md`. Don't read `designs/` or sweep through `plans/`.
11. **Clean released PRs, branches, and worktrees — perform directly, no
    prompt.** After registry verification, inventory `git worktree list
    --porcelain`, local/remote branches, and open PRs. Resolve the release tag's
    exact commit and prove each cleanup candidate is contained in it with
    `git merge-base --is-ancestor <branch> <tag>`; never infer safety from a
    similar commit message. Close obsolete stacked PRs whose complete heads are
    contained in the released tag. For every contained non-protected branch:
    verify its worktree is clean and inspect untracked/ignored task artifacts
    (especially `dev-docs/` and `inbox/`), archiving anything still needed;
    remove that dedicated worktree, delete the local branch with `git branch
    -d`, then delete its exact remote ref with `git push origin --delete
    <branch>`. Switch the primary workspace back to `main` when safe,
    preserving unrelated tracked and untracked files. Never use `-D`, remove a
    dirty worktree or one with unarchived artifacts, delete `main`/the release
    tag, or delete a branch with unique commits or a still-active
    non-superseded PR. Finish with `git fetch --prune`, show the remaining
    worktrees/local/remote branches, and report every retained branch with the
    reason it was not safe to remove.

## Publishing prerequisites

Publication is automated by `.github/workflows/release.yml` on a `v*` tag. It
publishes the three Rust crates in dependency order, builds platform wheels and
an sdist for PyPI trusted publishing, creates the GitHub Release, and attaches
standalone CLI/MCP bundles. Before tagging, verify that the exact minimum
`kglite` and `kglite-mcp-server` versions required by `Cargo.toml` exist on
crates.io and that the matching `kglite` Python package exists on PyPI. A tag is
the irreversible publication boundary; do not create it until every gate and
prerequisite passes.

## Notes
- Keep responses under 400 tokens; write long diffs/logs/bench tables to a file
  under `dev-docs/temp/` and report the path.
- Version source of truth: `[workspace.package] version` in the root
  `Cargo.toml` (all crates inherit via `version.workspace = true`).
- This skill is the only place the version bumps. `phased-plan` never bumps.
