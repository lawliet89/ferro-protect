#![allow(
    dead_code,
    reason = "shared between integration test crates; not every helper is used in every file"
)]
#![allow(
    clippy::pedantic,
    clippy::nursery,
    reason = "test helpers prioritise clarity over pedantic style"
)]

//! Shared helpers for CLI integration tests.
//!
//! The most important helpers are [`isolated_cmd`] and
//! [`cmd_with_tempdir_home`]: they build a `Command` pointing at the
//! freshly built `ferro-protect` binary with **every** `UNIFI_PROTECT_*`
//! env var scrubbed and `HOME` / `XDG_CONFIG_HOME` set to either a
//! sentinel path or a per-test temp directory.
//!
//! Without isolation, a developer who has previously run
//! `ferro-protect config init` would have their personal config silently
//! picked up by assert_cmd-driven tests that don't pass `--config`,
//! producing test failures that depend on the developer's machine.

use std::path::Path;

use assert_cmd::Command;
use tempfile::TempDir;

/// Names of every env var the CLI reads (or considers reading). Listed
/// once so isolation helpers can scrub them in a single pass; the
/// list also doubles as documentation of the env surface.
pub const SCRUBBED_ENV_VARS: &[&str] = &[
    "UNIFI_PROTECT_HOST",
    "UNIFI_PROTECT_BASE_URL",
    "UNIFI_PROTECT_API_KEY",
    "UNIFI_PROTECT_API_KEY_FILE",
    "UNIFI_PROTECT_INSECURE",
    "UNIFI_PROTECT_JSON",
    "UNIFI_PROTECT_LOG",
    "UNIFI_PROTECT_CONFIG_FILE",
    "XDG_CONFIG_HOME",
];

/// Sentinel `HOME` for tests that don't care about XDG discovery. The
/// path is deliberately under a top-level name that nothing else uses,
/// so XDG resolution lands on a directory that definitely doesn't
/// exist (the loader treats that as "no config", per design).
const NONEXISTENT_HOME: &str = "/__ferro_protect_test_no_home__";

/// Build a `Command` for the `ferro-protect` binary with all
/// `UNIFI_PROTECT_*` env vars scrubbed and `HOME` pointed at a
/// non-existent sentinel path. Use for tests that don't exercise
/// config-file discovery — they get clean isolation without paying
/// for a real tempdir.
pub fn isolated_cmd() -> Command {
    let mut c = Command::cargo_bin("ferro-protect").expect("binary built");
    for v in SCRUBBED_ENV_VARS {
        c.env_remove(v);
    }
    c.env("HOME", NONEXISTENT_HOME);
    c
}

/// Like `isolated_cmd` but `HOME` is a fresh temp directory. Use for
/// tests that need XDG discovery to resolve to a real on-disk path
/// (e.g. asserting `config path` returns the XDG fallback, or that
/// `config edit` creates the XDG default on first use). The caller
/// keeps the `TempDir` alive for the duration of the test.
pub fn cmd_with_tempdir_home() -> (TempDir, Command) {
    let dir = TempDir::new().expect("tempdir");
    let mut c = Command::cargo_bin("ferro-protect").expect("binary built");
    for v in SCRUBBED_ENV_VARS {
        c.env_remove(v);
    }
    c.env("HOME", dir.path());
    (dir, c)
}

/// Build a `Command` with `HOME` pointed at the given directory. Used
/// rarely; most tests want `isolated_cmd` or `cmd_with_tempdir_home`.
pub fn cmd_with_home(home: &Path) -> Command {
    let mut c = Command::cargo_bin("ferro-protect").expect("binary built");
    for v in SCRUBBED_ENV_VARS {
        c.env_remove(v);
    }
    c.env("HOME", home);
    c
}
