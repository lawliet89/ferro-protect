#![forbid(unsafe_code)]
#![allow(clippy::doc_markdown)]

//! Async Rust client for the UniFi Protect local integration API (v6.2.83).
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

pub mod error;
pub mod models;

mod auth;
mod cameras;
mod client;
mod generated;

pub use cameras::CamerasApi;
pub use client::{ProtectClient, ProtectClientBuilder, TlsMode};
pub use error::{Error, Result};
