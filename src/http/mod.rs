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

use self::img::{get_img, upload};

mod img;

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
    let router = Router::new()
        .route("/upload", post(upload))
        .route("/img/:id", get(get_img))
        .with_state(state);
    let listener = TcpListener::bind("[::]:8000").await.unwrap();
    axum::serve(listener, router).await.unwrap();
}
