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

use crate::context::domain::AppContext;
use crate::firmware::logic::{update_firmware_task};
use crate::message::domain::{FirmwareOk, HubMessage, ServerMessage, UpdateFirmware};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, sleep};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument};

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

pub enum Event {
    /// Comando interno para iniciar el timer.
    InitTimer(Duration),
    /// Comando interno para detener el timer.
    StopTimer,
    Timeout
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
    pub fn new(
        sender: mpsc::Sender<FirmwareServiceResponse>,
        receiver: mpsc::Receiver<FirmwareServiceCommand>,
        context: AppContext,
    ) -> Self {
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
        let (tx_to_timer, rx_from_update_task) = mpsc::channel::<Event>(50);
        let (tx_to_update, rx_from_timer) = mpsc::channel::<Event>(50);
        let (tx_msg, rx_msg) = mpsc::channel::<FirmwareServiceCommand>(50);

        let child_token = token.child_token();
        handles.push(tokio::spawn(update_firmware_task(
            tx_response,
            tx_to_timer,
            rx_msg,
            rx_from_timer,
            self.context.clone(),
            child_token,
        )));

        let child_token = token.child_token();
        handles.push(tokio::spawn(firmware_watchdog_timer(
            tx_to_update,
            rx_from_update_task,
            child_token,
        )));

        FirmwareRuntime {
            handles,
            cancel_token: token,
            tx_msg,
            rx_response,
        }
    }

    pub async fn run(mut self, shutdown: CancellationToken) {
        let mut runtime: Option<FirmwareRuntime> = None;

        loop {
            match runtime {
                Some(ref mut rt) => {
                    tokio::select! {
                        _ = shutdown.cancelled() => {
                            info!("shutdown recibido FirmwareService");
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
                                        error!("no se pudo enviar comando Update a update_firmware_task");
                                    }
                                },
                                FirmwareServiceCommand::HubResponse(response) => {
                                    if rt.tx_msg.send(FirmwareServiceCommand::HubResponse(response)).await.is_err() {
                                        error!("no se pudo enviar mensaje HubResponse a update_firmware_task");
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
                                        error!("no se pudo enviar respuesta HubCommand al Core");
                                    }
                                },
                                FirmwareServiceResponse::ServerAck(response) => {
                                    if self.sender.send(FirmwareServiceResponse::ServerAck(response)).await.is_err() {
                                        error!("no se pudo enviar respuesta ServerAck al Core");
                                    }
                                },
                            }
                        }
                    }
                }
                None => {
                    tokio::select! {
                        _ = shutdown.cancelled() => {
                            info!("shutdown recibido FirmwareService");
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



/// Tarea asíncrona dedicada al temporizador de seguridad (Watchdog).
///
/// Implementa un patrón "Dead Man's Switch". Espera un comando `InitTimer`.
/// Si el tiempo expira antes de recibir `StopTimer`, envía un evento `Timeout` a la FSM.
#[instrument(name = "firmware_watchdog_timer", skip(cmd_rx))]
async fn firmware_watchdog_timer(
    tx_to_fsm: mpsc::Sender<Event>,
    mut cmd_rx: mpsc::Receiver<Event>,
    cancel: CancellationToken,
) {
    loop {
        // Estado IDLE: Esperar comando de inicio
        let duration = match cmd_rx.recv().await {
            Some(Event::InitTimer(d)) => d,
            Some(Event::StopTimer) => continue, // Si ya estaba parado, ignorar
            None => break,                      // Canal cerrado, terminar tarea
            _ => continue,
        };

        // Estado ACTIVO: Corriendo temporizador
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("shutdown recibido firmware_watchdog_timer");
                break;
            }
            _ = sleep(duration) => {
                // El tiempo se agotó
                if tx_to_fsm.send(Event::Timeout).await.is_err() {
                    error!("no se pudo enviar evento Timeout");
                }
            }
            Some(Event::StopTimer) = cmd_rx.recv() => {
                debug!("watchdog timer de fsm firmware, cancelado");
            }
        }
    }
}