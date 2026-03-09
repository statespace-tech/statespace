#!/usr/bin/env python3

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

if sys.version_info < (3, 11):
    print("Error: Python 3.11+ is required for docs nav validation", file=sys.stderr)
    sys.exit(1)

import tomllib


def iter_nav_targets(node: Any):
    if isinstance(node, dict):
        for value in node.values():
            yield from iter_nav_targets(value)
        return

    if isinstance(node, list):
        for item in node:
            yield from iter_nav_targets(item)
        return

    if isinstance(node, str):
        yield node


def main() -> int:
    repo_root = Path(__file__).resolve().parent.parent
    config_path = repo_root / "zensical.toml"
    docs_root = repo_root / "docs"

    config = tomllib.loads(config_path.read_text(encoding="utf-8"))
    nav = config.get("project", {}).get("nav", [])

    missing: list[str] = []
    for target in iter_nav_targets(nav):
        if target.startswith(("http://", "https://", "mailto:")):
            continue

        doc_path = target.lstrip("/").split("#", 1)[0]
        if not doc_path:
            continue
        if not (docs_root / doc_path).is_file():
            missing.append(target)

    if missing:
        print("Missing docs pages referenced in zensical.toml nav:")
        for target in missing:
            print(f"- {target}")
        return 1

    print("Docs nav validation passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
