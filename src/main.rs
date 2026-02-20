use crate::channels::domain::Channels;
use crate::core::domain::Core;
use crate::database::domain::DataService;
use crate::database::repository::Repository;
use crate::firmware::domain::{FirmwareService};
use crate::fsm::domain::{FsmService};
use crate::grpc_service::domain::GrpcService;
use crate::heartbeat::domain::HeartbeatService;
use crate::message::domain::{MessageService};
use crate::metrics::domain::MetricsService;
use crate::mqtt::domain::MqttService;
use crate::network::domain::{NetworkService};
use crate::system::domain::{init_tracing};
use crate::system::fsm::init_fsm;

mod system;
mod mqtt;
mod message;
mod database;
mod config;
mod network;
mod fsm;
mod context;
mod firmware;
mod metrics;
mod grpc_service;
mod heartbeat;
mod quorum;
mod core;
mod channels;

pub mod grpc {
    tonic::include_proto!("grpc");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    init_tracing();

    let app_context = init_fsm().await?;

    let repo = Repository::create_repository(
        &app_context.system.db_path
    ).await;

    let channels = Channels::new(50);

    // ===================== CORE =====================

    let core = Core::builder()
        .core_from_data_service(channels.core_from_data_service)
        .core_to_data_service(channels.core_to_data_service)
        .core_from_firmware_service(channels.core_from_firmware_service)
        .core_to_firmware_service(channels.core_to_firmware_service)
        .core_from_fsm_service(channels.core_from_fsm_service)
        .core_to_fsm_service(channels.core_to_fsm_service)
        .core_from_grpc_service(channels.core_from_grpc_service)
        .core_to_grpc_service(channels.core_to_grpc_service)
        .core_from_heartbeat_service(channels.core_from_heartbeat_service)
        .core_to_heartbeat_service(channels.core_to_heartbeat_service)
        .core_from_message_service(channels.core_from_message_service)
        .core_to_message_service(channels.core_to_message_service)
        .core_from_metrics_service(channels.core_from_metrics_service)
        .core_from_mqtt_service(channels.core_from_mqtt_service)
        .core_to_mqtt_service(channels.core_to_mqtt_service)
        .core_from_network_service(channels.core_from_network_service)
        .core_to_network_service(channels.core_to_network_service)
        .build()?;

    // ===================== SERVICIOS =====================

    let core_handle = tokio::spawn(core.run());

    let data_service_handle = tokio::spawn(
        DataService::new(
            channels.data_service_to_core,
            channels.data_service_from_core,
            repo,
        ).run()
    );

    let firmware_handle = tokio::spawn(
        FirmwareService::new(
            channels.firmware_service_to_core,
            channels.firmware_service_from_core,
            app_context.clone(),
        ).run()
    );

    let fsm_handle = tokio::spawn(
        FsmService::new(
            channels.fsm_service_to_core,
            channels.fsm_service_from_core,
            app_context.clone(),
        ).run()
    );

    let grpc_handle = tokio::spawn(
        GrpcService::new(
            channels.grpc_service_to_core,
            channels.grpc_service_from_core,
            app_context.clone(),
        ).run()
    );

    let heartbeat_handle = tokio::spawn(
        HeartbeatService::new(
            channels.heartbeat_service_to_core,
            channels.heartbeat_service_from_core,
        ).run()
    );

    let message_handle = tokio::spawn(
        MessageService::new(
            channels.message_service_to_core,
            channels.message_service_from_core,
            app_context.clone(),
        ).run()
    );

    let metrics_handle = tokio::spawn(
        MetricsService::new(
            channels.metrics_service_to_core,
            app_context.clone(),
        ).run()
    );

    let mqtt_handle = tokio::spawn(
        MqttService::new(
            channels.mqtt_service_to_core,
            channels.mqtt_service_from_core,
            app_context.clone(),
        ).run()
    );

    let network_handle = tokio::spawn(
        NetworkService::new(
            channels.network_service_to_core,
            channels.network_service_from_core,
            app_context.clone(),
        ).run()
    );

    // ===================== SUPERVISION =====================

    tokio::try_join!(
        core_handle,
        data_service_handle,
        firmware_handle,
        fsm_handle,
        grpc_handle,
        heartbeat_handle,
        message_handle,
        metrics_handle,
        mqtt_handle,
        network_handle,
    )?;

    Ok(())
}
