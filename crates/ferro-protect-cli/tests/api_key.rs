#![forbid(unsafe_code)]
#![allow(
    clippy::pedantic,
    clippy::nursery,
    reason = "test files prioritise clarity over pedantic style"
)]

//! Unit-style tests for the API-key resolver.
//!
//! These run against the CLI crate's library half (set up in phase 3 so
//! resolver internals are reachable from `tests/`). The resolver takes an
//! `env` callback parameter; each test passes its own closure instead of
//! mutating `std::env::*`, so tests can run in parallel.

mod common;

use std::collections::HashMap;
use std::io::Write;

use ferro_protect_cli::api_key::{self, ApiKeyError, ApiKeySource, ENV_KEY, ENV_KEY_FILE, Sources};
use secrecy::{ExposeSecret, SecretString};
use tempfile::TempDir;

/// Build a closure suitable for `api_key::resolve`'s `env` parameter. The
/// keys and values are copied into an owned `HashMap` so callers can pass
/// references with any lifetime, including borrows of stack-local strings.
fn env_from<K, V, I>(pairs: I) -> impl Fn(&str) -> Option<String>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<String>,
    V: Into<String>,
{
    let map: HashMap<String, String> = pairs
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    move |k| map.get(k).cloned()
}

fn empty_env() -> impl Fn(&str) -> Option<String> {
    |_| None
}

/// Write `contents` to a fresh file in a fresh tempdir and return the
/// (tempdir, path). Keep the `TempDir` alive for the duration of the test
/// or it deletes itself.
fn write_key_file(contents: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("key");
    std::fs::write(&path, contents).expect("write");
    (dir, path)
}

fn sources_flag(path: &std::path::Path) -> Sources<'_> {
    Sources {
        flag_file: Some(path),
        ..Sources::default()
    }
}

#[test]
fn flag_wins_over_both_env_vars() {
    let (_d, path) = write_key_file("from-flag");
    let env = env_from([
        (ENV_KEY_FILE, "/should/not/be/read"),
        (ENV_KEY, "from-raw-env"),
    ]);
    let mut warnings = Vec::new();
    let (key, source) =
        api_key::resolve(&sources_flag(&path), &env, &mut warnings).expect("resolves");
    assert_eq!(key.expose_secret(), "from-flag");
    assert_eq!(source, ApiKeySource::Flag);
}

#[test]
fn file_env_wins_over_raw_env() {
    let (_d, path) = write_key_file("from-env-file");
    let env = env_from([
        (ENV_KEY_FILE, path.to_str().unwrap()),
        (ENV_KEY, "from-raw-env"),
    ]);
    let mut warnings = Vec::new();
    let (key, source) =
        api_key::resolve(&Sources::default(), &env, &mut warnings).expect("resolves");
    assert_eq!(key.expose_secret(), "from-env-file");
    assert_eq!(source, ApiKeySource::EnvFile);
}

#[test]
fn raw_env_works_alone() {
    let env = env_from([(ENV_KEY, "raw-key")]);
    let mut warnings = Vec::new();
    let (key, source) =
        api_key::resolve(&Sources::default(), &env, &mut warnings).expect("resolves");
    assert_eq!(key.expose_secret(), "raw-key");
    assert_eq!(source, ApiKeySource::EnvRaw);
}

#[test]
fn no_source_returns_not_provided_with_helpful_message() {
    let mut warnings = Vec::new();
    let err =
        api_key::resolve(&Sources::default(), &empty_env(), &mut warnings).expect_err("errors");
    assert!(matches!(err, ApiKeyError::NotProvided));
    let msg = err.to_string();
    assert!(msg.contains("--api-key-file"), "msg = {msg}");
    assert!(msg.contains(ENV_KEY_FILE), "msg = {msg}");
    assert!(msg.contains(ENV_KEY), "msg = {msg}");
    assert!(msg.contains("api_key_file"), "msg = {msg}");
    assert!(msg.contains("api_key"), "msg = {msg}");
}

#[test]
fn empty_file_via_flag_errors() {
    let (_d, path) = write_key_file("");
    let mut warnings = Vec::new();
    let err =
        api_key::resolve(&sources_flag(&path), &empty_env(), &mut warnings).expect_err("errors");
    match err {
        ApiKeyError::EmptyFile(p) => assert_eq!(p, path),
        other => panic!("expected EmptyFile, got {other:?}"),
    }
}

#[test]
fn whitespace_only_file_errors_as_empty() {
    let (_d, path) = write_key_file("   \n  \t  \n");
    let mut warnings = Vec::new();
    let err =
        api_key::resolve(&sources_flag(&path), &empty_env(), &mut warnings).expect_err("errors");
    assert!(matches!(err, ApiKeyError::EmptyFile(_)));
}

#[test]
fn nonexistent_file_errors_with_path() {
    let mut warnings = Vec::new();
    let phantom = std::path::PathBuf::from("/definitely/does/not/exist/key");
    let err =
        api_key::resolve(&sources_flag(&phantom), &empty_env(), &mut warnings).expect_err("errors");
    match err {
        ApiKeyError::ReadFailed { path, .. } => assert_eq!(path, phantom),
        other => panic!("expected ReadFailed, got {other:?}"),
    }
}

#[test]
fn trims_trailing_whitespace_from_file_contents() {
    let (_d, path) = write_key_file("  the-key  \n\n");
    let mut warnings = Vec::new();
    let (key, _) =
        api_key::resolve(&sources_flag(&path), &empty_env(), &mut warnings).expect("resolves");
    assert_eq!(key.expose_secret(), "the-key");
}

#[test]
fn empty_raw_env_falls_through_to_not_provided() {
    let env = env_from([(ENV_KEY, "   \n")]);
    let mut warnings = Vec::new();
    let err = api_key::resolve(&Sources::default(), &env, &mut warnings).expect_err("errors");
    assert!(matches!(err, ApiKeyError::NotProvided));
}

#[test]
fn empty_raw_env_falls_through_to_config_file_source() {
    let (_d, path) = write_key_file("from-config-file");
    let env = env_from([(ENV_KEY, "   ")]);
    let sources = Sources {
        config_file: Some(&path),
        ..Sources::default()
    };
    let mut warnings = Vec::new();
    let (key, source) = api_key::resolve(&sources, &env, &mut warnings).expect("resolves");
    assert_eq!(key.expose_secret(), "from-config-file");
    assert_eq!(source, ApiKeySource::ConfigFile);
}

#[test]
fn config_file_pointer_used_when_no_flag_or_env() {
    let (_d, path) = write_key_file("from-config-file");
    let sources = Sources {
        config_file: Some(&path),
        ..Sources::default()
    };
    let mut warnings = Vec::new();
    let (key, source) = api_key::resolve(&sources, &empty_env(), &mut warnings).expect("resolves");
    assert_eq!(key.expose_secret(), "from-config-file");
    assert_eq!(source, ApiKeySource::ConfigFile);
}

#[test]
fn config_raw_used_when_no_other_source() {
    let secret = SecretString::from("from-config-raw");
    let sources = Sources {
        config_raw: Some(&secret),
        ..Sources::default()
    };
    let mut warnings = Vec::new();
    let (key, source) = api_key::resolve(&sources, &empty_env(), &mut warnings).expect("resolves");
    assert_eq!(key.expose_secret(), "from-config-raw");
    assert_eq!(source, ApiKeySource::ConfigRaw);
}

#[test]
fn config_file_pointer_wins_over_config_raw() {
    let (_d, path) = write_key_file("from-config-pointer");
    let secret = SecretString::from("from-config-raw");
    let sources = Sources {
        config_file: Some(&path),
        config_raw: Some(&secret),
        ..Sources::default()
    };
    let mut warnings = Vec::new();
    let (key, source) = api_key::resolve(&sources, &empty_env(), &mut warnings).expect("resolves");
    assert_eq!(key.expose_secret(), "from-config-pointer");
    assert_eq!(source, ApiKeySource::ConfigFile);
}

#[test]
fn env_wins_over_config_sources() {
    let secret = SecretString::from("from-config-raw");
    let env = env_from([(ENV_KEY, "from-raw-env")]);
    let sources = Sources {
        config_raw: Some(&secret),
        ..Sources::default()
    };
    let mut warnings = Vec::new();
    let (key, source) = api_key::resolve(&sources, &env, &mut warnings).expect("resolves");
    assert_eq!(key.expose_secret(), "from-raw-env");
    assert_eq!(source, ApiKeySource::EnvRaw);
}

#[cfg(unix)]
#[test]
fn warns_on_lax_file_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let (_d, path) = write_key_file("key");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");

    let mut warnings: Vec<u8> = Vec::new();
    let _ = api_key::resolve(&sources_flag(&path), &empty_env(), &mut warnings).expect("resolves");
    let msg = String::from_utf8(warnings).unwrap();
    assert!(msg.contains("warning"), "expected warning, got: {msg}");
    assert!(msg.contains("chmod 600"), "expected fix hint, got: {msg}");
}

#[cfg(unix)]
#[test]
fn no_warning_on_tight_file_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let (_d, path) = write_key_file("key");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");

    let mut warnings: Vec<u8> = Vec::new();
    let _ = api_key::resolve(&sources_flag(&path), &empty_env(), &mut warnings).expect("resolves");
    assert!(
        warnings.is_empty(),
        "unexpected warning: {:?}",
        String::from_utf8_lossy(&warnings)
    );
}

/// Sanity: the CLI binary itself rejects the call when no key is provided.
#[test]
fn binary_rejects_when_no_key_provided() {
    use assert_cmd::Command;
    let assert = Command::cargo_bin("ferro-protect")
        .expect("binary")
        .env_remove(ENV_KEY)
        .env_remove(ENV_KEY_FILE)
        .env_remove("UNIFI_PROTECT_HOST")
        .env_remove("UNIFI_PROTECT_BASE_URL")
        .args(["--host", "ignored", "info"])
        .assert();
    let output = assert.failure().get_output().clone();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("--api-key-file") && combined.contains(ENV_KEY_FILE),
        "expected NotProvided message, got: {combined}"
    );
}

// Silence the unused-import warning on non-Unix where the chmod tests are
// cfg'd out.
fn _ensure_write_in_scope() {
    let mut v: Vec<u8> = Vec::new();
    let _ = v.write(b"");
}
