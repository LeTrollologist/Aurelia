from __future__ import annotations

from aurelia_release.envfile import load_env_file


def test_env_file_loads_values_without_overriding(tmp_path):
    path = tmp_path / ".env"
    path.write_text(
        "# comment\n\n"
        "EXISTING=process\n"
        "VIRUSTOTAL_API_KEY='quoted value'\n"
        "export GITHUB_TOKEN=abc123\n"
        "IGNORED LINE\n",
        encoding="utf-8",
    )
    env = {"EXISTING": "keep-me"}
    loaded = load_env_file(path, env)

    assert loaded == {"VIRUSTOTAL_API_KEY": "quoted value", "GITHUB_TOKEN": "abc123"}
    assert env["VIRUSTOTAL_API_KEY"] == "quoted value"
    assert env["EXISTING"] == "keep-me"


def test_missing_env_file_is_a_no_op(tmp_path):
    assert load_env_file(tmp_path / "missing.env") == {}
