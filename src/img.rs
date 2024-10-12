use std::{
    str::FromStr,
    sync::atomic::Ordering,
    time::{SystemTime, UNIX_EPOCH},
};

use rocket::{
    data::ToByteUnit,
    get,
    http::ContentType,
    post,
    tokio::{
        fs::{remove_file, File},
        io::AsyncWriteExt,
    },
    Data, Responder, State,
};
use rocket_db_pools::Connection;
use thiserror::Error;

use crate::{Config, Meta, TmpfileID};

#[derive(Error, Debug, Responder)]
pub enum ServError {
    #[response(status = 404)]
    #[error("Database Query did return nothing")]
    EmptyQuery(String),
    #[response(status = 500)]
    #[error("Reading file from disk failed")]
    ReadError(#[from] std::io::Error),
}

#[allow(clippy::module_name_repetitions)]
#[get("/img/<id>")]
pub async fn get_img(config: &State<Config>, id: i64) -> Result<(ContentType, File), ServError> {
    let path = config.data_dir.join(format!("data/{id}"));
    //parse mime type from first 4kib, no longer a need to store the mime-type or check it during upload
    let format = async { tree_magic_mini::from_filepath(&path) }.await;
    if let Some(format) = format {
        //this failing would mean database corruption
        File::open(path)
            .await
            .map(|file| (ContentType::from_str(format).unwrap_or_default(), file))
            .map_err(ServError::ReadError)
    } else {
        //this means the file does not exist, as every file is a octet-stream
        Err(ServError::EmptyQuery("Not found in Database".to_string()))
    }
}

#[derive(Error, Debug, Responder)]
pub enum UploadError {
    #[error("Time went backwards or you are in the year 146,138,513,298")]
    Time(String),
    #[error("Failed to write data")]
    Write(#[from] std::io::Error),
    #[error("Failed to write entry to metadata db")]
    Query(String),
    #[response(status = 422)]
    #[error("Empty file upload attempted")]
    EmptyRequest(()),
}

#[post("/upload", data = "<data_stream>")]
pub async fn upload(
    config: &State<Config>,
    tmpf_id: &State<TmpfileID>,
    mut db: Connection<Meta>,
    data_stream: Data<'_>,
) -> Result<String, UploadError> {
    //stream data to file in TEMP
    let tmp_path = config.data_dir.join(format!(
        "imgserv_{}",
        tmpf_id.0.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = data_stream
        .open(16.mebibytes())
        .into_file(&tmp_path)
        .await?;
    if !file.is_complete() {
        file.flush().await?;
    }
    //disallow empty uploads
    if file.is_empty() {
        file.shutdown().await?;
        rocket::tokio::fs::remove_file(&tmp_path).await?;
        return Err(UploadError::EmptyRequest(()));
    }
    file.shutdown().await?;

    //crash if time or id fails
    let time: i64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        //you and i should be long dead when this conversion starts failing (and sqlite supports unsigned ints then)
        .map(|val| i64::try_from(val.as_secs()).unwrap())
        .map_err(|err| UploadError::Time(err.to_string()))?;
    let id: i64 = sqlx::query!("INSERT INTO images (created) VALUES (?) RETURNING id", time)
        .fetch_one(&mut **db)
        .await
        .map_err(|err| UploadError::Query(err.to_string()))?
        .id;
    //move temp file to dest or copy is TEMP is a different mount point like tempfs
    let dest = config.data_dir.join(format!("data/{id}"));
    let res = rocket::tokio::fs::rename(&tmp_path, &dest).await;
    let res: Result<(), UploadError> = if res.is_ok() {
        Ok(())
    } else {
        let res = rocket::tokio::fs::copy(&tmp_path, dest)
            .await
            .map(|_| ())
            .map_err(UploadError::Write);
        remove_file(tmp_path).await?;
        res
    };
    //tmp file has either been moved or deleted, so we can decrement its id
    tmpf_id.0.fetch_sub(1, Ordering::Relaxed);
    res.map(|()| format!("{}/img/{id}", config.url))
}
