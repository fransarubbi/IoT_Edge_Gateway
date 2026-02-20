//! # Módulo Lógico de Firmware
//!
//! Este módulo actúa como el orquestador asíncrono. Gestiona la concurrencia,
//! la comunicación I/O con la red y ejecuta las acciones dictadas por el dominio.


use chrono::{Utc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};
use crate::context::domain::AppContext;
use crate::firmware::domain::{Action, FirmwareServiceCommand, FirmwareServiceResponse, FsmStateFirmware, Phase, StateFirmwareUpdate, UpdateSession};
use crate::firmware::domain::{Event, Transition};
use crate::message::domain::{FirmwareOk, FirmwareOutcome, HubMessage, Metadata, ServerMessage, UpdateFirmware};
use crate::config::firmware::{OTA_TIMEOUT};


/// Tarea principal de actualización de firmware.
///
/// Mantiene un bucle infinito que escucha múltiples canales (Server, Hub, FSM)
#[instrument(name = "update_firmware", skip(app_context))]
pub async fn update_firmware_task(tx_to_core: mpsc::Sender<FirmwareServiceResponse>,
                                  tx_to_fsm: mpsc::Sender<Event>,
                                  tx_to_timer: mpsc::Sender<Event>,
                                  mut rx_msg: mpsc::Receiver<FirmwareServiceCommand>,
                                  mut rx_from_fsm: mpsc::Receiver<Vec<Action>>,
                                  app_context: AppContext,
                                  cancel: CancellationToken) {

    let mut session : Option<UpdateSession> = None;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                break;
            }

            Some(msg_from_server) = rx_msg.recv() => {
                match msg_from_server {
                    FirmwareServiceCommand::Update(update) => {
                        let manager = app_context.net_man.read().await;
                        let id = update.network.clone();
                        if let Some(total) = manager.get_total_hubs_by_network(&id) {

                            info!(
                                total_hubs = total,
                                "Info: Iniciando nueva sesión de Actualización de Firmware"
                            );

                            session = Some(UpdateSession::new(
                                ServerMessage::UpdateFirmware(update),
                                total,
                                id
                            ));
                            if tx_to_fsm.send(Event::MessageFromServer).await.is_err() {
                                error!("Error: No se pudo enviar evento MessageUpdate a la fsm de actualización de firmware");
                            }
                        } else {
                            error!("Error: No se encontraron hubs en la red {:?}", id);
                        }
                    },
                    FirmwareServiceCommand::HubResponse(firmware) => {
                        if let Some(ref mut session_ref) = session {
                            handle_message_from_hub(session_ref, &firmware, &tx_to_fsm).await;
                        }
                    }
                    _ => {}
                }
            }

            Some(vec_action) = rx_from_fsm.recv() => {
                for action in vec_action {
                    match action {
                        Action::OnEntry(state_firm) => {
                            on_entry(state_firm, &mut session, &tx_to_timer).await;
                        },
                        Action::SelectRandomHub => {
                            handle_select_random(&mut session, &tx_to_core, &tx_to_fsm, &app_context).await;
                        },
                        Action::SendMessageToNetwork => {
                            handle_send_message_to_network(&mut session, &tx_to_core).await;
                        },
                        Action::SendMessageOkToServer => {
                            handle_send_message_outcome(&mut session, &tx_to_core, &app_context).await;
                        },
                        Action::SendMessageFailToServer => {
                            handle_send_message_outcome(&mut session, &tx_to_core, &app_context).await;
                        },
                        Action::StopTimer => {
                            if tx_to_timer.send(Event::StopTimer).await.is_err() {
                                error!("Error: No se pudo enviar evento de finalización del timer");
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}


/// Actor que encapsula la Máquina de Estados (FSM).
/// Mantiene el estado en memoria y procesa eventos secuencialmente.
pub async fn run_fsm_firmware(tx_actions: mpsc::Sender<Vec<Action>>,
                              mut rx_event: mpsc::Receiver<Event>,
                              cancel: CancellationToken) {

    let mut state = FsmStateFirmware::new();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                break;
            }
            Some(event) = rx_event.recv() => {
                let transition = state.step(event);
                match transition {
                    Transition::Valid(t) => {
                        state = t.get_change_state();
                        if tx_actions.send(t.get_actions()).await.is_err() {
                            error!("Error: No se pudo enviar el vector de acciones a update_firmware_task");
                        }
                    },
                    Transition::Invalid(t) => {
                        warn!("FSM Firmware transición inválida: {}", t.get_invalid());
                    }
                }
            }
        }
    }
}


/// Maneja la recepción de confirmaciones de firmware de los dispositivos.
/// Implementa la lógica de Canario y la política de éxito estricta.
async fn handle_message_from_hub(session_ref: &mut UpdateSession, firmware: &FirmwareOk, tx_to_fsm: &mpsc::Sender<Event>) {
    let sender_id = firmware.metadata.sender_user_id.clone();
    let is_ok = firmware.is_ok;

    debug!(
        hub_id = %sender_id,
        success = is_ok,
        phase = ?session_ref.phase,
        "Debug: Recibida respuesta de firmware"
    );

    if session_ref.is_canary() {
        if firmware.is_ok {
            info!(hub_id = %sender_id, "Info: Canario actualizado exitosamente. Avanzando a Broadcast.");
            if tx_to_fsm.send(Event::MessageFromHub).await.is_err() {
                error!("Error: No se pudo enviar evento MessageFromHub a la fsm de actualización de firmware");
            }
        } else {
            warn!(hub_id = %sender_id, "Warning: Canario falló la actualización. Abortando despliegue.");
            if tx_to_fsm.send(Event::Error).await.is_err() {
                error!("Error: No se pudo enviar evento Error a la fsm de actualización de firmware");
            }
        }
    }
    else if session_ref.is_broadcast() {
        let user_id = firmware.metadata.sender_user_id.to_string();
        if session_ref.responses.insert(user_id) {
            if firmware.is_ok {
                session_ref.successes += 1;
                session_ref.version = firmware.version.clone();
            } else {
                session_ref.failures += 1;
            }

            if session_ref.is_complete() {
                let rate = session_ref.success_rate();
                session_ref.percentage = session_ref.success_rate();

                if session_ref.failures == 0 {
                    if tx_to_fsm.send(Event::AllMessageReceived).await.is_err() {
                        error!("Error: No se pudo enviar evento AllMessageReceived a la fsm de actualización de firmware");
                    }
                } else {
                    if tx_to_fsm.send(Event::Error).await.is_err() {
                        error!("Error: No se pudo enviar evento Error a la fsm de actualización de firmware");
                    }
                }

                info!(
                    success_rate = rate,
                    failures = session_ref.failures,
                    total = session_ref.total_hubs,
                    "Info: Fase Broadcast finalizada. Reportando resultados."
                );
            }
        }
    }
}


/// Manejador de entrada de estados. Gestiona limpieza y timers.
async fn on_entry(state_firm: StateFirmwareUpdate, session: &mut Option<UpdateSession>, tx_to_timer: &mpsc::Sender<Event>) {
    match state_firm {
        StateFirmwareUpdate::WaitingForFirmware => {
            *session = None;
        },
        StateFirmwareUpdate::NotifyHub => {
            init_timer_and_update_phase(session, &tx_to_timer, Phase::Canary).await;
        },
        StateFirmwareUpdate::WaitingNetworkAnswer => {
            init_timer_and_update_phase(session, &tx_to_timer, Phase::Broadcast).await;
        }
    }
}


/// Helper para inicializar el timer y cambiar la fase de la sesión.
async fn init_timer_and_update_phase(session: &mut Option<UpdateSession>, tx_to_timer: &mpsc::Sender<Event>, phase: Phase) {
    if let Some(session_ref) = session {
        if tx_to_timer.send(Event::InitTimer(OTA_TIMEOUT)).await.is_err() {
            error!("Error: No se pudo enviar evento de inicialización del watchdog de firmware");
        }
        session_ref.phase = phase;
    }
}


/// Selecciona un Hub aleatorio (Canario) y le envía el firmware.
async fn handle_select_random(session: &mut Option<UpdateSession>,
                              tx_to_core: &mpsc::Sender<FirmwareServiceResponse>,
                              tx_to_fsm: &mpsc::Sender<Event>,
                              app_context: &AppContext) {

    if let Some(session_ref) = session {
        let manager = app_context.net_man.read().await;
        if let Some(id_hub) = manager.get_random_hub_id_by_network(&session_ref.network) {
            info!(
                network = %session_ref.network,
                target_hub = %id_hub,
                "Info: Seleccionado Hub Canario para prueba piloto"
            );

            match session_ref.message.clone() {
                ServerMessage::UpdateFirmware(update) => {
                    
                    let mut metadata = update.metadata.clone();
                    metadata.destination_id = id_hub.clone();
                    
                    let update_msg = UpdateFirmware {
                        metadata,
                        network: update.network,
                    };
                    
                    if tx_to_core.send(FirmwareServiceResponse::HubCommand(HubMessage::UpdateFirmware(update_msg))).await.is_err() {
                        error!("Error: No se pudo enviar mensaje al Hub canario");
                    }
                },
                _ => {},
            }
        } else {
            error!(network = %session_ref.network, "Error: La red está vacía, no se puede seleccionar canario.");
            if tx_to_fsm.send(Event::Error).await.is_err() {
                error!("Error: No se pudo enviar evento de Error a la fsm firmware");
            }
        }
    }
}


/// Envía mensaje Broadcast a toda la red ("all").
async fn handle_send_message_to_network(session: &mut Option<UpdateSession>,
                                        tx_to_hub: &mpsc::Sender<FirmwareServiceResponse>) {

    if let Some(session_ref) = session {
        match session_ref.message.clone() {
            ServerMessage::UpdateFirmware(update) => {
                let mut metadata = update.metadata.clone();
                metadata.destination_id = "all".to_string();
                
                let update_msg = UpdateFirmware {
                    metadata,
                    network: update.network.clone(),
                };
                
                if tx_to_hub.send(FirmwareServiceResponse::HubCommand(HubMessage::UpdateFirmware(update_msg))).await.is_err() {
                    error!("Error: No se pudo enviar mensaje al resto de Hubs (Broadcast Firmware)");
                }
            },
            _ => {},
        }
    }
}


/// Genera y envía el reporte final (Outcome) al servidor.
async fn handle_send_message_outcome(session: &mut Option<UpdateSession>,
                                     tx_to_server: &mpsc::Sender<FirmwareServiceResponse>,
                                     app_context: &AppContext) {

    if let Some(session_ref) = session {
        let timestamp = Utc::now().timestamp();

        let metadata = Metadata {
            sender_user_id: app_context.system.id_edge.clone(),
            destination_id: "Server0".to_string(),
            timestamp
        };

        let network = session_ref.network.clone();
        if session_ref.percentage == 100.00 {
            let firm_out = FirmwareOutcome { metadata, network, is_ok: true, percentage_ok: 100.00 };
            if tx_to_server.send(FirmwareServiceResponse::ServerAck(ServerMessage::FirmwareOutcome(firm_out))).await.is_err() {
                error!("Error: No se pudo enviar FirmwareOutcome Ok al servidor");
            }
        } else {
            let firm_out = FirmwareOutcome { metadata, network, is_ok: true, percentage_ok: session_ref.percentage };
            if tx_to_server.send(FirmwareServiceResponse::ServerAck(ServerMessage::FirmwareOutcome(firm_out))).await.is_err() {
                error!("Error: No se pudo enviar FirmwareOutcome Fail al servidor");
            }
        }
    }
}