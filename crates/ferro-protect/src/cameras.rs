//! Camera read endpoints. PATCH and action endpoints land in phases 8/9.

use bytes::Bytes;
use log::info;
use serde::Serialize;

use crate::client::ProtectClient;
use crate::error::Result;
use crate::generated::{CreatedRtspsStreams, TalkbackSession as GeneratedTalkbackSession};
use crate::models::{
    Camera, CameraId, ChannelQuality, RtspsStream, SnapshotOptions, TalkbackSession,
};

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

    /// `POST /v1/cameras/{id}/rtsps-stream`. Create one RTSPS stream
    /// URL per requested quality level.
    ///
    /// HTTP verb is POST per the spec (the server allocates stream
    /// credentials), but no persistent NVR state is mutated — this
    /// is grouped with the read-shaped endpoints in phase 5. The
    /// matching DELETE for tearing down a stream lives in a later
    /// phase.
    ///
    /// `qualities` must be non-empty; the server rejects an empty
    /// list with a 4xx. `ChannelQuality::Package` is only valid for
    /// cameras with `hasPackageCamera: true`.
    ///
    /// The returned vec preserves the order in `qualities` and only
    /// contains entries the server actually populated.
    ///
    /// # Errors
    /// [`Error`] -- typically `Http`, `Api { status: 404, .. }` for
    /// an unknown camera, or `Api { status: 400, .. }` for an empty
    /// or unsupported quality list.
    pub async fn rtsps_stream(
        &self,
        id: &CameraId,
        qualities: &[ChannelQuality],
    ) -> Result<Vec<RtspsStream>> {
        #[derive(Serialize)]
        struct Body<'a> {
            qualities: &'a [ChannelQuality],
        }
        let path = format!("/v1/cameras/{id}/rtsps-stream");
        let created: CreatedRtspsStreams =
            self.client.post_json(&path, &Body { qualities }).await?;
        let streams = streams_in_request_order(qualities, &created);
        info!(
            "created {} RTSPS stream URL(s) for camera {id} (requested {} qualit{})",
            streams.len(),
            qualities.len(),
            if qualities.len() == 1 { "y" } else { "ies" }
        );
        Ok(streams)
    }

    /// `POST /v1/cameras/{id}/talkback-session`. Allocate a
    /// talkback WebSocket URL and return the audio config the
    /// caller will need to encode the stream.
    ///
    /// HTTP verb is POST per the spec (no request body; server
    /// allocates session credentials), but no persistent NVR state
    /// is mutated. Same phase-5 read-shaped framing as
    /// [`Self::rtsps_stream`].
    ///
    /// # Errors
    /// [`Error`] -- typically `Http`, `Api { status: 404, .. }` for
    /// an unknown camera.
    pub async fn talkback_session(&self, id: &CameraId) -> Result<TalkbackSession> {
        let path = format!("/v1/cameras/{id}/talkback-session");
        let raw: GeneratedTalkbackSession = self.client.post_empty_json(&path).await?;
        let session = TalkbackSession {
            bits_per_sample: *raw.bits_per_sample,
            codec: raw.codec.0,
            sampling_rate: *raw.sampling_rate,
            url: raw.url.0,
        };
        info!(
            "created talkback session for camera {id} (codec={}, sample_rate={}, bits_per_sample={})",
            session.codec, session.sampling_rate, session.bits_per_sample
        );
        Ok(session)
    }
}

/// Map the API's flat-object response into a vec ordered by the
/// caller's request. Entries the server returned `None` for are
/// silently dropped — the alternative (returning them as
/// `Option<String>` inside `RtspsStream`) would push that nullable
/// edge case onto every caller for no real benefit, since callers
/// that asked for `[High, Low]` want either a usable URL or an
/// indication the server didn't honour that quality.
fn streams_in_request_order(
    requested: &[ChannelQuality],
    created: &CreatedRtspsStreams,
) -> Vec<RtspsStream> {
    requested
        .iter()
        .filter_map(|q| {
            let url = match q {
                ChannelQuality::High => created.high.as_ref(),
                ChannelQuality::Medium => created.medium.as_ref(),
                ChannelQuality::Low => created.low.as_ref(),
                ChannelQuality::Package => created.package.as_ref(),
            }?;
            Some(RtspsStream {
                quality: *q,
                url: url.clone(),
            })
        })
        .collect()
}

impl ProtectClient {
    /// Camera read endpoints.
    #[must_use]
    pub const fn cameras(&self) -> CamerasApi<'_> {
        CamerasApi::new(self)
    }
}
