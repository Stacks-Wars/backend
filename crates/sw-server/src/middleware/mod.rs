//! Middleware boundaries.
//!
//! Request logging / CORS are attached at the router layer via `tower-http`.
//! Redis rate limits live in [`rate_limit`].

pub mod rate_limit;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use tracing::debug;

/// Lightweight request logging hook.
pub async fn request_boundary(req: Request, next: Next) -> Response {
    debug!(method = %req.method(), path = %req.uri().path(), "request");
    next.run(req).await
}
