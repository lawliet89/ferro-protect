//! Device asset file read endpoint. Upload lands in phase 10.

use log::info;

use crate::client::ProtectClient;
use crate::error::Result;
use crate::models::{AssetFile, AssetFileType};

/// Device-asset-file API entry point. Cheap to construct; holds a
/// borrow of the [`ProtectClient`] that issued it.
pub struct FilesApi<'a> {
    client: &'a ProtectClient,
}

impl<'a> FilesApi<'a> {
    pub(crate) const fn new(client: &'a ProtectClient) -> Self {
        Self { client }
    }

    /// `GET /v1/files/{fileType}`. List every device asset file of one
    /// type.
    ///
    /// The type is an enum rather than a string because Protect 7.3.70
    /// answers an unknown `fileType` with `200 []` instead of an error,
    /// so a typo would otherwise look like an empty list.
    ///
    /// # Errors
    /// [`Error`](crate::Error) -- typically `Http` (network) or `Api` (4xx).
    pub async fn list(&self, file_type: AssetFileType) -> Result<Vec<AssetFile>> {
        let path = format!("/v1/files/{file_type}");
        let files: Vec<AssetFile> = self.client.get_json(&path).await?;
        info!("listed {} {file_type} file(s)", files.len());
        Ok(files)
    }
}

impl ProtectClient {
    /// Device asset file endpoints.
    #[must_use]
    pub const fn files(&self) -> FilesApi<'_> {
        FilesApi::new(self)
    }
}
