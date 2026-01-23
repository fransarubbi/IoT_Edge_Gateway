//! Módulo de persistencia para la configuración de Hubs.
//!
//! Este módulo gestiona el ciclo de vida de los datos de los dispositivos "Hub" en la base de datos SQLite.
//! Almacena información crítica de conectividad (WiFi, MQTT), identidad (IDs) y configuración operativa
//! (modos de energía, sample rate).
//!
//! # Tabla `hub`
//! Es una tabla plana que almacena tanto los metadatos aplanados (`MetadataRow`) como
//! los campos específicos de configuración del Hub (`HubRow`).


use sqlx::{Executor, SqlitePool};
use crate::network::domain::HubRow;

/// Inicializa la tabla `hub` en la base de datos.
///
/// Crea el esquema necesario para almacenar la configuración de los Hubs si no existe.
///
/// # Esquema
/// - `id`: Identificador interno (Primary Key).
/// - Campos de Metadatos: `sender_user_id`, `destination_type`, `destination_id`, `timestamp`, `topic_where_arrive`.
/// - Campos de Configuración: `network_id`, `wifi_ssid`, `wifi_password`, `mqtt_uri`, `device_name`, `sample`, `energy_mode`.
pub async fn create_table_hub(pool: &SqlitePool) -> Result<(), sqlx::Error>  {
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS hub (
            id                   INTEGER PRIMARY KEY AUTOINCREMENT,
            sender_user_id       TEXT NOT NULL UNIQUE,
            destination_id       TEXT NOT NULL,
            timestamp            INTEGER NOT NULL,
            network_id           TEXT NOT NULL,
            wifi_ssid            TEXT NOT NULL,
            wifi_password        TEXT NOT NULL,
            mqtt_uri             TEXT NOT NULL,
            device_name          TEXT NOT NULL,
            sample               INTEGER NOT NULL,
            energy_mode          INTEGER NOT NULL
        );
        "#
    )
        .await?;

    Ok(())
}


/// Inserta un nuevo registro de Hub en la base de datos.
///
/// Desglosa la estructura jerárquica `HubRow` (que contiene `MetadataRow`)
/// en una inserción plana SQL.
///
/// # Errores
/// Retorna `sqlx::Error` si falla la conexión o la restricción de datos.
pub async fn insert_hub_table(pool: &SqlitePool,
                              data: HubRow
) -> Result<(), sqlx::Error> {

    sqlx::query(
        r#"
            INSERT INTO hub (
                sender_user_id,
                destination_id,
                timestamp,
                network_id,
                wifi_ssid,
                wifi_password,
                mqtt_uri,
                device_name,
                sample,
                energy_mode
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#
    )
        .bind(data.metadata.sender_user_id)
        .bind(data.metadata.destination_id)
        .bind(data.metadata.timestamp)
        .bind(data.network_id)
        .bind(data.wifi_ssid)
        .bind(data.wifi_password)
        .bind(data.mqtt_uri)
        .bind(data.device_name)
        .bind(data.sample)
        .bind(data.energy_mode)
        .execute(pool)
        .await?;

    Ok(())
}


/// Elimina todos los Hubs asociados a una red específica.
///
/// Útil cuando se elimina una configuración de red completa y se deben limpiar
/// los dispositivos asociados a ella.
///
/// # Parámetros
/// - `id`: El `network_id` a buscar y eliminar.
pub async fn delete_hub_according_to_network(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        DELETE FROM hub
        WHERE network_id = ?
        "#
    )
        .bind(id)
        .execute(pool)
        .await?;

    Ok(())
}


/// Elimina un Hub específico basado en su identificador de usuario/dispositivo.
///
/// # Parámetros
/// - `id`: El `sender_user_id` único del dispositivo.
pub async fn delete_hub_according_to_id(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        DELETE FROM hub
        WHERE sender_user_id = ?
        "#
    )
        .bind(id)
        .execute(pool)
        .await?;

    Ok(())
}


/// Función que inserta o actualiza un Hub si ya existe.
pub async fn upsert_hub(pool: &SqlitePool, data: HubRow) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO hub (
            sender_user_id,
            destination_id,
            timestamp,
            network_id,
            wifi_ssid,
            wifi_password,
            mqtt_uri,
            device_name,
            sample,
            energy_mode
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(sender_user_id) DO UPDATE SET
            destination_id     = excluded.destination_id,
            timestamp          = excluded.timestamp,
            network_id         = excluded.network_id,
            wifi_ssid          = excluded.wifi_ssid,
            wifi_password      = excluded.wifi_password,
            mqtt_uri           = excluded.mqtt_uri,
            device_name        = excluded.device_name,
            sample             = excluded.sample,
            energy_mode        = excluded.energy_mode
        "#
    )
        .bind(&data.metadata.sender_user_id)
        .bind(&data.metadata.destination_id)
        .bind(&data.metadata.timestamp)
        .bind(&data.network_id)
        .bind(&data.wifi_ssid)
        .bind(&data.wifi_password)
        .bind(&data.mqtt_uri)
        .bind(&data.device_name)
        .bind(data.sample)
        .bind(data.energy_mode)
        .execute(pool)
        .await?;

    Ok(())
}


/// Recupera todos los Hubs registrados en el sistema.
///
/// Mapea automáticamente las filas SQL a la estructura `HubRow`.
pub async fn get_all_hubs(pool: &SqlitePool) -> Result<Vec<HubRow>, sqlx::Error> {
    let result = sqlx::query_as::<_, HubRow>(
        r#"
        SELECT * FROM hub
        "#
    )
        .fetch_all(pool)
        .await?;

    Ok(result)
}