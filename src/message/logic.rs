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
use crate::message::domain::{SerializedMessage, ServerStatus, LocalStatus, Message, Metadata, UpdateFirmware, DeleteHub, Settings, SettingOk, Network};
use crate::database::domain::{TableDataVector, TableDataVectorTypes};
use crate::grpc;
use crate::grpc::{edge_download, AlertAir, AlertTh, EdgeDownload, EdgeUpload, EdgeUploadToDataSaver, EdgeUploadToManager, FirmwareOutcome, Heartbeat, Measurement, Monitor, SettingOk as SettOk, StateBalanceMode, StateNormal, StateSafeMode, Settings as Sett};
use crate::grpc::edge_upload::Payload;
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
                    resolve_fsm_topic(&manager, &msg_from_fsm)
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
                    manager.get_topic_to_send_msg_to_hub(&msg_from_server)
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
                    manager.get_topic_to_send_msg_to_hub(&msg_from_network)
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
                    manager.get_topic_to_send_msg_to_hub(&msg_from_firmware)
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
                     msg: &Message
) -> Option<(String, u8, bool)> {
    match msg {
        Message::HandshakeToHub(_) =>
            Some((manager.topic_handshake.topic.clone(), manager.topic_handshake.qos, false)),

        Message::StateBalanceMode(_) =>
            Some((manager.topic_state.topic.clone(), manager.topic_state.qos, true)),

        Message::StateNormal(_) =>
            Some((manager.topic_state.topic.clone(), manager.topic_state.qos, true)),

        Message::StateSafeMode(_) =>
            Some((manager.topic_state.topic.clone(), manager.topic_state.qos, true)),

        Message::Heartbeat(_) =>
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

    let decoded: Result<Message, _> = from_slice(&message.payload);
    match decoded {
        Ok(Message::HandshakeFromHub(handshake)) => {
            if tx_to_fsm.send(Message::HandshakeFromHub(handshake)).await.is_err() {
                error!("Error: No se pudo enviar el mensaje Handshake a la FSM");
            }
        },
        Ok(Message::FromHubSettings(settings)) => {
            let msg = Message::FromHubSettings(settings);
            if tx_to_network.send(msg.clone()).await.is_err() {
                error!("Error: No se pudo enviar el mensaje Settings a network");
            }
            if matches!(state, ServerStatus::Connected) {
                if tx_to_server.send(msg).await.is_err() {
                    error!("Error: No se pudo enviar el mensaje Settings a msg_to_server");
                }
            }
        },
        Ok(Message::FromHubSettingsAck(settings_ok)) => {
            let msg = Message::FromHubSettingsAck(settings_ok);
            if tx_to_network.send(msg.clone()).await.is_err() {
                error!("Error: No se pudo enviar el mensaje SettingsOk a network");
            }
            if matches!(state, ServerStatus::Connected) {
                if tx_to_server.send(msg).await.is_err() {
                    error!("Error: No se pudo enviar el mensaje SettingsOk a msg_to_server");
                }
            }
        },
        Ok(Message::FirmwareOk(firmware_ok)) => {
            let msg = Message::FirmwareOk(firmware_ok);
            if tx_to_firmware.send(msg).await.is_err() {
                error!("Error: No se pudo enviar el mensaje FirmwareOk a update_firmware_task");
            }
        },
        Ok(Message::Monitor(monitor)) => {
            let msg = Message::Monitor(monitor);
            route_message(state, &tx_to_server, &tx_to_dba, msg).await;
        },
        Ok(Message::AlertAir(alert_air)) => {
            let msg = Message::AlertAir(alert_air);
            route_message(state, &tx_to_server, &tx_to_dba, msg).await;
        },
        Ok(Message::AlertTem(alert_temp)) => {
            let msg = Message::AlertTem(alert_temp);
            route_message(state, &tx_to_server, &tx_to_dba, msg).await;
        },
        Ok(Message::Report(report)) => {
            let msg = Message::Report(report);
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

pub async fn msg_to_server(tx_to_server: mpsc::Sender<EdgeUpload>,
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
                if let Some(proto_msg) = convert_to_proto_upload_data_saver(msg_from_hub, app_context.system.id_edge.clone()) {
                    if tx_to_server.send(proto_msg).await.is_err() {
                         error!("Error: No se puede enviar mensaje EdgeUpload al cliente gRPC");
                    }
                }
            }

            Some(msg_from_fsm) = rx_from_fsm.recv() => {
                if matches!(state, ServerStatus::Disconnected) {
                    warn!("Warning: Mensaje FSM descartado, servidor desconectado");
                    continue;
                }
                if let Some(proto_msg) = convert_to_proto_upload_manager(msg_from_fsm, app_context.system.id_edge.clone()) {
                    if tx_to_server.send(proto_msg).await.is_err() {
                         error!("Error: No se puede enviar mensaje EdgeUpload al cliente gRPC");
                    }
                }
            },

            Some(msg_from_dba_batch) = rx_from_dba_batch.recv() => {
                if matches!(state, ServerStatus::Disconnected) {
                    warn!("Warning: Mensaje batch descartado, servidor desconectado");
                    continue;
                }
                match process_batch(msg_from_dba_batch, &tx_to_server, &app_context.system).await {
                    Ok(_) => {},
                    Err(_) => error!("Error: Fallo en envío de batch al cliente gRPC"),
                }
            }

            Some(msg_from_firmware) = rx_from_firmware.recv() => {
                if matches!(state, ServerStatus::Disconnected) {
                    warn!("Warning: Mensaje firmware descartado, servidor desconectado");
                    continue;
                }
                if let Some(proto_msg) = convert_to_proto_upload_manager(msg_from_firmware, app_context.system.id_edge.clone()) {
                    if tx_to_server.send(proto_msg).await.is_err() {
                         error!("Error: No se puede enviar mensaje EdgeUpload al cliente gRPC");
                    }
                }
            }
        }
    }
}


fn convert_to_proto_upload_data_saver(msg: Message, edge_id: String) -> Option<EdgeUpload> {
    let mut metadata = Metadata::default();

    let payload = match msg {
        Message::Report(report) => {
            metadata.sender_user_id = report.metadata.sender_user_id;
            metadata.destination_id = report.metadata.destination_id;
            metadata.timestamp = report.metadata.timestamp;
            Some(Payload::ToDataSaver(EdgeUploadToDataSaver {
                payload: Some(grpc::edge_upload_to_data_saver::Payload::Measurement(
                    Measurement {
                        pulse_counter: report.pulse_counter,
                        pulse_max_duration: report.pulse_max_duration,
                        temperature: report.temperature,
                        humidity: report.humidity,
                        co2_ppm: report.co2_ppm,
                        sample: report.sample as u32,
                    }
                ))
            }))
        },
        Message::Monitor(monitor) => {
            metadata.sender_user_id = monitor.metadata.sender_user_id;
            metadata.destination_id = monitor.metadata.destination_id;
            metadata.timestamp = monitor.metadata.timestamp;
            Some(Payload::ToDataSaver(EdgeUploadToDataSaver {
                payload: Some(grpc::edge_upload_to_data_saver::Payload::Monitor(
                    Monitor {
                        mem_free: monitor.mem_free,
                        mem_free_hm: monitor.mem_free_hm,
                        mem_free_block: monitor.mem_free_block,
                        mem_free_internal: monitor.mem_free_internal,
                        stack_free_min_coll: monitor.stack_free_min_coll,
                        stack_free_min_pub: monitor.stack_free_min_pub,
                        stack_free_min_mic: monitor.stack_free_min_mic,
                        stack_free_min_th: monitor.stack_free_min_th,
                        stack_free_min_air: monitor.stack_free_min_air,
                        stack_free_min_mon: monitor.stack_free_min_mon,
                        wifi_ssid: monitor.wifi_ssid,
                        wifi_rssi: monitor.wifi_rssi as i32,
                        active_time: monitor.active_time,
                    }
                ))
            }))
        },
        Message::AlertAir(alert_air) => {
            metadata.sender_user_id = alert_air.metadata.sender_user_id;
            metadata.destination_id = alert_air.metadata.destination_id;
            metadata.timestamp = alert_air.metadata.timestamp;
            Some(Payload::ToDataSaver(EdgeUploadToDataSaver {
                payload: Some(grpc::edge_upload_to_data_saver::Payload::AlertAir(
                    AlertAir {
                        co2_initial_ppm: alert_air.co2_initial_ppm,
                        co2_actual_ppm: alert_air.co2_actual_ppm,
                    }
                ))
            }))
        },

        Message::AlertTem(alert_tem) => {
            metadata.sender_user_id = alert_tem.metadata.sender_user_id;
            metadata.destination_id = alert_tem.metadata.destination_id;
            metadata.timestamp = alert_tem.metadata.timestamp;
            Some(Payload::ToDataSaver(EdgeUploadToDataSaver {
                payload: Some(grpc::edge_upload_to_data_saver::Payload::AlertTh(
                    AlertTh {
                        initial_temp: alert_tem.initial_temp,
                        actual_temp: alert_tem.actual_temp,
                    }
                ))
            }))
        },
        _ => None,
    };

    generate_edge_upload(payload, metadata, edge_id.to_string())
}


fn convert_to_proto_upload_manager(msg: Message, edge_id: String) -> Option<EdgeUpload> {
    let mut metadata = Metadata::default();

    let payload = match msg {
        Message::FromHubSettings(hub_settings) => {
            metadata.sender_user_id = hub_settings.metadata.sender_user_id;
            metadata.destination_id = hub_settings.metadata.destination_id;
            metadata.timestamp = hub_settings.metadata.timestamp;
            Some(Payload::ToManager(EdgeUploadToManager {
                payload: Some(grpc::edge_upload_to_manager::Payload::Settings(
                    Sett {
                        network: hub_settings.network,
                        wifi_ssid: hub_settings.wifi_ssid,
                        wifi_password: hub_settings.wifi_password,
                        mqtt_uri: hub_settings.mqtt_uri,
                        device_name: hub_settings.device_name,
                        sample: hub_settings.sample,
                        energy_mode: hub_settings.energy_mode,
                    }
                ))
            }))
        },
        Message::FromHubSettingsAck(hub_settings_ack) => {
            metadata.sender_user_id = hub_settings_ack.metadata.sender_user_id;
            metadata.destination_id = hub_settings_ack.metadata.destination_id;
            metadata.timestamp = hub_settings_ack.metadata.timestamp;
            Some(Payload::ToManager(EdgeUploadToManager {
                payload: Some(grpc::edge_upload_to_manager::Payload::SettingOk(
                    SettOk {
                        network: hub_settings_ack.network,
                        handshake: hub_settings_ack.handshake,
                    }
                ))
            }))
        },
        Message::StateBalanceMode(balance_mode) => {
            metadata.sender_user_id = balance_mode.metadata.sender_user_id;
            metadata.destination_id = balance_mode.metadata.destination_id;
            metadata.timestamp = balance_mode.metadata.timestamp;
            Some(Payload::ToManager(EdgeUploadToManager {
                payload: Some(grpc::edge_upload_to_manager::Payload::BalanceMode(
                    StateBalanceMode {
                        state: balance_mode.state,
                        balance_epoch: balance_mode.balance_epoch,
                        sub_state: balance_mode.sub_state,
                        duration: balance_mode.duration,
                    }
                ))
            }))
        },
        Message::StateNormal(normal) => {
            metadata.sender_user_id = normal.metadata.sender_user_id;
            metadata.destination_id = normal.metadata.destination_id;
            metadata.timestamp = normal.metadata.timestamp;
            Some(Payload::ToManager(EdgeUploadToManager {
                payload: Some(grpc::edge_upload_to_manager::Payload::Normal(
                    StateNormal {
                        state: normal.state,
                    }
                ))
            }))
        },
        Message::StateSafeMode(safe_mode) => {
            metadata.sender_user_id = safe_mode.metadata.sender_user_id;
            metadata.destination_id = safe_mode.metadata.destination_id;
            metadata.timestamp = safe_mode.metadata.timestamp;
            Some(Payload::ToManager(EdgeUploadToManager {
                payload: Some(grpc::edge_upload_to_manager::Payload::SafeMode(
                    StateSafeMode {
                        state: safe_mode.state,
                        duration: safe_mode.duration,
                        frequency: safe_mode.frequency,
                        jitter: safe_mode.jitter,
                    }
                ))
            }))
        },
        Message::Heartbeat(heartbeat) => {
            metadata.sender_user_id = heartbeat.metadata.sender_user_id;
            metadata.destination_id = heartbeat.metadata.destination_id;
            metadata.timestamp = heartbeat.metadata.timestamp;
            Some(Payload::ToManager(EdgeUploadToManager {
                payload: Some(grpc::edge_upload_to_manager::Payload::Heartbeat(
                    Heartbeat {
                        beat: heartbeat.beat,
                    }
                ))
            }))
        },
        Message::FirmwareOutcome(firmware_outcome) => {
            metadata.sender_user_id = firmware_outcome.metadata.sender_user_id;
            metadata.destination_id = firmware_outcome.metadata.destination_id;
            metadata.timestamp = firmware_outcome.metadata.timestamp;
            Some(Payload::ToManager(EdgeUploadToManager {
                payload: Some(grpc::edge_upload_to_manager::Payload::FirmwareOutcome(
                    FirmwareOutcome {
                        version: firmware_outcome.version,
                        is_ok: firmware_outcome.is_ok,
                        percentage_ok: firmware_outcome.percentage_ok,
                    }
                ))
            }))
        },
        _ => None,
    };

    generate_edge_upload(payload, metadata, edge_id.to_string())
}


fn generate_edge_upload(payload: Option<Payload>,
                        metadata: Metadata,
                        edge_id: String
                       ) -> Option<EdgeUpload> {

    if let Some(p) = payload {
        Some(EdgeUpload {
            edge_id,
            sender_user_id: metadata.sender_user_id,
            destination_id: metadata.destination_id,
            timestamp: metadata.timestamp,
            payload: Some(p),
        })
    } else {
        None
    }
}


/// Itera sobre un vector de datos históricos y los envía uno a uno.
async fn process_batch(batch: TableDataVector,
                       tx: &mpsc::Sender<EdgeUpload>,
                       system: &System
) -> Result<(), ()> {

    match batch.vec {
        TableDataVectorTypes::Measurement(vec) => {
            for row in vec {
                if let Some(proto_msg) = convert_to_proto_upload_data_saver(Message::Report(row), system.id_edge.clone()) {
                    if tx.send(proto_msg).await.is_err() {
                        error!("Error: No se puede enviar mensaje EdgeUpload al cliente gRPC");
                    }
                }
            }
        },
        TableDataVectorTypes::Monitor(vec) => {
            for row in vec {
                if let Some(proto_msg) = convert_to_proto_upload_data_saver(Message::Monitor(row), system.id_edge.clone()) {
                    if tx.send(proto_msg).await.is_err() {
                        error!("Error: No se puede enviar mensaje EdgeUpload al cliente gRPC");
                    }
                }
            }
        },
        TableDataVectorTypes::AlertAir(vec) => {
            for row in vec {
                if let Some(proto_msg) = convert_to_proto_upload_data_saver(Message::AlertAir(row), system.id_edge.clone()) {
                    if tx.send(proto_msg).await.is_err() {
                        error!("Error: No se puede enviar mensaje EdgeUpload al cliente gRPC");
                    }
                }
            }
        },
        TableDataVectorTypes::AlertTemp(vec) => {
            for row in vec {
                if let Some(proto_msg) = convert_to_proto_upload_data_saver(Message::AlertTem(row), system.id_edge.clone()) {
                    if tx.send(proto_msg).await.is_err() {
                        error!("Error: No se puede enviar mensaje EdgeUpload al cliente gRPC");
                    }
                }
            }
        },
    }
    Ok(())
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
            InternalEvent::IncomingGrpc(edge_download) => {
                handle_grpc_message(edge_download, &tx_to_hub, &tx_to_network, &tx_to_firmware).await;
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


async fn handle_grpc_message(proto_msg: EdgeDownload,
                             tx_to_hub: &mpsc::Sender<Message>,
                             tx_to_network: &mpsc::Sender<Message>,
                             tx_to_firmware: &mpsc::Sender<Message>) {

    let mut metadata = Metadata::default();
    metadata.sender_user_id = proto_msg.sender_user_id;
    metadata.destination_id = proto_msg.destination_id;
    metadata.timestamp = proto_msg.timestamp;

    if let Some(payload) = proto_msg.payload {
        match payload {
            edge_download::Payload::FromManager(manager_msg) => {
                if let Some(inner) = manager_msg.payload {
                    match inner {
                        grpc::edge_download_from_manager::Payload::UpdateFirmware(update_firmware) => {
                            let msg = UpdateFirmware {
                                metadata,
                                network: update_firmware.network,
                                version: update_firmware.version,
                                url: update_firmware.url,
                                sha256: update_firmware.sha256,
                            };
                            if tx_to_firmware.send(Message::UpdateFirmware(msg)).await.is_err() {
                                error!("Error: No se pudo enviar mensaje UpdateFirmware a firmware");
                            }
                        },
                        grpc::edge_download_from_manager::Payload::DeleteHub(delete) => {
                            let msg = DeleteHub {
                                metadata,
                                network: delete.network,
                            };
                            if tx_to_network.send(Message::DeleteHub(msg)).await.is_err() {
                                error!("Error: No se pudo enviar mensaje a network");
                            }
                        },
                        grpc::edge_download_from_manager::Payload::Settings(settings) => {
                            let msg = Settings {
                                metadata,
                                network: settings.network,
                                wifi_ssid: settings.wifi_ssid,
                                wifi_password: settings.wifi_password,
                                mqtt_uri: settings.mqtt_uri,
                                device_name: settings.device_name,
                                sample: settings.sample,
                                energy_mode: settings.energy_mode,
                            };
                            if tx_to_network.send(Message::FromServerSettings(msg)).await.is_err() {
                                error!("Error: No se pudo enviar mensaje a network");
                            }
                        },
                        grpc::edge_download_from_manager::Payload::SettingOk(setting_ok) => {
                            let msg = SettingOk {
                                metadata,
                                network: setting_ok.network,
                                handshake: setting_ok.handshake,
                            };
                            if tx_to_hub.send(Message::FromServerSettingsAck(msg)).await.is_err() {
                                error!("Error: No se pudo enviar mensaje al hub");
                            }
                        },
                        grpc::edge_download_from_manager::Payload::Network(network) => {
                            let msg = Network {
                                metadata,
                                id_network: network.id_network,
                                name_network: network.name_network,
                                active: network.active,
                                delete_network: network.delete_network,
                            };
                            if tx_to_network.send(Message::Network(msg)).await.is_err() {
                                error!("Error: No se pudo enviar mensaje a network");
                            }
                        },
                    }
                }
            },
            edge_download::Payload::FromDataSaver(manager_msg) => {
                if let Some(inner) = manager_msg.payload {
                    match inner {
                        grpc::edge_download_from_data_saver::Payload::Heartbeat(_) => {
                            // HEARTBEAT
                        },
                    }
                }
            }
        }
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