use rmp_serde::{from_slice, to_vec};
use serde::Serialize;
use tokio::sync::{mpsc, watch};
use crate::context::domain::AppContext;
use crate::message::domain::{SerializedMessage, MessageToHub, MessageFromHub, MessageToServer, MessageFromServer, BrokerStatus, ServerStatus, MessageFromHubTypes, MessageFromServerTypes, MessageToHubTypes};
use crate::message::domain::MessageToServer::HubToServer;
use crate::fsm::domain::{InternalEvent};
use crate::database::domain::{TableDataVector, TableDataVectorTypes};
use crate::network::domain::NetworkManager;
use crate::system::domain::System;

pub async fn msg_to_hub(tx_to_hub: mpsc::Sender<SerializedMessage>,
                        mut rx_from_fsm: mpsc::Receiver<MessageToHub>,
                        mut rx_from_server: mpsc::Receiver<MessageToHub>,
                        mut rx_broker_status: watch::Receiver<BrokerStatus>,
                        app_context: AppContext
                        ) {

    let mut broker_is_connected = false;
    loop {
        tokio::select! {
            // 1. Gestión de estado del Broker
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

            // 2. Mensajes del Servidor hacia el Hub
            Some(msg_container) = rx_from_server.recv() => {
                if broker_is_connected {
                    if let MessageToHubTypes::ServerToHub(server_to_hub) = msg_container.msg {
                        if let MessageFromServerTypes::Settings(_) = server_to_hub.msg {
                             process_and_send_hub(&tx_to_hub, &msg_container.topic_to, system, network_manager, &server_to_hub.msg).await;
                        }
                    }
                } else {
                    log::warn!("Mensaje de Servidor descartado por desconexión del Broker: {:?}", msg_container.topic_to);
                }
            }

            // 3. Mensajes de la FSM hacia el Hub
            Some(msg_container) = rx_from_fsm.recv() => {
                if broker_is_connected {
                    process_and_send_hub(&tx_to_hub, &msg_container.topic_to, system, network_manager, &msg_container.msg).await;
                } else {
                    log::warn!("Mensaje FSM descartado por desconexión del Broker: {:?}", msg_container.topic_to);
                }
            }
        }
    }
}


/// Helper para msg_to_hub: Reduce la repetición de código
async fn process_and_send_hub<T: Serialize>(tx: &mpsc::Sender<SerializedMessage>,
                                            topic_in: &str,
                                            system: &System,
                                            net_man: &NetworkManager,
                                            payload: &T
                                            ) {
    let mut qos: u8 = 0;
    if let Some(topic_out) = net_man.topic_to_send_msg(topic_in, system, &mut qos) {
        // Ignoramos el resultado del send aquí porque msg_to_hub no es tan crítico si falla el canal interno
        let _ = send(tx, &topic_out, qos, payload, true).await;
    }
}


// -------------------------------------------------------------------------------------------------


pub async fn msg_from_hub(tx_internal: mpsc::Sender<MessageFromHub>,
                          tx_to_server: mpsc::Sender<MessageToServer>,
                          tx_to_dba: mpsc::Sender<MessageFromHub>,
                          app_context: AppContext,
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
                handle_message(&tx_to_server, &tx_to_dba, &tx_internal, msg, &state).await;
            }
        }
    }
}


// -------------------------------------------------------------------------------------------------

pub async fn msg_to_server(tx_to_server: mpsc::Sender<SerializedMessage>,
                           mut rx_from_fsm: mpsc::Receiver<MessageToServer>,
                           mut rx_from_hub: mpsc::Receiver<MessageToServer>,
                           mut rx_from_dba_batch: mpsc::Receiver<TableDataVector>,
                           app_context: AppContext,
                          ) {
    loop {
        let result: Result<(), ()> = tokio::select! {
            Some(_) = rx_from_fsm.recv() => { Ok(()) },

            Some(msg_from_hub) = rx_from_hub.recv() => {
                if let HubToServer(hub_to_server) = msg_from_hub {
                    handle_message_to_send(
                        &hub_to_server.topic_where_arrive,
                        system,
                        network_manager,
                        &hub_to_server.msg,
                        false,
                        &tx_to_server
                    ).await
                } else {
                    Ok(())
                }
            }

            Some(msg_from_dba_batch) = rx_from_dba_batch.recv() => {
                process_batch(msg_from_dba_batch, system, &tx_to_server, network_manager).await
            }
        };

        // Si alguna rama devolvió Err(()), rompemos el loop
        if result.is_err() {
            log::error!("🛑 Tarea msg_to_server detenida por cierre del canal de salida.");
            break;
        }
    }
}


// Helper para procesar el batch y mantener el select limpio
async fn process_batch(batch: TableDataVector,
                       system: &System,
                       tx: &mpsc::Sender<SerializedMessage>,
                       network_manager: &NetworkManager
                      ) -> Result<(), ()> {
    match batch.vec {
        TableDataVectorTypes::Measurement(vec) => {
            for row in vec {
                let topic = row.metadata.topic_where_arrive.clone();
                let payload = MessageFromHubTypes::Report(row.cast_measurement());
                handle_message_to_send(&topic, system, network_manager, &payload, false, tx).await?;
            }
        },
        TableDataVectorTypes::Monitor(monitor_vec) => {
            for row in monitor_vec {
                let topic = row.metadata.topic_where_arrive.clone();
                let payload = MessageFromHubTypes::Monitor(row.cast_monitor());
                handle_message_to_send(&topic, system, network_manager, &payload, false, &tx).await?;
            }
        },
        TableDataVectorTypes::AlertAir(alert_air_vec) => {
            for row in alert_air_vec {
                let topic = row.metadata.topic_where_arrive.clone();
                let payload = MessageFromHubTypes::AlertAir(row.cast_alert_air());
                handle_message_to_send(&topic, system, network_manager, &payload, false, &tx).await?;
            }
        },
        TableDataVectorTypes::AlertTemp(alert_temp_vec) => {
            for row in alert_temp_vec {
                let topic = row.metadata.topic_where_arrive.clone();
                let payload = MessageFromHubTypes::AlertTem(row.cast_alert_th());
                handle_message_to_send(&topic, system, network_manager, &payload, false, &tx).await?;
            }
        },
    }
    Ok(())
}


// -------------------------------------------------------------------------------------------------


pub async fn msg_from_server(tx_to_fsm: mpsc::Sender<MessageFromServer>,
                             tx_to_hub: mpsc::Sender<MessageFromServer>,
                             mut rx_from_server: mpsc::Receiver<InternalEvent>
                             ) {
    loop {
        tokio::select! {
            Some(msg_from_server) = rx_from_server.recv() => {
                match msg_from_server {
                    InternalEvent::IncomingMessage(packet) => {
                        let decoded: Result<MessageFromServerTypes, _> = from_slice(&packet.payload);
                        match decoded {
                            Ok(MessageFromServerTypes::Settings(settings)) => {
                                let msg = MessageFromServer::new(packet.topic, MessageFromServerTypes::Settings(settings));
                                if tx_to_hub.send(msg).await.is_err() {
                                    log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                                }
                            },
                            Ok(MessageFromServerTypes::Network(network)) => {
                                let msg = MessageFromServer::new(packet.topic, MessageFromServerTypes::Network(network));
                                if tx_to_fsm.send(msg).await.is_err() {
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


async fn handle_message_to_send(topic_in: &str,
                                system: &System,
                                network_manager: &NetworkManager,
                                msg: &MessageFromHubTypes,
                                retain: bool,
                                tx: &mpsc::Sender<SerializedMessage>
) -> Result<(), ()> {

    let mut qos: u8 = 0;
    if let Some(topic_out) = network_manager.topic_to_send_msg(topic_in, system, &mut qos) {
        return send(tx, &topic_out, qos, msg, retain).await;
    }

    Ok(()) // Si no se encuentra el tópico, retornamos Ok para seguir vivos
}


async fn send<T: Serialize>(tx: &mpsc::Sender<SerializedMessage>,
                            topic: &str,
                            qos: u8,
                            msg: &T,
                            retain: bool
                           ) -> Result<(), ()> {
    match to_vec(msg) {
        Ok(payload) => {
            let serialized = SerializedMessage::new(topic.to_string(), payload, qos, retain);
            if tx.send(serialized).await.is_err() {
                return Err(());
            }
        }
        Err(e) => log::error!("Error serializando: {:?}", e),
    }
    Ok(())
}


async fn handle_message(tx_to_server: &mpsc::Sender<MessageToServer>,
                        tx_to_dba: &mpsc::Sender<MessageFromHub>,
                        tx_internal: &mpsc::Sender<MessageFromHub>,
                        msg: InternalEvent,
                        state: &ServerStatus) {

    match msg {
        InternalEvent::IncomingMessage(packet) => {
            let decoded: Result<MessageFromHubTypes, _> = from_slice(&packet.payload);
            match decoded {
                Ok(MessageFromHubTypes::Handshake(handshake)) => {
                    let msg = MessageFromHub::new(packet.topic, MessageFromHubTypes::Handshake(handshake));
                    if tx_internal.send(msg).await.is_err() {
                        log::debug!("❌ Error: Receptor no disponible, mensaje descartado");
                    }
                },
                Ok(MessageFromHubTypes::Settings(settings)) => {
                    let msg = MessageFromHub::new(packet.topic, MessageFromHubTypes::Settings(settings));
                    route_message(state, tx_to_server, tx_to_dba, msg).await;
                },
                Ok(MessageFromHubTypes::Monitor(monitor)) => {
                    let msg = MessageFromHub::new(packet.topic, MessageFromHubTypes::Monitor(monitor));
                    route_message(state, tx_to_server, tx_to_dba, msg).await;
                },
                Ok(MessageFromHubTypes::AlertAir(alert_air)) => {
                    let msg = MessageFromHub::new(packet.topic, MessageFromHubTypes::AlertAir(alert_air));
                    route_message(state, tx_to_server, tx_to_dba, msg).await;
                },
                Ok(MessageFromHubTypes::AlertTem(alert_temp)) => {
                    let msg = MessageFromHub::new(packet.topic, MessageFromHubTypes::AlertTem(alert_temp));
                    route_message(state, tx_to_server, tx_to_dba, msg).await;
                },
                Ok(MessageFromHubTypes::Report(report)) => {
                    let msg = MessageFromHub::new(packet.topic, MessageFromHubTypes::Report(report));
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