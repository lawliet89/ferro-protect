//! Camera read endpoints. PATCH and action endpoints land in phases 5/6/7.

use crate::client::ProtectClient;
use crate::error::{Error, Result};
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
        let resp = self
            .client
            .inner
            .get_cameras()
            .await
            .map_err(Error::from_progenitor)?;
        Ok(resp.into_inner())
    }

    /// `GET /v1/cameras/{id}`. Look up one camera by ID.
    ///
    /// # Errors
    /// [`Error`] -- typically `Http`, `Api { status: 404, .. }` for an
    /// unknown ID, or `Json` if the response body fails the schema.
    pub async fn get(&self, id: &CameraId) -> Result<Camera> {
        let resp = self
            .client
            .inner
            .get_cameras_id(id)
            .await
            .map_err(Error::from_progenitor)?;
        Ok(resp.into_inner())
    }
}

impl ProtectClient {
    /// Camera read endpoints.
    #[must_use]
    pub const fn cameras(&self) -> CamerasApi<'_> {
        CamerasApi::new(self)
    }
}
