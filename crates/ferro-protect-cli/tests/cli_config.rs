#![forbid(unsafe_code)]
#![allow(
    clippy::pedantic,
    clippy::nursery,
    reason = "test files prioritise clarity over pedantic style"
)]

//! End-to-end tests for the `ferro-protect config` subcommand and the
//! `--config` / `UNIFI_PROTECT_CONFIG_FILE` file-discovery surface.
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

// ----------------- config show -----------------

#[test]
fn show_prints_full_table_with_field_and_value() {
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
    assert!(stdout.contains("insecure"), "stdout = {stdout}");
    assert!(stdout.contains("true"), "stdout = {stdout}");
    // Spike: source column dropped, so the header is just FIELD/VALUE.
    assert!(
        !stdout.contains("SOURCE"),
        "SOURCE column should be gone in the no-attribution spike: {stdout}"
    );
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
fn show_json_single_key_emits_value_object() {
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
    // Spike: no `source` key in the single-row JSON.
    assert!(parsed.get("source").is_none(), "got: {parsed}");
}

/// Inline `api_key = "..."` in the config file was removed: a
/// secret-bearing TOML field would land in commits, backups, and
/// dotfile syncs. `deny_unknown_fields` on `ConfigFile` rejects it at
/// parse time, and the secret never reaches stdout. Regression guard.
#[test]
fn inline_api_key_in_config_file_is_rejected() {
    const SECRET: &str = "supersecret-must-not-leak";
    let config_body = format!(
        "host = \"nvr.local\"\n\
         api_key = \"{SECRET}\"\n"
    );
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), &config_body).expect("write");
    let out = common::isolated_cmd()
        .args(["--config", cfg.path().to_str().unwrap(), "config", "show"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let merged = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        !merged.contains(SECRET),
        "secret leaked into stdout/stderr: {merged}"
    );
    // `deny_unknown_fields` surfaces the offending key in the parse
    // error, so the message names `api_key` even though we don't echo
    // the value.
    assert!(
        merged.contains("api_key"),
        "expected parse error to mention `api_key`: {merged}"
    );
}

// ----------------- config path -----------------

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
fn path_resolves_xdg_default_when_file_exists() {
    let (home, mut cmd) = common::cmd_with_tempdir_home();
    let expected: PathBuf = home
        .path()
        .join(".config")
        .join("ferro-protect")
        .join("config.toml");
    fs::create_dir_all(expected.parent().unwrap()).expect("mkdir");
    fs::write(&expected, SAMPLE).expect("write");

    let out = cmd
        .args(["config", "path"])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    assert_eq!(stdout.trim_end(), expected.display().to_string());
}

#[test]
fn path_errors_when_xdg_default_is_missing() {
    let (_home, mut cmd) = common::cmd_with_tempdir_home();
    cmd.args(["config", "path"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no config file"));
}

#[test]
fn path_errors_when_resolved_path_is_a_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut cmd = common::isolated_cmd();
    cmd.args(["--config", dir.path().to_str().unwrap(), "config", "path"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no config file"));
}

#[test]
fn path_json_emits_path_when_file_exists() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), SAMPLE).expect("write");
    let out = cmd
        .args([
            "--config",
            cfg.path().to_str().unwrap(),
            "--json=true",
            "config",
            "path",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(parsed["path"], cfg.path().display().to_string());
}

// ----------------- file discovery -----------------

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
fn missing_xdg_default_errors_with_actionable_hint() {
    let (_home, mut cmd) = common::cmd_with_tempdir_home();
    // No --config, no env, no file at HOME/.config/ferro-protect/config.toml.
    cmd.args(["config", "show"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no config file"))
        .stderr(predicate::str::contains("ferro-protect config template"));
}

// ----------------- config template -----------------

#[test]
fn template_writes_commented_scaffold_to_xdg_default() {
    let (home, mut cmd) = common::cmd_with_tempdir_home();
    cmd.args(["config", "template"]).assert().success();
    let path = home
        .path()
        .join(".config")
        .join("ferro-protect")
        .join("config.toml");
    let body = fs::read_to_string(&path).expect("file exists");
    // Every settable field appears, commented out.
    for key in [
        "host",
        "base_url",
        "api_key_file",
        "insecure",
        "json",
        "log_level",
    ] {
        let needle = format!("# {key} =");
        assert!(body.contains(&needle), "missing `{needle}` in: {body}");
    }
    // `api_key` is a `show`-only row -- it must not appear as a
    // file field so users don't paste a key into the TOML.
    assert!(
        !body.contains("# api_key ="),
        "scaffold should not include `api_key`: {body}"
    );
}

#[test]
fn template_refuses_to_overwrite_without_force() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), "host = \"keep-me\"\n").expect("write");
    cmd.args([
        "--config",
        cfg.path().to_str().unwrap(),
        "config",
        "template",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("already exists"));
    let body = fs::read_to_string(cfg.path()).expect("read");
    assert_eq!(body, "host = \"keep-me\"\n", "file was clobbered");
}

#[test]
fn template_force_overwrites_existing_file() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), "host = \"old\"\n").expect("write");
    cmd.args([
        "--config",
        cfg.path().to_str().unwrap(),
        "config",
        "template",
        "--force",
    ])
    .assert()
    .success();
    let body = fs::read_to_string(cfg.path()).expect("read");
    assert!(body.contains("# host ="), "template missing: {body}");
    assert!(
        !body.contains("host = \"old\""),
        "old line survived: {body}"
    );
}

#[test]
fn template_stdout_prints_without_writing() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    let original = "host = \"keep-me\"\n";
    fs::write(cfg.path(), original).expect("write");

    let out = cmd
        .args([
            "--config",
            cfg.path().to_str().unwrap(),
            "config",
            "template",
            "--stdout",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    assert!(
        stdout.contains("# host ="),
        "template missing from stdout: {stdout}"
    );
    // File untouched — `--stdout` must never modify the destination.
    let body = fs::read_to_string(cfg.path()).expect("read");
    assert_eq!(body, original, "file modified despite --stdout");
}

#[cfg(unix)]
#[test]
fn template_writes_with_0600_perms() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tmpdir");
    let cfg_path = dir.path().join("scaffold.toml");
    let mut cmd = common::isolated_cmd();
    cmd.args(["--config", cfg_path.to_str().unwrap(), "config", "template"])
        .assert()
        .success();
    let mode = fs::metadata(&cfg_path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    // Owner-read+write only -- no group, no world. The atomic
    // temp-write helper opens with mode 0o600 at creation, no
    // chmod-after-write window.
    assert_eq!(mode, 0o600, "got 0o{mode:o}");
}

#[cfg(unix)]
#[test]
fn template_atomic_write_does_not_leave_tmp_file() {
    // Use a per-test directory rather than placing the cfg directly in
    // `/tmp`: the assertion below scans the cfg's parent for any stale
    // `tmp_sibling` output, and if `/tmp` is the parent it also catches
    // *other* concurrent tests' in-flight tmp files (each test's
    // `template --force` creates one before rename). Isolating in a
    // fresh tempdir means the scan only sees this invocation's output.
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_path = dir.path().join("config.toml");
    fs::write(&cfg_path, "host = \"old\"\n").expect("write");

    let mut cmd = common::isolated_cmd();
    cmd.args([
        "--config",
        cfg_path.to_str().unwrap(),
        "config",
        "template",
        "--force",
    ])
    .assert()
    .success();

    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .expect("read_dir")
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
        .collect();
    assert!(
        leftovers.is_empty(),
        "stale temp files: {:?}",
        leftovers.iter().map(|e| e.path()).collect::<Vec<_>>()
    );
}

// ----------------- tilde expansion at load -----------------

/// The template example for `api_key_file` is `~/.config/...`; users
/// hand-edit that into their config. Without expansion, the runtime
/// would try to `read_to_string("~/...")` and fail. Verify the loader
/// expands tilde so `config show api_key_file` reports an absolute
/// path that matches `$HOME/...`.
#[test]
fn load_expands_tilde_in_api_key_file_so_runtime_can_read_it() {
    let (home, mut cmd) = common::cmd_with_tempdir_home();
    let cfg_dir = home.path().join(".config").join("ferro-protect");
    fs::create_dir_all(&cfg_dir).expect("mkdir");
    let cfg_path = cfg_dir.join("config.toml");
    fs::write(
        &cfg_path,
        "host = \"nvr.local\"\napi_key_file = \"~/keys/protect\"\n",
    )
    .expect("write");

    let out = cmd
        .args(["config", "show", "api_key_file"])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    let expected = home.path().join("keys").join("protect");
    assert_eq!(
        stdout.trim_end(),
        expected.display().to_string(),
        "tilde was not expanded by the loader",
    );
    assert!(!stdout.contains('~'), "tilde leaked through: {stdout}");
}

// ----------------- precedence + cross-source mutual exclusion -----------------

#[test]
fn precedence_env_wins_over_file_in_show() {
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

/// `config show api_key_file` reflects `UNIFI_PROTECT_API_KEY_FILE`
/// when it's set, not the lower-priority file path. The runtime
/// API-key resolver prefers the env var over the file's
/// `api_key_file`, so showing the file value would be a stale path.
/// Without source attribution there's no row to mis-attribute, but
/// the *value* still has to be the env-precedence winner.
#[test]
fn show_api_key_file_reflects_env_override() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(
        cfg.path(),
        "host = \"nvr.local\"\napi_key_file = \"/file/path\"\n",
    )
    .expect("write");
    let out = cmd
        .env("UNIFI_PROTECT_API_KEY_FILE", "/env/path")
        .args([
            "--config",
            cfg.path().to_str().unwrap(),
            "--json=true",
            "config",
            "show",
            "api_key_file",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(parsed["value"], "/env/path");
}

/// `config show` reports `log_level = warn` (the runtime default)
/// when neither flag nor file supplies a value.
#[test]
fn show_log_level_defaults_to_warn() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), "host = \"nvr.local\"\n").expect("write"); // no log_level
    let out = cmd
        .args([
            "--config",
            cfg.path().to_str().unwrap(),
            "--json=true",
            "config",
            "show",
            "log_level",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(parsed["value"], "warn");
}

/// Cross-source mutual exclusion: setting `host` in the file and
/// `--base-url` on the flag (or any other cross-source pairing) must
/// be rejected.
#[test]
fn host_in_file_plus_base_url_flag_is_rejected() {
    let mut cmd = common::isolated_cmd();
    let cfg = tempfile::NamedTempFile::new().expect("tempfile");
    fs::write(cfg.path(), "host = \"nvr.local\"\n").expect("write");
    cmd.args([
        "--config",
        cfg.path().to_str().unwrap(),
        "--base-url",
        "https://example/proxy/protect/integration",
        "info",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("cannot both be set"));
}

#[test]
fn host_and_base_url_both_on_argv_is_rejected_by_clap() {
    let mut cmd = common::isolated_cmd();
    cmd.args([
        "--host",
        "nvr.local",
        "--base-url",
        "https://example/proxy/protect/integration",
        "info",
    ])
    .assert()
    .failure()
    .stderr(
        predicate::str::contains("cannot be used with").or(predicate::str::contains("conflicts")),
    );
}
