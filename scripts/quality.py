#!/usr/bin/env python3
"""Release quality gate.

Runs the in-house suites that the plain release pipeline used to skip:

1. formatting and Clippy correctness lints;
2. the whole cargo workspace (unit, integration, oracles, digests, persistence corpus);
3. the release tool's own pytest suite;
4. the ignored sparse-16-bit ratio gate (asserts the 0.60 budget);
5. the ignored long-session soak (asserts bounded memory and eviction integrity).

The ignored gates are measurements with assertions, so they are run explicitly here rather than
left to a manual checklist. They are intentionally last: a broken unit test fails fast.

Usage: python scripts/quality.py
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

STEPS: list[tuple[str, list[str]]] = [
    ("formatting", ["cargo", "fmt", "--all", "--", "--check"]),
    (
        "clippy correctness",
        [
            "cargo",
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "clippy::correctness",
        ],
    ),
    ("cargo workspace (oracles, digests, persistence)", ["cargo", "test", "--release", "--workspace"]),
    ("release tool (pytest)", [sys.executable, "-m", "pytest", "tools/release/tests", "-q"]),
    (
        "sparse 16-bit ratio gate (ignored)",
        [
            "cargo",
            "test",
            "--release",
            "-p",
            "aurelia-core",
            "gate_2_throughput_and_memory",
            "--",
            "--ignored",
            "--nocapture",
        ],
    ),
    (
        "long-session soak (ignored)",
        [
            "cargo",
            "test",
            "--release",
            "-p",
            "aurelia-io",
            "--test",
            "soak",
            "--",
            "--ignored",
            "--nocapture",
        ],
    ),
]


def main() -> int:
    for label, command in STEPS:
        print(f"== quality: {label} ==", flush=True)
        print("$ " + " ".join(command), flush=True)
        result = subprocess.run(command, cwd=ROOT)
        if result.returncode != 0:
            print(f"quality gate failed: {label}", file=sys.stderr)
            return result.returncode
    print("quality: all in-house suites passed", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
