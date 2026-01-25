use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::time::sleep;
use crate::database::domain::{DataRequest, TableDataVector};
use crate::database::logic::{dba_get_task, dba_insert_task, dba_task};
use crate::firmware::domain::{firmware_watchdog_timer, Action, Event};
use crate::firmware::logic::{run_fsm_firmware, update_firmware_task};
use crate::fsm::logic::{fsm_general_channels, heartbeat_generator, heartbeat_to_send_watchdog_timer, run_fsm};
use crate::grpc::EdgeUpload;
use crate::grpc_service::logic::remote_grpc;
use crate::heartbeat::logic::{heartbeat, run_fsm_heartbeat};
use crate::message::domain::{Message, SerializedMessage};
use crate::message::logic::{msg_from_hub, msg_from_server, msg_to_hub, msg_to_server};
use crate::mqtt::local::local_mqtt;
use crate::network::domain::{HubChanged, NetworkChanged, UpdateNetwork};
use crate::network::logic::{network_admin, network_dba};
use crate::system::domain::{init_tracing, InternalEvent};
use crate::system::fsm::init_fsm;
use crate::heartbeat::domain::{Event as HeartbeatEvent, Action as HeartbeatAction, watchdog_timer_for_heartbeat};
use crate::fsm::domain::{Event as FsmEvent, Action as FsmAction, general_fsm_watchdog_timer};
use crate::metrics::logic::{metrics_timer, system_metrics, MetricsTimerEvent};

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
mod heartbeat;
mod quorum;

pub mod grpc {
    tonic::include_proto!("grpc");
}

#[tokio::main]
async fn main() {

    init_tracing();

    let (heartbeat_tx, broadcast_rx) = watch::channel(InternalEvent::ServerDisconnected);
    let (heartbeat_tx_to_timer, timer_rx_from_heartbeat) = mpsc::channel::<HeartbeatEvent>(100);

    let (fsm_heartbeat_tx, fsm_heartbeat_rx) = mpsc::channel::<HeartbeatEvent>(100);
    let (fsm_heartbeat_tx_to_heartbeat, heartbeat_rx_from_heartbeat) = mpsc::channel::<Vec<HeartbeatAction>>(100);

    let (fsm_general_tx_to_hub, msg_to_hub_rx_from_fsm_general) = mpsc::channel::<Message>(100);
    let (fsm_general_tx_to_server, msg_to_server_rx_from_fsm_general) = mpsc::channel::<Message>(100);
    let (fsm_general_tx_to_timer, timer_rx_from_fsm_general) = mpsc::channel::<FsmEvent>(100);
    let (fsm_general_tx_to_heartbeat, heartbeat_rx_from_fsm_general) = mpsc::channel::<FsmAction>(100);

    let (fsm_tx_to_general, general_rx_from_fsm) = mpsc::channel::<Vec<FsmAction>>(100);
    let (fsm_tx, fsm_rx) = mpsc::channel::<FsmEvent>(100);

    let (heartbeat_generator_tx_to_hub, msg_to_hub_rx_from_generator) = mpsc::channel::<Message>(100);
    let (heartbeat_generator_tx_to_timer, timer_rx_from_generator) = mpsc::channel::<FsmEvent>(100);

    let (heartbeat_to_send_tx_to_fsm, heartbeat_generator_rx_from_generator) = mpsc::channel::<FsmEvent>(100);

    let (msg_to_hub_tx_to_mqtt, local_mqtt_rx) = mpsc::channel::<SerializedMessage>(100);
    let (msg_to_server_tx_to_mqtt, server_mqtt_rx) = mpsc::channel::<EdgeUpload>(100);
    let (grpc_server_tx, rx_from_grpc_server) = mpsc::channel::<InternalEvent>(100);
    let (mqtt_local_tx, rx_from_mqtt_hub) = mpsc::channel::<InternalEvent>(100);

    let (msg_from_server_tx_to_hub, msg_to_hub_rx_from_server) = mpsc::channel::<Message>(100);
    let (msg_from_server_tx_to_network, network_rx_from_server) = mpsc::channel::<Message>(100);
    let (msg_from_server_tx_to_firmware, update_firmware_rx_from_server) = mpsc::channel::<Message>(100);
    let (msg_from_server_tx_to_heartbeat, heartbeat_rx_from_server) = mpsc::channel::<Message>(100);

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

    let (metrics_tx_to_server, msg_to_server_rx_from_metrics) = mpsc::channel::<Message>(100);
    let (metrics_tx_to_timer, timer_rx_from_metrics) = mpsc::channel::<MetricsTimerEvent>(100);

    let (metrics_timer_tx_to_metrics, metrics_rx_from_timer) = mpsc::channel::<MetricsTimerEvent>(100);


    let app_context = match init_fsm().await {
        Ok(app_context) => app_context,
        Err(_) => {
            return;
        }
    };

    let heartbeat_tx_to_fsm = fsm_heartbeat_tx.clone();
    tokio::spawn(heartbeat(heartbeat_tx,
                           heartbeat_tx_to_fsm,
                           heartbeat_tx_to_timer,
                           heartbeat_rx_from_server,
                           heartbeat_rx_from_heartbeat
    ));

    tokio::spawn(run_fsm_heartbeat(fsm_heartbeat_tx_to_heartbeat,
                                   fsm_heartbeat_rx
    ));

    let watchdog_heartbeat_tx_to_fsm = fsm_heartbeat_tx.clone();
    tokio::spawn(watchdog_timer_for_heartbeat(watchdog_heartbeat_tx_to_fsm,
                                              timer_rx_from_heartbeat
    ));

    let fsm_general_tx_to_fsm = fsm_tx.clone();
    tokio::spawn(fsm_general_channels(fsm_general_tx_to_hub,
                                      fsm_general_tx_to_server,
                                      fsm_general_tx_to_fsm,
                                      fsm_general_tx_to_timer,
                                      fsm_general_tx_to_heartbeat,
                                      fsm_rx_from_msg,
                                      general_rx_from_fsm,
                                      app_context.clone()
    ));

    tokio::spawn(run_fsm(fsm_tx_to_general,
                         fsm_rx
    ));

    tokio::spawn(heartbeat_to_send_watchdog_timer(heartbeat_to_send_tx_to_fsm,
                                                  timer_rx_from_generator
    ));

    tokio::spawn(heartbeat_generator(heartbeat_generator_tx_to_hub,
                                     heartbeat_generator_tx_to_timer,
                                     heartbeat_rx_from_fsm_general,
                                     heartbeat_generator_rx_from_generator,
                                     app_context.clone()
    ));

    let watchdog_fsm_general_tx_to_fsm = fsm_tx.clone();
    tokio::spawn(general_fsm_watchdog_timer(watchdog_fsm_general_tx_to_fsm,
                                            timer_rx_from_fsm_general
    ));

    tokio::spawn(local_mqtt(mqtt_local_tx,
                            local_mqtt_rx,
                            app_context.clone()
    ));

    tokio::spawn(remote_grpc(grpc_server_tx,
                             server_mqtt_rx,
                             app_context.clone()
    ));

    let msg_from_hub_rx_from_server = broadcast_rx.clone();
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
                            msg_to_hub_rx_from_fsm_general,
                            msg_to_hub_rx_from_server,
                            msg_to_hub_rx_from_hub,
                            msg_to_hub_rx_from_network,
                            msg_to_hub_rx_from_update_firmware,
                            msg_to_hub_rx_from_generator,
                            app_context.clone()
    ));

    tokio::spawn(msg_from_server(msg_from_server_tx_to_hub,
                                 msg_from_server_tx_to_network,
                                 msg_from_server_tx_to_firmware,
                                 msg_from_server_tx_to_heartbeat,
                                 rx_from_grpc_server
    ));

    let msg_to_server_rx_from_server = broadcast_rx.clone();
    tokio::spawn(msg_to_server(msg_to_server_tx_to_mqtt,
                               msg_to_server_rx_from_fsm_general,
                               msg_to_server_rx_from_hub,
                               msg_to_server_rx_from_update_firmware,
                               msg_to_server_rx_from_dba_batch,
                               msg_to_server_rx_from_server,
                               msg_to_server_rx_from_metrics,
                               app_context.clone()
    ));

    let dba_rx_from_heartbeat = broadcast_rx.clone();
    tokio::spawn(dba_task(dba_task_tx_to_server,
                          dba_task_tx_to_server_batch,
                          dba_task_tx_to_insert,
                          dba_task_tx_to_db,
                          dba_task_rx_from_msg,
                          dba_rx_from_heartbeat,
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

    tokio::spawn(system_metrics(metrics_tx_to_server,
                                metrics_tx_to_timer,
                                metrics_rx_from_timer,
                                app_context.clone()
    ));

    tokio::spawn(metrics_timer(metrics_timer_tx_to_metrics,
                               timer_rx_from_metrics
    ));

    loop {
        sleep(Duration::from_secs(60)).await;
    }
}

