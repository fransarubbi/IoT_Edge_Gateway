use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio::sync::mpsc::Receiver;
use tokio::sync::watch::Sender;
use tokio::time::sleep;
use system::fsm::{run_fsm};
use mqtt::local::run_local_mqtt;
use mqtt::remote::run_remote_mqtt;
use crate::message::msg_type::internal_message_processor;
use crate::system::fsm::{EventSystem, InternalEvent};

mod system;
mod mqtt;
mod message;
mod sqlite;


#[tokio::main]
async fn main() {

    let (tx_mqtt, rx_ip) = mpsc::channel::<InternalEvent>(32);
    let (tx_ip, rx_fsm) = mpsc::channel::<EventSystem>(32);
    let (tx_fsm, mut rx_system) = broadcast::channel::<EventSystem>(10);

    let rx_local_mqtt = tx_fsm.subscribe();
    let rx_remote_mqtt = tx_fsm.subscribe();
    tokio::spawn(run_fsm(tx_fsm, rx_fsm));

    let tx_local_mqtt = tx_mqtt.clone();
    tokio::spawn(run_local_mqtt(tx_local_mqtt, rx_local_mqtt));

    let tx_remote_mqtt = tx_mqtt.clone();
    tokio::spawn(run_remote_mqtt(tx_remote_mqtt, rx_remote_mqtt));

    tokio::spawn(internal_message_processor(tx_ip, rx_ip));



    // Mantener el main vivo
    //tokio::signal::ctrl_c().await.unwrap();
    loop {
        sleep(Duration::from_secs(60)).await;
    }
}


/*
fn main() {
    check_system_config();
    fix_system_config();
}*/
