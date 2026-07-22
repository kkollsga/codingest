"""Console-script shim for the Rust ``codingest-mcp`` server in the wheel.

Codingest supplies the source-builder hooks; KGlite supplies graph/Cypher
serving and pulls the generic MCP lifecycle from ``mcp-methods``. All three are
linked into the wheel's existing native extension, so users need only
``pip install codingest``.
"""

from __future__ import annotations

import json
import os
import sys


def main(argv: list[str] | None = None) -> int:
    """Run the bundled MCP server with ``argv`` or ``sys.argv[1:]``."""
    from codingest.codingest import _run_mcp_server

    args = list(sys.argv[1:] if argv is None else argv)
    # In a wheel console script, current_exe() is Python rather than the Rust
    # server. Give KGlite's selftest the exact command needed to re-enter this
    # module with the same interpreter and environment.
    os.environ["KGLITE_MCP_RESPAWN"] = json.dumps(
        [sys.executable, "-m", "codingest.mcp_server"]
    )
    try:
        _run_mcp_server(args)
    except KeyboardInterrupt:
        return 130
    except RuntimeError as exc:
        print(f"codingest-mcp: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
