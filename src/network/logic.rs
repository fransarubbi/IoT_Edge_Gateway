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


use std::sync::Arc;
use rmp_serde::from_slice;
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{error, instrument};
use crate::context::domain::AppContext;
use crate::database::repository::Repository;
use crate::fsm::domain::InternalEvent;
use crate::message::domain::{MessageFromServerTypes};
use crate::network::domain::{Network, NetworkChanged, NetworkManager, NetworkRow, UpdateNetwork};
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

#[instrument(name = "run_network_task", skip(app_context))]
pub async fn run_network_task(tx_to_insert_network: mpsc::Sender<NetworkChanged>,
                              tx_to_dba_insert: mpsc::Sender<UpdateNetwork>,
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
                                    handle_event(&tx_to_insert_network, &tx_to_dba_insert, net_chan).await;
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


/// Tarea de persistencia de configuración de red.
///
/// Actúa como consumidor de los cambios generados por [`run_network_task`].
/// Su única responsabilidad es reflejar los cambios de memoria en la base de datos SQLite.
///
/// # Acciones
///
/// - **Delete:** Elimina la fila correspondiente en la tabla `network`.
/// - **Update:** Actualiza el campo `active` en la tabla `network`.
/// - **Insert:** Crea un nuevo registro en la tabla `network`.

#[instrument(name = "network_dba_task", skip(app_context))]
pub async fn network_dba_task(mut rx_from_network: mpsc::Receiver<NetworkChanged>,
                              app_context: AppContext) {
    loop {
        tokio::select! {
            Some(msg_from_network) = rx_from_network.recv() => {
                match msg_from_network {
                    NetworkChanged::Delete { id} => {
                        match app_context.repo.delete_network(&id).await {
                            Ok(_) => {},
                            Err(e) => { error!("Error eliminando red en base de datos {}", e); }
                        }
                    },
                    NetworkChanged::Update(network) => {
                        match app_context.repo.update_network(network).await {
                            Ok(_) => {},
                            Err(e) => { error!("Error actualizando red en base de datos {}", e); }
                        }
                    },
                    NetworkChanged::Insert(network) => {
                        match app_context.repo.insert_network(network).await {
                            Ok(_) => {},
                            Err(e) => { error!("Error eliminando red en base insert {}", e); }
                        }
                    },
                }
            }
        }
    }
}


/// Función auxiliar para distribuir la notificación de cambio de red.
///
/// Envía el evento a dos destinos:
/// 1. `tx_to_insert_network`: Canal hacia [`network_dba_task`] para persistencia en DB.
/// 2. `tx_to_dba_insert`: Canal hacia la tarea de buffers (`dba_insert_task`) para
///    que actualice sus vectores en memoria.

#[instrument(name = "handle_event")]
async fn handle_event(tx_to_insert_network: &mpsc::Sender<NetworkChanged>,
                      tx_to_dba_insert: &mpsc::Sender<UpdateNetwork>,
                      net_chan: NetworkChanged) {

    if tx_to_insert_network.send(net_chan).await.is_err() {
        error!("No se pudo enviar NetworkChanged");
    }

    let _ = tx_to_dba_insert.send(UpdateNetwork::Changed).await;
}