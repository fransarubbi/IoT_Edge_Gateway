//! # Lógica de Recolección y Envío de Métricas
//!
//! Este módulo contiene la lógica central para el monitoreo del sistema. Su responsabilidad principal
//! es orquestar la recolección de datos de hardware (CPU, RAM, Disco, Red) y enviarlos al servidor
//! central en intervalos regulares.
//!
//! ## Arquitectura
//! El módulo funciona mediante la cooperación de dos tareas asíncronas principales:
//! 1.  **`system_metrics`**: El orquestador principal. Mantiene el estado del colector,
//!     construye los mensajes de protocolo y gestiona el ciclo de vida del reporte.
//! 2.  **`metrics_timer`**: Un temporizador dedicado que actúa como "despertador".
//!


use chrono::Utc;
use tokio::time::{sleep, Duration};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument};
use crate::context::domain::AppContext;
use crate::message::domain::{Metadata, ServerMessage};
use crate::metrics::domain::MetricsCollector;


/// Eventos de control para la coordinación del temporizador de métricas.
///
/// Este enumerado define el protocolo de comunicación entre la tarea lógica (`system_metrics`)
/// y la tarea de temporización (`metrics_timer`).
pub enum MetricsTimerEvent {
    /// Instrucción para iniciar una espera de la duración especificada.
    InitTimer(Duration),
    /// Evento emitido cuando el tiempo de espera ha concluido.
    Timeout,
}


/// Orquestador principal de métricas del sistema.
///
/// Esta función inicializa el colector de métricas y entra en un bucle infinito
/// impulsado por eventos del temporizador. Cada vez que el temporizador expira,
/// esta tarea recolecta las métricas actuales, las empaqueta y las envía al servidor via `tx_to_server`.
///
/// # Argumentos
///
/// * `tx_to_server` - Canal para enviar los mensajes `ServerMessage::Metrics`.
/// * `tx_to_timer` - Canal para enviar comandos de reinicio (`InitTimer`) a la tarea del temporizador.
/// * `rx_from_timer` - Canal para recibir notificaciones de `Timeout` cuando es hora de recolectar métricas.
/// * `app_context` - Contexto global de la aplicación, utilizado para obtener IDs de dispositivo y configuración.
///
#[instrument(name = "system_metrics", skip_all)]
pub async fn system_metrics(tx_to_server: mpsc::Sender<ServerMessage>, 
                            tx_to_timer: mpsc::Sender<MetricsTimerEvent>, 
                            mut rx_from_timer: mpsc::Receiver<MetricsTimerEvent>, 
                            app_context: AppContext,
                            shutdown: CancellationToken) {

    let mut metrics = MetricsCollector::new();

    if tx_to_timer.send(MetricsTimerEvent::InitTimer(Duration::from_secs(180))).await.is_err() {
        error!("no se pudo enviar evento InitTimer a metrics_timer");
        return;
    }

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("shutdown recibido system_metrics");
                break;
            }

            Some(msg) = rx_from_timer.recv() => {
                match msg {
                    MetricsTimerEvent::Timeout => {
                        let metadata = Metadata {
                            sender_user_id: app_context.system.id_edge.clone(),
                            destination_id: "server0".to_string(),
                            timestamp: Utc::now().timestamp(),
                        };
                        let sys_met = metrics.collect(metadata);
                        let msg = ServerMessage::Metrics(sys_met);
                        if tx_to_server.send(msg).await.is_err() {
                            error!("no se pudo enviar mensaje de métricas a MetricsService");
                        }
                        if tx_to_timer.send(MetricsTimerEvent::InitTimer(Duration::from_secs(180))).await.is_err() {
                            error!("no se pudo enviar evento InitTimer a metrics_timer");
                        }
                    },
                    _ => {}
                }
            }
        }
    }
}


/// Tarea asíncrona dedicada al temporizador (Ticker).
///
/// Funciona como un bucle simple que espera una instrucción de duración,
/// duerme el hilo asíncrono durante ese tiempo, y luego notifica que el tiempo ha pasado.
///
/// # Comportamiento
/// 1. **Espera pasiva:** Se bloquea en `cmd_rx.recv()` hasta recibir una duración.
/// 2. **Sleep:** Ejecuta `tokio::time::sleep` por la duración recibida.
/// 3. **Notificación:** Envía `MetricsTimerEvent::Timeout` de vuelta al controlador.
///
/// # Argumentos
///
/// * `tx_to_metrics` - Canal para notificar el timeout al orquestador (`system_metrics`).
/// * `cmd_rx` - Canal por donde recibe las instrucciones `InitTimer(Duration)`.
///
#[instrument(name = "metrics_timer", skip_all)]
pub async fn metrics_timer(tx_to_metrics: mpsc::Sender<MetricsTimerEvent>,
                           mut cmd_rx: mpsc::Receiver<MetricsTimerEvent>,
                           shutdown: CancellationToken) {

    loop {
        let duration = match cmd_rx.recv().await {
            Some(MetricsTimerEvent::InitTimer(d)) => d,
            None => break, // Canal cerrado, terminar tarea
            _ => continue,
        };

        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("shutdown recibido metrics_timer");
                break;
            }
            _ = sleep(duration) => {
                if tx_to_metrics.send(MetricsTimerEvent::Timeout).await.is_err() {
                    error!("no se pudo enviar evento Timeout en metrics_timer");
                }
            }
        }
    }
}