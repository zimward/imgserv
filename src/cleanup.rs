use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sqlx::{Pool, Sqlite};
use tokio::fs::remove_file;

use crate::Config;

async fn cleanup_img(ttl: Duration, data_dir: &Path, unix_time: i64, db: &Pool<Sqlite>) {
    //time 14 days ago
    #[allow(clippy::cast_possible_wrap)]
    let expiry_date: i64 = unix_time.saturating_sub(ttl.as_secs() as i64);
    //delete all expired images,returning the file id
    let expired: Vec<i64> = sqlx::query!(
        "DELETE FROM images WHERE expires <= ? RETURNING id",
        expiry_date
    )
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
            eprintln!("Failed to find data/{e}");
        }
    }
}

async fn cleanup_pastes(unix_time: i64, db: &Pool<Sqlite>) {
    sqlx::query!("DELETE FROM pastes WHERE expires <= ?", unix_time)
        .execute(db)
        .await
        .expect("cleanup of pastes failed");
}

pub async fn cleanup(db: Pool<Sqlite>, config: &Config) {
    let mut interval = tokio::time::interval(config.cleanup_interval);
    let ttl = config.image_ttl;
    let data_dir = config.data_dir.clone();
    tokio::spawn(async move {
        loop {
            interval.tick().await;

            //once this starts wrapping it is no longer your or my problem
            #[allow(clippy::cast_possible_wrap)]
            let unix_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            cleanup_img(ttl, &data_dir, unix_time, &db).await;
            cleanup_pastes(unix_time, &db).await;
        }
    });
}
