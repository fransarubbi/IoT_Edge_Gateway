//! # Módulo de Coordinación de la Máquina de Estados Finitos (FSM)
//!
//! Este módulo actúa como el **controlador principal** y la capa de "pegamento" entre la lógica pura
//! de la FSM, la comunicación de red (Hubs y Servidor) y los temporizadores del sistema.
//!
//! ## Arquitectura
//! El módulo sigue un patrón de diseño basado en actores utilizando canales de `tokio` (MPSC).
//! Se divide en las siguientes responsabilidades:
//!
//! 1.  **Orquestador de Canales (`fsm_general_channels`):** Es el bucle de eventos principal. Multiplexa
//!     mensajes entrantes de la red y comandos de la FSM lógica.
//! 2.  **Ejecutor de Efectos (`handle_action`):** Traduce las `Actions` abstractas de la FSM en operaciones
//!     concretas (enviar paquetes gRPC, iniciar timers, actualizar base de datos).
//! 3.  **Lógica de FSM (`run_fsm`):** Mantiene el estado actual y decide la siguiente transición basada en eventos.
//! 4.  **Subsistema de Heartbeats:** Genera latidos periódicos hacia los Hubs y monitorea timeouts.
//!


use chrono::Utc;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument};
use crate::config::fsm::{PERCENTAGE, PHASE_MAX_DURATION, SAFE_MODE_MAX_DURATION};
use crate::context::domain::AppContext;
use crate::fsm::domain::{Action, Event, FsmServiceCommand, FsmServiceResponse, FsmState, 
                         StateOfSession, SubStateBalanceMode, SubStatePhase, SubStateQuorum, 
                         Transition, UpdateSession};
use crate::message::domain::{HandshakeToHub, Heartbeat, HubMessage, MessageStateBalanceMode, 
                             MessageStateNormal, MessageStateSafeMode, Metadata, PhaseNotification, Ping};


/// Tarea principal asíncrona que gestiona la orquestación de mensajes y eventos de la FSM.
///
/// Actúa como un **Event Loop** que escucha múltiples canales utilizando `tokio::select!`.
/// Prioriza la recepción de mensajes para mantener la reactividad del sistema.
///
/// # Canales Monitorizados
/// * `rx_from_hub`: Mensajes provenientes de los dispositivos Hubs.
/// * `rx_from_server`: Mensajes provenientes del servidor central.
/// * `rx_from_fsm`: Vectores de acciones generados por la lógica pura de la FSM (`run_fsm`) que deben ejecutarse.
///
/// # Lógica Principal
/// 1.  **Handshakes:** Gestiona la confirmación de épocas de balanceo.
/// 2.  **Quorum:** Evalúa mensajes `EmptyQueue` para determinar si el sistema puede transicionar de estado.
/// 3.  **Ping/Pong:** Responde automáticamente a solicitudes de diagnóstico de red.
/// 4.  **Ejecución de Acciones:** Delega las acciones recibidas a `handle_action`.
#[instrument(name = "fsm", skip(rx_command, rx_from_fsm, app_context))]
pub async fn fsm(tx_to_core: mpsc::Sender<FsmServiceResponse>,
                                  tx_to_fsm: mpsc::Sender<Event>,
                                  tx_to_timer: mpsc::Sender<Event>,
                                  tx_to_heartbeat: mpsc::Sender<Action>,
                                  mut rx_command: mpsc::Receiver<FsmServiceCommand>,
                                  mut rx_from_fsm: mpsc::Receiver<Vec<Action>>,
                                  app_context: AppContext,
                                  cancel: CancellationToken) {

    info!("Info: iniciando tarea fsm");
    let mut current_epoch: u32 = 0;
    let mut session: UpdateSession = UpdateSession::new();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("Info: shutdown recibido fsm");
                break;
            }

            Some(msg) = rx_command.recv() => {
                match msg {
                    FsmServiceCommand::FromHub(hub_msg) => {
                        match hub_msg {
                            HubMessage::HandshakeFromHub(handshake) => {
                                debug!("Debug: mensaje entrante de HandshakeFromHub");
                                match session.get_state() {
                                    StateOfSession::InHandshake | StateOfSession::OutHandshake | StateOfSession::RepeatHandshake => {
                                        if handshake.balance_epoch == current_epoch {
                                            session.insert_handshake(handshake.metadata.sender_user_id, handshake.balance_epoch);
                                        }
                                    },
                                    _ => {}
                                }
                            },
                            HubMessage::EmptyQueue(msg) => {
                                debug!("Debug: mensaje entrante de EmptyQueue");
                                match session.get_state() {
                                    StateOfSession::PhaseAlert | StateOfSession::PhaseData | StateOfSession::PhaseMonitor => {
                                        quorum_phase(&mut session,
                                                     &tx_to_fsm,
                                                     &app_context,
                                                     HubMessage::EmptyQueue(msg),
                                                     &tx_to_timer).await;
                                    },
                                    _ => {}
                                }
                            },
                            HubMessage::EmptyQueueSafe(msg) => {
                                debug!("Debug: mensaje entrante de EmptyQueueSafe");
                                match session.get_state() {
                                    StateOfSession::SafeMode => {
                                        quorum_safe_mode(&mut session,
                                                         &tx_to_fsm,
                                                         &app_context,
                                                         HubMessage::EmptyQueueSafe(msg),
                                                         &tx_to_timer).await;
                                    },
                                    _ => {}
                                }
                            },
                            HubMessage::Ping(msg) => {
                                debug!("Debug: mensaje entrante de Ping");
                                if msg.ping {
                                    let metadata = build_metadata(&app_context, &msg.metadata.sender_user_id.clone());
                                    let ping_ack = Ping {
                                        metadata,
                                        network: msg.network,
                                        ping: true,
                                    };
                                    if tx_to_core.send(FsmServiceResponse::ToHub(HubMessage::Ping(ping_ack))).await.is_err() {
                                        error!("Error: no se pudo enviar mensaje Ping ack al hub");
                                    }
                                }
                            },
                            _ => {}
                        }
                    },
                    FsmServiceCommand::ErrorEpoch => {
                        if tx_to_fsm.send(Event::BalanceEpochNotOk).await.is_err() {
                            error!("Error: no se pudo enviar comando BalanceEpochNotOk");
                        }
                    },
                    FsmServiceCommand::Epoch(epoch) => {
                        current_epoch = epoch + 1;
                        if tx_to_fsm.send(Event::BalanceEpochOk).await.is_err() {
                            error!("Error: no se pudo enviar comando BalanceEpochOk");
                        }
                        if tx_to_core.send(FsmServiceResponse::NewEpoch(current_epoch)).await.is_err() {
                            error!("Error: no se pudo enviar comando NewEpoch");
                        }
                    }
                    _ => {}
                }
            }

            Some(vec_action) = rx_from_fsm.recv() => {
                debug!("Debug: vector de acciones entrante");
                for action in vec_action {
                    handle_action(
                        action,
                        &app_context,
                        &tx_to_core,
                        &tx_to_fsm,
                        &tx_to_timer,
                        &tx_to_heartbeat,
                        &mut current_epoch,
                        &mut session
                    ).await;
                }
            }
        }
    }
}


/// Ejecutor de efectos secundarios (Side-Effects Handler).
///
/// Esta función recibe una `Action` abstracta (definida en el dominio de la FSM) y realiza
/// la operación concreta requerida. Esto desacopla la lógica de decisión de la implementación de E/S.
///
/// # Acciones Manejadas
/// * **Inicialización:** Configura timers y saluda al servidor.
/// * **Modos de Balanceo:** Gestiona la entrada/salida de handshakes y actualiza el epoch en DB.
/// * **Quorums:** Ejecuta los algoritmos de verificación de votos.
/// * **Fases:** Configura notificaciones de fase (Alerta, Datos, Monitor).
/// * **Heartbeats:** Controla el inicio y parada del generador de latidos.
async fn handle_action(action: Action,
                       app_context: &AppContext,
                       tx_to_core: &mpsc::Sender<FsmServiceResponse>,
                       tx_to_fsm: &mpsc::Sender<Event>,
                       tx_to_timer: &mpsc::Sender<Event>,
                       tx_to_heartbeat: &mpsc::Sender<Action>,
                       current_epoch: &mut u32,
                       session: &mut UpdateSession) {

    debug!("Debug: entrando a handle_action");
    
    match action {
        Action::OnEntryBalance(sub_bm) => {
            debug!("Debug: acción recibida Action::OnEntryBalance");
            match sub_bm {
                SubStateBalanceMode::InitBalanceMode => {
                    on_entry_init_balance_mode(tx_to_core).await;
                },
                SubStateBalanceMode::InHandshake => {
                    session.set_state(StateOfSession::InHandshake);
                    on_entry_in_handshake(tx_to_core, tx_to_timer, current_epoch, app_context.clone(), sub_bm).await;
                },
                SubStateBalanceMode::OutHandshake => {
                    session.set_state(StateOfSession::OutHandshake);
                    on_entry_out_handshake(tx_to_core, tx_to_timer, current_epoch, app_context.clone(), sub_bm).await;
                },
                _ => {},
            }
        },
        Action::OnEntryQuorum(sub_q) => {
            debug!("Debug: acción recibida Action::OnEntryQuorum");
            match sub_q {
                SubStateQuorum::CheckQuorumIn | SubStateQuorum::CheckQuorumOut => {
                    session.set_state(StateOfSession::Quorum);
                    quorum_algorithm(session, tx_to_fsm, app_context).await;
                },
                SubStateQuorum::RepeatHandshakeIn | SubStateQuorum::RepeatHandshakeOut => {
                    session.set_state(StateOfSession::RepeatHandshake);
                    session.increment_attempts();
                    on_entry_repeat_handshake(tx_to_core, tx_to_timer, current_epoch, app_context, sub_q).await;
                },
            }
        },
        Action::OnEntryPhase(sub_p) => {
            debug!("Debug: acción recibida Action::OnEntryPhase");
            session.reset_total_attempts();
            session.reset_handshake_hash();
            session.reset_empty_hash();
            match sub_p {
                SubStatePhase::Alert => {
                    session.set_state(StateOfSession::PhaseAlert);
                    on_entry_alert(tx_to_core, tx_to_timer, tx_to_heartbeat, current_epoch, app_context).await;
                },
                SubStatePhase::Data => {
                    session.set_state(StateOfSession::PhaseData);
                    on_entry_data(tx_to_core, tx_to_timer, current_epoch, app_context).await;
                },
                SubStatePhase::Monitor => {
                    session.set_state(StateOfSession::PhaseMonitor);
                    on_entry_monitor(tx_to_core, tx_to_timer, current_epoch, app_context).await;
                },
            }
        },
        Action::OnEntryNormal => {
            debug!("Debug: acción recibida Action::OnEntryNormal");
            on_entry_normal(tx_to_core, tx_to_heartbeat, app_context).await;
        },
        Action::OnEntrySafeMode => {
            debug!("Debug: acción recibida Action::OnEntrySafeMode");
            session.reset_empty_hash();
            session.set_state(StateOfSession::SafeMode);
            on_entry_safe_mode(tx_to_core, tx_to_timer, tx_to_heartbeat, current_epoch, app_context).await;
        }
        Action::StopTimer => {
            debug!("Debug: acción recibida Action::StopTimer");
            if tx_to_timer.send(Event::StopTimer).await.is_err() {
                error!("Error: no se pudo enviar evento de finalización de watchdog de fsm general");
            }
        }
        Action::StopSendHeartbeatMessagePhase => {
            debug!("Debug: acción recibida Action::StopSendHeartbeatMessagePhase");
            if tx_to_heartbeat.send(Action::StopSendHeartbeatMessagePhase).await.is_err() {
                error!("Error: no se pudo enviar acción StopSendHeartbeatMessagePhase");
            }
        },
        Action::StopSendHeartbeatMessageSafeMode => {
            debug!("Debug: acción recibida Action::StopSendHeartbeatMessageSafeMode");
            if tx_to_heartbeat.send(Action::StopSendHeartbeatMessageSafeMode).await.is_err() {
                error!("Error: no se pudo enviar acción StopSendHeartbeatMessageSafeMode");
            }
        },
        _ => {}
    }
}


async fn init_timer(tx_to_timer: &mpsc::Sender<Event>, duration: Duration) {
    if tx_to_timer.send(Event::InitTimer(duration)).await.is_err() {
        error!("Error: no se pudo enviar evento de inicialización del watchdog de fsm general");
    }
}


async fn on_entry_init_balance_mode(tx_to_core: &mpsc::Sender<FsmServiceResponse>) {
    debug!("Debug: entrando a on_entry_init_balance_mode");
    let mut flag = true;
    loop {
        if tx_to_core.send(FsmServiceResponse::GetEpoch).await.is_err() {
            error!("Error: no se pudo enviar comando GetEpoch");
            flag = false;
        }
        if flag {
            break;
        } else {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

}


async fn on_entry_in_handshake(tx_to_core: &mpsc::Sender<FsmServiceResponse>,
                               tx_to_timer: &mpsc::Sender<Event>,
                               current_epoch: &u32,
                               app_context: AppContext,
                               state: SubStateBalanceMode) {

    debug!("Debug: entrando a on_entry_in_handshake");
    let metadata = build_metadata(&app_context, "all");
    send_balance_state(tx_to_core, metadata.clone(), *current_epoch, "in_handshake").await;
    send_handshake(tx_to_core, metadata, *current_epoch, state).await;
    init_timer(tx_to_timer, Duration::from_secs(180)).await;
}


async fn on_entry_out_handshake(tx_to_core: &mpsc::Sender<FsmServiceResponse>,
                                tx_to_timer: &mpsc::Sender<Event>,
                                current_epoch: &u32,
                                app_context: AppContext,
                                state: SubStateBalanceMode) {

    debug!("Debug: entrando a on_entry_out_handshake");
    let metadata = build_metadata(&app_context, "all");
    send_balance_state(tx_to_core, metadata.clone(), *current_epoch, "out_handshake").await;
    send_handshake(tx_to_core, metadata.clone(), *current_epoch, state).await;
    init_timer(tx_to_timer, Duration::from_secs(180)).await;
}


/// Algoritmo de Quorum para Handshakes.
///
/// Determina si suficientes Hubs han respondido para proceder al siguiente estado.
///
/// # Lógica
/// 1. Verifica si se han excedido los intentos máximos.
/// 2. Calcula el umbral de aceptación basado en `PERCENTAGE` y penalización por intentos.
/// 3. Envía `ApproveQuorum` o `NotApproveQuorum` a la FSM.
async fn quorum_algorithm(session: &mut UpdateSession,
                          tx_to_fsm: &mpsc::Sender<Event>,
                          app_context: &AppContext) {

    debug!("Debug: iniciando quorum algorithm");
    let max_attempts = app_context.quorum.get_max_attempts();

    if session.get_total_attempts() > max_attempts as f64 {
        debug!("Debug: no mas intentos de quorum");
        if tx_to_fsm.send(Event::NotApproveNotAttempts).await.is_err() {
            error!("Error: no se pudo enviar evento NotApproveNotAttempts a la fsm general");
        }
    } else {
        let manager = app_context.net_man.read().await;
        let total_hubs = manager.get_total_hubs();
        drop(manager);
        let threshold = PERCENTAGE - (session.get_total_attempts() * 5.0 - 5.0);

        if ((session.get_total_handshake() as f64 / total_hubs as f64) >= threshold) && threshold > 10.0 {
            debug!("Debug: quorum aprobado");
            if tx_to_fsm.send(Event::ApproveQuorum).await.is_err() {
                error!("Error: no se pudo enviar evento ApproveQuorum a la fsm general");
            }
        } else {
            debug!("Debug: quorum desaprobado");
            session.reset_handshake_hash();
            if tx_to_fsm.send(Event::NotApproveQuorum).await.is_err() {
                error!("Error: no se pudo enviar evento NotApproveQuorum a la fsm general");
            }
        }
    }
}


/// Algoritmo de Quorum para vaciado de colas (EmptyQueue).
///
/// Verifica si un porcentaje (>= 80%) de los hubs han reportado que sus colas están vacías.
/// Si se cumple, dispara el evento `QuorumPhase` y detiene el watchdog.
async fn quorum_phase(session: &mut UpdateSession,
                      tx_to_fsm: &mpsc::Sender<Event>,
                      app_context: &AppContext,
                      msg: HubMessage,
                      tx_to_timer: &mpsc::Sender<Event>) {

    debug!("Debug: iniciando quorum phase");
    match msg {
        HubMessage::EmptyQueue(empty) => {
            if empty.queue_empty {
                debug!("Debug: mensaje de cola vacía recibido");
                session.insert_empty(empty.metadata.sender_user_id, empty.queue_empty);
                let total_empty_msg = session.get_total_empty();
                let manager = app_context.net_man.read().await;
                let total_hubs = manager.get_total_hubs();
                let percentage = (total_empty_msg/total_hubs) as f64 * 100.0;
                drop(manager);
                if percentage >= 80.0 {
                    debug!("Debug: aprobado el QuorumPhase");
                    if tx_to_fsm.send(Event::QuorumPhase).await.is_err() {
                        error!("Error: no se pudo enviar evento QuorumPhase a la fsm general");
                    }
                    if tx_to_timer.send(Event::StopTimer).await.is_err() {
                        error!("Error: no se pudo enviar evento de finalización de watchdog de QuorumPhase");
                    }
                }
            }
        }
        _ => {}
    }
}


/// Algoritmo de Quorum para vaciado de colas (EmptyQueue).
///
/// Verifica si un porcentaje (>= 70%) de los hubs han reportado que sus colas están vacías.
/// Si se cumple, dispara el evento `QuorumSafeMode` y detiene el watchdog.
async fn quorum_safe_mode(session: &mut UpdateSession,
                         tx_to_fsm: &mpsc::Sender<Event>,
                         app_context: &AppContext,
                         msg: HubMessage,
                         tx_to_timer: &mpsc::Sender<Event>) {

    debug!("Debug: iniciando quorum_safe_mode");
    match msg {
        HubMessage::EmptyQueueSafe(empty) => {
            if empty.queue_empty {
                debug!("Debug: mensaje de cola vacía SafeMode recibido");
                session.insert_empty(empty.metadata.sender_user_id, empty.queue_empty);
                let total_empty_msg = session.get_total_empty();
                let manager = app_context.net_man.read().await;
                let total_hubs = manager.get_total_hubs();
                let percentage = (total_empty_msg/total_hubs) as f64 * 100.0;
                drop(manager);
                if percentage >= 70.0 {
                    debug!("Debug: quorum de SafeMode aprobado");
                    if tx_to_fsm.send(Event::QuorumSafeMode).await.is_err() {
                        error!("Error: no se pudo enviar evento QuorumSafeMode a la fsm general");
                    }
                    if tx_to_timer.send(Event::StopTimer).await.is_err() {
                        error!("Error: no se pudo enviar evento de finalización de watchdog de QuorumSafeMode");
                    }
                }
            }
        }
        _ => {}
    }
}


async fn on_entry_repeat_handshake(tx_to_core: &mpsc::Sender<FsmServiceResponse>,
                                   tx_to_timer: &mpsc::Sender<Event>,
                                   current_epoch: &u32,
                                   app_context: &AppContext,
                                   state: SubStateQuorum) {

    debug!("Debug: iniciando on_entry_repeat_handshake");
    match state {
        SubStateQuorum::RepeatHandshakeIn => {
            debug!("Debug: preparando mensaje Handshake en estado RepeatHandshakeIn");
            let metadata = build_metadata(app_context, "all");
            let handshake = HandshakeToHub {
                metadata,
                flag: "in".to_string(),
                balance_epoch: *current_epoch,
            };
            if tx_to_core.send(FsmServiceResponse::ToHub(HubMessage::HandshakeToHub(handshake))).await.is_err() {
                error!("Error: no se pudo enviar mensaje HandshakeToHub");
            }
        },
        SubStateQuorum::RepeatHandshakeOut => {
            debug!("Debug: preparando mensaje Handshake en estado RepeatHandshakeOut");
            let metadata = build_metadata(app_context, "all");
            let handshake = HandshakeToHub {
                metadata,
                flag: "out".to_string(),
                balance_epoch: *current_epoch,
            };
            if tx_to_core.send(FsmServiceResponse::ToHub(HubMessage::HandshakeToHub(handshake))).await.is_err() {
                error!("Error: no se pudo enviar mensaje HandshakeToHub");
            }
        }
        _ => {}
    }

    init_timer(tx_to_timer, Duration::from_secs(180)).await;
}


async fn on_entry_alert(tx_to_core: &mpsc::Sender<FsmServiceResponse>,
                        tx_to_timer: &mpsc::Sender<Event>,
                        tx_to_heartbeat: &mpsc::Sender<Action>,
                        current_epoch: &u32,
                        app_context: &AppContext) {

    debug!("Debug: entrando a on_entry_alert");
    init_timer(tx_to_timer, Duration::from_secs(PHASE_MAX_DURATION)).await;
    let metadata = build_metadata(app_context, "all");
    send_balance_state(tx_to_core, metadata.clone(), *current_epoch, "phase").await;
    send_phase_notification(tx_to_core, metadata, *current_epoch).await;

    if tx_to_heartbeat.send(Action::SendHeartbeatMessagePhase).await.is_err() {
        error!("Error: no se pudo enviar acción SendHeartbeatMessagePhase");
    }
}


async fn on_entry_data(tx_to_core: &mpsc::Sender<FsmServiceResponse>,
                       tx_to_timer: &mpsc::Sender<Event>,
                       current_epoch: &u32,
                       app_context: &AppContext) {

    debug!("Debug: entrando a on_entry_data");
    init_timer(tx_to_timer, Duration::from_secs(PHASE_MAX_DURATION)).await;
    let metadata = build_metadata(app_context, "all");
    send_balance_state(tx_to_core, metadata.clone(), *current_epoch, "phase").await;
    send_phase_notification(tx_to_core, metadata, *current_epoch).await;
}


async fn on_entry_monitor(tx_to_core: &mpsc::Sender<FsmServiceResponse>,
                          tx_to_timer: &mpsc::Sender<Event>,
                          current_epoch: &u32,
                          app_context: &AppContext) {

    debug!("Debug: entrando a on_entry_monitor");
    init_timer(tx_to_timer, Duration::from_secs(PHASE_MAX_DURATION)).await;
    let metadata = build_metadata(app_context, "all");
    send_balance_state(tx_to_core, metadata.clone(), *current_epoch, "phase").await;
    send_phase_notification(tx_to_core, metadata, *current_epoch).await;
}


async fn on_entry_normal(tx_to_core: &mpsc::Sender<FsmServiceResponse>,
                         tx_to_heartbeat: &mpsc::Sender<Action>,
                         app_context: &AppContext) {

    debug!("Debug: entrando a on_entry_normal");
    let metadata = build_metadata(app_context, "all");
    let state = MessageStateNormal {
        metadata,
        state: "normal".to_string(),
    };

    if tx_to_core.send(FsmServiceResponse::ToHub(HubMessage::StateNormal(state))).await.is_err() {
        error!("Error: no se pudo enviar el mensaje de Normal a los hubs");
    }

    if tx_to_heartbeat.send(Action::SendHeartbeatMessageNormal).await.is_err() {
        error!("Error: no se pudo enviar acción SendHeartbeatMessageNormal");
    }
}


async fn on_entry_safe_mode(tx_to_core: &mpsc::Sender<FsmServiceResponse>,
                            tx_to_timer: &mpsc::Sender<Event>,
                            tx_to_heartbeat: &mpsc::Sender<Action>,
                            current_epoch: &u32,
                            app_context: &AppContext) {

    debug!("Debug: entrando a on_entry_safe_mode");
    init_timer(tx_to_timer, Duration::from_secs(SAFE_MODE_MAX_DURATION)).await;  

    let metadata = build_metadata(app_context, "all");
    let jitter = fastrand::u32(0..=5);
    let state = MessageStateSafeMode {
        metadata: metadata.clone(),
        state: "safe_mode".to_string(),
        frequency: 4,
        jitter,
    };

    if tx_to_core.send(FsmServiceResponse::ToHub(HubMessage::StateSafeMode(state))).await.is_err() {
        error!("Error: no se pudo enviar mensaje de SafeMode a los hubs");
    }

    send_phase_notification(tx_to_core, metadata, *current_epoch).await;

    if tx_to_heartbeat.send(Action::SendHeartbeatMessageSafeMode).await.is_err() {
        error!("Error: no se pudo enviar acción SendHeartbeatMessageSafeMode");
    }
}


/// Tarea asíncrona que ejecuta la lógica pura de la Máquina de Estados.
///
/// Mantiene el estado persistente (`FsmState`) y avanza pasos tras recibir eventos.
///
/// * `tx_actions`: Canal para emitir los efectos secundarios que deben ejecutarse.
/// * `rx_event`: Canal de entrada de eventos (triggers).
#[instrument(name = "run_fsm", skip(rx_event))]
pub async fn run_fsm(tx_actions: mpsc::Sender<Vec<Action>>,
                     mut rx_event: mpsc::Receiver<Event>,
                     cancel: CancellationToken) {

    info!("Info: iniciando tarea fsm");
    let mut state = FsmState::new();

    handle_transition(state.step(Event::Start), &mut state, &tx_actions).await;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("Info: shutdown recibido run_fsm");
                break;
            }
            Some(event) = rx_event.recv() => {
                debug!("Debug: evento recibido en run_fsm");
                handle_transition(state.step(event), &mut state, &tx_actions).await;
            }
        }
    }
}


/// Watchdog Timer (Perro guardián) para el envío de Heartbeats.
///
/// Si este timer expira sin ser reseteado o detenido, envía un evento `Timeout`
/// que fuerza el envío de un nuevo latido.
#[instrument(name = "heartbeat_to_send_watchdog_timer", skip(cmd_rx))]
pub async fn heartbeat_generator_timer(tx_to_heartbeat: mpsc::Sender<Event>,
                                       mut cmd_rx: mpsc::Receiver<Event>,
                                       cancel: CancellationToken) {

    info!("Info: iniciando heartbeat_generator_timer");
    loop {
        let duration = match cmd_rx.recv().await {
            Some(Event::InitTimer(d)) => d,
            Some(Event::StopTimer) => continue,
            None => break,
            _ => continue,
        };

        tokio::select! {
            _ = cancel.cancelled() => {
                info!("Info: shutdown recibido heartbeat_generator_timer");
                break;
            }
            _ = sleep(duration) => {
                if tx_to_heartbeat.send(Event::Timeout).await.is_err() {
                    error!("Error: no se pudo enviar evento Timeout");
                }
            }
            Some(Event::StopTimer) = cmd_rx.recv() => {
                debug!("Debug: watchdog timer de heartbeat_generator_timer, cancelado");
            }
        }
    }
}


/// Generador de mensajes Heartbeat.
///
/// Gestiona la cadencia y el tipo de mensaje de latido (Heartbeat) enviado a los hubs
/// dependiendo del estado actual (Phase, Normal, SafeMode).
#[instrument(name = "heartbeat_generator", skip(rx_from_fsm, cmd_rx, app_context))]
pub async fn heartbeat_generator(tx_to_core: mpsc::Sender<FsmServiceResponse>,
                                 tx_to_timer: mpsc::Sender<Event>,
                                 mut rx_from_fsm: mpsc::Receiver<Action>,
                                 mut cmd_rx: mpsc::Receiver<Event>,
                                 app_context: AppContext,
                                 cancel: CancellationToken) {

    info!("Info: iniciando heartbeat_generator");
    enum HeartbeatState {
        BalanceMode,
        Normal,
        SafeMode,
        None,
    }

    let mut beat = HeartbeatState::None;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("Info: shutdown recibido heartbeat_generator");
                break;
            }
            Some(action) = rx_from_fsm.recv() => {
                match action {
                    Action::SendHeartbeatMessagePhase => {
                        debug!("Debug: acción recibida en heartbeat_generator. Action::SendHeartbeatMessagePhase");
                        beat = HeartbeatState::BalanceMode;
                        let duration = app_context.quorum.get_time_between_heartbeats_balance_mode();
                        if tx_to_timer.send(Event::InitTimer(Duration::from_secs(duration))).await.is_err() {
                            error!("Error: no se pudo enviar evento InitTimer");
                        }
                    },
                    Action::SendHeartbeatMessageNormal => {
                        debug!("Debug: acción recibida en heartbeat_generator. Action::SendHeartbeatMessageNormal");
                        beat = HeartbeatState::Normal;
                        let duration = app_context.quorum.get_time_between_heartbeats_normal();
                        if tx_to_timer.send(Event::InitTimer(Duration::from_secs(duration))).await.is_err() {
                            error!("Error: no se pudo enviar evento InitTimer");
                        }
                    },
                    Action::SendHeartbeatMessageSafeMode => {
                        debug!("Debug: acción recibida en heartbeat_generator. Action::SendHeartbeatMessageSafeMode");
                        beat = HeartbeatState::SafeMode;
                        let duration = app_context.quorum.get_time_between_heartbeats_safe_mode();
                        if tx_to_timer.send(Event::InitTimer(Duration::from_secs(duration))).await.is_err() {
                            error!("Error: no se pudo enviar evento InitTimer");
                        }
                    },
                    Action::StopSendHeartbeatMessagePhase | Action::StopSendHeartbeatMessageSafeMode => {
                        debug!("Debug: acción recibida en heartbeat_generator. Action::StopHeartbeat");
                        if tx_to_timer.send(Event::StopTimer).await.is_err() {
                            error!("Error: no se pudo enviar evento StopTimer");
                        }
                    }
                    _ => {}
                }
            }

            Some(Event::Timeout) = cmd_rx.recv() => {
                send_heartbeat(&tx_to_core, &app_context).await;
                let duration = match beat {
                    HeartbeatState::BalanceMode => Duration::from_secs(app_context.quorum.get_time_between_heartbeats_balance_mode()),
                    HeartbeatState::Normal => Duration::from_secs(app_context.quorum.get_time_between_heartbeats_normal()),
                    HeartbeatState::SafeMode => Duration::from_secs(app_context.quorum.get_time_between_heartbeats_safe_mode()),
                    HeartbeatState::None => continue,
                };
                if tx_to_timer.send(Event::InitTimer(duration)).await.is_err() {
                    error!("Error: no se pudo enviar evento de InitTimer");
                }
            }
        }
    }
}


// ===== Funciones auxiliares =====


fn build_metadata(app_context: &AppContext, destination: &str) -> Metadata {
    Metadata {
        sender_user_id: app_context.system.id_edge.clone(),
        destination_id: destination.to_string(),
        timestamp: Utc::now().timestamp(),
    }
}


async fn send_balance_state(tx_to_core: &mpsc::Sender<FsmServiceResponse>,
                            metadata: Metadata,
                            epoch: u32,
                            sub_state: &str) {

    let state = MessageStateBalanceMode {
        metadata,
        state: "balance_mode".to_string(),
        balance_epoch: epoch,
        sub_state: sub_state.to_string(),
        duration: 5,
    };

    if tx_to_core.send(FsmServiceResponse::ToHub(HubMessage::StateBalanceMode(state))).await.is_err() {
        error!("Error: no se pudo enviar mensaje StateBalanceMode");
    }
}


async fn send_handshake(tx: &mpsc::Sender<FsmServiceResponse>,
                        metadata: Metadata,
                        epoch: u32,
                        state: SubStateBalanceMode) {

    match state {
        SubStateBalanceMode::InHandshake => {
            let msg = HandshakeToHub {
                metadata,
                flag: "in".to_string(),
                balance_epoch: epoch,
            };
            if tx.send(FsmServiceResponse::ToHub(HubMessage::HandshakeToHub(msg))).await.is_err() {
                error!("Error: no se pudo enviar mensaje HandshakeToHub");
            }
        },
        SubStateBalanceMode::OutHandshake => {
            let msg = HandshakeToHub {
                metadata,
                flag: "out".to_string(),
                balance_epoch: epoch,
            };
            if tx.send(FsmServiceResponse::ToHub(HubMessage::HandshakeToHub(msg))).await.is_err() {
                error!("Error: no se pudo enviar mensaje HandshakeToHub");
            }
        }
        _ => {}
    }

}


async fn send_phase_notification(tx_to_hub: &mpsc::Sender<FsmServiceResponse>,
                                 metadata: Metadata,
                                 epoch: u32) {

    let phase = PhaseNotification {
        metadata,
        state: "a".to_string(),
        epoch,
        phase: "a".to_string(),
        frequency: 1,
        jitter: 2,
    };

    if tx_to_hub.send(FsmServiceResponse::ToHub(HubMessage::PhaseNotification(phase))).await.is_err() {
        error!("Error: no se pudo enviar PhaseNotification");
    }
}


async fn send_heartbeat(tx_to_core: &mpsc::Sender<FsmServiceResponse>, app_context: &AppContext) {
    let metadata = build_metadata(app_context, "all");
    let heartbeat = Heartbeat {
        metadata,
        beat: true,
    };

    if tx_to_core.send(FsmServiceResponse::ToHub(HubMessage::Heartbeat(heartbeat))).await.is_err() {
        error!("Error: no se pudo enviar Heartbeat");
    }
}


/// Procesa una transición de la FSM.
///
/// Si la transición es válida, actualiza el estado y transmite las acciones resultantes.
/// Si es inválida, loguea un error.
async fn handle_transition(transition: Transition,
                           state: &mut FsmState,
                           tx: &mpsc::Sender<Vec<Action>>) {

    match transition {
        Transition::Valid(t) => {
            *state = t.get_change_state();
            let _ = tx.send(t.get_actions()).await;
        }
        Transition::Invalid(t) => {
            error!("FSM transición inválida: {}", t.get_invalid());
        }
    }
}