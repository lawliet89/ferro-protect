#![forbid(unsafe_code)]
#![allow(clippy::pedantic, clippy::nursery)]

//! `client.nvrs().get()` against a mock NVR. Unlike most entities,
//! `GET /v1/nvrs` returns a single object (not a list); see `src/nvrs.rs`
//! for the rationale.

use ferro_protect::ProtectClient;
use secrecy::SecretString;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE_OK: &str = include_str!("fixtures/nvr_ok.json");

async fn client_for(server: &MockServer) -> ProtectClient {
    ProtectClient::builder()
        .base_url(server.uri())
        .api_key(SecretString::from("test-key".to_string()))
        .build()
        .expect("client builds")
}

#[tokio::test]
async fn get_returns_singleton_nvr() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/nvrs"))
        .and(header("x-api-key", "test-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(FIXTURE_OK)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let nvr = client.nvrs().get().await.expect("get call succeeds");
    assert_eq!(nvr.id.to_string(), "test-nvr-1");
}
