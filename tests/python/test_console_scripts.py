"""Tests for the thin Python launchers bundled in the wheel."""

from __future__ import annotations

import json
import sys

import codingest.codingest as native
from codingest import mcp_server


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
