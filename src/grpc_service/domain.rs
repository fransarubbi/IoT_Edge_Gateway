use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};
use tonic::codec::CompressionEncoding;
use tonic::Request;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use std::fs;
use tracing::{error, info, warn};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use crate::context::domain::AppContext;
use crate::grpc::{ToEdge, FromEdge};
use crate::grpc::edge_service_client::EdgeServiceClient;
use crate::system::domain::{InternalEvent, ErrorType};
use crate::config::grpc_service::*;


pub struct GrpcService {
    sender: mpsc::Sender<InternalEvent>,
    receiver: mpsc::Receiver<FromEdge>,
    context: AppContext,
}


impl GrpcService {
    pub fn new(sender: mpsc::Sender<InternalEvent>,
               receiver: mpsc::Receiver<FromEdge>,
               context: AppContext) -> Self {
        Self {
            sender,
            receiver,
            context,
        }
    }

    pub async fn run(mut self, shutdown: CancellationToken) {

        let (tx_to_core, mut rx_from_grpc) = mpsc::channel::<InternalEvent>(50);
        let (tx, rx) = mpsc::channel::<FromEdge>(50);

        tokio::spawn(grpc(tx_to_core, rx, self.context.clone(), shutdown.clone()));
        
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!("Info: Shutdown recibido GrpcService");
                    break;
                }

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


#[derive(Debug)]
enum StateClient {
    Init,
    Work {
        tx_session: mpsc::Sender<FromEdge>,
        inbound_stream: tonic::Streaming<ToEdge>,
    },
    Error,
}


async fn create_tls_channel(system: &crate::system::domain::System) -> Result<Channel, ErrorType> {
    let ca_pem = fs::read(CA_EDGE_GRPC)
        .map_err(|e| { error!("Error: fallo leyendo CA gRPC: {}", e); ErrorType::Generic })?;
    let cert_pem = fs::read(CRT_EDGE_GRPC)
        .map_err(|e| { error!("Error: fallo leyendo Cert gRPC: {}", e); ErrorType::Generic })?;
    let key_pem = fs::read(KEY_EDGE_GRPC)
        .map_err(|e| { error!("Error: fallo leyendo Key gRPC: {}", e); ErrorType::Generic })?;

    let ca = Certificate::from_pem(ca_pem);
    let identity = Identity::from_pem(cert_pem, key_pem);

    let tls = ClientTlsConfig::new()
        .ca_certificate(ca)
        .identity(identity)
        .domain_name(system.cn.clone());

    let url = format!("https://{}:{}", system.host_server, system.host_port);

    let endpoint = Channel::from_shared(url)
        .map_err(|_| ErrorType::Endpoint)?
        .tls_config(tls)
        .map_err(|_| ErrorType::Endpoint)?
        .connect_timeout(Duration::from_secs(TLS_CERT_TIMEOUT_SECS))
        .keep_alive_timeout(Duration::from_secs(KEEP_ALIVE_TIMEOUT_SECS))
        .http2_keep_alive_interval(Duration::from_secs(HTTP2_KEEP_ALIVE_INTERVAL_SECS))
        .keep_alive_while_idle(true);

    endpoint.connect().await.map_err(|e| {
        error!("Error: no se pudo conectar gRPC: {}", e);
        ErrorType::Endpoint
    })
}


async fn grpc(tx: mpsc::Sender<InternalEvent>,
              mut rx_outbound: mpsc::Receiver<FromEdge>,
              app_context: AppContext,
              shutdown: CancellationToken) {

    let mut state = StateClient::Init;

    loop {
        match &mut state {
            StateClient::Init => {
                info!("Info: intentando conectar gRPC");
                tokio::select! {
                    _ = shutdown.cancelled() => {
                        info!("Info: shutdown recibido en grpc (Init)");
                        break;
                    }

                    result = create_tls_channel(&app_context.system) => {
                        match result {
                            Ok(channel) => {
                                // Crear cliente con compresión
                                let mut grpc_client = EdgeServiceClient::new(channel)
                                    .send_compressed(CompressionEncoding::Gzip)
                                    .accept_compressed(CompressionEncoding::Gzip);

                                // Configurar stream bidireccional
                                let (tx_session, rx_session) = mpsc::channel::<FromEdge>(100);
                                let request = Request::new(ReceiverStream::new(rx_session));

                                match grpc_client.connect_stream(request).await {
                                    Ok(response) => {
                                        info!("Info: gRPC Conectado. Stream Bidireccional iniciado");

                                        let inbound_stream = response.into_inner();

                                        if tx.send(InternalEvent::ServerConnected).await.is_err() {
                                            error!("Error: no se pudo enviar el evento ServerConnected");
                                        }
                                        state = StateClient::Work { tx_session, inbound_stream };
                                    }
                                    Err(e) => {
                                        error!("Error: {e}");
                                        state = StateClient::Error;
                                    }
                                }
                            }
                            Err(e) => {
                                error!("Error: {e}");
                                state = StateClient::Error;
                            }
                        }
                    }
                }
            }
            StateClient::Work { tx_session, inbound_stream } => {
                tokio::select! {
                    _ = shutdown.cancelled() => {
                        info!("Info: shutdown recibido gRPC (Work)");
                        break;
                    }

                    msg_opt = rx_outbound.recv() => {   // Enviar datos (Upstream)
                        match msg_opt {
                            Some(msg) => {
                                if let Err(e) = tx_session.send(msg).await {
                                    warn!("Warning: stream de envío cerrado {e}");
                                    state = StateClient::Error;
                                }
                            }
                            None => {
                                info!("Info: canal de salida cerrado, terminando tarea remota");
                                return;
                            }
                        }
                    }

                    server_msg = inbound_stream.next() => {   // Recibir datos (Downstream)
                        match server_msg {
                            Some(Ok(download_msg)) => {
                                if tx.send(InternalEvent::IncomingGrpc(download_msg)).await.is_err() {
                                    error!("Error: no se pudo enviar el mensaje recibido del servidor");
                                }
                            }
                            Some(Err(e)) => {
                                error!("Error: stream gRPC {e}");
                                state = StateClient::Error;
                            }
                            None => {
                                warn!("Warning: stream cerrado por el servidor");
                                state = StateClient::Error;
                            }
                        }
                    }
                }
            }
            StateClient::Error => {
                if tx.send(InternalEvent::ServerDisconnected).await.is_err() {
                    error!("Error: no se pudo enviar el evento ServerDisconnected");
                }
                tokio::select! {
                    _ = shutdown.cancelled() => {
                        info!("Info: shutdown recibido gRPC (Error)");
                        break;
                    }
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {
                        state = StateClient::Init;
                    }
                }

            }
        }
    }
}