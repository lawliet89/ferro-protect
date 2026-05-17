//! Light read endpoints. PATCH and action endpoints land in phases 5/6.

use log::info;

use crate::client::ProtectClient;
use crate::error::Result;
use crate::models::{Light, LightId};

/// Light-scoped API entry point. Cheap to construct; holds a borrow
/// of the [`ProtectClient`] that issued it.
pub struct LightsApi<'a> {
    client: &'a ProtectClient,
}

impl<'a> LightsApi<'a> {
    pub(crate) const fn new(client: &'a ProtectClient) -> Self {
        Self { client }
    }

    /// `GET /v1/lights`. List every light the NVR knows about.
    ///
    /// # Errors
    /// [`Error`](crate::Error) -- typically `Http` (network) or `Api` (4xx).
    pub async fn list(&self) -> Result<Vec<Light>> {
        let lights: Vec<Light> = self.client.get_json("/v1/lights").await?;
        info!("listed {} light(s)", lights.len());
        Ok(lights)
    }

    /// `GET /v1/lights/{id}`. Look up one light by ID.
    ///
    /// # Errors
    /// [`Error`](crate::Error) -- typically `Http`, `Api { status: 404, .. }`
    /// for an unknown ID, or `Json` if the response body fails the schema.
    pub async fn get(&self, id: &LightId) -> Result<Light> {
        let path = format!("/v1/lights/{id}");
        let light: Light = self.client.get_json(&path).await?;
        info!("fetched light {} (name: {:?})", light.id, light.name);
        Ok(light)
    }
}

impl ProtectClient {
    /// Light read endpoints.
    #[must_use]
    pub const fn lights(&self) -> LightsApi<'_> {
        LightsApi::new(self)
    }
}
