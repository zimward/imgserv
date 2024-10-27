use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, HeaderValue, Response, StatusCode},
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

macro_rules! const_file {
    ($name:expr) => {
        || async move { serve_file(include_bytes!(concat!(env!("OUT_DIR"), $name))) }
    };
}

pub fn serve_file(content: &[u8]) -> Response<Body> {
    // let html = include_bytes!(concat!(env!("OUT_DIR"), file));
    let vectorized = Vec::from(content);
    let mut response = Body::from(vectorized).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_ENCODING, HeaderValue::from_static("zstd"));
    response
}

pub async fn serve(db: Pool<sqlx::Sqlite>, config: Config) {
    let state = Arc::new(AppState { db, config });
    let cors = tower_http::cors::CorsLayer::permissive();
    let router = Router::new()
        .route("/upload", post(img::upload))
        .route("/img/:id", get(img::get))
        .route("/paste/upload", post(paste::direct_upload))
        .route("/paste/:id", get(paste::get))
        .route(
            "/paste",
            get(const_file!("/paste.html.zstd")).post(paste::form_resp),
        )
        .with_state(state)
        .layer(cors);
    let listener = TcpListener::bind("[::]:8000").await.unwrap();
    axum::serve(listener, router).await.unwrap();
}
