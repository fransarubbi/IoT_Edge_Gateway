use std::net::Ipv4Addr;
use serde::{Serialize, Deserialize};


#[derive(Debug, Serialize, Deserialize)]
pub struct Metadata {
    pub sender_user_id: String,
    pub destination_type: String,
    pub destination_id: String,
    pub timestamp: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageMeasurement {
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

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageAlertAir {
    pub metadata: Metadata,
    pub co2_initial_ppm: f32,
    pub co2_actual_ppm: f32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageAlertTh {
    pub metadata: Metadata,
    pub initial_temp: f32,
    pub actual_temp: f32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageMonitor {
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

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageNetwork {
    pub metadata: Metadata,
    pub id_network: String,
    pub name_network: String,
    pub topic_data: String,
    pub topic_alert: String,
    pub topic_monitor: String,
    pub active: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageSettings {
    pub metadata: Metadata,
    pub wifi_ssid: String,
    pub wifi_password: String,
    pub mqtt_uri: String,
    pub mqtt_user: String,
    pub mqtt_password: String,
    pub device_name: String,
    pub sample: u16,
    pub topic_data: String,
    pub topic_alert: String,
    pub topic_monitor: String,
    pub energy_mode: u8,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Message {
    Report(MessageMeasurement),
    Monitor(MessageMonitor),
    AlertAir(MessageAlertAir),
    AlertTem(MessageAlertTh),
    Network(MessageNetwork),
    Settings(MessageSettings),
}


pub fn parse_message(msg: &mut Message) {
    match msg {
        Message::Report(measurement) => {
            println!("Guardando humedad: {}", measurement.humidity);
        },
        Message::Monitor(monitor) => {
            println!("Uptime del nodo: {}", monitor.active_time);
        },
        Message::AlertAir(alert_air) => {
            println!("ALARMA: {}", alert_air.co2_actual_ppm);
        },
        Message::AlertTem(alert_temp) => {
            println!("Alerta temperatura: {}", alert_temp.actual_temp)
        },
        Message::Network(network) => {
            println!("Red: {}", network.id_network)
        },
        Message::Settings(settings) => {
            println!("Settings: {}", settings.metadata.sender_user_id);
        }
    }
}