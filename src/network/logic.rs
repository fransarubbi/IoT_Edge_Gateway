use std::sync::Arc;
use rmp_serde::from_slice;
use tokio::sync::{broadcast, watch, RwLock};
use tracing::error;
use crate::context::domain::AppContext;
use crate::database::repository::Repository;
use crate::fsm::domain::InternalEvent;
use crate::message::domain::{MessageFromServerTypes, NetworkChanged};
use crate::network::domain::{Network, NetworkManager};
use crate::network::domain::NetworkAction::{Delete, Ignore, Insert, Update};
use crate::system::domain::System;


/// Carga las redes desde la base de datos, al Network Manager.
///
/// Como de la base de datos obtiene un vector de NetworkRow, se crea
/// el Hash Map y se inserta cada elemento del vector, casteado a Network.
///

pub async fn load_networks(repo: &Repository,
                           net_man: &Arc<RwLock<NetworkManager>>,
                           system: &System,
                           ) {

    match repo.get_all_network().await {
        Ok(networks_row_vec) => {
            let mut manager = net_man.write().await;

            for networks_row in networks_row_vec {
                let network = networks_row.cast_to_network(system);
                manager.add_network(network);
            }
        }
        Err(e) => {
            error!("{}", e);
        }
    }
}


/// Tarea que procesa mensajes del servidor para modificar las redes del sistema.
///
/// Una vez que llega el mensaje, se decodifica. Si la red existe:
/// - Se elimina si `delete_network` es true.
/// - Se modifica el valor de `active` si `delete_network` es false.
/// Si la red no existe:
/// - Se ingresa al sistema si `delete_network` es false
///
pub async fn run_network_task(tx_to_dba_insert: watch::Sender<NetworkChanged>,
                              mut rx_from_server: broadcast::Receiver<InternalEvent>,
                              app_context: AppContext) {
    loop {
        tokio::select! {
            msg_from_server = rx_from_server.recv() => {
                match msg_from_server {
                    Ok(msg) => {
                        match msg {
                            InternalEvent::IncomingMessage(internal) => {
                                if let Ok(MessageFromServerTypes::Network(network)) = from_slice::<MessageFromServerTypes>(&internal.payload) {
                                    let action = {
                                        let manager = app_context.net_man.read().await;
                                        match (manager.networks.get(&network.id_network), network.delete_network) {
                                            (Some(_), true) => Delete,
                                            (Some(existing), false) if existing.active != network.active => {
                                                Update
                                            }
                                            (None, false) => Insert,
                                            _ => Ignore,
                                        }
                                    };

                                    if let Delete = action {
                                        if let Err(e) = app_context.repo.delete_network(&network.id_network).await {
                                            error!("Error eliminando red {}: {}", network.id_network, e);
                                            continue;
                                        }
                                    }

                                    let mut manager = app_context.net_man.write().await;
                                    let changed = match action {
                                        Delete => {
                                            manager.remove_network(&network.id_network);
                                            true
                                        }
                                        Update => {
                                            manager.change_active(network.active, &network.id_network);
                                            true
                                        }
                                        Insert => {
                                            manager.add_network(Network::new(
                                                network.id_network,
                                                network.name_network,
                                                network.active,
                                                &app_context.system,
                                            ));
                                            true
                                        }
                                        Ignore => false,
                                    };
                                    drop(manager);
                                    if changed {
                                        let _ = tx_to_dba_insert.send(NetworkChanged::Changed);
                                    }
                                }
                            },
                            _ => {},
                        }
                    }
                    Err(e) => {
                        error!("{}", e);
                    }
                }
            }
        }
    }
}