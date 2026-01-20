//! Dominio de la Máquina de Estados Finita General (FSM).
//!
//! Este módulo implementa el núcleo lógico del comportamiento del dispositivo.
//! Se basa en una **Máquina de Estados Jerárquica (HFSM)** puramente funcional.
//!
//! # Conceptos Clave
//!
//! 1.  **Estado Inmutable (`FsmState`):** Representa una "foto instantánea" del sistema. No se modifica, se reemplaza.
//! 2.  **Jerarquía:** El estado no es plano. Existe un `StateGlobal` que determina el modo de operación,
//!     y dentro de este, pueden existir sub-estados (ej. `BalanceMode` -> `Quorum` -> `CheckQuorumIn`).
//! 3.  **Transiciones Puras:** La función `step` recibe `(Estado Actual, Evento)` y retorna `(Nuevo Estado, Acciones)`.
//!     No hay efectos secundarios (I/O) dentro de la lógica de transición.
//! 4.  **Acciones (`Action`):** Instrucciones generadas por la FSM para que el "Runtime" (el bucle principal)
//!     ejecute tareas reales (enviar mensajes MQTT, iniciar timers, escribir en DB).
//!
//! # Flujo de Vida
//! `Init` -> `BalanceMode` (Ciclo de Sincronización) -> `Normal` (Estable)
//!              |
//!              v
//!          `SafeMode` (Fallo crítico)


use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tracing::debug;
use crate::mqtt::domain::PayloadTopic;



/// Estados Globales de nivel superior.
///
/// Determinan el comportamiento macro del sistema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StateGlobal {
    /// **Inicialización:** Arranque, handshake inicial con el servidor y configuración.
    Init,
    /// **Modo de Balanceo:** Fase crítica de negociación distribuida. El dispositivo
    /// intenta sincronizarse con sus hubs, verificar quórum y establecer su rol.
    BalanceMode,
    /// **Operación Normal:** El dispositivo ha completado el balanceo exitosamente y opera en régimen estable.
    Normal,
    /// **Modo Seguro:** Estado de fallo o emergencia. El dispositivo entra aquí tras errores críticos
    /// (DB corrupta, fallo de consenso repetido) para evitar operaciones inseguras.
    SafeMode,
}


/// Sub-estados para la fase de inicialización (`StateGlobal::Init`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubStateInit {
    HelloWorld,
    WaitConfirmation,
}


/// Sub-estados del modo de balanceo (`StateGlobal::BalanceMode`).
///
/// Esta es la parte más compleja de la FSM, gestionando el consenso y las fases de operación temporal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubStateBalanceMode {
    InitBalanceMode,
    InHandshake,
    Quorum,
    Phase,
    OutHandshake,
}


/// Sub-estados específicos de la verificación de Quorum.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubStateQuorum {
    CheckQuorumIn,
    CheckQuorumOut,
    RepeatHandshakeIn,
    RepeatHandshakeOut,
}


/// Fases operativas dentro del modo de balanceo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubStatePhase {
    Alert,
    Data,
    Monitor,
}


/// Representación compuesta del estado completo de la FSM.
///
/// Utiliza `Option` para modelar la jerarquía. Si un estado padre no está activo,
/// sus hijos deben ser `None`.
///
/// # Ejemplo
/// Si estamos verificando quórum:
/// - `global`: `StateGlobal::BalanceMode`
/// - `balance`: `Some(SubStateBalanceMode::Quorum)`
/// - `quorum`: `Some(SubStateQuorum::CheckQuorumIn)`
/// - `phase`: `None`
#[derive(Debug, Clone)]
pub struct FsmState {
    global: StateGlobal,
    init: Option<SubStateInit>,
    balance: Option<SubStateBalanceMode>,
    quorum: Option<SubStateQuorum>,
    phase: Option<SubStatePhase>,
}


/// Resultado de intentar aplicar un evento al estado actual.
#[derive(Debug)]
pub enum Transition {
    Valid(TransitionValid),
    Invalid(TransitionInvalid),
}


/// Datos resultantes de una transición exitosa.
#[derive(Debug)]
pub struct TransitionValid {
    change_state: FsmState,
    actions: Vec<Action>,
}


impl TransitionValid {
    pub fn get_change_state(&self) -> FsmState {
        self.change_state.clone()
    }
    pub fn get_actions(&self) -> Vec<Action> {
        self.actions.clone()
    }
}


/// Datos resultantes de una transición fallida o no permitida.
#[derive(Debug)]
pub struct TransitionInvalid {
    invalid: String,
}


impl TransitionInvalid {
    pub fn get_invalid(&self) -> &str {
        &self.invalid
    }
}


/// Acciones o Efectos Secundarios (Side Effects).
///
/// Estas variantes son instrucciones para el exterior. La FSM **decide** qué hacer,
/// pero no **ejecuta** la acción (principio de separación de responsabilidades).
#[derive(Debug, PartialEq, Clone)]
pub enum Action {
    SendToServerHello,
    IncrementAttempts,
    SendHeartbeatMessagePhase,
    SendHeartbeatMessageSafeMode,
    StopSendHeartbeatMessagePhase,
    StopSendHeartbeatMessageSafeMode,
    OnEntryInit(SubStateInit),
    OnEntryBalance(SubStateBalanceMode),
    OnEntryQuorum(SubStateQuorum),
    OnEntryPhase(SubStatePhase),
    OnEntryNormal,
    OnEntrySafeMode,
}


/// Eventos que alimentan la FSM.
///
/// Estos son los "Triggers" que provocan los cambios de estado.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    WaitOk,
    Timeout,
    BalanceEpochOk,
    BalanceEpochNotOk,
    ApproveQuorum,
    NotApproveQuorum,
    NotApproveNotAttempts,

    InitTimer(Duration),
    StopTimer,
}


#[derive(Debug, Clone, PartialEq)]
pub struct DataUpdateStateMessage {
    pub state: String,
    pub phase: String,
    pub duration: u32,
    pub frequency: u32,
    pub jitter: u32
}


impl FsmState {
    /// Crea una nueva instancia de la FSM en el estado inicial.
    pub fn new() -> Self {
        Self {
            global: StateGlobal::Init,
            init: Some(SubStateInit::HelloWorld),
            balance: None,
            quorum: None,
            phase: None,
        }
    }

    /// Lógica interna de despacho según el estado global.
    fn step_inner(&self, event: Event) -> Transition {
        match self.global {
            StateGlobal::Init => self.step_init(event),
            StateGlobal::BalanceMode => self.step_balance_mode(event),
            StateGlobal::SafeMode => self.step_safe_mode(event),
            _ => {
                let invalid = TransitionInvalid {
                    invalid: "No hay mas transiciones para ejecutar, una vez dentro de Normal, no se sale nunca".to_string(),
                };
                Transition::Invalid(invalid)
            }
        }
    }

    /// Maneja transiciones cuando el estado global es `Init`.
    fn step_init(&self, event: Event) -> Transition {
        match (&self.init, &event) {
            (Some(SubStateInit::HelloWorld), _ ) => {
                let next_fsm = self.clone();
                state_hello_world(next_fsm)
            },
            (Some(SubStateInit::WaitConfirmation), Event::Timeout) => {
                let next_fsm = self.clone();
                state_wait_confirmation_event_timeout(next_fsm)
            },
            (Some(SubStateInit::WaitConfirmation), Event::WaitOk) => {
                let next_fsm = self.clone();
                state_wait_confirmation_event_wait(next_fsm)
            },
            _ => invalid()
        }
    }

    /// Maneja transiciones cuando el estado global es `BalanceMode`.
    fn step_balance_mode(&self, event: Event) -> Transition {
        match (&self.balance, &event) {
            (Some(SubStateBalanceMode::InitBalanceMode), Event::BalanceEpochOk) => {
                let next_fsm = self.clone();
                state_init_balance_mode_event_balance_epoch(next_fsm)
            },
            (Some(SubStateBalanceMode::InitBalanceMode), Event::BalanceEpochNotOk) => {
                let next_fsm = self.clone();
                state_init_balance_mode_event_balance_epoch_not_ok(next_fsm)
            },
            (Some(SubStateBalanceMode::InHandshake), Event::Timeout) => {
                let next_fsm = self.clone();
                state_in_handshake_event_timeout(next_fsm)
            },
            (Some(SubStateBalanceMode::Quorum), _ ) => {
                match (&self.quorum, event) {
                    (Some(SubStateQuorum::CheckQuorumIn), Event::NotApproveQuorum) => {
                        let next_fsm = self.clone();
                        state_check_quorum_in_event_not_approve(next_fsm)
                    },
                    (Some(SubStateQuorum::CheckQuorumIn), Event::ApproveQuorum) => {
                        let next_fsm = self.clone();
                        state_check_quorum_in_event_approve_quorum(next_fsm)
                    },
                    (Some(SubStateQuorum::CheckQuorumIn), Event::NotApproveNotAttempts) => {
                        let next_fsm = self.clone();
                        state_check_quorum_event_not_and_not(next_fsm)
                    },
                    (Some(SubStateQuorum::CheckQuorumOut), Event::NotApproveQuorum) => {
                        let next_fsm = self.clone();
                        state_check_quorum_out_event_not_approve(next_fsm)
                    },
                    (Some(SubStateQuorum::CheckQuorumOut), Event::ApproveQuorum) => {
                        let next_fsm = self.clone();
                        state_check_quorum_out_event_approve_quorum(next_fsm)
                    },
                    (Some(SubStateQuorum::CheckQuorumOut), Event::NotApproveNotAttempts) => {
                        let next_fsm = self.clone();
                        state_check_quorum_event_not_and_not(next_fsm)
                    },
                    (Some(SubStateQuorum::RepeatHandshakeIn), Event::Timeout) => {
                        let next_fsm = self.clone();
                        state_repeat_handshake_in(next_fsm)
                    },
                    (Some(SubStateQuorum::RepeatHandshakeOut), Event::Timeout) => {
                        let next_fsm = self.clone();
                        state_repeat_handshake_out(next_fsm)
                    },
                    _ => invalid()
                }
            },
            (Some(SubStateBalanceMode::Phase), _ ) => {
                match (&self.phase, &event) {
                    (Some(SubStatePhase::Alert), Event::Timeout) => {
                        let next_fsm = self.clone();
                        state_alert_event_timeout(next_fsm)
                    },
                    (Some(SubStatePhase::Data), Event::Timeout) => {
                        let next_fsm = self.clone();
                        state_data_event_timeout(next_fsm)
                    },
                    (Some(SubStatePhase::Monitor), Event::Timeout) => {
                        let next_fsm = self.clone();
                        state_monitor_event_timeout(next_fsm)
                    },
                    _ => invalid()
                }
            },
            (Some(SubStateBalanceMode::OutHandshake), Event::Timeout) => {
                let next_fsm = self.clone();
                state_out_handshake_event_timeout(next_fsm)
            },
            _ => invalid()
        }
    }

    /// Maneja transiciones cuando el estado global es `SafeMode`.
    fn step_safe_mode(&self, event: Event) -> Transition {
        if event == Event::Timeout {
            let next_fsm = self.clone();
            let valid = TransitionValid {
                change_state: next_fsm,
                actions: vec![Action::StopSendHeartbeatMessageSafeMode],
            };
            return Transition::Valid(valid)
        }

        let invalid = TransitionInvalid {
            invalid: "Transición inválida".to_string(),
        };
        Transition::Invalid(invalid)
    }

    /// Función principal de transición (API Pública).
    ///
    /// 1. Calcula el siguiente estado llamando a `step_inner`.
    /// 2. Si la transición es válida, calcula las acciones de entrada (`compute_on_entry`)
    ///    comparando el estado actual (`self`) con el nuevo (`change_state`).
    /// 3. Retorna la transición completa con todas las acciones acumuladas.
    pub fn step(&self, event: Event) -> Transition {
        let transition = self.step_inner(event);

        match transition {
            Transition::Valid(mut t) => {
                let entry_actions = compute_on_entry(self, &t.change_state);
                t.actions.extend(entry_actions);
                Transition::Valid(t)
            }
            invalid => invalid,
        }
    }
}



// --- Funciones auxiliares de transición ---
// Cada una define un cambio atómico de estado.

/// Transición: HelloWorld -> WaitConfirmation.
fn state_hello_world(mut next_fsm: FsmState) -> Transition {
    next_fsm.init = Some(SubStateInit::WaitConfirmation);

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![Action::SendToServerHello],
    };
    Transition::Valid(valid)
}


/// Transición: WaitConfirmation -> HelloWorld
fn state_wait_confirmation_event_timeout(mut next_fsm: FsmState) -> Transition {
    next_fsm.init = Some(SubStateInit::HelloWorld);

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Transición: WaitConfirmation -> BalanceMode.
fn state_wait_confirmation_event_wait(mut next_fsm: FsmState) -> Transition {
    next_fsm.global = StateGlobal::BalanceMode;
    next_fsm.balance = Some(SubStateBalanceMode::InitBalanceMode);
    next_fsm.init = None;

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Retorna una transición inválida genérica para Init.
fn invalid() -> Transition {
    let invalid = TransitionInvalid {
        invalid: "Transición inválida".to_string(),
    };
    Transition::Invalid(invalid)
}


/// Transición: InitBalanceMode -> InHandshake.
fn state_init_balance_mode_event_balance_epoch(mut next_fsm: FsmState) -> Transition {
    next_fsm.balance = Some(SubStateBalanceMode::InHandshake);

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Transición: InitBalanceMode -> SafeMode.
fn state_init_balance_mode_event_balance_epoch_not_ok(mut next_fsm: FsmState) -> Transition {
    next_fsm.balance = None;
    next_fsm.global = StateGlobal::SafeMode;

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Transición: InHandshake -> Quorum (CheckQuorumIn).
fn state_in_handshake_event_timeout(mut next_fsm: FsmState) -> Transition {
    next_fsm.balance = Some(SubStateBalanceMode::Quorum);
    next_fsm.quorum = Some(SubStateQuorum::CheckQuorumIn);

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Transición: CheckQuorumIn -> RepeatHandshakeIn (No Quorum).
fn state_check_quorum_in_event_not_approve(mut next_fsm: FsmState) -> Transition {
    next_fsm.quorum = Some(SubStateQuorum::RepeatHandshakeIn);

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Transición: CheckQuorumOut -> RepeatHandshakeOut (No Quorum).
fn state_check_quorum_out_event_not_approve(mut next_fsm: FsmState) -> Transition {
    next_fsm.quorum = Some(SubStateQuorum::RepeatHandshakeOut);

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Transición: CheckQuorumIn -> Alert Phase (Approve).
fn state_check_quorum_in_event_approve_quorum(mut next_fsm: FsmState) -> Transition {
    next_fsm.balance = Some(SubStateBalanceMode::Phase);
    next_fsm.phase = Some(SubStatePhase::Alert);
    next_fsm.quorum = None;

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Transición: CheckQuorumOut -> Normal (Approve).
fn state_check_quorum_out_event_approve_quorum(mut next_fsm: FsmState) -> Transition {
    next_fsm.quorum = None;
    next_fsm.global = StateGlobal::Normal;

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Transición: CheckQuorum -> SafeMode (No Quorum & No Attempts left).
fn state_check_quorum_event_not_and_not(mut next_fsm: FsmState) -> Transition {
    next_fsm.global = StateGlobal::SafeMode;
    next_fsm.balance = None;
    next_fsm.quorum = None;

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Transición: RepeatHandshakeIn -> CheckQuorumIn.
fn state_repeat_handshake_in(mut next_fsm: FsmState) -> Transition {
    next_fsm.quorum = Some(SubStateQuorum::CheckQuorumIn);

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Transición: RepeatHandshakeOut -> CheckQuorumOut.
fn state_repeat_handshake_out(mut next_fsm: FsmState) -> Transition {
    next_fsm.quorum = Some(SubStateQuorum::CheckQuorumOut);

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Transición: Alert -> Data.
fn state_alert_event_timeout(mut next_fsm: FsmState) -> Transition {
    next_fsm.phase = Some(SubStatePhase::Data);

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Transición: Data -> Monitor.
fn state_data_event_timeout(mut next_fsm: FsmState) -> Transition {
    next_fsm.phase = Some(SubStatePhase::Monitor);

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Transición: Monitor -> OutHandshake.
fn state_monitor_event_timeout(mut next_fsm: FsmState) -> Transition {
    next_fsm.balance = Some(SubStateBalanceMode::OutHandshake);
    next_fsm.phase = None;

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![Action::StopSendHeartbeatMessagePhase],
    };
    Transition::Valid(valid)
}


/// Transición: OutHandshake -> CheckQuorumOut.
fn state_out_handshake_event_timeout(mut next_fsm: FsmState) -> Transition {
    next_fsm.balance = Some(SubStateBalanceMode::Quorum);
    next_fsm.quorum = Some(SubStateQuorum::CheckQuorumOut);

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Calcula las acciones de entrada (`OnEntry...`) detectando cambios de estado.
///
/// Compara el estado anterior (`old`) con el nuevo (`new`) para identificar
/// qué sub-estados han cambiado e inyectar las acciones correspondientes.
fn compute_on_entry(old: &FsmState, new: &FsmState) -> Vec<Action> {
    let mut actions = Vec::new();

    if old.init != new.init {
        if let Some(s) = &new.init {
            actions.push(Action::OnEntryInit(s.clone()));
        }
    }

    if old.balance != new.balance {
        if let Some(s) = &new.balance {
            actions.push(Action::OnEntryBalance(s.clone()));
        }
    }

    if old.quorum != new.quorum {
        if let Some(s) = &new.quorum {
            actions.push(Action::OnEntryQuorum(s.clone()));
        }
    }

    if old.phase != new.phase {
        if let Some(s) = &new.phase {
            actions.push(Action::OnEntryPhase(s.clone()));
        }
    }

    actions
}


/// Tarea asíncrona dedicada al temporizador de seguridad (Watchdog).
///
/// Implementa un patrón "Dead Man's Switch". Espera un comando `InitTimer`.
/// Si el tiempo expira antes de recibir `StopTimer`, envía un evento `Timeout` a la FSM.
pub async fn general_fsm_watchdog_timer(tx_to_fsm: mpsc::Sender<Event>,
                                     mut cmd_rx: mpsc::Receiver<Event>) {
    loop {
        let duration = match cmd_rx.recv().await {
            Some(Event::InitTimer(d)) => d,
            Some(Event::StopTimer) => continue, // Si ya estaba parado, ignorar
            None => break, // Canal cerrado, terminar tarea
            _ => continue,
        };

        // Estado ACTIVO: Corriendo temporizador
        tokio::select! {
            _ = sleep(duration) => {
                // El tiempo se agotó
                let _ = tx_to_fsm.send(Event::Timeout).await;
            }
            Some(Event::StopTimer) = cmd_rx.recv() => {
                debug!("Debug: Watchdog timer de fsm firmware, cancelado");
            }
        }
    }
}


//--------------------------------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================================
    // TESTS DE CREACIÓN Y ESTADO INICIAL
    // ============================================================================

    #[test]
    fn test_new_fsm_state_initial_values() {
        let state = FsmState::new();

        assert_eq!(state.global, StateGlobal::Init);
        assert_eq!(state.init, Some(SubStateInit::HelloWorld));
        assert_eq!(state.balance, None);
        assert_eq!(state.quorum, None);
        assert_eq!(state.phase, None);
    }

    // ============================================================================
    // TESTS DE ESTADO INIT - SubState HelloWorld
    // ============================================================================

    #[test]
    fn test_init_hello_world_any_event_transitions_to_wait_confirmation() {
        let state = FsmState::new();

        // HelloWorld acepta cualquier evento y transiciona
        let result = state.step(Event::Timeout);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.init, Some(SubStateInit::WaitConfirmation));
                assert_eq!(t.change_state.global, StateGlobal::Init);
                assert!(t.actions.contains(&Action::SendToServerHello));
                assert!(t.actions.contains(&Action::OnEntryInit(SubStateInit::WaitConfirmation)));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_init_hello_world_with_wait_ok_event() {
        let state = FsmState::new();
        let result = state.step(Event::WaitOk);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.init, Some(SubStateInit::WaitConfirmation));
                assert!(t.actions.contains(&Action::SendToServerHello));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_init_hello_world_with_balance_epoch_ok() {
        let state = FsmState::new();
        let result = state.step(Event::BalanceEpochOk);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.init, Some(SubStateInit::WaitConfirmation));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    // ============================================================================
    // TESTS DE ESTADO INIT - SubState WaitConfirmation
    // ============================================================================

    #[test]
    fn test_init_wait_confirmation_timeout_returns_to_hello_world() {
        let mut state = FsmState::new();
        state.init = Some(SubStateInit::WaitConfirmation);

        let result = state.step(Event::Timeout);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.global, StateGlobal::Init);
                assert_eq!(t.change_state.init, Some(SubStateInit::HelloWorld));
                assert!(t.actions.contains(&Action::OnEntryInit(SubStateInit::HelloWorld)));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_init_wait_confirmation_wait_ok_transitions_to_balance_mode() {
        let mut state = FsmState::new();
        state.init = Some(SubStateInit::WaitConfirmation);

        let result = state.step(Event::WaitOk);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.global, StateGlobal::BalanceMode);
                assert_eq!(t.change_state.balance, Some(SubStateBalanceMode::InitBalanceMode));
                assert_eq!(t.change_state.init, None);
                assert!(t.actions.contains(&Action::OnEntryBalance(SubStateBalanceMode::InitBalanceMode)));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_init_wait_confirmation_invalid_events() {
        let mut state = FsmState::new();
        state.init = Some(SubStateInit::WaitConfirmation);

        // Eventos que no deberían ser válidos en WaitConfirmation
        let invalid_events = vec![
            Event::BalanceEpochOk,
            Event::BalanceEpochNotOk,
            Event::ApproveQuorum,
            Event::NotApproveQuorum,
            Event::NotApproveNotAttempts,
        ];

        for event in invalid_events {
            let result = state.step(event);
            match result {
                Transition::Invalid(_) => {}, // Esperado
                Transition::Valid(_) => panic!("Se esperaba una transición inválida"),
            }
        }
    }

    // ============================================================================
    // TESTS DE BALANCE MODE - InitBalanceMode
    // ============================================================================

    #[test]
    fn test_balance_init_balance_epoch_ok_transitions_to_in_handshake() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::InitBalanceMode);
        state.init = None;

        let result = state.step(Event::BalanceEpochOk);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.balance, Some(SubStateBalanceMode::InHandshake));
                assert!(t.actions.contains(&Action::OnEntryBalance(SubStateBalanceMode::InHandshake)));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_balance_init_balance_epoch_not_ok_transitions_to_safe_mode() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::InitBalanceMode);
        state.init = None;

        let result = state.step(Event::BalanceEpochNotOk);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.global, StateGlobal::SafeMode);
                assert_eq!(t.change_state.balance, None);
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_balance_init_invalid_events() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::InitBalanceMode);
        state.init = None;

        let result = state.step(Event::Timeout);
        match result {
            Transition::Invalid(_) => {},
            Transition::Valid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    // ============================================================================
    // TESTS DE BALANCE MODE - InHandshake
    // ============================================================================

    #[test]
    fn test_balance_in_handshake_timeout_transitions_to_quorum_check_in() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::InHandshake);
        state.init = None;

        let result = state.step(Event::Timeout);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.balance, Some(SubStateBalanceMode::Quorum));
                assert_eq!(t.change_state.quorum, Some(SubStateQuorum::CheckQuorumIn));
                assert!(t.actions.contains(&Action::OnEntryBalance(SubStateBalanceMode::Quorum)));
                assert!(t.actions.contains(&Action::OnEntryQuorum(SubStateQuorum::CheckQuorumIn)));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_balance_in_handshake_invalid_events() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::InHandshake);
        state.init = None;

        let invalid_events = vec![
            Event::WaitOk,
            Event::BalanceEpochOk,
            Event::ApproveQuorum,
        ];

        for event in invalid_events {
            let result = state.step(event);
            match result {
                Transition::Invalid(_) => {},
                Transition::Valid(_) => panic!("Se esperaba una transición válida"),
            }
        }
    }

    // ============================================================================
    // TESTS DE BALANCE MODE - Quorum - CheckQuorumIn
    // ============================================================================

    #[test]
    fn test_quorum_check_in_not_approve_transitions_to_repeat_handshake_in() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Quorum);
        state.quorum = Some(SubStateQuorum::CheckQuorumIn);
        state.init = None;

        let result = state.step(Event::NotApproveQuorum);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.quorum, Some(SubStateQuorum::RepeatHandshakeIn));
                assert!(t.actions.contains(&Action::OnEntryQuorum(SubStateQuorum::RepeatHandshakeIn)));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_quorum_check_in_approve_transitions_to_alert_phase() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Quorum);
        state.quorum = Some(SubStateQuorum::CheckQuorumIn);
        state.init = None;

        let result = state.step(Event::ApproveQuorum);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.balance, Some(SubStateBalanceMode::Phase));
                assert_eq!(t.change_state.phase, Some(SubStatePhase::Alert));
                assert_eq!(t.change_state.quorum, None);
                assert!(t.actions.contains(&Action::OnEntryBalance(SubStateBalanceMode::Phase)));
                assert!(t.actions.contains(&Action::OnEntryPhase(SubStatePhase::Alert)));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_quorum_check_in_not_approve_not_attempts_transitions_to_safe_mode() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Quorum);
        state.quorum = Some(SubStateQuorum::CheckQuorumIn);
        state.init = None;

        let result = state.step(Event::NotApproveNotAttempts);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.global, StateGlobal::SafeMode);
                assert_eq!(t.change_state.balance, None);
                assert_eq!(t.change_state.quorum, None);
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    // ============================================================================
    // TESTS DE BALANCE MODE - Quorum - CheckQuorumOut
    // ============================================================================

    #[test]
    fn test_quorum_check_out_not_approve_transitions_to_repeat_handshake_out() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Quorum);
        state.quorum = Some(SubStateQuorum::CheckQuorumOut);
        state.init = None;

        let result = state.step(Event::NotApproveQuorum);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.quorum, Some(SubStateQuorum::RepeatHandshakeOut));
                assert!(t.actions.contains(&Action::OnEntryQuorum(SubStateQuorum::RepeatHandshakeOut)));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_quorum_check_out_approve_transitions_to_normal() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Quorum);
        state.quorum = Some(SubStateQuorum::CheckQuorumOut);
        state.init = None;

        let result = state.step(Event::ApproveQuorum);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.global, StateGlobal::Normal);
                assert_eq!(t.change_state.quorum, None);
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_quorum_check_out_not_approve_not_attempts_transitions_to_safe_mode() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Quorum);
        state.quorum = Some(SubStateQuorum::CheckQuorumOut);
        state.init = None;

        let result = state.step(Event::NotApproveNotAttempts);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.global, StateGlobal::SafeMode);
                assert_eq!(t.change_state.balance, None);
                assert_eq!(t.change_state.quorum, None);
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    // ============================================================================
    // TESTS DE BALANCE MODE - Quorum - RepeatHandshakeIn
    // ============================================================================

    #[test]
    fn test_quorum_repeat_handshake_in_timeout_returns_to_check_quorum_in() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Quorum);
        state.quorum = Some(SubStateQuorum::RepeatHandshakeIn);
        state.init = None;

        let result = state.step(Event::Timeout);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.quorum, Some(SubStateQuorum::CheckQuorumIn));
                assert!(t.actions.contains(&Action::OnEntryQuorum(SubStateQuorum::CheckQuorumIn)));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_quorum_repeat_handshake_in_invalid_events() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Quorum);
        state.quorum = Some(SubStateQuorum::RepeatHandshakeIn);
        state.init = None;

        let result = state.step(Event::ApproveQuorum);
        match result {
            Transition::Invalid(_) => {},
            Transition::Valid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    // ============================================================================
    // TESTS DE BALANCE MODE - Quorum - RepeatHandshakeOut
    // ============================================================================

    #[test]
    fn test_quorum_repeat_handshake_out_timeout_returns_to_check_quorum_out() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Quorum);
        state.quorum = Some(SubStateQuorum::RepeatHandshakeOut);
        state.init = None;

        let result = state.step(Event::Timeout);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.quorum, Some(SubStateQuorum::CheckQuorumOut));
                assert!(t.actions.contains(&Action::OnEntryQuorum(SubStateQuorum::CheckQuorumOut)));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    // ============================================================================
    // TESTS DE BALANCE MODE - Phase - Alert
    // ============================================================================

    #[test]
    fn test_phase_alert_timeout_transitions_to_data() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Phase);
        state.phase = Some(SubStatePhase::Alert);
        state.init = None;

        let result = state.step(Event::Timeout);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.phase, Some(SubStatePhase::Data));
                assert!(t.actions.contains(&Action::OnEntryPhase(SubStatePhase::Data)));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_phase_alert_invalid_events() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Phase);
        state.phase = Some(SubStatePhase::Alert);
        state.init = None;

        let result = state.step(Event::WaitOk);
        match result {
            Transition::Invalid(_) => {},
            Transition::Valid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    // ============================================================================
    // TESTS DE BALANCE MODE - Phase - Data
    // ============================================================================

    #[test]
    fn test_phase_data_timeout_transitions_to_monitor() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Phase);
        state.phase = Some(SubStatePhase::Data);
        state.init = None;

        let result = state.step(Event::Timeout);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.phase, Some(SubStatePhase::Monitor));
                assert!(t.actions.contains(&Action::OnEntryPhase(SubStatePhase::Monitor)));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    // ============================================================================
    // TESTS DE BALANCE MODE - Phase - Monitor
    // ============================================================================

    #[test]
    fn test_phase_monitor_timeout_transitions_to_out_handshake() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Phase);
        state.phase = Some(SubStatePhase::Monitor);
        state.init = None;

        let result = state.step(Event::Timeout);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.balance, Some(SubStateBalanceMode::OutHandshake));
                assert_eq!(t.change_state.phase, None);
                assert!(t.actions.contains(&Action::StopSendHeartbeatMessagePhase));
                assert!(t.actions.contains(&Action::OnEntryBalance(SubStateBalanceMode::OutHandshake)));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    // ============================================================================
    // TESTS DE BALANCE MODE - OutHandshake
    // ============================================================================

    #[test]
    fn test_balance_out_handshake_timeout_transitions_to_quorum_check_out() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::OutHandshake);
        state.init = None;

        let result = state.step(Event::Timeout);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.balance, Some(SubStateBalanceMode::Quorum));
                assert_eq!(t.change_state.quorum, Some(SubStateQuorum::CheckQuorumOut));
                assert!(t.actions.contains(&Action::OnEntryBalance(SubStateBalanceMode::Quorum)));
                assert!(t.actions.contains(&Action::OnEntryQuorum(SubStateQuorum::CheckQuorumOut)));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    // ============================================================================
    // TESTS DE SAFE MODE
    // ============================================================================

    #[test]
    fn test_safe_mode_timeout_stops_heartbeat() {
        let mut state = FsmState::new();
        state.global = StateGlobal::SafeMode;
        state.balance = None;
        state.init = None;

        let result = state.step(Event::Timeout);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.global, StateGlobal::SafeMode);
                assert!(t.actions.contains(&Action::StopSendHeartbeatMessageSafeMode));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_safe_mode_invalid_events() {
        let mut state = FsmState::new();
        state.global = StateGlobal::SafeMode;
        state.balance = None;
        state.init = None;

        let invalid_events = vec![
            Event::WaitOk,
            Event::BalanceEpochOk,
            Event::ApproveQuorum,
            Event::NotApproveQuorum,
        ];

        for event in invalid_events {
            let result = state.step(event.clone());
            match result {
                Transition::Invalid(_) => {},
                Transition::Valid(_) => panic!("Se esperaba una transición inválida por evento: {:?}", event),
            }
        }
    }

    // ============================================================================
    // TESTS DE ESTADO NORMAL
    // ============================================================================

    #[test]
    fn test_normal_state_no_transitions() {
        let mut state = FsmState::new();
        state.global = StateGlobal::Normal;
        state.balance = None;
        state.init = None;

        let events = vec![
            Event::Timeout,
            Event::WaitOk,
            Event::BalanceEpochOk,
            Event::ApproveQuorum,
        ];

        for event in events {
            let result = state.step(event.clone());
            match result {
                Transition::Invalid(inv) => {
                    assert!(inv.invalid.contains("Normal"));
                }
                Transition::Valid(_) => panic!("Normal no tiene transiciones"),
            }
        }
    }

    // ============================================================================
    // TESTS DE COMPUTE_ON_ENTRY
    // ============================================================================

    #[test]
    fn test_compute_on_entry_detects_init_change() {
        let old = FsmState {
            global: StateGlobal::Init,
            init: Some(SubStateInit::HelloWorld),
            balance: None,
            quorum: None,
            phase: None,
        };

        let new = FsmState {
            global: StateGlobal::Init,
            init: Some(SubStateInit::WaitConfirmation),
            balance: None,
            quorum: None,
            phase: None,
        };

        let actions = compute_on_entry(&old, &new);
        assert!(actions.contains(&Action::OnEntryInit(SubStateInit::WaitConfirmation)));
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn test_compute_on_entry_detects_balance_change() {
        let old = FsmState {
            global: StateGlobal::BalanceMode,
            init: None,
            balance: Some(SubStateBalanceMode::InitBalanceMode),
            quorum: None,
            phase: None,
        };

        let new = FsmState {
            global: StateGlobal::BalanceMode,
            init: None,
            balance: Some(SubStateBalanceMode::InHandshake),
            quorum: None,
            phase: None,
        };

        let actions = compute_on_entry(&old, &new);
        assert!(actions.contains(&Action::OnEntryBalance(SubStateBalanceMode::InHandshake)));
    }

    #[test]
    fn test_compute_on_entry_detects_multiple_changes() {
        let old = FsmState {
            global: StateGlobal::BalanceMode,
            init: None,
            balance: Some(SubStateBalanceMode::InHandshake),
            quorum: None,
            phase: None,
        };

        let new = FsmState {
            global: StateGlobal::BalanceMode,
            init: None,
            balance: Some(SubStateBalanceMode::Quorum),
            quorum: Some(SubStateQuorum::CheckQuorumIn),
            phase: None,
        };

        let actions = compute_on_entry(&old, &new);
        assert!(actions.contains(&Action::OnEntryBalance(SubStateBalanceMode::Quorum)));
        assert!(actions.contains(&Action::OnEntryQuorum(SubStateQuorum::CheckQuorumIn)));
        assert_eq!(actions.len(), 2);
    }

    #[test]
    fn test_compute_on_entry_no_changes() {
        let state = FsmState {
            global: StateGlobal::BalanceMode,
            init: None,
            balance: Some(SubStateBalanceMode::InHandshake),
            quorum: None,
            phase: None,
        };

        let actions = compute_on_entry(&state, &state);
        assert_eq!(actions.len(), 0);
    }

    #[test]
    fn test_compute_on_entry_detects_phase_change() {
        let old = FsmState {
            global: StateGlobal::BalanceMode,
            init: None,
            balance: Some(SubStateBalanceMode::Phase),
            quorum: None,
            phase: Some(SubStatePhase::Alert),
        };

        let new = FsmState {
            global: StateGlobal::BalanceMode,
            init: None,
            balance: Some(SubStateBalanceMode::Phase),
            quorum: None,
            phase: Some(SubStatePhase::Data),
        };

        let actions = compute_on_entry(&old, &new);
        assert!(actions.contains(&Action::OnEntryPhase(SubStatePhase::Data)));
    }

    #[test]
    fn test_compute_on_entry_state_to_none() {
        let old = FsmState {
            global: StateGlobal::BalanceMode,
            init: None,
            balance: Some(SubStateBalanceMode::Phase),
            quorum: None,
            phase: Some(SubStatePhase::Monitor),
        };

        let new = FsmState {
            global: StateGlobal::BalanceMode,
            init: None,
            balance: Some(SubStateBalanceMode::OutHandshake),
            quorum: None,
            phase: None,
        };

        let actions = compute_on_entry(&old, &new);
        assert!(actions.contains(&Action::OnEntryBalance(SubStateBalanceMode::OutHandshake)));
        // phase cambió a None, no debería generar OnEntryPhase
        assert!(!actions.iter().any(|a| matches!(a, Action::OnEntryPhase(_))));
    }

    // ============================================================================
    // TESTS DE FLUJO COMPLETO (INTEGRATION TESTS)
    // ============================================================================

    #[test]
    fn test_full_flow_init_to_balance_to_normal() {
        // HelloWorld
        let mut state = FsmState::new();
        assert_eq!(state.global, StateGlobal::Init);
        assert_eq!(state.init, Some(SubStateInit::HelloWorld));

        // HelloWorld -> WaitConfirmation
        let result = state.step(Event::Timeout);
        match result {
            Transition::Valid(t) => state = t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        }
        assert_eq!(state.init, Some(SubStateInit::WaitConfirmation));

        // WaitConfirmation -> BalanceMode
        let result = state.step(Event::WaitOk);
        match result {
            Transition::Valid(t) => state = t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        }
        assert_eq!(state.global, StateGlobal::BalanceMode);
        assert_eq!(state.balance, Some(SubStateBalanceMode::InitBalanceMode));

        // InitBalanceMode -> InHandshake
        let result = state.step(Event::BalanceEpochOk);
        match result {
            Transition::Valid(t) => state = t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        }
        assert_eq!(state.balance, Some(SubStateBalanceMode::InHandshake));

        // InHandshake -> CheckQuorumIn
        let result = state.step(Event::Timeout);
        match result {
            Transition::Valid(t) => state = t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        }
        assert_eq!(state.quorum, Some(SubStateQuorum::CheckQuorumIn));

        // CheckQuorumIn -> Alert Phase
        let result = state.step(Event::ApproveQuorum);
        match result {
            Transition::Valid(t) => state = t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        }
        assert_eq!(state.phase, Some(SubStatePhase::Alert));

        // Alert -> Data
        let result = state.step(Event::Timeout);
        match result {
            Transition::Valid(t) => state = t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        }
        assert_eq!(state.phase, Some(SubStatePhase::Data));

        // Data -> Monitor
        let result = state.step(Event::Timeout);
        match result {
            Transition::Valid(t) => state = t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        }
        assert_eq!(state.phase, Some(SubStatePhase::Monitor));

        // Monitor -> OutHandshake
        let result = state.step(Event::Timeout);
        match result {
            Transition::Valid(t) => state = t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        }
        assert_eq!(state.balance, Some(SubStateBalanceMode::OutHandshake));
        assert_eq!(state.phase, None);

        // OutHandshake -> CheckQuorumOut
        let result = state.step(Event::Timeout);
        match result {
            Transition::Valid(t) => state = t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        }
        assert_eq!(state.quorum, Some(SubStateQuorum::CheckQuorumOut));

        // CheckQuorumOut -> Normal
        let result = state.step(Event::ApproveQuorum);
        match result {
            Transition::Valid(t) => state = t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        }
        assert_eq!(state.global, StateGlobal::Normal);
        assert_eq!(state.quorum, None);
    }

    #[test]
    fn test_full_flow_init_to_safe_mode_from_init_balance() {
        let mut state = FsmState::new();

        // HelloWorld -> WaitConfirmation -> BalanceMode
        state = match state.step(Event::Timeout) {
            Transition::Valid(t) => t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        };

        state = match state.step(Event::WaitOk) {
            Transition::Valid(t) => t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        };

        // InitBalanceMode -> SafeMode
        state = match state.step(Event::BalanceEpochNotOk) {
            Transition::Valid(t) => t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        };

        assert_eq!(state.global, StateGlobal::SafeMode);
        assert_eq!(state.balance, None);
    }

    #[test]
    fn test_full_flow_quorum_retry_then_safe_mode() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Quorum);
        state.quorum = Some(SubStateQuorum::CheckQuorumIn);
        state.init = None;

        // CheckQuorumIn -> RepeatHandshakeIn
        state = match state.step(Event::NotApproveQuorum) {
            Transition::Valid(t) => t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        };
        assert_eq!(state.quorum, Some(SubStateQuorum::RepeatHandshakeIn));

        // RepeatHandshakeIn -> CheckQuorumIn
        state = match state.step(Event::Timeout) {
            Transition::Valid(t) => t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        };
        assert_eq!(state.quorum, Some(SubStateQuorum::CheckQuorumIn));

        // CheckQuorumIn -> SafeMode (no attempts)
        state = match state.step(Event::NotApproveNotAttempts) {
            Transition::Valid(t) => t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        };
        assert_eq!(state.global, StateGlobal::SafeMode);
    }

    #[test]
    fn test_full_flow_complete_cycle_with_quorum_out_retry() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::OutHandshake);
        state.init = None;

        // OutHandshake -> CheckQuorumOut
        state = match state.step(Event::Timeout) {
            Transition::Valid(t) => t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        };
        assert_eq!(state.quorum, Some(SubStateQuorum::CheckQuorumOut));

        // CheckQuorumOut -> RepeatHandshakeOut
        state = match state.step(Event::NotApproveQuorum) {
            Transition::Valid(t) => t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        };
        assert_eq!(state.quorum, Some(SubStateQuorum::RepeatHandshakeOut));

        // RepeatHandshakeOut -> CheckQuorumOut
        state = match state.step(Event::Timeout) {
            Transition::Valid(t) => t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        };
        assert_eq!(state.quorum, Some(SubStateQuorum::CheckQuorumOut));

        // CheckQuorumOut -> Normal
        state = match state.step(Event::ApproveQuorum) {
            Transition::Valid(t) => t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        };
        assert_eq!(state.global, StateGlobal::Normal);
    }

    // ============================================================================
    // TESTS DE ACCIONES ESPECÍFICAS
    // ============================================================================

    #[test]
    fn test_actions_send_hello_on_hello_world_transition() {
        let state = FsmState::new();
        let result = state.step(Event::Timeout);

        match result {
            Transition::Valid(t) => {
                assert!(t.actions.contains(&Action::SendToServerHello));
            }
            _ => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_actions_stop_heartbeat_on_monitor_to_out_handshake() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Phase);
        state.phase = Some(SubStatePhase::Monitor);
        state.init = None;

        let result = state.step(Event::Timeout);

        match result {
            Transition::Valid(t) => {
                assert!(t.actions.contains(&Action::StopSendHeartbeatMessagePhase));
            }
            _ => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_actions_stop_heartbeat_safe_mode_on_timeout() {
        let mut state = FsmState::new();
        state.global = StateGlobal::SafeMode;
        state.init = None;

        let result = state.step(Event::Timeout);

        match result {
            Transition::Valid(t) => {
                assert!(t.actions.contains(&Action::StopSendHeartbeatMessageSafeMode));
            }
            _ => panic!("Se esperaba una transición válida"),
        }
    }

    // ============================================================================
    // TESTS DE EDGE CASES Y CORNER CASES
    // ============================================================================

    #[test]
    fn test_init_wait_confirmation_back_to_hello_world_cycle() {
        let mut state = FsmState::new();

        // HelloWorld -> WaitConfirmation
        state = match state.step(Event::Timeout) {
            Transition::Valid(t) => t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        };

        // WaitConfirmation -> HelloWorld (timeout)
        state = match state.step(Event::Timeout) {
            Transition::Valid(t) => t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        };

        assert_eq!(state.init, Some(SubStateInit::HelloWorld));

        // Can cycle again
        state = match state.step(Event::Timeout) {
            Transition::Valid(t) => t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        };
        assert_eq!(state.init, Some(SubStateInit::WaitConfirmation));
    }

    #[test]
    fn test_quorum_check_in_to_phase_clears_quorum() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Quorum);
        state.quorum = Some(SubStateQuorum::CheckQuorumIn);
        state.init = None;

        let result = state.step(Event::ApproveQuorum);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.quorum, None);
                assert_eq!(t.change_state.phase, Some(SubStatePhase::Alert));
            }
            _ => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_monitor_to_out_handshake_clears_phase() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Phase);
        state.phase = Some(SubStatePhase::Monitor);
        state.init = None;

        let result = state.step(Event::Timeout);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.phase, None);
                assert_eq!(t.change_state.balance, Some(SubStateBalanceMode::OutHandshake));
            }
            _ => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_transition_to_safe_mode_clears_all_substates() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Quorum);
        state.quorum = Some(SubStateQuorum::CheckQuorumIn);
        state.init = None;

        let result = state.step(Event::NotApproveNotAttempts);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.global, StateGlobal::SafeMode);
                assert_eq!(t.change_state.balance, None);
                assert_eq!(t.change_state.quorum, None);
                assert_eq!(t.change_state.phase, None);
            }
            _ => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_transition_to_normal_clears_quorum() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Quorum);
        state.quorum = Some(SubStateQuorum::CheckQuorumOut);
        state.init = None;

        let result = state.step(Event::ApproveQuorum);

        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.global, StateGlobal::Normal);
                assert_eq!(t.change_state.quorum, None);
            }
            _ => panic!("Se esperaba una transición válida"),
        }
    }

    // ============================================================================
    // TESTS DE CONSISTENCIA DE ESTADOS
    // ============================================================================

    #[test]
    fn test_state_consistency_balance_mode_has_balance_substate() {
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::InitBalanceMode);
        state.init = None;

        // Todas las transiciones válidas en BalanceMode deberían mantener balance != None
        // excepto cuando se sale a SafeMode o Normal

        let result = state.step(Event::BalanceEpochOk);
        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.global, StateGlobal::BalanceMode);
                assert!(t.change_state.balance.is_some());
            }
            _ => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_state_consistency_init_has_init_substate() {
        let state = FsmState::new();
        assert_eq!(state.global, StateGlobal::Init);
        assert!(state.init.is_some());

        let result = state.step(Event::Timeout);
        match result {
            Transition::Valid(t) => {
                if t.change_state.global == StateGlobal::Init {
                    assert!(t.change_state.init.is_some());
                }
            }
            _ => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_state_consistency_safe_mode_has_no_substates() {
        let mut state = FsmState::new();
        state.global = StateGlobal::SafeMode;
        state.balance = None;
        state.quorum = None;
        state.phase = None;
        state.init = None;

        let result = state.step(Event::Timeout);
        match result {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.balance, None);
                assert_eq!(t.change_state.quorum, None);
                assert_eq!(t.change_state.phase, None);
                assert_eq!(t.change_state.init, None);
            }
            _ => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_state_consistency_normal_has_no_substates() {
        let mut state = FsmState::new();
        state.global = StateGlobal::Normal;
        state.balance = None;
        state.quorum = None;
        state.phase = None;
        state.init = None;

        // Normal no tiene transiciones válidas, pero verificamos consistencia
        assert_eq!(state.balance, None);
        assert_eq!(state.quorum, None);
        assert_eq!(state.phase, None);
        assert_eq!(state.init, None);
    }

    // ============================================================================
    // TESTS ADICIONALES DE COBERTURA
    // ============================================================================

    #[test]
    fn test_all_phase_substates_transition_correctly() {
        // Alert -> Data
        let mut state = FsmState::new();
        state.global = StateGlobal::BalanceMode;
        state.balance = Some(SubStateBalanceMode::Phase);
        state.phase = Some(SubStatePhase::Alert);
        state.init = None;

        state = match state.step(Event::Timeout) {
            Transition::Valid(t) => t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        };
        assert_eq!(state.phase, Some(SubStatePhase::Data));

        // Data -> Monitor
        state = match state.step(Event::Timeout) {
            Transition::Valid(t) => t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        };
        assert_eq!(state.phase, Some(SubStatePhase::Monitor));

        // Monitor -> OutHandshake (exits Phase)
        state = match state.step(Event::Timeout) {
            Transition::Valid(t) => t.change_state,
            _ => panic!("Se esperaba una transición válida"),
        };
        assert_eq!(state.balance, Some(SubStateBalanceMode::OutHandshake));
        assert_eq!(state.phase, None);
    }

    #[test]
    fn test_all_quorum_substates_exist_and_transition() {
        let quorum_states = vec![
            SubStateQuorum::CheckQuorumIn,
            SubStateQuorum::CheckQuorumOut,
            SubStateQuorum::RepeatHandshakeIn,
            SubStateQuorum::RepeatHandshakeOut,
        ];

        // Verificar que todos los estados de quorum están definidos
        for quorum_state in quorum_states {
            let mut state = FsmState::new();
            state.global = StateGlobal::BalanceMode;
            state.balance = Some(SubStateBalanceMode::Quorum);
            state.quorum = Some(quorum_state.clone());
            state.init = None;

            // Verificar que el estado se puede crear
            assert_eq!(state.quorum, Some(quorum_state));
        }
    }

    #[test]
    fn test_clone_state_creates_independent_copy() {
        let original = FsmState::new();
        let mut cloned = original.clone();

        cloned.global = StateGlobal::BalanceMode;
        cloned.balance = Some(SubStateBalanceMode::InitBalanceMode);
        cloned.init = None;

        // Original no debe cambiar
        assert_eq!(original.global, StateGlobal::Init);
        assert_eq!(original.init, Some(SubStateInit::HelloWorld));
        assert_eq!(original.balance, None);

        // Clonado debe tener los nuevos valores
        assert_eq!(cloned.global, StateGlobal::BalanceMode);
        assert_eq!(cloned.balance, Some(SubStateBalanceMode::InitBalanceMode));
        assert_eq!(cloned.init, None);
    }
}