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
    sender_user_id: String,
    destination_type: DestinationType,
    destination_id: String,
    timestamp: String,
}


impl Metadata {
    pub fn new(sender_user_id: String,
               destination_type: DestinationType,
               destination_id: String,
               timestamp: String) -> Self {
        Self {
            sender_user_id,
            destination_type,
            destination_id,
            timestamp,
        }
    }
    
    pub fn set_sender_user_id(&mut self, sender_user_id: String) {
        self.sender_user_id = sender_user_id;
    }
    pub fn set_destination_type(&mut self, destination_type: DestinationType) {
        self.destination_type = destination_type;
    }
    pub fn set_destination_id(&mut self, destination_id: String) {
        self.destination_id = destination_id;
    }
    pub fn set_timestamp(&mut self, timestamp: String) {
        self.timestamp = timestamp;
    }
    pub fn get_sender_user_id(&self) -> &str {
        &self.sender_user_id
    }
    pub fn get_destination_type(&self) -> DestinationType {
        self.destination_type.clone()
    }
    pub fn get_destination_id(&self) -> &str {
        &self.destination_id
    }
    pub fn get_timestamp(&self) -> &str {
        &self.timestamp
    }
}


/// Mediciones de sensores ambientales y operativos.
///
/// Representa el paquete de datos principal generado por los nodos.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Measurement {
    metadata: Metadata,
    ipv4addr: String,
    wifi_ssid: String,
    pulse_counter: i64,
    pulse_max_duration: i64,
    temperature: f32,
    humidity: f32,
    co2_ppm: f32,
    sample: u16,
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
    
    pub fn set_metadata(&mut self, metadata: Metadata) {
        self.metadata = metadata;
    }
    pub fn set_wifi_ssid(&mut self, wsif_ssid: String) {
        self.wifi_ssid = wsif_ssid;
    }
    pub fn set_pulse_counter(&mut self, pulse_counter: i64) {
        self.pulse_counter = pulse_counter;
    }
    pub fn set_pulse_max_duration(&mut self, pulse_max_duration: i64) {
        self.pulse_max_duration = pulse_max_duration;
    }
    pub fn set_temperature(&mut self, temperature: f32) {
        self.temperature = temperature;
    }
    pub fn set_humidity(&mut self, humidity: f32) {
        self.humidity = humidity;
    }
    pub fn set_co2_ppm(&mut self, co2_ppm: f32) {
        self.co2_ppm = co2_ppm;
    }
    pub fn set_sample(&mut self, sample: u16) {
        self.sample = sample;
    }
}


/// Alerta de calidad de aire.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertAir {
    metadata: Metadata,
    co2_initial_ppm: f32,
    co2_actual_ppm: f32,
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
    
    pub fn set_metadata(&mut self, metadata: Metadata) {
        self.metadata = metadata;
    }
    pub fn set_co2_ppm(&mut self, co2_ppm: f32) {
        self.co2_initial_ppm = co2_ppm;
    }
    pub fn set_co2_actual_ppm(&mut self, co2_actual_ppm: f32) {
        self.co2_actual_ppm = co2_actual_ppm;
    }
}


/// Alerta de Temperatura y Humedad.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertTh {
    metadata: Metadata,
    initial_temp: f32,
    actual_temp: f32,
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
    
    pub fn set_metadata(&mut self, metadata: Metadata) {
        self.metadata = metadata;
    }
    pub fn set_initial_temp(&mut self, initial_temp: f32) {
        self.initial_temp = initial_temp;
    }
    pub fn set_actual_temp(&mut self, actual_temp: f32) {
        self.actual_temp = actual_temp;
    }
}


/// Datos de telemetría y salud del Hub.
///
/// Incluye información sobre memoria, stack y conectividad para diagnóstico.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Monitor {
    metadata: Metadata,
    mem_free: i64,
    mem_free_hm: i64,
    mem_free_block: i64,
    mem_free_internal: i64,
    stack_free_min_coll: i64,
    stack_free_min_pub: i64,
    stack_free_min_mic: i64,
    stack_free_min_th: i64,
    stack_free_min_air: i64,
    stack_free_min_mon: i64,
    wifi_ssid: String,
    wifi_rssi: i8,
    active_time: String,
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
    
    pub fn set_metadata(&mut self, metadata: Metadata) {
        self.metadata = metadata;
    }
    pub fn set_mem_free(&mut self, mem_free: i64) {
        self.mem_free = mem_free;
    }
    pub fn set_mem_free_block(&mut self, mem_free_block: i64) {
        self.mem_free_block = mem_free_block;
    }
    pub fn set_mem_free_internal(&mut self, mem_free_internal: i64) {
        self.mem_free_internal = mem_free_internal;
    }
    pub fn set_stack_free_min_coll(&mut self, stack_free_min_coll: i64) {
        self.stack_free_min_coll = stack_free_min_coll;
    }
    pub fn set_stack_free_min_pub(&mut self, stack_free_min_pub: i64) {
        self.stack_free_min_pub = stack_free_min_pub;
    }
    pub fn set_stack_free_min_mic(&mut self, stack_free_min_mic: i64) {
        self.stack_free_min_mic = stack_free_min_mic;
    }
    pub fn set_stack_free_min_th(&mut self, stack_free_min_th: i64) {
        self.stack_free_min_th = stack_free_min_th;
    }
    pub fn set_stack_free_min_air(&mut self, stack_free_min_air: i64) {
        self.stack_free_min_air = stack_free_min_air;
    }
    pub fn set_stack_free_min_mon(&mut self, stack_free_min_mon: i64) {
        self.stack_free_min_mon = stack_free_min_mon;
    }
    pub fn set_wifi_ssid(&mut self, wifi_ssid: String) {
        self.wifi_ssid = wifi_ssid;
    }
    pub fn set_wifi_rssi(&mut self, wifi_rssi: i8) {
        self.wifi_rssi = wifi_rssi;
    }
    pub fn set_active_time(&mut self, active_time: String) {
        self.active_time = active_time;
    }
}


/// Definición de una Red lógica.
///
/// Utilizada para agrupar dispositivos bajo un mismo identificador de red.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Network {
    metadata: Metadata,
    id_network: String,
    name_network: String,
    active: bool,
    delete_network: bool,
}


impl Network {
    pub fn get_id_network(&self) -> String {
        self.id_network.clone()
    }
    pub fn get_name_network(&self) -> String {
        self.name_network.clone()
    }
    pub fn get_active(&self) -> bool {
        self.active
    }
    pub fn get_delete_network(&self) -> bool {
        self.delete_network
    }
}


/// Configuración remota para un dispositivo (Hub/Nodo).
///
/// Contiene credenciales WiFi/MQTT y parámetros operativos.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Settings {
    metadata: Metadata,
    wifi_ssid: String,
    wifi_password: String,
    mqtt_uri: String,
    device_name: String,
    sample: u16,
    energy_mode: u8,
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
    pub fn get_metadata(&self) -> Metadata {
        self.metadata.clone()
    }
}


/// Mensaje de Handshake enviado HACIA el Hub (Downlink).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandshakeToHub {
    metadata: Metadata,
    balance_epoch: u32,
    duration: u32,
}


impl HandshakeToHub {
    pub fn new(metadata: Metadata, balance_epoch: u32, duration: u32) -> Self {
        Self { metadata, balance_epoch, duration }
    }
}


/// Mensaje de Handshake proveniente DEL Hub (Uplink).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandshakeFromHub {
    metadata: Metadata,
    state: String,
    balance_epoch: u32,
}


/// Notificación de cambio a Modo Balance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageStateBalanceMode {
    state: String,
    balance_epoch: u32,
    sub_state: String,
    duration: u32,
}


impl MessageStateBalanceMode {
    pub fn new(state: String, balance_epoch: u32, sub_state: String, duration: u32) -> Self {
        Self { state, balance_epoch, sub_state, duration }
    }
}


/// Notificación de cambio a Modo Normal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageStateNormal {
    state: String,
}


/// Notificación de cambio a Modo Seguro (Safe Mode).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageStateSafeMode {
    state: String,
    duration: u32,
    frequency: u32,
    jitter: u32,
}


/// Notificación de cambio de Fase dentro del modo Balance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhaseNotification {
    metadata: Metadata,
    state: String,
    epoch: u32,
    phase: String,
    frequency: u32,
    jitter: u32,
}


/// Mensaje de latido (Heartbeat) para indicar a los Hubs que el Edge está vivo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Heartbeat {
    metadata: Metadata,
}


/// Comando para eliminar un Hub del registro.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteHub {
    metadata: Metadata,
    id_hub: String,
}


impl DeleteHub {
    pub fn new(metadata: Metadata, id_hub: String) -> Self {
        Self {
            metadata,
            id_hub,
        }
    }
    pub fn get_id_hub(&self) -> String {
        self.id_hub.clone()
    }
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveHub {
    metadata: Metadata,
    active: bool,
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
    metadata: Metadata,
    handshake: bool,
}


impl SettingOk {
    pub fn get_metadata(&self) -> Metadata {
        self.metadata.clone()
    }
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
    metadata: Metadata,
    version: String,
    is_ok: bool,
}


impl FirmwareOk {
    pub fn get_metadata(&self) -> &Metadata {
        &self.metadata
    }
    pub fn get_is_ok(&self) -> bool {
        self.is_ok
    }
    pub fn get_version(&self) -> &str {
        &self.version
    }
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FirmwareOutcome {
    metadata: Metadata,
    version: String,
    is_ok: bool,
    percentage_ok: f32,
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


// -------------------------------------------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    topic_arrive: String,
    message: MessageTypes,
}


impl Message {
    pub fn new(topic_arrive: String, message: MessageTypes) -> Self {
        Self { topic_arrive, message }
    }
    pub fn get_topic_arrive(&self) -> &str {
        &self.topic_arrive
    }
    pub fn get_message(&self) -> MessageTypes {
        self.message.clone()
    }
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum MessageTypes {
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

    // Mensajes para el Server
    FirmwareOutcome(FirmwareOutcome),

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