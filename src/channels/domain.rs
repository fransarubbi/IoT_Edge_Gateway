use tokio::sync::mpsc;
use crate::database::domain::{DataServiceCommand, DataServiceResponse};
use crate::firmware::domain::{FirmwareServiceCommand, FirmwareServiceResponse};
use crate::fsm::domain::{FsmServiceCommand, FsmServiceResponse};
use crate::grpc::EdgeUpload;
use crate::message::domain::{MessageServiceCommand, MessageServiceResponse, SerializedMessage, ServerMessage};
use crate::network::domain::{NetworkServiceCommand, NetworkServiceResponse};
use crate::system::domain::InternalEvent;

pub struct Channels {
    pub data_service_to_core: mpsc::Sender<DataServiceResponse>,
    pub core_from_data_service: mpsc::Receiver<DataServiceResponse>,

    pub core_to_data_service: mpsc::Sender<DataServiceCommand>,
    pub data_service_from_core: mpsc::Receiver<DataServiceCommand>,

    pub firmware_service_to_core: mpsc::Sender<FirmwareServiceResponse>,
    pub core_from_firmware_service: mpsc::Receiver<FirmwareServiceResponse>,

    pub core_to_firmware_service: mpsc::Sender<FirmwareServiceCommand>,
    pub firmware_service_from_core: mpsc::Receiver<FirmwareServiceCommand>,

    pub fsm_service_to_core: mpsc::Sender<FsmServiceResponse>,
    pub core_from_fsm_service: mpsc::Receiver<FsmServiceResponse>,

    pub core_to_fsm_service: mpsc::Sender<FsmServiceCommand>,
    pub fsm_service_from_core: mpsc::Receiver<FsmServiceCommand>,

    pub grpc_service_to_core: mpsc::Sender<InternalEvent>,
    pub core_from_grpc_service: mpsc::Receiver<InternalEvent>,

    pub core_to_grpc_service: mpsc::Sender<EdgeUpload>,
    pub grpc_service_from_core: mpsc::Receiver<EdgeUpload>,

    pub heartbeat_service_to_core: mpsc::Sender<InternalEvent>,
    pub core_from_heartbeat_service: mpsc::Receiver<InternalEvent>,

    pub core_to_heartbeat_service: mpsc::Sender<ServerMessage>,
    pub heartbeat_service_from_core: mpsc::Receiver<ServerMessage>,

    pub message_service_to_core: mpsc::Sender<MessageServiceResponse>,
    pub core_from_message_service: mpsc::Receiver<MessageServiceResponse>,

    pub core_to_message_service: mpsc::Sender<MessageServiceCommand>,
    pub message_service_from_core: mpsc::Receiver<MessageServiceCommand>,

    pub metrics_service_to_core: mpsc::Sender<ServerMessage>,
    pub core_from_metrics_service: mpsc::Receiver<ServerMessage>,

    pub mqtt_service_to_core: mpsc::Sender<InternalEvent>,
    pub core_from_mqtt_service: mpsc::Receiver<InternalEvent>,

    pub core_to_mqtt_service: mpsc::Sender<SerializedMessage>,
    pub mqtt_service_from_core: mpsc::Receiver<SerializedMessage>,

    pub network_service_to_core: mpsc::Sender<NetworkServiceResponse>,
    pub core_from_network_service: mpsc::Receiver<NetworkServiceResponse>,

    pub core_to_network_service: mpsc::Sender<NetworkServiceCommand>,
    pub network_service_from_core: mpsc::Receiver<NetworkServiceCommand>
}


impl Channels {
    
    /// Crea una nueva instancia de Channels inicializando todos los canales MPSC.
    /// `buffer_size` determina la capacidad de la cola de cada canal (backpressure).
    pub fn new(buffer_size: usize) -> Self {
        // 1. Data Service
        let (data_s2c_tx, data_s2c_rx) = mpsc::channel(buffer_size);
        let (data_c2s_tx, data_c2s_rx) = mpsc::channel(buffer_size);

        // 2. Firmware Service
        let (fw_s2c_tx, fw_s2c_rx) = mpsc::channel(buffer_size);
        let (fw_c2s_tx, fw_c2s_rx) = mpsc::channel(buffer_size);

        // 3. FSM Service
        let (fsm_s2c_tx, fsm_s2c_rx) = mpsc::channel(buffer_size);
        let (fsm_c2s_tx, fsm_c2s_rx) = mpsc::channel(buffer_size);

        // 4. gRPC Service
        let (grpc_s2c_tx, grpc_s2c_rx) = mpsc::channel(buffer_size);
        let (grpc_c2s_tx, grpc_c2s_rx) = mpsc::channel(buffer_size);

        // 5. Heartbeat Service
        let (hb_s2c_tx, hb_s2c_rx) = mpsc::channel(buffer_size);
        let (hb_c2s_tx, hb_c2s_rx) = mpsc::channel(buffer_size);

        // 6. Message Service
        let (msg_s2c_tx, msg_s2c_rx) = mpsc::channel(buffer_size);
        let (msg_c2s_tx, msg_c2s_rx) = mpsc::channel(buffer_size);

        // 7. Metrics Service
        let (met_s2c_tx, met_s2c_rx) = mpsc::channel(buffer_size);

        // 8. MQTT Service
        let (mqtt_s2c_tx, mqtt_s2c_rx) = mpsc::channel(buffer_size);
        let (mqtt_c2s_tx, mqtt_c2s_rx) = mpsc::channel(buffer_size);

        // 9. Network Service
        let (net_s2c_tx, net_s2c_rx) = mpsc::channel(buffer_size);
        let (net_c2s_tx, net_c2s_rx) = mpsc::channel(buffer_size);

        Self {
            // Data
            data_service_to_core: data_s2c_tx,
            core_from_data_service: data_s2c_rx,
            core_to_data_service: data_c2s_tx,
            data_service_from_core: data_c2s_rx,

            // Firmware
            firmware_service_to_core: fw_s2c_tx,
            core_from_firmware_service: fw_s2c_rx,
            core_to_firmware_service: fw_c2s_tx,
            firmware_service_from_core: fw_c2s_rx,

            // FSM
            fsm_service_to_core: fsm_s2c_tx,
            core_from_fsm_service: fsm_s2c_rx,
            core_to_fsm_service: fsm_c2s_tx,
            fsm_service_from_core: fsm_c2s_rx,

            // gRPC
            grpc_service_to_core: grpc_s2c_tx,
            core_from_grpc_service: grpc_s2c_rx,
            core_to_grpc_service: grpc_c2s_tx,
            grpc_service_from_core: grpc_c2s_rx,

            // Heartbeat
            heartbeat_service_to_core: hb_s2c_tx,
            core_from_heartbeat_service: hb_s2c_rx,
            core_to_heartbeat_service: hb_c2s_tx,
            heartbeat_service_from_core: hb_c2s_rx,

            // Message
            message_service_to_core: msg_s2c_tx,
            core_from_message_service: msg_s2c_rx,
            core_to_message_service: msg_c2s_tx,
            message_service_from_core: msg_c2s_rx,

            // Metrics
            metrics_service_to_core: met_s2c_tx,
            core_from_metrics_service: met_s2c_rx,

            // MQTT
            mqtt_service_to_core: mqtt_s2c_tx,
            core_from_mqtt_service: mqtt_s2c_rx,
            core_to_mqtt_service: mqtt_c2s_tx,
            mqtt_service_from_core: mqtt_c2s_rx,

            // Network
            network_service_to_core: net_s2c_tx,
            core_from_network_service: net_s2c_rx,
            core_to_network_service: net_c2s_tx,
            network_service_from_core: net_c2s_rx,
        }
    }
}