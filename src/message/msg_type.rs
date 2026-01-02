use serde::{Serialize, Deserialize};
use sqlx::{FromRow, Type};
use crate::system::fsm::{State};


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum DestinationType {
    Node,
    Edge,
    Server,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, FromRow)]
pub struct Metadata {
    pub sender_user_id: String,
    pub destination_type: DestinationType,
    pub destination_id: String,
    pub timestamp: String,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromRow)]
pub struct Measurement {
    #[sqlx(flatten)]
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


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromRow)]
pub struct AlertAir {
    #[sqlx(flatten)]
    pub metadata: Metadata,
    pub co2_initial_ppm: f32,
    pub co2_actual_ppm: f32,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromRow)]
pub struct AlertTh {
    #[sqlx(flatten)]
    pub metadata: Metadata,
    pub initial_temp: f32,
    pub actual_temp: f32,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, FromRow)]
pub struct Monitor {
    #[sqlx(flatten)]
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


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Network {
    pub metadata: Metadata,
    pub id_network: String,
    pub name_network: String,
    pub topic_data: String,
    pub topic_alert: String,
    pub topic_monitor: String,
    pub active: bool,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Settings {
    pub metadata: Metadata,
    pub wifi_ssid: String,
    pub wifi_password: String,
    pub mqtt_uri: String,
    pub device_name: String,
    pub sample: u16,
    pub topic_data: String,
    pub topic_alert: String,
    pub topic_monitor: String,
    pub energy_mode: u8,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandshakeToHub {
    pub metadata: Metadata,
    pub state: State,
    pub balance_epoch: u32,
    pub duration: u32,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandshakeFromHub {
    pub metadata: Metadata,
    pub state: State,
    pub balance_epoch: u32,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StateBalanceMode {
    pub state: State,
    pub balance_epoch: u32,
    pub phase: String,
    pub duration: u32,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StateNormal {
    pub state: State,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StateSafeMode {
    pub state: State,
    pub duration: u32,
    pub frequency: u32,
    pub jitter: u32,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum MessageFromHub {
    Report(Measurement),
    Monitor(Monitor),
    AlertAir(AlertAir),
    AlertTem(AlertTh),
    Settings(Settings),
    Handshake(HandshakeFromHub),
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageToHub {
    Handshake(HandshakeToHub),
    StateBalanceMode(StateBalanceMode),
    StateNormal(StateNormal),
    StateSafeMode(StateSafeMode),
    ServerToHub(MessageFromServer),
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageFromServer {
    Network(Network),
    Settings(Settings),
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageToServer {
    HubToServer(MessageFromHub),
}


pub enum BrokerStatus { Connected, Disconnected }


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerStatus { Connected, Disconnected }


pub enum DataRequest { Get, NotGet }


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
