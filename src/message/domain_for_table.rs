//! Modelos de dominio para el mapeo de tablas de base de datos.
//!
//! Este módulo contiene las estructuras que representan una fila exacta
//! en las tablas SQLite del sistema (`measurement`, `monitor`, `alert_*`).
//!
//! Se utiliza `sqlx::FromRow` para el mapeo automático y `serde` para la
//! serialización/deserialización necesaria en la comunicación con el Hub.


use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use crate::message::domain::{DestinationType};


/// Metadatos comunes a todos los eventos y mediciones del sistema.
///
/// Esta estructura no suele tener una tabla propia, sino que se "aplana"
/// inside de otras tablas usando `#[sqlx(flatten)]`. Contiene la información
/// de enrutamiento y trazabilidad del mensaje.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, FromRow)]
pub struct MetadataRow {
    pub sender_user_id: String,
    pub destination_type: DestinationType,
    pub destination_id: String,
    pub timestamp: String,
    pub topic_where_arrive: String,
}


/// Representación de una fila de la tabla de mediciones (`measurement`).
///
/// Almacena los datos de sensores físicos reportados por los nodos.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, FromRow)]
pub struct MeasurementRow {
    #[sqlx(flatten)]
    pub metadata: MetadataRow,
    pub ipv4addr: String,
    pub wifi_ssid: String,
    pub pulse_counter: i64,
    pub pulse_max_duration: i64,
    pub temperature: f32,
    pub humidity: f32,
    pub co2_ppm: f32,
    pub sample: u16,
}


/// Representación de una fila de la tabla de alertas de aire (`alert_air`).
///
/// Se utiliza cuando los niveles de CO2 superan los umbrales configurados.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, FromRow)]
pub struct AlertAirRow {
    #[sqlx(flatten)]
    pub metadata: MetadataRow,
    pub co2_initial_ppm: f32,
    pub co2_actual_ppm: f32,
}


/// Representación de una fila de la tabla de alertas térmicas (`alert_temp`).
///
/// Se utiliza para eventos de temperatura fuera de rango (congelación/sobrecalentamiento).
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, FromRow)]
pub struct AlertThRow {
    #[sqlx(flatten)]
    pub metadata: MetadataRow,
    pub initial_temp: f32,
    pub actual_temp: f32,
}


/// Representación de una fila de la tabla de monitoreo (`monitor`).
///
/// Contiene telemetría interna sobre la salud del hardware y el firmware
/// de los nodos (memoria, stack, conectividad).
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, FromRow)]
pub struct MonitorRow {
    #[sqlx(flatten)]
    pub metadata: MetadataRow,
    pub mem_free: i64,
    pub mem_free_hm: i64,
    pub mem_free_block: i64,
    pub mem_free_internal: i64,
    pub stack_free_min_coll: i64,
    pub stack_free_min_pub: i64,
    pub stack_free_min_mic: i64,
    pub stack_free_min_th: i64,
    pub stack_free_min_air: i64,
    pub stack_free_min_mon: i64,
    pub wifi_ssid: String,
    pub wifi_rssi: i8,
    pub active_time: String,
}