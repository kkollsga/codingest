#!/usr/bin/env bash
#
# Release-path gate logic for codingest.
#
# WHY THIS FILE EXISTS
# --------------------
# `.github/workflows/release.yml` runs only on a `v*` tag push, so nothing in it
# can ever be exercised by branch CI. That made this project's hard rule — "a new
# gate is not trusted until you have seen it fail" — impossible to satisfy for
# any check written inline in that workflow. The fragile shell therefore lives
# here, where `tests/release/test_release_gates.py` drives every function through
# both its pass path and its fail path on every push, and the workflow steps are
# thin calls into these functions.
#
# Invoke either way:
#   scripts/release_gates.sh <function> [args...]     # from a workflow step
#   source scripts/release_gates.sh                    # from the unit test
#
# ============================================================================
# THE COMMAND-SUBSTITUTION TRAP — READ THIS BEFORE ADDING A CHECK HERE.
#
#     check() { [ -n "$1" ] || exit 1; printf '%s' "$1"; }
#     echo "version=$(check "$V")" >> "$GITHUB_OUTPUT"      # <-- BROKEN
#
# `exit 1` inside `$( )` kills only the substitution subshell. The enclosing
# `echo` still succeeds, so `set -e` sees rc 0 and the step passes green with an
# empty value. This is precisely how a gate is born dead, and it is how the
# upstream project's own first attempt at this fix ended up vacuous.
#
# THE RULE HERE: a function either COMPUTES A VALUE (prints to stdout, decides
# nothing) or ASSERTS A CONDITION / EMITS OUTPUTS (returns non-zero on failure,
# and is never invoked from inside a `$( )` nested in another command). A caller
# that needs both writes two statements, so the status is the status of a whole
# command and `set -e` can see it:
#
#     version=$(extract_version Cargo.toml)          # rc lands in $?
#     assert_something "$version" || exit 1          # rc is this command's rc
#
# No function in this file may call `exit` — the unit test enforces that. Use
# `return`, which propagates to a caller that is not hiding inside `$( )`.
#
# The same discipline applies to pipelines: a pipeline reports its LAST stage's
# status, so `grep ... | cut ...` is rc 0 whenever `cut` runs — which is always,
# match or no match. That masked pipeline is what made release.yml's version
# extraction unfailable. Split the pipeline and observe the producer.
# ============================================================================

set -euo pipefail

# crates.io policy requires a contact-email/URL User-Agent on API requests.
CRATES_IO_UA="codingest-ci/0.1.0 (https://github.com/kkollsga/codingest)"

# How many times the crates.io probe may ask before it gives up and fails the
# run. Overridable so the unit test can drive both the "second attempt settles
# it" and the "never settles" paths without waiting. The sleep is indirected for
# the same reason.
CRATES_IO_PROBE_ATTEMPTS="${CRATES_IO_PROBE_ATTEMPTS:-3}"
CRATES_IO_PROBE_BACKOFF="${CRATES_IO_PROBE_BACKOFF:-5}"

# ---------------------------------------------------------------------------
# Output plumbing
# ---------------------------------------------------------------------------

# EMIT. Append `KEY=VALUE` to the GitHub step-output file. Falls back to stdout
# when GITHUB_OUTPUT is unset (local runs); the unit test points it at a temp
# file and asserts the exact key/value block, so the workflow's output contract
# is covered by the same test as the logic.
gh_output() {
  printf '%s=%s\n' "$1" "$2" >> "${GITHUB_OUTPUT:-/dev/stdout}"
}

# ---------------------------------------------------------------------------
# Version
# ---------------------------------------------------------------------------

# COMPUTE. Print the workspace version declared in a Cargo manifest.
#
# Faithful port of release.yml's `grep -m 1 '^version' Cargo.toml | cut -d '"'
# -f 2`, with the pipeline split so grep's status is actually observed. As one
# pipeline the step reported `cut`'s status, which is 0 even when grep matched
# nothing at all.
#
# POSITIONAL FRAGILITY — kept faithful on purpose, flagged here so nobody reads
# it as robust. `grep -m 1 '^version'` selects the FIRST unindented `version`
# key in the file. It lands on the intended `[workspace.package] version` only
# because that table happens to precede `[workspace.dependencies]` in our root
# Cargo.toml. Any unindented `version` key added above it — a new table, a
# reordering — silently wins, and nothing here would notice. What DOES notice is
# `assert_tag_matches_manifest`: a wrong-but-well-formed version taken from the
# wrong table will not equal the tag being released, so on a tag build the skew
# gate fails the run. That is the real backstop; this grep is not.
#
#   rc 0 — a `^version` line was found. The printed value may still be wrong or
#          empty (e.g. an unquoted `version.workspace = true` line, which `cut`
#          passes through verbatim); validating it is `assert_version_shape`.
#   rc 1 — no unindented `version` line in the manifest.
extract_version() {
  local manifest="$1" line
  line=$(grep -m 1 '^version' "$manifest") || return 1
  printf '%s\n' "$line" | cut -d '"' -f 2
}

# ASSERT. The extracted version must look like a version: `^[0-9]+\.[0-9]+\.[0-9]+`
# (prefix-anchored, so `1.0.0-rc.1` passes and `1.2` / `abc` / `` do not).
#
# WHAT AN UNVALIDATED VERSION ACTUALLY BREAKS — state this correctly, an earlier
# note in this project got it wrong and the error is still repeated in
# `.claude/skills/release/SKILL.md`. An empty VERSION does NOT cause a silent
# non-release. The crates.io API returns **404** for an empty version segment
# (`/api/v1/crates/codingest/`), and 404 is our publish signal — so an empty
# version means "publish EVERYTHING", not "publish nothing". The damage is to
# re-run idempotency: the 404-guard exists so a retry after a partial failure
# skips what already went out, and with an unvalidated version that guard is
# blind, so the retry reaches `cargo publish` and hard-errors "crate version
# already uploaded". The silent-no-op release comes from tag/manifest skew
# instead — see `assert_tag_matches_manifest`.
#
#   rc 0 — well-formed.
#   rc 1 — empty or malformed; an `::error::` annotation is on stderr.
assert_version_shape() {
  local version="${1:-}"
  if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]; then
    printf '::error::malformed version %s — expected N.N.N (e.g. 0.1.3)\n' \
      "'${version}'" >&2
    return 1
  fi
}

# COMPUTE. Strip a leading `v` from a git ref name: `v0.1.3` -> `0.1.3`.
version_from_ref() {
  printf '%s\n' "${1#v}"
}

# ASSERT. On a tag build, the manifest version must equal the tag.
#
#   assert_tag_matches_manifest <manifest_version> <ref_name> <ref_type>
#
# THE SILENT NO-OP RELEASE THIS PREVENTS. Tag `v0.1.4` while Cargo.toml still
# says `0.1.3` and every downstream check agrees the run is fine: the crates.io
# probe asks about 0.1.3, gets 200 (already published), so all three crate
# publishes skip; the wheels build at 0.1.3 and `skip-existing: true` swallows
# the duplicates on PyPI; there is no `## [0.1.4]` changelog section so the
# release notes silently degrade to auto-generated ones; and a GitHub Release
# named `v0.1.4` is created with 0.1.3 artifacts attached. The whole run is
# green and nothing was released. No shell bug is required — before this gate
# there was no check anywhere comparing the tag to the manifest.
#
# ref_type is GitHub's `GITHUB_REF_TYPE` (`tag` or `branch`). On anything that
# is not a tag — notably `workflow_dispatch`, where GITHUB_REF_NAME is a BRANCH
# (`main`) and `${GITHUB_REF_NAME#v}` would compare the manifest against the
# string `main` — the gate does not fire. It reports the skip on stderr rather
# than passing mutely, so a dispatch run cannot be mistaken for a verified one.
# (Phase 5 hardens the dispatch path itself; this function only refuses to
# invent a comparison for it.)
#
#   rc 0 — versions agree, or this is not a tag build.
#   rc 1 — tag/manifest skew; an `::error::` annotation is on stderr.
assert_tag_matches_manifest() {
  local version="${1:-}" ref_name="${2:-}" ref_type="${3:-}" tag_version
  if [ "$ref_type" != "tag" ]; then
    printf 'ref %s is a %s, not a tag — tag/manifest skew gate not applicable\n' \
      "'${ref_name}'" "'${ref_type:-unknown}'" >&2
    return 0
  fi
  tag_version=$(version_from_ref "$ref_name")
  if [ "$version" != "$tag_version" ]; then
    printf '::error::tag/manifest skew: tag %s wants version %s but the manifest declares %s. Publishing would skip every crate (the 404 probe asks about %s), ship %s wheels under a %s release, and fall back to auto-generated notes — all green.\n' \
      "$ref_name" "'${tag_version}'" "'${version}'" "$version" "$version" \
      "$ref_name" >&2
    return 1
  fi
  printf 'tag %s matches the manifest version %s\n' "$ref_name" "$version" >&2
}

# ASSERT. The ref this workflow is running on is fit to publish under.
#
#   assert_publish_ref <ref_name> <ref_type> [event_name]
#
# WHAT THIS BLOCKS. `release.yml` used to also carry a `workflow_dispatch:`
# trigger. On a dispatch run GITHUB_REF_NAME is a BRANCH, not a tag, and every
# consumer of it downstream quietly does the wrong thing: `${GITHUB_REF_NAME#v}`
# becomes `main`, so the changelog lookup asks for a `## [main]` section and
# degrades to auto-generated notes; the binaries are packaged as
# `codingest-main-linux-x86_64.tar.gz`; and `softprops/action-gh-release` is
# handed a non-tag ref, which makes it CREATE a tag+release named after the
# branch. None of that is a failure anywhere — it is a green run that publishes
# garbage under a branch name.
#
# THE DECISION: dispatch is blocked from the publish path outright rather than
# allowed in a degraded dry-run mode. A dry-run mode would have to condition
# every publish and release action on it, and each of those conditions is
# itself a construct branch CI can never exercise — more unfailable surface, to
# buy a rehearsal that `ci.yml` already provides on every push (it builds the
# workspace, builds and verifies a wheel, and runs this suite). The `on:` block
# therefore lists only the `v*` tag push, and this function is the backstop for
# any other way a non-tag ref could reach a publish step.
#
# Belt and braces on purpose: `publish-crate` gates every other job, so one
# guard there is already structurally sufficient. It is repeated in the two jobs
# that would otherwise USE the ref name (`publish-pypi` for the changelog,
# `release-binaries` for the archive filenames) because "structurally
# sufficient" is a property of today's `needs:` graph, not an invariant.
#
#   rc 0 — a `v<N.N.N>` tag.
#   rc 1 — anything else; an `::error::` annotation is on stderr.
assert_publish_ref() {
  local ref_name="${1:-}" ref_type="${2:-}" event="${3:-}"
  if [ "$ref_type" != "tag" ]; then
    printf '::error::refusing to run the publish path on %s: ref %s is a %s, not a tag. GITHUB_REF_NAME would be a branch name, so the crates would publish under it, the binaries would be packaged as codingest-%s-<platform> and softprops would create a release named after the branch — all green. Push a v<N.N.N> tag instead.\n' \
      "'${event:-this event}'" "'${ref_name}'" "'${ref_type:-unknown}'" \
      "$ref_name" >&2
    return 1
  fi
  if [[ ! "$ref_name" =~ ^v[0-9]+\.[0-9]+\.[0-9]+ ]]; then
    printf '::error::refusing to run the publish path on tag %s — a release tag must be v<N.N.N> (e.g. v0.1.3). Anything else has no manifest version to agree with and no changelog section to find.\n' \
      "'${ref_name}'" >&2
    return 1
  fi
  printf 'publishing from tag %s (event %s)\n' "$ref_name" "'${event:-unknown}'" >&2
}

# ---------------------------------------------------------------------------
# crates.io publish decision
# ---------------------------------------------------------------------------

# COMPUTE. Print the HTTP status crates.io returns for <crate>/<version> as a
# three-digit string ("000" when curl could not reach the host — there is no
# `-f` here, so curl exits 0 for any status).
#
# The curl binary is indirected through CODINGEST_RELEASE_CURL so the unit test
# can stub it and stay offline.
crates_io_status() {
  local crate="$1" version="$2"
  "${CODINGEST_RELEASE_CURL:-curl}" -s -o /dev/null -w "%{http_code}" \
    -A "$CRATES_IO_UA" "https://crates.io/api/v1/crates/$crate/$version"
}

# COMPUTE. Ask crates.io until it gives a CONCLUSIVE answer or the attempts run
# out; print the last status seen.
#
#   crates_io_probe <crate> <version>
#
# Conclusive means 200 (already published) or 404 (not published). Everything
# else — 000 from a connection failure, 403 from the rate limiter, any 5xx — is
# retried with a fixed backoff, because those are overwhelmingly transient and
# the alternative (fail the run) costs a maintainer a whole re-run at the one
# moment a release is in flight. A curl that exits non-zero is folded into "000"
# here rather than killing the function under `set -e`: a hard curl error is
# exactly the transient case retrying exists for, and if it persists the caller's
# `assert_status_conclusive` still fails the run loudly.
#
# Retrying is the ONLY leniency added; the retry never invents an answer. When
# the attempts are exhausted the inconclusive status is printed as-is and the
# decision of what to do with it belongs to `assert_status_conclusive`.
crates_io_probe() {
  local crate="$1" version="$2" attempt=1 status
  while : ; do
    status=$(crates_io_status "$crate" "$version") || status="000"
    case "$status" in
      200 | 404)
        printf '%s\n' "$status"
        return 0
        ;;
    esac
    if [ "$attempt" -ge "$CRATES_IO_PROBE_ATTEMPTS" ]; then
      break
    fi
    printf 'crates.io answered HTTP %s for %s %s (attempt %d/%d) — retrying in %ss\n' \
      "$status" "$crate" "$version" "$attempt" "$CRATES_IO_PROBE_ATTEMPTS" \
      "$CRATES_IO_PROBE_BACKOFF" >&2
    "${CODINGEST_RELEASE_SLEEP:-sleep}" "$CRATES_IO_PROBE_BACKOFF"
    attempt=$((attempt + 1))
  done
  printf '%s\n' "$status"
}

# ASSERT. A crates.io probe status must be one we can act on.
#
#   assert_status_conclusive <crate> <version> <status>
#
# THE SILENT SKIP THIS REPLACES. `curl -s -o /dev/null -w '%{http_code}'` has no
# `-f`, so curl exits 0 for ANY outcome — including no outcome at all, which it
# reports as the status "000". The old decision was a two-way branch on `= 404`,
# so a DNS blip, a rate-limit 403 or a crates.io 5xx all landed in the else arm
# and skipped every one of the three crate publishes with the run still green.
# The "skip when unsure" half of that was deliberate and is KEPT — see
# `publish_decision_for`, which still refuses to publish on anything but a 404.
# What was wrong was that the uncertainty exited GREEN and SILENT, so a release
# that shipped nothing to crates.io looked exactly like a release that had
# nothing to ship. Uncertainty now fails the step.
#
#   rc 0 — 200 or 404.
#   rc 1 — anything else; an `::error::` annotation is on stderr.
assert_status_conclusive() {
  local crate="${1:-}" version="${2:-}" status="${3:-}"
  case "$status" in
    200 | 404) return 0 ;;
  esac
  printf '::error::crates.io returned HTTP %s for %s %s after %d attempt(s) — cannot tell whether this version is already published. Refusing to guess: treating it as "already published" would silently skip every crate publish and still leave the run green (the old behaviour), and treating it as "not published" could double-publish. Re-run the workflow once crates.io answers.\n' \
    "'${status:-none}'" "$crate" "'${version}'" "$CRATES_IO_PROBE_ATTEMPTS" >&2
  return 1
}

# COMPUTE. Map a CONCLUSIVE crates.io HTTP status to the publish decision,
# printing exactly `true` or `false`. HTTP 404 is the only publish signal: 200
# means the version is already there.
#
# The `else` arm stays a `false` — "never blindly publish on uncertainty" is a
# property worth keeping even though `assert_status_conclusive` now stops an
# uncertain status from reaching here at all. Two independent reasons not to
# publish on a status this function does not recognise is the right number.
publish_decision_for() {
  if [ "$1" = "404" ]; then
    printf 'true\n'
  else
    printf 'false\n'
  fi
}

# COMPUTE. The `$GITHUB_OUTPUT` key suffix for a crate name (`-` is not legal in
# a shell-ish output key the workflow reads back as `publish_codingest_cli`).
crate_output_key() {
  printf '%s\n' "$1" | tr '-' '_'
}

# EMIT. The whole `Check which crates need publishing` step: write the `version`
# output plus one `publish_<key>` output per crate named on the command line.
# Human-readable log lines go to stderr so stdout stays machine-readable.
#
#   check_crates_to_publish Cargo.toml codingest codingest-cli codingest-mcp
#
# The tag/manifest comparison reads GITHUB_REF_NAME / GITHUB_REF_TYPE, which
# GitHub Actions always sets; both default to empty, and an empty ref_type is
# not `tag`, so a local run skips the skew gate rather than inventing a verdict.
#
# ORDER MATTERS, TWICE OVER. Both version assertions run BEFORE the first
# `gh_output`, so a bad version can never reach `$GITHUB_OUTPUT` and no crates.io
# probe is even sent. And every crate is PROBED AND CHECKED before ANY decision
# is written, so one inconclusive status leaves the whole step's output empty
# instead of a half-written set — a step that fails after emitting
# `publish_codingest=true` is a step whose outputs describe a decision that was
# never actually taken.
#
#   rc 0 — outputs written.
#   rc 1 — no version line, a malformed version, tag/manifest skew, or a
#          crates.io status we cannot act on; NOTHING is written.
check_crates_to_publish() {
  local manifest="${1:-Cargo.toml}"
  if [ $# -gt 0 ]; then shift; fi
  local version crate key status decision probes="" unknown=0
  if ! version=$(extract_version "$manifest"); then
    printf '::error::no unindented `version` line in %s\n' "$manifest" >&2
    return 1
  fi
  assert_version_shape "$version" || return 1
  assert_tag_matches_manifest \
    "$version" "${GITHUB_REF_NAME:-}" "${GITHUB_REF_TYPE:-}" || return 1
  for crate in "$@"; do
    status=$(crates_io_probe "$crate" "$version")
    if ! assert_status_conclusive "$crate" "$version" "$status"; then
      unknown=$((unknown + 1))
    fi
    probes="$probes$crate"$'\t'"$status"$'\n'
  done
  if [ "$unknown" -ne 0 ]; then
    printf '::error::%d of %d crates.io probes for %s were inconclusive — refusing to decide what to publish. This used to be a silent skip of every crate publish with the run still green.\n' \
      "$unknown" "$#" "$version" >&2
    return 1
  fi
  gh_output version "$version"
  while IFS=$'\t' read -r crate status; do
    [ -n "$crate" ] || continue
    decision=$(publish_decision_for "$status")
    key=$(crate_output_key "$crate")
    if [ "$decision" = "true" ]; then
      printf '%s %s not on crates.io (HTTP %s) — will publish\n' \
        "$crate" "$version" "$status" >&2
    else
      printf '%s %s already on crates.io (HTTP %s) — skipping\n' \
        "$crate" "$version" "$status" >&2
    fi
    gh_output "publish_$key" "$decision"
  done <<EOF
$probes
EOF
}

# ---------------------------------------------------------------------------
# Changelog
# ---------------------------------------------------------------------------

# COMPUTE. Print the body of the `## [<version>]` section of a changelog —
# everything after that heading, up to the next `## [` heading. Empty output
# means there is no such section. Faithful port of release.yml's awk.
extract_changelog_section() {
  local version="$1" changelog="$2"
  awk "/^## \[${version}\]/{found=1; next} /^## \[/{if(found) exit} found" "$changelog"
}

# EMIT / ASSERT. The whole `Extract changelog for this version` step: set
# `has_notes`, and when true write the section body to <notes_path> for the
# release action's `body_path`.
#
#   changelog_notes <version> <changelog> <notes_path> [ref_type]
#
# WHY A MISSING SECTION IS FATAL ON A TAG. It used to set `has_notes=false`,
# which the workflow read as "fall back to auto-generated notes" — a silent
# downgrade from the curated changelog to a list of commit subjects, on the one
# artifact users actually read. Worse, it is a SYMPTOM of two real faults it was
# masking: the release skill's "promote the CHANGELOG" step having been skipped,
# and tag/manifest skew (tag v0.1.4 against a changelog that only knows 0.1.3).
# Degrading quietly turned the honour-system step into something nothing
# checked. On a tag build the section is now required, which is what makes that
# step a gate.
#
# The `ref_type` argument (GitHub's `GITHUB_REF_TYPE`) is what scopes the
# requirement. It is the same key `assert_tag_matches_manifest` uses, so a
# non-tag invocation — a local run, or a workflow trigger that should never have
# reached here — degrades rather than failing on a version it had no business
# looking up. Absent, it defaults to empty, i.e. not a tag.
#
#   rc 0     — `has_notes` written (true, or false off a tag build).
#   rc 1     — no `## [<version>]` section on a TAG build; NOTHING is written.
#   rc != 0  — the changelog file could not be read at all (set -e propagates
#              awk's failure; it must not silently degrade to has_notes=false).
changelog_notes() {
  local version="$1" changelog="$2" notes_path="$3" ref_type="${4:-}" notes
  notes=$(extract_changelog_section "$version" "$changelog")
  if [ -z "$notes" ]; then
    if [ "$ref_type" = "tag" ]; then
      printf '::error::%s has no `## [%s]` section, but this is a tag build of %s. Refusing to fall back to auto-generated release notes: a missing section means the CHANGELOG was never promoted for this release, or the tag disagrees with the version being released. Add the section and re-tag.\n' \
        "$changelog" "$version" "'${version}'" >&2
      return 1
    fi
    printf 'no `## [%s]` section in %s and ref_type is %s, not a tag — notes will be auto-generated\n' \
      "$version" "$changelog" "'${ref_type:-unset}'" >&2
    gh_output has_notes false
  else
    gh_output has_notes true
    printf '%s\n' "$notes" > "$notes_path"
  fi
}

# ---------------------------------------------------------------------------
# Artifact set (the wheels + sdist that go to PyPI)
# ---------------------------------------------------------------------------
#
# WHAT THIS REPLACES. `publish-pypi` used to "verify" its downloaded artifacts
# with `ls -la dist` — verification-shaped, asserting nothing, on a directory
# `download-artifact` always creates. Combined with `if-no-files-found`'s
# default of `warn` on the two upload steps and `skip-existing: true` on the
# PyPI publish, a build leg that exited 0 without producing a wheel would upload
# an empty artifact, and a PARTIAL wheel set would ship with the whole run
# green. The release flow verifies the *version* on PyPI, never the artifact
# *set*, so nothing downstream catches it either.
#
# WHY THE EXPECTATION IS DERIVED, NOT WRITTEN DOWN. "assert 6 files" is a dead
# gate the day someone adds or drops a matrix leg: the constant is now wrong and
# nothing says so. The expected set is therefore read out of the build-wheels
# matrix in release.yml itself — the same definition that decides how many
# wheels are built — and each leg is mapped to the platform tag its wheel must
# carry. Adding a leg cannot leave this passing on a short set:
#   * new leg, known os/target -> a new expected wheel; a short set fails;
#   * new leg, unknown os/target -> `wheel_pattern_for_leg` has no mapping and
#     hard-fails, demanding one (it never falls back to "anything goes");
#   * new leg whose tag duplicates an existing leg's -> the bijection below
#     leaves the second leg with nothing to claim, so it fails;
#   * a wheel nobody expected -> left unclaimed, which also fails.

# COMPUTE. Print one `os<TAB>target` line per leg of the `build-wheels` job's
# `strategy.matrix.include` list, in file order.
#
# Scoped to that one job on purpose: `release-binaries` has its own matrix whose
# legs carry `suffix`, not `target`. A leg missing `os` or `target` is an
# ::error::, not a skip — an unparseable matrix must not silently shrink the
# expected set (that is exactly the "gate passes on a short set" failure).
#
#   rc 0 — at least one well-formed leg, printed to stdout.
#   rc 1 — the file could not be read, no legs were found, or a leg lacked
#          os/target; an ::error:: annotation is on stderr.
#
# The awk below is a PURE EXTRACTOR — it prints `os<TAB>target` per leg and
# judges nothing, leaving empty fields where a key was absent. All the verdicts
# are taken in shell afterwards. That is not style: awk's only way to signal
# failure is `exit`, and this file's rule (enforced by
# `test_no_function_calls_exit`) is that nothing here may `exit`, because an
# `exit` inside a `$( )` is swallowed by the enclosing command. Keeping the
# judgement in shell keeps the guard absolute instead of carving an exception
# into it.
wheel_matrix_legs() {
  local workflow="${1:-.github/workflows/release.yml}" raw os target out="" n=0
  raw=$(awk '
    function flush(   ) {
      if (started) printf("%s\t%s\n", os, target)
      started = 0; os = ""; target = ""
    }
    # A job header is the only key at indent 2 with no value.
    /^  [A-Za-z0-9_-]+:[ \t]*$/ {
      if (injob) { flush(); injob = 0; inmatrix = 0 }
      if ($0 ~ /^  build-wheels:[ \t]*$/) injob = 1
      next
    }
    injob == 0 { next }
    /^[ \t]*include:[ \t]*$/ { inmatrix = 1; next }
    /^[ \t]*steps:[ \t]*$/   { flush(); inmatrix = 0; next }
    inmatrix == 0 { next }
    {
      line = $0
      sub(/^[ \t]+/, "", line)
      if (line == "") next
      if (substr(line, 1, 1) == "-") {
        flush()
        started = 1
        line = substr(line, 2)
        sub(/^[ \t]+/, "", line)
        if (line == "") next
      }
      idx = index(line, ":")
      if (idx == 0) next
      k = substr(line, 1, idx - 1)
      v = substr(line, idx + 1)
      gsub(/^[ \t]+|[ \t]+$/, "", v)
      gsub(/^['"'"'"]|['"'"'"]$/, "", v)
      if (k == "os") os = v
      else if (k == "target") target = v
    }
    END { flush() }
  ' "$workflow") || return 1

  if [ -z "$raw" ]; then
    printf '::error::no build-wheels matrix legs found in %s — the wheel matrix moved or was restructured, so the expected artifact set is unknown. Refusing to derive an EMPTY expectation (it would make the artifact-set gate vacuously true).\n' \
      "'${workflow}'" >&2
    return 1
  fi
  while IFS=$'\t' read -r os target; do
    n=$((n + 1))
    if [ -z "${os:-}" ] || [ -z "${target:-}" ]; then
      printf '::error::build-wheels matrix leg %d in %s declares os=%s target=%s — a half-declared leg must not silently shrink the expected wheel set\n' \
        "$n" "'${workflow}'" "'${os:-}'" "'${target:-}'" >&2
      return 1
    fi
    out="$out$os"$'\t'"$target"$'\n'
  done <<EOF
$raw
EOF
  printf '%s' "$out"
}

# COMPUTE. Print the filename glob the wheel for one matrix leg must match.
#
# THE MAPPING IS DELIBERATELY EXHAUSTIVE-BY-FAILURE. Only the os/target pairs
# the matrix actually uses are listed; anything else is an error, not a
# permissive default. That is what makes "add a matrix leg" impossible to get
# silently wrong: the new leg has no mapping, the gate fails, and the author has
# to state what tag the new wheel carries. A catch-all `*) printf '*.whl'` here
# would re-open the exact hole this whole function exists to close.
#
#   rc 0 — glob on stdout.
#   rc 1 — unmapped leg; an ::error:: annotation is on stderr.
wheel_pattern_for_leg() {
  local os="${1:-}" target="${2:-}"
  case "$os:$target" in
    ubuntu-*:x86_64)  printf '*manylinux*_x86_64.whl\n' ;;
    ubuntu-*:aarch64) printf '*manylinux*_aarch64.whl\n' ;;
    macos-*:aarch64)  printf '*macosx_*_arm64.whl\n' ;;
    macos-*:x86_64)   printf '*macosx_*_x86_64.whl\n' ;;
    windows-*:x64)    printf '*win_amd64.whl\n' ;;
    *)
      printf '::error::build-wheels matrix leg %s/%s has no wheel-tag mapping in wheel_pattern_for_leg (scripts/release_gates.sh). Add one — a new matrix leg must not be able to pass this gate unnoticed.\n' \
        "'${os}'" "'${target}'" >&2
      return 1
      ;;
  esac
}

# COMPUTE. Number of non-empty lines in a newline-separated list.
count_lines() {
  local list="${1:-}"
  if [ -z "$list" ]; then
    printf '0\n'
  else
    printf '%s\n' "$list" | grep -c '[^[:space:]]'
  fi
}

# ASSERT. The downloaded artifact set is exactly what the matrix promises.
#
#   assert_artifact_set <dist_dir> [workflow]
#
# It is a BIJECTION, not a count: every matrix leg claims exactly one wheel, and
# every wheel is claimed by exactly one leg. A count alone would pass a set that
# has the right number of wrong wheels (two manylinux builds and no macOS one),
# and a per-leg check alone would pass a set with an extra unexpected wheel in
# it. Plus exactly one sdist.
#
#   rc 0 — complete set; a per-leg trace is on stderr.
#   rc 1 — missing wheel, unexpected wheel, ambiguous leg mapping, wrong sdist
#          count, or an unreadable matrix; ::error:: annotations on stderr.
assert_artifact_set() {
  local dist="${1:-dist}" workflow="${2:-.github/workflows/release.yml}"
  local legs wheels sdists remaining rest pattern os target w matched
  local n_matched failures=0 n_leg=0 leftover n_wheel n_sdist

  if [ ! -d "$dist" ]; then
    printf '::error::artifact directory %s does not exist — the download step produced nothing\n' \
      "'${dist}'" >&2
    return 1
  fi
  if ! legs=$(wheel_matrix_legs "$workflow"); then
    printf '::error::refusing to publish: the expected wheel set could not be derived from %s\n' \
      "'${workflow}'" >&2
    return 1
  fi

  wheels=$(find "$dist" -maxdepth 1 -type f -name '*.whl' -exec basename {} \; | LC_ALL=C sort)
  sdists=$(find "$dist" -maxdepth 1 -type f -name '*.tar.gz' -exec basename {} \; | LC_ALL=C sort)
  n_wheel=$(count_lines "$wheels")
  n_sdist=$(count_lines "$sdists")
  remaining="$wheels"

  while IFS=$'\t' read -r os target; do
    [ -n "$os" ] || continue
    n_leg=$((n_leg + 1))
    if ! pattern=$(wheel_pattern_for_leg "$os" "$target"); then
      failures=$((failures + 1))
      continue
    fi
    n_matched=0
    matched=""
    rest=""
    while IFS= read -r w; do
      [ -n "$w" ] || continue
      case "$w" in
        $pattern)
          n_matched=$((n_matched + 1))
          matched="$w"
          ;;
        *)
          rest="$rest$w"$'\n'
          ;;
      esac
    done <<EOF
$remaining
EOF
    if [ "$n_matched" -eq 1 ]; then
      printf 'leg %s/%s -> %s\n' "$os" "$target" "$matched" >&2
      remaining="$rest"
    elif [ "$n_matched" -eq 0 ]; then
      printf '::error::no wheel for build-wheels matrix leg %s/%s — nothing in %s matches %s. A build leg produced no wheel and `if-no-files-found` let the empty artifact through.\n' \
        "$os" "$target" "'${dist}'" "'${pattern}'" >&2
      failures=$((failures + 1))
    else
      printf '::error::%d wheels match build-wheels matrix leg %s/%s (%s) — two legs share one tag mapping, so the set cannot be checked leg-by-leg. Fix wheel_pattern_for_leg.\n' \
        "$n_matched" "$os" "$target" "'${pattern}'" >&2
      failures=$((failures + 1))
    fi
  done <<EOF
$legs
EOF

  leftover=$(count_lines "$remaining")
  if [ "$leftover" -ne 0 ]; then
    printf '::error::%d wheel(s) in %s belong to no build-wheels matrix leg: %s\n' \
      "$leftover" "'${dist}'" "$(printf '%s' "$remaining" | tr '\n' ' ')" >&2
    failures=$((failures + 1))
  fi
  if [ "$n_sdist" -ne 1 ]; then
    printf '::error::expected exactly 1 sdist (*.tar.gz) in %s, found %d\n' \
      "'${dist}'" "$n_sdist" >&2
    failures=$((failures + 1))
  fi

  if [ "$failures" -ne 0 ]; then
    printf '::error::artifact set incomplete: %d problem(s) across %d wheel(s) + %d sdist(s) for %d matrix leg(s). Refusing to publish — `skip-existing: true` would ship the partial set to PyPI silently.\n' \
      "$failures" "$n_wheel" "$n_sdist" "$n_leg" >&2
    return 1
  fi
  printf 'artifact set complete: %d wheel(s), one per build-wheels matrix leg, + %d sdist\n' \
    "$n_wheel" "$n_sdist" >&2
}

# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------

# Allowlisted so a typo (or an injected argument) cannot run an arbitrary
# command through this entry point.
main() {
  local cmd="${1:-}"
  if [ -z "$cmd" ]; then
    printf 'usage: release_gates.sh <function> [args...]\n' >&2
    return 2
  fi
  shift
  case "$cmd" in
    extract_version | assert_version_shape | version_from_ref | \
      assert_tag_matches_manifest | assert_publish_ref | crates_io_status | \
      crates_io_probe | assert_status_conclusive | \
      publish_decision_for | crate_output_key | check_crates_to_publish | \
      extract_changelog_section | changelog_notes | wheel_matrix_legs | \
      wheel_pattern_for_leg | assert_artifact_set)
      "$cmd" "$@"
      ;;
    *)
      printf '::error::unknown release_gates.sh command: %s\n' "$cmd" >&2
      return 2
      ;;
  esac
}

if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
  main "$@"
fi
