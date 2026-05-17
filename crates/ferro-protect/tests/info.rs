#![forbid(unsafe_code)]
#![allow(clippy::pedantic, clippy::nursery)]

//! `client.info()` against a mock NVR. Covers the happy path and a 401
//! error response so our response mapping is exercised end-to-end.

use ferro_protect::{Error, ProtectClient};
use secrecy::SecretString;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE_OK: &str = include_str!("fixtures/info_ok.json");
const FIXTURE_UNAUTH: &str = include_str!("fixtures/info_unauthorized.json");

async fn client_for(server: &MockServer) -> ProtectClient {
    ProtectClient::builder()
        .base_url(server.uri())
        .api_key(SecretString::from("test-key".to_string()))
        .build()
        .expect("client builds")
}

#[tokio::test]
async fn info_happy_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/meta/info"))
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
    let info = client.info().await.expect("info call succeeds");
    assert_eq!(info.application_version.to_string(), "6.2.83");
}

#[tokio::test]
async fn info_unauthorized_maps_to_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/meta/info"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_string(FIXTURE_UNAUTH)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let err = client.info().await.expect_err("401 should error");
    match err {
        Error::Api {
            status,
            code,
            message,
        } => {
            assert_eq!(status, 401);
            assert_eq!(code, "unauthorized");
            assert_eq!(message, "Invalid API key");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}
