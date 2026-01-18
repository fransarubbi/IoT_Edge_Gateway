//! Módulo de Lógica de Mensajería.
//!
//! Este módulo actúa como el "Switchboard" del sistema.
//! Se encarga de enrutar mensajes entre los distintos componentes (Hub, Servidor, Base de Datos, FSM)
//! y de realizar las transformaciones necesarias (serialización, mapeo de tópicos, QoS).
//!
//! # Responsabilidades
//!
//! - **Uplink (Subida):** Recibir datos del Hub local, decidir si enviarlos al servidor o guardarlos en DB (según conectividad).
//! - **Downlink (Bajada):** Recibir comandos del servidor, traducirlos y enviarlos al Hub o a la gestión de redes.
//! - **Control:** Gestionar mensajes de estado (Handshakes, Heartbeats) generados por la FSM.


use rmp_serde::{from_slice, to_vec};
use serde::Serialize;
use tokio::sync::{mpsc};
use tracing::{error, warn};
use crate::context::domain::AppContext;
use crate::message::domain::{SerializedMessage, MessageToHub, MessageFromHub, MessageToServer, MessageFromServer, ServerStatus, MessageFromHubTypes, MessageFromServerTypes, MessageToHubTypes, LocalStatus};
use crate::message::domain::MessageToServer::ToServer;
use crate::fsm::domain::{InternalEvent};
use crate::database::domain::{TableDataVector, TableDataVectorTypes};
use crate::message::domain::MessageToHub::ToHub;
use crate::message::domain::MessageToHubTypes::ServerToHub;
use crate::mqtt::domain::PayloadTopic;
use crate::network::domain::{NetworkManager};
use crate::system::domain::System;


/// Gestor de mensajes salientes hacia el Hub (Downlink Local).
///
/// Esta tarea centraliza todo lo que debe ser enviado al broker MQTT local para consumo de los nodos.
///
/// # Fuentes de Mensajes
///
/// 1.  **FSM (`rx_from_fsm`):** Mensajes de control generados internamente.
/// 2.  **Servidor (`rx_from_server`):** Comandos remotos (Settings, Firmware) que deben bajar al Hub.
/// 3.  **Red (`rx_from_network`):** Notificaciones de estado de red (Active/Inactive).
///
/// # Lógica Principal
///
/// - Verifica constantemente el estado de conexión con el broker local (`LocalStatus`).
/// - Si está desconectado, descarta los mensajes con un `warn`.
/// - Resuelve dinámicamente el tópico de destino y QoS utilizando el [`NetworkManager`].
/// - Serializa los mensajes a bytes (MessagePack/JSON) antes de enviarlos a la capa de transporte MQTT.

pub async fn msg_to_hub(tx_to_mqtt_local: mpsc::Sender<SerializedMessage>,
                        mut rx_from_fsm: mpsc::Receiver<MessageToHub>,
                        mut rx_from_server: mpsc::Receiver<MessageToHub>,
                        mut rx_from_hub: mpsc::Receiver<InternalEvent>,
                        mut rx_from_network: mpsc::Receiver<MessageToHub>,
                        mut rx_from_firmware: mpsc::Receiver<MessageToHub>,
                        app_context: AppContext) {

    let mut state = LocalStatus::Connected;
    loop {
        tokio::select! {
            Some(ToHub(msg_from_fsm)) = rx_from_fsm.recv() => {
                if matches!(state, LocalStatus::Disconnected) {
                    warn!("Mensaje FSM descartado: Broker desconectado");
                    continue;
                }

                let metadata = {
                    let manager = app_context.net_man.read().await;
                    resolve_fsm_metadata(&manager, &msg_from_fsm)
                };

                if let Some((topic, qos, retain)) = metadata {
                    match send(&tx_to_mqtt_local, topic, qos, msg_from_fsm, retain).await {
                        Ok(_) => {}
                        Err(_) => error!("Error: No se pudo serializar el mensaje de al Broker"),
                    }
                }
            }

            Some(ToHub(ServerToHub(wrapper))) = rx_from_server.recv() => {
                if matches!(state, LocalStatus::Disconnected) {
                    warn!("Mensaje del Servidor descartado: Broker desconectado");
                    continue;
                }

                match wrapper.msg {
                    MessageFromServerTypes::Settings(settings) => {
                        let topic_opt = {
                            let manager = app_context.net_man.read().await;
                            manager.get_topic_to_send_msg_from_server(&wrapper.topic_where_arrive)
                        };

                        if let Some(t) = topic_opt {
                            match send(&tx_to_mqtt_local, t.topic, t.qos, MessageFromServerTypes::Settings(settings), false).await {
                                Ok(_) => {}
                                Err(_) => error!("Error: No se pudo serializar el mensaje de Setting al Broker"),
                            }
                        }
                    },
                    MessageFromServerTypes::SettingOk(setting_ok) => {
                        let topic_opt = {
                            let manager = app_context.net_man.read().await;
                            manager.get_topic_to_send_msg_from_server(&wrapper.topic_where_arrive)
                        };

                        if let Some(t) = topic_opt {
                            match send(&tx_to_mqtt_local, t.topic, t.qos, MessageFromServerTypes::SettingOk(setting_ok), false).await {
                                Ok(_) => {}
                                Err(_) => error!("Error: No se pudo serializar el mensaje de SettingOk al Broker"),
                            }
                        }
                    },
                    _ => {},
                }
            }

            Some(msg_from_hub) = rx_from_hub.recv() => {
                match msg_from_hub {
                    InternalEvent::LocalConnected => state = LocalStatus::Connected,
                    InternalEvent::LocalDisconnected => state = LocalStatus::Disconnected,
                    _ => {},
                }
            }

            Some(msg_from_network) = rx_from_network.recv() => {
                if matches!(state, LocalStatus::Disconnected) {
                    warn!("Mensaje de Red descartado: Broker desconectado");
                    continue;
                }

                match msg_from_network {
                    ToHub(ServerToHub(msg_from_server)) => {
                        match msg_from_server.msg {
                            MessageFromServerTypes::ActiveHub(active) => {
                                let topic_opt = {
                                    let manager = app_context.net_man.read().await;
                                    manager.get_topic_to_send_msg_active_hub(&msg_from_server.topic_where_arrive)
                                };
                                if let Some(t) = topic_opt {
                                    match send(&tx_to_mqtt_local, t.topic, t.qos, MessageFromServerTypes::ActiveHub(active), false).await {
                                        Ok(_) => {}
                                        Err(_) => error!("Error: No se pudo serializar el mensaje de ActiveHub al Broker"),
                                    }
                                }
                            },
                            MessageFromServerTypes::DeleteHub(delete_hub) => {
                                let topic_opt = {
                                    let manager = app_context.net_man.read().await;
                                    manager.get_topic_to_send_msg_delete_hub(&msg_from_server.topic_where_arrive)
                                };
                                if let Some(t) = topic_opt {
                                    match send(&tx_to_mqtt_local, t.topic, t.qos, MessageFromServerTypes::DeleteHub(delete_hub), false).await {
                                        Ok(_) => {}
                                        Err(_) => error!("Error: No se pudo serializar el mensaje de DeleteHub al Broker"),
                                    }
                                }
                            },
                            _ => {},
                        }
                    },
                    _ => {},
                }
            }
            
            Some(msg_from_firmware) = rx_from_firmware.recv() => {
                match msg_from_firmware {
                    ToHub(ServerToHub(msg)) => {
                        match msg.msg {
                            MessageFromServerTypes::UpdateFirmware(update) => {
                                let topic_opt = {
                                    let manager = app_context.net_man.read().await;
                                    manager.get_topic_to_send_msg_delete_hub(&msg.topic_where_arrive)
                                };
                                if let Some(t) = topic_opt {
                                    match send(&tx_to_mqtt_local, t.topic, t.qos, MessageFromServerTypes::UpdateFirmware(update), false).await {
                                        Ok(_) => {}
                                        Err(_) => error!("Error: No se pudo serializar el mensaje de UpdateFirmware al Broker"),
                                    }
                                }
                            },
                            _ => {},
                        }
                    },
                    _ => {},
                }
            }
        }
    }
}


/// Extrae tópico, QoS y flag Retain para mensajes generados por la FSM.
fn resolve_fsm_metadata(manager: &NetworkManager,
                        msg: &MessageToHubTypes
                       ) -> Option<(String, u8, bool)> {
    match msg {
        MessageToHubTypes::Handshake(_) =>
            Some((manager.topic_handshake.topic.clone(), manager.topic_handshake.qos, false)),

        MessageToHubTypes::StateBalanceMode(_) =>
            Some((manager.topic_state.topic.clone(), manager.topic_state.qos, true)),

        MessageToHubTypes::StateNormal(_) =>
            Some((manager.topic_state.topic.clone(), manager.topic_state.qos, true)),

        MessageToHubTypes::StateSafeMode(_) =>
            Some((manager.topic_state.topic.clone(), manager.topic_state.qos, true)),

        MessageToHubTypes::Heartbeat(_) =>
            Some((manager.topic_heartbeat.topic.clone(), manager.topic_heartbeat.qos, false)),

        _ => None,
    }
}


// -------------------------------------------------------------------------------------------------


/// Procesador de mensajes entrantes del Hub (Uplink Local).
///
/// Recibe datos crudos (`PayloadTopic`) desde el broker local, los deserializa y decide su destino.
///
/// # Flujo de Decisión
///
/// 1.  **Handshakes:** Se envían siempre a la FSM para gestión de estado.
/// 2.  **Settings:** Se envían al servidor solo si hay conexión (`Connected`).
/// 3.  **Datos/Alertas/Monitores:** Se enrutan dinámicamente mediante [`route_message`]:
///     - Si hay conexión -> Al Servidor.
///     - Si NO hay conexión -> A la Base de Datos (`dba_task`).

pub async fn msg_from_hub(tx_to_hub: mpsc::Sender<InternalEvent>,
                          tx_to_server: mpsc::Sender<MessageToServer>,
                          tx_to_dba: mpsc::Sender<MessageFromHub>,
                          tx_to_fsm: mpsc::Sender<MessageFromHub>,
                          tx_to_network: mpsc::Sender<MessageFromHub>,
                          tx_to_firmware: mpsc::Sender<MessageFromHub>,
                          mut rx_from_local_mqtt: mpsc::Receiver<InternalEvent>,
                          mut rx_from_server: mpsc::Receiver<InternalEvent>) {

    let mut state: ServerStatus = ServerStatus::Connected;
    loop {
        tokio::select! {
            Some(msg) = rx_from_local_mqtt.recv() => {
                match msg {
                    InternalEvent::LocalDisconnected => {
                        if tx_to_hub.send(InternalEvent::LocalDisconnected).await.is_err() {
                            error!("Error: No se pudo enviar el mensaje LocalDisconnected a msg_to_hub");
                        }
                    },
                    InternalEvent::LocalConnected => {
                        if tx_to_hub.send(InternalEvent::LocalConnected).await.is_err() {
                            error!("Error: No se pudo enviar el mensaje LocalConnected a msg_to_hub");
                        }
                    },
                    InternalEvent::IncomingMessage(message) => {
                        handle_message(&tx_to_server, &tx_to_dba, &tx_to_fsm, &tx_to_network, &tx_to_firmware, message, &state).await;
                    },
                    _ => {},
                }
            }

            Some(msg) = rx_from_server.recv() => {
                match msg {
                    InternalEvent::ServerConnected => state = ServerStatus::Connected,
                    InternalEvent::ServerDisconnected => state = ServerStatus::Disconnected,
                    _ => {},
                }
            }
        }
    }
}


/// Deserializa y clasifica mensajes del Hub.
async fn handle_message(tx_to_server: &mpsc::Sender<MessageToServer>,
                        tx_to_dba: &mpsc::Sender<MessageFromHub>,
                        tx_to_fsm: &mpsc::Sender<MessageFromHub>,
                        tx_to_network: &mpsc::Sender<MessageFromHub>,
                        tx_to_firmware: &mpsc::Sender<MessageFromHub>,
                        message: PayloadTopic,
                        state: &ServerStatus) {

    let decoded: Result<MessageFromHubTypes, _> = from_slice(&message.payload);
    match decoded {
        Ok(MessageFromHubTypes::Handshake(handshake)) => {
            let msg = MessageFromHub::new(message.topic, MessageFromHubTypes::Handshake(handshake));
            if tx_to_fsm.send(msg).await.is_err() {
                error!("Error: No se pudo enviar el mensaje Handshake a fsm");
            }
        },
        Ok(MessageFromHubTypes::Settings(settings)) => {
            let msg = MessageFromHub::new(message.topic, MessageFromHubTypes::Settings(settings));
            if tx_to_network.send(msg.clone()).await.is_err() {
                error!("Error: No se pudo enviar el mensaje Settings a network_task");
            }
            if matches!(state, ServerStatus::Connected) {
                if tx_to_server.send(ToServer(msg)).await.is_err() {
                    error!("Error: No se pudo enviar el mensaje Settings a msg_to_server");
                }
            }
        },
        Ok(MessageFromHubTypes::SettingsOk(settings_ok)) => {
            let msg = MessageFromHub::new(message.topic, MessageFromHubTypes::SettingsOk(settings_ok));
            if tx_to_network.send(msg.clone()).await.is_err() {
                error!("Error: No se pudo enviar el mensaje SettingsOk a network_task");
            }
            if matches!(state, ServerStatus::Connected) {
                if tx_to_server.send(ToServer(msg)).await.is_err() {
                    error!("Error: No se pudo enviar el mensaje SettingsOk a msg_to_server");
                }
            }
        },
        Ok(MessageFromHubTypes::FirmwareOk(firmware_ok)) => {
            let msg = MessageFromHub::new(message.topic, MessageFromHubTypes::FirmwareOk(firmware_ok));
            if tx_to_firmware.send(msg).await.is_err() {
                error!("Error: No se pudo enviar el mensaje FirmwareOk a update_firmware_task");
            }
        },
        Ok(MessageFromHubTypes::Monitor(monitor)) => {
            let msg = MessageFromHub::new(message.topic, MessageFromHubTypes::Monitor(monitor));
            route_message(state, &tx_to_server, &tx_to_dba, msg).await;
        },
        Ok(MessageFromHubTypes::AlertAir(alert_air)) => {
            let msg = MessageFromHub::new(message.topic, MessageFromHubTypes::AlertAir(alert_air));
            route_message(state, &tx_to_server, &tx_to_dba, msg).await;
        },
        Ok(MessageFromHubTypes::AlertTem(alert_temp)) => {
            let msg = MessageFromHub::new(message.topic, MessageFromHubTypes::AlertTem(alert_temp));
            route_message(state, &tx_to_server, &tx_to_dba, msg).await;
        },
        Ok(MessageFromHubTypes::Report(report)) => {
            let msg = MessageFromHub::new(message.topic, MessageFromHubTypes::Report(report));
            route_message(state, &tx_to_server, &tx_to_dba, msg).await;
        },
        _ => {},
    }
}


/// Enrutador inteligente Store & Forward.
/// Decide entre envío directo (Servidor) o persistencia (DB) según el estado.
async fn route_message(state: &ServerStatus,
                       tx_to_server: &mpsc::Sender<MessageToServer>,
                       tx_to_dba: &mpsc::Sender<MessageFromHub>,
                       msg: MessageFromHub) {

    if matches!(state, ServerStatus::Connected) {
        if tx_to_server.send(ToServer(msg)).await.is_err() {
            error!("Error: No se pudo enviar el mensaje de msg_from_hub a msg_to_server");
        }
    } else {
        if tx_to_dba.send(msg).await.is_err() {
            error!("Error: No se pudo enviar el mensaje de msg_from_hub a dba_task");
        }
    }
}


// -------------------------------------------------------------------------------------------------


/// Gestor de mensajes salientes hacia el Servidor (Uplink Remoto).
///
/// Centraliza el envío de datos a la nube, gestionando tanto el flujo en tiempo real como los datos
/// recuperados de la base de datos.
///
/// # Fuentes
///
/// - **Hub (`rx_from_hub`):** Datos en tiempo real que pasaron el filtro de conexión en `msg_from_hub`.
/// - **DB (`rx_from_dba_batch`):** Lotes de datos históricos recuperados tras una desconexión.
///
/// # Funcionalidad
///
/// Utiliza `get_topic_to_send_msg_from_hub` para transformar el tópico local en un tópico global
/// con identidad de Edge.

pub async fn msg_to_server(tx_to_server: mpsc::Sender<SerializedMessage>,
                           mut rx_from_fsm: mpsc::Receiver<MessageToServer>,
                           mut rx_from_hub: mpsc::Receiver<MessageToServer>,
                           mut rx_from_firmware: mpsc::Receiver<MessageToServer>,
                           mut rx_from_dba_batch: mpsc::Receiver<TableDataVector>,
                           mut rx_from_server: mpsc::Receiver<InternalEvent>,
                           app_context: AppContext) {

    let mut state = ServerStatus::Connected;
    loop {
        tokio::select! {
            Some(_) = rx_from_fsm.recv() => {

            },

            Some(msg_from_hub) = rx_from_hub.recv() => {
                if matches!(state, ServerStatus::Connected) {
                    match msg_from_hub {
                        ToServer(msg) => {
                            let manager = app_context.net_man.read().await;
                            if let Some(topic_out) = manager.get_topic_to_send_msg_from_hub(&msg.topic_where_arrive, &app_context.system) {
                                drop(manager);
                                match send(&tx_to_server, topic_out.topic, topic_out.qos, msg, false).await {
                                    Ok(_) => {},
                                    Err(_) => error!("Error: No se pudo enviar el mensaje al Server"),
                                }
                            }
                        }
                    }
                }
            }

            Some(msg_from_dba_batch) = rx_from_dba_batch.recv() => {
                if matches!(state, ServerStatus::Connected) {
                    let manager = app_context.net_man.read().await;
                    match process_batch(msg_from_dba_batch, &tx_to_server, &manager, &app_context.system).await {
                        Ok(_) => {},
                        Err(_) => error!("Error: No se pudo enviar el batch al Server"),
                    }
                }
            }

            Some(msg_from_server) = rx_from_server.recv() => {
                match msg_from_server {
                    InternalEvent::ServerConnected => state = ServerStatus::Connected,
                    InternalEvent::ServerDisconnected => state = ServerStatus::Disconnected,
                    _ => {},
                }
            }
            
            Some(msg_from_firmware) = rx_from_firmware.recv() => {
                if matches!(state, ServerStatus::Connected) {
                    match msg_from_firmware {
                        ToServer(msg) => {
                            let manager = app_context.net_man.read().await;
                            if let Some(topic_out) = manager.get_topic_to_send_msg_from_hub(&msg.topic_where_arrive, &app_context.system) {
                                drop(manager);
                                match send(&tx_to_server, topic_out.topic, topic_out.qos, msg, false).await {
                                    Ok(_) => {},
                                    Err(_) => error!("Error: No se pudo enviar el mensaje al Server"),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}


/// Itera sobre un vector de datos históricos y los envía uno a uno.
async fn process_batch(batch: TableDataVector,
                       tx: &mpsc::Sender<SerializedMessage>,
                       manager: &NetworkManager,
                       system: &System
                      ) -> Result<(), ()> {

    match batch.vec {
        TableDataVectorTypes::Measurement(vec) => {
            for row in vec {
                let topic = row.metadata.topic_where_arrive.clone();
                let payload = MessageFromHubTypes::Report(row.cast_measurement());
                dispatch_to_server(&topic, payload, tx, manager, system).await;
            }
        },
        TableDataVectorTypes::Monitor(vec) => {
            for row in vec {
                let topic = row.metadata.topic_where_arrive.clone();
                let payload = MessageFromHubTypes::Monitor(row.cast_monitor());
                dispatch_to_server(&topic, payload, tx, manager, system).await;
            }
        },
        TableDataVectorTypes::AlertAir(vec) => {
            for row in vec {
                let topic = row.metadata.topic_where_arrive.clone();
                let payload = MessageFromHubTypes::AlertAir(row.cast_alert_air());
                dispatch_to_server(&topic, payload, tx, manager, system).await;
            }
        },
        TableDataVectorTypes::AlertTemp(vec) => {
            for row in vec {
                let topic = row.metadata.topic_where_arrive.clone();
                let payload = MessageFromHubTypes::AlertTem(row.cast_alert_th());
                dispatch_to_server(&topic, payload, tx, manager, system).await;
            }
        },
    }
    Ok(())
}


/// Auxiliar para resolver tópico de salida y enviar.
async fn dispatch_to_server(topic_in: &str,
                            payload: MessageFromHubTypes,
                            tx: &mpsc::Sender<SerializedMessage>,
                            manager: &NetworkManager,
                            system: &System) {

    if let Some(topic_out) = manager.get_topic_to_send_msg_from_hub(topic_in, system) {
        if send(tx, topic_out.topic, topic_out.qos, payload, false).await.is_err() {
            error!("Error: No se pudo enviar el mensaje al Server");
        }
    }
}


// -------------------------------------------------------------------------------------------------


/// Procesador de mensajes entrantes del Servidor (Downlink Remoto).
///
/// Recibe eventos crudos del servidor y los clasifica en:
/// - Mensajes de negocio (`IncomingMessage`): Deserializa y enruta al Hub o Gestor de Redes.
/// - Eventos de conexión (`ServerConnected/Disconnected`): Notifica a DBA y Hub.

pub async fn msg_from_server(_tx_to_fsm: mpsc::Sender<MessageFromServer>,
                             tx_to_hub: mpsc::Sender<MessageToHub>,
                             tx_to_network: mpsc::Sender<MessageFromServer>,
                             tx_to_dba: mpsc::Sender<InternalEvent>,
                             tx_to_from_hub: mpsc::Sender<InternalEvent>,
                             tx_to_server: mpsc::Sender<InternalEvent>,
                             tx_to_firmware: mpsc::Sender<MessageFromServer>,
                             mut rx_from_server: mpsc::Receiver<InternalEvent>) {

    while let Some(msg_from_server) = rx_from_server.recv().await {
        match msg_from_server {
            InternalEvent::IncomingMessage(msg_data) => {
                handle_incoming_message(msg_data, &tx_to_hub, &tx_to_network, &tx_to_firmware).await;
            },
            InternalEvent::ServerConnected | InternalEvent::ServerDisconnected => {
                if tx_to_dba.send(msg_from_server.clone()).await.is_err() {
                    error!("Error: Receptor DBA no disponible");
                }
                if tx_to_from_hub.send(msg_from_server.clone()).await.is_err() {
                    error!("Error: Receptor FromHub no disponible");
                }
                if tx_to_server.send(msg_from_server).await.is_err() {
                    error!("Error: Receptor FromHub no disponible");
                }
            },
            _ => {}
        }
    }
}


/// Deserializa y distribuye mensajes del servidor a su destino.
async fn handle_incoming_message(message: PayloadTopic,
                                 tx_to_hub: &mpsc::Sender<MessageToHub>,
                                 tx_to_network: &mpsc::Sender<MessageFromServer>,
                                 tx_to_firmware: &mpsc::Sender<MessageFromServer>) {

    let decoded: Result<MessageFromServerTypes, _> = from_slice(&message.payload);
    match decoded {
        Ok(payload) => {
            let msg_wrapper = MessageFromServer::new(message.topic, payload.clone());

            match payload {
                MessageFromServerTypes::SettingOk(_) => {
                    let to_hub_msg = ToHub(MessageToHubTypes::ServerToHub(msg_wrapper));
                    if tx_to_hub.send(to_hub_msg).await.is_err() {
                        error!("Error: Receptor no disponible, mensaje descartado");
                    }
                },
                MessageFromServerTypes::Network(_) | MessageFromServerTypes::Settings(_) |
                MessageFromServerTypes::DeleteHub(_) => {
                    if tx_to_network.send(msg_wrapper).await.is_err() {
                        error!("Error: Receptor no disponible, mensaje descartado");
                    }
                },
                MessageFromServerTypes::UpdateFirmware(_firmware) => {
                    if tx_to_firmware.send(msg_wrapper).await.is_err() {
                        error!("Error: Receptor no disponible, mensaje descartado");
                    }
                },
                _ => {},
            }
        },
        Err(e) => error!("Error deserializando mensaje del servidor: {}", e),
    }
}


// -------------------------------------------------------------------------------------------------


/// Utilidad genérica de serialización y envío.
///
/// Serializa cualquier estructura a bytes (MessagePack) y la empaqueta en un
/// `SerializedMessage` listo para ser consumido por el cliente MQTT.

pub async fn send<T>(tx: &mpsc::Sender<SerializedMessage>,
                     topic: String,
                     qos: u8,
                     msg: T,
                     retain: bool
                    ) -> Result<(), ()>
where
    T: Serialize,
{
    match to_vec(&msg) {
        Ok(payload) => {
            let serialized = SerializedMessage::new(topic, payload, qos, retain);
            if tx.send(serialized).await.is_err() {
                return Err(());
            }
        }
        Err(e) => {
            error!("Error serializando mensaje: {:?}", e);
        }
    }
    Ok(())
}