mod admin;
mod games;
mod health;
mod leaderboard;
mod lobbies;
mod seasons;
mod users;
mod wallet;

use axum::middleware;
use axum::routing::get;
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::middleware::rate_limit;
use crate::middleware::request_boundary;
use crate::state::AppState;
use crate::ws;

pub fn router(state: AppState) -> Router {
    let sensitive = Router::new()
        .nest("/lobbies", lobbies::sensitive_router())
        .nest("/wallet", wallet::sensitive_router())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit::sensitive_limit,
        ));

    let write = Router::new()
        .nest("/lobbies", lobbies::write_router())
        .nest("/users", users::write_router())
        .nest("/wallet", wallet::write_router())
        .nest("/admin", admin::write_router())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit::write_limit,
        ));

    let limited = Router::new()
        .merge(sensitive)
        .merge(write)
        .nest("/users", users::read_router())
        .nest("/games", games::router())
        .nest("/lobbies", lobbies::read_router())
        .nest("/seasons", seasons::router())
        .nest("/leaderboard", leaderboard::router())
        .nest("/wallet", wallet::read_router())
        .nest("/admin", admin::read_router())
        .merge(ws::router())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit::global_limit,
        ));

    Router::new()
        .route("/", get(health::root))
        .merge(health::router())
        .merge(limited)
        .layer(middleware::from_fn(request_boundary))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state)
}
