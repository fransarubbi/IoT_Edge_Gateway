use rmp_serde::{from_slice, to_vec};
use serde::Serialize;
use tokio::sync::{mpsc, watch};
use crate::message::msg_type::{SerializedMessage, MessageToHub, MessageFromHub, MessageToServer, MessageFromServer, BrokerStatus, ServerStatus};
use crate::message::msg_type::MessageToServer::HubToServer;
use crate::system::fsm::{InternalEvent};
use crate::database::domain::TableDataVector;



pub async fn msg_to_hub(tx_to_hub: mpsc::Sender<SerializedMessage>,
                        mut rx_from_fsm: mpsc::Receiver<MessageToHub>,
                        mut rx_from_server: mpsc::Receiver<MessageToHub>,
                        mut rx_broker_status: watch::Receiver<BrokerStatus>) {
    let mut broker_is_connected = false;
    loop {
        tokio::select! {
            status_result = rx_broker_status.changed() => {
                if status_result.is_ok() {
                    let nuevo_estado = rx_broker_status.borrow();
                    match *nuevo_estado {
                        BrokerStatus::Connected => {
                            broker_is_connected = true;
                        },
                        BrokerStatus::Disconnected => {
                            broker_is_connected = false;
                        },
                    }
                } else {
                    break;
                }
            }

            msg_to_hub = rx_from_server.recv() => {
                if broker_is_connected {
                    match msg_to_hub {
                        Some(msg_to_hub) => {
                            match msg_to_hub {
                                MessageToHub::ServerToHub(server_to_hub) => {
                                    match server_to_hub {
                                        MessageFromServer::Settings(settings) => {
                                            send_to_hub(&tx_to_hub, "/topico/topico", 1, &settings).await;
                                        },
                                        _ => {},
                                    }
                                },
                                _ => {},
                            }
                        }
                        _ => {},
                    }
                }
            }

            msg_from_fsm = rx_from_fsm.recv() => {
                if broker_is_connected {
                    match msg_from_fsm {
                        Some(msg_from_fsm) => {
                            match msg_from_fsm {
                                MessageToHub::Handshake(handshake) => {
                                    send_to_hub(&tx_to_hub, "/topico/topico", 1, &handshake).await;
                                },
                                MessageToHub::StateBalanceMode(state_bm) => {
                                    send_to_hub(&tx_to_hub, "/topico/topico", 1, &state_bm).await;
                                },
                                MessageToHub::StateNormal(state_nm) => {
                                    send_to_hub(&tx_to_hub, "/topico/topico", 1, &state_nm).await;
                                },
                                MessageToHub::StateSafeMode(state_sm) => {
                                    send_to_hub(&tx_to_hub, "/topico/topico", 1, &state_sm).await;
                                },
                                _ => {},
                            }
                        }
                        _ => {},
                    }
                }
            }
        }
    }
}


// tx para multiples rx -> fsm y dba
// rx del mqtt
// rx_server_status proveniente del Server Status
/// Al llegar un mensaje de un Hub, se verifica si este esta destinado para la FSM o para el Server.
/// Si es para la FSM, entonces se envia por el canal tx_internal. Si es para el servidor, entonces
/// se valida que el servidor este disponible. Si lo esta, entonces se envia por el canal tx_to_server
/// a la funcion encargada de convertir el mensaje en message pack para ser enviada al Server. Si no esta
/// disponible, entonces se envia al administrador SQLite.
pub async fn msg_from_hub(tx_internal: mpsc::Sender<MessageFromHub>,
                          tx_to_server: mpsc::Sender<MessageToServer>,
                          mut rx: mpsc::Receiver<InternalEvent>,
                          mut rx_server_status: watch::Receiver<ServerStatus>) {

    let mut server_is_connected = false;
    loop {
        tokio::select! {
            status_result = rx_server_status.changed() => {
                if status_result.is_ok() {
                    let nuevo_estado = rx_server_status.borrow();
                    match *nuevo_estado {
                        ServerStatus::Connected => {
                            server_is_connected = true;
                        },
                        ServerStatus::Disconnected => {
                            server_is_connected = false;
                        },
                    }
                } else {
                    break;
                }
            }

            msg = rx.recv() => {
                match msg {
                    Some(InternalEvent::IncomingMessage(packet)) => {
                        let decoded: Result<MessageFromHub, _> = from_slice(&packet);
                        match decoded {
                            Ok(MessageFromHub::Handshake(handshake)) => {
                                if tx_internal.send(MessageFromHub::Handshake(handshake)).await.is_err() {
                                    log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                                }
                            },
                            Ok(MessageFromHub::Settings(settings)) => {
                                if server_is_connected {
                                    if tx_to_server.send(HubToServer(MessageFromHub::Settings(settings))).await.is_err() {
                                        log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                                    }
                                } else {
                                    if tx_internal.send(MessageFromHub::Settings(settings)).await.is_err() {
                                        log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                                    }
                                }
                            },
                            Ok(MessageFromHub::Monitor(monitor)) => {
                                if server_is_connected {
                                    if tx_to_server.send(HubToServer(MessageFromHub::Monitor(monitor))).await.is_err() {
                                        log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                                    }
                                } else {
                                    if tx_internal.send(MessageFromHub::Monitor(monitor)).await.is_err() {
                                        log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                                    }
                                }
                            },
                            Ok(MessageFromHub::AlertAir(alert_air)) => {
                                if server_is_connected {
                                    if tx_to_server.send(HubToServer(MessageFromHub::AlertAir(alert_air))).await.is_err() {
                                        log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                                    }
                                } else {
                                    if tx_internal.send(MessageFromHub::AlertAir(alert_air)).await.is_err() {
                                        log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                                    }
                                }
                            },
                            Ok(MessageFromHub::AlertTem(alert_temp)) => {
                                if server_is_connected {
                                    if tx_to_server.send(HubToServer(MessageFromHub::AlertTem(alert_temp))).await.is_err() {
                                        log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                                    }
                                } else {
                                    if tx_internal.send(MessageFromHub::AlertTem(alert_temp)).await.is_err() {
                                        log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                                    }
                                }
                            },
                            Ok(MessageFromHub::Report(report)) => {
                                if server_is_connected {
                                    if tx_to_server.send(HubToServer(MessageFromHub::Report(report))).await.is_err() {
                                        log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                                    }
                                } else {
                                    if tx_internal.send(MessageFromHub::Report(report)).await.is_err() {
                                        log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                                    }
                                }
                            },
                            _ => {},
                        }
                    }
                    _ => {},
                }
            }
        }
    }
}


pub async fn msg_to_server(tx_to_server: mpsc::Sender<SerializedMessage>,
                           mut rx_from_fsm: mpsc::Receiver<MessageToServer>,
                           mut rx_from_hub: mpsc::Receiver<MessageToServer>,
                           mut rx_from_dba: mpsc::Receiver<MessageToServer>,
                           mut rx_from_dba_batch: mpsc::Receiver<Vec<TableDataVector>>) {

    loop {
        tokio::select! {
            msg_from_fsm = rx_from_fsm.recv() => {
                match msg_from_fsm {
                    _ => {},
                }
            }
            
            msg_from_hub = rx_from_hub.recv() => {
                match msg_from_hub {
                    Some(HubToServer(hub_to_server)) => {
                        match hub_to_server {
                            MessageFromHub::Report(report) => {
                                send_to_hub(&tx_to_server, "/topico/topico", 0, &report).await;
                            },
                            MessageFromHub::Settings(settings) => {
                                send_to_hub(&tx_to_server, "/topico/topico", 0, &settings).await;
                            },
                            MessageFromHub::AlertAir(alert_air) => {
                                send_to_hub(&tx_to_server, "/topico/topico", 0, &alert_air).await;
                            },
                            MessageFromHub::AlertTem(alert_temp) => {
                                send_to_hub(&tx_to_server, "/topico/topico", 0, &alert_temp).await;
                            },
                            MessageFromHub::Monitor(monitor) => {
                                send_to_hub(&tx_to_server, "/topico/topico", 0, &monitor).await;
                            },
                            _ => {},
                        }
                    }
                    _ => {},
                }
                
            }
            
            msg_from_dba = rx_from_dba.recv() => {
                match msg_from_dba {
                    _ => {},
                }
            }
            
            msg_from_dba_batch = rx_from_dba_batch.recv() => {
                match msg_from_dba_batch {
                    _ => {},
                }
            }
        }
    }
}


pub async fn msg_from_server(tx_to_fsm: mpsc::Sender<MessageFromServer>,
                            tx_to_hub: mpsc::Sender<MessageFromServer>,
                            mut rx_from_server: mpsc::Receiver<InternalEvent>) {
    loop {
        match rx_from_server.recv().await {
            Some(InternalEvent::IncomingMessage(packet)) => {
                let decoded: Result<MessageFromServer, _> = from_slice(&packet);
                match decoded {
                    Ok(MessageFromServer::Settings(settings)) => {
                        if tx_to_hub.send(MessageFromServer::Settings(settings)).await.is_err() {
                            log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                        }
                    },
                    Ok(MessageFromServer::Network(network)) => {
                        if tx_to_fsm.send(MessageFromServer::Network(network)).await.is_err() {
                            log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                        }
                    },
                    _ => {},
                }
            }
            _ => {},
        }
    }
}


async fn send_to_hub<T: Serialize>(
    tx: &mpsc::Sender<SerializedMessage>,
    topic: &str,
    qos: u8,
    msg: &T,
) {
    match to_vec(msg) {
        Ok(payload) => {
            let serialized = SerializedMessage::new(
                topic.to_string(),
                payload,
                qos,
                false,
            );
            let _ = tx.send(serialized).await;
        }
        Err(e) => {
            log::error!("Error serializando mensaje: {:?}", e);
        }
    }
}
