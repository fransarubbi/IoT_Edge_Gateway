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


use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument};
use crate::context::domain::AppContext;
use crate::fsm::logic::{fsm, heartbeat_generator, heartbeat_generator_timer, run_fsm};
use crate::message::domain::{HubMessage, ServerMessage};


pub enum FsmServiceResponse {
    NewEpoch(u32),
    GetEpoch,
    ToServer(ServerMessage),
    ToHub(HubMessage),
}


pub enum FsmServiceCommand {
    ErrorEpoch,
    Epoch(u32),
    FromHub(HubMessage),
    FromServer(ServerMessage),
    CreateRuntime,
    DeleteRuntime,
}


pub struct FsmService {
    sender: mpsc::Sender<FsmServiceResponse>,
    receiver: mpsc::Receiver<FsmServiceCommand>,
    context: AppContext,
}


struct FsmRuntime {
    handles: Vec<JoinHandle<()>>,
    cancel_token: CancellationToken,
    tx_command: mpsc::Sender<FsmServiceCommand>,
    rx_response: mpsc::Receiver<FsmServiceResponse>,
    rx_heartbeat_response: mpsc::Receiver<FsmServiceResponse>,
}



impl FsmService {
    pub fn new(sender: mpsc::Sender<FsmServiceResponse>,
               receiver: mpsc::Receiver<FsmServiceCommand>,
               context: AppContext) -> Self {
        Self {
            sender,
            receiver,
            context,
        }
    }

    fn spawn_runtime(&self) -> FsmRuntime {

        let token = CancellationToken::new();
        let mut handles = Vec::new();

        let (tx_command, rx_command) = mpsc::channel::<FsmServiceCommand>(50);
        let (tx_to_core, rx_response) = mpsc::channel::<FsmServiceResponse>(50);
        let (tx_to_fsm, rx_event) = mpsc::channel::<Event>(50);
        let (general_tx_to_timer, rx_from_general) = mpsc::channel::<Event>(50);
        let (general_tx_to_heartbeat, rx_heartbeat_from_general) = mpsc::channel::<Action>(50);
        let (tx_actions, rx_from_fsm) = mpsc::channel::<Vec<Action>>(50);
        let (tx_to_heartbeat, rx_from_heartbeat_watchdog) = mpsc::channel::<Event>(50);
        let (heartbeat_tx_to_timer, rx_from_heartbeat) = mpsc::channel::<Event>(50);
        let (heartbeat_tx_to_core, rx_heartbeat_response) = mpsc::channel::<FsmServiceResponse>(50);

        let child_token = token.child_token();
        let general_tx_to_fsm = tx_to_fsm.clone();
        handles.push(tokio::spawn(fsm(tx_to_core,
                                      general_tx_to_fsm,
                                      general_tx_to_timer,
                                      general_tx_to_heartbeat,
                                      rx_command,
                                      rx_from_fsm,
                                      self.context.clone(),
                                      child_token)));

        let child_token = token.child_token();
        handles.push(tokio::spawn(run_fsm(tx_actions,
                                          rx_event,
                                          child_token)));

        let child_token = token.child_token();
        let timer_tx_to_fsm = tx_to_fsm.clone();
        handles.push(tokio::spawn(fsm_watchdog_timer(timer_tx_to_fsm,
                                                     rx_from_general,
                                                     child_token)));

        let child_token = token.child_token();
        handles.push(tokio::spawn(heartbeat_generator_timer(tx_to_heartbeat,
                                                            rx_from_heartbeat,
                                                            child_token)));

        let child_token = token.child_token();
        handles.push(tokio::spawn(heartbeat_generator(heartbeat_tx_to_core,
                                                      heartbeat_tx_to_timer,
                                                      rx_heartbeat_from_general,
                                                      rx_from_heartbeat_watchdog,
                                                      self.context.clone(),
                                                      child_token)));

        FsmRuntime {
            handles,
            cancel_token: token,
            tx_command,
            rx_response,
            rx_heartbeat_response,
        }
    }

    pub async fn run(mut self, shutdown: CancellationToken) {

        let mut runtime: Option<FsmRuntime> = None;

        loop {
            match runtime {
                Some(ref mut rt) => {
                    tokio::select! {
                        _ = shutdown.cancelled() => {
                            info!("Info: shutdown recibido FsmService");
                            if let Some(rt) = runtime.take() {
                                rt.cancel_token.cancel();
                                for h in rt.handles {
                                    let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
                                }
                            }
                            break;
                        }
                        Some(cmd) = self.receiver.recv() => {
                            match cmd {
                                FsmServiceCommand::Epoch(epoch) => {
                                    if rt.tx_command.send(FsmServiceCommand::Epoch(epoch)).await.is_err() {
                                        error!("Error: no se pudo enviar Epoch a fsm");
                                    }
                                },
                                FsmServiceCommand::ErrorEpoch => {
                                    if rt.tx_command.send(FsmServiceCommand::ErrorEpoch).await.is_err() {
                                        error!("Error: no se pudo enviar ErrorEpoch a fsm");
                                    }
                                },
                                FsmServiceCommand::FromServer(server_message) => {
                                    if rt.tx_command.send(FsmServiceCommand::FromServer(server_message)).await.is_err() {
                                        error!("Error: no se pudo enviar mensaje FromServer a fsm");
                                    }
                                },
                                FsmServiceCommand::FromHub(hub_message) => {
                                    if rt.tx_command.send(FsmServiceCommand::FromHub(hub_message)).await.is_err() {
                                        error!("Error: no se pudo enviar FromHub a fsm");
                                    }
                                },
                                FsmServiceCommand::DeleteRuntime => {
                                    if let Some(rt) = runtime.take() {
                                        rt.cancel_token.cancel();
                                        for h in rt.handles {
                                            let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        Some(response) = rt.rx_response.recv() => {
                            match response {
                                FsmServiceResponse::GetEpoch => {
                                    if self.sender.send(FsmServiceResponse::GetEpoch).await.is_err() {
                                        error!("Error: no se pudo enviar GetEpoch desde FsmService al Core");
                                    }
                                },
                                FsmServiceResponse::NewEpoch(epoch) => {
                                    if self.sender.send(FsmServiceResponse::NewEpoch(epoch)).await.is_err() {
                                        error!("Error: no se pudo enviar NewEpoch desde FsmService al Core");
                                    }
                                },
                                FsmServiceResponse::ToHub(hub_message) => {
                                    match hub_message {
                                        HubMessage::HandshakeToHub(handshake) => {
                                            if self.sender.send(FsmServiceResponse::ToHub(HubMessage::HandshakeToHub(handshake))).await.is_err() {
                                                error!("Error: no se pudo enviar mensaje de HandshakeToHub desde FsmService al Core");
                                            }
                                        },
                                        HubMessage::StateBalanceMode(state) => {
                                            if self.sender.send(FsmServiceResponse::ToHub(HubMessage::StateBalanceMode(state))).await.is_err() {
                                                error!("Error: no se pudo enviar mensaje de StateBalanceMode desde FsmService al Core");
                                            }
                                        },
                                        HubMessage::PhaseNotification(phase) => {
                                            if self.sender.send(FsmServiceResponse::ToHub(HubMessage::PhaseNotification(phase))).await.is_err() {
                                                error!("Error: no se pudo enviar mensaje de PhaseNotification desde FsmService al Core");
                                            }
                                        },
                                        HubMessage::StateNormal(state) => {
                                            if self.sender.send(FsmServiceResponse::ToHub(HubMessage::StateNormal(state))).await.is_err() {
                                                error!("Error: no se pudo enviar mensaje de StateNormal desde FsmService al Core");
                                            }
                                        },
                                        HubMessage::StateSafeMode(state) => {
                                            if self.sender.send(FsmServiceResponse::ToHub(HubMessage::StateSafeMode(state))).await.is_err() {
                                                error!("Error: no se pudo enviar mensaje de StateSafeMode desde FsmService al Core");
                                            }
                                        },
                                        HubMessage::Ping(msg) => {
                                            if self.sender.send(FsmServiceResponse::ToHub(HubMessage::Ping(msg))).await.is_err() {
                                                error!("Error: no se pudo enviar mensaje de Ping desde FsmService al Core");
                                            }
                                        },
                                        _ => {}
                                    }
                                },
                                FsmServiceResponse::ToServer(server_message) => {
                                    match server_message {
                                        ServerMessage::HelloWorld(hello) => {
                                            if self.sender.send(FsmServiceResponse::ToServer(ServerMessage::HelloWorld(hello))).await.is_err() {
                                                error!("Error: no se pudo enviar mensaje de Ping desde FsmService al Core");
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        Some(heartbeat) = rt.rx_heartbeat_response.recv() => {
                            match heartbeat {
                                FsmServiceResponse::ToHub(HubMessage::Heartbeat(heartbeat)) => {
                                    if self.sender.send(FsmServiceResponse::ToHub(HubMessage::Heartbeat(heartbeat))).await.is_err() {
                                        error!("Error: no se pudo enviar Heartbeat desde FsmService al Core");
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                None => {
                    tokio::select! {
                        _ = shutdown.cancelled() => {
                            info!("Info: Shutdown recibido FsmService");
                            break;
                        }
                        
                        Some(cmd) = self.receiver.recv() => {
                            match cmd {
                                FsmServiceCommand::CreateRuntime => {
                                    if runtime.is_none() {
                                        runtime = Some(self.spawn_runtime());
                                    }
                                },
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }
}



/// Estados Globales de nivel superior.
///
/// Determinan el comportamiento macro del sistema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StateGlobal {
    Start,
    /// **Modo de Balanceo:** Fase crítica de negociación distribuida. El dispositivo
    /// intenta sincronizarse con sus hubs, verificar quórum y establecer su rol.
    BalanceMode,
    /// **Operación Normal:** El dispositivo ha completado el balanceo exitosamente y opera en régimen estable.
    Normal,
    /// **Modo Seguro:** Estado de fallo o emergencia. El dispositivo entra aquí tras errores críticos
    /// (DB corrupta, fallo de consenso repetido) para evitar operaciones inseguras.
    SafeMode,
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
    SendHeartbeatMessagePhase,
    SendHeartbeatMessageSafeMode,
    SendHeartbeatMessageNormal,
    StopSendHeartbeatMessagePhase,
    StopSendHeartbeatMessageSafeMode,
    OnEntryBalance(SubStateBalanceMode),
    OnEntryQuorum(SubStateQuorum),
    OnEntryPhase(SubStatePhase),
    OnEntryNormal,
    OnEntrySafeMode,
    StopTimer,
}


/// Eventos que alimentan la FSM.
///
/// Estos son los "Triggers" que provocan los cambios de estado.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Start,
    WaitOk,
    Timeout,
    BalanceEpochOk,
    BalanceEpochNotOk,
    ApproveQuorum,
    QuorumPhase,
    QuorumSafeMode,
    NotApproveQuorum,
    NotApproveNotAttempts,
    InitTimer(Duration),
    StopTimer,
}


#[derive(Debug, Clone, PartialEq)]
pub enum StateOfSession {
    None,
    InHandshake,
    Quorum,
    RepeatHandshake,
    OutHandshake,
    PhaseAlert,
    PhaseData,
    PhaseMonitor,
    SafeMode,
}


pub struct UpdateSession {
    empty_hash: HashMap<String, bool>,
    handshake_hash: HashMap<String, u32>,
    state: StateOfSession,
    total_attempts: f64,
}


impl UpdateSession {
    pub fn new() -> Self {
        Self {
            empty_hash: HashMap::new(),
            handshake_hash: HashMap::new(),
            state: StateOfSession::None,
            total_attempts: 0.0,
        }
    }

    pub fn reset_total_attempts(&mut self) {
        self.total_attempts = 0.0;
    }

    pub fn reset_empty_hash(&mut self) {
        self.empty_hash.clear();
    }

    pub fn reset_handshake_hash(&mut self) {
        self.handshake_hash.clear();
    }

    pub fn insert_empty(&mut self, key: String, value: bool) {
        if !self.empty_hash.contains_key(&key) {
            self.empty_hash.insert(key, value);
        }
    }

    pub fn insert_handshake(&mut self, key: String, value: u32) {
        if !self.handshake_hash.contains_key(&key) {
            self.handshake_hash.insert(key, value);
        }
    }

    pub fn set_state(&mut self, state: StateOfSession) {
        self.state = state;
    }

    pub fn increment_attempts(&mut self) {
        self.total_attempts += 1.0;
    }

    pub fn get_state(&self) -> &StateOfSession {
        &self.state
    }

    pub fn get_total_handshake(&self) -> u64 { self.handshake_hash.len() as u64 }

    pub fn get_total_attempts(&self) -> f64 {
        self.total_attempts
    }

    pub fn get_total_empty(&self) -> u64 { self.empty_hash.len() as u64 }
}


impl FsmState {
    /// Crea una nueva instancia de la FSM en el estado inicial.
    pub fn new() -> Self {
        Self {
            global: StateGlobal::Start,
            balance: None,
            quorum: None,
            phase: None,
        }
    }

    /// Lógica interna de despacho según el estado global.
    fn step_inner(&self, event: Event) -> Transition {
        match self.global {
            StateGlobal::Start => self.step_start(event),
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

    fn step_start(&self, event: Event) -> Transition {
        match (&self.global, event) {
            (StateGlobal::Start, Event::Start) => {
                let mut next_fsm = self.clone();
                next_fsm.global = StateGlobal::BalanceMode;
                next_fsm.balance = Some(SubStateBalanceMode::InitBalanceMode);

                let valid = TransitionValid {
                    change_state: next_fsm,
                    actions: vec![Action::OnEntryBalance(SubStateBalanceMode::InitBalanceMode)],
                };
                Transition::Valid(valid)
            }
            _ => {
                let invalid = TransitionInvalid {
                    invalid: "Invalid".to_string(),
                };
                Transition::Invalid(invalid)
            },
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
                    (Some(SubStatePhase::Alert), Event::Timeout | Event::QuorumPhase) => {
                        let next_fsm = self.clone();
                        state_alert_event_timeout_or_quorum(next_fsm)
                    },
                    (Some(SubStatePhase::Data), Event::Timeout | Event::QuorumPhase) => {
                        let next_fsm = self.clone();
                        state_data_event_timeout_or_quorum(next_fsm)
                    },
                    (Some(SubStatePhase::Monitor), Event::Timeout | Event::QuorumPhase) => {
                        let next_fsm = self.clone();
                        state_monitor_event_timeout_or_quorum(next_fsm)
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
        if event == Event::Timeout || event == Event::QuorumSafeMode {
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
fn state_alert_event_timeout_or_quorum(mut next_fsm: FsmState) -> Transition {
    next_fsm.phase = Some(SubStatePhase::Data);

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Transición: Data -> Monitor.
fn state_data_event_timeout_or_quorum(mut next_fsm: FsmState) -> Transition {
    next_fsm.phase = Some(SubStatePhase::Monitor);

    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![],
    };
    Transition::Valid(valid)
}


/// Transición: Monitor -> OutHandshake.
fn state_monitor_event_timeout_or_quorum(mut next_fsm: FsmState) -> Transition {
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

    if old.global != new.global && new.global == StateGlobal::Normal {
        actions.push(Action::OnEntryNormal);
    }

    if old.global != new.global && new.global == StateGlobal::SafeMode {
        actions.push(Action::OnEntrySafeMode);
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
#[instrument(name = "fsm_watchdog_timer", skip(cmd_rx))]
pub async fn fsm_watchdog_timer(tx_to_fsm: mpsc::Sender<Event>,
                                mut cmd_rx: mpsc::Receiver<Event>,
                                cancel: CancellationToken) {
    loop {
        let duration = match cmd_rx.recv().await {
            Some(Event::InitTimer(d)) => d,
            Some(Event::StopTimer) => continue, // Si ya estaba parado, ignorar
            None => break, // Canal cerrado, terminar tarea
            _ => continue,
        };

        // Estado ACTIVO: Corriendo temporizador
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("Info: shutdown recibido fsm_watchdog_timer");
                break;
            }
            _ = sleep(duration) => {
                // El tiempo se agotó
                let _ = tx_to_fsm.send(Event::Timeout).await;
            }
            Some(Event::StopTimer) = cmd_rx.recv() => {
                debug!("Debug: Watchdog timer de fsm general, cancelado");
            }
        }
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    // ===== Funciones auxiliares =====

    /// Verifica si una acción específica está presente en el vector de acciones
    fn contains_action(actions: &[Action], target: &Action) -> bool {
        actions.iter().any(|a| a == target)
    }

    /// Trait para desempaquetar transiciones válidas de forma más limpia
    trait TransitionExt {
        fn unwrap_valid(self) -> TransitionValid;
        fn unwrap_invalid(self) -> TransitionInvalid;
    }

    impl TransitionExt for Transition {
        fn unwrap_valid(self) -> TransitionValid {
            match self {
                Transition::Valid(v) => v,
                Transition::Invalid(i) => panic!("Se esperaba transición válida, se obtuvo: {}", i.get_invalid()),
            }
        }

        fn unwrap_invalid(self) -> TransitionInvalid {
            match self {
                Transition::Invalid(i) => i,
                Transition::Valid(_) => panic!("Se esperaba transición inválida, pero fue válida"),
            }
        }
    }

    // ===== Tests de inicialización =====

    #[test]
    fn test_nuevo_fsm_estado_inicial() {
        let fsm = FsmState::new();
        assert_eq!(fsm.global, StateGlobal::Start);
        assert_eq!(fsm.balance, None);
        assert_eq!(fsm.quorum, None);
        assert_eq!(fsm.phase, None);
    }

    // ===== Tests para StateGlobal::Start =====

    #[test]
    fn test_start_evento_start() {
        let fsm = FsmState::new();
        let transition = fsm.step(Event::Start);

        let valid = transition.unwrap_valid();
        let new_fsm = valid.get_change_state();
        let actions = valid.get_actions();

        assert_eq!(new_fsm.global, StateGlobal::BalanceMode);
        assert_eq!(new_fsm.balance, Some(SubStateBalanceMode::InitBalanceMode));
        assert!(contains_action(&actions, &Action::OnEntryBalance(SubStateBalanceMode::InitBalanceMode)));
    }

    #[test]
    fn test_start_evento_invalido() {
        let fsm = FsmState::new();

        // Todos estos eventos deberían ser inválidos en Start
        let eventos_invalidos = vec![
            Event::WaitOk,
            Event::Timeout,
            Event::BalanceEpochOk,
            Event::StopTimer,
        ];

        for evento in eventos_invalidos {
            let transition = fsm.step(evento);
            match transition {
                Transition::Invalid(_) => {},
                _ => panic!("Se esperaba transición inválida para evento en Start"),
            }
        }
    }

    // ===== Tests para StateGlobal::BalanceMode - SubStateBalanceMode::InitBalanceMode =====

    #[test]
    fn test_init_balance_mode_evento_balance_epoch_ok() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::InitBalanceMode);

        let transition = fsm.step(Event::BalanceEpochOk);
        let valid = transition.unwrap_valid();
        let new_fsm = valid.get_change_state();
        let actions = valid.get_actions();

        assert_eq!(new_fsm.global, StateGlobal::BalanceMode);
        assert_eq!(new_fsm.balance, Some(SubStateBalanceMode::InHandshake));
        assert!(contains_action(&actions, &Action::OnEntryBalance(SubStateBalanceMode::InHandshake)));
    }

    #[test]
    fn test_init_balance_mode_evento_balance_epoch_not_ok() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::InitBalanceMode);

        let transition = fsm.step(Event::BalanceEpochNotOk);
        let valid = transition.unwrap_valid();
        let new_fsm = valid.get_change_state();

        assert_eq!(new_fsm.global, StateGlobal::SafeMode);
        assert_eq!(new_fsm.balance, None);
    }

    #[test]
    fn test_init_balance_mode_eventos_invalidos() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::InitBalanceMode);

        let eventos_invalidos = vec![
            Event::Timeout,
            Event::ApproveQuorum,
            Event::WaitOk,
        ];

        for evento in eventos_invalidos {
            let transition = fsm.step(evento);
            transition.unwrap_invalid();
        }
    }

    // ===== Tests para SubStateBalanceMode::InHandshake =====

    #[test]
    fn test_in_handshake_evento_timeout() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::InHandshake);

        let transition = fsm.step(Event::Timeout);
        let valid = transition.unwrap_valid();
        let new_fsm = valid.get_change_state();
        let actions = valid.get_actions();

        assert_eq!(new_fsm.balance, Some(SubStateBalanceMode::Quorum));
        assert_eq!(new_fsm.quorum, Some(SubStateQuorum::CheckQuorumIn));
        assert!(contains_action(&actions, &Action::OnEntryBalance(SubStateBalanceMode::Quorum)));
        assert!(contains_action(&actions, &Action::OnEntryQuorum(SubStateQuorum::CheckQuorumIn)));
    }

    // ===== Tests para SubStateQuorum::CheckQuorumIn =====

    #[test]
    fn test_check_quorum_in_evento_approve_quorum() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::Quorum);
        fsm.quorum = Some(SubStateQuorum::CheckQuorumIn);

        let transition = fsm.step(Event::ApproveQuorum);
        let valid = transition.unwrap_valid();
        let new_fsm = valid.get_change_state();
        let actions = valid.get_actions();

        assert_eq!(new_fsm.balance, Some(SubStateBalanceMode::Phase));
        assert_eq!(new_fsm.phase, Some(SubStatePhase::Alert));
        assert_eq!(new_fsm.quorum, None);
        assert!(contains_action(&actions, &Action::OnEntryBalance(SubStateBalanceMode::Phase)));
        assert!(contains_action(&actions, &Action::OnEntryPhase(SubStatePhase::Alert)));
    }

    #[test]
    fn test_check_quorum_in_evento_not_approve_quorum() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::Quorum);
        fsm.quorum = Some(SubStateQuorum::CheckQuorumIn);

        let transition = fsm.step(Event::NotApproveQuorum);
        let valid = transition.unwrap_valid();
        let new_fsm = valid.get_change_state();
        let actions = valid.get_actions();

        assert_eq!(new_fsm.quorum, Some(SubStateQuorum::RepeatHandshakeIn));
        assert!(contains_action(&actions, &Action::OnEntryQuorum(SubStateQuorum::RepeatHandshakeIn)));
    }

    #[test]
    fn test_check_quorum_in_evento_not_approve_not_attempts() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::Quorum);
        fsm.quorum = Some(SubStateQuorum::CheckQuorumIn);

        let transition = fsm.step(Event::NotApproveNotAttempts);
        let valid = transition.unwrap_valid();
        let new_fsm = valid.get_change_state();

        assert_eq!(new_fsm.global, StateGlobal::SafeMode);
        assert_eq!(new_fsm.balance, None);
        assert_eq!(new_fsm.quorum, None);
    }

    // ===== Tests para SubStateQuorum::CheckQuorumOut =====

    #[test]
    fn test_check_quorum_out_evento_approve_quorum() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::Quorum);
        fsm.quorum = Some(SubStateQuorum::CheckQuorumOut);

        let transition = fsm.step(Event::ApproveQuorum);
        let valid = transition.unwrap_valid();
        let new_fsm = valid.get_change_state();

        assert_eq!(new_fsm.global, StateGlobal::Normal);
        assert_eq!(new_fsm.quorum, None);
    }

    #[test]
    fn test_check_quorum_out_evento_not_approve_quorum() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::Quorum);
        fsm.quorum = Some(SubStateQuorum::CheckQuorumOut);

        let transition = fsm.step(Event::NotApproveQuorum);
        let valid = transition.unwrap_valid();
        let new_fsm = valid.get_change_state();
        let actions = valid.get_actions();

        assert_eq!(new_fsm.quorum, Some(SubStateQuorum::RepeatHandshakeOut));
        assert!(contains_action(&actions, &Action::OnEntryQuorum(SubStateQuorum::RepeatHandshakeOut)));
    }

    #[test]
    fn test_check_quorum_out_evento_not_approve_not_attempts() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::Quorum);
        fsm.quorum = Some(SubStateQuorum::CheckQuorumOut);

        let transition = fsm.step(Event::NotApproveNotAttempts);
        let valid = transition.unwrap_valid();
        let new_fsm = valid.get_change_state();

        assert_eq!(new_fsm.global, StateGlobal::SafeMode);
        assert_eq!(new_fsm.balance, None);
        assert_eq!(new_fsm.quorum, None);
    }

    // ===== Tests para SubStateQuorum::RepeatHandshakeIn =====

    #[test]
    fn test_repeat_handshake_in_evento_timeout() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::Quorum);
        fsm.quorum = Some(SubStateQuorum::RepeatHandshakeIn);

        let transition = fsm.step(Event::Timeout);
        let valid = transition.unwrap_valid();
        let new_fsm = valid.get_change_state();
        let actions = valid.get_actions();

        assert_eq!(new_fsm.quorum, Some(SubStateQuorum::CheckQuorumIn));
        assert!(contains_action(&actions, &Action::OnEntryQuorum(SubStateQuorum::CheckQuorumIn)));
    }

    // ===== Tests para SubStateQuorum::RepeatHandshakeOut =====

    #[test]
    fn test_repeat_handshake_out_evento_timeout() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::Quorum);
        fsm.quorum = Some(SubStateQuorum::RepeatHandshakeOut);

        let transition = fsm.step(Event::Timeout);
        let valid = transition.unwrap_valid();
        let new_fsm = valid.get_change_state();
        let actions = valid.get_actions();

        assert_eq!(new_fsm.quorum, Some(SubStateQuorum::CheckQuorumOut));
        assert!(contains_action(&actions, &Action::OnEntryQuorum(SubStateQuorum::CheckQuorumOut)));
    }

    // ===== Tests para SubStatePhase::Alert =====

    #[test]
    fn test_alert_evento_timeout() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::Phase);
        fsm.phase = Some(SubStatePhase::Alert);

        let transition = fsm.step(Event::Timeout);
        let valid = transition.unwrap_valid();
        let new_fsm = valid.get_change_state();
        let actions = valid.get_actions();

        assert_eq!(new_fsm.phase, Some(SubStatePhase::Data));
        assert!(contains_action(&actions, &Action::OnEntryPhase(SubStatePhase::Data)));
    }

    // ===== Tests para SubStatePhase::Data =====

    #[test]
    fn test_data_evento_timeout() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::Phase);
        fsm.phase = Some(SubStatePhase::Data);

        let transition = fsm.step(Event::Timeout);
        let valid = transition.unwrap_valid();
        let new_fsm = valid.get_change_state();
        let actions = valid.get_actions();

        assert_eq!(new_fsm.phase, Some(SubStatePhase::Monitor));
        assert!(contains_action(&actions, &Action::OnEntryPhase(SubStatePhase::Monitor)));
    }

    // ===== Tests para SubStatePhase::Monitor =====

    #[test]
    fn test_monitor_evento_timeout() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::Phase);
        fsm.phase = Some(SubStatePhase::Monitor);

        let transition = fsm.step(Event::Timeout);
        let valid = transition.unwrap_valid();
        let new_fsm = valid.get_change_state();
        let actions = valid.get_actions();

        assert_eq!(new_fsm.balance, Some(SubStateBalanceMode::OutHandshake));
        assert_eq!(new_fsm.phase, None);
        assert!(contains_action(&actions, &Action::StopSendHeartbeatMessagePhase));
        assert!(contains_action(&actions, &Action::OnEntryBalance(SubStateBalanceMode::OutHandshake)));
    }

    // ===== Tests para SubStateBalanceMode::OutHandshake =====

    #[test]
    fn test_out_handshake_evento_timeout() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::OutHandshake);

        let transition = fsm.step(Event::Timeout);
        let valid = transition.unwrap_valid();
        let new_fsm = valid.get_change_state();
        let actions = valid.get_actions();

        assert_eq!(new_fsm.balance, Some(SubStateBalanceMode::Quorum));
        assert_eq!(new_fsm.quorum, Some(SubStateQuorum::CheckQuorumOut));
        assert!(contains_action(&actions, &Action::OnEntryBalance(SubStateBalanceMode::Quorum)));
        assert!(contains_action(&actions, &Action::OnEntryQuorum(SubStateQuorum::CheckQuorumOut)));
    }

    // ===== Tests para StateGlobal::SafeMode =====

    #[test]
    fn test_safe_mode_evento_timeout() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::SafeMode;

        let transition = fsm.step(Event::Timeout);
        let valid = transition.unwrap_valid();
        let actions = valid.get_actions();

        assert!(contains_action(&actions, &Action::StopSendHeartbeatMessageSafeMode));
    }

    #[test]
    fn test_safe_mode_eventos_invalidos() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::SafeMode;

        let eventos_invalidos = vec![
            Event::WaitOk,
            Event::ApproveQuorum,
            Event::Start,
        ];

        for evento in eventos_invalidos {
            let transition = fsm.step(evento);
            transition.unwrap_invalid();
        }
    }

    // ===== Tests para StateGlobal::Normal =====

    #[test]
    fn test_normal_cualquier_evento_invalido() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::Normal;

        let eventos = vec![
            Event::Start,
            Event::WaitOk,
            Event::Timeout,
            Event::BalanceEpochOk,
            Event::ApproveQuorum,
        ];

        for evento in eventos {
            let transition = fsm.step(evento);
            let invalid = transition.unwrap_invalid();
            assert!(invalid.get_invalid().contains("Normal"));
        }
    }

    // ===== Tests de secuencias completas =====

    #[test]
    fn test_secuencia_completa_exitosa_path_in() {
        let mut fsm = FsmState::new();

        // Start -> BalanceMode.InitBalanceMode
        fsm = fsm.step(Event::Start).unwrap_valid().get_change_state();
        assert_eq!(fsm.global, StateGlobal::BalanceMode);
        assert_eq!(fsm.balance, Some(SubStateBalanceMode::InitBalanceMode));

        // InitBalanceMode -> InHandshake
        fsm = fsm.step(Event::BalanceEpochOk).unwrap_valid().get_change_state();
        assert_eq!(fsm.balance, Some(SubStateBalanceMode::InHandshake));

        // InHandshake -> Quorum.CheckQuorumIn
        fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();
        assert_eq!(fsm.balance, Some(SubStateBalanceMode::Quorum));
        assert_eq!(fsm.quorum, Some(SubStateQuorum::CheckQuorumIn));

        // CheckQuorumIn -> Phase.Alert
        fsm = fsm.step(Event::ApproveQuorum).unwrap_valid().get_change_state();
        assert_eq!(fsm.balance, Some(SubStateBalanceMode::Phase));
        assert_eq!(fsm.phase, Some(SubStatePhase::Alert));

        // Alert -> Data
        fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();
        assert_eq!(fsm.phase, Some(SubStatePhase::Data));

        // Data -> Monitor
        fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();
        assert_eq!(fsm.phase, Some(SubStatePhase::Monitor));

        // Monitor -> OutHandshake
        fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();
        assert_eq!(fsm.balance, Some(SubStateBalanceMode::OutHandshake));
        assert_eq!(fsm.phase, None);

        // OutHandshake -> Quorum.CheckQuorumOut
        fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();
        assert_eq!(fsm.balance, Some(SubStateBalanceMode::Quorum));
        assert_eq!(fsm.quorum, Some(SubStateQuorum::CheckQuorumOut));

        // CheckQuorumOut -> Normal
        fsm = fsm.step(Event::ApproveQuorum).unwrap_valid().get_change_state();
        assert_eq!(fsm.global, StateGlobal::Normal);
    }

    #[test]
    fn test_secuencia_quorum_in_con_retry() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::Quorum);
        fsm.quorum = Some(SubStateQuorum::CheckQuorumIn);

        // CheckQuorumIn -> RepeatHandshakeIn (no aprobado)
        fsm = fsm.step(Event::NotApproveQuorum).unwrap_valid().get_change_state();
        assert_eq!(fsm.quorum, Some(SubStateQuorum::RepeatHandshakeIn));

        // RepeatHandshakeIn -> CheckQuorumIn (timeout)
        fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();
        assert_eq!(fsm.quorum, Some(SubStateQuorum::CheckQuorumIn));

        // CheckQuorumIn -> RepeatHandshakeIn (no aprobado otra vez)
        fsm = fsm.step(Event::NotApproveQuorum).unwrap_valid().get_change_state();
        assert_eq!(fsm.quorum, Some(SubStateQuorum::RepeatHandshakeIn));

        // RepeatHandshakeIn -> CheckQuorumIn (timeout)
        fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();
        assert_eq!(fsm.quorum, Some(SubStateQuorum::CheckQuorumIn));

        // CheckQuorumIn -> Phase.Alert (finalmente aprobado)
        fsm = fsm.step(Event::ApproveQuorum).unwrap_valid().get_change_state();
        assert_eq!(fsm.balance, Some(SubStateBalanceMode::Phase));
        assert_eq!(fsm.phase, Some(SubStatePhase::Alert));
    }

    #[test]
    fn test_secuencia_quorum_out_con_retry() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::Quorum);
        fsm.quorum = Some(SubStateQuorum::CheckQuorumOut);

        // CheckQuorumOut -> RepeatHandshakeOut
        fsm = fsm.step(Event::NotApproveQuorum).unwrap_valid().get_change_state();
        assert_eq!(fsm.quorum, Some(SubStateQuorum::RepeatHandshakeOut));

        // RepeatHandshakeOut -> CheckQuorumOut
        fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();
        assert_eq!(fsm.quorum, Some(SubStateQuorum::CheckQuorumOut));

        // CheckQuorumOut -> Normal (aprobado)
        fsm = fsm.step(Event::ApproveQuorum).unwrap_valid().get_change_state();
        assert_eq!(fsm.global, StateGlobal::Normal);
    }

    #[test]
    fn test_secuencia_fallo_a_safe_mode_desde_init() {
        let mut fsm = FsmState::new();

        // Start -> BalanceMode
        fsm = fsm.step(Event::Start).unwrap_valid().get_change_state();

        // InitBalanceMode -> SafeMode (BalanceEpochNotOk)
        fsm = fsm.step(Event::BalanceEpochNotOk).unwrap_valid().get_change_state();
        assert_eq!(fsm.global, StateGlobal::SafeMode);
        assert_eq!(fsm.balance, None);
    }

    #[test]
    fn test_secuencia_fallo_a_safe_mode_desde_quorum_in() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::Quorum);
        fsm.quorum = Some(SubStateQuorum::CheckQuorumIn);

        // CheckQuorumIn -> SafeMode (sin intentos restantes)
        fsm = fsm.step(Event::NotApproveNotAttempts).unwrap_valid().get_change_state();
        assert_eq!(fsm.global, StateGlobal::SafeMode);
        assert_eq!(fsm.balance, None);
        assert_eq!(fsm.quorum, None);
    }

    #[test]
    fn test_secuencia_fallo_a_safe_mode_desde_quorum_out() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::Quorum);
        fsm.quorum = Some(SubStateQuorum::CheckQuorumOut);

        // CheckQuorumOut -> SafeMode (sin intentos restantes)
        fsm = fsm.step(Event::NotApproveNotAttempts).unwrap_valid().get_change_state();
        assert_eq!(fsm.global, StateGlobal::SafeMode);
        assert_eq!(fsm.balance, None);
        assert_eq!(fsm.quorum, None);
    }

    #[test]
    fn test_ciclo_completo_de_phases() {
        let mut fsm = FsmState::new();
        fsm.global = StateGlobal::BalanceMode;
        fsm.balance = Some(SubStateBalanceMode::Phase);
        fsm.phase = Some(SubStatePhase::Alert);

        // Alert -> Data -> Monitor -> OutHandshake
        fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();
        assert_eq!(fsm.phase, Some(SubStatePhase::Data));

        fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();
        assert_eq!(fsm.phase, Some(SubStatePhase::Monitor));

        let transition = fsm.step(Event::Timeout);
        let valid = transition.unwrap_valid();
        fsm = valid.get_change_state();
        let actions = valid.get_actions();

        assert_eq!(fsm.balance, Some(SubStateBalanceMode::OutHandshake));
        assert_eq!(fsm.phase, None);
        assert!(contains_action(&actions, &Action::StopSendHeartbeatMessagePhase));
    }

    // ===== Tests de compute_on_entry =====

    #[test]
    fn test_on_entry_sin_cambios() {
        let fsm1 = FsmState::new();
        let fsm2 = FsmState::new();

        let actions = compute_on_entry(&fsm1, &fsm2);
        assert_eq!(actions.len(), 0);
    }

    #[test]
    fn test_on_entry_cambio_balance() {
        let mut fsm1 = FsmState::new();
        fsm1.balance = Some(SubStateBalanceMode::InitBalanceMode);

        let mut fsm2 = FsmState::new();
        fsm2.balance = Some(SubStateBalanceMode::InHandshake);

        let actions = compute_on_entry(&fsm1, &fsm2);
        assert_eq!(actions.len(), 1);
        assert!(contains_action(&actions, &Action::OnEntryBalance(SubStateBalanceMode::InHandshake)));
    }

    #[test]
    fn test_on_entry_cambio_quorum() {
        let mut fsm1 = FsmState::new();
        fsm1.quorum = Some(SubStateQuorum::CheckQuorumIn);

        let mut fsm2 = FsmState::new();
        fsm2.quorum = Some(SubStateQuorum::RepeatHandshakeIn);

        let actions = compute_on_entry(&fsm1, &fsm2);
        assert_eq!(actions.len(), 1);
        assert!(contains_action(&actions, &Action::OnEntryQuorum(SubStateQuorum::RepeatHandshakeIn)));
    }

    #[test]
    fn test_on_entry_cambio_phase() {
        let mut fsm1 = FsmState::new();
        fsm1.phase = Some(SubStatePhase::Alert);

        let mut fsm2 = FsmState::new();
        fsm2.phase = Some(SubStatePhase::Data);

        let actions = compute_on_entry(&fsm1, &fsm2);
        assert_eq!(actions.len(), 1);
        assert!(contains_action(&actions, &Action::OnEntryPhase(SubStatePhase::Data)));
    }

    #[test]
    fn test_on_entry_multiples_cambios() {
        let mut fsm1 = FsmState::new();
        fsm1.balance = Some(SubStateBalanceMode::InHandshake);

        let mut fsm2 = FsmState::new();
        fsm2.balance = Some(SubStateBalanceMode::Quorum);
        fsm2.quorum = Some(SubStateQuorum::CheckQuorumIn);

        let actions = compute_on_entry(&fsm1, &fsm2);
        assert_eq!(actions.len(), 2);
        assert!(contains_action(&actions, &Action::OnEntryBalance(SubStateBalanceMode::Quorum)));
        assert!(contains_action(&actions, &Action::OnEntryQuorum(SubStateQuorum::CheckQuorumIn)));
    }

    #[test]
    fn test_on_entry_desaparicion_de_subestado() {
        let mut fsm1 = FsmState::new();
        fsm1.quorum = Some(SubStateQuorum::CheckQuorumIn);

        let mut fsm2 = FsmState::new();
        fsm2.quorum = None;

        let actions = compute_on_entry(&fsm1, &fsm2);
        assert_eq!(actions.len(), 0); // No se genera OnEntry cuando desaparece
    }

    // ===== Tests async del watchdog timer =====

    #[tokio::test]
    async fn test_watchdog_timer_timeout_normal() {
        let (tx_to_fsm, mut rx_from_timer) = mpsc::channel(10);
        let (tx_cmd, rx_cmd) = mpsc::channel(10);

        let token = CancellationToken::new();
        tokio::spawn(async move {
            fsm_watchdog_timer(tx_to_fsm, rx_cmd, token).await;
        });

        // Iniciar timer con duración corta
        tx_cmd.send(Event::InitTimer(Duration::from_millis(50))).await.unwrap();

        // Esperar el timeout
        match tokio::time::timeout(Duration::from_millis(200), rx_from_timer.recv()).await {
            Ok(Some(Event::Timeout)) => {},
            _ => panic!("Se esperaba evento Timeout"),
        }
    }

    #[tokio::test]
    async fn test_watchdog_timer_cancelacion() {
        let (tx_to_fsm, mut rx_from_timer) = mpsc::channel(10);
        let (tx_cmd, rx_cmd) = mpsc::channel(10);

        tokio::spawn(async move {
            let token = CancellationToken::new();
            fsm_watchdog_timer(tx_to_fsm, rx_cmd, token).await;
        });

        // Iniciar timer
        tx_cmd.send(Event::InitTimer(Duration::from_millis(200))).await.unwrap();

        // Cancelar inmediatamente
        sleep(Duration::from_millis(10)).await;
        tx_cmd.send(Event::StopTimer).await.unwrap();

        // No debería recibir timeout
        match tokio::time::timeout(Duration::from_millis(300), rx_from_timer.recv()).await {
            Err(_) => {}, // Timeout esperado, no se recibió evento
            Ok(_) => panic!("No debería recibir ningún evento después de cancelar"),
        }
    }

    #[tokio::test]
    async fn test_watchdog_timer_multiples_ciclos() {
        let (tx_to_fsm, mut rx_from_timer) = mpsc::channel(10);
        let (tx_cmd, rx_cmd) = mpsc::channel(10);

        tokio::spawn(async move {
            let token = CancellationToken::new();
            fsm_watchdog_timer(tx_to_fsm, rx_cmd, token).await;
        });

        // Ejecutar 3 ciclos de timer
        for i in 0..3 {
            tx_cmd.send(Event::InitTimer(Duration::from_millis(50))).await.unwrap();
            match tokio::time::timeout(Duration::from_millis(100), rx_from_timer.recv()).await {
                Ok(Some(Event::Timeout)) => {},
                _ => panic!("Se esperaba Timeout en ciclo {}", i),
            }
        }
    }

    #[tokio::test]
    async fn test_watchdog_timer_stop_antes_de_init() {
        let (tx_to_fsm, _rx_from_timer) = mpsc::channel(10);
        let (tx_cmd, rx_cmd) = mpsc::channel(10);

        tokio::spawn(async move {
            let token = CancellationToken::new();
            fsm_watchdog_timer(tx_to_fsm, rx_cmd, token).await;
        });

        // Enviar StopTimer sin InitTimer primero (debe ignorarse)
        tx_cmd.send(Event::StopTimer).await.unwrap();

        // Luego iniciar normalmente
        tx_cmd.send(Event::InitTimer(Duration::from_millis(50))).await.unwrap();

        // Debe funcionar normalmente
        sleep(Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn test_watchdog_timer_canal_cerrado() {
        let (tx_to_fsm, _rx_from_timer) = mpsc::channel(10);
        let (tx_cmd, rx_cmd) = mpsc::channel(10);

        let handle = tokio::spawn(async move {
            let token = CancellationToken::new();
            fsm_watchdog_timer(tx_to_fsm, rx_cmd, token).await;
        });

        // Cerrar canal de comandos
        drop(tx_cmd);

        // El watchdog debe terminar limpiamente
        tokio::time::timeout(Duration::from_millis(100), handle)
            .await
            .expect("El watchdog debería terminar cuando se cierra el canal")
            .expect("La tarea debería completarse exitosamente");
    }

    #[tokio::test]
    async fn test_watchdog_timer_evento_invalido_ignorado() {
        let (tx_to_fsm, mut rx_from_timer) = mpsc::channel(10);
        let (tx_cmd, rx_cmd) = mpsc::channel(10);

        tokio::spawn(async move {
            let token = CancellationToken::new();
            fsm_watchdog_timer(tx_to_fsm, rx_cmd, token).await;
        });

        // Enviar evento inválido
        tx_cmd.send(Event::WaitOk).await.unwrap();

        // Iniciar timer normal
        tx_cmd.send(Event::InitTimer(Duration::from_millis(50))).await.unwrap();

        // El timer debe funcionar normalmente a pesar del evento inválido
        match tokio::time::timeout(Duration::from_millis(100), rx_from_timer.recv()).await {
            Ok(Some(Event::Timeout)) => {},
            _ => panic!("Se esperaba Timeout a pesar del evento inválido previo"),
        }
    }

    #[tokio::test]
    async fn test_watchdog_timer_reinicio_rapido() {
        let (tx_to_fsm, mut rx_from_timer) = mpsc::channel(10);
        let (tx_cmd, rx_cmd) = mpsc::channel(10);

        tokio::spawn(async move {
            let token = CancellationToken::new();
            fsm_watchdog_timer(tx_to_fsm, rx_cmd, token).await;
        });

        // Iniciar timer
        tx_cmd.send(Event::InitTimer(Duration::from_millis(100))).await.unwrap();

        // Cancelar
        sleep(Duration::from_millis(20)).await;
        tx_cmd.send(Event::StopTimer).await.unwrap();

        // Reiniciar inmediatamente con un timer más corto
        tx_cmd.send(Event::InitTimer(Duration::from_millis(50))).await.unwrap();

        // Debería recibir timeout del segundo timer
        match tokio::time::timeout(Duration::from_millis(150), rx_from_timer.recv()).await {
            Ok(Some(Event::Timeout)) => {},
            _ => panic!("Se esperaba Timeout del segundo timer"),
        }
    }

    // ===== Tests de integración de secuencias complejas =====

    #[test]
    fn test_integracion_flujo_completo_sin_fallos() {
        let mut fsm = FsmState::new();

        // 1. Start
        assert_eq!(fsm.global, StateGlobal::Start);

        // 2. Start -> BalanceMode.InitBalanceMode
        fsm = fsm.step(Event::Start).unwrap_valid().get_change_state();
        assert_eq!(fsm.global, StateGlobal::BalanceMode);
        assert_eq!(fsm.balance, Some(SubStateBalanceMode::InitBalanceMode));

        // 3. InitBalanceMode -> InHandshake
        fsm = fsm.step(Event::BalanceEpochOk).unwrap_valid().get_change_state();
        assert_eq!(fsm.balance, Some(SubStateBalanceMode::InHandshake));

        // 4. InHandshake -> Quorum.CheckQuorumIn
        fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();
        assert_eq!(fsm.quorum, Some(SubStateQuorum::CheckQuorumIn));

        // 5. CheckQuorumIn -> Phase.Alert (aprobado en primer intento)
        fsm = fsm.step(Event::ApproveQuorum).unwrap_valid().get_change_state();
        assert_eq!(fsm.phase, Some(SubStatePhase::Alert));

        // 6-8. Ciclo completo de Phases: Alert -> Data -> Monitor
        fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();
        fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();
        fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();
        assert_eq!(fsm.balance, Some(SubStateBalanceMode::OutHandshake));

        // 9. OutHandshake -> Quorum.CheckQuorumOut
        fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();
        assert_eq!(fsm.quorum, Some(SubStateQuorum::CheckQuorumOut));

        // 10. CheckQuorumOut -> Normal (éxito final)
        fsm = fsm.step(Event::ApproveQuorum).unwrap_valid().get_change_state();
        assert_eq!(fsm.global, StateGlobal::Normal);

        // 11. Normal no acepta más eventos
        let invalid = fsm.step(Event::Timeout).unwrap_invalid();
        assert!(invalid.get_invalid().contains("Normal"));
    }

    #[test]
    fn test_integracion_flujo_con_multiples_reintentos() {
        let mut fsm = FsmState::new();

        // Llegar a CheckQuorumIn
        fsm = fsm.step(Event::Start).unwrap_valid().get_change_state();
        fsm = fsm.step(Event::BalanceEpochOk).unwrap_valid().get_change_state();
        fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();

        // Intentar quorum 3 veces antes de aprobar
        for _ in 0..3 {
            fsm = fsm.step(Event::NotApproveQuorum).unwrap_valid().get_change_state();
            assert_eq!(fsm.quorum, Some(SubStateQuorum::RepeatHandshakeIn));

            fsm = fsm.step(Event::Timeout).unwrap_valid().get_change_state();
            assert_eq!(fsm.quorum, Some(SubStateQuorum::CheckQuorumIn));
        }

        // Finalmente aprobar
        fsm = fsm.step(Event::ApproveQuorum).unwrap_valid().get_change_state();
        assert_eq!(fsm.phase, Some(SubStatePhase::Alert));
    }

    #[test]
    fn test_integracion_todos_los_caminos_a_safe_mode() {
        // Camino 1: Desde InitBalanceMode
        let mut fsm1 = FsmState::new();
        fsm1 = fsm1.step(Event::Start).unwrap_valid().get_change_state();
        fsm1 = fsm1.step(Event::BalanceEpochNotOk).unwrap_valid().get_change_state();
        assert_eq!(fsm1.global, StateGlobal::SafeMode);

        // Camino 2: Desde CheckQuorumIn
        let mut fsm2 = FsmState::new();
        fsm2.global = StateGlobal::BalanceMode;
        fsm2.balance = Some(SubStateBalanceMode::Quorum);
        fsm2.quorum = Some(SubStateQuorum::CheckQuorumIn);
        fsm2 = fsm2.step(Event::NotApproveNotAttempts).unwrap_valid().get_change_state();
        assert_eq!(fsm2.global, StateGlobal::SafeMode);

        // Camino 3: Desde CheckQuorumOut
        let mut fsm3 = FsmState::new();
        fsm3.global = StateGlobal::BalanceMode;
        fsm3.balance = Some(SubStateBalanceMode::Quorum);
        fsm3.quorum = Some(SubStateQuorum::CheckQuorumOut);
        fsm3 = fsm3.step(Event::NotApproveNotAttempts).unwrap_valid().get_change_state();
        assert_eq!(fsm3.global, StateGlobal::SafeMode);
    }
}