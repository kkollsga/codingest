"""Unit tests for `scripts/release_gates.sh` — the extracted release-path gates.

WHY THIS SUITE EXISTS
---------------------
`.github/workflows/release.yml` runs only on a `v*` tag push, so its logic can
never be exercised by branch CI and "break it and watch it go red" is impossible
to satisfy in place. The logic was therefore lifted into
`scripts/release_gates.sh`, and this suite drives every function through BOTH
its pass path and its fail path on every push (see the `release-gates` job in
`.github/workflows/ci.yml`).

Harness: pytest, already pinned and used by this repo (`tests/python`,
`ci.yml`), plus the `scripts/verify_wheel.py` precedent of Python for release
tooling. No new dependency, no bats install step. Tests shell out to bash and
source the real script, so what is under test is the shipped shell — not a
re-implementation. Nothing here touches the network: `crates_io_status`
indirects curl through `CODINGEST_RELEASE_CURL`, which the tests stub.

THREE WAYS A GATE IS BORN DEAD — each is guarded here:
  1. `exit` inside `$( )` — killed only the subshell, caller read empty as 0.
     Guarded structurally by `test_no_function_calls_exit`.
  2. Substring subsumption — `assert "cmd" in text` also matches
     `cmd --self-test`, so deleting the real invocation stays green. Every
     workflow-wiring assertion compares WHOLE STRIPPED LINES.
  3. Comment subsumption — the words asserted on usually also appear in the
     comment explaining them. `_code_lines()` strips comment lines first, and
     `test_code_lines_strips_comments` proves the stripper actually strips.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts" / "release_gates.sh"
RELEASE_WORKFLOW = REPO / ".github" / "workflows" / "release.yml"
CI_WORKFLOW = REPO / ".github" / "workflows" / "ci.yml"

UA = "codingest-ci/0.1.0 (https://github.com/kkollsga/codingest)"


# --------------------------------------------------------------------------
# helpers
# --------------------------------------------------------------------------


def run_shell(body: str, *, env: dict[str, str] | None = None, cwd: Path | None = None):
    """Source the real script in a fresh bash and run `body` against it."""
    full = f'set -euo pipefail\nsource "{SCRIPT}"\n{body}\n'
    return subprocess.run(
        ["bash", "-c", full],
        capture_output=True,
        text=True,
        cwd=str(cwd or REPO),
        env={"PATH": "/usr/bin:/bin:/usr/sbin:/sbin", **(env or {})},
    )


def call(args: list[str], *, env: dict[str, str] | None = None, cwd: Path | None = None):
    """Invoke the script through its command-line dispatch, as a workflow does."""
    return subprocess.run(
        [str(SCRIPT), *args],
        capture_output=True,
        text=True,
        cwd=str(cwd or REPO),
        env={"PATH": "/usr/bin:/bin:/usr/sbin:/sbin", **(env or {})},
    )


def stub_curl(tmp_path: Path, statuses: dict[str, str], *, rc: int = 0) -> Path:
    """An offline stand-in for curl: maps crate name -> HTTP status string.

    Also records its full argv to `<tmp_path>/curl_argv` so a test can assert the
    User-Agent and URL that the real curl would have received.
    """
    arms = "\n".join(
        f'    */api/v1/crates/{crate}/*) printf "%s" "{status}" ;;'
        for crate, status in statuses.items()
    )
    stub = tmp_path / "curl"
    stub.write_text(
        "#!/usr/bin/env bash\n"
        f'printf "%s\\n" "$*" >> "{tmp_path}/curl_argv"\n'
        'url=""\n'
        'for a in "$@"; do url="$a"; done\n'
        "case \"$url\" in\n"
        f"{arms}\n"
        '    *) printf "%s" "000" ;;\n'
        "esac\n"
        f"exit {rc}\n"
    )
    stub.chmod(0o755)
    return stub


def _code_lines(path: Path) -> list[str]:
    """Stripped, non-blank, non-comment lines of a file.

    Comment stripping is what stops COMMENT SUBSUMPTION: the prose explaining a
    command almost always quotes the command, so an assertion made against the
    raw text stays green after the real invocation is deleted.
    """
    out = []
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        out.append(line)
    return out


def assert_has_line(path: Path, expected: str) -> None:
    """Assert `expected` is present as a WHOLE stripped code line.

    Whole-line equality is what stops SUBSTRING SUBSUMPTION: `"foo" in text`
    also matches `foo --dry-run`, so a neutered invocation would keep the
    assertion green.
    """
    lines = _code_lines(path)
    assert expected in lines, (
        f"{path.name}: expected exact code line not found: {expected!r}"
    )


# --------------------------------------------------------------------------
# extract_version  (release.yml:48, ported)
# --------------------------------------------------------------------------


def test_extract_version_pass(tmp_path: Path):
    manifest = tmp_path / "Cargo.toml"
    manifest.write_text(
        '[workspace.package]\nversion = "0.1.3"\nedition = "2021"\n'
        '[workspace.dependencies]\nkglite = { version = "0.15.3" }\n'
    )
    res = call(["extract_version", str(manifest)])
    assert res.returncode == 0, res.stderr
    assert res.stdout == "0.1.3\n"


def test_extract_version_takes_first_unindented_version_only(tmp_path: Path):
    """`grep -m 1 '^version'` — an indented `version` key must not win."""
    manifest = tmp_path / "Cargo.toml"
    manifest.write_text(
        '[package]\n    version = "9.9.9"\nname = "x"\nversion = "0.2.0"\n'
    )
    res = call(["extract_version", str(manifest)])
    assert res.returncode == 0, res.stderr
    assert res.stdout == "0.2.0\n"


def test_extract_version_fail_path_no_version_line(tmp_path: Path):
    """FAIL PATH: no `^version` line at all must be rc 1, not a silent empty.

    This is the masked-pipeline bug: as `grep ... | cut ...` the step reported
    `cut`'s status, which is 0 even when grep matched nothing.
    """
    manifest = tmp_path / "Cargo.toml"
    manifest.write_text('[package]\nname = "x"\n')
    res = call(["extract_version", str(manifest)])
    assert res.returncode == 1, f"expected rc 1, got {res.returncode}: {res.stdout!r}"


def test_extract_version_fail_path_missing_manifest(tmp_path: Path):
    res = call(["extract_version", str(tmp_path / "nope.toml")])
    assert res.returncode != 0


def test_extract_version_reads_the_real_root_manifest():
    """The value the workflow will actually publish under."""
    res = call(["extract_version", "Cargo.toml"])
    assert res.returncode == 0, res.stderr
    assert res.stdout.strip(), "root Cargo.toml yielded an empty version"


# --------------------------------------------------------------------------
# assert_version_shape  (Phase 3 gate 1)
# --------------------------------------------------------------------------
#
# `extract_version` used to return whatever it parsed. The original
# `grep ... | cut ...` reported CUT's status, always 0, so an empty version
# passed silently. What that actually breaks is re-run idempotency, NOT the
# release: crates.io answers 404 for an empty version segment and 404 is the
# publish signal, so an empty version means "publish everything" and a retry
# after a partial failure hard-errors "crate version already uploaded".


@pytest.mark.parametrize("version", ["0.1.3", "1.0.0", "10.20.30", "1.0.0-rc.1"])
def test_assert_version_shape_accepts_a_version(version: str):
    res = call(["assert_version_shape", version])
    assert res.returncode == 0, res.stderr


@pytest.mark.parametrize(
    "version",
    [
        "",  # the masked-pipeline outcome
        "abc",
        "1.2",  # too few segments
        "1",
        "v0.1.3",  # a tag, not a manifest version
        "version.workspace = true",  # what `cut` prints for an unquoted key
        ".1.3",
        " 0.1.3",
    ],
)
def test_assert_version_shape_fail_path(version: str):
    """FAIL PATH: anything not matching ^N.N.N must be rc 1 with ::error::."""
    res = call(["assert_version_shape", version])
    assert res.returncode == 1, f"{version!r} was accepted as a version"
    assert "::error::" in res.stderr


def test_assert_version_shape_fail_path_with_no_argument():
    """A caller that forgets the argument must fail, not pass vacuously."""
    res = call(["assert_version_shape"])
    assert res.returncode == 1
    assert "::error::" in res.stderr


def test_assert_version_shape_accepts_the_real_root_manifest():
    version = call(["extract_version", "Cargo.toml"]).stdout.strip()
    assert call(["assert_version_shape", version]).returncode == 0


# --------------------------------------------------------------------------
# version_from_ref  (release.yml:213, ported)
# --------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("ref", "expected"),
    [("v0.1.3", "0.1.3"), ("0.1.3", "0.1.3"), ("v1.0.0-rc.1", "1.0.0-rc.1")],
)
def test_version_from_ref(ref: str, expected: str):
    res = call(["version_from_ref", ref])
    assert res.returncode == 0, res.stderr
    assert res.stdout == expected + "\n"


# --------------------------------------------------------------------------
# assert_tag_matches_manifest  (Phase 3 gate 2 — the highest-value gate)
# --------------------------------------------------------------------------
#
# THE SILENT NO-OP RELEASE. Tag v0.1.4 with the manifest at 0.1.3: the 404 probe
# asks about 0.1.3, gets 200, every crate publish skips; the wheels build at
# 0.1.3 and skip-existing swallows them on PyPI; there is no `## [0.1.4]`
# section so the notes degrade to auto-generated; and a GitHub Release v0.1.4 is
# cut with 0.1.3 artifacts. Green throughout. Nothing checked this before.


def test_assert_tag_matches_manifest_pass():
    res = call(["assert_tag_matches_manifest", "0.1.3", "v0.1.3", "tag"])
    assert res.returncode == 0, res.stderr


def test_assert_tag_matches_manifest_pass_without_the_v_prefix():
    res = call(["assert_tag_matches_manifest", "0.1.3", "0.1.3", "tag"])
    assert res.returncode == 0, res.stderr


@pytest.mark.parametrize(
    ("version", "ref"),
    [
        ("0.1.3", "v0.1.4"),  # the plan's exact scenario
        ("0.1.4", "v0.1.3"),  # manifest ahead of the tag
        ("0.1.3", "v0.1.30"),  # near-miss: one is a prefix of the other
        ("0.1.3", "v1.0.0"),
        ("0.1.3", "vv0.1.3"),  # only ONE leading v is stripped
        ("0.1.3", ""),
    ],
)
def test_assert_tag_matches_manifest_fail_path_skew(version: str, ref: str):
    """FAIL PATH: any tag/manifest disagreement on a tag build is rc 1."""
    res = call(["assert_tag_matches_manifest", version, ref, "tag"])
    assert res.returncode == 1, f"skew {version} vs {ref} was accepted"
    assert "::error::" in res.stderr


@pytest.mark.parametrize("ref_type", ["branch", "", "unknown"])
def test_assert_tag_matches_manifest_does_not_fire_off_a_tag(ref_type: str):
    """workflow_dispatch: GITHUB_REF_NAME is a BRANCH (`main`), so
    `${GITHUB_REF_NAME#v}` would compare the manifest against the string
    `main`. The gate must not invent that comparison — it reports the skip on
    stderr and returns 0."""
    res = call(["assert_tag_matches_manifest", "0.1.3", "main", ref_type])
    assert res.returncode == 0, res.stderr
    assert "not a tag" in res.stderr, "a skipped gate must say so, not pass mutely"


def test_assert_tag_matches_manifest_the_dispatch_skip_is_narrow():
    """The skip is keyed on ref_type ONLY — a real tag still gets compared even
    when its name looks nothing like a branch."""
    res = call(["assert_tag_matches_manifest", "0.1.3", "main", "tag"])
    assert res.returncode == 1
    assert "::error::" in res.stderr


# --------------------------------------------------------------------------
# publish_decision_for  (release.yml:58-64, ported)
# --------------------------------------------------------------------------


def test_publish_decision_404_is_the_only_publish_signal():
    res = call(["publish_decision_for", "404"])
    assert res.returncode == 0, res.stderr
    assert res.stdout == "true\n"


@pytest.mark.parametrize("status", ["200", "403", "500", "000", ""])
def test_publish_decision_everything_else_skips(status: str):
    res = call(["publish_decision_for", status])
    assert res.returncode == 0, res.stderr
    assert res.stdout == "false\n", f"HTTP {status!r} must not be a publish signal"


def test_crate_output_key_dashes_become_underscores():
    res = call(["crate_output_key", "codingest-cli"])
    assert res.stdout == "codingest_cli\n"


# --------------------------------------------------------------------------
# crates_io_status  (release.yml:55-56, ported) — offline via a stubbed curl
# --------------------------------------------------------------------------


def test_crates_io_status_sends_the_ua_and_the_right_url(tmp_path: Path):
    stub = stub_curl(tmp_path, {"codingest": "404"})
    res = call(
        ["crates_io_status", "codingest", "0.1.3"],
        env={"CODINGEST_RELEASE_CURL": str(stub)},
    )
    assert res.returncode == 0, res.stderr
    assert res.stdout == "404"
    argv = (tmp_path / "curl_argv").read_text()
    assert UA in argv, f"crates.io policy requires a contact UA; got: {argv!r}"
    assert "https://crates.io/api/v1/crates/codingest/0.1.3" in argv


def test_crates_io_status_fail_path_curl_error(tmp_path: Path):
    """FAIL PATH: a non-zero curl must not be swallowed into a bare status."""
    stub = stub_curl(tmp_path, {"codingest": "000"}, rc=7)
    res = call(
        ["crates_io_status", "codingest", "0.1.3"],
        env={"CODINGEST_RELEASE_CURL": str(stub)},
    )
    assert res.returncode != 0


# --------------------------------------------------------------------------
# check_crates_to_publish — the whole step, incl. the $GITHUB_OUTPUT contract
# --------------------------------------------------------------------------

CRATES = ["codingest", "codingest-cli", "codingest-mcp"]


def _run_check(
    tmp_path: Path,
    version: str,
    statuses: dict[str, str],
    *,
    manifest_body: str | None = None,
    ref: dict[str, str] | None = None,
):
    manifest = tmp_path / "Cargo.toml"
    manifest.write_text(
        manifest_body
        if manifest_body is not None
        else f'[workspace.package]\nversion = "{version}"\n'
    )
    stub = stub_curl(tmp_path, statuses)
    out_file = tmp_path / "github_output"
    out_file.write_text("")
    res = call(
        ["check_crates_to_publish", str(manifest), *CRATES],
        env={
            "CODINGEST_RELEASE_CURL": str(stub),
            "GITHUB_OUTPUT": str(out_file),
            **(ref or {}),
        },
    )
    return res, out_file


def test_check_crates_all_absent_publishes_everything(tmp_path: Path):
    res, out_file = _run_check(
        tmp_path, "0.1.4", {c: "404" for c in CRATES}
    )
    assert res.returncode == 0, res.stderr
    assert out_file.read_text().splitlines() == [
        "version=0.1.4",
        "publish_codingest=true",
        "publish_codingest_cli=true",
        "publish_codingest_mcp=true",
    ]


def test_check_crates_mixed_statuses(tmp_path: Path):
    """200 = already there, 500 = unverifiable; both must yield false."""
    res, out_file = _run_check(
        tmp_path,
        "0.1.3",
        {"codingest": "200", "codingest-cli": "404", "codingest-mcp": "500"},
    )
    assert res.returncode == 0, res.stderr
    assert out_file.read_text().splitlines() == [
        "version=0.1.3",
        "publish_codingest=false",
        "publish_codingest_cli=true",
        "publish_codingest_mcp=false",
    ]


def test_check_crates_fail_path_no_version_line(tmp_path: Path):
    """FAIL PATH: an unreadable version must be rc 1 with NO outputs written."""
    manifest = tmp_path / "Cargo.toml"
    manifest.write_text('[workspace.package]\nname = "x"\n')
    stub = stub_curl(tmp_path, {c: "404" for c in CRATES})
    out_file = tmp_path / "github_output"
    out_file.write_text("")
    res = call(
        ["check_crates_to_publish", str(manifest), *CRATES],
        env={"CODINGEST_RELEASE_CURL": str(stub), "GITHUB_OUTPUT": str(out_file)},
    )
    assert res.returncode == 1, f"expected rc 1, got {res.returncode}"
    assert "::error::" in res.stderr
    assert out_file.read_text() == "", "no outputs may be written on the fail path"


@pytest.mark.parametrize(
    ("label", "body"),
    [
        ("empty", '[workspace.package]\nversion = ""\n'),
        ("unquoted", "[workspace.package]\nversion.workspace = true\n"),
        ("malformed", '[workspace.package]\nversion = "abc"\n'),
        ("two-segment", '[workspace.package]\nversion = "1.2"\n'),
    ],
)
def test_check_crates_fail_path_bad_version_never_reaches_github_output(
    tmp_path: Path, label: str, body: str
):
    """FAIL PATH (gate 1): a version that does not look like one stops the step.

    An empty version is the masked-pipeline outcome. It does NOT silently skip
    the release — crates.io 404s on an empty version segment, so it would
    publish everything and destroy re-run idempotency.
    """
    res, out_file = _run_check(
        tmp_path, "", {c: "404" for c in CRATES}, manifest_body=body
    )
    assert res.returncode == 1, f"{label}: expected rc 1, got {res.returncode}"
    assert "::error::" in res.stderr
    assert out_file.read_text() == "", f"{label}: outputs written on the fail path"
    assert not (tmp_path / "curl_argv").exists(), (
        f"{label}: crates.io was probed with a bad version"
    )


def test_check_crates_fail_path_tag_manifest_skew(tmp_path: Path):
    """FAIL PATH (gate 2): tag v0.1.4 with the manifest at 0.1.3.

    Without this the run goes green end to end and publishes nothing.
    """
    res, out_file = _run_check(
        tmp_path,
        "0.1.3",
        {c: "200" for c in CRATES},
        ref={"GITHUB_REF_NAME": "v0.1.4", "GITHUB_REF_TYPE": "tag"},
    )
    assert res.returncode == 1, f"expected rc 1, got {res.returncode}"
    assert "::error::" in res.stderr
    assert out_file.read_text() == "", "no outputs may be written on the fail path"
    assert not (tmp_path / "curl_argv").exists(), "crates.io probed despite skew"


def test_check_crates_pass_path_matching_tag(tmp_path: Path):
    """GREEN: tag and manifest agree — the step behaves exactly as before."""
    res, out_file = _run_check(
        tmp_path,
        "0.1.4",
        {c: "404" for c in CRATES},
        ref={"GITHUB_REF_NAME": "v0.1.4", "GITHUB_REF_TYPE": "tag"},
    )
    assert res.returncode == 0, res.stderr
    assert out_file.read_text().splitlines() == [
        "version=0.1.4",
        "publish_codingest=true",
        "publish_codingest_cli=true",
        "publish_codingest_mcp=true",
    ]


def test_check_crates_workflow_dispatch_does_not_compare_a_branch(tmp_path: Path):
    """workflow_dispatch: GITHUB_REF_NAME is `main`. The skew gate must not fire
    (Phase 5 hardens the dispatch path itself); the version gate still does."""
    res, out_file = _run_check(
        tmp_path,
        "0.1.4",
        {c: "404" for c in CRATES},
        ref={"GITHUB_REF_NAME": "main", "GITHUB_REF_TYPE": "branch"},
    )
    assert res.returncode == 0, res.stderr
    assert out_file.read_text().splitlines()[0] == "version=0.1.4"


# --------------------------------------------------------------------------
# INVERTED CONTROLS — prove the strictness of each gate is load-bearing.
#
# Each runs a deliberately NAIVE version of the assertion against an input the
# real gate rejects, and asserts the naive one passes. If a naive check would
# have caught the same input, the real gate's strictness buys nothing.
# --------------------------------------------------------------------------


def test_inverted_control_a_non_empty_check_would_pass_a_malformed_version():
    """NAIVE: `[ -n "$version" ]` — the obvious fix for the masked pipeline.

    It accepts `abc`, `1.2` and `version.workspace = true`. The real gate does
    not, which is why the assertion is a shape match and not a presence check.
    """
    for bad in ("abc", "1.2", "version.workspace = true"):
        naive = run_shell(f'if [ -n "{bad}" ]; then echo NAIVE_PASS; else echo NAIVE_FAIL; fi')
        assert "NAIVE_PASS" in naive.stdout, f"premise broken for {bad!r}"
        assert call(["assert_version_shape", bad]).returncode == 1, (
            f"the real gate must reject {bad!r} where the naive check passes"
        )


def test_inverted_control_a_substring_tag_check_would_pass_a_near_miss():
    """NAIVE: `case "$ref" in *"$version"*)` — "the tag contains the version".

    Manifest 0.1.3 tagged v0.1.30 satisfies it; the real gate compares whole
    stripped values and fails. Proves the equality is doing the work.
    """
    naive = run_shell(
        'version=0.1.3; ref=v0.1.30\n'
        'case "$ref" in *"$version"*) echo NAIVE_PASS ;; *) echo NAIVE_FAIL ;; esac\n'
    )
    assert "NAIVE_PASS" in naive.stdout, "premise broken: the near-miss no longer matches"
    res = call(["assert_tag_matches_manifest", "0.1.3", "v0.1.30", "tag"])
    assert res.returncode == 1, "the real gate must reject the near-miss"


# --------------------------------------------------------------------------
# changelog  (release.yml:210-220, ported)
# --------------------------------------------------------------------------

CHANGELOG = """# Changelog

## [Unreleased]

- nothing yet

## [0.1.3] - 2026-07-20

### Added
- the AGC semantics

## [0.1.2] - 2026-07-01

- older stuff
"""


def _changelog(tmp_path: Path) -> Path:
    path = tmp_path / "CHANGELOG.md"
    path.write_text(CHANGELOG)
    return path


def test_extract_changelog_section_stops_at_the_next_heading(tmp_path: Path):
    res = call(["extract_changelog_section", "0.1.3", str(_changelog(tmp_path))])
    assert res.returncode == 0, res.stderr
    assert res.stdout == "\n### Added\n- the AGC semantics\n\n"
    assert "older stuff" not in res.stdout


def test_extract_changelog_section_fail_path_absent_version(tmp_path: Path):
    """FAIL PATH: a version with no section yields nothing (drives has_notes)."""
    res = call(["extract_changelog_section", "9.9.9", str(_changelog(tmp_path))])
    assert res.stdout == ""


def test_changelog_notes_pass_path(tmp_path: Path):
    out_file = tmp_path / "github_output"
    out_file.write_text("")
    notes = tmp_path / "release_notes.md"
    res = call(
        ["changelog_notes", "0.1.3", str(_changelog(tmp_path)), str(notes)],
        env={"GITHUB_OUTPUT": str(out_file)},
    )
    assert res.returncode == 0, res.stderr
    assert out_file.read_text().splitlines() == ["has_notes=true"]
    assert "the AGC semantics" in notes.read_text()


def test_changelog_notes_fail_path_missing_section(tmp_path: Path):
    """FAIL PATH: no section -> has_notes=false and no notes file written."""
    out_file = tmp_path / "github_output"
    out_file.write_text("")
    notes = tmp_path / "release_notes.md"
    res = call(
        ["changelog_notes", "9.9.9", str(_changelog(tmp_path)), str(notes)],
        env={"GITHUB_OUTPUT": str(out_file)},
    )
    assert res.returncode == 0, res.stderr
    assert out_file.read_text().splitlines() == ["has_notes=false"]
    assert not notes.exists()


def test_changelog_notes_fail_path_missing_changelog(tmp_path: Path):
    """FAIL PATH: an unreadable CHANGELOG must error, not degrade to false."""
    out_file = tmp_path / "github_output"
    out_file.write_text("")
    res = call(
        ["changelog_notes", "0.1.3", str(tmp_path / "nope.md"), str(tmp_path / "n.md")],
        env={"GITHUB_OUTPUT": str(out_file)},
    )
    assert res.returncode != 0
    assert "has_notes=false" not in out_file.read_text()


def test_real_changelog_has_a_section_for_the_real_version():
    """The two ported computations agreeing on the repo as it stands today."""
    version = call(["extract_version", "Cargo.toml"]).stdout.strip()
    res = call(["extract_changelog_section", version, "CHANGELOG.md"])
    assert res.stdout.strip(), f"CHANGELOG.md has no `## [{version}]` section"


# --------------------------------------------------------------------------
# dispatch
# --------------------------------------------------------------------------


def test_dispatch_rejects_an_unknown_command():
    res = call(["rm"])
    assert res.returncode == 2
    assert "::error::" in res.stderr


def test_dispatch_requires_a_command():
    res = call([])
    assert res.returncode == 2


# --------------------------------------------------------------------------
# BORN-DEAD GUARD 1: `exit` inside `$( )`
# --------------------------------------------------------------------------


def test_no_function_calls_exit():
    """No function in release_gates.sh may `exit`.

    `exit 1` inside a `$( )` substitution kills only the subshell; the enclosing
    command still succeeds, so the caller reads an empty value as a pass. This
    is the exact mistake that made upstream's own first fix vacuous. Functions
    must `return`, whose status a non-nested caller can see.
    """
    offenders = [
        line
        for line in _code_lines(SCRIPT)
        if line == "exit" or line.startswith("exit ") or " exit " in f" {line} "
    ]
    assert offenders == [], (
        "release_gates.sh must never `exit` — an exit inside $( ) is swallowed; "
        f"use `return`. Offending lines: {offenders}"
    )


def test_the_exit_probe_itself_detects_an_exit(tmp_path: Path):
    """VERIFY THE PROBE: the exit-detector must fire on a file that has one.

    An assertion that cannot fire is worth nothing; this proves the detector
    above would have caught a reintroduced `exit`.
    """
    bad = tmp_path / "bad.sh"
    bad.write_text("f() {\n  [ -n \"$1\" ] || exit 1\n  printf '%s' \"$1\"\n}\n")
    offenders = [
        line
        for line in _code_lines(bad)
        if line == "exit" or line.startswith("exit ") or " exit " in f" {line} "
    ]
    assert offenders, "the exit-detector failed to flag an obvious `exit 1`"


def test_the_exit_probe_ignores_exit_in_a_comment(tmp_path: Path):
    """VERIFY THE PROBE: a commented `exit 1` must NOT be flagged."""
    ok = tmp_path / "ok.sh"
    ok.write_text("# never write `|| exit 1` here\nf() {\n  return 1\n}\n")
    offenders = [
        line
        for line in _code_lines(ok)
        if line == "exit" or line.startswith("exit ") or " exit " in f" {line} "
    ]
    assert offenders == [], f"comment stripping failed: {offenders}"


def test_a_function_that_exits_would_be_swallowed_by_command_substitution():
    """Demonstrate the trap live, so the comment in the script is not folklore.

    `exit 1` inside `$( )` nested in another command leaves the outer command at
    rc 0 under `set -e`; `return`-based status checked as a whole command does
    not.
    """
    trap = run_shell(
        'bad() { exit 1; }\n'
        'echo "version=$(bad)" > /dev/null\n'
        'echo "OUTER_RC=$?"\n'
    )
    assert "OUTER_RC=0" in trap.stdout, (
        "premise broken: the trap this script guards against no longer exists"
    )

    correct = run_shell(
        'good() { return 1; }\n'
        'if good; then echo "OUTER_RC=0"; else echo "OUTER_RC=1"; fi\n'
    )
    assert "OUTER_RC=1" in correct.stdout


# --------------------------------------------------------------------------
# BORN-DEAD GUARD 2 + 3: the workflow wiring, matched on whole stripped
# comment-free lines
# --------------------------------------------------------------------------


def test_code_lines_strips_comments(tmp_path: Path):
    """VERIFY THE PROBE: `_code_lines` must drop comment lines.

    Without this, every wiring assertion below could be satisfied by the comment
    that explains it — the comment-subsumption failure mode.
    """
    sample = tmp_path / "s.yml"
    sample.write_text("# run: scripts/release_gates.sh check_crates_to_publish\nreal: line\n")
    assert _code_lines(sample) == ["real: line"]


def test_code_lines_does_not_do_substring_matching(tmp_path: Path):
    """VERIFY THE PROBE: a neutered invocation must NOT satisfy the assertion."""
    sample = tmp_path / "s.yml"
    sample.write_text("scripts/release_gates.sh check_crates_to_publish --dry-run\n")
    assert "scripts/release_gates.sh check_crates_to_publish" not in _code_lines(sample)


def test_release_workflow_calls_the_crate_check_script():
    assert_has_line(RELEASE_WORKFLOW, "scripts/release_gates.sh check_crates_to_publish \\")
    assert_has_line(RELEASE_WORKFLOW, "Cargo.toml codingest codingest-cli codingest-mcp")


def test_release_workflow_calls_the_changelog_script():
    assert_has_line(
        RELEASE_WORKFLOW,
        'version=$(scripts/release_gates.sh version_from_ref "$GITHUB_REF_NAME")',
    )
    assert_has_line(
        RELEASE_WORKFLOW,
        'scripts/release_gates.sh changelog_notes "$version" CHANGELOG.md /tmp/release_notes.md',
    )


def test_release_workflow_no_longer_inlines_the_extracted_logic():
    """The extraction is worthless if a copy of the old shell survives."""
    code = "\n".join(_code_lines(RELEASE_WORKFLOW))
    for stale in ("grep -m 1 '^version'", 'curl -s -o /dev/null', 'awk "/^## \\['):
        assert stale not in code, f"release.yml still inlines: {stale!r}"


def test_release_workflow_steps_declare_shell_bash():
    """A step without `shell: bash` loses `pipefail` — a whole class of masked
    failure. The sdist license check at release.yml:165-171 survives only via
    this line."""
    lines = _code_lines(RELEASE_WORKFLOW)
    assert lines.count("shell: bash") >= 3


def test_release_workflow_output_keys_are_unchanged():
    """The `$GITHUB_OUTPUT` keys the workflow's `if:` conditions read back.

    `check_crates_to_publish` / `changelog_notes` write these; a rename on either
    side silently turns every downstream `if:` false and skips the publish.
    """
    code = "\n".join(_code_lines(RELEASE_WORKFLOW))
    for key in (
        "steps.check.outputs.version",
        "steps.check.outputs.publish_codingest ==",
        "steps.check.outputs.publish_codingest_cli ==",
        "steps.check.outputs.publish_codingest_mcp ==",
        "steps.changelog.outputs.has_notes ==",
    ):
        assert key in code, f"release.yml no longer reads output {key!r}"


def test_release_gates_script_is_executable():
    """release.yml invokes the script directly, not via `bash <script>`.

    A lost exec bit (git mode 100644) would break every release-path step at tag
    time — the one moment nothing can be re-tested before it matters.
    """
    assert os.access(SCRIPT, os.X_OK), f"{SCRIPT} is not executable"


def test_ci_workflow_runs_this_suite():
    """Without this step the whole extraction is untested and the gates are back
    to being unfailable."""
    assert_has_line(CI_WORKFLOW, "run: python -m pytest tests/release -v")
