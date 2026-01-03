use std::mem;
use tokio::sync::{mpsc, watch};
use tokio::task;
use tokio::time::{interval, MissedTickBehavior};
use crate::config::sqlite::{BATCH_SIZE, FLUSH_INTERVAL};
use crate::message::domain::{DataRequest, MessageFromHub, MessageToServer, ServerStatus};
use super::domain::{route_message, Route, StateFlag, Table, TableDataVector, Vectors};
use crate::database::repository::Repository;



async fn send_ignore<T>(tx: &mpsc::Sender<T>, msg: T) {
    let _ = tx.send(msg).await;
}


pub async fn dba_task(tx_to_server: mpsc::Sender<MessageToServer>,
                      tx_to_server_batch: mpsc::Sender<TableDataVector>,
                      tx_to_insert: mpsc::Sender<MessageFromHub>,
                      tx_to_db: watch::Sender<DataRequest>,
                      mut rx_from_hub: mpsc::Receiver<MessageFromHub>,
                      mut rx_server_status: watch::Receiver<ServerStatus>,
                      mut rx_from_db: mpsc::Receiver<TableDataVector>,
                      repo: &Repository
                      ) {

    let mut state = ServerStatus::Connected;
    let mut state_flag = StateFlag::Init;

    loop {
        tokio::select! {
            status_result = rx_server_status.changed() => {
                if status_result.is_err() { break; }
                state = *rx_server_status.borrow();
                match state {
                    ServerStatus::Connected => {
                        log::info!("Status: Conectado.");
                        sync_pending_data(&repo, &mut state_flag, &tx_to_db).await;
                    },
                    ServerStatus::Disconnected => {
                        log::info!("Status: Desconectado. Pausando envíos.");
                    },
                }
            }

            Some(msg_from_hub) = rx_from_hub.recv() => {
                let is_connected = matches!(state, ServerStatus::Connected);
                match route_message(msg_from_hub, is_connected) {
                    Route::ToServer(m)   => send_ignore(&tx_to_server, m).await,
                    Route::ToDatabase(m) => send_ignore(&tx_to_insert, m).await,
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


pub async fn dba_insert_tasks(mut rx_from_dba: mpsc::Receiver<MessageFromHub>,
                              repo: &Repository
                              ) {

    let mut vectors_for_table : Vectors = Vectors::with_capacity(BATCH_SIZE);
    let mut timer = interval(FLUSH_INTERVAL);
    timer.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            Some(msg_from_dba) = rx_from_dba.recv() => {
                sort_by_vectors(msg_from_dba, &mut vectors_for_table).await;
                if vectors_for_table.is_full(BATCH_SIZE) {
                     let package = flush_buffer(&mut vectors_for_table).await;
                     repo.insert(package).await.ok();
                     timer.reset();
                }
            }

            _ = timer.tick() => {
                if !vectors_for_table.is_empty() {
                    let package = flush_buffer(&mut vectors_for_table).await;
                    repo.insert(package).await.ok();
                }
            }
        }
    }
}


pub async fn dba_get_task(tx_to_dba: mpsc::Sender<TableDataVector>,
                          mut rx_server_status: watch::Receiver<DataRequest>,
                          repo: &Repository
                          ) {

    let mut state : DataRequest;

    loop {
        tokio::select! {
            status_result = rx_server_status.changed() => {
                if status_result.is_err() { break; }
                state = *rx_server_status.borrow();
                match state {
                    DataRequest::Get => {
                        get_all_tables(&repo, &tx_to_dba).await;
                    },
                    DataRequest::NotGet => {},
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


async fn get_all_tables(repo: &Repository, tx: &mpsc::Sender<TableDataVector>) {

    for &table in Table::all() {
        match repo.pop_batch(table).await {
            Ok(tdv) => {
                if let Err(_) = tx.send(tdv).await {
                    log::warn!("❌ Receptor cerrado, deteniendo ciclo de lectura");
                    return;
                }
                task::yield_now().await;
            },
            Err(sqlx::Error::RowNotFound) => {},
            Err(e) => {
                log::error!("❌ Error leyendo tabla {:?}: {}", table, e);
            }
        }
    }
}


async fn sort_by_vectors(msg: MessageFromHub, all: &mut Vectors) {
    match msg {
        MessageFromHub::Report(report) => {
            all.measurements.push(report);
        },
        MessageFromHub::Monitor(monitor) => {
            all.monitors.push(monitor);
        },
        MessageFromHub::AlertTem(alert_temp) => {
            all.alert_temps.push(alert_temp);
        },
        MessageFromHub::AlertAir(alert_air) => {
            all.alert_airs.push(alert_air);
        },
        _ => {},
    }

}


async fn flush_buffer(vec: &mut Vectors) -> Vec<TableDataVector> {
    let mut batch = Vec::new();

    if !vec.measurements.is_empty() {
        let full_vec = mem::replace(&mut vec.measurements, Vec::with_capacity(BATCH_SIZE));
        batch.push(TableDataVector::Measurement(full_vec));
    }

    if !vec.monitors.is_empty() {
        let full_vec = mem::replace(&mut vec.monitors, Vec::with_capacity(BATCH_SIZE));
        batch.push(TableDataVector::Monitor(full_vec));
    }

    if !vec.alert_temps.is_empty() {
        let full_vec = mem::replace(&mut vec.alert_temps, Vec::with_capacity(BATCH_SIZE));
        batch.push(TableDataVector::AlertTemp(full_vec));
    }

    if !vec.alert_airs.is_empty() {
        let full_vec = mem::replace(&mut vec.alert_airs, Vec::with_capacity(BATCH_SIZE));
        batch.push(TableDataVector::AlertAir(full_vec));
    }

    batch
}



