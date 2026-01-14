use tokio::sync::mpsc;
use tracing::error;
use crate::fsm::domain::{ActionVector, Event, FsmState, Transition};



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