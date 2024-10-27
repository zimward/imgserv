use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    body::{Body, HttpBody},
    extract::{Path, Request, State},
    http::{header, HeaderValue, Response, StatusCode},
    Form,
};
use futures::TryStreamExt;
use serde::Deserialize;
use tokio::io::AsyncReadExt;
use tokio_util::io::StreamReader;

use super::{ApiError, AppState};

pub async fn direct_upload(
    state: State<Arc<AppState>>,
    request: Request,
) -> Result<(StatusCode, String), ApiError> {
    let ds = request.into_body().into_data_stream();
    if ds.is_end_stream() {
        return Err(ApiError::BadRequest);
    }

    let mut paste = String::default();
    let mut reader = StreamReader::new(
        ds.into_stream()
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::BrokenPipe, err)),
    );
    if reader.read_to_string(&mut paste).await.is_err() {
        return Err(ApiError::BadRequest);
    }
    Ok((StatusCode::OK, upload(state, &paste).await?))
}

async fn upload<'a>(
    State(state): State<Arc<AppState>>,
    paste: &'a str,
) -> Result<String, ApiError> {
    let db = &state.db;
    let paste = zstd::bulk::compress(paste.as_bytes(), 19);
    if let Ok(paste) = paste {
        //crash if time or id fails
        let time: i64 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            //you and i should be long dead when this conversion starts failing (and sqlite supports unsigned ints then)
            .map(|val| i64::try_from((val + state.config.paste_ttl).as_secs()).unwrap())
            .map_err(|_| ApiError::Internal("Welcome at the end of time!"))?;
        let id: i64 = sqlx::query!(
            "INSERT INTO pastes (expires,text) VALUES (?,?) RETURNING id",
            time,
            paste
        )
        .fetch_one(db)
        .await
        .map_err(|_| ApiError::Internal("DB write failed"))?
        .id;
        Ok(format!("{}/paste/{id}", state.config.url))
    } else {
        Err(ApiError::Internal("Compression failed"))
    }
}

#[derive(Deserialize)]
pub struct ID(i64);
pub async fn get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<ID>,
) -> Result<Response<Body>, ApiError> {
    let data = sqlx::query!("SELECT text FROM pastes WHERE id == ?", id.0)
        .fetch_one(&state.db)
        .await
        .map_err(|_| ApiError::NotFound)?
        .text;
    let mut response = Response::new(Body::from(data));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CONTENT_ENCODING, HeaderValue::from_static("zstd"));
    Ok(response)
}

#[derive(Deserialize)]
pub struct Upload {
    text: String,
}
pub async fn form_resp<'a>(
    state: State<Arc<AppState>>,
    Form(form): Form<Upload>,
) -> Result<Response<Body>, ApiError> {
    let url = upload(state, &form.text).await?;
    let mut resp = Response::new(Body::empty());
    resp.headers_mut()
        .insert(header::LOCATION, url.parse().unwrap());
    *resp.status_mut() = StatusCode::SEE_OTHER;
    Ok(resp)
}
