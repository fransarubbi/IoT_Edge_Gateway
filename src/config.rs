pub mod files {
    pub const SYSTEM_TOML_PATH: &str = "/etc/edge/files/system.toml";
    pub const PROTOCOL_TOML_PATH: &str = "/etc/edge/files/protocol.toml";
}

pub mod sqlite {
    use tokio::time::{Duration};

    pub const LIMIT: i64 = 10;
    pub const WAIT_FOR: u64 = 5;
    pub const BATCH_SIZE: usize = 49;
    pub const FLUSH_INTERVAL: Duration = Duration::from_secs(300);  // 5 min
}

pub mod firmware {
    use tokio::time::{Duration};
    pub const OTA_TIMEOUT: Duration = Duration::from_secs(300); // 5 min
}

pub mod fsm {
    pub const PERCENTAGE: f64 = 80.0;
    pub const PHASE_MAX_DURATION: u64 = 600;  // 10 min
    pub const SAFE_MODE_MAX_DURATION: u64 = 900;  // 15 min
}

pub mod grpc_service {
    pub const TLS_CERT_TIMEOUT_SECS: u64 = 10;
    pub const KEEP_ALIVE_TIMEOUT_SECS: u64 = 30;
    pub const HTTP2_KEEP_ALIVE_INTERVAL_SECS: u64 = 15;
    pub const CA_EDGE_GRPC : &str = "/etc/mosquitto/certs/root.crt";
    pub const CRT_EDGE_GRPC : &str = "/etc/mosquitto/certs/edge_fullchain.crt";
    pub const KEY_EDGE_GRPC : &str = "/etc/mosquitto/certs/edge.key";
}

pub mod heartbeat {
    use tokio::time::{Duration};
    pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(50);
}

pub mod mqtt_service {
    pub const CLEAN_SESSION: bool = true;
    pub const BUFFER_SIZE: usize = 1000;
    pub const MTLS_PORT: u16 = 8883;
    pub const KEEP_ALIVE_TIMEOUT_SECS: u64 = 30;
    pub const CA_EDGE_MQTT : &str = "/etc/mosquitto/certs/root.crt";
    pub const CRT_EDGE_MQTT : &str = "/etc/mosquitto/certs/edge_fullchain.crt";
    pub const KEY_EDGE_MQTT : &str = "/etc/mosquitto/certs/edge.key";
}