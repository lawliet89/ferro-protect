//! Camera read endpoints. PATCH and action endpoints land in phases 5/6/7.

use log::{debug, info};

use crate::client::ProtectClient;
use crate::error::Result;
use crate::models::{Camera, CameraId};

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
        debug!("GET {path}");
        let camera: Camera = self.client.get_json(&path).await?;
        info!("fetched camera {}", camera.id);
        Ok(camera)
    }
}

impl ProtectClient {
    /// Camera read endpoints.
    #[must_use]
    pub const fn cameras(&self) -> CamerasApi<'_> {
        CamerasApi::new(self)
    }
}
