//! Chime read endpoints. PATCH and action endpoints land in phases 5/6.

use log::{debug, info};

use crate::client::ProtectClient;
use crate::error::{Error, Result};
use crate::models::{Chime, ChimeId};

/// Chime-scoped API entry point.
pub struct ChimesApi<'a> {
    client: &'a ProtectClient,
}

impl<'a> ChimesApi<'a> {
    pub(crate) const fn new(client: &'a ProtectClient) -> Self {
        Self { client }
    }

    /// `GET /v1/chimes`. List every chime the NVR knows about.
    ///
    /// # Errors
    /// [`Error`] -- typically `Http` (network) or `Api` (4xx).
    pub async fn list(&self) -> Result<Vec<Chime>> {
        debug!("GET /v1/chimes");
        let resp = self
            .client
            .inner
            .get_chimes()
            .await
            .map_err(Error::from_progenitor)?;
        let chimes = resp.into_inner();
        info!("listed {} chime(s)", chimes.len());
        Ok(chimes)
    }

    /// `GET /v1/chimes/{id}`. Look up one chime by ID.
    ///
    /// # Errors
    /// [`Error`] -- typically `Http`, `Api { status: 404, .. }` for an
    /// unknown ID, or `Json` if the response body fails the schema.
    pub async fn get(&self, id: &ChimeId) -> Result<Chime> {
        debug!("GET /v1/chimes/{id}");
        let resp = self
            .client
            .inner
            .get_chimes_id(id)
            .await
            .map_err(Error::from_progenitor)?;
        let chime = resp.into_inner();
        info!("fetched chime {} ({})", chime.id, chime.name);
        Ok(chime)
    }
}

impl ProtectClient {
    /// Chime read endpoints.
    #[must_use]
    pub const fn chimes(&self) -> ChimesApi<'_> {
        ChimesApi::new(self)
    }
}
