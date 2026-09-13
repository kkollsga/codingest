"""Tests for the thin Python launchers bundled in the wheel."""

from __future__ import annotations

import importlib.metadata
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import sysconfig

import codingest
import codingest.codingest as native
import pytest
from codingest import mcp_server


def _installed_script(name: str) -> Path:
    suffix = ".exe" if os.name == "nt" else ""
    return Path(sysconfig.get_path("scripts")) / f"{name}{suffix}"


def _run(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, capture_output=True, text=True, check=False, **kwargs)


def _documented_command(name: str) -> list[str]:
    reference = (
        Path(__file__).parents[2]
        / "skills/codingest-code-review/references/mcp-upgrade.md"
    ).read_text()
    marked = reference.split(f"<!-- example: {name} -->", 1)[1]
    fence = marked.split("```console\n", 1)[1].split("\n```", 1)[0]
    return shlex.split(" ".join(line.rstrip("\\") for line in fence.splitlines()))


def test_mcp_launcher_forwards_argv_and_sets_respawn(monkeypatch) -> None:
    calls: list[list[str]] = []
    monkeypatch.setattr(native, "_run_mcp_server", lambda argv: calls.append(argv))

    assert mcp_server.main(["--watch", "/tmp/project"]) == 0
    assert calls == [["--watch", "/tmp/project"]]
    assert json.loads(mcp_server.os.environ["KGLITE_MCP_RESPAWN"]) == [
        sys.executable,
        "-m",
        "codingest.mcp_server",
    ]


def test_mcp_launcher_formats_native_errors(monkeypatch, capsys) -> None:
    def fail(_argv: list[str]) -> None:
        raise RuntimeError("boom")

    monkeypatch.setattr(native, "_run_mcp_server", fail)

    assert mcp_server.main([]) == 1
    assert capsys.readouterr().err == "codingest-mcp: boom\n"


def test_installed_console_scripts_dispatch_packaged_entry_points() -> None:
    declared = {
        entry.name: entry.value
        for entry in importlib.metadata.entry_points(group="console_scripts")
        if entry.name in {"codingest", "codingest-mcp"}
    }
    assert declared == {
        "codingest": "codingest.cli:main",
        "codingest-mcp": "codingest.mcp_server:main",
    }

    for name, arguments, expected in (
        ("codingest", ["--version"], "codingest "),
        ("codingest-mcp", ["--help"], "Usage: codingest-mcp"),
    ):
        executable = _installed_script(name)
        assert executable.is_file(), f"missing installed script: {executable}"
        completed = _run([str(executable), *arguments])
        assert completed.returncode == 0, completed.stderr
        assert expected in completed.stdout


def test_final_wheel_console_installs_exact_skill_for_both_hosts(
    tmp_path: Path,
) -> None:
    executable = _installed_script("codingest")
    canonical = Path(__file__).parents[2] / "skills/codingest-code-review"
    expected = {
        path.relative_to(canonical): path.read_bytes()
        for path in sorted(canonical.rglob("*"))
        if path.is_file()
    }
    assert set(expected) == {
        Path("SKILL.md"),
        Path("references/queries.md"),
        Path("references/public-repositories.md"),
        Path("references/mcp-upgrade.md"),
    }

    command = [
        str(executable),
        "skill",
        "install",
        "--project",
        "--host",
        "codex",
        "--host",
        "claude",
    ]
    installed = _run(command, cwd=tmp_path)
    assert installed.returncode == 0, installed.stderr
    destinations = [
        tmp_path / ".codex/skills/codingest-code-review",
        tmp_path / ".claude/skills/codingest-code-review",
    ]

    def assert_exact_bundle(destination: Path) -> None:
        actual = {
            path.relative_to(destination): path.read_bytes()
            for path in sorted(destination.rglob("*"))
            if path.is_file() and path.name != ".codingest-managed"
        }
        assert actual == expected
        assert (destination / ".codingest-managed").read_text() == (
            importlib.metadata.version("codingest")
        )

    for destination in destinations:
        assert_exact_bundle(destination)

    for destination in destinations:
        (destination / "references/mcp-upgrade.md").write_bytes(b"stale\n")
    updated = _run(command, cwd=tmp_path)
    assert updated.returncode == 0, updated.stderr
    for destination in destinations:
        assert_exact_bundle(destination)


def test_documented_optional_kglite_agent_cli_expands_retained_result(
    sample_tree: Path,
    tmp_path: Path,
) -> None:
    executable = _installed_script("kglite")
    if not executable.is_file():
        pytest.skip("optional kglite CLI unavailable; agent example untested")

    installed = importlib.metadata.version("kglite")
    release = tuple(map(int, installed.split(".")[:3]))
    assert (0, 17, 4) <= release < (0, 18), (
        f"kglite {installed} is outside the documented agent-response line"
    )
    version_run = _run([str(executable), "--version"])
    assert version_run.returncode == 0, version_run.stderr
    assert version_run.stdout.strip() == f"kglite {installed}"

    graph_path = tmp_path / "agent-example.kgl"
    built = codingest.build(str(sample_tree), save_to=str(graph_path))
    del built
    graph_identity = f"Graph: {graph_path}"
    environment = os.environ.copy()
    environment["KGLITE_AGENT_CACHE_DIR"] = str(tmp_path / "agent-cache")
    documented = _documented_command("kglite-agent-query")
    assert documented[0] == "kglite"
    documented[2] = str(graph_path)
    documented_run = _run(
        [str(executable), *documented[1:]],
        env=environment,
    )
    assert documented_run.returncode == 0, documented_run.stderr
    documented_result = json.loads(documented_run.stdout)
    assert documented_result["isError"] is False

    value = "bounded-agent-evidence-" * 2_000
    query = f"RETURN '{value}' AS text"
    initial = _run(
        [
            str(executable),
            "query",
            str(graph_path),
            query,
            "--format",
            "agent",
            "--response-max-bytes",
            "4096",
        ],
        env=environment,
    )
    assert initial.returncode == 0, initial.stderr
    assert len(initial.stdout.encode()) <= 4097
    result = json.loads(initial.stdout)
    body = json.loads(result["content"][0]["text"])
    budget = body["response_budget"]
    assert budget["complete"] is False
    target = next(
        item
        for item in budget["domain_commands"]
        if item["json_pointer"] == "/rows/0/0"
    )
    advertised = shlex.split(target["command"])
    assert advertised[0] == "kglite"

    graph_path.unlink()
    later_cwd = tmp_path / "later-process"
    later_cwd.mkdir()
    expanded = _run(
        [str(executable), *advertised[1:]],
        cwd=later_cwd,
        env=environment,
    )
    assert expanded.returncode == 0, expanded.stderr
    expanded_result = json.loads(expanded.stdout)
    expanded_body = json.loads(expanded_result["content"][0]["text"])
    preview = expanded_body["response_budget"]["preview"]
    offset = preview["offset"]
    end = preview["end"]
    assert preview["excerpt"] == value[offset:end]

    full_advertised = shlex.split(budget["next"]["full_result"])
    assert full_advertised[0] == "kglite"
    retained = _run(
        [str(executable), *full_advertised[1:]],
        cwd=later_cwd,
        env=environment,
    )
    assert retained.returncode == 0, retained.stderr
    retained_result = json.loads(retained.stdout)
    assert retained_result["structuredContent"]["rows"] == [[value]]
    assert (
        retained_result["structuredContent"]["identity"]["footer"]
        == graph_identity
    )
