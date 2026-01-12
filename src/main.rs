use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::time::sleep;
use tracing::error;
use crate::database::domain::{DataRequest, TableDataVector};
use crate::database::logic::{dba_get_task, dba_insert_task, dba_task};
use crate::fsm::domain::{InternalEvent};
use crate::fsm::logic::fsm;
use crate::message::domain::{MessageFromHub, MessageFromServer, MessageToHub, MessageToServer, SerializedMessage, ServerStatus};
use crate::message::logic::{msg_from_hub, msg_from_server, msg_to_hub, msg_to_server};
use crate::mqtt::local::local_mqtt;
use crate::mqtt::remote::remote_mqtt;
use crate::network::domain::{NetworkChanged, UpdateNetwork};
use crate::network::logic::{network_dba_task, network_task};
use crate::system::domain::{init_tracing};
use crate::system::fsm::init_fsm;

mod system;
mod mqtt;
mod message;
mod database;
mod config;
mod network;
mod fsm;
mod context;

#[tokio::main]
async fn main() {

    init_tracing();

    let (msg_to_hub_tx_to_mqtt, local_mqtt_rx) = mpsc::channel::<SerializedMessage>(100);
    let (msg_to_server_tx_to_mqtt, server_mqtt_rx) = mpsc::channel::<SerializedMessage>(100);
    let (mqtt_server_tx, rx_from_mqtt_server) = mpsc::channel::<InternalEvent>(100);
    let (mqtt_local_tx, rx_from_mqtt_hub) = mpsc::channel::<InternalEvent>(100);

    let (msg_from_server_tx_to_fsm, fsm_rx_from_server) = mpsc::channel::<MessageFromServer>(100);
    let (msg_from_server_tx_to_hub, msg_to_hub_rx_from_server) = mpsc::channel::<MessageToHub>(100);
    let (msg_from_server_tx_to_network, network_rx_from_server) = mpsc::channel::<MessageFromServer>(100);
    let (msg_from_server_tx_to_dba, dba_rx_from_server) = mpsc::channel::<InternalEvent>(100);
    let (msg_from_server_tx_to_from_hub, msg_from_hub_rx_from_server) = mpsc::channel::<InternalEvent>(100);
    let (msg_from_server_tx_to_server, msg_to_server_rx_from_server) = mpsc::channel::<InternalEvent>(100);

    let (msg_from_hub_tx_to_hub, msg_to_hub_rx_from_hub) = mpsc::channel::<InternalEvent>(100);
    let (msg_from_hub_tx_to_server, msg_to_server_rx_from_hub) = mpsc::channel::<MessageToServer>(100);
    let (msg_from_hub_tx_to_dba, dba_task_rx_from_msg) = mpsc::channel::<MessageFromHub>(100);
    let (msg_from_hub_tx_to_fsm, fsm_rx_from_msg) = mpsc::channel::<MessageFromHub>(100);

    let (network_tx_to_insert, network_insert_rx_from_network) = mpsc::channel::<NetworkChanged>(100);
    let (network_tx_to_dba_insert, dba_insert_rx_from_network) = mpsc::channel::<UpdateNetwork>(100);
    let (network_tx_to_hub, msg_to_hub_rx_from_network) = mpsc::channel::<MessageToHub>(100);

    let (fsm_tx_to_hub, msg_to_hub_rx_from_fsm) = mpsc::channel::<MessageToHub>(100);
    let (fsm_tx_to_server, msg_to_server_rx_from_fsm) = mpsc::channel::<MessageToServer>(100);

    let (dba_task_tx_to_server, msg_to_server_rx_from_dba) = mpsc::channel::<MessageFromHub>(100);
    let (dba_task_tx_to_server_batch, msg_to_server_rx_from_dba_batch) = mpsc::channel::<TableDataVector>(100);
    let (dba_task_tx_to_insert, dba_insert_task_rx_from_dba) = mpsc::channel::<MessageFromHub>(100);
    let (dba_task_tx_to_db, dba_get_rx_from_dba) = watch::channel(DataRequest::NotGet);
    let (dba_get_tx_to_dba, dba_task_rx_from_db) = mpsc::channel::<TableDataVector>(100);


    let app_context = match init_fsm().await {
        Ok(app_context) => app_context,
        Err(e) => {
            error!("{}", e);
            return;
        }
    };


    /*
    tokio::spawn(fsm(
        
    ));*/

    tokio::spawn(local_mqtt(mqtt_local_tx,
                            local_mqtt_rx,
                            app_context.clone()
    ));

    tokio::spawn(remote_mqtt(mqtt_server_tx,
                             server_mqtt_rx,
                             app_context.clone()
    ));

    tokio::spawn(msg_from_hub(msg_from_hub_tx_to_hub,
                              msg_from_hub_tx_to_server,
                              msg_from_hub_tx_to_dba,
                              msg_from_hub_tx_to_fsm,
                              rx_from_mqtt_hub,
                              msg_from_hub_rx_from_server
    ));

    tokio::spawn(msg_to_hub(msg_to_hub_tx_to_mqtt,
                            msg_to_hub_rx_from_fsm,
                            msg_to_hub_rx_from_server,
                            msg_to_hub_rx_from_hub,
                            msg_to_hub_rx_from_network,
                            app_context.clone()
    ));

    tokio::spawn(msg_from_server(msg_from_server_tx_to_fsm,
                                 msg_from_server_tx_to_hub,
                                 msg_from_server_tx_to_network,
                                 msg_from_server_tx_to_dba,
                                 msg_from_server_tx_to_from_hub,
                                 msg_from_server_tx_to_server,
                                 rx_from_mqtt_server
    ));

    tokio::spawn(msg_to_server(msg_to_server_tx_to_mqtt,
                               msg_to_server_rx_from_fsm,
                               msg_to_server_rx_from_hub,
                               msg_to_server_rx_from_dba_batch,
                               msg_to_server_rx_from_server,
                               app_context.clone()
    ));

    tokio::spawn(dba_task(dba_task_tx_to_server,
                          dba_task_tx_to_server_batch,
                          dba_task_tx_to_insert,
                          dba_task_tx_to_db,
                          dba_task_rx_from_msg,
                          dba_rx_from_server,
                          dba_task_rx_from_db,
                          app_context.clone()
    ));

    tokio::spawn(dba_insert_task(dba_insert_task_rx_from_dba,
                                 dba_insert_rx_from_network,
                                 app_context.clone()
    ));

    tokio::spawn(dba_get_task(dba_get_tx_to_dba,
                              dba_get_rx_from_dba,
                              app_context.clone()
    ));

    tokio::spawn(network_task(network_tx_to_insert,
                              network_tx_to_dba_insert,
                              network_tx_to_hub,
                              network_rx_from_server,
                              app_context.clone()
    ));

    tokio::spawn(network_dba_task(network_insert_rx_from_network,
                                  app_context.clone()
    ));

    loop {
        sleep(Duration::from_secs(60)).await;
    }
}

