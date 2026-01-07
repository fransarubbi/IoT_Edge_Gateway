use tokio::sync::mpsc;
use tokio::sync::broadcast;
use crate::system::check_config::{check_system_config, ErrorType};
use crate::system::configurate_system::configurate_system;
use super::domain::{EventSystem, State, SubStateInit, SubStateQuorum, SubStateBalanceMode, SubStatePhase, Flag};





pub async fn run_fsm(tx: broadcast::Sender<EventSystem>, mut rx: mpsc::Receiver<EventSystem>) {
    let mut state = State::Init(SubStateInit::CheckConfig);
    let mut flag = Flag::Null;

    while let Some(event) = rx.recv().await {

        match (&state, &flag) {

            (State::Init(SubStateInit::CheckConfig), _) => {
                match check_system_config() {
                    Ok(_) => state = State::Init(SubStateInit::InitSystem),
                    Err(error) => {
                        match error {
                            ErrorType::MosquittoNotInstalled => std::process::exit(1),  // Salir con codigo 1 (indica error al sistema operativo)
                            ErrorType::MosquittoServiceInactive => {
                                (state, flag) = (State::Init(SubStateInit::ConfigurateSystem), Flag::MosquittoServiceInactive);
                            },
                            ErrorType::MosquittoConf(_) => {
                                (state, flag) = (State::Init(SubStateInit::ConfigurateSystem), Flag::MosquittoConf);
                            },
                            ErrorType::MtlsConfig(_) => {
                                (state, flag) = (State::Init(SubStateInit::ConfigurateSystem), Flag::MtlsConf);
                            },
                            _ => {}
                        }
                    },
                }
            }

            (State::Init(SubStateInit::InitSystem), _) => {
                tx.send(EventSystem::EventSystemOk).unwrap();
                state = State::BalanceMode(SubStateBalanceMode::InitBalanceMode);
            },

            (State::Init(SubStateInit::ConfigurateSystem), _) => {
                match configurate_system(&flag) {
                    Ok(_) => state = State::Init(SubStateInit::InitSystem),
                    Err(error) => {
                        state = State::Init(SubStateInit::CheckConfig);
                    }
                }
            },

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







