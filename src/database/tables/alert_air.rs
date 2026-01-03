use sqlx::{Executor, SqlitePool};
use crate::database::repository::pop_batch_generic;
use crate::message::domain::AlertAir;

pub async fn create_table_alert_air(pool: &SqlitePool) -> Result<(), sqlx::Error>  {
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS alert_air (
            id                   INTEGER PRIMARY KEY AUTOINCREMENT,
            sender_user_id       TEXT NOT NULL,
            destination_type     TEXT NOT NULL,
            destination_id       TEXT NOT NULL,
            timestamp            TEXT NOT NULL,
            co2_initial_ppm      REAL NOT NULL,
            co2_actual_ppm       REAL NOT NULL
        );
        "#
    )
        .await?;
    Ok(())
}


pub async fn insert_alert_air(pool: &SqlitePool,
                          data_vec: Vec<AlertAir>
) -> Result<(), sqlx::Error> {
    
    for data in data_vec {
        sqlx::query(
            r#"
        INSERT INTO alert_air (sender_user_id,
                                  destination_type,
                                  destination_id,
                                  timestamp,
                                  co2_initial_ppm,
                                  co2_actual_ppm)
        VALUES (?, ?, ?, ?, ?, ?)
        "#
        )
            .bind(data.metadata.sender_user_id)
            .bind(data.metadata.destination_type)
            .bind(data.metadata.destination_id)
            .bind(data.metadata.timestamp)
            .bind(data.co2_initial_ppm)
            .bind(data.co2_actual_ppm)
            .execute(pool)
            .await?;
    }
    
    Ok(())
}


pub async fn pop_batch_alert_air(pool: &SqlitePool, table: &str) -> Result<Vec<AlertAir>, sqlx::Error> {
    pop_batch_generic(pool, table).await
}

