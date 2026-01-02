use tokio::sync::{mpsc, watch};
use crate::message::msg_type::{ DataRequest, MessageFromHub, MessageToServer, ServerStatus};
use super::domain::{route_message, Route, StateFlag, Table, TableData, TableDataVector, };
use crate::database::repository::Repository;



async fn send_ignore<T>(tx: &mpsc::Sender<T>, msg: T) {
    let _ = tx.send(msg).await;
}


pub async fn dba_task(tx_to_server: mpsc::Sender<MessageToServer>,
                      tx_to_server_batch: mpsc::Sender<Vec<TableDataVector>>,
                      tx_to_insert: mpsc::Sender<MessageFromHub>,
                      tx_to_db: watch::Sender<DataRequest>,
                      mut rx_from_hub: mpsc::Receiver<MessageFromHub>,
                      mut rx_server_status: watch::Receiver<ServerStatus>,
                      mut rx_from_db: mpsc::Receiver<Vec<TableDataVector>>,
                      repo: &Repository) {

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


pub async fn dba_insert_task(mut rx_from_dba: mpsc::Receiver<MessageFromHub>,
                             repo: &Repository
                             ) {
    loop {
        match rx_from_dba.recv().await {
            Some(MessageFromHub::Report(report)) => {
                match repo.insert(TableData::Measurement(report)).await {
                    Ok(_) => {},
                    Err(e) => {
                        log::warn!("❌ Error: No se pudo insertar en la tabla measurement: {:?}", e);
                    }
                }
            },
            Some(MessageFromHub::Monitor(monitor)) => {
                match repo.insert(TableData::Monitor(monitor)).await {
                    Ok(_) => {},
                    Err(e) => {
                        log::warn!("❌ Error: No se pudo insertar en la tabla monitor: {:?}", e);
                    },
                }
            },
            Some(MessageFromHub::AlertTem(alert_temp)) => {
                match repo.insert(TableData::AlertTemp(alert_temp)).await {
                    Ok(_) => {},
                    Err(e) => {
                        log::warn!("❌ Error: No se pudo insertar en la tabla alert_temp: {:?}", e);
                    },
                }
            },
            Some(MessageFromHub::AlertAir(alert_air)) => {
                match repo.insert(TableData::AlertAir(alert_air)).await {
                    Ok(_) => {},
                    Err(e) => {
                        log::warn!("❌ Error: No se pudo insertar en la tabla alert_air: {:?}", e);
                    },
                }
            },
            None => {
                continue;
            },
            _ => {},
        }
    }
}


pub async fn dba_get_task(tx_to_dba: mpsc::Sender<Vec<TableDataVector>>,
                          mut rx_server_status: watch::Receiver<DataRequest>,
                          repo: &Repository
                          ) {

    let mut flag_request:bool;

    loop {
        let status_result = rx_server_status.changed().await;
        if status_result.is_ok() {
            let nuevo_estado = rx_server_status.borrow();
            match *nuevo_estado {
                DataRequest::Get => {
                    flag_request = true;
                },
                DataRequest::NotGet => {
                    flag_request = false;
                },
            }
        } else {
            break;
        }

        if flag_request {
            let vector = repo.pop_batch().await
                .unwrap_or_else(|_| { log::warn!("❌ Error, se esta usando valor vacío"); Vec::new() });

            if tx_to_dba.send(vector).await.is_err() {
                log::warn!("❌ Error: No se pudo enviar batch de datos");
            }
        }
    }
}


async fn sync_pending_data(repo: &Repository, state_flag: &mut StateFlag, tx_to_db: &watch::Sender<DataRequest>) {

    let tables = [
        Table::Measurement,
        Table::Monitor,
        Table::AlertAir,
        Table::AlertTemp
    ];

    for table in tables {
        let has_data = repo.has_data(table).await.unwrap_or(false);
        state_flag.update_state(has_data, tx_to_db).await;
    }
}