//! Dominio de la Máquina de Estados Finitos (FSM).
//!
//! Este módulo define la estructura, estados, eventos y lógica de transición del sistema.
//! Implementa una **Máquina de Estados Jerárquica** donde el estado global puede contener
//! sub-estados activos (por ejemplo, estar en `BalanceMode` y a su vez en `Quorum`).
//!
//! # Arquitectura
//!
//! La lógica es puramente funcional:
//! 1. **Estado Inmutable:** `FsmState` representa una instantánea del sistema.
//! 2. **Transiciones Puras:** La función `step` toma un estado y un evento, y devuelve una nueva instancia del estado (`Transition`).
//! 3. **Acciones:** Las transiciones generan una lista de `Action` (efectos secundarios) que deben ser ejecutados por el runtime.


use serde::{Deserialize, Serialize};
use crate::mqtt::domain::PayloadTopic;


/// Eventos internos de conectividad y red.
///
/// Se utilizan para notificar cambios en la conexión MQTT local o remota.
#[derive(PartialEq, Clone)]
pub enum InternalEvent {
    ServerConnected,
    ServerDisconnected,
    LocalConnected,
    LocalDisconnected,
    IncomingMessage(PayloadTopic),
}


/// Estados Globales de nivel superior.
///
/// Determinan el comportamiento macro del sistema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StateGlobal {
    Init,
    BalanceMode,
    Normal,
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
/// Utiliza `Option` para los sub-estados, permitiendo representar la jerarquía.
/// Por ejemplo, si `global` es `Normal`, los sub-estados serán `None`.
#[derive(Debug, Clone)]
pub struct FsmState {
    pub global: StateGlobal,
    pub init: Option<SubStateInit>,
    pub balance: Option<SubStateBalanceMode>,
    pub quorum: Option<SubStateQuorum>,
    pub phase: Option<SubStatePhase>,
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
    pub change_state: FsmState,
    pub actions: Vec<Action>,
}


/// Datos resultantes de una transición fallida o no permitida.
#[derive(Debug)]
pub struct TransitionInvalid {
    pub invalid: String,
}


/// Acciones o Efectos Secundarios que el sistema debe ejecutar.
///
/// Estas acciones son generadas por la FSM pero ejecutadas externamente (envío de mensajes, timers, etc.).
#[derive(Debug, PartialEq)]
pub enum Action {
    SendToServerHello,
    InitTimerPhase,
    SendAlertMessage,
    IncrementAttempts,
    SendHeartbeatMessagePhase,
    SendHeartbeatMessageNormal,
    SendHeartbeatMessageSafeMode,
    StopSendHeartbeatMessagePhase,
    StopSendHeartbeatMessageSafeMode,
    OnEntryBalance(SubStateBalanceMode),
    OnEntryQuorum(SubStateQuorum),
    OnEntryPhase(SubStatePhase),
    OnEntryNormal,
    OnEntrySafeMode,
}


/// Wrapper para vector de acciones (utilidad).
#[derive(Debug, PartialEq)]
pub enum ActionVector {
    Vector(Vec<Action>),
}


/// Eventos que alimentan la FSM.
///
/// Estos son los "Triggers" que provocan los cambios de estado.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    WaitOk,
    TimeoutWaitOk,
    BalanceEpochOk,
    BalanceEpochNotOk,
    TimeoutInHandshake,
    ApproveQuorum,
    NotApproveQuorum,
    NotApproveNotAttempts,
    TimeoutAlert,
    TimeoutOutHandshake,
    TimeoutSafeMode,
}


/// Banderas de configuración o estado del sistema (Diagnóstico).
#[derive(Debug, Clone, PartialEq)]
pub enum Flag {
    MosquittoConf,
    MtlsConf,
    MosquittoServiceInactive,
    Null,
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
            (Some(SubStateInit::WaitConfirmation), Event::TimeoutWaitOk) => {
                let next_fsm = self.clone();
                state_wait_confirmation_event_timeout(next_fsm)
            },
            (Some(SubStateInit::WaitConfirmation), Event::WaitOk) => {
                let next_fsm = self.clone();
                state_wait_confirmation_event_wait(next_fsm)
            },
            _ => invalid_init()
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
            (Some(SubStateBalanceMode::InHandshake), Event::TimeoutInHandshake) => {
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
                    (Some(SubStateQuorum::RepeatHandshakeIn), _ ) => {
                        let next_fsm = self.clone();
                        state_repeat_handshake_in(next_fsm)
                    },
                    (Some(SubStateQuorum::RepeatHandshakeOut), _ ) => {
                        let next_fsm = self.clone();
                        state_repeat_handshake_out(next_fsm)
                    },
                    _ => invalid_balance_mode()
                }
            },
            (Some(SubStateBalanceMode::Phase), _ ) => {
                match (&self.phase, &event) {
                    (Some(SubStatePhase::Alert), Event::TimeoutAlert) => {
                        let next_fsm = self.clone();
                        state_alert_event_timeout(next_fsm)
                    },
                    (Some(SubStatePhase::Data), _ ) => {
                        let next_fsm = self.clone();
                        state_data_event_timeout(next_fsm)
                    },
                    (Some(SubStatePhase::Monitor), _ ) => {
                        let next_fsm = self.clone();
                        state_monitor_event_timeout(next_fsm)
                    },
                    _ => invalid_balance_mode()
                }
            },
            (Some(SubStateBalanceMode::OutHandshake), Event::TimeoutOutHandshake) => {
                let next_fsm = self.clone();
                state_out_handshake_event_timeout(next_fsm)
            },
            _ => invalid_balance_mode()
        }
    }

    /// Maneja transiciones cuando el estado global es `SafeMode`.
    fn step_safe_mode(&self, event: Event) -> Transition {
        if event == Event::TimeoutSafeMode {
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
fn invalid_init() -> Transition {
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
        actions: vec![Action::IncrementAttempts],
    };
    Transition::Valid(valid)
}


/// Transición: RepeatHandshakeOut -> CheckQuorumOut.
fn state_repeat_handshake_out(mut next_fsm: FsmState) -> Transition {
    next_fsm.quorum = Some(SubStateQuorum::CheckQuorumOut);

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![Action::IncrementAttempts],
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


/// Retorna una transición inválida genérica para BalanceMode.
fn invalid_balance_mode() -> Transition {
    let invalid = TransitionInvalid {
        invalid: "Transición inválida".to_string(),
    };
    Transition::Invalid(invalid)
}


/// Calcula las acciones de entrada (`OnEntry...`) detectando cambios de estado.
///
/// Compara el estado anterior (`old`) con el nuevo (`new`) para identificar
/// qué sub-estados han cambiado e inyectar las acciones correspondientes.
fn compute_on_entry(old: &FsmState, new: &FsmState) -> Vec<Action> {
    let mut actions = Vec::new();

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


/// Ejecutor de acciones (Side Effects).
///
/// Esta función es responsable de traducir las acciones abstractas de la FSM
/// en lógica de negocio real (envío de mensajes, timers, cálculos).
pub fn handle_action(action: Action) {
    match action {
        Action::OnEntryBalance(SubStateBalanceMode::InitBalanceMode) => {
            // UPDATE EPOCH
        },
        Action::OnEntryBalance(SubStateBalanceMode::InHandshake) => {
            /* UPDATE STATE MESSAGE
               SEND HANDSHAKE MESSAGE
               INIT TIMER IN HANDSHAKE
            */
        },
        Action::OnEntryBalance(SubStateBalanceMode::OutHandshake) => {
            /* UPDATE STATE MESSAGE
               SEND HANDSHAKE MESSAGE
               INIT TIMER OUT HANDSHAKE
            */
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
            */
        },
        Action::OnEntryQuorum(SubStateQuorum::RepeatHandshakeOut) => {
            /* SEND HANDSHAKE MESSAGE
               INIT TIMER IN HANDSHAKE
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
        },
        Action::OnEntrySafeMode => {
            // UPDATE STATE MESSAGE
            // INIT TIMER SAFE MODE
            // SEND HEARTBEAT SAFE MODE MESSAGE
        },
        _ => {},
    }
}