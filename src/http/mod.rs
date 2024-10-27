use core::panic;
use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{header, HeaderMap, HeaderValue, Response, StatusCode},
    middleware::map_response,
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

fn type_from_name(name:&str)->&'static str{
    let parts =name.rsplit('.').nth(1).unwrap();
    match parts{
        "html"=>"text/html; charset=UTF-8",
        "css"=>"text/css",
        _=>{
            panic!("couldn't infer content type of static file {name}. Parsed as {parts}");
        }
    }
}

macro_rules! const_file {
    ($name:expr) => {
        || async move { serve_file(include_bytes!(concat!(env!("OUT_DIR"), $name)),type_from_name($name)) }
    };
}

pub fn serve_file(content: &[u8],content_type:&'static str) -> Response<Body> {
    // let html = include_bytes!(concat!(env!("OUT_DIR"), file));
    let vectorized = Vec::from(content);
    let mut response = Body::from(vectorized).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_ENCODING, HeaderValue::from_static("zstd"));
    response.headers_mut().insert(header::CONTENT_TYPE,HeaderValue::from_static(content_type) );
    response
}

async fn decomp(headers: HeaderMap, resp: Response<Body>) -> Response<Body> {
    if
    //does client support zstd compressed response?
    headers
        .get(header::ACCEPT_ENCODING)
        .map_or(true, |h| !h.to_str().unwrap_or_default().contains("zstd"))
        && 
        //is the response compressed?
        resp
            .headers()
            .get(header::CONTENT_ENCODING)
            .map_or(false, |h| h.to_str().unwrap_or_default().contains("zstd"))
    {
        //drop compressed header
        let (mut parts, body) = resp.into_parts();
        parts.headers.remove(header::CONTENT_ENCODING);
        //decompress, if errors the response was malformed to begin with and we should crash
        to_bytes(body, usize::MAX).await.map(
            |bytes| {
                //if that failes some function sets its headers wrong
                let dec = zstd::decode_all(&*bytes).unwrap();
                println!("Decompressed to {}",dec.len());
                parts.headers.insert(header::CONTENT_LENGTH,dec.len().to_string().parse().unwrap() );
let apparent=                parts.headers.get(header::CONTENT_LENGTH).unwrap();
    println!("header:{apparent:?}");
                Response::from_parts(parts, dec.into())
            },
        ).unwrap()
    } else {
        resp
    }
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
        .layer(cors)
        .layer(map_response(decomp));
    let listener = TcpListener::bind("[::]:8000").await.unwrap();
    axum::serve(listener, router).await.unwrap();
}
