#![forbid(unsafe_code)]

//! Library half of the `ferro-protect` CLI. Exists so the API-key
//! resolver and other CLI internals can be unit-tested by integration
//! tests in `tests/`.

pub mod api_key;
pub mod commands;
pub mod config;
pub mod logging;
pub mod output;
