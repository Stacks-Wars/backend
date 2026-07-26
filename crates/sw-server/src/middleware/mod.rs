//! Middleware boundaries.
//!
//! Request logging / CORS are attached at the router layer via `tower-http`.
//! Domain-specific middleware (rate limits, admin gates, etc.) can land here later.

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use tracing::debug;

/// Placeholder middleware that proves the hook point without enforcing policy.
pub async fn request_boundary(req: Request, next: Next) -> Response {
    debug!(method = %req.method(), path = %req.uri().path(), "request");
    next.run(req).await
}
