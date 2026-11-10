"""Load local release secrets from a gitignored `.env` file."""

from __future__ import annotations

import os
import re
from pathlib import Path
from typing import MutableMapping

_KEY_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


def _parse_line(line: str) -> tuple[str, str] | None:
    line = line.strip()
    if not line or line.startswith("#"):
        return None
    if line.startswith("export "):
        line = line[len("export ") :].lstrip()
    key, separator, value = line.partition("=")
    key = key.strip()
    if not separator or not _KEY_RE.match(key):
        return None
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in {"'", '"'}:
        value = value[1:-1]
    return key, value


def load_env_file(
    path: Path, environ: MutableMapping[str, str] | None = None
) -> dict[str, str]:
    """Load `KEY=value` pairs without overriding variables already in the environment.

    The file is deliberately optional: release machines may set secrets in the process
    environment instead. Values are never printed or logged here.
    """
    target = os.environ if environ is None else environ
    loaded: dict[str, str] = {}
    if not path.is_file():
        return loaded
    for raw in path.read_text(encoding="utf-8-sig").splitlines():
        parsed = _parse_line(raw)
        if parsed is None:
            continue
        key, value = parsed
        if key in target:
            continue
        target[key] = value
        loaded[key] = value
    return loaded
