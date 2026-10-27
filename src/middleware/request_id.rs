use axum::{
    extract::{MatchedPath, Request},
    http::{HeaderName, HeaderValue, header},
    middleware::Next,
    response::Response,
};
use std::time::Instant;
use tracing::Instrument;
use uuid::Uuid;

pub async fn observe(mut request: Request, next: Next) -> Response {
    // Generate server-owned IDs rather than trusting arbitrary client input.
    let id = Uuid::new_v4().to_string();
    let id_header = HeaderValue::from_str(&id).expect("UUID is an ASCII header value");
    request
        .headers_mut()
        .insert(HeaderName::from_static("x-request-id"), id_header.clone());
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str())
        .unwrap_or("unmatched")
        .to_owned();
    // Log route templates, never URLs, query strings, headers, or body data.
    let span = tracing::info_span!("http_request", request_id = %id, method = %request.method(), route = %route);
    async move {
        let started = Instant::now();
        let mut response = next.run(request).await;
        tracing::info!(
            status = response.status().as_u16(),
            duration_ms = started.elapsed().as_millis() as u64,
            "request completed"
        );
        let headers = response.headers_mut();
        headers.insert(HeaderName::from_static("x-request-id"), id_header);
        headers.insert(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        );
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    }
    .instrument(span)
    .await
}
