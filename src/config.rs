
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
}

pub mod grpc_service {
    pub const TLS_CERT_TIMEOUT_SECS: u64 = 10;
    pub const KEEP_ALIVE_TIMEOUT_SECS: u64 = 30;
}

pub mod heartbeat {
    use tokio::time::{Duration};
    pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(50);
}