use chrono::Utc;
use tokio::time::{sleep, Duration};
use tokio::sync::mpsc;
use tracing::{error};
use crate::context::domain::AppContext;
use crate::message::domain::{Message, Metadata};
use crate::metrics::domain::MetricsCollector;


pub enum MetricsTimerEvent {
    InitTimer(Duration),
    Timeout,
}


pub async fn system_metrics(tx_to_server: mpsc::Sender<Message>, 
                            tx_to_timer: mpsc::Sender<MetricsTimerEvent>, 
                            mut rx_from_timer: mpsc::Receiver<MetricsTimerEvent>, 
                            app_context: AppContext) {

    let mut metrics = MetricsCollector::new();

    if tx_to_timer.send(MetricsTimerEvent::InitTimer(Duration::from_secs(180))).await.is_err() {
        error!("Error: No se pudo enviar evento InitTimer al timer de métricas");
        return;
    }

    while let Some(msg) = rx_from_timer.recv().await {
        match msg {
            MetricsTimerEvent::Timeout => {
                let metadata = Metadata {
                    sender_user_id: app_context.system.id_edge.clone(),
                    destination_id: "Server0".to_string(),
                    timestamp: Utc::now().timestamp(),
                };
                let sys_met = metrics.collect(metadata);
                let msg = Message::Metrics(sys_met);
                if tx_to_server.send(msg).await.is_err() {
                    error!("Error: No se pudo enviar mensaje de métricas al servidor");
                }
                if tx_to_timer.send(MetricsTimerEvent::InitTimer(Duration::from_secs(180))).await.is_err() {
                    error!("Error: No se pudo enviar evento InitTimer al timer de métricas");
                }
            },
            _ => {}
        }
    }
}


/// Tarea asíncrona dedicada al temporizador de envío de metricas.
///
/// Implementa un patrón "Dead Man's Switch". Espera un comando `InitTimer`.
/// Si el tiempo expira antes de recibir `StopTimer`, envía un evento `Timeout` a la tarea.
pub async fn metrics_timer(tx_to_metrics: mpsc::Sender<MetricsTimerEvent>,
                           mut cmd_rx: mpsc::Receiver<MetricsTimerEvent>) {
    loop {
        let duration = match cmd_rx.recv().await {
            Some(MetricsTimerEvent::InitTimer(d)) => d,
            None => break, // Canal cerrado, terminar tarea
            _ => continue,
        };

        // Estado ACTIVO: Corriendo temporizador
        tokio::select! {
            _ = sleep(duration) => {
                // El tiempo se agotó
                let _ = tx_to_metrics.send(MetricsTimerEvent::Timeout).await;
            }
        }
    }
}