//! Módulo de Lógica del Heartbeat.
//!
//! Este módulo implementa el mecanismo de "latido" (heartbeat) para supervisar
//! la conexión con el servidor central. Utiliza una arquitectura basada en **FSM**
//! desacoplada:
//!
//! 1.  **`run_fsm_heartbeat`**: Gestiona la lógica pura y el cambio de estados.
//! 2.  **`heartbeat`**: Gestiona los efectos secundarios (timers, I/O, notificaciones al sistema).
//!
//! El objetivo es detectar caídas de conexión cuando el servidor deja de enviar
//! mensajes periódicos y notificar al resto del sistema vía un canal `watch`.


use tokio::sync::{mpsc, watch};
use tracing::{error, warn};
use crate::heartbeat::domain::{Action, Event, FsmHeartbeat, State, Status, Transition};
use crate::message::domain::Message;
use crate::config::heartbeat::*;
use crate::system::domain::InternalEvent;


/// Tarea principal de orquestación del Heartbeat.
///
/// Esta función actúa como el "Actor" que ejecuta las decisiones tomadas por la FSM.
/// Escucha mensajes del servidor y acciones de la FSM para ejecutar efectos secundarios
/// como iniciar timers o notificar cambios de estado al sistema global.
///
/// # Canales
///
/// * `tx`: Canal **Watch** para distribuir el estado de conexión (`ServerConnected`/`ServerDisconnected`) a múltiples consumidores.
/// * `tx_to_fsm`: Canal para enviar eventos entrantes hacia la lógica de la FSM.
/// * `tx_to_timer`: Canal para controlar el temporizador de seguridad (Watchdog).
/// * `rx_from_server`: Canal por donde llegan los mensajes decodificados gRPC.
/// * `rx_fsm`: Canal por donde la FSM envía las instrucciones (Acciones) que deben ejecutarse.
pub async fn heartbeat(tx: watch::Sender<InternalEvent>,
                       tx_to_fsm: mpsc::Sender<Event>,
                       tx_to_timer: mpsc::Sender<Event>,
                       mut rx_from_server: mpsc::Receiver<Message>,
                       mut rx_fsm: mpsc::Receiver<Vec<Action>>) {

    loop {
        tokio::select! {
            Some(msg) = rx_from_server.recv() => {
                match msg {
                    Message::Heartbeat(_) => {
                        if tx_to_fsm.send(Event::Heartbeat).await.is_err() {
                            error!("Error: No se pudo enviar evento Heartbeat a la FSM heartbeat");
                        }
                    },
                    _ => {}
                }
            }

            Some(vec_action) = rx_fsm.recv() => {
                for action in vec_action {
                    match action {
                        Action::OnEntry(state) => {
                            match state {
                                State::StartingWait | State::NotHeartbeatYet | State::ItsAlive => {
                                    if tx_to_timer.send(Event::InitTimer(HEARTBEAT_TIMEOUT)).await.is_err() {
                                        error!("Error: No se pudo enviar evento de inicio de timer al watchdog del heartbeat");
                                    }
                                },
                                _ => {}
                            }
                        }
                        Action::SendStatusConditional(old_status, new_status) => {
                            if old_status != new_status {
                                if new_status == Status::Connected {
                                    if tx.send(InternalEvent::ServerConnected).is_err() {
                                        error!("Error: No se pudo distribuir el estado de ServerConnected");
                                    }
                                }
                                if new_status == Status::Disconnected {
                                    if tx.send(InternalEvent::ServerDisconnected).is_err() {
                                        error!("Error: No se pudo distribuir el estado de ServerDisconnected");
                                    }
                                }
                            }
                        }
                        Action::StopTimer => {
                            if tx_to_timer.send(Event::StopTimer).await.is_err() {
                                error!("Error: No se pudo parar el timer watchdog del heartbeat");
                            }
                        },
                        _ => {},
                    }
                }
            }
        }
    }
}


/// Tarea que ejecuta la lógica pura de la FSM del Heartbeat.
///
/// Mantiene el estado interno (`FsmHeartbeat`) y procesa eventos secuencialmente.
/// No tiene efectos secundarios directos; delega la ejecución a `heartbeat` mediante `tx_actions`.
///
/// # Argumentos
///
/// * `tx_actions`: Canal para enviar las acciones resultantes (Side Effects) al orquestador.
/// * `rx_from_server`: Canal de entrada de eventos (Heartbeats recibidos, Timeouts del timer).
pub async fn run_fsm_heartbeat(tx_actions: mpsc::Sender<Vec<Action>>,
                               mut rx_from_heartbeat: mpsc::Receiver<Event>) {

    let mut state = FsmHeartbeat::new();

    while let Some(event) = rx_from_heartbeat.recv().await {
        let transition = state.step(event);

        match transition {
            Transition::Valid(valid) => {
                state = valid.get_change_state();
                if tx_actions.send(valid.get_actions()).await.is_err() {
                    error!("Error: No se pudo enviar la acción a heartbeat");
                }
            },
            Transition::Invalid(invalid) => {
                warn!("Warning: FSM Heartbeat transición inválida: {}", invalid.get_invalid());
            }
        }
    }
}