//! # Módulo de Dominio de Firmware (FSM)
//!
//! Este módulo implementa la lógica de negocio pura para el proceso de actualización
//! OTA (Over-The-Air) utilizando una Máquina de Estados Finitos (FSM).
//!
//! ## Responsabilidades
//! * Definir los estados válidos del proceso de actualización.
//! * Calcular transiciones deterministas basadas en eventos.
//! * Generar acciones (efectos secundarios) que el orquestador debe ejecutar.
//! * Gestionar la sesión de actualización (métricas y progreso).


use std::cmp::PartialEq;
use std::collections::HashSet;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument};
use crate::context::domain::AppContext;
use crate::firmware::logic::{run_fsm_firmware, update_firmware_task};
use crate::message::domain::{FirmwareOk, HubMessage, ServerMessage, UpdateFirmware};


pub enum FirmwareServiceCommand {
    Update(UpdateFirmware),
    HubResponse(FirmwareOk),
    CreateRuntime,
    DeleteRuntime,
}


pub enum FirmwareServiceResponse {
    ServerAck(ServerMessage),
    HubCommand(HubMessage),
}


pub struct FirmwareService {
    sender: mpsc::Sender<FirmwareServiceResponse>,
    receiver: mpsc::Receiver<FirmwareServiceCommand>,
    context: AppContext,
}


struct FirmwareRuntime {
    handles: Vec<JoinHandle<()>>,
    cancel_token: CancellationToken,
    tx_msg: mpsc::Sender<FirmwareServiceCommand>,
    rx_response: mpsc::Receiver<FirmwareServiceResponse>,
}


impl FirmwareService {
    pub fn new(sender: mpsc::Sender<FirmwareServiceResponse>,
               receiver: mpsc::Receiver<FirmwareServiceCommand>,
               context: AppContext) -> Self {
        Self {
            sender,
            receiver,
            context,
        }
    }

    fn spawn_runtime(&self) -> FirmwareRuntime {

        let token = CancellationToken::new();
        let mut handles = Vec::new();

        let (tx_response, rx_response) = mpsc::channel::<FirmwareServiceResponse>(50);
        let (tx_to_fsm, fsm_rx_from_update_task) = mpsc::channel::<Event>(50);
        let (tx_to_timer, timer_rx_from_update_task) = mpsc::channel::<Event>(50);
        let (tx_msg, rx_msg) = mpsc::channel::<FirmwareServiceCommand>(50);
        let (tx_to_update_task, rx_from_fsm) = mpsc::channel::<Vec<Action>>(50);

        let child_token = token.child_token();
        let update_tx_to_fsm = tx_to_fsm.clone();
        handles.push(tokio::spawn(update_firmware_task(
                                          tx_response,
                                          update_tx_to_fsm,
                                          tx_to_timer,
                                          rx_msg,
                                          rx_from_fsm,
                                          self.context.clone(),
                                          child_token)));

        let child_token = token.child_token();
        handles.push(tokio::spawn(run_fsm_firmware(
                                         tx_to_update_task,
                                         fsm_rx_from_update_task,
                                         child_token)));

        let child_token = token.child_token();
        let timer_tx_to_fsm = tx_to_fsm.clone();
        handles.push(tokio::spawn(firmware_watchdog_timer(
                                            timer_tx_to_fsm,
                                            timer_rx_from_update_task,
                                            child_token)));

        FirmwareRuntime {
            handles,
            cancel_token: token,
            tx_msg,
            rx_response
        }
    }

    pub async fn run(mut self, shutdown: CancellationToken) {
        
        let mut runtime: Option<FirmwareRuntime> = None;
        
        loop {
            match runtime {
                Some(ref mut rt) => {
                    tokio::select! {
                        _ = shutdown.cancelled() => {
                            info!("Info: shutdown recibido FirmwareService");
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
                                FirmwareServiceCommand::Update(update) => {
                                    if rt.tx_msg.send(FirmwareServiceCommand::Update(update)).await.is_err() {
                                        error!("Error: no se pudo enviar comando Update a update_firmware_task");
                                    }
                                },
                                FirmwareServiceCommand::HubResponse(response) => {
                                    if rt.tx_msg.send(FirmwareServiceCommand::HubResponse(response)).await.is_err() {
                                        error!("Error: no se pudo enviar mensaje HubResponse a update_firmware_task");
                                    }
                                },
                                FirmwareServiceCommand::DeleteRuntime => {
                                    if let Some(rt) = runtime.take() {
                                        rt.cancel_token.cancel();
                                        for h in rt.handles {
                                            let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
                                        }
                                    }
                                },
                                _ => {}
                            }
                        }
                        Some(response) = rt.rx_response.recv() => {
                            match response {
                                FirmwareServiceResponse::HubCommand(response) => {
                                    if self.sender.send(FirmwareServiceResponse::HubCommand(response)).await.is_err() {
                                        error!("Error: no se pudo enviar respuesta HubCommand al Core");
                                    }
                                },
                                FirmwareServiceResponse::ServerAck(response) => {
                                    if self.sender.send(FirmwareServiceResponse::ServerAck(response)).await.is_err() {
                                        error!("Error: no se pudo enviar respuesta ServerAck al Core");
                                    }
                                },
                            }
                        }
                    }
                }
                None => {
                    tokio::select! {
                        _ = shutdown.cancelled() => {
                            info!("Info: shutdown recibido FirmwareService");
                            break;
                        }
                        
                        Some(cmd) = self.receiver.recv() => {
                            match cmd {
                                FirmwareServiceCommand::CreateRuntime => {
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


/// Envoltorio principal para el estado de la FSM.
#[derive(Debug, Clone)]
pub struct FsmStateFirmware {
    /// Estado actual del proceso.
    state: StateFirmwareUpdate,
}


/// Enumeración de todos los estados posibles en el ciclo de vida de la actualización OTA.
#[derive(Debug, Clone, PartialEq)]
pub enum StateFirmwareUpdate {
    /// Estado inicial/reposo. Esperando mensaje del servidor.
    WaitingForFirmware,
    /// Fase Canario: Se ha notificado a un Hub piloto y se espera su respuesta.
    NotifyHub,
    /// Fase Broadcast: Se ha notificado a toda la red y se recolectan resultados.
    WaitingNetworkAnswer,
}


/// Resultado de intentar aplicar un evento a la FSM.
pub enum Transition {
    /// La transición es lógica y permitida.
    Valid(TransitionValid),
    /// El evento no aplica al estado actual.
    Invalid(TransitionInvalid),
}


/// Representa un cambio de estado exitoso y las acciones resultantes.
#[derive(Debug)]
pub struct TransitionValid {
    /// El nuevo estado de la máquina.
    change_state: FsmStateFirmware,
    /// Lista ordenada de acciones que el ejecutor debe realizar.
    actions: Vec<Action>,
}


impl TransitionValid {
    pub fn get_change_state(&self) -> FsmStateFirmware {
        self.change_state.clone()
    }
    pub fn get_actions(&self) -> Vec<Action> {
        self.actions.clone()
    }
}


/// Representa un intento fallido de transición.
#[derive(Debug)]
pub struct TransitionInvalid {
    /// Razón del fallo.
    invalid: String,
}


impl TransitionInvalid {
    pub fn get_invalid(&self) -> &str {
        &self.invalid
    }
}


/// Eventos de entrada (Inputs) que estimulan la FSM.
pub enum Event {
    /// El servidor envió una orden de actualización.
    MessageFromServer,
    /// Un Hub respondió exitosamente ("OK").
    MessageFromHub,
    /// El Watchdog Timer expiró.
    Timeout,
    /// Se recibieron todas las respuestas esperadas en la fase Broadcast.
    AllMessageReceived,
    /// Ocurrió un error (fallo de hub, error de red, etc.).
    Error,
    /// Comando interno para iniciar el timer.
    InitTimer(Duration),
    /// Comando interno para detener el timer.
    StopTimer,
}


/// Acciones de salida (Outputs) o instrucciones para el ejecutor.
#[derive(Debug, PartialEq, Clone)]
pub enum Action {
    /// Acción interna: inicializar recursos al entrar a un estado (ej. Timers).
    OnEntry(StateFirmwareUpdate),
    /// Instrucción: Enviar reporte de ÉXITO al servidor.
    SendMessageOkToServer,
    /// Instrucción: Enviar reporte de FALLO al servidor.
    SendMessageFailToServer,
    /// Instrucción: Detener el Watchdog Timer.
    StopTimer,
    /// Instrucción: Buscar un hub aleatorio y enviarle el firmware (Canario).
    SelectRandomHub,
    /// Instrucción: Enviar firmware a toda la red (Broadcast).
    SendMessageToNetwork,
    /// No hacer nada.
    Nothing,
}


/// Define la estrategia de despliegue actual dentro de una sesión.
#[derive(Debug, PartialEq)]
pub enum Phase {
    /// Prueba piloto en un solo dispositivo.
    Canary,
    /// Despliegue masivo al resto de la red.
    Broadcast,
}


/// Mantiene el contexto volátil y las métricas de una actualización en curso.
pub struct UpdateSession {
    /// Mensaje original del servidor.
    pub message: ServerMessage,
    /// ID de la red a la que se le aplica la actualización
    pub network: String,
    /// Cantidad total de hubs esperados en la red.
    pub total_hubs: usize,
    /// Set de IDs que ya han respondido (para garantizar idempotencia).
    pub responses: HashSet<String>,
    /// Contador de éxitos.
    pub successes: usize,
    /// Contador de fallos.
    pub failures: usize,
    /// Versión del firmware reportada por los hubs.
    pub version: String,
    /// Porcentaje de éxito calculado.
    pub percentage: f32,
    /// Fase actual del despliegue.
    pub phase: Phase,
}


impl UpdateSession {
    /// Crea una nueva sesión de actualización en fase Canario.
    pub fn new(message: ServerMessage, total_hubs: usize, network: String) -> Self {
        Self {
            message,
            total_hubs,
            network,
            responses: HashSet::new(),
            successes: 0,
            failures: 0,
            version: String::new(),
            percentage: 0.0,
            phase: Phase::Canary,
        }
    }

    /// Verifica si ya se recibieron respuestas de todos los hubs esperados.
    pub fn is_complete(&self) -> bool {
        self.successes + self.failures == self.total_hubs
    }

    /// Retorna true si estamos en fase de prueba piloto.
    pub fn is_canary(&self) -> bool {
        self.phase == Phase::Canary
    }

    /// Retorna true si estamos en fase de despliegue masivo.
    pub fn is_broadcast(&self) -> bool {
        self.phase == Phase::Broadcast
    }

    /// Calcula el porcentaje de éxito (0.00 a 100.00).
    pub fn success_rate(&self) -> f32 {
        if self.total_hubs == 0 {
            return 0.0;
        }
        (self.successes as f32 / self.total_hubs as f32) * 100.00
    }
}


impl FsmStateFirmware {

    /// Inicializa la máquina en estado de espera.
    pub fn new() -> Self {
        Self {
            state: StateFirmwareUpdate::WaitingForFirmware,
        }
    }

    /// Lógica interna de transición paso a paso.
    pub fn step_inner(&self, event: Event) -> Transition {
        match (&self.state, event) {
            (StateFirmwareUpdate::WaitingForFirmware, Event::MessageFromServer) => {
                let next_fsm = self.clone();
                state_waiting_for_firmware_event_message_update(next_fsm)
            },
            (StateFirmwareUpdate::NotifyHub, Event::MessageFromHub) => {
                let next_fsm = self.clone();
                state_notify_hub_event_message_from_hub(next_fsm)
            },
            (StateFirmwareUpdate::NotifyHub, Event::Error) => {
                let next_fsm = self.clone();
                state_notify_hub_event_error(next_fsm)
            },
            (StateFirmwareUpdate::WaitingNetworkAnswer, Event::Timeout | Event::Error) => {
                let next_fsm = self.clone();
                state_waiting_network_answer_event_timeout_or_error(next_fsm)
            },
            (StateFirmwareUpdate::WaitingNetworkAnswer, Event::AllMessageReceived) => {
                let next_fsm = self.clone();
                state_waiting_network_answer_event_all_message_received(next_fsm)
            },
            _ => {
                let invalid = TransitionInvalid {
                    invalid: "Transición inválida".to_string(),
                };
                Transition::Invalid(invalid)
            },
        }
    }

    /// Función principal de transición.
    ///
    /// Calcula el siguiente estado e inyecta acciones automáticas (`OnEntry`)
    /// si ocurre un cambio de estado.
    pub fn step(&self, event: Event) -> Transition {
        let transition = self.step_inner(event);

        match transition {
            Transition::Valid(mut t) => {
                let entry_action = compute_on_entry(self, &t.change_state);
                if entry_action != Action::Nothing {
                    // Importante: Insertar al inicio para que timers se inicien antes de operaciones de red.
                    t.actions.insert(0, entry_action);
                }
                Transition::Valid(t)
            }
            invalid => invalid,
        }
    }
}


// --- Funciones auxiliares de transición de estado ---

fn state_waiting_for_firmware_event_message_update(mut next_fsm: FsmStateFirmware) -> Transition {
    next_fsm.state = StateFirmwareUpdate::NotifyHub;
    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![Action::SelectRandomHub],
    };
    Transition::Valid(valid)
}


fn state_notify_hub_event_message_from_hub(mut next_fsm: FsmStateFirmware) -> Transition {
    next_fsm.state = StateFirmwareUpdate::WaitingNetworkAnswer;
    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![Action::StopTimer, Action::SendMessageToNetwork],
    };
    Transition::Valid(valid)
}


fn state_notify_hub_event_error(mut next_fsm: FsmStateFirmware) -> Transition {
    next_fsm.state = StateFirmwareUpdate::WaitingForFirmware;
    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![Action::SendMessageFailToServer],
    };
    Transition::Valid(valid)
}


fn state_waiting_network_answer_event_timeout_or_error(mut next_fsm: FsmStateFirmware) -> Transition {
    next_fsm.state = StateFirmwareUpdate::WaitingForFirmware;
    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![Action::SendMessageFailToServer],
    };
    Transition::Valid(valid)
}


fn state_waiting_network_answer_event_all_message_received(mut next_fsm: FsmStateFirmware) -> Transition {
    next_fsm.state = StateFirmwareUpdate::WaitingForFirmware;
    let valid = TransitionValid {
        change_state: next_fsm,
        actions: vec![Action::StopTimer, Action::SendMessageOkToServer],
    };
    Transition::Valid(valid)
}


/// Calcula acciones automáticas al entrar en un nuevo estado.
/// Usado principalmente para inicializar timers (Watchdog).
fn compute_on_entry(old: &FsmStateFirmware, new: &FsmStateFirmware) -> Action {
    if old.state != new.state {
        let state = new.state.clone();
        return Action::OnEntry(state);
    }
    Action::Nothing
}


/// Tarea asíncrona dedicada al temporizador de seguridad (Watchdog).
///
/// Implementa un patrón "Dead Man's Switch". Espera un comando `InitTimer`.
/// Si el tiempo expira antes de recibir `StopTimer`, envía un evento `Timeout` a la FSM.
#[instrument(name = "firmware_watchdog_timer", skip(cmd_rx))]
async fn firmware_watchdog_timer(tx_to_fsm: mpsc::Sender<Event>,
                                mut cmd_rx: mpsc::Receiver<Event>,
                                cancel: CancellationToken) {
    loop {
        // Estado IDLE: Esperar comando de inicio
        let duration = match cmd_rx.recv().await {
            Some(Event::InitTimer(d)) => d,
            Some(Event::StopTimer) => continue, // Si ya estaba parado, ignorar
            None => break, // Canal cerrado, terminar tarea
            _ => continue,
        };

        // Estado ACTIVO: Corriendo temporizador
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("Info: shutdown recibido firmware_watchdog_timer");
                break;
            }
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
    use chrono::Utc;
    use super::*;

    // ============================================================
    //                   TRANSICIONES VÁLIDAS
    // ============================================================

    #[test]
    fn test_fsm_initial_state() {
        let fsm = FsmStateFirmware::new();
        assert_eq!(fsm.state, StateFirmwareUpdate::WaitingForFirmware);
    }

    #[test]
    fn test_waiting_to_notify_hub() {
        let fsm = FsmStateFirmware::new();
        let transition = fsm.step(Event::MessageFromServer);

        match transition {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.state, StateFirmwareUpdate::NotifyHub);
                assert!(t.actions.contains(&Action::SelectRandomHub));
                assert!(t.actions.contains(&Action::OnEntry(StateFirmwareUpdate::NotifyHub)));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_notify_hub_success_to_waiting_network() {
        let mut fsm = FsmStateFirmware::new();
        fsm.state = StateFirmwareUpdate::NotifyHub;

        let transition = fsm.step(Event::MessageFromHub);

        match transition {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.state, StateFirmwareUpdate::WaitingNetworkAnswer);
                assert!(t.actions.contains(&Action::SendMessageToNetwork));
                assert!(t.actions.contains(&Action::StopTimer));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_notify_hub_error_to_failure() {
        let mut fsm = FsmStateFirmware::new();
        fsm.state = StateFirmwareUpdate::NotifyHub;

        let transition = fsm.step(Event::Error);

        match transition {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.state, StateFirmwareUpdate::WaitingForFirmware);
                assert!(t.actions.contains(&Action::SendMessageFailToServer));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_waiting_network_timeout_to_failure() {
        let mut fsm = FsmStateFirmware::new();
        fsm.state = StateFirmwareUpdate::WaitingNetworkAnswer;

        let transition = fsm.step(Event::Timeout);

        match transition {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.state, StateFirmwareUpdate::WaitingForFirmware);
                assert!(t.actions.contains(&Action::SendMessageFailToServer));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_waiting_network_error_to_failure() {
        let mut fsm = FsmStateFirmware::new();
        fsm.state = StateFirmwareUpdate::WaitingNetworkAnswer;

        let transition = fsm.step(Event::Error);

        match transition {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.state, StateFirmwareUpdate::WaitingForFirmware);
                assert!(t.actions.contains(&Action::SendMessageFailToServer));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    #[test]
    fn test_waiting_network_all_received_to_success() {
        let mut fsm = FsmStateFirmware::new();
        fsm.state = StateFirmwareUpdate::WaitingNetworkAnswer;

        let transition = fsm.step(Event::AllMessageReceived);

        match transition {
            Transition::Valid(t) => {
                assert_eq!(t.change_state.state, StateFirmwareUpdate::WaitingForFirmware);
                assert!(t.actions.contains(&Action::StopTimer));
                assert!(t.actions.contains(&Action::SendMessageOkToServer));
            }
            Transition::Invalid(_) => panic!("Se esperaba una transición válida"),
        }
    }

    // ============================================================
    // TESTS DEL FSM - TRANSICIONES INVÁLIDAS
    // ============================================================

    #[test]
    fn test_invalid_timeout_in_waiting_state() {
        let fsm = FsmStateFirmware::new();
        let transition = fsm.step(Event::Timeout);
        assert!(matches!(transition, Transition::Invalid(_)));
    }

    #[test]
    fn test_invalid_all_received_in_notify_hub() {
        let mut fsm = FsmStateFirmware::new();
        fsm.state = StateFirmwareUpdate::NotifyHub;

        let transition = fsm.step(Event::AllMessageReceived);
        assert!(matches!(transition, Transition::Invalid(_)));
    }

    // ============================================================
    // TESTS DE ONENTRY
    // ============================================================

    #[test]
    fn test_on_entry_action_when_state_changes() {
        let old_fsm = FsmStateFirmware::new();
        let mut new_fsm = old_fsm.clone();
        new_fsm.state = StateFirmwareUpdate::NotifyHub;

        let action = compute_on_entry(&old_fsm, &new_fsm);
        assert_eq!(action, Action::OnEntry(StateFirmwareUpdate::NotifyHub));
    }

    #[test]
    fn test_no_on_entry_when_state_unchanged() {
        let old_fsm = FsmStateFirmware::new();
        let new_fsm = old_fsm.clone();

        let action = compute_on_entry(&old_fsm, &new_fsm);
        assert_eq!(action, Action::Nothing);
    }

    #[test]
    fn test_on_entry_inserted_at_beginning() {
        let fsm = FsmStateFirmware::new();
        let transition = fsm.step(Event::MessageFromServer);

        match transition {
            Transition::Valid(t) => {
                // OnEntry debe estar al principio
                assert_eq!(t.actions[0], Action::OnEntry(StateFirmwareUpdate::NotifyHub));
            }
            _ => panic!("Se esperaba una transición válida"),
        }
    }

    // ============================================================
    // TESTS DE UPDATESESSION
    // ============================================================

    // Helper para crear mensajes dummy
    fn create_test_message() -> ServerMessage {
        use crate::message::domain::*;
        let timestamp = Utc::now().timestamp();

        let metadata = Metadata { 
            sender_user_id: "server".to_string(),
            destination_id: "hub1".to_string(),
            timestamp,
        };

        let message = ServerMessage::UpdateFirmware(
            UpdateFirmware {
                metadata,
                network: "sala8".to_string(),
            }
        );

        message
    }

    #[test]
    fn test_session_starts_in_canary_phase() {
        let msg = create_test_message();
        let session = UpdateSession::new(msg, 10, "sala8".to_string());

        assert_eq!(session.phase, Phase::Canary);
        assert!(session.is_canary());
        assert!(!session.is_broadcast());
    }

    #[test]
    fn test_session_initialized_with_zeros() {
        let msg = create_test_message();
        let session = UpdateSession::new(msg, 10, "sala8".to_string());

        assert_eq!(session.successes, 0);
        assert_eq!(session.failures, 0);
        assert_eq!(session.responses.len(), 0);
        assert_eq!(session.percentage, 0.0);
        assert_eq!(session.version, "");
    }

    #[test]
    fn test_is_complete_false_initially() {
        let msg = create_test_message();
        let session = UpdateSession::new(msg, 10, "sala8".to_string());
        assert!(!session.is_complete());
    }

    #[test]
    fn test_is_complete_true_when_all_responded() {
        let msg = create_test_message();
        let mut session = UpdateSession::new(msg, 10, "sala8".to_string());

        session.successes = 7;
        session.failures = 3;

        assert!(session.is_complete());
    }

    #[test]
    fn test_is_complete_false_with_partial_responses() {
        let msg = create_test_message();
        let mut session = UpdateSession::new(msg, 10, "sala8".to_string());

        session.successes = 5;
        session.failures = 2;

        assert!(!session.is_complete());
    }

    #[test]
    fn test_success_rate_calculation() {
        let msg = create_test_message();
        let mut session = UpdateSession::new(msg, 10, "sala8".to_string());

        session.successes = 7;

        assert_eq!(session.success_rate(), 70.0);
    }

    #[test]
    fn test_success_rate_100_percent() {
        let msg = create_test_message();
        let mut session = UpdateSession::new(msg, 5, "sala8".to_string());

        session.successes = 5;

        assert_eq!(session.success_rate(), 100.0);
    }

    #[test]
    fn test_success_rate_0_percent() {
        let msg = create_test_message();
        let mut session = UpdateSession::new(msg, 5, "sala8".to_string());

        session.failures = 5;

        assert_eq!(session.success_rate(), 0.0);
    }

    #[test]
    fn test_success_rate_with_zero_hubs_no_panic() {
        let msg = create_test_message();
        let session = UpdateSession::new(msg, 0, "sala8".to_string());

        // No debe hacer panic por división por cero
        assert_eq!(session.success_rate(), 0.0);
    }

    #[test]
    fn test_responses_hashset_prevents_duplicates() {
        let msg = create_test_message();
        let mut session = UpdateSession::new(msg, 3, "sala8".to_string());

        assert!(session.responses.insert("hub1".to_string()));
        assert!(session.responses.insert("hub2".to_string()));
        assert!(!session.responses.insert("hub1".to_string())); // Duplicado

        assert_eq!(session.responses.len(), 2);
    }

    #[test]
    fn test_phase_transitions() {
        let msg = create_test_message();
        let mut session = UpdateSession::new(msg, 10, "sala8".to_string());

        // Comienza en Canary
        assert!(session.is_canary());

        // Cambiar a Broadcast
        session.phase = Phase::Broadcast;
        assert!(session.is_broadcast());
        assert!(!session.is_canary());
    }
}