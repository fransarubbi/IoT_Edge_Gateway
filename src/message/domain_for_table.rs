//! Modelos de dominio para el mapeo de tablas de base de datos.
//!
//! Este módulo contiene las estructuras que representan una fila exacta
//! en las tablas SQLite del sistema (`measurement`, `monitor`, `alert_*`).
//!
//! Se utiliza `sqlx::FromRow` para el mapeo automático y `serde` para la
//! serialización/deserialización necesaria en la comunicación con el Hub.


use serde::{Deserialize, Serialize};
use sqlx::{FromRow};
use crate::message::domain::{AlertAir, AlertTh, DestinationType, Measurement, Monitor};
use crate::network::domain::Hub;

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


impl MeasurementRow {
    pub fn cast_measurement(self) -> Measurement {
        let mut m = Measurement::default();
        m.metadata.sender_user_id = self.metadata.sender_user_id;
        m.metadata.destination_type = self.metadata.destination_type;
        m.metadata.destination_id = self.metadata.destination_id;
        m.metadata.timestamp = self.metadata.timestamp;
        m.wifi_ssid = self.wifi_ssid;
        m.pulse_counter = self.pulse_counter;
        m.pulse_max_duration = self.pulse_max_duration;
        m.temperature = self.temperature;
        m.humidity = self.humidity;
        m.co2_ppm = self.co2_ppm;
        m.sample = self.sample;
        m
    }
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


impl AlertAirRow {
    pub fn cast_alert_air(self) -> AlertAir {
        let mut aa = AlertAir::default();
        aa.metadata.sender_user_id = self.metadata.sender_user_id;
        aa.metadata.destination_type = self.metadata.destination_type;
        aa.metadata.destination_id = self.metadata.destination_id;
        aa.metadata.timestamp = self.metadata.timestamp;
        aa.co2_initial_ppm = self.co2_initial_ppm;
        aa.co2_actual_ppm = self.co2_actual_ppm;
        aa
    }
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


impl AlertThRow {
    pub fn cast_alert_th(self) -> AlertTh {
        let mut ath = AlertTh::default();
        ath.metadata.sender_user_id = self.metadata.sender_user_id;
        ath.metadata.destination_type = self.metadata.destination_type;
        ath.metadata.destination_id = self.metadata.destination_id;
        ath.metadata.timestamp = self.metadata.timestamp;
        ath.initial_temp = self.initial_temp;
        ath.actual_temp = self.actual_temp;
        ath
    }
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


impl MonitorRow {
    pub fn cast_monitor(self) -> Monitor {
        let mut mon = Monitor::default();
        mon.metadata.sender_user_id = self.metadata.sender_user_id;
        mon.metadata.destination_type = self.metadata.destination_type;
        mon.metadata.destination_id = self.metadata.destination_id;
        mon.metadata.timestamp = self.metadata.timestamp;
        mon.mem_free_hm = self.mem_free;
        mon.mem_free_block = self.mem_free_block;
        mon.mem_free_internal = self.mem_free_internal;
        mon.stack_free_min_coll = self.stack_free_min_coll;
        mon.stack_free_min_pub = self.stack_free_min_pub;
        mon.stack_free_min_mic = self.stack_free_min_mic;
        mon.stack_free_min_th = self.stack_free_min_th;
        mon.stack_free_min_air = self.stack_free_min_air;
        mon.stack_free_min_mon = self.stack_free_min_mon;
        mon.wifi_ssid = self.wifi_ssid;
        mon.wifi_rssi = self.wifi_rssi;
        mon.active_time = self.active_time;
        mon
    }
}


#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, FromRow)]
pub struct HubRow {
    #[sqlx(flatten)]
    pub metadata: MetadataRow,
    pub network_id: String,
    pub wifi_ssid: String,
    pub wifi_password: String,
    pub mqtt_uri: String,
    pub device_name: String,
    pub sample: u16,
    pub energy_mode: u8,
}


impl HubRow {
    pub fn cast_to_hub(self) -> Hub {
        let mut hub = Hub::default();
        hub.id = self.metadata.sender_user_id;
        hub.device_name = self.device_name;
        hub.energy_mode = self.energy_mode;
        hub
    }
}