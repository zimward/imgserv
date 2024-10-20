use std::{env, fs::read_to_string, path::PathBuf, time::Duration};

use cleanup::cleanup;
use serde::Deserialize;
use sqlx::sqlite::SqlitePoolOptions;

use tokio::fs::create_dir_all;

use duration_str::deserialize_duration;

mod cleanup;
mod http;

fn _default_path() -> PathBuf {
    PathBuf::from("/var/lib/imgserv")
}

#[derive(Deserialize, Clone)]
pub struct Config {
    url: String,
    #[serde(default = "_default_path")]
    data_dir: PathBuf,
    #[serde(deserialize_with = "deserialize_duration")]
    image_ttl: Duration,
    #[serde(deserialize_with = "deserialize_duration")]
    paste_ttl: Duration,
    #[serde(deserialize_with = "deserialize_duration")]
    cleanup_interval: Duration,
}

#[tokio::main]
async fn main() {
    let config_src = env::var("CONFIG_FILE").unwrap_or_else(|_| "/etc/imgserv.toml".to_string());
    let config_src = env::var("CONFIG")
        .or_else(|_| read_to_string(&config_src))
        .unwrap_or_default();

    let config: Config = toml::from_str(config_src.as_str()).expect("Failed to parse config");
    create_dir_all(config.data_dir.join("data/"))
        .await
        .expect("Failed to create data dir");
    let db = SqlitePoolOptions::new()
        .connect(
            config
                .data_dir
                .join("db.sqlite")
                .to_str()
                .unwrap_or_default(),
        )
        .await
        .expect("failed to connect to DB");

    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .expect("SQL migrations failed");

    cleanup(db.clone(), &config).await;
    http::serve(db, config).await;
}
