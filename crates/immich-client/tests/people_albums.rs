//! Integration tests for `ImmichClient::list_people` / `get_album` /
//! `is_album_writable`. Each test spins a fresh `wiremock` server, stubs the
//! relevant endpoint(s), points an `ImmichClient` at it, and asserts the
//! observable behavior on the typed error / success path.
//!
//! These tests deliberately mirror the live Immich response shapes captured
//! when M2-T5 was implemented — including the quirk that 400 (not 404) means
//! "album not found or no access". A future Immich tightening that 400 to a
//! real 404 will still work because both statuses map to `Ok(None)`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use immich_client::{ImmichClient, ValidationError};
use reqwest::StatusCode;
use serde_json::json;
use url::Url;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const API_KEY: &str = "test-api-key";

fn client_for(server: &MockServer) -> ImmichClient {
    let base = Url::parse(&server.uri()).unwrap();
    ImmichClient::new(base)
}

#[tokio::test]
async fn list_people_single_page_ok() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/people"))
        .and(query_param("withHidden", "false"))
        .and(query_param("page", "1"))
        .and(header("x-api-key", API_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "people": [
                {"id": "p1", "name": "Alice"},
                {"id": "p2", "name": "Bob"}
            ],
            "hasNextPage": false,
            "total": 2,
            "hidden": 0
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server);
    let people = client.list_people(API_KEY).await.unwrap();
    assert_eq!(people.len(), 2);
    assert_eq!(people[0].id, "p1");
    assert_eq!(people[0].name, "Alice");
    assert_eq!(people[1].id, "p2");
}

#[tokio::test]
async fn list_people_paginates() {
    let server = MockServer::start().await;
    // page 1: hasNextPage=true
    Mock::given(method("GET"))
        .and(path("/api/people"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "people": [{"id": "p1", "name": "Alice"}],
            "hasNextPage": true,
            "total": 2,
            "hidden": 0
        })))
        .expect(1)
        .mount(&server)
        .await;
    // page 2: hasNextPage=false
    Mock::given(method("GET"))
        .and(path("/api/people"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "people": [{"id": "p2", "name": "Bob"}],
            "hasNextPage": false,
            "total": 2,
            "hidden": 0
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server);
    let people = client.list_people(API_KEY).await.unwrap();
    let ids: Vec<&str> = people.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec!["p1", "p2"]);
}

#[tokio::test]
async fn list_people_empty_page_ok() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/people"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "people": [],
            "hasNextPage": false,
            "total": 0,
            "hidden": 0
        })))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let people = client.list_people(API_KEY).await.unwrap();
    assert!(people.is_empty());
}

#[tokio::test]
async fn list_people_unauthorized_maps_typed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/people"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "message": "Invalid API key",
            "statusCode": 401
        })))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let err = client.list_people(API_KEY).await.unwrap_err();
    match err {
        ValidationError::Unauthorized(s) => assert_eq!(s, StatusCode::UNAUTHORIZED),
        other => panic!("expected Unauthorized, got {other:?}"),
    }
}

#[tokio::test]
async fn list_people_forbidden_maps_typed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/people"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let err = client.list_people(API_KEY).await.unwrap_err();
    assert!(matches!(err, ValidationError::Unauthorized(_)));
}

#[tokio::test]
async fn list_people_5xx_maps_upstream() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/people"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let err = client.list_people(API_KEY).await.unwrap_err();
    match err {
        ValidationError::Upstream { status } => {
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        }
        other => panic!("expected Upstream, got {other:?}"),
    }
}

#[tokio::test]
async fn list_people_malformed_json_maps_bad_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/people"))
        // Body shape is wrong — `people` is a string, not an array.
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "people": "not-an-array",
            "hasNextPage": false
        })))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let err = client.list_people(API_KEY).await.unwrap_err();
    assert!(
        matches!(err, ValidationError::BadResponse(_)),
        "expected BadResponse, got {err:?}"
    );
}

#[tokio::test]
async fn get_album_owner_ok() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/albums/aaa"))
        .and(query_param("withoutAssets", "true"))
        .and(header("x-api-key", API_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "aaa",
            "ownerId": "owner-1",
            "albumUsers": []
        })))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let album = client.get_album(API_KEY, "aaa").await.unwrap().unwrap();
    assert_eq!(album.id, "aaa");
    assert_eq!(album.owner_id, "owner-1");
    assert!(album.album_users.is_empty());
}

#[tokio::test]
async fn get_album_with_shared_users_parses() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/albums/bbb"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "bbb",
            "ownerId": "owner-1",
            "albumUsers": [
                {"user": {"id": "shared-1"}, "role": "editor"},
                {"user": {"id": "shared-2"}, "role": "viewer"}
            ]
        })))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let album = client.get_album(API_KEY, "bbb").await.unwrap().unwrap();
    assert_eq!(album.album_users.len(), 2);
    assert_eq!(album.album_users[0].user_id, "shared-1");
    assert_eq!(album.album_users[0].role, "editor");
    assert_eq!(album.album_users[1].user_id, "shared-2");
    assert_eq!(album.album_users[1].role, "viewer");
}

#[tokio::test]
async fn get_album_400_not_found_returns_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/albums/ghost"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "message": "Not found or no album.read access",
            "error": "Bad Request",
            "statusCode": 400
        })))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let album = client.get_album(API_KEY, "ghost").await.unwrap();
    assert!(album.is_none());
}

#[tokio::test]
async fn get_album_404_returns_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/albums/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let album = client.get_album(API_KEY, "missing").await.unwrap();
    assert!(album.is_none());
}

#[tokio::test]
async fn get_album_401_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/albums/aaa"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let err = client.get_album(API_KEY, "aaa").await.unwrap_err();
    assert!(matches!(err, ValidationError::Unauthorized(_)));
}

#[tokio::test]
async fn get_album_5xx_upstream() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/albums/aaa"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let err = client.get_album(API_KEY, "aaa").await.unwrap_err();
    assert!(matches!(
        err,
        ValidationError::Upstream {
            status: StatusCode::INTERNAL_SERVER_ERROR,
        }
    ));
}

#[tokio::test]
async fn is_album_writable_owner_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/albums/own"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "own",
            "ownerId": "caller-user",
            "albumUsers": []
        })))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let writable = client
        .is_album_writable(API_KEY, "caller-user", "own")
        .await
        .unwrap();
    assert!(writable);
}

#[tokio::test]
async fn is_album_writable_editor_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/albums/sharedX"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sharedX",
            "ownerId": "other-user",
            "albumUsers": [
                {"user": {"id": "caller-user"}, "role": "editor"}
            ]
        })))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let writable = client
        .is_album_writable(API_KEY, "caller-user", "sharedX")
        .await
        .unwrap();
    assert!(writable);
}

#[tokio::test]
async fn is_album_writable_viewer_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/albums/sharedR"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "sharedR",
            "ownerId": "other-user",
            "albumUsers": [
                {"user": {"id": "caller-user"}, "role": "viewer"}
            ]
        })))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let writable = client
        .is_album_writable(API_KEY, "caller-user", "sharedR")
        .await
        .unwrap();
    assert!(!writable);
}

#[tokio::test]
async fn is_album_writable_unrelated_user_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/albums/foreign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "foreign",
            "ownerId": "other-user",
            "albumUsers": []
        })))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let writable = client
        .is_album_writable(API_KEY, "caller-user", "foreign")
        .await
        .unwrap();
    assert!(!writable);
}

#[tokio::test]
async fn is_album_writable_missing_album_false() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/albums/ghost"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "message": "Not found or no album.read access",
            "statusCode": 400
        })))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let writable = client
        .is_album_writable(API_KEY, "caller-user", "ghost")
        .await
        .unwrap();
    assert!(!writable);
}

#[tokio::test]
async fn is_album_writable_unauthorized_bubbles() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/albums/x"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let client = client_for(&server);
    let err = client
        .is_album_writable(API_KEY, "caller-user", "x")
        .await
        .unwrap_err();
    assert!(matches!(err, ValidationError::Unauthorized(_)));
}
