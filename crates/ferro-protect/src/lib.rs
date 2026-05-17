#![forbid(unsafe_code)]
#![allow(clippy::doc_markdown)]

//! Async Rust client for the UniFi Protect local integration API (v7.1.60).
//!
//! ```no_run
//! # async fn run() -> ferro_protect::Result<()> {
//! use ferro_protect::ProtectClient;
//! use secrecy::SecretString;
//!
//! let client = ProtectClient::builder()
//!     .host("nvr.local")
//!     .api_key("paste-key-here".to_string().into())
//!     .build()?;
//!
//! let info = client.info().await?;
//! println!("Protect {}", info.application_version);
//! # Ok(()) }
//! ```
//!
//! # Logging
//!
//! This crate emits log records through the [`log`](https://docs.rs/log)
//! facade. By itself it produces no output -- the binary using the crate
//! is responsible for configuring a logger (`env_logger`, `tracing-log`,
//! `fern`, etc.). The `ferro-protect` CLI uses `env_logger`; see
//! `crates/ferro-protect-cli/src/main.rs` for the wiring.
//!
//! Levels we emit at:
//!
//! - `info!`  -- client built, application info fetched, entity list/get
//!   operations completed
//! - `debug!` -- each outbound request, builder finalisation
//! - `warn!`  -- response-mapping fallback paths (unexpected body shape,
//!   unknown error code, etc.)

pub mod error;
pub mod models;

mod auth;
mod cameras;
mod chimes;
mod client;
mod generated;
mod lights;
mod liveviews;
mod nvrs;
mod sensors;

pub use cameras::CamerasApi;
pub use chimes::ChimesApi;
pub use client::{ProtectClient, ProtectClientBuilder, TlsMode};
pub use error::{Error, Result};
pub use lights::LightsApi;
pub use liveviews::LiveviewsApi;
pub use nvrs::NvrsApi;
pub use sensors::SensorsApi;
