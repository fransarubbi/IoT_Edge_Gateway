use chrono::{Utc};
use tokio::sync::mpsc;
use tokio::time::{Duration};
use tracing::{error, info};
use crate::config::fsm::HELLO_TIMEOUT;
use crate::context::domain::AppContext;
use crate::fsm::domain::{Action, Event, FsmState, SubStateBalanceMode, SubStateInit, SubStatePhase, SubStateQuorum, Transition};
use crate::message::domain::{HandshakeToHub, Message, MessageStateBalanceMode, Metadata};


pub async fn fsm_general_channels(tx_to_hub: mpsc::Sender<Message>,
                                  tx_to_server: mpsc::Sender<Message>,
                                  tx_to_fsm: mpsc::Sender<Event>,
                                  tx_to_timer: mpsc::Sender<Event>,
                                  mut rx_from_server: mpsc::Receiver<Message>,
                                  mut rx_from_hub: mpsc::Receiver<Message>,
                                  mut rx_from_fsm: mpsc::Receiver<Vec<Action>>,
                                  app_context: AppContext) {

    let mut current_epoch : u32 = 0;
    loop {
        tokio::select! {
            Some(msg_from_server) = rx_from_server.recv() => {
                match msg_from_server {
                    _ => {},
                }
            }

            Some(msg_from_hub) = rx_from_hub.recv() => {
                match msg_from_hub {
                    _ => {},
                }
            }

            Some(vec_action) = rx_from_fsm.recv() => {
                for action in vec_action {
                    on_entry(action, &app_context, &tx_to_fsm, &tx_to_timer, &tx_to_hub, &tx_to_server, &mut current_epoch).await;
                }
            }
        }
    }
}


async fn on_entry(action: Action,
                  app_context: &AppContext,
                  tx_to_fsm: &mpsc::Sender<Event>,
                  tx_to_timer: &mpsc::Sender<Event>,
                  tx_to_hub: &mpsc::Sender<Message>,
                  tx_to_server: &mpsc::Sender<Message>,
                  current_epoch: &mut u32) {

    match action {
        Action::OnEntryInit(SubStateInit::WaitConfirmation) => {
            init_timer(&tx_to_timer, HELLO_TIMEOUT).await;
        },
        Action::OnEntryBalance(SubStateBalanceMode::InitBalanceMode) => {
            on_entry_init_balance_mode(&app_context, &tx_to_fsm, current_epoch).await;
        },
        Action::OnEntryBalance(SubStateBalanceMode::InHandshake) => {
            on_entry_in_handshake(&tx_to_hub, &tx_to_server, &tx_to_timer, current_epoch, app_context.clone()).await;
        },
        Action::OnEntryBalance(SubStateBalanceMode::OutHandshake) => {
            on_entry_out_handshake(&tx_to_hub, &tx_to_server, &tx_to_timer, current_epoch, app_context.clone()).await;
        },
        Action::OnEntryQuorum(SubStateQuorum::CheckQuorumIn) => {
            // CALCULATE QUORUM PERCENTAGE
        },
        Action::OnEntryQuorum(SubStateQuorum::CheckQuorumOut) => {
            // CALCULATE QUORUM PERCENTAGE
        }
        Action::OnEntryQuorum(SubStateQuorum::RepeatHandshakeIn) => {
            /* SEND HANDSHAKE MESSAGE
                INIT TIMER IN HANDSHAKE
                INCREMENT ATTEMPTS
            */
        },
        Action::OnEntryQuorum(SubStateQuorum::RepeatHandshakeOut) => {
            /* SEND HANDSHAKE MESSAGE
                INIT TIMER IN HANDSHAKE
                INCREMENT ATTEMPTS
            */
        },
        Action::OnEntryPhase(SubStatePhase::Alert) => {
            /* INIT TIMER ALERT
            UPDATE STATE MESSAGE
            SEND PHASE MESSAGE
            SEND HEARTBEAT PHASE MESSAGE
            */
        },
        Action::OnEntryPhase(SubStatePhase::Data) => {
            /* INIT TIMER DATA
                UPDATE STATE MESSAGE
                SEND PHASE MESSAGE
            */
        },
        Action::OnEntryPhase(SubStatePhase::Monitor) => {
            /* INIT TIMER MONITOR
                UPDATE STATE MESSAGE
                SEND PHASE MESSAGE
            */
        },
        Action::OnEntryNormal => {
            // UPDATE STATE MESSAGE
            // SEND HEARTBEAT
        },
        Action::OnEntrySafeMode => {
            // UPDATE STATE MESSAGE
            // INIT TIMER SAFE MODE
            // SEND HEARTBEAT SAFE MODE MESSAGE
        },
        _ => {},
    }
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

    match update_balance_epoch(&app_context, current_epoch).await {
        Ok(_) => {
            info!("Info: Actualización epoch exitosa");
            if tx_to_fsm.send(Event::BalanceEpochOk).await.is_err() {
                error!("Error: No se pudo enviar evento BalanceEpochOk");
            }
        },
        Err(e) => {
            error!("Error: No se pudo actualizar epoch. {}", e);
            if tx_to_fsm.send(Event::BalanceEpochNotOk).await.is_err() {
                error!("Error: No se pudo enviar evento BalanceEpochNotOk");
            }
        },
    }
}


async fn on_entry_in_handshake(tx_to_hub: &mpsc::Sender<Message>,
                               tx_to_server: &mpsc::Sender<Message>,
                               tx_to_timer: &mpsc::Sender<Event>,
                               current_epoch: &u32,
                               app_context: AppContext) {

    let timestamp = Utc::now().timestamp();
    let metadata = Metadata{
        sender_user_id: app_context.system.id_edge.clone(),
        destination_id: "all".to_string(),
        timestamp,
    };
    let state = MessageStateBalanceMode{
        metadata,
        state: "Balance Mode".to_string(),
        balance_epoch: *current_epoch,
        sub_state: "in_handshake".to_string(),
        duration: 5
    };

    let msg = Message::StateBalanceMode(state);
    if tx_to_hub.send(msg.clone()).await.is_err() {
        error!("Error: No se pudo enviar evento balance Mode");
    }
    if tx_to_server.send(msg).await.is_err() {
        error!("Error: No se pudo enviar evento balance Mode");
    }

    let timestamp = Utc::now().timestamp();
    let metadata = Metadata{
        sender_user_id: app_context.system.id_edge.clone(),
        destination_id: "all".to_string(),
        timestamp,
    };
    let handshake = HandshakeToHub {
        metadata,
        balance_epoch: current_epoch.clone(),
        duration: 5
    };

    let msg = Message::HandshakeToHub(handshake);
    if tx_to_hub.send(msg.clone()).await.is_err() {
        error!("Error: No se pudo enviar evento balance Mode");
    }
    if tx_to_server.send(msg).await.is_err() {
        error!("Error: No se pudo enviar evento balance Mode");
    }

    init_timer(tx_to_timer, Duration::from_secs(180)).await;
}


async fn on_entry_out_handshake(tx_to_hub: &mpsc::Sender<Message>,
                               tx_to_server: &mpsc::Sender<Message>,
                               tx_to_timer: &mpsc::Sender<Event>,
                               current_epoch: &u32,
                               app_context: AppContext) {

    let timestamp = Utc::now().timestamp();
    let metadata = Metadata{
        sender_user_id: app_context.system.id_edge.clone(),
        destination_id: "all".to_string(),
        timestamp,
    };
    let state = MessageStateBalanceMode {
        metadata,
        state: "Balance Mode".to_string(),
        balance_epoch: *current_epoch,
        sub_state: "out_handshake".to_string(),
        duration: 5
    };

    let msg = Message::StateBalanceMode(state);
    if tx_to_hub.send(msg.clone()).await.is_err() {
        error!("Error: No se pudo enviar evento balance Mode");
    }
    if tx_to_server.send(msg).await.is_err() {
        error!("Error: No se pudo enviar evento balance Mode");
    }

    let timestamp = Utc::now().timestamp();
    let metadata = Metadata{
        sender_user_id: app_context.system.id_edge.clone(),
        destination_id: "all".to_string(),
        timestamp,
    };
    let handshake = HandshakeToHub {
        metadata,
        balance_epoch: current_epoch.clone(),
        duration: 5
    };

    let msg = Message::HandshakeToHub(handshake);
    if tx_to_hub.send(msg.clone()).await.is_err() {
        error!("Error: No se pudo enviar evento balance Mode");
    }
    if tx_to_server.send(msg).await.is_err() {
        error!("Error: No se pudo enviar evento balance Mode");
    }

    init_timer(tx_to_timer, Duration::from_secs(180)).await;
}






pub async fn run_fsm(tx_actions: mpsc::Sender<Vec<Action>>,
                     mut rx_event: mpsc::Receiver<Event>) {

    let mut state = FsmState::new();

    while let Some(event) = rx_event.recv().await {
        let transition = state.step(event);

        match transition {
            Transition::Valid(t) => {
                state = t.get_change_state();
                if tx_actions.send(t.get_actions()).await.is_err() {
                    error!("Error: No se pudo enviar el vector de acciones");
                }
            },
            Transition::Invalid(t) => {
                error!("FSM transición inválida: {}", t.get_invalid());
            }
        }
    }
}