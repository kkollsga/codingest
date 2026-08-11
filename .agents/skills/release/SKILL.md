---
name: release
description: Cut a codingest release — goal-check against the phased-plan, bump the version across every manifest site first (workspace table AND the internal path-dependency pins), then ONE release build + test pass (incl. parity) that serves gate, record refresh, and binaries; conditional bench refresh; promote CHANGELOG, commit, ff-push main, then tag vX.Y.Z (the publish trigger — crates.io + PyPI via release.yml) and verify. Then clean up the branch and tidy dev-docs. Runs to completion autonomously — invoking it IS the approval.
---

# Release

## This run ends in a published version or a named blocker

Read this before step 1; it governs every step below (doctrine [[R12]]).

**The completion condition.** The run is done when **step 11 has verified
`x.y.z` on crates.io (all three crates) and on PyPI with the full artifact set,
and the GitHub Release exists and is not a draft** — or when you have surfaced a
**specific blocker you cannot fix**. Nothing else is an ending.

**The non-endings.** "CI is running", "the commit is staged", "the tag is
pushed", "waiting for the wheels", "next I will…" are **not endings**. Each is a
natural pause point that feels like a reasonable place to hand back control, and
each is indistinguishable from the inside from genuinely needing input. The
failure is not a decision to stop — it is failing to notice that continuing is
an option. Three releases across the estate stalled exactly this way on
2026-07-30/31, every check green, no version shipped; the user noticed before
the agent did, twice.

**Waiting is not a checkpoint.** Poll it, background it, or block on it — all
three continue the run. Handing control back does not. A poll that takes twenty
minutes is twenty minutes of polling, not a reason to end the turn. The
crates.io index-propagation wait and the multi-platform wheel builds are the two
places this run is most likely to stall.

**A red CI is a task, not a verdict.** Diagnose, fix, push, re-poll, repeat —
within step 10's stated bound (~3 iterations), then surface what remains as a
named blocker.

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
  - **That verification is not a formality: `git add` is all-or-nothing across
    its pathspecs.** One bad path — a typo, a file the plan renamed, a
    `BENCHMARKS.md` you did not actually refresh — aborts the entire
    invocation, so *none* of the other files are staged either. The failure is
    quiet in the reassuring direction: the next `git commit` succeeds, on a
    release commit missing the bump. Read back the staged set
    (`git diff --cached --name-only`) and confirm it matches the intended list
    before committing. (KGLite hit this on 2026-08-09.)

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
   - **Escalation is one-way: it comes from the user, never from the agent**
     (doctrine 0.1.4). The agent never *suggests*, *recommends*, or *announces*
     a minor/major bump — not in a readiness report, not as a "0.X.0 unless you
     object" default. An agent-announced number the user did not repeat back in
     their own typed words is **void**: invoking `/release` past it adopts the
     patch default, not the announcement — silence over an agent-stated default
     is not user input. Cost of learning this: 0.2.0 shipped 2026-08-11 off
     exactly that shape, and crates.io versions are permanent.
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
   - **`cargo publish --dry-run --workspace --allow-dirty`** — packages all three crates,
     normalizes their manifests, and *builds each packaged copy*. Must exit 0.
     - **Why this is a gate and not a nicety.** `release.yml` publishes
       **crates.io first**, and every other job hangs off `needs:
       [publish-crate]`. So a packaging or metadata fault in `codingest-cli` or
       `codingest-mcp` would otherwise surface only *after* `codingest` is
       permanently published — a half-published release with no undo. Before
       0.1.4 this skill verified packaging only by doing it irreversibly.
     - **`--workspace` is load-bearing; do not reach for `-p <crate>`.** A bare
       `cargo publish --dry-run -p codingest-cli` **fails** (rc 101, *failed to
       select a version for the requirement `codingest = "^X.Y.Z"`*): it
       resolves the internal dependency against crates.io, where the new
       version does not exist yet. `--workspace` makes cargo resolve
       inter-member dependencies against the local unpublished crates and
       verify all three together. Verified on cargo 1.97.0. Without this note
       the natural conclusion is that dependents cannot be dry-run at all —
       which is wrong, and which cost this project a release's worth of
       unnecessary exposure.
     - **`--allow-dirty` is required here, not optional.** This step runs
       *after* step 2's bump, so the tree always has uncommitted manifest
       changes; without the flag cargo refuses with *"files in the working
       directory contain changes that were not yet committed into git"* and
       exits 101 having verified **nothing about packaging**. Discovered the
       hard way on the 0.1.5 run: the instruction was written and tested on a
       clean tree, so it had never met the sequence it actually lives in. The
       flag is also correct on the merits — the uncommitted bump is precisely
       what you want packaged and verified.
     - `codingest-py` is correctly absent: it is `publish = false` (it ships to
       PyPI as a wheel, not to crates.io). The dry-run set must match the three
       `cargo publish -p …` steps in `release.yml` exactly.
   - **Preflight the Python artifacts too** — both of these otherwise run
     *only* inside `release.yml`, i.e. only after crates.io has published:
     - wheel: `maturin build --release -m crates/codingest-py/Cargo.toml --out
       <dir>` then `python3 scripts/verify_wheel.py <dir>/*.whl`
     - sdist: `maturin sdist -m crates/codingest-py/Cargo.toml --out <dir>`
       then `test "$(tar -tzf <dir>/*.tar.gz | grep -Ec '/LICENSE$')" -eq 1`
       (use that exact pattern — a looser `grep LICENSE` can match a different
       entry and report 1 while the real gate sees 0). Keep the `grep -Ec`
       inside `$( )` as written: **`grep -c` exits 1 when the count is zero**,
       so lifting it out into a `grep -Ec … && …` chain — or running it under
       `set -e` — turns the one answer you need to act on into a dead script
       rather than a `0` you can test.
     This only covers the host platform; the other wheel legs still build in CI
     on the tag. That residual is real and worth stating in step 9's report
     rather than pretending it is covered.
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
     `codingest_bench` (release, min over median — but judge a heavy-tailed
     cell by its median, and a once-per-build cost by the mean of first
     events) against the reference
     repo(s); if timing moved beyond noise, update `BENCHMARKS.md` with the new
     numbers and date (the us-vs-in-tree tables are a frozen historical
     snapshot). If no perf-sensitive path changed, skip the bench and note it —
     an unchanged hot path doesn't need re-measuring every release.
   - **Perf anchor** (`tests/benchmarks/baselines/`) — **runs every release,
     unconditionally.** Unlike the `BENCHMARKS.md` refresh above, this one has
     no "only if perf-sensitive paths changed" escape: the whole point is
     cumulative drift that no single release looked responsible for. 0.1.6
     shipped nodes +13.03 % against a ≤12 % budget and build +20.16 % against
     ≤15 % because the budgets lived in prose and were read by a human against
     the wrong denominator. Full rationale: `tests/benchmarks/README.md`.
     1. Capture, from step 3's **release** build, in **both** docs modes.
        Redirect the streams separately — `codingest_bench` writes warnings to
        stderr before the JSON, so `2>&1` yields a file that is not JSON:
        ```sh
        cargo build --release -p codingest --bin codingest_bench
        ./target/release/codingest_bench tests/corpus --json           > /tmp/on.json  2>/dev/null
        ./target/release/codingest_bench tests/corpus --no-docs --json > /tmp/off.json 2>/dev/null
        ```
     2. Pick the anchor and compare **both** modes against it:
        ```sh
        anchor=$(scripts/bench_anchor.sh select-baseline tests/benchmarks/baselines --window 3)
        scripts/bench_anchor.sh compare --current /tmp/on.json  --baseline "$anchor"
        scripts/bench_anchor.sh compare --current /tmp/off.json --baseline "$anchor"
        ```
        `select-baseline` returns the **oldest** baseline within the last 3
        releases — anchoring to the previous release only ever sees one step
        of drift, which is how 0.1.6's slide went unnoticed.
     3. **Read the exit code, and do not put either command in a pipeline.** A
        pipeline reports its last stage, so `… | tail` turns every verdict
        into 0 — the exact mask that made release.yml's version extraction
        unfailable. The four codes mean different things and one of them is
        not a failure to fix:
        - **1 FAIL — this BLOCKS the tag.** Either the regression is real and
          gets fixed, or the growth is intended, gets argued in
          `BENCHMARKS.md`, and is re-baselined in the same commit. Never
          re-baseline to silence an unexplained diff — that is the golden-file
          rule from the parity gate, applied here.
        - **3 REFUSE** — corpus digest or docs mode differs. No delta was
          computed. Someone changed a `tests/corpus` fixture; recapture the
          baseline for the new corpus, and say so in the release notes.
        - **4 VOID** — the control query moved; the instrument moved, not
          necessarily the code. Re-measure on a settled machine. Do **not**
          bisect and do not read the other rows.
        - **0 PASS** — proceed.
     4. **After the release is green** (step 11 verified publication), capture
        the new baseline as `tests/benchmarks/baselines/<x.y.z>.json` with a
        full `captured_at_commit`, then
        `scripts/bench_anchor.sh prune tests/benchmarks/baselines --keep 4
        --delete`. Commit both together. The capture procedure — three runs
        per mode, min per query, mean for `build_secs`, floors at ~2.5x the
        observed spread — is in `tests/benchmarks/README.md`; follow it rather
        than reconstructing it.
     **Live but degraded until three baselines exist.** As of 0.1.7 there is
     exactly one, so `select-baseline` anchors to it and prints a DEGRADED
     notice on stderr: the gate works, it just spans one release of history
     instead of three. It tightens on its own as baselines accumulate — no
     action needed, but do not read a PASS as three releases of evidence until
     `ls tests/benchmarks/baselines/` shows three files.
   Only touch these files when they actually moved — a no-op release leaves
   them as-is.
5. **Binaries:** already built by step 3's workspace `--release` build at the
   new version. Confirm `target/release/codingest` and
   `target/release/codingest-mcp` are actually newer than everything they are
   built from — **run this, don't eyeball it.** "Fresh timestamps" is not a
   judgement call: a binary left over from before the version bump looks
   entirely plausible in `ls -l`, and shipping it means shipping the previous
   release's code under the new version number.

   ```sh
   for b in target/release/codingest target/release/codingest-mcp; do
     [ -x "$b" ] || { echo "ABSENT:  $b"; continue; }
     n=$( { find crates -name '*.rs' -newer "$b" -print -quit
            find Cargo.lock -newer "$b" -print -quit; } | head -1 )
     [ -z "$n" ] && echo "FRESH:   $b" || echo "STALE:   $b (newer: $n)"
   done
   ```

   **Rule: anything but `FRESH` on both lines means STOP and rebuild** —
   `cargo build --workspace --release` — then re-run the check until both read
   `FRESH`. Do not continue to step 6 on a `STALE` or `ABSENT` line, and do not
   reason about *why* a file is newer; rebuilding is cheaper than being wrong.
   `Cargo.lock` is in the comparison alongside `crates/**/*.rs` because a
   dependency bump changes the shipped binary without touching a single line of
   our own source. **The CLI binary is `codingest`, not
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
9. **Tag + push the tag — THE publish trigger. Report, then push; do not stop.**
   Invoking `/release` authorizes the **entire run, including this push**
   (doctrine [[R6]]). Immediately before pushing, *report* — the exact version,
   the semver findings, the bench verdict, whether every artifact built, and
   anything else learned since invocation — then push in the same turn.
   - **A report is not a gate, and substituting one for the other degrades
     both.** This step blocked on a confirmation between 2026-07-30 and
     2026-07-31; the rule was reversed on evidence. It fired *after* the
     decision it claimed to guard — by the time the release commit exists the
     bump, the constants and the CHANGELOG are settled, so the prompt could not
     change the outcome, only delay it. And it broke unattended runs: a kglite
     release stalled at a staged commit while the user was away, and the failure
     mode was not "published something wrong" but **"published nothing,
     silently"** — the direction the strict rule was not considering.
   - The safety on this irreversible push lives in **checks that can fail** and
     that all sit upstream of it: green branch CI, the resolving `cargo
     metadata`, the `--dry-run --workspace` preflight, parity, the artifact-set
     verification in step 11. **A prompt cannot fail; it can only wait** — see
     [[R1]]. Adding one where a check belongs feels like rigour and buys none.
   - What still needs case-by-case approval is unchanged: outward-facing
     publication that is *not* a release — issues, comments, emails, anything
     attributed to the maintainer. `/release` authorizes a release. It
     authorizes nothing else.

   The tag is the irreversible publication boundary — don't
   create it until every gate and prerequisite above passes, and **never move or
   replace an already-pushed tag.** `git tag vx.y.z && git push origin vx.y.z`
   fires `release.yml`: crates.io in dependency order (`codingest` →
   `codingest-cli` → `codingest-mcp`), then platform wheels + sdist → PyPI via
   trusted publishing, then the GitHub Release with standalone CLI/MCP bundles
   attached. All pre-push
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

    **Agent worktrees live in `codingest-worktrees/<name>`** (a sibling
    directory of the repo, never loose in the `Rust/` parent), and that
    directory exists **only while worktrees are in progress** — this step empties
    it and deletes it. Per worktree, in order: migrate outstanding actions into
    `dev-docs/todos.md` (branch, state, what remains, how to resume) → if dirty,
    save its `git diff` under `dev-docs/` **first** → `git worktree remove` +
    `git worktree prune`. Removing a worktree never deletes its branch (the ref
    lives in the main repo's `.git`), so unmerged work always survives removal.
    **Trap: a branch whose commits landed by *rebase* reads as unmerged** to
    `git merge-base --is-ancestor`, so the containment proof above will claim
    unique work that does not exist — `git cherry -v main <branch>` sees through
    it (`-` = already upstream). Second trap: a fresh worktree does **not**
    inherit the repo's build-cache symlink, so it cold-builds onto whatever
    volume the workspace sits on.
13. **Tidy dev-docs — perform directly, no prompt** (the `/release` invocation
    is the authorization). Follow the **`dev-docs-cleanup`** logic (todos.md-
    driven): auto-purge the time-boxed dirs, then read **only `todos.md`** —
    archive the now-shipped plan to `dev-docs/bin/` and prune its `todos.md`
    entry, move any other completed/stale docs to `bin/`, trim the entries (read
    a backlinked doc only to confirm it shipped). Carry the step-1 gaps into
    `todos.md`. Don't read `designs/` or sweep through `plans/`.

    **Adapter resync — diff each adapter against its declared authority,
    rename-aware.** Identical: done. Divergent: classify each hunk before
    touching either side — an *improvement* is merged into the **authority**
    first and the adapter regenerated from it; *staleness* is simply regenerated
    away. Never run a blind sync on a divergent pair: blind sync deletes
    improvements (sonara, 2026-08-10, ~20 lines), and no sync preserves stale
    doctrine the other harness will follow. The mirror check must pass
    afterwards. Here the two pairs are: the **conventions files**, which must
    come out byte-identical (`diff` empty — the Authority line is exempt from the
    substitution and reads the same in both copies), and each **skill pair**,
    which must differ only by the harness-name substitution
    (`diff <(sed 's/<other>/<mine>/g' <authority>) <adapter>` empty). Which side
    is the authority is stated in the Authority line at the top of the
    conventions file: the conventions file itself, and the tracked
    `.agents/skills/` tree.

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
