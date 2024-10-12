use std::{
    env,
    fs::{create_dir_all, read_to_string},
    path::{Path, PathBuf},
    sync::atomic::AtomicU16,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rocket::serde::Deserialize;
use rocket::{
    fairing::{self, AdHoc},
    routes,
    tokio::fs::remove_file,
    Build, Orbit, Rocket,
};
use rocket_db_pools::Database;
use sqlx::{Pool, Sqlite};

use img::{get_img, upload};

mod img;

fn _default_path() -> PathBuf {
    PathBuf::from("/var/lib/imgserv")
}

#[derive(Deserialize, Clone)]
#[serde(crate = "rocket::serde")]
pub struct Config {
    url: String,
    #[serde(default = "_default_path")]
    data_dir: PathBuf,
    image_ttl: u64,
    cleanup_interval: u64,
}

#[derive(Database)]
#[database("db")]
struct Meta(sqlx::SqlitePool);

//holds count of tmp files
struct TmpfileID(AtomicU16);

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
        if remove_file(format!("{}/data/{e}", data_dir.to_str().unwrap()))
            .await
            .is_err()
        {
            eprintln!("Failed to find data/{}", e);
        }
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
