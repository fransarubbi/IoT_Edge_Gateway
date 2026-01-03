use sqlx::{Executor, SqlitePool};
use crate::database::repository::pop_batch_generic;
use crate::message::domain::Measurement;

pub async fn create_table_measurement(pool: &SqlitePool) -> Result<(), sqlx::Error>  {
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS measurement (
            id                   INTEGER PRIMARY KEY AUTOINCREMENT,
            sender_user_id       TEXT NOT NULL,
            destination_type     TEXT NOT NULL,
            destination_id       TEXT NOT NULL,
            timestamp            TEXT NOT NULL,
            ipv4addr             TEXT NOT NULL,
            wifi_ssid            TEXT NOT NULL,
            pulse_counter        INTEGER NOT NULL,
            pulse_max_duration   INTEGER NOT NULL,
            temperature          REAL NOT NULL,
            humidity             REAL NOT NULL,
            co2_ppm              REAL NOT NULL,
            sample               INTEGER NOT NULL
        );
        "#
    )
        .await?;

    Ok(())
}


pub async fn insert_measurement(pool: &SqlitePool,
                            data_vec: Vec<Measurement>
) -> Result<(), sqlx::Error> {
    for data in data_vec {
        sqlx::query(
            r#"
        INSERT INTO measurements (sender_user_id,
                                  destination_type,
                                  destination_id,
                                  timestamp,
                                  ipv4addr,
                                  wifi_ssid,
                                  pulse_counter,
                                  pulse_max_duration,
                                  temperature,
                                  humidity,
                                  co2_ppm,
                                  sample)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#
        )
            .bind(data.metadata.sender_user_id)
            .bind(data.metadata.destination_type)
            .bind(data.metadata.destination_id)
            .bind(data.metadata.timestamp)
            .bind(data.ipv4addr.to_string())
            .bind(data.wifi_ssid)
            .bind(data.pulse_counter)
            .bind(data.pulse_max_duration)
            .bind(data.temperature)
            .bind(data.humidity)
            .bind(data.co2_ppm)
            .bind(data.sample)
            .execute(pool)
            .await?;
    }

    Ok(())
}


pub async fn pop_batch_measurement(pool: &SqlitePool, table: &str) -> Result<Vec<Measurement>, sqlx::Error> {
    pop_batch_generic(pool, table).await
}

