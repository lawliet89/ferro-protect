#![forbid(unsafe_code)]
#![allow(
    clippy::pedantic,
    clippy::nursery,
    reason = "test files prioritise clarity over pedantic style"
)]

//! `client.viewers().list() / .get(id)` against a mock NVR.

use ferro_protect::models::ViewerId;
use ferro_protect::{Error, ProtectClient};
use secrecy::SecretString;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE_EMPTY_LIST: &str = include_str!("fixtures/viewers_list_empty.json");
const FIXTURE_NOT_FOUND: &str = include_str!("fixtures/viewer_not_found.json");

async fn client_for(server: &MockServer) -> ProtectClient {
    ProtectClient::builder()
        .base_url(server.uri())
        .api_key(SecretString::from("test-key".to_string()))
        .build()
        .expect("client builds")
}

#[tokio::test]
async fn list_returns_empty_vec_when_no_viewers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/viewers"))
        .and(header("x-api-key", "test-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(FIXTURE_EMPTY_LIST)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let viewers = client.viewers().list().await.expect("list call succeeds");
    assert!(viewers.is_empty());
}

#[tokio::test]
async fn get_nonexistent_viewer_maps_to_404_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/viewers/abc123"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string(FIXTURE_NOT_FOUND)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let id = ViewerId::from("abc123".to_string());
    let err = client
        .viewers()
        .get(&id)
        .await
        .expect_err("404 should error");
    match err {
        Error::Api {
            status,
            code,
            message,
        } => {
            assert_eq!(status, 404);
            assert_eq!(code, "notFound");
            assert!(
                message.contains("abc123"),
                "expected message to include id, got: {message}"
            );
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}
