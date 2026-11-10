from __future__ import annotations

from aurelia_release.versioning import (
    bump_files,
    check_files,
    declared_versions,
    normalize_text,
)


def test_normalizes_product_declarations_but_not_dependencies(tmp_path):
    text = """[package]
name = "aurelia-io"
version = "3.0.2"

[dependencies]
aurelia-core = { path = "../aurelia-core", version = "3.0.2" }
serde = "1.0"
"""
    updated, count = normalize_text(text, "3.1.0")
    assert count == 2
    assert 'version = "3.1.0"' in updated
    assert 'version = "3.1.0" }' in updated
    assert 'serde = "1.0"' in updated
    assert '3.0.2' not in updated

    again, _ = normalize_text(updated, "3.1.0")
    assert again == updated


def test_declared_versions_collects_docs_and_installer():
    text = """
Release-v3.0.2
Aurelia-v3.0.2-win64.zip
Aurelia-Setup-3.0.2.exe
TAG ?= v3.0.2
v3.0.2 · PRODUCTION
MyAppVersion "3.0.2"
MyAppVersionNumeric "3.0.2.0"
"""
    assert declared_versions(text) == {"3.0.2"}


def test_files_must_all_agree(tmp_path):
    (tmp_path / "a.toml").write_text('[package]\nversion = "3.0.2"\n', encoding="utf-8")
    (tmp_path / "b.md").write_text("Release-v3.0.2\n", encoding="utf-8")
    assert check_files(tmp_path, ["a.toml", "b.md"], "3.0.2") == []

    (tmp_path / "b.md").write_text("Release-v3.0.1\n", encoding="utf-8")
    problems = check_files(tmp_path, ["a.toml", "b.md"], "3.0.2")
    assert any("b.md" in problem for problem in problems)


def test_bump_files_reports_only_real_changes(tmp_path):
    (tmp_path / "a.toml").write_text('[package]\nversion = "3.0.2"\n', encoding="utf-8")
    changed = bump_files(tmp_path, ["a.toml"], "3.1.0")
    assert changed == ["a.toml"]
    assert '3.1.0' in (tmp_path / "a.toml").read_text(encoding="utf-8")


def test_files_without_a_version_declaration_are_tolerated(tmp_path):
    (tmp_path / "workspace.toml").write_text("[workspace]\nmembers = []\n", encoding="utf-8")
    (tmp_path / "a.toml").write_text('[package]\nversion = "3.0.2"\n', encoding="utf-8")
    assert check_files(tmp_path, ["workspace.toml", "a.toml"], "3.0.2") == []
    assert bump_files(tmp_path, ["workspace.toml", "a.toml"], "3.0.3") == ["a.toml"]


def test_prerelease_and_release_candidate_versions_are_supported():
    text = """version = "3.13.0-beta"
Release-v3.13.0-beta
Aurelia-Setup-3.13.0-beta.exe
MyAppVersion "3.13.0-beta"
MyAppVersionNumeric "3.13.0.0"
"""
    assert declared_versions(text) == {"3.13.0-beta"}

    updated, count = normalize_text(text, "3.13.0-rc.1")
    assert count >= 3
    assert 'version = "3.13.0-rc.1"' in updated
    assert "Release-v3.13.0-rc.1" in updated
    assert 'MyAppVersion "3.13.0"' in updated
    assert 'MyAppVersionNumeric "3.13.0.0"' in updated


def test_prerelease_files_must_agree(tmp_path):
    (tmp_path / "a.toml").write_text('[package]\nversion = "3.13.0-rc.1"\n', encoding="utf-8")
    assert check_files(tmp_path, ["a.toml"], "3.13.0-rc.1") == []
    assert check_files(tmp_path, ["a.toml"], "3.13.0") != []


def test_current_release_notes_are_rewritten_in_docs():
    text = """**Status:** v3.13.0 · Windows
## Now — v3.13.0: Colour management (production)
> **v3.13.0 is the current release.**
| Colour management (v3.13.0) | shipped |
"""
    updated, count = normalize_text(text, "3.14.0")
    assert count >= 4
    assert "v3.14.0" in updated
    assert "v3.13.0" not in updated
