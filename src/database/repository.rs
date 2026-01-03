use std::time::Duration;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{FromRow, SqlitePool};
use crate::config::sqlite::{LIMIT, WAIT_FOR};
use crate::database::domain::{Table, TableDataVector};
use crate::database::tables::alert_air::{create_table_alert_air, insert_alert_air, pop_batch_alert_air};
use crate::database::tables::alert_temp::{create_table_alert_temp, insert_alert_temp, pop_batch_alert_temp};
use crate::database::tables::measurement::{create_table_measurement, insert_measurement, pop_batch_measurement};
use crate::database::tables::monitor::{create_table_monitor, insert_monitor, pop_batch_monitor};


pub struct Repository {
    pool: SqlitePool,
}


impl Repository {
    pub async fn new(path: &str) -> Result<Self, sqlx::Error> {
        let pool = create_pool(path).await?;
        configure_db(&pool).await?;
        init_schema(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn create_repository(path: &str) -> Self {
        loop {
            match Self::new(path).await {
                Ok(repo) => return repo,
                Err(e) => {
                    log::error!("❌ Error inicializando repo: {:?}", e);
                    tokio::time::sleep(Duration::from_secs(WAIT_FOR)).await;
                }
            }
        }
    }

    pub async fn insert(&self, tdv: Vec<TableDataVector>) -> Result<(), sqlx::Error> {
        for v in tdv {
            match v {
                TableDataVector::Measurement(measurement) => {
                    insert_measurement(&self.pool, measurement).await?;
                },
                TableDataVector::Monitor(monitor) => {
                    insert_monitor(&self.pool, monitor).await?;
                },
                TableDataVector::AlertTemp(alert_temp) => {
                    insert_alert_temp(&self.pool, alert_temp).await?;
                },
                TableDataVector::AlertAir(alert_air) => {
                    insert_alert_air(&self.pool, alert_air).await?;
                },
            }
        }

        Ok(())
    }

    pub async fn pop_batch(&self, table: Table) -> Result<TableDataVector, sqlx::Error> {
        match table {
            Table::Measurement => {
                pop_batch_measurement(&self.pool, Table::Measurement.table_name())
                    .await
                    .map(TableDataVector::Measurement)
            },
            Table::Monitor => {
                pop_batch_monitor(&self.pool, Table::Monitor.table_name())
                    .await
                    .map(TableDataVector::Monitor)
            },
            Table::AlertTemp => {
                pop_batch_alert_temp(&self.pool, Table::AlertTemp.table_name())
                    .await
                    .map(TableDataVector::AlertTemp)
            },
            Table::AlertAir => {
                pop_batch_alert_air(&self.pool, Table::AlertAir.table_name())
                    .await
                    .map(TableDataVector::AlertAir)
            },
        }
    }

    pub async fn has_data(&self, table: Table) -> Result<bool, sqlx::Error> {
        match table {
            Table::Measurement => {
                table_has_data(&self.pool, table.table_name()).await
            },
            Table::Monitor => {
                table_has_data(&self.pool, table.table_name()).await
            },
            Table::AlertTemp => {
                table_has_data(&self.pool, table.table_name()).await
            },
            Table::AlertAir => {
                table_has_data(&self.pool, table.table_name()).await
            },
        }
    }
}


async fn create_pool(db_path: &str) -> Result<SqlitePool, sqlx::Error> {
    let database_url = format!("sqlite://{}", db_path);

    let pool = SqlitePoolOptions::new()
        .max_connections(3)
        .connect(&database_url)
        .await?;

    Ok(pool)
}


async fn configure_db(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query("PRAGMA journal_mode = WAL;").execute(pool).await?;
    sqlx::query("PRAGMA synchronous = NORMAL;").execute(pool).await?;
    sqlx::query("PRAGMA busy_timeout = 5000;").execute(pool).await?;
    Ok(())
}


async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    create_table_measurement(pool).await?;
    create_table_monitor(pool).await?;
    create_table_alert_temp(pool).await?;
    create_table_alert_air(pool).await?;
    Ok(())
}


pub async fn pop_batch_generic<T>(pool: &SqlitePool,
                              table: &str,
                              ) -> Result<Vec<T>, sqlx::Error>
where
    T: for<'r> FromRow<'r, sqlx::sqlite::SqliteRow>
    + Send
    + Unpin
    + 'static,
{
    let sql = format!(
        r#"
        DELETE FROM {table}
        WHERE id IN (
            SELECT id FROM {table}
            ORDER BY id ASC
            LIMIT ?
        )
        RETURNING *
        "#
    );

    let result = sqlx::query_as::<_, T>(&sql)
        .bind(LIMIT)
        .fetch_all(pool)
        .await?;

    Ok(result)
}


async fn table_has_data(pool: &SqlitePool, table: &str) -> Result<bool, sqlx::Error> {
    let sql = format!("SELECT 1 FROM {table} LIMIT 1");
    let row = sqlx::query(&sql).fetch_optional(pool).await?;
    Ok(row.is_some())
}


