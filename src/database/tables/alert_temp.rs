use sqlx::{Executor, SqlitePool};
use crate::database::repository::pop_batch_generic;
use crate::message::msg_type::AlertTh;

pub async fn create_table_alert_temp(pool: &SqlitePool) -> Result<(), sqlx::Error>  {
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS alert_temp (
            id                   INTEGER PRIMARY KEY AUTOINCREMENT,
            sender_user_id       TEXT NOT NULL,
            destination_type     TEXT NOT NULL,
            destination_id       TEXT NOT NULL,
            timestamp            TEXT NOT NULL,
            initial_temp         REAL NOT NULL,
            actual_temp          REAL NOT NULL
        );
        "#
    )
        .await?;
    
    Ok(())
}


pub async fn insert_alert_temp(pool: &SqlitePool,
                           data: AlertTh
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO alert_temp (sender_user_id,
                                  destination_type,
                                  destination_id,
                                  timestamp,
                                  initial_temp,
                                  actual_temp)
        VALUES (?, ?, ?, ?, ?, ?)
        "#
    )
        .bind(data.metadata.sender_user_id)
        .bind(data.metadata.destination_type)
        .bind(data.metadata.destination_id)
        .bind(data.metadata.timestamp)
        .bind(data.initial_temp)
        .bind(data.actual_temp)
        .execute(pool)
        .await?;

    Ok(())
}


pub async fn pop_batch_alert_temp(pool: &SqlitePool, table: &str) -> Result<Vec<AlertTh>, sqlx::Error> {
    pop_batch_generic(pool, table).await
}

