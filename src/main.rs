use std::time::Duration;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::sleep;
use tracing::error;
use mqtt::local::run_local_mqtt;
use mqtt::remote::run_remote_mqtt;
use crate::database::domain::{TableDataVector};
use crate::database::logic::{dba_get_task, dba_insert_task, dba_task};
use crate::database::repository::Repository;
use crate::fsm::domain::{EventSystem, InternalEvent};
use crate::message::domain::{BrokerStatus, DataRequest, MessageFromHub, MessageFromServer, MessageToHub, MessageToServer, NetworkChanged, SerializedMessage, ServerStatus};
use crate::message::logic::{msg_from_hub, msg_from_server, msg_to_hub, msg_to_server};
use crate::network::domain::NetworkManager;
use crate::system::domain::{init_tracing, System};
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
    let app_context = match run_init_fsm().await {
        Ok(app_context) => app_context,
        Err(e) => {
            error!("{}", e);
            return;
        }
    };

    // para las tareas hacer app_context.clone()
    
    let (tx_to_server_batch, rx_from_dba_batch) = mpsc::channel::<TableDataVector>(100);
    let (tx_to_insert, rx_to_insert) = mpsc::channel::<MessageFromHub>(100);
    let (tx_to_dba_insert, rx_from_get) = mpsc::channel::<TableDataVector>(100);
    let (tx_to_db, rx_to_db) = watch::channel::<DataRequest>;
    let (tx_to_dba, rx_from_hub) = mpsc::channel::<MessageFromHub>(100);
    let (tx_to_server, rx_from_server) = mpsc::channel::<MessageToServer>(100);
    let (tx_internal, rx_internal) = mpsc::channel::<MessageFromHub>(100);
    let (event_local_tx, event_local_rx) = mpsc::channel::<InternalEvent>(100);
    let (event_server_tx, event_server_rx) = mpsc::channel::<InternalEvent>(100);
    let (tx_fsm, mut rx_system) = broadcast::channel::<EventSystem>(10);
    let (tx_msg_to_hub, rx_serialized_to_hub) = mpsc::channel::<SerializedMessage>(100);

    let repo = Repository::create_repository("a/a/a");
    let mut network_manager = NetworkManager::new("topic/handshake".to_string(), 1, "topic/state".to_string(), 1);
    let system = System::new("edge0".to_string(), "host".to_string(), "host".to_string());


    /*
    tokio::spawn(run_fsm(
        
    ));*/

    let rx_system_local = rx_system.resubscribe();
    tokio::spawn(run_local_mqtt(event_local_tx,
                                rx_system_local,
                                rx_serialized_to_hub,
                                &network_manager,
                                &system
    ));

    let rx_system_server = rx_system.resubscribe();
    tokio::spawn(run_remote_mqtt(event_server_tx,
                                 rx_system_server,
                                 &network_manager,
                                 &system
    ));

    tokio::spawn(dba_task(tx_to_server_batch,
                          tx_to_insert,
                          tx_to_db,
                          rx_from_hub,
                          mut rx_server_status: watch::Receiver<ServerStatus>,
                          rx_from_get,
                          &repo
    ));

    tokio::spawn(dba_insert_task(rx_to_insert,
                                 mut rx_from_net_man: watch::Receiver<NetworkChanged>,
                                 &repo,
                                 &network_manager,  // esta bien?
    ));

    tokio::spawn(dba_get_task(tx_to_dba_insert,
                              mut rx_server_status: watch::Receiver<DataRequest>,
                              &repo,
                              &network_manager
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

