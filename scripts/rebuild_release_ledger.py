#!/usr/bin/env python3
"""Rebuild the public machine-readable release ledger from GitHub Releases.

The historical per-tag JSON files are retained for reference. This ledger is the single
current index: it covers every released tag from v3.0.2 onward and records each asset's
published size and SHA-256 as reported by GitHub.

Requires the authenticated GitHub CLI (`gh auth login`).
"""

from __future__ import annotations

import json
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

REPO = "LeTrollologist/Aurelia"
ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "releases" / "ledger.json"


def gh_json(*args: str) -> object:
    raw = subprocess.check_output(
        ["gh", "api", "--paginate", "--slurp", *args],
        text=True,
        encoding="utf-8",
    )
    return json.loads(raw)


def main() -> int:
    pages = gh_json(f"repos/{REPO}/releases?per_page=100")
    releases = []
    for page in pages:
        releases.extend(page)

    ledger = []
    for release in releases:
        tag = release["tag_name"]
        if not tag.startswith("v3."):
            continue
        detail = gh_json(f"repos/{REPO}/releases/tags/{tag}")
        if isinstance(detail, list):
            detail = detail[0]
        assets = []
        for asset in detail.get("assets", []):
            digest = asset.get("digest") or ""
            if digest.startswith("sha256:"):
                digest = digest[len("sha256:"):]
            assets.append(
                {
                    "name": asset["name"],
                    "bytes": asset["size"],
                    "sha256": digest.lower(),
                }
            )
        ledger.append(
            {
                "tag": tag,
                "version": tag[1:],
                "published_at": release.get("published_at"),
                "prerelease": bool(release.get("prerelease")),
                "url": release.get("html_url"),
                "assets": assets,
            }
        )

    ledger.sort(key=lambda item: item["published_at"] or "", reverse=True)
    payload = {
        "product": "Aurelia",
        "source": f"https://github.com/{REPO}/releases",
        "generated_at": datetime.now(timezone.utc).replace(microsecond=0).isoformat(),
        "releases": ledger,
    }
    OUT.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(f"wrote {OUT.relative_to(ROOT)} ({len(ledger)} releases)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
