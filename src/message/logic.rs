//! Módulo de Lógica de Mensajería y Enrutamiento (Router/Switchboard).
//!
//! Este módulo es el núcleo de comunicaciones del Edge Gateway. Actúa como un "Switchboard"
//! o enrutador central que conecta los nodos físicos (Hubs vía MQTT) con la nube (Servidor vía gRPC),
//! pasando por la máquina de estados local (FSM) y la persistencia de datos (DB).
//!
//! # Responsabilidades Principales
//!
//! - **Uplink Local (`msg_from_hub`):** Recibe telemetría MQTT (MessagePack), la deserializa y
//!   decide si enviarla a la nube en tiempo real o a la base de datos si no hay conexión.
//! - **Downlink Local (`msg_to_hub`):** Recibe comandos internos o remotos, los serializa a
//!   MessagePack y los publica en el broker MQTT local hacia los Hubs.
//! - **Uplink Remoto (`msg_to_server`):** Convierte los mensajes del dominio a estructuras Protobuf
//!   y los transmite al servidor central a través de gRPC.
//! - **Downlink Remoto (`msg_from_server`):** Recibe instrucciones gRPC de la nube, las traduce
//!   al modelo de dominio y las distribuye a la FSM o a los Hubs.

use chrono::Utc;
use rmp_serde::{from_slice, to_vec};
use serde::Serialize;
use tokio::sync::{mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, warn};
use crate::context::domain::AppContext;
use crate::message::domain::{SerializedMessage, ServerStatus, LocalStatus, Metadata, UpdateFirmware, DeleteHub, Settings, SettingOk, Network, Heartbeat as HeartbeatMsg, HelloWorld, MessageServiceCommand, ServerMessage, HubMessage, MessageServiceResponse};
use crate::database::domain::{TableDataVector};
use crate::grpc;
use crate::grpc::{to_edge, FromEdge, ToEdge,
                  FirmwareOutcome, SettingOk as SettOk,
                  Settings as Sett, SystemMetrics, HelloWorld as Hello};
use crate::grpc::from_edge::Payload;  
use crate::network::domain::{NetworkManager};
use crate::system::domain::{InternalEvent, System};


/// Gestor de mensajes salientes hacia el Hub (Downlink Local).
///
/// Centraliza todo el tráfico que debe ser publicado en el broker MQTT local para
/// que los dispositivos finales (Hubs) lo consuman.
///
/// # Fuentes de Mensajes
///
/// 1.  **FSM (`rx`):** Mensajes de control generados internamente (Handshakes, Heartbeats, Ping).
/// 2.  **Servidor (`rx_server_msg`):** Comandos remotos (Settings, acks) provenientes de la nube.
/// 3.  **Red (`rx_internal`):** Notificaciones de conexión/desconexión del broker MQTT local.
///
/// # Lógica de Procesamiento
///
/// - Monitorea el estado de la conexión local. Si está desconectado, los mensajes se descartan
///   para evitar desbordar colas.
/// - Resuelve dinámicamente el `topic`, `QoS` y flag de `Retain` a través del `NetworkManager`.
/// - Llama a la función genérica `send` para serializar en **MessagePack** y despachar a la capa MQTT.
#[instrument(name = "msg_to_hub", skip(app_context))]
pub async fn msg_to_hub(tx_to_mqtt_local: mpsc::Sender<MessageServiceResponse>,
                        mut rx_internal: mpsc::Receiver<InternalEvent>,
                        mut rx_server_msg: mpsc::Receiver<ServerMessage>,
                        mut rx: mpsc::Receiver<MessageServiceCommand>,
                        app_context: AppContext, 
                        shutdown: CancellationToken) {

    let mut state = LocalStatus::Connected;
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Info: shutdown recibido msg_to_hub");
                break;
            }
            
            Some(internal) = rx_internal.recv() => {
                match internal {
                    InternalEvent::LocalConnected => state = LocalStatus::Connected,
                    InternalEvent::LocalDisconnected => state = LocalStatus::Disconnected,
                    _ => {},
                }
            }

            Some(msg) = rx_server_msg.recv() => {
                match msg {
                    ServerMessage::FromServerSettingsAck(ack) => {
                        if matches!(state, LocalStatus::Disconnected) {
                            warn!("Warning: Mensaje descartado, broker desconectado");
                            continue;
                        }
                        let topic_opt = {
                            let manager = app_context.net_man.read().await;
                            manager.get_topic_to_send_msg_to_hub(&HubMessage::FromServerSettingsAck(ack.clone()))
                        };

                        if let Some(t) = topic_opt {
                            match send(&tx_to_mqtt_local, t.topic, t.qos, HubMessage::FromServerSettingsAck(ack), false).await {
                                Ok(_) => {}
                                Err(_) => error!("Error: No se pudo serializar el mensaje del servidor al broker"),
                            }
                        }
                    }
                    _ => {}
                }
            }

            Some(msg) = rx.recv() => {
                match msg {
                    MessageServiceCommand::ToHub(to_hub) => {
                        if matches!(state, LocalStatus::Disconnected) {
                            warn!("Warning: Mensaje descartado, broker desconectado");
                            continue;
                        }

                        let topic_out = {
                            let manager = app_context.net_man.read().await;
                            resolve_fsm_topic(&manager, &to_hub)
                        };

                        if let Some((topic, qos, retain)) = topic_out {
                            match send(&tx_to_mqtt_local, topic, qos, to_hub, retain).await {
                                Ok(_) => {}
                                Err(_) => error!("Error: No se pudo serializar el mensaje de la FSM al broker"),
                            }
                        } else {
                            let topic_opt = {
                                let manager = app_context.net_man.read().await;
                                manager.get_topic_to_send_msg_to_hub(&to_hub)
                            };

                            if let Some(t) = topic_opt {
                                match send(&tx_to_mqtt_local, t.topic, t.qos, to_hub, false).await {
                                    Ok(_) => {}
                                    Err(_) => error!("Error: No se pudo serializar el mensaje del servidor al broker"),
                                }
                            }
                        }
                    },
                    _ => {}
                }
            }
        }
    }
}


/// Extrae el tópico MQTT, el nivel QoS y la bandera Retain específicos para mensajes de estado y control (FSM).
///
/// Mensajes críticos como los cambios de estado operativos (`StateBalanceMode`, `StateNormal`)
/// se publican con el flag `Retain = true` para que los dispositivos nuevos los reciban al conectar.
fn resolve_fsm_topic(manager: &NetworkManager,
                     msg: &HubMessage
) -> Option<(String, u8, bool)> {

    match msg {
        HubMessage::HandshakeToHub(_) =>
            Some((manager.topic_handshake.topic.clone(), manager.topic_handshake.qos, false)),

        HubMessage::StateBalanceMode(_) =>
            Some((manager.topic_state.topic.clone(), manager.topic_state.qos, true)),

        HubMessage::StateNormal(_) =>
            Some((manager.topic_state.topic.clone(), manager.topic_state.qos, true)),

        HubMessage::StateSafeMode(_) =>
            Some((manager.topic_state.topic.clone(), manager.topic_state.qos, true)),

        HubMessage::Heartbeat(_) =>
            Some((manager.topic_heartbeat.topic.clone(), manager.topic_heartbeat.qos, false)),

        HubMessage::Ping(_) => {
            if let Some(topic) = manager.get_topic_to_send_msg_to_hub(msg) {
                Some((topic.topic, topic.qos, false))
            } else {
                None
            }
        },

        _ => None,
    }
}


// -------------------------------------------------------------------------------------------------


/// Procesador de mensajes entrantes del Hub (Uplink Local).
///
/// Actúa como la primera línea de procesamiento para los datos que llegan desde la red de sensores
/// a través de MQTT. Realiza la deserialización desde MessagePack y enruta según el estado de la conexión externa.
///
/// # Reglas de Enrutamiento (Store and Forward Core)
///
/// 1.  Si el servidor externo está **Conectado**, reenvía la telemetría (Reports, Monitors, Alerts)
///     directamente al módulo `msg_to_server`.
/// 2.  Si el servidor externo está **Desconectado**, enruta la telemetría hacia `tx` (Canal general)
///     donde será interceptada por la base de datos (`dba_insert_task`) para almacenamiento temporal.
#[instrument(name = "msg_from_hub", skip(rx))]
pub async fn msg_from_hub(tx: mpsc::Sender<MessageServiceResponse>,
                          tx_to_msg_to_server: mpsc::Sender<ServerMessage>,
                          tx_to_msg_to_hub: mpsc::Sender<InternalEvent>,
                          mut rx: mpsc::Receiver<MessageServiceCommand>, 
                          shutdown: CancellationToken) {

    let mut state: ServerStatus = ServerStatus::Connected;
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Info: shutdown recibido msg_from_hub");
                break;
            }
            
            Some(msg) = rx.recv() => {
                match msg {
                    MessageServiceCommand::Internal(internal) => {
                        match internal {
                            InternalEvent::LocalDisconnected => {
                                if tx_to_msg_to_hub.send(InternalEvent::LocalDisconnected).await.is_err() {
                                    error!("Error: No se pudo enviar el mensaje LocalDisconnected a msg_to_hub");
                                }
                            },
                            InternalEvent::LocalConnected => {
                                if tx_to_msg_to_hub.send(InternalEvent::LocalConnected).await.is_err() {
                                    error!("Error: No se pudo enviar el mensaje LocalConnected a msg_to_hub");
                                }
                            },
                            InternalEvent::IncomingMessage(message) => {
                                if let Ok(decoded) = from_slice(&message.payload) {
                                    if matches!(state, ServerStatus::Connected) {
                                        match decoded {
                                            HubMessage::Report(report) => {
                                                if tx_to_msg_to_server.send(ServerMessage::Report(report)).await.is_err() {
                                                    error!("Error: no se pudo enviar el mensaje FromHub");
                                                }
                                            },
                                            HubMessage::Monitor(monitor) => {
                                                if tx_to_msg_to_server.send(ServerMessage::Monitor(monitor)).await.is_err() {
                                                    error!("Error: no se pudo enviar el mensaje FromHub");
                                                }
                                            },
                                            HubMessage::AlertAir(alert) => {
                                                if tx_to_msg_to_server.send(ServerMessage::AlertAir(alert)).await.is_err() {
                                                    error!("Error: no se pudo enviar el mensaje FromHub");
                                                }
                                            },
                                            HubMessage::AlertTem(alert) => {
                                                if tx_to_msg_to_server.send(ServerMessage::AlertTem(alert)).await.is_err() {
                                                    error!("Error: no se pudo enviar el mensaje FromHub");
                                                }
                                            },
                                            HubMessage::FromHubSettings(settings) => {
                                                if tx_to_msg_to_server.send(ServerMessage::FromHubSettings(settings)).await.is_err() {
                                                    error!("Error: no se pudo enviar el mensaje FromHub");
                                                }
                                            },
                                            HubMessage::FromHubSettingsAck(ack) => {
                                                if tx_to_msg_to_server.send(ServerMessage::FromHubSettingsAck(ack)).await.is_err() {
                                                    error!("Error: no se pudo enviar el mensaje FromHub");
                                                }
                                            }
                                            _ => {}
                                        }
                                    } else {
                                        if tx.send(MessageServiceResponse::FromHub(decoded)).await.is_err() {
                                            error!("Error: no se pudo enviar el mensaje FromHub");
                                        }
                                    }
                                }
                            },
                            InternalEvent::ServerConnected => {
                                state = ServerStatus::Connected;
                            }
                            InternalEvent::ServerDisconnected => {
                                state = ServerStatus::Disconnected;
                            }
                            _ => {},
                        }
                    },
                    _ => {}
                }
            }
        }
    }
}


// -------------------------------------------------------------------------------------------------


/// Gestor de mensajes salientes hacia el Servidor (Uplink Remoto vía gRPC).
///
/// Recolecta mensajes del dominio, los transforma al modelo Protobuf (gRPC) y los encola
/// para su envío hacia la nube.
///
/// # Flujos de Datos Procesados
///
/// - **Tiempo Real (`rx_from_hub`):** Datos recién llegados que pasaron el filtro de conexión.
/// - **Datos Históricos (`Batch`):** Lotes de información extraídos de SQLite tras recuperar conexión.
/// - **Comandos Internos:** Mensajes como `HelloWorld` para establecer sesión en la nube.
#[instrument(name = "msg_to_server", skip(rx_from_hub, rx, app_context))]
pub async fn msg_to_server(tx_to_server: mpsc::Sender<MessageServiceResponse>,
                           mut rx_from_hub: mpsc::Receiver<ServerMessage>,
                           mut rx: mpsc::Receiver<MessageServiceCommand>,
                           app_context: AppContext, 
                           shutdown: CancellationToken) {

    let mut state = ServerStatus::Connected;
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Info: shutdown recibido msg_to_server");
                break;
            }
            
            Some(msg) = rx.recv() => {
                match msg {
                    MessageServiceCommand::Internal(internal) => {
                        match internal {
                            InternalEvent::ServerConnected => {
                                state = ServerStatus::Connected;
                            }
                            InternalEvent::ServerDisconnected => {
                                state = ServerStatus::Disconnected;
                            }
                            _ => {}
                        }
                    },
                    MessageServiceCommand::Batch(batch) => {
                        if matches!(state, ServerStatus::Disconnected) {
                            warn!("Warning: mensaje del hub descartado, servidor desconectado");
                            continue;
                        }
                        match process_batch(batch, &tx_to_server, &app_context.system).await {
                            Ok(_) => {},
                            Err(_) => error!("Error: fallo en envío de batch al cliente gRPC"),
                        }
                    },
                    MessageServiceCommand::ToServer(server_message) => {
                        if matches!(state, ServerStatus::Disconnected) {
                            warn!("Warning: mensaje del hub descartado, servidor desconectado");
                            continue;
                        }
                        if let Some(proto_msg) = convert_to_proto_upload(server_message, app_context.system.id_edge.clone()) {
                            if tx_to_server.send(MessageServiceResponse::EdgeUpload(proto_msg)).await.is_err() {
                                error!("Error: no se puede enviar mensaje EdgeUpload al cliente gRPC");
                            }
                        }
                    },
                    MessageServiceCommand::GenerateHelloWorld => {
                        let metadata = Metadata {
                            sender_user_id: app_context.system.id_edge.clone(),
                            destination_id: "Server0".to_string(),
                            timestamp: Utc::now().timestamp(),
                        };
                        let hello = HelloWorld {
                            metadata,
                            hello: true
                        };
                        if let Some(proto_msg) = convert_to_proto_upload(ServerMessage::HelloWorld(hello), app_context.system.id_edge.clone()) {
                            if tx_to_server.send(MessageServiceResponse::EdgeUpload(proto_msg)).await.is_err() {
                                error!("Error: no se puede enviar mensaje EdgeUpload al cliente gRPC");
                            }
                        }
                    }
                    _ => {}
                }
            }

            Some(server_message) = rx_from_hub.recv() => {
                if matches!(state, ServerStatus::Disconnected) {
                    warn!("Warning: Mensaje del hub descartado, servidor desconectado");
                    continue;
                }
                if let Some(proto_msg) = convert_to_proto_upload(server_message, app_context.system.id_edge.clone()) {
                    if tx_to_server.send(MessageServiceResponse::EdgeUpload(proto_msg)).await.is_err() {
                        error!("Error: no se puede enviar mensaje EdgeUpload al cliente gRPC");
                    }
                }
            }
        }
    }
}


/// Mapea las estructuras internas de dominio (`ServerMessage`) a los tipos generados
/// por Protobuf (`FromEdge` / `Payload`).
///
/// Este es el punto de traducción formal antes de que la capa de red gRPC envíe los bytes.
fn convert_to_proto_upload(msg: ServerMessage, edge_id: String) -> Option<FromEdge> {

    let payload = match msg {
        ServerMessage::HelloWorld(hello) => {
           Some(Payload::HelloWorld(
               Hello {
                   metadata: Some(grpc::Metadata {
                       sender_user_id: hello.metadata.sender_user_id,
                       destination_id: hello.metadata.destination_id,
                       timestamp: hello.metadata.timestamp,
                   }),
                   hello: hello.hello,
               }
           ))
        },
        ServerMessage::FromHubSettings(hub_settings) => {
            Some(Payload::Settings(
                Sett {
                    metadata: Some(grpc::Metadata {
                        sender_user_id: hub_settings.metadata.sender_user_id,
                        destination_id: hub_settings.metadata.destination_id,
                        timestamp: hub_settings.metadata.timestamp,
                    }),
                    network: hub_settings.network,
                    wifi_ssid: hub_settings.wifi_ssid,
                    wifi_password: hub_settings.wifi_password,
                    mqtt_uri: hub_settings.mqtt_uri,
                    device_name: hub_settings.device_name,
                    sample: hub_settings.sample,
                    energy_mode: hub_settings.energy_mode,
                }
            ))
        },
        ServerMessage::FromHubSettingsAck(hub_settings_ack) => {
            Some(Payload::SettingOk(
                    SettOk {
                        metadata: Some(grpc::Metadata {
                            sender_user_id: hub_settings_ack.metadata.sender_user_id,
                            destination_id: hub_settings_ack.metadata.destination_id,
                            timestamp: hub_settings_ack.metadata.timestamp,
                        }),
                        network: hub_settings_ack.network,
                        handshake: hub_settings_ack.handshake,
                    }
            ))
        },
        ServerMessage::FirmwareOutcome(firmware_outcome) => {
            Some(Payload::FirmwareOutcome(
                    FirmwareOutcome {
                        metadata: Some(grpc::Metadata {
                            sender_user_id: firmware_outcome.metadata.sender_user_id,
                            destination_id: firmware_outcome.metadata.destination_id,
                            timestamp: firmware_outcome.metadata.timestamp,
                        }),
                        network: firmware_outcome.network,
                        is_ok: firmware_outcome.is_ok,
                        percentage_ok: firmware_outcome.percentage_ok,
                    }
                ))
        },
        ServerMessage::Report(report) => {
            Some(Payload::Measurement(grpc::Measurement {
                metadata: Some(grpc::Metadata {
                    sender_user_id: report.metadata.sender_user_id,
                    destination_id: report.metadata.destination_id,
                    timestamp: report.metadata.timestamp,
                }),
                network: report.network, // Asegúrate de mapear este campo requerido por el proto
                pulse_counter: report.pulse_counter,
                pulse_max_duration: report.pulse_max_duration,
                temperature: report.temperature,
                humidity: report.humidity,
                co2_ppm: report.co2_ppm,
                sample: report.sample as u32,
            }))
        },
        ServerMessage::Monitor(monitor) => {
            Some(Payload::Monitor(grpc::Monitor {
                metadata: Some(grpc::Metadata {
                    sender_user_id: monitor.metadata.sender_user_id,
                    destination_id: monitor.metadata.destination_id,
                    timestamp: monitor.metadata.timestamp,
                }),
                network: monitor.network,
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
            }))
        },
        ServerMessage::AlertAir(alert_air) => {
            Some(Payload::AlertAir(grpc::AlertAir {
                metadata: Some(grpc::Metadata {
                    sender_user_id: alert_air.metadata.sender_user_id,
                    destination_id: alert_air.metadata.destination_id,
                    timestamp: alert_air.metadata.timestamp,
                }),
                network: alert_air.network,
                co2_initial_ppm: alert_air.co2_initial_ppm,
                co2_actual_ppm: alert_air.co2_actual_ppm,
            }))
        },
        ServerMessage::AlertTem(alert_tem) => {
            Some(Payload::AlertTh(grpc::AlertTh {
                metadata: Some(grpc::Metadata {
                    sender_user_id: alert_tem.metadata.sender_user_id,
                    destination_id: alert_tem.metadata.destination_id,
                    timestamp: alert_tem.metadata.timestamp,
                }),
                network: alert_tem.network,
                initial_temp: alert_tem.initial_temp,
                actual_temp: alert_tem.actual_temp,
            }))
        },
        ServerMessage::Metrics(metrics) => {
            Some(Payload::Metrics(SystemMetrics {
                metadata: Some(grpc::Metadata {
                    sender_user_id: metrics.metadata.sender_user_id,
                    destination_id: metrics.metadata.destination_id,
                    timestamp: metrics.metadata.timestamp,
                }),
                // Nota: SystemMetrics en tu proto NO tiene campo 'network', así que no lo ponemos aquí.
                uptime_seconds: metrics.uptime_seconds,
                cpu_usage_percent: metrics.cpu_usage_percent,
                cpu_temp_celsius: metrics.cpu_temp_celsius,
                ram_total_mb: metrics.ram_total_mb,
                ram_used_mb: metrics.ram_used_mb,
                sd_total_gb: metrics.sd_total_gb,
                sd_used_gb: metrics.sd_used_gb,
                sd_usage_percent: metrics.sd_usage_percent,
                network_rx_bytes: metrics.network_rx_bytes,
                network_tx_bytes: metrics.network_tx_bytes,
                wifi_rssi: metrics.wifi_rssi.unwrap_or(0),
                wifi_signal_dbm: metrics.wifi_signal_dbm.unwrap_or(0),
            }))
        },
        _ => None,
    };

    generate_edge_upload(payload, edge_id.to_string())
}


/// Envuelve el `Payload` gRPC validado en la estructura final de transmisión `FromEdge`.
fn generate_edge_upload(payload: Option<Payload>,
                        edge_id: String
                       ) -> Option<FromEdge> {

    if let Some(p) = payload {
        Some(FromEdge {
            edge_id,
            payload: Some(p),
        })
    } else {
        None
    }
}


/// Itera y procesa por lotes los datos históricos recuperados de la base de datos local.
///
/// Se ejecuta cuando el patrón "Store and Forward" detecta que la red regresó y
/// la base de datos inyecta un `TableDataVector` (Batch). Convierte cada fila a gRPC
/// y las encola para envío.
async fn process_batch(batch: TableDataVector,
                       tx: &mpsc::Sender<MessageServiceResponse>,
                       system: &System
) -> Result<(), ()> {

    for row in batch.measurement {
        if let Some(proto_msg) = convert_to_proto_upload(ServerMessage::Report(row), system.id_edge.clone()) {
            if tx.send(MessageServiceResponse::EdgeUpload(proto_msg)).await.is_err() {
                error!("Error: no se puede enviar mensaje EdgeUpload al cliente gRPC");
            }
        }
    }

    for row in batch.monitor {
        if let Some(proto_msg) = convert_to_proto_upload(ServerMessage::Monitor(row), system.id_edge.clone()) {
            if tx.send(MessageServiceResponse::EdgeUpload(proto_msg)).await.is_err() {
                error!("Error: no se puede enviar mensaje EdgeUpload al cliente gRPC");
            }
        }
    }
    
    for row in batch.alert_air {
        if let Some(proto_msg) = convert_to_proto_upload(ServerMessage::AlertAir(row), system.id_edge.clone()) {
            if tx.send(MessageServiceResponse::EdgeUpload(proto_msg)).await.is_err() {
                error!("Error: no se puede enviar mensaje EdgeUpload al cliente gRPC");
            }
        }
    }
    
    for row in batch.alert_th {
        if let Some(proto_msg) = convert_to_proto_upload(ServerMessage::AlertTem(row), system.id_edge.clone()) {
            if tx.send(MessageServiceResponse::EdgeUpload(proto_msg)).await.is_err() {
                error!("Error: no se puede enviar mensaje EdgeUpload al cliente gRPC");
            }
        }
    }
    
    Ok(())
}


// -------------------------------------------------------------------------------------------------


/// Procesador de mensajes entrantes del Servidor (Downlink Remoto).
///
/// Recibe eventos crudos de la nube (vía el motor gRPC) y los clasifica.
///
/// # Funcionalidad
///
/// Extrae la carga útil (`Payload`) de los mensajes Protobuf y delega la traducción
/// al dominio a la función `handle_grpc_message`.
#[instrument(name = "msg_from_server", skip(rx))]
pub async fn msg_from_server(tx: mpsc::Sender<MessageServiceResponse>,
                             tx_to_msg_to_hub: mpsc::Sender<ServerMessage>,
                             mut rx: mpsc::Receiver<MessageServiceCommand>, 
                             shutdown: CancellationToken) {

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Info: shutdown recibido msg_from_server");
                break;
            }
            
            Some(msg) = rx.recv() => {
                match msg {
                    MessageServiceCommand::Internal(internal) => {
                        match internal {
                            InternalEvent::IncomingGrpc(edge_download) => {
                                handle_grpc_message(edge_download,
                                            &tx,
                                            &tx_to_msg_to_hub).await;
                            },
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}


/// Convierte los mensajes gRPC (`ToEdge`) entrantes en tipos nativos de dominio (`ServerMessage`)
/// y los enruta al destino adecuado (Hub local o procesos internos del Edge).
async fn handle_grpc_message(proto_msg: ToEdge,
                             tx: &mpsc::Sender<MessageServiceResponse>,
                             tx_to_msg_to_hub: &mpsc::Sender<ServerMessage>) {

    if let Some(payload) = proto_msg.payload {
        match payload {
            to_edge::Payload::UpdateFirmware(update_firmware) => {
                let msg = UpdateFirmware {
                    metadata: extract_metadata(update_firmware.metadata),
                    network: update_firmware.network,
                };
                if tx.send(MessageServiceResponse::FromServer(ServerMessage::UpdateFirmware(msg))).await.is_err() {
                    error!("Error: no se pudo enviar mensaje UpdateFirmware a firmware");
                }
            },
            to_edge::Payload::Settings(settings) => {
                let msg = Settings {
                    metadata: extract_metadata(settings.metadata),
                    network: settings.network,
                    wifi_ssid: settings.wifi_ssid,
                    wifi_password: settings.wifi_password,
                    mqtt_uri: settings.mqtt_uri,
                    device_name: settings.device_name,
                    sample: settings.sample,
                    energy_mode: settings.energy_mode,
                };
                if tx.send(MessageServiceResponse::FromServer(ServerMessage::FromServerSettings(msg))).await.is_err() {
                    error!("Error: no se pudo enviar mensaje a network");
                }
            },
            to_edge::Payload::DeleteHub(delete) => {
                let msg = DeleteHub {
                    metadata: extract_metadata(delete.metadata),
                    network: delete.network,
                };
                if tx.send(MessageServiceResponse::FromServer(ServerMessage::DeleteHub(msg))).await.is_err() {
                    error!("Error: no se pudo enviar mensaje a network");
                }
            },
            to_edge::Payload::SettingOk(setting_ok) => {
                let msg = SettingOk {
                    metadata: extract_metadata(setting_ok.metadata),
                    network: setting_ok.network,
                    handshake: setting_ok.handshake,
                };
                if tx_to_msg_to_hub.send(ServerMessage::FromServerSettingsAck(msg)).await.is_err() {
                    error!("Error: no se pudo enviar mensaje al hub");
                }
            },
            to_edge::Payload::Network(network) => {
                let msg = Network {
                    metadata: extract_metadata(network.metadata),
                    id_network: network.id_network,
                    name_network: network.name_network,
                    active: network.active,
                    delete_network: network.delete_network,
                };
                if tx.send(MessageServiceResponse::FromServer(ServerMessage::Network(msg))).await.is_err() {
                    error!("Error: no se pudo enviar mensaje a network");
                }
            },
            to_edge::Payload::Heartbeat(heartbeat) => {
                let msg = HeartbeatMsg {
                    metadata: extract_metadata(heartbeat.metadata),
                    beat: true,
                };
                if tx.send(MessageServiceResponse::FromServer(ServerMessage::Heartbeat(msg))).await.is_err() {
                    error!("Error: no se pudo enviar mensaje a heartbeat");
                }
            },
            to_edge::Payload::HelloWorld(hello_ack) => {
                let msg = HelloWorld {
                    metadata: extract_metadata(hello_ack.metadata),
                    hello: hello_ack.hello,
                };
                if tx.send(MessageServiceResponse::FromServer(ServerMessage::HelloWorld(msg))).await.is_err() {
                    error!("Error: no se pudo enviar mensaje HelloWorld a la fsm general");
                }
            },
        }
    }
}


/// Helper para convertir la metadata de red generada por Protobuf en la estructura
/// plana de `Metadata` usada en el dominio del negocio.
fn extract_metadata(proto_meta: Option<grpc::Metadata>) -> Metadata {
    let meta = proto_meta.unwrap_or_default();
    Metadata {
        sender_user_id: meta.sender_user_id,
        destination_id: meta.destination_id,
        timestamp: meta.timestamp,
    }
}


// -------------------------------------------------------------------------------------------------


/// Utilidad genérica de serialización y encolamiento para el Downlink Local.
///
/// Serializa cualquier estructura de dominio (T) a un arreglo de bytes utilizando **MessagePack**
/// y la empaqueta en un objeto [`SerializedMessage`] listo para ser procesado y publicado
/// por el cliente MQTT.
pub async fn send<T>(tx: &mpsc::Sender<MessageServiceResponse>,
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
            if tx.send(MessageServiceResponse::Serialized(serialized)).await.is_err() {
                return Err(());
            }
        }
        Err(e) => {
            error!("Error: no se pudo serializar mensaje: {:?}", e);
        }
    }
    Ok(())
}