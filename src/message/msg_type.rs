use std::net::Ipv4Addr;
use rmp_serde::{from_slice, to_vec};
use serde::{Serialize, Deserialize};
use tokio::sync::mpsc;
use crate::message::msg_type::Message::{AlertAir, AlertTem, Monitor, Network, Report, Settings};
use crate::system::fsm::{EventSystem, InternalEvent};


#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DestinationType {
    Node,
    Edge,
    Server,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub sender_user_id: String,
    pub destination_type: DestinationType,
    pub destination_id: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageAlertAir {
    pub metadata: Metadata,
    pub co2_initial_ppm: f32,
    pub co2_actual_ppm: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageAlertTh {
    pub metadata: Metadata,
    pub initial_temp: f32,
    pub actual_temp: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageNetwork {
    pub metadata: Metadata,
    pub id_network: String,
    pub name_network: String,
    pub topic_data: String,
    pub topic_alert: String,
    pub topic_monitor: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSettings {
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


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Message {
    Report(MessageMeasurement),
    Monitor(MessageMonitor),
    AlertAir(MessageAlertAir),
    AlertTem(MessageAlertTh),
    Network(MessageNetwork),
    Settings(MessageSettings),
}




pub async fn process_event_message(rx: &mut mpsc::Receiver<EventSystem>) {
    let msg = rx.recv().await.unwrap();
    match msg {
        EventSystem::EventServerConnected => {},
        EventSystem::EventServerDisconnected => {},
        EventSystem::EventLocalBrokerConnected => {},
        EventSystem::EventLocalBrokerDisconnected => {},
        EventSystem::EventMessage(message) => {
            match message {
                Report(data) => {
                    print!("{:?}", data);
                },
                Monitor(monitor) => {
                    print!("{:?}", monitor);
                },
                AlertAir(alert_air) => {
                    print!("{:?}", alert_air);
                },
                AlertTem(alert_tem) => {
                    print!("{:?}", alert_tem);
                },
                Network(network) => {
                    print!("{:?}", network);
                },
                Settings(settings) => {
                    print!("{:?}", settings);
                },
            }
        }
        _ => {},
    }
}


pub async fn internal_message_processor(tx: mpsc::Sender<EventSystem>, mut rx: mpsc::Receiver<InternalEvent>) {
    let msg = rx.recv().await.unwrap();
    match msg {
        InternalEvent::ServerConnected => tx.send(EventSystem::EventServerConnected).await.unwrap(),
        InternalEvent::ServerDisconnected => tx.send(EventSystem::EventServerDisconnected).await.unwrap(),
        InternalEvent::LocalBrokerConnected => tx.send(EventSystem::EventLocalBrokerConnected).await.unwrap(),
        InternalEvent::LocalBrokerDisconnected => tx.send(EventSystem::EventLocalBrokerDisconnected).await.unwrap(),
        InternalEvent::IncomingFromServer(packet) => {
            let decoded = from_slice(&packet).unwrap();
            match decoded {
                Network(network) => {
                    tx.send(EventSystem::EventMessage(Network(network)).try_into().unwrap()).await.unwrap();
                },
                _ => {},
            }
        },
        InternalEvent::FromLocalBroker(packet) => {
            let decoded = from_slice(&packet).unwrap();
            match decoded {
                Report(measurement) => {
                    tx.send(EventSystem::EventMessage(Report(measurement)).try_into().unwrap()).await.unwrap();
                },
                Monitor(monitor) => {
                    tx.send(EventSystem::EventMessage(Monitor(monitor)).try_into().unwrap()).await.unwrap();
                },
                AlertAir(alert_air) => {
                    tx.send(EventSystem::EventMessage(AlertAir(alert_air)).try_into().unwrap()).await.unwrap();
                },
                AlertTem(alert_temp) => {
                    tx.send(EventSystem::EventMessage(AlertTem(alert_temp)).try_into().unwrap()).await.unwrap();
                },
                Settings(settings) => {
                    tx.send(EventSystem::EventMessage(Settings(settings)).try_into().unwrap()).await.unwrap();
                },
                _ => {},
            }
        },
    }
}


pub async fn message_backup(rx: mpsc::Receiver<EventSystem>) {
    // funcion que parsea los mensajes y los guarda en BD (quizas solo parsee y una funcion en BD sea quien guarde)
}

