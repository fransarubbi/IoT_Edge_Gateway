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
use tracing::{error, instrument, warn};
use crate::context::domain::AppContext;
use crate::message::domain::{SerializedMessage, ServerStatus, LocalStatus, Message, MessageTypes};
use crate::database::domain::{TableDataVector, TableDataVectorTypes};
use crate::mqtt::domain::PayloadTopic;
use crate::network::domain::{NetworkManager};
use crate::system::domain::{InternalEvent, System};


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

#[instrument(name = "msg_to_hub", skip(app_context))]
pub async fn msg_to_hub(tx_to_mqtt_local: mpsc::Sender<SerializedMessage>,
                        mut rx_from_fsm: mpsc::Receiver<Message>,
                        mut rx_from_server: mpsc::Receiver<Message>,
                        mut rx_from_hub: mpsc::Receiver<InternalEvent>,
                        mut rx_from_network: mpsc::Receiver<Message>,
                        mut rx_from_firmware: mpsc::Receiver<Message>,
                        app_context: AppContext) {

    let mut state = LocalStatus::Connected;
    loop {
        tokio::select! {
            Some(msg_from_hub) = rx_from_hub.recv() => {
                match msg_from_hub {
                    InternalEvent::LocalConnected => state = LocalStatus::Connected,
                    InternalEvent::LocalDisconnected => state = LocalStatus::Disconnected,
                    _ => {},
                }
            }

            Some(msg_from_fsm) = rx_from_fsm.recv() => {
                if matches!(state, LocalStatus::Disconnected) {
                    warn!("Warning: Mensaje FSM descartado, broker desconectado");
                    continue;
                }

                let topic_out = {
                    let manager = app_context.net_man.read().await;
                    resolve_fsm_topic(&manager, &msg_from_fsm.get_message())
                };

                if let Some((topic, qos, retain)) = topic_out {
                    match send(&tx_to_mqtt_local, topic, qos, msg_from_fsm, retain).await {
                        Ok(_) => {}
                        Err(_) => error!("Error: No se pudo serializar el mensaje de la FSM al broker"),
                    }
                }
            }

            Some(msg_from_server) = rx_from_server.recv() => {
                if matches!(state, LocalStatus::Disconnected) {
                    warn!("Warning: Mensaje del servidor descartado, broker desconectado");
                    continue;
                }

                let topic_opt = {
                    let manager = app_context.net_man.read().await;
                    manager.get_topic_to_send_msg_from_server(&msg_from_server.get_topic_arrive())
                };

                if let Some(t) = topic_opt {
                    match send(&tx_to_mqtt_local, t.topic, t.qos, msg_from_server, false).await {
                        Ok(_) => {}
                        Err(_) => error!("Error: No se pudo serializar el mensaje del servidor al broker"),
                    }
                }
            }

            Some(msg_from_network) = rx_from_network.recv() => {
                if matches!(state, LocalStatus::Disconnected) {
                    warn!("Warning: Mensaje de network descartado, broker desconectado");
                    continue;
                }

                let topic_opt = {
                    let manager = app_context.net_man.read().await;
                    manager.get_topic_to_send_msg_from_network(&msg_from_network.get_topic_arrive())
                };

                if let Some(t) = topic_opt {
                    match send(&tx_to_mqtt_local, t.topic, t.qos, msg_from_network, false).await {
                        Ok(_) => {}
                        Err(_) => error!("Error: No se pudo serializar el mensaje de network al broker"),
                    }
                }
            }
            
            Some(msg_from_firmware) = rx_from_firmware.recv() => {
                if matches!(state, LocalStatus::Disconnected) {
                    warn!("Warning: Mensaje de firmware descartado, broker desconectado");
                    continue;
                }

                let topic_opt = {
                    let manager = app_context.net_man.read().await;
                    manager.get_topic_to_send_msg_from_server(&msg_from_firmware.get_topic_arrive())
                };

                if let Some(t) = topic_opt {
                    match send(&tx_to_mqtt_local, t.topic, t.qos, msg_from_firmware, false).await {
                        Ok(_) => {}
                        Err(_) => error!("Error: No se pudo serializar el mensaje de UpdateFirmware al broker"),
                    }
                }
            }
        }
    }
}


/// Extrae tópico, QoS y flag Retain para mensajes generados por la FSM.
fn resolve_fsm_topic(manager: &NetworkManager,
                        msg: &MessageTypes
                       ) -> Option<(String, u8, bool)> {
    match msg {
        MessageTypes::HandshakeToHub(_) =>
            Some((manager.topic_handshake.topic.clone(), manager.topic_handshake.qos, false)),

        MessageTypes::StateBalanceMode(_) =>
            Some((manager.topic_state.topic.clone(), manager.topic_state.qos, true)),

        MessageTypes::StateNormal(_) =>
            Some((manager.topic_state.topic.clone(), manager.topic_state.qos, true)),

        MessageTypes::StateSafeMode(_) =>
            Some((manager.topic_state.topic.clone(), manager.topic_state.qos, true)),

        MessageTypes::Heartbeat(_) =>
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
                          tx_to_server: mpsc::Sender<Message>,
                          tx_to_dba: mpsc::Sender<Message>,
                          tx_to_fsm: mpsc::Sender<Message>,
                          tx_to_network: mpsc::Sender<Message>,
                          tx_to_firmware: mpsc::Sender<Message>,
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
async fn handle_message(tx_to_server: &mpsc::Sender<Message>,
                        tx_to_dba: &mpsc::Sender<Message>,
                        tx_to_fsm: &mpsc::Sender<Message>,
                        tx_to_network: &mpsc::Sender<Message>,
                        tx_to_firmware: &mpsc::Sender<Message>,
                        message: PayloadTopic,
                        state: &ServerStatus) {

    let decoded: Result<MessageTypes, _> = from_slice(&message.payload);
    match decoded {
        Ok(MessageTypes::HandshakeFromHub(handshake)) => {
            let msg = Message::new(message.topic, MessageTypes::HandshakeFromHub(handshake));
            if tx_to_fsm.send(msg).await.is_err() {
                error!("Error: No se pudo enviar el mensaje Handshake a la FSM");
            }
        },
        Ok(MessageTypes::FromHubSettings(settings)) => {
            let msg = Message::new(message.topic, MessageTypes::FromHubSettings(settings));
            if tx_to_network.send(msg.clone()).await.is_err() {
                error!("Error: No se pudo enviar el mensaje Settings a network");
            }
            if matches!(state, ServerStatus::Connected) {
                if tx_to_server.send(msg).await.is_err() {
                    error!("Error: No se pudo enviar el mensaje Settings a msg_to_server");
                }
            }
        },
        Ok(MessageTypes::FromHubSettingsAck(settings_ok)) => {
            let msg = Message::new(message.topic, MessageTypes::FromHubSettingsAck(settings_ok));
            if tx_to_network.send(msg.clone()).await.is_err() {
                error!("Error: No se pudo enviar el mensaje SettingsOk a network");
            }
            if matches!(state, ServerStatus::Connected) {
                if tx_to_server.send(msg).await.is_err() {
                    error!("Error: No se pudo enviar el mensaje SettingsOk a msg_to_server");
                }
            }
        },
        Ok(MessageTypes::FirmwareOk(firmware_ok)) => {
            let msg = Message::new(message.topic, MessageTypes::FirmwareOk(firmware_ok));
            if tx_to_firmware.send(msg).await.is_err() {
                error!("Error: No se pudo enviar el mensaje FirmwareOk a update_firmware_task");
            }
        },
        Ok(MessageTypes::Monitor(monitor)) => {
            let msg = Message::new(message.topic, MessageTypes::Monitor(monitor));
            route_message(state, &tx_to_server, &tx_to_dba, msg).await;
        },
        Ok(MessageTypes::AlertAir(alert_air)) => {
            let msg = Message::new(message.topic, MessageTypes::AlertAir(alert_air));
            route_message(state, &tx_to_server, &tx_to_dba, msg).await;
        },
        Ok(MessageTypes::AlertTem(alert_temp)) => {
            let msg = Message::new(message.topic, MessageTypes::AlertTem(alert_temp));
            route_message(state, &tx_to_server, &tx_to_dba, msg).await;
        },
        Ok(MessageTypes::Report(report)) => {
            let msg = Message::new(message.topic, MessageTypes::Report(report));
            route_message(state, &tx_to_server, &tx_to_dba, msg).await;
        },
        _ => {},
    }
}


/// Enrutador inteligente Store & Forward.
/// Decide entre envío directo (Servidor) o persistencia (DB) según el estado.
async fn route_message(state: &ServerStatus,
                       tx_to_server: &mpsc::Sender<Message>,
                       tx_to_dba: &mpsc::Sender<Message>,
                       msg: Message) {

    if matches!(state, ServerStatus::Connected) {
        if tx_to_server.send(msg).await.is_err() {
            error!("Error: No se pudo enviar el mensaje de msg_from_hub a msg_to_server");
        }
    } else {
        if tx_to_dba.send(msg).await.is_err() {
            error!("Error: No se pudo enviar el mensaje de msg_from_hub a data_dba_admin");
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
                           mut rx_from_fsm: mpsc::Receiver<Message>,
                           mut rx_from_hub: mpsc::Receiver<Message>,
                           mut rx_from_firmware: mpsc::Receiver<Message>,
                           mut rx_from_dba_batch: mpsc::Receiver<TableDataVector>,
                           mut rx_from_server: mpsc::Receiver<InternalEvent>,
                           app_context: AppContext) {

    let mut state = ServerStatus::Connected;
    loop {
        tokio::select! {
            Some(msg_from_server) = rx_from_server.recv() => {
                match msg_from_server {
                    InternalEvent::ServerConnected => state = ServerStatus::Connected,
                    InternalEvent::ServerDisconnected => state = ServerStatus::Disconnected,
                    _ => {},
                }
            }

            Some(msg_from_hub) = rx_from_hub.recv() => {
                if matches!(state, ServerStatus::Disconnected) {
                    warn!("Warning: Mensaje del hub descartado, servidor desconectado");
                    continue;
                }
                let manager = app_context.net_man.read().await;
                if let Some(topic) = manager.get_topic_to_send_msg_from_hub(msg_from_hub.get_topic_arrive(), &app_context.system.id_edge) {
                    match send(&tx_to_server, topic.topic, topic.qos, msg_from_hub, false).await {
                        Ok(_) => {}
                        Err(_) => error!("Error: No se pudo serializar el mensaje de FirmwareOutcome al servidor"),
                    }
                }
            }

            Some(msg_from_fsm) = rx_from_fsm.recv() => {
                if matches!(state, ServerStatus::Disconnected) {
                    warn!("Warning: Mensaje FSM descartado, servidor desconectado");
                    continue;
                }
                let manager = app_context.net_man.read().await;
                let topic = manager.topic_state.topic.clone();
                let qos = manager.topic_state.qos;
                match send(&tx_to_server, topic, qos, msg_from_fsm, true).await {
                    Ok(_) => {},
                    Err(_) => error!("Error: No se pudo serializar el mensaje al servidor"),
                }
            },

            Some(msg_from_dba_batch) = rx_from_dba_batch.recv() => {
                if matches!(state, ServerStatus::Disconnected) {
                    warn!("Warning: Mensaje batch descartado, servidor desconectado");
                    continue;
                }
                let manager = app_context.net_man.read().await;
                match process_batch(msg_from_dba_batch, &tx_to_server, &manager, &app_context.system).await {
                    Ok(_) => {},
                    Err(_) => error!("Error: No se pudo serializar el batch al servidor"),
                }
            }

            Some(msg_from_firmware) = rx_from_firmware.recv() => {
                if matches!(state, ServerStatus::Disconnected) {
                    warn!("Warning: Mensaje firmware descartado, servidor desconectado");
                    continue;
                }
                let manager = app_context.net_man.read().await;
                if let Some(topic) = manager.get_topic_to_send_firmware_ok(msg_from_firmware.get_topic_arrive(), &app_context.system.id_edge) {
                    match send(&tx_to_server, topic.topic, topic.qos, msg_from_firmware, false).await {
                        Ok(_) => {}
                        Err(_) => error!("Error: No se pudo serializar el mensaje de FirmwareOutcome al servidor"),
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
                let payload = MessageTypes::Report(row.cast_measurement());
                dispatch_to_server(&topic, payload, tx, manager, system).await;
            }
        },
        TableDataVectorTypes::Monitor(vec) => {
            for row in vec {
                let topic = row.metadata.topic_where_arrive.clone();
                let payload = MessageTypes::Monitor(row.cast_monitor());
                dispatch_to_server(&topic, payload, tx, manager, system).await;
            }
        },
        TableDataVectorTypes::AlertAir(vec) => {
            for row in vec {
                let topic = row.metadata.topic_where_arrive.clone();
                let payload = MessageTypes::AlertAir(row.cast_alert_air());
                dispatch_to_server(&topic, payload, tx, manager, system).await;
            }
        },
        TableDataVectorTypes::AlertTemp(vec) => {
            for row in vec {
                let topic = row.metadata.topic_where_arrive.clone();
                let payload = MessageTypes::AlertTem(row.cast_alert_th());
                dispatch_to_server(&topic, payload, tx, manager, system).await;
            }
        },
    }
    Ok(())
}


/// Auxiliar para resolver tópico de salida y enviar.
async fn dispatch_to_server(topic_in: &str,
                            payload: MessageTypes,
                            tx: &mpsc::Sender<SerializedMessage>,
                            manager: &NetworkManager,
                            system: &System) {

    if let Some(topic_out) = manager.get_topic_to_send_msg_from_hub(topic_in, &system.id_edge) {
        if send(tx, topic_out.topic, topic_out.qos, payload, false).await.is_err() {
            error!("Error: No se pudo serializar el mensaje al servidor");
        }
    }
}


// -------------------------------------------------------------------------------------------------


/// Procesador de mensajes entrantes del Servidor (Downlink Remoto).
///
/// Recibe eventos crudos del servidor y los clasifica en:
/// - Mensajes de negocio (`IncomingMessage`): Deserializa y enruta al Hub o Gestor de Redes.
/// - Eventos de conexión (`ServerConnected/Disconnected`): Notifica a DBA y Hub.

pub async fn msg_from_server(tx_to_hub: mpsc::Sender<Message>,
                             tx_to_network: mpsc::Sender<Message>,
                             tx_to_dba: mpsc::Sender<InternalEvent>,
                             tx_to_from_hub: mpsc::Sender<InternalEvent>,
                             tx_to_server: mpsc::Sender<InternalEvent>,
                             tx_to_firmware: mpsc::Sender<Message>,
                             mut rx_from_server: mpsc::Receiver<InternalEvent>) {

    while let Some(msg_from_server) = rx_from_server.recv().await {
        match msg_from_server {
            InternalEvent::IncomingMessage(msg_data) => {
                handle_incoming_message(msg_data, &tx_to_hub, &tx_to_network, &tx_to_firmware).await;
            },
            InternalEvent::ServerConnected | InternalEvent::ServerDisconnected => {
                if tx_to_dba.send(msg_from_server.clone()).await.is_err() {
                    error!("Error: No se pudo enviar mensaje de ServerConnected/ServerDisconnected");
                }
                if tx_to_from_hub.send(msg_from_server.clone()).await.is_err() {
                    error!("Error: No se pudo enviar mensaje de ServerConnected/ServerDisconnected");
                }
                if tx_to_server.send(msg_from_server).await.is_err() {
                    error!("Error: No se pudo enviar mensaje de ServerConnected/ServerDisconnected");
                }
            },
            _ => {}
        }
    }
}


/// Deserializa y distribuye mensajes del servidor a su destino.
async fn handle_incoming_message(message: PayloadTopic,
                                 tx_to_hub: &mpsc::Sender<Message>,
                                 tx_to_network: &mpsc::Sender<Message>,
                                 tx_to_firmware: &mpsc::Sender<Message>) {

    let decoded: Result<MessageTypes, _> = from_slice(&message.payload);
    match decoded {
        Ok(payload) => {
            let msg = Message::new(message.topic, payload.clone());

            match payload {
                MessageTypes::FromServerSettingsAck(_) => {
                    if tx_to_hub.send(msg).await.is_err() {
                        error!("Error: No se pudo enviar mensaje al hub");
                    }
                },
                MessageTypes::Network(_) | MessageTypes::FromServerSettings(_) |
                MessageTypes::DeleteHub(_) => {
                    if tx_to_network.send(msg).await.is_err() {
                        error!("Error: No se pudo enviar mensaje a network");
                    }
                },
                MessageTypes::UpdateFirmware(_) => {
                    if tx_to_firmware.send(msg).await.is_err() {
                        error!("Error: No se pudo enviar mensaje UpdateFirmware a firmware");
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