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
use chrono::Local;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, instrument};
use crate::context::domain::AppContext;
use crate::database::repository::Repository;
use crate::message::domain::{ActiveHub, DeleteHub, DestinationType, MessageFromHub, MessageFromHubTypes, MessageFromServer, MessageFromServerTypes, MessageToHub, MessageToHubTypes, Metadata};
use crate::message::domain::MessageToHub::ToHub;
use crate::message::domain::MessageToHubTypes::ServerToHub;
use crate::message::domain_for_table::HubRow;
use crate::network::domain::{HubChanged, Network, NetworkChanged, NetworkManager, NetworkRow, UpdateNetwork};
use crate::network::domain::NetworkAction::{Delete, Ignore, Insert, Update};
use crate::system::domain::{ErrorType, System};


/// Carga las redes y los hubs desde la base de datos, al Network Manager.
pub async fn load_networks(repo: &Repository,
                           net_man: &Arc<RwLock<NetworkManager>>,
                           system: &System) -> Result<(), ErrorType> {

    let hubs_row_vec = repo.get_all_hubs().await?;
    let networks_row_vec = repo.get_all_network().await?;
    let mut manager = net_man.write().await;

    for networks_row in networks_row_vec {
        let network = networks_row.cast_to_network(system);
        manager.add_network(network);
    }

    for hubs_row in hubs_row_vec {
        let net_id = hubs_row.network_id.clone();
        let hub = hubs_row.cast_to_hub();
        manager.add_hub(net_id, hub);
    }

    info!("Estado cargado: {} redes procesadas.", manager.networks.len());
    Ok(())
}


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

#[instrument(name = "network_task", skip(app_context))]
pub async fn network_task(tx_to_insert_network: mpsc::Sender<NetworkChanged>,
                          tx_to_dba_insert: mpsc::Sender<UpdateNetwork>,
                          tx_to_hub: mpsc::Sender<MessageToHub>,
                          tx_to_insert_hub: mpsc::Sender<HubChanged>,
                          mut rx_from_server: mpsc::Receiver<MessageFromServer>,
                          mut rx_from_hub: mpsc::Receiver<MessageFromHub>,
                          app_context: AppContext) {

    let mut hub_hash_aux : HashMap<String, HashSet<HubRow>> = HashMap::new();

    loop {
        tokio::select! {
            Some(msg_from_server) = rx_from_server.recv() => {
                match msg_from_server.msg {
                    MessageFromServerTypes::Network(network) => {
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

                        let mut manager = app_context.net_man.write().await;
                        let net_chan : NetworkChanged = match action {
                            Delete => {
                                manager.remove_network(&network.id_network);
                                NetworkChanged::Delete { id: network.id_network }
                            }
                            Update => {
                                manager.change_active(network.active, &network.id_network);
                                let net = NetworkRow::new(network.id_network, network.name_network, network.active);
                                NetworkChanged::Update(net)
                            }
                            Insert => {
                                manager.add_network(Network::new(
                                    network.id_network.clone(),
                                    network.name_network.clone(),
                                    network.active,
                                    &app_context.system,
                                ));
                                let net = NetworkRow::new(network.id_network, network.name_network, network.active);
                                NetworkChanged::Insert(net)
                            }
                            _ => unreachable!(),
                        };
                        drop(manager);
                        handle_event(&tx_to_insert_network, &tx_to_dba_insert, &tx_to_hub, net_chan, app_context.clone(), &msg_from_server.topic_where_arrive).await;
                    },
                    MessageFromServerTypes::Settings(settings) => {
                        let manager = app_context.net_man.read().await;
                        if let Some(network_id) = manager.extract_net_id(&msg_from_server.topic_where_arrive) {
                            hub_hash_aux.entry(settings.metadata.sender_user_id.clone())
                            .or_default()
                            .insert(settings.clone().cast_settings_to_hub_row(network_id, msg_from_server.topic_where_arrive));
                        }
                        if tx_to_hub.send(ToHub(MessageToHubTypes::Settings(settings))).await.is_err() {
                            error!("Error: No se pudo enviar el mensaje de nueva configuración al Hub");
                        }
                    },
                    MessageFromServerTypes::DeleteHub(hub) => {
                        let mut manager = app_context.net_man.write().await;
                        if let Some(id) = manager.extract_net_id(&msg_from_server.topic_where_arrive) {
                            manager.remove_hub(&id, &hub.id_hub);
                            if tx_to_insert_hub.send(HubChanged::Delete(id)).await.is_err() {
                                error!("Error: No se pudo notificar a network_dba_task que debe eliminar un Hub");
                            }
                            let msg = MessageFromServer::new(msg_from_server.topic_where_arrive, MessageFromServerTypes::DeleteHub(hub));
                            if tx_to_hub.send(ToHub(ServerToHub(msg))).await.is_err() {
                                error!("Error: No se pudo enviar el mensaje de nueva configuración al Hub");
                            }
                        }
                    }
                    _ => {},
                }
            }
            
            Some(msg_from_hub) = rx_from_hub.recv() => {
                match msg_from_hub.msg {
                    MessageFromHubTypes::Settings(settings) => {
                        let mut manager = app_context.net_man.write().await;
                        if let Some(id) = manager.extract_net_id(&msg_from_hub.topic_where_arrive) {
                            if !manager.search_hub(&id, &settings.metadata.sender_user_id) {
                                let hub_row = settings.cast_settings_to_hub_row(id.clone(), msg_from_hub.topic_where_arrive.clone());
                                manager.add_hub(id, hub_row.clone().cast_to_hub());
                                drop(manager);
                                if tx_to_insert_hub.send(HubChanged::Insert(hub_row)).await.is_err() {
                                    error!("Error: No se pudo notificar a network_dba_task que un nuevo Hub debe insertarse");
                                }
                            }
                        }
                    },
                    MessageFromHubTypes::SettingsOk(settings_ok) => {
                        let manager = app_context.net_man.read().await;
                        if let Some(id) = manager.extract_net_id(&msg_from_hub.topic_where_arrive) {
                            drop(manager);
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
                        }
                    },
                    _ => {},
                }
            }
        }
    }
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

#[instrument(name = "network_dba_task", skip(app_context))]
pub async fn network_dba_task(mut rx_from_network: mpsc::Receiver<NetworkChanged>,
                              mut rx_from_network_hub: mpsc::Receiver<HubChanged>,
                              app_context: AppContext) {
    loop {
        tokio::select! {
            Some(msg_from_network) = rx_from_network.recv() => {
                match msg_from_network {
                    NetworkChanged::Delete { id} => {
                        match app_context.repo.delete_network(&id).await {
                            Ok(_) => {},
                            Err(e) => error!("Error eliminando red en base de datos {}", e)
                        }
                        match app_context.repo.delete_hub_network(&id).await {
                            Ok(_) => {},
                            Err(e) => error!("Error eliminando hubs de la base de datos asociadas a la red: {}. {}", id, e)
                        }
                    },
                    NetworkChanged::Update(network) => {
                        match app_context.repo.update_network(network).await {
                            Ok(_) => {},
                            Err(e) => error!("Error actualizando red en base de datos {}", e)
                        }
                    },
                    NetworkChanged::Insert(network) => {
                        match app_context.repo.insert_network(network).await {
                            Ok(_) => {},
                            Err(e) => error!("Error eliminando red en base insert {}", e)
                        }
                    },
                }
            }

            Some(msg_hub_changed) = rx_from_network_hub.recv() => {
                match msg_hub_changed {
                    HubChanged::Insert(hub_row) => {
                        match app_context.repo.insert_hub(hub_row).await {
                            Ok(_) => {},
                            Err(e) => { error!("Error insertando hub en la base de datos {}", e); }
                        }
                    },
                    HubChanged::Delete(id) => {
                        match app_context.repo.delete_hub(&id).await {
                            Ok(_) => {},
                            Err(e) => error!("Error eliminando hub la base de datos {}", e),
                        }
                    },
                    HubChanged::Update(hub_row) => {
                        match app_context.repo.update_hub(hub_row).await {
                            Ok(_) => (),
                            Err(e) => error!("Error: No se pudo actualizar hub en la base de datos: {}", e),
                        }
                    }
                }
            }
        }
    }
}


/// Función auxiliar para distribuir la notificación de cambio de red.
///
/// Envía el evento a tres destinos:
/// 1. `tx_to_insert_network`: Canal hacia [`network_dba_task`] para persistencia en DB.
/// 2. `tx_to_dba_insert`: Canal hacia la tarea de buffers (`dba_insert_task`) para
///    que actualice sus vectores en memoria.
/// 3. `tx_to_hub`: Canal hacia la tarea (`msg_to_hub`) para que envíe el mensaje
/// de cambio de actividad de red a los Hubs.

#[instrument(name = "handle_event")]
async fn handle_event(tx_to_insert_network: &mpsc::Sender<NetworkChanged>,
                      tx_to_dba_insert: &mpsc::Sender<UpdateNetwork>,
                      tx_to_hub: &mpsc::Sender<MessageToHub>,
                      net_chan: NetworkChanged,
                      app_context: AppContext,
                      topic: &str) {

    match net_chan.clone() {
        NetworkChanged::Delete { id } => {
            let now = Local::now();
            let timestamp = now.format("%d/%m/%Y %H:%M:%S").to_string();
            let metadata = Metadata {
                sender_user_id: app_context.system.id_edge.clone(),
                destination_type: DestinationType::Node,
                destination_id: "all".to_string(),
                timestamp,
            };
            let msg_types = MessageFromServerTypes::DeleteHub(DeleteHub::new(metadata, id));
            let msg = MessageFromServer::new(topic.to_string(), msg_types);
            if tx_to_hub.send(ToHub(ServerToHub(msg))).await.is_err() {
                error!("No se pudo enviar mensaje de desactivación de red");
            }
        },
        NetworkChanged::Update(network) => {
            let now = Local::now();
            let timestamp = now.format("%d/%m/%Y %H:%M:%S").to_string();
            let metadata = Metadata {
                sender_user_id: app_context.system.id_edge.clone(),
                destination_type: DestinationType::Node,
                destination_id: "all".to_string(),
                timestamp,
            };
            let msg_types = MessageFromServerTypes::ActiveHub(ActiveHub::new(metadata, network.active));
            let msg = MessageFromServer::new(topic.to_string(), msg_types);
            if tx_to_hub.send(ToHub(ServerToHub(msg))).await.is_err() {
                error!("No se pudo enviar mensaje de activación/desactivación de red");
            }
        },
        _ => {},
    }

    if tx_to_insert_network.send(net_chan).await.is_err() {
        error!("No se pudo enviar NetworkChanged");
    }

    if tx_to_dba_insert.send(UpdateNetwork::Changed).await.is_err() {
        error!("No se pudo enviar UpdateNetwork::Changed");
    }
}