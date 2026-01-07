use rmp_serde::{from_slice, to_vec};
use serde::Serialize;
use tokio::sync::{mpsc, watch};
use crate::message::domain::{SerializedMessage, MessageToHub, MessageFromHub, MessageToServer, MessageFromServer, BrokerStatus, ServerStatus, MessageFromHubTypes};
use crate::message::domain::MessageToServer::HubToServer;
use crate::fsm::domain::{InternalEvent};
use crate::database::domain::TableDataVector;
use crate::network::domain::NetworkManager;
use crate::system::domain::System;

pub async fn msg_to_hub(tx_to_hub: mpsc::Sender<SerializedMessage>,
                        mut rx_from_fsm: mpsc::Receiver<MessageToHub>,
                        mut rx_from_server: mpsc::Receiver<MessageToHub>,
                        mut rx_broker_status: watch::Receiver<BrokerStatus>
                        ) {

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


pub async fn msg_from_hub(tx_internal: mpsc::Sender<MessageFromHub>,
                          tx_to_server: mpsc::Sender<MessageToServer>,
                          tx_to_dba: mpsc::Sender<MessageFromHub>,
                          mut rx_mqtt: mpsc::Receiver<InternalEvent>,
                          mut rx_server_status: watch::Receiver<ServerStatus>,
                         ) {

    let mut state: ServerStatus = ServerStatus::Connected;
    let mut last_state: Option<ServerStatus> = None;

    loop {
        tokio::select! {
            status_result = rx_server_status.changed() => {
                if status_result.is_err() { break; }
                let new_state = *rx_server_status.borrow();
                if last_state != Some(new_state) {
                    last_state = Some(new_state);
                    state = new_state;
                }
            }

            Some(msg) = rx_mqtt.recv() => {
                handle_message(&tx_to_server, &tx_to_dba, &tx_internal, &msg, &state).await;
            }
        }
    }
}


pub async fn msg_to_server(tx_to_server: mpsc::Sender<SerializedMessage>,
                           mut rx_from_fsm: mpsc::Receiver<MessageToServer>,
                           mut rx_from_hub: mpsc::Receiver<MessageToServer>,
                           mut rx_from_dba_batch: mpsc::Receiver<TableDataVector>,
                           network_manager: &NetworkManager,
                           system: &System,
                           ) {

    loop {
        tokio::select! {
            Some(msg_from_fsm) = rx_from_fsm.recv() => {
                match msg_from_fsm {
                    _ => {},
                }
            }
            
            Some(msg_from_hub) = rx_from_hub.recv() => {
                match msg_from_hub {
                    HubToServer(hub_to_server) => {
                        let topic = hub_to_server.topic_where_arrive;
                        match hub_to_server.msg {
                            MessageFromHubTypes::Report(report) => {
                                handle_message_to_hub(&topic, system, &network_manager, MessageFromHubTypes::Report(report), &tx_to_server).await;
                            },
                            MessageFromHubTypes::Settings(settings) => {
                                handle_message_to_hub(&topic, system, &network_manager, MessageFromHubTypes::Settings(settings), &tx_to_server).await;
                            },
                            MessageFromHubTypes::AlertAir(alert_air) => {
                                handle_message_to_hub(&topic, system, &network_manager, MessageFromHubTypes::AlertAir(alert_air), &tx_to_server).await;
                            },
                            MessageFromHubTypes::AlertTem(alert_temp) => {
                                handle_message_to_hub(&topic, system, &network_manager, MessageFromHubTypes::AlertTem(alert_temp), &tx_to_server).await;
                            },
                            MessageFromHubTypes::Monitor(monitor) => {
                                handle_message_to_hub(&topic, system, &network_manager, MessageFromHubTypes::Monitor(monitor), &tx_to_server).await;
                            },
                            _ => {},
                        }
                    }
                }
            }
            
            Some(msg_from_dba_batch) = rx_from_dba_batch.recv() => {
                match msg_from_dba_batch {
                    TableDataVector::Measurement(measurement) => {
                        for vec in measurement {
                            let topic = &vec.metadata.topic_where_arrive;
                            let msg = MessageFromHub::new(&vec.metadata.topic_where_arrive, MessageFromHubTypes::Report(vec.clone()));
                            handle_message_to_hub(&topic, system, &network_manager, msg.msg, &tx_to_server).await;
                            send(&tx_to_server, "a/a", 0, &vec).await;
                        }
                        /*
                        send<T: Serialize>(tx: &mpsc::Sender<SerializedMessage>,
                            topic: &str,
                            qos: u8,
                            msg: &T,
                            )
                         */
                    },
                    TableDataVector::Monitor(monitor) => {

                    },
                    TableDataVector::AlertAir(alert_air) => {

                    },
                    TableDataVector::AlertTemp(alert_temp) => {

                    },
                }
            }
        }
    }
}


pub async fn msg_from_server(tx_to_fsm: mpsc::Sender<MessageFromServer>,
                            tx_to_hub: mpsc::Sender<MessageFromServer>,
                            mut rx_from_server: mpsc::Receiver<InternalEvent>
                            ) {
    loop {
        tokio::select! {
            Some(msg_from_server) = rx_from_server.recv() => {
                match msg_from_server {
                    InternalEvent::IncomingMessage(packet) => {
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
                    _ => {}
                }
            }
        }
    }
}


async fn handle_message_to_hub(topic: &str,
                               system: &System,
                               network_manager: &NetworkManager,
                               msg: MessageFromHubTypes,
                               tx_to_server: &mpsc::Sender<SerializedMessage>
                              ) {

    let mut qos : u8 = 0;
    if let Some(topic_out) = network_manager.topic_to_send_msg(&topic, system, &mut qos) {
        send(&tx_to_server, &topic_out, qos, &msg).await;
    }
}


async fn handle_message_to_ser(topic: &str,
                               system: &System,
                               network_manager: &NetworkManager,
                               msg: MessageFromHubTypes,
                               tx_to_server: &mpsc::Sender<SerializedMessage>
) {

    let mut qos : u8 = 0;
    if let Some(topic_out) = network_manager.topic_to_send_msg(&topic, system, &mut qos) {
        send(&tx_to_server, &topic_out, qos, &msg).await;
    }
}


async fn send<T: Serialize>(tx: &mpsc::Sender<SerializedMessage>,
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



async fn handle_message(tx_to_server: &mpsc::Sender<MessageToServer>,
                        tx_to_dba: &mpsc::Sender<MessageFromHub>,
                        tx_internal: &mpsc::Sender<MessageFromHub>,
                        msg: &InternalEvent,
                        state: &ServerStatus,) {

    match msg {
        InternalEvent::IncomingMessage(packet) => {
            let decoded: Result<MessageFromHubTypes, _> = from_slice(&packet.payload);
            match decoded {
                Ok(MessageFromHubTypes::Handshake(handshake)) => {
                    let msg = MessageFromHub::new(&packet.topic, MessageFromHubTypes::Handshake(handshake));
                    if tx_internal.send(msg).await.is_err() {
                        log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                    }
                },
                Ok(MessageFromHubTypes::Settings(settings)) => {
                    let msg = MessageFromHub::new(&packet.topic, MessageFromHubTypes::Settings(settings));
                    route_message(state, tx_to_server, tx_to_dba, msg).await;
                },
                Ok(MessageFromHubTypes::Monitor(monitor)) => {
                    let msg = MessageFromHub::new(&packet.topic, MessageFromHubTypes::Monitor(monitor));
                    route_message(state, tx_to_server, tx_to_dba, msg).await;
                },
                Ok(MessageFromHubTypes::AlertAir(alert_air)) => {
                    let msg = MessageFromHub::new(&packet.topic, MessageFromHubTypes::AlertAir(alert_air));
                    route_message(state, tx_to_server, tx_to_dba, msg).await;
                },
                Ok(MessageFromHubTypes::AlertTem(alert_temp)) => {
                    let msg = MessageFromHub::new(&packet.topic, MessageFromHubTypes::AlertTem(alert_temp));
                    route_message(state, tx_to_server, tx_to_dba, msg).await;
                },
                Ok(MessageFromHubTypes::Report(report)) => {
                    let msg = MessageFromHub::new(&packet.topic, MessageFromHubTypes::Report(report));
                    route_message(state, tx_to_server, tx_to_dba, msg).await;
                },
                _ => {},
            }
        }
        _ => {},
    }
}



async fn route_message(state: &ServerStatus,
                       tx_to_server: &mpsc::Sender<MessageToServer>,
                       tx_to_dba: &mpsc::Sender<MessageFromHub>,
                       msg: MessageFromHub) {

    if matches!(state, ServerStatus::Connected) {
        if tx_to_server.send(HubToServer(msg)).await.is_err() {
            log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
        }
    } else {
        if tx_to_dba.send(msg).await.is_err() {
            log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
        }
    }
}