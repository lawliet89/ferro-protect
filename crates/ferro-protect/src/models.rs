//! Public re-exports of types that cross the library boundary.
//!
//! This module is the integration seam between the typify-generated code
//! (which lives in `crate::generated`) and the rest of the library.
//! Every public type the library exposes is re-exported (and, where it
//! helps ergonomics, renamed) here.
//!
//! When the OpenAPI spec is upgraded and a generated type is renamed or
//! restructured, this file is the first place that should change. See
//! `UPGRADING.md` ("When wrappers fail to compile") for the workflow.
//!
//! **Do not** name `crate::generated::...` types in any public signature.

use serde::{Deserialize, Serialize};

pub use crate::generated::{
    Camera, CameraId, Chime, ChimeId, Light, LightId, Liveview, LiveviewId, Nvr, NvrId,
    ProtectVersion, Sensor, SensorId, Viewer, ViewerId,
};

/// Application metadata returned by `GET /v1/meta/info`.
///
/// This response schema is inline in the OpenAPI operation rather than named
/// under `components.schemas`, so the models-only codegen pipeline cannot
/// produce it from the component set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationInfo {
    #[serde(rename = "applicationVersion")]
    pub application_version: ProtectVersion,
}
