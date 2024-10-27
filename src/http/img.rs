use std::{
    io,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    body::{Body, HttpBody},
    extract::{Path, Request, State},
    http::{header, HeaderValue, Response, StatusCode},
};
use futures::TryStreamExt;
use serde::Deserialize;
use tokio::{fs::File, io::BufWriter};
use tokio_util::io::{ReaderStream, StreamReader};

use super::{ApiError, AppState};

#[derive(Deserialize)]
pub struct ImageID(u64);

pub async fn get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<ImageID>,
) -> Result<Response<Body>, ApiError> {
    let config = &state.config;
    let path = config.data_dir.join(format!("data/{}", id.0));
    //parse mime type from first 4kib, no longer a need to store the mime-type or check it during upload
    let format = async { tree_magic_mini::from_filepath(&path) }.await;
    if let Some(format) = format {
        //this failing would mean database corruption
        let file = File::open(path).await.expect("Failed to read from disk");
        let size = file.metadata().await.map(|meta| meta.len()).unwrap_or(0);
        let reader = ReaderStream::new(file);
        let body = Body::from_stream(reader);
        let mut resp = Response::new(body);
        resp.headers_mut().insert(
            header::CONTENT_TYPE,
            format
                .parse()
                .map_err(|_| ApiError::Internal("Parsing mime-type failed"))?,
        );
        //we havent read from the stream so its probably ok
        resp.headers_mut()
            .insert(header::CONTENT_LENGTH, HeaderValue::from(size));
        resp.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("*"),
        );
        //allow caching for the ttl of images, might lead to doubleing of ttl
        //on clients, but ttl is only meant to keep disk usage low
        resp.headers_mut().insert(
            header::CACHE_CONTROL,
            format!("max-age={}", config.image_ttl.as_secs())
                .as_str()
                .parse()
                .expect("max-age parsing failed"),
        );
        Ok(resp)
    } else {
        //this means the file does not exist, as every file is a octet-stream
        Err(ApiError::NotFound)
    }
}

pub async fn upload(
    State(state): State<Arc<AppState>>,
    request: Request,
) -> Result<(StatusCode, String), ApiError> {
    let db = &state.db;
    let ds = request.into_body().into_data_stream();
    //terminate if stream is empty
    if ds.is_end_stream() {
        return Err(ApiError::BadRequest);
    }

    //crash if time or id fails
    let time: i64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        //you and i should be long dead when this conversion starts failing (and sqlite supports unsigned ints then)
        .map(|val| i64::try_from((val + state.config.image_ttl).as_secs()).unwrap())
        .map_err(|_| ApiError::Internal("Welcome at the end of time!"))?;
    let id: i64 = sqlx::query!("INSERT INTO images (expires) VALUES (?) RETURNING id", time)
        .fetch_one(db)
        .await
        .map_err(|_| ApiError::Internal("DB write failed"))?
        .id;

    let dest = state.config.data_dir.join(format!("data/{id}"));

    let reader = StreamReader::new(
        ds.into_stream()
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err)),
    );
    futures::pin_mut!(reader);
    let mut file = BufWriter::new(File::create_new(dest).await.unwrap());
    tokio::io::copy(&mut reader, &mut file)
        .await
        .map(|_| (StatusCode::OK, format!("{}/img/{id}", state.config.url)))
        .map_err(|_| ApiError::Internal("failed to write file to disk"))
}
