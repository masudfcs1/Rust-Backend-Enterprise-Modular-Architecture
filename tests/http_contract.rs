use std::{collections::HashMap, net::SocketAddr, time::Duration};

use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
};
use enterprise_rust_backend::{
    bootstrap::{AppState, build_app},
    config::Config,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

async fn app(rate_limit: u32) -> Router {
    let values: HashMap<_, _> = [
        (
            "DATABASE_URL",
            "postgres://app:unused@127.0.0.1:1/test?sslmode=disable".to_owned(),
        ),
        (
            "JWT_SECRET",
            "test-only-random-looking-secret-e48bb939c22b4085".to_owned(),
        ),
        ("CORS_ALLOWED_ORIGINS", "https://client.example".to_owned()),
        ("AUTH_REQUESTS_PER_MINUTE", rate_limit.to_string()),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect();
    let config = Config::from_values(&values).unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(50))
        .connect_lazy(&config.database_url)
        .unwrap();
    build_app(AppState::new(pool, config).await.unwrap())
}

fn request(method: &str, uri: &str, body: Body) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .body(body)
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:45000".parse::<SocketAddr>().unwrap(),
    ));
    request
}

async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn health_errors_and_security_headers_follow_the_http_contract() {
    let app = app(20).await;
    let mut req = request("GET", "/health/live", Body::empty());
    req.headers_mut()
        .insert("x-request-id", "untrusted-client-value".parse().unwrap());
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(uuid::Uuid::parse_str(response.headers()["x-request-id"].to_str().unwrap()).is_ok());
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(json_body(response).await, json!({"data": {"status": "ok"}}));

    for (method, uri, status, code) in [
        (
            "GET",
            "/does-not-exist?secret=must-not-echo",
            StatusCode::NOT_FOUND,
            "not_found",
        ),
        (
            "DELETE",
            "/health/live",
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
        ),
        (
            "GET",
            "/api/v1/users/me",
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        ),
        ("GET", "/api/v1/users", StatusCode::NOT_FOUND, "not_found"),
        (
            "GET",
            "/api/v1/users/",
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        ),
        (
            "GET",
            "/health/ready",
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(request(method, uri, Body::empty()))
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert!(response.headers().contains_key("x-request-id"));
        if status == StatusCode::UNAUTHORIZED {
            assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
        }
        let body = json_body(response).await;
        assert_eq!(body["error"]["code"], code);
        assert!(!body.to_string().contains("must-not-echo"));
    }
}

#[tokio::test]
async fn invalid_credentials_and_payloads_fail_before_database_access() {
    let app = app(20).await;
    for body in [
        json!({"email":"person@example.com","display_name":"Person","password":"short"}),
        json!({"email":"person@example.com","display_name":"Person","password":"long secret passphrase","role":"admin"}),
        json!({"email":"invalid","display_name":"Person","password":"long secret passphrase"}),
    ] {
        let mut req = request(
            "POST",
            "/api/v1/auth/register",
            Body::from(body.to_string()),
        );
        req.headers_mut()
            .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let error = json_body(response).await.to_string();
        assert!(!error.contains("long secret passphrase"));
    }
    let response = app
        .clone()
        .oneshot(request("POST", "/api/v1/auth/login", Body::from("{}")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "unsupported_media_type"
    );

    let mut req = request("POST", "/api/v1/auth/login", Body::from("x".repeat(20_000)));
    req.headers_mut()
        .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "payload_too_large"
    );

    for authorization in [
        "Basic abc",
        "Bearer ",
        "Bearer forged.jwt.token",
        "Bearer too many fields",
    ] {
        let mut req = request("GET", "/api/v1/users/me", Body::empty());
        req.headers_mut()
            .insert(header::AUTHORIZATION, authorization.parse().unwrap());
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn cors_is_an_allowlist_and_forwarded_headers_cannot_bypass_rate_limits() {
    let app = app(1).await;
    for (origin, allowed) in [
        ("https://client.example", true),
        ("https://attacker.example", false),
    ] {
        let mut req = request("OPTIONS", "/api/v1/auth/login", Body::empty());
        req.headers_mut()
            .insert(header::ORIGIN, origin.parse().unwrap());
        req.headers_mut().insert(
            header::ACCESS_CONTROL_REQUEST_METHOD,
            "POST".parse().unwrap(),
        );
        req.headers_mut().insert(
            header::ACCESS_CONTROL_REQUEST_HEADERS,
            "content-type".parse().unwrap(),
        );
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            allowed
        );
    }
    for (index, status) in [
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        StatusCode::TOO_MANY_REQUESTS,
    ]
    .into_iter()
    .enumerate()
    {
        let mut req = request("POST", "/api/v1/auth/login", Body::from("{}"));
        req.headers_mut().insert(
            "x-forwarded-for",
            format!("192.0.2.{}", index + 1).parse().unwrap(),
        );
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), status);
        if status == StatusCode::TOO_MANY_REQUESTS {
            assert_eq!(response.headers()[header::RETRY_AFTER], "60");
            assert_eq!(json_body(response).await["error"]["code"], "rate_limited");
        }
    }
}
