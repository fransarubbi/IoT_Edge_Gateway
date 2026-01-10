use tokio::sync::mpsc;
use tokio::sync::broadcast;
use super::domain::{EventSystem, State, SubStateQuorum, SubStateBalanceMode, SubStatePhase, Flag};



pub async fn run_fsm(tx: broadcast::Sender<EventSystem>,
                     mut rx: mpsc::Receiver<EventSystem>) {

    let mut state = State::BalanceMode(SubStateBalanceMode::InitBalanceMode);

    /*
    Necesito balance_epoch en base de datos

     */

    while let Some(event) = rx.recv().await {

        match &state {
            
            State::BalanceMode(SubStateBalanceMode::InitBalanceMode) => {
                println!("entry: update_balance_epoch()");
                state = State::BalanceMode(SubStateBalanceMode::InHandshake);
            }

            State::BalanceMode(SubStateBalanceMode::InHandshake) => {
                println!("entry: update_state_msg(), ...");
                state = State::BalanceMode(SubStateBalanceMode::Quorum(SubStateQuorum::CheckQuorum));
            }

            State::BalanceMode(SubStateBalanceMode::Quorum(SubStateQuorum::CheckQuorum)) => {
                println!("entry: update_state_msg(), ...");
                state = State::BalanceMode(SubStateBalanceMode::Quorum(SubStateQuorum::RepeatHandshake));
                state = State::Normal;
                state = State::SafeMode;
                state = State::BalanceMode(SubStateBalanceMode::Phase(SubStatePhase::Alert));
            }

            State::BalanceMode(SubStateBalanceMode::Quorum(SubStateQuorum::RepeatHandshake)) => {
                println!("entry: update_state_msg(), ...");
                state = State::BalanceMode(SubStateBalanceMode::Quorum(SubStateQuorum::CheckQuorum));
            }

            State::BalanceMode(SubStateBalanceMode::Phase(SubStatePhase::Alert)) => {
                println!("entry: update_state_msg(), ...");
                state = State::BalanceMode(SubStateBalanceMode::Phase(SubStatePhase::Data));
            }

            State::BalanceMode(SubStateBalanceMode::Phase(SubStatePhase::Data)) => {
                println!("entry: update_state_msg(), ...");
                state = State::BalanceMode(SubStateBalanceMode::Phase(SubStatePhase::Monitor));
            }

            State::BalanceMode(SubStateBalanceMode::Phase(SubStatePhase::Monitor)) => {
                println!("entry: update_state_msg(), ...");
                state = State::BalanceMode(SubStateBalanceMode::OutHandshake);
            }

            State::BalanceMode(SubStateBalanceMode::OutHandshake) => {
                println!("entry: update_state_msg(), ...");
                state = State::BalanceMode(SubStateBalanceMode::Quorum(SubStateQuorum::CheckQuorum));
            }

            State::Normal => {
                println!("entry: normal");
            }

            State::SafeMode => {
                println!("entry: safe");
                state = State::Normal;
            }
        }
    }
}


async fn in_handshake() {

}