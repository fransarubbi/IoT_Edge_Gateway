use tokio::sync::mpsc;
use tracing::error;
use crate::fsm::domain::{ActionVector, Event, FsmState, Transition};
use crate::message::domain::{MessageFromHub, MessageFromHubTypes, MessageFromServer};


pub async fn converter_message_to_event(mut rx_from_server: mpsc::Receiver<MessageFromServer>) {
    loop {
        tokio::select! {
            Some(msg_from_server) = rx_from_server.recv() => {
                match msg_from_server {
                    _ => {}
                }
            }
        }
    }
}


pub async fn quorum(mut rx_from_hub: mpsc::Receiver<MessageFromHub>) {

    while let Some(msg_from_hub) = rx_from_hub.recv().await {
        match msg_from_hub.msg {
            MessageFromHubTypes::Handshake(handshake) => {

            },
            _ => {}
        }
    }
}


pub async fn run_fsm(tx_actions: mpsc::Sender<ActionVector>,
                     mut rx_event: mpsc::Receiver<Event>) {

    let mut state = FsmState::new();

    while let Some(event) = rx_event.recv().await {
        let transition = state.step(event);

        match transition {
            Transition::Valid(t) => {
                state = t.change_state;
                if tx_actions.send(ActionVector::Vector(t.actions)).await.is_err() {
                    error!("Error: No se pudo enviar el vector de acciones");
                }
            },
            Transition::Invalid(t) => {
                error!("FSM transición inválida: {}", t.invalid);
            }
        }
    }
}