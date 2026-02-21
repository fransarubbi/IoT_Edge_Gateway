//! # Secuencia de Inicialización del Sistema (Bootstrapper)
//!
//! Este módulo gestiona el ciclo de vida de arranque de la aplicación mediante
//! una Máquina de Estados Finitos (FSM). Asegura que el entorno del sistema operativo
//! (como el broker MQTT local y los certificados TLS) esté correctamente configurado
//! antes de instanciar el contexto principal de la aplicación.
//!
//! ## Flujo de Estados
//! 1. **CheckSystem:** Verifica las dependencias externas (Mosquitto, mTLS).
//! 2. **ConfigSystem:** Estado de recuperación. Si falta configuración, intenta repararla o aplicarla.
//! 3. **InitSystem:** Una vez que el entorno es seguro, inicializa la memoria y las estructuras base.


use crate::system::check::check_system_config;
use crate::system::configurate::{configurate_system, initializing_system};
use crate::system::domain::{ErrorType, Flag, StateInit};
use crate::context::domain::AppContext;


/// Ejecuta la máquina de estados de inicialización del dispositivo.
///
/// Esta función es el punto de entrada lógico antes de lanzar los servicios concurrentes.
/// Realiza un chequeo exhaustivo de las dependencias. Si detecta problemas recuperables
/// (ej. el servicio de Mosquitto está inactivo o faltan certificados mTLS), transiciona a un
/// estado de configuración para intentar solventarlos automáticamente.
///
/// # Comportamiento de Fallos
/// * Si `mosquitto` no está instalado en el sistema operativo, el proceso **abortará inmediatamente** con código 1.
/// * Si la configuración falla, o la inicialización del contexto falla, retornará el error propagado.
///
/// # Retorno
/// Retorna `Ok(AppContext)` conteniendo el estado global de la aplicación listo para ser
/// compartido entre los micro-servicios de Tokio, o un `ErrorType` si la inicialización
/// falla de manera irrecuperable.
///
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
                    Err(e) => return Err(e),
                }
            },
            (StateInit::InitSystem, _ ) => {
                return match initializing_system() {
                    Ok(context) => Ok(context),
                    Err(error) => Err(error),
                };
            },
        }
    }
}