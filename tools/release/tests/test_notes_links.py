from __future__ import annotations

from pathlib import Path

from aurelia_release.stages import notes_relative_link_problem


def test_absolute_links_and_anchors_are_fine():
    text = (
        "[VirusTotal](https://www.virustotal.com/gui/file/abc)\n"
        "[jump](#verified) [mail](mailto:someone@example.com)\n"
    )
    assert notes_relative_link_problem(text) is None


def test_relative_links_are_rejected_with_the_fix():
    for text in (
        "[docs](../docs/PERFORMANCE.md)",
        "[docs](docs/PERFORMANCE.md)",
        "[file](./README.md)",
        "[asset](../docs/benchmarks/style-baseline.json)",
    ):
        problem = notes_relative_link_problem(text)
        assert problem, text
        assert "relative link" in problem
        assert "absolute https://" in problem


def test_real_release_notes_have_only_absolute_links():
    root = Path(__file__).resolve().parents[3] / "release-notes"
    for notes in sorted(root.glob("v3.*.md")):
        text = notes.read_text(encoding="utf-8")
        assert notes_relative_link_problem(text) is None, notes.name
