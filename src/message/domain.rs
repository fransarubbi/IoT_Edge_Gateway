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
use sqlx::Type;
use crate::message::domain_for_table::{AlertAirRow, AlertThRow, HubRow, MeasurementRow, MonitorRow};


/// Tipo de destino.
///
/// - `Node`: Dispositivo final.
/// - `Edge`: Este dispositivo intermediario.
/// - `Server`: El backend central.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type, Hash)]
#[repr(u8)]
pub enum DestinationType {
    #[default]
    Node  = 0,
    Edge  = 1,
    Server = 2,
}


/// Metadatos estándar para todos los mensajes del sistema.
///
/// Proporciona contexto de trazabilidad, origen y destino para cada paquete de datos.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Metadata {
    pub sender_user_id: String,
    pub destination_type: DestinationType,
    pub destination_id: String,
    pub timestamp: String,
}


/// Mediciones de sensores ambientales y operativos.
///
/// Representa el paquete de datos principal generado por los nodos.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Measurement {
    pub metadata: Metadata,
    pub ipv4addr: String,
    pub wifi_ssid: String,
    pub pulse_counter: i64,
    pub pulse_max_duration: i64,
    pub temperature: f32,
    pub humidity: f32,
    pub co2_ppm: f32,
    pub sample: u16,
}


impl Measurement {

    /// Convierte el modelo de memoria `Measurement` a su representación persistible `MeasurementRow`.
    ///
    /// # Parámetros
    /// - `topic`: El tópico MQTT por donde llegó el mensaje (necesario para la traza en DB).
    pub fn cast_measurement_to_row(self, topic: String) -> MeasurementRow {
        let mut mr = MeasurementRow::default();
        mr.metadata.sender_user_id = self.metadata.sender_user_id;
        mr.metadata.destination_type = self.metadata.destination_type;
        mr.metadata.destination_id = self.metadata.destination_id;
        mr.metadata.timestamp = self.metadata.timestamp;
        mr.metadata.topic_where_arrive = topic;
        mr.wifi_ssid = self.wifi_ssid;
        mr.pulse_counter = self.pulse_counter;
        mr.pulse_max_duration = self.pulse_max_duration;
        mr.temperature = self.temperature;
        mr.humidity = self.humidity;
        mr.co2_ppm = self.co2_ppm;
        mr.sample = self.sample;
        mr
    }
}


/// Alerta de calidad de aire.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertAir {
    pub metadata: Metadata,
    pub co2_initial_ppm: f32,
    pub co2_actual_ppm: f32,
}


impl AlertAir {

    /// Convierte la alerta de aire a su formato de fila para base de datos.
    pub fn cast_alert_air_to_row(self, topic: String) -> AlertAirRow {
        let mut aar = AlertAirRow::default();
        aar.metadata.sender_user_id = self.metadata.sender_user_id;
        aar.metadata.destination_type = self.metadata.destination_type;
        aar.metadata.destination_id = self.metadata.destination_id;
        aar.metadata.timestamp = self.metadata.timestamp;
        aar.metadata.topic_where_arrive = topic;
        aar.co2_initial_ppm = self.co2_initial_ppm;
        aar.co2_actual_ppm = self.co2_actual_ppm;
        aar
    }
}


/// Alerta de Temperatura y Humedad.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertTh {
    pub metadata: Metadata,
    pub initial_temp: f32,
    pub actual_temp: f32,
}


impl AlertTh {

    /// Convierte la alerta térmica a su formato de fila para base de datos.
    pub fn cast_alert_th_to_row(self, topic: String) -> AlertThRow {
        let mut ath = AlertThRow::default();
        ath.metadata.sender_user_id = self.metadata.sender_user_id;
        ath.metadata.destination_type = self.metadata.destination_type;
        ath.metadata.destination_id = self.metadata.destination_id;
        ath.metadata.timestamp = self.metadata.timestamp;
        ath.metadata.topic_where_arrive = topic;
        ath.initial_temp = self.initial_temp;
        ath.actual_temp = self.actual_temp;
        ath
    }
}


/// Datos de telemetría y salud del Hub.
///
/// Incluye información sobre memoria, stack y conectividad para diagnóstico.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Monitor {
    pub metadata: Metadata,
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


impl Monitor {

    /// Convierte la telemetría a su formato de fila para base de datos.
    pub fn cast_monitor_to_row(self, topic: String) -> MonitorRow {
        let mut mr = MonitorRow::default();
        mr.metadata.sender_user_id = self.metadata.sender_user_id;
        mr.metadata.destination_type = self.metadata.destination_type;
        mr.metadata.destination_id = self.metadata.destination_id;
        mr.metadata.timestamp = self.metadata.timestamp;
        mr.metadata.topic_where_arrive = topic;
        mr.mem_free_hm = self.mem_free;
        mr.mem_free_block = self.mem_free_block;
        mr.mem_free_internal = self.mem_free_internal;
        mr.stack_free_min_coll = self.stack_free_min_coll;
        mr.stack_free_min_pub = self.stack_free_min_pub;
        mr.stack_free_min_mic = self.stack_free_min_mic;
        mr.stack_free_min_th = self.stack_free_min_th;
        mr.stack_free_min_air = self.stack_free_min_air;
        mr.stack_free_min_mon = self.stack_free_min_mon;
        mr.wifi_ssid = self.wifi_ssid;
        mr.wifi_rssi = self.wifi_rssi;
        mr.active_time = self.active_time;
        mr
    }
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
    pub wifi_ssid: String,
    pub wifi_password: String,
    pub mqtt_uri: String,
    pub device_name: String,
    pub sample: u16,
    pub energy_mode: u8,
}


impl Settings {

    /// Convierte la configuración recibida en un registro de Hub (`HubRow`) para persistencia.
    ///
    /// # Parámetros
    /// - `network`: ID de la red a la que se asocia este dispositivo.
    /// - `topic`: Tópico de llegada.
    pub fn cast_settings_to_hub_row(self, network: String, topic: String) -> HubRow {
        let mut hr = HubRow::default();
        hr.metadata.sender_user_id = self.metadata.sender_user_id;
        hr.metadata.destination_type = self.metadata.destination_type;
        hr.metadata.destination_id = self.metadata.destination_id;
        hr.metadata.timestamp = self.metadata.timestamp;
        hr.metadata.topic_where_arrive = topic;
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
    pub flag: String,
    pub balance_epoch: u32,
    pub duration: u32,
}


impl HandshakeToHub {
    pub fn new(metadata: Metadata, flag: String, balance_epoch: u32, duration: u32) -> Self {
        Self { metadata, flag, balance_epoch, duration }
    }
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
    pub state: String,
    pub balance_epoch: u32,
    pub phase: String,
    pub duration: u32,
}


/// Notificación de cambio a Modo Normal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageStateNormal {
    pub state: String,
}


/// Notificación de cambio a Modo Seguro (Safe Mode).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageStateSafeMode {
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
}


/// Comando para eliminar un Hub del registro.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteHub {
    pub metadata: Metadata,
    pub id_hub: String,
}


impl DeleteHub {
    pub fn new(metadata: Metadata, id_hub: String) -> Self {
        Self {
            metadata,
            id_hub,
        }
    }
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveHub {
    pub metadata: Metadata,
    pub active: bool,
}


impl ActiveHub {
    pub fn new(metadata: Metadata, active: bool) -> Self {
        Self {
            metadata,
            active,
        }
    }
}


/// Confirmación de recepción de configuración (Handshake bidireccional).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettingOk {
    pub metadata: Metadata,
    pub handshake: bool,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateFirmware {
    pub metadata: Metadata,
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


impl FirmwareOutcome {
    pub fn new(metadata: Metadata, version: String, is_ok: bool, percentage_ok: f32) -> Self {
        Self {
            metadata,
            version,
            is_ok,
            percentage_ok,
        }
    }
}


/// Wrapper principal para mensajes provenientes del Hub (Uplink).
///
/// Agrupa el tópico de llegada y el contenido polimórfico del mensaje.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageFromHub {
    pub topic_where_arrive: String,
    pub msg: MessageFromHubTypes,
}


impl MessageFromHub {
    pub fn new(topic_where_arrive: String, msg: MessageFromHubTypes) -> Self {
        Self { topic_where_arrive, msg }
    }
}


/// Enum polimórfico (`untagged`) para todos los tipos de mensajes Uplink.
///
/// Utiliza `#[serde(untagged)]` para que la deserialización se base en la estructura de los campos
/// y no en una etiqueta externa, optimizando el payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum MessageFromHubTypes {
    Report(Measurement),
    Monitor(Monitor),
    AlertAir(AlertAir),
    AlertTem(AlertTh),
    Settings(Settings),
    SettingsOk(SettingOk),
    Handshake(HandshakeFromHub),
    FirmwareOk(FirmwareOk),
    FirmwareOutcome(FirmwareOutcome),
}


/// Wrapper superior para mensajes destinados al Hub (Downlink).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum MessageToHub {
    ToHub(MessageToHubTypes),
}


/// Enum polimórfico (`untagged`) para todos los tipos de mensajes Downlink.
///
/// Incluye comandos de control, cambios de estado y mensajes pasantes del servidor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum MessageToHubTypes {
    Handshake(HandshakeToHub),
    StateBalanceMode(MessageStateBalanceMode),
    StateNormal(MessageStateNormal),
    StateSafeMode(MessageStateSafeMode),
    Heartbeat(Heartbeat),
    PhaseNotification(PhaseNotification),
    ServerToHub(MessageFromServer),
    Settings(Settings),
}


/// Wrapper para mensajes provenientes del Servidor Remoto.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageFromServer {
    pub topic_where_arrive: String,
    pub msg: MessageFromServerTypes,
}


impl MessageFromServer {
    pub fn new(topic_where_arrive: String, msg: MessageFromServerTypes) -> Self {
        Self { topic_where_arrive, msg }
    }
}


/// Enum polimórfico (`untagged`) para comandos remotos del servidor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum MessageFromServerTypes {
    Network(Network),
    Settings(Settings),
    SettingOk(SettingOk),
    DeleteHub(DeleteHub),
    ActiveHub(ActiveHub),
    UpdateFirmware(UpdateFirmware),
}


/// Wrapper de salida hacia el Servidor (Uplink Remoto).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum MessageToServer {
    ToServer(MessageFromHub),
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
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: u8,
    pub retain: bool,
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
}