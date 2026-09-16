"""The installed wheel's ``codingest-mcp`` serves codingest's methodology.

The Rust response-contract suite proves this on the cargo-built binary; this
drives the *installed console script* over stdio with no manifest at all —
the shape a ``pip install codingest`` user runs — and checks that the
producer layer (kglite 0.17.7 ``with_skills`` / ``with_recipes``) is served:
the ``code_review`` skill through the lazy loader, the recipe routes, and the
pointer in the tool descriptions that tells an agent to load the skill first.
"""

from __future__ import annotations

import json
import os
import subprocess
import sysconfig
from pathlib import Path
from typing import Any


def _installed_script(name: str) -> Path:
    suffix = ".exe" if os.name == "nt" else ""
    return Path(sysconfig.get_path("scripts")) / f"{name}{suffix}"


class _Stdio:
    """A minimal JSON-RPC client over the server's stdin/stdout."""

    def __init__(self, watch_root: Path) -> None:
        self.process = subprocess.Popen(
            [str(_installed_script("codingest-mcp")), "--watch", str(watch_root)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        self.next_id = 0

    def _send(self, frame: dict[str, Any]) -> None:
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps(frame) + "\n")
        self.process.stdin.flush()

    def request(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        self.next_id += 1
        frame_id = self.next_id
        self._send({"jsonrpc": "2.0", "id": frame_id, "method": method, "params": params})
        assert self.process.stdout is not None
        while True:
            line = self.process.stdout.readline()
            if not line:
                stderr = self.process.stderr.read() if self.process.stderr else ""
                raise AssertionError(f"server closed during {method}: {stderr[-2000:]}")
            frame = json.loads(line)
            if frame.get("id") == frame_id:
                assert "error" not in frame, f"{method} failed: {frame}"
                return frame["result"]
            # Notifications (tools/list_changed, …) are not replies.

    def notify(self, method: str) -> None:
        self._send({"jsonrpc": "2.0", "method": method})

    def call(self, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        return self.request("tools/call", {"name": name, "arguments": arguments})

    def close(self) -> str:
        # communicate() closes stdin itself; the server exits on EOF.
        _, stderr = self.process.communicate(timeout=30)
        return stderr


def test_installed_mcp_serves_the_code_review_skill_and_recipes(sample_tree: Path) -> None:
    rpc = _Stdio(sample_tree)
    try:
        info = rpc.request(
            "initialize",
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "codingest-tests", "version": "0"},
            },
        )
        # No manifest: the server names itself after the mode, not the producer.
        assert "version" in info["serverInfo"], info
        rpc.notify("notifications/initialized")

        tools = {tool["name"]: tool for tool in rpc.request("tools/list", {})["tools"]}
        for name in ("run_recipe_query", "list_recipe_queries", "skill"):
            assert name in tools, f"{name} missing from tools/list: {sorted(tools)}"
        for name in ("cypher_query", "run_recipe_query", "read_code_source"):
            assert 'skill("code_review")' in tools[name]["description"], name

        body = rpc.call("skill", {"name": "code_review"})
        assert body.get("isError") is not True, body
        text = body["content"][0]["text"]
        assert "# Reviewing a codingest code graph" in text
        assert "`code_review/target_coverage`" in text

        catalogue = rpc.call("list_recipe_queries", {})
        recipes = catalogue["structuredContent"]["recipes"]
        assert [(r["name"], r["query_count"]) for r in recipes] == [("code_review", 7)]
    finally:
        stderr = rpc.close()
    # The boot summary attributes the layer; it is the only surface that does.
    assert "producer skills: 1 served" in stderr, stderr[-2000:]
    assert "producer recipes: 7 served" in stderr, stderr[-2000:]
