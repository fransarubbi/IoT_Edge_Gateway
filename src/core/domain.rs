use tokio::sync::mpsc;
use tracing::error;
use crate::database::domain::{DataServiceCommand, DataServiceResponse};
use crate::firmware::domain::{FirmwareServiceCommand, FirmwareServiceResponse};
use crate::fsm::domain::{FsmServiceCommand, FsmServiceResponse};
use crate::grpc::EdgeUpload;
use crate::message::domain::{HubMessage, MessageServiceCommand, MessageServiceResponse, SerializedMessage, ServerMessage};
use crate::network::domain::{NetworkServiceCommand, NetworkServiceResponse};
use crate::system::domain::InternalEvent;


pub struct Core {
    core_from_data_service: mpsc::Receiver<DataServiceResponse>,
    core_to_data_service: mpsc::Sender<DataServiceCommand>,
    core_from_firmware_service: mpsc::Receiver<FirmwareServiceResponse>,
    core_to_firmware_service: mpsc::Sender<FirmwareServiceCommand>,
    core_from_fsm_service: mpsc::Receiver<FsmServiceResponse>,
    core_to_fsm_service: mpsc::Sender<FsmServiceCommand>,
    core_from_grpc_service: mpsc::Receiver<InternalEvent>,
    core_to_grpc_service: mpsc::Sender<EdgeUpload>,
    core_from_heartbeat_service: mpsc::Receiver<InternalEvent>,
    core_to_heartbeat_service: mpsc::Sender<ServerMessage>,
    core_from_message_service: mpsc::Receiver<MessageServiceResponse>,
    core_to_message_service: mpsc::Sender<MessageServiceCommand>,
    core_from_metrics_service: mpsc::Receiver<ServerMessage>,
    core_from_mqtt_service: mpsc::Receiver<InternalEvent>,
    core_to_mqtt_service: mpsc::Sender<SerializedMessage>,
    core_from_network_service: mpsc::Receiver<NetworkServiceResponse>,
    core_to_network_service: mpsc::Sender<NetworkServiceCommand>
}


#[derive(Default)]
pub struct CoreBuilder {
    core_from_data_service: Option<mpsc::Receiver<DataServiceResponse>>,
    core_to_data_service: Option<mpsc::Sender<DataServiceCommand>>,
    core_from_firmware_service: Option<mpsc::Receiver<FirmwareServiceResponse>>,
    core_to_firmware_service: Option<mpsc::Sender<FirmwareServiceCommand>>,
    core_from_fsm_service: Option<mpsc::Receiver<FsmServiceResponse>>,
    core_to_fsm_service: Option<mpsc::Sender<FsmServiceCommand>>,
    core_from_grpc_service: Option<mpsc::Receiver<InternalEvent>>,
    core_to_grpc_service: Option<mpsc::Sender<EdgeUpload>>,
    core_from_heartbeat_service: Option<mpsc::Receiver<InternalEvent>>,
    core_to_heartbeat_service: Option<mpsc::Sender<ServerMessage>>,
    core_from_message_service: Option<mpsc::Receiver<MessageServiceResponse>>,
    core_to_message_service: Option<mpsc::Sender<MessageServiceCommand>>,
    core_from_metrics_service: Option<mpsc::Receiver<ServerMessage>>,
    core_from_mqtt_service: Option<mpsc::Receiver<InternalEvent>>,
    core_to_mqtt_service: Option<mpsc::Sender<SerializedMessage>>,
    core_from_network_service: Option<mpsc::Receiver<NetworkServiceResponse>>,
    core_to_network_service: Option<mpsc::Sender<NetworkServiceCommand>>,
}


impl CoreBuilder {
    // --- Métodos Setter ---
    // Cada método consume self y devuelve self para permitir encadenamiento.

    pub fn core_from_data_service(mut self, ch: mpsc::Receiver<DataServiceResponse>) -> Self {
        self.core_from_data_service = Some(ch);
        self
    }
    pub fn core_to_data_service(mut self, ch: mpsc::Sender<DataServiceCommand>) -> Self {
        self.core_to_data_service = Some(ch);
        self
    }

    pub fn core_from_firmware_service(mut self, ch: mpsc::Receiver<FirmwareServiceResponse>) -> Self {
        self.core_from_firmware_service = Some(ch);
        self
    }
    pub fn core_to_firmware_service(mut self, ch: mpsc::Sender<FirmwareServiceCommand>) -> Self {
        self.core_to_firmware_service = Some(ch);
        self
    }

    pub fn core_from_fsm_service(mut self, ch: mpsc::Receiver<FsmServiceResponse>) -> Self {
        self.core_from_fsm_service = Some(ch);
        self
    }
    pub fn core_to_fsm_service(mut self, ch: mpsc::Sender<FsmServiceCommand>) -> Self {
        self.core_to_fsm_service = Some(ch);
        self
    }

    pub fn core_from_grpc_service(mut self, ch: mpsc::Receiver<InternalEvent>) -> Self {
        self.core_from_grpc_service = Some(ch);
        self
    }
    pub fn core_to_grpc_service(mut self, ch: mpsc::Sender<EdgeUpload>) -> Self {
        self.core_to_grpc_service = Some(ch);
        self
    }

    pub fn core_from_heartbeat_service(mut self, ch: mpsc::Receiver<InternalEvent>) -> Self {
        self.core_from_heartbeat_service = Some(ch);
        self
    }
    pub fn core_to_heartbeat_service(mut self, ch: mpsc::Sender<ServerMessage>) -> Self {
        self.core_to_heartbeat_service = Some(ch);
        self
    }

    pub fn core_from_message_service(mut self, ch: mpsc::Receiver<MessageServiceResponse>) -> Self {
        self.core_from_message_service = Some(ch);
        self
    }
    pub fn core_to_message_service(mut self, ch: mpsc::Sender<MessageServiceCommand>) -> Self {
        self.core_to_message_service = Some(ch);
        self
    }

    pub fn core_from_metrics_service(mut self, ch: mpsc::Receiver<ServerMessage>) -> Self {
        self.core_from_metrics_service = Some(ch);
        self
    }

    pub fn core_from_mqtt_service(mut self, ch: mpsc::Receiver<InternalEvent>) -> Self {
        self.core_from_mqtt_service = Some(ch);
        self
    }
    pub fn core_to_mqtt_service(mut self, ch: mpsc::Sender<SerializedMessage>) -> Self {
        self.core_to_mqtt_service = Some(ch);
        self
    }

    pub fn core_from_network_service(mut self, ch: mpsc::Receiver<NetworkServiceResponse>) -> Self {
        self.core_from_network_service = Some(ch);
        self
    }
    pub fn core_to_network_service(mut self, ch: mpsc::Sender<NetworkServiceCommand>) -> Self {
        self.core_to_network_service = Some(ch);
        self
    }

    // Valida que todos los campos estén presentes.
    pub fn build(self) -> Result<Core, String> {
        Ok(Core {
            core_from_data_service: self.core_from_data_service.ok_or("Missing field: core_from_data_service")?,
            core_to_data_service: self.core_to_data_service.ok_or("Missing field: core_to_data_service")?,
            core_from_firmware_service: self.core_from_firmware_service.ok_or("Missing field: core_from_firmware_service")?,
            core_to_firmware_service: self.core_to_firmware_service.ok_or("Missing field: core_to_firmware_service")?,
            core_from_fsm_service: self.core_from_fsm_service.ok_or("Missing field: core_from_fsm_service")?,
            core_to_fsm_service: self.core_to_fsm_service.ok_or("Missing field: core_to_fsm_service")?,
            core_from_grpc_service: self.core_from_grpc_service.ok_or("Missing field: core_from_grpc_service")?,
            core_to_grpc_service: self.core_to_grpc_service.ok_or("Missing field: core_to_grpc_service")?,
            core_from_heartbeat_service: self.core_from_heartbeat_service.ok_or("Missing field: core_from_heartbeat_service")?,
            core_to_heartbeat_service: self.core_to_heartbeat_service.ok_or("Missing field: core_to_heartbeat_service")?,
            core_from_message_service: self.core_from_message_service.ok_or("Missing field: core_from_message_service")?,
            core_to_message_service: self.core_to_message_service.ok_or("Missing field: core_to_message_service")?,
            core_from_metrics_service: self.core_from_metrics_service.ok_or("Missing field: core_from_metrics_service")?,
            core_from_mqtt_service: self.core_from_mqtt_service.ok_or("Missing field: core_from_mqtt_service")?,
            core_to_mqtt_service: self.core_to_mqtt_service.ok_or("Missing field: core_to_mqtt_service")?,
            core_from_network_service: self.core_from_network_service.ok_or("Missing field: core_from_network_service")?,
            core_to_network_service: self.core_to_network_service.ok_or("Missing field: core_to_network_service")?,
        })
    }
}


impl Core {
    pub fn builder() -> CoreBuilder {
        CoreBuilder::default()
    }

    pub async fn run(mut self) {

        loop {
            tokio::select! {
                Some(response) = self.core_from_data_service.recv() => {
                    match response {
                        DataServiceResponse::Batch(batch) => {
                            if self.core_to_message_service.send(MessageServiceCommand::Batch(batch)).await.is_err() {
                                error!("Failed to send message to message service");
                            }
                        },
                        DataServiceResponse::Epoch(epoch) => {
                            if self.core_to_fsm_service.send(FsmServiceCommand::Epoch(epoch)).await.is_err() {
                                error!("Failed to send message to fsm service");
                            }
                        },
                        DataServiceResponse::ErrorEpoch => {
                            if self.core_to_fsm_service.send(FsmServiceCommand::ErrorEpoch).await.is_err() {
                                error!("Failed to send message to fsm service");
                            }
                        },
                    }
                }
                Some(response) = self.core_from_firmware_service.recv() => {
                    match response {
                        FirmwareServiceResponse::ServerAck(server_msg) => {
                            if self.core_to_message_service.send(MessageServiceCommand::ToServer(server_msg)).await.is_err() {
                                error!("Failed to send message to message service");
                            }
                        },
                        FirmwareServiceResponse::HubCommand(hub_msg) => {
                            if self.core_to_message_service.send(MessageServiceCommand::ToHub(hub_msg)).await.is_err() {
                                error!("Failed to send message to message service");
                            }
                        }
                    }
                }
                Some(response) = self.core_from_fsm_service.recv() => {
                    match response {
                        FsmServiceResponse::ToServer(to_server) => {
                            if self.core_to_message_service.send(MessageServiceCommand::ToServer(to_server)).await.is_err() {
                                error!("Failed to send message to message service");
                            }
                        },
                        FsmServiceResponse::ToHub(to_hub) => {
                            if self.core_to_message_service.send(MessageServiceCommand::ToHub(to_hub)).await.is_err() {
                                error!("Failed to send message to message service");
                            }
                        },
                        FsmServiceResponse::NewEpoch(new_epoch) => {
                            if self.core_to_data_service.send(DataServiceCommand::NewEpoch(new_epoch)).await.is_err() {
                                error!("Failed to send message to data service");
                            }
                        },
                        FsmServiceResponse::GetEpoch => {
                            if self.core_to_data_service.send(DataServiceCommand::GetEpoch).await.is_err() {
                                error!("Failed to send message to data service");
                            }
                        }
                    }
                }
                Some(response) = self.core_from_grpc_service.recv() => {
                    if self.core_to_message_service.send(MessageServiceCommand::Internal(response)).await.is_err() {
                        error!("Failed to send message to message service");
                    }
                }
                Some(response) = self.core_from_heartbeat_service.recv() => {
                    if self.core_to_message_service.send(MessageServiceCommand::Internal(response.clone())).await.is_err() {
                        error!("Failed to send message to message service");
                    }
                    if self.core_to_data_service.send(DataServiceCommand::Internal(response)).await.is_err() {
                        error!("Failed to send message to data service");
                    }
                }
                Some(response) = self.core_from_message_service.recv() => {
                    match response {
                        MessageServiceResponse::EdgeUpload(upload) => {
                            if self.core_to_grpc_service.send(upload).await.is_err() {
                                error!("Error: ");
                            }
                        },
                        MessageServiceResponse::FromHub(from_hub) => {
                            match from_hub {
                                HubMessage::Report(_) => {
                                    if self.core_to_data_service.send(DataServiceCommand::Hub(from_hub)).await.is_err() {
                                        error!("Error: ");
                                    }
                                },
                                HubMessage::Monitor(_) => {
                                    if self.core_to_data_service.send(DataServiceCommand::Hub(from_hub)).await.is_err() {
                                        error!("Error: ");
                                    }
                                },
                                HubMessage::AlertAir(_) => {
                                    if self.core_to_data_service.send(DataServiceCommand::Hub(from_hub)).await.is_err() {
                                        error!("Error: ");
                                    }
                                },
                                HubMessage::AlertTem(_) => {
                                    if self.core_to_data_service.send(DataServiceCommand::Hub(from_hub)).await.is_err() {
                                        error!("Error: ");
                                    }
                                },
                                HubMessage::FromHubSettings(_) => {
                                    if self.core_to_data_service.send(DataServiceCommand::Hub(from_hub)).await.is_err() {
                                        error!("Error: ");
                                    }
                                },
                                HubMessage::FromHubSettingsAck(_) => {
                                    if self.core_to_data_service.send(DataServiceCommand::Hub(from_hub)).await.is_err() {
                                        error!("Error: ");
                                    }
                                }
                                HubMessage::FirmwareOk(firmware) => {
                                    if self.core_to_firmware_service.send(FirmwareServiceCommand::HubResponse(firmware)).await.is_err() {
                                        error!("Error: ");
                                    }
                                }
                                HubMessage::HandshakeFromHub(_) => {
                                    if self.core_to_fsm_service.send(FsmServiceCommand::FromHub(from_hub)).await.is_err() {
                                        error!("Error: ");
                                    }
                                },
                                HubMessage::EmptyQueue(_) => {
                                    if self.core_to_fsm_service.send(FsmServiceCommand::FromHub(from_hub)).await.is_err() {
                                        error!("Error: ");
                                    }
                                },
                                HubMessage::EmptyQueueSafe(_) => {
                                    if self.core_to_fsm_service.send(FsmServiceCommand::FromHub(from_hub)).await.is_err() {
                                        error!("Error: ");
                                    }
                                },
                                HubMessage::Ping(_) => {
                                    if self.core_to_fsm_service.send(FsmServiceCommand::FromHub(from_hub)).await.is_err() {
                                        error!("Error: ");
                                    }
                                }
                                _ => {}
                            }
                        },
                        MessageServiceResponse::FromServer(from_server) => {
                            match from_server {
                                ServerMessage::UpdateFirmware(update) => {
                                    if self.core_to_firmware_service.send(FirmwareServiceCommand::Update(update)).await.is_err() {
                                        error!("Error: ");
                                    }
                                },
                                ServerMessage::DeleteHub(_) => {
                                    if self.core_to_network_service.send(NetworkServiceCommand::ServerMessage(from_server)).await.is_err() {
                                        error!("Error: ");
                                    }
                                },
                                ServerMessage::FromServerSettings(_) => {
                                    if self.core_to_network_service.send(NetworkServiceCommand::ServerMessage(from_server)).await.is_err() {
                                        error!("Error: ");
                                    }
                                },
                                ServerMessage::FromServerSettingsAck(ack) => {
                                    if self.core_to_message_service.send(MessageServiceCommand::ToHub(HubMessage::FromHubSettingsAck(ack))).await.is_err() {
                                        error!("Error: ");
                                    }
                                },
                                ServerMessage::Network(_) => {
                                    if self.core_to_network_service.send(NetworkServiceCommand::ServerMessage(from_server)).await.is_err() {
                                        error!("Error: ");
                                    }
                                },
                                ServerMessage::Heartbeat(_) => {
                                    if self.core_to_heartbeat_service.send(from_server).await.is_err() {
                                        error!("Error: ");
                                    }
                                },
                                ServerMessage::HelloWorld(_) => {
                                    if self.core_to_fsm_service.send(FsmServiceCommand::FromServer(from_server)).await.is_err() {
                                        error!("Error: ");
                                    }
                                }
                                _ => {}
                            }
                        },
                        MessageServiceResponse::Serialized(serialized) => {
                            if self.core_to_mqtt_service.send(serialized).await.is_err() {
                                error!("Failed to send message to message service");
                            }
                        }
                    }
                }
                Some(response) = self.core_from_metrics_service.recv() => {
                    if self.core_to_message_service.send(MessageServiceCommand::ToServer(response)).await.is_err() {
                        error!("Error");
                    }
                }
                Some(response) = self.core_from_mqtt_service.recv() => {
                    if self.core_to_message_service.send(MessageServiceCommand::Internal(response)).await.is_err() {
                        error!("Error");
                    }
                }
                Some(response) = self.core_from_network_service.recv() => {
                    match response {
                        NetworkServiceResponse::HubMessage(hub_msg) => {
                            if self.core_to_message_service.send(MessageServiceCommand::ToHub(hub_msg)).await.is_err() {
                                error!("Error");
                            }
                        },
                        NetworkServiceResponse::DataCommand(data_command) => {
                            match data_command {
                                DataServiceCommand::DeleteNetwork(id) => {
                                    if self.core_to_data_service.send(DataServiceCommand::DeleteNetwork(id)).await.is_err() {
                                        error!("Error");
                                    }
                                },
                                DataServiceCommand::DeleteAllHubByNetwork(id) => {
                                    if self.core_to_data_service.send(DataServiceCommand::DeleteAllHubByNetwork(id)).await.is_err() {
                                        error!("Error");
                                    }
                                },
                                DataServiceCommand::UpdateNetwork(network) => {
                                    if self.core_to_data_service.send(DataServiceCommand::UpdateNetwork(network)).await.is_err() {
                                        error!("Error");
                                    }
                                },
                                DataServiceCommand::NewNetwork(network) => {
                                    if self.core_to_data_service.send(DataServiceCommand::NewNetwork(network)).await.is_err() {
                                        error!("Error");
                                    }
                                },
                                DataServiceCommand::NewHub(id) => {
                                    if self.core_to_data_service.send(DataServiceCommand::NewHub(id)).await.is_err() {
                                        error!("Error");
                                    }
                                },
                                DataServiceCommand::DeleteHub(id) => {
                                    if self.core_to_data_service.send(DataServiceCommand::DeleteHub(id)).await.is_err() {
                                        error!("Error");
                                    }
                                },
                                DataServiceCommand::UpdateHub(id) => {
                                    if self.core_to_data_service.send(DataServiceCommand::UpdateHub(id)).await.is_err() {
                                        error!("Error");
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }
}
