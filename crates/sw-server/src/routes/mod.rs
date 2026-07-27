mod admin;
mod auth;
mod chain;
mod games;
mod health;
mod leaderboard;
mod lobbies;
mod seasons;
mod users;

use axum::middleware;
use axum::routing::get;
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::middleware::request_boundary;
use crate::state::AppState;
use crate::ws;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(health::root))
        .merge(health::router())
        .nest("/auth", auth::router())
        .nest("/users", users::router())
        .nest("/games", games::router())
        .nest("/lobbies", lobbies::router())
        .nest("/seasons", seasons::router())
        .nest("/leaderboard", leaderboard::router())
        .nest("/chain", chain::router())
        .nest("/admin", admin::router())
        .merge(ws::router())
        .layer(middleware::from_fn(request_boundary))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state)
}
