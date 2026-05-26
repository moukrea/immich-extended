//! Integration tests for `ImmichClient::download_thumbnail` and
//! `download_original` (M5-T5). Both methods are thin GET-and-collect-bytes
//! wrappers; the tests assert byte fidelity on 200 and the standard
//! Unauthorized/Upstream error mapping on the failure paths.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use immich_client::{ImmichClient, ValidationError};
use reqwest::StatusCode;
use url::Url;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const API_KEY: &str = "TESTKEY";

fn client_for(server: &MockServer) -> ImmichClient {
    let base = Url::parse(&server.uri()).unwrap();
    ImmichClient::new(base)
}

// 4-byte JFIF/JPEG SOI + APP0 segment start. Enough to prove the body bytes
// round-trip without bundling a real JPEG into the test source.
const JPEG_HEADER: &[u8] = &[0xff, 0xd8, 0xff, 0xe0];

#[tokio::test]
async fn download_thumbnail_returns_bytes_on_200() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/assets/abc/thumbnail"))
        .and(query_param("size", "preview"))
        .and(header("x-api-key", API_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(JPEG_HEADER))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server);
    let bytes = client.download_thumbnail(API_KEY, "abc").await.unwrap();
    assert_eq!(bytes, JPEG_HEADER);
}

#[tokio::test]
async fn download_thumbnail_returns_error_on_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/assets/missing/thumbnail"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let err = client
        .download_thumbnail(API_KEY, "missing")
        .await
        .unwrap_err();
    match err {
        ValidationError::Upstream { status } => assert_eq!(status, StatusCode::NOT_FOUND),
        other => panic!("expected Upstream(404), got {other:?}"),
    }
}

#[tokio::test]
async fn download_thumbnail_401_maps_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/assets/abc/thumbnail"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let err = client.download_thumbnail(API_KEY, "abc").await.unwrap_err();
    assert!(matches!(err, ValidationError::Unauthorized(_)));
}

#[tokio::test]
async fn download_original_returns_bytes_on_200() {
    let server = MockServer::start().await;
    let body: Vec<u8> = (0u8..32).collect();
    Mock::given(method("GET"))
        .and(path("/api/assets/abc/original"))
        .and(header("x-api-key", API_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server);
    let bytes = client.download_original(API_KEY, "abc").await.unwrap();
    assert_eq!(bytes, body);
}

#[tokio::test]
async fn download_original_returns_error_on_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/assets/missing/original"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let err = client
        .download_original(API_KEY, "missing")
        .await
        .unwrap_err();
    match err {
        ValidationError::Upstream { status } => assert_eq!(status, StatusCode::NOT_FOUND),
        other => panic!("expected Upstream(404), got {other:?}"),
    }
}

#[tokio::test]
async fn download_original_500_maps_upstream() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/assets/abc/original"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let err = client.download_original(API_KEY, "abc").await.unwrap_err();
    assert!(
        matches!(err, ValidationError::Upstream { status } if status == StatusCode::INTERNAL_SERVER_ERROR),
    );
}
