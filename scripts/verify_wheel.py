"""Verify that a built Codingest wheel fulfills the pip installation contract."""

from __future__ import annotations

import configparser
import glob
import sys
import zipfile
from email.parser import BytesParser
from pathlib import Path


def verify(pattern: str) -> None:
    matches = [Path(path) for path in glob.glob(pattern)]
    if len(matches) != 1:
        raise SystemExit(f"expected one wheel matching {pattern!r}, found {matches}")

    wheel = matches[0]
    with zipfile.ZipFile(wheel) as archive:
        names = archive.namelist()
        entry_name = next(
            (name for name in names if name.endswith(".dist-info/entry_points.txt")),
            None,
        )
        metadata_name = next(
            (name for name in names if name.endswith(".dist-info/METADATA")),
            None,
        )
        if entry_name is None or metadata_name is None:
            raise SystemExit(f"{wheel}: missing wheel metadata")

        entries = configparser.ConfigParser()
        entries.read_string(archive.read(entry_name).decode())
        scripts = dict(entries["console_scripts"])
        expected = {
            "codingest": "codingest.cli:main",
            "codingest-mcp": "codingest.mcp_server:main",
        }
        if scripts != expected:
            raise SystemExit(f"{wheel}: console scripts {scripts!r} != {expected!r}")

        required = {"codingest/cli.py", "codingest/mcp_server.py"}
        missing = required.difference(names)
        if missing:
            raise SystemExit(f"{wheel}: missing payload files {sorted(missing)!r}")
        if not any(
            name.startswith("codingest/codingest")
            and (name.endswith(".so") or name.endswith(".pyd"))
            for name in names
        ):
            raise SystemExit(f"{wheel}: missing native Codingest extension")

        metadata = BytesParser().parsebytes(archive.read(metadata_name))
        requirements = metadata.get_all("Requires-Dist", [])
        if not any(requirement.lower().startswith("kglite") for requirement in requirements):
            raise SystemExit(f"{wheel}: missing KGlite runtime dependency")
        if any(requirement.lower().startswith("mcp-methods") for requirement in requirements):
            raise SystemExit(f"{wheel}: must not duplicate mcp-methods as a Python dependency")

    print(f"verified pip contract: {wheel}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: verify_wheel.py '<wheel-glob>'")
    verify(sys.argv[1])
