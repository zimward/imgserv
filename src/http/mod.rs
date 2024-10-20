use std::sync::Arc;

use axum::{
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use sqlx::Pool;
use tokio::net::TcpListener;

use crate::Config;

mod img;
mod paste;

// #[derive(Clone)]
struct AppState {
    db: Pool<sqlx::Sqlite>,
    config: Config,
}

enum ApiError {
    NotFound,
    BadRequest,
    Internal(&'static str),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "The Requested resource doesn't exsist.",
            ),
            Self::BadRequest => (StatusCode::BAD_REQUEST, "Malformed Request"),
            Self::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        }
        .into_response()
    }
}

pub async fn serve(db: Pool<sqlx::Sqlite>, config: Config) {
    let state = Arc::new(AppState { db, config });
    let cors = tower_http::cors::CorsLayer::permissive();
    let router = Router::new()
        .route("/upload", post(img::upload))
        .route("/img/:id", get(img::get))
        .route("/paste/upload", post(paste::upload))
        .route("/paste/:id", get(paste::get))
        .with_state(state)
        .layer(cors);
    let listener = TcpListener::bind("[::]:8000").await.unwrap();
    axum::serve(listener, router).await.unwrap();
}
