//! Camera read endpoints. PATCH and action endpoints land in phases 8/9.

use bytes::Bytes;
use log::info;

use crate::client::ProtectClient;
use crate::error::Result;
use crate::models::{Camera, CameraId, SnapshotOptions};

/// Camera-scoped API entry point. Cheap to construct; holds a borrow
/// of the [`ProtectClient`] that issued it.
pub struct CamerasApi<'a> {
    client: &'a ProtectClient,
}

impl<'a> CamerasApi<'a> {
    pub(crate) const fn new(client: &'a ProtectClient) -> Self {
        Self { client }
    }

    /// `GET /v1/cameras`. List every camera the NVR knows about.
    ///
    /// # Errors
    /// [`Error`] -- typically `Http` (network) or `Api` (4xx).
    pub async fn list(&self) -> Result<Vec<Camera>> {
        let cameras: Vec<Camera> = self.client.get_json("/v1/cameras").await?;
        info!("listed {} camera(s)", cameras.len());
        Ok(cameras)
    }

    /// `GET /v1/cameras/{id}`. Look up one camera by ID.
    ///
    /// # Errors
    /// [`Error`] -- typically `Http`, `Api { status: 404, .. }` for an
    /// unknown ID, or `Json` if the response body fails the schema.
    pub async fn get(&self, id: &CameraId) -> Result<Camera> {
        let path = format!("/v1/cameras/{id}");
        let camera: Camera = self.client.get_json(&path).await?;
        info!("fetched camera {} (name: {:?})", camera.id, camera.name);
        Ok(camera)
    }

    /// `GET /v1/cameras/{id}/snapshot`. Fetch a JPEG snapshot from the
    /// camera's main channel, at the camera's negotiated stream quality.
    ///
    /// Convenience wrapper around [`Self::snapshot_with`] using
    /// [`SnapshotOptions::default`].
    ///
    /// # Errors
    /// [`Error`] -- typically `Http`, `Api { status: 404 | 503, .. }`
    /// (unknown camera, or camera offline / not reachable).
    pub async fn snapshot(&self, id: &CameraId) -> Result<Bytes> {
        self.snapshot_with(id, &SnapshotOptions::default()).await
    }

    /// `GET /v1/cameras/{id}/snapshot` with optional channel and
    /// quality overrides. See [`SnapshotOptions`].
    ///
    /// # Errors
    /// As [`Self::snapshot`]; additionally returns a 4xx if `channel
    /// = Some(SnapshotChannel::Package)` is requested for a camera
    /// that does not have a package camera.
    pub async fn snapshot_with(&self, id: &CameraId, opts: &SnapshotOptions) -> Result<Bytes> {
        let mut path = format!("/v1/cameras/{id}/snapshot");
        let mut sep = '?';
        if let Some(ch) = opts.channel.as_ref() {
            path.push(sep);
            path.push_str("channel=");
            path.push_str(&ch.to_string());
            sep = '&';
        }
        if opts.high_quality {
            path.push(sep);
            path.push_str("highQuality=true");
        }
        let bytes = self.client.get_bytes(&path).await?;
        info!(
            "fetched snapshot for camera {id} ({} bytes, channel={:?}, high_quality={})",
            bytes.len(),
            opts.channel,
            opts.high_quality
        );
        Ok(bytes)
    }
}

impl ProtectClient {
    /// Camera read endpoints.
    #[must_use]
    pub const fn cameras(&self) -> CamerasApi<'_> {
        CamerasApi::new(self)
    }
}
