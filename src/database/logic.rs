//! Módulo de orquestación de base de datos y persistencia.
//!
//! Este módulo implementa el patrón **"Store and Forward"** (Almacenar y Reenviar)
//! para garantizar la integridad de los datos cuando se pierde la conexión con el servidor exterior,
//! y gestiona las operaciones CRUD de la configuración local.
//!
//! # Arquitectura
//!
//! El sistema utiliza un modelo de actores basado en tareas asíncronas de Tokio, las cuales
//! se comunican mediante canales `mpsc` y comparten acceso a la base de datos a través de `Repository`.
//! Se divide en tres tareas principales:
//!
//! 1.  **[`dba_insert_task`]:** El consumidor de escritura. Agrupa los datos de telemetría en memoria
//!     y realiza inserciones por lotes (*Batch Insert*). También maneja la inserción/actualización de configuración.
//! 2.  **[`dba_remove_task`]:** El consumidor de borrado. Gestiona la eliminación de entidades (Redes, Hubs)
//!     y notifica al sistema sobre los cambios.
//! 3.  **[`dba_get_task`]:** El productor de lectura y retransmisor. Extrae datos almacenados
//!     y los inyecta en el flujo de envío al servidor cuando se recupera la conexión. También sirve consultas de estado.
//!
//! # Apagado Seguro (Graceful Shutdown)
//!
//! Todas las tareas monitorean un `CancellationToken` para interrumpir sus bucles principales
//! y finalizar de manera limpia cuando el sistema se apaga.


use tokio::sync::{mpsc};
use tokio::time::{interval, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument};
use crate::config::sqlite::{FLUSH_INTERVAL};
use crate::message::domain::{HubMessage, ServerStatus};
use super::domain::{DataCommandDelete, DataCommandGet, DataCommandInsert, DataServiceResponse, TableDataVector};
use crate::database::repository::Repository;
use crate::system::domain::InternalEvent;


/// Tarea de persistencia y buffering de datos de escritura.
///
/// Recibe comandos a través de `rx_cmd` y maneja dos flujos de trabajo principales:
/// 1. **Datos de configuración:** Inserción y actualización inmediata de Redes, Hubs y Epoch.
/// 2. **Datos de telemetría (`HubMessage`):** Los acumula en buffers de memoria organizados
///    en un [`TableDataVector`] para optimizar las escrituras en disco.
///
/// # Estrategia de Escritura de Telemetría (Batching)
///
/// Para evitar bloquear la base de datos con escrituras constantes, los mensajes se insertan
/// en lote cuando ocurre **una de dos condiciones**:
/// - **Capacidad:** Alguno de los vectores internos del `TableDataVector` se llena (`is_some_vector_full`).
/// - **Tiempo:** El temporizador definido por [`FLUSH_INTERVAL`] expira, evitando que los datos se queden estancados en memoria.
#[instrument(name = "dba_insert_task", skip(repo))]
pub async fn dba_insert_task(tx_to_core: mpsc::Sender<DataServiceResponse>,
                             mut rx_cmd: mpsc::Receiver<DataCommandInsert>,
                             repo: Repository,
                             shutdown: CancellationToken) {

    info!("Info: iniciando tarea dba_insert_task");

    let mut tdv = TableDataVector::new();
    let mut timer = interval(FLUSH_INTERVAL);
    timer.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Info: shutdown recibido dba_insert_task");
                break;
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
                    DataCommandInsert::InsertHubMessage(message) => {
                        debug!("Debug: mensaje HubMessage entrante a dba_insert_task");
                        sort_by_vectors(message, &mut tdv);
                        if tdv.is_some_vector_full() {
                            repo.insert(&tdv).await.ok();
                            tdv.clear();
                        }
                    },
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
                            Ok(_) => {
                                if tx_to_core.send(DataServiceResponse::NetworksUpdated).await.is_err() {
                                    error!("Error: no se pudo enviar NetworksUpdated desde dba_insert_task");
                                }
                            }
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
                            Ok(_) => {
                                if tx_to_core.send(DataServiceResponse::NetworksUpdated).await.is_err() {
                                    error!("Error: no se pudo enviar NetworksUpdated desde dba_insert_task");
                                }
                            }
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


/// Clasifica un mensaje de telemetría entrante y lo apila en el vector correspondiente.
///
/// Dependiendo de la variante del enum `HubMessage`, el dato se enruta a la tabla
/// lógica pertinente dentro del buffer `TableDataVector`.
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


/// Tarea encargada de la eliminación de datos persistentes.
///
/// Escucha comandos de tipo [`DataCommandDelete`] para borrar Redes o Hubs específicos.
/// Tras una eliminación, evalúa si el sistema se quedó sin redes configuradas para notificar
/// al Core mediante el evento `NoNetworks`.
pub async fn dba_remove_task(tx: mpsc::Sender<DataServiceResponse>,
                             mut rx_cmd: mpsc::Receiver<DataCommandDelete>,
                             repo: Repository,
                             shutdown: CancellationToken) {

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Info: shutdown recibido dba_remove_task");
                break;
            }
            
            Some(msg) = rx_cmd.recv() => {
                match msg {
                    DataCommandDelete::DeleteNetwork(id) => {
                        match repo.delete_network(&id).await {
                            Ok(_) => {
                                if tx.send(DataServiceResponse::NetworksUpdated).await.is_err() {
                                    error!("Error: no se pudo enviar NetworksUpdated desde dba_remove_task");
                                }
                            }
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
    }
}


// -------------------------------------------------------------------------------------------------


/// Tarea de recuperación de datos (El "Forward" del patrón Store-and-Forward) y lectura general.
///
/// Tiene dos responsabilidades principales:
/// 1. **Atender consultas (Read):** Escucha `DataCommandGet` para devolver estados actuales
///    (Epoch, lista de Redes, lista de Hubs).
/// 2. **Retransmitir telemetría atrasada:** Monitorea el estado de la conexión mediante `InternalEvent`.
///    Al detectar una transición de `Disconnected` a `Connected`, dispara el vaciado de los
///    datos almacenados en SQLite hacia el servidor exterior.
#[instrument(name = "dba_get_task", skip(rx_from_core, repo))]
pub async fn dba_get_task(tx_to_core: mpsc::Sender<DataServiceResponse>,
                          mut rx_from_core: mpsc::Receiver<InternalEvent>,
                          mut rx_from: mpsc::Receiver<DataCommandGet>,
                          repo: Repository,
                          shutdown: CancellationToken) {

    info!("Info: iniciando tarea dba_get_task");

    let mut state : ServerStatus = ServerStatus::Disconnected;
    let mut old_state : ServerStatus = ServerStatus::Disconnected;

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Info: shutdown recibido dba_get_task");
                break;
            }
            
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


/// Helper para extraer iterativamente los datos persistidos y enviarlos al Core.
///
/// Realiza un bucle llamando a `repo.pop_batch()` que elimina y retorna un bloque de datos de la base de datos.
/// El proceso se detiene cuando `pop_batch` retorna un bloque vacío.
///
/// **Nota de Control de Flujo:** Se aplica un `sleep` de 100ms entre cada envío para evitar
/// saturar el canal `tx` y darle respiro al Core para procesar los batches enviados.
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