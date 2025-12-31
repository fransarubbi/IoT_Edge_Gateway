use std::net::Ipv4Addr;
use serde::{Serialize, Deserialize};
use crate::system::fsm::{State};


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DestinationType {
    Node,
    Edge,
    Server,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Metadata {
    pub sender_user_id: String,
    pub destination_type: DestinationType,
    pub destination_id: String,
    pub timestamp: String,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Measurement {
    pub metadata: Metadata,
    pub ipv4addr: Ipv4Addr,
    pub wifi_ssid: String,
    pub pulse_counter: u64,
    pub pulse_max_duration: u64,
    pub temperature: f32,
    pub humidity: f32,
    pub co2_ppm: f32,
    pub sample: u16,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertAir {
    pub metadata: Metadata,
    pub co2_initial_ppm: f32,
    pub co2_actual_ppm: f32,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertTh {
    pub metadata: Metadata,
    pub initial_temp: f32,
    pub actual_temp: f32,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Monitor {
    pub metadata: Metadata,
    pub mem_free: u64,
    pub mem_free_hm: u64,
    pub mem_free_block: u64,
    pub mem_free_internal: u64,
    pub stack_free_min_coll: u64,
    pub stack_free_min_pub: u64,
    pub stack_free_min_mic: u64,
    pub stack_free_min_th: u64,
    pub stack_free_min_air: u64,
    pub stack_free_min_mon: u64,
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


pub enum MessageFlags {
    Handshake,
    Data,
}


enum BrokerStatus { Connected, Disconnected }
enum ServerStatus { Connected, Disconnected }




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
