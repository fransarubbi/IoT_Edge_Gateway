use serde::{Serialize, Deserialize};
use sqlx::{Type};
use crate::fsm::domain::{State};
use crate::message::domain_for_table::{AlertAirRow, AlertThRow, MeasurementRow, MonitorRow};


#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub enum DestinationType {
    #[default]
    Node,
    Edge,
    Server,
}


#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Metadata {
    pub sender_user_id: String,
    pub destination_type: DestinationType,
    pub destination_id: String,
    pub timestamp: String,
}


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


#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertAir {
    pub metadata: Metadata,
    pub co2_initial_ppm: f32,
    pub co2_actual_ppm: f32,
}


impl AlertAir {
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


#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertTh {
    pub metadata: Metadata,
    pub initial_temp: f32,
    pub actual_temp: f32,
}


impl AlertTh {
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


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Network {
    pub metadata: Metadata,
    pub id_network: String,
    pub name_network: String,
    pub active: bool,
    pub delete_network: bool,
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
pub struct MessageFromHub {
    pub topic_where_arrive: String,
    pub msg: MessageFromHubTypes,
}


impl MessageFromHub {
    pub fn new(topic_where_arrive: String, msg: MessageFromHubTypes) -> Self {
        Self { topic_where_arrive, msg }
    }
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum MessageFromHubTypes {
    Report(Measurement),
    Monitor(Monitor),
    AlertAir(AlertAir),
    AlertTem(AlertTh),
    Settings(Settings),
    Handshake(HandshakeFromHub),
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageToHub {
    pub topic_to: String,
    pub msg: MessageToHubTypes,
}


impl MessageToHub {
    pub fn new(topic_to: String, msg: MessageToHubTypes) -> Self {
        Self { topic_to, msg }
    }
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageToHubTypes {
    Handshake(HandshakeToHub),
    StateBalanceMode(StateBalanceMode),
    StateNormal(StateNormal),
    StateSafeMode(StateSafeMode),
    ServerToHub(MessageFromServer),
}


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


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageFromServerTypes {
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


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkChanged { Changed, NotChanged }


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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