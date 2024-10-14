use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sqlx::{Pool, Sqlite};
use tokio::fs::remove_file;

use crate::Config;

pub async fn cleanup(db: Pool<Sqlite>, config: &Config) {
    let mut interval = tokio::time::interval(Duration::from_secs(config.cleanup_interval));
    let ttl = Duration::from_secs(config.image_ttl);
    let data_dir = config.data_dir.clone();
    tokio::spawn(async move {
        loop {
            interval.tick().await;
            //time 14 days ago
            let expiry_date: i64 = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .saturating_sub(ttl)
                .as_secs()
                .try_into()
                .unwrap();
            //has the file been created before that
            let expired: Vec<i64> =
                sqlx::query!("SELECT id FROM images WHERE created <= ?", expiry_date)
                    .fetch_all(&db)
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
            let res = sqlx::query!("DELETE FROM images WHERE created <= ?", expiry_date)
                .execute(&db)
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
    });
}
