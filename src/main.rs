use std::time::Duration;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::sleep;
use tracing::error;
use mqtt::local::run_local_mqtt;
use mqtt::remote::run_remote_mqtt;
use crate::database::domain::{TableDataVector};
use crate::database::logic::{dba_get_task, dba_insert_task, dba_task};
use crate::fsm::domain::{InternalEvent};
use crate::message::domain::{DataRequest, MessageFromHub, ServerStatus};
use crate::network::domain::{NetworkChanged, UpdateNetwork};
use crate::network::logic::{network_dba_task, run_network_task};
use crate::system::domain::{init_tracing};
use crate::system::fsm::run_init_fsm;

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

    let (dba_task_tx_to_server, msg_to_server_rx_from_dba) = mpsc::channel::<MessageFromHub>(100);
    let (dba_task_tx_to_server_batch, msg_to_server_rx_from_dba_batch) = mpsc::channel::<TableDataVector>(100);
    let (dba_task_tx_to_insert, dba_insert_task_rx_from_dba) = mpsc::channel::<MessageFromHub>(100);
    let (dba_task_tx_to_db, dba_get_rx_from_dba) = watch::channel(DataRequest::NotGet);
    let (msg_from_hub_tx_to_dba, dba_task_rx_from_msg) = mpsc::channel::<MessageFromHub>(100);
    let (tx_server_status, rx) = watch::channel(ServerStatus::Connected);
    let (dba_get_tx_to_dba, dba_task_rx_from_db) = mpsc::channel::<TableDataVector>(100);
    let (network_tx_to_dba_insert, dba_insert_rx_from_network) = mpsc::channel::<UpdateNetwork>(100);
    let (mqtt_server_tx, _rx_from_mqtt_server) = broadcast::channel::<InternalEvent>(100);
    let (mqtt_local_tx, _rx_from_mqtt_hub) = broadcast::channel::<InternalEvent>(100);




    let app_context = match run_init_fsm().await {
        Ok(app_context) => app_context,
        Err(e) => {
            error!("{}", e);
            return;
        }
    };


    /*
    tokio::spawn(run_fsm(
        
    ));*/


    tokio::spawn(run_local_mqtt(mqtt_local_tx,
                                rx_system_local,
                                rx_serialized_to_hub,
                                &network_manager,
                                &system
    ));

    let rx_network = mqtt_server_tx.subscribe();
    tokio::spawn(run_remote_mqtt(mqtt_server_tx,
                                 rx_system_server,
                                 &network_manager,
                                 &system
    ));

    tokio::spawn(run_network_task(network_tx_to_dba_insert,
                                  rx_network,
                                  app_context.clone()
    ));
    
    tokio::spawn(network_dba_task(
        
    ));

    tokio::spawn(dba_task(dba_task_tx_to_server,
                          dba_task_tx_to_server_batch,
                          dba_task_tx_to_insert,
                          dba_task_tx_to_db,
                          dba_task_rx_from_msg,
                          tx_server_status.subscribe(),
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

    tokio::spawn(msg_from_hub(tx_internal,
                              tx_to_server,
                              tx_to_dba,
                              mut rx_mqtt: mpsc::Receiver<InternalEvent>,
                              mut rx_server_status: watch::Receiver<ServerStatus>
    ));

    tokio::spawn(msg_to_hub(tx_msg_to_hub,
                            mut rx_from_fsm: mpsc::Receiver<MessageToHub>,
                            mut rx_from_server: mpsc::Receiver<MessageToHub>,
                            mut rx_broker_status: watch::Receiver<BrokerStatus>,
                            &network_manager,
                            &system
    ));

    tokio::spawn(msg_from_server(tx_to_fsm: mpsc::Sender<MessageFromServer>,
                                 tx_to_hub: mpsc::Sender<MessageFromServer>,
                                 mut rx_from_server: mpsc::Receiver<InternalEvent>
    ));

    tokio::spawn(msg_to_server(tx_to_server: mpsc::Sender<SerializedMessage>,
                               mut rx_from_fsm: mpsc::Receiver<MessageToServer>,
                               rx_from_hub,
                               rx_from_dba_batch,
                               &network_manager,
                               &system
    ));

    loop {
        sleep(Duration::from_secs(60)).await;
    }
}

