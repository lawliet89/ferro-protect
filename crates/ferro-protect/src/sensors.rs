//! Sensor read endpoints. PATCH lands in phase 5.

use log::info;

use crate::client::ProtectClient;
use crate::error::Result;
use crate::models::{Sensor, SensorId};

/// Sensor-scoped API entry point. Cheap to construct; holds a borrow
/// of the [`ProtectClient`] that issued it.
pub struct SensorsApi<'a> {
    client: &'a ProtectClient,
}

impl<'a> SensorsApi<'a> {
    pub(crate) const fn new(client: &'a ProtectClient) -> Self {
        Self { client }
    }

    /// `GET /v1/sensors`. List every sensor the NVR knows about.
    ///
    /// # Errors
    /// [`Error`](crate::Error) -- typically `Http` (network) or `Api` (4xx).
    pub async fn list(&self) -> Result<Vec<Sensor>> {
        let sensors: Vec<Sensor> = self.client.get_json("/v1/sensors").await?;
        info!("listed {} sensor(s)", sensors.len());
        Ok(sensors)
    }

    /// `GET /v1/sensors/{id}`. Look up one sensor by ID.
    ///
    /// # Errors
    /// [`Error`](crate::Error) -- typically `Http`, `Api { status: 404, .. }`
    /// for an unknown ID, or `Json` if the response body fails the schema.
    pub async fn get(&self, id: &SensorId) -> Result<Sensor> {
        let path = format!("/v1/sensors/{id}");
        let sensor: Sensor = self.client.get_json(&path).await?;
        info!("fetched sensor {} (name: {:?})", sensor.id, sensor.name);
        Ok(sensor)
    }
}

impl ProtectClient {
    /// Sensor read endpoints.
    #[must_use]
    pub const fn sensors(&self) -> SensorsApi<'_> {
        SensorsApi::new(self)
    }
}
