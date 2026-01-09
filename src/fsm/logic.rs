use tokio::sync::mpsc;
use tokio::sync::broadcast;
use super::domain::{EventSystem, State, SubStateQuorum, SubStateBalanceMode, SubStatePhase, Flag};



pub async fn run_fsm(tx: broadcast::Sender<EventSystem>, mut rx: mpsc::Receiver<EventSystem>) {
    let mut state = State::BalanceMode(SubStateBalanceMode::InitBalanceMode);
    let mut flag = Flag::Null;

    while let Some(event) = rx.recv().await {

        match (&state, &flag) {
            
            (State::BalanceMode(SubStateBalanceMode::InitBalanceMode), _) => {
                println!("entry: update_balance_epoch()");
                state = State::BalanceMode(SubStateBalanceMode::InHandshake);
            }

            (State::BalanceMode(SubStateBalanceMode::InHandshake), _) => {
                println!("entry: update_state_msg(), ...");
                state = State::BalanceMode(SubStateBalanceMode::Quorum(SubStateQuorum::CheckQuorum));
            }

            (State::BalanceMode(SubStateBalanceMode::Quorum(SubStateQuorum::CheckQuorum)), _) => {
                println!("entry: update_state_msg(), ...");
                state = State::BalanceMode(SubStateBalanceMode::Quorum(SubStateQuorum::RepeatHandshake));
                state = State::Normal;
                state = State::SafeMode;
                state = State::BalanceMode(SubStateBalanceMode::Phase(SubStatePhase::Alert));
            }

            (State::BalanceMode(SubStateBalanceMode::Quorum(SubStateQuorum::RepeatHandshake)), _) => {
                println!("entry: update_state_msg(), ...");
                state = State::BalanceMode(SubStateBalanceMode::Quorum(SubStateQuorum::CheckQuorum));
            }

            (State::BalanceMode(SubStateBalanceMode::Phase(SubStatePhase::Alert)), _) => {
                println!("entry: update_state_msg(), ...");
                state = State::BalanceMode(SubStateBalanceMode::Phase(SubStatePhase::Data));
            }

            (State::BalanceMode(SubStateBalanceMode::Phase(SubStatePhase::Data)), _) => {
                println!("entry: update_state_msg(), ...");
                state = State::BalanceMode(SubStateBalanceMode::Phase(SubStatePhase::Monitor));
            }

            (State::BalanceMode(SubStateBalanceMode::Phase(SubStatePhase::Monitor)), _) => {
                println!("entry: update_state_msg(), ...");
                state = State::BalanceMode(SubStateBalanceMode::OutHandshake);
            }

            (State::BalanceMode(SubStateBalanceMode::OutHandshake), _) => {
                println!("entry: update_state_msg(), ...");
                state = State::BalanceMode(SubStateBalanceMode::Quorum(SubStateQuorum::CheckQuorum));
            }

            (State::Normal, _) => {
                println!("entry: normal");
            }

            (State::SafeMode, _) => {
                println!("entry: safe");
                state = State::Normal;
            }
        }
    }
}







