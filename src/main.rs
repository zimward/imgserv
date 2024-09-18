use std::{
    env,
    fs::{create_dir_all, read_to_string},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rocket::{
    fairing::{self, AdHoc},
    fs::{NamedFile, TempFile},
    get,
    http::ContentType,
    post, routes,
    tokio::fs::remove_file,
    Build, Orbit, Responder, Rocket, State,
};
use rocket_db_pools::{Connection, Database};
use serde::Deserialize;
use sqlx::{Pool, Sqlite};
use thiserror::Error;

fn _default_path() -> PathBuf {
    PathBuf::from("/var/lib/imgserv")
}

#[derive(Deserialize, Clone)]
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
) -> Result<(ContentType, NamedFile), ServError> {
    let type_id = sqlx::query!("SELECT type from images where id == ?", id)
        .fetch_one(&mut **db)
        .await;
    if let Ok(type_id) = type_id.map(|t| t.r#type) {
        //this failing would mean database corruption
        let format = Formats::try_from(type_id.unwrap()).unwrap();
        NamedFile::open(config.data_dir.join("data/").join(id.to_string()))
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
}

async fn upload_any(
    config: &State<Config>,
    mut db: Connection<Meta>,
    mut file: TempFile<'_>,
    format: Formats,
) -> Result<String, UploadError> {
    let format = i64::from(format);
    //crash if time or id fails
    let time: i64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        //you and i should be long dead when this conversion starts failing (and sqlite supports unsigned ints then)
        .map(|val| i64::try_from(val.as_secs()).unwrap())
        .map_err(|err| UploadError::Time(err.to_string()))?;
    let id: i64 = sqlx::query!(
        "INSERT INTO images (created,type) VALUES (?,?) RETURNING id",
        time,
        format
    )
    .fetch_one(&mut **db)
    .await
    .map_err(|err| UploadError::Query(err.to_string()))?
    .id;
    file.persist_to(config.data_dir.join("data/").join(id.to_string()))
        .await
        .map(|()| format!("{}/img/{id}", config.url))
        .map_err(UploadError::Write)
}

#[post("/upload", format = "image/png", data = "<file>")]
async fn upload_png(
    config: &State<Config>,
    db: Connection<Meta>,
    file: TempFile<'_>,
) -> Result<String, UploadError> {
    upload_any(config, db, file, Formats::Png).await
}
#[post("/upload", format = "image/jpeg", data = "<file>")]
async fn upload_jpeg(
    config: &State<Config>,
    db: Connection<Meta>,
    file: TempFile<'_>,
) -> Result<String, UploadError> {
    upload_any(config, db, file, Formats::Jpeg).await
}
#[post("/upload", format = "image/jxl", data = "<file>")]
async fn upload_jxl(
    config: &State<Config>,
    db: Connection<Meta>,
    file: TempFile<'_>,
) -> Result<String, UploadError> {
    upload_any(config, db, file, Formats::JpegXl).await
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
            .mount("/", routes![get_img, upload_png, upload_jpeg, upload_jxl])
            .launch()
            .await?;
    } else {
        eprintln!("No configuration could be found!");
    }
    Ok(())
}
