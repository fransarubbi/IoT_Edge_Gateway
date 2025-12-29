use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use sqlx::Executor;


pub async fn create_pool(db_path: &str) -> Result<SqlitePool, sqlx::Error> {
    let database_url = format!("sqlite://{}", db_path);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;

    Ok(pool)
}


pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS measurements (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            node_id     TEXT NOT NULL,
            value       REAL NOT NULL,
            timestamp   INTEGER NOT NULL
        );
        "#
    )
        .await?;

    Ok(())
}


pub async fn insert_measurement(
    pool: &SqlitePool,
    node_id: &str,
    value: f64,
    timestamp: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO measurements (node_id, value, timestamp)
        VALUES (?, ?, ?)
        "#
    )
        .bind(node_id)
        .bind(value)
        .bind(timestamp)
        .execute(pool)
        .await?;

    Ok(())
}
