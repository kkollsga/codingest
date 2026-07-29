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
#   rc 0 — a `^version` line was found. The printed value may still be wrong or
#          empty (e.g. an unquoted `version.workspace = true` line, which `cut`
#          passes through verbatim); validating it is a separate assertion.
#   rc 1 — no unindented `version` line in the manifest.
extract_version() {
  local manifest="$1" line
  line=$(grep -m 1 '^version' "$manifest") || return 1
  printf '%s\n' "$line" | cut -d '"' -f 2
}

# COMPUTE. Strip a leading `v` from a git ref name: `v0.1.3` -> `0.1.3`.
version_from_ref() {
  printf '%s\n' "${1#v}"
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
#   rc 0 — outputs written.
#   rc 1 — the manifest has no version line; nothing written.
check_crates_to_publish() {
  local manifest="${1:-Cargo.toml}"
  if [ $# -gt 0 ]; then shift; fi
  local version crate key status decision
  if ! version=$(extract_version "$manifest"); then
    printf '::error::no unindented `version` line in %s\n' "$manifest" >&2
    return 1
  fi
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
    extract_version | version_from_ref | crates_io_status | \
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
