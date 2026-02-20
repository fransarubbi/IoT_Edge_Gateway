//! Módulo de orquestación de base de datos y persistencia.
//!
//! Este módulo implementa el patrón **"Store and Forward"** (Almacenar y Reenviar)
//! para garantizar la integridad de los datos cuando se pierde la conexión con el servidor.
//!
//! # Arquitectura
//!
//! El sistema se divide en dos tareas asíncronas principales que se comunican mediante canales:
//!
//! 2.  **[`dba_insert_task`]:** El consumidor de escritura. Agrupa los datos en memoria (Buffers)
//!     y realiza inserciones por lotes (*Batch Insert*) en SQLite para maximizar el rendimiento.
//! 3.  **[`dba_get_task`]:** El productor de lectura. Extrae datos de la base de datos y los
//!     inyecta de nuevo en el flujo de envío cuando se recupera la conexión.
//!
//! # Concurrencia
//!
//! Se utiliza [`AppContext`] para compartir el acceso seguro
//! al repositorio y a la configuración de red (`NetworkManager`) mediante `Arc<RwLock<...>>`.


use tokio::sync::{mpsc};
use tokio::time::{interval, MissedTickBehavior};
use tracing::{debug, error, info, instrument};
use crate::config::sqlite::{FLUSH_INTERVAL};
use crate::message::domain::{HubMessage, ServerStatus};
use super::domain::{DataCommandDelete, DataCommandGet, DataCommandInsert, DataServiceResponse, TableDataVector};
use crate::database::repository::Repository;
use crate::system::domain::InternalEvent;


/// Tarea de persistencia y buffering.
///
/// Recibe mensajes individuales y los acumula en buffers de memoria ([`Vectors`]) organizados
/// por red (`network_id`).
///
/// # Estrategia de Escritura
///
/// Los datos se escriben en disco (SQLite) cuando ocurre una de dos condiciones:
/// 1.  **Capacidad:** Un buffer alcanza [`BATCH_SIZE`] elementos.
/// 2.  **Tiempo:** El temporizador [`FLUSH_INTERVAL`] expira (evita datos estancados).
///
/// # Gestión de Configuración
///
/// Escucha cambios en `NetworkManager`. Si la configuración de redes cambia,
/// sincroniza los buffers locales (crea nuevos para redes nuevas, elimina los de redes borradas).

#[instrument(name = "dba_insert_task", skip(repo))]
pub async fn dba_insert_task(tx_to_core: mpsc::Sender<DataServiceResponse>,
                             mut rx_msg: mpsc::Receiver<HubMessage>,
                             mut rx_cmd: mpsc::Receiver<DataCommandInsert>,
                             repo: Repository) {

    info!("Info: iniciando tarea dba_insert_task");

    let mut tdv = TableDataVector::new();
    let mut timer = interval(FLUSH_INTERVAL);
    timer.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            Some(msg_from_dba) = rx_msg.recv() => {
                debug!("Debug: mensaje HubMessage entrante a dba_insert_task");
                sort_by_vectors(msg_from_dba, &mut tdv);
                if tdv.is_some_vector_full() {
                    repo.insert(&tdv).await.ok();
                    tdv.clear();
                }
            }

            _ = timer.tick() => {
                debug!("Debug: se acabo el tiempo de batch en dba_insert_task");
                if !tdv.is_empty() {
                    repo.insert(&tdv).await.ok();
                    tdv.clear();
                }
            }

            Some(command) = rx_cmd.recv() => {
                match command {
                    DataCommandInsert::InsertNetwork(network) => {
                        match repo.get_number_of_networks().await {
                            Ok(networks) => {
                                if networks == 0 {
                                    if tx_to_core.send(DataServiceResponse::ThereAreNetworks).await.is_err() {
                                        error!("Error: no se pudo enviar ThereAreNetworks desde dba_insert_task");
                                    }
                                } 
                            }
                            Err(e) => error!("Error: no se pudo obtener el total de redes presentes en el sistema. {e}"),
                        }
                        match repo.insert_network(network).await {
                            Ok(_) => {}
                            Err(e) => error!("Error: no se pudo insertar NetworkRow en base de datos. {e}"),
                        }
                    },
                    DataCommandInsert::UpdateNetwork(network) => {
                        match repo.get_number_of_networks().await {
                            Ok(networks) => {
                                if networks == 0 {
                                    if tx_to_core.send(DataServiceResponse::ThereAreNetworks).await.is_err() {
                                        error!("Error: no se pudo enviar ThereAreNetworks desde dba_insert_task");
                                    }
                                } 
                            }
                            Err(e) => error!("Error: no se pudo obtener el total de redes presentes en el sistema. {e}"),
                        }
                        match repo.update_network(network).await {
                            Ok(_) => {}
                            Err(e) => error!("Error: no se pudo actualizar NetworkRow en base de datos. {e}"),
                        }
                    },
                    DataCommandInsert::NewEpoch(epoch) => {
                        match repo.update_epoch(epoch).await {
                            Ok(_) => {}
                            Err(e) => error!("Error: no se pudo insertar nuevo Epoch en base de datos. {e}"),
                        }
                    },
                    DataCommandInsert::InsertHub(hub) => {
                        match repo.insert_hub(hub).await {
                            Ok(_) => {}
                            Err(e) => error!("Error: no se pudo insertar un nuevo Hub en base de datos. {e}"),
                        }
                    },
                    DataCommandInsert::UpdateHub(hub) => {
                        match repo.update_hub(hub).await {
                            Ok(_) => {}
                            Err(e) => error!("Error: no se pudo actualizar Hub en base de datos. {e}"),
                        }
                    }
                }
            }
        }
    }
}


/// Clasifica un mensaje entrante y lo inserta en el vector correspondiente en memoria.
///
/// Identifica la red y el tipo de tabla basándose en la red.
fn sort_by_vectors(msg: HubMessage, tdv: &mut TableDataVector) {

    match msg {
        HubMessage::Report(report) => {
            tdv.measurement.push(report);
        },
        HubMessage::Monitor(monitor) => {
            tdv.monitor.push(monitor);
        },
        HubMessage::AlertAir(alert_air) => {
            tdv.alert_air.push(alert_air);
        },
        HubMessage::AlertTem(alert_tem) => {
            tdv.alert_th.push(alert_tem);
        },
        _ => {},
    }
}


// -------------------------------------------------------------------------------------------------


pub async fn dba_remove_task(tx: mpsc::Sender<DataServiceResponse>,
                             mut rx_cmd: mpsc::Receiver<DataCommandDelete>,
                             repo: Repository) {

    while let Some(msg) = rx_cmd.recv().await {
        match msg {
            DataCommandDelete::DeleteNetwork(id) => {
                match repo.delete_network(&id).await {
                    Ok(_) => {}
                    Err(e) => error!("Error: no se pudo eliminar red con id: {id}. {e}"),
                }
                match repo.get_number_of_networks().await {
                    Ok(networks) => {
                        if networks == 0 {
                            if tx.send(DataServiceResponse::NoNetworks).await.is_err() {
                                error!("Error: no se pudo enviar NoNetworks desde dba_remove_task");
                            }
                        }
                    }
                    Err(e) => error!("Error: no se pudo obtener el total de redes presentes en el sistema. {e}"),
                }
            },
            DataCommandDelete::DeleteAllHubByNetwork(id) => {
                match repo.delete_hub_network(&id).await {
                    Ok(_) => {}
                    Err(e) => error!("Error: no se pudo eliminar todos los hub de la red con id: {id}. {e}"),
                }
            },
            DataCommandDelete::DeleteHub(id) => {
                match repo.delete_hub(&id).await {
                    Ok(_) => {}
                    Err(e) => error!("Error: no se pudo eliminar hub con id: {id}. {e}"),
                }
            },
        }
    }
}


// -------------------------------------------------------------------------------------------------


/// Tarea de recuperación de datos.
///
/// Extrae datos de la base de datos para ser enviados al servidor.
/// Funciona bajo demanda controlada por el estado del servidor.
///
/// # Funcionamiento
///
/// 1. Espera la señal [`InternalEvent::ServerConnected`].
/// 2. Itera sobre cada tabla, extrayendo lotes (`pop_batch`) y enviándolos.

#[instrument(name = "dba_get_task", skip(rx_from_core, repo))]
pub async fn dba_get_task(tx_to_core: mpsc::Sender<DataServiceResponse>,
                          mut rx_from_core: mpsc::Receiver<InternalEvent>,
                          mut rx_from: mpsc::Receiver<DataCommandGet>,
                          repo: Repository) {

    info!("Info: iniciando tarea dba_get_task");

    let mut state : ServerStatus = ServerStatus::Disconnected;
    let mut old_state : ServerStatus = ServerStatus::Disconnected;

    loop {
        tokio::select! {
            Some(internal) = rx_from_core.recv() => {
                match internal {
                    InternalEvent::ServerConnected => {
                        debug!("Debug: server connected entrante en dba_get_task");
                        state = ServerStatus::Connected;
                    },
                    InternalEvent::ServerDisconnected => {
                        debug!("Debug: server disconnected entrante en dba_get_task");
                        state = ServerStatus::Disconnected;
                        old_state = ServerStatus::Disconnected;
                    }
                    _ => {}
                }

                if state == ServerStatus::Connected && old_state == ServerStatus::Disconnected {
                    debug!("Debug: obtener batch en dba_get_task");
                    old_state = state;
                    get_all_tables(&repo, &tx_to_core).await;
                }
            }

            Some(cmd) = rx_from.recv() => {
                match cmd {
                    DataCommandGet::GetEpoch => {
                        match repo.get_epoch().await {
                            Ok(epoch) => {
                                if tx_to_core.send(DataServiceResponse::Epoch(epoch)).await.is_err() {
                                    error!("Error: no se pudo enviar Epoch a DataService desde dba_get_task");
                                }
                            }
                            Err(_) => {
                                if tx_to_core.send(DataServiceResponse::ErrorEpoch).await.is_err() {
                                    error!("Error: no se pudo enviar ErrorEpoch a DataService desde dba_get_task");
                                }
                            }
                        }
                    },
                    DataCommandGet::GetTotalOfNetworks => {
                        match repo.get_number_of_networks().await {
                            Ok(networks) => {
                                if networks > 0 {
                                    if tx_to_core.send(DataServiceResponse::ThereAreNetworks).await.is_err() {
                                        error!("Error: no se pudo enviar ThereAreNetworks desde dba_get_task");
                                    }
                                } 
                            }
                            Err(e) => error!("Error: no se pudo obtener el total de redes presentes en el sistema. {e}"),
                        }
                        match repo.get_all_network().await {
                            Ok(networks) => {
                                if tx_to_core.send(DataServiceResponse::BatchNetwork(networks)).await.is_err() {
                                    error!("Error: no se pudo enviar BatchNetwork desde dba_get_task");
                                }
                            }
                            Err(e) => error!("Error: no se pudo obtener todas las redes de la base de datos. {e}"),
                        }
                        match repo.get_all_hubs().await {
                            Ok(hubs) => {
                                if tx_to_core.send(DataServiceResponse::BatchHub(hubs)).await.is_err() {
                                    error!("Error: no se pudo enviar BatchHub desde dba_get_task");
                                }
                            }
                            Err(e) => error!("Error: no se pudo obtener todos los hubs de la base de datos. {e}"),
                        }
                    }
                }
            }
        }
    }
}


/// Itera sobre todas las tablas y envia batch de datos al Core
async fn get_all_tables(repo: &Repository, tx: &mpsc::Sender<DataServiceResponse>) {
    loop {
        match repo.pop_batch().await {
            Ok(tdv) => {
                if tdv.is_empty() {
                    break;
                }
                if tx.send(DataServiceResponse::Batch(tdv)).await.is_err() {
                    error!("Error: canal cerrado, no se pudo enviar pop batch");
                    break;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            },
            Err(e) => {
                error!("Error: no se pudo hacer pop batch. {}", e);
                break;
            }
        }
    }
}