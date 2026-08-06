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
import re
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


def stub_curl_sequence(tmp_path: Path, statuses: list[str]) -> Path:
    """A curl stub that answers a DIFFERENT status on each successive call.

    Needed to drive the probe's retry: a stub with one fixed answer cannot tell
    "retried and settled" apart from "never retried at all". The call counter
    lives in a file so it survives the subshells `$( )` puts each call in.
    """
    arms = "\n".join(f"  {i}) printf '%s' '{s}' ;;" for i, s in enumerate(statuses, 1))
    stub = tmp_path / "curl"
    stub.write_text(
        "#!/usr/bin/env bash\n"
        f'n=$(cat "{tmp_path}/curl_calls" 2>/dev/null || echo 0)\n'
        "n=$((n + 1))\n"
        f'printf "%s" "$n" > "{tmp_path}/curl_calls"\n'
        f'printf "%s\\n" "$*" >> "{tmp_path}/curl_argv"\n'
        "case \"$n\" in\n"
        f"{arms}\n"
        f"  *) printf '%s' '{statuses[-1]}' ;;\n"
        "esac\n"
    )
    stub.chmod(0o755)
    return stub


def curl_calls(tmp_path: Path) -> int:
    path = tmp_path / "curl_argv"
    return len(path.read_text().splitlines()) if path.exists() else 0


# A no-op stand-in for `sleep`, so retry tests cost nothing in wall time. It is
# indirected through CODINGEST_RELEASE_SLEEP for exactly this reason.
NO_SLEEP = {"CODINGEST_RELEASE_SLEEP": "/usr/bin/true"}


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


def _step_block(path: Path, step_name: str) -> list[str]:
    """The stripped, comment-free lines of one `- name: <step_name>` step.

    Scoping matters here: `lines.count("if-no-files-found: error") == 2` would be
    satisfied by putting BOTH copies on one upload step and none on the other.
    The keys must be checked inside the step that needs them.
    """
    lines = _code_lines(path)
    start = next((i for i, l in enumerate(lines) if l == f"- name: {step_name}"), None)
    assert start is not None, f"{path.name}: no step named {step_name!r}"
    block = [lines[start]]
    for line in lines[start + 1 :]:
        if line.startswith("- "):
            break
        block.append(line)
    return block


def _job_block(path: Path, job: str) -> list[str]:
    """The stripped, comment-free lines of one top-level job.

    Scoped like `_step_block`, and for the same reason: three jobs now carry an
    identically-named `Guard the publish ref` step, so a file-wide assertion
    would be satisfied by three copies in ONE job. Indentation has to be read
    from the RAW lines here — `_code_lines` strips it, which is exactly what
    makes a job header indistinguishable from any other key.
    """
    raw = path.read_text().splitlines()
    start = next((i for i, l in enumerate(raw) if l == f"  {job}:"), None)
    assert start is not None, f"{path.name}: no job named {job!r}"
    out = []
    for line in raw[start + 1 :]:
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if not line.startswith("    "):  # back out to indent 2 -> next job
            break
        out.append(stripped)
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
# crates_io_probe + assert_status_conclusive  (Phase 5 gate 1)
# --------------------------------------------------------------------------
#
# THE DEFECT. `curl -s -o /dev/null -w "%{http_code}"` carries no `-f`, so curl
# exits 0 for ANY outcome — including none, which it reports as "000". The
# decision was a two-way branch on `= 404`, so a DNS blip, a 403 from the rate
# limiter or a crates.io 5xx all fell into the else arm and skipped ALL THREE
# crate publishes with the run still green: a release that shipped nothing to
# crates.io was indistinguishable from one with nothing to ship.
#
# THE FIX SPLITS THE THREE CASES rather than bolting on `curl -f`. The
# skip-on-unknown was deliberate and is kept — `publish_decision_for` still
# refuses to publish on anything but a 404 — but uncertainty no longer exits
# green. 404 publishes, 200 skips cleanly (the documented idempotent re-run),
# anything else is retried a bounded number of times and then FAILS THE STEP.


def test_crates_io_probe_conclusive_on_the_first_try(tmp_path: Path):
    stub = stub_curl_sequence(tmp_path, ["404"])
    res = call(
        ["crates_io_probe", "codingest", "0.1.3"],
        env={"CODINGEST_RELEASE_CURL": str(stub), **NO_SLEEP},
    )
    assert res.returncode == 0, res.stderr
    assert res.stdout == "404\n"
    assert curl_calls(tmp_path) == 1, "a conclusive answer must not be re-asked"


def test_crates_io_probe_retries_a_transient_status_and_settles(tmp_path: Path):
    """A blip then a real answer: the retry is what stops a one-off 503 from
    failing a release that is otherwise perfectly fine."""
    stub = stub_curl_sequence(tmp_path, ["000", "503", "404"])
    res = call(
        ["crates_io_probe", "codingest", "0.1.3"],
        env={"CODINGEST_RELEASE_CURL": str(stub), **NO_SLEEP},
    )
    assert res.returncode == 0, res.stderr
    assert res.stdout == "404\n", "the settled answer must win, not the blip"
    assert curl_calls(tmp_path) == 3


def test_crates_io_probe_gives_up_after_the_attempt_budget(tmp_path: Path):
    """A PERSISTENT outage is not retried forever, and it is not laundered into
    a conclusive-looking status either — the last inconclusive one is printed
    and `assert_status_conclusive` is what refuses it."""
    stub = stub_curl_sequence(tmp_path, ["000"])
    res = call(
        ["crates_io_probe", "codingest", "0.1.3"],
        env={
            "CODINGEST_RELEASE_CURL": str(stub),
            "CRATES_IO_PROBE_ATTEMPTS": "3",
            **NO_SLEEP,
        },
    )
    assert res.returncode == 0, res.stderr
    assert res.stdout == "000\n"
    assert curl_calls(tmp_path) == 3, "the attempt budget must be honoured exactly"


def test_crates_io_probe_folds_a_hard_curl_error_into_an_inconclusive_status(
    tmp_path: Path,
):
    """A curl that EXITS non-zero (rc 7, connection refused) must not kill the
    function under `set -e` — that is the transient case retrying exists for.
    It becomes "000", which is inconclusive, which fails loudly downstream."""
    stub = stub_curl(tmp_path, {"codingest": "000"}, rc=7)
    res = call(
        ["crates_io_probe", "codingest", "0.1.3"],
        env={
            "CODINGEST_RELEASE_CURL": str(stub),
            "CRATES_IO_PROBE_ATTEMPTS": "2",
            **NO_SLEEP,
        },
    )
    assert res.returncode == 0, res.stderr
    assert res.stdout == "000\n"


@pytest.mark.parametrize("status", ["200", "404"])
def test_assert_status_conclusive_accepts_the_two_real_answers(status: str):
    res = call(["assert_status_conclusive", "codingest", "0.1.3", status])
    assert res.returncode == 0, res.stderr


@pytest.mark.parametrize("status", ["000", "403", "429", "500", "502", "301", ""])
def test_assert_status_conclusive_fail_path(status: str):
    """FAIL PATH — THE PHASE 5 GATE. Every one of these used to be a silent skip
    of all three crate publishes with the run green."""
    res = call(["assert_status_conclusive", "codingest", "0.1.3", status])
    assert res.returncode == 1, f"HTTP {status!r} was accepted as an answer"
    assert "::error::" in res.stderr


def test_publish_decision_still_refuses_to_publish_on_an_odd_status():
    """The FAIL-SAFE half of the old behaviour is deliberately KEPT.

    `assert_status_conclusive` now stops these reaching the mapping at all, but
    "never blindly publish on uncertainty" is worth two independent guards.
    """
    for status in ("000", "403", "500", ""):
        assert call(["publish_decision_for", status]).stdout == "false\n"


# --------------------------------------------------------------------------
# internal path-dependency pins — the workspace-version lockstep
# --------------------------------------------------------------------------
#
# THE BUG. Each crate's own `package.version` is `version.workspace = true`, so
# one line in the root manifest moves all four. An internal path dependency's
# `version` REQUIREMENT does not inherit — it is a hand-written literal — and all
# five of them sat at `0.1.0` while the workspace was at `0.1.3`, across two
# releases, under a fully green release gate. `cargo publish` rejects a path-only
# dependency and emits that literal verbatim, so published `codingest-cli 0.1.3`
# declares a dependency on `codingest ^0.1.0`.
#
# Severity is LOW AND LATENT, and these tests do not pretend otherwise: `^0.1.0`
# resolves to 0.1.3 for every consumer today. It breaks at the first minor bump
# and is already wrong under `-Z minimal-versions`.
#
# WHAT IS ACTUALLY BEING TESTED is the discovery, not the comparison. Comparing
# five known lines is easy and rots the day a crate is added — and the crate that
# is added is exactly the one whose pin nobody thought about. So the fixtures
# below include "add a whole new member with a stale pin", and
# `test_inverted_control_a_hardcoded_pin_list_misses_a_new_crate` shows the naive
# form staying green through it.
#
# FIXTURES, NOT THE LIVE REPO, for everything except the one regression guard:
# the real version bumps on every release and a suite that asserted against
# `0.1.3` would go red on the next one for no reason. The single test that DOES
# read the real manifests reads the version out of them too.

OMIT = object()  # a path dependency declared with no `version` key at all

# The real workspace's shape: four members, five internal pins, one crate
# (`codingest`) depending on nothing.
DEFAULT_CRATES: dict[str, list] = {
    "codingest": [],
    "codingest-cli": [("codingest", None)],
    "codingest-mcp": [("codingest", None)],
    "codingest-py": [
        ("codingest", None),
        ("codingest-cli", None),
        ("codingest-mcp", None),
    ],
}

# The five real pin sites, as `internal_pins` reports them. A LIST IN A TEST IS
# FINE — a test that goes red when a crate is added is a prompt; a GATE that
# stays green when a crate is added is the dead gate this whole section exists to
# prevent. See test_the_real_repo_pin_sites_are_all_discovered.
REAL_PIN_SITES = [
    ("crates/codingest-cli/Cargo.toml", "codingest"),
    ("crates/codingest-mcp/Cargo.toml", "codingest"),
    ("crates/codingest-py/Cargo.toml", "codingest"),
    ("crates/codingest-py/Cargo.toml", "codingest-cli"),
    ("crates/codingest-py/Cargo.toml", "codingest-mcp"),
]


def write_workspace(
    root: Path,
    version: str,
    crates: dict[str, list] | None = None,
    *,
    root_extra: str = "",
) -> Path:
    """Write a miniature cargo workspace under `root`; return its root manifest.

    `crates` maps crate name -> list of `(sibling, pin)`. A pin of `None` means
    "the workspace version" (the correct state), `OMIT` means "declare the path
    dependency with no `version` key", and any string is written literally — that
    is how a single stale pin is planted without touching anything else.
    """
    if crates is None:
        crates = DEFAULT_CRATES
    members = "".join(f'    "crates/{name}",\n' for name in crates)
    manifest = root / "Cargo.toml"
    manifest.parent.mkdir(parents=True, exist_ok=True)
    manifest.write_text(
        f"[workspace]\nmembers = [\n{members}]\nresolver = \"2\"\n\n"
        f'[workspace.package]\nversion = "{version}"\n{root_extra}'
    )
    for name, deps in crates.items():
        crate_dir = root / "crates" / name
        crate_dir.mkdir(parents=True, exist_ok=True)
        body = [f'[package]\nname = "{name}"\nversion.workspace = true\n']
        if deps:
            body.append("\n[dependencies]\n")
            for dep, pin in deps:
                if pin is OMIT:
                    body.append(f'{dep} = {{ path = "../{dep}" }}\n')
                else:
                    resolved = version if pin is None else pin
                    body.append(
                        f'{dep} = {{ version = "{resolved}", path = "../{dep}" }}\n'
                    )
        (crate_dir / "Cargo.toml").write_text("".join(body))
    return manifest


def stale(site: tuple[str, str], pin: str = "0.1.0") -> dict[str, list]:
    """DEFAULT_CRATES with exactly one pin changed.

    `site` is a REAL_PIN_SITES entry — (manifest path, sibling) — so the
    parametrised fail-path tests are driven by the real pin sites rather than by a
    second list written just for the fixtures.
    """
    crate = Path(site[0]).parent.name
    sibling = site[1]
    out = {k: list(v) for k, v in DEFAULT_CRATES.items()}
    out[crate] = [(d, pin if d == sibling else p) for d, p in out[crate]]
    assert out[crate] != DEFAULT_CRATES[crate], f"premise broken: {site} not planted"
    return out


def pin_lines(res) -> list[tuple[str, str, str]]:
    return [tuple(l.split("\t")) for l in res.stdout.splitlines() if l]


def test_internal_pins_finds_every_internal_pin(tmp_path: Path):
    """The whole gate rests on this: the discovered set is the complete set."""
    manifest = write_workspace(tmp_path, "0.1.3")
    res = call(["internal_pins", str(manifest)])
    assert res.returncode == 0, res.stderr
    assert pin_lines(res) == [
        ("crates/codingest-cli/Cargo.toml", "codingest", "0.1.3"),
        ("crates/codingest-mcp/Cargo.toml", "codingest", "0.1.3"),
        ("crates/codingest-py/Cargo.toml", "codingest", "0.1.3"),
        ("crates/codingest-py/Cargo.toml", "codingest-cli", "0.1.3"),
        ("crates/codingest-py/Cargo.toml", "codingest-mcp", "0.1.3"),
    ]


def test_internal_pins_discovers_a_crate_that_did_not_exist_yet(tmp_path: Path):
    """DISCOVERY, NOT ENUMERATION. A new member's pins are scanned because the
    scan set comes from `[workspace] members` — the same declaration that decides
    what cargo builds — not from a list in the gate."""
    crates = {**DEFAULT_CRATES, "codingest-extra": [("codingest", None)]}
    manifest = write_workspace(tmp_path, "0.1.3", crates)
    res = call(["internal_pins", str(manifest)])
    assert res.returncode == 0, res.stderr
    assert ("crates/codingest-extra/Cargo.toml", "codingest", "0.1.3") in pin_lines(res)
    assert len(pin_lines(res)) == 6


def test_assert_internal_pins_lockstep_pass_path(tmp_path: Path):
    manifest = write_workspace(tmp_path, "0.1.3")
    res = call(["assert_internal_pins_lockstep", "0.1.3", str(manifest)])
    assert res.returncode == 0, res.stderr
    assert "5 internal path-dependency pin(s) in lockstep at 0.1.3" in res.stderr


@pytest.mark.parametrize("site", REAL_PIN_SITES, ids=lambda s: f"{s[0]}:{s[1]}")
def test_assert_internal_pins_lockstep_fail_path_one_stale_pin(
    tmp_path: Path, site: tuple[str, str]
):
    """FAIL PATH, once per real pin site: reverting ANY ONE of them must go red.

    A gate that only notices when all five drift together is no gate — the five
    are edited by hand, one at a time, by whoever adds a dependency.
    """
    manifest = write_workspace(tmp_path, "0.1.3", stale(site))
    res = call(["assert_internal_pins_lockstep", "0.1.3", str(manifest)])
    assert res.returncode == 1, f"{site} drifted to 0.1.0 and the gate stayed green"
    assert "::error::version-pin drift" in res.stderr
    assert f"{site[0]} pins '{site[1]}' at '0.1.0'" in res.stderr
    assert "1 of 5 internal path-dependency pin(s) disagree" in res.stderr


def test_assert_internal_pins_lockstep_fail_path_a_new_crate_with_a_stale_pin(
    tmp_path: Path,
):
    """THE DESIGN REQUIREMENT. Adding a workspace crate whose pin is stale must
    NOT be able to leave this gate passing.

    This is the failure class the gate is guarding against being: correct for the
    crates that existed when it was written, blind to the next one. The five
    correct pins are untouched, so nothing but the new crate can fail it.
    """
    crates = {**DEFAULT_CRATES, "codingest-extra": [("codingest", "0.1.0")]}
    manifest = write_workspace(tmp_path, "0.1.3", crates)
    res = call(["assert_internal_pins_lockstep", "0.1.3", str(manifest)])
    assert res.returncode == 1, (
        "a NEW crate with a stale pin passed the gate — the gate is a snapshot of "
        "the crates that existed when it was written, which is the dead-gate "
        "failure it exists to prevent"
    )
    assert "crates/codingest-extra/Cargo.toml pins 'codingest' at '0.1.0'" in res.stderr
    assert "1 of 6 internal path-dependency pin(s) disagree" in res.stderr


def test_assert_internal_pins_lockstep_fail_path_a_pin_ahead_of_the_workspace(
    tmp_path: Path,
):
    """Drift is not "older than" — it is "not equal to". A pin bumped ahead of
    the workspace version cannot resolve at all once published."""
    manifest = write_workspace(tmp_path, "0.1.3", stale(REAL_PIN_SITES[0], "0.1.4"))
    res = call(["assert_internal_pins_lockstep", "0.1.3", str(manifest)])
    assert res.returncode == 1, "a pin AHEAD of the workspace version passed"


def test_assert_internal_pins_lockstep_fail_path_no_version_key(tmp_path: Path):
    """A path-only sibling has no pin to check — and `cargo publish` refuses it
    outright, so the crate could not be published anyway."""
    manifest = write_workspace(tmp_path, "0.1.3", stale(REAL_PIN_SITES[0], OMIT))
    res = call(["assert_internal_pins_lockstep", "0.1.3", str(manifest)])
    assert res.returncode == 1, "a path-only internal dependency passed the gate"
    assert "with no `version` key" in res.stderr


def test_assert_internal_pins_lockstep_fail_path_without_a_version(tmp_path: Path):
    """An empty version makes every comparison meaningless — it must not pass."""
    manifest = write_workspace(tmp_path, "0.1.3")
    res = call(["assert_internal_pins_lockstep", "", str(manifest)])
    assert res.returncode == 1
    assert "::error::" in res.stderr


def test_internal_pins_reads_the_dependencies_section_form(tmp_path: Path):
    """`[dependencies.codingest]` with the keys on their own lines is the same
    declaration in TOML, and a scanner that only reads one-line inline tables
    would report zero pins for it — silently."""
    manifest = write_workspace(tmp_path, "0.1.3", {"codingest": [], "b": []})
    (tmp_path / "crates" / "b" / "Cargo.toml").write_text(
        '[package]\nname = "b"\nversion.workspace = true\n\n'
        '[dependencies.codingest]\nversion = "0.1.0"\npath = "../codingest"\n'
    )
    res = call(["internal_pins", str(manifest)])
    assert res.returncode == 0, res.stderr
    assert pin_lines(res) == [("crates/b/Cargo.toml", "codingest", "0.1.0")]
    assert call(["assert_internal_pins_lockstep", "0.1.3", str(manifest)]).returncode == 1


def test_internal_pins_reads_the_root_workspace_dependencies_table(tmp_path: Path):
    """A pin moved to the root `[workspace.dependencies]` table is still a pin.

    Scanning only `crates/*/Cargo.toml` would take it out of scope, so the drift
    would become invisible by relocation rather than by editing.
    """
    manifest = write_workspace(
        tmp_path,
        "0.1.3",
        {"codingest": [], "b": []},
        root_extra='\n[workspace.dependencies]\n'
        'codingest = { version = "0.1.0", path = "crates/codingest" }\n',
    )
    res = call(["internal_pins", str(manifest)])
    assert res.returncode == 0, res.stderr
    assert pin_lines(res) == [("Cargo.toml", "codingest", "0.1.0")]
    assert call(["assert_internal_pins_lockstep", "0.1.3", str(manifest)]).returncode == 1


def test_internal_pins_skips_a_path_dep_outside_the_workspace(tmp_path: Path):
    """An external path dependency's version is not the workspace version, so
    holding it to the lockstep would be wrong. It is skipped WITH A NOTE, never
    mutely."""
    manifest = write_workspace(tmp_path, "0.1.3")
    outside = tmp_path / "vendor" / "thing"
    outside.mkdir(parents=True)
    (outside / "Cargo.toml").write_text('[package]\nname = "thing"\nversion = "9.9.9"\n')
    cli = tmp_path / "crates" / "codingest-cli" / "Cargo.toml"
    cli.write_text(
        cli.read_text() + 'thing = { version = "9.9.9", path = "../../vendor/thing" }\n'
    )
    res = call(["internal_pins", str(manifest)])
    assert res.returncode == 0, res.stderr
    assert len(pin_lines(res)) == 5, "an external path dep was pulled into the lockstep"
    assert "resolves outside the workspace" in res.stderr


@pytest.mark.parametrize(
    ("label", "dep_block", "needle"),
    [
        (
            "multi-line inline table",
            '[dependencies]\ncodingest = {\n  version = "0.1.0",\n  path = "../codingest",\n}\n',
            "MULTI-LINE inline table",
        ),
        (
            "dotted key",
            '[dependencies]\ncodingest.version = "0.1.0"\ncodingest.path = "../codingest"\n',
            "dotted-key form",
        ),
    ],
)
def test_internal_pins_fail_path_unreadable_dependency_form(
    tmp_path: Path, label: str, dep_block: str, needle: str
):
    """FAIL PATH — the line-oriented scan's own blind spots, turned LOUD.

    These are legal TOML that a one-line-per-dependency scan cannot read. The
    dangerous outcome is not "unsupported", it is "reports zero pins for that
    crate and passes": the pin would go unchecked with nothing said. So the
    scanner recognises both forms and refuses instead.
    """
    manifest = write_workspace(tmp_path, "0.1.3", {"codingest": [], "b": []})
    (tmp_path / "crates" / "b" / "Cargo.toml").write_text(
        f'[package]\nname = "b"\nversion.workspace = true\n\n{dep_block}'
    )
    res = call(["internal_pins", str(manifest)])
    assert res.returncode == 1, f"{label}: a pin the scanner cannot read passed silently"
    assert needle in res.stderr
    gate = call(["assert_internal_pins_lockstep", "0.1.3", str(manifest)])
    assert gate.returncode == 1, f"{label}: the gate did not inherit the failure"
    assert "could not be read" in gate.stderr


def test_internal_pins_fail_path_a_path_that_resolves_to_nothing(tmp_path: Path):
    manifest = write_workspace(tmp_path, "0.1.3")
    cli = tmp_path / "crates" / "codingest-cli" / "Cargo.toml"
    cli.write_text(cli.read_text().replace("../codingest", "../ghost"))
    res = call(["internal_pins", str(manifest)])
    assert res.returncode == 1
    assert "no Cargo.toml there" in res.stderr


def test_internal_pins_fail_path_no_members(tmp_path: Path):
    """REFUSE AN EMPTY EXPECTATION — the `wheel_matrix_legs` principle. If the
    workspace layout moved, zero manifests to scan must not read as zero
    problems."""
    manifest = tmp_path / "Cargo.toml"
    manifest.write_text('[workspace.package]\nversion = "0.1.3"\n')
    res = call(["internal_pins", str(manifest)])
    assert res.returncode == 1
    assert "vacuously true" in res.stderr


def test_internal_pins_fail_path_zero_pins_found(tmp_path: Path):
    """Same principle one level down: members that declare no internal path
    dependency at all means the scan stopped matching, because every publishable
    crate here depends on a sibling."""
    manifest = write_workspace(tmp_path, "0.1.3", {"codingest": [], "b": []})
    res = call(["internal_pins", str(manifest)])
    assert res.returncode == 1
    assert "no internal path-dependency pins found" in res.stderr


def test_workspace_member_manifests_fail_path_unregistered_crate_dir(tmp_path: Path):
    """A crate dir that no `members` entry claims is an ERROR, not a skip.

    Without this, "add a crate" has a second silent path: create
    `crates/foo/Cargo.toml`, forget the members entry, and its pins are outside
    every scan.
    """
    manifest = write_workspace(tmp_path, "0.1.3")
    rogue = tmp_path / "crates" / "codingest-rogue"
    rogue.mkdir()
    (rogue / "Cargo.toml").write_text(
        '[package]\nname = "codingest-rogue"\n\n[dependencies]\n'
        'codingest = { version = "0.1.0", path = "../codingest" }\n'
    )
    res = call(["workspace_member_manifests", str(manifest)])
    assert res.returncode == 1, "an unregistered crate dir was scanned past silently"
    assert "not a `[workspace] members` entry" in res.stderr
    assert call(["assert_internal_pins_lockstep", "0.1.3", str(manifest)]).returncode == 1


def test_workspace_member_manifests_fail_path_member_without_a_manifest(tmp_path: Path):
    manifest = write_workspace(tmp_path, "0.1.3")
    manifest.write_text(manifest.read_text().replace(
        '    "crates/codingest",\n',
        '    "crates/codingest",\n    "crates/ghost",\n',
        1,
    ))
    res = call(["workspace_member_manifests", str(manifest)])
    assert res.returncode == 1
    assert "has no Cargo.toml" in res.stderr


def test_workspace_member_manifests_reads_the_real_root_manifest():
    res = call(["workspace_member_manifests", "Cargo.toml"])
    assert res.returncode == 0, res.stderr
    got = [Path(l).relative_to(REPO.resolve()).as_posix() for l in res.stdout.split()]
    assert got == [
        "crates/codingest/Cargo.toml",
        "crates/codingest-cli/Cargo.toml",
        "crates/codingest-mcp/Cargo.toml",
        "crates/codingest-py/Cargo.toml",
    ]


def test_the_real_repo_pins_are_in_lockstep_with_the_workspace_version():
    """THE REGRESSION GUARD for the bug this gate was written for.

    The five real pins sat at 0.1.0 with the workspace at 0.1.3. The version is
    read out of the manifest rather than written here, so this test does not go
    red on the next release — only on drift.
    """
    version = call(["extract_version", "Cargo.toml"]).stdout.strip()
    assert version, "could not read the real workspace version"
    res = call(["assert_internal_pins_lockstep", version, "Cargo.toml"])
    assert res.returncode == 0, (
        "the REAL manifests are out of lockstep — a published crate would declare "
        f"the wrong requirement on its sibling:\n{res.stderr}"
    )


def test_kglite_cargo_and_python_floors_are_in_lockstep():
    """The Rust writer and Python reader must share one KGLite patch floor.

    `codingest.build()` writes `.kgl` bytes with the Rust crate and loads them
    through the separately installed Python wheel. A Cargo-only floor bump can
    therefore leave the acceptance suite exercising the previous Python engine.
    """
    cargo = (REPO / "Cargo.toml").read_text()
    rust_floors = re.findall(
        r'^kglite(?:-mcp-server)? = \{ version = "([^"]+)"',
        cargo,
        flags=re.MULTILINE,
    )
    assert len(rust_floors) == 2, "did not find both KGLite workspace dependencies"
    assert len(set(rust_floors)) == 1, "kglite and kglite-mcp-server floors differ"

    pyproject = (REPO / "pyproject.toml").read_text()
    match = re.search(r'"kglite>=([^,]+),<0\.16"', pyproject)
    assert match, "did not find the bounded Python KGLite runtime requirement"
    assert match.group(1) == rust_floors[0], (
        "Python KGLite floor differs from the Rust engine/MCP floor"
    )


def test_the_real_repo_pin_sites_are_all_discovered():
    """Coverage of the real tree, not just of the fixtures.

    `internal_pins` refusing an EMPTY set catches a scanner that matches nothing;
    it cannot catch one that matches three of five. This pins the discovered set
    to the five known sites, so a scanner regression is visible and a new crate
    prompts an update here.
    """
    res = call(["internal_pins", "Cargo.toml"])
    assert res.returncode == 0, res.stderr
    assert [(m, d) for m, d, _ in pin_lines(res)] == REAL_PIN_SITES


def test_inverted_control_a_hardcoded_pin_list_misses_a_new_crate(tmp_path: Path):
    """INVERTED CONTROL: both naive forms stay GREEN through mutations the real
    gate fails on.

    NAIVE 1 — check the three manifests known at writing time. A new member with
    a stale pin is simply not looked at, so it passes.
    NAIVE 2 — grep for the literal `0.1.0`. Drift to any other wrong value (a pin
    left at `0.1.2` through a patch bump) contains no `0.1.0` at all, so it
    passes.
    Both are exactly how this gate would have been born dead.
    """
    known = ["codingest-cli", "codingest-mcp", "codingest-py"]

    def naive_known_three(root: Path, version: str) -> bool:
        """GREEN iff every pin in the three hardcoded manifests matches."""
        for name in known:
            text = (root / "crates" / name / "Cargo.toml").read_text()
            for line in text.splitlines():
                if "path = " in line and "version = " in line:
                    if f'version = "{version}"' not in line:
                        return False
        return True

    def naive_grep_0_1_0(root: Path, _version: str) -> bool:
        """GREEN iff the literal `0.1.0` appears nowhere."""
        return not any(
            "0.1.0" in p.read_text() for p in root.rglob("Cargo.toml")
        )

    # Mutation A: a NEW crate, with a stale pin.
    a = tmp_path / "a"
    write_workspace(a, "0.1.3", {**DEFAULT_CRATES, "codingest-extra": [("codingest", "0.1.0")]})
    assert naive_known_three(a, "0.1.3"), "premise broken: naive 1 should not see it"
    assert call(["assert_internal_pins_lockstep", "0.1.3", str(a / "Cargo.toml")]).returncode == 1

    # Mutation B: drift to a value that is not the literal `0.1.0`.
    b = tmp_path / "b"
    write_workspace(b, "0.1.3", stale(REAL_PIN_SITES[0], "0.1.2"))
    assert naive_grep_0_1_0(b, "0.1.3"), "premise broken: naive 2 should not see it"
    assert call(["assert_internal_pins_lockstep", "0.1.3", str(b / "Cargo.toml")]).returncode == 1

    # And the control is a control: both naive forms DO pass the clean tree, so
    # their greens above are not a constant `True`.
    clean = tmp_path / "clean"
    write_workspace(clean, "0.1.3")
    assert naive_known_three(clean, "0.1.3")
    assert naive_grep_0_1_0(clean, "0.1.3")
    assert call(["assert_internal_pins_lockstep", "0.1.3", str(clean / "Cargo.toml")]).returncode == 0


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
    crates: dict[str, list] | None = None,
):
    if manifest_body is None:
        # A REAL fixture workspace, not a two-line manifest: the step now also
        # asserts the internal path-dependency pins are in lockstep, and a gate
        # that its own step-level tests route around is not wired in.
        manifest = write_workspace(tmp_path, version, crates)
    else:
        manifest = tmp_path / "Cargo.toml"
        manifest.write_text(manifest_body)
    stub = stub_curl(tmp_path, statuses)
    out_file = tmp_path / "github_output"
    out_file.write_text("")
    res = call(
        ["check_crates_to_publish", str(manifest), *CRATES],
        env={
            "CODINGEST_RELEASE_CURL": str(stub),
            "GITHUB_OUTPUT": str(out_file),
            **NO_SLEEP,
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
    """200 = already there (skip), 404 = publish. Both are CONCLUSIVE.

    This used to include a 500 leg asserting it silently yielded `false`. That
    was the defect, not the contract: see
    `test_check_crates_fail_path_inconclusive_status` below.
    """
    res, out_file = _run_check(
        tmp_path,
        "0.1.3",
        {"codingest": "200", "codingest-cli": "404", "codingest-mcp": "200"},
    )
    assert res.returncode == 0, res.stderr
    assert out_file.read_text().splitlines() == [
        "version=0.1.3",
        "publish_codingest=false",
        "publish_codingest_cli=true",
        "publish_codingest_mcp=false",
    ]


def test_check_crates_all_present_skips_cleanly(tmp_path: Path):
    """GREEN: every crate already on crates.io — the documented idempotent
    re-run. A genuine 200 must stay a quiet, successful skip; only UNCERTAINTY
    became loud."""
    res, out_file = _run_check(tmp_path, "0.1.3", {c: "200" for c in CRATES})
    assert res.returncode == 0, res.stderr
    assert out_file.read_text().splitlines() == [
        "version=0.1.3",
        "publish_codingest=false",
        "publish_codingest_cli=false",
        "publish_codingest_mcp=false",
    ]


@pytest.mark.parametrize("status", ["000", "403", "500", "502"])
def test_check_crates_fail_path_inconclusive_status(tmp_path: Path, status: str):
    """FAIL PATH — THE PHASE 5 GATE, at step level.

    One crate the probe cannot resolve fails the whole step and writes NOTHING.
    Before this, HTTP 000 from a transient network error fell into the else arm
    of a `= 404` branch and set all three `publish_*` outputs to `false`: every
    crate publish skipped, no annotation, run green.

    Nothing is written even for the crates that DID answer, on purpose — a step
    that fails after emitting `publish_codingest=true` leaves outputs describing
    a decision that was never taken.
    """
    res, out_file = _run_check(
        tmp_path,
        "0.1.4",
        {"codingest": "404", "codingest-cli": status, "codingest-mcp": "404"},
    )
    assert res.returncode == 1, f"HTTP {status} was silently skipped again"
    assert "::error::" in res.stderr
    assert out_file.read_text() == "", "no outputs may be written on the fail path"


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


def test_check_crates_fail_path_stale_pin_never_reaches_github_output(tmp_path: Path):
    """FAIL PATH (gate 3) AT STEP LEVEL — this is the wiring proof.

    The lockstep assertion has to run inside the step that already carries the
    version-shape and tag-skew gates, before the first `$GITHUB_OUTPUT` write and
    before any network probe. Asserting on those three properties is what makes
    the wiring itself testable: an assertion added below the first `gh_output`, or
    left out of the step entirely, fails here.
    """
    res, out_file = _run_check(
        tmp_path,
        "0.1.3",
        {c: "404" for c in CRATES},
        crates=stale(REAL_PIN_SITES[0]),
        ref={"GITHUB_REF_NAME": "v0.1.3", "GITHUB_REF_TYPE": "tag"},
    )
    assert res.returncode == 1, "a stale internal pin did not fail the release step"
    assert "::error::version-pin drift" in res.stderr
    assert out_file.read_text() == "", "no outputs may be written on the fail path"
    assert not (tmp_path / "curl_argv").exists(), (
        "crates.io was probed before the pin gate ran — the gate must sit with the "
        "other manifest assertions, ahead of every network call"
    )


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
# assert_publish_ref  (Phase 5 gate 3 — the workflow_dispatch path)
# --------------------------------------------------------------------------
#
# release.yml used to also carry a `workflow_dispatch:` trigger, and on a
# dispatch run GITHUB_REF_NAME is a BRANCH. `${GITHUB_REF_NAME#v}` is then
# `main`, so the changelog lookup asks for a `## [main]` section and degrades to
# auto-generated notes; the binaries are packaged
# `codingest-main-linux-x86_64.tar.gz`; and softprops, handed a non-tag ref,
# CREATES a tag and release named after the branch. All green.
#
# THE DECISION: dispatch is blocked from the publish path outright, not allowed
# in a degraded dry-run mode. A dry-run would have to condition every publish
# and release action on the ref type, and each such condition is itself
# something branch CI can never exercise — more unfailable surface, to buy a
# rehearsal `ci.yml` already provides. The `on:` block therefore lists only the
# `v*` tag push, and this function is the backstop.


def test_assert_publish_ref_accepts_a_version_tag():
    res = call(["assert_publish_ref", "v0.1.3", "tag", "push"])
    assert res.returncode == 0, res.stderr


def test_assert_publish_ref_accepts_a_prerelease_tag():
    res = call(["assert_publish_ref", "v1.0.0-rc.1", "tag", "push"])
    assert res.returncode == 0, res.stderr


@pytest.mark.parametrize(
    ("ref_name", "ref_type", "event"),
    [
        ("main", "branch", "workflow_dispatch"),  # THE case
        ("fix/publish-path-gates", "branch", "workflow_dispatch"),
        ("v0.1.3", "branch", "workflow_dispatch"),  # a BRANCH named like a tag
        ("main", "", ""),  # ref_type unset entirely
        ("", "", ""),
    ],
)
def test_assert_publish_ref_fail_path_not_a_tag(ref_name, ref_type, event):
    """FAIL PATH: anything that is not a tag cannot reach a publish step."""
    res = call(["assert_publish_ref", ref_name, ref_type, event])
    assert res.returncode == 1, f"{ref_name!r}/{ref_type!r} reached the publish path"
    assert "::error::" in res.stderr


@pytest.mark.parametrize("ref_name", ["nightly", "v1.2", "latest", "0.1.3", "v", "vnext"])
def test_assert_publish_ref_fail_path_tag_is_not_a_version(ref_name: str):
    """FAIL PATH: a real tag whose name is not `v<N.N.N>`.

    There is no manifest version for it to agree with and no changelog section
    for it to find, so every downstream gate would be comparing against noise.
    """
    res = call(["assert_publish_ref", ref_name, "tag", "push"])
    assert res.returncode == 1, f"tag {ref_name!r} was accepted as a release tag"
    assert "::error::" in res.stderr


def test_assert_publish_ref_fail_path_with_no_arguments():
    res = call(["assert_publish_ref"])
    assert res.returncode == 1
    assert "::error::" in res.stderr


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


def test_inverted_control_a_not_404_skip_would_swallow_a_transient_status():
    """NAIVE: `[ "$status" = 404 ] && publish || skip` — the SHIPPED behaviour.

    It is green on HTTP 000, 403 and 5xx alike, having decided to publish
    nothing. That is the whole Phase 5 defect: the fail-safe was right, its
    silence was not. The real gate refuses the same inputs out loud.
    """
    for status in ("000", "403", "500"):
        naive = run_shell(
            f'status="{status}"\n'
            'if [ "$status" = "404" ]; then echo NAIVE_PUBLISH; else echo NAIVE_PASS; fi\n'
        )
        assert "NAIVE_PASS" in naive.stdout, f"premise broken for {status!r}"
        assert naive.returncode == 0, "premise broken: the naive form must be green"
        res = call(["assert_status_conclusive", "codingest", "0.1.3", status])
        assert res.returncode == 1, (
            f"the real gate must fail on {status!r} where the naive skip passes"
        )


def test_inverted_control_a_v_prefix_check_would_pass_a_branch():
    """NAIVE: `case "$GITHUB_REF_NAME" in v*)` — "it looks like a tag".

    A branch named `vnext`, or a dispatch on a branch someone tagged-and-named
    `v0.1.3`, satisfies it. The real gate keys on GITHUB_REF_TYPE, which is the
    only thing that actually distinguishes a dispatch from a tag push.
    """
    for ref in ("vnext", "v0.1.3"):
        naive = run_shell(
            f'ref="{ref}"\n'
            'case "$ref" in v*) echo NAIVE_PASS ;; *) echo NAIVE_FAIL ;; esac\n'
        )
        assert "NAIVE_PASS" in naive.stdout, f"premise broken for {ref!r}"
        assert call(["assert_publish_ref", ref, "branch", "workflow_dispatch"]).returncode == 1, (
            f"the real gate must reject the branch {ref!r} the naive check passes"
        )


def test_inverted_control_a_presence_check_would_pass_an_absent_changelog(
    tmp_path: Path,
):
    """NAIVE: `[ -f CHANGELOG.md ]` — "the changelog exists", the honour-system
    version of the release skill's promote step.

    A CHANGELOG.md with no `## [0.9.9]` section satisfies it; that is exactly
    the state a forgotten promotion leaves behind, and it used to degrade to
    auto-generated notes without a word.
    """
    changelog = _changelog(tmp_path)
    naive = run_shell(
        f'if [ -f "{changelog}" ]; then echo NAIVE_PASS; else echo NAIVE_FAIL; fi'
    )
    assert "NAIVE_PASS" in naive.stdout, "premise broken: the changelog is missing"
    out_file = tmp_path / "github_output"
    out_file.write_text("")
    res = call(
        ["changelog_notes", "0.9.9", str(changelog), str(tmp_path / "n.md"), "tag"],
        env={"GITHUB_OUTPUT": str(out_file)},
    )
    assert res.returncode == 1, "the real gate must reject the unpromoted changelog"


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


@pytest.mark.parametrize("ref_type", ["", "branch", "unknown"])
def test_changelog_notes_missing_section_off_a_tag_degrades(tmp_path, ref_type):
    """OFF A TAG: no section -> has_notes=false and no notes file written.

    The requirement is scoped to tag builds — a local run has no release to
    gate. Kept as a real branch rather than made unreachable, so a caller that
    forgets the ref_type argument degrades instead of erroring on a version it
    had no business looking up.
    """
    out_file = tmp_path / "github_output"
    out_file.write_text("")
    notes = tmp_path / "release_notes.md"
    args = ["changelog_notes", "9.9.9", str(_changelog(tmp_path)), str(notes)]
    res = call(
        args + ([ref_type] if ref_type else []),
        env={"GITHUB_OUTPUT": str(out_file)},
    )
    assert res.returncode == 0, res.stderr
    assert out_file.read_text().splitlines() == ["has_notes=false"]
    assert not notes.exists()


def test_changelog_notes_fail_path_missing_section_on_a_tag(tmp_path: Path):
    """FAIL PATH — THE PHASE 5 GATE. A missing `## [x.y.z]` on a `v*` tag.

    It used to set has_notes=false, which the workflow read as "use
    auto-generated notes": a silent downgrade from the curated changelog to a
    list of commit subjects, fired at precisely the moment something was wrong
    — the release skill's promote-the-CHANGELOG step was skipped, or the tag
    disagrees with the manifest. This is what makes that step a gate rather
    than an honour-system one.
    """
    out_file = tmp_path / "github_output"
    out_file.write_text("")
    notes = tmp_path / "release_notes.md"
    res = call(
        ["changelog_notes", "9.9.9", str(_changelog(tmp_path)), str(notes), "tag"],
        env={"GITHUB_OUTPUT": str(out_file)},
    )
    assert res.returncode == 1, "an unpromoted CHANGELOG still released quietly"
    assert "::error::" in res.stderr
    assert out_file.read_text() == "", "no outputs may be written on the fail path"
    assert not notes.exists()


def test_changelog_notes_pass_path_on_a_tag(tmp_path: Path):
    """GREEN: a promoted section on a tag build behaves exactly as before."""
    out_file = tmp_path / "github_output"
    out_file.write_text("")
    notes = tmp_path / "release_notes.md"
    res = call(
        ["changelog_notes", "0.1.3", str(_changelog(tmp_path)), str(notes), "tag"],
        env={"GITHUB_OUTPUT": str(out_file)},
    )
    assert res.returncode == 0, res.stderr
    assert out_file.read_text().splitlines() == ["has_notes=true"]
    assert "the AGC semantics" in notes.read_text()


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
# artifact set  (Phase 4 — replaces release.yml's `ls -la dist`)
# --------------------------------------------------------------------------
#
# WHAT WAS BROKEN. `publish-pypi` "verified" its downloaded artifacts with
# `ls -la dist` — a command that cannot fail, on a directory `download-artifact`
# always creates. With `if-no-files-found` defaulting to `warn` on the uploads
# and `skip-existing: true` on the publish, a leg that produced no wheel shipped
# a PARTIAL wheel set to PyPI with the whole run green.
#
# WHY THE EXPECTATION IS DERIVED. "assert 6 files" rots the moment a matrix leg
# is added or removed, and a stale constant is its own dead gate. The expected
# set is read out of the build-wheels matrix in release.yml — the same
# definition that decides how many wheels get built — and each leg is mapped to
# the platform tag its wheel must carry. The tests below pin all four ways that
# has to behave when a leg is ADDED: known os/target grows the expectation
# (short set -> red), unknown os/target hard-fails for a mapping, a duplicate
# tag mapping fails the bijection, and an unclaimed wheel fails too.

WHEELS = {
    "ubuntu-latest/x86_64": "codingest-0.1.3-cp38-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl",
    "ubuntu-latest/aarch64": "codingest-0.1.3-cp38-abi3-manylinux_2_28_aarch64.whl",
    "macos-latest/aarch64": "codingest-0.1.3-cp38-abi3-macosx_11_0_arm64.whl",
    "macos-latest/x86_64": "codingest-0.1.3-cp38-abi3-macosx_10_12_x86_64.whl",
    "windows-latest/x64": "codingest-0.1.3-cp38-abi3-win_amd64.whl",
}
SDIST = "codingest-0.1.3.tar.gz"

REAL_LEGS = [
    "ubuntu-latest\tx86_64",
    "ubuntu-latest\taarch64",
    "macos-latest\taarch64",
    "macos-latest\tx86_64",
    "windows-latest\tx64",
]


def make_dist(tmp_path: Path, names, *, sub: str = "dist") -> Path:
    dist = tmp_path / sub
    dist.mkdir(parents=True, exist_ok=True)
    for name in names:
        (dist / name).write_bytes(b"")
    return dist


def complete_dist(tmp_path: Path, **kw) -> Path:
    return make_dist(tmp_path, [*WHEELS.values(), SDIST], **kw)


def workflow_with_extra_leg(tmp_path: Path, os_: str, target: str, name="wf.yml") -> Path:
    """The REAL release.yml with one extra build-wheels matrix leg spliced in.

    Deliberately derived from the shipped file rather than a hand-written stub:
    the whole design claim is that the expectation tracks the real matrix, and a
    stub would only prove the parser works on a stub. The insertion anchor is
    asserted, so a matrix restructure breaks this fixture loudly instead of
    quietly testing nothing.
    """
    raw = RELEASE_WORKFLOW.read_text().splitlines(keepends=True)
    anchor = next(
        (i for i, l in enumerate(raw) if l.strip() == "- os: windows-latest"), None
    )
    assert anchor is not None, "build-wheels matrix anchor moved; fix this fixture"
    indent = raw[anchor][: len(raw[anchor]) - len(raw[anchor].lstrip())]
    leg = f"{indent}- os: {os_}\n{indent}  target: {target}\n"
    out = tmp_path / name
    out.write_text("".join(raw[:anchor] + [leg] + raw[anchor:]))
    return out


def test_wheel_matrix_legs_reads_the_real_matrix():
    """The expectation comes from the shipped matrix, not from a constant."""
    res = call(["wheel_matrix_legs", str(RELEASE_WORKFLOW)])
    assert res.returncode == 0, res.stderr
    assert res.stdout.splitlines() == REAL_LEGS


def test_wheel_matrix_legs_ignores_the_release_binaries_matrix():
    """`release-binaries` has its own matrix whose legs carry `suffix`, not
    `target`. Leaking into it would both inflate the count and trip the
    missing-target error."""
    res = call(["wheel_matrix_legs", str(RELEASE_WORKFLOW)])
    assert "linux-x86_64" not in res.stdout
    assert len(res.stdout.splitlines()) == 5


def test_wheel_matrix_legs_follows_an_added_leg(tmp_path: Path):
    """ADD A LEG -> the derived expectation grows. This is the property that
    makes a hardcoded count unnecessary."""
    wf = workflow_with_extra_leg(tmp_path, "ubuntu-latest", "armv7")
    res = call(["wheel_matrix_legs", str(wf)])
    assert res.returncode == 0, res.stderr
    assert res.stdout.splitlines() == REAL_LEGS[:4] + [
        "ubuntu-latest\tarmv7",
        "windows-latest\tx64",
    ]


def test_wheel_matrix_legs_fail_path_no_matrix(tmp_path: Path):
    """FAIL PATH: a restructured/renamed matrix must be an error, not zero legs.

    Zero legs would make `assert_artifact_set` vacuously true — the exact shape
    of a gate that passes on a short set.
    """
    wf = tmp_path / "wf.yml"
    wf.write_text("jobs:\n  other-job:\n    runs-on: ubuntu-latest\n")
    res = call(["wheel_matrix_legs", str(wf)])
    assert res.returncode == 1
    assert "::error::" in res.stderr
    # The DIAGNOSIS, not just the status: without the empty-input guard the
    # half-declared-leg check downstream also returns 1, but blames a phantom
    # "leg 1" instead of saying the matrix moved. At tag time that is the
    # difference between a fixable message and a confusing one.
    assert "no build-wheels matrix legs found" in res.stderr


def test_wheel_matrix_legs_fail_path_leg_without_target(tmp_path: Path):
    """FAIL PATH: a half-declared leg must error rather than be skipped."""
    wf = tmp_path / "wf.yml"
    wf.write_text(
        "jobs:\n  build-wheels:\n    strategy:\n      matrix:\n        include:\n"
        "          - os: ubuntu-latest\n            target: x86_64\n"
        "          - os: macos-latest\n            suffix: oops\n"
        "    steps:\n      - uses: actions/checkout@v7\n"
    )
    res = call(["wheel_matrix_legs", str(wf)])
    assert res.returncode == 1
    assert "::error::" in res.stderr


@pytest.mark.parametrize(
    ("os_", "target", "pattern"),
    [
        ("ubuntu-latest", "x86_64", "*manylinux*_x86_64.whl"),
        ("ubuntu-latest", "aarch64", "*manylinux*_aarch64.whl"),
        ("macos-latest", "aarch64", "*macosx_*_arm64.whl"),
        ("macos-latest", "x86_64", "*macosx_*_x86_64.whl"),
        ("windows-latest", "x64", "*win_amd64.whl"),
    ],
)
def test_wheel_pattern_for_leg_maps_every_real_leg(os_: str, target: str, pattern: str):
    res = call(["wheel_pattern_for_leg", os_, target])
    assert res.returncode == 0, res.stderr
    assert res.stdout == pattern + "\n"


@pytest.mark.parametrize(
    ("os_", "target"),
    [
        ("ubuntu-latest", "armv7"),  # a plausible new leg
        ("ubuntu-latest", "s390x"),
        ("windows-latest", "aarch64"),
        ("freebsd-latest", "x86_64"),
        ("", ""),
    ],
)
def test_wheel_pattern_for_leg_fail_path_unmapped(os_: str, target: str):
    """FAIL PATH — THE LOAD-BEARING ONE. An unmapped leg must be an error.

    A catch-all `*) printf '*.whl'` here would re-open the whole hole: every
    unknown leg would be satisfied by any wheel at all.
    """
    res = call(["wheel_pattern_for_leg", os_, target])
    assert res.returncode == 1, f"{os_}/{target} silently got a pattern"
    assert "::error::" in res.stderr


def test_assert_artifact_set_pass_path(tmp_path: Path):
    res = call(["assert_artifact_set", str(complete_dist(tmp_path)), str(RELEASE_WORKFLOW)])
    assert res.returncode == 0, res.stderr
    assert "artifact set complete" in res.stderr


@pytest.mark.parametrize("missing", sorted(WHEELS))
def test_assert_artifact_set_fail_path_short_set(tmp_path: Path, missing: str):
    """FAIL PATH: any ONE missing wheel is red — including the ones a count-only
    check would let through when some other leg double-built."""
    names = [w for k, w in WHEELS.items() if k != missing] + [SDIST]
    res = call(["assert_artifact_set", str(make_dist(tmp_path, names)), str(RELEASE_WORKFLOW)])
    assert res.returncode == 1, f"a set missing the {missing} wheel was accepted"
    assert "::error::" in res.stderr


def test_assert_artifact_set_fail_path_empty_dist(tmp_path: Path):
    """FAIL PATH: the empty-artifact outcome `if-no-files-found: warn` allows."""
    res = call(["assert_artifact_set", str(make_dist(tmp_path, [])), str(RELEASE_WORKFLOW)])
    assert res.returncode == 1
    assert "::error::" in res.stderr


def test_assert_artifact_set_fail_path_missing_dist_dir(tmp_path: Path):
    res = call(["assert_artifact_set", str(tmp_path / "nope"), str(RELEASE_WORKFLOW)])
    assert res.returncode == 1
    assert "::error::" in res.stderr


def test_assert_artifact_set_fail_path_right_count_wrong_wheels(tmp_path: Path):
    """FAIL PATH: five wheels, but two of them are the same platform.

    A count-based gate ("assert 6 files") passes this. The bijection does not.
    """
    names = [
        WHEELS["ubuntu-latest/x86_64"],
        WHEELS["ubuntu-latest/aarch64"],
        WHEELS["macos-latest/aarch64"],
        WHEELS["macos-latest/x86_64"],
        "codingest-0.1.3-cp38-abi3-macosx_12_0_arm64.whl",  # a second macOS arm64
        SDIST,
    ]
    res = call(["assert_artifact_set", str(make_dist(tmp_path, names)), str(RELEASE_WORKFLOW)])
    assert res.returncode == 1, "a full-count but wrong-platform set was accepted"
    assert "::error::" in res.stderr


def test_assert_artifact_set_fail_path_unclaimed_wheel(tmp_path: Path):
    """FAIL PATH: an extra wheel no matrix leg asked for.

    Usually means a leg was added to the matrix but never mapped here, so the
    mapping table has silently stopped describing the build.
    """
    names = [*WHEELS.values(), "codingest-0.1.3-cp38-abi3-musllinux_1_2_x86_64.whl", SDIST]
    res = call(["assert_artifact_set", str(make_dist(tmp_path, names)), str(RELEASE_WORKFLOW)])
    assert res.returncode == 1
    assert "::error::" in res.stderr


@pytest.mark.parametrize(
    ("label", "sdists"),
    [("none", []), ("two", [SDIST, "codingest-0.1.3.post1.tar.gz"])],
)
def test_assert_artifact_set_fail_path_sdist_count(tmp_path: Path, label, sdists):
    """FAIL PATH: exactly one sdist. Zero is the `if-no-files-found: warn`
    outcome for the sdist job; two means two versions got merged into one dist.
    """
    res = call(
        ["assert_artifact_set", str(make_dist(tmp_path, [*WHEELS.values(), *sdists])),
         str(RELEASE_WORKFLOW)]
    )
    assert res.returncode == 1, f"{label} sdist(s) accepted"
    assert "::error::" in res.stderr


def test_assert_artifact_set_fail_path_added_leg_with_the_old_wheel_set(tmp_path: Path):
    """FAIL PATH — THE CENTRAL CLAIM OF THE DESIGN.

    Someone adds a 6th matrix leg for a platform the mapping already knows
    (macos-latest/x86_64 duplicated here as a same-tag leg is covered
    separately; this one uses ubuntu-latest/aarch64 on a second manylinux) and
    the wheel set stays at 5. A hardcoded `-eq 5` would still be green. The
    derived expectation goes red, because the 6th leg has nothing to claim.
    """
    wf = workflow_with_extra_leg(tmp_path, "macos-latest", "aarch64")
    res = call(["assert_artifact_set", str(complete_dist(tmp_path)), str(wf)])
    assert res.returncode == 1, "a leg was added and the short set still passed"
    assert "::error::" in res.stderr


def test_assert_artifact_set_fail_path_added_leg_is_unmapped(tmp_path: Path):
    """FAIL PATH: a new leg on a platform with no tag mapping fails EVEN IF a
    sixth wheel is present — the gate refuses to guess what the wheel should be
    called rather than accepting whatever showed up."""
    wf = workflow_with_extra_leg(tmp_path, "ubuntu-latest", "armv7")
    names = [*WHEELS.values(), "codingest-0.1.3-cp38-abi3-manylinux_2_17_armv7l.whl", SDIST]
    res = call(["assert_artifact_set", str(make_dist(tmp_path, names)), str(wf)])
    assert res.returncode == 1
    assert "no wheel-tag mapping" in res.stderr


def test_assert_artifact_set_fail_path_broken_matrix_is_not_vacuous(tmp_path: Path):
    """FAIL PATH: if the matrix cannot be read, refuse — do not pass with an
    empty expectation. Zero legs against any dist would otherwise be green."""
    wf = tmp_path / "wf.yml"
    wf.write_text("jobs:\n  other-job:\n    runs-on: ubuntu-latest\n")
    res = call(["assert_artifact_set", str(complete_dist(tmp_path)), str(wf)])
    assert res.returncode == 1
    assert "::error::" in res.stderr


# --------------------------------------------------------------------------
# INVERTED CONTROLS for the artifact-set gate
# --------------------------------------------------------------------------


def test_inverted_control_a_presence_check_would_pass_a_short_set(tmp_path: Path):
    """NAIVE: `ls -la dist` / "there is at least one wheel" — the shipped state.

    A one-wheel dist satisfies it. The real gate demands one per matrix leg.
    """
    dist = make_dist(tmp_path, [WHEELS["ubuntu-latest/x86_64"], SDIST])
    naive = run_shell(
        f'if ls -la "{dist}" >/dev/null && ls "{dist}"/*.whl >/dev/null 2>&1; then\n'
        "  echo NAIVE_PASS\nelse echo NAIVE_FAIL\nfi\n"
    )
    assert "NAIVE_PASS" in naive.stdout, "premise broken: the naive check no longer passes"
    res = call(["assert_artifact_set", str(dist), str(RELEASE_WORKFLOW)])
    assert res.returncode == 1, "the real gate must reject the short set"


def test_inverted_control_a_hardcoded_count_rots_when_a_leg_is_added(tmp_path: Path):
    """NAIVE: `[ $(ls dist/*.whl | wc -l) -eq 5 ]` — the obvious implementation.

    Add a 6th matrix leg and leave the wheel set at 5: the constant is now a
    LIE, and it stays green. This is the entire reason the expectation is
    derived from the matrix instead of written down.
    """
    dist = complete_dist(tmp_path)
    wf = workflow_with_extra_leg(tmp_path, "macos-latest", "aarch64")
    naive = run_shell(
        f'n=$(ls "{dist}"/*.whl | wc -l)\n'
        'if [ "$n" -eq 5 ]; then echo NAIVE_PASS; else echo NAIVE_FAIL; fi\n'
    )
    assert "NAIVE_PASS" in naive.stdout, "premise broken: the fixture is not 5 wheels"
    res = call(["assert_artifact_set", str(dist), str(wf)])
    assert res.returncode == 1, (
        "the real gate must notice the added leg the hardcoded count cannot"
    )


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
        'scripts/release_gates.sh changelog_notes "$version" CHANGELOG.md'
        ' /tmp/release_notes.md "$GITHUB_REF_TYPE"',
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


def test_step_block_does_not_leak_into_the_next_step(tmp_path: Path):
    """VERIFY THE PROBE: the block extractor must stop at the next step.

    If it ran on, an `if-no-files-found: error` on the wheels upload would
    satisfy the assertion made about the sdist upload three steps later.
    """
    sample = tmp_path / "s.yml"
    sample.write_text(
        "      - name: A\n        with:\n          key: 1\n"
        "      - name: B\n        with:\n          key: 2\n"
    )
    assert _step_block(sample, "A") == ["- name: A", "with:", "key: 1"]
    assert "key: 2" not in _step_block(sample, "A")


def test_step_block_strips_comments(tmp_path: Path):
    """VERIFY THE PROBE: `if-no-files-found` appears in the COMMENT explaining
    it, so an unstripped block would be satisfied by the prose alone."""
    sample = tmp_path / "s.yml"
    sample.write_text(
        "      # if-no-files-found: error is load-bearing\n"
        "      - name: A\n        with:\n          path: dist/*.whl\n"
    )
    assert _step_block(sample, "A") == ["- name: A", "with:", "path: dist/*.whl"]


@pytest.mark.parametrize("step", ["Upload wheels", "Upload sdist"])
def test_both_uploads_error_on_no_files(step: str):
    """actions/upload-artifact defaults `if-no-files-found` to `warn`.

    A build job that exits 0 having produced no wheel then uploads an EMPTY
    artifact and stays green, and `skip-existing: true` ships the partial set to
    PyPI without a word. Checked PER STEP, not by counting: two copies on one
    upload must not cover for a missing one on the other.
    """
    assert "if-no-files-found: error" in _step_block(RELEASE_WORKFLOW, step), (
        f"release.yml step {step!r} lost `if-no-files-found: error` — an empty "
        "artifact would upload green again"
    )


def test_release_workflow_verifies_the_artifact_set():
    """The real assertion that replaced `ls -la dist`."""
    assert_has_line(
        RELEASE_WORKFLOW,
        "scripts/release_gates.sh assert_artifact_set dist .github/workflows/release.yml",
    )


def test_release_workflow_no_longer_fakes_the_artifact_check():
    """`ls -la dist` is verification-shaped and asserts nothing — on a directory
    `download-artifact` always creates, it cannot fail."""
    assert "ls -la dist" not in _code_lines(RELEASE_WORKFLOW)


def test_sdist_license_step_keeps_its_explicit_shell_bash():
    """An INVISIBLE DEPENDENCY, made visible and then enforced.

    `shell: bash` upgrades GitHub's default `bash -e {0}` to
    `bash --noprofile --norc -eo pipefail {0}`. The delta is pipefail, and this
    step is a pipeline (`tar | grep -Ec`) whose PRODUCER can fail while its
    consumer succeeds: a tar that dies partway through a corrupt archive after
    listing a LICENSE entry leaves grep at rc 0 and count 1, so the step is
    green on a broken sdist. The comment above the step records the measured
    behaviour; this test is what stops the line being deleted as noise.
    """
    assert "shell: bash" in _step_block(RELEASE_WORKFLOW, "Verify sdist license payload")


# --------------------------------------------------------------------------
# Phase 5 wiring: the trigger, the ref guard, the notes fallback, the race
# --------------------------------------------------------------------------


def test_job_block_does_not_leak_into_the_next_job(tmp_path: Path):
    """VERIFY THE PROBE: the job extractor must stop at the next job header.

    If it ran on, the `Guard the publish ref` step in `publish-crate` would
    satisfy the assertion made about `release-binaries` two jobs later — and the
    per-job scoping below would be worth nothing.
    """
    sample = tmp_path / "s.yml"
    sample.write_text(
        "jobs:\n  a:\n    needs: [x]\n    steps:\n      - name: A\n"
        "  # a comment between jobs\n  b:\n    needs: [y]\n"
    )
    assert _job_block(sample, "a") == ["needs: [x]", "steps:", "- name: A"]
    assert _job_block(sample, "b") == ["needs: [y]"]


def test_release_workflow_only_triggers_on_a_version_tag():
    """`workflow_dispatch:` IS GONE — the primary block on the dispatch path.

    On dispatch GITHUB_REF_NAME is a BRANCH, so `${GITHUB_REF_NAME#v}` is
    `main`: the changelog lookup asks for `## [main]`, the binaries are named
    `codingest-main-…`, and softprops creates a tag+release named after the
    branch. Asserted as the exact shape of the trigger block, not as "the string
    is absent", so re-adding the trigger anywhere in it goes red.
    """
    lines = _code_lines(RELEASE_WORKFLOW)
    start = lines.index("on:")
    assert lines[start : start + 4] == ["on:", "push:", "tags:", "- 'v*'"]
    assert lines[start + 4] == "env:", (
        f"an extra trigger was added to release.yml: {lines[start + 4]!r}"
    )


GUARD_CALL = 'scripts/release_gates.sh assert_publish_ref \\'
GUARD_ARGS = '"$GITHUB_REF_NAME" "$GITHUB_REF_TYPE" "$GITHUB_EVENT_NAME"'


@pytest.mark.parametrize("job", ["publish-crate", "publish-pypi", "release-binaries"])
def test_every_publishing_job_guards_the_ref(job: str):
    """The backstop, checked PER JOB.

    Three jobs carry an identically-named guard step, so a file-wide count would
    be satisfied by three copies in one job and none in the others. Whole-line
    equality (not `in`) keeps a neutered `… --dry-run` from passing.
    """
    block = _job_block(RELEASE_WORKFLOW, job)
    assert GUARD_CALL in block, f"job {job!r} does not run assert_publish_ref"
    assert GUARD_ARGS in block, f"job {job!r} calls the guard without the ref"


def _job_steps(path: Path, job: str) -> list[str]:
    """The step headers of one job, in order.

    Anchored on `steps:` because a job's `strategy.matrix.include` entries are
    also `- ` lines and sit ABOVE it.
    """
    block = _job_block(path, job)
    return [l for l in block[block.index("steps:") + 1 :] if l.startswith("- ")]


@pytest.mark.parametrize(
    ("job", "dangerous"),
    [
        ("publish-crate", "- name: Publish codingest"),
        ("publish-pypi", "- name: Publish to PyPI (trusted publishing)"),
        ("release-binaries", "- name: Build codingest-cli + codingest-mcp (release)"),
    ],
)
def test_the_ref_guard_is_the_first_step_after_checkout(job: str, dangerous: str):
    """POSITION. A guard that runs after `cargo publish` guards nothing.

    Asserted as "immediately after checkout" rather than "somewhere before the
    dangerous step". The weaker form was a real hole, found by mutation: moving
    the guard down one step left it still ahead of the build and the assertion
    stayed GREEN, so the guard could drift arbitrarily far down the job one
    harmless-looking step at a time. Immediately-after-checkout is a position
    that cannot drift, and checkout has to come first because the guard is a
    script in the repo.

    The `dangerous` leg is kept alongside it so the fixture cannot rot into
    vacuity: it names the step whose lateness is the actual consequence, and
    fails loudly if that step is renamed or moved out of the job.
    """
    steps = _job_steps(RELEASE_WORKFLOW, job)
    assert steps[:2] == ["- uses: actions/checkout@v7", "- name: Guard the publish ref"], (
        f"job {job!r} does not guard the ref immediately after checkout: {steps[:3]}"
    )
    assert steps.index("- name: Guard the publish ref") < steps.index(dangerous)


def test_release_workflow_has_no_auto_generated_notes_fallback():
    """The silent downgrade is REMOVED, not merely unreachable.

    A missing changelog section is now fatal on a tag, so this branch could
    never fire — but leaving a degrade-quietly step in the file is an invitation
    to restore the behaviour by relaxing the gate.
    """
    code = _code_lines(RELEASE_WORKFLOW)
    assert "generate_release_notes: true" not in code
    assert "if: steps.changelog.outputs.has_notes != 'true'" not in code
    assert code.count("uses: softprops/action-gh-release@v3") == 2, (
        "expected exactly two release actions: the notes one in publish-pypi and "
        "the attach one in release-binaries"
    )


def test_the_changelog_gate_runs_before_the_pypi_publish():
    """A fatal check belongs on the near side of an irreversible action.

    PyPI accepts a given filename exactly once; failing the changelog gate after
    the upload leaves a published version with no release notes and no way to
    re-upload.
    """
    block = _job_block(RELEASE_WORKFLOW, "publish-pypi")
    assert block.index("- name: Extract changelog for this version") < block.index(
        "- name: Publish to PyPI (trusted publishing)"
    )


def test_release_binaries_waits_for_publish_pypi():
    """THE RELEASE-CREATION RACE. Both this job and publish-pypi call
    softprops/action-gh-release, and whichever arrives first CREATES the
    release. With `needs: [publish-crate]` only, this job habitually won and
    created it with an EMPTY body.
    """
    assert "needs: [publish-crate, publish-pypi]" in _job_block(
        RELEASE_WORKFLOW, "release-binaries"
    )


def test_no_job_depends_on_release_binaries():
    """WHY THE NEW `needs:` EDGE CANNOT MAKE THIS JOB BLOCK A PUBLISH.

    `needs:` makes the DECLARING job wait; blocking would require the reverse
    edge — some other job declaring `needs: release-binaries`. None does, which
    is what actually makes this best-effort job unable to gate the crates.io or
    PyPI publish (the dependency graph, with `continue-on-error` a distant
    second). This test is what stops that reverse edge appearing later.
    """
    needs = [l for l in _code_lines(RELEASE_WORKFLOW) if l.startswith("needs:")]
    assert len(needs) >= 4, f"premise broken: only {len(needs)} needs: lines found"
    for line in needs:
        assert "release-binaries" not in line, (
            f"a job now waits on the best-effort binaries job: {line!r} — it can "
            "block the publish path"
        )


# --------------------------------------------------------------------------
# Phase 6: `continue-on-error` narrowed from JOB scope to three steps
# --------------------------------------------------------------------------
#
# WHAT WAS BROKEN. `release-binaries` carried `continue-on-error: true` at JOB
# level, under a comment justifying it as "a packaging hiccup must not block the
# crates.io/PyPI publish". That intent is legitimate — but it is ALREADY
# satisfied by the dependency graph (`test_no_job_depends_on_release_binaries`:
# nothing declares `needs: release-binaries`, so this job cannot block anything
# whatever the flag says). What the job-level flag bought on top of that was
# purely negative: every assertion in the job — including the
# `Guard the publish ref` step that stops archives being named after a branch —
# became unable to fail, from a line sitting above the steps where a reader
# misses it. Upstream reported this exact pattern holding a job green for two
# days, with two false claims committed on the strength of that green.
#
# THE SUBSTRING-SUBSUMPTION TRAP IS SHARP HERE. `continue-on-error: true` is
# byte-identical at job scope and step scope; only INDENTATION distinguishes the
# fixed file from the broken one. A `"continue-on-error: true" in text` check —
# or anything built on `_code_lines`/`_job_block`, both of which strip
# indentation — is green either way. Hence `_job_keys` below, and the inverted
# control that shows the naive form staying green on the broken file.

FRAGILE_STEPS = [
    "Package binaries (unix)",
    "Package binaries (windows)",
    "Attach to the GitHub Release",
]

# Steps whose failure is a real defect, not a hiccup. `cargo build --release` is
# the build ci.yml already runs on every push; the ref guard is an assertion,
# and an assertion that cannot fail is not one.
MUST_FAIL_STEPS = [
    "Guard the publish ref",
    "Build codingest-cli + codingest-mcp (release)",
]

COE = "continue-on-error"


def _coe(lines: list[str]) -> list[str]:
    """Every `continue-on-error` declaration in `lines`, whatever its value.

    MATCH THE KEY, NOT THE LITERAL. Asserting on the exact text
    `continue-on-error: true` was a hole, found by mutation: the key takes an
    EXPRESSION, so `continue-on-error: ${{ github.event_name == 'push' }}` is
    unfailable on every real run of this workflow while never writing the string
    being matched on. Both the job-scope and the release-build mutations came
    back GREEN against the literal form.
    """
    return [l for l in lines if l.split(":", 1)[0].strip() == COE]


def _job_keys(path: Path, job: str) -> list[str]:
    """The JOB-LEVEL keys of one job — indent exactly 4, nothing nested.

    This is the only helper in the file that can tell `continue-on-error: true`
    on the job apart from the same text on a step, because it is the only one
    that reads indentation instead of stripping it. `_job_block` deliberately
    strips (it compares whole lines across nesting levels); reusing it here
    would make every assertion below satisfiable by the step-level copies.
    """
    raw = path.read_text().splitlines()
    start = next((i for i, l in enumerate(raw) if l == f"  {job}:"), None)
    assert start is not None, f"{path.name}: no job named {job!r}"
    out = []
    for line in raw[start + 1 :]:
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if not line.startswith("    "):  # back out to indent 2 -> next job
            break
        if line.startswith("     "):  # indent > 4 -> nested under a job key
            continue
        out.append(stripped)
    return out


def _job_step_block(path: Path, job: str, step_name: str) -> list[str]:
    """`_step_block`, but scoped to one job.

    `_step_block` searches the whole file and takes the FIRST match, which for
    `Guard the publish ref` is publish-crate's copy — three jobs carry a step
    with that name.
    """
    lines = _job_block(path, job)
    start = next((i for i, l in enumerate(lines) if l == f"- name: {step_name}"), None)
    assert start is not None, f"{path.name}: job {job!r} has no step {step_name!r}"
    block = [lines[start]]
    for line in lines[start + 1 :]:
        if line.startswith("- "):
            break
        block.append(line)
    return block


def test_job_keys_separates_job_scope_from_step_scope(tmp_path: Path):
    """VERIFY THE PROBE — the one that makes this whole section non-vacuous.

    Two jobs with IDENTICAL text at different indents. The helper must see the
    job-level flag in `a` and must NOT see the step-level flag in `b`; if it
    saw both, every assertion below would pass on the broken file.
    """
    sample = tmp_path / "s.yml"
    sample.write_text(
        "jobs:\n"
        "  a:\n"
        "    continue-on-error: true\n"
        "    permissions:\n"
        "      contents: write\n"
        "    steps:\n"
        "      - name: A\n"
        "  b:\n"
        "    steps:\n"
        "      - name: B\n"
        "        continue-on-error: true\n"
    )
    assert _job_keys(sample, "a") == ["continue-on-error: true", "permissions:", "steps:"]
    assert _job_keys(sample, "b") == ["steps:"]
    # And the scope-blind helpers genuinely cannot tell them apart — which is
    # why `_job_keys` exists rather than being a stylistic preference.
    assert "continue-on-error: true" in _job_block(sample, "a")
    assert "continue-on-error: true" in _job_block(sample, "b")


def test_job_step_block_is_scoped_to_its_job(tmp_path: Path):
    """VERIFY THE PROBE: a same-named step in an earlier job must not answer for
    the one being asserted on."""
    sample = tmp_path / "s.yml"
    sample.write_text(
        "jobs:\n"
        "  a:\n"
        "    steps:\n"
        "      - name: Guard\n"
        "        continue-on-error: true\n"
        "  b:\n"
        "    steps:\n"
        "      - name: Guard\n"
        "        run: real\n"
    )
    assert _job_step_block(sample, "b", "Guard") == ["- name: Guard", "run: real"]
    assert "continue-on-error: true" not in _job_step_block(sample, "b", "Guard")


def test_release_binaries_has_no_job_level_continue_on_error():
    """THE PHASE 6 GATE. Job-scope tolerance makes every check inside the job —
    the ref guard, the release build — unable to fail, and buys nothing the
    `needs:` graph does not already provide (see
    `test_no_job_depends_on_release_binaries`).
    """
    keys = _coe(_job_keys(RELEASE_WORKFLOW, "release-binaries"))
    assert keys == [], (
        "release-binaries carries JOB-LEVEL continue-on-error again — every "
        f"assertion in the job, including the publish-ref guard, is now "
        f"unfailable: {keys}"
    )


@pytest.mark.parametrize("step", FRAGILE_STEPS)
def test_the_fragile_steps_stay_best_effort(step: str):
    """The documented intent is PRESERVED, not deleted: archive plumbing and the
    releases API are genuinely fragile and run after both publishes are done."""
    assert "continue-on-error: true" in _job_step_block(
        RELEASE_WORKFLOW, "release-binaries", step
    ), f"step {step!r} lost its best-effort tolerance — a packaging hiccup now fails"


@pytest.mark.parametrize("step", MUST_FAIL_STEPS)
def test_the_real_checks_in_release_binaries_can_fail(step: str):
    """The other half: tolerance must not creep back onto a step that asserts
    something. `cargo build --release` breaking at tag time is a regression.

    An explicit `continue-on-error: false` is allowed (it is the default said out
    loud); anything else — literal or expression — is not.
    """
    found = [
        l
        for l in _coe(_job_step_block(RELEASE_WORKFLOW, "release-binaries", step))
        if l != f"{COE}: false"
    ]
    assert found == [], f"step {step!r} was made unfailable: {found}"


def test_release_binaries_tolerates_exactly_the_three_fragile_steps():
    """A COUNT, to catch what per-step assertions cannot.

    Three of release-binaries' eight steps are unnamed `- uses:` steps
    (checkout, toolchain, cache) that `_job_step_block` cannot address by name.
    Pinning the total to three — with the parametrised test above proving WHICH
    three — is what stops tolerance appearing on one of those.
    """
    declared = _coe(_job_block(RELEASE_WORKFLOW, "release-binaries"))
    assert declared == [f"{COE}: true"] * len(FRAGILE_STEPS), (
        "release-binaries tolerates a different set of failures than the "
        f"{len(FRAGILE_STEPS)} fragile steps: {declared}"
    )


def test_release_binaries_keeps_fail_fast_false():
    """WITHOUT THE JOB-LEVEL FLAG THIS LINE BECAME LOAD-BEARING.

    `fail-fast` defaults to TRUE. Previously no leg could go red so it never
    fired; now a leg can (broken build, bad ref), and its removal would let one
    failing platform CANCEL the other two mid-package — one real failure turned
    into three missing archives, reported as cancellation rather than failure.
    Asserted as the exact line, so `fail-fast: true` is caught too.
    """
    assert "fail-fast: false" in _job_block(RELEASE_WORKFLOW, "release-binaries"), (
        "release-binaries lost `fail-fast: false`; one red leg now cancels the "
        "other platforms mid-package"
    )


def test_the_attach_step_is_guarded_against_an_empty_out():
    """The one NEW hole step-level tolerance opens, closed.

    A hard failure skips the rest of the job, so a broken build never reaches
    the attach step. A TOLERATED packaging failure does not: the job carries on
    with `out/` empty or absent. `success()` is no help — a
    `continue-on-error` step's conclusion is success — so the guard has to ask
    about the files themselves. Without it softprops is handed an empty glob and
    updates the release with nothing, indistinguishably from a real attach.
    """
    block = _job_step_block(RELEASE_WORKFLOW, "release-binaries", "Attach to the GitHub Release")
    assert "if: hashFiles('out/*') != ''" in block, (
        "the attach step lost its empty-`out/` guard; after a tolerated "
        f"packaging failure it will attach nothing, quietly: {block}"
    )


def test_inverted_control_a_scope_blind_flag_check_cannot_tell_job_from_step(
    tmp_path: Path,
):
    """INVERTED CONTROL: the naive assertion stays GREEN on the broken file.

    `"continue-on-error: true" in text` — the obvious way to write this — is
    satisfied by the fixed file AND by the file with the flag moved back to job
    level, because the two differ only in indentation. So is anything built on
    `_code_lines`/`_job_block`, which strip it. That is precisely the
    substring-subsumption trap, and it is why the real assertion reads scope.
    """
    real = RELEASE_WORKFLOW.read_text()
    broken = real.replace(
        "    needs: [publish-crate, publish-pypi]\n",
        "    needs: [publish-crate, publish-pypi]\n    continue-on-error: true\n",
    ).replace("        continue-on-error: true\n", "")
    assert broken != real, "premise broken: the mutation did not change the file"
    wf = tmp_path / "release.yml"
    wf.write_text(broken)

    # NAIVE forms — green on both, therefore worthless.
    for text in (real, broken):
        assert "continue-on-error: true" in text
    assert "continue-on-error: true" in _job_block(wf, "release-binaries")
    assert "continue-on-error: true" in _job_block(RELEASE_WORKFLOW, "release-binaries")

    # The real assertion separates them.
    assert _coe(_job_keys(RELEASE_WORKFLOW, "release-binaries")) == []
    assert _coe(_job_keys(wf, "release-binaries")) == ["continue-on-error: true"], (
        "premise broken: the job-scope probe no longer sees a job-level flag"
    )


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
