use crate::fsm::domain::Flag;
use crate::system::check::check_system_config;
use crate::system::configurate::{configurate_system, initializing_system};
use crate::system::domain::{ErrorType, StateInit};
use crate::context::domain::AppContext;


pub async fn init_fsm() -> Result<AppContext, ErrorType> {
    let mut state = StateInit::CheckSystem;
    let mut flag = Flag::Null;

    loop {
        match (&state, &flag) {
            (StateInit::CheckSystem, _ ) => {
                match check_system_config() {
                    Ok(_) => state = StateInit::InitSystem,
                    Err(error) => {
                        match error {
                            ErrorType::MosquittoNotInstalled => std::process::exit(1),  // Salir con código 1 (indica error al sistema operativo)
                            ErrorType::MosquittoServiceInactive => {
                                (state, flag) = (StateInit::ConfigSystem, Flag::MosquittoServiceInactive);
                            },
                            ErrorType::MosquittoConf(_) => {
                                (state, flag) = (StateInit::ConfigSystem, Flag::MosquittoConf);
                            },
                            ErrorType::MtlsConfig(_) => {
                                (state, flag) = (StateInit::ConfigSystem, Flag::MtlsConf);
                            },
                            _ => {}
                        }
                    },
                }
            },
            (StateInit::ConfigSystem, _ ) => {
                match configurate_system(&flag) {
                    Ok(_) => state = StateInit::InitSystem,
                    Err(_) => {
                        state = StateInit::CheckSystem;
                    }
                }
            },
            (StateInit::InitSystem, _ ) => {
                return match initializing_system().await {
                    Ok(context) => Ok(context),
                    Err(error) => Err(error),
                };
            },
        }
    }
}