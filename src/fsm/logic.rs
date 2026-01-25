use chrono::Utc;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tracing::{debug, error};
use crate::config::fsm::PERCENTAGE;
use crate::context::domain::AppContext;
use crate::fsm::domain::{Action, Event, FsmState, StateOfSession, SubStateBalanceMode, SubStatePhase, SubStateQuorum, Transition, UpdateSession};
use crate::message::domain::{HandshakeToHub, Heartbeat, HelloWorld, Message, MessageStateBalanceMode, MessageStateNormal, MessageStateSafeMode, Metadata, PhaseNotification};


pub async fn fsm_general_channels(tx_to_hub: mpsc::Sender<Message>,
                                  tx_to_server: mpsc::Sender<Message>,
                                  tx_to_fsm: mpsc::Sender<Event>,
                                  tx_to_timer: mpsc::Sender<Event>,
                                  tx_to_heartbeat: mpsc::Sender<Action>,
                                  mut rx_from_hub: mpsc::Receiver<Message>,
                                  mut rx_from_fsm: mpsc::Receiver<Vec<Action>>,
                                  app_context: AppContext) {

    let mut current_epoch: u32 = 0;
    let mut session: UpdateSession = UpdateSession::new();

    loop {
        tokio::select! {
            Some(msg_from_hub) = rx_from_hub.recv() => {
                if let Message::HandshakeFromHub(_) = msg_from_hub {
                    match session.get_state() {
                        StateOfSession::InHandshake | StateOfSession::OutHandshake | StateOfSession::RepeatHandshake => {
                            session.increment_messages();
                        },
                        _ => {}
                    }
                }
            }

            Some(vec_action) = rx_from_fsm.recv() => {
                for action in vec_action {
                    handle_action(
                        action,
                        &app_context,
                        &tx_to_hub,
                        &tx_to_server,
                        &tx_to_fsm,
                        &tx_to_timer,
                        &tx_to_heartbeat,
                        &mut current_epoch,
                        &mut session,
                    ).await;
                }
            }
        }
    }
}


async fn handle_action(action: Action,
                       app_context: &AppContext,
                       tx_to_hub: &mpsc::Sender<Message>,
                       tx_to_server: &mpsc::Sender<Message>,
                       tx_to_fsm: &mpsc::Sender<Event>,
                       tx_to_timer: &mpsc::Sender<Event>,
                       tx_to_heartbeat: &mpsc::Sender<Action>,
                       current_epoch: &mut u32,
                       session: &mut UpdateSession) {

    match action {
        Action::OnEntryInit(_) => {
            on_entry_init(app_context, tx_to_timer, tx_to_server).await;
        },
        Action::OnEntryBalance(sub_bm) => {
            match sub_bm {
                SubStateBalanceMode::InitBalanceMode => {
                    on_entry_init_balance_mode(app_context, tx_to_fsm, current_epoch).await;
                },
                SubStateBalanceMode::InHandshake => {
                    session.set_state(StateOfSession::InHandshake);
                    on_entry_in_handshake(tx_to_hub, tx_to_server, tx_to_timer, current_epoch, app_context.clone()).await;
                },
                SubStateBalanceMode::OutHandshake => {
                    session.set_state(StateOfSession::OutHandshake);
                    on_entry_out_handshake(tx_to_hub, tx_to_server, tx_to_timer, current_epoch, app_context.clone()).await;
                },
                _ => {},
            }
        },
        Action::OnEntryQuorum(sub_q) => {
            match sub_q {
                SubStateQuorum::CheckQuorumIn | SubStateQuorum::CheckQuorumOut => {
                    session.set_state(StateOfSession::Quorum);
                    quorum_algorithm(session, tx_to_fsm, app_context).await;
                },
                SubStateQuorum::RepeatHandshakeIn | SubStateQuorum::RepeatHandshakeOut => {
                    session.set_state(StateOfSession::RepeatHandshake);
                    session.increment_attempts();
                    on_entry_repeat_handshake(tx_to_hub, tx_to_timer, current_epoch, app_context).await;
                },
            }
        },
        Action::OnEntryPhase(sub_p) => {
            session.reset();
            match sub_p {
                SubStatePhase::Alert => {
                    on_entry_alert(tx_to_hub, tx_to_server, tx_to_timer, tx_to_heartbeat, current_epoch, app_context).await;
                },
                SubStatePhase::Data => {
                    on_entry_data(tx_to_hub, tx_to_server, tx_to_timer, current_epoch, app_context).await;
                },
                SubStatePhase::Monitor => {
                    on_entry_monitor(tx_to_hub, tx_to_server, tx_to_timer, current_epoch, app_context).await;
                },
            }
        },
        Action::OnEntryNormal => {
            on_entry_normal(tx_to_hub, tx_to_server, tx_to_heartbeat, app_context).await;
        },
        Action::OnEntrySafeMode => {
            on_entry_safe_mode(tx_to_hub, tx_to_server, tx_to_timer, tx_to_heartbeat, current_epoch, app_context).await;
        }
        Action::StopTimer => {
            if tx_to_timer.send(Event::StopTimer).await.is_err() {
                error!("Error: No se pudo enviar evento de finalización de watchdog de fsm general");
            }
        }
        Action::StopSendHeartbeatMessagePhase => {
            if tx_to_heartbeat.send(Action::StopSendHeartbeatMessagePhase).await.is_err() {
                error!("Error: No se pudo enviar acción StopSendHeartbeatMessagePhase");
            }
        },
        Action::StopSendHeartbeatMessageSafeMode => {
            if tx_to_heartbeat.send(Action::StopSendHeartbeatMessageSafeMode).await.is_err() {
                error!("Error: No se pudo enviar acción StopSendHeartbeatMessageSafeMode");
            }
        },
        _ => {}
    }
}


async fn on_entry_init(app_context: &AppContext,
                       tx_to_timer: &mpsc::Sender<Event>,
                       tx_to_server: &mpsc::Sender<Message>) {

    let manager = app_context.quorum.read().await;
    let duration = Duration::from_secs(manager.get_hello_timeout());
    init_timer(tx_to_timer, duration).await;

    let metadata = build_metadata(app_context, "Server0");
    let msg = HelloWorld {
        metadata,
        hello: true,
    };

    send_message(tx_to_server, Message::HelloWorld(msg)).await;
}


async fn init_timer(tx_to_timer: &mpsc::Sender<Event>, duration: Duration) {
    if tx_to_timer.send(Event::InitTimer(duration)).await.is_err() {
        error!("Error: No se pudo enviar evento de inicialización del watchdog de fsm general");
    }
}


async fn update_balance_epoch(app_context: &AppContext, current_epoch: &mut u32) -> Result<(), sqlx::Error> {
    let epoch = app_context.repo.get_epoch().await?;
    let new_epoch = epoch + 1;
    app_context.repo.update_epoch(new_epoch).await?;
    *current_epoch = new_epoch;
    Ok(())
}


async fn on_entry_init_balance_mode(app_context: &AppContext,
                                    tx_to_fsm: &mpsc::Sender<Event>,
                                    current_epoch: &mut u32) {

    let event = match update_balance_epoch(app_context, current_epoch).await {
        Ok(_) => Event::BalanceEpochOk,
        Err(e) => {
            error!("Error: No se pudo actualizar epoch: {}", e);
            Event::BalanceEpochNotOk
        }
    };

    let _ = tx_to_fsm.send(event).await;
}


async fn on_entry_in_handshake(tx_to_hub: &mpsc::Sender<Message>,
                               tx_to_server: &mpsc::Sender<Message>,
                               tx_to_timer: &mpsc::Sender<Event>,
                               current_epoch: &u32,
                               app_context: AppContext) {

    let metadata = build_metadata(&app_context, "all");

    send_balance_state(tx_to_hub, tx_to_server, metadata.clone(), *current_epoch, "in_handshake").await;
    send_handshake(tx_to_hub, metadata, *current_epoch).await;
    init_timer(tx_to_timer, Duration::from_secs(180)).await;
}


async fn on_entry_out_handshake(tx_to_hub: &mpsc::Sender<Message>,
                                tx_to_server: &mpsc::Sender<Message>,
                                tx_to_timer: &mpsc::Sender<Event>,
                                current_epoch: &u32,
                                app_context: AppContext) {

    let metadata = build_metadata(&app_context, "all");

    send_balance_state(tx_to_hub, tx_to_server, metadata.clone(), *current_epoch, "out_handshake").await;
    send_handshake(tx_to_hub, metadata.clone(), *current_epoch).await;
    send_handshake(tx_to_server, metadata, *current_epoch).await;
    init_timer(tx_to_timer, Duration::from_secs(180)).await;
}


async fn quorum_algorithm(session: &mut UpdateSession,
                          tx_to_fsm: &mpsc::Sender<Event>,
                          app_context: &AppContext) {

    let manager = app_context.quorum.read().await;
    let max_attempts = manager.get_max_attempts();
    if session.get_total_attempts() > max_attempts as f64 {
        if tx_to_fsm.send(Event::NotApproveNotAttempts).await.is_err() {
            error!("Error: No se pudo enviar evento NotApproveNotAttempts a la fsm general");
        }
    } else {
        let manager = app_context.net_man.read().await;
        let total_hubs = manager.get_total_hubs();
        let threshold = PERCENTAGE - (session.get_total_attempts() * 5.0 - 5.0);

        if ((session.get_total_handshake() / total_hubs as f64) >= threshold) && threshold > 10.0 {
            if tx_to_fsm.send(Event::ApproveQuorum).await.is_err() {
                error!("Error: No se pudo enviar evento ApproveQuorum a la fsm general");
            }
        } else {
            session.reset_total_handshake_msg();
            if tx_to_fsm.send(Event::NotApproveQuorum).await.is_err() {
                error!("Error: No se pudo enviar evento NotApproveQuorum a la fsm general");
            }
        }
    }
}


async fn on_entry_repeat_handshake(tx_to_hub: &mpsc::Sender<Message>,
                                   tx_to_timer: &mpsc::Sender<Event>,
                                   current_epoch: &u32,
                                   app_context: &AppContext) {

    let metadata = build_metadata(app_context, "all");
    let handshake = HandshakeToHub {
        metadata,
        balance_epoch: *current_epoch,
        duration: 5,
    };

    send_message(tx_to_hub, Message::HandshakeToHub(handshake)).await;
    init_timer(tx_to_timer, Duration::from_secs(180)).await;
}


async fn on_entry_alert(tx_to_hub: &mpsc::Sender<Message>,
                        tx_to_server: &mpsc::Sender<Message>,
                        tx_to_timer: &mpsc::Sender<Event>,
                        tx_to_heartbeat: &mpsc::Sender<Action>,
                        current_epoch: &u32,
                        app_context: &AppContext) {

    init_timer(tx_to_timer, Duration::from_secs(180)).await;

    let metadata = build_metadata(app_context, "all");

    send_balance_state(tx_to_hub, tx_to_server, metadata.clone(), *current_epoch, "phase").await;
    send_phase_notification(tx_to_hub, metadata, *current_epoch).await;

    if tx_to_heartbeat.send(Action::SendHeartbeatMessagePhase).await.is_err() {
        error!("Error: No se pudo enviar acción SendHeartbeatMessagePhase");
    }
}


async fn on_entry_data(tx_to_hub: &mpsc::Sender<Message>,
                       tx_to_server: &mpsc::Sender<Message>,
                       tx_to_timer: &mpsc::Sender<Event>,
                       current_epoch: &u32,
                       app_context: &AppContext) {

    init_timer(tx_to_timer, Duration::from_secs(180)).await;

    let metadata = build_metadata(app_context, "all");

    send_balance_state(tx_to_hub, tx_to_server, metadata.clone(), *current_epoch, "phase").await;
    send_phase_notification(tx_to_hub, metadata, *current_epoch).await;
}


async fn on_entry_monitor(tx_to_hub: &mpsc::Sender<Message>,
                          tx_to_server: &mpsc::Sender<Message>,
                          tx_to_timer: &mpsc::Sender<Event>,
                          current_epoch: &u32,
                          app_context: &AppContext) {

    init_timer(tx_to_timer, Duration::from_secs(180)).await;

    let metadata = build_metadata(app_context, "all");

    send_balance_state(tx_to_hub, tx_to_server, metadata.clone(), *current_epoch, "phase").await;
    send_phase_notification(tx_to_hub, metadata, *current_epoch).await;
}


async fn on_entry_normal(tx_to_hub: &mpsc::Sender<Message>,
                         tx_to_server: &mpsc::Sender<Message>,
                         tx_to_heartbeat: &mpsc::Sender<Action>,
                         app_context: &AppContext) {

    let metadata = build_metadata(app_context, "all");
    let state = MessageStateNormal {
        metadata,
        state: "normal".to_string(),
    };

    let msg = Message::StateNormal(state);
    let _ = tx_to_hub.send(msg.clone()).await;
    let _ = tx_to_server.send(msg).await;

    if tx_to_heartbeat.send(Action::SendHeartbeatMessageNormal).await.is_err() {
        error!("Error: No se pudo enviar acción SendHeartbeatMessageNormal");
    }
}


async fn on_entry_safe_mode(tx_to_hub: &mpsc::Sender<Message>,
                            tx_to_server: &mpsc::Sender<Message>,
                            tx_to_timer: &mpsc::Sender<Event>,
                            tx_to_heartbeat: &mpsc::Sender<Action>,
                            current_epoch: &u32,
                            app_context: &AppContext) {

    init_timer(tx_to_timer, Duration::from_secs(180)).await;

    let metadata = build_metadata(app_context, "all");
    let state = MessageStateSafeMode {
        metadata: metadata.clone(),
        state: "safe_mode".to_string(),
        duration: 5,
        frequency: 1,
        jitter: 2,
    };

    let msg = Message::StateSafeMode(state);
    let _ = tx_to_hub.send(msg.clone()).await;
    let _ = tx_to_server.send(msg).await;

    send_phase_notification(tx_to_hub, metadata, *current_epoch).await;

    if tx_to_heartbeat.send(Action::SendHeartbeatMessageSafeMode).await.is_err() {
        error!("Error: No se pudo enviar acción SendHeartbeatMessageSafeMode");
    }
}


pub async fn run_fsm(tx_actions: mpsc::Sender<Vec<Action>>,
                     mut rx_event: mpsc::Receiver<Event>) {

    let mut state = FsmState::new();

    handle_transition(state.step(Event::Start), &mut state, &tx_actions).await;

    while let Some(event) = rx_event.recv().await {
        handle_transition(state.step(event), &mut state, &tx_actions).await;
    }
}


pub async fn heartbeat_to_send_watchdog_timer(tx_to_fsm: mpsc::Sender<Event>,
                                              mut cmd_rx: mpsc::Receiver<Event>) {

    loop {
        let duration = match cmd_rx.recv().await {
            Some(Event::InitTimer(d)) => d,
            Some(Event::StopTimer) => continue,
            None => break,
            _ => continue,
        };

        tokio::select! {
            _ = sleep(duration) => {
                let _ = tx_to_fsm.send(Event::Timeout).await;
            }
            Some(Event::StopTimer) = cmd_rx.recv() => {
                debug!("Debug: Watchdog timer de heartbeat para los hubs, cancelado");
            }
        }
    }
}


pub async fn heartbeat_generator(tx_to_hub: mpsc::Sender<Message>,
                                 tx_to_timer: mpsc::Sender<Event>,
                                 mut rx_from_fsm: mpsc::Receiver<Action>,
                                 mut cmd_rx: mpsc::Receiver<Event>,
                                 app_context: AppContext) {
    enum HeartbeatState {
        Phase,
        Normal,
        SafeMode,
        None,
    }

    let mut beat = HeartbeatState::None;

    loop {
        tokio::select! {
            Some(action) = rx_from_fsm.recv() => {
                let manager = app_context.quorum.read().await;
                match action {
                    Action::SendHeartbeatMessagePhase => {
                        beat = HeartbeatState::Phase;
                        let duration = manager.get_time_between_heartbeats_phase();
                        let _ = tx_to_timer.send(Event::InitTimer(Duration::from_secs(duration))).await;
                    },
                    Action::SendHeartbeatMessageNormal => {
                        beat = HeartbeatState::Normal;
                        let duration = manager.get_time_between_heartbeats_normal();
                        let _ = tx_to_timer.send(Event::InitTimer(Duration::from_secs(duration))).await;
                    },
                    Action::SendHeartbeatMessageSafeMode => {
                        beat = HeartbeatState::SafeMode;
                        let duration = manager.get_time_between_heartbeats_safe_mode();
                        let _ = tx_to_timer.send(Event::InitTimer(Duration::from_secs(duration))).await;
                    },
                    Action::StopSendHeartbeatMessagePhase | Action::StopSendHeartbeatMessageSafeMode => {
                        let _ = tx_to_timer.send(Event::StopTimer).await;
                    }
                    _ => {}
                }
            }

            Some(Event::Timeout) = cmd_rx.recv() => {
                send_heartbeat(&tx_to_hub, &app_context).await;
                let manager = app_context.quorum.read().await;
                let duration = match beat {
                    HeartbeatState::Phase => Duration::from_secs(manager.get_time_between_heartbeats_phase()),
                    HeartbeatState::Normal => Duration::from_secs(manager.get_time_between_heartbeats_normal()),
                    HeartbeatState::SafeMode => Duration::from_secs(manager.get_time_between_heartbeats_safe_mode()),
                    HeartbeatState::None => continue,
                };

                let _ = tx_to_timer.send(Event::InitTimer(duration)).await;
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


async fn send_message(tx: &mpsc::Sender<Message>, msg: Message) {
    let _ = tx.send(msg).await;
}


async fn send_balance_state(tx_to_hub: &mpsc::Sender<Message>,
                            tx_to_server: &mpsc::Sender<Message>,
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

    let msg = Message::StateBalanceMode(state);
    let _ = tx_to_hub.send(msg.clone()).await;
    let _ = tx_to_server.send(msg).await;
}


async fn send_handshake(tx: &mpsc::Sender<Message>,
                        metadata: Metadata,
                        epoch: u32) {

    let msg = Message::HandshakeToHub(HandshakeToHub {
        metadata,
        balance_epoch: epoch,
        duration: 5,
    });

    let _ = tx.send(msg).await;
}


async fn send_phase_notification(tx_to_hub: &mpsc::Sender<Message>,
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

    let _ = tx_to_hub.send(Message::PhaseNotification(phase)).await;
}


async fn send_heartbeat(tx_to_hub: &mpsc::Sender<Message>, app_context: &AppContext) {
    let metadata = build_metadata(app_context, "all");
    let heartbeat = Heartbeat {
        metadata,
        beat: true,
    };

    let _ = tx_to_hub.send(Message::Heartbeat(heartbeat)).await;
}


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