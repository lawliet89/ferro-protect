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
    Camera, CameraId, ChannelQuality, Chime, ChimeId, Light, LightId, Liveview, LiveviewId, Nvr,
    NvrId, ProtectVersion, Sensor, SensorId, SnapshotChannel, Viewer, ViewerId,
};

/// Optional query parameters for [`crate::CamerasApi::snapshot_with`].
///
/// The defaults (`channel = None`, `high_quality = false`) hit the
/// main channel at the camera's negotiated stream quality, which is
/// what callers want 99% of the time. The fields are exposed for
/// the package-camera and 1080p-force special cases.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotOptions {
    /// Camera channel to capture from. `Some(SnapshotChannel::Package)`
    /// requires the camera to have `hasPackageCamera: true`; otherwise
    /// the NVR returns a 4xx.
    pub channel: Option<SnapshotChannel>,
    /// Force 1080P or higher resolution snapshot. Spec query param
    /// `highQuality=true`.
    pub high_quality: bool,
}

/// One RTSPS stream URL paired with the quality level it was created
/// for. Returned by [`crate::CamerasApi::rtsps_stream`].
///
/// The underlying API returns a flat object with one optional field
/// per quality (`high` / `medium` / `low` / `package`); this wrapper
/// flattens it into a vec so callers can iterate without checking
/// four `Option`s. Only the qualities the caller actually requested
/// will appear in the returned vec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtspsStream {
    pub quality: ChannelQuality,
    pub url: String,
}

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
