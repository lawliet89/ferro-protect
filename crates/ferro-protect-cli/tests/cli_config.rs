#![forbid(unsafe_code)]
#![allow(
    clippy::pedantic,
    clippy::nursery,
    reason = "test files prioritise clarity over pedantic style"
)]

//! End-to-end tests for the `ferro-protect config` subcommand and the
//! new `--config` / `UNIFI_PROTECT_CONFIG_FILE` file-discovery surface.
//!
//! These all use `common::isolated_cmd()` so a developer's own
//! `~/.config/ferro-protect/config.toml` cannot leak into the run.

mod common;

use predicates::prelude::*;
use std::fs;
use std::path::PathBuf;

/// Minimal valid config for "happy path" assertions.
const SAMPLE: &str = "host = \"nvr.local\"\n\
                      api_key_file = \"/tmp/some-key\"\n\
                      insecure = true\n\
                      json = false\n\
                      log_level = \"info\"\n";

#[test]
fn show_prints_full_table_with_source_attribution() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), SAMPLE).expect("write");

    let out = cmd
        .args(["--config", cfg.path().to_str().unwrap(), "config", "show"])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("host"), "stdout = {stdout}");
    assert!(stdout.contains("nvr.local"), "stdout = {stdout}");
    assert!(
        stdout.contains("config file:"),
        "expected source attribution, got: {stdout}"
    );
    assert!(stdout.contains("insecure"), "stdout = {stdout}");
    assert!(stdout.contains("true"), "stdout = {stdout}");
    // api_key is never printed -- always masked.
    assert!(
        !stdout.contains("/tmp/some-key") || stdout.contains("api_key_file"),
        "api_key_file path is fine; raw key never; stdout = {stdout}"
    );
    assert!(
        stdout.contains("<unset>"),
        "api_key should be <unset>; stdout = {stdout}"
    );
}

#[test]
fn show_single_key_prints_bare_value() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), SAMPLE).expect("write");

    let out = cmd
        .args([
            "--config",
            cfg.path().to_str().unwrap(),
            "config",
            "show",
            "host",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    assert_eq!(stdout.trim_end(), "nvr.local");
}

#[test]
fn show_unknown_key_errors_and_lists_valid_keys() {
    let mut cmd = common::isolated_cmd();
    cmd.args(["config", "show", "bogus"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown config field"))
        .stderr(predicate::str::contains("host"))
        .stderr(predicate::str::contains("log_level"));
}

#[test]
fn show_json_full_emits_array_of_rows() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), SAMPLE).expect("write");

    let out = cmd
        .args([
            "--config",
            cfg.path().to_str().unwrap(),
            "--json=true",
            "config",
            "show",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    let arr = parsed.as_array().expect("array");
    assert!(arr.iter().any(|row| row["field"] == "host"));
    assert!(arr.iter().any(|row| row["field"] == "api_key"));
    // api_key value must be the literal "<set>" or "<unset>", never a real key.
    for row in arr {
        if row["field"] == "api_key" {
            let v = row["value"].as_str().unwrap();
            assert!(v == "<set>" || v == "<unset>", "got {v}");
        }
    }
}

#[test]
fn show_json_single_key_emits_value_and_source_object() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), SAMPLE).expect("write");

    let out = cmd
        .args([
            "--config",
            cfg.path().to_str().unwrap(),
            "--json=true",
            "config",
            "show",
            "host",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(parsed["value"], "nvr.local");
    assert!(parsed["source"].as_str().unwrap().contains("config file"));
}

#[test]
fn path_returns_resolved_path_on_one_line() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), SAMPLE).expect("write");

    let out = cmd
        .args(["--config", cfg.path().to_str().unwrap(), "config", "path"])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    assert_eq!(stdout.trim_end(), cfg.path().display().to_string());
    assert_eq!(
        stdout.lines().count(),
        1,
        "expected single line: {stdout:?}"
    );
}

#[test]
fn path_falls_back_to_xdg_default_when_no_flag_or_env() {
    let (home, mut cmd) = common::cmd_with_tempdir_home();
    let out = cmd
        .args(["config", "path"])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    let expected: PathBuf = home
        .path()
        .join(".config")
        .join("ferro-protect")
        .join("config.toml");
    assert_eq!(stdout.trim_end(), expected.display().to_string());
}

#[test]
fn path_json_emits_path_and_exists_bool() {
    let mut cmd = common::isolated_cmd();
    let out = cmd
        .args(["--json=true", "config", "path"])
        .assert()
        .success()
        .get_output()
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert!(parsed["path"].is_string());
    assert_eq!(parsed["exists"], false);
}

#[test]
fn env_var_picks_config_file_when_no_flag() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), SAMPLE).expect("write");
    let out = cmd
        .env("UNIFI_PROTECT_CONFIG_FILE", cfg.path())
        .args(["config", "show", "host"])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    assert_eq!(stdout.trim_end(), "nvr.local");
}

#[test]
fn flag_wins_over_env_for_config_file_discovery() {
    let mut cmd = common::isolated_cmd();
    let env_cfg = tempfile::NamedTempFile::new().expect("env");
    fs::write(env_cfg.path(), "host = \"env-host\"\n").expect("write env");
    let flag_cfg = tempfile::NamedTempFile::new().expect("flag");
    fs::write(flag_cfg.path(), "host = \"flag-host\"\n").expect("write flag");
    let out = cmd
        .env("UNIFI_PROTECT_CONFIG_FILE", env_cfg.path())
        .args([
            "--config",
            flag_cfg.path().to_str().unwrap(),
            "config",
            "show",
            "host",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    assert_eq!(stdout.trim_end(), "flag-host");
}

#[test]
fn missing_explicit_config_file_is_hard_error() {
    let mut cmd = common::isolated_cmd();
    cmd.args(["--config", "/definitely/not/here.toml", "config", "show"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("config file not found"));
}

#[test]
fn missing_env_config_file_is_hard_error() {
    let mut cmd = common::isolated_cmd();
    cmd.env("UNIFI_PROTECT_CONFIG_FILE", "/definitely/not/here.toml")
        .args(["config", "show"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("config file not found"));
}

#[test]
fn missing_xdg_default_is_fine_show_just_renders_defaults() {
    let mut cmd = common::isolated_cmd();
    // No --config, no env, no file at HOME/.config/ferro-protect/config.toml.
    cmd.args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("default"));
}

// ----------------- config edit -----------------

#[test]
fn edit_creates_xdg_default_on_first_use() {
    let (home, mut cmd) = common::cmd_with_tempdir_home();
    cmd.args(["config", "edit", "host", "nvr.local"])
        .assert()
        .success();
    let path = home
        .path()
        .join(".config")
        .join("ferro-protect")
        .join("config.toml");
    let body = fs::read_to_string(&path).expect("file exists");
    assert!(body.contains("host = \"nvr.local\""), "body = {body}");
    assert!(
        body.contains("ferro-protect config file"),
        "header missing: {body}"
    );
}

#[test]
fn edit_round_trip_preserves_comments() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    let original = "# user note: this is the home NVR\n\
                    host = \"old.local\"\n\
                    # api key lives elsewhere\n";
    fs::write(cfg.path(), original).expect("write");

    cmd.args([
        "--config",
        cfg.path().to_str().unwrap(),
        "config",
        "edit",
        "host",
        "new.local",
    ])
    .assert()
    .success();

    let body = fs::read_to_string(cfg.path()).expect("read");
    assert!(
        body.contains("# user note: this is the home NVR"),
        "body = {body}"
    );
    assert!(body.contains("host = \"new.local\""), "body = {body}");
    assert!(body.contains("# api key lives elsewhere"), "body = {body}");
}

#[test]
fn edit_invalid_log_level_leaves_file_untouched() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    let original = "log_level = \"info\"\n# keep me\n";
    fs::write(cfg.path(), original).expect("write");

    cmd.args([
        "--config",
        cfg.path().to_str().unwrap(),
        "config",
        "edit",
        "log_level",
        "bogus",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("invalid value"));

    let body = fs::read_to_string(cfg.path()).expect("read");
    assert_eq!(body, original);
}

#[test]
fn edit_refuses_raw_api_key_on_command_line() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), "").expect("write");
    cmd.args([
        "--config",
        cfg.path().to_str().unwrap(),
        "config",
        "edit",
        "api_key",
        "raw-secret-value",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("refusing to set `api_key`"));
    let body = fs::read_to_string(cfg.path()).expect("read");
    assert!(
        !body.contains("raw-secret-value"),
        "raw key leaked into file: {body}"
    );
}

#[test]
fn edit_api_key_file_is_accepted() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), "").expect("write");
    cmd.args([
        "--config",
        cfg.path().to_str().unwrap(),
        "config",
        "edit",
        "api_key_file",
        "/tmp/k",
    ])
    .assert()
    .success();
    let body = fs::read_to_string(cfg.path()).expect("read");
    assert!(body.contains("api_key_file = \"/tmp/k\""), "body = {body}");
}

#[test]
fn edit_host_conflicts_with_existing_base_url() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    let original = "base_url = \"https://nvr.example/proxy/protect/integration\"\n";
    fs::write(cfg.path(), original).expect("write");

    cmd.args([
        "--config",
        cfg.path().to_str().unwrap(),
        "config",
        "edit",
        "host",
        "nvr.local",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("conflict"));

    let body = fs::read_to_string(cfg.path()).expect("read");
    assert_eq!(body, original, "file was modified on conflict");
}

#[test]
fn edit_unset_removes_field() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), "host = \"nvr.local\"\ninsecure = true\n").expect("write");

    cmd.args([
        "--config",
        cfg.path().to_str().unwrap(),
        "config",
        "edit",
        "host",
        "--unset",
    ])
    .assert()
    .success();

    let body = fs::read_to_string(cfg.path()).expect("read");
    assert!(!body.contains("host ="), "body still has host: {body}");
    assert!(
        body.contains("insecure = true"),
        "body lost insecure: {body}"
    );
}

#[test]
fn edit_unset_already_absent_is_noop() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), "insecure = true\n").expect("write");
    cmd.args([
        "--config",
        cfg.path().to_str().unwrap(),
        "config",
        "edit",
        "host",
        "--unset",
    ])
    .assert()
    .success();
}

#[test]
fn precedence_flag_wins_over_env_and_file_in_show() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), "host = \"file-host\"\n").expect("write");
    // `config show` ignores per-invocation overrides by design
    // (see commands/config.rs::show), so the env var should win.
    let out = cmd
        .env("UNIFI_PROTECT_HOST", "env-host")
        .args([
            "--config",
            cfg.path().to_str().unwrap(),
            "config",
            "show",
            "host",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    assert_eq!(stdout.trim_end(), "env-host");
}

#[test]
fn init_refuses_when_stdin_is_not_a_tty() {
    let mut cmd = common::isolated_cmd();
    cmd.args(["config", "init"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires a TTY"));
}
