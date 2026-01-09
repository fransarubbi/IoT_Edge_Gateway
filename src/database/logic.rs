use std::collections::HashMap;
use std::mem;
use tokio::sync::{mpsc, watch};
use tokio::time::{interval, MissedTickBehavior};
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::config::sqlite::{BATCH_SIZE, FLUSH_INTERVAL};
use crate::message::domain::{DataRequest, MessageFromHub, MessageFromHubTypes, MessageToServer, NetworkChanged, ServerStatus};
use super::domain::{StateFlag, Table, TableDataVector, TableDataVectorTypes, Vectors};
use crate::database::repository::Repository;
use crate::network::domain::{NetworkManager};

async fn send_ignore<T>(tx: &mpsc::Sender<T>, msg: T) {
    let _ = tx.send(msg).await;
}


/// # Función principal de orquestación de la base de datos
///
/// Orquesta el flujo de mensajes entre el hub, el servidor y la base de datos,
/// actuando como punto central de decisión según el estado de conectividad
/// del servidor.
///
/// ## Flujo de mensajes entrantes
///
/// Recibe mensajes provenientes del módulo [`crate::message::logic::msg_from_hub`] y:
///
/// - Reenvía los mensajes directamente al servidor cuando el estado es
///   [`ServerStatus::Connected`].
/// - Persiste los mensajes en la base de datos cuando el servidor se encuentra
///   [`ServerStatus::Disconnected`].
///
/// ## Sincronización de datos pendientes
///
/// Cuando el servidor recupera conectividad:
///
/// - Solicita datos pendientes a la tarea [`dba_get_task`].
/// - Reenvía los lotes recuperados al servidor para su procesamiento.
///
/// ## Notas de diseño
///
/// - Esta tarea es *long-lived* y está pensada para ejecutarse durante
///   todo el ciclo de vida del servicio.
/// - No garantiza entrega confiable si los canales de comunicación se cierran.
/// - Los errores de envío se ignoran deliberadamente, ya que el sistema
///   está diseñado para ejecutarse como un servicio `systemd`.

pub async fn dba_task(tx_to_server_batch: mpsc::Sender<TableDataVector>,
                      tx_to_insert: mpsc::Sender<MessageFromHub>,
                      tx_to_db: watch::Sender<DataRequest>,
                      mut rx_from_hub: mpsc::Receiver<MessageFromHub>,
                      mut rx_server_status: watch::Receiver<ServerStatus>,
                      mut rx_from_db: mpsc::Receiver<TableDataVector>,
                      repo: &Repository
                      ) {

    let mut state = ServerStatus::Connected;
    let mut last_state = None;
    let mut state_flag = StateFlag::Init;

    loop {
        tokio::select! {
            status_result = rx_server_status.changed() => {
                if status_result.is_err() { break; }
                state = *rx_server_status.borrow();

                if last_state != Some(state) {
                    last_state = Some(state);

                    if state == ServerStatus::Connected {
                        sync_pending_data(&repo, &mut state_flag, &tx_to_db).await;
                    }
                }
            }

            Some(msg_from_hub) = rx_from_hub.recv() => {
                let is_connected = matches!(state, ServerStatus::Connected);
                if is_connected {
                    send_ignore(&tx_to_insert, msg_from_hub).await;
                }
            }

            Some(msg_from_db) = rx_from_db.recv() => {
                match state {
                    ServerStatus::Connected => {
                        send_ignore(&tx_to_server_batch, msg_from_db).await;
                    },
                    ServerStatus::Disconnected => {
                        if tx_to_db.send(DataRequest::NotGet).is_err() {
                            log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                        }
                    }
                }
            }
        }
    }
}


/// # Función de inserción de elementos en la base de datos
///
/// Recibe mensajes de [`dba_task`] de tipo [`MessageFromHub`] y los agrupa
/// en vectores según su tipo lógico.
/// Cuando se alcanza la capacidad máxima [`BATCH_SIZE`] o expira el intervalo
/// de tiempo [`FLUSH_INTERVAL`], los datos acumulados se empaquetan y se
/// persisten en la base de datos mediante la API de [`Repository`].
///
/// ## Flujo de mensajes entrantes
///
/// Esta tarea solo recibe mensajes de [`dba_task`] cuando el servidor se
/// encuentra en estado [`ServerStatus::Disconnected`].
///
/// ## Notas de diseño
///
/// - Esta tarea es *long-lived* y está pensada para ejecutarse durante
///   todo el ciclo de vida del servicio.
/// - No garantiza entrega confiable si los canales de comunicación se cierran.
/// - Los errores de envío se ignoran deliberadamente, ya que el sistema
///   está diseñado para ejecutarse como un servicio `systemd`.

pub async fn dba_insert_task(mut rx_from_dba: mpsc::Receiver<MessageFromHub>,
                             mut rx_from_net_man: watch::Receiver<NetworkChanged>,
                             repo: &Repository,
                             shared_network_manager: Arc<RwLock<NetworkManager>>,
                            ) {

    let mut net_vec: HashMap<String, Vectors> = HashMap::new();
    let mut timer = interval(FLUSH_INTERVAL);
    timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut last_state = *rx_from_net_man.borrow();

    {   // Inicialización: Bloquear para leer la config inicial
        let manager = shared_network_manager.read().await;
        for (id, _) in &manager.networks {
            net_vec.insert(id.clone(), Vectors::with_capacity(BATCH_SIZE));
        }
    }

    loop {
        tokio::select! {
            Some(msg_from_dba) = rx_from_dba.recv() => {
                let manager = shared_network_manager.read().await;
                sort_by_vectors(msg_from_dba, &mut net_vec, &manager);

                for (_, vectors) in net_vec.iter_mut() {
                    if vectors.is_full(BATCH_SIZE) {
                        let package = flush_buffer(vectors);
                        repo.insert(package).await.ok();
                    }
                }
            }

            _ = timer.tick() => {
                for (_, vectors) in net_vec.iter_mut() {
                    if !vectors.is_empty() {
                        let package = flush_buffer(vectors);
                        repo.insert(package).await.ok();
                    }
                }
            }

            status_result = rx_from_net_man.changed() => {
                if status_result.is_err() { break; }

                let current_state = *rx_from_net_man.borrow();

                if last_state != current_state {
                    last_state = current_state;

                    if current_state == NetworkChanged::Changed {
                        log::info!("Actualizando configuración de redes en memoria...");
                        // Obtenemos el lock de lectura para ver la NUEVA configuración
                        let manager = shared_network_manager.read().await;
                        sync_local_with_global(&mut net_vec, &manager);
                    }
                }
            }
        }
    }
}


/// # Función de obtención de elementos de la base de datos
///
/// Cuando el servidor pasa al estado [`ServerStatus::Connected`] luego de estar
/// [`ServerStatus::Disconnected`], esta función recibe la notificación desde
/// [`dba_task`] indicando que puede quitar elementos de la base de datos y
/// enviarlos a [`dba_task`] que es la tarea administradora. Se envían varios
/// batch de elementos, donde cada uno corresponde a un tipo de datos en particular.
///
/// ## Notas de diseño
///
/// - Esta tarea es *long-lived* y está pensada para ejecutarse durante
///   todo el ciclo de vida del servicio.
/// - No garantiza entrega confiable si los canales de comunicación se cierran.
/// - Los errores de envío se ignoran deliberadamente, ya que el sistema
///   está diseñado para ejecutarse como un servicio `systemd`.

pub async fn dba_get_task(tx_to_dba: mpsc::Sender<TableDataVector>,
                          mut rx_server_status: watch::Receiver<DataRequest>,
                          repo: &Repository,
                          net_man: &NetworkManager
                         ) {
    loop {
        let current_state = *rx_server_status.borrow();

        match current_state {
            DataRequest::NotGet => {
                if rx_server_status.changed().await.is_err() {
                    break;
                }
            }
            DataRequest::Get => {
                tokio::select! {
                    _ = rx_server_status.changed() => {
                        continue;
                    }

                    _ = get_all_tables(repo, net_man, &tx_to_dba) => {
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                }
            }
        }
    }
}


async fn sync_pending_data(repo: &Repository, state_flag: &mut StateFlag, tx_to_db: &watch::Sender<DataRequest>) {
    for &table in Table::all() {
        let has_data = repo.has_data(table).await.unwrap_or(false);
        state_flag.update_state(has_data, tx_to_db).await;
    }
}


async fn get_all_tables(repo: &Repository, net_man: &NetworkManager, tx: &mpsc::Sender<TableDataVector>) {
    for (_, network) in &net_man.networks {
        get_for_table_and_send(repo, Table::Measurement, &network.topic_data.topic, tx).await;
        get_for_table_and_send(repo, Table::Monitor, &network.topic_monitor.topic, tx).await;
        get_for_table_and_send(repo, Table::AlertAir, &network.topic_alert_air.topic, tx).await;
        get_for_table_and_send(repo, Table::AlertTemp, &network.topic_alert_temp.topic, tx).await;
    }
}


async fn get_for_table_and_send(repo: &Repository, table: Table, topic: &str, tx: &mpsc::Sender<TableDataVector>) {
    match repo.pop_batch(table, topic).await {
        Ok(tdv) => {
            if tdv.is_empty() {
                return;
            }
            if let Err(_) = tx.send(tdv).await {
                log::warn!("❌ Receptor cerrado, deteniendo ciclo de lectura");
                return;
            }
        },
        Err(e) => {
            log::error!("❌ Error leyendo tabla {:?}: {}", table, e);
        }
    }
}


fn sort_by_vectors(msg: MessageFromHub, net_vec: &mut HashMap<String, Vectors>, net_manager: &NetworkManager) {

    let id = match net_manager.extract_net_id(&msg.topic_where_arrive) {
        Some(id) => id,
        None => {
            log::error!("No existe la red con ID: {}", msg.topic_where_arrive);
            return;
        }
    };

    let table_type = net_manager.extract_topic(&msg.topic_where_arrive, &id);
    match table_type {
        Table::Measurement | Table::Monitor | Table::AlertAir | Table::AlertTemp => {
            match_message_with_row(msg.msg, msg.topic_where_arrive, &id, net_vec);
        },
        Table::Error => {
            log::warn!("Error de tópico: {}", msg.topic_where_arrive);
        }
    }
}


fn match_message_with_row(msg: MessageFromHubTypes, topic: String, id: &str, net_vec: &mut HashMap<String, Vectors>) {
    match msg {
        MessageFromHubTypes::Report(report) => {
            let mr = report.cast_measurement_to_row(topic);
            net_vec.entry(id.to_string()).or_default().measurements.push(mr);
        },
        MessageFromHubTypes::Monitor(monitor) => {
            let mr = monitor.cast_monitor_to_row(topic);
            net_vec.entry(id.to_string()).or_default().monitors.push(mr);
        },
        MessageFromHubTypes::AlertAir(alert_air) => {
            let aar = alert_air.cast_alert_air_to_row(topic);
            net_vec.entry(id.to_string()).or_default().alert_airs.push(aar);
        },
        MessageFromHubTypes::AlertTem(alert_temp) => {
            let ath = alert_temp.cast_alert_th_to_row(topic);
            net_vec.entry(id.to_string()).or_default().alert_temps.push(ath);
        },
        _ => {},
    }
}


fn flush_buffer(vec: &mut Vectors) -> Vec<TableDataVectorTypes> {
    let mut batch = Vec::new();

    if !vec.measurements.is_empty() {
        let full_vec = mem::replace(&mut vec.measurements, Vec::with_capacity(BATCH_SIZE));
        batch.push(TableDataVectorTypes::Measurement(full_vec));
    }

    if !vec.monitors.is_empty() {
        let full_vec = mem::replace(&mut vec.monitors, Vec::with_capacity(BATCH_SIZE));
        batch.push(TableDataVectorTypes::Monitor(full_vec));
    }

    if !vec.alert_temps.is_empty() {
        let full_vec = mem::replace(&mut vec.alert_temps, Vec::with_capacity(BATCH_SIZE));
        batch.push(TableDataVectorTypes::AlertTemp(full_vec));
    }

    if !vec.alert_airs.is_empty() {
        let full_vec = mem::replace(&mut vec.alert_airs, Vec::with_capacity(BATCH_SIZE));
        batch.push(TableDataVectorTypes::AlertAir(full_vec));
    }

    batch
}


pub fn sync_local_with_global(local_buffers: &mut HashMap<String, Vectors>,
                              network_manager: &NetworkManager
                             ) {

    local_buffers.retain(|id, _| {
        let exists = network_manager.networks.contains_key(id);
        if !exists {
            log::info!("Red eliminada, descartando buffer: {}", id);
        }
        exists
    });

    for (id, _) in &network_manager.networks {
        local_buffers
            .entry(id.clone())
            .or_insert_with(|| {
                log::info!("Buffer iniciado para nueva red: {}", id);
                Vectors::with_capacity(BATCH_SIZE)
            });
    }
}
