use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};
use tonic::codec::CompressionEncoding;
use tonic::Request;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use std::fs;
use tracing::{error, info, warn};
use std::time::Duration;
use crate::context::domain::AppContext;
use crate::grpc::{EdgeDownload, EdgeUpload};
use crate::grpc::iot_service_client::IotServiceClient;
use crate::system::domain::{InternalEvent, ErrorType};
use crate::config::grpc_service::*;


pub struct GrpcService {
    sender: mpsc::Sender<InternalEvent>,
    receiver: mpsc::Receiver<EdgeUpload>,
    context: AppContext,
}


impl GrpcService {
    pub fn new(sender: mpsc::Sender<InternalEvent>,
               receiver: mpsc::Receiver<EdgeUpload>,
               context: AppContext) -> Self {
        Self {
            sender,
            receiver,
            context,
        }
    }

    pub async fn run(mut self) {

        let (tx_to_core, mut rx_from_grpc) = mpsc::channel::<InternalEvent>(50);
        let (tx, rx) = mpsc::channel::<EdgeUpload>(50);

        tokio::spawn(remote_grpc(tx_to_core, rx, self.context.clone()));
        
        loop {
            tokio::select! {
                Some(edge_upload) = self.receiver.recv() => {
                    if tx.send(edge_upload).await.is_err() {
                        error!("Error: no se pudo enviar EdgeUpload a remote_grpc");
                    }
                }
                
                Some(response) = rx_from_grpc.recv() => {
                    if self.sender.send(response).await.is_err() {
                        error!("Error: no se pudo enviar InternalEvent desde remote_grpc");
                    }
                }
            }
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq)]
enum StateClient {
    Init,
    Work,
    Error,
}


async fn create_tls_channel(system: &crate::system::domain::System) -> Result<Channel, ErrorType> {
    let ca_pem = fs::read("/etc/mosquitto/certs_edge_remote/ca_edge_remote.crt")
        .map_err(|e| { error!("Fallo leyendo CA: {}", e); ErrorType::Generic })?;
    let cert_pem = fs::read("/etc/mosquitto/certs_edge_remote/edge_remote.crt")
        .map_err(|e| { error!("Fallo leyendo Cert: {}", e); ErrorType::Generic })?;
    let key_pem = fs::read("/etc/mosquitto/certs_edge_remote/edge_remote.key")
        .map_err(|e| { error!("Fallo leyendo Key: {}", e); ErrorType::Generic })?;

    let ca = Certificate::from_pem(ca_pem);
    let identity = Identity::from_pem(cert_pem, key_pem);

    let tls = ClientTlsConfig::new()
        .ca_certificate(ca)
        .identity(identity)
        .domain_name("dominio-del-certificado.com");

    let url = format!("https://{}:50051", system.host_server);

    let endpoint = Channel::from_shared(url)
        .map_err(|_| ErrorType::Endpoint)?
        .tls_config(tls)
        .map_err(|_| ErrorType::Endpoint)?
        .connect_timeout(Duration::from_secs(TLS_CERT_TIMEOUT_SECS))
        .keep_alive_timeout(Duration::from_secs(KEEP_ALIVE_TIMEOUT_SECS))
        .http2_keep_alive_interval(Duration::from_secs(15))
        .keep_alive_while_idle(true);

    endpoint.connect().await.map_err(|e| {
        error!("Error: No se pudo conectar gRPC: {}", e);
        ErrorType::Endpoint
    })
}


async fn remote_grpc(tx: mpsc::Sender<InternalEvent>,
                     mut rx_outbound: mpsc::Receiver<EdgeUpload>,
                     app_context: AppContext) {

    let mut state = StateClient::Init;
    let mut tx_session: Option<mpsc::Sender<EdgeUpload>> = None;
    let mut inbound_stream: Option<tonic::Streaming<EdgeDownload>> = None;

    loop {
        match state {
            StateClient::Init => {
                info!("Info: Intentando conectar gRPC...");
                match create_tls_channel(&app_context.system).await {
                    Ok(channel) => {
                        // Crear cliente con compresión
                        let mut grpc_client = IotServiceClient::new(channel)
                            .send_compressed(CompressionEncoding::Gzip)
                            .accept_compressed(CompressionEncoding::Gzip);

                        // Configurar stream bidireccional
                        let (tx_sess, rx_session) = mpsc::channel::<EdgeUpload>(100);
                        let request = Request::new(ReceiverStream::new(rx_session));

                        match grpc_client.connect_stream(request).await {
                            Ok(response) => {
                                info!("Info: gRPC Conectado. Stream Bidireccional iniciado");

                                tx_session = Some(tx_sess);
                                inbound_stream = Some(response.into_inner());

                                if tx.send(InternalEvent::ServerConnected).await.is_err() {
                                    error!("Error: No se pudo enviar el evento ServerConnected");
                                }
                                state = StateClient::Work;
                            }
                            Err(e) => {
                                error!("{}", e);
                                state = StateClient::Error;
                            }
                        }
                    }
                    Err(e) => {
                        error!("{:?}", e);
                        state = StateClient::Error;
                    }
                }
            }

            StateClient::Work => {
                if let (Some(tx_sess), Some(stream)) = (tx_session.as_ref(), inbound_stream.as_mut()) {
                    tokio::select! {
                        msg_opt = rx_outbound.recv() => {   // Enviar datos (Upstream)
                            match msg_opt {
                                Some(msg) => {
                                    if let Err(e) = tx_sess.send(msg).await {
                                        warn!("Warning: Stream de envío cerrado {}", e);
                                        state = StateClient::Error;
                                    }
                                }
                                None => {
                                    info!("Info: Canal de salida cerrado, terminando tarea remota");
                                    return;
                                }
                            }
                        }

                        server_msg = stream.next() => {   // Recibir datos (Downstream)
                            match server_msg {
                                Some(Ok(download_msg)) => {
                                    if tx.send(InternalEvent::IncomingGrpc(download_msg)).await.is_err() {
                                        error!("Error: No se pudo enviar el mensaje recibido del servidor");
                                    }
                                }
                                Some(Err(e)) => {
                                    error!("Error: stream gRPC {}", e);
                                    state = StateClient::Error;
                                }
                                None => {
                                    warn!("Warning: Stream cerrado por el servidor");
                                    state = StateClient::Error;
                                }
                            }
                        }
                    }
                } else {
                    warn!("Warning: Estado Work sin stream válido, reiniciando...");
                    state = StateClient::Init;
                }
            }

            StateClient::Error => {
                warn!("Warning: Desconectado del servidor. Reintentando en 5s...");
                if tx.send(InternalEvent::ServerDisconnected).await.is_err() {
                    error!("Error: No se pudo enviar el evento ServerDisconnected");
                }

                // Limpiar recursos
                tx_session = None;
                inbound_stream = None;

                tokio::time::sleep(Duration::from_secs(5)).await;
                state = StateClient::Init;
            }
        }
    }
}