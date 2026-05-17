//! Public re-exports of types that cross the library boundary.
//!
//! This module is the integration seam between the progenitor-generated
//! code (which lives in `crate::generated`) and the rest of the library.
//! Every public type the library exposes is re-exported (and, where it
//! helps ergonomics, renamed) here.
//!
//! When the OpenAPI spec is upgraded and a generated type is renamed or
//! restructured, this file is the first place that should change. See
//! `UPGRADING.md` ("When wrappers fail to compile") for the workflow.
//!
//! **Do not** name `crate::generated::...` types in any public signature.

pub use crate::generated::types::GetMetaInfoResponse as ApplicationInfo;
pub use crate::generated::types::ProtectVersion;
