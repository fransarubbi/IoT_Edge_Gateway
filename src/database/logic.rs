//! Módulo de orquestación de base de datos y persistencia.
//!
//! Este módulo implementa el patrón **"Store and Forward"** (Almacenar y Reenviar)
//! para garantizar la integridad de los datos cuando se pierde la conexión con el servidor.
//!
//! # Arquitectura
//!
//! El sistema se divide en tres tareas asíncronas principales que se comunican mediante canales:
//!
//! 1.  **[`dba_task`]:** El despachador central. Decide si los datos van al servidor (si hay red)
//!     o al buffer de inserción (si no hay red). También coordina la extracción de datos guardados.
//! 2.  **[`dba_insert_task`]:** El consumidor de escritura. Agrupa los datos en memoria (Buffers)
//!     y realiza inserciones por lotes (*Batch Insert*) en SQLite para maximizar el rendimiento.
//! 3.  **[`dba_get_task`]:** El productor de lectura. Extrae datos de la base de datos y los
//!     inyecta de nuevo en el flujo de envío cuando se recupera la conexión.
//!
//! # Concurrencia
//!
//! Se utiliza [`AppContext`] para compartir el acceso seguro
//! al repositorio y a la configuración de red (`NetworkManager`) mediante `Arc<RwLock<...>>`.


use std::collections::HashMap;
use std::mem;
use tokio::sync::{mpsc, watch};
use tokio::time::{interval, MissedTickBehavior};
use tracing::{error, info, instrument};
use crate::config::sqlite::{BATCH_SIZE, FLUSH_INTERVAL};
use crate::context::domain::AppContext;
use crate::message::domain::{DataRequest, MessageFromHub, MessageFromHubTypes, NetworkChanged, ServerStatus};
use super::domain::{NetworkAux, StateFlag, Table, TableDataVector, TableDataVectorTypes, Vectors};
use crate::database::repository::Repository;
use crate::network::domain::{NetworkManager};


/// Envío auxiliar "Fire-and-Forget".
///
/// Intenta enviar un mensaje por el canal sin bloquear y descartando errores.
/// Se usa para no detener el flujo principal si un canal secundario está saturado o cerrado.
async fn send_ignore<T>(tx: &mpsc::Sender<T>, msg: T) {
    let _ = tx.send(msg).await;
}


/// Orquestador principal del flujo de datos.
///
/// Actúa como un **Router** inteligente que dirige el tráfico basándose en el
/// estado de la conexión con el servidor (`ServerStatus`).
///
/// # Máquina de Estados Implícita
///
/// - **Connected:**
///   - Los mensajes entrantes del Hub se envían directo al servidor.
///   - Se activa la sincronización de datos pendientes (`sync_pending_data`).
///   - Los datos recuperados de la DB se reenvían al servidor.
///
/// - **Disconnected:**
///   - Los mensajes entrantes se desvían a [`dba_insert_task`] para ser guardados.
///   - Se ordena a [`dba_get_task`] que detenga la lectura (`DataRequest::NotGet`).
///
/// # Diseño
///
/// Utiliza `tokio::select!` para manejar concurrentemente cambios de estado y flujo de mensajes.

#[instrument(name = "dba_task", skip(app_context))]
pub async fn dba_task(tx_to_server: mpsc::Sender<MessageFromHub>,
                      tx_to_server_batch: mpsc::Sender<TableDataVector>,
                      tx_to_insert: mpsc::Sender<MessageFromHub>,
                      tx_to_db: watch::Sender<DataRequest>,
                      mut rx_from_hub: mpsc::Receiver<MessageFromHub>,
                      mut rx_server_status: watch::Receiver<ServerStatus>,
                      mut rx_from_db: mpsc::Receiver<TableDataVector>,
                      app_context: AppContext) {

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
                        sync_pending_data(&app_context.repo, &mut state_flag, &tx_to_db).await;
                    }
                }
            }

            Some(msg_from_hub) = rx_from_hub.recv() => {
                let is_connected = matches!(state, ServerStatus::Connected);
                if is_connected {
                    send_ignore(&tx_to_server, msg_from_hub).await;
                } else {
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
                            error!("Error: Receptor no disponible, mensaje descartado");
                        }
                    }
                }
            }
        }
    }
}


/// Verifica si existen datos en la base de datos y actualiza la bandera de estado.
///
/// Esto permite que el sistema sepa si debe iniciar el proceso de recuperación (`dba_get_task`).
async fn sync_pending_data(repo: &Repository, state_flag: &mut StateFlag, tx_to_db: &watch::Sender<DataRequest>) {
    for &table in Table::all() {
        let has_data = repo.has_data(table).await.unwrap_or(false);
        state_flag.update_state(has_data, tx_to_db).await;
    }
}


// -------------------------------------------------------------------------------------------------


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

#[instrument(name = "dba_insert_task", skip(app_context))]
pub async fn dba_insert_task(mut rx_from_dba: mpsc::Receiver<MessageFromHub>,
                             mut rx_from_net_man: watch::Receiver<NetworkChanged>,
                             app_context: AppContext) {

    let mut net_vec: HashMap<String, Vectors> = HashMap::new();
    let mut timer = interval(FLUSH_INTERVAL);
    timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut last_state = *rx_from_net_man.borrow();

    {   // Inicialización: Bloquear para leer la config inicial
        let manager = app_context.net_man.read().await;
        for (id, _ ) in &manager.networks {
            net_vec.insert(id.clone(), Vectors::with_capacity(BATCH_SIZE));
        }
    }

    loop {
        tokio::select! {
            Some(msg_from_dba) = rx_from_dba.recv() => {
                let manager = app_context.net_man.read().await;
                sort_by_vectors(msg_from_dba, &mut net_vec, &manager);

                for (_, vectors) in net_vec.iter_mut() {
                    if vectors.is_full(BATCH_SIZE) {
                        let package = flush_buffer(vectors);
                        app_context.repo.insert(package).await.ok();
                    }
                }
            }

            _ = timer.tick() => {
                for (_, vectors) in net_vec.iter_mut() {
                    if !vectors.is_empty() {
                        let package = flush_buffer(vectors);
                        app_context.repo.insert(package).await.ok();
                    }
                }
            }

            status_result = rx_from_net_man.changed() => {
                if status_result.is_err() { break; }

                let current_state = *rx_from_net_man.borrow();

                if last_state != current_state {
                    last_state = current_state;

                    if current_state == NetworkChanged::Changed {
                        info!("Actualizando configuración de redes en memoria...");
                        let manager = app_context.net_man.read().await;
                        sync_local_with_global(&mut net_vec, &manager);
                    }
                }
            }
        }
    }
}


/// Clasifica un mensaje entrante y lo inserta en el vector correspondiente en memoria.
///
/// Identifica la red y el tipo de tabla basándose en el tópico MQTT.
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


/// Convierte el mensaje genérico a una fila de base de datos específica y lo empuja al vector.
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


/// Vacía los vectores de memoria y retorna un paquete listo para insertar.
///
/// Utiliza `mem::replace` para una operación eficiente de "swap", dejando
/// vectores nuevos vacíos en su lugar con la capacidad pre-reservada.
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


/// Sincroniza el mapa local de buffers con la configuración global de redes.
///
/// - Elimina buffers de redes que ya no existen (Pruning).
/// - Crea buffers para redes nuevas (Populating).
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


// -------------------------------------------------------------------------------------------------


/// Tarea de recuperación de datos.
///
/// Extrae datos de la base de datos para ser enviados al servidor.
/// Funciona bajo demanda controlada por el canal `rx_server_status`.
///
/// # Funcionamiento
///
/// 1. Espera la señal [`DataRequest::Get`].
/// 2. Crea una **snapshot** de la lista de redes (`collect`) para no bloquear el `RwLock`
///    durante las operaciones de I/O de base de datos.
/// 3. Itera sobre cada red y tabla, extrayendo lotes (`pop_batch`) y enviándolos.
/// 4. Incluye un mecanismo de pausa (`sleep`) para no saturar la CPU/DB en bucles vacíos.

#[instrument(name = "dba_get_task")]
pub async fn dba_get_task(tx_to_dba: mpsc::Sender<TableDataVector>,
                          mut rx_from_dba: watch::Receiver<DataRequest>,
                          app_context: AppContext,
                         ) {

    loop {
        let state = *rx_from_dba.borrow();
        match state {
            DataRequest::NotGet => {
                if rx_from_dba.changed().await.is_err() {
                    break;
                }
            }

            DataRequest::Get => {
                let vec_networks = {
                    let manager = app_context.net_man.read().await;
                    collect(&manager)
                };

                tokio::select! {
                    _ = rx_from_dba.changed() => {
                        continue;
                    }

                    _ = get_all_tables(&app_context.repo, vec_networks, &tx_to_dba) => {
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                }
            }
        }
    }
}


/// Itera sobre todas las redes configuradas y consulta todas las tablas relevantes.
async fn get_all_tables(repo: &Repository, vec: Vec<NetworkAux>, tx: &mpsc::Sender<TableDataVector>) {
    for net in vec {
        get_for_table_and_send(repo, Table::Measurement, &net.topic_data.topic, tx).await;
        get_for_table_and_send(repo, Table::Monitor, &net.topic_monitor.topic, tx).await;
        get_for_table_and_send(repo, Table::AlertAir, &net.topic_alert_air.topic, tx).await;
        get_for_table_and_send(repo, Table::AlertTemp, &net.topic_alert_temp.topic, tx).await;
    }
}


/// Extrae un lote de una tabla específica y lo envía si no está vacío.
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


/// Genera una copia ligera (Snapshot) de la configuración de redes actual.
///
/// Esto permite iterar sobre las redes para hacer consultas a BD sin mantener
/// bloqueado el `RwLock` del `NetworkManager`.
fn collect(manager: &NetworkManager) -> Vec<NetworkAux> {
    let mut networks : Vec<NetworkAux> = Vec::new();
    for net in manager.networks.values() {
        let net_aux = NetworkAux::new(
                                      net.id_network.clone(),
                                      net.topic_data.clone(),
                                      net.topic_alert_air.clone(),
                                      net.topic_alert_temp.clone(),
                                      net.topic_monitor.clone());
        networks.push(net_aux);
    }
    networks
}