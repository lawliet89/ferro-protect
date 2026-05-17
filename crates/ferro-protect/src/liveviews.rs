//! Liveview read endpoints. PATCH/POST/DELETE land in phase 5.

use log::info;

use crate::client::ProtectClient;
use crate::error::Result;
use crate::models::{Liveview, LiveviewId};

/// Liveview-scoped API entry point. Cheap to construct; holds a borrow
/// of the [`ProtectClient`] that issued it.
pub struct LiveviewsApi<'a> {
    client: &'a ProtectClient,
}

impl<'a> LiveviewsApi<'a> {
    pub(crate) const fn new(client: &'a ProtectClient) -> Self {
        Self { client }
    }

    /// `GET /v1/liveviews`. List every liveview the NVR has.
    ///
    /// # Errors
    /// [`Error`](crate::Error) -- typically `Http` (network) or `Api` (4xx).
    pub async fn list(&self) -> Result<Vec<Liveview>> {
        let liveviews: Vec<Liveview> = self.client.get_json("/v1/liveviews").await?;
        info!("listed {} liveview(s)", liveviews.len());
        Ok(liveviews)
    }

    /// `GET /v1/liveviews/{id}`. Look up one liveview by ID.
    ///
    /// # Errors
    /// [`Error`](crate::Error) -- typically `Http`, `Api { status: 404, .. }`
    /// for an unknown ID, or `Json` if the response body fails the schema.
    pub async fn get(&self, id: &LiveviewId) -> Result<Liveview> {
        let path = format!("/v1/liveviews/{id}");
        let liveview: Liveview = self.client.get_json(&path).await?;
        info!("fetched liveview {} ({:?})", liveview.id, liveview.name);
        Ok(liveview)
    }
}

impl ProtectClient {
    /// Liveview read endpoints.
    #[must_use]
    pub const fn liveviews(&self) -> LiveviewsApi<'_> {
        LiveviewsApi::new(self)
    }
}
