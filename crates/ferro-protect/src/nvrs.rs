//! NVR read endpoint. `GET /v1/nvrs` returns a single `Nvr` object (not
//! an array), so the wrapper exposes a `get()` returning `Nvr` rather
//! than a list. PLAN.md originally wrote this entity up as
//! `list` + `get` per the other-entity template; the spec only defines
//! one endpoint and it is singular -- one NVR per installation.

use log::info;

use crate::client::ProtectClient;
use crate::error::Result;
use crate::models::Nvr;

/// NVR-scoped API entry point. Cheap to construct; holds a borrow
/// of the [`ProtectClient`] that issued it.
pub struct NvrsApi<'a> {
    client: &'a ProtectClient,
}

impl<'a> NvrsApi<'a> {
    pub(crate) const fn new(client: &'a ProtectClient) -> Self {
        Self { client }
    }

    /// `GET /v1/nvrs`. Fetch the NVR for this installation.
    ///
    /// The Protect API exposes one NVR per installation; the response is
    /// a single object, not an array (despite the plural path).
    ///
    /// # Errors
    /// [`Error`](crate::Error) -- typically `Http` (network) or `Api` (4xx).
    pub async fn get(&self) -> Result<Nvr> {
        let nvr: Nvr = self.client.get_json("/v1/nvrs").await?;
        info!("fetched nvr {} (name: {:?})", nvr.id, nvr.name);
        Ok(nvr)
    }
}

impl ProtectClient {
    /// NVR read endpoint.
    #[must_use]
    pub const fn nvrs(&self) -> NvrsApi<'_> {
        NvrsApi::new(self)
    }
}
