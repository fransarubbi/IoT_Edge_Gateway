use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::time::sleep;
use crate::database::domain::{DataRequest, TableDataVector};
use crate::database::logic::{dba_get_task, dba_insert_task, dba_task};
use crate::firmware::domain::{firmware_watchdog_timer, Action, Event};
use crate::firmware::logic::{run_fsm_firmware, update_firmware_task};
use crate::grpc::EdgeUpload;
use crate::message::domain::{Message, SerializedMessage};
use crate::message::logic::{msg_from_hub, msg_from_server, msg_to_hub, msg_to_server};
use crate::mqtt::local::local_mqtt;
use crate::network::domain::{HubChanged, NetworkChanged, UpdateNetwork};
use crate::network::logic::{network_admin, network_dba};
use crate::system::domain::{init_tracing, InternalEvent};
use crate::system::fsm::init_fsm;

mod system;
mod mqtt;
mod message;
mod database;
mod config;
mod network;
mod fsm;
mod context;
mod firmware;
mod metrics;
mod grpc_service;

pub mod grpc {
    tonic::include_proto!("grpc");
}

#[tokio::main]
async fn main() {

    init_tracing();

    let (msg_to_hub_tx_to_mqtt, local_mqtt_rx) = mpsc::channel::<SerializedMessage>(100);
    let (msg_to_server_tx_to_mqtt, server_mqtt_rx) = mpsc::channel::<EdgeUpload>(100);
    let (mqtt_server_tx, rx_from_mqtt_server) = mpsc::channel::<InternalEvent>(100);
    let (mqtt_local_tx, rx_from_mqtt_hub) = mpsc::channel::<InternalEvent>(100);

    let (msg_from_server_tx_to_fsm, fsm_rx_from_server) = mpsc::channel::<Message>(100);
    let (msg_from_server_tx_to_hub, msg_to_hub_rx_from_server) = mpsc::channel::<Message>(100);
    let (msg_from_server_tx_to_network, network_rx_from_server) = mpsc::channel::<Message>(100);
    let (msg_from_server_tx_to_dba, dba_rx_from_server) = mpsc::channel::<InternalEvent>(100);
    let (msg_from_server_tx_to_from_hub, msg_from_hub_rx_from_server) = mpsc::channel::<InternalEvent>(100);
    let (msg_from_server_tx_to_server, msg_to_server_rx_from_server) = mpsc::channel::<InternalEvent>(100);
    let (msg_from_server_tx_to_firmware, update_firmware_rx_from_server) = mpsc::channel::<Message>(100);

    let (msg_from_hub_tx_to_hub, msg_to_hub_rx_from_hub) = mpsc::channel::<InternalEvent>(100);
    let (msg_from_hub_tx_to_server, msg_to_server_rx_from_hub) = mpsc::channel::<Message>(100);
    let (msg_from_hub_tx_to_dba, dba_task_rx_from_msg) = mpsc::channel::<Message>(100);
    let (msg_from_hub_tx_to_fsm, fsm_rx_from_msg) = mpsc::channel::<Message>(100);
    let (msg_from_hub_tx_to_network, network_rx_form_hub) = mpsc::channel::<Message>(100);
    let (msg_from_hub_tx_to_firmware, update_firmware_rx_form_hub) = mpsc::channel::<Message>(100);

    let (network_tx_to_insert, network_insert_rx_from_network) = mpsc::channel::<NetworkChanged>(100);
    let (network_tx_to_dba_insert, dba_insert_rx_from_network) = mpsc::channel::<UpdateNetwork>(100);
    let (network_tx_to_hub, msg_to_hub_rx_from_network) = mpsc::channel::<Message>(100);
    let (network_tx_to_insert_hub, network_dba_rx_from_network) = mpsc::channel::<HubChanged>(100);

    let (fsm_tx_to_hub, msg_to_hub_rx_from_fsm) = mpsc::channel::<Message>(100);
    let (fsm_tx_to_server, msg_to_server_rx_from_fsm) = mpsc::channel::<Message>(100);


    let (run_fsm_firmware_tx_actions, update_network_rx_from_fsm) = mpsc::channel::<Vec<Action>>(100);

    let (update_firmware_tx_to_hub, msg_to_hub_rx_from_update_firmware) = mpsc::channel::<Message>(100);
    let (update_firmware_tx_to_server, msg_to_server_rx_from_update_firmware) = mpsc::channel::<Message>(100);
    let (update_firmware_tx_to_timer, timer_rx_from_update_firmware) = mpsc::channel::<Event>(100);

    let (tx_to_fsm_firmware, run_fsm_firmware_rx_event) = mpsc::channel::<Event>(100);

    let (dba_task_tx_to_server, msg_to_server_rx_from_dba) = mpsc::channel::<Message>(100);
    let (dba_task_tx_to_server_batch, msg_to_server_rx_from_dba_batch) = mpsc::channel::<TableDataVector>(100);
    let (dba_task_tx_to_insert, dba_insert_task_rx_from_dba) = mpsc::channel::<Message>(100);
    let (dba_task_tx_to_db, dba_get_rx_from_dba) = watch::channel(DataRequest::NotGet);
    let (dba_get_tx_to_dba, dba_task_rx_from_db) = mpsc::channel::<TableDataVector>(100);


    let app_context = match init_fsm().await {
        Ok(app_context) => app_context,
        Err(_) => {
            return;
        }
    };


    /*
    tokio::spawn(run_fsm(
        
    ));*/

    tokio::spawn(local_mqtt(mqtt_local_tx,
                            local_mqtt_rx,
                            app_context.clone()
    ));



    
    tokio::spawn(msg_from_hub(msg_from_hub_tx_to_hub,
                              msg_from_hub_tx_to_server,
                              msg_from_hub_tx_to_dba,
                              msg_from_hub_tx_to_fsm,
                              msg_from_hub_tx_to_network,
                              msg_from_hub_tx_to_firmware,
                              rx_from_mqtt_hub,
                              msg_from_hub_rx_from_server
    ));

    tokio::spawn(msg_to_hub(msg_to_hub_tx_to_mqtt,
                            msg_to_hub_rx_from_fsm,
                            msg_to_hub_rx_from_server,
                            msg_to_hub_rx_from_hub,
                            msg_to_hub_rx_from_network,
                            msg_to_hub_rx_from_update_firmware,
                            app_context.clone()
    ));

    tokio::spawn(msg_from_server(msg_from_server_tx_to_hub,
                                 msg_from_server_tx_to_network,
                                 msg_from_server_tx_to_dba,
                                 msg_from_server_tx_to_from_hub,
                                 msg_from_server_tx_to_server,
                                 msg_from_server_tx_to_firmware,
                                 rx_from_mqtt_server
    ));

    tokio::spawn(msg_to_server(msg_to_server_tx_to_mqtt,
                               msg_to_server_rx_from_fsm,
                               msg_to_server_rx_from_hub,
                               msg_to_server_rx_from_update_firmware,
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

    tokio::spawn(network_admin(network_tx_to_insert,
                              network_tx_to_dba_insert,
                              network_tx_to_hub,
                              network_tx_to_insert_hub,
                              network_rx_from_server,
                              network_rx_form_hub,
                              app_context.clone()
    ));

    tokio::spawn(network_dba(network_insert_rx_from_network,
                                  network_dba_rx_from_network,
                                  app_context.clone()
    ));

    let update_firmware_tx_to_fsm = tx_to_fsm_firmware.clone();
    tokio::spawn(update_firmware_task(update_firmware_tx_to_hub,
                                      update_firmware_tx_to_server,
                                      update_firmware_tx_to_fsm,
                                      update_firmware_tx_to_timer,
                                      update_firmware_rx_from_server,
                                      update_firmware_rx_form_hub,
                                      update_network_rx_from_fsm,
                                      app_context.clone()
    ));

    tokio::spawn(run_fsm_firmware(run_fsm_firmware_tx_actions,
                                  run_fsm_firmware_rx_event
    ));

    let tx_to_fsm = tx_to_fsm_firmware.clone();
    tokio::spawn(firmware_watchdog_timer(tx_to_fsm,
                                         timer_rx_from_update_firmware
    ));

    loop {
        sleep(Duration::from_secs(60)).await;
    }
}

