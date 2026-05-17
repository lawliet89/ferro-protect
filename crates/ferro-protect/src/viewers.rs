//! Viewer read endpoints. PATCH lands in phase 5.

use log::info;

use crate::client::ProtectClient;
use crate::error::Result;
use crate::models::{Viewer, ViewerId};

/// Viewer-scoped API entry point. Cheap to construct; holds a borrow
/// of the [`ProtectClient`] that issued it.
pub struct ViewersApi<'a> {
    client: &'a ProtectClient,
}

impl<'a> ViewersApi<'a> {
    pub(crate) const fn new(client: &'a ProtectClient) -> Self {
        Self { client }
    }

    /// `GET /v1/viewers`. List every viewer the NVR knows about.
    ///
    /// # Errors
    /// [`Error`](crate::Error) -- typically `Http` (network) or `Api` (4xx).
    pub async fn list(&self) -> Result<Vec<Viewer>> {
        let viewers: Vec<Viewer> = self.client.get_json("/v1/viewers").await?;
        info!("listed {} viewer(s)", viewers.len());
        Ok(viewers)
    }

    /// `GET /v1/viewers/{id}`. Look up one viewer by ID.
    ///
    /// # Errors
    /// [`Error`](crate::Error) -- typically `Http`, `Api { status: 404, .. }`
    /// for an unknown ID, or `Json` if the response body fails the schema.
    pub async fn get(&self, id: &ViewerId) -> Result<Viewer> {
        let path = format!("/v1/viewers/{id}");
        let viewer: Viewer = self.client.get_json(&path).await?;
        info!("fetched viewer {} (name: {:?})", viewer.id, viewer.name);
        Ok(viewer)
    }
}

impl ProtectClient {
    /// Viewer read endpoints.
    #[must_use]
    pub const fn viewers(&self) -> ViewersApi<'_> {
        ViewersApi::new(self)
    }
}
