use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;
use crate::system::check_system_config::check_system_config;
use crate::system::fix_system_config::fix_system_config;
use crate::system::fsm::{first_config_task, tarea_mqtt};

mod system;
mod mqtt;
mod message;

/*
#[tokio::main]
async fn main() {

    let (tx, rx) = mpsc::channel(32);  // Crear el canal. Capacidad de 32 mensajes en cola.

    // 2. Lanzar la máquina de estados.
    // tokio::spawn es como lanzar un hilo, pero super ligero.
    tokio::spawn(crate::system::fsm::run_fsm(rx));

    // 3. Lanzar la tarea de autoconfiguración
    let tx_config = tx.clone();
    tokio::spawn(first_config_task(tx_config));

    // 4. Lanzar tarea MQTT
    let tx_mqtt = tx.clone();
    tokio::spawn(tarea_mqtt(tx_mqtt));

    // Mantener el main vivo (en un caso real, esperarías señales de terminación como Ctrl+C)
    // tokio::signal::ctrl_c().await.unwrap();
    loop {
        sleep(Duration::from_secs(60)).await;
    }
}*/


fn main() {
    check_system_config();
    fix_system_config();
}
