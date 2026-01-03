use sqlx::{Executor, SqlitePool};
use crate::database::repository::pop_batch_generic;
use crate::message::domain::Monitor;

pub async fn create_table_monitor(pool: &SqlitePool) -> Result<(), sqlx::Error>  {
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS monitor (
            id                   INTEGER PRIMARY KEY AUTOINCREMENT,
            sender_user_id       TEXT NOT NULL,
            destination_type     TEXT NOT NULL,
            destination_id       TEXT NOT NULL,
            timestamp            TEXT NOT NULL,
            mem_free             INTEGER NOT NULL,
            mem_free_hm          INTEGER NOT NULL,
            mem_free_block       INTEGER NOT NULL,
            mem_free_internal    INTEGER NOT NULL,
            stack_free_min_coll  INTEGER NOT NULL,
            stack_free_min_pub   INTEGER NOT NULL,
            stack_free_min_mic   INTEGER NOT NULL,
            stack_free_min_th    INTEGER NOT NULL,
            stack_free_min_air   INTEGER NOT NULL,
            stack_free_min_mon   INTEGER NOT NULL,
            wifi_ssid            TEXT NOT NULL,
            wifi_rssi            INTEGER NOT NULL,
            active_time TEXT NOT NULL
        );
        "#
    )
        .await?;

    Ok(())
}


pub async fn insert_monitor(pool: &SqlitePool,
                        data_vec: Vec<Monitor>
) -> Result<(), sqlx::Error> {
    
    for data in data_vec {
        sqlx::query(
            r#"
        INSERT INTO monitor (sender_user_id,
                             destination_type,
                             destination_id,
                             timestamp,
                             mem_free,
                             mem_free_hm,
                             mem_free_block,
                             mem_free_internal,
                             stack_free_min_coll,
                             stack_free_min_pub,
                             stack_free_min_mic,
                             stack_free_min_th,
                             stack_free_min_air,
                             stack_free_min_mon,
                             wifi_ssid,
                             wifi_rssi,
                             active_time)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#
        )
            .bind(data.metadata.sender_user_id)
            .bind(data.metadata.destination_type)
            .bind(data.metadata.destination_id)
            .bind(data.metadata.timestamp)
            .bind(data.mem_free)
            .bind(data.mem_free_hm)
            .bind(data.mem_free_block)
            .bind(data.mem_free_internal)
            .bind(data.stack_free_min_coll)
            .bind(data.stack_free_min_pub)
            .bind(data.stack_free_min_mic)
            .bind(data.stack_free_min_th)
            .bind(data.stack_free_min_air)
            .bind(data.stack_free_min_mon)
            .bind(data.wifi_ssid)
            .bind(data.wifi_rssi)
            .bind(data.active_time)
            .execute(pool)
            .await?;
    }
    
    Ok(())
}


pub async fn pop_batch_monitor(pool: &SqlitePool, table: &str) -> Result<Vec<Monitor>, sqlx::Error> {
    pop_batch_generic(pool, table).await
}

