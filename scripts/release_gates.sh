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

# COMPUTE. Map a crates.io HTTP status to the publish decision, printing exactly
# `true` or `false`. HTTP 404 is the only publish signal: 200 means the version
# is already there, and anything else (403 rate-limit, 5xx, 000 no-response)
# means "can't tell -> skip safely".
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
# ORDER MATTERS. Both assertions run BEFORE the first `gh_output`, so a bad
# version can never reach `$GITHUB_OUTPUT` and no crates.io probe is even sent —
# the step fails on the value itself, not on something downstream of it.
#
#   rc 0 — outputs written.
#   rc 1 — no version line, a malformed version, or tag/manifest skew; NOTHING
#          is written.
check_crates_to_publish() {
  local manifest="${1:-Cargo.toml}"
  if [ $# -gt 0 ]; then shift; fi
  local version crate key status decision
  if ! version=$(extract_version "$manifest"); then
    printf '::error::no unindented `version` line in %s\n' "$manifest" >&2
    return 1
  fi
  assert_version_shape "$version" || return 1
  assert_tag_matches_manifest \
    "$version" "${GITHUB_REF_NAME:-}" "${GITHUB_REF_TYPE:-}" || return 1
  gh_output version "$version"
  for crate in "$@"; do
    status=$(crates_io_status "$crate" "$version")
    decision=$(publish_decision_for "$status")
    key=$(crate_output_key "$crate")
    if [ "$decision" = "true" ]; then
      printf '%s %s not on crates.io (HTTP %s) — will publish\n' \
        "$crate" "$version" "$status" >&2
    else
      printf '%s %s already on crates.io or unverifiable (HTTP %s) — skipping\n' \
        "$crate" "$version" "$status" >&2
    fi
    gh_output "publish_$key" "$decision"
  done
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

# EMIT. The whole `Extract changelog for this version` step: set `has_notes`,
# and when true write the section body to <notes_path> for the release action's
# `body_path`.
#
#   rc 0     — `has_notes` written (true or false).
#   rc != 0  — the changelog file could not be read at all (set -e propagates
#              awk's failure; it must not silently degrade to has_notes=false).
changelog_notes() {
  local version="$1" changelog="$2" notes_path="$3" notes
  notes=$(extract_changelog_section "$version" "$changelog")
  if [ -z "$notes" ]; then
    gh_output has_notes false
  else
    gh_output has_notes true
    printf '%s\n' "$notes" > "$notes_path"
  fi
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
      assert_tag_matches_manifest | crates_io_status | \
      publish_decision_for | crate_output_key | check_crates_to_publish | \
      extract_changelog_section | changelog_notes)
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
