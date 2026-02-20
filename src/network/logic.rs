//! Gestión de la configuración dinámica de redes.
//!
//! Este módulo es responsable de mantener sincronizada la configuración de las redes
//! entre el Servidor (Nube), la Memoria (Runtime) y la Base de Datos (Persistencia).
//!
//! # Flujo de Trabajo
//!
//! 1. **Recepción:** Escucha mensajes MQTT del servidor (`InternalEvent`).
//! 2. **Decisión:** Compara el estado deseado con el estado actual en memoria (`NetworkManager`).
//! 3. **Aplicación:** Actualiza la memoria (Thread-Safe) y notifica los cambios.
//! 4. **Persistencia:** Una tarea dedicada escribe los cambios en SQLite asíncronamente.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use chrono::Utc;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, instrument};
use crate::context::domain::AppContext;
use crate::database::domain::DataServiceCommand;
use crate::database::repository::Repository;
use crate::message::domain::{ActiveHub, DeleteHub, HubMessage, Metadata, Network as NetworkMsg, ServerMessage};
use crate::network::domain::{Batch, HubChanged, HubRow, Network, NetworkAction, NetworkChanged, NetworkManager, NetworkRow};
use crate::network::domain::NetworkAction::{Delete, Ignore, Insert, Update};
use crate::system::domain::{ErrorType};


/// Tarea principal de procesamiento de actualizaciones de red.
///
/// Escucha eventos del servidor (vía MQTT/InternalEvent) y determina qué acción
/// tomar sobre la configuración de las redes (Insertar, Actualizar, Borrar o Ignorar).
///
/// # Lógica de Decisión
///
/// Para evitar bloqueos de escritura innecesarios (`RwLock`), la función realiza
/// una verificación en dos fases:
///
/// 1. **Fase de Lectura:** Obtiene un `read lock` para comparar el mensaje entrante
///    con la configuración actual y decidir la `NetworkAction`.
/// 2. **Fase de Escritura:** Si la acción no es `Ignore`, obtiene un `write lock`
///    para modificar el mapa de redes en memoria.
///
/// # Notificaciones
///
/// Si se aplica un cambio, invoca a [`handle_event`] para:
/// - Notificar a la tarea de base de datos (`network_dba_task`).
/// - Notificar a la tarea de inserción de datos (`dba_insert_task`) para refrescar buffers.

#[instrument(name = "network_admin", skip(app_context))]
pub async fn network_admin(tx_to_insert_network: mpsc::Sender<NetworkChanged>,
                           tx_to_hub: mpsc::Sender<HubMessage>,
                           tx_to_insert_hub: mpsc::Sender<HubChanged>,
                           mut rx_from_server: mpsc::Receiver<ServerMessage>,
                           mut rx_from_hub: mpsc::Receiver<HubMessage>,
                           mut rx_batch: mpsc::Receiver<Batch>,
                           app_context: AppContext) {

    let mut hub_hash_aux : HashMap<String, HashSet<HubRow>> = HashMap::new();
    
    loop {
        tokio::select! {
            Some(msg_from_server) = rx_from_server.recv() => {
                match msg_from_server {
                    ServerMessage::Network(network) => {
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
                        if action == Ignore {
                            continue;
                        }
                        let net_chan = handle_action(app_context.clone(), &network, action).await;
                        handle_event(&tx_to_insert_network, &tx_to_hub, net_chan, app_context.clone()).await;
                    },
                    ServerMessage::FromServerSettings(ref settings) => {
                        let id = settings.network.clone();
                        hub_hash_aux.entry(settings.metadata.sender_user_id.clone())
                            .or_default()
                            .insert(settings.clone().cast_settings_to_hub_row(id));
                        if tx_to_hub.send(HubMessage::FromServerSettings(settings.clone())).await.is_err() {
                            error!("Error: No se pudo enviar el mensaje de nueva configuración al Hub");
                        }
                    },
                    ServerMessage::DeleteHub(ref hub) => {
                        let id = hub.network.clone();
                        let mut manager = app_context.net_man.write().await;
                        manager.remove_hub(&id, &hub.metadata.destination_id);
                        if tx_to_insert_hub.send(HubChanged::Delete(id)).await.is_err() {
                            error!("Error: No se pudo notificar a network_dba_task que debe eliminar un Hub");
                        }
                        if tx_to_hub.send(HubMessage::DeleteHub(hub.clone())).await.is_err() {
                            error!("Error: No se pudo enviar el mensaje de nueva configuración al Hub");
                        }
                    }
                    _ => {},
                }
            }
            
            Some(msg_from_hub) = rx_from_hub.recv() => {
                match msg_from_hub {
                    HubMessage::FromHubSettings(settings) => {
                        let id = settings.network.clone();
                        let mut manager = app_context.net_man.write().await;
                        if !manager.search_hub(&id, &settings.metadata.sender_user_id) {
                            let hub_row = settings.cast_settings_to_hub_row(id.clone());
                            manager.add_hub(id, hub_row.clone().cast_to_hub());
                            drop(manager);
                            if tx_to_insert_hub.send(HubChanged::Insert(hub_row)).await.is_err() {
                                error!("Error: No se pudo notificar a network_dba_task que un nuevo Hub debe insertarse");
                            }
                        }
                    },
                    HubMessage::FromHubSettingsAck(settings_ok) => {
                        let id = settings_ok.network.clone();
                        if let Some(hub_row) = hub_hash_aux.get_mut(&id) {
                            if let Some(row) = hub_row.iter().find(|r| r.metadata.sender_user_id == settings_ok.metadata.sender_user_id) {
                                if tx_to_insert_hub.send(HubChanged::Update(row.clone())).await.is_err() {
                                    error!("Error: No se pudo notificar a network_dba_task que un Hub debe actualizarse");
                                }
                                let mut manager = app_context.net_man.write().await;
                                manager.remove_hub(&id, &row.metadata.sender_user_id);
                                manager.add_hub(id, row.clone().cast_to_hub());
                            }
                        }
                    },
                    _ => {},
                }
            }
            
            Some(batch) = rx_batch.recv() => {
                match batch {
                    Batch::Network(networks) => {
                        let mut manager = app_context.net_man.write().await;
                        for networks_row in networks {
                            let network = networks_row.cast_to_network();
                            manager.add_network(network);
                        }
                        info!("Info: estado cargado, {} redes en sistema", manager.networks.len());
                    }
                    Batch::Hub(hubs) => {
                        let mut manager = app_context.net_man.write().await;
                        for hubs_row in hubs {
                            let net_id = hubs_row.network_id.clone();
                            let hub = hubs_row.cast_to_hub();
                            manager.add_hub(net_id, hub);
                        }
                        info!("Info: estado cargado, {} hubs en sistema", manager.get_total_hubs());
                    }
                }
            }
        }
    }
}


async fn handle_action(app_context: AppContext, network: &NetworkMsg, action: NetworkAction) -> NetworkChanged {
    let mut manager = app_context.net_man.write().await;
    let net_chan : NetworkChanged = match action {
        Delete => {
            manager.remove_network(&network.id_network);
            NetworkChanged::Delete { id: network.id_network.clone() }
        }
        Update => {
            manager.change_active(network.active, &network.id_network);
            let net = NetworkRow::new(network.id_network.clone(), network.name_network.clone(), network.active);
            NetworkChanged::Update(net)
        }
        Insert => {
            manager.add_network( Network::new(
                network.id_network.clone(),
                network.name_network.clone(),
                network.active
            ));
            let net = NetworkRow::new(network.id_network.clone(), network.name_network.clone(), network.active);
            NetworkChanged::Insert(net)
        }
        _ => unreachable!(),
    };
    net_chan
}


/// Función auxiliar para distribuir la notificación de cambio de red.
///
/// Envía el evento a tres destinos:
/// 1. `tx_to_insert_network`: Canal hacia [`network_dba_task`] para persistencia en DB.
/// 2. `tx_to_dba_insert`: Canal hacia la tarea de buffers (`dba_insert_task`) para
///    que actualice sus vectores en memoria.
/// 3. `tx_to_hub`: Canal hacia la tarea (`msg_to_hub`) para que envíe el mensaje
/// de cambio de actividad de red a los Hubs.

async fn handle_event(tx_to_insert_network: &mpsc::Sender<NetworkChanged>,
                      tx_to_hub: &mpsc::Sender<HubMessage>,
                      net_chan: NetworkChanged,
                      app_context: AppContext) {

    match net_chan.clone() {
        NetworkChanged::Delete { id } => {
            let metadata = create_metadata(app_context.clone());
            let delete_hub = DeleteHub {
                metadata,
                network: id,
            };
            if tx_to_hub.send(HubMessage::DeleteHub(delete_hub)).await.is_err() {
                error!("Error: No se pudo enviar mensaje de eliminación de red a los hubs");
            }
        },
        NetworkChanged::Update(network) => {
            let metadata = create_metadata(app_context.clone());
            let active_hub = ActiveHub {
                metadata,
                network: network.id_network,
                active: network.active,
            };
            if tx_to_hub.send(HubMessage::ActiveHub(active_hub)).await.is_err() {
                error!("Error: No se pudo enviar mensaje de activación/desactivación de red a los hubs");
            }
        },
        _ => {},
    }

    if tx_to_insert_network.send(net_chan).await.is_err() {
        error!("Error: No se pudo enviar NetworkChanged");
    }
}


fn create_metadata(app_context: AppContext) -> Metadata {
    let timestamp = Utc::now().timestamp();

    let metadata = Metadata {
        sender_user_id: app_context.system.id_edge.clone(),
        destination_id: "all".to_string(),
        timestamp,
    };
    metadata
}


/// Tarea de persistencia de configuración de red.
///
/// Actúa como consumidor de los cambios generados por [`run_network_task`].
/// Su única responsabilidad es reflejar los cambios de memoria en la base de datos SQLite.
///
/// # Acciones
///
/// - **Delete:** Elimina la fila correspondiente en la tabla `network` y los `hubs` asociados.
/// - **Update:** Actualiza el campo `active` en la tabla `network`.
/// - **Insert:** Crea un nuevo registro en la tabla `network`.

#[instrument(name = "network_dba", skip(rx_from_network, rx_from_network_hub))]
pub async fn network_dba(tx: mpsc::Sender<DataServiceCommand>,
                         mut rx_from_network: mpsc::Receiver<NetworkChanged>,
                         mut rx_from_network_hub: mpsc::Receiver<HubChanged>) {

    loop {
        tokio::select! {
            Some(msg_from_network) = rx_from_network.recv() => {
                match msg_from_network {
                    NetworkChanged::Delete { id} => {
                        if tx.send(DataServiceCommand::DeleteNetwork(id.clone())).await.is_err() {
                            error!("Error: no se pudo enviar comando DeleteNetwork");
                        }
                        if tx.send(DataServiceCommand::DeleteAllHubByNetwork(id)).await.is_err() {
                            error!("Error: no se pudo enviar comando DeleteNetwork");
                        }
                    },
                    NetworkChanged::Update(network) => {
                        if tx.send(DataServiceCommand::UpdateNetwork(network)).await.is_err() {
                            error!("Error: no se pudo enviar comando UpdateNetwork");
                        }
                    },
                    NetworkChanged::Insert(network) => {
                        if tx.send(DataServiceCommand::NewNetwork(network)).await.is_err() {
                            error!("Error: no se pudo enviar comando NewNetwork");
                        }
                    },
                }
            }

            Some(msg_hub_changed) = rx_from_network_hub.recv() => {
                match msg_hub_changed {
                    HubChanged::Insert(hub) => {
                        if tx.send(DataServiceCommand::NewHub(hub)).await.is_err() {
                            error!("Error: no se pudo enviar comando NewHub");
                        }
                    },
                    HubChanged::Delete(id) => {
                        if tx.send(DataServiceCommand::DeleteHub(id)).await.is_err() {
                            error!("Error: no se pudo enviar comando DeleteHub");
                        }
                    },
                    HubChanged::Update(hub) => {
                        if tx.send(DataServiceCommand::UpdateHub(hub)).await.is_err() {
                            error!("Error: no se pudo enviar comando UpdateHub");
                        }
                    }
                }
            }
        }
    }
}