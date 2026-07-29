---
name: release
description: Cut a codingest release — goal-check against the phased-plan, bump the version across every manifest site first (workspace table AND the internal path-dependency pins), then ONE release build + test pass (incl. parity) that serves gate, record refresh, and binaries; conditional bench refresh; promote CHANGELOG, commit, ff-push main, then tag vX.Y.Z (the publish trigger — crates.io + PyPI via release.yml) and verify. Then clean up the branch and tidy dev-docs.
---

# Release

## Preconditions
- **No double-stage.** Check no release is already staged:
  `git log origin/main..HEAD --oneline | grep -E "release\("`. If a
  `release(x.y.z)` sits unpushed, **keep that version** — fold work into the
  same `[x.y.z]` block (one version bump per release).
- **Surgical staging.** If there's unrelated uncommitted work in the tree, don't
  block on it and don't sweep it in: **stage every release file explicitly by
  path** (`git add <file> …`, never `git add -A`/`.`) and verify with
  `git status --porcelain` that only release files are staged. Leave unrelated
  changes untouched for their author.

## Steps
1. **Goal check — did we achieve what we set out to do?** If this release ships
   a `phased-plan` project, read its plan (`dev-docs/plans/<slug>.md`) and the
   PR checklist, and confirm every planned phase actually shipped. List any
   phase **dropped, deferred, or only partially done** and surface the gaps
   before bumping — each gap is a conscious choice: finish it now, or carry it
   to `dev-docs/todos.md`. Don't let it vanish silently.
2. **Bump version FIRST — always patch** (`x.y.Z` → `x.y.Z+1`) **unless the
   release command itself named a minor or major.** Bumping before the gate
   means the single release build below serves the gate, the record refresh,
   AND the shipped binaries — no post-bump rebuild.
   - **Bump size is not a decision — do not stop and ask** (doctrine
     [[R6]]). This repo ships documented breaking changes in patch bumps, so a
     breaking change is *not* grounds to prompt; the prompt only ever
     re-confirmed a standing default. Semver findings are evidence for what to
     **write** in the CHANGELOG and the downstream notes, never a gate on what
     to **number**. Strictness belongs at the irreversible act (step 9), not at
     this routine one — and spending it here while step 9 self-authorized was
     exactly backwards.
   - **The version lives in SIX places, not one.** `[workspace.package] version`
     in the root `Cargo.toml` covers each crate's *own* `package.version` via
     `version.workspace = true`. It does **not** cover the internal dependency
     requirements: `crates/codingest-cli`, `crates/codingest-mcp`, and
     `crates/codingest-py` each declare `codingest = { version = "X.Y.Z", path
     = … }` (and `codingest-py` also pins `codingest-cli` and `codingest-mcp`),
     because `cargo publish` rejects a `path`-only dependency. Bump all of them
     together. This is the exact belief that broke KGLite's 0.15.0 release:
     "one line, no per-manifest bump" is **false**, and a patch bump only hides
     it — `^0.1.0` covers all of `0.1.x`, so the tree stays resolvable until the
     first minor bump, then every cargo call dies at once.
   - **Verify with a RESOLVING `cargo metadata` — never `--no-deps`.**
     `--no-deps` skips resolution entirely and passes on exactly the broken
     tree. Reproduced 2026-07-29 on a synthetic workspace of this shape: after
     bumping only `[workspace.package]` to `0.2.0`, `cargo metadata --no-deps`
     exits 0 while plain `cargo metadata` fails with *failed to select a version
     for the requirement `wsprobe-lib = "^0.1.0"`*. So run
     `cargo metadata --format-version 1 >/dev/null` (it must exit 0), then
     `cargo metadata --no-deps --format-version 1 | grep -o '"version":"[^"]*"'
     | sort -u` only to eyeball that every member reports the new version.
   - **kglite-floor prerequisite** (release.yml header): the minimum `kglite` /
     `kglite-mcp-server` version in the root `Cargo.toml` must already be live
     on crates.io (and the matching `kglite` Python package on PyPI) **before
     tagging** — a tag pushed ahead of the floor fails the publish jobs.
3. **Gate — one release build, one test pass, at the new version.** All green
   before continuing:
   - `cargo build --workspace --release` (this build IS the shipped binaries —
     `codingest` from the `codingest-cli` crate, plus `codingest-mcp` — no
     separate rebuild step)
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo test --workspace --release` — **and confirm `cargo test -p
     codingest --test parity` is in the run and green.** The golden-digest
     parity (`golden_parity` + `rev_self_consistency`, against digests frozen
     from the now-deleted in-tree `kglite::code_tree`) is the hard release
     gate; a subset that skips it can ship a graph-equivalence regression.
     This run doubles as step 4's parity evidence — don't re-run it there.
4. **Refresh the regression record (codingest's "captured constants").** These
   are the committed files that gate the project — refresh them off step 3's
   `--release` build so they reflect the shipped state:
   - **Parity** (`PARITY.md`): step 3's parity run is the evidence — no re-run.
     If the verdict or corpus set changed, update `PARITY.md` (date + result).
     A green `golden_parity` is the invariant — if a corpus digest drifts,
     STOP: either it's a real regression to fix, or an intended builder change
     whose goldens you regenerate (`--ignored capture_goldens`) with a recorded
     reason, in the same commit.
   - **Benchmarks** (`BENCHMARKS.md`) — **only if perf-sensitive paths changed
     since the last release** (parser hot loops, builder walk/partition/resolve
     stages, anything per-file; check
     `git diff <last-release-tag>..HEAD --stat -- crates/codingest/src`): run
     `codingest_bench` (release, min over median) against the reference
     repo(s); if timing moved beyond noise, update `BENCHMARKS.md` with the new
     numbers and date (the us-vs-in-tree tables are a frozen historical
     snapshot). If no perf-sensitive path changed, skip the bench and note it —
     an unchanged hot path doesn't need re-measuring every release.
   Only touch these files when they actually moved — a no-op release leaves
   them as-is.
5. **Binaries:** already built by step 3's workspace `--release` build at the
   new version. Just confirm `target/release/codingest` and
   `target/release/codingest-mcp` exist with fresh timestamps — rebuild only if
   something changed after step 3. **The CLI binary is `codingest`, not
   `codingest-cli`:** `codingest-cli` is the *crate* name, and its
   `Cargo.toml` declares `[[bin]] name = "codingest"`. There has never been a
   `target/release/codingest-cli`; release.yml packages `codingest` +
   `codingest-mcp`, so those two are what ship.
6. **Promote CHANGELOG** `[Unreleased]` → `[x.y.z]` (with the date). Create
   `CHANGELOG.md` from the Keep-a-Changelog skeleton if it doesn't exist yet.
7. **Commit** as the final phase: `release(x.y.z): ...` — version bump +
   CHANGELOG promotion + any refreshed `PARITY.md`/`BENCHMARKS.md` in one commit
   (only the release files staged).
8. **ff-push `main` — don't `checkout main`.** When releasing from a feature
   branch with unrelated WIP in the tree, a local `git checkout main` drags the
   WIP across and risks conflicts. Instead: confirm fast-forward
   (`git merge-base --is-ancestor origin/main HEAD`), then
   `git push origin HEAD:<branch>` (update the PR) and
   `git push origin HEAD:main` (ff `main`). The working tree never moves.
   Pushing `main` runs CI but **publishes nothing** — publish only triggers on
   a version tag.
9. **Tag + push the tag — THE publish trigger. STOP AND ASK FIRST.** Invoking
   `/release` authorized everything up to and including the release commit; it
   does **not** authorize this push (doctrine [[R6]]). Take **one confirmation
   immediately before pushing the tag**, stating the exact version and anything
   this run turned up that the user did not know when they typed `/release` —
   semver findings, the bench verdict, whether every artifact built.
   - The reason is informational, not ceremonial: `/release` is typed *before*
     the run learns any of that, so treating it as approval for the last push
     approves a decision made with strictly less information than exists when it
     is made. The asymmetry is total — strictness costs one prompt, the other
     direction costs an immutable publish whose only "undo" permanently breaks
     every pinned install.
   - **Accepted consequence: a release cannot complete unattended.** In a
     background session, stop at the staged release commit and wait.

   The tag is the irreversible publication boundary — don't
   create it until every gate and prerequisite above passes, and **never move or
   replace an already-pushed tag.** `git tag vx.y.z && git push origin vx.y.z`
   fires `release.yml`: crates.io in dependency order (`codingest` →
   `codingest-cli` → `codingest-mcp`), then platform wheels + sdist → PyPI via
   trusted publishing, then the GitHub Release with standalone CLI/MCP bundles
   attached. The confirmation above is scoped to this one release run (the tag
   push + its CI fix loop) and lapses once published or the user pivots — a new
   tag needs a new confirmation. All pre-push
   safeguards apply: gate green, parity zero-discrepancy, surgical staging,
   ff clean. Every publish path is idempotent-safe (404 guard /
   `skip-existing` / release-update), so a re-run after a partial failure is
   safe — re-run the failed job, don't re-tag a new version.
10. **Poll CI until green** — poll the GitHub **Checks API** directly
    (`gh api` / `gh run list`) rather than eyeballing the web UI; aggregate
    check names can register late. Wait for the `main` CI run AND every
    `release.yml` job to reach `conclusion: success`. Fix-and-push loop: a
    shipped-code/infra failure gets `fix(...)`/`ci(...)` folded in without
    re-asking; stop after ~3 iterations or any release-shape change (a tag
    already pushed means fixes ship via re-running the failed job or a patch
    release — surface which).
**Don't hand-verify what the workflow now enforces.** The fragile publish-path
shell no longer lives inline in `release.yml` (which only runs on a `v*` tag, so
nothing inline there could ever be *seen* to fail). It lives in
**`scripts/release_gates.sh`**, driven through both its pass and its fail path
by **`tests/release/test_release_gates.py`** in ci.yml's **`release-gates`**
job, on every push. Enforced there, not by you:
   - the extracted version is well-formed and matches the tag being released
     (the old `grep … | cut` masked pipeline reported `cut`'s status, always 0);
   - `if-no-files-found: error` on the wheel/sdist uploads, plus a real
     assertion on the artifact *set*;
   - `continue-on-error` narrowed to the genuinely fragile packaging steps
     rather than a whole job.
   If one of these goes red, read that job — don't re-derive it by hand.

**Still yours to check: the artifact SET, not just the version.** A version
check answers "did something publish", never "did everything publish" — compare
the artifact count and platform tags against the previous release.

**Two failure modes worth naming correctly:**
   - **A malformed/empty version does NOT publish nothing.** The crates.io probe
     treats **HTTP 404 as the publish signal**, and an empty version segment
     returns 404 (live-probed) — so an empty version means *publish
     everything*. The damage is to **re-run idempotency**: step 9's promise
     ("re-run the failed job, don't re-tag") breaks, because the retry now
     hard-errors `crate version already uploaded`.
   - **The real silent non-release is tag/manifest skew.** Tag `v0.1.4` while
     the manifest still says `0.1.3` → the probe finds 0.1.3 already live → all
     three crate publishes skip, `skip-existing` swallows the duplicate wheels,
     and a GitHub Release `v0.1.4` is still cut carrying **0.1.3** artifacts.
     Entire run green, nothing new shipped. Step 2's bump is what keeps these in
     lockstep; the gate script asserts it.

11. **Verify published — a green branch CI run is NOT publication.** Check the
    registries independently: crates.io shows all three crates at `x.y.z`
    (`curl -s -H "User-Agent: codingest-release" https://crates.io/api/v1/crates/<c> | jq -r .crate.max_version`
    — **without the User-Agent header crates.io returns null** and looks like a
    failed publish); PyPI shows `codingest` at `x.y.z`
    (`curl -s https://pypi.org/pypi/codingest/json | jq -r .info.version`) with
    the full expected wheel set plus the sdist; and the GitHub Release for
    `vx.y.z` exists and is **not a draft**.
12. **Clean released PRs, branches, and worktrees — perform directly, no
    prompt** (the `/release` invocation authorizes this). After registry
    verification, inventory `git worktree list --porcelain`, local/remote
    branches, and open PRs. Resolve the release tag's exact commit and **prove**
    each cleanup candidate is contained in it with
    `git merge-base --is-ancestor <branch> <tag>` — never infer safety from a
    similar commit message. Close obsolete stacked PRs whose complete heads are
    contained in the released tag. For every contained, non-protected branch:
    verify its worktree is clean and inspect untracked/ignored task artifacts
    (especially `dev-docs/` and `inbox/`), archiving anything still needed;
    remove that dedicated worktree, then `git branch -d <branch>` (refuses if
    somehow unmerged — don't `-D` past that) and `git push origin --delete
    <branch>`. To return the primary workspace to `main` without disturbing
    WIP: `git branch -f main origin/main`, then `git switch main` (a zero-diff
    switch when `main == HEAD`, so tracked and untracked WIP is preserved).
    Confirm the released PR shows `MERGED` (a ff-push to `main` auto-marks it;
    if OPEN, the commits didn't land — investigate, don't force-close).
    **Never** use `-D`, remove a dirty worktree or one with unarchived
    artifacts, delete `main`/`gh-pages`/the release tag, or delete a branch
    with unique commits, an open `dependabot/*` PR, or a still-active
    non-superseded PR. Finish with `git fetch --prune`, show the remaining
    worktrees and local/remote branches, and report every retained branch with
    the reason it wasn't safe to remove. Periodic sweep:
    `git branch --merged origin/main` surfaces any stale-branch backlog.
13. **Tidy dev-docs — perform directly, no prompt** (the `/release` invocation
    is the authorization). Follow the **`dev-docs-cleanup`** logic (todos.md-
    driven): auto-purge the time-boxed dirs, then read **only `todos.md`** —
    archive the now-shipped plan to `dev-docs/bin/` and prune its `todos.md`
    entry, move any other completed/stale docs to `bin/`, trim the entries (read
    a backlinked doc only to confirm it shipped). Carry the step-1 gaps into
    `todos.md`. Don't read `designs/` or sweep through `plans/`.

## Notes
- Keep responses under 400 tokens; write long diffs/logs/bench tables to a file
  under `dev-docs/temp/` and report the path.
- Version source of truth: `[workspace.package] version` in the root
  `Cargo.toml` (all crates inherit their own `package.version` via
  `version.workspace = true`) — **plus** the five internal path-dependency pins
  in `crates/codingest-cli`, `crates/codingest-mcp`, and `crates/codingest-py`,
  which the workspace table does not reach. Six sites, one version.
- This skill is the only place the version bumps. `phased-plan` never bumps.
- Branch and `main` pushes are routine (CI only). The **version-tag push** is
  the one publish-triggering, authorization-gated action.
