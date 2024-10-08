use std::{
    env,
    fs::{create_dir_all, read_to_string},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU16, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rocket::serde::Deserialize;
use rocket::{
    data::ToByteUnit,
    fairing::{self, AdHoc},
    get,
    http::ContentType,
    post, routes,
    tokio::{
        fs::{remove_file, File},
        io::AsyncWriteExt,
    },
    Build, Data, Orbit, Responder, Rocket, State,
};
use rocket_db_pools::{Connection, Database};
use sqlx::{Pool, Sqlite};
use thiserror::Error;

fn _default_path() -> PathBuf {
    PathBuf::from("/var/lib/imgserv")
}

#[derive(Deserialize, Clone)]
#[serde(crate = "rocket::serde")]
struct Config {
    url: String,
    #[serde(default = "_default_path")]
    data_dir: PathBuf,
    image_ttl: u64,
    cleanup_interval: u64,
}

enum Formats {
    Png = 0,
    Jpeg,
    JpegXl,
}

impl From<Formats> for i64 {
    fn from(val: Formats) -> Self {
        val as Self
    }
}

impl TryFrom<i64> for Formats {
    type Error = ();

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Png),
            1 => Ok(Self::Jpeg),
            2 => Ok(Self::JpegXl),
            _ => Err(()),
        }
    }
}

impl Formats {
    fn get_mime(&self) -> ContentType {
        match self {
            Self::Png => ContentType::new("image", "png"),
            Self::Jpeg => ContentType::new("image", "jpeg"),
            Self::JpegXl => ContentType::new("image", "jxl"),
        }
    }
}

impl TryFrom<&ContentType> for Formats {
    type Error = UploadError;

    fn try_from(value: &ContentType) -> Result<Self, Self::Error> {
        match value.sub().as_str() {
            "png" => Ok(Self::Png),
            "jpeg" => Ok(Self::Jpeg),
            "jxl" => Ok(Self::JpegXl),
            _ => Err(Self::Error::Unsupported(())),
        }
    }
}

#[derive(Error, Debug, Responder)]
enum ServError {
    #[response(status = 404)]
    #[error("Database Query did return nothing")]
    EmptyQuery(String),
    #[response(status = 500)]
    #[error("Reading file from disk failed")]
    ReadError(#[from] std::io::Error),
}

#[derive(Database)]
#[database("db")]
struct Meta(sqlx::SqlitePool);

#[get("/img/<id>")]
async fn get_img(
    mut db: Connection<Meta>,
    config: &State<Config>,
    id: i64,
) -> Result<(ContentType, File), ServError> {
    let type_id = sqlx::query!("SELECT type from images where id == ?", id)
        .fetch_one(&mut **db)
        .await;
    if let Ok(type_id) = type_id.map(|t| t.r#type) {
        //this failing would mean database corruption
        let format = Formats::try_from(type_id.unwrap()).unwrap();
        File::open(config.data_dir.join("data/").join(id.to_string()))
            .await
            .map(|file| (format.get_mime(), file))
            .map_err(ServError::ReadError)
    } else {
        Err(ServError::EmptyQuery("Not found in Database".to_string()))
    }
}

#[derive(Error, Debug, Responder)]
enum UploadError {
    #[error("Time went backwards or you are in the year 146,138,513,298")]
    Time(String),
    #[error("Failed to write data")]
    Write(#[from] std::io::Error),
    #[error("Failed to write entry to metadata db")]
    Query(String),
    #[response(status = 422)]
    #[error("Empty file upload attempted")]
    EmptyRequest(()),
    #[response(status = 415)]
    #[error("Unsupported MIME-type")]
    Unsupported(()),
}

//holds count of tmp files
struct TmpfileID(AtomicU16);

#[post("/upload", data = "<data_stream>")]
async fn upload(
    config: &State<Config>,
    tmpf_id: &State<TmpfileID>,
    mut db: Connection<Meta>,
    data_stream: Data<'_>,
    ct: &ContentType,
) -> Result<String, UploadError> {
    //stream data to file in TEMP
    let tmp_path = config.data_dir.join(format!(
        "imgserv_{}",
        tmpf_id.0.fetch_add(1, Ordering::Relaxed)
    ));
    let mime_type = Formats::try_from(ct)?;
    let mime_id = i64::from(mime_type);
    let mut file = data_stream
        .open(16.mebibytes())
        .into_file(&tmp_path)
        .await?;
    if !file.is_complete() {
        file.flush().await?;
    }
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
    let id: i64 = sqlx::query!(
        "INSERT INTO images (created,type) VALUES (?,?) RETURNING id",
        time,
        mime_id
    )
    .fetch_one(&mut **db)
    .await
    .map_err(|err| UploadError::Query(err.to_string()))?
    .id;
    //move temp file to dest or copy is TEMP is a different mount point like tempfs
    let dest = config.data_dir.join("data/").join(id.to_string());
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

async fn cleanup(db: &Pool<Sqlite>, data_dir: &Path, ttl: Duration) {
    //time 14 days ago
    let expiry_date: i64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .saturating_sub(ttl)
        .as_secs()
        .try_into()
        .unwrap();
    //has the file been created before that
    let expired: Vec<i64> = sqlx::query!("SELECT id FROM images WHERE created <= ?", expiry_date)
        .fetch_all(db)
        .await
        .unwrap()
        .iter()
        .map(|r| r.id)
        .collect();
    //delete all expired files
    for e in &expired {
        remove_file(format!("{}/data/{e}", data_dir.to_str().unwrap()))
            .await
            .unwrap();
    }
    let res = sqlx::query!("DELETE FROM images WHERE created <= ?", expiry_date)
        .execute(db)
        .await
        .unwrap()
        .rows_affected();
    if expired.len() as u64 != res {
        eprintln!(
            "Error: query deleted {} elements while {} where expected",
            res,
            expired.len()
        );
    }
}

fn cleanup_fairing(rocket: &Rocket<Orbit>, config: &Config) {
    let db = Meta::fetch(rocket)
        .map_or_else(|| Err(rocket), |db| Ok(db.0.clone()))
        .unwrap();
    let mut interval = rocket::tokio::time::interval(Duration::from_secs(config.cleanup_interval));
    let ttl = Duration::from_secs(config.image_ttl);
    let data_dir = config.data_dir.clone();
    rocket::tokio::spawn(async move {
        loop {
            interval.tick().await;
            cleanup(&db, &data_dir, ttl).await;
        }
    });
}

async fn run_migrations(rocket: Rocket<Build>) -> fairing::Result {
    match Meta::fetch(&rocket) {
        Some(db) => match sqlx::migrate!("./migrations/").run(&**db).await {
            Ok(()) => Ok(rocket),
            Err(e) => {
                eprintln!("Error: faild to run migrations:{e}");
                Err(rocket)
            }
        },
        None => Err(rocket),
    }
}

#[rocket::main]
async fn main() -> Result<(), rocket::Error> {
    let config_src = env::var("CONFIG_FILE").unwrap_or_else(|_| "/etc/imgserv.toml".to_string());
    let config_src = env::var("CONFIG")
        .or_else(|_| read_to_string(&config_src))
        .unwrap_or_default();

    let config: Result<Config, toml::de::Error> = toml::from_str(config_src.as_str());
    if let Ok(config) = config {
        create_dir_all(config.data_dir.join("data/")).unwrap();
        let figment = rocket::Config::figment().merge((
            "databases.db.url",
            config.data_dir.join("db.sqlite").to_str(),
        ));
        let config_clone = config.clone();
        #[allow(clippy::no_effect_underscore_binding)]
        let _rocket = rocket::custom(figment)
            .attach(Meta::init())
            .attach(AdHoc::on_liftoff("cleanup", move |rocket| {
                Box::pin(async move {
                    cleanup_fairing(rocket, &config_clone);
                })
            }))
            .attach(AdHoc::try_on_ignite("SQLx Migrations", run_migrations))
            .manage(config)
            .manage(TmpfileID(AtomicU16::new(0)))
            .mount("/", routes![get_img, upload])
            .launch()
            .await?;
    } else {
        eprintln!("No configuration could be found!");
    }
    Ok(())
}
