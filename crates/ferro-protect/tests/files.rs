#![forbid(unsafe_code)]
#![allow(
    clippy::pedantic,
    clippy::nursery,
    reason = "test files prioritise clarity over pedantic style"
)]

//! `client.files().list(file_type)` against a mock NVR.

use ferro_protect::models::AssetFileType;
use ferro_protect::{Error, ProtectClient};
use secrecy::SecretString;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Sanitized live capture from a 7.3.70 NVR. Keeps the fields the spec
/// does not declare (`id`, `size`, `createdAt`, ...) so the test proves
/// they are tolerated, and drops `originalName` from the second entry to
/// cover the optional field.
const FIXTURE_LIST_OK: &str = include_str!("fixtures/files_list_ok.json");

async fn client_for(server: &MockServer) -> ProtectClient {
    ProtectClient::builder()
        .base_url(server.uri())
        .api_key(SecretString::from("test-key".to_string()))
        .build()
        .expect("client builds")
}

#[tokio::test]
async fn list_animations_parses_live_shaped_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/files/animations"))
        .and(header("x-api-key", "test-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(FIXTURE_LIST_OK)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let files = client
        .files()
        .list(AssetFileType::Animations)
        .await
        .expect("list call succeeds");

    assert_eq!(files.len(), 2);
    assert_eq!(files[0].type_, AssetFileType::Animations);
    assert_eq!(
        files[0].name.as_str(),
        "4af06290-0fe5-11af-32b9-2b402c5e86ca.png"
    );
    assert_eq!(
        files[0].original_name.as_deref().map(String::as_str),
        Some("welcome.png")
    );
    assert!(
        files[0]
            .path
            .starts_with("/srv/unifi-protect/storage/animations/")
    );
    assert!(files[1].original_name.is_none());
}

#[tokio::test]
async fn list_returns_empty_vec_when_no_files() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/files/animations"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("[]")
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let files = client
        .files()
        .list(AssetFileType::Animations)
        .await
        .expect("list call succeeds");
    assert!(files.is_empty());
}

#[tokio::test]
async fn list_maps_non_2xx_to_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/files/animations"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_string(r#"{"name":"forbidden","error":"Insufficient permissions"}"#)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let err = client
        .files()
        .list(AssetFileType::Animations)
        .await
        .expect_err("403 should error");
    match err {
        Error::Api { status, code, .. } => {
            assert_eq!(status, 403);
            assert_eq!(code, "forbidden");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}
