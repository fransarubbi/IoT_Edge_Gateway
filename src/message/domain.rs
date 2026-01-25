//! Dominio de Mensajería y Modelos de Datos.
//!
//! Este módulo define las estructuras de datos fundamentales que se intercambian
//! entre los distintos componentes del sistema (Hub, Edge, Servidor).
//! Actúa como el lenguaje común para la serialización (MessagePack) y
//! la persistencia en base de datos.
//!
//! # Organización
//!
//! - **Modelos Base:** Estructuras atómicas como `Metadata`, `DestinationType`.
//! - **Payloads de Negocio:** Estructuras como `Measurement`, `Monitor`, `Alert`.
//! - **Wrappers de Transporte:** Enums y Structs contenedores (`MessageFromHub`, `MessageToHub`)
//!   que agrupan los payloads para su enrutamiento.
//! - **Utilidades:** Funciones de casting para transformar modelos de memoria en filas de base de datos (`..._row`).


use serde::{Serialize, Deserialize};
use sqlx::FromRow;
use crate::network::domain::HubRow;

/// Metadatos estándar para todos los mensajes del sistema.
///
/// Proporciona contexto de trazabilidad, origen y destino para cada paquete de datos.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, FromRow, Hash)]
pub struct Metadata {
    pub sender_user_id: String,
    pub destination_id: String,
    pub timestamp: i64,
}


/// Mediciones de sensores ambientales y operativos.
///
/// Representa el paquete de datos principal generado por los nodos.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, FromRow)]
pub struct Measurement {
    #[sqlx(flatten)]
    pub metadata: Metadata,
    pub network: String,
    pub pulse_counter: i64,
    pub pulse_max_duration: i64,
    pub temperature: f32,
    pub humidity: f32,
    pub co2_ppm: f32,
    pub sample: u16,
}


/// Alerta de calidad de aire.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, FromRow)]
pub struct AlertAir {
    #[sqlx(flatten)]
    pub metadata: Metadata,
    pub network: String,
    pub co2_initial_ppm: f32,
    pub co2_actual_ppm: f32,
}


/// Alerta de Temperatura y Humedad.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, FromRow)]
pub struct AlertTh {
    #[sqlx(flatten)]
    pub metadata: Metadata,
    pub network: String,
    pub initial_temp: f32,
    pub actual_temp: f32,
}


/// Datos de telemetría y salud del Hub.
///
/// Incluye información sobre memoria, stack y conectividad para diagnóstico.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, FromRow)]
pub struct Monitor {
    #[sqlx(flatten)]
    pub metadata: Metadata,
    pub network: String,
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
    pub active_time: i64,
}


/// Definición de una Red lógica.
///
/// Utilizada para agrupar dispositivos bajo un mismo identificador de red.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Network {
    pub metadata: Metadata,
    pub id_network: String,
    pub name_network: String,
    pub active: bool,
    pub delete_network: bool,
}


/// Configuración remota para un dispositivo (Hub/Nodo).
///
/// Contiene credenciales WiFi/MQTT y parámetros operativos.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Settings {
    pub metadata: Metadata,
    pub network: String,
    pub wifi_ssid: String,
    pub wifi_password: String,
    pub mqtt_uri: String,
    pub device_name: String,
    pub sample: u32,
    pub energy_mode: u32,
}


impl Settings {

    /// Convierte la configuración recibida en un registro de Hub (`HubRow`) para persistencia.
    ///
    /// # Parámetros
    /// - `network`: ID de la red a la que se asocia este dispositivo.
    pub fn cast_settings_to_hub_row(self, network: String) -> HubRow {
        let mut hr = HubRow::default();
        hr.metadata.sender_user_id = self.metadata.sender_user_id;
        hr.metadata.destination_id = self.metadata.destination_id;
        hr.metadata.timestamp = self.metadata.timestamp;
        hr.network_id = network;
        hr.wifi_ssid = self.wifi_ssid;
        hr.wifi_password = self.wifi_password;
        hr.mqtt_uri = self.mqtt_uri;
        hr.device_name = self.device_name;
        hr.sample = self.sample;
        hr.energy_mode = self.energy_mode;
        hr
    }
}


/// Mensaje de Handshake enviado HACIA el Hub (Downlink).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandshakeToHub {
    pub metadata: Metadata,
    pub balance_epoch: u32,
    pub duration: u32,
}


/// Mensaje de Handshake proveniente DEL Hub (Uplink).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandshakeFromHub {
    pub metadata: Metadata,
    pub state: String,
    pub balance_epoch: u32,
}


/// Notificación de cambio a Modo Balance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageStateBalanceMode {
    pub metadata: Metadata,
    pub state: String,
    pub balance_epoch: u32,
    pub sub_state: String,
    pub duration: u32,
}


/// Notificación de cambio a Modo Normal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageStateNormal {
    pub metadata: Metadata,
    pub state: String,
}


/// Notificación de cambio a Modo Seguro (Safe Mode).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageStateSafeMode {
    pub metadata: Metadata,
    pub state: String,
    pub duration: u32,
    pub frequency: u32,
    pub jitter: u32,
}


/// Notificación de cambio de Fase dentro del modo Balance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhaseNotification {
    pub metadata: Metadata,
    pub state: String,
    pub epoch: u32,
    pub phase: String,
    pub frequency: u32,
    pub jitter: u32,
}


/// Mensaje de latido (Heartbeat) para indicar a los Hubs que el Edge está vivo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Heartbeat {
    pub metadata: Metadata,
    pub beat: bool,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HelloWorld {
    pub metadata: Metadata,
    pub hello: bool,
}


/// Comando para eliminar un Hub del registro.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteHub {
    pub metadata: Metadata,  // El destination_id es el hub destino
    pub network: String,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveHub {
    pub metadata: Metadata,  // El destination_id es el hub destino
    pub network: String,
    pub active: bool,
}


/// Confirmación de recepción de configuración (Handshake bidireccional).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettingOk {
    pub metadata: Metadata,
    pub network: String,
    pub handshake: bool,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateFirmware {
    pub metadata: Metadata,
    pub network: String,
    pub version: String,
    pub url: String,
    pub sha256: String,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct FirmwareOk {
    pub metadata: Metadata,
    pub version: String,
    pub is_ok: bool,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FirmwareOutcome {
    pub metadata: Metadata,
    pub version: String,
    pub is_ok: bool,
    pub percentage_ok: f32,
}


#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct SystemMetrics {
    pub metadata: Metadata,
    pub uptime_seconds: u64,
    pub cpu_usage_percent: f32,
    pub cpu_temp_celsius: f32,
    pub ram_total_mb: u64,
    pub ram_used_mb: u64,
    pub sd_total_gb: u64,
    pub sd_used_gb: u64,
    pub sd_usage_percent: f32,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
    pub wifi_rssi: Option<i32>,
    pub wifi_signal_dbm: Option<i32>,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Message {
    // Mensajes provenientes del Hub
    Report(Measurement),
    Monitor(Monitor),
    AlertAir(AlertAir),
    AlertTem(AlertTh),
    HandshakeFromHub(HandshakeFromHub),
    FirmwareOk(FirmwareOk),
    FromHubSettings(Settings),
    FromHubSettingsAck(SettingOk),

    // Mensajes para el Hub
    Heartbeat(Heartbeat),
    HandshakeToHub(HandshakeToHub),
    PhaseNotification(PhaseNotification),

    // Mensajes provenientes del Server
    UpdateFirmware(UpdateFirmware),
    DeleteHub(DeleteHub),
    ActiveHub(ActiveHub),
    FromServerSettings(Settings),
    FromServerSettingsAck(SettingOk),
    Network(Network),
    Metrics(SystemMetrics),

    // Mensajes para el Server
    FirmwareOutcome(FirmwareOutcome),
    HelloWorld(HelloWorld),

    // Mensajes para el Server y el Hub
    StateBalanceMode(MessageStateBalanceMode),
    StateNormal(MessageStateNormal),
    StateSafeMode(MessageStateSafeMode),
}


/// Estado de conexión con el servidor remoto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerStatus { Connected, Disconnected }


/// Estado de conexión con el broker MQTT local.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalStatus { Connected, Disconnected }


/// Representación final de un mensaje listo para ser enviado por MQTT.
///
/// Contiene el payload binario (serializado) y los parámetros de transporte.
#[derive(Debug, Serialize, Deserialize)]
pub struct SerializedMessage {
    topic: String,
    payload: Vec<u8>,
    qos: u8,
    retain: bool,
}


impl SerializedMessage {
    pub fn new(topic: String,
               payload: Vec<u8>,
               qos: u8,
               retain: bool) -> Self {
        Self {
            topic,
            payload,
            qos,
            retain,
        }
    }
    pub fn get_topic(&self) -> &str {
        &self.topic
    }
    pub fn get_payload(&self) -> &[u8] {
        &self.payload
    }
    pub fn get_qos(&self) -> u8 {
        self.qos
    }
    pub fn get_retain(&self) -> bool {
        self.retain
    }
}